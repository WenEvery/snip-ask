/**
 * overlay 窗口：冻结层 + 选框 + 放大镜 + 动作菜单（契约 §2 / §5 / §7、实施架构 M06–M07）。
 *
 * 启动流程：
 * - 真实 Tauri：等待后端（热键/托盘）显示窗口并发来 `overlay:ready`；
 * - 浏览器 mock：800ms 内未收到 `overlay:ready` 则自动 `capture_freeze`（便于手动验收）。
 *
 * 交互：拖拽框选 → `selection_commit` → 等 `menu:ready` → 选择动作 → `menu_execute`。
 * 覆盖翻译（契约 v4 / 研究 §7.2）：菜单「覆盖原句」→ 前端子阶段 `translating → covered`
 * （**仅前端**；后端会话保持 `showingResult` 语义不变，因为 `translate_cover` 不推进会话阶段），
 * 收到 `translate:progress` / `translate:ready` 后由 `TranslationOverlay` 绘制覆盖层；
 * ESC 第一次回原文、第二次退出覆盖；O/B/C 或工具条切换 原文/双语/覆盖。
 *
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
  CoverResult,
  CoveredBlock,
  FrozenFrameMeta,
  MenuReadyPayload,
  Mode,
  OcrBox,
  OcrStatusPayload,
  OverlayReadyPayload,
  RectPhys,
  SessionId,
  TranslateProgressPayload,
  TranslateReadyPayload,
} from "../../shared/contract/types";
import { useIpcEvent } from "../../shared/hooks/useIpcEvent";
import { ipc, isTauri, toAppError } from "../../shared/ipc";
import { ActionMenu } from "./ActionMenu";
import { Magnifier } from "./Magnifier";
import { SelectionLayer } from "./SelectionLayer";
import { TranslationOverlay } from "./TranslationOverlay";
import type { CoverPhase, CoverView } from "./TranslationOverlay";
import { toPhys } from "./coords";
import type { Point } from "./coords";
import { mergeLines } from "./translationLayout";
import "./overlay.css";

type OverlayPhase =
  "waiting" | "selecting" | "committing" | "menu" | "translating" | "covered" | "executing";
type OcrState = "idle" | "scanning" | "done" | "failed";

interface FrozenState {
  session: SessionId;
  virtualRect: RectPhys;
  dpr: number;
  frame: FrozenFrameMeta;
}

/** 覆盖翻译的前端状态（后端会话阶段不变）。 */
interface CoverState {
  session: SessionId;
  /** 覆盖层阶段：翻译中 / 已覆盖 / 全部失败（保留原文 + 红框）。 */
  phase: CoverPhase;
  view: CoverView;
  results: Record<string, CoveredBlock>;
  progress: { done: number; total: number } | null;
  engine: string | null;
  blockCount: number;
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
  const [boxes, setBoxes] = useState<OcrBox[]>([]);
  const [cover, setCover] = useState<CoverState | null>(null);

  const frozenRef = useRef<FrozenState | null>(null);
  const phaseRef = useRef<OverlayPhase>("waiting");
  const rectRef = useRef<RectPhys | null>(null);
  const boxesRef = useRef<OcrBox[]>([]);
  const coverRef = useRef<CoverState | null>(null);

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
  const setBoxesSync = useCallback((next: OcrBox[]) => {
    boxesRef.current = next;
    setBoxes(next);
  }, []);
  const setCoverSync = useCallback((next: CoverState | null) => {
    coverRef.current = next;
    setCover(next);
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
    setBoxesSync([]);
    setCoverSync(null);
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
    if (Array.isArray(payload.boxes) && payload.boxes.length > 0) {
      setBoxesSync(payload.boxes);
    }
  });

  useIpcEvent<MenuReadyPayload>(EVENTS.menuReady, (payload) => {
    const current = frozenRef.current;
    if (!current || payload.session !== current.session) return;
    setMenu(payload);
    if (phaseRef.current === "committing") setPhaseSync("menu");
  });

  useIpcEvent<TranslateProgressPayload>(EVENTS.translateProgress, (payload) => {
    const active = coverRef.current;
    const current = frozenRef.current;
    if (!active || !current || active.session !== payload.session) return;
    setCoverSync({
      ...active,
      progress: { done: payload.done, total: payload.total },
    });
  });

