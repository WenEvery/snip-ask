//! 本地推理引擎监督者（M12 / 研究 §1.3-2 / 契约 v4）。
//!
//! 职责：按 `local.device` / `local.gpuBackend` 拼装 llama.cpp 参数、启动 `llama-server`
//! 子进程、健康检查、日志解析、空闲卸载、**按 PID/进程树**清理（禁止按镜像名 taskkill）。
//!
//! **Vulkan 优先的回退链**（研究 §1.3-2）：
//! `gpuBackend=auto` → Vulkan → CUDA → CPU；`vulkan` → Vulkan → CPU；`cuda` → CUDA → Vulkan → CPU。
//! 每个 attempt 失败后记录原因并继续下一个，最终 `engine:status` 携带 `fallbackReason`。
//!
//! 关键约束：
//! - **显式 `-ngl`**（研究 §1.2 踩坑）：CPU `-ngl 0`，GPU `-ngl <local.gpuLayers>`；
//! - `--device Vulkan0|CUDA0` 显式指定设备；设备判定只认 llama.cpp 自身输出，**不调用 nvidia-smi**；
//! - 工具链禁止 `unsafe`：进程操作全部走 [`crate::platform::process`] 子进程；
//! - 开发/验证钩子：`SNIP_ASK_BACKENDS_DIR`（后端目录）、`SNIP_ASK_MODEL_PATH`（GGUF 直连）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tauri::{AppHandle, Manager};

use crate::contract::config::{Config, GpuBackend, LocalBackend, LocalDevice};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::EngineStatusPayload;
use crate::contract::types::{EngineStatus, GpuProbeResult};
use crate::domain::models::{find_entry, parse_manifest};
use crate::events::emit_engine_status_payload;
use crate::platform::gpu;
use crate::platform::paths::AppPaths;
use crate::platform::process;
use crate::services::backend_manager::BACKENDS_DIR_ENV;
use crate::services::model_manager;

/// `/health` 就绪默认超时（契约 §1.3：20 s）。
pub const DEFAULT_HEALTH_TIMEOUT_SEC: u64 = 20;
/// 开发钩子：启动时直接指向 GGUF（验证/调试用，产品文档标注为调试用）。
pub const MODEL_PATH_ENV: &str = "SNIP_ASK_MODEL_PATH";
/// 开发钩子：覆盖就绪超时（秒）；仅用于慢速 GPU / 冷启动验证。
pub const HEALTH_TIMEOUT_ENV: &str = "SNIP_ASK_ENGINE_READY_TIMEOUT_SEC";
/// 默认上下文窗口（清单 `params.context` 缺失时使用）。
pub const DEFAULT_CONTEXT: u32 = 4096;
/// 空闲卸载定时器轮询间隔。
const IDLE_TICK: Duration = Duration::from_secs(2);
/// 停止后等待进程树消失的上限。
const PROCESS_GONE_TIMEOUT: Duration = Duration::from_secs(5);

/// 一次启动尝试（纯数据，可单测）。
#[derive(Debug, Clone, PartialEq)]
pub struct EngineAttempt {
    /// 后端（含 CPU）。
    pub backend: LocalBackend,
    /// 设备序号（`--device Vulkan0` 的 `0`）；CPU 为 `None`。
    pub device: Option<u32>,
    /// offload 层数（CPU 为 0）。
    pub gpu_layers: u32,
    /// 面向日志的标签（`vulkan:Vulkan0` / `cpu`）。
    pub label: String,
}

/// 引擎运行态快照。
#[derive(Debug, Clone, Default)]
pub struct EngineRuntimeStatus {
    /// 状态。
    pub status: EngineStatus,
    /// 设备（解析结果：`cpu` / `gpu`）。
    pub device: Option<LocalDevice>,
    /// 实际后端。
    pub backend: Option<LocalBackend>,
    /// 回退原因。
    pub fallback_reason: Option<String>,
    /// 实际 offload 层数。
    pub offload_layers: Option<u32>,
    /// 设备名。
    pub device_name: Option<String>,
    /// 监听端口。
    pub port: Option<u16>,
    /// 基础 URL（`http://127.0.0.1:<port>`，**不含 `/v1`**；`HttpLlmGateway` 内部自行拼
    /// `/v1/chat/completions`）。
    pub base_url: Option<String>,
    /// 子进程 PID（诊断 / 证据用）。
    pub pid: Option<u32>,
    /// 引擎日志路径（诊断 / 证据用）。
    pub log_path: Option<String>,
}

impl EngineRuntimeStatus {
    /// 转为 `engine:status` 载荷。
    pub fn to_payload(&self) -> EngineStatusPayload {
        EngineStatusPayload {
            local: self.status,
            device: self.device,
            backend: self.backend,
            fallback_reason: self.fallback_reason.clone(),
            offload_layers: self.offload_layers,
            device_name: self.device_name.clone(),
            port: self.port,
        }
    }
}

/// 引擎内部运行态（含子进程句柄；不对外）。
#[derive(Debug)]
struct EngineRuntime {
    /// 对外快照。
    status: EngineRuntimeStatus,
    /// 子进程（`None` = 未运行）。
    child: Option<process::ServerProcess>,
    /// 最后一次使用时间（空闲卸载依据：每次 `ensure_ready` 刷新）。
    last_used: Instant,
    /// 当前运行参数指纹：配置变化时重启引擎。
    spawn_key: String,
}

impl Default for EngineRuntime {
    fn default() -> Self {
        Self {
            status: EngineRuntimeStatus::default(),
            child: None,
            last_used: Instant::now(),
            spawn_key: String::new(),
        }
    }
}

/// 本地引擎监督者。
#[derive(Debug)]
pub struct LocalEngine {
    /// 应用数据路径。
    paths: AppPaths,
    /// 运行态。
    runtime: Mutex<EngineRuntime>,
    /// 串行化启动（避免并发请求各 spawn 一个 llama-server）。
    start_lock: tokio::sync::Mutex<()>,
    /// 空闲卸载定时器是否已启动（进程内只启动一次）。
    idle_timer_started: AtomicBool,
}

impl LocalEngine {
    /// 新建（不启动）。
    pub fn new(paths: AppPaths) -> Self {
        Self {
            paths,
            runtime: Mutex::new(EngineRuntime::default()),
            start_lock: tokio::sync::Mutex::new(()),
            idle_timer_started: AtomicBool::new(false),
        }
    }

