//! 选区命令（契约 §6 第 3–4 条）。

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::{MenuReadyPayload, OcrStatusPayload, OcrStatusState};
use crate::contract::types::{RectPhys, SelectionResult, SessionId};
use crate::events::{emit_menu_ready, emit_ocr_status};
use crate::services::ocr_service;

/// 命令：提交选区 → 返回识别结果；异步触发 `ocr:status(done)` 与 `menu:ready`。
#[tauri::command]
pub async fn selection_commit(
    app: AppHandle,
    state: State<'_, AppState>,
    session: SessionId,
    rect_phys: RectPhys,
) -> AppResult<SelectionResult> {
    let config = state.snapshot_config();
    let rect = rect_phys.normalize();
    if rect.is_empty() {
        return Err(
            AppError::new(ErrorCode::OcrFailed, Some("选区面积小于 1px²".to_string()))
                .with_message("请先框选内容")
                .with_fallback(vec!["retry"]),
        );
    }

    // 骨架模式：单条覆盖选区的占位框；文字存在判定使用真实纯逻辑。
    let scan = ocr_service::placeholder_selection(&rect);
    let boxes = ocr_service::filter_boxes(&scan.boxes, &rect, config.ocr.text_threshold);
    let has_text = ocr_service::presence(&boxes, &rect, config.ocr.text_threshold);
    let preview = {
        let merged = ocr_service::merge_text(&boxes);
        if merged.is_empty() {
            scan.preview.clone()
        } else {
            merged
        }
    };

    let result = SelectionResult {
        session: session.clone(),
        rect_phys: rect,
        has_text,
        preview,
        boxes,
    };

    {
        let mut store = state.session.lock();
        store.commit_selection(&session, result.clone())?;
    }

    // 异步：先 ocr:status(done)，再 menu:ready（不阻塞命令返回）。
    let items = crate::domain::action::visible_actions(&config, result.has_text);
    let background_app = app.clone();
    let background_result = result.clone();
    let _handle = tauri::async_runtime::spawn(async move {
        emit_ocr_status(
            &background_app,
            &OcrStatusPayload {
                session: background_result.session.clone(),
                state: OcrStatusState::Done,
                boxes: Some(background_result.boxes.clone()),
            },
        );
        emit_menu_ready(
            &background_app,
            &MenuReadyPayload {
                session: background_result.session.clone(),
                items,
                has_text: background_result.has_text,
                preview: background_result.preview.clone(),
            },
        );
    });

    Ok(result)
}

/// 命令：重新框选（复用整屏 OCR 缓存，仅重置选区）。
#[tauri::command]
pub async fn selection_reset(
    app: AppHandle,
    state: State<'_, AppState>,
    session: SessionId,
) -> AppResult<()> {
    {
        let mut store = state.session.lock();
        store.reset_selection(&session)?;
    }
    // 确保 overlay 可见（用户从结果窗 / 菜单返回重选）。
    crate::app::windows::show_overlay(&app)?;
    Ok(())
}
