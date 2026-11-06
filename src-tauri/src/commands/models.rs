//! 本地模型命令（契约 §6 第 18–21 条）。

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::errors::AppResult;
use crate::contract::types::{DownloadId, ModelInfo};
use crate::events::emit_engine_status;

/// 命令：列出模型（清单 + 安装 / 下载状态）。
#[tauri::command]
pub async fn models_list(state: State<'_, AppState>) -> AppResult<Vec<ModelInfo>> {
    state.models.refresh_installed();
    Ok(state.models.list())
}

/// 命令：开始下载（事件推送进度）。
#[tauri::command]
pub async fn models_download(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<DownloadId> {
    state.models.start_download(&app, &id)
}

/// 命令：取消下载。
#[tauri::command]
pub async fn models_cancel(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let cancelled = state.models.cancel(&app, &id);
    tracing::debug!(model = %id, cancelled = cancelled, "取消模型下载");
    Ok(())
}

/// 命令：删除已安装模型（可能影响本地引擎状态）。
#[tauri::command]
pub async fn models_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    state.models.remove(&id)?;
    emit_engine_status(&app, state.engine_status());
    Ok(())
}
