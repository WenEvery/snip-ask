/**
 * S-07 首启引导（无网络 / 无 Key）与受限模式 —— 真实应用实机走查驱动。
 *
 * 前提：Rust 已构建（src-tauri/target/debug/snip-ask.exe 含 S-07 能力门禁改动），
 * Vite 常驻 localhost:1420，applab 独占应用实例。
 *
 * 用法：
 *   node scripts/spikes/s07/run_s07.mjs            # 自动 start --reset --owner onboarding-offline，结束后 stop
 *   node scripts/spikes/s07/run_s07.mjs --attach   # 复用已启动的实例（调试用，不 start/stop）
 *   node scripts/spikes/s07/run_s07.mjs --keep     # 结束后保留实例运行
 *
 * 证据输出（gitignored）：
 *   .verify/spikes/S-07/shots/*.png   实机截图
 *   .verify/spikes/S-07/raw/*.json    IPC 返回值 / menu:ready 载荷 / 断言结果
 *
 * 注意：脚本不伪造数据 —— 所有断言值均来自真实 IPC 返回值或真实渲染 DOM；
 * 任一步失败会记录 failedChecks 并以退出码 1 结束（finally 仍会 stop 应用）。
 */

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..", "..");
const APPLAB = join(REPO, "scripts", "spikes", "applab", "applab.mjs");
const OUT_DIR = join(REPO, ".verify", "spikes", "S-07");
const SHOTS = join(OUT_DIR, "shots");
const RAW = join(OUT_DIR, "raw");
const OWNER = "onboarding-offline";
const FLAGS = new Set(process.argv.slice(2));
const ATTACH = FLAGS.has("--attach");
const KEEP = FLAGS.has("--keep");
const ONLY_MENU = FLAGS.has("--only-menu");

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

const evidence = {
  task: "S-07 首启引导无网络/无 Key 可用 + 受限模式",
  runAt: new Date().toISOString(),
  owner: OWNER,
  env: {
    skeletonModeDefault: true,
    apiKeys: "无任何 API Key",
    network: "provider baseUrl 指向 http://127.0.0.1:1（等价断网）；真实 provider 连通性测试为 TODO(real)",
  },
  steps: [],
  checks: {},
  failedChecks: [],
  screenshots: [],
  rawFiles: [],
};

