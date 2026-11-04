/**
 * 浏览器 mock 后端（契约 §12 骨架模式 + §13）。
 *
 * 目标：**无 Rust 环境**也能在浏览器里完整走通
 * 「capture_freeze → overlay:ready → 框选 → menu:ready → menu_execute → task.* 流式」。
 *
 * 行为约定：
 * - 配置持久化：`localStorage["snipask.mock.config"]`（深合并默认值）；密钥只存内存；
 * - 引导状态：`localStorage["snipask.mock.onboarding"]`，初始 `completed=false`；
 * - 所有文案前缀「【骨架模式】」；
 * - 事件与真实后端同形（`EVENTS` 常量），跨标签页通过 BroadcastChannel 同步，
 *   便于同时打开 `?window=overlay` 与 `?window=result` 手动验收；
 * - 命令失败统一 `Promise.reject(AppError)`。
 */

import { COMMANDS, EVENTS, IPC_CONTRACT_VERSION } from "../contract/names";
import type { CommandName, EventName } from "../contract/names";
import {
  ACTION_LABELS,
  createDefaultConfig,
  createDefaultOnboardingState,
} from "../contract/defaults";
import type {
  ActionId,
  ActionItem,
  ActionTarget,
  AppError,
  Config,
  ConfigChangedPayload,
  ConfigView,
  DeepPartial,
  DownloadProgressPayload,
  DownloadState,
  DownloadStatePayload,
  HotkeyStatus,
  MenuReadyPayload,
  Mode,
  ModelInfo,
  OcrStatusPayload,
  OnboardingDraft,
  OnboardingState,
  OverlayReadyPayload,
  ProviderDraft,
  RectPhys,
  SelectionResult,
  SessionId,
  SessionPhase,
  Stats,
  TaskDeltaPayload,
  TaskDonePayload,
  TaskId,
  TaskStartedPayload,
  TestResult,
} from "../contract/types";
import type { IpcBackend } from "./index";
import {
  createPlaceholderModels,
  SKELETON_ANSWER,
  SKELETON_FOLLOWUP_ANSWER,
  SKELETON_OCR_PREVIEW,
  SKELETON_SOURCES,
  splitChunks,
} from "./mockData";

/* ───────────────────────── 工具 ───────────────────────── */

const CONFIG_STORAGE_KEY = "snipask.mock.config";
const ONBOARDING_STORAGE_KEY = "snipask.mock.onboarding";

