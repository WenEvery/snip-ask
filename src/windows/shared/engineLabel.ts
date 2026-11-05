/**
 * 引擎标识 → 面向用户的中文标签（结果窗 / 覆盖翻译共用）。
 *
 * 契约里 `engine` 是自由字符串：
 * - `"skeleton"`：浏览器预览下的演示数据；
 * - `"local"`：本地模型；
 * - `"cloud"`：云端（未指明具体供应商时）；
 * - 其余取值按供应商 id 原样显示（如 `"deepseek"`、`"dashscope"`）。
 */

export interface EngineLabelInfo {
  /** 展示用短标签。 */
  label: string;
  /** 是否是演示数据（用于提示）。 */
  demo: boolean;
}

export function engineLabelInfo(engine: string | null | undefined): EngineLabelInfo | null {
  if (!engine) return null;
  switch (engine) {
    case "skeleton":
      return { label: "演示", demo: true };
    case "local":
      return { label: "本地模型", demo: false };
    case "cloud":
      return { label: "云端", demo: false };
    default:
      return { label: engine, demo: false };
  }
}

/** 展示用标签；空值返回 null（调用方自行决定是否渲染）。 */
export function engineLabel(engine: string | null | undefined): string | null {
  return engineLabelInfo(engine)?.label ?? null;
}

/** 鼠标悬停时的说明（方便反馈问题时提供具体来源）。 */
export function engineTitle(engine: string | null | undefined): string | undefined {
  if (!engine) return undefined;
  if (engine === "skeleton") return "预览模式下的示例回答";
  if (engine === "local") return "由本机模型生成";
  if (engine === "cloud") return "由云端服务生成";
  return `由 ${engine} 生成`;
}
