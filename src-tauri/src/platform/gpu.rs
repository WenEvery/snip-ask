//! GPU 探测与后端发现（契约 v4 / 研究 §1.3-4）。
//!
//! **Vulkan 优先**：探测只用 llama.cpp 自身的 `--list-devices` 输出判定“真能用”，
//! 不依赖 `nvidia-smi` 等 NVIDIA 专用工具，因此 AMD / Intel / NVIDIA 走同一条路径
//! （AMD 可用性是本需求的验收项，见 spec §1.1）。
//!
//! 目录布局（spec §1.2，优先级从高到低，每个目录内有 `llama-server.exe`）：
//! 1. `SNIP_ASK_BACKENDS_DIR`（开发/验证钩子）；
//! 2. `<data_dir>/backends/<variant>/`（按需下载安装位置）；
//! 3. `<resource_dir>/backends/<variant>/`（随包分发的 CPU 后端）。
//!
//! 本文件负责：
//! 1. 在给定的根目录里找 `<root>/<variant>/llama-server.exe`；
//! 2. 执行 `--list-devices` 解析设备与显存（失败再给出可操作 reason，不做 WMI 假探测）；
//! 3. 过滤虚拟/软件适配器（不计入“可用”）；
//! 4. 显存预算 `need = model.vramMB + 512 MiB`；
//! 5. 60s 缓存 + `recommend_backend`（Vulkan → CUDA）。
//!
//! 注意：`platform` 目录禁止 `unsafe`；需要系统信息时一律通过子进程获取。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::contract::config::{GpuBackend, LocalBackend};
use crate::contract::types::{GpuBackendStatus, GpuBudget, GpuDevice, GpuProbeResult};

/// 探测缓存有效期（研究 §1.3-4：60 s）。
pub const PROBE_CACHE_TTL: Duration = Duration::from_secs(60);

/// `llama-server --list-devices` 超时（Vulkan 首次初始化可能较慢）。
const LIST_DEVICES_TIMEOUT: Duration = Duration::from_secs(20);
/// `llama-server --version` 超时。
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
/// 轮询子进程退出的间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(20);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 后端组件可执行文件名。
pub const LLAMA_SERVER_EXE: &str = "llama-server.exe";

/// 虚拟/软件适配器关键字（小写比较；命中即不计入“可用 GPU”）。
const VIRTUAL_ADAPTER_MARKERS: &[&str] = &[
    "microsoft basic render",
    "basic render driver",
    "basic display adapter",
    "microsoft basic display",
    "remote desktop",
    "remote display",
    "rdp",
    "virtualbox",
    "vmware",
    "virtual machine",
    "hyper-v",
    "hyperv",
    "llvmpipe",
    "swiftshader",
    "software adapter",
    "software render",
    "virtual adapter",
    "virtual display",
    "citrix",
    "parallels",
    "qemu",
    "virtio",
    "mirage driver",
    "indirect display",
];

/// GPU 探测结果缓存。
#[derive(Debug, Default)]
pub struct GpuProbeCache {
    inner: Mutex<Option<CachedProbe>>,
}

#[derive(Debug, Clone)]
struct CachedProbe {
    at: Instant,
    /// 缓存对应的模型显存需求；模型切换后预算不同，必须重新探测。
    model_vram_mb: u32,
    result: GpuProbeResult,
}

impl GpuProbeCache {
    /// 新建缓存。
    pub fn new() -> Self {
        Self::default()
    }

    /// 探测（`force = true` 时跳过缓存）。
    ///
    /// `backends_dir`：后端根目录（`<dir>/vulkan/llama-server.exe` 等）；
    /// `model_vram_mb`：当前模型显存需求（清单 `vramMB`；0 表示未知/不约束）。
    pub fn probe(
        &self,
        backends_dir: Option<&Path>,
        model_vram_mb: u32,
        force: bool,
    ) -> GpuProbeResult {
        let roots: Vec<PathBuf> = backends_dir
            .map(|dir| vec![dir.to_path_buf()])
            .unwrap_or_default();
        self.probe_roots(&roots, model_vram_mb, force)
    }

