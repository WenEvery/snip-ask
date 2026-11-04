/** 设置 · 核心：语言、启动行为、历史记录、导入导出与恢复默认 */

import { useState } from "react";
import { Button } from "@/components/ui";
import type { ConfigView } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { buildPatch, hasChanges } from "../diff";
import {
  ConfirmAction,
  Field,
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

export function GeneralSection(props: SectionProps) {
  const { draft, baseline, saving, onEdit, onApply, onReplace, onError } = props;
  const [importPath, setImportPath] = useState("");
  const [exportedPath, setExportedPath] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const dirty = hasChanges(baseline, draft, ["app", "history"]);

  const saveGroup = async () => {
    const patch = buildPatch(baseline, draft, ["app", "history"]);
    if (Object.keys(patch).length === 0) return;
    const ok = await onApply(patch, "核心设置");
    if (ok) setNotice("核心设置已保存并即时生效");
  };

  const applyAutostart = async (enabled: boolean) => {
    const previous = draft.app.autostart;
    onEdit((current) => ({ ...current, app: { ...current.app, autostart: enabled } }));
    setBusy("autostart");
    try {
      await ipc.invoke<void>(COMMANDS.setAutostart, { enabled });
      const nextDraft: ConfigView = { ...draft, app: { ...draft.app, autostart: enabled } };
      const patch = buildPatch(baseline, nextDraft, ["app"]);
      if (Object.keys(patch).length > 0) await onApply(patch, "开机自启");
      setNotice(enabled ? "已注册开机自启" : "已取消开机自启");
    } catch (err) {
      onEdit((current) => ({ ...current, app: { ...current.app, autostart: previous } }));
      onError(err);
    } finally {
      setBusy(null);
    }
  };

  const exportConfig = async () => {
    setBusy("export");
    try {
      const path = await ipc.invoke<string>(COMMANDS.configExport, {});
      setExportedPath(path);
      setNotice("配置已导出（默认不含密钥）");
    } catch (err) {
      onError(err);
    } finally {
      setBusy(null);
    }
  };

  const importConfig = async () => {
    const path = importPath.trim();
    if (!path) {
      setNotice("请先填写要导入的配置文件路径");
      return;
    }
    setBusy("import");
    try {
      const view = await ipc.invoke<ConfigView>(COMMANDS.configImport, { path });
      onReplace(view);
      setImportPath("");
      setNotice("配置已导入，已整份替换（本机密钥保留）");
    } catch (err) {
      onError(err);
    } finally {
      setBusy(null);
    }
  };

  const resetAll = async () => {
    setBusy("reset");
    try {
      const view = await ipc.invoke<ConfigView>(COMMANDS.configReset, { scope: "all" });
      onReplace(view);
      setNotice("已恢复全部默认配置");
    } catch (err) {
      onError(err);
    } finally {
      setBusy(null);
    }
  };

  return (
    <SectionShell
      title="核心"
      description="界面语言、启动行为、历史记录与配置文件管理。修改后点击「应用本组设置」生效。"
    >
      <Panel title="界面与启动">
        <SelectField
          label="界面语言"
          value={draft.app.language}
          options={[
            { value: "zh-CN", label: "简体中文" },
            { value: "en-US", label: "English（预留）" },
          ]}
          hint="英文界面为预留项，脚手架阶段仅切换配置"
          onChange={(value) =>
            onEdit((current) => ({
              ...current,
              app: { ...current.app, language: value === "en-US" ? "en-US" : "zh-CN" },
            }))
          }
        />
        <InfoRow
          label="单实例模式"
          value={draft.app.singleInstance ? "已启用（后端强制）" : "已关闭"}
          hint="由后端单实例插件保证，界面不可关闭"
        />
        <SwitchRow
          label="启动时最小化到托盘"
          description="开机后不弹出主窗口，仅保留托盘图标"
          checked={draft.app.startMinimized}
          onChange={(checked) =>
            onEdit((current) => ({ ...current, app: { ...current.app, startMinimized: checked } }))
          }
        />
        <SwitchRow
          label="开机自启"
          description="注册为当前用户的开机启动项；会同时调用 set_autostart 命令"
          checked={draft.app.autostart}
          disabled={busy === "autostart"}
          onChange={(checked) => void applyAutostart(checked)}
        />
      </Panel>

      <Panel title="历史记录" description="历史与用量存于本机 SQLite，不上传。">
        <SwitchRow
          label="保存历史记录"
          checked={draft.history.enabled}
          onChange={(checked) =>
            onEdit((current) => ({ ...current, history: { ...current.history, enabled: checked } }))
          }
        />
        <div className="sa-grid-2">
          <NumberField
            label="最多保留条数"
            value={draft.history.maxEntries}
            min={0}
            max={100000}
            step={100}
            suffix="条"
            onChange={(value) =>
              onEdit((current) => ({ ...current, history: { ...current.history, maxEntries: value } }))
            }
          />
          <NumberField
            label="保留天数"
            value={draft.history.retentionDays}
            min={1}
            max={3650}
            suffix="天"
            onChange={(value) =>
              onEdit((current) => ({
                ...current,
                history: { ...current.history, retentionDays: value },
              }))
            }
          />
        </div>
      </Panel>

      <Panel title="配置文件">
        <InfoRow
          label="数据目录"
          value="%APPDATA%\\SnipAsk"
          mono
          hint="配置文件 config.json、引导状态 onboarding.json、日志与历史库均在此目录"
        />
        <Field label="导出配置" hint="默认导出到数据目录 exports/ 下，不含密钥。">
          <div className="sa-inline">
            <Button variant="secondary" size="sm" disabled={busy === "export"} onClick={() => void exportConfig()}>
              {busy === "export" ? "导出中…" : "导出配置"}
            </Button>
            {exportedPath ? <code className="sa-path">{exportedPath}</code> : null}
          </div>
        </Field>
        <Field label="导入配置" hint="读取 → 迁移 → 校验 → 整份替换；本机密钥保留。">
          <div className="sa-inline">
            <TextInput
              value={importPath}
              placeholder="例如 C:\\Users\\me\\config-backup.json"
              onChange={setImportPath}
              monospace
            />
            <Button variant="secondary" size="sm" disabled={busy === "import"} onClick={() => void importConfig()}>
              {busy === "import" ? "导入中…" : "导入"}
            </Button>
          </div>
        </Field>
        <Field label="恢复默认">
          <ConfirmAction
            description="将把全部配置恢复为默认值（含 AI 服务、热键、外观），确定继续？"
            confirmLabel="确认恢复全部默认"
            disabled={busy === "reset"}
            onConfirm={() => void resetAll()}
          >
            恢复全部默认配置
          </ConfirmAction>
        </Field>
      </Panel>

      {notice ? <Hint>{notice}</Hint> : null}
      {dirty ? <WarnText>核心设置存在未应用的修改，点击下方按钮提交。</WarnText> : null}
      <SaveBar label="核心设置" dirty={dirty} saving={saving} onSave={() => void saveGroup()} />
    </SectionShell>
  );
}
