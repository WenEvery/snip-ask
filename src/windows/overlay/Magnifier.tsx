/**
 * 放大镜：跟随光标 3× 放大（有冻结帧时用背景图裁切，骨架模式无帧时显示十字与坐标）。
 * 同时显示当前选区物理坐标 / 尺寸。
 */

import type { CSSProperties } from "react";
import type { FrozenFrameMeta, RectPhys } from "../../shared/contract/types";
import { toCssPoint } from "./coords";
import type { Point } from "./coords";

const SIZE = 150;
const OFFSET = 18;
const ZOOM = 3;

export interface MagnifierProps {
  cursorPhys: Point;
  rect: RectPhys | null;
  virtualRect: RectPhys;
  dpr: number;
  frame: FrozenFrameMeta;
}

export function Magnifier({ cursorPhys, rect, virtualRect, dpr, frame }: MagnifierProps) {
  const viewportWidth = typeof window !== "undefined" ? window.innerWidth : 1920;
  const viewportHeight = typeof window !== "undefined" ? window.innerHeight : 1080;
  const cursorCss = toCssPoint(cursorPhys, virtualRect, dpr);

  const placeLeft = cursorCss.x + OFFSET + SIZE > viewportWidth;
  const placeUp = cursorCss.y + OFFSET + SIZE > viewportHeight;
  const left = placeLeft ? cursorCss.x - SIZE - OFFSET : cursorCss.x + OFFSET;
  const top = placeUp ? cursorCss.y - SIZE - OFFSET : cursorCss.y + OFFSET;

  let backgroundStyle: CSSProperties | undefined;
  if (frame.dataUrl) {
    const frameX = cursorPhys.x - virtualRect.x;
    const frameY = cursorPhys.y - virtualRect.y;
    backgroundStyle = {
      backgroundImage: `url(${frame.dataUrl})`,
      backgroundSize: `${frame.width * ZOOM}px ${frame.height * ZOOM}px`,
      backgroundPosition: `${SIZE / 2 - frameX * ZOOM}px ${SIZE / 2 - frameY * ZOOM}px`,
      backgroundRepeat: "no-repeat",
    };
  }

  const readout = rect
    ? `${rect.x}, ${rect.y} · ${Math.round(rect.width)}×${Math.round(rect.height)}`
    : `${cursorPhys.x}, ${cursorPhys.y}`;

  return (
    <div
      className="overlay-magnifier"
      style={{ left, top, width: SIZE, height: SIZE }}
      aria-hidden="true"
    >
      {frame.dataUrl ? (
        <div className="overlay-magnifier__image" style={backgroundStyle} />
      ) : (
        <div className="overlay-magnifier__placeholder">
          <span className="overlay-magnifier__grid" />
          <span className="overlay-magnifier__crosshair" />
          <span className="overlay-magnifier__hint">骨架模式：无冻结帧</span>
        </div>
      )}
      <span className="overlay-magnifier__readout">{readout}</span>
    </div>
  );
}
