//! 窗口命令（契约 §6 第 27–28 条，脚手架扩展）。

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::errors::AppResult;
use crate::contract::types::WindowLabel;

/// 命令：显示并聚焦窗口（main / overlay / result）。
#[tauri::command]
pub async fn window_show(
    app: AppHandle,
    _state: State<'_, AppState>,
    window: WindowLabel,
) -> AppResult<()> {
    crate::app::windows::show(&app, window)
}

/// 命令：隐藏窗口。
#[tauri::command]
pub async fn window_hide(
    app: AppHandle,
    _state: State<'_, AppState>,
    window: WindowLabel,
) -> AppResult<()> {
    crate::app::windows::hide(&app, window);
    Ok(())
}
