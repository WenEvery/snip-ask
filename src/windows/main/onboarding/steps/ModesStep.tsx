/** 引导第 3 步：AI 模式 —— 四个动作 × 快速/深度，含时延与成本参考（报告 §3.2.6） */

import type { Mode } from "@/shared/contract/types";
import type { StepProps } from "../types";
import { DEFAULT_MODES, MODE_ROWS } from "../presets";

export function ModesStep(props: StepProps) {
  const { draft, onEdit } = props;
  const modes = draft.modes;

  const setMode = (key: (typeof MODE_ROWS)[number]["key"], mode: Mode) => {
    onEdit({ modes: { ...modes, [key]: mode } });
  };

  const setAll = (mode: Mode) => {
    onEdit({ modes: { ...DEFAULT_MODES, search: mode, imageSearch: mode, translate: mode, answer: mode } });
  };

  return (
    <div className="ob-step">
      <h2 className="ob-step-title">为每个动作选择默认模式</h2>
      <p className="ob-hint">
        同一入口可在动作菜单中临时切换。深度模式会开启思考、多次检索并抓取正文，时延与费用都更高，建议只对需要质量的动作开启。
      </p>

      <div className="ob-inline">
        <button type="button" className="ob-btn ob-btn-secondary" onClick={() => setAll("quick")}>
          全部快速
        </button>
        <button type="button" className="ob-btn ob-btn-secondary" onClick={() => setAll("deep")}>
          全部深度
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
                  <span className="ob-mode-name">快速</span>
                  <span className="ob-mode-meta">{row.quick}</span>
                </button>
                <button
                  type="button"
                  className={`ob-mode-btn${current === "deep" ? " is-active" : ""}`}
                  onClick={() => setMode(row.key, "deep")}
                >
                  <span className="ob-mode-name">深度</span>
                  <span className="ob-mode-meta">{row.deep}</span>
                </button>
              </div>
            </div>
          );
        })}
      </div>

      <p className="ob-hint">
        深度模式预算（检索次数 / 抓取篇数 / token 上限）可在设置 → 高级中自定义，默认为 3 / 2 / 1500。
      </p>
    </div>
  );
}
