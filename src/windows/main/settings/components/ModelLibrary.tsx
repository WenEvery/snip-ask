/**
 * 本地翻译模型库：展示可下载模型，下载时显示进度，支持取消 / 删除。
 *
 * 状态约定（契约 v1.0.1）：
 * - `idle`：还没下载（默认），显示「下载」按钮，不显示进度/取消；
 * - `queued/downloading/verifying`：下载中，显示进度与「取消」；
 * - `failed`：下载失败，显示「重试」；
 * - `installed`：已安装，显示「删除」。
 */

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui";
import type {
  DownloadProgressPayload,
  DownloadState,
  DownloadStatePayload,
  LocalEngine,
  ModelInfo,
} from "@/shared/contract/types";
import { COMMANDS, EVENTS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { useIpcEvent } from "@/shared/hooks/useIpcEvent";
import { formatBytes, formatSpeed } from "../format";
import { t } from "@/i18n";
import { EmptyHint, Hint, LoadingBlock, StatusTag } from "../parts";

export interface ModelLibraryProps {
  engine: LocalEngine;
  onError: (error: unknown) => void;
}

const STATE_LABELS: Record<DownloadState, string> = {
  idle: t("未下载"),
  queued: t("等待下载"),
  downloading: t("下载中"),
  verifying: t("正在检查文件"),
  installed: t("已安装"),
  failed: t("下载失败"),
  cancelled: t("已取消"),
};

function engineKind(engine: LocalEngine): ModelInfo["kind"] | null {
  if (engine === "llama.cpp") return "gguf";
  if (engine === "ct2") return "ct2";
  return null;
}

export function ModelLibrary(props: ModelLibraryProps) {
  const { engine, onError } = props;
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [states, setStates] = useState<Record<string, DownloadState>>({});
  const [progress, setProgress] = useState<Record<string, DownloadProgressPayload>>({});
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const list = await ipc.invoke<ModelInfo[]>(COMMANDS.modelsList);
      setModels(list);
    } catch (err) {
      onError(err);
    } finally {
      setLoading(false);
    }
  }, [onError]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useIpcEvent<DownloadProgressPayload>(EVENTS.downloadProgress, (payload) => {
    setProgress((current) => ({ ...current, [payload.model]: payload }));
  });

  useIpcEvent<DownloadStatePayload>(EVENTS.downloadState, (payload) => {
    setStates((current) => ({ ...current, [payload.model]: payload.state }));
    if (
      payload.state === "installed" ||
      payload.state === "failed" ||
      payload.state === "cancelled"
    ) {
      void refresh();
    }
  });

  const startDownload = async (model: ModelInfo) => {
    setBusyId(model.id);
    try {
      await ipc.invoke<string>(COMMANDS.modelsDownload, { id: model.id });
      setStates((current) => ({ ...current, [model.id]: "queued" }));
    } catch (err) {
      onError(err);
    } finally {
      setBusyId(null);
    }
  };

  const cancelDownload = async (model: ModelInfo) => {
    setBusyId(model.id);
    try {
      await ipc.invoke<void>(COMMANDS.modelsCancel, { id: model.id });
    } catch (err) {
      onError(err);
    } finally {
      setBusyId(null);
    }
  };

  const removeModel = async (model: ModelInfo) => {
    setBusyId(model.id);
    try {
      await ipc.invoke<void>(COMMANDS.modelsRemove, { id: model.id });
      setNotice(t("已删除本地模型「{name}」", { name: model.name }));
      await refresh();
    } catch (err) {
      onError(err);
    } finally {
      setBusyId(null);
    }
  };

  if (engine === "ollama") {
    return (
      <EmptyHint>
        {t(
          "使用 Ollama 时不需要在这里下载模型。先在终端执行 `ollama pull <模型名>`，再在上方填写模型名即可。",
        )}
      </EmptyHint>
    );
  }

  if (loading && models.length === 0) {
    return <LoadingBlock text={t("正在读取模型列表…")} />;
  }

  const kind = engineKind(engine);
  const visible = models.filter((model) => (kind === null ? true : model.kind === kind));

  return (
    <div className="sa-models">
      <Hint>{t("下载前请预留足够的磁盘空间；中途退出后可以继续下载。")}</Hint>
      {visible.length === 0 ? <EmptyHint>{t("这里还没有可下载的模型。")}</EmptyHint> : null}
      {visible.map((model) => {
        const state = states[model.id] ?? model.downloadState;
        const installed = state === "installed" || model.installed;
        const active = state === "queued" || state === "downloading" || state === "verifying";
        const item = progress[model.id];
        const percent =
          item && item.total > 0 ? Math.min(100, Math.round((item.loaded / item.total) * 100)) : 0;
        return (
          <div key={model.id} className="sa-model-row">
            <div className="sa-model-main">
              <div className="sa-model-title">
                <strong>{model.name}</strong>
                <code className="sa-mono">{model.id}</code>
                <StatusTag
                  tone={
                    installed ? "ok" : active ? "warn" : state === "failed" ? "danger" : "neutral"
                  }
                >
                  {STATE_LABELS[state]}
                </StatusTag>
              </div>
              <p className="sa-model-meta">
                {formatBytes(model.sizeBytes)} · {t("运行内存约")}{" "}
                {formatBytes(model.minRamMB * 1024 * 1024)}
              </p>
              {active || (installed && model.installedBytes > 0) ? (
                <div className="sa-progress">
                  <div className="sa-progressbar">
                    <div
                      className="sa-progressbar-fill"
                      style={{ width: `${installed ? 100 : percent}%` }}
                    />
                  </div>
                  <span className="sa-progress-text">
                    {installed
                      ? t("已下载 {size}", {
                          size: formatBytes(model.installedBytes || model.sizeBytes),
                        })
                      : item
                        ? `${formatBytes(item.loaded)} / ${formatBytes(item.total)} · ${formatSpeed(item.speedBps)} · ${percent}%`
                        : t("正在准备下载…")}
                  </span>
                </div>
              ) : null}
            </div>
            <div className="sa-model-actions">
              {installed ? (
                <Button
                  variant="danger"
                  size="sm"
                  disabled={busyId === model.id}
                  onClick={() => void removeModel(model)}
                >
                  {t("删除")}
                </Button>
              ) : active ? (
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busyId === model.id}
                  onClick={() => void cancelDownload(model)}
                >
                  {t("取消")}
                </Button>
              ) : (
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={busyId === model.id}
                  onClick={() => void startDownload(model)}
                >
                  {busyId === model.id
                    ? t("启动中…")
                    : state === "failed"
                      ? t("重试下载")
                      : t("下载")}
                </Button>
              )}
            </div>
          </div>
        );
      })}
      {notice ? <p className="sa-hint">{notice}</p> : null}
    </div>
  );
}
