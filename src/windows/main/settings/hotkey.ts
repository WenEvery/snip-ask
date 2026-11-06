/**
 * 热键组合键的捕获与展示。
 *
 * 契约 §4.3：非空 spec 必须匹配 `^(Ctrl|Alt|Shift|Win)(\+(Ctrl|Alt|Shift|Win))*\+[A-Z0-9]$`，
 * 后端会把结果规范化为 `Ctrl+Q` 形式。
 */

import { t } from "@/i18n";
import type { HotkeyStatus, Hotkeys } from "@/shared/contract/types";

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta", "CapsLock"]);

interface KeyLike {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** 把键盘事件归一化为 `Ctrl+Shift+Q` 形式；无修饰键或非 A-Z0-9 时返回 null */
export function captureHotkey(event: KeyLike): string | null {
  if (MODIFIER_KEYS.has(event.key)) return null;
  const modifiers: string[] = [];
  if (event.ctrlKey) modifiers.push("Ctrl");
  if (event.altKey) modifiers.push("Alt");
  if (event.shiftKey) modifiers.push("Shift");
  if (event.metaKey) modifiers.push("Win");
  if (modifiers.length === 0) return null;
  const isLetterOrDigit = event.key.length === 1 && /[a-zA-Z0-9]/.test(event.key);
  if (!isLetterOrDigit) return null;
  return [...modifiers, event.key.toUpperCase()].join("+");
}

/** 展示用：空串表示未绑定 */
export function hotkeyLabel(spec: string): string {
  return spec.trim() ? spec.trim() : t("未绑定");
}

/** 热键行定义（设置页与引导共用字段顺序，各自渲染）。
 *
 * v1.0.6：只展示**真实可用**的热键——划词翻译 / 翻译选区 / 切换模式此前仅发了一个前端未处理的事件，
 * 已从界面移除（配置字段保留兼容，不再注册）。 */
export const HOTKEY_ROWS: { key: keyof Hotkeys; label: string; description: string }[] = [
  {
    key: "capture",
    label: t("开始框选"),
    description: t("按下后冻结屏幕并框选（默认 Ctrl+Q）"),
  },
  {
    key: "quickTranslate",
    label: t("快速翻译（剪贴板）"),
    description: t("读取剪贴板文字并直接翻译"),
  },
  {
    key: "imageSearch",
    label: t("图片搜索（框选）"),
    description: t("按下后框选，选完自动以图搜索"),
  },
  {
    key: "answer",
    label: t("解答选区（框选）"),
    description: t("按下后框选，选完自动解答"),
  },
  {
    key: "togglePause",
    label: t("暂停 / 恢复热键"),
    description: t("游戏等场景临时暂停全部全局热键"),
  },
  {
    key: "openSettings",
    label: t("打开设置"),
    description: t("直接打开本设置窗口"),
  },
];

/** 配置内冲突字段名（`hotkeys.*`）→ 可读标签，用于把 conflictWith 展示成用户能看懂的动作名 */
export const HOTKEY_FIELD_LABELS: Record<string, string> = Object.fromEntries(
  HOTKEY_ROWS.map((row) => [`hotkeys.${row.key}`, row.label]),
);

/** 契约 §6 中 `conflictWith = "system"` 表示被其它程序 / 系统占用（非配置内冲突） */
export const SYSTEM_CONFLICT = "system";

/** 把 `hotkey_probe` 的 `HotkeyStatus` 映射为**可操作**提示（含改键引导） */
export function hotkeyStatusHint(status: HotkeyStatus): { level: "ok" | "bad"; text: string } {
  if (!status.valid) {
    return {
      level: "bad",
      text: t("格式不合法：需至少一个修饰键 + 字母/数字，例如 Ctrl+Shift+Q"),
    };
  }
  if (status.available) {
    const normalized =
      status.normalized && status.normalized !== status.spec
        ? t("（规范化为 {spec}）", { spec: status.normalized })
        : "";
    return { level: "ok", text: t("可用{state}", { state: normalized }) };
  }
  if (status.conflictWith === SYSTEM_CONFLICT) {
    return {
      level: "bad",
      text: t("被其他程序占用（如 QQ / 微信 / 输入法），请点上方按键换一个组合，例如 Ctrl+Shift+Q"),
    };
  }
  const label = status.conflictWith
    ? (HOTKEY_FIELD_LABELS[status.conflictWith] ?? status.conflictWith)
    : t("其他热键");
  return {
    level: "bad",
    text: t("与应用内「{label}」重复，请点上方按键换一个组合", { label }),
  };
}
