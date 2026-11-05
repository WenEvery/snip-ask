/**
 * 文字识别状态卡（设置页「翻译」与首启引导共用）。
 *
 * 数据来源（契约）：`ocr_status`（引擎与模型是否可用）、`models_list`（缺失模型的下载信息）、
 * `download:progress` / `download:state`（下载进度）、`open_path`（手动放入模型的文件夹）。
 */

import { useCallback, useEffect, useState } from "react";
import { Badge, Button, Spinner } from "@/components/ui";
import type {
  DownloadProgressPayload,
  DownloadState,
  DownloadStatePayload,
  ModelInfo,
  OcrStatus,
  Stats,
} from "@/shared/contract/types";
import { COMMANDS, EVENTS } from "@/shared/contract/names";
import { ipc, toAppError } from "@/shared/ipc";
import { useIpcEvent } from "@/shared/hooks/useIpcEvent";
import { formatBytes, formatSpeed } from "../settings/format";
import {
  allBundled,
  editionBadge,
  mergeOcrRows,
  ocrAssetLabel,
  pendingOcrRows,
  summarizeOcr,
  type OcrAssetRow,
  type OcrTone,
} from "./status";
import "./ocr.css";

export interface OcrStatusCardProps {
  /** 紧凑模式（引导页）：隐藏运行时路径 / sha，仅保留状态与下载入口。 */
  compact?: boolean;
  /** 追加说明（如精简版首次使用提示）。 */
  note?: string;
}

const TONE_TO_BADGE: Record<OcrTone, "neutral" | "accent" | "success" | "warning" | "danger"> = {
  neutral: "neutral",
  ok: "success",
  warn: "warning",
  danger: "danger",
};

const STATE_LABELS: Record<DownloadState, string> = {
  idle: "未下载",
  queued: "等待下载",
  downloading: "下载中",
  verifying: "正在检查文件",
  installed: "已下载",
  failed: "下载失败",
  cancelled: "已取消",
};