    /// 当前状态快照。
    pub fn snapshot(&self) -> EngineStatusPayload {
        let mut runtime = self.runtime.lock();
        Self::refresh_liveness(&mut runtime);
        runtime.status.to_payload()
    }

    /// 当前状态。
    pub fn status(&self) -> EngineStatus {
        let mut runtime = self.runtime.lock();
        Self::refresh_liveness(&mut runtime);
        runtime.status.status
    }

    /// 是否已就绪。
    pub fn is_ready(&self) -> bool {
        self.status() == EngineStatus::Ready
    }

    /// 就绪时的 OpenAI 兼容基础 URL（`http://127.0.0.1:<port>`，不含 `/v1`）。
    pub fn base_url(&self) -> Option<String> {
        let mut runtime = self.runtime.lock();
        Self::refresh_liveness(&mut runtime);
        if runtime.status.status == EngineStatus::Ready {
            runtime.status.base_url.clone()
        } else {
            None
        }
    }

    /// 当前子进程 PID（证据 / 诊断用）。
    pub fn pid(&self) -> Option<u32> {
        self.runtime.lock().status.pid
    }

    /// 当前引擎日志路径（证据 / 诊断用）。
    pub fn log_path(&self) -> Option<String> {
        self.runtime.lock().status.log_path.clone()
    }

    /// 记录一次「使用」，推迟空闲卸载。
    pub fn note_activity(&self) {
        self.runtime.lock().last_used = Instant::now();
    }

    /// 确保引擎可用（已就绪且参数未变直接复用；否则按配置启动）。
    pub async fn ensure_ready(
        &self,
        app: &AppHandle,
        config: &Config,
    ) -> AppResult<EngineStatusPayload> {
        self.note_activity();
        let _guard = self.start_lock.lock().await;
        self.note_activity();
        let key = spawn_key(config);
        {
            let mut runtime = self.runtime.lock();
            Self::refresh_liveness(&mut runtime);
            if runtime.status.status == EngineStatus::Ready && runtime.spawn_key == key {
                tracing::debug!(backend = ?runtime.status.backend, "复用已就绪的本地引擎");
                return Ok(runtime.status.to_payload());
            }
        }
        self.start_chain(app, config).await
    }

    /// 显式启动（设置页「试译 / 预热」；失败返回 SA-4001 / SA-4002）。
    pub async fn start(&self, app: &AppHandle, config: &Config) -> AppResult<EngineStatusPayload> {
        self.ensure_ready(app, config).await
    }

    /// 停止并回收子进程（按 PID/进程树）。
    pub async fn stop(&self) -> AppResult<()> {
        self.stop_blocking()
    }

    /// 同步停止（生命周期 `shutdown` 中调用；不依赖 tokio 运行时）。
    pub fn stop_blocking(&self) -> AppResult<()> {
        let child = {
            let mut runtime = self.runtime.lock();
            runtime.child.take()
        };
        if let Some(mut server) = child {
            let pid = server.pid;
            server.kill();
            if !process::wait_process_gone(pid, PROCESS_GONE_TIMEOUT) {
                tracing::warn!(pid, "本地引擎停止后进程仍残留（已尽力回收）");
            } else {
                tracing::info!(pid, "本地引擎已停止（进程树已回收）");
            }
        }
        let mut runtime = self.runtime.lock();
        runtime.status = EngineRuntimeStatus {
            status: EngineStatus::Stopped,
            ..EngineRuntimeStatus::default()
        };
        runtime.spawn_key.clear();
        runtime.last_used = Instant::now();
        Ok(())
    }

    /// 检测子进程是否意外退出并同步状态（崩溃 → `failed`）。
    fn refresh_liveness(runtime: &mut EngineRuntime) {
        if runtime.status.status != EngineStatus::Ready {
            return;
        }
        let Some(child) = runtime.child.as_mut() else {
            runtime.status.status = EngineStatus::Failed;
            runtime.status.base_url = None;
            runtime.status.fallback_reason = Some("本地推理进程句柄丢失，已标记为失败".to_string());
            return;
        };
        if let Some(code) = child.exit_code() {
            tracing::warn!(pid = runtime.status.pid, code, "本地推理进程已退出");
            runtime.status.status = EngineStatus::Failed;
            runtime.status.base_url = None;
            runtime.status.fallback_reason = Some(format!(
                "本地推理进程意外退出（退出码 {code}）；请重新启动引擎"
            ));
            runtime.child = None;
        }
    }

    /// 更新状态并广播 `engine:status`；同时镜像到 `AppState.engine`（设置页 / `app_stats` 一致）。
    fn publish(&self, app: &AppHandle) {
        let payload = self.snapshot();
        emit_engine_status_payload(app, &payload);
        // 说明：`services` 通常不感知 `app`，此处仅为同步 `AppState.engine` 镜像
        // （Lead 在 task-2 集成说明中要求），不做任何业务逻辑。
        if let Some(state) = app.try_state::<crate::app::state::AppState>() {
            *state.engine.write() = payload.local;
        }
    }

    /// 记录「启动中」并回收旧进程（配置变更重启路径）。
    fn mark_starting(&self, config: &Config) {
        let old = {
            let mut runtime = self.runtime.lock();
            let old = runtime.child.take();
            runtime.status = EngineRuntimeStatus {
                status: EngineStatus::Starting,
                ..EngineRuntimeStatus::default()
            };
            runtime.spawn_key = spawn_key(config);
            runtime.last_used = Instant::now();
            old
        };
        if let Some(mut server) = old {
            server.kill();
        }
    }

