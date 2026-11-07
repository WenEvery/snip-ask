/**
 * OCR 状态纯逻辑（无 React / 无 IPC，可用 `node --test` 直接跑）。
 *
 * 国际化：本模块被 `node --test` 直接 import，不能依赖 `@/i18n`（Node 无法解析别名/无扩展名
 * 导入）。面向用户的文案通过可选参数 `translate` 注入（UI 传 `t`），默认实现保留中文并
 * 按 `{name}` 插值，保证纯逻辑测试与无 DOM 环境行为不变。
 *
 * 契约 v5：
 * - `ocr_status` 给出运行时 / 各 OCR 资产（det/rec/cls/dict）的安装与随包状态；
 * - `models_list` 中 `kind === "onnx"` 的条目提供下载 / 删除所需的元数据；
 * - `app_stats` 给出 `edition`（full / slim / dev）与 `ocrReady`。
 *
 * 本模块把三者合并成设置页 / 引导页共用的展示模型，避免 UI 里散落判断。
 */

import type {
  ModelInfo,
  OcrAssetStatus,
  OcrLanguage,
  OcrLanguageStatus,
  OcrStatus,
} from "@/shared/contract/types";

/** 与 `src/i18n` 的 `t(zh, vars?)` 同形；默认实现仅插值、不翻译（中文回退）。 */
export type TranslateText = (zh: string, vars?: Record<string, string | number>) => string;

function interpolate(zh: string, vars?: Record<string, string | number>): string {
  if (!vars) return zh;
  return zh.replace(/\{([a-zA-Z0-9_]+)\}/g, (whole, name: string) => {
    const value = vars[name];
    return value === undefined ? whole : String(value);
  });
}

const identityTranslate: TranslateText = (zh, vars) => interpolate(zh, vars);

/* ───────────────────────── 安装包类型 ───────────────────────── */

/** 安装包类型文案（full=内置 OCR 模型 / slim=首次使用时下载 / dev=本地运行）。 */
const EDITION_LABELS: Record<string, string> = {
  full: "已内置文字识别模型",
  slim: "首次使用时下载文字识别模型",
  dev: "本地运行",
};

export function editionLabel(
  edition: string | null | undefined,
  translate: TranslateText = identityTranslate,
): string {
  if (!edition) return translate("未知");
  return translate(EDITION_LABELS[edition] ?? "未知");
}

/** 版本类型短标签（徽标用）。 */
export function editionBadge(
  edition: string | null | undefined,
  translate: TranslateText = identityTranslate,
): string {
  if (edition === "full") return translate("内置模型");
  if (edition === "slim") return translate("在线下载");
  if (edition === "dev") return translate("本地运行");
  return translate("未知");
}

/* ───────────────────────── OCR 资产合并 ───────────────────────── */

/** 一行 OCR 资产（ocr_status 的资产 + 可选的 models_list 元数据）。 */
export interface OcrAssetRow {
  id: string;
  name: string;
  installed: boolean;
  bundled: boolean;
  sizeBytes: number;
  /** 对应 `models_list` 中 `kind === "onnx"` 的条目（真实后端有；mock / 旧版可能没有）。 */
  model: ModelInfo | null;
}

/** `models_list` 中属于 OCR 的条目（契约 v5：`kind === "onnx"`；语言包不计入基础 4 项资产）。 */
export function ocrModels(models: ModelInfo[]): ModelInfo[] {
  return models.filter((model) => model.kind === "onnx" && !isOcrLanguagePackId(model.id));
}

/* ───────────────────── 识别语言与语言包（契约 v7 增量） ───────────────────── */

/** `OcrLanguage` 的展示顺序（与后端 `OcrLanguage::ALL` 一致）。 */
export const OCR_LANGUAGES: readonly OcrLanguage[] = ["zh", "ru", "ja", "ko"];

