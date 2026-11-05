/**
 * 引擎标识 → 面向用户的中文标签（结果窗 / 覆盖翻译 / 引擎状态共用）。
 *
 * 契约里 `engine` 是自由字符串（`EngineId = string`）：
 * - `"skeleton"`：骨架 / 占位实现（调试模式或浏览器预览）；
 * - `"local"`：本地模型（llama.cpp / CTranslate2）；
 * - `"cloud"`：云端（未指明具体供应商时）；
 * - 其余取值按供应商 id 原样显示（如 `"deepseek"`、`"dashscope"`）。
 */

export interface EngineLabelInfo {
  /** 展示用短标签。 */
  label: string;
  /** 是否是占位 / 骨架实现（用于警示色或「预览」徽标）。 */
  skeleton: boolean;
}

export function engineLabelInfo(engine: string | null | undefined): EngineLabelInfo | null {
  if (!engine) return null;
  switch (engine) {
    case "skeleton":
      return { label: "骨架模式", skeleton: true };
    case "local":
      return { label: "本地模型", skeleton: false };
    case "cloud":
      return { label: "云端", skeleton: false };
    default:
      return { label: engine, skeleton: false };
  }
}

/** 展示用标签；空值返回 null（调用方自行决定是否渲染）。 */
export function engineLabel(engine: string | null | undefined): string | null {
  return engineLabelInfo(engine)?.label ?? null;
}

/** 原始引擎 id（`title` 提示，便于用户反馈问题时提供）。 */
export function engineTitle(engine: string | null | undefined): string | undefined {
  return engine ? `实际引擎：${engine}` : undefined;
}
