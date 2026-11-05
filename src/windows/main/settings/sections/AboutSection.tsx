/**
 * 设置 · 自定义与关于（契约 v5）：
 * - 展示 `app_stats` 的版本、版本类型（edition）、OCR 就绪与运行状态；
 * - 通过 `open_path` 打开配置 / 提示词 / 模型 / 日志 / 后端 / 数据 / 安装文档目录（白名单）；
 * - 明确「本软件不会自动更新」并给出《自定义指南》入口（安装文档目录）。
 *
 * `open_path` 的错误（如 SA-5001 / 目录不存在）就地内联展示，不打断页面。
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
  { kind: "config", label: "配置目录", description: "config.json / onboarding.json（可直接编辑）" },
  {
    kind: "prompts",
    label: "提示词目录",
    description: "各动作的中文提示词模板，可按需修改",
  },
  { kind: "models", label: "模型目录", description: "本地 GGUF / ONNX / OCR 字典" },
  { kind: "logs", label: "日志目录", description: "运行日志（反馈问题时请提供）" },
  { kind: "backends", label: "后端目录", description: "llama.cpp Vulkan / CUDA 组件" },
  { kind: "data", label: "数据目录", description: "历史 SQLite 与导出的配置" },
  { kind: "docs", label: "安装文档目录", description: "README、自定义指南、许可证与安全说明" },
];

export function AboutSection(_props: SectionProps) {
  const [stats, setStats] = useState<Stats | null>(null);
  const [loading, setLoading] = useState(true);
  const [statsError, setStatsError] = useState<string | null>(null);
  const [busyKind, setBusyKind] = useState<OpenPathKind | null>(null);
  const [pathError, setPathError] = useState<{
    code: string;
    message: string;
    kind: string;
  } | null>(null);
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
      setNotice(`已请求打开「${label}」。若未弹出窗口，请检查系统文件管理器权限。`);
    } catch (err) {
      // 契约要求 AppError 内联展示（SA-5001 = 非法 kind，其余为目录不存在 / 打开失败）。
      const appError = toAppError(err);
      setPathError({ code: appError.code, message: appError.message, kind: label });
    } finally {
      setBusyKind(null);
    }
  };

  return (
    <SectionShell
      title="自定义与关于"
      description="查看版本与运行状态、打开本机配置与文档目录。按《自定义指南》可修改提示词、术语表、配置、模型与本地引擎。"
    >
      <Panel
        title="版本与运行状态"
        description="数据来源：app_stats（每 5 秒自动刷新的高级页同源）。"
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
            <InfoRow label="版本类型" value={editionLabel(stats.edition)} />
            <InfoRow
              label="OCR 就绪"
              value={
                stats.ocrReady ? (
                  <StatusTag tone="ok">已就绪（运行时 + 模型）</StatusTag>
                ) : (
                  <StatusTag tone="warn">未就绪（缺模型或运行时）</StatusTag>
                )
              }
              hint="精简版首次使用 OCR 时会下载模型；可在「翻译」分组查看与下载。"
            />
            <InfoRow label="契约版本" value={`v${stats.contractVersion}`} />
            <InfoRow
              label="运行模式"
              value={
                stats.skeletonMode ? (
                  <StatusTag tone="warn">调试 / 骨架模式</StatusTag>
                ) : (
                  <StatusTag tone="ok">真实能力</StatusTag>
                )
              }
            />
            <InfoRow label="本地引擎" value={stats.localEngine} mono />
            <InfoRow label="已安装模型" value={`${stats.modelsInstalled} 个`} />
          </div>
        ) : (
          <Hint>正在读取版本信息…</Hint>
        )}
      </Panel>

      <Panel
        title="本机目录"
        description="仅能打开下列白名单目录；目录不存在时给出内联错误，不会静默失败。"
      >
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
            {pathError.code} · 打开「{pathError.kind}」失败：{pathError.message}
          </p>
        ) : null}
        {notice ? <Hint>{notice}</Hint> : null}
      </Panel>

      <Panel title="关于更新与自定义" description="v1.0.0 起不提供自动更新。">
        <p className="sa-hint">
          本软件<strong>不会自动更新</strong>
          ，也不会在后台检查新版本：升级请手动下载新的安装包覆盖安装，
          配置、密钥与历史数据均会保留。
        </p>
        <p className="sa-hint">
          欢迎按《自定义指南》修改提示词、术语表、配置、模型与本地引擎：安装目录的 <code>docs</code>{" "}
          下提供 README、自定义指南与许可证；应用内也可直接打开该目录。
        </p>
        <div className="sa-inline">
          <Button
            variant="secondary"
            size="sm"
            disabled={busyKind !== null}
            onClick={() => void openPath("docs", "安装文档目录")}
          >
            打开《自定义指南》所在目录
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={busyKind !== null}
            onClick={() => void openPath("config", "配置目录")}
          >
            打开配置目录
          </Button>
        </div>
        <Hint>
          提示：修改配置文件后可在「核心」分组导入配置或重启应用生效；修改提示词后由后端重新加载。
        </Hint>
      </Panel>
    </SectionShell>
  );
}
