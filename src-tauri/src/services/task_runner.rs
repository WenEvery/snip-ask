//! 统一流式任务执行器（M08/M09/M13 / 契约 v4 + v5 §3.4）。
//!
//! 职责：登记任务 → 发 `task:started`（带 `parent`）→ 流式执行（逐个 `task:delta`）→
//! `task:done` / `task:error`；支持取消、错误映射、历史落库与配置降级。
//!
//! # v1.0.0 真实云端链路（非骨架模式）
//!
//! - **文本 AI 搜索**：query → DeepSeek `tools/search` → deep 模式并发 `tools/fetch` →
//!   以 `text_search.md` 渲染的提示词 + 真实来源证据流式生成结论；
//! - **图片搜索（四阶段）**：视觉识别（多模态）→ 检索词生成 → `tools/search` →
//!   deep 模式并发 `tools/fetch` → 以 `image_search.md` 渲染的提示词流式生成结论；
//! - **翻译**：`translator::translate_attempts` 的降级链（云端 provider / Qwen-MT /
//!   本地引擎），LLM 后端实时流式、Qwen-MT 原生端点非流式后一次性输出；
//! - **浏览器搜索**：`open_external` 用 `tauri_plugin_opener` 真实打开系统浏览器。
//!
//! 骨架模式（`advanced.skeletonMode=true`）保持契约 §12 的确定性占位行为（CI/浏览器预览）。
//!
//! # 隐私
//!
//! 图片上传由 `privacy.allowImageUpload` 在命令层控制：未允许上传时 `image_data_url = None`，
//! 图片搜索退化为「本地 OCR 文本 → 检索 → 摘要」，并在提示词里如实标注「未上传图片」；
//! `privacy.localOnly=true` 时禁止任何联网检索（返回可操作错误，不伪造结果）。
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
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::app::state::AppState;
use crate::contract::config::{Config, ProviderKind};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::{TaskDeltaPayload, TaskErrorPayload, TaskStartedPayload};
use crate::contract::types::{
    ActionId, Currency, Mode, RunActionOptions, SessionId, SourceRef, StreamKind, TaskDonePayload,
    TaskId, TokenUsage,
};
use crate::domain::ids::new_task_id;
use crate::domain::task::{TaskKind, TaskRecord};
use crate::domain::turn::Turn;
use crate::events::{emit_task_delta, emit_task_done, emit_task_error, emit_task_started};
use crate::services::action_router::RoutedAction;
use crate::services::llm_gateway::{LlmGateway, LlmRequest, StreamChunk, TaskOutcome};
use crate::services::local_gateway::HttpLlmGateway;
use crate::services::search_gateway::{
    self, build_search_url, fetch_concurrently, open_external, DeepSeekToolsSearcher, FetchedPage,
    SearchHit, Searcher, SkeletonSearcher,
};
use crate::services::translator::{self, TranslateAttemptKind, TranslateRequest};

/// 快速模式单次搜索返回条数（≤ 真实协议上限 50）。
pub const QUICK_SEARCH_COUNT: usize = 8;
/// 深度模式单次搜索返回条数。
pub const DEEP_SEARCH_COUNT: usize = 12;
/// 深度模式最多并发 Fetch 页数（真实协议 5 QPS/端点，取 4 以内足够）。
pub const MAX_FETCH_PAGES: usize = 4;
/// 传给摘要模型的每条来源正文上限（字符）。
pub const EVIDENCE_PER_SOURCE_CHARS: usize = 1600;
/// 传给摘要模型的证据总量上限（字符，防止长上下文拖慢首字/超限）。
pub const EVIDENCE_TOTAL_CHARS: usize = 8000;
/// 视觉识别描述上限（字符）。
pub const VISION_DESCRIPTION_MAX_CHARS: usize = 3000;
/// 最终来源列表上限（与契约 `sources` 一致，避免 UI 过长）。
pub const MAX_SOURCES: usize = 10;
/// 图片模式生成检索词的最大条数。
pub const MAX_IMAGE_QUERIES: usize = 3;
/// 检索词上限（真实 Search 协议 ≤128 Unicode 字符）。
pub const MAX_QUERY_CHARS: usize = 128;

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
    /// 图片 DataURL（契约 v5：图片搜索用；受 `privacy.allowImageUpload` 约束）。
    pub image_data_url: Option<String>,
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

/// 研究流水线选项（文本 AI 搜索 / 图片搜索共用）。
#[derive(Debug, Clone)]
pub struct ResearchOptions {
    /// 模式（quick / deep）。
    pub mode: Mode,
    /// 输出语言（`zh-CN` / `en-US`）。
    pub lang: String,
    /// 单次搜索返回条数。
    pub search_count: usize,
    /// deep 模式并发 Fetch 页数（0 = 不 Fetch）。
    pub fetch_pages: usize,
    /// 每条来源正文截断上限（字符）。
    pub fetch_max_chars: usize,
    /// 证据总量上限（字符）。
    pub evidence_total_chars: usize,
    /// 图片模式检索词条数上限。
    pub max_queries: usize,
    /// 已渲染的最终摘要提示词（`text_search.md` / `image_search.md`）。
    pub summary_prompt: String,
}

/// 研究流水线结算结果。
#[derive(Debug, Clone)]
pub struct ResearchOutcome {
    /// 流式输出的完整正文。
    pub text: String,
    /// 真实来源列表。
    pub sources: Vec<SourceRef>,
    /// Token 用量 + 搜索/抓取计费。
    pub usage: TokenUsage,
    /// 实际引擎（供应商 id）。
    pub engine: String,
    /// 端到端耗时（毫秒）。
    pub latency_ms: u64,
    /// 实际 Search 调用次数（如实回传）。
    pub search_calls: u32,
    /// 实际 Fetch 调用次数（如实回传）。
    pub fetch_calls: u32,
    /// 阶段检查点耗时（毫秒，自流水线开始累计）：用于证据与时延诊断。
    pub stage_latency_ms: Vec<(String, u64)>,
    /// 视觉识别是否退化为本地 OCR 文本（未上传图片）。
    pub from_local_ocr: bool,
}

/// 云端供应商后端（已解析出 Key 与端点）。
#[derive(Debug, Clone)]
struct CloudBackend {
    /// 供应商 id（回传 engine）。
    id: String,
    /// Base URL（不含 `/v1`）。
    base_url: String,
    /// 模型名。
    model: String,
    /// 明文 Key（只用于请求头，不写日志）。
    api_key: String,
}

