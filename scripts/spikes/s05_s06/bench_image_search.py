#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S-06 图片搜索端到端时延基准（视觉识别 → 生成检索词 → Search → 摘要）。

被测流水线（按 resources/prompts/image_search.md 版本 1 拆分）：
    ① 视觉识别：OpenAI 兼容 vision chat/completions，输入框选截图，输出「识别结果」+「检索词」
    ② 生成检索词：解析阶段①输出（本地解析，耗时单列；deep 模式支持 2–3 组检索词）
    ③ Search：真实检索 API（Tavily / Serper，可配）或 mock；quick 1 次、deep 2–3 次
    ④ 摘要：把识别结果 + 检索结果交给同一模型，输出「结论」+「来源」

DONE 阈值（技术验证清单 §2）：快速模式端到端 P95 ≤ 10s；深度模式 P95 ≤ 25s。

环境变量：
    SNIPASK_BENCH_VISION_BASE_URL  （回退 SNIPASK_BENCH_BASE_URL）  例如 https://api.openai.com/v1
    SNIPASK_BENCH_VISION_MODEL     vision 模型名，例如 gpt-4o-mini / qwen-vl-max
    SNIPASK_BENCH_VISION_API_KEY   （回退 SNIPASK_BENCH_API_KEY）
    SNIPASK_BENCH_SEARCH_PROVIDER  none | tavily | serper（默认 none → mock）
    SNIPASK_BENCH_SEARCH_API_KEY   检索 API Key

用法：
    python scripts/spikes/s05_s06/bench_image_search.py --modes quick,deep --runs 20
    python scripts/spikes/s05_s06/bench_image_search.py --mock-self-check   # 无 Key 时验证框架
    python scripts/spikes/s05_s06/bench_image_search.py --help

退出码：0 = 真实链路测量完成；2 = BLOCKED（缺 Key/模型/检索服务，或全部请求失败）；
        3 = 脚本异常。

重要：--mock-self-check 的数字来自 mock（本地假数据、假延时），**只能证明测量框架可跑通，
绝不能作为 S-06 DONE/FAILED 依据**；mock 输出的来源 URL 使用 example.invalid 域名，明确不可信。
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import http.client
import json
import os
import platform
import re
import socket
import statistics
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple
from urllib.parse import urlsplit

SCRIPT_VERSION = "1.0.0"
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
DEFAULT_OUT_DIR = REPO_ROOT / ".verify" / "spikes" / "S-05-S-06" / "raw"
PROMPT_FILE = REPO_ROOT / "resources" / "prompts" / "image_search.md"
DEFAULT_IMAGE = REPO_ROOT / "docs" / "images" / "settings-ai.png"
BODY_MARKER = "以下为发送给模型的正文"

THRESHOLDS_MS = {"quick": 10000.0, "deep": 25000.0}
MOCK_URL_MARK = "https://example.invalid/mock-search/"

EXIT_OK, EXIT_BLOCKED, EXIT_ERROR = 0, 2, 3


# --------------------------------------------------------------------------- #
# 提示词：读取 image_search.md 正文，按两阶段拆分（阶段①识别+检索词，阶段④结论+来源）
# --------------------------------------------------------------------------- #
def load_image_search_template(lang: str, mode: str) -> str:
    raw = PROMPT_FILE.read_text(encoding="utf-8")
    marker = raw.find(BODY_MARKER)
    body = raw[marker:] if marker >= 0 else raw
    if "\n" in body:
        body = body.split("\n", 1)[1]
    return body.replace("{lang}", lang).replace("{mode}", mode)


def build_phase_a_prompt(template: str) -> str:
    return (
        template
        + "\n\n【分阶段执行（基准口径）】\n"
        + "本阶段是流水线第 1 步，只需输出「识别结果」与「检索词」两段，不要输出结论与来源；\n"
        + "结论与来源将由后续步骤基于真实检索结果生成。\n"
        + "格式：\n识别结果：<客观描述>\n检索词：<1 组；deep 模式 2–3 组，行内用、分隔>\n"
    )


