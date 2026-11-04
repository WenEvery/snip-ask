//! 本地模型管理（M12 / 契约 §12）。
//!
//! - 清单来自内嵌 `resources/models.manifest.json`；
//! - 安装状态扫描 `<models_dir>/<id>/.installed`；
//! - **骨架模式**（`advanced.skeletonMode = true`，默认）：每 100ms 推进 4%，到 100% 后
//!   `verifying → installed`，写入空 `.installed` 标记；清单中不存在的 id 直接
//!   `download:state(failed)`（契约 §12，行为保持不变，S-07 离线验证依赖它）；
//! - **真实模式**（`skeletonMode = false`，S-08）：reqwest + HTTP Range 断点续传 +
//!   流式 SHA256 + 文件大小校验 + 多镜像回退。真实/骨架分支由
//!   [`ModelManager::set_skeleton_mode`] 注入的开关决定，默认仍为骨架模式。
//!
//! 真实下载核心逻辑见 [`download_with_resume`]：它完全不依赖 Tauri（`AppHandle` 只出现在
//! manager 的事件回调里），因此可在 `src-tauri/tests/` 的集成测试中用本地 `TcpListener`
//! HTTP server 直接验证；服务层只负责状态机、事件节流与 `.installed` 标记。
//!
//! 已知集成剩余项（见 `docs/spikes/S-08-*.md`）：
//! - `ModelManager::load` 目前拿不到配置，真实/骨架开关需由命令层 / `AppState::initialize`
//!   调用 [`ModelManager::set_skeleton_mode`] 注入；
//! - 下载客户端的 `advanced.proxy` 代理配置尚未接入（`TODO(real)`）。

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::{
    DownloadProgressPayload, DownloadStatePayload, EVENT_DOWNLOAD_PROGRESS, EVENT_DOWNLOAD_STATE,
};
use crate::contract::types::{
    DownloadId, DownloadState, ModelInfo, ModelManifest, ModelManifestEntry,
};
use crate::domain::ids::new_download_id;
use crate::domain::models::{
    entry_to_info, find_entry, installed_count, merge_installed, parse_manifest,
};
use crate::platform::paths::AppPaths;

/// 骨架下载进度推进间隔（契约 §12：每 100ms 4%）。
const DOWNLOAD_STEP_MS: u64 = 100;
/// 进度事件节流（契约 §7：≤ 200ms）。
const PROGRESS_THROTTLE_MS: u64 = 200;
/// 断点临时文件后缀：`<file>.partial`（校验通过后 rename 为最终文件）。
pub const PARTIAL_SUFFIX: &str = ".partial";
/// 速度滑动平均窗口。
const SPEED_WINDOW: Duration = Duration::from_secs(5);
/// 续传时计算已有前缀哈希的读块大小。
const HASH_BLOCK_BYTES: usize = 256 * 1024;
/// HTTP 客户端连接超时。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// HTTP 客户端整体超时（大模型 1~3GB，给足余量；断流由 body 读错误触发重试）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// 模型管理器（克隆后可在异步任务中使用）。
#[derive(Clone)]
pub struct ModelManager {
    models_dir: PathBuf,
    manifest: ModelManifest,
    states: Arc<parking_lot::Mutex<HashMap<String, DownloadState>>>,
    installed: Arc<parking_lot::Mutex<HashMap<String, u64>>>,
    cancel_flags: Arc<parking_lot::Mutex<HashMap<String, Arc<AtomicBool>>>>,
    /// 是否骨架模式（默认 true；真实下载只在 false 时启用）。
    skeleton_mode: Arc<AtomicBool>,
}

impl ModelManager {
    /// 加载内置清单并扫描已安装模型。
    pub fn load(paths: &AppPaths) -> AppResult<Self> {
        let manifest = parse_manifest(crate::resources::MODELS_MANIFEST)?;
        let installed = scan_installed(&paths.models_dir, &manifest);
        Ok(Self {
            models_dir: paths.models_dir.clone(),
            manifest,
            states: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            installed: Arc::new(parking_lot::Mutex::new(installed)),
            cancel_flags: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            skeleton_mode: Arc::new(AtomicBool::new(true)),
        })
    }

    /// 模型目录。
    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    /// 清单。
    pub fn manifest(&self) -> &ModelManifest {
        &self.manifest
    }

    /// 设置是否骨架模式。真实下载（HTTP Range 断点续传）仅在 `false` 时启用；
    /// 默认 `true` 时行为与契约 §12 的骨架下载完全一致。
    ///
    /// 调用方：命令层 / `AppState::initialize`（读取 `advanced.skeletonMode` 后注入）。
    pub fn set_skeleton_mode(&self, skeleton: bool) {
        self.skeleton_mode.store(skeleton, Ordering::SeqCst);
    }

    /// 当前是否骨架模式。
    pub fn is_skeleton_mode(&self) -> bool {
        self.skeleton_mode.load(Ordering::SeqCst)
    }

    /// 当前模型列表（清单静态字段 + 安装状态 + 下载状态）。
    pub fn list(&self) -> Vec<ModelInfo> {
        let installed = self.installed.lock().clone();
        let states = self.states.lock().clone();
        merge_installed(&self.manifest, &installed, &states)
    }

