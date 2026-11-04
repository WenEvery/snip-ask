/**
 * 框选 AI · 真实应用验证台（spike 共用）。
 *
 * 为什么需要它：本仓库的 P0 验证需要在**真实 Tauri 应用**里跑（全局热键、overlay 窗口几何、
 * 首启引导、模型下载状态），而机器上只允许同时存在一个应用实例（单实例插件），
 * 且 `%APPDATA%\SnipAsk` 是用户真实配置。本工具解决三件事：
 *   1) 便携隔离：把 debug 可执行文件复制到 `.verify/applab/app/` 并放 `portable.flag`，
 *      数据写入 `.verify/applab/app/data/`，不污染用户目录；
 *   2) 一键起停：自动拉起 `pnpm dev:web`（Vite）+ 便携 app，等待 WebView2 CDP 就绪；
 *   3) 远程驱动：通过 WebView2 CDP（`--remote-debugging-port=9333`）在 webview 里执行 JS，
 *      可调用 `window.__TAURI_INTERNALS__.invoke(...)`，从而在没有测试专用命令的情况下驱动真实 IPC。
 *
 * 用法（Node ≥ 20；本机 v24）：
 *   node scripts/spikes/applab/applab.mjs start [--owner <who>] [--wait-sec 600] [--reset]
 *   node scripts/spikes/applab/applab.mjs stop [--owner <who>] [--force] [--vite]
 *   node scripts/spikes/applab/applab.mjs status
 *   node scripts/spikes/applab/applab.mjs targets
 *   node scripts/spikes/applab/applab.mjs eval --window main --js "window.__TAURI_INTERNALS__ ? 1 : 0"
 *   node scripts/spikes/applab/applab.mjs shot --window main --out .verify/shots/main.png
 *   node scripts/spikes/applab/applab.mjs reset            # 删除便携数据目录（模拟首次启动）
 *   node scripts/spikes/applab/applab.mjs reserve --owner <who> --minutes 10   # 时段预约（防重启循环期间被插队）
 *   node scripts/spikes/applab/applab.mjs release-reserve --owner <who>
 *
 * 约定：`start` 会抢占「app」独占锁（状态文件 `.verify/applab/state.json`）；
 * 其他 spike 角色请等持锁者 `stop` 后再 `start`，或带 `--wait-sec` 轮询等待。
 */
import { execFileSync, spawn } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..", "..");
const ROOT = join(REPO, ".verify", "applab");
const APP_DIR = join(ROOT, "app");
const STATE_FILE = join(ROOT, "state.json");
const RESERVE_FILE = join(ROOT, "reserve.json");
const VITE_LOG = join(ROOT, "vite.log");
const CDP_PORT = 9333;
const VITE_PORT = 1420;
const APP_SRC = join(REPO, "src-tauri", "target", "debug", "snip-ask.exe");
const APP_EXE = join(APP_DIR, "snip-ask.exe");

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

function parseArgs(argv) {
  const out = { _: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg.startsWith("--")) {
      const key = arg.slice(2);
      const next = argv[i + 1];
      if (next !== undefined && !next.startsWith("--")) {
        out[key] = next;
        i += 1;
      } else {
        out[key] = true;
      }
    } else {
      out._.push(arg);
    }
  }
  return out;
}

function readState() {
  if (!existsSync(STATE_FILE)) return null;
  try {
    return JSON.parse(readFileSync(STATE_FILE, "utf8"));
  } catch {
    return { corrupt: true };
  }
}

function writeState(state) {
  mkdirSync(ROOT, { recursive: true });
  writeFileSync(STATE_FILE, `${JSON.stringify(state, null, 2)}\n`);
}

/**
 * 「时段预约」：某成员在一次验证里可能要反复 start/stop（例如热键矩阵每 2-4s 重启一次 app），
 * 期间 state.json 会闪断，容易被别人插队导致单实例竞态。预约文件在预约到期前让其他 owner
 * 的 start 一律等待。预约可跨多次 start/stop 存活，用完 `release-reserve` 释放。
 */
function readReserve() {
  if (!existsSync(RESERVE_FILE)) return null;
  try {
    return JSON.parse(readFileSync(RESERVE_FILE, "utf8"));
  } catch {
    return null;
  }
}

function reserveActive(reserve) {
  if (!reserve?.expiresAt) return false;
  return new Date(reserve.expiresAt).getTime() > Date.now();
}

function describeReserve(reserve) {
  return `owner=${reserve.owner} until=${reserve.expiresAt} note=${reserve.note ?? ""}`;
}

