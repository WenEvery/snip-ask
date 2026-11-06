//! 本地推理后端组件管理（契约 v4 / S-18 体积门禁配套）。
//!
//! 背景（研究 §1.3-5）：主安装包（≤25 MiB）**只带 CPU 后端**；GPU 后端按需下载：
//! - **Vulkan（默认 GPU 组件）**：跨 AMD / Intel / NVIDIA，无需额外 runtime；
//! - **CUDA（可选）**：NVIDIA 用户可选的更快路径，需遵守 NVIDIA CUDA EULA；
//! - **CPU**：随主安装包分发，缺失时可从同一清单重新下载。
//!
//! 目录布局：`<backends_root>/<variant>/llama-server.exe`，`variant ∈ vulkan|cuda|cpu`。
//! 查找优先级：`SNIP_ASK_BACKENDS_DIR`（开发/验证钩子）→ `<data_dir>/backends`
//!（安装位置）→ `<exe_dir>/backends` / `<exe_dir>/resources/backends`（随包资源）。
//!
//! 下载实现复用 `services::model_manager::download_with_resume`
//! （HTTP Range 断点续传 + 流式 SHA256 + 大小校验 + 镜像回退），不复制整套模型下载逻辑；
//! 解压用系统 `tar.exe -xf`（bsdtar 支持 zip），失败再退 `powershell Expand-Archive`。
//! 进度/状态复用 `download:progress` / `download:state` 事件，`model` 字段填后端组件 id。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::contract::config::LocalBackend;
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::{
    DownloadProgressPayload, DownloadStatePayload, EVENT_DOWNLOAD_PROGRESS, EVENT_DOWNLOAD_STATE,
};
use crate::contract::types::{BackendInfo, DownloadId, DownloadState};
use crate::domain::ids::new_download_id;
use crate::platform::gpu::{find_backend_exe, LLAMA_SERVER_EXE};
use crate::platform::paths::AppPaths;
use crate::resources::BACKENDS_MANIFEST;
use crate::services::model_manager::{
    download_with_resume, is_placeholder_sha256, DownloadError, DownloadProgress, RetryPolicy,
};

/// 后端组件根目录环境变量（**开发/验证钩子**，产品文档标注为调试用）。
pub const BACKENDS_DIR_ENV: &str = "SNIP_ASK_BACKENDS_DIR";

/// 已安装标记文件名（与模型目录 `.installed` 约定一致）。
const INSTALLED_MARKER: &str = ".installed";
/// 下载临时子目录名（`<install_dir>/.download/`）。
const DOWNLOAD_DIR: &str = ".download";
/// 进度事件节流（契约 §7：≤ 200ms）。
const PROGRESS_THROTTLE_MS: u64 = 200;
/// HTTP 连接超时（受限网络下主源不可达时尽快切换镜像）。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 单组件压缩包较大（CUDA 约 680 MB），给足整体超时。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60 * 30);
/// 单次读超时：镜像中途停发时快速失败并切换 URL（避免后台任务无限挂起）。
const READ_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------------------------------------------------------------------
// 清单（`resources/backends.manifest.json`）
// ---------------------------------------------------------------------------

/// 后端组件清单文件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BackendManifest {
    /// 清单版本。
    pub version: u32,
    /// 是否所有条目的 sha256 都经过实下校验（当前 Vulkan 已实测，其余未逐一实下 → false）。
    pub verified: bool,
    /// 人类可读说明（失败原因、验证范围）。
    pub notes: String,
    /// 组件条目。
    pub backends: Vec<BackendManifestEntry>,
}

