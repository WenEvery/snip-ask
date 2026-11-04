/**
 * 契约默认值（与 `docs/接口契约.md` §4.1 / §9 一致）。
 *
 * 用途：mock 后端初值、表单缺省回退、`config_reset` 目标状态。
 * 注意：默认 `Config` **不含**密钥；`ConfigView` 的 providers 带 `hasKey: false` / `keyMask: null`。
 */

import type {
  Config,
  ConfigView,
  Hotkeys,
  OnboardingDraft,
  OnboardingState,
  ProviderView,
} from "./types";

/** 深拷贝：常量导出的对象不被调用方意外修改 */
function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

/** 契约 §4.1 的默认热键（仅 `capture` 有默认绑定） */
export const defaultHotkeys: Hotkeys = {
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

/** 契约 §4.1：首次运行写入磁盘的配置 */
export const defaultConfig: Config = {
  schemaVersion: 3,
  app: {
    language: "zh-CN",
    autostart: false,
    singleInstance: true,
    startMinimized: true,
  },
  ui: {
    theme: "system",
    accent: "blue",
    fontSize: "md",
    resultWindow: {
      position: "follow",
      pin: true,
      opacity: 1.0,
      width: 460,
      height: 420,
    },
  },
  hotkeys: defaultHotkeys,
  actions: {
    menu: {
      text: ["search", "translate", "answer", "copy"],
      image: ["imageSearch", "answer", "translate"],
    },
    defaultAction: null,
    defaultMode: "quick",
    autoCopy: false,
  },
  providers: [
    {
      id: "deepseek",
      name: "DeepSeek",
      baseUrl: "https://api.deepseek.com",
      model: "deepseek-flash",
      caps: ["vision", "search", "thinking"],
      temperature: 0.2,
      maxTokens: 1024,
      enabled: true,
    },
  ],
  modes: {
    search: "quick",
    imageSearch: "quick",
    translate: "quick",
    answer: "quick",
    deepBudget: { search: 3, fetch: 2, maxTokens: 1500 },
  },
  search: { mode: "browser", engine: "bing", engineUrlTemplate: null },
  translate: {
    backend: "cloud",
    providerId: "deepseek",
    model: "qwen-mt-lite",
    targetLang: "zh-CHS",
    glossary: [],
  },
  local: {
    engine: "llama.cpp",
    model: "hy-mt-1.8b-q4",
    port: 39281,
    idleUnloadSec: 600,
  },
  ocr: {
    engine: "rapidocr",
    detMaxEdge: 3200,
    textThreshold: 0.6,
    preserveBoxes: true,
  },
  privacy: {
    allowTextUpload: true,
    allowImageUpload: true,
    localOnly: false,
  },
  history: { enabled: true, maxEntries: 1000, retentionDays: 90 },
  advanced: {
    proxy: null,
    updateChannel: "stable",
    telemetry: false,
    skeletonMode: true,
  },
};

/** 默认 `ConfigView`：providers 只有掩码状态（未配置密钥） */
export const defaultProviderViews: ProviderView[] = defaultConfig.providers.map((provider) => ({
  ...provider,
  hasKey: false,
  keyMask: null,
}));

export const defaultConfigView: ConfigView = {
  ...clone(defaultConfig),
  providers: clone(defaultProviderViews),
};

/** 返回默认 `Config` 的深拷贝（避免调用方污染常量） */
export function createDefaultConfig(): Config {
  return clone(defaultConfig);
}

/** 返回默认 `ConfigView` 的深拷贝 */
export function createDefaultConfigView(): ConfigView {
  return clone(defaultConfigView);
}

/** 默认引导草稿（步骤 0 的初始状态） */
export function createDefaultOnboardingDraft(): OnboardingDraft {
  return {
    language: "zh-CN",
    ai: null,
    translate: null,
    modes: {},
    ui: {
      theme: "system",
      fontSize: "md",
      autostart: false,
      resultPosition: "follow",
    },
    hotkeys: { capture: "Ctrl+Q" },
    privacy: {
      agreed: false,
      allowTextUpload: true,
      allowImageUpload: true,
    },
    finishedAt: null,
  };
}

/** 默认引导状态：未完成、未跳过、停在第 0 步 */
export function createDefaultOnboardingState(): OnboardingState {
  const now = new Date().toISOString();
  return {
    completed: false,
    skipped: false,
    version: 1,
    currentStep: 0,
    startedAt: now,
    updatedAt: now,
    data: createDefaultOnboardingDraft(),
  };
}

/** 常量形态的默认引导状态（时间戳取模块加载时刻；需要新鲜时间戳请用工厂函数） */
export const defaultOnboardingState: OnboardingState = createDefaultOnboardingState();

/** 动作菜单可用动作的中文标签（overlay / 设置页共用） */
export const ACTION_LABELS = {
  search: "AI 搜索",
  imageSearch: "图片搜索",
  translate: "翻译",
  answer: "解答",
  copy: "复制",
} as const;
