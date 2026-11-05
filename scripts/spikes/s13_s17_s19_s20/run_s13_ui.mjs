/**
 * 临时脚本：S-13 前端现状取证（设置→翻译→本地（离线）→ 模型库静态 minRamMB 文案）——用完即删。
 */
import { applabStart, applabStop, evalWin, saveRaw, shot, sleep, waitFor } from "./lib.mjs";

const CLICK_NAV = (label) => `(() => {
  const button = [...document.querySelectorAll(".main-nav__item")].find((b) => (b.textContent || "").includes(${JSON.stringify(label)}));
  if (!button) return { clicked: false };
  button.click();
  return { clicked: true };
})()`;
const CLICK_SECTION = (label) => `(() => {
  const button = [...document.querySelectorAll(".settings-nav-item")].find((b) => (b.textContent || "").includes(${JSON.stringify(label)}));
  if (!button) return { clicked: false };
  button.click();
  return { clicked: true };
})()`;
const CLICK_LOCAL = `(() => {
  const labels = [...document.querySelectorAll(".sa-segmented button, .sa-choice, .settings-root button, .settings-root label")];
  const hit = labels.find((node) => /本地（离线）|本地\(离线\)/.test(node.textContent || ""));
  if (!hit) return { clicked: false, candidates: labels.map((n) => (n.textContent || "").trim()).filter((t) => t && t.length < 20).slice(0, 20) };
  hit.click();
  return { clicked: true, text: hit.textContent.trim() };
})()`;
const READ_MODELS = `(() => {
  const rows = [...document.querySelectorAll(".sa-model-row")].map((row) => ({
    title: row.querySelector(".sa-model-title")?.innerText?.trim() ?? null,
    meta: row.querySelector(".sa-model-meta")?.innerText?.trim() ?? null,
  }));
  return {
    rows,
    bodyText: (document.querySelector(".settings-root")?.innerText ?? "").slice(0, 2500),
    deviceMemory: typeof navigator.deviceMemory === "number" ? navigator.deviceMemory : null,
  };
})()`;

async function main() {
  const start = applabStart({ reset: false, waitSec: 180 });
  if (start.code !== 0) throw new Error(`start 失败 ${start.stderr}`);
  await waitFor("main", `typeof window.__TAURI_INTERNALS__ !== "undefined"`, "IPC", 30000);
  evalWin("main", `window.__TAURI_INTERNALS__.invoke("window_show", { window: "main" }).then(() => true)`);
  await sleep(600);
  evalWin("main", CLICK_NAV("设置"));
  await waitFor("main", `!!document.querySelector(".settings-nav-item")`, "设置页", 20000);
  evalWin("main", CLICK_SECTION("翻译"));
  await waitFor("main", `!!document.querySelector(".settings-root")`, "翻译分组", 20000);
  await sleep(600);
  const local = evalWin("main", CLICK_LOCAL);
  console.log("clickLocal:", JSON.stringify(local));
  await sleep(1200);
  const models = evalWin("main", READ_MODELS);
  saveRaw("s13", "settings-model-library.json", models);
  shot("main", "s13/02-settings-model-library.png");
  console.log(`[s13-ui] model rows=${models.rows.length} deviceMemory=${models.deviceMemory}`);
  for (const row of models.rows) console.log(`  - ${row.title} | ${row.meta}`);
  applabStop();
}

main().catch(async (error) => {
  console.error("s13-ui failed:", error.message);
  applabStop();
  process.exit(1);
});
