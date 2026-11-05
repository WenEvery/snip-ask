#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S-12 图片搜索可信度基准（四段完整性 / 检索词相关性 / 来源可访问与真实性 / 不确定标注）。

真实四段流水线（对齐 `resources/prompts/image_search.md` 版本 1 与 S-06 真实链路）：
    ① 视觉识别：DeepSeek `deepseek-flash`（thinking disabled）读取 30 张截图，
       输出「识别结果」+「检索词」（阶段 A）
    ② 检索词：本地解析阶段 A 输出（quick 模式 1 组）；可编辑性属产品 UI，本脚本只评生成质量
    ③ 检索：DeepSeek 官方 `POST /v1/tools/search`（count=5，need_content=false）真实联网检索
    ④ 摘要：同一模型基于识别结果 + 真实检索结果输出「结论」+「来源」（阶段 B）

逐图检查（阈值见 docs/技术验证清单.md S-12）：
    1) 四段齐全（识别结果/检索词/结论/来源）          目标 100%
    2) 检索词相关性：命中语料期望关键词，且不是对识别结果的整段复述   目标 100%
    3) 来源可访问率：对「来源」中每个 URL 实测 HEAD（失败回退 GET），超时 8s，
       2xx/3xx 记可访问；404/超时/连接失败记不可访问              目标 ≥95%
    4) 来源真实性（附加）：来源 URL 必须出现在本次 Search 返回结果中，否则记「疑似编造」
    5) 不确定处显式标注：语料标记 expect_uncertain 的图（低清/低对比/虚构产品/无上下文）
       必须在识别或结论中出现中文/英文不确定标记                     目标 100%
    另记录识别结果对语料已知文字片段的命中率（辅助人工复核）。

语料：`scripts/spikes/s12_s14/make_corpus.py` 生成（30 张，文字/图表/商品/代码各分类）。

Key 纪律：只从 `.verify/p1/key.txt`（优先）或环境变量 `SNIPASK_BENCH_API_KEY` 读取；
不打印、不落盘；输出只记录 `api_key_provided` 与脱敏 host。

用法：
    # 无 Key：只做框架自检（MOCK，不联网、不作验收）
    python scripts/spikes/s12_s14/bench_image_search_trust.py --mock-self-check
    # 真实链路（需 Key + 已生成语料）
    python scripts/spikes/s12_s14/bench_image_search_trust.py --limit 30
    # 重新核验已有结果的来源可访问率（不重跑流水线）
    python scripts/spikes/s12_s14/bench_image_search_trust.py --recheck-sources <raw.json>

退出码：0 = 真实链路完成（或 mock 自检通过）；2 = BLOCKED（无 Key/无语料）；3 = 异常。
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple
from urllib.parse import urlsplit, urlunsplit

import requests

SCRIPT_VERSION = "1.0.0"
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
RAW_DIR = REPO_ROOT / ".verify" / "spikes" / "S-12-S-14" / "raw"
CORPUS_JSON = REPO_ROOT / ".verify" / "spikes" / "S-12-S-14" / "corpus" / "corpus.json"
KEY_FILE = REPO_ROOT / ".verify" / "p1" / "key.txt"
PROMPT_FILE = REPO_ROOT / "resources" / "prompts" / "image_search.md"
BODY_MARKER = "以下为发送给模型的正文"

TZ = timezone(timedelta(hours=8))
EXIT_OK, EXIT_BLOCKED, EXIT_ERROR = 0, 2, 3

THRESHOLDS = {
    "four_sections_rate": 1.0,
    "query_relevance_rate": 1.0,
    "source_accessible_rate": 0.95,
    "uncertainty_marked_rate": 1.0,
}

RETRY_STATUS = {408, 429, 500, 502, 503, 504}

# 不确定标注词（中文为主；等价英文也接受）
UNCERTAINTY_MARKERS = [
    "不确定",
    "无法确认",
    "无法确定",
    "无法辨认",
    "无法核实",
    "无法判断",
    "不能确定",
    "可能",
    "疑似",
    "推测",
    "尚不确定",
    "不够清晰",
    "清晰度不足",
    "模糊",
    "看不清",
    "uncertain",
    "not sure",
    "possibly",
    "likely",
    "maybe",
    "unclear",
    "cannot confirm",
    "hard to tell",
    "insufficient",
]


# --------------------------------------------------------------------------- #
# 通用工具
# --------------------------------------------------------------------------- #
def now_iso() -> str:
    return datetime.now(TZ).isoformat()


def stamp() -> str:
    return datetime.now().strftime("%Y%m%d-%H%M%S")


def sha256_file(path: Path) -> Optional[str]:
    import hashlib

    try:
        h = hashlib.sha256()
        with path.open("rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return None


def load_api_key(cli_key: str) -> Tuple[str, str]:
    if cli_key:
        return cli_key, "cli"
    if KEY_FILE.is_file():
        try:
            key = KEY_FILE.read_text(encoding="utf-8").strip().splitlines()[0].strip()
            if key:
                return key, "file:.verify/p1/key.txt"
        except (OSError, IndexError):
            pass
    key = os.environ.get("SNIPASK_BENCH_API_KEY", "").strip()
    if key:
        return key, "env:SNIPASK_BENCH_API_KEY"
    return "", "none"


def mask_base_url(base_url: str) -> str:
    return re.sub(r"^(https?://[^/]+).*$", r"\1", base_url.rstrip("/"))


def chat_completions_url(base_url: str) -> str:
    base = base_url.rstrip("/")
    return base + ("/chat/completions" if base.endswith("/v1") else "/v1/chat/completions")


def tools_url(base_url: str, name: str) -> str:
    base = base_url.rstrip("/")
    suffix = "/tools/" + name
    return base + ("/tools/" + name if base.endswith("/v1") else "/v1" + suffix)


# --------------------------------------------------------------------------- #
# 提示词（image_search.md 正文；阶段 A 识别+检索词，阶段 B 结论+来源）
# --------------------------------------------------------------------------- #
def load_template() -> str:
    raw = PROMPT_FILE.read_text(encoding="utf-8")
    marker = raw.find(BODY_MARKER)
    body = raw[marker:] if marker >= 0 else raw
    if "\n" in body:
        body = body.split("\n", 1)[1]
    return body


def build_phase_a_prompt(template: str) -> str:
    return (
        template.replace("{lang}", "zh-CN").replace("{mode}", "quick")
        + "\n\n【分阶段执行（S-12 可信度基准口径）】\n"
        + "本阶段是流水线第 1 步，只需输出「识别结果」与「检索词」两段，不要输出结论与来源；\n"
        + "识别结果必须如实描述图中可辨认的文字/物体界面/图表，看不清或不确定处直接标注「不确定」；\n"
        + "检索词应是凝练的可搜索关键词（不要复述整段识别结果），并按需做隐私泛化。\n"
        + "格式：\n识别结果：<客观描述；分「文字」「物体/界面」「图表」小点>\n检索词：<1 组，行内用、分隔>\n"
    )


def build_phase_b_prompt(
    recognized: str,
    queries: List[str],
    search_items: List[Dict[str, Any]],
    lang: str = "zh-CN",
) -> str:
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
        "3. 识别或结论中存在不确定处（看不清、信息不足、检索无佐证）时，必须显式标注「不确定」，不要装作确定。",
        "4. 不要寒暄，不要输出图片内容之外的东西。",
        "",
        "【输出格式（只输出这两段，顺序固定）】",
        "结论：<基于检索与识别的结论>",
        "来源：<每行一条「标题 - URL」；没有则写「未检索到可靠来源」>",
    ]
    return "\n".join(lines)