    /// 已安装模型数。
    pub fn installed_count(&self) -> u32 {
        let installed = self.installed.lock();
        installed_count(&merge_installed(
            &self.manifest,
            &installed,
            &HashMap::new(),
        ))
    }

    /// 模型是否已安装。
    pub fn is_installed(&self, id: &str) -> bool {
        self.installed.lock().contains_key(id)
    }

    /// 重新扫描磁盘安装状态。
    pub fn refresh_installed(&self) {
        let scanned = scan_installed(&self.models_dir, &self.manifest);
        *self.installed.lock() = scanned;
    }

    fn set_state(&self, id: &str, state: DownloadState) {
        self.states.lock().insert(id.to_string(), state);
    }

    fn state_of(&self, id: &str) -> DownloadState {
        self.states
            .lock()
            .get(id)
            .copied()
            .unwrap_or(DownloadState::Queued)
    }

    /// 开始下载；返回下载 ID。
    ///
    /// - 骨架模式：占位进度（每 100ms 4%）；
    /// - 真实模式：HTTP Range 断点续传 + SHA256/大小校验 + 镜像回退。
    pub fn start_download(&self, app: &AppHandle, id: &str) -> AppResult<DownloadId> {
        let entry = find_entry(&self.manifest, id)
            .cloned()
            .ok_or_else(|| model_missing_error(id))?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flags
            .lock()
            .insert(id.to_string(), cancel.clone());
        self.set_state(id, DownloadState::Queued);
        emit_state(app, id, DownloadState::Queued);

        let download_id = new_download_id();
        let manager = self.clone();
        let app = app.clone();
        let model_id = id.to_string();
        let skeleton = self.is_skeleton_mode();
        let _handle = tauri::async_runtime::spawn(async move {
            if skeleton {
                manager
                    .run_skeleton_download(&app, &model_id, entry.size_bytes.max(1), cancel)
                    .await;
            } else {
                manager
                    .run_real_download(&app, &model_id, entry, cancel)
                    .await;
            }
        });
        Ok(download_id)
    }

    /// 骨架下载（契约 §12，行为与 M12 初始骨架逐字一致，S-07 依赖）。
    async fn run_skeleton_download(
        &self,
        app: &AppHandle,
        id: &str,
        total: u64,
        cancel: Arc<AtomicBool>,
    ) {
        self.set_state(id, DownloadState::Downloading);
        emit_state(app, id, DownloadState::Downloading);

        let started = Instant::now();
        let mut last_emit = Instant::now() - Duration::from_millis(PROGRESS_THROTTLE_MS);
        let mut percent: u64 = 0;
        loop {
            if cancel.load(Ordering::SeqCst) {
                self.set_state(id, DownloadState::Cancelled);
                emit_state(app, id, DownloadState::Cancelled);
                return;
            }
            if percent >= 100 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(DOWNLOAD_STEP_MS)).await;
            percent = (percent + 4).min(100);
            let loaded = total * percent / 100;
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            let speed_bps = (loaded as f64 / elapsed) as u64;
            if last_emit.elapsed() >= Duration::from_millis(PROGRESS_THROTTLE_MS) || percent == 100
            {
                let _ = app.emit(
                    EVENT_DOWNLOAD_PROGRESS,
                    DownloadProgressPayload {
                        model: id.to_string(),
                        loaded,
                        total,
                        speed_bps,
                    },
                );
                last_emit = Instant::now();
            }
        }

        if cancel.load(Ordering::SeqCst) {
            self.set_state(id, DownloadState::Cancelled);
            emit_state(app, id, DownloadState::Cancelled);
            return;
        }

        // 校验（骨架模式直接通过）。
        self.set_state(id, DownloadState::Verifying);
        emit_state(app, id, DownloadState::Verifying);
        tokio::time::sleep(Duration::from_millis(DOWNLOAD_STEP_MS)).await;