function storageGet(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function storageSet(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    /* 隐私模式 / 配额不足时静默降级 */
  }
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function uuid(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  const bytes = new Uint8Array(16);
  if (typeof crypto !== "undefined" && typeof crypto.getRandomValues === "function") {
    crypto.getRandomValues(bytes);
  } else {
    for (let i = 0; i < bytes.length; i += 1) bytes[i] = Math.floor(Math.random() * 256);
  }
  bytes[6] = ((bytes[6] ?? 0) & 0x0f) | 0x40;
  bytes[8] = ((bytes[8] ?? 0) & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function appError(
  code: string,
  message: string,
  detail: string | null = null,
  fallback: string[] = [],
): AppError {
  return { code, message, detail, retryable: false, fallback, context: null };
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 深合并：对象递归、数组整体替换、undefined 跳过（与后端 `config_patch` 一致） */
function deepMerge<T>(base: T, patch: unknown): T {
  if (!isPlainObject(base) || !isPlainObject(patch)) {
    return patch === undefined ? base : (patch as T);
  }
  const merged: Record<string, unknown> = { ...base };
  for (const [key, value] of Object.entries(patch)) {
    if (value === undefined) continue;
    merged[key] =
      isPlainObject(value) && isPlainObject(merged[key]) ? deepMerge(merged[key], value) : value;
  }
  return merged as T;
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

/** 契约 §4.3：热键非空必须匹配 `(Ctrl|Alt|Shift|Win)(+…)*+[A-Z0-9]`（大小写不敏感） */
const MODIFIER_ORDER = ["Ctrl", "Alt", "Shift", "Win"] as const;

function normalizeHotkey(spec: string): HotkeyStatus {
  const trimmed = spec.trim();
  if (trimmed === "")
    return { spec, valid: true, available: true, normalized: "", conflictWith: null };
  const modifiers = new Set<string>();
  let key: string | null = null;
  for (const rawPart of trimmed.split("+")) {
    const part = rawPart.trim();
    const modifier = MODIFIER_ORDER.find((item) => item.toLowerCase() === part.toLowerCase());
    if (modifier) {
      modifiers.add(modifier);
      continue;
    }
    if (key === null && /^[a-z0-9]$/i.test(part)) {
      key = part.toUpperCase();
      continue;
    }
    return { spec, valid: false, available: false, normalized: null, conflictWith: null };
  }
  if (key === null || modifiers.size === 0) {
    return { spec, valid: false, available: false, normalized: null, conflictWith: null };
  }
  const ordered = MODIFIER_ORDER.filter((modifier) => modifiers.has(modifier));
  return {
    spec,
    valid: true,
    available: true,
    normalized: [...ordered, key].join("+"),
    conflictWith: null,
  };
}

/* ───────────────────── 事件总线（含跨标签页同步） ───────────────────── */

type IpcHandler = (payload: unknown) => void;

const listeners = new Map<EventName, Set<IpcHandler>>();

function dispatchLocal(event: EventName, payload: unknown): void {
  const handlers = listeners.get(event);
  if (!handlers) return;
  for (const handler of [...handlers]) {
    try {
      handler(payload);
    } catch (error) {
      console.error(`[snipask mock] 事件处理器异常：${event}`, error);
    }
  }
}

let channel: BroadcastChannel | null = null;
try {
  const created =
    typeof BroadcastChannel === "function" ? new BroadcastChannel("snipask.mock.ipc") : null;
  channel = created;
  created?.addEventListener("message", (message: MessageEvent) => {
    const data = message.data as { event?: unknown; payload?: unknown } | null;
    if (!data || typeof data.event !== "string") return;
    dispatchLocal(data.event as EventName, data.payload);
  });
} catch {
  channel = null;
}

function emit<T>(event: EventName, payload: T): void {
  dispatchLocal(event, payload);
  try {
    channel?.postMessage({ event, payload });
  } catch {
    /* 跨标签页同步失败不影响本页流程 */
  }
}

/* ───────────────────────── 状态 ───────────────────────── */

interface MockTask {
  cancelled: boolean;
  timer: number | null;
  startedAt: number;
}

interface MockDownload {
  timer: number | null;
  loaded: number;
  state: DownloadState;
}

interface MockState {
  config: Config;
  secrets: Map<string, string>;
  onboarding: OnboardingState;
  session: SessionId | null;
  phase: SessionPhase;
  selection: SelectionResult | null;
  lastAction: ActionId | null;
  lastMode: Mode;
  tasks: Map<TaskId, MockTask>;
  downloads: Map<string, MockDownload>;
  models: ModelInfo[];
  startedAt: number;
  sessionTimers: number[];
}

function loadStoredConfig(): Config {
  const base = createDefaultConfig();
  const raw = storageGet(CONFIG_STORAGE_KEY);
  if (!raw) return base;
  try {
    const parsed: unknown = JSON.parse(raw);
    return isPlainObject(parsed) ? deepMerge(base, parsed) : base;
  } catch {
    return base;
  }
}

function loadStoredOnboarding(): OnboardingState {
  const fallback = createDefaultOnboardingState();
  const raw = storageGet(ONBOARDING_STORAGE_KEY);
  if (!raw) return fallback;
  try {
    const parsed: unknown = JSON.parse(raw);
    return isPlainObject(parsed) ? deepMerge(fallback, parsed) : fallback;
  } catch {
    return fallback;
  }
}

const state: MockState = {
  config: loadStoredConfig(),
  secrets: new Map<string, string>(),
  onboarding: loadStoredOnboarding(),
  session: null,
  phase: "idle",
  selection: null,
  lastAction: null,
  lastMode: "quick",
  tasks: new Map<TaskId, MockTask>(),
  downloads: new Map<string, MockDownload>(),
  models: createPlaceholderModels(),
  startedAt: Date.now(),
  sessionTimers: [],
};

function persistConfig(): void {
  storageSet(CONFIG_STORAGE_KEY, JSON.stringify(state.config));
}

function persistOnboarding(): void {
  storageSet(ONBOARDING_STORAGE_KEY, JSON.stringify(state.onboarding));
}

function toConfigView(): ConfigView {
  const config = clone(state.config);
  return {
    ...config,
    providers: config.providers.map((provider) => {
      const secret = state.secrets.get(provider.id) ?? null;
      return {
        ...provider,
        hasKey: secret !== null && secret.length > 0,
        keyMask: secret === null || secret.length === 0 ? null : `••••${secret.slice(-4)}`,
      };
    }),
  };
}

function emitConfigChanged(changedKeys: string[]): void {
  emit<ConfigChangedPayload>(EVENTS.configChanged, { config: toConfigView(), changedKeys });
}

/* ───────────────────── 会话 / 任务 / 下载辅助 ───────────────────── */

function clearSessionTimers(): void {
  for (const timer of state.sessionTimers) window.clearTimeout(timer);
  state.sessionTimers = [];
}

function cancelAllTasks(): void {
  for (const task of state.tasks.values()) {
    task.cancelled = true;
    if (task.timer !== null) window.clearTimeout(task.timer);
  }
  state.tasks.clear();
}

function resetSession(): void {
  clearSessionTimers();
  cancelAllTasks();
  state.session = null;
  state.selection = null;
  state.phase = "idle";
}

function notifyWindowChange(windowLabel: string, visible: boolean): void {
  console.info(`[snipask mock] window_${visible ? "show" : "hide"}(${windowLabel})`);
  if (typeof window !== "undefined" && typeof window.dispatchEvent === "function") {
    window.dispatchEvent(
      new CustomEvent("mock:window-change", { detail: { window: windowLabel, visible } }),
    );
  }
}

function requireString(
  args: Record<string, unknown> | undefined,
  key: string,
  code = "SA-1099",
): string {
  const value = args?.[key];
  if (typeof value !== "string" || value.length === 0) {
    throw appError(code, `参数 ${key} 缺失或非法`);
  }
  return value;
}

function requireBoolean(args: Record<string, unknown> | undefined, key: string): boolean {
  const value = args?.[key];
  if (typeof value !== "boolean") throw appError("SA-5001", `参数 ${key} 必须为布尔值`);
  return value;
}

/** 当前活动会话必须存在且与入参一致（selection_commit / menu_execute 等） */
function requireSession(args: Record<string, unknown> | undefined): SessionId {
  const session = requireString(args, "session");
  if (state.session === null || state.session !== session) {
    throw appError("SA-1099", "会话状态非法或已过期");
  }
  return session;
}

/** 取消/关闭类命令：会话已不存在时也允许（视为幂等） */
function requireActiveSession(args: Record<string, unknown> | undefined): SessionId {
  const session = requireString(args, "session");
  if (state.session !== null && state.session !== session) {
    throw appError("SA-1099", "会话状态非法或已过期");
  }
  return session;
}

function parseRect(value: unknown): RectPhys {
  if (!isPlainObject(value)) throw appError("SA-2001", "选区矩形非法");
  const { x, y, width, height } = value;
  if (
    typeof x !== "number" ||
    typeof y !== "number" ||
    typeof width !== "number" ||
    typeof height !== "number" ||
    width < 0 ||
    height < 0
  ) {
    throw appError("SA-2001", "选区矩形非法");
  }
  return { x, y, width, height };
}

const ACTION_IDS: ActionId[] = ["search", "imageSearch", "translate", "answer", "copy"];

function parseAction(value: unknown): ActionId {
  if (typeof value === "string" && (ACTION_IDS as string[]).includes(value)) {
    return value as ActionId;
  }
  throw appError("SA-1099", "动作非法");
}

function parseMode(value: unknown, fallback: Mode): Mode {
  return value === "quick" || value === "deep" ? value : fallback;
}

const ACTION_TARGETS: Record<ActionId, ActionTarget> = {
  search: "text",
  imageSearch: "image",
  translate: "both",
  answer: "both",
  copy: "text",
};

function actionMode(config: Config, action: ActionId): Mode {
  switch (action) {
    case "search":
      return config.modes.search;
    case "imageSearch":
      return config.modes.imageSearch;
    case "translate":
      return config.modes.translate;
    case "answer":
      return config.modes.answer;
    default:
      return config.actions.defaultMode;
  }
}

function actionShortcut(config: Config, action: ActionId): string | null {
  switch (action) {
    case "imageSearch":
      return config.hotkeys.imageSearch || null;
    case "translate":
      return config.hotkeys.translate || null;
    case "answer":
      return config.hotkeys.answer || null;
    default:
      return null;
  }
}

function buildMenuItems(config: Config, hasText: boolean): ActionItem[] {
  const list = hasText ? config.actions.menu.text : config.actions.menu.image;
  return list.map((action) => {
    const enabled = action !== "copy" || hasText;
    return {
      id: action,
      label: ACTION_LABELS[action],
      target: ACTION_TARGETS[action],
      mode: actionMode(config, action),
      enabled,
      disabledReason: enabled ? null : "未识别到文本，无法复制",
      shortcut: actionShortcut(config, action),
    };
  });
}

function emitMenuReady(session: SessionId): void {
  const hasText = state.selection?.hasText ?? true;
  emit<MenuReadyPayload>(EVENTS.menuReady, {
    session,
    items: buildMenuItems(state.config, hasText),
    hasText,
    preview: state.selection?.preview ?? SKELETON_OCR_PREVIEW,
  });
}

function buildTaskText(action: ActionId, mode: Mode, question?: string): string {
  if (question) return SKELETON_FOLLOWUP_ANSWER;
  if (action === "translate") {
    return `【骨架模式译文】${state.selection?.preview ?? SKELETON_OCR_PREVIEW}`;
  }
  if (action === "search") {
    const query = state.selection?.preview ?? SKELETON_OCR_PREVIEW;
    return mode === "deep"
      ? `【骨架模式】将打开浏览器搜索 “${query}” 并执行深度检索（占位）。${SKELETON_ANSWER}`
      : `【骨架模式】将打开浏览器搜索 “${query}”，不实际访问网络。${SKELETON_ANSWER}`;
  }
  if (action === "copy") {
    return `【骨架模式】已模拟复制识别文本到剪贴板：${SKELETON_OCR_PREVIEW}`;
  }
  return SKELETON_ANSWER;
}

/** 流式任务：task:started 之后每 25–40ms 推一块，共 8 块，最后 task:done */
function startTaskStream(
  taskId: TaskId,
  options: { action: ActionId; mode: Mode; question?: string },
): void {
  const task = state.tasks.get(taskId);
  if (!task) return;
  const chunks = splitChunks(buildTaskText(options.action, options.mode, options.question), 8);
  let index = 0;
  const push = (): void => {
    if (task.cancelled) return;
    if (index < chunks.length) {
      emit<TaskDeltaPayload>(EVENTS.taskDelta, {
        task: taskId,
        kind: "text",
        chunk: chunks[index] ?? "",
      });
      index += 1;
      task.timer = window.setTimeout(push, 25 + Math.floor(Math.random() * 16));
      return;
    }
    task.timer = null;
    state.tasks.delete(taskId);
    state.phase = "showingResult";
    const done: TaskDonePayload = {
      task: taskId,
      usage: {
        inputTokens: 120,
        outputTokens: 60,
        cachedInputTokens: 0,
        costEstimate: 0,
        currency: "CNY",
      },
      latencyMs: Math.max(1, Date.now() - task.startedAt),
      engine: "skeleton",
      sources: options.action === "imageSearch" ? clone(SKELETON_SOURCES) : [],
    };
    emit<TaskDonePayload>(EVENTS.taskDone, done);
  };
  task.timer = window.setTimeout(push, 25 + Math.floor(Math.random() * 16));
}

function startCapture(): SessionId {
  resetSession();
  const session = uuid();
  state.session = session;
  state.phase = "selecting";
  const virtualRect: RectPhys = { x: 0, y: 0, width: 1920, height: 1080 };
  const dpr = 1.25;
  state.sessionTimers.push(
    window.setTimeout(() => {
      if (state.session !== session) return;
      emit<OverlayReadyPayload>(EVENTS.overlayReady, {
        session,
        virtualRect,
        dpr,
        frame: { width: 1920, height: 1080, dataUrl: null },
      });
      state.sessionTimers.push(
        window.setTimeout(() => {
          if (state.session !== session) return;
          emit<OcrStatusPayload>(EVENTS.ocrStatus, { session, state: "scanning" });
          state.sessionTimers.push(
            window.setTimeout(() => {
              if (state.session !== session) return;
              emit<OcrStatusPayload>(EVENTS.ocrStatus, {
                session,
                state: "done",
                boxes: [{ text: SKELETON_OCR_PREVIEW, confidence: 0.98, box: virtualRect }],
              });
              emitMenuReady(session);
            }, 300),
          );
        }, 50),
      );
    }, 80),
  );
  return session;
}

function selectionCommit(args: Record<string, unknown> | undefined): SelectionResult {
  const session = requireSession(args);
  const rectPhys = parseRect(args?.rectPhys);
  const selection: SelectionResult = {
    session,
    rectPhys,
    hasText: true,
    preview: SKELETON_OCR_PREVIEW,
    boxes: [{ text: SKELETON_OCR_PREVIEW, confidence: 0.98, box: rectPhys }],
  };
  state.selection = selection;
  state.phase = "menu";
  state.sessionTimers.push(
    window.setTimeout(() => {
      if (state.session !== session) return;
      emit<OcrStatusPayload>(EVENTS.ocrStatus, { session, state: "done", boxes: selection.boxes });
      emitMenuReady(session);
    }, 150),
  );
  return selection;
}

function selectionReset(args: Record<string, unknown> | undefined): void {
  const session = requireSession(args);
  if (state.phase !== "menu" && state.phase !== "executing") {
    throw appError("SA-1099", "当前会话不在动作菜单状态");
  }
  state.selection = null;
  state.phase = "selecting";
  state.sessionTimers.push(
    window.setTimeout(() => {
      if (state.session !== session) return;
      emit<OcrStatusPayload>(EVENTS.ocrStatus, { session, state: "scanning" });
    }, 30),
  );
}

function menuExecute(args: Record<string, unknown> | undefined): TaskId {
  const session = requireSession(args);
  if (state.phase !== "menu") throw appError("SA-1099", "当前会话不在动作菜单状态");
  const action = parseAction(args?.action);
  const mode = parseMode(args?.mode, actionMode(state.config, action));
  const taskId = uuid();
  state.tasks.set(taskId, { cancelled: false, timer: null, startedAt: Date.now() });
  state.phase = "executing";
  state.lastAction = action;
  state.lastMode = mode;
  emit<TaskStartedPayload>(EVENTS.taskStarted, {
    task: taskId,
    session,
    action,
    mode,
    engine: "skeleton",
  });
  notifyWindowChange("result", true);
  startTaskStream(taskId, { action, mode });
  return taskId;
}

function taskFollowup(args: Record<string, unknown> | undefined): TaskId {
  const previous = requireString(args, "task");
  const text = requireString(args, "text");
  if (state.session === null) throw appError("SA-1099", "会话状态非法或已过期");
  if (state.phase !== "showingResult" && !state.tasks.has(previous)) {
    throw appError("SA-1099", "当前会话不在结果展示状态");
  }
  const taskId = uuid();
  state.tasks.set(taskId, { cancelled: false, timer: null, startedAt: Date.now() });
  state.phase = "executing";
  emit<TaskStartedPayload>(EVENTS.taskStarted, {
    task: taskId,
    session: state.session,
    action: state.lastAction ?? "answer",
    mode: state.lastMode,
    engine: "skeleton",
  });
  notifyWindowChange("result", true);
  startTaskStream(taskId, {
    action: state.lastAction ?? "answer",
    mode: state.lastMode,
    question: text,
  });
  return taskId;
}

function taskCancel(args: Record<string, unknown> | undefined): void {
  const taskId = requireString(args, "task");
  const task = state.tasks.get(taskId);
  if (!task) return;
  task.cancelled = true;
  if (task.timer !== null) window.clearTimeout(task.timer);
  state.tasks.delete(taskId);
}

/* ───────────────────────── 配置 / 引导 / 系统 ───────────────────────── */

const CONFIG_GROUPS = [
  "app",
  "ui",
  "hotkeys",
  "actions",
  "providers",
  "modes",
  "search",
  "translate",
  "local",
  "ocr",
  "privacy",
  "history",
  "advanced",
] as const;

type ConfigGroup = (typeof CONFIG_GROUPS)[number];

function isConfigGroup(value: string): value is ConfigGroup {
  return (CONFIG_GROUPS as readonly string[]).includes(value);
}

function onboardingConfigPatch(draft: OnboardingDraft): DeepPartial<Config> {
  return {
    app: { language: draft.language, autostart: draft.ui.autostart },
    ui: {
      theme: draft.ui.theme,
      fontSize: draft.ui.fontSize,
      resultWindow: { position: draft.ui.resultPosition },
    },
    hotkeys: { capture: draft.hotkeys.capture },
    privacy: {
      allowTextUpload: draft.privacy.allowTextUpload,
      allowImageUpload: draft.privacy.allowImageUpload,
    },
  };
}

function parseProviderDraft(value: unknown): ProviderDraft {
  if (!isPlainObject(value)) throw appError("SA-3001", "供应商草稿缺失");
  const { id, name, baseUrl, model, caps, apiKey } = value;
  if (
    typeof id !== "string" ||
    typeof name !== "string" ||
    typeof baseUrl !== "string" ||
    typeof model !== "string" ||
    !Array.isArray(caps)
  ) {
    throw appError("SA-3001", "供应商草稿字段不完整");
  }
  return {
    id,
    name,
    baseUrl,
    model,
    caps: caps.filter(
      (cap): cap is "vision" | "search" | "thinking" | "translate" =>
        cap === "vision" || cap === "search" || cap === "thinking" || cap === "translate",
    ),
    apiKey: typeof apiKey === "string" && apiKey.length > 0 ? apiKey : null,
  };
}

function findModel(id: string): ModelInfo {
  const model = state.models.find((item) => item.id === id);
  if (!model) throw appError("SA-4001", `模型不存在：${id}`, null, ["download_model"]);
  return model;
}

function appStats(): Stats {
  return {
    version: "0.1.0",
    contractVersion: IPC_CONTRACT_VERSION,
    uptimeSec: Math.max(1, Math.round((Date.now() - state.startedAt) / 1000)),
    memoryMb: 186,
    activeSession: state.session,
    activeTasks: state.tasks.size,
    localEngine: "stopped",
    modelsInstalled: state.models.filter((model) => model.installed).length,
    skeletonMode: state.config.advanced.skeletonMode,
  };
}

async function dispatch(
  command: CommandName,
  args: Record<string, unknown> = {},
): Promise<unknown> {
  switch (command) {
    case COMMANDS.captureFreeze:
      return startCapture();

    case COMMANDS.captureCancel: {
      requireActiveSession(args);
      resetSession();
      notifyWindowChange("overlay", false);
      return undefined;
    }

    case COMMANDS.selectionCommit:
      return selectionCommit(args);

    case COMMANDS.selectionReset:
      selectionReset(args);
      return undefined;

    case COMMANDS.menuExecute:
      return menuExecute(args);

    case COMMANDS.taskCancel:
      taskCancel(args);
      return undefined;

    case COMMANDS.taskFollowup:
      return taskFollowup(args);

    case COMMANDS.resultClose: {
      requireActiveSession(args);
      resetSession();
      notifyWindowChange("result", false);
      return undefined;
    }

    case COMMANDS.configGet:
      return toConfigView();

    case COMMANDS.configPatch: {
      if (!isPlainObject(args.patch)) throw appError("SA-5001", "配置补丁必须为对象");
      state.config = deepMerge(state.config, args.patch);
      persistConfig();
      emitConfigChanged(Object.keys(args.patch));
      return toConfigView();
    }

    case COMMANDS.configImport: {
      const path = typeof args.path === "string" ? args.path : "";
      if (path.length === 0) throw appError("SA-5003", "导入失败：路径为空");
      console.info(`[snipask mock] 模拟导入配置：${path}`);
      emitConfigChanged(["*"]);
      return toConfigView();
    }

    case COMMANDS.configExport: {
      if (args.includeSecrets === true) {
        throw appError("SA-5002", "骨架模式不支持导出明文密钥");
      }
      const path =
        typeof args.path === "string" && args.path.length > 0
          ? args.path
          : `exports/config-${new Date().toISOString().replace(/[:.]/g, "-")}.json`;
      console.info(`[snipask mock] 模拟导出配置：${path}`);
      return path;
    }

    case COMMANDS.configReset: {
      const scope = requireString(args, "scope", "SA-5001");
      if (scope === "all") {
        state.config = createDefaultConfig();
      } else if (isConfigGroup(scope)) {
        const defaults = createDefaultConfig();
        state.config = { ...state.config, [scope]: clone(defaults[scope]) } as Config;
      } else {
        throw appError("SA-5001", `未知配置分组：${scope}`);
      }
      persistConfig();
      emitConfigChanged([scope]);
      return toConfigView();
    }

    case COMMANDS.onboardingState:
      return clone(state.onboarding);

    case COMMANDS.onboardingUpdate: {
      const step = typeof args.step === "number" ? args.step : 0;
      const data = isPlainObject(args.data) ? args.data : {};
      state.onboarding.currentStep = Math.min(5, Math.max(0, Math.floor(step)));
      state.onboarding.updatedAt = new Date().toISOString();
      state.onboarding.data = deepMerge<OnboardingDraft>(state.onboarding.data, data);
      persistOnboarding();
      return clone(state.onboarding);
    }

    case COMMANDS.onboardingFinish: {
      const skip = requireBoolean(args, "skip");
      const now = new Date().toISOString();
      state.onboarding.completed = !skip;
      state.onboarding.skipped = skip;
      state.onboarding.updatedAt = now;
      state.onboarding.data.finishedAt = now;
      if (!skip) {
        state.config = deepMerge<Config>(
          state.config,
          onboardingConfigPatch(state.onboarding.data),
        );
        persistConfig();
      }
      persistOnboarding();
      emitConfigChanged(skip ? [] : ["app", "ui", "hotkeys", "privacy"]);
      return undefined;
    }

    case COMMANDS.providerTest: {
      const draft = parseProviderDraft(args.draft);
      if (draft.apiKey) state.secrets.set(draft.id, draft.apiKey);
      const latencyMs = 200 + Math.floor(Math.random() * 201);
      await delay(latencyMs);
      const result: TestResult = {
        ok: true,
        latencyMs,
        model: draft.model || "deepseek-flash",
        status: 200,
        error: null,
      };
      return result;
    }

    case COMMANDS.modelsList:
      return clone(state.models);

    case COMMANDS.modelsDownload: {
      const id = requireString(args, "id", "SA-4001");
      const model = findModel(id);
      const downloadId = uuid();
      const previous = state.downloads.get(id);
      if (previous?.timer != null) window.clearInterval(previous.timer);
      if (model.installed) {
        emit<DownloadStatePayload>(EVENTS.downloadState, { model: id, state: "installed" });
        return downloadId;
      }
      const total = model.sizeBytes;
      const step = Math.max(1, Math.round(total * 0.04));
      const entry: MockDownload = { timer: null, loaded: 0, state: "downloading" };
      state.downloads.set(id, entry);
      model.downloadState = "downloading";
      model.installedBytes = 0;
      emit<DownloadStatePayload>(EVENTS.downloadState, { model: id, state: "downloading" });
      entry.timer = window.setInterval(() => {
        const current = state.downloads.get(id);
        if (current !== entry || entry.state !== "downloading") return;
        entry.loaded = Math.min(total, entry.loaded + step);
        model.installedBytes = entry.loaded;
        emit<DownloadProgressPayload>(EVENTS.downloadProgress, {
          model: id,
          loaded: entry.loaded,
          total,
          speedBps: Math.round(step * 10),
        });
        if (entry.loaded < total) return;
        if (entry.timer !== null) window.clearInterval(entry.timer);
        entry.timer = null;
        entry.state = "verifying";
        emit<DownloadStatePayload>(EVENTS.downloadState, { model: id, state: "verifying" });
        window.setTimeout(() => {
          if (entry.state !== "verifying") return;
          entry.state = "installed";
          model.installed = true;
          model.installedBytes = total;
          model.downloadState = "installed";
          emit<DownloadStatePayload>(EVENTS.downloadState, { model: id, state: "installed" });
        }, 120);
      }, 100);
      return downloadId;
    }

    case COMMANDS.modelsCancel: {
      const id = requireString(args, "id", "SA-4001");
      const entry = state.downloads.get(id);
      if (!entry) return undefined;
      if (entry.timer !== null) window.clearInterval(entry.timer);
      entry.timer = null;
      entry.state = "cancelled";
      const model = state.models.find((item) => item.id === id);
      if (model) {
        model.downloadState = "cancelled";
        model.installedBytes = 0;
      }
      emit<DownloadStatePayload>(EVENTS.downloadState, { model: id, state: "cancelled" });
      return undefined;
    }

    case COMMANDS.modelsRemove: {
      const id = requireString(args, "id", "SA-4001");
      const model = findModel(id);
      const entry = state.downloads.get(id);
      if (entry?.timer != null) window.clearInterval(entry.timer);
      state.downloads.delete(id);
      model.installed = false;
      model.installedBytes = 0;
      model.downloadState = "queued";
      return undefined;
    }

    case COMMANDS.translateTest: {
      const text = requireString(args, "text", "SA-4001");
      const target =
        typeof args.target === "string" && args.target.length > 0 ? args.target : "zh-CHS";
      await delay(150);
      return `【骨架模式译文】${text}（目标语言：${target}，未调用真实翻译服务）`;
    }

    case COMMANDS.hotkeyProbe: {
      if (typeof args.spec !== "string") throw appError("SA-1003", "热键规格必须为字符串");
      return normalizeHotkey(args.spec);
    }

    case COMMANDS.setAutostart: {
      const enabled = requireBoolean(args, "enabled");
      state.config.app.autostart = enabled;
      persistConfig();
      emitConfigChanged(["app.autostart"]);
      return undefined;
    }

    case COMMANDS.openExternal: {
      const url = requireString(args, "url", "SA-5005");
      const allowed = /^https:\/\//i.test(url) || /^http:\/\/127\.0\.0\.1(:\d+)?(\/|$)/i.test(url);
      if (!allowed) throw appError("SA-5005", "仅允许 https:// 或 http://127.0.0.1 链接");
      console.info(`[snipask mock] 模拟打开外部链接：${url}`);
      return undefined;
    }

    case COMMANDS.appStats:
      return appStats();

    case COMMANDS.windowShow: {
      const label = typeof args.window === "string" ? args.window : "main";
      notifyWindowChange(label, true);
      return undefined;
    }

    case COMMANDS.windowHide: {
      const label = typeof args.window === "string" ? args.window : "main";
      notifyWindowChange(label, false);
      return undefined;
    }

    case COMMANDS.appQuit:
      console.info("[snipask mock] 已忽略退出请求（浏览器预览不会退出）");
      return undefined;

    default:
      throw appError("SA-9001", `未知命令：${String(command)}`);
  }
}

/* ───────────────────────── 对外导出 ───────────────────────── */

export const mockBackend: IpcBackend = {
  invoke<T>(command: CommandName, args?: Record<string, unknown>): Promise<T> {
    return dispatch(command, args) as Promise<T>;
  },

  on<T>(event: EventName, handler: (payload: T) => void): () => void {
    let handlers = listeners.get(event);
    if (!handlers) {
      handlers = new Set<IpcHandler>();
      listeners.set(event, handlers);
    }
    const wrapped = handler as IpcHandler;
    handlers.add(wrapped);
    const target = handlers;
    return () => {
      target.delete(wrapped);
      if (target.size === 0) listeners.delete(event);
    };
  },
};