    /// 按回退链逐个 attempt 启动，全部失败返回可操作错误。
    async fn start_chain(
        &self,
        app: &AppHandle,
        config: &Config,
    ) -> AppResult<EngineStatusPayload> {
        // 1) 模型先解析：缺失时直接 SA-4001，不必逐个 attempt 重复尝试。
        let model_path = self.resolve_model_path(config)?;
        self.mark_starting(config);
        self.publish(app);

        // 2) 真实探测（`--list-devices`）+ 用磁盘上真实存在的 exe 校正 `installed`，
        //    让后端组件「已安装但探测失败」与「组件缺失」两种情况都不阻塞回退链。
        let roots = self.backends_roots(app);
        let model_vram_mb = self.model_vram_mb(config);
        let mut probe = gpu::probe_once(roots.first().map(PathBuf::as_path), model_vram_mb);
        apply_exe_presence(&mut probe, &roots);
        let attempts = plan_attempts(config, &probe);
        tracing::info!(
            attempts = ?attempts,
            probe_cached = probe.cached,
            useable = probe.useable,
            backends = ?probe
                .backends
                .iter()
                .map(|item| (item.backend, item.installed, item.available))
                .collect::<Vec<_>>(),
            "本地引擎启动链已规划"
        );

        // 3) 依次启动，失败继续下一个。
        let mut failures: Vec<String> = Vec::new();
        let mut failure_codes: Vec<String> = Vec::new();
        for attempt in &attempts {
            match self
                .try_attempt(app, config, attempt, &probe, &model_path, &failures)
                .await
            {
                Ok(()) => {
                    let payload = self.snapshot();
                    self.publish(app);
                    tracing::info!(
                        backend = attempt.backend.as_str(),
                        device = ?payload.device,
                        port = ?payload.port,
                        offload = ?payload.offload_layers,
                        fallback = ?payload.fallback_reason,
                        "本地引擎启动成功"
                    );
                    return Ok(payload);
                }
                Err(err) => {
                    tracing::warn!(
                        backend = attempt.backend.as_str(),
                        code = %err.code,
                        detail = ?err.detail,
                        "本地引擎尝试失败，继续回退链"
                    );
                    failures.push(format!(
                        "{}（{}）",
                        attempt.label,
                        err.detail.clone().unwrap_or_else(|| err.message.clone())
                    ));
                    failure_codes.push(err.code.clone());
                }
            }
        }

        // 4) 全部失败：状态 failed + 可操作错误。
        let missing_component = !failure_codes.is_empty()
            && failure_codes
                .iter()
                .all(|code| code == ErrorCode::ModelMissing);
        let error = if missing_component {
            AppError::new(ErrorCode::ModelMissing, Some(failures.join("；")))
                .with_message("缺少本地推理后端组件")
                .with_fallback(vec!["download_model", "cloud_translate"])
        } else {
            AppError::new(ErrorCode::OutOfMemory, Some(failures.join("；")))
                .with_message("本地推理引擎启动失败")
                .with_fallback(vec!["light_model", "cloud_translate"])
        };
        {
            let mut runtime = self.runtime.lock();
            runtime.status = EngineRuntimeStatus {
                status: EngineStatus::Failed,
                fallback_reason: Some(format!("本地推理引擎启动失败：{}", failures.join("；"))),
                ..EngineRuntimeStatus::default()
            };
            runtime.spawn_key = spawn_key(config);
        }
        self.publish(app);
        Err(error)
    }

    /// 单个 attempt：挑端口 → spawn → 等 `/health` → 解析日志 → 落地运行态。
    async fn try_attempt(
        &self,
        app: &AppHandle,
        config: &Config,
        attempt: &EngineAttempt,
        probe: &GpuProbeResult,
        model_path: &Path,
        failures: &[String],
    ) -> AppResult<()> {
        let roots = self.backends_roots(app);
        let exe = resolve_exe(&roots, attempt.backend).ok_or_else(|| {
            AppError::new(
                ErrorCode::ModelMissing,
                Some(format!(
                    "未找到 {} 后端 llama-server.exe（查找目录：{}）",
                    attempt.backend.as_str(),
                    roots
                        .iter()
                        .map(|root| root.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" ; ")
                )),
            )
            .with_message("缺少本地推理后端组件")
            .with_fallback(vec!["download_model", "cloud_translate"])
        })?;

        let port = process::pick_port(config.local.port)?;
        let threads = engine_threads();
        let context = self.model_context(config);
        let log_path = {
            let now = chrono::Local::now();
            self.paths.logs_dir.join(format!(
                "engine-{}-{}-{:03}.log",
                attempt.backend.as_str(),
                now.format("%Y%m%d-%H%M%S"),
                now.timestamp_subsec_millis()
            ))
        };
        let mut args = build_args_with_context(attempt, model_path, port, threads, context);
        // `--no-webui` 仅在该构建支持时追加（契约 §1.3：不支持则省略，以实测为准）。
        if process::server_supports_flag(&exe, "--no-webui") {
            args.push("--no-webui".to_string());
        }
        // 日志策略：优先 `--log-file`（见 `apply_log_args` 注释的实测踩坑）。
        apply_log_args(&exe, &mut args, &log_path);
        tracing::info!(
            backend = attempt.backend.as_str(),
            device = ?attempt.device,
            gpu_layers = attempt.gpu_layers,
            exe = %exe.display(),
            model = %model_path.display(),
            port,
            log = %log_path.display(),
            "启动 llama-server"
        );

        let started = Instant::now();
        let mut server = spawn_engine_server(&exe, &args, &log_path, exe.parent())?;
        let pid = server.pid;
        let base_url = format!("http://127.0.0.1:{port}");
        let client = process::build_http_client(Duration::from_secs(5))?;

        if let Err(err) =
            process::wait_until_healthy(&client, &base_url, &mut server, health_timeout()).await
        {
            let log = read_engine_log(&log_path);
            server.kill();
            let _ = process::wait_process_gone(pid, PROCESS_GONE_TIMEOUT);
            return Err(AppError::new(
                ErrorCode::OutOfMemory,
                Some(format!(
                    "{} 启动失败：{}；日志尾部：{}",
                    attempt.label,
                    err.describe(),
                    log_tail_lines(&log, 6)
                )),
            ));
        }

        let ready_ms = started.elapsed().as_millis() as u64;
        let log = read_engine_log(&log_path);
        let offload = parse_offload(&log);
        let device_token = parse_device_line(&log);
        if let Some((loaded, total)) = offload {
            tracing::info!(loaded, total, "日志 offload 解析成功");
        } else {
            tracing::warn!("未能从日志解析 offload 层数（不阻塞启动）");
        }
        let device_name = preferred_device(probe, attempt.backend)
            .map(|device| device.name.clone())
            .or_else(|| parse_device_name(&log));
        let device = if attempt.backend.is_gpu() {
            LocalDevice::Gpu
        } else {
            LocalDevice::Cpu
        };
        let offload_layers = match offload {
            Some((loaded, _)) => Some(loaded),
            None if attempt.backend.is_gpu() => Some(attempt.gpu_layers),
            None => Some(0),
        };
        tracing::info!(
            ready_ms,
            backend = attempt.backend.as_str(),
            device = device.as_str(),
            device_token = ?device_token,
            device_name = ?device_name,
            offload = ?offload,
            "本地引擎 /health 就绪"
        );

        {
            let mut runtime = self.runtime.lock();
            runtime.child = Some(server);
            runtime.status = EngineRuntimeStatus {
                status: EngineStatus::Ready,
                device: Some(device),
                backend: Some(attempt.backend),
                fallback_reason: fallback_reason(config, probe, failures, attempt.backend),
                offload_layers,
                device_name,
                port: Some(port),
                base_url: Some(base_url),
                pid: Some(pid),
                log_path: Some(log_path.display().to_string()),
            };
            runtime.spawn_key = spawn_key(config);
            runtime.last_used = Instant::now();
        }
        self.ensure_idle_timer(app);
        Ok(())
    }

