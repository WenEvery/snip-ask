//! 选区命令（契约 §6 第 3–4 条）。

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::{MenuReadyPayload, OcrStatusPayload, OcrStatusState};
use crate::contract::types::{RectPhys, SelectionResult, SessionId};
use crate::events::{emit_menu_ready, emit_ocr_status};
use crate::platform::capture::FrozenFrame;
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

    // S-02/S-03：优先用「整屏 det 缓存 → 选区过滤 + rec-only」路径（实测 P95 596ms）；
    // 缓存缺失/无命中时 pipeline 内部回退选区 det+rec（约 1s 级降级）。
    let (frame_meta, ocr_key) = {
        let store = state.session.lock();
        let current = store.current();
        (
            current.and_then(|item| item.frame.clone()),
            current
                .and_then(|item| item.ocr_session_id.clone())
                .unwrap_or_else(|| session.as_str().to_string()),
        )
    };
    // 帧左上角为虚拟桌面左上角（绝对物理坐标，可为负）；缓存框为绝对坐标，选区可直接比较。
    let desktop = crate::services::capture_service::virtual_desktop(&app);
    // 真实捕获的完整帧（含 bgra）从 AppState 取；缺失时退回会话 meta（骨架/旧会话，无 bgra）。
    let frozen = state.frozen_frame();
    let frame = frozen.or_else(|| frame_meta.as_ref().map(FrozenFrame::from_meta));
    let scan = match frame {
        Some(frame) => {
            // S-09 目标区域切片判定：选区纯黑/空 → SA-1001（混合 DPI 双屏整帧判定会漏检）。
            // 骨架帧无 bgra 时该函数直接通过。
            crate::services::capture_service::validate_region(&frame, &desktop.rect, &rect)?;
            state
                .ocr
                .recognize_selection(
                    &ocr_key,
                    &frame,
                    &desktop.rect,
                    &rect,
                    config.ocr.det_max_edge,
                    config.ocr.text_threshold,
                )
                .await?
        }
        // 理论不可达（选区只在冻结帧存在时产生）：退回占位结果，保证命令不因缓存缺失失败。
        None => ocr_service::placeholder_selection(&rect),
    };
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
    // 诊断（不记录文本内容）：行数/是否命中/字符数，便于反馈「部分文字没识别」时定位 det/rec 阶段。
    tracing::info!(
        boxes = boxes.len(),
        has_text,
        preview_chars = preview.chars().count(),
        "选区 OCR 完成"
    );

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
    // S-07 受限模式：按运行时能力（云端 Key / 本地模型）决定云动作可用性。
    // 注意：`config_view()` 会 await 凭据存储，必须放在 session 锁作用域之外。
    let config_view = state.config_view().await;
    let cloud_available = config_view
        .providers
        .iter()
        .any(|provider| provider.enabled && provider.has_key);
    let local_model_available = state.models.list().iter().any(|model| model.installed);
    let capabilities =
        crate::domain::action::Capabilities::derive(cloud_available, local_model_available);
    let items = crate::domain::action::visible_actions_with_capabilities(
        &config,
        result.has_text,
        &capabilities,
    );
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
