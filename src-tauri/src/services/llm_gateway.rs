//! LLM 网关（M09 / 契约 §12 流式占位行为）。
//!
//! 抽象为 [`LlmGateway`]：真实实现为 OpenAI 兼容 SSE（`TODO(real)`），骨架实现
//! [`SkeletonLlmGateway`] 以 25–40ms/块确定性输出占位内容，事件流与真实实现一致。
//!
//! TODO(real): OpenAI 兼容流式
//! 1) `POST {baseUrl}/v1/chat/completions`，`stream: true`，解析 `data:` 行与 `[DONE]`；
//! 2) 429/5xx/超时指数退避 ≤ 2 次（401/402 不重试）；
//! 3) 思考开关 `thinking: {"type":"disabled"}`（供应商不支持则忽略）；
//! 4) 多模态 `content` 数组含 `image_url`（data URL）；
//! 5) 计费按真实 `usage` 字段换算人民币。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{
    ActionId, Currency, EngineId, Mode, SessionId, SourceRef, StreamKind, TaskId, TokenUsage,
};

/// 一个流式增量。
#[derive(Debug, Clone, PartialEq)]
pub struct StreamChunk {
    /// 增量类型。
    pub kind: StreamKind,
    /// 增量文本。
    pub chunk: String,
}

/// LLM 调用请求。
#[derive(Debug, Clone)]
pub struct LlmRequest {
    /// 任务 ID。
    pub task: TaskId,
    /// 会话 ID。
    pub session: SessionId,
    /// 动作。
    pub action: ActionId,
    /// 模式。
    pub mode: Mode,
    /// 引擎 ID（主路径）。
    pub engine: EngineId,
    /// 渲染后的提示词。
    pub prompt: String,
    /// 用户输入原文（翻译 / 复制 / 占位回答使用）。
    pub input_text: String,
    /// 图像 DataURL（受隐私开关约束）。
    pub image_data_url: Option<String>,
    /// 取消令牌。
    pub cancel: Arc<AtomicBool>,
}

/// 任务结算结果。
#[derive(Debug, Clone, PartialEq)]
pub struct TaskOutcome {
    /// Token 用量与费用。
    pub usage: TokenUsage,
    /// 端到端耗时。
    pub latency_ms: u64,
    /// 实际使用的引擎。
    pub engine: EngineId,
    /// 来源列表。
    pub sources: Vec<SourceRef>,
}

/// LLM 网关抽象。
#[async_trait]
pub trait LlmGateway: Send + Sync {
    /// 执行流式请求；每个增量通过 `sink` 推送，结束后返回结算结果。
    async fn stream(
        &self,
        request: &LlmRequest,
        sink: &(dyn Fn(StreamChunk) + Send + Sync),
    ) -> AppResult<TaskOutcome>;
}

/// 骨架网关（契约 §12）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SkeletonLlmGateway;

const CHUNK_SIZE: usize = 16;
const CHUNK_DELAY_MS: u64 = 30;

impl SkeletonLlmGateway {
    /// 按动作生成占位内容（确定性，便于联调）。
    pub fn placeholder_text(action: ActionId, input: &str, mode: Mode) -> String {
        let preview: String = input.chars().take(60).collect();
        match action {
            ActionId::Translate => format!("【骨架模式译文】{input}"),
            ActionId::ImageSearch => format!(
                "【骨架模式】图片检索为占位结果。\n\n识别结果：{preview}\n检索词：占位检索词\n结论：当前未接入真实模型与检索服务，事件流与真实实现一致。"
            ),
            ActionId::Search => format!(
                "【骨架模式】AI 搜索占位回答\n\n已收到你的选区文本（{} 字）。\n当前未接入真实模型，事件流与真实实现一致。\n关闭骨架模式并配置 API Key 后即可获得真实结果。",
                input.chars().count()
            ),
            ActionId::Answer => format!(
                "【骨架模式】解答占位回答\n\n问题摘录：{preview}\n模式：{}。\n当前未接入真实模型，事件流与真实实现一致。\n关闭骨架模式并配置 API Key 后即可获得真实结果。",
                mode
            ),
            ActionId::Copy => input.to_string(),
        }
    }

    /// 按动作生成占位来源。
    pub fn placeholder_sources(action: ActionId) -> Vec<SourceRef> {
        match action {
            ActionId::ImageSearch => vec![
                SourceRef {
                    title: "骨架模式占位来源 1".to_string(),
                    url: "https://example.com/skeleton/image-search-1".to_string(),
                    site_name: Some("example.com".to_string()),
                    snippet: Some("骨架模式：图片检索来源占位".to_string()),
                },
                SourceRef {
                    title: "骨架模式占位来源 2".to_string(),
                    url: "https://example.com/skeleton/image-search-2".to_string(),
                    site_name: Some("example.com".to_string()),
                    snippet: Some("骨架模式：图片检索来源占位".to_string()),
                },
            ],
            _ => Vec::new(),
        }
    }
}

