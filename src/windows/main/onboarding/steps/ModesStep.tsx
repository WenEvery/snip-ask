/** 引导第 3 步：AI 模式 —— 四个动作 × 快速/深度，含时延与成本参考（报告 §3.2.6） */

import type { Mode } from "@/shared/contract/types";
import type { StepProps } from "../types";
import { DEFAULT_MODES, MODE_ROWS } from "../presets";
import { t } from "../../../../i18n";

export function ModesStep(props: StepProps) {
  const { draft, onEdit } = props;
  const modes = draft.modes;

  const setMode = (key: (typeof MODE_ROWS)[number]["key"], mode: Mode) => {
    onEdit({ modes: { ...modes, [key]: mode } });
  };

  const setAll = (mode: Mode) => {
    onEdit({
      modes: { ...DEFAULT_MODES, search: mode, imageSearch: mode, translate: mode, answer: mode },
    });
  };

  return (
    <div className="ob-step">
      <h2 className="ob-step-title">{t("为每个动作选择默认模式")}</h2>
      <p className="ob-hint">
        {t(
          "动作菜单里还能临时切换这一次的模式。深度模式会多查几次、多读几篇网页再回答，更慢、费用也更高，建议只对需要质量的动作开启。",
        )}
      </p>

      <div className="ob-inline">
        <button type="button" className="ob-btn ob-btn-secondary" onClick={() => setAll("quick")}>
          {t("全部快速")}
        </button>
        <button type="button" className="ob-btn ob-btn-secondary" onClick={() => setAll("deep")}>
          {t("全部深度")}
        </button>
      </div>

      <div className="ob-modes">
        {MODE_ROWS.map((row) => {
          const current = modes[row.key] ?? "quick";
          return (
            <div key={row.key} className="ob-mode-row">
              <div className="ob-mode-info">
                <span className="ob-mode-label">{row.label}</span>
                <span className="ob-hint">{row.description}</span>
              </div>
              <div className="ob-mode-options">
                <button
                  type="button"
                  className={`ob-mode-btn${current === "quick" ? " is-active" : ""}`}
                  onClick={() => setMode(row.key, "quick")}
                >
                  <span className="ob-mode-name">{t("快速")}</span>
                  <span className="ob-mode-meta">{row.quick}</span>
                </button>
                <button
                  type="button"
                  className={`ob-mode-btn${current === "deep" ? " is-active" : ""}`}
                  onClick={() => setMode(row.key, "deep")}
                >
                  <span className="ob-mode-name">{t("深度")}</span>
                  <span className="ob-mode-meta">{row.deep}</span>
                </button>
              </div>
            </div>
          );
        })}
      </div>

      <p className="ob-hint">
        {t(
          "深度模式每次最多搜索几次、读几篇网页、输出多长，可以在「设置 → 高级」中调整，默认是 3 次、2 篇、约 1500 字。",
        )}
      </p>
    </div>
  );
}
