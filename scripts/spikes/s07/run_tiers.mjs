/**
 * item3 三档 AI 服务（云端 / 极小模型 / 普通模型）—— applab 真机走查驱动。
 *
 * 覆盖：
 *  1) --reset 首次启动 → 引导第 3/6 步（翻译方式）分别选三档并截图；
 *  2) 断言性能文案可见、选「极小/普通」时对应模型出现在下载列表；
 *  3) 每档选择经 onboarding_update 持久化（读 onboarding_state.data.translate 断言映射）；
 *  4) 最终选「极小模型」完成引导 → config_get 断言 config.translate.backend / config.local.model。
 *
 * 用法：
 *   node scripts/spikes/s07/run_tiers.mjs            # 自动 start --reset / stop
 *   node scripts/spikes/s07/run_tiers.mjs --attach   # 复用已启动实例（调试）
 *
 * 证据：.verify/spikes/S-07/shots/20-item3-*.png、.verify/spikes/S-07/raw/item3-*.json
 */

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..", "..");
const APPLAB = join(REPO, "scripts", "spikes", "applab", "applab.mjs");
const OUT_DIR = join(REPO, ".verify", "spikes", "S-07");
const SHOTS = join(OUT_DIR, "shots");
const RAW = join(OUT_DIR, "raw");
const OWNER = "onboarding-offline";
const ATTACH = new Set(process.argv.slice(2)).has("--attach");
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

const TINY_ID = "qwen2.5-0.5b-instruct-q4";
const STANDARD_ID = "hy-mt-1.8b-q4";