    /// 空闲卸载定时器（`local.idleUnloadSec`，0 = 常驻）：进程内只启动一次。
    fn ensure_idle_timer(&self, app: &AppHandle) {
        if self.idle_timer_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(IDLE_TICK).await;
                let Some(state) = app.try_state::<crate::app::state::AppState>() else {
                    break;
                };
                let engine = state.local_engine.clone();
                let idle_sec = state.snapshot_config().local.idle_unload_sec;
                if idle_sec == 0 {
                    continue;
                }
                let idle = {
                    let runtime = engine.runtime.lock();
                    runtime.status.status == EngineStatus::Ready
                        && runtime.last_used.elapsed() >= Duration::from_secs(u64::from(idle_sec))
                };
                if idle {
                    tracing::info!(idle_sec, "本地引擎空闲超时，自动卸载");
                    let _ = engine.stop().await;
                    engine.publish(&app);
                }
            }
        });
    }

    /// 后端组件根目录（§1.2 顺序：`SNIP_ASK_BACKENDS_DIR` → `<data_dir>/backends` → 随包 resources）。
    pub fn backends_roots(&self, app: &AppHandle) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(custom) = std::env::var(BACKENDS_DIR_ENV) {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                roots.push(PathBuf::from(trimmed));
            }
        }
        roots.push(self.paths.data_dir.join("backends"));
        if let Ok(resource_dir) = app.path().resource_dir() {
            roots.push(resource_dir.join("backends"));
        }
        roots
    }

    /// 解析模型 GGUF 路径（`SNIP_ASK_MODEL_PATH` 优先，其次 `<data_dir>/models/<id>/`）。
    pub fn resolve_model_path(&self, config: &Config) -> AppResult<PathBuf> {
        if let Ok(custom) = std::env::var(MODEL_PATH_ENV) {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                let path = PathBuf::from(trimmed);
                if path.is_file() {
                    return Ok(path);
                }
                return Err(AppError::new(
                    ErrorCode::ModelMissing,
                    Some(format!(
                        "{MODEL_PATH_ENV} 指向的模型文件不存在：{}",
                        path.display()
                    )),
                )
                .with_message("本地模型缺失")
                .with_fallback(vec!["download_model", "cloud_translate"]));
            }
        }
        let manifest = parse_manifest(crate::resources::MODELS_MANIFEST)?;
        let entry = find_entry(&manifest, &config.local.model).ok_or_else(|| {
            AppError::new(
                ErrorCode::ModelMissing,
                Some(format!(
                    "模型 {} 不在 models.manifest.json 中；请在设置页选择已安装模型",
                    config.local.model
                )),
            )
            .with_message("本地模型缺失")
        })?;
        let dir = model_manager::model_dir(&self.paths.models_dir, &entry.id);
        let expected = dir.join(model_manager::download_filename(entry));
        if expected.is_file() {
            return Ok(expected);
        }
        // 兼容用户手动放入的 GGUF（文件名可能与清单不同）。
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for item in entries.flatten() {
                let path = item.path();
                if path
                    .extension()
                    .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("gguf"))
                {
                    return Ok(path);
                }
            }
        }
        Err(AppError::new(
            ErrorCode::ModelMissing,
            Some(format!(
                "本地模型文件不存在：{}（可设置 {MODEL_PATH_ENV} 指向真实 GGUF 用于验证）",
                expected.display()
            )),
        )
        .with_message("本地模型缺失")
        .with_fallback(vec!["download_model", "cloud_translate"]))
    }

    /// 当前模型的显存预算（清单 `vramMB`；未知为 0）。
    fn model_vram_mb(&self, config: &Config) -> u32 {
        parse_manifest(crate::resources::MODELS_MANIFEST)
            .ok()
            .and_then(|manifest| {
                find_entry(&manifest, &config.local.model).map(|item| item.vram_mb)
            })
            .unwrap_or(0)
    }

    /// 当前模型的上下文窗口（清单 `params.context`；缺省 [`DEFAULT_CONTEXT`]）。
    fn model_context(&self, config: &Config) -> u32 {
        parse_manifest(crate::resources::MODELS_MANIFEST)
            .ok()
            .and_then(|manifest| {
                find_entry(&manifest, &config.local.model)
                    .and_then(|entry| entry.params.get("context"))
                    .and_then(serde_json::Value::as_u64)
                    .map(|value| value as u32)
            })
            .unwrap_or(DEFAULT_CONTEXT)
    }
}

/// 运行参数指纹：模型 / 设备策略变化时重启引擎。
pub fn spawn_key(config: &Config) -> String {
    format!(
        "{}|{}|{}|{}",
        config.local.model,
        config.local.device.as_str(),
        config.local.gpu_backend.as_str(),
        config.local.effective_gpu_layers()
    )
}

