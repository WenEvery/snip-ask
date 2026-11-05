#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S-14 翻译一致性基准（术语命中 + 格式保留 + 后端标注）。

被测对象（真实链路，非模拟）：
    云端：DeepSeek OpenAI 兼容 `POST {base}/v1/chat/completions`，模型 deepseek-flash，
          显式 `{"thinking":{"type":"disabled"}}`（S-05 实测结论：不关闭思考会推高首字）。
          提示词用产品 `resources/prompts/translate.md`（产品契约：云端/1.8B 走长提示词）。
    本地：llama.cpp `llama-server` + GGUF（默认 Qwen2.5-0.5B-Instruct Q4_K_M，可选 1.5B），
          提示词用 `resources/prompts/translate_minimal.md`（S-05 实测：0.5B 用长提示词 10/10 复述原文）。
          另提供 0.5B + 产品长提示词的对照臂（复现 S-05 失败模式）与「minimal + 格式守卫」实验臂。

判分（样例见 s14_cases.py）：
    术语命中率 = 命中 required_terms / 总数（目标 100%）
    格式保留率 = 通过格式检查项 / 总数，按换行/空行/列表/编号/代码围栏/URL/邮箱/占位符/标识符逐项统计（目标 100%）
    译文有效性 = 中→英 CJK < 30%；英→中 CJK > 15% 且不等于原文（识别「未翻译/复述」）
    后端标注：每条结果记录 backend_label（脚本侧 100% 标注）；产品侧契约见文档 §4。

Key 纪律：只从 `.verify/p1/key.txt`（优先）或环境变量 `SNIPASK_BENCH_API_KEY` 读取，
**不打印、不落盘、不写入任何输出 JSON**；输出只记录 `api_key_provided` 与脱敏 host。

用法：
    # 本地 0.5B + 极简提示词（无 Key 也可跑）
    python scripts/spikes/s12_s14/bench_translate_consistency.py --arms local-tiny-minimal
    # 本地对照/实验臂
    python scripts/spikes/s12_s14/bench_translate_consistency.py --arms local-tiny-product
    python scripts/spikes/s12_s14/bench_translate_consistency.py --arms local-tiny-minimal-format
    python scripts/spikes/s12_s14/bench_translate_consistency.py --arms local-15b-minimal
    # 云端（需 Key；无 Key 时该臂记为 BLOCKED，本地臂继续）
    python scripts/spikes/s12_s14/bench_translate_consistency.py --arms cloud
    # 无 Key 的框架自检（MOCK，不作验收）
    python scripts/spikes/s12_s14/bench_translate_consistency.py --mock-self-check

退出码：0 = 至少一个臂完成真实测量（阈值是否达标见输出/meta）；2 = 全部 BLOCKED；3 = 脚本异常。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import socket
import subprocess
import sys
import threading
import time
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import requests

SCRIPT_VERSION = "1.0.0"
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
RAW_DIR = REPO_ROOT / ".verify" / "spikes" / "S-12-S-14" / "raw"
KEY_FILE = REPO_ROOT / ".verify" / "p1" / "key.txt"
PROMPT_DIR = REPO_ROOT / "resources" / "prompts"
ARTIFACT_DIR = REPO_ROOT / ".verify" / "spikes" / "S-05-S-06" / "artifacts"

sys.path.insert(0, str(SCRIPT_DIR))
from s14_cases import (  # noqa: E402
    CASES,
    cjk_ratio,
    evaluate_check,
    render_glossary,
    strip_maintenance_header,
    translation_effective,
)

EXIT_OK, EXIT_BLOCKED, EXIT_ERROR = 0, 2, 3
TZ = timezone(timedelta(hours=8))

MODEL_FILES = {
    "tiny": "qwen2.5-0.5b-instruct-q4_k_m.gguf",
    "tiny15b": "qwen2.5-1.5b-instruct-q4_k_m.gguf",
    "hymt": "HY-MT1.5-1.8B-Q4_K_M.gguf",
}

