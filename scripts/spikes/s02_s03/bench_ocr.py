# -*- coding: utf-8 -*-
"""S-02（整屏 det）/ S-03（选区 det+rec）OCR 时延基准。

真实 ONNX 推理：RapidOCR 1.4.4 包内 ch_PP-OCRv4 det/rec（PaddleOCR PP-OCRv4 的中文模型），
ONNX Runtime CPU EP。计时全部用 time.perf_counter()（毫秒）。

用法（在仓库根目录执行，使用 .verify/venv-ocr/Scripts/python.exe）：
    # 冒烟：验证模型可用 + 打印真实识别文本
    python scripts/spikes/s02_s03/bench_ocr.py smoke
    # S-02：整屏 det（双 4K / 单 4K / 主屏 2560x1440 对照）
    python scripts/spikes/s02_s03/bench_ocr.py s02 --reps 30 --threads 8
    # S-03：选区 det+rec，100 次取 P95（并测 det 缓存 → rec-only 对照）
    python scripts/spikes/s02_s03/bench_ocr.py s03 --reps 100 --threads 8
    # 60Hz 定时器抖动（worker 线程跑整屏 det）
    python scripts/spikes/s02_s03/bench_ocr.py jitter --reps 20 --threads 8
    # detMaxEdge 扫描（时延/框数权衡，供处置建议）
    python scripts/spikes/s02_s03/bench_ocr.py edge-sweep --reps 8 --threads 8
    # 全部正式测量（自动独占 bench.lock，测量结束后释放）
    python scripts/spikes/s02_s03/bench_ocr.py all --reps 30 --threads 8

注意：
- `all` 会在测量期间持有 `.verify/bench.lock`，期间不要并行编译/下载/装包；
- 测量数据是**真实** ONNX 推理结果，不做任何平滑或剔除；warmup 次数单独记录且不计入统计。
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import threading
import time
from pathlib import Path
from typing import Any, Dict, List, Tuple

import cv2
import numpy as np
import yaml

from bench_common import (
    INPUTS_DIR,
    RESULTS_DIR,
    SUMMARY_DIR,
    bench_lock,
    env_info,
    ort_info,
    save_csv,
    save_json,
    summarize,
)

import rapidocr_onnxruntime
from rapidocr_onnxruntime.ch_ppocr_det import TextDetector
from rapidocr_onnxruntime.ch_ppocr_rec import TextRecognizer

PKG_DIR = Path(rapidocr_onnxruntime.__file__).resolve().parent
MODELS_DIR = PKG_DIR / "models"
DEFAULT_THREADS = 8
DEFAULT_DET_MAX_EDGE = 3200  # config.json ocr.detMaxEdge
TEXT_THRESHOLD = 0.6  # config.json ocr.textThreshold
PERIOD_60HZ_MS = 1000.0 / 60.0


def budget_target_size(
    w: int, h: int, budget_px: float, long_edge_cap: int = DEFAULT_DET_MAX_EDGE
) -> Tuple[int, int, float]:
    """像素预算口径：det 输入同时满足「长边 ≤ long_edge_cap」与「像素 ≤ budget_px」。

    scale = min(long_edge_cap / max(w,h), sqrt(budget_px / (w*h)))，scale ≥ 1 时不放大。
    例：双 4K 7680x2160 + 3.0Mpx → 受长边 3200 约束 → 3200x900（2.88Mpx）；
        单 4K 3840x2160 + 3.0Mpx → 受像素预算约束 → 2309x1299（3.0Mpx）。
    """
    scale = min(long_edge_cap / max(w, h), math.sqrt(budget_px / float(w * h)))
    if scale >= 1.0:
        return w, h, 1.0
    return max(1, int(round(w * scale))), max(1, int(round(h * scale))), scale


# ------------------------------------------------------------------ 引擎
def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def model_info() -> Dict[str, Any]:
    from importlib.metadata import version

    out: Dict[str, Any] = {"package": f"rapidocr-onnxruntime {version('rapidocr-onnxruntime')}", "files": {}}
    for name in ("ch_PP-OCRv4_det_infer.onnx", "ch_PP-OCRv4_rec_infer.onnx"):
        p = MODELS_DIR / name
        out["files"][name] = {"bytes": p.stat().st_size, "sha256": _sha256(p)}
    out["package_dir"] = str(PKG_DIR)
    return out


class OcrEngines:
    """RapidOCR 的 det/rec，可指定线程数；构造耗时单独记录（= 冷启动模型加载）。"""

    def __init__(self, threads: int = DEFAULT_THREADS, det_max_edge: int = DEFAULT_DET_MAX_EDGE):
        cfg = yaml.safe_load((PKG_DIR / "config.yaml").read_text(encoding="utf-8"))

        det_cfg = dict(cfg["Det"])
        det_cfg.update(
            model_path=str(MODELS_DIR / "ch_PP-OCRv4_det_infer.onnx"),
            intra_op_num_threads=threads,
            inter_op_num_threads=-1,  # 交给 ORT 默认
            box_thresh=TEXT_THRESHOLD,  # 对齐 config.json ocr.textThreshold
        )
        t0 = time.perf_counter()
        self.det = TextDetector(det_cfg)
        self.det_init_ms = (time.perf_counter() - t0) * 1000

        rec_cfg = dict(cfg["Rec"])
        rec_cfg.update(
            model_path=str(MODELS_DIR / "ch_PP-OCRv4_rec_infer.onnx"),
            intra_op_num_threads=threads,
            inter_op_num_threads=-1,
            rec_batch_num=6,
            rec_img_shape=[3, 48, 320],
        )
        t0 = time.perf_counter()
        self.rec = TextRecognizer(rec_cfg)
        self.rec_init_ms = (time.perf_counter() - t0) * 1000

        self.threads = threads
        self.det_max_edge = det_max_edge

    # -- det：输入整帧（native），先降采样到 long_edge，再 det（计时分开）
    def det_full(
        self, img: np.ndarray, det_max_edge: int | None = None, do_resize: bool = True
    ) -> Tuple[np.ndarray, float, float, Dict[str, Any]]:
        long_edge = self.det_max_edge if det_max_edge is None else det_max_edge
        h, w = img.shape[:2]
        m = max(h, w)
        resize_ms = 0.0
        resized = img
        if do_resize and m > long_edge:
            scale = long_edge / m
            tw, th = int(round(w * scale)), int(round(h * scale))
            t0 = time.perf_counter()
            resized = cv2.resize(img, (tw, th), interpolation=cv2.INTER_AREA)
            resize_ms = (time.perf_counter() - t0) * 1000
        boxes, det_s = self.det(resized)
        boxes = np.asarray(boxes, dtype=np.float32) if boxes is not None else np.zeros((0, 4, 2), np.float32)
        if boxes.ndim != 3:
            boxes = np.zeros((0, 4, 2), np.float32)
        meta = {"det_input_h": resized.shape[0], "det_input_w": resized.shape[1]}
        return boxes, resize_ms, det_s * 1000.0, meta

    # -- 像素预算口径 det：输入同时满足「长边 ≤ det_max_edge」与「像素 ≤ budget_px」
    def det_budget(
        self, img: np.ndarray, budget_mpx: float, det_max_edge: int | None = None
    ) -> Tuple[np.ndarray, float, float, Dict[str, Any]]:
        cap = self.det_max_edge if det_max_edge is None else det_max_edge
        h, w = img.shape[:2]
        budget_px = budget_mpx * 1e6
        tw, th, scale = budget_target_size(w, h, budget_px, cap)
        resize_ms = 0.0
        resized = img
        if scale < 1.0 and (tw, th) != (w, h):
            t0 = time.perf_counter()
            resized = cv2.resize(img, (tw, th), interpolation=cv2.INTER_AREA)
            resize_ms = (time.perf_counter() - t0) * 1000
        boxes, det_s = self.det(resized)
        boxes = np.asarray(boxes, dtype=np.float32) if boxes is not None else np.zeros((0, 4, 2), np.float32)
        if boxes.ndim != 3:
            boxes = np.zeros((0, 4, 2), np.float32)
        meta = {
            "det_input_h": resized.shape[0],
            "det_input_w": resized.shape[1],
            "det_input_mpx": round(resized.shape[0] * resized.shape[1] / 1e6, 3),
            "scale": round((resized.shape[0] * resized.shape[1]) / (h * w), 4),
            "budget_mpx": budget_mpx,
            "long_edge_cap": cap,
        }
        return boxes, resize_ms, det_s * 1000.0, meta

    # -- 选区：det → 裁框 → rec（各段分别计时）
    def det_rec_region(
        self, img: np.ndarray, cached_boxes: np.ndarray | None = None
    ) -> Dict[str, Any]:
        t_all = time.perf_counter()
        if cached_boxes is None:
            boxes, det_s = self.det(img)
            det_ms = det_s * 1000.0
            boxes = np.asarray(boxes, dtype=np.float32)
            if boxes.ndim != 3:
                boxes = np.zeros((0, 4, 2), np.float32)
        else:
            boxes, det_ms = cached_boxes, -1.0

        t0 = time.perf_counter()
        crops = self._crops(img, boxes)
        crop_ms = (time.perf_counter() - t0) * 1000

        rec_ms = 0.0
        rec_res: List[Tuple[str, float]] = []
        if crops:
            rec_res, rec_s = self.rec(crops)
            rec_ms = rec_s * 1000.0
        total_ms = (time.perf_counter() - t_all) * 1000
        return {
            "total_ms": total_ms,
            "det_ms": det_ms,
            "crop_ms": crop_ms,
            "rec_ms": rec_ms,
            "boxes": boxes,
            "texts": rec_res,
        }

    @staticmethod
    def _crops(img: np.ndarray, boxes: np.ndarray) -> List[np.ndarray]:
        h, w = img.shape[:2]
        crops = []
        for pts in boxes:
            x0 = int(max(0, np.floor(pts[:, 0].min())))
            y0 = int(max(0, np.floor(pts[:, 1].min())))
            x1 = int(min(w, np.ceil(pts[:, 0].max())))
            y1 = int(min(h, np.ceil(pts[:, 1].max())))
            if x1 - x0 < 3 or y1 - y0 < 3:
                continue
            crops.append(img[y0:y1, x0:x1])
        return crops


# ------------------------------------------------------------------ 输入
def load_bgr(path: Path) -> np.ndarray:
    # cv2.imread 不支持非 ASCII 路径（本仓库路径含中文），改用 fromfile + imdecode
    buf = np.fromfile(str(path), dtype=np.uint8)
    img = cv2.imdecode(buf, cv2.IMREAD_COLOR)
    if img is None:
        raise SystemExit(f"无法读取图片: {path}")
    return img


S02_CASES = [
    ("dual4k_7680x2160", "dual_4k_7680x2160.png", "4K 双屏冻结帧（主用例）：7680x2160 → 长边 3200"),
    ("single4k_3840x2160", "screen_4k_3840x2160.png", "单屏 4K 对照：3840x2160 → 长边 3200"),
    ("primary_2560x1440", "screen_primary_2560x1440.png", "主屏真实桌面对照：2560x1440（不降采样）"),
]


def selection_paths() -> List[Path]:
    paths = sorted(INPUTS_DIR.glob("sel_*.png"))
    if not paths:
        raise SystemExit(f"未找到选区图：{INPUTS_DIR}/sel_*.png，请先运行 make_inputs.py")
    return paths


def _box_count(boxes: np.ndarray) -> int:
    return int(boxes.shape[0]) if boxes.ndim == 3 else 0


# ------------------------------------------------------------------ S-02
def run_s02(threads: int, reps: int, warmup: int, det_max_edge: int) -> Dict[str, Any]:
    print(f"\n=== S-02 整屏 det（threads={threads}, reps={reps}, warmup={warmup}, detMaxEdge={det_max_edge}）===")
    eng = OcrEngines(threads, det_max_edge)
    out: Dict[str, Any] = {
        "threads": threads,
        "reps": reps,
        "warmup": warmup,
        "det_max_edge": det_max_edge,
        "engine_init_ms": {"det": round(eng.det_init_ms, 1), "rec": round(eng.rec_init_ms, 1)},
        "cases": {},
    }
    all_rows: List[Dict[str, Any]] = []
    for tag, fname, desc in S02_CASES:
        path = INPUTS_DIR / fname
        img = load_bgr(path)
        for _ in range(warmup):  # warmup 不计入统计
            eng.det_full(img)
        rows = []
        for i in range(reps):
            t0 = time.perf_counter()
            boxes, resize_ms, det_ms, meta = eng.det_full(img)
            total_ms = (time.perf_counter() - t0) * 1000
            rows.append(
                {
                    "case": tag,
                    "rep": i,
                    "resize_ms": round(resize_ms, 3),
                    "det_ms": round(det_ms, 3),
                    "total_ms": round(total_ms, 3),
                    "boxes": _box_count(boxes),
                    "det_input_w": meta["det_input_w"],
                    "det_input_h": meta["det_input_h"],
                }
            )
        all_rows.extend(rows)
        n_boxes = sorted({r["boxes"] for r in rows})
        case = {
            "desc": desc,
            "input": str(path.relative_to(INPUTS_DIR.parent.parent.parent)),
            "input_w": int(img.shape[1]),
            "input_h": int(img.shape[0]),
            "det_input_w": meta["det_input_w"],
            "det_input_h": meta["det_input_h"],
            "downsample_scale": round(max(img.shape[:2]) / max(meta["det_input_w"], meta["det_input_h"]), 4),
            "summary": {
                "resize_ms": summarize([r["resize_ms"] for r in rows]),
                "det_ms": summarize([r["det_ms"] for r in rows]),
                "total_ms": summarize([r["total_ms"] for r in rows]),
            },
            "boxes_observed": n_boxes,
            "rows": rows,
        }
        out["cases"][tag] = case
        s = case["summary"]["total_ms"]
        print(
            f"  {tag:20s} 冻结帧 {img.shape[1]}x{img.shape[0]} → det {meta['det_input_w']}x{meta['det_input_h']}"
            f" | total P50={s['p50']:.0f}ms P95={s['p95']:.0f}ms max={s['max']:.0f}ms"
            f" | det P50={case['summary']['det_ms']['p50']:.0f}ms | resize P50={case['summary']['resize_ms']['p50']:.1f}ms"
            f" | boxes={n_boxes}"
        )

    save_csv(
        RESULTS_DIR / "s02_raw.csv",
        all_rows,
        ["case", "rep", "resize_ms", "det_ms", "total_ms", "boxes", "det_input_w", "det_input_h"],
    )
    save_json(RESULTS_DIR / "s02_det.json", out)
    print(f"  -> {RESULTS_DIR / 's02_raw.csv'}")
    return out


# ------------------------------------------------------------------ S-03
def run_s03(threads: int, reps: int, warmup: int) -> Dict[str, Any]:
    print(f"\n=== S-03 选区 det+rec（threads={threads}, reps={reps}, warmup={warmup}）===")
    eng = OcrEngines(threads)
    paths = selection_paths()
    imgs = {p.name: load_bgr(p) for p in paths}
    print("  选区：" + ", ".join(f"{p.name} {imgs[p.name].shape[1]}x{imgs[p.name].shape[0]}" for p in paths))

    # warmup（同时建立 det 缓存，供 rec-only 对照复用）
    cache: Dict[str, np.ndarray] = {}
    for i in range(warmup):
        name = paths[i % len(paths)].name
        r = eng.det_rec_region(imgs[name])
        cache[name] = r["boxes"]

    rows: List[Dict[str, Any]] = []
    for i in range(reps):
        name = paths[i % len(paths)].name
        r = eng.det_rec_region(imgs[name])
        rows.append(
            {
                "selection": name,
                "rep": i,
                "total_ms": round(r["total_ms"], 3),
                "det_ms": round(r["det_ms"], 3),
                "crop_ms": round(r["crop_ms"], 3),
                "rec_ms": round(r["rec_ms"], 3),
                "boxes": _box_count(r["boxes"]),
                "n_crops": len(r["texts"]),
            }
        )
        cache[name] = r["boxes"]

    # det 缓存复用 → rec-only 对照（架构中「整屏 det 缓存 + 选区只过滤/识别」的优化路径）
    rec_rows: List[Dict[str, Any]] = []
    for i in range(reps):
        name = paths[i % len(paths)].name
        r = eng.det_rec_region(imgs[name], cached_boxes=cache[name])
        rec_rows.append(
            {
                "selection": name,
                "rep": i,
                "total_ms": round(r["total_ms"], 3),
                "crop_ms": round(r["crop_ms"], 3),
                "rec_ms": round(r["rec_ms"], 3),
                "n_crops": len(r["texts"]),
            }
        )

    def by_sel(rows_in: List[Dict[str, Any]]) -> Dict[str, Any]:
        d = {}
        for name in imgs:
            sub = [r for r in rows_in if r["selection"] == name]
            if sub:
                d[name] = {
                    "n": len(sub),
                    "total_ms": summarize([r["total_ms"] for r in sub]),
                    "det_ms": summarize([r["det_ms"] for r in sub]) if "det_ms" in sub[0] else None,
                    "rec_ms": summarize([r["rec_ms"] for r in sub]),
                    "boxes": sub[0].get("boxes"),
                }
        return d

    out: Dict[str, Any] = {
        "threads": threads,
        "reps": reps,
        "warmup": warmup,
        "selections": [
            {"name": p.name, "w": int(imgs[p.name].shape[1]), "h": int(imgs[p.name].shape[0])} for p in paths
        ],
        "det_plus_rec": {
            "summary": {
                "total_ms": summarize([r["total_ms"] for r in rows]),
                "det_ms": summarize([r["det_ms"] for r in rows]),
                "crop_ms": summarize([r["crop_ms"] for r in rows]),
                "rec_ms": summarize([r["rec_ms"] for r in rows]),
            },
            "by_selection": by_sel(rows),
            "rows": rows,
        },
        "det_cache_rec_only": {
            "summary": {
                "total_ms": summarize([r["total_ms"] for r in rec_rows]),
                "rec_ms": summarize([r["rec_ms"] for r in rec_rows]),
            },
            "by_selection": by_sel(rec_rows),
            "rows": rec_rows,
        },
        "sample_texts": {},
    }
    # 真实识别文本样本（证明是真实 ONNX 推理，不是伪造数字）
    for p in paths:
        r = eng.det_rec_region(imgs[p.name])
        texts = [t for t, sc in r["texts"] if sc >= TEXT_THRESHOLD]
        out["sample_texts"][p.name] = {
            "n_boxes": _box_count(r["boxes"]),
            "n_texts": len(texts),
            "texts": texts[:12],
        }

    s = out["det_plus_rec"]["summary"]
    print(
        f"  det+rec {len(rows)} 次：total P50={s['total_ms']['p50']:.0f}ms P95={s['total_ms']['p95']:.0f}ms "
        f"max={s['total_ms']['max']:.0f}ms | det P95={s['det_ms']['p95']:.0f}ms | rec P95={s['rec_ms']['p95']:.0f}ms"
    )
    print(f"  det 缓存→rec-only：total P95={out['det_cache_rec_only']['summary']['total_ms']['p95']:.0f}ms")
    for name, txt in out["sample_texts"].items():
        print(f"  [{name}] boxes={txt['n_boxes']} 识别样本: {txt['texts'][:4]}")

    save_csv(
        RESULTS_DIR / "s03_raw.csv",
        rows,
        ["selection", "rep", "total_ms", "det_ms", "crop_ms", "rec_ms", "boxes", "n_crops"],
    )
    save_csv(
        RESULTS_DIR / "s03_rec_only_raw.csv",
        rec_rows,
        ["selection", "rep", "total_ms", "crop_ms", "rec_ms", "n_crops"],
    )
    save_json(RESULTS_DIR / "s03_det_rec.json", out)
    print(f"  -> {RESULTS_DIR / 's03_raw.csv'}")
    return out


# ------------------------------------------------------------------ 60Hz 抖动
def _spin_timer_loop(duration_s: float, stop: threading.Event | None = None) -> Tuple[List[float], int]:
    """忙等到每个 60Hz 截止点，返回相邻间隔(ms)与循环次数。"""
    period = 1.0 / 60.0
    t0 = time.perf_counter()
    deadline = t0 + period
    intervals: List[float] = []
    n = 0
    last = t0
    while True:
        if stop is not None and stop.is_set():
            break
        while time.perf_counter() < deadline:  # 忙等，避免 Windows sleep 粒度影响
            pass
        now = time.perf_counter()
        intervals.append((now - last) * 1000.0)
        last = now
        n += 1
        deadline += period
        if now - t0 > duration_s:
            break
    return intervals, n


def jitter_stats(intervals: List[float]) -> Dict[str, Any]:
    a = np.asarray(intervals, dtype=np.float64)
    abs_jit = np.abs(a - PERIOD_60HZ_MS)
    return {
        "n": int(a.size),
        "interval_ms": summarize(a.tolist()),
        "abs_jitter_ms": {
            "mean": round(float(abs_jit.mean()), 3),
            "p50": round(float(np.percentile(abs_jit, 50)), 3),
            "p95": round(float(np.percentile(abs_jit, 95)), 3),
            "max": round(float(abs_jit.max()), 3),
        },
        "frames_over_2x_period": int((a > 2 * PERIOD_60HZ_MS).sum()),
        "frames_over_3x_period": int((a > 3 * PERIOD_60HZ_MS).sum()),
    }


def run_jitter(threads: int, reps: int) -> Dict[str, Any]:
    print(f"\n=== 60Hz 定时器抖动（worker 线程跑整屏 det，threads={threads}, reps={reps}）===")
    eng = OcrEngines(threads)
    img = load_bgr(INPUTS_DIR / "dual_4k_7680x2160.png")

    baseline_intervals, _ = _spin_timer_loop(2.0)
    baseline = jitter_stats(baseline_intervals)

    done = threading.Event()
    det_rows: List[Dict[str, Any]] = []

    def worker() -> None:
        try:
            for i in range(reps):
                t0 = time.perf_counter()
                _, resize_ms, det_ms, meta = eng.det_full(img)
                det_rows.append(
                    {
                        "rep": i,
                        "resize_ms": round(resize_ms, 3),
                        "det_ms": round(det_ms, 3),
                        "total_ms": round((time.perf_counter() - t0) * 1000, 3),
                        "det_input_w": meta["det_input_w"],
                        "det_input_h": meta["det_input_h"],
                    }
                )
        finally:
            done.set()

    th = threading.Thread(target=worker, name="ocr-worker", daemon=True)
    t_start = time.perf_counter()
    th.start()
    intervals, _ = _spin_timer_loop(duration_s=60.0, stop=done)
    th.join(timeout=30)
    wall = time.perf_counter() - t_start
    with_bg = jitter_stats(intervals)

    out = {
        "threads": threads,
        "reps": reps,
        "worker_wall_s": round(wall, 2),
        "worker_det_total_ms": summarize([r["total_ms"] for r in det_rows]) if det_rows else None,
        "timer_baseline_no_bg": baseline,
        "timer_with_bg_det": with_bg,
        "interval_period_ms": round(PERIOD_60HZ_MS, 3),
        "worker_rows": det_rows,
        "note": "Python 线程 + GIL 与生产 Rust 线程不同，仅作量级参考；详见文档「局限」",
    }
    print(
        f"  基线（无后台）：interval P50={baseline['interval_ms']['p50']:.2f}ms 绝对抖动 max={baseline['abs_jitter_ms']['max']:.2f}ms"
    )
    print(
        f"  后台 det 时：interval P50={with_bg['interval_ms']['p50']:.2f}ms 绝对抖动 mean={with_bg['abs_jitter_ms']['mean']:.2f}ms"
        f" P95={with_bg['abs_jitter_ms']['p95']:.2f}ms max={with_bg['abs_jitter_ms']['max']:.2f}ms"
        f" 超过 2 帧周期次数={with_bg['frames_over_2x_period']}"
    )
    save_json(RESULTS_DIR / "jitter.json", out)
    return out


# ------------------------------------------------------------------ detMaxEdge 扫描
def run_edge_sweep(threads: int, reps: int) -> Dict[str, Any]:
    print(f"\n=== detMaxEdge 扫描（dual 4K 冻结帧，threads={threads}, reps={reps}）===")
    eng = OcrEngines(threads)
    img = load_bgr(INPUTS_DIR / "dual_4k_7680x2160.png")
    sweep: Dict[str, Any] = {"threads": threads, "reps": reps, "input": "dual_4k_7680x2160.png", "edges": {}}
    for edge in (3200, 4000, 4800):
        eng2 = OcrEngines(threads, edge)
        for _ in range(3):
            eng2.det_full(img)
        rows = []
        for i in range(reps):
            t0 = time.perf_counter()
            boxes, resize_ms, det_ms, meta = eng2.det_full(img)
            rows.append(
                {
                    "edge": edge,
                    "rep": i,
                    "resize_ms": round(resize_ms, 3),
                    "det_ms": round(det_ms, 3),
                    "total_ms": round((time.perf_counter() - t0) * 1000, 3),
                    "boxes": _box_count(boxes),
                    "det_input_w": meta["det_input_w"],
                    "det_input_h": meta["det_input_h"],
                }
            )
        sweep["edges"][str(edge)] = {
            "det_input_w": meta["det_input_w"],
            "det_input_h": meta["det_input_h"],
            "megapixels": round(meta["det_input_w"] * meta["det_input_h"] / 1e6, 3),
            "summary": {
                "resize_ms": summarize([r["resize_ms"] for r in rows]),
                "det_ms": summarize([r["det_ms"] for r in rows]),
                "total_ms": summarize([r["total_ms"] for r in rows]),
            },
            "boxes_observed": sorted({r["boxes"] for r in rows}),
            "rows": rows,
        }
        s = sweep["edges"][str(edge)]["summary"]["total_ms"]
        print(
            f"  detMaxEdge={edge:5d} → {meta['det_input_w']}x{meta['det_input_h']}"
            f" ({sweep['edges'][str(edge)]['megapixels']} Mpx) total P50={s['p50']:.0f}ms P95={s['p95']:.0f}ms"
            f" | boxes={sweep['edges'][str(edge)]['boxes_observed']}"
        )
    save_json(RESULTS_DIR / "edge_sweep.json", sweep)
    return sweep


# ------------------------------------------------------------------ 像素预算复核（phase 2 / task-12）
def run_budget_review(threads: int = DEFAULT_THREADS) -> Dict[str, Any]:
    """复核「det 输入 = min(长边 3200, 像素预算)」口径（item2 用户决策）。

    用例：
      1) 双 4K 7680x2160，旧口径长边 3200 → 3200x900，30 次（应 ≈ 旧值 P95 926ms）
      2) 单 4K 3840x2160，旧口径长边 3200 → 3200x1800，10 次（同窗口参考，对照 1695ms）
      3) 单 4K，预算 3.0Mpx → 2309x1299，30 次（新口径主用例）
      4) 单 4K，预算扫描 2.5 / 3.0 / 3.5 Mpx，各 20 次（记录框数）
      5) 主屏 2560x1440，预算 3.0Mpx → 2309x1299，20 次（附带：旧口径 979ms，余量小）
    """
    print(f"\n=== 像素预算复核（threads={threads}, long_edge_cap={DEFAULT_DET_MAX_EDGE}）===")
    eng = OcrEngines(threads)
    img_dual = load_bgr(INPUTS_DIR / "dual_4k_7680x2160.png")
    img_single = load_bgr(INPUTS_DIR / "screen_4k_3840x2160.png")
    img_primary = load_bgr(INPUTS_DIR / "screen_primary_2560x1440.png")

    out: Dict[str, Any] = {
        "reviewed_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "threads": threads,
        "long_edge_cap": DEFAULT_DET_MAX_EDGE,
        "engine_init_ms": {"det": round(eng.det_init_ms, 1), "rec": round(eng.rec_init_ms, 1)},
        "cases": {},
    }
    all_rows: List[Dict[str, Any]] = []

    def run_case(
        tag: str,
        img: np.ndarray,
        reps: int,
        budget_mpx: float | None,
        desc: str,
        warmup: int = 3,
    ) -> Dict[str, Any]:
        runner = (lambda: eng.det_budget(img, budget_mpx)) if budget_mpx else (lambda: eng.det_full(img))
        for _ in range(warmup):
            runner()
        rows = []
        for i in range(reps):
            t0 = time.perf_counter()
            boxes, resize_ms, det_ms, meta = runner()
            rows.append(
                {
                    "case": tag,
                    "rep": i,
                    "resize_ms": round(resize_ms, 3),
                    "det_ms": round(det_ms, 3),
                    "total_ms": round((time.perf_counter() - t0) * 1000, 3),
                    "boxes": _box_count(boxes),
                    "det_input_w": meta["det_input_w"],
                    "det_input_h": meta["det_input_h"],
                    "det_input_mpx": meta.get("det_input_mpx"),
                    "budget_mpx": meta.get("budget_mpx"),
                }
            )
        all_rows.extend(rows)
        box_obs = sorted({r["boxes"] for r in rows})
        case = {
            "desc": desc,
            "input_w": int(img.shape[1]),
            "input_h": int(img.shape[0]),
            "det_input_w": meta["det_input_w"],
            "det_input_h": meta["det_input_h"],
            "det_input_mpx": meta.get("det_input_mpx")
            or round(meta["det_input_w"] * meta["det_input_h"] / 1e6, 3),
            "budget_mpx": budget_mpx,
            "reps": reps,
            "summary": {
                "resize_ms": summarize([r["resize_ms"] for r in rows]),
                "det_ms": summarize([r["det_ms"] for r in rows]),
                "total_ms": summarize([r["total_ms"] for r in rows]),
            },
            "boxes_observed": box_obs,
            "rows": rows,
        }
        out["cases"][tag] = case
        s = case["summary"]["total_ms"]
        print(
            f"  [{tag:28s}] {img.shape[1]}x{img.shape[0]} → det {meta['det_input_w']}x{meta['det_input_h']}"
            f" ({case['det_input_mpx']} Mpx) | total P50={s['p50']:.0f}ms P95={s['p95']:.0f}ms max={s['max']:.0f}ms"
            f" | resize P50={case['summary']['resize_ms']['p50']:.1f}ms | boxes={box_obs}"
        )
        return case

    run_case("dual4k_legacy3200_30", img_dual, 30, None, "双 4K 7680x2160，旧口径长边 3200（=像素预算口径下的同一结果）")
    run_case("single4k_legacy3200_10", img_single, 10, None, "单 4K 3840x2160，旧口径长边 3200（对照 1695ms）")
    run_case("single4k_budget3.0mpx_30", img_single, 30, 3.0, "单 4K 3840x2160，像素预算 3.0Mpx（新口径主用例）")
    for b in (2.5, 3.0, 3.5):
        run_case(f"single4k_budget{b:.1f}mpx_20", img_single, 20, b, f"单 4K 像素预算扫描 {b:.1f}Mpx")
    run_case("primary2560x1440_budget3.0mpx_20", img_primary, 20, 3.0, "主屏 2560x1440，像素预算 3.0Mpx（附带）")

    save_csv(
        RESULTS_DIR / "budget_sweep_raw.csv",
        all_rows,
        ["case", "rep", "resize_ms", "det_ms", "total_ms", "boxes", "det_input_w", "det_input_h", "det_input_mpx", "budget_mpx"],
    )
    save_json(RESULTS_DIR / "budget_sweep.json", out)
    print(f"  -> {RESULTS_DIR / 'budget_sweep_raw.csv'}")
    return out


def update_summary_with_budget(review: Dict[str, Any]) -> None:
    """把预算复核结果增补进 docs/spikes/data/s02_s03/summary.json（保留原有 phase 1 字段）。"""
    path = SUMMARY_DIR / "summary.json"
    summary = json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}
    compact = {}
    for tag, c in review["cases"].items():
        compact[tag] = {
            "desc": c["desc"],
            "input_w": c["input_w"],
            "input_h": c["input_h"],
            "det_input_w": c["det_input_w"],
            "det_input_h": c["det_input_h"],
            "det_input_mpx": c["det_input_mpx"],
            "budget_mpx": c["budget_mpx"],
            "reps": c["reps"],
            "boxes_observed": c["boxes_observed"],
            "summary": c["summary"],
        }
    summary["budget_sweep"] = {
        "reviewed_at": review["reviewed_at"],
        "threads": review["threads"],
        "long_edge_cap": review["long_edge_cap"],
        "recommendation": {
            "rule": "det 输入 scale = min(long_edge_cap / max(w,h), sqrt(budget_px / (w*h)))，不放大",
            "budget_mpx": 3.0,
            "long_edge_cap": DEFAULT_DET_MAX_EDGE,
            "note": "双 4K 仍由长边 3200 约束（3200x900）；单 4K/2560x1440 由 3.0Mpx 预算约束（2309x1299）。"
                    "单 4K P95 由 1695ms 降至 ~850ms 量级，双 4K 保持 ~926ms。",
        },
        "cases": compact,
        "raw": ".verify/spikes/S-02-S-03/results/budget_sweep_raw.csv",
        "full_json": ".verify/spikes/S-02-S-03/results/budget_sweep.json",
    }
    save_json(path, summary)
    print(f"  -> 已增补 {path}（budget_sweep 字段）")


# ------------------------------------------------------------------ 线程数扫描
def run_thread_sweep(reps: int = 10) -> Dict[str, Any]:
    print(f"\n=== ORT 线程数扫描（dual 4K 冻结帧，det 总耗时，reps={reps}）===")
    img = load_bgr(INPUTS_DIR / "dual_4k_7680x2160.png")
    out: Dict[str, Any] = {"input": "dual_4k_7680x2160.png", "reps": reps, "configs": {}}
    for threads in (4, 6, 8, 12, 16):
        eng = OcrEngines(threads)
        for _ in range(3):
            eng.det_full(img)
        rows = []
        for i in range(reps):
            t0 = time.perf_counter()
            boxes, resize_ms, det_ms, meta = eng.det_full(img)
            rows.append(
                {
                    "threads": threads,
                    "rep": i,
                    "resize_ms": round(resize_ms, 3),
                    "det_ms": round(det_ms, 3),
                    "total_ms": round((time.perf_counter() - t0) * 1000, 3),
                    "boxes": _box_count(boxes),
                }
            )
        out["configs"][str(threads)] = {
            "det_input_w": meta["det_input_w"],
            "det_input_h": meta["det_input_h"],
            "summary": {
                "resize_ms": summarize([r["resize_ms"] for r in rows]),
                "det_ms": summarize([r["det_ms"] for r in rows]),
                "total_ms": summarize([r["total_ms"] for r in rows]),
            },
            "rows": rows,
        }
        s = out["configs"][str(threads)]["summary"]["total_ms"]
        print(f"  threads={threads:2d}: total P50={s['p50']:.0f}ms P95={s['p95']:.0f}ms max={s['max']:.0f}ms")
    save_json(RESULTS_DIR / "thread_sweep.json", out)
    return out


# ------------------------------------------------------------------ smoke
def run_smoke(threads: int) -> None:
    print(f"=== smoke：模型可用性 + 真实识别（threads={threads}）===")
    eng = OcrEngines(threads)
    print(f"  引擎初始化：det={eng.det_init_ms:.0f}ms rec={eng.rec_init_ms:.0f}ms")
    for name in ("screen_primary_2560x1440.png", selection_paths()[0].name):
        img = load_bgr(INPUTS_DIR / name)
        r = eng.det_rec_region(img)
        print(f"  [{name}] {img.shape[1]}x{img.shape[0]} det={r['det_ms']:.0f}ms crop={r['crop_ms']:.1f}ms "
              f"rec={r['rec_ms']:.0f}ms total={r['total_ms']:.0f}ms boxes={_box_count(r['boxes'])}")
        for text, score in r["texts"][:6]:
            print(f"      {score:.3f}  {text}")
    boxes, resize_ms, det_ms, meta = eng.det_full(load_bgr(INPUTS_DIR / "dual_4k_7680x2160.png"))
    print(f"  [dual4k] resize={resize_ms:.1f}ms det={det_ms:.0f}ms boxes={_box_count(boxes)} "
          f"det_input={meta['det_input_w']}x{meta['det_input_h']}")
    print("  smoke OK：真实 ONNX 推理可用")


# ------------------------------------------------------------------ all
def run_all(threads: int, reps_s02: int, reps_s03: int, reps_jitter: int) -> Dict[str, Any]:
    with bench_lock(owner="ocr-bench", wait=True, timeout_s=900):
        RESULTS_DIR.mkdir(parents=True, exist_ok=True)
        env = env_info()
        ort = ort_info()
        print("环境:", json.dumps(env, ensure_ascii=False))
        print("ORT:", json.dumps(ort, ensure_ascii=False))
        model = model_info()
        print("模型:", json.dumps(model["files"], ensure_ascii=False))

        s02 = run_s02(threads, reps_s02, 3, DEFAULT_DET_MAX_EDGE)
        s03 = run_s03(threads, reps_s03, 3)
        jit = run_jitter(threads, reps_jitter)
        sweep = run_edge_sweep(threads, 6)
        tsweep = run_thread_sweep(8)

        summary = {
            "env": env,
            "ort": ort,
            "model": model,
            "config": {
                "detMaxEdge": DEFAULT_DET_MAX_EDGE,
                "textThreshold": TEXT_THRESHOLD,
                "threads": threads,
                "reps_s02": reps_s02,
                "reps_s03": reps_s03,
                "reps_jitter": reps_jitter,
            },
            "s02": {
                "cases": {
                    tag: {
                        "desc": c["desc"],
                        "input_w": c["input_w"],
                        "input_h": c["input_h"],
                        "det_input_w": c["det_input_w"],
                        "det_input_h": c["det_input_h"],
                        "downsample_scale": c["downsample_scale"],
                        "summary": c["summary"],
                        "boxes_observed": c["boxes_observed"],
                    }
                    for tag, c in s02["cases"].items()
                },
                "engine_init_ms": s02["engine_init_ms"],
            },
            "s03": {
                "summary": s03["det_plus_rec"]["summary"],
                "by_selection": s03["det_plus_rec"]["by_selection"],
                "det_cache_rec_only_summary": s03["det_cache_rec_only"]["summary"],
                "sample_texts": s03["sample_texts"],
            },
            "jitter": {
                "baseline": jit["timer_baseline_no_bg"],
                "with_bg_det": jit["timer_with_bg_det"],
                "worker_wall_s": jit["worker_wall_s"],
            },
            "edge_sweep": {
                k: {x: v[x] for x in ("det_input_w", "det_input_h", "megapixels", "summary", "boxes_observed")}
                for k, v in sweep["edges"].items()
            },
            "thread_sweep": {
                k: {
                    "det_input_w": v["det_input_w"],
                    "det_input_h": v["det_input_h"],
                    "summary": {kk: v["summary"][kk] for kk in ("det_ms", "total_ms")},
                }
                for k, v in tsweep["configs"].items()
            },
            "artifact_paths": {
                "raw_results_dir": ".verify/spikes/S-02-S-03/results/",
                "s02_raw_csv": ".verify/spikes/S-02-S-03/results/s02_raw.csv",
                "s03_raw_csv": ".verify/spikes/S-02-S-03/results/s03_raw.csv",
                "jitter_json": ".verify/spikes/S-02-S-03/results/jitter.json",
                "edge_sweep_json": ".verify/spikes/S-02-S-03/results/edge_sweep.json",
                "thread_sweep_json": ".verify/spikes/S-02-S-03/results/thread_sweep.json",
                "summary_json": "docs/spikes/data/s02_s03/summary.json",
                "inputs_manifest": ".verify/spikes/S-02-S-03/inputs/inputs_manifest.json",
            },
        }
        save_json(SUMMARY_DIR / "summary.json", summary)
        save_json(RESULTS_DIR / "summary.json", summary)
        print(f"\n汇总数据已写入：{SUMMARY_DIR / 'summary.json'}")
        return summary


def main() -> None:
    ap = argparse.ArgumentParser(description="S-02/S-03 OCR 时延基准")
    ap.add_argument(
        "cmd",
        choices=["smoke", "s02", "s03", "jitter", "edge-sweep", "thread-sweep", "budget", "all"],
        help="要执行的基准",
    )
    ap.add_argument("--threads", type=int, default=DEFAULT_THREADS, help="ORT intra_op 线程数")
    ap.add_argument("--reps", type=int, default=0, help="重复次数（不指定则用各基准默认值）")
    ap.add_argument("--warmup", type=int, default=3)
    ap.add_argument("--det-max-edge", type=int, default=DEFAULT_DET_MAX_EDGE)
    args = ap.parse_args()

    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    if args.cmd == "smoke":
        run_smoke(args.threads)
    elif args.cmd == "s02":
        with bench_lock("ocr-bench"):
            run_s02(args.threads, args.reps or 30, args.warmup, args.det_max_edge)
    elif args.cmd == "s03":
        with bench_lock("ocr-bench"):
            run_s03(args.threads, args.reps or 100, args.warmup)
    elif args.cmd == "jitter":
        with bench_lock("ocr-bench"):
            run_jitter(args.threads, args.reps or 20)
    elif args.cmd == "edge-sweep":
        with bench_lock("ocr-bench"):
            run_edge_sweep(args.threads, args.reps or 8)
    elif args.cmd == "thread-sweep":
        with bench_lock("ocr-bench"):
            run_thread_sweep(args.reps or 10)
    elif args.cmd == "budget":
        with bench_lock("ocr-bench"):
            review = run_budget_review(args.threads)
            update_summary_with_budget(review)
    else:
        run_all(args.threads, 30, 100, 20)


if __name__ == "__main__":
    main()
