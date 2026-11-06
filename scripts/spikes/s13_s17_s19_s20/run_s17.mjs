/**
 * S-17 网络异常与额度降级 · 真实应用验证驱动。
 *
 * 做什么：
 *   1) 起本地 mock HTTP 服务（401/402/429/5xx/超时/非 JSON/慢响应），
 *   2) 在真实 Tauri 应用（applab 便携实例）里通过 IPC `provider_test` 逐条核对
 *      `TestResult.error` 的 code / retryable / fallback（契约 §8.2），
 *   3) 断网场景（127.0.0.1 未监听端口 / DNS 失败 / 连接超时）与降级链：
 *      真实 `capture_freeze` → overlay 拖拽 → `menu:ready` 能力/禁用原因，
 *      以及设置页「测试连接」的真实错误 UI 截图，
 *   4) 全程收集 webview 未捕获异常与 Rust 日志，确认应用不崩、无 uncaught。
 *
 * 用法（仓库根目录；应用独占，先 `applab reserve --owner app-runtime`）：
 *   node scripts/spikes/s13_s17_s19_s20/run_s17.mjs [--attach] [--keep]
 *
 * 证据：
 *   - 原始数据 `.verify/spikes/S-13-S-17-S-19-S-20/raw/s17/`
 *   - 截图 `.../shots/s17/`
 *   - 聚合 `docs/spikes/data/s13_s17_s19_s20/s17_error_matrix.json`
 *
 * 安全：本脚本**不使用任何真实 API Key**。mock 用假 Key `sk-s17-mock-not-real-key`，
 * 且本构建 `AppState` 的 CredentialStore 是 `InMemoryCredentialStore`（不落 Windows 凭据管理器）。
 */
import { spawn } from "node:child_process";
import { readFileSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import {
  EVID,
  JS_INSTALL_UNCAUGHT,
  JS_READ_UNCAUGHT,
  REPO,
  applabStart,
  applabStop,
  evalWin,
  listAppPids,
  saveDocsData,
  saveRaw,
  shot,
  sleep,
  waitFor,
} from "./lib.mjs";

const FLAGS = new Set(process.argv.slice(2));
const ATTACH = FLAGS.has("--attach");
const KEEP = FLAGS.has("--keep");
const MOCK_PORT = 39117;
const CLOSED_PORT = 39199; // 不监听：模拟断网 / 连接拒绝
const FAKE_KEY = "sk-s17-mock-not-real-key";

const MOCK_LOG = join(EVID, "raw", "s17", "mock-requests.jsonl");

function startMock() {
  rmSync(MOCK_LOG, { force: true });
  const child = spawn(process.execPath, [
    join(REPO, "scripts", "spikes", "s13_s17_s19_s20", "mock_provider.mjs"),
    "--port",
    String(MOCK_PORT),
    "--log",
    MOCK_LOG,
  ]);
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  return new Promise((resolve, reject) => {
    let out = "";
    const timer = setTimeout(() => reject(new Error(`mock 服务未就绪：${out.slice(-400)}`)), 15000);
    child.stdout.on("data", (chunk) => {
      out += chunk;
      if (out.includes("MOCK_READY")) {
        clearTimeout(timer);
        resolve({ child, readyLine: out.trim() });
      }
    });
    child.stderr.on("data", (chunk) => {
      out += chunk;
    });
    child.on("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`mock 服务提前退出 code=${code}：${out.slice(-400)}`));
    });
  });
}

// ---------------------------------------------------------------------------
// 页面内脚本
// ---------------------------------------------------------------------------

const JS_PATCH = (patchJson) => `(async () => {
  const view = await window.__TAURI_INTERNALS__.invoke("config_patch", { patch: ${patchJson} });
  return {
    skeletonMode: view.advanced.skeletonMode,
    providers: view.providers.map((p) => ({ id: p.id, baseUrl: p.baseUrl, hasKey: p.hasKey, enabled: p.enabled })),
    translateBackend: view.translate.backend,
  };
})()`;

