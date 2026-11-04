#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成「框选 AI」的 Tauri 打包图标。

用法（在仓库根目录执行）：
    python scripts/gen-icons.py

产物（src-tauri/icons/）：
    32x32.png · 128x128.png · 128x128@2x.png（256）· icon.png（512）· icon.ico（多尺寸）

设计：圆角矩形渐变底（深蓝 → 靛蓝）+ 白色「选框四角 + 中心 AI 星芒」。
为保证边缘平滑，所有几何图形先按 4 倍超采样绘制，再 LANCZOS 缩小。
依赖：Python 3.11+、Pillow（脚本仅使用 Pillow + numpy 计算渐变）。
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw
except ImportError:  # pragma: no cover - 环境缺失时给出可操作提示
    sys.exit("缺少 Pillow，请先执行：python -m pip install pillow")

try:
    import numpy as np
except ImportError:  # pragma: no cover - numpy 仅用于加速渐变，缺失时走纯 Python 分支
    np = None

# ---------------------------------------------------------------- 基本参数

ROOT = Path(__file__).resolve().parent.parent
ICON_DIR = ROOT / "src-tauri" / "icons"

MASTER = 1024          # 主图边长（所有尺寸由它缩放）
SS = 4                 # 超采样倍数（抗锯齿）
RADIUS = 184           # 圆角半径（主图像素）

COLOR_START = (27, 58, 139)     # 左上：深蓝 #1B3A8B
COLOR_END = (79, 70, 229)       # 右下：靛蓝 #4F46E5
MARK_COLOR = (255, 255, 255)    # 选框与星芒：纯白

# 选框四角：向内缩进、单臂长、线宽
CORNER_MARGIN = 196
CORNER_ARM = 156
CORNER_WIDTH = 40
CORNER_RADIUS = 18

# 中心 AI 星芒（四角星）半径与内凹比例
SPARK_RADIUS = 176
SPARK_INNER = 0.17

PNG_TARGETS = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 512,
}
ICO_SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]


# ---------------------------------------------------------------- 绘制辅助

def diagonal_gradient(size: int) -> Image.Image:
    """生成左上深蓝 → 右下靛蓝的对角线性渐变（RGB）。"""
    if np is not None:
        # 用一行一列的插值矩阵合成对角渐变，避免 Python 逐像素循环
        ramp = np.linspace(0.0, 1.0, size, dtype=np.float32)
        t = (ramp[:, None] + ramp[None, :]) / 2.0
        t = np.clip(t, 0.0, 1.0)[..., None]
        start = np.array(COLOR_START, dtype=np.float32)
        end = np.array(COLOR_END, dtype=np.float32)
        rgb = (start + (end - start) * t).astype(np.uint8)
        return Image.fromarray(rgb, mode="RGB")

    # 无 numpy 时的纯 Python 兜底（较慢但结果一致）
    img = Image.new("RGB", (size, size))
    pixels = []
    for y in range(size):
        for x in range(size):
            t = (x + y) / (2.0 * (size - 1))
            pixels.append(tuple(round(COLOR_START[i] + (COLOR_END[i] - COLOR_START[i]) * t) for i in range(3)))
    img.putdata(pixels)
    return img


def rounded_mask(size: int, radius: int, ss: int) -> Image.Image:
    """圆角矩形蒙版（超采样后缩小，得到平滑圆角）。"""
    big = Image.new("L", (size * ss, size * ss), 0)
    draw = ImageDraw.Draw(big)
    draw.rounded_rectangle(
        [0, 0, size * ss - 1, size * ss - 1],
        radius=radius * ss,
        fill=255,
    )
    return big.resize((size, size), Image.LANCZOS)


def draw_corners(draw: ImageDraw.ImageDraw, scale: int) -> None:
    """绘制选框四角（四条 L 形短线，用圆角矩形实现）。"""
    m = CORNER_MARGIN * scale
    arm = CORNER_ARM * scale
    w = CORNER_WIDTH * scale
    r = CORNER_RADIUS * scale
    edge = MASTER * scale - 1 - m

    # 左上
    draw.rounded_rectangle([m, m, m + arm, m + w], radius=r, fill=MARK_COLOR)
    draw.rounded_rectangle([m, m, m + w, m + arm], radius=r, fill=MARK_COLOR)
    # 右上
    draw.rounded_rectangle([edge - arm, m, edge, m + w], radius=r, fill=MARK_COLOR)
    draw.rounded_rectangle([edge - w, m, edge, m + arm], radius=r, fill=MARK_COLOR)
    # 左下
    draw.rounded_rectangle([m, edge - w, m + arm, edge], radius=r, fill=MARK_COLOR)
    draw.rounded_rectangle([m, edge - arm, m + w, edge], radius=r, fill=MARK_COLOR)
    # 右下
    draw.rounded_rectangle([edge - arm, edge - w, edge, edge], radius=r, fill=MARK_COLOR)
    draw.rounded_rectangle([edge - w, edge - arm, edge, edge], radius=r, fill=MARK_COLOR)


