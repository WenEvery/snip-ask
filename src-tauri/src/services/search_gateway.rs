//! 搜索网关（M10 / 说明书 §4.4 / §4.5）。
//!
//! - `browser` 分支：拼装搜索引擎 URL（`build_search_url` 纯函数），骨架模式只记日志；
//! - `ai` 分支：真实实现为 DeepSeek Search/Fetch（`TODO(real)`），骨架由 `llm_gateway` 输出占位；
//! - `open_external` 的安全白名单：https 全部允许，http 仅 127.0.0.1（契约 §6）。

use async_trait::async_trait;

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{Mode, SourceRef};

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
        let url = build_search_url(
            &request.engine,
            request.template.as_deref(),
            &request.query,
        );
        tracing::info!(
            host = %url::Url::parse(&url).ok().and_then(|parsed| parsed.host_str().map(str::to_string)).unwrap_or_default(),
            "骨架模式：不打开浏览器"
        );
        Ok(SearchOutcome {
            sources: Vec::new(),
            text: format!("骨架模式：将打开浏览器搜索 “{}”", request.query),
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

/// 打开外部链接（骨架模式只记日志）。
///
/// TODO(real): 使用 `tauri_plugin_opener::OpenerExt::opener().open_url(url, None::<&str>)`
/// 打开系统默认浏览器；先经 [`is_allowed_url`] 白名单。
pub async fn open_external(app: Option<&tauri::AppHandle>, url: &str) -> AppResult<()> {
    if !is_allowed_url(url) {
        return Err(AppError::new(
            ErrorCode::UrlRejected,
            Some("仅允许 https:// 或 http://127.0.0.1".to_string()),
        ));
    }
    let host = url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_default();
    // 骨架模式：只打印日志，不真正打开（契约 §12）。
    tracing::info!(host = %host, "骨架模式：open_external 仅记录日志，不打开浏览器");
    let _ = app;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_urls_for_known_engines() {
        assert_eq!(
            build_search_url("bing", None, "框选 AI"),
            format!(
                "https://www.bing.com/search?q={}",
                encode_query("框选 AI")
            )
        );
        assert!(build_search_url("google", None, "rust").starts_with("https://www.google.com/"));
        assert!(build_search_url("baidu", None, "rust").starts_with("https://www.baidu.com/"));
        assert!(build_search_url("duckduckgo", None, "rust")
            .starts_with("https://duckduckgo.com/"));
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
}
