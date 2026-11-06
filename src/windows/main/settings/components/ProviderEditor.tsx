/** AI 服务 · 单个供应商编辑表单（字段直接写回配置草稿，密钥只随测试连接提交） */

import type {
  Cap,
  Provider,
  ProviderKind,
  ProviderView,
  TestResult,
} from "@/shared/contract/types";
import { Button } from "@/components/ui";
import { t } from "@/i18n";
import { ALL_CAPS, CAP_LABELS } from "../presets";
import { Field, NumberField, SelectField, StatusTag, TextInput } from "../parts";

/** 契约 v5：供应商 API 类型（`openAiCompatible` 默认 / `qwenMt` 专用翻译 API）。 */
const PROVIDER_KIND_OPTIONS: { value: ProviderKind; label: string }[] = [
  { value: "openAiCompatible", label: t("OpenAI 兼容（Chat Completions）") },
  { value: "qwenMt", label: t("DashScope Qwen-MT（专用翻译）") },
];

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
    : t("仅允许小写字母、数字、- 和 _，长度 1–32");
  const urlError = URL_PATTERN.test(provider.baseUrl)
    ? undefined
    : t("仅允许 https:// 或 http://127.0.0.1[:端口]");

  return (
    <div className="sa-provider-editor">
      <div className="sa-grid-2">
        <TextInput
          label={t("显示名称")}
          value={provider.name}
          placeholder={t("例如 DeepSeek")}
          onChange={(name) => props.onChange({ name })}
        />
        <TextInput
          label={t("供应商 ID")}
          value={provider.id}
          placeholder={t("例如 my-provider")}
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
        hint={t("OpenAI 兼容服务填服务地址；本机服务请用 http://127.0.0.1:端口")}
        onChange={(baseUrl) => props.onChange({ baseUrl: baseUrl.trim() })}
      />
      <div className="sa-grid-2">
        <SelectField
          label={t("API 类型")}
          value={provider.kind}
          options={PROVIDER_KIND_OPTIONS}
          hint={t("多数服务选「OpenAI 兼容」；DashScope 的 Qwen-MT 是专门的翻译接口，只用于翻译")}
          onChange={(value) =>
            props.onChange({
              kind: value === "qwenMt" ? "qwenMt" : "openAiCompatible",
            })
          }
        />
        <TextInput
          label={t("模型名")}
          value={provider.model}
          placeholder="deepseek-flash"
          error={provider.model.trim() ? undefined : t("模型名不能为空")}
          monospace
          onChange={(model) => props.onChange({ model: model.trim() })}
        />
      </div>
      <TextInput
        label="API Key"
        value={props.apiKey}
        type="password"
        placeholder={provider.hasKey ? t("已保存；不修改请留空") : t("粘贴 API Key")}
        hint={t("保存在 Windows 凭据管理器，不会写进配置文件；点「测试连接」时会一并保存")}
        onChange={props.onApiKeyChange}
      />
      <Field
        label={t("能力标记")}
        hint={t("决定该供应商能被哪些动作选用（视觉 / 搜索 / 思考 / 翻译）。")}
      >
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
          {props.testing ? t("测试中…") : t("测试连接")}
        </Button>
        <span className="sa-field-hint">
          {t("密钥状态：{state}", {
            state: provider.hasKey
              ? t("已配置（{mask}）", { mask: provider.keyMask ?? "••••" })
              : t("未配置"),
          })}
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
        <StatusTag tone="ok">{t("连接成功")}</StatusTag>
        <span>
          {t("延迟 {latency} ms · 模型 {model}", {
            latency: result.latencyMs ?? "—",
            model: result.model ?? t("未知"),
          })}
        </span>
      </div>
    );
  }
  const error = result.error;
  return (
    <div className="sa-testresult is-bad">
      <StatusTag tone="danger">{t("连接失败")}</StatusTag>
      <span>
        {error?.code ? `${error.code} · ` : ""}
        {error?.message ?? t("HTTP {status}", { status: result.status ?? t("无响应") })}
      </span>
      {error?.fallback.length ? (
        <span className="sa-field-hint">
          {t("可以试试：{list}", { list: error.fallback.join("、") })}
        </span>
      ) : null}
    </div>
  );
}
