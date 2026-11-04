//! S-01 混合 DPI 多屏坐标一致性 —— 纯函数不变量测试（集成测试）。
//!
//! 真机参数（2026-11-04 实测，Windows 11 26200）：
//! - 主屏 \\.\DISPLAY1：(0,0) 2560x1440 @150%（GetDpiForMonitor=144）
//! - 副屏 \\.\DISPLAY2：(-1080,-470) 1080x1920 @100%（=96）
//! - 虚拟桌面外接矩形：(-1080,-470) 3640x1920（= SM_XVIRTUALSCREEN…）
//!
//! 本文件只测不依赖 `AppHandle` 的纯函数：外接矩形、负坐标、点覆盖、
//! 统一 dpr 的换算不变式、以及「逐屏混用 dpr」时的偏移量级。

use snip_ask_lib::contract::types::RectPhys;
use snip_ask_lib::platform::coords::{mixed_dpr_error, rect_contains, union_rect, virtual_desktop};

/// 本机实测两屏物理矩形。
fn primary() -> RectPhys {
    RectPhys {
        x: 0.0,
        y: 0.0,
        width: 2560.0,
        height: 1440.0,
    }
}

fn secondary() -> RectPhys {
    RectPhys {
        x: -1080.0,
        y: -470.0,
        width: 1080.0,
        height: 1920.0,
    }
}

#[test]
fn virtual_desktop_union_matches_win32_sm_metrics() {
    // 工具实测：SM_XVIRTUALSCREEN=-1080, SM_YVIRTUALSCREEN=-470,
    // SM_CXVIRTUALSCREEN=3640, SM_CYVIRTUALSCREEN=1920。
    let union = union_rect([primary(), secondary()]).expect("至少一个显示器");
    assert_eq!(
        union,
        RectPhys {
            x: -1080.0,
            y: -470.0,
            width: 3640.0,
            height: 1920.0,
        }
    );
    // 负坐标原点必须原样保留，不能被 clamp 到 0。
    assert!(union.x < 0.0 && union.y < 0.0);
}

#[test]
fn virtual_desktop_keeps_negative_origin_and_dpr() {
    let union = union_rect([primary(), secondary()]).expect("至少一个显示器");
    // 锚定屏取包含外接矩形左上角的显示器（100% 副屏）：dpr=1.0。
    let desktop = virtual_desktop(union.x, union.y, union.width, union.height, 1.0);
    assert_eq!(desktop.rect.x, -1080.0);
    assert_eq!(desktop.rect.y, -470.0);
    assert_eq!(desktop.rect.width, 3640.0);
    assert_eq!(desktop.rect.height, 1920.0);
    assert_eq!(desktop.dpr, 1.0);
}

#[test]
fn union_rect_handles_negative_and_vertical_layouts() {
    // 副屏在主屏正上方（行排布）
    let union = union_rect([
        primary(),
        RectPhys {
            x: 0.0,
            y: -1080.0,
            width: 1920.0,
            height: 1080.0,
        },
    ])
    .expect("非空");
    assert_eq!((union.x, union.y), (0.0, -1080.0));
    assert_eq!((union.width, union.height), (2560.0, 2520.0));

    // 副屏在左侧但更矮（y 不变）
    let union = union_rect([
        primary(),
        RectPhys {
            x: -1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        },
    ])
    .expect("非空");
    assert_eq!((union.x, union.y), (-1920.0, 0.0));
    assert_eq!((union.width, union.height), (4480.0, 1440.0));
}

#[test]
fn overlay_dpr_must_equal_window_scale_factor() {
    // 单个跨屏 overlay 只有一个 WebView devicePixelRatio；本机实测窗口跨两屏时
    // Windows 按重叠面积最大者取 DPI=144 → dpr=1.5。
    let rect = RectPhys {
        x: -1080.0,
        y: -470.0,
        width: 3640.0,
        height: 1920.0,
    };
    let point = RectPhys {
        x: -540.0,
        y: 490.0,
        width: 0.0,
        height: 0.0,
    };
    // 正确：overlay_dpr == 窗口 scale_factor == 1.5 → 无偏移。
    let ok = mixed_dpr_error(&point, &rect, 1.5, 1.5);
    assert!(ok.x.abs() < 1e-9 && ok.y.abs() < 1e-9);

    // 错误 1：窗口 1.5 却把副屏 100% 当全局 dpr → 偏离原点距离 × (1.0/1.5 - 1)。
    let per_monitor = mixed_dpr_error(&point, &rect, 1.5, 1.0);
    assert!(per_monitor.y.abs() > 100.0, "逐屏 dpr 误差应达数百物理像素");

    // 错误 2：窗口 1.0 却下发主屏 1.5 → 反之放大 50%。
    let primary_dpr = mixed_dpr_error(&point, &rect, 1.0, 1.5);
    assert!((primary_dpr.x - 0.5 * 540.0).abs() < 1e-9);
    assert!((primary_dpr.y - 0.5 * 960.0).abs() < 1e-9);
}

#[test]
fn physical_to_css_roundtrip_within_tolerance_for_all_dpr() {
    // 契约：前端 CSS = (物理 - virtualRect 原点) / dpr；反向换算误差 ≤2px。
    let virtual_rect = RectPhys {
        x: -1080.0,
        y: -470.0,
        width: 3640.0,
        height: 1920.0,
    };
    let samples = [
        (-1080.0, -470.0),
        (-540.0, 490.0),
        (-2.0, 100.0),
        (2.0, 100.0),
        (1280.0, 720.0),
        (2559.0, 1439.0),
    ];
    for dpr in [1.0_f64, 1.25, 1.5, 2.0] {
        for (x, y) in samples {
            // 浮点 CSS：往返必须精确。
            let css_x = (x - virtual_rect.x) / dpr;
            let css_y = (y - virtual_rect.y) / dpr;
            let back_x = virtual_rect.x + css_x * dpr;
            let back_y = virtual_rect.y + css_y * dpr;
            assert!((back_x - x).abs() < 1e-9, "dpr={dpr} x={x}");
            assert!((back_y - y).abs() < 1e-9, "dpr={dpr} y={y}");
            // 整数 CSS（鼠标 client 坐标被量化）时误差 ≤ dpr/2 ≤ 1px。
            let quantized_x = virtual_rect.x + css_x.round() * dpr;
            let quantized_y = virtual_rect.y + css_y.round() * dpr;
            assert!((quantized_x - x).abs() <= dpr / 2.0 + 1e-9, "dpr={dpr}");
            assert!((quantized_y - y).abs() <= dpr / 2.0 + 1e-9, "dpr={dpr}");
        }
    }
}

#[test]
fn rect_contains_distinguishes_monitors_and_dead_zones() {
    // 副屏 y 范围 -470..1450，因此 y=1448 仍在副屏可见区内；主屏 y 只到 1440。
    assert!(rect_contains(&secondary(), -540.0, -300.0));
    assert!(rect_contains(&secondary(), -540.0, 1448.0));
    // 外接矩形内、但没有任何显示器覆盖的死区（主屏上方 / 下方，x>0 区域）。
    assert!(!rect_contains(&primary(), 1280.0, -468.0)); // 主屏上方死区
    assert!(!rect_contains(&primary(), 1280.0, 1448.0)); // 主屏下方死区
                                                         // 交界 x=0 属于主屏（左闭右开）。
    assert!(rect_contains(&primary(), 0.0, 720.0));
    assert!(!rect_contains(&secondary(), 0.0, 720.0));
}