const evidence = { task: "item3 三档 AI 服务选择（S-07 扩展）", runAt: new Date().toISOString(), checks: {}, failed: [] };
function record(name, ok, detail) {
  evidence.checks[name] = { ok, detail };
  if (!ok) evidence.failed.push({ name, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}  ${JSON.stringify(detail)}`);
}

function applab(argv) {
  return execFileSync(process.execPath, [APPLAB, ...argv], {
    cwd: REPO,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
}
function evalWin(windowName, js) {
  const out = applab(["eval", "--window", windowName, "--js", js]);
  let parsed;
  try {
    parsed = JSON.parse(out);
  } catch {
    parsed = { ok: false, raw: out };
  }
  if (!parsed.ok) throw new Error(`eval(${windowName}) 失败：${JSON.stringify(parsed.exception ?? parsed)}`);
  return parsed.value;
}
async function waitFor(js, label, timeoutMs = 20000, intervalMs = 300) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    try {
      last = evalWin("main", js);
      if (last) return last;
    } catch (error) {
      last = String(error.message ?? error);
    }
    await sleep(intervalMs);
  }
  throw new Error(`等待超时（${label}）：${JSON.stringify(last)}`);
}
function shot(name) {
  const out = join(SHOTS, name);
  applab(["shot", "--window", "main", "--out", out]);
  evidence.screenshots = evidence.screenshots ?? [];
  evidence.screenshots.push(out);
  console.log(`SHOT  ${out}`);
}
function saveRaw(name, value) {
  const out = join(RAW, name);
  writeFileSync(out, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  evidence.rawFiles = evidence.rawFiles ?? [];
  evidence.rawFiles.push(out);
  console.log(`RAW   ${out}`);
}

const JS_SHOW_MAIN = `window.__TAURI_INTERNALS__.invoke("window_show", { window: "main" }).then(() => true)`;
const JS_STEP_INFO = `(() => {
  const counter = document.querySelector(".ob-step-counter");
  return counter ? counter.textContent.trim() : null;
})()`;
const JS_CLICK_NEXT = `(() => {
  const button = [...document.querySelectorAll(".ob-footer-right button")].find((b) =>
    /下一步|完成并进入/.test(b.textContent || ""),
  );
  if (!button) return { clicked: false };
  button.click();
  return { clicked: true, label: button.textContent.trim() };
})()`;
const JS_CLICK_PREV = `(() => {
  const button = [...document.querySelectorAll(".ob-footer-right button")].find((b) =>
    /上一步/.test(b.textContent || ""),
  );
  if (!button) return { clicked: false };
  button.click();
  return { clicked: true };
})()`;
const JS_CLICK_TIER = (title) => `(() => {
  const cards = [...document.querySelectorAll(".ob-cards3 .ob-card")];
  const card = cards.find((c) => (c.textContent || "").includes(${JSON.stringify(title)}));
  if (!card) return { clicked: false, cards: cards.map((c) => c.textContent?.slice(0, 30)) };
  card.click();
  return { clicked: true };
})()`;
const JS_READ_STEP3 = `(() => {
  const cards = [...document.querySelectorAll(".ob-cards3 .ob-card")].map((card) => ({
    title: card.querySelector(".ob-card-title")?.textContent?.trim() ?? null,
    selected: card.classList.contains("is-selected"),
    text: (card.innerText || "").replace(/\\s+/g, " ").trim(),
  }));
  const models = [...document.querySelectorAll(".ob-model")].map((model) => ({
    title: model.querySelector(".ob-model-title strong")?.textContent?.trim() ?? null,
    text: (model.innerText || "").replace(/\\s+/g, " ").trim(),
    selected: model.classList.contains("is-selected"),
  }));
  const bodyText = (document.querySelector(".ob-step")?.innerText || "").replace(/\\s+/g, " ").trim();
  return { cards, models, bodyText, stepLabel: document.querySelector(".ob-step-counter")?.textContent?.trim() ?? null };
})()`;
const JS_READ_TRANSLATE_DRAFT = `(async () => {
  const state = await window.__TAURI_INTERNALS__.invoke("onboarding_state");
  const config = await window.__TAURI_INTERNALS__.invoke("config_get");
  return {
    translate: state.data.translate,
    currentStep: state.currentStep,
    configBackend: config.translate.backend,
    configLocalModel: config.local.model,
    configCloudModel: config.translate.model,
  };
})()`;
const JS_AGREE_PRIVACY = `(() => {
  const boxes = [...document.querySelectorAll(".ob-checkline input[type=checkbox]")];
  if (boxes.length < 3) return { ok: false, count: boxes.length };
  const agreed = boxes[boxes.length - 1];
  if (!agreed.checked) agreed.click();
  return { ok: agreed.checked, count: boxes.length };
})()`;

async function openStepThree() {
  await sleep(1200);
  for (let i = 0; i < 2; i += 1) {
    const before = await waitFor(JS_STEP_INFO, "引导步骤渲染");
    evalWin("main", JS_CLICK_NEXT);
    await waitFor(
      `(() => { const c = document.querySelector(".ob-step-counter")?.textContent?.trim() ?? null; return c && c !== ${JSON.stringify(before)} ? c : null; })()`,
      "下一步切换",
    );
  }
  const label = evalWin("main", JS_STEP_INFO);
  return label;
}

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  mkdirSync(SHOTS, { recursive: true });
  mkdirSync(RAW, { recursive: true });
  try {
    if (!ATTACH) {
      console.log("[item3] 启动便携应用（--reset = 首次启动）…");
      applab(["start", "--owner", OWNER, "--wait-sec", "300", "--reset"]);
    }
    await waitFor(`typeof window.__TAURI_INTERNALS__ !== "undefined"`, "Tauri IPC 就绪", 30000);
    for (let i = 0; i < 3; i += 1) {
      try {
        evalWin("main", JS_SHOW_MAIN);
        break;
      } catch {
        await sleep(500);
      }
    }
    await waitFor(`!!document.querySelector(".ob-root")`, "首启引导自动弹出", 30000);
    const stepLabel = await openStepThree();
    record("tiers.step3_reached", /翻译方式/.test(stepLabel ?? ""), { stepLabel });

    const tiers = [
      {
        key: "cloud",
        title: "云端翻译",
        expectBackend: "cloud",
        expectModel: null,
        expectText: ["≤1.5s", "S-05 云端实测待补"],
        screenshot: "20-item3-tier-cloud.png",
      },
      {
        key: "tiny",
        title: "极小模型",
        expectBackend: "local",
        expectModel: TINY_ID,
        expectText: ["470 MB", "预估", "低配"],
        expectModelTitle: "Qwen2.5-0.5B",
        screenshot: "21-item3-tier-tiny.png",
      },
      {
        key: "standard",
        title: "普通模型",
        expectBackend: "local",
        expectModel: STANDARD_ID,
        expectText: ["11.76s", "21.84s", "8s 目标仅云端满足"],
        expectModelTitle: "HY-MT1.5-1.8B",
        screenshot: "22-item3-tier-standard.png",
      },
    ];

    for (const tier of tiers) {
      const click = evalWin("main", JS_CLICK_TIER(tier.title));
      record(`tiers.${tier.key}.card_clicked`, click.clicked === true, click);
      await sleep(500);
      const dom = evalWin("main", JS_READ_STEP3);
      shot(tier.screenshot);
      saveRaw(`item3-${tier.key}-dom.json`, dom);
      const card = dom.cards.find((item) => (item.title ?? "").includes(tier.title));
      record(`tiers.${tier.key}.selected`, card?.selected === true, card);
      const missing = tier.expectText.filter((text) => !(dom.bodyText ?? "").includes(text));
      record(`tiers.${tier.key}.perf_copy_visible`, missing.length === 0, { missing, bodyText: dom.bodyText.slice(0, 500) });
      if (tier.expectModelTitle) {
        const model = dom.models.find((item) => (item.title ?? "").includes(tier.expectModelTitle));
        record(`tiers.${tier.key}.model_in_download_list`, !!model, {
          expectModelTitle: tier.expectModelTitle,
          models: dom.models.map((item) => item.title),
        });
      } else {
        record(`tiers.${tier.key}.no_local_model_list`, dom.models.length === 0, {
          models: dom.models.map((item) => item.title),
        });
      }
      // 持久化映射：下一步保存 step2 数据 → 读 onboarding_state，再回上一步继续测下一档。
      evalWin("main", JS_CLICK_NEXT);
      await waitFor(`(document.querySelector(".ob-step-counter")?.textContent ?? "").includes("AI 模式")`, "到第 4 步");
      const draft = evalWin("main", JS_READ_TRANSLATE_DRAFT);
      record(`tiers.${tier.key}.draft_mapping`, draft.translate?.backend === tier.expectBackend && (draft.translate?.modelId ?? null) === tier.expectModel, {
        translate: draft.translate,
        expectBackend: tier.expectBackend,
        expectModel: tier.expectModel,
      });
      // 每档读到映射后回到第 3 步继续下一档；普通模型档也回退，便于做切换清理校验并继续完成引导。
      evalWin("main", JS_CLICK_PREV);
      await waitFor(`(document.querySelector(".ob-step-counter")?.textContent ?? "").includes("翻译方式")`, "回到翻译方式");
    }

    // 最终选中「普通模型」，继续完成引导；先切到另一个档再切回，验证切换清理无关字段。
    evalWin("main", JS_CLICK_TIER("云端翻译"));
    await sleep(300);
    evalWin("main", JS_CLICK_TIER("普通模型"));
    await sleep(300);
    const switched = evalWin("main", JS_READ_TRANSLATE_DRAFT);
    record("tiers.switch_cleanup", switched.translate?.backend === "local" && switched.translate?.modelId === STANDARD_ID && switched.translate?.cloudModel === "", {
      translate: switched.translate,
    });

    // 剩余步骤：AI 模式 → 外观与行为 → 隐私与完成。
    for (let i = 0; i < 3; i += 1) {
      const before = evalWin("main", JS_STEP_INFO);
      evalWin("main", JS_CLICK_NEXT);
      await waitFor(
        `(() => { const c = document.querySelector(".ob-step-counter")?.textContent?.trim() ?? null; return c && c !== ${JSON.stringify(before)} ? c : null; })()`,
        "继续下一步",
      );
    }
    const last = evalWin("main", JS_STEP_INFO);
    record("tiers.finish_on_last_step", /隐私与完成/.test(last ?? ""), { stepLabel: last });
    const agree = evalWin("main", JS_AGREE_PRIVACY);
    record("tiers.privacy_agreed", agree.ok === true, agree);
    await waitFor(
      `(() => { const b = [...document.querySelectorAll(".ob-footer-right button")].find((x) => /完成并进入/.test(x.textContent || "")); return b && !b.disabled ? true : null; })()`,
      "完成按钮可点击",
    );
    evalWin("main", JS_CLICK_NEXT);
    const result = await waitFor(
      `(async () => {
        const onboarding = await window.__TAURI_INTERNALS__.invoke("onboarding_state");
        const config = await window.__TAURI_INTERNALS__.invoke("config_get");
        return onboarding.completed
          ? { onboarding, backend: config.translate.backend, localModel: config.local.model, cloudModel: config.translate.model }
          : null;
      })()`,
      "引导完成",
      20000,
    );
    saveRaw("item3-final-config.json", result);
    shot("23-item3-final-config.png");
    record("tiers.finish_config", result.onboarding.completed === true && result.backend === "local" && result.localModel === STANDARD_ID, {
      backend: result.backend,
      localModel: result.localModel,
      cloudModel: result.cloudModel,
    });

    saveRaw("item3-evidence.json", evidence);
    console.log(`\n===== item3 三档汇总：断言 ${Object.keys(evidence.checks).length}，失败 ${evidence.failed.length} =====`);
    if (evidence.failed.length > 0) {
      console.log(JSON.stringify(evidence.failed, null, 2));
      process.exitCode = 1;
    }
  } catch (error) {
    evidence.error = String(error?.stack ?? error);
    saveRaw("item3-evidence.json", evidence);
    console.error(`[item3] 失败：${error?.stack ?? error}`);
    process.exitCode = 1;
  } finally {
    if (!ATTACH) {
      try {
        applab(["stop", "--owner", OWNER]);
      } catch (error) {
        console.error(`[item3] stop 失败：${error.message}`);
      }
    }
  }
}

await main();
