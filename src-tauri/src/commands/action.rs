//! 动作 / 任务命令（契约 §6 第 5–8 条）。
//!
//! 语义补充（契约修订）：
//! - `menu_execute` 允许在 `menu` **或** `showingResult` 态调用（结果窗「重答」）：
//!   会先取消该会话的进行中任务，再以新任务重跑（`parent = None`，清空线程）；
//! - `task_followup` 只在 `showingResult` 态合法，且**不再依赖 `TaskStore`**：
//!   任务完成后记录会被移除，改以「当前活动会话 + `session.task_ids.last()`」定位（修 F1）。

use std::collections::HashMap;

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::errors::{AppError, AppResult};
use crate::contract::types::{
    ActionId, FrozenFrameMeta, Mode, RectPhys, RunActionOptions, SelectionResult, SessionId, TaskId,
};
use crate::domain::action::mode_for;
use crate::domain::session::{Session, SessionPhase};
use crate::domain::turn::{self, FOLLOWUP_CONTEXT_MAX_CHARS};
use crate::services::action_router::ActionRouter;
use crate::services::task_runner::{self, TaskRequest};
use crate::services::translator;

/// 追问定位结果（纯逻辑，便于回归测试）。
#[derive(Debug, Clone, PartialEq)]
pub struct FollowupTarget {
    /// 当前活动会话。
    pub session: SessionId,
    /// 本轮追问的父任务（= 会话最近一次任务；`task:started.parent` 用它）。
    pub parent: TaskId,
    /// 会话选区（`{context}` 的选区原文 + 问答上下文来源）。
    pub selection: SelectionResult,
}

/// 追问相关的 SA-1099：`detail` 与 `message` 都带上可操作文案（前端直接内联展示 message）。
fn followup_state_error(message: impl Into<String>) -> AppError {
    let message = message.into();
    AppError::invalid_state(message.clone()).with_message(message)
}

/// 追问定位（**修 F1**）：不依赖 `TaskStore`，只依赖当前活动会话快照。
///
/// 原实现第一步即 `tasks.session_of(&task)`；任务完成后执行器会 `tasks.remove`，
/// 于是用户真实时序（回答完成后再追问）**必然** `SA-1099`。
///
/// 现语义（研究 §3.4 / 任务描述）：
/// 1. 必须有活动会话且处于 `showingResult`（流式中/其余阶段一律可读拒绝，提示等待）；
/// 2. `parent` 取 `session.task_ids.last()`（最近一次任务，含被取消的任务）；
/// 3. 入参 `task` 仅作校验：
///    - 属于本会话但**不是最近一轮** → 拒绝（只能对最近一轮追问）；
///    - 不存在（如已完成后被清理、旧窗口残留的 id）→ **回退最近一轮，不得必失败**。
pub fn locate_followup_target(
    current: Option<&Session>,
    requested: &TaskId,
) -> AppResult<FollowupTarget> {
    let current = current.ok_or_else(|| followup_state_error("没有活动会话，无法追问"))?;
    if current.phase != SessionPhase::ShowingResult {
        let message = if current.phase == SessionPhase::Executing {
            "当前回答仍在生成，请等待结束后再追问".to_string()
        } else {
            format!(
                "会话处于 {} 阶段，仅结果展示阶段允许追问",
                current.phase.as_str()
            )
        };
        return Err(followup_state_error(message));
    }
    let parent = current
        .task_ids
        .last()
        .cloned()
        .ok_or_else(|| followup_state_error("会话没有可追问的任务"))?;
    if requested != &parent && current.task_ids.contains(requested) {
        return Err(followup_state_error("只能对最近一轮回答继续追问"));
    }
    let selection = current
        .selection
        .clone()
        .ok_or_else(|| followup_state_error("会话缺少选区，请重新框选"))?;
    Ok(FollowupTarget {
        session: current.id.clone(),
        parent,
        selection,
    })
}

/// 命令：执行动作（`menu` / `showingResult` → `executing`）。
#[tauri::command]
pub async fn menu_execute(
    app: AppHandle,
    state: State<'_, AppState>,
    session: SessionId,
    action: ActionId,
    mode: Option<Mode>,
    options: Option<RunActionOptions>,
) -> AppResult<TaskId> {
    execute_action(&app, &state, session, action, mode, options).await
}

