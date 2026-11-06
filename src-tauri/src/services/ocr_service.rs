//! 前置 OCR（M05 / 契约 §12）＋ S-02 / S-03 落地。
//!
//! 骨架模式行为（**契约时序不变**）：150ms 后 `ocr:status(scanning)`、400ms 后 `done`；
//! 占位框覆盖选区、`hasText = true`、`preview` 为契约固定文案。时序由命令层
//! （`commands::capture` 的后台任务）负责，本模块的引擎方法不 sleep。
//!
//! 本模块提供：
//! - [`det_input_size`]：det 输入尺寸的**像素预算**约束（S-02 实测结论：只按长边限制会让
//!   单 4K 屏反而更慢，必须叠加「宽×高 ≤ [`DET_PIXEL_BUDGET`]」双约束）；
//! - [`OcrEngine`] trait：`detect`（整屏只 det）/ `recognize`（只 rec）/ `detect_and_recognize`
//!   （选区兜底 det+rec）+ [`SkeletonOcrEngine`] 骨架实现；
//! - [`OcrPipeline`]：整屏 det 结果按 `ocr_session_id` 缓存 → 选区只做「过滤 + rec-only」；
//!   缓存缺失/无命中才回退 det+rec（S-03：det+rec P95=968ms 超阈值，rec-only P95=596ms 达标）；
//! - 选区过滤 / 文字存在判定 / 文本合并（纯逻辑，可单测）。
//!
//! 坐标约定：引擎 `detect` 返回 **帧局部物理像素**（det 降采样后按 scale 还原）；[`OcrPipeline`]
//! 用 `frame_origin`（虚拟桌面左上角，可负）平移为**绝对物理像素**后再缓存与过滤，因此
//! 多屏负坐标选区可直接与缓存框做相交判定。
//!
//! TODO(real): RapidOCR ONNX
//! 1) 整屏 det：按 [`det_input_size`] 双约束降采样，后台单 worker 串行、intra_op 6–8；
//! 2) 选区 rec：按 `rectPhys` 从缓存框裁剪原图（必要时 2× 放大），批量 rec；
//! 3) 置信度阈值 `ocr.textThreshold`；新会话 / 取消时作废旧 `ocr_session_id`，丢弃迟到结果。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::contract::errors::AppResult;
use crate::contract::types::{OcrBox, RectPhys};

use crate::platform::capture::FrozenFrame;

/// 契约 §12：`ocr:status(scanning)` 的延迟（毫秒）。
pub const SKELETON_SCANNING_DELAY_MS: u64 = 150;
/// 契约 §12：`ocr:status(done)` 的总延迟（毫秒）。
pub const SKELETON_DONE_DELAY_MS: u64 = 400;
/// 契约 §12：骨架模式占位识别文本。
pub const SKELETON_PREVIEW: &str = "【骨架模式】本地 OCR 未接入，此为占位识别文本。";
/// 骨架模式占位框置信度。
pub const SKELETON_CONFIDENCE: f64 = 0.99;

/// S-02 det 输入**像素预算**（宽×高上限，物理像素）。
///
/// 依据：本机实测 det 吞吐 ≈3.7–4.0 Mpx/s；只按长边 ≤3200 时双 4K 被压到 3200x900（2.88 Mpx，
/// P95=926ms 达标），而单 4K 是 3200x1800（5.76 Mpx，P95=1695ms 超阈值）。叠加 3.0 Mpx 预算后
/// 单 4K → 约 2309x1299，把 det 计算量拉回与双 4K 同级。详见
/// `docs/spikes/S-02-S-03-OCR时延基准.md` §10.1。
pub const DET_PIXEL_BUDGET: u64 = 3_000_000;

/// 计算 det 输入尺寸：**双约束取小、绝不放大**。
///
/// 约束 1：长边 ≤ `det_max_edge`（配置 `ocr.detMaxEdge`，默认 3200）；
/// 约束 2：宽 × 高 ≤ [`DET_PIXEL_BUDGET`]（3.0 Mpx）。
///
/// 返回 `(out_w, out_h, scale)`，其中 `scale` 为「帧 → det 输入」的缩放比（≤1），
/// det 输出的框坐标除以 `scale`（或乘以 `1/scale`）即可还原为帧局部物理像素。
/// 取整用 `floor` 保证不放大、不超预算；因此 `out_h` 与 `frame_h*scale` 可能相差 ≤1px。
pub fn det_input_size(frame_w: u32, frame_h: u32, det_max_edge: u32) -> (u32, u32, f64) {
    let frame_w = frame_w.max(1);
    let frame_h = frame_h.max(1);
    let max_edge = det_max_edge.max(1);

    let long_edge = frame_w.max(frame_h) as f64;
    let mut scale = if long_edge > max_edge as f64 {
        max_edge as f64 / long_edge
    } else {
        1.0
    };
    // 约束 2：像素预算（等比缩放，宽高比不变）。
    let pixels = frame_w as f64 * frame_h as f64 * scale * scale;
    if pixels > DET_PIXEL_BUDGET as f64 {
        scale *= (DET_PIXEL_BUDGET as f64 / pixels).sqrt();
    }

    let out_w = ((frame_w as f64 * scale).floor() as u32).clamp(1, frame_w);
    let out_h = ((frame_h as f64 * scale).floor() as u32).clamp(1, frame_h);
    // 单值缩放比：取两轴较小者，保证还原后的框不会超出帧范围。
    let effective = (out_w as f64 / frame_w as f64).min(out_h as f64 / frame_h as f64);
    (out_w, out_h, effective)
}

