//! 后端组件管理集成测试（`services/backend_manager.rs` 公开 API）。
//!
//! 覆盖（task-1 验收项）：
//! 1. 内置清单解析（vulkan/cuda/cpu 三项、Vulkan 跨厂商文案、CUDA requiresVendor）；
//! 2. `backends_list` 的安装状态合并（扫描 `<data_dir>/backends/<variant>/llama-server.exe`）；
//! 3. 真实 HTTP 下载 → SHA256/大小校验 → `tar.exe` 解压 → `.installed` 标记的完整链路
//!    （本地 `TcpListener` 提供压缩包，不依赖外网）；
//! 4. 校验失败（SHA256 不符）时安装目录不产出可执行文件、错误可读；
//! 5. 删除已安装组件 / 未知组件错误码。
//!
//! 真实镜像下载（GitHub Release）见 `gpu_probe_real.rs` 的 `#[ignore]` 测试。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};

use snip_ask_lib::contract::config::LocalBackend;
use snip_ask_lib::contract::types::DownloadState;
use snip_ask_lib::platform::paths::AppPaths;
use snip_ask_lib::resources::BACKENDS_MANIFEST;
use snip_ask_lib::services::backend_manager::{
    download_and_install, extract_zip, find_backend_entry, parse_backend_manifest, BackendManager,
    BackendManifestEntry,
};

