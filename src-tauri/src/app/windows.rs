//! 窗口管理（M13 / 契约 §2）。
//!
//! - `main` 由 `tauri.conf.json` 静态创建（`visible: false`）；
//! - `overlay` / `result` 惰性创建、复用；overlay 覆盖**整个虚拟桌面**（所有显示器物理矩形
//!   的外接矩形，允许负坐标），见 S-01「混合 DPI 多屏坐标一致性」。
//!
//! ## overlay 几何契约（S-01 实测修正）
//!
//! 前端 `toPhys` 的换算是 `物理 = virtualRect.左上角 + client CSS 像素 × dpr`，因此必须保证：
//! 1. overlay **client 区**左上角 == `virtualRect` 左上角（不是外框左上角）；
//! 2. client 区宽高 == `virtualRect` 宽高（物理像素）；
//! 3. 下发的 dpr == 该 WebView 实际生效的 `devicePixelRatio`（= 窗口 `scale_factor()`）。
//!
//! 两个实测坑（2026-11-04，本机 2560x1440@150% + 1080x1920@100%）：
//! - Tauri builder 的 `.inner_size()/.position()` 只接受**逻辑像素**，跨混合 DPI 直接传物理值
//!   会被二次缩放；本实现改用 `set_size(PhysicalSize) / set_position(PhysicalPosition)`
//!   （`set_size` 是 client 尺寸、`set_position` 是外框位置）；
//! - 无边框窗口仍可能带 DWM 阴影 inset，实测 client 原点比外框偏 `(11,2)px`。仅靠
//!   `set_position` 会让坐标整体偏移，因此这里按 `inner_position()` 迭代校正，并对
//!   `shadow(false)` 的窗口保持幂等（偏差为 0 时提前退出）。
//!
//! TODO(real): 显示器热插拔 / 分辨率变化时监听 `WindowEvent::ScaleFactorChanged` 主动重排；
//! 当前在每次 `capture_freeze` 时重新对齐（幂等）。

use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::OverlayReadyPayload;
use crate::contract::types::{VirtualDesktop, WindowLabel};
use crate::domain::session::SessionPhase;
use crate::events::emit_overlay_ready;
use crate::services::capture_service::virtual_desktop;

/// 主窗口 label（静态创建）。
pub const MAIN_WINDOW: &str = "main";
/// overlay 窗口 label。
pub const OVERLAY_WINDOW: &str = "overlay";
/// 结果窗 label。
pub const RESULT_WINDOW: &str = "result";

/// overlay 几何对齐容差（物理像素）。S-01 验收阈值 2px。
const ALIGN_TOLERANCE_PX: i32 = 2;

/// 把窗口 **client 区**对齐到虚拟桌面物理矩形。
///
/// - `set_size(PhysicalSize)`：设置 client 尺寸（不是外框）；
/// - `set_position(PhysicalPosition)`：设置外框位置；
/// - 以 `inner_position()`（client 原点物理坐标）为准迭代修正外框位置，吸收无边框阴影 inset
///   与 DPI 切换带来的重排；
/// - 尺寸再次复核：DPI 变化时 Windows 会按建议矩形重排，需重新设置 client 尺寸。
fn align_overlay_window(window: &WebviewWindow, desktop: &VirtualDesktop) -> AppResult<()> {
    let target_x = desktop.rect.x.round() as i32;
    let target_y = desktop.rect.y.round() as i32;
    let target_width = desktop.rect.width.round().max(1.0) as u32;
    let target_height = desktop.rect.height.round().max(1.0) as u32;

    window
        .set_size(PhysicalSize::new(target_width, target_height))
        .map_err(window_error)?;
    // 位置修正：outer 与 client 原点的差值可能随窗口 DPI / 阴影 inset 变化，迭代 3 次足够收敛。
    for _ in 0..3 {
        let inner = window.inner_position().map_err(window_error)?;
        let dx = target_x - inner.x;
        let dy = target_y - inner.y;
        if dx == 0 && dy == 0 {
            break;
        }
        let outer = window.outer_position().map_err(window_error)?;
        window
            .set_position(PhysicalPosition::new(outer.x + dx, outer.y + dy))
            .map_err(window_error)?;
    }
    // 尺寸复核（位置修正后 client 尺寸可能因 DPI 重排而变）。
    let inner_size = window.inner_size().map_err(window_error)?;
    if inner_size.width != target_width || inner_size.height != target_height {
        window
            .set_size(PhysicalSize::new(target_width, target_height))
            .map_err(window_error)?;
    }
    Ok(())
}

/// 当前 overlay 窗口 client 区是否已经与虚拟桌面一致（容差 [`ALIGN_TOLERANCE_PX`]）。
fn overlay_matches(window: &WebviewWindow, desktop: &VirtualDesktop) -> bool {
    let Ok(inner) = window.inner_position() else {
        return false;
    };
    let Ok(size) = window.inner_size() else {
        return false;
    };
    let target_x = desktop.rect.x.round() as i32;
    let target_y = desktop.rect.y.round() as i32;
    let target_width = desktop.rect.width.round().max(1.0) as u32 as i32;
    let target_height = desktop.rect.height.round().max(1.0) as u32 as i32;
    (inner.x - target_x).abs() <= ALIGN_TOLERANCE_PX
        && (inner.y - target_y).abs() <= ALIGN_TOLERANCE_PX
        && (size.width as i32 - target_width).abs() <= ALIGN_TOLERANCE_PX
        && (size.height as i32 - target_height).abs() <= ALIGN_TOLERANCE_PX
}