/// 一次 OCR 结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OcrScan {
    /// 命中的文字行（**绝对物理像素**坐标；`detect` 层输出为帧局部，由 pipeline 平移）。
    pub boxes: Vec<OcrBox>,
    /// 是否存在文字。
    pub has_text: bool,
    /// 可编辑预览文本。
    pub preview: String,
}

/// OCR 引擎抽象（S-02/S-03：det / rec 拆开，支持缓存复用）。
///
/// - [`OcrEngine::detect`]：整屏检测，只跑 det；返回**帧局部物理像素**框，`text` 为空；
/// - [`OcrEngine::recognize`]：只跑 rec，为给定框补齐 `text`，框坐标原样返回（帧局部）；
/// - [`OcrEngine::detect_and_recognize`]：选区兜底（缓存未命中），det+rec。
#[async_trait]
pub trait OcrEngine: Send + Sync {
    /// 整屏检测（只 det）：`frame` 为冻结帧，返回帧局部物理像素框（`text` 为空）。
    async fn detect(&self, frame: &FrozenFrame, det_max_edge: u32) -> AppResult<Vec<OcrBox>>;

    /// 只做识别：对给定帧局部框补齐 `text`，返回同样的框（顺序与数量保持）。
    async fn recognize(&self, frame: &FrozenFrame, boxes: Vec<OcrBox>) -> AppResult<Vec<OcrBox>>;

    /// 选区兜底：`rect_phys` 为**绝对物理像素**选区，`frame_origin` 为帧左上角绝对坐标
    /// （虚拟桌面左上角，可为负）；返回绝对物理像素框（含文本）。
    async fn detect_and_recognize(
        &self,
        frame: &FrozenFrame,
        frame_origin: &RectPhys,
        rect_phys: &RectPhys,
        det_max_edge: u32,
    ) -> AppResult<Vec<OcrBox>>;
}

/// 骨架 OCR 引擎：瞬时返回占位结果（契约 150/400ms 时序由命令层控制）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SkeletonOcrEngine;

impl SkeletonOcrEngine {
    /// 整屏占位检测框（帧局部坐标，无文本；rec 阶段补占位文本）。
    fn frame_placeholder(frame: &FrozenFrame) -> OcrBox {
        OcrBox {
            text: String::new(),
            confidence: SKELETON_CONFIDENCE,
            bbox: RectPhys {
                x: 0.0,
                y: 0.0,
                width: frame.width as f64,
                height: frame.height as f64,
            },
        }
    }
}

#[async_trait]
impl OcrEngine for SkeletonOcrEngine {
    async fn detect(&self, frame: &FrozenFrame, _det_max_edge: u32) -> AppResult<Vec<OcrBox>> {
        // 占位语义：整帧一个框；宽高即帧尺寸（帧局部物理像素）。
        Ok(vec![Self::frame_placeholder(frame)])
    }

    async fn recognize(&self, _frame: &FrozenFrame, boxes: Vec<OcrBox>) -> AppResult<Vec<OcrBox>> {
        // 占位语义：所有框文本统一为契约固定文案。
        Ok(boxes
            .into_iter()
            .map(|mut item| {
                item.text = SKELETON_PREVIEW.to_string();
                item
            })
            .collect())
    }

    async fn detect_and_recognize(
        &self,
        _frame: &FrozenFrame,
        _frame_origin: &RectPhys,
        rect_phys: &RectPhys,
        _det_max_edge: u32,
    ) -> AppResult<Vec<OcrBox>> {
        // 占位语义：单条覆盖选区的框（与历史 `placeholder_selection` 一致）。
        Ok(vec![placeholder_box(rect_phys)])
    }
}

