/** 设置 · 高级：深度预算、代理、更新通道、遥测、骨架模式与运行信息 */

import { useEffect, useState } from "react";
import { Button } from "@/components/ui";
import type { Stats } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { buildPatch, hasChanges } from "../diff";
import { errorMessage } from "../error";
import { formatDuration } from "../format";
import { UPDATE_CHANNELS } from "../presets";
import {
  Hint,
  InfoRow,
  NumberField,
  Panel,
  SaveBar,
  SectionShell,
  SelectField,
  SwitchRow,
  TextInput,
  WarnText,
} from "../parts";
import type { SectionProps } from "../types";

const STATS_POLL_MS = 5000;

const ENGINE_LABELS: Record<Stats["localEngine"], string> = {
  stopped: "未启动",
  starting: "启动中",
  ready: "就绪",
  failed: "启动失败",
};

const HELP_LINKS: { label: string; url: string; description: string }[] = [
  {
    label: "DeepSeek API 文档",
    url: "https://api-docs.deepseek.com/",
    description: "配置 Base URL / 模型名 / 计费说明",
  },
  {
    label: "Ollama 官网",
    url: "https://ollama.com",
    description: "本地推理服务与模型库",
  },
];

export function AdvancedSection(props: SectionProps) {
  const { draft, baseline, saving, onEdit, onApply, onError } = props;
  const advanced = draft.advanced;
  const budget = draft.modes.deepBudget;
  const dirty = hasChanges(baseline, draft, ["modes", "advanced"]);

  const [stats, setStats] = useState<Stats | null>(null);
  const [statsError, setStatsError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const value = await ipc.invoke<Stats>(COMMANDS.appStats);
        if (cancelled) return;
        setStats(value);
        setStatsError(null);
      } catch (err) {
        if (cancelled) return;
        setStatsError(errorMessage(err));
      }
    };
    void load();
    const timer = window.setInterval(() => void load(), STATS_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const updateBudget = (patch: Partial<typeof budget>) => {
    onEdit((current) => ({
      ...current,
      modes: { ...current.modes, deepBudget: { ...current.modes.deepBudget, ...patch } },
    }));
  };

  const updateAdvanced = (patch: Partial<typeof advanced>) => {
    onEdit((current) => ({ ...current, advanced: { ...current.advanced, ...patch } }));
  };

  const save = async () => {
    const patch = buildPatch(baseline, draft, ["modes", "advanced"]);
    if (Object.keys(patch).length === 0) return;
    await onApply(patch, "高级设置");
  };

  const openExternal = async (url: string) => {
    try {
      await ipc.invoke<void>(COMMANDS.openExternal, { url });
    } catch (err) {
      onError(err);
    }
  };

  return (
    <SectionShell
      title="高级"
      description="深度模式预算、网络代理、更新与诊断信息。除代理外均为即时生效。"
    >
      <Panel title="深度模式预算" description="限制单次深度任务的最坏开销（契约 §4.3 约束范围）。">
        <div className="sa-grid-3">
          <NumberField
            label="搜索次数"
            value={budget.search}
            min={0}
            max={5}
            suffix="次"
            onChange={(search) => updateBudget({ search })}
          />
          <NumberField
            label="抓取篇数"
            value={budget.fetch}
            min={0}
            max={5}
            suffix="篇"
            onChange={(fetch) => updateBudget({ fetch })}
          />
          <NumberField
            label="输出 token 上限"
            value={budget.maxTokens}
            min={128}
            max={8192}
            step={128}
            suffix="tokens"
            onChange={(maxTokens) => updateBudget({ maxTokens })}
          />
        </div>
        <Hint>深度模式会开启 thinking、多次检索并抓取正文，预计时延 5–25s，成本约为快速模式 3–10 倍。</Hint>
      </Panel>

      <Panel title="网络与更新">
        <TextInput
          label="代理"
          value={advanced.proxy ?? ""}
          placeholder="http://127.0.0.1:7890"
          monospace
          hint="留空表示不使用代理；修改后需重启应用生效"
          onChange={(proxy) => updateAdvanced({ proxy: proxy.trim() ? proxy.trim() : null })}
        />
        <SelectField
          label="更新通道"
          value={advanced.updateChannel}
          options={UPDATE_CHANNELS}
          onChange={(value) =>
            updateAdvanced({ updateChannel: value === "preview" ? "preview" : "stable" })
          }
          hint="预览通道可能包含未完成的功能"
        />
        <SwitchRow
          label="匿名使用统计（遥测）"
          description="默认关闭；开启后仅上报版本与崩溃计数，不含截图/文本/密钥"
          checked={advanced.telemetry}
          onChange={(telemetry) => updateAdvanced({ telemetry })}
        />
      </Panel>

      <Panel title="骨架模式">
        <InfoRow
          label="advanced.skeletonMode"
          value={advanced.skeletonMode ? "已开启（骨架 / 占位实现）" : "已关闭（真实能力）"}
          hint="由配置文件只读决定；骨架模式下捕获、OCR、LLM、搜索、下载均为确定性占位实现，便于无网络联调。"
        />
        {advanced.skeletonMode ? (
          <Hint>当前为脚手架默认状态：事件流与真实实现一致，但不会访问真实网络或模型。</Hint>
        ) : (
          <WarnText>真实能力尚未在脚手架阶段实现，关闭骨架模式可能导致动作失败。</WarnText>
        )}
      </Panel>

      <Panel title="运行信息" description="每 5 秒通过 app_stats 刷新；离开本页后停止轮询。">
        {statsError ? <WarnText>读取运行信息失败：{statsError}</WarnText> : null}
        {stats ? (
          <div className="sa-stats">
            <InfoRow label="版本" value={stats.version} mono />
            <InfoRow label="契约版本" value={`v${stats.contractVersion}`} />
            <InfoRow label="运行时长" value={formatDuration(stats.uptimeSec)} />
            <InfoRow label="内存占用" value={`${stats.memoryMb} MB`} />
            <InfoRow label="本地引擎" value={ENGINE_LABELS[stats.localEngine]} />
            <InfoRow label="活动任务" value={`${stats.activeTasks} 个`} />
            <InfoRow label="已安装模型" value={`${stats.modelsInstalled} 个`} />
          </div>
        ) : (
          <Hint>正在读取运行信息…</Hint>
        )}
      </Panel>

      <Panel title="帮助与文档" description="外部链接仅允许 https 或 127.0.0.1。">
        <div className="sa-linklist">
          {HELP_LINKS.map((link) => (
            <div key={link.url} className="sa-linkrow">
              <div>
                <span className="sa-linkrow-title">{link.label}</span>
                <span className="sa-field-hint">{link.description}</span>
              </div>
              <Button variant="secondary" size="sm" onClick={() => void openExternal(link.url)}>
                打开
              </Button>
            </div>
          ))}
        </div>
        <Hint>项目帮助站点将在正式发布后补充；当前提供供应商文档入口。</Hint>
      </Panel>

      <SaveBar label="高级设置" dirty={dirty} saving={saving} onSave={() => void save()} />
    </SectionShell>
  );
}
