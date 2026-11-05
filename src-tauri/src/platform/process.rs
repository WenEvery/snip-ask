//! 本地推理进程管理（M12 / 说明书 §5.4 / 契约 v4 §1.3）。
//!
//! 职责（真实实现，不含任何 `unsafe`）：
//! 1. **端口探测**：`local.port` 起，在 `39281..=39289` 内挑可绑定端口（仅 `127.0.0.1`）；
//! 2. **启动 sidecar**：`std::process::Command` + `CREATE_NO_WINDOW`（不弹控制台），
//!    stdout/stderr 重定向到 `<data_dir>/logs/engine-<timestamp>.log`；
//! 3. **健康检查**：轮询 `GET /health`（llama-server 就绪返回 200），失败前先检测子进程早退；
//! 4. **进程树清理**：`taskkill /PID <pid> /T /F`（禁止按镜像名 taskkill，研究 §1.4）；
//! 5. **进程查询 / 日志读取**：`tasklist` 子进程（不用 Win32 API），日志只读尾部。
//!
//! 说明：`platform` 层需要系统信息时一律走**子进程**（`taskkill` / `tasklist`），
//! 因为 crate 级 `#![forbid(unsafe_code)]`。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::EngineStatus;

/// 固定端口段起点（研究 §1.3-3：39281–39289）。
pub const PORT_RANGE_START: u16 = 39281;
/// 固定端口段终点（含）。
pub const PORT_RANGE_END: u16 = 39289;
/// `/health` 轮询间隔。
pub const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// 单次 `/health` 请求超时（不等同于整体就绪超时）。
pub const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
/// 日志尾部读取上限（llama.cpp 日志 1.8B 模型约 100 KB，8 MiB 足够且防止异常放大）。
pub const LOG_READ_LIMIT_BYTES: u64 = 8 * 1024 * 1024;
/// `CREATE_NO_WINDOW`：spawn 时隐藏控制台窗口（避免闪黑框）。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------------------------------------------------------------------
// 引擎监管者 trait（保留，供平台侧抽象；真实实现见 services::local_engine）
// ---------------------------------------------------------------------------

/// 本地引擎监管者。
#[async_trait]
pub trait EngineSupervisor: Send + Sync {
    /// 当前状态。
    fn status(&self) -> EngineStatus;
    /// 启动引擎。
    async fn start(&self) -> AppResult<()>;
    /// 停止引擎。
    async fn stop(&self) -> AppResult<()>;
}

/// 停用实现：永远 `stopped`，`start` 返回 `SA-9001`（骨架模式默认行为）。
#[derive(Debug, Clone, Copy, Default)]
pub struct StoppedSupervisor;

#[async_trait]
impl EngineSupervisor for StoppedSupervisor {
    fn status(&self) -> EngineStatus {
        EngineStatus::Stopped
    }

    async fn start(&self) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::SkeletonNotImplemented,
            Some("TODO(real): 本地推理引擎启动未接入".to_string()),
        ))
    }

    async fn stop(&self) -> AppResult<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 端口探测
// ---------------------------------------------------------------------------