/// 覆盖整个冻结帧的占位扫描结果（帧局部坐标，骨架模式）。
pub fn full_frame_scan(frame: &FrozenFrame) -> OcrScan {
    let boxes = vec![OcrBox {
        text: SKELETON_PREVIEW.to_string(),
        confidence: SKELETON_CONFIDENCE,
        bbox: RectPhys {
            x: 0.0,
            y: 0.0,
            width: frame.width as f64,
            height: frame.height as f64,
        },
    }];
    OcrScan {
        boxes,
        has_text: true,
        preview: SKELETON_PREVIEW.to_string(),
    }
}

/// 单条覆盖选区的占位框（契约 §12；绝对物理坐标）。
pub fn placeholder_box(rect: &RectPhys) -> OcrBox {
    OcrBox {
        text: SKELETON_PREVIEW.to_string(),
        confidence: SKELETON_CONFIDENCE,
        bbox: rect.normalize(),
    }
}

/// 选区的骨架识别结果（`selection_commit` 兜底路径）。
pub fn placeholder_selection(rect: &RectPhys) -> OcrScan {
    OcrScan {
        boxes: vec![placeholder_box(rect)],
        has_text: true,
        preview: SKELETON_PREVIEW.to_string(),
    }
}

/// 过滤出落在选区内且置信度达标的文字行（纯逻辑；框与选区均为绝对物理坐标）。
pub fn filter_boxes(boxes: &[OcrBox], rect: &RectPhys, min_confidence: f64) -> Vec<OcrBox> {
    let target = rect.normalize();
    boxes
        .iter()
        .filter(|item| item.confidence >= min_confidence)
        .filter(|item| {
            let (center_x, center_y) = item.bbox.center();
            target.contains_point(center_x, center_y) || item.bbox.intersects(&target)
        })
        .cloned()
        .collect()
}

/// 文字存在判定（纯逻辑）：命中框数量 ≥ 1 即认为有文字。
pub fn presence(boxes: &[OcrBox], rect: &RectPhys, min_confidence: f64) -> bool {
    if rect.is_empty() {
        return false;
    }
    !filter_boxes(boxes, rect, min_confidence).is_empty()
}

