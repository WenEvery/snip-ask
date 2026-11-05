//! 真机 GPU 探测与真实后端下载验证（`#[ignore]`，需要本机 llama.cpp 后端/网络）。
//!
//! 本文件是 task-1 的**真机证据**入口（spec §5 真机 GPU 验收 1、5 与 §6-10 环境）：
//! 1. `--list-devices` 真实识别（本机 RTX 4060 Laptop，Vulkan0）；
//! 2. 干净环境（只有 CPU 后端）返回 `useable=false` + 可操作原因；
//! 3. 真实 HTTP 下载 Vulkan 组件（GitHub Release）→ SHA256 校验 → `tar.exe` 解压 →
//!    再用解压出的官方 `llama-server.exe` 复测 Vulkan0。
//!
//! 运行方式（PowerShell）：
//! ```powershell
//! cd src-tauri
//! $env:SNIP_ASK_BACKENDS_DIR        = "<repo>\.verify\evidence\gpu-probe\backends"      # 含 vulkan/cuda/cpu 的根目录
//! $env:SNIP_ASK_BACKENDS_CPU_DIR    = "<repo>\.verify\evidence\gpu-probe\backends-cpu"  # 仅 cpu
//! $env:SNIP_ASK_BACKENDS_VERIFY_DIR = "<repo>\.verify\evidence\gpu-probe\install"       # 真实下载安装位置
//! cargo test --offline --test gpu_probe_real -- --ignored --nocapture
//! ```
//! 证据 JSON 由测试打印到 stdout，用 `Tee-Object` 落盘（见 `.verify/evidence/gpu-probe/`）。

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use snip_ask_lib::contract::config::{GpuBackend, LocalBackend};
use snip_ask_lib::platform::gpu::{
    find_backend_exe, list_devices_text, probe_once_roots, version_text, LLAMA_SERVER_EXE,
};
use snip_ask_lib::resources::BACKENDS_MANIFEST;
use snip_ask_lib::services::backend_manager::{
    download_and_install, find_backend_entry, parse_backend_manifest,
};

fn env_root(name: &str) -> Option<PathBuf> {
    let value = std::env::var(name).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// 证据目录（`SNIP_ASK_EVIDENCE_DIR`）：真机验证的 JSON / 原始输出落盘位置。
fn evidence_dir() -> Option<PathBuf> {
    env_root("SNIP_ASK_EVIDENCE_DIR")
}

fn write_evidence(name: &str, content: &str) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("写入证据失败（创建目录 {}）：{err}", dir.display());
        return;
    }
    let path = dir.join(name);
    if let Err(err) = std::fs::write(&path, content) {
        eprintln!("写入证据失败（{}）：{err}", path.display());
    } else {
        println!("[evidence] {}", path.display());
    }
}

fn print_json<T: serde::Serialize>(label: &str, value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("=== {label} ===\n{text}"),
        Err(err) => println!("=== {label} ===\n序列化失败：{err}"),
    }
}

fn dump_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|err| format!("{{\"error\":\"{err}\"}}"))
}

#[test]
#[ignore = "真机验证：需要本机 llama.cpp Vulkan 后端与真实 GPU"]
fn real_probe_identifies_vulkan_device() {
    let Some(root) = env_root("SNIP_ASK_BACKENDS_DIR") else {
        eprintln!("SKIP：未设置 SNIP_ASK_BACKENDS_DIR（应为 <root>/{{vulkan,cuda,cpu}}/llama-server.exe）");
        return;
    };

    // 原始 `--list-devices` 文本（证据：#1）。
    if let Some(exe) = find_backend_exe(std::slice::from_ref(&root), LocalBackend::Vulkan) {
        println!("=== vulkan exe ===\n{}", exe.display());
        println!("=== --list-devices (raw) ===");
        let raw = match list_devices_text(&exe) {
            Ok(text) => {
                println!("{text}");
                text
            }
            Err(err) => {
                println!("探测失败：{err}");
                format!("探测失败：{err}")
            }
        };
        write_evidence("vulkan-list-devices.txt", &raw);
        let version = version_text(&exe);
        println!("=== --version (raw) ===");
        println!("{version:?}");
        write_evidence(
            "vulkan-version.txt",
            &format!("{version:?}\nexe={}\n", exe.display()),
        );
    } else {
        println!(
            "=== 未在 {} 下找到 vulkan/llama-server.exe ===",
            root.display()
        );
    }

    let result = probe_once_roots(&[root], 1600);
    print_json("local_gpu_probe (model.vramMB=1600)", &result);
    write_evidence("probe.json", &dump_json(&result));

    assert!(
        result.useable,
        "真机应检测到可用 GPU：{}",
        result.issue.unwrap_or_default()
    );
    assert_eq!(
        result.recommended,
        Some(GpuBackend::Vulkan),
        "Vulkan 必须优先于 CUDA"
    );
    assert!(!result.devices.is_empty(), "应解析出至少一块 GPU");
    assert!(
        result
            .devices
            .iter()
            .any(|device| device.backend == LocalBackend::Vulkan),
        "设备列表应包含 Vulkan 设备"
    );
    assert!(
        result.devices.iter().all(|device| !device.virtual_adapter),
        "可用设备不应该是虚拟/软件适配器"
    );
    assert!(
        result.devices.iter().any(|device| device.vram_total_mb > 0),
        "应解析出显存总量"
    );
}