impl CloudBackend {
    /// OpenAI 兼容流式网关。
    fn gateway(&self) -> HttpLlmGateway {
        HttpLlmGateway::new(
            format!("{}/v1", self.base_url.trim_end_matches('/')),
            Some(self.api_key.clone()),
            self.model.clone(),
        )
    }

    /// DeepSeek Search / Fetch 搜索器。
    fn searcher(&self) -> DeepSeekToolsSearcher {
        DeepSeekToolsSearcher::new(self.base_url.clone(), Some(self.api_key.clone()))
    }
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
                let _ = record_history_turn(&state, &request, &task_id, &text, &outcome);
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

/// 连续提问历史持久化（契约 v5 / T4 `HistoryRepo::record_turn`）。
///
/// 成功路径调用一次（user + assistant 两条）；`history.enabled=false` 时仓储内部 no-op。
fn record_history_turn(
    state: &AppState,
    request: &TaskRequest,
    task_id: &TaskId,
    answer: &str,
    outcome: &TaskOutcome,
) -> usize {
    state.history.record_turn(
        &request.session,
        task_id,
        request.action,
        request.mode,
        &request.turn_question(),
        answer,
        Some(outcome.engine.as_str()),
        "done",
        None,
    )
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
    // 浏览器搜索分支：不走 LLM，直接拼 URL 并真实打开系统浏览器。
    if request.routed.kind == TaskKind::Search && request.routed.engine == "browser" {
        return run_browser_search(app, task_id, request, cancel).await;
    }

    // 真实云端链路（骨架模式保持契约 §12 占位行为）。
    let real_mode = !app.state::<AppState>().is_skeleton();
    if real_mode {
        match request.action {
            ActionId::ImageSearch => {
                return run_image_search_pipeline(app, task_id, request, cancel).await
            }
            ActionId::Search => {
                return run_text_search_pipeline(app, task_id, request, cancel).await
            }
            ActionId::Translate => return run_translate_task(app, task_id, request, cancel).await,
            _ => {}
        }
    }

    // 网关选择（契约 v4）：骨架模式 / 本地引擎（Vulkan 优先）/ 云端真实 SSE。
    run_llm_task(app, task_id, request, cancel).await
}

/// 通用 LLM 流式任务（解答 / 追问 / 骨架模式所有动作）。
async fn run_llm_task(
    app: &AppHandle,
    task_id: &TaskId,
    request: &TaskRequest,
    cancel: &Arc<AtomicBool>,
) -> AppResult<ExecutedTask> {
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
        image_data_url: request.image_data_url.clone(),
        cancel: cancel.clone(),
    };
    let (collected, sink) = stream_sink(app, task_id);
    let outcome = gateway.stream(&llm_request, &sink).await?;
    let text = collected.lock().clone();
    Ok(ExecutedTask { outcome, text })
}