        // 写入空的 .installed 标记（契约 §12）。
        let dir = self.models_dir.join(id);
        if let Err(err) = std::fs::create_dir_all(&dir) {
            tracing::warn!(error = %err, model = %id, "创建模型目录失败");
        }
        if let Err(err) = std::fs::write(dir.join(".installed"), b"") {
            tracing::warn!(error = %err, model = %id, "写入 .installed 标记失败");
        }
        self.installed.lock().insert(id.to_string(), total);
        self.set_state(id, DownloadState::Installed);
        emit_state(app, id, DownloadState::Installed);
    }

    /// 真实下载（S-08）：HTTP Range 断点续传 + 流式 SHA256 + 大小校验 + 镜像回退。
    async fn run_real_download(
        &self,
        app: &AppHandle,
        id: &str,
        entry: ModelManifestEntry,
        cancel: Arc<AtomicBool>,
    ) {
        self.set_state(id, DownloadState::Downloading);
        emit_state(app, id, DownloadState::Downloading);

        let dir = self.models_dir.join(id);
        if let Err(err) = std::fs::create_dir_all(&dir) {
            tracing::warn!(error = %err, model = %id, "创建模型目录失败");
            self.set_state(id, DownloadState::Failed);
            emit_state(app, id, DownloadState::Failed);
            return;
        }

        let hash_skipped = is_placeholder_sha256(&entry.sha256);
        if hash_skipped {
            tracing::warn!(
                model = %id,
                sha256 = %entry.sha256,
                "清单 sha256 为占位值，跳过哈希校验；接入真实下载前必须替换为官方 sha256"
            );
        }

        let filename = download_filename(&entry);
        let dest = dir.join(&filename);
        tracing::info!(
            model = %id,
            file = %filename,
            size = entry.size_bytes,
            urls = entry.urls.len(),
            "开始真实下载（HTTP Range 断点续传）"
        );

        let client = match build_http_client() {
            Ok(client) => client,
            Err(err) => {
                tracing::warn!(model = %id, error = %err, "创建 HTTP 客户端失败");
                self.set_state(id, DownloadState::Failed);
                emit_state(app, id, DownloadState::Failed);
                return;
            }
        };

        // 事件节流 ≤200ms；loaded 单调递增（服务端忽略 Range 导致整段重下时事件也不回退）。
        let total_hint = entry.size_bytes;
        let mut progress = {
            let app = app.clone();
            let model_id = id.to_string();
            let mut last_emit = Instant::now() - Duration::from_millis(PROGRESS_THROTTLE_MS);
            let mut last_loaded: u64 = 0;
            let mut verifying_emitted = false;
            move |p: DownloadProgress| {
                let loaded = p.loaded.max(last_loaded);
                last_loaded = loaded;
                let total = if p.total > 0 { p.total } else { total_hint };
                if total > 0 && loaded >= total && !verifying_emitted {
                    verifying_emitted = true;
                    let _ = app.emit(
                        EVENT_DOWNLOAD_STATE,
                        DownloadStatePayload {
                            model: model_id.clone(),
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
                            model: model_id.clone(),
                            loaded,
                            total,
                            speed_bps: p.speed_bps,
                        },
                    );
                    last_emit = Instant::now();
                }
            }
        };

        let result = download_with_resume(
            &client,
            &entry.urls,
            &dest,
            entry.size_bytes,
            &entry.sha256,
            &cancel,
            &RetryPolicy::default(),
            &mut progress,
        )
        .await;

        match result {
            Ok(outcome) => {
                if outcome.hash_skipped {
                    tracing::warn!(
                        model = %id,
                        actual_sha256 = %outcome.sha256,
                        url_host = %url_host(&outcome.url),
                        "下载完成但未校验哈希（清单 sha256 为占位值）；请用实际哈希更新清单并在 CI 中校验"
                    );
                } else {
                    tracing::info!(
                        model = %id,
                        bytes = outcome.bytes,
                        url_host = %url_host(&outcome.url),
                        "下载并通过 SHA256/大小校验"
                    );
                }
                // 写入空的 .installed 标记（保持与 scan_installed / 契约 §12 兼容）。
                if let Err(err) = std::fs::write(dir.join(".installed"), b"") {
                    tracing::warn!(error = %err, model = %id, "写入 .installed 标记失败");
                    self.set_state(id, DownloadState::Failed);
                    emit_state(app, id, DownloadState::Failed);
                    return;
                }
                self.installed.lock().insert(id.to_string(), outcome.bytes);
                self.set_state(id, DownloadState::Installed);
                emit_state(app, id, DownloadState::Installed);
            }
            Err(DownloadError::Cancelled) => {
                tracing::info!(model = %id, "下载已取消，保留 .partial 供下次续传");
                self.set_state(id, DownloadState::Cancelled);
                emit_state(app, id, DownloadState::Cancelled);
            }
            Err(err) => {
                tracing::warn!(model = %id, error = %err, "模型下载失败");
                self.set_state(id, DownloadState::Failed);
                emit_state(app, id, DownloadState::Failed);
            }
        }
    }

    /// 取消下载；返回是否命中。取消会保留 `.partial` 供下次续传。
    pub fn cancel(&self, app: &AppHandle, id: &str) -> bool {
        let Some(flag) = self.cancel_flags.lock().get(id).cloned() else {
            return false;
        };
        flag.store(true, Ordering::SeqCst);
        self.set_state(id, DownloadState::Cancelled);
        emit_state(app, id, DownloadState::Cancelled);
        true
    }

    /// 删除已安装模型（清单中不存在返回 `SA-4001`）。
    pub fn remove(&self, id: &str) -> AppResult<()> {
        if find_entry(&self.manifest, id).is_none() {
            return Err(model_missing_error(id));
        }
        let dir = self.models_dir.join(id);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir).map_err(|err| {
                AppError::io_failed(format!("删除模型 {} 失败：{err}", dir.display()))
            })?;
        }
        self.installed.lock().remove(id);
        self.states.lock().remove(id);
        Ok(())
    }

    /// 当前下载状态（诊断用）。
    pub fn download_state(&self, id: &str) -> DownloadState {
        self.state_of(id)
    }

    /// 按 id 取清单条目。
    pub fn entry(&self, id: &str) -> Option<&ModelManifestEntry> {
        find_entry(&self.manifest, id)
    }
}

