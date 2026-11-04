/**
 * 设置页外壳：左侧分组导航 + 右侧分组内容。
 *
 * 数据流：`useConfigDraft` 拉取 / 维护草稿 → 各分组就地编辑 →
 * 分组「应用」通过 `config_patch` 只提交本组差异 → `config.changed` 回写。
 */

import { useState } from "react";
import { Button } from "@/components/ui";
import type { ConfigView } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { hasChanges } from "./diff";
import { ConfirmAction, ErrorNotice, LoadingBlock } from "./parts";
import { useConfigDraft } from "./useConfigDraft";
import type { SectionId, SectionProps } from "./types";
import { GeneralSection } from "./sections/GeneralSection";
import { AppearanceSection } from "./sections/AppearanceSection";
import { AiProvidersSection } from "./sections/AiProvidersSection";
import { TranslateSection } from "./sections/TranslateSection";
import { ActionsHotkeysSection } from "./sections/ActionsHotkeysSection";
import { AdvancedSection } from "./sections/AdvancedSection";
import "./settings.css";

const NAV: { id: SectionId; label: string; hint: string }[] = [
  { id: "general", label: "核心", hint: "语言 / 启动 / 历史" },
  { id: "appearance", label: "外观", hint: "主题 / 字号 / 结果窗" },
  { id: "ai", label: "AI 服务", hint: "供应商与连接测试" },
  { id: "translate", label: "翻译", hint: "云端 / 本地 / 术语表" },
  { id: "actions", label: "动作与热键", hint: "菜单 / 热键 / 搜索" },
  { id: "advanced", label: "高级", hint: "预算 / 代理 / 运行信息" },
];

const SECTION_ALIASES: Record<string, SectionId> = {
  general: "general",
  core: "general",
  appearance: "appearance",
  ui: "appearance",
  theme: "appearance",
  ai: "ai",
  providers: "ai",
  provider: "ai",
  translate: "translate",
  translation: "translate",
  actions: "actions",
  hotkeys: "actions",
  hotkey: "actions",
  advanced: "advanced",
  advance: "advanced",
};

function resolveSection(initial?: string): SectionId {
  if (!initial) return "general";
  return SECTION_ALIASES[initial.trim().toLowerCase()] ?? "general";
}

const STATUS_TEXT: Record<string, string> = {
  idle: "已同步",
  draft: "草稿 · 有未应用修改",
  ok: "已应用",
  error: "保存失败",
};

export interface SettingsPageProps {
  initialSection?: string;
}

export default function SettingsPage({ initialSection }: SettingsPageProps) {
  const [section, setSection] = useState<SectionId>(() => resolveSection(initialSection));
  const [resetting, setResetting] = useState(false);
  const {
    draft,
    baseline,
    loading,
    loadError,
    saving,
    status,
    statusText,
    error,
    reload,
    edit,
    apply,
    replace,
    reportError,
    clearError,
  } = useConfigDraft();

  const dirty = draft !== null && baseline !== null ? hasChanges(baseline, draft) : false;
  const tone = error ? "error" : dirty ? "draft" : status === "ok" ? "ok" : "idle";
  const statusLabel = error ? STATUS_TEXT.error : dirty ? STATUS_TEXT.draft : statusText ?? STATUS_TEXT[tone];

  const resetAll = async () => {
    setResetting(true);
    try {
      const view = await ipc.invoke<ConfigView>(COMMANDS.configReset, { scope: "all" });
      replace(view);
    } catch (err) {
      reportError(err);
    } finally {
      setResetting(false);
    }
  };

  if (!draft || !baseline) {
    return (
      <div className="settings-root">
        {loading ? (
          <LoadingBlock text="正在读取配置…" />
        ) : (
          <div className="settings-fallback">
            <ErrorNotice error={loadError} />
            <p className="sa-hint">无法读取配置，请确认后端已启动。</p>
            <Button variant="primary" size="sm" onClick={() => void reload()}>
              重新加载
            </Button>
          </div>
        )}
      </div>
    );
  }

  const sectionProps: SectionProps = {
    draft,
    baseline,
    saving,
    onEdit: edit,
    onApply: apply,
    onReplace: replace,
    onError: reportError,
  };

  return (
    <div className="settings-root">
      <header className="settings-header">
        <div className="settings-title">
          <h1 className="settings-title-text">设置</h1>
          <p className="settings-subtitle">框选 AI · Snip Ask</p>
        </div>
        <div className="settings-header-side">
          <span className={`settings-status is-${tone}`} title={statusText ?? undefined}>
            {statusLabel}
          </span>
          <ConfirmAction
            description="将把全部配置恢复为默认值，确定继续？"
            confirmLabel="确认恢复"
            disabled={resetting}
            onConfirm={() => void resetAll()}
          >
            恢复默认
          </ConfirmAction>
        </div>
      </header>

      {error ? <ErrorNotice error={error} onDismiss={clearError} /> : null}

      <div className="settings-main">
        <nav className="settings-nav" aria-label="设置分组">
          {NAV.map((item) => (
            <button
              key={item.id}
              type="button"
              className={`settings-nav-item${section === item.id ? " is-active" : ""}`}
              onClick={() => setSection(item.id)}
            >
              <span className="settings-nav-label">{item.label}</span>
              <span className="settings-nav-hint">{item.hint}</span>
            </button>
          ))}
          <p className="settings-nav-note">修改后需点击各分组底部的「应用」按钮保存。</p>
        </nav>

        <div className="settings-content">
          {section === "general" ? <GeneralSection key="general" {...sectionProps} /> : null}
          {section === "appearance" ? <AppearanceSection key="appearance" {...sectionProps} /> : null}
          {section === "ai" ? <AiProvidersSection key="ai" {...sectionProps} /> : null}
          {section === "translate" ? <TranslateSection key="translate" {...sectionProps} /> : null}
          {section === "actions" ? <ActionsHotkeysSection key="actions" {...sectionProps} /> : null}
          {section === "advanced" ? <AdvancedSection key="advanced" {...sectionProps} /> : null}
        </div>
      </div>
    </div>
  );
}