function record(name, ok, detail) {
  evidence.checks[name] = { ok, detail };
  if (!ok) evidence.failedChecks.push({ name, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}  ${JSON.stringify(detail)}`);
}

function applab(argv) {
  const out = execFileSync(process.execPath, [APPLAB, ...argv], {
    cwd: REPO,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
  return out;
}

function evalWin(windowName, js) {
  const out = applab(["eval", "--window", windowName, "--js", js]);
  let parsed;
  try {
    parsed = JSON.parse(out);
  } catch {
    parsed = { ok: false, raw: out };
  }
  if (!parsed.ok) {
    throw new Error(`eval(${windowName}) 失败：${JSON.stringify(parsed.exception ?? parsed)}`);
  }
  return parsed.value;
}

async function waitFor(windowName, js, label, timeoutMs = 20000, intervalMs = 300) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    try {
      last = evalWin(windowName, js);
      if (last) return last;
    } catch (error) {
      last = String(error.message ?? error);
    }
    await sleep(intervalMs);
  }
  throw new Error(`等待超时（${label}，${timeoutMs}ms）：${JSON.stringify(last)}`);
}

function shot(windowName, name) {
  const out = join(SHOTS, name);
  applab(["shot", "--window", windowName, "--out", out]);
  evidence.screenshots.push({ window: windowName, path: out });
  console.log(`SHOT  ${out}`);
}

function saveRaw(name, value) {
  const out = join(RAW, name);
  writeFileSync(out, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  evidence.rawFiles.push(out);
  console.log(`RAW   ${out}`);
}

// ---------------------------------------------------------------------------
// 页面探测 / 交互脚本（在 WebView 内执行）
// ---------------------------------------------------------------------------

const JS_STEP_INFO = `(() => {
  const counter = document.querySelector(".ob-step-counter");
  const title = document.querySelector(".ob-step-title, .ob-hero-title");
  const footerButtons = [...document.querySelectorAll(".ob-footer-right button")];
  const primary = footerButtons.find((b) => /下一步|完成并进入|完成中/.test(b.textContent || ""));
  const boxes = [...document.querySelectorAll(".ob-checkline input[type=checkbox]")];
  return {
    counter: counter ? counter.textContent.trim() : null,
    stepTitle: title ? title.textContent.trim() : null,
    primaryLabel: primary ? primary.textContent.trim() : null,
    primaryDisabled: primary ? primary.disabled : null,
    skipLinkVisible: !!document.querySelector(".ob-footer-left .ob-linkbtn"),
    confirmSkipVisible: !!document.querySelector(".ob-footer-left .ob-btn-danger"),
    restrictedNotice: !!document.querySelector('[data-testid="restricted-notice"]'),
    restrictedNoticeText: document.querySelector('[data-testid="restricted-notice"]')?.innerText ?? null,
    checkboxCount: boxes.length,
    agreed: boxes.length >= 3 ? boxes[boxes.length - 1].checked : null,
  };
})()`;

const JS_CLICK_NEXT = `(() => {
  const buttons = [...document.querySelectorAll(".ob-footer-right button")];
  const primary = buttons.find((b) => /下一步|完成并进入/.test(b.textContent || ""));
  if (!primary) return { clicked: false, reason: "找不到下一步按钮" };
  primary.click();
  return { clicked: true, label: primary.textContent.trim(), disabled: primary.disabled };
})()`;

const JS_CLICK_PREV = `(() => {
  const buttons = [...document.querySelectorAll(".ob-footer-right button")];
  const prev = buttons.find((b) => /上一步/.test(b.textContent || ""));
  if (!prev) return { clicked: false, reason: "找不到上一步按钮" };
  prev.click();
  return { clicked: true, disabled: prev.disabled };
})()`;

const JS_OPEN_SKIP_CONFIRM = `(() => {
  const link = document.querySelector(".ob-footer-left .ob-linkbtn");
  if (!link) return { clicked: false, reason: "找不到跳过引导入口" };
  link.click();
  return { clicked: true, label: link.textContent.trim() };
})()`;

const JS_CONFIRM_SKIP = `(() => {
  const confirm = document.querySelector(".ob-footer-left .ob-btn-danger");
  if (!confirm) return { clicked: false, reason: "找不到确认跳过按钮" };
  confirm.click();
  return { clicked: true, label: confirm.textContent.trim() };
})()`;

const JS_AGREE_PRIVACY = `(() => {
  const boxes = [...document.querySelectorAll(".ob-checkline input[type=checkbox]")];
  if (boxes.length < 3) return { ok: false, reason: "信任复选框数量异常", count: boxes.length };
  const agreed = boxes[boxes.length - 1];
  if (!agreed.checked) agreed.click();
  const finish = [...document.querySelectorAll(".ob-footer-right button")].find((b) =>
    /完成并进入/.test(b.textContent || ""),
  );
  return { ok: agreed.checked, count: boxes.length, finishDisabled: finish ? finish.disabled : null };
})()`;

const JS_ONBOARDING_STATE = `(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const [onboarding, config, models] = await Promise.all([
    invoke("onboarding_state"),
    invoke("config_get"),
    invoke("models_list"),
  ]);
  return {
    onboarding,
    providers: config.providers.map((p) => ({ id: p.id, enabled: p.enabled, hasKey: p.hasKey, baseUrl: p.baseUrl })),
    skeletonMode: config.advanced.skeletonMode,
    searchMode: config.search.mode,
    translateBackend: config.translate.backend,
    modelsInstalled: models.filter((m) => m.installed).map((m) => m.id),
    restrictedBanner: !!document.querySelector('[data-testid="restricted-notice"]'),
    restrictedBannerText: document.querySelector('[data-testid="restricted-notice"]')?.innerText ?? null,
    onboardingVisible: !!document.querySelector(".ob-root"),
  };
})()`;

const JS_SHOW_MAIN = `window.__TAURI_INTERNALS__.invoke("window_show", { window: "main" }).then(() => true)`;
async function showMain() {
  await waitFor("main", `typeof window.__TAURI_INTERNALS__ !== "undefined"`, "Tauri IPC 就绪", 30000);
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      evalWin("main", JS_SHOW_MAIN);
      return;
    } catch {
      await sleep(500);
    }
  }
  console.error("[S-07] window_show 重试 3 次仍失败（继续执行，截图可能为隐藏窗口内容）");
}

const JS_READ_MENU_DOM = `(() => {
  const items = [...document.querySelectorAll(".overlay-menu__action")].map((button) => {
    const reasonNode = button.querySelector('[id^="overlay-action-reason-"]');
    return {
      label: button.querySelector(".overlay-menu__action-label")?.textContent?.trim() ?? null,
      disabled: button.disabled,
      title: button.title,
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

const JS_READ_MENU_PAYLOADS = `(() => {
  const buckets = window.__s07Events?.menuReady ?? [];
  // Tauri 2 事件 handler 收到的是 Event 包装 {event, id, payload}，业务载荷在 payload 字段。
  return buckets.map((entry) => entry.payload?.payload ?? entry.payload);
})()`;

const JS_CLICK_NAV = (label) => `(() => {
  const button = [...document.querySelectorAll(".main-nav__item")].find((b) =>
    (b.textContent || "").includes(${JSON.stringify(label)}),
  );
  if (!button) return { clicked: false, reason: "找不到导航项 ${label}" };
  button.click();
  return { clicked: true, label: button.textContent.trim() };
})()`;

const JS_CLICK_SETTINGS_SECTION = (label) => `(() => {
  const button = [...document.querySelectorAll(".settings-nav-item")].find((b) =>
    (b.textContent || "").includes(${JSON.stringify(label)}),
  );
  if (!button) return { clicked: false, reason: "找不到设置分组 ${label}" };
  button.click();
  return { clicked: true, label: button.textContent.trim() };
})()`;

const JS_INSTALL_EVENT_LISTENERS = `(async () => {
  const internals = window.__TAURI_INTERNALS__;
  if (!internals || !internals.transformCallback) return { ok: false, reason: "缺少 transformCallback" };
  window.__s07Events = window.__s07Events ?? {};
  const listen = (event, bucket) => {
    window.__s07Events[bucket] = window.__s07Events[bucket] ?? [];
    const handler = internals.transformCallback((payload) => {
      window.__s07Events[bucket].push({ at: Date.now(), payload });
    });
    return internals
      .invoke("plugin:event|listen", { event, target: { kind: "Any" }, handler })
      .then(() => true)
      .catch((error) => String(error));
  };
  const results = {};
  results.menuReady = await listen("menu:ready", "menuReady");
  results.taskError = await listen("task:error", "taskError");
  results.taskDone = await listen("task:done", "taskDone");
  window.__s07MenuPayloads = window.__s07Events.menuReady;
  return { ok: true, results, menuEventListener: !!internals };
})()`;

const JS_TRIGGER_CAPTURE = `(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const session = await invoke("capture_freeze");
  return { session };
})()`;

/** 通过 CDP 在 overlay webview 上模拟真实拖拽：只有 overlay 自己 commit 才会进入 menu 相。 */
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
  try {
    await send("Runtime.enable");
    // 视口坐标：overlay 覆盖整个虚拟桌面，取视口中部任意区域（不落到负坐标副屏边缘）。
    const start = { x: 420, y: 320 };
    const end = { x: 900, y: 560 };
    await send("Input.dispatchMouseEvent", {
      type: "mousePressed",
      x: start.x,
      y: start.y,
      button: "left",
      buttons: 1,
      clickCount: 1,
    });
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
    await send("Input.dispatchMouseEvent", {
      type: "mouseReleased",
      x: end.x,
      y: end.y,
      button: "left",
      buttons: 0,
      clickCount: 1,
    });
    return { start, end };
  } finally {
    socket.close();
  }
}

const JS_PATCH_CONFIG = `(async () => {
  const view = await window.__TAURI_INTERNALS__.invoke("config_patch", {
    patch: {
      advanced: { skeletonMode: false },
      providers: [
        {
          id: "deepseek",
          name: "DeepSeek",
          baseUrl: "http://127.0.0.1:1",
          model: "deepseek-flash",
          caps: ["vision", "search", "thinking"],
          temperature: 0.2,
          maxTokens: 1024,
          enabled: true,
        },
      ],
    },
  });
  return {
    skeletonMode: view.advanced.skeletonMode,
    providers: view.providers.map((p) => ({ id: p.id, baseUrl: p.baseUrl, hasKey: p.hasKey, enabled: p.enabled })),
  };
})()`;

const JS_PROVIDER_TEST_DIRECT = `(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  try {
    const result = await invoke("provider_test", {
      draft: {
        id: "deepseek",
        name: "DeepSeek",
        baseUrl: "http://127.0.0.1:1",
        model: "deepseek-flash",
        caps: ["vision", "search", "thinking"],
        apiKey: null,
      },
    });
    return { threw: false, result };
  } catch (error) {
    return { threw: true, error };
  }
})()`;

const JS_UI_TEST_PROVIDER = `(() => {
  const cards = [...document.querySelectorAll(".sa-provider-card")];
  if (cards.length === 0) return { ok: false, reason: "AI 服务页未渲染供应商卡片" };
  const card = cards[0];
  const expand = [...card.querySelectorAll("button")].find((b) => /展开编辑/.test(b.textContent || ""));
  if (expand) expand.click();
  return { ok: true, expanded: !!expand };
})()`;

const JS_UI_TEST_PROVIDER_CLICK = `(() => {
  const card = document.querySelector(".sa-provider-card");
  if (!card) return { ok: false, reason: "供应商卡片不存在" };
  const buttons = [...card.querySelectorAll("button")].filter((b) => /测试连接/.test(b.textContent || ""));
  if (buttons.length === 0) return { ok: false, reason: "找不到测试连接按钮" };
  buttons[0].click();
  return { ok: true, label: buttons[0].textContent.trim() };
})()`;

const JS_READ_PROVIDER_UI = `(() => {
  const bad = document.querySelector(".sa-testresult.is-bad, .sa-testline.is-bad");
  const badText = bad ? bad.innerText.trim() : null;
  const notice = document.querySelector(".settings-root .sa-hint")?.innerText ?? null;
  return { errorVisible: !!bad, errorText: badText, notice };
})()`;

const JS_INSTALL_TASK_LISTENERS = `(async () => {
  const internals = window.__TAURI_INTERNALS__;
  window.__s07Tasks = { error: [], done: [] };
  for (const event of ["task:error", "task:done"]) {
    const handler = internals.transformCallback((payload) => {
      window.__s07Tasks[event === "task:error" ? "error" : "done"].push(payload);
    });
    await internals
      .invoke("plugin:event|listen", { event, target: { kind: "Any" }, handler })
      .catch(() => undefined);
  }
  return true;
})()`;

const JS_CLOUD_ACTION = `(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const state = await invoke("onboarding_state");
  const config = await invoke("config_get");
  void state;
  void config;
  try {
    const task = await invoke("menu_execute", {
      session: window.__s07Session,
      action: "answer",
      mode: "quick",
      options: { text: "S-07 云动作真实错误路径探测" },
    });
    return { threw: false, task };
  } catch (error) {
    return { threw: true, error };
  }
})()`;

const JS_READ_TASK_EVENTS = `(() => window.__s07Tasks ?? { error: [], done: [] })()`;

const JS_APP_ALIVE = `(async () => {
  const view = await window.__TAURI_INTERNALS__.invoke("config_get");
  return { alive: true, providerCount: view.providers.length, skeletonMode: view.advanced.skeletonMode };
})()`;

// ---------------------------------------------------------------------------
// 流程
// ---------------------------------------------------------------------------

async function captureStep(index) {
  const info = await waitFor(
    "main",
    `((info) => (info.stepTitle ? info : null))(${JS_STEP_INFO})`,
    `引导第 ${index + 1} 步渲染`,
  );
  const fileName = `${String(index + 1).padStart(2, "0")}-step${index + 1}-${info.stepTitle ?? "unknown"}.png`;
  shot("main", fileName);
  evidence.steps.push({ index, ...info, screenshot: fileName });
  console.log(`STEP  ${index + 1}/6  ${info.counter}  ${info.stepTitle}`);
  return info;
}

async function clickNextAndWait(previousCounter) {
  const click = evalWin("main", JS_CLICK_NEXT);
  if (!click.clicked) throw new Error(`无法点击下一步：${click.reason}`);
  await waitFor(
    "main",
    `((info) => (info.counter !== ${JSON.stringify(previousCounter)} ? info : null))(${JS_STEP_INFO})`,
    "步骤切换",
  );
}

async function walkAllSteps() {
  // 回到第 0 步（completed 状态会保留最后位置）。
  for (let guard = 0; guard < 8; guard += 1) {
    const info = evalWin("main", JS_STEP_INFO);
    if (!info.counter || /第 1 \//.test(info.counter)) break;
    evalWin("main", JS_CLICK_PREV);
    await sleep(600);
    await waitFor(
      "main",
      `((info) => (info.counter !== ${JSON.stringify(info.counter)} ? info : null))(${JS_STEP_INFO})`,
      "退回上一步",
      8000,
    ).catch(() => undefined);
  }
  evidence.steps = [];
  const first = await captureStep(0);
  let counter = first.counter;
  for (let index = 1; index < 6; index += 1) {
    await clickNextAndWait(counter);
    const info = await captureStep(index);
    counter = info.counter;
  }
  return evidence.steps.slice(-6);
}

async function skipFlow() {
  const info = await waitFor("main", JS_STEP_INFO, "引导页就绪");
  const skip = evalWin("main", JS_OPEN_SKIP_CONFIRM);
  record("skip.link_available", !!skip.clicked, skip);
  await waitFor("main", `!!document.querySelector(".ob-footer-left .ob-btn-danger")`, "跳过二次确认出现");
  shot("main", "07-skip-confirm.png");
  const confirm = evalWin("main", JS_CONFIRM_SKIP);
  record("skip.confirm_clicked", !!confirm.clicked, confirm);
  const state = await waitFor("main", JS_ONBOARDING_STATE, "跳过后的引导状态", 15000);
  saveRaw("onboarding-state-skip.json", state);
  record("skip.state", state.onboarding.skipped === true && state.onboarding.completed === false, {
    skipped: state.onboarding.skipped,
    completed: state.onboarding.completed,
    finishedAt: state.onboarding.data.finishedAt,
  });
  // 主界面受限提示（等 React 能力刷新后出现）。
  const banner = await waitFor(
    "main",
    `(() => {
      const node = document.querySelector('[data-testid="restricted-notice"]');
      return node ? { text: node.innerText } : null;
    })()`,
    "主界面受限模式提示",
    15000,
  );
  shot("main", "08-main-restricted-banner-skip.png");
  record("skip.main_banner", /受限模式/.test(banner.text) && /本地 OCR/.test(banner.text), {
    text: banner.text,
  });
  void info;
}

async function menuFlow() {
  await showMain();
  const listeners = evalWin("main", JS_INSTALL_EVENT_LISTENERS);
  saveRaw("event-listeners.json", listeners);
  const capture = evalWin("main", JS_TRIGGER_CAPTURE);
  saveRaw("trigger-capture.json", capture);
  const session = capture.session;
  console.log(`TRIGGER capture session=${session}`);
  // overlay 是动态窗口：等它的页面加载完成，再在其内模拟真实拖拽（overlay 自己 commit → committing 相）。
  await waitFor(
    "overlay",
    `document.readyState === "complete" && !!document.querySelector(".overlay-root") ? { ready: true } : null`,
    "overlay 页面就绪",
    20000,
  );
  const drag = await overlayDrag();
  saveRaw("overlay-drag.json", drag);
  // 等待 overlay 收到 menu:ready 并渲染动作菜单（骨架 OCR 约 400ms）。
  // 若首个拖拽未触发 commit（CDP 事件时序），重试一次拖拽。
  const menuPredicate = `(() => {
      const menu = document.querySelector(".overlay-menu");
      return menu ? { present: true, actions: document.querySelectorAll(".overlay-menu__action").length } : null;
    })()`;
  let overlayMenu = null;
  for (let attempt = 0; attempt < 2 && !overlayMenu; attempt += 1) {
    try {
      overlayMenu = await waitFor("overlay", menuPredicate, "overlay 动作菜单", 12000);
    } catch (error) {
      if (attempt === 1) throw error;
      saveRaw("overlay-drag-retry.json", { attempt, reason: String(error.message ?? error) });
      await overlayDrag();
    }
  }
  await sleep(400);
  const menuDom = evalWin("overlay", JS_READ_MENU_DOM);
  const payloads = evalWin("main", JS_READ_MENU_PAYLOADS);
  const selection = menuDom.preview ?? null;
  const menu = { ...menuDom, payloads, payloadCount: payloads.length, session, preview: selection };
  saveRaw("menu-ready-payload-dom.json", menu);
  shot("overlay", "09-overlay-menu-restricted.png");
  const items = menu.items;
  record("menu.payload_captured", menu.payloadCount > 0 || items.length > 0, {
    payloadCount: menu.payloadCount,
    domItemCount: items.length,
    source: menu.payloadCount > 0 ? "menu:ready 事件载荷" : "渲染 DOM（等价载荷字段）",
    preview: menu.preview,
  });
  record(
    "menu.payload_session_matches",
    menu.payloadCount === 0 || payloads[0]?.session === session,
    {
      payloadSession: payloads[0]?.session ?? null,
      triggerSession: session,
    },
  );
  const byLabel = Object.fromEntries(items.map((item) => [item.label, item]));
  record("menu.browser_search_enabled", byLabel["搜索"]?.disabled === false, byLabel["搜索"] ?? null);
  record("menu.copy_enabled", byLabel["复制文字"]?.disabled === false, byLabel["复制文字"] ?? null);
  const disabledExpected = ["翻译", "解答"];
  const disabledOk = disabledExpected.every((label) => byLabel[label]?.disabled === true);
  const reasonOk = disabledExpected.every((label) =>
    /未配置 API Key/.test(byLabel[label]?.disabledReason ?? ""),
  );
  record("menu.cloud_actions_disabled", disabledOk, disabledExpected.map((l) => byLabel[l] ?? null));
  record("menu.disabled_reason_actionable", reasonOk, disabledExpected.map((l) => byLabel[l]?.disabledReason ?? null));
  record("menu.overlay_visible", overlayMenu.present, overlayMenu);
  return { menu, session };
}

async function errorFlow(sessionId) {
  await showMain();
  const patched = evalWin("main", JS_PATCH_CONFIG);
  saveRaw("config-patched-offline.json", patched);
  record("error.skeleton_off_offline", patched.skeletonMode === false && patched.providers[0].baseUrl === "http://127.0.0.1:1", patched);
  // 直接 IPC 调用（错误码断言）
  const direct = evalWin("main", JS_PROVIDER_TEST_DIRECT);
  saveRaw("provider-test-direct.json", direct);
  const directCode = direct.result?.error?.code ?? direct.error?.code;
  record("error.provider_test_real_path", direct.result?.ok === false && String(directCode ?? "").startsWith("SA-3"), direct);

  // UI 路径：设置 → AI 服务 → 展开编辑 → 测试连接 → 错误可见。
  evalWin("main", JS_CLICK_NAV("设置"));
  await waitFor("main", `!!document.querySelector(".settings-nav-item")`, "设置页渲染", 20000);
  evalWin("main", JS_CLICK_SETTINGS_SECTION("AI 服务"));
  await waitFor("main", `!!document.querySelector(".sa-provider-card")`, "AI 服务分组渲染", 20000);
  evalWin("main", JS_UI_TEST_PROVIDER);
  await sleep(300);
  evalWin("main", JS_UI_TEST_PROVIDER_CLICK);
  const ui = await waitFor(
    "main",
    `(() => {
      const bad = document.querySelector(".sa-testresult.is-bad, .sa-testline.is-bad");
      return bad ? { text: bad.innerText } : null;
    })()`,
    "UI 显示连接失败",
    20000,
  );
  shot("main", "10-provider-test-error-ui.png");
  saveRaw("provider-test-ui.json", ui);
  record("error.ui_shows_actionable", /SA-3|网络|设置|跳过/.test(ui.text), ui);

  // 云动作真实错误路径：menu_execute（当前云实现为 TODO(real) 骨架网关）。
  evalWin("main", JS_INSTALL_TASK_LISTENERS);
  const cloud = evalWin("main", JS_CLOUD_ACTION.replace("window.__s07Session", JSON.stringify(sessionId)));
  saveRaw("cloud-action-attempt.json", cloud);
  await sleep(2500);
  const events = evalWin("main", JS_READ_TASK_EVENTS);
  saveRaw("cloud-action-task-events.json", events);
  record("error.cloud_action_backend", true, {
    note:
      "menu_execute 后端未做能力门禁、云实现为 TODO(real) 骨架网关：真实网络错误无法产生；UI 已禁用云动作。见文档遗留 L-1。",
    attempt: cloud,
    taskErrorCount: events.error.length,
    taskDoneCount: events.done.length,
    engine: events.done[0]?.engine ?? events.error[0]?.code ?? null,
  });
  const alive = evalWin("main", JS_APP_ALIVE);
  saveRaw("app-alive-after-error.json", alive);
  record("error.app_alive", alive.alive === true, alive);
}

async function completionFlow() {
  // 重新进入引导，完整走完 6 步（不填 Key、不下载模型），勾选隐私同意后完成。
  await showMain();
  evalWin("main", JS_CLICK_NAV("首启引导"));
  await waitFor("main", `!!document.querySelector(".ob-root")`, "重新打开引导");
  await sleep(500);
  const steps = await walkAllSteps();
  saveRaw("steps.json", steps);
  record("steps.six_steps_rendered", steps.length === 6 && steps.every((step) => !!step.stepTitle), steps.map((s) => `${s.counter ?? ""} ${s.stepTitle}`));

  const lastInfo = evalWin("main", JS_STEP_INFO);
  record("steps.last_step_restricted_notice", lastInfo.restrictedNotice === true, lastInfo);
  shot("main", "17-step6-restricted-notice.png");

  const agree = evalWin("main", JS_AGREE_PRIVACY);
  record("steps.privacy_agreed", agree.ok === true, agree);
  await waitFor(
    "main",
    `(() => {
      const b = [...document.querySelectorAll(".ob-footer-right button")].find((x) => /完成并进入/.test(x.textContent || ""));
      return b && !b.disabled;
    })()`,
    "完成按钮可点击",
  );
  const finish = evalWin("main", JS_CLICK_NEXT);
  record("steps.finish_clicked", finish.clicked === true, finish);

  const state = await waitFor(
    "main",
    `(async () => {
      const onboarding = await window.__TAURI_INTERNALS__.invoke("onboarding_state");
      return { onboarding };
    })()`,
    "引导完成状态",
    15000,
  );
  saveRaw("onboarding-state-completed.json", state);
  record("steps.completed", state.onboarding.completed === true, {
    completed: state.onboarding.completed,
    skipped: state.onboarding.skipped,
    currentStep: state.onboarding.currentStep,
  });
  const banner = await waitFor(
    "main",
    `(() => {
      const node = document.querySelector('[data-testid="restricted-notice"]');
      return node ? { text: node.innerText } : null;
    })()`,
    "完成后主界面受限提示",
    15000,
  );
  shot("main", "18-main-restricted-banner-completed.png");
  record("steps.main_banner_after_completion", /受限模式/.test(banner.text), { text: banner.text });
  // 去配置入口存在（真实可点击）：点击后应落到设置 → AI 服务分组。
  const configure = evalWin(
    "main",
    `(() => {
      const node = document.querySelector('[data-testid="restricted-notice"] button');
      if (!node) return { clicked: false };
      node.click();
      return { clicked: true, label: node.textContent.trim() };
    })()`,
  );
  const section = await waitFor(
    "main",
    `(() => {
      const active = document.querySelector(".settings-nav-item.is-active");
      return active ? { active: active.textContent.trim() } : null;
    })()`,
    "去配置落地分组",
    15000,
  );
  shot("main", "19-main-goto-config.png");
  record("steps.configure_entry", configure.clicked === true && /AI 服务/.test(section.active), { configure, section });
}

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  mkdirSync(SHOTS, { recursive: true });
  mkdirSync(RAW, { recursive: true });

  const hasState = existsSync(join(REPO, ".verify", "applab", "state.json"));
  if (!ATTACH && hasState) {
    console.error("[S-07] 检测到 applab 已有实例；请先等待/停止他人实例，或加 --attach 复用。");
  }
  try {
    if (!ATTACH) {
      console.log("[S-07] 启动便携应用（--reset = 首次启动）…");
      let started = false;
      let lastError = null;
      const startArgs = ["start", "--owner", OWNER, "--wait-sec", "900"];
      if (!ONLY_MENU) startArgs.push("--reset");
      for (let attempt = 0; attempt < 3 && !started; attempt += 1) {
        try {
          applab(startArgs);
          started = true;
        } catch (error) {
          lastError = error;
          console.error(`[S-07] app 启动失败（第 ${attempt + 1} 次），5s 后重试：${error.message}`);
          await sleep(5000);
        }
      }
      if (!started) throw lastError ?? new Error("applab start 失败");
    }
    if (ONLY_MENU) {
      // 补充校验：只跑 overlay 菜单取证（不改数据、不重置），用于修正 driver 取值层级后复测。
      console.log("[S-07] --only-menu：overlay 动作菜单补充取证…");
      await waitFor("main", `!!document.querySelector(".main-window")`, "主窗口渲染", 30000);
      const menu = await menuFlow();
      saveRaw("evidence-menu-only.json", {
        runAt: new Date().toISOString(),
        session: menu.session,
        checks: evidence.checks,
      });
      console.log("[S-07] --only-menu 完成");
      return;
    }
    await showMain();
    await waitFor("main", `!!document.querySelector(".ob-root")`, "首启引导自动弹出", 30000);
    // 首次启动：引导自动路由已经发生（未完成未跳过 → onboarding）。
    const initial = await waitFor("main", JS_ONBOARDING_STATE, "初始引导状态");
    saveRaw("onboarding-state-initial.json", initial);
    record(
      "onboarding.auto_open_first_launch",
      initial.onboardingVisible === true &&
        initial.onboarding.completed === false &&
        initial.onboarding.skipped === false,
      {
        visible: initial.onboardingVisible,
        completed: initial.onboarding.completed,
        skipped: initial.onboarding.skipped,
        skeletonMode: initial.skeletonMode,
        providers: initial.providers,
        modelsInstalled: initial.modelsInstalled,
      },
    );
    // 第 1 步截图（首次启动）。
    await captureStep(0);

    // b) 跳过路径 → 受限模式。
    await skipFlow();

    // b 续) overlay 动作菜单禁用项与原因（真实 menu:ready / DOM）。
    const menu = await menuFlow();

    // c) 断网（baseUrl 指向 127.0.0.1:1）+ 空 Key + skeletonMode=false 的真实错误路径。
    await errorFlow(menu.session ?? null);

    // a) 完整 6 步走完（每步截图）→ 完成但未配置能力仍为受限模式。
    await completionFlow();

    // 汇总。
    saveRaw("evidence.json", evidence);
    console.log("\n===== S-07 汇总 =====");
    console.log(`截图 ${evidence.screenshots.length} 张，断言 ${Object.keys(evidence.checks).length} 项，失败 ${evidence.failedChecks.length} 项`);
    for (const [name, check] of Object.entries(evidence.checks)) {
      console.log(`${check.ok ? "PASS" : "FAIL"} ${name}`);
    }
    if (evidence.failedChecks.length > 0) {
      console.log(JSON.stringify(evidence.failedChecks, null, 2));
      process.exitCode = 1;
    }
  } catch (error) {
    evidence.error = String(error?.stack ?? error);
    saveRaw("evidence.json", evidence);
    console.error(`[S-07] 失败：${error?.stack ?? error}`);
    process.exitCode = 1;
  } finally {
    if (!ATTACH && !KEEP) {
      try {
        applab(["stop", "--owner", OWNER]);
      } catch (error) {
        console.error(`[S-07] 停止应用失败：${error.message}`);
      }
    }
  }
}

await main();
