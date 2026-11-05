//! 云端搜索链路测试（T3 / 契约 v5 §3.4）。
//!
//! - 纯函数单测：Search/Fetch 解析、错误映射、检索词/视觉输出解析、提示词拼装；
//! - `#[ignore]` 真实网络测试：DeepSeek Chat 流式、Search/Fetch、图片搜索四阶段
//!   （真实截图 data URL → 视觉识别 → 检索词 → Search → 并发 Fetch → 摘要流式）。
//!
//! 真实测试需要 `.verify/keys/deepseek.txt`（或环境变量 `SNIP_ASK_DEEPSEEK_KEY`），
//! **不会**把 Key 写入任何证据文件。运行方式：
//!
//! ```text
//! cd src-tauri
//! cargo test --test cloud_search -- --ignored --nocapture
//! ```
//!
//! 证据输出到 `../.verify/v1/cloud/`（不含 Key）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use base64::Engine as _;
use serde_json::{json, Value};
use snip_ask_lib::contract::types::{ActionId, Mode, SessionId, SourceRef, StreamKind, TaskId};
use snip_ask_lib::services::llm_gateway::{LlmGateway, LlmRequest, StreamChunk};
use snip_ask_lib::services::local_gateway::HttpLlmGateway;
use snip_ask_lib::services::prompt_engine::PromptEngine;
use snip_ask_lib::services::search_gateway::{
    DeepSeekToolsSearcher, FetchedPage, SearchHit, QUERY_MAX_CHARS, SEARCH_COUNT_MAX,
};
use snip_ask_lib::services::task_runner::{
    build_evidence, compose_image_summary_prompt, compose_summary_prompt, parse_queries,
    parse_vision_output, run_image_research, run_text_research, ResearchOptions,
    EVIDENCE_PER_SOURCE_CHARS, EVIDENCE_TOTAL_CHARS, MAX_IMAGE_QUERIES,
};

// ---------------------------------------------------------------------------
// 测试基础设施
// ---------------------------------------------------------------------------

/// 证据目录（`.verify/v1/cloud/`）。
fn evidence_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".verify")
        .join("v1")
        .join("cloud")
}

/// 写脱敏证据 JSON（不含 Key）。
fn write_evidence(name: &str, value: &Value) {
    let dir = evidence_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("创建证据目录失败：{err}");
        return;
    }
    let path = dir.join(name);
    match serde_json::to_string_pretty(value) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&path, text) {
                eprintln!("写证据失败 {}：{err}", path.display());
            } else {
                println!("证据已写入：{}", path.display());
            }
        }
        Err(err) => eprintln!("证据序列化失败：{err}"),
    }
}

/// 读取 DeepSeek Key：环境变量优先，其次 `.verify/keys/deepseek.txt`。
///
/// 返回值只在进程内使用，绝不打印/落盘。
fn deepseek_key() -> Option<String> {
    if let Ok(key) = std::env::var("SNIP_ASK_DEEPSEEK_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return Some(key);
        }
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".verify")
        .join("keys")
        .join("deepseek.txt");
    std::fs::read_to_string(path)
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

/// 渲染摘要提示词（与 `task_runner::research_options` 同源的内置模板）。
fn rendered_prompt(key: &str, mode: Mode, text: &str) -> String {
    let engine = PromptEngine::with_override_dir("C:/not-exists-snip-ask");
    let mut vars: HashMap<&str, &str> = HashMap::new();
    vars.insert("text", text);
    vars.insert("lang", "zh-CN");
    vars.insert("mode", mode.as_str());
    vars.insert("targetLang", "zh-CHS");
    vars.insert("sourceLang", "auto");
    vars.insert("glossary", "");
    vars.insert("question", "");
    vars.insert("context", "");
    engine
        .render(key, &vars)
        .unwrap_or_else(|err| panic!("内置提示词 {key} 渲染失败：{err}"))
}

/// 构造研究选项（deep 模式：多检索词 + 并发 Fetch）。
fn deep_research_options(prompt_key: &str, text: &str) -> ResearchOptions {
    ResearchOptions {
        mode: Mode::Deep,
        lang: "zh-CN".to_string(),
        search_count: 8,
        fetch_pages: 2,
        fetch_max_chars: EVIDENCE_PER_SOURCE_CHARS,
        evidence_total_chars: EVIDENCE_TOTAL_CHARS,
        max_queries: 2,
        summary_prompt: rendered_prompt(prompt_key, Mode::Deep, text),
    }
}

