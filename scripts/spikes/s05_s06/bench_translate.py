#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S-05 云端翻译链路时延基准（OpenAI 兼容 chat/completions）。

被测链路：200 字中文样例 → 按 resources/prompts/translate.md（版本 1）渲染提示词
→ POST {base_url}/chat/completions → 记录 HTTP 状态 / 首字节 / 总耗时 / token 用量。

DONE 阈值（技术验证清单 §2）：云端 200 字 ≤ 1.5s（P95，非流式端到端）。
本地 1.8B Q4 阈值由 bench_local_translate.mjs 负责。

环境变量（三选一或全部）：
    SNIPASK_BENCH_BASE_URL   例如 https://api.deepseek.com/v1
    SNIPASK_BENCH_MODEL      例如 deepseek-chat / qwen-mt-turbo
    SNIPASK_BENCH_API_KEY    API Key（脚本绝不打印完整 Key）
可选：
    SNIPASK_BENCH_EXTRA_JSON 透传给 chat/completions 的额外 JSON（如 Qwen-MT 的
                             {"translation_options": {...}}），默认 {}

用法：
    python scripts/spikes/s05_s06/bench_translate.py --runs 20 --warmup 2
    python scripts/spikes/s05_s06/bench_translate.py --self-test   # 无 Key 时验证脚本框架
    python scripts/spikes/s05_s06/bench_translate.py --help

退出码：0 = 已完成真实测量（DONE/FAILED 见输出）；2 = BLOCKED（缺 Key/URL/模型）；
        3 = 脚本自身异常；4 = 全部请求失败。
注意：--self-test 用脚本内置 mock 服务验证框架，其数字是 MOCK，**不得作为 DONE 依据**。
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import http.client
import json
import os
import platform
import socket
import statistics
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone, timedelta
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple
from urllib.parse import urlsplit

SCRIPT_VERSION = "1.0.0"
THRESHOLD_MS = 1500.0  # 云端 200 字 P95 阈值
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
DEFAULT_OUT_DIR = REPO_ROOT / ".verify" / "spikes" / "S-05-S-06" / "raw"
TRANSLATE_PROMPT = REPO_ROOT / "resources" / "prompts" / "translate.md"
BODY_MARKER = "以下为发送给模型的正文"

# 200 字中文样例（产品文案风格；共 200 个字符，含 160 个汉字 + 标点/空格/ASCII 标识符）。
# 与 bench_local_translate.mjs 中的 SAMPLE_TEXT 必须逐字一致（两边都断言长度 = 200）。
SAMPLE_TEXT = (
    "框选 AI 是一款屏幕取词与截图问答工具：按下 Ctrl+Q 唤起选框，框住屏幕上的文字或图像，"
    "松开鼠标后弹出动作菜单。你可以翻译选中段落、提取文字并搜索、识别图片内容，或向 AI 提问。"
    "所有动作都在结果窗口中展示，支持复制、重新执行与追问。云端模型需要配置 API Key，"
    "本地模型下载完成后即可离线使用。首次使用建议先完成引导，选择常用语言与默认动作。"
    "若遇到快捷键冲突，可在设置中修改或暂停热键。"
)
if len(SAMPLE_TEXT) != 200:  # 防手改漂移
    raise SystemExit(f"SAMPLE_TEXT 必须为 200 字符，当前 {len(SAMPLE_TEXT)}")

EXIT_OK, EXIT_FAILED, EXIT_BLOCKED, EXIT_ERROR = 0, 1, 2, 3


