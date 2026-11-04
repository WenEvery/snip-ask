//! 更新检查（M18 / 契约 §12：不联网，仅发 `update:status(idle)`）。
//!
//! TODO(real): `tauri-plugin-updater` 签名通道、稳定 / 预览通道、下载与回滚。

use std::time::Duration;

use tauri::AppHandle;

use crate::contract::events::UpdateState;
use crate::events::emit_update_status;

/// 启动时检查一次更新（后台任务，不阻塞启动）。
pub async fn check_once(app: &AppHandle) {
    emit_update_status(app, UpdateState::Checking, None, None);
    // 骨架模式不联网，短暂延迟后回到 idle，保证事件顺序可复现。
    tokio::time::sleep(Duration::from_millis(200)).await;
    // TODO(real): 读取 advanced.updateChannel，请求签名 manifest 并比较版本。
    emit_update_status(app, UpdateState::Idle, None, None);
    tracing::debug!("骨架模式：更新检查已完成（不联网）");
}

/// 当前通道描述（诊断用）。
pub fn channel_label(channel: crate::contract::config::UpdateChannel) -> &'static str {
    match channel {
        crate::contract::config::UpdateChannel::Stable => "stable",
        crate::contract::config::UpdateChannel::Preview => "preview",
    }
}