/// 解析后端可执行文件（按目录顺序查找 `<root>/<variant>/llama-server.exe`）。
pub fn resolve_exe(roots: &[PathBuf], backend: LocalBackend) -> Option<PathBuf> {
    for root in roots {
        let candidate = gpu::backend_exe_path(root, backend);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 用磁盘上真实存在的 `llama-server.exe` 校正探测结果的 `installed` 字段。
///
/// 理由：`installed` 决定回退链是否尝试该后端；探测实现（T1）可能尚未接入真实目录扫描，
/// 而「组件是否就位」本引擎可以自己可靠判定。`available`（有无非虚拟设备）保持探测结论，
/// 用于解释 `fallbackReason`。
pub fn apply_exe_presence(probe: &mut GpuProbeResult, roots: &[PathBuf]) {
    for status in &mut probe.backends {
        match resolve_exe(roots, status.backend) {
            Some(exe) => {
                status.installed = true;
                status.exe_path = Some(exe.display().to_string());
                if status.reason.is_none() && !status.available {
                    status.reason = Some(format!(
                        "{} 后端已安装但未检测到可用设备",
                        status.backend.as_str()
                    ));
                }
            }
            None => {
                status.installed = false;
                if status.reason.is_none() {
                    status.reason = Some(format!("未安装 {} 后端组件", status.backend.as_str()));
                }
            }
        }
    }
}

/// 指定后端的首选设备（非虚拟 + 空闲显存最大）。
pub fn preferred_device(
    probe: &GpuProbeResult,
    backend: LocalBackend,
) -> Option<&crate::contract::types::GpuDevice> {
    probe
        .devices
        .iter()
        .filter(|device| device.backend == backend && !device.virtual_adapter)
        .max_by(|left, right| {
            left.vram_free_mb
                .cmp(&right.vram_free_mb)
                .then_with(|| right.index.cmp(&left.index))
        })
}

/// 显存预算是否明确不足（`probe.budget.fits == false`）。
pub fn budget_exhausted(probe: &GpuProbeResult) -> bool {
    matches!(&probe.budget, Some(budget) if !budget.fits)
}

/// 规划启动尝试链（纯函数，Vulkan 优先）。
///
/// 规则：
/// - `local.device = cpu` 或 `gpuLayers = 0` → 仅 CPU（显式 `-ngl 0`）；
/// - `gpuBackend = auto` → `Vulkan → CUDA → CPU`；`vulkan` → `Vulkan → CPU`；
///   `cuda` → `CUDA → Vulkan → CPU`（Vulkan 优先，绝不把 `nvidia-smi` 当判据）；
/// - 组件未安装（`installed = false`）的后端跳过；
/// - 探测确实跑通（`devices` 非空）且该后端无可用设备时跳过；
/// - 显存预算明确不足 → 跳过 GPU，回退 CPU 并在 `fallbackReason` 说明。
/// - **CPU 恒为链尾**：它是最终回退项，其组件缺失时由 `try_attempt` 给出可操作错误。
pub fn plan_attempts(config: &Config, probe: &GpuProbeResult) -> Vec<EngineAttempt> {
    let layers = config.local.effective_gpu_layers();
    let mut attempts = Vec::new();
    if config.local.device.allows_gpu() && layers > 0 && !budget_exhausted(probe) {
        let order: &[LocalBackend] = match config.local.gpu_backend {
            GpuBackend::Auto => &[LocalBackend::Vulkan, LocalBackend::Cuda],
            GpuBackend::Vulkan => &[LocalBackend::Vulkan],
            GpuBackend::Cuda => &[LocalBackend::Cuda, LocalBackend::Vulkan],
        };
        let probe_ran = !probe.devices.is_empty();
        for backend in order {
            let Some(status) = probe.backends.iter().find(|item| item.backend == *backend) else {
                continue;
            };
            if !status.installed {
                continue;
            }
            if probe_ran && !status.available {
                continue;
            }
            let device_index = preferred_device(probe, *backend)
                .map(|device| device.index)
                .unwrap_or(0);
            attempts.push(EngineAttempt {
                backend: *backend,
                device: Some(device_index),
                gpu_layers: layers,
                label: format!(
                    "{}:{}{}",
                    backend.as_str(),
                    device_prefix(*backend),
                    device_index
                ),
            });
        }
    }
    attempts.push(EngineAttempt {
        backend: LocalBackend::Cpu,
        device: None,
        gpu_layers: 0,
        label: "cpu".to_string(),
    });
    attempts
}

/// GPU 后端设备前缀（`Vulkan0` / `CUDA0`）。
fn device_prefix(backend: LocalBackend) -> &'static str {
    match backend {
        LocalBackend::Vulkan => "Vulkan",
        LocalBackend::Cuda => "CUDA",
        LocalBackend::Cpu => "",
    }
}

/// 计算 `fallbackReason`（纯函数）：只有在「配置想要 GPU、最终落在 CPU」时给出。
pub fn fallback_reason(
    config: &Config,
    probe: &GpuProbeResult,
    failures: &[String],
    final_backend: LocalBackend,
) -> Option<String> {
    if final_backend.is_gpu() {
        return None;
    }
    if !config.local.device.allows_gpu() || config.local.effective_gpu_layers() == 0 {
        return None;
    }
    if let Some(first) = failures.first() {
        return Some(format!("GPU 后端启动失败（{first}），已回退 CPU"));
    }
    if let Some(budget) = &probe.budget {
        if !budget.fits {
            return Some(format!(
                "显存不足：需要约 {} MB，目标设备空闲 {} MB，已回退 CPU（可在设置页降低 GPU 层数或改用更小模型）",
                budget.required_mb, budget.available_mb
            ));
        }
    }
    Some(
        probe
            .issue
            .clone()
            .unwrap_or_else(|| "未检测到可用 GPU 后端，已使用 CPU".to_string()),
    )
}

/// 拼装 llama-server 参数（纯函数，使用默认上下文窗口）。
pub fn build_args(
    attempt: &EngineAttempt,
    model_path: &Path,
    port: u16,
    threads: u32,
) -> Vec<String> {
    build_args_with_context(attempt, model_path, port, threads, DEFAULT_CONTEXT)
}

/// 拼装 llama-server 参数（纯函数）。
///
/// 真实实现（研究 §1.2）：
/// - `-m <gguf>`、`--host 127.0.0.1`、`--port <p>`、`-c <ctx>`；
/// - CPU：`-ngl 0 -t <threads>`；GPU：`-ngl <layers> --device Vulkan0|CUDA0`；
/// - **必须显式传 `-ngl`**（LM Studio 构建省略会默认全量 offload，官方构建语义不同）。
pub fn build_args_with_context(
    attempt: &EngineAttempt,
    model_path: &Path,
    port: u16,
    threads: u32,
    context: u32,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-m".to_string(),
        model_path.display().to_string(),
        "--host".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        port.to_string(),
        "-c".to_string(),
        context.max(128).to_string(),
    ];
    if attempt.backend == LocalBackend::Cpu || attempt.gpu_layers == 0 {
        args.push("-ngl".to_string());
        args.push("0".to_string());
        args.push("-t".to_string());
        args.push(threads.to_string());
    } else {
        args.push("-ngl".to_string());
        args.push(attempt.gpu_layers.to_string());
        if let Some(index) = attempt.device {
            args.push("--device".to_string());
            args.push(format!("{}{}", device_prefix(attempt.backend), index));
        }
        args.push("-t".to_string());
        args.push(threads.to_string());
    }
    args
}

