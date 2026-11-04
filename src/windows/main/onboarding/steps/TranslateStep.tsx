/** 引导第 2 步：翻译方式 —— 云端（复用 AI 服务 / 单独模型）或本地（档位 + 实时下载） */

import { useState } from "react";
import type { TranslateBackend } from "@/shared/contract/types";
import { ModelDownloadList } from "../components/ModelDownloadList";
import { OptionCard } from "../components/OptionCard";
import { LOCAL_TIERS } from "../presets";
import type { StepProps } from "../types";

type TranslateDraft = NonNullable<StepProps["draft"]["translate"]>;

const FALLBACK: TranslateDraft = { backend: "cloud", modelId: null, cloudModel: "" };

export function TranslateStep(props: StepProps) {
  const { draft, onEdit, onError } = props;
  const translate = draft.translate ?? FALLBACK;
  const [cloudMode, setCloudMode] = useState<"reuse" | "separate">(
    translate.cloudModel ? "separate" : "reuse",
  );
  const [tier, setTier] = useState("light");

  const update = (patch: Partial<TranslateDraft>) => onEdit({ translate: { ...translate, ...patch } });

  const selectBackend = (backend: TranslateBackend) => update({ backend });

  return (
    <div className="ob-step">
      <h2 className="ob-step-title">选择翻译方式</h2>
      <p className="ob-hint">
        云端质量更高、无需下载；本地离线运行、更重隐私，但需要下载模型（建议 16 GB 内存体验质量档）。
      </p>

      <div className="ob-cards2">
        <OptionCard
          selected={translate.backend === "cloud"}
          title="云端翻译（推荐）"
          description="复用第 2 步的 AI 服务 Key，或使用单独的 Qwen-MT 模型"
          onSelect={() => selectBackend("cloud")}
        />
        <OptionCard
          selected={translate.backend === "local"}
          title="本地翻译（离线）"
          description="下载小模型后离线运行，不上传任何文本"
          onSelect={() => selectBackend("local")}
        />
      </div>

      {translate.backend === "cloud" ? (
        <div className="ob-block">
          <h3 className="ob-block-title">云端配置</h3>
          <div className="ob-radio-group">
            <label className="ob-radio">
              <input
                type="radio"
                name="cloud-mode"
                checked={cloudMode === "reuse"}
                onChange={() => {
                  setCloudMode("reuse");
                  update({ cloudModel: "", modelId: null });
                }}
              />
              <span>复用第 2 步的 AI 服务（推荐）</span>
            </label>
            <label className="ob-radio">
              <input
                type="radio"
                name="cloud-mode"
                checked={cloudMode === "separate"}
                onChange={() => setCloudMode("separate")}
              />
              <span>单独使用 Qwen-MT 专用翻译模型</span>
            </label>
          </div>
          {cloudMode === "separate" ? (
            <label className="ob-field">
              <span className="ob-field-label">翻译模型名</span>
              <input
                className="ob-input is-mono"
                value={translate.cloudModel}
                placeholder="qwen-mt-lite"
                onChange={(event) => update({ cloudModel: event.target.value.trim() })}
              />
              <span className="ob-hint">使用同一供应商的 Key，可在设置 → 翻译中调整目标语言与术语表。</span>
            </label>
          ) : (
            <p className="ob-hint">将复用上一步填写的供应商；稍后可在设置 → 翻译中改为专用模型。</p>
          )}
        </div>
      ) : (
        <div className="ob-block">
          <h3 className="ob-block-title">本地模型档位</h3>
          <div className="ob-cards2">
            {LOCAL_TIERS.map((item) => (
              <OptionCard
                key={item.id}
                selected={tier === item.id}
                title={item.title}
                description={item.description}
                onSelect={() => setTier(item.id)}
              >
                <span className="ob-tier-meta">
                  <span>体积 {item.sizeText}</span>
                  <span>{item.ramText}</span>
                  <span>许可 {item.licenseText}</span>
                  <span>{item.speedText}</span>
                </span>
              </OptionCard>
            ))}
          </div>
          <p className="ob-notice">
            下载前请预留约 {tier === "quality" ? "1.5 GB" : "400 MB"} 磁盘空间（模型体积 + 20%）；下载支持断点续传与
            SHA256 校验，可在后台继续。
          </p>
          <h3 className="ob-block-title">可下载模型</h3>
          <ModelDownloadList
            selectedModelId={translate.modelId}
            onSelectModel={(modelId) => update({ modelId })}
            onError={onError}
          />
          {translate.modelId ? (
            <p className="ob-hint">已选用模型：{translate.modelId}；保存完成时会写入翻译配置。</p>
          ) : (
            <p className="ob-hint">尚未选用模型；也可以先跳过，稍后在设置 → 翻译中下载。</p>
          )}
        </div>
      )}
    </div>
  );
}
