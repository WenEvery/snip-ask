/**
 * main 窗口壳：自绘标题栏（无系统边框）+ 左侧导航（设置 / 历史 / 使用引导）+ 右侧内容。
 *
 * 行为约定：
 * - 启动读 `onboarding_state`：未完成且未跳过 → 进入引导，否则设置页；
 * - 标题栏：拖拽移动、最小化到任务栏、最大化/还原（双击标题栏亦可）、关闭 = 隐藏到托盘；
 * - 四边四角缩放手柄；最大化时隐藏（窗口 API 在浏览器预览下为 no-op）；
 * - 订阅 `config:changed` 同步主题/强调色，订阅 `tray:action` 切换路由；
 * - 引导完成后隐藏「首启引导」导航；从托盘「重新运行引导」进入时仍显示。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import { Button, IconButton, Spinner } from "../../components/ui";
import { COMMANDS, EVENTS } from "../../shared/contract/names";
import type {
  ConfigChangedPayload,
  ConfigView,
  ModelInfo,
  OnboardingState,
  TrayActionPayload,
} from "../../shared/contract/types";
import { useIpcEvent } from "../../shared/hooks/useIpcEvent";
import { ipc, isBrowserPreview, toAppError } from "../../shared/ipc";
import { applyAccent, applyTheme } from "../../shared/theme/theme";
import { t } from "../../i18n";
import HistoryPage from "./history/HistoryPage";
import OnboardingWizard from "./onboarding/OnboardingWizard";
import type { Capabilities } from "./onboarding/capabilities";
import { deriveCapabilities, isRestricted } from "./onboarding/capabilities";
import { RestrictedNotice } from "./onboarding/components/RestrictedNotice";
import SettingsPage from "./settings/SettingsPage";
import { useWindowControls, WindowResizeHandles } from "./windowChrome";
import "./main.css";

type MainRoute = "settings" | "onboarding" | "history";

export default function MainWindow() {
  const [route, setRoute] = useState<MainRoute>("settings");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const themeCleanupRef = useRef<(() => void) | null>(null);
  // 能力快照（用于「未配置 AI 服务」提示与「去配置」入口）与引导状态。
  const [capabilities, setCapabilities] = useState<Capabilities | null>(null);
  const [onboardingFlags, setOnboardingFlags] = useState<{
    completed: boolean;
    skipped: boolean;
  } | null>(null);
  const [settingsSection, setSettingsSection] = useState<string>("general");
  const { maximized, minimize, toggleMaximize, toggleMaximizeFromDoubleClick } =
    useWindowControls();

  // 路由 `/` 支持 `?section=xxx` 直达设置分组
  const [initialSection] = useState<string | undefined>(() => {
    if (typeof window === "undefined") return undefined;
    return new URLSearchParams(window.location.search).get("section") ?? undefined;
  });

  // 读取能力快照：受限提示与「去配置」入口的数据来源。
  const refreshCapabilities = useCallback(async () => {
    try {
      const [view, models, state] = await Promise.all([
        ipc.invoke<ConfigView>(COMMANDS.configGet),
        ipc.invoke<ModelInfo[]>(COMMANDS.modelsList),
        ipc.invoke<OnboardingState>(COMMANDS.onboardingState),
      ]);
      setCapabilities(deriveCapabilities({ providers: view.providers, models }));
      setOnboardingFlags({ completed: state.completed, skipped: state.skipped });
    } catch {
      // 能力探测失败时静默降级：不阻塞主界面。
    }
  }, []);

  const openConfigure = useCallback(() => {
    setSettingsSection("ai");
    setRoute("settings");
  }, []);

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
    void refreshCapabilities();
    return () => {
      cancelled = true;
    };
  }, [reloadToken, refreshCapabilities]);

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
    // 配置变化可能补齐 Key / 模型 / 隐私开关 → 重新评估受限模式。
    void refreshCapabilities();
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

  const handleFinish = useCallback(
    (_skip: boolean) => {
      setRoute("settings");
      void refreshCapabilities();
    },
    [refreshCapabilities],
  );

  const restricted = capabilities !== null && isRestricted(capabilities) && route === "settings";
  // 引导完成后隐藏入口；从托盘「重新运行引导」进入时仍显示。
  const onboardingDone =
    onboardingFlags !== null && (onboardingFlags.completed || onboardingFlags.skipped);
  const showOnboardingNav = !onboardingDone || route === "onboarding";

  const onTitlebarDoubleClick = (event: ReactMouseEvent<HTMLElement>) => {
    if ((event.target as HTMLElement).closest("button")) return;
    toggleMaximizeFromDoubleClick();
  };

  return (
    <div className={`main-window${maximized ? " is-maximized" : ""}`}>
      <header
        className="main-titlebar"
        data-tauri-drag-region
        onDoubleClick={onTitlebarDoubleClick}
      >
        <div className="main-titlebar__brand" data-tauri-drag-region>
          <span className="main-titlebar__logo" aria-hidden="true">
            框
          </span>
          <span className="main-titlebar__title">{t("框选 AI")}</span>
          {isBrowserPreview() ? <span className="main-titlebar__badge">{t("预览")}</span> : null}
        </div>
        <div className="main-titlebar__actions">
          <IconButton label={t("最小化到任务栏")} title={t("最小化")} onClick={minimize}>
            <span aria-hidden="true">—</span>
          </IconButton>
          <IconButton
            label={maximized ? t("还原窗口") : t("最大化")}
            title={maximized ? t("还原") : t("最大化")}
            onClick={toggleMaximize}
          >
            <span aria-hidden="true">{maximized ? "❐" : "□"}</span>
          </IconButton>
          <IconButton
            label={t("关闭窗口（继续在托盘运行）")}
            title={t("关闭窗口（继续在托盘运行）")}
            onClick={hideWindow}
          >
            <span aria-hidden="true">×</span>
          </IconButton>
        </div>
      </header>

      <div className="main-body">
        <nav className="main-nav" aria-label={t("主导航")}>
          <button
            type="button"
            className={`main-nav__item${route === "settings" ? " main-nav__item--active" : ""}`}
            aria-current={route === "settings" ? "page" : undefined}
            onClick={() => setRoute("settings")}
          >
            {t("设置")}
          </button>
          <button
            type="button"
            className={`main-nav__item${route === "history" ? " main-nav__item--active" : ""}`}
            aria-current={route === "history" ? "page" : undefined}
            onClick={() => setRoute("history")}
          >
            {t("历史")}
          </button>
          {showOnboardingNav ? (
            <button
              type="button"
              className={`main-nav__item${route === "onboarding" ? " main-nav__item--active" : ""}`}
              aria-current={route === "onboarding" ? "page" : undefined}
              onClick={() => setRoute("onboarding")}
            >
              {t("使用引导")}
            </button>
          ) : null}
          <p className="main-nav__note">{t("历史与配置都保存在本机，不会上传。")}</p>
        </nav>

        <main className="main-content sa-scroll">
          {loading ? (
            <div className="main-state">
              <Spinner size="lg" label={t("正在读取配置…")} />
            </div>
          ) : error ? (
            <div className="main-state">
              <p className="main-state__error">{t("初始化失败：{message}", { message: error })}</p>
              <Button onClick={() => setReloadToken((token) => token + 1)}>{t("重试")}</Button>
            </div>
          ) : (
            <>
              {restricted && capabilities ? (
                <RestrictedNotice
                  capabilities={capabilities}
                  variant="banner"
                  skipped={onboardingFlags?.skipped ?? false}
                  onConfigure={openConfigure}
                />
              ) : null}
              {route === "onboarding" ? (
                <OnboardingWizard onFinish={handleFinish} onOpenSettings={openConfigure} />
              ) : route === "history" ? (
                <HistoryPage />
              ) : (
                <SettingsPage
                  key={`settings-${settingsSection}`}
                  initialSection={settingsSection === "general" ? initialSection : settingsSection}
                />
              )}
            </>
          )}
        </main>
      </div>
      <WindowResizeHandles visible={!maximized} />
    </div>
  );
}
