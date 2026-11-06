//! 选区命令（契约 §6 第 3–4 条）。

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::{MenuReadyPayload, OcrStatusPayload, OcrStatusState};
use crate::contract::types::{ActionId, ActionItem, OcrBox, RectPhys, SelectionResult, SessionId};
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
    let recognized = ocr_service::filter_boxes(&scan.boxes, &rect, config.ocr.text_threshold);
    let has_text = ocr_service::presence(&recognized, &rect, config.ocr.text_threshold);
    let preview = {
        let merged = ocr_service::merge_text(&recognized);
        if merged.is_empty() {
            scan.preview.clone()
        } else {
            merged
        }
    };
    // v1.1.0：`ocr.preserveBoxes = false` 时不回传文字框（`hasText` / `preview` 保留），
    // 依赖 boxes 的覆盖式翻译因此不可用。
    let boxes = result_boxes(recognized, config.ocr.preserve_boxes);
    // 诊断（不记录文本内容）：行数/是否命中/字符数，便于反馈「部分文字没识别」时定位 det/rec 阶段。
    tracing::info!(
        boxes = boxes.len(),
        preserve_boxes = config.ocr.preserve_boxes,
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
    // v1.1.0：`providers[].caps` 参与动作可用性 —— 只统计 `enabled && hasKey` 的供应商，
    // 分别派生 vision / search / translate（`kind = qwenMt` 视为具备翻译能力）。
    let mut cloud_available = false;
    let mut cloud_caps = crate::domain::action::CloudCaps::default();
    for provider in config_view
        .providers
        .iter()
        .filter(|provider| provider.enabled && provider.has_key)
    {
        cloud_available = true;
        cloud_caps.add_provider(
            &provider.caps,
            provider.kind == crate::contract::config::ProviderKind::QwenMt,
        );
    }
    let local_model_available = state.models.list().iter().any(|model| model.installed);
    let capabilities = crate::domain::action::Capabilities::derive_from_cloud_caps(
        cloud_available,
        cloud_caps,
        local_model_available,
    );
    let items = crate::domain::action::visible_actions_with_capabilities(
        &config,
        result.has_text,
        &capabilities,
    );
    // 自动执行策略（v1.1.0）：
    // 1. 热键「图片搜索 / 解答选区」触发的 pendingAction 优先级最高（v1.0.6）；
    // 2. 无热键 pending 时，`actions.defaultAction` 若在当前菜单可用则自动执行一次（此前未生效）；
    // 3. 自动执行失败 / 不可用 → 回退动作菜单（下方后台事件）。
    // 热键优先：即使 pending 动作当前不可用，也不回落到 defaultAction。
    let pending_action = state.take_pending_action();
    let decision = decide_auto_action(pending_action, config.actions.default_action, &items);
    if let Some(decision) = decision {
        let action = decision.action;
        emit_ocr_status(
            &app,
            &OcrStatusPayload {
                session: result.session.clone(),
                state: OcrStatusState::Done,
                boxes: Some(result.boxes.clone()),
            },
        );
        let mode = crate::domain::action::mode_for(&config, action);
        match crate::commands::action::execute_action(
            &app,
            &state,
            result.session.clone(),
            action,
            Some(mode),
            None,
        )
        .await
        {
            Ok(_) => {
                crate::app::windows::hide_overlay(&app);
                return Ok(result);
            }
            Err(err) => {
                tracing::warn!(
                    action = action.as_str(),
                    source = decision.source.as_str(),
                    code = %err.code,
                    message = %err.message,
                    "自动执行动作失败，回退显示动作菜单"
                );
            }
        }
    } else if let Some(action) = pending_action {
        tracing::info!(
            action = action.as_str(),
            "热键动作当前不可用，回退显示动作菜单"
        );
    }

    // 异步：先 ocr:status(done)，再 menu:ready（不阻塞命令返回）。
    // S-07 受限模式：按运行时能力（云端 Key / 本地模型）决定云动作可用性。
    // 注意：`config_view()` 会 await 凭据存储，必须放在 session 锁作用域之外。
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

// ---------------------------------------------------------------------------
// 自动执行决策（v1.1.0：`actions.defaultAction` 真正生效）
// ---------------------------------------------------------------------------

/// 自动执行来源（诊断日志 / 单测断言）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoActionSource {
    /// 全局热键触发的 `pendingAction`（图片搜索 / 解答选区）。
    Hotkey,
    /// 配置项 `actions.defaultAction`。
    DefaultAction,
}

impl AutoActionSource {
    /// 日志用短名。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hotkey => "hotkey",
            Self::DefaultAction => "defaultAction",
        }
    }
}

/// 框选提交后的自动执行决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoActionDecision {
    /// 要执行的动作。
    pub action: ActionId,
    /// 决策来源。
    pub source: AutoActionSource,
}