/// 构造「捕获正文 + 转发前端」的流式 sink。
fn stream_sink(
    app: &AppHandle,
    task_id: &TaskId,
) -> (
    Arc<Mutex<String>>,
    impl Fn(StreamChunk) + Send + Sync + 'static,
) {
    let collected = Arc::new(Mutex::new(String::new()));
    let buffer = collected.clone();
    let sink_app = app.clone();
    let sink_task = task_id.clone();
    let sink = move |chunk: StreamChunk| {
        if chunk.kind == StreamKind::Text {
            buffer.lock().push_str(&chunk.chunk);
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
    (collected, sink)
}

// ---------------------------------------------------------------------------
// 浏览器搜索
// ---------------------------------------------------------------------------

async fn run_browser_search(
    app: &AppHandle,
    task_id: &TaskId,
    request: &TaskRequest,
    cancel: &Arc<AtomicBool>,
) -> AppResult<ExecutedTask> {
    let started = Instant::now();
    let state = app.state::<AppState>();
    let skeleton = state.is_skeleton();
    let searcher = SkeletonSearcher;
    let search_request = search_gateway::SearchRequest {
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
    let text = if skeleton {
        // 契约 §12：骨架模式只记日志，不真正打开浏览器。
        tracing::info!(
            host = %url::Url::parse(&url).ok().and_then(|parsed| parsed.host_str().map(str::to_string)).unwrap_or_default(),
            "骨架模式：不打开浏览器"
        );
        outcome.text.clone()
    } else {
        open_external(Some(app), &url).await?;
        format!(
            "已在系统默认浏览器打开搜索页（引擎：{}）：\n{url}",
            request.routed.engine
        )
    };

    if cancel.load(Ordering::SeqCst) {
        return Err(AppError::invalid_state("任务已取消"));
    }

    emit_task_delta(
        app,
        &TaskDeltaPayload {
            task: task_id.clone(),
            kind: StreamKind::Text,
            chunk: text.clone(),
        },
    );

    Ok(ExecutedTask {
        outcome: TaskOutcome {
            usage: zero_usage(),
            latency_ms: started.elapsed().as_millis() as u64,
            engine: request.routed.engine.clone(),
            sources: outcome.sources,
        },
        text,
    })
}

// ---------------------------------------------------------------------------
// 文本 AI 搜索（真实检索 + 摘要）
// ---------------------------------------------------------------------------

async fn run_text_search_pipeline(
    app: &AppHandle,
    task_id: &TaskId,
    request: &TaskRequest,
    cancel: &Arc<AtomicBool>,
) -> AppResult<ExecutedTask> {
    let state = app.state::<AppState>();
    let config = state.snapshot_config();
    guard_privacy_for_search(&config)?;
    let backend = resolve_cloud_backend(&state, &request.routed.engine).await?;
    let gateway = backend.gateway();
    let searcher = backend.searcher();
    let options = research_options(
        &state,
        &config,
        ActionId::Search,
        request.mode,
        &request.input_text,
    );
    let (collected, sink) = stream_sink(app, task_id);
    let outcome = run_text_research(
        &gateway,
        &searcher,
        &backend.id,
        &request.input_text,
        &options,
        cancel,
        &sink,
    )
    .await?;
    finish_research(&collected, outcome)
}

/// 文本 AI 搜索流水线（可独立测试）：单次检索 → deep 并发 Fetch → 流式摘要。
pub async fn run_text_research(
    gateway: &dyn LlmGateway,
    searcher: &DeepSeekToolsSearcher,
    engine: &str,
    input_text: &str,
    options: &ResearchOptions,
    cancel: &Arc<AtomicBool>,
    sink: &(dyn Fn(StreamChunk) + Send + Sync),
) -> AppResult<ResearchOutcome> {
    let started = Instant::now();
    let mut stage_latency_ms: Vec<(String, u64)> = Vec::new();
    let query = search_gateway::normalize_query(input_text);
    if query.is_empty() {
        return Err(AppError::invalid_state("选区文本为空，无法检索"));
    }
    check_cancel(cancel)?;

    // 阶段 1：真实 Search。
    let client = searcher.client()?;
    let hits = searcher
        .search(
            &client,
            &query,
            options.search_count,
            false,
            Some(input_text),
        )
        .await?;
    let search_calls = 1u32;
    let mut usages: Vec<TokenUsage> = Vec::new();
    stage_latency_ms.push(("search".to_string(), started.elapsed().as_millis() as u64));

    // 阶段 2：deep 模式并发 Fetch。
    let (pages, fetch_calls) =
        fetch_pages_for_hits(searcher, &client, &hits, options, cancel).await?;
    check_cancel(cancel)?;
    stage_latency_ms.push(("fetch".to_string(), started.elapsed().as_millis() as u64));

    // 阶段 3：摘要（流式）。
    let evidence = build_evidence(&hits, &pages, options);
    let sources = collect_sources(&hits);
    let prompt = compose_summary_prompt(&options.summary_prompt, &evidence, false);
    let (text, usage) = stream_llm_with_image(
        gateway,
        engine,
        ActionId::Search,
        options.mode,
        prompt,
        input_text.to_string(),
        None,
        cancel,
        sink,
    )
    .await?;
    usages.push(usage);
    stage_latency_ms.push(("summary".to_string(), started.elapsed().as_millis() as u64));

    Ok(ResearchOutcome {
        text,
        sources,
        usage: combine_usage(&usages, search_calls, fetch_calls),
        engine: engine.to_string(),
        latency_ms: started.elapsed().as_millis() as u64,
        search_calls,
        fetch_calls,
        stage_latency_ms,
        from_local_ocr: false,
    })
}

// ---------------------------------------------------------------------------
// 图片搜索（视觉识别 → 检索词 → Search → 并发 Fetch → 摘要流式）
// ---------------------------------------------------------------------------

async fn run_image_search_pipeline(
    app: &AppHandle,
    task_id: &TaskId,
    request: &TaskRequest,
    cancel: &Arc<AtomicBool>,
) -> AppResult<ExecutedTask> {
    let state = app.state::<AppState>();
    let config = state.snapshot_config();
    guard_privacy_for_search(&config)?;
    // 图片上传在命令层按 privacy.allowImageUpload 决定是否附带；这里只做防御性校验。
    let image_data_url = request
        .image_data_url
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .filter(|_| config.privacy.allow_image_upload && !config.privacy.local_only)
        .map(str::to_string);
    let backend = resolve_cloud_backend(&state, &request.routed.engine).await?;
    let gateway = backend.gateway();
    let searcher = backend.searcher();
    let options = research_options(
        &state,
        &config,
        ActionId::ImageSearch,
        request.mode,
        &request.input_text,
    );
    let (collected, sink) = stream_sink(app, task_id);
    let outcome = run_image_research(
        &gateway,
        &searcher,
        &backend.id,
        image_data_url.as_deref(),
        &request.input_text,
        &options,
        cancel,
        &sink,
    )
    .await?;
    finish_research(&collected, outcome)
}

/// 图片搜索四阶段流水线（可独立测试）。
///
/// `image_data_url = None` 表示未上传图片（隐私限制或骨架/无帧）：
/// 阶段 1 退化为「本地 OCR 文本」，其余阶段照常，并在摘要提示词里如实标注。
pub async fn run_image_research(
    gateway: &dyn LlmGateway,
    searcher: &DeepSeekToolsSearcher,
    engine: &str,
    image_data_url: Option<&str>,
    local_text: &str,
    options: &ResearchOptions,
    cancel: &Arc<AtomicBool>,
    sink: &(dyn Fn(StreamChunk) + Send + Sync),
) -> AppResult<ResearchOutcome> {
    let started = Instant::now();
    let mut stage_latency_ms: Vec<(String, u64)> = Vec::new();
    check_cancel(cancel)?;
    let mut usages: Vec<TokenUsage> = Vec::new();

    // 阶段 1：视觉识别（多模态）或本地 OCR 兜底。
    let (description, from_local_ocr) = match image_data_url {
        Some(data_url) => {
            let (raw, usage) = vision_describe(gateway, engine, data_url, options, cancel).await?;
            usages.push(usage);
            (raw, false)
        }
        None => {
            let text = local_text.trim();
            if text.is_empty() {
                return Err(AppError::invalid_state(
                    "图片搜索缺少可用输入：未允许上传图片，且本地 OCR 没有识别到文字",
                )
                .with_message("图片搜索需要上传图片或可识别的文字：请在设置中允许图像上传，或重新框选清晰区域")
                .with_fallback(vec!["open_settings", "image_path"]));
            }
            (text.to_string(), true)
        }
    };
    let description = search_gateway::truncate(&description, VISION_DESCRIPTION_MAX_CHARS);
    tracing::info!(
        engine = %engine,
        from_local_ocr,
        description_chars = description.chars().count(),
        "图片搜索阶段 1 完成：视觉识别"
    );
    stage_latency_ms.push(("vision".to_string(), started.elapsed().as_millis() as u64));
    check_cancel(cancel)?;

    // 阶段 2：检索词生成。
    let (queries, usage) = generate_queries(gateway, engine, &description, options, cancel).await?;
    usages.push(usage);
    tracing::info!(
        engine = %engine,
        query_count = queries.len(),
        "图片搜索阶段 2 完成：检索词"
    );
    stage_latency_ms.push(("queries".to_string(), started.elapsed().as_millis() as u64));
    check_cancel(cancel)?;

    // 阶段 3：真实 Search（多组检索词串行，遵守 5 QPS）。
    let client = searcher.client()?;
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut search_calls = 0u32;
    let mut first_error: Option<AppError> = None;
    for query in &queries {
        if cancel.load(Ordering::SeqCst) {
            return Err(AppError::invalid_state("任务已取消"));
        }
        match searcher
            .search(&client, query, options.search_count, false, None)
            .await
        {
            Ok(mut found) => {
                search_calls += 1;
                hits.append(&mut found);
            }
            Err(err) => {
                tracing::warn!(code = %err.code, "图片搜索的某组检索词失败，继续其它检索词");
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        // 5 QPS：多检索词之间留出间隔（quick 只有 1 组，不额外等待）。
        if queries.len() > 1 {
            tokio::time::sleep(std::time::Duration::from_millis(220)).await;
        }
    }
    if search_calls == 0 {
        if let Some(err) = first_error {
            return Err(err);
        }
    }
    dedupe_hits(&mut hits);
    tracing::info!(
        engine = %engine,
        hits = hits.len(),
        search_calls,
        "图片搜索阶段 3 完成：Search"
    );
    stage_latency_ms.push(("search".to_string(), started.elapsed().as_millis() as u64));
    check_cancel(cancel)?;

    // 阶段 4：deep 模式并发 Fetch。
    let (pages, fetch_calls) =
        fetch_pages_for_hits(searcher, &client, &hits, options, cancel).await?;
    tracing::info!(
        engine = %engine,
        fetched = pages.len(),
        fetch_calls,
        "图片搜索阶段 4 完成：并发 Fetch"
    );
    stage_latency_ms.push(("fetch".to_string(), started.elapsed().as_millis() as u64));
    check_cancel(cancel)?;

    // 阶段 5：摘要流式输出。
    let evidence = build_evidence(&hits, &pages, options);
    let sources = collect_sources(&hits);
    let prompt = compose_image_summary_prompt(
        &options.summary_prompt,
        &description,
        &queries,
        &evidence,
        from_local_ocr,
    );
    let (text, usage) = stream_llm_with_image(
        gateway,
        engine,
        ActionId::ImageSearch,
        // 图片摘要固定 Quick（thinking disabled）：四段结构化输出不需要推理 token，
        // 否则 deep 模式会把 max_tokens 花在 reasoning 上导致「来源」段被截断
        // （2026-11-06 真实 E2E 发现）。deep 的「深度」来自多检索词 + 并发 Fetch。
        Mode::Quick,
        prompt,
        description,
        None,
        cancel,
        sink,
    )
    .await?;
    usages.push(usage);
    stage_latency_ms.push(("summary".to_string(), started.elapsed().as_millis() as u64));

    Ok(ResearchOutcome {
        text,
        sources,
        usage: combine_usage(&usages, search_calls, fetch_calls),
        engine: engine.to_string(),
        latency_ms: started.elapsed().as_millis() as u64,
        search_calls,
        fetch_calls,
        stage_latency_ms,
        from_local_ocr,
    })
}

// ---------------------------------------------------------------------------
// 翻译（降级链；LLM 实时流式，Qwen-MT 非流式一次输出）
// ---------------------------------------------------------------------------

async fn run_translate_task(
    app: &AppHandle,
    task_id: &TaskId,
    request: &TaskRequest,
    cancel: &Arc<AtomicBool>,
) -> AppResult<ExecutedTask> {
    let started = Instant::now();
    let state = app.state::<AppState>();
    let config = state.snapshot_config();
    let target_lang = request
        .options
        .target_lang
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or(config.translate.target_lang.as_str())
        .to_string();
    let translate_request = TranslateRequest::new(&request.input_text, &target_lang, request.mode)
        .with_glossary(config.translate.glossary.clone());
    let attempts = translator::translate_attempts(&state, app, &config, &translate_request).await?;

    let (collected, sink) = stream_sink(app, task_id);
    let mut failures: Vec<String> = Vec::new();
    let mut last_error: Option<AppError> = None;
    for attempt in &attempts {
        check_cancel(cancel)?;
        match &attempt.kind {
            TranslateAttemptKind::Llm { gateway, prompt } => {
                let llm_request = LlmRequest {
                    task: task_id.clone(),
                    session: request.session.clone(),
                    action: ActionId::Translate,
                    mode: request.mode,
                    engine: attempt.engine.clone(),
                    prompt: prompt.clone(),
                    input_text: translate_request.text.clone(),
                    image_data_url: None,
                    cancel: cancel.clone(),
                };
                if attempt.engine == "local" {
                    // 本地后端：先缓冲流式输出，校验通过后才推送前端。
                    // 否则小模型回显原文时，用户会先看到原文再收到「失败/降级」提示（重复/串台）。
                    let local_buffer = Arc::new(Mutex::new(String::new()));
                    let local_collected = local_buffer.clone();
                    let local_sink = move |chunk: StreamChunk| {
                        if chunk.kind == StreamKind::Text {
                            local_collected.lock().push_str(&chunk.chunk);
                        }
                    };
                    match gateway.stream(&llm_request, &local_sink).await {
                        Ok(outcome) => {
                            let text = local_buffer.lock().trim().to_string();
                            if text.is_empty() {
                                failures.push(format!("{}:EMPTY", attempt.engine));
                                last_error = Some(AppError::invalid_state("翻译后端未返回内容"));
                                continue;
                            }
                            if translator::looks_untranslated(
                                &translate_request.text,
                                &text,
                                &target_lang,
                            ) {
                                let err = translator::untranslated_output_error(&target_lang);
                                tracing::warn!(
                                    engine = %attempt.engine,
                                    code = %err.code,
                                    "本地翻译返回原文（未翻译）：不推送前端，继续云端降级"
                                );
                                failures.push(format!("{}:{}", attempt.engine, err.code));
                                last_error = Some(err);
                                continue;
                            }
                            emit_task_delta(
                                app,
                                &TaskDeltaPayload {
                                    task: task_id.clone(),
                                    kind: StreamKind::Text,
                                    chunk: text.clone(),
                                },
                            );
                            return Ok(ExecutedTask {
                                outcome: TaskOutcome {
                                    usage: outcome.usage,
                                    latency_ms: started.elapsed().as_millis() as u64,
                                    engine: attempt.engine.clone(),
                                    sources: Vec::new(),
                                },
                                text,
                            });
                        }
                        Err(err) => {
                            tracing::warn!(
                                engine = %attempt.engine,
                                code = %err.code,
                                "本地翻译后端失败（尚未推送任何正文），尝试下一个"
                            );
                            failures.push(format!("{}:{}", attempt.engine, err.code));
                            last_error = Some(err);
                            continue;
                        }
                    }
                }
                let before = collected.lock().chars().count();
                match gateway.stream(&llm_request, &sink).await {
                    Ok(outcome) => {
                        let text = collected.lock().clone();
                        if text.trim().is_empty() {
                            failures.push(format!("{}:EMPTY", attempt.engine));
                            last_error = Some(AppError::invalid_state("翻译后端未返回内容"));
                            continue;
                        }
                        return Ok(ExecutedTask {
                            outcome: TaskOutcome {
                                usage: outcome.usage,
                                latency_ms: started.elapsed().as_millis() as u64,
                                engine: attempt.engine.clone(),
                                sources: Vec::new(),
                            },
                            text,
                        });
                    }
                    Err(err) => {
                        // 已经向用户输出过部分正文时不切换后端（避免重复/串台），直接报错。
                        let emitted = collected.lock().chars().count() > before;
                        tracing::warn!(engine = %attempt.engine, code = %err.code, emitted, "LLM 翻译后端失败");
                        if emitted && cancel.load(Ordering::SeqCst) {
                            return Err(err);
                        }
                        failures.push(format!("{}:{}", attempt.engine, err.code));
                        last_error = Some(err);
                        if emitted {
                            return Err(AppError::invalid_state(
                                "翻译中断：已输出部分译文后上游失败，请重试",
                            )
                            .with_message("翻译过程中网络中断：已输出的部分不完整，请点击重试"));
                        }
                    }
                }
            }
            TranslateAttemptKind::QwenMt(translator) => {
                match translator.translate_with_usage(&translate_request).await {
                    Ok((text, usage)) => {
                        emit_task_delta(
                            app,
                            &TaskDeltaPayload {
                                task: task_id.clone(),
                                kind: StreamKind::Text,
                                chunk: text.clone(),
                            },
                        );
                        collected.lock().push_str(&text);
                        return Ok(ExecutedTask {
                            outcome: TaskOutcome {
                                usage,
                                latency_ms: started.elapsed().as_millis() as u64,
                                engine: attempt.engine.clone(),
                                sources: Vec::new(),
                            },
                            text,
                        });
                    }
                    Err(err) => {
                        tracing::warn!(engine = %attempt.engine, code = %err.code, "Qwen-MT 翻译失败，尝试降级");
                        failures.push(format!("{}:{}", attempt.engine, err.code));
                        last_error = Some(err);
                    }
                }
            }
        }
    }

    let mut error =
        last_error.unwrap_or_else(|| AppError::invalid_config("translate", "没有可用的翻译后端"));
    if !failures.is_empty() {
        error.message = format!("全部翻译后端均失败（已尝试：{}）", failures.join(", "));
    }
    Err(error)
}

// ---------------------------------------------------------------------------
// 流水线公共步骤
// ---------------------------------------------------------------------------

/// 视觉识别（阶段 1）：多模态请求 → 客观描述（含图中文字）。
///
/// 中间阶段固定 `Mode::Quick`：forced 关闭 thinking，避免推理 token 占满 `max_tokens`
/// 导致 `content` 为空（2026-11-06 实测 deepseek-flash deep 模式首轮 18s 且正文被
/// reasoning 挤空）。
async fn vision_describe(
    gateway: &dyn LlmGateway,
    engine: &str,
    image_data_url: &str,
    options: &ResearchOptions,
    cancel: &Arc<AtomicBool>,
) -> AppResult<(String, TokenUsage)> {
    let prompt = vision_prompt(&options.lang);
    let (raw, usage) = run_llm_with_image(
        gateway,
        engine,
        ActionId::ImageSearch,
        Mode::Quick,
        prompt,
        String::new(),
        Some(image_data_url.to_string()),
        cancel,
    )
    .await?;
    let description = parse_vision_output(&raw);
    if description.trim().is_empty() {
        return Err(AppError::invalid_state(
            "视觉识别未返回可用描述（模型可能把 token 用在了思考上）",
        )
        .with_message("图片识别没有返回内容：请重试，或改选更小/更清晰的区域")
        .with_fallback(vec!["retry", "image_path"]));
    }
    Ok((description, usage))
}

/// 视觉识别提示词（阶段 1；不属于用户可覆盖提示词，保持最小可用）。
pub fn vision_prompt(lang: &str) -> String {
    format!(
        "【任务】你是看图助手。请客观描述这张屏幕截图区域中的内容，供后续网页检索使用。\n\
         【要求】\n\
         1. 先原样摘录图中可辨认的文字（最多 500 字；没有文字就写「无」）；\n\
         2. 再描述物体 / 界面 / 图表（1–3 句），使用语言：{lang}；\n\
         3. 不要编造看不见的细节；看不清就写「不清晰」。\n\
         【输出】只输出一个 JSON 对象，不要代码块、不要解释：\n\
         {{\"text\":\"图中文字\",\"description\":\"内容描述\",\"uncertain\":\"不确定部分或空串\"}}"
    )
}

/// 解析视觉识别输出：优先取 JSON 的 `description` + `text`；解析失败时原样使用。
pub fn parse_vision_output(raw: &str) -> String {
    let trimmed = raw.trim();
    let json_slice = match (trimmed.find('{'), trimmed.rfind('}')) {
        (Some(start), Some(end)) if end > start => &trimmed[start..=end],
        _ => "",
    };
    if !json_slice.is_empty() {
        if let Ok(value) = serde_json::from_str::<Value>(json_slice) {
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let description = value
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let uncertain = value
                .get("uncertain")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let mut combined = String::new();
            if !text.is_empty() && text != "无" {
                combined.push_str(&format!("图中文字：{text}\n"));
            }
            if !description.is_empty() {
                combined.push_str(&format!("内容描述：{description}\n"));
            }
            if !uncertain.is_empty() {
                combined.push_str(&format!("不确定部分：{uncertain}\n"));
            }
            if !combined.trim().is_empty() {
                return combined.trim().to_string();
            }
        }
    }
    search_gateway::truncate(trimmed, VISION_DESCRIPTION_MAX_CHARS)
}

/// 检索词生成（阶段 2）：LLM 输出多行关键词；失败时回退描述本身。
async fn generate_queries(
    gateway: &dyn LlmGateway,
    engine: &str,
    description: &str,
    options: &ResearchOptions,
    cancel: &Arc<AtomicBool>,
) -> AppResult<(Vec<String>, TokenUsage)> {
    let count =
        options
            .max_queries
            .clamp(1, MAX_IMAGE_QUERIES)
            .min(if options.mode == Mode::Quick {
                1
            } else {
                MAX_IMAGE_QUERIES
            });
    let prompt = query_prompt(description, count, &options.lang);
    match run_llm_with_image(
        gateway,
        engine,
        ActionId::ImageSearch,
        Mode::Quick,
        prompt,
        description.to_string(),
        None,
        cancel,
    )
    .await
    {
        Ok((raw, usage)) => {
            let queries = parse_queries(&raw, count);
            if queries.is_empty() {
                tracing::warn!("检索词生成返回空结果，回退为识别描述前 60 字");
                Ok((vec![fallback_query(description)], usage))
            } else {
                Ok((queries, usage))
            }
        }
        Err(err) => {
            tracing::warn!(code = %err.code, "检索词生成失败，回退为识别描述前 60 字");
            Ok((
                vec![fallback_query(description)],
                TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_input_tokens: 0,
                    cost_estimate: 0.0,
                    currency: Currency::Cny,
                },
            ))
        }
    }
}

/// 检索词生成提示词（阶段 2）。
pub fn query_prompt(description: &str, count: usize, lang: &str) -> String {
    format!(
        "【任务】根据下面的图像识别结果，生成用于网页搜索的关键词。\n\
         【要求】\n\
         1. 只输出关键词，每行一组，不要编号、引号或解释；共 {count} 行；\n\
         2. 关键词应是可搜索的实体 / 主题，不要复述整段描述；\n\
         3. 不要包含隐私信息（人名、账号、手机号等）；使用语言：{lang}。\n\
         【识别结果】\n{description}"
    )
}

/// 解析检索词输出：去编号/项目符号/引号，去重，截断到协议上限。
pub fn parse_queries(raw: &str, max_count: usize) -> Vec<String> {
    let mut queries: Vec<String> = Vec::new();
    for line in raw.lines() {
        let mut line = line.trim();
        if line.is_empty() {
            continue;
        }
        // 去掉常见列表前缀：`- `、`1. `、`1、`、`* `。
        for prefix in ["- ", "* ", "• "] {
            if let Some(stripped) = line.strip_prefix(prefix) {
                line = stripped.trim();
            }
        }
        let digits: String = line.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !digits.is_empty() {
            let rest = line[digits.len()..].trim_start();
            if rest.starts_with('.') || rest.starts_with('、') || rest.starts_with(')') {
                line = rest[1..].trim();
            }
        }
        let line = line.trim_matches(|ch: char| {
            ch == '"' || ch == '\'' || ch == '「' || ch == '」' || ch == '“' || ch == '”'
        });
        let line = search_gateway::truncate(line.trim(), MAX_QUERY_CHARS);
        if line.is_empty() {
            continue;
        }
        if queries.iter().any(|item| item == &line) {
            continue;
        }
        queries.push(line);
        if queries.len() >= max_count {
            break;
        }
    }
    queries
}

fn fallback_query(description: &str) -> String {
    let first_line = description
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(description);
    let stripped = first_line
        .trim_start_matches("图中文字：")
        .trim_start_matches("内容描述：")
        .trim();
    let candidate = if stripped.is_empty() {
        first_line
    } else {
        stripped
    };
    search_gateway::truncate(candidate, 60)
}

/// deep 模式并发 Fetch（quick 不 Fetch）。
async fn fetch_pages_for_hits(
    searcher: &DeepSeekToolsSearcher,
    client: &reqwest::Client,
    hits: &[SearchHit],
    options: &ResearchOptions,
    cancel: &Arc<AtomicBool>,
) -> AppResult<(Vec<FetchedPage>, u32)> {
    if options.mode != Mode::Deep || options.fetch_pages == 0 || hits.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let urls: Vec<String> = hits
        .iter()
        .map(|hit| hit.url.clone())
        .take(options.fetch_pages.min(MAX_FETCH_PAGES))
        .collect();
    let pages = fetch_concurrently(searcher, client, &urls, options.fetch_max_chars).await;
    let fetch_calls = pages.len() as u32;
    if cancel.load(Ordering::SeqCst) {
        return Err(AppError::invalid_state("任务已取消"));
    }
    Ok((pages, fetch_calls))
}

/// 拼接给摘要模型的「已检索资料」证据块。
pub fn build_evidence(
    hits: &[SearchHit],
    pages: &[FetchedPage],
    options: &ResearchOptions,
) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut evidence = String::new();
    for (index, hit) in hits.iter().enumerate() {
        if evidence.chars().count() >= options.evidence_total_chars {
            break;
        }
        let page = pages.iter().find(|page| page.url == hit.url);
        let fetched_body;
        let body: &str = match page {
            Some(page) if !page.content.trim().is_empty() => page.content.as_str(),
            _ => {
                fetched_body = hit.evidence_text(EVIDENCE_PER_SOURCE_CHARS);
                fetched_body.as_str()
            }
        };
        let body = search_gateway::truncate(body, EVIDENCE_PER_SOURCE_CHARS);
        let date = hit.date.as_deref().unwrap_or("未知日期");
        let site = hit.site_name.as_deref().unwrap_or("未知站点");
        evidence.push_str(&format!(
            "{}. 标题：{}\n   站点：{}｜URL：{}｜日期：{}\n   内容：{}\n\n",
            index + 1,
            if hit.title.trim().is_empty() {
                hit.url.as_str()
            } else {
                hit.title.trim()
            },
            site,
            hit.url,
            date,
            body
        ));
    }
    search_gateway::truncate(&evidence, options.evidence_total_chars)
}

/// 把渲染好的摘要提示词与证据块组合。
pub fn compose_summary_prompt(
    summary_prompt: &str,
    evidence: &str,
    from_local_ocr: bool,
) -> String {
    let mut prompt = summary_prompt.trim_end().to_string();
    if from_local_ocr {
        prompt.push_str(
            "\n\n【重要说明】本次**未上传图片**（用户隐私设置或未取到冻结帧），\
             识别结果来自本地 OCR 文本，可能不完整；请在「识别结果」中如实说明这一点。",
        );
    }
    prompt.push_str("\n\n【已检索到的资料（只能基于这些资料与常识作答；「来源」只引用其中真实 URL，不得编造）】\n");
    if evidence.trim().is_empty() {
        prompt.push_str(
            "（未检索到任何结果：请在「来源」中写「未检索到可靠来源」，不要编造链接。）\n",
        );
    } else {
        prompt.push_str(evidence.trim_end());
        prompt.push('\n');
    }
    prompt.push_str("\n【再次强调】严格按模板输出「识别结果 / 检索词 / 结论 / 来源」四段（文本搜索按模板输出结论/要点/来源），不要输出其它内容。\n");
    prompt
}

/// 图片搜索摘要提示词：把**阶段 1 的识别结果**与**阶段 2 的检索词**显式喂给摘要模型。
///
/// 否则摘要模型看不到图片（图片只在阶段 1 使用），会误报「未收到图片」
/// （2026-11-06 真实 E2E 发现并修复）。
pub fn compose_image_summary_prompt(
    summary_prompt: &str,
    recognition: &str,
    queries: &[String],
    evidence: &str,
    from_local_ocr: bool,
) -> String {
    let mut prompt = summary_prompt.trim_end().to_string();
    let source = if from_local_ocr {
        "本地 OCR 文本（本次未上传图片）"
    } else {
        "真实截图（阶段 1 多模态识别）"
    };
    prompt.push_str(&format!(
        "\n\n【图像识别结果（阶段 1，来自{source}；请直接以此填充「识别结果」段，不要声称没有收到图片）】\n{}\n",
        recognition.trim()
    ));
    if !queries.is_empty() {
        prompt.push_str("\n【检索词（阶段 2，已用于真实网页检索）】\n");
        for (index, query) in queries.iter().enumerate() {
            prompt.push_str(&format!("{}. {}\n", index + 1, query));
        }
    }
    if from_local_ocr {
        prompt.push_str(
            "\n【重要说明】本次**未上传图片**（用户隐私设置或未取到冻结帧），\
             识别结果来自本地 OCR 文本，可能不完整；请在「识别结果」中如实说明这一点。\n",
        );
    }
    prompt.push_str("\n【已检索到的资料（只能基于这些资料与常识作答；「来源」只引用其中真实 URL，不得编造）】\n");
    if evidence.trim().is_empty() {
        prompt.push_str(
            "（未检索到任何结果：请在「来源」中写「未检索到可靠来源」，不要编造链接。）\n",
        );
    } else {
        prompt.push_str(evidence.trim_end());
        prompt.push('\n');
    }
    prompt.push_str(
        "\n【再次强调】严格按模板输出「识别结果 / 检索词 / 结论 / 来源」四段，不要输出其它内容。\n",
    );
    prompt.push_str("【篇幅】识别结果段 ≤ 200 字（概括，不要复述全部识别细节）；结论 ≤ 300 字；来源列 3–5 条真实 URL。\n");
    prompt
}
fn collect_sources(hits: &[SearchHit]) -> Vec<SourceRef> {
    let mut sources: Vec<SourceRef> = Vec::new();
    for hit in hits {
        if sources.iter().any(|source| source.url == hit.url) {
            continue;
        }
        sources.push(hit.to_source_ref());
        if sources.len() >= MAX_SOURCES {
            break;
        }
    }
    sources
}

fn dedupe_hits(hits: &mut Vec<SearchHit>) {
    let mut seen: Vec<String> = Vec::new();
    hits.retain(|hit| {
        if seen.iter().any(|url| url == &hit.url) {
            return false;
        }
        seen.push(hit.url.clone());
        true
    });
}

/// 组合多次 LLM 调用 + Search/Fetch 的用量与费用。
fn combine_usage(usages: &[TokenUsage], search_calls: u32, fetch_calls: u32) -> TokenUsage {
    let mut combined = TokenUsage {
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cost_estimate: 0.0,
        currency: Currency::Cny,
    };
    for usage in usages {
        combined.input_tokens += usage.input_tokens;
        combined.output_tokens += usage.output_tokens;
        combined.cached_input_tokens += usage.cached_input_tokens;
        combined.cost_estimate += usage.cost_estimate;
    }
    combined.cost_estimate += search_gateway::search_fetch_cost(search_calls, fetch_calls);
    combined.cost_estimate = (combined.cost_estimate * 1_000_000.0).round() / 1_000_000.0;
    combined
}

fn zero_usage() -> TokenUsage {
    TokenUsage {
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cost_estimate: 0.0,
        currency: Currency::Cny,
    }
}

fn check_cancel(cancel: &Arc<AtomicBool>) -> AppResult<()> {
    if cancel.load(Ordering::SeqCst) {
        return Err(AppError::invalid_state("任务已取消"));
    }
    Ok(())
}

/// 非流式收集一次 LLM 调用（阶段 1/2 与最终摘要共用；摘要由调用方传入真 sink）。
#[allow(clippy::too_many_arguments)]
async fn run_llm_with_image(
    gateway: &dyn LlmGateway,
    engine: &str,
    action: ActionId,
    mode: Mode,
    prompt: String,
    input_text: String,
    image_data_url: Option<String>,
    cancel: &Arc<AtomicBool>,
) -> AppResult<(String, TokenUsage)> {
    let request = LlmRequest {
        task: TaskId::from("research"),
        session: SessionId::from("research"),
        action,
        mode,
        engine: engine.to_string(),
        prompt,
        input_text,
        image_data_url,
        cancel: cancel.clone(),
    };
    let collected = Arc::new(Mutex::new(String::new()));
    let buffer = collected.clone();
    let sink = move |chunk: StreamChunk| {
        if chunk.kind == StreamKind::Text {
            buffer.lock().push_str(&chunk.chunk);
        }
    };
    let outcome = gateway.stream(&request, &sink).await?;
    let text = collected.lock().clone();
    Ok((text, outcome.usage))
}

/// 流式摘要：边把增量推给前端 `sink`，边本地累积完整正文。
#[allow(clippy::too_many_arguments)]
async fn stream_llm_with_image(
    gateway: &dyn LlmGateway,
    engine: &str,
    action: ActionId,
    mode: Mode,
    prompt: String,
    input_text: String,
    image_data_url: Option<String>,
    cancel: &Arc<AtomicBool>,
    sink: &(dyn Fn(StreamChunk) + Send + Sync),
) -> AppResult<(String, TokenUsage)> {
    let collected = Arc::new(Mutex::new(String::new()));
    let buffer = collected.clone();
    let forwarding = move |chunk: StreamChunk| {
        if chunk.kind == StreamKind::Text {
            buffer.lock().push_str(&chunk.chunk);
        }
        sink(chunk);
    };
    let request = LlmRequest {
        task: TaskId::from("research"),
        session: SessionId::from("research"),
        action,
        mode,
        engine: engine.to_string(),
        prompt,
        input_text,
        image_data_url,
        cancel: cancel.clone(),
    };
    let outcome = gateway.stream(&request, &forwarding).await?;
    let mut usage = outcome.usage;
    let mut text = collected.lock().clone();
    // 深度模式在 DeepSeek 类「默认思考」供应商上可能把 max_tokens 全用在 reasoning 上，
    // 导致正文为空（2026-11-06 实测）。此时回退一次「无思考」快速模式重试，避免整任务失败。
    if text.trim().is_empty() && mode == Mode::Deep {
        tracing::warn!("深度模式摘要为空（思考 token 耗尽），回退无思考模式重试一次");
        collected.lock().clear();
        let mut retry = request;
        retry.mode = Mode::Quick;
        let retry_outcome = gateway.stream(&retry, &forwarding).await?;
        usage = merge_usage(&usage, &retry_outcome.usage);
        text = collected.lock().clone();
    }
    Ok((text, usage))
}

/// 合并两次调用的用量/费用（重试场景）。
fn merge_usage(first: &TokenUsage, second: &TokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: first.input_tokens + second.input_tokens,
        output_tokens: first.output_tokens + second.output_tokens,
        cached_input_tokens: first.cached_input_tokens + second.cached_input_tokens,
        cost_estimate: ((first.cost_estimate + second.cost_estimate) * 1_000_000.0).round()
            / 1_000_000.0,
        currency: Currency::Cny,
    }
}

/// 研究结果 → `ExecutedTask`（正文以 sink 累积为准，避免双份拷贝不一致）。
fn finish_research(
    collected: &Arc<Mutex<String>>,
    outcome: ResearchOutcome,
) -> AppResult<ExecutedTask> {
    let text = if collected.lock().trim().is_empty() {
        outcome.text.clone()
    } else {
        collected.lock().clone()
    };
    if text.trim().is_empty() {
        return Err(AppError::invalid_state("摘要模型未返回内容"));
    }
    Ok(ExecutedTask {
        outcome: TaskOutcome {
            usage: outcome.usage,
            latency_ms: outcome.latency_ms,
            engine: outcome.engine,
            sources: outcome.sources,
        },
        text,
    })
}

/// 安全解析云端供应商（enabled + Key；拒绝 qwenMt 供应商用于视觉/搜索）。
async fn resolve_cloud_backend(state: &AppState, engine: &str) -> AppResult<CloudBackend> {
    let config = state.snapshot_config();
    let provider = config
        .providers
        .iter()
        .find(|provider| provider.id == engine)
        .ok_or_else(|| {
            AppError::invalid_config(
                "providers",
                format!("供应商 {engine} 不在配置中：请检查设置或重新选择供应商"),
            )
        })?;
    if !provider.enabled {
        return Err(AppError::invalid_config(
            "providers",
            format!(
                "供应商 {} 已停用：请在设置中启用并填写 API Key",
                provider.name
            ),
        )
        .with_message(format!(
            "供应商「{}」未启用：请先在设置中启用",
            provider.name
        ))
        .with_fallback(vec!["open_settings"]));
    }
    if provider.kind == ProviderKind::QwenMt {
        return Err(AppError::invalid_config(
            "providers",
            format!(
                "供应商 {} 是翻译专用（Qwen-MT），不支持视觉识别 / 联网搜索",
                provider.name
            ),
        )
        .with_message(format!(
            "「{}」是翻译专用供应商，请在设置中选择支持视觉 / 搜索的供应商",
            provider.name
        ))
        .with_fallback(vec!["open_settings"]));
    }
    let key = state
        .credential_service()
        .provider_key(&provider.id)
        .await?
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::AuthFailed,
                Some(format!("供应商 {} 未配置 API Key", provider.id)),
            )
            .with_message(format!(
                "尚未填写「{}」的 API Key：请在设置中填写后重试",
                provider.name
            ))
            .with_fallback(vec!["open_settings"])
        })?;
    Ok(CloudBackend {
        id: provider.id.clone(),
        base_url: provider.base_url.clone(),
        model: provider.model.clone(),
        api_key: key,
    })
}

