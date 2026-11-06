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

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::app::state::AppState;
use crate::contract::config::{ResultPosition, ResultWindowPrefs};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::OverlayReadyPayload;
use crate::contract::types::{RectPhys, VirtualDesktop, WindowLabel};
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

// ---------------------------------------------------------------------------
// 结果窗几何（v1.1.0：`ui.resultWindow` 真正生效）
// ---------------------------------------------------------------------------

/// 结果窗最小逻辑宽度（与 `min_inner_size` 一致，防御配置被手改为非法值）。
const RESULT_MIN_WIDTH_LOGICAL: f64 = 320.0;
/// 结果窗最小逻辑高度（与 `min_inner_size` 一致）。
const RESULT_MIN_HEIGHT_LOGICAL: f64 = 240.0;
/// 结果窗最大逻辑尺寸（防御异常配置产生超大窗口）。
const RESULT_MAX_LOGICAL_SIZE: f64 = 8192.0;

/// 已应用的结果窗外观偏好（进程级）。
///
/// 仅在偏好**变化**时才重新设置尺寸：结果窗宽度允许用户手动调整、高度由前端按内容
/// 自适应，若每次显示都强制改回配置尺寸，会覆盖用户调整并导致窗口抖动。
static APPLIED_RESULT_PREFS: Lazy<Mutex<Option<ResultWindowPrefs>>> =
    Lazy::new(|| Mutex::new(None));

/// 结果窗几何计划（物理像素；纯逻辑结果，便于单测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultWindowPlan {
    /// 目标 client 宽度（物理像素）。
    pub width_phys: u32,
    /// 目标 client 高度（物理像素）。
    pub height_phys: u32,
    /// 需要移动到的左上角（物理像素）；`None` = 保持窗口当前位置。
    pub position_phys: Option<(i32, i32)>,
    /// 是否置顶（`ui.resultWindow.pin`）。
    pub always_on_top: bool,
}

/// 防御非法缩放比。
fn safe_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// 逻辑尺寸收敛：非法值回落默认，合法值夹在 `[min, 8192]`。
fn clamp_logical_size(value: f64, fallback: f64, min: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value.clamp(min, RESULT_MAX_LOGICAL_SIZE)
    } else {
        fallback
    }
}

/// 由 `ui.resultWindow` + 当前选区 + 虚拟桌面几何计算结果窗几何（纯函数）。
///
/// - 尺寸：逻辑宽高 × 窗口 `scale_factor` = 物理像素（混合 DPI 下以结果窗自身 dpr 为准）；
/// - `position = follow`：以当前选区与虚拟桌面 bounds 调用
///   [`crate::platform::coords::follow_position`]（选区右下 + 8px，越界翻转到左上并夹回桌面内）；
/// - `position = fixed`：返回 `None`，保持窗口创建时的默认居中 / 用户上次位置；
/// - `pin` → 是否置顶。
pub fn plan_result_window(
    prefs: &ResultWindowPrefs,
    selection: Option<&RectPhys>,
    desktop: &RectPhys,
    scale_factor: f64,
) -> ResultWindowPlan {
    let scale = safe_scale(scale_factor);
    let defaults = ResultWindowPrefs::default();
    let width_logical = clamp_logical_size(prefs.width, defaults.width, RESULT_MIN_WIDTH_LOGICAL);
    let height_logical =
        clamp_logical_size(prefs.height, defaults.height, RESULT_MIN_HEIGHT_LOGICAL);
    let width_phys = (width_logical * scale).round().max(1.0) as u32;
    let height_phys = (height_logical * scale).round().max(1.0) as u32;
    let position_phys = match (prefs.position, selection) {
        (ResultPosition::Follow, Some(selection)) => {
            let placed = crate::platform::coords::follow_position(
                selection,
                f64::from(width_phys),
                f64::from(height_phys),
                desktop,
            );
            Some((placed.x.round() as i32, placed.y.round() as i32))
        }
        // Fixed：保持默认居中 / 上次位置；Follow 但尚无选区时同样保持。
        _ => None,
    };
    ResultWindowPlan {
        width_phys,
        height_phys,
        position_phys,
        always_on_top: prefs.pin,
    }
}

