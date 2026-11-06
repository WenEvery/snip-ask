/**
 * 外观分组的即时预览。
 *
 * 正式主题由 `@/shared/theme/theme` 的 initTheme 在启动时写入；这里只在用户
 * 操作控件时写 DOM 属性，保证「改一下马上看得见」，保存后以后端配置为准。
 */

import type { FontSize, Theme } from "@/shared/contract/types";

export function previewTheme(theme: Theme): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = theme;
}

export function previewAccent(accent: string): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.accent = accent;
}

export function previewFontSize(size: FontSize): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.fontSize = size;
}
