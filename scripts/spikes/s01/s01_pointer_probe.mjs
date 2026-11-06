/**
 * S-01 overlay 跨屏拖选指针探针（真实应用 + CDP 合成指针事件）。
 *
 * 做什么：
 *   1. 连接 WebView2 CDP，定位 overlay 页面，读取 `devicePixelRatio / innerWidth / innerHeight`；
 *   2. 用 `Input.dispatchMouseEvent` 派发真实指针拖拽（mousePressed → mouseMoved… → mouseReleased），
 *      覆盖「跨屏正向 / 反向 / 进入负坐标 / 跨 x=0 交界 / 近全虚拟桌面」；
 *   3. 拖拽过程中读取 `.overlay-selection__box` 的 CSS 位置与尺寸标签（物理像素），
 *      按 `物理 = virtualRect原点 + CSS × devicePixelRatio` 回算物理坐标，与期望落点比较（≤2px）；
 *   4. 最后做一次有效跨屏拖选并 release（触发 selection_commit），留证据供全屏截图；
 *   5. 结果写 `.verify/spikes/S-01/s01_pointer_probe.json`。
 *
 * 关键不变式：overlay WebView 的 devicePixelRatio 必须 **等于** 后端下发的 dpr
 * （payload.dpr）。若不等，回算物理坐标会按比例整体偏移，本脚本会把它量化为 px 误差。
 *
 * 用法（仓库根目录，applab 已 start）：
 *   node scripts/spikes/s01/s01_pointer_probe.mjs \
 *     --vd -1080,-470,3640,1920 --out .verify/spikes/S-01/s01_pointer_probe.json
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const CDP_PORT = 9333;

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (!arg.startsWith("--")) continue;
    const key = arg.slice(2);
    const next = argv[i + 1];
    if (next !== undefined && !next.startsWith("--")) {
      out[key] = next;
      i += 1;
    } else {
      out[key] = true;
    }
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
const vd = String(args.vd ?? "-1080,-470,3640,1920")
  .split(",")
  .map((value) => Number(value.trim()));
if (vd.length !== 4 || vd.some((value) => !Number.isFinite(value))) {
  console.error("非法 --vd，应为 x,y,width,height");
  process.exit(3);
}
const [vdX, vdY, vdW, vdH] = vd;
const outPath = resolve(process.cwd(), String(args.out ?? ".verify/spikes/S-01/s01_pointer_probe.json"));

async function cdpTargets() {
  const response = await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`);
  return response.json();
}

function connectCdp(wsUrl) {
  return new Promise((resolvePromise, rejectPromise) => {
    const socket = new WebSocket(wsUrl);
    socket.addEventListener("open", () => resolvePromise(socket));
    socket.addEventListener("error", (event) =>
      rejectPromise(new Error(String(event?.message ?? "CDP 连接失败"))),
    );
  });
}

function withCdp(target, action) {
  return new Promise((resolvePromise, rejectPromise) => {
    const socket = new WebSocket(target.webSocketDebuggerUrl);
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
    socket.addEventListener("error", (event) =>
      rejectPromise(new Error(String(event?.message ?? "CDP 连接失败"))),
    );
    socket.addEventListener("open", async () => {
      const send = (method, params = {}) =>
        new Promise((done, reject) => {
          nextId += 1;
          pending.set(nextId, { resolve: done, reject });
          socket.send(JSON.stringify({ id: nextId, method, params }));
        });
      try {
        await send("Runtime.enable");
        resolvePromise(await action(send));
      } catch (error) {
        rejectPromise(error);
      } finally {
        socket.close();
      }
    });
  });
}

async function evaluate(send, expression) {
  const result = await send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
    userGesture: true,
  });
  if (result.exceptionDetails) {
    throw new Error(`页面 JS 异常：${JSON.stringify(result.exceptionDetails)}`);
  }
  return result.result?.value;
}

async function evaluateJson(send, expression) {
  const value = await evaluate(send, `JSON.stringify(${expression})`);
  return value ? JSON.parse(value) : null;
}

async function dispatchMouse(send, type, x, y, buttons) {
  await send("Input.dispatchMouseEvent", {
    type,
    x,
    y,
    button: type === "mouseMoved" ? (buttons ? "left" : "none") : "left",
    buttons,
    clickCount: 1,
    pointerType: "mouse",
  });
}

/** overlay 当前选框状态（CSS 尺寸 + 标签中的物理尺寸）。 */
const READ_SELECTION = `(() => {
  const box = document.querySelector(".overlay-selection__box");
  if (!box) return { hasBox: false };
  const style = box.style;
  const labelNode = box.querySelector(".overlay-selection__size");
  const label = labelNode ? labelNode.textContent : "";
  const match = /宽\\s*(\\d+)\\s*×\\s*高\\s*(\\d+)/.exec(label || "");
  const rect = box.getBoundingClientRect();
  return {
    hasBox: true,
    left: parseFloat(style.left),
    top: parseFloat(style.top),
    width: parseFloat(style.width),
    height: parseFloat(style.height),
    label,
    labelWidth: match ? Number(match[1]) : null,
    labelHeight: match ? Number(match[2]) : null,
    domRect: { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
  };
})()`;