/** 语言包资产 id 前缀：`ocr-lang-<language>`（`zh` 没有语言包）。 */
const LANGUAGE_PACK_ID_PREFIX = "ocr-lang-";

const LANGUAGE_LABELS: Record<OcrLanguage, string> = {
  zh: "中文（含英文）",
  ru: "俄语",
  ja: "日语",
  ko: "韩语",
};

/** 契约 §0.3 的语言包字节数（`models_list` 缺条目时兜底展示，避免出现 0 B）。 */
const LANGUAGE_PACK_FALLBACK_BYTES: Record<Exclude<OcrLanguage, "zh">, number> = {
  ru: 8_074_092,
  ja: 9_753_335,
  ko: 13_488_748,
};

export function isOcrLanguage(value: string): value is OcrLanguage {
  return (OCR_LANGUAGES as readonly string[]).includes(value);
}

/** 面向用户的识别语言名称（`translate` 注入 UI 的 `t`）。 */
export function ocrLanguageLabel(
  language: OcrLanguage,
  translate: TranslateText = identityTranslate,
): string {
  return translate(LANGUAGE_LABELS[language] ?? language);
}

/** 语言包资产 id；`zh` 使用基础 rec/dict 资产，返回 `null`。 */
export function ocrLanguagePackId(language: OcrLanguage): string | null {
  return language === "zh" ? null : `${LANGUAGE_PACK_ID_PREFIX}${language}`;
}

/** `models_list` / `download:*` 事件里的 id 是否为语言包资产。 */
export function isOcrLanguagePackId(id: string): boolean {
  return /^ocr-lang-(ru|ja|ko)$/.test(id);
}

/**
 * 该识别语言是否可用：`zh` 使用基础 rec/dict 资产，恒为可用；
 * 状态缺失（旧后端未返回 `languages`）时不做限制，避免把合法选项全部禁用。
 */
export function languageInstalled(status: OcrStatus | null, language: OcrLanguage): boolean {
  if (language === "zh") return true;
  const entries = status?.languages;
  if (!entries || entries.length === 0) return true;
  return entries.some((item) => item.language === language && item.installed);
}

/** 未安装的语言包（不含 `zh`），决定「语言包」下载行是否出现。 */
export function pendingLanguagePacks(status: OcrStatus | null): OcrLanguageStatus[] {
  return (status?.languages ?? []).filter((item) => !item.installed && item.language !== "zh");
}

/** 识别语言下拉选项：未安装的语言包后缀「（未下载）」并禁用。 */
export function ocrLanguageOptions(
  status: OcrStatus | null,
  translate: TranslateText = identityTranslate,
): { value: OcrLanguage; label: string; disabled: boolean }[] {
  return OCR_LANGUAGES.map((language) => {
    const label = ocrLanguageLabel(language, translate);
    const installed = languageInstalled(status, language);
    return {
      value: language,
      label: installed ? label : `${label}${translate("（未下载）")}`,
      disabled: !installed,
    };
  });
}

/** 语言包在 `models_list` 中的条目（用于大小与下载状态展示）。 */
export function languagePackModel(models: ModelInfo[], language: OcrLanguage): ModelInfo | null {
  const id = ocrLanguagePackId(language);
  if (!id) return null;
  return models.find((model) => model.id === id) ?? null;
}

/** 语言包大小：优先取 `models_list`，缺失时用契约固定大小兜底。 */
export function languagePackSize(models: ModelInfo[], language: OcrLanguage): number {
  if (language === "zh") return 0;
  return languagePackModel(models, language)?.sizeBytes ?? LANGUAGE_PACK_FALLBACK_BYTES[language];
}

/**
 * 合并 OCR 资产与模型清单：以 `ocr_status.assets` 顺序为准，
 * 缺失的资产用 `models_list` 补位（真实后端两者应一致，这里容忍差异）。
 */
