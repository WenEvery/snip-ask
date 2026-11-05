/** 设置 · 隐私：上传权限与「仅本机处理」开关 */

import { buildPatch, hasChanges } from "../diff";
import { Hint, Panel, SaveBar, SectionShell, SwitchRow } from "../parts";
import type { SectionProps } from "../types";

export function PrivacySection(props: SectionProps) {
  const { draft, baseline, saving, onEdit, onApply } = props;
  const privacy = draft.privacy;
  const dirty = hasChanges(baseline, draft, ["privacy"]);

  const updatePrivacy = (patch: Partial<typeof privacy>) => {
    onEdit((current) => ({ ...current, privacy: { ...current.privacy, ...patch } }));
  };

  const save = async () => {
    const patch = buildPatch(baseline, draft, ["privacy"]);
    if (Object.keys(patch).length === 0) return;
    await onApply(patch, "隐私设置");
  };

  return (
    <SectionShell
      title="隐私"
      description="控制哪些内容可以离开本机。默认只在本机处理，只有你主动使用云端功能时才会上传。"
    >
      <Panel title="上传权限">
        <SwitchRow
          label="允许上传识别到的文字"
          description="关闭后，AI 搜索、解答与云端翻译不可用；本地识别、浏览器搜索、复制文字仍然可用。"
          checked={privacy.allowTextUpload}
          onChange={(allowTextUpload) => updatePrivacy({ allowTextUpload })}
        />
        <SwitchRow
          label="允许上传截图"
          description="关闭后，图片搜索与看图解答不可用；本地识别不受影响。"
          checked={privacy.allowImageUpload}
          onChange={(allowImageUpload) => updatePrivacy({ allowImageUpload })}
        />
      </Panel>

      <Panel title="仅本机处理">
        <SwitchRow
          label="只在本机处理内容"
          description="开启后不会把任何文字或图片发送到云端，只能使用本地识别、本地翻译与浏览器搜索。"
          checked={privacy.localOnly}
          onChange={(localOnly) => updatePrivacy({ localOnly })}
        />
        <Hint>需要联网的 AI 功能会自动停用；关闭后恢复你选择的搜索与翻译方式。</Hint>
      </Panel>

      <Panel title="数据保存">
        <Hint>
          历史记录、配置与日志都保存在本机，不会上传。可在「核心」中关闭历史记录，或调整保留条数与天数。
        </Hint>
      </Panel>

      <SaveBar label="隐私设置" dirty={dirty} saving={saving} onSave={() => void save()} />
    </SectionShell>
  );
}
