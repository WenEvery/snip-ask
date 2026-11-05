/**
 * 覆盖翻译 Canvas 覆盖层（研究 §7.1 / 实施规范 §3.5）。
 *
 * 渲染层次（均位于 overlay 根内、冻结帧 `<img class="overlay-frame">` 之上）：
 * 1. `translation-canvas`：逐块「模糊原文补丁 + 半透明遮罩 + 原位译文」；
 *    深/浅底自动字色（对比度 <4.5:1 加衬底）、失败块红框保留原文、溢出块降级为小气泡。
 * 2. `translation-toolbar`：原文 / 双语 / 覆盖三态切换 + 重译 / 重新截屏 / 关闭。
 *
 * 三态语义（研究 §7.2）：
 * - `cover`：模糊原文并原位绘制译文（默认）；
 * - `bilingual`：原文不模糊，译文以浮层形式画在块下方（放不下时上方/块内），可对照阅读；
 * - `original`：隐藏覆盖层，恢复冻结帧原样。
 *
 * 坐标口径：全程物理像素；canvas 内部缓冲 = 虚拟桌面物理尺寸，绘制时减去 `virtualRect` 原点，
 * CSS 尺寸由 100% 铺满（overlay client 区 == virtualRect），DPR 换算只发生这一次。
 */

import { useCallback, useEffect, useRef, useState } from "react";

import { Button, Spinner } from "../../components/ui";
import type { CoveredBlock, FrozenFrameMeta, OcrBox, RectPhys } from "../../shared/contract/types";
import {
  estimateTextWidth,
  mergeLines,
  planCover,
  type CoverPlan,
  type CoverPlanItem,
  type OcrLine,
  type PlanBlockInput,
} from "./translationLayout";

/** 覆盖层三态。 */
export type CoverView = "cover" | "bilingual" | "original";

/** 前端覆盖阶段（后端会话仍保持 showingResult，见 spec §3.5 偏差说明）。 */
export type CoverPhase = "translating" | "covered" | "failed";

export interface TranslationOverlayProps {
  /** 冻结帧（必须与 OCR 框同源：同一 session / 同一 virtualRect / 同一 dpr）。 */
  frame: FrozenFrameMeta;
  virtualRect: RectPhys;
  dpr: number;
  /** 选区命中的 OCR 行（绝对物理像素）。 */
  boxes: OcrBox[];
  /** 后端逐块结果（按块 id 索引；允许分批到达）。 */
  results: Record<string, CoveredBlock>;
  phase: CoverPhase;
  view: CoverView;
  /** 批处理进度（`translate:progress`）。 */
  progress: { done: number; total: number } | null;
  /** 实际使用的引擎（`skeleton` / 供应商 id / `local`）。 */
  engine: string | null;
  onViewChange: (view: CoverView) => void;
  onRetranslate: () => void;
  onRecapture: () => void;
  onClose: () => void;
}

const FONT_FAMILY = `'Segoe UI', 'Microsoft YaHei', 'PingFang SC', system-ui, sans-serif`;
const FAILED_COLOR = "#FF6B6B";

const EMPTY_PLAN: CoverPlan = {
  items: [],
  total: 0,
  sameCount: 0,
  shrinkCount: 0,
  wrapCount: 0,
  bubbleCount: 0,
  medianFontRatio: 0,
  minFontPx: 0,
};

interface CanvasRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

function toCanvasRect(rect: RectPhys, virtualRect: RectPhys): CanvasRect {
  return {
    x: rect.x - virtualRect.x,
    y: rect.y - virtualRect.y,
    width: rect.width,
    height: rect.height,
  };
}

function fontFor(fontSizePx: number): string {
  return `${fontSizePx}px ${FONT_FAMILY}`;
}

function roundRectPath(ctx: CanvasRenderingContext2D, rect: CanvasRect, radius: number): void {
  const r = Math.max(0, Math.min(radius, rect.width / 2, rect.height / 2));
  ctx.beginPath();
  ctx.moveTo(rect.x + r, rect.y);
  ctx.lineTo(rect.x + rect.width - r, rect.y);
  ctx.quadraticCurveTo(rect.x + rect.width, rect.y, rect.x + rect.width, rect.y + r);
  ctx.lineTo(rect.x + rect.width, rect.y + rect.height - r);
  ctx.quadraticCurveTo(
    rect.x + rect.width,
    rect.y + rect.height,
    rect.x + rect.width - r,
    rect.y + rect.height,
  );
  ctx.lineTo(rect.x + r, rect.y + rect.height);
  ctx.quadraticCurveTo(rect.x, rect.y + rect.height, rect.x, rect.y + rect.height - r);
  ctx.lineTo(rect.x, rect.y + r);
  ctx.quadraticCurveTo(rect.x, rect.y, rect.x + r, rect.y);
  ctx.closePath();
}

