//! 矩形纯逻辑：规范化、面积、命中、交叠、物理 / 逻辑像素换算（契约 §1.5 / §3）。
//!
//! 约定：`RectPhys` 的宽高可能为负（反向拖拽），所有几何判断前先 [`RectPhys::normalize`]。

use crate::contract::types::{RectLogical, RectPhys};

/// 防御非法 DPR（0 / 负数 / NaN），统一回落到 1.0。
fn safe_dpr(dpr: f64) -> f64 {
    if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    }
}

impl RectPhys {
    /// 规范化为左上角原点 + 非负宽高（负宽高的反向拖拽会被翻转）。
    pub fn normalize(&self) -> RectPhys {
        let x = if self.width < 0.0 {
            self.x + self.width
        } else {
            self.x
        };
        let y = if self.height < 0.0 {
            self.y + self.height
        } else {
            self.y
        };
        RectPhys {
            x,
            y,
            width: self.width.abs(),
            height: self.height.abs(),
        }
    }

    /// 面积（物理像素²），始终非负。
    pub fn area(&self) -> f64 {
        let rect = self.normalize();
        rect.width * rect.height
    }

    /// 是否为空选区（面积 < 1px²）。
    pub fn is_empty(&self) -> bool {
        self.area() < 1.0
    }

    /// 点是否落在矩形内（含边界，按物理像素）。
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        let rect = self.normalize();
        x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height
    }

    /// 另一个矩形是否与本矩形相交（交叠面积 > 0）。
    pub fn intersects(&self, other: &RectPhys) -> bool {
        self.intersection(other).is_some()
    }

    /// 求交集；无交集返回 `None`。
    pub fn intersection(&self, other: &RectPhys) -> Option<RectPhys> {
        let a = self.normalize();
        let b = other.normalize();
        let left = a.x.max(b.x);
        let top = a.y.max(b.y);
        let right = (a.x + a.width).min(b.x + b.width);
        let bottom = (a.y + a.height).min(b.y + b.height);
        if right <= left || bottom <= top {
            None
        } else {
            Some(RectPhys {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            })
        }
    }

    /// 由两个对角点构造矩形（允许任意拖拽方向）。
    pub fn from_points(x1: f64, y1: f64, x2: f64, y2: f64) -> RectPhys {
        RectPhys {
            x: x1,
            y: y1,
            width: x2 - x1,
            height: y2 - y1,
        }
        .normalize()
    }

    /// 中心点（规范坐标）。
    pub fn center(&self) -> (f64, f64) {
        let rect = self.normalize();
        (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
    }

    /// 换算为逻辑（CSS）像素矩形。
    pub fn to_logical(&self, dpr: f64) -> RectLogical {
        let rect = self.normalize();
        let dpr = safe_dpr(dpr);
        RectLogical {
            x: rect.x / dpr,
            y: rect.y / dpr,
            width: rect.width / dpr,
            height: rect.height / dpr,
        }
    }

    /// 由逻辑（CSS）像素矩形换算为物理像素矩形。
    pub fn from_logical(logical: &RectLogical, dpr: f64) -> RectPhys {
        let dpr = safe_dpr(dpr);
        RectPhys {
            x: logical.x * dpr,
            y: logical.y * dpr,
            width: logical.width * dpr,
            height: logical.height * dpr,
        }
        .normalize()
    }

    /// 把矩形限制在边界矩形内（用于多屏 / 结果窗定位）。
    pub fn clamp_inside(&self, bounds: &RectPhys) -> RectPhys {
        let rect = self.normalize();
        let bounds = bounds.normalize();
        let width = rect.width.min(bounds.width);
        let height = rect.height.min(bounds.height);
        let x = rect.x.max(bounds.x).min(bounds.x + bounds.width - width);
        let y = rect.y.max(bounds.y).min(bounds.y + bounds.height - height);
        RectPhys {
            x,
            y,
            width,
            height,
        }
    }
}

impl RectLogical {
    /// 换算为物理像素矩形。
    pub fn to_phys(&self, dpr: f64) -> RectPhys {
        RectPhys::from_logical(self, dpr)
    }
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

    #[test]
    fn normalize_flips_reverse_drag() {
        let reversed = rect(120.0, 90.0, -20.0, -30.0);
        assert_eq!(reversed.normalize(), rect(100.0, 60.0, 20.0, 30.0));
        assert_eq!(reversed.area(), 600.0);
    }

    #[test]
    fn negative_virtual_desktop_coordinates_are_supported() {
        // 多显示器场景：副屏位于主屏左侧，原点为负。
        let secondary = rect(-1920.0, 0.0, 1920.0, 1080.0);
        assert!(secondary.contains_point(-100.0, 500.0));
        assert!(!secondary.contains_point(100.0, 500.0));
        assert_eq!(secondary.area(), 1920.0 * 1080.0);

        let reversed = rect(-100.0, -50.0, -200.0, -100.0);
        assert_eq!(reversed.normalize(), rect(-300.0, -150.0, 200.0, 100.0));
    }

    #[test]
    fn intersection_and_hit_test() {
        let a = rect(0.0, 0.0, 100.0, 100.0);
        let b = rect(50.0, 50.0, 100.0, 100.0);
        assert!(a.intersects(&b));
        assert_eq!(a.intersection(&b).unwrap(), rect(50.0, 50.0, 50.0, 50.0));

        let far = rect(500.0, 500.0, 10.0, 10.0);
        assert!(!a.intersects(&far));
        assert!(a.intersection(&far).is_none());

        assert!(a.contains_point(0.0, 0.0));
        assert!(a.contains_point(100.0, 100.0));
        assert!(!a.contains_point(101.0, 50.0));
    }

    #[test]
    fn empty_rect_is_empty() {
        assert!(rect(10.0, 10.0, 0.0, 40.0).is_empty());
        assert!(rect(10.0, 10.0, 0.5, 0.5).is_empty());
        assert!(!rect(10.0, 10.0, 1.0, 1.0).is_empty());
    }

    #[test]
    fn dpr_conversion_roundtrip() {
        let physical = rect(200.0, 100.0, 300.0, 150.0);
        let logical = physical.to_logical(2.0);
        assert_eq!(
            logical,
            RectLogical {
                x: 100.0,
                y: 50.0,
                width: 150.0,
                height: 75.0
            }
        );
        assert_eq!(RectPhys::from_logical(&logical, 2.0), physical);

        // 1.5 倍缩放（混合 DPI）与防御非法 dpr。
        assert_eq!(physical.to_logical(1.5).width, 200.0);
        assert_eq!(physical.to_logical(0.0).width, 300.0);
        assert_eq!(physical.to_logical(f64::NAN).width, 300.0);
    }

    #[test]
    fn clamp_inside_bounds() {
        let bounds = rect(0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(
            rect(1800.0, 1000.0, 300.0, 200.0).clamp_inside(&bounds),
            rect(1620.0, 880.0, 300.0, 200.0)
        );
        assert_eq!(
            rect(-50.0, -50.0, 100.0, 100.0).clamp_inside(&bounds),
            rect(0.0, 0.0, 100.0, 100.0)
        );
    }
}
