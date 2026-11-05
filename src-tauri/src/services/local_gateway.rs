//! 真实 OpenAI 兼容流式 LLM 网关（M09 / 研究 §4.1 / 契约 §12 备注）。
//!
//! 同一实现服务两种后端：
//! - **本地 llama-server**：`http://127.0.0.1:<port>`（引擎给的 `base_url` **不含 `/v1`**），
//!   本网关内部拼 `/v1/chat/completions`，无 API Key；
//! - **云端供应商**：`<baseUrl>/v1/chat/completions`，`Authorization: Bearer <key>`。
//!
//! 行为：
//! 1. `stream: true`，逐行解析 `data: {...}` / `data: [DONE]`，取 `choices[0].delta.content`
//!    （`reasoning_content` 作为 reasoning 增量）；
//! 2. 429/5xx/连接错误/超时指数退避 **≤ 2 次**（401/402/403 不重试，映射 SA-3002/SA-3003）；
//! 3. 思考开关 `thinking: {"type":"disabled"}`（仅云端 + 快速模式；供应商不支持则忽略）；
//! 4. 多模态 `content` 数组含 `image_url`（data URL）；
//! 5. 按真实 `usage` 换算人民币（本地引擎无 usage 时按字符估算，成本恒 0）；
//! 6. 取消令牌在块间检查，取消返回 `SA-1099`（与骨架网关语义一致）。

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{Currency, Mode, StreamKind, TokenUsage};
use crate::domain::usage::PricingTable;
use crate::services::llm_gateway::{LlmGateway, LlmRequest, StreamChunk, TaskOutcome};

/// 429/5xx 的最大重试次数（不含首次请求）。
pub const MAX_RETRIES: u32 = 2;
/// 首次退避时长（指数退避：300ms → 600ms）。
pub const BASE_BACKOFF: Duration = Duration::from_millis(300);
/// 退避上限。
pub const MAX_BACKOFF: Duration = Duration::from_secs(2);
/// 本地请求整体超时（1.8B 冷启动 + 流式生成给足余量）。
pub const LOCAL_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// 云端请求整体超时。
pub const CLOUD_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// 快速模式最大输出 token（`LlmRequest` 不携带供应商温度/上限，取契约默认档位）。
pub const QUICK_MAX_TOKENS: u32 = 1024;
/// 深度模式最大输出 token。
///
/// v1.0.0：从 1500 提升到 4096。DeepSeek 深度模式默认开启 thinking，思考 token 会挤占
/// max_tokens 预算；T3 真实 E2E 实测 1500 时偶发正文为空 / 来源段被截断。4096 给
/// 「思考 + 正文 + 来源」留足空间（快速模式仍走 1024 且显式关闭 thinking）。
pub const DEEP_MAX_TOKENS: u32 = 4096;
/// 默认温度。
pub const DEFAULT_TEMPERATURE: f64 = 0.2;
/// 错误 detail 中保留的上游响应体长度上限。
const ERROR_BODY_LIMIT: usize = 500;

/// OpenAI 兼容流式网关。
#[derive(Debug, Clone)]
pub struct HttpLlmGateway {
    /// 基础 URL：本地为 `http://127.0.0.1:<port>`（不含 `/v1`）；
    /// 云端为 `https://<host>/v1`（`AppState::gateway_for` 已拼 `/v1`）。
    pub base_url: String,
    /// 明文 API Key；本地引擎为 `None`。
    pub api_key: Option<String>,
    /// 模型名。
    pub model: String,
}