# 臂（backend × 提示词 × 模型）：label 会写入每条结果的 backend_label
ARMS: Dict[str, Dict[str, Any]] = {
    "cloud": {
        "kind": "cloud",
        "variant": "product",
        "prompt_key": "translate",
        "model": "deepseek-flash",
        "temperature": 0.0,
        "label": "cloud:deepseek-flash/translate.md",
        "desc": "云端 DeepSeek deepseek-flash（thinking disabled）+ translate.md",
    },
    "local-tiny-minimal": {
        "kind": "local",
        "variant": "minimal",
        "prompt_key": "translate_minimal",
        "model": "tiny",
        "temperature": 0.2,
        "label": "local:qwen2.5-0.5b-instruct-q4/translate_minimal.md",
        "desc": "本地 Qwen2.5-0.5B Q4 + 精简提示词（S-05 推荐极小档；按产品契约渲染 BCP-47 语言码）",
    },
    "local-tiny-minimal-names": {
        "kind": "local",
        "variant": "minimal_names",
        "prompt_key": "translate_minimal(lang-name)",
        "model": "tiny",
        "temperature": 0.2,
        "label": "local:qwen2.5-0.5b-instruct-q4/translate_minimal+lang-names",
        "desc": "诊断/修复验证臂：同一 translate_minimal.md，把 zh-CHS/en 渲染为「中文/英文」",
    },
    "local-tiny-product": {
        "kind": "local",
        "variant": "product",
        "prompt_key": "translate",
        "model": "tiny",
        "temperature": 0.2,
        "label": "local:qwen2.5-0.5b-instruct-q4/translate.md",
        "desc": "本地 0.5B + 产品长提示词（对照：复现 S-05「复述原文」失败模式）",
    },
    "local-tiny-minimal-format": {
        "kind": "local",
        "variant": "minimal_format",
        "prompt_key": "translate_minimal+format_guard",
        "model": "tiny",
        "temperature": 0.2,
        "label": "local:qwen2.5-0.5b-instruct-q4/translate_minimal+format",
        "desc": "本地 0.5B + 精简提示词 + 格式守卫句（实验臂，验证能否补回格式保留）",
    },
    "local-15b-minimal": {
        "kind": "local",
        "variant": "minimal",
        "prompt_key": "translate_minimal",
        "model": "tiny15b",
        "temperature": 0.2,
        "label": "local:qwen2.5-1.5b-instruct-q4/translate_minimal.md",
        "desc": "本地 Qwen2.5-1.5B Q4 + 精简提示词（质量优先档候选）",
    },
    "local-tiny-minimal-v2": {
        "kind": "local",
        "variant": "minimal_v2",
        "prompt_key": "translate_minimal_v2(candidate)",
        "model": "tiny",
        "temperature": 0.2,
        "label": "local:qwen2.5-0.5b-instruct-q4/translate_minimal_v2",
        "desc": "处置复核臂：候选修复版精简提示词（语言名称 + 术语并入指令 + <<< >>> 分隔 + 前置格式约束）",
    },
    "local-15b-minimal-v2": {
        "kind": "local",
        "variant": "minimal_v2",
        "prompt_key": "translate_minimal_v2(candidate)",
        "model": "tiny15b",
        "temperature": 0.2,
        "label": "local:qwen2.5-1.5b-instruct-q4/translate_minimal_v2",
        "desc": "处置复核臂：1.5B + 候选修复版精简提示词（验证格式保留能否到 100%）",
    },
    "local-tiny-minimal-v3": {
        "kind": "local",
        "variant": "minimal_v3",
        "prompt_key": "translate_minimal_v3(candidate)",
        "model": "tiny",
        "temperature": 0.2,
        "label": "local:qwen2.5-0.5b-instruct-q4/translate_minimal_v3",
        "desc": "处置复核臂 v3：仅修「术语表标题行被复述」结构缺陷（术语并入指令行、保留语言码与原文分段）",
    },
    "local-15b-minimal-v3": {
        "kind": "local",
        "variant": "minimal_v3",
        "prompt_key": "translate_minimal_v3(candidate)",
        "model": "tiny15b",
        "temperature": 0.2,
        "label": "local:qwen2.5-1.5b-instruct-q4/translate_minimal_v3",
        "desc": "处置复核臂 v3：1.5B + 仅修结构缺陷的精简提示词",
    },
}

MOCK_ARMS = {
    "mock-pass": {
        "kind": "mock",
        "variant": "product",
        "prompt_key": "translate",
        "model": "mock-model",
        "temperature": 0.0,
        "label": "MOCK:mock-model/translate.md",
        "satisfy": True,
    },
    "mock-fail": {
        "kind": "mock",
        "variant": "product",
        "prompt_key": "translate",
        "model": "mock-model",
        "temperature": 0.0,
        "label": "MOCK:mock-model/translate.md（故意失败臂）",
        "satisfy": False,
    },
}


# --------------------------------------------------------------------------- #
# 基础工具
# --------------------------------------------------------------------------- #
def now_iso() -> str:
    return datetime.now(TZ).isoformat()