/// 后端组件清单条目（对应 [`BackendInfo`] 的静态部分）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BackendManifestEntry {
    /// 组件 id：`"vulkan"` / `"cuda"` / `"cpu"`。
    pub id: String,
    /// 展示名（Vulkan 需注明 AMD/Intel/NVIDIA 通用）。
    pub label: String,
    /// 对应后端枚举。
    pub backend: LocalBackend,
    /// llama.cpp 版本（如 `"b12128"`）。
    pub version: String,
    /// 主压缩包字节数。
    pub size_bytes: u64,
    /// 主压缩包 SHA256。
    pub sha256: String,
    /// 许可标识。
    pub license: String,
    /// 主压缩包下载地址（主源 + 镜像）。
    pub urls: Vec<String>,
    /// 需要的显卡厂商（`"nvidia"` 表示仅 NVIDIA 可用；Vulkan/CPU 为 `null`）。
    pub requires_vendor: Option<String>,
    /// 主压缩包文件名（缺省从 `urls` 推导）。
    pub archive: String,
    /// 额外压缩包（如 CUDA 的 cudart 运行时），按顺序下载并解压到同一目录。
    pub additional: Vec<BackendArchive>,
}

/// 额外压缩包（清单内部字段，不跨 IPC 暴露）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BackendArchive {
    /// 压缩包文件名。
    pub name: String,
    /// 压缩包字节数。
    pub size_bytes: u64,
    /// 压缩包 SHA256。
    pub sha256: String,
    /// 下载地址（主源 + 镜像）。
    pub urls: Vec<String>,
}

/// 一个待下载压缩包的完整规格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSpec {
    /// 压缩包文件名。
    pub name: String,
    /// 压缩包字节数。
    pub size_bytes: u64,
    /// 压缩包 SHA256。
    pub sha256: String,
    /// 下载地址。
    pub urls: Vec<String>,
}

impl BackendManifestEntry {
    /// 需要下载的全部压缩包（主包 + 附加包），按解压顺序排列。
    pub fn archives(&self) -> Vec<ArchiveSpec> {
        let primary = ArchiveSpec {
            name: primary_archive_name(self),
            size_bytes: self.size_bytes,
            sha256: self.sha256.clone(),
            urls: self.urls.clone(),
        };
        let mut specs = vec![primary];
        specs.extend(self.additional.iter().map(|archive| ArchiveSpec {
            name: if archive.name.trim().is_empty() {
                derive_archive_name(&archive.urls).unwrap_or_else(|| "additional.zip".to_string())
            } else {
                archive.name.clone()
            },
            size_bytes: archive.size_bytes,
            sha256: archive.sha256.clone(),
            urls: archive.urls.clone(),
        }));
        specs
    }

    /// 压缩包总字节数（进度分母）。
    pub fn total_size_bytes(&self) -> u64 {
        self.archives()
            .iter()
            .map(|archive| archive.size_bytes)
            .sum()
    }
}

/// 主压缩包文件名（清单 `archive` 优先，否则从 URL 推导）。
fn primary_archive_name(entry: &BackendManifestEntry) -> String {
    let explicit = entry.archive.trim();
    if !explicit.is_empty() {
        return explicit.to_string();
    }
    derive_archive_name(&entry.urls).unwrap_or_else(|| format!("{}.zip", entry.id))
}

/// 从 URL 的 path 末段推导压缩包文件名。
fn derive_archive_name(urls: &[String]) -> Option<String> {
    urls.iter().find_map(|raw| {
        url::Url::parse(raw).ok().and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
    })
}

/// 解析后端组件清单 JSON。
pub fn parse_backend_manifest(json: &str) -> AppResult<BackendManifest> {
    serde_json::from_str(json).map_err(|err| {
        AppError::new(
            ErrorCode::InvalidConfig,
            Some(format!("解析 resources/backends.manifest.json 失败：{err}")),
        )
    })
}

/// 在清单中查找组件条目。
pub fn find_backend_entry<'a>(
    manifest: &'a BackendManifest,
    id: &str,
) -> Option<&'a BackendManifestEntry> {
    manifest.backends.iter().find(|entry| entry.id == id)
}

// ---------------------------------------------------------------------------
// 管理器
// ---------------------------------------------------------------------------

