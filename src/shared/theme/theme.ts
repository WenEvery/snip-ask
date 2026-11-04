/**
 * 主题应用（契约 §14）：`<html data-theme="light|dark">` + `data-accent`。
 *
 * - `applyTheme(theme)`：`system` 时监听系统深色偏好，返回清理函数；
 * - `applyAccent(accent)`：写入 `data-accent`，由 tokens.css 覆盖强调色；
 * - `initTheme()`：先用 localStorage 缓存即时应用（避免闪白），再用 `config_get` 同步，
 *   并订阅 `config:changed` 保持多窗口一致。
 */

import { COMMANDS, EVENTS } from "../contract/names";
import type { ConfigChangedPayload, ConfigView, Theme } from "../contract/types";
import { ipc } from "../ipc";

export type ResolvedTheme = "light" | "dark";

const THEME_STORAGE_KEY = "snipask.theme";
const DARK_QUERY = "(prefers-color-scheme: dark)";

function prefersDark(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia(DARK_QUERY).matches
  );
}

/** 把契约主题解析为实际生效的 light / dark */
export function resolveTheme(theme: Theme): ResolvedTheme {
  if (theme === "system") return prefersDark() ? "dark" : "light";
  return theme;
}

/** 写入强调色（blue/green/purple/orange，或自定义色值标识） */
export function applyAccent(accent: string): void {
  if (typeof document === "undefined") return;
  const normalized = accent.trim() || "blue";
  document.documentElement.dataset.accent = normalized;
}

/**
 * 应用主题；返回清理函数（取消 system 主题的媒体查询监听）。
 * 调用方应在切换主题前执行上一次返回的清理函数。
 */
export function applyTheme(theme: Theme): () => void {
  if (typeof document === "undefined") return () => undefined;
  const root = document.documentElement;
  const applyResolved = (): void => {
    root.dataset.theme = resolveTheme(theme);
  };
  applyResolved();

  if (theme !== "system" || typeof window.matchMedia !== "function") {
    return () => undefined;
  }
  const media = window.matchMedia(DARK_QUERY);
  const onChange = (): void => {
    applyResolved();
  };
  media.addEventListener("change", onChange);
  return () => {
    media.removeEventListener("change", onChange);
  };
}

/** 同时应用主题与强调色（含字号档位，供页面 CSS 使用） */
export function applyUiPreferences(theme: Theme, accent: string, fontSize?: string): () => void {
  const cleanup = applyTheme(theme);
  applyAccent(accent);
  if (typeof document !== "undefined" && fontSize) {
    document.documentElement.dataset.fontSize = fontSize;
  }
  return cleanup;
}

function readCachedTheme(): Theme | null {
  try {
    const raw = window.localStorage.getItem(THEME_STORAGE_KEY);
    return raw === "light" || raw === "dark" || raw === "system" ? raw : null;
  } catch {
    return null;
  }
}

function cacheTheme(theme: Theme): void {
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    /* 隐私模式下忽略 */
  }
}

let initialized = false;
let currentCleanup: (() => void) | null = null;

/**
 * 初始化主题：缓存即时生效 → `config_get` 同步 → 订阅 `config:changed`。
 * 返回清理函数（取消订阅与媒体查询监听）。
 */
export async function initTheme(): Promise<() => void> {
  if (typeof document === "undefined") return () => undefined;
  if (initialized) return () => undefined;
  initialized = true;

  currentCleanup = applyTheme(readCachedTheme() ?? "system");

  const applyConfig = (config: ConfigView): void => {
    currentCleanup?.();
    currentCleanup = applyUiPreferences(config.ui.theme, config.ui.accent, config.ui.fontSize);
    cacheTheme(config.ui.theme);
  };

  let disposed = false;
  const offConfigChanged = ipc.on<ConfigChangedPayload>(EVENTS.configChanged, (payload) => {
    if (payload && payload.config) applyConfig(payload.config);
  });

  try {
    const config = await ipc.invoke<ConfigView>(COMMANDS.configGet);
    if (!disposed && config) applyConfig(config);
  } catch (error) {
    console.warn("[theme] 读取配置失败，沿用缓存主题", error);
  }

  return () => {
    disposed = true;
    offConfigChanged();
    currentCleanup?.();
    currentCleanup = null;
  };
}
