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
  box: RectPhys;
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
export type UpdateChannel = "stable" | "preview";
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
}

export interface OcrPrefs {
  engine: "rapidocr";
  detMaxEdge: number;
  textThreshold: number;
  preserveBoxes: boolean;
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
  updateChannel: UpdateChannel;
  telemetry: boolean;
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
  "queued" | "downloading" | "verifying" | "installed" | "failed" | "cancelled";

export interface ModelInfo {
  id: string;
  name: string;
  kind: "gguf" | "ct2" | "onnx";
  sizeBytes: number;
  sha256: string;
  license: string;
  minRamMB: number;
  urls: string[];
  params: Record<string, string | number | boolean>;
  installed: boolean;
  installedBytes: number;
  downloadState: DownloadState;
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
}

export type UpdateState = "idle" | "checking" | "available" | "downloading" | "ready" | "error";

export interface UpdateStatusPayload {
  state: UpdateState;
  version?: string;
  error?: string;
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
  urls: string[];
  params: Record<string, string | number | boolean>;
}

export interface ModelsManifest {
  version: number;
  verified: boolean;
  models: ModelManifestEntry[];
}
