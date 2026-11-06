#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S-06 负向证据探针：验证 DeepSeek API 是否支持「视觉识别」与「Search」。

用途：单 Key（DeepSeek）是否足以支撑图片搜索链路（视觉识别 → 检索词 → 1 次 Search → 摘要）。
结论只来自真实 HTTP 响应，响应体原样落盘（脱敏：不记录请求头/Authorization）。

Key 纪律：Key 只从环境变量 SNIPASK_BENCH_API_KEY 读取，**不写入任何文件/日志/输出**。

用法：
    $env:SNIPASK_BENCH_API_KEY = "sk-****"
    $env:SNIPASK_BENCH_BASE_URL = "https://api.deepseek.com"   # 默认
    python scripts/spikes/s05_s06/probe_s06_negative.py
输出：
    .verify/spikes/S-05-S-06/raw/deepseek_capability_probe_<时间戳>.json
退出码：0 = 探测完成（无论结论）；2 = 无 Key（BLOCKED）。
"""

from __future__ import annotations

import base64
import http.client
import json
import os
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path
from urllib.parse import urlsplit

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
RAW = REPO_ROOT / ".verify" / "spikes" / "S-05-S-06" / "raw"
IMAGE = REPO_ROOT / "docs" / "images" / "settings-ai.png"
BASE_URL = os.environ.get("SNIPASK_BENCH_BASE_URL", "https://api.deepseek.com").rstrip("/")
API_KEY = os.environ.get("SNIPASK_BENCH_API_KEY", "")


def request(method: str, url: str, payload=None, timeout=60):
    parts = urlsplit(url)
    path = parts.path + (f"?{parts.query}" if parts.query else "")
    if parts.scheme == "https":
        conn = http.client.HTTPSConnection(parts.hostname, parts.port or 443, timeout=timeout)
    else:
        conn = http.client.HTTPConnection(parts.hostname, parts.port or 80, timeout=timeout)
    headers = {"Authorization": f"Bearer {API_KEY}", "Content-Type": "application/json"}
    body = json.dumps(payload, ensure_ascii=False).encode("utf-8") if payload is not None else None
    try:
        conn.request(method, path, body=body, headers=headers)
        resp = conn.getresponse()
        raw = resp.read().decode("utf-8", "replace")
        return {"status": resp.status, "body": raw[:4000]}
    except Exception as err:  # noqa: BLE001 - 网络错误也是证据
        return {"status": None, "body": f"{type(err).__name__}: {err}"}
    finally:
        conn.close()


def main() -> int:
    if not API_KEY:
        print("BLOCKED：未提供 SNIPASK_BENCH_API_KEY，无法做负向证据探测。")
        return 2
    if not IMAGE.exists():
        print(f"BLOCKED：测试图片不存在：{IMAGE}")
        return 2
    RAW.mkdir(parents=True, exist_ok=True)
    data_url = "data:image/png;base64," + base64.b64encode(IMAGE.read_bytes()).decode("ascii")
    timestamp = datetime.now(timezone(timedelta(hours=8))).isoformat()
    model_guess = os.environ.get("SNIPASK_BENCH_VISION_MODEL") or "deepseek-flash"
    vision_max_tokens = int(os.environ.get("SNIPASK_PROBE_MAX_TOKENS", "800"))

    result = {
        "probe": "S-06 负向证据：DeepSeek 单 Key 是否支持视觉与 Search",
        "generated_at": timestamp,
        "base_url": BASE_URL,
        "image": {
            "path": str(IMAGE.relative_to(REPO_ROOT)),
            "size_bytes": IMAGE.stat().st_size,
        },
        "api_key_provided": True,
        "probes": [],
    }

    # 1) /models（记录可用模型与模态声明，不含敏感信息）
    models = request("GET", f"{BASE_URL}/models")
    model_ids = []
    if models["status"] == 200:
        try:
            obj = json.loads(models["body"])
            model_ids = [m.get("id") for m in obj.get("data", [])]
            result["models"] = [
                {
                    "id": m.get("id"),
                    "input_modalities": m.get("input_modalities"),
                    "output_modalities": m.get("output_modalities"),
                    "context_window": m.get("context_window"),
                    "effort": m.get("effort"),
                    "api_capabilities": m.get("api_capabilities"),
                }
                for m in obj.get("data", [])
            ]
        except json.JSONDecodeError:
            result["models"] = []
    result["probes"].append({"name": "GET /models", "status": models["status"], "body": models["body"][:2000]})

    # 2) 视觉能力：图片消息（OpenAI 兼容 content 数组 + data URL）
    for model in model_ids or [model_guess]:
        payload = {
            "model": model,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "描述这张截图的文字和界面内容，30 字以内。"},
                        {"type": "image_url", "image_url": {"url": data_url}},
                    ],
                }
            ],
            "max_tokens": vision_max_tokens,
        }
        probe = request("POST", f"{BASE_URL}/chat/completions", payload)
        entry = {
            "name": f"vision: POST /chat/completions (image content, model={model})",
            "model": model,
            "status": probe["status"],
            "body": probe["body"][:3000],
        }
        if probe["status"] == 200:
            try:
                obj = json.loads(probe["body"])
                msg = (obj.get("choices") or [{}])[0].get("message") or {}
                entry["reply"] = (msg.get("content") or "")[:800]
                entry["reasoning_content"] = (msg.get("reasoning_content") or "")[:400]
                entry["usage"] = obj.get("usage")
            except json.JSONDecodeError:
                pass
        result["probes"].append(entry)

    # 3) Search 端点探测（DeepSeek 是否暴露搜索 API）
    for method, path, payload in [
        ("POST", "/search", {"q": "框选 AI"}),
        ("GET", "/search", None),
        ("POST", "/v1/search", {"query": "框选 AI"}),
        ("POST", "/search/completions", {"query": "框选 AI"}),
    ]:
        probe = request(method, f"{BASE_URL}{path}", payload)
        result["probes"].append(
            {"name": f"search: {method} {path}", "status": probe["status"], "body": probe["body"][:1200]}
        )

    result["conclusion"] = {
        "vision_supported": any(
            p.get("name", "").startswith("vision") and p.get("status") == 200
            for p in result["probes"]
        ),
        "search_endpoint_found": any(
            p.get("name", "").startswith("search") and p.get("status") == 200
            for p in result["probes"]
        ),
        "note": "结论以真实 HTTP 响应为准；视觉 200 表示 Chat API 接受图片输入，但不等于具备产品所需"
        "「识别 + 检索 + 来源」链路；Search 端点全部非 200 则图片搜索链路仍缺检索腿。",
    }

    out = RAW / f"deepseek_capability_probe_{datetime.now().strftime('%Y%m%d-%H%M%S')}.json"
    out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"probe written: {out}")
    for p in result["probes"]:
        print(f"  {p['name']}: status={p['status']}")
    print(f"vision_supported={result['conclusion']['vision_supported']}, "
          f"search_endpoint_found={result['conclusion']['search_endpoint_found']}")
    return 0


if __name__ == "__main__":
    if hasattr(sys.stdout, "reconfigure"):
        try:
            sys.stdout.reconfigure(encoding="utf-8")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
