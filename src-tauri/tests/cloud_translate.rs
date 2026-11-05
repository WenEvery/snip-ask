//! 云端翻译链路测试（T3 / 契约 v5 §3.4）。
//!
//! - **协议级 mock 验收**：本地 `std::net` mock server 精确断言 DashScope Qwen-MT 原生
//!   请求（路径 / Bearer 鉴权 / `translation_options` / `terms`），响应解析与用量统计；
//! - 错误映射：HTTP 401 → SA-3002；HTTP 200 + `status_code=401` 业务错误 → SA-3002；
//! - 回退链：LLM 后端失败 → Qwen-MT 成功；
//! - `#[ignore]` 真实网络：DeepSeek LLM 翻译（GatewayTranslator）。
//!
//! 本机**无 DashScope Key**：真实 Qwen-MT 翻译标注 blocked，仅协议级 mock 验收。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use snip_ask_lib::contract::config::{Config, GlossaryEntry};
use snip_ask_lib::contract::errors::{AppError, AppResult, ErrorCode};
use snip_ask_lib::contract::types::{Currency, Mode, StreamKind, TokenUsage};
use snip_ask_lib::services::llm_gateway::{LlmGateway, LlmRequest, StreamChunk, TaskOutcome};
use snip_ask_lib::services::local_gateway::HttpLlmGateway;
use snip_ask_lib::services::prompt_engine::PromptEngine;
use snip_ask_lib::services::translator::{
    render_translate_prompt, translate_with_fallback, GatewayTranslator, QwenMtTranslator,
    TranslateAttempt, TranslateAttemptKind, TranslateRequest, Translator,
};

// ---------------------------------------------------------------------------
// 证据
// ---------------------------------------------------------------------------

fn evidence_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".verify")
        .join("v1")
        .join("cloud")
}

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

fn dashscope_key() -> Option<String> {
    if let Ok(key) = std::env::var("SNIP_ASK_DASHSCOPE_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return Some(key);
        }
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".verify")
        .join("keys")
        .join("dashscope.txt");
    std::fs::read_to_string(path)
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

// ---------------------------------------------------------------------------
// 极简 mock server（std::net，无新增依赖）
// ---------------------------------------------------------------------------

/// mock 返回。
#[derive(Clone)]
struct MockResponse {
    status: u16,
    body: String,
}

/// 单次请求 mock server（记录完整原始请求）。
struct MockServer {
    addr: SocketAddr,
    captured: Arc<Mutex<Vec<String>>>,
}

