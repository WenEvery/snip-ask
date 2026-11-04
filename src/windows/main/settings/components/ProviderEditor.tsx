/** AI 服务 · 单个供应商编辑表单（字段直接写回配置草稿，密钥只随测试连接提交） */

import type { Cap, Provider, ProviderView, TestResult } from "@/shared/contract/types";
import { Button } from "@/components/ui";
import { ALL_CAPS, CAP_LABELS } from "../presets";
import { Field, NumberField, StatusTag, TextInput } from "../parts";

export interface ProviderEditorProps {
  provider: ProviderView;
  apiKey: string;
  onApiKeyChange: (value: string) => void;
  onChange: (patch: Partial<Provider>) => void;
  onTest: () => void;
  testing: boolean;
  testResult: TestResult | null;
}

const ID_PATTERN = /^[a-z0-9-_]{1,32}$/;
const URL_PATTERN = /^(https:\/\/|http:\/\/127\.0\.0\.1(:\d{1,5})?(\/|$))/;

export function ProviderEditor(props: ProviderEditorProps) {
  const { provider } = props;
  const toggleCap = (cap: Cap) => {
    const caps = provider.caps.includes(cap)
      ? provider.caps.filter((item) => item !== cap)
      : [...provider.caps, cap];
    props.onChange({ caps });
  };

  const idError = ID_PATTERN.test(provider.id)
    ? undefined
    : "仅允许小写字母、数字、- 和 _，长度 1–32";
  const urlError = URL_PATTERN.test(provider.baseUrl)
    ? undefined
    : "仅允许 https:// 或 http://127.0.0.1[:端口]";

  return (
    <div className="sa-provider-editor">
      <div className="sa-grid-2">
        <TextInput
          label="显示名称"
          value={provider.name}
          placeholder="例如 DeepSeek"
          onChange={(name) => props.onChange({ name })}
        />
        <TextInput
          label="供应商 ID"
          value={provider.id}
          placeholder="例如 my-provider"
          error={idError}
          monospace
          onChange={(id) => props.onChange({ id: id.toLowerCase() })}
        />
      </div>
      <TextInput
        label="Base URL"
        value={provider.baseUrl}
        placeholder="https://api.deepseek.com"
        error={urlError}
        monospace
        hint="OpenAI 兼容端点；本地服务请用 http://127.0.0.1:端口"
        onChange={(baseUrl) => props.onChange({ baseUrl: baseUrl.trim() })}
      />
      <TextInput
        label="模型名"
        value={provider.model}
        placeholder="deepseek-flash"
        error={provider.model.trim() ? undefined : "模型名不能为空"}
        monospace
        onChange={(model) => props.onChange({ model: model.trim() })}
      />
      <TextInput
        label="API Key"
        value={props.apiKey}
        type="password"
        placeholder={provider.hasKey ? "已保存，如不修改请留空" : "粘贴 API Key"}
        hint="不写入配置文件；点击「测试连接」时随请求提交，由后端凭据服务保存（骨架模式存内存）"
        onChange={props.onApiKeyChange}
      />
      <Field label="能力标记" hint="决定该供应商能被哪些动作选用（视觉 / 搜索 / 思考 / 翻译）。">
        <div className="sa-caps">
          {ALL_CAPS.map((cap) => (
            <button
              key={cap}
              type="button"
              className={`sa-chip${provider.caps.includes(cap) ? " is-on" : ""}`}
              onClick={() => toggleCap(cap)}
            >
              {CAP_LABELS[cap]}
            </button>
          ))}
        </div>
      </Field>
      <div className="sa-grid-2">
        <NumberField
          label="temperature"
          value={provider.temperature}
          min={0}
          max={2}
          step={0.1}
          onChange={(temperature) => props.onChange({ temperature })}
        />
        <NumberField
          label="maxTokens"
          value={provider.maxTokens}
          min={128}
          max={8192}
          step={128}
          onChange={(maxTokens) => props.onChange({ maxTokens })}
        />
      </div>
      <div className="sa-inline sa-provider-editor-foot">
        <Button variant="secondary" size="sm" disabled={props.testing} onClick={props.onTest}>
          {props.testing ? "测试中…" : "测试连接"}
        </Button>
        <span className="sa-field-hint">
          密钥状态：{provider.hasKey ? `已配置（${provider.keyMask ?? "••••"}）` : "未配置"}
        </span>
      </div>
      {props.testResult ? <TestResultBlock result={props.testResult} /> : null}
    </div>
  );
}

function TestResultBlock(props: { result: TestResult }) {
  const { result } = props;
  if (result.ok) {
    return (
      <div className="sa-testresult is-ok">
        <StatusTag tone="ok">连接成功</StatusTag>
        <span>
          延迟 {result.latencyMs ?? "—"} ms · 模型 {result.model ?? "未知"}
        </span>
      </div>
    );
  }
  const error = result.error;
  return (
    <div className="sa-testresult is-bad">
      <StatusTag tone="danger">连接失败</StatusTag>
      <span>
        {error?.code ? `${error.code} · ` : ""}
        {error?.message ?? `HTTP ${result.status ?? "无响应"}`}
      </span>
      {error?.fallback.length ? <span className="sa-field-hint">可尝试：{error.fallback.join("、")}</span> : null}
    </div>
  );
}