/// 当前配置里的结果窗偏好（`AppState` 尚未托管时回退默认值）。
fn result_window_prefs(app: &AppHandle) -> ResultWindowPrefs {
    app.try_state::<AppState>()
        .map(|state| state.snapshot_config().ui.result_window)
        .unwrap_or_default()
}

/// 当前活动会话的选区（物理像素）；无活动会话 / 未提交选区 / 无屏幕的合成文本动作
/// （快速翻译热键的 1px² 占位选区）时 `None`，避免 `follow` 把结果窗放到虚拟桌面左上角。
fn current_selection_rect(app: &AppHandle) -> Option<RectPhys> {
    let state = app.try_state::<AppState>()?;
    let selection = state.session.lock().current_selection()?;
    let rect = selection.rect_phys;
    if rect.width <= 1.0 || rect.height <= 1.0 {
        return None;
    }
    Some(rect)
}

/// 确保结果窗存在（不显示；尺寸 / 置顶按 `ui.resultWindow` 初始化）。
pub fn ensure_result(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(window) = app.get_webview_window(RESULT_WINDOW) {
        return Ok(window);
    }
    let prefs = result_window_prefs(app);
    let defaults = ResultWindowPrefs::default();
    let width = clamp_logical_size(prefs.width, defaults.width, RESULT_MIN_WIDTH_LOGICAL);
    let height = clamp_logical_size(prefs.height, defaults.height, RESULT_MIN_HEIGHT_LOGICAL);
    let window = WebviewWindowBuilder::new(
        app,
        RESULT_WINDOW,
        WebviewUrl::App("index.html?window=result".into()),
    )
    .title("框选 AI · 结果")
    .inner_size(width, height)
    .min_inner_size(320.0, 240.0)
    .decorations(false)
    // 不透明 + 无系统阴影：内容铺满窗口，窗口自适应内容高度，避免卡片外出现透明一圈。
    .transparent(false)
    .shadow(false)
    .always_on_top(prefs.pin)
    .skip_taskbar(true)
    .resizable(true)
    // 位置策略 = fixed 时的默认位置（居中）；follow 在 `show_result` 显示前重新定位。
    .center()
    .visible(false)
    .build()
    .map_err(|err| {
        AppError::new(ErrorCode::OverlayFailed, Some(err.to_string())).with_fallback(vec!["retry"])
    })?;
    // 记录已应用的偏好：首次显示不再重复改尺寸（宽度可能已被用户调整、高度由前端自适应）。
    *APPLIED_RESULT_PREFS.lock() = Some(prefs);
    Ok(window)
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

/// 显示结果窗：应用 `ui.resultWindow`（初始尺寸 / 置顶 / 跟随选区定位）。
pub fn show_result(app: &AppHandle) -> AppResult<()> {
    let window = ensure_result(app)?;
    let prefs = result_window_prefs(app);
    let desktop = virtual_desktop(app);
    // 跨混合 DPI：以结果窗自身缩放比换算逻辑尺寸（与虚拟桌面 dpr 可能不同）。
    let scale = window.scale_factor().unwrap_or(desktop.dpr);
    let selection = current_selection_rect(app);
    let plan = plan_result_window(&prefs, selection.as_ref(), &desktop.rect, scale);

    // 尺寸只在偏好变化时应用一次，避免覆盖用户手动调整的宽度（高度由前端按内容自适应）。
    let resized = {
        let mut applied = APPLIED_RESULT_PREFS.lock();
        if applied.as_ref() == Some(&prefs) {
            false
        } else {
            *applied = Some(prefs.clone());
            true
        }
    };
    if resized {
        window
            .set_size(PhysicalSize::new(plan.width_phys, plan.height_phys))
            .map_err(window_error)?;
    }
    if let Err(err) = window.set_always_on_top(plan.always_on_top) {
        tracing::warn!(error = %err, "结果窗置顶设置失败");
    }
    // position = follow：显示前移动到选区旁；fixed 时保持默认居中 / 上次位置。
    if let Some((x, y)) = plan.position_phys {
        window
            .set_position(PhysicalPosition::new(x, y))
            .map_err(window_error)?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RectPhys {
        RectPhys {
            x,
            y,
            width,
            height,
        }
    }

    fn prefs(position: ResultPosition, pin: bool, width: f64, height: f64) -> ResultWindowPrefs {
        ResultWindowPrefs {
            position,
            pin,
            opacity: 1.0,
            width,
            height,
            extra: serde_json::Map::new(),
        }
    }

    fn desktop() -> RectPhys {
        rect(0.0, 0.0, 1920.0, 1080.0)
    }

    #[test]
    fn follow_places_result_below_right_of_selection() {
        let selection = rect(100.0, 100.0, 200.0, 100.0);
        let plan = plan_result_window(
            &prefs(ResultPosition::Follow, true, 460.0, 420.0),
            Some(&selection),
            &desktop(),
            1.0,
        );
        assert_eq!(plan.width_phys, 460);
        assert_eq!(plan.height_phys, 420);
        // 选区右下 + 8px（与 coords::follow_position 语义一致）。
        assert_eq!(plan.position_phys, Some((308, 208)));
        assert!(plan.always_on_top);
    }

    #[test]
    fn follow_flips_and_clamps_near_bottom_right() {
        let selection = rect(1800.0, 1000.0, 100.0, 80.0);
        let plan = plan_result_window(
            &prefs(ResultPosition::Follow, true, 460.0, 420.0),
            Some(&selection),
            &desktop(),
            1.0,
        );
        let (x, y) = plan.position_phys.expect("follow 应给出位置");
        assert!(x >= 0 && y >= 0);
        assert!(x + plan.width_phys as i32 <= 1920);
        assert!(y + plan.height_phys as i32 <= 1080);
    }

    #[test]
    fn physical_size_scales_with_result_window_dpr() {
        let selection = rect(100.0, 100.0, 200.0, 100.0);
        let plan = plan_result_window(
            &prefs(ResultPosition::Follow, true, 460.0, 420.0),
            Some(&selection),
            &desktop(),
            1.5,
        );
        // 尺寸物理 = 逻辑 × scaleFactor。
        assert_eq!(plan.width_phys, 690);
        assert_eq!(plan.height_phys, 630);
    }

    #[test]
    fn follow_works_on_negative_multi_monitor_bounds() {
        let desktop = rect(-1080.0, -470.0, 3640.0, 1920.0);
        let selection = rect(-1000.0, -400.0, 200.0, 100.0);
        let plan = plan_result_window(
            &prefs(ResultPosition::Follow, true, 460.0, 420.0),
            Some(&selection),
            &desktop,
            1.0,
        );
        let (x, y) = plan.position_phys.expect("follow 应给出位置");
        assert!(x >= -1080);
        assert!(y >= -470);
        assert!(x + plan.width_phys as i32 <= -1080 + 3640);
        assert!(y + plan.height_phys as i32 <= -470 + 1920);
    }

    #[test]
    fn fixed_keeps_position_and_applies_pin() {
        let selection = rect(100.0, 100.0, 200.0, 100.0);
        let plan = plan_result_window(
            &prefs(ResultPosition::Fixed, false, 500.0, 400.0),
            Some(&selection),
            &desktop(),
            2.0,
        );
        // Fixed：不移动窗口（保持默认居中 / 上次位置）。
        assert_eq!(plan.position_phys, None);
        assert_eq!(plan.width_phys, 1000);
        assert_eq!(plan.height_phys, 800);
        assert!(!plan.always_on_top);
    }

    #[test]
    fn follow_without_selection_keeps_position() {
        let plan = plan_result_window(
            &prefs(ResultPosition::Follow, true, 460.0, 420.0),
            None,
            &desktop(),
            1.0,
        );
        assert_eq!(plan.position_phys, None);
    }

    #[test]
    fn invalid_scale_and_size_fall_back_to_defaults() {
        let selection = rect(10.0, 10.0, 10.0, 10.0);
        // 非法缩放比按 1.0；非法 / 过小尺寸回落到默认或最小值。
        let plan = plan_result_window(
            &prefs(ResultPosition::Follow, true, 0.0, -5.0),
            Some(&selection),
            &desktop(),
            f64::NAN,
        );
        assert_eq!(plan.width_phys, 460);
        assert_eq!(plan.height_phys, 420);
        assert_eq!(plan.position_phys, Some((28, 28)));

        let plan = plan_result_window(
            &prefs(ResultPosition::Follow, false, 10.0, 10.0),
            None,
            &desktop(),
            0.0,
        );
        assert_eq!(plan.width_phys, 320);
        assert_eq!(plan.height_phys, 240);
        assert_eq!(plan.position_phys, None);
        assert!(!plan.always_on_top);
    }
}
