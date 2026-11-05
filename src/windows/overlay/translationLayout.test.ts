// @ts-nocheck
/**
 * `translationLayout.ts` 纯逻辑测试（Node 24 原生 TypeScript type stripping，无需构建/框架）。
 *
 * 运行：`node --test src/windows/overlay/translationLayout.test.ts`
 *
 * 断言口径来自 `docs/research/覆盖翻译-证据报告.md` §6 的实测数字：
 * - 行合并：2288 行 → 1552 块（−32.2%），平均 1.47 行/块，多行段 9.3%；
 * - 中→英：字符扩张 P50 2.46× / P95 3.59×，同字号行宽扩张 P50 1.82× / P95 2.62×；
 * - 推荐配置（段块 + 允许吃空白）：字号中位 0.80、溢出 0%，<12px 与极窄控件降级气泡。
 *
 * 说明：本仓库未安装 `@types/node`，因此文件级 `@ts-nocheck`（运行时由 Node 原生类型剥离，
 * 与 `pnpm typecheck`（tsc --noEmit）互不干扰）。
 */

import test from "node:test";
import assert from "node:assert/strict";

import {
  COVER_TEXT_DARK,
  COVER_TEXT_LIGHT,
  DEFAULT_MIN_FONT_PX,
  FIT_MAX_RATIO,
  FIT_MIN_RATIO,
  FIT_STEP,
  LINE_HEIGHT_RATIO,
  clampNumber,
  clampRect,
  contrastRatio,
  estimateTextWidth,
  expandForBlock,
  expandRect,
  fitFont,
  isCjkChar,
  isValidRect,
  mergeLines,
  pickTextStyle,
  planCover,
  rectBottom,
  rectRight,
  rectsOverlap,
  relativeLuminance,
  unionRect,
  wrapText,
} from "./translationLayout.ts";

/* ───────────────────────────── 工具 ───────────────────────────── */

const BASE = 20;

function ocrLine(text, x, y, width, height = BASE, confidence = 0.95) {
  return { text, confidence, box: { x, y, width, height } };
}

/** 直接构造一个单行段块（绕过 mergeLines，便于单测 fitFont/planCover）。 */
function makeBlock({
  id = "b0",
  text = "中文段落",
  x = 100,
  y = 100,
  width = 200,
  height = BASE,
} = {}) {
  return {
    id,
    text,
    lines: [{ text, confidence: 0.95, box: { x, y, width, height } }],
    bbox: { x, y, width, height },
    fontSizePx: height,
    lineHeightPx: height * LINE_HEIGHT_RATIO,
    headingRatio: 1,
  };
}

/** 生成「同字号下宽度压力 = pressure×块宽」的英文译文（英文每字符 0.52×字号）。 */
function englishForWidthPressure(cjkChars, pressure) {
  const letters = Math.max(1, Math.round((cjkChars * pressure) / 0.52));
  return "w".repeat(letters);
}

function assertNoOverflow(item, measure = estimateTextWidth) {
  assert.equal(item.bubble, false, `块 ${item.id} 不应降级为气泡`);
  assert.ok(item.lines.length > 0, `块 ${item.id} 应有译文行`);
  assert.ok(item.lines.length <= 3, `块 ${item.id} 行数超上限：${item.lines.length}`);
  const totalHeight = item.lines.length * item.lineHeightPx;
  assert.ok(
    totalHeight <= item.textRect.height + 0.5,
    `块 ${item.id} 纵向溢出：${totalHeight} > ${item.textRect.height}`,
  );
  for (const line of item.lines) {
    const width = measure(line, item.fontSizePx);
    assert.ok(
      width <= item.textRect.width + 0.5,
      `块 ${item.id} 横向溢出：「${line}」${width} > ${item.textRect.width}`,
    );
  }
}

/* ───────────────────────── 几何 / 宽度工具 ───────────────────────── */

