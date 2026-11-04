//! 窗口管理（M13 / 契约 §2）。
//!
//! - `main` 由 `tauri.conf.json` 静态创建（`visible: false`）；
//! - `overlay` / `result` 惰性创建、复用；overlay 尺寸取主显示器物理尺寸。
//!
//! TODO(real): 多显示器虚拟桌面矩形（overlay 需覆盖全部显示器，坐标为负时正确摆放）。
//! TODO(api-check): 校验 `WebviewWindowBuilder` 的 `transparent` / `skip_taskbar` /
//! `always_on_top` 方法名（Tauri 2.x）。

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::WindowLabel;
use crate::platform::capture::primary_monitor_geometry;

/// 主窗口 label（静态创建）。
pub const MAIN_WINDOW: &str = "main";
/// overlay 窗口 label。
pub const OVERLAY_WINDOW: &str = "overlay";
/// 结果窗 label。
pub const RESULT_WINDOW: &str = "result";

/// 确保 overlay 窗口存在（不显示）。
pub fn ensure_overlay(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(window) = app.get_webview_window(OVERLAY_WINDOW) {
        return Ok(window);
    }
    let desktop = primary_monitor_geometry(app);
    let width = desktop.rect.width.max(1.0);
    let height = desktop.rect.height.max(1.0);

    WebviewWindowBuilder::new(
        app,
        OVERLAY_WINDOW,
        WebviewUrl::App("index.html?window=overlay".into()),
    )
    .title("框选 AI · 选框")
    .inner_size(width, height)
    .position(desktop.rect.x, desktop.rect.y)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .visible(false)
    .build()
    .map_err(|err| {
        AppError::new(ErrorCode::OverlayFailed, Some(err.to_string()))
            .with_fallback(vec!["retry"])
    })
}

/// 确保结果窗存在（不显示）。
pub fn ensure_result(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(window) = app.get_webview_window(RESULT_WINDOW) {
        return Ok(window);
    }
    WebviewWindowBuilder::new(
        app,
        RESULT_WINDOW,
        WebviewUrl::App("index.html?window=result".into()),
    )
    .title("框选 AI · 结果")
    .inner_size(460.0, 420.0)
    .min_inner_size(320.0, 240.0)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(true)
    .visible(false)
    .build()
    .map_err(|err| {
        AppError::new(ErrorCode::OverlayFailed, Some(err.to_string()))
            .with_fallback(vec!["retry"])
    })
}

/// 显示 overlay 并聚焦。
pub fn show_overlay(app: &AppHandle) -> AppResult<()> {
    let window = ensure_overlay(app)?;
    window.show().map_err(window_error)?;
    let _ = window.set_focus();
    Ok(())
}

/// 隐藏 overlay。
pub fn hide_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(OVERLAY_WINDOW) {
        let _ = window.hide();
    }
}

/// 显示结果窗。
pub fn show_result(app: &AppHandle) -> AppResult<()> {
    let window = ensure_result(app)?;
    window.show().map_err(window_error)?;
    let _ = window.set_focus();
    Ok(())
}

/// 隐藏结果窗。
pub fn hide_result(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(RESULT_WINDOW) {
        let _ = window.hide();
    }
}

/// 显示主窗口（设置 / 引导）。
pub fn show_main(app: &AppHandle) -> AppResult<()> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return Err(AppError::new(
            ErrorCode::OverlayFailed,
            Some("主窗口不存在".to_string()),
        ));
    };
    window.show().map_err(window_error)?;
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
}

/// 隐藏主窗口。
pub fn hide_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.hide();
    }
}

/// 按契约 `WindowLabel` 显示窗口。
pub fn show(app: &AppHandle, label: WindowLabel) -> AppResult<()> {
    match label {
        WindowLabel::Main => show_main(app),
        WindowLabel::Overlay => show_overlay(app),
        WindowLabel::Result => show_result(app),
    }
}

/// 按契约 `WindowLabel` 隐藏窗口。
pub fn hide(app: &AppHandle, label: WindowLabel) {
    match label {
        WindowLabel::Main => hide_main(app),
        WindowLabel::Overlay => hide_overlay(app),
        WindowLabel::Result => hide_result(app),
    }
}

/// 启动时预创建结果窗（隐藏），overlay 保持惰性创建（概要设计 §4.1 第 6 步）。
pub fn prepare_hidden_windows(app: &AppHandle) -> AppResult<()> {
    ensure_result(app)?;
    // overlay 不预创建：首次 capture_freeze 时按虚拟桌面尺寸创建。
    Ok(())
}

fn window_error(err: tauri::Error) -> AppError {
    AppError::new(ErrorCode::OverlayFailed, Some(err.to_string()))
}