    /// 按 §1.2 的根目录优先级探测（调用方传入已排序的候选根目录）。
    ///
    /// 缓存以 `(roots 无关, model_vram_mb)` 为键：模型切换后预算变化会重新探测；
    /// 后端组件安装/删除后由调用方 [`Self::invalidate`]。
    pub fn probe_roots(
        &self,
        roots: &[PathBuf],
        model_vram_mb: u32,
        force: bool,
    ) -> GpuProbeResult {
        let mut guard = self.inner.lock().expect("gpu probe cache poisoned");
        if !force {
            if let Some(cached) = guard.as_ref() {
                if cached.at.elapsed() < PROBE_CACHE_TTL && cached.model_vram_mb == model_vram_mb {
                    let mut result = cached.result.clone();
                    result.cached = true;
                    return result;
                }
            }
        }
        let result = probe_once_roots(roots, model_vram_mb);
        *guard = Some(CachedProbe {
            at: Instant::now(),
            model_vram_mb,
            result: result.clone(),
        });
        result
    }

    /// 清空缓存（后端组件安装/删除后调用）。
    pub fn invalidate(&self) {
        *self.inner.lock().expect("gpu probe cache poisoned") = None;
    }
}

/// 执行一次真实探测（单根目录版本，兼容既有调用方）。
pub fn probe_once(backends_dir: Option<&Path>, model_vram_mb: u32) -> GpuProbeResult {
    let roots: Vec<PathBuf> = backends_dir
        .map(|dir| vec![dir.to_path_buf()])
        .unwrap_or_default();
    probe_once_roots(&roots, model_vram_mb)
}

/// 执行一次真实探测（多根目录，按顺序查找后端组件）。
///
/// CPU 后端不参与 `--list-devices`：`cpu.available = installed`（CPU 永远可跑）。
pub fn probe_once_roots(roots: &[PathBuf], model_vram_mb: u32) -> GpuProbeResult {
    let mut devices: Vec<GpuDevice> = Vec::new();
    let mut backends: Vec<GpuBackendStatus> = Vec::with_capacity(3);

    for backend in [LocalBackend::Vulkan, LocalBackend::Cuda] {
        let exe = find_backend_exe(roots, backend);
        let status = match exe {
            None => GpuBackendStatus {
                backend,
                installed: false,
                available: false,
                exe_path: None,
                version: None,
                reason: Some(missing_reason(backend).to_string()),
            },
            Some(exe) => {
                let exe_path = Some(exe.display().to_string());
                let version = version_text(&exe);
                match list_devices_text(&exe) {
                    Ok(text) => {
                        let parsed = parse_list_devices(&text, backend);
                        let usable: Vec<GpuDevice> = parsed
                            .into_iter()
                            .filter(|device| !device.virtual_adapter)
                            .collect();
                        let available = !usable.is_empty();
                        let reason = if available {
                            None
                        } else {
                            Some(format!(
                                "{} 后端已安装，但未检测到可用 GPU 设备",
                                backend_label(backend)
                            ))
                        };
                        devices.extend(usable);
                        GpuBackendStatus {
                            backend,
                            installed: true,
                            available,
                            exe_path,
                            version,
                            reason,
                        }
                    }
                    Err(err) => GpuBackendStatus {
                        backend,
                        installed: true,
                        available: false,
                        exe_path,
                        version,
                        reason: Some(format!("{} 后端探测失败：{err}", backend_label(backend))),
                    },
                }
            }
        };
        backends.push(status);
    }

    let cpu_exe = find_backend_exe(roots, LocalBackend::Cpu);
    backends.push(GpuBackendStatus {
        backend: LocalBackend::Cpu,
        installed: cpu_exe.is_some(),
        available: cpu_exe.is_some(),
        exe_path: cpu_exe.as_ref().map(|exe| exe.display().to_string()),
        version: cpu_exe.as_ref().and_then(|exe| version_text(exe)),
        reason: if cpu_exe.is_some() {
            None
        } else {
            Some("CPU 后端未安装（应随主安装包分发）".to_string())
        },
    });

    let recommended = recommend_backend(&backends);
    let useable = backends
        .iter()
        .any(|status| status.backend.is_gpu() && status.available);
    let gpu_installed = backends
        .iter()
        .any(|status| status.backend.is_gpu() && status.installed);

    // 显存预算：以推荐后端的目标设备（第一块非虚拟 GPU）的空闲显存为基准。
    let budget = if model_vram_mb > 0 {
        recommended
            .and_then(|recommended| {
                let wanted = gpu_backend_as_local(recommended);
                devices.iter().find(|device| device.backend == wanted)
            })
            .map(|device| budget_for(model_vram_mb, device))
    } else {
        None
    };

    let issue = if useable {
        None
    } else if gpu_installed {
        Some("GPU 后端已安装，但未检测到可用 GPU 设备；将使用 CPU".to_string())
    } else {
        Some("未检测到可用 GPU 后端，将使用 CPU".to_string())
    };

    GpuProbeResult {
        checked_at: chrono::Utc::now().to_rfc3339(),
        cached: false,
        useable,
        recommended,
        devices,
        backends,
        budget,
        issue,
    }
}