def draw_spark(draw: ImageDraw.ImageDraw, scale: int) -> None:
    """绘制中心四角星芒（AI 生成感），8 个顶点即四角星。"""
    center = (MASTER * scale) / 2.0
    outer = SPARK_RADIUS * scale
    inner = outer * SPARK_INNER
    points = [
        (center, center - outer),
        (center + inner, center - inner),
        (center + outer, center),
        (center + inner, center + inner),
        (center, center + outer),
        (center - inner, center + inner),
        (center - outer, center),
        (center - inner, center - inner),
    ]
    draw.polygon(points, fill=MARK_COLOR)


def render_master() -> Image.Image:
    """合成 1024×1024 主图（RGBA）。"""
    # 1. 渐变底 + 圆角蒙版
    base = diagonal_gradient(MASTER).convert("RGBA")
    base.putalpha(rounded_mask(MASTER, RADIUS, SS))

    # 2. 白色图形层：4 倍超采样绘制后缩小
    big = MASTER * SS
    layer = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    draw_corners(draw, SS)
    draw_spark(draw, SS)
    layer = layer.resize((MASTER, MASTER), Image.LANCZOS)

    # 3. 叠加到渐变底（白色图形自然裁掉圆角外区域之外的部分由底图 alpha 决定）
    return Image.alpha_composite(base, layer)


# ---------------------------------------------------------------- 导出

def export_assets(master: Image.Image) -> list[Path]:
    """把主图导出为各尺寸 PNG 与多尺寸 ICO，返回生成的文件列表。"""
    ICON_DIR.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []

    for name, size in PNG_TARGETS.items():
        target = ICON_DIR / name
        resized = master.resize((size, size), Image.LANCZOS)
        resized.save(target, format="PNG", optimize=True)
        written.append(target)

    ico_path = ICON_DIR / "icon.ico"
    # Pillow 会按 sizes 列表逐尺寸用 LANCZOS 缩略后写入 ICO
    master.save(ico_path, format="ICO", sizes=ICO_SIZES)
    written.append(ico_path)

    return written


def verify(written: list[Path]) -> None:
    """回读产物，校验格式与尺寸是否符合预期。"""
    problems: list[str] = []

    for name, size in PNG_TARGETS.items():
        path = ICON_DIR / name
        with Image.open(path) as img:
            if img.format != "PNG":
                problems.append(f"{name}: 格式为 {img.format}，期望 PNG")
            if img.size != (size, size):
                problems.append(f"{name}: 尺寸 {img.size}，期望 {(size, size)}")
            if img.mode != "RGBA":
                problems.append(f"{name}: 颜色模式 {img.mode}，期望 RGBA")

    with Image.open(ICON_DIR / "icon.ico") as ico:
        if ico.format != "ICO":
            problems.append(f"icon.ico: 格式为 {ico.format}，期望 ICO")
        sizes = sorted(set(ico.info.get("sizes", [])))
        expected = sorted(set(ICO_SIZES))
        if sizes != expected:
            problems.append(f"icon.ico: 内含尺寸 {sizes}，期望 {expected}")

    if problems:
        print("图标校验失败：")
        for item in problems:
            print("  - " + item)
        raise SystemExit(1)

    print("图标校验通过：")
    for path in written:
        with Image.open(path) as img:
            print(f"  - {path.relative_to(ROOT)}  {img.format}  {img.size[0]}x{img.size[1]}")


# ---------------------------------------------------------------- 入口

def main() -> None:
    print(f"仓库根目录：{ROOT}")
    master = render_master()
    written = export_assets(master)
    verify(written)
    print(f"完成，共生成 {len(written)} 个文件 → {ICON_DIR.relative_to(ROOT)}/")


if __name__ == "__main__":
    main()
