//! 搜索网关（M10 / 说明书 §4.4 / §4.5 / 契约 v5 §3.4）。
//!
//! 两种分支：
//! - `browser` 分支：拼装搜索引擎 URL（`build_search_url` 纯函数），`open_external` 用
//!   `tauri_plugin_opener` 真实打开系统默认浏览器（骨架模式只记日志，保留契约 §12 调试语义）；
//! - `ai` 分支：真实实现 [`DeepSeekToolsSearcher`] —— DeepSeek Search / Fetch API
//!   （`POST {base}/v1/tools/search`、`POST {base}/v1/tools/fetch`），Bearer 鉴权，
//!   结果映射为 [`SourceRef`]，错误映射契约 §8.2 错误码。
//!
//! # 实测协议事实（2026-11-06，真实 Key 实测，证据见 `.verify/v1/cloud/`）
//!
//! - **Search**：请求 `{"query": "1–128 Unicode 字符", "count": 1–50 (默认 10),
//!   "need_content": bool, "ori_question": "可选 1–128 字符", "localization": "可选 IP"}`。
//!   注意：spec 早期写的 `limit` **不是**真实字段（未知字段会被忽略），本实现把内部 `limit`
//!   映射为 `count`。响应 `{"results": [{"title","url","site_name","site_icon","summary",
//!   "snippet","content","date","authority","rerank_score"}]}`；`need_content=false` 时
//!   `content` 恒为空串。
//! - **Fetch**：请求 `{"url": "≤8192 字符、带 host、不含账号密码的 http(s) URL"}`。
//!   spec 早期写的 `max_chars` **不是**真实字段（未知字段被忽略，实测返回全文），
//!   本实现在客户端按 `max_chars` 截断（字符级，UTF-8 安全），并在结果里标明是否截断。
//!   响应 `{"title","content"}`（`content` 为 Markdown）。
//! - 计费：Search / Fetch 均**按成功调用次数**计费（仅 HTTP 200 且非空结果计费），
//!   单价来自 `resources/pricing.json` 的 `search[]`（`deepseek-search` / `deepseek-fetch`）。
//! - `open_external` 的安全白名单：https 全部允许，http 仅 127.0.0.1（契约 §6）。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{Mode, SourceRef};
use crate::domain::usage::PricingTable;

/// Search 端点路径（相对 Base URL）。
pub const TOOLS_SEARCH_PATH: &str = "/v1/tools/search";
/// Fetch 端点路径（相对 Base URL）。
pub const TOOLS_FETCH_PATH: &str = "/v1/tools/fetch";
/// Search 请求 `query` 最大字符数（真实协议约束）。
pub const QUERY_MAX_CHARS: usize = 128;
/// Search 单次结果条数默认值（真实协议默认 10）。
pub const SEARCH_COUNT_DEFAULT: usize = 10;
/// Search 单次结果条数上限（真实协议 1–50）。
pub const SEARCH_COUNT_MAX: usize = 50;
/// 客户端 Fetch 内容截断默认值（字符）。
pub const FETCH_MAX_CHARS_DEFAULT: usize = 4000;
/// 深度模式默认并发 Fetch 页数。
pub const FETCH_TOP_DEFAULT: usize = 2;
/// 连接超时。
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 单次 Search / Fetch 请求总超时。
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// 错误 detail 中保留的上游响应体长度上限（避免把整段 HTML 塞进 UI/日志）。
const ERROR_BODY_LIMIT: usize = 500;
/// 计费表里 Search 服务 id。
pub const PRICING_SEARCH_ID: &str = "deepseek-search";
/// 计费表里 Fetch 服务 id。
pub const PRICING_FETCH_ID: &str = "deepseek-fetch";

/// 搜索请求。
#[derive(Debug, Clone)]
pub struct SearchRequest {
    /// 搜索引擎标识（bing / google / baidu / duckduckgo / custom）。
    pub engine: String,
    /// 自定义 URL 模板（含 `{q}` 或 `{query}`）。
    pub template: Option<String>,
    /// 搜索词。
    pub query: String,
    /// 模式。
    pub mode: Mode,
}

