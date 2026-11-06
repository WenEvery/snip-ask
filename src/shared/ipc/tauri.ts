/**
 * 真实后端适配（Tauri 2）：`invoke` + `listen`。
 *
 * 错误规范化：Tauri 的 reject 值可能是字符串、Error 或后端 `AppError`；
 * 这里保证向业务层抛出的**一定是** `AppError`（契约 §8.1）。
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CommandName, EventName } from "../contract/names";
import type { AppError } from "../contract/types";
import type { IpcBackend } from "./index";

function stringifyUnknown(raw: unknown): string | null {
  if (typeof raw === "string") return raw;
  if (raw instanceof Error) return raw.message;
  try {
    return JSON.stringify(raw);
  } catch {
    return null;
  }
}

/** 把任意 reject 值规范化为契约 `AppError` */
export function toAppError(raw: unknown, fallbackMessage = "操作失败，请稍后重试"): AppError {
  if (typeof raw === "object" && raw !== null) {
    const candidate = raw as Partial<AppError>;
    if (typeof candidate.code === "string" && candidate.code.length > 0) {
      return {
        code: candidate.code,
        message: typeof candidate.message === "string" ? candidate.message : fallbackMessage,
        detail: typeof candidate.detail === "string" ? candidate.detail : null,
        retryable: candidate.retryable === true,
        fallback: Array.isArray(candidate.fallback)
          ? candidate.fallback.filter((item): item is string => typeof item === "string")
          : [],
        context:
          typeof candidate.context === "object" &&
          candidate.context !== null &&
          !Array.isArray(candidate.context)
            ? (candidate.context as Record<string, unknown>)
            : null,
      };
    }
  }
  return {
    code: "SA-9001",
    message: fallbackMessage,
    detail: stringifyUnknown(raw),
    retryable: false,
    fallback: [],
    context: null,
  };
}

export const tauriBackend: IpcBackend = {
  async invoke<T>(command: CommandName, args?: Record<string, unknown>): Promise<T> {
    try {
      return await tauriInvoke<T>(command, args);
    } catch (error) {
      throw toAppError(error);
    }
  },

  on<T>(event: EventName, handler: (payload: T) => void): () => void {
    let unlisten: UnlistenFn | null = null;
    let disposed = false;
    void listen<T>(event, (tauriEvent) => {
      handler(tauriEvent.payload);
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch((error: unknown) => {
        console.error(`[ipc] 订阅事件失败：${event}`, toAppError(error, "事件订阅失败"));
      });
    return () => {
      disposed = true;
      if (unlisten) {
        unlisten();
        unlisten = null;
      }
    };
  },
};
