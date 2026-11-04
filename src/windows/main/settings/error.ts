/**
 * 错误归一化：IPC 失败时 reject 的是契约 `AppError`（对象），
 * 这里统一收敛为页面内可展示的 `UiError`，避免各分组重复判断。
 */

import type { UiError } from "./types";

const GENERIC: UiError = {
  code: "SA-0000",
  message: "操作失败，请稍后重试。",
  detail: null,
  retryable: true,
  fallback: [],
};

/** 把任意 catch 到的值收敛为 `UiError` */
export function toUiError(error: unknown): UiError {
  if (error && typeof error === "object") {
    const candidate = error as Partial<Record<string, unknown>>;
    const code = typeof candidate.code === "string" ? candidate.code : null;
    const message = typeof candidate.message === "string" ? candidate.message : null;
    if (code && message) {
      return {
        code,
        message,
        detail: typeof candidate.detail === "string" ? candidate.detail : null,
        retryable: candidate.retryable === true,
        fallback: Array.isArray(candidate.fallback)
          ? candidate.fallback.filter((item): item is string => typeof item === "string")
          : [],
      };
    }
    if (error instanceof Error) {
      return { ...GENERIC, message: error.message || GENERIC.message, detail: error.stack ?? null };
    }
  }
  if (typeof error === "string" && error.trim()) {
    return { ...GENERIC, detail: error };
  }
  return GENERIC;
}

/** 只取面向用户的一句话 */
export function errorMessage(error: unknown): string {
  return toUiError(error).message;
}
