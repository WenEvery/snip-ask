/**
 * 本地翻译模型库：`models_list` 展示清单 + `models_download` 下载，
 * 订阅 `download:progress` / `download:state` 更新进度条、速度与状态，支持取消 / 删除。
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
import { EmptyHint, Hint, LoadingBlock, StatusTag } from "../parts";

export interface ModelLibraryProps {
  engine: LocalEngine;
  onError: (error: unknown) => void;
}

const STATE_LABELS: Record<DownloadState, string> = {
  queued: "排队中",
  downloading: "下载中",
  verifying: "校验中",
  installed: "已安装",
  failed: "下载失败",
  cancelled: "已取消",
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
    if (payload.state === "installed" || payload.state === "failed" || payload.state === "cancelled") {
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
      setNotice(`已删除本地模型「${model.name}」`);
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
        Ollama 复用本机已安装模型，无需在应用内下载。请先在终端执行 `ollama pull &lt;模型名&gt;`，再在上方填写模型名。
      </EmptyHint>
    );
  }

  if (loading && models.length === 0) {
    return <LoadingBlock text="正在读取模型清单…" />;
  }

  const kind = engineKind(engine);
  const visible = models.filter((model) => (kind === null ? true : model.kind === kind));

  return (
    <div className="sa-models">
      <Hint>下载前请预留「模型体积 + 20%」的磁盘空间；下载支持断点续传与 SHA256 校验。</Hint>
      {visible.length === 0 ? (
        <EmptyHint>模型清单为空，或当前引擎没有可下载模型。请检查 resources/models.manifest.json。</EmptyHint>
      ) : null}
      {visible.map((model) => {
        const state = states[model.id] ?? model.downloadState;
        const installed = state === "installed" || model.installed;
        const active = state === "queued" || state === "downloading" || state === "verifying";
        const item = progress[model.id];
        const percent = item && item.total > 0 ? Math.min(100, Math.round((item.loaded / item.total) * 100)) : 0;
        return (
          <div key={model.id} className="sa-model-row">
            <div className="sa-model-main">
              <div className="sa-model-title">
                <strong>{model.name}</strong>
                <code className="sa-mono">{model.id}</code>
                <StatusTag tone={installed ? "ok" : active ? "warn" : state === "failed" ? "danger" : "neutral"}>
                  {STATE_LABELS[state]}
                </StatusTag>
              </div>
              <p className="sa-model-meta">
                {formatBytes(model.sizeBytes)} · 需内存 {model.minRamMB} MB · 许可 {model.license} · {model.kind}
              </p>
              {active || (installed && model.installedBytes > 0) ? (
                <div className="sa-progress">
                  <div className="sa-progressbar">
                    <div className="sa-progressbar-fill" style={{ width: `${installed ? 100 : percent}%` }} />
                  </div>
                  <span className="sa-progress-text">
                    {installed
                      ? `已安装 ${formatBytes(model.installedBytes || model.sizeBytes)}`
                      : item
                        ? `${formatBytes(item.loaded)} / ${formatBytes(item.total)} · ${formatSpeed(item.speedBps)} · ${percent}%`
                        : "等待下载进度…"}
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
                  删除
                </Button>
              ) : active ? (
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busyId === model.id}
                  onClick={() => void cancelDownload(model)}
                >
                  取消
                </Button>
              ) : (
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={busyId === model.id}
                  onClick={() => void startDownload(model)}
                >
                  {busyId === model.id ? "启动中…" : state === "failed" ? "重试下载" : "下载"}
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
