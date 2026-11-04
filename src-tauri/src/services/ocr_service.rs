//! 前置 OCR（M05 / 契约 §12）。
//!
//! 骨架模式行为：150ms 后 `ocr:status(scanning)`、400ms 后 `done`；`boxes` 为单条覆盖
//! 选区的占位框；`hasText = true`；`preview` 为契约固定文案。
//!
//! 本模块提供：
//! - [`OcrEngine`] trait + [`SkeletonOcrEngine`]（真实引擎见 `TODO(real)`）；
//! - 选区过滤 / 文字存在判定（纯逻辑，可单测）；
//! - 以 `ocr_session_id` 为键的整屏检测缓存。
//!
//! TODO(real): RapidOCR ONNX
//! 1) 整屏 det：长边降采样至 `ocr.detMaxEdge`，后台单 worker 串行；
//! 2) 选区 det + rec：按 `rectPhys` 裁剪原图（必要时 2× 放大）；
//! 3) 置信度阈值 `ocr.textThreshold`，覆盖面积占比 ≥ 阈值判定有文字；
//! 4) 新会话 / 取消时作废旧 `ocr_session_id`，丢弃迟到结果。

use std::collections::HashMap;
use std::time::Duration;

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

/// 一次 OCR 结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OcrScan {
    /// 命中的文字行。
    pub boxes: Vec<OcrBox>,
    /// 是否存在文字。
    pub has_text: bool,
    /// 可编辑预览文本。
    pub preview: String,
}

/// OCR 引擎抽象。
#[async_trait]
pub trait OcrEngine: Send + Sync {
    /// 整屏预扫描（只跑检测）。
    async fn scan(&self, frame: &FrozenFrame) -> AppResult<OcrScan>;
}

/// 骨架 OCR 引擎：等待契约规定的时长后返回整屏占位框。
#[derive(Debug, Clone, Copy, Default)]
pub struct SkeletonOcrEngine;

#[async_trait]
impl OcrEngine for SkeletonOcrEngine {
    async fn scan(&self, frame: &FrozenFrame) -> AppResult<OcrScan> {
        // 契约 §12：150ms 后 scanning、400ms 后 done；这里等待总时长。
        tokio::time::sleep(Duration::from_millis(SKELETON_DONE_DELAY_MS)).await;
        Ok(full_frame_scan(frame))
    }
}

/// 覆盖整个冻结帧的占位框。
pub fn full_frame_scan(frame: &FrozenFrame) -> OcrScan {
    let rect = RectPhys {
        x: 0.0,
        y: 0.0,
        width: frame.width as f64,
        height: frame.height as f64,
    };
    OcrScan {
        boxes: vec![placeholder_box(&rect)],
        has_text: true,
        preview: SKELETON_PREVIEW.to_string(),
    }
}

/// 单条覆盖选区的占位框（契约 §12）。
pub fn placeholder_box(rect: &RectPhys) -> OcrBox {
    OcrBox {
        text: SKELETON_PREVIEW.to_string(),
        confidence: SKELETON_CONFIDENCE,
        bbox: rect.normalize(),
    }
}

/// 选区的骨架识别结果（`selection_commit` 使用）。
pub fn placeholder_selection(rect: &RectPhys) -> OcrScan {
    OcrScan {
        boxes: vec![placeholder_box(rect)],
        has_text: true,
        preview: SKELETON_PREVIEW.to_string(),
    }
}

/// 过滤出落在选区内且置信度达标的文字行（纯逻辑）。
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

    /// 写入整屏检测结果。
    pub fn put(&self, ocr_session_id: &str, boxes: Vec<OcrBox>) {
        self.entries
            .lock()
            .insert(ocr_session_id.to_string(), boxes);
    }

    /// 读取整屏检测结果。
    pub fn get(&self, ocr_session_id: &str) -> Option<Vec<OcrBox>> {
        self.entries.lock().get(ocr_session_id).cloned()
    }

    /// 作废某会话缓存（新会话 / 取消）。
    pub fn invalidate(&self, ocr_session_id: &str) {
        self.entries.lock().remove(ocr_session_id);
    }

    /// 清空缓存（关闭结果窗 / 退出）。
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
