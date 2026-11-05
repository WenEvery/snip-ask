/**
 * P1 应用运行时验证（S-13 / S-17 / S-19 / S-20）共享工具。
 *
 * 复用 `scripts/spikes/applab/applab.mjs` 的独占锁与 WebView2 CDP 驱动：
 *   - `evalWin(window, js)`：在 main/overlay webview 里执行 JS，返回按值 JSON；
 *   - `waitFor(...)`：轮询直到 JS 表达式返回真值（引导页/菜单/状态切换）；
 *   - `applabStart/Stop`：起停便携应用（portable 数据目录 `.verify/applab/app/data`）。
 *
 * 注意：本机 PowerShell 5.1 给原生 node 传参会丢双引号，故一律用 `execFileSync(node, [args])`
 * 直接调用，不经过 shell；JS 参数用单引号对象字面量（约定同 S-07）。
 */
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const HERE = dirname(fileURLToPath(import.meta.url));
export const REPO = resolve(HERE, "..", "..", "..");
export const APPLAB = join(REPO, "scripts", "spikes", "applab", "applab.mjs");
export const EVID = join(REPO, ".verify", "spikes", "S-13-S-17-S-19-S-20");
export const DOCS_DATA = join(REPO, "docs", "spikes", "data", "s13_s17_s19_s20");
export const OWNER = "app-runtime";

export const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

export function ensureDir(dir) {
  mkdirSync(dir, { recursive: true });
}

/** 直接调 node applab.mjs；返回 { code, stdout, stderr }，不抛异常（调用方判断 code）。 */
export function applab(argv, timeout = 180000) {
  try {
    const stdout = execFileSync(process.execPath, [APPLAB, ...argv], {
      encoding: "utf8",
      timeout,
      windowsHide: true,
    });
    return { code: 0, stdout, stderr: "" };
  } catch (error) {
    return {
      code: typeof error.status === "number" ? error.status : -1,
      stdout: String(error.stdout ?? ""),
      stderr: String(error.stderr ?? error.message ?? ""),
    };
  }
}

export function applabStart({ reset = false, waitSec = 300 } = {}) {
  const argv = ["start", "--owner", OWNER, "--wait-sec", String(waitSec)];
  if (reset) argv.push("--reset");
  return applab(argv, (waitSec + 120) * 1000);
}

export function applabStop() {
  return applab(["stop", "--owner", OWNER], 60000);
}

/** 在指定 webview 里执行 JS 并返回 value；异常/非 JSON 输出直接抛错。 */
export function evalWin(windowName, js) {
  const result = applab(["eval", "--window", windowName, "--js", js], 120000);
  let parsed;
  try {
    parsed = JSON.parse(result.stdout);
  } catch {
    throw new Error(`eval 输出非 JSON（${windowName}）：${result.stdout.slice(0, 400)} ${result.stderr.slice(0, 400)}`);
  }
  if (!parsed.ok) throw new Error(`eval 异常（${windowName}）：${JSON.stringify(parsed.exception).slice(0, 800)}`);
  return parsed.value;
}

/** 轮询等待 JS 表达式返回真值；超时抛错（附最后一次值）。 */
export async function waitFor(windowName, js, label, timeoutMs = 20000, intervalMs = 300) {
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
  throw new Error(`等待超时（${label}，${timeoutMs}ms）；最后一次=${JSON.stringify(last)?.slice(0, 500)}`);
}

export function shot(windowName, name) {
  const rel = join(".verify", "spikes", "S-13-S-17-S-19-S-20", "shots", name);
  const out = join(REPO, rel);
  ensureDir(dirname(out));
  const result = applab(["shot", "--window", windowName, "--out", rel], 60000);
  return { ok: result.code === 0, path: out, stdout: result.stdout.trim(), stderr: result.stderr.trim() };
}

export function saveRaw(subdir, name, value) {
  const out = join(EVID, "raw", subdir, name);
  ensureDir(dirname(out));
  writeFileSync(out, `${JSON.stringify(value, null, 2)}\n`);
  return out;
}

export function saveDocsData(name, value) {
  ensureDir(DOCS_DATA);
  const out = join(DOCS_DATA, name);
  writeFileSync(out, `${JSON.stringify(value, null, 2)}\n`);
  return out;
}

export function readJson(file) {
  return JSON.parse(readFileSync(file, "utf8"));
}

export function exists(file) {
  return existsSync(file);
}

export function listAppPids() {
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

export function applabStatus() {
  const result = applab(["status"], 60000);
  return { code: result.code, text: `${result.stdout}${result.stderr}`.trim() };
}

/** 浏览器内未捕获异常收集器：安装一次后所有流程结束读 window.__p1Uncaught。 */
export const JS_INSTALL_UNCAUGHT = `(() => {
  if (window.__p1UncaughtInstalled) return { installed: true, reused: true };
  window.__p1Uncaught = { errors: [], rejections: [] };
  window.addEventListener("error", (event) => {
    window.__p1Uncaught.errors.push({
      at: new Date().toISOString(),
      message: String(event.message ?? ""),
      source: String(event.filename ?? ""),
      line: event.lineno ?? null,
    });
  });
  window.addEventListener("unhandledrejection", (event) => {
    const reason = event.reason;
    window.__p1Uncaught.rejections.push({
      at: new Date().toISOString(),
      message: String(reason?.message ?? reason ?? ""),
      code: reason?.code ?? null,
    });
  });
  window.__p1UncaughtInstalled = true;
  return { installed: true, reused: false };
})()`;

export const JS_READ_UNCAUGHT = `(() => window.__p1Uncaught ?? { errors: [], rejections: [] })()`;