def build_phase_b_prompt(recognized: str, queries: List[str], search_items: List[Dict[str, Any]], lang: str) -> str:
    lines = [
        "你是一名看图检索助手。前序步骤已完成图片识别与联网检索，请基于以下材料生成最终答复。",
        f"请使用语言：{lang}。",
        "",
        "【识别结果】",
        recognized or "（无）",
        "",
        "【已执行的检索词】",
        "、".join(queries) if queries else "（无）",
        "",
        "【检索结果（真实返回，可能不完整）】",
    ]
    if not search_items:
        lines.append("（未检索到可靠来源）")
    for idx, item in enumerate(search_items, 1):
        lines.append(f"{idx}. {item.get('title', '')} - {item.get('url', '')}")
        snippet = (item.get("snippet") or "").strip()
        if snippet:
            lines.append(f"   摘要：{snippet[:300]}")
    lines += [
        "",
        "【要求】",
        "1. 结论只能基于上面的识别结果与检索结果；不得编造新闻、价格、版本号等具体事实。",
        "2. 「来源」每行一条「标题 - URL」，只能使用上面检索结果中真实出现的 URL；没有则写「未检索到可靠来源」。",
        "3. 不确定处直接标注「不确定」。不要寒暄。",
        "",
        "【输出格式（只输出这两段，顺序固定）】",
        "结论：<基于检索与识别的结论>",
        "来源：<每行一条「标题 - URL」；没有则写「未检索到可靠来源」>",
    ]
    return "\n".join(lines)


# --------------------------------------------------------------------------- #
# HTTP（OpenAI 兼容 + 检索 API），保留首字节/总耗时
# --------------------------------------------------------------------------- #
class HttpClient:
    def __init__(self, base_url: str, timeout: float) -> None:
        parts = urlsplit(base_url)
        if parts.scheme not in ("http", "https"):
            raise ValueError(f"base_url 必须为 http(s)://，实际：{base_url}")
        self.scheme = parts.scheme
        self.host = parts.hostname or "127.0.0.1"
        self.port = parts.port or (443 if parts.scheme == "https" else 80)
        self.base_path = parts.path.rstrip("/")
        self.timeout = timeout
        self.masked_url = f"{self.scheme}://{self.host}:{self.port}{self.base_path}"
        self.conn: Optional[http.client.HTTPConnection] = None

    def _connection(self) -> http.client.HTTPConnection:
        if self.conn is None:
            cls = http.client.HTTPSConnection if self.scheme == "https" else http.client.HTTPConnection
            self.conn = cls(self.host, self.port, timeout=self.timeout)
            self.conn.connect()
        return self.conn

    def close(self) -> None:
        if self.conn is not None:
            try:
                self.conn.close()
            finally:
                self.conn = None

    def chat_completions(
        self, api_key: str, payload: Dict[str, Any]
    ) -> Dict[str, Any]:
        url_path = self.base_path
        if not url_path.endswith("/chat/completions"):
            url_path += "/chat/completions"
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        headers = {
            "Content-Type": "application/json",
            "Authorization": f"Bearer {api_key}",
            "User-Agent": f"snipask-bench-image-search/{SCRIPT_VERSION}",
        }
        last_err: Optional[Exception] = None
        for attempt in range(2):
            conn = self._connection()
            t0 = time.perf_counter()
            try:
                conn.request("POST", url_path, body=body, headers=headers)
                resp = conn.getresponse()
                t_headers = time.perf_counter()
                raw = resp.read().decode("utf-8", "replace")
                t_end = time.perf_counter()
                if resp.will_close:
                    self.close()
                return {
                    "status": resp.status,
                    "raw": raw,
                    "first_byte_ms": round((t_headers - t0) * 1000.0, 2),
                    "total_ms": round((t_end - t0) * 1000.0, 2),
                }
            except (ConnectionError, http.client.HTTPException, socket.timeout, TimeoutError) as err:
                last_err = err
                self.close()
                if attempt == 0:
                    continue
                raise
        raise RuntimeError(f"请求失败：{last_err}")

    def post_json(
        self, url: str, payload: Dict[str, Any], headers: Dict[str, str]
    ) -> Dict[str, Any]:
        parts = urlsplit(url)
        cls = http.client.HTTPSConnection if parts.scheme == "https" else http.client.HTTPConnection
        conn = cls(parts.hostname, parts.port or (443 if parts.scheme == "https" else 80), timeout=self.timeout)
        path = parts.path + (f"?{parts.query}" if parts.query else "")
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        t0 = time.perf_counter()
        try:
            conn.request("POST", path, body=data, headers=headers)
            resp = conn.getresponse()
            t_headers = time.perf_counter()
            raw = resp.read().decode("utf-8", "replace")
            t_end = time.perf_counter()
            return {
                "status": resp.status,
                "raw": raw,
                "first_byte_ms": round((t_headers - t0) * 1000.0, 2),
                "total_ms": round((t_end - t0) * 1000.0, 2),
                "url": f"{parts.scheme}://{parts.hostname}{path}",
            }
        finally:
            conn.close()


