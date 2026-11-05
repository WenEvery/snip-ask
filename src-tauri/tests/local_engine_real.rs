//! T2 真机验证：真实 `llama-server`（Vulkan 优先）+ 真实 `/v1/chat/completions` 推理。
//!
//! 本文件是**真机证据产出**（契约 §0 铁律 1：不接受 mock 充当验收），默认跳过，
//! 只有显式设置 `SNIP_ASK_REAL_ENGINE=1` 时才执行；需要：
//! - `SNIP_ASK_BACKENDS_DIR`：后端根目录（`<root>/vulkan/llama-server.exe`、
//!   `<root>/cpu/llama-server.exe`），见 spec §1.2 / §6-10；
//! - `SNIP_ASK_MODEL_PATH`：真实 GGUF（本机为
//!   `.verify/spikes/S-05-S-06/artifacts/HY-MT1.5-1.8B-Q4_K_M.gguf`）。
//!
//! 覆盖（spec §5 真机 GPU 验收项）：
//! 1. Vulkan 真实启动（`-ngl 99 --device Vulkan0`），记录 `/health` 就绪耗时；
//! 2. 日志出现 offload 行 + `using device Vulkan0` + 设备名；
//! 3. `/v1/chat/completions` 真实推理一次（SSE 流式），记录输出 / 时延 / usage；
//! 4. `nvidia-smi` 显存佐证（**仅佐证**，不作为可用判据）；
//! 5. 停止后按 PID 校验无残留；
//! 6. CPU 回退路径真实启动 + 推理一次。
//!
//! 运行（PowerShell，仓库根）：
//! ```powershell
//! $env:SNIP_ASK_REAL_ENGINE='1'
//! $env:SNIP_ASK_BACKENDS_DIR="$env:TEMP\snipask-verify\backends"
//! $env:SNIP_ASK_MODEL_PATH=".verify/spikes/S-05-S-06/artifacts/HY-MT1.5-1.8B-Q4_K_M.gguf"
//! cd src-tauri; cargo test --offline --test local_engine_real -- --nocapture --test-threads=1
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use snip_ask_lib::contract::config::{Config, GpuBackend, LocalBackend};
use snip_ask_lib::contract::types::{
    ActionId, GpuBackendStatus, GpuDevice, GpuProbeResult, Mode, SessionId, StreamKind, TaskId,
};
use snip_ask_lib::platform::gpu::backend_exe_path;
use snip_ask_lib::platform::process::{
    self, find_processes_by_image, is_process_alive, wait_until_healthy,
};
use snip_ask_lib::services::llm_gateway::{LlmGateway, LlmRequest, StreamChunk};
use snip_ask_lib::services::local_engine::{
    apply_log_args, build_args_with_context, log_tail_lines, parse_device_line, parse_device_name,
    parse_offload, plan_attempts, read_engine_log, spawn_engine_server, EngineAttempt,
};
use snip_ask_lib::services::local_gateway::HttpLlmGateway;

const ENABLE_ENV: &str = "SNIP_ASK_REAL_ENGINE";
const BACKENDS_ENV: &str = "SNIP_ASK_BACKENDS_DIR";
const MODEL_ENV: &str = "SNIP_ASK_MODEL_PATH";
const DEFAULT_MODEL_NAME: &str = "hy-mt-1.8b-q4";

// ---------------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------------

fn enabled() -> bool {
    let on = std::env::var(ENABLE_ENV).ok().as_deref() == Some("1");
    if !on {
        eprintln!("跳过真机验证：未设置 {ENABLE_ENV}=1");
    }
    on
}

fn evidence_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".verify")
        .join("evidence")
        .join("local-engine");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn backends_root() -> Option<PathBuf> {
    let raw = std::env::var(BACKENDS_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

fn model_path() -> Option<PathBuf> {
    let raw = std::env::var(MODEL_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("跳过：{MODEL_ENV} 指向的文件不存在：{}", path.display());
        None
    }
}

fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn run_command(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            text
        }
        Err(err) => format!("<{program} 调用失败：{err}>"),
    }
}

fn nvidia_smi_sample() -> String {
    run_command(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.used,memory.total,utilization.gpu",
            "--format=csv,noheader",
        ],
    )
    .trim()
    .to_string()
}

fn list_devices(exe: &Path) -> String {
    match Command::new(exe).arg("--list-devices").output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            text
        }
        Err(err) => format!("<--list-devices 失败：{err}>"),
    }
}

fn version(exe: &Path) -> String {
    match Command::new(exe).arg("--version").output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            text.lines().next().unwrap_or_default().to_string()
        }
        Err(err) => format!("<--version 失败：{err}>"),
    }
}

