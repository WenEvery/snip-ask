/** 引导页内置预设（静态文案参考可行性报告 §2.4.3 / §3.2.6） */

import type { Cap, Language, Mode } from "@/shared/contract/types";

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
    caps: ["vision", "search", "thinking"],
    note: "推荐，支持视觉与联网搜索",
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

export interface LocalTier {
  id: string;
  title: string;
  description: string;
  sizeText: string;
  ramText: string;
  licenseText: string;
  speedText: string;
}

/** 报告 §2.4.3 的本地翻译档位 */
export const LOCAL_TIERS: LocalTier[] = [
  {
    id: "light",
    title: "轻量档",
    description: "Bergamot / OPUS-MT 类小模型，低配机器可用",
    sizeText: "约 100–300 MB",
    ramText: "内存 < 1 GB",
    licenseText: "MIT / CC-BY 等开源许可",
    speedText: "约 20–40 句/秒",
  },
  {
    id: "quality",
    title: "质量档",
    description: "HY-MT1.5-1.8B 量化版，译文更自然",
    sizeText: "约 1.2 GB",
    ramText: "建议 16 GB 内存",
    licenseText: "Apache-2.0",
    speedText: "约 5–10 句/秒",
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
