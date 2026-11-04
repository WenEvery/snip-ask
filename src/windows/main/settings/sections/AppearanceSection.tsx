/** 设置 · 外观：主题、强调色、字号、结果窗显示参数（即时预览 + 应用） */

import type { FontSize, ResultPosition, Theme } from "@/shared/contract/types";
import { buildPatch, hasChanges } from "../diff";
import {
  NumberField,
  Panel,
  SaveBar,
  SectionShell,
  SegmentedControl,
  SliderField,
  SwitchRow,
} from "../parts";
import { previewAccent, previewFontSize, previewTheme } from "../themePreview";
import type { SectionProps } from "../types";

const ACCENTS: { value: string; label: string }[] = [
  { value: "blue", label: "科技蓝" },
  { value: "green", label: "极光绿" },
  { value: "purple", label: "质子紫" },
  { value: "orange", label: "活力橙" },
];

export function AppearanceSection(props: SectionProps) {
  const { draft, baseline, saving, onEdit, onApply } = props;
  const ui = draft.ui;
  const dirty = hasChanges(baseline, draft, ["ui"]);

  const changeTheme = (theme: Theme) => {
    previewTheme(theme);
    onEdit((current) => ({ ...current, ui: { ...current.ui, theme } }));
  };

  const changeAccent = (accent: string) => {
    previewAccent(accent);
    onEdit((current) => ({ ...current, ui: { ...current.ui, accent } }));
  };

  const changeFontSize = (fontSize: FontSize) => {
    previewFontSize(fontSize);
    onEdit((current) => ({ ...current, ui: { ...current.ui, fontSize } }));
  };

  const updateResultWindow = (patch: Partial<typeof ui.resultWindow>) => {
    onEdit((current) => ({
      ...current,
      ui: { ...current.ui, resultWindow: { ...current.ui.resultWindow, ...patch } },
    }));
  };

  const save = async () => {
    const patch = buildPatch(baseline, draft, ["ui"]);
    if (Object.keys(patch).length === 0) return;
    await onApply(patch, "外观");
  };

  return (
    <SectionShell
      title="外观"
      description="主题与字号即时预览；点「应用本组设置」后写入配置并对所有窗口生效。"
    >
      <Panel title="主题">
        <SegmentedControl<Theme>
          value={ui.theme}
          onChange={changeTheme}
          options={[
            { value: "system", label: "跟随系统", description: "随系统深浅色自动切换" },
            { value: "light", label: "浅色" },
            { value: "dark", label: "深色" },
          ]}
        />
        <div className="sa-field is-stacked">
          <span className="sa-field-label">强调色</span>
          <div className="sa-swatches">
            {ACCENTS.map((accent) => (
              <button
                key={accent.value}
                type="button"
                className={`sa-swatch sa-swatch-${accent.value}${
                  ui.accent === accent.value ? " is-active" : ""
                }`}
                title={accent.label}
                onClick={() => changeAccent(accent.value)}
              >
                <span className="sa-swatch-dot" />
                <span className="sa-swatch-label">{accent.label}</span>
              </button>
            ))}
          </div>
        </div>
        <SegmentedControl<FontSize>
          value={ui.fontSize}
          onChange={changeFontSize}
          options={[
            { value: "sm", label: "小字号" },
            { value: "md", label: "中字号" },
            { value: "lg", label: "大字号" },
          ]}
        />
      </Panel>

      <Panel title="结果窗" description="控制流式结果窗出现的位置、透明度与尺寸。">
        <SegmentedControl<ResultPosition>
          value={ui.resultWindow.position}
          onChange={(position) => updateResultWindow({ position })}
          options={[
            { value: "follow", label: "跟随选区", description: "贴近框选位置弹出" },
            { value: "fixed", label: "固定位置", description: "固定在屏幕右下角" },
          ]}
        />
        <SwitchRow
          label="结果窗置顶"
          description="始终显示在其他窗口之上"
          checked={ui.resultWindow.pin}
          onChange={(pin) => updateResultWindow({ pin })}
        />
        <SliderField
          label="透明度"
          value={ui.resultWindow.opacity}
          min={0.3}
          max={1}
          step={0.05}
          hint="范围 0.3 ~ 1.0，数值越小越透明"
          format={(value) => `${Math.round(value * 100)}%`}
          onChange={(opacity) => updateResultWindow({ opacity: Math.round(opacity * 100) / 100 })}
        />
        <div className="sa-grid-2">
          <NumberField
            label="窗口宽度"
            value={ui.resultWindow.width}
            min={260}
            max={900}
            step={10}
            suffix="px"
            onChange={(width) => updateResultWindow({ width })}
          />
          <NumberField
            label="窗口高度"
            value={ui.resultWindow.height}
            min={240}
            max={800}
            step={10}
            suffix="px"
            onChange={(height) => updateResultWindow({ height })}
          />
        </div>
      </Panel>

      <SaveBar label="外观" dirty={dirty} saving={saving} onSave={() => void save()} />
    </SectionShell>
  );
}
