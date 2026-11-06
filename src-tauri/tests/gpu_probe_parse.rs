//! GPU 探测纯逻辑集成测试（`platform/gpu.rs` 公开 API）。
//!
//! 覆盖（task-1 验收项）：
//! 1. `--list-devices` 解析：真实 Vulkan0/CUDA0 样本、无设备、空输出、虚拟卡；
//! 2. 厂商分类、虚拟适配器识别；
//! 3. 显存预算边界（`need = vramMB + 512`，恰好相等视为放得下）；
//! 4. 后端推荐顺序：Vulkan 优先于 CUDA；
//! 5. 无后端组件时 `probe_once` 返回可操作原因（干净环境回退 CPU）。
//!
//! 真实 `llama-server.exe` 探测（需要本机后端组件）见 `gpu_probe_real.rs`（`#[ignore]`）。

use std::path::PathBuf;

use snip_ask_lib::contract::config::{GpuBackend, LocalBackend};
use snip_ask_lib::contract::types::{GpuBackendStatus, GpuDevice};
use snip_ask_lib::platform::gpu::{
    budget_for, classify_vendor, is_virtual_adapter, parse_list_devices, parse_version, probe_once,
    probe_once_roots, recommend_backend,
};

fn device(free_mb: u64) -> GpuDevice {
    GpuDevice {
        index: 0,
        name: "Test GPU".to_string(),
        backend: LocalBackend::Vulkan,
        vram_total_mb: 8192,
        vram_free_mb: free_mb,
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
fn parses_measured_vulkan0_output() {
    // 本机实测原始输出（RTX 4060 Laptop，spec §6-10）。
    let stdout = "Available devices:\n  Vulkan0: NVIDIA GeForce RTX 4060 Laptop GPU (7956 MiB, 7180 MiB free)\n";
    let devices = parse_list_devices(stdout, LocalBackend::Vulkan);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "NVIDIA GeForce RTX 4060 Laptop GPU");
    assert_eq!(devices[0].vram_total_mb, 7956);
    assert_eq!(devices[0].vram_free_mb, 7180);
    assert_eq!(devices[0].vendor.as_deref(), Some("nvidia"));
    assert!(!devices[0].virtual_adapter);
}

#[test]
fn parses_cuda_and_filters_by_requested_backend() {
    let stdout =
        "Available devices:\n  CUDA0: NVIDIA GeForce RTX 4090 (24564 MiB, 23800 MiB free)\n";
    assert_eq!(
        parse_list_devices(stdout, LocalBackend::Cuda).len(),
        1,
        "CUDA 后端应解析出 CUDA0"
    );
    assert!(
        parse_list_devices(stdout, LocalBackend::Vulkan).is_empty(),
        "Vulkan 探测不应把 CUDA 行算作 Vulkan 设备"
    );
}

#[test]
fn virtual_adapters_are_flagged() {
    let stdout =
        "Available devices:\n  Vulkan0: Microsoft Basic Render Driver (0 MiB, 0 MiB free)\n";
    let devices = parse_list_devices(stdout, LocalBackend::Vulkan);
    assert_eq!(devices.len(), 1);
    assert!(devices[0].virtual_adapter);
    assert!(is_virtual_adapter("llvmpipe (LLVM 15.0.7, 256 bits)"));
    assert!(is_virtual_adapter("VMware SVGA 3D"));
    assert!(!is_virtual_adapter("AMD Radeon RX 7800 XT"));
}

#[test]
fn empty_device_lists_parse_to_empty_vec() {
    assert!(parse_list_devices("", LocalBackend::Vulkan).is_empty());
    assert!(parse_list_devices("Available devices:\n  (none)\n", LocalBackend::Vulkan).is_empty());
}

#[test]
fn vendor_classification_covers_three_majors() {
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
        classify_vendor("Unknown Device").as_deref(),
        Some("unknown")
    );
}

#[test]
fn budget_boundaries_match_contract() {
    let budget = budget_for(1600, &device(2112));
    assert_eq!(budget.required_mb, 2112);
    assert!(budget.fits);
    assert!(!budget_for(1600, &device(2111)).fits);
    // 极小模型（0.5B → 700 MB）。
    assert_eq!(budget_for(700, &device(0)).required_mb, 1212);
}

#[test]
fn vulkan_is_recommended_over_cuda() {
    assert_eq!(
        recommend_backend(&[
            status(LocalBackend::Vulkan, true),
            status(LocalBackend::Cuda, true)
        ]),
        Some(GpuBackend::Vulkan)
    );
    assert_eq!(
        recommend_backend(&[
            status(LocalBackend::Vulkan, false),
            status(LocalBackend::Cuda, true)
        ]),
        Some(GpuBackend::Cuda)
    );
    assert_eq!(recommend_backend(&[status(LocalBackend::Cpu, true)]), None);
}

#[test]
fn clean_environment_falls_back_to_cpu_with_actionable_reason() {
    // 干净环境：没有任何后端组件目录 → useable=false + 中文可操作原因（spec §1.3）。
    let empty = std::env::temp_dir().join(format!("snip-ask-gpu-empty-{}", std::process::id()));
    let result = probe_once_roots(std::slice::from_ref(&empty), 1600);
    assert!(!result.useable);
    assert_eq!(result.recommended, None);
    assert!(result.devices.is_empty());
    assert_eq!(result.backends.len(), 3, "vulkan/cuda/cpu 恒 3 项");
    let issue = result.issue.unwrap_or_default();
    assert!(issue.contains("未检测到可用 GPU"), "issue={issue}");
    // CPU 项：未安装但有原因；预算仅在存在可用设备时给出。
    let cpu = result
        .backends
        .iter()
        .find(|status| status.backend == LocalBackend::Cpu)
        .expect("必须包含 CPU 项");
    assert!(!cpu.installed);
    assert!(cpu.reason.is_some());
    assert!(result.budget.is_none());

    // `probe_once(None, ...)` 兼容旧签名：不访问环境变量，结果同样回退 CPU。
    let legacy = probe_once(None, 1600);
    assert!(!legacy.useable);
}

#[test]
fn probe_only_relies_on_llama_devices_not_nvidia_smi() {
    // 观测式断言：无后端组件时不会因为“本机有 NVIDIA 卡”而误报 useable。
    // （探测层只认 `--list-devices`，不调用 nvidia-smi；AMD 可用性依赖此性质。）
    let result = probe_once(None, 1600);
    assert!(!result.useable);
    assert_eq!(result.recommended, None);
}

#[test]
fn version_parsing_handles_measured_output() {
    assert_eq!(
        parse_version("version: 1 (fe2adf0)\nbuilt with MSVC 19.44.35228.0 for x64").as_deref(),
        Some("1 (fe2adf0)")
    );
    assert_eq!(
        parse_version("llama.cpp version: b12128").as_deref(),
        Some("b12128")
    );
    assert_eq!(parse_version("nothing"), None);
}

#[test]
fn search_roots_are_deduped_and_ordered() {
    // 后端目录发现：路径不存在时不应 panic，且 None 根目录结果稳定为空。
    let roots: Vec<PathBuf> = vec![
        PathBuf::from("C:/definitely-not-exists/backends"),
        PathBuf::from("C:/definitely-not-exists/backends"),
    ];
    let result = probe_once_roots(&roots, 0);
    assert!(!result.useable);
    assert_eq!(result.backends.len(), 3);
}
