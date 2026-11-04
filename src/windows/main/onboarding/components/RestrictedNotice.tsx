/**
 * S-07 受限模式提示：引导完成页 / 主界面共用。
 *
 * - `variant="inline"`：显示在引导第 6 步内容区，说明完成后将进入受限模式；
 * - `variant="banner"`：显示在主界面顶部，说明当前受限状态并提供「去配置」入口。
 */

import type { ReactNode } from "react";
import type { Capabilities } from "../capabilities";
import { availableLabels, unavailableLabels } from "../capabilities";

export interface RestrictedNoticeProps {
  capabilities: Capabilities;
  variant?: "inline" | "banner";
  skipped?: boolean;
  onConfigure?: () => void;
  children?: ReactNode;
}

const COLORS = {
  bg: "rgba(245, 158, 11, 0.10)",
  border: "rgba(245, 158, 11, 0.45)",
  accent: "#b45309",
};

export function RestrictedNotice({
  capabilities,
  variant = "inline",
  skipped = false,
  onConfigure,
  children,
}: RestrictedNoticeProps) {
  const available = availableLabels(capabilities).join("、");
  const unavailable = unavailableLabels(capabilities).join("、");
  const title = skipped ? "已跳过引导 · 进入受限模式" : "受限模式";
  const intro = skipped
    ? "你没有配置 AI 服务，应用会以受限模式运行，之后可随时在设置页补齐。"
    : "当前未配置任何 AI 服务（无 API Key、无本地模型），应用将以受限模式运行。";

  return (
    <section
      role="status"
      aria-label="受限模式提示"
      data-testid="restricted-notice"
      style={{
        display: "flex",
        gap: variant === "banner" ? 12 : 10,
        alignItems: "flex-start",
        justifyContent: "space-between",
        flexWrap: "wrap",
        margin: variant === "banner" ? "12px 0 0" : "12px 0 0",
        padding: variant === "banner" ? "10px 14px" : "12px 14px",
        border: `1px solid ${COLORS.border}`,
        borderLeft: `4px solid ${COLORS.accent}`,
        borderRadius: 8,
        background: COLORS.bg,
        lineHeight: 1.6,
      }}
    >
      <div style={{ minWidth: 240, flex: "1 1 320px" }}>
        <strong style={{ color: COLORS.accent }}>{title}</strong>
        <p style={{ margin: "4px 0 0" }}>{intro}</p>
        <p style={{ margin: "2px 0 0" }}>
          <strong>仍可用：</strong>
          {available}
        </p>
        <p style={{ margin: "2px 0 0" }}>
          <strong>不可用：</strong>
          {unavailable}（原因与修复入口见动作菜单 / 设置页）
        </p>
        {children}
      </div>
      {onConfigure ? (
        <button
          type="button"
          onClick={onConfigure}
          style={{
            flex: "0 0 auto",
            alignSelf: "center",
            padding: "6px 14px",
            borderRadius: 6,
            border: `1px solid ${COLORS.accent}`,
            background: "transparent",
            color: COLORS.accent,
            cursor: "pointer",
            fontWeight: 600,
          }}
        >
          去配置 AI 服务
        </button>
      ) : null}
    </section>
  );
}
