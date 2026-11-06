/** 设置 · 搜索：搜索方式（AI / 浏览器）、搜索引擎与自定义 URL 模板 */

import type { SearchMode } from "@/shared/contract/types";
import { buildPatch, hasChanges } from "../diff";
import { t } from "@/i18n";
import { SEARCH_ENGINES } from "../presets";
import {
  Hint,
  Panel,
  SaveBar,
  SectionShell,
  SegmentedControl,
  SelectField,
  TextInput,
  WarnText,
} from "../parts";
import type { SectionProps } from "../types";

export function SearchSection(props: SectionProps) {
  const { draft, baseline, saving, onEdit, onApply } = props;
  const search = draft.search;
  const dirty = hasChanges(baseline, draft, ["search"]);
  const hasCloudKey = draft.providers.some((provider) => provider.enabled && provider.hasKey);

  const save = async () => {
    const patch = buildPatch(baseline, draft, ["search"]);
    if (Object.keys(patch).length === 0) return;
    await onApply(patch, t("搜索设置"));
  };

  return (
    <SectionShell
      title={t("搜索")}
      description={t("框选文字或图片后，可以交给 AI 直接给结论，也可以交给浏览器打开搜索结果。")}
    >
      <Panel title={t("搜索方式")}>
        <SegmentedControl<SearchMode>
          value={search.mode}
          onChange={(mode) =>
            onEdit((current) => ({ ...current, search: { ...current.search, mode } }))
          }
          options={[
            { value: "ai", label: t("AI 搜索"), description: t("在结果窗直接给出结论和来源") },
            {
              value: "browser",
              label: t("浏览器搜索"),
              description: t("用默认浏览器打开搜索结果"),
            },
          ]}
        />
        <Hint>
          {t(
            "AI 搜索需要先在「AI 服务」中配置云端 API Key；浏览器搜索不需要 Key，也不会把内容发送给任何服务。",
          )}
        </Hint>
        {search.mode === "ai" && !hasCloudKey ? (
          <WarnText>
            {t("当前还没有可用的 AI 服务，AI 搜索暂时用不了；可到「AI 服务」填写 API Key。")}
          </WarnText>
        ) : null}
      </Panel>

      <Panel
        title={t("搜索引擎")}
        description={t("仅浏览器搜索使用；选择「自定义」时可以填自己的网址模板。")}
      >
        <SelectField
          label={t("搜索引擎")}
          value={search.engine}
          options={SEARCH_ENGINES}
          onChange={(engine) =>
            onEdit((current) => ({ ...current, search: { ...current.search, engine } }))
          }
        />
        {search.engine === "custom" ? (
          <TextInput
            label={t("自定义搜索网址")}
            value={search.engineUrlTemplate ?? ""}
            placeholder="https://www.example.com/search?q={query}"
            monospace
            hint={t(
              "用 {query} 表示搜索词，例如 https://www.example.com/search?q={query}；只支持 https 开头的网址。",
            )}
            onChange={(template) =>
              onEdit((current) => ({
                ...current,
                search: {
                  ...current.search,
                  engineUrlTemplate: template.trim() ? template : null,
                },
              }))
            }
          />
        ) : null}
      </Panel>

      <SaveBar label={t("搜索设置")} dirty={dirty} saving={saving} onSave={() => void save()} />
    </SectionShell>
  );
}
