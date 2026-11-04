//! 框选 AI / Snip Ask —— Tauri 2 + Rust 应用入口（M01）。
//!
//! 分层（单向依赖）：
//! `commands → app / events → services → domain`；`services` 依赖 `platform` trait；
//! `contract` 为最底层契约（不依赖任何内部模块）；`platform` 是唯一允许未来 `unsafe`
//! / Win32 的目录。
//!
//! 本阶段为 v0.1 骨架：外部能力（捕获 / OCR / LLM / 搜索 / 翻译 / 下载 / 更新）为
//! 契约 §12 的确定性占位实现，真实实现以 `TODO(real)` 标注。

#![forbid(unsafe_code)]
// 骨架阶段的 Clippy 约定：
// - AppError 是跨 IPC 边界的完整错误对象（契约 §8.1 六个字段），体积较大是刻意的；
// - 命令 / 契约函数的参数个数由 docs/接口契约.md 固定，不可拆包；
// - 单元测试中大量「默认配置 + 改字段」的写法。
#![allow(clippy::result_large_err)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::collapsible_if)]

pub mod app;
pub mod commands;
pub mod contract;
pub mod domain;
pub mod events;
pub mod platform;
pub mod resources;
pub mod services;

/// 构建并运行 Tauri 应用。
///
/// 启动顺序（概要设计 §4.1）：单实例 → 插件 → setup（生命周期：路径/日志/配置/托盘/热键/
/// 窗口/引擎/更新检查）→ 命令注册 → 事件循环。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例必须最先注册：第二实例唤起已有实例后退出（概要设计 §4.1）。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Err(err) = crate::app::windows::show_main(app) {
                tracing::warn!(code = %err.code, "单实例唤起主窗口失败");
            }
            crate::events::emit_tray_action(
                app,
                &crate::contract::events::TrayActionPayload {
                    id: "singleInstance".to_string(),
                },
            );
        }))
        // 全局热键插件（M03）；注册逻辑见 `app::hotkeys`，回调在此转发。
        // TODO(api-check): 校验 `with_handler` 闭包参数类型与 `event.state()` 名字。
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    crate::app::hotkeys::on_shortcut(app, shortcut, event.state());
                })
                .build(),
        )
        // 开机自启（M02）；骨架模式只改内存状态。
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // 外部链接打开（M19）；骨架模式只记日志。
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let mut handle = app.handle().clone();
            match crate::app::lifecycle::setup(&mut handle) {
                Ok(()) => Ok(()),
                Err(err) => {
                    tracing::error!(
                        code = %err.code,
                        message = %err.message,
                        "应用初始化失败"
                    );
                    Err(crate::app::lifecycle::setup_error(err))
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            // —— 会话 / 选区 / 动作（1–8）
            commands::capture::capture_freeze,
            commands::capture::capture_cancel,
            commands::selection::selection_commit,
            commands::selection::selection_reset,
            commands::action::menu_execute,
            commands::action::task_cancel,
            commands::action::task_followup,
            commands::action::result_close,
            // —— 配置 / 引导（9–16）
            commands::config::config_get,
            commands::config::config_patch,
            commands::config::config_import,
            commands::config::config_export,
            commands::config::config_reset,
            commands::onboarding::onboarding_state,
            commands::onboarding::onboarding_update,
            commands::onboarding::onboarding_finish,
            // —— 供应商 / 模型 / 翻译（17–22）
            commands::provider::provider_test,
            commands::models::models_list,
            commands::models::models_download,
            commands::models::models_cancel,
            commands::models::models_remove,
            commands::translate::translate_test,
            // —— 系统 / 窗口（23–29）
            commands::system::hotkey_probe,
            commands::system::set_autostart,
            commands::system::open_external,
            commands::system::app_stats,
            commands::window::window_show,
            commands::window::window_hide,
            commands::system::app_quit,
        ])
        .run(tauri::generate_context!())
        .expect("框选 AI 启动失败");
}