# --------------------------------------------------------------------------- #
# 检索适配器：Tavily / Serper / mock
# --------------------------------------------------------------------------- #
class SearchProvider:
    name = "mock"

    def search(self, query: str, timeout: float) -> Tuple[List[Dict[str, Any]], Dict[str, Any]]:
        raise NotImplementedError


class MockSearchProvider(SearchProvider):
    name = "mock"

    def search(self, query: str, timeout: float) -> Tuple[List[Dict[str, Any]], Dict[str, Any]]:
        time.sleep(0.02)  # 模拟本地解析/调度，不代表真实网络
        items = [
            {
                "title": f"[MOCK] 关于「{query}」的示例结果（不可信）",
                "url": MOCK_URL_MARK + hashlib.md5(query.encode("utf-8")).hexdigest()[:8],
                "snippet": "这是 mock 检索数据，仅用于验证流水线框架；不得作为结论或 DONE 依据。",
            }
        ]
        return items, {"provider": "mock", "mock": True, "query": query}


class TavilySearchProvider(SearchProvider):
    name = "tavily"

    def __init__(self, api_key: str) -> None:
        self.api_key = api_key
        self.client = HttpClient("https://api.tavily.com", timeout=30)

    def search(self, query: str, timeout: float) -> Tuple[List[Dict[str, Any]], Dict[str, Any]]:
        res = self.client.post_json(
            "https://api.tavily.com/search",
            {"api_key": self.api_key, "query": query, "max_results": 5, "include_answer": False},
            {"Content-Type": "application/json"},
        )
        if res["status"] != 200:
            raise RuntimeError(f"Tavily HTTP {res['status']}: {res['raw'][:300]}")
        obj = json.loads(res["raw"])
        items = [
            {"title": r.get("title", ""), "url": r.get("url", ""), "snippet": r.get("content", "")}
            for r in obj.get("results", [])[:5]
        ]
        return items, {"provider": "tavily", "http_ms": res["total_ms"], "query": query}


class SerperSearchProvider(SearchProvider):
    name = "serper"

    def __init__(self, api_key: str) -> None:
        self.api_key = api_key

    def search(self, query: str, timeout: float) -> Tuple[List[Dict[str, Any]], Dict[str, Any]]:
        client = HttpClient("https://google.serper.dev", timeout=timeout)
        try:
            res = client.post_json(
                "https://google.serper.dev/search",
                {"q": query, "num": 5},
                {"Content-Type": "application/json", "X-API-KEY": self.api_key},
            )
        finally:
            client.close()
        if res["status"] != 200:
            raise RuntimeError(f"Serper HTTP {res['status']}: {res['raw'][:300]}")
        obj = json.loads(res["raw"])
        items = [
            {"title": r.get("title", ""), "url": r.get("link", ""), "snippet": r.get("snippet", "")}
            for r in obj.get("organic", [])[:5]
        ]
        return items, {"provider": "serper", "http_ms": res["total_ms"], "query": query}


# --------------------------------------------------------------------------- #
# mock 模型输出（仅框架自检）
# --------------------------------------------------------------------------- #
MOCK_RECOGNITION = (
    "识别结果：\n"
    "- 文字：[MOCK] 设置页，含「AI 服务」「API Key」「模型」「语言」等字样。\n"
    "- 物体/界面：[MOCK] 应用设置窗口，左侧导航与右侧表单。\n"
    "- 图表：无。\n"
    "检索词：[MOCK] 截图问答工具 AI 服务设置、API Key 配置"
)
MOCK_SUMMARY = (
    "结论：[MOCK] 这是基准确认用的假结论，不代表任何真实检索结果；真实测量需配置视觉模型与检索 Key。\n"
    "来源：未检索到可靠来源（MOCK）"
)