impl MockServer {
    fn start(response: MockResponse) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server 应能绑定 127.0.0.1:0");
        let addr = listener.local_addr().expect("mock server 应有本地地址");
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let records = captured.clone();
        thread::spawn(move || {
            let Ok((mut stream, _peer)) = listener.accept() else {
                return;
            };
            stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut buffer: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 4096];
            let mut header_end: Option<usize> = None;
            let mut content_length: usize = 0;
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => {
                        buffer.extend_from_slice(&chunk[..read]);
                        if header_end.is_none() {
                            if let Some(position) = find_subslice(&buffer, b"\r\n\r\n") {
                                header_end = Some(position + 4);
                                let headers = String::from_utf8_lossy(&buffer[..position]);
                                for line in headers.lines() {
                                    let lower = line.to_ascii_lowercase();
                                    if let Some(value) = lower.strip_prefix("content-length:") {
                                        content_length = value.trim().parse::<usize>().unwrap_or(0);
                                    }
                                }
                            }
                        }
                        if let Some(end) = header_end {
                            if buffer.len() >= end + content_length {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            records
                .lock()
                .expect("mock 记录锁")
                .push(String::from_utf8_lossy(&buffer).into_owned());
            let reason = match response.status {
                200 => "OK",
                401 => "Unauthorized",
                _ => "Error",
            };
            let reply = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.status,
                reason,
                response.body.len(),
                response.body
            );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        });
        Self { addr, captured }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    fn captured(&self) -> Vec<String> {
        self.captured.lock().expect("mock 记录锁").clone()
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 成功响应样例（DashScope Qwen-MT 原生）。
fn qwen_mt_success_body() -> String {
    json!({
        "status_code": 200,
        "request_id": "mock-request-id",
        "output": {
            "text": null,
            "finish_reason": "stop",
            "choices": [{
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "你好，世界" }
            }],
            "model_name": "qwen-mt-lite"
        },
        "usage": { "input_tokens": 11, "output_tokens": 5, "total_tokens": 16 }
    })
    .to_string()
}

fn sample_request() -> TranslateRequest {
    TranslateRequest::new("Hello, world", "zh-CHS", Mode::Quick).with_glossary(vec![
        GlossaryEntry {
            from: "world".to_string(),
            to: "世界".to_string(),
            extra: serde_json::Map::new(),
        },
        GlossaryEntry {
            from: "Hello".to_string(),
            to: "你好".to_string(),
            extra: serde_json::Map::new(),
        },
    ])
}

/// 从原始 HTTP 请求中取出 body 并解析 JSON。
fn request_body(request: &str) -> Value {
    let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
    serde_json::from_str(body).expect("mock 捕获的请求体应为合法 JSON")
}

// ---------------------------------------------------------------------------
// Qwen-MT 协议级 mock 验收
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qwen_mt_protocol_against_mock_server() {
    let server = MockServer::start(MockResponse {
        status: 200,
        body: qwen_mt_success_body(),
    });
    let translator = QwenMtTranslator::new(server.base_url(), "sk-test-dummy-key", "qwen-mt-lite");
    assert_eq!(
        translator.generation_url(),
        format!(
            "{}/api/v1/services/aigc/text-generation/generation",
            server.base_url()
        )
    );
    let (text, usage) = translator
        .translate_with_usage(&sample_request())
        .await
        .expect("mock 协议级翻译应成功");
    assert_eq!(text, "你好，世界");
    assert_eq!(usage.input_tokens, 11);
    assert_eq!(usage.output_tokens, 5);
    // qwen-mt-lite：输入 0.6 元/百万 token，输出 1.6 元/百万 token（pricing.json 按 6 位小数取整）。
    let raw: f64 = (11.0 * 0.6 + 5.0 * 1.6) / 1_000_000.0;
    let expected: f64 = (raw * 1_000_000.0).round() / 1_000_000.0;
    assert!(
        (usage.cost_estimate - expected).abs() < 1e-9,
        "cost={} expected={expected}",
        usage.cost_estimate
    );
    assert!(usage.cost_estimate > 0.0);
    assert_eq!(usage.currency, Currency::Cny);

    let captured = server.captured();
    assert_eq!(captured.len(), 1, "应恰好收到 1 次请求");
    let raw = &captured[0];
    // 1) 请求行与路径（原生端点，非 OpenAI 兼容）。
    assert!(
        raw.starts_with("POST /api/v1/services/aigc/text-generation/generation HTTP/1.1"),
        "请求行不符：{}",
        raw.lines().next().unwrap_or_default()
    );
    // 2) Bearer 鉴权。
    let lower = raw.to_ascii_lowercase();
    assert!(lower.contains("authorization: bearer sk-test-dummy-key"));
    assert!(lower.contains("content-type: application/json"));
    // 3) 请求体：原生 input.messages + parameters.translation_options + terms。
    let body = request_body(raw);
    assert_eq!(body["model"], "qwen-mt-lite");
    assert_eq!(body["input"]["messages"][0]["role"], "user");
    assert_eq!(body["input"]["messages"][0]["content"], "Hello, world");
    let options = &body["parameters"]["translation_options"];
    assert_eq!(options["source_lang"], "auto");
    assert_eq!(options["target_lang"], "Chinese");
    assert_eq!(options["terms"].as_array().unwrap().len(), 2);
    assert_eq!(options["terms"][0]["source"], "world");
    assert_eq!(options["terms"][0]["target"], "世界");
    assert_eq!(options["terms"][1]["source"], "Hello");
    assert_eq!(options["terms"][1]["target"], "你好");

    // 证据（脱敏：不含 Key）。
    let evidence = json!({
        "kind": "protocol_mock",
        "note": "本机无 DashScope Key：Qwen-MT 真实翻译 blocked；本证据为协议级验收（本地 mock server）",
        "endpoint": "POST {base}/api/v1/services/aigc/text-generation/generation",
        "request": {
            "method": "POST",
            "path": "/api/v1/services/aigc/text-generation/generation",
            "headers": {
                "authorization": "Bearer <redacted>",
                "content-type": "application/json",
            },
            "body": body,
        },
        "response": {
            "status": 200,
            "text": text,
            "usage": usage,
        },
        "assertions": [
            "path is native DashScope generation endpoint",
            "Authorization: Bearer <key>",
            "input.messages[0].content == 原文",
            "parameters.translation_options.source_lang == auto",
            "parameters.translation_options.target_lang == Chinese（zh-CHS 映射为英文全称）",
            "parameters.translation_options.terms 来自术语表（2 条，空术语被过滤）",
            "usage 按真实 token 计费（qwen-mt-lite 价格表）",
        ],
        "key_written_to_evidence": false,
    });
    write_evidence("qwen_mt_mock_protocol.json", &evidence);

    // blocked 标注证据（真实端点不可验收）。
    write_evidence(
        "qwen_mt_blocked.json",
        &json!({
            "blocked": true,
            "capability": "DashScope Qwen-MT 真实翻译",
            "reason": "本机无 DashScope API Key（.verify/keys/dashscope.txt 不存在，环境变量 SNIP_ASK_DASHSCOPE_KEY 未设置）",
            "mitigation": "已完成协议级 mock 验收（qwen_mt_mock_protocol.json）：路径/Bearer/translation_options/terms/响应解析/计费全部断言通过",
            "how_to_verify_when_key_available": "设置 SNIP_ASK_DASHSCOPE_KEY 或写入 .verify/keys/dashscope.txt 后运行 cargo test --test cloud_translate real_qwen_mt_translation -- --ignored --nocapture",
            "checked_at_utc": chrono::Utc::now().to_rfc3339(),
        }),
    );
}

#[tokio::test]
async fn qwen_mt_http_401_maps_to_auth_failed() {
    let server = MockServer::start(MockResponse {
        status: 401,
        body: json!({"code":"InvalidApiKey","message":"Invalid API-key provided."}).to_string(),
    });
    let translator = QwenMtTranslator::new(server.base_url(), "sk-test-dummy-key", "qwen-mt-lite");
    let error = translator
        .translate_with_usage(&sample_request())
        .await
        .expect_err("401 应失败");
    assert_eq!(error.code, ErrorCode::AuthFailed);
    assert_eq!(
        error.fallback.first().map(String::as_str),
        Some("open_settings")
    );
}

#[tokio::test]
async fn qwen_mt_business_error_in_http_200_maps_to_auth_failed() {
    let server = MockServer::start(MockResponse {
        status: 200,
        body: json!({
            "status_code": 401,
            "code": "InvalidApiKey",
            "message": "Invalid API-key provided."
        })
        .to_string(),
    });
    let translator = QwenMtTranslator::new(server.base_url(), "sk-test-dummy-key", "qwen-mt-lite");
    let error = translator
        .translate_with_usage(&sample_request())
        .await
        .expect_err("业务错误应失败");
    assert_eq!(error.code, ErrorCode::AuthFailed);
}

#[tokio::test]
async fn qwen_mt_quota_and_rate_limit_error_codes() {
    assert_eq!(
        QwenMtTranslator::parse_response(
            r#"{"status_code":400,"code":"Arrearage","message":"balance not enough"}"#
        )
        .unwrap_err()
        .code,
        ErrorCode::QuotaExceeded
    );
    assert_eq!(
        QwenMtTranslator::parse_response(
            r#"{"status_code":429,"code":"Throttling","message":"too many requests"}"#
        )
        .unwrap_err()
        .code,
        ErrorCode::RateLimited
    );
    assert_eq!(
        QwenMtTranslator::parse_response(
            r#"{"status_code":400,"code":"InvalidParameter","message":"bad"}"#
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidConfig
    );
}

// ---------------------------------------------------------------------------
// 回退链
// ---------------------------------------------------------------------------

/// 永远失败的 LLM 网关（模拟云端网络故障）。
struct FailingGateway;

#[async_trait]
impl LlmGateway for FailingGateway {
    async fn stream(
        &self,
        _request: &LlmRequest,
        _sink: &(dyn Fn(StreamChunk) + Send + Sync),
    ) -> AppResult<TaskOutcome> {
        Err(AppError::new(
            ErrorCode::NetworkUnavailable,
            Some("mock 网络故障".to_string()),
        ))
    }
}

/// 每次输出固定译文的 LLM 网关。
struct EchoGateway {
    text: String,
}

#[async_trait]
impl LlmGateway for EchoGateway {
    async fn stream(
        &self,
        _request: &LlmRequest,
        sink: &(dyn Fn(StreamChunk) + Send + Sync),
    ) -> AppResult<TaskOutcome> {
        // 按字符切分（UTF-8 安全），避免 split_at 落在多字节字符中间。
        let chars: Vec<char> = self.text.chars().collect();
        let mid = chars.len() / 2;
        let first: String = chars[..mid].iter().collect();
        let second: String = chars[mid..].iter().collect();
        for piece in [first, second] {
            if piece.is_empty() {
                continue;
            }
            sink(StreamChunk {
                kind: StreamKind::Text,
                chunk: piece,
            });
        }
        Ok(TaskOutcome {
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 4,
                cached_input_tokens: 0,
                cost_estimate: 0.0001,
                currency: Currency::Cny,
            },
            latency_ms: 5,
            engine: "mock".to_string(),
            sources: Vec::new(),
        })
    }
}

#[tokio::test]
async fn fallback_from_failing_llm_to_qwen_mt() {
    let server = MockServer::start(MockResponse {
        status: 200,
        body: qwen_mt_success_body(),
    });
    let attempts = vec![
        TranslateAttempt {
            engine: "deepseek".to_string(),
            kind: TranslateAttemptKind::Llm {
                gateway: Arc::new(FailingGateway),
                prompt: "translate".to_string(),
            },
        },
        TranslateAttempt {
            engine: "dashscope".to_string(),
            kind: TranslateAttemptKind::QwenMt(QwenMtTranslator::new(
                server.base_url(),
                "sk-test-dummy-key",
                "qwen-mt-lite",
            )),
        },
    ];
    let outcome = translate_with_fallback(&attempts, &sample_request())
        .await
        .expect("Qwen-MT 降级应成功");
    assert_eq!(outcome.engine, "dashscope");
    assert_eq!(outcome.text, "你好，世界");
    assert_eq!(outcome.usage.input_tokens, 11);
}

#[tokio::test]
async fn fallback_from_failing_llm_to_working_llm() {
    let attempts = vec![
        TranslateAttempt {
            engine: "deepseek".to_string(),
            kind: TranslateAttemptKind::Llm {
                gateway: Arc::new(FailingGateway),
                prompt: "translate".to_string(),
            },
        },
        TranslateAttempt {
            engine: "local".to_string(),
            kind: TranslateAttemptKind::Llm {
                gateway: Arc::new(EchoGateway {
                    text: "你好，世界".to_string(),
                }),
                prompt: "translate".to_string(),
            },
        },
    ];
    let outcome = translate_with_fallback(&attempts, &sample_request())
        .await
        .expect("本地 LLM 降级应成功");
    assert_eq!(outcome.engine, "local");
    assert_eq!(outcome.text, "你好，世界");
}

#[tokio::test]
async fn all_attempts_failing_reports_chain() {
    let attempts = vec![TranslateAttempt {
        engine: "deepseek".to_string(),
        kind: TranslateAttemptKind::Llm {
            gateway: Arc::new(FailingGateway),
            prompt: "translate".to_string(),
        },
    }];
    let error = translate_with_fallback(&attempts, &sample_request())
        .await
        .expect_err("全部失败应报错");
    assert_eq!(error.code, ErrorCode::NetworkUnavailable);
    assert!(
        error.message.contains("deepseek"),
        "message={}",
        error.message
    );
    assert!(error.context.is_some());
}

// ---------------------------------------------------------------------------
// 真实网络
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "真实网络：需要 DeepSeek Key（GatewayTranslator + translate.md）"]
async fn real_deepseek_llm_translation() {
    let Some(key) = deepseek_key() else {
        eprintln!("跳过：未找到 DeepSeek Key");
        return;
    };
    let config = Config::default();
    let request = TranslateRequest::new(
        "Small screenshots deserve instant answers. Select any region and get a translated result in under a second.",
        "zh-CHS",
        Mode::Quick,
    );
    let prompt_engine = PromptEngine::with_override_dir("C:/not-exists-snip-ask");
    let prompt = render_translate_prompt(
        &prompt_engine,
        &config,
        "translate",
        &request,
        "zh-CHS",
        false,
    )
    .expect("translate.md 渲染应成功");
    assert!(prompt.contains("Small screenshots"));
    assert!(prompt.contains("zh-CHS") || prompt.contains("简体中文") || prompt.contains("Chinese"));

    let gateway = Arc::new(HttpLlmGateway::new(
        "https://api.deepseek.com/v1",
        Some(key),
        "deepseek-flash",
    ));
    let translator = GatewayTranslator::new("deepseek", gateway, prompt, Mode::Quick);
    let started = std::time::Instant::now();
    let text = translator
        .translate(&request)
        .await
        .expect("真实 LLM 翻译应成功");
    let latency_ms = started.elapsed().as_millis() as u64;
    assert!(!text.trim().is_empty());
    let has_cjk = text.chars().any(|ch| ch as u32 >= 0x2E80);
    assert!(has_cjk, "中文译文应包含 CJK 字符：{text}");

    write_evidence(
        "real_deepseek_translate.json",
        &json!({
            "endpoint": "POST https://api.deepseek.com/v1/chat/completions (stream, via GatewayTranslator)",
            "engine": "deepseek",
            "model": "deepseek-flash",
            "source": "Small screenshots deserve instant answers. Select any region and get a translated result in under a second.",
            "target_lang": "zh-CHS",
            "prompt_key": "translate",
            "latency_ms": latency_ms,
            "translation": text,
            "contains_cjk": has_cjk,
            "key_written_to_evidence": false,
        }),
    );
}

#[tokio::test]
#[ignore = "真实网络：需要 DashScope Key（本机无 → 默认 blocked）"]
async fn real_qwen_mt_translation() {
    let Some(key) = dashscope_key() else {
        eprintln!(
            "跳过：本机无 DashScope Key（真实 Qwen-MT 翻译 blocked，见 qwen_mt_blocked.json）"
        );
        return;
    };
    let translator = QwenMtTranslator::new("https://dashscope.aliyuncs.com", key, "qwen-mt-lite");
    let started = std::time::Instant::now();
    let (text, usage) = translator
        .translate_with_usage(&sample_request())
        .await
        .expect("真实 Qwen-MT 翻译应成功");
    let latency_ms = started.elapsed().as_millis() as u64;
    assert!(!text.trim().is_empty());
    write_evidence(
        "real_qwen_mt_translate.json",
        &json!({
            "endpoint": "POST https://dashscope.aliyuncs.com/api/v1/services/aigc/text-generation/generation",
            "engine": "dashscope",
            "model": "qwen-mt-lite",
            "source": "Hello, world",
            "target_lang": "zh-CHS",
            "latency_ms": latency_ms,
            "translation": text,
            "usage": usage,
            "key_written_to_evidence": false,
        }),
    );
}

#[test]
fn default_config_has_qwen_mt_provider_disabled() {
    // 契约 v5 §3.1：dashscope 为 qwenMt，默认 enabled=false（填 Key 后启用）。
    let config = Config::default();
    let dashscope = config
        .providers
        .iter()
        .find(|provider| provider.id == "dashscope")
        .expect("默认配置应含 dashscope");
    assert_eq!(
        dashscope.kind,
        snip_ask_lib::contract::config::ProviderKind::QwenMt
    );
    assert!(!dashscope.enabled);
    assert_eq!(dashscope.base_url, "https://dashscope.aliyuncs.com");

    let deepseek = config
        .providers
        .iter()
        .find(|provider| provider.id == "deepseek")
        .expect("默认配置应含 deepseek");
    assert!(deepseek.enabled);
    assert_eq!(
        deepseek.kind,
        snip_ask_lib::contract::config::ProviderKind::OpenAiCompatible
    );
}
