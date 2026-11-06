/**
 * S-20 托盘 / 单实例 / 热键释放 · 真实应用验证驱动。
 *
 * 覆盖：
 *   A) 第二实例：10 次「直接启动第二个 snip-ask.exe」→ 应有实例被唤起（主窗口显示 +
 *      `tray:action{id:"singleInstance"}` 事件）、第二进程退出、进程数保持 1；
 *   B) 托盘：托盘宿主窗口 / Shell 图标注册 / 溢出区可访问性探测（Win32 + UIA，脚本 `s20_tray.ps1`），
 *      并记录 Win11 溢出区下原生菜单自动化的真实限制（S-04 已记录同类问题）；
 *   C) 热键释放：暂停/恢复热键路径（togglePause）下 `Ctrl+Q` 的真实 RegisterHotKey 探测，
 *      以及「正常退出后 Ctrl+Q 无残留」的 N 次 quit/restart 循环。
 *
 * 用法（仓库根目录；先用 applab 独占）：
 *   node scripts/spikes/s13_s17_s19_s20/run_s20.mjs [--attach] [--keep]
 *
 * 证据：`.verify/spikes/S-13-S-17-S-19-S-20/raw/s20/`、`.../shots/s20/`、
 *      聚合 `docs/spikes/data/s13_s17_s19_s20/s20_runtime.json`。
 */
import { spawn, execFileSync } from "node:child_process";
import { join } from "node:path";
import {
  JS_INSTALL_UNCAUGHT,
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
const SINGLE_INSTANCE_RUNS = 10;
const QUIT_RELEASE_RUNS = 5;
const PAUSE_RUNS = 3;

const APP_DIR = join(REPO, ".verify", "applab", "app");
const APP_EXE = join(APP_DIR, "snip-ask.exe");
const TRAY_PS1 = join(REPO, "scripts", "spikes", "s13_s17_s19_s20", "s20_tray.ps1");
const HOLD_PS1 = join(REPO, "scripts", "spikes", "s04", "hold_hotkey.ps1");
const PRESS_PS1 = join(REPO, "scripts", "spikes", "s04", "press_hotkey.ps1");

function runPwsh(script, args = [], timeout = 60000) {
  return execFileSync("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script, ...args], {
    encoding: "utf8",
    timeout,
    windowsHide: true,
  });
}

/** 运行 s20_tray.ps1 的某个 action，返回其 JSON。 */
function trayAction(action, extra = []) {
  const out = runPwsh(TRAY_PS1, ["-Action", action, ...extra]);
  const line = out
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter((item) => item.startsWith("{"))
    .pop();
  return JSON.parse(line);
}

/** 用 S-04 的 hold_hotkey.ps1 做 OS 级真实注册探测：registered=true 表示该组合当前空闲。 */
function hotkeyProbe(spec = "Ctrl+Q") {
  try {
    const out = runPwsh(HOLD_PS1, ["-Spec", spec, "-HoldSec", "0"]);
    const lines = out.split(/\r?\n/).filter((line) => line.trim().startsWith("{")).map((line) => JSON.parse(line));
    const registered = lines.find((line) => line.phase === "registered") ?? null;
    const released = lines.find((line) => line.phase === "released") ?? null;
    return { spec, registered: registered?.registered ?? false, win32Error: registered?.win32Error ?? null, released: released ?? null };
  } catch (error) {
    // hold_hotkey.ps1 注册失败时退出码 2，stdout 仍有一行 JSON
    const text = String(error.stdout ?? "");
    const lines = text.split(/\r?\n/).filter((line) => line.trim().startsWith("{")).map((line) => JSON.parse(line));
    const registered = lines.find((line) => line.phase === "registered") ?? null;
    return { spec, registered: registered?.registered ?? false, win32Error: registered?.win32Error ?? null, released: null };
  }
}

function pressHotkey(spec) {
  const out = runPwsh(PRESS_PS1, ["-Spec", spec]);
  return JSON.parse(out.split(/\r?\n/).find((line) => line.trim().startsWith("{")));
}

function appWindows() {
  return trayAction("windows");
}

/**
 * 主窗口当前是否可见。
 * 注意：窗口 title 经 PowerShell 5.1 控制台编码后可能乱码，因此只按窗口类名 `Tauri Window` 判定；
 * 应用在没有结果窗时只有 main 一个 Tauri 窗口，故「Tauri 窗口可见数」= main 是否被唤起。
 */
function mainWindowVisible(windowsResult) {
  return (windowsResult.windows ?? []).some(
    (item) => String(item.class).includes("Tauri Window") && item.visible === true,
  );
}