def parse_section(text: str, section: str) -> str:
    """从模型四段式输出中提取某一段的正文。"""
    pattern = re.compile(rf"{section}\s*[：:]\s*(.*?)(?=\n(?:识别结果|检索词|结论|来源)\s*[：:]|\Z)", re.S)
    m = pattern.search(text)
    return (m.group(1).strip() if m else "")


def parse_queries(text: str, mode: str, max_queries: int) -> List[str]:
    raw = parse_section(text, "检索词")
    if not raw:
        # 兜底：整段文本中以「、」分隔的短语
        raw = text
    raw = re.sub(r"^\[MOCK\]\s*", "", raw)
    parts = [p.strip(" 。；;，,") for p in re.split(r"[、\n]", raw) if p.strip(" 。；;，,")]
    cleaned: List[str] = []
    for p in parts:
        if p.startswith(("识别结果", "结论", "来源")):
            continue
        if 2 <= len(p) <= 80:
            cleaned.append(p)
    if mode == "quick":
        return cleaned[:1] or []
    return cleaned[:max_queries]


# --------------------------------------------------------------------------- #
# 一次完整流水线
# --------------------------------------------------------------------------- #
def run_pipeline_once(
    *,
    mode: str,
    image_data_url: str,
    vision_client: HttpClient,
    vision_model: str,
    vision_key: str,
    search: SearchProvider,
    lang: str,
    mock: bool,
    timeout: float,
    template: str,
    extra: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    record: Dict[str, Any] = {"mode": mode, "mock": mock, "ok": False, "stages": {}}
    t_pipeline0 = time.perf_counter()

    # ---- ① 视觉识别（含检索词生成） ----
    phase_a = build_phase_a_prompt(template)
    payload = {
        "model": vision_model,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": phase_a},
                    {"type": "image_url", "image_url": {"url": image_data_url}},
                ],
            }
        ],
        "temperature": 0.0,
        "max_tokens": 700,
    }
    if extra:
        payload.update(extra)
    try:
        if mock:
            time.sleep(0.05)
            vision_text = MOCK_RECOGNITION
            record["stages"]["vision"] = {"ok": True, "total_ms": 50.0, "mock": True}
        else:
            res = vision_client.chat_completions(vision_key, payload)
            if res["status"] != 200:
                raise RuntimeError(f"vision HTTP {res['status']}: {res['raw'][:300]}")
            obj = json.loads(res["raw"])
            vision_text = ((obj.get("choices") or [{}])[0].get("message") or {}).get("content") or ""
            record["stages"]["vision"] = {
                "ok": True,
                "first_byte_ms": res["first_byte_ms"],
                "total_ms": res["total_ms"],
                "usage": obj.get("usage"),
            }
        record["vision_output"] = vision_text[:600]
    except Exception as err:  # noqa: BLE001
        record["stages"]["vision"] = {"ok": False, "error": f"{type(err).__name__}: {err}"}
        record["total_ms"] = round((time.perf_counter() - t_pipeline0) * 1000.0, 2)
        return record

    # ---- ② 生成检索词（本地解析） ----
    t_parse0 = time.perf_counter()
    recognized = parse_section(vision_text, "识别结果") or vision_text
    max_q = 1 if mode == "quick" else 3
    queries = parse_queries(vision_text, mode, max_q)
    if not queries:
        queries = ["截图 内容 识别"] if mock else []
    parse_ms = round((time.perf_counter() - t_parse0) * 1000.0, 3)
    record["queries"] = queries
    record["stages"]["query_gen"] = {"ok": bool(queries), "total_ms": parse_ms, "queries": queries}

    # ---- ③ Search ----
    t_search0 = time.perf_counter()
    results: List[Dict[str, Any]] = []
    search_meta: List[Dict[str, Any]] = []
    try:
        for q in queries:
            items, meta = search.search(q, timeout)
            results.extend(items)
            search_meta.append(meta)
        record["stages"]["search"] = {
            "ok": True,
            "total_ms": round((time.perf_counter() - t_search0) * 1000.0, 2),
            "provider": search.name,
            "queries": len(queries),
            "results": len(results),
            "detail": search_meta,
        }
    except Exception as err:  # noqa: BLE001
        record["stages"]["search"] = {
            "ok": False,
            "error": f"{type(err).__name__}: {err}",
            "total_ms": round((time.perf_counter() - t_search0) * 1000.0, 2),
        }
        record["total_ms"] = round((time.perf_counter() - t_pipeline0) * 1000.0, 2)
        return record
    record["search_results"] = [
        {"title": r.get("title", ""), "url": r.get("url", "")} for r in results[:6]
    ]

    # ---- ④ 摘要 ----
    phase_b = build_phase_b_prompt(recognized, queries, results, lang)
    summary_payload = {
        "model": vision_model,
        "messages": [{"role": "user", "content": phase_b}],
        "temperature": 0.0,
        "max_tokens": 600,
    }
    if extra:
        summary_payload.update(extra)
    try:
        if mock:
            time.sleep(0.05)
            summary_text = MOCK_SUMMARY
            record["stages"]["summary"] = {"ok": True, "total_ms": 50.0, "mock": True}
        else:
            res = vision_client.chat_completions(vision_key, summary_payload)
            if res["status"] != 200:
                raise RuntimeError(f"summary HTTP {res['status']}: {res['raw'][:300]}")
            obj = json.loads(res["raw"])
            summary_text = ((obj.get("choices") or [{}])[0].get("message") or {}).get("content") or ""
            record["stages"]["summary"] = {
                "ok": True,
                "first_byte_ms": res["first_byte_ms"],
                "total_ms": res["total_ms"],
                "usage": obj.get("usage"),
            }
        record["summary_output"] = summary_text[:600]
        record["conclusion"] = parse_section(summary_text, "结论")[:400]
        record["sources"] = [
            ln.strip()
            for ln in parse_section(summary_text, "来源").splitlines()
            if ln.strip()
        ][:6]
    except Exception as err:  # noqa: BLE001
        record["stages"]["summary"] = {"ok": False, "error": f"{type(err).__name__}: {err}"}
        record["total_ms"] = round((time.perf_counter() - t_pipeline0) * 1000.0, 2)
        return record

    record["ok"] = True
    record["total_ms"] = round((time.perf_counter() - t_pipeline0) * 1000.0, 2)
    record["stage_sum_ms"] = round(
        sum(v.get("total_ms", 0) for v in record["stages"].values()), 2
    )
    return record


