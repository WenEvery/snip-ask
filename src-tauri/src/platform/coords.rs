//! 坐标换算纯函数（实施架构 M06 / ADR-0003）。
//!
//! 坐标约定（契约 §1.5）：`rectPhys` 为物理像素、原点为虚拟桌面左上角（可为负）；
//! 前端 CSS 像素 = 物理像素 / `dpr`。前端不得自行换算后回传逻辑坐标。

use crate::contract::types::{RectLogical, RectPhys, VirtualDesktop};

/// 防御非法 DPR。
fn safe_dpr(dpr: f64) -> f64 {
    if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    }
}

/// 逻辑像素 → 物理像素。
pub fn logical_to_physical(value: f64, dpr: f64) -> f64 {
    value * safe_dpr(dpr)
}

/// 物理像素 → 逻辑像素。
pub fn physical_to_logical(value: f64, dpr: f64) -> f64 {
    value / safe_dpr(dpr)
}

/// 物理矩形 → 逻辑矩形（保留负坐标）。
pub fn rect_to_logical(rect: &RectPhys, dpr: f64) -> RectLogical {
    rect.to_logical(dpr)
}

/// 逻辑矩形 → 物理矩形。
pub fn rect_from_logical(rect: &RectLogical, dpr: f64) -> RectPhys {
    RectPhys::from_logical(rect, dpr)
}

/// 由显示器位置 + 物理尺寸 + 缩放构造虚拟桌面信息。
pub fn virtual_desktop(x: f64, y: f64, width: f64, height: f64, dpr: f64) -> VirtualDesktop {
    VirtualDesktop {
        rect: RectPhys {
            x,
            y,
            width,
            height,
        },
        dpr: safe_dpr(dpr),
    }
}

/// 多个物理矩形的最小外接矩形（虚拟桌面口径；S-01 多显示器）。
///
/// - 允许负坐标与不连续排布（副屏在主屏左侧 / 上方）；
/// - 入参宽高按 `normalize` 处理，返回矩形宽高恒非负；
/// - 空迭代返回 `None`（调用方回退到默认分辨率）。
pub fn union_rect(rects: impl IntoIterator<Item = RectPhys>) -> Option<RectPhys> {
    let mut iter = rects.into_iter().map(|rect| rect.normalize());
    let first = iter.next()?;
    let (mut min_x, mut min_y) = (first.x, first.y);
    let (mut max_x, mut max_y) = (first.x + first.width, first.y + first.height);
    for rect in iter {
        min_x = min_x.min(rect.x);
        min_y = min_y.min(rect.y);
        max_x = max_x.max(rect.x + rect.width);
        max_y = max_y.max(rect.y + rect.height);
    }
    Some(RectPhys {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    })
}