test("rect helpers: union / clamp / overlap", () => {
  const a = { x: 10, y: 10, width: 100, height: 20 };
  const b = { x: 60, y: 40, width: 100, height: 20 };
  assert.deepEqual(unionRect(a, b), { x: 10, y: 10, width: 150, height: 50 });
  assert.deepEqual(clampRect(a, { x: 0, y: 0, width: 60, height: 40 }), {
    x: 10,
    y: 10,
    width: 50,
    height: 20,
  });
  // 不相交 → 零尺寸
  assert.deepEqual(clampRect(a, { x: 500, y: 500, width: 10, height: 10 }), {
    x: 500,
    y: 500,
    width: 0,
    height: 0,
  });
  assert.equal(rectsOverlap(a, { x: 100, y: 10, width: 10, height: 20 }), true);
  assert.equal(rectsOverlap(a, { x: 111, y: 10, width: 10, height: 20 }), false);
  assert.equal(rectsOverlap(a, { x: 112, y: 10, width: 10, height: 20 }, 3), true);
  assert.deepEqual(expandRect({ x: 10, y: 10, width: 20, height: 10 }, 2, 2, 2, 2), {
    x: 8,
    y: 8,
    width: 24,
    height: 14,
  });
  assert.equal(rectRight(a), 110);
  assert.equal(rectBottom(a), 30);
  // 非法矩形 / 零面积
  assert.equal(isValidRect(a), true);
  assert.equal(isValidRect({ x: 0, y: 0, width: -1, height: 2 }), false);
  assert.equal(isValidRect({ x: Number.NaN, y: 0, width: 1, height: 2 }), false);
  assert.equal(isValidRect(null), false);
  assert.equal(clampNumber(5, 0, 3), 3);
});

test("text width estimation: CJK ~1em, latin ~0.52em, whitespace narrower", () => {
  assert.equal(isCjkChar("中"), true);
  assert.equal(isCjkChar("A"), false);
  const width = estimateTextWidth("中文abc", 20);
  // 2 × 20 + 3 × 0.52 × 20 = 40 + 31.2
  assert.ok(Math.abs(width - 71.2) < 0.001);
  assert.ok(estimateTextWidth("  ", 20) < estimateTextWidth("中中", 20));
  // 常数口径（研究 §6.1）：英文 0.518×行高
  assert.ok(Math.abs(estimateTextWidth("w", 20) - 0.52 * 20) < 0.001);
});

/* ───────────────────────── 行 → 段块合并 ───────────────────────── */

/** 按研究口径构造 1552 块 / 2288 行（144 个 6 行段 + 1408 个单行段）。 */
function buildResearchCorpus() {
  const lines = [];
  const sixLineBlocks = 144;
  const singleBlocks = 1408;
  let y = -240; // 负坐标：虚拟桌面原点可为负
  for (let index = 0; index < sixLineBlocks; index += 1) {
    for (let lineIndex = 0; lineIndex < 6; lineIndex += 1) {
      lines.push(ocrLine(`第${index}段第${lineIndex}行中文`, 100, y, 240));
      y += BASE + 4; // 行距 4px ≤ 0.55×行高（11px）→ 应合并
    }
    // 段间距 26px（远大于 0.55×行高）→ 应断开
    y += 26 - 4;
  }
  for (let index = 0; index < singleBlocks; index += 1) {
    lines.push(ocrLine(`单行中文第${index}条`, 100, y, 180));
    y += BASE + 26;
  }
  return lines;
}