# --------------------------------------------------------------------------- #
# 统计
# --------------------------------------------------------------------------- #
def pct(values: List[float], p: float) -> float:
    if not values:
        return float("nan")
    s = sorted(values)
    if len(s) == 1:
        return s[0]
    k = (len(s) - 1) * (p / 100.0)
    lo, hi = int(k), min(int(k) + 1, len(s) - 1)
    return s[lo] + (s[hi] - s[lo]) * (k - lo)


def summarize(values: List[float]) -> Dict[str, Any]:
    if not values:
        return {}
    return {
        "count": len(values),
        "mean_ms": round(statistics.fmean(values), 2),
        "p50_ms": round(pct(values, 50), 2),
        "p95_ms": round(pct(values, 95), 2),
        "max_ms": round(max(values), 2),
        "min_ms": round(min(values), 2),
    }


def image_info(path: Path) -> Dict[str, Any]:
    data = path.read_bytes()
    width = height = None
    if data[:8] == b"\x89PNG\r\n\x1a\n" and len(data) >= 24:
        width = int.from_bytes(data[16:20], "big")
        height = int.from_bytes(data[20:24], "big")
    return {
        "path": str(path.relative_to(REPO_ROOT)) if path.is_relative_to(REPO_ROOT) else str(path),
        "size_bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "png_width": width,
        "png_height": height,
        "data_url_chars": len("data:image/png;base64," + base64.b64encode(data).decode("ascii")),
    }


