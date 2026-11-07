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
  isOcrLanguage,
  isOcrLanguagePackId,
  languageInstalled,
  languagePackModel,
  languagePackSize,
  mergeOcrRows,
  ocrLanguageLabel,
  ocrLanguageOptions,
  ocrLanguagePackId,
  ocrModels,
  pendingLanguagePacks,
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
  assert.equal(editionLabel("full"), "已内置文字识别模型");
  assert.equal(editionLabel("slim"), "首次使用时下载文字识别模型");
  assert.equal(editionLabel("dev"), "本地运行");
  assert.equal(editionLabel(null), "未知");
  assert.equal(editionLabel("weird"), "未知");
  assert.equal(editionBadge("full"), "内置模型");
  assert.equal(editionBadge("slim"), "在线下载");
  assert.equal(editionBadge("dev"), "本地运行");
  assert.equal(editionBadge(undefined), "未知");
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
  assert.equal(ready.label, "文字识别已就绪");

  const bundled = summarizeOcr(
    status({ bundled: true, assets: [asset("ocr-det", { bundled: true })] }),
  );
  assert.equal(bundled.tone, "ok");
  assert.equal(bundled.label, "文字识别已就绪（随安装包内置）");

  const noRuntime = summarizeOcr(
    status({ ready: false, runtimeReady: false, missing: [], message: "缺少 onnxruntime.dll" }),
  );
  assert.equal(noRuntime.tone, "danger");
  assert.equal(noRuntime.label, "缺少识别引擎组件");
  assert.equal(noRuntime.detail, "缺少 onnxruntime.dll");

  const slim = summarizeOcr(
    status({
      ready: false,
      assets: [asset("ocr-det", { installed: false })],
      missing: ["ocr-det", "ocr-rec"],
    }),
  );
  assert.equal(slim.tone, "warn");
  assert.equal(slim.label, "还缺 2 个模型");
  assert.match(slim.detail, /16 MB/);

  const other = summarizeOcr(status({ ready: false, runtimeReady: true, missing: [] }));
  assert.equal(other.tone, "warn");
  assert.equal(other.label, "文字识别暂不可用");
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

/* ───────────────── 识别语言 / 语言包（契约 v7 增量） ───────────────── */

function languageStatus(language, installed) {
  return { language, installed };
}

function langStatus(overrides = {}) {
  return status({
    language: "zh",
    languages: [
      languageStatus("zh", true),
      languageStatus("ru", false),
      languageStatus("ja", false),
      languageStatus("ko", false),
    ],
    ...overrides,
  });
}

test("ocrLanguageLabel / id 映射：zh 无语言包，ru/ja/ko 固定资产 id", () => {
  assert.equal(ocrLanguageLabel("zh"), "中文（含英文）");
  assert.equal(ocrLanguageLabel("ru"), "俄语");
  assert.equal(ocrLanguageLabel("ja"), "日语");
  assert.equal(ocrLanguageLabel("ko"), "韩语");
  assert.equal(ocrLanguagePackId("zh"), null);
  assert.equal(ocrLanguagePackId("ru"), "ocr-lang-ru");
  assert.equal(ocrLanguagePackId("ja"), "ocr-lang-ja");
  assert.equal(ocrLanguagePackId("ko"), "ocr-lang-ko");
  assert.equal(isOcrLanguage("zh"), true);
  assert.equal(isOcrLanguage("de"), false);
  assert.equal(isOcrLanguagePackId("ocr-lang-ru"), true);
  assert.equal(isOcrLanguagePackId("ocr-rec"), false);
});

test("ocrModels：语言包 ONNX 不混入基础 4 项资产", () => {
  const models = [
    model("ocr-det"),
    model("ocr-lang-ru"),
    model("ocr-lang-ja"),
    model("ocr-lang-ko"),
  ];
  assert.deepEqual(
    ocrModels(models).map((item) => item.id),
    ["ocr-det"],
  );
});

test("languageInstalled：zh 恒可用；状态缺失不限制；按 languages.installed 判定", () => {
  assert.equal(languageInstalled(null, "zh"), true);
  assert.equal(languageInstalled(langStatus(), "zh"), true);
  // 旧后端 / 状态未加载：没有 languages 字段时不把其它语言全部禁用
  assert.equal(languageInstalled(status(), "ru"), true);
  const current = langStatus({
    languages: [
      languageStatus("zh", true),
      languageStatus("ru", true),
      languageStatus("ja", false),
    ],
  });
  assert.equal(languageInstalled(current, "ru"), true);
  assert.equal(languageInstalled(current, "ja"), false);
  assert.equal(languageInstalled(current, "ko"), false);
});

test("pendingLanguagePacks：只列出未安装的非中文语言包", () => {
  assert.deepEqual(
    pendingLanguagePacks(langStatus()).map((item) => item.language),
    ["ru", "ja", "ko"],
  );
  assert.deepEqual(pendingLanguagePacks(null), []);
  assert.deepEqual(
    pendingLanguagePacks(
      langStatus({
        languages: [
          languageStatus("zh", true),
          languageStatus("ru", true),
          languageStatus("ja", false),
          languageStatus("ko", false),
        ],
      }),
    ).map((item) => item.language),
    ["ja", "ko"],
  );
});

test("ocrLanguageOptions：未下载语言后缀「（未下载）」并禁用", () => {
  const options = ocrLanguageOptions(langStatus());
  assert.deepEqual(
    options.map((option) => option.value),
    ["zh", "ru", "ja", "ko"],
  );
  assert.deepEqual(
    options.map((option) => option.disabled),
    [false, true, true, true],
  );
  assert.equal(options[0].label, "中文（含英文）");
  assert.equal(options[1].label, "俄语（未下载）");

  const allInstalled = ocrLanguageOptions(
    langStatus({
      languages: [
        languageStatus("zh", true),
        languageStatus("ru", true),
        languageStatus("ja", true),
        languageStatus("ko", true),
      ],
    }),
  );
  assert.deepEqual(
    allInstalled.map((option) => option.disabled),
    [false, false, false, false],
  );
});

test("languagePackModel / languagePackSize：优先 models_list，缺失用契约大小兜底", () => {
  assert.equal(languagePackSize([], "zh"), 0);
  assert.equal(languagePackSize([], "ru"), 8_074_092);
  assert.equal(languagePackSize([], "ja"), 9_753_335);
  assert.equal(languagePackSize([], "ko"), 13_488_748);
  const models = [model("ocr-lang-ru", { sizeBytes: 123 })];
  assert.equal(languagePackModel(models, "ru")?.id, "ocr-lang-ru");
  assert.equal(languagePackSize(models, "ru"), 123);
  assert.equal(languagePackModel(models, "ko"), null);
});
