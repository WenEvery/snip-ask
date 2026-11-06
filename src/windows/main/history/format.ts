/**
 * 历史页纯逻辑（无 React / 无 IPC / 无 DOM，可用 `node --test` 直接跑）。
 *
 * 国际化：本模块被 `node --test` 直接 import，不能依赖 `@/i18n`（Node 无法解析别名/无扩展名
 * 导入）。因此面向用户的文案通过可选参数 `translate` 注入（UI 传 `t`），默认实现保留中文并
 * 按 `{name}` 插值，保证纯逻辑测试与无 DOM 环境行为不变。
 *
 * 契约 v5：`HistorySession.startedAt` / `HistoryTurn.createdAt` 均为 Unix 毫秒；
 * `engine` 为自由字符串（`skeleton` / 供应商 id / `local` / `cloud`）。
 */

import type { ActionId, HistorySession, HistoryThread, HistoryTurn } from "@/shared/contract/types";

/** 与 `src/i18n` 的 `t(zh, vars?)` 同形；默认实现仅插值、不翻译（中文回退）。 */
export type TranslateText = (zh: string, vars?: Record<string, string | number>) => string;

function interpolate(zh: string, vars?: Record<string, string | number>): string {
  if (!vars) return zh;
  return zh.replace(/\{([a-zA-Z0-9_]+)\}/g, (whole, name: string) => {
    const value = vars[name];
    return value === undefined ? whole : String(value);
  });
}

const identityTranslate: TranslateText = (zh, vars) => interpolate(zh, vars);

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** 动作中文名（与契约 §4.1 `ACTION_LABELS` 一致；此处本地副本保证本模块可被 node 直接加载）。 */
const ACTION_LABELS: Record<ActionId, string> = {
  search: "AI 搜索",
  imageSearch: "图片搜索",
  translate: "翻译",
  answer: "解答",
  copy: "复制",
};

function pad2(value: number): string {
  return String(value).padStart(2, "0");
}

function hhmm(date: Date): string {
  return `${pad2(date.getHours())}:${pad2(date.getMinutes())}`;
}

function sameDay(left: Date, right: Date): boolean {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}

/** 绝对时间 `YYYY-MM-DD HH:mm`（历史页展开后的精确时间）。 */
export function formatAbsoluteTime(timestamp: number): string {
  if (!Number.isFinite(timestamp) || timestamp <= 0) return "—";
  const date = new Date(timestamp);
  return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())} ${hhmm(date)}`;
}

/**
 * 列表相对时间：刚刚 / N 分钟前 / 今天 HH:mm / 昨天 HH:mm / M月D日 HH:mm / YYYY-MM-DD HH:mm。
 * `now` 可注入，便于单测（默认 Date.now()）。
 */
export function formatHistoryTime(
  timestamp: number,
  now: number = Date.now(),
  translate: TranslateText = identityTranslate,
): string {
  if (!Number.isFinite(timestamp) || timestamp <= 0) return "—";
  const diff = now - timestamp;
  if (diff >= 0 && diff < MINUTE) return translate("刚刚");
  if (diff >= 0 && diff < HOUR) {
    return translate("{minutes} 分钟前", { minutes: Math.floor(diff / MINUTE) });
  }
  const date = new Date(timestamp);
  const current = new Date(now);
  if (sameDay(date, current)) return translate("今天 {time}", { time: hhmm(date) });
  const yesterday = new Date(now - DAY);
  if (sameDay(date, yesterday)) return translate("昨天 {time}", { time: hhmm(date) });
  if (date.getFullYear() === current.getFullYear()) {
    return translate("{month}月{day}日 {time}", {
      month: date.getMonth() + 1,
      day: date.getDate(),
      time: hhmm(date),
    });
  }
  return formatAbsoluteTime(timestamp);
}

/** 动作中文名（`null` 时返回「—」）。 */
export function historyActionLabel(
  action: ActionId | null,
  translate: TranslateText = identityTranslate,
): string {
  return action ? translate(ACTION_LABELS[action]) : "—";
}

/** 引擎中文名：skeleton → 演示 / local → 本地模型 / cloud → 云端 / 供应商 id 原样 / 空 → 未知。 */
export function historyEngineLabel(
  engine: string | null,
  translate: TranslateText = identityTranslate,
): string {
  if (!engine) return translate("未知来源");
  switch (engine) {
    case "skeleton":
      return translate("演示");
    case "local":
      return translate("本地模型");
    case "cloud":
      return translate("云端");
    default:
      return engine;
  }
}

/** 轮次状态中文名（契约 v5：done / error / cancelled）。 */
export function turnStatusLabel(
  status: HistoryTurn["status"],
  translate: TranslateText = identityTranslate,
): string {
  switch (status) {
    case "done":
      return translate("已完成");
    case "error":
      return translate("失败");
    case "cancelled":
      return translate("已取消");
    default:
      return translate("未知状态");
  }
}

/** 按开始时间倒序（后端已排序，前端再兜底一次）。 */
export function sortSessions(sessions: HistorySession[]): HistorySession[] {
  return [...sessions].sort((left, right) => right.startedAt - left.startedAt);
}

/** 标题摘要：单行化并截断（列表宽度有限）。 */
export function summarizeTitle(
  title: string,
  maxChars = 60,
  translate: TranslateText = identityTranslate,
): string {
  const flat = (title || "").replace(/\s+/g, " ").trim();
  if (!flat) return translate("（无标题）");
  const chars = Array.from(flat);
  if (chars.length <= maxChars) return flat;
  return `${chars.slice(0, maxChars).join("")}…`;
}

/** 按标题 / 引擎 / 动作过滤（大小写不敏感，空查询返回原列表）。 */
export function filterSessions(
  sessions: HistorySession[],
  query: string,
  translate: TranslateText = identityTranslate,
): HistorySession[] {
  const keyword = query.trim().toLowerCase();
  if (!keyword) return sessions;
  return sessions.filter((session) => {
    const haystack = [
      session.title,
      historyEngineLabel(session.engine, translate),
      session.engine ?? "",
      historyActionLabel(session.action, translate),
    ]
      .join(" ")
      .toLowerCase();
    return haystack.includes(keyword);
  });
}

/** 线程总轮数（空线程为 0）。 */
export function threadTurnCount(thread: HistoryThread | null): number {
  return thread ? thread.turns.length : 0;
}

/** 线程摘要（列表悬停提示）：首轮问题 → 末轮问题。 */
export function threadPreview(
  thread: HistoryThread,
  translate: TranslateText = identityTranslate,
): string {
  if (thread.turns.length === 0) return translate("（空线程）");
  const first = thread.turns[0]?.question ?? "";
  const last = thread.turns[thread.turns.length - 1]?.question ?? "";
  return first === last
    ? summarizeTitle(first, 60, translate)
    : `${summarizeTitle(first, 60, translate)} … ${summarizeTitle(last, 60, translate)}`;
}

/** 线程是否有失败 / 取消的轮次（列表徽标）。 */
export function threadHasIssue(thread: HistoryThread | null): boolean {
  return thread?.turns.some((turn) => turn.status !== "done") ?? false;
}
