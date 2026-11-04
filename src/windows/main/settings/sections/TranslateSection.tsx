/** 设置 · 翻译：云端 / 本地后端、目标语言、术语表、模型库与试译 */

import { useState } from "react";
import { Button } from "@/components/ui";
import type { GlossaryEntry, TranslateBackend } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { buildPatch, hasChanges } from "../diff";
import { CUSTOM_LANG, LOCAL_ENGINES, TARGET_LANGS } from "../presets";
import {
  Field,
  Hint,
  InfoRow,
  NumberField,
  Panel,
  SaveBar,
  SectionShell,
  SegmentedControl,
  SelectField,
  TextInput,
} from "../parts";
import { ModelLibrary } from "../components/ModelLibrary";
import type { SectionProps } from "../types";

export function TranslateSection(props: SectionProps) {
  const { draft, baseline, saving, onEdit, onApply, onError } = props;
  const translate = draft.translate;
  const local = draft.local;
  const dirty = hasChanges(baseline, draft, ["translate", "local"]);

  const [customLang, setCustomLang] = useState(
    () => !TARGET_LANGS.some((item) => item.value === translate.targetLang),
  );
  const [testText, setTestText] = useState("The quick brown fox jumps over the lazy dog.");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const updateTranslate = (patch: Partial<typeof translate>) => {
    onEdit((current) => ({ ...current, translate: { ...current.translate, ...patch } }));
  };

  const updateLocal = (patch: Partial<typeof local>) => {
    onEdit((current) => ({ ...current, local: { ...current.local, ...patch } }));
  };

  const updateGlossary = (index: number, patch: Partial<GlossaryEntry>) => {
    onEdit((current) => ({
      ...current,
      translate: {
        ...current.translate,
        glossary: current.translate.glossary.map((entry, entryIndex) =>
          entryIndex === index ? { ...entry, ...patch } : entry,
        ),
      },
    }));
  };

  const addGlossary = () => updateTranslate({ glossary: [...translate.glossary, { from: "", to: "" }] });

  const removeGlossary = (index: number) =>
    updateTranslate({ glossary: translate.glossary.filter((_, entryIndex) => entryIndex !== index) });

  const runTranslateTest = async () => {
    if (!testText.trim()) {
      setNotice("请先输入要试译的文本");
      return;
    }
    setTesting(true);
    try {
      const result = await ipc.invoke<string>(COMMANDS.translateTest, {
        text: testText,
        target: translate.targetLang,
      });
      setTestResult(result);
    } catch (err) {
      onError(err);
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    const cleaned = {
      ...draft,
      translate: {
        ...translate,
        glossary: translate.glossary.filter((entry) => entry.from.trim() || entry.to.trim()),
      },
    };
    const patch = buildPatch(baseline, cleaned, ["translate", "local"]);
    if (Object.keys(patch).length === 0) {
      setNotice("翻译设置没有变化");
      return;
    }
    const ok = await onApply(patch, "翻译设置");
    if (ok) setNotice("翻译设置已保存；切换本地引擎会触发引擎重启");
  };

  const providerOptions = draft.providers.map((provider) => ({
    value: provider.id,
    label: `${provider.name}（${provider.model}）`,
    disabled: !provider.enabled,
  }));

  return (
    <SectionShell
      title="翻译"
      description="云端复用 AI 服务或使用专用翻译模型；本地模型离线运行、更重隐私。"
    >
      <Panel title="翻译后端">
        <SegmentedControl<TranslateBackend>
          value={translate.backend}
          onChange={(backend) => {
            updateTranslate({ backend });
            setNotice(backend === "local" ? "本地翻译需要先下载并安装模型" : "云端翻译将复用所选供应商");
          }}
          options={[
            { value: "cloud", label: "云端（推荐）", description: "复用 AI 服务 Key，质量高" },
            { value: "local", label: "本地（离线）", description: "隐私好，需下载模型" },
          ]}
        />
      </Panel>

      {translate.backend === "cloud" ? (
        <Panel title="云端翻译" description="可选择专用翻译模型（如 Qwen-MT）或任意通用模型。">
          <SelectField
            label="供应商"
            value={translate.providerId ?? ""}
            options={[
              { value: "", label: "跟随默认供应商" },
              ...providerOptions,
            ]}
            onChange={(providerId) => updateTranslate({ providerId: providerId || null })}
            hint="只列出已配置的 AI 供应商，停用的不可选"
          />
          <TextInput
            label="模型名"
            value={translate.model}
            placeholder="qwen-mt-lite"
            monospace
            hint="留空时使用供应商的默认模型"
            onChange={(model) => updateTranslate({ model: model.trim() })}
          />
        </Panel>
      ) : (
        <Panel title="本地翻译" description="本地引擎仅监听 127.0.0.1；空闲超时后自动卸载以节省内存。">
          <SelectField
            label="推理引擎"
            value={local.engine}
            options={LOCAL_ENGINES.map((engine) => ({ value: engine.value, label: engine.label }))}
            onChange={(value) =>
              updateLocal({ engine: value === "ollama" ? "ollama" : value === "ct2" ? "ct2" : "llama.cpp" })
            }
          />
          <TextInput
            label="模型"
            value={local.model}
            placeholder="hy-mt-1.8b-q4"
            monospace
            hint="llama.cpp / Ollama 使用模型名；CTranslate2 使用模型目录名"
            onChange={(model) => updateLocal({ model: model.trim() })}
          />
          <div className="sa-grid-2">
            <NumberField
              label="本地服务端口"
              value={local.port}
              min={1024}
              max={65535}
              suffix=""
              onChange={(port) => updateLocal({ port })}
            />
            <NumberField
              label="空闲卸载"
              value={local.idleUnloadSec}
              min={0}
              max={86400}
              step={30}
              suffix="秒"
              onChange={(idleUnloadSec) => updateLocal({ idleUnloadSec })}
            />
          </div>
          <ModelLibrary engine={local.engine} onError={onError} />
        </Panel>
      )}

      <Panel title="目标语言与术语表">
        <div className="sa-grid-2">
          <SelectField
            label="默认目标语言"
            value={customLang ? CUSTOM_LANG : translate.targetLang}
            options={[
              ...TARGET_LANGS,
              { value: CUSTOM_LANG, label: "自定义语言代码…" },
            ]}
            onChange={(value) => {
              if (value === CUSTOM_LANG) {
                setCustomLang(true);
                return;
              }
              setCustomLang(false);
              updateTranslate({ targetLang: value });
            }}
          />
          {customLang ? (
            <TextInput
              label="自定义目标语言"
              value={translate.targetLang}
              placeholder="例如 pt / ar"
              monospace
              hint="BCP-47 风格代码，如 zh-CHT、pt、ar"
              onChange={(targetLang) => updateTranslate({ targetLang })}
            />
          ) : (
            <InfoRow label="当前目标语言" value={translate.targetLang} mono />
          )}
        </div>
        <Field label="术语表" hint="翻译时强制使用的对照词；仅对云端与本地翻译同时生效。">
          <div className="sa-glossary">
            {translate.glossary.length === 0 ? (
              <p className="sa-hint">暂无术语，点击下方按钮添加。</p>
            ) : (
              <table className="sa-table">
                <thead>
                  <tr>
                    <th>原文</th>
                    <th>译文</th>
                    <th aria-label="操作" />
                  </tr>
                </thead>
                <tbody>
                  {translate.glossary.map((entry, index) => (
                    <tr key={index}>
                      <td>
                        <TextInput
                          value={entry.from}
                          placeholder="原文术语"
                          onChange={(from) => updateGlossary(index, { from })}
                        />
                      </td>
                      <td>
                        <TextInput
                          value={entry.to}
                          placeholder="译文术语"
                          onChange={(to) => updateGlossary(index, { to })}
                        />
                      </td>
                      <td>
                        <Button variant="ghost" size="sm" onClick={() => removeGlossary(index)}>
                          删除
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
            <Button variant="secondary" size="sm" onClick={addGlossary}>
              + 添加术语
            </Button>
          </div>
        </Field>
      </Panel>

      <Panel title="试译" description="调用 translate_test 用当前目标语言翻译一句示例文本，验证链路。">
        <TextInput
          label="示例文本"
          value={testText}
          placeholder="输入要试译的文本"
          onChange={setTestText}
        />
        <div className="sa-inline">
          <Button variant="secondary" size="sm" disabled={testing} onClick={() => void runTranslateTest()}>
            {testing ? "翻译中…" : "试译"}
          </Button>
          <span className="sa-field-hint">目标语言：{translate.targetLang}</span>
        </div>
        {testResult ? <pre className="sa-translate-result">{testResult}</pre> : null}
      </Panel>

      {notice ? <Hint>{notice}</Hint> : null}
      <SaveBar label="翻译设置" dirty={dirty} saving={saving} onSave={() => void save()} />
    </SectionShell>
  );
}
