/** 设置页与引导共用的枚举/预设（本地常量，不改契约） */

import type {
  ActionId,
  Cap,
  DownloadState,
  GpuBackend,
  GpuProbeResult,
  Hotkeys,
  LocalBackend,
  LocalDevice,
  LocalEngine,
  ProviderKind,
} from "@/shared/contract/types";

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

/* ─────────────── 本地 GPU 推理（契约 v4）：计算设备 / 后端组件 ─────────────── */

/** 计算设备选项（`local.device`）。 */
export const DEVICE_OPTIONS: { value: LocalDevice; label: string; description?: string }[] = [
  { value: "auto", label: "自动（推荐）", description: "优先 GPU，不可用时自动回退 CPU" },
  { value: "cpu", label: "CPU", description: "省电、低发热；速度较慢" },
  { value: "gpu", label: "GPU", description: "显卡加速；显存不足/无可用后端时仍回退 CPU" },
];

/**
 * GPU 后端选项（`local.gpuBackend`）。
 * `auto` 的解析顺序是 Vulkan → CUDA → CPU；**Vulkan 支持 AMD / Intel / NVIDIA**，
 * CUDA 仅 NVIDIA 可选组件。
 */
export const GPU_BACKEND_OPTIONS: {
  value: GpuBackend;
  label: string;
  description?: string;
}[] = [
  {
    value: "auto",
    label: "自动（Vulkan 优先）",
    description: "Vulkan → CUDA → CPU",
  },
  {
    value: "vulkan",
    label: "Vulkan（AMD / Intel / NVIDIA 通用，推荐）",
    description: "跨厂商，无需额外运行时 DLL",
  },
  {
    value: "cuda",
    label: "CUDA（NVIDIA 可选组件）",
    description: "仅 NVIDIA 显卡；需下载 CUDA 组件",
  },
];

/** 后端组件下载状态文案（复用 `download:state`）。 */
export const BACKEND_STATE_LABELS: Record<DownloadState, string> = {
  queued: "排队中",
  downloading: "下载中",
  verifying: "校验中",
  installed: "已安装",
  failed: "下载失败",
  cancelled: "已取消",
};

/** 实际后端的中文名（含 CPU 回退）。 */
export const LOCAL_BACKEND_LABELS: Record<LocalBackend, string> = {
  vulkan: "Vulkan",
  cuda: "CUDA",
  cpu: "CPU",
};

/** 计算设备的中文名。 */
export const DEVICE_LABELS: Record<LocalDevice, string> = {
  auto: "自动（推荐）",
  cpu: "CPU",
  gpu: "GPU",
};

/** GPU 后端配置值的中文名。 */
export const GPU_BACKEND_LABELS: Record<GpuBackend, string> = {
  auto: "自动（Vulkan 优先）",
  cuda: "CUDA",
  vulkan: "Vulkan",
};

/** MiB → 人类可读（≥1024 MiB 用 GB，保留 1 位小数）。 */
export function formatMiB(mib: number): string {
  if (!Number.isFinite(mib) || mib <= 0) return "0 MB";
  if (mib >= 1024) return `${(mib / 1024).toFixed(1)} GB`;
  return `${Math.round(mib)} MB`;
}

/**
 * GPU 探测结果 → 设置页「设备状态」文案。
 *
 * 可用时形如「已检测：AMD Radeon RX 7800 XT（显存 16.0 GB，空闲 14.8 GB，Vulkan 可用，推荐 Vulkan）」；
 * 不可用时直接使用后端给出的可操作原因（如「未检测到可用 GPU 后端，将使用 CPU」）。
 */
export function describeGpuProbe(probe: GpuProbeResult): string {
  const recommended = probe.recommended;
  if (!probe.useable || !recommended) {
    return probe.issue ?? "未检测到可用 GPU，将使用 CPU";
  }
  const backend: LocalBackend = recommended === "cuda" ? "cuda" : "vulkan";
  const device = probe.devices.find((item) => item.backend === backend) ?? probe.devices[0];
  if (!device) {
    return probe.issue ?? "未检测到可用 GPU，将使用 CPU";
  }
  const vendor =
    device.vendor === "amd"
      ? "AMD"
      : device.vendor === "intel"
        ? "Intel"
        : device.vendor === "nvidia"
          ? "NVIDIA"
          : "";
  const name =
    vendor && !device.name.toLowerCase().includes(vendor.toLowerCase())
      ? `${vendor} ${device.name}`
      : device.name;
  const budget = probe.budget
    ? `；当前模型需 ${formatMiB(probe.budget.requiredMb)} 显存，${probe.budget.fits ? "显存充足" : "显存可能不足"}`
    : "";
  return `已检测：${name}（显存 ${formatMiB(device.vramTotalMb)}，空闲 ${formatMiB(
    device.vramFreeMb,
  )}，${LOCAL_BACKEND_LABELS[backend]} 可用，推荐 ${LOCAL_BACKEND_LABELS[backend]}${budget}）`;
}

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
  kind: ProviderKind;
  caps: Cap[];
  temperature: number;
  maxTokens: number;
  note: string;
}

/**
 * 供应商预设（契约 v5 §3.1：三条，顺序固定，与 Rust 默认 provider 一致）。
 *
 * - `deepseek`：OpenAI 兼容，默认启用；Chat + Search/Fetch（DeepSeek 官方工具 API）；
 * - `dashscope`：`kind: "qwenMt"`，DashScope Qwen-MT **专用翻译**原生端点（默认停用，填 Key 后启用）；
 * - `generic-openai`：通用 OpenAI 兼容（自定义 Base URL，默认停用）。
 */
export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com",
    model: "deepseek-flash",
    kind: "openAiCompatible",
    caps: ["vision", "search", "thinking"],
    temperature: 0.2,
    maxTokens: 1024,
    note: "默认推荐：Chat + 真实联网搜索（Search/Fetch API），支持视觉识别",
  },
  {
    id: "dashscope",
    name: "DashScope（Qwen-MT 翻译）",
    baseUrl: "https://dashscope.aliyuncs.com",
    model: "qwen-mt-lite",
    kind: "qwenMt",
    caps: ["translate"],
    temperature: 0.2,
    maxTokens: 1024,
    note: "专用翻译 API（DashScope 原生 translation_options），仅用于翻译；需填写 DashScope Key",
  },
  {
    id: "generic-openai",
    name: "通用 OpenAI 兼容",
    baseUrl: "https://api.openai.com",
    model: "gpt-4o-mini",
    kind: "openAiCompatible",
    caps: ["vision", "search", "thinking", "translate"],
    temperature: 0.2,
    maxTokens: 1024,
    note: "任意 OpenAI 兼容端点（可改 Base URL / 模型），仅允许 https 或 127.0.0.1",
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