  useIpcEvent<TranslateReadyPayload>(EVENTS.translateReady, (payload) => {
    const active = coverRef.current;
    const current = frozenRef.current;
    if (!active || !current || active.session !== payload.session) return;
    const results: Record<string, CoveredBlock> = {};
    for (const block of payload.blocks) results[block.id] = block;
    const allFailed = payload.blocks.length > 0 && payload.blocks.every((block) => block.failed);
    setCoverSync({
      ...active,
      phase: allFailed ? "failed" : "covered",
      results,
      engine: payload.engine,
      progress: { done: payload.blocks.length, total: payload.blocks.length },
    });
    if (phaseRef.current === "translating") setPhaseSync("covered");
  });

  /* ───────── 启动：确保拿到当前冻结会话 ───────── */

  useEffect(() => {
    if (isTauri()) {
      // 真实模式：overlay 窗口在 `capture_freeze` 中惰性创建，"广播 overlay:ready" 往往早于
      // 本页面 WebView 完成加载，冷启动会丢事件。`capture_freeze` 是幂等的（已有会话直接复用
      // 并无副作用地补发 overlay:ready），因此挂载后按梯度主动拉取，直到收到冻结帧为止。
      const timers = [150, 500, 1200].map((delay) =>
        window.setTimeout(() => {
          if (frozenRef.current || phaseRef.current !== "waiting") return;
          void ipc.invoke<SessionId>(COMMANDS.captureFreeze).catch(() => undefined);
        }, delay),
      );
      return () => timers.forEach((timer) => window.clearTimeout(timer));
    }
    // 浏览器 mock：800ms 内未收到 overlay:ready 则自动触发一次框选
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
    setBoxesSync([]);
    setCoverSync(null);
    setMenu(null);
    setOcrState("idle");
    setPhaseSync("waiting");
  }, [setBoxesSync, setCoverSync, setFrozenSync, setPhaseSync, setRectSync]);

