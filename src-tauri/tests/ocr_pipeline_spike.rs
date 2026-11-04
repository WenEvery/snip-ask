//! S-02 / S-03 落地集成测试（公开 API 层）：
//! `det_input_size` 像素预算 + `OcrPipeline` 的 det 缓存 / 选区 rec-only / 兜底路径。
//!
//! 这里用**可注入 fake engine** 精确断言「缓存命中时不触发 det」等行为，不依赖真实 ONNX 模型；
//! 真实时延基准见 `docs/spikes/S-02-S-03-OCR时延基准.md`（rec-only P95=596ms 达标）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use snip_ask_lib::contract::types::{OcrBox, RectPhys};
use snip_ask_lib::platform::capture::FrozenFrame;
use snip_ask_lib::services::ocr_service::{
    det_input_size, filter_boxes, merge_text, presence, OcrEngine, OcrPipeline, OcrScan,
    SkeletonOcrEngine, DET_PIXEL_BUDGET, SKELETON_PREVIEW,
};

fn rect(x: f64, y: f64, width: f64, height: f64) -> RectPhys {
    RectPhys {
        x,
        y,
        width,
        height,
    }
}

fn frame(width: u32, height: u32) -> FrozenFrame {
    FrozenFrame {
        width,
        height,
        data_url: None,
    }
}

fn ocr_box(text: &str, confidence: f64, bbox: RectPhys) -> OcrBox {
    OcrBox {
        text: text.to_string(),
        confidence,
        bbox,
    }
}

// ───────── 1) 像素预算（S-02 实施边界） ─────────

#[test]
fn pixel_budget_matches_s02_measurements() {
    // 双 4K 7680x2160 → 3200x900（2.88 Mpx），与 S-02 主用例输入一致。
    let (w, h, scale) = det_input_size(7680, 2160, 3200);
    assert_eq!((w, h), (3200, 900));
    assert!((w * h) <= 3_000_000);
    assert!(scale <= 1.0);

    // 单 4K 3840x2160 → 约 2309x1299（3.0 Mpx），修正「按长边」导致的 5.76 Mpx 超时路径。
    let (w, h, _) = det_input_size(3840, 2160, 3200);
    assert_eq!((w, h), (2309, 1299));
    assert!((w * h) as u64 <= DET_PIXEL_BUDGET);
}

#[test]
fn pixel_budget_covers_real_and_extreme_screens() {
    let cases = [
        (2560u32, 1440u32),
        (3840, 2160),
        (7680, 2160),
        (5160, 2160),
        (2160, 3840),
        (3640, 1920),
        (1024, 768),
        (640, 480),
    ];
    for (fw, fh) in cases {
        for det_max_edge in [1600u32, 3200, 4800] {
            let (w, h, scale) = det_input_size(fw, fh, det_max_edge);
            assert!(w >= 1 && h >= 1);
            assert!(w <= fw && h <= fh, "不得放大：{fw}x{fh} @ {det_max_edge}");
            assert!(
                w.max(h) <= det_max_edge,
                "长边约束失效：{fw}x{fh} @ {det_max_edge} -> {w}x{h}"
            );
            assert!(
                (w * h) as u64 <= DET_PIXEL_BUDGET,
                "像素预算超限：{fw}x{fh} @ {det_max_edge} -> {w}x{h} = {}",
                w as u64 * h as u64
            );
            assert!(scale > 0.0 && scale <= 1.0);
        }
    }
}

// ───────── 2) fake engine（记录调用路径） ─────────

struct FakeEngine {
    det_calls: AtomicUsize,
    rec_calls: AtomicUsize,
    fallback_calls: AtomicUsize,
    detected: Vec<OcrBox>,
    fallback: Vec<OcrBox>,
}

impl FakeEngine {
    fn new(detected: Vec<OcrBox>, fallback: Vec<OcrBox>) -> Self {
        Self {
            det_calls: AtomicUsize::new(0),
            rec_calls: AtomicUsize::new(0),
            fallback_calls: AtomicUsize::new(0),
            detected,
            fallback,
        }
    }
}

