/**
 * IPC 事件订阅 hook（StrictMode 安全：用 ref 保存最新 handler，避免重订阅）。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { CommandName, EventName } from "../contract/names";
import type { AppError } from "../contract/types";
import { ipc } from "../ipc";
import { toAppError } from "../ipc/tauri";

/**
 * 订阅后端事件；组件卸载或 `event` 变化时自动解绑。
 * `handler` 可以每次渲染变化，不会导致重新订阅。
 */
export function useIpcEvent<T>(event: EventName, handler: (payload: T) => void): void {
  const handlerRef = useRef(handler);
  useEffect(() => {
    handlerRef.current = handler;
  }, [handler]);

  useEffect(() => ipc.on<T>(event, (payload) => handlerRef.current(payload)), [event]);
}

export interface UseIpcInvokeResult {
  loading: boolean;
  error: AppError | null;
  invoke: <T>(command: CommandName, args?: Record<string, unknown>) => Promise<T>;
  reset: () => void;
}

/** 统一的命令调用状态（loading / AppError），失败时仍会 reject，调用方可 try/catch */
export function useIpcInvoke(): UseIpcInvokeResult {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const invoke = useCallback(async function invokeCommand<T>(
    command: CommandName,
    args?: Record<string, unknown>,
  ): Promise<T> {
    setLoading(true);
    setError(null);
    try {
      const result = await ipc.invoke<T>(command, args);
      if (mountedRef.current) setLoading(false);
      return result;
    } catch (raw) {
      const appError = toAppError(raw);
      if (mountedRef.current) {
        setLoading(false);
        setError(appError);
      }
      throw appError;
    }
  }, []);

  const reset = useCallback(() => {
    setError(null);
    setLoading(false);
  }, []);

  return { loading, error, invoke, reset };
}
