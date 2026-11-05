#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S-12 图片搜索可信度 —— 30 张评测语料生成器（HTML → headless Edge 截图）。

语料构成（30 张）：
    文字类 8：新闻正文、聊天记录、设置面板、错误弹窗、表格、中英混排、低对比度小字（难）、FAQ 列表
    图表类 8：柱状图、折线图、饼图、散点图、KPI 仪表盘、面积图、热力图（小图例，难）、堆叠柱状图
    商品类 7：耳机商品卡、键盘、手环、咖啡机、空气炸锅、台灯、虚构产品 Nebula X9（难：检索可能无来源）
    代码类 7：Python、JavaScript、终端输出、JSON、git diff、SQL、压缩混淆 JS（难）

每张图在 `corpus.json` 中带客观元数据：
    - expected_query_terms：一个好的检索词应包含的关键词（用于自动相关性判定）
    - recognizable_content：图中确实存在、识别结果应命中的文字片段
    - expect_uncertain：该图是否「应当显式标注不确定」（低清/低对比/虚构产品/无上下文）
    - ground_truth：人工核对用的客观描述

渲染：headless Edge（`--headless=new`），1000×700、deviceScaleFactor=1。
    参考 `scripts/spikes/s05_s06/bench_image_search.py` = 真实链路口径；截图本身为真实渲染结果，
    不含任何 mock 数据。

用法：
    python scripts/spikes/s12_s14/make_corpus.py            # 生成 HTML + 30 张截图 + corpus.json
    python scripts/spikes/s12_s14/make_corpus.py --no-shots # 只生成 HTML（调试）
