/**
 * 设置 · 自定义与关于：
 * - 展示版本、安装包类型、文字识别是否可用、运行情况；
 * - 一键打开配置 / 提示词 / 模型 / 日志 / 后端 / 数据 / 安装文档目录（`open_path`）；
 * - 说明本软件不会自动更新，并给出《自定义指南》与「重新运行引导」的入口。
 *
 * `open_path` 出错时（如文件夹不存在）在按钮下方内联提示，不打断页面。
 */

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui";
import type { OpenPathKind, Stats } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc, toAppError } from "@/shared/ipc";
import { editionLabel } from "../../ocr/status";
import { Hint, InfoRow, Panel, SectionShell, StatusTag, WarnText } from "../parts";
import type { SectionProps } from "../types";

const OPEN_PATHS: { kind: OpenPathKind; label: string; description: string }[] = [
  { kind: "config", label: "配置", description: "应用的所有设置都在这里，可直接编辑" },
  { kind: "prompts", label: "提示词", description: "各动作使用的提示词，可按自己的习惯修改" },
  { kind: "models", label: "模型", description: "下载的本地模型与文字识别模型" },
  { kind: "logs", label: "日志", description: "运行记录，反馈问题时可以附上" },
  { kind: "backends", label: "本地引擎", description: "显卡加速组件所在位置" },
  { kind: "data", label: "数据", description: "历史记录与导出的配置文件" },
  { kind: "docs", label: "安装文档", description: "使用说明、自定义指南与许可证" },
];

const ENGINE_TEXT: Record<string, string> = {
  stopped: "未启动",
  starting: "正在启动",
  ready: "已就绪",
  failed: "启动失败",
};

export function AboutSection(_props: SectionProps) {
  const [stats, setStats] = useState<Stats | null>(null);
  const [loading, setLoading] = useState(true);
  const [statsError, setStatsError] = useState<string | null>(null);
  const [busyKind, setBusyKind] = useState<OpenPathKind | null>(null);
  const [pathError, setPathError] = useState<{ message: string; kind: string } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const loadStats = useCallback(async () => {
    setLoading(true);
    try {
      const value = await ipc.invoke<Stats>(COMMANDS.appStats);
      setStats(value);
      setStatsError(null);
    } catch (err) {
      setStatsError(toAppError(err).message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadStats();
  }, [loadStats]);

  const openPath = async (kind: OpenPathKind, label: string) => {
    setBusyKind(kind);
    setPathError(null);
    setNotice(null);
    try {
      await ipc.invoke<void>(COMMANDS.openPath, { kind });
      setNotice(`已打开「${label}」文件夹。`);
    } catch (err) {
      // 文件夹不存在或没有权限时，把原因直接显示在按钮下面。
      setPathError({ message: toAppError(err).message, kind: label });
    } finally {
      setBusyKind(null);
    }
  };

  return (
    <SectionShell
      title="自定义与关于"
      description="查看版本与运行情况、打开本机的配置与文档。按《自定义指南》可以修改提示词、术语表、设置、模型和本地引擎。"
    >
      <Panel
        title="版本与状态"
        actions={
          <Button variant="secondary" size="sm" disabled={loading} onClick={() => void loadStats()}>
            {loading ? "刷新中…" : "刷新"}
          </Button>
        }
      >
        {statsError ? <WarnText>读取版本信息失败：{statsError}</WarnText> : null}
        {stats ? (
          <div className="sa-stats">
            <InfoRow label="版本" value={stats.version} mono />
            <InfoRow label="安装包" value={editionLabel(stats.edition)} />
            <InfoRow
              label="文字识别"
              value={
                stats.ocrReady ? (
                  <StatusTag tone="ok">可以正常使用</StatusTag>
                ) : (
                  <StatusTag tone="warn">缺少模型，可在「翻译」中下载</StatusTag>
                )
              }
            />
            <InfoRow label="本地引擎" value={ENGINE_TEXT[stats.localEngine] ?? stats.localEngine} />
            <InfoRow label="已下载模型" value={`${stats.modelsInstalled} 个`} />
          </div>
        ) : (
          <Hint>正在读取…</Hint>
        )}
      </Panel>

      <Panel title="本机文件夹" description="需要手动修改设置或查看文件时，可以在这里直接打开。">
        <div className="sa-linklist">
          {OPEN_PATHS.map((item) => (
            <div key={item.kind} className="sa-linkrow">
              <div>
                <span className="sa-linkrow-title">{item.label}</span>
                <span className="sa-field-hint">{item.description}</span>
              </div>
              <Button
                variant="secondary"
                size="sm"
                disabled={busyKind !== null}
                onClick={() => void openPath(item.kind, item.label)}
              >
                {busyKind === item.kind ? "打开中…" : "打开"}
              </Button>
            </div>
          ))}
        </div>
        {pathError ? (
          <p className="sa-error-inline" role="alert">
            打开「{pathError.kind}」失败：{pathError.message}
          </p>
        ) : null}
        {notice ? <Hint>{notice}</Hint> : null}
      </Panel>

      <Panel title="关于更新与自定义">
        <p className="sa-hint">
          本软件不会自动更新，也不会在后台检查新版本。需要升级时，下载新的安装包覆盖安装即可，配置和历史记录都会保留。
        </p>
        <p className="sa-hint">
          想改提示词、术语表、设置、模型或本地引擎，可以打开「安装文档」里的《自定义指南》；本机设置也可以直接在「配置」文件夹中查看。
        </p>
        <p className="sa-hint">
          想重新看一遍首次使用的引导：右键点击托盘图标，选择「重新运行引导」。
        </p>
        <div className="sa-inline">
          <Button
            variant="secondary"
            size="sm"
            disabled={busyKind !== null}
            onClick={() => void openPath("docs", "安装文档")}
          >
            打开《自定义指南》
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={busyKind !== null}
            onClick={() => void openPath("config", "配置")}
          >
            打开配置文件夹
          </Button>
        </div>
        <Hint>修改配置文件后，重新打开应用即可生效；提示词修改后下次动作会使用新内容。</Hint>
      </Panel>
    </SectionShell>
  );
}
