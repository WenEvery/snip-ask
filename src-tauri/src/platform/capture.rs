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
