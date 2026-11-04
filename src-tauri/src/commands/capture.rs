//! 捕获与冻结命令（契约 §6 第 1–2 条）。

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager, State};

use crate::app::state::AppState;
use crate::app::windows;
use crate::contract::errors::AppResult;
use crate::contract::events::{OcrStatusPayload, OcrStatusState, OverlayReadyPayload};
use crate::contract::types::SessionId;
use crate::domain::session::SessionPhase;
use crate::events::{emit_ocr_status, emit_overlay_ready};
use crate::platform::capture::{ScreenCapturer, SkeletonCapturer};
use crate::services::capture_service::{virtual_desktop, CaptureService};
use crate::services::ocr_service::{SKELETON_DONE_DELAY_MS, SKELETON_SCANNING_DELAY_MS};

/// `capture_freeze`：捕获虚拟桌面 → 创建会话 → 显示 overlay → `overlay.ready`。
///
/// 也供托盘 / 热键直接调用（不经过 IPC）。
pub async fn do_capture_freeze(app: &AppHandle) -> AppResult<SessionId> {
    let state = app.state::<AppState>();

    // 1) 新会话（自动作废旧会话；旧任务取消）。
    let start = {
        let mut session = state.session.lock();
        session.begin_capture()
    };
    if let Some(replaced) = &start.replaced_session {
        tracing::info!(replaced = %replaced, "新会话开始，旧会话已作废");
    }
    {
        let mut tasks = state.tasks.lock();
        for task in &start.replaced_tasks {
            tasks.cancel(task);
        }
    }
    let session = start.session;

    // 2) 捕获（骨架模式：主显示器物理尺寸合成帧，dataUrl = null）。
    let capturer: Arc<dyn ScreenCapturer> = Arc::new(SkeletonCapturer::new(app.clone()));
    let service = CaptureService::new(capturer);
    let frame = match service.freeze().await {
        Ok(frame) => frame,
        Err(err) => {
            let _ = state.session.lock().cancel(&session);
            return Err(err);
        }
    };

    // 3) 记录冻结帧 + 进入选区交互。
    let desktop = virtual_desktop(app);
    {
        let mut store = state.session.lock();
        store.attach_frame(&session, frame.to_meta())?;
        store.set_selecting(&session)?;
    }

    // 4) 历史会话（含 OCR 原文时会脱敏；骨架模式无原文）。
    state.history.record_session(
        &session,
        chrono::Utc::now().timestamp_millis(),
        None,
        false,
        None,
        None,
    );

    // 5) 显示 overlay（惰性创建，尺寸为虚拟桌面物理尺寸）。
    if let Err(err) = windows::show_overlay(app) {
        let _ = state.session.lock().cancel(&session);
        return Err(err);
    }

    // 6) overlay.ready
    emit_overlay_ready(
        app,
        &OverlayReadyPayload {
            session: session.clone(),
            virtual_rect: desktop.rect,
            dpr: desktop.dpr,
            frame: frame.to_meta(),
        },
    );

    // 7) 前置 OCR 状态时序：150ms scanning → 400ms done（契约 §12）。
    let background_app = app.clone();
    let background_session = session.clone();
    let _handle = tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SKELETON_SCANNING_DELAY_MS)).await;
        emit_ocr_status(
            &background_app,
            &OcrStatusPayload {
                session: background_session.clone(),
                state: OcrStatusState::Scanning,
                boxes: None,
            },
        );
        tokio::time::sleep(Duration::from_millis(
            SKELETON_DONE_DELAY_MS - SKELETON_SCANNING_DELAY_MS,
        ))
        .await;
        // 仅在仍处于选区阶段时补发 done；用户已提交选区时由 selection_commit 负责。
        let still_selecting = {
            let state = background_app.state::<AppState>();
            let store = state.session.lock();
            store.is_current(&background_session)
                && store.current_phase() == SessionPhase::Selecting
        };
        if still_selecting {
            emit_ocr_status(
                &background_app,
                &OcrStatusPayload {
                    session: background_session,
                    state: OcrStatusState::Done,
                    boxes: None,
                },
            );
        }
    });

    Ok(session)
}

/// 命令：开始框选。
#[tauri::command]
pub async fn capture_freeze(app: AppHandle) -> AppResult<SessionId> {
    do_capture_freeze(&app).await
}

/// 命令：取消框选（任意非 idle 态 → idle）。
#[tauri::command]
pub async fn capture_cancel(
    app: AppHandle,
    state: State<'_, AppState>,
    session: SessionId,
) -> AppResult<()> {
    {
        let mut tasks = state.tasks.lock();
        tasks.cancel_session(&session);
    }
    {
        let mut store = state.session.lock();
        store.cancel(&session)?;
    }
    windows::hide_overlay(&app);
    Ok(())
}
