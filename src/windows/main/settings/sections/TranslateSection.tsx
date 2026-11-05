/** 设置 · 翻译：云端 / 本地翻译、显卡加速、目标语言、术语表、模型库与试译 */

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui";
import type {
  BackendInfo,
  DownloadProgressPayload,
  DownloadState,
  DownloadStatePayload,
  GpuBackend,
  GpuProbeResult,
  GlossaryEntry,
  LocalDevice,
  TranslateBackend,
} from "@/shared/contract/types";
import { COMMANDS, EVENTS } from "@/shared/contract/names";
import { ipc } from "@/shared/ipc";
import { useIpcEvent } from "@/shared/hooks/useIpcEvent";
import { buildPatch, hasChanges } from "../diff";
import { formatBytes } from "../format";
import {
  BACKEND_STATE_LABELS,
  CUSTOM_LANG,
  DEVICE_OPTIONS,
  GPU_BACKEND_OPTIONS,
  LOCAL_ENGINES,
  TARGET_LANGS,
  describeGpuProbe,
  formatMiB,
} from "../presets";
import {
  Field,
  Hint,
  InfoRow,
  LoadingBlock,
  NumberField,
  Panel,
  SaveBar,
  SectionShell,
  SegmentedControl,
  SelectField,
  StatusTag,
  TextInput,
} from "../parts";
import { ModelLibrary } from "../components/ModelLibrary";
import { OcrStatusCard } from "../../ocr/OcrStatusCard";
import type { SectionProps } from "../types";