/// 物理点是否落在物理矩形内（左闭右开，与 Windows 显示器矩形语义一致）。
pub fn rect_contains(rect: &RectPhys, x: f64, y: f64) -> bool {
    let rect = rect.normalize();
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

/// 逐屏缩放比 → overlay 窗口的单一 dpr 时的换算不变式检查（供单测 / 诊断使用）。
///
/// 单个跨屏 overlay 只有一个 WebView 缩放比 `overlay_dpr`；若错误地把「指针所在屏的 dpr」
/// 逐屏混用，物理坐标会按 `monitor_dpr / overlay_dpr` 的比例整体偏移。
pub fn mixed_dpr_error(
    point_phys: &RectPhys,
    virtual_rect: &RectPhys,
    overlay_dpr: f64,
    monitor_dpr: f64,
) -> RectPhys {
    let overlay = safe_dpr(overlay_dpr);
    let monitor = safe_dpr(monitor_dpr);
    let dx = point_phys.x - virtual_rect.x;
    let dy = point_phys.y - virtual_rect.y;
    RectPhys {
        x: virtual_rect.x + dx / overlay * monitor - point_phys.x,
        y: virtual_rect.y + dy / overlay * monitor - point_phys.y,
        width: 0.0,
        height: 0.0,
    }
}

/// 键盘微调（方向键）：返回平移后的矩形，并限制在虚拟桌面内。
pub fn nudge(rect: &RectPhys, dx: f64, dy: f64, bounds: &RectPhys) -> RectPhys {
    let normalized = rect.normalize();
    let moved = RectPhys {
        x: normalized.x + dx,
        y: normalized.y + dy,
        ..normalized
    };
    moved.clamp_inside(bounds)
}

/// 结果窗跟随选区的候选位置：选区右下 + 8px，越界则翻转到左上。
pub fn follow_position(
    selection: &RectPhys,
    window_width: f64,
    window_height: f64,
    bounds: &RectPhys,
) -> RectPhys {
    let selection = selection.normalize();
    let bounds = bounds.normalize();
    let gap = 8.0;
    let mut x = selection.x + selection.width + gap;
    let mut y = selection.y + selection.height + gap;
    if x + window_width > bounds.x + bounds.width {
        x = selection.x - window_width - gap;
    }
    if y + window_height > bounds.y + bounds.height {
        y = selection.y - window_height - gap;
    }
    RectPhys {
        x,
        y,
        width: window_width,
        height: window_height,
    }
    .clamp_inside(&bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpr_conversions() {
        assert_eq!(logical_to_physical(100.0, 2.0), 200.0);
        assert_eq!(physical_to_logical(300.0, 1.5), 200.0);
        // 非法 dpr 回落到 1.0
        assert_eq!(logical_to_physical(100.0, 0.0), 100.0);
        assert_eq!(physical_to_logical(100.0, f64::NAN), 100.0);
    }

    #[test]
    fn rect_roundtrip() {
        let physical = RectPhys {
            x: -500.0,
            y: 100.0,
            width: 400.0,
            height: 300.0,
        };
        let logical = rect_to_logical(&physical, 2.0);
        assert_eq!(logical.x, -250.0);
        assert_eq!(logical.width, 200.0);
        assert_eq!(rect_from_logical(&logical, 2.0), physical);
    }

    #[test]
    fn nudge_keeps_rect_inside_bounds() {
        let bounds = RectPhys {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let rect = RectPhys {
            x: 10.0,
            y: 10.0,
            width: 100.0,
            height: 100.0,
        };
        assert_eq!(nudge(&rect, 1.0, 0.0, &bounds).x, 11.0);
        // 越界后被夹回
        let clamped = nudge(&rect, -9999.0, 0.0, &bounds);
        assert_eq!(clamped.x, 0.0);
    }

    #[test]
    fn follow_position_flips_near_edges() {
        let bounds = RectPhys {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let selection = RectPhys {
            x: 100.0,
            y: 100.0,
            width: 200.0,
            height: 100.0,
        };
        let position = follow_position(&selection, 460.0, 420.0, &bounds);
        assert_eq!(position.x, 308.0);
        assert_eq!(position.y, 208.0);

        // 靠近右下角时翻转到选区左上。
        let selection = RectPhys {
            x: 1800.0,
            y: 1000.0,
            width: 100.0,
            height: 80.0,
        };
        let position = follow_position(&selection, 460.0, 420.0, &bounds);
        assert!(position.x >= 0.0);
        assert!(position.y >= 0.0);
        assert!(position.x + position.width <= bounds.width + 0.001);
        assert!(position.y + position.height <= bounds.height + 0.001);
    }

    // ───────── S-01 混合 DPI 多屏坐标一致性 ─────────

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RectPhys {
        RectPhys {
            x,
            y,
            width,
            height,
        }
    }

    /// 本机实测布局：主屏 (0,0) 2560x1440@150% + 副屏 (-1080,-470) 1080x1920@100%。
    #[test]
    fn union_rect_mixed_dpi_two_monitors_with_negative_origin() {
        let primary = rect(0.0, 0.0, 2560.0, 1440.0);
        let secondary = rect(-1080.0, -470.0, 1080.0, 1920.0);
        let union = union_rect([primary, secondary]).expect("非空");
        assert_eq!(union, rect(-1080.0, -470.0, 3640.0, 1920.0));
    }

    #[test]
    fn union_rect_supports_negative_and_disjoint_layouts() {
        // 副屏在左且更高：上边界为负、下边界超出主屏。
        let union = union_rect([
            rect(0.0, 0.0, 1920.0, 1080.0),
            rect(-1200.0, -300.0, 1200.0, 1500.0),
        ])
        .expect("非空");
        assert_eq!(union, rect(-1200.0, -300.0, 3120.0, 1500.0));

        // 副屏在上方（行排布）：y 为负、x 不重合。
        let union = union_rect([
            rect(0.0, 0.0, 1920.0, 1080.0),
            rect(300.0, -1200.0, 1920.0, 1200.0),
        ])
        .expect("非空");
        assert_eq!(union, rect(0.0, -1200.0, 2220.0, 2280.0));
    }

    #[test]
    fn union_rect_single_monitor_and_empty() {
        let only = rect(-500.0, -200.0, 1000.0, 800.0);
        assert_eq!(union_rect([only]).expect("非空"), only);
        assert!(union_rect(std::iter::empty::<RectPhys>()).is_none());
    }

    #[test]
    fn union_rect_normalizes_negative_sizes() {
        // 反向拖拽式负宽高不应把外接矩形算反。
        let union = union_rect([rect(0.0, 0.0, 100.0, 100.0), rect(80.0, 80.0, -30.0, -30.0)])
            .expect("非空");
        assert_eq!(union, rect(0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn rect_contains_edges_are_left_closed_right_open() {
        let primary = rect(0.0, 0.0, 2560.0, 1440.0);
        assert!(rect_contains(&primary, 0.0, 0.0));
        assert!(rect_contains(&primary, 2559.0, 1439.0));
        // 右/下边界不属于本屏（属于相邻屏或死区）
        assert!(!rect_contains(&primary, 2560.0, 1439.0));
        assert!(!rect_contains(&primary, -1.0, 100.0));
        // 负坐标副屏
        let secondary = rect(-1080.0, -470.0, 1080.0, 1920.0);
        assert!(rect_contains(&secondary, -1080.0, -470.0));
        assert!(!rect_contains(&secondary, 0.0, 0.0)); // 右边界开区间
    }

    /// 换算不变式：物理点 → CSS（overlay 单一 dpr）→ 物理，误差只来自 CSS 取整，且 ≤1 device px。
    #[test]
    fn to_phys_roundtrip_is_stable_for_mixed_dpr() {
        let virtual_rect = rect(-1080.0, -470.0, 3640.0, 1920.0);
        let points = [
            (-1040.0, -430.0), // 副屏左上
            (-540.0, 490.0),   // 副屏中心
            (0.0, 720.0),      // 两屏交界
            (1280.0, 720.0),   // 主屏中心
            (2520.0, 1400.0),  // 主屏右下
            (100.0, 100.0),    // 主屏偏左上
        ];
        for dpr in [1.0_f64, 1.25, 1.5, 2.0] {
            for (x, y) in points {
                let css_x = (x - virtual_rect.x) / dpr;
                let css_y = (y - virtual_rect.y) / dpr;
                // 指针 client 坐标被 Chromium 按 device px 量化：CSS 往返偏差 ≤ 1 device px。
                let phys_x = virtual_rect.x + css_x * dpr;
                let phys_y = virtual_rect.y + css_y * dpr;
                assert!((phys_x - x).abs() < 1e-9, "dpr={dpr} x={x}");
                assert!((phys_y - y).abs() < 1e-9, "dpr={dpr} y={y}");
                // 整数 CSS（鼠标真实取值）时，落点误差 ≤ dpr/2 device px，恒 ≤2px（dpr≤2）。
                let css_int_x = css_x.round();
                let css_int_y = css_y.round();
                let err_x = (virtual_rect.x + css_int_x * dpr - x).abs();
                let err_y = (virtual_rect.y + css_int_y * dpr - y).abs();
                assert!(err_x <= 2.0, "dpr={dpr} err_x={err_x}");
                assert!(err_y <= 2.0, "dpr={dpr} err_y={err_y}");
            }
        }
    }

    /// 单个 overlay 必须用统一 dpr：逐屏混用 dpr 会把副屏坐标整体放大（本机实测可达数百 px）。
    #[test]
    fn mixed_dpr_error_grows_away_from_anchor_monitor() {
        let virtual_rect = rect(-1080.0, -470.0, 3640.0, 1920.0);
        let secondary_center = rect(-540.0, 490.0, 0.0, 0.0);
        // overlay_dpr=1.0（窗口在 100% 屏）、错误地用副屏 100% dpr → 无误差。
        let same = mixed_dpr_error(&secondary_center, &virtual_rect, 1.0, 1.0);
        assert!(same.x.abs() < 1e-9 && same.y.abs() < 1e-9);
        // 若把主屏 150% 当全局 dpr：偏离虚拟桌面原点的距离被放大 50%。
        let wrong = mixed_dpr_error(&secondary_center, &virtual_rect, 1.0, 1.5);
        assert!((wrong.x - 0.5 * (-540.0 - -1080.0)).abs() < 1e-9);
        assert!((wrong.y - 0.5 * (490.0 - -470.0)).abs() < 1e-9);
        assert!(wrong.y.abs() > 100.0);
    }
}
