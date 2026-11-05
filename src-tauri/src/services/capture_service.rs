//! 捕获编排（M04）：调用 [`ScreenCapturer`]，返回冻结帧；会话状态更新由命令层负责。
//!
//! # S-09 截图失败降级（DRM / 全屏独占）
//!
//! 真实 BitBlt/WGC 调用不进入产品代码（`unsafe_code = "forbid"`），但**降级策略**是纯逻辑，
//! 在这里以 [`resolve_capture`] 落成可单测的决策链：
//!
//! 1. BitBlt 拿到内容 → 直接使用；
//! 2. BitBlt 黑屏 / 空帧 → **自动尝试 WGC**（[`CaptureDecision::FallbackToWgc`]）；
//! 3. WGC 拿到内容 → 使用 WGC 帧；WGC 仍黑屏 / 空帧 / 未取到帧 → [`CaptureDecision::Failed`]，
//!    由 [`capture_failed_error`] 生成 `SA-1001`（retryable，fallback `["wgc", "retry"]`）。
//!
//! 实测依据（`docs/spikes/S-09-截图失败降级与WGC对比.md`，2026-11-05）：
//! - `WDA_MONITOR(0x01)`/DRM 保护面：BitBlt 与 WGC 都是纯黑，WGC **不能绕过**；
//! - `WDA_EXCLUDEFROMCAPTURE(0x11)`：窗口从截屏中消失（不是黑屏），且 WGC 窗口项 3/3 次
//!   5000ms 内取不到帧——所以「未取到帧」必须与「黑帧」一样按失败处理；
//! - 混合 DPI 双屏只有一屏黑时整帧黑占比约 76%，真实实现需在目标区域/显示器切片上判定。

use std::sync::Arc;

use crate::contract::errors::{AppError, AppResult};
use crate::contract::types::VirtualDesktop;
use crate::platform::capture::{
    capture_failed_error, is_black_frame, virtual_desktop_geometry, CaptureBackend, FrameLumaStats,
    FrozenFrame, ScreenCapturer,
};

/// 捕获服务。
pub struct CaptureService {
    capturer: Arc<dyn ScreenCapturer>,
}

impl CaptureService {
    /// 注入捕获器（骨架模式为 `SkeletonCapturer`）。
    pub fn new(capturer: Arc<dyn ScreenCapturer>) -> Self {
        Self { capturer }
    }

    /// 捕获虚拟桌面。
    pub async fn freeze(&self) -> AppResult<FrozenFrame> {
        self.capturer.capture_virtual_desktop().await
    }
}

/// 当前虚拟桌面几何信息（overlay 尺寸、坐标换算基准）。
///
/// S-01：改为**所有显示器物理矩形的最小外接矩形**（含负坐标），不再只取主屏。
pub fn virtual_desktop(app: &tauri::AppHandle) -> VirtualDesktop {
    virtual_desktop_geometry(app)
}

// ───────────────────────── S-09：降级决策（纯 safe，无 Win32） ─────────────────────────

/// 单条捕获路径的结果分类（由捕获层提供的亮度统计推导）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    /// 拿到可用内容。
    Content,
    /// 黑屏（DRM / 受保护内容 / 全屏独占的典型表现）。
    BlackFrame,
    /// 空帧 / 未取到画面（如 WGC 窗口项超时）。
    EmptyFrame,
}

impl CaptureOutcome {
    /// 由亮度统计推导结果；`None`（未取到帧）与空统计都按 [`Self::EmptyFrame`]。
    pub fn from_stats(stats: Option<&FrameLumaStats>) -> Self {
        match stats {
            None => Self::EmptyFrame,
            Some(stats) if stats.is_empty() => Self::EmptyFrame,
            Some(stats) if is_black_frame(stats) => Self::BlackFrame,
            Some(_) => Self::Content,
        }
    }

    /// 是否拿到可用内容。
    pub fn is_content(self) -> bool {
        matches!(self, Self::Content)
    }
}

/// 降级决策结果。
#[derive(Debug)]
pub enum CaptureDecision {
    /// 当前帧可用（BitBlt 或 WGC 之一）。
    UseFrame,
    /// 当前帧不可用：自动尝试 WGC。
    FallbackToWgc,
    /// 两条路径都失败：返回 `SA-1001`（retryable，fallback `["wgc", "retry"]`）。
    Failed(AppError),
}

