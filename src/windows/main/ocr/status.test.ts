// @ts-nocheck
// T6 OCR 状态聚合纯逻辑测试（Node 24 原生 TS type stripping；不依赖 React/DOM）。
//
// 运行：node --test src/windows/main/ocr/status.test.ts
// 说明：仓库未安装 @types/node（`node:test` 无类型声明），故文件级 @ts-nocheck。

import test from "node:test";
import assert from "node:assert/strict";

import {
  allBundled,
  editionBadge,
  editionLabel,
  mergeOcrRows,
  ocrModels,
  pendingOcrRows,
  summarizeOcr,
} from "./status.ts";

function asset(id, overrides = {}) {
  return {
    id,
    name: `资产 ${id}`,
    installed: true,
    bundled: false,
    sizeBytes: 1024,
    sha256: "a".repeat(64),
    ...overrides,
  };
}

function model(id, overrides = {}) {
  return {
    id,
    name: `模型 ${id}`,
    kind: "onnx",
    sizeBytes: 2048,
    sha256: "b".repeat(64),
    license: "Apache-2.0",
    minRamMB: 0,
    vramMB: 0,
    urls: [],
    params: {},
    installed: false,
    installedBytes: 0,
    downloadState: "queued",
    ...overrides,
  };
}

function status(overrides = {}) {
  return {
    ready: true,
    bundled: false,
    runtimeReady: true,
    runtimePath: "C:/app/ocr/onnxruntime.dll",
    assets: [asset("ocr-det"), asset("ocr-rec"), asset("ocr-cls"), asset("ocr-dict")],
    missing: [],
    message: null,
    ...overrides,
  };
}

test("editionLabel / editionBadge：full / slim / dev / 未知", () => {
  assert.equal(editionLabel("full"), "完整版（内置 OCR 模型）");
  assert.equal(editionLabel("slim"), "精简版（安装后下载 OCR 模型）");
  assert.equal(editionLabel("dev"), "开发构建");
  assert.equal(editionLabel(null), "未知");
  assert.equal(editionLabel("weird"), "未知（weird）");
  assert.equal(editionBadge("full"), "完整版");
  assert.equal(editionBadge("slim"), "精简版");
  assert.equal(editionBadge("dev"), "开发构建");
  assert.equal(editionBadge(undefined), "未知版本");
});

test("ocrModels：只保留 kind === onnx 的模型", () => {
  const models = [
    model("ocr-det"),
    model("hy-mt", { kind: "gguf" }),
    model("ct2", { kind: "ct2" }),
  ];
  assert.deepEqual(
    ocrModels(models).map((item) => item.id),
    ["ocr-det"],
  );
});

test("mergeOcrRows：以 ocr_status 资产顺序为准并合并 models_list 元数据", () => {
  const rows = mergeOcrRows(status(), [
    model("ocr-rec", { installed: true, installedBytes: 2048 }),
    model("ocr-det"),
  ]);
  assert.deepEqual(
    rows.map((row) => row.id),
    ["ocr-det", "ocr-rec", "ocr-cls", "ocr-dict"],
  );
  assert.equal(rows[0].model?.id, "ocr-det");
  assert.equal(rows[1].model?.installed, true);
  assert.equal(rows[2].model, null);
});

test("mergeOcrRows：资产缺失时用 onnx 模型补位（按 models_list 顺序）", () => {
  const partial = status({ assets: [asset("ocr-det")] });
  const rows = mergeOcrRows(partial, [model("ocr-rec"), model("ocr-dict")]);
  assert.deepEqual(
    rows.map((row) => row.id),
    ["ocr-det", "ocr-rec", "ocr-dict"],
  );
  assert.equal(rows[0].name, "资产 ocr-det");
  assert.equal(rows[1].name, "模型 ocr-rec");
  assert.equal(rows[1].bundled, false);
});

test("mergeOcrRows：status 为 null 时退化为 models_list", () => {
  const rows = mergeOcrRows(null, [model("ocr-det")]);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].installed, false);
});

test("summarizeOcr：未读取 / 就绪 / 随包内置 / 缺运行时 / 缺模型 / 其它", () => {
  assert.equal(summarizeOcr(null).tone, "neutral");

  const ready = summarizeOcr(status());
  assert.equal(ready.tone, "ok");
  assert.equal(ready.label, "OCR 已就绪");

  const bundled = summarizeOcr(
    status({ bundled: true, assets: [asset("ocr-det", { bundled: true })] }),
  );
  assert.equal(bundled.tone, "ok");
  assert.equal(bundled.label, "OCR 已就绪（随包内置）");

  const noRuntime = summarizeOcr(
    status({ ready: false, runtimeReady: false, missing: [], message: "缺少 onnxruntime.dll" }),
  );
  assert.equal(noRuntime.tone, "danger");
  assert.equal(noRuntime.label, "缺少 ONNX 运行时");
  assert.equal(noRuntime.detail, "缺少 onnxruntime.dll");

  const slim = summarizeOcr(
    status({
      ready: false,
      assets: [asset("ocr-det", { installed: false })],
      missing: ["ocr-det", "ocr-rec"],
    }),
  );
  assert.equal(slim.tone, "warn");
  assert.equal(slim.label, "缺少 2 个 OCR 模型");
  assert.match(slim.detail, /ocr-det、ocr-rec/);

  const other = summarizeOcr(status({ ready: false, runtimeReady: true, missing: [] }));
  assert.equal(other.tone, "warn");
  assert.equal(other.label, "OCR 未就绪");
});

test("pendingOcrRows / allBundled：决定下载入口与内置徽标", () => {
  const rows = [
    { id: "a", name: "a", installed: true, bundled: false, sizeBytes: 1, model: model("a") },
    { id: "b", name: "b", installed: false, bundled: false, sizeBytes: 1, model: model("b") },
    { id: "c", name: "c", installed: false, bundled: true, sizeBytes: 1, model: null },
  ];
  assert.deepEqual(
    pendingOcrRows(rows).map((row) => row.id),
    ["b"],
  );
  assert.equal(allBundled(rows), false);
  assert.equal(allBundled(rows.map((row) => ({ ...row, bundled: true }))), true);
  assert.equal(allBundled([]), false);
});
