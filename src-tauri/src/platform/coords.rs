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
}
