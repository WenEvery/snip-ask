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

/* ───────────────────────── 版本类型 ───────────────────────── */

/** 契约 v5：版本类型文案（full=内置 OCR 模型版 / slim=安装后下载版 / dev=开发构建）。 */
const EDITION_LABELS: Record<string, string> = {
  full: "完整版（内置 OCR 模型）",
  slim: "精简版（安装后下载 OCR 模型）",
  dev: "开发构建",
};

export function editionLabel(edition: string | null | undefined): string {
  if (!edition) return "未知";
  return EDITION_LABELS[edition] ?? `未知（${edition}）`;
}

/** 版本类型短标签（徽标用）。 */
export function editionBadge(edition: string | null | undefined): string {
  if (edition === "full") return "完整版";
  if (edition === "slim") return "精简版";
  if (edition === "dev") return "开发构建";
  return "未知版本";
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
      label: "未读取 OCR 状态",
      detail: "点击「重新检查」读取本机 OCR 运行时与模型状态。",
    };
  }
  if (status.ready) {
    return {
      tone: "ok",
      label: status.bundled ? "OCR 已就绪（随包内置）" : "OCR 已就绪",
      detail:
        status.bundled || edition === "full"
          ? "运行时与全部 OCR 模型随安装包内置，离线开箱可用。"
          : "运行时与全部 OCR 模型已就位。",
    };
  }
  if (!status.runtimeReady) {
    return {
      tone: "danger",
      label: "缺少 ONNX 运行时",
      detail:
        status.message ??
        "未找到 onnxruntime.dll（应在安装目录 ocr/ 下，或由安装脚本随包提供）。重新安装完整版本可修复。",
    };
  }
  if (status.missing.length > 0) {
    return {
      tone: "warn",
      label: `缺少 ${status.missing.length} 个 OCR 模型`,
      detail:
        status.message ??
        `未就绪：${status.missing.join("、")}。精简版需首次使用 OCR 时下载模型（约 16 MB，det/rec/cls/dict）。`,
    };
  }
  return {
    tone: "warn",
    label: "OCR 未就绪",
    detail: status.message ?? "运行时或模型不完整，请检查安装目录或重新下载 OCR 模型。",
  };
}

/** 未安装且未随包的资产（决定「下载」按钮是否出现）。 */
export function pendingOcrRows(rows: OcrAssetRow[]): OcrAssetRow[] {
  return rows.filter((row) => !row.installed && !row.bundled);
}

/** 是否全部资产都随包内置（full 版徽标）。 */
export function allBundled(rows: OcrAssetRow[]): boolean {
  return rows.length > 0 && rows.every((row) => row.bundled);
}