/// 搜索结果。
#[derive(Debug, Clone, Default)]
pub struct SearchOutcome {
    /// 来源列表。
    pub sources: Vec<SourceRef>,
    /// 结果摘要文本。
    pub text: String,
    /// 本次实际发生的 Search 调用次数（计费用，如实回传）。
    pub search_calls: u32,
    /// 本次实际发生的 Fetch 调用次数（计费用，如实回传）。
    pub fetch_calls: u32,
}

/// Search API 的单条命中（保留协议里的全部可用字段，供摘要阶段使用）。
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// 标题。
    pub title: String,
    /// 链接。
    pub url: String,
    /// 站点名（`site_name`）。
    pub site_name: Option<String>,
    /// 展示摘要（约 100–200 字，`snippet`）。
    pub snippet: Option<String>,
    /// 模型摘要（约 600–800 字，`summary`）。
    pub summary: Option<String>,
    /// 正文 Markdown（`need_content=true` 时才有）。
    pub content: String,
    /// 发布时间（RFC 3339）。
    pub date: Option<String>,
    /// 权威度（0–4）。
    pub authority: Option<i64>,
    /// 相关性评分（0–3）。
    pub rerank_score: Option<f64>,
}

impl SearchHit {
    /// 映射为契约 `SourceRef`；snippet 缺失时回退 summary 前 160 字。
    pub fn to_source_ref(&self) -> SourceRef {
        let snippet = match self.snippet.as_deref() {
            Some(snippet) if !snippet.trim().is_empty() => Some(snippet.trim().to_string()),
            _ => self
                .summary
                .as_deref()
                .map(|summary| truncate(summary.trim(), 160))
                .filter(|summary| !summary.is_empty()),
        };
        SourceRef {
            title: if self.title.trim().is_empty() {
                self.url.clone()
            } else {
                self.title.trim().to_string()
            },
            url: self.url.clone(),
            site_name: self.site_name.clone(),
            snippet,
        }
    }

    /// 交给摘要模型的「证据文本」：优先正文，其次 summary，最后 snippet。
    pub fn evidence_text(&self, max_chars: usize) -> String {
        let body = if !self.content.trim().is_empty() {
            self.content.as_str()
        } else if let Some(summary) = self
            .summary
            .as_deref()
            .filter(|item| !item.trim().is_empty())
        {
            summary
        } else {
            self.snippet.as_deref().unwrap_or_default()
        };
        truncate(body.trim(), max_chars)
    }
}

/// Fetch API 的返回：页面标题 + Markdown 正文。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedPage {
    /// 页面标题（可能为空串）。
    pub title: String,
    /// 页面 URL（请求方回填）。
    pub url: String,
    /// Markdown 正文（已按 `max_chars` 客户端截断）。
    pub content: String,
    /// 是否发生截断。
    pub truncated: bool,
}

/// DeepSeek Search / Fetch 真实搜索器（契约 v5 §3.4）。
#[derive(Debug, Clone)]
pub struct DeepSeekToolsSearcher {
    /// Base URL（`https://api.deepseek.com`；允许带 `/v1`，内部会规整）。
    pub base_url: String,
    /// Bearer API Key。
    pub api_key: Option<String>,
}

