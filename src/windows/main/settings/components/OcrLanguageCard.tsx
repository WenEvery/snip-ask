/**
 * 设置 · 高级 →「文字识别」卡片：识别语言切换 + 语言包按需下载。
 *
 * 契约（v7 增量）：
 * - 配置 `ocr.language`（`zh` / `ru` / `ja` / `ko`，保存后后端热切换引擎语言）；
 * - `ocr_status` 返回 `language` 与 `languages: [{ language, installed }]`；
 * - 语言包资产 id 固定 `ocr-lang-ru` / `ocr-lang-ja` / `ocr-lang-ko`，
 *   下载复用 `models_download` / `models_cancel` 与 `download:progress` / `download:state` 事件。
 *
 * 语言选择只改草稿（`onChange`），由设置页既有 draft/patch 机制保存；
 * 语言包下载完成时刷新 `ocr_status`，解除「未下载」选项的禁用。
 */

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui";
import type {
  DownloadProgressPayload,
  DownloadState,
  DownloadStatePayload,
  ModelInfo,
  OcrLanguage,
  OcrStatus,
} from "@/shared/contract/types";
import { COMMANDS, EVENTS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { t } from "@/i18n";
import { useIpcEvent } from "@/shared/hooks/useIpcEvent";
import { errorMessage } from "../error";
import { formatBytes, formatSpeed } from "../format";
import { Hint, SelectField, WarnText } from "../parts";
import {
  isOcrLanguage,
  isOcrLanguagePackId,
  languageInstalled,
  languagePackSize,
  ocrLanguageLabel,
  ocrLanguageOptions,
  ocrLanguagePackId,
  pendingLanguagePacks,
} from "../../ocr/status";

export interface OcrLanguageCardProps {
  /** 当前草稿里的识别语言（`draft.ocr.language`）。 */
  language: OcrLanguage;
  /** 切换识别语言（只改草稿，保存由设置页提交 `ocr.language`）。 */
  onChange: (language: OcrLanguage) => void;
  /** 上报下载失败等错误到设置页顶部提示。 */
  onError: (error: unknown) => void;
}

const ACTIVE_STATES: DownloadState[] = ["queued", "downloading", "verifying"];

export function OcrLanguageCard({ language, onChange, onError }: OcrLanguageCardProps) {
  const [status, setStatus] = useState<OcrStatus | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [states, setStates] = useState<Record<string, DownloadState>>({});
  const [progress, setProgress] = useState<Record<string, DownloadProgressPayload>>({});

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [nextStatus, nextModels] = await Promise.all([
        ipc.invoke<OcrStatus>(COMMANDS.ocrStatus),
        ipc.invoke<ModelInfo[]>(COMMANDS.modelsList),
      ]);
      setStatus(nextStatus);
      setModels(nextModels);
      setLoadError(null);
    } catch (err) {
      setLoadError(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useIpcEvent<DownloadProgressPayload>(EVENTS.downloadProgress, (payload) => {
    if (!isOcrLanguagePackId(payload.model)) return;
    setProgress((current) => ({ ...current, [payload.model]: payload }));
  });

  useIpcEvent<DownloadStatePayload>(EVENTS.downloadState, (payload) => {
    if (!isOcrLanguagePackId(payload.model)) return;
    setStates((current) => ({ ...current, [payload.model]: payload.state }));
    if (
      payload.state === "installed" ||
      payload.state === "failed" ||
      payload.state === "cancelled"
    ) {
      // 语言包就位 / 失败 / 取消后重新读取：下载完成即可解除下拉选项的禁用。
      void refresh();
    }
  });

  const startDownload = async (id: string) => {
    setBusyId(id);
    try {
      await ipc.invoke<string>(COMMANDS.modelsDownload, { id });
      setStates((current) => ({ ...current, [id]: "queued" }));
    } catch (err) {
      onError(err);
    } finally {
      setBusyId(null);
    }
  };

  const cancelDownload = async (id: string) => {
    setBusyId(id);
    try {
      await ipc.invoke<void>(COMMANDS.modelsCancel, { id });
    } catch (err) {
      onError(err);
    } finally {
      setBusyId(null);
    }
  };

  const options = ocrLanguageOptions(status, t);
  const pending = pendingLanguagePacks(status);
  const currentMissing = status !== null && !languageInstalled(status, language);

  return (
    <div className="sa-ocr-language">
      <div className="sa-inline">
        <SelectField
          label={t("识别语言")}
          value={language}
          options={options}
          onChange={(value) => {
            if (isOcrLanguage(value)) onChange(value);
          }}
        />
        <Button variant="ghost" size="sm" disabled={loading} onClick={() => void refresh()}>
          {loading ? t("检查中…") : t("重新检查")}
        </Button>
      </div>

      {loadError ? (
        <WarnText>{t("读取语言包状态失败：{error}", { error: loadError })}</WarnText>
      ) : null}
      {currentMissing ? (
        <WarnText>
          {t("当前识别语言的语言包未下载，识别暂时不可用；请下载语言包或改回中文。")}
        </WarnText>
      ) : null}

      {pending.length > 0 ? (
        <div className="sa-models">
          {pending.map((entry) => {
            const id = ocrLanguagePackId(entry.language) ?? "";
            const model = models.find((item) => item.id === id) ?? null;
            const state = states[id] ?? model?.downloadState ?? null;
            const installed = states[id] === "installed" || model?.installed === true;
            const active = !installed && state !== null && ACTIVE_STATES.includes(state);
            const item = progress[id];
            const percent =
              item && item.total > 0
                ? Math.min(100, Math.round((item.loaded / item.total) * 100))
                : 0;
            return (
              <div key={id} className="sa-model-row">
                <div className="sa-model-main">
                  <div className="sa-model-title">
                    <strong>
                      {t("{language}语言包（约 {size}）", {
                        language: ocrLanguageLabel(entry.language, t),
                        size: formatBytes(languagePackSize(models, entry.language)),
                      })}
                    </strong>
                  </div>
                  {active ? (
                    <div className="sa-progress">
                      <div className="sa-progressbar">
                        <div className="sa-progressbar-fill" style={{ width: `${percent}%` }} />
                      </div>
                      <span className="sa-progress-text">
                        {item
                          ? `${formatBytes(item.loaded)} / ${formatBytes(item.total)} · ${formatSpeed(item.speedBps)} · ${percent}%`
                          : t("正在准备下载…")}
                      </span>
                    </div>
                  ) : state === "failed" ? (
                    <p className="sa-field-hint">
                      {t("下载失败，可以重试；如果网络不稳定，稍后再试也可以。")}
                    </p>
                  ) : null}
                </div>
                <div className="sa-model-actions">
                  {active ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={busyId === id}
                      onClick={() => void cancelDownload(id)}
                    >
                      {t("取消下载")}
                    </Button>
                  ) : (
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={busyId === id}
                      onClick={() => void startDownload(id)}
                    >
                      {busyId === id
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
        </div>
      ) : null}

      <Hint>{t("语言包下载后立即生效；中文模型同时支持英文、数字与常见符号。")}</Hint>
    </div>
  );
}
