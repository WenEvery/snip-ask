/**
 * IPC 出口：根据运行环境选择 Tauri 适配或浏览器 mock 后端。
 *
 * 业务代码只依赖 `ipc` 单例与 `IpcBackend` 接口，不直接 import `@tauri-apps/api`。
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import type { CommandName, EventName } from "../contract/names";
import { mockBackend } from "./mock";
import { tauriBackend } from "./tauri";

export { toAppError } from "./tauri";

/** 契约 §13：前端唯一 IPC 抽象 */
export interface IpcBackend {
  invoke<T>(command: CommandName, args?: Record<string, unknown>): Promise<T>;
  on<T>(event: EventName, handler: (payload: T) => void): () => void;
}

/** 是否运行在 Tauri WebView 中（Tauri 2 注入 `__TAURI_INTERNALS__`） */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** 单例：Tauri 环境用真实适配，浏览器预览用确定性 mock */
export const ipc: IpcBackend = isTauri() ? tauriBackend : mockBackend;

/**
 * 当前窗口标签：
 * 1. `?window=main|overlay|result`（浏览器预览与静态配置都用它）；
 * 2. Tauri 窗口 label 兜底；
 * 3. 缺省 "main"。
 *
 * 注意：返回 `string` 而不是联合类型 —— 非法 `?window=xxx` 需要被 App 识别并提示。
 */
export function getWindowLabel(): string {
  if (typeof window === "undefined") return "main";
  const query = new URLSearchParams(window.location.search).get("window");
  if (query) return query;
  if (isTauri()) {
    try {
      return getCurrentWindow().label;
    } catch {
      /* 非窗口上下文（如测试）回落到 main */
    }
  }
  return "main";
}

/** 浏览器直连预览（无 Tauri）时为 true，用于页面提示骨架模式 */
export function isBrowserPreview(): boolean {
  return !isTauri();
}