# --------------------------------------------------------------------------- #
# 解析
# --------------------------------------------------------------------------- #
SECTION_NAMES = ("识别结果", "检索词", "结论", "来源")


def parse_section(text: str, section: str) -> str:
    pattern = re.compile(
        rf"{section}\s*[：:]\s*(.*?)(?=\n(?:识别结果|检索词|结论|来源)\s*[：:]|\Z)", re.S
    )
    m = pattern.search(text)
    return m.group(1).strip() if m else ""


def parse_queries(text: str, max_queries: int = 3) -> List[str]:
    raw = parse_section(text, "检索词")
    if not raw:
        raw = text
    parts = [p.strip(" 。；;，,") for p in re.split(r"[、\n]", raw) if p.strip(" 。；;，,")]
    cleaned = []
    for p in parts:
        if p.startswith(("识别结果", "结论", "来源")):
            continue
        if 2 <= len(p) <= 80:
            cleaned.append(p)
    return cleaned[:max_queries]


URL_RE = re.compile(r"https?://[^\s<>\"'）)\]，。；、]+")


def extract_urls(text: str) -> List[str]:
    urls = []
    for match in URL_RE.findall(text or ""):
        url = match.rstrip(".,;:!?)]}>\"'")
        if url and url not in urls:
            urls.append(url)
    return urls


def normalized_url(url: str) -> str:
    """宽松归一化：小写 host、去 fragment、去末尾斜杠（保留 query，用于展示/去重）。"""
    try:
        parts = urlsplit(url.strip())
    except ValueError:
        return url.strip()
    host = (parts.hostname or "").lower()
    path = parts.path.rstrip("/")
    return urlunsplit((parts.scheme.lower(), host, path, parts.query, ""))


def identity_url(url: str) -> str:
    """URL 身份归一化：`scheme://host/path`（忽略 query 与 fragment）。

    来源真实性判定只关心「是否指向同一页面」；追踪参数/查询串差异（如 `?pd_rd_w=...`）
    不应把同一来源误判为编造。该口径在文档 §2 中明确说明。
    """
    try:
        parts = urlsplit(url.strip())
    except ValueError:
        return url.strip()
    host = (parts.hostname or "").lower()
    path = parts.path.rstrip("/") or "/"
    return urlunsplit((parts.scheme.lower(), host, path, "", ""))


def looks_truncated(url: str) -> bool:
    """疑似被模型截断的 URL：以分隔符结尾或末尾百分号编码不完整（正常以 "/" 结尾不算）。"""
    tail = url.strip()[-1:] if url.strip() else ""
    if tail in "-_%&=?:":
        return True
    if re.search(r"%([0-9A-Fa-f])?$", url):
        return True
    return False


def contains_uncertainty(text: str) -> Tuple[bool, List[str]]:
    low = (text or "").lower()
    hits = [m for m in UNCERTAINTY_MARKERS if m.lower() in low]
    return bool(hits), hits


def query_relevance(queries: List[str], expected_terms: List[str], recognition: str) -> Dict[str, Any]:
    query_text = " ".join(queries).lower()
    matched = [t for t in expected_terms if t.lower() in query_text]
    # 「不是对识别结果的整段复述」：检索词总长度应明显短于识别结果，且不是识别结果的子串
    query_join = " ".join(queries).strip()
    not_copy = bool(query_join) and len(query_join) <= max(60, int(len(recognition) * 0.6))
    pass_ = bool(matched) and not_copy
    return {
        "pass": pass_,
        "matched_terms": matched,
        "expected_terms": expected_terms,
        "match_rate": round(len(matched) / len(expected_terms), 4) if expected_terms else None,
        "not_copy_of_recognition": not_copy,
        "query_text": query_join,
    }


def recognition_hit(recognition: str, recognizable: List[str]) -> Dict[str, Any]:
    text = (recognition or "").lower()
    matched = [s for s in recognizable if s.lower() in text]
    return {
        "matched": matched,
        "matched_count": len(matched),
        "expected_count": len(recognizable),
        "rate": round(len(matched) / len(recognizable), 4) if recognizable else None,
    }