impl DeepSeekToolsSearcher {
    /// 新建。
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
        }
    }

    /// 去掉尾部 `/`、`/v1`、`/v1/tools` 后的根地址。
    pub fn root(&self) -> String {
        let mut base = self.base_url.trim().trim_end_matches('/').to_string();
        for suffix in ["/v1/tools", "/v1"] {
            if let Some(stripped) = base.strip_suffix(suffix) {
                base = stripped.to_string();
            }
        }
        base.trim_end_matches('/').to_string()
    }

    /// Search 端点。
    pub fn search_url(&self) -> String {
        format!("{}{}", self.root(), TOOLS_SEARCH_PATH)
    }

    /// Fetch 端点。
    pub fn fetch_url(&self) -> String {
        format!("{}{}", self.root(), TOOLS_FETCH_PATH)
    }

    /// HTTP 客户端（连接 15s / 总 60s，带 UA）。
    pub fn client(&self) -> AppResult<reqwest::Client> {
        crate::platform::http::client_builder_for(&self.base_url)
            .user_agent(concat!("snip-ask/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| AppError::io_failed(format!("创建搜索 HTTP 客户端失败：{err}")))
    }

    /// 渲染 Search 请求体（纯函数，便于单测）。
    ///
    /// `limit` 会被收敛到 1–50；`query` 超过 128 字符会被截断（协议上限）。
    pub fn search_body(
        query: &str,
        limit: usize,
        need_content: bool,
        ori_question: Option<&str>,
    ) -> Value {
        let count = limit.clamp(1, SEARCH_COUNT_MAX);
        let mut body = json!({
            "query": normalize_query(query),
            "count": count,
            "need_content": need_content,
        });
        if let Some(question) = ori_question.map(str::trim).filter(|item| !item.is_empty()) {
            body["ori_question"] = Value::String(truncate(question, QUERY_MAX_CHARS));
        }
        body
    }

    /// 渲染 Fetch 请求体（真实协议**只有** `url`；`max_chars` 在客户端截断）。
    pub fn fetch_body(url: &str) -> Value {
        json!({ "url": url.trim() })
    }

    /// 解析 Search 响应体（纯函数）。空 `results` 视为合法（未命中不计费）。
    pub fn parse_search_response(body: &str) -> AppResult<Vec<SearchHit>> {
        let value: Value = serde_json::from_str(body).map_err(|err| {
            AppError::new(
                ErrorCode::InvalidState,
                Some(format!(
                    "Search 响应不是合法 JSON：{err}；响应片段：{}",
                    truncate(body, 200)
                )),
            )
        })?;
        let results = value
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::InvalidState,
                    Some(format!(
                        "Search 响应缺少 results 数组：{}",
                        truncate(body, 200)
                    )),
                )
            })?;
        Ok(results
            .iter()
            .filter_map(|item| {
                let url = item.get("url").and_then(Value::as_str)?.trim().to_string();
                if url.is_empty() {
                    return None;
                }
                Some(SearchHit {
                    title: item
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    url,
                    site_name: string_field(item, "site_name"),
                    snippet: string_field(item, "snippet"),
                    summary: string_field(item, "summary"),
                    content: item
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    date: string_field(item, "date"),
                    authority: item.get("authority").and_then(Value::as_i64),
                    rerank_score: item.get("rerank_score").and_then(Value::as_f64),
                })
            })
            .collect())
    }

    /// 解析 Fetch 响应体（纯函数）；按 `max_chars` 客户端截断。
    pub fn parse_fetch_response(body: &str, url: &str, max_chars: usize) -> AppResult<FetchedPage> {
        let value: Value = serde_json::from_str(body).map_err(|err| {
            AppError::new(
                ErrorCode::InvalidState,
                Some(format!(
                    "Fetch 响应不是合法 JSON：{err}；响应片段：{}",
                    truncate(body, 200)
                )),
            )
        })?;
        let content = value
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if content.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidState,
                Some(format!("Fetch 未返回正文（可能被站点拒绝）：{url}")),
            ));
        }
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let truncated = content.chars().count() > max_chars;
        Ok(FetchedPage {
            title,
            url: url.to_string(),
            content: truncate(&content, max_chars),
            truncated,
        })
    }

    /// HTTP 状态码 → 契约错误码（Search / Fetch 共用）。
    pub fn map_http_error(status: u16, body: &str, endpoint: &str) -> AppError {
        let detail = truncate(body.trim(), ERROR_BODY_LIMIT);
        let with_status =
            |message: &str| format!("{endpoint} 返回 HTTP {status}（{message}）：{detail}");
        match status {
            401 | 403 => AppError::new(
                ErrorCode::AuthFailed,
                Some(with_status("API Key 无效或无权限")),
            )
            .with_message("API Key 无效或没有权限：请检查设置中的 DeepSeek Key")
            .with_fallback(vec!["open_settings"]),
            402 => AppError::new(ErrorCode::QuotaExceeded, Some(with_status("额度不足")))
                .with_message("账户额度不足：请充值或换用其他供应商")
                .with_fallback(vec!["open_settings"]),
            408 => AppError::new(ErrorCode::Timeout, Some(with_status("上游超时")))
                .with_message("搜索服务超时：请稍后重试")
                .with_fallback(vec!["retry"]),
            429 => AppError::new(
                ErrorCode::RateLimited,
                Some(with_status("触发限流（5 QPS）")),
            )
            .with_message("搜索请求过于频繁：请稍后重试（Search/Fetch 各限 5 QPS）")
            .with_fallback(vec!["retry"]),
            _ if (500..600).contains(&status) => AppError::new(
                ErrorCode::NetworkUnavailable,
                Some(with_status("上游服务错误")),
            )
            .with_message("搜索服务暂时不可用：可稍后重试或改用浏览器搜索")
            .with_fallback(vec!["retry", "browser_search"]),
            400 | 404 | 422 => AppError::new(
                ErrorCode::InvalidConfig,
                Some(with_status("请求/端点被拒绝")),
            )
            .with_message(format!(
                "{endpoint} 请求被拒绝（HTTP {status}）：请检查 Base URL 与参数"
            )),
            _ => AppError::new(
                ErrorCode::InvalidState,
                Some(with_status("未预期的上游状态")),
            ),
        }
    }

    /// 传输层错误 → 契约错误码（超时 SA-3005 / 其他网络 SA-3001）。
    pub fn map_transport_error(err: &reqwest::Error, endpoint: &str) -> AppError {
        if err.is_timeout() {
            AppError::new(
                ErrorCode::Timeout,
                Some(format!("{endpoint} 请求超时：{err}")),
            )
            .with_message("搜索服务超时：请检查网络后重试")
            .with_fallback(vec!["retry", "browser_search"])
        } else {
            AppError::new(
                ErrorCode::NetworkUnavailable,
                Some(format!("{endpoint} 网络错误：{err}")),
            )
            .with_message("网络不可用：无法连接搜索服务，可改用浏览器搜索")
            .with_fallback(vec!["browser_search", "local_translate"])
        }
    }

    /// 单次 Search（真实 HTTP）。返回值附带实际调用次数 1（成功）/ 0（失败由 Err 表达）。
    pub async fn search(
        &self,
        client: &reqwest::Client,
        query: &str,
        limit: usize,
        need_content: bool,
        ori_question: Option<&str>,
    ) -> AppResult<Vec<SearchHit>> {
        let body = Self::search_body(query, limit, need_content, ori_question);
        let payload = serde_json::to_string(&body).map_err(|err| {
            AppError::invalid_config("search", format!("请求体序列化失败：{err}"))
        })?;
        let mut builder = client
            .post(self.search_url())
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload);
        if let Some(key) = self
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        {
            builder = builder.bearer_auth(key);
        }
        let response = builder
            .send()
            .await
            .map_err(|err| Self::map_transport_error(&err, "Search"))?;
        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(Self::map_http_error(status.as_u16(), &body_text, "Search"));
        }
        Self::parse_search_response(&body_text)
    }

    /// 单次 Fetch（真实 HTTP）。`max_chars` 为客户端截断上限。
    pub async fn fetch(
        &self,
        client: &reqwest::Client,
        url: &str,
        max_chars: usize,
    ) -> AppResult<FetchedPage> {
        let payload = serde_json::to_string(&Self::fetch_body(url))
            .map_err(|err| AppError::invalid_config("fetch", format!("请求体序列化失败：{err}")))?;
        let mut builder = client
            .post(self.fetch_url())
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload);
        if let Some(key) = self
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        {
            builder = builder.bearer_auth(key);
        }
        let response = builder
            .send()
            .await
            .map_err(|err| Self::map_transport_error(&err, "Fetch"))?;
        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(Self::map_http_error(status.as_u16(), &body_text, "Fetch"));
        }
        Self::parse_fetch_response(&body_text, url, max_chars)
    }
}

