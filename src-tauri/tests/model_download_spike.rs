//! S-08 集成测试：模型真实下载（HTTP Range 断点续传 / 流式 SHA256 / 大小校验 / 镜像回退）。
//!
//! 不引入第三方 HTTP server 依赖：用 `std::net::TcpListener` 在 127.0.0.1 起本地 HTTP server，
//! 可配置「支持 Range / 忽略 Range / 固定状态码 / 慢速发送 / 首个请求在指定比例处断连」
//! 等行为，并记录每次请求的 `Range` 起点，用于断言续传从断点开始。
//!
//! 覆盖（`docs/技术验证清单.md` S-08 验收项）：
//! 1. 支持 Range 的正常下载 + SHA256 / 大小校验通过；
//! 2. 传输到 40% 主动断开 → 自动重试续传，第二次请求 Range 起点 == 断点；
//! 3. 取消 → `Cancelled`、`.partial` 保留 → 再次下载续传成功；
//! 4. 20 次在 20%~80% 随机断开 → 恢复成功率 100%（总时长 < 15s）；
//! 5. 镜像回退：首个 URL 404 / 连接拒绝 → 自动切换第二个镜像成功；
//! 6. SHA256 不匹配 → 删除 `.partial` 并失败（按「重下一次」策略）；
//! 7. 服务端忽略 Range（200）→ 丢弃断点从头下载；
//! 8. 进度正确性：loaded 单调递增、最终 loaded == total、speedBps > 0、可推导剩余时间；
//! 9. 清单 sha256 占位（64 个 0）→ 跳过哈希校验但记录实际哈希；
//! 10. **真实杀进程**：子进程下载到中途被`TerminateProcess`强杀 → 父进程（新进程）复用磁盘
//!     `.partial` 续传成功（验证「杀进程后恢复」）。

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use snip_ask_lib::services::model_manager::{
    download_with_resume, partial_path, DownloadError, DownloadOutcome, DownloadProgress,
    RetryPolicy,
};

const PLACEHOLDER_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

// ---------------------------------------------------------------------------
// 本地 HTTP server（std TcpListener，仅实现测试所需的最小 HTTP/1.1）
// ---------------------------------------------------------------------------

/// server 行为。
#[derive(Debug, Clone, Copy)]
enum Mode {
    /// 支持 Range：无 Range 返回 200，有 Range 返回 206 + Content-Range。
    RangeOk,
    /// 支持 Range，但**首个请求**只发送 `fraction × 总大小` 后断开（模拟网络中断）。
    AbortFirst(f64),
    /// 忽略 Range：无论是否带 Range 都返回 200 + 全量内容（模拟不支持断点续传的镜像）。
    IgnoreRange,
    /// 路径含 `missing` 返回 404，否则按 RangeOk 处理（模拟镜像失效）。
    NotFoundPath,
    /// 支持 Range，但每块之间 sleep，用于取消/进度测试。
    SlowRange { chunk: usize, delay: Duration },
}

struct ServerState {
    mode: Mode,
    payload: Vec<u8>,
    requests: AtomicUsize,
    /// 每次请求解析出的 `Range: bytes=<start>-` 起点（无 Range 为 None）。
    ranges: Mutex<Vec<Option<u64>>>,
}

