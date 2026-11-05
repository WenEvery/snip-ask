/**
 * S-17 补充：**真实 DeepSeek** `provider_test` 证据（Key 来自 `.verify/p1/key.txt`，不落盘、不打印）。
 * 同时补充 S-13 前端现状实机取证：设置 → 翻译/模型库的静态内存文案 + `navigator.deviceMemory` 实际值。
 *
 * 与 mock 矩阵（run_s17.mjs）的区别：本脚本只打真实外网 1 次，用于证明
 * 「真实可达（200）+ 真实错误映射（401/402/429 等由 mock 矩阵覆盖）」两类证据齐全。
 *
 * 安全约束：
 *   - Key 只在运行时从 `.verify/p1/key.txt` 读入内存，仅作为 `provider_test` 的 `draft.apiKey` 传入；
 *   - **不打印 Key、不写入任何证据文件**；聚合 JSON 只记录 `apiKeyProvided: true` 与脱敏 host；
 *   - 本构建 `AppState` 的 CredentialStore 是 `InMemoryCredentialStore`（不写 Windows 凭据管理器），
 *     测试成功后 Key 只存在于该进程内存，应用退出即丢失。
 *
 * 用法（仓库根目录；需 app 独占）：
 *   node scripts/spikes/s13_s17_s19_s20/run_s17_real.mjs [--attach] [--keep]
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  JS_INSTALL_UNCAUGHT,
  JS_READ_UNCAUGHT,
  REPO,
  applabStart,
  applabStop,
  evalWin,
  saveDocsData,
  saveRaw,
  shot,
  sleep,
  waitFor,
} from "./lib.mjs";

const FLAGS = new Set(process.argv.slice(2));
const ATTACH = FLAGS.has("--attach");
const KEEP = FLAGS.has("--keep");
const KEY_FILE = join(REPO, ".verify", "p1", "key.txt");

function readKey() {
  if (!existsSync(KEY_FILE)) return null;
  const text = readFileSync(KEY_FILE, "utf8").trim();
  return text.length > 8 ? text : null;
}

const JS_REAL_PROVIDER_TEST = (key) => `(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const draft = {
    id: "deepseek",
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com",
    model: "deepseek-flash",
    caps: ["vision", "search", "thinking"],
    apiKey: ${JSON.stringify(key)},
  };
  const started = performance.now();
  try {
    const result = await invoke("provider_test", { draft });
    return { threw: false, wallMs: Math.round(performance.now() - started), result, apiKeyProvided: true };
  } catch (error) {
    return { threw: true, wallMs: Math.round(performance.now() - started), error, apiKeyProvided: true };
  }
})()`;

const JS_SET_SKELETON = (enabled) => `(async () => {
  const view = await window.__TAURI_INTERNALS__.invoke("config_patch", { patch: { advanced: { skeletonMode: ${enabled} } } });
  return { skeletonMode: view.advanced.skeletonMode, providers: view.providers.map((p) => ({ id: p.id, baseUrl: p.baseUrl, hasKey: p.hasKey })) };
})()`;

const JS_APP_ALIVE = `(async () => {
  const view = await window.__TAURI_INTERNALS__.invoke("config_get");
  return { alive: true, skeletonMode: view.advanced.skeletonMode };
})()`;

const JS_DEVICE_MEMORY = `(() => ({
  deviceMemory: typeof navigator.deviceMemory === "number" ? navigator.deviceMemory : null,
  hardwareConcurrency: navigator.hardwareConcurrency ?? null,
  userAgent: navigator.userAgent,
}))()`;

const JS_CLICK_NAV = (label) => `(() => {
  const button = [...document.querySelectorAll(".main-nav__item")].find((b) => (b.textContent || "").includes(${JSON.stringify(label)}));
  if (!button) return { clicked: false };
  button.click();
  return { clicked: true, label: button.textContent.trim() };
})()`;

const JS_CLICK_SETTINGS_SECTION = (label) => `(() => {
  const button = [...document.querySelectorAll(".settings-nav-item")].find((b) => (b.textContent || "").includes(${JSON.stringify(label)}));
  if (!button) return { clicked: false };
  button.click();
  return { clicked: true, label: button.textContent.trim() };
})()`;

const JS_READ_MODEL_LIBRARY = `(() => {
  const rows = [...document.querySelectorAll(".sa-model-row")].map((row) => ({
    title: row.querySelector(".sa-model-title")?.innerText?.trim() ?? null,
    meta: row.querySelector(".sa-model-meta")?.innerText?.trim() ?? null,
  }));
  const hint = document.querySelector(".settings-root .sa-hint")?.innerText ?? null;
  return { rows, hint, sectionText: (document.querySelector(".settings-content, .settings-root")?.innerText ?? "").slice(0, 1200) };
})()`;

async function main() {
  const key = readKey();
  const evidence = {
    kind: "real-provider-test + s13-frontend-probe",
    date: new Date().toISOString(),
    keySource: ".verify/p1/key.txt",
    providerTest: null,
    deviceMemory: null,
    modelLibrary: null,
    uncaught: null,
    appAlive: null,
    note: "证据文件不含 Key；providerTest 只记录 ok/status/latencyMs/model/error.code 与 apiKeyProvided=true",
  };
  try {
    if (!ATTACH) {
      const start = applabStart({ reset: false, waitSec: 300 });
      if (start.code !== 0) throw new Error(`applab start 失败：${start.stderr || start.stdout}`);
    }
    await waitFor("main", `typeof window.__TAURI_INTERNALS__ !== "undefined"`, "Tauri IPC 就绪", 30000);
    evalWin("main", JS_INSTALL_UNCAUGHT);
    evalWin("main", `window.__TAURI_INTERNALS__.invoke("window_show", { window: "main" }).then(() => true)`);
    await sleep(500);

    // ---- 1) 真实 DeepSeek provider_test（skeletonMode=false + 真实 Key）----
    const patchOn = evalWin("main", JS_SET_SKELETON(false));
    evidence.configPatch = patchOn;
    if (key) {
      const real = evalWin("main", JS_REAL_PROVIDER_TEST(key));
      evidence.providerTest = {
        threw: real.threw,
        wallMs: real.wallMs,
        apiKeyProvided: true,
        host: "api.deepseek.com",
        ok: real.result?.ok ?? null,
        status: real.result?.status ?? null,
        latencyMs: real.result?.latencyMs ?? null,
        model: real.result?.model ?? null,
        error: real.result
          ? real.result.error
            ? { code: real.result.error.code, retryable: real.result.error.retryable, fallback: real.result.error.fallback, detail: real.result.error.detail }
            : null
          : real.error ?? null,
      };
      console.log(
        `[s17-real] provider_test(DeepSeek): ok=${evidence.providerTest.ok} status=${evidence.providerTest.status} latencyMs=${evidence.providerTest.latencyMs} model=${evidence.providerTest.model}`,
      );
      saveRaw("s17", "provider-test-real-deepseek.json", evidence.providerTest);
    } else {
      evidence.providerTest = { skipped: true, reason: ".verify/p1/key.txt 不存在或内容过短" };
      console.warn("[s17-real] 未找到可用 Key，跳过真实 provider_test（不伪造证据）");
    }
    // 恢复骨架模式，避免影响后续成员/复跑
    evidence.configRestore = evalWin("main", JS_SET_SKELETON(true));

    // ---- 2) S-13 前端现状实机取证：navigator.deviceMemory + 设置→翻译/模型库静态内存文案 ----
    evidence.deviceMemory = evalWin("main", JS_DEVICE_MEMORY);
    evalWin("main", JS_CLICK_NAV("设置"));
    await waitFor("main", `!!document.querySelector(".settings-nav-item")`, "设置页渲染", 20000);
    evalWin("main", JS_CLICK_SETTINGS_SECTION("翻译"));
    await waitFor("main", `!!document.querySelector(".settings-root")`, "翻译设置渲染", 20000);
    await sleep(800);
    // 若模型库在翻译分组内，直接读取；否则按可下载模型行读取。
    const library = (() => {
      try {
        return evalWin("main", JS_READ_MODEL_LIBRARY);
      } catch {
        return null;
      }
    })();
    evidence.modelLibrary = library;
    shot("main", "s13/01-settings-translate-model-library.png");
    // 引导第 3 步（三档）静态文案也从设置页无法复现，引用自动走查截图（shots/s19/step3-*）。
    evidence.uncaught = evalWin("main", JS_READ_UNCAUGHT);
    evidence.appAlive = evalWin("main", JS_APP_ALIVE);
    saveRaw("s17", "real-provider-and-s13-ui.json", evidence);
    saveDocsData("s17_real_provider.json", {
      kind: evidence.kind,
      date: evidence.date,
      keySource: evidence.keySource,
      providerTest: evidence.providerTest,
      deviceMemory: {
        deviceMemory: evidence.deviceMemory?.deviceMemory ?? null,
        hardwareConcurrency: evidence.deviceMemory?.hardwareConcurrency ?? null,
        note: "Chromium 对 deviceMemory 上限报 8（本机 16GB 报 8，8GB 机型会报 4），因此前端不能直接拿它和 minRamMB 比，需要用「< 8 视为低内存」这类阈值口径",
      },
      modelLibrary: {
        rows: evidence.modelLibrary?.rows ?? null,
        hint: evidence.modelLibrary?.hint ?? null,
      },
      uncaught: evidence.uncaught,
      appAlive: evidence.appAlive,
    });
    console.log(`[s17-real] deviceMemory=${evidence.deviceMemory?.deviceMemory} 模型库行数=${evidence.modelLibrary?.rows?.length ?? "?"}`);
  } finally {
    if (!ATTACH && !KEEP) {
      applabStop();
      await sleep(800);
    }
    console.log("[s17-real] 应用已停止");
  }
}

main().catch(async (error) => {
  console.error(`[s17-real] 失败：${error.message}`);
  saveRaw("s17", "real-failure.json", { at: new Date().toISOString(), message: String(error.message ?? error) });
  if (!ATTACH && !KEEP) {
    try {
      applabStop();
    } catch {
      /* ignore */
    }
  }
  process.exit(1);
});