# --------------------------------------------------------------------------- #
# HTTP：视觉/摘要 chat + 官方 Search + Fetch（用于来源真实性和可选补充）
# --------------------------------------------------------------------------- #
class ApiClient:
    def __init__(self, base_url: str, api_key: str, timeout: float = 60.0):
        self.base_url = base_url
        self.api_key = api_key
        self.timeout = timeout
        self.session = requests.Session()
        self.session.headers.update(
            {
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
                "User-Agent": f"snipask-bench-s12/{SCRIPT_VERSION}",
            }
        )
        self.chat_url = chat_completions_url(base_url)
        self.search_url = tools_url(base_url, "search")
        self.stats = {"chat_calls": 0, "chat_failures": 0, "search_calls": 0, "search_failures": 0}

    def _post(self, url: str, payload: Dict[str, Any], timeout: float, retries: int = 2) -> Dict[str, Any]:
        attempts = 0
        last: Dict[str, Any] = {}
        for attempt in range(retries + 1):
            attempts = attempt + 1
            t0 = time.perf_counter()
            try:
                resp = self.session.post(url, json=payload, timeout=timeout)
                latency = round((time.perf_counter() - t0) * 1000.0, 2)
                try:
                    body = resp.json()
                except ValueError:
                    body = resp.text[:1500]
                last = {
                    "status": resp.status_code,
                    "latency_ms": latency,
                    "attempts": attempts,
                    "body": body,
                }
                if resp.status_code == 200 or resp.status_code not in RETRY_STATUS:
                    return last
            except requests.RequestException as err:
                last = {
                    "status": None,
                    "latency_ms": round((time.perf_counter() - t0) * 1000.0, 2),
                    "attempts": attempts,
                    "body": f"{type(err).__name__}: {err}",
                }
            if attempt < retries:
                time.sleep(0.5 * (2**attempt))
        return last

    def chat(self, model: str, prompt: str, max_tokens: int = 900) -> Dict[str, Any]:
        self.stats["chat_calls"] += 1
        res = self._post(
            self.chat_url,
            {
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "temperature": 0.0,
                "max_tokens": max_tokens,
                "thinking": {"type": "disabled"},
            },
            self.timeout,
        )
        if res.get("status") != 200:
            self.stats["chat_failures"] += 1
            return {"ok": False, "status": res.get("status"), "error": str(res.get("body"))[:400], "latency_ms": res.get("latency_ms")}
        body = res.get("body") or {}
        content = (((body.get("choices") or [{}])[0]).get("message") or {}).get("content") or ""
        return {
            "ok": True,
            "status": 200,
            "content": content,
            "usage": body.get("usage"),
            "latency_ms": res.get("latency_ms"),
            "attempts": res.get("attempts"),
        }

    def chat_vision(self, model: str, prompt: str, image_data_url: str, max_tokens: int = 900) -> Dict[str, Any]:
        self.stats["chat_calls"] += 1
        res = self._post(
            self.chat_url,
            {
                "model": model,
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            {"type": "text", "text": prompt},
                            {"type": "image_url", "image_url": {"url": image_data_url}},
                        ],
                    }
                ],
                "temperature": 0.0,
                "max_tokens": max_tokens,
                "thinking": {"type": "disabled"},
            },
            self.timeout,
        )
        if res.get("status") != 200:
            self.stats["chat_failures"] += 1
            return {"ok": False, "status": res.get("status"), "error": str(res.get("body"))[:400], "latency_ms": res.get("latency_ms")}
        body = res.get("body") or {}
        content = (((body.get("choices") or [{}])[0]).get("message") or {}).get("content") or ""
        return {
            "ok": True,
            "status": 200,
            "content": content,
            "usage": body.get("usage"),
            "latency_ms": res.get("latency_ms"),
            "attempts": res.get("attempts"),
        }

    def search(self, query: str, count: int = 5) -> Dict[str, Any]:
        self.stats["search_calls"] += 1
        res = self._post(
            self.search_url,
            {"query": query, "count": count, "need_content": False},
            self.timeout,
        )
        if res.get("status") != 200:
            self.stats["search_failures"] += 1
            return {
                "ok": False,
                "status": res.get("status"),
                "error": str(res.get("body"))[:400],
                "latency_ms": res.get("latency_ms"),
                "items": [],
            }
        body = res.get("body") or {}
        items = []
        for r in (body.get("results") or [])[:count]:
            items.append(
                {
                    "title": r.get("title", ""),
                    "url": r.get("url", ""),
                    "snippet": r.get("snippet", ""),
                    "summary": r.get("summary", ""),
                    "date": r.get("date"),
                }
            )
        return {
            "ok": True,
            "status": 200,
            "latency_ms": res.get("latency_ms"),
            "items": items,
            "result_count": len(items),
            "empty": len(items) == 0,
        }

    def close(self) -> None:
        self.session.close()


# --------------------------------------------------------------------------- #
# 来源可访问性（HEAD 优先，失败回退 GET；超时 8s；2xx/3xx = 可访问）
# --------------------------------------------------------------------------- #
def check_url(url: str, timeout: float = 8.0) -> Dict[str, Any]:
    headers = {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
        "(KHTML, like Gecko) Chrome/138.0 Safari/537.36 SnipAsk-S12-SourceCheck"
    }
    last: Dict[str, Any] = {
        "url": url,
        "method": None,
        "status": None,
        "accessible": False,
        "latency_ms": None,
        "final_url": None,
        "error": "no attempt",
    }
    for method in ("HEAD", "GET"):
        t0 = time.perf_counter()
        try:
            resp = requests.request(
                method,
                url,
                timeout=timeout,
                allow_redirects=True,
                headers=headers,
                stream=(method == "GET"),
            )
            latency = round((time.perf_counter() - t0) * 1000.0, 2)
            accessible = 200 <= resp.status_code < 400
            result = {
                "url": url,
                "method": method,
                "status": resp.status_code,
                "accessible": accessible,
                "latency_ms": latency,
                "final_url": resp.url,
                "error": None if accessible else f"HTTP {resp.status_code}",
            }
            resp.close()
            last = result
            # 可访问直接返回；HEAD 的 403/405/500/501 常被服务器拒绝，回退 GET 再判一次
            if accessible or (method == "HEAD" and resp.status_code not in (403, 405, 500, 501)):
                return result
        except requests.RequestException as err:
            last = {
                "url": url,
                "method": method,
                "status": None,
                "accessible": False,
                "latency_ms": round((time.perf_counter() - t0) * 1000.0, 2),
                "final_url": None,
                "error": f"{type(err).__name__}: {str(err)[:200]}",
            }
            if method == "GET":
                return last
    return last


def classify_source_results(results: List[Dict[str, Any]]) -> Dict[str, Any]:
    """把不可访问结果分为：客观失效（4xx/5xx）、反爬拒绝（401/403/429）、网络不可达。

    反爬与网络不可达在本机脚本口径下「无法判定」，文档同时给出三种口径：
      - raw：2xx/3xx 才可访问（清单字面口径）
      - excluding_unverifiable：反爬/网络不计入分母
      - optimistic：反爬/网络视为「浏览器可访问」的上界
    """
    buckets = {"dead_http": [], "antibot": [], "network": []}
    for r in results:
        if r.get("accessible"):
            continue
        status = r.get("status")
        if status in (401, 403, 429):
            buckets["antibot"].append(r)
        elif status is not None:
            buckets["dead_http"].append(r)
        else:
            buckets["network"].append(r)
    return {name: {"count": len(items), "urls": [r["url"] for r in items]} for name, items in buckets.items()}


