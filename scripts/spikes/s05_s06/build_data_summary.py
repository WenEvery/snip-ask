#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""汇总 S-05/S-06 基准原始数据 → docs/spikes/data/s05_s06/ 与聚合 JSON。

只做「读取 raw → 复制精选文件 → 聚合关键数字」，不修改任何测量结果。
用法：python scripts/spikes/s05_s06/build_data_summary.py
"""

from __future__ import annotations

import glob
import hashlib
import json
import shutil
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
RAW = REPO_ROOT / ".verify" / "spikes" / "S-05-S-06" / "raw"
DATA = REPO_ROOT / "docs" / "spikes" / "data" / "s05_s06"

COPY_FILES = [
    "cloud_blocked_evidence.json",
    "deepseek_empty_key_body.json",
    "dashscope_empty_key_body.json",
    "cloud_translate_selftest_final.json",
    "local_translate_formal_20261105-000321.json",
    "local_translate_sens100_20261105-000321.json",
    "formal_local_summary_20261105-000321.json",
    "tune_threads6_20261105-000321.json",
    "tune_threads16_20261105-000321.json",
    "local_translate_20261104-235408.json",
    "local_translate_20261104-235614.json",
    "image_search_mock_20261104-235528.json",
    "formal_local_20261105-000321.log",
    "download.log",
]


def load(path: Path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def main() -> None:
    DATA.mkdir(parents=True, exist_ok=True)
    for name in COPY_FILES:
        src = RAW / name
        if src.exists():
            shutil.copy2(src, DATA / name)
    # 最新 image_search mock（文件名带时间戳时同步复制最后一份）
    for f in sorted(RAW.glob("image_search_mock_*.json")):
        shutil.copy2(f, DATA / f.name)

    formal = load(RAW / "local_translate_formal_20261105-000321.json")
    sens = load(RAW / "local_translate_sens100_20261105-000321.json")
    formal_sum = load(RAW / "formal_local_summary_20261105-000321.json")
    tune6 = load(RAW / "tune_threads6_20261105-000321.json")
    tune16 = load(RAW / "tune_threads16_20261105-000321.json")
    contaminated = load(RAW / "local_translate_20261104-235408.json")
    contaminated2 = load(RAW / "local_translate_20261104-235614.json")

    def brief(doc):
        return {
            "measured_runs": doc["summary"]["measured_runs"],
            "failures": doc["summary"]["failures"],
            "total_ms": doc["summary"]["total_ms"],
            "server_prompt_ms": doc["summary"]["server_prompt_ms"],
            "server_predicted_ms": doc["summary"]["server_predicted_ms"],
            "tok_per_sec": doc["summary"]["tok_per_sec"],
            "completion_tokens": doc["summary"]["completion_tokens"],
        }

    p50 = formal["summary"]["total_ms"]["p50_ms"]
    p95 = formal["summary"]["total_ms"]["p95_ms"]
    prefill = formal["summary"]["server_prompt_ms"]["p50_ms"]
    tps = formal["summary"]["tok_per_sec"]["p50_ms"]
    estimate = {
        str(n): round(prefill + (n / tps) * 1000.0, 1) for n in (100, 150, 200, 300)
    }

    summary = {
        "generated_at": datetime.now(timezone(timedelta(hours=8))).isoformat(),
        "repo_commit": formal["meta"].get("repo_commit"),
        "environment": {
            "os": "Windows 11 专业版 Build 26200",
            "cpu": "12th Gen Intel Core i7-12650H（10 核 16 线程，6P+4E）",
            "ram_bytes": 16944848896,
            "gpu": "NVIDIA GeForce RTX 4060 Laptop GPU（本次未使用，llama.cpp ngl=0 CPU 推理）",
            "python": "3.11.3",
            "node": "24.14.1",
            "date": "2026-11-04",
        },
        "s05": {
            "threshold_cloud_ms": 1500,
            "threshold_local_ms": 8000,
            "cloud": {
                "verdict": "BLOCKED",
                "reason": "本机无任何云端 API Key；DeepSeek 空 Key 实测 401、DashScope 空 Key 400",
                "blocked_evidence": "cloud_blocked_evidence.json",
                "mock_self_check": "cloud_translate_selftest_final.json（MOCK，不作为验收依据）",
                "unlock": [
                    "设置 SNIPASK_BENCH_BASE_URL / SNIPASK_BENCH_MODEL / SNIPASK_BENCH_API_KEY",
                    "python scripts/spikes/s05_s06/bench_translate.py --runs 20 --warmup 2",
                ],
            },
            "local": {
                "verdict": "FAILED",
                "model": {
                    "id": "hy-mt-1.8b-q4",
                    "file": "HY-MT1.5-1.8B-Q4_K_M.gguf",
                    "size_bytes": formal["meta"].get("model_size_bytes"),
                    "sha256": formal["meta"].get("model_sha256"),
                    "source": "ModelScope Tencent-Hunyuan/HY-MT1.5-1.8B-GGUF（manifest 原 URL 404，已修正文件名大小写）",
                },
                "engine": {
                    "exe": formal["meta"].get("exe"),
                    "version": formal["meta"].get("exe_version"),
                    "provenance": "本机 LM Studio 内置 llama.cpp 后端 llama.cpp-win-x86_64-avx2-2.28.2（llama-server 前端，MSVC 19.44 构建）",
                    "args": formal["meta"].get("server_args"),
                },
                "sampling": formal["meta"].get("sampling"),
                "cold_load_ms": formal["meta"].get("health_ready_ms"),
                "memory": {
                    "after_load": formal["meta"].get("memory_after_load"),
                    "after_runs": formal["meta"].get("memory_after_runs"),
                },
                "formal_10_runs_threads16": brief(formal),
                "thread_tuning": {
                    "threads_6": brief(tune6),
                    "threads_16": brief(tune16),
                },
                "max_tokens_100_sensitivity": brief(sens),
                "estimate_total_ms_by_completion_tokens": estimate,
                "contaminated_runs": {
                    "temp0.7_parallel_load": brief(contaminated),
                    "temp0.2_parallel_load": brief(contaminated2),
                },
                "load_notes": {
                    "quiet_window_threshold_percent": formal_sum.get("quiet_load_threshold"),
                    "load_at_start_percent": formal_sum.get("load_at_start"),
                    "load_before_formal_percent": formal_sum.get("load_before_formal"),
                    "load_after_formal_percent": formal_sum.get("load_after_formal"),
                },
                "idle_unload": formal["meta"].get("idle_unload"),
            },
        },
        "s06": {
            "thresholds_ms": {"quick": 10000, "deep": 25000},
            "verdict": "BLOCKED",
            "reason": "无 vision Key、无搜索 API Key；真实验收要求「1 次真实 Search」",
            "mock_self_check": {
                "note": "MOCK_SELF_CHECK_OK：仅证明测量框架可跑通，数字不是验收结果",
                "files": [p.name for p in sorted(DATA.glob("image_search_mock_*.json"))],
            },
            "unlock": [
                "SNIPASK_BENCH_VISION_BASE_URL / SNIPASK_BENCH_VISION_MODEL / SNIPASK_BENCH_VISION_API_KEY",
                "SNIPASK_BENCH_SEARCH_PROVIDER=tavily|serper + SNIPASK_BENCH_SEARCH_API_KEY",
                "python scripts/spikes/s05_s06/bench_image_search.py --modes quick,deep --runs 20",
            ],
        },
    }
    out = DATA / "summary_s05_s06.json"
    out.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")

    copied = sorted(p.name for p in DATA.glob("*"))
    (DATA / "README.md").write_text(
        "# S-05 / S-06 原始数据与汇总\n\n"
        f"生成时间：{summary['generated_at']}\n\n"
        "本目录文件由 `scripts/spikes/s05_s06/build_data_summary.py` 从 "
        "`.verify/spikes/S-05-S-06/raw/` 复制/聚合，均为真实测量输出。\n\n"
        "## 文件\n\n"
        + "\n".join(f"- `{name}`" for name in copied)
        + "\n\n## 关键口径\n\n"
        "- 所有 JSON 由脚本自动生成，未手工改数；mock/self-test 文件在 `meta.mode` 中明确标注。\n"
        "- 本地翻译正式数据：`load_before_formal_percent=100` 时机器上仍有并行负载，"
        "逐次 tok/s 波动已保留在 records 中；结论 FAILED 在污染期与低负载期一致。\n"
        "- 云端/图片搜索因无 Key 均为 BLOCKED，仅 mock 自检验证框架。\n",
        encoding="utf-8",
    )
    print(f"wrote {out}")
    print(f"copied/created {len(copied)} files in {DATA}")


if __name__ == "__main__":
    main()