/// 后端组件管理器。
#[derive(Debug, Clone)]
pub struct BackendManager {
    paths: AppPaths,
    manifest: BackendManifest,
    states: Arc<Mutex<HashMap<String, DownloadState>>>,
    cancel_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl BackendManager {
    /// 由应用路径构造（内嵌清单解析失败时退化为空清单并记录错误，不阻塞应用启动）。
    pub fn new(paths: AppPaths) -> Self {
        let manifest = match parse_backend_manifest(BACKENDS_MANIFEST) {
            Ok(manifest) => manifest,
            Err(err) => {
                tracing::error!(error = %err, "解析内置后端组件清单失败");
                BackendManifest::default()
            }
        };
        Self {
            paths,
            manifest,
            states: Arc::new(Mutex::new(HashMap::new())),
            cancel_flags: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 内置清单。
    pub fn manifest(&self) -> &BackendManifest {
        &self.manifest
    }

    /// 按 id 取清单条目。
    pub fn entry(&self, id: &str) -> Option<&BackendManifestEntry> {
        find_backend_entry(&self.manifest, id)
    }

    /// 管理目录（应用自管的后端安装位置，`<data_dir>/backends`）。
    pub fn install_root(&self) -> PathBuf {
        self.paths.data_dir.join("backends")
    }

    /// 后端根目录（`SNIP_ASK_BACKENDS_DIR` 优先；否则应用管理目录）。
    ///
    /// 仅用于展示/诊断；实际探测与安装应使用 [`Self::search_roots`] / [`Self::install_root`]。
    pub fn backends_root(&self) -> PathBuf {
        if let Ok(custom) = std::env::var(BACKENDS_DIR_ENV) {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
        self.install_root()
    }

    /// 按 spec §1.2 优先级返回候选根目录（已去重）。
    ///
    /// 1. `SNIP_ASK_BACKENDS_DIR`（开发/验证钩子）；
    /// 2. `<data_dir>/backends`（按需下载安装位置）；
    /// 3. `<exe_dir>/backends`、`<exe_dir>/resources/backends`（随包分发，CPU 后端）。
    pub fn search_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(custom) = std::env::var(BACKENDS_DIR_ENV) {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                roots.push(PathBuf::from(trimmed));
            }
        }
        roots.push(self.install_root());
        roots.push(self.paths.exe_dir.join("backends"));
        roots.push(self.paths.exe_dir.join("resources").join("backends"));
        dedupe_paths(roots)
    }

    /// 组件清单（静态清单字段 + 本地安装/下载状态）。
    pub fn list(&self) -> AppResult<Vec<BackendInfo>> {
        self.list_with_roots(&self.search_roots())
    }

    /// 组件清单，但使用调用方给定的根目录顺序（打包后可用 Tauri `resource_dir` 精确补位）。
    pub fn list_with_roots(&self, roots: &[PathBuf]) -> AppResult<Vec<BackendInfo>> {
        let states = self.states.lock().clone();
        Ok(self
            .manifest
            .backends
            .iter()
            .map(|entry| {
                let exe = find_backend_exe(roots, entry.backend);
                let installed = exe.is_some();
                let installed_bytes = exe
                    .as_deref()
                    .and_then(Path::parent)
                    .map(dir_size)
                    .unwrap_or(0);
                let download_state = states.get(&entry.id).copied().unwrap_or(if installed {
                    DownloadState::Installed
                } else {
                    DownloadState::Idle
                });
                BackendInfo {
                    id: entry.id.clone(),
                    label: entry.label.clone(),
                    backend: entry.backend,
                    version: entry.version.clone(),
                    size_bytes: entry.size_bytes,
                    sha256: entry.sha256.clone(),
                    license: entry.license.clone(),
                    urls: entry.urls.clone(),
                    requires_vendor: entry.requires_vendor.clone(),
                    installed,
                    installed_bytes,
                    download_state,
                }
            })
            .collect())
    }

    /// 组件是否已安装（对应后端可执行文件已就位）。
    pub fn is_installed(&self, id: &str) -> bool {
        let Some(entry) = self.entry(id) else {
            return false;
        };
        find_backend_exe(&self.search_roots(), entry.backend).is_some()
    }

    /// 当前下载状态（诊断用）。
    pub fn download_state(&self, id: &str) -> DownloadState {
        self.states
            .lock()
            .get(id)
            .copied()
            .unwrap_or(DownloadState::Idle)
    }

    /// 启动下载（返回 `DownloadId`；进度通过 `download:progress` / `download:state` 事件推送）。
    pub fn download(&self, app: &tauri::AppHandle, id: &str) -> AppResult<DownloadId> {
        let entry = self
            .entry(id)
            .cloned()
            .ok_or_else(|| backend_missing_error(id))?;
        if self.is_installed(id) {
            return Err(AppError::new(
                ErrorCode::InvalidState,
                Some(format!("后端组件 {id} 已安装，无需重复下载")),
            ));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flags
            .lock()
            .insert(id.to_string(), cancel.clone());
        self.set_state(id, DownloadState::Queued);
        emit_state(app, id, DownloadState::Queued);

        let download_id = new_download_id();
        let manager = self.clone();
        let app = app.clone();
        let id = id.to_string();
        let _handle = tauri::async_runtime::spawn(async move {
            manager.run_download(&app, &id, entry, cancel).await;
        });
        Ok(download_id)
    }

    /// 取消下载（保留 `.partial` 供下次续传）。
    pub fn cancel(&self, id: &str) -> AppResult<()> {
        if let Some(flag) = self.cancel_flags.lock().get(id).cloned() {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    /// 删除已安装组件（仅删除应用管理目录；外部目录/随包资源不可删）。
    pub fn remove(&self, id: &str) -> AppResult<()> {
        let entry = self.entry(id).ok_or_else(|| backend_missing_error(id))?;
        let managed = self.install_root().join(id);
        let external = find_backend_exe(&self.search_roots(), entry.backend)
            .filter(|exe| !exe.starts_with(&managed));
        if managed.is_dir() {
            std::fs::remove_dir_all(&managed).map_err(|err| {
                AppError::io_failed(format!(
                    "删除后端组件目录 {} 失败：{err}",
                    managed.display()
                ))
            })?;
        } else if let Some(exe) = external {
            return Err(AppError::new(
                ErrorCode::UnsupportedOperation,
                Some(format!(
                    "后端组件 {id} 来自外部目录（SNIP_ASK_BACKENDS_DIR / 随包资源），应用内不删除：{}",
                    exe.display()
                )),
            ));
        }
        self.states.lock().remove(id);
        self.cancel_flags.lock().remove(id);
        Ok(())
    }

    fn set_state(&self, id: &str, state: DownloadState) {
        self.states.lock().insert(id.to_string(), state);
    }

    /// 后台下载 + 校验 + 解压 + 安装标记 + 事件推送。
    async fn run_download(
        &self,
        app: &AppHandle,
        id: &str,
        entry: BackendManifestEntry,
        cancel: Arc<AtomicBool>,
    ) {
        self.set_state(id, DownloadState::Downloading);
        emit_state(app, id, DownloadState::Downloading);

        let mut progress = {
            let app = app.clone();
            let component_id = id.to_string();
            let mut last_emit = Instant::now() - Duration::from_millis(PROGRESS_THROTTLE_MS);
            let mut verifying_emitted = false;
            move |loaded: u64, total: u64| {
                if total > 0 && loaded >= total && !verifying_emitted {
                    verifying_emitted = true;
                    let _ = app.emit(
                        EVENT_DOWNLOAD_STATE,
                        DownloadStatePayload {
                            model: component_id.clone(),
                            state: DownloadState::Verifying,
                        },
                    );
                }
                if last_emit.elapsed() >= Duration::from_millis(PROGRESS_THROTTLE_MS)
                    || (total > 0 && loaded >= total)
                {
                    let _ = app.emit(
                        EVENT_DOWNLOAD_PROGRESS,
                        DownloadProgressPayload {
                            model: component_id.clone(),
                            loaded,
                            total,
                            speed_bps: 0,
                        },
                    );
                    last_emit = Instant::now();
                }
            }
        };

        let install_dir = self.install_root().join(id);
        let result =
            download_and_install(&entry, &install_dir, cancel.as_ref(), &mut progress).await;

        match result {
            Ok(outcome) => {
                tracing::info!(
                    component = %id,
                    bytes = outcome.bytes,
                    urls = ?outcome.urls,
                    "后端组件安装完成"
                );
                self.set_state(id, DownloadState::Installed);
                emit_state(app, id, DownloadState::Installed);
            }
            Err(BackendInstallError::Cancelled) => {
                tracing::info!(component = %id, "后端组件下载已取消");
                self.set_state(id, DownloadState::Cancelled);
                emit_state(app, id, DownloadState::Cancelled);
            }
            Err(err) => {
                tracing::warn!(component = %id, error = %err, "后端组件下载/安装失败");
                self.set_state(id, DownloadState::Failed);
                emit_state(app, id, DownloadState::Failed);
            }
        }
    }
}

/// 安装失败原因。
#[derive(Debug, thiserror::Error)]
pub enum BackendInstallError {
    /// 用户取消下载（保留 `.partial`）。
    #[error("下载已取消")]
    Cancelled,
    /// 网络/校验失败。
    #[error("下载失败：{0}")]
    Download(String),
    /// 解压/落盘失败。
    #[error("安装失败：{0}")]
    Install(String),
}

impl From<DownloadError> for BackendInstallError {
    fn from(err: DownloadError) -> Self {
        match err {
            DownloadError::Cancelled => Self::Cancelled,
            other => Self::Download(other.to_string()),
        }
    }
}

impl From<std::io::Error> for BackendInstallError {
    fn from(err: std::io::Error) -> Self {
        Self::Install(err.to_string())
    }
}

/// 安装结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    /// 全部压缩包解压后写入的字节数合计。
    pub bytes: u64,
    /// 实际成功使用的下载地址（主源不可达时会是镜像，用于诊断/证据）。
    pub urls: Vec<String>,
    /// 是否有压缩包因清单 sha256 为占位值而跳过哈希校验。
    pub hash_skipped: bool,
}

/// 下载 → SHA256/大小校验 → 解压 → 写 `.installed` 标记。
///
/// 该函数不依赖 Tauri（事件由 `on_progress` 回调转成），因此集成测试可直接调用：
/// 本地 HTTP server / 真实镜像均可用（`src-tauri/tests/gpu_probe_*.rs`）。
pub async fn download_and_install(
    entry: &BackendManifestEntry,
    install_dir: &Path,
    cancel: &AtomicBool,
    on_progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<InstallOutcome, BackendInstallError> {
    let archives = entry.archives();
    if archives.is_empty() {
        return Err(BackendInstallError::Install(
            "清单未配置下载地址".to_string(),
        ));
    }
    if !entry.additional.is_empty()
        && entry
            .additional
            .iter()
            .any(|archive| archive.urls.is_empty())
    {
        return Err(BackendInstallError::Install(
            "附加压缩包缺少下载地址".to_string(),
        ));
    }

    let work_dir = install_dir.join(DOWNLOAD_DIR);
    std::fs::create_dir_all(&work_dir)?;
    let total: u64 = archives.iter().map(|archive| archive.size_bytes).sum();
    let client = build_client(
        archives
            .iter()
            .flat_map(|archive| archive.urls.iter())
            .map(String::as_str)
            .next(),
    )?;
    // 组件下载通常配置「官方主源 + 镜像」：主源不可达时快速切换镜像，减少等待。
    let retry = RetryPolicy {
        max_retries_per_url: 1,
        base_backoff: Duration::from_millis(200),
        max_backoff: Duration::from_secs(1),
    };

    let mut base = 0u64;
    let mut used_urls: Vec<String> = Vec::new();
    let mut hash_skipped = false;
    for archive in &archives {
        if cancel.load(Ordering::SeqCst) {
            return Err(BackendInstallError::Cancelled);
        }
        let dest = work_dir.join(&archive.name);
        if is_placeholder_sha256(&archive.sha256) {
            tracing::warn!(
                component = %entry.id,
                archive = %archive.name,
                "清单 sha256 为占位值，跳过哈希校验（仅大小校验）"
            );
        }
        let mut archive_progress = |progress: DownloadProgress| {
            on_progress(base.saturating_add(progress.loaded), total);
        };
        let outcome = download_with_resume(
            &client,
            &archive.urls,
            &dest,
            archive.size_bytes,
            &archive.sha256,
            cancel,
            &retry,
            &mut archive_progress,
        )
        .await?;
        base = base.saturating_add(outcome.bytes);
        hash_skipped |= outcome.hash_skipped;
        used_urls.push(outcome.url.clone());
        on_progress(base, total);
        tracing::info!(
            component = %entry.id,
            archive = %archive.name,
            bytes = outcome.bytes,
            url = %outcome.url,
            hash_skipped = outcome.hash_skipped,
            "压缩包下载完成"
        );
    }

    on_progress(total, total);
    for archive in &archives {
        let path = work_dir.join(&archive.name);
        extract_zip(&path, install_dir)?;
        tracing::info!(
            component = %entry.id,
            archive = %archive.name,
            "压缩包已解压"
        );
    }
    flatten_to_root(install_dir).map_err(BackendInstallError::Install)?;

    let exe = install_dir.join(LLAMA_SERVER_EXE);
    if !exe.is_file() {
        return Err(BackendInstallError::Install(format!(
            "解压后未在 {} 找到 {LLAMA_SERVER_EXE}",
            install_dir.display()
        )));
    }
    std::fs::write(install_dir.join(INSTALLED_MARKER), entry.version.as_bytes())?;
    if let Err(err) = std::fs::remove_dir_all(&work_dir) {
        tracing::debug!(error = %err, "清理下载临时目录失败（可忽略）");
    }
    Ok(InstallOutcome {
        bytes: base,
        urls: used_urls,
        hash_skipped,
    })
}

/// 构造真实下载用 HTTP 客户端。
///
/// v1.1.0：接入 `advanced.proxy`（启动时注入 `platform::http` 的进程级代理）。
/// 首个下载地址为回环（测试 mock server / 本地镜像）时 `no_proxy()`，避免本机系统代理
/// 不绕过 127.0.0.1 导致下载被代理拦截（见 `platform::http`）。
fn build_client(first_url: Option<&str>) -> Result<reqwest::Client, BackendInstallError> {
    crate::platform::http::client_builder_for(first_url.unwrap_or_default())
        .user_agent(concat!("snip-ask/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        // 单次请求总时长上限（大组件最多 680 MB）。
        .timeout(REQUEST_TIMEOUT)
        // 镜像/网络中途停发时快速失败并切换下一个 URL，避免无限等待（真机验证曾触发挂死）。
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(|err| BackendInstallError::Download(format!("创建 HTTP 客户端失败：{err}")))
}

/// 解压 zip 到目标目录：`tar.exe -xf`（bsdtar 支持 zip），失败再 `Expand-Archive`。
pub fn extract_zip(archive: &Path, dest: &Path) -> Result<(), BackendInstallError> {
    std::fs::create_dir_all(dest)?;
    match run_tar_extract(archive, dest) {
        Ok(()) => Ok(()),
        Err(tar_err) => {
            tracing::debug!(error = %tar_err, "tar.exe 解压失败，改用 Expand-Archive");
            run_expand_archive(archive, dest).map_err(|ps_err| {
                BackendInstallError::Install(format!(
                    "解压 {} 失败：tar.exe（{tar_err}）；Expand-Archive（{ps_err}）",
                    archive.display()
                ))
            })
        }
    }
}

fn run_tar_extract(archive: &Path, dest: &Path) -> Result<(), String> {
    let mut command = Command::new("tar.exe");
    command
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .output()
        .map_err(|err| format!("无法启动 tar.exe：{err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "退出码 {}：{}",
        output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        stderr.trim()
    ))
}

fn run_expand_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    let archive = quote_ps(archive);
    let dest = quote_ps(dest);
    let script = format!(
        "Expand-Archive -LiteralPath '{archive}' -DestinationPath '{dest}' -Force -ErrorAction Stop"
    );
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .output()
        .map_err(|err| format!("无法启动 powershell.exe：{err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(stderr.trim().to_string())
}

/// PowerShell 单引号字符串转义（`'` → `''`）。
fn quote_ps(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

/// 文件/目录是否已存在（`Path::exists` 对断链返回 false）。
fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// 若 `llama-server.exe` 位于解压目录的嵌套子目录（官方 zip 布局可能多一层），
/// 把该层内容平铺到目标根目录。
pub fn flatten_to_root(root: &Path) -> Result<(), String> {
    if root.join(LLAMA_SERVER_EXE).is_file() {
        return Ok(());
    }
    let Some(found) = find_exe_dir(root, 3) else {
        return Ok(());
    };
    if found == root {
        return Ok(());
    }
    let entries =
        std::fs::read_dir(&found).map_err(|err| format!("读取 {} 失败：{err}", found.display()))?;
    for entry in entries.flatten() {
        let from = entry.path();
        let Some(name) = from.file_name() else {
            continue;
        };
        let to = root.join(name);
        if exists(&to) {
            continue;
        }
        std::fs::rename(&from, &to)
            .map_err(|err| format!("移动 {} 失败：{err}", from.display()))?;
    }
    // 平铺后删除空目录（忽略非空/失败）。
    let _ = std::fs::remove_dir(&found);
    Ok(())
}

/// 在 `dir` 下最多 `depth` 层寻找包含 `llama-server.exe` 的目录。
fn find_exe_dir(dir: &Path, depth: u32) -> Option<PathBuf> {
    if dir.join(LLAMA_SERVER_EXE).is_file() {
        return Some(dir.to_path_buf());
    }
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_exe_dir(&path, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

/// 目录占用字节数（递归；失败项计 0）。
pub fn dir_size(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            total = total.saturating_add(dir_size(&entry.path()));
        }
    }
    total
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique: Vec<PathBuf> = Vec::with_capacity(paths.len());
    for path in paths {
        if !unique.iter().any(|existing| existing == &path) {
            unique.push(path);
        }
    }
    unique
}

fn backend_missing_error(id: &str) -> AppError {
    AppError::new(
        ErrorCode::ModelMissing,
        Some(format!("后端组件 {id} 不在清单中")),
    )
    .with_fallback(vec!["open_settings"])
}

fn emit_state(app: &AppHandle, id: &str, state: DownloadState) {
    let _ = app.emit(
        EVENT_DOWNLOAD_STATE,
        DownloadStatePayload {
            model: id.to_string(),
            state,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths() -> AppPaths {
        let root = std::env::temp_dir().join(format!("snip-ask-backends-{}", uuid::Uuid::new_v4()));
        AppPaths::from_roots(
            false,
            PathBuf::from("C:/tmp/snipask-exe"),
            &root.join("config"),
            &root.join("local"),
        )
        .unwrap()
    }

    fn manifest() -> BackendManifest {
        parse_backend_manifest(BACKENDS_MANIFEST).unwrap()
    }

    #[test]
    fn embedded_manifest_has_three_backends() {
        let manifest = manifest();
        let ids: Vec<&str> = manifest
            .backends
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(ids, vec!["vulkan", "cuda", "cpu"]);
        let vulkan = find_backend_entry(&manifest, "vulkan").unwrap();
        assert_eq!(vulkan.backend, LocalBackend::Vulkan);
        assert!(vulkan.requires_vendor.is_none());
        assert_eq!(vulkan.version, "b12128");
        assert!(vulkan.size_bytes > 0);
        assert_eq!(vulkan.sha256.len(), 64);
        assert!(vulkan.license.contains("MIT"));
        assert!(!vulkan.urls.is_empty());
        // Vulkan 的展示文案必须明确跨厂商（AMD 可用性验收项）。
        assert!(vulkan.label.contains("AMD"));
        assert!(vulkan.label.contains("NVIDIA"));

        let cuda = find_backend_entry(&manifest, "cuda").unwrap();
        assert_eq!(cuda.backend, LocalBackend::Cuda);
        assert_eq!(cuda.requires_vendor.as_deref(), Some("nvidia"));
        // CUDA 需要额外 cudart 运行时包。
        assert_eq!(cuda.archives().len(), 2);
        assert!(cuda.archives()[1].name.contains("cudart"));

        let cpu = find_backend_entry(&manifest, "cpu").unwrap();
        assert_eq!(cpu.backend, LocalBackend::Cpu);
    }

    #[test]
    fn archive_names_and_totals_are_derived() {
        let manifest = manifest();
        let vulkan = find_backend_entry(&manifest, "vulkan").unwrap();
        let archives = vulkan.archives();
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].name, "llama-b12128-bin-win-vulkan-x64.zip");
        assert_eq!(vulkan.total_size_bytes(), vulkan.size_bytes);

        let cuda = find_backend_entry(&manifest, "cuda").unwrap();
        let total: u64 = cuda
            .archives()
            .iter()
            .map(|archive| archive.size_bytes)
            .sum();
        assert_eq!(cuda.total_size_bytes(), total);
        assert!(cuda.total_size_bytes() > cuda.size_bytes);
    }

    #[test]
    fn unknown_component_is_rejected() {
        let manager = BackendManager::new(temp_paths());
        assert!(manager.entry("nope").is_none());
        assert_eq!(manager.list().unwrap().len(), 3);
    }

    #[test]
    fn list_merges_install_state_from_disk() {
        let paths = temp_paths();
        let root = paths.data_dir.join("backends");
        let vulkan_dir = root.join("vulkan");
        std::fs::create_dir_all(&vulkan_dir).unwrap();
        std::fs::write(vulkan_dir.join(LLAMA_SERVER_EXE), b"stub").unwrap();
        std::fs::write(vulkan_dir.join("ggml-vulkan.dll"), vec![0u8; 32]).unwrap();

        let manager = BackendManager::new(paths.clone());
        let infos = manager.list().unwrap();
        let vulkan = infos.iter().find(|info| info.id == "vulkan").unwrap();
        assert!(vulkan.installed);
        assert_eq!(vulkan.download_state, DownloadState::Installed);
        assert!(vulkan.installed_bytes >= 36);
        assert!(manager.is_installed("vulkan"));
        // 未安装组件默认状态为 idle（未下载）。
        let cuda = infos.iter().find(|info| info.id == "cuda").unwrap();
        assert!(!cuda.installed);
        assert_eq!(cuda.download_state, DownloadState::Idle);

        std::fs::remove_dir_all(&paths.data_dir).unwrap();
    }

    #[test]
    fn remove_deletes_managed_component_dir() {
        let paths = temp_paths();
        let root = paths.data_dir.join("backends");
        let cpu_dir = root.join("cpu");
        std::fs::create_dir_all(&cpu_dir).unwrap();
        std::fs::write(cpu_dir.join(LLAMA_SERVER_EXE), b"stub").unwrap();
        let manager = BackendManager::new(paths.clone());
        assert!(manager.is_installed("cpu"));
        manager.remove("cpu").unwrap();
        assert!(!cpu_dir.exists());
        assert!(!manager.is_installed("cpu"));
        std::fs::remove_dir_all(&paths.data_dir).unwrap();
    }

    #[test]
    fn remove_rejects_unknown_component() {
        let manager = BackendManager::new(temp_paths());
        assert_eq!(manager.remove("nope").unwrap_err().code, "SA-4001");
    }

    #[test]
    fn flatten_moves_nested_exe_to_root() {
        let root = std::env::temp_dir().join(format!("snip-ask-flatten-{}", uuid::Uuid::new_v4()));
        let nested = root.join("llama-b12128-bin-win-vulkan-x64");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(LLAMA_SERVER_EXE), b"exe").unwrap();
        std::fs::write(nested.join("ggml-vulkan.dll"), b"dll").unwrap();
        flatten_to_root(&root).unwrap();
        assert!(root.join(LLAMA_SERVER_EXE).is_file());
        assert!(root.join("ggml-vulkan.dll").is_file());
        assert!(!nested.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dir_size_walks_recursively() {
        let root = std::env::temp_dir().join(format!("snip-ask-size-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a/one.bin"), vec![0u8; 10]).unwrap();
        std::fs::write(root.join("two.bin"), vec![0u8; 5]).unwrap();
        assert_eq!(dir_size(&root), 15);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
