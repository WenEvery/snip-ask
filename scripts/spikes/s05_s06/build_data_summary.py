#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""汇总 S-05/S-06 基准原始数据（含 phase 2：DeepSeek 云端 + 极小模型 + S-06 能力探测）。

只做「读取 raw → 复制精选文件 → 聚合关键数字」，不修改任何测量结果。
用法：python scripts/spikes/s05_s06/build_data_summary.py
"""

from __future__ import annotations

import glob
import json
import shutil
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
RAW = REPO_ROOT / ".verify" / "spikes" / "S-05-S-06" / "raw"
DATA = REPO_ROOT / "docs" / "spikes" / "data" / "s05_s06"


def load(path: Path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def latest(pattern: str):
    files = sorted(glob.glob(str(RAW / pattern)))
    return Path(files[-1]) if files else None


def brief(doc, key="summary"):
    s = doc.get(key) or {}
    return {
        "measured_runs": s.get("measured_runs"),
        "failures": s.get("failures"),
        "failure_rate": s.get("failure_rate"),
        "total_ms": s.get("total_ms"),
        "first_byte_ms": s.get("first_byte_ms"),
        "first_token_ms": s.get("first_token_ms"),
        "verdicts": s.get("verdicts"),
    }


def copy(names):
    copied = []
    for name in names:
        src = RAW / name
        if src.exists():
            shutil.copy2(src, DATA / name)
            copied.append(name)
    return copied


def main() -> None:
    DATA.mkdir(parents=True, exist_ok=True)

    # --- phase 1 数据（保留） ---
    fixed = copy(
        [
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
            "formal_local_20261105-000321.log",
            "download.log",
            "download_tiny.log",
        ]
    )

    # --- phase 2 数据（动态取最新） ---
    cloud_default_p = latest("cloud_translate_stream_20_*.json")
    cloud_nothink_p = latest("cloud_translate_stream20_nothink_*.json")
    tiny_p = latest("local_translate_tiny_qwen05b_*.json")
    probe_p = latest("deepseek_capability_probe_*.json")
    partial_p = latest("image_search_partial_deepseek_*.json")
    for p in (cloud_default_p, cloud_nothink_p, tiny_p, probe_p, partial_p):
        if p:
            shutil.copy2(p, DATA / p.name)
    for f in sorted(RAW.glob("image_search_mock_*.json")):
        shutil.copy2(f, DATA / f.name)
    for f in sorted(RAW.glob("cloud_effort*_smoke.json")):
        shutil.copy2(f, DATA / f.name)

    for f in sorted(RAW.glob("tiny_variant_*.json")) + sorted(RAW.glob("tiny_candidate_*.json")):
        shutil.copy2(f, DATA / f.name)
    if (RAW / "download_qwen15b.log").exists():
        shutil.copy2(RAW / "download_qwen15b.log", DATA / "download_qwen15b.log")

    cloud_default = load(cloud_default_p) if cloud_default_p else {}
    cloud_nothink = load(cloud_nothink_p) if cloud_nothink_p else {}
    tiny = load(tiny_p) if tiny_p else {}
    probe = load(probe_p) if probe_p else {}
    partial = load(partial_p) if partial_p else {}
    formal = load(RAW / "local_translate_formal_20261105-000321.json")
    sens = load(RAW / "local_translate_sens100_20261105-000321.json")
    contaminated = load(RAW / "local_translate_20261104-235408.json")
    contaminated2 = load(RAW / "local_translate_20261104-235614.json")
    formal_sum = load(RAW / "formal_local_summary_20261105-000321.json")
    tune6 = load(RAW / "tune_threads6_20261105-000321.json")
    tune16 = load(RAW / "tune_threads16_20261105-000321.json")

    def cloud_section(doc):
        s = doc.get("summary") or {}
        return {
            "model": doc.get("config", {}).get("model"),
            "stream": doc.get("config", {}).get("stream"),
            "extra_params": doc.get("config", {}).get("extra_params"),
            "calibers": doc.get("config", {}).get("calibers"),
            "measured_runs": s.get("measured_runs"),
            "failures": s.get("failures"),
            "http_status_distribution": s.get("http_status_distribution"),
            "total_ms": s.get("total_ms"),
            "first_byte_ms": s.get("first_byte_ms"),
            "first_token_ms": s.get("first_token_ms"),
            "verdicts": s.get("verdicts"),
            "usage_reasoning_tokens": [
                (r.get("usage") or {}).get("completion_tokens_details", {}).get("reasoning_tokens")
                for r in (doc.get("records") or [])
                if not r.get("warmup") and r.get("ok")
            ],
            "output_sample": next(
                (r.get("output_sample") for r in (doc.get("records") or []) if not r.get("warmup") and r.get("ok")),
                None,
            ),
        }

    tiny_summary = tiny.get("summary") or {}
    tiny_model_meta = tiny.get("meta") or {}
    s06_probes = {}
    for p in (probe.get("probes") or []):
        s06_probes[p.get("name")] = {"status": p.get("status"), "reply": p.get("reply")}

    summary = {
        "generated_at": datetime.now(timezone(timedelta(hours=8))).isoformat(),
        "repo_commit": formal["meta"].get("repo_commit"),
        "environment": {
            "os": "Windows 11 专业版 Build 26200",
            "cpu": "12th Gen Intel Core i7-12650H（10 核 16 线程，6P+4E）",
            "ram_bytes": 16944848896,
            "gpu": "NVIDIA GeForce RTX 4060 Laptop GPU（本次本地基准未使用，CPU 推理 ngl=0）",
            "python": "3.11.3",
            "node": "24.14.1",
            "date": "2026-11-04",
            "cloud_key": "sk-****（用户提供，已脱敏）",
        },
        "s05": {
            "thresholds_ms": {"cloud_fast_ttft": 1500, "local_1_8b": 8000},
            "cloud": {
                "provider": "DeepSeek（OpenAI 兼容 /chat/completions，stream=true）",
                "models_available": ["deepseek-flash", "deepseek-v4-pro"],
                "default_config": cloud_section(cloud_default),
                "thinking_disabled": cloud_section(cloud_nothink),
                "verdict": {
                    "default": "两个口径均 FAILED（首字 P95 4.59s / 总耗时 P95 5.15s，>1.5s）",
                    "thinking_disabled": "首字口径 DONE（P95 0.87s ≤1.5s）；总耗时口径 FAILED（P95 1.75s >1.5s）",
                    "reasoning_evidence": "默认 completion_tokens 中 reasoning_tokens 309–574，是首字 3–4.6s 的主因；"
                    "`{\"thinking\":{\"type\":\"disabled\"}}` 后首字降至 0.61s(P50)/0.87s(P95)。",
                },
            },
            "local_1_8b": {
                "verdict": "FAILED",
                "model": {
                    "id": "hy-mt-1.8b-q4",
                    "file": "HY-MT1.5-1.8B-Q4_K_M.gguf",
                    "size_bytes": formal["meta"].get("model_size_bytes"),
                    "sha256": formal["meta"].get("model_sha256"),
                },
                "engine": {
                    "exe": formal["meta"].get("exe"),
                    "version": formal["meta"].get("exe_version"),
                    "args": formal["meta"].get("server_args"),
                },
                "sampling": formal["meta"].get("sampling"),
                "cold_load_ms": formal["meta"].get("health_ready_ms"),
                "memory": {
                    "after_load": formal["meta"].get("memory_after_load"),
                    "after_runs": formal["meta"].get("memory_after_runs"),
                },
                "formal_10_runs_threads16": {
                    "total_ms": formal["summary"]["total_ms"],
                    "server_prompt_ms": formal["summary"]["server_prompt_ms"],
                    "server_predicted_ms": formal["summary"]["server_predicted_ms"],
                    "tok_per_sec": formal["summary"]["tok_per_sec"],
                    "completion_tokens": formal["summary"]["completion_tokens"],
                },
                "thread_tuning": {
                    "threads_6": tune6["summary"]["total_ms"],
                    "threads_16": tune16["summary"]["total_ms"],
                },
                "max_tokens_100_sensitivity": {
                    "total_ms": sens["summary"]["total_ms"],
                    "tok_per_sec": sens["summary"]["tok_per_sec"],
                    "completion_tokens": sens["summary"]["completion_tokens"],
                },
                "contaminated_runs": {
                    "temp0.7_parallel_load": {
                        "total_ms": contaminated["summary"]["total_ms"],
                        "completion_tokens": contaminated["summary"]["completion_tokens"],
                    },
                    "temp0.2_parallel_load": {
                        "total_ms": contaminated2["summary"]["total_ms"],
                        "completion_tokens": contaminated2["summary"]["completion_tokens"],
                    },
                },
                "load_notes": {
                    "quiet_load_threshold_percent": formal_sum.get("quiet_load_threshold"),
                    "load_at_start_percent": formal_sum.get("load_at_start"),
                    "load_before_formal_percent": formal_sum.get("load_before_formal"),
                    "load_after_formal_percent": formal_sum.get("load_after_formal"),
                },
                "idle_unload": formal["meta"].get("idle_unload"),
            },
            "tiny_model": {
                "verdict": tiny.get("verdict"),
                "model": {
                    "id": "Qwen2.5-0.5B-Instruct Q4_K_M",
                    "path": tiny_model_meta.get("model_path"),
                    "size_bytes": tiny_model_meta.get("model_size_bytes"),
                    "sha256": tiny_model_meta.get("model_sha256"),
                    "source": "ModelScope Qwen/Qwen2.5-0.5B-Instruct-GGUF，491,400,032 B，48.26s 下载",
                },
                "engine": tiny_model_meta.get("exe"),
                "exe_version": tiny_model_meta.get("exe_version"),
                "server_args": tiny_model_meta.get("server_args"),
                "sampling": tiny_model_meta.get("sampling"),
                "cold_load_ms": tiny_model_meta.get("health_ready_ms"),
                "memory": {
                    "after_load": tiny_model_meta.get("memory_after_load"),
                    "after_runs": tiny_model_meta.get("memory_after_runs"),
                },
                "summary": {
                    "total_ms": tiny_summary.get("total_ms"),
                    "server_prompt_ms": tiny_summary.get("server_prompt_ms"),
                    "server_predicted_ms": tiny_summary.get("server_predicted_ms"),
                    "tok_per_sec": tiny_summary.get("tok_per_sec"),
                    "completion_tokens": tiny_summary.get("completion_tokens"),
                    "failures": tiny_summary.get("failures"),
                },
                "samples": [r.get("output_sample") for r in (tiny.get("records") or []) if not r.get("warmup")][:2],
            },
        },
        "s06": {
            "verdict": "BLOCKED（验收口径）",
            "capability_probe": {
                "models": probe.get("models"),
                "vision_supported": (probe.get("conclusion") or {}).get("vision_supported"),
                "search_endpoint_found": (probe.get("conclusion") or {}).get("search_endpoint_found"),
                "probes": s06_probes,
            },
            "partial_measurement": {
                "note": "视觉/摘要为真实 DeepSeek 调用，Search 为 mock；PARTIAL_REAL_VISION_MOCK_SEARCH，不得作为验收结论",
                "modes": {
                    mode: {
                        "verdict": data.get("verdict"),
                        "total_ms": (data.get("total_ms") or {}),
                        "stages_ms": {
                            k: (v.get("p50_ms") if v else None)
                            for k, v in (data.get("stages_ms") or {}).items()
                        },
                    }
                    for mode, data in (partial.get("modes") or {}).items()
                },
            },
            "minimum_keys": "DeepSeek Key（已有，作 vision）+ 1 个搜索 API Key（Tavily/Serper）",
            "thresholds_ms": {"quick": 10000, "deep": 25000},
        },
    }
    # --- 极小模型候选复核（task-13）---
    def translated_ratio(records):
        recs = [r for r in records if not r.get("warmup") and r.get("ok")]
        if not recs:
            return None
        return round(sum(1 for r in recs if r.get("looks_translated")) / len(recs), 2)

    variant_docs = []
    for f in sorted(RAW.glob("tiny_variant_*.json")):
        d = load(f)
        s = d.get("summary") or {}
        variant_docs.append(
            {
                "file": f.name,
                "prompt_variant": (d.get("meta") or {}).get("prompt_variant"),
                "text_chars": (d.get("meta") or {}).get("sample_len_chars"),
                "total_ms": s.get("total_ms"),
                "tok_per_sec": s.get("tok_per_sec"),
                "completion_tokens": s.get("completion_tokens"),
                "translated_ratio": translated_ratio(d.get("records") or []),
                "sample": next(
                    (r.get("output_sample") for r in (d.get("records") or []) if not r.get("warmup")),
                    None,
                ),
            }
        )
    candidate_docs = []
    for f in sorted(RAW.glob("tiny_candidate_*.json")):
        d = load(f)
        s = d.get("summary") or {}
        m = d.get("meta") or {}
        candidate_docs.append(
            {
                "file": f.name,
                "model": m.get("model_path"),
                "model_sha256": m.get("model_sha256"),
                "prompt_variant": m.get("prompt_variant"),
                "text_chars": m.get("sample_len_chars"),
                "cold_load_ms": m.get("health_ready_ms"),
                "memory_after_load": m.get("memory_after_load"),
                "total_ms": s.get("total_ms"),
                "tok_per_sec": s.get("tok_per_sec"),
                "completion_tokens": s.get("completion_tokens"),
                "translated_ratio": translated_ratio(d.get("records") or []),
                "sample": next(
                    (r.get("output_sample") for r in (d.get("records") or []) if not r.get("warmup")),
                    None,
                ),
            }
        )

    summary["s05"]["tiny_candidates"] = {
        "note": "0.5B 用产品 translate.md 会复述中文；换极简指令后可正常翻译。"
        "1.5B+极简指令 P95 6.72s、质量更好。注意 translated_ratio 只按 CJK 占比自动判定，"
        "minimal-fewshot 200 字实际只输出示例句（样例人工判断为不可用）。",
        "prompt_variants_0_5b": variant_docs,
        "qwen2_5_1_5b": candidate_docs,
        "recommendation": "极小档 = Qwen2.5-0.5B Q4 + 极简翻译指令（P95 3.21s / 0.55GiB）；"
        "质量优先用 Qwen2.5-1.5B Q4 + 极简指令（P95 6.72s / 1.70GiB）；"
        "两者都不能使用产品 translate.md 提示词（0.5B 会复述）。",
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
        "- 所有 JSON 由脚本自动生成，未手工改数；mock/PARTIAL 文件在 `meta.mode` 与 `labels` 中明确标注，"
        "不得作为验收依据。\n"
        "- 云端：DeepSeek `deepseek-flash`，stream=true；默认 thinking 与关闭 thinking 两组均记录 TTFT/总耗时。\n"
        "- 本地 1.8B：正式 10 次线程 16；污染期数据保留为对照。\n"
        "- 极小模型 Qwen2.5-0.5B：正式 10 次线程 16，记录 tok/s、内存、冷加载与输出样例。\n"
        "- S-06：DeepSeek 单 Key 视觉可用（真实截图识别成功）但无 Search API（404），验收仍 BLOCKED；"
        "PARTIAL 测量只用于估算视觉腿时延。\n",
        encoding="utf-8",
    )
    print(f"wrote {out}")
    print(f"files in {DATA}: {len(copied)}")
    if tiny_p:
        print(f"tiny data: {tiny_p.name}")
    else:
        print("WARNING: tiny model data missing")


if __name__ == "__main__":
    main()