def collect_meta(mock: bool, partial: bool = False) -> Dict[str, Any]:
    commit = "unknown"
    try:
        commit = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=str(REPO_ROOT), capture_output=True, text=True, timeout=10
        ).stdout.strip() or "unknown"
    except Exception:  # noqa: BLE001
        pass
    return {
        "script": "bench_image_search.py",
        "script_version": SCRIPT_VERSION,
        "generated_at": datetime.now(timezone(timedelta(hours=8))).isoformat(),
        "mode": "MOCK_SELF_CHECK"
        if mock
        else ("PARTIAL_REAL_VISION_MOCK_SEARCH" if partial else "REAL"),
        "host": {
            "os": platform.platform(),
            "cpu_count": os.cpu_count(),
            "python": platform.python_version(),
            "hostname": socket.gethostname(),
        },
        "repo_commit": commit,
        "prompt_file": str(PROMPT_FILE.relative_to(REPO_ROOT)),
        "prompt_sha256": hashlib.sha256(PROMPT_FILE.read_bytes()).hexdigest(),
        "thresholds_ms": THRESHOLDS_MS,
        "labels": (
            ["MOCK_SELF_CHECK：数字来自本地 mock，仅验证框架，不得作为 S-06 DONE/FAILED 依据。"]
            if mock
            else (
                [
                    "PARTIAL：视觉识别与摘要为真实 DeepSeek 调用，Search 为 mock（20ms 假延时）；"
                    "不得作为 S-06 验收结论，仅用于估算视觉腿时延。"
                ]
                if partial
                else []
            )
        ),
    }


