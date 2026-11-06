/**
 * 结果窗线程模型（纯逻辑，无 React / 无 IPC / 无 DOM，可用 `node --test` 直接跑）。
 *
 * 契约 v4（`TaskStartedPayload.parent`）语义：
 * - `parent == null` → 首轮 / 重答：**重置线程**（丢弃旧气泡）；
 * - `parent != null`   → 追问：**追加**一轮，旧回答保留为历史气泡。
 *
 * 事件归约（研究 §3.6）：
 * - `task:started` → 新建 assistant 气泡（追问时前面追加 user 气泡）；
 * - `task:delta`   → 只更新对应任务的气泡（同一时刻只有最后一条在流式）；
 * - `task:done`    → 定稿该气泡并记录 usage / sources；
 * - `task:error`   → 错误内联在该气泡；线程其它轮次不受影响。
 */

import type {
  ActionId,
  Mode,
  TaskDeltaPayload,
  TaskDonePayload,
  TaskErrorPayload,
  TaskStartedPayload,
} from "@/shared/contract/types";
import type { ResultError } from "./helpers";

/** 单轮状态。 */
export type TurnStatus = "streaming" | "done" | "error" | "cancelled";

/** 线程中的一轮问答（assistant 气泡 + 可选 user 气泡）。 */
export interface ResultTurn {
  /** 任务 id（assistant 气泡的 key）。 */
  task: string;
  /** 会话 id。 */
  session: string;
  /** 动作（首轮动作；追问沿用）。 */
  action: ActionId;
  /** 模式。 */
  mode: Mode;
  /** 引擎。 */
  engine: string;
  /** 父任务：首轮 / 重答为 null，追问为上一轮任务 id。 */
  parent: string | null;
  /** 本轮用户问题（首轮为空字符串；追问为输入框文本）。 */
  question: string;
  /** 累积正文。 */
  answer: string;
  /** 累积思考过程。 */
  reasoning: string;
  /** 元信息增量。 */
  meta: string[];
  /** 完成后的结算信息（流式中为 null）。 */
  done: TaskDonePayload | null;
  /** 错误（失败时内联展示）。 */
  error: ResultError | null;
  /** 状态。 */
  status: TurnStatus;
}

/** 由 `task:started` 构造一轮。 */
export function createTurn(payload: TaskStartedPayload, question: string): ResultTurn {
  return {
    task: payload.task,
    session: payload.session,
    action: payload.action,
    mode: payload.mode,
    engine: payload.engine,
    parent: payload.parent ?? null,
    question: payload.parent ? question : "",
    answer: "",
    reasoning: "",
    meta: [],
    done: null,
    error: null,
    status: "streaming",
  };
}

/**
 * `task:started`：`parent == null` 重置线程，否则追加一轮。
 *
 * `question` 是本次追问的输入文本（只对追问生效；首轮/重答不使用，避免重答后残留上一轮问题）。
 */
export function applyStarted(
  turns: ResultTurn[],
  payload: TaskStartedPayload,
  question = "",
): ResultTurn[] {
  if (!payload.parent) {
    return [createTurn(payload, "")];
  }
  return [...turns, createTurn(payload, question)];
}

/** `task:delta`：只更新匹配任务的那一轮；不匹配时原样返回（避免无意义重渲染）。 */
export function applyDelta(turns: ResultTurn[], payload: TaskDeltaPayload): ResultTurn[] {
  if (!turns.some((turn) => turn.task === payload.task)) return turns;
  return turns.map((turn) => {
    if (turn.task !== payload.task) return turn;
    if (payload.kind === "text") return { ...turn, answer: turn.answer + payload.chunk };
    if (payload.kind === "reasoning") {
      return { ...turn, reasoning: turn.reasoning + payload.chunk };
    }
    return { ...turn, meta: [...turn.meta, payload.chunk] };
  });
}

/** `task:done`：定稿该轮。 */
export function applyDone(turns: ResultTurn[], payload: TaskDonePayload): ResultTurn[] {
  if (!turns.some((turn) => turn.task === payload.task)) return turns;
  return turns.map((turn) =>
    turn.task === payload.task ? { ...turn, done: payload, status: "done" as const } : turn,
  );
}

/** `task:error`：错误内联到该轮，线程其它轮次保留。 */
export function applyError(
  turns: ResultTurn[],
  payload: TaskErrorPayload,
  error: ResultError,
): ResultTurn[] {
  if (!turns.some((turn) => turn.task === payload.task)) return turns;
  return turns.map((turn) =>
    turn.task === payload.task ? { ...turn, error, status: "error" as const } : turn,
  );
}

/** 取消（后端取消后不再推送事件，前端本地定稿已生成的部分）。 */
export function markCancelled(turns: ResultTurn[], task: string): ResultTurn[] {
  if (!turns.some((turn) => turn.task === task)) return turns;
  return turns.map((turn) =>
    turn.task === task ? { ...turn, status: "cancelled" as const } : turn,
  );
}

/** 最后一轮（结果窗的操作都作用于最近一轮）。 */
export function latestTurn(turns: ResultTurn[]): ResultTurn | null {
  return turns.length === 0 ? null : (turns[turns.length - 1] ?? null);
}

/** 线程里是否有正在流式生成的一轮（流式期间禁用追问输入）。 */
export function threadIsStreaming(turns: ResultTurn[]): boolean {
  return turns.some((turn) => turn.status === "streaming");
}

/** 复制全文时「问 / 答」前缀（由界面层注入翻译，保持本模块纯逻辑、可被 node --test 直接加载）。 */
export interface ThreadCopyLabels {
  question: string;
  answer: string;
}

const DEFAULT_COPY_LABELS: ThreadCopyLabels = { question: "问：", answer: "答：" };

/** 复制全文：按「问 / 答」拼接所有轮次；无追问时等价于最后一条回答。 */
export function threadPlainText(
  turns: ResultTurn[],
  labels: ThreadCopyLabels = DEFAULT_COPY_LABELS,
): string {
  const blocks: string[] = [];
  for (const turn of turns) {
    if (turn.question) blocks.push(`${labels.question}${turn.question}`);
    if (turn.answer) blocks.push(`${labels.answer}${turn.answer}`);
  }
  return blocks.join("\n\n");
}

/** 历史气泡折叠摘要（单行化 + 截断）。 */
export function summarize(text: string, maxChars = 120): string {
  const flat = text.replace(/\s+/g, " ").trim();
  const chars = Array.from(flat);
  if (chars.length <= maxChars) return flat;
  return `${chars.slice(0, maxChars).join("")}…`;
}