/** 逐条跑 provider_test 矩阵；返回每条结果与 wall/ latency。 */
const JS_PROVIDER_MATRIX = (casesJson, fakeKey) => `(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const cases = ${casesJson};
  const results = [];
  for (const item of cases) {
    const draft = {
      id: "deepseek",
      name: "DeepSeek",
      baseUrl: item.baseUrl,
      model: item.model ?? "deepseek-flash",
      caps: ["vision", "search", "thinking"],
      apiKey: item.noKey ? null : ${JSON.stringify(fakeKey)},
    };
    const started = performance.now();
    let record = { name: item.name, expect: item.expect, draft: { ...draft, apiKey: draft.apiKey ? "<fake-key>" : null } };
    try {
      const result = await invoke("provider_test", { draft });
      record.threw = false;
      record.result = result;
      record.wallMs = Math.round(performance.now() - started);
    } catch (error) {
      record.threw = true;
      record.error = error;
      record.wallMs = Math.round(performance.now() - started);
    }
    results.push(record);
  }
  return results;
})()`;

const JS_INSTALL_EVENTS = `(async () => {
  const internals = window.__TAURI_INTERNALS__;
  window.__s17Events = window.__s17Events ?? {};
  const listen = (event, bucket) => {
    window.__s17Events[bucket] = window.__s17Events[bucket] ?? [];
    const handler = internals.transformCallback((payload) => {
      window.__s17Events[bucket].push({ at: Date.now(), payload });
    });
    return internals
      .invoke("plugin:event|listen", { event, target: { kind: "Any" }, handler })
      .then(() => true)
      .catch((error) => String(error));
  };
  const out = {};
  out.menuReady = await listen("menu:ready", "menuReady");
  out.taskError = await listen("task:error", "taskError");
  out.taskDone = await listen("task:done", "taskDone");
  return out;
})()`;

const JS_READ_MENU_PAYLOADS = `(() => {
  const buckets = window.__s17Events?.menuReady ?? [];
  return buckets.map((entry) => entry.payload?.payload ?? entry.payload);
})()`;

const JS_TRIGGER_CAPTURE = `(async () => {
  const session = await window.__TAURI_INTERNALS__.invoke("capture_freeze");
  return { session };
})()`;

const JS_READ_MENU_DOM = `(() => {
  const items = [...document.querySelectorAll(".overlay-menu__action")].map((button) => {
    const reasonNode = button.querySelector('[id^="overlay-action-reason-"]');
    return {
      label: button.querySelector(".overlay-menu__action-label")?.textContent?.trim() ?? null,
      disabled: button.disabled,
      disabledReason: reasonNode ? reasonNode.textContent.trim() : null,
      mode: button.querySelector(".overlay-menu__action-mode")?.textContent?.trim() ?? null,
    };
  });
  return {
    menuPresent: !!document.querySelector(".overlay-menu"),
    items,
    preview: document.querySelector(".overlay-menu__preview")?.textContent?.trim() ?? null,
  };
})()`;

/** 通过 overlay CDP 真实拖拽，让 overlay 自己 commitSelection → 后端 selection_commit → menu:ready。 */
async function overlayDrag(cdpPort = 9333) {
  const targets = await fetch(`http://127.0.0.1:${cdpPort}/json/list`).then((response) => response.json());
  const target = targets.find((entry) => String(entry.url ?? "").includes("window=overlay"));
  if (!target) throw new Error("CDP targets 中没有 window=overlay");
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve);
    socket.addEventListener("error", () => reject(new Error("overlay CDP 连接失败")));
  });
  let nextId = 0;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(JSON.stringify(message.error)));
      else resolve(message.result);
    }
  });
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      nextId += 1;
      pending.set(nextId, { resolve, reject });
      socket.send(JSON.stringify({ id: nextId, method, params }));
    });
  const start = { x: 420, y: 320 };
  const end = { x: 900, y: 560 };
  try {
    await send("Runtime.enable");
    await send("Input.dispatchMouseEvent", { type: "mousePressed", x: start.x, y: start.y, button: "left", buttons: 1, clickCount: 1 });
    await sleep(120);
    for (let step = 1; step <= 6; step += 1) {
      await send("Input.dispatchMouseEvent", {
        type: "mouseMoved",
        x: start.x + ((end.x - start.x) * step) / 6,
        y: start.y + ((end.y - start.y) * step) / 6,
        button: "left",
        buttons: 1,
      });
      await sleep(40);
    }
    await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: end.x, y: end.y, button: "left", buttons: 0, clickCount: 1 });
    return { start, end };
  } finally {
    socket.close();
  }
}

