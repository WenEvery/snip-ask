/** 引导第 4 步：外观与行为 —— 主题、字号、结果窗位置、开机自启、热键录制与冲突检测 */

import type { FontSize, ResultPosition, Theme } from "@/shared/contract/types";
import { HotkeyRecorder } from "../components/HotkeyRecorder";
import type { StepProps } from "../types";

const THEMES: { value: Theme; label: string }[] = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
];

const FONT_SIZES: { value: FontSize; label: string; hint: string }[] = [
  { value: "sm", label: "小", hint: "紧凑，适合小屏" },
  { value: "md", label: "中", hint: "默认" },
  { value: "lg", label: "大", hint: "更易读" },
];

const POSITIONS: { value: ResultPosition; label: string; hint: string }[] = [
  { value: "follow", label: "跟随选区", hint: "贴近框选位置" },
  { value: "fixed", label: "固定位置", hint: "固定在右下角" },
];

export function AppearanceStep(props: StepProps) {
  const { draft, onEdit, onError } = props;
  const { ui, hotkeys } = draft;

  const updateUi = (patch: Partial<typeof ui>) => onEdit({ ui: { ...ui, ...patch } });

  const selectTheme = (theme: Theme) => {
    updateUi({ theme });
    if (typeof document !== "undefined") document.documentElement.dataset.theme = theme;
  };

  const selectFontSize = (fontSize: FontSize) => {
    updateUi({ fontSize });
    if (typeof document !== "undefined") document.documentElement.dataset.fontSize = fontSize;
  };

  return (
    <div className="ob-step">
      <h2 className="ob-step-title">外观与行为</h2>
      <p className="ob-hint">这些设置保存后立即生效，之后可随时在设置中调整。</p>

      <div className="ob-block">
        <h3 className="ob-block-title">主题</h3>
        <div className="ob-option-row">
          {THEMES.map((theme) => (
            <button
              key={theme.value}
              type="button"
              className={`ob-option${ui.theme === theme.value ? " is-selected" : ""}`}
              onClick={() => selectTheme(theme.value)}
            >
              {theme.label}
            </button>
          ))}
        </div>
      </div>

      <div className="ob-block">
        <h3 className="ob-block-title">字号</h3>
        <div className="ob-option-row">
          {FONT_SIZES.map((size) => (
            <button
              key={size.value}
              type="button"
              className={`ob-option${ui.fontSize === size.value ? " is-selected" : ""}`}
              title={size.hint}
              onClick={() => selectFontSize(size.value)}
            >
              {size.label}
            </button>
          ))}
        </div>
      </div>

      <div className="ob-block">
        <h3 className="ob-block-title">结果窗位置</h3>
        <div className="ob-option-row">
          {POSITIONS.map((position) => (
            <button
              key={position.value}
              type="button"
              className={`ob-option${ui.resultPosition === position.value ? " is-selected" : ""}`}
              onClick={() => updateUi({ resultPosition: position.value })}
            >
              {position.label}
              <span className="ob-option-hint">{position.hint}</span>
            </button>
          ))}
        </div>
        <p className="ob-hint">结果窗置顶、透明度、尺寸可在设置 → 外观中调整；启动时最小化在设置 → 核心中调整。</p>
      </div>

      <div className="ob-block">
        <label className="ob-checkline">
          <input
            type="checkbox"
            checked={ui.autostart}
            onChange={(event) => updateUi({ autostart: event.target.checked })}
          />
          <span>
            <strong>开机自动启动</strong>
            <span className="ob-hint">随 Windows 登录启动并常驻托盘（注册失败不影响完成引导）。</span>
          </span>
        </label>
      </div>

      <div className="ob-block">
        <h3 className="ob-block-title">框选热键</h3>
        <HotkeyRecorder
          value={hotkeys.capture}
          onChange={(spec) => onEdit({ hotkeys: { capture: spec } })}
          onError={onError}
        />
        <p className="ob-hint">按下的组合键会注册为全局热键；若与系统或其他软件冲突，检测结果会给出占用提示。</p>
      </div>
    </div>
  );
}