/// 搜索证据的计费估算：`(search_calls, fetch_calls)` → 人民币。
///
/// 价格来自 `resources/pricing.json`（`search[]`）；解析失败按 0 计，绝不因计费失败影响主流程。
pub fn search_fetch_cost(search_calls: u32, fetch_calls: u32) -> f64 {
    let Ok(table) = PricingTable::from_json(crate::resources::PRICING) else {
        return 0.0;
    };
    let search = table.search_cost(PRICING_SEARCH_ID) * f64::from(search_calls);
    let fetch = table.search_cost(PRICING_FETCH_ID) * f64::from(fetch_calls);
    ((search + fetch) * 1_000_000.0).round() / 1_000_000.0
}

/// 搜索词规范化：去首尾空白 + 截断到协议上限（字符级）。
pub fn normalize_query(query: &str) -> String {
    truncate(query.trim(), QUERY_MAX_CHARS)
}

/// 截断到 `limit` 字符（不切断 UTF-8）。
pub fn truncate(text: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= limit {
            break;
        }
        out.push(ch);
    }
    out
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

#[async_trait]
impl Searcher for DeepSeekToolsSearcher {
    /// 真实搜索：deep 模式额外并发 Fetch 前 [`FETCH_TOP_DEFAULT`] 条命中的正文。
    async fn search(&self, request: &SearchRequest) -> AppResult<SearchOutcome> {
        let client = self.client()?;
        let limit = match request.mode {
            Mode::Quick => SEARCH_COUNT_DEFAULT,
            Mode::Deep => SEARCH_COUNT_MAX.min(SEARCH_COUNT_DEFAULT * 2),
        };
        let hits = self
            .search(&client, &request.query, limit, false, None)
            .await?;
        let sources: Vec<SourceRef> = hits.iter().map(SearchHit::to_source_ref).collect();
        let mut search_calls = 1u32;
        let mut fetch_calls = 0u32;
        let mut text = String::new();
        for hit in &hits {
            text.push_str(&format!("- {}（{}）\n", hit.title, hit.url));
        }
        if request.mode == Mode::Deep {
            let urls: Vec<String> = hits
                .iter()
                .take(FETCH_TOP_DEFAULT)
                .map(|hit| hit.url.clone())
                .collect();
            let pages = fetch_concurrently(self, &client, &urls, FETCH_MAX_CHARS_DEFAULT).await;
            fetch_calls = pages.len() as u32;
            for page in pages {
                text.push_str(&format!(
                    "\n## {}\n{}\n",
                    if page.title.is_empty() {
                        page.url.as_str()
                    } else {
                        page.title.as_str()
                    },
                    page.content
                ));
            }
        }
        let _ = &mut search_calls;
        Ok(SearchOutcome {
            sources,
            text,
            search_calls,
            fetch_calls,
        })
    }
}

