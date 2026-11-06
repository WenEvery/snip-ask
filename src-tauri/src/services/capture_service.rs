//! 捕获编排（M04）：调用 [`ScreenCapturer`]，校验真实帧并返回冻结帧；会话状态更新由命令层负责。
//!
//! # S-09 截图失败降级（DRM / 全屏独占）
//!
//! 真实 BitBlt 调用集中在 `platform::capture::win32_impl`，本模块只做**纯逻辑编排**：
//!
//! 1. [`CaptureService::freeze`] 调用捕获器拿帧（真实模式 = Win32 BitBlt 虚拟桌面）；
//! 2. [`validate_captured_frame`] 复用 S-09 黑屏/空帧判定（[`is_capture_failed`]）：
//!    整帧黑/空 → `SA-1001`（retryable，fallback `["wgc", "retry"]`）；骨架帧（无 BGRA）跳过；
//! 3. 真实模式逐显示器切片做**诊断**（S-09：混合 DPI 双屏只有一屏黑时整帧黑占比仅 ~76%，
//!    整帧阈值会漏检；目标区域判定见 [`validate_region`]，由 `selection_commit` 对选区调用）；
//! 4. 历史决策链 [`resolve_capture`] 保留（BitBlt 黑/空 → WGC → 仍失败 → `SA-1001`），
//!    与真实 BitBlt 实现解耦，供未来接入 WGC 降级后端时直接复用；WGC 暂非主路径（spec §7）。
//!
//! 实测依据（`docs/spikes/S-09-截图失败降级与WGC对比.md`，2026-11-05）：
//! - `WDA_MONITOR(0x01)`/DRM 保护面：BitBlt 与 WGC 都是纯黑，WGC **不能绕过**；
//! - `WDA_EXCLUDEFROMCAPTURE(0x11)`：窗口从截屏中消失（不是黑屏），且 WGC 窗口项 3/3 次
//!   5000ms 内取不到帧——所以「未取到帧」必须与「黑帧」一样按失败处理；
//! - 混合 DPI 双屏只有一屏黑时整帧黑占比约 76%，真实实现需在目标区域/显示器切片上判定。

use std::sync::Arc;
use std::time::Instant;

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{RectPhys, VirtualDesktop};
use crate::platform::capture::{
    bitblt_capture_failed_error, capture_failed_error, is_black_frame, is_capture_failed,
    region_capture_failed_error, system_monitors, system_virtual_desktop_rect,
    virtual_desktop_geometry, CaptureBackend, FrameLumaStats, FrozenFrame, ScreenCapturer,
};

/// 捕获服务。
pub struct CaptureService {
    capturer: Arc<dyn ScreenCapturer>,
}

impl CaptureService {
    /// 注入捕获器（真实模式为 `Win32Capturer`，骨架模式为 `SkeletonCapturer`）。
    pub fn new(capturer: Arc<dyn ScreenCapturer>) -> Self {
        Self { capturer }
    }

    /// 捕获虚拟桌面并对真实帧做 S-09 校验（黑屏/空帧 → `SA-1001`）。
    pub async fn freeze(&self) -> AppResult<FrozenFrame> {
        let started = Instant::now();
        let frame = self.capturer.capture_virtual_desktop().await?;
        validate_captured_frame(&frame)?;
        if frame.bgra.is_some() {
            log_monitor_diagnostics(&frame);
            tracing::debug!(
                target: "snip_ask_lib::trace",
                width = frame.width,
                height = frame.height,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "真实冻结帧捕获/校验完成"
            );
        }
        Ok(frame)
    }
}