/// 真实下载进度快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    /// 已下载字节（跨续传单调递增）。
    pub loaded: u64,
    /// 总字节（来自 `Content-Range`/`Content-Length`/清单；未知时为 0）。
    pub total: u64,
    /// 滑动平均速度（B/s）。
    pub speed_bps: u64,
}

/// 真实下载结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOutcome {
    /// 最终文件字节数。
    pub bytes: u64,
    /// 实际 SHA256（小写十六进制；即使清单为占位值也会计算，便于回填清单）。
    pub sha256: String,
    /// 最终成功使用的 URL（用于记录镜像回退结果）。
    pub url: String,
    /// 是否因清单 sha256 为占位值而跳过哈希校验。
    pub hash_skipped: bool,
}

/// 真实下载错误。
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// 用户取消（保留 `.partial`）。
    #[error("下载已取消")]
    Cancelled,
    /// 网络错误（可重试或已耗尽重试）。
    #[error("网络错误：{0}")]
    Http(String),
    /// 本地文件错误（不可重试）。
    #[error("文件错误：{0}")]
    File(String),
    /// 校验失败（大小 / SHA256），`.partial` 已删除。
    #[error("校验失败：{0}")]
    Verify(String),
    /// 所有镜像均失败。
    #[error("全部镜像失败：{0}")]
    MirrorsExhausted(String),
}

/// 重试策略（按 URL 独立计数；指数退避）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// 单个 URL 的最大重试次数（不含首次请求）。
    pub max_retries_per_url: u32,
    /// 首次退避时长。
    pub base_backoff: Duration,
    /// 退避上限。
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries_per_url: 3,
            base_backoff: Duration::from_millis(200),
            max_backoff: Duration::from_secs(2),
        }
    }
}

impl RetryPolicy {
    /// 第 `attempt` 次重试（0 起）的退避时长：base × 2^attempt，封顶 `max_backoff`。
    fn backoff(&self, attempt: u32) -> Duration {
        let factor = 1u32 << attempt.min(20);
        (self.base_backoff * factor).min(self.max_backoff)
    }
}

/// `.partial` 路径：`<file>.partial`。
pub fn partial_path(dest: &Path) -> PathBuf {
    let mut name = dest
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_else(|| std::ffi::OsString::from("model"));
    name.push(PARTIAL_SUFFIX);
    dest.with_file_name(name)
}

/// 清单 sha256 是否为占位/无效值（空、全 `0`、或非 64 位十六进制）。
///
/// 占位值（当前清单为 64 个 `0`）时跳过哈希校验并由调用方记录 `warn`。
pub fn is_placeholder_sha256(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty()
        || trimmed.len() != 64
        || trimmed.chars().all(|ch| ch == '0')
        || !trimmed.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// 从 URL 推导落盘文件名（清单条目没有 file 字段，取 URL path 最后一段）。
pub fn download_filename(entry: &ModelManifestEntry) -> String {
    entry
        .urls
        .iter()
        .find_map(|raw| {
            url::Url::parse(raw).ok().and_then(|parsed| {
                parsed
                    .path_segments()
                    .and_then(|mut segments| segments.next_back())
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
            })
        })
        .unwrap_or_else(|| format!("{}.bin", entry.id))
}

/// 真实下载：镜像回退 + 断点续传 + 流式 SHA256 + 大小校验。
///
/// 行为：
/// 1. 目标写 `<dest>.partial`；已存在则发 `Range: bytes=<len>-`，服务端 206 续写；
///    服务端返回 200（忽略 Range）则丢弃断点从头写；
/// 2. 每个 URL 独立重试（指数退避）；404/4xx 立即切换下一个镜像，5xx/408/429/网络错误重试；
/// 3. 流式 SHA256：续传时先读已有前缀喂 hasher，再逐块 `update`；
/// 4. 大小校验 `expected_size > 0` 时生效；哈希校验在清单 sha256 非占位值时生效；
///    校验失败删除 `.partial` 并整轮重下一次（最多 2 轮）；
/// 5. 全部通过后删除同名旧文件并 `rename(.partial → dest)`；`dest` 才被视为可安装文件，
///    `.installed` 标记由调用方（manager）写入；
/// 6. 取消时保留 `.partial` 并返回 [`DownloadError::Cancelled`]。
pub async fn download_with_resume<F>(
    client: &reqwest::Client,
    urls: &[String],
    dest: &Path,
    expected_size: u64,
    expected_sha256: &str,
    cancel: &AtomicBool,
    retry: &RetryPolicy,
    mut on_progress: F,
) -> Result<DownloadOutcome, DownloadError>
where
    F: FnMut(DownloadProgress),
{
    if urls.is_empty() {
        return Err(DownloadError::MirrorsExhausted(
            "清单未配置下载地址".to_string(),
        ));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            DownloadError::File(format!("创建目录 {} 失败：{err}", parent.display()))
        })?;
    }

    let partial = partial_path(dest);
    let hash_skipped = is_placeholder_sha256(expected_sha256);

    // 校验失败（大小/哈希不符）时删除 .partial 并整轮重下一次；最多 2 轮。
    for round in 0..2u32 {
        if cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::Cancelled);
        }
        let fetched = fetch_with_mirrors(
            client,
            urls,
            &partial,
            expected_size,
            cancel,
            retry,
            &mut on_progress,
        )
        .await?;

        if expected_size > 0 && fetched.bytes != expected_size {
            let err = DownloadError::Verify(format!(
                "文件大小不符：期望 {expected_size} 字节，实际 {} 字节",
                fetched.bytes
            ));
            let _ = std::fs::remove_file(&partial);
            if round == 0 {
                tracing::warn!(error = %err, "校验失败，删除 .partial 后重下一次");
                continue;
            }
            return Err(err);
        }

        if !hash_skipped {
            let expected = expected_sha256.trim().to_ascii_lowercase();
            if fetched.sha256 != expected {
                let err = DownloadError::Verify(format!(
                    "SHA256 不符：期望 {expected}，实际 {}",
                    fetched.sha256
                ));
                let _ = std::fs::remove_file(&partial);
                if round == 0 {
                    tracing::warn!(error = %err, "校验失败，删除 .partial 后重下一次");
                    continue;
                }
                return Err(err);
            }
        }

        // 校验通过：rename 为最终文件（先删除同名旧文件，Windows rename 不覆盖）。
        if dest.exists() {
            std::fs::remove_file(dest).map_err(|err| {
                DownloadError::File(format!("删除旧文件 {} 失败：{err}", dest.display()))
            })?;
        }
        std::fs::rename(&partial, dest).map_err(|err| {
            DownloadError::File(format!(
                "重命名 {} → {} 失败：{err}",
                partial.display(),
                dest.display()
            ))
        })?;
        return Ok(DownloadOutcome {
            bytes: fetched.bytes,
            sha256: fetched.sha256,
            url: fetched.url,
            hash_skipped,
        });
    }
    Err(DownloadError::Verify("校验重试次数耗尽".to_string()))
}

