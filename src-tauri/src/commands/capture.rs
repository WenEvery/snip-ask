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
use crate::platform::capture::{self, ScreenCapturer, SkeletonCapturer, Win32Capturer};
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
    // 契约 v4：新框选 = 新对话。清理被作废会话的轮次（避免 TurnStore 长会话泄漏），
    // 并保证新会话从空线程开始（追问上下文只覆盖当前会话）。
    {
        let mut turns = state.turns.lock();
        if let Some(replaced) = start.replaced_session.as_ref() {
            turns.drop_session(replaced);
        }
        turns.begin(&session);
    }
    // S-02/S-03：新会话作废旧整屏 det 缓存（缓存键为 Session::ocr_session_id，新会话会换键）。
    state.ocr.clear();
    let ocr_session_id = state
        .session
        .lock()
        .current()
        .and_then(|current| current.ocr_session_id.clone())
        .unwrap_or_else(|| session.as_str().to_string());

    // 2) 捕获：真实模式（默认，skeletonMode=false）= Win32 BitBlt 虚拟桌面物理像素
    //    （含 PNG data URL + BGRA）；骨架模式（skeletonMode=true，CI/无桌面）仍合成帧。
    let skeleton_mode = state.is_skeleton();
    let capturer: Arc<dyn ScreenCapturer> = if skeleton_mode {
        Arc::new(SkeletonCapturer::new(app.clone()))
    } else {
        Arc::new(Win32Capturer::new())
    };
    let service = CaptureService::new(capturer);
    let frame = match service.freeze().await {
        Ok(frame) => frame,
        Err(err) => {
            // 捕获失败（含 SA-1001 黑屏/DRM）→ 清空冻结帧并取消会话，前端展示可操作错误。
            state.clear_frozen_frame();
            let _ = state.session.lock().cancel(&session);
            return Err(err);
        }
    };
    // 真实帧存入 AppState：selection_commit / 真实 OCR 从这里取含 BGRA 的完整帧。
    state.set_frozen_frame(frame.clone());

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

    // 5.6) 真实捕获几何对照 + 运行期证据落盘（仅开发钩子，env 未设置时零副作用）。
    //      Win32 虚拟屏幕（捕获源）与 Tauri available_monitors（overlay 尺寸）理论一致；
    //      不一致会导致选区绝对坐标整体偏移，必须记录。
    if !skeleton_mode {
        let mismatch = capture::geometry_mismatch(&frame, &desktop.rect);
        if let Some(diff) = &mismatch {
            tracing::warn!(target: "snip_ask_lib::trace", %diff, "真实捕获帧与 overlay 虚拟桌面几何不一致");
        }
        capture::maybe_dump_capture(
            &frame,
            serde_json::json!({
                "session": session.as_str(),
                "skeletonMode": false,
                "tauriVirtualDesktop": {
                    "x": desktop.rect.x,
                    "y": desktop.rect.y,
                    "width": desktop.rect.width,
                    "height": desktop.rect.height,
                },
                "overlayDpr": desktop.dpr,
                "geometryMismatch": mismatch,
            }),
        );
    }

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

    // 7) 前置 OCR：整屏 det（S-02 像素预算降采样）→ 缓存；时序保持 150ms scanning → 400ms done。
    let background_app = app.clone();
    let background_session = session.clone();
    let background_frame = frame.clone();
    // 帧左上角的绝对物理坐标（虚拟桌面左上角，可为负）；缓存框 = 帧局部 + 该 origin。
    let background_origin = desktop.rect;
    let background_ocr_key = ocr_session_id;
    let _handle = tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        tokio::time::sleep(Duration::from_millis(SKELETON_SCANNING_DELAY_MS)).await;
        emit_ocr_status(
            &background_app,
            &OcrStatusPayload {
                session: background_session.clone(),
                state: OcrStatusState::Scanning,
                boxes: None,
            },
        );

        // S-02/S-03：整屏 det 并缓存（骨架引擎瞬时；真实引擎 ~0.9s 在后台完成）。
        // 缓存命中的选区识别（rec-only）在 selection_commit 中使用，不重复 det。
        {
            let state = background_app.state::<AppState>();
            let config = state.snapshot_config();
            let scan = state
                .ocr
                .scan_screen(
                    &background_ocr_key,
                    &background_frame,
                    &background_origin,
                    config.ocr.det_max_edge,
                    config.ocr.text_threshold,
                )
                .await;
            match scan {
                Ok(result) => tracing::debug!(
                    target: "snip_ask_lib::trace",
                    "整屏 OCR det 完成：{} 框，hasText={}",
                    result.boxes.len(),
                    result.has_text
                ),
                Err(err) => tracing::warn!(
                    code = %err.code,
                    message = %err.message,
                    "整屏 OCR det 失败，选区将走 det+rec 兜底"
                ),
            }
        }

        // 契约时序：150ms scanning → 400ms done。det 更快时不提前、更慢时不再额外等待。
        let elapsed = started.elapsed().as_millis() as u64;
        tokio::time::sleep(Duration::from_millis(
            SKELETON_DONE_DELAY_MS.saturating_sub(elapsed),
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
    // 用户取消框选：清除热键设置的「框选后自动执行动作」，避免影响下一次普通 Ctrl+Q。
    state.set_pending_action(None);
    {
        let mut tasks = state.tasks.lock();
        tasks.cancel_session(&session);
    }
    {
        let mut store = state.session.lock();
        store.cancel(&session)?;
    }
    state.clear_frozen_frame();
    windows::hide_overlay(&app);
    Ok(())
}