test("mergeLines reproduces research corpus reduction (2288→1552, −32.2%)", () => {
  const lines = buildResearchCorpus();
  assert.equal(lines.length, 144 * 6 + 1408); // 2272
  const blocks = mergeLines(lines);
  assert.equal(blocks.length, 1552);
  const reduction = 1 - blocks.length / lines.length;
  // 研究实测 −32.2%（本语料 31.7%，允许 ±3 个百分点）
  assert.ok(reduction > 0.28 && reduction < 0.35, `line reduction=${reduction}`);
  const multiLineBlocks = blocks.filter((block) => block.lines.length > 1);
  assert.equal(multiLineBlocks.length, 144);
  assert.ok(multiLineBlocks.every((block) => block.lines.length === 6));
  // 多行段占比 9.3%（研究 §6.2）
  const multiRatio = multiLineBlocks.length / blocks.length;
  assert.ok(multiRatio > 0.08 && multiRatio < 0.11, `multi-line ratio=${multiRatio}`);
  // 平均 1.47 行/块（研究 §6.2）
  const avgLines = lines.length / blocks.length;
  assert.ok(Math.abs(avgLines - 1.47) < 0.05, `avg lines=${avgLines}`);
  // id 稳定且按 y 排序
  assert.equal(blocks[0].id, "b0");
  assert.equal(blocks[1551].id, "b1551");
  for (let index = 1; index < blocks.length; index += 1) {
    assert.ok(blocks[index].bbox.y >= blocks[index - 1].bbox.y);
  }
  // bbox 覆盖段内各行；fontSizePx ≈ 行高中位；标题比 = 1（全语料同字号）
  const first = blocks[0];
  assert.equal(first.bbox.x, 100);
  assert.equal(first.bbox.width, 240);
  assert.equal(first.fontSizePx, BASE);
  assert.equal(first.headingRatio, 1);
  assert.ok(first.text.length > 0);
});

test("mergeLines respects gap / alignment / height / max-6-lines boundaries", () => {
  // 行距超过 0.55×行高 → 不合并
  const far = mergeLines([
    ocrLine("第一行", 100, 100, 200),
    ocrLine("第二行", 100, 100 + BASE + 30, 200),
  ]);
  assert.equal(far.length, 2);

  // 左边缘差过大且横向重叠 <50% → 不合并
  const misaligned = mergeLines([ocrLine("左列", 100, 100, 120), ocrLine("右列", 400, 124, 120)]);
  assert.equal(misaligned.length, 2);

  // 行高比超出 0.7–1.45 → 不合并
  const differentSize = mergeLines([
    ocrLine("正文", 100, 100, 200, 20),
    ocrLine("小字", 100, 122, 200, 10),
  ]);
  assert.equal(differentSize.length, 2);

  // 同一段最多 6 行 → 第 7 行另起一段
  const seven = [];
  for (let index = 0; index < 7; index += 1) {
    seven.push(ocrLine(`行${index}`, 100, 100 + index * (BASE + 2), 200));
  }
  const sevenBlocks = mergeLines(seven);
  assert.equal(sevenBlocks.length, 2);
  assert.equal(sevenBlocks[0].lines.length, 6);
  assert.equal(sevenBlocks[1].lines.length, 1);

  // 负坐标（虚拟桌面原点可为负）正常合并
  const negative = mergeLines([
    ocrLine("负坐标一", -1080, -470, 200),
    ocrLine("负坐标二", -1080, -470 + BASE + 3, 200),
  ]);
  assert.equal(negative.length, 1);
  assert.equal(negative[0].bbox.x, -1080);
  assert.equal(negative[0].bbox.y, -470);
});

test("mergeLines degrades safely on empty / invalid / duplicate input", () => {
  assert.deepEqual(mergeLines([]), []);
  assert.deepEqual(mergeLines(null), []);
  assert.deepEqual(mergeLines([undefined, null]), []);
  // 零尺寸、NaN、空文本全部过滤
  assert.deepEqual(
    mergeLines([
      { text: "零宽", confidence: 0.9, box: { x: 0, y: 0, width: 0, height: 20 } },
      { text: "NaN", confidence: 0.9, box: { x: Number.NaN, y: 0, width: 10, height: 20 } },
      { text: "", confidence: 0.9, box: { x: 0, y: 0, width: 10, height: 20 } },
    ]),
    [],
  );
  // 置信度越界被钳制；重叠的重复行不合并（避免把同一行重复翻译两次）
  const clamped = mergeLines([
    ocrLine("置信度", 0, 0, 100, 20, 3),
    ocrLine("置信度", 0, 0, 100, 20, -1),
  ]);
  assert.equal(clamped.length, 2);
  assert.equal(clamped[0].lines[0].confidence, 1);
  assert.equal(clamped[1].lines[0].confidence, 0);
});

