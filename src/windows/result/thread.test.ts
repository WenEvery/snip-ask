// @ts-nocheck
// T3 结果窗线程模型纯逻辑测试（Node 24 原生 TS type stripping；不依赖 React/DOM）。
//
// 运行：node --test src/windows/result/thread.test.ts
// 说明：tsconfig 的 `include` 只有 `src`，本文件用 `@ts-nocheck` 避免引入 @types/node 依赖
// （仓库未安装 @types/node，`node:test` 无类型声明）。

import test from "node:test";
import assert from "node:assert/strict";

import {
  applyDelta,
  applyDone,
  applyError,
  applyStarted,
  createTurn,
  latestTurn,
  markCancelled,
  summarize,
  threadIsStreaming,
  threadPlainText,
} from "./thread.ts";

function started(overrides = {}) {
  return {
    task: "t-1",
    session: "s-1",
    action: "answer",
    mode: "quick",
    engine: "skeleton",
    parent: null,
    ...overrides,
  };
}

function delta(task, chunk, kind = "text") {
  return { task, kind, chunk };
}

function done(task) {
  return {
    task,
    usage: {
      inputTokens: 120,
      outputTokens: 60,
      cachedInputTokens: 0,
      costEstimate: 0,
      currency: "CNY",
    },
    latencyMs: 42,
    engine: "skeleton",
    sources: [],
  };
}

function errorPayload(task) {
  return {
    task,
    code: "SA-3001",
    message: "网络请求失败",
    retryable: true,
    fallback: ["retry"],
  };
}

test("parent=null 表示首轮/重答：重置线程", () => {
  let turns = [];
  turns = applyStarted(turns, started({ task: "t-1", parent: null }), "不该用的问题");
  turns = applyDelta(turns, delta("t-1", "第一轮回答"));
  assert.equal(turns.length, 1);
  assert.equal(turns[0].question, "", "首轮不应挂追问文本");

  // 追问追加一轮
  turns = applyStarted(turns, started({ task: "t-2", parent: "t-1" }), "再解释第二点");
  assert.equal(turns.length, 2);
  assert.equal(turns[1].parent, "t-1");
  assert.equal(turns[1].question, "再解释第二点");

  // 重答（parent=null）清空旧线程（F3：不再覆盖式渲染，但重答必须重置）
  turns = applyStarted(turns, started({ task: "t-3", parent: null }), "残留问题");
  assert.equal(turns.length, 1, "重答必须重置线程");
  assert.equal(turns[0].task, "t-3");
  assert.equal(turns[0].question, "");
});

test("delta 只更新对应轮次：历史回答冻结，仅最后一条流式", () => {
  let turns = [];
  turns = applyStarted(turns, started({ task: "t-1", parent: null }));
  turns = applyDelta(turns, delta("t-1", "历史回答"));
  turns = applyStarted(turns, started({ task: "t-2", parent: "t-1" }), "追问");

  const before = turns[0].answer;
  turns = applyDelta(turns, delta("t-2", "新回答"));
  assert.equal(turns[0].answer, before, "历史轮次不得被新 delta 改写");
  assert.equal(turns[1].answer, "新回答");
  assert.equal(threadIsStreaming(turns), true);

  // 不匹配任何轮次的 delta 原样返回（同一数组引用，避免无意义重渲染）
  assert.equal(applyDelta(turns, delta("t-unknown", "x")), turns);
});

test("done 定稿该轮并记录 usage；其它轮次不受影响", () => {
  let turns = [];
  turns = applyStarted(turns, started({ task: "t-1", parent: null }));
  turns = applyDelta(turns, delta("t-1", "回答一"));
  turns = applyDone(turns, done("t-1"));
  assert.equal(turns[0].status, "done");
  assert.equal(turns[0].done.latencyMs, 42);
  assert.equal(threadIsStreaming(turns), false);

  turns = applyStarted(turns, started({ task: "t-2", parent: "t-1" }), "追问");
  turns = applyDelta(turns, delta("t-2", "回答二"));
  turns = applyDone(turns, done("t-2"));
  assert.equal(turns.length, 2);
  assert.equal(turns[0].answer, "回答一");
  assert.equal(turns[1].status, "done");
});

test("error 内联在对应轮次：状态与错误都落在该气泡", () => {
  let turns = [];
  turns = applyStarted(turns, started({ task: "t-1", parent: null }));
  turns = applyDelta(turns, delta("t-1", "部分回答"));
  turns = applyError(turns, errorPayload("t-1"), {
    code: "SA-3001",
    message: "网络请求失败",
    detail: null,
    retryable: true,
    fallback: ["retry"],
  });
  assert.equal(turns[0].status, "error");
  assert.equal(turns[0].error.code, "SA-3001");
  assert.equal(turns[0].answer, "部分回答", "失败时保留已生成的部分内容");

  // 失败后仍可继续追问（状态机契约 v4：失败回 showingResult）
  turns = applyStarted(turns, started({ task: "t-2", parent: "t-1" }), "重试一下");
  assert.equal(turns.length, 2);
  assert.equal(turns[1].status, "streaming");
});

test("取消：本地把该轮定稿为 cancelled，保留部分正文", () => {
  let turns = [];
  turns = applyStarted(turns, started({ task: "t-1", parent: null }));
  turns = applyDelta(turns, delta("t-1", "已生成一半"));
  turns = markCancelled(turns, "t-1");
  assert.equal(turns[0].status, "cancelled");
  assert.equal(turns[0].answer, "已生成一半");
  assert.equal(turns[0].error, null);
  assert.equal(threadIsStreaming(turns), false);
});

test("threadPlainText 复制全文包含所有轮次问答", () => {
  let turns = [];
  turns = applyStarted(turns, started({ task: "t-1", parent: null }));
  turns = applyDelta(turns, delta("t-1", "首轮回答"));
  turns = applyStarted(turns, started({ task: "t-2", parent: "t-1" }), "第二点是什么");
  turns = applyDelta(turns, delta("t-2", "第二轮回答"));
  const text = threadPlainText(turns);
  assert.ok(text.includes("首轮回答"));
  assert.ok(text.includes("问：第二点是什么"));
  assert.ok(text.includes("第二轮回答"));
  assert.equal(threadPlainText([]), "");
});

test("latestTurn / createTurn / summarize 边界", () => {
  assert.equal(latestTurn([]), null);
  const turn = createTurn(started({ task: "t-9", parent: "t-8" }), "问题");
  assert.equal(turn.task, "t-9");
  assert.equal(turn.parent, "t-8");
  assert.equal(turn.question, "问题");
  assert.equal(turn.status, "streaming");

  const long = "很长的回答".repeat(50);
  const brief = summarize(long, 20);
  assert.ok(Array.from(brief).length <= 21);
  assert.ok(brief.endsWith("…"));
  assert.equal(summarize("  多行\n回答  ", 100), "多行 回答");
});