/// 从日志解析 offload 层数：`offloaded 25/25 layers to GPU` → `(25, 25)`。
pub fn parse_offload(log: &str) -> Option<(u32, u32)> {
    let marker = "offloaded ";
    let start = log.rfind(marker)? + marker.len();
    let rest = &log[start..];
    let end = rest.find(" layers")?;
    let pair = &rest[..end];
    let (loaded, total) = pair.split_once('/')?;
    Some((loaded.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// 从日志解析实际设备行：`using device Vulkan0` → `Vulkan0`。
pub fn parse_device_line(log: &str) -> Option<String> {
    for line in log.lines().rev() {
        if let Some(rest) = line.split("using device ").nth(1) {
            let token = rest.split_whitespace().next()?.trim_end_matches(',');
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

/// 从日志解析设备展示名（Vulkan / CUDA 两种格式）。
///
/// - `using device Vulkan0 (NVIDIA GeForce RTX 4060 Laptop GPU) (0000:01:00.0) - 7180 MiB free`
/// - `ggml_vulkan: 0 = NVIDIA GeForce RTX 4060 Laptop GPU (NVIDIA) | uma: 0 | ...`
/// - `Device 0: NVIDIA GeForce RTX 4060 Laptop GPU, compute capability 8.9, VMM: yes`
pub fn parse_device_name(log: &str) -> Option<String> {
    const VULKAN_MARKER: &str = "ggml_vulkan: 0 = ";
    for line in log.lines().rev() {
        if let Some(index) = line.find(VULKAN_MARKER) {
            let rest = &line[index + VULKAN_MARKER.len()..];
            let name = rest.split(" (").next().unwrap_or(rest).trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
        if let Some(rest) = line.split("using device ").nth(1) {
            // 只取**第一个**括号内内容：`(NVIDIA ... GPU) (0000:01:00.0) - 7180 MiB free`。
            if let Some(open) = rest.find('(') {
                let after_open = &rest[open + 1..];
                if let Some(close) = after_open.find(')') {
                    let name = after_open[..close].trim();
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
        if let Some(index) = line.find("Device 0: ") {
            let rest = &line[index + "Device 0: ".len()..];
            let name = rest.split(',').next().unwrap_or(rest).trim();
            if !name.is_empty() && !name.to_ascii_lowercase().contains("unknown") {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// 取日志末尾 `count` 行（错误 detail 用）。
pub fn log_tail_lines(log: &str, count: usize) -> String {
    let lines: Vec<&str> = log.lines().filter(|line| !line.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(count);
    lines[start..].join(" | ")
}

/// 推理线程数（物理核心数，上限 32；获取失败回退 4）。
pub fn engine_threads() -> u32 {
    std::thread::available_parallelism()
        .map(|value| value.get() as u32)
        .unwrap_or(4)
        .clamp(1, 32)
}

// ---------------------------------------------------------------------------
// 日志采集策略（实测踩坑：stdout 重定向会丢模型加载日志）
// ---------------------------------------------------------------------------

/// `--log-file` 构建的 stdout/stderr sidecar 后缀（`<engine-log>.stdout.log`）。
pub const STDOUT_LOG_SUFFIX: &str = ".stdout.log";

/// 日志采集方式：是否用 llama.cpp 自己的 `--log-file`。
pub fn supports_log_file(exe: &Path) -> bool {
    process::server_supports_flag(exe, "--log-file")
}

/// 追加日志相关启动参数，返回 stdout/stderr 的重定向目标。
///
/// 实测（本机 LM Studio llama.cpp 2.28.2 构建）：
/// - 直接重定向 stdout/stderr 时，`offloaded X/Y layers to GPU` / `using device Vulkan0`
///   这类 `llama_model_loader` 日志走 **C stdio 块缓冲**，`/health` 就绪时刻尚未落盘
///   （缓冲 < 4 KiB 不 flush），只有 `srv` 组件的少量行可见；
/// - 该构建支持 `--log-file`（logger 直写文件，立即落盘）+ `-lv 4`
///   （默认 verbosity 不含加载 INFO；`-v` 会输出大量 DEBUG，日志 ~250 KB）；
///   因此优先 `--log-file` + `-lv 4`，stdout/stderr 另存 sidecar（不覆盖主日志）。
/// - 不支持 `--log-file` 的构建退回 stdout/stderr 重定向（官方构建通常加载日志走 stderr）。
pub fn apply_log_args(exe: &Path, args: &mut Vec<String>, log_path: &Path) -> PathBuf {
    if supports_log_file(exe) {
        args.push("--log-file".to_string());
        args.push(log_path.display().to_string());
        if process::server_supports_flag(exe, "--log-verbosity")
            || process::server_supports_flag(exe, "-lv")
        {
            args.push("-lv".to_string());
            args.push("4".to_string());
        }
        PathBuf::from(format!("{}{}", log_path.display(), STDOUT_LOG_SUFFIX))
    } else {
        log_path.to_path_buf()
    }
}

/// 按日志策略启动子进程（自动区分 `--log-file` / stdout 重定向）。
pub fn spawn_engine_server(
    exe: &Path,
    args: &[String],
    log_path: &Path,
    work_dir: Option<&Path>,
) -> AppResult<process::ServerProcess> {
    let use_log_file = args.iter().any(|arg| arg == "--log-file");
    if use_log_file {
        let stdio_log = PathBuf::from(format!("{}{}", log_path.display(), STDOUT_LOG_SUFFIX));
        process::spawn_server_split(exe, args, log_path, &stdio_log, work_dir)
    } else {
        process::spawn_server(exe, args, log_path, work_dir)
    }
}

/// 读取引擎日志（主日志 = `--log-file` 或 stdout 重定向；合并 sidecar）。
pub fn read_engine_log(log_path: &Path) -> String {
    let mut log = process::read_log(log_path);
    let sidecar = PathBuf::from(format!("{}{}", log_path.display(), STDOUT_LOG_SUFFIX));
    if sidecar.is_file() {
        let text = process::read_log(&sidecar);
        if !text.is_empty() {
            log.push('\n');
            log.push_str(&text);
        }
    }
    log
}

/// 就绪超时（`SNIP_ASK_ENGINE_READY_TIMEOUT_SEC` 开发钩子可覆盖）。
pub fn health_timeout() -> Duration {
    let secs = std::env::var(HEALTH_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_HEALTH_TIMEOUT_SEC);
    Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::types::{GpuBackendStatus, GpuBudget, GpuDevice};

    fn device(index: u32, backend: LocalBackend, free: u64) -> GpuDevice {
        GpuDevice {
            index,
            name: "Test GPU".to_string(),
            backend,
            vram_total_mb: 8192,
            vram_free_mb: free,
            vendor: Some("unknown".to_string()),
            virtual_adapter: false,
        }
    }

    fn status(backend: LocalBackend, installed: bool, available: bool) -> GpuBackendStatus {
        GpuBackendStatus {
            backend,
            installed,
            available,
            exe_path: None,
            version: None,
            reason: None,
        }
    }

    fn probe(backends: Vec<GpuBackendStatus>, devices: Vec<GpuDevice>) -> GpuProbeResult {
        GpuProbeResult {
            checked_at: "2026-11-05T00:00:00Z".to_string(),
            cached: false,
            useable: backends
                .iter()
                .any(|item| item.backend.is_gpu() && item.available),
            recommended: None,
            devices,
            backends,
            budget: None,
            issue: None,
        }
    }

    #[test]
    fn cpu_args_are_explicit_ngl_zero() {
        let attempt = EngineAttempt {
            backend: LocalBackend::Cpu,
            device: None,
            gpu_layers: 0,
            label: "cpu".to_string(),
        };
        let args = build_args(&attempt, Path::new("C:/m.gguf"), 39281, 16);
        // 必须显式 -ngl 0（研究 §1.2 踩坑：省略语义在不同构建下不同）。
        let ngl = args
            .iter()
            .position(|arg| arg == "-ngl")
            .expect("-ngl 必须存在");
        assert_eq!(args[ngl + 1], "0");
        assert!(args.contains(&"-t".to_string()));
        assert!(!args.contains(&"--device".to_string()));
    }

    #[test]
    fn vulkan_args_pass_device_and_layers() {
        let attempt = EngineAttempt {
            backend: LocalBackend::Vulkan,
            device: Some(0),
            gpu_layers: 99,
            label: "vulkan:Vulkan0".to_string(),
        };
        let args = build_args(&attempt, Path::new("C:/m.gguf"), 39281, 16);
        let ngl = args.iter().position(|arg| arg == "-ngl").unwrap();
        assert_eq!(args[ngl + 1], "99");
        let device = args.iter().position(|arg| arg == "--device").unwrap();
        assert_eq!(args[device + 1], "Vulkan0");
        let context = args.iter().position(|arg| arg == "-c").unwrap();
        assert_eq!(args[context + 1], DEFAULT_CONTEXT.to_string());
    }

    #[test]
    fn cuda_args_use_cuda_device_prefix() {
        let attempt = EngineAttempt {
            backend: LocalBackend::Cuda,
            device: Some(1),
            gpu_layers: 24,
            label: "cuda:CUDA1".to_string(),
        };
        let args = build_args(&attempt, Path::new("C:/m.gguf"), 39281, 8);
        let device = args.iter().position(|arg| arg == "--device").unwrap();
        assert_eq!(args[device + 1], "CUDA1");
        assert_eq!(LocalBackend::Vulkan.as_str(), "vulkan");
        assert_eq!(LocalBackend::Cuda.as_gpu_backend(), Some(GpuBackend::Cuda));
        assert_eq!(LocalBackend::Cpu.as_gpu_backend(), None);
    }

    #[test]
    fn parses_offload_and_device_from_log() {
        let log = "load_tensors: offloaded 25/25 layers to GPU\n\
                   ggml_vulkan: using device Vulkan0 (NVIDIA GeForce RTX 4060 Laptop GPU)";
        assert_eq!(parse_offload(log), Some((25, 25)));
        assert_eq!(parse_device_line(log).as_deref(), Some("Vulkan0"));
        assert_eq!(
            parse_device_name(log).as_deref(),
            Some("NVIDIA GeForce RTX 4060 Laptop GPU")
        );
        assert_eq!(parse_offload("no gpu here"), None);
    }

    #[test]
    fn parses_device_name_from_real_vulkan_lines() {
        let log = "ggml_vulkan: Found 1 Vulkan devices:\n\
                   ggml_vulkan: 0 = NVIDIA GeForce RTX 4060 Laptop GPU (NVIDIA) | uma: 0 | fp16: 1 | warp size: 32\n\
                   load_tensors: offloaded 25/25 layers to GPU";
        assert_eq!(
            parse_device_name(log).as_deref(),
            Some("NVIDIA GeForce RTX 4060 Laptop GPU")
        );
        let cuda = "ggml_cuda_init: found 1 CUDA devices:\n\
                    Device 0: NVIDIA GeForce RTX 4060 Laptop GPU, compute capability 8.9, VMM: yes";
        assert_eq!(
            parse_device_name(cuda).as_deref(),
            Some("NVIDIA GeForce RTX 4060 Laptop GPU")
        );
        assert_eq!(parse_device_name("no device"), None);
    }

    #[test]
    fn cpu_device_forces_zero_layers() {
        let mut config = Config::default();
        config.local.device = LocalDevice::Cpu;
        config.local.gpu_layers = 99;
        let attempts = plan_attempts(&config, &crate::platform::gpu::probe_once(None, 0));
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].gpu_layers, 0);
        assert_eq!(attempts[0].backend, LocalBackend::Cpu);
    }

    #[test]
    fn auto_backend_chain_is_vulkan_then_cuda_then_cpu() {
        let config = Config::default();
        let probed = probe(
            vec![
                status(LocalBackend::Vulkan, true, true),
                status(LocalBackend::Cuda, true, true),
                status(LocalBackend::Cpu, true, false),
            ],
            vec![
                device(0, LocalBackend::Vulkan, 7180),
                device(0, LocalBackend::Cuda, 7020),
            ],
        );
        let attempts = plan_attempts(&config, &probed);
        let backends: Vec<LocalBackend> = attempts.iter().map(|item| item.backend).collect();
        assert_eq!(
            backends,
            vec![LocalBackend::Vulkan, LocalBackend::Cuda, LocalBackend::Cpu]
        );
        assert_eq!(attempts[0].gpu_layers, 99);
        assert_eq!(attempts[0].device, Some(0));
        assert_eq!(attempts[2].gpu_layers, 0);
    }

    #[test]
    fn explicit_cuda_prefers_cuda_but_keeps_vulkan_fallback() {
        let mut config = Config::default();
        config.local.gpu_backend = GpuBackend::Cuda;
        let probed = probe(
            vec![
                status(LocalBackend::Vulkan, true, true),
                status(LocalBackend::Cuda, true, true),
                status(LocalBackend::Cpu, true, false),
            ],
            vec![device(0, LocalBackend::Vulkan, 7000)],
        );
        let backends: Vec<LocalBackend> = plan_attempts(&config, &probed)
            .iter()
            .map(|item| item.backend)
            .collect();
        assert_eq!(
            backends,
            vec![LocalBackend::Cuda, LocalBackend::Vulkan, LocalBackend::Cpu]
        );
    }

    #[test]
    fn explicit_vulkan_does_not_fall_back_to_cuda() {
        let mut config = Config::default();
        config.local.gpu_backend = GpuBackend::Vulkan;
        let probed = probe(
            vec![
                status(LocalBackend::Vulkan, true, true),
                status(LocalBackend::Cuda, true, true),
                status(LocalBackend::Cpu, true, false),
            ],
            vec![device(0, LocalBackend::Vulkan, 7000)],
        );
        let backends: Vec<LocalBackend> = plan_attempts(&config, &probed)
            .iter()
            .map(|item| item.backend)
            .collect();
        assert_eq!(backends, vec![LocalBackend::Vulkan, LocalBackend::Cpu]);
    }

    #[test]
    fn missing_components_are_skipped_but_cpu_stays() {
        let config = Config::default();
        let probed = probe(
            vec![
                status(LocalBackend::Vulkan, false, false),
                status(LocalBackend::Cuda, false, false),
                status(LocalBackend::Cpu, false, false),
            ],
            Vec::new(),
        );
        let attempts = plan_attempts(&config, &probed);
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].backend, LocalBackend::Cpu);
    }

    #[test]
    fn backend_without_devices_is_skipped_when_probe_ran() {
        let config = Config::default();
        let probed = probe(
            vec![
                status(LocalBackend::Vulkan, true, false),
                status(LocalBackend::Cuda, false, false),
                status(LocalBackend::Cpu, true, false),
            ],
            vec![device(0, LocalBackend::Cuda, 7020)],
        );
        let attempts = plan_attempts(&config, &probed);
        // Vulkan 已安装但探测不到设备（probe 确实跑通：CUDA 有设备）→ 跳过 Vulkan。
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].backend, LocalBackend::Cpu);
    }

    #[test]
    fn insufficient_vram_falls_back_to_cpu_with_reason() {
        let config = Config::default();
        let mut probed = probe(
            vec![
                status(LocalBackend::Vulkan, true, true),
                status(LocalBackend::Cuda, false, false),
                status(LocalBackend::Cpu, true, false),
            ],
            vec![device(0, LocalBackend::Vulkan, 900)],
        );
        probed.budget = Some(GpuBudget {
            required_mb: 2112,
            available_mb: 900,
            fits: false,
        });
        let attempts = plan_attempts(&config, &probed);
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].backend, LocalBackend::Cpu);
        let reason = fallback_reason(&config, &probed, &[], LocalBackend::Cpu).unwrap();
        assert!(reason.contains("显存不足"), "reason = {reason}");
        assert!(reason.contains("2112"));
        // GPU 成功时不应有 fallbackReason。
        assert!(fallback_reason(&config, &probed, &[], LocalBackend::Vulkan).is_none());
    }

    #[test]
    fn gpu_layers_zero_means_cpu_only() {
        let mut config = Config::default();
        config.local.gpu_layers = 0;
        let probed = probe(
            vec![
                status(LocalBackend::Vulkan, true, true),
                status(LocalBackend::Cpu, true, false),
            ],
            vec![device(0, LocalBackend::Vulkan, 7000)],
        );
        let attempts = plan_attempts(&config, &probed);
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].backend, LocalBackend::Cpu);
        assert_eq!(attempts[0].gpu_layers, 0);
        assert!(fallback_reason(&config, &probed, &[], LocalBackend::Cpu).is_none());
    }

    #[test]
    fn fallback_reason_reports_failure_and_missing_gpu() {
        let config = Config::default();
        let mut probed = probe(
            vec![
                status(LocalBackend::Vulkan, true, true),
                status(LocalBackend::Cpu, true, false),
            ],
            vec![device(0, LocalBackend::Vulkan, 7000)],
        );
        let reason = fallback_reason(
            &config,
            &probed,
            &["vulkan:Vulkan0（等待 /health 就绪超时）".to_string()],
            LocalBackend::Cpu,
        )
        .unwrap();
        assert!(reason.contains("启动失败"), "reason = {reason}");

        probed.issue = Some("未检测到可用 GPU 后端，将使用 CPU".to_string());
        let reason = fallback_reason(&config, &probed, &[], LocalBackend::Cpu).unwrap();
        assert!(reason.contains("未检测到可用 GPU"));
    }

    #[test]
    fn spawn_key_changes_with_device_strategy() {
        let config = Config::default();
        let base = spawn_key(&config);
        let mut changed = config.clone();
        changed.local.device = LocalDevice::Cpu;
        assert_ne!(base, spawn_key(&changed));
        changed = config.clone();
        changed.local.gpu_backend = GpuBackend::Cuda;
        assert_ne!(base, spawn_key(&changed));
        changed = config.clone();
        changed.local.gpu_layers = 8;
        assert_ne!(base, spawn_key(&changed));
        // 端口变化不触发重启（引擎自己挑端口）。
        changed = config.clone();
        changed.local.port = 39300;
        assert_eq!(base, spawn_key(&changed));
    }

    #[test]
    fn apply_exe_presence_marks_real_files() {
        let root = std::env::temp_dir().join(format!("snip-ask-backends-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("vulkan")).unwrap();
        std::fs::write(root.join("vulkan").join("llama-server.exe"), b"fake").unwrap();
        let mut probed = probe(
            vec![
                status(LocalBackend::Vulkan, false, false),
                status(LocalBackend::Cuda, false, false),
                status(LocalBackend::Cpu, false, false),
            ],
            Vec::new(),
        );
        apply_exe_presence(&mut probed, std::slice::from_ref(&root));
        assert!(probed.backends[0].installed);
        assert!(probed.backends[0].exe_path.is_some());
        assert!(!probed.backends[1].installed);
        assert!(!probed.backends[2].installed);
        assert!(resolve_exe(std::slice::from_ref(&root), LocalBackend::Vulkan).is_some());
        assert!(resolve_exe(std::slice::from_ref(&root), LocalBackend::Cuda).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn log_tail_lines_keeps_last_non_empty_lines() {
        let log = "line1\n\nline2\nline3\n";
        assert_eq!(log_tail_lines(log, 2), "line2 | line3");
        assert_eq!(log_tail_lines("", 3), "");
    }
}