/// 并发抓取多个 URL（每个失败只记日志、不影响其它 URL）；返回成功页。
pub async fn fetch_concurrently(
    searcher: &DeepSeekToolsSearcher,
    client: &reqwest::Client,
    urls: &[String],
    max_chars: usize,
) -> Vec<FetchedPage> {
    if urls.is_empty() {
        return Vec::new();
    }
    let mut handles = Vec::with_capacity(urls.len());
    for url in urls {
        let searcher = searcher.clone();
        let client = client.clone();
        let url = url.clone();
        handles.push(tokio::spawn(async move {
            searcher.fetch(&client, &url, max_chars).await
        }));
    }
    let mut pages = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(page)) => pages.push(page),
            Ok(Err(err)) => {
                tracing::warn!(code = %err.code, "并发 Fetch 失败，已跳过该来源");
            }
            Err(err) => {
                tracing::warn!(error = %err, "并发 Fetch 任务异常，已跳过该来源");
            }
        }
    }
    pages
}

/// 搜索器抽象。
#[async_trait]
pub trait Searcher: Send + Sync {
    /// 执行搜索。
    async fn search(&self, request: &SearchRequest) -> AppResult<SearchOutcome>;
}

/// 骨架搜索器：不访问网络，返回占位文本（契约 §12）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SkeletonSearcher;