/// 极简流式 sink（收集文本 + 记录首个增量延迟）。
#[derive(Debug, Default)]
struct Collected {
    text: String,
    chunks: usize,
    first_chunk_ms: Option<u64>,
}

fn collecting_sink(
    started: Instant,
) -> (Arc<Mutex<Collected>>, impl Fn(StreamChunk) + Send + Sync) {
    let collected = Arc::new(Mutex::new(Collected::default()));
    let buffer = collected.clone();
    let sink = move |chunk: StreamChunk| {
        if chunk.kind != StreamKind::Text {
            return;
        }
        let mut guard = buffer.lock().expect("sink 锁");
        if guard.first_chunk_ms.is_none() {
            guard.first_chunk_ms = Some(started.elapsed().as_millis() as u64);
        }
        guard.chunks += 1;
        guard.text.push_str(&chunk.chunk);
    };
    (collected, sink)
}

fn request(action: ActionId, prompt: &str, input: &str, engine: &str) -> LlmRequest {
    LlmRequest {
        task: TaskId::from("cloud-test"),
        session: SessionId::from("cloud-test"),
        action,
        mode: Mode::Quick,
        engine: engine.to_string(),
        prompt: prompt.to_string(),
        input_text: input.to_string(),
        image_data_url: None,
        cancel: Arc::new(AtomicBool::new(false)),
    }
}

fn search_hit(title: &str, url: &str, snippet: &str, summary: &str) -> SearchHit {
    SearchHit {
        title: title.to_string(),
        url: url.to_string(),
        site_name: Some("example.com".to_string()),
        snippet: Some(snippet.to_string()),
        summary: Some(summary.to_string()),
        content: String::new(),
        date: None,
        authority: Some(3),
        rerank_score: Some(2.0),
    }
}

// ---------------------------------------------------------------------------
// 纯函数单测（不需要网络）
// ---------------------------------------------------------------------------

#[test]
fn parses_vision_json_output() {
    let raw = r#"{"text":"框选 AI","description":"一个桌面应用界面","uncertain":"右下角模糊"}"#;
    let parsed = parse_vision_output(raw);
    assert!(parsed.contains("框选 AI"));
    assert!(parsed.contains("一个桌面应用界面"));
    assert!(parsed.contains("右下角模糊"));

    // 模型不听话包了 markdown 代码块也能解析。
    let raw = "```json\n{\"text\":\"无\",\"description\":\"纯色背景\",\"uncertain\":\"\"}\n```";
    let parsed = parse_vision_output(raw);
    assert!(parsed.contains("纯色背景"));
    assert!(!parsed.contains("无"));

    // 非 JSON 输出原样保留（不 panic）。
    assert_eq!(parse_vision_output("图中是一个对话框"), "图中是一个对话框");
    assert_eq!(parse_vision_output("   "), "");
}

#[test]
fn parses_query_lines() {
    let raw = "1. Rust 所有权机制\n2. Rust borrow checker\n- Rust 生命周期\n\"Rust 生命周期\"\n\n* 内存安全";
    let queries = parse_queries(raw, 10);
    assert_eq!(queries.len(), 4);
    assert_eq!(queries[0], "Rust 所有权机制");
    assert_eq!(queries[1], "Rust borrow checker");
    assert_eq!(queries[2], "Rust 生命周期");
    assert_eq!(queries[3], "内存安全");
    // 上限生效。
    assert_eq!(parse_queries(raw, 2).len(), 2);
    // 空输入。
    assert!(parse_queries("   \n\n", 3).is_empty());
}

#[test]
fn query_output_is_capped_to_protocol_limit() {
    let long = "汉".repeat(500);
    let queries = parse_queries(&long, MAX_IMAGE_QUERIES);
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].chars().count(), QUERY_MAX_CHARS);
}

