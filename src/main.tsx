/**
 * 前端入口：初始化主题 → 挂载窗口分发。
 * 窗口标签由 `?window=` / Tauri 窗口 label 决定（见 shared/ipc 的 getWindowLabel）。
 */

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./app/App";
import { ErrorBoundary } from "./app/ErrorBoundary";
import { initTheme } from "./shared/theme/theme";
import "./styles/tokens.css";
import "./styles/base.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("未找到 #root 挂载点，请检查 index.html");
}

void initTheme();

createRoot(container).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