/// 把引擎主日志与 stdout sidecar 复制到 evidence 目录（证据可复跑查看）。
fn copy_engine_logs(source: &Path, target: &Path) {
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if source.is_file() {
        let _ = std::fs::copy(source, target);
    }
    let sidecar = PathBuf::from(format!("{}{}", source.display(), ".stdout.log"));
    if sidecar.is_file() {
        let sidecar_target = PathBuf::from(format!("{}.stdout.log", target.display()));
        let _ = std::fs::copy(&sidecar, sidecar_target);
    }
}

/// 持有子进程：断言 panic 时也会回收，避免留下 llama-server 残留。
struct RunningServer {
    server: Option<process::ServerProcess>,
}

impl RunningServer {
    fn new(server: process::ServerProcess) -> Self {
        Self {
            server: Some(server),
        }
    }

    fn current(&mut self) -> &mut process::ServerProcess {
        self.server.as_mut().expect("子进程已被回收")
    }

    fn kill(&mut self) {
        if let Some(mut server) = self.server.take() {
            server.kill();
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.kill();
    }
}

/// 执行一次真实流式推理，返回（正文、思考、耗时、usage）。
async fn infer_once(base_url: &str, prompt: &str) -> (String, String, u64, (u64, u64), u64) {
    let gateway = HttpLlmGateway::new(base_url, None, DEFAULT_MODEL_NAME);
    let request = LlmRequest {
        task: TaskId::from("real-engine-task"),
        session: SessionId::from("real-engine-session"),
        action: ActionId::Translate,
        mode: Mode::Quick,
        engine: "local".to_string(),
        prompt: prompt.to_string(),
        input_text: prompt.to_string(),
        image_data_url: None,
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let text = Arc::new(parking_lot::Mutex::new(String::new()));
    let reasoning = Arc::new(parking_lot::Mutex::new(String::new()));
    let sink_text = text.clone();
    let sink_reasoning = reasoning.clone();
    let sink = move |chunk: StreamChunk| match chunk.kind {
        StreamKind::Text => sink_text.lock().push_str(&chunk.chunk),
        StreamKind::Reasoning => sink_reasoning.lock().push_str(&chunk.chunk),
        StreamKind::Meta => {}
    };
    let started = Instant::now();
    let outcome = gateway
        .stream(&request, &sink)
        .await
        .expect("真实 /v1/chat/completions 推理失败");
    let latency = started.elapsed().as_millis() as u64;
    let text = text.lock().clone();
    let reasoning = reasoning.lock().clone();
    (
        text,
        reasoning,
        latency,
        (outcome.usage.input_tokens, outcome.usage.output_tokens),
        outcome.latency_ms,
    )
}

/// 合成探测结果：仅用于让 `plan_attempts` 生成 Vulkan/CPU 链（真机断言不依赖 T1 探测实现）。
fn synthetic_probe(backend: LocalBackend) -> GpuProbeResult {
    let vulkan = backend == LocalBackend::Vulkan;
    let cpu = backend == LocalBackend::Cpu;
    let backends = vec![
        GpuBackendStatus {
            backend: LocalBackend::Vulkan,
            installed: vulkan,
            available: vulkan,
            exe_path: None,
            version: None,
            reason: None,
        },
        GpuBackendStatus {
            backend: LocalBackend::Cuda,
            installed: false,
            available: false,
            exe_path: None,
            version: None,
            reason: Some("未安装 cuda 后端组件".to_string()),
        },
        GpuBackendStatus {
            backend: LocalBackend::Cpu,
            installed: cpu,
            available: cpu,
            exe_path: None,
            version: None,
            reason: None,
        },
    ];
    let mut devices = Vec::new();
    if vulkan {
        devices.push(GpuDevice {
            index: 0,
            name: "NVIDIA GeForce RTX 4060 Laptop GPU".to_string(),
            backend: LocalBackend::Vulkan,
            vram_total_mb: 7956,
            vram_free_mb: 7180,
            vendor: Some("nvidia".to_string()),
            virtual_adapter: false,
        });
    }
    GpuProbeResult {
        checked_at: chrono::Utc::now().to_rfc3339(),
        cached: false,
        useable: vulkan,
        recommended: if vulkan {
            Some(GpuBackend::Vulkan)
        } else {
            None
        },
        devices,
        backends,
        budget: None,
        issue: if vulkan {
            None
        } else {
            Some("合成测试探测：仅 CPU".to_string())
        },
    }
}

/// 真实启动一次引擎并跑一次推理，返回证据 JSON。
async fn run_real_engine(backend: LocalBackend) -> serde_json::Value {
    let Some(root) = backends_root() else {
        panic!("缺少 {BACKENDS_ENV}");
    };
    let Some(model) = model_path() else {
        panic!("缺少 {MODEL_ENV}");
    };
    let variant = backend.as_str();
    let candidate = backend_exe_path(&root, backend);
    if !candidate.is_file() {
        panic!("未找到 {variant} 后端：{}", candidate.display());
    }
    let exe = candidate;
    eprintln!("使用后端 exe：{}", exe.display());
    eprintln!("使用模型：{}", model.display());

    // 0) 记录启动前已有的 llama-server 进程（用于「无残留」断言）。
    let before: Vec<u32> = find_processes_by_image("llama-server.exe");

    // 1) 规划 attempt（Vulkan → CPU 链；CPU 用例只取 CPU）。
    let config = Config::default();
    let probe = synthetic_probe(backend);
    let attempts = plan_attempts(&config, &probe);
    let attempt: EngineAttempt = attempts
        .iter()
        .find(|item| item.backend == backend)
        .cloned()
        .unwrap_or_else(|| panic!("plan_attempts 未包含 {variant}"));
    eprintln!("attempt = {attempt:?}");

    // 2) 参数：显式 -ngl / --device / -c + 日志策略（--log-file / -lv 4，见实测注释）。
    let port = process::pick_port(config.local.port).expect("挑端口失败");
    let threads = 8u32;
    let stamp = timestamp();
    let evidence = evidence_dir();
    // 引擎日志先写 ASCII 临时目录：部分 llama.cpp 构建的 `--log-file` 无法打开非 ASCII 路径
    // （本仓库路径含中文），此时日志会退回流式 stdout 捕获；证据完成后复制回 evidence 目录。
    let log_dir = std::env::temp_dir().join(format!("snipask-engine-{variant}-{stamp}"));
    std::fs::create_dir_all(&log_dir).expect("创建日志目录失败");
    let log_path = log_dir.join(format!("engine-{variant}.log"));
    let evidence_log_path = evidence.join(format!("engine-{variant}-{stamp}.log"));
    let mut args = build_args_with_context(&attempt, &model, port, threads, 4096);
    if process::server_supports_flag(&exe, "--no-webui") {
        args.push("--no-webui".to_string());
    }
    apply_log_args(&exe, &mut args, &log_path);
    eprintln!("端口 = {port}");
    eprintln!("参数 = {args:?}");

    // 3) 启动 + /health 就绪耗时。
    let started = Instant::now();
    let server = spawn_engine_server(&exe, &args, &log_path, exe.parent())
        .unwrap_or_else(|err| panic!("spawn 失败：{err}"));
    let pid = server.pid;
    let mut running = RunningServer::new(server);
    let base_url = format!("http://127.0.0.1:{port}");
    let client = process::build_http_client(Duration::from_secs(10)).expect("HTTP 客户端");
    let health_timeout = Duration::from_secs(
        std::env::var("SNIP_ASK_ENGINE_READY_TIMEOUT_SEC")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(20),
    );
    let health_ready_ms = wait_until_healthy(&client, &base_url, running.current(), health_timeout)
        .await
        .unwrap_or_else(|err| {
            panic!(
                "{variant} 引擎未就绪：{}；日志尾部：{}",
                err.describe(),
                log_tail_lines(&read_engine_log(&log_path), 8)
            )
        });
    let total_ready_ms = started.elapsed().as_millis() as u64;
    eprintln!("/health 就绪：{health_ready_ms} ms（含 spawn：{total_ready_ms} ms）");
    assert_eq!(
        client
            .get(format!("{base_url}/health"))
            .send()
            .await
            .expect("再次 /health 失败")
            .status()
            .as_u16(),
        200
    );

    // 4) 日志证据：offload 行 / using device 行 / 设备名。
    let log = read_engine_log(&log_path);
    let offload = parse_offload(&log);
    let device_line = parse_device_line(&log);
    let device_name = parse_device_name(&log);
    eprintln!("offload = {offload:?}");
    eprintln!("device_line = {device_line:?}");
    eprintln!("device_name = {device_name:?}");
    if backend == LocalBackend::Vulkan {
        let (loaded, total) = offload.expect("Vulkan 日志必须出现 offloaded X/Y layers to GPU");
        assert!(
            loaded > 0,
            "Vulkan 必须真实 offload 层数 > 0，实际 {loaded}/{total}"
        );
        assert!(
            device_line
                .as_deref()
                .is_some_and(|token| token.starts_with("Vulkan")),
            "日志必须出现 using device Vulkan*，实际 {device_line:?}"
        );
    }

    // 5) nvidia-smi 显存佐证（仅佐证；失败静默）。
    let vram_idle = nvidia_smi_sample();

    // 6) 真实推理（产品 translate 提示词风格）。
    let prompt = "把下面这句话翻译成英文，只输出译文：人工智能正在改变世界。";
    let (output, reasoning, latency_ms, usage, gateway_latency_ms) =
        infer_once(&base_url, prompt).await;
    eprintln!("推理输出 = {output}");
    eprintln!("推理耗时 = {latency_ms} ms（网关内部 {gateway_latency_ms} ms），usage = {usage:?}");
    assert!(!output.trim().is_empty(), "真实推理输出不得为空");
    let vram_infer = nvidia_smi_sample();

    // 7) 停止 + 无残留校验（按 PID，不按镜像名）。
    running.kill();
    let gone = process::wait_process_gone(pid, Duration::from_secs(10));
    assert!(gone, "停止后 PID {pid} 仍存活（有残留进程）");
    assert!(!is_process_alive(pid), "停止后 PID {pid} 不应存活");
    let after: Vec<u32> = find_processes_by_image("llama-server.exe");
    let residual: Vec<u32> = after
        .iter()
        .copied()
        .filter(|candidate| !before.contains(candidate))
        .collect();
    assert!(
        residual.is_empty(),
        "停止后出现残留 llama-server 进程：{residual:?}（启动前：{before:?}）"
    );

    // 8) 证据落盘：原始日志复制到 evidence，再写 JSON。
    // 进程退出后再读一次日志（`--log-file` 构建的日志落盘完整；stdout 侧为尽力而为）。
    let final_log = read_engine_log(&log_path);
    copy_engine_logs(&log_path, &evidence_log_path);
    let evidence_json = serde_json::json!({
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "kind": "real-llama-server",
        "backend": variant,
        "exe": exe.display().to_string(),
        "exe_version": version(&exe),
        "model": model.display().to_string(),
        "model_size_bytes": std::fs::metadata(&model).map(|meta| meta.len()).unwrap_or(0),
        "list_devices": list_devices(&exe),
        "attempt": {
            "backend": attempt.backend.as_str(),
            "device": attempt.device,
            "gpu_layers": attempt.gpu_layers,
            "label": attempt.label,
        },
        "port": port,
        "args": args,
        "health_ready_ms": health_ready_ms,
        "total_ready_ms": total_ready_ms,
        "health_status": 200,
        "log_path": evidence_log_path.display().to_string(),
        "log_evidence": {
            "offload": offload.map(|(loaded, total)| serde_json::json!({"loaded": loaded, "total": total})),
            "device_line": device_line,
            "device_name": device_name,
            "grep_offload_line": final_log.lines().find(|line| line.contains("offloaded")).map(str::to_string),
            "grep_using_device_line": final_log.lines().find(|line| line.contains("using device")).map(str::to_string),
            "log_bytes": final_log.len(),
        },
        "inference": {
            "endpoint": format!("{base_url}/v1/chat/completions"),
            "prompt": prompt,
            "output": output,
            "reasoning": reasoning,
            "latency_ms": latency_ms,
            "gateway_latency_ms": gateway_latency_ms,
            "usage": {"input_tokens": usage.0, "output_tokens": usage.1},
        },
        "vram_smi_idle": vram_idle,
        "vram_smi_during_inference": vram_infer,
        "pid": pid,
        "residual_check": {
            "pid_alive_after_stop": is_process_alive(pid),
            "llama_server_pids_before": before,
            "llama_server_pids_after": after,
            "residual": residual,
        },
    });
    let json_path = evidence.join(format!("real-{variant}-{stamp}.json"));
    std::fs::write(
        &json_path,
        serde_json::to_string_pretty(&evidence_json).unwrap_or_default(),
    )
    .expect("写证据 JSON 失败");
    eprintln!("证据：{}", json_path.display());
    evidence_json
}

// ---------------------------------------------------------------------------
// 用例（默认跳过，需 SNIP_ASK_REAL_ENGINE=1）
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn real_vulkan_engine_inference_and_cleanup() {
    if !enabled() || backends_root().is_none() || model_path().is_none() {
        return;
    }
    let evidence = run_real_engine(LocalBackend::Vulkan).await;
    assert_eq!(evidence["backend"], "vulkan");
    let loaded = evidence["log_evidence"]["offload"]["loaded"]
        .as_u64()
        .unwrap_or(0);
    assert!(loaded > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn real_cpu_fallback_engine_inference_and_cleanup() {
    if !enabled() || backends_root().is_none() || model_path().is_none() {
        return;
    }
    let evidence = run_real_engine(LocalBackend::Cpu).await;
    assert_eq!(evidence["backend"], "cpu");
    // CPU 路径必须显式 -ngl 0（研究 §1.2 踩坑）。
    let args = evidence["args"]
        .as_array()
        .expect("args 必须是数组")
        .iter()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect::<Vec<String>>();
    let position = args
        .iter()
        .position(|arg| arg == "-ngl")
        .expect("CPU 参数必须包含 -ngl");
    assert_eq!(args[position + 1], "0");
    assert!(
        !args.iter().any(|arg| arg == "--device"),
        "CPU 路径不应传 --device"
    );
}
