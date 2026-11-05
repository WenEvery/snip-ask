//! 系统命令（契约 §6 第 23–26 条 + 契约 v5 第 39–40 条）。
//!
//! v1.0.0 追加：
//! - `open_path`：打开白名单目录（自定义指南 / 提示词 / 配置 / 模型 / 日志 / 后端 / 安装文档）；
//! - `ocr_status`：OCR（ONNX Runtime + det/rec/cls/字典）就绪状态，供设置页与引导页使用。

use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::app::state::AppState;
use crate::contract::errors::{AppError, AppResult};
use crate::contract::events::ConfigChangedPayload;
use crate::contract::types::{HotkeyStatus, OcrStatus, Stats};
use crate::events::emit_config_changed;

/// 命令：探测快捷键（格式 / 规范化 / 配置内冲突 / OS 级占用 `system`）。
///
/// OS 级探测复用 `tauri-plugin-global-shortcut` 的 `register` / `unregister`
/// （插件在 OS 注册失败时返回 `Err`），临时注册成功后立即注销，避免探测本身长期占用热键。
#[tauri::command]
pub async fn hotkey_probe(
    app: AppHandle,
    state: State<'_, AppState>,
    spec: String,
) -> AppResult<HotkeyStatus> {
    let config = state.snapshot_config();
    Ok(crate::app::hotkeys::probe_os(&app, &config, &spec))
}

/// 命令：设置开机自启（同时写入 `app.autostart` 并广播 `config:changed`）。
#[tauri::command]
pub async fn set_autostart(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<()> {
    let skeleton_mode = state.is_skeleton();
    // 先改系统自启（失败返回 SA-5004，不修改配置）。
    crate::platform::autostart::set_enabled(&app, enabled, skeleton_mode)?;

    let old = state.snapshot_config();
    let mut candidate = old.clone();
    let (view, changed) = state
        .config_center
        .patch(
            &mut candidate,
            serde_json::json!({ "app": { "autostart": enabled } }),
            state.credentials.as_ref(),
        )
        .await?;
    *state.config.write() = candidate;
    emit_config_changed(
        &app,
        &ConfigChangedPayload {
            config: view,
            changed_keys: changed,
        },
    );
    Ok(())
}

/// 命令：打开外部链接（仅 https 或 http://127.0.0.1；骨架模式只记日志）。
#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> AppResult<()> {
    crate::services::search_gateway::open_external(Some(&app), &url).await
}

/// 计算 OCR 资产状态（契约 v5）。
///
/// 委托 `services::ocr_onnx`（与真实引擎同一套资源解析），保证「下载完模型 → 状态立即变就绪」。
pub fn ocr_status_snapshot(app: &AppHandle, state: &AppState) -> OcrStatus {
    let resource_dir = app.path().resource_dir().ok();
    crate::services::ocr_onnx::ocr_status(resource_dir.as_deref(), &state.paths.models_dir)
}

/// 命令：OCR 状态（契约 v5 第 40 条）。
#[tauri::command]
pub async fn ocr_status(app: AppHandle, state: State<'_, AppState>) -> AppResult<OcrStatus> {
    Ok(ocr_status_snapshot(&app, &state))
}

/// 命令：打开白名单目录（契约 v5 第 39 条）。
///
/// `kind` ∈ `config | prompts | models | logs | backends | data | docs`；
/// 数据目录不存在时自动创建；`docs` 由安装包随附，缺失时返回可操作错误。
#[tauri::command]
pub async fn open_path(app: AppHandle, state: State<'_, AppState>, kind: String) -> AppResult<()> {
    let paths = state.paths.clone();
    let target = match kind.as_str() {
        // 「data」与「config」当前都指向应用数据根目录（配置/历史/日志同级）。
        "config" | "data" => paths.data_dir.clone(),
        "prompts" => paths.prompts_dir.clone(),
        "models" => paths.models_dir.clone(),
        "logs" => paths.logs_dir.clone(),
        "backends" => state.backends.backends_root(),
        "docs" => app
            .path()
            .resource_dir()
            .map_err(|err| AppError::io_failed(format!("无法解析安装目录：{err}")))?
            .join("docs"),
        other => {
            return Err(AppError::invalid_config(
                "kind",
                format!("不支持的目录类型：{other}"),
            ))
        }
    };

    if kind == "docs" {
        if !target.is_dir() {
            return Err(AppError::io_failed(format!(
                "文档目录不存在：{}（开发模式请直接查看仓库 docs/）",
                target.display()
            )));
        }
    } else {
        std::fs::create_dir_all(&target).map_err(|err| {
            AppError::io_failed(format!("创建目录 {} 失败：{err}", target.display()))
        })?;
    }

    app.opener()
        .open_path(target.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|err| AppError::io_failed(format!("打开目录失败：{err}")))
}

/// 命令：运行诊断信息。
#[tauri::command]
pub async fn app_stats(app: AppHandle, state: State<'_, AppState>) -> AppResult<Stats> {
    let ocr = ocr_status_snapshot(&app, &state);
    Ok(crate::services::diagnostics::collect(
        state.active_session_id(),
        state.active_task_count() as u32,
        state.engine_status(),
        state.models.installed_count(),
        state.is_skeleton(),
        state.uptime_sec(),
        ocr.ready,
    ))
}

/// 命令：退出应用（保存配置后退出）。
#[tauri::command]
pub async fn app_quit(app: AppHandle) -> AppResult<()> {
    if let Err(err) = crate::app::lifecycle::shutdown(&app) {
        tracing::warn!(code = %err.code, "退出前清理失败");
    }
    app.exit(0);
    Ok(())
}