#[test]
#[ignore = "真机验证：需要只含 CPU 后端的目录（干净环境回退）"]
fn real_probe_cpu_only_reports_cpu_fallback() {
    let Some(root) = env_root("SNIP_ASK_BACKENDS_CPU_DIR") else {
        eprintln!("SKIP：未设置 SNIP_ASK_BACKENDS_CPU_DIR");
        return;
    };
    let result = probe_once_roots(std::slice::from_ref(&root), 1600);
    print_json("local_gpu_probe (cpu-only)", &result);
    write_evidence("probe-cpu-only.json", &dump_json(&result));

    assert!(!result.useable, "只有 CPU 后端时不应声称 GPU 可用");
    assert_eq!(result.recommended, None);
    let issue = result.issue.clone().unwrap_or_default();
    assert!(issue.contains("CPU"), "不可用原因应提示回退 CPU：{issue}");
    let cpu = result
        .backends
        .iter()
        .find(|status| status.backend == LocalBackend::Cpu)
        .expect("恒含 CPU 后端项");
    assert!(cpu.installed, "cpu/llama-server.exe 应被识别为已安装");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "真机验证：从 GitHub Release 真实下载 Vulkan 后端（约 37 MB）"]
async fn real_download_vulkan_component_then_probe() {
    let Some(root) = env_root("SNIP_ASK_BACKENDS_VERIFY_DIR") else {
        eprintln!("SKIP：未设置 SNIP_ASK_BACKENDS_VERIFY_DIR");
        return;
    };
    let manifest = parse_backend_manifest(BACKENDS_MANIFEST).expect("内置清单必须可解析");
    let entry = find_backend_entry(&manifest, "vulkan")
        .expect("清单必须含 vulkan")
        .clone();
    let install_dir = root.join("vulkan");
    let exe = install_dir.join(LLAMA_SERVER_EXE);

    if !exe.is_file() {
        let mut last_emit = std::time::Instant::now();
        let mut on_progress = |loaded: u64, total: u64| {
            if last_emit.elapsed() >= std::time::Duration::from_secs(5) || loaded >= total {
                println!("[download] {loaded}/{total} bytes");
                last_emit = std::time::Instant::now();
            }
        };
        // 硬超时：即使镜像停发/网络异常，测试也必须在有限时间内失败退出，不能挂住构建锁。
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            download_and_install(
                &entry,
                &install_dir,
                &AtomicBool::new(false),
                &mut on_progress,
            ),
        )
        .await
        .expect("Vulkan 后端下载超时（300s），请检查网络/镜像")
        .expect("Vulkan 后端真实下载 + 校验 + 解压应成功");
        let summary = serde_json::json!({
            "id": entry.id,
            "version": entry.version,
            "urls": entry.urls,
            "usedUrls": outcome.urls,
            "expectedSize": entry.size_bytes,
            "sha256": entry.sha256,
            "downloadedBytes": outcome.bytes,
            "hashSkipped": outcome.hash_skipped,
            "installDir": install_dir.display().to_string(),
        });
        print_json("backend_download (vulkan)", &summary);
        write_evidence("download.json", &dump_json(&summary));
    } else {
        println!("已存在安装，跳过下载：{}", install_dir.display());
    }

    assert!(exe.is_file(), "安装后必须存在 {}", exe.display());
    let installed_bytes = std::fs::metadata(&exe).map(|meta| meta.len()).unwrap_or(0);
    assert!(installed_bytes > 0);

    // 解压出的官方 llama-server.exe 应真实可执行并列出设备。
    println!("=== installed exe ===\n{}", exe.display());
    let version = version_text(&exe);
    println!("=== installed --version ===");
    println!("{version:?}");
    println!("=== installed --list-devices (raw) ===");
    let raw = match list_devices_text(&exe) {
        Ok(text) => {
            println!("{text}");
            text
        }
        Err(err) => {
            println!("探测失败：{err}");
            format!("探测失败：{err}")
        }
    };
    write_evidence("installed-list-devices.txt", &raw);
    write_evidence(
        "installed-version.txt",
        &format!("{version:?}\nexe={}\n", exe.display()),
    );
    let result = probe_once_roots(&[root], 1600);
    print_json("local_gpu_probe (after real install)", &result);
    write_evidence("probe-after-install.json", &dump_json(&result));
    assert!(result.useable, "真实安装后应检测到可用 GPU");
    assert_eq!(result.recommended, Some(GpuBackend::Vulkan));
}