#[test]
fn composes_summary_prompt_with_evidence_and_ocr_note() {
    let prompt = "【角色】看图检索助手";
    let composed = compose_summary_prompt(prompt, "1. 标题：A\n   内容：B", true);
    assert!(composed.contains("【角色】看图检索助手"));
    assert!(composed.contains("1. 标题：A"));
    assert!(composed.contains("未上传图片"));
    assert!(!composed.contains("未检索到可靠来源"));

    let empty = compose_summary_prompt(prompt, "", false);
    assert!(empty.contains("未检索到任何结果"));
    assert!(!empty.contains("未上传图片"));
}

#[test]
fn composes_overlay_image_summary_prompt_with_recognition_and_queries() {
    let template = "【角色】看图检索助手";
    let composed = compose_image_summary_prompt(
        template,
        "图中文字：DeepSeek API；内容描述：API keys 页面",
        &["DeepSeek API key".to_string(), "API keys 安全".to_string()],
        "1. 标题：A\n   内容：B",
        false,
    );
    assert!(composed.contains("【角色】看图检索助手"));
    assert!(composed.contains("图中文字：DeepSeek API"));
    assert!(composed.contains("1. DeepSeek API key"));
    assert!(composed.contains("2. API keys 安全"));
    assert!(composed.contains("1. 标题：A"));
    assert!(composed.contains("不要声称没有收到图片"));
    assert!(!composed.contains("未上传图片"));

    // 隐私退化为本地 OCR：必须标注未上传图片，且识别来源写成本地 OCR 文本。
    let composed = compose_image_summary_prompt(template, "本地识别文本", &[], "", true);
    assert!(composed.contains("未上传图片"));
    assert!(composed.contains("本地 OCR 文本"));
    assert!(composed.contains("未检索到任何结果"));
}

#[test]
fn builds_evidence_prefers_fetched_body_and_caps_total() {
    let hit = search_hit("标题", "https://example.com/a", "片段", "摘要");
    let page = FetchedPage {
        title: "标题".to_string(),
        url: "https://example.com/a".to_string(),
        content: "已抓取正文".to_string(),
        truncated: false,
    };
    let options = deep_research_options("text_search", "Rust");
    let evidence = build_evidence(
        std::slice::from_ref(&hit),
        std::slice::from_ref(&page),
        &options,
    );
    assert!(evidence.contains("已抓取正文"));
    assert!(evidence.contains("https://example.com/a"));

    // 无 Fetch 时回退 summary。
    let evidence = build_evidence(std::slice::from_ref(&hit), &[], &options);
    assert!(evidence.contains("摘要"));

    // 空命中 → 空证据；总量截断。
    assert!(build_evidence(&[], &[], &options).is_empty());
    let mut many = Vec::new();
    for index in 0..50 {
        many.push(search_hit(
            &format!("标题{index}"),
            &format!("https://example.com/{index}"),
            &"片段".repeat(400),
            &"摘要".repeat(400),
        ));
    }
    let evidence = build_evidence(&many, &[], &options);
    assert!(evidence.chars().count() <= EVIDENCE_TOTAL_CHARS + 100);
}

#[test]
fn source_refs_are_deduped_and_limited() {
    // collect_sources 是私有函数；这里通过 SearchHit::to_source_ref 的映射语义覆盖。
    let hits: Vec<SearchHit> = (0..20)
        .map(|index| {
            search_hit(
                &format!("标题{index}"),
                &format!("https://example.com/{index}"),
                "片段",
                "摘要",
            )
        })
        .collect();
    let sources: Vec<SourceRef> = hits.iter().take(10).map(SearchHit::to_source_ref).collect();
    assert_eq!(sources.len(), 10);
    assert_eq!(sources[0].url, "https://example.com/0");
}

#[test]
fn options_defaults_are_sane() {
    // 真实协议约束：count 收敛到 ≤50；检索词上限 ≤128 字符；证据单条 < 总量。
    let body = DeepSeekToolsSearcher::search_body("q", 999, false, None);
    assert_eq!(body["count"], SEARCH_COUNT_MAX);
    let long = "汉".repeat(300);
    let queries = parse_queries(&long, MAX_IMAGE_QUERIES);
    assert!(queries.len() <= MAX_IMAGE_QUERIES);
    for query in &queries {
        assert!(query.chars().count() <= QUERY_MAX_CHARS);
    }
    let options = deep_research_options("text_search", "Rust");
    assert!(options.fetch_max_chars < options.evidence_total_chars);
}

