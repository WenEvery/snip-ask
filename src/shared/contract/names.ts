/**
 * 接口契约 v1 的命令名 / 事件名常量（逐字对应 `docs/接口契约.md` §6、§7）。
 * 禁止在业务代码中手写字符串；一律引用本文件常量。
 */

/** 契约版本：出现在 `Stats.contractVersion` */
export const IPC_CONTRACT_VERSION = 1;

/** §6 命令表：29 个命令 */
export const COMMANDS = {
  captureFreeze: "capture_freeze",
  captureCancel: "capture_cancel",
  selectionCommit: "selection_commit",
  selectionReset: "selection_reset",
  menuExecute: "menu_execute",
  taskCancel: "task_cancel",
  taskFollowup: "task_followup",
  resultClose: "result_close",
  configGet: "config_get",
  configPatch: "config_patch",
  configImport: "config_import",
  configExport: "config_export",
  configReset: "config_reset",
  onboardingState: "onboarding_state",
  onboardingUpdate: "onboarding_update",
  onboardingFinish: "onboarding_finish",
  providerTest: "provider_test",
  modelsList: "models_list",
  modelsDownload: "models_download",
  modelsCancel: "models_cancel",
  modelsRemove: "models_remove",
  translateTest: "translate_test",
  hotkeyProbe: "hotkey_probe",
  setAutostart: "set_autostart",
  openExternal: "open_external",
  appStats: "app_stats",
  windowShow: "window_show",
  windowHide: "window_hide",
  appQuit: "app_quit",
} as const;

export type CommandName = (typeof COMMANDS)[keyof typeof COMMANDS];

/** §7 事件表：14 个事件 */
export const EVENTS = {
  overlayReady: "overlay:ready",
  overlaySelection: "overlay:selection",
  ocrStatus: "ocr:status",
  menuReady: "menu:ready",
  taskStarted: "task:started",
  taskDelta: "task:delta",
  taskDone: "task:done",
  taskError: "task:error",
  downloadProgress: "download:progress",
  downloadState: "download:state",
  configChanged: "config:changed",
  trayAction: "tray:action",
  engineStatus: "engine:status",
  updateStatus: "update:status",
} as const;

export type EventName = (typeof EVENTS)[keyof typeof EVENTS];

export const WINDOW_LABELS = ["main", "overlay", "result"] as const;

/** 从 `?window=` 读取的窗口标签是否合法 */
export function isWindowLabel(value: string | null): value is "main" | "overlay" | "result" {
  return value === "main" || value === "overlay" || value === "result";
}