test("mergeLines joins CJK lines without spaces and latin lines with spaces", () => {
  const cjk = mergeLines([ocrLine("缓存口径", 0, 0, 200), ocrLine("已落地", 0, 22, 200)]);
  assert.equal(cjk.length, 1);
  assert.equal(cjk[0].text, "缓存口径已落地");

  const latin = mergeLines([ocrLine("Cache key", 0, 0, 200), ocrLine("is stable", 0, 22, 200)]);
  assert.equal(latin[0].text, "Cache key is stable");

  // 标题比：40px 行高相对 20px 中位 → 2
  const heading = mergeLines([
    ocrLine("正文一", 0, 0, 200, 20),
    ocrLine("正文二", 0, 24, 200, 20),
    ocrLine("大标题", 0, 200, 200, 40),
  ]);
  const headingBlock = heading.find((block) => block.fontSizePx === 40);
  assert.ok(headingBlock);
  assert.equal(headingBlock.headingRatio, 2);
});

/* ───────────────────────── 自动换行 ───────────────────────── */

test("wrapText wraps latin words greedily", () => {
  const lines = wrapText("Hello world foo bar", 100, 20);
  assert.deepEqual(lines, ["Hello", "world foo", "bar"]);
});

test("wrapText breaks CJK per character", () => {
  const lines = wrapText("你好世界你好世界", 60, 20);
  assert.deepEqual(lines, ["你好世", "界你好", "世界"]);
});

test("wrapText hard-splits over-long words", () => {
  const lines = wrapText("abcdefghij", 20, 10);
  assert.deepEqual(lines, ["abc", "def", "ghi", "j"]);
});

test("wrapText supports injected measure and maxLines", () => {
  // 注入测量：每个字符固定 8px（与字号无关）
  const measure = (value) => value.length * 8;
  assert.deepEqual(wrapText("aa bb cc dd", 40, 12, measure), ["aa bb", "cc dd"]);
  assert.deepEqual(wrapText("aa bb cc dd", 40, 12, measure, { maxLines: 1 }), ["aa bb"]);
  // 空文本 / 非法宽度
  assert.deepEqual(wrapText("", 100, 20), []);
  assert.deepEqual(wrapText("   ", 100, 20), []);
  assert.deepEqual(wrapText("文本", 0, 20), []);
});

/* ───────────────────────── 字号自适应 ───────────────────────── */

test("fitFont keeps original size when text already fits (tier same)", () => {
  const block = makeBlock({ text: "Hello", width: 200, height: 20 });
  const result = fitFont(block, "Hello", {});
  assert.equal(result.ok, true);
  assert.equal(result.tier, "same");
  assert.equal(result.fontRatio, 1);
  assert.equal(result.fontSizePx, 20);
  assert.deepEqual(result.lines, ["Hello"]);
});

test("fitFont shrinks the size for a single line (tier shrink, 5% steps)", () => {
  const block = makeBlock({ text: "a".repeat(40), width: 300, height: 20 });
  const result = fitFont(block, "a".repeat(40), {});
  assert.equal(result.ok, true);
  assert.equal(result.tier, "shrink");
  // 40 × 0.52 × 20 = 416 > 300；0.70×20 = 14px → 291.2 ≤ 300
  assert.equal(result.fontRatio, 0.7);
  assert.equal(result.fontSizePx, 14);
  assert.ok(result.fontSizePx >= FIT_MIN_RATIO * 20);
  assert.ok(result.fontSizePx <= FIT_MAX_RATIO * 20);
  // 全部档位都落在 5% 网格上
  assert.ok(Math.abs((result.fontRatio * 100) % 5) < 1e-6);
});

