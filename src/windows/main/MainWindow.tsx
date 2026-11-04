/**
 * main 窗口壳：自绘顶栏 + 左侧导航（设置 / 首启引导）+ 右侧内容。
 * 设置页、引导页内部由 frontend-pages 实现，这里只做路由与启动逻辑。
 *
 * 行为约定：
 * - 启动读 `onboarding_state`：未完成且未跳过 → 进入引导，否则设置页；
 * - 顶栏关闭 = 隐藏窗口（`window_hide`），不退出应用（退出走托盘 / app_quit）；
 * - 订阅 `config:changed` 同步主题/强调色，订阅 `tray:action` 切换路由。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { Button, IconButton, Spinner } from "../../components/ui";
import { COMMANDS, EVENTS } from "../../shared/contract/names";
import type {
  ConfigChangedPayload,
  ConfigView,
  OnboardingState,
  TrayActionPayload,
} from "../../shared/contract/types";
import { useIpcEvent } from "../../shared/hooks/useIpcEvent";
import { ipc, isBrowserPreview, toAppError } from "../../shared/ipc";
import { applyAccent, applyTheme } from "../../shared/theme/theme";
import OnboardingWizard from "./onboarding/OnboardingWizard";
import SettingsPage from "./settings/SettingsPage";
import "./main.css";

type MainRoute = "settings" | "onboarding";

export default function MainWindow() {
  const [route, setRoute] = useState<MainRoute>("settings");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const themeCleanupRef = useRef<(() => void) | null>(null);

  // 路由 `/` 支持 `?section=xxx` 直达设置分组（脚手架可选能力）
  const [initialSection] = useState<string | undefined>(() => {
    if (typeof window === "undefined") return undefined;
    return new URLSearchParams(window.location.search).get("section") ?? undefined;
  });

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    ipc
      .invoke<OnboardingState>(COMMANDS.onboardingState)
      .then((state) => {
        if (cancelled) return;
        if (!state.completed && !state.skipped) setRoute("onboarding");
        setLoading(false);
      })
      .catch((raw) => {
        if (cancelled) return;
        setError(toAppError(raw).message);
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [reloadToken]);

  const applyConfigTheme = useCallback((config: ConfigView) => {
    themeCleanupRef.current?.();
    themeCleanupRef.current = applyTheme(config.ui.theme);
    applyAccent(config.ui.accent);
  }, []);

  useEffect(
    () => () => {
      themeCleanupRef.current?.();
      themeCleanupRef.current = null;
    },
    [],
  );

  useIpcEvent<ConfigChangedPayload>(EVENTS.configChanged, (payload) => {
    if (!payload || !payload.config) return;
    applyConfigTheme(payload.config);
  });

  useIpcEvent<TrayActionPayload>(EVENTS.trayAction, (payload) => {
    if (!payload) return;
    if (payload.id === "openSettings") {
      setRoute("settings");
      void ipc.invoke(COMMANDS.windowShow, { window: "main" }).catch(() => undefined);
    } else if (payload.id === "rerunOnboarding") {
      setRoute("onboarding");
      void ipc.invoke(COMMANDS.windowShow, { window: "main" }).catch(() => undefined);
    }
  });

  const hideWindow = useCallback(() => {
    void ipc.invoke(COMMANDS.windowHide, { window: "main" }).catch(() => undefined);
  }, []);

  const handleFinish = useCallback((_skip: boolean) => {
    setRoute("settings");
  }, []);

  return (
    <div className="main-window">
      <header className="main-titlebar" data-tauri-drag-region>
        <div className="main-titlebar__brand" data-tauri-drag-region>
          <span className="main-titlebar__logo" aria-hidden="true">
            框
          </span>
          <span className="main-titlebar__title">框选 AI</span>
          <span className="main-titlebar__badge">
            {isBrowserPreview() ? "浏览器预览 · 骨架模式" : "骨架模式"}
          </span>
        </div>
        <div className="main-titlebar__actions">
          <IconButton label="最小化到托盘" title="最小化到托盘" onClick={hideWindow}>
            <span aria-hidden="true">—</span>
          </IconButton>
          <IconButton
            label="关闭窗口（继续在托盘运行）"
            title="关闭窗口（继续在托盘运行）"
            onClick={hideWindow}
          >
            <span aria-hidden="true">×</span>
          </IconButton>
        </div>
      </header>

      <div className="main-body">
        <nav className="main-nav" aria-label="主导航">
          <button
            type="button"
            className={`main-nav__item${route === "settings" ? " main-nav__item--active" : ""}`}
            aria-current={route === "settings" ? "page" : undefined}
            onClick={() => setRoute("settings")}
          >
            设置
          </button>
          <button
            type="button"
            className={`main-nav__item${route === "onboarding" ? " main-nav__item--active" : ""}`}
            aria-current={route === "onboarding" ? "page" : undefined}
            onClick={() => setRoute("onboarding")}
          >
            首启引导
          </button>
          <p className="main-nav__note">
            脚手架阶段：捕获 / OCR / 模型均为骨架实现，命令与事件遵循 IPC 契约 v1。
          </p>
        </nav>

        <main className="main-content sa-scroll">
          {loading ? (
            <div className="main-state">
              <Spinner size="lg" label="正在读取配置…" />
            </div>
          ) : error ? (
            <div className="main-state">
              <p className="main-state__error">初始化失败：{error}</p>
              <Button onClick={() => setReloadToken((token) => token + 1)}>重试</Button>
            </div>
          ) : route === "onboarding" ? (
            <OnboardingWizard onFinish={handleFinish} />
          ) : (
            <SettingsPage initialSection={initialSection} />
          )}
        </main>
      </div>
    </div>
  );
}
