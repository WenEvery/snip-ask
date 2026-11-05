# -*- coding: utf-8 -*-
"""S-11（文字存在判定阈值）/ S-15（OCR 字符准确率）评测脚本（可复跑）。

真实推理：RapidOCR 1.4.4 包内 PP-OCRv4 det/rec ONNX（与 S-02/S-03 同一套模型与预处理），
复用 `scripts/spikes/s02_s03/bench_ocr.py` 的引擎加载/降采样/裁剪逻辑，只做两处最小扩展：
  1. **保留 det 框的 DB 置信度**（RapidOCR 的 `TextDetector.__call__` 丢弃了 score；
     这里直接调它的 preprocess/infer/DBPostProcess/后处理，不重写预处理）；
  2. **det 框分数阈值可调**（S-11 阈值扫描需要看到低分框；产品默认 box_thresh=textThreshold=0.6）。

子命令：
    det     —— S-11 阶段 A：100 张语料逐张跑整屏 det（生产像素预算口径），把每张图的框
               （x/y/w/h/面积/DB 置信度）缓存到 results/s11_boxes.json
    sweep   —— S-11 阶段 B：在缓存框上扫描「面积 / 置信度 / 行数」三阈值网格，输出混淆矩阵、
               误判率/漏判率、推荐组合；可用 --min-area/--min-confidence/--min-lines 评估指定组合
    ocr     —— S-15：100 张真实截图跑 det+rec，按阅读顺序拼接，与构造真值算 CER
               （native=原分辨率口径；budget=生产像素预算口径，见下）
    all     —— det + sweep + ocr + 汇总（docs/spikes/data/s11_s15/summary.json）
    judge   —— 只做「指定阈值组合」的单点评估（从缓存框读），用于验证阈值可配置

口径：
- S-11 判定：`hasText = count(box.det_score >= min_conf 且 box.area_px >= min_area) >= min_lines`；
  area_px 为**还原到原图物理像素**后的轴对齐框面积；det 输入按生产预算 `scale=min(3200/max, sqrt(3.0Mpx/(w*h)))`。
- S-15 两口径：
  * native：帧原分辨率 det → 原分辨率裁框 rec（RapidOCR det 预处理 limit_type=min/736，不会缩小大步长输入）；
  * budget：按生产 `det_input_size` 把整帧降采样到 ≤3.0Mpx 且长边≤3200 后 det，框坐标还原到原帧，
    再**从原帧裁框** rec（与产品实现一致：det 降采样只影响定位，rec 仍用原图）。
- 阅读顺序拼接：框先按视觉行分组（按中心 y 排序，相邻 y 差 > 0.5*行高 则换行），行内按中心 x 排序，
  行间用 '\n' 拼接；随后做 NFKC + 去除所有空白再算编辑距离（拼接规则与归一化都会写进记录）。
- CER = 编辑距离 / max(1, 真值字符数)，准确率 = 1 - CER；错误按 rapidfuzz 的 delete/insert/replace 拆分
  （未安装 rapidfuzz 时退化为纯 Python DP，速度慢但结果一致）。

用法（仓库根目录，`.verify/venv-ocr/Scripts/python.exe`）：
    python scripts/spikes/s11_s15/run_ocr_eval.py all
    python scripts/spikes/s11_s15/run_ocr_eval.py det --limit 10
    python scripts/spikes/s11_s15/run_ocr_eval.py sweep
    python scripts/spikes/s11_s15/run_ocr_eval.py judge --min-area 40 --min-confidence 0.5 --min-lines 1
    python scripts/spikes/s11_s15/run_ocr_eval.py ocr --profiles native,budget
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import sys
import time
import unicodedata
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

import cv2
import numpy as np
import yaml

REPO = Path(__file__).resolve().parents[3]
S02_DIR = REPO / "scripts" / "spikes" / "s02_s03"
if str(S02_DIR) not in sys.path:
    sys.path.insert(0, str(S02_DIR))

import bench_ocr  # noqa: E402  （复用 OcrEngines / budget_target_size / load_bgr / save_csv / summarize）
from bench_common import save_csv, save_json, summarize  # noqa: E402

SPIKE = REPO / ".verify" / "spikes" / "S-11-S-15"
CORPUS = SPIKE / "corpus"
RESULTS = SPIKE / "results"
MANIFEST = CORPUS / "corpus_manifest.json"
DATA_DIR = REPO / "docs" / "spikes" / "data" / "s11_s15"

from rapidocr_onnxruntime.ch_ppocr_det.utils import DBPostProcess  # noqa: E402
from rapidocr_onnxruntime.ch_ppocr_rec import TextRecognizer  # noqa: E402

DET_MAX_EDGE = 3200  # config.json ocr.detMaxEdge
DET_PIXEL_BUDGET_MPX = 3.0  # services/ocr_service.rs DET_PIXEL_BUDGET
PRODUCT_TEXT_THRESHOLD = 0.6  # config.json ocr.textThreshold（当前仅有置信度语义）
DET_RAW_BOX_THRESH = 0.05  # S-11 测量用：让低分框进入统计，才能扫描 min_conf
DEFAULT_THREADS = 8

CONF_GRID = [0.0, 0.10, 0.20, 0.30, 0.40, 0.50, 0.55, 0.60, 0.65, 0.70, 0.75, 0.80, 0.85, 0.90]
AREA_GRID = [0, 8, 16, 24, 32, 40, 50, 64, 80, 100, 128, 160, 200, 256, 320, 400, 512, 700, 1000, 1500, 2500, 4000, 7000]
AMAX_GRID = [None, 3000, 4000, 5000, 6000, 8000, 10000, 15000, 30000, 50000]
HMAX_GRID = [None, 40, 50, 60, 70]
LINES_GRID = [1, 2, 3]


# ================================================================== 引擎（复用 RapidOCR）
class ScoredOcr:
    """RapidOCR det/rec + det 分数保留 + 可选 det 输入像素预算。

    只复用包内组件：TextDetector 的 preprocess_op / infer / DBPostProcess / 后处理；
    识别复用 OcrEngines.rec（rapidocr TextRecognizer）。
    """

    def __init__(self, threads: int = DEFAULT_THREADS):
        self.base = bench_ocr.OcrEngines(threads)  # det+rec 模型加载（真实 ONNX）
        self.det = self.base.det
        self.rec: TextRecognizer = self.base.rec
        self.threads = threads
        self.det_max_edge = DET_MAX_EDGE
        self._det_cfg = yaml.safe_load((bench_ocr.PKG_DIR / "config.yaml").read_text(encoding="utf-8"))["Det"]
        self.set_det_box_thresh(DET_RAW_BOX_THRESH)

    # -- 可调 det 后处理分数阈值（产品默认 = ocr.textThreshold）
    def set_det_box_thresh(self, box_thresh: float) -> None:
        self.det_box_thresh = float(box_thresh)
        self.det.postprocess_op = DBPostProcess(
            thresh=float(self._det_cfg.get("thresh", 0.3)),
            box_thresh=float(box_thresh),
            max_candidates=int(self._det_cfg.get("max_candidates", 1000)),
            unclip_ratio=float(self._det_cfg.get("unclip_ratio", 1.6)),
            use_dilation=bool(self._det_cfg.get("use_dilation", True)),
            score_mode=str(self._det_cfg.get("score_mode", "fast")),
        )

    # -- det 原始结果（保留 DB score；框坐标为输入图坐标）
    def det_raw(self, img: np.ndarray) -> Tuple[List[Tuple[np.ndarray, float]], Dict[str, Any], float]:
        h, w = img.shape[:2]
        t0 = time.perf_counter()
        self.det.preprocess_op = self.det.get_preprocess(max(h, w))
        pre = self.det.preprocess_op(img)
        if pre is None:
            return [], {"det_input_w": w, "det_input_h": h}, 0.0
        pre_h, pre_w = pre.shape[2], pre.shape[3]
        preds = self.det.infer(pre)[0]
        boxes, scores = self.det.postprocess_op(preds, (h, w))
        out: List[Tuple[np.ndarray, float]] = []
        for box, score in zip(boxes, scores):
            b = self.det.order_points_clockwise(np.asarray(box, dtype=np.float32))
            b = self.det.clip_det_res(b, h, w)
            rect_w = int(np.linalg.norm(b[0] - b[1]))
            rect_h = int(np.linalg.norm(b[0] - b[3]))
            if rect_w <= 3 or rect_h <= 3:  # 与 TextDetector.filter_tag_det_res 一致
                continue
            out.append((b, float(score)))
        det_ms = (time.perf_counter() - t0) * 1000.0
        meta = {"det_input_w": int(pre_w), "det_input_h": int(pre_h),
                "frame_w": w, "frame_h": h, "det_native_limit": True}
        return out, meta, det_ms

    # -- 生产预算口径 det：先按 min(长边 3200, 3.0Mpx) 降采样，再 det，框坐标还原到原图
    def det_budget(self, img: np.ndarray) -> Tuple[List[Tuple[np.ndarray, float]], Dict[str, Any], float]:
        h, w = img.shape[:2]
        tw, th, scale = bench_ocr.budget_target_size(w, h, DET_PIXEL_BUDGET_MPX * 1e6, self.det_max_edge)
        resized = img
        resize_ms = 0.0
        if scale < 1.0 and (tw, th) != (w, h):
            t0 = time.perf_counter()
            resized = cv2.resize(img, (tw, th), interpolation=cv2.INTER_AREA)
            resize_ms = (time.perf_counter() - t0) * 1000.0
        boxes, meta, det_ms = self.det_raw(resized)
        eff = min(tw / w, th / h)  # 与 det_input_size 的 effective scale 口径一致
        restored: List[Tuple[np.ndarray, float]] = []
        if eff < 1.0:
            for b, s in boxes:
                bb = b / eff
                bb[:, 0] = np.clip(bb[:, 0], 0, w - 1)
                bb[:, 1] = np.clip(bb[:, 1], 0, h - 1)
                restored.append((bb.astype(np.float32), s))
        else:
            restored = boxes
        meta.update({
            "frame_w": w, "frame_h": h,
            "det_input_w": meta["det_input_w"], "det_input_h": meta["det_input_h"],
            "pre_resize_w": tw, "pre_resize_h": th,
            "pre_resize_scale": round(float(eff), 4),
            "budget_mpx": DET_PIXEL_BUDGET_MPX,
            "resize_ms": round(resize_ms, 3),
        })
        return restored, meta, det_ms

    # -- 轴对齐裁框（记录原始下标，避免小框被丢后错位）
    @staticmethod
    def crops_with_index(img: np.ndarray, boxes_xyxy: List[Tuple[int, int, int, int]]
                         ) -> Tuple[List[np.ndarray], List[int]]:
        h, w = img.shape[:2]
        crops, idx = [], []
        for i, (x0, y0, x1, y1) in enumerate(boxes_xyxy):
            x0 = int(max(0, x0)); y0 = int(max(0, y0))
            x1 = int(min(w, x1)); y1 = int(min(h, y1))
            if x1 - x0 < 3 or y1 - y0 < 3:
                continue
            crops.append(img[y0:y1, x0:x1])
            idx.append(i)
        return crops, idx

    def det_rec(self, img: np.ndarray, use_budget: bool) -> Dict[str, Any]:
        """det(+可选预算) → 裁框 → rec；返回逐框 text/score 与拼接文本。"""
        boxes = self.det_budget(img)[0] if use_budget else self.det_raw(img)[0]
        xyxy = []
        for b, _s in boxes:
            x0 = int(np.floor(b[:, 0].min())); y0 = int(np.floor(b[:, 1].min()))
            x1 = int(np.ceil(b[:, 0].max())); y1 = int(np.ceil(b[:, 1].max()))
            xyxy.append((x0, y0, x1, y1))
        crops, kept_idx = self.crops_with_index(img, xyxy)
        rec_res: List[Tuple[str, float]] = []
        rec_ms = 0.0
        if crops:
            t0 = time.perf_counter()
            rec_res, _elapse = self.rec(crops)
            rec_ms = (time.perf_counter() - t0) * 1000.0
        items = []
        for k, (text, score) in zip(kept_idx, rec_res):
            b, det_score = boxes[k]
            items.append({
                "box": [float(b[:, 0].min()), float(b[:, 1].min()),
                        float(b[:, 0].max()), float(b[:, 1].max())],
                "det_score": round(float(det_score), 4),
                "text": text,
                "rec_score": round(float(score), 4),
            })
        items = [it for it in items if it["text"].strip()]
        return {"items": items, "n_det": len(boxes), "rec_ms": round(rec_ms, 1)}


# ================================================================== 文本归一化 / CER
def normalize_text(s: str) -> str:
    """CER 口径：NFKC（全角→半角等）+ 去除所有空白字符。"""
    s = unicodedata.normalize("NFKC", s)
    return "".join(ch for ch in s if not ch.isspace())


def edit_ops(truth: str, hyp: str) -> Tuple[int, int, int, int]:
    """返回 (distance, delete, insert, replace)。"""
    try:
        from rapidfuzz.distance import Levenshtein  # type: ignore

        ops = Levenshtein.editops(truth, hyp)
        deleted = sum(1 for o in ops if o.tag == "delete")
        inserted = sum(1 for o in ops if o.tag == "insert")
        replaced = sum(1 for o in ops if o.tag == "replace")
        return len(ops), deleted, inserted, replaced
    except Exception:  # noqa: BLE001 —— 没装 rapidfuzz 时的纯 Python 回退
        n, m = len(truth), len(hyp)
        d = [[0] * (m + 1) for _ in range(n + 1)]
        for i in range(n + 1):
            d[i][0] = i
        for j in range(m + 1):
            d[0][j] = j
        for i in range(1, n + 1):
            for j in range(1, m + 1):
                d[i][j] = min(d[i - 1][j] + 1, d[i][j - 1] + 1,
                              d[i - 1][j - 1] + (truth[i - 1] != hyp[j - 1]))
        i, j = n, m
        deleted = inserted = replaced = 0
        while i > 0 or j > 0:
            if i > 0 and j > 0 and d[i][j] == d[i - 1][j - 1] + (truth[i - 1] != hyp[j - 1]):
                if truth[i - 1] != hyp[j - 1]:
                    replaced += 1
                i -= 1; j -= 1
            elif i > 0 and d[i][j] == d[i - 1][j] + 1:
                deleted += 1; i -= 1
            else:
                inserted += 1; j -= 1
        return d[n][m], deleted, inserted, replaced


def order_items(items: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """阅读顺序：按中心 y 做视觉行分组（阈值 = 0.5×行高），行内按中心 x 排序。"""
    if not items:
        return []
    enriched = []
    for it in items:
        x0, y0, x1, y1 = it["box"]
        enriched.append({"it": it, "cx": (x0 + x1) / 2.0, "cy": (y0 + y1) / 2.0, "h": max(1.0, y1 - y0)})
    enriched.sort(key=lambda e: (e["cy"], e["cx"]))
    rows: List[List[Dict[str, Any]]] = []
    row_cy = None
    row_h = None
    for e in enriched:
        tol = 0.5 * max(row_h or e["h"], e["h"])
        if row_cy is None or e["cy"] - row_cy > tol:
            rows.append([e])
            row_cy = e["cy"]
            row_h = e["h"]
        else:
            rows[-1].append(e)
            row_cy = sum(x["cy"] for x in rows[-1]) / len(rows[-1])
            row_h = max(x["h"] for x in rows[-1])
    ordered = []
    for row in rows:
        row.sort(key=lambda e: e["cx"])
        ordered.extend(e["it"] for e in row)
    return ordered


def join_text(items: List[Dict[str, Any]]) -> str:
    return "\n".join(it["text"].strip() for it in order_items(items) if it["text"].strip())


# ================================================================== 数据加载
def load_manifest() -> Dict[str, Any]:
    if not MANIFEST.exists():
        raise SystemExit(f"缺少语料 manifest：{MANIFEST}\n请先运行 build_corpus.py")
    return json.loads(MANIFEST.read_text(encoding="utf-8"))


def entries_by_id(man: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    d = {}
    for e in man["real"] + man["synth"]:
        d[e["id"]] = e
    return d


def resolve_path(entry: Dict[str, Any]) -> Path:
    return REPO / entry["file"]


def s11_ids(man: Dict[str, Any], limit: int | None) -> List[str]:
    ids = man.get("s11_ids") or ([r["id"] for r in man["real"]] + [s["id"] for s in man["synth"]])
    if limit:
        # 小样本冒烟：按 real/synth 比例取前 N 张（保持两类都覆盖）
        real = [i for i in ids if i in man.get("s11_real_ids", [])]
        synth = [i for i in ids if i in man.get("s11_synth_ids", [])]
        half = max(1, limit // 2)
        ids = real[:half] + synth[:half]
    return ids


def _path_of(entry: Dict[str, Any]) -> Path:
    return resolve_path(entry)


# ================================================================== S-11 阶段 A：det
def box_metrics(boxes: List[Dict[str, Any]]) -> Dict[str, Any]:
    areas = [b["area"] for b in boxes]
    scores = [b["score"] for b in boxes]
    return {
        "n_boxes": len(boxes),
        "max_conf": round(max(scores), 4) if scores else 0.0,
        "max_area": round(max(areas), 1) if areas else 0.0,
        "sum_area": round(float(sum(areas)), 1) if areas else 0.0,
        "median_area": round(float(np.median(areas)), 1) if areas else 0.0,
        "n_conf_ge_0.6": int(sum(1 for s in scores if s >= PRODUCT_TEXT_THRESHOLD)),
        "n_area_ge_40": int(sum(1 for a in areas if a >= 40)),
    }


def run_det(args: argparse.Namespace) -> Dict[str, Any]:
    man = load_manifest()
    by_id = entries_by_id(man)
    ids = s11_ids(man, args.limit)
    eng = ScoredOcr(args.threads)
    # 记录时使用低 box_thresh，保证 min_conf 扫描有数据；同时按生产 box_thresh 再跑一遍会得到同样的高分框子集
    eng.set_det_box_thresh(DET_RAW_BOX_THRESH)
    out: Dict[str, Any] = {
        "stage": "s11_det",
        "ran_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "engine": {
            "rapidocr_onnxruntime": _pkg_version(),
            "det_model": "ch_PP-OCRv4_det_infer.onnx",
            "threads": args.threads,
            "det_box_thresh_measure": DET_RAW_BOX_THRESH,
            "product_box_thresh": PRODUCT_TEXT_THRESHOLD,
            "det_max_edge": DET_MAX_EDGE,
            "det_pixel_budget_mpx": DET_PIXEL_BUDGET_MPX,
            "profile": "整屏生产预算口径（长边≤3200 且 ≤3.0Mpx，不放大）",
        },
        "images": {},
    }
    t0 = time.time()
    for n, image_id in enumerate(ids):
        e = by_id[image_id]
        img = bench_ocr.load_bgr(_path_of(e))
        boxes_raw, meta, det_ms = eng.det_budget(img)
        boxes = []
        for b, score in boxes_raw:
            x0, y0 = float(b[:, 0].min()), float(b[:, 1].min())
            x1, y1 = float(b[:, 0].max()), float(b[:, 1].max())
            boxes.append({
                "x": round(x0, 1), "y": round(y0, 1), "w": round(max(0.0, x1 - x0), 1),
                "h": round(max(0.0, y1 - y0), 1),
                "area": round(max(0.0, x1 - x0) * max(0.0, y1 - y0), 1),
                "score": round(score, 4),
            })
        rec = {
            "id": image_id,
            "kind": e["kind"],
            "category": e["category"],
            "width": e["width"], "height": e["height"],
            "hasText": bool(e["hasText"]),
            "det": {**meta, "det_ms": round(det_ms, 1)},
            **box_metrics(boxes),
            "boxes": boxes,
        }
        out["images"][image_id] = rec
        print(f"[det {n+1:3d}/{len(ids)}] {image_id:14s} {e['kind']:5s} {e['category']:14s} "
              f"{e['width']}x{e['height']} 框={rec['n_boxes']:4d} maxConf={rec['max_conf']:.2f} "
              f"maxArea={rec['max_area']:8.0f}  {det_ms:6.0f}ms")
    out["elapsed_s"] = round(time.time() - t0, 1)
    save_json(RESULTS / "s11_boxes.json", out)
    print(f"[det] 用时 {out['elapsed_s']}s -> {RESULTS / 's11_boxes.json'}")
    return out


def _pkg_version() -> str:
    from importlib.metadata import version

    return version("rapidocr-onnxruntime")


# ================================================================== S-11 阶段 B：阈值扫描
def judge_batch(images: List[Dict[str, Any]], min_conf: float, min_area: float, min_lines: int,
                max_area: float | None = None, max_box_height: float | None = None) -> Dict[str, Any]:
    """对一批图应用阈值，返回混淆矩阵与逐图判定。

    判定：pass(box) = det_score >= min_conf 且 area >= min_area 且
                     (max_area is None or area <= max_area) 且
                     (max_box_height is None or box_h <= max_box_height)
          hasText = count(pass) >= min_lines
    """
    tp = fp = tn = fn = 0
    decisions = {}
    for rec in images:
        passed = 0
        for b in rec["boxes"]:
            if b["score"] < min_conf or b["area"] < min_area:
                continue
            if max_area is not None and b["area"] > max_area:
                continue
            if max_box_height is not None and b["h"] > max_box_height:
                continue
            passed += 1
        pred = passed >= min_lines
        truth = rec["hasText"]
        if truth and pred:
            tp += 1
        elif truth and not pred:
            fn += 1
        elif (not truth) and pred:
            fp += 1
        else:
            tn += 1
        decisions[rec["id"]] = {"pred": bool(pred), "passed_boxes": passed}
    n_pos = tp + fn
    n_neg = fp + tn
    return {
        "min_conf": min_conf, "min_area": min_area, "min_lines": min_lines,
        "max_area": max_area, "max_box_height": max_box_height,
        "tp": tp, "fp": fp, "tn": tn, "fn": fn,
        "n_pos": n_pos, "n_neg": n_neg,
        "fp_rate": round(fp / n_neg, 4) if n_neg else None,
        "fn_rate": round(fn / n_pos, 4) if n_pos else None,
        "accuracy": round((tp + tn) / max(1, len(images)), 4),
        "decisions": decisions,
    }


def _combo_row(res: Dict[str, Any]) -> Dict[str, Any]:
    keys = ("min_conf", "min_area", "max_area", "min_lines", "max_box_height",
            "tp", "fp", "tn", "fn", "fp_rate", "fn_rate", "accuracy")
    return {k: res[k] for k in keys}


def run_sweep(args: argparse.Namespace) -> Dict[str, Any]:
    path = RESULTS / "s11_boxes.json"
    if not path.exists():
        raise SystemExit(f"缺少检测缓存 {path}，请先运行 `run_ocr_eval.py det`")
    det = json.loads(path.read_text(encoding="utf-8"))
    images = [det["images"][k] for k in det["images"]]
    images.sort(key=lambda r: (0 if r["kind"] == "real" else 1, r["category"], r["id"]))
    n_pos = sum(1 for r in images if r["hasText"])
    n_neg = sum(1 for r in images if not r["hasText"])

    def valid(rows: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        out = []
        for c in rows:
            if c["fp_rate"] is not None and c["fn_rate"] is not None \
                    and c["fp_rate"] <= 0.05 and c["fn_rate"] <= 0.05:
                cc = dict(c)
                cc["margin"] = round(min(0.05 - c["fp_rate"], 0.05 - c["fn_rate"]), 4)
                cc["simplicity"] = round(c["min_area"] / 7000.0 + c["min_conf"] * 0.2
                                         + (c["min_lines"] - 1) * 0.05
                                         + (0.0 if c["max_area"] is None else c["max_area"] / 50000.0), 4)
                out.append(cc)
        return out

    # --- 家族 A：只有面积下限（清单要求的三阈值原义）
    family_a = []
    for lines in LINES_GRID:
        for conf in CONF_GRID:
            for area in AREA_GRID:
                family_a.append(_combo_row(judge_batch(images, conf, area, lines)))
    valid_a = valid(family_a)
    # 家族 A 的「最接近」候选（按 max(FP,FN) 最小）
    best_a_pareto = sorted(family_a, key=lambda c: (max(c["fp_rate"], c["fn_rate"]),
                                                    c["fp_rate"] + c["fn_rate"]))[0]

    # --- 家族 B：面积区间（min_area..max_area），仍在「面积」这一个维度内
    family_b = []
    for lines in (1, 2):
        for conf in (0.5, 0.55, 0.6, 0.65, 0.7, 0.75):
            for amin in (0, 100, 200, 300, 400, 500, 700, 1000):
                for amax in AMAX_GRID:
                    family_b.append(_combo_row(judge_batch(images, conf, amin, lines, max_area=amax)))
    valid_b = sorted(valid(family_b), key=lambda c: (-c["margin"], c["simplicity"]))
    best_b = valid_b[0] if valid_b else None

    # --- 家族 C：面积下限 + 行高上限（第 4 个几何阈值，推荐增强）
    family_c = []
    for lines in (1, 2):
        for conf in (0.5, 0.55, 0.6, 0.65, 0.7):
            for amin in (0, 100, 200, 300, 400, 500, 700):
                for hmax in HMAX_GRID:
                    family_c.append(_combo_row(judge_batch(images, conf, amin, lines, max_box_height=hmax)))
    # 选择原则（写进记录）：满足 FP≤5%/FN≤5% 的候选按以下优先级取：
    #   ① FN 更低（宁可不漏） ② 置信度 = 产品默认 0.6 ③ 行数 = 1（单行页面不漏判）
    #   ④ 行高上限更大（大字号不漏） ⑤ 面积下限在 [300,500]（过滤碎片同时保留小字）
    #   ⑥ 误判率更低 ⑦ 参数更简单
    def _c_key(c: Dict[str, Any]):
        amin = c["min_area"]
        in_band = 0 if 300 <= amin <= 500 else (1 if amin >= 500 else 2)
        return (c["fn_rate"], abs(c["min_conf"] - PRODUCT_TEXT_THRESHOLD), c["min_lines"] - 1,
                -(c["max_box_height"] or 0), in_band, c["fp_rate"], c["simplicity"])

    valid_c = sorted(valid(family_c), key=_c_key)
    best_c = valid_c[0] if valid_c else None

    if best_c is None and best_b is None:
        raise SystemExit("两族组合都没有同时满足 误判≤5% 且 漏判≤5%")

    # 推荐：优先家族 C（达标且余量最大）；否则退回家族 B
    recommended = best_c or best_b
    recommended_family = "C: 面积下限 + 行高上限（需新增行高配置）" if best_c else "B: 面积区间（三阈值内可达标）"

    # 指定组合（参数化验证）：命令行参数优先，否则用推荐值
    judge_conf = args.min_confidence if args.min_confidence is not None else recommended["min_conf"]
    judge_area = args.min_area if args.min_area is not None else recommended["min_area"]
    judge_lines = args.min_lines if args.min_lines is not None else recommended["min_lines"]
    judge_amax = args.max_area if getattr(args, "max_area", None) is not None else recommended["max_area"]
    judge_hmax = args.max_box_height if getattr(args, "max_box_height", None) is not None \
        else recommended["max_box_height"]
    judged = judge_batch(images, judge_conf, judge_area, judge_lines,
                         max_area=judge_amax, max_box_height=judge_hmax)

    # 基线：产品现状（仅 ocr.textThreshold=0.6 置信度，无面积/行数/行高阈值）
    baseline = judge_batch(images, PRODUCT_TEXT_THRESHOLD, 0, 1)

    # 分类型统计（真实截图按内容类型；合成按负样本类型）
    per_cat: Dict[str, Any] = {}
    for rec in images:
        cat = rec["category"]
        d = per_cat.setdefault(cat, {"kind": rec["kind"], "n": 0, "n_pos": 0, "n_neg": 0,
                                     "fp": 0, "fn": 0, "judged": 0, "baseline_fp": 0, "baseline_fn": 0})
        d["n"] += 1
        d["n_pos"] += int(rec["hasText"])
        d["n_neg"] += int(not rec["hasText"])
        truth = rec["hasText"]
        pred = judged["decisions"][rec["id"]]["pred"]
        if pred:
            d["judged"] += 1
        if pred and not truth:
            d["fp"] += 1
        if truth and not pred:
            d["fn"] += 1
        bpred = baseline["decisions"][rec["id"]]["pred"]
        if bpred and not truth:
            d["baseline_fp"] += 1
        if truth and not bpred:
            d["baseline_fn"] += 1
    for cat, d in per_cat.items():
        d["fp_rate"] = round(d["fp"] / d["n_neg"], 4) if d["n_neg"] else None
        d["fn_rate"] = round(d["fn"] / d["n_pos"], 4) if d["n_pos"] else None
        d["baseline_fp_rate"] = round(d["baseline_fp"] / d["n_neg"], 4) if d["n_neg"] else None
        d["baseline_fn_rate"] = round(d["baseline_fn"] / d["n_pos"], 4) if d["n_pos"] else None

    # 失败样本
    fp_images = [{"id": r["id"], "category": r["category"], "n_boxes": r["n_boxes"],
                  "max_conf": r["max_conf"], "max_area": r["max_area"],
                  "passed": judged["decisions"][r["id"]]["passed_boxes"],
                  "passed_boxes": [{"w": b["w"], "h": b["h"], "area": b["area"], "score": b["score"]}
                                   for b in r["boxes"]
                                   if b["score"] >= judge_conf and b["area"] >= judge_area
                                   and (judge_amax is None or b["area"] <= judge_amax)
                                   and (judge_hmax is None or b["h"] <= judge_hmax)]}
                 for r in images if not r["hasText"] and judged["decisions"][r["id"]]["pred"]]
    fn_images = [{"id": r["id"], "category": r["category"], "n_boxes": r["n_boxes"],
                  "max_conf": r["max_conf"], "max_area": r["max_area"],
                  "passed": judged["decisions"][r["id"]]["passed_boxes"]}
                 for r in images if r["hasText"] and not judged["decisions"][r["id"]]["pred"]]

    # 2D 扫描曲线：推荐家族的 hmax/amax 固定，conf × min_area 的 FP/FN（%）
    curve = []
    for conf in CONF_GRID:
        row = {"min_conf": conf}
        for area in AREA_GRID:
            r = judge_batch(images, conf, area, judge_lines,
                            max_area=judge_amax, max_box_height=judge_hmax)
            row[f"fp_area_{area}"] = round(r["fp_rate"] * 100, 1) if r["fp_rate"] is not None else None
            row[f"fn_area_{area}"] = round(r["fn_rate"] * 100, 1) if r["fn_rate"] is not None else None
        curve.append(row)

    summary = {
        "stage": "s11_sweep",
        "ran_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "threshold_semantics": (
            "pass(box)=det_score>=min_conf 且 area>=min_area 且 (max_area 空或 area<=max_area) "
            "且 (max_box_height 空或 box_h<=max_box_height)；hasText=count(pass)>=min_lines"
        ),
        "n_images": len(images),
        "n_pos": n_pos,
        "n_neg": n_neg,
        "det_box_thresh_measure": det["engine"]["det_box_thresh_measure"],
        "product_box_thresh": PRODUCT_TEXT_THRESHOLD,
        "recommended": {**recommended, "family": recommended_family},
        "alternative_area_band": best_b,
        "three_threshold_best_effort": {
            **best_a_pareto,
            "note": "只设面积下限的三阈值原义组合，误判率无法压到 5% 以内（最佳 max(FP,FN)）",
        },
        "valid_combos": {
            "area_band": len(valid_b), "area_plus_height": len(valid_c),
        },
        "top_valid": {"area_band": valid_b[:8], "area_plus_height": valid_c[:8]},
        "baseline_product_conf_only": {k: baseline[k] for k in ("min_conf", "min_area", "min_lines",
                                                                "tp", "fp", "tn", "fn", "fp_rate", "fn_rate", "accuracy")},
        "judged": {k: judged[k] for k in ("min_conf", "min_area", "max_area", "min_lines", "max_box_height",
                                         "tp", "fp", "tn", "fn", "fp_rate", "fn_rate", "accuracy")},
        "per_category": per_cat,
        "failures": {"false_positive": fp_images, "false_negative": fn_images},
        "curve": {"fixed": {"max_area": judge_amax, "max_box_height": judge_hmax, "min_lines": judge_lines},
                  "rows": curve},
    }
    save_json(RESULTS / "s11_sweep.json", summary)
    save_csv(RESULTS / "s11_sweep_all_combos.csv",
             family_a + family_b + family_c,
             ["min_conf", "min_area", "max_area", "min_lines", "max_box_height",
              "tp", "fp", "tn", "fn", "fp_rate", "fn_rate", "accuracy"])
    # 逐图 CSV：推荐组合 + 基线 + 真值
    rows = []
    for r in images:
        d = judged["decisions"][r["id"]]
        b = baseline["decisions"][r["id"]]
        rows.append({
            "image_id": r["id"], "kind": r["kind"], "category": r["category"],
            "width": r["width"], "height": r["height"],
            "hasText_truth": int(r["hasText"]),
            "n_boxes": r["n_boxes"], "max_conf": r["max_conf"], "max_area": r["max_area"],
            "median_area": r["median_area"], "sum_area": r["sum_area"],
            "n_conf_ge_0.6": r["n_conf_ge_0.6"],
            "passed_boxes_recommended": d["passed_boxes"],
            "hasText_pred_recommended": int(d["pred"]),
            "correct_recommended": int(d["pred"] == r["hasText"]),
            "hasText_pred_baseline": int(b["pred"]),
            "correct_baseline": int(b["pred"] == r["hasText"]),
        })
    save_csv(RESULTS / "s11_per_image.csv", rows, list(rows[0].keys()))
    print(f"[sweep] 推荐（{recommended_family}）：conf>={judge_conf} area>={judge_area} "
          f"max_area={judge_amax} hmax={judge_hmax} lines>={judge_lines} | "
          f"FP={judged['fp_rate']*100:.1f}% FN={judged['fn_rate']*100:.1f}%")
    print(f"[sweep] 基线 conf-only：FP={baseline['fp_rate']*100:.1f}% FN={baseline['fn_rate']*100:.1f}% | "
          f"三阈值最佳 FP={best_a_pareto['fp_rate']*100:.1f}% FN={best_a_pareto['fn_rate']*100:.1f}% | "
          f"有效组合：band={len(valid_b)} area+h={len(valid_c)}")
    return summary


# ================================================================== S-15：det+rec + CER
def s15_profiles(arg: str) -> List[str]:
    allowed = {"native", "budget"}
    out = [p.strip() for p in arg.split(",") if p.strip()]
    bad = [p for p in out if p not in allowed]
    if bad:
        raise SystemExit(f"未知 profile：{bad}，可选 {sorted(allowed)}")
    return out or ["native", "budget"]


def run_ocr(args: argparse.Namespace) -> Dict[str, Any]:
    man = load_manifest()
    by_id = entries_by_id(man)
    ids = [r["id"] for r in man["real"]]
    if args.limit:
        ids = ids[:args.limit]
    profiles = s15_profiles(args.profiles)
    eng = ScoredOcr(args.threads)
    eng.set_det_box_thresh(PRODUCT_TEXT_THRESHOLD)  # 与产品一致：det 只保留 ≥0.6 的框
    out: Dict[str, Any] = {
        "stage": "s15_ocr",
        "ran_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "engine": {
            "rapidocr_onnxruntime": _pkg_version(),
            "det_model": "ch_PP-OCRv4_det_infer.onnx",
            "rec_model": "ch_PP-OCRv4_rec_infer.onnx",
            "threads": args.threads,
            "det_box_thresh": PRODUCT_TEXT_THRESHOLD,
            "text_threshold": PRODUCT_TEXT_THRESHOLD,
            "det_max_edge": DET_MAX_EDGE,
            "det_pixel_budget_mpx": DET_PIXEL_BUDGET_MPX,
        },
        "normalization": "NFKC; 去除所有空白字符；按视觉行分组（y 差>0.5×行高换行）+ 行内 x 排序拼接",
        "profiles": profiles,
        "images": {},
    }
    t0 = time.time()
    for n, image_id in enumerate(ids):
        e = by_id[image_id]
        img = bench_ocr.load_bgr(_path_of(e))
        truth = normalize_text(e["truth_text"])
        rec_entry: Dict[str, Any] = {
            "id": image_id, "category": e["category"], "width": e["width"], "height": e["height"],
            "truth_chars": len(truth), "profiles": {},
        }
        for prof in profiles:
            use_budget = prof == "budget"
            r = eng.det_rec(img, use_budget)
            for variant, items in (("all", r["items"]),
                                   ("rec_conf_0.6", [it for it in r["items"] if it["rec_score"] >= PRODUCT_TEXT_THRESHOLD])):
                hyp_raw = join_text(items)
                hyp = normalize_text(hyp_raw)
                dist, deleted, inserted, replaced = edit_ops(truth, hyp)
                rec_entry["profiles"].setdefault(prof, {})[variant] = {
                    "n_det_boxes": r["n_det"],
                    "n_items": len(items),
                    "hyp_chars": len(hyp),
                    "distance": dist,
                    "cer": round(dist / max(1, len(truth)), 4),
                    "accuracy": round(1 - dist / max(1, len(truth)), 4),
                    "delete": deleted, "insert": inserted, "replace": replaced,
                    "hyp_text": hyp_raw,
                }
            rec_entry["profiles"][prof]["rec_ms"] = r["rec_ms"]
        out["images"][image_id] = rec_entry
        prim = rec_entry["profiles"][profiles[0]]["rec_conf_0.6"]
        print(f"[ocr {n+1:3d}/{len(ids)}] {image_id:14s} {e['category']:5s} 真值={len(truth):4d} "
              f"{profiles[0]}: CER={prim['cer']*100:5.1f}% acc={prim['accuracy']*100:5.1f}% "
              f"del={prim['delete']} ins={prim['insert']} sub={prim['replace']}")
    out["elapsed_s"] = round(time.time() - t0, 1)
    save_json(RESULTS / "s15_ocr.json", out)
    # 逐图 CSV（两个 profile × 两个 variant 展平）
    rows = []
    for image_id, rec in out["images"].items():
        for prof in profiles:
            for variant in ("all", "rec_conf_0.6"):
                v = rec["profiles"][prof][variant]
                rows.append({
                    "image_id": image_id, "category": rec["category"], "width": rec["width"],
                    "height": rec["height"], "profile": prof, "variant": variant,
                    "truth_chars": rec["truth_chars"], "n_det_boxes": v["n_det_boxes"],
                    "n_items": v["n_items"], "hyp_chars": v["hyp_chars"],
                    "distance": v["distance"], "cer": v["cer"], "accuracy": v["accuracy"],
                    "delete": v["delete"], "insert": v["insert"], "replace": v["replace"],
                })
    save_csv(RESULTS / "s15_per_image.csv", rows, list(rows[0].keys()))
    print(f"[ocr] 用时 {out['elapsed_s']}s -> {RESULTS / 's15_ocr.json'}")
    return out


# ================================================================== 汇总
def aggregate_s15(ocr: Dict[str, Any], profiles: List[str]) -> Dict[str, Any]:
    def agg(profile: str, variant: str) -> Dict[str, Any]:
        imgs = [(i, r) for i, r in ocr["images"].items()]
        dist = sum(r["profiles"][profile][variant]["distance"] for _, r in imgs)
        chars = sum(r["truth_chars"] for _, r in imgs)
        accs = [r["profiles"][profile][variant]["accuracy"] for _, r in imgs]
        cers = [r["profiles"][profile][variant]["cer"] for _, r in imgs]
        del_ = sum(r["profiles"][profile][variant]["delete"] for _, r in imgs)
        ins_ = sum(r["profiles"][profile][variant]["insert"] for _, r in imgs)
        sub_ = sum(r["profiles"][profile][variant]["replace"] for _, r in imgs)
        return {
            "profile": profile, "variant": variant,
            "n_images": len(imgs), "truth_chars": chars, "distance": dist,
            "accuracy_global": round(1 - dist / max(1, chars), 4),
            "cer_global": round(dist / max(1, chars), 4),
            "image_accuracy_mean": round(float(np.mean(accs)), 4),
            "image_accuracy_min": round(float(np.min(accs)), 4),
            "image_cer_p50": round(float(np.percentile(cers, 50)), 4),
            "image_cer_p95": round(float(np.percentile(cers, 95)), 4),
            "images_ge_95pct": int(sum(1 for a in accs if a >= 0.95)),
            "errors": {"delete": del_, "insert": ins_, "replace": sub_,
                       "delete_pct": round(100 * del_ / max(1, dist), 1),
                       "insert_pct": round(100 * ins_ / max(1, dist), 1),
                       "replace_pct": round(100 * sub_ / max(1, dist), 1)},
        }

    per_cat: Dict[str, Any] = {}
    for cat in ["web", "chat", "doc", "code", "dark"]:
        imgs = [(i, r) for i, r in ocr["images"].items() if r["category"] == cat]
        if not imgs:
            continue
        d: Dict[str, Any] = {"n_images": len(imgs),
                             "truth_chars": sum(r["truth_chars"] for _, r in imgs)}
        for profile in profiles:
            for variant in ("all", "rec_conf_0.6"):
                dist = sum(r["profiles"][profile][variant]["distance"] for _, r in imgs)
                chars = sum(r["truth_chars"] for _, r in imgs)
                accs = [r["profiles"][profile][variant]["accuracy"] for _, r in imgs]
                d[f"{profile}__{variant}"] = {
                    "accuracy_global": round(1 - dist / max(1, chars), 4),
                    "distance": dist,
                    "image_accuracy_mean": round(float(np.mean(accs)), 4),
                    "image_accuracy_min": round(float(np.min(accs)), 4),
                }
        per_cat[cat] = d

    # 失败样本：按主 profile 的 rec_conf_0.6 CER 降序
    primary = profiles[0]
    fails = []
    for image_id, r in ocr["images"].items():
        v = r["profiles"][primary]["rec_conf_0.6"]
        item = {"image_id": image_id, "category": r["category"], "truth_chars": r["truth_chars"],
                **{k: v[k] for k in ("distance", "cer", "accuracy", "delete", "insert", "replace")}}
        for p in profiles:
            item[f"cer_{p}"] = r["profiles"][p]["rec_conf_0.6"]["cer"]
            item[f"accuracy_{p}"] = r["profiles"][p]["rec_conf_0.6"]["accuracy"]
        fails.append(item)
    fails.sort(key=lambda x: (-x["cer"], -x["distance"]))

    # 两口径影响：逐图 accuracy(native) - accuracy(budget)
    profile_delta = None
    if "native" in profiles and "budget" in profiles:
        deltas = []
        for image_id, r in ocr["images"].items():
            n = r["profiles"]["native"]["rec_conf_0.6"]["accuracy"]
            b = r["profiles"]["budget"]["rec_conf_0.6"]["accuracy"]
            deltas.append({"image_id": image_id, "category": r["category"],
                           "acc_native": round(n, 4), "acc_budget": round(b, 4),
                           "delta_native_minus_budget": round(n - b, 4)})
        ds = np.asarray([d["delta_native_minus_budget"] for d in deltas])
        profile_delta = {
            "metric": "accuracy(native, rec_conf_0.6) - accuracy(budget, rec_conf_0.6)，正数=预算口径更差",
            "n": len(deltas),
            "mean": round(float(ds.mean()), 4),
            "min": round(float(ds.min()), 4),
            "max": round(float(ds.max()), 4),
            "n_same": int((np.abs(ds) < 1e-9).sum()),
            "n_budget_worse": int((ds > 1e-9).sum()),
            "n_budget_better": int((ds < -1e-9).sum()),
            "images_budget_better": sorted([d for d in deltas if d["delta_native_minus_budget"] < -1e-9],
                                           key=lambda d: d["delta_native_minus_budget"])[:5],
            "images_budget_worse": sorted([d for d in deltas if d["delta_native_minus_budget"] > 1e-9],
                                          key=lambda d: -d["delta_native_minus_budget"])[:8],
            "rows": deltas,
        }

    return {
        "profiles": profiles,
        "overall": {f"{p}__{v}": agg(p, v) for p in profiles for v in ("all", "rec_conf_0.6")},
        "per_category": per_cat,
        "worst_images": fails[:15],
        "profile_delta": profile_delta,
    }


def build_docs_summary(s11: Dict[str, Any], s15: Dict[str, Any], man: Dict[str, Any]) -> Dict[str, Any]:
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    corpus_index = {
        "generated_at": man["generated_at"],
        "counts": {"real": len(man["real"]), "synth": len(man["synth"])},
        "categories": {},
        "s11_ids": man.get("s11_ids"),
        "s15_ids": man.get("s15_ids"),
        "conventions": man["conventions"],
        "real": [{k: e[k] for k in ("id", "category", "width", "height", "truth_chars_nonspace",
                                    "sparse_design", "sha256")} |
                 {"dom_valid": e["render"]["valid"], "dom_inner_match": e["render"]["inner_match"]}
                 for e in man["real"]],
        "synth": [{k: e[k] for k in ("id", "category", "width", "height", "method", "sha256")}
                  for e in man["synth"]],
    }
    for e in man["real"]:
        d = corpus_index["categories"].setdefault(e["category"], {"n": 0, "sizes": set()})
        d["n"] += 1
        d["sizes"].add(f"{e['width']}x{e['height']}")
    for k, v in corpus_index["categories"].items():
        v["sizes"] = sorted(v["sizes"])
    save_json(DATA_DIR / "corpus_index.json", corpus_index)

    summary = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "s11": {
            "n_images": s11["n_images"], "n_pos": s11["n_pos"], "n_neg": s11["n_neg"],
            "threshold_semantics": s11["threshold_semantics"],
            "recommended": s11["recommended"],
            "alternative_area_band": s11.get("alternative_area_band"),
            "three_threshold_best_effort": s11.get("three_threshold_best_effort"),
            "baseline_product_conf_only": s11["baseline_product_conf_only"],
            "judged": s11["judged"],
            "per_category": s11["per_category"],
            "failures": s11["failures"],
            "valid_combos": s11.get("valid_combos"),
            "top_valid": s11.get("top_valid"),
            "det_box_thresh_measure": s11["det_box_thresh_measure"],
            "product_box_thresh": s11["product_box_thresh"],
            "engine": det_engine_from_boxes(),
            "artifacts": {
                "per_image_csv": ".verify/spikes/S-11-S-15/results/s11_per_image.csv",
                "sweep_all_combos_csv": ".verify/spikes/S-11-S-15/results/s11_sweep_all_combos.csv",
                "sweep_json": ".verify/spikes/S-11-S-15/results/s11_sweep.json",
                "boxes_json": ".verify/spikes/S-11-S-15/results/s11_boxes.json",
            },
        },
        "s15": {
            "n_images": len(s15["images"]),
            "engine": s15["engine"],
            "normalization": s15["normalization"],
            "reading_order": "按中心 y 做视觉行分组（相邻 y 差 > 0.5×行高换行），行内按中心 x 排序，行间以 \\n 拼接；"
                             "随后 NFKC + 去空白再算编辑距离",
            "profiles": s15["profiles"],
            "overall": s15["overall"],
            "per_category": s15["per_category"],
            "worst_images": s15["worst_images"],
            "profile_delta": s15.get("profile_delta"),
            "artifacts": {
                "per_image_csv": ".verify/spikes/S-11-S-15/results/s15_per_image.csv",
                "ocr_json": ".verify/spikes/S-11-S-15/results/s15_ocr.json",
            },
        },
        "corpus": {
            "n_real": len(man["real"]), "n_synth": len(man["synth"]),
            "real_categories": {c: sum(1 for e in man["real"] if e["category"] == c) for c in
                                ["web", "chat", "doc", "code", "dark"]},
            "dom_valid": sum(1 for e in man["real"] if e["render"]["valid"]),
            "index": "docs/spikes/data/s11_s15/corpus_index.json",
        },
    }
    save_json(DATA_DIR / "summary.json", summary)
    print(f"[docs] 聚合数据 -> {DATA_DIR / 'summary.json'}")
    return summary


def det_engine_from_boxes() -> Dict[str, Any]:
    try:
        d = json.loads((RESULTS / "s11_boxes.json").read_text(encoding="utf-8"))
        return d["engine"]
    except Exception:  # noqa: BLE001
        return {}


# ================================================================== CLI
def main() -> None:
    ap = argparse.ArgumentParser(description="S-11 文字存在判定阈值 + S-15 OCR 字符准确率评测")
    sub = ap.add_subparsers(dest="cmd", required=True)

    def common(p: argparse.ArgumentParser) -> None:
        p.add_argument("--threads", type=int, default=DEFAULT_THREADS)
        p.add_argument("--limit", type=int, default=0, help="小样本冒烟：只跑前 N 张")
        p.add_argument("--corpus-manifest", type=str, default="")
        p.add_argument("--min-area", type=float, default=None, help="S-11 面积下限（px²/框，100%% 缩放口径）")
        p.add_argument("--max-area", type=float, default=None, help="S-11 面积上限（px²/框，可选）")
        p.add_argument("--min-confidence", type=float, default=None, help="S-11 det 置信度阈值")
        p.add_argument("--min-lines", type=int, default=None, help="S-11 行数阈值")
        p.add_argument("--max-box-height", type=float, default=None, help="S-11 单个文字行框高上限（px，100%% 缩放口径）")

    for name, help_text in (("det", "S-11 阶段 A：逐图 det 并缓存框"),
                            ("sweep", "S-11 阶段 B：阈值网格扫描"),
                            ("judge", "S-11：评估指定阈值组合"),
                            ("ocr", "S-15：det+rec+CER"),
                            ("summary", "从已有 det/sweep/ocr 结果重新生成 docs 聚合数据（不重跑推理）"),
                            ("all", "全部跑通并汇总")):
        p = sub.add_parser(name, help=help_text)
        common(p)
        if name in ("ocr", "summary"):
            p.add_argument("--profiles", type=str, default="native,budget")

    args = ap.parse_args()
    RESULTS.mkdir(parents=True, exist_ok=True)
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    global MANIFEST
    if getattr(args, "corpus_manifest", ""):
        MANIFEST = Path(args.corpus_manifest)

    if args.cmd == "det":
        run_det(args)
    elif args.cmd in ("sweep", "judge"):
        run_sweep(args)
    elif args.cmd == "ocr":
        run_ocr(args)
    elif args.cmd == "summary":
        s11 = json.loads((RESULTS / "s11_sweep.json").read_text(encoding="utf-8"))
        ocr = json.loads((RESULTS / "s15_ocr.json").read_text(encoding="utf-8"))
        profiles = s15_profiles(args.profiles)
        s15 = aggregate_s15(ocr, profiles)
        build_docs_summary(s11, {"images": ocr["images"], "engine": ocr["engine"],
                                 "normalization": ocr["normalization"], **s15}, load_manifest())
    else:
        det = run_det(args)
        s11 = run_sweep(args)
        ocr = run_ocr(args)
        profiles = s15_profiles("native,budget")
        s15 = aggregate_s15(ocr, profiles)
        build_docs_summary(s11, {"images": ocr["images"], "engine": ocr["engine"],
                                 "normalization": ocr["normalization"], **s15}, load_manifest())


if __name__ == "__main__":
    main()
