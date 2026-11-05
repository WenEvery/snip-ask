//! 统一流式任务执行器（脚手架新增，M08/M09/M13）。
//!
//! 职责：登记任务 → 发 `task:started`（带 `parent`）→ 流式执行（逐个 `task:delta`）→
//! `task:done` / `task:error`；支持取消、错误映射、历史落库与配置降级。
//!
//! 多轮追问（契约 v4 / 研究 §3.4）：
//! - `parent = Some(上一轮任务)` 表示追问；`None` 表示首轮或重答（重答会清空该会话轮次）；
//! - 成功完成时把「本次输入 + 累积完整回答 + 引擎」写入 [`AppState::turns`]，
//!   供 `task_followup` 注入 `followup.md` 的 `{context}`；**取消/失败不写正文**。
//!
//! 取消语义：`task_cancel` 置位 `AtomicBool`，网关在流式块之间检查后返回 `SA-1099`；
//! 执行器识别「已取消」后不再推送该任务的任何事件（契约 §6 第 6 条）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager};

use crate::app::state::AppState;
use crate::contract::errors::{AppError, AppResult};
use crate::contract::events::{TaskDeltaPayload, TaskErrorPayload, TaskStartedPayload};
use crate::contract::types::{
    ActionId, Currency, Mode, RunActionOptions, SessionId, StreamKind, TaskDonePayload, TaskId,
    TokenUsage,
};
use crate::domain::ids::new_task_id;
use crate::domain::task::{TaskKind, TaskRecord};
use crate::domain::turn::Turn;
use crate::events::{emit_task_delta, emit_task_done, emit_task_error, emit_task_started};
use crate::services::action_router::RoutedAction;
use crate::services::llm_gateway::{LlmGateway, LlmRequest, StreamChunk, TaskOutcome};
use crate::services::search_gateway::{
    build_search_url, open_external, SearchRequest, Searcher, SkeletonSearcher,
};

/// 任务执行请求（命令层组装）。
#[derive(Debug, Clone)]
pub struct TaskRequest {
    /// 所属会话。
    pub session: SessionId,
    /// 动作。
    pub action: ActionId,
    /// 模式。
    pub mode: Mode,
    /// 路由结果。
    pub routed: RoutedAction,
    /// 渲染后的提示词。
    pub prompt: String,
    /// 用户输入原文。
    pub input_text: String,
    /// 父任务：追问 = 上一轮任务 ID；首轮 / 重答 = `None`（契约 v4）。
    pub parent: Option<TaskId>,
    /// 自定义搜索 URL 模板（浏览器搜索用）。
    pub search_template: Option<String>,
    /// 命令可选参数。
    pub options: RunActionOptions,
}

impl TaskRequest {
    /// 本轮写入 TurnStore 的「问题」：追问为问题本身，首轮为原始输入（选区文本）。
    pub fn turn_question(&self) -> String {
        self.input_text.clone()
    }
}

/// 执行结果 + 累积正文（多轮上下文的数据源）。
#[derive(Debug, Clone)]
struct ExecutedTask {
    /// 网关结算结果。
    outcome: TaskOutcome,
    /// 累积的正文（`kind == Text` 的 chunk），失败/取消路径不落 TurnStore。
    text: String,
}

/// 启动任务；返回任务 ID。任务执行在后台 tokio 任务中进行。
pub fn spawn(app: AppHandle, request: TaskRequest) -> AppResult<TaskId> {
    let state = app.state::<AppState>();
    let task_id = new_task_id();

    // 1) 会话状态机：menu/showingResult → executing。
    {
        let mut session = state.session.lock();
        session.begin_task(&request.session, task_id.clone())?;
    }

    // 2) 轮次表：首轮 / 重答（parent = None）开启新线程；追问沿用已有线程。
    {
        let mut turns = state.turns.lock();
        if request.parent.is_none() {
            turns.prune_except(&request.session);
            turns.begin(&request.session);
        }
    }

    // 3) 任务登记 + 取消令牌。
    let record = TaskRecord::new(
        task_id.clone(),
        request.session.clone(),
        request.action,
        request.mode,
    );
    let cancel = record.cancel_token();
    {
        let mut tasks = state.tasks.lock();
        tasks.insert(record);
    }

    // 4) 首个 delta 之前发出 task:started（`parent` 让前端区分「首轮/重答」与「追问」）。
    emit_task_started(
        &app,
        &TaskStartedPayload {
            task: task_id.clone(),
            session: request.session.clone(),
            action: request.action,
            mode: request.mode,
            engine: request.routed.engine.clone(),
            parent: request.parent.clone(),
        },
    );

    // 5) 后台流式执行。
    let background_app = app.clone();
    let background_task = task_id.clone();
    let _handle = tauri::async_runtime::spawn(async move {
        run_task(background_app, background_task, request, cancel).await;
    });

    Ok(task_id)
}