/// 单次下载的中间结果。
struct Fetched {
    bytes: u64,
    sha256: String,
    url: String,
}

/// 单个请求尝试的错误：区分「重试当前 URL」与「切换镜像/终止」。
enum AttemptError {
    Retryable(DownloadError),
    Fatal(DownloadError),
}

/// 按清单顺序尝试镜像，任一成功即返回；全部失败返回
/// [`DownloadError::MirrorsExhausted`]。取消立即透传。
async fn fetch_with_mirrors<F>(
    client: &reqwest::Client,
    urls: &[String],
    partial: &Path,
    expected_size: u64,
    cancel: &AtomicBool,
    retry: &RetryPolicy,
    on_progress: &mut F,
) -> Result<Fetched, DownloadError>
where
    F: FnMut(DownloadProgress) + ?Sized,
{
    let mut errors: Vec<String> = Vec::new();
    for url in urls {
        match fetch_one_url(
            client,
            url,
            partial,
            expected_size,
            cancel,
            retry,
            on_progress,
        )
        .await
        {
            Ok(fetched) => return Ok(fetched),
            Err(DownloadError::Cancelled) => return Err(DownloadError::Cancelled),
            Err(err) => {
                tracing::warn!(
                    url_host = %url_host(url),
                    error = %err,
                    "镜像下载失败，切换下一个镜像"
                );
                errors.push(format!("{}: {err}", url_host(url)));
            }
        }
    }
    Err(DownloadError::MirrorsExhausted(errors.join("; ")))
}

/// 对单个 URL 做「请求 + 重试」；返回成功结果或最终错误。
async fn fetch_one_url<F>(
    client: &reqwest::Client,
    url: &str,
    partial: &Path,
    expected_size: u64,
    cancel: &AtomicBool,
    retry: &RetryPolicy,
    on_progress: &mut F,
) -> Result<Fetched, DownloadError>
where
    F: FnMut(DownloadProgress) + ?Sized,
{
    let mut attempt: u32 = 0;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::Cancelled);
        }
        match attempt_fetch(client, url, partial, expected_size, cancel, on_progress).await {
            Ok(result) => {
                return Ok(Fetched {
                    bytes: result.bytes,
                    sha256: result.sha256,
                    url: url.to_string(),
                });
            }
            Err(AttemptError::Fatal(err)) => return Err(err),
            Err(AttemptError::Retryable(err)) => {
                if attempt >= retry.max_retries_per_url {
                    return Err(err);
                }
                let backoff = retry.backoff(attempt);
                tracing::debug!(
                    url_host = %url_host(url),
                    attempt,
                    backoff_ms = backoff.as_millis(),
                    error = %err,
                    "下载失败，指数退避后重试"
                );
                attempt += 1;
                if !sleep_cancellable(backoff, cancel).await {
                    return Err(DownloadError::Cancelled);
                }
            }
        }
    }
}

/// 单次 HTTP 尝试的结果。
struct FetchResult {
    bytes: u64,
    sha256: String,
}