export function OcrStatusCard({ compact = false, note }: OcrStatusCardProps) {
  const [status, setStatus] = useState<OcrStatus | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [stats, setStats] = useState<Stats | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [states, setStates] = useState<Record<string, DownloadState>>({});
  const [progress, setProgress] = useState<Record<string, DownloadProgressPayload>>({});

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [nextStatus, nextModels, nextStats] = await Promise.all([
        ipc.invoke<OcrStatus>(COMMANDS.ocrStatus),
        ipc.invoke<ModelInfo[]>(COMMANDS.modelsList),
        ipc.invoke<Stats>(COMMANDS.appStats),
      ]);
      setStatus(nextStatus);
      setModels(nextModels);
      setStats(nextStats);
      setLoadError(null);
    } catch (err) {
      setLoadError(toAppError(err).message);
    } finally {
      setLoading(false);
    }
  }, []);

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

  const rows = mergeOcrRows(status, models);
  const pending = pendingOcrRows(rows);
  const summary = summarizeOcr(status, stats?.edition);
  const edition = stats?.edition ?? null;

  const runAction = async (id: string, action: () => Promise<unknown>) => {
    setBusyId(id);
    setActionError(null);
    setNotice(null);
    try {
      await action();
    } catch (err) {
      setActionError(toAppError(err).message);
    } finally {
      setBusyId(null);
    }
  };

  const startDownload = (row: OcrAssetRow) =>
    runAction(row.id, async () => {
      await ipc.invoke<string>(COMMANDS.modelsDownload, { id: row.id });
      setStates((current) => ({ ...current, [row.id]: "queued" }));
      setNotice(`已开始下载「${ocrAssetLabel(row.id, row.name)}」。`);
    });

  const cancelDownload = (row: OcrAssetRow) =>
    runAction(row.id, async () => {
      await ipc.invoke<void>(COMMANDS.modelsCancel, { id: row.id });
      setNotice(`已取消下载「${ocrAssetLabel(row.id, row.name)}」。`);
    });

  const removeAsset = (row: OcrAssetRow) =>
    runAction(row.id, async () => {
      await ipc.invoke<void>(COMMANDS.modelsRemove, { id: row.id });
      setNotice(`已删除「${ocrAssetLabel(row.id, row.name)}」，下次识别前需要重新下载。`);
      await refresh();
    });

  const downloadAll = async () => {
    const targets = pending.filter((row) => row.model !== null);
    if (targets.length === 0) return;
    setActionError(null);
    for (const row of targets) {
      try {
        await ipc.invoke<string>(COMMANDS.modelsDownload, { id: row.id });
        setStates((current) => ({ ...current, [row.id]: "queued" }));
      } catch (err) {
        setActionError(toAppError(err).message);
        break;
      }
    }
  };

  const openModelsDir = () =>
    runAction("open-models", async () => {
      await ipc.invoke<void>(COMMANDS.openPath, { kind: "models" });
      setNotice("已打开模型文件夹。");
    });

  return (
    <div className="sa-ocr-card">
      <div className="sa-ocr-head">
        <div className="sa-ocr-head-main">
          <span className="sa-ocr-title">文字识别</span>
          <Badge tone={TONE_TO_BADGE[summary.tone]} title={summary.detail}>
            {summary.label}
          </Badge>
          {edition ? <Badge tone="neutral">{editionBadge(edition)}</Badge> : null}
          {allBundled(rows) ? <Badge tone="success">模型已内置</Badge> : null}
        </div>
        <div className="sa-ocr-head-actions">
          <Button variant="ghost" size="sm" disabled={loading} onClick={() => void refresh()}>
            {loading ? "检查中…" : "重新检查"}
          </Button>
        </div>
      </div>

      {loading && !status ? (
        <div className="sa-ocr-loading">
          <Spinner />
          <span>正在检查文字识别是否可用…</span>
        </div>
      ) : loadError ? (
        <p className="sa-ocr-error" role="alert">
          检查失败：{loadError}
        </p>
      ) : (
        <>
          <p className="sa-ocr-summary">{summary.detail}</p>
          {!compact ? (
            <div className="sa-ocr-runtime">
              <span className="sa-ocr-runtime-label">识别引擎</span>
              <span className="sa-ocr-runtime-value">
                {status?.runtimeReady ? "已就绪" : "未找到，暂时无法识别文字"}
              </span>
              {status?.runtimePath ? (
                <code className="sa-ocr-path" title={status.runtimePath}>
                  {status.runtimePath}
                </code>
              ) : null}
            </div>
          ) : null}

          <div className="sa-ocr-rows">
            {rows.map((row) => {
              const state = states[row.id] ?? row.model?.downloadState ?? null;
              const installed =
                row.installed || state === "installed" || row.model?.installed === true;
              // 只有「未安装」时才把 queued/downloading/verifying 视为进行中
              // （mock / 真实后端可能同时返回 installed=true 与历史 downloadState）。
              const active =
                !installed &&
                (state === "queued" || state === "downloading" || state === "verifying");
              const item = progress[row.id];
              const percent =
                item && item.total > 0
                  ? Math.min(100, Math.round((item.loaded / item.total) * 100))
                  : 0;
              return (
                <div key={row.id} className="sa-ocr-row">
                  <div className="sa-ocr-row-main">
                    <div className="sa-ocr-row-title">
                      <span>{ocrAssetLabel(row.id, row.name)}</span>
                      {row.bundled ? (
                        <Badge tone="success" title="随安装包提供，无需下载">
                          随安装包内置
                        </Badge>
                      ) : installed ? (
                        <Badge tone="success">已下载</Badge>
                      ) : active ? (
                        <Badge tone="warning">{state ? STATE_LABELS[state] : "下载中"}</Badge>
                      ) : (
                        <Badge tone="warning">未下载</Badge>
                      )}
                    </div>
                    <p className="sa-ocr-row-meta">{formatBytes(row.sizeBytes)}</p>
                    {active ? (
                      <div className="sa-ocr-progress">
                        <div className="sa-progressbar">
                          <div className="sa-progressbar-fill" style={{ width: `${percent}%` }} />
                        </div>
                        <span className="sa-ocr-progress-text">
                          {item
                            ? `${formatBytes(item.loaded)} / ${formatBytes(item.total)} · ${formatSpeed(item.speedBps)} · ${percent}%`
                            : "正在准备下载…"}
                        </span>
                      </div>
                    ) : null}
                  </div>
                  {!row.bundled ? (
                    <div className="sa-ocr-row-actions">
                      {installed && !compact ? (
                        <Button
                          variant="danger"
                          size="sm"
                          disabled={busyId === row.id}
                          onClick={() => void removeAsset(row)}
                        >
                          删除
                        </Button>
                      ) : active ? (
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={busyId === row.id}
                          onClick={() => void cancelDownload(row)}
                        >
                          取消
                        </Button>
                      ) : installed ? null : row.model ? (
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={busyId === row.id}
                          onClick={() => void startDownload(row)}
                        >
                          {busyId === row.id
                            ? "正在开始…"
                            : state === "failed"
                              ? "重试下载"
                              : "下载"}
                        </Button>
                      ) : (
                        <span className="sa-ocr-row-hint">缺少下载信息</span>
                      )}
                    </div>
                  ) : null}
                </div>
              );
            })}
          </div>

          {status && !status.runtimeReady ? (
            <p className="sa-ocr-warn">
              识别引擎文件缺失，暂时无法识别文字。重新安装应用通常可以修复。
            </p>
          ) : null}

          {note ? <p className="sa-ocr-note">{note}</p> : null}
          {!compact && edition === "slim" && pending.length > 0 ? (
            <p className="sa-ocr-note">
              这个安装包不含文字识别模型：首次使用时会自动下载（约 16 MB），也可以稍后再下载。
            </p>
          ) : null}

          {pending.length > 0 && pending.some((row) => row.model !== null) ? (
            <div className="sa-ocr-actions">
              <Button
                variant="secondary"
                size="sm"
                disabled={busyId !== null}
                onClick={() => void downloadAll()}
              >
                下载全部模型
              </Button>
            </div>
          ) : null}

          {status && status.missing.length > 0 ? (
            <div className="sa-ocr-actions">
              <Button
                variant="ghost"
                size="sm"
                disabled={busyId !== null}
                onClick={() => void openModelsDir()}
              >
                打开模型文件夹
              </Button>
              <span className="sa-ocr-row-hint">
                也可以把下载好的模型文件放进这个文件夹，再点「重新检查」。
              </span>
            </div>
          ) : null}
        </>
      )}

      {actionError ? (
        <p className="sa-ocr-error" role="alert">
          {actionError}
        </p>
      ) : null}
      {notice ? <p className="sa-ocr-notice">{notice}</p> : null}
    </div>
  );
}
