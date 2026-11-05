/**
 * 覆盖翻译版式算法（纯函数 / 零依赖 / 全程物理像素）。
 *
 * 依据 `docs/research/覆盖翻译-证据报告.md` §6 的实测口径：
 * - 行 → 段块合并：垂直间距 ≤ 0.55×行高、左边缘差 ≤ 0.9×行高或横向重叠 ≥ 50%、
 *   行高比 0.7–1.45、单段 ≤ 6 行；100 张真实语料 2288 行 → 1552 块（−32.2%）。
 * - 字号搜索：源行高 × {1.00, 0.95, …, 0.60}（5% 步长），行高 1.20×字号；
 *   推荐配置（段块 + 允许吃已知空白）下字号中位 0.80、溢出 0%。
 * - 空白扩展：横向 ≤ min(右侧空白, 0.6×块宽, 320px)；纵向 ≤ min(上下空白, 1.8×块高)，
 *   留 3px 不压邻居。
 * - 溢出降级：不截断，改为小气泡（`bubble: true`）；< 12px 或极窄控件（宽 < 3×行高）直接气泡。
 * - 字色：背景亮度 < 0.45 → 浅字 #F5F7FA，否则深字 #11151A；对比度 < 4.5:1 加 0.35–0.45 衬底。
 * - 模糊：σ = clamp(0.38×源行高, ≥1.8)，覆盖矩形外扩 pad = max(2, 0.14×行高)，上边只留 2px。
 *
 * 本模块不访问 DOM（不依赖 Canvas `measureText`）：默认用字符类别宽度估算，
 * 渲染层可注入真实 `measure` 以获得与 Canvas 一致的结果。DPR 换算只在渲染层做一次。
 */

import type { RectPhys } from "../../shared/contract/types";

/* ───────────────────────────── 常量 ───────────────────────────── */

/** 字号搜索下限（源行高的倍数）。 */
export const FIT_MIN_RATIO = 0.6;
/** 字号搜索上限（源行高的倍数）。 */
export const FIT_MAX_RATIO = 1;
/** 字号搜索步长（5%）。 */
export const FIT_STEP = 0.05;
/** 译文行高 / 字号（英文 UI 正文常见 1.2）。 */
export const LINE_HEIGHT_RATIO = 1.2;
/** 物理像素字号硬下限（研究 §6.6：`<12px` 直接气泡）。 */
export const DEFAULT_MIN_FONT_PX = 12;
/** 可读性下限（CSS px）：物理下限 = 11 × dpr（研究 §6.6）。 */
export const MIN_READABLE_CSS_PX = 11;
/** 单段最大行数（研究 §6.2 行合并参数）。 */
export const MAX_BLOCK_LINES = 6;
/** 换行最多 3 行（研究 §6.5 `2 ≤ lines ≤ min(3, …)`）。 */
export const DEFAULT_MAX_LINES = 3;
/** 缺省 DPR（渲染层应传入 overlay 实际 dpr）。 */
export const DEFAULT_DPR = 1;
/** 覆盖矩形外扩 pad / 源行高。 */
export const DEFAULT_PAD_RATIO = 0.14;
/** 覆盖矩形外扩最小 pad（px）。 */
export const MIN_PAD_PX = 2;
/** 气泡最小/最大宽度（物理 px）。 */
export const BUBBLE_MIN_WIDTH = 200;
export const BUBBLE_MAX_WIDTH = 360;
/** 横向空白扩展上限：0.6×块宽、320px。 */
export const MAX_HORIZONTAL_EXPAND_RATIO = 0.6;
export const MAX_HORIZONTAL_EXPAND_PX = 320;
/** 纵向空白扩展上限：1.8×块高。 */
export const MAX_VERTICAL_EXPAND_RATIO = 1.8;
/**
 * 向上扩展的保守上限（0.6×块高）。
 * 研究 §6.5 的冲突回退里「先禁向上扩展」；默认限制上扩量可避免跨出卡片/段落背景，
 * 把模糊补丁画到上一块内容上（真实语料中上邻多为上一段正文）。
 */
export const MAX_UPWARD_EXPAND_RATIO = 0.6;
/** 与邻居保留的间隙（px）。 */
export const NEIGHBOR_GAP_PX = 3;
/** 浅底深字 / 深底浅字（研究 §3.4）。 */
export const COVER_TEXT_DARK = "#11151A";
export const COVER_TEXT_LIGHT = "#F5F7FA";
/** WCAG AA 正文对比度。 */
export const MIN_CONTRAST_RATIO = 4.5;

const EPS = 1e-6;

/* ───────────────────────────── 类型 ───────────────────────────── */

/** OCR 行（物理像素）。 */
export interface OcrLine {
  text: string;
  confidence: number;
  /** 文字框（物理像素，绝对坐标）。 */
  box: RectPhys;
}