/// 执行一次「发请求 → 写 `.partial` → 流式哈希」。
///
/// 续传：`.partial` 已存在则带 `Range` 请求；206 续写并校验 `Content-Range` 起点，
/// 200 视为服务端不支持 Range（丢弃断点从头写）。body 读错误 / 5xx 等返回
/// `Retryable`，交给上层退避重试；4xx 返回 `Fatal` 以切换镜像。
async fn attempt_fetch<F>(
    client: &reqwest::Client,
    url: &str,
    partial: &Path,
    expected_size: u64,
    cancel: &AtomicBool,
    on_progress: &mut F,
) -> Result<FetchResult, AttemptError>
where
    F: FnMut(DownloadProgress) + ?Sized,
{
    if cancel.load(Ordering::SeqCst) {
        return Err(AttemptError::Fatal(DownloadError::Cancelled));
    }

    let existing = match std::fs::metadata(partial) {
        Ok(meta) => meta.len(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
        Err(err) => {
            return Err(AttemptError::Fatal(DownloadError::File(format!(
                "读取 .partial 元数据失败：{err}"
            ))));
        }
    };

    let mut request = client
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if existing > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={existing}-"));
        tracing::debug!(
            url_host = %url_host(url),
            breakpoint = existing,
            "发现 .partial 断点，发送 Range 请求续传"
        );
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            return Err(AttemptError::Retryable(DownloadError::Http(format!(
                "请求发送失败：{err}"
            ))));
        }
    };

    let status = response.status();
    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let content_length = response.content_length();
    let mut response = response;

    // 416：断点已 >= 期望大小，可能上次已下完但未 rename —— 直接对现有文件算哈希。
    if status == StatusCode::RANGE_NOT_SATISFIABLE {
        if existing > 0 && (expected_size == 0 || existing >= expected_size) {
            let sha256 = match hash_existing(partial, existing, cancel).await {
                Ok(sha256) => sha256,
                Err(err) => return Err(AttemptError::Fatal(err)),
            };
            return Ok(FetchResult {
                bytes: existing,
                sha256,
            });
        }
        // 断点无效（例如清单大小变小）：丢弃后重下。
        let _ = std::fs::remove_file(partial);
        return Err(AttemptError::Retryable(DownloadError::Http(format!(
            "HTTP {status}，丢弃断点重下"
        ))));
    }

    let offset: u64;
    let total_hint: u64;
    if status == StatusCode::PARTIAL_CONTENT {
        let (start, total) = match content_range.as_deref().and_then(parse_content_range) {
            Some(parsed) => parsed,
            None => {
                return Err(AttemptError::Retryable(DownloadError::Http(
                    "206 响应缺少/无法解析 Content-Range".to_string(),
                )));
            }
        };
        if start != existing {
            let _ = std::fs::remove_file(partial);
            return Err(AttemptError::Retryable(DownloadError::Http(format!(
                "Content-Range 起点 {start} 与本地断点 {existing} 不一致，丢弃断点重下"
            ))));
        }
        offset = existing;
        total_hint = total
            .or_else(|| content_length.map(|cl| existing.saturating_add(cl)))
            .unwrap_or(expected_size);
    } else if status.is_success() {
        // 200：服务端忽略 Range（或首次请求），丢弃旧断点从头写。
        if existing > 0 {
            tracing::debug!(
                url_host = %url_host(url),
                breakpoint = existing,
                "服务端未支持 Range（200），丢弃断点从头下载"
            );
            let _ = std::fs::remove_file(partial);
        }
        offset = 0;
        total_hint = content_length.unwrap_or(expected_size);
    } else if status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
    {
        return Err(AttemptError::Retryable(DownloadError::Http(format!(
            "HTTP {status}"
        ))));
    } else {
        // 其余 4xx（404/403 等）不可重试：切换下一个镜像。
        return Err(AttemptError::Fatal(DownloadError::Http(format!(
            "HTTP {status}"
        ))));
    }

    let total = if total_hint > 0 {
        total_hint
    } else {
        expected_size
    };
    let mut file = if offset == 0 {
        tokio::fs::File::create(partial).await.map_err(|err| {
            AttemptError::Fatal(DownloadError::File(format!("创建 .partial 失败：{err}")))
        })?
    } else {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(partial)
            .await
            .map_err(|err| {
                AttemptError::Fatal(DownloadError::File(format!("打开 .partial 失败：{err}")))
            })?
    };

    let mut hasher = Sha256::new();
    if offset > 0 {
        // 续传：先把已有前缀流式喂给 hasher（进程重启后 hasher 状态丢失，必须重读前缀）。
        if let Err(err) = hash_prefix_into(partial, offset, &mut hasher, cancel).await {
            let _ = file.flush().await;
            return Err(AttemptError::Fatal(err));
        }
    }

    let mut loaded = offset;
    let mut samples: VecDeque<(Instant, u64)> = VecDeque::new();
    samples.push_back((Instant::now(), loaded));
    on_progress(DownloadProgress {
        loaded,
        total,
        speed_bps: 0,
    });

    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = file.flush().await;
            return Err(AttemptError::Fatal(DownloadError::Cancelled));
        }
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(err) => {
                let _ = file.flush().await;
                return Err(AttemptError::Retryable(DownloadError::Http(format!(
                    "读取响应体失败：{err}"
                ))));
            }
        };
        hasher.update(&chunk);
        if let Err(err) = file.write_all(&chunk).await {
            return Err(AttemptError::Fatal(DownloadError::File(format!(
                "写入 .partial 失败：{err}"
            ))));
        }
        loaded = loaded.saturating_add(chunk.len() as u64);

        // 滑动平均速度：窗口 SPEED_WINDOW。
        let now = Instant::now();
        while let Some(&(at, _)) = samples.front() {
            if now.duration_since(at) > SPEED_WINDOW {
                samples.pop_front();
            } else {
                break;
            }
        }
        samples.push_back((now, loaded));
        let (first_at, first_loaded) = *samples.front().unwrap_or(&(now, loaded));
        let elapsed = now.duration_since(first_at).as_secs_f64();
        let speed_bps = if elapsed > 0.0 {
            ((loaded.saturating_sub(first_loaded)) as f64 / elapsed).round() as u64
        } else {
            0
        };
        on_progress(DownloadProgress {
            loaded,
            total: if total > 0 { total } else { loaded },
            speed_bps,
        });
    }

    if let Err(err) = file.flush().await {
        return Err(AttemptError::Fatal(DownloadError::File(format!(
            "flush .partial 失败：{err}"
        ))));
    }
    drop(file);

    if total > 0 && loaded < total {
        return Err(AttemptError::Retryable(DownloadError::Http(format!(
            "响应体提前结束：收到 {loaded}/{total} 字节"
        ))));
    }

    let bytes = match std::fs::metadata(partial) {
        Ok(meta) => meta.len(),
        Err(err) => {
            return Err(AttemptError::Fatal(DownloadError::File(format!(
                "读取 .partial 大小失败：{err}"
            ))));
        }
    };
    Ok(FetchResult {
        bytes,
        sha256: hex::encode(hasher.finalize()),
    })
}

