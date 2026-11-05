/** 设置 · AI 服务：供应商卡片、展开编辑、连接测试、新增 / 删除 */

import { useState } from "react";
import { Button } from "@/components/ui";
import type {
  Provider,
  ProviderDraft,
  ProviderKind,
  ProviderView,
  TestResult,
} from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { buildPatch, hasChanges } from "../diff";
import { CAP_LABELS, PROVIDER_PRESETS } from "../presets";
import {
  EmptyHint,
  InfoRow,
  Panel,
  SaveBar,
  SectionShell,
  SelectField,
  StatusTag,
  SwitchRow,
} from "../parts";
import { ProviderEditor } from "../components/ProviderEditor";
import type { SectionProps } from "../types";

const PROVIDER_KIND_LABELS: Record<ProviderKind, string> = {
  openAiCompatible: "OpenAI 兼容",
  qwenMt: "Qwen-MT 翻译",
};

function uniqueProviderId(base: string, providers: ProviderView[]): string {
  const used = new Set(providers.map((provider) => provider.id));
  if (!used.has(base)) return base;
  let index = 2;
  while (used.has(`${base}-${index}`)) index += 1;
  return `${base}-${index}`;
}

export function AiProvidersSection(props: SectionProps) {
  const { draft, baseline, saving, onEdit, onApply, onError } = props;
  const providers = draft.providers;
  const [expandedId, setExpandedId] = useState<string | null>(providers[0]?.id ?? null);
  const [presetId, setPresetId] = useState("deepseek");
  const [apiKeys, setApiKeys] = useState<Record<string, string>>({});
  const [testResults, setTestResults] = useState<Record<string, TestResult>>({});
  const [testingId, setTestingId] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const dirty = hasChanges(baseline, draft, ["providers"]);

  const updateProvider = (id: string, patch: Partial<Provider>) => {
    onEdit((current) => ({
      ...current,
      providers: current.providers.map((provider) =>
        provider.id === id ? { ...provider, ...patch } : provider,
      ),
    }));
    if (!patch.id || patch.id === id) return;
    const nextId = patch.id;
    setApiKeys((current) => moveEntry(current, id, nextId));
    setTestResults((current) => moveEntry(current, id, nextId));
    setExpandedId((current) => (current === id ? nextId : current));
  };

  const addProvider = () => {
    const preset = PROVIDER_PRESETS.find((item) => item.id === presetId) ?? PROVIDER_PRESETS[0];
    const id = uniqueProviderId(preset.id, providers);
    const view: ProviderView = {
      id,
      name: preset.name,
      baseUrl: preset.baseUrl,
      model: preset.model,
      kind: preset.kind,
      caps: [...preset.caps],
      temperature: preset.temperature,
      maxTokens: preset.maxTokens,
      enabled: true,
      hasKey: false,
      keyMask: null,
    };
    onEdit((current) => ({ ...current, providers: [...current.providers, view] }));
    setExpandedId(id);
    setNotice(`已添加「${preset.name}」，填好地址、模型和密钥后点「应用 AI 服务」`);
  };

  const removeProvider = (id: string) => {
    if (providers.length <= 1) {
      setNotice("至少需要保留一个 AI 供应商");
      return;
    }
    const name = providers.find((provider) => provider.id === id)?.name ?? id;
    onEdit((current) => ({
      ...current,
      providers: current.providers.filter((provider) => provider.id !== id),
    }));
    setExpandedId((current) => (current === id ? null : current));
    setNotice(`已删除「${name}」，点「应用 AI 服务」后生效。`);
  };

  const testProvider = async (provider: ProviderView) => {
    const apiKey = (apiKeys[provider.id] ?? "").trim();
    const draftProvider: ProviderDraft = {
      id: provider.id,
      name: provider.name,
      baseUrl: provider.baseUrl,
      model: provider.model,
      kind: provider.kind,
      caps: provider.caps,
      apiKey: apiKey ? apiKey : null,
      temperature: provider.temperature,
      maxTokens: provider.maxTokens,
    };
    setTestingId(provider.id);
    try {
      const result = await ipc.invoke<TestResult>(COMMANDS.providerTest, { draft: draftProvider });
      setTestResults((current) => ({ ...current, [provider.id]: result }));
      if (result.ok) {
        const latency = result.latencyMs === null ? "" : `，延迟 ${result.latencyMs} ms`;
        setNotice(`「${provider.name}」连接正常${latency}`);
      }
    } catch (err) {
      onError(err);
    } finally {
      setTestingId(null);
    }
  };

  const save = async () => {
    const patch = buildPatch(baseline, draft, ["providers"]);
    if (Object.keys(patch).length === 0) {
      setNotice("供应商配置没有变化");
      return;
    }
    const ok = await onApply(patch, "AI 服务");
    if (!ok) return;
    const pending = draft.providers.filter((provider) => (apiKeys[provider.id] ?? "").trim());
    for (const provider of pending) await testProvider(provider);
    if (pending.length > 0) {
      setApiKeys({});
      setNotice(`已提交 ${pending.length} 个新密钥用于连接测试（保存在 Windows 凭据管理器）`);
    } else {
      setNotice("供应商配置已保存");
    }
  };

  const defaultProvider =
    providers.find((provider) => provider.enabled) ?? (providers.length > 0 ? providers[0] : null);

  return (
    <SectionShell
      title="AI 服务"
      description="添加你使用的 AI 服务。API Key 保存在 Windows 凭据管理器，不会写进配置文件。"
    >
      <Panel
        title="供应商列表"
        description="至少保留一个供应商；被停用的供应商不会用于任何动作。"
        actions={
          <div className="sa-inline">
            <SelectField
              value={presetId}
              options={PROVIDER_PRESETS.map((preset) => ({
                value: preset.id,
                label: preset.name,
              }))}
              onChange={setPresetId}
            />
            <Button variant="secondary" size="sm" onClick={addProvider}>
              新增供应商
            </Button>
          </div>
        }
      >
        {providers.length === 0 ? (
          <EmptyHint>还没有 AI 供应商，请从上方预设新增一个。</EmptyHint>
        ) : null}
        {providers.map((provider) => {
          const expanded = expandedId === provider.id;
          return (
            <div key={provider.id} className={`sa-provider-card${expanded ? " is-expanded" : ""}`}>
              <div className="sa-provider-head">
                <div className="sa-provider-title">
                  <span className="sa-provider-name">{provider.name}</span>
                  <code className="sa-provider-id">{provider.id}</code>
                  <StatusTag tone={provider.hasKey ? "ok" : "warn"}>
                    {provider.hasKey ? `密钥 ${provider.keyMask ?? "已配置"}` : "未配置密钥"}
                  </StatusTag>
                  {provider.enabled ? null : <StatusTag tone="neutral">已停用</StatusTag>}
                </div>
                <div className="sa-provider-actions">
                  <SwitchRow
                    label="启用"
                    checked={provider.enabled}
                    onChange={(enabled) => updateProvider(provider.id, { enabled })}
                  />
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => setExpandedId(expanded ? null : provider.id)}
                  >
                    {expanded ? "收起" : "展开编辑"}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={testingId === provider.id}
                    onClick={() => void testProvider(provider)}
                  >
                    {testingId === provider.id ? "测试中…" : "测试连接"}
                  </Button>
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={providers.length <= 1}
                    onClick={() => removeProvider(provider.id)}
                  >
                    删除
                  </Button>
                </div>
              </div>
              <p className="sa-provider-meta">
                <span className="sa-mono">{provider.baseUrl}</span>
                <span>·</span>
                <span className="sa-mono">{provider.model || "未填写模型"}</span>
                <span>·</span>
                <StatusTag tone={provider.kind === "qwenMt" ? "warn" : "neutral"}>
                  {PROVIDER_KIND_LABELS[provider.kind]}
                </StatusTag>
                <span>
                  能力：
                  {provider.caps.length
                    ? provider.caps.map((cap) => CAP_LABELS[cap]).join(" / ")
                    : "无"}
                </span>
              </p>
              {testResults[provider.id] && !expanded ? (
                <p
                  className={
                    testResults[provider.id].ok ? "sa-testline is-ok" : "sa-testline is-bad"
                  }
                >
                  上次测试：{testResults[provider.id].ok ? "连接成功" : "连接失败"}
                </p>
              ) : null}
              {expanded ? (
                <ProviderEditor
                  provider={provider}
                  apiKey={apiKeys[provider.id] ?? ""}
                  onApiKeyChange={(value) =>
                    setApiKeys((current) => ({ ...current, [provider.id]: value }))
                  }
                  onChange={(patch) => updateProvider(provider.id, patch)}
                  onTest={() => void testProvider(provider)}
                  testing={testingId === provider.id}
                  testResult={testResults[provider.id] ?? null}
                />
              ) : null}
            </div>
          );
        })}
      </Panel>

      <Panel title="默认供应商">
        <InfoRow
          label="当前默认"
          value={defaultProvider ? `${defaultProvider.name} / ${defaultProvider.model}` : "未配置"}
          hint="所有动作共用默认供应商：取第一个已启用且填好模型的供应商"
        />
      </Panel>

      {notice ? <p className="sa-hint">{notice}</p> : null}
      <SaveBar label="AI 服务" dirty={dirty} saving={saving} onSave={() => void save()} />
    </SectionShell>
  );
}

/** 供应商 ID 改名时迁移以 ID 为键的本地表 */
function moveEntry<T>(source: Record<string, T>, from: string, to: string): Record<string, T> {
  if (!(from in source)) return source;
  const next = { ...source };
  const value = next[from];
  delete next[from];
  if (value !== undefined) next[to] = value;
  return next;
}
