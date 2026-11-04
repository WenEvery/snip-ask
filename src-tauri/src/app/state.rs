//! 应用状态容器（M01 / 实施架构 §5.3）。
//!
//! `AppState` 由 Tauri 在 `setup` 中 `manage`，命令通过 `State<'_, AppState>` 访问。
//! 所有需要在异步命令中使用的字段都必须 `Send + Sync`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, RwLock};

use crate::contract::config::{Config, ConfigView};
use crate::contract::errors::AppResult;
use crate::contract::types::{EngineStatus, OnboardingState, SessionId};
use crate::domain::session::{SessionPhase, SessionStore};
use crate::domain::task::TaskStore;
use crate::platform::credential::{CredentialStore, InMemoryCredentialStore};
use crate::platform::paths::AppPaths;
use crate::services::config_center::ConfigCenter;
use crate::services::credential_service::CredentialService;
use crate::services::history_repo::HistoryRepo;
use crate::services::model_manager::ModelManager;
use crate::services::ocr_service::OcrPipeline;
use crate::services::onboarding_service::OnboardingService;
use crate::services::prompt_engine::PromptEngine;

/// 应用状态。
pub struct AppState {
    /// 当前生效配置（热生效由命令层写回）。
    pub config: RwLock<Config>,
    /// 全部数据路径。
    pub paths: AppPaths,
    /// 会话状态机。
    pub session: Mutex<SessionStore>,
    /// 任务表（取消令牌）。
    pub tasks: Mutex<TaskStore>,
    /// 配置中心（加载 / 原子写 / 迁移）。
    pub config_center: ConfigCenter,
    /// 凭据存储（骨架模式为内存实现）。
    pub credentials: Arc<dyn CredentialStore>,
    /// 历史与用量仓储。
    pub history: HistoryRepo,
    /// OCR 流水线（S-02/S-03：整屏 det 缓存 → 选区 rec-only，按 `ocr_session_id` 关联会话）。
    pub ocr: OcrPipeline,
    /// 本地模型管理。
    pub models: ModelManager,
    /// 首启引导状态。
    pub onboarding: Mutex<OnboardingState>,
    /// 本地推理进程状态。
    pub engine: RwLock<EngineStatus>,
    /// 进程启动时间。
    pub started_at: Instant,
    /// 热键是否暂停。
    pub paused: AtomicBool,
}

impl AppState {
    /// 初始化应用状态：加载配置 / 引导 / 历史 / 凭据 / 模型清单。
    pub fn initialize(paths: AppPaths) -> AppResult<Self> {
        paths.ensure()?;

        let config_center = ConfigCenter::new(paths.clone());
        let loaded = config_center.load();
        for note in &loaded.notes {
            tracing::warn!(note = %note, "配置加载提示");
        }
        if loaded.recovered {
            tracing::warn!("配置已恢复为默认值");
        }

        let onboarding = OnboardingService::new(&paths).load();
        let credentials: Arc<dyn CredentialStore> = Arc::new(InMemoryCredentialStore::new());
        let history = HistoryRepo::open(&paths, loaded.config.history.enabled);
        let models = ModelManager::load(&paths)?;
        // S-08：把配置里的骨架模式注入模型管理器（默认 true = 保持原骨架下载行为；
        // skeletonMode=false 时才走真实 HTTP Range 下载 + 校验）。
        let skeleton_mode = loaded.config.advanced.skeleton_mode;
        models.set_skeleton_mode(skeleton_mode);

        Ok(Self {
            config: RwLock::new(loaded.config),
            paths,
            session: Mutex::new(SessionStore::new()),
            tasks: Mutex::new(TaskStore::new()),
            config_center,
            credentials,
            history,
            // 骨架模式：占位 OCR 引擎（真实 RapidOCR 引擎见 ocr_service 的 TODO(real)）。
            ocr: OcrPipeline::with_skeleton(),
            models,
            onboarding: Mutex::new(onboarding),
            engine: RwLock::new(EngineStatus::Stopped),
            started_at: Instant::now(),
            paused: AtomicBool::new(false),
        })
    }

    /// 是否骨架模式（契约 §12，默认 true）。
    pub fn is_skeleton(&self) -> bool {
        self.config.read().advanced.skeleton_mode
    }

    /// 配置快照（读锁短持有，避免跨 await 持锁）。
    pub fn snapshot_config(&self) -> Config {
        self.config.read().clone()
    }

    /// 生成 `ConfigView`（providers 带 hasKey / keyMask）。
    pub async fn config_view(&self) -> ConfigView {
        let config = self.snapshot_config();
        self.config_center
            .snapshot(&config, self.credentials.as_ref())
            .await
    }

    /// 当前活动会话 ID。
    pub fn active_session_id(&self) -> Option<SessionId> {
        self.session.lock().current_id()
    }

    /// 当前会话阶段。
    pub fn session_phase(&self) -> SessionPhase {
        self.session.lock().current_phase()
    }

    /// 活动任务数。
    pub fn active_task_count(&self) -> usize {
        self.tasks.lock().active_count()
    }

    /// 运行时长（秒）。
    pub fn uptime_sec(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// 本地引擎状态。
    pub fn engine_status(&self) -> EngineStatus {
        *self.engine.read()
    }

    /// 更新本地引擎状态并广播 `engine:status` 由调用方负责。
    pub fn set_engine_status(&self, status: EngineStatus) {
        *self.engine.write() = status;
    }

    /// 热键是否暂停。
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// 设置热键暂停标记。
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }

    /// 凭据服务（复用同一存储）。
    pub fn credential_service(&self) -> CredentialService {
        CredentialService::new(self.credentials.clone())
    }

    /// 提示词引擎。
    pub fn prompt_engine(&self) -> PromptEngine {
        PromptEngine::new(&self.paths)
    }

    /// 引导服务。
    pub fn onboarding_service(&self) -> OnboardingService {
        OnboardingService::new(&self.paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths() -> AppPaths {
        let root = std::env::temp_dir().join(format!("snip-ask-state-{}", uuid::Uuid::new_v4()));
        AppPaths::from_roots(
            true,
            root,
            std::path::Path::new("C:/tmp/config"),
            std::path::Path::new("C:/tmp/local"),
        )
        .unwrap()
    }

    #[test]
    fn initialize_loads_defaults() {
        let paths = temp_paths();
        let state = AppState::initialize(paths.clone()).unwrap();
        assert!(state.is_skeleton());
        // S-08：骨架模式开关必须同步注入模型管理器（默认 true）。
        assert_eq!(state.is_skeleton(), state.models.is_skeleton_mode());
        assert_eq!(state.session_phase(), SessionPhase::Idle);
        assert_eq!(state.active_session_id(), None);
        assert_eq!(state.active_task_count(), 0);
        assert_eq!(state.engine_status(), EngineStatus::Stopped);
        assert!(!state.is_paused());
        assert_eq!(state.snapshot_config().schema_version, 3);
        assert!(!state.onboarding.lock().completed);
        assert!(paths.config_file.is_file());
        assert!(state.history.is_enabled());

        let _ = std::fs::remove_dir_all(&paths.data_dir);
    }

    #[tokio::test]
    async fn config_view_reports_no_keys_initially() {
        let paths = temp_paths();
        let state = AppState::initialize(paths.clone()).unwrap();
        let view = state.config_view().await;
        assert_eq!(view.providers.len(), 1);
        assert!(!view.providers[0].has_key);
        assert!(view.providers[0].key_mask.is_none());
        let _ = std::fs::remove_dir_all(&paths.data_dir);
    }
}