def stamp() -> str:
    return datetime.now().strftime("%Y%m%d-%H%M%S")


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def sha256_file(path: Path) -> Optional[str]:
    try:
        h = hashlib.sha256()
        with path.open("rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return None


def load_api_key(cli_key: str) -> Tuple[str, str]:
    """返回 (key, source)。只读文件/环境变量；绝不打印 key。"""
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


def prompt_path(prompt_key: str) -> Path:
    return PROMPT_DIR / f"{prompt_key}.md"


def render_prompt(
    prompt_key: str,
    case: Dict[str, Any],
    append: str = "",
    source_lang: Optional[str] = None,
    target_lang: Optional[str] = None,
) -> str:
    """按 prompt_engine.rs 语义渲染：先剔除维护注释头，再替换变量。

    source_lang/target_lang 仅用于诊断臂（把 BCP-47 码换成语言名称），默认用样例语言码。
    """
    raw = prompt_path(prompt_key).read_text(encoding="utf-8")
    body = strip_maintenance_header(raw)
    rendered = (
        body.replace("{text}", case["text"])
        .replace("{targetLang}", target_lang or case["target_lang"])
        .replace("{sourceLang}", source_lang or case["source_lang"])
        .replace("{glossary}", render_glossary(case["glossary"]))
    )
    for var in ("text", "targetLang", "sourceLang", "glossary"):
        if "{" + var + "}" in rendered:
            raise RuntimeError(f"提示词渲染失败：仍有未替换变量 {{{var}}}")
    return rendered + append


FORMAT_GUARD = "\n保留原文的换行、空行、列表符号、编号、代码块、URL、邮箱与 {变量} 占位符。"

# 诊断臂用：语言码 → 人类可读名称（不改产品文件，只在基准脚本里替换渲染变量）
LANG_NAMES = {
    "zh-chs": "中文",
    "zh-cht": "繁体中文",
    "zh": "中文",
    "en": "英文",
    "ja": "日文",
    "ko": "韩文",
}


def build_v2_prompt(case: Dict[str, Any]) -> str:
    """候选修复版精简提示词（处置复核臂，不修改产品资源文件）。

    针对 S-14 实测的两个缺陷：
      1) 语言变量用 BCP-47 码（zh-CHS/en）时 0.5B 不翻译 → 改用语言名称；
      2) 术语表行在正文之前会被模型复述进输出（1.5B 14/24 例）→ 术语并入指令行，正文用 <<< >>> 分隔；
      3) 格式约束放在正文之前（追加到正文之后会被当成待翻译内容）。
    """
    src = LANG_NAMES.get(case["source_lang"].lower(), case["source_lang"])
    tgt = LANG_NAMES.get(case["target_lang"].lower(), case["target_lang"])
    glossary = render_glossary(case["glossary"]) or "（空）"
    return (
        f"将 <<< >>> 之间的文本从 {src} 翻译成 {tgt}，只输出译文，不要解释、不要复述原文、不要输出原文。\n"
        f"术语表（命中时按指定译法；可为空）：{glossary}\n"
        "要求：严格保留原文的换行、空行、列表符号、编号、代码块、URL、邮箱与大括号变量占位符；"
        "不要翻译代码与 URL。\n"
        "<<<\n"
        f"{case['text']}\n"
        ">>>"
    )


def build_v3_prompt(case: Dict[str, Any]) -> str:
    """候选修复版 v3：只修 translate_minimal.md 的结构缺陷（术语表标题行被模型复述）。

    保持与产品一致：语言变量仍用 BCP-47 码（1.5B 实测可正确理解）；把术语表并入第一条指令句，
    不再单独占一行「术语表（可为空…）：」，正文前保留一个空行分段。不追加任何格式守卫句/分隔符。
    """
    glossary = render_glossary(case["glossary"]) or "（空）"
    return (
        f"将下面的文本从 {case['source_lang']} 翻译成 {case['target_lang']}，只输出译文，"
        f"不要解释、不要复述原文、不要输出原文。严格保留原文的换行、列表符号、编号、代码块、URL 与占位符。\n"
        f"术语表（命中时按指定译法）：{glossary}\n"
        f"\n{case['text']}"
    )


def build_prompt(variant: str, case: Dict[str, Any]) -> Tuple[str, str]:
    """返回 (rendered_prompt, prompt_key)。
    variant: product | minimal | minimal_names | minimal_format | minimal_v2 | minimal_v3
    """
    if variant == "product":
        return render_prompt("translate", case), "translate"
    if variant == "minimal_v2":
        return build_v2_prompt(case), "translate_minimal_v2(candidate)"
    if variant == "minimal_v3":
        return build_v3_prompt(case), "translate_minimal_v3(candidate)"
    if variant == "minimal_names":
        src = LANG_NAMES.get(case["source_lang"].lower(), case["source_lang"])
        tgt = LANG_NAMES.get(case["target_lang"].lower(), case["target_lang"])
        return (
            render_prompt("translate_minimal", case, source_lang=src, target_lang=tgt),
            "translate_minimal(lang-name)",
        )
    guard = FORMAT_GUARD if variant == "minimal_format" else ""
    rendered = render_prompt("translate_minimal", case, append=guard)
    return rendered, "translate_minimal" + ("+format_guard" if guard else "")


# --------------------------------------------------------------------------- #
# 云端（OpenAI 兼容 chat/completions）
# --------------------------------------------------------------------------- #
def chat_completions_url(base_url: str) -> str:
    base = base_url.rstrip("/")
    if base.endswith("/v1"):
        return base + "/chat/completions"
    return base + "/v1/chat/completions"


RETRY_STATUS = {408, 429, 500, 502, 503, 504}


def cloud_chat(
    session: requests.Session,
    base_url: str,
    api_key: str,
    payload: Dict[str, Any],
    timeout: float = 90.0,
    retries: int = 2,
) -> Dict[str, Any]:
    url = chat_completions_url(base_url)
    attempts = 0
    last: Dict[str, Any] = {}
    for attempt in range(retries + 1):
        attempts = attempt + 1
        t0 = time.perf_counter()
        try:
            resp = session.post(
                url,
                json=payload,
                timeout=timeout,
                headers={
                    "Authorization": f"Bearer {api_key}",
                    "Content-Type": "application/json",
                    "User-Agent": f"snipask-bench-s14/{SCRIPT_VERSION}",
                },
            )
            latency = round((time.perf_counter() - t0) * 1000.0, 2)
            try:
                body = resp.json()
            except ValueError:
                body = resp.text[:2000]
            content = ""
            usage = None
            if resp.status_code == 200 and isinstance(body, dict):
                content = (((body.get("choices") or [{}])[0]).get("message") or {}).get(
                    "content"
                ) or ""
                usage = body.get("usage")
            last = {
                "status": resp.status_code,
                "latency_ms": latency,
                "attempts": attempts,
                "content": content,
                "usage": usage,
                "error": None
                if resp.status_code == 200
                else str(body)[:400],
            }
            if resp.status_code == 200 or resp.status_code not in RETRY_STATUS:
                return last
        except requests.RequestException as err:
            last = {
                "status": None,
                "latency_ms": round((time.perf_counter() - t0) * 1000.0, 2),
                "attempts": attempts,
                "content": "",
                "usage": None,
                "error": f"{type(err).__name__}: {err}",
            }
        if attempt < retries:
            time.sleep(0.5 * (2**attempt))
    return last


# --------------------------------------------------------------------------- #
# 本地 llama-server
# --------------------------------------------------------------------------- #
def find_llama_executable() -> str:
    env = os.environ.get("LLAMA_SERVER_EXE", "")
    if env and Path(env).is_file():
        return env
    local = ARTIFACT_DIR / "llama-b12110-bin-win-cpu-x64" / "llama-server.exe"
    if local.is_file():
        return str(local)
    lm_root = Path(os.environ.get("USERPROFILE", "C:/")) / ".lmstudio" / "extensions" / "backends"
    for name in (
        "llama.cpp-win-x86_64-avx2-2.28.2",
        "llama.cpp-win-x86_64-nvidia-cuda12-avx2-2.67.0",
        "llama.cpp-win-x86_64-nvidia-cuda-avx2-2.28.2",
    ):
        candidate = lm_root / name / "llama-server.exe"
        if candidate.is_file():
            return str(candidate)
    return ""


def find_model(model_key: str) -> str:
    filename = MODEL_FILES.get(model_key, "")
    if not filename:
        return ""
    candidate = ARTIFACT_DIR / filename
    return str(candidate) if candidate.is_file() else ""


def llama_pids() -> List[int]:
    """列出当前 llama-server.exe 进程 PID（用于启动前基线 + 结束后残留检查）。"""
    try:
        out = subprocess.run(
            ["tasklist", "/FI", "IMAGENAME eq llama-server.exe", "/FO", "CSV", "/NH"],
            capture_output=True,
            text=True,
            timeout=20,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    pids: List[int] = []
    for line in out.splitlines():
        parts = [p.strip('"') for p in line.split('","')]
        if len(parts) >= 2 and parts[0].lower().startswith("llama-server"):
            try:
                pids.append(int(parts[1]))
            except ValueError:
                pass
    return pids


def pick_free_port(preferred: int) -> int:
    for port in range(preferred, preferred + 20):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            sock.settimeout(0.3)
            if sock.connect_ex(("127.0.0.1", port)) != 0:
                return port
    raise RuntimeError(f"{preferred}..{preferred + 20} 端口均被占用")


def kill_tree(pid: int) -> None:
    subprocess.run(
        ["taskkill", "/PID", str(pid), "/T", "/F"],
        capture_output=True,
        text=True,
        timeout=20,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )


class LocalServer:
    """llama-server 生命周期：启动 → /health 就绪 → 逐条请求 → taskkill /T /F + 残留检查。"""

    def __init__(self, exe: str, model: str, port: int, threads: int, log_path: Path):
        self.exe = exe
        self.model = model
        self.port = port
        self.threads = threads
        self.log_path = log_path
        self.proc: Optional[subprocess.Popen] = None
        self.pre_pids: List[int] = []
        self.health_ms: Optional[float] = None
        self.log_handle = None

    def start(self) -> None:
        ARTIFACT_DIR.mkdir(parents=True, exist_ok=True)
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        self.pre_pids = llama_pids()
        self.log_handle = self.log_path.open("a", encoding="utf-8", errors="replace")
        self.log_handle.write(
            f"\n===== S-14 llama-server start {now_iso()} =====\n"
            f"cmd: {self.exe} -m {self.model} -c 4096 -t {self.threads} "
            f"--port {self.port} --host 127.0.0.1 --no-webui\n"
            f"preexisting llama-server pids: {self.pre_pids}\n"
        )
        self.log_handle.flush()
        t0 = time.perf_counter()
        self.proc = subprocess.Popen(
            [
                self.exe,
                "-m",
                self.model,
                "-c",
                "4096",
                "-t",
                str(self.threads),
                "--port",
                str(self.port),
                "--host",
                "127.0.0.1",
                "--no-webui",
            ],
            cwd=str(Path(self.exe).parent),
            stdout=self.log_handle,
            stderr=subprocess.STDOUT,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        deadline = time.perf_counter() + 180.0
        last_err = ""
        while time.perf_counter() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError(
                    f"llama-server 提前退出（code={self.proc.returncode}），日志 {self.log_path}"
                )
            try:
                resp = requests.get(
                    f"http://127.0.0.1:{self.port}/health", timeout=3.0
                )
                if resp.status_code == 200:
                    self.health_ms = round((time.perf_counter() - t0) * 1000.0, 2)
                    self.log_handle.write(f"\n/health ready after {self.health_ms}ms\n")
                    self.log_handle.flush()
                    return
                last_err = f"HTTP {resp.status_code}"
            except requests.RequestException as err:
                last_err = type(err).__name__
            time.sleep(0.1)
        raise RuntimeError(f"等待 /health 超时（180s）：{last_err}")

    def chat(self, messages: List[Dict[str, str]], temperature: float, max_tokens: int = 2048) -> Dict[str, Any]:
        payload = {
            "model": Path(self.model).stem,
            "messages": messages,
            "temperature": temperature,
            "top_p": 0.95,
            "max_tokens": max_tokens,
            "stream": False,
            "cache_prompt": False,
        }
        # 本机 llama-server 在连续多臂/高负载下偶发连接被重置：单次重试（不改变提示词与采样）
        last_err: Dict[str, Any] = {}
        for attempt in range(2):
            t0 = time.perf_counter()
            try:
                resp = requests.post(
                    f"http://127.0.0.1:{self.port}/v1/chat/completions",
                    json=payload,
                    timeout=300,
                )
                latency = round((time.perf_counter() - t0) * 1000.0, 2)
                body = resp.json() if resp.headers.get("content-type", "").startswith("application/json") else {}
                if resp.status_code != 200:
                    return {
                        "status": resp.status_code,
                        "latency_ms": latency,
                        "content": "",
                        "usage": None,
                        "timings": None,
                        "error": f"HTTP {resp.status_code}: {str(body)[:300]}",
                    }
                content = (((body.get("choices") or [{}])[0]).get("message") or {}).get("content") or ""
                return {
                    "status": 200,
                    "latency_ms": latency,
                    "content": content,
                    "usage": body.get("usage"),
                    "timings": body.get("timings"),
                    "error": None,
                }
            except requests.RequestException as err:
                last_err = {
                    "status": None,
                    "latency_ms": round((time.perf_counter() - t0) * 1000.0, 2),
                    "content": "",
                    "usage": None,
                    "timings": None,
                    "error": f"{type(err).__name__}: {err}",
                    "attempts": attempt + 1,
                }
                if attempt == 0:
                    time.sleep(0.5)
        return last_err

    def stop(self) -> Dict[str, Any]:
        info: Dict[str, Any] = {"pid": self.proc.pid if self.proc else None}
        if self.proc is not None and self.proc.poll() is None:
            kill_tree(self.proc.pid)
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                pass
        time.sleep(0.3)
        post_pids = llama_pids()
        residue = [pid for pid in post_pids if pid not in self.pre_pids]
        for pid in residue:
            kill_tree(pid)
        time.sleep(0.3)
        after = llama_pids()
        info.update(
            {
                "preexisting_pids": self.pre_pids,
                "post_pids": post_pids,
                "residue_killed": residue,
                "remaining_after_cleanup": after,
                "clean": len(after) == len(self.pre_pids),
            }
        )
        if self.log_handle:
            self.log_handle.write(f"\nshutdown: {json.dumps(info, ensure_ascii=False)}\n")
            self.log_handle.close()
        return info


# --------------------------------------------------------------------------- #
# 判分
# --------------------------------------------------------------------------- #
def score_case(case: Dict[str, Any], output: str) -> Dict[str, Any]:
    checks = [evaluate_check(chk, case["text"], output) for chk in case["format_checks"]]
    term_results = []
    for term in case["required_terms"]:
        if term.isascii():
            hit = term.lower() in output.lower()
        else:
            hit = term in output
        term_results.append({"term": term, "hit": hit})
    term_hits = sum(1 for t in term_results if t["hit"])
    format_pass = sum(1 for c in checks if c["pass"])
    effective = translation_effective(case, output)
    return {
        "term_results": term_results,
        "term_hits": term_hits,
        "term_total": len(term_results),
        "all_terms_hit": term_hits == len(term_results),
        "format_checks": checks,
        "format_pass": format_pass,
        "format_total": len(checks),
        "all_format_pass": all(c["pass"] for c in checks),
        "translation_effective": effective,
    }


def summarize_arm(records: List[Dict[str, Any]]) -> Dict[str, Any]:
    scored = [r for r in records if r.get("ok")]
    term_hits = sum(r["score"]["term_hits"] for r in scored)
    term_total = sum(r["score"]["term_total"] for r in scored)
    format_pass = sum(r["score"]["format_pass"] for r in scored)
    format_total = sum(r["score"]["format_total"] for r in scored)
    per_kind: Dict[str, Dict[str, int]] = {}
    for r in scored:
        for chk in r["score"]["format_checks"]:
            slot = per_kind.setdefault(chk["kind"], {"pass": 0, "total": 0})
            slot["total"] += 1
            slot["pass"] += 1 if chk["pass"] else 0
    case_pass = sum(
        1
        for r in scored
        if r["score"]["all_terms_hit"]
        and r["score"]["all_format_pass"]
        and r["score"]["translation_effective"]["pass"]
    )
    translated = sum(1 for r in scored if r["score"]["translation_effective"]["pass"])
    failures = []
    for r in records:
        reasons = []
        if not r.get("ok"):
            reasons.append(f"请求失败：{r.get('error')}")
        else:
            s = r["score"]
            if not s["all_terms_hit"]:
                missing = [t["term"] for t in s["term_results"] if not t["hit"]]
                reasons.append(f"术语未命中：{missing}")
            if not s["all_format_pass"]:
                bad = [c["label"] for c in s["format_checks"] if not c["pass"]]
                reasons.append(f"格式未保留：{bad}")
            if not s["translation_effective"]["pass"]:
                reasons.append(f"译文无效：{s['translation_effective']['detail']}")
        if reasons:
            failures.append({"case_id": r["case_id"], "reasons": reasons})

    def rate(num: int, den: int) -> Optional[float]:
        return round(num / den, 4) if den else None

    return {
        "cases": len(records),
        "cases_ok": len(scored),
        "term_hit_rate": rate(term_hits, term_total),
        "term_hits": term_hits,
        "term_total": term_total,
        "format_pass_rate": rate(format_pass, format_total),
        "format_pass": format_pass,
        "format_total": format_total,
        "format_by_kind": per_kind,
        "translation_effective_rate": rate(translated, len(scored)),
        "case_pass_rate": rate(case_pass, len(scored)),
        "case_pass": case_pass,
        "failures": failures,
    }


def verdict_for(summary: Dict[str, Any], failed_requests: int) -> str:
    if failed_requests:
        return "FAILED（存在请求失败）"
    if summary["term_hit_rate"] == 1.0 and summary["format_pass_rate"] == 1.0:
        return "DONE"
    return "FAILED"


# --------------------------------------------------------------------------- #
# 臂执行
# --------------------------------------------------------------------------- #
def make_record(
    case: Dict[str, Any],
    arm: Dict[str, Any],
    call: Dict[str, Any],
    prompt: str,
    prompt_key: str,
) -> Dict[str, Any]:
    ok = call.get("status") == 200
    output = call.get("content") or ""
    score = score_case(case, output) if ok else None
    record = {
        "case_id": case["id"],
        "direction": case["direction"],
        "category": case["category"],
        "backend": arm["kind"],
        "backend_label": arm["label"],
        "prompt_key": prompt_key,
        "model": arm["model"],
        "ok": ok,
        "status": call.get("status"),
        "latency_ms": call.get("latency_ms"),
        "usage": call.get("usage"),
        "error": call.get("error"),
        "source_text": case["text"],
        "source_sha256": sha256_text(case["text"]),
        "glossary_rendered": render_glossary(case["glossary"]),
        "required_terms": case["required_terms"],
        "prompt_sha256": sha256_text(prompt),
        "prompt_chars": len(prompt),
        "output": output,
        "output_sha256": sha256_text(output),
        "cjk_ratio": round(cjk_ratio(output), 4),
        "score": score,
    }
    return record


def run_cloud_arm(arm: Dict[str, Any], base_url: str, api_key: str) -> Dict[str, Any]:
    session = requests.Session()
    records: List[Dict[str, Any]] = []
    try:
        for case in CASES:
            prompt, prompt_key = build_prompt(arm["variant"], case)
            payload: Dict[str, Any] = {
                "model": arm["model"],
                "messages": [{"role": "user", "content": prompt}],
                "temperature": arm["temperature"],
                "max_tokens": 2048,
                "thinking": {"type": "disabled"},
            }
            call = cloud_chat(session, base_url, api_key, payload, timeout=90.0)
            records.append(make_record(case, arm, call, prompt, prompt_key))
    finally:
        session.close()
    summary = summarize_arm(records)
    failed_requests = sum(1 for r in records if not r["ok"])
    return {
        "arm": arm["label"],
        "kind": "cloud",
        "status": "REAL",
        "meta": {
            "base_url_masked": mask_base_url(base_url),
            "endpoint": chat_completions_url(base_url),
            "model": arm["model"],
            "api_key_provided": True,
            "temperature": arm["temperature"],
            "thinking": "disabled",
            "prompt_key": arm["prompt_key"],
        },
        "records": records,
        "summary": summary,
        "verdict": verdict_for(summary, failed_requests),
    }


def run_local_arm(
    arm: Dict[str, Any],
    exe: str,
    model_path: str,
    port: int,
    threads: int,
    tag: str,
) -> Dict[str, Any]:
    records: List[Dict[str, Any]] = []
    log_path = RAW_DIR / "llama-logs" / f"llama_server_{tag}_{stamp()}.log"
    server = LocalServer(exe, model_path, port, threads, log_path)
    meta: Dict[str, Any] = {
        "exe": exe,
        "exe_sha256": sha256_file(Path(exe)),
        "model_path": model_path,
        "model_file": Path(model_path).name,
        "model_sha256": sha256_file(Path(model_path)),
        "port": port,
        "threads": threads,
        "temperature": arm["temperature"],
        "prompt_key": arm["prompt_key"],
        "log": str(log_path.relative_to(REPO_ROOT)),
    }
    shutdown = None
    try:
        server.start()
        meta["health_ready_ms"] = server.health_ms
        for case in CASES:
            prompt, prompt_key = build_prompt(arm["variant"], case)
            call = server.chat([{"role": "user", "content": prompt}], arm["temperature"])
            records.append(make_record(case, arm, call, prompt, prompt_key))
    finally:
        shutdown = server.stop()
        meta["shutdown"] = shutdown
    summary = summarize_arm(records)
    failed_requests = sum(1 for r in records if not r["ok"])
    status = "REAL" if any(r["ok"] for r in records) else "BLOCKED"
    return {
        "arm": arm["label"],
        "kind": "local",
        "status": status,
        "meta": meta,
        "records": records,
        "summary": summary,
        "verdict": verdict_for(summary, failed_requests) if status == "REAL" else "BLOCKED",
    }


# --------------------------------------------------------------------------- #
# mock 自检：本地 HTTP mock 服务 + 两个臂（一个应全通过、一个应被检出失败）
# --------------------------------------------------------------------------- #
def mock_output(case: Dict[str, Any], satisfy: bool) -> str:
    """确定性构造「模拟译文」：保留结构并满足/故意违反检查项，用于验证判分框架。"""
    needles = [chk["value"] for chk in case["format_checks"] if chk["kind"] == "contains"]
    lines = case["text"].split("\n")
    zh_to_en = case["direction"] == "zh->en"
    filler = "Mock English translation." if zh_to_en else "这是模拟的中文译文内容。"
    out: List[str] = []
    injected = False
    for line in lines:
        if not line.strip():
            out.append("")
            continue
        if line.strip().startswith(("```", "~~~")):
            out.append(line.strip())
            continue
        body = filler
        matched = [n for n in needles if n in line]
        if matched:
            body += " " + " ".join(matched)
        else:
            bullet = re.match(r"^(\s*[-*•]\s+)", line)
            numbered = re.match(r"^(\s*\d+[.)．、]\s+)", line)
            body = (bullet.group(1) if bullet else (numbered.group(1) if numbered else "")) + body
        if not injected and case["required_terms"]:
            if satisfy:
                body += " " + " ".join(case["required_terms"])
                injected = True
            # satisfy=False：故意不注入术语 → 应被术语检查检出
        out.append(body)
    if not satisfy and out:
        # 故意丢最后一行 → 应被 lines_exact 检出
        for idx in range(len(out) - 1, -1, -1):
            if out[idx].strip():
                out.pop(idx)
                break
    return "\n".join(out)


class _MockHandler(BaseHTTPRequestHandler):
    def do_POST(self):  # noqa: N802
        if self.path.rstrip("/") != "/v1/chat/completions":
            self.send_response(404)
            self.end_headers()
            return
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length).decode("utf-8", "replace")
        body = json.loads(raw)
        prompt = (body.get("messages") or [{}])[0].get("content", "")
        satisfy = self.headers.get("X-Mock-Arm", "pass") != "fail"
        target = None
        for case in CASES:
            if case["text"] in prompt:
                target = case
                break
        if target is None:
            payload = {
                "error": "mock cannot locate case",
                "choices": [{"message": {"content": "[MOCK] case not found"}}],
            }
        else:
            payload = {
                "id": "mock-s14",
                "model": "mock-model",
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": mock_output(target, satisfy),
                        }
                    }
                ],
                "usage": {"prompt_tokens": len(prompt), "completion_tokens": 0},
            }
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):  # noqa: D401
        return