def check_sources(urls: List[str], timeout: float = 8.0, workers: int = 6) -> Dict[str, Any]:
    unique = list(dict.fromkeys(urls))
    if not unique:
        return {
            "checked": 0,
            "accessible": 0,
            "accessible_rate": None,
            "results": [],
            "failure_buckets": {"dead_http": {"count": 0, "urls": []}, "antibot": {"count": 0, "urls": []}, "network": {"count": 0, "urls": []}},
            "rates": {"raw": None, "excluding_unverifiable": None, "optimistic": None},
        }
    with ThreadPoolExecutor(max_workers=max(1, min(workers, len(unique)))) as pool:
        results = list(pool.map(lambda u: check_url(u, timeout), unique))
    accessible = sum(1 for r in results if r["accessible"])
    buckets = classify_source_results(results)
    antibot = buckets["antibot"]["count"]
    network = buckets["network"]["count"]
    dead = buckets["dead_http"]["count"]
    total = len(results)

    def rnd(num: float) -> Optional[float]:
        return round(num, 4) if total else None

    return {
        "checked": total,
        "accessible": accessible,
        "accessible_rate": rnd(accessible / total),
        "failure_buckets": buckets,
        "rates": {
            "raw": rnd(accessible / total),
            "excluding_unverifiable": rnd(accessible / (total - antibot - network)) if (total - antibot - network) else None,
            "optimistic": rnd((accessible + antibot + network) / total),
        },
        "results": results,
    }


# --------------------------------------------------------------------------- #
# mock（仅框架自检；不联网）
# --------------------------------------------------------------------------- #
MOCK_URL_PREFIX = "https://example.invalid/mock-search/"


def mock_vision(item: Dict[str, Any]) -> str:
    lines = ["识别结果："]
    lines.append(f"- 文字：[MOCK] {item['ground_truth'][:120]}")
    lines.append("- 物体/界面：[MOCK] 由语料元数据生成的占位识别")
    lines.append("- 图表：无" if item["category"] != "chart" else "- 图表：[MOCK] 占位图表描述")
    if item["expect_uncertain"]:
        lines.append("- 不确定：该图文字/图例偏小，部分内容识别不确定")
    queries = "、".join(item["expected_query_terms"][:2]) or "截图 内容"
    lines.append(f"检索词：{queries}")
    return "\n".join(lines)


def mock_summary(item: Dict[str, Any]) -> str:
    conclusion = f"结论：[MOCK] {item['ground_truth']}"
    if item["expect_uncertain"]:
        conclusion += "（部分内容不确定）"
    sources = "\n".join(
        f"{item['name']} 检索结果 {i} - {MOCK_URL_PREFIX}{item['id']}/{i}" for i in (1, 2)
    )
    return f"{conclusion}\n来源：\n{sources}"


def strip_uncertainty(text: str) -> str:
    """mock-fail 臂用：删除所有不确定标记词（验证检查能检出「未标注」）。"""
    out = text
    for marker in UNCERTAINTY_MARKERS:
        out = out.replace(marker, "")
    return out


def run_mock_pipeline(items: List[Dict[str, Any]], mock_fail: bool) -> Dict[str, Any]:
    records = []
    for idx, item in enumerate(items):
        vision_text = mock_vision(item)
        summary_text = mock_summary(item)
        if mock_fail and idx == 0:
            summary_text = summary_text.split("\n来源：")[0]  # 故意缺「来源」段
        if mock_fail and item["expect_uncertain"]:
            # 故意去掉所有不确定标记词 → 检查应检出「应标注而未标注」
            summary_text = strip_uncertainty(summary_text)
            vision_text = strip_uncertainty(vision_text)
        recognized = parse_section(vision_text, "识别结果")
        queries = parse_queries(vision_text)
        results = [
            {"title": f"{item['name']} 检索结果 1", "url": f"{MOCK_URL_PREFIX}{item['id']}/1", "snippet": ""},
            {"title": f"{item['name']} 检索结果 2", "url": f"{MOCK_URL_PREFIX}{item['id']}/2", "snippet": ""},
        ]
        record = build_record(
            item,
            vision=vision_text,
            summary=summary_text,
            queries=queries,
            recognized=recognized,
            search_items=results,
            search_meta={"provider": "mock", "results": len(results), "empty": False},
            timings={"mock": True},
            mock=True,
        )
        records.append(record)
    checks = check_sources([u for r in records for u in r["sources"]["urls"]], timeout=3.0)
    return {"records": records, "source_check": checks, "mock": True}


# --------------------------------------------------------------------------- #
# 单图检查
# --------------------------------------------------------------------------- #
def build_record(
    item: Dict[str, Any],
    *,
    vision: str,
    summary: str,
    queries: List[str],
    recognized: str,
    search_items: List[Dict[str, Any]],
    search_meta: Dict[str, Any],
    timings: Dict[str, Any],
    mock: bool,
) -> Dict[str, Any]:
    conclusion = parse_section(summary, "结论")
    source_section = parse_section(summary, "来源")
    urls = extract_urls(source_section)
    if not urls and source_section:
        urls = extract_urls(summary)

    sections = {
        "识别结果": bool(recognized.strip()),
        "检索词": bool(queries),
        "结论": bool(conclusion.strip()),
        "来源": bool(source_section.strip()),
    }
    four_complete = all(sections.values())

    relevance = query_relevance(queries, item["expected_query_terms"], recognized)
    recognition = recognition_hit(recognized, item["recognizable_content"])
    search_ids = {identity_url(i["url"]) for i in search_items if i.get("url")}
    in_search = [u for u in urls if identity_url(u) in search_ids]
    fabricated = [u for u in urls if identity_url(u) not in search_ids]
    truncated = [u for u in urls if looks_truncated(u)]

    uncertain_in_recognition, hits_rec = contains_uncertainty(recognized)
    uncertain_in_conclusion, hits_con = contains_uncertainty(conclusion)
    uncertain_marked = uncertain_in_recognition or uncertain_in_conclusion

    return {
        "id": item["id"],
        "category": item["category"],
        "name": item["name"],
        "difficulty": item["difficulty"],
        "expect_uncertain": item["expect_uncertain"],
        "mock": mock,
        "ground_truth": item["ground_truth"],
        "image": item["png"],
        "vision_output": vision,
        "summary_output": summary,
        "recognized": recognized,
        "queries": queries,
        "search": {
            "provider": search_meta.get("provider"),
            "result_count": len(search_items),
            "empty": search_meta.get("empty"),
            "results": [{"title": i.get("title", ""), "url": i.get("url", "")} for i in search_items[:6]],
        },
        "conclusion": conclusion,
        "source_section": source_section,
        "sources": {
            "urls": urls,
            "in_search_results": in_search,
            "fabricated": fabricated,
            "truncated": truncated,
        },
        "checks": {
            "four_sections": {"sections": sections, "complete": four_complete},
            "query_relevance": relevance,
            "recognition_hit": recognition,
            "source_authenticity": {
                "total": len(urls),
                "in_search_results": len(in_search),
                "fabricated_count": len(fabricated),
                "fabricated": fabricated,
                "truncated_count": len(truncated),
                "truncated": truncated,
                "pass": len(fabricated) == 0,
            },
            "uncertainty": {
                "expect_uncertain": item["expect_uncertain"],
                "marked": uncertain_marked,
                "markers_recognition": hits_rec,
                "markers_conclusion": hits_con,
                "pass": (not item["expect_uncertain"]) or uncertain_marked,
            },
        },
        "timings": timings,
    }