# --------------------------------------------------------------------------- #
# 提示词渲染：与 src-tauri/src/services/prompt_engine.rs 的 render_template 对齐，
# 并按 translate.md 自身说明剔除 "#" 开头维护注释（取「正文」标记之后的内容）。
# --------------------------------------------------------------------------- #
def render_translate_prompt(
    text: str, target_lang: str, source_lang: str, glossary: str = ""
) -> str:
    raw = TRANSLATE_PROMPT.read_text(encoding="utf-8")
    marker_at = raw.find(BODY_MARKER)
    body = raw[marker_at:] if marker_at >= 0 else raw
    body = body.split("\n", 1)[1] if "\n" in body else body  # 去掉标记所在行
    rendered = (
        body.replace("{text}", text)
        .replace("{targetLang}", target_lang)
        .replace("{sourceLang}", source_lang)
        .replace("{glossary}", glossary)
    )
    for var in ("text", "targetLang", "sourceLang", "glossary"):
        if "{" + var + "}" in rendered:
            raise SystemExit(f"提示词渲染失败：仍有未替换变量 {{{var}}}")
    return rendered.strip("\n")


# --------------------------------------------------------------------------- #
# HTTP 客户端（支持 keep-alive 连接复用；记录建连/首字节/总耗时）
# --------------------------------------------------------------------------- #
class HttpClient:
    def __init__(self, base_url: str, timeout: float, keep_alive: bool = True) -> None:
        parts = urlsplit(base_url)
        if parts.scheme not in ("http", "https"):
            raise ValueError(f"base_url 必须为 http(s)://，实际：{base_url}")
        self.scheme = parts.scheme
        self.host = parts.hostname or "127.0.0.1"
        self.port = parts.port or (443 if parts.scheme == "https" else 80)
        path = parts.path.rstrip("/")
        self.endpoint = path if path.endswith("/chat/completions") else path + "/chat/completions"
        self.timeout = timeout
        self.keep_alive = keep_alive
        self.conn: Optional[http.client.HTTPConnection] = None
        self.new_connection = True
        self.masked_url = f"{self.scheme}://{self.host}:{self.port}{self.endpoint}"

    def _connect(self) -> Tuple[http.client.HTTPConnection, float]:
        t0 = time.perf_counter()
        cls = (
            http.client.HTTPSConnection
            if self.scheme == "https"
            else http.client.HTTPConnection
        )
        conn = cls(self.host, self.port, timeout=self.timeout)
        conn.connect()
        return conn, (time.perf_counter() - t0) * 1000.0

    def close(self) -> None:
        if self.conn is not None:
            try:
                self.conn.close()
            finally:
                self.conn = None

    def post_json(
        self, payload: Dict[str, Any], api_key: str, stream: bool = False
    ) -> Dict[str, Any]:
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        headers = {
            "Content-Type": "application/json",
            "Accept": "text/event-stream" if stream else "application/json",
            "Authorization": f"Bearer {api_key}",
            "User-Agent": f"snipask-bench-translate/{SCRIPT_VERSION}",
        }
        reconnects = 0
        last_err: Optional[Exception] = None
        for attempt in range(2):
            conn = self.conn
            reused = conn is not None and not self.new_connection
            connect_ms = 0.0
            if conn is None or self.new_connection:
                conn, connect_ms = self._connect()
                self.conn = conn
                self.new_connection = False
            t_req = time.perf_counter()
            try:
                conn.request("POST", self.endpoint, body=body, headers=headers)
                t_sent = time.perf_counter()
                resp = conn.getresponse()
                t_first = time.perf_counter()
                if stream:
                    text, usage, first_token_ms = self._read_sse(resp, t_req)
                    raw = text
                else:
                    raw_bytes = resp.read()
                    t_done = time.perf_counter()
                    raw = raw_bytes.decode("utf-8", "replace")
                if resp.will_close:
                    self.new_connection = True
                return {
                    "status": resp.status,
                    "raw": raw,
                    "stream": stream,
                    "reused_connection": reused,
                    "connect_ms": connect_ms,
                    "send_ms": (t_sent - t_req) * 1000.0,
                    "first_byte_ms": (t_first - t_req) * 1000.0,
                    "total_ms": (
                        first_token_ms if stream else (t_done - t_req) * 1000.0
                    ),
                    "first_token_ms": first_token_ms if stream else None,
                    "reconnect_count": reconnects,
                }
            except (
                ConnectionError,
                http.client.HTTPException,
                socket.timeout,
                TimeoutError,
            ) as err:  # 连接可能已被服务端关闭：强制重连一次
                last_err = err
                reconnects += 1
                self.new_connection = True
                self.close()
                if attempt == 0:
                    continue
                raise
        raise RuntimeError(f"请求失败：{last_err}")

    @staticmethod
    def _read_sse(
        resp: http.client.HTTPResponse, t_req: float
    ) -> Tuple[str, Optional[Dict[str, Any]], float]:
        """读取 SSE 流，返回完整文本、usage 与首 token 延时（ms）。"""
        chunks: List[str] = []
        usage: Optional[Dict[str, Any]] = None
        first_token_ms: Optional[float] = None
        while True:
            line = resp.readline()
            if not line:
                break
            line_s = line.decode("utf-8", "replace").strip()
            if not line_s.startswith("data:"):
                continue
            data = line_s[5:].strip()
            if data == "[DONE]":
                break
            try:
                obj = json.loads(data)
            except json.JSONDecodeError:
                continue
            if obj.get("usage"):
                usage = obj["usage"]
            for choice in obj.get("choices") or []:
                delta = choice.get("delta") or {}
                content = delta.get("content")
                if content:
                    if first_token_ms is None:
                        first_token_ms = (time.perf_counter() - t_req) * 1000.0
                    chunks.append(content)
        if first_token_ms is None:
            first_token_ms = (time.perf_counter() - t_req) * 1000.0
        return "".join(chunks), usage, first_token_ms


