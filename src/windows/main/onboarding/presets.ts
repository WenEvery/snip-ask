/** 引导页内置预设（静态文案参考可行性报告 §2.4.3 / §3.2.6） */

import type { Cap, Language, Mode, TranslateBackend } from "@/shared/contract/types";

export interface OnboardingProviderPreset {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  caps: Cap[];
  note: string;
}

/** 报告 §2.4.3 的供应商预设 */
export const ONBOARDING_PROVIDERS: OnboardingProviderPreset[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com",
    model: "deepseek-flash",
    caps: ["vision", "thinking"],
    note: "文本+视觉（deepseek-flash，实测可识别截图）；无联网搜索 API；默认思考会显著推高首字延迟",
  },
  {
    id: "openai",
    name: "OpenAI",
    baseUrl: "https://api.openai.com",
    model: "gpt-4o-mini",
    caps: ["vision", "search", "thinking"],
    note: "OpenAI 兼容接口",
  },
  {
    id: "qwen",
    name: "通义千问",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-mt-lite",
    caps: ["vision", "translate"],
    note: "Qwen-MT 专用翻译模型",
  },
  {
    id: "ollama",
    name: "Ollama（本地）",
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "qwen2.5:7b",
    caps: [],
    note: "本机服务，无需 Key",
  },
  {
    id: "custom",
    name: "自定义",
    baseUrl: "https://",
    model: "",
    caps: [],
    note: "任意 OpenAI 兼容端点",
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
 * 引导第 3 步「翻译方式」三档（item3 用户决策）。
 *
 * 数据来源：`docs/spikes/data/s05_s06/summary_s05_s06.json`（2026-11-05 实测）
 * - 普通模型（HY-MT1.5-1.8B Q4，200 字）：P50 11.76s / P95 21.84s / 最快 9.89s；
 * - 极小模型（Qwen2.5-0.5B-Instruct Q4 + 精简提示词，200 字）：P50 3.09s / P95 3.21s，常驻 ≈0.55 GB；
 *   注意：必须配 `resources/prompts/translate_minimal.md`，用产品长提示词会复述原文（S-05 实测）；
 * - 云端（DeepSeek deepseek-flash，200 字）：thinking 关闭时首字 P50 0.61s / P95 0.87s（达标）；
 *   默认 thinking 时首字 P95 4.59s（不达标，网关接入时需显式关闭思考）。
 */
export const TRANSLATE_TIERS: TranslateTier[] = [
  {
    id: "cloud",
    title: "云端翻译",
    tag: "推荐 · 最快",
    description: "需 API Key；不占磁盘与内存，按量计费",
    backend: "cloud",
    modelId: null,
    sizeText: null,
    ramText: null,
    speedText: "实测（DeepSeek deepseek-flash / thinking 关闭）：首字 P50 0.61s / P95 0.87s 达标；默认 thinking 首字 P95 4.59s（不达标，接入网关需显式关闭思考）",
    qualityText: "质量最高（大模型）；需联网并同意文本上传，按量计费",
  },
  {
    id: "tiny",
    title: "极小模型（本地）",
    tag: "低配友好",
    description: "Qwen2.5-0.5B-Instruct Q4_K_M，隐私/离线/零成本",
    backend: "local",
    modelId: TINY_LOCAL_MODEL_ID,
    sizeText: "≈470 MB",
    ramText: "内存 ≈1 GB",
    speedText: "实测 P50 3.1s / P95 3.2s（200 字，纯 CPU；需精简提示词）；30 字短句约 0.4s",
    qualityText: "快、省内存（常驻 ≈0.55 GB）；译文质量一般，适合短句/轻度使用，长文建议云端或普通模型",
  },
  {
    id: "standard",
    title: "普通模型（本地）",
    tag: "本地推荐质量",
    description: "HY-MT1.5-1.8B Q4，专注翻译、译文更自然",
    backend: "local",
    modelId: STANDARD_LOCAL_MODEL_ID,
    sizeText: "≈1.08 GB",
    ramText: "常驻内存 ≈2.1 GB",
    speedText: "实测 P50 11.76s / P95 21.84s / 最快 9.89s（200 字，纯 CPU i7-12650H）",
    qualityText: "质量好但纯 CPU 慢：8s 目标仅云端满足；适合重质量、可等待的场景",
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
    label: "文本搜索",
    description: "框选文字后生成检索结论与来源",
    quick: "1–3s · 成本 1x",
    deep: "5–25s · 成本 3–10x",
  },
  {
    key: "imageSearch",
    label: "图片搜索",
    description: "识别图像内容并检索来源",
    quick: "2–4s · 成本 1–2x",
    deep: "6–25s · 成本 3–10x",
  },
  {
    key: "translate",
    label: "翻译",
    description: "框选文字直接翻译为目标语言",
    quick: "1–3s · 成本 1x",
    deep: "5–15s · 成本 3–5x",
  },
  {
    key: "answer",
    label: "解答",
    description: "解释、分析或改写框选内容",
    quick: "1–3s · 成本 1x",
    deep: "5–25s · 成本 3–10x",
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
    label: "简体中文",
    preview: "框选屏幕任意区域，立刻获得搜索、翻译与解答。",
  },
  {
    value: "en-US",
    label: "English",
    preview: "Select any area on screen to search, translate or explain instantly.",
  },
];

export const CAP_LABELS: Record<Cap, string> = {
  vision: "视觉",
  search: "联网搜索",
  thinking: "深度思考",
  translate: "翻译",
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
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
] as const;
