//! S-17 预置：`provider_test` 真实连通性探测的集成测试。
//!
//! 用 `std::net::TcpListener` 起本地 mock HTTP server，覆盖 task-11 要求的场景：
//! 200 / 401 / 402 / 429 / 500 / 超时（不回包）/ 连接拒绝，另测 baseUrl 尾斜杠拼接、
//! 非 https 拒绝、请求头（Authorization / User-Agent）、响应非 JSON、以及「结果不包含 Key」。
//!
//! 真实 DeepSeek 冒烟为 `#[ignore]` 测试，Key 只从环境变量 `SNIPASK_BENCH_API_KEY` 读取：
//! `cargo test --test provider_probe_spike -- --ignored --nocapture`
//! 原始数据写入 `.verify/spikes/S-17/`（gitignored），不写入任何 Key。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use snip_ask_lib::commands::provider::{
    missing_api_key_result, models_url, probe_provider, probe_provider_detail,
    validate_probe_draft, ProbeOptions,
};
use snip_ask_lib::contract::config::{Cap, Config};
use snip_ask_lib::contract::errors::ErrorCode;
use snip_ask_lib::contract::types::ProviderDraft;

/// 测试用假 Key（不是真实凭据；仅用于断言 Authorization 头与「不回显」）。
const TEST_KEY: &str = "sk-test-probe-not-a-real-key";
/// OpenAI 兼容 `/models` 响应样例。
const MODELS_BODY: &str = r#"{"object":"list","data":[{"id":"deepseek-flash","object":"model"},{"id":"deepseek-v4-pro","object":"model"}]}"#;

fn fast_options() -> ProbeOptions {
    ProbeOptions {
        connect_timeout: Duration::from_secs(2),
        request_timeout: Duration::from_secs(3),
    }
}

fn short_timeout_options() -> ProbeOptions {
    ProbeOptions {
        connect_timeout: Duration::from_millis(800),
        request_timeout: Duration::from_millis(300),
    }
}

fn draft_with_base_url(base_url: &str) -> ProviderDraft {
    ProviderDraft {
        id: "deepseek".to_string(),
        name: "DeepSeek".to_string(),
        base_url: base_url.to_string(),
        model: "deepseek-flash".to_string(),
        caps: vec![Cap::Vision, Cap::Search],
        api_key: None,
        temperature: Some(0.2),
        max_tokens: Some(1024),
    }
}

#[derive(Clone, Copy)]
enum MockBehavior {
    Respond {
        status: u16,
        body: &'static str,
    },
    /// 读走请求后不响应（触发客户端超时）。
    Hang,
}

struct MockServer {
    addr: SocketAddr,
    request: Arc<Mutex<Vec<u8>>>,
}

