//! 首启引导命令（契约 §6 第 14–16 条 / §9）。

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::config::validate;
use crate::contract::errors::AppResult;
use crate::contract::events::ConfigChangedPayload;
use crate::contract::types::OnboardingState;
use crate::domain::onboarding::{finish, merge_into_config};
use crate::events::emit_config_changed;
use crate::services::onboarding_service::now_iso;

use super::config::apply_config_side_effects;

/// 命令：读取引导状态。
#[tauri::command]
pub async fn onboarding_state(state: State<'_, AppState>) -> AppResult<OnboardingState> {
    Ok(state.onboarding.lock().clone())
}

/// 命令：保存某一步草稿（即时持久化）。
#[tauri::command]
pub async fn onboarding_update(
    state: State<'_, AppState>,
    step: i32,
    data: serde_json::Value,
) -> AppResult<OnboardingState> {
    let service = state.onboarding_service();
    let mut guard = state.onboarding.lock();
    service.update(&mut guard, step, data)?;
    Ok(guard.clone())
}

/// 命令：完成 / 跳过引导。
///
/// - `skip = true` → `skipped = true, completed = false`（受限模式，不合并草稿）；
/// - `skip = false` → `completed = true`，把草稿中已填项合并进 `Config` 并广播 `config:changed`。
#[tauri::command]
pub async fn onboarding_finish(
    app: AppHandle,
    state: State<'_, AppState>,
    skip: bool,
) -> AppResult<()> {
    let service = state.onboarding_service();
    let snapshot = {
        let mut guard = state.onboarding.lock();
        finish(&mut guard, skip, &now_iso());
        service.save_atomic(&guard)?;
        guard.clone()
    };

    if !skip {
        let old = state.snapshot_config();
        let mut candidate = old.clone();
        if merge_into_config(&snapshot, &mut candidate) {
            validate(&candidate)?;
            state.config_center.save_atomic(&candidate)?;
            let changed = crate::services::config_center::classify_change(&old, &candidate);
            *state.config.write() = candidate;
            apply_config_side_effects(&app, &state, &changed);
            let view = state.config_view().await;
            emit_config_changed(
                &app,
                &ConfigChangedPayload {
                    config: view,
                    changed_keys: changed,
                },
            );
        }
    }
    Ok(())
}