function pidAlive(pid) {
  if (!pid) return false;
  try {
    process.kill(Number(pid), 0);
    return true;
  } catch {
    return false;
  }
}

function listAppPids() {
  try {
    const text = execFileSync(
      "powershell.exe",
      ["-NoProfile", "-Command", "(Get-Process snip-ask -ErrorAction SilentlyContinue).Id -join ','"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
    ).trim();
    return text ? text.split(",").filter(Boolean).map(Number) : [];
  } catch {
    return [];
  }
}

function killTree(pid) {
  if (!pid) return;
  try {
    execFileSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
  } catch {
    /* 已退出 */
  }
}

function killAllApp() {
  for (const pid of listAppPids()) killTree(pid);
}

async function portUp(url, timeoutMs = 1500) {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    const response = await fetch(url, { signal: controller.signal });
    clearTimeout(timer);
    return response.status < 500;
  } catch {
    return false;
  }
}

async function cdpTargets() {
  const response = await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`);
  return response.json();
}

async function waitCdp(timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const targets = await cdpTargets();
      if (targets.some((target) => String(target.url ?? "").includes("window=main"))) {
        return targets;
      }
    } catch {
      /* 还没起来 */
    }
    await sleep(400);
  }
  throw new Error(`WebView2 CDP 未在 ${timeoutMs}ms 内就绪（端口 ${CDP_PORT}）`);
}

async function ensureVite() {
  // 注意：本机 Vite 只监听 IPv6 localhost(`::1`)，必须用 localhost 而不是 127.0.0.1 探测。
  if (await portUp(`http://localhost:${VITE_PORT}/`)) {
    return { reused: true, pid: readState()?.vitePid ?? null };
  }
  mkdirSync(ROOT, { recursive: true });
  console.error("[applab] 启动 Vite dev server…");
  // Windows 上 `pnpm` 是 .cmd 包装，必须经 cmd.exe 启动。
  const comspec = process.env.ComSpec ?? "cmd.exe";
  const child = spawn(comspec, ["/d", "/s", "/c", "pnpm dev:web"], {
    cwd: REPO,
    detached: true,
    stdio: "ignore",
    windowsHide: true,
  });
  child.unref();
  const deadline = Date.now() + 90000;
  while (Date.now() < deadline) {
    if (await portUp(`http://localhost:${VITE_PORT}/`)) {
      return { reused: false, pid: child.pid };
    }
    await sleep(500);
  }
  throw new Error("Vite dev server 启动超时（端口 1420）");
}

async function waitNoApp(timeoutMs = 10000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (listAppPids().length === 0) return true;
    killAllApp();
    await sleep(500);
  }
  return listAppPids().length === 0;
}

async function copyPortableApp() {
  if (!existsSync(APP_SRC)) {
    throw new Error(`找不到 ${APP_SRC}，请先构建：cargo build --manifest-path src-tauri/Cargo.toml`);
  }
  mkdirSync(APP_DIR, { recursive: true });
  // 若旧实例在跑，先杀掉，否则 Windows 会锁住 exe 导致覆盖失败。
  killAllApp();
  await waitNoApp(10000);
  copyFileSync(APP_SRC, APP_EXE);
  writeFileSync(join(APP_DIR, "portable.flag"), "applab portable data dir\n");
}

