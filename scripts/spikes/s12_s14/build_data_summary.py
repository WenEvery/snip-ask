#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""把 S-12 / S-14 的原始数据聚合到 `docs/spikes/data/s12_s14/`（供文档引用的小体量汇总）。

做三件事：
    1. 从 `.verify/spikes/S-12-S-14/raw/` 选出最新一次真实运行 / mock 自检 JSON，复制并改名；
    2. 生成 `summary_s12_s14.json`（只含关键指标，去掉逐图逐例全文，体量可控）；
    3. 生成 `README.md`（文件清单 + 关键口径 + 复跑命令）。

用法：
    python scripts/spikes/s12_s14/build_data_summary.py
"""

from __future__ import annotations

import json
import shutil
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
RAW = REPO_ROOT / ".verify" / "spikes" / "S-12-S-14" / "raw"
CORPUS = REPO_ROOT / ".verify" / "spikes" / "S-12-S-14" / "corpus"
OUT = REPO_ROOT / "docs" / "spikes" / "data" / "s12_s14"
TZ = timezone(timedelta(hours=8))


def now_iso() -> str:
    return datetime.now(TZ).isoformat()


def newest(pattern: str, exclude: str = "") -> Optional[Path]:
    files = sorted(RAW.glob(pattern), key=lambda p: p.stat().st_mtime, reverse=True)
    for f in files:
        if exclude and exclude in f.name:
            continue
        return f
    return None


def load(path: Optional[Path]) -> Optional[Dict[str, Any]]:
    if not path or not path.is_file():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None


def copy(src: Optional[Path], dest_name: str) -> Optional[str]:
    if not src or not src.is_file():
        return None
    OUT.mkdir(parents=True, exist_ok=True)
    target = OUT / dest_name
    shutil.copyfile(src, target)
    return target.name


def summarize_s14(data: Optional[Dict[str, Any]]) -> Dict[str, Any]:
    if not data:
        return {"status": "MISSING"}
    meta = data.get("meta", {})
    runs = []
    for run in data.get("runs", []):
        s = run.get("summary") or {}
        runs.append(
            {
                "arm": run.get("arm"),
                "kind": run.get("kind"),
                "status": run.get("status"),
                "verdict": run.get("verdict"),
                "term_hit_rate": s.get("term_hit_rate"),
                "format_pass_rate": s.get("format_pass_rate"),
                "translation_effective_rate": s.get("translation_effective_rate"),
                "case_pass": s.get("case_pass"),
                "cases": s.get("cases"),
                "format_by_kind": s.get("format_by_kind"),
                "failures_count": len(s.get("failures") or []),
                "model": (run.get("meta") or {}).get("model_file") or (run.get("meta") or {}).get("model"),
                "prompt_key": (run.get("meta") or {}).get("prompt_key"),
                "health_ready_ms": (run.get("meta") or {}).get("health_ready_ms"),
                "shutdown_clean": ((run.get("meta") or {}).get("shutdown") or {}).get("clean"),
            }
        )
    return {
        "mode": meta.get("mode"),
        "generated_at": meta.get("generated_at"),
        "cases": meta.get("cases"),
        "thresholds": meta.get("thresholds"),
        "api_key_provided": meta.get("api_key_provided"),
        "api_key_source": meta.get("api_key_source"),
        "runs": runs,
        "aggregate": data.get("aggregate"),
        "verdicts": {r["arm"]: r["verdict"] for r in runs},
        "self_check_ok": data.get("self_check_ok"),
        "self_check_assertions": data.get("self_check_assertions"),
    }


def summarize_s12(data: Optional[Dict[str, Any]]) -> Dict[str, Any]:
    if not data:
        return {"status": "MISSING"}
    meta = data.get("meta", {})
    if meta.get("mode") == "MOCK_SELF_TEST":
        return {
            "mode": "MOCK_SELF_TEST",
            "generated_at": meta.get("generated_at"),
            "mock_pass": (data.get("mock_pass") or {}).get("summary"),
            "mock_fail": (data.get("mock_fail") or {}).get("summary"),
            "self_check_ok": data.get("self_check_ok"),
            "self_check_assertions": data.get("self_check_assertions"),
        }
    summary = data.get("summary") or {}
    return {
        "mode": meta.get("mode"),
        "generated_at": meta.get("generated_at"),
        "corpus": meta.get("corpus"),
        "model": meta.get("model"),
        "search": meta.get("search"),
        "thresholds": meta.get("thresholds"),
        "four_sections": summary.get("four_sections"),
        "query_relevance": summary.get("query_relevance"),
        "source_authenticity": summary.get("source_authenticity"),
        "uncertainty": summary.get("uncertainty"),
        "recognition_hit_mean": summary.get("recognition_hit_mean"),
        "by_category": summary.get("by_category"),
        "source_check": {
            "checked": (data.get("source_check") or {}).get("checked"),
            "accessible": (data.get("source_check") or {}).get("accessible"),
            "accessible_rate": (data.get("source_check") or {}).get("accessible_rate"),
            "rates": (data.get("source_check") or {}).get("rates"),
            "failure_buckets": {
                name: bucket.get("count")
                for name, bucket in ((data.get("source_check") or {}).get("failure_buckets") or {}).items()
            },
        },
        "verdict": data.get("verdict"),
        "failures": data.get("failures"),
        "client_stats": data.get("client_stats"),
    }


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)

    s14_real_path = newest("translate_consistency_2*.json", exclude="mock")
    s14_v3_path = newest("translate_consistency_v3fix_*.json")
    s14_mock_path = newest("translate_consistency_mock_*.json")
    s12_real_path = newest("image_search_trust_2*.json", exclude="mock")
    s12_mock_path = newest("image_search_trust_mock_*.json")
    corpus_path = CORPUS / "corpus.json"

    s14_real = load(s14_real_path)
    s14_v3 = load(s14_v3_path)
    s14_mock = load(s14_mock_path)
    s12_real = load(s12_real_path)
    s12_mock = load(s12_mock_path)
    corpus = load(corpus_path)

    copied: List[str] = []
    for path, name in (
        (s14_real_path, "s14_translate_consistency_real.json"),
        (s14_v3_path, "s14_translate_consistency_v3fix.json"),
        (s14_mock_path, "s14_translate_consistency_mock.json"),
        (s12_real_path, "s12_image_search_trust_real.json"),
        (s12_mock_path, "s12_image_search_trust_mock.json"),
        (corpus_path, "s12_corpus_manifest.json"),
    ):
        got = copy(path, name)
        if got:
            copied.append(got)

    # 4 张代表性语料截图（每类 1 张），控制体量
    representatives = [
        ("text-01-article.png", "s12_example_text.png"),
        ("chart-01-bar.png", "s12_example_chart.png"),
        ("product-01-headphones.png", "s12_example_product.png"),
        ("code-01-python.png", "s12_example_code.png"),
    ]
    for src_name, dest_name in representatives:
        src = CORPUS / "shots" / src_name
        got = copy(src, dest_name)
        if got:
            copied.append(got)

    summary = {
        "generated_at": now_iso(),
        "sources": {
            "s14_real": str(s14_real_path.relative_to(REPO_ROOT)).replace("\\", "/") if s14_real_path else None,
            "s14_v3fix": str(s14_v3_path.relative_to(REPO_ROOT)).replace("\\", "/") if s14_v3_path else None,
            "s14_mock": str(s14_mock_path.relative_to(REPO_ROOT)).replace("\\", "/") if s14_mock_path else None,
            "s12_real": str(s12_real_path.relative_to(REPO_ROOT)).replace("\\", "/") if s12_real_path else None,
            "s12_mock": str(s12_mock_path.relative_to(REPO_ROOT)).replace("\\", "/") if s12_mock_path else None,
            "corpus": str(corpus_path.relative_to(REPO_ROOT)).replace("\\", "/") if corpus_path.is_file() else None,
        },
        "s14": summarize_s14(s14_real),
        "s14_v3fix": summarize_s14(s14_v3),
        "s14_mock_self_check": summarize_s14(s14_mock),
        "s12": summarize_s12(s12_real),
        "s12_mock_self_check": summarize_s12(s12_mock),
        "files": ["README.md", "summary_s12_s14.json"] + copied,
    }
    (OUT / "summary_s12_s14.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")

    key_lines = []
    for run in summary["s14"].get("runs", []):
        key_lines.append(
            f"- S-14 `{run['arm']}`：{run['status']} / {run['verdict']}；"
            f"术语 {run['term_hit_rate']}、格式 {run['format_pass_rate']}、译文有效 {run['translation_effective_rate']}、"
            f"整例 {run['case_pass']}/{run['cases']}"
        )
    for run in summary["s14_v3fix"].get("runs", []):
        key_lines.append(
            f"- S-14 处置复核 `{run['arm']}`：{run['status']} / {run['verdict']}；"
            f"术语 {run['term_hit_rate']}、格式 {run['format_pass_rate']}、译文有效 {run['translation_effective_rate']}、"
            f"整例 {run['case_pass']}/{run['cases']}"
        )
    s12 = summary["s12"]
    if str(s12.get("mode") or "").startswith("REAL"):
        buckets = s12["source_check"].get("failure_buckets") or {}
        rates = s12["source_check"].get("rates") or {}
        key_lines.append(
            f"- S-12 真实链路：四段 {s12['four_sections']['rate']}、检索词相关 {s12['query_relevance']['rate']}、"
            f"来源可访问 raw={s12['source_check']['accessible_rate']}（剔除不可判定 {rates.get('excluding_unverifiable')}、"
            f"乐观上界 {rates.get('optimistic')}；客观失效 {buckets.get('dead_http')} / 反爬 {buckets.get('antibot')} / "
            f"网络 {buckets.get('network')}）、不确定标注 {s12['uncertainty']['rate']}；"
            f"判定 {(s12.get('verdict') or {}).get('verdict')}"
        )
    else:
        key_lines.append(
            "- S-12：真实链路 BLOCKED（无 DeepSeek Key），已交付语料 + 脚本 + MOCK 框架自检"
            f"（self_check_ok={summary['s12_mock_self_check'].get('self_check_ok')}）"
        )

    readme = f"""# S-12 / S-14 原始数据与汇总