/** 段块：OCR 行合并 + 版式求解的输入单位。 */
export interface CoverBlock {
  /** 稳定 id（`b0`、`b1`…，按 (y, x) 排序生成）。 */
  id: string;
  /** 段内各行拼接后的原文。 */
  text: string;
  /** 段内 OCR 行（保留原文与行框，供「切回原文」与无障碍）。 */
  lines: OcrLine[];
  /** 段块外接矩形（物理像素）。 */
  bbox: RectPhys;
  /** 源字号（≈段内行高中位数，物理 px）。 */
  fontSizePx: number;
  /** 译文行高（= 字号 × 1.2，物理 px）。 */
  lineHeightPx: number;
  /** 标题比：该块字号 / 全图行高中位数（正文 ≈1，标题 >1）。 */
  headingRatio: number;
}

/** 版式档位：原字号不换行 / 缩小字号单行 / 换行 / 降级。 */
export type FitTier = "same" | "shrink" | "wrap" | "fallback";

/** 绝对宽度测量函数：给定文本与字号（物理 px），返回宽度（物理 px）。 */
export type MeasureFn = (text: string, fontSizePx: number) => number;

export interface FitOptions {
  /** overlay 实际 dpr（用于 11 CSS px 可读性下限）。 */
  dpr?: number;
  /** 字号硬下限（物理 px），缺省 12；实际下限还会与 11×dpr 取最大。 */
  minPx?: number;
  /** 最多换行数，缺省 3。 */
  maxLines?: number;
  /** 可用区域（默认块自身 bbox；允许吃空白时传入扩展后的矩形）。 */
  availRect?: RectPhys;
  /** 注入测量函数（测试确定性）；缺省按字符类别估算。 */
  measure?: MeasureFn;
  /** 译文行高 / 字号，缺省 1.2。 */
  lineHeightRatio?: number;
}

/** 单块字号求解结果。 */
export interface FitResult {
  ok: boolean;
  fontSizePx: number;
  lineHeightPx: number;
  /** 已换行文本；失败时为整段原文（供气泡重排）。 */
  lines: string[];
  tier: FitTier;
  /** 实际字号 / 源字号（0.60–1.00）。 */
  fontRatio: number;
}

export interface WrapOptions {
  /** 最多行数；超出截断（不追加省略号，由渲染层决定）。 */
  maxLines?: number;
}

/** `planCover` 的单块输入。 */
export interface PlanBlockInput {
  block: CoverBlock;
  /** 译文（真实译文优先，不要用预估长度）。 */
  translation: string;
  /** 覆盖区域偏深色底（渲染层按模糊后中位亮度判定）。 */
  dark: boolean;
  /** 可选：精确背景相对亮度（0–1），优先于 `dark`。 */
  backgroundLuminance?: number;
}

export interface PlanOptions {
  /** overlay 实际 dpr。 */
  dpr?: number;
  /** 字号硬下限（物理 px），缺省 12。 */
  minPx?: number;
  /** 最多换行数，缺省 3。 */
  maxLines?: number;
  /** 是否允许吃已知空白（横向右侧 + 纵向上下），缺省 true。 */
  allowExpand?: boolean;
  /** 帧/虚拟桌面物理矩形：用于空白扩展与钳制。 */
  bounds?: RectPhys;
  /** 注入测量函数。 */
  measure?: MeasureFn;
  /** 译文行高 / 字号。 */
  lineHeightRatio?: number;
}

/** 单块覆盖版式。 */
export interface CoverPlanItem {
  id: string;
  source: string;
  translation: string;
  /** 已换行译文。 */
  lines: string[];
  fontSizePx: number;
  lineHeightPx: number;
  /** 实际字号 / 源字号。 */
  fontRatio: number;
  tier: FitTier;
  /** 是否降级为气泡（不覆盖原文，绘制在块旁）。 */
  bubble: boolean;
  /** 文本绘制区域（物理像素）。 */
  textRect: RectPhys;
  /** 模糊 + 遮罩 / 气泡背景区域（物理像素，含外扩 pad）。 */
  coverRect: RectPhys;
  /** 文字颜色（深浅自适应）。 */
  color: string;
  /** 衬底 alpha（0 = 不需要）。 */
  scrimAlpha: number;
  /** 文字与覆盖底色的对比度（≥4.5 为达标）。 */
  contrastRatio: number;
  /** 覆盖区域是否偏深色底。 */
  dark: boolean;
  /** 模糊半径 σ（物理 px）。 */
  blurSigmaPx: number;
}

/** 整屏覆盖版式计划。 */
export interface CoverPlan {
  /** 按 (y, x) 排序的逐块版式（渲染顺序即数组顺序）。 */
  items: CoverPlanItem[];
  total: number;
  sameCount: number;
  shrinkCount: number;
  wrapCount: number;
  bubbleCount: number;
  /** 非气泡块的 fontRatio 中位数（无则 0）。 */
  medianFontRatio: number;
  /** 最小实际字号（含气泡）。 */
  minFontPx: number;
}

/* ───────────────────────── 几何工具 ───────────────────────── */

/** 矩形右边界。 */
export function rectRight(rect: RectPhys): number {
  return rect.x + rect.width;
}

/** 矩形下边界。 */
export function rectBottom(rect: RectPhys): number {
  return rect.y + rect.height;
}

/** 是否为有限且非负尺寸的矩形。 */
export function isValidRect(rect: RectPhys | null | undefined): rect is RectPhys {
  if (!rect) return false;
  return (
    [rect.x, rect.y, rect.width, rect.height].every((value) => Number.isFinite(value)) &&
    rect.width >= 0 &&
    rect.height >= 0
  );
}

