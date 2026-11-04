//! 翻译试译命令（契约 §6 第 22 条）。

use tauri::State;

use crate::app::state::AppState;
use crate::contract::errors::AppResult;
use crate::services::translator;

/// 命令：试译（骨架模式返回固定占位译文）。
#[tauri::command]
pub async fn translate_test(
    state: State<'_, AppState>,
    text: String,
    target: String,
) -> AppResult<String> {
    let config = state.snapshot_config();
    translator::translate_test(&config, &text, &target).await
}
