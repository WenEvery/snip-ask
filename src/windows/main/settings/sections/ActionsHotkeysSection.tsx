/** 设置 · 动作与热键：菜单顺序、各动作默认模式、9 项热键 */

import { Button } from "@/components/ui";
import type { ActionId, Hotkeys, Mode } from "@/shared/contract/types";
import { t } from "@/i18n";
import { buildPatch, hasChanges } from "../diff";
import { HOTKEY_ROWS } from "../hotkey";
import { ACTION_LABELS, DEFAULT_HOTKEYS, IMAGE_MENU_ACTIONS, TEXT_MENU_ACTIONS } from "../presets";
import {
  Hint,
  Panel,
  SaveBar,
  SectionShell,
  SegmentedControl,
  SelectField,
  SwitchRow,
} from "../parts";
import { HotkeyField } from "../components/HotkeyField";
import { MenuOrderEditor } from "../components/MenuOrderEditor";
import type { SectionProps } from "../types";

/** `modes.*` 四个动作的默认模式（与引导第 4 步一致）。 */
const MODE_ACTIONS: {
  key: "search" | "imageSearch" | "translate" | "answer";
  label: string;
  description: string;
}[] = [
  { key: "search", label: t("搜索"), description: t("框选文字后联网搜索并给出结论") },
  {
    key: "imageSearch",
    label: t("图片搜索"),
    description: t("识别图片内容后搜索来源"),
  },
  { key: "translate", label: t("翻译"), description: t("把选中文字翻译成目标语言") },
  { key: "answer", label: t("解答"), description: t("解释、分析或改写选中内容") },
];

export function ActionsHotkeysSection(props: SectionProps) {
  const { draft, baseline, saving, onEdit, onApply, onError } = props;
  const actions = draft.actions;
  const modes = draft.modes;
  const dirty = hasChanges(baseline, draft, ["actions", "hotkeys", "modes"]);

  const menuActions = Array.from(new Set<ActionId>([...actions.menu.text, ...actions.menu.image]));

  const updateHotkey = (key: keyof Hotkeys, spec: string) => {
    onEdit((current) => ({ ...current, hotkeys: { ...current.hotkeys, [key]: spec } }));
  };

  const changeMenu = (target: "text" | "image", next: ActionId[]) => {
    onEdit((current) => ({
      ...current,
      actions: { ...current.actions, menu: { ...current.actions.menu, [target]: next } },
    }));
  };

  const setMode = (key: (typeof MODE_ACTIONS)[number]["key"], mode: Mode) => {
    onEdit((current) => ({ ...current, modes: { ...current.modes, [key]: mode } }));
  };

  const save = async () => {
    const patch = buildPatch(baseline, draft, ["actions", "hotkeys", "modes"]);
    if (Object.keys(patch).length === 0) return;
    await onApply(patch, t("动作与热键"));
  };

  return (
    <SectionShell
      title={t("动作与热键")}
      description={t("挑选框选后显示哪些动作、每个动作默认用多快的方式回答，并设置全局热键。")}
    >
      <Panel
        title={t("动作菜单")}
        description={t("菜单从上到下按这里的顺序显示；取消勾选即可隐藏该动作。")}
        actions={
          <Button
            variant="ghost"
            size="sm"
            onClick={() =>
              onEdit((current) => ({
                ...current,
                actions: {
                  ...current.actions,
                  menu: { text: [...TEXT_MENU_ACTIONS], image: [...IMAGE_MENU_ACTIONS] },
                },
              }))
            }
          >
            {t("恢复默认菜单")}
          </Button>
        }
      >
        <MenuOrderEditor
          title={t("框选到文字")}
          description={t("选中文字时出现的动作")}
          candidates={TEXT_MENU_ACTIONS}
          value={actions.menu.text}
          onChange={(next) => changeMenu("text", next)}
        />
        <MenuOrderEditor
          title={t("框选到图片")}
          description={t("选中图片时出现的动作")}
          candidates={IMAGE_MENU_ACTIONS}
          value={actions.menu.image}
          onChange={(next) => changeMenu("image", next)}
        />
      </Panel>

      <Panel
        title={t("每个动作的默认模式")}
        description={t(
          "快速模式几秒内出结果、费用低；深度模式会多搜索几次、读更多内容，更慢也更贵。",
        )}
      >
        <div className="sa-action-modes">
          {MODE_ACTIONS.map((row) => (
            <div key={row.key} className="sa-action-mode-row">
              <div className="sa-action-mode-info">
                <span className="sa-field-label">{row.label}</span>
                <span className="sa-field-hint">{row.description}</span>
              </div>
              <SegmentedControl<Mode>
                value={modes[row.key]}
                onChange={(mode) => setMode(row.key, mode)}
                options={[
                  { value: "quick", label: t("快速") },
                  { value: "deep", label: t("深度") },
                ]}
              />
            </div>
          ))}
        </div>
        <Hint>{t("在框选后的动作菜单里还能临时切换这一次的模式。")}</Hint>
      </Panel>

      <Panel title={t("其他行为")}>
        <div className="sa-grid-2">
          <SelectField
            label={t("自动执行的动作")}
            value={actions.defaultAction ?? ""}
            options={[
              { value: "", label: t("不自动执行（每次手动选）") },
              ...menuActions.map((action) => ({ value: action, label: ACTION_LABELS[action] })),
            ]}
            onChange={(value) =>
              onEdit((current) => ({
                ...current,
                actions: {
                  ...current.actions,
                  defaultAction: menuActions.includes(value as ActionId)
                    ? (value as ActionId)
                    : null,
                },
              }))
            }
            hint={t("可选当前菜单中已有的动作")}
          />
          <div className="sa-field is-stacked">
            <span className="sa-field-label">{t("结果处理")}</span>
            <SwitchRow
              label={t("自动复制结果")}
              description={t("结果生成后自动复制到剪贴板")}
              checked={actions.autoCopy}
              onChange={(autoCopy) =>
                onEdit((current) => ({ ...current, actions: { ...current.actions, autoCopy } }))
              }
            />
          </div>
        </div>
      </Panel>

      <Panel
        title={t("全局热键")}
        description={t(
          "点击热键框后按下组合键（至少一个修饰键 + 字母/数字）；留空表示不使用。图片搜索/解答选区按下后会先进入框选，选完自动执行对应动作。",
        )}
        actions={
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onEdit((current) => ({ ...current, hotkeys: { ...DEFAULT_HOTKEYS } }))}
          >
            {t("恢复全部默认")}
          </Button>
        }
      >
        {HOTKEY_ROWS.map((row) => (
          <HotkeyField
            key={row.key}
            label={row.label}
            description={row.description}
            value={draft.hotkeys[row.key]}
            defaultValue={DEFAULT_HOTKEYS[row.key]}
            onChange={(spec) => updateHotkey(row.key, spec)}
            onError={onError}
          />
        ))}
        <Hint>{t("如果热键被其他软件占用，这里会显示冲突提示，换一个组合即可。")}</Hint>
      </Panel>

      <SaveBar label={t("动作与热键")} dirty={dirty} saving={saving} onSave={() => void save()} />
    </SectionShell>
  );
}