/** 两矩形是否相交（`margin` 为额外间隙）。 */
export function rectsOverlap(a: RectPhys, b: RectPhys, margin = 0): boolean {
  return (
    a.x < rectRight(b) + margin &&
    rectRight(a) > b.x - margin &&
    a.y < rectBottom(b) + margin &&
    rectBottom(a) > b.y - margin
  );
}

/** 两矩形外接矩形。 */
export function unionRect(a: RectPhys, b: RectPhys): RectPhys {
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  const right = Math.max(rectRight(a), rectRight(b));
  const bottom = Math.max(rectBottom(a), rectBottom(b));
  return { x, y, width: right - x, height: bottom - y };
}

/** 取交集（不重叠时返回零尺寸矩形）。 */
export function clampRect(inner: RectPhys, outer: RectPhys): RectPhys {
  const x = Math.max(inner.x, outer.x);
  const y = Math.max(inner.y, outer.y);
  const right = Math.min(rectRight(inner), rectRight(outer));
  const bottom = Math.min(rectBottom(inner), rectBottom(outer));
  return {
    x,
    y,
    width: Math.max(0, right - x),
    height: Math.max(0, bottom - y),
  };
}

/** 四边外扩矩形。 */
export function expandRect(
  rect: RectPhys,
  left: number,
  top: number,
  right: number,
  bottom: number,
): RectPhys {
  return {
    x: rect.x - left,
    y: rect.y - top,
    width: rect.width + left + right,
    height: rect.height + top + bottom,
  };
}