#[async_trait]
impl Searcher for SkeletonSearcher {
    async fn search(&self, request: &SearchRequest) -> AppResult<SearchOutcome> {
        let url = build_search_url(&request.engine, request.template.as_deref(), &request.query);
        tracing::info!(
            host = %url::Url::parse(&url).ok().and_then(|parsed| parsed.host_str().map(str::to_string)).unwrap_or_default(),
            "骨架模式：不打开浏览器、不访问搜索 API"
        );
        Ok(SearchOutcome {
            sources: Vec::new(),
            text: format!("骨架模式：将打开浏览器搜索 “{}”", request.query),
            search_calls: 0,
            fetch_calls: 0,
        })
    }
}

/// URL 查询串编码（UTF-8 percent-encoding）。
pub fn encode_query(query: &str) -> String {
    url::form_urlencoded::byte_serialize(query.as_bytes()).collect()
}

/// 按引擎拼装搜索 URL；自定义模板优先。
///
/// 模板占位符同时支持 `{q}` 与 `{query}`（契约 §4.1 未指定具体形态）。
pub fn build_search_url(engine: &str, template: Option<&str>, query: &str) -> String {
    let encoded = encode_query(query);
    if let Some(template) = template {
        if template.contains("{q}") || template.contains("{query}") {
            return template
                .replace("{q}", &encoded)
                .replace("{query}", &encoded);
        }
    }
    let base = match engine.to_ascii_lowercase().as_str() {
        "google" => "https://www.google.com/search?q=",
        "baidu" => "https://www.baidu.com/s?wd=",
        "duckduckgo" | "ddg" => "https://duckduckgo.com/?q=",
        // 默认 bing（契约 §4.1 默认引擎）。
        _ => "https://www.bing.com/search?q=",
    };
    format!("{base}{encoded}")
}

/// 判断外部 URL 是否被安全策略允许（契约 §6 `open_external` 白名单）。
pub fn is_allowed_url(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(parsed) => match parsed.scheme() {
            "https" => true,
            "http" => parsed.host_str() == Some("127.0.0.1"),
            _ => false,
        },
        Err(_) => false,
    }
}

