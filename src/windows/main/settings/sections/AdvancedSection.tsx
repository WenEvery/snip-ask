/** 设置 · 高级：深度模式预算、网络代理、本地引擎与运行信息 */

import { useEffect, useState } from "react";
import { Button } from "@/components/ui";
import type { EngineStatusPayload, EngineStatus, Stats } from "@/shared/contract/types";
import { COMMANDS, EVENTS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { t } from "@/i18n";
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
  TextInput,
  WarnText,
} from "../parts";
import type { SectionProps } from "../types";

const STATS_POLL_MS = 5000;

const ENGINE_LABELS: Record<EngineStatus, string> = {
  stopped: t("未启动（用到时自动启动）"),
  starting: t("正在启动…"),
  ready: t("已就绪"),
  failed: t("启动失败"),
};

const HELP_LINKS: { label: string; url: string; description: string }[] = [
  {
    label: t("DeepSeek 接口文档"),
    url: "https://api-docs.deepseek.com/",
    description: t("查询地址、模型名与计费方式"),
  },
  {
    label: t("Ollama 官网"),
    url: "https://ollama.com",
    description: t("在本机运行开源模型"),
  },
];

export function AdvancedSection(props: SectionProps) {
  const { draft, baseline, saving, onEdit, onApply, onError } = props;
  const advanced = draft.advanced;
  const budget = draft.modes.deepBudget;
  const dirty = hasChanges(baseline, draft, ["modes", "advanced"]);

  const [stats, setStats] = useState<Stats | null>(null);
  const [statsError, setStatsError] = useState<string | null>(null);
  const [engine, setEngine] = useState<EngineStatusPayload | null>(null);
  const [engineBusy, setEngineBusy] = useState<"start" | "stop" | null>(null);
  const [engineError, setEngineError] = useState<string | null>(null);

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
    await onApply(patch, t("高级设置"));
  };

  const startEngine = async () => {
    setEngineBusy("start");
    setEngineError(null);
    try {
      const status = await ipc.invoke<EngineStatusPayload>(COMMANDS.localEngineStart);
      if (status) setEngine(status);
    } catch (err) {
      setEngineError(errorMessage(err));
    } finally {
      setEngineBusy(null);
    }
  };

  const stopEngine = async () => {
    setEngineBusy("stop");
    setEngineError(null);
    try {
      const status = await ipc.invoke<EngineStatusPayload>(COMMANDS.localEngineStop);
      if (status) setEngine(status);
    } catch (err) {
      setEngineError(errorMessage(err));
    } finally {
      setEngineBusy(null);
    }
  };

  const openExternal = async (url: string) => {
    try {
      await ipc.invoke<void>(COMMANDS.openExternal, { url });
    } catch (err) {
      onError(err);
    }
  };

  const engineRunning = engine?.local === "ready" || engine?.local === "starting";
  const engineLabel = engine ? ENGINE_LABELS[engine.local] : t("正在读取…");

  return (
    <SectionShell
      title={t("高级")}
      description={t("深度模式的用量上限、网络代理，以及本地模型的运行情况。")}
    >
      <Panel
        title={t("深度模式的用量上限")}
        description={t(
          "深度模式为了答得更准，会多搜索几次、多读几篇网页；这里限制它每次最多花多少。",
        )}
      >
        <div className="sa-grid-3">
          <NumberField
            label={t("最多搜索几次")}
            value={budget.search}
            min={0}
            max={5}
            suffix={t("次")}
            onChange={(search) => updateBudget({ search })}
          />
          <NumberField
            label={t("最多读几篇网页")}
            value={budget.fetch}
            min={0}
            max={5}
            suffix={t("篇")}
            onChange={(fetch) => updateBudget({ fetch })}
          />
          <NumberField
            label={t("最长输出多少字")}
            value={budget.maxTokens}
            min={128}
            max={8192}
            step={128}
            suffix={t("字")}
            onChange={(maxTokens) => updateBudget({ maxTokens })}
          />
        </div>
        <Hint>
          {t(
            "数值越大答得越完整，也越慢、越费钱；一般保持默认即可（搜索 3 次、读 2 篇、约 1500 字）。只影响「深度」模式。",
          )}
        </Hint>
      </Panel>

      <Panel title={t("网络")}>
        <TextInput
          label={t("代理地址")}
          value={advanced.proxy ?? ""}
          placeholder="http://127.0.0.1:7890"
          monospace
          hint={t("如果访问云端服务需要代理，在这里填写；留空表示直连。修改后重新打开应用生效。")}
          onChange={(proxy) => updateAdvanced({ proxy: proxy.trim() ? proxy.trim() : null })}
        />
        <Hint>
          {t(
            "本软件不会自动更新。需要升级时，下载新的安装包覆盖安装即可，配置和历史记录都会保留。",
          )}
        </Hint>
      </Panel>

      <Panel
        title={t("本地引擎")}
        description={t("本地翻译和本地模型需要它。平时用到时会自动启动，也可以在这里手动开关。")}
        actions={
          <div className="sa-inline">
            {engineRunning ? (
              <Button
                variant="secondary"
                size="sm"
                disabled={engineBusy !== null}
                onClick={() => void stopEngine()}
              >
                {engineBusy === "stop" ? t("正在停止…") : t("停止引擎")}
              </Button>
            ) : (
              <Button
                variant="primary"
                size="sm"
                disabled={engineBusy !== null}
                onClick={() => void startEngine()}
              >
                {engineBusy === "start" ? t("正在启动…") : t("启动引擎")}
              </Button>
            )}
          </div>
        }
      >
        <div className="sa-stats">
          <InfoRow label={t("运行状态")} value={engineLabel} />
          <InfoRow
            label={t("计算设备")}
            value={engine?.device ? DEVICE_LABELS[engine.device] : "—"}
          />
          <InfoRow
            label={t("显卡加速")}
            value={engine?.backend ? LOCAL_BACKEND_LABELS[engine.backend] : "—"}
            hint={t("没有可用显卡时会自动用 CPU")}
          />
          <InfoRow label={t("设备名称")} value={engine?.deviceName ?? "—"} />
          <InfoRow
            label={t("显存使用")}
            value={
              engine?.offloadLayers === null || engine?.offloadLayers === undefined
                ? "—"
                : t("已加载 {count} 层到显卡", { count: engine.offloadLayers })
            }
          />
          <InfoRow
            label={t("本机端口")}
            value={engine?.port ? `127.0.0.1:${engine.port}` : "—"}
            mono
          />
        </div>
        {engine?.fallbackReason ? <WarnText>{engine.fallbackReason}</WarnText> : null}
        {engineError ? (
          <p className="sa-error-inline" role="alert">
            {t("启动本地引擎失败：{error}", { error: engineError })}
          </p>
        ) : null}
        <Hint>{t("本地引擎没有启动时，云端 AI 和浏览器搜索仍然可以正常使用。")}</Hint>
      </Panel>

      <Panel title={t("运行信息")} description={t("每 5 秒自动更新。")}>
        {statsError ? (
          <WarnText>{t("读取运行信息失败：{error}", { error: statsError })}</WarnText>
        ) : null}
        {stats ? (
          <div className="sa-stats">
            <InfoRow label={t("版本")} value={stats.version} mono />
            <InfoRow label={t("安装包")} value={editionLabel(stats.edition, t)} />
            <InfoRow
              label={t("文字识别")}
              value={stats.ocrReady ? t("已就绪") : t("未就绪（缺少模型，可在「翻译」中下载）")}
            />
            <InfoRow label={t("运行时长")} value={formatDuration(stats.uptimeSec)} />
            <InfoRow label={t("内存占用")} value={`${stats.memoryMb} MB`} />
            <InfoRow label={t("本地引擎")} value={ENGINE_LABELS[stats.localEngine]} />
            <InfoRow
              label={t("正在处理的任务")}
              value={t("{count} 个", { count: stats.activeTasks })}
            />
            <InfoRow
              label={t("已下载模型")}
              value={t("{count} 个", { count: stats.modelsInstalled })}
            />
          </div>
        ) : (
          <Hint>{t("正在读取运行信息…")}</Hint>
        )}
      </Panel>

      <Panel title={t("帮助与文档")}>
        <div className="sa-linklist">
          {HELP_LINKS.map((link) => (
            <div key={link.url} className="sa-linkrow">
              <div>
                <span className="sa-linkrow-title">{link.label}</span>
                <span className="sa-field-hint">{link.description}</span>
              </div>
              <Button variant="secondary" size="sm" onClick={() => void openExternal(link.url)}>
                {t("打开")}
              </Button>
            </div>
          ))}
        </div>
      </Panel>

      <SaveBar label={t("高级设置")} dirty={dirty} saving={saving} onSave={() => void save()} />
    </SectionShell>
  );
}
