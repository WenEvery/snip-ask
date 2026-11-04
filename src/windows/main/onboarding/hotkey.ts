/** 引导页热键组合键捕获（规则同设置页，见契约 §4.3） */

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta", "CapsLock"]);

interface KeyLike {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

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

export function hotkeyLabel(spec: string): string {
  return spec.trim() ? spec.trim() : "未绑定";
}