/// S-09 降级链（纯决策，不调用 Win32；真实捕获层按此执行）。
///
/// 参数：
/// - `bitblt`：BitBlt 主路径的亮度统计；`None` 表示空帧/未取到帧；
/// - `wgc`：WGC 降级尝试结果；`None` 表示**尚未尝试**，`Some(None)` 表示尝试了但
///   **未取到帧**（WGC 窗口项超时），`Some(Some(stats))` 表示取到了帧。
///
/// 决策：
/// 1. BitBlt 有内容 → [`CaptureDecision::UseFrame`]；
/// 2. BitBlt 黑屏/空帧且 WGC 未尝试 → [`CaptureDecision::FallbackToWgc`]；
/// 3. WGC 有内容 → [`CaptureDecision::UseFrame`]；
/// 4. WGC 黑屏/空帧/无帧 → [`CaptureDecision::Failed`]（SA-1001，记录失败后端上下文）。
pub fn resolve_capture(
    bitblt: Option<&FrameLumaStats>,
    wgc: Option<Option<&FrameLumaStats>>,
) -> CaptureDecision {
    match CaptureOutcome::from_stats(bitblt) {
        CaptureOutcome::Content => CaptureDecision::UseFrame,
        CaptureOutcome::BlackFrame | CaptureOutcome::EmptyFrame => match wgc {
            None => CaptureDecision::FallbackToWgc,
            Some(Some(stats)) if CaptureOutcome::from_stats(Some(stats)).is_content() => {
                CaptureDecision::UseFrame
            }
            Some(Some(stats)) => {
                CaptureDecision::Failed(capture_failed_error(CaptureBackend::Wgc, Some(stats)))
            }
            Some(None) => CaptureDecision::Failed(capture_failed_error(CaptureBackend::Wgc, None)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::errors::ErrorCode;

    /// 全黑帧统计（1920×1080，luma 0）。
    fn black_stats() -> FrameLumaStats {
        FrameLumaStats {
            pixels: 1920 * 1080,
            black_pixels: 1920 * 1080,
            mean_luma: 0.0,
            variance: 0.0,
        }
    }

    /// 正常内容帧统计（均值/方差都明显非黑）。
    fn content_stats() -> FrameLumaStats {
        FrameLumaStats {
            pixels: 1920 * 1080,
            black_pixels: 30_000,
            mean_luma: 118.0,
            variance: 6_400.0,
        }
    }

    fn empty_stats() -> FrameLumaStats {
        FrameLumaStats {
            pixels: 0,
            black_pixels: 0,
            mean_luma: 0.0,
            variance: 0.0,
        }
    }

    #[test]
    fn capture_outcome_classifies_stats() {
        assert_eq!(CaptureOutcome::from_stats(None), CaptureOutcome::EmptyFrame);
        assert_eq!(
            CaptureOutcome::from_stats(Some(&empty_stats())),
            CaptureOutcome::EmptyFrame
        );
        assert_eq!(
            CaptureOutcome::from_stats(Some(&black_stats())),
            CaptureOutcome::BlackFrame
        );
        assert_eq!(
            CaptureOutcome::from_stats(Some(&content_stats())),
            CaptureOutcome::Content
        );
    }

    #[test]
    fn resolve_capture_uses_bitblt_content_without_wgc() {
        let decision = resolve_capture(Some(&content_stats()), None);
        assert!(matches!(decision, CaptureDecision::UseFrame));
    }

    #[test]
    fn resolve_capture_falls_back_to_wgc_when_bitblt_black() {
        // BitBlt 黑屏且 WGC 尚未尝试 → 自动尝试 WGC
        let decision = resolve_capture(Some(&black_stats()), None);
        assert!(matches!(decision, CaptureDecision::FallbackToWgc));
        // 空帧同样触发 WGC 降级
        let decision = resolve_capture(None, None);
        assert!(matches!(decision, CaptureDecision::FallbackToWgc));
        let decision = resolve_capture(Some(&empty_stats()), None);
        assert!(matches!(decision, CaptureDecision::FallbackToWgc));
    }

    #[test]
    fn resolve_capture_recovers_with_wgc_content() {
        // S-09：BitBlt 黑屏后 WGC 拿到内容（部分遮挡/深色 UI 场景）→ 使用 WGC 帧
        let decision = resolve_capture(Some(&black_stats()), Some(Some(&content_stats())));
        assert!(matches!(decision, CaptureDecision::UseFrame));
        let decision = resolve_capture(None, Some(Some(&content_stats())));
        assert!(matches!(decision, CaptureDecision::UseFrame));
    }

    #[test]
    fn resolve_capture_fails_with_sa1001_when_wgc_still_black() {
        let decision = resolve_capture(Some(&black_stats()), Some(Some(&black_stats())));
        match decision {
            CaptureDecision::Failed(error) => {
                assert_eq!(error.code, ErrorCode::CaptureFailed);
                assert_eq!(error.code, "SA-1001");
                assert!(error.retryable);
                assert_eq!(error.fallback, vec!["wgc".to_string(), "retry".to_string()]);
                assert!(error.message.contains("受保护"));
                let detail = error.detail.as_deref().unwrap_or_default();
                assert!(detail.contains("backend=wgc"), "detail: {detail}");
                let context = error.context.expect("应有 capture.backend 上下文");
                assert_eq!(context["capture.backend"], "wgc");
                assert_eq!(context["capture.blackFrame"], true);
            }
            other => panic!("应返回 Failed，实际 {other:?}"),
        }
    }

    #[test]
    fn resolve_capture_fails_with_sa1001_when_wgc_returns_no_frame() {
        // S-09 实测：WDA_EXCLUDEFROMCAPTURE 窗口的 WGC 窗口项 3/3 次取不到帧
        let decision = resolve_capture(Some(&black_stats()), Some(None));
        match decision {
            CaptureDecision::Failed(error) => {
                assert_eq!(error.code, "SA-1001");
                assert!(error.retryable);
                assert_eq!(error.fallback, vec!["wgc".to_string(), "retry".to_string()]);
                let detail = error.detail.as_deref().unwrap_or_default();
                assert!(detail.contains("未能取到画面"), "detail: {detail}");
                let context = error.context.expect("应有 capture.backend 上下文");
                assert_eq!(context["capture.backend"], "wgc");
                assert_eq!(context["capture.blackFrame"], false);
            }
            other => panic!("应返回 Failed，实际 {other:?}"),
        }
    }
}
