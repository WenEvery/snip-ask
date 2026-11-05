/** 引导第 1 步：AI 服务 —— 预设、Base URL、API Key、模型名、测试连接 */

import { useState } from "react";
import type { ProviderDraft, TestResult } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import type { StepProps } from "../types";
import { ONBOARDING_PROVIDERS } from "../presets";

type AiDraft = NonNullable<StepProps["draft"]["ai"]>;

export function AiServiceStep(props: StepProps) {
  const { draft, onEdit, onError } = props;
  const ai: AiDraft = draft.ai ?? { providerId: "", baseUrl: "", model: "", tested: false };
  const [apiKey, setApiKey] = useState("");
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<TestResult | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const updateAi = (patch: Partial<AiDraft>) => onEdit({ ai: { ...ai, ...patch } });

  const selectPreset = (presetId: string) => {
    const preset = ONBOARDING_PROVIDERS.find((item) => item.id === presetId);
    if (!preset) return;
    setResult(null);
    setNote(null);
    onEdit({
      ai: { providerId: preset.id, baseUrl: preset.baseUrl, model: preset.model, tested: false },
    });
  };

  const testConnection = async () => {
    if (!ai.baseUrl.trim() || !ai.model.trim()) {
      setNote("请先填写 Base URL 与模型名，再测试连接。");
      return;
    }
    const preset = ONBOARDING_PROVIDERS.find((item) => item.id === ai.providerId);
    const draftProvider: ProviderDraft = {
      id: ai.providerId || "custom",
      name: preset?.name ?? "自定义",
      baseUrl: ai.baseUrl.trim(),
      model: ai.model.trim(),
      kind: preset?.kind ?? "openAiCompatible",
      caps: preset?.caps ?? [],
      apiKey: apiKey.trim() ? apiKey.trim() : null,
    };
    setTesting(true);
    setNote(null);
    try {
      const testResult = await ipc.invoke<TestResult>(COMMANDS.providerTest, {
        draft: draftProvider,
      });
      setResult(testResult);
      if (testResult.ok) updateAi({ tested: true });
    } catch (err) {
      onError(err);
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="ob-step">
      <h2 className="ob-step-title">连接一个 AI 服务</h2>
      <p className="ob-hint">
        所有云端能力均为 BYOK（自带 Key），Key 只存 Windows 凭据管理器，不会进入配置文件。没有 Key
        也可以先跳过。
      </p>

      <div className="ob-presets">
        {ONBOARDING_PROVIDERS.map((preset) => (
          <button
            key={preset.id}
            type="button"
            className={`ob-preset${ai.providerId === preset.id ? " is-selected" : ""}`}
            onClick={() => selectPreset(preset.id)}
          >
            <span className="ob-preset-name">{preset.name}</span>
            <span className="ob-preset-note">{preset.note}</span>
          </button>
        ))}
      </div>

      <div className="ob-form">
        <label className="ob-field">
          <span className="ob-field-label">Base URL</span>
          <input
            className="ob-input is-mono"
            value={ai.baseUrl}
            placeholder="https://api.deepseek.com"
            onChange={(event) => updateAi({ baseUrl: event.target.value.trim() })}
          />
          <span className="ob-hint">仅允许 https，或 http://127.0.0.1 本地服务。</span>
        </label>
        <label className="ob-field">
          <span className="ob-field-label">API Key</span>
          <input
            className="ob-input is-mono"
            type="password"
            value={apiKey}
            placeholder="粘贴 API Key（可稍后再填）"
            onChange={(event) => setApiKey(event.target.value)}
          />
          <span className="ob-hint">输入后掩码显示；骨架模式下仅保存在内存。</span>
        </label>
        <label className="ob-field">
          <span className="ob-field-label">模型名</span>
          <input
            className="ob-input is-mono"
            value={ai.model}
            placeholder="deepseek-flash"
            onChange={(event) => updateAi({ model: event.target.value.trim() })}
          />
        </label>
      </div>

      <div className="ob-inline">
        <button
          type="button"
          className="ob-btn ob-btn-primary"
          disabled={testing}
          onClick={() => void testConnection()}
        >
          {testing ? "测试中…" : "测试连接"}
        </button>
        {ai.tested ? <span className="ob-tag is-ok">已通过测试</span> : null}
        <span className="ob-hint">测试会发送一次最小请求；失败也可继续，之后可在设置页补齐。</span>
      </div>

      {note ? <p className="ob-warn">{note}</p> : null}
      {result ? <TestResultBlock result={result} /> : null}

      <p className="ob-hint">
        也可以稍后配置：直接点「下一步」，AI 动作会保持禁用，可在设置 → AI 服务中补齐。
      </p>
    </div>
  );
}

function TestResultBlock(props: { result: TestResult }) {
  const { result } = props;
  if (result.ok) {
    return (
      <div className="ob-testresult is-ok">
        <strong>连接成功</strong>
        <span>
          延迟 {result.latencyMs ?? "—"} ms · 模型 {result.model ?? "未知"}
        </span>
      </div>
    );
  }
  const error = result.error;
  return (
    <div className="ob-testresult is-bad">
      <strong>连接失败</strong>
      <span>
        {error?.code ? `${error.code} · ` : ""}
        {error?.message ?? `HTTP ${result.status ?? "无响应"}`}
      </span>
      {actionableHint(result) ? <span className="ob-hint">{actionableHint(result)}</span> : null}
    </div>
  );
}

/** 报告 §2.4.3：401/402/429/超时/DNS 给出可操作提示 */
function actionableHint(result: TestResult): string | null {
  const code = result.error?.code;
  if (code === "SA-3002" || result.status === 401) {
    return "Key 无效或没有权限：请确认 Key 复制完整、与所选供应商匹配。";
  }
  if (code === "SA-3003" || result.status === 402) {
    return "账户额度不足：请充值，或先换用其他供应商。";
  }
  if (code === "SA-3004" || result.status === 429) {
    return "请求被限流：稍后再试，或降低调用频率。";
  }
  if (code === "SA-3005") {
    return "连接超时：检查网络 / 代理，并确认 Base URL 是否正确。";
  }
  if (code === "SA-3001") {
    return "网络不可用：可先跳过，联网后再到设置页配置。";
  }
  return null;
}
