/** 引导页内置预设（静态文案参考可行性报告 §2.4.3 / §3.2.6） */

import type { Cap, Language, Mode, ProviderKind, TranslateBackend } from "@/shared/contract/types";
import { t } from "../../../i18n";

export interface OnboardingProviderPreset {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  kind: ProviderKind;
  caps: Cap[];
  note: string;
}

/**
 * 引导页供应商预设（契约 v5 §3.1：三条，顺序与默认 provider 一致）。
 *
 * 与设置页 `PROVIDER_PRESETS` 保持同一事实：DeepSeek 默认启用；DashScope 仅翻译
 * （`kind: "qwenMt"`，需 DashScope Key）；通用 OpenAI 兼容需自填 Base URL。
 */
export const ONBOARDING_PROVIDERS: OnboardingProviderPreset[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com",
    model: "deepseek-flash",
    kind: "openAiCompatible",
    caps: ["vision", "search", "thinking", "translate"],
    note: t("默认推荐：能看图、能联网搜索，回答质量高"),
  },
  {
    id: "dashscope",
    name: "DashScope（Qwen-MT 翻译）",
    baseUrl: "https://dashscope.aliyuncs.com",
    model: "qwen-mt-lite",
    kind: "qwenMt",
    caps: ["translate"],
    note: t("专门的翻译服务：填入 DashScope 的 Key 后可以用于翻译（不参与搜索和看图）"),
  },
  {
    id: "generic-openai",
    name: "通用 OpenAI 兼容",
    baseUrl: "https://api.openai.com",
    model: "gpt-4o-mini",
    kind: "openAiCompatible",
    caps: ["vision", "search", "thinking", "translate"],
    note: t("支持任何 OpenAI 兼容的服务，可自行填写地址和模型名"),
  },
];

/** 「极小模型」id（与 models.manifest.json 一致，task-9 新增）。 */
export const TINY_LOCAL_MODEL_ID = "qwen2.5-0.5b-instruct-q4";
/** 「普通模型」id（沿用现有 HY-MT1.5-1.8B）。 */
export const STANDARD_LOCAL_MODEL_ID = "hy-mt-1.8b-q4";

export type TranslateTierId = "cloud" | "tiny" | "standard";

export interface TranslateTier {
  id: TranslateTierId;
  title: string;
  tag: string | null;
  description: string;
  /** 写回 `draft.translate.backend`。 */
  backend: TranslateBackend;
  /** 写回 `draft.translate.modelId`；云端为 null。 */
  modelId: string | null;
  /** 体积 / 内存提示（云端为 null）。 */
  sizeText: string | null;
  ramText: string | null;
  /** 性能文案：使用 S-05/S-06 实测数据，不足处明确标注「预估/待实测」。 */
  speedText: string;
  /** 质量/隐私/成本定位文案（用户选本地的主要理由）。 */
  qualityText: string;
}

/**
 * 引导第 3 步「翻译方式」三档。
 *
 * 速度文案来自 S-05/S-06 实测（200 字、纯 CPU），这里改成用户能直接理解的说法。
 */
export const TRANSLATE_TIERS: TranslateTier[] = [
  {
    id: "cloud",
    title: t("云端翻译"),
    tag: t("推荐 · 最快"),
    description: t("需要 API Key；不占磁盘和内存，按使用量付费"),
    backend: "cloud",
    modelId: null,
    sizeText: null,
    ramText: null,
    speedText: t("通常 1 秒左右就能出结果"),
    qualityText: t("效果最好；需要联网，文字会上传到你的 AI 服务"),
  },
  {
    id: "tiny",
    title: t("小模型（本地）"),
    tag: t("省资源"),
    description: t("适合低配电脑，不上传内容、不按次收费"),
    backend: "local",
    modelId: TINY_LOCAL_MODEL_ID,
    sizeText: t("约 470 MB"),
    ramText: t("内存约 1 GB"),
    speedText: t("短句通常 1 秒内，长段落几秒"),
    qualityText: t("速度快、占用低；译文质量一般，适合短句和日常使用"),
  },
  {
    id: "standard",
    title: t("普通模型（本地）"),
    tag: t("本地效果最好"),
    description: t("专注翻译的本地模型，译文更自然"),
    backend: "local",
    modelId: STANDARD_LOCAL_MODEL_ID,
    sizeText: t("约 1.08 GB"),
    ramText: t("内存约 2.1 GB"),
    speedText: t("一段 200 字大约需要 10–20 秒（仅用 CPU 时）"),
    qualityText: t("译文质量好，但纯 CPU 运行较慢；适合愿意等待、看重质量的场景"),
  },
];

export interface ModeRow {
  key: "search" | "imageSearch" | "translate" | "answer";
  label: string;
  description: string;
  quick: string;
  deep: string;
}

/** 报告 §3.2.6：快速 1–3s / 1x；深度 5–25s / 3–10x */
export const MODE_ROWS: ModeRow[] = [
  {
    key: "search",
    label: t("文本搜索"),
    description: t("框选文字后生成检索结论与来源"),
    quick: t("1–3 秒 · 花费少"),
    deep: t("5–25 秒 · 花费高"),
  },
  {
    key: "imageSearch",
    label: t("图片搜索"),
    description: t("识别图像内容并检索来源"),
    quick: t("2–4 秒 · 花费少"),
    deep: t("6–25 秒 · 花费高"),
  },
  {
    key: "translate",
    label: t("翻译"),
    description: t("框选文字直接翻译为目标语言"),
    quick: t("1–3 秒 · 花费少"),
    deep: t("5–15 秒 · 花费较高"),
  },
  {
    key: "answer",
    label: t("解答"),
    description: t("解释、分析或改写框选内容"),
    quick: t("1–3 秒 · 花费少"),
    deep: t("5–25 秒 · 花费高"),
  },
];

export interface LanguageOption {
  value: Language;
  label: string;
  preview: string;
}

export const LANGUAGES: LanguageOption[] = [
  {
    value: "zh-CN",
    label: t("简体中文"),
    preview: t("框选屏幕任意区域，立刻获得搜索、翻译与解答。"),
  },
  {
    value: "en-US",
    label: "English",
    preview: "Select any area on screen to search, translate or explain instantly.",
  },
];

export const CAP_LABELS: Record<Cap, string> = {
  vision: t("视觉"),
  search: t("联网搜索"),
  thinking: t("深度思考"),
  translate: t("翻译"),
};

/** 所有动作默认快速 */
export const DEFAULT_MODES: Record<ModeRow["key"], Mode> = {
  search: "quick",
  imageSearch: "quick",
  translate: "quick",
  answer: "quick",
};

export const DEFAULT_CAPTURE_HOTKEY = "Ctrl+Q";

export const DEFAULT_THEME_OPTIONS = [
  { value: "system", label: t("跟随系统") },
  { value: "light", label: t("浅色") },
  { value: "dark", label: t("深色") },
] as const;