/// 动作执行核心（命令层与全局热键/自动执行共用；不含 Tauri `State` 包装）。
pub(crate) async fn execute_action(
    app: &AppHandle,
    state: &AppState,
    session: SessionId,
    action: ActionId,
    mode: Option<Mode>,
    options: Option<RunActionOptions>,
) -> AppResult<TaskId> {
    let config = state.snapshot_config();
    let options = options.unwrap_or_default();

    // 校验会话：允许 menu 与 showingResult（重答）。
    let selection = {
        let store = state.session.lock();
        if !store.is_current(&session) {
            return Err(AppError::invalid_state(format!(
                "会话 {session} 不是当前活动会话"
            )));
        }
        let phase = store.current_phase();
        if phase != SessionPhase::Menu && phase != SessionPhase::ShowingResult {
            return Err(AppError::invalid_state(format!(
                "会话处于 {} 阶段，不允许执行动作",
                phase.as_str()
            )));
        }
        store
            .current_selection()
            .ok_or_else(|| AppError::invalid_state("会话缺少选区，请重新框选"))?
    };

    // 重答 / 重跑：取消该会话进行中的旧任务（不再推送旧任务事件）。
    {
        let mut tasks = state.tasks.lock();
        let cancelled = tasks.cancel_session(&session);
        if !cancelled.is_empty() {
            tracing::info!(count = cancelled.len(), "已取消会话旧任务，准备重跑");
        }
    }

    let mode = mode.unwrap_or_else(|| mode_for(&config, action));
    let input_text = options
        .text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| selection.preview.clone());
    let target_lang = options
        .target_lang
        .clone()
        .unwrap_or_else(|| config.translate.target_lang.clone());

    let routed = ActionRouter.resolve(&config, action, mode, selection.has_text)?;
    // 「复制文字」不调用 LLM（v1.0.5）：不需要渲染提示词，原文即正文。
    let prompt = if action == ActionId::Copy {
        input_text.clone()
    } else {
        render_prompt(
            state,
            &routed.prompt_key,
            &input_text,
            &config,
            mode,
            &target_lang,
        )
    };

    // 显示结果窗（前端据此初始化流式容器）。
    if let Err(err) = crate::app::windows::show_result(app) {
        tracing::warn!(code = %err.code, "显示结果窗失败");
    }

    // 图片搜索：**只上传用户框选的区域**（v1.0.6 修复：此前误传整张虚拟桌面冻结帧），
    // 受 privacy.allowImageUpload / localOnly 约束。未允许上传或裁切失败时不带图，
    // task_runner 会退化为「本地 OCR 文本 → 检索 → 摘要」并如实标注「未上传图片」。
    let image_data_url = if action == ActionId::ImageSearch {
        if config.privacy.allow_image_upload && !config.privacy.local_only {
            let desktop = crate::services::capture_service::virtual_desktop(app);
            let cropped = state.frozen_frame().map(|frame| {
                crate::platform::capture::cropped_data_url(
                    &frame,
                    &desktop.rect,
                    &selection.rect_phys,
                )
            });
            match cropped {
                Some(Ok(Some(url))) => {
                    tracing::info!(
                        data_url_chars = url.chars().count(),
                        selection_width = selection.rect_phys.width,
                        selection_height = selection.rect_phys.height,
                        "图片搜索：已附带选区截图（仅框选区域，不含整桌面）"
                    );
                    Some(url)
                }
                Some(Ok(None)) => {
                    tracing::info!("图片搜索：未能从冻结帧裁出选区，回退本地 OCR 文本");
                    None
                }
                Some(Err(err)) => {
                    tracing::warn!(
                        code = %err.code,
                        detail = %err.detail.as_deref().unwrap_or(""),
                        "图片搜索：选区截图编码失败，回退本地 OCR 文本"
                    );
                    None
                }
                None => {
                    tracing::info!("图片搜索：未取到冻结帧，回退本地 OCR 文本");
                    None
                }
            }
        } else {
            tracing::info!("图片搜索：隐私设置禁止上传图片，回退本地 OCR 文本");
            None
        }
    } else {
        None
    };

    let request = TaskRequest {
        session,
        action,
        mode,
        routed,
        prompt,
        input_text,
        // 首轮 / 重答：parent = None（前端据此重置线程，`task_runner` 清空会话轮次）。
        parent: None,
        search_template: config.search.engine_url_template.clone(),
        image_data_url,
        options,
    };
    task_runner::spawn(app.clone(), request)
}

