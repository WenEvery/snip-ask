/**
 * 动作菜单：锚定选区右下角，自动翻转避免出屏。
 * 内容：识别预览、动作按钮（禁用时展示 disabledReason）、快/深切换、重新框选、取消。
 */

import { useMemo } from "react";
import { Button, Kbd } from "../../components/ui";
import type { ActionItem, MenuReadyPayload, Mode, RectPhys } from "../../shared/contract/types";
import { toCssRect } from "./coords";

const MENU_WIDTH = 300;
const MENU_ESTIMATED_HEIGHT = 330;
const GAP = 8;

export interface ActionMenuProps {
  menu: MenuReadyPayload;
  anchorRect: RectPhys;
  virtualRect: RectPhys;
  dpr: number;
  mode: Mode;
  disabled?: boolean;
  onToggleMode: () => void;
  onAction: (item: ActionItem) => void;
  onReset: () => void;
  onCancel: () => void;
}

export function ActionMenu({
  menu,
  anchorRect,
  virtualRect,
  dpr,
  mode,
  disabled = false,
  onToggleMode,
  onAction,
  onReset,
  onCancel,
}: ActionMenuProps) {
  const position = useMemo(() => {
    const css = toCssRect(anchorRect, virtualRect, dpr);
    const viewportWidth = typeof window === "undefined" ? 1920 : window.innerWidth;
    const viewportHeight = typeof window === "undefined" ? 1080 : window.innerHeight;
    const rightEdge = css.left + css.width;
    const placeRight = rightEdge + GAP + MENU_WIDTH <= viewportWidth;
    const preferredLeft = placeRight ? rightEdge + GAP : css.left - MENU_WIDTH - GAP;
    const maxLeft = Math.max(GAP, viewportWidth - MENU_WIDTH - GAP);
    const left = Math.min(Math.max(GAP, preferredLeft), maxLeft);
    const preferredTop = css.top + css.height - 8;
    const maxTop = Math.max(GAP, viewportHeight - MENU_ESTIMATED_HEIGHT - GAP);
    const top = Math.min(Math.max(GAP, preferredTop), maxTop);
    return { left, top };
  }, [anchorRect, virtualRect, dpr]);

  return (
    <div
      className="overlay-menu sa-anim-slide"
      style={{ left: position.left, top: position.top, width: MENU_WIDTH }}
      role="dialog"
      aria-label="动作菜单"
    >
      <div className="overlay-menu__header">
        <span className="overlay-menu__title">识别结果</span>
        <div className="overlay-menu__mode" role="group" aria-label="回答模式">
          <button
            type="button"
            className={`overlay-menu__mode-btn${mode === "quick" ? " overlay-menu__mode-btn--active" : ""}`}
            aria-pressed={mode === "quick"}
            onClick={onToggleMode}
          >
            快速
          </button>
          <button
            type="button"
            className={`overlay-menu__mode-btn${mode === "deep" ? " overlay-menu__mode-btn--active" : ""}`}
            aria-pressed={mode === "deep"}
            onClick={onToggleMode}
          >
            深度
          </button>
        </div>
      </div>

      <p className="overlay-menu__preview" title={menu.preview}>
        {menu.preview}
      </p>

      <div className="overlay-menu__actions" role="group" aria-label="动作">
        {menu.items.map((item) => {
          const unavailable = disabled || !item.enabled;
          const reason = item.disabledReason ?? "当前不可用";
          const title = item.enabled ? item.label : `${item.label}：${reason}`;
          return (
            <button
              key={item.id}
              type="button"
              className="overlay-menu__action"
              disabled={unavailable}
              title={title}
              aria-describedby={unavailable ? `overlay-action-reason-${item.id}` : undefined}
              onClick={() => onAction(item)}
            >
              <span
                style={{
                  display: "flex",
                  flexDirection: "column",
                  alignItems: "flex-start",
                  gap: 2,
                  minWidth: 0,
                  flex: "1 1 auto",
                }}
              >
                <span className="overlay-menu__action-label">{item.label}</span>
                {unavailable ? (
                  // 受限模式：禁用原因直接可见（不只放在 tooltip），文案为可操作路径。
                  <span
                    id={`overlay-action-reason-${item.id}`}
                    style={{
                      display: "block",
                      fontSize: 11,
                      lineHeight: 1.35,
                      opacity: 0.8,
                      whiteSpace: "normal",
                      textAlign: "left",
                    }}
                  >
                    {reason}
                  </span>
                ) : null}
              </span>
              <span className="overlay-menu__action-meta">
                {item.shortcut ? <Kbd>{item.shortcut}</Kbd> : null}
                <span
                  className={`overlay-menu__action-mode overlay-menu__action-mode--${item.mode}`}
                >
                  {item.mode === "deep" ? "深度" : "快速"}
                </span>
              </span>
            </button>
          );
        })}
      </div>

      <div className="overlay-menu__footer">
        <Button size="sm" variant="ghost" onClick={onReset} disabled={disabled}>
          重新框选
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel} disabled={disabled}>
          取消
        </Button>
      </div>
    </div>
  );
}
