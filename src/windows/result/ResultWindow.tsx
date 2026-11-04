/**
 * 结果窗：订阅 task.started / task.delta / task.done / task.error，
 * 增量渲染流式文本，展示用量与来源，支持复制 / 追问 / 重答 / 取消 / 关闭。
 *
 * 浏览器预览（无 Tauri）：600ms 内未收到 task.started 时自动发起一次
 * `menu_execute` 走 mock 后端，便于单独预览 `/?window=result`。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { ipc, isTauri } from "@/shared/ipc";
import { COMMANDS, EVENTS } from "@/shared/contract/names";
import { useIpcEvent } from "@/shared/hooks/useIpcEvent";
import type {
  ActionId,
  Mode,
  SelectionResult,
  TaskDeltaPayload,
  TaskDonePayload,
  TaskErrorPayload,
  TaskStartedPayload,
} from "@/shared/contract/types";
import { StreamView } from "./StreamView";
import { SourceList } from "./SourceList";
import { ResultHeader } from "./ResultHeader";
import { ResultFooter } from "./ResultFooter";
import {
  fallbackText,
  formatCost,
  formatLatency,
  fromTaskError,
  normalizeError,
  type ResultError,
} from "./helpers";
import "./result.css";

type Phase = "empty" | "streaming" | "done" | "error" | "cancelled";

interface TaskContext {
  task: string;
  session: string;
  action: ActionId;
  mode: Mode;
  engine: string;
}

const ACTION_TITLES: Record<ActionId, string> = {
  search: "文本搜索",
  imageSearch: "图片搜索",
  translate: "翻译",
  answer: "解答",
  copy: "复制文字",
};

export default function ResultWindow() {
  const [ctx, setCtx] = useState<TaskContext | null>(null);
  const [phase, setPhase] = useState<Phase>("empty");
  const [text, setText] = useState("");
  const [reasoning, setReasoning] = useState("");
  const [meta, setMeta] = useState<string[]>([]);
  const [done, setDone] = useState<TaskDonePayload | null>(null);
  const [error, setError] = useState<ResultError | null>(null);
  const [followup, setFollowup] = useState("");
  const [sending, setSending] = useState(false);
  const [copied, setCopied] = useState(false);

  const taskIdRef = useRef<string | null>(null);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const followRef = useRef(true);

  // ── 任务事件 ────────────────────────────────────────────────
  useIpcEvent<TaskStartedPayload>(EVENTS.taskStarted, (payload) => {
    taskIdRef.current = payload.task;
    followRef.current = true;
    setCtx({
      task: payload.task,
      session: payload.session,
      action: payload.action,
      mode: payload.mode,
      engine: payload.engine,
    });
    setText("");
    setReasoning("");
    setMeta([]);
    setDone(null);
    setError(null);
    setPhase("streaming");
  });

  useIpcEvent<TaskDeltaPayload>(EVENTS.taskDelta, (payload) => {
    if (payload.task !== taskIdRef.current) return;
    if (payload.kind === "text") {
      setText((current) => current + payload.chunk);
    } else if (payload.kind === "reasoning") {
      setReasoning((current) => current + payload.chunk);
    } else {
      setMeta((current) => [...current, payload.chunk]);
    }
  });

  useIpcEvent<TaskDonePayload>(EVENTS.taskDone, (payload) => {
    if (payload.task !== taskIdRef.current) return;
    setDone(payload);
    setPhase("done");
  });

  useIpcEvent<TaskErrorPayload>(EVENTS.taskError, (payload) => {
    if (payload.task !== taskIdRef.current) return;
    setError(fromTaskError(payload));
    setPhase("error");
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

  // ── 浏览器 mock 预览：600ms 内没有 task.started 时自动演示一次 ──
  useEffect(() => {
    if (isTauri()) return;
    const timer = window.setTimeout(() => {
      if (taskIdRef.current) return;
      void startMockTask().catch((err) => {
        setError(normalizeError(err));
        setPhase("error");
      });
    }, 600);
    return () => window.clearTimeout(timer);
  }, [startMockTask]);

  // ── 自动滚动：用户上滚后暂停，回到底部恢复 ──────────────────
  useEffect(() => {
    const element = bodyRef.current;
    if (!element || !followRef.current) return;
    element.scrollTop = element.scrollHeight;
  }, [text, reasoning, meta, phase]);

  const onBodyScroll = () => {
    const element = bodyRef.current;
    if (!element) return;
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
    followRef.current = distance < 48;
  };

  // ── 操作 ────────────────────────────────────────────────────
  const copyText = useCallback(async () => {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      const ok = fallbackCopy(text);
      setCopied(ok);
      if (!ok) {
        setError({
          code: "SA-0000",
          message: "复制失败：剪贴板不可用，请手动选择文本复制。",
          detail: null,
          retryable: false,
          fallback: [],
        });
        setPhase("error");
      }
    }
  }, [text]);

  const close = async () => {
    const session = ctx?.session;
    if (session) {
      try {
        await ipc.invoke<void>(COMMANDS.resultClose, { session });
      } catch {
        /* 关闭失败也要隐藏窗口 */
      }
    }
    try {
      await ipc.invoke<void>(COMMANDS.windowHide, { window: "result" });
    } catch {
      /* mock 环境为 no-op */
    }
  };

  const rerun = async () => {
    if (!ctx) return;
    setError(null);
    try {
      await ipc.invoke<string>(COMMANDS.menuExecute, {
        session: ctx.session,
        action: ctx.action,
        mode: ctx.mode,
      });
    } catch (err) {
      const parsed = normalizeError(err);
      // 浏览器预览：后端会话已离开 menu 态，用 mock 演示链重新起一次
      if (!isTauri() && parsed.code === "SA-1099") {
        try {
          await startMockTask();
          return;
        } catch (mockErr) {
          setError(normalizeError(mockErr));
        }
      } else {
        setError(parsed);
      }
      setPhase("error");
    }
  };

  const cancel = async () => {
    if (!ctx) return;
    try {
      await ipc.invoke<void>(COMMANDS.taskCancel, { task: ctx.task });
      setPhase("cancelled");
    } catch (err) {
      setError(normalizeError(err));
      setPhase("error");
    }
  };

  const submitFollowup = async () => {
    if (!ctx || sending) return;
    const question = followup.trim();
    if (!question) return;
    setSending(true);
    try {
      await ipc.invoke<string>(COMMANDS.taskFollowup, { task: ctx.task, text: question });
      setFollowup("");
    } catch (err) {
      setError(normalizeError(err));
      setPhase("error");
    } finally {
      setSending(false);
    }
  };

  const title = ctx ? ACTION_TITLES[ctx.action] : "结果";

  return (
    <div className="result-root">
      <div className="result-card">
        <ResultHeader
          title={title}
          mode={ctx ? ctx.mode : null}
          engine={ctx ? ctx.engine : null}
          browserPreview={!isTauri()}
          canCopy={Boolean(text)}
          copied={copied}
          onCopy={() => void copyText()}
          onClose={() => void close()}
        />

        <div className="result-body" ref={bodyRef} onScroll={onBodyScroll}>
          {phase === "empty" ? (
            <div className="result-empty">
              <p className="result-empty-title">等待框选结果…</p>
              <p className="result-hint">框选屏幕区域并选择动作后，结果会流式显示在这里。</p>
            </div>
          ) : null}

          {ctx ? (
            <StreamView text={text} reasoning={reasoning} meta={meta} streaming={phase === "streaming"} />
          ) : null}

          {phase === "cancelled" ? <p className="result-hint">已取消本次生成。</p> : null}

          {phase === "error" && error ? (
            <div className="result-error" role="alert">
              <div className="result-error-head">
                <strong className="result-error-message">{error.message}</strong>
                <code className="result-error-code">{error.code}</code>
                {error.retryable ? <span className="result-hint">可重试</span> : null}
              </div>
              {error.detail ? <pre className="result-error-detail">{error.detail}</pre> : null}
              {error.fallback.length > 0 ? (
                <p className="result-error-fallback">可尝试：{fallbackText(error.fallback)}</p>
              ) : null}
            </div>
          ) : null}

          {done ? (
            <div className="result-usage">
              <span>延迟 {formatLatency(done.latencyMs)}</span>
              <span>输入 {done.usage.inputTokens} tokens</span>
              <span>输出 {done.usage.outputTokens} tokens</span>
              <span>费用 {formatCost(done.usage.costEstimate)}</span>
              <span>引擎 {done.engine}</span>
            </div>
          ) : null}

          {done && done.sources.length > 0 ? (
            <SourceList
              sources={done.sources}
              onError={(err) => {
                setError(normalizeError(err));
                setPhase("error");
              }}
            />
          ) : null}
        </div>

        {ctx ? (
          <ResultFooter
            streaming={phase === "streaming"}
            followup={followup}
            sending={sending}
            onFollowupChange={setFollowup}
            onSubmit={() => void submitFollowup()}
            onRerun={() => void rerun()}
            onCancel={() => void cancel()}
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
