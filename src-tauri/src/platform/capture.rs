//! 屏幕捕获（M04）。
//!
//! 抽象为 [`ScreenCapturer`] trait，骨架模式使用 [`SkeletonCapturer`]（不调用 Win32，
//! 用主显示器物理尺寸合成帧，`data_url = None`）。
//!
//! ```text
//! // TODO(real): BitBlt/WGC 捕获虚拟桌面
//! // 1) BitBlt 主路径：GetDC(NULL) + CreateCompatibleBitmap + StretchBlt；
//! // 2) WGC 增强：GraphicsCaptureItem + Direct3D11 回读，支持 HDR / 受保护内容检测；
//! // 3) 命中 DRM / 全屏独占时返回 SA-1001 并建议切换 WGC；
//! // 4) 多显示器：virtual desktop = 各 monitor rect 的外接矩形，坐标为物理像素（可为负）。
//! ```
//!
//! # S-09 黑屏检测与降级（纯 safe 算法）
//!
//! 真实 BitBlt/WGC 调用受 `unsafe_code = "forbid"` 约束，不进产品代码；本模块只承载
//! **可单测的纯函数**：像素亮度统计 → 黑屏判定 → 降级动作 → SA-1001 映射。
//! 真实对比证据见 `docs/spikes/S-09-截图失败降级与WGC对比.md`，关键实测结论：
//!
//! - **WDA_MONITOR(0x01) / 真实 DRM 保护面**：BitBlt 与 WGC 都只能得到纯黑帧
//!   （本次 0x01 实测 900×600 区域 blackRatio=1.000000、均值 0.00、标准差 0.00），
//!   WGC **不能绕过**保护，降级后应返回 SA-1001 并给出可操作提示；
//! - **WDA_EXCLUDEFROMCAPTURE(0x11)**：窗口在截屏中「消失」（露出背后内容，不是黑屏），
//!   WGC 窗口项 **完全取不到帧**（3/3 次 5s 超时）——因此降级链除了「黑帧」还要处理
//!   「空帧 / 未取到帧」；
//! - 黑屏阈值按 **整帧** 定义（`>99% 像素亮度 <16 且均值 <16 且标准差 <6`）。混合 DPI
//!   双屏下若只有主屏被受保护内容覆盖，虚拟桌面整帧黑占比只有 ~76%，整帧判定会漏检；
//!   真实实现应在**目标区域/显示器切片**上调用本判定（见 S-09 记录「处置/遗留」）。

use async_trait::async_trait;

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{FrozenFrameMeta, RectPhys, VirtualDesktop};

use super::coords;

/// 冻结帧：屏幕上某一时刻的静态画面。
#[derive(Debug, Clone, Default)]
pub struct FrozenFrame {
    /// 帧宽（物理像素）。
    pub width: u32,
    /// 帧高（物理像素）。
    pub height: u32,
    /// DataURL；骨架模式为 `None`（前端渲染占位背景）。
    pub data_url: Option<String>,
}

impl FrozenFrame {
    /// 转换为契约中的 `FrozenFrameMeta`。
    pub fn to_meta(&self) -> FrozenFrameMeta {
        FrozenFrameMeta {
            width: self.width,
            height: self.height,
            data_url: self.data_url.clone(),
        }
    }
}

/// 屏幕捕获抽象（真实实现见 `TODO(real)`，保持同一 trait）。
#[async_trait]
pub trait ScreenCapturer: Send + Sync {
    /// 捕获虚拟桌面。
    async fn capture_virtual_desktop(&self) -> AppResult<FrozenFrame>;
}