/** 后端组件 id（`download:*` 事件按 `model` 字段区分组件与模型）。 */
const BACKEND_COMPONENT_IDS = new Set(["vulkan", "cuda", "cpu"]);

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

  // ── 本地 GPU 推理：真实探测 + 后端组件按需下载 ──
  const [probe, setProbe] = useState<GpuProbeResult | null>(null);
  const [probeLoading, setProbeLoading] = useState(false);
  const [backends, setBackends] = useState<BackendInfo[]>([]);
  const [backendsLoading, setBackendsLoading] = useState(false);
  const [backendBusy, setBackendBusy] = useState<string | null>(null);
  const [backendStates, setBackendStates] = useState<Record<string, DownloadState>>({});
  const [backendProgress, setBackendProgress] = useState<Record<string, DownloadProgressPayload>>(
    {},
  );

  const refreshProbe = useCallback(
    async (refresh: boolean) => {
      setProbeLoading(true);
      try {
        const result = await ipc.invoke<GpuProbeResult>(COMMANDS.localGpuProbe, { refresh });
        setProbe(result);
      } catch (err) {
        onError(err);
      } finally {
        setProbeLoading(false);
      }
    },
    [onError],
  );

  const refreshBackends = useCallback(async () => {
    setBackendsLoading(true);
    try {
      const list = await ipc.invoke<BackendInfo[]>(COMMANDS.backendsList);
      setBackends(list);
    } catch (err) {
      onError(err);
    } finally {
      setBackendsLoading(false);
    }
  }, [onError]);

  useEffect(() => {
    if (translate.backend !== "local") return;
    void refreshBackends();
    void refreshProbe(false);
  }, [translate.backend, refreshBackends, refreshProbe]);

  useIpcEvent<DownloadProgressPayload>(EVENTS.downloadProgress, (payload) => {
    if (!BACKEND_COMPONENT_IDS.has(payload.model)) return;
    setBackendProgress((current) => ({ ...current, [payload.model]: payload }));
  });

  useIpcEvent<DownloadStatePayload>(EVENTS.downloadState, (payload) => {
    if (!BACKEND_COMPONENT_IDS.has(payload.model)) return;
    setBackendStates((current) => ({ ...current, [payload.model]: payload.state }));
    if (
      payload.state === "installed" ||
      payload.state === "failed" ||
      payload.state === "cancelled"
    ) {
      void refreshBackends();
      if (payload.state === "installed") {
        // 组件就位后 GPU 可用性变化：强制刷新探测（绕过 60s 缓存）。
        void refreshProbe(true);
      }
    }
  });

  const startBackendDownload = async (id: string) => {
    setBackendBusy(id);
    try {
      await ipc.invoke<string>(COMMANDS.backendsDownload, { id });
      setBackendStates((current) => ({ ...current, [id]: "queued" }));
    } catch (err) {
      onError(err);
    } finally {
      setBackendBusy(null);
    }
  };

  const cancelBackendDownload = async (id: string) => {
    setBackendBusy(id);
    try {
      await ipc.invoke<void>(COMMANDS.backendsCancel, { id });
    } catch (err) {
      onError(err);
    } finally {
      setBackendBusy(null);
    }
  };

  const removeBackend = async (id: string) => {
    setBackendBusy(id);
    try {
      await ipc.invoke<void>(COMMANDS.backendsRemove, { id });
      setNotice(`已删除加速组件「${id}」`);
      await refreshBackends();
      await refreshProbe(true);
    } catch (err) {
      onError(err);
    } finally {
      setBackendBusy(null);
    }
  };

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

  const addGlossary = () =>
    updateTranslate({ glossary: [...translate.glossary, { from: "", to: "" }] });

  const removeGlossary = (index: number) =>
    updateTranslate({
      glossary: translate.glossary.filter((_, entryIndex) => entryIndex !== index),
    });

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
    if (ok) setNotice("翻译设置已保存；切换引擎或计算设备后，本地引擎会自动重启。");
  };

  const providerOptions = draft.providers.map((provider) => ({
    value: provider.id,
    label: `${provider.name}（${provider.model}）`,
    disabled: !provider.enabled,
  }));

  return (
    <SectionShell
      title="翻译"
      description="可以选择云端翻译（更快、效果更好，需要 API Key），也可以用本地模型离线翻译。"
    >
      <Panel title="翻译方式">
        <SegmentedControl<TranslateBackend>
          value={translate.backend}
          onChange={(backend) => {
            updateTranslate({ backend });
            setNotice(
              backend === "local" ? "本地翻译需要先下载模型" : "云端翻译会使用你配置的 AI 服务",
            );
          }}
          options={[
            { value: "cloud", label: "云端翻译", description: "用已配置的 AI 服务，效果好" },
            { value: "local", label: "本地翻译", description: "离线可用，不上传内容" },
          ]}
        />
      </Panel>

      <Panel
        title="文字识别（框选识别）"
        description="框选后的文字识别在本机完成，不联网。识别需要的模型随安装包提供，或在首次使用时自动下载。"
      >
        <OcrStatusCard />
      </Panel>

      {translate.backend === "cloud" ? (
        <Panel title="云端翻译" description="可以使用专用翻译模型，也可以直接用普通的 AI 模型。">
          <SelectField
            label="供应商"
            value={translate.providerId ?? ""}
            options={[{ value: "", label: "使用默认供应商" }, ...providerOptions]}
            onChange={(providerId) => updateTranslate({ providerId: providerId || null })}
            hint="只列出已配置的供应商，停用的不可选"
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
        <>
          <Panel title="本地翻译" description="本地引擎只在本机运行；长时间不用会自动释放内存。">
            <SelectField
              label="本地引擎"
              value={local.engine}
              options={LOCAL_ENGINES.map((engine) => ({
                value: engine.value,
                label: engine.label,
              }))}
              onChange={(value) =>
                updateLocal({
                  engine: value === "ollama" ? "ollama" : value === "ct2" ? "ct2" : "llama.cpp",
                })
              }
            />
            <TextInput
              label="模型"
              value={local.model}
              placeholder="hy-mt-1.8b-q4"
              monospace
              hint="填写本地模型的名称；不同引擎的填法略有不同"
              onChange={(model) => updateLocal({ model: model.trim() })}
            />
            <div className="sa-grid-2">
              <NumberField
                label="本机端口"
                value={local.port}
                min={1024}
                max={65535}
                suffix=""
                hint="一般保持默认即可"
                onChange={(port) => updateLocal({ port })}
              />
              <NumberField
                label="闲置多久后释放内存"
                value={local.idleUnloadSec}
                min={0}
                max={86400}
                step={30}
                suffix="秒"
                hint="设为 0 表示不自动释放"
                onChange={(idleUnloadSec) => updateLocal({ idleUnloadSec })}
              />
            </div>
            <ModelLibrary engine={local.engine} onError={onError} />
          </Panel>

          <Panel
            title="显卡加速"
            description="有显卡时会优先用显卡加速，没有也能用 CPU。NVIDIA 用户还可以额外启用 CUDA。"
            actions={
              <Button
                variant="secondary"
                size="sm"
                disabled={probeLoading}
                onClick={() => void refreshProbe(true)}
              >
                {probeLoading ? "检测中…" : "重新检测"}
              </Button>
            }
          >
            <div className="sa-grid-3">
              <SelectField
                label="计算设备"
                value={local.device}
                options={DEVICE_OPTIONS.map((option) => ({
                  value: option.value,
                  label: option.label,
                  disabled: false,
                }))}
                onChange={(value) => updateLocal({ device: value as LocalDevice })}
                hint="自动：有显卡用显卡，否则用 CPU"
              />
              <SelectField
                label="显卡加速方式"
                value={local.gpuBackend}
                options={GPU_BACKEND_OPTIONS.map((option) => ({
                  value: option.value,
                  label: option.label,
                  disabled: false,
                }))}
                onChange={(value) => updateLocal({ gpuBackend: value as GpuBackend })}
                disabled={local.device === "cpu"}
                hint="大多数显卡选 Vulkan；CUDA 仅支持 NVIDIA"
              />
              <NumberField
                label="交给显卡的比例"
                value={local.gpuLayers}
                min={0}
                max={512}
                hint="数值越大，越多计算交给显卡；显存不够时调小"
                disabled={local.device === "cpu"}
                onChange={(gpuLayers) => updateLocal({ gpuLayers })}
              />
            </div>
            <InfoRow
              label="设备状态"
              value={probeLoading ? "正在检测…" : probe ? describeGpuProbe(probe) : "还没有检测"}
              hint="自动检测显卡型号和可用显存"
            />
            {local.device === "cpu" ? <Hint>已选择 CPU：更省电但速度较慢。</Hint> : null}
            {probe?.budget && !probe.budget.fits ? (
              <div className="sa-warn" role="status">
                <span>
                  这个模型大约需要 {formatMiB(probe.budget.requiredMb)} 显存，当前空闲{" "}
                  {formatMiB(probe.budget.availableMb)}，可能不够用（会变慢或改用 CPU）。
                </span>
                <span className="sa-inline">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => updateLocal({ device: "cpu", gpuLayers: 0 })}
                  >
                    改用 CPU
                  </Button>
                </span>
              </div>
            ) : null}
            {!probeLoading && probe && !probe.useable ? (
              <Hint>
                没有检测到可用的显卡加速，将使用 CPU。可在上方「显卡加速方式」下载 Vulkan 组件（AMD
                / Intel / NVIDIA 都适用）。
              </Hint>
            ) : null}

            <Field
              label="加速组件"
              hint="Vulkan 是通用组件（AMD / Intel / NVIDIA 都可用）；CUDA 只适用于 NVIDIA 显卡；CPU 随安装包提供。下载会自动继续，也能随时取消。"
            >
              {backendsLoading && backends.length === 0 ? (
                <LoadingBlock text="正在读取加速组件…" />
              ) : (
                <div className="sa-models">
                  {backends.length === 0 ? <Hint>这里暂时没有可下载的加速组件。</Hint> : null}
                  {backends.map((component) => {
                    const state = backendStates[component.id] ?? component.downloadState;
                    const installed = state === "installed" || component.installed;
                    const active =
                      state === "queued" || state === "downloading" || state === "verifying";
                    const progress = backendProgress[component.id];
                    const percent =
                      progress && progress.total > 0
                        ? Math.min(100, Math.round((progress.loaded / progress.total) * 100))
                        : 0;
                    const vendorHint = component.requiresVendor === "nvidia" ? " · 仅 NVIDIA" : "";
                    return (
                      <div key={component.id} className="sa-model-row">
                        <div className="sa-model-main">
                          <div className="sa-model-title">
                            <strong>{component.label}</strong>
                            <code className="sa-mono">{component.id}</code>
                            <StatusTag
                              tone={
                                installed
                                  ? "ok"
                                  : active
                                    ? "warn"
                                    : state === "failed"
                                      ? "danger"
                                      : "neutral"
                              }
                            >
                              {BACKEND_STATE_LABELS[state]}
                            </StatusTag>
                          </div>
                          <p className="sa-model-meta">
                            {formatBytes(component.sizeBytes)}
                            {component.version ? ` · 版本 ${component.version}` : ""}
                            {vendorHint}
                          </p>
                          {active || (installed && component.installedBytes > 0) ? (
                            <div className="sa-progress">
                              <div className="sa-progressbar">
                                <div
                                  className="sa-progressbar-fill"
                                  style={{ width: `${installed ? 100 : percent}%` }}
                                />
                              </div>
                              <span className="sa-progress-text">
                                {installed
                                  ? `已安装 ${formatBytes(component.installedBytes || component.sizeBytes)}`
                                  : progress
                                    ? `${formatBytes(progress.loaded)} / ${formatBytes(progress.total)} · ${percent}%`
                                    : "正在准备下载…"}
                              </span>
                            </div>
                          ) : null}
                          {state === "failed" ? (
                            <p className="sa-field-hint">
                              下载失败，可以重试；如果网络不稳定，稍后再试也可以。
                            </p>
                          ) : null}
                        </div>
                        <div className="sa-model-actions">
                          {installed ? (
                            component.id === "cpu" ? (
                              <StatusTag tone="ok">随包</StatusTag>
                            ) : (
                              <Button
                                variant="danger"
                                size="sm"
                                disabled={backendBusy === component.id}
                                onClick={() => void removeBackend(component.id)}
                              >
                                删除
                              </Button>
                            )
                          ) : active ? (
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={backendBusy === component.id}
                              onClick={() => void cancelBackendDownload(component.id)}
                            >
                              取消
                            </Button>
                          ) : (
                            <Button
                              variant="secondary"
                              size="sm"
                              disabled={backendBusy === component.id}
                              onClick={() => void startBackendDownload(component.id)}
                            >
                              {backendBusy === component.id
                                ? "启动中…"
                                : state === "failed"
                                  ? "重试下载"
                                  : "下载"}
                            </Button>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </Field>
          </Panel>
        </>
      )}

      <Panel title="目标语言与术语表">
        <div className="sa-grid-2">
          <SelectField
            label="默认目标语言"
            value={customLang ? CUSTOM_LANG : translate.targetLang}
            options={[...TARGET_LANGS, { value: CUSTOM_LANG, label: "自定义语言代码…" }]}
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
              hint="填写语言代码，例如 en、ja、pt"
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

      <Panel title="试译" description="用当前设置翻译一段示例文字，看看效果和速度。">
        <TextInput
          label="示例文本"
          value={testText}
          placeholder="输入要试译的文本"
          onChange={setTestText}
        />
        <div className="sa-inline">
          <Button
            variant="secondary"
            size="sm"
            disabled={testing}
            onClick={() => void runTranslateTest()}
          >
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