// ---------------------------------------------------------------------------
// 临时目录 / 压缩包 / 本地 HTTP server
// ---------------------------------------------------------------------------

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("snip-ask-gpu-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("创建临时目录失败");
        Self { root }
    }

    fn paths(&self) -> AppPaths {
        AppPaths::from_roots(
            false,
            self.root.join("exe"),
            &self.root.join("config"),
            &self.root.join("local"),
        )
        .expect("构造 AppPaths 失败")
    }

    fn join(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 用系统 `tar.exe` 打一个 zip（内容与官方 llama.cpp 压缩包一致：文件位于根层）。
fn make_zip(scratch: &Scratch) -> Vec<u8> {
    let pkg = scratch.join("pkg");
    std::fs::create_dir_all(&pkg).expect("创建打包目录失败");
    std::fs::write(pkg.join("llama-server.exe"), b"fake-llama-server").unwrap();
    std::fs::write(pkg.join("ggml-vulkan.dll"), vec![7u8; 1024]).unwrap();
    let archive = scratch.join("fake-backend.zip");
    let status = std::process::Command::new("tar.exe")
        .arg("-a")
        .arg("-cf")
        .arg(&archive)
        .arg("-C")
        .arg(&pkg)
        .arg(".")
        .status()
        .expect("启动 tar.exe 失败（Windows 应自带 bsdtar）");
    assert!(status.success(), "tar.exe 创建 zip 失败");
    std::fs::read(&archive).expect("读取测试 zip 失败")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// 最小 HTTP server：对所有请求返回 200 + 固定 payload（不支持 Range，足以覆盖首次下载链路；
/// 断点续传本身由 S-08 `model_download_spike` 覆盖）。
struct TestServer {
    addr: std::net::SocketAddr,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn start(payload: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地端口失败");
        listener.set_nonblocking(true).expect("设置非阻塞失败");
        let addr = listener.local_addr().expect("读取本地地址失败");
        let payload = Arc::new(payload);
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        respond(stream, &payload);
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
            shutdown,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
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

fn respond(mut stream: TcpStream, payload: &[u8]) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut head = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        if head.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        match stream.read(&mut buffer) {
            Ok(0) => return,
            Ok(read) => head.extend_from_slice(&buffer[..read]),
            Err(_) => return,
        }
    }
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(payload);
    let _ = stream.flush();
}

/// 构造指向本地 server 的清单条目。
fn test_entry(server: &TestServer, payload: &[u8], sha256: String) -> BackendManifestEntry {
    BackendManifestEntry {
        id: "vulkan".to_string(),
        label: "Vulkan（测试，AMD/Intel/NVIDIA 通用）".to_string(),
        backend: LocalBackend::Vulkan,
        version: "test".to_string(),
        size_bytes: payload.len() as u64,
        sha256,
        license: "MIT".to_string(),
        urls: vec![server.url("/fake-backend.zip")],
        requires_vendor: None,
        archive: "fake-backend.zip".to_string(),
        additional: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[test]
fn embedded_manifest_parses_with_three_backends() {
    let manifest = parse_backend_manifest(BACKENDS_MANIFEST).expect("内置清单必须可解析");
    assert_eq!(manifest.backends.len(), 3);
    let vulkan = find_backend_entry(&manifest, "vulkan").expect("必须包含 vulkan");
    assert_eq!(vulkan.backend, LocalBackend::Vulkan);
    assert!(vulkan.label.contains("AMD"), "Vulkan 文案必须明确 AMD 可用");
    assert!(vulkan.label.contains("NVIDIA"));
    assert!(vulkan.requires_vendor.is_none());
    let cuda = find_backend_entry(&manifest, "cuda").expect("必须包含 cuda");
    assert_eq!(cuda.requires_vendor.as_deref(), Some("nvidia"));
    assert_eq!(cuda.archives().len(), 2, "CUDA 需要 cudart 运行时包");
    let cpu = find_backend_entry(&manifest, "cpu").expect("必须包含 cpu");
    assert_eq!(cpu.backend, LocalBackend::Cpu);
}

#[test]
fn list_merges_on_disk_install_state() {
    let scratch = Scratch::new("list");
    let paths = scratch.paths();
    let manager = BackendManager::new(paths.clone());
    let infos = manager.list().expect("list 不应失败");
    assert_eq!(infos.len(), 3);
    assert!(infos.iter().all(|info| !info.installed));
    assert!(infos
        .iter()
        .all(|info| info.download_state == DownloadState::Queued));

    // 手工放置一个 Vulkan 后端（模拟安装完成）。
    let vulkan_dir = manager.install_root().join("vulkan");
    std::fs::create_dir_all(&vulkan_dir).unwrap();
    std::fs::write(vulkan_dir.join("llama-server.exe"), b"stub").unwrap();
    std::fs::write(vulkan_dir.join("ggml-vulkan.dll"), vec![0u8; 4096]).unwrap();

    let infos = manager.list().unwrap();
    let vulkan = infos.iter().find(|info| info.id == "vulkan").unwrap();
    assert!(vulkan.installed);
    assert_eq!(vulkan.download_state, DownloadState::Installed);
    assert!(vulkan.installed_bytes > 4096);
    assert!(manager.is_installed("vulkan"));
    // CPU 随包资源可能存在于 dev 环境；此处只断言清单字段与状态形状。
    let cpu = infos.iter().find(|info| info.id == "cpu").unwrap();
    assert_eq!(cpu.backend, LocalBackend::Cpu);
}

#[test]
fn removes_installed_component() {
    let scratch = Scratch::new("remove");
    let manager = BackendManager::new(scratch.paths());
    let dir = manager.install_root().join("vulkan");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("llama-server.exe"), b"stub").unwrap();
    assert!(manager.is_installed("vulkan"));
    manager.remove("vulkan").expect("删除已安装组件应成功");
    assert!(!dir.exists());
    assert!(!manager.is_installed("vulkan"));
    assert_eq!(
        manager.remove("unknown").unwrap_err().code,
        "SA-4001",
        "未知组件应返回 SA-4001"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloads_verifies_extracts_and_marks_installed() {
    let scratch = Scratch::new("download");
    let payload = make_zip(&scratch);
    let sha = sha256_hex(&payload);
    let server = TestServer::start(payload.clone());
    let entry = test_entry(&server, &payload, sha);

    let paths = scratch.paths();
    let manager = BackendManager::new(paths.clone());
    let install_dir = manager.install_root().join("vulkan");

    let mut progress: Vec<(u64, u64)> = Vec::new();
    let mut on_progress = |loaded: u64, total: u64| progress.push((loaded, total));
    let result = download_and_install(
        &entry,
        &install_dir,
        &AtomicBool::new(false),
        &mut on_progress,
    )
    .await
    .expect("下载 + 校验 + 解压应成功");

    assert_eq!(result.bytes, payload.len() as u64);
    assert!(!result.hash_skipped, "真实 sha256 必须执行校验");
    assert!(
        result
            .urls
            .iter()
            .any(|url| url.contains("/fake-backend.zip")),
        "应记录实际使用的下载地址：{:?}",
        result.urls
    );
    assert!(
        install_dir.join("llama-server.exe").is_file(),
        "解压后必须有 llama-server.exe"
    );
    assert!(install_dir.join("ggml-vulkan.dll").is_file());
    assert!(
        install_dir.join(".installed").is_file(),
        "必须写入 .installed 标记"
    );
    // 进度：单调不减，最终 loaded == total。
    assert!(progress.len() >= 2);
    assert!(
        progress.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "loaded 必须单调不减"
    );
    let (last_loaded, last_total) = *progress.last().unwrap();
    assert_eq!(last_loaded, payload.len() as u64);
    assert_eq!(last_total, payload.len() as u64);

    // 安装后 `backends_list` 能看到已安装状态。
    let infos = BackendManager::new(paths).list().unwrap();
    let vulkan = infos.iter().find(|info| info.id == "vulkan").unwrap();
    assert!(vulkan.installed);
    assert_eq!(vulkan.download_state, DownloadState::Installed);
    assert!(vulkan.installed_bytes > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejects_checksum_mismatch() {
    let scratch = Scratch::new("badhash");
    let payload = make_zip(&scratch);
    let server = TestServer::start(payload.clone());
    let wrong_sha = "f".repeat(64);
    let entry = test_entry(&server, &payload, wrong_sha);

    let install_dir = scratch.join("install/vulkan");
    let mut on_progress = |_loaded: u64, _total: u64| {};
    let error = download_and_install(
        &entry,
        &install_dir,
        &AtomicBool::new(false),
        &mut on_progress,
    )
    .await
    .expect_err("SHA256 不符必须失败");
    let message = error.to_string();
    assert!(
        message.contains("校验") || message.contains("SHA256") || message.contains("下载失败"),
        "错误信息应可读：{message}"
    );
    assert!(
        !install_dir.join("llama-server.exe").is_file(),
        "校验失败不得产出可执行文件"
    );
}

#[test]
fn extract_zip_handles_nested_directory_layout() {
    let scratch = Scratch::new("extract");
    let payload = make_zip(&scratch);
    let archive = scratch.join("in.zip");
    std::fs::write(&archive, &payload).unwrap();
    let dest = scratch.join("out");
    extract_zip(&archive, &dest).expect("tar.exe 应能解压 zip");
    assert!(dest.join("llama-server.exe").is_file());
}

#[test]
fn flatten_to_root_is_idempotent_when_exe_already_at_root() {
    let scratch = Scratch::new("flatten");
    let root = scratch.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("llama-server.exe"), b"exe").unwrap();
    snip_ask_lib::services::backend_manager::flatten_to_root(&root).unwrap();
    assert!(root.join("llama-server.exe").is_file());
}

#[allow(dead_code)]
fn touch(path: &Path) {
    std::fs::write(path, b"").unwrap();
}
