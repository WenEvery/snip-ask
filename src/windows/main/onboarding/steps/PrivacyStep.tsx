/** 引导第 5 步：隐私与完成 —— 上传开关、隐私同意、数据目录说明与配置摘要 */

import type { Mode } from "@/shared/contract/types";
import type { StepProps } from "../types";
import { MODE_ROWS, ONBOARDING_PROVIDERS } from "../presets";

const THEME_LABELS: Record<string, string> = {
  system: "跟随系统",
  light: "浅色",
  dark: "深色",
};

const POSITION_LABELS: Record<string, string> = {
  follow: "跟随选区",
  fixed: "固定位置",
};

const MODE_LABELS: Record<Mode, string> = { quick: "快速", deep: "深度" };

export function PrivacyStep(props: StepProps) {
  const { draft, onEdit } = props;
  const privacy = draft.privacy;

  const updatePrivacy = (patch: Partial<typeof privacy>) =>
    onEdit({ privacy: { ...privacy, ...patch } });

  const provider = draft.ai
    ? (ONBOARDING_PROVIDERS.find((item) => item.id === draft.ai?.providerId)?.name ??
      draft.ai.providerId)
    : null;
  const aiSummary = draft.ai
    ? `${provider ?? "自定义"} / ${draft.ai.model || "未填写模型"}${draft.ai.tested ? "（已测试）" : ""}`
    : "未配置（部分功能不可用）";
  const translateSummary = draft.translate
    ? draft.translate.backend === "cloud"
      ? `云端${draft.translate.cloudModel ? `（${draft.translate.cloudModel}）` : "（复用 AI 服务）"}`
      : `本地${draft.translate.modelId ? `（${draft.translate.modelId}）` : "（未选择模型）"}`
    : "云端（复用 AI 服务）";
  const modesSummary = MODE_ROWS.map(
    (row) => `${row.label} ${MODE_LABELS[draft.modes[row.key] ?? "quick"]}`,
  ).join(" · ");
  const uiSummary = `${THEME_LABELS[draft.ui.theme] ?? draft.ui.theme} · 字号 ${draft.ui.fontSize} · 结果窗${
    POSITION_LABELS[draft.ui.resultPosition] ?? draft.ui.resultPosition
  } · 开机自启${draft.ui.autostart ? "开" : "关"}`;

  return (
    <div className="ob-step">
      <h2 className="ob-step-title">隐私与完成</h2>
      <p className="ob-hint">
        框选 AI
        默认只在本机处理；只有你主动触发云端动作时，才会把对应文本/图片发送给你自己配置的模型服务。
      </p>

      <div className="ob-block">
        <h3 className="ob-block-title">上传开关</h3>
        <label className="ob-checkline">
          <input
            type="checkbox"
            checked={privacy.allowTextUpload}
            onChange={(event) => updatePrivacy({ allowTextUpload: event.target.checked })}
          />
          <span>
            <strong>允许把识别到的文本发送给云端模型</strong>
            <span className="ob-hint">关闭后文本搜索 / 解答 / 云端翻译将不可用。</span>
          </span>
        </label>
        <label className="ob-checkline">
          <input
            type="checkbox"
            checked={privacy.allowImageUpload}
            onChange={(event) => updatePrivacy({ allowImageUpload: event.target.checked })}
          />
          <span>
            <strong>允许把截图发送给视觉模型</strong>
            <span className="ob-hint">图片搜索与看图解答需要；关闭时这些动作禁用并说明原因。</span>
          </span>
        </label>
      </div>

      <div className="ob-block">
        <h3 className="ob-block-title">知情同意</h3>
        <label className="ob-checkline">
          <input
            type="checkbox"
            checked={privacy.agreed}
            onChange={(event) => updatePrivacy({ agreed: event.target.checked })}
          />
          <span>
            <strong>我已阅读并同意隐私声明与开源许可（Apache-2.0）</strong>
            <span className="ob-hint">不同意也可以直接跳过引导，先使用本机功能。</span>
          </span>
        </label>
        {privacy.agreed ? null : (
          <p className="ob-warn">勾选同意后才能「完成并进入」；也可以直接跳过引导。</p>
        )}
      </div>

      <div className="ob-block">
        <h3 className="ob-block-title">数据目录与自定义</h3>
        <p className="ob-line">
          <span>数据目录</span>
          <code className="ob-code">%APPDATA%\SnipAsk</code>
        </p>
        <p className="ob-line">
          <span>自定义</span>
          <span>提示词 / 术语表 / 配置 / 模型 / 本地引擎均可按《自定义指南》修改</span>
        </p>
        <p className="ob-line">
          <span>更新方式</span>
          <span>本软件不会自动更新：升级请手动下载新安装包覆盖安装（配置与历史保留）</span>
        </p>
      </div>

      <div className="ob-block">
        <h3 className="ob-block-title">配置摘要</h3>
        <p className="ob-line">
          <span>AI 服务</span>
          <span>{aiSummary}</span>
        </p>
        <p className="ob-line">
          <span>翻译方式</span>
          <span>{translateSummary}</span>
        </p>
        <p className="ob-line">
          <span>默认模式</span>
          <span>{modesSummary}</span>
        </p>
        <p className="ob-line">
          <span>外观与启动</span>
          <span>{uiSummary}</span>
        </p>
        <p className="ob-line">
          <span>框选热键</span>
          <span>{draft.hotkeys.capture || "未绑定"}</span>
        </p>
      </div>

      <p className="ob-privacy">
        完成后就可以开始框选；如果跳过了关键配置，部分功能会暂时不可用，随时可以在设置里补齐。
      </p>
    </div>
  );
}