/// 用系统默认浏览器真实打开外部链接（契约 v5 / T3）。
///
/// - 先经 [`is_allowed_url`] 白名单（https 全部允许；http 仅 127.0.0.1）；
/// - 真实实现走 `tauri_plugin_opener`（`AppHandle::opener().open_url`）；
/// - 骨架模式的「只记日志」由调用方 `task_runner` 负责（保持本函数语义单一、可复用）。
pub async fn open_external(app: Option<&tauri::AppHandle>, url: &str) -> AppResult<()> {
    if !is_allowed_url(url) {
        return Err(AppError::new(
            ErrorCode::UrlRejected,
            Some("仅允许 https:// 或 http://127.0.0.1".to_string()),
        ));
    }
    let Some(app) = app else {
        return Err(AppError::io_failed("缺少 AppHandle，无法打开系统浏览器"));
    };
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url.to_string(), None::<String>)
        .map_err(|err| {
            AppError::io_failed(format!(
                "打开系统浏览器失败（{}）：{err}",
                url::Url::parse(url)
                    .ok()
                    .and_then(|parsed| parsed.host_str().map(str::to_string))
                    .unwrap_or_else(|| "<invalid-url>".to_string())
            ))
        })
        .map_err(|err| err.with_fallback(vec!["retry"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_urls_for_known_engines() {
        assert_eq!(
            build_search_url("bing", None, "框选 AI"),
            format!("https://www.bing.com/search?q={}", encode_query("框选 AI"))
        );
        assert!(build_search_url("google", None, "rust").starts_with("https://www.google.com/"));
        assert!(build_search_url("baidu", None, "rust").starts_with("https://www.baidu.com/"));
        assert!(build_search_url("duckduckgo", None, "rust").starts_with("https://duckduckgo.com/"));
        // 未知引擎回落到 bing。
        assert!(build_search_url("unknown", None, "rust").starts_with("https://www.bing.com/"));
    }

    #[test]
    fn custom_template_takes_priority() {
        assert_eq!(
            build_search_url("bing", Some("https://example.com/s?q={q}"), "a b"),
            "https://example.com/s?q=a+b"
        );
        assert_eq!(
            build_search_url("bing", Some("https://example.com/?query={query}"), "a b"),
            "https://example.com/?query=a+b"
        );
        // 无占位符的模板回落到内置引擎。
        assert!(build_search_url("bing", Some("https://example.com/"), "a")
            .starts_with("https://www.bing.com/"));
    }

    #[test]
    fn query_is_percent_encoded() {
        assert_eq!(encode_query("a b&c"), "a+b%26c");
        assert_eq!(encode_query("中文"), "%E4%B8%AD%E6%96%87");
    }

    #[test]
    fn url_whitelist() {
        assert!(is_allowed_url("https://example.com/a?b=c"));
        assert!(is_allowed_url("http://127.0.0.1:39281/v1/models"));
        assert!(!is_allowed_url("http://example.com"));
        assert!(!is_allowed_url("file:///C:/Windows"));
        assert!(!is_allowed_url("javascript:alert(1)"));
        assert!(!is_allowed_url("not a url"));
    }

    #[test]
    fn endpoints_tolerate_base_url_variants() {
        let plain = DeepSeekToolsSearcher::new("https://api.deepseek.com", None);
        assert_eq!(
            plain.search_url(),
            "https://api.deepseek.com/v1/tools/search"
        );
        assert_eq!(plain.fetch_url(), "https://api.deepseek.com/v1/tools/fetch");

        let with_v1 = DeepSeekToolsSearcher::new("https://api.deepseek.com/v1/", None);
        assert_eq!(
            with_v1.search_url(),
            "https://api.deepseek.com/v1/tools/search"
        );

        let with_tools = DeepSeekToolsSearcher::new("https://api.deepseek.com/v1/tools", None);
        assert_eq!(
            with_tools.fetch_url(),
            "https://api.deepseek.com/v1/tools/fetch"
        );
    }

    #[test]
    fn search_body_maps_limit_to_count_and_truncates_query() {
        let body = DeepSeekToolsSearcher::search_body("  rust 异步  ", 0, false, None);
        assert_eq!(body["query"], "rust 异步");
        assert_eq!(body["count"], 1);
        assert_eq!(body["need_content"], false);
        assert!(body.get("ori_question").is_none());

        let body = DeepSeekToolsSearcher::search_body("q", 500, true, Some("原始问题"));
        assert_eq!(body["count"], 50);
        assert_eq!(body["need_content"], true);
        assert_eq!(body["ori_question"], "原始问题");

        let long = "汉".repeat(200);
        let body = DeepSeekToolsSearcher::search_body(&long, 10, false, None);
        assert_eq!(
            body["query"].as_str().unwrap().chars().count(),
            QUERY_MAX_CHARS
        );
    }

    #[test]
    fn fetch_body_has_only_url() {
        let body = DeepSeekToolsSearcher::fetch_body(" https://example.com/a ");
        assert_eq!(body["url"], "https://example.com/a");
        assert_eq!(body.as_object().unwrap().len(), 1);
    }

    #[test]
    fn parses_search_response_into_hits_and_sources() {
        let body = r#"{
          "results": [
            {"title":"DeepSeek API","url":"https://api-docs.deepseek.com/","site_name":"DeepSeek",
             "summary":"Search API 说明","snippet":"网页搜索与抓取","content":"","date":null,
             "authority":3,"rerank_score":2.5,"site_icon":"https://x/icon.png"},
            {"title":"","url":"https://example.com/no-title","site_name":null,"summary":"只有摘要" },
            {"title":"bad-without-url"},
            {"url":"  "}
          ]
        }"#;
        let hits = DeepSeekToolsSearcher::parse_search_response(body).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://api-docs.deepseek.com/");
        assert_eq!(hits[0].site_name.as_deref(), Some("DeepSeek"));
        assert_eq!(hits[0].authority, Some(3));
        assert_eq!(hits[0].rerank_score, Some(2.5));
        // 原始 title 保持协议原样（空串），映射为 SourceRef 时回退 URL；snippet 缺失回退 summary。
        assert_eq!(hits[1].title, "");
        assert_eq!(
            hits[1].to_source_ref().title,
            "https://example.com/no-title"
        );
        assert_eq!(hits[1].to_source_ref().snippet.as_deref(), Some("只有摘要"));
    }

    #[test]
    fn empty_results_are_valid() {
        let hits = DeepSeekToolsSearcher::parse_search_response(r#"{"results":[]}"#).unwrap();
        assert!(hits.is_empty());
        // 缺 results 数组 → 明确错误（不当作空命中）。
        let error = DeepSeekToolsSearcher::parse_search_response(r#"{"foo":1}"#).unwrap_err();
        assert_eq!(error.code, "SA-1099");
        let error = DeepSeekToolsSearcher::parse_search_response("<html>502</html>").unwrap_err();
        assert_eq!(error.code, "SA-1099");
    }

    #[test]
    fn parses_fetch_and_truncates_client_side() {
        let page = DeepSeekToolsSearcher::parse_fetch_response(
            r##"{"title":"T","content":"# 标题\n\n正文内容"}"##,
            "https://example.com",
            100,
        )
        .unwrap();
        assert_eq!(page.title, "T");
        assert_eq!(page.url, "https://example.com");
        assert!(page.content.contains("正文内容"));
        assert!(!page.truncated);

        let page = DeepSeekToolsSearcher::parse_fetch_response(
            r#"{"title":"","content":"abcdefghij"}"#,
            "u",
            3,
        )
        .unwrap();
        assert_eq!(page.content, "abc");
        assert!(page.truncated);

        // 空正文 → 明确错误（站点拒绝/不可抽取）。
        let error =
            DeepSeekToolsSearcher::parse_fetch_response(r#"{"title":"T","content":"  "}"#, "u", 10)
                .unwrap_err();
        assert_eq!(error.code, "SA-1099");
    }

    #[test]
    fn http_errors_map_to_contract_codes() {
        for (status, code) in [
            (401, "SA-3002"),
            (403, "SA-3002"),
            (402, "SA-3003"),
            (429, "SA-3004"),
            (408, "SA-3005"),
            (500, "SA-3001"),
            (503, "SA-3001"),
            (400, "SA-5001"),
            (404, "SA-5001"),
        ] {
            let error = DeepSeekToolsSearcher::map_http_error(status, "boom", "Search");
            assert_eq!(error.code, code, "status={status}");
            assert!(error.detail.is_some());
        }
        // 错误 detail 截断，不把整段响应塞进 UI。
        let error = DeepSeekToolsSearcher::map_http_error(500, &"x".repeat(10_000), "Fetch");
        assert!(error.detail.unwrap().chars().count() <= ERROR_BODY_LIMIT + 100);
    }

    #[test]
    fn search_fetch_cost_uses_pricing_json() {
        // pricing.json: deepseek-search/deepseek-fetch = 0.0053 元/次。
        let cost = search_fetch_cost(2, 1);
        assert!((cost - 0.0159).abs() < 1e-6, "cost={cost}");
        assert_eq!(search_fetch_cost(0, 0), 0.0);
    }

    #[test]
    fn normalize_query_is_utf8_safe() {
        let query = "汉".repeat(200);
        let normalized = normalize_query(&query);
        assert_eq!(normalized.chars().count(), QUERY_MAX_CHARS);
        assert_eq!(normalize_query("  a  "), "a");
    }
}
