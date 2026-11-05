// @ts-nocheck
// T6 历史页纯逻辑测试（Node 24 原生 TS type stripping；不依赖 React/DOM）。
//
// 运行：node --test src/windows/main/history/format.test.ts
// 说明：仓库未安装 @types/node（`node:test` 无类型声明），故文件级 @ts-nocheck。

import test from "node:test";
import assert from "node:assert/strict";

import {
  filterSessions,
  formatAbsoluteTime,
  formatHistoryTime,
  historyActionLabel,
  historyEngineLabel,
  sortSessions,
  summarizeTitle,
  threadHasIssue,
  threadPreview,
  threadTurnCount,
  turnStatusLabel,
} from "./format.ts";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

function session(overrides = {}) {
  return {
    session: "s-1",
    startedAt: new Date(2026, 10, 6, 9, 5).getTime(),
    title: "什么是框选 AI？",
    action: "answer",
    turns: 2,
    engine: "deepseek",
    ...overrides,
  };
}

test("formatAbsoluteTime：固定本地时间格式化为 YYYY-MM-DD HH:mm", () => {
  assert.equal(formatAbsoluteTime(new Date(2026, 10, 6, 9, 5).getTime()), "2026-11-06 09:05");
  assert.equal(formatAbsoluteTime(0), "—");
  assert.equal(formatAbsoluteTime(Number.NaN), "—");
});

test("formatHistoryTime：刚刚 / 分钟前 / 今天 / 昨天 / 月日 / 跨年", () => {
  const now = new Date(2026, 10, 6, 15, 30).getTime();
  assert.equal(formatHistoryTime(now - 30_000, now), "刚刚");
  assert.equal(formatHistoryTime(now - 5 * MINUTE, now), "5 分钟前");
  assert.equal(formatHistoryTime(new Date(2026, 10, 6, 8, 2).getTime(), now), "今天 08:02");
  assert.equal(formatHistoryTime(new Date(2026, 10, 5, 23, 59).getTime(), now), "昨天 23:59");
  assert.equal(formatHistoryTime(new Date(2026, 9, 1, 7, 0).getTime(), now), "10月1日 07:00");
  assert.equal(formatHistoryTime(new Date(2025, 11, 31, 7, 0).getTime(), now), "2025-12-31 07:00");
  // 未来时间不会出现负数「分钟前」，按「今天 HH:mm」展示
  assert.equal(formatHistoryTime(now + 2 * HOUR, now), "今天 17:30");
});

test("historyActionLabel：动作中文名与空值兜底", () => {
  assert.equal(historyActionLabel("translate"), "翻译");
  assert.equal(historyActionLabel("imageSearch"), "图片搜索");
  assert.equal(historyActionLabel(null), "—");
});

test("historyEngineLabel：骨架 / 本地 / 云端 / 供应商 id / 空值", () => {
  assert.equal(historyEngineLabel("skeleton"), "骨架模式");
  assert.equal(historyEngineLabel("local"), "本地模型");
  assert.equal(historyEngineLabel("cloud"), "云端");
  assert.equal(historyEngineLabel("deepseek"), "deepseek");
  assert.equal(historyEngineLabel(null), "未知引擎");
});

test("turnStatusLabel：done / error / cancelled", () => {
  assert.equal(turnStatusLabel("done"), "已完成");
  assert.equal(turnStatusLabel("error"), "失败");
  assert.equal(turnStatusLabel("cancelled"), "已取消");
});

test("sortSessions：按 startedAt 倒序且不改原数组", () => {
  const older = session({ session: "a", startedAt: 1000 });
  const newer = session({ session: "b", startedAt: 5000 });
  const input = [older, newer];
  const sorted = sortSessions(input);
  assert.deepEqual(
    sorted.map((item) => item.session),
    ["b", "a"],
  );
  assert.deepEqual(
    input.map((item) => item.session),
    ["a", "b"],
  );
});

test("summarizeTitle：单行化、截断与空标题兜底", () => {
  assert.equal(summarizeTitle("第一行\n第二行"), "第一行 第二行");
  assert.equal(summarizeTitle("  "), "（无标题）");
  const long = "甲".repeat(10);
  assert.equal(summarizeTitle(long, 5), "甲甲甲甲甲…");
});

test("filterSessions：按标题 / 引擎 / 动作过滤，空查询返回全部", () => {
  const list = [
    session({
      session: "a",
      title: "Hello world 翻译成中文",
      action: "translate",
      engine: "skeleton",
    }),
    session({ session: "b", title: "什么是框选 AI？", action: "answer", engine: "deepseek" }),
  ];
  assert.equal(filterSessions(list, "").length, 2);
  assert.deepEqual(
    filterSessions(list, "hello").map((item) => item.session),
    ["a"],
  );
  assert.deepEqual(
    filterSessions(list, "骨架").map((item) => item.session),
    ["a"],
  );
  assert.deepEqual(
    filterSessions(list, "解答").map((item) => item.session),
    ["b"],
  );
  assert.equal(filterSessions(list, "不存在").length, 0);
});

test("线程摘要：轮数 / 预览 / 异常徽标", () => {
  assert.equal(threadTurnCount(null), 0);
  const thread = {
    session: session(),
    turns: [
      {
        task: "t1",
        action: "answer",
        mode: "quick",
        question: "问题一",
        answer: "答一",
        engine: "deepseek",
        status: "done",
        createdAt: 1,
      },
      {
        task: "t2",
        action: "answer",
        mode: "quick",
        question: "问题二",
        answer: "答二",
        engine: "deepseek",
        status: "error",
        createdAt: 2,
      },
    ],
  };
  assert.equal(threadTurnCount(thread), 2);
  assert.equal(threadPreview(thread), "问题一 … 问题二");
  assert.equal(threadHasIssue(thread), true);
  assert.equal(threadHasIssue(null), false);
  assert.equal(threadPreview({ session: session(), turns: [] }), "（空线程）");
});