function launchSecondInstance() {
  const startedAt = Date.now();
  const child = spawn(APP_EXE, [], { cwd: APP_DIR, detached: true, stdio: "ignore", windowsHide: true });
  child.unref();
  return { pid: child.pid, startedAt };
}

const JS_SHOW_MAIN = `window.__TAURI_INTERNALS__.invoke("window_show", { window: "main" }).then(() => true)`;
const JS_HIDE_MAIN = `window.__TAURI_INTERNALS__.invoke("window_hide", { window: "main" }).then(() => true)`;
const JS_APP_QUIT = `(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  try { await invoke("app_quit"); return { quit: true }; } catch (error) { return { quit: false, error }; }
})()`;
const JS_ALIVE = `(async () => {
  try {
    const view = await window.__TAURI_INTERNALS__.invoke("config_get");
    return { alive: true, providers: view.providers.length, skeletonMode: view.advanced.skeletonMode };
  } catch (error) {
    return { alive: false, error: String(error?.message ?? error) };
  }
})()`;
const JS_HOTKEY_PROBE = (spec) => `(async () => {
  const result = await window.__TAURI_INTERNALS__.invoke("hotkey_probe", { spec: ${JSON.stringify(spec)} });
  return result;
})()`;
const JS_EVENTS_SETUP = `(async () => {
  const internals = window.__TAURI_INTERNALS__;
  window.__s20Events = window.__s20Events ?? { tray: [] };
  if (!window.__s20TrayInstalled) {
    const handler = internals.transformCallback((payload) => {
      window.__s20Events.tray.push({ at: Date.now(), payload });
    });
    await internals.invoke("plugin:event|listen", { event: "tray:action", target: { kind: "Any" }, handler });
    window.__s20TrayInstalled = true;
  }
  return { installed: true, count: window.__s20Events.tray.length };
})()`;
const JS_EVENTS_READ = `(() => (window.__s20Events?.tray ?? []).map((entry) => ({ at: entry.at, ...(entry.payload?.payload ?? entry.payload) })))()`;

async function patchTogglePause(spec) {
  return evalWin(
    "main",
    `(async () => {
      const view = await window.__TAURI_INTERNALS__.invoke("config_patch", { patch: { hotkeys: { togglePause: ${JSON.stringify(spec)} } } });
      return { togglePause: view.hotkeys.togglePause, capture: view.hotkeys.capture };
    })()`,
  );
}

function summarizeEvents(events) {
  return events.map((event) => event.id ?? event);
}