function srgbToLinear(channel: number): number {
  const normalized = channel / 255;
  return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
}

/** 采样一块区域的背景色（相对亮度 + 平均 RGB）：用于深/浅字色与「向局部背景色收敛」的遮罩。 */
interface SampledBackground {
  luminance: number;
  r: number;
  g: number;
  b: number;
}

function createBackgroundSampler(
  image: HTMLImageElement | null,
): ((rect: RectPhys, virtualRect: RectPhys) => SampledBackground | null) | null {
  if (!image || typeof document === "undefined") return null;
  const canvas = document.createElement("canvas");
  const SAMPLE_EDGE = 8;
  canvas.width = SAMPLE_EDGE;
  canvas.height = SAMPLE_EDGE;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return null;
  return (rect, virtualRect) => {
    try {
      const scaleX = image.naturalWidth / Math.max(1, virtualRect.width);
      const scaleY = image.naturalHeight / Math.max(1, virtualRect.height);
      const sx = Math.max(0, (rect.x - virtualRect.x) * scaleX);
      const sy = Math.max(0, (rect.y - virtualRect.y) * scaleY);
      const width = Math.min(rect.width * scaleX, Math.max(1, image.naturalWidth - sx));
      const height = Math.min(rect.height * scaleY, Math.max(1, image.naturalHeight - sy));
      if (width <= 0 || height <= 0) return null;
      ctx.clearRect(0, 0, SAMPLE_EDGE, SAMPLE_EDGE);
      ctx.drawImage(image, sx, sy, width, height, 0, 0, SAMPLE_EDGE, SAMPLE_EDGE);
      const data = ctx.getImageData(0, 0, SAMPLE_EDGE, SAMPLE_EDGE).data;
      // 取中位亮度像素（而不是平均值）：文字像素只占少数，平均值会被亮字拉高，
      // 中位数更接近「局部背景色」，用于 flatten 遮罩与深/浅字色判定。
      const pixels: Array<{ luminance: number; r: number; g: number; b: number }> = [];
      for (let index = 0; index < data.length; index += 4) {
        const r = data[index];
        const g = data[index + 1];
        const b = data[index + 2];
        pixels.push({
          luminance: 0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b),
          r,
          g,
          b,
        });
      }
      if (pixels.length === 0) return null;
      pixels.sort((left, right) => left.luminance - right.luminance);
      return pixels[Math.floor(pixels.length / 2)];
    } catch {
      // 画布不可读（极端情况下 data URL 被污染）→ 调用方用默认深/浅判定
      return null;
    }
  };
}