test("fitFont wraps into 2–3 lines when vertical space allows (tier wrap)", () => {
  const block = makeBlock({ text: "b".repeat(30), width: 200, height: 20 });
  const result = fitFont(block, "b".repeat(30), {
    availRect: { x: 100, y: 100, width: 200, height: 40 },
  });
  assert.equal(result.ok, true);
  assert.equal(result.tier, "wrap");
  assert.ok(result.lines.length >= 2 && result.lines.length <= 3);
  const totalHeight = result.lines.length * result.lineHeightPx;
  assert.ok(totalHeight <= 40 + 1e-6);
  assert.ok(result.fontSizePx <= 20 && result.fontSizePx >= 12);
});

test("fitFont falls back when nothing fits, and respects the 11 CSS px floor", () => {
  // 纵向空间不足 + 长文本 → fallback（调用方降级气泡）
  const tight = makeBlock({ text: "c".repeat(50), width: 100, height: 10 });
  const failed = fitFont(tight, "c".repeat(50), {});
  assert.equal(failed.ok, false);
  assert.equal(failed.tier, "fallback");

  // dpr=2 ⇒ 物理下限 22px > 源字号 20px ⇒ 任何档位都不可读 → fallback
  const hiDpi = makeBlock({ text: "Hello", width: 2000, height: 20 });
  const hiDpiResult = fitFont(hiDpi, "Hello", { dpr: 2 });
  assert.equal(hiDpiResult.ok, false);
  assert.equal(hiDpiResult.tier, "fallback");

  // 空译文 / 非法区域 → fallback（不抛异常）
  assert.equal(fitFont(makeBlock(), "").ok, false);
  assert.equal(
    fitFont(makeBlock(), "文本", { availRect: { x: 0, y: 0, width: 0, height: 0 } }).ok,
    false,
  );
});

test("fitFont never picks a size outside [0.60, 1.00] × base", () => {
  for (const width of [80, 120, 200, 320, 480]) {
    for (const height of [12, 20, 32]) {
      const block = makeBlock({ text: "样例文本", width, height });
      for (const pressure of [1.0, 1.82, 2.46, 2.62]) {
        const translation = englishForWidthPressure(20, pressure);
        const result = fitFont(block, translation, { maxLines: 3 });
        if (!result.ok) continue;
        assert.ok(result.fontRatio >= FIT_MIN_RATIO - 1e-9, `ratio=${result.fontRatio}`);
        assert.ok(result.fontRatio <= FIT_MAX_RATIO + 1e-9, `ratio=${result.fontRatio}`);
        assert.ok(result.fontSizePx >= DEFAULT_MIN_FONT_PX - 1e-9);
      }
    }
  }
});

/* ───────────────────────── 颜色 / 对比度 ───────────────────────── */

test("contrast helpers follow WCAG relative luminance", () => {
  assert.ok(Math.abs(relativeLuminance("#FFFFFF") - 1) < 1e-6);
  assert.ok(relativeLuminance("#000000") < 1e-6);
  const white = contrastRatio(COVER_TEXT_LIGHT, "#0B0E12");
  const black = contrastRatio(COVER_TEXT_DARK, "#0B0E12");
  assert.ok(white > black);
  assert.ok(white > 4.5);
  // 参数既可传颜色也可传亮度（亮度 0 等价于纯黑）
  assert.ok(
    Math.abs(contrastRatio(COVER_TEXT_LIGHT, 0) - contrastRatio(COVER_TEXT_LIGHT, "#000000")) <
      1e-9,
  );
});