def run_mock_self_check() -> int:
    server = ThreadingHTTPServer(("127.0.0.1", 0), _MockHandler)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base_url = f"http://127.0.0.1:{port}"
    session = requests.Session()
    runs = []
    assertions = []
    try:
        for arm_name in ("mock-pass", "mock-fail"):
            arm = MOCK_ARMS[arm_name]
            records = []
            for case in CASES:
                prompt, prompt_key = build_prompt(arm["variant"], case)
                payload = {
                    "model": arm["model"],
                    "messages": [{"role": "user", "content": prompt}],
                    "temperature": 0.0,
                    "max_tokens": 2048,
                }
                t0 = time.perf_counter()
                resp = session.post(
                    f"{base_url}/v1/chat/completions",
                    json=payload,
                    timeout=10,
                    headers={"X-Mock-Arm": "pass" if arm["satisfy"] else "fail"},
                )
                latency = round((time.perf_counter() - t0) * 1000.0, 2)
                body = resp.json()
                content = (((body.get("choices") or [{}])[0]).get("message") or {}).get(
                    "content"
                ) or ""
                records.append(
                    make_record(
                        case,
                        {**arm, "kind": "mock"},
                        {"status": resp.status_code, "latency_ms": latency, "content": content, "usage": None, "error": None},
                        prompt,
                        prompt_key,
                    )
                )
            summary = summarize_arm(records)
            runs.append(
                {
                    "arm": arm["label"],
                    "kind": "mock",
                    "status": "MOCK_SELF_TEST",
                    "meta": {"mock": True, "satisfy": arm["satisfy"]},
                    "records": records,
                    "summary": summary,
                    "verdict": "MOCK_SELF_CHECK_OK",
                }
            )
        pass_summary = runs[0]["summary"]
        fail_summary = runs[1]["summary"]
        assertions.append(
            {
                "name": "mock-pass 全通过",
                "ok": pass_summary["case_pass"] == len(CASES)
                and pass_summary["term_hit_rate"] == 1.0
                and pass_summary["format_pass_rate"] == 1.0,
                "detail": f"case_pass={pass_summary['case_pass']}/{len(CASES)} "
                f"term={pass_summary['term_hit_rate']} format={pass_summary['format_pass_rate']}",
            }
        )
        assertions.append(
            {
                "name": "mock-fail 失败被检出（术语 + 格式）",
                "ok": fail_summary["term_hit_rate"] is not None
                and fail_summary["term_hit_rate"] < 1.0
                and fail_summary["format_pass_rate"] is not None
                and fail_summary["format_pass_rate"] < 1.0
                and len(fail_summary["failures"]) > 0,
                "detail": f"term={fail_summary['term_hit_rate']} "
                f"format={fail_summary['format_pass_rate']} failures={len(fail_summary['failures'])}",
            }
        )
    finally:
        session.close()
        server.shutdown()

    assertions_ok = all(a["ok"] for a in assertions)
    out = {
        "meta": {
            "script": "bench_translate_consistency.py",
            "script_version": SCRIPT_VERSION,
            "generated_at": now_iso(),
            "mode": "MOCK_SELF_TEST",
            "labels": ["MOCK"],
            "note": "mock 数据仅验证判分框架可跑通，不得作为 S-14 DONE/FAILED 依据。",
        },
        "runs": runs,
        "self_check_assertions": assertions,
        "self_check_ok": assertions_ok,
    }
    out_path = RAW_DIR / f"translate_consistency_mock_{stamp()}.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8")
    print("=" * 78)
    print("S-14 翻译一致性 —— MOCK 框架自检（非验收数据）")
    for item in assertions:
        print(f"  [{'PASS' if item['ok'] else 'FAIL'}] {item['name']}：{item['detail']}")
    print(f"  原始数据：{out_path}")
    print("=" * 78)
    return EXIT_OK if assertions_ok else EXIT_ERROR