/// 端口是否可绑定（仅 `127.0.0.1`，不对外暴露）。
///
/// 注意：探测成功后立即释放监听，因此存在「探测与真正绑定之间被抢占」的理论竞态；
/// 本应用的端口段 39281–39289 专用于本地引擎，实际冲突概率极低，且启动失败会自动换下一个。
pub fn is_port_bindable(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// 候选端口序列（纯函数）：`local.port` 优先，其后按 39281→39289 升序补全。
///
/// - `preferred` 在端口段内 → 从 `preferred` 开始升序到 39289，再回绕到 39281；
/// - `preferred` 在端口段外（如用户把 `local.port` 改成 8080）→ 先试 `preferred`，
///   再依次试 39281–39289。
pub fn candidate_ports(preferred: u16) -> Vec<u16> {
    let mut ports = Vec::new();
    if preferred != 0 && !(PORT_RANGE_START..=PORT_RANGE_END).contains(&preferred) {
        ports.push(preferred);
    }
    let start = preferred.clamp(PORT_RANGE_START, PORT_RANGE_END);
    for port in start..=PORT_RANGE_END {
        ports.push(port);
    }
    for port in PORT_RANGE_START..start {
        ports.push(port);
    }
    ports
}

/// 挑一个可绑定的端口。
pub fn pick_port(preferred: u16) -> AppResult<u16> {
    for port in candidate_ports(preferred) {
        if is_port_bindable(port) {
            return Ok(port);
        }
    }
    Err(AppError::new(
        ErrorCode::IoFailed,
        Some(format!(
            "本地端口 {PORT_RANGE_START}-{PORT_RANGE_END} 均被占用（含配置端口 {preferred}）"
        )),
    )
    .with_message("本地端口被占用")
    .with_fallback(vec!["retry"]))
}

// ---------------------------------------------------------------------------
// 子进程启动 / 回收
// ---------------------------------------------------------------------------

/// 已启动的引擎子进程句柄。
#[derive(Debug)]
pub struct ServerProcess {
    /// 子进程句柄（`std::process::Child`）。
    pub child: Child,
    /// 进程 ID（`taskkill /PID` 用）。
    pub pid: u32,
    /// stdout/stderr 重定向目标。
    pub log_path: PathBuf,
}

impl ServerProcess {
    /// 可变子进程引用（健康检查时检测早退）。
    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// 已退出则返回退出码（未退出返回 `None`；异常退出码记 `-1`）。
    pub fn exit_code(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
            Ok(None) => None,
            Err(_) => Some(-1),
        }
    }

    /// 按 PID/进程树强制回收（幂等；禁止按镜像名 taskkill）。
    pub fn kill(&mut self) {
        kill_process_tree(self.pid);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 隐藏控制台窗口（Windows；非 Windows 平台为空操作）。
fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// 启动引擎子进程：stdout/stderr 合并重定向到 `log_path`，工作目录为 exe 所在目录
/// （llama.cpp 依赖同目录的 `ggml-*.dll`，工作目录不对会缺 DLL）。
pub fn spawn_server(
    exe: &Path,
    args: &[String],
    log_path: &Path,
    work_dir: Option<&Path>,
) -> AppResult<ServerProcess> {
    spawn_server_split(exe, args, log_path, log_path, work_dir)
}

/// 启动引擎子进程，stdout/stderr 写到 `stdio_log_path`，而 `log_path` 仅用于记录
/// 「引擎自身日志文件」路径（`--log-file` 构建：llama.cpp logger 直接写文件，
/// 避免 C stdio 块缓冲导致就绪时刻日志尚未落盘）。
pub fn spawn_server_split(
    exe: &Path,
    args: &[String],
    log_path: &Path,
    stdio_log_path: &Path,
    work_dir: Option<&Path>,
) -> AppResult<ServerProcess> {
    if !exe.is_file() {
        return Err(AppError::new(
            ErrorCode::ModelMissing,
            Some(format!("llama-server 可执行文件不存在：{}", exe.display())),
        )
        .with_message("缺少本地推理后端组件")
        .with_fallback(vec!["download_model", "cloud_translate"]));
    }
    if let Some(parent) = stdio_log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            AppError::io_failed(format!("创建日志目录 {} 失败：{err}", parent.display()))
        })?;
    }
    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(stdio_log_path)
        .map_err(|err| {
            AppError::io_failed(format!(
                "打开引擎日志 {} 失败：{err}",
                stdio_log_path.display()
            ))
        })?;
    let stderr = stdout
        .try_clone()
        .map_err(|err| AppError::io_failed(format!("复制引擎日志句柄失败：{err}")))?;

    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if let Some(dir) = work_dir {
        command.current_dir(dir);
    }
    hide_console(&mut command);

    let child = command.spawn().map_err(|err| {
        AppError::new(
            ErrorCode::ModelMissing,
            Some(format!("启动 {} 失败：{err}", exe.display())),
        )
        .with_message("本地推理引擎启动失败")
        .with_fallback(vec!["retry", "cloud_translate"])
    })?;
    let pid = child.id();
    tracing::info!(
        pid,
        exe = %exe.display(),
        args = ?args,
        log = %log_path.display(),
        stdio_log = %stdio_log_path.display(),
        "llama-server 子进程已启动"
    );
    Ok(ServerProcess {
        child,
        pid,
        log_path: log_path.to_path_buf(),
    })
}