test("pickTextStyle: dark background → light text, light background → dark text, mid → scrim", () => {
  const dark = pickTextStyle(0.08);
  assert.equal(dark.color, COVER_TEXT_LIGHT);
  assert.ok(dark.contrastRatio >= 4.5);
  assert.equal(dark.scrimAlpha, 0);

  const light = pickTextStyle(0.92);
  assert.equal(light.color, COVER_TEXT_DARK);
  assert.ok(light.contrastRatio >= 4.5);
  assert.equal(light.scrimAlpha, 0);

  // 中灰底（≈0.19）两种字色都 <4.5:1 → 触发 0.35–0.45 衬底并达到 4.5:1
  const mid = pickTextStyle(0.19);
  assert.ok(mid.scrimAlpha >= 0.35 && mid.scrimAlpha <= 0.45);
  assert.ok(mid.contrastRatio >= 4.5);
  // 越界亮度被钳制，不抛异常
  assert.equal(pickTextStyle(-2).color, COVER_TEXT_LIGHT);
  assert.equal(pickTextStyle(Number.NaN).color, COVER_TEXT_DARK);
});

/* ───────────────────────── 空白扩展 ───────────────────────── */

test("expandForBlock uses right/down whitespace with research caps", () => {
  const block = makeBlock({ text: "中文", width: 100, height: 20 });
  const rightPeer = makeBlock({ id: "right", x: 300, y: 100, width: 100, height: 20 });
  const belowPeer = makeBlock({ id: "below", x: 100, y: 160, width: 100, height: 20 });
  const expanded = expandForBlock(block, [block, rightPeer, belowPeer]);
  // 横向 ≤ min(右侧空白 100, 0.6×100=60, 320) − (3px 邻居间隙 + 2.8px 模糊 pad) = 54.2
  assert.ok(Math.abs(expanded.width - 154.2) < 1e-9, `width=${expanded.width}`);
  // 纵向 ≤ min(下方空白 40, 1.8×20=36) − 5.8px → 高 56
  assert.ok(Math.abs(expanded.height - 56) < 1e-9, `height=${expanded.height}`);
  assert.equal(expanded.x, 100);

  // 只吃横向空白（冲突回退路径）：高度不变
  const horizontalOnly = expandForBlock(block, [block, rightPeer, belowPeer], undefined, {
    vertical: false,
  });
  assert.ok(Math.abs(horizontalOnly.width - 154.2) < 1e-9);
  assert.equal(horizontalOnly.height, 20);

  // 无邻居时按 0.6×块宽 / 1.8×块高封顶
  const alone = expandForBlock(block, [block]);
  assert.ok(Math.abs(alone.width - 154.2) < 1e-9);
  assert.ok(Math.abs(alone.height - 56) < 1e-9);
});

/* ───────────────────────── 整屏版式计划 ───────────────────────── */

/** 构造一份符合研究分布（宽度压力 P50 1.82 / P95 2.62）的合成版面。 */
function buildPressureCorpus({ blocks = 20, stepY = 28 } = {}) {
  const inputs = [];
  for (let index = 0; index < blocks; index += 1) {
    const cjk = 10;
    const pressure = index % 5 === 4 ? 2.62 : 1.82; // 约 20% 落在 P95
    const block = makeBlock({
      id: `b${index}`,
      text: "中".repeat(cjk),
      x: 100,
      y: 100 + index * stepY,
      width: cjk * BASE,
      height: BASE,
    });
    inputs.push({
      block,
      translation: englishForWidthPressure(cjk, pressure),
      dark: index % 2 === 0,
    });
  }
  return inputs;
}

