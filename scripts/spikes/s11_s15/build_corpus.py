# -*- coding: utf-8 -*-
"""S-11 / S-15 语料构造脚本（可复跑）。

产出（默认写入 `.verify/spikes/S-11-S-15/corpus/`，gitignored）：
  1. `real/`  —— **100 张真实渲染截图**：headless Edge 渲染自建 HTML 页面（网页/聊天/文档/代码/深色模式
     各 20 张，中英混排，尺寸覆盖 1080p / 1200p / 1620p / 1440p / 3440x1440 超宽），每张都有
     **构造真值**（页面里每一行可见文本）。渲染时同一次 Edge 调用里用页面 JS 做三重校验：
       a. 每个 `.tr` 文本行的 `getBoundingClientRect()` 完全落在视口内（无裁切）；
       b. `body.innerText` 去空白后与嵌入的真值字符串**逐字符相等**（无隐藏元素 / 无省略号 / 无额外文本）；
       c. 截图裁剪后尺寸 == JS 报告的视口尺寸。
     校验结果写进 manifest，截图存 PNG。
  2. `synth/` —— **50 张合成无文字图**：纯色、渐变、噪声纹理、图标阵列、线条/条形、几何图形、空白 UI
     （不含任何文字字形），作为 S-11「无文字」负样本。
  3. `corpus_manifest.json` —— 逐图元数据：id / kind / category / size / hasText 真值 / 真值文本 /
     DOM 校验 / sha256 / 生成方法；并给定 S-11 的 100 张（50 real + 50 synth）与 S-15 的 100 张（全部 real）
     的 id 列表。

用法（仓库根目录，使用 .verify/venv-ocr）：
    # 小样本冒烟（每类 2 张、合成 6 张）
    .verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/build_corpus.py --real-per-cat 2 --synth 6
    # 全量（100 real + 50 synth，约 3 分钟）
    .verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/build_corpus.py
    # 只重建合成图 / 只重建真实截图
    .verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/build_corpus.py --skip-real
    .verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/build_corpus.py --skip-synth

说明：
- 页面全部用**确定性 seed** 生成；同一 seed 重跑得到相同文本与截图（Edge 字体渲染在同一台机器上稳定）。
- 「真值」来自构造内容，不是 OCR 结果；页面 JS 的 innerText 校验保证 DOM 文本 == 截图可见文本。
- 页面布局是**单列自上而下**的绝对定位行：DOM 顺序 == 视觉 y 顺序，因此 OCR 按「行分组 + 行内 x」
  排序后的拼接顺序可直接与真值比较（拼接规则见 run_ocr_eval.py）。
"""
from __future__ import annotations

import argparse
import hashlib
import html
import json
import math
import random
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, List, Tuple

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