/// 联网检索的隐私前置校验。
fn guard_privacy_for_search(config: &Config) -> AppResult<()> {
    if config.privacy.local_only {
        return Err(AppError::invalid_config(
            "privacy.localOnly",
            "隐私设置仅本地模式：已禁用联网检索",
        )
        .with_message("「仅本地模式」已开启：联网搜索不可用，请在隐私设置中关闭或改用浏览器搜索")
        .with_fallback(vec!["open_settings", "browser_search"]));
    }
    if !config.privacy.allow_text_upload {
        return Err(AppError::invalid_config(
            "privacy.allowTextUpload",
            "隐私设置不允许文本上传：联网检索需要上传检索词",
        )
        .with_message("「允许文本上传」已关闭：联网搜索不可用，请在隐私设置中允许后重试")
        .with_fallback(vec!["open_settings", "browser_search"]));
    }
    Ok(())
}

/// 生成研究流水线选项（文本 / 图片共用；含最终摘要提示词渲染）。
pub fn research_options(
    state: &AppState,
    config: &Config,
    action: ActionId,
    mode: Mode,
    input_text: &str,
) -> ResearchOptions {
    let lang = match config.app.language {
        crate::contract::config::Language::ZhCn => "zh-CN",
        crate::contract::config::Language::EnUs => "en-US",
    }
    .to_string();
    let (prompt_key, summary_prefix) = match action {
        ActionId::ImageSearch => ("image_search", "image_search"),
        _ => ("text_search", "text_search"),
    };
    let summary_prompt = render_research_prompt(state, config, prompt_key, input_text, mode, &lang);
    tracing::debug!(prompt = %summary_prefix, "研究摘要提示词已渲染");
    let fetch_pages = match mode {
        Mode::Quick => 0,
        Mode::Deep => (config.modes.deep_budget.fetch.max(0) as usize).min(MAX_FETCH_PAGES),
    };
    let max_queries = match mode {
        Mode::Quick => 1,
        Mode::Deep => (config.modes.deep_budget.search.max(1) as usize).min(MAX_IMAGE_QUERIES),
    };
    ResearchOptions {
        mode,
        lang,
        search_count: match mode {
            Mode::Quick => QUICK_SEARCH_COUNT,
            Mode::Deep => DEEP_SEARCH_COUNT,
        },
        fetch_pages,
        fetch_max_chars: EVIDENCE_PER_SOURCE_CHARS,
        evidence_total_chars: EVIDENCE_TOTAL_CHARS,
        max_queries,
        summary_prompt,
    }
}

/// 渲染研究摘要提示词（`prompt_engine`，失败时回退最小提示词并告警）。
fn render_research_prompt(
    state: &AppState,
    config: &Config,
    prompt_key: &str,
    input_text: &str,
    mode: Mode,
    target_lang: &str,
) -> String {
    let lang = match config.app.language {
        crate::contract::config::Language::ZhCn => "zh-CN",
        crate::contract::config::Language::EnUs => "en-US",
    };
    let glossary = translator::render_glossary(config);
    let mut vars: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
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
            tracing::warn!(code = %err.code, key = %prompt_key, "研究提示词渲染失败，回退最小提示词");
            format!("请基于资料回答问题，使用语言 {lang}，模式 {}", mode.as_str())
        })
}
