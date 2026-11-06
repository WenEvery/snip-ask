/**
 * 渲染异常兜底：捕获子树渲染/生命周期异常，展示错误码与重试。
 */

import { Component } from "react";
import type { ErrorInfo, ReactNode } from "react";
import { Button } from "../components/ui";
import { t } from "../i18n";
import "./app.css";

/** 前端渲染异常统一错误码（契约 §8 之外的 UI 侧兜底码） */
export const FRONTEND_ERROR_CODE = "SA-9000";

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  error: Error | null;
}

function toError(raw: unknown): Error {
  return raw instanceof Error ? raw : new Error(String(raw));
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(raw: unknown): ErrorBoundaryState {
    return { error: toError(raw) };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error(`[${FRONTEND_ERROR_CODE}] 前端渲染异常`, error, info.componentStack);
  }

  private readonly handleRetry = (): void => {
    this.setState({ error: null });
  };

  render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="sa-fatal">
        <div className="sa-fatal__card sa-anim-slide">
          <h1 className="sa-fatal__title">{t("界面渲染异常")}</h1>
          <p className="sa-fatal__code">
            {t("错误码：")}
            {FRONTEND_ERROR_CODE}
          </p>
          <p className="sa-fatal__desc">
            {t("请点击重试；若问题持续，请重启应用并在日志中检索该错误码。")}
          </p>
          <pre className="sa-fatal__detail">{error.message}</pre>
          <div className="sa-fatal__actions">
            <Button variant="primary" onClick={this.handleRetry}>
              {t("重试")}
            </Button>
          </div>
        </div>
      </div>
    );
  }
}
