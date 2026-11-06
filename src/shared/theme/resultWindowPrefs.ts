/**
 * 结果窗外观偏好（v1.1.0）：不透明度即时应用。
 *
 * 结果窗固定为不透明窗口（避免 WebView2 透明窗口在部分显卡上的闪烁/点击穿透），
 * 因此这里把「不透明度」应用到内容根节点，视觉上呈现半透明卡片效果；
 * 宽度/高度/置顶/位置策略由后端 `app/windows.rs` 在创建/显示结果窗时应用。
 */

const RESULT_OPACITY_MIN = 0.3;
const RESULT_OPACITY_MAX = 1;

/** 当前是否结果窗（主窗口/框选层不应用）。 */
function isResultWindow(): boolean {
  if (typeof window === "undefined") return false;
  const search = new URLSearchParams(window.location.search);
  if (search.get("window") === "result") return true;
  return window.location.pathname.includes("/result");
}

/** 应用结果窗内容不透明度；范围 0.3–1.0，非法值回退 1.0。 */
export function applyResultWindowOpacity(opacity: number | null | undefined): void {
  if (!isResultWindow() || typeof document === "undefined") return;
  const root = document.getElementById("root");
  if (!root) return;
  const value = typeof opacity === "number" && Number.isFinite(opacity) ? opacity : 1;
  const clamped = Math.min(RESULT_OPACITY_MAX, Math.max(RESULT_OPACITY_MIN, value));
  root.style.opacity = String(clamped);
}