test("planCover: no overflow, font in [0.60,1.00], P50 target ≈0.80 with research expansion", () => {
  const inputs = buildPressureCorpus();
  const plan = planCover(inputs, {
    dpr: 1,
    bounds: { x: 0, y: 0, width: 1200, height: 900 },
    allowExpand: true,
  });
  assert.equal(plan.total, inputs.length);
  assert.equal(plan.bubbleCount, 0, "研究口径（段块+吃空白）不应有溢出块");
  assert.equal(plan.sameCount + plan.shrinkCount + plan.wrapCount, plan.total);
  // 研究实测「段块 + 允许吃空白」字号中位 0.80
  assert.ok(
    plan.medianFontRatio >= 0.6 && plan.medianFontRatio <= 1,
    `median=${plan.medianFontRatio}`,
  );
  assert.ok(
    plan.medianFontRatio >= 0.65 && plan.medianFontRatio <= 0.95,
    `P50 目标≈0.80，实际 ${plan.medianFontRatio}`,
  );
  assert.ok(plan.minFontPx >= DEFAULT_MIN_FONT_PX);
  for (const item of plan.items) {
    const base = inputs.find((input) => input.block.id === item.id).block.fontSizePx;
    assert.ok(item.fontRatio >= FIT_MIN_RATIO - 1e-9 && item.fontRatio <= FIT_MAX_RATIO + 1e-9);
    assert.ok(item.fontSizePx <= base + 1e-6);
    assert.ok(
      Math.abs((item.fontRatio * 100) % 5) < 1e-6,
      `fontRatio 必须落在 5% 网格：${item.fontRatio}`,
    );
    assertNoOverflow(item);
    assert.ok(item.coverRect.width > 0 && item.coverRect.height > 0);
    assert.ok(item.blurSigmaPx >= 1.8);
  }
  // 非气泡覆盖矩形互不重叠（冲突消解生效）
  const covers = plan.items.filter((item) => !item.bubble);
  for (let i = 0; i < covers.length; i += 1) {
    for (let j = i + 1; j < covers.length; j += 1) {
      assert.equal(rectsOverlap(covers[i].coverRect, covers[j].coverRect, 1), false);
    }
  }
});

test("planCover: char-expansion P50 2.46× / P95 3.59× never overflows (wrap/shrink/bubble)", () => {
  const inputs = [];
  for (let index = 0; index < 30; index += 1) {
    const cjk = 6 + (index % 4);
    const expansion = index % 10 === 9 ? 3.59 : 2.46;
    const block = makeBlock({
      id: `b${index}`,
      text: "字".repeat(cjk),
      x: 60,
      y: 60 + index * 26,
      width: cjk * BASE,
      height: BASE,
    });
    // 字符扩张比直接作用到字符数（研究 §6.1）
    const translation = "t".repeat(Math.round(cjk * expansion));
    inputs.push({ block, translation, dark: false });
  }
  const plan = planCover(inputs, { bounds: { x: 0, y: 0, width: 1000, height: 900 } });
  assert.equal(plan.total, 30);
  for (const item of plan.items) {
    if (item.bubble) {
      // 降级路径必须仍有可绘制气泡区域
      assert.ok(item.coverRect.width >= 0 && item.coverRect.height > 0);
      continue;
    }
    assertNoOverflow(item);
    assert.ok(item.fontSizePx >= DEFAULT_MIN_FONT_PX);
  }
});

test("planCover: single-block original-size wrap when whitespace allows", () => {
  const block = makeBlock({ text: "中".repeat(10), width: 200, height: 20 });
  const translation = englishForWidthPressure(10, 1.82);
  const plan = planCover([{ block, translation, dark: false }], {
    bounds: { x: 0, y: 0, width: 1200, height: 900 },
  });
  const item = plan.items[0];
  assert.equal(item.bubble, false);
  assert.ok(item.tier === "wrap" || item.tier === "shrink");
  assertNoOverflow(item);
});

