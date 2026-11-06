//! 应用状态容器（M01 / 实施架构 §5.3）。
//!
//! `AppState` 由 Tauri 在 `setup` 中 `manage`，命令通过 `State<'_, AppState>` 访问。
//! 所有需要在异步命令中使用的字段都必须 `Send + Sync`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, RwLock};

use crate::contract::config::{Config, ConfigView};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{ActionId, EngineStatus, OnboardingState, SessionId};
use crate::domain::session::{SessionPhase, SessionStore};
use crate::domain::task::TaskStore;
use crate::domain::turn::TurnStore;
use crate::platform::capture::FrozenFrame;
use crate::platform::credential::{default_store, CredentialStore};
use crate::platform::gpu::GpuProbeCache;
use crate::platform::paths::AppPaths;
use crate::services::action_router::RoutedAction;
use crate::services::backend_manager::BackendManager;
use crate::services::config_center::ConfigCenter;
use crate::services::credential_service::CredentialService;
use crate::services::history_repo::HistoryRepo;
use crate::services::llm_gateway::{LlmGateway, SkeletonLlmGateway};
use crate::services::local_engine::LocalEngine;
use crate::services::local_gateway::HttpLlmGateway;
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
    /// 本会话已完成的问答轮次（追问上下文；契约 v4）。
    pub turns: Mutex<TurnStore>,
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
    /// 后端组件管理（Vulkan 默认 GPU 组件 / CUDA 可选 / CPU；契约 v4）。
    pub backends: BackendManager,
    /// GPU 探测缓存（60s）。
    pub gpu_probe: Arc<GpuProbeCache>,
    /// 本地推理引擎监督者（Vulkan → CUDA → CPU 回退链）。
    pub local_engine: Arc<LocalEngine>,
    /// 首启引导状态。
    pub onboarding: Mutex<OnboardingState>,
    /// 本地推理进程状态。
    pub engine: RwLock<EngineStatus>,
    /// 进程启动时间。
    pub started_at: Instant,
    /// 热键是否暂停。
    pub paused: AtomicBool,
    /// 全局热键触发的「捕获后自动执行动作」（图片搜索 / 解答选区）：`selection_commit` 消费一次。
    ///
    /// 只用于热键语义，不进入配置/契约；用户取消框选或捕获失败时必须清空。
    pub pending_action: RwLock<Option<ActionId>>,
    /// 当前冻结帧（真实捕获含 BGRA 像素与 PNG data URL；T2/T3 共用）。
    pub frozen: RwLock<Option<FrozenFrame>>,
    /// 本地引擎未启动的可读原因（用于设置页展示；启动成功后清空）。
    pub engine_note: RwLock<Option<String>>,
}

impl AppState {
    /// 初始化应用状态（真实 Windows Credential Manager 凭据存储）。
    pub fn initialize(paths: AppPaths) -> AppResult<Self> {
        Self::initialize_with(paths, default_store())
    }