  const commitSelection = useCallback(
    (next: RectPhys) => {
      const current = frozenRef.current;
      if (!current) return;
      setPhaseSync("committing");
      setOcrState("scanning");
      void ipc
        .invoke<{ boxes?: OcrBox[] }>(COMMANDS.selectionCommit, {
          session: current.session,
          rectPhys: next,
        })
        .then((result) => {
          if (Array.isArray(result?.boxes) && result.boxes.length > 0) {
            setBoxesSync(result.boxes);
          }
        })
        .catch((raw) => {
          handleError(raw);
          setPhaseSync("selecting");
        });
    },
    [handleError, setBoxesSync, setPhaseSync],
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
            setBoxesSync([]);
            setCoverSync(null);
            setOcrState("idle");
            setPhaseSync("waiting");
          }
        })
        .catch((raw) => {
          handleError(raw);
          setPhaseSync("menu");
        });
    },
    [handleError, setBoxesSync, setCoverSync, setFrozenSync, setPhaseSync, setRectSync],
  );

  const toggleMode = useCallback(() => {
    setMode((current) => (current === "quick" ? "deep" : "quick"));
  }, []);

  /* ───────── 覆盖翻译（前端子阶段 translating → covered，后端保持 showingResult） ───────── */

  const runCover = useCallback(
    (session: SessionId, ocrBoxes: OcrBox[]) => {
      const blocks = mergeLines(
        ocrBoxes.map((box) => ({ text: box.text, confidence: box.confidence, box: box.bbox })),
      );
      if (blocks.length === 0) {
        handleError({
          code: "SA-2002",
          message: "选区内没有可覆盖翻译的文本",
          detail: null,
          retryable: false,
          fallback: ["image_path"],
          context: null,
        });
        return;
      }
      const payload = blocks.map((block) => ({
        id: block.id,
        text: block.text,
        bbox: block.bbox,
        confidence: block.lines[0]?.confidence ?? 0,
      }));
      setPhaseSync("translating");
      setCoverSync({
        session,
        phase: "translating",
        view: coverRef.current?.view ?? "cover",
        results: {},
        progress: { done: 0, total: payload.length },
        engine: null,
        blockCount: payload.length,
      });
      void ipc
        .invoke<CoverResult>(COMMANDS.translateCover, {
          session,
          blocks: payload,
        })
        .then((result) => {
          if (frozenRef.current?.session !== session) return;
          const results: Record<string, CoveredBlock> = {};
          for (const block of result.blocks) results[block.id] = block;
          const allFailed = result.totalBlocks > 0 && result.translatedBlocks === 0;
          setCoverSync({
            session,
            phase: allFailed ? "failed" : "covered",
            view: coverRef.current?.view ?? "cover",
            results,
            progress: { done: result.totalBlocks, total: result.totalBlocks },
            engine: result.engine,
            blockCount: result.totalBlocks,
          });
          setPhaseSync("covered");
        })
        .catch((raw) => {
          if (frozenRef.current?.session !== session) return;
          setCoverSync(null);
          setPhaseSync("menu");
          handleError(raw);
        });
    },
    [handleError, setCoverSync, setPhaseSync],
  );

  const startCover = useCallback(() => {
    const current = frozenRef.current;
    if (!current) return;
    if (boxesRef.current.length === 0) {
      handleError({
        code: "SA-2002",
        message: "选区内没有可覆盖翻译的文字框",
        detail: null,
        retryable: false,
        fallback: ["image_path"],
        context: null,
      });
      return;
    }
    runCover(current.session, boxesRef.current);
  }, [handleError, runCover]);

  const retranslateCover = useCallback(() => {
    const current = frozenRef.current;
    if (!current) return;
    runCover(current.session, boxesRef.current);
  }, [runCover]);

  const changeCoverView = useCallback(
    (view: CoverView) => {
      const active = coverRef.current;
      if (!active) return;
      setCoverSync({ ...active, view });
    },
    [setCoverSync],
  );

  const recaptureCover = useCallback(() => {
    setCoverSync(null);
    setBoxesSync([]);
    setPhaseSync("waiting");
    setError(null);
    void ipc.invoke<SessionId>(COMMANDS.captureFreeze).catch(handleError);
  }, [handleError, setBoxesSync, setCoverSync, setPhaseSync]);

  /* ───────── 键盘：ESC / O·B·C / Enter / 方向键 ───────── */

  useEffect(() => {
    if (!frozen) return undefined;
    const handleKeyDown = (event: KeyboardEvent): void => {
      const activeCover = coverRef.current;
      const inCover =
        activeCover !== null &&
        (phaseRef.current === "translating" || phaseRef.current === "covered");
      if (event.key === "Escape") {
        event.preventDefault();
        // 覆盖态：第一次回原文，第二次退出覆盖（研究 §7.6）
        if (inCover) {
          if (activeCover.view !== "original") {
            setCoverSync({ ...activeCover, view: "original" });
          } else {
            cancelCapture();
          }
          return;
        }
        cancelCapture();
        return;
      }
      if (inCover) {
        const key = event.key.toLowerCase();
        if (key === "o" || key === "1") {
          event.preventDefault();
          setCoverSync({ ...activeCover, view: "original" });
          return;
        }
        if (key === "b" || key === "2") {
          event.preventDefault();
          setCoverSync({ ...activeCover, view: "bilingual" });
          return;
        }
        if (key === "c" || key === "3") {
          event.preventDefault();
          setCoverSync({ ...activeCover, view: "cover" });
          return;
        }
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
  }, [frozen, cancelCapture, commitSelection, setCoverSync, setRectSync]);

  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>): void => {
    const current = frozenRef.current;
    if (!current) return;
    setCursor(toPhys(event.clientX, event.clientY, current.virtualRect, current.dpr));
  };

  /* ───────── 渲染 ───────── */

  const coverActive = cover !== null;
  const coverAvailable = Boolean(menu?.hasText) && boxes.length > 0;
  const coverDisabledReason = !menu?.hasText
    ? "未识别到文本"
    : boxes.length === 0
      ? "选区内没有文字框"
      : null;

  return (
    <div
      className={`overlay-root${frozen ? "" : " overlay-root--waiting"}${coverActive ? " overlay-root--covering" : ""}`}
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

      {frozen && cover ? (
        <TranslationOverlay
          frame={frozen.frame}
          virtualRect={frozen.virtualRect}
          dpr={frozen.dpr}
          boxes={boxes}
          results={cover.results}
          phase={cover.phase}
          view={cover.view}
          progress={cover.progress}
          engine={cover.engine}
          onViewChange={changeCoverView}
          onRetranslate={retranslateCover}
          onRecapture={recaptureCover}
          onClose={cancelCapture}
        />
      ) : null}

      {frozen && !cover ? (
        <SelectionLayer
          virtualRect={frozen.virtualRect}
          dpr={frozen.dpr}
          rect={rect}
          disabled={phase !== "selecting"}
          onRectChange={setRectSync}
          onCommit={commitSelection}
        />
      ) : null}

      {frozen && !cover && cursor && phase === "selecting" ? (
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

      {frozen && rect && menu && phase === "menu" && !cover ? (
        <ActionMenu
          menu={menu}
          anchorRect={rect}
          virtualRect={frozen.virtualRect}
          dpr={frozen.dpr}
          mode={mode}
          coverAvailable={coverAvailable}
          coverDisabledReason={coverDisabledReason}
          onToggleMode={toggleMode}
          onAction={(item) => executeAction(item, mode)}
          onCover={startCover}
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