/// 无屏幕选区的文本动作（当前用于「快速翻译」剪贴板热键）。
///
/// 复用正常任务链路：创建一个仅含文本的会话/选区（无真实截图），再调用 [`execute_action`]。
/// 这样结果窗、历史轮次、重答与错误展示都与普通动作一致，不需要新增命令或事件。
pub(crate) async fn execute_text_action(
    app: &AppHandle,
    state: &AppState,
    action: ActionId,
    text: String,
) -> AppResult<TaskId> {
    if text.trim().is_empty() {
        return Err(
            AppError::invalid_config("text", "剪贴板为空，没有可翻译的文字")
                .with_message("剪贴板里没有可翻译的文字：请先复制文本再按快速翻译热键")
                .with_fallback(vec!["retry"]),
        );
    }
    let start = { state.session.lock().begin_capture() };
    for task in &start.replaced_tasks {
        state.tasks.lock().cancel(task);
    }
    if let Some(replaced) = start.replaced_session.as_ref() {
        state.turns.lock().drop_session(replaced);
    }
    let session = start.session;
    {
        let mut store = state.session.lock();
        store.attach_frame(
            &session,
            FrozenFrameMeta {
                width: 1,
                height: 1,
                data_url: None,
            },
        )?;
        store.set_selecting(&session)?;
        store.commit_selection(
            &session,
            SelectionResult {
                session: session.clone(),
                rect_phys: RectPhys {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
                has_text: true,
                preview: text,
                boxes: Vec::new(),
            },
        )?;
    }
    state.clear_frozen_frame();
    execute_action(app, state, session, action, None, None).await
}

/// 命令：取消任务（停止流，不再推送该任务事件）。
#[tauri::command]
pub fn task_cancel(app: AppHandle, task: TaskId) -> AppResult<()> {
    let cancelled = task_runner::request_cancel(&app, &task);
    tracing::debug!(task = %task, cancelled = cancelled, "取消任务");
    Ok(())
}

/// 命令：追问（`showingResult` → `executing`，新任务 ID）。
///
/// 修 F1：按「当前活动会话 + 最近一次任务」定位，不再要求任务仍在 `TaskStore` 中。
#[tauri::command]
pub async fn task_followup(
    app: AppHandle,
    state: State<'_, AppState>,
    task: TaskId,
    text: String,
) -> AppResult<TaskId> {
    let config = state.snapshot_config();

    // 1) 定位（不依赖 TaskStore）：当前会话快照 → 最近一轮任务 → 选区。
    let current = { state.session.lock().current().cloned() };
    let target = locate_followup_target(current.as_ref(), &task)?;

    // 2) 多轮上下文：选区原文 + 最近 N 轮问答（研究 §3.7 MVP）。
    let context = {
        let turns = state.turns.lock();
        turn::build_followup_context(
            &turns,
            &target.session,
            &target.selection.preview,
            FOLLOWUP_CONTEXT_MAX_CHARS,
        )
    };

    let routed = ActionRouter.resolve_followup(&config, target.selection.has_text)?;
    let mut vars: HashMap<&str, &str> = HashMap::new();
    vars.insert("question", text.as_str());
    vars.insert("context", context.as_str());
    let prompt = state
        .prompt_engine()
        .render(&routed.prompt_key, &vars)
        .unwrap_or_else(|err| {
            tracing::warn!(code = %err.code, "追问提示词渲染失败，使用原始问题");
            text.clone()
        });

    task_runner::spawn(
        app,
        TaskRequest {
            session: target.session,
            action: ActionId::Answer,
            mode: config.modes.answer,
            routed,
            prompt,
            input_text: text,
            // 追问：parent 指向上一轮任务（前端据此追加线程气泡）。
            parent: Some(target.parent),
            search_template: None,
            image_data_url: None,
            options: RunActionOptions::default(),
        },
    )
}

/// 命令：关闭结果窗并释放会话（任意非 idle 态 → idle）。
#[tauri::command]
pub async fn result_close(
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
        store.close(&session)?;
    }
    // 关窗即结束对话：清空内存线程（研究 §3.2 方案 A）。
    state.turns.lock().clear(&session);
    crate::app::windows::hide_result(&app);
    crate::app::windows::hide_overlay(&app);
    Ok(())
}

fn render_prompt(
    state: &AppState,
    prompt_key: &str,
    input_text: &str,
    config: &crate::contract::config::Config,
    mode: Mode,
    target_lang: &str,
) -> String {
    let lang = match config.app.language {
        crate::contract::config::Language::ZhCn => "zh-CN",
        crate::contract::config::Language::EnUs => "en-US",
    };
    let glossary = translator::render_glossary(config);
    let mut vars: HashMap<&str, &str> = HashMap::new();
    vars.insert("text", input_text);
    vars.insert("lang", lang);
    vars.insert("mode", mode.as_str());
    vars.insert("targetLang", target_lang);
    vars.insert("sourceLang", "auto");
    vars.insert("glossary", glossary.as_str());
    vars.insert("question", "");
    vars.insert("context", "");

    state
        .prompt_engine()
        .render(prompt_key, &vars)
        .unwrap_or_else(|err| {
            tracing::warn!(code = %err.code, key = %prompt_key, "提示词渲染失败，回退原始文本");
            input_text.to_string()
        })
}