生成时间：{summary['generated_at']}

本目录文件由 `scripts/spikes/s12_s14/build_data_summary.py` 从
`.verify/spikes/S-12-S-14/` 复制/聚合并生成，均为真实测量输出；mock 文件在文件名与
`meta.mode` 中明确标注，**不得作为验收依据**。

## 文件

""" + "\n".join(f"- `{name}`" for name in summary["files"]) + f"""

## 关键口径

- S-14：24 条样例（中→英 12 / 英→中 12），含换行/空行/列表/编号/代码块/URL/邮箱/占位符/标识符/术语；
  术语命中率与格式保留率逐项判定，目标均 100%；每条结果都带 `backend` 与 `backend_label`。
- S-12：30 张 headless Edge 真实渲染截图（文字 8 / 图表 8 / 商品 7 / 代码 7，其中 4 张标记为
  `expect_uncertain`）；四段=识别结果/检索词/结论/来源；来源可访问率为脚本 HEAD/GET 实测（8s 超时）；
  来源真实性=URL 是否出现在本次 Search 返回中（识别编造）。
- 云端 Key 不落盘：`s14_*`/`s12_*` JSON 只含 `api_key_provided` 与脱敏 host。

## 本次关键数字

""" + "\n".join(key_lines) + "\n"

    (OUT / "README.md").write_text(readme, encoding="utf-8")
    print(f"已生成 {OUT/'summary_s12_s14.json'}")
    print(f"已生成 {OUT/'README.md'}")
    print(f"复制文件 {len(copied)} 个")
    return 0


if __name__ == "__main__":
    sys.exit(main())