#[async_trait]
impl OcrEngine for FakeEngine {
    async fn detect(
        &self,
        _frame: &FrozenFrame,
        _det_max_edge: u32,
    ) -> snip_ask_lib::contract::errors::AppResult<Vec<OcrBox>> {
        self.det_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.detected.clone())
    }

    async fn recognize(
        &self,
        _frame: &FrozenFrame,
        boxes: Vec<OcrBox>,
    ) -> snip_ask_lib::contract::errors::AppResult<Vec<OcrBox>> {
        self.rec_calls.fetch_add(1, Ordering::SeqCst);
        Ok(boxes
            .into_iter()
            .map(|mut item| {
                item.text = format!("rec@{:.0}", item.bbox.x);
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
    ) -> snip_ask_lib::contract::errors::AppResult<Vec<OcrBox>> {
        self.fallback_calls.fetch_add(1, Ordering::SeqCst);
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

fn fake_pipeline() -> (OcrPipeline, Arc<FakeEngine>) {
    let engine = Arc::new(FakeEngine::new(
        // 帧局部坐标（origin (-1080,-470) 平移后为绝对坐标）
        vec![
            ocr_box("", 0.95, rect(1100.0, 500.0, 200.0, 40.0)),
            ocr_box("", 0.95, rect(3000.0, 1500.0, 200.0, 40.0)),
        ],
        // 兜底结果置信度取 0.99：即使调用方把阈值提到 0.99 也能通过过滤（用于断言兜底路径）。
        vec![ocr_box("兜底文本", 0.99, rect(0.0, 0.0, 10.0, 10.0))],
    ));
    (OcrPipeline::new(engine.clone()), engine)
}

const ORIGIN_X: f64 = -1080.0;
const ORIGIN_Y: f64 = -470.0;

fn origin() -> RectPhys {
    rect(ORIGIN_X, ORIGIN_Y, 3640.0, 1920.0)
}

// ───────── 3) pipeline：缓存复用 / 过滤 / 兜底 / 会话失效 ─────────

#[tokio::test]
async fn scan_caches_absolute_boxes() {
    let (pipeline, engine) = fake_pipeline();
    let scan = pipeline
        .scan_screen("ocr-a", &frame(3640, 1920), &origin(), 3200, 0.6)
        .await
        .expect("整屏 det");
    assert_eq!(engine.det_calls.load(Ordering::SeqCst), 1);
    assert_eq!(scan.boxes.len(), 2);
    // 帧局部 (1100,500) + (-1080,-470) = 绝对 (20,30)
    assert_eq!(scan.boxes[0].bbox.x, 20.0);
    assert_eq!(scan.boxes[0].bbox.y, 30.0);
    let cached = pipeline.cache().get("ocr-a").expect("已缓存");
    assert_eq!(cached.len(), 2);
}

#[tokio::test]
async fn cache_hit_uses_rec_only_and_never_det() {
    let (pipeline, engine) = fake_pipeline();
    pipeline
        .scan_screen("ocr-a", &frame(3640, 1920), &origin(), 3200, 0.6)
        .await
        .expect("整屏 det");
    let scan = pipeline
        .recognize_selection(
            "ocr-a",
            &frame(3640, 1920),
            &origin(),
            &rect(-500.0, -200.0, 800.0, 500.0),
            3200,
            0.6,
        )
        .await
        .expect("选区识别");
    assert_eq!(
        engine.det_calls.load(Ordering::SeqCst),
        1,
        "命中缓存后不得重复 det"
    );
    assert_eq!(engine.rec_calls.load(Ordering::SeqCst), 1, "应只跑 rec");
    assert_eq!(engine.fallback_calls.load(Ordering::SeqCst), 0);
    assert_eq!(scan.boxes.len(), 1);
    assert!(scan.boxes[0].text.starts_with("rec@"));
    assert!(scan.has_text);
}

#[tokio::test]
async fn selection_filters_by_intersection_and_confidence() {
    let (pipeline, _engine) = fake_pipeline();
    let origin_rect = origin();
    pipeline
        .scan_screen("ocr-a", &frame(3640, 1920), &origin_rect, 3200, 0.6)
        .await
        .expect("整屏 det");
    // 只与第一个框（绝对 (20,30) 200x40）部分相交：选区不含框中心 (120,50)。
    let partial = rect(0.0, -20.0, 100.0, 60.0);
    let scan = pipeline
        .recognize_selection(
            "ocr-a",
            &frame(3640, 1920),
            &origin_rect,
            &partial,
            3200,
            0.6,
        )
        .await
        .expect("识别");
    assert_eq!(scan.boxes.len(), 1);
    assert_eq!(scan.boxes[0].bbox.x, 20.0);

    // 置信度 0.99 → 缓存框（0.95）全被过滤 → 回退 det+rec。
    let scan = pipeline
        .recognize_selection(
            "ocr-a",
            &frame(3640, 1920),
            &origin_rect,
            &partial,
            3200,
            0.99,
        )
        .await
        .expect("兜底");
    assert_eq!(scan.boxes[0].text, "兜底文本");
}

#[tokio::test]
async fn cache_miss_and_empty_cache_fall_back_to_det_and_recognize() {
    let (pipeline, engine) = fake_pipeline();
    let scan = pipeline
        .recognize_selection(
            "ocr-miss",
            &frame(3640, 1920),
            &origin(),
            &rect(0.0, 0.0, 100.0, 100.0),
            3200,
            0.6,
        )
        .await
        .expect("兜底");
    assert_eq!(engine.fallback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(engine.det_calls.load(Ordering::SeqCst), 0);
    assert_eq!(scan.boxes[0].bbox, rect(0.0, 0.0, 100.0, 100.0));
    assert!(scan.has_text);
}

#[tokio::test]
async fn new_session_key_does_not_reuse_old_cache() {
    let (pipeline, engine) = fake_pipeline();
    pipeline
        .scan_screen("ocr-old", &frame(3640, 1920), &origin(), 3200, 0.6)
        .await
        .expect("整屏 det");
    let scan = pipeline
        .recognize_selection(
            "ocr-new",
            &frame(3640, 1920),
            &origin(),
            &rect(0.0, 0.0, 500.0, 500.0),
            3200,
            0.6,
        )
        .await
        .expect("兜底");
    assert_eq!(engine.rec_calls.load(Ordering::SeqCst), 0);
    assert_eq!(engine.fallback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(scan.boxes[0].text, "兜底文本");
}

#[tokio::test]
async fn empty_or_cancelled_selection_triggers_no_inference() {
    let (pipeline, engine) = fake_pipeline();
    pipeline
        .scan_screen("ocr-a", &frame(3640, 1920), &origin(), 3200, 0.6)
        .await
        .expect("整屏 det");
    let scan = pipeline
        .recognize_selection(
            "ocr-a",
            &frame(3640, 1920),
            &origin(),
            &rect(10.0, 10.0, 0.0, 50.0),
            3200,
            0.6,
        )
        .await
        .expect("空选区");
    assert!(!scan.has_text);
    assert!(scan.boxes.is_empty());
    assert_eq!(engine.rec_calls.load(Ordering::SeqCst), 0);
    assert_eq!(engine.fallback_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn skeleton_pipeline_keeps_placeholder_contract_semantics() {
    // 真实骨架引擎走完整路径：整屏 det → 缓存 → 选区 rec-only → 固定占位文案。
    let pipeline = OcrPipeline::new(Arc::new(SkeletonOcrEngine));
    let origin_rect = origin();
    let scan = pipeline
        .scan_screen("ocr-skeleton", &frame(3640, 1920), &origin_rect, 3200, 0.6)
        .await
        .expect("整屏 det");
    assert!(scan.has_text);
    let selected: OcrScan = pipeline
        .recognize_selection(
            "ocr-skeleton",
            &frame(3640, 1920),
            &origin_rect,
            &rect(-500.0, -200.0, 300.0, 150.0),
            3200,
            0.6,
        )
        .await
        .expect("选区识别");
    assert!(selected.has_text);
    assert_eq!(selected.preview, SKELETON_PREVIEW);
    assert_eq!(selected.boxes.len(), 1);
    // 纯逻辑函数在负坐标选区上的行为。
    assert!(presence(
        &selected.boxes,
        &rect(-500.0, -200.0, 300.0, 150.0),
        0.6
    ));
    let filtered = filter_boxes(&selected.boxes, &rect(-500.0, -200.0, 300.0, 150.0), 0.6);
    assert_eq!(merge_text(&filtered), SKELETON_PREVIEW);
}
