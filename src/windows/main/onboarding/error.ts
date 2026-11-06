/** 引导页错误归一化（与设置页实现同构，保持窗口内自洽，避免跨窗口依赖） */

import type { UiError } from "./types";
import { t } from "../../../i18n";

const GENERIC: UiError = {
  code: "SA-0000",
  message: t("操作失败，请稍后重试。"),
  detail: null,
  retryable: true,
  fallback: [],
};

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
  if (typeof error === "string" && error.trim()) return { ...GENERIC, detail: error };
  return GENERIC;
}

export function errorMessage(error: unknown): string {
  return toUiError(error).message;
}