async function cmdStart(args) {
  const owner = String(args.owner ?? "unknown");
  const waitSec = Number(args["wait-sec"] ?? 600);
  mkdirSync(ROOT, { recursive: true });

  // 1) 抢占 app 独占锁（以应用进程存活为准；另有「时段预约」可覆盖 start/stop 循环的闪断）
  const deadline = Date.now() + waitSec * 1000;
  for (;;) {
    const state = readState();
    const live = state && !state.corrupt && pidAlive(state.appPid);
    const reserve = readReserve();
    const reservedByOther = reserveActive(reserve) && reserve.owner !== owner;
    if (!live && !reservedByOther) {
      if (state && !state.corrupt) console.error(`[applab] 接管失效状态（旧 app pid ${state.appPid} 已退出）`);
      break;
    }
    if (live && state.owner === owner && !reservedByOther) {
      console.error(`[applab] 该 owner 的应用已在运行：${JSON.stringify(state)}`);
      return 0;
    }
    if (Date.now() > deadline) {
      console.error(
        `[applab] 等待超时：${live ? `应用被 ${state.owner} 占用（pid ${state.appPid}）` : ""}${
          reservedByOther ? `；时段被预约（${describeReserve(reserve)}）` : ""
        }`,
      );
      return 2;
    }
    if (live) console.error(`[applab] 应用被 ${state.owner} 占用，等待…（pid ${state.appPid}）`);
    if (reservedByOther) console.error(`[applab] 时段被预约，等待…（${describeReserve(reserve)}）`);
    await sleep(3000);
  }

  if (args.reset) {
    console.error("[applab] --reset：删除便携数据目录，模拟首次启动");
    rmSync(join(APP_DIR, "data"), { recursive: true, force: true });
    rmSync(join(REPO, ".verify", "applab", "shots"), { recursive: true, force: true });
  }

  // 2) 清理残留：单实例插件会让新进程直接退出，且被 kill 的实例需要时间释放命名互斥体 /
  //    WebView2 用户数据锁，否则新实例可能静默退出、CDP 永不就绪（已实测踩坑）。
  killAllApp();
  await waitNoApp(10000);
  await sleep(1200);

  // 3) Vite + 便携应用
  const vite = await ensureVite();
  await copyPortableApp();
  await waitNoApp(5000);

  // 4) 启动（最多 2 次：偶发的单实例/WebView2 锁竞争会让进程静默退出）
  let targets = null;
  let appPid = null;
  let lastError = null;
  for (let attempt = 1; attempt <= 2 && !targets; attempt += 1) {
    console.error(`[applab] 启动便携应用（第 ${attempt} 次，WebView2 CDP 端口 ${CDP_PORT}）…`);
    const child = spawn(APP_EXE, [], {
      cwd: APP_DIR,
      detached: true,
      stdio: "ignore",
      env: {
        ...process.env,
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
      },
      windowsHide: false,
    });
    child.unref();
    try {
      targets = await waitCdp(attempt === 1 ? 25000 : 30000);
      appPid = child.pid;
    } catch (error) {
      lastError = error;
      console.error(`[applab] 第 ${attempt} 次启动未就绪：${error.message}`);
      killTree(child.pid);
      killAllApp();
      await waitNoApp(8000);
      await sleep(1500);
    }
  }
  if (!targets || !appPid) {
    throw lastError ?? new Error("应用启动失败（CDP 未就绪）");
  }
  writeState({
    owner,
    appPid,
    vitePid: vite.pid,
    startedAt: new Date().toISOString(),
    appDir: APP_DIR,
    cdpPort: CDP_PORT,
  });
  console.error(`[applab] 就绪 owner=${owner} appPid=${appPid} vite=${vite.reused ? "复用" : `pid ${vite.pid}`}`);
  console.log(JSON.stringify({ ok: true, owner, appPid, targets: targets.map((t) => t.url) }, null, 2));
  return 0;
}

function cmdStop(args) {
  const state = readState();
  if (state && !state.corrupt && args.owner && state.owner !== args.owner && !args.force) {
    console.error(`[applab] 拒绝停止：当前 owner=${state.owner}，你是 ${args.owner}（--force 可强制）`);
    return 2;
  }
  killAllApp();
  if (args.vite && state?.vitePid) killTree(state.vitePid);
  try {
    rmSync(STATE_FILE, { force: true });
  } catch {
    /* ignore */
  }
  console.log(`[applab] 已停止${args.vite ? "（含 Vite）" : ""}`);
  return 0;
}

function cmdReserve(args) {
  const owner = String(args.owner ?? "");
  const minutes = Number(args.minutes ?? 10);
  if (!owner) {
    console.error("用法：reserve --owner <who> --minutes N [--note ...] [--force]");
    return 3;
  }
  const existing = readReserve();
  if (reserveActive(existing) && existing.owner !== owner && !args.force) {
    console.error(`[applab] 预约已被占用：${describeReserve(existing)}（--force 可抢占）`);
    return 2;
  }
  const payload = {
    owner,
    expiresAt: new Date(Date.now() + minutes * 60000).toISOString(),
    note: args.note ?? "",
  };
  writeFileSync(RESERVE_FILE, `${JSON.stringify(payload, null, 2)}\n`);
  console.log(`[applab] 预约成功：owner=${owner} until=${payload.expiresAt}`);
  return 0;
}