# --------------------------------------------------------------------------- #
# 主流程
# --------------------------------------------------------------------------- #
def collect_meta() -> Dict[str, Any]:
    import platform

    return {
        "script": "bench_translate_consistency.py",
        "script_version": SCRIPT_VERSION,
        "generated_at": now_iso(),
        "mode": "REAL",
        "host": {
            "os": f"{platform.system()} {platform.release()}",
            "python": platform.python_version(),
            "requests": requests.__version__,
            "cpu_logical": os.cpu_count(),
        },
        "cases": {
            "count": len(CASES),
            "zh_to_en": sum(1 for c in CASES if c["direction"] == "zh->en"),
            "en_to_zh": sum(1 for c in CASES if c["direction"] == "en->zh"),
            "with_glossary": sum(1 for c in CASES if c["required_terms"]),
            "format_checks_total": sum(len(c["format_checks"]) for c in CASES),
            "required_terms_total": sum(len(c["required_terms"]) for c in CASES),
        },
        "prompts": {
            "translate.md": {
                "path": "resources/prompts/translate.md",
                "sha256": sha256_file(prompt_path("translate")),
                "chars_body": len(strip_maintenance_header(prompt_path("translate").read_text(encoding="utf-8"))),
            },
            "translate_minimal.md": {
                "path": "resources/prompts/translate_minimal.md",
                "sha256": sha256_file(prompt_path("translate_minimal")),
                "chars_body": len(strip_maintenance_header(prompt_path("translate_minimal").read_text(encoding="utf-8"))),
            },
        },
        "glossary_rendering": "与 src-tauri/src/services/translator.rs::render_glossary 一致：'from => to' 以 '; ' 连接",
        "backend_labeling": "每条记录 backend + backend_label；产品侧契约见 docs/spikes/S-14-翻译一致性.md §4",
        "thresholds": {"term_hit_rate": 1.0, "format_pass_rate": 1.0},
    }


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description="S-14 翻译一致性（术语 + 格式 + 后端标注）")
    ap.add_argument(
        "--arms",
        default="local-tiny-minimal",
        help=f"逗号分隔；可选：{','.join(ARMS)}（默认 local-tiny-minimal）",
    )
    ap.add_argument("--base-url", default=os.environ.get("SNIPASK_BENCH_BASE_URL", "https://api.deepseek.com"))
    ap.add_argument("--api-key", default="", help="仅测试用；优先 .verify/p1/key.txt 或环境变量")
    ap.add_argument("--llama-exe", default="")
    ap.add_argument("--port", type=int, default=39281)
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--mock-self-check", action="store_true")
    ap.add_argument("--out", default="")
    args = ap.parse_args(argv)

    if args.mock_self_check:
        return run_mock_self_check()

    arms = [a.strip() for a in args.arms.split(",") if a.strip()]
    unknown = [a for a in arms if a not in ARMS]
    if unknown:
        print(f"未知臂：{unknown}；可选：{list(ARMS)}")
        return EXIT_ERROR

    api_key, key_source = load_api_key(args.api_key)
    exe = args.llama_exe or find_llama_executable()
    out_dir = Path(args.out).parent if args.out else RAW_DIR
    out_dir.mkdir(parents=True, exist_ok=True)

    meta = collect_meta()
    meta["api_key_provided"] = bool(api_key)
    meta["api_key_source"] = key_source
    meta["base_url_masked"] = mask_base_url(args.base_url)
    meta["llama_exe"] = exe or "(未找到)"
    meta["requested_arms"] = arms

    runs: List[Dict[str, Any]] = []
    for arm_name in arms:
        arm = ARMS[arm_name]
        print("-" * 78)
        print(f"[臂] {arm_name} —— {arm['desc']}")
        if arm["kind"] == "cloud":
            if not api_key:
                print("  BLOCKED：无 Key（解锁步骤见 scripts/spikes/s12_s14/README.md §2）")
                runs.append(
                    {
                        "arm": arm["label"],
                        "kind": "cloud",
                        "status": "BLOCKED",
                        "reason": "无 Key：.verify/p1/key.txt 不存在且 SNIPASK_BENCH_API_KEY 为空",
                        "meta": {"api_key_provided": False},
                        "records": [],
                        "summary": {},
                        "verdict": "BLOCKED",
                    }
                )
                continue
            run = run_cloud_arm(arm, args.base_url, api_key)
        else:
            model_path = find_model(arm["model"])
            if not exe or not model_path:
                print(f"  BLOCKED：llama-server={exe or '(缺失)'} model={model_path or '(缺失)'}")
                runs.append(
                    {
                        "arm": arm["label"],
                        "kind": "local",
                        "status": "BLOCKED",
                        "reason": "缺少 llama-server.exe 或 GGUF",
                        "meta": {"exe": exe, "model": model_path},
                        "records": [],
                        "summary": {},
                        "verdict": "BLOCKED",
                    }
                )
                continue
            port = pick_free_port(args.port)
            run = run_local_arm(arm, exe, model_path, port, args.threads, tag=arm_name)
        s = run["summary"]
        print(
            f"  {run['verdict']}：术语 {s.get('term_hit_rate')} "
            f"格式 {s.get('format_pass_rate')} 译文有效 {s.get('translation_effective_rate')} "
            f"整例通过 {s.get('case_pass')}/{s.get('cases')}"
        )
        if run.get("meta", {}).get("shutdown"):
            sd = run["meta"]["shutdown"]
            print(f"  llama-server 清理：clean={sd.get('clean')} 残留={sd.get('remaining_after_cleanup')}")
        runs.append(run)

    result = {
        "meta": meta,
        "runs": runs,
        "aggregate": aggregate(runs),
    }
    out_path = Path(args.out) if args.out else RAW_DIR / f"translate_consistency_{stamp()}.json"
    out_path.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")

    print("=" * 78)
    print("S-14 翻译一致性汇总")
    for run in runs:
        s = run.get("summary") or {}
        print(
            f"  {run['arm']:<60} {run['status']:<8} "
            f"术语={s.get('term_hit_rate')} 格式={s.get('format_pass_rate')} 有效={s.get('translation_effective_rate')}"
        )
    print(f"  原始数据：{out_path}")
    print("=" * 78)

    real = sum(1 for r in runs if r["status"] == "REAL")
    if real == 0:
        return EXIT_BLOCKED
    return EXIT_OK


def aggregate(runs: List[Dict[str, Any]]) -> Dict[str, Any]:
    done = [r for r in runs if r["status"] == "REAL"]
    return {
        "real_arms": len(done),
        "blocked_arms": [r["arm"] for r in runs if r["status"] == "BLOCKED"],
        "done_arms": [r["arm"] for r in done if r["verdict"] == "DONE"],
        "failed_arms": [r["arm"] for r in done if r["verdict"].startswith("FAILED")],
        "targets_met": bool(done)
        and all(
            r["summary"]["term_hit_rate"] == 1.0 and r["summary"]["format_pass_rate"] == 1.0
            for r in done
        ),
    }


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
