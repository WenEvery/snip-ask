/**
 * 前端国际化（v1.1.0）。
 *
 * 设计：以**中文原文**为 key，英文词典按 key 查表；缺失时回退中文（方便逐步覆盖，
 * 也保证不出现空白 UI）。`t()` 支持 `{name}` 形式的变量插值。
 *
 * 语言来源：`config.app.language`（`zh-CN` / `en-US`），由 `initTheme()` 在启动时同步；
 * 用户在设置中切换语言后，`config:changed` 会触发所有已加载窗口 `location.reload()`，
 * 缓存写入 localStorage 保证 reload 后立即以新语言渲染。
 */

import { EN_MAIN } from "./en.main";
import { EN_OVERLAY } from "./en.overlay";
import { EN_SHARED } from "./en.shared";

export type Language = "zh-CN" | "en-US";

const CACHE_KEY = "snipask.language";
const EN: Record<string, string> = { ...EN_SHARED, ...EN_MAIN, ...EN_OVERLAY };

function readCachedLanguage(): Language {
  try {
    const raw = window.localStorage.getItem(CACHE_KEY);
    return raw === "en-US" ? "en-US" : "zh-CN";
  } catch {
    return "zh-CN";
  }
}

function cacheLanguage(language: Language): void {
  try {
    window.localStorage.setItem(CACHE_KEY, language);
  } catch {
    /* 隐私模式忽略 */
  }
}

let current: Language = readCachedLanguage();

/** 在 React 首次渲染前调用，避免英文界面闪中文。 */
export function initLanguage(): Language {
  if (typeof document !== "undefined") {
    document.documentElement.lang = current === "en-US" ? "en" : "zh-CN";
  }
  return current;
}

export function getLanguage(): Language {
  return current;
}

export function setLanguage(language: Language): void {
  current = language;
  cacheLanguage(language);
  if (typeof document !== "undefined") {
    document.documentElement.lang = language === "en-US" ? "en" : "zh-CN";
  }
}

/** 从 `config.app.language` 同步；若语言变化则整窗 reload（调用方负责避免循环）。 */
export function applyLanguageFromConfig(language: string | null | undefined): void {
  const next: Language = language === "en-US" ? "en-US" : "zh-CN";
  if (next === current) return;
  setLanguage(next);
  if (typeof window !== "undefined") window.location.reload();
}

/**
 * 翻译中文原文；英文词典缺失时回退中文。
 *
 * @example t("开始框选") / t("已复制 {name}", { name: "42" })
 */
export function t(zh: string, vars?: Record<string, string | number>): string {
  const template = current === "en-US" ? (EN[zh] ?? zh) : zh;
  return interpolate(template, vars);
}

function interpolate(template: string, vars?: Record<string, string | number>): string {
  if (!vars) return template;
  return template.replace(/\{([a-zA-Z0-9_]+)\}/g, (whole, name: string) => {
    const value = vars[name];
    return value === undefined ? whole : String(value);
  });
}

/** 供开发期检查英文覆盖率（node --test 可 import 本函数统计）。 */
export function englishDictionary(): Readonly<Record<string, string>> {
  return EN;
}
