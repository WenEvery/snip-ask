//! 本地模型管理（M12 / 契约 §12）。
//!
//! - 清单来自内嵌 `resources/models.manifest.json`；
//! - 安装状态扫描 `<models_dir>/<id>/.installed`；
//! - 骨架下载：每 100ms 推进 4%，到 100% 后 `verifying → installed`，写入空 `.installed` 标记；
//!   清单中不存在的 id 直接 `download.state(failed)`。
//!
//! TODO(real): HTTP Range 断点续传 + HuggingFace/ModelScope 镜像回退 + SHA256 校验。
//! TODO(real): 真实实现不应在 service 内持有 `AppHandle`；当前为骨架期简化。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::{
    DownloadProgressPayload, DownloadStatePayload, EVENT_DOWNLOAD_PROGRESS, EVENT_DOWNLOAD_STATE,
};
use crate::contract::types::{
    DownloadId, DownloadState, ModelInfo, ModelManifest, ModelManifestEntry,
};
use crate::domain::ids::new_download_id;
use crate::domain::models::{entry_to_info, find_entry, installed_count, merge_installed, parse_manifest};
use crate::platform::paths::AppPaths;

/// 下载进度推进间隔（契约 §12：每 100ms 4%）。
const DOWNLOAD_STEP_MS: u64 = 100;
/// 进度事件节流（契约 §7：≤ 200ms）。
const PROGRESS_THROTTLE_MS: u64 = 200;

/// 模型管理器（克隆后可在异步任务中使用）。
#[derive(Clone)]
pub struct ModelManager {
    models_dir: PathBuf,
    manifest: ModelManifest,
    states: Arc<parking_lot::Mutex<HashMap<String, DownloadState>>>,
    installed: Arc<parking_lot::Mutex<HashMap<String, u64>>>,
    cancel_flags: Arc<parking_lot::Mutex<HashMap<String, Arc<AtomicBool>>>>,
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

    /// 当前模型列表（清单静态字段 + 安装状态 + 下载状态）。
    pub fn list(&self) -> Vec<ModelInfo> {
        let installed = self.installed.lock().clone();
        let states = self.states.lock().clone();
        merge_installed(&self.manifest, &installed, &states)
    }

    /// 已安装模型数。
    pub fn installed_count(&self) -> u32 {
        let installed = self.installed.lock();
        installed_count(&merge_installed(&self.manifest, &installed, &HashMap::new()))
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

    /// 开始下载（骨架模式：占位进度）；返回下载 ID。
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
        let total = entry.size_bytes.max(1);
        let _ = tauri::async_runtime::spawn(async move {
            manager.run_download(&app, &model_id, total, cancel).await;
        });
        Ok(download_id)
    }

    async fn run_download(
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
            if last_emit.elapsed() >= Duration::from_millis(PROGRESS_THROTTLE_MS)
                || percent == 100
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

    /// 取消下载；返回是否命中。
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
            std::fs::remove_dir_all(&dir)
                .map_err(|err| AppError::io_failed(format!("删除模型 {} 失败：{err}", dir.display())))?;
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
            models_dir: std::env::temp_dir().join(format!("snip-ask-models-{}", uuid::Uuid::new_v4())),
            manifest,
            states: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            installed: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            cancel_flags: Arc::new(parking_lot::Mutex::new(HashMap::new())),
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