def score_records(records: List[Dict[str, Any]]) -> Dict[str, Any]:
    total = len(records)
    four = sum(1 for r in records if r["checks"]["four_sections"]["complete"])
    relevance = sum(1 for r in records if r["checks"]["query_relevance"]["pass"])
    authenticity = sum(1 for r in records if r["checks"]["source_authenticity"]["pass"])
    fabricated_total = sum(r["checks"]["source_authenticity"]["fabricated_count"] for r in records)
    truncated_total = sum(r["checks"]["source_authenticity"].get("truncated_count", 0) for r in records)
    source_urls_total = sum(len(r["sources"]["urls"]) for r in records)
    uncertainty_items = [r for r in records if r["expect_uncertain"]]
    uncertainty_ok = sum(1 for r in uncertainty_items if r["checks"]["uncertainty"]["pass"])

    def rate(num: int, den: int) -> Optional[float]:
        return round(num / den, 4) if den else None

    by_category: Dict[str, Dict[str, Any]] = {}
    for cat in sorted({r["category"] for r in records}):
        subset = [r for r in records if r["category"] == cat]
        by_category[cat] = {
            "images": len(subset),
            "four_sections_ok": sum(1 for r in subset if r["checks"]["four_sections"]["complete"]),
            "query_relevance_ok": sum(1 for r in subset if r["checks"]["query_relevance"]["pass"]),
            "source_authenticity_ok": sum(1 for r in subset if r["checks"]["source_authenticity"]["pass"]),
            "recognition_hit_mean": round(
                sum(r["checks"]["recognition_hit"]["rate"] or 0 for r in subset) / len(subset), 4
            ),
            "sources_total": sum(len(r["sources"]["urls"]) for r in subset),
        }

    summary = {
        "images": total,
        "four_sections": {"ok": four, "rate": rate(four, total), "target": THRESHOLDS["four_sections_rate"]},
        "query_relevance": {"ok": relevance, "rate": rate(relevance, total), "target": THRESHOLDS["query_relevance_rate"]},
        "source_authenticity": {
            "ok": authenticity,
            "rate": rate(authenticity, total),
            "fabricated_urls": fabricated_total,
            "truncated_urls": truncated_total,
            "cited_urls_total": source_urls_total,
            "target": 1.0,
        },
        "uncertainty": {
            "expected_images": len(uncertainty_items),
            "marked": uncertainty_ok,
            "rate": rate(uncertainty_ok, len(uncertainty_items)),
            "target": THRESHOLDS["uncertainty_marked_rate"],
        },
        "recognition_hit_mean": round(
            sum(r["checks"]["recognition_hit"]["rate"] or 0 for r in records) / total, 4
        )
        if total
        else None,
        "by_category": by_category,
    }
    return summary


def verdict_for(summary: Dict[str, Any], source_check: Dict[str, Any], *, mock: bool) -> Dict[str, Any]:
    accessible_rate = source_check.get("accessible_rate")
    rates = source_check.get("rates") or {}
    met_sections = (summary["four_sections"]["rate"] or 0) >= THRESHOLDS["four_sections_rate"]
    met_relevance = (summary["query_relevance"]["rate"] or 0) >= THRESHOLDS["query_relevance_rate"]
    met_sources = accessible_rate is not None and accessible_rate >= THRESHOLDS["source_accessible_rate"]
    met_uncertainty = (summary["uncertainty"]["rate"] if summary["uncertainty"]["expected_images"] else 1.0) >= THRESHOLDS["uncertainty_marked_rate"]
    met_authenticity = (
        summary["source_authenticity"]["fabricated_urls"] == 0
        and summary["source_authenticity"].get("truncated_urls", 0) == 0
    )
    all_met = met_sections and met_relevance and met_sources and met_uncertainty and met_authenticity
    if mock:
        verdict = "MOCK_SELF_CHECK_OK"
    else:
        verdict = "DONE" if all_met else "FAILED"
    return {
        "verdict": verdict,
        "targets": {
            "four_sections": met_sections,
            "query_relevance": met_relevance,
            "source_accessible": met_sources,
            "uncertainty_marked": met_uncertainty,
            "no_fabricated_sources": met_authenticity,
        },
        "source_accessible_rate": accessible_rate,
        "source_accessible_rates": rates,
        "source_accessible_target": THRESHOLDS["source_accessible_rate"],
        "all_met": all_met,
        "note": "按清单字面口径（2xx/3xx）判定；反爬/网络不可达的拆分见 source_check.failure_buckets 与 rates。",
    }


# --------------------------------------------------------------------------- #
# 真实流水线
# --------------------------------------------------------------------------- #
def image_data_url(png_path: Path) -> str:
    data = png_path.read_bytes()
    return "data:image/png;base64," + base64.b64encode(data).decode("ascii")


