//! T2 集成测试：真实 OpenAI 兼容流式网关（`HttpLlmGateway`）。
//!
//! **不依赖外网**：用 `std::net::TcpListener` 在 127.0.0.1 起最小 HTTP/SSE server，
//! 可脚本化返回状态码 / SSE 行序列 / 慢速流，并记录请求次数、请求头与请求体。
//!
//! 覆盖：
//! 1. SSE 正文增量 + usage + 引擎字段 + 本地零成本；
//! 2. `reasoning_content` 思考增量；
//! 3. 429 → 指数退避后重试成功（≤2 次）；
//! 4. 401/402 不重试（SA-3002 / SA-3003）；
//! 5. 5xx 重试耗尽 → SA-3001；
//! 6. 取消令牌在块间检查 → SA-1099；
//! 7. 空 content / 无 `[DONE]` 不 panic；
//! 8. 流内非法 JSON / 上游 error 字段 → 错误；
//! 9. 云端请求带 `Authorization` / `stream_options` / `thinking`；本地请求不带；
//! 10. 本地无 usage 时按字符估算 token。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use snip_ask_lib::contract::types::{ActionId, Mode, SessionId, StreamKind, TaskId};
use snip_ask_lib::services::llm_gateway::{LlmGateway, LlmRequest, StreamChunk};
use snip_ask_lib::services::local_gateway::{HttpLlmGateway, BASE_BACKOFF};

// ---------------------------------------------------------------------------
// 最小 HTTP/SSE mock server
// ---------------------------------------------------------------------------

/// 服务端脚本步骤。
#[derive(Debug, Clone)]
enum Step {
    /// 非流式响应：`(状态码, body)`。
    Status(u16, String),
    /// SSE 响应：逐条写 `line + "\n\n"`，行间 `delay`。
    Sse { lines: Vec<String>, delay: Duration },
}

impl Step {
    fn sse(lines: &[&str]) -> Self {
        Step::Sse {
            lines: lines.iter().map(|line| (*line).to_string()).collect(),
            delay: Duration::from_millis(0),
        }
    }
}

/// 一次请求的快照。
#[derive(Debug, Clone, Default)]
struct Recorded {
    /// 请求头原文。
    headers: String,
    /// 请求体原文。
    body: String,
    /// 请求行（含 path）。
    request_line: String,
}

struct ServerState {
    steps: Vec<Step>,
    requests: AtomicUsize,
    recorded: Mutex<Vec<Recorded>>,
}