/// 虚拟桌面几何（S-01：多显示器外接矩形 + overlay 尺寸 / 坐标换算基准）。
///
/// - `rect`：**所有显示器物理矩形的最小外接矩形**，允许负坐标（副屏在主屏左上时本机为
///   `(-1080,-470) 3640x1920`）；与 Win32 `GetSystemMetrics(SM_*VIRTUALSCREEN)` 口径一致。
/// - `dpr`：**预期**窗口缩放比，取覆盖虚拟桌面左上角的显示器的 `scale_factor()`
///   （Windows 决定跨屏窗口 DPI 的归属屏）。overlay 的最终 dpr 必须以窗口实际
///   `scale_factor()` 为准（见 `app::windows::overlay_virtual_desktop`），因为单个跨屏
///   WebView 只有一个 devicePixelRatio。
///
/// TODO(real): 真实捕获按该矩形做 BitBlt / WGC，帧尺寸与本矩形口径保持一致。
pub fn virtual_desktop_geometry(app: &tauri::AppHandle) -> VirtualDesktop {
    let monitors = app.available_monitors().unwrap_or_default();
    let rects: Vec<RectPhys> = monitors
        .iter()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            RectPhys {
                x: position.x as f64,
                y: position.y as f64,
                width: size.width as f64,
                height: size.height as f64,
            }
        })
        .collect();
    let Some(rect) = coords::union_rect(rects) else {
        // 没有任何显示器（远程会话异常）：回退 1080p，保证 overlay 仍可创建。
        return VirtualDesktop {
            rect: RectPhys {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            dpr: 1.0,
        };
    };
    let anchor = monitors
        .iter()
        .find(|monitor| {
            let position = monitor.position();
            coords::rect_contains(&rect, position.x as f64, position.y as f64)
        })
        .or_else(|| monitors.first());
    let dpr = anchor.map(|monitor| monitor.scale_factor()).unwrap_or(1.0);
    coords::virtual_desktop(rect.x, rect.y, rect.width, rect.height, dpr)
}

// ───────────────────────── S-09：黑屏检测 / 降级决策（纯 safe，无 Win32） ─────────────────────────

/// 黑像素的亮度上限（0–255 灰度，含）。纯黑帧的像素亮度为 0，受保护/DRM 黑屏实测为 0。
pub const BLACK_LUMA_MAX: f64 = 16.0;
/// 判黑所需的最小黑像素占比（> 99%），避免深色 UI 被误判。
pub const BLACK_RATIO_MIN: f64 = 0.99;
/// 判黑所需的最大亮度标准差（< 6）；黑底白字/深色图片会因方差过大而不判黑。
pub const BLACK_STD_DEV_MAX: f64 = 6.0;

/// 一帧的亮度统计（灰度口径）。
///
/// 由采样像素（BGRA / RGBA 四字节一组，见 [`luma_stats_from_bgra`]）计算；真实捕获实现
/// 可先按像素预算降采样再调用，避免逐像素统计成为冻结时延瓶颈。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameLumaStats {
    /// 参与统计的像素数（0 表示空帧）。
    pub pixels: usize,
    /// 亮度 `< BLACK_LUMA_MAX` 的像素数。
    pub black_pixels: usize,
    /// 平均亮度（0–255）。
    pub mean_luma: f64,
    /// 亮度总体方差。
    pub variance: f64,
}

impl FrameLumaStats {
    /// 黑像素占比（0.0–1.0）；空帧返回 0。
    pub fn black_ratio(&self) -> f64 {
        if self.pixels == 0 {
            0.0
        } else {
            self.black_pixels as f64 / self.pixels as f64
        }
    }

    /// 亮度标准差。
    pub fn std_dev(&self) -> f64 {
        self.variance.max(0.0).sqrt()
    }

    /// 是否为空帧（没有可统计像素）。
    pub fn is_empty(&self) -> bool {
        self.pixels == 0
    }
}

/// 从 BGRA 像素缓冲（每像素 4 字节：B/G/R/A）统计亮度；不足 4 字节的尾巴忽略。
///
/// 亮度按 BT.601 加权：`0.114*B + 0.587*G + 0.299*R`，与 spike 工具
/// `scripts/spikes/s09/Program.cs` 的判定口径逐字一致（记录里的 blackRatio 可直接对照）。
pub fn luma_stats_from_bgra(pixels: &[u8]) -> FrameLumaStats {
    let mut count = 0usize;
    let mut black_pixels = 0usize;
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    for px in pixels.chunks_exact(4) {
        let luma = 0.114 * f64::from(px[0]) + 0.587 * f64::from(px[1]) + 0.299 * f64::from(px[2]);
        count += 1;
        sum += luma;
        sum_sq += luma * luma;
        if luma < BLACK_LUMA_MAX {
            black_pixels += 1;
        }
    }
    let mean_luma = if count == 0 { 0.0 } else { sum / count as f64 };
    let variance = if count == 0 {
        0.0
    } else {
        (sum_sq / count as f64 - mean_luma * mean_luma).max(0.0)
    };
    FrameLumaStats {
        pixels: count,
        black_pixels,
        mean_luma,
        variance,
    }
}

