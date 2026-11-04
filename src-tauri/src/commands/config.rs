//! 配置命令（契约 §6 第 9–13 条 / §4.4）。

use std::path::Path;

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::config::ConfigView;
use crate::contract::errors::AppResult;
use crate::contract::events::ConfigChangedPayload;
use crate::events::emit_config_changed;

/// 命令：读取当前配置视图（providers 含 hasKey / keyMask）。
#[tauri::command]
pub async fn config_get(state: State<'_, AppState>) -> AppResult<ConfigView> {
    Ok(state.config_view().await)
}

/// 命令：深合并 patch → 校验 → 原子持久化 → 广播 `config.changed`。
#[tauri::command]
pub async fn config_patch(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: serde_json::Value,
) -> AppResult<ConfigView> {
    let old = state.snapshot_config();
    let mut candidate = old.clone();
    let (view, changed) = state
        .config_center
        .patch(&mut candidate, patch, state.credentials.as_ref())
        .await?;
    *state.config.write() = candidate;
    apply_config_side_effects(&app, &state, &changed);
    emit_config_changed(
        &app,
        &ConfigChangedPayload {
            config: view.clone(),
            changed_keys: changed,
        },
    );
    Ok(view)
}

/// 命令：从文件导入配置（迁移 + 校验 + 整份替换，保留本机密钥）。
#[tauri::command]
pub async fn config_import(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<ConfigView> {
    let old = state.snapshot_config();
    let mut candidate = old.clone();
    let (view, changed) = state
        .config_center
        .import(&mut candidate, Path::new(&path), state.credentials.as_ref())
        .await?;
    *state.config.write() = candidate;
    apply_config_side_effects(&app, &state, &changed);
    emit_config_changed(
        &app,
        &ConfigChangedPayload {
            config: view.clone(),
            changed_keys: changed,
        },
    );
    Ok(view)
}

/// 命令：导出配置（默认不含密钥；`includeSecrets = true` 返回 SA-5002）。
#[tauri::command]
pub fn config_export(
    state: State<'_, AppState>,
    path: Option<String>,
    include_secrets: Option<bool>,
) -> AppResult<String> {
    let config = state.snapshot_config();
    let target = path.as_deref().map(Path::new);
    let exported = state
        .config_center
        .export(&config, target, include_secrets.unwrap_or(false))?;
    Ok(exported.to_string_lossy().to_string())
}

/// 命令：恢复默认（`all` 或分组名）。
#[tauri::command]
pub async fn config_reset(
    app: AppHandle,
    state: State<'_, AppState>,
    scope: String,
) -> AppResult<ConfigView> {
    let old = state.snapshot_config();
    let mut candidate = old.clone();
    let (view, changed) = state
        .config_center
        .reset(&mut candidate, &scope, state.credentials.as_ref())
        .await?;
    *state.config.write() = candidate;
    apply_config_side_effects(&app, &state, &changed);
    emit_config_changed(
        &app,
        &ConfigChangedPayload {
            config: view.clone(),
            changed_keys: changed,
        },
    );
    Ok(view)
}

/// 配置热生效副作用（契约 §4.4）：
/// - `hotkeys.*` / `app.autostart` → 重新注册热键；
/// - `history.enabled` → 历史仓储开关；
/// - `local.*` / `translate.backend` → 重启本地引擎（TODO(real)）；
/// - `advanced.proxy` → 需重启应用（仅标注）。
pub(crate) fn apply_config_side_effects(
    app: &AppHandle,
    state: &AppState,
    changed: &[String],
) {
    if changed.iter().any(|key| key.starts_with("hotkeys.")) {
        if let Err(err) = crate::app::hotkeys::refresh(app) {
            tracing::warn!(code = %err.code, "热键重新注册失败");
        }
    }
    if changed.iter().any(|key| key == "history.enabled") {
        let config = state.snapshot_config();
        state.history.set_enabled(config.history.enabled);
        tracing::info!(enabled = config.history.enabled, "历史记录开关已更新");
    }
    if changed
        .iter()
        .any(|key| key.starts_with("local.") || key == "translate.backend")
    {
        tracing::info!("本地引擎配置变化：TODO(real) 重启推理进程");
    }
    if changed.iter().any(|key| key == "advanced.proxy") {
        tracing::warn!("代理配置变更需重启应用后生效");
    }
}
