//! 捕获编排（M04）：调用 [`ScreenCapturer`]，返回冻结帧；会话状态更新由命令层负责。

use std::sync::Arc;

use crate::contract::errors::AppResult;
use crate::contract::types::VirtualDesktop;
use crate::platform::capture::{virtual_desktop_geometry, FrozenFrame, ScreenCapturer};

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
