#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""DeepSeek Search / Fetch 工具 API 单独时延基准（S-06 参考数据 + 能力验证）。

端点（官方文档 https://api-docs.deepseek.com/zh-cn/guides/search）：
    POST https://api.deepseek.com/v1/tools/search
         body: {"query": str, "count": 1-50, "need_content": bool, "ori_question"?: str}
         resp: {"results": [{title, url, snippet, summary, content?, date?, authority?, rerank_score?}]}
    POST https://api.deepseek.com/v1/tools/fetch
         body: {"url": str}
         resp: {"title": str, "content": markdown str}
鉴权同 Bearer。QPS 限制：Search 100、Fetch 25；计费按成功请求。
错误码：400/401/402/403/404/408/413/429/500/504；生产策略对 429/5xx/超时退避重试 ≤2 次。

Key 纪律：Key 仅从环境变量 `SNIPASK_BENCH_API_KEY` 读取，**不写入任何文件/日志/输出**，不打印请求头。
用法：
    $env:SNIPASK_BENCH_API_KEY = "sk-****"
    python scripts/spikes/s05_s06/bench_deepseek_tools.py                 # 默认 search 20 + fetch 20
    python scripts/spikes/s05_s06/bench_deepseek_tools.py --inspect       # 只跑 1 次，落盘响应结构
    python scripts/spikes/s05_s06/bench_deepseek_tools.py --runs-search 5 --runs-fetch 5
退出码：0 完成；2 无 Key；3 全部失败。
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

import requests

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
RAW = REPO_ROOT / ".verify" / "spikes" / "S-05-S-06" / "raw"
BASE_URL = os.environ.get("SNIPASK_BENCH_BASE_URL", "https://api.deepseek.com").rstrip("/")
API_KEY = os.environ.get("SNIPASK_BENCH_API_KEY", "")

SEARCH_QUERIES = [
    "DeepSeek V4 模型 发布",
    "AI 屏幕翻译 截图 工具",
    "Qwen2.5 模型 中文 翻译 质量",
    "截图 问答 工具 推荐",
    "大模型 本地推理 性能 对比",
]
FALLBACK_FETCH_URLS = [
    "https://api-docs.deepseek.com/zh-cn/guides/search",
    "https://api-docs.deepseek.com/zh-cn/",
]
RETRY_STATUS = {408, 429, 500, 502, 503, 504}


def percentile(values, p):
    if not values:
        return float("nan")
    s = sorted(values)
    if len(s) == 1:
        return s[0]
    k = (len(s) - 1) * (p / 100.0)
    lo, hi = int(k), min(int(k) + 1, len(s) - 1)
    return s[lo] + (s[hi] - s[lo]) * (k - lo)


def summarize(values):
    if not values:
        return {}
    return {
        "count": len(values),
        "mean_ms": round(statistics.fmean(values), 2),
        "p50_ms": round(percentile(values, 50), 2),
        "p95_ms": round(percentile(values, 95), 2),
        "max_ms": round(max(values), 2),
        "min_ms": round(min(values), 2),
    }