/// 黑屏判定：黑像素占比 ≥ [`BLACK_RATIO_MIN`]、均值 < [`BLACK_LUMA_MAX`] 且标准差
/// < [`BLACK_STD_DEV_MAX`]。空帧不算黑帧（空帧另由 [`is_capture_failed`] 判失败）。
pub fn is_black_frame(stats: &FrameLumaStats) -> bool {
    !stats.is_empty()
        && stats.black_ratio() >= BLACK_RATIO_MIN
        && stats.mean_luma < BLACK_LUMA_MAX
        && stats.std_dev() < BLACK_STD_DEV_MAX
}

/// 捕获是否失败：空帧（没取到画面）或黑屏。
///
/// 真实降级链中 `None` 表示 WGC 窗口项超时/未取到帧——S-09 实测
/// `WDA_EXCLUDEFROMCAPTURE` 窗口的 WGC 窗口项 3/3 次均为 `None`，因此必须按失败处理。
pub fn is_capture_failed(stats: Option<&FrameLumaStats>) -> bool {
    match stats {
        None => true,
        Some(stats) => stats.is_empty() || is_black_frame(stats),
    }
}

/// 捕获后端（契约 §8.2 的 SA-1001 fallback `["wgc", "retry"]` 对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    /// 主路径：BitBlt 屏幕拷贝。
    BitBlt,
    /// 降级路径：Windows.Graphics.Capture。
    Wgc,
}

impl CaptureBackend {
    /// 契约/诊断用的后端名（`capture.backend` 上下文的值）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BitBlt => "bitblt",
            Self::Wgc => "wgc",
        }
    }
}

/// 单条捕获路径的失败原因（用于填写 SA-1001 的 detail / context）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureFailure {
    /// 空帧 / 未取到画面（如 WGC 窗口项超时）。
    EmptyFrame,
    /// 黑屏（DRM / 受保护内容 / 全屏独占的典型表现）。
    BlackFrame,
}

/// 由亮度统计推导失败原因；`None` 视为空帧。
pub fn classify_failure(stats: Option<&FrameLumaStats>) -> CaptureFailure {
    match stats {
        Some(stats) if !stats.is_empty() && is_black_frame(stats) => CaptureFailure::BlackFrame,
        _ => CaptureFailure::EmptyFrame,
    }
}

/// 构造 SA-1001 捕获失败错误（retryable = true，fallback = `["wgc", "retry"]`）。
///
/// 契约字段由 [`AppError::new`] 按错误码表补全；本函数额外：
/// - 覆盖面向用户的 message 为可操作提示（DRM/全屏独占 + 已尝试 WGC）；
/// - 在 `context` 记录 `capture.backend` 与 `capture.blackFrame` / `capture.blackRatio`，
///   便于日志与前端排查（UI 会显示 `message`，详见 `src/windows/overlay/OverlayWindow.tsx`）。
pub fn capture_failed_error(backend: CaptureBackend, stats: Option<&FrameLumaStats>) -> AppError {
    let failure = classify_failure(stats);
    let reason = match failure {
        CaptureFailure::EmptyFrame => "未能取到画面".to_string(),
        CaptureFailure::BlackFrame => {
            let stats = stats.expect("BlackFrame 必然带有亮度统计");
            format!(
                "画面为黑屏（黑像素占比 {:.1}%，平均亮度 {:.1}）",
                stats.black_ratio() * 100.0,
                stats.mean_luma
            )
        }
    };
    let detail = format!("backend={}：{}", backend.as_str(), reason);
    let mut error = AppError::new(ErrorCode::CaptureFailed, Some(detail))
        .with_message(
            "屏幕捕获失败：画面可能受保护（DRM/受保护视频）或处于全屏独占，WGC 也无法获取；请退出独占应用或换用其他截图方式后重试",
        )
        // 与契约 §8.2 SA-1001 的 fallback 一致，显式写出便于单测断言与前端兜底。
        .with_fallback(["wgc", "retry"]);
    let context = match (failure, stats) {
        (CaptureFailure::BlackFrame, Some(stats)) => serde_json::json!({
            "capture.backend": backend.as_str(),
            "capture.blackFrame": true,
            "capture.blackRatio": stats.black_ratio(),
            "capture.meanLuma": stats.mean_luma,
        }),
        _ => serde_json::json!({
            "capture.backend": backend.as_str(),
            "capture.blackFrame": false,
        }),
    };
    error = error.with_context(context);
    error
}

