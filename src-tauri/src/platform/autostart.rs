//! 开机自启（M02）。
//!
//! 安装版走 `tauri-plugin-autostart`；骨架模式只修改内存状态（不写注册表）。
//!
//! 已真实编译验证（Tauri 2.12 / Rust 1.99）: 校验 `tauri_plugin_autostart::ManagerExt::autolaunch()` 的方法名与
//! `AutoLaunchManager::{enable, disable, is_enabled}` 签名（插件版本见 Cargo.toml `"2"`）。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::AppHandle;

use crate::contract::errors::{AppError, AppResult, ErrorCode};

/// 骨架模式下的内存自启状态（进程退出即丢，符合契约 §12）。
static SKELETON_AUTOSTART: AtomicBool = AtomicBool::new(false);

/// 骨架模式内存状态（便于测试与诊断）。
pub fn skeleton_state() -> bool {
    SKELETON_AUTOSTART.load(Ordering::SeqCst)
}

/// 开启 / 关闭开机自启。
pub fn set_enabled(app: &AppHandle, enabled: bool, skeleton_mode: bool) -> AppResult<()> {
    if skeleton_mode {
        SKELETON_AUTOSTART.store(enabled, Ordering::SeqCst);
        tracing::info!(enabled, "骨架模式：仅更新内存中的开机自启状态");
        return Ok(());
    }
    apply_autostart(app, enabled)
}

/// 查询开机自启状态。
pub fn is_enabled(app: &AppHandle, skeleton_mode: bool) -> AppResult<bool> {
    if skeleton_mode {
        return Ok(skeleton_state());
    }
    query_autostart(app)
}

#[cfg(windows)]
fn apply_autostart(app: &AppHandle, enabled: bool) -> AppResult<()> {
    use tauri_plugin_autostart::ManagerExt;

    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|err| {
        AppError::new(ErrorCode::AutostartFailed, Some(err.to_string()))
            .with_fallback(vec!["open_settings"])
    })
}

#[cfg(not(windows))]
fn apply_autostart(_app: &AppHandle, _enabled: bool) -> AppResult<()> {
    Err(AppError::new(
        ErrorCode::AutostartFailed,
        Some("开机自启当前仅支持 Windows".to_string()),
    ))
}

#[cfg(windows)]
fn query_autostart(app: &AppHandle) -> AppResult<bool> {
    use tauri_plugin_autostart::ManagerExt;

    app.autolaunch()
        .is_enabled()
        .map_err(|err| AppError::new(ErrorCode::AutostartFailed, Some(err.to_string())))
}

#[cfg(not(windows))]
fn query_autostart(_app: &AppHandle) -> AppResult<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_state_is_memory_only() {
        // 测试进程内共享状态，仅验证读写路径（不触碰系统注册表）。
        SKELETON_AUTOSTART.store(true, Ordering::SeqCst);
        assert!(skeleton_state());
        SKELETON_AUTOSTART.store(false, Ordering::SeqCst);
        assert!(!skeleton_state());
    }
}
