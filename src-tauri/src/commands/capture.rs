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
use crate::services::capture_service::CaptureService;
use crate::services::ocr_service::{SKELETON_DONE_DELAY_MS, SKELETON_SCANNING_DELAY_MS};

/// `capture_freeze`：捕获虚拟桌面 → 创建会话 → 显示 overlay → `overlay:ready`。
///
/// 幂等（契约 §6）：若当前会话已处于冻结/选区阶段，直接复用并无副作用地补发
/// `overlay:ready`（供 overlay 前端页面加载后主动拉取，解决冷启动事件竞态）。
///
/// 也供托盘 / 热键直接调用（不经过 IPC）。
pub async fn do_capture_freeze(app: &AppHandle) -> AppResult<SessionId> {
    let state = app.state::<AppState>();
    tracing::debug!(target: "snip_ask_lib::trace", "capture_freeze 入口，当前阶段={:?}", state.session.lock().current_phase());

    // 0) 幂等路径：已有冻结/选区中的会话 → 复用并补发快照，不重建窗口与 OCR。
    {
        let snapshot = {
            let store = state.session.lock();
            match store.current() {
                Some(current)
                    if matches!(
                        current.phase,
                        SessionPhase::Freezing | SessionPhase::Selecting
                    ) =>
                {
                    let session = current.id.clone();
                    let phase = current.phase;
                    let frame = current.frame.clone();
                    Some((session, phase, frame))
                }
                _ => None,
            }
        };
        if let Some((session, phase, frame)) = snapshot {
            tracing::debug!(target: "snip_ask_lib::trace", "capture_freeze 幂等复用 session={session} phase={phase:?} 有帧={}", frame.is_some());
            windows::show_overlay(app)?;
            // S-01：show 后等窗口 DPI 稳定并复核几何，再按 overlay 实际 dpr 下发快照。
            windows::settle_overlay_geometry(app).await;
            if phase == SessionPhase::Selecting {
                if let Some(frame) = frame {
                    let desktop = windows::overlay_virtual_desktop(app);
                    emit_overlay_ready(
                        app,
                        &OverlayReadyPayload {
                            session: session.clone(),
                            virtual_rect: desktop.rect,
                            dpr: desktop.dpr,
                            frame,
                        },
                    );
                    tracing::debug!(target: "snip_ask_lib::trace", "capture_freeze 幂等补发 overlay:ready session={session}");
                }
            }
            return Ok(session);
        }
    }

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

    // 3) 记录冻结帧 + 进入选区交互（帧尺寸 = 虚拟桌面外接矩形物理像素，S-01）。
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

    // 5) 显示 overlay（惰性创建，client 区覆盖整个虚拟桌面物理矩形）。
    if let Err(err) = windows::show_overlay(app) {
        let _ = state.session.lock().cancel(&session);
        return Err(err);
    }
    // 5.5) S-01：等 Windows 异步 DPI 消息处理完并复核几何；dpr 取 overlay 窗口实际缩放比。
    windows::settle_overlay_geometry(app).await;
    let desktop = windows::overlay_virtual_desktop(app);

    // 6) overlay:ready
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