/// S-09 整帧校验：真实帧黑屏/空帧 → `SA-1001`；骨架帧（无 BGRA）直接通过。
///
/// 判定口径与 spike 完全一致（[`is_capture_failed`]：空帧、或黑像素占比 ≥99% 且
/// 均值 <16 且标准差 <6）。真实帧缺少 data URL 视为不完整（overlay 无法渲染）。
pub fn validate_captured_frame(frame: &FrozenFrame) -> AppResult<()> {
    if frame.bgra.is_none() {
        // 骨架模式 / 旧会话 meta 帧：没有像素数据，不做黑屏判定。
        return Ok(());
    }
    if !frame.has_complete_bgra() {
        return Err(bitblt_capture_failed_error(None));
    }
    if frame.data_url.is_none() {
        return Err(AppError::new(
            ErrorCode::CaptureFailed,
            Some(format!(
                "真实冻结帧缺少 data URL（{}x{}）",
                frame.width, frame.height
            )),
        )
        .with_message("屏幕捕获失败：冻结帧缺少显示数据，请重试")
        .with_fallback(["retry"]));
    }
    match frame.luma_stats() {
        Some(stats) if !is_capture_failed(Some(&stats)) => Ok(()),
        Some(stats) => Err(bitblt_capture_failed_error(Some(&stats))),
        None => Err(bitblt_capture_failed_error(None)),
    }
}

/// S-09 目标区域校验（`selection_commit` 用）：选区纯黑/空 → `SA-1001`。
///
/// 混合 DPI 双屏只有一屏被 DRM 覆盖时整帧判定会漏检，因此对**选区切片**单独判定；
/// 区域与帧不相交（理论上不可达）时不拦截，交给上层 OCR 兜底。骨架帧（无 BGRA）跳过。
pub fn validate_region(
    frame: &FrozenFrame,
    frame_origin: &RectPhys,
    region: &RectPhys,
) -> AppResult<()> {
    if frame.bgra.is_none() {
        return Ok(());
    }
    if !frame.has_complete_bgra() {
        return Err(bitblt_capture_failed_error(None));
    }
    match frame.region_luma_stats(frame_origin, region) {
        Some(stats) if is_capture_failed(Some(&stats)) => {
            Err(region_capture_failed_error(region, &stats))
        }
        _ => Ok(()),
    }
}