/// 流式计算文件前 `len` 字节的 SHA256（续传时重建 hasher 状态）。
async fn hash_prefix_into(
    path: &Path,
    len: u64,
    hasher: &mut Sha256,
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|err| DownloadError::File(format!("打开断点文件失败：{err}")))?;
    let mut buffer = vec![0u8; HASH_BLOCK_BYTES];
    let mut remaining = len;
    while remaining > 0 {
        if cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::Cancelled);
        }
        let want = remaining.min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..want])
            .await
            .map_err(|err| DownloadError::File(format!("读取断点文件失败：{err}")))?;
        if read == 0 {
            return Err(DownloadError::File(
                "断点文件比记录的断点短（可能被外部截断）".to_string(),
            ));
        }
        hasher.update(&buffer[..read]);
        remaining -= read as u64;
    }
    Ok(())
}

/// 流式计算整个文件（已知长度）的 SHA256。
async fn hash_existing(
    path: &Path,
    len: u64,
    cancel: &AtomicBool,
) -> Result<String, DownloadError> {
    let mut hasher = Sha256::new();
    hash_prefix_into(path, len, &mut hasher, cancel).await?;
    Ok(hex::encode(hasher.finalize()))
}

/// 解析 `Content-Range: bytes <start>-<end>/<total|*>`，返回 `(start, total)`。
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes")?.trim();
    let (range_part, total_part) = rest.split_once('/')?;
    let (start_part, _end_part) = range_part.split_once('-')?;
    let start = start_part.trim().parse::<u64>().ok()?;
    let total = if total_part.trim() == "*" {
        None
    } else {
        Some(total_part.trim().parse::<u64>().ok()?)
    };
    Some((start, total))
}

/// 可被取消打断的 sleep；返回 `false` 表示期间检测到取消。
async fn sleep_cancellable(total: Duration, cancel: &AtomicBool) -> bool {
    let deadline = Instant::now() + total;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        let remaining = deadline.saturating_duration_since(now);
        tokio::time::sleep(remaining.min(Duration::from_millis(50))).await;
    }
}