/// 决定框选提交后是否自动执行动作（纯逻辑，可单测）。
///
/// 规则（v1.1.0）：
/// 1. 热键 `pendingAction` 优先：即使它当前不可用，也不回落到 `defaultAction`（用户明确按了热键）；
/// 2. 无 pending 时才看 `actions.defaultAction`，且该动作必须在当前菜单里**可用**；
/// 3. 其余情况返回 `None`，由调用方显示动作菜单。
pub fn decide_auto_action(
    pending: Option<ActionId>,
    default_action: Option<ActionId>,
    items: &[ActionItem],
) -> Option<AutoActionDecision> {
    let available = |action: ActionId| items.iter().any(|item| item.id == action && item.enabled);
    if let Some(action) = pending {
        return available(action).then_some(AutoActionDecision {
            action,
            source: AutoActionSource::Hotkey,
        });
    }
    let action = default_action?;
    available(action).then_some(AutoActionDecision {
        action,
        source: AutoActionSource::DefaultAction,
    })
}

/// 按 `ocr.preserveBoxes` 决定返回给前端的文字框。
///
/// `false` 时返回空列表（不回传文字框），`hasText` 与 `preview` 仍按识别结果保留；
/// 覆盖式翻译依赖 boxes，因此该设置关闭后覆盖式翻译不可用。
pub fn result_boxes(boxes: Vec<OcrBox>, preserve: bool) -> Vec<OcrBox> {
    if preserve {
        boxes
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::types::{ActionTarget, Mode};

    fn item(id: ActionId, enabled: bool) -> ActionItem {
        ActionItem {
            id,
            label: id.as_str().to_string(),
            target: ActionTarget::Both,
            mode: Mode::Quick,
            enabled,
            disabled_reason: if enabled {
                None
            } else {
                Some("当前不可用".to_string())
            },
            shortcut: None,
        }
    }

    fn ocr_box(text: &str) -> OcrBox {
        OcrBox {
            text: text.to_string(),
            confidence: 0.9,
            bbox: RectPhys {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
        }
    }

    #[test]
    fn hotkey_pending_wins_over_default_action() {
        let items = [item(ActionId::Translate, true), item(ActionId::Copy, true)];
        assert_eq!(
            decide_auto_action(Some(ActionId::Translate), Some(ActionId::Copy), &items),
            Some(AutoActionDecision {
                action: ActionId::Translate,
                source: AutoActionSource::Hotkey,
            })
        );
    }

    #[test]
    fn unavailable_hotkey_does_not_fall_back_to_default_action() {
        // 热键优先：pending 不可用时直接回退菜单，不应改为执行 defaultAction。
        let items = [
            item(ActionId::ImageSearch, false),
            item(ActionId::Copy, true),
        ];
        assert_eq!(
            decide_auto_action(Some(ActionId::ImageSearch), Some(ActionId::Copy), &items),
            None
        );
        // pending 不在当前菜单（如文本选区按了图片搜索热键）同样不回落。
        let items = [item(ActionId::Copy, true)];
        assert_eq!(
            decide_auto_action(Some(ActionId::ImageSearch), Some(ActionId::Copy), &items),
            None
        );
    }

    #[test]
    fn default_action_runs_without_hotkey_pending() {
        let items = [item(ActionId::Translate, true), item(ActionId::Copy, true)];
        assert_eq!(
            decide_auto_action(None, Some(ActionId::Copy), &items),
            Some(AutoActionDecision {
                action: ActionId::Copy,
                source: AutoActionSource::DefaultAction,
            })
        );
    }

    #[test]
    fn default_action_must_be_enabled_in_current_menu() {
        // 动作在当前菜单但被禁用（隐私 / 能力门禁）→ 不自动执行。
        let items = [item(ActionId::Answer, false)];
        assert_eq!(
            decide_auto_action(None, Some(ActionId::Answer), &items),
            None
        );
        // 动作不在当前菜单（文本 / 图片菜单差异）→ 不自动执行。
        let items = [item(ActionId::Copy, true)];
        assert_eq!(
            decide_auto_action(None, Some(ActionId::Answer), &items),
            None
        );
    }

    #[test]
    fn no_pending_and_no_default_action_keeps_menu() {
        let items = [item(ActionId::Search, true)];
        assert_eq!(decide_auto_action(None, None, &items), None);
    }

    #[test]
    fn preserve_boxes_switch_controls_returned_boxes() {
        let boxes = vec![ocr_box("第一行"), ocr_box("第二行")];
        // preserveBoxes = true：原样返回（覆盖翻译可用）。
        assert_eq!(result_boxes(boxes.clone(), true).len(), 2);
        // preserveBoxes = false：清空 boxes（hasText / preview 不受影响，覆盖翻译不可用）。
        assert!(result_boxes(boxes, false).is_empty());
    }
}
