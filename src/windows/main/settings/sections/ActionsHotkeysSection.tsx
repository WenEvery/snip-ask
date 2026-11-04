/** 设置 · 动作与热键：菜单顺序、默认动作/模式、9 项热键、搜索方式与搜索引擎 */

import { Button } from "@/components/ui";
import type { ActionId, Hotkeys, Mode, SearchMode } from "@/shared/contract/types";
import { buildPatch, hasChanges } from "../diff";
import { HOTKEY_ROWS } from "../hotkey";
import {
  ACTION_LABELS,
  DEFAULT_HOTKEYS,
  IMAGE_MENU_ACTIONS,
  SEARCH_ENGINES,
  TEXT_MENU_ACTIONS,
} from "../presets";
import {
  Hint,
  Panel,
  SaveBar,
  SectionShell,
  SegmentedControl,
  SelectField,
  SwitchRow,
  TextInput,
} from "../parts";
import { HotkeyField } from "../components/HotkeyField";
import { MenuOrderEditor } from "../components/MenuOrderEditor";
import type { SectionProps } from "../types";

export function ActionsHotkeysSection(props: SectionProps) {
  const { draft, baseline, saving, onEdit, onApply, onError } = props;
  const actions = draft.actions;
  const search = draft.search;
  const dirty = hasChanges(baseline, draft, ["actions", "hotkeys", "search"]);

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

  const save = async () => {
    const patch = buildPatch(baseline, draft, ["actions", "hotkeys", "search"]);
    if (Object.keys(patch).length === 0) return;
    await onApply(patch, "动作与热键");
  };

  return (
    <SectionShell
      title="动作与热键"
      description="调整框选后动作菜单的显示顺序、默认行为，并录制全局热键（保存后由后端重新注册）。"
    >
      <Panel
        title="动作菜单顺序"
        description="菜单按数组顺序展示；取消勾选即隐藏该动作。"
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
            恢复默认菜单
          </Button>
        }
      >
        <MenuOrderEditor
          title="文本选区"
          description="框选到文字时展示的动作"
          candidates={TEXT_MENU_ACTIONS}
          value={actions.menu.text}
          onChange={(next) => changeMenu("text", next)}
        />
        <MenuOrderEditor
          title="图片选区"
          description="框选到图像时展示的动作"
          candidates={IMAGE_MENU_ACTIONS}
          value={actions.menu.image}
          onChange={(next) => changeMenu("image", next)}
        />
      </Panel>

      <Panel title="默认行为">
        <div className="sa-grid-2">
          <SelectField
            label="默认动作"
            value={actions.defaultAction ?? ""}
            options={[
              { value: "", label: "不自动执行（手动选择）" },
              ...menuActions.map((action) => ({ value: action, label: ACTION_LABELS[action] })),
            ]}
            onChange={(value) =>
              onEdit((current) => ({
                ...current,
                actions: {
                  ...current.actions,
                  defaultAction: menuActions.includes(value as ActionId) ? (value as ActionId) : null,
                },
              }))
            }
            hint="仅可选当前菜单中存在的动作（契约约束）"
          />
          <div className="sa-field is-stacked">
            <span className="sa-field-label">默认模式</span>
            <SegmentedControl<Mode>
              value={actions.defaultMode}
              options={[
                { value: "quick", label: "快速", description: "1–3s，成本低" },
                { value: "deep", label: "深度", description: "5–25s，成本 3–10x" },
              ]}
              onChange={(mode) =>
                onEdit((current) => ({ ...current, actions: { ...current.actions, defaultMode: mode } }))
              }
            />
          </div>
        </div>
        <SwitchRow
          label="自动复制结果"
          description="结果生成后自动写入剪贴板"
          checked={actions.autoCopy}
          onChange={(autoCopy) =>
            onEdit((current) => ({ ...current, actions: { ...current.actions, autoCopy } }))
          }
        />
      </Panel>

      <Panel
        title="全局热键"
        description="点击热键框后按下组合键；至少包含一个修饰键（Ctrl/Alt/Shift/Win）+ 字母或数字。"
        actions={
          <Button
            variant="ghost"
            size="sm"
            onClick={() =>
              onEdit((current) => ({ ...current, hotkeys: { ...DEFAULT_HOTKEYS } }))
            }
          >
            恢复全部默认
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
        <Hint>热键冲突时会显示占用来源；建议保留一个「框选并唤起」热键。</Hint>
      </Panel>

      <Panel title="搜索方式">
        <SegmentedControl<SearchMode>
          value={search.mode}
          onChange={(mode) =>
            onEdit((current) => ({ ...current, search: { ...current.search, mode } }))
          }
          options={[
            { value: "browser", label: "浏览器搜索", description: "打开系统默认浏览器" },
            { value: "ai", label: "AI 搜索", description: "结果窗输出结论 + 来源" },
          ]}
        />
        <SelectField
          label="搜索引擎"
          value={search.engine}
          options={SEARCH_ENGINES}
          onChange={(engine) =>
            onEdit((current) => ({ ...current, search: { ...current.search, engine } }))
          }
          hint="仅浏览器搜索模式使用"
        />
        {search.engine === "custom" ? (
          <TextInput
            label="自定义搜索 URL 模板"
            value={search.engineUrlTemplate ?? ""}
            placeholder="https://www.example.com/search?q={query}"
            monospace
            hint="必须包含 {query} 占位符；仅允许 https"
            onChange={(template) =>
              onEdit((current) => ({
                ...current,
                search: { ...current.search, engineUrlTemplate: template.trim() ? template : null },
              }))
            }
          />
        ) : null}
      </Panel>

      <SaveBar label="动作与热键" dirty={dirty} saving={saving} onSave={() => void save()} />
    </SectionShell>
  );
}