/// 骨架捕获器：不调用 Win32，用虚拟桌面物理尺寸合成帧。
pub struct SkeletonCapturer {
    app: tauri::AppHandle,
}

impl SkeletonCapturer {
    /// 创建骨架捕获器。
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl ScreenCapturer for SkeletonCapturer {
    async fn capture_virtual_desktop(&self) -> AppResult<FrozenFrame> {
        let desktop = virtual_desktop_geometry(&self.app);
        let width = desktop.rect.width.max(1.0).round() as u32;
        let height = desktop.rect.height.max(1.0).round() as u32;
        if width == 0 || height == 0 {
            return Err(AppError::new(
                ErrorCode::CaptureFailed,
                Some("无法获取显示器尺寸".to_string()),
            ));
        }
        Ok(FrozenFrame {
            width,
            height,
            data_url: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造 BGRA 缓冲：每个像素给相同灰度值（B=G=R=v，BT.601 加权恰好等于 v）。
    fn bgra_uniform(pixels: usize, luma: u8) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(pixels * 4);
        for _ in 0..pixels {
            buffer.extend_from_slice(&[luma, luma, luma, 255]);
        }
        buffer
    }

    fn black_stats(pixels: usize) -> FrameLumaStats {
        FrameLumaStats {
            pixels,
            black_pixels: pixels,
            mean_luma: 0.0,
            variance: 0.0,
        }
    }

    fn content_stats() -> FrameLumaStats {
        // 30% 纯白 + 70% 中灰：均值≈107、方差大，肯定不是黑屏。
        let mut buffer = bgra_uniform(70_000, 40);
        buffer.extend_from_slice(&bgra_uniform(30_000, 255));
        luma_stats_from_bgra(&buffer)
    }

    #[test]
    fn luma_stats_follow_bt601_weights() {
        let white = luma_stats_from_bgra(&bgra_uniform(4, 255));
        assert_eq!(white.pixels, 4);
        assert_eq!(white.black_pixels, 0);
        assert!((white.mean_luma - 255.0).abs() < 1e-6);
        assert!(white.variance.abs() < 1e-6);

        // BGRA：B=0, G=0, R=255 → 0.299*255 = 76.245
        let red = luma_stats_from_bgra(&[0, 0, 255, 255]);
        assert!((red.mean_luma - 76.245).abs() < 1e-3);

        // 不足 4 字节的尾巴被忽略
        let malformed = luma_stats_from_bgra(&[0, 0, 0, 255, 42]);
        assert_eq!(malformed.pixels, 1);
    }

    #[test]
    fn normal_frame_is_not_black_and_not_failed() {
        let stats = content_stats();
        assert!(!is_black_frame(&stats));
        assert!(!is_capture_failed(Some(&stats)));
    }

    #[test]
    fn pure_black_frame_is_detected() {
        let black = black_stats(1920 * 1080);
        assert!(is_black_frame(&black));
        assert!(is_capture_failed(Some(&black)));
        assert_eq!(classify_failure(Some(&black)), CaptureFailure::BlackFrame);
    }

    #[test]
    fn near_black_gradient_boundary() {
        // 全部亮度 0..=15（<16）→ 判黑（均值 7.5、标准差 ~4.6 < 6）
        let mut data = Vec::new();
        for value in 0..=15u8 {
            for _ in 0..100 {
                data.extend_from_slice(&[value, value, value, 255]);
            }
        }
        let stats = luma_stats_from_bgra(&data);
        assert!((stats.black_ratio() - 1.0).abs() < 1e-9);
        assert!(is_black_frame(&stats), "全 <16 亮度应判黑: {stats:?}");

        // 混入 16/17 → 黑占比 16/18 ≈ 0.889 < 0.99 → 不判黑（边界外）
        let mut data = Vec::new();
        for value in 0..=17u8 {
            for _ in 0..100 {
                data.extend_from_slice(&[value, value, value, 255]);
            }
        }
        let stats = luma_stats_from_bgra(&data);
        assert!(stats.black_ratio() < BLACK_RATIO_MIN);
        assert!(!is_black_frame(&stats), "黑占比不足不应判黑: {stats:?}");
    }

    #[test]
    fn dark_ui_with_small_bright_text_is_not_black() {
        // 99.5% 纯黑 + 0.5% 白字：黑占比 ≥0.99、均值 <16，但方差极大 → 不能判黑，
        // 否则深色主题的截图会被误当成捕获失败。
        let mut data = bgra_uniform(19_900, 0);
        data.extend_from_slice(&bgra_uniform(100, 255));
        let stats = luma_stats_from_bgra(&data);
        assert!(stats.black_ratio() >= BLACK_RATIO_MIN);
        assert!(stats.mean_luma < BLACK_LUMA_MAX);
        assert!(stats.std_dev() > BLACK_STD_DEV_MAX);
        assert!(
            !is_black_frame(&stats),
            "深色 UI + 少量亮文字不应判黑: {stats:?}"
        );
    }

    #[test]
    fn empty_frame_is_capture_failure() {
        let empty = luma_stats_from_bgra(&[]);
        assert!(empty.is_empty());
        assert!(!is_black_frame(&empty), "空帧不是黑帧");
        assert!(is_capture_failed(None), "None（WGC 未取到帧）按失败处理");
        assert!(is_capture_failed(Some(&empty)));
        assert_eq!(classify_failure(None), CaptureFailure::EmptyFrame);
        assert_eq!(classify_failure(Some(&empty)), CaptureFailure::EmptyFrame);
    }

    #[test]
    fn capture_failed_error_maps_to_sa1001_with_wgc_fallback() {
        let error = capture_failed_error(CaptureBackend::Wgc, Some(&black_stats(1920 * 1080)));
        assert_eq!(error.code, ErrorCode::CaptureFailed);
        assert_eq!(error.code, "SA-1001");
        assert!(error.retryable, "SA-1001 必须 retryable");
        assert_eq!(error.fallback, vec!["wgc".to_string(), "retry".to_string()]);
        assert!(
            error.message.contains("受保护"),
            "message 应可操作: {}",
            error.message
        );
        assert!(
            error.message.contains("全屏独占"),
            "message 应可操作: {}",
            error.message
        );
        let detail = error.detail.as_deref().unwrap_or_default();
        assert!(
            detail.contains("backend=wgc"),
            "detail 应记录后端: {detail}"
        );
        assert!(detail.contains("黑屏"), "detail 应说明失败类型: {detail}");
        let context = error.context.expect("应有 capture.backend 上下文");
        assert_eq!(context["capture.backend"], "wgc");
        assert_eq!(context["capture.blackFrame"], true);
        assert_eq!(context["capture.blackRatio"], 1.0);
    }

    #[test]
    fn capture_failed_error_handles_empty_frame_from_bitblt() {
        let error = capture_failed_error(CaptureBackend::BitBlt, None);
        assert_eq!(error.code, "SA-1001");
        assert!(error.retryable);
        assert_eq!(error.fallback, vec!["wgc".to_string(), "retry".to_string()]);
        let detail = error.detail.as_deref().unwrap_or_default();
        assert!(detail.contains("backend=bitblt"), "detail: {detail}");
        assert!(detail.contains("未能取到画面"), "detail: {detail}");
        let context = error.context.expect("应有 capture.backend 上下文");
        assert_eq!(context["capture.backend"], "bitblt");
        assert_eq!(context["capture.blackFrame"], false);
    }

    #[test]
    fn thresholds_match_spike_tool() {
        // 与 scripts/spikes/s09/Program.cs 的 BlackLumaMax/BlackRatioMin/BlackStdDevMax 一致
        assert!((BLACK_LUMA_MAX - 16.0).abs() < 1e-9);
        assert!((BLACK_RATIO_MIN - 0.99).abs() < 1e-9);
        assert!((BLACK_STD_DEV_MAX - 6.0).abs() < 1e-9);
    }

    #[test]
    fn backend_names_match_contract_fallback_tokens() {
        assert_eq!(CaptureBackend::BitBlt.as_str(), "bitblt");
        assert_eq!(CaptureBackend::Wgc.as_str(), "wgc");
        assert_eq!(
            ErrorCode::fallback(ErrorCode::CaptureFailed),
            &["wgc", "retry"]
        );
    }

    #[test]
    fn frame_to_meta_keeps_null_data_url() {
        let frame = FrozenFrame {
            width: 1920,
            height: 1080,
            data_url: None,
        };
        let meta = frame.to_meta();
        assert_eq!(meta.width, 1920);
        assert_eq!(meta.height, 1080);
        assert!(meta.data_url.is_none());
    }
}
