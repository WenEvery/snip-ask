#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S-14 翻译一致性样例集（术语 + 格式保留）。

24 条样例，覆盖中→英 / 英→中；每条包含：
    - 待翻译文本（含换行 / 空行 / 列表 / 编号 / 代码块 / URL / 邮箱 / 占位符 / 版本号 / 术语）
    - 术语表（与产品 `render_glossary` 的 "原文 => 译文" 语义一致）
    - 期望命中的术语（required_terms：术语表命中词，允许出现在输出任意位置）
    - 格式检查项（format checks：换行数 / 空行 / 列表 / 编号 / 代码围栏 / 代码内容 /
      URL / 邮箱 / 占位符 / 标识符），全部逐项判定、可复算。

判分口径（详见 docs/spikes/S-14-翻译一致性.md §2）：
    术语命中率 = 命中的 required_terms 数 / 总 required_terms 数（目标 100%）
    格式保留率 = 通过的格式检查项 / 总格式检查项（按换行/列表/代码/URL 等逐项统计，目标 100%）
    译文有效性 = 中→英输出 CJK 占比 < 0.30；英→中输出 CJK 占比 > 0.15 且不等于原文
                  （用于识别 S-05 已实测的「0.5B 复述原文」失败模式）

本模块只含纯数据与纯函数，不联网、不启动任何进程；由 bench_translate_consistency.py 使用。
"""

from __future__ import annotations

import re
from typing import Any, Dict, List

# 术语表（产品契约：TranslatePrefs.glossary: [{from, to}]；渲染为 "from => to" 以 "; " 连接）
GLOSSARY_ZH_EN = [
    {"from": "框选 AI", "to": "SnipAsk"},
    {"from": "结果窗", "to": "Result Window"},
    {"from": "术语表", "to": "Glossary"},
    {"from": "引导", "to": "Onboarding"},
]
GLOSSARY_EN_ZH = [
    {"from": "API", "to": "接口"},
    {"from": "SnipAsk", "to": "框选 AI"},
    {"from": "Result Window", "to": "结果窗"},
    {"from": "Onboarding", "to": "引导"},
    {"from": "Glossary", "to": "术语表"},
]


def _chk_contains(*items: str) -> List[Dict[str, Any]]:
    """把一组必须原样保留的字符串展开成逐项 contains 检查。"""
    return [{"kind": "contains", "label": f"contains:{item}", "value": item} for item in items]


def _case(
    case_id: str,
    direction: str,
    category: str,
    text: str,
    glossary: List[Dict[str, str]],
    required_terms: List[str],
    checks: List[Dict[str, Any]],
    note: str = "",
) -> Dict[str, Any]:
    src, tgt = ("zh-CHS", "en") if direction == "zh->en" else ("en", "zh-CHS")
    return {
        "id": case_id,
        "direction": direction,
        "category": category,
        "source_lang": src,
        "target_lang": tgt,
        "text": text,
        "glossary": glossary,
        "required_terms": required_terms,
        "format_checks": checks,
        "note": note,
    }


CASES: List[Dict[str, Any]] = [
    # ---------------------------------------------------------------- 中 → 英 --
    _case(
        "zh-en-01-plain",
        "zh->en",
        "段落",
        "框选 AI 提供截图问答与翻译能力。按下 Ctrl+Q 即可唤起选框，松开后弹出动作菜单。",
        [GLOSSARY_ZH_EN[0]],
        ["SnipAsk"],
        [{"kind": "lines_exact", "label": "lines_exact", "value": 1}] + _chk_contains("Ctrl+Q"),
        "普通段落 + 品牌术语 + 快捷键标识符",
    ),
    _case(
        "zh-en-02-multiline",
        "zh->en",
        "多行",
        "设置已保存。\n重新启动应用后生效。\n如需帮助请查看文档。",
        [],
        [],
        [{"kind": "lines_exact", "label": "lines_exact", "value": 3}],
        "三行界面文案，必须保留换行（不得合并成一行）",
    ),
    _case(
        "zh-en-03-bullets",
        "zh->en",
        "列表",
        "- 支持屏幕取词\n- 支持截图问答\n- 支持本地翻译",
        [],
        [],
        [
            {"kind": "lines_exact", "label": "lines_exact", "value": 3},
            {"kind": "bullets_min", "label": "bullets_min", "value": 3},
        ],
        "无序列表 3 项，列表符号必须保留",
    ),
    _case(
        "zh-en-04-numbered",
        "zh->en",
        "列表",
        "1. 打开设置\n2. 选择 AI 服务\n3. 保存配置",
        [],
        [],
        [
            {"kind": "lines_exact", "label": "lines_exact", "value": 3},
            {"kind": "numbered_min", "label": "numbered_min", "value": 3},
        ],
        "有序列表 3 项，编号必须保留",
    ),
    _case(
        "zh-en-05-code",
        "zh->en",
        "代码",
        "请运行以下命令：\n```\npython bench.py --runs 20\n```\n完成后查看输出。",
        [],
        [],
        [
            {"kind": "lines_exact", "label": "lines_exact", "value": 5},
            {"kind": "code_fences_exact", "label": "code_fences_exact", "value": 2},
        ]
        + _chk_contains("python bench.py --runs 20"),
        "代码块必须原样保留（围栏 + 代码内容不翻译）",
    ),
    _case(
        "zh-en-06-url-mail",
        "zh->en",
        "URL",
        "文档地址 https://example.com/docs/s14 与支持邮箱 support@example.com 均可使用。",
        [],
        [],
        _chk_contains("https://example.com/docs/s14", "support@example.com"),
        "URL 与邮箱必须逐字保留",
    ),
    _case(
        "zh-en-07-placeholder",
        "zh->en",
        "占位符",
        "欢迎 {user}，你有 {count} 条未读消息。",
        [],
        [],
        [{"kind": "lines_exact", "label": "lines_exact", "value": 1}]
        + _chk_contains("{user}", "{count}"),
        "变量占位符必须原样保留（translate.md 要求）",
    ),
    _case(
        "zh-en-08-identifiers",
        "zh->en",
        "标识符",
        "请升级到 v1.2.3，模型 deepseek-flash 已在 Linux 与 macOS 上验证。",
        [],
        [],
        _chk_contains("v1.2.3", "deepseek-flash", "Linux", "macOS"),
        "版本号 / 模型名 / 产品名必须保留原文",
    ),
    _case(
        "zh-en-09-term-heavy",
        "zh->en",
        "术语",
        "在结果窗中打开术语表，即可查看本次翻译使用的术语。",
        [GLOSSARY_ZH_EN[1], GLOSSARY_ZH_EN[2]],
        ["Result Window", "Glossary"],
        [{"kind": "lines_exact", "label": "lines_exact", "value": 1}],
        "两个术语必须命中指定译法",
    ),
    _case(
        "zh-en-10-blankline",
        "zh->en",
        "空行",
        "第一段：框选 AI 支持离线翻译。\n\n第二段：首次使用请完成引导。",
        [GLOSSARY_ZH_EN[0], GLOSSARY_ZH_EN[3]],
        ["SnipAsk", "Onboarding"],
        [
            {"kind": "lines_exact", "label": "lines_exact", "value": 2},
            {"kind": "blank_lines_min", "label": "blank_lines_min", "value": 1},
        ],
        "段落间空行必须保留",
    ),
    _case(
        "zh-en-11-error",
        "zh->en",
        "错误码",
        "错误码 SA-1001：截图失败，该错误可重试。请检查屏幕录制权限。",
        [],
        [],
        _chk_contains("SA-1001"),
        "错误码必须保留；译文语气为界面文案",
    ),
    _case(
        "zh-en-12-long",
        "zh->en",
        "长段落",
        "框选 AI 是一款屏幕取词与截图问答工具：按下 Ctrl+Q 唤起选框，框住屏幕上的文字或图像，"
        "松开鼠标后弹出动作菜单。你可以翻译选中段落、提取文字并搜索、识别图片内容，或向 AI 提问。"
        "所有动作都在结果窗中展示，支持复制、重新执行与追问。",
        [GLOSSARY_ZH_EN[0], GLOSSARY_ZH_EN[1]],
        ["SnipAsk", "Result Window"],
        _chk_contains("Ctrl+Q"),
        "200 字级长段落（对齐 S-05 时延样例口径），术语 + 标识符",
    ),
    # ---------------------------------------------------------------- 英 → 中 --
    _case(
        "en-zh-01-plain",
        "en->zh",
        "段落",
        "SnipAsk captures text from your screen and answers questions about screenshots.",
        [GLOSSARY_EN_ZH[1]],
        ["框选 AI"],
        [{"kind": "lines_exact", "label": "lines_exact", "value": 1}],
        "普通段落 + 品牌术语（英→中）",
    ),
    _case(
        "en-zh-02-multiline",
        "en->zh",
        "多行",
        "Settings saved.\nRestart the app to apply.\nSee the documentation for help.",
        [],
        [],
        [{"kind": "lines_exact", "label": "lines_exact", "value": 3}],
        "三行界面文案（英→中），必须保留换行",
    ),
    _case(
        "en-zh-03-bullets",
        "en->zh",
        "列表",
        "- Capture text from the screen\n- Answer questions about screenshots\n- Translate locally",
        [],
        [],
        [
            {"kind": "lines_exact", "label": "lines_exact", "value": 3},
            {"kind": "bullets_min", "label": "bullets_min", "value": 3},
        ],
        "无序列表 3 项（英→中）",
    ),
    _case(
        "en-zh-04-numbered",
        "en->zh",
        "列表",
        "1. Open settings\n2. Choose the API provider\n3. Save the configuration",
        [GLOSSARY_EN_ZH[0]],
        ["接口"],
        [
            {"kind": "lines_exact", "label": "lines_exact", "value": 3},
            {"kind": "numbered_min", "label": "numbered_min", "value": 3},
        ],
        "有序列表 + 术语 API→接口",
    ),
    _case(
        "en-zh-05-code",
        "en->zh",
        "代码",
        "Run this command:\n```\npython bench.py --runs 20\n```\nThen check the output.",
        [],
        [],
        [
            {"kind": "lines_exact", "label": "lines_exact", "value": 5},
            {"kind": "code_fences_exact", "label": "code_fences_exact", "value": 2},
        ]
        + _chk_contains("python bench.py --runs 20"),
        "代码块（英→中）必须原样保留",
    ),
    _case(
        "en-zh-06-url-mail",
        "en->zh",
        "URL",
        "Docs: https://example.com/docs/s14 — support: support@example.com",
        [],
        [],
        _chk_contains("https://example.com/docs/s14", "support@example.com"),
        "URL 与邮箱（英→中）",
    ),
    _case(
        "en-zh-07-placeholder",
        "en->zh",
        "占位符",
        "Welcome {user}, you have {count} unread messages.",
        [],
        [],
        [{"kind": "lines_exact", "label": "lines_exact", "value": 1}]
        + _chk_contains("{user}", "{count}"),
        "占位符（英→中）",
    ),
    _case(
        "en-zh-08-identifiers",
        "en->zh",
        "标识符",
        "Upgrade to v1.2.3; the deepseek-flash model is verified on Linux and macOS.",
        [],
        [],
        _chk_contains("v1.2.3", "deepseek-flash", "Linux", "macOS"),
        "版本号 / 模型名（英→中）",
    ),
    _case(
        "en-zh-09-term-heavy",
        "en->zh",
        "术语",
        "Open the Glossary from the Result Window to review the terms used in this translation.",
        [GLOSSARY_EN_ZH[2], GLOSSARY_EN_ZH[4]],
        ["结果窗", "术语表"],
        [{"kind": "lines_exact", "label": "lines_exact", "value": 1}],
        "两个术语必须命中指定译法（英→中）",
    ),
    _case(
        "en-zh-10-blankline",
        "en->zh",
        "空行",
        "Paragraph one: SnipAsk works offline.\n\nParagraph two: finish Onboarding on first launch.",
        [GLOSSARY_EN_ZH[1], GLOSSARY_EN_ZH[3]],
        ["框选 AI", "引导"],
        [
            {"kind": "lines_exact", "label": "lines_exact", "value": 2},
            {"kind": "blank_lines_min", "label": "blank_lines_min", "value": 1},
        ],
        "段落间空行（英→中）",
    ),
    _case(
        "en-zh-11-error",
        "en->zh",
        "错误码",
        "Error SA-1001: screenshot failed and can be retried. Check screen recording permission.",
        [],
        [],
        _chk_contains("SA-1001"),
        "错误码（英→中）",
    ),
    _case(
        "en-zh-12-long",
        "en->zh",
        "长段落",
        "SnipAsk is a screen-capture and screenshot question-and-answer tool. Press Ctrl+Q to open the "
        "selection box, then select text or images on the screen. Release the mouse to open the action "
        "menu. You can translate the selection, extract and search text, recognize image content, or ask "
        "the AI a question. All actions appear in the Result Window and support copy, retry and follow-up.",
        [GLOSSARY_EN_ZH[1], GLOSSARY_EN_ZH[2]],
        ["框选 AI", "结果窗"],
        _chk_contains("Ctrl+Q"),
        "150 字级长段落（英→中），术语 + 标识符",
    ),
]

# 自检：样例数量与覆盖面（防止手改漂移）
assert len(CASES) >= 20, f"S-14 需要 ≥20 条样例，当前 {len(CASES)}"
assert sum(1 for c in CASES if c["direction"] == "zh->en") >= 8
assert sum(1 for c in CASES if c["direction"] == "en->zh") >= 8
assert sum(1 for c in CASES if c["required_terms"]) >= 8, "至少 8 条含术语命中要求"


def render_glossary(glossary: List[Dict[str, str]]) -> str:
    """与 src-tauri/src/services/translator.rs::render_glossary 语义一致。

    Rust 实现：过滤 from 非空的条目，映射为 "from => to"，以 "; " 连接。
    """
    return "; ".join(
        f"{entry['from']} => {entry['to']}" for entry in glossary if entry.get("from")
    )


def strip_maintenance_header(template: str) -> str:
    """与 prompt_engine.rs::strip_maintenance_header 一致：跳过开头 # 注释行与空行。"""
    body_started = False
    body: List[str] = []
    for line in template.splitlines():
        if not body_started:
            if line.lstrip().startswith("#") or not line.strip():
                continue
            body_started = True
        body.append(line)
    return "\n".join(body).rstrip()