test("planCover: narrow controls and sub-12px blocks degrade to bubble", () => {
  const narrow = makeBlock({ id: "narrow", text: "页签", x: 100, y: 100, width: 40, height: 20 });
  const tiny = makeBlock({ id: "tiny", text: "小字", x: 100, y: 300, width: 400, height: 10 });
  const plan = planCover(
    [
      { block: narrow, translation: "Collection.txt", dark: false },
      { block: tiny, translation: "tiny label", dark: true },
    ],
    { bounds: { x: 0, y: 0, width: 1200, height: 900 } },
  );
  assert.equal(plan.bubbleCount, 2);
  assert.ok(plan.items.every((item) => item.bubble && item.tier === "fallback"));
  assert.ok(plan.items.every((item) => item.coverRect.width > 0 && item.coverRect.height > 0));
  // 气泡不覆盖原块（绘制在块下方，可读性优先）
  const narrowItem = plan.items.find((item) => item.id === "narrow");
  assert.ok(narrowItem.coverRect.y >= narrow.bbox.y + narrow.bbox.height);
});

test("planCover: colors follow dark/light background and contrast target", () => {
  const darkInput = {
    block: makeBlock({ id: "dark", text: "深色", width: 200, height: 20 }),
    translation: "dark mode label",
    dark: true,
  };
  const lightInput = {
    block: makeBlock({ id: "light", text: "浅色", x: 100, y: 300, width: 200, height: 20 }),
    translation: "light mode label",
    dark: false,
  };
  const plan = planCover([darkInput, lightInput], {
    bounds: { x: 0, y: 0, width: 1200, height: 900 },
  });
  const darkItem = plan.items.find((item) => item.id === "dark");
  const lightItem = plan.items.find((item) => item.id === "light");
  assert.equal(darkItem.color, COVER_TEXT_LIGHT);
  assert.equal(lightItem.color, COVER_TEXT_DARK);
  for (const item of plan.items) {
    assert.ok(item.contrastRatio >= 4.5, `对比度不足：${item.contrastRatio}`);
  }
  // 精确亮度输入优先于 dark 布尔
  const custom = planCover(
    [
      {
        block: makeBlock({ id: "explicit", text: "显式亮度", width: 200, height: 20 }),
        translation: "explicit luminance",
        dark: true,
        backgroundLuminance: 0.95,
      },
    ],
    { bounds: { x: 0, y: 0, width: 1200, height: 900 } },
  );
  assert.equal(custom.items[0].color, COVER_TEXT_DARK);
});

test("planCover: conflict resolution re-lays out expanded covers", () => {
  // 两个水平相邻块：允许吃空白时两块的覆盖补丁都向右扩，仍不能压到一起
  const left = makeBlock({ id: "left", text: "左块中文", x: 100, y: 100, width: 100, height: 20 });
  const right = makeBlock({
    id: "right",
    text: "右块中文",
    x: 320,
    y: 100,
    width: 100,
    height: 20,
  });
  const plan = planCover(
    [
      { block: left, translation: "left block text", dark: false },
      { block: right, translation: "right block text", dark: false },
    ],
    { bounds: { x: 0, y: 0, width: 1200, height: 900 } },
  );
  assert.equal(plan.bubbleCount, 0);
  const [first, second] = plan.items;
  assert.equal(rectsOverlap(first.coverRect, second.coverRect, 1), false);
  assert.ok(first.coverRect.width > first.textRect.width);
});

test("planCover: handles empty / degenerate input without throwing", () => {
  assert.deepEqual(planCover([]), {
    items: [],
    total: 0,
    sameCount: 0,
    shrinkCount: 0,
    wrapCount: 0,
    bubbleCount: 0,
    medianFontRatio: 0,
    minFontPx: 0,
  });
  assert.equal(planCover(null).total, 0);
  const weird = planCover(
    [
      { block: makeBlock({ width: Number.NaN }), translation: "abc", dark: false },
      {
        block: {
          ...makeBlock({ id: "empty", text: "", width: 100, height: 20 }),
          text: "",
        },
        translation: "",
        dark: true,
      },
    ],
    {},
  );
  assert.ok(weird.total <= 2);
  for (const item of weird.items) {
    assert.ok(Number.isFinite(item.coverRect.x) && Number.isFinite(item.coverRect.y));
    assert.ok(item.coverRect.width >= 0 && item.coverRect.height >= 0);
  }
});