impl HttpLlmGateway {
    /// 新建。
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
            model: model.into(),
        }
    }

    /// 是否为本地 llama-server 端点。
    ///
    /// 判定 = 回环地址 **且** 无 API Key：本地引擎用 `127.0.0.1` 且不带 Key；
    /// 云端即使走本地代理/回环 mock，只要带 Key 就按云端语义处理（`stream_options` /
    /// `thinking` / 计费）。避免「回环地址 = 本地」的误判导致云端字段缺失。
    pub fn is_local(&self) -> bool {
        let loopback = self.base_url.contains("127.0.0.1") || self.base_url.contains("localhost");
        let no_key = self
            .api_key
            .as_deref()
            .map_or(true, |key| key.trim().is_empty());
        loopback && no_key
    }

    /// 去掉尾部 `/chat/completions` 与 `/v1` 后的根地址。
    fn root(&self) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        let base = base.strip_suffix("/chat/completions").unwrap_or(base);
        let base = base.strip_suffix("/v1").unwrap_or(base);
        base.trim_end_matches('/').to_string()
    }

    /// `POST /v1/chat/completions` 端点（兼容 `base_url` 带 / 不带 `/v1`）。
    pub fn chat_completions_url(&self) -> String {
        format!("{}/v1/chat/completions", self.root())
    }

    /// `GET /health` 端点（引擎就绪探测同款，便于诊断）。
    pub fn health_url(&self) -> String {
        format!("{}/health", self.root())
    }

    /// 构造 HTTP 客户端（流式：不设过短的读超时）。
    pub fn client(&self) -> AppResult<reqwest::Client> {
        let timeout = if self.is_local() {
            LOCAL_REQUEST_TIMEOUT
        } else {
            CLOUD_REQUEST_TIMEOUT
        };
        crate::platform::http::client_builder_for(&self.base_url)
            .connect_timeout(Duration::from_secs(15))
            .timeout(timeout)
            .build()
            .map_err(|err| AppError::io_failed(format!("创建 HTTP 客户端失败：{err}")))
    }

    /// 渲染请求体（纯函数，便于单测）。
    pub fn request_body(&self, request: &LlmRequest) -> Value {
        let prompt = display_prompt(request);
        let content = match request.image_data_url.as_deref() {
            Some(url) if !url.trim().is_empty() => json!([
                { "type": "text", "text": prompt },
                { "type": "image_url", "image_url": { "url": url } }
            ]),
            _ => Value::String(prompt.to_string()),
        };
        let max_tokens = match request.mode {
            Mode::Quick => QUICK_MAX_TOKENS,
            Mode::Deep => DEEP_MAX_TOKENS,
        };
        let mut body = json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": content }],
            "stream": true,
            "temperature": DEFAULT_TEMPERATURE,
            "max_tokens": max_tokens,
        });
        if !self.is_local() {
            // 只有云端才带 `stream_options` / `thinking`：本地 llama-server 对未知字段的
            // 容忍度因构建而异，宁可不发（`include_usage` 缺失时按字符估算 usage）。
            body["stream_options"] = json!({ "include_usage": true });
            if request.mode == Mode::Quick {
                body["thinking"] = json!({ "type": "disabled" });
            }
        }
        body
    }

    /// 消费 SSE 流：逐块推 `sink`，返回 `(usage, 正文全文)`。
    async fn consume_stream(
        &self,
        response: reqwest::Response,
        request: &LlmRequest,
        sink: &(dyn Fn(StreamChunk) + Send + Sync),
    ) -> AppResult<(Option<(u64, u64)>, String)> {
        let mut response = response;
        let mut buffer: Vec<u8> = Vec::new();
        let mut usage: Option<(u64, u64)> = None;
        let mut output = String::new();
        loop {
            if request.cancel.load(Ordering::SeqCst) {
                return Err(AppError::invalid_state("任务已取消"));
            }
            let chunk = match response.chunk().await {
                Ok(chunk) => chunk,
                Err(err) => {
                    if request.cancel.load(Ordering::SeqCst) {
                        return Err(AppError::invalid_state("任务已取消"));
                    }
                    return Err(map_transport_error(&err));
                }
            };
            let Some(bytes) = chunk else { break };
            buffer.extend_from_slice(&bytes);
            for line in drain_complete_lines(&mut buffer) {
                let Some(event) = classify_sse_line(&line) else {
                    continue;
                };
                match event {
                    SseEvent::Done => return Ok((usage, output)),
                    SseEvent::Data(payload) => {
                        let parsed = parse_sse_data(&payload);
                        if let Some(message) = parsed.error {
                            return Err(AppError::new(
                                ErrorCode::NetworkUnavailable,
                                Some(format!("上游流式返回错误：{message}")),
                            ));
                        }
                        if let Some(found) = parsed.usage {
                            usage = Some(found);
                        }
                        for piece in parsed.chunks {
                            if request.cancel.load(Ordering::SeqCst) {
                                return Err(AppError::invalid_state("任务已取消"));
                            }
                            if piece.kind == StreamKind::Text {
                                output.push_str(&piece.chunk);
                            }
                            sink(piece);
                        }
                    }
                }
            }
        }
        // 宽容处理：部分代理/网关不发送 `[DONE]` 直接关闭连接。
        Ok((usage, output))
    }

    /// 结算结果（usage / 费用 / 耗时）。
    fn outcome(
        &self,
        request: &LlmRequest,
        usage: Option<(u64, u64)>,
        output: &str,
        started: Instant,
    ) -> TaskOutcome {
        let prompt = display_prompt(request);
        let (input_tokens, output_tokens) = match usage {
            Some((input, output_tokens)) => (input, output_tokens),
            None => (estimate_tokens(prompt), estimate_tokens(output)),
        };
        let usage = if self.is_local() {
            TokenUsage {
                input_tokens,
                output_tokens,
                cached_input_tokens: 0,
                cost_estimate: 0.0,
                currency: Currency::Cny,
            }
        } else {
            match PricingTable::from_json(crate::resources::PRICING) {
                Ok(table) => table.estimate(&self.model, input_tokens, output_tokens, 0),
                Err(_) => TokenUsage {
                    input_tokens,
                    output_tokens,
                    cached_input_tokens: 0,
                    cost_estimate: 0.0,
                    currency: Currency::Cny,
                },
            }
        };
        TaskOutcome {
            usage,
            latency_ms: started.elapsed().as_millis() as u64,
            engine: request.engine.clone(),
            sources: Vec::new(),
        }
    }

    /// HTTP 状态码 → 契约错误码（401/402/403 不重试）。
    fn map_http_error(&self, status: u16, body: &str) -> AppError {
        let detail = truncate(body.trim(), ERROR_BODY_LIMIT);
        let with_status = |message: &str| format!("HTTP {status}（{message}）：{detail}");
        match status {
            401 | 403 => AppError::new(
                ErrorCode::AuthFailed,
                Some(with_status("API Key 无效或无权限")),
            ),
            402 => AppError::new(ErrorCode::QuotaExceeded, Some(with_status("额度不足"))),
            404 => AppError::new(
                ErrorCode::ModelMissing,
                Some(format!(
                    "HTTP 404（端点或模型不存在：{}）：{detail}",
                    self.chat_completions_url()
                )),
            ),
            408 => AppError::new(ErrorCode::Timeout, Some(with_status("上游超时"))),
            429 => AppError::new(
                ErrorCode::RateLimited,
                Some(with_status("触发限流，重试后仍失败")),
            ),
            _ if (500..600).contains(&status) => AppError::new(
                ErrorCode::NetworkUnavailable,
                Some(with_status("上游服务错误，重试后仍失败")),
            ),
            _ => AppError::new(
                ErrorCode::InvalidState,
                Some(format!("LLM 请求被上游拒绝（HTTP {status}）：{detail}")),
            ),
        }
    }
}