/// 按 PID/进程树强制结束（`taskkill /PID <pid> /T /F`）。
///
/// **禁止按镜像名 taskkill**（研究 §1.4 踩坑：会误杀 LM Studio / 其他应用的同名进程）。
/// 进程不存在时 `taskkill` 返回非 0，属正常情况，仅记 debug。
pub fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    let mut command = Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T", "/F"]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
    match command.status() {
        Ok(status) => {
            tracing::debug!(pid, code = status.code(), "taskkill /PID /T /F 完成");
        }
        Err(err) => tracing::warn!(pid, error = %err, "taskkill 调用失败"),
    }
}

/// 等待进程从系统进程表消失（最多 `timeout`）；用于「停止后无残留」校验。
pub fn wait_process_gone(pid: u32, timeout: Duration) -> bool {
    let started = Instant::now();
    loop {
        if !is_process_alive(pid) {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 查询 llama-server 构建是否支持某个命令行开关（用 `--help` 输出判定，结果按 exe 缓存）。
///
/// 契约 §1.3：`--no-webui` 若该构建不支持则省略，以实测为准。这里用 `--help` 实测而非
/// 版本号猜测，避免把不支持的开关传给 llama.cpp 导致启动直接失败。
pub fn server_supports_flag(exe: &Path, flag: &str) -> bool {
    use once_cell::sync::OnceCell;
    use std::collections::HashMap;

    /// `<exe 路径>` → `--help` 全文（进程内缓存，避免每次启动都跑一次 `--help`）。
    static HELP_CACHE: OnceCell<std::sync::Mutex<HashMap<PathBuf, String>>> = OnceCell::new();
    let cache = HELP_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let key = exe.to_path_buf();
    let help = {
        let guard = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.get(&key).cloned()
    };
    let help = match help {
        Some(text) => text,
        None => {
            let mut command = Command::new(exe);
            command.arg("--help");
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            hide_console(&mut command);
            let text = match command.output() {
                Ok(output) => {
                    let mut merged = String::from_utf8_lossy(&output.stdout).into_owned();
                    merged.push_str(&String::from_utf8_lossy(&output.stderr));
                    merged
                }
                Err(err) => {
                    tracing::warn!(exe = %exe.display(), error = %err, "探测 --help 失败");
                    String::new()
                }
            };
            let mut guard = cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.insert(key, text.clone());
            text
        }
    };
    help.contains(flag)
}

// ---------------------------------------------------------------------------
// 进程查询（tasklist 子进程，不引入 Win32 unsafe）
// ---------------------------------------------------------------------------

/// 解析 `tasklist /FO CSV /NH` 输出中的 PID 集合（纯函数，便于单测）。
///
/// 输出示例：
/// ```text
/// "llama-server.exe","26124","Console","1","1,234 K"
/// ```
/// 无匹配时 Windows 中文系统输出 `信息: 没有运行的任务匹配指定标准。`（无 CSV 行）→ 空。
pub fn parse_tasklist_pids(stdout: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('"') {
            continue;
        }
        let fields: Vec<&str> = line.split("\",\"").collect();
        if fields.len() < 2 {
            continue;
        }
        // 第二个字段是 PID（首字段带前引号）。
        let raw = fields[1].trim_matches('"');
        if let Ok(pid) = raw.parse::<u32>() {
            pids.push(pid);
        }
    }
    pids
}

/// 按镜像名查询进程 PID（`tasklist /FI "IMAGENAME eq <image>"`）。
pub fn find_processes_by_image(image: &str) -> Vec<u32> {
    let mut command = Command::new("tasklist");
    command.args(["/FI", &format!("IMAGENAME eq {image}"), "/FO", "CSV", "/NH"]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut command);
    match command.output() {
        Ok(output) => parse_tasklist_pids(&String::from_utf8_lossy(&output.stdout)),
        Err(err) => {
            tracing::warn!(image, error = %err, "tasklist 调用失败");
            Vec::new()
        }
    }
}

/// 指定 PID 是否仍存活。
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let mut command = Command::new("tasklist");
    command.args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut command);
    match command.output() {
        Ok(output) => parse_tasklist_pids(&String::from_utf8_lossy(&output.stdout)).contains(&pid),
        Err(err) => {
            tracing::warn!(pid, error = %err, "tasklist 调用失败");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// 健康检查
// ---------------------------------------------------------------------------

/// `/health` 等待失败原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthError {
    /// 子进程在就绪前退出（携带退出码）。
    Exited(Option<i32>),
    /// 超时仍未就绪。
    Timeout {
        /// 实际等待毫秒数。
        waited_ms: u64,
    },
}

impl HealthError {
    /// 面向日志/错误 detail 的中文描述。
    pub fn describe(&self) -> String {
        match self {
            HealthError::Exited(Some(code)) => format!("进程提前退出（退出码 {code}）"),
            HealthError::Exited(None) => "进程提前退出".to_string(),
            HealthError::Timeout { waited_ms } => {
                format!("等待 /health 就绪超时（{waited_ms} ms）")
            }
        }
    }
}

/// 轮询 `GET {base_url}/health` 直到 200；返回从开始等待到就绪的毫秒数。
///
/// - 每 [`HEALTH_POLL_INTERVAL`] 轮询一次，单次请求超时 [`HEALTH_REQUEST_TIMEOUT`]；
/// - 轮询期间检测子进程早退（如缺 DLL / 参数非法 / 显存不足），立即返回错误；
/// - 超过 `timeout` 返回 [`HealthError::Timeout`]。
pub async fn wait_until_healthy(
    client: &reqwest::Client,
    base_url: &str,
    server: &mut ServerProcess,
    timeout: Duration,
) -> Result<u64, HealthError> {
    let started = Instant::now();
    let url = format!("{}/health", base_url.trim_end_matches('/'));
    loop {
        if let Some(code) = server.exit_code() {
            return Err(HealthError::Exited(Some(code)));
        }
        if let Ok(response) = client
            .get(&url)
            .timeout(HEALTH_REQUEST_TIMEOUT)
            .send()
            .await
        {
            if response.status().is_success() {
                return Ok(started.elapsed().as_millis() as u64);
            }
        }
        if started.elapsed() >= timeout {
            return Err(HealthError::Timeout {
                waited_ms: started.elapsed().as_millis() as u64,
            });
        }
        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
    }
}

// ---------------------------------------------------------------------------
// 日志
// ---------------------------------------------------------------------------

/// 读取引擎日志（超出上限只取尾部；非 UTF-8 字节用替换字符，解析容忍）。
pub fn read_log(path: &Path) -> String {
    read_log_tail(path, LOG_READ_LIMIT_BYTES)
}

/// 读取日志尾部 `max_bytes` 字节。
pub fn read_log_tail(path: &Path, max_bytes: u64) -> String {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return String::new(),
    };
    let limit = max_bytes as usize;
    let slice = if bytes.len() > limit {
        &bytes[bytes.len() - limit..]
    } else {
        &bytes[..]
    };
    String::from_utf8_lossy(slice).into_owned()
}

/// 构造健康检查 / 引擎探测用的 HTTP 客户端（本地回环，短超时）。
pub fn build_http_client(total_timeout: Duration) -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(total_timeout)
        .build()
        .map_err(|err| AppError::io_failed(format!("创建 HTTP 客户端失败：{err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_ports_start_at_preferred_and_cover_range() {
        let ports = candidate_ports(39283);
        assert_eq!(ports.first().copied(), Some(39283));
        assert_eq!(ports.last().copied(), Some(39282));
        assert_eq!(ports.len(), 9);
        assert!(ports.contains(&39281));
        assert!(ports.contains(&39289));
    }

    #[test]
    fn candidate_ports_outside_range_are_tried_first() {
        let ports = candidate_ports(8080);
        assert_eq!(ports.first().copied(), Some(8080));
        assert_eq!(ports[1], 39281);
        assert_eq!(ports.len(), 10);
        // 0 不作为候选（0 = 由系统分配，无法用于固定端点）。
        assert!(!candidate_ports(0).contains(&0));
        assert_eq!(candidate_ports(0).len(), 9);
    }

    #[test]
    fn parses_tasklist_csv_pids() {
        let stdout = "\"llama-server.exe\",\"26124\",\"Console\",\"1\",\"1,234 K\"\r\n\
                      \"llama-server.exe\",\"4100\",\"Console\",\"1\",\"2,048 K\"\r\n";
        assert_eq!(parse_tasklist_pids(stdout), vec![26124, 4100]);
        assert!(parse_tasklist_pids("信息: 没有运行的任务匹配指定标准。").is_empty());
        assert!(parse_tasklist_pids("").is_empty());
    }

    #[test]
    fn reads_log_tail_only() {
        let path =
            std::env::temp_dir().join(format!("snip-ask-process-log-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, "old-line\nnew-line\n").unwrap();
        assert_eq!(read_log_tail(&path, 4), "ine\n");
        assert_eq!(read_log(&path), "old-line\nnew-line\n");
        assert_eq!(read_log_tail(&path, 0).len(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pick_port_returns_bindable_port() {
        let port = pick_port(39281).expect("本地端口段应至少有一个可用");
        assert!(is_port_bindable(port));
        assert!((PORT_RANGE_START..=PORT_RANGE_END).contains(&port));
    }

    #[tokio::test]
    async fn spawn_health_and_kill_real_process() {
        // 用 `cmd.exe /c ping` 模拟一个「会一直运行、没有 /health」的 sidecar：
        // 只验证 spawn / 日志重定向 / PID 存活判定 / 进程树回收链路，不依赖 llama-server。
        let log_path = std::env::temp_dir().join(format!(
            "snip-ask-process-spawn-{}.log",
            uuid::Uuid::new_v4()
        ));
        let exe = Path::new("cmd.exe");
        if !exe.is_file() && std::env::var_os("ComSpec").is_none() {
            eprintln!("跳过：找不到 cmd.exe");
            return;
        }
        let exe = std::env::var("ComSpec")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("cmd.exe"));
        let args = vec![
            "/c".to_string(),
            "echo".to_string(),
            "snip-ask-spawn-ok".to_string(),
            "&".to_string(),
            "ping".to_string(),
            "-n".to_string(),
            "20".to_string(),
            "127.0.0.1".to_string(),
        ];
        let mut server = spawn_server(&exe, &args, &log_path, None).expect("spawn 失败");
        // 子进程确实在运行。
        std::thread::sleep(Duration::from_millis(300));
        assert!(is_process_alive(server.pid));
        // 日志重定向生效。
        let log = read_log(&log_path);
        assert!(log.contains("snip-ask-spawn-ok"), "日志内容：{log}");
        assert!(!log.trim().is_empty());
        // 健康检查：不存在 /health → 超时（同时也验证了不会一直阻塞）。
        let client = build_http_client(Duration::from_secs(2)).unwrap();
        let error = wait_until_healthy(
            &client,
            "http://127.0.0.1:1",
            &mut server,
            Duration::from_millis(400),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, HealthError::Timeout { .. }));
        // PID 树回收后无残留。
        server.kill();
        assert!(wait_process_gone(server.pid, Duration::from_secs(5)));
        let _ = std::fs::remove_file(&log_path);
    }
}