async function main() {
  const evidence = {
    kind: "real-app-runtime",
    date: new Date().toISOString(),
    env: {
      os: "Windows 11 26200",
      cpu: "i7-12650H (10C16T)",
      ramGiB: 15.78,
      screens: "2560x1440@150% (0,0) + 1080x1920@100% (-1080,-470)",
      app: "src-tauri/target/debug/snip-ask.exe（applab 便携副本）",
      powershell: "Windows PowerShell 5.1（本机无 pwsh）",
      single_instance_plugin: "tauri-plugin-single-instance（第二次启动唤起已有实例并广播 tray:action{id:singleInstance}）",
    },
    singleInstance: { runs: [], summary: null },
    tray: { status: null, click: null, select: null, limitation: null },
    pauseResume: { runs: [], summary: null },
    quitRelease: { runs: [], summary: null },
    uncaught: null,
    cleanup: null,
  };

  let started = false;
  try {
    if (!ATTACH) {
      const start = applabStart({ reset: false, waitSec: 300 });
      if (start.code !== 0) throw new Error(`applab start 失败：${start.stderr || start.stdout}`);
      started = true;
    }
    await waitFor("main", `typeof window.__TAURI_INTERNALS__ !== "undefined"`, "Tauri IPC 就绪", 30000);
    evalWin("main", JS_INSTALL_UNCAUGHT);
    await evalWin("main", JS_EVENTS_SETUP);
    const basePids = listAppPids();
    evidence.singleInstance.baselinePids = basePids;
    const baselinePid = basePids[0];
    console.log(`[s20] 基线：snip-ask pid=${JSON.stringify(basePids)}`);

    // ---- A) 第二实例 ×10 ----
    for (let index = 1; index <= SINGLE_INSTANCE_RUNS; index += 1) {
      evalWin("main", JS_HIDE_MAIN);
      await sleep(400);
      const before = appWindows();
      const eventsBefore = evalWin("main", JS_EVENTS_READ);
      const launched = launchSecondInstance();
      // 等第二实例退出（单实例插件唤起已有实例后立即退出）
      let secondAlive = true;
      const deadline = Date.now() + 8000;
      while (Date.now() < deadline) {
        const pids = listAppPids();
        secondAlive = pids.includes(launched.pid) || pids.length > 1;
        if (!secondAlive) break;
        await sleep(250);
      }
      await sleep(600);
      const after = appWindows();
      const eventsAfter = evalWin("main", JS_EVENTS_READ);
      const pids = listAppPids();
      const row = {
        index,
        launchedPid: launched.pid,
        secondExitedMs: Date.now() - launched.startedAt,
        pids,
        pidCount: pids.length,
        originalPidAlive: pids.includes(baselinePid),
        mainVisibleBefore: mainWindowVisible(before),
        mainVisibleAfter: mainWindowVisible(after),
        invokedMainWindow: mainWindowVisible(before) === false && mainWindowVisible(after) === true,
        singleInstanceEvents: summarizeEvents(eventsAfter).filter((id) => id === "singleInstance").length -
          summarizeEvents(eventsBefore).filter((id) => id === "singleInstance").length,
        allEvents: summarizeEvents(eventsAfter).slice(-6),
      };
      row.pass =
        row.pidCount === 1 && row.originalPidAlive && row.singleInstanceEvents === 1 && row.invokedMainWindow === true;
      evidence.singleInstance.runs.push(row);
      console.log(
        `[s20] 第二实例 #${index}: pidCount=${row.pidCount} 唤起=${row.invokedMainWindow} 事件=${row.singleInstanceEvents} 退出=${row.secondExitedMs}ms pass=${row.pass}`,
      );
    }
    evidence.singleInstance.summary = {
      runs: evidence.singleInstance.runs.length,
      passed: evidence.singleInstance.runs.filter((row) => row.pass).length,
      avgExitMs: Math.round(
        evidence.singleInstance.runs.reduce((sum, row) => sum + row.secondExitedMs, 0) / evidence.singleInstance.runs.length,
      ),
    };
    saveRaw("s20", "single-instance.json", evidence.singleInstance);

    // ---- B) 托盘：宿主窗口 / Shell 注册 / 原生菜单自动化探测 ----
    trayAction("showicon");
    await sleep(500);
    const status = trayAction("status");
    evidence.tray.status = status;
    console.log(`[s20] 托盘宿主=${JSON.stringify(status.steps?.[1] ?? null)} icon=${JSON.stringify(status.icon)} uiaHits=${status.uiaHits?.length ?? "?"}`);
    shot("main", "s20/01-main-after-single-instance.png");
    const click = trayAction("click");
    evidence.tray.click = click;
    if (click.menu?.visible) {
      const select = trayAction("select", ["-ItemIndex", "1"]); // 1=设置
      evidence.tray.select = select;
      console.log(`[s20] 托盘菜单可见，已选择第 1 项（设置）：${JSON.stringify(select.steps?.slice(-1))}`);
      await sleep(600);
      const events = evalWin("main", JS_EVENTS_READ);
      evidence.tray.afterSelectEvents = summarizeEvents(events).slice(-5);
    } else {
      evidence.tray.limitation = {
        reason: "Win11 溢出区/前台激活限制：托盘图标向 Shell 注册成功（Shell_NotifyIconGetRect hr=S_OK），但原生 TrackPopupMenu 的 #32768 菜单窗口未出现，键盘/鼠标选择无法进行",
        taskbarNote:
          "本机任务栏为「自动隐藏」：隐藏时 Shell_TrayWnd 顶端位于 y≈1438（仅露出 2px），Shell_NotifyIconGetRect 返回的图标矩形也随之落在屏幕外（y 1438~1510），SetCursorPos 会被夹到屏幕边缘，真实点击无法命中图标；UIA 任务栏树中也检索不到该图标元素（隐藏/溢出态不可访问）",
        evidence: {
          trayHostFound: (status.steps ?? []).some((step) => step.step === "tray-host" && step.count === 1),
          iconRegistered: status.icon?.uid >= 0,
          iconRect: status.icon?.rect ?? null,
          uiaHits: status.uiaHits ?? [],
          clickMode: (click.steps ?? []).find((step) => step.step === "menu-after-click")?.mode ?? null,
          menuVisible: click.menu?.visible ?? false,
        },
        impact: "托盘菜单项的**动作正确性**改为按「同一处理函数 + 等价入口」验证（见 trayActionsEquivalent），并把原生菜单点击列为环境限制（与 S-04「局限性」一致，建议人工走查补充）",
      };
      console.log(`[s20] 托盘原生菜单不可见：${evidence.tray.limitation.reason}`);
      shot("main", "s20/02-tray-automation-fallback.png");
    }
    // 托盘「开始框选」动作的等价入口：capture_freeze（tray.rs 的 MENU_CAPTURE 调 do_capture_freeze）
    const capture = evalWin("main", `(async () => {
      const session = await window.__TAURI_INTERNALS__.invoke("capture_freeze");
      return { session };
    })()`);
    const overlayReady = await waitFor(
      "overlay",
      `document.readyState === "complete" && !!document.querySelector(".overlay-root") ? { ready: true } : null`,
      "overlay 就绪（等价托盘「开始框选」）",
      20000,
    );
    await sleep(300);
    // 关闭 overlay，避免遮挡
    evalWin("main", `(async () => { try { await window.__TAURI_INTERNALS__.invoke("capture_cancel", { session: ${JSON.stringify(capture.session)} }); return { cancelled: true }; } catch (error) { return { cancelled: false, error: String(error?.message ?? error) }; } })()`);
    evidence.tray.trayActionsEquivalent = {
      capture: { note: "MENU_CAPTURE → do_capture_freeze（与 IPC capture_freeze 同一函数）", session: capture.session, overlayReady: overlayReady.ready === true },
      settings: { note: "MENU_SETTINGS → show_main + tray:action{id:openSettings}", windowShowVerified: evalWin("main", JS_SHOW_MAIN) },
      rerunOnboarding: { note: "MENU_RERUN_ONBOARDING → show_main + tray:action{id:rerunOnboarding}（前端路由由 tray:action 驱动）" },
      togglePause: { note: "MENU_TOGGLE_PAUSE → hotkeys::toggle_pause（与 togglePause 热键同一函数），见 pauseResume" },
      quit: { note: "MENU_QUIT → lifecycle::shutdown + app.exit(0)（与 app_quit 同一路径），见 quitRelease" },
    };
    saveRaw("s20", "tray.json", evidence.tray);

    // ---- C) 暂停 / 恢复热键（真实 OS 注册探测）----
    const occupiedBefore = hotkeyProbe("Ctrl+Q");
    const probeIpcBefore = evalWin("main", JS_HOTKEY_PROBE("Ctrl+Q"));
    const bind = await patchTogglePause("Ctrl+Alt+P");
    await sleep(1200);
    const toggleProbeBefore = hotkeyProbe("Ctrl+Alt+P");
    evidence.pauseResume.setup = { occupiedBefore, probeIpcBefore, bind, toggleProbeBefore };
    for (let index = 1; index <= PAUSE_RUNS; index += 1) {
      const pauseKey = pressHotkey("Ctrl+Alt+P");
      await sleep(900);
      const duringPause = hotkeyProbe("Ctrl+Q");
      const ipcDuring = evalWin("main", JS_HOTKEY_PROBE("Ctrl+Q"));
      const resumeKey = pressHotkey("Ctrl+Alt+P");
      await sleep(900);
      const afterResume = hotkeyProbe("Ctrl+Q");
      const ipcAfter = evalWin("main", JS_HOTKEY_PROBE("Ctrl+Q"));
      const row = {
        index,
        pauseKey,
        resumeKey,
        duringPause,
        ipcDuring,
        afterResume,
        ipcAfter,
        pass:
          duringPause.registered === true && afterResume.registered === false && afterResume.win32Error === 1409,
      };
      evidence.pauseResume.runs.push(row);
      console.log(
        `[s20] 暂停/恢复 #${index}: 暂停后 Ctrl+Q 可注册=${duringPause.registered}（期望 true）恢复后 win32=${afterResume.win32Error} pass=${row.pass}`,
      );
    }
    evidence.pauseResume.summary = {
      runs: evidence.pauseResume.runs.length,
      passed: evidence.pauseResume.runs.filter((row) => row.pass).length,
    };
    // 还原 togglePause 绑定（避免影响后续成员）
    await patchTogglePause("");
    saveRaw("s20", "pause-resume.json", evidence.pauseResume);

    // ---- D) 正常退出后 Ctrl+Q 无残留 ×5（quit → probe → restart）----
    for (let index = 1; index <= QUIT_RELEASE_RUNS; index += 1) {
      const beforeQuit = hotkeyProbe("Ctrl+Q");
      const quit = (() => {
        try {
          return evalWin("main", JS_APP_QUIT);
        } catch (error) {
          return { quit: "ipc-closed", error: String(error.message ?? error) };
        }
      })();
      // 等进程完全退出
      let pids = listAppPids();
      const deadline = Date.now() + 15000;
      while (pids.length > 0 && Date.now() < deadline) {
        await sleep(300);
        pids = listAppPids();
      }
      await sleep(700);
      const afterQuit = hotkeyProbe("Ctrl+Q");
      const row = {
        index,
        beforeQuitWin32: beforeQuit.win32Error,
        quit,
        exited: pids.length === 0,
        afterQuit,
        pass: beforeQuit.registered === false && beforeQuit.win32Error === 1409 && pids.length === 0 && afterQuit.registered === true,
      };
      evidence.quitRelease.runs.push(row);
      console.log(
        `[s20] 退出释放 #${index}: 退出前 Ctrl+Q 占用=${beforeQuit.win32Error} 退出=${row.exited} 退出后可注册=${afterQuit.registered} pass=${row.pass}`,
      );
      if (index < QUIT_RELEASE_RUNS) {
        const start = applabStart({ reset: false, waitSec: 180 });
        if (start.code !== 0) throw new Error(`重启失败：${start.stderr || start.stdout}`);
        await waitFor("main", `typeof window.__TAURI_INTERNALS__ !== "undefined"`, "重启后 IPC 就绪", 30000);
        await sleep(800);
      }
    }
    evidence.quitRelease.summary = {
      runs: evidence.quitRelease.runs.length,
      passed: evidence.quitRelease.runs.filter((row) => row.pass).length,
    };
    saveRaw("s20", "quit-release.json", evidence.quitRelease);

    // 收尾：确认最终无残留 + 无未捕获异常（最后实例已退出，异常数据从启动日志与本轮 JSON 获取）
    evidence.cleanup = {
      finalPids: listAppPids(),
      finalProbeCtrlQ: hotkeyProbe("Ctrl+Q"),
    };
    saveDocsData("s20_runtime.json", {
      kind: evidence.kind,
      date: evidence.date,
      env: evidence.env,
      singleInstance: evidence.singleInstance.summary,
      singleInstanceRuns: evidence.singleInstance.runs.map((row) => ({
        index: row.index,
        pidCount: row.pidCount,
        invokedMainWindow: row.invokedMainWindow,
        singleInstanceEvents: row.singleInstanceEvents,
        secondExitedMs: row.secondExitedMs,
        pass: row.pass,
      })),
      tray: {
        hostFound: (evidence.tray.status?.steps ?? []).some((step) => step.step === "tray-host" && step.count === 1),
        iconRegistered: evidence.tray.status?.icon?.uid >= 0,
        iconUid: evidence.tray.status?.icon?.uid ?? null,
        nativeMenuVisible: evidence.tray.click?.menu?.visible ?? false,
        limitation: evidence.tray.limitation?.reason ?? null,
        trayActionsEquivalent: evidence.tray.trayActionsEquivalent,
      },
      pauseResume: evidence.pauseResume.summary,
      pauseResumeRuns: evidence.pauseResume.runs.map((row) => ({
        index: row.index,
        pausedCtrlQRegistrable: row.duringPause.registered,
        resumedWin32: row.afterResume.win32Error,
        pass: row.pass,
      })),
      quitRelease: evidence.quitRelease.summary,
      quitReleaseRuns: evidence.quitRelease.runs.map((row) => ({
        index: row.index,
        occupiedBeforeWin32: row.beforeQuitWin32,
        exited: row.exited,
        registrableAfter: row.afterQuit.registered,
        pass: row.pass,
      })),
      cleanup: evidence.cleanup,
    });
    console.log(
      `[s20] 完成：单实例 ${evidence.singleInstance.summary.passed}/${evidence.singleInstance.summary.runs}；暂停恢复 ${evidence.pauseResume.summary.passed}/${evidence.pauseResume.summary.runs}；退出释放 ${evidence.quitRelease.summary.passed}/${evidence.quitRelease.summary.runs}`,
    );
  } finally {
    if (!ATTACH && !KEEP) {
      applabStop();
      await sleep(800);
    }
    const pids = listAppPids();
    if (pids.length > 0 && !KEEP) {
      // 兜底：只清理本次验证产生的便携实例（applab stop 已 killAllApp）
      console.log(`[s20] 注意：仍有 snip-ask 进程 ${JSON.stringify(pids)}`);
    }
    console.log(`[s20] 清理：snip-ask 残留=${listAppPids().length}`);
  }
}

main().catch(async (error) => {
  console.error(`[s20] 失败：${error.message}`);
  saveRaw("s20", "failure.json", { at: new Date().toISOString(), message: String(error.message ?? error), stack: String(error.stack ?? "") });
  if (!ATTACH && !KEEP) {
    try {
      applabStop();
    } catch {
      /* ignore */
    }
  }
  process.exit(1);
});