/// 构造真实下载用 HTTP 客户端（TODO(real): 接入 `advanced.proxy`）。
fn build_http_client() -> Result<reqwest::Client, DownloadError> {
    reqwest::Client::builder()
        .user_agent(concat!("snip-ask/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|err| DownloadError::Http(format!("创建 HTTP 客户端失败：{err}")))
}

/// 记录日志用的 host（契约 §15：不记录完整 URL 查询串）。
fn url_host(raw: &str) -> String {
    url::Url::parse(raw)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| "<invalid>".to_string())
}

fn model_missing_error(id: &str) -> AppError {
    AppError::new(
        ErrorCode::ModelMissing,
        Some(format!("模型 {id} 不在清单中")),
    )
    .with_fallback(vec!["download_model", "cloud_translate"])
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

/// 扫描 `<models_dir>/<id>/.installed`。
///
/// 注意：只认 `.installed` 标记，`.partial` / 未完成文件不计入已安装。
pub fn scan_installed(models_dir: &Path, manifest: &ModelManifest) -> HashMap<String, u64> {
    let mut installed = HashMap::new();
    for entry in &manifest.models {
        if models_dir.join(&entry.id).join(".installed").is_file() {
            installed.insert(entry.id.clone(), entry.size_bytes);
        }
    }
    installed
}

/// 安装目录示例（供 UI 展示 / 诊断）。
pub fn model_dir(models_dir: &Path, id: &str) -> PathBuf {
    models_dir.join(id)
}

/// 构造 `ModelInfo`（命令层 / 测试直接使用）。
pub fn info(entry: &ModelManifestEntry, installed_bytes: Option<u64>) -> ModelInfo {
    entry_to_info(entry, installed_bytes, DownloadState::Queued)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> ModelManager {
        let manifest = parse_manifest(crate::resources::MODELS_MANIFEST).unwrap();
        ModelManager {
            models_dir: std::env::temp_dir()
                .join(format!("snip-ask-models-{}", uuid::Uuid::new_v4())),
            manifest,
            states: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            installed: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            cancel_flags: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            skeleton_mode: Arc::new(AtomicBool::new(true)),
        }
    }

    #[test]
    fn lists_manifest_models_with_install_state() {
        let manager = manager();
        let infos = manager.list();
        assert!(!infos.is_empty());
        assert!(infos.iter().all(|info| !info.installed));
        assert_eq!(manager.installed_count(), 0);
        assert!(!manager.is_installed("hy-mt-1.8b-q4"));
        assert!(manager.entry("hy-mt-1.8b-q4").is_some());
        assert!(manager.entry("nope").is_none());
    }

    #[test]
    fn scan_installed_uses_marker_file() {
        let manifest = parse_manifest(crate::resources::MODELS_MANIFEST).unwrap();
        let root = std::env::temp_dir().join(format!("snip-ask-models-{}", uuid::Uuid::new_v4()));
        let entry_id = manifest.models[0].id.clone();
        std::fs::create_dir_all(root.join(&entry_id)).unwrap();
        std::fs::write(root.join(&entry_id).join(".installed"), b"").unwrap();
        let installed = scan_installed(&root, &manifest);
        assert_eq!(installed.len(), 1);
        assert!(installed.contains_key(&entry_id));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn partial_files_are_not_counted_as_installed() {
        let manifest = parse_manifest(crate::resources::MODELS_MANIFEST).unwrap();
        let root = std::env::temp_dir().join(format!("snip-ask-models-{}", uuid::Uuid::new_v4()));
        let entry_id = manifest.models[0].id.clone();
        let dir = root.join(&entry_id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.gguf.partial"), b"partial-bytes").unwrap();
        assert!(scan_installed(&root, &manifest).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn skeleton_mode_defaults_on_and_can_be_switched() {
        let manager = manager();
        assert!(manager.is_skeleton_mode());
        manager.set_skeleton_mode(false);
        assert!(!manager.is_skeleton_mode());
    }

    #[test]
    fn derives_download_filename_from_url() {
        let manifest = parse_manifest(crate::resources::MODELS_MANIFEST).unwrap();
        // 清单已按 ModelScope 官方文件校正（S-08 §6.1 / Lead 修正）：文件名大小写与官方一致。
        let entry = find_entry(&manifest, "hy-mt-1.8b-q4").unwrap();
        assert_eq!(download_filename(entry), "HY-MT1.5-1.8B-Q4_K_M.gguf");
        // opus-mt 官方仓库暂无 Ct2 产物，清单 urls 置空 → 文件名回退到 `<id>.bin`。
        let entry = find_entry(&manifest, "opus-mt-zh-en").unwrap();
        assert!(entry.urls.is_empty());
        assert_eq!(download_filename(entry), "opus-mt-zh-en.bin");
    }

    #[test]
    fn detects_placeholder_sha256() {
        assert!(is_placeholder_sha256(""));
        assert!(is_placeholder_sha256(
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));
        assert!(!is_placeholder_sha256(
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        ));
        assert!(is_placeholder_sha256("not-a-sha"));
    }

    #[test]
    fn partial_path_appends_suffix() {
        let dest = Path::new("models").join("hy").join("model.gguf");
        assert_eq!(
            partial_path(&dest),
            Path::new("models").join("hy").join("model.gguf.partial")
        );
    }

    #[test]
    fn parses_content_range() {
        assert_eq!(
            parse_content_range("bytes 0-1023/1258291200"),
            Some((0, Some(1_258_291_200)))
        );
        assert_eq!(
            parse_content_range("bytes 1048576-2097151/*"),
            Some((1_048_576, None))
        );
        assert_eq!(parse_content_range("not-bytes"), None);
    }

    #[test]
    fn remove_rejects_unknown_model() {
        let manager = manager();
        assert_eq!(manager.remove("unknown").unwrap_err().code, "SA-4001");
    }

    #[test]
    fn remove_deletes_installed_dir() {
        let manager = manager();
        let id = manager.manifest.models[0].id.clone();
        let dir = manager.models_dir().join(&id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".installed"), b"").unwrap();
        manager.refresh_installed();
        assert!(manager.is_installed(&id));
        manager.remove(&id).unwrap();
        assert!(!manager.is_installed(&id));
        assert!(!dir.exists());
    }
}