class ToolsClient:
    def __init__(self, base_url: str, api_key: str, timeout: float = 30.0):
        self.session = requests.Session()
        self.session.headers.update(
            {
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
                "User-Agent": "snipask-bench-tools/1.0",
            }
        )
        self.base = base_url
        self.timeout = timeout

    def post(self, path: str, payload: dict, retries: int = 2):
        """返回 (status, latency_ms, json_or_text, attempts)。429/5xx/超时退避重试。"""
        attempts = 0
        last = None
        for attempt in range(retries + 1):
            attempts = attempt + 1
            t0 = time.perf_counter()
            try:
                resp = self.session.post(
                    f"{self.base}{path}", json=payload, timeout=self.timeout
                )
                latency = (time.perf_counter() - t0) * 1000.0
                try:
                    body = resp.json()
                except ValueError:
                    body = resp.text[:2000]
                last = (resp.status_code, round(latency, 2), body, attempts)
                if resp.status_code == 200 or resp.status_code not in RETRY_STATUS:
                    return last
            except requests.RequestException as err:
                latency = (time.perf_counter() - t0) * 1000.0
                last = (None, round(latency, 2), f"{type(err).__name__}: {err}", attempts)
            if attempt < retries:
                time.sleep(0.5 * (2**attempt))
        return last

    def search(self, query: str, count: int, need_content: bool = False):
        return self.post(
            "/v1/tools/search",
            {"query": query, "count": count, "need_content": need_content},
        )

    def fetch(self, url: str):
        return self.post("/v1/tools/fetch", {"url": url})


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="DeepSeek Search/Fetch 单独时延基准")
    ap.add_argument("--base-url", default=BASE_URL)
    ap.add_argument("--api-key", default=API_KEY)
    ap.add_argument("--runs-search", type=int, default=20)
    ap.add_argument("--runs-fetch", type=int, default=20)
    ap.add_argument("--count", type=int, default=5, help="Search 返回条数")
    ap.add_argument("--timeout", type=float, default=30.0)
    ap.add_argument("--inspect", action="store_true", help="各跑 1 次并落盘响应结构")
    ap.add_argument(
        "--fetch-urls",
        default="",
        help="逗号分隔的 Fetch URL 列表；默认用 Search 返回的真实 URL（可能包含 404/慢页）",
    )
    ap.add_argument("--out", default="")
    args = ap.parse_args(argv)

    if not args.api_key:
        print("BLOCKED：未提供 SNIPASK_BENCH_API_KEY。")
        return 2

    client = ToolsClient(args.base_url, args.api_key, timeout=args.timeout)
    search_runs = 1 if args.inspect else args.runs_search
    fetch_runs = 1 if args.inspect else args.runs_fetch

    result = {
        "tool": "bench_deepseek_tools.py",
        "generated_at": datetime.now(timezone(timedelta(hours=8))).isoformat(),
        "base_url": args.base_url,
        "api_key_provided": True,
        "search": {"count": args.count, "runs": search_runs, "records": []},
        "fetch": {"runs": fetch_runs, "records": []},
        "inspect": None,
    }

    # --- Search ---
    fetch_urls = []
    search_latencies = []
    for i in range(search_runs):
        query = SEARCH_QUERIES[i % len(SEARCH_QUERIES)]
        status, latency, body, attempts = client.search(query, args.count)
        n_results = 0
        urls = []
        if status == 200 and isinstance(body, dict):
            items = body.get("results") or []
            n_results = len(items)
            urls = [it.get("url") for it in items if it.get("url")]
            fetch_urls.extend(urls[:3])
        rec = {
            "run": f"search-{i + 1}",
            "query": query,
            "status": status,
            "latency_ms": latency,
            "attempts": attempts,
            "result_count": n_results,
            "empty": status == 200 and n_results == 0,
            "error": None if status == 200 else str(body)[:400],
        }
        result["search"]["records"].append(rec)
        if status == 200 and latency is not None:
            search_latencies.append(latency)

    # --- Fetch（优先用真实检索结果的 URL，避免固定 URL 失效；可 --fetch-urls 指定稳定集合） ---
    if args.fetch_urls.strip():
        urls = [u.strip() for u in args.fetch_urls.split(",") if u.strip()]
    else:
        urls = list(dict.fromkeys(fetch_urls))[:10] or FALLBACK_FETCH_URLS
    fetch_latencies = []
    fetch_ok = 0
    for i in range(fetch_runs):
        url = urls[i % len(urls)]
        status, latency, body, attempts = client.fetch(url)
        content_chars = 0
        title = None
        if status == 200 and isinstance(body, dict):
            content_chars = len(body.get("content") or "")
            title = body.get("title")
            if content_chars > 0:
                fetch_ok += 1
        rec = {
            "run": f"fetch-{i + 1}",
            "url": url,
            "status": status,
            "latency_ms": latency,
            "attempts": attempts,
            "title": title,
            "content_chars": content_chars,
            "error": None if status == 200 else str(body)[:400],
        }
        result["fetch"]["records"].append(rec)
        if status == 200 and latency is not None:
            fetch_latencies.append(latency)

    result["search"]["latency_ms"] = summarize(search_latencies)
    result["search"]["failures"] = sum(
        1 for r in result["search"]["records"] if r["status"] != 200
    )
    result["search"]["empty_results"] = sum(
        1 for r in result["search"]["records"] if r["empty"]
    )
    result["fetch"]["latency_ms"] = summarize(fetch_latencies)
    result["fetch"]["failures"] = sum(
        1 for r in result["fetch"]["records"] if r["status"] != 200
    )
    result["fetch"]["success_with_content"] = fetch_ok
    result["status_codes"] = {
        str(code): sum(
            1
            for r in result["search"]["records"] + result["fetch"]["records"]
            if r["status"] == code
        )
        for code in sorted(
            {
                r["status"]
                for r in result["search"]["records"] + result["fetch"]["records"]
                if r["status"] is not None
            }
        )
    }

    if args.inspect:
        status, latency, body, attempts = client.search(SEARCH_QUERIES[0], 3, False)
        s_body = body
        status2, latency2, body2, attempts2 = client.fetch(FALLBACK_FETCH_URLS[0])
        f_body = body2
        result["inspect"] = {
            "search_shape": {
                "status": status,
                "latency_ms": latency,
                "top_level_keys": sorted(s_body.keys()) if isinstance(s_body, dict) else None,
                "results_count": len((s_body or {}).get("results") or []) if isinstance(s_body, dict) else None,
                "first_result": (
                    {k: (str(v)[:300] if not isinstance(v, (int, float, bool, type(None))) else v)
                     for k, v in ((s_body.get("results") or [{}])[0]).items()}
                    if isinstance(s_body, dict) and (s_body.get("results") or [])
                    else None
                ),
            },
            "fetch_shape": {
                "status": status2,
                "latency_ms": latency2,
                "keys": sorted(f_body.keys()) if isinstance(f_body, dict) else None,
                "title": (f_body.get("title") if isinstance(f_body, dict) else None),
                "content_chars": len(f_body.get("content") or "") if isinstance(f_body, dict) else None,
                "content_head": (f_body.get("content") or "")[:500] if isinstance(f_body, dict) else None,
            },
        }

    print("=" * 74)
    print("DeepSeek Search / Fetch 单独时延")
    print(f"Search：n={search_runs} count={args.count}  {result['search'].get('latency_ms')}")
    print(f"        failures={result['search']['failures']} empty={result['search']['empty_results']}")
    print(f"Fetch ：n={fetch_runs} {result['fetch'].get('latency_ms')}")
    print(f"        failures={result['fetch']['failures']} ok_with_content={fetch_ok}")
    print(f"status_codes={result['status_codes']}")
    print("=" * 74)

    out = Path(args.out) if args.out else (
        RAW / f"deepseek_tools_{'inspect' if args.inspect else 'bench'}_{datetime.now().strftime('%Y%m%d-%H%M%S')}.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"原始数据：{out}")

    if not search_latencies and not fetch_latencies:
        return 3
    return 0


if __name__ == "__main__":
    if hasattr(sys.stdout, "reconfigure"):
        try:
            sys.stdout.reconfigure(encoding="utf-8")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