    /// 以给定凭据存储初始化（单测 / 无 Credential Manager 时的降级路径）。
    pub fn initialize_with(
        paths: AppPaths,
        credentials: Arc<dyn CredentialStore>,
    ) -> AppResult<Self> {
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
        let history = HistoryRepo::open(&paths, loaded.config.history.enabled);
        // 启动时按历史保留策略清理一次（shutdown 也会清理）。
        history.cleanup(
            loaded.config.history.max_entries,
            loaded.config.history.retention_days,
        );
        let models = ModelManager::load(&paths)?;
        // S-08：把配置里的骨架模式注入模型管理器（默认 true = 保持原骨架下载行为；
        // skeletonMode=false 时才走真实 HTTP Range 下载 + 校验）。
        let skeleton_mode = loaded.config.advanced.skeleton_mode;
        models.set_skeleton_mode(skeleton_mode);

        Ok(Self {
            config: RwLock::new(loaded.config),
            paths: paths.clone(),
            session: Mutex::new(SessionStore::new()),
            tasks: Mutex::new(TaskStore::new()),
            turns: Mutex::new(TurnStore::new()),
            config_center,
            credentials,
            history,
            // 非骨架模式：真实 RapidOCR ONNX 引擎（懒加载；缺模型不阻塞启动，首次 OCR 报可操作错误）。
            ocr: if skeleton_mode {
                OcrPipeline::with_skeleton()
            } else {
                let assets = crate::services::ocr_onnx::OcrAssets::resolve(&paths.models_dir);
                OcrPipeline::with_real(Box::new(crate::services::ocr_onnx::OnnxOcrEngine::new(
                    assets,
                )))
            },
            models,
            backends: BackendManager::new(paths.clone()),
            gpu_probe: Arc::new(GpuProbeCache::new()),
            local_engine: Arc::new(LocalEngine::new(paths)),
            onboarding: Mutex::new(onboarding),
            engine: RwLock::new(EngineStatus::Stopped),
            started_at: Instant::now(),
            paused: AtomicBool::new(false),
            pending_action: RwLock::new(None),
            frozen: RwLock::new(None),
            engine_note: RwLock::new(None),
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

    /// 设置「框选完成后自动执行的动作」（热键语义；`None` 清除）。
    pub fn set_pending_action(&self, action: Option<ActionId>) {
        *self.pending_action.write() = action;
    }

    /// 取出并清除待执行动作（只消费一次，避免影响下一次普通框选）。
    pub fn take_pending_action(&self) -> Option<ActionId> {
        self.pending_action.write().take()
    }

    /// 记录本地引擎未启动的可读原因（设置页展示用）。
    pub fn set_engine_note(&self, note: Option<String>) {
        *self.engine_note.write() = note;
    }

    /// 读取本地引擎未启动原因。
    pub fn engine_note(&self) -> Option<String> {
        self.engine_note.read().clone()
    }

    /// 保存冻结帧（真实捕获帧；overlay / OCR / 图片搜索共用）。
    pub fn set_frozen_frame(&self, frame: FrozenFrame) {
        *self.frozen.write() = Some(frame);
    }

    /// 读取当前冻结帧（克隆；BGRA 用 Arc 共享，克隆成本低）。
    pub fn frozen_frame(&self) -> Option<FrozenFrame> {
        self.frozen.read().clone()
    }

    /// 清空冻结帧（关闭会话 / 取消捕获时调用）。
    pub fn clear_frozen_frame(&self) {
        *self.frozen.write() = None;
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

    /// 选择流式网关（契约 v4 / 研究 §4.1）。
    ///
    /// 优先级：
    /// 1. 骨架模式（`advanced.skeletonMode = true`，默认）→ `SkeletonLlmGateway`（契约 §12 行为不变）；
    /// 2. `routed.engine == "local"` → 确保本地引擎就绪（**Vulkan → CUDA → CPU** 回退链）后，
    ///    走本地 OpenAI 兼容端点（无需 API Key，真实推理）；
    /// 3. 云端供应商 → 已配置密钥则走真实 SSE 网关，否则回退骨架网关并告警（不伪造成功）。
    pub async fn gateway_for(
        &self,
        app: &tauri::AppHandle,
        routed: &RoutedAction,
    ) -> AppResult<Arc<dyn LlmGateway>> {
        if self.is_skeleton() {
            return Ok(Arc::new(SkeletonLlmGateway));
        }

        let config = self.snapshot_config();

        if routed.engine == "local" {
            // 本地翻译/解答：启动或复用本地引擎（GPU 优先，失败自动回退 CPU）。
            self.local_engine.ensure_ready(app, &config).await?;
            let base_url = self.local_engine.base_url().ok_or_else(|| {
                AppError::new(
                    ErrorCode::ModelMissing,
                    Some("本地引擎已就绪但缺少端点地址".to_string()),
                )
            })?;
            return Ok(Arc::new(HttpLlmGateway::new(
                base_url,
                None,
                config.local.model.clone(),
            )));
        }

        if let Some(provider) = config
            .providers
            .iter()
            .find(|provider| provider.enabled && provider.id == routed.engine)
        {
            let key = self.credential_service().provider_key(&provider.id).await?;
            if key.is_some() {
                let base_url = format!("{}/v1", provider.base_url.trim_end_matches('/'));
                return Ok(Arc::new(HttpLlmGateway::new(
                    base_url,
                    key,
                    provider.model.clone(),
                )));
            }
            tracing::warn!(
                provider = %provider.id,
                "供应商未配置密钥，回退骨架网关（不发起真实请求）"
            );
        }

        Ok(Arc::new(SkeletonLlmGateway))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::credential::InMemoryCredentialStore;

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

    fn test_state(paths: AppPaths) -> AppState {
        AppState::initialize_with(paths, Arc::new(InMemoryCredentialStore::new())).unwrap()
    }

    #[test]
    fn initialize_loads_defaults() {
        let paths = temp_paths();
        let state = test_state(paths.clone());
        // v5：默认走真实能力（skeletonMode = false），骨架模式仅作调试开关。
        assert!(!state.is_skeleton());
        // S-08：骨架模式开关必须同步注入模型管理器。
        assert_eq!(state.is_skeleton(), state.models.is_skeleton_mode());
        assert_eq!(state.session_phase(), SessionPhase::Idle);
        assert_eq!(state.active_session_id(), None);
        assert_eq!(state.active_task_count(), 0);
        assert_eq!(state.engine_status(), EngineStatus::Stopped);
        assert!(!state.is_paused());
        assert_eq!(state.snapshot_config().schema_version, 7);
        assert!(!state.onboarding.lock().completed);
        assert!(paths.config_file.is_file());
        assert!(state.history.is_enabled());

        let _ = std::fs::remove_dir_all(&paths.data_dir);
    }

    #[test]
    fn pending_action_is_consumed_once() {
        let paths = temp_paths();
        let state = test_state(paths.clone());
        assert_eq!(state.take_pending_action(), None);
        state.set_pending_action(Some(ActionId::ImageSearch));
        assert_eq!(state.take_pending_action(), Some(ActionId::ImageSearch));
        // 只消费一次：下一次普通框选不会被上一次热键影响。
        assert_eq!(state.take_pending_action(), None);
        state.set_pending_action(Some(ActionId::Answer));
        state.set_pending_action(None);
        assert_eq!(state.take_pending_action(), None);
        let _ = std::fs::remove_dir_all(&paths.data_dir);
    }

    #[tokio::test]
    async fn config_view_reports_no_keys_initially() {
        let paths = temp_paths();
        let state = test_state(paths.clone());
        let view = state.config_view().await;
        assert_eq!(view.providers.len(), 3);
        assert!(!view.providers[0].has_key);
        assert!(view.providers[0].key_mask.is_none());
        let _ = std::fs::remove_dir_all(&paths.data_dir);
    }
}