def cjk_ratio(text: str) -> float:
    chars = [c for c in text]
    if not chars:
        return 0.0
    cjk = sum(1 for c in chars if "\u4e00" <= c <= "\u9fff")
    return cjk / len(chars)


def _line_kind(line: str) -> str:
    stripped = line.strip()
    if stripped.startswith(("```", "~~~")):
        return "fence"
    if re.match(r"^\s*[-*•]\s+", line):
        return "bullet"
    if re.match(r"^\s*\d+[.)．、]\s+", line):
        return "numbered"
    return "text"


# 用于逐项格式检查的纯函数（供脚本与自检复用）
def evaluate_check(check: Dict[str, Any], source: str, output: str) -> Dict[str, Any]:
    """执行单项格式检查，返回 {label, kind, expected, actual, pass, detail}。"""
    kind = check["kind"]
    label = check.get("label", kind)
    expected = check.get("value")
    out_lines = output.splitlines()
    nonempty = [ln for ln in out_lines if ln.strip()]

    if kind == "lines_exact":
        actual = len(nonempty)
        ok = actual == expected
        detail = f"非空行数 {actual} / 期望 {expected}"
    elif kind == "lines_min":
        actual = len(nonempty)
        ok = actual >= expected
        detail = f"非空行数 {actual} / 至少 {expected}"
    elif kind == "blank_lines_min":
        blank = sum(1 for ln in out_lines if not ln.strip())
        actual = blank
        ok = blank >= expected
        detail = f"空行数 {blank} / 至少 {expected}"
    elif kind == "bullets_min":
        bullets = sum(1 for ln in out_lines if _line_kind(ln) == "bullet")
        actual = bullets
        ok = bullets >= expected
        detail = f"列表行 {bullets} / 至少 {expected}"
    elif kind == "numbered_min":
        numbered = sum(1 for ln in out_lines if _line_kind(ln) == "numbered")
        actual = numbered
        ok = numbered >= expected
        detail = f"编号行 {numbered} / 至少 {expected}"
    elif kind == "code_fences_exact":
        fences = output.count("```")
        actual = fences
        ok = fences == expected
        detail = f"代码围栏 ``` 出现 {fences} 次 / 期望 {expected}"
    elif kind == "contains":
        needle = str(expected)
        actual = needle if needle in output else None
        ok = needle in output
        detail = f"应原样保留 {needle!r}；{'已保留' if ok else '缺失'}"
    else:  # pragma: no cover - 防御未知检查项
        raise ValueError(f"未知格式检查 kind：{kind}")

    return {
        "label": label,
        "kind": kind,
        "expected": expected,
        "actual": actual,
        "pass": bool(ok),
        "detail": detail,
    }


def translation_effective(case: Dict[str, Any], output: str) -> Dict[str, Any]:
    """译文有效性：用于识别「未翻译/复述原文」（S-05 已实测 0.5B + 产品长提示词的失败模式）。"""
    ratio = cjk_ratio(output)
    if case["direction"] == "zh->en":
        ok = ratio < 0.30
        reason = f"中文占比 {ratio:.1%}（期望 <30%，否则视为未翻译/复述）"
    else:
        same = output.strip() == case["text"].strip()
        ok = ratio > 0.15 and not same
        reason = f"中文占比 {ratio:.1%}（期望 >15%）且{'等于原文' if same else '不等于原文'}"
    return {"pass": bool(ok), "cjk_ratio": round(ratio, 4), "detail": reason}