function normalizeRect(a, b) {
  return {
    x: Math.min(a[0], b[0]),
    y: Math.min(a[1], b[1]),
    width: Math.abs(b[0] - a[0]),
    height: Math.abs(b[1] - a[1]),
  };
}

function toCss(pointPhys, dpr) {
  return [(pointPhys[0] - vdX) / dpr, (pointPhys[1] - vdY) / dpr];
}

const cases = [
  { name: "cross_primary_to_secondary", start: [100, 100], end: [-900, 800] },
  { name: "cross_secondary_to_primary", start: [-900, 800], end: [100, 100] },
  { name: "cross_into_negative_y", start: [1280, 720], end: [-540, -300] },
  { name: "straddle_x0_boundary", start: [-200, 700], end: [200, 700] },
  { name: "near_full_virtual_desktop", start: [-1000, -400], end: [2500, 1380] },
  { name: "negative_area_reverse_drag", start: [-900, 800], end: [-100, 100] },
];

async function main() {
  const targets = await cdpTargets();
  const overlayTarget = targets.find((target) =>
    String(target.url ?? "").includes("window=overlay"),
  );
  if (!overlayTarget) {
    throw new Error(
      `未找到 overlay webview；当前 targets=${JSON.stringify(targets.map((t) => t.url))}`,
    );
  }

  const result = await withCdp(overlayTarget, async (send) => {
    const env = await evaluateJson(
      send,
      "{dpr: window.devicePixelRatio, innerWidth: window.innerWidth, innerHeight: window.innerHeight, " +
        "screenX: window.screenX, screenY: window.screenY, hasTauri: !!window.__TAURI_INTERNALS__, " +
        "phaseHint: document.querySelector('.overlay-selection') ? 'selecting-layer' : 'no-layer'}",
    );
    const dpr = env.dpr;
    const expectedSelectionCss = { width: vdW / dpr, height: vdH / dpr };
    // 若 overlay 尚未进入选区态（页面刚加载），先补一次 capture_freeze（幂等）。
    if (env.phaseHint === "no-layer") {
      await evaluate(
        send,
        "window.__TAURI_INTERNALS__.invoke('capture_freeze').catch(() => undefined)",
      );
      await sleep(1500);
    }

    const measurements = [];
    let latest = null;
    for (const testCase of cases) {
      const startCss = toCss(testCase.start, dpr);
      const endCss = toCss(testCase.end, dpr);
      // 真实拖拽：起点按下 → 两段中间移动 → 终点
      await dispatchMouse(send, "mouseMoved", startCss[0], startCss[1], 0);
      await sleep(30);
      await dispatchMouse(send, "mousePressed", startCss[0], startCss[1], 1);
      await sleep(30);
      const mid1 = [startCss[0] + (endCss[0] - startCss[0]) * 0.4, startCss[1] + (endCss[1] - startCss[1]) * 0.4];
      await dispatchMouse(send, "mouseMoved", mid1[0], mid1[1], 1);
      const mid2 = [startCss[0] + (endCss[0] - startCss[0]) * 0.75, startCss[1] + (endCss[1] - startCss[1]) * 0.75];
      await dispatchMouse(send, "mouseMoved", mid2[0], mid2[1], 1);
      await dispatchMouse(send, "mouseMoved", endCss[0], endCss[1], 1);
      await sleep(80);

      const selection = await evaluateJson(send, READ_SELECTION);
      latest = selection;
      const expected = normalizeRect(testCase.start, testCase.end);
      const item = {
        name: testCase.name,
        startPhys: testCase.start,
        endPhys: testCase.end,
        expectedPhys: expected,
        cssStart: startCss.map((value) => Number(value.toFixed(3))),
        cssEnd: endCss.map((value) => Number(value.toFixed(3))),
        selection,
      };
      if (selection?.hasBox) {
        // 回算物理坐标：物理 = virtualRect 原点 + CSS × WebView devicePixelRatio
        const derived = {
          x: vdX + selection.left * dpr,
          y: vdY + selection.top * dpr,
          width: selection.width * dpr,
          height: selection.height * dpr,
        };
        // payload.dpr 与 WebView dpr 一致性：标签物理宽 / CSS 宽（期望 = dpr）
        const impliedDpr =
          selection.labelWidth != null && selection.width > 0.5
            ? selection.labelWidth / selection.width
            : null;
        item.derivedPhys = {
          x: Number(derived.x.toFixed(2)),
          y: Number(derived.y.toFixed(2)),
          width: Number(derived.width.toFixed(2)),
          height: Number(derived.height.toFixed(2)),
        };
        item.errors = {
          dx: Number((derived.x - expected.x).toFixed(2)),
          dy: Number((derived.y - expected.y).toFixed(2)),
          dwidth: Number((derived.width - expected.width).toFixed(2)),
          dheight: Number((derived.height - expected.height).toFixed(2)),
          maxAbs: Number(
            Math.max(
              Math.abs(derived.x - expected.x),
              Math.abs(derived.y - expected.y),
              Math.abs(derived.width - expected.width),
              Math.abs(derived.height - expected.height),
            ).toFixed(2),
          ),
        };
        item.impliedPayloadDpr = impliedDpr == null ? null : Number(impliedDpr.toFixed(4));
        item.labelMatchesExpected =
          selection.labelWidth == null
            ? null
            : Math.abs(selection.labelWidth - Math.round(expected.width)) <= 1 &&
              Math.abs(selection.labelHeight - Math.round(expected.height)) <= 1;
      }

      // 收尾：把指针移回起点让选框塌缩（宽高 <3px），再松开 → 不提交，保持选区态可继续测。
      await dispatchMouse(send, "mouseMoved", startCss[0], startCss[1], 1);
      await sleep(20);
      await dispatchMouse(send, "mouseReleased", startCss[0], startCss[1], 0);
      await sleep(80);
      measurements.push(item);
    }

    // 证据截图（overlay webview 局部）：最后留一个有效跨屏选框并提交，供菜单/全屏截图对照。
    const evidence = { name: "evidence_cross_screen_commit", start: [120, 120], end: [-880, 820] };
    const startCss = toCss(evidence.start, dpr);
    const endCss = toCss(evidence.end, dpr);
    await dispatchMouse(send, "mouseMoved", startCss[0], startCss[1], 0);
    await dispatchMouse(send, "mousePressed", startCss[0], startCss[1], 1);
    await dispatchMouse(send, "mouseMoved", endCss[0], endCss[1], 1);
    await sleep(80);
    const evidenceSelection = await evaluateJson(send, READ_SELECTION);
    const shot = await send("Page.captureScreenshot", { format: "png" });
    await dispatchMouse(send, "mouseReleased", endCss[0], endCss[1], 0);
    await sleep(1200);
    const afterCommit = await evaluateJson(
      send,
      "{hasMenu: !!document.querySelector('.overlay-menu'), commitPhase: document.body.innerText.includes('正在识别') || document.body.innerText.includes('识别中'), box: !!document.querySelector('.overlay-selection__box')}",
    );

    return {
      generatedAt: new Date().toISOString(),
      expectedVirtualDesktop: { x: vdX, y: vdY, width: vdW, height: vdH },
      webview: env,
      expectedSelectionCss,
      measurements,
      evidence: {
        startPhys: evidence.start,
        endPhys: evidence.end,
        selection: evidenceSelection,
        afterCommit,
        screenshotBase64Png: shot.data,
      },
    };
  });

  // 汇总
  const rows = result.measurements.map((item) => ({
    name: item.name,
    expectedPhys: item.expectedPhys,
    derivedPhys: item.derivedPhys ?? null,
    errors: item.errors ?? null,
    impliedPayloadDpr: item.impliedPayloadDpr ?? null,
    labelMatchesExpected: item.labelMatchesExpected ?? null,
  }));
  const validRows = rows.filter((row) => row.errors);
  const maxError = validRows.length
    ? Math.max(...validRows.map((row) => row.errors.maxAbs))
    : null;
  const dprMatches = validRows.every(
    (row) => row.impliedPayloadDpr != null && Math.abs(row.impliedPayloadDpr - result.webview.dpr) < 0.01,
  );
  const summary = {
    result:
      validRows.length === result.measurements.length && maxError != null && maxError <= 2 && dprMatches
        ? "DONE"
        : "FAILED",
    devicePixelRatio: result.webview.dpr,
    expectedSelectionCss: result.expectedSelectionCss,
    webviewInner: { width: result.webview.innerWidth, height: result.webview.innerHeight },
    cases: validRows.length,
    maxErrorPx: maxError,
    meanErrorPx: validRows.length
      ? Number(
          (
            validRows.reduce((sum, row) => sum + row.errors.maxAbs, 0) / validRows.length
          ).toFixed(3),
        )
      : null,
    payloadDprMatchesWebviewDpr: dprMatches,
    rows,
    afterCommit: result.evidence.afterCommit,
  };

  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(
    outPath,
    `${JSON.stringify(
      {
        summary,
        webview: result.webview,
        expectedVirtualDesktop: result.expectedVirtualDesktop,
        measurements: result.measurements.map(({ selection, ...rest }) => ({ ...rest, selection })),
        evidence: { ...result.evidence, screenshotBase64Png: undefined },
      },
      null,
      2,
    )}\n`,
  );
  // 证据截图单独落盘
  const shotPath = outPath.replace(/\.json$/, "-overlay.png");
  if (result.evidence.screenshotBase64Png) {
    writeFileSync(shotPath, Buffer.from(result.evidence.screenshotBase64Png, "base64"));
  }

  console.log("== S-01 overlay 跨屏拖选指针探针 ==");
  console.log(`overlay WebView: dpr=${result.webview.dpr} inner=${result.webview.innerWidth}x${result.webview.innerHeight} window=(${result.webview.screenX},${result.webview.screenY})`);
  console.log(`期望 CSS 视口（= 虚拟桌面物理/dpr）：${result.expectedSelectionCss.width.toFixed(2)}x${result.expectedSelectionCss.height.toFixed(2)}`);
  for (const row of summary.rows) {
    console.log(
      `  ${row.name.padEnd(30)} 期望=${JSON.stringify(row.expectedPhys)} 回算=${JSON.stringify(row.derivedPhys)} err=${JSON.stringify(row.errors)} impliedDpr=${row.impliedPayloadDpr} labelOk=${row.labelMatchesExpected}`,
    );
  }
  console.log(
    `结论：${summary.result}  最大误差=${summary.maxErrorPx}px  平均=${summary.meanErrorPx}px  payloadDpr==webviewDpr=${summary.payloadDprMatchesWebviewDpr}`,
  );
  console.log(`commit 后状态：${JSON.stringify(summary.afterCommit)}`);
  console.log(`输出：${outPath}`);
  console.log(`截图：${shotPath}`);
  process.exit(summary.result === "DONE" ? 0 : 1);
}

main().catch((error) => {
  console.error(`[s01-pointer] 失败：${error.message}`);
  process.exit(1);
});