/// 在根目录列表中定位后端的 `llama-server.exe`（优先级从前到后）。
pub fn find_backend_exe(roots: &[PathBuf], backend: LocalBackend) -> Option<PathBuf> {
    for root in roots {
        let candidate = backend_exe_path(root, backend);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 解析 `llama-server --list-devices` 输出。
///
/// 真实输出示例（本机实测，`Vulkan0` 为 RTX 4060 Laptop）：
/// ```text
/// Available devices:
///   Vulkan0: NVIDIA GeForce RTX 4060 Laptop GPU (7956 MiB, 7180 MiB free)
/// ```
/// 无设备时输出 `Available devices:\n  (none)` → 空列表。
pub fn parse_list_devices(stdout: &str, backend: LocalBackend) -> Vec<GpuDevice> {
    let mut devices = Vec::new();
    for raw in stdout.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("available devices")
            || lower == "(none)"
            || lower.starts_with("device count")
        {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        let prefix = line[..colon].trim();
        let rest = line[colon + 1..].trim();
        if rest.is_empty() {
            continue;
        }
        let Some((declared, index)) = parse_device_header(prefix) else {
            continue;
        };
        // 只接受与请求后端一致的设备行（如用 Vulkan 后端探测时忽略 CUDA 行）。
        let device_backend = match declared {
            Some(declared) if declared != backend => continue,
            Some(declared) => declared,
            None => backend,
        };
        if !device_backend.is_gpu() {
            continue;
        }
        let (name, vram_total_mb, vram_free_mb) = parse_device_tail(rest);
        if name.is_empty() {
            continue;
        }
        devices.push(GpuDevice {
            index,
            vendor: classify_vendor(&name),
            virtual_adapter: is_virtual_adapter(&name),
            backend: device_backend,
            name,
            vram_total_mb,
            vram_free_mb,
        });
    }
    devices
}

/// 解析设备行前缀（`Vulkan0` / `CUDA1` / `Device 0`），返回 `(后端, 序号)`。
///
/// `None` 表示声明的后端（`Device 0: ...` 形式），由调用方按请求后端填充。
fn parse_device_header(prefix: &str) -> Option<(Option<LocalBackend>, u32)> {
    let lower = prefix.to_ascii_lowercase();
    for (token, backend) in [
        ("vulkan", LocalBackend::Vulkan),
        ("cuda", LocalBackend::Cuda),
    ] {
        if let Some(rest) = lower.strip_prefix(token) {
            return Some((Some(backend), parse_index(rest)));
        }
    }
    if let Some(rest) = lower.strip_prefix("device") {
        return Some((None, parse_index(rest)));
    }
    None
}

/// 从 `0` / ` 01` 之类的后缀解析设备序号（无法解析时为 0）。
fn parse_index(rest: &str) -> u32 {
    rest.trim().parse::<u32>().unwrap_or(0)
}

/// 把 `NVIDIA GeForce RTX 4060 Laptop GPU (7956 MiB, 7180 MiB free)` 拆成
/// `(设备名, 显存总量 MiB, 空闲显存 MiB)`。
fn parse_device_tail(rest: &str) -> (String, u64, u64) {
    let (name, vram) = match rest.rfind('(') {
        Some(open) if rest.ends_with(')') => (
            rest[..open].trim(),
            Some(rest[open + 1..rest.len() - 1].trim()),
        ),
        _ => (rest, None),
    };
    let mut total = 0u64;
    let mut free = 0u64;
    if let Some(inner) = vram {
        let mut parts = inner.split(',');
        if let Some(first) = parts.next() {
            total = parse_mib(first).unwrap_or(0);
        }
        if let Some(second) = parts.next() {
            if second.to_ascii_lowercase().contains("free") {
                free = parse_mib(second).unwrap_or(0);
            }
        }
    }
    (name.to_string(), total, free)
}

/// 解析 `7956 MiB` / `8 GiB` / `7180 MB` 这类片段为 MiB。
fn parse_mib(token: &str) -> Option<u64> {
    let token = token.trim();
    let number_start = token.find(|ch: char| ch.is_ascii_digit())?;
    let rest = &token[number_start..];
    let number_end = rest
        .find(|ch: char| !ch.is_ascii_digit() && ch != '.')
        .unwrap_or(rest.len());
    let value: f64 = rest[..number_end].parse().ok()?;
    let unit = rest[number_end..].trim().to_ascii_lowercase();
    let mib = if unit.starts_with("gib") || unit.starts_with('g') {
        value * 1024.0
    } else {
        value
    };
    Some(mib.round() as u64)
}

/// 按设备名归类厂商（`nvidia` / `amd` / `intel` / `unknown`）。
pub fn classify_vendor(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let vendor = if ["nvidia", "geforce", "quadro", "tesla", "rtx", "gtx"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        "nvidia"
    } else if ["amd", "radeon", "ati ", "ryzen", "firepro"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        "amd"
    } else if [
        "intel",
        "arc a",
        "arc b",
        "iris",
        "uhd graphics",
        "hd graphics",
        "xe graphics",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        "intel"
    } else {
        "unknown"
    };
    Some(vendor.to_string())
}

/// 是否虚拟/软件适配器（Microsoft Basic Render Driver、RDP、VirtualBox 等）。
pub fn is_virtual_adapter(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    VIRTUAL_ADAPTER_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// 显存预算：`need = model.vramMB + 512 MiB`。
pub fn budget_for(model_vram_mb: u32, device: &GpuDevice) -> GpuBudget {
    let required_mb = u64::from(model_vram_mb) + 512;
    GpuBudget {
        required_mb,
        available_mb: device.vram_free_mb,
        fits: device.vram_free_mb >= required_mb,
    }
}

/// 推荐后端：**Vulkan 优先**，其次 CUDA。
pub fn recommend_backend(backends: &[GpuBackendStatus]) -> Option<GpuBackend> {
    for wanted in [LocalBackend::Vulkan, LocalBackend::Cuda] {
        if backends
            .iter()
            .any(|status| status.backend == wanted && status.available)
        {
            return wanted.as_gpu_backend();
        }
    }
    None
}

/// 后端组件目录名（`<backends_dir>/<variant>/`）。
pub fn backend_variant_dir(backend: LocalBackend) -> &'static str {
    backend.variant()
}

/// 拼接后端可执行文件路径。
pub fn backend_exe_path(backends_dir: &Path, backend: LocalBackend) -> PathBuf {
    backends_dir
        .join(backend_variant_dir(backend))
        .join(LLAMA_SERVER_EXE)
}

// ---------------------------------------------------------------------------
// 子进程探测（`unsafe` 禁止：只用 std::process）
// ---------------------------------------------------------------------------

/// 子进程输出（stdout / stderr 合并视图）。
#[derive(Debug, Clone)]
struct CommandOutput {
    stdout: String,
    stderr: String,
}

impl CommandOutput {
    /// stdout 优先，空则取 stderr（llama.cpp 不同构建把版本写到 stderr）。
    fn combined(&self) -> String {
        if self.stdout.trim().is_empty() {
            self.stderr.clone()
        } else {
            let mut text = self.stdout.clone();
            if !self.stderr.trim().is_empty() {
                text.push('\n');
                text.push_str(&self.stderr);
            }
            text
        }
    }
}

/// 运行 `llama-server --list-devices` 并返回合并输出。
pub fn list_devices_text(exe: &Path) -> Result<String, String> {
    let mut command = Command::new(exe);
    command.arg("--list-devices");
    let output = run_with_timeout(&mut command, LIST_DEVICES_TIMEOUT)?;
    let text = output.combined();
    if text.trim().is_empty() {
        return Err("--list-devices 无输出".to_string());
    }
    Ok(text)
}

/// 运行 `llama-server --version` 并解析版本（失败静默为 `None`）。
pub fn version_text(exe: &Path) -> Option<String> {
    if !exe.is_file() {
        return None;
    }
    let mut command = Command::new(exe);
    command.arg("--version");
    let output = run_with_timeout(&mut command, VERSION_TIMEOUT).ok()?;
    parse_version(&output.combined())
}

/// 从 `version: 2.28.2 (fe2adf0)` 之类的输出里取版本串。
pub fn parse_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = line.get(8..) {
            if lower.starts_with("version:") {
                let value = rest.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
        if let Some(index) = lower.find("llama.cpp version:") {
            let value = line.get(index + "llama.cpp version:".len()..)?.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 带超时的子进程执行（超时 kill，避免驱动异常时探测卡死）。
fn run_with_timeout(command: &mut Command, timeout: Duration) -> Result<CommandOutput, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("无法启动 {}：{err}", command_label(command)))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(err) => {
                let _ = child.kill();
                return Err(format!("等待进程退出失败：{err}"));
            }
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("探测超时（{} 秒）", timeout.as_secs()));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    let output = child
        .wait_with_output()
        .map_err(|err| format!("读取进程输出失败：{err}"))?;
    Ok(CommandOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// 错误信息里显示的可执行文件名（不泄露完整用户路径）。
fn command_label(command: &Command) -> String {
    command.get_program().to_string_lossy().to_string()
}

fn backend_label(backend: LocalBackend) -> &'static str {
    match backend {
        LocalBackend::Vulkan => "Vulkan",
        LocalBackend::Cuda => "CUDA",
        LocalBackend::Cpu => "CPU",
    }
}

fn missing_reason(backend: LocalBackend) -> &'static str {
    match backend {
        LocalBackend::Vulkan => "Vulkan 后端未安装（AMD/Intel/NVIDIA 通用，推荐下载）",
        LocalBackend::Cuda => "CUDA 后端未安装（NVIDIA 可选组件）",
        LocalBackend::Cpu => "CPU 后端未安装（应随主安装包分发）",
    }
}

fn gpu_backend_as_local(backend: GpuBackend) -> LocalBackend {
    match backend {
        GpuBackend::Vulkan => LocalBackend::Vulkan,
        GpuBackend::Cuda => LocalBackend::Cuda,
        GpuBackend::Auto => LocalBackend::Vulkan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(free: u64) -> GpuDevice {
        GpuDevice {
            index: 0,
            name: "Test GPU".to_string(),
            backend: LocalBackend::Vulkan,
            vram_total_mb: 8192,
            vram_free_mb: free,
            vendor: Some("unknown".to_string()),
            virtual_adapter: false,
        }
    }

    fn status(backend: LocalBackend, available: bool) -> GpuBackendStatus {
        GpuBackendStatus {
            backend,
            installed: available,
            available,
            exe_path: None,
            version: None,
            reason: None,
        }
    }

    #[test]
    fn budget_requires_extra_512_mib() {
        assert!(budget_for(1600, &device(8000)).fits);
        assert!(!budget_for(1600, &device(2000)).fits);
        assert_eq!(budget_for(700, &device(0)).required_mb, 1212);
        // 边界：恰好等于 required 时视为放得下。
        let budget = budget_for(1600, &device(2112));
        assert_eq!(budget.required_mb, 2112);
        assert!(budget.fits);
        assert!(!budget_for(1600, &device(2111)).fits);
    }

    #[test]
    fn probe_is_not_useable_without_backend() {
        let cache = GpuProbeCache::new();
        let result = cache.probe(None, 1600, true);
        assert!(!result.useable);
        assert_eq!(result.backends.len(), 3);
        assert!(result.issue.is_some());
        // CPU 项恒在第三位。
        assert_eq!(result.backends[2].backend, LocalBackend::Cpu);
        // 第二次命中缓存（force=false）
        let cached = cache.probe(None, 1600, false);
        assert!(cached.cached);
    }

    #[test]
    fn cache_misses_when_model_vram_changes() {
        let cache = GpuProbeCache::new();
        let first = cache.probe(None, 700, true);
        assert!(!first.cached);
        let hit = cache.probe(None, 700, false);
        assert!(hit.cached);
        // 模型变化 → 预算不同 → 重新探测。
        let second = cache.probe(None, 1600, false);
        assert!(!second.cached);
    }

    #[test]
    fn parses_real_vulkan0_sample() {
        let stdout = "Available devices:\n  Vulkan0: NVIDIA GeForce RTX 4060 Laptop GPU (7956 MiB, 7180 MiB free)\n";
        let devices = parse_list_devices(stdout, LocalBackend::Vulkan);
        assert_eq!(devices.len(), 1);
        let device = &devices[0];
        assert_eq!(device.index, 0);
        assert_eq!(device.name, "NVIDIA GeForce RTX 4060 Laptop GPU");
        assert_eq!(device.backend, LocalBackend::Vulkan);
        assert_eq!(device.vram_total_mb, 7956);
        assert_eq!(device.vram_free_mb, 7180);
        assert_eq!(device.vendor.as_deref(), Some("nvidia"));
        assert!(!device.virtual_adapter);
    }

    #[test]
    fn parses_amd_and_intel_samples() {
        let stdout = "Available devices:\n\
                      Vulkan0: AMD Radeon RX 7800 XT (16384 MiB, 15200 MiB free)\n\
                      Vulkan1: Intel(R) Arc(TM) A770 Graphics (8192 MiB, 8000 MiB free)\n";
        let devices = parse_list_devices(stdout, LocalBackend::Vulkan);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].vendor.as_deref(), Some("amd"));
        assert_eq!(devices[0].vram_free_mb, 15200);
        assert_eq!(devices[1].index, 1);
        assert_eq!(devices[1].vendor.as_deref(), Some("intel"));
    }

    #[test]
    fn parses_cuda_devices() {
        let stdout =
            "Available devices:\n  CUDA0: NVIDIA GeForce RTX 4090 (24564 MiB, 23800 MiB free)\n";
        let devices = parse_list_devices(stdout, LocalBackend::Cuda);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].backend, LocalBackend::Cuda);
        assert_eq!(devices[0].vram_total_mb, 24564);
    }

    #[test]
    fn parses_gib_units_and_device_prefix() {
        let stdout = "Available devices:\n  Device 0: Some GPU (8 GiB, 7 GiB free)\n";
        let devices = parse_list_devices(stdout, LocalBackend::Vulkan);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].vram_total_mb, 8192);
        assert_eq!(devices[0].vram_free_mb, 7168);
    }

    #[test]
    fn empty_and_none_outputs_yield_no_devices() {
        assert!(parse_list_devices("", LocalBackend::Vulkan).is_empty());
        assert!(
            parse_list_devices("Available devices:\n  (none)\n", LocalBackend::Vulkan).is_empty()
        );
        assert!(parse_list_devices("no gpu here\n", LocalBackend::Cuda).is_empty());
    }

    #[test]
    fn ignores_other_backend_device_lines() {
        let stdout =
            "Available devices:\n  CUDA0: NVIDIA GeForce RTX 4090 (24564 MiB, 1 MiB free)\n";
        assert!(parse_list_devices(stdout, LocalBackend::Vulkan).is_empty());
    }

    #[test]
    fn marks_virtual_adapters() {
        assert!(is_virtual_adapter("Microsoft Basic Render Driver"));
        assert!(is_virtual_adapter("Microsoft Basic Display Adapter"));
        assert!(is_virtual_adapter("llvmpipe (LLVM 15.0.7, 256 bits)"));
        assert!(is_virtual_adapter("VirtualBox Graphics Adapter"));
        assert!(is_virtual_adapter("VMware SVGA 3D"));
        assert!(is_virtual_adapter("Microsoft Hyper-V Video"));
        assert!(is_virtual_adapter("Google SwiftShader"));
        assert!(!is_virtual_adapter("NVIDIA GeForce RTX 4060 Laptop GPU"));
        assert!(!is_virtual_adapter("AMD Radeon RX 7800 XT"));
        // 虚拟卡解析后带 virtual_adapter 标记，探测层会过滤掉。
        let stdout =
            "Available devices:\n  Vulkan0: Microsoft Basic Render Driver (0 MiB, 0 MiB free)\n";
        let devices = parse_list_devices(stdout, LocalBackend::Vulkan);
        assert_eq!(devices.len(), 1);
        assert!(devices[0].virtual_adapter);
    }

    #[test]
    fn classifies_vendors() {
        assert_eq!(
            classify_vendor("NVIDIA GeForce RTX 4060 Laptop GPU").as_deref(),
            Some("nvidia")
        );
        assert_eq!(
            classify_vendor("AMD Radeon RX 7800 XT").as_deref(),
            Some("amd")
        );
        assert_eq!(
            classify_vendor("Intel(R) Arc(TM) A770 Graphics").as_deref(),
            Some("intel")
        );
        assert_eq!(
            classify_vendor("Intel(R) UHD Graphics 630").as_deref(),
            Some("intel")
        );
        assert_eq!(
            classify_vendor("Some Unknown GPU").as_deref(),
            Some("unknown")
        );
    }

    #[test]
    fn recommends_vulkan_over_cuda() {
        let backends = vec![
            status(LocalBackend::Vulkan, false),
            status(LocalBackend::Cuda, true),
        ];
        assert_eq!(recommend_backend(&backends), Some(GpuBackend::Cuda));

        let backends = vec![
            status(LocalBackend::Vulkan, true),
            status(LocalBackend::Cuda, true),
        ];
        assert_eq!(recommend_backend(&backends), Some(GpuBackend::Vulkan));

        let backends = vec![status(LocalBackend::Cpu, true)];
        assert_eq!(recommend_backend(&backends), None);
    }

    #[test]
    fn parses_version_lines() {
        assert_eq!(
            parse_version("version: 1 (fe2adf0)\nbuilt with MSVC").as_deref(),
            Some("1 (fe2adf0)")
        );
        assert_eq!(
            parse_version("llama.cpp version: b12128\n").as_deref(),
            Some("b12128")
        );
        assert_eq!(parse_version("no version here"), None);
    }
}
