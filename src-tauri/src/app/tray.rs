//! 系统托盘（M02 / 概要设计 §4.1）。
//!
//! 原生 `TrayIcon`（非 Web 实现），菜单：开始框选 / 设置 / 重新运行引导 / 暂停热键 / 退出。
//! 菜单事件同时：本地处理 `capture`/`openSettings`/`togglePause`/`quit`，并广播 `tray.action`。
//!
//! TODO(api-check): 校验 `TrayIconBuilder` / `MenuItem::with_id` / `Menu::with_items` 的
//! 实际签名（Tauri 2.x）。

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use crate::app::state::AppState;
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::TrayActionPayload;
use crate::events::emit_tray_action;

/// 托盘菜单项 id。
pub const MENU_CAPTURE: &str = "capture";
/// 打开设置。
pub const MENU_SETTINGS: &str = "settings";
/// 重新运行首启引导。
pub const MENU_RERUN_ONBOARDING: &str = "rerunOnboarding";
/// 暂停 / 恢复热键。
pub const MENU_TOGGLE_PAUSE: &str = "togglePause";
/// 退出应用。
pub const MENU_QUIT: &str = "quit";

/// 创建托盘图标与菜单。
pub fn create(app: &AppHandle) -> AppResult<()> {
    let capture = MenuItem::with_id(app, MENU_CAPTURE, "开始框选", true, None::<&str>)
        .map_err(menu_error)?;
    let settings = MenuItem::with_id(app, MENU_SETTINGS, "设置", true, None::<&str>)
        .map_err(menu_error)?;
    let rerun = MenuItem::with_id(app, MENU_RERUN_ONBOARDING, "重新运行引导", true, None::<&str>)
        .map_err(menu_error)?;
    let toggle = MenuItem::with_id(app, MENU_TOGGLE_PAUSE, "暂停热键", true, None::<&str>)
        .map_err(menu_error)?;
    let separator = PredefinedMenuItem::separator(app).map_err(menu_error)?;
    let quit =
        MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>).map_err(menu_error)?;

    let menu = Menu::with_items(
        app,
        &[&capture, &settings, &rerun, &toggle, &separator, &quit],
    )
    .map_err(menu_error)?;

    let mut builder = TrayIconBuilder::new()
        .tooltip("框选 AI")
        .menu(&menu)
        .on_menu_event(on_menu_event);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app).map_err(tray_error)?;
    Ok(())
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id.as_ref().to_string();
    handle_action(app, &id);
}

/// 托盘菜单动作分发。
pub fn handle_action(app: &AppHandle, id: &str) {
    match id {
        MENU_CAPTURE => {
            emit_tray_action(
                app,
                &TrayActionPayload {
                    id: MENU_CAPTURE.to_string(),
                },
            );
            let app = app.clone();
            let _ = tauri::async_runtime::spawn(async move {
                if let Err(err) = crate::commands::capture::do_capture_freeze(&app).await {
                    tracing::warn!(code = %err.code, message = %err.message, "托盘触发框选失败");
                }
            });
        }
        MENU_SETTINGS => {
            if let Err(err) = crate::app::windows::show_main(app) {
                tracing::warn!(error = %err, "显示主窗口失败");
            }
            emit_tray_action(
                app,
                &TrayActionPayload {
                    id: "openSettings".to_string(),
                },
            );
        }
        MENU_RERUN_ONBOARDING => {
            if let Err(err) = crate::app::windows::show_main(app) {
                tracing::warn!(error = %err, "显示主窗口失败");
            }
            emit_tray_action(
                app,
                &TrayActionPayload {
                    id: MENU_RERUN_ONBOARDING.to_string(),
                },
            );
        }
        MENU_TOGGLE_PAUSE => {
            let _paused = crate::app::hotkeys::toggle_pause(app);
            emit_tray_action(
                app,
                &TrayActionPayload {
                    id: MENU_TOGGLE_PAUSE.to_string(),
                },
            );
        }
        MENU_QUIT => {
            emit_tray_action(
                app,
                &TrayActionPayload {
                    id: MENU_QUIT.to_string(),
                },
            );
            if let Err(err) = crate::app::lifecycle::shutdown(app) {
                tracing::warn!(code = %err.code, "退出前保存配置失败");
            }
            app.exit(0);
        }
        other => {
            tracing::debug!(menu = %other, "未处理的托盘菜单项");
        }
    }
    // 暂停状态变化后同步菜单文案（如果托盘仍存在）。
    sync_pause_text(app);
}

fn sync_pause_text(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let paused = app
            .try_state::<AppState>()
            .map(|state| state.is_paused())
            .unwrap_or(false);
        let _ = tray.set_tooltip(Some(if paused {
            "框选 AI（热键已暂停）"
        } else {
            "框选 AI"
        }));
    }
}

fn menu_error(err: tauri::Error) -> AppError {
    AppError::new(ErrorCode::OverlayFailed, Some(format!("托盘菜单创建失败：{err}")))
}

fn tray_error(err: tauri::Error) -> AppError {
    AppError::new(ErrorCode::OverlayFailed, Some(format!("托盘创建失败：{err}")))
}
