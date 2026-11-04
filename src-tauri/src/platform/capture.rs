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

/// 主显示器几何信息（骨架模式与 overlay 初始尺寸共用）。
///
/// TODO(real): 多显示器虚拟桌面矩形（`EnumDisplayMonitors` 外接矩形）。
pub fn primary_monitor_geometry(app: &tauri::AppHandle) -> VirtualDesktop {
    match app.primary_monitor() {
        Ok(Some(monitor)) => {
            let size = monitor.size();
            let position = monitor.position();
            coords::virtual_desktop(
                position.x as f64,
                position.y as f64,
                size.width as f64,
                size.height as f64,
                monitor.scale_factor(),
            )
        }
        _ => VirtualDesktop {
            rect: RectPhys {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            dpr: 1.0,
        },
    }
}

/// 骨架捕获器：不调用 Win32，用主显示器物理尺寸合成帧。
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
        let desktop = primary_monitor_geometry(&self.app);
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