/// 确保 overlay 窗口存在（不显示），并已对齐虚拟桌面。
pub fn ensure_overlay(app: &AppHandle) -> AppResult<WebviewWindow> {
    let desktop = virtual_desktop(app);
    if let Some(window) = app.get_webview_window(OVERLAY_WINDOW) {
        // 复用窗口：显示器分辨率 / 缩放变化后重新对齐几何（幂等、无副作用）。
        align_overlay_window(&window, &desktop)?;
        return Ok(window);
    }

    // builder 只接受逻辑像素：这里给一个占位尺寸/位置，随后立刻用 Physical* 对齐。
    let window = WebviewWindowBuilder::new(
        app,
        OVERLAY_WINDOW,
        WebviewUrl::App("index.html?window=overlay".into()),
    )
    .title("框选 AI · 选框")
    .inner_size(1024.0, 768.0)
    .decorations(false)
    // 关闭 DWM 阴影：无边框跨屏窗口不需要阴影，且阴影 inset 会引入 client 原点偏移。
    .shadow(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .visible(false)
    .build()
    .map_err(|err| {
        AppError::new(ErrorCode::OverlayFailed, Some(err.to_string())).with_fallback(vec!["retry"])
    })?;
    align_overlay_window(&window, &desktop)?;
    Ok(window)
}

/// overlay 覆盖的虚拟桌面几何：物理矩形取显示器外接矩形，**dpr 以 overlay 窗口实际缩放比为准**。
///
/// 为什么不能直接用主屏 / 逐屏 dpr：overlay 是单个跨屏窗口，WebView 只有一个
/// `devicePixelRatio`；本机混合 DPI 下 Windows 按「窗口与各显示器重叠面积最大者」决定窗口
/// DPI（实测为 144/96 = 1.5，即使窗口左上角在 100% 副屏上）。前端 `toPhys` 必须用这个
/// 统一值，否则 100% 副屏区域会按 1.5 倍偏移（实测可达数百 px）。
pub fn overlay_virtual_desktop(app: &AppHandle) -> VirtualDesktop {
    let mut desktop = virtual_desktop(app);
    if let Some(window) = app.get_webview_window(OVERLAY_WINDOW) {
        if let Ok(scale) = window.scale_factor() {
            if scale.is_finite() && scale > 0.0 {
                desktop.dpr = scale;
            }
        }
    }
    desktop
}

/// `show` 之后的异步复核：Windows 的 DPI / 窗口位置消息是异步投递的，show 后可能继续重排。
///
/// 最多等 `5 × 40ms`；已对齐时首个 40ms 后即返回（幂等、快速路径无额外开销）。
pub async fn settle_overlay_geometry(app: &AppHandle) {
    for _ in 0..5 {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        let Some(window) = app.get_webview_window(OVERLAY_WINDOW) else {
            return;
        };
        let desktop = virtual_desktop(app);
        if overlay_matches(&window, &desktop) {
            return;
        }
        if let Err(err) = align_overlay_window(&window, &desktop) {
            tracing::warn!(code = %err.code, "overlay 几何二次对齐失败");
        }
    }
}

/// 在 overlay 的 WebView 完成页面加载后补发 `overlay:ready` 快照。
///
/// 背景：`capture_freeze` 在 `show_overlay` 后立即广播 `overlay:ready`，而 overlay 窗口
/// 首次创建时 WebView 尚未加载完 JS，该事件会丢失（冷启动必现）。`lib.rs` 的
/// `Builder::on_page_load` 在页面加载完成时调用本函数，从 `AppState` 取当前会话快照补发；
/// 前端对重复的 `overlay:ready` 做幂等处理即可。
pub fn emit_overlay_snapshot(app: &AppHandle) {
    let state = app.state::<crate::app::state::AppState>();
    let payload = {
        let store = state.session.lock();
        let Some(session) = store.current() else {
            return;
        };
        if session.phase != SessionPhase::Selecting {
            return;
        }
        let Some(frame) = session.frame.clone() else {
            return;
        };
        // S-01：dpr 取 overlay 窗口实际缩放比，而不是显示器/主屏 dpr。
        let desktop = overlay_virtual_desktop(app);
        OverlayReadyPayload {
            session: session.id.clone(),
            virtual_rect: desktop.rect,
            dpr: desktop.dpr,
            frame,
        }
    };
    emit_overlay_ready(app, &payload);
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
        AppError::new(ErrorCode::OverlayFailed, Some(err.to_string())).with_fallback(vec!["retry"])
    })
}

/// 显示 overlay 并聚焦（显示前/后各对齐一次，吸收异步 DPI 重排）。
pub fn show_overlay(app: &AppHandle) -> AppResult<()> {
    let window = ensure_overlay(app)?;
    window.show().map_err(window_error)?;
    let _ = window.set_focus();
    // show 后窗口可能收到 WM_DPICHANGED / WM_WINDOWPOSCHANGING 的异步重排，再对齐一次（幂等）。
    align_overlay_window(&window, &virtual_desktop(app))?;
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

/// 显示主窗口（设置 / 引导）；已最小化时还原并聚焦。
///
/// 若窗口已被销毁（例如被外部强制关闭 / 旧版本遗留状态），按当前布局重建主窗口——
/// 保证「托盘左键 / 托盘菜单 / 单实例唤起」始终能把设置页叫回来。
pub fn show_main(app: &AppHandle) -> AppResult<()> {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        window.show().map_err(window_error)?;
        let _ = window.set_focus();
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(
        app,
        MAIN_WINDOW,
        WebviewUrl::App("index.html?window=main".into()),
    )
    .title("框选 AI")
    .inner_size(980.0, 720.0)
    .min_inner_size(820.0, 600.0)
    // 无系统标题栏：由前端自绘标题栏（最小化 / 最大化 / 关闭 / 拖拽 / 缩放）。
    .decorations(false)
    .center()
    .visible(true)
    .build()
    .map_err(window_error)?;
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
