/** 设置 · 高级：深度预算、代理、遥测、骨架模式、引擎状态与运行信息 */

import { useEffect, useState } from "react";
import { Button } from "@/components/ui";
import type { EngineStatusPayload, Stats } from "@/shared/contract/types";
import { COMMANDS, EVENTS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { useIpcEvent } from "@/shared/hooks/useIpcEvent";
import { buildPatch, hasChanges } from "../diff";
import { errorMessage } from "../error";
import { formatDuration } from "../format";
import { editionLabel } from "../../ocr/status";
import { DEVICE_LABELS, LOCAL_BACKEND_LABELS } from "../presets";
import {
  Hint,
  InfoRow,
  NumberField,
  Panel,
  SaveBar,
  SectionShell,
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
  // 契约 v4/v5：本地引擎事件携带实际设备 / 后端 / 回退原因（结果窗与覆盖翻译共用 engine 标签）。
  const [engine, setEngine] = useState<EngineStatusPayload | null>(null);

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

  useEffect(() => {
    let cancelled = false;
    void ipc
      .invoke<EngineStatusPayload>(COMMANDS.localEngineStatus)
      .then((value) => {
        if (!cancelled) setEngine(value);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  useIpcEvent<EngineStatusPayload>(EVENTS.engineStatus, (payload) => {
    if (payload) setEngine(payload);
  });

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
      description="深度模式预算、网络代理、遥测、骨架模式与诊断信息。除代理与骨架模式外均为即时生效。"
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
        <Hint>
          深度模式会开启 thinking、多次检索并抓取正文，预计时延 5–25s，成本约为快速模式 3–10 倍。
        </Hint>
      </Panel>

      <Panel title="网络">
        <TextInput
          label="代理"
          value={advanced.proxy ?? ""}
          placeholder="http://127.0.0.1:7890"
          monospace
          hint="留空表示不使用代理；修改后需重启应用生效"
          onChange={(proxy) => updateAdvanced({ proxy: proxy.trim() ? proxy.trim() : null })}
        />
        <SwitchRow
          label="匿名使用统计（遥测）"
          description="默认关闭；开启后仅上报版本与崩溃计数，不含截图/文本/密钥"
          checked={advanced.telemetry}
          onChange={(telemetry) => updateAdvanced({ telemetry })}
        />
        <Hint>
          本软件不提供自动更新：升级请手动下载新版本安装包覆盖安装，配置与历史数据会保留。
        </Hint>
      </Panel>

      <Panel title="调试 / 骨架模式">
        <InfoRow
          label="advanced.skeletonMode"
          value={advanced.skeletonMode ? "已开启（骨架 / 占位实现）" : "已关闭（默认，真实能力）"}
          hint="默认关闭；开启后捕获、OCR、LLM、搜索、下载均为确定性占位实现，便于无网络联调。"
        />
        <Hint>调试 / 骨架模式（默认关闭，切换后需重启应用生效）。</Hint>
        {advanced.skeletonMode ? (
          <Hint>
            当前为调试状态：事件流与真实实现一致，但不会访问真实网络或模型；用户升级 v1.0
            后该值默认被重置为关闭。
          </Hint>
        ) : (
          <Hint>
            真实能力已启用：捕获 / OCR / 本地推理 /
            云端供应商均走真实实现；如遇问题可临时开启骨架模式定位。
          </Hint>
        )}
      </Panel>

      <Panel
        title="本地引擎状态"
        description="真实引擎状态由 local_engine_status / engine:status 事件同步。"
      >
        {engine ? (
          <div className="sa-stats">
            <InfoRow label="引擎" value={ENGINE_LABELS[engine.local]} />
            <InfoRow
              label="计算设备"
              value={engine.device ? DEVICE_LABELS[engine.device] : "—"}
              hint="实际使用的计算设备（auto 可能解析为 CPU）"
            />
            <InfoRow
              label="推理后端"
              value={engine.backend ? LOCAL_BACKEND_LABELS[engine.backend] : "—"}
              hint="实际后端（含 CPU 回退）；配置中的 gpuBackend 只表达偏好"
            />
            <InfoRow label="设备名称" value={engine.deviceName ?? "—"} />
            <InfoRow
              label="offload 层数"
              value={
                engine.offloadLayers === null || engine.offloadLayers === undefined
                  ? "—"
                  : `${engine.offloadLayers} 层`
              }
            />
            <InfoRow label="监听端口" value={engine.port ? `127.0.0.1:${engine.port}` : "—"} mono />
          </div>
        ) : (
          <Hint>正在读取引擎状态…</Hint>
        )}
        {engine?.fallbackReason ? <WarnText>回退原因：{engine.fallbackReason}</WarnText> : null}
        <Hint>
          本地引擎未启动时，云端供应商与浏览器搜索仍可用；覆盖翻译会按配置回退云端 / 本地。
        </Hint>
      </Panel>

      <Panel title="运行信息" description="每 5 秒通过 app_stats 刷新；离开本页后停止轮询。">
        {statsError ? <WarnText>读取运行信息失败：{statsError}</WarnText> : null}
        {stats ? (
          <div className="sa-stats">
            <InfoRow label="版本" value={stats.version} mono />
            <InfoRow label="版本类型" value={editionLabel(stats.edition)} />
            <InfoRow
              label="OCR 就绪"
              value={stats.ocrReady ? "是（运行时 + 模型齐全）" : "否（缺少模型或运行时）"}
            />
            <InfoRow label="契约版本" value={`v${stats.contractVersion}`} />
            <InfoRow label="运行时长" value={formatDuration(stats.uptimeSec)} />
            <InfoRow label="内存占用" value={`${stats.memoryMb} MB`} />
            <InfoRow label="本地引擎" value={ENGINE_LABELS[stats.localEngine]} />
            <InfoRow label="活动任务" value={`${stats.activeTasks} 个`} />
            <InfoRow label="已安装模型" value={`${stats.modelsInstalled} 个`} />
            <InfoRow
              label="骨架模式"
              value={stats.skeletonMode ? "已开启（调试）" : "已关闭（真实能力）"}
            />
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