/// 合并选区内的文字行为预览文本（按行拼接）。
pub fn merge_text(boxes: &[OcrBox]) -> String {
    boxes
        .iter()
        .map(|item| item.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 把框平移 `(dx, dy)`（帧局部 ↔ 绝对物理坐标）。
fn shift_box(item: &OcrBox, dx: f64, dy: f64) -> OcrBox {
    let mut shifted = item.clone();
    shifted.bbox.x += dx;
    shifted.bbox.y += dy;
    shifted
}

/// 以 `ocr_session_id` 为键的整屏检测缓存（重选只重过滤，不重复推理）。
#[derive(Debug, Default)]
pub struct OcrCache {
    entries: Mutex<HashMap<String, Vec<OcrBox>>>,
}

impl OcrCache {
    /// 创建空缓存。
    pub fn new() -> Self {
        Self::default()
    }

    /// 写入整屏检测结果（绝对物理坐标）。
    pub fn put(&self, ocr_session_id: &str, boxes: Vec<OcrBox>) {
        self.entries
            .lock()
            .insert(ocr_session_id.to_string(), boxes);
    }

    /// 读取整屏检测结果（绝对物理坐标）。
    pub fn get(&self, ocr_session_id: &str) -> Option<Vec<OcrBox>> {
        self.entries.lock().get(ocr_session_id).cloned()
    }

    /// 作废某会话缓存（新会话 / 取消）。
    pub fn invalidate(&self, ocr_session_id: &str) {
        self.entries.lock().remove(ocr_session_id);
    }

    /// 清空缓存（新会话 / 关闭结果窗 / 退出）。
    pub fn clear(&self) {
        self.entries.lock().clear();
    }

    /// 缓存条目数。
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }
}

/// OCR 流水线：整屏 det 缓存 + 选区 rec-only（S-03）+ 兜底 det+rec。
///
/// 生命周期：`AppState` 持有一个实例；缓存键为 `Session::ocr_session_id`（每个新会话生成新键），
/// 新 `capture_freeze` 时由命令层 `clear()` 作废旧缓存，避免旧帧的框污染新会话。
pub struct OcrPipeline {
    engine: Arc<dyn OcrEngine>,
    cache: OcrCache,
}

impl OcrPipeline {
    /// 用指定引擎创建流水线。
    pub fn new(engine: Arc<dyn OcrEngine>) -> Self {
        Self {
            engine,
            cache: OcrCache::new(),
        }
    }

    /// 骨架模式流水线。
    pub fn with_skeleton() -> Self {
        Self::new(Arc::new(SkeletonOcrEngine))
    }

    /// 真实 OCR 流水线（契约 v5 / spec §3.4）。
    ///
    /// 引擎为 [`crate::services::ocr_onnx::OnnxOcrEngine`]（或测试用 fake）；流水线本身只负责
    /// det 缓存 / 选区过滤 / rec-only，与引擎实现无关。
    pub fn with_real(engine: Box<dyn OcrEngine>) -> Self {
        Self::new(Arc::from(engine))
    }

    /// 当前引擎（供诊断 / 测试断言使用）。
    pub fn engine(&self) -> &Arc<dyn OcrEngine> {
        &self.engine
    }

    /// 整屏检测缓存（只读访问，供观察/测试）。
    pub fn cache(&self) -> &OcrCache {
        &self.cache
    }

    /// 作废某会话的整屏缓存。
    pub fn invalidate(&self, ocr_session_id: &str) {
        self.cache.invalidate(ocr_session_id);
    }

    /// 清空全部缓存（新会话开始时调用）。
    pub fn clear(&self) {
        self.cache.clear();
    }

    /// 整屏扫描：只 det → 帧局部框平移为绝对物理像素 → 按 `ocr_session_id` 缓存。
    ///
    /// 返回的 `OcrScan.boxes` 为绝对物理坐标（`preview` 在 det-only 阶段通常为空）。
    pub async fn scan_screen(
        &self,
        ocr_session_id: &str,
        frame: &FrozenFrame,
        frame_origin: &RectPhys,
        det_max_edge: u32,
        min_confidence: f64,
    ) -> AppResult<OcrScan> {
        let local = self.engine.detect(frame, det_max_edge).await?;
        let origin = frame_origin.normalize();
        let boxes: Vec<OcrBox> = local
            .iter()
            .map(|item| shift_box(item, origin.x, origin.y))
            .collect();
        self.cache.put(ocr_session_id, boxes.clone());
        let boxes = filter_boxes(&boxes, &frame_rect(frame_origin, frame), min_confidence);
        Ok(OcrScan {
            has_text: !boxes.is_empty(),
            preview: merge_text(&boxes),
            boxes,
        })
    }

    /// 选区识别（S-03 核心路径）：
    ///
    /// 1. 缓存命中且有框与选区相交 → **只做 rec**（不重复 det，实测 P95 596ms）；
    /// 2. 缓存缺失 / 无命中 / 空框 → 回退 `detect_and_recognize`（选区 det+rec，约 1s 级降级）。
    ///
    /// 返回的 `boxes` 为绝对物理坐标，已按 `min_confidence` 过滤。
    pub async fn recognize_selection(
        &self,
        ocr_session_id: &str,
        frame: &FrozenFrame,
        frame_origin: &RectPhys,
        rect_phys: &RectPhys,
        det_max_edge: u32,
        min_confidence: f64,
    ) -> AppResult<OcrScan> {
        let rect = rect_phys.normalize();
        // 取消 / 空选：不触发任何推理。
        if rect.is_empty() {
            return Ok(OcrScan::default());
        }
        let origin = frame_origin.normalize();

        if let Some(cached) = self.cache.get(ocr_session_id) {
            let picked = filter_boxes(&cached, &rect, min_confidence);
            if !picked.is_empty() {
                // 缓存路径：只跑 rec（框转回帧局部给引擎，识别后再平移回绝对坐标）。
                let local: Vec<OcrBox> = picked
                    .iter()
                    .map(|item| shift_box(item, -origin.x, -origin.y))
                    .collect();
                let recognized = self.engine.recognize(frame, local).await?;
                let boxes: Vec<OcrBox> = recognized
                    .iter()
                    .map(|item| shift_box(item, origin.x, origin.y))
                    .filter(|item| item.confidence >= min_confidence)
                    .collect();
                if !boxes.is_empty() {
                    return Ok(OcrScan {
                        has_text: true,
                        preview: merge_text(&boxes),
                        boxes,
                    });
                }
            }
        }

        // 兜底：缓存缺失 / 无命中（骨架引擎瞬时；真实引擎约 1s 级）。
        let boxes = self
            .engine
            .detect_and_recognize(frame, &origin, &rect, det_max_edge)
            .await?;
        let boxes = filter_boxes(&boxes, &rect, min_confidence);
        Ok(OcrScan {
            has_text: !boxes.is_empty(),
            preview: merge_text(&boxes),
            boxes,
        })
    }
}

/// 帧在绝对物理坐标系中的矩形（用于整屏扫描结果的置信度过滤）。
fn frame_rect(frame_origin: &RectPhys, frame: &FrozenFrame) -> RectPhys {
    let origin = frame_origin.normalize();
    RectPhys {
        x: origin.x,
        y: origin.y,
        width: frame.width as f64,
        height: frame.height as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RectPhys {
        RectPhys {
            x,
            y,
            width,
            height,
        }
    }

    fn ocr_box(text: &str, confidence: f64, bbox: RectPhys) -> OcrBox {
        OcrBox {
            text: text.to_string(),
            confidence,
            bbox,
        }
    }

    fn frame(width: u32, height: u32) -> FrozenFrame {
        // T2 起 `FrozenFrame` 增加 `bgra: Option<Arc<Vec<u8>>>`；骨架帧无像素数据。
        FrozenFrame::skeleton(width, height)
    }

    // ───────── det 像素预算（S-02） ─────────

    #[test]
    fn det_input_size_dual_4k_uses_long_edge_budget() {
        // 7680x2160：长边 3200 → 3200x900（2.88 Mpx）与 S-02 主用例完全一致。
        let (w, h, scale) = det_input_size(7680, 2160, 3200);
        assert_eq!((w, h), (3200, 900));
        assert!(scale < 1.0);
        assert_eq!((w * h) as u64, 2_880_000);
        assert!((w * h) as u64 <= DET_PIXEL_BUDGET);
    }

    #[test]
    fn det_input_size_single_4k_is_pixel_budget_bound() {
        // 3840x2160：只按长边会得到 3200x1800=5.76Mpx（S-02 实测 1695ms 超阈值），
        // 叠加 3.0Mpx 预算后应约 2309x1299。
        let (w, h, scale) = det_input_size(3840, 2160, 3200);
        assert_eq!((w, h), (2309, 1299));
        assert!((w * h) as u64 <= DET_PIXEL_BUDGET);
        assert!(w < 3200 && h < 1800);
        // 缩放比是「帧 → det 输入」的比例，不放大。
        assert!(scale > 0.0 && scale <= 1.0);
        assert!((w as f64 / 3840.0 - scale).abs() < 1e-9);
    }

    #[test]
    fn det_input_size_2560x1440_shrinks_under_budget() {
        let (w, h, _scale) = det_input_size(2560, 1440, 3200);
        assert!(w <= 2560 && h <= 1440);
        assert!((w * h) as u64 <= DET_PIXEL_BUDGET);
        // 3.69 Mpx → 3.0 Mpx：约 0.902x。
        assert!((w as f64 / 2560.0 - 0.9019).abs() < 0.01);
        assert!((h as f64 / 1440.0 - 0.9021).abs() < 0.01);
    }

    #[test]
    fn det_input_size_never_upscales_small_frames() {
        for (fw, fh) in [(320u32, 200u32), (800, 600), (1920, 1080), (1, 1)] {
            let (w, h, scale) = det_input_size(fw, fh, 3200);
            assert_eq!((w, h), (fw, fh), "小帧不应放大：{fw}x{fh}");
            assert_eq!(scale, 1.0);
        }
    }

    #[test]
    fn det_input_size_ultra_wide_and_tall() {
        // 32:9 超宽：长边压到 3200 → 3200x900（2.88 Mpx，预算内）。
        let (w, h, _) = det_input_size(5120, 1440, 3200);
        assert_eq!((w, h), (3200, 900));
        // 9:16 超长（竖屏拼屏）：长边压到 3200 后仍超预算，再按预算缩到 1299x2309。
        let (w, h, _) = det_input_size(2160, 3840, 3200);
        assert!((w * h) as u64 <= DET_PIXEL_BUDGET);
        assert!(w.max(h) <= 3200);
        assert!(w < h, "竖长帧应保持宽 < 高");
    }

    #[test]
    fn det_input_size_respects_edge_and_budget_for_all_real_screens() {
        // 覆盖常见屏组合：不放大、长边 ≤ det_max_edge、像素 ≤ 预算、缩放比一致。
        let frames = [
            (2560u32, 1440u32), // 2K
            (3840, 2160),       // 单 4K
            (7680, 2160),       // 双 4K 横排
            (4320, 7680),       // 双 4K 竖排极端
            (3640, 1920),       // 本机混合 DPI 虚拟桌面
            (1280, 800),
        ];
        for (fw, fh) in frames {
            for max_edge in [1600u32, 3200, 4800] {
                let (w, h, scale) = det_input_size(fw, fh, max_edge);
                assert!(w <= fw && h <= fh, "不放大 {fw}x{fh}@{max_edge}");
                assert!(w.max(h) <= max_edge, "长边约束 {fw}x{fh}@{max_edge}");
                assert!(
                    (w * h) as u64 <= DET_PIXEL_BUDGET,
                    "像素预算 {fw}x{fh}@{max_edge} -> {w}x{h}"
                );
                assert!(scale > 0.0 && scale <= 1.0);
                // scale 取两轴较小者：不超过任一轴的实际比例，且与实际比例相差 <1%。
                assert!(scale <= w as f64 / fw as f64 + 1e-9);
                assert!(scale <= h as f64 / fh as f64 + 1e-9);
                assert!((w as f64 / fw as f64 - scale) < 0.01);
            }
        }
    }

    // ───────── 骨架引擎 / 纯逻辑 ─────────

    #[test]
    fn placeholder_covers_selection_and_reports_text() {
        let selection = rect(10.0, 20.0, 100.0, 50.0);
        let scan = placeholder_selection(&selection);
        assert!(scan.has_text);
        assert_eq!(scan.boxes.len(), 1);
        assert_eq!(scan.boxes[0].bbox, selection);
        assert_eq!(scan.preview, SKELETON_PREVIEW);
        assert_eq!(SKELETON_SCANNING_DELAY_MS, 150);
        assert_eq!(SKELETON_DONE_DELAY_MS, 400);
    }

    #[test]
    fn filters_boxes_by_selection_and_confidence() {
        let selection = rect(0.0, 0.0, 100.0, 100.0);
        let boxes = vec![
            ocr_box("inside", 0.9, rect(10.0, 10.0, 30.0, 20.0)),
            ocr_box("outside", 0.9, rect(500.0, 500.0, 30.0, 20.0)),
            ocr_box("low", 0.2, rect(10.0, 40.0, 30.0, 20.0)),
        ];
        let kept = filter_boxes(&boxes, &selection, 0.6);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].text, "inside");
        assert!(presence(&boxes, &selection, 0.6));
        assert!(!presence(&boxes, &rect(900.0, 900.0, 10.0, 10.0), 0.6));
    }

    #[test]
    fn negative_selection_coordinates_are_supported() {
        // 副屏位于主屏左侧时的负坐标选区。
        let selection = rect(-500.0, -200.0, 300.0, 150.0);
        let boxes = vec![ocr_box("负坐标", 0.9, rect(-480.0, -180.0, 100.0, 40.0))];
        assert!(presence(&boxes, &selection, 0.6));
        assert_eq!(merge_text(&filter_boxes(&boxes, &selection, 0.6)), "负坐标");
    }

    #[test]
    fn empty_selection_has_no_text() {
        let boxes = vec![ocr_box("x", 0.9, rect(0.0, 0.0, 10.0, 10.0))];
        assert!(!presence(&boxes, &rect(10.0, 10.0, 0.0, 20.0), 0.6));
    }

    #[test]
    fn cache_is_keyed_by_ocr_session() {
        let cache = OcrCache::new();
        assert!(cache.is_empty());
        cache.put("ocr-1", vec![placeholder_box(&rect(0.0, 0.0, 10.0, 10.0))]);
        assert_eq!(cache.len(), 1);
        assert!(cache.get("ocr-1").is_some());
        assert!(cache.get("ocr-2").is_none());
        cache.invalidate("ocr-1");
        assert!(cache.is_empty());
        cache.put("ocr-3", Vec::new());
        cache.clear();
        assert!(cache.is_empty());
    }

    // ───────── OcrPipeline（fake engine） ─────────

    /// 可注入 fake engine：记录各路径调用次数，返回受控框。
    struct FakeEngine {
        detect_calls: AtomicUsize,
        recognize_calls: AtomicUsize,
        fallback_calls: AtomicUsize,
        detected: Vec<OcrBox>,
        fallback: Vec<OcrBox>,
    }

    impl FakeEngine {
        fn new(detected: Vec<OcrBox>, fallback: Vec<OcrBox>) -> Self {
            Self {
                detect_calls: AtomicUsize::new(0),
                recognize_calls: AtomicUsize::new(0),
                fallback_calls: AtomicUsize::new(0),
                detected,
                fallback,
            }
        }
    }

    #[async_trait]
    impl OcrEngine for FakeEngine {
        async fn detect(&self, _frame: &FrozenFrame, _det_max_edge: u32) -> AppResult<Vec<OcrBox>> {
            self.detect_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.detected.clone())
        }

        async fn recognize(
            &self,
            _frame: &FrozenFrame,
            boxes: Vec<OcrBox>,
        ) -> AppResult<Vec<OcrBox>> {
            self.recognize_calls.fetch_add(1, Ordering::SeqCst);
            Ok(boxes
                .into_iter()
                .map(|mut item| {
                    item.text = format!("rec:{}", item.bbox.x);
                    item
                })
                .collect())
        }

        async fn detect_and_recognize(
            &self,
            _frame: &FrozenFrame,
            _frame_origin: &RectPhys,
            rect_phys: &RectPhys,
            _det_max_edge: u32,
        ) -> AppResult<Vec<OcrBox>> {
            self.fallback_calls.fetch_add(1, Ordering::SeqCst);
            // 兜底语义：选区 det+rec 的框落在选区内（bbox 用选区替换，便于断言路径）。
            Ok(self
                .fallback
                .iter()
                .map(|item| {
                    let mut cloned = item.clone();
                    cloned.bbox = *rect_phys;
                    cloned
                })
                .collect())
        }
    }

    fn pipeline_with_fake() -> (OcrPipeline, Arc<FakeEngine>) {
        let engine = Arc::new(FakeEngine::new(
            // 帧局部坐标：两个字框，整屏 det 缓存时会平移 origin (-1080,-470)。
            vec![
                ocr_box("", 0.9, rect(1100.0, 500.0, 200.0, 40.0)),
                ocr_box("", 0.9, rect(3000.0, 1500.0, 200.0, 40.0)),
            ],
            vec![ocr_box("兜底", 0.99, rect(-500.0, -200.0, 100.0, 40.0))],
        ));
        let pipeline = OcrPipeline::new(engine.clone());
        (pipeline, engine)
    }

    #[tokio::test]
    async fn pipeline_scan_translates_boxes_to_absolute_and_caches() {
        let (pipeline, engine) = pipeline_with_fake();
        let origin = rect(-1080.0, -470.0, 3640.0, 1920.0);
        let scan = pipeline
            .scan_screen("ocr-1", &frame(3640, 1920), &origin, 3200, 0.6)
            .await
            .expect("扫描成功");
        assert_eq!(engine.detect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(scan.boxes.len(), 2);
        // 帧局部 (1100,500) + origin (-1080,-470) = 绝对 (20,30)。
        assert_eq!(scan.boxes[0].bbox.x, 20.0);
        assert_eq!(scan.boxes[0].bbox.y, 30.0);
        assert_eq!(pipeline.cache().get("ocr-1").map(|b| b.len()), Some(2));
    }

    #[tokio::test]
    async fn pipeline_selection_hit_uses_rec_only_without_det() {
        let (pipeline, engine) = pipeline_with_fake();
        let origin = rect(-1080.0, -470.0, 3640.0, 1920.0);
        pipeline
            .scan_screen("ocr-1", &frame(3640, 1920), &origin, 3200, 0.6)
            .await
            .expect("扫描成功");
        // 选区落在第一个框（绝对 (20,30) 200x40）内。
        let scan = pipeline
            .recognize_selection(
                "ocr-1",
                &frame(3640, 1920),
                &origin,
                &rect(0.0, 0.0, 300.0, 200.0),
                3200,
                0.6,
            )
            .await
            .expect("识别成功");
        assert_eq!(
            engine.detect_calls.load(Ordering::SeqCst),
            1,
            "命中缓存不再 det"
        );
        assert_eq!(engine.fallback_calls.load(Ordering::SeqCst), 0);
        assert_eq!(engine.recognize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(scan.boxes.len(), 1);
        assert_eq!(scan.boxes[0].bbox.x, 20.0);
        assert!(scan.boxes[0].text.starts_with("rec:"));
        assert!(scan.has_text);
    }

    #[tokio::test]
    async fn pipeline_selection_filters_intersection_and_confidence() {
        let (pipeline, _engine) = pipeline_with_fake();
        let origin = rect(0.0, 0.0, 3640.0, 1920.0);
        pipeline
            .scan_screen("ocr-1", &frame(3640, 1920), &origin, 3200, 0.6)
            .await
            .expect("扫描成功");
        // 选区与第一个框相交（不包含中心）→ intersects 命中；第二个框不相关。
        let scan = pipeline
            .recognize_selection(
                "ocr-1",
                &frame(3640, 1920),
                &origin,
                &rect(1150.0, 520.0, 40.0, 10.0),
                3200,
                0.6,
            )
            .await
            .expect("识别成功");
        assert_eq!(scan.boxes.len(), 1);
        assert_eq!(scan.boxes[0].bbox.x, 1100.0);
        // 置信度阈值提高到 0.99 → 缓存框全部被过滤 → 走兜底路径。
        let scan = pipeline
            .recognize_selection(
                "ocr-1",
                &frame(3640, 1920),
                &origin,
                &rect(1150.0, 520.0, 40.0, 10.0),
                3200,
                0.99,
            )
            .await
            .expect("识别成功");
        assert_eq!(scan.boxes[0].text, "兜底");
    }

    #[tokio::test]
    async fn pipeline_falls_back_when_cache_missing_or_empty() {
        let (pipeline, engine) = pipeline_with_fake();
        let origin = rect(0.0, 0.0, 3640.0, 1920.0);
        // 未扫描 → 无缓存 → 兜底。
        let scan = pipeline
            .recognize_selection(
                "ocr-miss",
                &frame(3640, 1920),
                &origin,
                &rect(0.0, 0.0, 100.0, 100.0),
                3200,
                0.6,
            )
            .await
            .expect("兜底成功");
        assert_eq!(engine.fallback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(scan.boxes[0].text, "兜底");

        // 扫描结果为空框 → 缓存命中但无框 → 仍兜底。
        let (pipeline, engine) = {
            let engine = Arc::new(FakeEngine::new(
                Vec::new(),
                vec![placeholder_box(&rect(0.0, 0.0, 10.0, 10.0))],
            ));
            (OcrPipeline::new(engine.clone()), engine)
        };
        pipeline
            .scan_screen("ocr-empty", &frame(3640, 1920), &origin, 3200, 0.6)
            .await
            .expect("扫描成功");
        let scan = pipeline
            .recognize_selection(
                "ocr-empty",
                &frame(3640, 1920),
                &origin,
                &rect(0.0, 0.0, 100.0, 100.0),
                3200,
                0.6,
            )
            .await
            .expect("兜底成功");
        assert_eq!(engine.fallback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(engine.recognize_calls.load(Ordering::SeqCst), 0);
        assert!(scan.has_text);
    }

    #[tokio::test]
    async fn pipeline_new_session_does_not_reuse_old_cache() {
        let (pipeline, engine) = pipeline_with_fake();
        let origin = rect(0.0, 0.0, 3640.0, 1920.0);
        pipeline
            .scan_screen("ocr-old", &frame(3640, 1920), &origin, 3200, 0.6)
            .await
            .expect("扫描成功");
        // 新会话键（新 catch_freeze 会生成新的 ocr_session_id）→ 不命中旧缓存。
        let scan = pipeline
            .recognize_selection(
                "ocr-new",
                &frame(3640, 1920),
                &origin,
                &rect(0.0, 0.0, 300.0, 200.0),
                3200,
                0.6,
            )
            .await
            .expect("兜底成功");
        assert_eq!(engine.fallback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(engine.recognize_calls.load(Ordering::SeqCst), 0);
        assert_eq!(scan.boxes[0].text, "兜底");
    }

    #[tokio::test]
    async fn pipeline_invalidate_and_empty_rect_are_no_ops() {
        let (pipeline, engine) = pipeline_with_fake();
        let origin = rect(0.0, 0.0, 3640.0, 1920.0);
        pipeline
            .scan_screen("ocr-1", &frame(3640, 1920), &origin, 3200, 0.6)
            .await
            .expect("扫描成功");
        pipeline.invalidate("ocr-1");
        assert!(pipeline.cache().is_empty());
        // 空选区（取消/无效框）：不触发任何推理。
        let scan = pipeline
            .recognize_selection(
                "ocr-1",
                &frame(3640, 1920),
                &origin,
                &rect(10.0, 10.0, 0.0, 20.0),
                3200,
                0.6,
            )
            .await
            .expect("空选区成功");
        assert_eq!(engine.fallback_calls.load(Ordering::SeqCst), 0);
        assert!(!scan.has_text && scan.boxes.is_empty());
    }

    #[tokio::test]
    async fn pipeline_skeleton_scan_then_selection_keeps_placeholder_semantics() {
        // 真实骨架引擎：整屏 det → 缓存 → 选区 rec-only → 占位文本 / hasText，与历史行为一致。
        let pipeline = OcrPipeline::with_skeleton();
        let origin = rect(-1080.0, -470.0, 3640.0, 1920.0);
        let scan = pipeline
            .scan_screen("ocr-skeleton", &frame(3640, 1920), &origin, 3200, 0.6)
            .await
            .expect("扫描成功");
        assert!(scan.has_text);
        let selection = pipeline
            .recognize_selection(
                "ocr-skeleton",
                &frame(3640, 1920),
                &origin,
                &rect(-500.0, -200.0, 300.0, 150.0),
                3200,
                0.6,
            )
            .await
            .expect("识别成功");
        assert!(selection.has_text);
        assert_eq!(selection.preview, SKELETON_PREVIEW);
        assert_eq!(selection.boxes.len(), 1);
        // 没有缓存时兜底也返回占位选区框。
        let pipeline = OcrPipeline::with_skeleton();
        let selection = pipeline
            .recognize_selection(
                "ocr-fallback",
                &frame(3640, 1920),
                &origin,
                &rect(-500.0, -200.0, 300.0, 150.0),
                3200,
                0.6,
            )
            .await
            .expect("兜底成功");
        assert_eq!(selection.boxes[0].bbox, rect(-500.0, -200.0, 300.0, 150.0));
    }
}