# --------------------------------------------------------------------------- #
# 内置 mock OpenAI 服务：仅用于 --self-test 验证基准框架可跑通
# --------------------------------------------------------------------------- #
class _MockHandler(BaseHTTPRequestHandler):
    mock_latency_ms = 70.0

    def log_message(self, fmt: str, *args: Any) -> None:  # 静默
        pass

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        payload = json.loads(self.rfile.read(length) or b"{}")
        time.sleep(self.mock_latency_ms / 1000.0)
        text = "MOCK TRANSLATION OUTPUT (NOT A REAL MODEL). " + SAMPLE_TEXT[:12]
        body = {
            "id": "mock-1",
            "object": "chat.completion",
            "model": payload.get("model", "mock"),
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": text},
                    "finish_reason": "stop",
                }
            ],
            "usage": {
                "prompt_tokens": 300,
                "completion_tokens": 80,
                "total_tokens": 380,
            },
        }
        data = json.dumps(body).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def start_mock_server() -> Tuple[ThreadingHTTPServer, str]:
    server = ThreadingHTTPServer(("127.0.0.1", 0), _MockHandler)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    host, port = server.server_address
    return server, f"http://{host}:{port}/v1"


# --------------------------------------------------------------------------- #
# 统计工具
# --------------------------------------------------------------------------- #
def pct(values: List[float], p: float) -> float:
    if not values:
        return float("nan")
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    k = (len(ordered) - 1) * (p / 100.0)
    lo, hi = int(k), min(int(k) + 1, len(ordered) - 1)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (k - lo)


def summarize(values: List[float]) -> Dict[str, float]:
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


# --------------------------------------------------------------------------- #
# 主流程
# --------------------------------------------------------------------------- #
def collect_meta(mock: bool) -> Dict[str, Any]:
    commit = "unknown"
    try:
        commit = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=str(REPO_ROOT),
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip() or "unknown"
    except Exception:  # noqa: BLE001 - 取不到不影响基准
        pass
    return {
        "script": "bench_translate.py",
        "script_version": SCRIPT_VERSION,
        "generated_at": datetime.now(timezone(timedelta(hours=8))).isoformat(),
        "mode": "MOCK_SELF_TEST" if mock else "REAL",
        "host": {
            "os": platform.platform(),
            "machine": platform.machine(),
            "cpu_count": os.cpu_count(),
            "python": platform.python_version(),
            "hostname": socket.gethostname(),
        },
        "repo_commit": commit,
        "prompt_file": str(TRANSLATE_PROMPT.relative_to(REPO_ROOT)),
        "prompt_sha256": hashlib.sha256(TRANSLATE_PROMPT.read_bytes()).hexdigest(),
        "sample_len_chars": len(SAMPLE_TEXT),
        "threshold_ms": THRESHOLD_MS,
        "sample_text_sha256": hashlib.sha256(SAMPLE_TEXT.encode("utf-8")).hexdigest(),
    }


