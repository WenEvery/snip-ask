/**
 * OCR 状态纯逻辑（无 React / 无 IPC，可用 `node --test` 直接跑）。
 *
 * 契约 v5：
 * - `ocr_status` 给出运行时 / 各 OCR 资产（det/rec/cls/dict）的安装与随包状态；
 * - `models_list` 中 `kind === "onnx"` 的条目提供下载 / 删除所需的元数据；
 * - `app_stats` 给出 `edition`（full / slim / dev）与 `ocrReady`。
 *
 * 本模块把三者合并成设置页 / 引导页共用的展示模型，避免 UI 里散落判断。
 */

import type { ModelInfo, OcrAssetStatus, OcrStatus } from "@/shared/contract/types";

/* ───────────────────────── 安装包类型 ───────────────────────── */

/** 安装包类型文案（full=内置 OCR 模型 / slim=首次使用时下载 / dev=本地运行）。 */
const EDITION_LABELS: Record<string, string> = {
  full: "已内置文字识别模型",
  slim: "首次使用时下载文字识别模型",
  dev: "本地运行",
};

export function editionLabel(edition: string | null | undefined): string {
  if (!edition) return "未知";
  return EDITION_LABELS[edition] ?? "未知";
}

/** 版本类型短标签（徽标用）。 */
export function editionBadge(edition: string | null | undefined): string {
  if (edition === "full") return "内置模型";
  if (edition === "slim") return "在线下载";
  if (edition === "dev") return "本地运行";
  return "未知";
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

/** `models_list` 中属于 OCR 的条目（契约 v5：`kind === "onnx"`）。 */
export function ocrModels(models: ModelInfo[]): ModelInfo[] {
  return models.filter((model) => model.kind === "onnx");
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
export function summarizeOcr(status: OcrStatus | null, edition?: string | null): OcrSummary {
  if (!status) {
    return {
      tone: "neutral",
      label: "还没读取文字识别状态",
      detail: "点击「重新检查」看看本机是否能正常识别文字。",
    };
  }
  if (status.ready) {
    return {
      tone: "ok",
      label: status.bundled ? "文字识别已就绪（随安装包内置）" : "文字识别已就绪",
      detail:
        status.bundled || edition === "full"
          ? "识别所需的组件和模型都已随安装包提供，离线也能用。"
          : "识别所需的组件和模型都已就位。",
    };
  }
  if (!status.runtimeReady) {
    return {
      tone: "danger",
      label: "缺少识别引擎组件",
      detail:
        status.message ?? "没有找到识别引擎文件，暂时无法识别文字。重新安装应用通常可以修复。",
    };
  }
  if (status.missing.length > 0) {
    return {
      tone: "warn",
      label: `还缺 ${status.missing.length} 个模型`,
      detail:
        status.message ??
        "首次使用文字识别需要下载模型（约 16 MB）。下载完成后即可使用，也可以稍后再下载。",
    };
  }
  return {
    tone: "warn",
    label: "文字识别暂不可用",
    detail: status.message ?? "识别所需的组件或模型不完整，可重新下载模型后再试。",
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

export function ocrAssetLabel(id: string, fallback: string): string {
  return ASSET_LABELS[id] ?? fallback;
}

/** 是否全部资产都随包内置（显示「随安装包内置」徽标）。 */
export function allBundled(rows: OcrAssetRow[]): boolean {
  return rows.length > 0 && rows.every((row) => row.bundled);
}