/// 取消任务（幂等）；返回是否命中活动任务。
pub fn request_cancel(app: &AppHandle, task: &TaskId) -> bool {
    let state = app.state::<AppState>();
    let mut tasks = state.tasks.lock();
    tasks.cancel(task)
}

/// 该任务是否仍是会话当前线程的「最近一轮」。
///
/// 重答 / 新框选会取消旧任务并立刻启动新任务；旧任务可能在之后才结束（或才发现取消标记）。
/// 这类「已被取代」的任务不得再写轮次或推进会话阶段，否则会把新任务的 `executing`
/// 拉回 `showingResult`（出现并发追问）。
fn is_current_thread(state: &AppState, session: &SessionId, task_id: &TaskId) -> bool {
    let store = state.session.lock();
    store.is_current(session) && store.last_task_id().as_ref() == Some(task_id)
}

async fn run_task(app: AppHandle, task_id: TaskId, request: TaskRequest, cancel: Arc<AtomicBool>) {
    let started = Instant::now();
    let executed = execute(&app, &task_id, &request, &cancel).await;
    let state = app.state::<AppState>();

    match executed {
        Ok(executed) => {
            let ExecutedTask { outcome, text } = executed;
            if cancel.load(Ordering::SeqCst) {
                finish_cancelled(&state, &task_id, &request);
                return;
            }
            let current_thread = is_current_thread(&state, &request.session, &task_id);
            // 多轮上下文：先记录本轮问答（完整正文），再发 `task:done`。
            // 顺序很重要：前端收到 done 后用户可能立刻追问，此时 `{context}` 必须已包含本轮。
            // 取消 / 失败 / 已被取代的路径不写正文。
            if current_thread {
                state.turns.lock().record(
                    &request.session,
                    Turn::now(
                        task_id.clone(),
                        request.turn_question(),
                        text,
                        outcome.engine.clone(),
                        request.parent.clone(),
                    ),
                );
            }
            emit_task_done(
                &app,
                &TaskDonePayload {
                    task: task_id.clone(),
                    usage: outcome.usage.clone(),
                    latency_ms: outcome.latency_ms,
                    engine: outcome.engine.clone(),
                    sources: outcome.sources.clone(),
                },
            );
            if current_thread {
                let mut session = state.session.lock();
                let _ = session.task_finished(&request.session);
            }
            let removed = {
                let mut tasks = state.tasks.lock();
                tasks.remove(&task_id)
            };
            if let Some(record) = removed {
                state.history.record_task(
                    &record,
                    Some(outcome.engine.as_str()),
                    "done",
                    None,
                    Some(started.elapsed().as_millis() as u64),
                    Some(&outcome.usage),
                    None,
                );
            }
        }
        Err(error) => {
            if cancel.load(Ordering::SeqCst) {
                finish_cancelled(&state, &task_id, &request);
                return;
            }
            emit_task_error(
                &app,
                &TaskErrorPayload {
                    task: task_id.clone(),
                    code: error.code.clone(),
                    message: error.message.clone(),
                    retryable: error.retryable,
                    fallback: error.fallback.clone(),
                },
            );
            if is_current_thread(&state, &request.session, &task_id) {
                // 契约 v4：失败回到结果态（showingResult），错误内联展示，之后仍可追问/重答。
                let mut session = state.session.lock();
                let _ = session.task_failed(&request.session);
            }
            let removed = {
                let mut tasks = state.tasks.lock();
                tasks.remove(&task_id)
            };
            if let Some(record) = removed {
                state.history.record_task(
                    &record,
                    Some(request.routed.engine.as_str()),
                    "error",
                    None,
                    Some(started.elapsed().as_millis() as u64),
                    None,
                    Some(error.code.as_str()),
                );
            }
        }
    }
}

