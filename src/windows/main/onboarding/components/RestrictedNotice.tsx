/**
 * 「部分功能暂不可用」提示：引导完成页 / 主界面共用。
 *
 * - `variant="inline"`：显示在引导最后一步，说明完成后哪些功能还不能用；
 * - `variant="banner"`：显示在主界面顶部，并给出「去配置」入口。
 */

import type { ReactNode } from "react";
import type { Capabilities } from "../capabilities";
import { availableLabels, unavailableLabels } from "../capabilities";
import { t } from "../../../../i18n";

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
  const available = availableLabels(capabilities).join(t("、"));
  const unavailable = unavailableLabels(capabilities).join(t("、"));
  const title = skipped ? t("已跳过引导，部分功能暂不可用") : t("部分功能暂不可用");
  const intro = skipped
    ? t("你还没有配置 AI 服务，所以需要联网的 AI 功能暂时用不了；随时可以在设置里补上。")
    : t("还没有配置任何 AI 服务，需要联网的 AI 功能暂时用不了。");

  return (
    <section
      role="status"
      aria-label={t("功能可用性提示")}
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
          <strong>{t("现在可用：")}</strong>
          {available}
        </p>
        <p style={{ margin: "2px 0 0" }}>
          <strong>{t("暂时不可用：")}</strong>
          {unavailable}
          {t("（可以在设置中补齐）")}
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
          {t("去配置 AI 服务")}
        </button>
      ) : null}
    </section>
  );
}