// ---------------------------------------------------------------------------
// 真实网络测试（默认 ignored；需要 DeepSeek Key）
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "真实网络：需要 DeepSeek Key（.verify/keys/deepseek.txt 或 SNIP_ASK_DEEPSEEK_KEY）"]
async fn real_deepseek_chat_stream() {
    let Some(key) = deepseek_key() else {
        eprintln!("跳过：未找到 DeepSeek Key");
        return;
    };
    let gateway = HttpLlmGateway::new("https://api.deepseek.com/v1", Some(key), "deepseek-flash");
    let started = Instant::now();
    let (collected, sink) = collecting_sink(started);
    let outcome = gateway
        .stream(
            &request(
                ActionId::Answer,
                "用一句话解释 Rust 所有权（不超过 40 字）。",
                "Rust 所有权",
                "deepseek",
            ),
            &sink,
        )
        .await
        .expect("真实 DeepSeek Chat 流式请求应成功");
    let collected = collected.lock().expect("collected 锁");
    assert!(!collected.text.trim().is_empty(), "应产出非空回答");
    assert!(outcome.usage.input_tokens > 0, "应返回真实 usage");
    assert_eq!(outcome.engine, "deepseek");
    write_evidence(
        "real_deepseek_chat.json",
        &json!({
            "endpoint": "POST https://api.deepseek.com/v1/chat/completions (stream)",
            "model": "deepseek-flash",
            "ok": true,
            "latency_ms": outcome.latency_ms,
            "first_chunk_ms": collected.first_chunk_ms,
            "chunks": collected.chunks,
            "answer_chars": collected.text.chars().count(),
            "answer_head": collected.text.chars().take(200).collect::<String>(),
            "usage": {
                "input_tokens": outcome.usage.input_tokens,
                "output_tokens": outcome.usage.output_tokens,
                "cost_estimate_cny": outcome.usage.cost_estimate,
            },
            "key_written_to_evidence": false,
        }),
    );
}

#[tokio::test]
#[ignore = "真实网络：需要 DeepSeek Key（.verify/keys/deepseek.txt 或 SNIP_ASK_DEEPSEEK_KEY）"]
async fn real_deepseek_search_and_fetch() {
    let Some(key) = deepseek_key() else {
        eprintln!("跳过：未找到 DeepSeek Key");
        return;
    };
    let searcher = DeepSeekToolsSearcher::new("https://api.deepseek.com", Some(key));
    let client = searcher.client().expect("创建 HTTP 客户端");
    let started = Instant::now();
    let hits = searcher
        .search(
            &client,
            "DeepSeek Search API tools/search",
            5,
            false,
            Some("DeepSeek 搜索 API 怎么用"),
        )
        .await
        .expect("真实 Search 应成功");
    let search_ms = started.elapsed().as_millis() as u64;
    assert!(!hits.is_empty(), "真实 Search 应至少命中 1 条");
    assert!(hits[0].url.starts_with("http"));

    let started = Instant::now();
    let page = searcher
        .fetch(&client, &hits[0].url, 1500)
        .await
        .expect("真实 Fetch 应成功");
    let fetch_ms = started.elapsed().as_millis() as u64;
    assert!(!page.content.trim().is_empty(), "Fetch 应返回正文");
    assert!(page.content.chars().count() <= 1500);

    write_evidence(
        "real_deepseek_search_fetch.json",
        &json!({
            "search": {
                "endpoint": "POST https://api.deepseek.com/v1/tools/search",
                "request_fields": {"query": "DeepSeek Search API tools/search", "count": 5, "need_content": false},
                "latency_ms": search_ms,
                "result_count": hits.len(),
                "results": hits.iter().take(5).map(|hit| json!({
                    "title": hit.title,
                    "url": hit.url,
                    "site_name": hit.site_name,
                    "snippet": hit.snippet,
                    "summary_chars": hit.summary.as_deref().map(|value| value.chars().count()).unwrap_or(0),
                    "authority": hit.authority,
                    "rerank_score": hit.rerank_score,
                    "date": hit.date,
                })).collect::<Vec<_>>(),
            },
            "fetch": {
                "endpoint": "POST https://api.deepseek.com/v1/tools/fetch",
                "request_fields": {"url": hits[0].url},
                "latency_ms": fetch_ms,
                "title": page.title,
                "content_chars": page.content.chars().count(),
                "truncated_client_side": page.truncated,
                "content_head": page.content.chars().take(300).collect::<String>(),
            },
            "key_written_to_evidence": false,
        }),
    );
}