#[async_trait]
impl LlmGateway for HttpLlmGateway {
    async fn stream(
        &self,
        request: &LlmRequest,
        sink: &(dyn Fn(StreamChunk) + Send + Sync),
    ) -> AppResult<TaskOutcome> {
        let started = Instant::now();
        let client = self.client()?;
        let url = self.chat_completions_url();
        let body = self.request_body(request);
        let payload = serde_json::to_string(&body).map_err(|err| {
            AppError::invalid_config("llm_request", format!("请求体序列化失败：{err}"))
        })?;
        let mut attempt: u32 = 0;
        loop {
            if request.cancel.load(Ordering::SeqCst) {
                return Err(AppError::invalid_state("任务已取消"));
            }
            // 说明：reqwest 未启用 `json` feature（见 Cargo.toml），这里手动序列化并设置
            // `Content-Type: application/json`，避免新增依赖特性。
            let mut builder = client
                .post(&url)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(payload.clone());
            if let Some(key) = self.api_key.as_deref() {
                if !key.trim().is_empty() {
                    builder = builder.bearer_auth(key.trim());
                }
            }
            match builder.send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        let (usage, output) = self.consume_stream(response, request, sink).await?;
                        return Ok(self.outcome(request, usage, &output, started));
                    }
                    let body_text = response.text().await.unwrap_or_default();
                    let retryable_status = status.as_u16() == 429 || status.is_server_error();
                    let mapped = self.map_http_error(status.as_u16(), &body_text);
                    if retryable_status && attempt < MAX_RETRIES {
                        let backoff = backoff_for(attempt);
                        tracing::warn!(
                            status = status.as_u16(),
                            attempt,
                            backoff_ms = backoff.as_millis() as u64,
                            "LLM 请求失败，指数退避后重试"
                        );
                        tokio::time::sleep(backoff).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(mapped);
                }
                Err(err) => {
                    let retryable = err.is_timeout() || err.is_connect() || err.is_request();
                    if retryable && attempt < MAX_RETRIES {
                        let backoff = backoff_for(attempt);
                        tracing::warn!(
                            error = %err,
                            attempt,
                            backoff_ms = backoff.as_millis() as u64,
                            "LLM 连接失败，指数退避后重试"
                        );
                        tokio::time::sleep(backoff).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(map_transport_error(&err));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 纯函数（可单测）
// ---------------------------------------------------------------------------

/// 展示给模型的提示词：优先 `prompt`，否则退回 `input_text`。
pub fn display_prompt(request: &LlmRequest) -> &str {
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        &request.input_text
    } else {
        &request.prompt
    }
}

/// 指数退避：`BASE_BACKOFF × 2^attempt`，封顶 [`MAX_BACKOFF`]。
pub fn backoff_for(attempt: u32) -> Duration {
    let factor = 1u32 << attempt.min(20);
    (BASE_BACKOFF * factor).min(MAX_BACKOFF)
}

/// transport 层错误 → 契约错误码（超时 SA-3005 / 其他网络 SA-3001）。
pub fn map_transport_error(err: &reqwest::Error) -> AppError {
    if err.is_timeout() {
        AppError::new(ErrorCode::Timeout, Some(format!("LLM 请求超时：{err}")))
    } else if err.is_connect() {
        AppError::new(
            ErrorCode::NetworkUnavailable,
            Some(format!("无法连接 LLM 端点：{err}")),
        )
    } else {
        AppError::new(
            ErrorCode::NetworkUnavailable,
            Some(format!("LLM 网络错误：{err}")),
        )
    }
}

/// 粗略 token 估算（本地引擎不返回 usage 时使用）：
/// CJK 字符约 1 token/字，其他字符约 4 字符/token。
pub fn estimate_tokens(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    let mut cjk: u64 = 0;
    let mut other: u64 = 0;
    for ch in text.chars() {
        if ch as u32 >= 0x2E80 {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    (cjk + other.div_ceil(4)).max(1)
}

/// 从字节缓冲区取出所有「完整行」（`\n` 分隔；去掉行尾 `\r`），未完成的后半行保留。
pub fn drain_complete_lines(buffer: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
        let line_bytes: Vec<u8> = buffer.drain(..=position).collect();
        let text = String::from_utf8_lossy(&line_bytes[..line_bytes.len() - 1]).into_owned();
        lines.push(text.trim_end_matches('\r').to_string());
    }
    lines
}

/// 一条 SSE 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    /// `data: {json}`。
    Data(String),
    /// `data: [DONE]`。
    Done,
}

/// 解析一行 SSE：只认 `data:` 前缀；`event:` / `id:` / `: comment` / 空行（事件分隔）忽略。
pub fn classify_sse_line(line: &str) -> Option<SseEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let data = trimmed.strip_prefix("data:")?.trim();
    if data.is_empty() {
        return None;
    }
    if data == "[DONE]" {
        return Some(SseEvent::Done);
    }
    Some(SseEvent::Data(data.to_string()))
}

/// 单个 `data:` JSON 的解析结果。
#[derive(Debug, Default)]
pub struct SseDelta {
    /// 文本 / 思考增量（按出现顺序）。
    pub chunks: Vec<StreamChunk>,
    /// `(prompt_tokens, completion_tokens)`。
    pub usage: Option<(u64, u64)>,
    /// 上游在流内返回的错误消息。
    pub error: Option<String>,
}

/// 解析一个 SSE `data:` 载荷（纯函数）。
///
/// 兼容：
/// - OpenAI / llama-server：`choices[0].delta.content`、`choices[0].delta.reasoning_content`；
/// - usage：顶层 `usage.{prompt_tokens,completion_tokens}`，或 llama.cpp `timings.{prompt_n,predicted_n}`；
/// - 空 content / 空 choices 不产生 panic（返回空增量）。
pub fn parse_sse_data(payload: &str) -> SseDelta {
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return SseDelta {
            error: Some(format!("无法解析的 SSE JSON：{}", truncate(payload, 200))),
            ..SseDelta::default()
        };
    };
    let mut delta = SseDelta::default();
    if let Some(error) = value.get("error").filter(|item| !item.is_null()) {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| truncate(&error.to_string(), 200));
        delta.error = Some(message);
        return delta;
    }
    if let Some(choice) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
    {
        if let Some(reasoning) = choice
            .get("delta")
            .and_then(|item| item.get("reasoning_content"))
            .and_then(Value::as_str)
        {
            if !reasoning.is_empty() {
                delta.chunks.push(StreamChunk {
                    kind: StreamKind::Reasoning,
                    chunk: reasoning.to_string(),
                });
            }
        }
        if let Some(content) = choice
            .get("delta")
            .and_then(|item| item.get("content"))
            .and_then(Value::as_str)
        {
            if !content.is_empty() {
                delta.chunks.push(StreamChunk {
                    kind: StreamKind::Text,
                    chunk: content.to_string(),
                });
            }
        }
    }
    delta.usage = parse_usage(&value);
    delta
}

