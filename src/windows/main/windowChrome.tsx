/**
 * main 窗口的自绘窗口控件（v1.0.1：去掉系统标题栏后由前端实现）。
 *
 * - 最小化：最小化到任务栏（`minimize`）；
 * - 最大化 / 还原：按钮 + 双击标题栏（`toggleMaximize`）；
 * - 关闭：隐藏到托盘（沿用 `window_hide`，不退出应用）；
 * - 八向缩放手柄：`startResizeDragging(direction)`，最大化时隐藏；
 * - 浏览器预览（非 Tauri）下所有窗口 API 均为 no-op，不抛错，便于 UI 预览。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "../../shared/ipc";

/** 与 `@tauri-apps/api/window` 的 `ResizeDirection` 一致（该类型未导出，这里本地声明）。 */
type ResizeDirection =
  "East" | "North" | "NorthEast" | "NorthWest" | "South" | "SouthEast" | "SouthWest" | "West";

export interface WindowControls {
  /** 当前是否最大化（浏览器预览恒为 false）。 */
  maximized: boolean;
  /** 最小化到任务栏。 */
  minimize: () => void;
  /** 最大化 ↔ 还原。 */
  toggleMaximize: () => void;
  /**
   * 双击标题栏时调用：Windows 在拖拽区双击时通常已由系统完成最大化/还原，
   * 这里用「最近一次窗口尺寸变化」判断，避免再切换一次导致没有变化。
   */
  toggleMaximizeFromDoubleClick: () => void;
}

/** 封装窗口操作与最大化状态订阅。 */
export function useWindowControls(): WindowControls {
  const [maximized, setMaximized] = useState(false);
  const lastResizeRef = useRef(0);

  useEffect(() => {
    if (!isTauri()) return undefined;
    const win = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | undefined;

    const sync = () => {
      lastResizeRef.current = Date.now();
      void win
        .isMaximized()
        .then((value) => {
          if (!disposed) setMaximized(value);
        })
        .catch(() => undefined);
    };

    sync();
    void win
      .onResized(sync)
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => undefined);

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const minimize = useCallback(() => {
    if (!isTauri()) return;
    void getCurrentWindow()
      .minimize()
      .catch(() => undefined);
  }, []);

  const toggleMaximize = useCallback(() => {
    if (!isTauri()) return;
    const win = getCurrentWindow();
    void win
      .toggleMaximize()
      .then(() => syncMaximized(win, setMaximized))
      .catch(() => undefined);
  }, []);

  const toggleMaximizeFromDoubleClick = useCallback(() => {
    if (!isTauri()) return;
    // 双击发生在拖拽区内时，Windows 会原生执行最大化/还原；
    // 若刚刚发生过窗口尺寸变化，说明系统已经处理，不再重复切换。
    if (Date.now() - lastResizeRef.current < 400) return;
    toggleMaximize();
  }, [toggleMaximize]);

  return { maximized, minimize, toggleMaximize, toggleMaximizeFromDoubleClick };
}

function syncMaximized(
  win: ReturnType<typeof getCurrentWindow>,
  setMaximized: (value: boolean) => void,
): Promise<void> {
  return win
    .isMaximized()
    .then((value) => setMaximized(value))
    .catch(() => undefined);
}

const RESIZE_HANDLES: { direction: ResizeDirection; className: string }[] = [
  { direction: "North", className: "n" },
  { direction: "South", className: "s" },
  { direction: "East", className: "e" },
  { direction: "West", className: "w" },
  { direction: "NorthEast", className: "ne" },
  { direction: "NorthWest", className: "nw" },
  { direction: "SouthEast", className: "se" },
  { direction: "SouthWest", className: "sw" },
];

export interface WindowResizeHandlesProps {
  /** 最大化时隐藏手柄。 */
  visible: boolean;
}

/** 八向缩放手柄（覆盖在窗口四边/四角）。 */
export function WindowResizeHandles({ visible }: WindowResizeHandlesProps) {
  if (!visible) return null;
  return (
    <div className="window-resize" aria-hidden="true">
      {RESIZE_HANDLES.map((handle) => (
        <div
          key={handle.direction}
          className={`window-resize__handle window-resize__handle--${handle.className}`}
          onPointerDown={(event) => {
            if (!isTauri()) return;
            event.preventDefault();
            void getCurrentWindow()
              .startResizeDragging(handle.direction)
              .catch(() => undefined);
          }}
        />
      ))}
    </div>
  );
}
