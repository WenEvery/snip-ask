//! 系统命令（契约 §6 第 23–26 条）。

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::errors::AppResult;
use crate::contract::events::ConfigChangedPayload;
use crate::contract::types::{HotkeyStatus, Stats};
use crate::events::emit_config_changed;

/// 命令：探测快捷键（格式 / 规范化 / 配置内冲突 / OS 级占用 `system`）。
///
/// OS 级探测复用 `tauri-plugin-global-shortcut` 的 `register` / `unregister`
/// （插件在 OS 注册失败时返回 `Err`），临时注册成功后立即注销，避免探测本身长期占用热键。
#[tauri::command]
pub async fn hotkey_probe(
    app: AppHandle,
    state: State<'_, AppState>,
    spec: String,
) -> AppResult<HotkeyStatus> {
    let config = state.snapshot_config();
    Ok(crate::app::hotkeys::probe_os(&app, &config, &spec))
}

/// 命令：设置开机自启（同时写入 `app.autostart` 并广播 `config:changed`）。
#[tauri::command]
pub async fn set_autostart(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<()> {
    let skeleton_mode = state.is_skeleton();
    // 先改系统自启（失败返回 SA-5004，不修改配置）。
    crate::platform::autostart::set_enabled(&app, enabled, skeleton_mode)?;

    let old = state.snapshot_config();
    let mut candidate = old.clone();
    let (view, changed) = state
        .config_center
        .patch(
            &mut candidate,
            serde_json::json!({ "app": { "autostart": enabled } }),
            state.credentials.as_ref(),
        )
        .await?;
    *state.config.write() = candidate;
    emit_config_changed(
        &app,
        &ConfigChangedPayload {
            config: view,
            changed_keys: changed,
        },
    );
    Ok(())
}

/// 命令：打开外部链接（仅 https 或 http://127.0.0.1；骨架模式只记日志）。
#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> AppResult<()> {
    crate::services::search_gateway::open_external(Some(&app), &url).await
}

/// 命令：运行诊断信息。
#[tauri::command]
pub async fn app_stats(state: State<'_, AppState>) -> AppResult<Stats> {
    Ok(crate::services::diagnostics::collect(
        state.active_session_id(),
        state.active_task_count() as u32,
        state.engine_status(),
        state.models.installed_count(),
        state.is_skeleton(),
        state.uptime_sec(),
    ))
}

/// 命令：退出应用（保存配置后退出）。
#[tauri::command]
pub async fn app_quit(app: AppHandle) -> AppResult<()> {
    if let Err(err) = crate::app::lifecycle::shutdown(&app) {
        tracing::warn!(code = %err.code, "退出前清理失败");
    }
    app.exit(0);
    Ok(())
}