/** 数值钳制。 */
export function clampNumber(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function round(value: number, digits = 2): number {
  const factor = 10 ** digits;
  return Math.round(value * factor) / factor;
}

function median(values: number[]): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

/* ───────────────────────── 文本宽度估算 ───────────────────────── */

/** 是否 CJK 字符（中文 / 日文 / 韩文 / 全角标点）。 */
export function isCjkChar(char: string): boolean {
  const code = char.codePointAt(0) ?? 0;
  return (
    (code >= 0x2e80 && code <= 0x2fff) ||
    (code >= 0x3000 && code <= 0x303f) ||
    (code >= 0x3040 && code <= 0x30ff) ||
    (code >= 0x3400 && code <= 0x4dbf) ||
    (code >= 0x4e00 && code <= 0x9fff) ||
    (code >= 0xf900 && code <= 0xfaff) ||
    (code >= 0xff00 && code <= 0xffef)
  );
}

/** 单字符前进宽度（× 字号）。CJK 1.0em；拉丁按类别 0.52–0.62em。 */
export function charWidthFactor(char: string): number {
  if (/\s/.test(char)) return 0.28;
  if (isCjkChar(char)) return 1;
  if (/[A-Z]/.test(char)) return 0.62;
  if (/[0-9]/.test(char)) return 0.56;
  return 0.52;
}

/**
 * 估算文本宽度（物理 px）。
 * 系数来自研究 §6.1：英文每字符前进宽度 k P50 = 0.518×行高，中文 1.0×字号。
 */
export function estimateTextWidth(text: string, fontSizePx: number): number {
  let width = 0;
  for (const char of text) width += charWidthFactor(char) * fontSizePx;
  return width;
}

/* ───────────────────────── 行 → 段块合并 ───────────────────────── */

function normalizeLine(line: OcrLine): OcrLine | null {
  if (!line || !isValidRect(line.box)) return null;
  const text = typeof line.text === "string" ? line.text : "";
  if (text.trim().length === 0 || line.box.width <= 0 || line.box.height <= 0) return null;
  return {
    text,
    confidence: Number.isFinite(line.confidence) ? clampNumber(line.confidence, 0, 1) : 0,
    box: { ...line.box },
  };
}

function compareLines(a: OcrLine, b: OcrLine): number {
  const dy = a.box.y - b.box.y;
  if (Math.abs(dy) > 0.5) return dy;
  return a.box.x - b.box.x;
}

function horizontalOverlapRatio(a: RectPhys, b: RectPhys): number {
  const overlap = Math.min(rectRight(a), rectRight(b)) - Math.max(a.x, b.x);
  const base = Math.min(a.width, b.width);
  return base > EPS ? Math.max(0, overlap) / base : 0;
}

function shouldMerge(
  previous: OcrLine,
  next: OcrLine,
  lineCount: number,
  gapXRatio: number,
  gapYRatio: number,
): boolean {
  if (lineCount >= MAX_BLOCK_LINES) return false;
  const previousHeight = previous.box.height;
  const nextHeight = next.box.height;
  if (previousHeight <= EPS || nextHeight <= EPS) return false;
  // 同字号：0.7–1.45
  const heightRatio = nextHeight / previousHeight;
  if (heightRatio < 0.7 || heightRatio > 1.45) return false;
  // 同段行距：最多上压 0.3×行高，向下 ≤ 0.55×行高
  const gap = next.box.y - rectBottom(previous.box);
  if (gap < -0.3 * previousHeight || gap > gapYRatio * previousHeight) return false;
  // 同列 / 同缩进
  const leftDelta = Math.abs(next.box.x - previous.box.x);
  const aligned = leftDelta <= gapXRatio * previousHeight;
  const overlapping = horizontalOverlapRatio(previous.box, next.box) >= 0.5;
  return aligned || overlapping;
}

function joinLineTexts(texts: string[]): string {
  let joined = "";
  for (const raw of texts) {
    const text = raw.trim();
    if (!text) continue;
    if (!joined) {
      joined = text;
      continue;
    }
    const previousChar = joined[joined.length - 1] ?? "";
    const nextChar = text[0] ?? "";
    if (isCjkChar(previousChar) && isCjkChar(nextChar)) {
      // 中文换行处直接拼接（原文本身无空格）
      joined += text;
    } else if (previousChar === "-") {
      // 连字符断行不额外补空格
      joined += text;
    } else {
      joined += ` ${text}`;
    }
  }
  return joined;
}

/**
 * OCR 行 → 段块（研究 §6.2 参数）。
 *
 * @param lines OCR 行（物理像素，已按选区内过滤）
 * @param gapX 左边缘对齐容差（× 行高），缺省 0.9
 * @param gapY 同段最大行间距（× 行高），缺省 0.55
 */
export function mergeLines(lines: OcrLine[], gapX = 0.9, gapY = 0.55): CoverBlock[] {
  const valid = (Array.isArray(lines) ? lines : [])
    .map(normalizeLine)
    .filter((line): line is OcrLine => line !== null)
    .sort(compareLines);
  if (valid.length === 0) return [];

  const globalMedianHeight = median(valid.map((line) => line.box.height));
  const groups: OcrLine[][] = [];
  let current: OcrLine[] = [];
  for (const line of valid) {
    if (current.length === 0) {
      current.push(line);
      continue;
    }
    if (shouldMerge(current[current.length - 1], line, current.length, gapX, gapY)) {
      current.push(line);
    } else {
      groups.push(current);
      current = [line];
    }
  }
  if (current.length > 0) groups.push(current);

  return groups.map((group, index) => buildBlock(group, index, globalMedianHeight));
}

function buildBlock(lines: OcrLine[], index: number, globalMedianHeight: number): CoverBlock {
  const bbox = lines.reduce<RectPhys>((acc, line) => unionRect(acc, line.box), { ...lines[0].box });
  const fontSizePx = Math.max(1, median(lines.map((line) => line.box.height)));
  return {
    id: `b${index}`,
    text: joinLineTexts(lines.map((line) => line.text)),
    lines: lines.map((line) => ({ ...line, box: { ...line.box } })),
    bbox,
    fontSizePx: round(fontSizePx, 2),
    lineHeightPx: round(fontSizePx * LINE_HEIGHT_RATIO, 2),
    headingRatio: globalMedianHeight > EPS ? round(fontSizePx / globalMedianHeight, 3) : 1,
  };
}

/* ───────────────────────── 自动换行 ───────────────────────── */

function tokenize(text: string): string[] {
  const tokens: string[] = [];
  let buffer = "";
  const flush = (): void => {
    if (buffer) {
      tokens.push(buffer);
      buffer = "";
    }
  };
  for (const char of text) {
    if (/\s/.test(char)) {
      flush();
      tokens.push(" ");
    } else if (isCjkChar(char)) {
      flush();
      tokens.push(char);
    } else {
      buffer += char;
    }
  }
  flush();
  return tokens;
}

function hardSplit(token: string, maxWidth: number, widthOf: (value: string) => number): string[] {
  const chunks: string[] = [];
  let current = "";
  for (const char of token) {
    const candidate = current + char;
    if (current && widthOf(candidate) > maxWidth + EPS) {
      chunks.push(current);
      current = char;
    } else {
      current = candidate;
    }
  }
  if (current) chunks.push(current);
  return chunks.length > 0 ? chunks : [token];
}

/**
 * 自动换行（CJK / 拉丁混排）：贪心按词，CJK 逐字可断，超长单词按字符硬切。
 *
 * @param text 译文
 * @param maxWidth 单行最大宽度（物理 px）
 * @param fontSizePx 字号（物理 px）
 * @param measure 指定字号下的宽度测量；缺省按字符类别估算
 * @param opts `maxLines` 截断行数
 */
export function wrapText(
  text: string,
  maxWidth: number,
  fontSizePx: number,
  measure?: (value: string) => number,
  opts: WrapOptions = {},
): string[] {
  const clean = typeof text === "string" ? text.trim() : "";
  if (!clean || maxWidth <= EPS || fontSizePx <= 0) return [];
  const widthOf = measure ?? ((value: string) => estimateTextWidth(value, fontSizePx));
  const maxLines = opts.maxLines && opts.maxLines > 0 ? Math.floor(opts.maxLines) : undefined;

  const lines: string[] = [];
  let line = "";
  const pushLine = (): void => {
    const trimmed = line.trim();
    if (trimmed) lines.push(trimmed);
    line = "";
  };

  for (const token of tokenize(clean)) {
    if (maxLines !== undefined && lines.length >= maxLines) break;
    if (token === " ") {
      if (line) line += " ";
      continue;
    }
    if (line && widthOf(line + token) > maxWidth + EPS) {
      pushLine();
      if (widthOf(token) > maxWidth + EPS) {
        const chunks = hardSplit(token, maxWidth, widthOf);
        for (let index = 0; index < chunks.length - 1; index += 1) {
          lines.push(chunks[index]);
        }
        line = chunks[chunks.length - 1] ?? "";
      } else {
        line = token;
      }
    } else if (!line && widthOf(token) > maxWidth + EPS) {
      const chunks = hardSplit(token, maxWidth, widthOf);
      for (let index = 0; index < chunks.length - 1; index += 1) {
        lines.push(chunks[index]);
      }
      line = chunks[chunks.length - 1] ?? "";
    } else {
      line = line + token;
    }
  }
  if (line.trim()) pushLine();
  const result = maxLines !== undefined ? lines.slice(0, maxLines) : lines;
  return result.map((entry) => entry.trim()).filter((entry) => entry.length > 0);
}

/* ───────────────────────── 字号自适应 ───────────────────────── */

/**
 * 字号自适应：在 [0.60, 1.00]×源字号内按 5% 步长从大到小搜索，
 * 优先「原字号单行」→「原字号换行」→「缩小字号单行」…，全部失败返回 `tier: "fallback"`。
 */
export function fitFont(block: CoverBlock, text: string, opts: FitOptions = {}): FitResult {
  const base = block.fontSizePx > EPS ? block.fontSizePx : Math.max(1, block.bbox.height);
  const avail = opts.availRect && isValidRect(opts.availRect) ? opts.availRect : block.bbox;
  const dpr = opts.dpr && opts.dpr > 0 ? opts.dpr : DEFAULT_DPR;
  const minPx = Math.max(opts.minPx ?? DEFAULT_MIN_FONT_PX, MIN_READABLE_CSS_PX * dpr);
  const maxLines = Math.max(1, Math.floor(opts.maxLines ?? DEFAULT_MAX_LINES));
  const lineHeightRatio = opts.lineHeightRatio ?? LINE_HEIGHT_RATIO;
  const measureAt: MeasureFn = opts.measure ?? estimateTextWidth;
  const clean = typeof text === "string" ? text.trim() : "";

  const fallback: FitResult = {
    ok: false,
    fontSizePx: round(base, 2),
    lineHeightPx: round(base * lineHeightRatio, 2),
    lines: clean ? [clean] : [],
    tier: "fallback",
    fontRatio: 1,
  };
  if (!clean || !isValidRect(avail) || avail.width <= EPS || avail.height <= EPS) return fallback;

  const steps = Math.round((FIT_MAX_RATIO - FIT_MIN_RATIO) / FIT_STEP);
  for (let step = 0; step <= steps; step += 1) {
    const ratio = round(FIT_MAX_RATIO - step * FIT_STEP, 2);
    const fontSizePx = round(base * ratio, 2);
    // 字号只会越来越小：一旦低于可读下限就可以停止
    if (fontSizePx < minPx - EPS) break;

    const widthAt = (value: string): number => measureAt(value, fontSizePx);
    const wrapped = wrapText(clean, avail.width, fontSizePx, widthAt, { maxLines });
    const measuredWidth = widthAt(clean);

    // 档位 1/3：单行放下（原字号 = same；缩小 = shrink）
    if (wrapped.length <= 1 && measuredWidth <= avail.width + EPS) {
      return {
        ok: true,
        fontSizePx,
        lineHeightPx: round(fontSizePx * lineHeightRatio, 2),
        lines: [clean],
        tier: ratio >= 1 - EPS ? "same" : "shrink",
        fontRatio: ratio,
      };
    }

    // 档位 2：换行 2–maxLines 行（同字号优先）
    if (wrapped.length >= 2 && wrapped.length <= maxLines) {
      const totalHeight = wrapped.length * fontSizePx * lineHeightRatio;
      const widest = Math.max(...wrapped.map((entry) => widthAt(entry)));
      if (totalHeight <= avail.height + EPS && widest <= avail.width + EPS) {
        return {
          ok: true,
          fontSizePx,
          lineHeightPx: round(fontSizePx * lineHeightRatio, 2),
          lines: wrapped,
          tier: "wrap",
          fontRatio: ratio,
        };
      }
    }
  }
  return fallback;
}

/* ───────────────────────── 颜色 / 对比度 ───────────────────────── */

function parseHexColor(color: string): { r: number; g: number; b: number } | null {
  const match = /^#?([0-9a-f]{6})$/i.exec(color.trim());
  if (!match) return null;
  const value = Number.parseInt(match[1], 16);
  return {
    r: (value >> 16) & 0xff,
    g: (value >> 8) & 0xff,
    b: value & 0xff,
  };
}

function srgbToLinear(channel: number): number {
  const normalized = channel / 255;
  return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
}

/** WCAG 相对亮度（0–1）。 */
export function relativeLuminance(color: string): number {
  const rgb = parseHexColor(color);
  if (!rgb) return 0;
  return 0.2126 * srgbToLinear(rgb.r) + 0.7152 * srgbToLinear(rgb.g) + 0.0722 * srgbToLinear(rgb.b);
}

/** 按亮度计算对比度（WCAG 公式）。 */
export function contrastRatioByLuminance(foreground: number, background: number): number {
  const lighter = Math.max(foreground, background);
  const darker = Math.min(foreground, background);
  return (lighter + 0.05) / (darker + 0.05);
}

/**
 * 对比度：`background` 可传十六进制颜色或相对亮度（0–1）。
 * 覆盖层只有模糊后的中位亮度，因此渲染层通常直接传亮度。
 */
export function contrastRatio(foreground: string, background: string | number): number {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance =
    typeof background === "number" ? background : relativeLuminance(background);
  return contrastRatioByLuminance(foregroundLuminance, backgroundLuminance);
}

/** 覆盖文字样式（颜色 + 衬底 + 最终对比度）。 */
export interface TextStyle {
  color: string;
  /** 是否使用浅色字。 */
  light: boolean;
  /** 衬底 alpha（0 = 不需要）。 */
  scrimAlpha: number;
  /** 加衬底后的对比度（≥4.5 为达标）。 */
  contrastRatio: number;
}

/**
 * 深/浅底字色选择（研究 §7.6）：
 * 背景亮度 < 0.45 → 浅字 #F5F7FA，否则深字 #11151A；对比度 < 4.5:1 时加 0.35–0.45 衬底。
 */
export function pickTextStyle(backgroundLuminance: number): TextStyle {
  const background = clampNumber(
    Number.isFinite(backgroundLuminance) ? backgroundLuminance : 0.9,
    0,
    1,
  );
  const lightLuminance = relativeLuminance(COVER_TEXT_LIGHT);
  const darkLuminance = relativeLuminance(COVER_TEXT_DARK);
  const lightContrast = contrastRatioByLuminance(lightLuminance, background);
  const darkContrast = contrastRatioByLuminance(darkLuminance, background);
  const light = lightContrast >= darkContrast;
  const color = light ? COVER_TEXT_LIGHT : COVER_TEXT_DARK;
  let ratio = light ? lightContrast : darkContrast;
  let scrimAlpha = 0;

  if (ratio < MIN_CONTRAST_RATIO) {
    // 浅字需要压暗背景；深字需要提亮背景
    const target = light
      ? (lightLuminance + 0.05) / MIN_CONTRAST_RATIO - 0.05
      : (darkLuminance + 0.05) * MIN_CONTRAST_RATIO - 0.05;
    const rawAlpha = light
      ? background > EPS
        ? 1 - target / background
        : 0
      : 1 - background > EPS
        ? (target - background) / (1 - background)
        : 0;
    scrimAlpha = round(clampNumber(rawAlpha, 0.35, 0.45), 3);
    const scrimmedBackground = light
      ? background * (1 - scrimAlpha)
      : background * (1 - scrimAlpha) + scrimAlpha;
    ratio = contrastRatioByLuminance(light ? lightLuminance : darkLuminance, scrimmedBackground);
  }

  return {
    color,
    light,
    scrimAlpha,
    contrastRatio: round(ratio, 2),
  };
}

/* ───────────────────────── 覆盖区域 / 空白扩展 ───────────────────────── */

function coverRectFor(rect: RectPhys, pad: number, bounds?: RectPhys): RectPhys {
  // 研究 §3.4：上边只留 2px，避免擦到上一行
  const topPad = Math.min(MIN_PAD_PX, pad);
  const expanded = expandRect(rect, pad, topPad, pad, pad);
  if (bounds && isValidRect(bounds)) {
    const clamped = clampRect(expanded, bounds);
    return clamped.width > 0 && clamped.height > 0 ? clamped : expanded;
  }
  return expanded;
}

function verticalOverlapRatio(a: RectPhys, b: RectPhys): number {
  const overlap = Math.min(rectBottom(a), rectBottom(b)) - Math.max(a.y, b.y);
  const base = Math.min(a.height, b.height);
  return base > EPS ? Math.max(0, overlap) / base : 0;
}

/** `expandForBlock` 选项。 */
export interface ExpandOptions {
  /** 是否允许纵向扩展（冲突回退时只吃横向空白），缺省 true。 */
  vertical?: boolean;
}

/**
 * 已知空白扩展（研究 §6.5）：横向 ≤ min(右侧空白, 0.6×块宽, 320px)，
 * 纵向 ≤ min(上下空白, 1.8×块高)。只向右 / 上下扩展，保持左对齐。
 *
 * 预留 `pad + 3px`：覆盖矩形还要向外扩 `pad`（0.14×行高）画模糊补丁，
 * 因此可用区域与邻居之间必须同时留出 pad 与 3px 间隙，否则相邻覆盖补丁会互相擦边。
 */
export function expandForBlock(
  block: CoverBlock,
  peers: CoverBlock[],
  bounds?: RectPhys,
  opts: ExpandOptions = {},
): RectPhys {
  const box = block.bbox;
  const verticalAllowed = opts.vertical ?? true;
  if (!isValidRect(box) || box.height <= EPS) return { ...box };
  const pad = Math.max(MIN_PAD_PX, DEFAULT_PAD_RATIO * block.fontSizePx);
  // 横向：覆盖矩形右侧还要 pad，故留 3px 邻居间隙 + pad
  const horizontalReserve = NEIGHBOR_GAP_PX + pad;
  // 纵向：本块向下 pad + 下一块向上 top-pad(2px) + 2px 间隙
  const verticalReserve = pad + MIN_PAD_PX + 2;

  let rightGap = MAX_HORIZONTAL_EXPAND_PX;
  let downGap = MAX_VERTICAL_EXPAND_RATIO * box.height;
  let upGap = MAX_VERTICAL_EXPAND_RATIO * box.height;

  for (const peer of peers) {
    if (!peer || peer.id === block.id || !isValidRect(peer.bbox)) continue;
    const other = peer.bbox;
    if (other.x >= rectRight(box) && verticalOverlapRatio(box, other) > 0.2) {
      rightGap = Math.min(rightGap, other.x - rectRight(box));
    }
    if (other.y >= rectBottom(box) && horizontalOverlapRatio(box, other) > 0.2) {
      downGap = Math.min(downGap, other.y - rectBottom(box));
    }
    if (rectBottom(other) <= box.y && horizontalOverlapRatio(box, other) > 0.2) {
      upGap = Math.min(upGap, box.y - rectBottom(other));
    }
  }

  const right = Math.max(
    0,
    Math.min(rightGap, MAX_HORIZONTAL_EXPAND_RATIO * box.width, MAX_HORIZONTAL_EXPAND_PX) -
      horizontalReserve,
  );
  let up = 0;
  let down = 0;
  if (verticalAllowed) {
    const verticalBudget = Math.min(downGap + upGap, MAX_VERTICAL_EXPAND_RATIO * box.height);
    down = Math.min(verticalBudget, Math.max(0, downGap - verticalReserve));
    up = Math.min(
      Math.max(0, verticalBudget - down),
      Math.max(0, upGap - verticalReserve),
      MAX_UPWARD_EXPAND_RATIO * box.height,
    );
  }

  let expanded: RectPhys = {
    x: box.x,
    y: box.y - up,
    width: box.width + right,
    height: box.height + up + down,
  };
  if (bounds && isValidRect(bounds)) expanded = clampRect(expanded, bounds);
  if (expanded.width <= EPS || expanded.height <= EPS) return { ...box };
  return expanded;
}

/* ───────────────────────── 整屏版式计划 ───────────────────────── */

function makeCoverItem(
  input: PlanBlockInput,
  style: TextStyle,
  fit: FitResult,
  textRect: RectPhys,
  bounds?: RectPhys,
): CoverPlanItem {
  const base = Math.max(1, input.block.fontSizePx);
  const pad = Math.max(MIN_PAD_PX, DEFAULT_PAD_RATIO * base);
  return {
    id: input.block.id,
    source: input.block.text,
    translation: input.translation,
    lines: fit.lines,
    fontSizePx: fit.fontSizePx,
    lineHeightPx: fit.lineHeightPx,
    fontRatio: fit.fontRatio,
    tier: fit.tier,
    bubble: false,
    textRect: { ...textRect },
    coverRect: coverRectFor(textRect, pad, bounds),
    color: style.color,
    scrimAlpha: style.scrimAlpha,
    contrastRatio: style.contrastRatio,
    dark: input.dark,
    blurSigmaPx: round(clampNumber(0.38 * base, 1.8, 48), 2),
  };
}

interface BubbleOptions {
  minPx: number;
  maxLines: number;
  lineHeightRatio: number;
  measure: MeasureFn;
}

function makeBubbleItem(
  input: PlanBlockInput,
  threshold: BubbleOptions,
  bounds?: RectPhys,
): CoverPlanItem {
  const base = Math.max(1, input.block.fontSizePx);
  const translation = input.translation.trim() || input.block.text;
  const bubbleFont = Math.max(threshold.minPx, Math.min(base, 16));
  const padding = Math.max(MIN_PAD_PX + 1, 0.35 * bubbleFont);
  const width = clampNumber(
    Math.max(input.block.bbox.width, BUBBLE_MIN_WIDTH),
    BUBBLE_MIN_WIDTH,
    BUBBLE_MAX_WIDTH,
  );
  const textWidth = Math.max(bubbleFont, width - padding * 2);
  const lines = wrapText(
    translation,
    textWidth,
    bubbleFont,
    (value) => threshold.measure(value, bubbleFont),
    { maxLines: Math.max(threshold.maxLines, 6) },
  );
  const lineHeightPx = round(bubbleFont * threshold.lineHeightRatio, 2);
  const height = Math.max(lineHeightPx, lines.length * lineHeightPx) + padding * 2;

  let x = input.block.bbox.x;
  let y = rectBottom(input.block.bbox) + 4;
  if (bounds && isValidRect(bounds)) {
    x = clampNumber(x, bounds.x + 2, Math.max(bounds.x + 2, rectRight(bounds) - width - 2));
    if (y + height > rectBottom(bounds) - 2) y = input.block.bbox.y - height - 4;
    if (y < bounds.y + 2) {
      y = clampNumber(
        input.block.bbox.y,
        bounds.y + 2,
        Math.max(bounds.y + 2, rectBottom(bounds) - height - 2),
      );
    }
  }

  // 气泡固定深底浅字（tooltip 语义），保证任何背景下可读
  const bubbleStyle = pickTextStyle(0.08);
  return {
    id: input.block.id,
    source: input.block.text,
    translation,
    lines: lines.length > 0 ? lines : [translation],
    fontSizePx: round(bubbleFont, 2),
    lineHeightPx,
    fontRatio: round(bubbleFont / base, 3),
    tier: "fallback",
    bubble: true,
    textRect: {
      x: x + padding,
      y: y + padding,
      width: textWidth,
      height: Math.max(lineHeightPx, lines.length * lineHeightPx),
    },
    coverRect: { x, y, width, height },
    color: bubbleStyle.color,
    scrimAlpha: 0.9,
    contrastRatio: bubbleStyle.contrastRatio,
    dark: true,
    blurSigmaPx: 0,
  };
}

/**
 * 整屏覆盖版式：逐块求解字号 / 换行 / 吃空白 / 溢出降级气泡 / 字色对比度，
 * 并按 y 顺序消解覆盖矩形冲突（先禁向上扩展、再不扩展，最后降级气泡）。
 */
export function planCover(inputs: PlanBlockInput[], opts: PlanOptions = {}): CoverPlan {
  const dpr = opts.dpr && opts.dpr > 0 ? opts.dpr : DEFAULT_DPR;
  const minPx = Math.max(opts.minPx ?? DEFAULT_MIN_FONT_PX, MIN_READABLE_CSS_PX * dpr);
  const maxLines = Math.max(1, Math.floor(opts.maxLines ?? DEFAULT_MAX_LINES));
  const lineHeightRatio = opts.lineHeightRatio ?? LINE_HEIGHT_RATIO;
  const allowExpand = opts.allowExpand ?? true;
  const measure: MeasureFn = opts.measure ?? estimateTextWidth;
  const bounds = opts.bounds && isValidRect(opts.bounds) ? opts.bounds : undefined;
  const bubbleOptions: BubbleOptions = { minPx, maxLines, lineHeightRatio, measure };

  const list = (Array.isArray(inputs) ? inputs : []).filter(
    (input) => input && input.block && isValidRect(input.block.bbox),
  );
  const blocks = list.map((input) => input.block);
  const sorted = [...list].sort((a, b) => {
    const dy = a.block.bbox.y - b.block.bbox.y;
    return Math.abs(dy) > 0.5 ? dy : a.block.bbox.x - b.block.bbox.x;
  });

  const placed: CoverPlanItem[] = [];
  const items: CoverPlanItem[] = [];

  for (const input of sorted) {
    const block = input.block;
    const translation = (input.translation ?? "").trim() || block.text;
    const backgroundLuminance = Number.isFinite(input.backgroundLuminance)
      ? (input.backgroundLuminance as number)
      : input.dark
        ? 0.08
        : 0.92;
    const style = pickTextStyle(backgroundLuminance);
    const base = Math.max(1, block.fontSizePx);
    const narrow = block.bbox.width < 3 * base && block.bbox.width < BUBBLE_MIN_WIDTH;
    const tiny = base < minPx;

    let item: CoverPlanItem | null = null;
    if (!narrow && !tiny) {
      // 依次尝试：原框 → 全向吃空白 → 只吃横向空白；任一版式与已放置覆盖矩形冲突则换下一档
      const attempts: RectPhys[] = [{ ...block.bbox }];
      if (allowExpand) {
        const full = expandForBlock(block, blocks, bounds);
        if (full.width > block.bbox.width + EPS || full.height > block.bbox.height + EPS) {
          attempts.push(full);
        }
        const horizontal = expandForBlock(block, blocks, bounds, { vertical: false });
        if (horizontal.width > block.bbox.width + EPS) {
          attempts.push(horizontal);
        }
      }
      for (const availRect of attempts) {
        const fit = fitFont(block, translation, {
          dpr,
          minPx,
          maxLines,
          lineHeightRatio,
          measure,
          availRect,
        });
        if (!fit.ok) continue;
        const candidate = makeCoverItem(input, style, fit, availRect, bounds);
        const conflict = placed.some(
          (other) => !other.bubble && rectsOverlap(candidate.coverRect, other.coverRect, 1),
        );
        if (conflict) continue;
        item = candidate;
        break;
      }
    }
    // 第三轮：仍放不下 → 降级小气泡（研究 §6.5：不截断、不硬压邻居）
    if (!item) item = makeBubbleItem(input, bubbleOptions, bounds);

    placed.push(item);
    items.push(item);
  }

  const nonBubble = items.filter((item) => !item.bubble);
  return {
    items,
    total: items.length,
    sameCount: nonBubble.filter((item) => item.tier === "same").length,
    shrinkCount: nonBubble.filter((item) => item.tier === "shrink").length,
    wrapCount: nonBubble.filter((item) => item.tier === "wrap").length,
    bubbleCount: items.filter((item) => item.bubble).length,
    medianFontRatio: round(median(nonBubble.map((item) => item.fontRatio)), 3),
    minFontPx: items.length > 0 ? Math.min(...items.map((item) => item.fontSizePx)) : 0,
  };
}
