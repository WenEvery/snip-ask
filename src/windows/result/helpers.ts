/** 结果窗工具：错误归一化、降级动作中文名、时延/费用格式化 */

import type { TaskErrorPayload } from "@/shared/contract/types";
import { t } from "@/i18n";

export interface ResultError {
  code: string;
  message: string;
  detail: string | null;
  retryable: boolean;
  fallback: string[];
}

const FALLBACK_LABELS: Record<string, string> = {
  retry: t("重试"),
  browser_search: t("改用浏览器搜索"),
  local_translate: t("改用本地翻译"),
  cloud_translate: t("改用云端翻译"),
  open_settings: t("打开设置"),
  image_path: t("改用图片识别"),
  download_model: t("下载本地模型"),
  light_model: t("换用轻量模型"),
  change_hotkey: t("更换热键"),
  pause_hotkey: t("暂停热键"),
  mirror: t("切换镜像源"),
  manual_download: t("手动下载"),
  redownload: t("重新下载"),
  reset_group: t("恢复该分组默认值"),
};

export function fallbackLabel(value: string): string {
  return FALLBACK_LABELS[value] ?? value;
}

export function fallbackText(fallbacks: string[]): string {
  return fallbacks.map(fallbackLabel).join(t("、"));
}

/** 命令 reject 的 AppError / 任意异常 → 可展示错误 */
export function normalizeError(error: unknown): ResultError {
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
      return {
        code: "SA-0000",
        message: error.message || t("操作失败，请稍后重试。"),
        detail: null,
        retryable: true,
        fallback: [],
      };
    }
  }
  return {
    code: "SA-0000",
    message: t("操作失败，请稍后重试。"),
    detail: typeof error === "string" ? error : null,
    retryable: true,
    fallback: [],
  };
}

/** `task:error` 事件载荷（契约无 detail 字段）→ 错误卡片数据 */
export function fromTaskError(payload: TaskErrorPayload): ResultError {
  return {
    code: payload.code,
    message: payload.message,
    detail: null,
    retryable: payload.retryable,
    fallback: payload.fallback,
  };
}

export function formatLatency(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

export function formatCost(cost: number): string {
  if (!Number.isFinite(cost) || cost <= 0) return "¥0.0000";
  if (cost < 0.01) return `¥${cost.toFixed(4)}`;
  return `¥${cost.toFixed(2)}`;
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}