def run_real_pipeline(items: List[Dict[str, Any]], args: argparse.Namespace, api_key: str) -> Dict[str, Any]:
    client = ApiClient(args.base_url, api_key, timeout=args.timeout)
    template = load_template()
    phase_a = build_phase_a_prompt(template)
    records: List[Dict[str, Any]] = []
    try:
        for idx, item in enumerate(items, 1):
            png = REPO_ROOT / item["png"]
            if not png.is_file():
                records.append(
                    {
                        "id": item["id"],
                        "category": item["category"],
                        "name": item["name"],
                        "error": f"缺截图 {item['png']}",
                        "mock": False,
                    }
                )
                print(f"[{idx}/{len(items)}] {item['id']:<24} FAIL 缺截图")
                continue
            t0 = time.perf_counter()
            vision = client.chat_vision(args.model, phase_a, image_data_url(png))
            if not vision["ok"]:
                records.append(
                    {
                        "id": item["id"],
                        "category": item["category"],
                        "name": item["name"],
                        "error": f"vision 失败：{vision.get('error')}",
                        "mock": False,
                    }
                )
                print(f"[{idx}/{len(items)}] {item['id']:<24} FAIL vision {vision.get('error')}")
                continue
            vision_text = vision["content"]
            recognized = parse_section(vision_text, "识别结果") or vision_text
            queries = parse_queries(vision_text, max_queries=1)
            search = client.search(queries[0], count=args.search_count) if queries else {
                "ok": False, "error": "无检索词", "items": [], "result_count": 0, "empty": True
            }
            search_items = search.get("items") or []
            phase_b = build_phase_b_prompt(recognized, queries, search_items)
            summary = client.chat(args.model, phase_b)
            if not summary["ok"]:
                records.append(
                    {
                        "id": item["id"],
                        "category": item["category"],
                        "name": item["name"],
                        "error": f"summary 失败：{summary.get('error')}",
                        "mock": False,
                    }
                )
                print(f"[{idx}/{len(items)}] {item['id']:<24} FAIL summary")
                continue
            record = build_record(
                item,
                vision=vision_text,
                summary=summary["content"],
                queries=queries,
                recognized=recognized,
                search_items=search_items,
                search_meta={"provider": "deepseek", "results": len(search_items), "empty": search.get("empty")},
                timings={
                    "vision_ms": vision.get("latency_ms"),
                    "search_ms": search.get("latency_ms"),
                    "summary_ms": summary.get("latency_ms"),
                    "total_ms": round((time.perf_counter() - t0) * 1000.0, 1),
                },
                mock=False,
            )
            records.append(record)
            print(
                f"[{idx}/{len(items)}] {item['id']:<24} "
                f"四段={'OK' if record['checks']['four_sections']['complete'] else 'MISS'} "
                f"检索词={'OK' if record['checks']['query_relevance']['pass'] else 'WEAK'} "
                f"来源={len(record['sources']['urls'])} 疑似编造={record['checks']['source_authenticity']['fabricated_count']} "
                f"不确定标注={'OK' if record['checks']['uncertainty']['pass'] else 'MISS'}"
            )
    finally:
        client.close()
    urls = [u for r in records if r.get("sources") for u in r["sources"]["urls"]]
    source_check = check_sources(urls, timeout=args.source_timeout, workers=args.source_workers)
    return {"records": records, "source_check": source_check, "mock": False, "client_stats": client.stats}


# --------------------------------------------------------------------------- #
# 汇总输出
# --------------------------------------------------------------------------- #
def failure_samples(records: List[Dict[str, Any]], limit: int = 8) -> List[Dict[str, Any]]:
    out = []
    for r in records:
        if r.get("error"):
            out.append({"id": r["id"], "reasons": [r["error"]]})
            continue
        reasons = []
        if not r["checks"]["four_sections"]["complete"]:
            missing = [k for k, v in r["checks"]["four_sections"]["sections"].items() if not v]
            reasons.append(f"四段缺失：{missing}")
        if not r["checks"]["query_relevance"]["pass"]:
            qr = r["checks"]["query_relevance"]
            reasons.append(f"检索词相关性不足：命中 {qr['matched_terms']} / 期望 {qr['expected_terms']}（非复述={qr['not_copy_of_recognition']}）")
        if not r["checks"]["source_authenticity"]["pass"]:
            sa = r["checks"]["source_authenticity"]
            if sa.get("fabricated_count"):
                reasons.append(f"来源不在检索结果中（疑似编造）：{sa['fabricated_count']} 条")
            if sa.get("truncated_count"):
                reasons.append(f"来源 URL 疑似截断：{sa['truncated_count']} 条")
        if not r["checks"]["uncertainty"]["pass"]:
            reasons.append("该图应标注不确定但未标注")
        if reasons:
            out.append({"id": r["id"], "category": r["category"], "name": r["name"], "reasons": reasons})
        if len(out) >= limit:
            break
    return out


def print_summary(result: Dict[str, Any]) -> None:
    s = result["summary"]
    verdict = result["verdict"]
    print("=" * 84)
    print("S-12 图片搜索可信度汇总")
    print(f"模式：{result['meta']['mode']}")
    print(f"四段齐全：{s['four_sections']['ok']}/{s['images']} = {s['four_sections']['rate']}（目标 100%）")
    print(f"检索词相关性：{s['query_relevance']['ok']}/{s['images']} = {s['query_relevance']['rate']}（目标 100%）")
    src = result["source_check"]
    print(f"来源可访问率（清单口径）：{src['accessible']}/{src['checked']} = {src['accessible_rate']}（目标 ≥95%，超时 8s）")
    buckets = src.get("failure_buckets") or {}
    if buckets:
        print(
            f"  不可访问拆分：客观失效 {buckets['dead_http']['count']} / 反爬拒绝 {buckets['antibot']['count']} "
            f"/ 网络不可达 {buckets['network']['count']}"
        )
    rates = src.get("rates") or {}
    if rates:
        print(
            f"  三口径：raw={rates.get('raw')}；剔除反爬/网络={rates.get('excluding_unverifiable')}；"
            f"反爬/网络视为可访问上界={rates.get('optimistic')}"
        )
    print(
        f"来源真实性：疑似编造 {s['source_authenticity']['fabricated_urls']} 条 / "
        f"截断 {s['source_authenticity'].get('truncated_urls', 0)} 条（目标 0）"
    )
    print(f"不确定标注：{s['uncertainty']['marked']}/{s['uncertainty']['expected_images']} = {s['uncertainty']['rate']}（目标 100%）")
    print(f"识别已知文字命中率（辅助）：{s['recognition_hit_mean']}")
    print(f"判定：{verdict['verdict']}  {verdict['targets']}")
    print("-" * 84)
    fail = result["failures"]
    if fail:
        print(f"典型失败 / 风险样本（前 {len(fail)} 条）：")
        for item in fail:
            print(f"  - {item['id']}（{item.get('category', '')}）：{'；'.join(item['reasons'])}")
    else:
        print("无失败样本。")
    print(f"原始数据：{result['meta']['out_path']}")
    print("=" * 84)