/** 覆盖层组件。 */
export function TranslationOverlay({
  frame,
  virtualRect,
  dpr,
  boxes,
  results,
  phase,
  view,
  progress,
  engine,
  onViewChange,
  onRetranslate,
  onRecapture,
  onClose,
}: TranslationOverlayProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const measureCtxRef = useRef<CanvasRenderingContext2D | null | undefined>(undefined);
  const [imageReady, setImageReady] = useState(0);
  const [plan, setPlan] = useState<CoverPlan>(EMPTY_PLAN);
  const [failedRects, setFailedRects] = useState<RectPhys[]>([]);
  const [backgrounds, setBackgrounds] = useState<Record<string, SampledBackground>>({});

  /* ───────── 冻结帧加载（用于模糊补丁与背景亮度采样） ───────── */

  useEffect(() => {
    if (typeof window === "undefined") return undefined;
    if (!frame.dataUrl) {
      imageRef.current = null;
      setImageReady((value) => value + 1);
      return undefined;
    }
    const image = new Image();
    let disposed = false;
    image.onload = () => {
      if (disposed) return;
      imageRef.current = image;
      setImageReady((value) => value + 1);
    };
    image.onerror = () => {
      if (disposed) return;
      imageRef.current = null;
      setImageReady((value) => value + 1);
    };
    image.src = frame.dataUrl;
    return () => {
      disposed = true;
      image.onload = null;
      image.onerror = null;
    };
  }, [frame.dataUrl]);

  /* ───────── 测量上下文（与渲染使用同一字体，保证换行与绘制一致） ───────── */

  const measureCanvasText = useCallback((text: string, fontSizePx: number): number => {
    if (typeof document === "undefined") return estimateTextWidth(text, fontSizePx);
    if (measureCtxRef.current === undefined) {
      const canvas = document.createElement("canvas");
      canvas.width = 1;
      canvas.height = 1;
      measureCtxRef.current = canvas.getContext("2d");
    }
    const ctx = measureCtxRef.current;
    if (!ctx) return estimateTextWidth(text, fontSizePx);
    ctx.font = fontFor(fontSizePx);
    return ctx.measureText(text).width;
  }, []);

  /* ───────── 版式求解：OCR 行 → 段块 → 每块字号/换行/颜色 ───────── */

  useEffect(() => {
    const ocrLines: OcrLine[] = boxes
      .filter((box) => box && box.text && box.text.trim().length > 0 && box.bbox)
      .map((box) => ({
        text: box.text,
        confidence: box.confidence,
        box: box.bbox,
      }));
    const blocks = mergeLines(ocrLines);
    const image = imageRef.current;
    const sampler = createBackgroundSampler(image);
    const inputs: PlanBlockInput[] = [];
    const failed: RectPhys[] = [];
    const sampledBackgrounds: Record<string, SampledBackground> = {};

    for (const block of blocks) {
      const result = results[block.id];
      if (!result) continue; // 译文尚未到达（分词/分批）：先不覆盖该块
      if (result.failed) {
        failed.push(block.bbox);
        continue;
      }
      const translation = (result.translation ?? "").trim();
      if (!translation) continue;
      const sampled = sampler ? sampler(block.bbox, virtualRect) : null;
      if (sampled) sampledBackgrounds[block.id] = sampled;
      // 无真实帧（浏览器 mock / dataUrl 缺失）时按深色底处理，保证遮罩上的占位文字可读
      const luminance = sampled ? sampled.luminance : frame.dataUrl ? 0.9 : 0.08;
      inputs.push({
        block,
        translation,
        dark: luminance < 0.45,
        backgroundLuminance: luminance,
      });
    }

    setPlan(
      planCover(inputs, {
        dpr,
        bounds: virtualRect,
        measure: measureCanvasText,
      }),
    );
    setFailedRects(failed);
    setBackgrounds(sampledBackgrounds);
  }, [boxes, results, dpr, virtualRect, frame.dataUrl, imageReady, measureCanvasText]);

  /* ───────── Canvas 绘制 ───────── */

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const width = Math.max(1, Math.round(virtualRect.width));
    const height = Math.max(1, Math.round(virtualRect.height));
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, width, height);
    if (view === "original") return;

    const image = imageRef.current;

    // 失败块：保留原文，红框提示（研究 §5.4-3：含替换符/低置信度块回退原文）
    for (const rect of failedRects) {
      const box = toCanvasRect(rect, virtualRect);
      ctx.save();
      ctx.setLineDash([6, 4]);
      ctx.lineWidth = 2;
      ctx.strokeStyle = FAILED_COLOR;
      ctx.strokeRect(box.x + 1, box.y + 1, Math.max(0, box.width - 2), Math.max(0, box.height - 2));
      ctx.restore();
    }

    for (let index = 0; index < plan.items.length; index += 1) {
      const item = plan.items[index];
      const background = backgrounds[item.id];
      if (view === "bilingual") {
        const previous = index > 0 ? plan.items[index - 1] : null;
        const next = index + 1 < plan.items.length ? plan.items[index + 1] : null;
        const previousRect = previous ? toCanvasRect(previous.textRect, virtualRect) : null;
        const nextRect = next ? toCanvasRect(next.textRect, virtualRect) : null;
        drawBilingualItem(
          ctx,
          item,
          virtualRect,
          width,
          height,
          background,
          previousRect ? previousRect.y + previousRect.height : 0,
          nextRect ? nextRect.y : null,
        );
      } else {
        drawCoverItem(ctx, item, virtualRect, image, width, height, background);
      }
    }
  }, [plan, failedRects, backgrounds, view, virtualRect, imageReady]);

  const isTranslating = phase === "translating";
  const done = progress?.done ?? 0;
  const total = progress?.total ?? 0;
  const viewLabel = view === "cover" ? "覆盖" : view === "bilingual" ? "双语" : "原文";

  return (
    <div className="translation-overlay" aria-label="覆盖翻译层">
      <canvas
        ref={canvasRef}
        className="translation-canvas"
        role="img"
        aria-label={`覆盖翻译结果：${plan.total} 块，当前 ${viewLabel} 模式`}
        data-testid="translation-canvas"
      />

      {!frame.dataUrl ? (
        // 降级提示（研究 §8.2）：DRM/受保护窗口没有可模糊的冻结帧时，只能绘制译文，不能「擦掉原文」。
        <div className="translation-notice" role="note">
          未获取到冻结帧（骨架模式或受保护窗口），无法模糊原文；当前仅叠加译文。
        </div>
      ) : null}

      <div className="translation-toolbar" role="toolbar" aria-label="覆盖翻译工具条">
        <span className="translation-toolbar__status" role="status" aria-live="polite">
          {isTranslating ? (
            <>
              <Spinner size="sm" />
              <span>
                翻译中 {done}/{total || plan.total}
              </span>
            </>
          ) : (
            <span>
              已覆盖 {plan.total} 块{plan.bubbleCount > 0 ? ` · 气泡 ${plan.bubbleCount}` : ""}
              {engine ? ` · 引擎 ${engine}` : ""}
            </span>
          )}
        </span>

        <div className="translation-toolbar__views" role="group" aria-label="显示模式">
          <button
            type="button"
            className={`translation-toolbar__view${view === "original" ? " translation-toolbar__view--active" : ""}`}
            aria-pressed={view === "original"}
            title="只看原文（O）"
            onClick={() => onViewChange("original")}
          >
            原文
          </button>
          <button
            type="button"
            className={`translation-toolbar__view${view === "bilingual" ? " translation-toolbar__view--active" : ""}`}
            aria-pressed={view === "bilingual"}
            title="原文 + 译文对照（B）"
            onClick={() => onViewChange("bilingual")}
          >
            双语
          </button>
          <button
            type="button"
            className={`translation-toolbar__view${view === "cover" ? " translation-toolbar__view--active" : ""}`}
            aria-pressed={view === "cover"}
            title="模糊原文并原位显示译文（C）"
            onClick={() => onViewChange("cover")}
          >
            覆盖
          </button>
        </div>

        <span
          className="translation-toolbar__meta"
          title="覆盖翻译只作用于框选 AI 自己的冻结画面，不修改目标应用"
        >
          仅覆盖本窗口冻结画面
        </span>

        <div className="translation-toolbar__actions">
          <Button size="sm" variant="ghost" onClick={onRetranslate} disabled={isTranslating}>
            重译
          </Button>
          <Button size="sm" variant="ghost" onClick={onRecapture}>
            重新截屏
          </Button>
          <Button size="sm" variant="ghost" onClick={onClose}>
            关闭
          </Button>
        </div>
      </div>
    </div>
  );
}

