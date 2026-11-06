//! 翻译试译命令（契约 §6 第 22 条）。
//!
//! v1.0.0：真实配置后端（云端供应商 / Qwen-MT / 本地引擎，按降级链尝试）；
//! `skeletonMode=true` 时保持契约 §12 占位译文。实现见 `services::translator`。

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::errors::AppResult;
use crate::services::translator;

/// 命令：试译。
#[tauri::command]
pub async fn translate_test(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
    target: String,
) -> AppResult<String> {
    let config = state.snapshot_config();
    translator::translate_test(&state, &app, &config, &text, &target).await
}
