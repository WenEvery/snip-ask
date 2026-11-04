/**
 * overlay 窗口：冻结层 + 选框 + 放大镜 + 动作菜单（契约 §2 / §5 / §7、实施架构 M06–M07）。
 *
 * 启动流程：
 * - 真实 Tauri：等待后端（热键/托盘）显示窗口并发来 `overlay.ready`；
 * - 浏览器 mock：800ms 内未收到 `overlay.ready` 则自动 `capture_freeze`（便于手动验收）。
 *
 * 交互：拖拽框选 → `selection_commit` → 等 `menu.ready` → 选择动作 → `menu_execute`。
 * ESC 取消、Enter 确认、方向键微调（Shift 10 物理像素）。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
import { Button, Spinner } from "../../components/ui";
import { COMMANDS, EVENTS } from "../../shared/contract/names";
import type {
  ActionItem,
  AppError,
  ConfigView,
  FrozenFrameMeta,
  MenuReadyPayload,
  Mode,
  OcrStatusPayload,
  OverlayReadyPayload,
  RectPhys,
  SessionId,
} from "../../shared/contract/types";
import { useIpcEvent } from "../../shared/hooks/useIpcEvent";
import { ipc, isTauri, toAppError } from "../../shared/ipc";
import { ActionMenu } from "./ActionMenu";
import { Magnifier } from "./Magnifier";
import { SelectionLayer } from "./SelectionLayer";
import { toPhys } from "./coords";
import type { Point } from "./coords";
import "./overlay.css";

type OverlayPhase = "waiting" | "selecting" | "committing" | "menu" | "executing";
type OcrState = "idle" | "scanning" | "done" | "failed";

interface FrozenState {
  session: SessionId;
  virtualRect: RectPhys;
  dpr: number;
  frame: FrozenFrameMeta;
}

export default function OverlayWindow() {
  const [frozen, setFrozen] = useState<FrozenState | null>(null);
  const [phase, setPhase] = useState<OverlayPhase>("waiting");
  const [rect, setRect] = useState<RectPhys | null>(null);
  const [cursor, setCursor] = useState<Point | null>(null);
  const [ocrState, setOcrState] = useState<OcrState>("idle");
  const [menu, setMenu] = useState<MenuReadyPayload | null>(null);
  const [mode, setMode] = useState<Mode>("quick");
  const [error, setError] = useState<AppError | null>(null);

  const frozenRef = useRef<FrozenState | null>(null);
  const phaseRef = useRef<OverlayPhase>("waiting");
  const rectRef = useRef<RectPhys | null>(null);

  const setFrozenSync = useCallback((next: FrozenState | null) => {
    frozenRef.current = next;
    setFrozen(next);
  }, []);
  const setPhaseSync = useCallback((next: OverlayPhase) => {
    phaseRef.current = next;
    setPhase(next);
  }, []);
  const setRectSync = useCallback((next: RectPhys | null) => {
    rectRef.current = next;
    setRect(next);
  }, []);

  const handleError = useCallback((raw: unknown) => {
    setError(toAppError(raw));
  }, []);

  /* ───────── 事件订阅 ───────── */

  useIpcEvent<OverlayReadyPayload>(EVENTS.overlayReady, (payload) => {
    setFrozenSync({
      session: payload.session,
      virtualRect: payload.virtualRect,
      dpr: payload.dpr,
      frame: payload.frame,
    });
    setRectSync(null);
    setMenu(null);
    setOcrState("idle");
    setError(null);
    setCursor(null);
    setPhaseSync("selecting");
    // 每次新会话都回到配置默认模式（快/深）
    void ipc
      .invoke<ConfigView>(COMMANDS.configGet)
      .then((config) => setMode(config.actions.defaultMode))
      .catch(() => undefined);
  });

  useIpcEvent<OcrStatusPayload>(EVENTS.ocrStatus, (payload) => {
    const current = frozenRef.current;
    if (!current || payload.session !== current.session) return;
    setOcrState(payload.state);
  });

  useIpcEvent<MenuReadyPayload>(EVENTS.menuReady, (payload) => {
    const current = frozenRef.current;
    if (!current || payload.session !== current.session) return;
    setMenu(payload);
    if (phaseRef.current === "committing") setPhaseSync("menu");
  });

  /* ───────── 启动：浏览器 mock 自动触发一次框选 ───────── */

  useEffect(() => {
    if (isTauri()) return undefined;
    const timer = window.setTimeout(() => {
      if (frozenRef.current) return;
      void ipc.invoke<SessionId>(COMMANDS.captureFreeze).catch(handleError);
    }, 800);
    return () => window.clearTimeout(timer);
  }, [handleError]);

  /* ───────── 动作 ───────── */

  const triggerCapture = useCallback(() => {
    setError(null);
    void ipc.invoke<SessionId>(COMMANDS.captureFreeze).catch(handleError);
  }, [handleError]);

  const cancelCapture = useCallback(() => {
    const current = frozenRef.current;
    if (current) {
      void ipc.invoke(COMMANDS.captureCancel, { session: current.session }).catch(() => undefined);
    }
    setFrozenSync(null);
    setRectSync(null);
    setMenu(null);
    setOcrState("idle");
    setPhaseSync("waiting");
  }, [setFrozenSync, setPhaseSync, setRectSync]);

  const commitSelection = useCallback(
    (next: RectPhys) => {
      const current = frozenRef.current;
      if (!current) return;
      setPhaseSync("committing");
      setOcrState("scanning");
      void ipc
        .invoke(COMMANDS.selectionCommit, { session: current.session, rectPhys: next })
        .catch((raw) => {
          handleError(raw);
          setPhaseSync("selecting");
        });
    },
    [handleError, setPhaseSync],
  );

  const resetSelection = useCallback(() => {
    const current = frozenRef.current;
    if (current) {
      void ipc.invoke(COMMANDS.selectionReset, { session: current.session }).catch(handleError);
    }
    setRectSync(null);
    setMenu(null);
    setOcrState("idle");
    setPhaseSync("selecting");
  }, [handleError, setPhaseSync, setRectSync]);

  const executeAction = useCallback(
    (item: ActionItem, selectedMode: Mode) => {
      const current = frozenRef.current;
      if (!current) return;
      setPhaseSync("executing");
      void ipc
        .invoke(COMMANDS.menuExecute, {
          session: current.session,
          action: item.id,
          mode: selectedMode,
        })
        .then(() => {
          void ipc.invoke(COMMANDS.windowHide, { window: "overlay" }).catch(() => undefined);
          if (!isTauri()) {
            // 浏览器预览没有真实窗口隐藏：回到等待态，方便重复演示
            setFrozenSync(null);
            setRectSync(null);
            setOcrState("idle");
            setPhaseSync("waiting");
          }
        })
        .catch((raw) => {
          handleError(raw);
          setPhaseSync("menu");
        });
    },
    [handleError, setFrozenSync, setPhaseSync, setRectSync],
  );

  const toggleMode = useCallback(() => {
    setMode((current) => (current === "quick" ? "deep" : "quick"));
  }, []);

  /* ───────── 键盘：ESC 取消 / Enter 确认 / 方向键微调 ───────── */

  useEffect(() => {
    if (!frozen) return undefined;
    const handleKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancelCapture();
        return;
      }
      if (phaseRef.current !== "selecting") return;
      const current = rectRef.current;
      if (!current) return;
      if (event.key === "Enter") {
        event.preventDefault();
        commitSelection(current);
        return;
      }
      const step = event.shiftKey ? 10 : 1;
      let dx = 0;
      let dy = 0;
      switch (event.key) {
        case "ArrowLeft":
          dx = -step;
          break;
        case "ArrowRight":
          dx = step;
          break;
        case "ArrowUp":
          dy = -step;
          break;
        case "ArrowDown":
          dy = step;
          break;
        default:
          return;
      }
      event.preventDefault();
      setRectSync({ ...current, x: current.x + dx, y: current.y + dy });
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [frozen, cancelCapture, commitSelection, setRectSync]);

  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>): void => {
    const current = frozenRef.current;
    if (!current) return;
    setCursor(toPhys(event.clientX, event.clientY, current.virtualRect, current.dpr));
  };

  /* ───────── 渲染 ───────── */

  return (
    <div
      className={`overlay-root${frozen ? "" : " overlay-root--waiting"}`}
      onPointerMove={handlePointerMove}
      onPointerLeave={() => setCursor(null)}
    >
      {frozen?.frame.dataUrl ? (
        <img className="overlay-frame" src={frozen.frame.dataUrl} alt="" draggable={false} />
      ) : (
        <div className="overlay-placeholder" aria-hidden="true">
          <div className="overlay-placeholder__grid" />
        </div>
      )}

      {frozen ? (
        <SelectionLayer
          virtualRect={frozen.virtualRect}
          dpr={frozen.dpr}
          rect={rect}
          disabled={phase !== "selecting"}
          onRectChange={setRectSync}
          onCommit={commitSelection}
        />
      ) : null}

      {frozen && cursor && phase === "selecting" ? (
        <Magnifier
          cursorPhys={cursor}
          rect={rect}
          virtualRect={frozen.virtualRect}
          dpr={frozen.dpr}
          frame={frozen.frame}
        />
      ) : null}

      {frozen ? (
        <div className="overlay-status" role="status" aria-live="polite">
          {ocrState === "scanning" ? (
            <span className="overlay-status__pill">
              <Spinner size="sm" />
              识别中…
            </span>
          ) : null}
          {phase === "committing" ? (
            <span className="overlay-status__pill">
              <Spinner size="sm" />
              正在识别选区…
            </span>
          ) : null}
          {phase === "executing" ? (
            <span className="overlay-status__pill">
              <Spinner size="sm" />
              正在执行动作…
            </span>
          ) : null}
          {phase === "selecting" && !rect && ocrState !== "scanning" ? (
            <span className="overlay-status__hint">
              拖拽选择区域 · Esc 取消 · Enter 确认 · 方向键微调 1px（Shift 10px）
            </span>
          ) : null}
        </div>
      ) : null}

      {frozen && rect && menu && phase === "menu" ? (
        <ActionMenu
          menu={menu}
          anchorRect={rect}
          virtualRect={frozen.virtualRect}
          dpr={frozen.dpr}
          mode={mode}
          onToggleMode={toggleMode}
          onAction={(item) => executeAction(item, mode)}
          onReset={resetSelection}
          onCancel={cancelCapture}
        />
      ) : null}

      {!frozen ? (
        <div className="overlay-waiting">
          <div className="overlay-waiting__card sa-anim-slide">
            <h1 className="overlay-waiting__title">框选 AI</h1>
            <p className="overlay-waiting__desc">
              {isTauri()
                ? "等待后端触发框选（全局热键或托盘菜单）…"
                : "浏览器预览（mock 后端）：点击下方按钮模拟一次框选。"}
            </p>
            {isTauri() ? (
              <Spinner size="md" label="等待冻结帧…" />
            ) : (
              <Button variant="primary" onClick={triggerCapture}>
                模拟触发框选
              </Button>
            )}
            <p className="overlay-waiting__hint">骨架模式 · 无真实捕获 / OCR / 网络请求</p>
          </div>
        </div>
      ) : null}

      {error ? (
        <div className="overlay-error" role="alert">
          <span className="overlay-error__code">{error.code}</span>
          <span className="overlay-error__message">{error.message}</span>
          <Button size="sm" variant="ghost" onClick={() => setError(null)}>
            关闭
          </Button>
        </div>
      ) : null}
    </div>
  );
}
