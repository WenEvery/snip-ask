/** 设置页与引导共用的枚举/预设（本地常量，不改契约） */

import type { ActionId, Cap, Hotkeys, LocalEngine } from "@/shared/contract/types";

export const ACTION_LABELS: Record<ActionId, string> = {
  search: "文本搜索",
  imageSearch: "图片搜索",
  translate: "翻译",
  answer: "解答",
  copy: "复制文字",
};

/** 文本选区下可出现的动作（契约 §4.1 默认值） */
export const TEXT_MENU_ACTIONS: ActionId[] = ["search", "translate", "answer", "copy"];
/** 图片选区下可出现的动作 */
export const IMAGE_MENU_ACTIONS: ActionId[] = ["imageSearch", "answer", "translate"];

export const CAP_LABELS: Record<Cap, string> = {
  vision: "视觉（看图）",
  search: "联网搜索",
  thinking: "深度思考",
  translate: "翻译",
};

export const ALL_CAPS: Cap[] = ["vision", "search", "thinking", "translate"];

/** 翻译目标语言（契约无枚举，后端接受 BCP-47 风格代码） */
export const TARGET_LANGS: { value: string; label: string }[] = [
  { value: "zh-CHS", label: "简体中文" },
  { value: "zh-CHT", label: "繁体中文" },
  { value: "en", label: "英语" },
  { value: "ja", label: "日语" },
  { value: "ko", label: "韩语" },
  { value: "fr", label: "法语" },
  { value: "de", label: "德语" },
  { value: "es", label: "西班牙语" },
  { value: "ru", label: "俄语" },
];

export const CUSTOM_LANG = "__custom__";

export const SEARCH_ENGINES: { value: string; label: string }[] = [
  { value: "bing", label: "Bing" },
  { value: "google", label: "Google" },
  { value: "baidu", label: "百度" },
  { value: "duckduckgo", label: "DuckDuckGo" },
  { value: "custom", label: "自定义 URL 模板" },
];

export const LOCAL_ENGINES: { value: LocalEngine; label: string }[] = [
  { value: "llama.cpp", label: "llama.cpp（内置二进制）" },
  { value: "ollama", label: "Ollama（复用本机服务）" },
  { value: "ct2", label: "CTranslate2（低配兜底）" },
];

export const UPDATE_CHANNELS: { value: string; label: string }[] = [
  { value: "stable", label: "稳定通道（推荐）" },
  { value: "preview", label: "预览通道（可能不稳定）" },
];

export const DEFAULT_HOTKEYS: Hotkeys = {
  capture: "Ctrl+Q",
  wordGrab: "",
  quickTranslate: "",
  imageSearch: "",
  translate: "",
  answer: "",
  toggleMode: "",
  togglePause: "",
  openSettings: "",
};

export interface ProviderPreset {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  caps: Cap[];
  temperature: number;
  maxTokens: number;
  note: string;
}

/** 契约 §4.1 的 DeepSeek 默认 + 报告 §2.4.3 的预设供应商 */
export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com",
    model: "deepseek-flash",
    caps: ["vision", "search", "thinking"],
    temperature: 0.2,
    maxTokens: 1024,
    note: "默认推荐，支持视觉与联网搜索",
  },
  {
    id: "openai",
    name: "OpenAI",
    baseUrl: "https://api.openai.com",
    model: "gpt-4o-mini",
    caps: ["vision", "search", "thinking"],
    temperature: 0.2,
    maxTokens: 1024,
    note: "OpenAI 兼容接口",
  },
  {
    id: "qwen",
    name: "通义千问",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-mt-lite",
    caps: ["vision", "translate"],
    temperature: 0.2,
    maxTokens: 1024,
    note: "Qwen-MT 专用翻译模型",
  },
  {
    id: "ollama",
    name: "Ollama（本地）",
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "qwen2.5:7b",
    caps: [],
    temperature: 0.2,
    maxTokens: 1024,
    note: "本机 Ollama 服务，无需 API Key",
  },
  {
    id: "custom",
    name: "自定义（OpenAI 兼容）",
    baseUrl: "https://",
    model: "",
    caps: [],
    temperature: 0.2,
    maxTokens: 1024,
    note: "任意 OpenAI 兼容端点，仅允许 https 或 127.0.0.1",
  },
];

/** 失败降级动作的中文标签（契约 §8.2 的 fallback 值） */
export const FALLBACK_LABELS: Record<string, string> = {
  retry: "重试",
  wgc: "改用 WGC 捕获",
  browser_search: "改用浏览器搜索",
  image_path: "改用图片识别",
  local_translate: "改用本地翻译",
  cloud_translate: "改用云端翻译",
  open_settings: "打开设置",
  change_hotkey: "更换热键",
  pause_hotkey: "暂停热键",
  download_model: "下载模型",
  light_model: "换用轻量模型",
  mirror: "切换镜像源",
  manual_download: "手动下载",
  redownload: "重新下载",
  reset_group: "恢复该分组默认值",
};

export function fallbackLabel(value: string): string {
  return FALLBACK_LABELS[value] ?? value;
}
