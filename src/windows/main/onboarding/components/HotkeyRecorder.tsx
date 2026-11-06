/** 引导第 5 步的热键录制：聚焦后捕获组合键 + `hotkey_probe` 冲突检测 + 恢复默认 */

import { useState } from "react";
import type { HotkeyStatus } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { captureHotkey, hotkeyLabel, hotkeyStatusHint } from "../hotkey";
import { DEFAULT_CAPTURE_HOTKEY } from "../presets";
import { t } from "../../../../i18n";

export interface HotkeyRecorderProps {
  value: string;
  onChange: (spec: string) => void;
  onError: (error: unknown) => void;
}

export function HotkeyRecorder(props: HotkeyRecorderProps) {
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

  return (
    <div className="ob-hotkey">
      <button
        type="button"
        className={`ob-hotkey-btn${recording ? " is-recording" : ""}`}
        onClick={() => {
          setRecording(true);
          setStatus(null);
        }}
        onKeyDown={handleKeyDown}
        onBlur={() => setRecording(false)}
        title={t("点击后按下组合键；Backspace 清空，Esc 取消")}
      >
        {recording ? t("请按下组合键…") : hotkeyLabel(props.value)}
      </button>
      <button
        type="button"
        className="ob-linkbtn"
        disabled={probing || !props.value}
        onClick={() => void probe()}
      >
        {probing ? t("检测中…") : t("检测冲突")}
      </button>
      <button
        type="button"
        className="ob-linkbtn"
        onClick={() => {
          props.onChange(DEFAULT_CAPTURE_HOTKEY);
          setStatus(null);
        }}
      >
        {t("恢复默认 Ctrl+Q")}
      </button>
      {status ? <HotkeyStatusText status={status} /> : null}
    </div>
  );
}

function HotkeyStatusText(props: { status: HotkeyStatus }) {
  // 与设置页共用映射：conflictWith="system" → 被其它程序占用 + 改键建议
  const hint = hotkeyStatusHint(props.status);
  return (
    <span className={`ob-hotkey-status ${hint.level === "ok" ? "is-ok" : "is-bad"}`}>
      {hint.text}
    </span>
  );
}