const JS_CLICK_NAV = (label) => `(() => {
  const button = [...document.querySelectorAll(".main-nav__item")].find((b) => (b.textContent || "").includes(${JSON.stringify(label)}));
  if (!button) return { clicked: false, reason: "找不到导航项" };
  button.click();
  return { clicked: true, label: button.textContent.trim() };
})()`;

const JS_CLICK_SETTINGS_SECTION = (label) => `(() => {
  const button = [...document.querySelectorAll(".settings-nav-item")].find((b) => (b.textContent || "").includes(${JSON.stringify(label)}));
  if (!button) return { clicked: false, reason: "找不到设置分组" };
  button.click();
  return { clicked: true, label: button.textContent.trim() };
})()`;

const JS_UI_TEST_PROVIDER = `(() => {
  const card = document.querySelector(".sa-provider-card");
  if (!card) return { ok: false, reason: "AI 服务页未渲染供应商卡片" };
  const expand = [...card.querySelectorAll("button")].find((b) => /展开编辑/.test(b.textContent || ""));
  if (expand) expand.click();
  const buttons = [...card.querySelectorAll("button")].filter((b) => /测试连接/.test(b.textContent || ""));
  if (buttons.length === 0) return { ok: false, reason: "找不到测试连接按钮" };
  buttons[0].click();
  return { ok: true, expanded: !!expand, label: buttons[0].textContent.trim() };
})()`;

const JS_READ_PROVIDER_UI = `(() => {
  const bad = document.querySelector(".sa-testresult.is-bad, .sa-testline.is-bad");
  return {
    errorVisible: !!bad,
    errorText: bad ? bad.innerText.trim() : null,
    bodyText: (document.querySelector(".settings-root")?.innerText ?? "").slice(0, 2000),
  };
})()`;

const JS_APP_ALIVE = `(async () => {
  const view = await window.__TAURI_INTERNALS__.invoke("config_get");
  return { alive: true, skeletonMode: view.advanced.skeletonMode, providerCount: view.providers.length };
})()`;

// ---------------------------------------------------------------------------

function buildCases() {
  const base = `http://127.0.0.1:${MOCK_PORT}`;
  // 顺序敏感：`err_no_key` 必须在任何成功用例之前（成功用例会把假 Key 写入**进程内**凭据存储，
  // 之后的「无 Key」用例会回退到该 Key，从而测不到缺 Key 分支）。
  return [
    { name: "err_no_key", baseUrl: `${base}/c/200`, noKey: true, expect: "ok=false/SA-3002（不发网络请求）" },
    { name: "err_empty_model", baseUrl: `${base}/c/200`, model: "", expect: "IPC 拒绝 SA-5001（model 非空）" },
    { name: "err_non_https_rejected", baseUrl: "http://api.deepseek.com", expect: "IPC 拒绝 SA-5001（baseUrl 白名单）" },
    { name: "success_200", baseUrl: `${base}/c/200`, expect: "ok=true/status=200/model=mock-model-a" },
    { name: "success_200_slow_3s", baseUrl: `${base}/c/slow`, expect: "ok=true/status=200/latency>=3000ms" },
    { name: "success_200_non_json_body", baseUrl: `${base}/c/200badjson`, expect: "ok=true/status=200/model 回退配置值" },
    { name: "success_200_empty_models", baseUrl: `${base}/c/empty`, expect: "ok=true/status=200/model 回退配置值" },
    { name: "success_trailing_slash", baseUrl: `${base}/c/200///`, expect: "规范化到 /c/200/models → ok=true" },
    { name: "err_401", baseUrl: `${base}/c/401`, expect: "SA-3002/retryable=false/open_settings" },
    { name: "err_403", baseUrl: `${base}/c/403`, expect: "SA-3002/retryable=false/open_settings" },
    { name: "err_402", baseUrl: `${base}/c/402`, expect: "SA-3003/retryable=false/open_settings" },
    { name: "err_429", baseUrl: `${base}/c/429`, expect: "SA-3004/retryable=true/retry" },
    { name: "err_500", baseUrl: `${base}/c/500`, expect: "SA-3001/retryable=true/retry" },
    { name: "err_503", baseUrl: `${base}/c/503`, expect: "SA-3001/retryable=true/retry" },
    { name: "err_404", baseUrl: `${base}/c/404`, expect: "SA-5001/retryable=false/reset_group" },
    { name: "err_400", baseUrl: `${base}/c/400`, expect: "SA-5001/retryable=false/reset_group" },
    { name: "err_408", baseUrl: `${base}/c/408`, expect: "SA-5001/retryable=false/reset_group" },
    { name: "err_timeout_hang", baseUrl: `${base}/c/hang`, expect: "SA-3005/retryable=true/retry+browser_search（约 15s）" },
    { name: "err_dns_failure", baseUrl: "https://nonexistent.invalid", expect: "SA-3001/retryable=true/browser_search+local_translate" },
    { name: "err_connection_refused", baseUrl: `http://127.0.0.1:${CLOSED_PORT}`, expect: "SA-3001/retryable=true/browser_search+local_translate" },
  ];
}

