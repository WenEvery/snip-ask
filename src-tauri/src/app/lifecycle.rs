//! 应用生命周期（M01 / 概要设计 §4.1）。
//!
//! 启动顺序（契约对齐）：
//! 1. 解析数据路径（便携版 / 安装版）；
//! 2. 初始化日志（按日滚动，脱敏）；
//! 3. 加载配置（失败回退默认 + 备份损坏文件）；
//! 4. 初始化 `AppState` 并 `manage`；
//! 5. 创建托盘；
//! 6. 注册全局热键；
//! 7. 预创建隐藏窗口（result；overlay 惰性）；
//! 8. 按需启动本地引擎（骨架模式不启动）；
//! 9. 后台更新检查（不阻塞启动）。

use once_cell::sync::OnceCell;
use tauri::{AppHandle, Manager};

use crate::app::state::AppState;
use crate::app::{hotkeys, tray, windows};
use crate::contract::config::TranslateBackend;
use crate::contract::errors::{AppError, AppResult};
use crate::contract::types::EngineStatus;
use crate::events::emit_engine_status;
use crate::platform::paths::AppPaths;
use crate::services::updater_service;
use tracing_subscriber::EnvFilter;

/// 日志 WorkerGuard（进程生命周期内保持存活，保证文件日志落盘）。
static LOG_GUARD: OnceCell<tracing_appender::non_blocking::WorkerGuard> = OnceCell::new();

/// 应用启动流程（由 `lib.rs` 的 `setup` 调用）。
pub fn setup(app: &mut AppHandle) -> AppResult<()> {
    // 1) 路径
    let paths = AppPaths::resolve()?;

    // 2) 日志（必须早于配置加载失败分支）
    init_logging(&paths);
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        portable = paths.portable,
        data_dir = %paths.data_dir.display(),
        "框选 AI 启动中"
    );

    // 3) 状态（内部加载配置 / 引导 / 历史 / 凭据 / 模型清单）
    let state = AppState::initialize(paths)?;
    let first_run = !state.onboarding.lock().completed;
    let skeleton_mode = state.is_skeleton();
    if !app.manage(state) {
        tracing::warn!("AppState 已被托管，跳过重复初始化");
    }

    // 4) 托盘
    if let Err(err) = tray::create(app) {
        tracing::warn!(code = %err.code, "托盘创建失败，应用将继续以无托盘模式运行");
    }

    // 5) 全局热键
    if let Err(err) = hotkeys::register_all(app) {
        tracing::warn!(code = %err.code, "全局热键注册失败");
    }

    // 6) 隐藏窗口（结果窗预创建；overlay 惰性创建）
    if let Err(err) = windows::prepare_hidden_windows(app) {
        tracing::warn!(code = %err.code, "结果窗预创建失败（首次使用时将重试）");
    }

    // 7) 本地引擎状态（骨架模式：stopped，不启动进程）
    emit_engine_status(app, EngineStatus::Stopped);
    if !skeleton_mode {
        // 非骨架模式：按 local.* 配置**异步**预热本地引擎（不阻塞启动）。
        // 只有确实会用本地推理时才预热（翻译后端 = 本地 / 仅本地模式），避免无谓占用显存。
        let engine_app = app.clone();
        let _handle = tauri::async_runtime::spawn(async move {
            let Some(state) = engine_app.try_state::<AppState>() else {
                return;
            };
            let config = state.snapshot_config();
            let prewarm =
                config.translate.backend == TranslateBackend::Local || config.privacy.local_only;
            if !prewarm {
                tracing::info!("本地引擎预热跳过（未配置本地翻译 / 仅本地模式）");
                return;
            }
            let engine = state.local_engine.clone();
            state.set_engine_status(EngineStatus::Starting);
            emit_engine_status(&engine_app, EngineStatus::Starting);
            match engine.ensure_ready(&engine_app, &config).await {
                Ok(payload) => {
                    *state.engine.write() = payload.local;
                    crate::events::emit_engine_status_payload(&engine_app, &payload);
                    tracing::info!(
                        backend = ?payload.backend,
                        device = ?payload.device,
                        device_name = ?payload.device_name,
                        offload = ?payload.offload_layers,
                        port = ?payload.port,
                        fallback = ?payload.fallback_reason,
                        "本地引擎预热完成"
                    );
                }
                Err(err) => {
                    // 不阻塞启动：任务执行时 `gateway_for` → `ensure_ready` 会重试并返回真实错误。
                    tracing::warn!(
                        code = %err.code,
                        detail = ?err.detail,
                        "本地引擎预热失败（任务执行时会重试）"
                    );
                    *state.engine.write() = EngineStatus::Failed;
                    emit_engine_status(&engine_app, EngineStatus::Failed);
                }
            }
        });
    }

    // 8) 首启引导：未完成时显示主窗口
    if first_run {
        if let Err(err) = windows::show_main(app) {
            tracing::warn!(code = %err.code, "显示首启引导窗口失败");
        }
    }

    // 9) 后台更新检查（不阻塞启动）
    let update_app = app.clone();
    let _handle = tauri::async_runtime::spawn(async move {
        updater_service::check_once(&update_app).await;
    });

    tracing::info!("框选 AI 初始化完成（skeletonMode = {skeleton_mode}）");
    Ok(())
}

/// 应用退出前清理：取消任务、关闭会话、保存配置、裁剪历史。
pub fn shutdown(app: &AppHandle) -> AppResult<()> {
    let Some(state) = app.try_state::<AppState>() else {
        return Ok(());
    };

    // 取消全部任务（不再推送事件）。
    {
        let mut tasks = state.tasks.lock();
        let ids = tasks.ids();
        for task in ids {
            tasks.cancel(&task);
        }
    }

    // 关闭会话（释放选区 / 冻结帧）。
    {
        let mut session = state.session.lock();
        if let Some(id) = session.current_id() {
            let _ = session.close(&id);
        }
    }

    // 保存配置（原子写）。
    let config = state.snapshot_config();
    if let Err(err) = state.config_center.save_atomic(&config) {
        tracing::warn!(code = %err.code, "退出前保存配置失败");
    }

    // 历史裁剪。
    state
        .history
        .cleanup(config.history.max_entries, config.history.retention_days);

    // 停止本地推理进程并回收 sidecar（按 PID/进程树，禁止按镜像名 taskkill）。
    // 必须在 `AppState` 仍可访问时执行（随后进程退出）。
    if let Err(err) = state.local_engine.stop_blocking() {
        tracing::warn!(code = %err.code, "退出前停止本地引擎失败");
    }
    *state.engine.write() = EngineStatus::Stopped;
    emit_engine_status(app, EngineStatus::Stopped);

    tracing::info!("框选 AI 已退出");
    Ok(())
}

/// 初始化 tracing（按日滚动文件 + 环境变量级别）。
pub fn init_logging(paths: &AppPaths) {
    if let Err(err) = std::fs::create_dir_all(&paths.logs_dir) {
        eprintln!("创建日志目录失败：{err}");
        return;
    }
    let file_appender = tracing_appender::rolling::daily(&paths.logs_dir, "snipask.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let _ = LOG_GUARD.set(guard);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .try_init();
}

/// 初始化失败时构造可转换为 `Box<dyn Error>` 的错误（`lib.rs` 使用）。
pub fn setup_error(err: AppError) -> Box<dyn std::error::Error> {
    Box::new(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_error_is_boxable() {
        let error = AppError::from_code(crate::contract::errors::ErrorCode::CaptureFailed);
        let boxed: Box<dyn std::error::Error> = setup_error(error);
        assert!(boxed.to_string().contains("SA-1001"));
    }
}
