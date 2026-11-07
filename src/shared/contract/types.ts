/**
 * 接口契约 v1 的 TypeScript 侧（逐字对应 `docs/接口契约.md`）。
 *
 * 该文件是前端唯一的事实来源：字段名、可选性、枚举取值不得随实现随意改动。
 * Rust 侧对应 `src-tauri/src/contract/`，两端必须逐字段一致。
 */

/* ───────────────────────── §3 基础类型 ───────────────────────── */

export type SessionId = string;
export type TaskId = string;
export type DownloadId = string;

/** 动作模式：`quick` 快速 / `deep` 深度 */
export type Mode = "quick" | "deep";
export type ActionId = "search" | "imageSearch" | "translate" | "answer" | "copy";
export type ActionTarget = "text" | "image" | "both";
export type EngineId = string;

/** 物理像素矩形，原点为虚拟桌面左上角（可为负） */
export interface RectPhys {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface RectLogical {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface VirtualDesktop {
  rect: RectPhys;
  dpr: number;
}

export interface OcrBox {
  text: string;
  confidence: number;
  /** 文字框（物理像素）。字段名与 Rust `OcrBox::bbox` 一致。 */
  bbox: RectPhys;
}

export type WindowLabel = "main" | "overlay" | "result";

/* ───────────────────────── §4 配置模型 ───────────────────────── */

export type Language = "zh-CN" | "en-US";
export type Theme = "system" | "light" | "dark";
export type FontSize = "sm" | "md" | "lg";
export type ResultPosition = "follow" | "fixed";
export type SearchMode = "browser" | "ai";
export type TranslateBackend = "cloud" | "local";
export type LocalEngine = "llama.cpp" | "ollama" | "ct2";
/** 计算设备（契约 v4）。`auto` = 优先 GPU，不可用时自动回退 CPU。 */
export type LocalDevice = "auto" | "cpu" | "gpu";
/** GPU 后端（契约 v4）。`auto` 的解析顺序为 Vulkan → CUDA → CPU（Vulkan 跨厂商，AMD 可用）。 */
export type GpuBackend = "auto" | "vulkan" | "cuda";
/** 实际使用的推理后端（含 CPU；配置里的 gpuBackend 不含 cpu，因为 CPU 是回退结果）。 */
export type LocalBackend = "vulkan" | "cuda" | "cpu";
/** 供应商 API 类型（契约 v5）：OpenAI 兼容 Chat Completions / DashScope Qwen-MT 翻译 API。 */
export type ProviderKind = "openAiCompatible" | "qwenMt";
export type Cap = "vision" | "search" | "thinking" | "translate";

export interface AppPrefs {
  language: Language;
  autostart: boolean;
  singleInstance: boolean;
  startMinimized: boolean;
}

export interface ResultWindowPrefs {
  position: ResultPosition;
  pin: boolean;
  opacity: number;
  width: number;
  height: number;
}

export interface UiPrefs {
  theme: Theme;
  accent: string;
  fontSize: FontSize;
  resultWindow: ResultWindowPrefs;
}

export interface Hotkeys {
  capture: string;
  wordGrab: string;
  quickTranslate: string;
  imageSearch: string;
  translate: string;
  answer: string;
  toggleMode: string;
  togglePause: string;
  openSettings: string;
}

export interface MenuPrefs {
  text: ActionId[];
  image: ActionId[];
}

export interface ActionPrefs {
  menu: MenuPrefs;
  defaultAction: ActionId | null;
  defaultMode: Mode;
  autoCopy: boolean;
}

export interface Provider {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  /** API 类型（契约 v5；默认 OpenAI 兼容）。 */
  kind: ProviderKind;
  caps: Cap[];
  temperature: number;
  maxTokens: number;
  enabled: boolean;
}

/** `ConfigView.providers[]`：仅暴露掩码与是否已配置密钥 */
export interface ProviderView extends Provider {
  hasKey: boolean;
  keyMask: string | null;
}

export interface DeepBudget {
  search: number;
  fetch: number;
  maxTokens: number;
}

export interface ModesPrefs {
  search: Mode;
  imageSearch: Mode;
  translate: Mode;
  answer: Mode;
  deepBudget: DeepBudget;
}

export interface SearchPrefs {
  mode: SearchMode;
  engine: string;
  engineUrlTemplate: string | null;
}

export interface GlossaryEntry {
  from: string;
  to: string;
}

export interface TranslatePrefs {
  backend: TranslateBackend;
  providerId: string | null;
  model: string;
  targetLang: string;
  glossary: GlossaryEntry[];
}

export interface LocalPrefs {
  engine: LocalEngine;
  model: string;
  port: number;
  idleUnloadSec: number;
  /** 计算设备；`auto` 优先 GPU、不可用回退 CPU。 */
  device: LocalDevice;
  /** GPU 后端；`auto` = Vulkan 优先（AMD/Intel/NVIDIA 通用），CUDA 为可选组件。 */
  gpuBackend: GpuBackend;
  /** offload 到 GPU 的层数；99 = 全部，0 = 不 offload。 */
  gpuLayers: number;
}

/** OCR 识别语言（契约 v7 增量）：中文（含英文）/ 俄语 / 日语 / 韩语。 */
export type OcrLanguage = "zh" | "ru" | "ja" | "ko";

export interface OcrPrefs {
  engine: "rapidocr";
  detMaxEdge: number;
  textThreshold: number;
  preserveBoxes: boolean;
  /** 识别语言；非 `zh` 需要对应语言包（按需下载）。 */
  language: OcrLanguage;
}

export interface PrivacyPrefs {
  allowTextUpload: boolean;
  allowImageUpload: boolean;
  localOnly: boolean;
}

export interface HistoryPrefs {
  enabled: boolean;
  maxEntries: number;
  retentionDays: number;
}

export interface AdvancedPrefs {
  proxy: string | null;
  /** 开发者配置：骨架模式（不在用户界面出现，默认关闭）。 */
  skeletonMode: boolean;
}

/** 持久化配置（不含密钥） */
export interface Config {
  schemaVersion: number;
  app: AppPrefs;
  ui: UiPrefs;
  hotkeys: Hotkeys;
  actions: ActionPrefs;
  providers: Provider[];
  modes: ModesPrefs;
  search: SearchPrefs;
  translate: TranslatePrefs;
  local: LocalPrefs;
  ocr: OcrPrefs;
  privacy: PrivacyPrefs;
  history: HistoryPrefs;
  advanced: AdvancedPrefs;
}

/** `config_get` / `config_patch` / `config_import` / `config_reset` 的返回：providers 换为带密钥状态视图 */
export interface ConfigView extends Omit<Config, "providers"> {
  providers: ProviderView[];
}

/**
 * 递归可选（对象与数组）：`config_patch` 的入参形状。
 * 数组整体替换（不是逐项合并），与后端深合并语义一致。
 */
export type DeepPartial<T> = T extends readonly (infer U)[]
  ? DeepPartial<U>[]
  : T extends object
    ? { [K in keyof T]?: DeepPartial<T[K]> }
    : T;

/* ─────────────────── §5 会话 / 选区 / 动作 / 任务类型 ─────────────────── */

export type SessionPhase =
  "idle" | "freezing" | "selecting" | "menu" | "executing" | "showingResult" | "error";

export interface FrozenFrameMeta {
  width: number;
  height: number;
  dataUrl: string | null;
}

export interface SelectionResult {
  session: SessionId;
  rectPhys: RectPhys;
  hasText: boolean;
  preview: string;
  boxes: OcrBox[];
}

export interface ActionItem {
  id: ActionId;
  label: string;
  target: ActionTarget;
  mode: Mode;
  enabled: boolean;
  disabledReason: string | null;
  shortcut: string | null;
}

export interface RunActionOptions {
  text?: string | null;
  targetLang?: string | null;
  /** 覆盖式翻译开关（`translate_cover` 使用）。 */
  cover?: boolean | null;
}

export type StreamKind = "text" | "reasoning" | "meta";

export interface TokenUsage {
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  costEstimate: number;
  currency: "CNY";
}

export interface SourceRef {
  title: string;
  url: string;
  siteName: string | null;
  snippet: string | null;
}

export interface TaskStartedPayload {
  task: TaskId;
  session: SessionId;
  action: ActionId;
  mode: Mode;
  engine: EngineId;
  /** 父任务：追问/重答指向上一轮任务；首轮为 null（契约 v4）。 */
  parent?: TaskId | null;
}

export interface TaskDonePayload {
  task: TaskId;
  usage: TokenUsage;
  latencyMs: number;
  engine: EngineId;
  sources: SourceRef[];
}

/* ───────────────────── §6 补充结构（命令入参/出参） ───────────────────── */

export interface ProviderDraft {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  /** API 类型（可选：缺省 = OpenAI 兼容）。 */
  kind?: ProviderKind;
  caps: Cap[];
  apiKey: string | null;
  temperature?: number;
  maxTokens?: number;
}

export interface TestResult {
  ok: boolean;
  latencyMs: number | null;
  model: string | null;
  status: number | null;
  error: AppError | null;
}

export type DownloadState =
  "idle" | "queued" | "downloading" | "verifying" | "installed" | "failed" | "cancelled";

export interface ModelInfo {
  id: string;
  name: string;
  kind: "gguf" | "ct2" | "onnx";
  sizeBytes: number;
  sha256: string;
  license: string;
  minRamMB: number;
  /** 推理时显存预算（MB，契约 v4）。 */
  vramMB: number;
  urls: string[];
  params: Record<string, string | number | boolean>;
  installed: boolean;
  installedBytes: number;
  downloadState: DownloadState;
}

/** 本地推理后端组件（`backends_list` 元素，契约 v4）。 */
export interface BackendInfo {
  /** `"vulkan"` / `"cuda"` / `"cpu"` */
  id: string;
  label: string;
  backend: LocalBackend;
  version: string;
  sizeBytes: number;
  sha256: string;
  license: string;
  urls: string[];
  /** CUDA 为 `"nvidia"`；Vulkan/CPU 为 null。 */
  requiresVendor: string | null;
  installed: boolean;
  installedBytes: number;
  downloadState: DownloadState;
}

export interface GpuDevice {
  index: number;
  name: string;
  backend: LocalBackend;
  vramTotalMb: number;
  vramFreeMb: number;
  vendor: string | null;
  virtualAdapter: boolean;
}

export interface GpuBackendStatus {
  backend: LocalBackend;
  installed: boolean;
  available: boolean;
  exePath: string | null;
  version: string | null;
  reason: string | null;
}

export interface GpuBudget {
  requiredMb: number;
  availableMb: number;
  fits: boolean;
}

/** `local_gpu_probe` 返回（契约 v4）。 */
export interface GpuProbeResult {
  checkedAt: string;
  cached: boolean;
  useable: boolean;
  recommended: GpuBackend | null;
  devices: GpuDevice[];
  backends: GpuBackendStatus[];
  budget: GpuBudget | null;
  issue: string | null;
}

/* ─────────────────── 覆盖式翻译（契约 v4，研究 §2） ─────────────────── */

export interface CoverBlock {
  id: string;
  text: string;
  bbox: RectPhys;
  confidence?: number;
}

export interface CoveredBlock {
  id: string;
  source: string;
  translation: string;
  failed: boolean;
  engine: string;
}

export interface CoverResult {
  session: SessionId;
  blocks: CoveredBlock[];
  engine: string;
  latencyMs: number;
  totalBlocks: number;
  translatedBlocks: number;
}

export interface HotkeyStatus {
  spec: string;
  valid: boolean;
  available: boolean;
  normalized: string | null;
  conflictWith: string | null;
}

export type EngineStatus = "stopped" | "starting" | "ready" | "failed";

export interface Stats {
  version: string;
  contractVersion: number;
  /** 版本类型：`full`（内置 OCR 模型版）/ `slim`（安装后下载版）/ `dev`（开发构建）。 */
  edition: string;
  /** OCR 是否就绪（运行时 + 模型）。 */
  ocrReady: boolean;
  uptimeSec: number;
  memoryMb: number;
  activeSession: SessionId | null;
  activeTasks: number;
  localEngine: EngineStatus;
  modelsInstalled: number;
  skeletonMode: boolean;
}

/* ───────────────────────── §7 事件载荷 ───────────────────────── */

export interface OverlayReadyPayload {
  session: SessionId;
  virtualRect: RectPhys;
  dpr: number;
  frame: FrozenFrameMeta;
}

export interface OverlaySelectionPayload {
  session: SessionId;
  rectPhys: RectPhys;
}

export type OcrState = "scanning" | "done" | "failed";

export interface OcrStatusPayload {
  session: SessionId;
  state: OcrState;
  boxes?: OcrBox[];
}

export interface MenuReadyPayload {
  session: SessionId;
  items: ActionItem[];
  hasText: boolean;
  preview: string;
}

export interface TaskDeltaPayload {
  task: TaskId;
  kind: StreamKind;
  chunk: string;
}

export interface TaskErrorPayload {
  task: TaskId;
  code: string;
  message: string;
  retryable: boolean;
  fallback: string[];
}

export interface DownloadProgressPayload {
  model: string;
  loaded: number;
  total: number;
  speedBps: number;
}

export interface DownloadStatePayload {
  model: string;
  state: DownloadState;
}

export interface ConfigChangedPayload {
  config: ConfigView;
  changedKeys: string[];
}

/** 托盘菜单动作：`openSettings` / `rerunOnboarding` / `togglePause` / `quit` / `capture` */
export interface TrayActionPayload {
  id: string;
}

export interface EngineStatusPayload {
  local: EngineStatus;
  /** 实际使用的计算设备。 */
  device?: LocalDevice | null;
  /** 实际使用的后端（含 CPU）。 */
  backend?: LocalBackend | null;
  /** 回退原因（如「Vulkan 后端未安装，已回退 CPU」）。 */
  fallbackReason?: string | null;
  /** 实际 offload 到 GPU 的层数。 */
  offloadLayers?: number | null;
  /** 设备名（如 `AMD Radeon RX 7800 XT`）。 */
  deviceName?: string | null;
  /** 监听端口。 */
  port?: number | null;
}

export interface TranslateProgressPayload {
  session: SessionId;
  done: number;
  total: number;
}

export interface TranslateReadyPayload {
  session: SessionId;
  blocks: CoveredBlock[];
  engine: string;
}


/* ─────────────────── 自定义 / OCR 状态 / 历史线程（契约 v5） ─────────────────── */

/** `open_path` 的白名单目录类型。 */
export type OpenPathKind =
  "config" | "prompts" | "models" | "logs" | "backends" | "data" | "docs";

/** OCR 资产状态（`ocr_status`）。 */
export interface OcrAssetStatus {
  id: string;
  name: string;
  installed: boolean;
  bundled: boolean;
  sizeBytes: number;
  sha256: string;
}

/** 一种识别语言的语言包安装状态（`ocr_status` 的 languages[] 元素）。 */
export interface OcrLanguageStatus {
  language: OcrLanguage;
  /** 语言包文件是否存在（`zh` 恒为 true：使用基础 rec/dict 资产）。 */
  installed: boolean;
}

/** `ocr_status` 返回（契约 v5；契约 v7 增量增加 `language` / `languages`）。 */
export interface OcrStatus {
  ready: boolean;
  bundled: boolean;
  runtimeReady: boolean;
  runtimePath: string | null;
  assets: OcrAssetStatus[];
  missing: string[];
  message: string | null;
  /** 当前识别语言。 */
  language: OcrLanguage;
  /** 各识别语言的语言包状态（固定 4 项，含 zh）。 */
  languages: OcrLanguageStatus[];
}

/** 历史会话摘要（`history_list`）。 */
export interface HistorySession {
  session: SessionId;
  startedAt: number;
  title: string;
  action: ActionId | null;
  turns: number;
  engine: string | null;
}

/** 历史问答轮次（`history_thread`）。 */
export interface HistoryTurn {
  task: TaskId;
  action: ActionId;
  mode: Mode;
  question: string;
  answer: string;
  engine: string | null;
  status: "done" | "error" | "cancelled";
  createdAt: number;
}

/** `history_thread` 返回（契约 v5）。 */
export interface HistoryThread {
  session: HistorySession;
  turns: HistoryTurn[];
}

/* ───────────────────────── §8 错误模型 ───────────────────────── */

export interface AppError {
  /** "SA-3002" */
  code: string;
  /** 面向用户的中文短句 */
  message: string;
  /** 技术细节（脱敏） */
  detail: string | null;
  retryable: boolean;
  /** 可尝试的降级动作，如 ["browser_search"] */
  fallback: string[];
  context: Record<string, unknown> | null;
}

/* ───────────────────────── §9 首启引导 ───────────────────────── */

export interface OnboardingDraft {
  language: Language;
  ai: { providerId: string; baseUrl: string; model: string; tested: boolean } | null;
  translate: { backend: TranslateBackend; modelId: string | null; cloudModel: string } | null;
  modes: Partial<Record<"search" | "imageSearch" | "translate" | "answer", Mode>>;
  ui: { theme: Theme; fontSize: FontSize; autostart: boolean; resultPosition: ResultPosition };
  hotkeys: { capture: string };
  privacy: { agreed: boolean; allowTextUpload: boolean; allowImageUpload: boolean };
  finishedAt: string | null;
}

export interface OnboardingState {
  completed: boolean;
  skipped: boolean;
  /** 当前 1 */
  version: number;
  /** 0..5；completed 后保留最后位置 */
  currentStep: number;
  /** ISO8601 */
  startedAt: string;
  /** ISO8601 */
  updatedAt: string;
  data: OnboardingDraft;
}

export type OnboardingStepId = "welcome" | "ai" | "translate" | "modes" | "appearance" | "privacy";

/* ────────────── §10 内置资源（models.manifest.json 静态条目） ────────────── */

/** 清单文件中的模型静态部分（无 installed 等运行时字段） */
export interface ModelManifestEntry {
  id: string;
  name: string;
  kind: "gguf" | "ct2" | "onnx";
  sizeBytes: number;
  sha256: string;
  license: string;
  minRamMB: number;
  /** 推理时显存预算（MB，契约 v4）。 */
  vramMB: number;
  urls: string[];
  params: Record<string, string | number | boolean>;
}

export interface ModelsManifest {
  version: number;
  verified: boolean;
  models: ModelManifestEntry[];
}