/** config_patch 的 providers 是**数组整体替换**（非按 id 合并）：必须给完整对象，否则 model/caps 会被清空。 */
function fullProvider(baseUrl) {
  return {
    id: "deepseek",
    name: "DeepSeek",
    baseUrl,
    model: "deepseek-flash",
    caps: ["vision", "search", "thinking"],
    temperature: 0.2,
    maxTokens: 1024,
    enabled: true,
  };
}

function classify(record) {
  const result = record.result ?? null;
  const error = result?.error ?? record.error ?? null;
  return {
    name: record.name,
    expect: record.expect,
    threw: record.threw,
    ok: result?.ok ?? null,
    status: result?.status ?? null,
    latencyMs: result?.latencyMs ?? null,
    model: result?.model ?? null,
    wallMs: record.wallMs,
    code: error?.code ?? null,
    retryable: error?.retryable ?? null,
    fallback: error?.fallback ?? null,
    message: error?.message ?? null,
    detail: error?.detail ?? error?.details ?? null,
    actionableHint: error?.actionableHint ?? null,
  };
}

async function main() {
  const commit = (() => {
    try {
      const text = readFileSync(join(REPO, ".git", "HEAD"), "utf8").trim();
      return text.startsWith("ref: ") ? readFileSync(join(REPO, ".git", text.slice(5)), "utf8").trim() : text;
    } catch {
      return "unknown";
    }
  })();

  let mock = null;
  const evidence = {
    kind: "real-app-ipc + local-mock-http",
    date: new Date().toISOString(),
    commit,
    env: {
      os: "Windows 11 26200",
      cpu: "i7-12650H (10C16T)",
      ramGiB: 15.78,
      screens: "2560x1440@150% (0,0) + 1080x1920@100% (-1080,-470)",
      app: "src-tauri/target/debug/snip-ask.exe（applab 便携副本）",
      mock: `http://127.0.0.1:${MOCK_PORT}（Node http，脚本 mock_provider.mjs）`,
      fakeKeyNote: "假 Key 仅用于 draft.apiKey；本构建 CredentialStore=InMemoryCredentialStore，不落 OS 凭据管理器",
    },
    configPatches: [],
    matrix: [],
    mockRequests: null,
    degradationMenu: null,
    uiError: null,
    uncaught: null,
    appAlive: null,
    rustLogIssues: null,
  };

  try {
    mock = await startMock();
    console.log(`[s17] mock 就绪：${mock.readyLine}`);
    if (!ATTACH) {
      const start = applabStart({ reset: false, waitSec: 300 });
      if (start.code !== 0) throw new Error(`applab start 失败：${start.stderr || start.stdout}`);
    }
    await waitFor("main", `typeof window.__TAURI_INTERNALS__ !== "undefined"`, "Tauri IPC 就绪", 30000);
    evalWin("main", JS_INSTALL_UNCAUGHT);
    evalWin("main", `window.__TAURI_INTERNALS__.invoke("window_show", { window: "main" }).then(() => true)`);
    await sleep(600);
    const events = evalWin("main", JS_INSTALL_EVENTS);
    console.log(`[s17] 事件监听：${JSON.stringify(events)}`);

    // ---- 1) 断网 + 无 Key 的降级链：真实 capture → overlay 拖拽 → menu:ready ----
    const offlinePatch = evalWin(
      "main",
      JS_PATCH(
        JSON.stringify({
          advanced: { skeletonMode: false },
          providers: [fullProvider(`http://127.0.0.1:${CLOSED_PORT}`)],
          translate: { backend: "local" },
        }),
      ),
    );
    evidence.configPatches.push({ stage: "offline-no-key", patch: offlinePatch });
    console.log(`[s17] 断网配置：${JSON.stringify(offlinePatch)}`);

    let menu = null;
    let menuError = null;
    try {
      const capture = evalWin("main", JS_TRIGGER_CAPTURE);
      await waitFor(
        "overlay",
        `document.readyState === "complete" && !!document.querySelector(".overlay-root") ? { ready: true } : null`,
        "overlay 页面就绪",
        20000,
      );
      await overlayDrag();
      const predicate = `(() => {
        const menu = document.querySelector(".overlay-menu");
        return menu ? { present: true, actions: document.querySelectorAll(".overlay-menu__action").length } : null;
      })()`;
      for (let attempt = 0; attempt < 2; attempt += 1) {
        try {
          await waitFor("overlay", predicate, "overlay 动作菜单", 12000);
          break;
        } catch (error) {
          if (attempt === 1) throw error;
          await overlayDrag();
        }
      }
      await sleep(400);
      const dom = evalWin("overlay", JS_READ_MENU_DOM);
      const payloads = evalWin("main", JS_READ_MENU_PAYLOADS);
      menu = { session: capture.session, dom, payloads, payloadCount: payloads.length };
      shot("overlay", "s17/01-overlay-menu-offline-no-key.png");
      shot("main", "s17/02-main-after-overlay.png");
      const cleanup = evalWin("main", `(async () => {
        try { await window.__TAURI_INTERNALS__.invoke("capture_cancel", { session: ${JSON.stringify(capture.session)} }); return { cancelled: true }; }
        catch (error) { return { cancelled: false, error }; }
      })()`);
      menu.cleanup = cleanup;
    } catch (error) {
      menuError = String(error.message ?? error);
      saveRaw("s17", "degradation-menu-error.json", { message: menuError });
    }
    evidence.degradationMenu = { error: menuError, ...(menu ?? {}) };
    if (menu) {
      const items = menu.dom.items;
      console.log(`[s17] menu:ready 动作=${items.map((item) => `${item.label}${item.disabled ? "(禁用)" : ""}`).join(" / ")}`);
      if (menu.payloadCount > 0) {
        saveRaw("s17", "menu-ready-payload.json", menu.payloads);
      }
    }

    // ---- 2) provider_test 错误码矩阵（真实 IPC，baseUrl 指向 mock/断网）----
    const cases = buildCases();
    const rawResults = evalWin("main", JS_PROVIDER_MATRIX(JSON.stringify(cases), FAKE_KEY));
    evidence.matrix = rawResults.map(classify);
    saveRaw("s17", "provider-matrix-raw.json", rawResults);
    for (const row of evidence.matrix) {
      const flag = row.threw ? "THREW" : row.ok ? "OK" : row.code;
      console.log(
        `[s17] ${row.name.padEnd(28)} ${String(flag).padEnd(8)} status=${String(row.status).padEnd(4)} latency=${String(
          row.latencyMs,
        ).padEnd(6)} retryable=${String(row.retryable).padEnd(5)} fallback=${JSON.stringify(row.fallback)}`,
      );
    }

    // ---- 3) mock 请求头证据（Bearer / UA / Accept）----
    try {
      const lines = readFileSync(MOCK_LOG, "utf8").trim().split("\n").filter(Boolean).map((line) => JSON.parse(line));
      const modelRequests = lines.filter((line) => line.scenario);
      evidence.mockRequests = {
        total: lines.length,
        modelsRequests: modelRequests.length,
        scenarios: [...new Set(modelRequests.map((line) => line.scenario))],
        authorizationPresentAll: modelRequests.every((line) => line.headers.authorizationPresent === true),
        authorizationScheme: [...new Set(modelRequests.map((line) => line.headers.authorizationScheme))],
        userAgentSample: modelRequests[0]?.headers.userAgent ?? null,
        acceptSample: modelRequests[0]?.headers.accept ?? null,
        sample: lines.slice(0, 6),
      };
      saveRaw("s17", "mock-requests-parsed.json", lines);
    } catch (error) {
      evidence.mockRequests = { error: String(error.message ?? error) };
    }

    // ---- 4) 设置页真实错误 UI：配置指向未监听端口 + 已保存（内存）假 Key → 真实网络错误 ----
    const uiPatch = evalWin(
      "main",
      JS_PATCH(
        JSON.stringify({
          advanced: { skeletonMode: false },
          providers: [fullProvider(`http://127.0.0.1:${CLOSED_PORT}`)],
        }),
      ),
    );
    evidence.configPatches.push({ stage: "offline-with-key", patch: uiPatch });
    try {
      evalWin("main", JS_CLICK_NAV("设置"));
      await waitFor("main", `!!document.querySelector(".settings-nav-item")`, "设置页渲染", 20000);
      evalWin("main", JS_CLICK_SETTINGS_SECTION("AI 服务"));
      await waitFor("main", `!!document.querySelector(".sa-provider-card")`, "AI 服务分组渲染", 20000);
      await sleep(500);
      const uiClick = evalWin("main", JS_UI_TEST_PROVIDER);
      if (uiClick.ok) {
        const ui = await waitFor(
          "main",
          `(() => {
            const bad = document.querySelector(".sa-testresult.is-bad, .sa-testline.is-bad");
            return bad ? { errorText: bad.innerText.trim() } : null;
          })()`,
          "UI 显示连接失败",
          30000,
        );
        const uiFull = evalWin("main", JS_READ_PROVIDER_UI);
        shot("main", "s17/03-settings-provider-test-network-error.png");
        evidence.uiError = { click: uiClick, ...ui, full: uiFull };
        console.log(`[s17] UI 错误提示：${ui.errorText.replace(/\s+/g, " ").slice(0, 160)}`);
      } else {
        evidence.uiError = { click: uiClick, error: "找不到测试连接按钮" };
      }
    } catch (error) {
      const dump = (() => {
        try {
          return evalWin("main", `(() => (document.querySelector(".sa-provider-card")?.innerText ?? "").slice(0, 1200))()`);
        } catch {
          return null;
        }
      })();
      evidence.uiError = { error: String(error.message ?? error), cardText: dump };
      shot("main", "s17/03-settings-provider-test-error-timeout.png");
      console.warn(`[s17] UI 错误路径未取到（已记录 DOM 快照）：${error.message}`);
    }

    // ---- 5) 应用存活 / 未捕获异常 / Rust 日志 ----
    evidence.appAlive = evalWin("main", JS_APP_ALIVE);
    evidence.uncaught = evalWin("main", JS_READ_UNCAUGHT);
    try {
      const logDir = join(REPO, ".verify", "applab", "app", "data", "logs");
      const files = readdirSync(logDir).filter((name) => name.startsWith("snipask.log"));
      const tails = files.map((name) => ({ name, text: readFileSync(join(logDir, name), "utf8") }));
      const issues = tails
        .flatMap((file) => file.text.split("\n").filter((line) => /panic|ERROR|热键注册失败|uncaught/i.test(line)))
        .slice(0, 20);
      evidence.rustLogIssues = { files, issueLines: issues };
    } catch (error) {
      evidence.rustLogIssues = { error: String(error.message ?? error) };
    }

    // ---- 6) 恢复便携配置（避免影响后续成员/复跑）----
    const restore = evalWin(
      "main",
      JS_PATCH(
        JSON.stringify({
          advanced: { skeletonMode: true },
          providers: [fullProvider("https://api.deepseek.com")],
          translate: { backend: "cloud" },
        }),
      ),
    );
    evidence.configPatches.push({ stage: "restore", patch: restore });

    saveRaw("s17", "provider-error-matrix.json", evidence);
    const pass = (row) => {
      const expected = {
        success_200: row.ok === true && row.status === 200,
        success_200_slow_3s: row.ok === true && (row.latencyMs ?? 0) >= 2500,
        success_200_non_json_body: row.ok === true && row.status === 200,
        success_200_empty_models: row.ok === true && row.status === 200,
        success_trailing_slash: row.ok === true && row.status === 200,
        err_401: row.code === "SA-3002" && row.retryable === false && (row.fallback ?? []).includes("open_settings"),
        err_403: row.code === "SA-3002" && row.retryable === false,
        err_402: row.code === "SA-3003" && row.retryable === false && (row.fallback ?? []).includes("open_settings"),
        err_429: row.code === "SA-3004" && row.retryable === true && (row.fallback ?? []).includes("retry"),
        err_500: row.code === "SA-3001" && row.retryable === true && (row.fallback ?? []).includes("retry"),
        err_503: row.code === "SA-3001" && row.retryable === true,
        err_404: row.code === "SA-5001" && row.retryable === false,
        err_400: row.code === "SA-5001" && row.retryable === false,
        err_408: row.code === "SA-5001" && row.retryable === false,
        err_timeout_hang: row.code === "SA-3005" && row.retryable === true && (row.fallback ?? []).includes("retry"),
        err_dns_failure: row.code === "SA-3001" && row.retryable === true && (row.fallback ?? []).includes("browser_search"),
        err_connection_refused: row.code === "SA-3001" && row.retryable === true && (row.fallback ?? []).includes("local_translate"),
        err_non_https_rejected: row.threw === true && row.code === "SA-5001",
        err_no_key: row.ok === false && row.code === "SA-3002" && (row.fallback ?? []).includes("open_settings"),
        err_empty_model: row.threw === true && row.code === "SA-5001",
      };
      return expected[row.name] ?? null;
    };
    const summary = {
      kind: evidence.kind,
      date: evidence.date,
      commit: evidence.commit,
      env: evidence.env,
      total: evidence.matrix.length,
      passed: evidence.matrix.filter((row) => pass(row) === true).length,
      rows: evidence.matrix.map((row) => ({ ...row, pass: pass(row) })),
      degradationMenu: {
        error: evidence.degradationMenu.error,
        items: evidence.degradationMenu.dom?.items ?? null,
        payloadCount: evidence.degradationMenu.payloadCount ?? 0,
        session: evidence.degradationMenu.session ?? null,
      },
      uiError: {
        visible: evidence.uiError?.errorVisible ?? null,
        text: evidence.uiError?.errorText ?? null,
        nav: evidence.uiError?.full?.errorText ?? null,
      },
      uncaught: evidence.uncaught,
      appAlive: evidence.appAlive,
      rustLogIssues: evidence.rustLogIssues?.issueLines ?? [],
      mockRequests: evidence.mockRequests,
    };
    saveDocsData("s17_error_matrix.json", summary);
    console.log(
      `[s17] 矩阵通过 ${summary.passed}/${summary.total}；未捕获异常=${
        (evidence.uncaught?.errors?.length ?? 0) + (evidence.uncaught?.rejections?.length ?? 0)
      }；应用存活=${evidence.appAlive?.alive === true}`,
    );
  } finally {
    if (!ATTACH && !KEEP) {
      applabStop();
      await sleep(800);
    }
    if (mock?.child) {
      mock.child.kill();
      await sleep(500);
    }
    const pids = listAppPids();
    console.log(`[s17] 清理：snip-ask 残留=${pids.length}；mock 已 kill=${!!mock}`);
  }
}

main().catch(async (error) => {
  console.error(`[s17] 失败：${error.message}`);
  saveRaw("s17", "failure.json", { at: new Date().toISOString(), message: String(error.message ?? error), stack: String(error.stack ?? "") });
  if (!ATTACH && !KEEP) {
    try {
      applabStop();
    } catch {
      /* ignore */
    }
  }
  process.exit(1);
});