def run_benchmark(
    base_url: str,
    model: str,
    api_key: str,
    runs: int,
    warmup: int,
    timeout: float,
    stream: bool,
    target_lang: str,
    source_lang: str,
    extra_json: str,
    mock: bool,
) -> Tuple[Dict[str, Any], int]:
    prompt = render_translate_prompt(SAMPLE_TEXT, target_lang, source_lang, "")
    try:
        extra = json.loads(extra_json) if extra_json.strip() else {}
    except json.JSONDecodeError as err:
        raise SystemExit(f"--extra-json 不是合法 JSON：{err}")
    payload: Dict[str, Any] = {
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.0,
        "stream": stream,
    }
    payload.update(extra)

    client = HttpClient(base_url, timeout=timeout, keep_alive=True)
    records: List[Dict[str, Any]] = []
    failures = 0
    first_failure: Optional[Dict[str, Any]] = None

    def timed_call(label: str) -> Dict[str, Any]:
        nonlocal first_failure
        try:
            res = client.post_json(payload, api_key, stream=stream)
        except Exception as err:  # noqa: BLE001 - 失败要计数，不中断基准
            return {
                "run": label,
                "ok": False,
                "error": f"{type(err).__name__}: {err}",
                "total_ms": None,
                "first_byte_ms": None,
                "status": None,
            }
        if res["status"] == 200:
            try:
                obj = json.loads(res["raw"])
            except json.JSONDecodeError:
                obj = None
            content = ""
            usage = None
            if obj is not None:
                content = (
                    ((obj.get("choices") or [{}])[0].get("message") or {}).get("content")
                    or ""
                )
                usage = obj.get("usage")
            return {
                "run": label,
                "ok": True,
                "status": res["status"],
                "reused_connection": res["reused_connection"],
                "connect_ms": round(res["connect_ms"], 2),
                "send_ms": round(res["send_ms"], 2),
                "first_byte_ms": round(res["first_byte_ms"], 2),
                "total_ms": round(res["total_ms"], 2),
                "first_token_ms": (
                    round(res["first_token_ms"], 2)
                    if res.get("first_token_ms") is not None
                    else None
                ),
                "usage": usage,
                "output_chars": len(content),
                "output_sample": content[:120],
                "output_sha256": hashlib.sha256(content.encode("utf-8")).hexdigest(),
            }
        else:
            err_body = res["raw"][:400]
            try:
                err_json = json.loads(res["raw"])
            except json.JSONDecodeError:
                err_json = None
            return {
                "run": label,
                "ok": False,
                "status": res["status"],
                "error": str(err_json) if err_json else err_body,
                "total_ms": round(res["total_ms"], 2),
                "first_byte_ms": round(res["first_byte_ms"], 2),
            }

    for i in range(warmup):
        rec = timed_call(f"warmup-{i + 1}")
        rec["warmup"] = True
        records.append(rec)
    for i in range(runs):
        rec = timed_call(f"run-{i + 1}")
        rec["warmup"] = False
        records.append(rec)
        if not rec["ok"]:
            failures += 1
            if first_failure is None:
                first_failure = rec

    client.close()
    measured = [r for r in records if not r.get("warmup") and r.get("ok")]
    total_ms = [r["total_ms"] for r in measured]
    fb_ms = [r["first_byte_ms"] for r in measured]
    statuses: Dict[str, int] = {}
    for r in records:
        key = str(r.get("status"))
        statuses[key] = statuses.get(key, 0) + 1
    usage_total = sum(
        (r.get("usage") or {}).get("total_tokens", 0) for r in measured
    )

    result: Dict[str, Any] = {
        "meta": collect_meta(mock),
        "config": {
            "base_url_masked": client.masked_url,
            "model": model,
            "stream": stream,
            "runs": runs,
            "warmup": warmup,
            "timeout_sec": timeout,
            "target_lang": target_lang,
            "source_lang": source_lang,
            "temperature": 0.0,
            "prompt_chars": len(prompt),
            "extra_params": sorted(extra.keys()),
            "api_key_provided": bool(api_key),
        },
        "records": records,
        "summary": {
            "measured_runs": len(measured),
            "failures": failures,
            "failure_rate": round(failures / runs, 4) if runs else None,
            "http_status_distribution": statuses,
            "total_ms": summarize(total_ms),
            "first_byte_ms": summarize(fb_ms),
            "usage_total_tokens_all_runs": usage_total,
            "first_failure": first_failure,
        },
    }
    if mock:
        result["labels"] = [
            "MOCK_SELF_TEST：以上数字来自脚本内置 mock 服务，仅证明基准框架可跑通，不得作为 S-05 DONE/FAILED 依据。"
        ]
    return result, EXIT_OK