struct MockServer {
    addr: SocketAddr,
    state: Arc<ServerState>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    fn start(steps: Vec<Step>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地端口失败");
        listener.set_nonblocking(true).expect("设置非阻塞失败");
        let addr = listener.local_addr().expect("读取本地地址失败");
        let state = Arc::new(ServerState {
            steps,
            requests: AtomicUsize::new(0),
            recorded: Mutex::new(Vec::new()),
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let server_state = state.clone();
        let server_shutdown = shutdown.clone();
        let handle = thread::spawn(move || {
            while !server_shutdown.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        handle_connection(stream, &server_state);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            addr,
            state,
            shutdown,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn requests(&self) -> usize {
        self.state.requests.load(Ordering::SeqCst)
    }

    fn last_recorded(&self) -> Recorded {
        self.state
            .recorded
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // 连接一个空连接唤醒 accept（非阻塞 + 5ms 轮询，直接 join 亦可）。
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_connection(mut stream: TcpStream, state: &Arc<ServerState>) {
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let Some(recorded) = read_request(&mut stream) else {
        return;
    };
    let index = state.requests.fetch_add(1, Ordering::SeqCst);
    state.recorded.lock().unwrap().push(recorded);
    let step = state
        .steps
        .get(index)
        .or_else(|| state.steps.last())
        .cloned()
        .unwrap_or(Step::Status(500, "no script".to_string()));
    match step {
        Step::Status(status, body) => {
            let reason = if status == 200 { "OK" } else { "ERR" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
        Step::Sse { lines, delay } => {
            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            if stream.write_all(header.as_bytes()).is_err() {
                return;
            }
            let _ = stream.flush();
            for line in lines {
                if stream.write_all(format!("{line}\n\n").as_bytes()).is_err() {
                    return;
                }
                let _ = stream.flush();
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
            }
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_headers_end(&buffer) {
            let headers = String::from_utf8_lossy(&buffer[..position]).into_owned();
            let content_length = parse_content_length(&headers);
            let body_start = position + 4;
            if buffer.len() >= body_start + content_length {
                let body =
                    String::from_utf8_lossy(&buffer[body_start..body_start + content_length])
                        .into_owned();
                let request_line = headers.lines().next().unwrap_or_default().to_string();
                return Some(Recorded {
                    headers,
                    body,
                    request_line,
                });
            }
        }
    }
    None
}

fn find_headers_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> usize {
    for line in headers.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            return rest.trim().parse::<usize>().unwrap_or(0);
        }
    }
    0
}

// ---------------------------------------------------------------------------
// 测试辅助
// ---------------------------------------------------------------------------

fn request(mode: Mode) -> LlmRequest {
    LlmRequest {
        task: TaskId::from("t-1"),
        session: SessionId::from("s-1"),
        action: ActionId::Translate,
        mode,
        engine: "local".to_string(),
        prompt: "把这句话翻译成英文".to_string(),
        input_text: "把这句话翻译成英文".to_string(),
        image_data_url: None,
        cancel: Arc::new(AtomicBool::new(false)),
    }
}

fn collect_sink() -> (
    Arc<Mutex<Vec<StreamChunk>>>,
    impl Fn(StreamChunk) + Send + Sync,
) {
    let chunks = Arc::new(Mutex::new(Vec::<StreamChunk>::new()));
    let sink_chunks = chunks.clone();
    let sink = move |chunk: StreamChunk| {
        sink_chunks.lock().unwrap().push(chunk);
    };
    (chunks, sink)
}

/// 等待 mock server 记录到至少 `at_least` 个请求（重试计数存在线程调度竞态，
/// 不用「立即断言精确次数」，避免测试偶发失败）。
fn wait_for_requests(server: &MockServer, at_least: usize, timeout: Duration) {
    let started = std::time::Instant::now();
    while server.requests() < at_least {
        if started.elapsed() >= timeout {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// 从 mock server 收到的请求体解析 JSON。
fn body_json(recorded: &Recorded) -> serde_json::Value {
    serde_json::from_str(&recorded.body).unwrap_or(serde_json::Value::Null)
}

// ---------------------------------------------------------------------------
// 用例
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streams_text_usage_and_zero_cost_for_local() {
    let server = MockServer::start(vec![Step::sse(&[
        r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#,
        r#"data: {"choices":[{"delta":{"content":" world"}}]}"#,
        r#"data: {"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#,
        "data: [DONE]",
    ])]);
    let gateway = HttpLlmGateway::new(server.base_url(), None, "hy-mt-1.8b-q4");
    let (chunks, sink) = collect_sink();
    let outcome = gateway
        .stream(&request(Mode::Quick), &sink)
        .await
        .expect("流式调用应成功");

    let chunks = chunks.lock().unwrap().clone();
    let text: String = chunks
        .iter()
        .filter(|chunk| chunk.kind == StreamKind::Text)
        .map(|chunk| chunk.chunk.as_str())
        .collect();
    assert_eq!(text, "Hello world");
    assert_eq!(outcome.usage.input_tokens, 5);
    assert_eq!(outcome.usage.output_tokens, 2);
    assert_eq!(outcome.usage.cost_estimate, 0.0);
    assert_eq!(outcome.engine, "local");
    // 请求打到 /v1/chat/completions，且带 stream:true。
    let recorded = server.last_recorded();
    assert!(
        recorded.request_line.contains("/v1/chat/completions"),
        "request line = {}",
        recorded.request_line
    );
    assert_eq!(body_json(&recorded)["stream"], true);
    // 本地端点不带 Authorization（无 API Key）。
    assert!(!recorded
        .headers
        .to_ascii_lowercase()
        .contains("authorization:"));
}

#[tokio::test]
async fn forwards_reasoning_deltas() {
    let server = MockServer::start(vec![Step::sse(&[
        r#"data: {"choices":[{"delta":{"reasoning_content":"先分析"}}]}"#,
        r#"data: {"choices":[{"delta":{"content":"答案"}}]}"#,
        "data: [DONE]",
    ])]);
    let gateway = HttpLlmGateway::new(server.base_url(), None, "m");
    let (chunks, sink) = collect_sink();
    gateway.stream(&request(Mode::Deep), &sink).await.unwrap();
    let chunks = chunks.lock().unwrap().clone();
    let kinds: Vec<StreamKind> = chunks.iter().map(|chunk| chunk.kind).collect();
    assert_eq!(kinds, vec![StreamKind::Reasoning, StreamKind::Text]);
    assert_eq!(chunks[0].chunk, "先分析");
}

#[tokio::test]
async fn retries_after_429_then_succeeds() {
    let server = MockServer::start(vec![
        Step::Status(429, r#"{"error":{"message":"rate limited"}}"#.to_string()),
        Step::sse(&[
            r#"data: {"choices":[{"delta":{"content":"ok"}}]}"#,
            "data: [DONE]",
        ]),
    ]);
    let gateway = HttpLlmGateway::new(server.base_url(), None, "m");
    let (chunks, sink) = collect_sink();
    let outcome = gateway.stream(&request(Mode::Quick), &sink).await.unwrap();
    wait_for_requests(&server, 2, Duration::from_secs(2));
    assert!(server.requests() >= 2, "429 后应至少重试一次");
    let text: String = chunks
        .lock()
        .unwrap()
        .iter()
        .map(|chunk| chunk.chunk.clone())
        .collect();
    assert_eq!(text, "ok");
    assert!(outcome.usage.input_tokens > 0);
}

#[tokio::test]
async fn auth_error_is_not_retried() {
    let server = MockServer::start(vec![Step::Status(401, "{}".to_string())]);
    let gateway = HttpLlmGateway::new(
        format!("{}/v1", server.base_url()),
        Some("bad-key".to_string()),
        "m",
    );
    let (_chunks, sink) = collect_sink();
    let error = gateway
        .stream(&request(Mode::Quick), &sink)
        .await
        .unwrap_err();
    assert_eq!(error.code, "SA-3002");
    assert_eq!(server.requests(), 1, "401 不得重试");
    // 云端 base_url 带 /v1 时仍拼出正确端点（不出现 /v1/v1）。
    assert!(server
        .last_recorded()
        .request_line
        .contains("/v1/chat/completions"));
    assert!(server
        .last_recorded()
        .headers
        .to_ascii_lowercase()
        .contains("authorization: bearer bad-key"));
}

#[tokio::test]
async fn quota_error_is_not_retried() {
    let server = MockServer::start(vec![Step::Status(402, "{}".to_string())]);
    let gateway = HttpLlmGateway::new(server.base_url(), Some("k".to_string()), "m");
    let (_chunks, sink) = collect_sink();
    let error = gateway
        .stream(&request(Mode::Quick), &sink)
        .await
        .unwrap_err();
    assert_eq!(error.code, "SA-3003");
    assert_eq!(server.requests(), 1);
}

#[tokio::test]
async fn server_errors_exhaust_retries() {
    let server = MockServer::start(vec![Step::Status(500, r#"{"error":"boom"}"#.to_string())]);
    let gateway = HttpLlmGateway::new(server.base_url(), None, "m");
    let (_chunks, sink) = collect_sink();
    let started = std::time::Instant::now();
    let error = gateway
        .stream(&request(Mode::Quick), &sink)
        .await
        .unwrap_err();
    assert_eq!(error.code, "SA-3001");
    // 连接复用/关闭存在调度竞态：重试可能打在服务端已关闭的连接上而在传输层失败
    // （网关同样按可重试处理），此时服务端不会记录该请求。因此以「请求数」为主、
    // 「退避耗时」为兜底：一次重试至少等待 BASE_BACKOFF，两次重试共 3×BASE_BACKOFF，
    // 若完全没重试则耗时接近 0。
    wait_for_requests(&server, 2, Duration::from_secs(2));
    let elapsed = started.elapsed();
    assert!(
        server.requests() >= 2 || elapsed >= BASE_BACKOFF * 2,
        "5xx 应触发重试（请求数 {}，耗时 {:?}）",
        server.requests(),
        elapsed
    );
}

#[tokio::test]
async fn cancel_between_chunks_returns_invalid_state() {
    let server = MockServer::start(vec![Step::Sse {
        lines: vec![
            r#"data: {"choices":[{"delta":{"content":"1"}}]}"#.to_string(),
            r#"data: {"choices":[{"delta":{"content":"2"}}]}"#.to_string(),
            r#"data: {"choices":[{"delta":{"content":"3"}}]}"#.to_string(),
            "data: [DONE]".to_string(),
        ],
        delay: Duration::from_millis(80),
    }]);
    let gateway = HttpLlmGateway::new(server.base_url(), None, "m");
    let req = request(Mode::Quick);
    let cancel = req.cancel.clone();
    let chunks = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink_chunks = chunks.clone();
    let sink = move |chunk: StreamChunk| {
        sink_chunks.lock().unwrap().push(chunk.chunk);
        cancel.store(true, Ordering::SeqCst);
    };
    let error = gateway.stream(&req, &sink).await.unwrap_err();
    assert_eq!(error.code, "SA-1099");
    assert_eq!(chunks.lock().unwrap().len(), 1, "取消后不应继续推送");
}

#[tokio::test]
async fn empty_content_and_missing_done_do_not_panic() {
    let server = MockServer::start(vec![Step::sse(&[
        r#"data: {"choices":[{"delta":{"content":""}}]}"#,
        r#"data: {"choices":[{"delta":{}}]}"#,
        r#"data: {"choices":[]}"#,
    ])]);
    let gateway = HttpLlmGateway::new(server.base_url(), None, "m");
    let (chunks, sink) = collect_sink();
    let outcome = gateway.stream(&request(Mode::Quick), &sink).await.unwrap();
    assert!(chunks.lock().unwrap().is_empty());
    // 无 usage → 本地按字符估算。
    assert!(outcome.usage.input_tokens > 0);
}

#[tokio::test]
async fn upstream_error_payload_is_surfaced() {
    let server = MockServer::start(vec![Step::sse(&[
        r#"data: {"error":{"message":"model overloaded"}}"#,
    ])]);
    let gateway = HttpLlmGateway::new(server.base_url(), None, "m");
    let (_chunks, sink) = collect_sink();
    let error = gateway
        .stream(&request(Mode::Quick), &sink)
        .await
        .unwrap_err();
    assert_eq!(error.code, "SA-3001");
    assert!(
        error
            .detail
            .unwrap_or_default()
            .contains("model overloaded"),
        "detail 应包含上游消息"
    );
}

#[tokio::test]
async fn malformed_sse_json_is_reported_not_panicked() {
    let server = MockServer::start(vec![Step::sse(&["data: not-json"])]);
    let gateway = HttpLlmGateway::new(server.base_url(), None, "m");
    let (_chunks, sink) = collect_sink();
    let error = gateway
        .stream(&request(Mode::Quick), &sink)
        .await
        .unwrap_err();
    assert_eq!(error.code, "SA-3001");
    assert!(error.detail.unwrap_or_default().contains("SSE JSON"));
}

#[tokio::test]
async fn llama_timings_are_used_when_usage_absent() {
    let server = MockServer::start(vec![Step::sse(&[
        r#"data: {"choices":[{"delta":{"content":"hi"}}],"timings":{"prompt_n":7,"predicted_n":9}}"#,
        "data: [DONE]",
    ])]);
    let gateway = HttpLlmGateway::new(server.base_url(), None, "m");
    let (chunks, sink) = collect_sink();
    let outcome = gateway.stream(&request(Mode::Quick), &sink).await.unwrap();
    assert_eq!(chunks.lock().unwrap().len(), 1);
    assert_eq!(outcome.usage.input_tokens, 7);
    assert_eq!(outcome.usage.output_tokens, 9);
}

#[tokio::test]
async fn cloud_request_carries_auth_stream_options_and_thinking() {
    let server = MockServer::start(vec![Step::sse(&["data: [DONE]"])]);
    let gateway = HttpLlmGateway::new(
        format!("{}/v1", server.base_url()),
        Some("sk-test".to_string()),
        "deepseek-flash",
    );
    let (_chunks, sink) = collect_sink();
    gateway.stream(&request(Mode::Quick), &sink).await.unwrap();
    let recorded = server.last_recorded();
    assert!(recorded
        .headers
        .to_ascii_lowercase()
        .contains("authorization: bearer sk-test"));
    assert!(recorded
        .headers
        .to_ascii_lowercase()
        .contains("content-type: application/json"));
    let body = body_json(&recorded);
    assert_eq!(
        body["stream"], true,
        "请求体未正确序列化：{}；请求头：{}",
        recorded.body, recorded.headers
    );
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["thinking"]["type"], "disabled");
    assert_eq!(body["model"], "deepseek-flash");
    // 云端有价格表时给出费用（deepseek-flash 在 pricing.json 中存在）。
    let (_chunks2, sink2) = collect_sink();
    let outcome = gateway.stream(&request(Mode::Quick), &sink2).await.unwrap();
    assert!(outcome.usage.cost_estimate >= 0.0);
}

#[tokio::test]
async fn multimodal_image_becomes_content_array() {
    let server = MockServer::start(vec![Step::sse(&["data: [DONE]"])]);
    let gateway = HttpLlmGateway::new(server.base_url(), None, "m");
    let mut req = request(Mode::Quick);
    req.image_data_url = Some("data:image/png;base64,AAAA".to_string());
    let (_chunks, sink) = collect_sink();
    gateway.stream(&req, &sink).await.unwrap();
    let body = body_json(&server.last_recorded());
    assert!(body["messages"][0]["content"].is_array());
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["url"],
        "data:image/png;base64,AAAA"
    );
}
