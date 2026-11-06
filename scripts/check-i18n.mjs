/**
 * 英文词典覆盖率检查（CI / 本地门禁）。
 *
 * 扫描 `src` 下所有 ts/tsx（排除测试）中静态 `t("中文...")` key，与
 * en.shared/en.main/en.overlay 的 key 并集比较；缺失即失败，避免英文界面回退中文。
 *
 * 运行：node scripts/check-i18n.mjs
 */
import { readFileSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(fileURLToPath(import.meta.url), "..", "..");
const SRC = join(ROOT, "src");

function walk(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walk(path));
    else if (/\.(ts|tsx)$/.test(entry.name) && !/\.test\.(ts|tsx)$/.test(entry.name)) out.push(path);
  }
  return out;
}

function tKeys(file, text) {
  const keys = new Set();
  const pattern = /\bt\(\s*"((?:[^"\\]|\\.)*)"/g;
  for (const match of text.matchAll(pattern)) {
    // 只收集含中文的 key（纯英文/标识符不是 UI 文案）
    if (/[\u3400-\u9fff]/.test(match[1])) keys.add(match[1]);
  }
  // 支持 t(`...`) 模板字符串中不含 ${} 的静态文案
  const templatePattern = /\bt\(\s*`((?:[^`\\$]|\\.)*)`/g;
  for (const match of text.matchAll(templatePattern)) {
    if (/[\u3400-\u9fff]/.test(match[1]) && !match[1].includes("${")) keys.add(match[1]);
  }
  return { file, keys };
}

function dictionaryKeys(file) {
  const text = readFileSync(file, "utf8");
  const keys = new Set();
  // 带引号 key（prettier-ignore 或包含特殊字符）
  for (const match of text.matchAll(/^\s*"((?:[^"\\]|\\.)*)"\s*:/gm)) keys.add(match[1]);
  // 中文是合法 JS 标识符：Prettier 默认会去掉引号，需同时识别裸 key。
  for (const match of text.matchAll(/^\s*([^\s:{}"'/][^:{}"']*?)\s*:\s*["'`]/gm)) {
    const key = match[1].trim();
    if (/[\u3400-\u9fff]/.test(key)) keys.add(key);
  }
  return keys;
}

const dictionaries = ["en.shared.ts", "en.main.ts", "en.overlay.ts"].map((name) =>
  dictionaryKeys(join(SRC, "i18n", name)),
);
const known = new Set(dictionaries.flatMap((set) => [...set]));

const used = new Map();
for (const file of walk(SRC)) {
  if (file.includes(`${join("src", "i18n")}`)) continue;
  for (const key of tKeys(file, readFileSync(file, "utf8")).keys) {
    if (!used.has(key)) used.set(key, relative(ROOT, file));
  }
}

const missing = [...used.entries()].filter(([key]) => !known.has(key));
const unused = [...known].filter((key) => !used.has(key));

console.log(`[i18n] t() keys=${used.size} dictionary keys=${known.size}`);
if (unused.length > 0) console.log(`[i18n] unused dictionary keys=${unused.length}`);
for (const [key, file] of missing.slice(0, 80)) console.log(`MISSING ${JSON.stringify(key)}  (${file})`);
if (missing.length > 80) console.log(`... 还有 ${missing.length - 80} 条缺失`);
if (missing.length > 0) {
  console.error(`[i18n] 缺少 ${missing.length} 条英文翻译`);
  process.exit(1);
}
console.log("[i18n] PASS：所有静态 t() 文案都有英文翻译");