function cmdReleaseReserve(args) {
  const owner = String(args.owner ?? "");
  const existing = readReserve();
  if (!existing) {
    console.log("[applab] 无预约");
    return 0;
  }
  if (existing.owner !== owner && !args.force) {
    console.error(`[applab] 拒绝释放预约：持有者是 ${existing.owner}（--force 可强制）`);
    return 2;
  }
  rmSync(RESERVE_FILE, { force: true });
  console.log(`[applab] 预约已释放：${owner}（原 ${describeReserve(existing)}）`);
  return 0;
}

function cmdStatus() {
  const state = readState();
  if (!state) {
    console.log("[applab] 未运行（无 state.json）");
    return 0;
  }
  const alive = pidAlive(state.appPid);
  console.log(
    `[applab] owner=${state.owner} appPid=${state.appPid} alive=${alive} since=${state.startedAt} vitePid=${state.vitePid ?? "-"} cdp=${state.cdpPort}`,
  );
  console.log(`[applab] 数据目录：${join(APP_DIR, "data")}`);
  const reserve = readReserve();
  if (reserveActive(reserve)) console.log(`[applab] 时段预约：${describeReserve(reserve)}`);
  return alive ? 0 : 1;
}

async function findTarget(windowName) {
  const targets = await cdpTargets();
  const match = targets.find((target) => String(target.url ?? "").includes(`window=${windowName}`));
  if (!match) {
    throw new Error(
      `未找到 window=${windowName} 的 webview；当前 targets=${JSON.stringify(targets.map((t) => t.url))}`,
    );
  }
  return match;
}

function connectCdp(wsUrl) {
  return new Promise((resolvePromise, rejectPromise) => {
    const socket = new WebSocket(wsUrl);
    socket.addEventListener("open", () => resolvePromise(socket));
    socket.addEventListener("error", (event) => rejectPromise(new Error(String(event?.message ?? "CDP 连接失败"))));
  });
}

async function withCdp(windowName, action) {
  const target = await findTarget(windowName);
  const socket = await connectCdp(target.webSocketDebuggerUrl);
  let nextId = 0;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (!message.id || !pending.has(message.id)) return;
    const { resolve: done, reject } = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) reject(new Error(JSON.stringify(message.error)));
    else done(message.result);
  });
  const send = (method, params = {}) =>
    new Promise((done, reject) => {
      nextId += 1;
      pending.set(nextId, { resolve: done, reject });
      socket.send(JSON.stringify({ id: nextId, method, params }));
    });
  try {
    await send("Runtime.enable");
    return await action(send);
  } finally {
    socket.close();
  }
}

async function cmdEval(args) {
  const windowName = String(args.window ?? "main");
  const js = String(args.js ?? "");
  if (!js) {
    console.error('用法：eval --window main --js "<表达式>"');
    return 3;
  }
  const result = await withCdp(windowName, async (send) => {
    const evaluated = await send("Runtime.evaluate", {
      expression: js,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
    });
    if (evaluated.exceptionDetails) {
      return { ok: false, exception: evaluated.exceptionDetails };
    }
    return { ok: true, value: evaluated.result?.value ?? null };
  });
  console.log(JSON.stringify(result, null, 2));
  return result.ok ? 0 : 1;
}

async function cmdShot(args) {
  const windowName = String(args.window ?? "main");
  const out = resolve(REPO, String(args.out ?? ".verify/applab/shots/shot.png"));
  mkdirSync(dirname(out), { recursive: true });
  await withCdp(windowName, async (send) => {
    const shot = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
    writeFileSync(out, Buffer.from(shot.data, "base64"));
  });
  console.log(`[applab] 截图已保存：${out}`);
  return 0;
}

const args = parseArgs(process.argv.slice(2));
const command = args._[0];
try {
  let code = 3;
  if (command === "start") code = await cmdStart(args);
  else if (command === "stop") code = cmdStop(args);
  else if (command === "status") code = cmdStatus();
  else if (command === "reserve") code = cmdReserve(args);
  else if (command === "release-reserve") code = cmdReleaseReserve(args);
  else if (command === "targets") {
    console.log(JSON.stringify(await cdpTargets(), null, 2));
    code = 0;
  } else if (command === "eval") code = await cmdEval(args);
  else if (command === "shot") code = await cmdShot(args);
  else if (command === "reset") {
    rmSync(join(APP_DIR, "data"), { recursive: true, force: true });
    console.log("[applab] 便携数据目录已清空（下次 start 为首次启动）");
    code = 0;
  } else {
    console.error("用法：applab.mjs <start|stop|status|targets|eval|shot|reset> [...]");
  }
  process.exit(code);
} catch (error) {
  console.error(`[applab] 失败：${error.message}`);
  process.exit(1);
}
