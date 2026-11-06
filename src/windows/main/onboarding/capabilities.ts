/**
 * S-07 受限模式：前端能力判定（纯函数，便于 UI 复用与解释）。
 *
 * 与后端 `domain::action::Capabilities` 语义一致：
 * - `cloudAvailable`：存在 `enabled` 且 `hasKey` 的供应商（`ConfigView` 提供）；
 * - `localOcrAvailable`：内嵌本地 OCR 始终可用（骨架模式视为可用）；
 * - `browserSearchAvailable`：外部浏览器搜索始终可用；
 * - `localModelAvailable`：`models_list` 中至少一个 `installed`。
 *
 * 受限模式 = 无任何云端能力（`!cloudAvailable`）。此时本地 OCR、浏览器搜索、复制仍可用；
 * 本地模型就绪时本地翻译也可用；其余云动作由后端 `disabledReason` 给出可操作路径。
 */

import type { ConfigView, ModelInfo } from "@/shared/contract/types";
import { t } from "../../../i18n";

export interface Capabilities {
  cloudAvailable: boolean;
  localOcrAvailable: boolean;
  browserSearchAvailable: boolean;
  localModelAvailable: boolean;
}

export interface CapabilityInputs {
  providers?: Pick<ConfigView["providers"][number], "enabled" | "hasKey">[];
  models?: Pick<ModelInfo, "installed">[];
}

/** 由配置视图 + 模型列表派生能力快照。 */
export function deriveCapabilities(inputs: CapabilityInputs): Capabilities {
  return {
    cloudAvailable: (inputs.providers ?? []).some(
      (provider) => provider.enabled && provider.hasKey,
    ),
    localOcrAvailable: true,
    browserSearchAvailable: true,
    localModelAvailable: (inputs.models ?? []).some((model) => model.installed),
  };
}

/** 是否处于受限模式（无任何云端 Key）。 */
export function isRestricted(capabilities: Capabilities): boolean {
  return !capabilities.cloudAvailable;
}

/** 受限模式下仍可用的能力（中文枚举，用于提示文案）。 */
export function availableLabels(capabilities: Capabilities): string[] {
  const labels = [t("本地文字识别"), t("浏览器搜索"), t("复制文字")];
  if (capabilities.localModelAvailable) labels.push(t("本地翻译"));
  return labels;
}

/** 受限模式下不可用的能力（中文枚举，用于提示文案）。 */
export function unavailableLabels(capabilities: Capabilities): string[] {
  const labels = [t("AI 搜索"), t("解答"), t("图片搜索"), t("云端翻译")];
  if (!capabilities.localModelAvailable) labels.push(t("本地翻译"));
  return labels;
}

/** 「去配置」按钮的落地分组（设置页 section id）。 */
export const CONFIG_SECTION = "ai";