# --------------------------------------------------------------------------- #
# 主流程
# --------------------------------------------------------------------------- #
def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(
        description="S-06 图片搜索端到端时延基准（视觉识别 → 检索词 → Search → 摘要）",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    vision_base = os.environ.get("SNIPASK_BENCH_VISION_BASE_URL") or os.environ.get("SNIPASK_BENCH_BASE_URL", "")
    vision_model = os.environ.get("SNIPASK_BENCH_VISION_MODEL", "")
    vision_key = os.environ.get("SNIPASK_BENCH_VISION_API_KEY") or os.environ.get("SNIPASK_BENCH_API_KEY", "")
    search_provider = os.environ.get("SNIPASK_BENCH_SEARCH_PROVIDER", "none").lower()
    search_key = os.environ.get("SNIPASK_BENCH_SEARCH_API_KEY", "")

    ap.add_argument("--base-url", default=vision_base)
    ap.add_argument("--model", default=vision_model)
    ap.add_argument("--api-key", default=vision_key)
    ap.add_argument("--search-provider", default=search_provider, choices=["none", "tavily", "serper"])
    ap.add_argument("--search-api-key", default=search_key)
    ap.add_argument("--image", default=str(DEFAULT_IMAGE))
    ap.add_argument("--lang", default="zh-CN")
    ap.add_argument("--modes", default="quick,deep", help="quick / deep / quick,deep")
    ap.add_argument("--runs", type=int, default=20)
    ap.add_argument("--warmup", type=int, default=1)
    ap.add_argument("--timeout", type=float, default=60.0)
    ap.add_argument("--max-queries-deep", type=int, default=3)
    ap.add_argument("--mock-self-check", action="store_true", help="无 Key 时跑 mock 流水线自检")
    ap.add_argument(
        "--allow-mock-search",
        action="store_true",
        help="视觉/摘要走真实端点、Search 用 mock 做部分测量（结果标注 PARTIAL，不得作为验收结论）",
    )
    ap.add_argument("--out", default="")
    args = ap.parse_args(argv)

    image_path = Path(args.image)
    if not image_path.is_absolute():
        image_path = (REPO_ROOT / image_path).resolve()
    if not image_path.exists():
        print(f"BLOCKED：测试图片不存在：{image_path}")
        return EXIT_BLOCKED
    if not PROMPT_FILE.exists():
        print(f"BLOCKED：提示词文件不存在：{PROMPT_FILE}")
        return EXIT_BLOCKED

    modes = [m.strip() for m in args.modes.split(",") if m.strip()]
    if not modes:
        print("--modes 至少需要 quick 或 deep。")
        return EXIT_ERROR

    missing: List[str] = []
    mock = bool(args.mock_self_check)
    partial = False
    if not mock:
        if not args.base_url:
            missing.append("SNIPASK_BENCH_VISION_BASE_URL")
        if not args.model:
            missing.append("SNIPASK_BENCH_VISION_MODEL")
        if not args.api_key:
            missing.append("SNIPASK_BENCH_VISION_API_KEY")
        if args.search_provider != "none" and not args.search_api_key:
            missing.append("SNIPASK_BENCH_SEARCH_API_KEY")
        if missing:
            print("BLOCKED：未提供真实视觉/检索凭据，无法测量 S-06 端到端时延。")
            print(f"缺失：{', '.join(missing)}")
            print("")
            print("解锁步骤（拿到 Key 后约 15 分钟可完成）：")
            print('  $env:SNIPASK_BENCH_VISION_BASE_URL = "https://api.openai.com/v1"   # 任意 OpenAI 兼容 vision 端点')
            print('  $env:SNIPASK_BENCH_VISION_MODEL    = "gpt-4o-mini"                # 或 qwen-vl-max 等')
            print('  $env:SNIPASK_BENCH_VISION_API_KEY  = "<你的 Key>"')
            print('  $env:SNIPASK_BENCH_SEARCH_PROVIDER = "tavily"                     # 或 serper')
            print('  $env:SNIPASK_BENCH_SEARCH_API_KEY  = "<检索 Key>"')
            print("  python scripts/spikes/s05_s06/bench_image_search.py --modes quick,deep --runs 20")
            print("")
            print("本机无任何云端 Key（DeepSeek 空 Key 实测 401），故 S-06 判定 BLOCKED。")
            print("先跑 mock 自检以证明测量框架可跑通（数据为 MOCK，不得作为 DONE 依据）：")
            print("  python scripts/spikes/s05_s06/bench_image_search.py --mock-self-check --runs 3")
            return EXIT_BLOCKED

    file_bytes = image_path.read_bytes()
    data_url = "data:image/png;base64," + base64.b64encode(file_bytes).decode("ascii")
    template = load_image_search_template(args.lang, modes[0])
    try:
        vision_extra = json.loads(os.environ.get("SNIPASK_BENCH_VISION_EXTRA_JSON") or "{}")
    except json.JSONDecodeError as err:
        print(f"BLOCKED：SNIPASK_BENCH_VISION_EXTRA_JSON 不是合法 JSON：{err}")
        return EXIT_ERROR

    if mock:
        print("[MOCK] 使用 mock 视觉模型 + mock Search，仅验证测量框架；数字不可作为 DONE 依据。")
        vision_client = HttpClient("http://127.0.0.1:1/v1", timeout=1)  # 不会真正使用
        search: SearchProvider = MockSearchProvider()
        runs, warmup = max(3, args.runs), 0
    else:
        vision_client = HttpClient(args.base_url, timeout=args.timeout)
        if args.search_provider == "tavily":
            search = TavilySearchProvider(args.search_api_key)
        elif args.search_provider == "serper":
            search = SerperSearchProvider(args.search_api_key)
        elif args.allow_mock_search:
            # 部分真实：视觉/摘要真实调用，Search 用 mock（不满足验收口径，结果标注 PARTIAL）
            search = MockSearchProvider()
            partial = True
            print(
                "[PARTIAL] Search 使用 mock（20ms 假延时）；视觉/摘要为真实调用。"
                "结果标注 PARTIAL_REAL_VISION_MOCK_SEARCH，不得作为 S-06 验收结论。"
            )
        else:
            print(
                "BLOCKED：未配置真实检索服务（SNIPASK_BENCH_SEARCH_PROVIDER=none）。\n"
                "S-06 验收要求「1 次 Search」为真实检索，纯 mock 不能作为 DONE 依据。"
            )
            return EXIT_BLOCKED
        runs, warmup = args.runs, args.warmup

    result: Dict[str, Any] = {
        "meta": collect_meta(mock, partial),
        "config": {
            "base_url_masked": vision_client.masked_url,
            "model": args.model or "mock-vision",
            "search_provider": search.name,
            "vision_extra_param_keys": sorted(vision_extra.keys()),
            "lang": args.lang,
            "modes": modes,
            "runs": runs,
            "warmup": warmup,
            "timeout_sec": args.timeout,
            "image": image_info(image_path),
            "phases": "①视觉识别 ②生成检索词 ③Search ④摘要",
            "prompt_derivation": "resources/prompts/image_search.md 正文按阶段拆分（阶段①识别+检索词；阶段④结论+来源）",
        },
        "modes": {},
    }

    exit_code = EXIT_OK
    for mode in modes:
        template_mode = load_image_search_template(args.lang, mode)
        records: List[Dict[str, Any]] = []
        for i in range(warmup):
            rec = run_pipeline_once(
                mode=mode, image_data_url=data_url, vision_client=vision_client,
                vision_model=args.model or "mock-vision", vision_key=args.api_key or "mock-key",
                search=search, lang=args.lang, mock=mock, timeout=args.timeout,
                template=template_mode, extra=vision_extra,
            )
            rec["warmup"] = True
            records.append(rec)
        for i in range(runs):
            rec = run_pipeline_once(
                mode=mode, image_data_url=data_url, vision_client=vision_client,
                vision_model=args.model or "mock-vision", vision_key=args.api_key or "mock-key",
                search=search, lang=args.lang, mock=mock, timeout=args.timeout,
                template=template_mode, extra=vision_extra,
            )
            rec["warmup"] = False
            records.append(rec)

        measured = [r for r in records if not r.get("warmup") and r.get("ok")]
        failures = [r for r in records if not r.get("warmup") and not r.get("ok")]
        total = [r["total_ms"] for r in measured]
        stage_vals: Dict[str, List[float]] = {}
        for stage in ("vision", "query_gen", "search", "summary"):
            stage_vals[stage] = [
                r["stages"][stage]["total_ms"]
                for r in measured
                if stage in r.get("stages", {}) and r["stages"][stage].get("ok")
            ]
        threshold = THRESHOLDS_MS[mode]
        p95 = pct(total, 95) if total else float("nan")
        if mock:
            # mock 自检永远不给出 DONE/FAILED 验收结论
            verdict = "MOCK_SELF_CHECK_OK" if measured else "MOCK_SELF_CHECK_FAILED"
        elif partial:
            # 部分真实（Search 为 mock）也不给验收结论
            verdict = "PARTIAL_REAL_VISION_MOCK_SEARCH" if measured else "PARTIAL_FAILED"
        else:
            verdict = "DONE" if (measured and p95 <= threshold) else ("BLOCKED" if not measured else "FAILED")
        result["modes"][mode] = {
            "threshold_ms": threshold,
            "measured_runs": len(measured),
            "failures": len(failures),
            "failure_rate": round(len(failures) / runs, 4) if runs else None,
            "total_ms": summarize(total),
            "stages_ms": {k: summarize(v) for k, v in stage_vals.items()},
            "verdict": verdict,
            "records": records,
        }
        print("=" * 74)
        print(f"S-06 图片搜索时延基准  mode={mode}  [{result['meta']['mode']}]")
        print(f"图片：{image_path.name}  视觉模型：{args.model or 'mock-vision'}  检索：{search.name}")
        print(f"运行：warmup={warmup} + measured={runs}  阈值 P95 ≤ {threshold:.0f}ms")
        if total:
            st = result["modes"][mode]["total_ms"]
            print(f"端到端：P50 {st['p50_ms']}ms | P95 {st['p95_ms']}ms | max {st['max_ms']}ms (n={st['count']})")
            for stage in ("vision", "query_gen", "search", "summary"):
                s = result["modes"][mode]["stages_ms"].get(stage) or {}
                if s.get("count"):
                    print(f"  {stage:9s} P50 {s['p50_ms']}ms | P95 {s['p95_ms']}ms | max {s['max_ms']}ms")
            print(f"失败率：{result['modes'][mode]['failure_rate']}  判定：{verdict}")
        else:
            print(f"无成功样本，判定 BLOCKED。失败 {len(failures)}/{runs}")
            if failures:
                print(f"首个失败：{json.dumps(failures[0].get('stages', {}), ensure_ascii=False)[:400]}")
        print("=" * 74)
        if not measured:
            exit_code = EXIT_BLOCKED

    out = Path(args.out) if args.out else (
        DEFAULT_OUT_DIR / f"image_search_{'mock' if mock else 'real'}_{datetime.now().strftime('%Y%m%d-%H%M%S')}.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"原始数据：{out}")
    return exit_code


if __name__ == "__main__":
    if hasattr(sys.stdout, "reconfigure"):
        try:
            sys.stdout.reconfigure(encoding="utf-8")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