/* ───────────────────────── 绘制细节 ───────────────────────── */

function drawCoverItem(
  ctx: CanvasRenderingContext2D,
  item: CoverPlanItem,
  virtualRect: RectPhys,
  image: HTMLImageElement | null,
  width: number,
  height: number,
  background: SampledBackground | undefined,
): void {
  const cover = toCanvasRect(item.coverRect, virtualRect);
  const text = toCanvasRect(item.textRect, virtualRect);

  ctx.save();
  roundRectPath(ctx, cover, item.bubble ? 6 : 2);
  ctx.clip();

  // ① 模糊原文区域（研究 §7.1：ctx.filter = blur(σ)）
  if (!item.bubble && image) {
    const pad = Math.ceil(item.blurSigmaPx * 3);
    const canvasX = Math.max(0, cover.x - pad);
    const canvasY = Math.max(0, cover.y - pad);
    const canvasW = Math.min(width, cover.x + cover.width + pad) - canvasX;
    const canvasH = Math.min(height, cover.y + cover.height + pad) - canvasY;
    // 帧可能并非 1:1 映射到虚拟桌面（例如截图被缩放），源坐标按自然尺寸换算
    const scaleX = image.naturalWidth / Math.max(1, width);
    const scaleY = image.naturalHeight / Math.max(1, height);
    if (canvasW > 0 && canvasH > 0) {
      ctx.filter = `blur(${Math.max(1.8, item.blurSigmaPx)}px)`;
      ctx.drawImage(
        image,
        canvasX * scaleX,
        canvasY * scaleY,
        canvasW * scaleX,
        canvasH * scaleY,
        canvasX,
        canvasY,
        canvasW,
        canvasH,
      );
      ctx.filter = "none";
    }
  }

  // ② 半透明遮罩：向**采样到的局部背景色**收敛（研究 §3.4「flatten α=0.5」），
  //    避免固定黑/白遮罩在深浅交界处产生「灰糊块」或压到相邻背景。
  //    气泡固定深底（tooltip 语义），不跟随采样背景。
  const flattenColor = item.bubble
    ? "rgba(12, 16, 22, 0.92)"
    : background
      ? `rgba(${background.r}, ${background.g}, ${background.b}, ${item.dark ? 0.62 : 0.58})`
      : item.dark
        ? "rgba(8, 10, 14, 0.62)"
        : "rgba(255, 255, 255, 0.58)";
  ctx.fillStyle = flattenColor;
  ctx.fillRect(cover.x, cover.y, cover.width, cover.height);

  // ③ 对比度不足时的衬底（WCAG <4.5:1）
  if (item.scrimAlpha > 0) {
    ctx.fillStyle = item.dark
      ? `rgba(0, 0, 0, ${item.scrimAlpha})`
      : `rgba(255, 255, 255, ${item.scrimAlpha})`;
    ctx.fillRect(cover.x, cover.y, cover.width, cover.height);
  }

  ctx.restore();
  drawItemText(ctx, item, text);
}

