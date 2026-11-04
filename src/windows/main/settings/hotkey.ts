/**
 * 热键组合键的捕获与展示。
 *
 * 契约 §4.3：非空 spec 必须匹配 `^(Ctrl|Alt|Shift|Win)(\+(Ctrl|Alt|Shift|Win))*\+[A-Z0-9]$`，
 * 后端会把结果规范化为 `Ctrl+Q` 形式。
 */

import type { Hotkeys } from "@/shared/contract/types";

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
  return spec.trim() ? spec.trim() : "未绑定";
}

/** 热键行定义（设置页与引导共用字段顺序，各自渲染） */
export const HOTKEY_ROWS: { key: keyof Hotkeys; label: string; description: string }[] = [
  { key: "capture", label: "框选并唤起", description: "全局默认 Ctrl+Q" },
  { key: "wordGrab", label: "划词翻译", description: "选中文字后触发翻译" },
  { key: "quickTranslate", label: "快速翻译", description: "剪贴板文本一键翻译" },
  { key: "imageSearch", label: "图片搜索", description: "框选区域直接以图搜" },
  { key: "translate", label: "翻译选区", description: "框选文字直接翻译" },
  { key: "answer", label: "解答选区", description: "框选内容直接解答" },
  { key: "toggleMode", label: "切换快速 / 深度", description: "临时切换下一动作模式" },
  { key: "togglePause", label: "暂停 / 恢复热键", description: "游戏等场景临时暂停" },
  { key: "openSettings", label: "打开设置", description: "直接打开本设置窗口" },
];