#[async_trait]
impl LlmGateway for SkeletonLlmGateway {
    async fn stream(
        &self,
        request: &LlmRequest,
        sink: &(dyn Fn(StreamChunk) + Send + Sync),
    ) -> AppResult<TaskOutcome> {
        let started = Instant::now();
        let content = Self::placeholder_text(request.action, &request.input_text, request.mode);
        let sources = Self::placeholder_sources(request.action);

        // 复制动作不需要流式输出，一次性返回。
        if request.action == ActionId::Copy {
            sink(StreamChunk {
                kind: StreamKind::Text,
                chunk: content,
            });
            return Ok(TaskOutcome {
                usage: zero_usage(),
                latency_ms: started.elapsed().as_millis() as u64,
                engine: request.engine.clone(),
                sources,
            });
        }

        for piece in chunk_string(&content, CHUNK_SIZE) {
            if request.cancel.load(Ordering::SeqCst) {
                return Err(AppError::invalid_state("任务已取消"));
            }
            tokio::time::sleep(Duration::from_millis(CHUNK_DELAY_MS)).await;
            sink(StreamChunk {
                kind: StreamKind::Text,
                chunk: piece,
            });
        }

        if request.cancel.load(Ordering::SeqCst) {
            return Err(AppError::invalid_state("任务已取消"));
        }

        // 契约 §12 固定用量。
        Ok(TaskOutcome {
            usage: TokenUsage {
                input_tokens: 120,
                output_tokens: 60,
                cached_input_tokens: 0,
                cost_estimate: 0.0,
                currency: Currency::Cny,
            },
            latency_ms: started.elapsed().as_millis() as u64,
            engine: request.engine.clone(),
            sources,
        })
    }
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

/// 按字符数切块（中文按字符而非字节，避免 WebView 渲染半个汉字）。
pub fn chunk_string(content: &str, chunk_size: usize) -> Vec<String> {
    if content.is_empty() || chunk_size == 0 {
        return Vec::new();
    }
    let chars: Vec<char> = content.chars().collect();
    chars
        .chunks(chunk_size)
        .map(|slice| slice.iter().collect::<String>())
        .collect()
}

/// 契约 §12 固定错误：骨架模式未接入的能力。
pub fn skeleton_unavailable(what: &str) -> AppError {
    AppError::new(
        ErrorCode::SkeletonNotImplemented,
        Some(format!("骨架模式未实现：{what}")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(action: ActionId, input: &str) -> LlmRequest {
        LlmRequest {
            task: TaskId::from("t-1"),
            session: SessionId::from("s-1"),
            action,
            mode: Mode::Quick,
            engine: "skeleton".to_string(),
            prompt: String::new(),
            input_text: input.to_string(),
            image_data_url: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn chunking_is_char_based() {
        let chunks = chunk_string("你好世界abcdef", 3);
        assert_eq!(chunks, vec!["你好世", "界abc", "def"]);
        assert!(chunk_string("", 4).is_empty());
    }

    #[test]
    fn placeholder_text_matches_contract_semantics() {
        assert_eq!(
            SkeletonLlmGateway::placeholder_text(ActionId::Translate, "hello", Mode::Quick),
            "【骨架模式译文】hello"
        );
        assert!(SkeletonLlmGateway::placeholder_text(ActionId::Copy, "abc", Mode::Quick) == "abc");
        assert!(SkeletonLlmGateway::placeholder_text(ActionId::Answer, "问题", Mode::Deep)
            .contains("解答占位回答"));
        assert_eq!(
            SkeletonLlmGateway::placeholder_sources(ActionId::ImageSearch).len(),
            2
        );
        assert!(SkeletonLlmGateway::placeholder_sources(ActionId::Answer).is_empty());
    }

    #[tokio::test]
    async fn skeleton_streams_text_and_reports_fixed_usage() {
        let gateway = SkeletonLlmGateway;
        let chunks = std::sync::Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
        let sink_chunks = chunks.clone();
        let sink = move |chunk: StreamChunk| {
            sink_chunks.lock().push(chunk.chunk);
        };
        let outcome = gateway
            .stream(&request(ActionId::Answer, "测试问题"), &sink)
            .await
            .unwrap();
        assert!(!chunks.lock().is_empty());
        assert_eq!(outcome.usage.input_tokens, 120);
        assert_eq!(outcome.usage.output_tokens, 60);
        assert_eq!(outcome.usage.cost_estimate, 0.0);
        assert_eq!(outcome.engine, "skeleton");
    }

    #[tokio::test]
    async fn cancelled_request_stops_without_outcome() {
        let gateway = SkeletonLlmGateway;
        let request = request(ActionId::Answer, "测试");
        request.cancel.store(true, Ordering::SeqCst);
        let sink = |_chunk: StreamChunk| {};
        let error = gateway.stream(&request, &sink).await.unwrap_err();
        assert_eq!(error.code, "SA-1099");
    }
}