impl MockServer {
    fn start(behavior: MockBehavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server 应能绑定 127.0.0.1:0");
        let addr = listener.local_addr().expect("mock server 应有本地地址");
        let request = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&request);
        thread::spawn(move || {
            let Ok((mut stream, _peer)) = listener.accept() else {
                return;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let bytes = read_request(&mut stream);
            *captured.lock().unwrap() = bytes;
            match behavior {
                MockBehavior::Respond { status, body } => {
                    let response = format!(
                        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        reason_phrase(status),
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
                MockBehavior::Hang => {
                    thread::sleep(Duration::from_secs(3));
                }
            }
        });
        Self { addr, request }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    fn request_text(&self) -> String {
        String::from_utf8_lossy(&self.request.lock().unwrap()).to_string()
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
                if buffer.len() > 16 * 1024 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    buffer
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

/// 断言错误码 + 可重试语义 + 可操作降级动作（不只断言字符串）。
fn assert_error(
    result: &snip_ask_lib::contract::types::TestResult,
    expected_code: &str,
    expected_retryable: bool,
    expected_fallbacks: &[&str],
) {
    assert!(!result.ok, "失败场景 ok 必须为 false");
    let error = result.error.as_ref().expect("失败场景必须带 AppError");
    assert_eq!(error.code, expected_code, "错误码映射不符");
    assert_eq!(error.retryable, expected_retryable, "retryable 语义不符");
    for fallback in expected_fallbacks {
        assert!(
            error.fallback.iter().any(|item| item == fallback),
            "fallback 应包含可操作动作 {fallback}，实际 {:?}",
            error.fallback
        );
    }
}

#[tokio::test]
async fn success_200_returns_ok_with_models_and_sends_auth_headers() {
    let server = MockServer::start(MockBehavior::Respond {
        status: 200,
        body: MODELS_BODY,
    });
    let outcome = probe_provider_detail(
        &server.base_url(),
        TEST_KEY,
        Some("deepseek-flash"),
        fast_options(),
    )
    .await
    .expect("200 场景不应返回 Err");
    let result = &outcome.result;
    assert!(result.ok);
    assert_eq!(result.status, Some(200));
    assert_eq!(result.model.as_deref(), Some("deepseek-flash"));
    assert!(result.latency_ms.is_some(), "成功场景应记录耗时");
    assert!(result.error.is_none());
    assert_eq!(
        outcome.available_models,
        vec!["deepseek-flash".to_string(), "deepseek-v4-pro".to_string()],
        "应解析出 /models 的模型 id 列表"
    );

    let request = server.request_text().to_ascii_lowercase();
    assert!(
        request.starts_with("get /models http/1.1"),
        "应请求 {{baseUrl}}/models"
    );
    assert!(
        request.contains("authorization: bearer "),
        "请求必须带 Authorization: Bearer"
    );
    assert!(request.contains(TEST_KEY), "Authorization 应携带本次 Key");
    assert!(
        request.contains("user-agent: snip-ask/"),
        "请求必须带 User-Agent: snip-ask/<version>"
    );
}

#[tokio::test]
async fn model_falls_back_to_first_available_when_preferred_missing() {
    let server = MockServer::start(MockBehavior::Respond {
        status: 200,
        body: MODELS_BODY,
    });
    let result = probe_provider(
        &server.base_url(),
        TEST_KEY,
        Some("some-unknown-model"),
        fast_options(),
    )
    .await
    .expect("200 场景不应返回 Err");
    assert!(result.ok);
    assert_eq!(
        result.model.as_deref(),
        Some("deepseek-flash"),
        "配置模型不在列表时取列表首项"
    );
}

#[tokio::test]
async fn success_200_with_non_json_body_still_ok() {
    let server = MockServer::start(MockBehavior::Respond {
        status: 200,
        body: "not-json",
    });
    let result = probe_provider(
        &server.base_url(),
        TEST_KEY,
        Some("deepseek-flash"),
        fast_options(),
    )
    .await
    .expect("200 场景不应返回 Err");
    assert!(result.ok);
    assert_eq!(result.status, Some(200));
    assert_eq!(
        result.model.as_deref(),
        Some("deepseek-flash"),
        "响应无法解析时回退到配置模型名"
    );
}

#[tokio::test]
async fn maps_401_and_403_to_auth_failed() {
    for status in [401u16, 403u16] {
        let server = MockServer::start(MockBehavior::Respond {
            status,
            body: r#"{"error":{"message":"invalid api key"}}"#,
        });
        let result = probe_provider(&server.base_url(), TEST_KEY, None, fast_options())
            .await
            .expect("HTTP 错误应以 TestResult 返回而非 Err");
        assert_eq!(result.status, Some(status));
        assert_error(&result, ErrorCode::AuthFailed, false, &["open_settings"]);
    }
}

#[tokio::test]
async fn maps_402_to_quota_exceeded() {
    let server = MockServer::start(MockBehavior::Respond {
        status: 402,
        body: r#"{"error":{"message":"insufficient balance"}}"#,
    });
    let result = probe_provider(&server.base_url(), TEST_KEY, None, fast_options())
        .await
        .expect("HTTP 错误应以 TestResult 返回而非 Err");
    assert_eq!(result.status, Some(402));
    assert_error(&result, ErrorCode::QuotaExceeded, false, &["open_settings"]);
}

#[tokio::test]
async fn maps_429_to_rate_limited_retryable() {
    let server = MockServer::start(MockBehavior::Respond {
        status: 429,
        body: r#"{"error":{"message":"rate limited"}}"#,
    });
    let result = probe_provider(&server.base_url(), TEST_KEY, None, fast_options())
        .await
        .expect("HTTP 错误应以 TestResult 返回而非 Err");
    assert_eq!(result.status, Some(429));
    assert_error(&result, ErrorCode::RateLimited, true, &["retry"]);
}

#[tokio::test]
async fn maps_5xx_to_retryable_network_error() {
    for status in [500u16, 503u16] {
        let server = MockServer::start(MockBehavior::Respond {
            status,
            body: r#"{"error":{"message":"upstream error"}}"#,
        });
        let result = probe_provider(&server.base_url(), TEST_KEY, None, fast_options())
            .await
            .expect("HTTP 错误应以 TestResult 返回而非 Err");
        assert_eq!(result.status, Some(status));
        assert_error(&result, ErrorCode::NetworkUnavailable, true, &["retry"]);
    }
}

#[tokio::test]
async fn maps_other_4xx_to_invalid_config() {
    let server = MockServer::start(MockBehavior::Respond {
        status: 404,
        body: r#"{"error":{"message":"not found"}}"#,
    });
    let result = probe_provider(&server.base_url(), TEST_KEY, None, fast_options())
        .await
        .expect("HTTP 错误应以 TestResult 返回而非 Err");
    assert_eq!(result.status, Some(404));
    assert_error(&result, ErrorCode::InvalidConfig, false, &["reset_group"]);
}

#[tokio::test]
async fn maps_timeout_to_timeout_error() {
    let server = MockServer::start(MockBehavior::Hang);
    let started = Instant::now();
    let result = probe_provider(&server.base_url(), TEST_KEY, None, short_timeout_options())
        .await
        .expect("超时也应以 TestResult 返回，便于前端展示可操作提示");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "客户端应按注入的超时（300ms）中断，实际耗时 {elapsed:?}"
    );
    assert!(result.status.is_none(), "超时场景无 HTTP 状态码");
    assert_error(
        &result,
        ErrorCode::Timeout,
        true,
        &["retry", "browser_search"],
    );
}

#[tokio::test]
async fn maps_connection_refused_to_network_unavailable() {
    // 先绑定再释放，拿到一个确定无人监听的端口。
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("addr").port()
    };
    let result = probe_provider(
        &format!("http://127.0.0.1:{port}"),
        TEST_KEY,
        None,
        fast_options(),
    )
    .await
    .expect("连接拒绝也应以 TestResult 返回");
    assert!(result.status.is_none(), "连接失败无 HTTP 状态码");
    eprintln!("DEBUG_REFUSED: {:?}", result.error);
    assert_error(
        &result,
        ErrorCode::NetworkUnavailable,
        true,
        &["browser_search", "local_translate"],
    );
}

#[tokio::test]
async fn trailing_slash_is_normalized_to_single_models_path() {
    for suffix in ["", "/", "///"] {
        let server = MockServer::start(MockBehavior::Respond {
            status: 200,
            body: MODELS_BODY,
        });
        let base = format!("{}{suffix}", server.base_url());
        let result = probe_provider(&base, TEST_KEY, None, fast_options())
            .await
            .expect("尾斜杠场景不应返回 Err");
        assert!(result.ok, "baseUrl={base} 应成功");
        let request = server.request_text().to_ascii_lowercase();
        assert!(
            request.starts_with("get /models http/1.1"),
            "baseUrl={base} 应拼接为单层 /models，实际请求：{}",
            request.lines().next().unwrap_or("")
        );
    }
    assert_eq!(
        models_url("https://api.deepseek.com/"),
        "https://api.deepseek.com/models"
    );
    assert_eq!(
        models_url("https://api.deepseek.com"),
        "https://api.deepseek.com/models"
    );
}

#[test]
fn rejects_non_https_base_url() {
    let config = Config::default();
    for base_url in ["http://api.deepseek.com", "ftp://api.deepseek.com"] {
        let draft = draft_with_base_url(base_url);
        let error = validate_probe_draft(&config, &draft)
            .expect_err("非 https 且非 127.0.0.1 的 Base URL 必须被拒绝");
        assert_eq!(error.code, ErrorCode::InvalidConfig, "Base URL={base_url}");
        assert!(
            error
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("baseUrl"),
            "错误上下文应指向 providers.baseUrl"
        );
    }
    // 本机 http://127.0.0.1 白名单应放行（契约 §4.2）。
    let draft = draft_with_base_url("http://127.0.0.1:39281");
    assert!(validate_probe_draft(&config, &draft).is_ok());
}

#[tokio::test]
async fn rejects_disallowed_url_before_network() {
    // 防御性校验：即使绕过配置校验直接调用探测，也不得向非白名单地址发请求。
    let error = probe_provider(
        "http://api.deepseek.com",
        TEST_KEY,
        None,
        short_timeout_options(),
    )
    .await
    .expect_err("非白名单 URL 应直接报错");
    assert_eq!(error.code, ErrorCode::InvalidConfig);
}

#[test]
fn missing_api_key_result_is_actionable_auth_error() {
    let result = missing_api_key_result();
    assert!(!result.ok);
    assert!(result.status.is_none());
    assert!(result.latency_ms.is_none());
    let error = result.error.as_ref().expect("应带 AppError");
    assert_eq!(error.code, ErrorCode::AuthFailed);
    assert!(!error.retryable);
    assert!(error.fallback.iter().any(|item| item == "open_settings"));
    assert!(
        error.message.contains("Key"),
        "缺少 Key 的提示应可操作：{}",
        error.message
    );
}

#[tokio::test]
async fn test_result_never_contains_api_key() {
    // 成功与失败两条路径都不允许把 Key 泄漏进 TestResult（日志同理由实现保证）。
    for behavior in [
        MockBehavior::Respond {
            status: 200,
            body: MODELS_BODY,
        },
        MockBehavior::Respond {
            status: 401,
            body: r#"{"error":{"message":"invalid api key"}}"#,
        },
    ] {
        let server = MockServer::start(behavior);
        let result = probe_provider(&server.base_url(), TEST_KEY, None, fast_options())
            .await
            .expect("应以 TestResult 返回");
        let debug = format!("{result:?}");
        assert!(!debug.contains(TEST_KEY), "TestResult 不得包含完整 Key");
        assert!(!debug.contains("sk-test"), "TestResult 不得包含 Key 片段");
    }
}

/// 真实 DeepSeek 冒烟（默认忽略；Key 从环境变量读取，绝不写死）。
///
/// 运行：
/// ```powershell
/// $env:SNIPASK_BENCH_API_KEY='<真实 Key>'
/// cargo test --manifest-path src-tauri/Cargo.toml --test provider_probe_spike -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "需要真实 SNIPASK_BENCH_API_KEY 环境变量（外网）"]
async fn deepseek_live_smoke() {
    let Ok(api_key) = std::env::var("SNIPASK_BENCH_API_KEY") else {
        eprintln!("SKIP: 未设置 SNIPASK_BENCH_API_KEY，跳过真实 DeepSeek 冒烟");
        return;
    };
    assert!(
        api_key.starts_with("sk-"),
        "SNIPASK_BENCH_API_KEY 看起来不是 DeepSeek Key（不记录其内容）"
    );

    let started = Instant::now();
    let outcome = probe_provider_detail(
        "https://api.deepseek.com",
        &api_key,
        Some("deepseek-flash"),
        ProbeOptions::default(),
    )
    .await
    .expect("真实探测本身不应返回 Err（网络失败会以 TestResult.error 表达）");
    let wall_ms = started.elapsed().as_millis() as u64;

    // 原始数据落盘（只记录脱敏状态 / 耗时 / 模型 id，不记录 Key）。
    let record = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "baseUrl": "https://api.deepseek.com",
        "endpoint": models_url("https://api.deepseek.com"),
        "ok": outcome.result.ok,
        "status": outcome.result.status,
        "latencyMs": outcome.result.latency_ms,
        "wallMs": wall_ms,
        "model": outcome.result.model,
        "modelCount": outcome.available_models.len(),
        "models": outcome.available_models,
        "errorCode": outcome.result.error.as_ref().map(|err| err.code.clone()),
        "errorMessage": outcome.result.error.as_ref().map(|err| err.message.clone()),
        "apiKeyProvided": true,
    });
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".verify")
        .join("spikes")
        .join("S-17");
    std::fs::create_dir_all(&dir).expect("创建 .verify/spikes/S-17");
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let path = dir.join(format!("deepseek-smoke-{stamp}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&record).unwrap()).expect("写冒烟原始数据");

    eprintln!(
        "DeepSeek 冒烟：ok={} status={:?} latencyMs={:?} wallMs={} models={:?}",
        outcome.result.ok,
        outcome.result.status,
        outcome.result.latency_ms,
        wall_ms,
        outcome.available_models
    );
    eprintln!("原始数据：{}", path.display());

    // 网络/代理导致的失败不算任务失败（Lead 要求：保留失败证据即可），
    // 但 200 + 模型列表必须满足契约语义。
    if outcome.result.ok {
        assert_eq!(outcome.result.status, Some(200));
        assert!(
            outcome
                .available_models
                .iter()
                .any(|model| model == "deepseek-flash"),
            "DeepSeek /models 应包含 deepseek-flash，实际 {:?}",
            outcome.available_models
        );
    } else {
        assert!(
            outcome.result.error.is_some(),
            "失败场景必须带可操作 AppError 证据"
        );
    }
}
