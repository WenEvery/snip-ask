/** 引导第 3 步的本地模型下载列表（models_list + 下载事件 + 取消） */

import { useCallback, useEffect, useState } from "react";
import type {
  DownloadProgressPayload,
  DownloadState,
  DownloadStatePayload,
  ModelInfo,
} from "@/shared/contract/types";
import { COMMANDS, EVENTS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { useIpcEvent } from "@/shared/hooks/useIpcEvent";
import { t } from "../../../../i18n";
import { formatBytes, formatSpeed } from "../format";

const STATE_LABELS: Record<DownloadState, string> = {
  idle: "未下载",
  queued: "等待下载",
  downloading: "下载中",
  verifying: "正在检查文件",
  installed: "已安装",
  failed: "下载失败",
  cancelled: "已取消",
};

export interface ModelDownloadListProps {
  selectedModelId: string | null;
  /** 仅展示该模型（item3 三档「聚焦」对应模型下载）；null 表示展示全部。 */
  focusModelId?: string | null;
  onSelectModel: (id: string) => void;
  onError: (error: unknown) => void;
}

export function ModelDownloadList(props: ModelDownloadListProps) {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [states, setStates] = useState<Record<string, DownloadState>>({});
  const [progress, setProgress] = useState<Record<string, DownloadProgressPayload>>({});

  const refresh = useCallback(async () => {
    try {
      const list = await ipc.invoke<ModelInfo[]>(COMMANDS.modelsList);
      setModels(list.filter((model) => model.kind !== "onnx"));
    } catch (err) {
      props.onError(err);
    } finally {
      setLoading(false);
    }
  }, [props]);

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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

  const download = async (model: ModelInfo) => {
    setBusyId(model.id);
    try {
      await ipc.invoke<string>(COMMANDS.modelsDownload, { id: model.id });
      setStates((current) => ({ ...current, [model.id]: "queued" }));
    } catch (err) {
      props.onError(err);
    } finally {
      setBusyId(null);
    }
  };

  const cancel = async (model: ModelInfo) => {
    setBusyId(model.id);
    try {
      await ipc.invoke<void>(COMMANDS.modelsCancel, { id: model.id });
    } catch (err) {
      props.onError(err);
    } finally {
      setBusyId(null);
    }
  };

  if (loading) return <p className="ob-hint">{t("正在读取本地模型…")}</p>;
  const focusModelId = props.focusModelId ?? null;
  const visibleModels = focusModelId ? models.filter((model) => model.id === focusModelId) : models;
  if (models.length === 0) return <p className="ob-hint">{t("这里暂时没有可下载的模型。")}</p>;
  if (visibleModels.length === 0) {
    return (
      <p className="ob-hint">
        {t("没有找到 {model}；也可以先继续，稍后在「设置 → 翻译」中下载。", {
          model: focusModelId ?? "",
        })}
      </p>
    );
  }

  return (
    <div className="ob-models">
      {visibleModels.map((model) => {
        const state = states[model.id] ?? model.downloadState;
        const installed = state === "installed" || model.installed;
        const active = state === "queued" || state === "downloading" || state === "verifying";
        const item = progress[model.id];
        const percent =
          item && item.total > 0 ? Math.min(100, Math.round((item.loaded / item.total) * 100)) : 0;
        const selected = props.selectedModelId === model.id;
        return (
          <div key={model.id} className={`ob-model${selected ? " is-selected" : ""}`}>
            <div className="ob-model-main">
              <div className="ob-model-title">
                <strong>{model.name}</strong>
                <span className={`ob-tag${installed ? " is-ok" : ""}`}>
                  {t(STATE_LABELS[state])}
                </span>
                {selected ? <span className="ob-tag is-ok">{t("已选用")}</span> : null}
              </div>
              <p className="ob-hint">
                {formatBytes(model.sizeBytes)} · {t("运行内存约")}{" "}
                {formatBytes(model.minRamMB * 1024 * 1024)}
              </p>
              {active ? (
                <>
                  <div className="ob-progressbar">
                    <div className="ob-progressbar-fill" style={{ width: `${percent}%` }} />
                  </div>
                  <p className="ob-hint">
                    {item
                      ? `${formatBytes(item.loaded)} / ${formatBytes(item.total)} · ${formatSpeed(item.speedBps)} · ${percent}%`
                      : t("正在准备下载…")}
                  </p>
                </>
              ) : null}
            </div>
            <div className="ob-model-actions">
              {installed ? (
                <button
                  type="button"
                  className="ob-btn ob-btn-secondary"
                  disabled={selected}
                  onClick={() => props.onSelectModel(model.id)}
                >
                  {selected ? t("已选用") : t("选用此模型")}
                </button>
              ) : active ? (
                <button
                  type="button"
                  className="ob-btn ob-btn-ghost"
                  disabled={busyId === model.id}
                  onClick={() => void cancel(model)}
                >
                  {t("取消下载")}
                </button>
              ) : (
                <button
                  type="button"
                  className="ob-btn ob-btn-secondary"
                  disabled={busyId === model.id}
                  onClick={() => void download(model)}
                >
                  {state === "failed" ? t("重试下载") : t("下载")}
                </button>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
