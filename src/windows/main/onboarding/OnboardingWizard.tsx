/**
 * 首启引导外壳：6 步状态机。
 *
 * - 挂载 `onboarding_state` → 草稿初始化；
 * - 上一步/下一步时 `onboarding_update({ step, data })` 保存当前步骤字段（失败提示但不阻塞）；
 * - 跳过：二次确认 → `onboarding_finish({ skip: true })`；
 * - 完成：隐私同意后才可 `onboarding_finish({ skip: false })`。
 */

import { useEffect, useState } from "react";
import type {
  ConfigView,
  ModelInfo,
  OnboardingDraft,
  OnboardingState,
} from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { t } from "../../../i18n";
import { toUiError } from "./error";
import type { Capabilities } from "./capabilities";
import { deriveCapabilities, isRestricted } from "./capabilities";
import type { StepProps, UiError } from "./types";
import { STEP_COUNT, STEP_META } from "./types";
import { WelcomeStep } from "./steps/WelcomeStep";
import { AiServiceStep } from "./steps/AiServiceStep";
import { TranslateStep } from "./steps/TranslateStep";
import { ModesStep } from "./steps/ModesStep";
import { AppearanceStep } from "./steps/AppearanceStep";
import { PrivacyStep } from "./steps/PrivacyStep";
import { RestrictedNotice } from "./components/RestrictedNotice";
import "./onboarding.css";

export interface OnboardingWizardProps {
  onFinish: (skip: boolean) => void;
  /** 「去配置」入口（跳到设置 → AI 服务）；无渲染宿主时可省略。 */
  onOpenSettings?: () => void;
}

/** 每步保存到 `onboarding_update.data` 的字段（与契约 §9 OnboardingDraft 对应） */
function buildStepData(step: number, draft: OnboardingDraft): Record<string, unknown> {
  switch (step) {
    case 0:
      return { language: draft.language };
    case 1:
      return { ai: draft.ai };
    case 2:
      return { translate: draft.translate };
    case 3:
      return { modes: draft.modes };
    case 4:
      return { ui: draft.ui, hotkeys: draft.hotkeys };
    case 5:
      return { privacy: draft.privacy };
    default:
      return {};
  }
}

function renderStep(step: number, props: StepProps) {
  switch (step) {
    case 0:
      return <WelcomeStep {...props} />;
    case 1:
      return <AiServiceStep {...props} />;
    case 2:
      return <TranslateStep {...props} />;
    case 3:
      return <ModesStep {...props} />;
    case 4:
      return <AppearanceStep {...props} />;
    case 5:
      return <PrivacyStep {...props} />;
    default:
      return null;
  }
}