def write_result(result: Dict[str, Any], out_path: Path) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    result["meta"]["out_path"] = str(out_path)
    out_path.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")


# --------------------------------------------------------------------------- #
# 主流程
# --------------------------------------------------------------------------- #
def load_corpus(limit: int = 0) -> List[Dict[str, Any]]:
    data = json.loads(CORPUS_JSON.read_text(encoding="utf-8"))
    items = data["items"]
    return items[:limit] if limit > 0 else items


def collect_meta(mode: str, args: argparse.Namespace, items: List[Dict[str, Any]], key_source: str) -> Dict[str, Any]:
    return {
        "script": "bench_image_search_trust.py",
        "script_version": SCRIPT_VERSION,
        "generated_at": now_iso(),
        "mode": mode,
        "host": {
            "os": f"{sys.platform}",
            "python": sys.version.split()[0],
            "requests": requests.__version__,
        },
        "corpus": {
            "path": str(CORPUS_JSON.relative_to(REPO_ROOT)).replace("\\", "/"),
            "images": len(items),
            "categories": {
                cat: sum(1 for i in items if i["category"] == cat)
                for cat in sorted({i["category"] for i in items})
            },
            "expect_uncertain": sum(1 for i in items if i["expect_uncertain"]),
        },
        "prompt": {
            "path": "resources/prompts/image_search.md",
            "sha256": sha256_file(PROMPT_FILE),
            "phases": "阶段A=识别结果+检索词；阶段B=结论+来源（与 S-06 真实链路口径一致）",
        },
        "model": {
            "vision": args.model,
            "summary": args.model,
            "thinking": "disabled",
            "temperature": 0.0,
            "base_url_masked": mask_base_url(args.base_url),
            "api_key_provided": bool(key_source not in ("", "none")),
            "api_key_source": key_source,
        },
        "search": {
            "endpoint": tools_url(args.base_url, "search"),
            "count": args.search_count,
            "provider": "deepseek" if mode == "REAL" else "mock",
        },
        "thresholds": THRESHOLDS,
        "notes": [
            "检索词可编辑性属产品 UI 能力，本基准只评生成质量（相关性 / 非整段复述）。",
            "来源可访问性为脚本实测 HEAD/GET（8s 超时），2xx/3xx 记可访问；反爬/网络不可达在 failure_buckets 单独统计。",
            "来源真实性 = 来源 URL 与本次 Search 返回结果按 scheme://host/path 身份比对（忽略 query/fragment），"
            "模型截断/改写 URL 单独计 truncated；用于识别视觉/摘要幻觉。",
            "mock 数据只验证检查框架，不得作为 S-12 验收依据。",
        ],
    }


