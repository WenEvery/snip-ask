/**
 * 引导第 3 步：翻译方式 —— 三档选择（item3 用户决策）。
 *
 * 三档：云端翻译（需 Key，最快）/ 极小模型（Qwen2.5-0.5B，≈470MB，低配友好）/
 * 普通模型（HY-MT1.5-1.8B，≈1.08GB，质量好但纯 CPU 慢）。
 *
 * 选择即写回 `draft.translate = { backend, modelId, cloudModel }`：
 * - 云端：`backend="cloud"`，`modelId=null`（沿用既有 cloudModel：复用 AI 服务 / 专用 Qwen-MT）；
 * - 极小/普通：`backend="local"`，`modelId` 对应模型 id，并清空无关的 `cloudModel`。
 */

import { useState } from "react";
import { ModelDownloadList } from "../components/ModelDownloadList";
import { OptionCard } from "../components/OptionCard";
import { TINY_LOCAL_MODEL_ID, TRANSLATE_TIERS, type TranslateTier } from "../presets";
import type { StepProps } from "../types";

type TranslateDraft = NonNullable<StepProps["draft"]["translate"]>;
type TierId = TranslateTier["id"];

const FALLBACK: TranslateDraft = { backend: "cloud", modelId: null, cloudModel: "" };

/** 由草稿反推当前档位（云端 / 极小 / 普通）。 */
function resolveTierId(translate: TranslateDraft): TierId {
  if (translate.backend === "cloud") return "cloud";
  if (translate.modelId === TINY_LOCAL_MODEL_ID) return "tiny";
  return "standard";
}

export function TranslateStep(props: StepProps) {
  const { draft, onEdit, onError } = props;
  const translate = draft.translate ?? FALLBACK;
  const [cloudMode, setCloudMode] = useState<"reuse" | "separate">(
    translate.cloudModel ? "separate" : "reuse",
  );
  const activeTierId = resolveTierId(translate);
  const activeTier = TRANSLATE_TIERS.find((item) => item.id === activeTierId) ?? TRANSLATE_TIERS[0];

  const update = (patch: Partial<TranslateDraft>) =>
    onEdit({ translate: { ...translate, ...patch } });

  const selectTier = (tier: TranslateTier) => {
    update({
      backend: tier.backend,
      modelId: tier.modelId,
      // 切到本地时清空云端专用字段；切回云端时保留用户已填的 cloudModel。
      cloudModel: tier.backend === "cloud" ? translate.cloudModel : "",
    });
  };

  return (
    <div className="ob-step">
      <h2 className="ob-step-title">选择翻译方式</h2>
      <p className="ob-hint">
        三种方式任选其一：云端翻译最快、效果最好（需要 API
        Key，文字会上传）；本地模型不上传、不按次收费， 但用 CPU 运行会慢一些。之后可以随时在「设置
        → 翻译」中更改。
      </p>

      <div className="ob-cards3">
        {TRANSLATE_TIERS.map((tier) => (
          <OptionCard
            key={tier.id}
            selected={activeTierId === tier.id}
            title={tier.title}
            description={tier.description}
            onSelect={() => selectTier(tier)}
          >
            <span className="ob-tier-meta">
              {tier.tag ? <strong className="ob-tier-tag">{tier.tag}</strong> : null}
              {tier.sizeText ? (
                <span>
                  体积 {tier.sizeText} · {tier.ramText}
                </span>
              ) : null}
              <span>{tier.speedText}</span>
              <span>{tier.qualityText}</span>
            </span>
          </OptionCard>
        ))}
      </div>

      {activeTierId === "cloud" ? (
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
              <span className="ob-hint">
                使用同一供应商的 Key，可在设置 → 翻译中调整目标语言与术语表。
              </span>
            </label>
          ) : (
            <p className="ob-hint">
              将使用上一步填写的 AI 服务；稍后可以在「设置 → 翻译」中换成专门的翻译模型。
            </p>
          )}
        </div>
      ) : (
        <div className="ob-block">
          <h3 className="ob-block-title">
            {activeTierId === "tiny" ? "小模型（约 470 MB）" : "普通模型（约 1.08 GB）"}
          </h3>
          <p className="ob-notice">
            本地模型的好处是内容不离开本机、不按次数收费；代价是只用 CPU 时会慢一些
            {activeTierId === "standard"
              ? "（一段 200 字大约要 10–20 秒）"
              : "（小模型译文质量一般）"}
            。 下载会在后台继续，中途退出也没关系；模型没下载完之前，本地翻译暂时用不了。
          </p>
          <h3 className="ob-block-title">对应模型下载</h3>
          <ModelDownloadList
            selectedModelId={translate.modelId}
            focusModelId={activeTier.modelId}
            onSelectModel={(modelId) => update({ modelId })}
            onError={onError}
          />
          {translate.modelId ? (
            <p className="ob-hint">已选用模型：{translate.modelId}；保存后生效。</p>
          ) : (
            <p className="ob-hint">尚未选用模型；也可以先跳过，稍后在设置 → 翻译中下载。</p>
          )}
        </div>
      )}
    </div>
  );
}