struct TestServer {
    addr: std::net::SocketAddr,
    state: Arc<ServerState>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn start(mode: Mode, payload: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地端口失败");
        listener.set_nonblocking(true).expect("设置非阻塞失败");
        let addr = listener.local_addr().expect("读取本地地址失败");
        let state = Arc::new(ServerState {
            mode,
            payload,
            requests: AtomicUsize::new(0),
            ranges: Mutex::new(Vec::new()),
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_state = Arc::clone(&state);
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_nodelay(true);
                        handle_connection(stream, &thread_state);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
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

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    fn ranges(&self) -> Vec<Option<u64>> {
        self.state.ranges.lock().expect("ranges 锁中毒").clone()
    }

    fn requests(&self) -> usize {
        self.state.requests.load(Ordering::SeqCst)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_connection(mut stream: TcpStream, state: &ServerState) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        if head.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(read) => head.extend_from_slice(&buf[..read]),
            Err(_) => return,
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let _method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or("/").to_string();
    let mut range_start: Option<u64> = None;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("range:") {
            if let Some(bytes) = rest.trim().strip_prefix("bytes=") {
                if let Some(start) = bytes.split('-').next() {
                    range_start = start.trim().parse::<u64>().ok();
                }
            }
        }
    }
    let request_index = state.requests.fetch_add(1, Ordering::SeqCst);
    state
        .ranges
        .lock()
        .expect("ranges 锁中毒")
        .push(range_start);
    respond(&mut stream, state, request_index, &path, range_start);
}

fn respond(
    stream: &mut TcpStream,
    state: &ServerState,
    request_index: usize,
    path: &str,
    range_start: Option<u64>,
) {
    let len = state.payload.len() as u64;

    if matches!(state.mode, Mode::NotFoundPath) && path.contains("missing") {
        let body = b"not found";
        let header = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body);
        return;
    }

    let ignore_range = matches!(state.mode, Mode::IgnoreRange);
    let start = if ignore_range {
        0
    } else {
        range_start.unwrap_or(0)
    };
    if start > 0 && start >= len {
        let header = format!(
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(header.as_bytes());
        return;
    }

    let is_partial = !ignore_range && range_start.is_some();
    let (status_line, content_range) = if is_partial {
        (
            "HTTP/1.1 206 Partial Content",
            format!("Content-Range: bytes {start}-{}/{len}\r\n", len - 1),
        )
    } else {
        ("HTTP/1.1 200 OK", String::new())
    };

    let remaining = &state.payload[start as usize..];
    let abort_bytes = match state.mode {
        Mode::AbortFirst(fraction) if request_index == 0 => {
            Some(((len as f64) * fraction).clamp(0.0, len as f64) as usize)
        }
        _ => None,
    };
    let header = format!(
        "{status_line}\r\nContent-Length: {}\r\n{content_range}Connection: close\r\n\r\n",
        remaining.len()
    );
    if stream.write_all(header.as_bytes()).is_err() {
        return;
    }

    let send_bytes = abort_bytes.unwrap_or(remaining.len()).min(remaining.len());
    let chunk = match state.mode {
        Mode::SlowRange { chunk, .. } => chunk.max(1),
        _ => 64 * 1024,
    };
    let mut written = 0;
    while written < send_bytes {
        let take = chunk.min(send_bytes - written);
        if stream
            .write_all(&remaining[written..written + take])
            .is_err()
        {
            return;
        }
        written += take;
        if let Mode::SlowRange { delay, .. } = state.mode {
            thread::sleep(delay);
        }
    }
    let _ = stream.flush();
    if abort_bytes.is_some() {
        // 提前关闭写端：客户端按 Content-Length 会收到「响应体不完整」错误。
        let _ = stream.shutdown(Shutdown::Write);
    }
}

// ---------------------------------------------------------------------------
// 测试辅助
// ---------------------------------------------------------------------------

fn test_payload(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| ((index * 31 + 7) % 251) as u8)
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn test_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(5))
        // 测试全部走 127.0.0.1 的本地 mock server：必须绕过用户系统代理
        // （开发机可能开启 Clash/Steam++ 等系统代理；reqwest `system-proxy` 会拦截 loopback，
        //  代理返回 404 导致本测试假失败——CI 无代理故历史上未暴露）。
        .no_proxy()
        // 真实镜像（ModelScope/Aliyun CDN）对「无 User-Agent」请求返回 403，必须带 UA；
        // 这里与生产客户端 `build_http_client()` 保持一致（`snip-ask/<version>`）。
        .user_agent(concat!("snip-ask/", env!("CARGO_PKG_VERSION"), "-test"))
        .build()
        .expect("创建测试 HTTP 客户端失败")
}

fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        max_retries_per_url: 3,
        base_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(100),
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("snip-ask-s08-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("创建临时目录失败");
    dir
}

async fn download_to_completion(
    client: &reqwest::Client,
    urls: &[String],
    dest: &std::path::Path,
    len: u64,
    sha256: &str,
) -> Result<DownloadOutcome, DownloadError> {
    let cancel = AtomicBool::new(false);
    download_with_resume(
        client,
        urls,
        dest,
        len,
        sha256,
        &cancel,
        &fast_retry(),
        |_| {},
    )
    .await
}

// ---------------------------------------------------------------------------
// 1. 正常下载 + 校验
// ---------------------------------------------------------------------------

#[tokio::test]
async fn range_download_passes_sha256_and_size_checks() {
    let payload = test_payload(512 * 1024);
    let expected = sha256_hex(&payload);
    let server = TestServer::start(Mode::RangeOk, payload.clone());
    let dir = temp_dir("normal");
    let dest = dir.join("model.gguf");
    let urls = vec![server.url("/model.gguf")];

    let outcome = download_to_completion(
        &test_client(),
        &urls,
        &dest,
        payload.len() as u64,
        &expected,
    )
    .await
    .expect("正常下载应成功");

    assert_eq!(outcome.bytes, payload.len() as u64);
    assert_eq!(outcome.sha256, expected);
    assert!(!outcome.hash_skipped);
    assert_eq!(outcome.url, urls[0]);
    assert_eq!(std::fs::read(&dest).expect("读取下载文件失败"), payload);
    assert!(!partial_path(&dest).exists(), "成功后不应残留 .partial");
    let ranges = server.ranges();
    assert_eq!(ranges.len(), 1);
    assert!(ranges[0].is_none(), "首次请求不应带 Range: {ranges:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. 40% 处断连 → 续传
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resumes_from_breakpoint_after_connection_dropped_at_40_percent() {
    let payload = test_payload(2 * 1024 * 1024);
    let expected = sha256_hex(&payload);
    let server = TestServer::start(Mode::AbortFirst(0.40), payload.clone());
    let dir = temp_dir("resume40");
    let dest = dir.join("model.gguf");
    let urls = vec![server.url("/model.gguf")];

    let outcome = download_to_completion(
        &test_client(),
        &urls,
        &dest,
        payload.len() as u64,
        &expected,
    )
    .await
    .expect("断点续传应成功");

    assert_eq!(outcome.bytes, payload.len() as u64);
    assert_eq!(std::fs::read(&dest).expect("读取下载文件失败"), payload);

    let ranges = server.ranges();
    assert!(
        ranges.len() >= 2,
        "应至少两次请求（首断 + 续传）：{ranges:?}"
    );
    assert!(ranges[0].is_none(), "首次请求不应带 Range");
    let abort_bytes = (payload.len() as f64 * 0.40) as u64;
    assert_eq!(
        ranges[1],
        Some(abort_bytes),
        "第二次请求必须从断点续传：{ranges:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. 取消 → 保留 .partial → 续传成功
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_keeps_partial_then_resume_completes() {
    let payload = test_payload(1024 * 1024);
    let expected = sha256_hex(&payload);
    let server = TestServer::start(
        Mode::SlowRange {
            chunk: 16 * 1024,
            delay: Duration::from_millis(3),
        },
        payload.clone(),
    );
    let dir = temp_dir("cancel");
    let dest = dir.join("model.gguf");
    let partial = partial_path(&dest);
    let urls = vec![server.url("/model.gguf")];
    let cancel = Arc::new(AtomicBool::new(false));
    let client = test_client();
    let retry = fast_retry();

    let mut fut = Box::pin(download_with_resume(
        &client,
        &urls,
        &dest,
        payload.len() as u64,
        &expected,
        &cancel,
        &retry,
        |_| {},
    ));
    tokio::select! {
        result = &mut fut => panic!("慢速下载不应在取消前完成：{result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(120)) => {}
    }
    cancel.store(true, Ordering::SeqCst);
    let first = fut.await;
    assert!(
        matches!(&first, Err(DownloadError::Cancelled)),
        "取消后应返回 Cancelled，实际：{first:?}"
    );
    assert!(!dest.exists(), "取消后不应出现最终文件");
    let partial_len = std::fs::metadata(&partial)
        .expect("取消后 .partial 必须保留")
        .len();
    assert!(
        partial_len > 0 && partial_len < payload.len() as u64,
        "取消时断点应介于 0 与总大小之间，实际 {partial_len}"
    );

    // 再次下载：不加取消标志，服务端按 Range 续传剩余部分。
    let cancel2 = AtomicBool::new(false);
    let outcome = download_with_resume(
        &client,
        &urls,
        &dest,
        payload.len() as u64,
        &expected,
        &cancel2,
        &fast_retry(),
        |_| {},
    )
    .await
    .expect("取消后再次下载应续传成功");
    assert_eq!(outcome.bytes, payload.len() as u64);
    assert_eq!(std::fs::read(&dest).expect("读取下载文件失败"), payload);
    assert!(!partial.exists());
    let ranges = server.ranges();
    assert!(
        ranges.iter().skip(1).any(|range| range.is_some()),
        "续传请求必须带 Range：{ranges:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4. 20 次随机中断 → 恢复率 100%
// ---------------------------------------------------------------------------

#[tokio::test]
async fn twenty_random_interruptions_recover_100_percent() {
    let started = Instant::now();
    let payload = test_payload(2 * 1024 * 1024);
    let expected = sha256_hex(&payload);
    let client = test_client();
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut recovered = 0u32;
    let mut failures: Vec<String> = Vec::new();

    for index in 0..20u32 {
        // xorshift：确定性伪随机，避免引入 rand 依赖。
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let fraction = 0.2 + (seed % 1000) as f64 / 1000.0 * 0.6; // [0.2, 0.8)
        let server = TestServer::start(Mode::AbortFirst(fraction), payload.clone());
        let dir = temp_dir(&format!("stress-{index}"));
        let dest = dir.join("model.gguf");
        let urls = vec![server.url("/model.gguf")];

        let result =
            download_to_completion(&client, &urls, &dest, payload.len() as u64, &expected).await;
        match result {
            Ok(outcome) => {
                assert_eq!(outcome.bytes, payload.len() as u64);
                assert_eq!(std::fs::read(&dest).expect("读取下载文件失败"), payload);
                let ranges = server.ranges();
                assert!(
                    ranges.len() >= 2,
                    "第 {index} 次应发生断连 + 续传：{ranges:?}"
                );
                let sent = (payload.len() as f64 * fraction) as u64;
                assert_eq!(
                    ranges[1],
                    Some(sent),
                    "第 {index} 次续传起点错误（fraction={fraction:.3}）：{ranges:?}"
                );
                recovered += 1;
            }
            Err(err) => failures.push(format!("第 {index} 次（fraction={fraction:.3}）：{err}")),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    let elapsed = started.elapsed();
    println!(
        "S-08 20 次随机中断恢复：{recovered}/20，成功率 {}%，耗时 {:.2}s",
        recovered * 5,
        elapsed.as_secs_f64()
    );
    assert!(failures.is_empty(), "存在恢复失败：{}", failures.join("; "));
    assert_eq!(recovered, 20, "恢复成功率必须 100%");
    assert!(
        elapsed < Duration::from_secs(15),
        "20 次恢复耗时 {elapsed:?} 超过 15s 上限"
    );
}

// ---------------------------------------------------------------------------
// 5. 镜像回退
// ---------------------------------------------------------------------------

#[tokio::test]
async fn falls_back_to_next_mirror_on_404() {
    let payload = test_payload(128 * 1024);
    let expected = sha256_hex(&payload);
    let server = TestServer::start(Mode::NotFoundPath, payload.clone());
    let dir = temp_dir("mirror404");
    let dest = dir.join("model.gguf");
    let urls = vec![
        server.url("/missing/model.gguf"),
        server.url("/mirror/model.gguf"),
    ];

    let outcome = download_to_completion(
        &test_client(),
        &urls,
        &dest,
        payload.len() as u64,
        &expected,
    )
    .await
    .expect("首个镜像 404 后应回退成功");

    assert_eq!(outcome.url, urls[1]);
    assert_eq!(std::fs::read(&dest).expect("读取下载文件失败"), payload);
    assert_eq!(server.requests(), 2, "应恰好：404 + 回退请求");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn falls_back_to_next_mirror_on_connection_refused() {
    let payload = test_payload(64 * 1024);
    let expected = sha256_hex(&payload);
    let server = TestServer::start(Mode::RangeOk, payload.clone());
    let dir = temp_dir("mirrorrefused");
    let dest = dir.join("model.gguf");
    // 绑定后立即释放端口，得到一个必然连接拒绝的地址。
    let dead_port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定临时端口失败");
        listener.local_addr().expect("读取临时地址失败").port()
    };
    let urls = vec![
        format!("http://127.0.0.1:{dead_port}/model.gguf"),
        server.url("/model.gguf"),
    ];

    let outcome = download_to_completion(
        &test_client(),
        &urls,
        &dest,
        payload.len() as u64,
        &expected,
    )
    .await
    .expect("首个镜像连接拒绝后应回退成功");

    assert_eq!(outcome.url, urls[1]);
    assert_eq!(std::fs::read(&dest).expect("读取下载文件失败"), payload);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 6. SHA256 不匹配 → 删除 .partial + 失败
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sha256_mismatch_deletes_partial_and_fails() {
    let payload = test_payload(256 * 1024);
    let wrong_expected = sha256_hex(b"another payload");
    let server = TestServer::start(Mode::RangeOk, payload.clone());
    let dir = temp_dir("shamismatch");
    let dest = dir.join("model.gguf");
    let urls = vec![server.url("/model.gguf")];

    let result = download_to_completion(
        &test_client(),
        &urls,
        &dest,
        payload.len() as u64,
        &wrong_expected,
    )
    .await;

    match result {
        Err(DownloadError::Verify(message)) => {
            assert!(
                message.contains("SHA256"),
                "错误信息应为 SHA256 不符：{message}"
            );
        }
        other => panic!("SHA256 不匹配应返回 Verify 错误，实际：{other:?}"),
    }
    assert!(!dest.exists(), "校验失败不应留下最终文件");
    assert!(!partial_path(&dest).exists(), "校验失败必须删除 .partial");
    assert!(
        server.requests() >= 2,
        "按策略应「删除 .partial 后重下一次」，实际请求数：{}",
        server.requests()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn size_mismatch_fails_after_retry() {
    let payload = test_payload(256 * 1024);
    let expected = sha256_hex(&payload);
    let server = TestServer::start(Mode::RangeOk, payload.clone());
    let dir = temp_dir("sizemismatch");
    let dest = dir.join("model.gguf");
    let urls = vec![server.url("/model.gguf")];

    let result = download_to_completion(
        &test_client(),
        &urls,
        &dest,
        payload.len() as u64 + 1,
        &expected,
    )
    .await;

    match result {
        Err(DownloadError::Verify(message)) => {
            assert!(message.contains("大小"), "错误信息应为大小不符：{message}");
        }
        other => panic!("大小不符应返回 Verify 错误，实际：{other:?}"),
    }
    assert!(!dest.exists());
    assert!(!partial_path(&dest).exists(), "校验失败必须删除 .partial");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 7. 服务端忽略 Range（200）→ 丢弃断点从头下载
// ---------------------------------------------------------------------------

#[tokio::test]
async fn server_ignoring_range_restarts_from_scratch() {
    let payload = test_payload(128 * 1024);
    let expected = sha256_hex(&payload);
    let server = TestServer::start(Mode::IgnoreRange, payload.clone());
    let dir = temp_dir("ignorerange");
    let dest = dir.join("model.gguf");
    let partial = partial_path(&dest);
    std::fs::write(&partial, vec![0xAAu8; 40 * 1024]).expect("预置 .partial 失败");
    let urls = vec![server.url("/model.gguf")];

    let outcome = download_to_completion(
        &test_client(),
        &urls,
        &dest,
        payload.len() as u64,
        &expected,
    )
    .await
    .expect("服务端忽略 Range 时应从头重下成功");

    assert_eq!(outcome.bytes, payload.len() as u64);
    assert_eq!(
        std::fs::read(&dest).expect("读取下载文件失败"),
        payload,
        "不应残留旧 .partial 的垃圾数据"
    );
    let ranges = server.ranges();
    assert_eq!(
        ranges[0],
        Some(40 * 1024),
        "首次请求应携带断点 Range：{ranges:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 8. 进度正确性：loaded 单调、total 恒定、speedBps > 0、终值 == total
// ---------------------------------------------------------------------------

#[tokio::test]
async fn progress_is_monotonic_with_positive_speed_and_correct_total() {
    let payload = test_payload(256 * 1024);
    let expected = sha256_hex(&payload);
    let server = TestServer::start(
        Mode::SlowRange {
            chunk: 16 * 1024,
            delay: Duration::from_millis(3),
        },
        payload.clone(),
    );
    let dir = temp_dir("progress");
    let dest = dir.join("model.gguf");
    let urls = vec![server.url("/model.gguf")];
    let cancel = AtomicBool::new(false);
    let events: Arc<Mutex<Vec<DownloadProgress>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);

    download_with_resume(
        &test_client(),
        &urls,
        &dest,
        payload.len() as u64,
        &expected,
        &cancel,
        &fast_retry(),
        move |progress| sink.lock().expect("进度锁中毒").push(progress),
    )
    .await
    .expect("进度测试下载应成功");

    let events = events.lock().expect("进度锁中毒").clone();
    assert!(!events.is_empty(), "必须上报进度事件");
    let total = payload.len() as u64;
    for (index, event) in events.iter().enumerate() {
        assert_eq!(
            event.total, total,
            "第 {index} 条进度 total 错误：{event:?}"
        );
        if index > 0 {
            assert!(
                event.loaded >= events[index - 1].loaded,
                "loaded 必须单调递增：{} → {}",
                events[index - 1].loaded,
                event.loaded
            );
        }
    }
    assert_eq!(
        events.last().expect("进度事件非空").loaded,
        total,
        "最终 loaded 必须 == total"
    );
    assert!(
        events.iter().any(|event| event.speed_bps > 0),
        "至少一条进度应有正速度：{events:?}"
    );

    // 剩余时间：契约事件无 ETA 字段，前端用 (total - loaded) / speedBps 推导；
    // 断言中途每条可推导 ETA 均有限且为正、终值 ETA 为 0。
    for event in &events {
        if event.loaded < total && event.speed_bps > 0 {
            let eta_seconds = (total - event.loaded) as f64 / event.speed_bps as f64;
            assert!(
                eta_seconds.is_finite() && eta_seconds > 0.0,
                "ETA 应可推导：{event:?} → {eta_seconds}"
            );
            assert!(
                eta_seconds < 60.0,
                "本地慢速下载的 ETA 不应超过 60s：{event:?} → {eta_seconds}"
            );
        }
    }
    let final_event = events.last().expect("进度事件非空");
    let final_eta = if final_event.speed_bps > 0 {
        (total - final_event.loaded) as f64 / final_event.speed_bps as f64
    } else {
        0.0
    };
    assert_eq!(final_eta, 0.0, "完成时剩余时间应为 0");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 9. 清单 sha256 占位 → 跳过哈希校验但记录实际哈希
// ---------------------------------------------------------------------------

#[tokio::test]
async fn placeholder_sha256_skips_hash_check_but_reports_actual_hash() {
    let payload = test_payload(64 * 1024);
    let actual = sha256_hex(&payload);
    let server = TestServer::start(Mode::RangeOk, payload.clone());
    let dir = temp_dir("placeholder");
    let dest = dir.join("model.gguf");
    let urls = vec![server.url("/model.gguf")];

    let outcome = download_to_completion(
        &test_client(),
        &urls,
        &dest,
        payload.len() as u64,
        PLACEHOLDER_SHA256,
    )
    .await
    .expect("占位 sha256 应跳过哈希校验并成功");

    assert!(outcome.hash_skipped, "占位 sha256 应标记 hash_skipped");
    assert_eq!(outcome.sha256, actual, "仍应记录实际哈希供回填清单");
    assert_eq!(std::fs::read(&dest).expect("读取下载文件失败"), payload);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 10. 真实杀进程 → 新进程复用 .partial 续传（子进程 helper + 父进程 Kill）
// ---------------------------------------------------------------------------

const KILL_HELPER_ENV_URL: &str = "SNIP_ASK_S08_KILL_URL";
const KILL_HELPER_ENV_DEST: &str = "SNIP_ASK_S08_KILL_DEST";
const KILL_HELPER_ENV_TOTAL: &str = "SNIP_ASK_S08_KILL_TOTAL";

/// 子进程 helper：被 [`resume_after_process_kill`] 以
/// `--exact s08_kill_child_helper --ignored` 触发，慢速下载直到被父进程 Kill。
///
/// 这是唯一带 `#[ignore]` 的测试：直接运行会一直下载（慢速），故仅在父进程测试中启动。
#[test]
#[ignore = "仅由 resume_after_process_kill 作为子进程触发（会被 Kill，不单独运行）"]
fn s08_kill_child_helper() {
    let url = std::env::var(KILL_HELPER_ENV_URL).expect("子进程缺少下载 URL 环境变量");
    let dest =
        PathBuf::from(std::env::var(KILL_HELPER_ENV_DEST).expect("子进程缺少目标路径环境变量"));
    let total: u64 = std::env::var(KILL_HELPER_ENV_TOTAL)
        .expect("子进程缺少文件大小环境变量")
        .parse()
        .expect("文件大小环境变量非数字");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("子进程创建 tokio runtime 失败");
    runtime.block_on(async {
        let cancel = AtomicBool::new(false);
        let client = test_client();
        // 慢速 server 会持续发送，正常情况这里永远不会返回（父进程会 Kill 本进程）。
        let _ = download_with_resume(
            &client,
            &[url],
            &dest,
            total,
            "",
            &cancel,
            &fast_retry(),
            |_| {},
        )
        .await;
    });
}

/// 真实「杀进程」恢复：子进程下载到中途被强制终止，父进程（新进程）直接复用磁盘
/// `.partial` 续传成功 —— 证明断点不依赖进程内存状态。
#[test]
fn resume_after_process_kill() {
    let payload = test_payload(1024 * 1024);
    let total = payload.len() as u64;
    let expected = sha256_hex(&payload);
    let server = TestServer::start(
        Mode::SlowRange {
            chunk: 16 * 1024,
            delay: Duration::from_millis(3),
        },
        payload.clone(),
    );
    let dir = temp_dir("kill");
    let dest = dir.join("model.gguf");
    let partial = partial_path(&dest);
    let exe = std::env::current_exe().expect("获取当前测试进程路径失败");
    let mut child = std::process::Command::new(exe)
        .args([
            "--exact",
            "s08_kill_child_helper",
            "--ignored",
            "--nocapture",
        ])
        .env(KILL_HELPER_ENV_URL, server.url("/model.gguf"))
        .env(KILL_HELPER_ENV_DEST, &dest)
        .env(KILL_HELPER_ENV_TOTAL, total.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("启动下载子进程失败");

    // 等子进程把断点写到磁盘（≥5% 且 <100%），再 TerminateProcess 强杀。
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut breakpoint_seen = 0u64;
    while Instant::now() < deadline {
        if let Ok(metadata) = std::fs::metadata(&partial) {
            let len = metadata.len();
            if len >= total / 20 && len < total {
                breakpoint_seen = len;
                break;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        breakpoint_seen > 0,
        "子进程未在 10s 内写出可续传的 .partial"
    );
    child.kill().expect("强制终止下载子进程失败");
    let status = child.wait().expect("等待子进程退出失败");
    assert!(!status.success(), "子进程应被强制终止而非正常退出");

    let bytes_after_kill = std::fs::metadata(&partial)
        .expect("进程被 Kill 后 .partial 必须保留")
        .len();
    assert!(
        bytes_after_kill >= breakpoint_seen && bytes_after_kill < total,
        "Kill 时断点应介于 0 与总大小之间：seen={breakpoint_seen}, after={bytes_after_kill}"
    );
    assert!(!dest.exists(), "Kill 后不应存在最终文件");

    // 新进程（父进程）直接续传：无需任何内存态，只读磁盘 .partial 长度。
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("父进程创建 tokio runtime 失败");
    let cancel = AtomicBool::new(false);
    let outcome = runtime
        .block_on(download_with_resume(
            &test_client(),
            &[server.url("/model.gguf")],
            &dest,
            total,
            &expected,
            &cancel,
            &fast_retry(),
            |_| {},
        ))
        .expect("杀进程后新进程应能续传成功");
    assert_eq!(outcome.bytes, total);
    assert_eq!(std::fs::read(&dest).expect("读取下载文件失败"), payload);
    let ranges = server.ranges();
    assert!(
        ranges
            .iter()
            .any(|range| matches!(range, Some(start) if *start > 0)),
        "杀进程后的续传请求必须从磁盘断点开始（带 Range）：{ranges:?}"
    );
    println!(
        "S-08 杀进程恢复：Kill 时断点 {} 字节（{:.1}%），新进程续传成功，请求记录 {ranges:?}",
        bytes_after_kill,
        bytes_after_kill as f64 / total as f64 * 100.0
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 11. 真实网络联调（#[ignore]，需联网；用本项目 reqwest 栈对 ModelScope 发 Range）
// ---------------------------------------------------------------------------

/// ModelScope 上 HY-MT1.5-1.8B-Q4_K_M.gguf 的实际下载 URL（与修正后的清单一致）。
const REAL_MODELSCOPE_URL: &str = "https://modelscope.cn/models/Tencent-Hunyuan/HY-MT1.5-1.8B-GGUF/resolve/master/HY-MT1.5-1.8B-Q4_K_M.gguf";
/// 修正前的清单 URL（小写文件名，历史 404 证据；清单已于 2026-11-04 由 Lead 修正）。
const REAL_MANIFEST_URL: &str = "https://modelscope.cn/models/Tencent-Hunyuan/HY-MT1.5-1.8B-GGUF/resolve/master/hy-mt-1.5-1.8b-q4_k_m.gguf";
const REAL_FILE_SIZE: u64 = 1_133_080_512;

/// 用真实网络验证：本项目 reqwest + native-tls 栈对 ModelScope 发 `Range: bytes=0-1023`
/// 能拿到 `206 + Content-Range`（不下载整模型）。`#[ignore]`：默认套件不联网。
///
/// 复跑：
/// `cargo test --manifest-path src-tauri/Cargo.toml --test model_download_spike real_modelscope -- --ignored --nocapture`
#[tokio::test]
#[ignore = "需要真实网络（ModelScope）；只取前 1KB，不下载整模型"]
async fn real_modelscope_range_probe() {
    let client = test_client();
    let started = Instant::now();
    let response = client
        .get(REAL_MODELSCOPE_URL)
        .header(reqwest::header::RANGE, "bytes=0-1023")
        .send()
        .await
        .expect("请求 ModelScope 失败（检查网络）");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::PARTIAL_CONTENT,
        "ModelScope 应返回 206（支持 Range）"
    );
    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_range.starts_with("bytes 0-1023/"),
        "缺少/错误的 Content-Range：{content_range:?}"
    );
    assert_eq!(
        content_range,
        format!("bytes 0-1023/{REAL_FILE_SIZE}"),
        "Content-Range 总量与官方文件大小不一致"
    );
    let body = response.bytes().await.expect("读取 Range 响应体失败");
    assert_eq!(body.len(), 1024, "Range 响应应恰好 1024 字节");
    let elapsed = started.elapsed();

    // 清单内 URL（占位、文件名与官方不一致）应 404 —— 反向证据。
    let manifest_status = client
        .get(REAL_MANIFEST_URL)
        .header(reqwest::header::RANGE, "bytes=0-1023")
        .send()
        .await
        .map(|response| response.status());
    println!(
        "S-08 真实 ModelScope Range 探测：HTTP 206，Content-Range: {content_range}，body=1024B，耗时 {:.3}s；清单 URL 状态：{manifest_status:?}",
        elapsed.as_secs_f64()
    );
}