export function mergeOcrRows(status: OcrStatus | null, models: ModelInfo[]): OcrAssetRow[] {
  const onnx = ocrModels(models);
  const byId = new Map(onnx.map((model) => [model.id, model]));
  const rows: OcrAssetRow[] = [];
  const seen = new Set<string>();

  for (const asset of status?.assets ?? []) {
    rows.push(toRow(asset, byId.get(asset.id) ?? null));
    seen.add(asset.id);
  }
  for (const model of onnx) {
    if (seen.has(model.id)) continue;
    rows.push({
      id: model.id,
      name: model.name,
      installed: model.installed,
      bundled: false,
      sizeBytes: model.sizeBytes,
      model,
    });
  }
  return rows;
}

function toRow(asset: OcrAssetStatus, model: ModelInfo | null): OcrAssetRow {
  return {
    id: asset.id,
    name: asset.name,
    installed: asset.installed,
    bundled: asset.bundled,
    sizeBytes: asset.sizeBytes,
    model,
  };
}

/* ───────────────────────── 状态摘要 ───────────────────────── */

export type OcrTone = "ok" | "warn" | "danger" | "neutral";

export interface OcrSummary {
  tone: OcrTone;
  /** 一句话状态（徽标 / 标题）。 */
  label: string;
  /** 可操作说明（缺失什么 / 去哪下载）。 */
  detail: string;
}

/** 把 `ocr_status` 收敛成一句状态 + 可操作说明。 */
export function summarizeOcr(
  status: OcrStatus | null,
  edition?: string | null,
  translate: TranslateText = identityTranslate,
): OcrSummary {
  if (!status) {
    return {
      tone: "neutral",
      label: translate("还没读取文字识别状态"),
      detail: translate("点击「重新检查」看看本机是否能正常识别文字。"),
    };
  }
  if (status.ready) {
    return {
      tone: "ok",
      label: status.bundled
        ? translate("文字识别已就绪（随安装包内置）")
        : translate("文字识别已就绪"),
      detail:
        status.bundled || edition === "full"
          ? translate("识别所需的组件和模型都已随安装包提供，离线也能用。")
          : translate("识别所需的组件和模型都已就位。"),
    };
  }
  if (!status.runtimeReady) {
    return {
      tone: "danger",
      label: translate("缺少识别引擎组件"),
      detail:
        status.message ??
        translate("没有找到识别引擎文件，暂时无法识别文字。重新安装应用通常可以修复。"),
    };
  }
  if (status.missing.length > 0) {
    return {
      tone: "warn",
      label: translate("还缺 {count} 个模型", { count: status.missing.length }),
      detail:
        status.message ??
        translate(
          "首次使用文字识别需要下载模型（约 16 MB）。下载完成后即可使用，也可以稍后再下载。",
        ),
    };
  }
  return {
    tone: "warn",
    label: translate("文字识别暂不可用"),
    detail: status.message ?? translate("识别所需的组件或模型不完整，可重新下载模型后再试。"),
  };
}

/** 未安装且未随包的资产（决定「下载」按钮是否出现）。 */
export function pendingOcrRows(rows: OcrAssetRow[]): OcrAssetRow[] {
  return rows.filter((row) => !row.installed && !row.bundled);
}

/** 面向用户的资产名称（后端名称含模型代号，这里换成白话）。 */
const ASSET_LABELS: Record<string, string> = {
  "ocr-det": "文字检测模型",
  "ocr-rec": "文字识别模型",
  "ocr-cls": "文字方向校正",
  "ocr-dict": "识别字典",
};

export function ocrAssetLabel(
  id: string,
  fallback: string,
  translate: TranslateText = identityTranslate,
): string {
  const label = ASSET_LABELS[id];
  return label ? translate(label) : fallback;
}

/** 是否全部资产都随包内置（显示「随安装包内置」徽标）。 */
export function allBundled(rows: OcrAssetRow[]): boolean {
  return rows.length > 0 && rows.every((row) => row.bundled);
}
