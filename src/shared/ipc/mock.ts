/**
 * 浏览器预览后端（不连接真实服务，仅用于在没有安装桌面版时预览界面）。
 *
 * 目标：**无 Rust 环境**也能在浏览器里完整走通
 * 「capture_freeze → overlay:ready → 框选 → menu:ready → menu_execute → task.* 流式」。
 *
 * 行为约定：
 * - 配置持久化：`localStorage["snipask.mock.config"]`（深合并默认值）；密钥只存内存；
 * - 引导状态：`localStorage["snipask.mock.onboarding"]`，初始 `completed=false`；
 * - 文案保持自然，贴近真实结果；
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
  BackendInfo,
  Config,
  ConfigChangedPayload,
  ConfigView,
  CoverBlock,
  CoveredBlock,
  CoverResult,
  DeepPartial,
  DownloadProgressPayload,
  DownloadState,
  DownloadStatePayload,
  EngineStatusPayload,
  GpuProbeResult,
  HistorySession,
  HistoryThread,
  HotkeyStatus,
  MenuReadyPayload,
  Mode,
  ModelInfo,
  OcrAssetStatus,
  OcrBox,
  OcrStatus,
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
  TranslateProgressPayload,
  TranslateReadyPayload,
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
  /** 本会话启动过的任务（按时间顺序；追问定位与真实后端 `session.task_ids` 一致）。 */
  taskIds: TaskId[];
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
  taskIds: [],
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
  state.taskIds = [];
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
    return `（预览模式示例译文）${state.selection?.preview ?? SKELETON_OCR_PREVIEW}`;
  }
  if (action === "search") {
    const query = state.selection?.preview ?? SKELETON_OCR_PREVIEW;
    return mode === "deep"
      ? `正在浏览器中搜索 “${query}”（深度模式会多查资料再总结）。${SKELETON_ANSWER}`
      : `正在浏览器中搜索 “${query}”。${SKELETON_ANSWER}`;
  }
  if (action === "copy") {
    return `已复制识别文字：${SKELETON_OCR_PREVIEW}`;
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

/**
 * 浏览器 mock 的合成冻结帧（真实位图，非占位）。
 *
 * 为什么要真实位图：覆盖式翻译的 Canvas 模糊/遮罩/深浅字色只能在真实像素上验证
 * （研究 §2.2 的原型也是真实截图）。这里用 Canvas 画一个浅色页面 + 深色卡片 + 四行中英文，
 * 既无外部资源依赖，又能让 TranslationOverlay 采样背景亮度、绘制模糊与译文。
 */
const MOCK_FRAME_LINES: Array<{
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
}> = [
  { text: "框选 AI（Snip Ask）", x: 180, y: 205, width: 520, height: 56 },
  { text: "本地 GPU 推理：Vulkan 优先，CUDA 可选。", x: 180, y: 295, width: 900, height: 56 },
  { text: "翻译覆盖原句 · 连续提问", x: 180, y: 385, width: 620, height: 56 },
  { text: "AMD / Intel / NVIDIA 均可使用 Vulkan 后端。", x: 180, y: 475, width: 1000, height: 56 },
];

let mockFrameCache: string | null = null;

function mockFrameDataUrl(): string | null {
  if (mockFrameCache) return mockFrameCache;
  if (typeof document === "undefined") return null;
  try {
    const canvas = document.createElement("canvas");
    canvas.width = 1920;
    canvas.height = 1080;
    const ctx = canvas.getContext("2d");
    if (!ctx) return null;
    // 浅色页面背景 + 深色内容卡片（制造深浅两种底色，验证字色与衬底）。
    ctx.fillStyle = "#f2f4f8";
    ctx.fillRect(0, 0, 1920, 1080);
    ctx.fillStyle = "#1f2430";
    ctx.fillRect(120, 160, 1400, 420);
    ctx.font = "44px 'Microsoft YaHei', 'Segoe UI', sans-serif";
    ctx.textBaseline = "top";
    MOCK_FRAME_LINES.forEach((line) => {
      ctx.fillStyle = "#e8ecf3";
      ctx.fillText(line.text, line.x, line.y);
    });
    mockFrameCache = canvas.toDataURL("image/png");
    return mockFrameCache;
  } catch {
    return null;
  }
}

/** 合成帧对应的 OCR 行框（覆盖翻译的段块合并输入）。 */
function mockLineBoxes(within?: RectPhys): OcrBox[] {
  return MOCK_FRAME_LINES.map((line, index) => ({
    text: line.text,
    confidence: 0.95 - index * 0.01,
    bbox: {
      x: line.x,
      y: line.y,
      width: line.width,
      height: line.height,
    },
  })).filter((box) => {
    if (!within) return true;
    return (
      box.bbox.x < within.x + within.width &&
      box.bbox.x + box.bbox.width > within.x &&
      box.bbox.y < within.y + within.height &&
      box.bbox.y + box.bbox.height > within.y
    );
  });
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
        frame: { width: 1920, height: 1080, dataUrl: mockFrameDataUrl() },
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
                boxes: mockLineBoxes(virtualRect),
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
    boxes: mockLineBoxes(rectPhys),
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
  // 与真实后端一致：menu（首轮）或 showingResult（结果窗「重答」）都可执行。
  if (state.phase !== "menu" && state.phase !== "showingResult") {
    throw appError("SA-1099", "当前会话不在动作菜单状态");
  }
  const action = parseAction(args?.action);
  const mode = parseMode(args?.mode, actionMode(state.config, action));
  // 重答：取消进行中的旧任务（不再推送旧任务事件）。
  for (const [oldId, oldTask] of state.tasks) {
    oldTask.cancelled = true;
    if (oldTask.timer !== null) window.clearTimeout(oldTask.timer);
    state.tasks.delete(oldId);
  }
  const taskId = uuid();
  state.tasks.set(taskId, { cancelled: false, timer: null, startedAt: Date.now() });
  state.taskIds.push(taskId);
  state.phase = "executing";
  state.lastAction = action;
  state.lastMode = mode;
  emit<TaskStartedPayload>(EVENTS.taskStarted, {
    task: taskId,
    session,
    action,
    mode,
    engine: "skeleton",
    // 首轮 / 重答：parent = null（前端重置线程）。
    parent: null,
  });
  notifyWindowChange("result", true);
  startTaskStream(taskId, { action, mode });
  return taskId;
}

function taskFollowup(args: Record<string, unknown> | undefined): TaskId {
  const previous = requireString(args, "task");
  const text = requireString(args, "text");
  // 与真实后端 `locate_followup_target` 语义一致（修 F8：mock 不再放行真实后端会拒绝的路径）：
  // 1) 必须有活动会话；2) 必须处于 showingResult（流式中拒绝并给出可读提示）；
  // 3) parent = 本会话最近一次任务；4) 传入的 task 仅作校验，
  //    不属于本会话/已不存在时回退最近一轮（不得必失败），属于本会话但不是最近一轮则拒绝。
  if (state.session === null) throw appError("SA-1099", "没有活动会话，无法追问");
  if (state.phase !== "showingResult") {
    throw appError("SA-1099", "当前回答仍在生成，请等待结束后再追问");
  }
  const last = state.taskIds[state.taskIds.length - 1];
  if (!last) throw appError("SA-1099", "会话没有可追问的任务");
  if (previous !== last && state.taskIds.includes(previous)) {
    throw appError("SA-1099", "只能对最近一轮回答继续追问");
  }
  const taskId = uuid();
  state.tasks.set(taskId, { cancelled: false, timer: null, startedAt: Date.now() });
  state.taskIds.push(taskId);
  state.phase = "executing";
  emit<TaskStartedPayload>(EVENTS.taskStarted, {
    task: taskId,
    session: state.session,
    action: state.lastAction ?? "answer",
    mode: state.lastMode,
    engine: "skeleton",
    // 追问：parent 指向上一轮任务（前端追加气泡而不是重置线程）。
    parent: last,
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
  // 真实后端取消后会话回到结果态（可继续追问/重答），且该任务仍是「最近一轮」。
  if (state.phase === "executing") state.phase = "showingResult";
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
    version: "1.0.1",
    contractVersion: IPC_CONTRACT_VERSION,
    edition: "dev",
    ocrReady: true,
    uptimeSec: Math.max(1, Math.round((Date.now() - state.startedAt) / 1000)),
    memoryMb: 186,
    activeSession: state.session,
    activeTasks: state.tasks.size,
    localEngine: "stopped",
    modelsInstalled: state.models.filter((model) => model.installed).length,
    skeletonMode: state.config.advanced.skeletonMode,
  };
}


/** 浏览器预览的 OCR 状态（确定性 mock：视为已就绪，便于预览 UI）。 */
function mockOcrStatus(): OcrStatus {
  const assets: OcrAssetStatus[] = [
    {
      id: "ocr-det",
      name: "OCR 文本检测（PP-OCRv4 det）",
      installed: true,
      bundled: false,
      sizeBytes: 4745517,
      sha256: "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9",
    },
    {
      id: "ocr-rec",
      name: "OCR 文字识别（PP-OCRv4 rec）",
      installed: true,
      bundled: false,
      sizeBytes: 10857958,
      sha256: "48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b",
    },
    {
      id: "ocr-cls",
      name: "OCR 方向分类（PP-OCRv4 cls）",
      installed: true,
      bundled: false,
      sizeBytes: 585532,
      sha256: "e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c",
    },
    {
      id: "ocr-dict",
      name: "OCR 字典（ppocr_keys_v1）",
      installed: true,
      bundled: false,
      sizeBytes: 26250,
      sha256: "a1c84d9bdb9ab29043c58896224d32941783eb821629618416dc08f12886492",
    },
  ];
  return {
    ready: true,
    bundled: false,
    runtimeReady: true,
    runtimePath: "C:/mock/ocr/onnxruntime.dll",
    assets,
    missing: [],
    message: null,
  };
}

/** 浏览器预览的历史会话存储（支持「清空单个 / 全部」手测）。 */
let mockHistoryStore: HistorySession[] | null = null;

/** 默认历史会话（确定性 mock）。 */
function defaultMockHistorySessions(): HistorySession[] {
  return [
    {
      session: "mock-session-2",
      startedAt: Date.now() - 60_000,
      title: "什么是框选 AI？",
      action: "answer",
      turns: 2,
      engine: "deepseek",
    },
    {
      session: "mock-session-1",
      startedAt: Date.now() - 3_600_000,
      title: "Hello world 翻译成中文",
      action: "translate",
      turns: 1,
      engine: "skeleton",
    },
  ];
}

/** 浏览器预览的历史会话列表（返回副本，避免调用方改内部状态）。 */
function mockHistorySessions(): HistorySession[] {
  if (mockHistoryStore === null) mockHistoryStore = defaultMockHistorySessions();
  return mockHistoryStore.map((item) => ({ ...item }));
}

/** 浏览器预览的单个历史线程（确定性 mock）。 */
function mockHistoryThread(session: SessionId): HistoryThread {
  const summary = mockHistorySessions().find((item) => item.session === session);
  if (!summary) throw appError("SA-1099", "会话不存在或已被清理");
  return {
    session: summary,
    turns: [
      {
        task: "mock-task-1",
        action: summary.action ?? "answer",
        mode: "quick",
        question: summary.title,
        answer: "这是浏览器预览的历史回答示例。桌面版会在这里显示真实的问答记录。",
        engine: summary.engine,
        status: "done",
        createdAt: summary.startedAt,
      },
    ],
  };
}

/** 浏览器预览的 GPU 探测结果（确定性 mock）。真实后端会执行
 * `llama-server --list-devices`（Vulkan 优先），此处仅演示字段形状与文案。
 */
function mockGpuProbe(): GpuProbeResult {
  return {
    checkedAt: new Date().toISOString(),
    cached: false,
    useable: true,
    recommended: "vulkan",
    devices: [
      {
        index: 0,
        name: "浏览器预览设备（AMD Radeon RX 7800 XT）",
        backend: "vulkan",
        vramTotalMb: 16384,
        vramFreeMb: 15200,
        vendor: "amd",
        virtualAdapter: false,
      },
    ],
    backends: [
      {
        backend: "vulkan",
        installed: true,
        available: true,
        exePath: "C:/mock/backends/vulkan/llama-server.exe",
        version: "2.28.2",
        reason: null,
      },
      {
        backend: "cuda",
        installed: false,
        available: false,
        exePath: null,
        version: null,
        reason: "CUDA 为 NVIDIA 可选组件，未安装",
      },
      {
        backend: "cpu",
        installed: true,
        available: true,
        exePath: "C:/mock/backends/cpu/llama-server.exe",
        version: "2.28.2",
        reason: null,
      },
    ],
    budget: { requiredMb: 2112, availableMb: 15200, fits: true },
    issue: null,
  };
}

/** 浏览器预览的引擎状态（真实后端由 `engine:status` 事件同步）。 */
function mockEngineStatus(local: EngineStatusPayload["local"] = "stopped"): EngineStatusPayload {
  if (local === "stopped") {
    return {
      local,
      device: null,
      backend: null,
      fallbackReason: null,
      offloadLayers: null,
      deviceName: null,
      port: null,
    };
  }
  return {
    local,
    device: "gpu",
    backend: "vulkan",
    fallbackReason: null,
    offloadLayers: 25,
    deviceName: "浏览器预览设备（AMD Radeon RX 7800 XT）",
    port: 39281,
  };
}

/** 浏览器预览的后端组件清单。 */
function mockBackends(): BackendInfo[] {
  return [
    {
      id: "vulkan",
      label: "Vulkan（AMD/Intel/NVIDIA 通用，推荐）",
      backend: "vulkan",
      version: "2.28.2",
      sizeBytes: 58_515_456,
      sha256: "0".repeat(64),
      license: "MIT",
      urls: ["https://example.com/llama-vulkan.zip"],
      requiresVendor: null,
      installed: true,
      installedBytes: 58_515_456,
      downloadState: "installed",
    },
    {
      id: "cuda",
      label: "CUDA（NVIDIA 可选组件）",
      backend: "cuda",
      version: "2.67.0",
      sizeBytes: 176_160_768,
      sha256: "0".repeat(64),
      license: "NVIDIA CUDA EULA",
      urls: ["https://example.com/llama-cuda.zip"],
      requiresVendor: "nvidia",
      installed: false,
      installedBytes: 0,
      downloadState: "idle",
    },
    {
      id: "cpu",
      label: "CPU（随主安装包）",
      backend: "cpu",
      version: "2.28.2",
      sizeBytes: 16_887_808,
      sha256: "0".repeat(64),
      license: "MIT",
      urls: [],
      requiresVendor: null,
      installed: true,
      installedBytes: 16_887_808,
      downloadState: "installed",
    },
  ];
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
        throw appError("SA-5002", "预览模式不支持导出明文密钥");
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
      return `（预览模式示例译文，目标语言：${target}）${text}`;
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

    // ── 契约 v5：自定义目录 / OCR 状态 / 历史线程 ──
    case COMMANDS.openPath: {
      const kind = requireString(args, "kind", "SA-5001");
      const allowed = ["config", "prompts", "models", "logs", "backends", "data", "docs"];
      if (!allowed.includes(kind)) throw appError("SA-5001", `不支持的目录类型：${kind}`);
      console.info(`[snipask mock] 模拟打开目录：${kind}`);
      return undefined;
    }

    case COMMANDS.ocrStatus:
      return mockOcrStatus();

    case COMMANDS.historyList:
      return mockHistorySessions();

    case COMMANDS.historyThread: {
      const session = requireString(args, "session", "SA-1099");
      return mockHistoryThread(session);
    }

    case COMMANDS.historyClear: {
      const store = mockHistorySessions();
      const session = typeof args?.session === "string" && args.session.length > 0 ? args.session : null;
      mockHistoryStore = session === null ? [] : store.filter((item) => item.session !== session);
      return undefined;
    }

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

    // ── 契约 v4：本地 GPU 推理 / 后端组件 / 覆盖翻译（浏览器预览用确定性 mock） ──
    case COMMANDS.localGpuProbe:
      return mockGpuProbe();

    case COMMANDS.localEngineStatus:
      return mockEngineStatus();

    case COMMANDS.localEngineStart:
      return mockEngineStatus("ready");

    case COMMANDS.localEngineStop:
      return mockEngineStatus("stopped");

    case COMMANDS.backendsList:
      return mockBackends();

    case COMMANDS.backendsDownload: {
      const id = requireString(args, "id", "SA-4001");
      console.info(`[snipask mock] 模拟后端组件下载：${id}`);
      return `mock-download-${id}`;
    }

    case COMMANDS.backendsCancel:
      return undefined;

    case COMMANDS.backendsRemove:
      return undefined;

    case COMMANDS.translateCover: {
      const session = requireSession(args);
      const blocks = Array.isArray(args.blocks) ? (args.blocks as CoverBlock[]) : [];
      if (blocks.length === 0) throw appError("SA-2002", "没有可覆盖翻译的文本块");
      const total = blocks.length;
      const covered: CoveredBlock[] = blocks.map((block) => ({
        id: block.id,
        source: block.text,
        translation: `（预览模式示例译文）${block.text}`,
        failed: false,
        engine: "skeleton",
      }));
      const progress: TranslateProgressPayload = { session, done: total, total };
      emit<TranslateProgressPayload>(EVENTS.translateProgress, progress);
      const ready: TranslateReadyPayload = { session, blocks: covered, engine: "skeleton" };
      emit<TranslateReadyPayload>(EVENTS.translateReady, ready);
      await delay(120);
      const result: CoverResult = {
        session,
        blocks: covered,
        engine: "skeleton",
        latencyMs: 120,
        totalBlocks: total,
        translatedBlocks: total,
      };
      return result;
    }

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