def rescore_file(path: Path, out_path: Optional[Path] = None) -> int:
    """离线重评分：只基于已保存的模型输出重算检查项（不调用 LLM、不重发检索、不重测来源）。

    用途：修正检查器口径（如 URL 身份归一化）后，对同一批真实输出复算，避免重复调用付费接口。
    """
    data = json.loads(path.read_text(encoding="utf-8"))
    records = data.get("records") or []
    if not records:
        print(f"BLOCKED：{path} 中没有 records")
        return EXIT_BLOCKED
    rescored = 0
    for rec in records:
        if not rec.get("checks") or not rec.get("source_section") or not rec.get("search"):
            continue
        search_items = [{"url": u["url"], "title": u.get("title", "")} for u in rec["search"].get("results", [])]
        urls = extract_urls(rec["source_section"]) or extract_urls(rec.get("summary_output") or "")
        search_ids = {identity_url(i["url"]) for i in search_items if i.get("url")}
        in_search = [u for u in urls if identity_url(u) in search_ids]
        fabricated = [u for u in urls if identity_url(u) not in search_ids]
        truncated = [u for u in urls if looks_truncated(u)]
        rec["sources"] = {
            "urls": urls,
            "in_search_results": in_search,
            "fabricated": fabricated,
            "truncated": truncated,
        }
        rec["checks"]["source_authenticity"] = {
            "total": len(urls),
            "in_search_results": len(in_search),
            "fabricated_count": len(fabricated),
            "fabricated": fabricated,
            "truncated_count": len(truncated),
            "truncated": truncated,
            "pass": len(fabricated) == 0,
        }
        rescored += 1

    summary = score_records(records)
    source_check = data.get("source_check") or {}
    if source_check.get("results"):
        source_check["failure_buckets"] = classify_source_results(source_check["results"])
        antibot = source_check["failure_buckets"]["antibot"]["count"]
        network = source_check["failure_buckets"]["network"]["count"]
        total = source_check["checked"]
        accessible = source_check["accessible"]
        source_check["rates"] = {
            "raw": round(accessible / total, 4) if total else None,
            "excluding_unverifiable": round(accessible / (total - antibot - network), 4)
            if (total - antibot - network)
            else None,
            "optimistic": round((accessible + antibot + network) / total, 4) if total else None,
        }
    verdict = verdict_for(summary, source_check, mock=False)
    data["records"] = records
    data["source_check"] = source_check
    data["summary"] = summary
    data["verdict"] = verdict
    data["failures"] = failure_samples(records)
    data["meta"]["mode"] = "REAL_RESCORED"
    data["meta"]["rescored_from"] = str(path)
    data["meta"]["rescored_at"] = now_iso()
    data["meta"]["rescore_note"] = (
        "离线重评分：同一次真实运行的模型输出，按修正后的来源真实性口径（URL 身份=scheme://host/path，"
        "忽略 query/fragment，另计截断 URL）与来源失败分桶重算；未重新调用模型/检索接口，来源可访问性沿用原实测结果。"
    )
    data["meta"]["rescored_records"] = rescored
    target = out_path or path.with_name(path.stem + "_rescored.json")
    write_result(data, target)
    print_summary(data)
    print(f"重评分完成：{target}（records={rescored}）")
    return EXIT_OK


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description="S-12 图片搜索可信度（四段 / 检索词 / 来源 / 不确定标注）")
    ap.add_argument("--base-url", default=os.environ.get("SNIPASK_BENCH_BASE_URL", "https://api.deepseek.com"))
    ap.add_argument("--model", default=os.environ.get("SNIPASK_BENCH_VISION_MODEL", "deepseek-flash"))
    ap.add_argument("--api-key", default="")
    ap.add_argument("--search-count", type=int, default=5)
    ap.add_argument("--timeout", type=float, default=60.0, help="LLM/Search 请求超时（秒）")
    ap.add_argument("--source-timeout", type=float, default=8.0, help="来源 URL 可访问性实测超时（秒）")
    ap.add_argument("--source-workers", type=int, default=6)
    ap.add_argument("--limit", type=int, default=0, help="只跑前 N 张（调试）")
    ap.add_argument("--mock-self-check", action="store_true", help="无 Key 框架自检（MOCK，不作验收）")
    ap.add_argument("--rescore", default="", help="离线重评分指定 JSON（不调用模型/检索）")
    ap.add_argument("--out", default="")
    args = ap.parse_args(argv)

    if args.rescore:
        return rescore_file(Path(args.rescore), Path(args.out) if args.out else None)

    if not CORPUS_JSON.is_file():
        print(f"BLOCKED：语料清单不存在 {CORPUS_JSON}")
        print("  解锁：python scripts/spikes/s12_s14/make_corpus.py")
        return EXIT_BLOCKED
    items = load_corpus(args.limit)
    if not items:
        print("BLOCKED：语料为空")
        return EXIT_BLOCKED

    if args.mock_self_check:
        pass_result = run_mock_pipeline(items, mock_fail=False)
        fail_result = run_mock_pipeline(items, mock_fail=True)
        pass_summary = score_records(pass_result["records"])
        fail_summary = score_records(fail_result["records"])
        pass_verdict = verdict_for(pass_summary, pass_result["source_check"], mock=True)
        fail_verdict = verdict_for(fail_summary, fail_result["source_check"], mock=True)
        assertions = [
            {
                "name": "mock-pass 四段齐全 100%",
                "ok": pass_summary["four_sections"]["rate"] == 1.0,
                "detail": f"{pass_summary['four_sections']['ok']}/{pass_summary['images']}",
            },
            {
                "name": "mock-pass 检索词相关性 100%",
                "ok": pass_summary["query_relevance"]["rate"] == 1.0,
                "detail": f"{pass_summary['query_relevance']['ok']}/{pass_summary['images']}",
            },
            {
                "name": "mock-pass 不确定标注 100%（expect_uncertain 图）",
                "ok": pass_summary["uncertainty"]["rate"] == 1.0,
                "detail": f"{pass_summary['uncertainty']['marked']}/{pass_summary['uncertainty']['expected_images']}",
            },
            {
                "name": "mock-pass 来源真实性检查执行（0 条编造）",
                "ok": pass_summary["source_authenticity"]["fabricated_urls"] == 0
                and pass_result["source_check"]["checked"] > 0,
                "detail": f"编造={pass_summary['source_authenticity']['fabricated_urls']} 检查URL={pass_result['source_check']['checked']}",
            },
            {
                "name": "mock-fail 失败被检出（缺来源段 + 缺不确定标注）",
                "ok": fail_summary["four_sections"]["rate"] < 1.0
                and fail_summary["uncertainty"]["rate"] < 1.0,
                "detail": f"四段={fail_summary['four_sections']['rate']} 不确定={fail_summary['uncertainty']['rate']}",
            },
        ]
        ok = all(a["ok"] for a in assertions)
        out_path = Path(args.out) if args.out else RAW_DIR / f"image_search_trust_mock_{stamp()}.json"
        result = {
            "meta": collect_meta("MOCK_SELF_TEST", args, items, "none"),
            "mock_pass": {"summary": pass_summary, "verdict": pass_verdict},
            "mock_fail": {"summary": fail_summary, "verdict": fail_verdict},
            "self_check_assertions": assertions,
            "self_check_ok": ok,
        }
        write_result(result, out_path)
        print("=" * 84)
        print("S-12 图片搜索可信度 —— MOCK 框架自检（非验收数据）")
        for a in assertions:
            print(f"  [{'PASS' if a['ok'] else 'FAIL'}] {a['name']}：{a['detail']}")
        print(f"  原始数据：{out_path}")
        print("=" * 84)
        return EXIT_OK if ok else EXIT_ERROR

    # 真实链路
    api_key, key_source = load_api_key(args.api_key)
    if not api_key:
        print("BLOCKED：未提供 DeepSeek Key。")
        print("  解锁步骤：")
        print("    1) 将 Key 写入 .verify/p1/key.txt（仅此文件，不得提交到仓库）；或")
        print("       设置环境变量 SNIPASK_BENCH_API_KEY（仅当前会话）。")
        print("    2) 生成语料：python scripts/spikes/s12_s14/make_corpus.py")
        print("    3) 运行：python scripts/spikes/s12_s14/bench_image_search_trust.py --limit 30")
        return EXIT_BLOCKED

    run = run_real_pipeline(items, args, api_key)
    summary = score_records(run["records"])
    verdict = verdict_for(summary, run["source_check"], mock=False)
    out_path = Path(args.out) if args.out else RAW_DIR / f"image_search_trust_{stamp()}.json"
    result = {
        "meta": collect_meta("REAL", args, items, key_source),
        "client_stats": run.get("client_stats"),
        "records": run["records"],
        "source_check": run["source_check"],
        "summary": summary,
        "verdict": verdict,
        "failures": failure_samples(run["records"]),
    }
    write_result(result, out_path)
    print_summary(result)
    return EXIT_OK


if __name__ == "__main__":
    if hasattr(sys.stdout, "reconfigure"):
        try:
            sys.stdout.reconfigure(encoding="utf-8")
        except Exception:  # noqa: BLE001
            pass
    try:
        sys.exit(main())
    except Exception as exc:  # noqa: BLE001
        print(f"脚本异常：{type(exc).__name__}: {exc}")
        sys.exit(EXIT_ERROR)
