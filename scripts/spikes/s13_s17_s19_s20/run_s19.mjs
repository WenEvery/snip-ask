/**
 * S-19 自动走查：`--reset` 从首次启动（无 Key、不下载模型）完整走完 6 步引导，逐步计时 + 截图。
 *
 * 定位：这是**自动走查（参考数据）**，不是真人验收；真人 3 人走查见
 * `scripts/spikes/s13_s17_s19_s20/S-19-新用户走查协议.md`（仍待招人，S-19 状态 BLOCKED）。
 *
 * 用法（仓库根目录）：
 *   node scripts/spikes/s13_s17_s19_s20/run_s19.mjs [--attach] [--keep]
 *     --attach  复用已在跑的 applab 应用（不自行 start/stop；需先 --reset 起好）
 *     --keep    跑完不停止应用
 *
 * 原始数据：`.verify/spikes/S-13-S-17-S-19-S-20/raw/s19/`；截图：`.../shots/s19/`；
 * 聚合：`docs/spikes/data/s13_s17_s19_s20/s19_auto_walkthrough.json`。
 */
import {
  JS_INSTALL_UNCAUGHT,
  JS_READ_UNCAUGHT,
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

const JS_STEP_INFO = `(() => {
  const counter = document.querySelector(".ob-step-counter");
  const title = document.querySelector(".ob-step-title, .ob-hero-title");
  const primary = [...document.querySelectorAll(".ob-footer-right button")].find((b) =>
    /下一步|完成并进入|完成中/.test(b.textContent || ""),
  );
  const cards = [...document.querySelectorAll(".ob-card")].map((card) => ({
    title: card.querySelector(".ob-card-title")?.textContent?.trim() ?? "",
    desc: card.querySelector(".ob-card-desc")?.textContent?.trim() ?? "",
    meta: card.querySelector(".ob-tier-meta")?.innerText?.trim() ?? null,
    selected: card.getAttribute("aria-pressed") === "true",
  }));
  return {
    counter: counter ? counter.textContent.trim() : null,
    stepTitle: title ? title.textContent.trim() : null,
    primaryLabel: primary ? primary.textContent.trim() : null,
    primaryDisabled: primary ? primary.disabled : null,
    skipLinkVisible: !!document.querySelector(".ob-footer-left .ob-linkbtn"),
    restrictedNotice: !!document.querySelector('[data-testid="restricted-notice"]'),
    cards,
    renderMark: (window.__s19RenderMark = (window.__s19RenderMark ?? 0)),
  };
})()`;

const JS_CLICK_NEXT = `(() => {
  const primary = [...document.querySelectorAll(".ob-footer-right button")].find((b) =>
    /下一步|完成并进入/.test(b.textContent || ""),
  );
  if (!primary) return { clicked: false, reason: "找不到下一步按钮" };
  if (primary.disabled) return { clicked: false, reason: "下一步按钮禁用", label: primary.textContent.trim() };
  primary.click();
  return { clicked: true, label: primary.textContent.trim() };
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
  const [onboarding, config] = await Promise.all([invoke("onboarding_state"), invoke("config_get")]);
  return {
    completed: onboarding.completed,
    skipped: onboarding.skipped,
    currentStep: onboarding.currentStep,
    finishedAt: onboarding.data?.finishedAt ?? null,
    translateBackend: config.translate.backend,
    localModel: config.local.model,
    providers: config.providers.map((p) => ({ id: p.id, enabled: p.enabled, hasKey: p.hasKey })),
    skeletonMode: config.advanced.skeletonMode,
    restrictedBanner: !!document.querySelector('[data-testid="restricted-notice"]'),
    restrictedBannerText: document.querySelector('[data-testid="restricted-notice"]')?.innerText ?? null,
  };
})()`;

async function main() {
  const started = Date.now();
  const runs = [];
  runs.push({
    at: new Date().toISOString(),
    label: "自动走查开始",
    note: ATTACH ? "attach 模式（应用已由外部启动）" : "applab start --reset（首次启动）",
  });

  if (!ATTACH) {
    const start = applabStart({ reset: true, waitSec: 300 });
    if (start.code !== 0) throw new Error(`applab start 失败：${start.stderr || start.stdout}`);
    runs.push({ at: new Date().toISOString(), label: "应用与 WebView2 CDP 就绪", elapsedMs: Date.now() - started });
  }
  const launchMs = Date.now() - started;

  // 首次启动应自动进入引导（隐藏窗口也渲染；CDP 可直接驱动）。
  const first = await waitFor(
    "main",
    `(() => {
      const info = ${JS_STEP_INFO};
      return info.counter && document.querySelector(".ob-root") ? info : null;
    })()`,
    "首启引导自动打开",
    45000,
  );
  const onboardingShownAt = Date.now();
  evalWin("main", JS_INSTALL_UNCAUGHT);
  runs.push({
    at: new Date().toISOString(),
    label: "引导首屏渲染",
    counter: first.counter,
    stepTitle: first.stepTitle,
    msSinceLaunch: onboardingShownAt - started,
  });

  const steps = [];
  let counter = first.counter;
  for (let index = 0; index < 6; index += 1) {
    const stepStart = Date.now();
    const info = index === 0 ? first : await waitFor(
      "main",
      `((info) => (info.counter ? info : null))(${JS_STEP_INFO})`,
      `第 ${index + 1} 步渲染`,
      20000,
    );
    const shotInfo = shot("main", `s19/step${index + 1}-${String(info.stepTitle ?? "unknown").replace(/[\\/:*?"<>|]/g, "_")}.png`);
    steps.push({
      index: index + 1,
      counter: info.counter,
      stepTitle: info.stepTitle,
      primaryLabel: info.primaryLabel,
      primaryDisabled: info.primaryDisabled,
      skipLinkVisible: info.skipLinkVisible,
      restrictedNotice: info.restrictedNotice,
      cards: info.cards,
      screenshot: shotInfo.path,
      at: new Date().toISOString(),
    });
    console.log(`[s19] STEP ${index + 1}/6 ${info.counter} ${info.stepTitle} cards=${info.cards.length}`);

    if (index < 5) {
      const click = evalWin("main", JS_CLICK_NEXT);
      if (!click.clicked) throw new Error(`第 ${index + 1} 步无法进入下一步：${click.reason}`);
      const next = await waitFor(
        "main",
        `((info) => (info.counter && info.counter !== ${JSON.stringify(counter)} ? info : null))(${JS_STEP_INFO})`,
        `第 ${index + 2} 步出现`,
        20000,
      );
      counter = next.counter;
      steps[steps.length - 1].stepMs = Date.now() - stepStart;
      steps[steps.length - 1].clickedNext = click;
    } else {
      // 第 6 步：勾选隐私同意 → 完成并进入。
      const agree = evalWin("main", JS_AGREE_PRIVACY);
      steps[steps.length - 1].agreePrivacy = agree;
      await waitFor(
        "main",
        `(() => {
          const b = [...document.querySelectorAll(".ob-footer-right button")].find((x) => /完成并进入/.test(x.textContent || ""));
          return b && !b.disabled;
        })()`,
        "完成按钮可点击",
        15000,
      );
      const finish = evalWin("main", JS_CLICK_NEXT);
      steps[steps.length - 1].finish = finish;
      steps[steps.length - 1].stepMs = Date.now() - stepStart;
      await waitFor(
        "main",
        `(async () => {
          const state = await window.__TAURI_INTERNALS__.invoke("onboarding_state");
          return state.completed === true ? { completed: true } : null;
        })()`,
        "引导完成状态",
        20000,
      );
    }
  }
  const completedAt = Date.now();
  const state = await waitFor("main", JS_ONBOARDING_STATE, "引导完成后的主界面状态", 20000);
  await sleep(600);
  shot("main", "s19/after-finish-main.png");
  const uncaught = evalWin("main", JS_READ_UNCAUGHT);

  const totalMs = completedAt - onboardingShownAt;
  const raw = {
    kind: "automated-walkthrough",
    note: "自动走查（参考数据），不代表真人新用户验收；口径：首次启动到第 6 步点击「完成并进入」",
    startedAt: new Date(started).toISOString(),
    launchMs,
    onboardingShownMsSinceScriptStart: onboardingShownAt - started,
    totalOnboardingMs: totalMs,
    totalOnboardingSec: Number((totalMs / 1000).toFixed(2)),
    steps,
    finalState: state,
    uncaught,
    runs,
  };
  saveRaw("s19", "auto-walkthrough.json", raw);
  saveDocsData("s19_auto_walkthrough.json", {
    kind: raw.kind,
    note: raw.note,
    date: new Date().toISOString().slice(0, 10),
    totalOnboardingSec: raw.totalOnboardingSec,
    stepMs: steps.map((s) => ({ index: s.index, title: s.stepTitle, ms: s.stepMs ?? null })),
    stepTitles: steps.map((s) => `${s.counter} ${s.stepTitle}`),
    translateTierCards: steps[2]?.cards ?? [],
    finalState: {
      completed: state.completed,
      skipped: state.skipped,
      skeletonMode: state.skeletonMode,
      restrictedBanner: state.restrictedBanner,
      restrictedBannerText: state.restrictedBannerText,
    },
    uncaught,
    screenshots: steps.map((s) => s.screenshot),
  });

  console.log(
    `[s19] 完成：总耗时 ${raw.totalOnboardingSec}s；各步 ms=${steps.map((s) => s.stepMs ?? "-").join("/")}；未捕获异常=${
      uncaught.errors.length + uncaught.rejections.length
    }`,
  );
  if (!uncaught.errors.length && !uncaught.rejections.length) console.log("[s19] 无未捕获异常");

  if (!ATTACH && !KEEP) {
    applabStop();
    console.log("[s19] 应用已停止");
  }
}

main().catch(async (error) => {
  console.error(`[s19] 失败：${error.message}`);
  saveRaw("s19", "failure.json", { at: new Date().toISOString(), message: String(error.message ?? error) });
  if (!ATTACH && !KEEP) {
    try {
      applabStop();
    } catch {
      /* ignore */
    }
  }
  process.exit(1);
});
