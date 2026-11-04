/**
 * overlay 坐标换算：契约要求**全程物理像素**（以虚拟桌面左上角为原点，可为负）。
 * 窗口内 CSS 像素 = 物理像素 / dpr；overlay 覆盖区域原点是 virtualRect 的左上角。
 */

import type { RectPhys } from "../../shared/contract/types";

export interface Point {
  x: number;
  y: number;
}

/** 鼠标 client 坐标（相对 overlay 窗口）→ 虚拟桌面物理像素 */
export function toPhys(
  clientX: number,
  clientY: number,
  virtualRect: RectPhys,
  dpr: number,
): Point {
  return {
    x: Math.round(virtualRect.x + clientX * dpr),
    y: Math.round(virtualRect.y + clientY * dpr),
  };
}

/** 物理矩形 → overlay 窗口内 CSS 矩形（用于绝对定位） */
export function toCssRect(
  rectPhys: RectPhys,
  virtualRect: RectPhys,
  dpr: number,
): { left: number; top: number; width: number; height: number } {
  return {
    left: (rectPhys.x - virtualRect.x) / dpr,
    top: (rectPhys.y - virtualRect.y) / dpr,
    width: rectPhys.width / dpr,
    height: rectPhys.height / dpr,
  };
}

/** 物理点 → overlay 窗口内 CSS 点 */
export function toCssPoint(pointPhys: Point, virtualRect: RectPhys, dpr: number): Point {
  return {
    x: (pointPhys.x - virtualRect.x) / dpr,
    y: (pointPhys.y - virtualRect.y) / dpr,
  };
}

/** 反向拖拽归一化：返回非负宽高的矩形 */
export function normalizeRect(start: Point, end: Point): RectPhys {
  return {
    x: Math.min(start.x, end.x),
    y: Math.min(start.y, end.y),
    width: Math.abs(end.x - start.x),
    height: Math.abs(end.y - start.y),
  };
}