/// 逐显示器切片诊断：真实双屏下只有部分屏幕是黑屏时，记录哪些切片判黑（不改变主流程）。
fn log_monitor_diagnostics(frame: &FrozenFrame) {
    let Some(origin) = system_virtual_desktop_rect() else {
        return;
    };
    for monitor in system_monitors() {
        let Some(stats) = frame.region_luma_stats(&origin, &monitor.rect) else {
            continue;
        };
        if is_black_frame(&stats) {
            tracing::warn!(
                target: "snip_ask_lib::trace",
                x = monitor.rect.x,
                y = monitor.rect.y,
                width = monitor.rect.width,
                height = monitor.rect.height,
                black_ratio = stats.black_ratio(),
                "显示器切片为黑屏（可能受 DRM 保护/全屏独占；整帧判定可能漏检，选区判定仍生效）"
            );
        }
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

    // ───────────────────────── T2：真实冻结帧校验（S-09 分区域） ─────────────────────────

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RectPhys {
        RectPhys {
            x,
            y,
            width,
            height,
        }
    }

    fn uniform_bgra(pixels: usize, luma: u8) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(pixels * 4);
        for _ in 0..pixels {
            buffer.extend_from_slice(&[luma, luma, luma, 255]);
        }
        buffer
    }

    /// 构造真实帧（带完整 data URL 占位，避免测试里跑 PNG 编码）。
    fn real_frame(width: u32, height: u32, pixels: Vec<u8>) -> FrozenFrame {
        FrozenFrame {
            width,
            height,
            data_url: Some("data:image/png;base64,AAAA".to_string()),
            bgra: Some(std::sync::Arc::new(pixels)),
        }
    }

    #[test]
    fn validate_captured_frame_skips_skeleton_frame() {
        // 骨架帧无 BGRA：不判黑（否则骨架模式在深色桌面会误报 SA-1001）。
        assert!(validate_captured_frame(&FrozenFrame::skeleton(1920, 1080)).is_ok());
    }

    #[test]
    fn validate_captured_frame_accepts_content() {
        let mut pixels = uniform_bgra(100, 30);
        pixels.extend_from_slice(&uniform_bgra(100, 240));
        let frame = real_frame(20, 10, pixels);
        assert!(validate_captured_frame(&frame).is_ok());
    }

    #[test]
    fn validate_captured_frame_rejects_black_frame_as_sa1001() {
        let frame = real_frame(20, 10, uniform_bgra(200, 0));
        let error = validate_captured_frame(&frame).expect_err("纯黑帧必须失败");
        assert_eq!(error.code, "SA-1001");
        assert!(error.retryable);
        assert_eq!(error.fallback, vec!["wgc".to_string(), "retry".to_string()]);
        assert_eq!(error.context.expect("上下文")["capture.backend"], "bitblt");
    }

    #[test]
    fn validate_captured_frame_rejects_truncated_or_missing_data_url() {
        // 截断的 BGRA → 空帧（SA-1001）。
        let truncated = real_frame(20, 10, vec![0u8; 100]);
        assert_eq!(
            validate_captured_frame(&truncated)
                .expect_err("截断帧必须失败")
                .code,
            "SA-1001"
        );
        // 有像素但缺 data URL → overlay 无法显示，按失败处理。
        let mut frame = real_frame(20, 10, uniform_bgra(200, 200));
        frame.data_url = None;
        assert_eq!(
            validate_captured_frame(&frame)
                .expect_err("缺 data URL 必须失败")
                .code,
            "SA-1001"
        );
    }

    #[test]
    fn validate_region_rejects_black_selection_only() {
        // 虚拟桌面 (-1080,-470) 4x1：左半纯黑、右半纯白。
        let width = 4u32;
        let height = 1u32;
        let origin = rect(-1080.0, -470.0, 4.0, 1.0);
        let mut pixels = uniform_bgra(2, 0);
        pixels.extend_from_slice(&uniform_bgra(2, 255));
        let frame = real_frame(width, height, pixels);

        // 选区落在左半（纯黑）→ SA-1001，且提示选区/黑屏。
        let error = validate_region(&frame, &origin, &rect(-1080.0, -470.0, 2.0, 1.0))
            .expect_err("纯黑选区必须失败");
        assert_eq!(error.code, "SA-1001");
        assert_eq!(error.fallback, vec!["retry".to_string()]);
        assert!(error.message.contains("选区"), "{}", error.message);
        let context = error.context.expect("区域上下文");
        assert_eq!(context["capture.blackFrame"], true);

        // 选区落在右半（纯白）→ 通过。
        assert!(validate_region(&frame, &origin, &rect(-1078.0, -470.0, 2.0, 1.0)).is_ok());
        // 骨架帧（无 BGRA）→ 跳过判定。
        assert!(validate_region(
            &FrozenFrame::skeleton(4, 1),
            &origin,
            &rect(0.0, 0.0, 1.0, 1.0)
        )
        .is_ok());
    }

    #[test]
    fn validate_region_mixed_frame_not_flagged_by_whole_frame_threshold() {
        // S-09 漏检场景：整帧 75% 黑（混合双屏），整帧判定不失败，但目标选区（黑屏主屏）必须失败。
        let width = 40u32;
        let height = 10u32;
        let origin = rect(0.0, 0.0, 40.0, 10.0);
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..height {
            // 每行：左侧 30px 纯黑（主屏 DRM 黑面）+ 右侧 10px 白色内容（副屏）。
            pixels.extend_from_slice(&uniform_bgra(30, 0));
            pixels.extend_from_slice(&uniform_bgra(10, 255));
        }
        let frame = real_frame(width, height, pixels);
        assert!(
            validate_captured_frame(&frame).is_ok(),
            "整帧黑占比 75% 不应判黑"
        );
        assert!(
            validate_region(&frame, &origin, &rect(0.0, 0.0, 30.0, 10.0)).is_err(),
            "黑色目标区域必须被切片判定拦截"
        );
    }
}
