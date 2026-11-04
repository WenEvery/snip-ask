//! 动作 / 任务命令（契约 §6 第 5–8 条）。
//!
//! 语义补充（契约修订）：
//! - `menu_execute` 允许在 `menu` **或** `showingResult` 态调用（结果窗「重答」）：
//!   会先取消该会话的进行中任务，再以新任务重跑；
//! - `task_followup` 只在 `showingResult` 态合法。

use std::collections::HashMap;

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::errors::{AppError, AppResult};
use crate::contract::types::{ActionId, Mode, RunActionOptions, SessionId, TaskId};
use crate::domain::action::mode_for;
use crate::domain::session::SessionPhase;
use crate::services::action_router::ActionRouter;
use crate::services::task_runner::{self, TaskRequest};
use crate::services::translator;

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
    let config = state.snapshot_config();
    let options = options.unwrap_or_default();

    // 校验会话：允许 menu 与 showingResult（重答）。
    let selection = {
        let store = state.session.lock();
        if !store.is_current(&session) {
            return Err(AppError::invalid_state(format!("会话 {session} 不是当前活动会话")));
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
    let prompt = render_prompt(
        &state,
        &routed.prompt_key,
        &input_text,
        &config,
        mode,
        &target_lang,
    );

    // 显示结果窗（前端据此初始化流式容器）。
    if let Err(err) = crate::app::windows::show_result(&app) {
        tracing::warn!(code = %err.code, "显示结果窗失败");
    }

    let request = TaskRequest {
        session,
        action,
        mode,
        routed,
        prompt,
        input_text,
        search_template: config.search.engine_url_template.clone(),
        options,
    };
    task_runner::spawn(app, request)
}

/// 命令：取消任务（停止流，不再推送该任务事件）。
#[tauri::command]
pub fn task_cancel(app: AppHandle, task: TaskId) -> AppResult<()> {
    let cancelled = task_runner::request_cancel(&app, &task);
    tracing::debug!(task = %task, cancelled = cancelled, "取消任务");
    Ok(())
}

/// 命令：追问（`showingResult` → `executing`，新任务 ID）。
#[tauri::command]
pub async fn task_followup(
    app: AppHandle,
    state: State<'_, AppState>,
    task: TaskId,
    text: String,
) -> AppResult<TaskId> {
    let config = state.snapshot_config();

    let session = {
        let tasks = state.tasks.lock();
        tasks
            .session_of(&task)
            .ok_or_else(|| AppError::invalid_state(format!("任务 {task} 不存在")))?
    };

    let selection = {
        let store = state.session.lock();
        if !store.is_current(&session) {
            return Err(AppError::invalid_state("追问所属会话已结束"));
        }
        if store.current_phase() != SessionPhase::ShowingResult {
            return Err(AppError::invalid_state(format!(
                "会话处于 {} 阶段，仅结果展示阶段允许追问",
                store.current_phase().as_str()
            )));
        }
        store
            .current_selection()
            .ok_or_else(|| AppError::invalid_state("会话缺少选区"))?
    };

    let routed = ActionRouter.resolve_followup(&config, selection.has_text)?;
    let mut vars: HashMap<&str, &str> = HashMap::new();
    vars.insert("question", text.as_str());
    vars.insert("context", selection.preview.as_str());
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
            session,
            action: ActionId::Answer,
            mode: config.modes.answer,
            routed,
            prompt,
            input_text: text,
            search_template: None,
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
