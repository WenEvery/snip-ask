/**
 * 配置差异计算：设置页按分组「只提交差异」。
 *
 * 契约 §4.3：`config_patch` 为深度合并，数组整体替换。
 * 因此这里对每个顶层键做整体比较，只把真正变化的键放进 patch。
 */

import type { Config, ConfigView } from "@/shared/contract/types";
import type { ConfigPatch } from "./types";

/** 配置顶层键（用于差异计算） */
export const CONFIG_KEYS: readonly (keyof Config)[] = [
  "app",
  "ui",
  "hotkeys",
  "actions",
  "providers",
  "modes",
  "search",
  "translate",
  "local",
  "ocr",
  "privacy",
  "history",
  "advanced",
];

function stableString(value: unknown): string {
  try {
    return JSON.stringify(value) ?? "undefined";
  } catch {
    return "unserializable";
  }
}

/** providers 入参是 `Provider`（契约 §4），不能把 `hasKey` / `keyMask` 提交给后端 */
function sanitizeProviders(value: unknown): unknown {
  if (!Array.isArray(value)) return value;
  return value.map((item) => {
    const provider = item as Record<string, unknown>;
    return {
      id: provider.id,
      name: provider.name,
      baseUrl: provider.baseUrl,
      model: provider.model,
      // 契约 v5：Provider.kind（OpenAI 兼容 / Qwen-MT），漏掉会被后端按默认值覆盖。
      kind: provider.kind,
      caps: provider.caps,
      temperature: provider.temperature,
      maxTokens: provider.maxTokens,
      enabled: provider.enabled,
    };
  });
}

/**
 * 计算 `draft` 相对 `baseline` 的差异补丁。
 * - `keys` 只传本分组负责的顶层键，实现「只提交该分组」。
 * - 数组整体替换；对象整体提交（后端深合并）。
 */
export function buildPatch(
  baseline: ConfigView,
  draft: ConfigView,
  keys: readonly (keyof Config)[] = CONFIG_KEYS,
): ConfigPatch {
  const base = baseline as unknown as Record<string, unknown>;
  const next = draft as unknown as Record<string, unknown>;
  const patch: Record<string, unknown> = {};
  for (const key of keys) {
    if (stableString(base[key]) === stableString(next[key])) continue;
    patch[key] = key === "providers" ? sanitizeProviders(next[key]) : next[key];
  }
  return patch as ConfigPatch;
}

/** 分组内是否存在未提交修改 */
export function hasChanges(
  baseline: ConfigView,
  draft: ConfigView,
  keys: readonly (keyof Config)[] = CONFIG_KEYS,
): boolean {
  return Object.keys(buildPatch(baseline, draft, keys)).length > 0;
}
