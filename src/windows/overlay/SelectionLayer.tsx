/**
 * 选框层：拖拽创建选区（支持反向拖拽归一化）、选区外 4 边遮罩、
 * 1px accent 边框 + 四角手柄 + 尺寸标签。
 * 所有回传坐标均为物理像素。
 */

import { useRef } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
import type { RectPhys } from "../../shared/contract/types";
import { normalizeRect, toCssRect, toPhys } from "./coords";
import type { Point } from "./coords";

export interface SelectionLayerProps {
  virtualRect: RectPhys;
  dpr: number;
  rect: RectPhys | null;
  disabled: boolean;
  onRectChange: (rect: RectPhys | null) => void;
  onCommit: (rect: RectPhys) => void;
}

interface DragState {
  start: Point;
  pointerId: number;
}

export function SelectionLayer({
  virtualRect,
  dpr,
  rect,
  disabled,
  onRectChange,
  onCommit,
}: SelectionLayerProps) {
  const dragRef = useRef<DragState | null>(null);

  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>): void => {
    if (disabled || event.button !== 0) return;
    event.preventDefault();
    const start = toPhys(event.clientX, event.clientY, virtualRect, dpr);
    dragRef.current = { start, pointerId: event.pointerId };
    event.currentTarget.setPointerCapture(event.pointerId);
    onRectChange({ x: start.x, y: start.y, width: 0, height: 0 });
  };

  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>): void => {
    const drag = dragRef.current;
    if (!drag) return;
    const current = toPhys(event.clientX, event.clientY, virtualRect, dpr);
    onRectChange(normalizeRect(drag.start, current));
  };

  const handlePointerUp = (event: ReactPointerEvent<HTMLDivElement>): void => {
    const drag = dragRef.current;
    if (!drag) return;
    dragRef.current = null;
    if (event.currentTarget.hasPointerCapture(drag.pointerId)) {
      event.currentTarget.releasePointerCapture(drag.pointerId);
    }
    const end = toPhys(event.clientX, event.clientY, virtualRect, dpr);
    const next = normalizeRect(drag.start, end);
    // 契约：宽高均 ≥ 3 物理像素才视为有效选区
    if (next.width >= 3 && next.height >= 3) onCommit(next);
    else onRectChange(null);
  };

  const handlePointerCancel = (): void => {
    dragRef.current = null;
    onRectChange(null);
  };

  const css = rect ? toCssRect(rect, virtualRect, dpr) : null;
  const sizeBelow =
    css !== null && typeof window !== "undefined" && css.top + css.height + 30 < window.innerHeight;

  return (
    <div
      className={`overlay-selection${disabled ? " overlay-selection--disabled" : ""}`}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerCancel={handlePointerCancel}
    >
      {css && rect ? (
        <>
          <div
            className="overlay-selection__mask"
            style={{ left: css.left, top: css.top, width: css.width, height: css.height }}
            aria-hidden="true"
          />
          <div
            className="overlay-selection__box"
            style={{ left: css.left, top: css.top, width: css.width, height: css.height }}
          >
            <span className="overlay-selection__corner overlay-selection__corner--tl" />
            <span className="overlay-selection__corner overlay-selection__corner--tr" />
            <span className="overlay-selection__corner overlay-selection__corner--bl" />
            <span className="overlay-selection__corner overlay-selection__corner--br" />
            <span
              className={`overlay-selection__size${
                sizeBelow ? " overlay-selection__size--below" : " overlay-selection__size--above"
              }`}
            >
              宽 {Math.round(rect.width)} × 高 {Math.round(rect.height)} px（物理）
            </span>
          </div>
        </>
      ) : null}
    </div>
  );
}
