/**
 * 热键录入控件：聚焦后监听 keydown 捕获组合键，并调用 `hotkey_probe` 检测冲突。
 * 设置页「动作与热键」分组使用（引导第 5 步有简化版）。
 */

import { useState } from "react";
import { Button } from "@/components/ui";
import type { HotkeyStatus } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { captureHotkey, hotkeyLabel, hotkeyStatusHint } from "../hotkey";

export interface HotkeyFieldProps {
  label: string;
  description?: string;
  value: string;
  /** 传入默认值时显示「恢复默认」 */
  defaultValue?: string;
  disabled?: boolean;
  onChange: (spec: string) => void;
  onError: (error: unknown) => void;
}

export function HotkeyField(props: HotkeyFieldProps) {
  const [recording, setRecording] = useState(false);
  const [probing, setProbing] = useState(false);
  const [status, setStatus] = useState<HotkeyStatus | null>(null);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (!recording) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      setRecording(false);
      return;
    }
    if (event.key === "Backspace" || event.key === "Delete") {
      props.onChange("");
      setStatus(null);
      setRecording(false);
      return;
    }
    const spec = captureHotkey(event);
    if (spec) {
      props.onChange(spec);
      setStatus(null);
      setRecording(false);
    }
  };

  const probe = async () => {
    setProbing(true);
    try {
      const result = await ipc.invoke<HotkeyStatus>(COMMANDS.hotkeyProbe, { spec: props.value });
      setStatus(result);
    } catch (err) {
      props.onError(err);
    } finally {
      setProbing(false);
    }
  };

  const resetVisible = props.defaultValue !== undefined && props.defaultValue !== props.value;

  return (
    <div className="sa-hotkey">
      <div className="sa-hotkey-info">
        <span className="sa-hotkey-label">{props.label}</span>
        {props.description ? <span className="sa-field-hint">{props.description}</span> : null}
        {status ? <HotkeyStatusLine status={status} /> : null}
      </div>
      <div className="sa-hotkey-controls">
        <button
          type="button"
          className={`sa-hotkey-btn${recording ? " is-recording" : ""}${props.value ? "" : " is-empty"}`}
          disabled={props.disabled}
          onClick={() => {
            setRecording(true);
            setStatus(null);
          }}
          onKeyDown={handleKeyDown}
          onBlur={() => setRecording(false)}
          title="点击后按下组合键；Backspace 清空，Esc 取消"
        >
          {recording ? "请按下组合键…" : hotkeyLabel(props.value)}
        </button>
        <Button variant="secondary" size="sm" disabled={probing || !props.value} onClick={probe}>
          {probing ? "检测中…" : "检测"}
        </Button>
        {resetVisible ? (
          <Button
            variant="ghost"
            size="sm"
            disabled={props.disabled}
            onClick={() => {
              props.onChange(props.defaultValue ?? "");
              setStatus(null);
            }}
          >
            恢复默认
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function HotkeyStatusLine(props: { status: HotkeyStatus }) {
  // conflictWith="system"（被其它程序占用）与配置内热键字段名由同一映射给出可操作文案
  const hint = hotkeyStatusHint(props.status);
  return <span className={`sa-hotkey-status ${hint.level === "ok" ? "is-ok" : "is-bad"}`}>{hint.text}</span>;
}