REPO = Path(__file__).resolve().parents[3]
SPIKE = REPO / ".verify" / "spikes" / "S-11-S-15"
CORPUS = SPIKE / "corpus"
HTML_DIR = CORPUS / "html"
REAL_DIR = CORPUS / "real"
SYNTH_DIR = CORPUS / "synth"
META_DIR = SPIKE / "meta"
EDGE = Path(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")
# 本机 headless=new 的窗口装饰开销（Windows 11 / 系统 150% 缩放实测：2400x1400 窗口 → 2370x1304 视口）；
# 用 30/96 把目标视口换算成 --window-size，截图后按 JS 报告的实际视口裁剪。
WIN_PAD_X, WIN_PAD_Y = 30, 96
MAX_RENDER_TRIES = 2

CATEGORIES = ["web", "chat", "doc", "code", "dark"]
CAT_CN = {"web": "网页", "chat": "聊天", "doc": "文档", "code": "代码", "dark": "深色模式"}

# 尺寸计划：覆盖 1080p（≤3.0Mpx，预算不降采样）/ 2K（>3.0Mpx，预算降采样）/ 超宽屏
SIZE_PLAN = [
    (2560, 1440),
    (1920, 1080),
    (2560, 1440),
    (2880, 1620),
    (2560, 1440),
    (3440, 1440),
    (2560, 1440),
    (1920, 1200),
    (2560, 1440),
    (1920, 1080),
]

# 行样式：cls -> (行高 px, 与下一行间距 px, 字体尺寸/粗细等样式)
ROW_STYLE: Dict[str, Dict[str, Any]] = {
    "nav":      dict(h=40, gap=18, size=17, weight=500, family="ui"),
    "link":     dict(h=32, gap=14, size=16, weight=400, family="ui", color="#2563eb"),
    "h1":       dict(h=66, gap=22, size=46, weight=700, family="ui"),
    "h2":       dict(h=46, gap=16, size=30, weight=600, family="ui"),
    "h3":       dict(h=36, gap=12, size=22, weight=600, family="ui"),
    "body":     dict(h=34, gap=8, size=17, weight=400, family="ui"),
    "small":    dict(h=26, gap=6, size=14, weight=400, family="ui"),
    "meta":     dict(h=24, gap=6, size=13, weight=400, family="ui"),
    "btn":      dict(h=36, gap=14, size=16, weight=600, family="ui"),
    "chip":     dict(h=30, gap=12, size=14, weight=500, family="ui"),
    "bullet":   dict(h=32, gap=6, size=17, weight=400, family="ui"),
    "quote":    dict(h=36, gap=10, size=17, weight=400, family="ui", italic=True),
    "bubble":   dict(h=40, gap=16, size=17, weight=400, family="ui", bg=True),
    "bubble_r": dict(h=40, gap=16, size=17, weight=400, family="ui", bg=True, right=True),
    "sendername": dict(h=24, gap=2, size=14, weight=600, family="ui"),
    "code":     dict(h=25, gap=1, size=15, weight=400, family="mono", pre=True),
    "codehl":   dict(h=25, gap=1, size=15, weight=400, family="mono", pre=True, hl=True),
    "codetab":  dict(h=30, gap=0, size=14, weight=500, family="mono"),
    "status":   dict(h=26, gap=0, size=13, weight=400, family="mono"),
    "track":    dict(h=40, gap=6, size=17, weight=400, family="ui", bg=True),
    "toggle":   dict(h=38, gap=8, size=16, weight=400, family="ui"),
    "accent":   dict(h=34, gap=8, size=17, weight=600, family="ui"),
    "docbody":  dict(h=34, gap=8, size=17, weight=400, family="serif"),
    "doctitle": dict(h=58, gap=18, size=34, weight=700, family="serif"),
    "docmeta":  dict(h=26, gap=14, size=14, weight=400, family="ui"),
    "iconcap":  dict(h=30, gap=8, size=15, weight=500, family="ui"),
}

# ------------------------------------------------------------------ 文本池
CN_SENT = [
    "框选 AI 把「看懂屏幕」做成本地优先的能力：先检测文字，再识别，最后交给模型推理。",
    "整屏检测只需要一次前向推理，选区识别则复用检测缓存，避免重复计算。",
    "在混合 DPI 双屏环境下，截图坐标必须统一到物理像素，否则框选会偏出目标区域。",
    "本地 OCR 走 PP-OCRv4 的检测与识别两个 ONNX 模型，全部跑在 CPU 上。",
    "用户按下快捷键后，冻结帧在后台完成检测，选区一落定就能立刻识别。",
    "我们把「有没有文字」和「文字是什么」拆成两个问题，前者决定是否触发后续动作。",
    "噪点、渐变和图标阵列是最容易让检测模型误判的负样本，需要阈值兜底。",
    "字符准确率用编辑距离衡量，删除、插入、替换逐项统计，才能定位真实短板。",
    "深色模式下的低对比度小字，是识别质量下降的主要来源之一。",
    "整屏像素预算把单屏 4K 的检测耗时压到八百毫秒以内，同时几乎不丢框。",
    "混合中英文的界面文本，需要识别模型同时覆盖两种字符集与标点。",
    "我们选择不对屏幕文本做方向分类，因为绝大多数界面文字都是水平的。",
    "冻结帧只读一次，检测与识别都基于同一份内存位图，避免屏幕内容变化导致错位。",
    "当缓存没有命中时，流水线会回退到选区检测加识别，保证结果可用。",
    "产品把本地模型的下载、校验与断点续传都放在引导流程里，首次使用更顺滑。",
    "隐私方面，本地 OCR 与本地模型不会把截图内容发到任何服务器。",
    "云端模型能力更强，但需要用户显式填入密钥，并承担调用费用。",
    "把识别结果做成可编辑的检索词，是降低视觉幻觉影响的低成本手段。",
    "搜索结果的来源必须可访问、可核对，不确定的地方要显式标注。",
    "翻译链路同时支持云端与本地：质量优先走云端，离线可用走本地小模型。",
    "界面文案要短，提示要具体；错误信息必须给出可执行的下一步。",
    "托盘常驻进程需要把空闲内存控制在 120MB 以内，才算真正「轻」。",
    "冷启动时间从进程创建算起，到托盘可交互为止。",
    "全局热键的冲突检测要在注册失败时立刻给出改键引导。",
    "混合 DPI 的坐标换算要同时处理负坐标、反向拖拽与跨屏拖选。",
    "整屏截图失败时，先尝试另一条捕获路径，再给用户可操作的说明。",
    "离屏渲染与合成测试可以覆盖大量布局分支，但仍需要在真机上看一眼。",
    "灰度发布期间，体积门禁与性能基线都要进 CI，避免悄悄变慢。",
    "我们把阈值参数化，产品配置可以按机型与场景覆盖默认值。",
    "评测语料要覆盖真实页面、真实字体与真实分辨率，而不是只用理想化样本。",
    "每一轮验证记录都要包含日期、环境、commit、原始数据与结论。",
    "如果指标没达标，先定位是检测、识别还是拼接环节的问题，再决定处置。",
]
EN_UI = [
    "Search docs, guides and API references",
    "Local-first OCR · offline capable · CPU only",
    "Release notes: v0.9.2 (2026-11-05)",
    "Download for Windows 11",
    "Sign in with your workspace account",
    "Keyboard shortcut: Ctrl + Q",
    "All processing happens on this device",
    "Image search pipeline: vision → query → search → summary",
    "Premium plan saves 20% annually",
    "Team members: 12 seats included",
    "Status: all systems operational",
    "Latency budget: 800 ms (P95)",
]
CHAT_PEOPLE = ["林晓", "周明", "陈可", "王磊", "Yuki", "Alex", "李然", "赵一"]
CHAT_MSGS = [
    "这个阈值是不是太激进了？噪声图会误判吧。",
    "我刚跑了一版：面积阈值 40 以上基本就干净了。",
    "深色模式那几张识别率有点低，可能是小字太细。",
    "整屏检测的缓存口径确认了吗？选区只做 rec 对吧。",
    "对，P95 已经压到六百毫秒以内了。",
    "把失败样本贴一下，我看看是不是行序拼错了。",
    "中英混排的标点要不要统一成半角？",
    "先别改模型，这轮只调阈值和后处理。",
    "The corpus includes 100 rendered pages and 50 synthetic negatives.",
    "Can you share the per-category accuracy table?",
    "I'll rerun the eval with min area = 50 and compare.",
    "截图语料已经放进 .verify 了，大文件不进 git。",
    "行序按 y 分组再按 x 排，多列布局先不做。",
    "明天把推荐阈值写进产品配置，先不改 Rust。",
    "这条消息故意长一点，用来测试气泡内的中文换行与识别，请不要折叠。",
]
DOC_TITLE = [
    "框选 AI 技术验证阶段报告",
    "OCR 阈值与字符准确率评估说明",
    "本地优先 OCR 的工程实践",
    "关于整屏检测像素预算的说明",
    "图像搜索链路的可信度设计",
]
DOC_HEADINGS = [
    "一、验证目标",
    "二、方法与语料",
    "三、阈值扫描结果",
    "四、字符准确率",
    "五、产品处置建议",
    "六、遗留与后续",
]
DOC_SENT = [
    "本阶段的目标是把「有没有文字」与「文字识别得多准」两个问题拆开验证，分别给出可判定的阈值。",
    "语料由真实渲染截图与合成负样本两部分组成，前者覆盖网页、聊天、文档、代码与深色模式。",
    "每条记录都包含构造真值、检测框数、面积与置信度分布，以及最终的判定结果。",
    "阈值扫描以误判率与漏判率为约束，优先选择两类错误都留有余量的组合。",
    "字符准确率使用编辑距离计算，并区分删除、插入与替换三类错误。",
    "整屏检测的像素预算会改变检测精度，需要在原分辨率与预算口径下各测一遍。",
    "对于低对比度的小字，建议在产品侧提供「强制文本模式」作为兜底。",
    "后续计划把面积阈值与行数阈值配置化，并补充更多真实系统窗口截图。",
    "表格与公式的版面还原不在本轮范围内，留给后续的版面分析模型。",
]
DOC_META = ["作者：验证组 · 2026-11-05", "文档版本 v1.3 · 内部评审稿"]
DOC_FOOT = ["第 1 页 共 3 页", "字数：1,284 · 阅读时长约 5 分钟"]

CODE_SNIPPETS = [
    ("# 整屏检测：按像素预算降采样后再跑 det", "code"),
    ("def det_input_size(frame_w: int, frame_h: int, det_max_edge: int):", "code"),
    ("    scale = min(det_max_edge / max(frame_w, frame_h), 1.0)", "code"),
    ("    if frame_w * frame_h * scale * scale > DET_PIXEL_BUDGET:", "code"),
    ("        scale *= math.sqrt(DET_PIXEL_BUDGET / (frame_w * frame_h * scale * scale))", "code"),
    ("    return max(1, int(frame_w * scale)), max(1, int(frame_h * scale))", "code"),
    ("", "code"),
    ("// 文字存在判定：面积 / 置信度 / 行数三阈值", "code"),
    ("export function presence(boxes: OcrBox[], rect: RectPhys): boolean {", "code"),
    ("  const kept = boxes.filter((b) => b.confidence >= minConfidence", "code"),
    ("    && b.bbox.width * b.bbox.height >= minArea);", "code"),
    ("  return kept.length >= minLines;", "code"),
    ("}", "code"),
    ("", "code"),
    ("# 行序拼接：先按行分组，再按 x 排序", "code"),
    ("rows = group_by_visual_row(boxes, tol=0.5)", "code"),
    ("text = '\\n'.join(line for row in rows for line in row)", "code"),
    ("assert len(text) > 0, 'empty preview'", "code"),
    ("", "code"),
    ("// 选区识别：缓存命中时只跑 rec", "code"),
    ("async fn recognize_selection(&self, rect: RectPhys) -> OcrScan {", "code"),
    ("    let picked = filter_boxes(&cached, &rect, min_confidence);", "code"),
    ("    let boxes = self.engine.recognize(frame, picked).await?;", "code"),
    ("    Ok(OcrScan { boxes, has_text: true, preview })", "code"),
    ("}", "code"),
    ("", "code"),
    ("# 编辑距离：逐字符计算 CER", "code"),
    ("def cer(truth: str, hyp: str) -> float:", "code"),
    ("    dist = levenshtein(truth, hyp)", "code"),
    ("    return dist / max(1, len(truth))", "code"),
    ("", "code"),
    ("// 失败样本要带图、带真值、带识别文本", "code"),
    ("const failures = results.filter((r) => r.cer > 0.05);", "code"),
    ("console.log(failures.slice(0, 10));", "code"),
]
CODE_TABS = ["ocr_service.rs", "bench_ocr.py", "run_ocr_eval.py", "README.md"]
DARK_TRACKS = [
    ("夜航星", "不才", "4:12"),
    ("海底", "一支榴莲", "4:36"),
    ("光年之外", "G.E.M. 邓紫棋", "3:55"),
    ("Lemon", "米津玄師", "4:16"),
    ("起风了", "买辣椒也用券", "5:11"),
    ("平凡之路", "朴树", "5:02"),
    ("Something Just Like This", "The Chainsmokers", "4:07"),
    ("Merry Christmas Mr. Lawrence", "坂本龍一", "4:41"),
    ("晴天", "周杰伦", "4:29"),
    ("Blowing In The Wind", "Bob Dylan", "2:48"),
    ("这世界那么多人", "莫文蔚", "4:45"),
    ("Weightless", "Marconi Union", "8:08"),
]
DARK_SETTINGS = [
    ("外观", "跟随系统", "浅色 / 深色 / 跟随系统"),
    ("语言", "简体中文", "界面语言与 OCR 字符集"),
    ("快捷键", "Ctrl + Q", "全局唤起与框选"),
    ("本地模型", "Qwen2.5-0.5B Q4", "已下载 · 0.55 GiB"),
    ("网络代理", "自动", "仅云端能力需要"),
    ("隐私", "本地优先", "截图内容默认不出设备"),
    ("自动更新", "开启", "仅检查稳定版"),
    ("日志级别", "信息", "调试模式会额外写盘"),
]


# ------------------------------------------------------------------ 工具
def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def norm_ws(s: str) -> str:
    return "".join(s.split())


def esc(s: str) -> str:
    return html.escape(s, quote=False)


def bg_for(cat: str) -> str:
    if cat == "dark":
        return "#16181d"
    if cat == "code":
        return "#ffffff"
    return "#ffffff"


def palette(cat: str) -> Dict[str, str]:
    if cat == "dark":
        return dict(fg="#e8e8ea", dim="#9aa0aa", accent="#6ea8fe", sub="#b8bcc4", panel="#1d2026")
    if cat == "code":
        return dict(fg="#1f2328", dim="#6e7781", accent="#0550ae", sub="#57606a", panel="#f6f8fa")
    return dict(fg="#111827", dim="#6b7280", accent="#2563eb", sub="#374151", panel="#f3f4f6")


# ------------------------------------------------------------------ 页面构造
class PageBuilder:
    """绝对定位的单列页面：deco（无文字装饰）在前，truth 行（.tr）在后，DOM 顺序 == y 顺序。"""

    def __init__(self, cat: str, w: int, h: int, seed: str):
        self.cat = cat
        self.w, self.h = w, h
        self.pal = palette(cat)
        self.rows: List[Dict[str, Any]] = []  # 视觉行：每行可有 1 个或多个文本片段
        self.decos: List[str] = []
        self.specs: List[Tuple[str, str, Dict[str, Any]]] = []  # (text, cls, extra)
        self.rng = random.Random(seed)
        self.margin = 80
        self.content_w = w - 2 * self.margin

    # -- 装饰（无文字）
    def deco_rect(self, x, y, w, h, color, radius=0, border=None):
        b = f"border:1px solid {border};" if border else ""
        self.decos.append(
            f'<div class="deco" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;'
            f'background:{color};border-radius:{radius}px;{b}"></div>'
        )

    def deco_circle(self, x, y, d, color):
        self.deco_rect(x, y, d, d, color, radius=d // 2)

    def top_bar(self, color, h=64, logo=True):
        self.deco_rect(0, 0, self.w, h, color)
        if logo:
            self.deco_rect(28, 16, 32, 32, self.pal["accent"], radius=8)
        self.deco_rect(self.w - 220, 20, 160, 26, self.pal["accent"], radius=13)

    # -- 文本行
    def add(self, text: str, cls: str = "body", left: int | None = None,
            right: int | None = None, extra: Dict[str, Any] | None = None):
        self.specs.append((text, cls, dict(left=left, right=right, **(extra or {}))))

    def _row_css(self, cls: str, y: int, left, right) -> str:
        st = ROW_STYLE[cls]
        # 注意：这段 CSS 放进 HTML 的 style="..." 双引号属性里，字体名必须用单引号，否则属性会被截断。
        fam = {
            "ui": "'Microsoft YaHei','Segoe UI',sans-serif",
            "mono": "Consolas,'Cascadia Mono',monospace",
            "serif": "SimSun,'Microsoft YaHei',serif",
        }[st["family"]]
        css = [
            f"top:{y}px",
            f"height:{st['h']}px",
            f"line-height:{st['h']}px",
            f"font-size:{st['size']}px",
            f"font-weight:{st['weight']}",
            f"font-family:{fam}",
            f"color:{self.pal['fg']}",
            "position:absolute",
            "white-space:pre" if st.get("pre") else "white-space:nowrap",
            "z-index:2",
        ]
        if st.get("italic"):
            css.append("font-style:italic")
        if left is not None:
            css.append(f"left:{left}px")
        elif right is not None:
            css.append(f"right:{right}px")
        else:
            css.append(f"left:{self.margin}px")
        if st.get("bg"):
            css += [f"background:{self.pal['panel']}", "border-radius:10px", "padding:0 16px"]
        if st.get("hl"):
            css.append("background:#fff8c5")
        return ";".join(css)

    def build(self, start_y: int, max_y: int) -> Tuple[List[Dict[str, Any]], str]:
        """按顺序放置行，超出 max_y 的行丢弃（不留半截）；返回实际行与真值文本。"""
        y = start_y
        out: List[Dict[str, Any]] = []
        truth_lines: List[str] = []
        for text, cls, extra in self.specs:
            st = ROW_STYLE[cls]
            h = st["h"]
            if y + h > max_y:
                break
            if text.strip():
                truth_lines.append(text)
                out.append({"text": text, "cls": cls, "y": y,
                            "css": self._row_css(cls, y, extra.get("left"), extra.get("right"))})
            y += h + st["gap"]
        truth = "\n".join(truth_lines)
        return out, truth

    def html(self, rows: List[Dict[str, Any]], truth_join: str) -> str:
        body_rows = "\n".join(
            f'<div class="tr" data-i="{i}" style="{r["css"]}">{esc(r["text"])}</div>'
            for i, r in enumerate(rows)
        )
        decos = "\n".join(self.decos)
        return f"""<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8">
<title>corpus</title>
<style>
html,body{{margin:0;padding:0;background:{bg_for(self.cat)};overflow:visible;}}
body{{width:{self.w}px;height:{self.h}px;position:relative;}}
.deco{{position:absolute;z-index:0;}}
.tr{{}}
</style></head>
<body>
{decos}
{body_rows}
<script>
const EXPECTED = {json.dumps(norm_ws(truth_join))};
const norm = (s) => s.replace(/\\s+/g, '');
window.addEventListener('load', () => {{
  setTimeout(() => {{
    const els = Array.from(document.querySelectorAll('.tr'));
    const bad = [];
    els.forEach((el, i) => {{
      const r = el.getBoundingClientRect();
      if (r.left < -0.5 || r.top < -0.5 || r.right > innerWidth + 0.5 || r.bottom > innerHeight + 0.5) bad.push(i);
    }});
    const inner = norm(document.body.innerText || '');
    document.title = JSON.stringify({{
      w: innerWidth, h: innerHeight, dpr: devicePixelRatio,
      scrollW: document.documentElement.scrollWidth, scrollH: document.documentElement.scrollHeight,
      n: els.length, bad: bad, innerMatch: inner === EXPECTED,
      innerLen: inner.length, expectedLen: EXPECTED.length,
    }});
  }}, 0);
}});
</script>
</body></html>"""


# ------------------------------------------------------------------ 各类型页面内容
def content_web(pb: PageBuilder, idx: int, h: int, sparse: bool) -> Tuple[int, int]:
    pal = pb.pal
    w = pb.w
    pb.top_bar("#ffffff" if idx % 2 == 0 else "#f8fafc")
    pb.deco_rect(0, 64, w, 1, "#e5e7eb")
    start = 96
    if sparse:
        # 图标密集页 + 少量 known 文本（S-11 低文本量正样本）
        for gx in range(4):
            for gy in range(3):
                x = pb.margin + gx * 180
                y = start + gy * 150
                pb.deco_rect(x, y, 120, 96, pal["panel"], radius=14)
                pb.deco_circle(x + 60, y + 26, 40, pal["accent"] if (gx + gy) % 2 == 0 else "#9ca3af")
                pb.deco_rect(x + 20, y + 74, 80, 8, "#d1d5db", radius=4)
        pb.add("框选 AI · 能力总览", "h2")
        pb.add("截图之后，先看有没有文字，再决定要不要识别。", "body")
        pb.add("离线可用 · 本地优先 · 平均检测 0.8 秒", "small")
        return start + 3 * 150 + 16, start + 3 * 150 + 16 + 320
    pb.add("框选 AI   产品   文档   定价   更新日志   关于我们", "nav")
    pb.add("截图即理解：把屏幕上的文字变成可用的信息", "h1")
    pb.add("本地 OCR + 云端推理，框选一次就能搜索、翻译或解答。", "h2", extra={})
    if h >= 1300:
        pb.add("支持 Windows 11，纯 CPU 运行，截图内容默认不出设备。", "body")
        pb.add("从整屏检测到选区识别，全链路在本地完成文字定位与转写。", "body")
    pb.add("核心能力", "h3")
    pb.add("· 整屏只做一次 det，选区复用缓存只跑 rec", "bullet")
    pb.add("· 混合 DPI 双屏坐标统一到物理像素，框选不偏", "bullet")
    pb.add("· 无网络、无密钥时可进入受限模式继续使用", "bullet")
    if h >= 1200:
        pb.add("为什么是本地优先", "h3")
        pb.add("截图里可能有聊天记录、代码、合同与身份信息。把 OCR 放在本地，", "body")
        pb.add("意味着这些内容不需要离开设备就能变成可编辑文本。", "body")
        pb.add("需要更强能力时，用户再显式开启云端模型与图片搜索。", "body")
    if h >= 1000:
        pb.add("性能基线（本机实测）", "h3")
        pb.add("整屏检测 P95 743 ms    ·    选区识别 P95 596 ms    ·    冷启动 1.2 s", "small")
        pb.add("以上数字来自 Windows 11 / i7-12650H / 混合 DPI 双屏环境。", "meta")
    if h >= 1300:
        pb.add("开始使用", "h3")
        pb.add("下载安装包后按引导完成三档 AI 服务选择，即可框选识别。", "body")
        pb.add("免费下载", "btn")
    footer_y = h - 70
    pb.deco_rect(0, footer_y - 1, w, 1, "#e5e7eb")
    pb.add("© 2026 框选 AI · 隐私政策 · 服务条款 · 反馈", "meta")
    return 96, footer_y - 30


def content_chat(pb: PageBuilder, idx: int, h: int, sparse: bool) -> Tuple[int, int]:
    pal = pb.pal
    w = pb.w
    pb.top_bar("#ffffff")
    pb.deco_rect(0, 64, w, 1, "#e5e7eb")
    pb.deco_circle(28, 84, 44, "#93c5fd")
    pb.deco_rect(88, 92, 160, 12, "#111827", radius=6)
    pb.deco_rect(88, 112, 110, 9, "#9ca3af", radius=4)
    pb.deco_rect(0, h - 76, w, 76, "#f8fafc")
    pb.deco_rect(pb.margin, h - 58, w - 2 * pb.margin - 120, 40, "#ffffff", radius=20, border="#e5e7eb")
    pb.deco_rect(w - pb.margin - 100, h - 58, 100, 40, pal["accent"], radius=20)
    start = 96
    if sparse:
        pb.add("产品讨论组 · 3 人", "h3")
        pb.add("林晓  10:24", "sendername")
        pb.add("阈值扫描结果出来了，误判是 0。", "bubble")
        pb.add("周明  10:26", "sendername")
        pb.add("深色小字那两张识别率 94%。", "bubble")
        return start, h - 100
    pb.add("产品讨论组 · 5 人 · 3 人在线", "h2")
    pb.add("林晓  10:24", "sendername")
    pb.add("阈值扫描跑完了，面积 40 以上噪声图基本就干净了。", "bubble")
    pb.add("周明  10:26", "sendername")
    pb.add("深色模式那几张识别率有点低，可能是 13px 小字太细。", "bubble")
    pb.add("林晓  10:27", "sendername")
    pb.add("先用置信度 0.5 + 面积 40 + 至少 1 行，漏判再看。", "bubble")
    pb.add("我  10:29", "sendername")
    pb.add("我把失败样本写到 .verify 了，截图和真值都在。", "bubble_r")
    if h >= 1100:
        pb.add("陈可  10:31", "sendername")
        pb.add("行序拼接确认按 y 分组再按 x 排，多列布局先不做。", "bubble")
        pb.add("Yuki  10:33", "sendername")
        pb.add("The corpus includes 100 rendered pages and 50 synthetic negatives.", "bubble_r")
        pb.add("周明  10:35", "sendername")
        pb.add("Can you share the per-category accuracy table?", "bubble")
        pb.add("林晓  10:36", "sendername")
        pb.add("整屏检测的缓存口径已经落地：选区只跑 rec，P95 596ms。", "bubble")
    if h >= 1300:
        pb.add("我  10:38", "sendername")
        pb.add("推荐阈值先写进配置，Rust 这轮不改，避免构建冲突。", "bubble_r")
        pb.add("陈可  10:40", "sendername")
        pb.add("好，我把阈值参数化脚本补上，参数名就用 min-area / min-confidence / min-lines。", "bubble")
        pb.add("Yuki  10:42", "sendername")
        pb.add("I'll rerun the eval with min area = 50 and compare the failure cases.", "bubble_r")
    pb.add("输入消息…", "small")
    return start, h - 100


def content_doc(pb: PageBuilder, idx: int, h: int, sparse: bool) -> Tuple[int, int]:
    pal = pb.pal
    w = pb.w
    pb.deco_rect(0, 0, w, 88, "#f3f4f6")
    pb.deco_rect(0, 88, w, 1, "#d1d5db")
    for i in range(6):
        pb.deco_rect(28 + i * 150, 14, 110, 22, "#e5e7eb", radius=4)
    pb.deco_rect(0, h - 60, w, 60, "#f9fafb")
    pb.deco_rect(0, h - 60, w, 1, "#e5e7eb")
    start = 116
    if sparse:
        pb.add("技术验证 · 摘要页", "doctitle")
        pb.add("作者：验证组 · 2026-11-05", "docmeta")
        pb.add("本轮只回答一个问题：屏幕上有文字时，检测与识别能否给出可用结果。", "docbody")
        return start, h - 90
    pb.add("文件   编辑   视图   插入   格式   帮助", "nav")
    pb.add("框选 AI 技术验证阶段报告", "doctitle")
    pb.add("作者：验证组 · 2026-11-05    文档版本 v1.3 · 内部评审稿", "docmeta")
    pb.add("一、验证目标", "h3")
    pb.add("本阶段的目标是把「有没有文字」与「文字识别得多准」两个问题拆开验证，分别给出可判定的阈值。", "docbody")
    pb.add("语料由真实渲染截图与合成负样本两部分组成，前者覆盖网页、聊天、文档、代码与深色模式。", "docbody")
    if h >= 1100:
        pb.add("二、方法与语料", "h3")
        pb.add("每条记录都包含构造真值、检测框数、面积与置信度分布，以及最终的判定结果。", "docbody")
        pb.add("阈值扫描以误判率与漏判率为约束，优先选择两类错误都留有余量的组合。", "docbody")
        pb.add("字符准确率使用编辑距离计算，并区分删除、插入与替换三类错误。", "docbody")
    if h >= 1250:
        pb.add("三、阈值扫描结果", "h3")
        pb.add("· 仅置信度阈值：噪声图仍有 6% 被判为有文字。", "bullet")
        pb.add("· 加面积阈值后误判率降到 2%，漏判率 0%。", "bullet")
        pb.add("· 行数阈值建议保持 1，否则稀疏页会漏判。", "bullet")
    if h >= 1350:
        pb.add("四、字符准确率", "h3")
        pb.add("整屏预算口径与原分辨率口径的差异主要来自检测框定位精度，而不是识别模型本身。", "docbody")
        pb.add("对于低对比度的小字，建议在产品侧提供「强制文本模式」作为兜底。", "quote")
        pb.add("后续计划把面积阈值与行数阈值配置化，并补充更多真实系统窗口截图。", "docbody")
    pb.add("第 1 页 共 3 页", "meta")
    return start, h - 90


def content_code(pb: PageBuilder, idx: int, h: int, sparse: bool) -> Tuple[int, int]:
    pal = pb.pal
    w = pb.w
    pb.deco_rect(0, 0, w, 92, "#f6f8fa")
    pb.deco_rect(0, 92, w, 1, "#d0d7de")
    for i, tab in enumerate(CODE_TABS):
        pb.deco_rect(24 + i * 210, 14, 190, 30, "#ffffff" if i == 0 else "#eaeef2", radius=6)
    pb.deco_rect(0, h - 40, w, 40, "#f6f8fa")
    pb.deco_rect(0, h - 40, w, 1, "#d0d7de")
    pb.deco_rect(0, 96, 76, h - 96 - 40, "#f6f8fa")
    start = 128
    if sparse:
        pb.add("app.py  ×", "codetab")
        pb.add("  1  # 文字存在判定（S-11）", "code")
        pb.add("  2  def presence(boxes, rect) -> bool:", "code")
        pb.add("  3      kept = filter_boxes(boxes, rect, MIN_CONF)", "code")
        pb.add("  4      return len(kept) >= MIN_LINES", "code")
        pb.add("  5", "code")
        return start, h - 80
    line_no = 1
    pb.add("ocr_service.rs   ×", "codetab")
    for text, _cls in CODE_SNIPPETS:
        if start + 25 * (line_no - 1) > h - 90:
            break
        pb.add(f"{line_no:>3}  {text}", "code")
        line_no += 1
    if h >= 1250:
        pb.add("Ln 12, Col 1    Spaces: 4    UTF-8    LF", "status")
    return start, h - 80


def content_dark(pb: PageBuilder, idx: int, h: int, sparse: bool) -> Tuple[int, int]:
    pal = pb.pal
    w = pb.w
    pb.deco_rect(0, 0, w, 92, "#1b1e24")
    pb.deco_rect(0, 92, w, 1, "#2a2e36")
    pb.deco_circle(32, 26, 40, "#3b4250")
    pb.deco_rect(92, 34, 200, 12, "#e8e8ea", radius=6)
    pb.deco_rect(92, 54, 120, 9, "#9aa0aa", radius=4)
    pb.deco_rect(w - 500, 26, 120, 34, "#262b33", radius=17)
    pb.deco_rect(w - 360, 26, 150, 34, "#6ea8fe", radius=17)
    pb.deco_rect(0, h - 54, w, 54, "#1b1e24")
    pb.deco_rect(0, h - 54, w, 1, "#2a2e36")
    start = 124
    if sparse:
        pb.add("系统设置 · 偏好", "h2")
        pb.add("外观", "accent")
        pb.add("跟随系统", "toggle")
        pb.add("语言", "accent")
        pb.add("简体中文", "toggle")
        pb.add("本地模型", "accent")
        pb.add("Qwen2.5-0.5B Q4 · 已下载", "toggle")
        pb.add("截图内容默认不出设备", "meta")
        return start, h - 80
    pb.add("系统设置 · 偏好", "h2")
    pb.add("现在播放", "h3")
    pb.add("夜航星 · 不才            4:12", "track")
    pb.add("海底 · 一支榴莲          4:36", "bubble")
    pb.add("Lemon · 米津玄師         4:16", "bubble")
    if h >= 1100:
        pb.add("外观", "accent")
        pb.add("跟随系统 · 深色模式在日落后自动切换", "toggle")
        pb.add("语言", "accent")
        pb.add("简体中文（OCR 字符集：中英混排）", "toggle")
        pb.add("快捷键", "accent")
        pb.add("Ctrl + Q 全局唤起，Esc 退出框选", "toggle")
    if h >= 1250:
        pb.add("本地模型", "accent")
        pb.add("Qwen2.5-0.5B Q4 · 0.55 GiB · 已下载", "toggle")
        pb.add("网络代理", "accent")
        pb.add("自动 · 仅云端能力需要", "toggle")
        pb.add("隐私", "accent")
        pb.add("截图内容默认不出设备，识别结果只写入本地历史", "toggle")
        pb.add("自动更新", "accent")
        pb.add("仅检查稳定版，下载后等你确认再安装", "toggle")
    pb.add("已是最新版本 · v0.9.2", "meta")
    return start, h - 80


CONTENT_FN = {"web": content_web, "chat": content_chat, "doc": content_doc,
              "code": content_code, "dark": content_dark}


# ------------------------------------------------------------------ 渲染
def run_edge(html_path: Path, png_path: Path, win_w: int, win_h: int,
             profile_dir: Path, timeout_s: int = 120) -> Tuple[str, str]:
    args = [
        str(EDGE),
        "--headless=new",
        "--disable-gpu",
        "--hide-scrollbars",
        "--force-device-scale-factor=1",
        f"--window-size={win_w},{win_h}",
        "--virtual-time-budget=3000",
        f"--user-data-dir={profile_dir}",
        "--no-first-run",
        "--no-default-browser-check",
        f"--screenshot={png_path}",
        "--dump-dom",
        html_path.as_uri(),
    ]
    proc = subprocess.run(args, capture_output=True, timeout=timeout_s)
    out = proc.stdout.decode("utf-8", errors="replace")
    err = proc.stderr.decode("utf-8", errors="replace")
    return out, err


def parse_title(dom: str) -> Dict[str, Any] | None:
    m = re.search(r"<title>(.*?)</title>", dom, re.S)
    if not m:
        return None
    raw = html.unescape(m.group(1)).strip()
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return None


def render_page(cat: str, idx: int, w: int, h: int, seed: str,
                profile_dir: Path, keep_html: bool = True) -> Dict[str, Any]:
    """渲染一张真实截图，返回 manifest 条目（含 DOM 校验）。"""
    sparse = idx >= 18
    pb = PageBuilder(cat, w, h, seed)
    start_y, max_y = CONTENT_FN[cat](pb, idx, h, sparse)
    # 顶部装饰已占位；正文从 start_y 开始；底部预留
    rows, truth = pb.build(start_y, max_y)
    if not rows:
        raise RuntimeError(f"{cat}-{idx} 无可用文本行")
    truth_lines = [r["text"] for r in rows]
    html_doc = pb.html(rows, "\n".join(truth_lines))
    page_id = f"{cat}_{idx:02d}"
    html_path = HTML_DIR / f"{page_id}.html"
    html_path.write_text(html_doc, encoding="utf-8")
    tmp_png = META_DIR / f"tmp_{page_id}.png"
    target_png = REAL_DIR / f"{page_id}.png"

    dom_info = None
    n_try = 0
    win_w, win_h = w + WIN_PAD_X, h + WIN_PAD_Y
    while n_try < MAX_RENDER_TRIES:
        n_try += 1
        dom_out, err = run_edge(html_path, tmp_png, win_w, win_h, profile_dir)
        dom_info = parse_title(dom_out)
        if dom_info is None:
            # 首次运行可能还没有用户数据目录；重试一次
            time.sleep(0.5)
            continue
        # 用 JS 报告的视口尺寸校正窗口尺寸后重渲染一次，保证截图尺寸 == 目标分辨率
        if (dom_info["w"], dom_info["h"]) != (w, h):
            win_w += w - dom_info["w"]
            win_h += h - dom_info["h"]
            continue
        break
    if dom_info is None:
        raise RuntimeError(f"{page_id} Edge 渲染失败：无 title 校验信息")
    if not tmp_png.exists():
        raise RuntimeError(f"{page_id} Edge 未产出截图")
    img = Image.open(tmp_png).convert("RGB")
    vw, vh = dom_info["w"], dom_info["h"]
    if (img.width, img.height) != (vw, vh):
        # 截图可能带窗口边距；按视口左上角裁剪
        img = img.crop((0, 0, vw, vh))
    if img.size != (w, h):
        raise RuntimeError(f"{page_id} 截图尺寸 {img.size} != 目标 {(w, h)}")
    img.save(target_png, optimize=True)
    tmp_png.unlink(missing_ok=True)
    if not keep_html:
        pass

    valid = bool(dom_info.get("innerMatch")) and not dom_info.get("bad") \
        and dom_info.get("scrollW", 0) <= vw and dom_info.get("scrollH", 0) <= vh
    return {
        "id": page_id,
        "kind": "real",
        "category": cat,
        "category_cn": CAT_CN[cat],
        "file": str(target_png.relative_to(REPO)).replace("\\", "/"),
        "width": w,
        "height": h,
        "hasText": True,
        "sparse_design": sparse,
        "truth_lines": truth_lines,
        "truth_text": "\n".join(truth_lines),
        "truth_chars_nonspace": len(norm_ws("".join(truth_lines))),
        "render": {
            "engine": "headless Edge (--headless=new), viewport via --window-size",
            "html": str(html_path.relative_to(REPO)).replace("\\", "/"),
            "win_size": [win_w, win_h],
            "dom_viewport": [vw, vh],
            "dpr": dom_info.get("dpr"),
            "tr_count": dom_info.get("n"),
            "bad_rects": dom_info.get("bad"),
            "inner_match": dom_info.get("innerMatch"),
            "inner_len": dom_info.get("innerLen"),
            "expected_len": dom_info.get("expectedLen"),
            "scroll_w": dom_info.get("scrollW"),
            "scroll_h": dom_info.get("scrollH"),
            "valid": valid,
            "tries": n_try,
        },
        "sha256": sha256_of(target_png),
        "bytes": target_png.stat().st_size,
        "seed": seed,
    }


# ------------------------------------------------------------------ 合成负样本
def _save_synth(name: str, img: Image.Image, method: str, cat: str,
                entries: List[Dict[str, Any]]) -> None:
    path = SYNTH_DIR / f"{name}.png"
    img.convert("RGB").save(path, optimize=True)
    entries.append({
        "id": name,
        "kind": "synth",
        "category": f"synth_{cat}",
        "category_cn": f"合成-{cat}",
        "file": str(path.relative_to(REPO)).replace("\\", "/"),
        "width": img.width,
        "height": img.height,
        "hasText": False,
        "method": method,
        "sha256": sha256_of(path),
        "bytes": path.stat().st_size,
    })


def build_synth(limit: int | None = None) -> List[Dict[str, Any]]:
    SYNTH_DIR.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(20261105)
    entries: List[Dict[str, Any]] = []

    def add(name, img, method, cat):
        if limit is not None and len(entries) >= limit:
            return
        _save_synth(name, img, method, cat, entries)

    def canvas(w, h, color=(255, 255, 255)):
        return Image.new("RGB", (w, h), color)

    # 1) 纯色 / 近纯色（6）
    solids = [((2560, 1440), (245, 246, 248), "浅灰纯色（空白页面）"),
              ((1920, 1080), (32, 34, 38), "深灰纯色（空白深色页面）"),
              ((2560, 1440), (10, 132, 255), "蓝色纯色（强调色块）"),
              ((3440, 1440), (250, 250, 250), "超宽近白纯色"),
              ((1920, 1200), (18, 18, 18), "近黑纯色（全屏独占画面）"),
              ((2560, 1440), (240, 244, 255), "浅蓝纯色")]
    for i, (size, color, desc) in enumerate(solids):
        add(f"synth_solid_{i:02d}", canvas(*size, color), f"PIL Image.new 纯色 {color}；{desc}", "solid")

    # 2) 渐变（7）
    grads = [
        ("线性渐变（对角，浅蓝→白）", (2560, 1440), "linear", (219, 234, 254), (255, 255, 255)),
        ("径向渐变（暖色辉光）", (2560, 1440), "radial", (255, 214, 170), (255, 248, 240)),
        ("线性渐变（深色主题）", (1920, 1080), "linear", (24, 28, 38), (44, 52, 70)),
        ("线性渐变（品牌紫→粉）", (2560, 1440), "linear", (139, 92, 246), (236, 72, 153)),
        ("径向渐变（中心亮）", (2880, 1620), "radial", (250, 250, 250), (226, 232, 240)),
        ("多段渐变（彩虹）", (1920, 1200), "rainbow", (0, 0, 0), (0, 0, 0)),
        ("线性渐变（4K 壁纸风格）", (3440, 1440), "linear", (15, 23, 42), (56, 189, 248)),
    ]

    def gradient(size, kind, c0, c1):
        w, h = size
        y = np.linspace(0, 1, h, dtype=np.float32)[:, None]
        x = np.linspace(0, 1, w, dtype=np.float32)[None, :]
        if kind == "linear":
            t = (x + y) / 2
        elif kind == "radial":
            t = np.sqrt((x - 0.5) ** 2 + (y - 0.5) ** 2) / 0.707
        elif kind == "rainbow":
            t = x
            out = np.zeros((h, w, 3), dtype=np.float32)
            out[..., 0] = 128 + 127 * np.sin(2 * math.pi * (t + 0.0))
            out[..., 1] = 128 + 127 * np.sin(2 * math.pi * (t + 0.33))
            out[..., 2] = 128 + 127 * np.sin(2 * math.pi * (t + 0.66))
            return Image.fromarray(np.clip(out, 0, 255).astype(np.uint8))
        else:
            t = x
        t = np.clip(t, 0, 1)[..., None]
        a = np.array(c0, dtype=np.float32)
        b = np.array(c1, dtype=np.float32)
        out = a[None, None, :] * (1 - t) + b[None, None, :] * t
        return Image.fromarray(np.clip(out, 0, 255).astype(np.uint8))

    for i, (desc, size, kind, c0, c1) in enumerate(grads):
        add(f"synth_gradient_{i:02d}", gradient(size, kind, c0, c1),
            f"numpy 生成 {kind} 渐变；{desc}", "gradient")

    # 3) 噪声 / 纹理（8）
    def noise_img(size, mode):
        w, h = size
        if mode == "uniform":
            arr = rng.integers(0, 256, (h, w, 3), dtype=np.uint8)
        elif mode == "gauss":
            arr = np.clip(rng.normal(128, 38, (h, w, 3)), 0, 255).astype(np.uint8)
        elif mode == "saltpepper":
            arr = np.full((h, w, 3), 220, dtype=np.uint8)
            mask = rng.random((h, w))
            arr[mask < 0.04] = 0
            arr[mask > 0.96] = 255
        elif mode == "grain":
            base = np.full((h, w, 3), 128, dtype=np.uint8)
            arr = np.clip(base + rng.normal(0, 18, (h, w, 3)), 0, 255).astype(np.uint8)
        elif mode == "smoothed":
            raw = rng.normal(128, 55, (h // 8, w // 8, 3)).astype(np.float32)
            im = Image.fromarray(np.clip(raw, 0, 255).astype(np.uint8)).resize((w, h), Image.BICUBIC)
            arr = np.asarray(im).copy()
        elif mode == "stripes":
            arr = np.zeros((h, w, 3), dtype=np.uint8) + 240
            for y0 in range(0, h, 3):
                arr[y0:y0 + 1, :, :] = 30
        elif mode == "checker":
            arr = np.zeros((h, w, 3), dtype=np.uint8)
            cell = 8
            ys, xs = np.mgrid[0:h, 0:w]
            m = ((ys // cell + xs // cell) % 2).astype(bool)
            arr[:] = 235
            arr[m] = 40
        else:  # speckle
            arr = np.full((h, w, 3), 245, dtype=np.uint8)
            for _ in range(4000):
                cx, cy = int(rng.integers(0, w)), int(rng.integers(0, h))
                r = int(rng.integers(1, 4))
                arr[max(0, cy - r):cy + r, max(0, cx - r):cx + r] = int(rng.integers(0, 90))
        return Image.fromarray(arr)

    noises = [("uniform", (2560, 1440), "均匀随机噪声（全屏）"),
              ("gauss", (1920, 1080), "高斯噪声（中等对比度）"),
              ("saltpepper", (2560, 1440), "椒盐噪声"),
              ("grain", (2880, 1620), "胶片颗粒（浅灰底）"),
              ("smoothed", (2560, 1440), "平滑随机纹理（低频色块）"),
              ("stripes", (1920, 1080), "横向细条纹纹理（类似扫描线）"),
              ("checker", (2560, 1440), "棋盘格纹理"),
              ("speckle", (3440, 1440), "随机散点纹理（浅底）")]
    for i, (mode, size, desc) in enumerate(noises):
        add(f"synth_noise_{i:02d}", noise_img(size, mode), f"numpy 生成 {mode} 纹理；{desc}", "noise")

    # 4) 图标阵列 / 类文字行（9）——最接近「文字但无文字」的困难负样本
    def icons_img(size, variant, seed_i):
        w, h = size
        img = canvas(w, h, (255, 255, 255) if variant % 2 == 0 else (18, 20, 24))
        d = ImageDraw.Draw(img)
        dark = variant % 2 == 1
        fg = (230, 230, 235) if dark else (45, 55, 70)
        acc = (110, 168, 254) if dark else (37, 99, 235)
        sub = (150, 155, 165) if dark else (180, 186, 195)
        if variant in (0, 1):  # 图标网格 + 条状说明
            cols = 4 if w >= 2500 else 3
            for gy in range(3):
                for gx in range(cols):
                    x = 90 + gx * ((w - 180) // cols)
                    y = 110 + gy * 210
                    d.rounded_rectangle([x, y, x + 120, y + 92], radius=12, fill=sub if not dark else (38, 42, 50))
                    d.ellipse([x + 40, y + 20, x + 80, y + 60], fill=acc if (gx + gy) % 2 == 0 else fg)
                    d.rounded_rectangle([x + 24, y + 68, x + 96, y + 78], radius=4, fill=sub)
            for i in range(6):
                y = 110 + 3 * 210 + 40 + i * 28
                d.rounded_rectangle([90, y, 90 + int(rng.integers(160, 520)), y + 12], radius=6,
                                    fill=fg if i % 3 else sub)
        elif variant in (2, 3):  # 类文字行：短条阵列（图标密集页的视觉近似）
            y = 120
            while y < h - 100:
                n = int(rng.integers(2, 5))
                x = 100
                for _ in range(n):
                    seg = int(rng.integers(60, 300))
                    d.rounded_rectangle([x, y, x + seg, y + 15], radius=5,
                                        fill=fg if rng.random() > 0.3 else sub)
                    x += seg + int(rng.integers(24, 60))
                y += int(rng.integers(26, 46))
        elif variant in (4, 5):  # 桌面图标阵列（无文字）
            for gy in range(4):
                for gx in range(5):
                    x, y = 80 + gx * 260, 100 + gy * 240
                    d.rounded_rectangle([x, y, x + 96, y + 80], radius=16,
                                        fill=acc if (gx * 7 + gy) % 3 == 0 else sub)
                    d.ellipse([x + 26, y + 20, x + 70, y + 64], fill=fg)
                    d.rounded_rectangle([x + 10, y + 92, x + 86, y + 104], radius=4, fill=fg)
        elif variant in (6, 7):  # 表格线 / 分隔线阵列
            for i in range(14):
                y = 100 + i * 88
                d.line([(80, y), (w - 80, y)], fill=sub, width=2)
            for i in range(8):
                x = 120 + i * 300
                d.line([(x, 100), (x, h - 60)], fill=sub, width=1)
            for i in range(10):
                y = 130 + i * 88
                d.rounded_rectangle([110, y, 110 + int(rng.integers(120, 360)), y + 14], radius=5, fill=fg)
        else:  # 条形码 / 波形（强竖线）
            x = 100
            while x < w - 120:
                bw = int(rng.integers(3, 14))
                if rng.random() < 0.5:
                    d.rectangle([x, 140, x + bw, h - 140], fill=fg)
                x += bw + int(rng.integers(4, 12))
        return img

    for i in range(9):
        size = [(2560, 1440), (1920, 1080), (2880, 1620), (2560, 1440), (3440, 1440),
                (1920, 1200), (2560, 1440), (1920, 1080), (2560, 1440)][i]
        add(f"synth_icons_{i:02d}", icons_img(size, i, i),
            f"PIL 绘制图标阵列/类文字条形（variant={i}，无任何字符）；最接近文字布局的困难负样本", "icons")

    # 5) 几何图形 / 线条（8）
    def shapes_img(size, variant):
        w, h = size
        dark = variant % 2 == 1
        img = canvas(w, h, (16, 18, 22) if dark else (249, 250, 251))
        d = ImageDraw.Draw(img)
        fg = (200, 205, 215) if dark else (90, 100, 115)
        acc = (110, 168, 254) if dark else (37, 99, 235)
        if variant in (0, 1):
            for i in range(40):
                cx, cy = int(rng.integers(0, w)), int(rng.integers(0, h))
                r = int(rng.integers(10, 90))
                d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=fg if i % 3 else acc, width=3)
        elif variant in (2, 3):
            for i in range(26):
                cx, cy = int(rng.integers(0, w)), int(rng.integers(0, h))
                s = int(rng.integers(20, 120))
                d.polygon([(cx, cy - s), (cx - s, cy + s), (cx + s, cy + s)], outline=fg, width=3)
        elif variant in (4, 5):
            for i in range(30):
                x0, y0 = int(rng.integers(0, w)), int(rng.integers(0, h))
                d.line([(x0, y0), (x0 + int(rng.integers(-400, 400)), y0 + int(rng.integers(-300, 300)))],
                       fill=fg if i % 2 else acc, width=int(rng.integers(1, 4)))
        else:
            for i in range(18):
                cx, cy = w // 2, h // 2
                r = 40 + i * 34
                d.ellipse([cx - r, cy - r * 0.6, cx + r, cy + r * 0.6], outline=fg, width=2)
        return img

    for i in range(8):
        size = [(2560, 1440), (1920, 1080), (2880, 1620), (2560, 1440),
                (3440, 1440), (1920, 1200), (2560, 1440), (1920, 1080)][i]
        add(f"synth_shapes_{i:02d}", shapes_img(size, i), f"PIL 绘制几何图形/线条（variant={i}）；无文字", "shapes")

    # 6) 空白 UI / 壁纸（7）
    def blank_ui(size, variant):
        w, h = size
        dark = variant >= 3
        img = canvas(w, h, (20, 22, 27) if dark else (240, 242, 245))
        d = ImageDraw.Draw(img)
        if variant == 0:  # 空白桌面 + 任务栏
            for _ in range(6):
                cx = int(rng.integers(0, w)); cy = int(rng.integers(0, h // 3))
                r = int(rng.integers(120, 320))
                d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(45, 90, 160))
            d.rectangle([0, h - 56, w, h], fill=(28, 30, 36))
            for i in range(8):
                d.rounded_rectangle([80 + i * 70, h - 44, 80 + i * 70 + 36, h - 12], radius=6, fill=(60, 64, 72))
        elif variant == 1:  # 空白 App 面板
            d.rectangle([0, 0, w, 72], fill=(255, 255, 255))
            d.rounded_rectangle([40, 20, 360, 52], radius=8, fill=(230, 233, 238))
            for i in range(4):
                d.rounded_rectangle([60, 130 + i * 140, w - 60, 230 + i * 140], radius=14, fill=(250, 251, 252),
                                    outline=(226, 229, 234), width=2)
        elif variant == 2:  # 空白浏览器窗口
            d.rectangle([0, 0, w, 120], fill=(248, 249, 251))
            for i in range(5):
                d.rounded_rectangle([30 + i * 220, 50, 210 + i * 220, 92], radius=10, fill=(232, 235, 240))
            d.rectangle([120, 160, w - 120, h - 120], fill=(255, 255, 255), outline=(220, 224, 230), width=3)
        elif variant == 3:  # 深色播放器（无文字）
            d.rounded_rectangle([w // 2 - 260, 180, w // 2 + 260, 180 + 520], radius=26, fill=(30, 33, 40))
            d.ellipse([w // 2 - 160, 300, w // 2 + 160, 620], outline=(90, 96, 108), width=3)
            d.ellipse([w // 2 - 70, 390, w // 2 + 70, 530], fill=(110, 168, 254))
            for i in range(3):
                d.rounded_rectangle([w // 2 - 180, 700 + i * 46, w // 2 + 180, 720 + i * 46], radius=8,
                                    fill=(58, 62, 72))
        elif variant == 4:  # 深色 IDE 空编辑器
            d.rectangle([0, 0, w, 64], fill=(38, 41, 48))
            d.rectangle([0, 64, 90, h], fill=(32, 34, 40))
            for i in range(40):
                d.rounded_rectangle([110, 96 + i * 30, 110 + int(rng.integers(30, 60)), 108 + i * 30],
                                    radius=3, fill=(70, 74, 84))
        elif variant == 5:  # 锁屏壁纸（模糊渐变 + 圆角卡片）
            arr = np.zeros((h, w, 3), dtype=np.uint8)
            for y in range(h):
                t = y / h
                arr[y, :, 0] = int(30 + 60 * t)
                arr[y, :, 1] = int(40 + 70 * t)
                arr[y, :, 2] = int(90 + 90 * t)
            img = Image.fromarray(arr).filter(ImageFilter.GaussianBlur(40))
            d = ImageDraw.Draw(img)
            d.rounded_rectangle([w // 2 - 300, h // 2 - 90, w // 2 + 300, h // 2 + 90], radius=28,
                                fill=(245, 246, 250))
        else:  # 空白文件管理器：图标占位格（无任何文字）
            d.rectangle([0, 0, w, 64], fill=(255, 255, 255))
            for i in range(6):
                d.rounded_rectangle([40 + i * 210, 18, 40 + i * 210 + 160, 46], radius=8, fill=(235, 238, 243))
            for gy in range(3):
                for gx in range(5):
                    x, y = 80 + gx * 300, 120 + gy * 260
                    d.rounded_rectangle([x, y, x + 120, y + 100], radius=12, fill=(238, 241, 246))
                    d.ellipse([x + 34, y + 20, x + 86, y + 72], fill=(180, 200, 230))
                    d.rounded_rectangle([x + 24, y + 112, x + 96, y + 124], radius=4, fill=(150, 160, 175))
        return img

    for i in range(6):
        size = [(2560, 1440), (1920, 1080), (2880, 1620), (1920, 1200), (2560, 1440), (3440, 1440)][i]
        add(f"synth_blankui_{i:02d}", blank_ui(size, i), f"PIL 绘制空白 UI/壁纸（variant={i}）；无文字", "blankui")

    # 7) 补充负样本到 50 张（2 纯色 + 2 渐变 + 1 摩尔纹 + 1 规则网格图标）
    for i, (size, color, desc) in enumerate([
            ((1920, 1080), (244, 246, 235), "浅绿纯色（阅读器空页）"),
            ((2560, 1440), (12, 22, 48), "深蓝纯色（播放器背景）")]):
        add(f"synth_solid_extra_{i:02d}", canvas(*size, color), f"PIL Image.new 纯色 {color}；{desc}", "solid")
    for i, (size, kind, c0, c1, desc) in enumerate([
            ((1920, 1080), "linear", (255, 255, 255), (226, 232, 240), "纵向浅灰渐变"),
            ((2560, 1440), "radial", (18, 20, 26), (46, 52, 66), "深色径向渐变")]):
        add(f"synth_gradient_extra_{i:02d}", gradient(size, kind, c0, c1),
            f"numpy 生成 {kind} 渐变；{desc}", "gradient")
    # 摩尔纹（干涉条纹，高频纹理，检测器容易产生伪框）
    w, h = 2560, 1440
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    moire = (np.sin(xx * 0.35) * np.sin(yy * 0.27) * 0.5 + 0.5) * 255
    add("synth_noise_extra_moire", Image.fromarray(np.stack([moire] * 3, axis=-1).astype(np.uint8)),
        "numpy 生成正弦干涉摩尔纹；无文字", "noise")
    # 规则网格图标（无文字）
    def grid_icons(size):
        w, h = size
        img = canvas(w, h, (250, 251, 252))
        d = ImageDraw.Draw(img)
        for gy in range(4):
            for gx in range(6):
                x, y = 60 + gx * 190, 100 + gy * 170
                d.rounded_rectangle([x, y, x + 100, y + 80], radius=14, fill=(230, 234, 240))
                d.ellipse([x + 26, y + 16, x + 74, y + 64], fill=(120, 150, 200))
                d.rounded_rectangle([x + 20, y + 88, x + 80, y + 96], radius=4, fill=(170, 178, 190))
        return img
    add("synth_icons_extra_grid", grid_icons((2560, 1440)),
        "PIL 绘制规则图标网格（无字符）；图标密集页负样本", "icons")

    return entries


# ------------------------------------------------------------------ 主流程
def pick_s11_ids(real: List[Dict[str, Any]], synth: List[Dict[str, Any]]) -> Dict[str, List[str]]:
    """S-11：每类 10 张真实（0-7 常规 + 18/19 稀疏）共 50，加 50 合成。"""
    real_ids = []
    for cat in CATEGORIES:
        sub = sorted([r for r in real if r["category"] == cat], key=lambda r: r["id"])
        idx = [r for r in sub if r["id"].endswith(("_00", "_01", "_02", "_03", "_04", "_05", "_06", "_07",
                                                  "_18", "_19"))]
        real_ids.extend(r["id"] for r in idx[:10])
    if len(real_ids) != 50:
        raise RuntimeError(f"S-11 real id 选取异常：{len(real_ids)}")
    synth_ids = [s["id"] for s in synth][:50]
    return {"real_ids": real_ids, "synth_ids": synth_ids}


def main() -> None:
    ap = argparse.ArgumentParser(description="S-11/S-15 语料构造")
    ap.add_argument("--real-per-cat", type=int, default=20, help="每类真实截图数量（全量 20）")
    ap.add_argument("--synth", type=int, default=50, help="合成图数量（全量 50）")
    ap.add_argument("--skip-real", action="store_true")
    ap.add_argument("--skip-synth", action="store_true")
    ap.add_argument("--keep-html", action="store_true", default=True)
    args = ap.parse_args()

    for d in (CORPUS, HTML_DIR, REAL_DIR, SYNTH_DIR, META_DIR):
        d.mkdir(parents=True, exist_ok=True)
    if not EDGE.exists():
        raise SystemExit(f"找不到 headless Edge：{EDGE}")

    t0 = time.time()
    manifest: Dict[str, Any] = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "host": {
            "os": "Windows 11 26200",
            "cpu": "i7-12650H (10C16T)",
            "ram_gib": 15.78,
            "display": "混合 DPI 双屏 2560x1440@150% + 1080x1920@100%",
        },
        "conventions": {
            "viewport": "headless Edge --headless=new --force-device-scale-factor=1；"
                        f"--window-size=(视口+{WIN_PAD_X},{WIN_PAD_Y})，截图按 JS 视口裁剪。",
            "truth": "页面构造文本；渲染时页面 JS 校验 innerText 去空白 == 真值、且所有 .tr 行完全在视口内。",
            "reading_order": "页面为单列绝对定位，DOM 顺序 == 视觉 y 顺序；OCR 按视觉行分组后行内 x 排序拼接。",
        },
        "real": [],
        "synth": [],
    }

    if not args.skip_real:
        profile_dir = META_DIR / "edge-profile"
        if profile_dir.exists():
            shutil.rmtree(profile_dir, ignore_errors=True)
        profile_dir.mkdir(parents=True, exist_ok=True)
        for cat in CATEGORIES:
            for i in range(args.real_per_cat):
                w, h = SIZE_PLAN[i % len(SIZE_PLAN)]
                seed = f"{cat}-{i:02d}-2026"
                try:
                    entry = render_page(cat, i, w, h, seed, profile_dir, args.keep_html)
                except Exception as exc:  # noqa: BLE001
                    print(f"[real] {cat}-{i:02d} 失败：{exc}", file=sys.stderr)
                    raise
                manifest["real"].append(entry)
                print(f"[real] {entry['id']:12s} {w}x{h} 行={len(entry['truth_lines']):3d} "
                      f"字符={entry['truth_chars_nonspace']:4d} dom_ok={entry['render']['valid']} "
                      f"{entry['bytes']//1024}KB  t={time.time()-t0:6.1f}s")
                if not entry["render"]["valid"]:
                    print(f"  !! DOM 校验未通过：{entry['render']}", file=sys.stderr)

    if not args.skip_synth:
        manifest["synth"] = build_synth(None if args.synth >= 50 else args.synth)
        print(f"[synth] {len(manifest['synth'])} 张合成无文字图")

    # S-11 / S-15 id 列表（小样本模式下只列现有 id，不计入验收）
    if manifest["real"]:
        if args.real_per_cat >= 20 and len(manifest["synth"]) >= 50:
            s11 = pick_s11_ids(manifest["real"], manifest["synth"])
            manifest["s11_real_ids"] = s11["real_ids"]
            manifest["s11_synth_ids"] = s11["synth_ids"]
        else:
            manifest["s11_real_ids"] = [r["id"] for r in manifest["real"]]
            manifest["s11_synth_ids"] = [s["id"] for s in manifest["synth"]]
        manifest["s11_ids"] = manifest["s11_real_ids"] + manifest["s11_synth_ids"]
        manifest["s15_ids"] = [r["id"] for r in manifest["real"]]

    mpath = CORPUS / "corpus_manifest.json"
    mpath.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    n_real = len(manifest["real"])
    n_synth = len(manifest["synth"])
    n_bad = sum(1 for r in manifest["real"] if not r["render"]["valid"])
    print(f"\n完成：real={n_real} synth={n_synth} dom_invalid={n_bad} 用时 {time.time()-t0:.1f}s")
    print("manifest ->", mpath)


if __name__ == "__main__":
    main()
