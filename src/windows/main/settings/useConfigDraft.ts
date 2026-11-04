/**
 * 设置页配置草稿状态机。
 *
 * 职责：
 * - 挂载时 `config_get` 拉取 `ConfigView`；
 * - 本地维护 `draft`（可编辑）与 `baseline`（上次后端状态）；
 * - 分组「应用」走 `config_patch`，成功后用返回值同步 draft + baseline；
 * - 订阅 `config:changed`，外部变更（托盘 / 热键 / 自启）即时同步。
 */

import { useCallback, useEffect, useState } from "react";
import type { ConfigChangedPayload, ConfigView } from "@/shared/contract/types";
import { COMMANDS, EVENTS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { useIpcEvent } from "@/shared/hooks/useIpcEvent";
import { toUiError } from "./error";
import type { ConfigPatch, UiError } from "./types";

export type SaveStatus = "idle" | "ok" | "error";

export interface ConfigDraftApi {
  draft: ConfigView | null;
  baseline: ConfigView | null;
  loading: boolean;
  loadError: UiError | null;
  saving: boolean;
  status: SaveStatus;
  /** 顶部提示文案（如「外观」已应用） */
  statusText: string | null;
  error: UiError | null;
  reload: () => Promise<void>;
  /** 就地修改草稿（不可变更新，返回新对象） */
  edit: (updater: (draft: ConfigView) => ConfigView) => void;
  /** 提交补丁；成功返回 true */
  apply: (patch: ConfigPatch, label: string) => Promise<boolean>;
  /** 用命令返回的完整配置覆盖草稿 */
  replace: (view: ConfigView) => void;
  reportError: (error: unknown) => void;
  clearError: () => void;
}

export function useConfigDraft(): ConfigDraftApi {
  const [draft, setDraft] = useState<ConfigView | null>(null);
  const [baseline, setBaseline] = useState<ConfigView | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<UiError | null>(null);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<SaveStatus>("idle");
  const [statusText, setStatusText] = useState<string | null>(null);
  const [error, setError] = useState<UiError | null>(null);

  const replace = useCallback((view: ConfigView) => {
    setDraft(view);
    setBaseline(view);
    setLoadError(null);
  }, []);

  const reload = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const view = await ipc.invoke<ConfigView>(COMMANDS.configGet);
      replace(view);
    } catch (err) {
      setLoadError(toUiError(err));
    } finally {
      setLoading(false);
    }
  }, [replace]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // 后端配置热生效广播（外部修改或本页保存成功后都会到达）
  useIpcEvent<ConfigChangedPayload>(EVENTS.configChanged, (payload) => {
    setDraft(payload.config);
    setBaseline(payload.config);
  });

  const edit = useCallback((updater: (current: ConfigView) => ConfigView) => {
    setDraft((current) => (current ? updater(current) : current));
  }, []);

  const apply = useCallback(
    async (patch: ConfigPatch, label: string) => {
      setSaving(true);
      try {
        const view = await ipc.invoke<ConfigView>(COMMANDS.configPatch, { patch });
        replace(view);
        setError(null);
        setStatus("ok");
        setStatusText(`「${label}」已应用`);
        return true;
      } catch (err) {
        setError(toUiError(err));
        setStatus("error");
        setStatusText(`「${label}」保存失败`);
        return false;
      } finally {
        setSaving(false);
      }
    },
    [replace],
  );

  const reportError = useCallback((err: unknown) => {
    setError(toUiError(err));
    setStatus("error");
  }, []);

  const clearError = useCallback(() => setError(null), []);

  return {
    draft,
    baseline,
    loading,
    loadError,
    saving,
    status,
    statusText,
    error,
    reload,
    edit,
    apply,
    replace,
    reportError,
    clearError,
  };
}
