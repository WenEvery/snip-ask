# -*- coding: utf-8 -*-
"""S-02/S-03 输入数据构造脚本。

从真实桌面截图（scripts/spikes/tools/screenshot.ps1 输出，虚拟桌面物理像素 3640x1920）
构造 OCR 基准所需的输入：

  1. screen_primary_2560x1440.png  主屏真实桌面裁剪（2560x1440@150% 的物理像素）
  2. screen_4k_3840x2160.png       主屏内容按 1.5x LANCZOS 放大到 4K（单屏 4K 对照）
  3. dual_4k_7680x2160.png         两块 4K 屏横向拼接（左=主屏 4K 放大，右=同一内容），
                                   代表「4K 双屏冻结帧」的物理像素尺寸
  4. sel_*.png                     从 4K 单屏内容裁出的典型选区（800x600 / 1200x800，疏密各异）

所有构造方法、尺寸、sha256 写入 inputs_manifest.json，便于复跑与审计。
用法：
    python scripts/spikes/s02_s03/make_inputs.py            # 默认读取 .verify/spikes/S-02-S-03/inputs/desktop_full.png
    python scripts/spikes/s02_s03/make_inputs.py --shot X   # 指定其它截图
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image

REPO = Path(__file__).resolve().parents[3]
DEFAULT_DIR = REPO / ".verify" / "spikes" / "S-02-S-03" / "inputs"

# 虚实桌面元数据（由 screenshot.ps1 / GetSystemMetrics 得到）
VIRTUAL_ORIGIN = (-1080, -470)  # 虚拟桌面左上角物理像素
PRIMARY_RECT = (0, 0, 2560, 1440)  # 主屏物理像素矩形（x, y, w, h）
SECONDARY_RECT = (-1080, -470, 1080, 1920)  # 副屏竖屏物理像素矩形


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def crop_physical(img: Image.Image, rect) -> Image.Image:
    """按虚拟桌面物理像素矩形裁剪。"""
    x, y, w, h = rect
    ox, oy = VIRTUAL_ORIGIN
    left = x - ox
    top = y - oy
    return img.crop((left, top, left + w, top + h))


def upscale(img: Image.Image, w: int, h: int) -> Image.Image:
    return img.resize((w, h), Image.LANCZOS)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--shot", default=str(DEFAULT_DIR / "desktop_full.png"))
    ap.add_argument("--out", default=str(DEFAULT_DIR))
    args = ap.parse_args()

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    shot = Image.open(args.shot).convert("RGB")
    assert shot.size == (3640, 1920), f"截图尺寸异常: {shot.size}"

    manifest = {
        "source_screenshot": {
            "path": str(Path(args.shot).relative_to(REPO)),
            "size": list(shot.size),
            "sha256": sha256_of(Path(args.shot)),
            "virtual_desktop_origin": list(VIRTUAL_ORIGIN),
            "note": "scripts/spikes/tools/screenshot.ps1 输出的虚拟桌面物理像素截图（PerMonitorV2）",
        },
        "derived": {},
    }

    # 1) 主屏真实裁剪 2560x1440
    primary = crop_physical(shot, PRIMARY_RECT)
    assert primary.size == (2560, 1440)
    p1 = out_dir / "screen_primary_2560x1440.png"
    primary.save(p1, optimize=True)
    manifest["derived"][p1.name] = {
        "size": list(primary.size),
        "method": "虚拟桌面截图按主屏物理矩形 (0,0,2560,1440) 裁剪；150% 缩放主屏真实内容",
    }

    # 2) 单屏 4K：主屏内容 1.5x LANCZOS 放大（2560x1440 -> 3840x2160）
    k4 = upscale(primary, 3840, 2160)
    p2 = out_dir / "screen_4k_3840x2160.png"
    k4.save(p2, optimize=True)
    manifest["derived"][p2.name] = {
        "size": list(k4.size),
        "method": "主屏真实内容 2560x1440 按 1.5x LANCZOS 放大到 3840x2160；文字笔画等效于 4K 屏上的 UI",
    }

    # 3) 4K 双屏冻结帧 7680x2160：左=主屏 4K，右=同一 4K 内容（两屏同源，文本复杂度一致）
    dual = Image.new("RGB", (7680, 2160))
    dual.paste(k4, (0, 0))
    dual.paste(k4, (3840, 0))
    p3 = out_dir / "dual_4k_7680x2160.png"
    dual.save(p3, optimize=True)
    manifest["derived"][p3.name] = {
        "size": list(dual.size),
        "method": "两块 3840x2160 横向拼接（左/右均为同一真实主屏内容的 4K 放大）；"
                  "用于模拟 4K 双屏冻结帧的物理像素尺寸 7680x2160",
    }

    # 降采样对照（detMaxEdge=3200 对应 dual 4K -> 3200x900）
    for long_edge, tag in ((3200, "dual4k_to_3200"), (1920, "dual4k_to_1920")):
        scale = long_edge / max(dual.size)
        dsize = (int(round(dual.width * scale)), int(round(dual.height * scale)))
        d = dual.resize(dsize, Image.LANCZOS)
        p = out_dir / f"dual_4k_det_{dsize[0]}x{dsize[1]}.png"
        d.save(p, optimize=True)
        manifest["derived"][p.name] = {
            "size": list(dsize),
            "method": f"dual_4k_7680x2160 按长边 {long_edge} LANCZOS 降采样（长边/短边 = {long_edge/dsize[1]:.2f}）",
        }

    # 4) 选区裁剪：取自 4K 单屏（选定区域为真实 UI 文本区或空白区）
    #    (left, top, w, h, tag, 描述)
    sel_specs = [
        (900, 260, 1200, 800, "dense", "聊天正文/侧栏混合，文字密集（中文正文+英文 UI）"),
        (980, 620, 800, 600, "dense_small", "聊天消息气泡附近，文字密集"),
        (0, 0, 800, 600, "sparse", "应用顶部空白/菜单栏区域，文字稀疏"),
        (2900, 300, 800, 600, "sparse_blank", "聊天区右侧空白区域，几乎无文字"),
        (0, 380, 1200, 800, "sidebar", "左侧会话列表，短文本多行"),
    ]
    for left, top, w, h, tag, desc in sel_specs:
        c = k4.crop((left, top, left + w, top + h))
        p = out_dir / f"sel_{w}x{h}_{tag}.png"
        c.save(p, optimize=True)
        manifest["derived"][p.name] = {
            "size": list(c.size),
            "method": f"从 screen_4k_3840x2160 裁剪 rect=({left},{top},{w},{h})；{desc}",
        }

    # 清理中间预览图（仅保留必要输入）
    half = out_dir / "desktop_full_half.png"
    if half.exists():
        half.unlink()

    for name, meta in manifest["derived"].items():
        meta["sha256"] = sha256_of(out_dir / name)
        meta["bytes"] = (out_dir / name).stat().st_size
    manifest["source_screenshot"]["bytes"] = Path(args.shot).stat().st_size

    (out_dir / "inputs_manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    for name, meta in manifest["derived"].items():
        print(f"{name:40s} {meta['size'][0]:5d}x{meta['size'][1]:<5d} {meta['bytes']/1024:8.1f} KB")
    print("manifest ->", out_dir / "inputs_manifest.json")


if __name__ == "__main__":
    main()