/// 从 JSON 中提取 `(input, output)` token 用量。
pub fn parse_usage(value: &Value) -> Option<(u64, u64)> {
    if let Some(usage) = value.get("usage").filter(|item| !item.is_null()) {
        let prompt = usage.get("prompt_tokens").and_then(Value::as_u64);
        let completion = usage.get("completion_tokens").and_then(Value::as_u64);
        if prompt.is_some() || completion.is_some() {
            return Some((prompt.unwrap_or(0), completion.unwrap_or(0)));
        }
    }
    if let Some(timings) = value.get("timings") {
        let prompt = timings.get("prompt_n").and_then(Value::as_u64);
        let predicted = timings.get("predicted_n").and_then(Value::as_u64);
        if prompt.is_some() || predicted.is_some() {
            return Some((prompt.unwrap_or(0), predicted.unwrap_or(0)));
        }
    }
    None
}

/// 截断字符串到 `limit` 字符（错误 detail 用，避免把整段响应塞进日志/UI）。
pub fn truncate(text: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= limit {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::types::{ActionId, SessionId, TaskId};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn request(prompt: &str) -> LlmRequest {
        LlmRequest {
            task: TaskId::from("t-1"),
            session: SessionId::from("s-1"),
            action: ActionId::Translate,
            mode: Mode::Quick,
            engine: "local".to_string(),
            prompt: prompt.to_string(),
            input_text: prompt.to_string(),
            image_data_url: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn endpoints_tolerate_base_url_with_or_without_v1() {
        let plain = HttpLlmGateway::new("http://127.0.0.1:39281", None, "m");
        assert_eq!(
            plain.chat_completions_url(),
            "http://127.0.0.1:39281/v1/chat/completions"
        );
        assert_eq!(plain.health_url(), "http://127.0.0.1:39281/health");
        assert!(plain.is_local());

        let with_v1 = HttpLlmGateway::new("http://127.0.0.1:39281/v1", None, "m");
        assert_eq!(
            with_v1.chat_completions_url(),
            "http://127.0.0.1:39281/v1/chat/completions"
        );
        assert_eq!(with_v1.health_url(), "http://127.0.0.1:39281/health");

        let cloud = HttpLlmGateway::new("https://api.deepseek.com/v1", Some("k".into()), "m");
        assert_eq!(
            cloud.chat_completions_url(),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert!(!cloud.is_local());
    }

    #[test]
    fn request_body_is_streaming_and_local_stays_minimal() {
        let local = HttpLlmGateway::new("http://127.0.0.1:39281", None, "hy-mt");
        let body = local.request_body(&request("hello"));
        assert_eq!(body["model"], "hy-mt");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["content"], "hello");
        assert_eq!(body["max_tokens"], QUICK_MAX_TOKENS);
        assert!(body.get("stream_options").is_none());
        assert!(body.get("thinking").is_none());

        let cloud = HttpLlmGateway::new("https://api.deepseek.com/v1", Some("k".into()), "m");
        let body = cloud.request_body(&request("hello"));
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["thinking"]["type"], "disabled");

        let mut deep = request("hello");
        deep.mode = Mode::Deep;
        let body = cloud.request_body(&deep);
        assert_eq!(body["max_tokens"], DEEP_MAX_TOKENS);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn multimodal_image_uses_content_array() {
        let gateway = HttpLlmGateway::new("https://api.deepseek.com/v1", Some("k".into()), "m");
        let mut req = request("这是什么");
        req.image_data_url = Some("data:image/png;base64,AAAA".to_string());
        let body = gateway.request_body(&req);
        assert!(body["messages"][0]["content"].is_array());
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
        assert_eq!(
            body["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AAAA"
        );
    }

    #[test]
    fn drains_complete_lines_and_keeps_remainder() {
        let mut buffer = b"data: a\n\ndata: b\r\npart".to_vec();
        assert_eq!(
            drain_complete_lines(&mut buffer),
            vec!["data: a".to_string(), String::new(), "data: b".to_string()]
        );
        assert_eq!(buffer, b"part".to_vec());
        assert!(drain_complete_lines(&mut Vec::new()).is_empty());
    }

    #[test]
    fn classifies_sse_lines() {
        assert_eq!(
            classify_sse_line("data: {\"a\":1}"),
            Some(SseEvent::Data("{\"a\":1}".to_string()))
        );
        assert_eq!(classify_sse_line("data: [DONE]"), Some(SseEvent::Done));
        assert_eq!(classify_sse_line("event: message"), None);
        assert_eq!(classify_sse_line(": keep-alive"), None);
        assert_eq!(classify_sse_line(""), None);
        assert_eq!(classify_sse_line("data: "), None);
    }

    #[test]
    fn parses_content_reasoning_and_usage() {
        let delta = parse_sse_data(
            r#"{"choices":[{"delta":{"content":"你好","reasoning_content":"想一下"}}]}"#,
        );
        assert_eq!(delta.chunks.len(), 2);
        assert_eq!(delta.chunks[0].kind, StreamKind::Reasoning);
        assert_eq!(delta.chunks[1].kind, StreamKind::Text);
        assert_eq!(delta.chunks[1].chunk, "你好");
        assert!(delta.usage.is_none());
        assert!(delta.error.is_none());

        let delta =
            parse_sse_data(r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":34}}"#);
        assert!(delta.chunks.is_empty());
        assert_eq!(delta.usage, Some((12, 34)));

        let delta = parse_sse_data(
            r#"{"choices":[{"delta":{}}],"timings":{"prompt_n":7,"predicted_n":9}}"#,
        );
        assert!(delta.chunks.is_empty());
        assert_eq!(delta.usage, Some((7, 9)));

        // 空 content 不 panic。
        let delta = parse_sse_data(r#"{"choices":[{"delta":{"content":""}}]}"#);
        assert!(delta.chunks.is_empty());

        // 上游错误。
        let delta = parse_sse_data(r#"{"error":{"message":"model overloaded"}}"#);
        assert_eq!(delta.error.as_deref(), Some("model overloaded"));

        // 非法 JSON。
        let delta = parse_sse_data("not-json");
        assert!(delta.error.is_some());
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        assert_eq!(backoff_for(0), BASE_BACKOFF);
        assert_eq!(backoff_for(1), BASE_BACKOFF * 2);
        assert_eq!(backoff_for(2), BASE_BACKOFF * 4);
        assert_eq!(backoff_for(20), MAX_BACKOFF);
    }

    #[test]
    fn estimates_tokens_for_cjk_and_latin() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("你好世界"), 4);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
        assert_eq!(estimate_tokens("a"), 1);
    }

    #[test]
    fn truncates_long_text() {
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("ab", 3), "ab");
        assert_eq!(truncate("abc", 0), "");
    }

    #[test]
    fn http_errors_map_to_contract_codes() {
        let gateway = HttpLlmGateway::new("http://127.0.0.1:1", None, "m");
        assert_eq!(gateway.map_http_error(401, "x").code, "SA-3002");
        assert_eq!(gateway.map_http_error(403, "x").code, "SA-3002");
        assert_eq!(gateway.map_http_error(402, "x").code, "SA-3003");
        assert_eq!(gateway.map_http_error(429, "x").code, "SA-3004");
        assert_eq!(gateway.map_http_error(404, "x").code, "SA-4001");
        assert_eq!(gateway.map_http_error(408, "x").code, "SA-3005");
        assert_eq!(gateway.map_http_error(503, "x").code, "SA-3001");
        assert_eq!(gateway.map_http_error(400, "x").code, "SA-1099");
        // 错误 detail 不泄漏整段响应。
        assert!(
            gateway
                .map_http_error(400, &"x".repeat(10_000))
                .detail
                .unwrap()
                .chars()
                .count()
                <= ERROR_BODY_LIMIT + 100
        );
    }
}