function drawBilingualItem(
  ctx: CanvasRenderingContext2D,
  item: CoverPlanItem,
  virtualRect: RectPhys,
  width: number,
  height: number,
  background: SampledBackground | undefined,
  prevBottom: number,
  nextTop: number | null,
): void {
  // 双语：原文保持清晰，译文优先画在块下方空隙里；下方不够时依次尝试上方、块内，
  // 避免译文浮层压住相邻原文行（研究 §7.2「下移 15% 或气泡」的保守实现）。
  const text = toCanvasRect(item.textRect, virtualRect);
  const pad = 4;
  const panelWidth = Math.max(text.width + pad * 2, 120);
  const panelHeight = item.lines.length * item.lineHeightPx + pad * 2;
  let x = text.x - pad;
  const belowY = text.y + text.height + 2;
  const aboveY = text.y - panelHeight - 2;
  let y = belowY;
  let panelAlpha = 1;
  if (nextTop !== null && belowY + panelHeight > nextTop - 2) {
    if (aboveY > prevBottom + 2 && aboveY >= 0) {
      y = aboveY;
    } else {
      // 上下都没有空隙：盖在本块原文上（0.8 透明度仍可辨认原文轮廓）
      y = text.y + Math.max(0, (text.height - panelHeight) / 2);
      panelAlpha = 0.8;
    }
  }
  if (y + panelHeight > height) y = Math.max(0, height - panelHeight);
  x = Math.min(Math.max(0, x), Math.max(0, width - panelWidth));
  const panel: CanvasRect = { x, y, width: panelWidth, height: panelHeight };

  ctx.save();
  roundRectPath(ctx, panel, 4);
  const baseAlpha = background ? 0.94 : item.dark ? 0.9 : 0.94;
  const alpha = baseAlpha * panelAlpha;
  ctx.fillStyle = background
    ? `rgba(${background.r}, ${background.g}, ${background.b}, ${alpha})`
    : item.dark
      ? `rgba(12, 16, 22, ${alpha})`
      : `rgba(250, 251, 253, ${alpha})`;
  ctx.fill();
  ctx.strokeStyle = item.dark ? "rgba(255,255,255,0.18)" : "rgba(0,0,0,0.12)";
  ctx.lineWidth = 1;
  ctx.stroke();
  ctx.restore();

  drawItemText(ctx, item, {
    x: panel.x + pad,
    y: panel.y + pad,
    width: panel.width - pad * 2,
    height: panel.height - pad * 2,
  });
}

function drawItemText(ctx: CanvasRenderingContext2D, item: CoverPlanItem, text: CanvasRect): void {
  if (item.lines.length === 0) return;
  ctx.save();
  ctx.font = fontFor(item.fontSizePx);
  ctx.fillStyle = item.color;
  ctx.textBaseline = "middle";
  ctx.textAlign = "left";
  ctx.shadowColor = item.dark ? "rgba(0, 0, 0, 0.45)" : "rgba(255, 255, 255, 0.35)";
  ctx.shadowBlur = 1.5;
  const totalHeight = item.lines.length * item.lineHeightPx;
  const startY = text.y + Math.max(0, (text.height - totalHeight) / 2);
  for (let index = 0; index < item.lines.length; index += 1) {
    ctx.fillText(item.lines[index], text.x, startY + item.lineHeightPx * (index + 0.5));
  }
  ctx.restore();
}
