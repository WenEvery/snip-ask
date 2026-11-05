/**
 * 结果窗：线程式多轮视图（契约 v4 / 研究 §3.6）。
 *
 * - 订阅 `task:started` / `task:delta` / `task:done` / `task:error`，把事件归约为 `turns[]`
 *   （`task:started.parent == null` → 重置线程；非 null → 追加一轮，旧回答保留）；
 * - 仅最后一轮流式渲染；历史轮次折叠为摘要（可展开）；错误内联在对应轮次；
 * - 支持追问 / 重答（清空线程后重跑）/ 取消 / 清空线程 / 关闭；
 * - 流式期间禁用追问输入（研究 §3.10-7）。
 *
 * 浏览器预览（无 Tauri）：600ms 内未收到 `task:started` 时自动发起一次
 * `menu_execute` 走 mock 后端，便于单独预览 `/?window=result`。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { ipc, isTauri } from "@/shared/ipc";
import { COMMANDS, EVENTS } from "@/shared/contract/names";
import { useIpcEvent } from "@/shared/hooks/useIpcEvent";
import type {
  ActionId,
  SelectionResult,
  TaskDeltaPayload,
  TaskDonePayload,
  TaskErrorPayload,
  TaskStartedPayload,
} from "@/shared/contract/types";
import { ThreadView } from "./ThreadView";
import { ResultHeader } from "./ResultHeader";
import { ResultFooter } from "./ResultFooter";
import { fallbackText, fromTaskError, normalizeError, type ResultError } from "./helpers";
import {
  applyDelta,
  applyDone,
  applyError,
  applyStarted,
  latestTurn,
  markCancelled,
  threadIsStreaming,
  threadPlainText,
  type ResultTurn,
} from "./thread";
import "./result.css";

const ACTION_TITLES: Record<ActionId, string> = {
  search: "文本搜索",
  imageSearch: "图片搜索",
  translate: "翻译",
  answer: "解答",
  copy: "复制文字",
};

export default function ResultWindow() {
  /** 线程：一轮问答一个元素（最后一条才是活动气泡）。 */
  const [turns, setTurns] = useState<ResultTurn[]>([]);
  const [followup, setFollowup] = useState("");
  const [sending, setSending] = useState(false);
  /** 追问提交失败（invoke 未产生任务时）内联展示；保留输入便于重试。 */
  const [submitError, setSubmitError] = useState<ResultError | null>(null);
  const [copied, setCopied] = useState(false);

  const activeTaskRef = useRef<string | null>(null);
  /** 待挂到下一轮 user 气泡上的追问文本（`task:started` 到达时消费）。 */
  const pendingQuestionRef = useRef<string | null>(null);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const cardRef = useRef<HTMLDivElement | null>(null);
  const followRef = useRef(true);

  const latest = latestTurn(turns);
  const first = turns.length > 0 ? turns[0] : null;
  const streaming = threadIsStreaming(turns);
  const busy = streaming || sending;

  // ── 任务事件 → 线程归约 ────────────────────────────────────
  useIpcEvent<TaskStartedPayload>(EVENTS.taskStarted, (payload) => {
    activeTaskRef.current = payload.task;
    followRef.current = true;
    setSubmitError(null);
    const question = pendingQuestionRef.current ?? "";
    pendingQuestionRef.current = null;
    setTurns((current) => applyStarted(current, payload, question));
  });

  useIpcEvent<TaskDeltaPayload>(EVENTS.taskDelta, (payload) => {
    setTurns((current) => applyDelta(current, payload));
  });

  useIpcEvent<TaskDonePayload>(EVENTS.taskDone, (payload) => {
    setTurns((current) => applyDone(current, payload));
  });

  useIpcEvent<TaskErrorPayload>(EVENTS.taskError, (payload) => {
    setTurns((current) => applyError(current, payload, fromTaskError(payload)));
  });

  // 浏览器预览用的完整 mock 演示链（真实环境不调用）：
  // 契约要求会话状态机合法，孤立的结果窗没有 overlay 阶段，因此在 mock 下补
  // capture_freeze → selection_commit，再 menu_execute。
  const startMockTask = useCallback(async () => {
    const session = await ipc.invoke<string>(COMMANDS.captureFreeze);
    await ipc.invoke<SelectionResult>(COMMANDS.selectionCommit, {
      session,
      rectPhys: { x: 0, y: 0, width: 480, height: 240 },
    });
    await ipc.invoke<string>(COMMANDS.menuExecute, {
      session,
      action: "answer",
      mode: "quick",
    });
  }, []);

  // ── 浏览器 mock 预览：600ms 内没有 task:started 时自动演示一次 ──
  useEffect(() => {
    if (isTauri()) return;
    const timer = window.setTimeout(() => {
      if (activeTaskRef.current) return;
      void startMockTask().catch((err) => {
        setSubmitError(normalizeError(err));
      });
    }, 600);
    return () => window.clearTimeout(timer);
  }, [startMockTask]);

  // ── 自动滚动：用户上滚后暂停，回到底部恢复 ──────────────────
  useEffect(() => {
    const element = bodyRef.current;
    if (!element || !followRef.current) return;
    element.scrollTop = element.scrollHeight;
  }, [turns, submitError]);

  const onBodyScroll = () => {
    const element = bodyRef.current;
    if (!element) return;
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
    followRef.current = distance < 48;
  };

  // ── 操作 ────────────────────────────────────────────────────
  const copyText = useCallback(async () => {
    const content = threadPlainText(turns);
    if (!content) return;
    try {
      await navigator.clipboard.writeText(content);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      const ok = fallbackCopy(content);
      setCopied(ok);
      if (!ok) {
        setSubmitError({
          code: "SA-0000",
          message: "复制失败：剪贴板不可用，请手动选择文本复制。",
          detail: null,
          retryable: false,
          fallback: [],
        });
      }
    }
  }, [turns]);

  /** 关闭 / 清空线程：结束当前对话（后端 `result_close` 同时清空会话与内存线程）。 */
  const endConversation = useCallback(async () => {
    const session = latest?.session;
    if (session) {
      try {
        await ipc.invoke<void>(COMMANDS.resultClose, { session });
      } catch {
        /* 关闭失败也要隐藏窗口 */
      }
    }
    pendingQuestionRef.current = null;
    activeTaskRef.current = null;
    setTurns([]);
    setFollowup("");
    setSubmitError(null);
    try {
      await ipc.invoke<void>(COMMANDS.windowHide, { window: "result" });
    } catch {
      /* mock 环境为 no-op */
    }
  }, [latest]);

  const rerun = async () => {
    if (!first || busy) return;
    setSubmitError(null);
    try {
      // 重答：parent = null → 前端重置线程、后端清空轮次后重跑。
      await ipc.invoke<string>(COMMANDS.menuExecute, {
        session: first.session,
        action: first.action,
        mode: first.mode,
      });
    } catch (err) {
      const parsed = normalizeError(err);
      // 浏览器预览：后端会话已离开 menu 态，用 mock 演示链重新起一次
      if (!isTauri() && parsed.code === "SA-1099") {
        try {
          await startMockTask();
          return;
        } catch (mockErr) {
          setSubmitError(normalizeError(mockErr));
        }
      } else {
        setSubmitError(parsed);
      }
    }
  };

  const cancel = async () => {
    if (!latest) return;
    try {
      await ipc.invoke<void>(COMMANDS.taskCancel, { task: latest.task });
      // 后端取消后不再推送该任务事件，前端本地把该轮定稿为「已取消」。
      setTurns((current) => markCancelled(current, latest.task));
    } catch (err) {
      setSubmitError(normalizeError(err));
    }
  };

  const submitFollowup = async () => {
    if (!latest || busy) return;
    const question = followup.trim();
    if (!question) return;
    setSending(true);
    setSubmitError(null);
    pendingQuestionRef.current = question;
    try {
      // 最近一轮任务 id 仅作校验；后端以当前会话 + 最近任务定位（F1 修复）。
      await ipc.invoke<string>(COMMANDS.taskFollowup, {
        task: latest.task,
        text: question,
      });
      setFollowup("");
    } catch (err) {
      pendingQuestionRef.current = null;
      // 保留输入内容，错误内联提示，便于直接重试（研究 §3.6）。
      setSubmitError(normalizeError(err));
    } finally {
      setSending(false);
    }
  };

  // 结果窗高度随内容自适应：去掉固定的 460×420 窗口下多余透明区域。
  // 宽度保持用户调整后的宽度；高度 = 顶栏 + 正文内容 + 底栏，上限为屏幕可用高度的 85%。
  useEffect(() => {
    if (!isTauri()) return undefined;
    const win = getCurrentWindow();
    let disposed = false;
    let raf = 0;

    const apply = async () => {
      const card = cardRef.current;
      if (!card || disposed) return;
      // 卡片在窗口内被拉满（height:100%），直接量 scrollHeight 会得到窗口高度而不是内容高度。
      // 先临时取消拉伸，量出真实内容高度，再恢复（同步执行，不产生可见闪烁）。
      const previousHeight = card.style.height;
      const previousMaxHeight = card.style.maxHeight;
      card.style.height = "auto";
      card.style.maxHeight = "none";
      const content = card.getBoundingClientRect().height;
      card.style.height = previousHeight;
      card.style.maxHeight = previousMaxHeight;
      const maxHeight = Math.min(window.screen.availHeight * 0.85, 900);
      const height = Math.min(Math.max(Math.ceil(content), 240), maxHeight);
      try {
        const scale = await win.scaleFactor();
        const current = await win.innerSize();
        const target = Math.round(height * scale);
        if (Math.abs(current.height - target) <= 2) return;
        await win.setSize(new LogicalSize(current.width / scale, height));
      } catch {
        // 窗口 API 不可用（权限/预览环境）时保持默认尺寸，不阻塞内容渲染。
      }
    };
    const schedule = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => void apply());
    };

    schedule();
    const resizeObserver = new ResizeObserver(schedule);
    if (cardRef.current) resizeObserver.observe(cardRef.current);
    const mutationObserver = new MutationObserver(schedule);
    if (cardRef.current) {
      mutationObserver.observe(cardRef.current, {
        childList: true,
        subtree: true,
        characterData: true,
      });
    }
    return () => {
      disposed = true;
      cancelAnimationFrame(raf);
      resizeObserver.disconnect();
      mutationObserver.disconnect();
    };
  }, []);

  const title = first ? ACTION_TITLES[first.action] : "结果";

  return (
    <div className="result-root">
      <div className="result-card" ref={cardRef}>
        <ResultHeader
          title={title}
          mode={first ? first.mode : null}
          engine={latest ? latest.engine : null}
          round={turns.length}
          browserPreview={!isTauri()}
          canCopy={Boolean(threadPlainText(turns))}
          copied={copied}
          onCopy={() => void copyText()}
          onClose={() => void endConversation()}
        />

        <div className="result-body" ref={bodyRef} onScroll={onBodyScroll}>
          {turns.length === 0 ? (
            <div className="result-empty">
              <p className="result-empty-title">等待框选结果…</p>
              <p className="result-hint">框选屏幕上的内容并选择一个动作，回答会显示在这里。</p>
            </div>
          ) : (
            <ThreadView
              turns={turns}
              onSourceError={(err) => setSubmitError(normalizeError(err))}
            />
          )}

          {submitError ? (
            <div className="result-error" role="alert">
              <div className="result-error-head">
                <strong className="result-error-message">{submitError.message}</strong>
                <code className="result-error-code">{submitError.code}</code>
                {submitError.retryable ? <span className="result-hint">可重试</span> : null}
              </div>
              {submitError.detail ? (
                <pre className="result-error-detail">{submitError.detail}</pre>
              ) : null}
              {submitError.fallback.length > 0 ? (
                <p className="result-error-fallback">
                  可以试试：{fallbackText(submitError.fallback)}
                </p>
              ) : null}
            </div>
          ) : null}
        </div>

        {turns.length > 0 ? (
          <ResultFooter
            streaming={streaming}
            disabled={busy}
            followup={followup}
            sending={sending}
            onFollowupChange={setFollowup}
            onSubmit={() => void submitFollowup()}
            onRerun={() => void rerun()}
            onCancel={() => void cancel()}
            onClearThread={() => void endConversation()}
          />
        ) : null}
      </div>
    </div>
  );
}

/** 剪贴板降级：临时 textarea + execCommand */
function fallbackCopy(content: string): boolean {
  try {
    const textarea = document.createElement("textarea");
    textarea.value = content;
    textarea.setAttribute("readonly", "");
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    textarea.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(textarea);
    return ok;
  } catch {
    return false;
  }
}
