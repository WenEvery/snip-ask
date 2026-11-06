//! 历史线程命令（契约 v5 第 41–43 条）。
//!
//! v1.0.0 起连续提问的问答正文持久化到 SQLite `messages` 表（`history_repo`），
//! 主窗口可浏览历史线程；本文件只做参数校验与薄转发。

use tauri::State;

use crate::app::state::AppState;
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{HistorySession, HistoryThread, SessionId};

/// 命令：历史会话列表（默认 50 条、上限 200 条，按开始时间倒序）。
#[tauri::command]
pub async fn history_list(
    state: State<'_, AppState>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> AppResult<Vec<HistorySession>> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let offset = offset.unwrap_or(0);
    Ok(state.history.list_sessions(limit, offset))
}

/// 命令：单个会话的完整问答线程（用于历史回溯展示）。
///
/// 会话不存在（已清理 / 非法 id）→ `SA-1099`（invalid state），前端展示为空态 + 提示。
#[tauri::command]
pub async fn history_thread(
    state: State<'_, AppState>,
    session: SessionId,
) -> AppResult<HistoryThread> {
    state.history.session_thread(&session).ok_or_else(|| {
        AppError::new(
            ErrorCode::InvalidState,
            Some(format!("会话 {session} 不存在或已被清理")),
        )
    })
}

/// 命令：清空历史（传 session 清该会话；不传清全部）。
#[tauri::command]
pub async fn history_clear(
    state: State<'_, AppState>,
    session: Option<SessionId>,
) -> AppResult<()> {
    state.history.clear_history(session.as_ref());
    Ok(())
}