fn finish_cancelled(state: &AppState, task_id: &TaskId, request: &TaskRequest) {
    // 取消后不再推送事件；会话回到结果态以便关闭 / 追问。
    // 仅当该任务仍是当前线程的最近一轮时才推进阶段：重答会先取消旧任务再启动新任务，
    // 旧任务此时才发现取消标记，不能把新任务的 executing 拉回 showingResult。
    if is_current_thread(state, &request.session, task_id) {
        let mut session = state.session.lock();
        let _ = session.task_finished(&request.session);
    }
    let removed = {
        let mut tasks = state.tasks.lock();
        tasks.remove(task_id)
    };
    if let Some(record) = removed {
        state.history.record_task(
            &record,
            Some(request.routed.engine.as_str()),
            "cancelled",
            None,
            None,
            None,
            None,
        );
    }
}

async fn execute(
    app: &AppHandle,
    task_id: &TaskId,
    request: &TaskRequest,
    cancel: &Arc<AtomicBool>,
) -> AppResult<ExecutedTask> {
    // 浏览器搜索分支：不走 LLM，直接拼 URL（骨架模式只记日志）。
    if request.routed.kind == TaskKind::Search && request.routed.engine == "browser" {
        return run_browser_search(app, task_id, request, cancel).await;
    }

    // 网关选择（契约 v4）：骨架模式 / 本地引擎（Vulkan 优先）/ 云端真实 SSE。
    let gateway: Arc<dyn LlmGateway> = {
        let state = app.state::<AppState>();
        state.gateway_for(app, &request.routed).await?
    };
    let llm_request = LlmRequest {
        task: task_id.clone(),
        session: request.session.clone(),
        action: request.action,
        mode: request.mode,
        engine: request.routed.engine.clone(),
        prompt: request.prompt.clone(),
        input_text: request.input_text.clone(),
        image_data_url: None,
        cancel: cancel.clone(),
    };
    let collected = Arc::new(Mutex::new(String::new()));
    let sink_app = app.clone();
    let sink_task = task_id.clone();
    let sink_collected = collected.clone();
    let sink = move |chunk: StreamChunk| {
        if chunk.kind == StreamKind::Text {
            sink_collected.lock().push_str(&chunk.chunk);
        }
        emit_task_delta(
            &sink_app,
            &TaskDeltaPayload {
                task: sink_task.clone(),
                kind: chunk.kind,
                chunk: chunk.chunk,
            },
        );
    };
    let outcome = gateway.stream(&llm_request, &sink).await?;
    let text = collected.lock().clone();
    Ok(ExecutedTask { outcome, text })
}

async fn run_browser_search(
    app: &AppHandle,
    task_id: &TaskId,
    request: &TaskRequest,
    cancel: &Arc<AtomicBool>,
) -> AppResult<ExecutedTask> {
    let started = Instant::now();
    let searcher = SkeletonSearcher;
    let search_request = SearchRequest {
        engine: request.routed.engine.clone(),
        template: request.search_template.clone(),
        query: request.input_text.clone(),
        mode: request.mode,
    };
    let outcome = searcher.search(&search_request).await?;

    let url = build_search_url(
        &request.routed.engine,
        request.search_template.as_deref(),
        &request.input_text,
    );
    if let Err(err) = open_external(Some(app), &url).await {
        tracing::warn!(code = %err.code, "打开浏览器搜索被拒绝");
    }

    if cancel.load(Ordering::SeqCst) {
        return Err(AppError::invalid_state("任务已取消"));
    }

    emit_task_delta(
        app,
        &TaskDeltaPayload {
            task: task_id.clone(),
            kind: StreamKind::Text,
            chunk: outcome.text.clone(),
        },
    );

    Ok(ExecutedTask {
        outcome: TaskOutcome {
            usage: TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                cached_input_tokens: 0,
                cost_estimate: 0.0,
                currency: Currency::Cny,
            },
            latency_ms: started.elapsed().as_millis() as u64,
            engine: request.routed.engine.clone(),
            sources: outcome.sources,
        },
        text: outcome.text,
    })
}