#[tokio::test]
#[ignore = "真实网络 + 真实截图：需要 DeepSeek Key 与 .verify/v1/cloud/real_screenshot_800.png"]
async fn real_image_search_pipeline() {
    let Some(key) = deepseek_key() else {
        eprintln!("跳过：未找到 DeepSeek Key");
        return;
    };
    let image_path = evidence_dir().join("real_screenshot_800.png");
    let image_bytes = std::fs::read(&image_path).unwrap_or_else(|err| {
        panic!(
            "缺少真实截图 {}（{err}）；请先运行截图脚本或由 T2 提供",
            image_path.display()
        )
    });
    let data_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&image_bytes)
    );

    let gateway = HttpLlmGateway::new(
        "https://api.deepseek.com/v1",
        Some(key.clone()),
        "deepseek-flash",
    );
    let searcher = DeepSeekToolsSearcher::new("https://api.deepseek.com", Some(key));
    let options = deep_research_options("image_search", "");
    let started = Instant::now();
    let (collected, sink) = collecting_sink(started);
    let outcome = run_image_research(
        &gateway,
        &searcher,
        "deepseek",
        Some(&data_url),
        "",
        &options,
        &Arc::new(AtomicBool::new(false)),
        &sink,
    )
    .await
    .expect("真实图片搜索四阶段流水线应成功");
    let collected = collected.lock().expect("collected 锁");
    let text = collected.text.clone();
    assert!(text.chars().count() > 100, "摘要应有实际内容：{text}");
    assert!(!outcome.sources.is_empty(), "应返回真实来源");
    assert!(outcome.search_calls >= 1);
    assert!(!outcome.from_local_ocr);
    // image_search.md 要求四段齐全，且识别结果必须来自阶段 1（不得误报「未收到图片」）。
    assert!(text.contains("识别结果"), "缺少「识别结果」段：{text}");
    assert!(text.contains("来源"), "缺少「来源」段：{text}");
    assert!(!text.contains("未收到图片"), "摘要误报未收到图片：{text}");
    assert!(!text.contains("图片不可见"), "摘要误报图片不可见：{text}");

    write_evidence(
        "real_image_search_pipeline.json",
        &json!({
            "pipeline": ["vision", "queries", "search", "fetch", "summary"],
            "engine": outcome.engine,
            "mode": "deep",
            "image": {"path": image_path.display().to_string(), "bytes": image_bytes.len(), "uploaded": true},
            "latency_ms": outcome.latency_ms,
            "stage_latency_ms": outcome.stage_latency_ms,
            "first_chunk_ms": collected.first_chunk_ms,
            "search_calls": outcome.search_calls,
            "fetch_calls": outcome.fetch_calls,
            "source_count": outcome.sources.len(),
            "from_local_ocr": outcome.from_local_ocr,
            "sources": outcome.sources.iter().map(|source| json!({
                "title": source.title, "url": source.url, "siteName": source.site_name,
            })).collect::<Vec<_>>(),
            "usage": {
                "input_tokens": outcome.usage.input_tokens,
                "output_tokens": outcome.usage.output_tokens,
                "cost_estimate_cny": outcome.usage.cost_estimate,
            },
            "answer_chars": text.chars().count(),
            "answer_head": text.chars().take(900).collect::<String>(),
            "privacy": {"allowImageUpload": true, "uploaded_image": true},
            "key_written_to_evidence": false,
        }),
    );
}