def print_report(result: Dict[str, Any]) -> None:
    meta = result["meta"]
    cfg = result["config"]
    s = result["summary"]
    print("=" * 74)
    print(f"S-05 云端翻译链路时延基准  [{meta['mode']}]")
    print(f"时间：{meta['generated_at']}")
    print(f"端点：{cfg['base_url_masked']}  模型：{cfg['model']}  stream={cfg['stream']}")
    print(f"样例：{meta['sample_len_chars']} 字（prompt {cfg['prompt_chars']} 字符）")
    print(f"运行：warmup={cfg['warmup']} + measured={cfg['runs']}  阈值 P95 ≤ {meta['threshold_ms']:.0f}ms")
    print("-" * 74)
    for label, key in (("总耗时", "total_ms"), ("首字节", "first_byte_ms")):
        st = s.get(key) or {}
        if st.get("count"):
            print(
                f"{label}：P50 {st['p50_ms']:.1f}ms | P95 {st['p95_ms']:.1f}ms | "
                f"max {st['max_ms']:.1f}ms | mean {st['mean_ms']:.1f}ms (n={st['count']})"
            )
    print(f"HTTP 状态分布：{s['http_status_distribution']}  失败率：{s['failure_rate']}")
    if s.get("first_failure"):
        print(f"首个失败：{json.dumps(s['first_failure'], ensure_ascii=False)[:300]}")
    verdict = "UNKNOWN"
    if s["measured_runs"] > 0:
        p95 = (s["total_ms"] or {}).get("p95_ms")
        if p95 is not None:
            if meta["mode"] == "MOCK_SELF_TEST":
                # mock 自检永远不给出 DONE/FAILED 验收结论
                verdict = "MOCK_SELF_CHECK_OK"
                print(f"判定：{verdict}（mock 数据 P95 {p95:.1f}ms，仅验证框架，不得作为 DONE 依据）")
            else:
                verdict = "DONE" if p95 <= meta["threshold_ms"] else "FAILED"
                print(f"判定：{verdict}（P95 {p95:.1f}ms vs 阈值 {meta['threshold_ms']:.0f}ms）")
    print("=" * 74)


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(
        description="S-05 云端翻译链路时延基准（OpenAI 兼容 chat/completions）",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    ap.add_argument("--base-url", default=os.environ.get("SNIPASK_BENCH_BASE_URL", ""))
    ap.add_argument("--model", default=os.environ.get("SNIPASK_BENCH_MODEL", ""))
    ap.add_argument("--api-key", default=os.environ.get("SNIPASK_BENCH_API_KEY", ""))
    ap.add_argument("--runs", type=int, default=20)
    ap.add_argument("--warmup", type=int, default=2)
    ap.add_argument("--timeout", type=float, default=60.0)
    ap.add_argument("--stream", action="store_true", help="流式请求，额外记录首 token 延时")
    ap.add_argument("--target-lang", default="en")
    ap.add_argument("--source-lang", default="zh-CHS")
    ap.add_argument(
        "--extra-json",
        default=os.environ.get("SNIPASK_BENCH_EXTRA_JSON", ""),
        help="透传给 chat/completions 的额外 JSON（如 Qwen-MT 的 translation_options）",
    )
    ap.add_argument(
        "--out",
        default="",
        help=f"结果 JSON 路径（默认 {DEFAULT_OUT_DIR}/cloud_translate_<时间戳>.json）",
    )
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="用内置 mock 服务自检基准框架（MOCK 数据，不得作为 DONE 依据）",
    )
    args = ap.parse_args(argv)

    mock = bool(args.self_test)
    mock_server: Optional[ThreadingHTTPServer] = None
    base_url, model, api_key = args.base_url, args.model, args.api_key

    if mock:
        mock_server, base_url = start_mock_server()
        model = model or "mock-translate"
        api_key = api_key or "mock-key"
        print("[SELF-TEST] 已启动内置 mock OpenAI 服务：仅验证脚本框架，数据为 MOCK。")
    else:
        missing = [
            name
            for name, val in (
                ("SNIPASK_BENCH_BASE_URL", base_url),
                ("SNIPASK_BENCH_MODEL", model),
                ("SNIPASK_BENCH_API_KEY", api_key),
            )
            if not val
        ]
        if missing:
            print("BLOCKED：未提供 SNIPASK_BENCH_* 环境变量，无法做真实云端翻译时延测量。")
            print(f"缺失：{', '.join(missing)}")
            print("")
            print("解锁步骤（拿到 Key 后 1 条命令复跑）：")
            print('  $env:SNIPASK_BENCH_BASE_URL = "https://api.deepseek.com/v1"   # 或 DashScope 兼容模式')
            print('  $env:SNIPASK_BENCH_MODEL    = "deepseek-chat"                 # 或 qwen-mt-turbo')
            print('  $env:SNIPASK_BENCH_API_KEY  = "<你的 Key>"')
            print("  python scripts/spikes/s05_s06/bench_translate.py --runs 20")
            print("")
            print("当前机器无任何云端 Key（DeepSeek 空 Key 实测 HTTP 401），故 S-05 云端判定 BLOCKED。")
            return EXIT_BLOCKED

    try:
        result, code = run_benchmark(
            base_url=base_url,
            model=model,
            api_key=api_key,
            runs=args.runs,
            warmup=args.warmup,
            timeout=args.timeout,
            stream=args.stream,
            target_lang=args.target_lang,
            source_lang=args.source_lang,
            extra_json=args.extra_json,
            mock=mock,
        )
    except SystemExit:
        raise
    except Exception as err:  # noqa: BLE001
        print(f"基准执行异常：{type(err).__name__}: {err}")
        return EXIT_ERROR
    finally:
        if mock_server is not None:
            mock_server.shutdown()

    print_report(result)
    out = Path(args.out) if args.out else (
        DEFAULT_OUT_DIR / f"cloud_translate_{datetime.now().strftime('%Y%m%d-%H%M%S')}.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"原始数据：{out}")

    measured = result["summary"]["measured_runs"]
    if measured == 0:
        print("所有请求均失败，判定 BLOCKED/FAILED（见 first_failure）。")
        return EXIT_FAILED
    if result["summary"]["failure_rate"] and result["summary"]["failure_rate"] > 0.2:
        print(f"失败率 {result['summary']['failure_rate']} 过高，判定 FAILED。")
        return EXIT_FAILED
    return EXIT_OK if not mock else EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