"""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
CORPUS_DIR = REPO_ROOT / ".verify" / "spikes" / "S-12-S-14" / "corpus"
HTML_DIR = CORPUS_DIR / "html"
SHOT_DIR = CORPUS_DIR / "shots"
EDGE_CANDIDATES = [
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
]
VIEW_W, VIEW_H = 1000, 700
CATEGORY_COUNTS = {"text": 8, "chart": 8, "product": 7, "code": 7}


# --------------------------------------------------------------------------- #
# 通用模板与图形工具（全部内联，离线可渲染）
# --------------------------------------------------------------------------- #
BASE_CSS = """
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body {
  font-family: "Microsoft YaHei", "Segoe UI", "PingFang SC", sans-serif;
  background: #f2f4f7; color: #1f2328; width: 1000px; height: 700px; overflow: hidden;
}
.page { width: 100%; height: 100%; padding: 26px 32px; }
.h1 { font-size: 26px; font-weight: 700; margin: 0 0 6px; }
.h2 { font-size: 18px; font-weight: 600; margin: 0 0 10px; }
.sub { color: #6b7280; font-size: 13px; }
.card { background: #fff; border-radius: 10px; box-shadow: 0 1px 3px rgba(16,24,40,.10); padding: 18px 20px; }
.tag { display: inline-block; background: #eef2ff; color: #4338ca; border-radius: 999px; padding: 2px 10px; font-size: 12px; }
"""


def page(body: str, extra_css: str = "") -> str:
    return (
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\">"
        f"<style>{BASE_CSS}{extra_css}</style></head><body>{body}</body></html>"
    )


def svg(width: int, height: int, inner: str) -> str:
    return (
        f'<svg width="{width}" height="{height}" viewBox="0 0 {width} {height}" '
        f'xmlns="http://www.w3.org/2000/svg" font-family="Microsoft YaHei,Segoe UI,sans-serif">{inner}</svg>'
    )


def _text(x: float, y: float, s: str, size: int = 12, fill: str = "#374151",
          anchor: str = "start", weight: str = "normal") -> str:
    return (
        f'<text x="{x:.1f}" y="{y:.1f}" font-size="{size}" fill="{fill}" '
        f'text-anchor="{anchor}" font-weight="{weight}">{s}</text>'
    )


def bar_chart(title: str, labels: List[str], values: List[float], unit: str = "万元") -> str:
    w, h, pad_l, pad_b, pad_t = 660, 300, 56, 38, 26
    plot_w, plot_h = w - pad_l - 24, h - pad_b - pad_t
    vmax = max(values) * 1.15
    parts = [_text(pad_l, 18, title, 15, "#111827", weight="600")]
    # y 轴网格
    for i in range(5):
        y = pad_t + plot_h - plot_h * i / 4
        val = vmax * i / 4
        parts.append(f'<line x1="{pad_l}" y1="{y:.1f}" x2="{w-24}" y2="{y:.1f}" stroke="#e5e7eb" stroke-width="1"/>')
        parts.append(_text(pad_l - 8, y + 4, f"{val:.0f}", 10, "#9ca3af", anchor="end"))
    bw = plot_w / len(values) * 0.6
    gap = plot_w / len(values)
    for i, (lab, val) in enumerate(zip(labels, values)):
        x = pad_l + gap * i + (gap - bw) / 2
        bh = plot_h * val / vmax
        y = pad_t + plot_h - bh
        parts.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{bw:.1f}" height="{bh:.1f}" rx="3" fill="#4f6ef7"/>')
        parts.append(_text(x + bw / 2, y - 6, f"{val:g}", 10, "#1f2937", anchor="middle"))
        parts.append(_text(x + bw / 2, h - 12, lab, 11, "#4b5563", anchor="middle"))
    parts.append(_text(w - 24, 18, unit, 11, "#9ca3af", anchor="end"))
    return svg(w, h, "".join(parts))


def line_chart(title: str, series: Dict[str, List[float]], xlabels: List[str]) -> str:
    w, h, pad_l, pad_b, pad_t = 660, 300, 56, 38, 26
    plot_w, plot_h = w - pad_l - 30, h - pad_b - pad_t
    vals = [v for s in series.values() for v in s]
    vmin, vmax = min(vals) * 0.9, max(vals) * 1.1
    parts = [_text(pad_l, 18, title, 15, "#111827", weight="600")]
    for i in range(5):
        y = pad_t + plot_h - plot_h * i / 4
        parts.append(f'<line x1="{pad_l}" y1="{y:.1f}" x2="{w-30}" y2="{y:.1f}" stroke="#eef0f4"/>')
        parts.append(_text(pad_l - 8, y + 4, f"{vmin + (vmax-vmin)*i/4:.0f}", 10, "#9ca3af", anchor="end"))
    colors = ["#4f6ef7", "#f59e0b", "#10b981"]
    for si, (name, ys) in enumerate(series.items()):
        pts = []
        for i, y in enumerate(ys):
            px = pad_l + plot_w * i / max(1, len(ys) - 1)
            py = pad_t + plot_h - plot_h * (y - vmin) / (vmax - vmin)
            pts.append(f"{px:.1f},{py:.1f}")
        parts.append(f'<polyline points="{" ".join(pts)}" fill="none" stroke="{colors[si%3]}" stroke-width="2.5"/>')
        for i, p in enumerate(pts):
            x, y = p.split(",")
            parts.append(f'<circle cx="{x}" cy="{y}" r="3" fill="{colors[si%3]}"/>')
        parts.append(f'<rect x="{pad_l+8+si*110}" y="{pad_t+4}" width="10" height="10" rx="2" fill="{colors[si%3]}"/>')
        parts.append(_text(pad_l + 24 + si * 110, pad_t + 13, name, 11, "#374151"))
    for i, lab in enumerate(xlabels):
        x = pad_l + plot_w * i / max(1, len(xlabels) - 1)
        parts.append(_text(x, h - 12, lab, 11, "#4b5563", anchor="middle"))
    return svg(w, h, "".join(parts))


def pie_chart(title: str, slices: List[Tuple[str, float]]) -> str:
    w, h, r, cx, cy = 660, 300, 105, 190, 160
    total = sum(v for _, v in slices)
    colors = ["#4f6ef7", "#f59e0b", "#10b981", "#ef4444", "#8b5cf6", "#06b6d4"]
    parts = [_text(30, 18, title, 15, "#111827", weight="600")]
    angle = -math.pi / 2
    for i, (name, val) in enumerate(slices):
        frac = val / total
        end = angle + frac * 2 * math.pi
        large = 1 if frac > 0.5 else 0
        x1, y1 = cx + r * math.cos(angle), cy + r * math.sin(angle)
        x2, y2 = cx + r * math.cos(end), cy + r * math.sin(end)
        parts.append(
            f'<path d="M{cx},{cy} L{x1:.1f},{y1:.1f} A{r},{r} 0 {large} 1 {x2:.1f},{y2:.1f} Z" '
            f'fill="{colors[i%6]}" stroke="#fff" stroke-width="2"/>'
        )
        mid = (angle + end) / 2
        tx, ty = cx + r * 0.62 * math.cos(mid), cy + r * 0.62 * math.sin(mid)
        parts.append(_text(tx, ty + 4, f"{frac*100:.0f}%", 11, "#ffffff", anchor="middle", weight="600"))
        parts.append(f'<rect x="430" y="{70+i*28}" width="11" height="11" rx="2" fill="{colors[i%6]}"/>')
        parts.append(_text(448, 80 + i * 28, f"{name} {val:g}", 12, "#374151"))
        angle = end
    return svg(w, h, "".join(parts))


def scatter_chart(title: str, points: List[Tuple[float, float]]) -> str:
    w, h, pad_l, pad_b, pad_t = 660, 300, 56, 38, 26
    plot_w, plot_h = w - pad_l - 30, h - pad_b - pad_t
    parts = [_text(pad_l, 18, title, 15, "#111827", weight="600")]
    for i in range(5):
        y = pad_t + plot_h - plot_h * i / 4
        x = pad_l + plot_w * i / 4
        parts.append(f'<line x1="{pad_l}" y1="{y:.1f}" x2="{w-30}" y2="{y:.1f}" stroke="#eef0f4"/>')
        parts.append(f'<line x1="{x:.1f}" y1="{pad_t}" x2="{x:.1f}" y2="{pad_t+plot_h}" stroke="#eef0f4"/>')
    for i, (x, y) in enumerate(points):
        px = pad_l + plot_w * x / 100
        py = pad_t + plot_h - plot_h * y / 100
        parts.append(f'<circle cx="{px:.1f}" cy="{py:.1f}" r="5" fill="#4f6ef7" opacity="0.75"/>')
    parts.append(_text(pad_l + plot_w / 2, h - 8, "使用时长（小时）", 11, "#6b7280", anchor="middle"))
    parts.append(_text(pad_l - 10, pad_t + 6, "满意度", 10, "#6b7280", anchor="end"))
    return svg(w, h, "".join(parts))


def dashboard(kpis: List[Tuple[str, str, str]]) -> str:
    cells = []
    for i, (label, value, delta) in enumerate(kpis):
        x = 20 + (i % 4) * 240
        y = 70 + (i // 4) * 150
        up = delta.startswith("+")
        color = "#10b981" if up else "#ef4444"
        cells.append(
            f'<g><rect x="{x}" y="{y}" width="220" height="120" rx="10" fill="#fff" stroke="#e5e7eb"/>'
            + _text(x + 18, y + 30, label, 13, "#6b7280")
            + _text(x + 18, y + 76, value, 28, "#111827", weight="700")
            + _text(x + 18, y + 104, delta, 12, color)
            + "</g>"
        )
    return svg(960, 660, "".join(cells))


def area_chart(title: str, labels: List[str], series: List[List[float]]) -> str:
    w, h, pad_l, pad_b, pad_t = 660, 300, 56, 38, 26
    plot_w, plot_h = w - pad_l - 30, h - pad_b - pad_t
    vmax = max(v for s in series for v in s) * 1.1
    parts = [_text(pad_l, 18, title, 15, "#111827", weight="600")]
    colors = ["#4f6ef7", "#10b981"]
    for si, ys in enumerate(series):
        pts = []
        for i, v in enumerate(ys):
            px = pad_l + plot_w * i / max(1, len(ys) - 1)
            py = pad_t + plot_h - plot_h * v / vmax
            pts.append(f"{px:.1f},{py:.1f}")
        poly = f'M{pts[0]} L' + " L".join(pts[1:]) + f' L{pad_l+plot_w},{pad_t+plot_h} L{pad_l},{pad_t+plot_h} Z'
        parts.append(f'<path d="{poly}" fill="{colors[si%2]}" opacity="0.28"/>')
        parts.append(f'<polyline points="{" ".join(pts)}" fill="none" stroke="{colors[si%2]}" stroke-width="2"/>')
    for i, lab in enumerate(labels):
        x = pad_l + plot_w * i / max(1, len(labels) - 1)
        parts.append(_text(x, h - 12, lab, 11, "#4b5563", anchor="middle"))
    return svg(w, h, "".join(parts))


def heatmap(title: str, rows: List[str], cols: List[str], grid: List[List[int]]) -> str:
    cell = 44
    x0, y0 = 70, 46
    parts = [_text(20, 22, title, 15, "#111827", weight="600")]
    for j, col in enumerate(cols):
        parts.append(_text(x0 + j * cell + cell / 2, y0 - 10, col, 10, "#6b7280", anchor="middle"))
    for i, row in enumerate(rows):
        parts.append(_text(x0 - 12, y0 + i * cell + cell / 2 + 4, row, 10, "#6b7280", anchor="end"))
        for j in range(len(cols)):
            v = grid[i][j]
            alpha = 0.12 + 0.88 * v / max(1, max(max(r) for r in grid))
            parts.append(
                f'<rect x="{x0+j*cell}" y="{y0+i*cell}" width="{cell-4}" height="{cell-4}" rx="3" '
                f'fill="#ef4444" opacity="{alpha:.2f}"/>'
            )
            parts.append(_text(x0 + j * cell + cell / 2 - 2, y0 + i * cell + cell / 2 + 4, str(v), 10, "#fff", anchor="middle"))
    return svg(660, 320, "".join(parts))


def stacked_chart(title: str, labels: List[str], stacks: Dict[str, List[float]]) -> str:
    w, h, pad_l, pad_b, pad_t = 660, 300, 56, 38, 26
    plot_w, plot_h = w - pad_l - 30, h - pad_b - pad_t
    totals = [sum(s[i] for s in stacks.values()) for i in range(len(labels))]
    vmax = max(totals) * 1.12
    colors = ["#4f6ef7", "#f59e0b", "#10b981"]
    parts = [_text(pad_l, 18, title, 15, "#111827", weight="600")]
    gap = plot_w / len(labels)
    bw = gap * 0.55
    for i in range(len(labels)):
        base = pad_t + plot_h
        x = pad_l + gap * i + (gap - bw) / 2
        for si, (name, ys) in enumerate(stacks.items()):
            bh = plot_h * ys[i] / vmax
            base -= bh
            parts.append(f'<rect x="{x:.1f}" y="{base:.1f}" width="{bw:.1f}" height="{bh:.1f}" fill="{colors[si%3]}"/>')
        parts.append(_text(x + bw / 2, h - 12, labels[i], 11, "#4b5563", anchor="middle"))
    for si, name in enumerate(stacks):
        parts.append(f'<rect x="{pad_l+8+si*100}" y="{pad_t+2}" width="10" height="10" rx="2" fill="{colors[si%3]}"/>')
        parts.append(_text(pad_l + 24 + si * 100, pad_t + 11, name, 11, "#374151"))
    return svg(w, h, "".join(parts))


# --------------------------------------------------------------------------- #
# 页面内容
# --------------------------------------------------------------------------- #
def text_article() -> str:
    return page(
        '<div class="page"><div class="card">'
        '<div class="sub">科技 · 2026-11-04</div>'
        '<h1 class="h1">国产 AI 编程助手进入「本地优先」阶段</h1>'
        '<p style="font-size:15px;line-height:1.9;color:#374151">'
        '越来越多的开发者开始在本地运行代码大模型。与云端调用相比，本地推理不需要上传源码，'
        '在隐私与合规上更有优势；代价是首次加载较慢、内存占用更高。业内普遍认为，'
        '「本地完成补全、云端负责复杂重构」的混合模式将在未来两年成为主流。</p>'
        '<p style="font-size:15px;line-height:1.9;color:#374151">'
        '报告显示，2026 年第三季度本地推理工具的使用量同比增长 210%，其中代码补全占比 46%。</p>'
        '<div style="display:flex;gap:10px;margin-top:10px"><span class="tag">AI 编程</span>'
        '<span class="tag">本地推理</span><span class="tag">隐私</span></div>'
        "</div></div>"
    )


def text_chat() -> str:
    bubbles = [
        ("李工", "周五之前能把接口文档发我吗？", False),
        ("我", "可以，我今晚整理完发到群里。", True),
        ("李工", "顺便确认下 SA-1001 这个错误码是不是可重试的？", False),
        ("我", "是的，截图失败属于可重试错误，会提示用户重新框选。", True),
        ("李工", "👍 收到", False),
    ]
    rows = []
    for i, (who, msg, mine) in enumerate(bubbles):
        align = "flex-end" if mine else "flex-start"
        bg = "#dbeafe" if mine else "#ffffff"
        rows.append(
            f'<div style="display:flex;justify-content:{align};margin:8px 0">'
            f'<div style="max-width:70%;background:{bg};border-radius:10px;padding:10px 14px;font-size:14px">'
            f'<div class="sub" style="margin-bottom:3px">{who}</div>{msg}</div></div>'
        )
    return page(
        '<div class="page"><div class="h2">项目群 · 李工</div>'
        '<div style="display:flex;flex-direction:column">' + "".join(rows) + "</div></div>"
    )


def text_settings() -> str:
    rows = [
        ("AI 服务", "DeepSeek · deepseek-flash", "已连接"),
        ("翻译方式", "云端翻译", "默认"),
        ("本地模型", "Qwen2.5-0.5B Q4_K_M", "已下载"),
        ("快捷键", "Ctrl+Q", "可修改"),
        ("历史记录", "保留 30 天", "开启"),
    ]
    tr = "".join(
        f'<tr><td style="padding:12px 12px;color:#6b7280;width:140px">{a}</td>'
        f'<td style="padding:12px 12px">{b}</td>'
        f'<td style="padding:12px 12px;color:#10b981">{c}</td></tr>'
        for a, b, c in rows
    )
    return page(
        '<div class="page"><div class="card">'
        '<h2 class="h2">设置 · AI 与翻译</h2>'
        f'<table style="border-collapse:collapse;width:100%;font-size:14px">{tr}</table></div></div>'
    )


def text_error_dialog() -> str:
    return page(
        '<div class="page" style="display:flex;align-items:center;justify-content:center">'
        '<div class="card" style="width:520px;border:1px solid #fecaca">'
        '<div style="display:flex;gap:12px;align-items:flex-start">'
        '<div style="width:40px;height:40px;border-radius:50%;background:#fee2e2;color:#dc2626;'
        'display:flex;align-items:center;justify-content:center;font-size:22px">!</div>'
        '<div><div style="font-size:17px;font-weight:600;margin-bottom:8px">截图失败（SA-1001）</div>'
        '<div style="font-size:14px;color:#4b5563;line-height:1.7">'
        '当前窗口可能受 DRM 保护或处于全屏独占状态，无法获取画面。<br>'
        '该错误可重试：请尝试使用窗口截图模式，或退出全屏后重新框选。</div>'
        '<div style="margin-top:14px;display:flex;gap:10px">'
        '<button style="background:#2563eb;color:#fff;border:0;border-radius:6px;padding:8px 18px;font-size:14px">重试</button>'
        '<button style="background:#fff;border:1px solid #d1d5db;border-radius:6px;padding:8px 18px;font-size:14px">打开设置</button>'
        "</div></div></div></div></div>"
    )


def text_table() -> str:
    header = ["模型", "参数量", "常驻内存", "P95 时延", "适用场景"]
    data = [
        ["Qwen2.5-0.5B", "0.5B", "0.55 GiB", "3.21 s", "极小档 / 离线"],
        ["Qwen2.5-1.5B", "1.5B", "1.70 GiB", "6.72 s", "质量优先"],
        ["HY-MT1.5", "1.8B", "2.10 GiB", "21.84 s", "专用翻译"],
        ["deepseek-flash", "云端", "—", "0.87 s", "默认"],
    ]
    head = "".join(f'<th style="padding:10px;text-align:left;background:#f9fafb;font-weight:600">{h}</th>' for h in header)
    body = "".join(
        "<tr>" + "".join(f'<td style="padding:10px;border-top:1px solid #f1f2f4">{c}</td>' for c in row) + "</tr>"
        for row in data
    )
    return page(
        '<div class="page"><div class="card"><h2 class="h2">本地模型对比（实测）</h2>'
        f'<table style="border-collapse:collapse;width:100%;font-size:13px">{head}{body}</table></div></div>'
    )


def text_bilingual() -> str:
    return page(
        '<div class="page"><div class="card" style="font-size:15px;line-height:2">'
        '<h2 class="h2">Release Notes v1.2.3</h2>'
        '<p><b>新增</b>：本地极小模型（Qwen2.5-0.5B）支持，P95 3.21s，内存占用 0.55 GiB。</p>'
        '<p><b>Fixed</b>：hotkey conflicts are now detected reliably; Ctrl+Q can be changed.</p>'
        '<p><b>变更</b>：默认关闭 thinking，避免首字时延被 reasoning 推高。</p>'
        '<p><b>Known issue</b>：4K + 200% 缩放机型尚未完成真机验证。</p>'
        "</div></div>"
    )


def text_blurry() -> str:
    """低对比度 + 小字号：识别应明确标注不确定。"""
    return page(
        '<div class="page" style="background:#f0f0f0">'
        '<div style="font-size:9px;color:#c3c3c3;line-height:1.9">'
        '内部纪要（非公开）：本次评审讨论了三个方向，一是继续优化 OCR 的像素预算，'
        '二是补齐混合 DPI 下的坐标换算，三是评估本地模型的降档策略。'
        '结论是优先保证 P95 与稳定性，功能排期顺延到下一迭代。'
        '参与人：张三、李四、王五。记录时间 2026-11-03。<br>'
        '备注：该文档为草稿，数字可能不准确，请勿外传。</div></div>'
    )


def text_faq() -> str:
    qa = [
        ("为什么我的 Ctrl+Q 没有反应？", "可能被其他软件占用。打开设置 → 快捷键，检测冲突或改键。"),
        ("本地模型下载很慢怎么办？", "支持断点续传；可在设置里切换镜像源后重新下载。"),
        ("翻译结果格式乱了？", "确认使用的提示词与模型档位匹配：极小档使用精简提示词。"),
        ("截图是黑屏？", "受 DRM 保护或全屏独占窗口无法截取，请使用窗口模式。"),
    ]
    rows = "".join(
        f'<div style="padding:12px 0;border-bottom:1px solid #f1f2f4">'
        f'<div style="font-weight:600;font-size:14px;margin-bottom:4px">Q：{q}</div>'
        f'<div style="font-size:13px;color:#4b5563">A：{a}</div></div>'
        for q, a in qa
    )
    return page(f'<div class="page"><div class="card"><h2 class="h2">常见问题</h2>{rows}</div></div>')


def chart_bar_page() -> str:
    return page(
        '<div class="page"><div class="card">'
        + bar_chart("2025 年各季度销售额（万元）", ["Q1", "Q2", "Q3", "Q4"], [820, 1060, 1340, 1580])
        + "</div></div>"
    )


def chart_line_page() -> str:
    return page(
        '<div class="page"><div class="card">'
        + line_chart(
            "月度活跃用户（万）",
            {"国内": [120, 138, 156, 172, 198, 226], "海外": [40, 46, 55, 63, 78, 92]},
            ["6月", "7月", "8月", "9月", "10月", "11月"],
        )
        + "</div></div>"
    )


def chart_pie_page() -> str:
    return page(
        '<div class="page"><div class="card">'
        + pie_chart("用户设备类型占比", [("Windows", 58), ("macOS", 22), ("Linux", 12), ("其他", 8)])
        + "</div></div>"
    )


def chart_scatter_page() -> str:
    pts = [(10, 32), (22, 45), (35, 52), (41, 61), (58, 66), (63, 74), (77, 71), (85, 88), (94, 92), (28, 38)]
    return page('<div class="page"><div class="card">' + scatter_chart("用户满意度与使用时长", pts) + "</div></div>")


def chart_dashboard_page() -> str:
    kpis = [
        ("今日任务数", "12,846", "+8.2%"),
        ("平均时延", "6.01 s", "-12.4%"),
        ("失败率", "0.42%", "-0.08%"),
        ("Token 消耗", "8.6M", "+3.1%"),
    ]
    return page('<div class="page" style="background:#f7f8fa">' + dashboard(kpis) + "</div>")


def chart_area_page() -> str:
    return page(
        '<div class="page"><div class="card">'
        + area_chart(
            "云端与本地请求量对比",
            ["周一", "周二", "周三", "周四", "周五", "周六", "周日"],
            [[320, 380, 410, 460, 520, 610, 590], [120, 140, 160, 190, 220, 280, 270]],
        )
        + "</div></div>"
    )


def chart_heatmap_page() -> str:
    return page(
        '<div class="page"><div class="card">'
        + heatmap(
            "每小时错误分布（近 7 天）",
            ["周一", "周二", "周三", "周四", "周五", "周六", "周日"],
            ["00", "04", "08", "12", "16", "20"],
            [
                [2, 1, 4, 9, 7, 3],
                [1, 2, 6, 12, 9, 4],
                [3, 1, 5, 10, 8, 2],
                [2, 2, 7, 14, 11, 5],
                [1, 1, 4, 9, 6, 3],
                [4, 2, 3, 6, 5, 8],
                [5, 3, 4, 7, 6, 9],
            ],
        )
        + "</div></div>"
    )


def chart_stacked_page() -> str:
    return page(
        '<div class="page"><div class="card">'
        + stacked_chart(
            "各端请求构成（万次）",
            ["周一", "周二", "周三", "周四", "周五"],
            {"云端": [42, 48, 51, 58, 63], "本地": [12, 15, 17, 19, 22], "浏览器": [8, 9, 10, 11, 13]},
        )
        + "</div></div>"
    )


def _product_card(name: str, price: str, desc: str, tags: List[str], rating: str, color: str) -> str:
    return (
        '<div class="page"><div class="card" style="display:flex;gap:24px">'
        f'<div style="width:300px;height:300px;border-radius:12px;background:linear-gradient(135deg,{color});'
        'display:flex;align-items:center;justify-content:center;color:#fff;font-size:20px;letter-spacing:2px">产品图</div>'
        f'<div style="flex:1"><div style="font-size:22px;font-weight:700;margin-bottom:10px">{name}</div>'
        f'<div style="font-size:13px;color:#6b7280;margin-bottom:14px">{desc}</div>'
        f'<div style="font-size:30px;color:#dc2626;font-weight:700;margin-bottom:8px">{price}</div>'
        f'<div style="font-size:13px;color:#6b7280;margin-bottom:12px">评分 {rating} · 已售 2.3 万+</div>'
        '<div style="display:flex;gap:8px">'
        + "".join(f'<span class="tag">{t}</span>' for t in tags)
        + "</div></div></div></div>"
    )


def product_headphones() -> str:
    return _product_card("无线降噪耳机 Pro", "¥1,299", "主动降噪 · 40 小时续航 · 蓝牙 5.4", ["降噪", "蓝牙", "耳机"], "4.8", "#6366f1,#8b5cf6")


def product_keyboard() -> str:
    return _product_card("机械键盘 K8 87 键", "¥499", "热插拔轴体 · 三模连接 · PBT 键帽", ["键盘", "机械", "87 键"], "4.7", "#0ea5e9,#22d3ee")


def product_band() -> str:
    return _product_card("智能手环 5", "¥269", "血氧监测 · 14 天续航 · 50 米防水", ["手环", "运动", "血氧"], "4.6", "#10b981,#34d399")


def product_coffee() -> str:
    return _product_card("便携咖啡机 Mini", "¥389", "兼容胶囊与咖啡粉 · USB-C 供电", ["咖啡机", "便携", "胶囊"], "4.5", "#b45309,#f59e0b")


def product_airfryer() -> str:
    return _product_card("空气炸锅 4L 家用", "¥359", "可视窗口 · 8 种预设 · 不粘涂层", ["空气炸锅", "厨房", "4L"], "4.4", "#ef4444,#f97316")


def product_lamp() -> str:
    return _product_card("学习护眼台灯", "¥199", "无频闪 · 国 AA 级照度 · 三档色温", ["台灯", "护眼", "学习"], "4.9", "#8b5cf6,#ec4899")


def product_fictional() -> str:
    """虚构产品（难）：检索很可能没有可靠来源，模型不应编造参数，应标注不确定。"""
    return _product_card(
        "Nebula X9 智能音箱（工程样机）",
        "价格待定",
        "样机编号 NX9-2601，参数未公开；仅用于内部评审，未上市。",
        ["智能音箱", "Nebula X9", "工程样机"],
        "无",
        "#334155,#64748b",
    )


def code_python() -> str:
    code = (
        "def binary_search(nums, target):\n"
        '    """在有序数组中查找 target，返回下标或 -1。"""\n'
        "    lo, hi = 0, len(nums) - 1\n"
        "    while lo <= hi:\n"
        "        mid = (lo + hi) // 2\n"
        "        if nums[mid] == target:\n"
        "            return mid\n"
        "        if nums[mid] < target:\n"
        "            lo = mid + 1\n"
        "        else:\n"
        "            hi = mid - 1\n"
        "    return -1\n\n"
        "print(binary_search([1, 3, 5, 7, 9], 7))"
    )
    return page(
        '<div class="page"><div class="card" style="padding:0;overflow:hidden">'
        '<div style="background:#1e293b;color:#e2e8f0;padding:12px 16px;font-size:13px">editor · binary_search.py</div>'
        f'<pre style="margin:0;padding:16px 20px;font-family:Consolas,monospace;font-size:13px;line-height:1.55;color:#0f172a">{code}</pre>'
        "</div></div>"
    )


def code_javascript() -> str:
    code = (
        "const debounce = (fn, wait = 300) => {\n"
        "  let timer = null;\n"
        "  return (...args) => {\n"
        "    clearTimeout(timer);\n"
        "    timer = setTimeout(() => fn(...args), wait);\n"
        "  };\n"
        "};\n\n"
        "document.addEventListener('scroll', debounce(() => {\n"
        "  console.log('scrolled', window.scrollY);\n"
        "}));"
    )
    return page(
        '<div class="page"><div class="card" style="padding:0;overflow:hidden">'
        '<div style="background:#0f172a;color:#cbd5e1;padding:12px 16px;font-size:13px">VS Code · debounce.js</div>'
        f'<pre style="margin:0;padding:16px 20px;font-family:Consolas,monospace;font-size:13px;line-height:1.6;color:#1f2937">{code}</pre>'
        "</div></div>"
    )


def code_terminal() -> str:
    lines = [
        "$ cargo test --all",
        "   Compiling snip-ask v0.1.0",
        "    Finished test [unoptimized] target(s) in 42.18s",
        "     Running unittests src\\lib.rs",
        "running 148 tests",
        "test services::translator::tests::glossary_rendering ... ok",
        "test services::prompt_engine::tests::render_replaces_known_variables ... ok",
        "test result: ok. 148 passed; 0 failed; 0 ignored; 0 measured",
        "",
        "$ pnpm test",
        "Test Files  12 passed (12)",
        "     Tests  96 passed (96)",
    ]
    body = "\n".join(lines)
    return page(
        '<div class="page" style="background:#0b1020">'
        '<div style="background:#111827;border:1px solid #1f2937;border-radius:10px;padding:18px 20px;height:100%">'
        '<div style="color:#94a3b8;font-family:Consolas,monospace;font-size:13px;line-height:1.75;white-space:pre">'
        + body.replace("&", "&amp;").replace("<", "&lt;")
        + "</div></div></div>"
    )


def code_json() -> str:
    body = (
        '{\n'
        '  "models": {\n'
        '    "translate": "qwen2.5-0.5b-instruct-q4",\n'
        '    "vision": "deepseek-flash"\n'
        "  },\n"
        '  "limits": { "maxTokens": 2048, "timeoutMs": 8000 },\n'
        '  "features": ["screenshot", "translate", "image_search"],\n'
        '  "version": "v1.2.3"\n'
        "}"
    )
    return page(
        '<div class="page"><div class="card" style="padding:0;overflow:hidden">'
        '<div style="background:#f8fafc;border-bottom:1px solid #e5e7eb;padding:12px 16px;font-size:13px">config.json</div>'
        f'<pre style="margin:0;padding:16px 20px;font-family:Consolas,monospace;font-size:13px;line-height:1.6;color:#0f172a">{body}</pre>'
        "</div></div>"
    )


def code_diff() -> str:
    rows = [
        ("-", "fn placeholder(text: &str) -> String { format!(\"【骨架模式译文】{text}\") }", "#fee2e2", "#b91c1c"),
        ("+", "async fn translate(&self, request: &TranslateRequest) -> AppResult<String> {", "#dcfce7", "#15803d"),
        ("+", "    let prompt = self.prompt_engine.render(\"translate\", &vars)?;", "#dcfce7", "#15803d"),
        ("+", "    self.gateway.stream(&request, sink).await", "#dcfce7", "#15803d"),
        (" ", "}", "#f8fafc", "#374151"),
    ]
    body = "".join(
        f'<div style="background:{bg};color:{fg};font-family:Consolas,monospace;font-size:13px;'
        f'padding:6px 14px;white-space:pre">{sign} {text}</div>'
        for sign, text, bg, fg in rows
    )
    return page(
        '<div class="page"><div class="card" style="padding:0;overflow:hidden">'
        '<div style="background:#f1f5f9;padding:12px 16px;font-size:13px">git diff · translator.rs</div>'
        + body
        + "</div></div>"
    )


def code_sql() -> str:
    body = (
        "SELECT t.action,\n"
        "       COUNT(*) AS total,\n"
        "       ROUND(AVG(t.total_ms), 1) AS avg_ms,\n"
        "       SUM(CASE WHEN t.status = 'failed' THEN 1 ELSE 0 END) AS failed\n"
        "FROM tasks t\n"
        "WHERE t.created_at >= strftime('%s', 'now', '-7 days')\n"
        "GROUP BY t.action\n"
        "ORDER BY total DESC;"
    )
    return page(
        '<div class="page"><div class="card" style="padding:0;overflow:hidden">'
        '<div style="background:#f8fafc;border-bottom:1px solid #e5e7eb;padding:12px 16px;font-size:13px">history.db · 查询任务统计</div>'
        f'<pre style="margin:0;padding:16px 20px;font-family:Consolas,monospace;font-size:13px;line-height:1.6;color:#0f172a">{body}</pre>'
        "</div></div>"
    )


def code_minified() -> str:
    """压缩混淆 JS（难）：缺少上下文，识别/检索应标注不确定。"""
    body = (
        "!function(e,t){\"object\"==typeof exports&&\"object\"==typeof module?"
        "module.exports=t():\"function\"==typeof define&&define.amd?define([],t):"
        "\"object\"==typeof exports?exports[e]=t():t()}(\"snipask\",function(){"
        "return function(e){var t={};function n(r){if(t[r])return t[r].exports;"
        "var o=t[r]={i:r,l:!1,exports:{}};return e[r].call(o.exports,o,o.exports,n),"
        "o.l=!0,o.exports}return n.m=e,n.c=t,n.d=function(e,t,r){n.o(e,t)||"
        "Object.defineProperty(e,t,{enumerable:!0,get:r})},n.r=function(e){"
        "\"undefined\"!=typeof Symbol&&Symbol.toStringTag&&Object.defineProperty(e,"
        "Symbol.toStringTag,{value:\"Module\"}),Object.defineProperty(e,\"__esModule\",{value:!0})},n.n=function(e){var t=e&&e.__esModule?"
        "function(){return e.default}:function(){return e};return n.d(t,\"a\",t),t},n.o=function(e,t){"
        "return Object.prototype.hasOwnProperty.call(e,t)},n.p=\"\",n(n.s=0)}([function(e,t){e.exports=1}])});"
    )
    return page(
        '<div class="page"><div class="card" style="padding:0;overflow:hidden">'
        '<div style="background:#0f172a;color:#cbd5e1;padding:12px 16px;font-size:13px">bundle.min.js</div>'
        f'<pre style="margin:0;padding:16px 20px;font-family:Consolas,monospace;font-size:11px;line-height:1.5;color:#1f2937;white-space:pre-wrap;word-break:break-all">{body}</pre>'
        "</div></div>"
    )


def code_stacktrace() -> str:
    lines = [
        "Unhandled exception. System.NullReferenceException:",
        "Object reference not set to an instance of an object.",
        "   at SnipAsk.Services.Translator.TranslateAsync(TranslateRequest req) in translator.rs:line 82",
        "   at SnipAsk.Commands.ActionCommand.ExecuteAsync(ActionRequest request) in action.rs:line 147",
        "   at SnipAsk.App.RunAsync() in main.rs:line 63",
        "--- End of stack trace ---",
        "warn: retry 1/2 after 500ms (SA-5001)",
        "error: translation failed after 2 attempts",
    ]
    body = "\n".join(lines)
    return page(
        '<div class="page" style="background:#0b1020">'
        '<div style="background:#111827;border:1px solid #1f2937;border-radius:10px;padding:18px 20px;height:100%">'
        '<div style="color:#fca5a5;font-family:Consolas,monospace;font-size:12.5px;line-height:1.7;white-space:pre">'
        + body.replace("&", "&amp;").replace("<", "&lt;")
        + "</div></div></div>"
    )


# --------------------------------------------------------------------------- #
# 语料清单
# --------------------------------------------------------------------------- #
def build_manifest() -> List[Dict[str, Any]]:
    items: List[Dict[str, Any]] = []

    def add(item_id: str, category: str, name: str, html: str, meta: Dict[str, Any]) -> None:
        items.append(
            {
                "id": item_id,
                "category": category,
                "name": name,
                "html": html,
                "difficulty": meta.get("difficulty", "easy"),
                "expect_uncertain": meta.get("expect_uncertain", False),
                "expected_query_terms": meta.get("expected_query_terms", []),
                "recognizable_content": meta.get("recognizable_content", []),
                "ground_truth": meta.get("ground_truth", ""),
            }
        )

    # ---- 文字类 8 ----
    add("text-01-article", "text", "新闻文章阅读页", text_article(), {
        "difficulty": "easy",
        "expected_query_terms": ["AI", "编程助手", "本地"],
        "recognizable_content": ["国产 AI 编程助手", "本地优先", "2026"],
        "ground_truth": "一篇科技新闻：国产 AI 编程助手进入本地优先阶段，含 Q3 使用量同比增长 210%、代码补全占比 46%。",
    })
    add("text-02-chat", "text", "项目群聊天记录", text_chat(), {
        "difficulty": "easy",
        "expected_query_terms": ["SA-1001", "截图失败", "错误码"],
        "recognizable_content": ["接口文档", "SA-1001", "可重试", "李工"],
        "ground_truth": "群聊记录：讨论周五前发接口文档，并确认 SA-1001 截图失败错误码可重试。",
    })
    add("text-03-settings", "text", "设置面板", text_settings(), {
        "difficulty": "easy",
        "expected_query_terms": ["设置", "deepseek-flash", "Qwen2.5"],
        "recognizable_content": ["AI 服务", "deepseek-flash", "Qwen2.5-0.5B", "Ctrl+Q"],
        "ground_truth": "设置页：AI 服务 DeepSeek/deepseek-flash 已连接，本地模型 Qwen2.5-0.5B 已下载，快捷键 Ctrl+Q。",
    })
    add("text-04-error-dialog", "text", "错误弹窗", text_error_dialog(), {
        "difficulty": "easy",
        "expected_query_terms": ["SA-1001", "截图失败", "DRM"],
        "recognizable_content": ["截图失败", "SA-1001", "DRM", "重试"],
        "ground_truth": "错误弹窗：截图失败（SA-1001），可能因 DRM 或全屏独占，可重试或使用窗口模式。",
    })
    add("text-05-table", "text", "本地模型对比表格", text_table(), {
        "difficulty": "medium",
        "expected_query_terms": ["Qwen2.5", "HY-MT1.5", "P95 时延"],
        "recognizable_content": ["Qwen2.5-0.5B", "0.55 GiB", "3.21 s", "HY-MT1.5"],
        "ground_truth": "表格对比 4 个模型的参数量、常驻内存、P95 时延与适用场景。",
    })
    add("text-06-bilingual", "text", "中英混排更新说明", text_bilingual(), {
        "difficulty": "medium",
        "expected_query_terms": ["Release Notes", "v1.2.3", "thinking"],
        "recognizable_content": ["Release Notes v1.2.3", "Qwen2.5-0.5B", "thinking", "Ctrl+Q"],
        "ground_truth": "Release Notes v1.2.3：新增本地极小模型、修复热键冲突、默认关闭 thinking，4K/200% 未验证。",
    })
    add("text-07-blurry", "text", "低对比度小字会议纪要", text_blurry(), {
        "difficulty": "hard",
        "expect_uncertain": True,
        "expected_query_terms": ["OCR", "像素预算", "混合 DPI", "降档"],
        "recognizable_content": ["像素预算", "混合 DPI", "降档策略"],
        "ground_truth": "低对比度 9px 灰色小字会议纪要：讨论 OCR 像素预算、混合 DPI 坐标、本地模型降档，文字模糊应标注识别不确定。",
    })
    add("text-08-faq", "text", "常见问题列表", text_faq(), {
        "difficulty": "easy",
        "expected_query_terms": ["Ctrl+Q", "快捷键", "断点续传", "黑屏"],
        "recognizable_content": ["Ctrl+Q 没有反应", "断点续传", "黑屏", "DRM"],
        "ground_truth": "FAQ：Ctrl+Q 无反应、本地模型下载慢、翻译格式乱、截图黑屏四个问题与解答。",
    })

    # ---- 图表类 8 ----
    add("chart-01-bar", "chart", "季度销售额柱状图", chart_bar_page(), {
        "difficulty": "medium",
        "expected_query_terms": ["销售额", "季度", "柱状图"],
        "recognizable_content": ["2025 年各季度销售额", "Q1", "1580"],
        "ground_truth": "柱状图：2025 年 Q1-Q4 销售额 820/1060/1340/1580 万元，逐季上升。",
    })
    add("chart-02-line", "chart", "月度活跃用户折线图", chart_line_page(), {
        "difficulty": "medium",
        "expected_query_terms": ["活跃用户", "月度", "折线图"],
        "recognizable_content": ["月度活跃用户", "国内", "海外", "11月"],
        "ground_truth": "双折线：国内活跃用户 6-11 月 120→226 万，海外 40→92 万，国内增速更高。",
    })
    add("chart-03-pie", "chart", "设备类型饼图", chart_pie_page(), {
        "difficulty": "easy",
        "expected_query_terms": ["设备类型", "占比", "Windows"],
        "recognizable_content": ["用户设备类型占比", "Windows", "58%"],
        "ground_truth": "饼图：Windows 58%、macOS 22%、Linux 12%、其他 8%。",
    })
    add("chart-04-scatter", "chart", "满意度散点图", chart_scatter_page(), {
        "difficulty": "medium",
        "expected_query_terms": ["满意度", "使用时长", "散点图"],
        "recognizable_content": ["用户满意度与使用时长", "满意度", "小时"],
        "ground_truth": "散点图：满意度随使用时长上升的正相关趋势。",
    })
    add("chart-05-dashboard", "chart", "运维 KPI 仪表盘", chart_dashboard_page(), {
        "difficulty": "medium",
        "expected_query_terms": ["任务数", "平均时延", "失败率"],
        "recognizable_content": ["今日任务数", "12,846", "平均时延", "6.01 s", "失败率"],
        "ground_truth": "KPI 仪表盘 4 卡片：今日任务 12,846（+8.2%）、平均时延 6.01s（-12.4%）、失败率 0.42%、Token 8.6M。",
    })
    add("chart-06-area", "chart", "请求量面积图", chart_area_page(), {
        "difficulty": "medium",
        "expected_query_terms": ["请求量", "云端", "本地", "面积图"],
        "recognizable_content": ["云端与本地请求量对比", "周一", "周日"],
        "ground_truth": "面积图：云端请求量高于本地，周末两边都上升。",
    })
    add("chart-07-heatmap", "chart", "错误分布热力图（小图例）", chart_heatmap_page(), {
        "difficulty": "hard",
        "expect_uncertain": True,
        "expected_query_terms": ["错误分布", "热力图", "时段"],
        "recognizable_content": ["每小时错误分布", "12", "20"],
        "ground_truth": "热力图：近 7 天按小时错误分布，12/16 点错误最多；图例与坐标字号很小，应标注不确定。",
    })
    add("chart-08-stacked", "chart", "请求构成堆叠柱状图", chart_stacked_page(), {
        "difficulty": "medium",
        "expected_query_terms": ["请求构成", "云端", "浏览器"],
        "recognizable_content": ["各端请求构成", "云端", "本地", "浏览器"],
        "ground_truth": "堆叠柱状图：周一至周五云端/本地/浏览器请求构成，云端占比最高。",
    })

    # ---- 商品类 7 ----
    add("product-01-headphones", "product", "无线降噪耳机商品卡", product_headphones(), {
        "difficulty": "easy",
        "expected_query_terms": ["降噪耳机", "无线", "价格"],
        "recognizable_content": ["无线降噪耳机 Pro", "¥1,299", "降噪", "蓝牙"],
        "ground_truth": "商品卡：无线降噪耳机 Pro，¥1299，主动降噪/40 小时续航/蓝牙 5.4，评分 4.8。",
    })
    add("product-02-keyboard", "product", "机械键盘商品卡", product_keyboard(), {
        "difficulty": "easy",
        "expected_query_terms": ["机械键盘", "87 键", "热插拔"],
        "recognizable_content": ["机械键盘 K8", "¥499", "热插拔", "PBT"],
        "ground_truth": "商品卡：机械键盘 K8 87 键，¥499，热插拔轴体/三模/PBT 键帽。",
    })
    add("product-03-band", "product", "智能手环商品卡", product_band(), {
        "difficulty": "easy",
        "expected_query_terms": ["智能手环", "血氧", "续航"],
        "recognizable_content": ["智能手环 5", "¥269", "血氧", "防水"],
        "ground_truth": "商品卡：智能手环 5，¥269，血氧监测/14 天续航/50 米防水。",
    })
    add("product-04-coffee", "product", "便携咖啡机商品卡", product_coffee(), {
        "difficulty": "medium",
        "expected_query_terms": ["便携咖啡机", "胶囊", "USB-C"],
        "recognizable_content": ["便携咖啡机 Mini", "¥389", "胶囊", "USB-C"],
        "ground_truth": "商品卡：便携咖啡机 Mini，¥389，兼容胶囊与咖啡粉、USB-C 供电。",
    })
    add("product-05-airfryer", "product", "空气炸锅商品卡", product_airfryer(), {
        "difficulty": "easy",
        "expected_query_terms": ["空气炸锅", "4L", "预设"],
        "recognizable_content": ["空气炸锅 4L", "¥359", "可视窗口", "8 种预设"],
        "ground_truth": "商品卡：空气炸锅 4L 家用，¥359，可视窗口/8 种预设/不粘涂层。",
    })
    add("product-06-lamp", "product", "护眼台灯商品卡", product_lamp(), {
        "difficulty": "easy",
        "expected_query_terms": ["护眼台灯", "无频闪", "色温"],
        "recognizable_content": ["学习护眼台灯", "¥199", "无频闪", "国 AA 级"],
        "ground_truth": "商品卡：学习护眼台灯，¥199，无频闪/国 AA 照度/三档色温。",
    })
    add("product-07-fictional", "product", "虚构产品 Nebula X9（工程样机）", product_fictional(), {
        "difficulty": "hard",
        "expect_uncertain": True,
        "expected_query_terms": ["Nebula X9", "智能音箱", "工程样机"],
        "recognizable_content": ["Nebula X9", "工程样机", "NX9-2601", "参数未公开"],
        "ground_truth": "虚构产品卡：Nebula X9 智能音箱工程样机，价格待定、参数未公开；检索很可能无可靠来源，模型不应编造参数，应标注不确定。",
    })

    # ---- 代码类 7 ----
    add("code-01-python", "code", "Python 二分查找代码", code_python(), {
        "difficulty": "easy",
        "expected_query_terms": ["Python", "二分查找", "binary_search"],
        "recognizable_content": ["binary_search", "def binary_search", "while lo <= hi", "print"],
        "ground_truth": "Python 代码：二分查找实现，含 docstring、while 循环与 print 调用。",
    })
    add("code-02-javascript", "code", "JS 防抖函数代码", code_javascript(), {
        "difficulty": "easy",
        "expected_query_terms": ["JavaScript", "debounce", "防抖"],
        "recognizable_content": ["const debounce", "setTimeout", "addEventListener"],
        "ground_truth": "JavaScript 代码：debounce 防抖函数与 scroll 事件监听。",
    })
    add("code-03-terminal", "code", "终端测试输出", code_terminal(), {
        "difficulty": "medium",
        "expected_query_terms": ["cargo test", "测试通过", "148 tests"],
        "recognizable_content": ["cargo test", "148 tests", "pnpm test", "96 passed"],
        "ground_truth": "终端输出：cargo test 148 项通过、pnpm test 96 项通过。",
    })
    add("code-04-json", "code", "JSON 配置片段", code_json(), {
        "difficulty": "easy",
        "expected_query_terms": ["JSON", "config", "maxTokens"],
        "recognizable_content": ["config.json", "deepseek-flash", "maxTokens", "v1.2.3"],
        "ground_truth": "JSON 配置：模型 translate/vision、limits、features 与版本 v1.2.3。",
    })
    add("code-05-diff", "code", "git diff 片段", code_diff(), {
        "difficulty": "medium",
        "expected_query_terms": ["git diff", "Rust", "translator"],
        "recognizable_content": ["git diff", "translator.rs", "TranslateRequest", "prompt_engine"],
        "ground_truth": "git diff：translator.rs 将占位实现替换为真实 translate 调用。",
    })
    add("code-06-sql", "code", "SQL 统计查询", code_sql(), {
        "difficulty": "medium",
        "expected_query_terms": ["SQL", "GROUP BY", "任务统计"],
        "recognizable_content": ["SELECT", "COUNT(*)", "GROUP BY", "history.db"],
        "ground_truth": "SQL：按 action 统计任务总数、平均耗时与失败数，近 7 天。",
    })
    add("code-07-minified", "code", "压缩混淆 JS（无上下文）", code_minified(), {
        "difficulty": "hard",
        "expect_uncertain": True,
        "expected_query_terms": ["minified", "JavaScript", "webpack"],
        "recognizable_content": ["bundle.min.js", "exports", "define.amd"],
        "ground_truth": "压缩混淆的 webpack UMD 打包代码，11px 小字；无上下文，识别与检索都应标注不确定。",
    })

    # 分类数量校验（防止手改漂移）
    for category, expected in CATEGORY_COUNTS.items():
        actual = sum(1 for it in items if it["category"] == category)
        assert actual == expected, f"{category} 分类数量应为 {expected}，实际 {actual}"
    return items


# --------------------------------------------------------------------------- #
# 渲染与截图
# --------------------------------------------------------------------------- #
def find_edge() -> str:
    for candidate in EDGE_CANDIDATES:
        if Path(candidate).is_file():
            return candidate
    raise RuntimeError("未找到 msedge.exe（尝试了 Program Files (x86) 与 Program Files）")


def png_size(path: Path) -> Optional[Tuple[int, int]]:
    try:
        data = path.read_bytes()[:32]
    except OSError:
        return None
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    width, height = struct.unpack(">II", data[16:24])
    return width, height


def shoot(edge: str, html_path: Path, png_path: Path, profile_dir: Path, timeout: float = 90.0) -> Dict[str, Any]:
    import tempfile

    png_path.parent.mkdir(parents=True, exist_ok=True)
    if png_path.exists():
        png_path.unlink()
    # Edge 的 --user-data-dir 用 ASCII 临时目录，避免中文路径 + 并发 profile 锁问题
    profile_dir.mkdir(parents=True, exist_ok=True)
    args = [
        edge,
        "--headless=new",
        "--disable-gpu",
        "--hide-scrollbars",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        "--force-device-scale-factor=1",
        f"--user-data-dir={profile_dir}",
        f"--screenshot={png_path}",
        f"--window-size={VIEW_W},{VIEW_H}",
        html_path.resolve().as_uri(),
    ]
    t0 = time.perf_counter()
    last_err = ""
    for _ in range(2):
        try:
            proc = subprocess.run(
                args,
                capture_output=True,
                timeout=timeout,
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            )
            # Edge 在中文 Windows 上会输出 GBK/UTF-8 混合日志：按字节读取后宽松解码，避免解码异常
            last_err = (proc.stderr or b"").decode("utf-8", "replace")[-500:]
        except subprocess.TimeoutExpired:
            last_err = "timeout"
        if png_path.exists() and png_path.stat().st_size > 1000:
            size = png_size(png_path)
            return {
                "ok": True,
                "elapsed_ms": round((time.perf_counter() - t0) * 1000.0, 1),
                "bytes": png_path.stat().st_size,
                "png_size": list(size) if size else None,
            }
    return {"ok": False, "elapsed_ms": round((time.perf_counter() - t0) * 1000.0, 1), "error": last_err}


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description="S-12 30 张评测语料生成器（headless Edge 截图）")
    ap.add_argument("--no-shots", action="store_true", help="只生成 HTML 与清单")
    ap.add_argument("--out-json", default=str(CORPUS_DIR / "corpus.json"))
    args = ap.parse_args(argv)

    HTML_DIR.mkdir(parents=True, exist_ok=True)
    SHOT_DIR.mkdir(parents=True, exist_ok=True)
    items = build_manifest()
    edge = "" if args.no_shots else find_edge()
    tmp_root = Path(os.environ.get("TEMP", str(CORPUS_DIR / "tmp"))) / "s12_edge_profiles"

    records: List[Dict[str, Any]] = []
    failures = 0
    for item in items:
        html_path = HTML_DIR / f"{item['id']}.html"
        html_path.write_text(item["html"], encoding="utf-8")
        png_path = SHOT_DIR / f"{item['id']}.png"
        shot: Dict[str, Any] = {"ok": False, "skipped": args.no_shots}
        if not args.no_shots:
            shot = shoot(edge, html_path, png_path, tmp_root / item["id"])
            if not shot["ok"]:
                failures += 1
        record = {
            k: v for k, v in item.items() if k != "html"
        }
        record.update(
            {
                "html": str(html_path.relative_to(REPO_ROOT)).replace("\\", "/"),
                "png": str(png_path.relative_to(REPO_ROOT)).replace("\\", "/"),
                "png_bytes": png_path.stat().st_size if png_path.exists() else None,
                "png_size": list(png_size(png_path) or ()),
                "shot": shot,
            }
        )
        records.append(record)
        status = "SKIP" if args.no_shots else ("OK" if shot["ok"] else "FAIL")
        print(f"[{status}] {item['id']:<24} {item['category']:<8} {item['name']}")

    manifest = {
        "purpose": "S-12 图片搜索可信度评测语料（真实 headless Edge 渲染截图）",
        "generated_at": __import__("datetime").datetime.now(
            __import__("datetime").timezone(__import__("datetime").timedelta(hours=8))
        ).isoformat(),
        "view": {"width": VIEW_W, "height": VIEW_H, "device_scale_factor": 1},
        "counts": {
            "total": len(records),
            **{cat: sum(1 for r in records if r["category"] == cat) for cat in CATEGORY_COUNTS},
            "expect_uncertain": sum(1 for r in records if r["expect_uncertain"]),
        },
        "edge": edge,
        "items": records,
    }
    out_path = Path(args.out_json)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    print("-" * 78)
    print(f"语料：{len(records)} 张；截图失败 {failures}；清单 {out_path}")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    if hasattr(sys.stdout, "reconfigure"):
        try:
            sys.stdout.reconfigure(encoding="utf-8")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