export default function OnboardingWizard({ onFinish, onOpenSettings }: OnboardingWizardProps) {
  const [draft, setDraft] = useState<OnboardingDraft | null>(null);
  const [step, setStep] = useState(0);
  const [loadError, setLoadError] = useState<UiError | null>(null);
  const [error, setError] = useState<UiError | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmSkip, setConfirmSkip] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  // 真实能力快照：用于第 6 步「完成后将进入受限模式」的明确说明。
  const [capabilities, setCapabilities] = useState<Capabilities | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      setLoadError(null);
      try {
        const state = await ipc.invoke<OnboardingState>(COMMANDS.onboardingState);
        if (cancelled) return;
        setDraft(state.data);
        setStep(Math.min(Math.max(state.currentStep, 0), STEP_COUNT - 1));
      } catch (err) {
        if (cancelled) return;
        setLoadError(toUiError(err));
      }
      // 能力快照失败不阻塞引导（引导本身仍可完成）。
      try {
        const [view, models] = await Promise.all([
          ipc.invoke<ConfigView>(COMMANDS.configGet),
          ipc.invoke<ModelInfo[]>(COMMANDS.modelsList),
        ]);
        if (!cancelled) {
          setCapabilities(deriveCapabilities({ providers: view.providers, models }));
        }
      } catch {
        /* 忽略：受限说明降级为草稿摘要 */
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [reloadKey]);

  const edit = (patch: Partial<OnboardingDraft>) => {
    setDraft((current) => (current ? { ...current, ...patch } : current));
  };

  const persistStep = async (currentStep: number, currentDraft: OnboardingDraft) => {
    try {
      await ipc.invoke<OnboardingState>(COMMANDS.onboardingUpdate, {
        step: currentStep,
        data: buildStepData(currentStep, currentDraft),
      });
    } catch (err) {
      // 报告 §2.4.5：即时保存失败只提示，不阻塞流程
      setError(toUiError(err));
    }
  };

  const goPrev = async () => {
    if (!draft || step === 0) return;
    setBusy(true);
    await persistStep(step, draft);
    setBusy(false);
    setStep(step - 1);
  };

  const goNext = async () => {
    if (!draft || step >= STEP_COUNT - 1) return;
    setBusy(true);
    await persistStep(step, draft);
    setBusy(false);
    setStep(step + 1);
  };

  const finish = async (skip: boolean) => {
    if (!draft) return;
    if (!skip && !draft.privacy.agreed) {
      setError({
        code: "SA-5001",
        message: t("请先勾选同意隐私声明，或选择跳过引导。"),
        detail: null,
        retryable: false,
        fallback: [],
      });
      return;
    }
    setBusy(true);
    setError(null);
    try {
      if (!skip) await persistStep(step, draft);
      await ipc.invoke<void>(COMMANDS.onboardingFinish, { skip });
      onFinish(skip);
    } catch (err) {
      setError(toUiError(err));
    } finally {
      setBusy(false);
    }
  };

  if (!draft) {
    return (
      <div className="ob-root">
        <div className="ob-fallback">
          {loadError ? (
            <>
              <p className="ob-error-message">{loadError.message}</p>
              <button
                type="button"
                className="ob-btn ob-btn-primary"
                onClick={() => setReloadKey(reloadKey + 1)}
              >
                {t("重新加载")}
              </button>
            </>
          ) : (
            <p className="ob-hint">{t("正在读取引导状态…")}</p>
          )}
        </div>
      </div>
    );
  }

  const meta = STEP_META[step] ?? STEP_META[0];
  const stepProps: StepProps = {
    draft,
    onEdit: edit,
    onError: (err) => setError(toUiError(err)),
  };
  const isLast = step === STEP_COUNT - 1;
  const canFinish = draft.privacy.agreed && !busy;

  return (
    <div className="ob-root">
      <header className="ob-header">
        <div className="ob-header-title">
          <span className="ob-brand">{t("框选 AI")}</span>
          <span className="ob-step-counter">
            {t("第 {step} / {total} 步 · {title}", {
              step: step + 1,
              total: STEP_COUNT,
              title: meta.title,
            })}
          </span>
        </div>
        <div className="ob-progress" aria-label={t("引导进度")}>
          {STEP_META.map((item, index) => (
            <span
              key={item.title}
              className={`ob-dot${index === step ? " is-current" : ""}${index < step ? " is-done" : ""}`}
              title={item.title}
            />
          ))}
        </div>
      </header>

      {error ? (
        <div className="ob-error" role="alert">
          <strong className="ob-error-message">{error.message}</strong>
          <span className="ob-error-code">{error.code}</span>
          {error.detail ? <code className="ob-error-detail">{error.detail}</code> : null}
          <button type="button" className="ob-linkbtn" onClick={() => setError(null)}>
            {t("关闭")}
          </button>
        </div>
      ) : null}

      <div className="ob-content">
        {renderStep(step, stepProps)}
        {isLast && capabilities && isRestricted(capabilities) ? (
          <RestrictedNotice
            capabilities={capabilities}
            skipped={false}
            onConfigure={onOpenSettings}
          >
            <p style={{ margin: "2px 0 0" }}>
              {t("完成后可随时在「设置 → AI 服务 / 翻译」补齐；补齐后云动作会自动恢复可用。")}
            </p>
          </RestrictedNotice>
        ) : null}
      </div>

      <footer className="ob-footer">
        <div className="ob-footer-left">
          {confirmSkip ? (
            <span className="ob-inline">
              <span className="ob-warn-inline">
                {t(
                  "跳过之后，AI 搜索、解答、图片搜索和云端翻译暂时用不了；本地识别、复制和浏览器搜索仍然可用，随时可以在设置里补齐。确定跳过吗？",
                )}
              </span>
              <button
                type="button"
                className="ob-btn ob-btn-danger"
                disabled={busy}
                onClick={() => void finish(true)}
              >
                {t("确认跳过")}
              </button>
              <button
                type="button"
                className="ob-btn ob-btn-ghost"
                onClick={() => setConfirmSkip(false)}
              >
                {t("继续配置")}
              </button>
            </span>
          ) : (
            <button type="button" className="ob-linkbtn" onClick={() => setConfirmSkip(true)}>
              {t("跳过引导")}
            </button>
          )}
        </div>
        <div className="ob-footer-right">
          <button
            type="button"
            className="ob-btn ob-btn-ghost"
            disabled={step === 0 || busy}
            onClick={() => void goPrev()}
          >
            {t("上一步")}
          </button>
          {isLast ? (
            <button
              type="button"
              className="ob-btn ob-btn-primary"
              disabled={!canFinish}
              title={canFinish ? undefined : t("需勾选同意隐私声明")}
              onClick={() => void finish(false)}
            >
              {busy ? t("完成中…") : t("完成并进入")}
            </button>
          ) : (
            <button
              type="button"
              className="ob-btn ob-btn-primary"
              disabled={busy}
              onClick={() => void goNext()}
            >
              {busy ? t("保存中…") : t("下一步")}
            </button>
          )}
        </div>
      </footer>
    </div>
  );
}