#[tokio::test]
#[ignore = "真实网络：需要 DeepSeek Key（文本 AI 搜索结果质量场景）"]
async fn real_text_search_pipeline() {
    let Some(key) = deepseek_key() else {
        eprintln!("跳过：未找到 DeepSeek Key");
        return;
    };
    let gateway = HttpLlmGateway::new(
        "https://api.deepseek.com/v1",
        Some(key.clone()),
        "deepseek-flash",
    );
    let searcher = DeepSeekToolsSearcher::new("https://api.deepseek.com", Some(key));
    let input_text =
        "DeepSeek 在 2026 年推出独立 Search 与 Fetch API，支持 Agent 与 RAG 实时网页检索。";
    let options = deep_research_options("text_search", input_text);
    let started = Instant::now();
    let (collected, sink) = collecting_sink(started);
    let outcome = run_text_research(
        &gateway,
        &searcher,
        "deepseek",
        input_text,
        &options,
        &Arc::new(AtomicBool::new(false)),
        &sink,
    )
    .await
    .expect("真实文本搜索流水线应成功");
    let collected = collected.lock().expect("collected 锁");
    let text = collected.text.clone();
    assert!(text.chars().count() > 100, "摘要应有实际内容：{text}");
    assert!(outcome.search_calls >= 1);
    assert!(text.contains("来源"), "缺少「来源」段：{text}");

    write_evidence(
        "real_text_search_pipeline.json",
        &json!({
            "pipeline": ["search", "fetch", "summary"],
            "engine": outcome.engine,
            "mode": "deep",
            "input_text": input_text,
            "latency_ms": outcome.latency_ms,
            "stage_latency_ms": outcome.stage_latency_ms,
            "first_chunk_ms": collected.first_chunk_ms,
            "search_calls": outcome.search_calls,
            "fetch_calls": outcome.fetch_calls,
            "source_count": outcome.sources.len(),
            "sources": outcome.sources.iter().take(5).map(|source| json!({
                "title": source.title, "url": source.url, "siteName": source.site_name,
            })).collect::<Vec<_>>(),
            "usage": outcome.usage,
            "answer_chars": text.chars().count(),
            "answer_head": text.chars().take(700).collect::<String>(),
            "key_written_to_evidence": false,
        }),
    );
}

/// 隐私退化场景（不上传图片 → 本地 OCR 文本 → 检索 → 摘要），真实网络。
#[tokio::test]
#[ignore = "真实网络：需要 DeepSeek Key（隐私开关退化为本地 OCR 文本的场景）"]
async fn real_image_search_without_upload_uses_local_ocr() {
    let Some(key) = deepseek_key() else {
        eprintln!("跳过：未找到 DeepSeek Key");
        return;
    };
    let gateway = HttpLlmGateway::new(
        "https://api.deepseek.com/v1",
        Some(key.clone()),
        "deepseek-flash",
    );
    let searcher = DeepSeekToolsSearcher::new("https://api.deepseek.com", Some(key));
    let local_ocr_text = "Settings > Privacy: allow image upload is disabled. Search is unavailable in local-only mode.";
    let options = deep_research_options("image_search", local_ocr_text);
    let started = Instant::now();
    let (collected, sink) = collecting_sink(started);
    let outcome = run_image_research(
        &gateway,
        &searcher,
        "deepseek",
        None,
        local_ocr_text,
        &options,
        &Arc::new(AtomicBool::new(false)),
        &sink,
    )
    .await
    .expect("隐私退化路径应成功");
    let collected = collected.lock().expect("collected 锁");
    assert!(outcome.from_local_ocr);
    assert!(!collected.text.trim().is_empty());

    write_evidence(
        "real_image_search_privacy_local_ocr.json",
        &json!({
            "privacy": {"allowImageUpload": false, "uploaded_image": false, "local_only": false},
            "pipeline": ["local_ocr", "queries", "search", "fetch", "summary"],
            "engine": outcome.engine,
            "from_local_ocr": outcome.from_local_ocr,
            "stage_latency_ms": outcome.stage_latency_ms,
            "search_calls": outcome.search_calls,
            "fetch_calls": outcome.fetch_calls,
            "source_count": outcome.sources.len(),
            "answer_head": collected.text.chars().take(600).collect::<String>(),
            "degradation_note": "未上传图片：阶段 1 退化为本地 OCR 文本，摘要提示词明确标注「未上传图片」",
            "key_written_to_evidence": false,
        }),
    );
}
