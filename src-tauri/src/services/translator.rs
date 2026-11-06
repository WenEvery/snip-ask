//! 翻译引擎（M11 / 说明书 §4.6 / §7.2 / 契约 v5 §3.4）。
//!
//! 三类真实实现 + 一条回退链：
//! - [`GatewayTranslator`]：通用 OpenAI 兼容 LLM（云端供应商 / 本地 llama-server），
//!   提示词来自 `prompt_engine`（`translate.md`，含 `{glossary}`；本地极小模型用
//!   `translate_minimal.md` / 术语表非空时 `translate_minimal_glossary.md`）；
//! - [`QwenMtTranslator`]：DashScope Qwen-MT 原生翻译端点（`translation_options`，
//!   含术语干预 `terms`），真实协议见文件内 [`qwen_mt_generation_url`] 注释；
//! - [`SkeletonTranslator`]：骨架模式占位（契约 §12，`skeletonMode=true` 时保持原行为）；
//! - [`translate_with_config`] / [`translate_attempts`]：按配置解析「主后端 → 降级链」
//!   顺序（云端 provider 之间、云端↔本地），逐个尝试，全部失败返回可操作错误。
//!
//! # v1.0.4 本地翻译「回显原文」修复
//!
//! 本地小模型（Qwen2.5-0.5B/1.5B、HY-MT1.5-1.8B）不认识 `zh-CHS` 这类 BCP-47 码，
//! 提示词里带语言码时会把原文原样吐出。修复分三层：
//! 1. [`prompt_language_name`] 把语言码映射为中文语言名，仅用于本地精简提示词与覆盖翻译提示词；
//! 2. [`looks_untranslated`] 严格检测「输出 == 原文且不含目标文字脚本」，命中记失败（[`untranslated_output_error`]）
//!    并继续降级链；同语言合法原样输出与未知语言码不会被误判；
//! 3. `task_runner` 对本地结果先缓冲、校验通过后才推送前端（不把原文先推给用户再报错）。
//!
//! # 实测协议事实（2026-11-06，DashScope 官方文档核对）
//!
//! Qwen-MT 原生端点：`POST {base}/api/v1/services/aigc/text-generation/generation`
//! - 请求体：`{"model": "qwen-mt-lite", "input": {"messages": [{"role": "user",
//!   "content": "待翻译文本"}]}, "parameters": {"translation_options": {"source_lang": "auto",
//!   "target_lang": "Chinese", "terms": [{"source": "...", "target": "..."}]}}}`
//!   （`source_lang` / `target_lang` 为**英文语言全称**，不是 BCP-47 码）；
//! - 响应：`{"status_code": 200, "output": {"choices": [{"message": {"content": "译文"}}],
//!   "text": null}, "usage": {"input_tokens": N, "output_tokens": N}}`；
//! - 鉴权：`Authorization: Bearer <DASHSCOPE_API_KEY>`。
//!
//! 本机无 DashScope Key → 真机翻译标注 blocked，协议级验收用本地 mock server
//! （`src-tauri/tests/cloud_translate.rs`）。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::contract::config::{Config, GlossaryEntry, ProviderKind, TranslateBackend};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{Mode, TokenUsage};

/// 翻译请求。
#[derive(Debug, Clone)]
pub struct TranslateRequest {
    /// 原文。
    pub text: String,
    /// 源语言（`None` / `auto` 表示自动检测）。
    pub source_lang: Option<String>,
    /// 目标语言。
    pub target_lang: String,
    /// 模式。
    pub mode: Mode,
    /// 术语表（Qwen-MT 走 `translation_options.terms`；LLM 走提示词 `{glossary}`）。
    pub glossary: Vec<GlossaryEntry>,
}

impl TranslateRequest {
    /// 便捷构造：源语言 `auto`、无术语表。
    pub fn new(text: impl Into<String>, target_lang: impl Into<String>, mode: Mode) -> Self {
        Self {
            text: text.into(),
            source_lang: Some("auto".to_string()),
            target_lang: target_lang.into(),
            mode,
            glossary: Vec::new(),
        }
    }

    /// 补充术语表。
    pub fn with_glossary(mut self, glossary: Vec<GlossaryEntry>) -> Self {
        self.glossary = glossary;
        self
    }

    /// 源语言字符串（`auto` 缺省）。
    pub fn source_lang_or_auto(&self) -> &str {
        self.source_lang
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .unwrap_or("auto")
    }
}

/// 翻译器抽象。
#[async_trait]
pub trait Translator: Send + Sync {
    /// 执行翻译，返回译文。
    async fn translate(&self, request: &TranslateRequest) -> AppResult<String>;
}

/// 骨架翻译器（契约 §12：流式输出 `【骨架模式译文】` + 原文）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SkeletonTranslator;

impl SkeletonTranslator {
    /// 骨架模式占位译文。
    pub fn placeholder(text: &str) -> String {
        format!("【骨架模式译文】{text}")
    }
}

#[async_trait]
impl Translator for SkeletonTranslator {
    async fn translate(&self, request: &TranslateRequest) -> AppResult<String> {
        if request.text.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidState,
                Some("待翻译文本为空".to_string()),
            ));
        }
        Ok(Self::placeholder(&request.text))
    }
}

/// 视图与展示：回退链（按配置给出可选降级路径）。
pub fn fallback_chain(config: &Config) -> Vec<&'static str> {
    match config.translate.backend {
        TranslateBackend::Cloud => vec!["local", "another_cloud"],
        TranslateBackend::Local => vec!["cloud", "another_cloud"],
    }
}

/// DashScope Qwen-MT 原生端点路径（相对 Base URL）。
pub const QWEN_MT_GENERATION_PATH: &str = "/api/v1/services/aigc/text-generation/generation";
/// Qwen-MT 连接超时。
pub const QWEN_MT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Qwen-MT 单次请求总超时（长文翻译给足余量）。
pub const QWEN_MT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// 错误 detail 中保留的上游响应体长度上限。
const ERROR_BODY_LIMIT: usize = 500;

/// Qwen-MT 原生端点 URL（兼容 baseUrl 带/不带 `/api/v1`、`/compatible-mode/v1`、`/v1`）。
///
/// 官方实测口径（2026-11-06 核对 `help.aliyun.com/zh/model-studio/qwen-mt-api`）：
/// `POST https://dashscope.aliyuncs.com/api/v1/services/aigc/text-generation/generation`
/// 请求 `{model, input:{messages}, parameters:{translation_options}}`，
/// 响应 `{status_code, output:{choices|text}, usage:{input_tokens, output_tokens}}`。
pub fn qwen_mt_generation_url(base_url: &str) -> String {
    let mut base = base_url.trim().trim_end_matches('/').to_string();
    for suffix in [
        QWEN_MT_GENERATION_PATH,
        "/api/v1",
        "/compatible-mode/v1",
        "/v1",
    ] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.to_string();
        }
    }
    format!("{}{}", base.trim_end_matches('/'), QWEN_MT_GENERATION_PATH)
}

/// 是否为 DashScope Qwen-MT 的 Base URL（`kind` 字段缺失时的兜底判定）。
pub fn is_qwen_mt_base_url(base_url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(base_url.trim()) else {
        return false;
    };
    matches!(
        parsed.host_str(),
        Some("dashscope.aliyuncs.com") | Some("dashscope-intl.aliyuncs.com")
    )
}

/// 供应商是否为 Qwen-MT：优先显式 `kind`，其次按 DashScope 域名兜底。
pub fn provider_is_qwen_mt(kind: ProviderKind, base_url: &str) -> bool {
    kind == ProviderKind::QwenMt || is_qwen_mt_base_url(base_url)
}

/// BCP-47 / 简化语言码 → Qwen-MT 要求的「英文语言全称」。
///
/// Qwen-MT 的 `source_lang` / `target_lang` 必须是英文全称（如 `Chinese` / `English`），
/// 不接受 `zh-CHS` 这类 BCP-47 码；未知代码原样透传（由上游给出明确错误，不静默猜错语言）。
pub fn qwen_mt_language_name(code: &str) -> String {
    let normalized = code.trim();
    if normalized.is_empty() {
        return "auto".to_string();
    }
    let key = normalized.to_ascii_lowercase().replace('_', "-");
    let mapped = match key.as_str() {
        "auto" => "auto",
        "zh" | "zh-chs" | "zh-cn" | "zh-hans" | "chinese" | "chi" => "Chinese",
        "zh-cht" | "zh-tw" | "zh-hk" | "zh-hant" | "traditional chinese" => "Traditional Chinese",
        "en" | "en-us" | "en-gb" | "english" => "English",
        "ja" | "jp" | "japanese" => "Japanese",
        "ko" | "kr" | "korean" => "Korean",
        "fr" | "french" => "French",
        "de" | "ger" | "german" => "German",
        "es" | "spanish" => "Spanish",
        "ru" | "russian" => "Russian",
        "pt" | "portuguese" => "Portuguese",
        "it" | "italian" => "Italian",
        "ar" | "arabic" => "Arabic",
        "th" | "thai" => "Thai",
        "vi" | "vietnamese" => "Vietnamese",
        "id" | "indonesian" => "Indonesian",
        "ms" | "malay" => "Malay",
        "nl" | "dutch" => "Dutch",
        "tr" | "turkish" => "Turkish",
        "hi" | "hindi" => "Hindi",
        "pl" | "polish" => "Polish",
        "sv" | "swedish" => "Swedish",
        "da" | "danish" => "Danish",
        "fi" | "finnish" => "Finnish",
        "no" | "norwegian" => "Norwegian",
        "cs" | "czech" => "Czech",
        "ro" | "romanian" => "Romanian",
        "hu" | "hungarian" => "Hungarian",
        "el" | "greek" => "Greek",
        "he" | "hebrew" => "Hebrew",
        "uk" | "ukrainian" => "Ukrainian",
        "bg" | "bulgarian" => "Bulgarian",
        "fa" | "persian" => "Persian",
        "ur" | "urdu" => "Urdu",
        "sr" | "serbian" => "Serbian",
        "hr" | "croatian" => "Croatian",
        "sk" | "slovak" => "Slovak",
        "sl" | "slovenian" => "Slovenian",
        "lt" | "lithuanian" => "Lithuanian",
        "lv" | "latvian" => "Latvian",
        "et" | "estonian" => "Estonian",
        "ca" | "catalan" => "Catalan",
        "fil" | "tl" | "filipino" => "Filipino",
        "ne" | "nepali" => "Nepali",
        "km" | "khmer" => "Khmer",
        "lo" | "lao" => "Lao",
        "my" | "burmese" => "Burmese",
        "si" | "sinhala" => "Sinhala",
        "ta" | "tamil" => "Tamil",
        "te" | "telugu" => "Telugu",
        "bn" | "bengali" => "Bengali",
        "gu" | "gujarati" => "Gujarati",
        "kn" | "kannada" => "Kannada",
        "ml" | "malayalam" => "Malayalam",
        "mr" | "marathi" => "Marathi",
        "pa" | "punjabi" => "Punjabi",
        "sw" | "swahili" => "Swahili",
        "yue" | "cantonese" => "Cantonese",
        _ => return normalized.to_string(),
    };
    mapped.to_string()
}

/// BCP-47 / ISO 语言码 → 人类可读**中文语言名**（仅用于本地精简提示词与覆盖翻译提示词）。
///
/// 本地小模型（Qwen2.5-0.5B/1.5B、HY-MT1.5-1.8B 实测）不认识 `zh-CHS` 这类 BCP-47 码：
/// 提示词里带语言码时会把原文原样复述；换成「简体中文」等中文语言名后能正确翻译。
/// 云端 `translate.md` 继续使用 BCP-47 码（云端模型已验证正常），不经过本函数。
///
/// 已是中文名或无法识别的代码原样透传（trim 后）；空串返回空串。
pub fn prompt_language_name(code: &str) -> String {
    let normalized = code.trim();
    if normalized.is_empty() {
        return String::new();
    }
    // 已是中文语言名：原样返回（不再二次映射）。
    if normalized.chars().any(|ch| (ch as u32) >= 0x2E80) {
        return normalized.to_string();
    }
    let key = normalized.to_ascii_lowercase().replace('_', "-");
    let mapped = match key.as_str() {
        "auto" => "自动检测",
        "zh" | "zh-chs" | "zh-cn" | "zh-hans" | "zh-hans-cn" | "chinese" | "chi" => "简体中文",
        "zh-cht" | "zh-tw" | "zh-hk" | "zh-hant" | "zh-hant-tw" | "traditional chinese" => {
            "繁体中文"
        }
        "en" | "en-us" | "en-gb" | "english" => "英文",
        "ja" | "jp" | "japanese" => "日文",
        "ko" | "kr" | "korean" => "韩文",
        "fr" | "french" => "法文",
        "de" | "ger" | "german" => "德文",
        "es" | "spanish" => "西班牙文",
        "ru" | "russian" => "俄文",
        "pt" | "portuguese" => "葡萄牙文",
        "it" | "italian" => "意大利文",
        "ar" | "arabic" => "阿拉伯文",
        "th" | "thai" => "泰文",
        "vi" | "vietnamese" => "越南文",
        "id" | "indonesian" => "印度尼西亚文",
        "ms" | "malay" => "马来文",
        "nl" | "dutch" => "荷兰文",
        "tr" | "turkish" => "土耳其文",
        "hi" | "hindi" => "印地文",
        "pl" | "polish" => "波兰文",
        "sv" | "swedish" => "瑞典文",
        "da" | "danish" => "丹麦文",
        "fi" | "finnish" => "芬兰文",
        "no" | "norwegian" => "挪威文",
        "cs" | "czech" => "捷克文",
        "ro" | "romanian" => "罗马尼亚文",
        "hu" | "hungarian" => "匈牙利文",
        "el" | "greek" => "希腊文",
        "he" | "hebrew" => "希伯来文",
        "uk" | "ukrainian" => "乌克兰文",
        "bg" | "bulgarian" => "保加利亚文",
        "fa" | "persian" => "波斯文",
        "ur" | "urdu" => "乌尔都文",
        "sr" | "serbian" => "塞尔维亚文",
        "hr" | "croatian" => "克罗地亚文",
        "sk" | "slovak" => "斯洛伐克文",
        "sl" | "slovenian" => "斯洛文尼亚文",
        "lt" | "lithuanian" => "立陶宛文",
        "lv" | "latvian" => "拉脱维亚文",
        "et" | "estonian" => "爱沙尼亚文",
        "ca" | "catalan" => "加泰罗尼亚文",
        "fil" | "tl" | "filipino" => "菲律宾文",
        "ne" | "nepali" => "尼泊尔文",
        "km" | "khmer" => "高棉文",
        "lo" | "lao" => "老挝文",
        "my" | "burmese" => "缅甸文",
        "si" | "sinhala" => "僧伽罗文",
        "ta" | "tamil" => "泰米尔文",
        "te" | "telugu" => "泰卢固文",
        "bn" | "bengali" => "孟加拉文",
        "gu" | "gujarati" => "古吉拉特文",
        "kn" | "kannada" => "卡纳达文",
        "ml" | "malayalam" => "马拉雅拉姆文",
        "mr" | "marathi" => "马拉地文",
        "pa" | "punjabi" => "旁遮普文",
        "sw" | "swahili" => "斯瓦希里文",
        "yue" | "cantonese" => "粤语",
        _ => return normalized.to_string(),
    };
    mapped.to_string()
}

/// 目标语言使用的文字脚本（用于判断译文是否真的用目标语言书写）。
///
/// 只用于 [`looks_untranslated`] 的「输出与原文完全相同」判定，避免把
/// 「源语言 == 目标语言原样输出」误判为未翻译。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetScript {
    /// 汉字（简繁中文、粤语）。
    Han,
    /// 日文：假名或汉字。
    HanKana,
    /// 韩文：Hangul。
    Hangul,
    /// 拉丁字母（英/法/德/西/葡/意/荷…）。
    Latin,
    /// 西里尔字母（俄/乌/保/塞）。
    Cyrillic,
    /// 希腊字母。
    Greek,
    /// 希伯来字母。
    Hebrew,
    /// 阿拉伯字母（阿/波斯/乌尔都）。
    Arabic,
    /// 泰文。
    Thai,
    /// 天城文（印地/马拉地/尼泊尔）。
    Devanagari,
    /// 孟加拉文。
    Bengali,
    /// 古木基文（旁遮普）。
    Gurmukhi,
    /// 古吉拉特文。
    Gujarati,
    /// 泰米尔文。
    Tamil,
    /// 泰卢固文。
    Telugu,
    /// 卡纳达文。
    Kannada,
    /// 马拉雅拉姆文。
    Malayalam,
    /// 僧伽罗文。
    Sinhala,
    /// 高棉文。
    Khmer,
    /// 老挝文。
    Lao,
    /// 缅甸文。
    Myanmar,
}

/// 目标语言对应的文字脚本；未知语言码返回 `None`（未知一律不判，避免误伤）。
fn target_script(target_lang: &str) -> Option<TargetScript> {
    let canonical = prompt_language_name(target_lang);
    match canonical.as_str() {
        "简体中文" | "繁体中文" | "粤语" | "中文" => Some(TargetScript::Han),
        "日文" => Some(TargetScript::HanKana),
        "韩文" => Some(TargetScript::Hangul),
        "英文" | "法文" | "德文" | "西班牙文" | "葡萄牙文" | "意大利文" | "荷兰文" | "土耳其文"
        | "波兰文" | "瑞典文" | "丹麦文" | "芬兰文" | "挪威文" | "捷克文" | "罗马尼亚文"
        | "匈牙利文" | "斯洛伐克文" | "斯洛文尼亚文" | "克罗地亚文" | "立陶宛文" | "拉脱维亚文"
        | "爱沙尼亚文" | "加泰罗尼亚文" | "菲律宾文" | "印度尼西亚文" | "马来文" | "越南文"
        | "斯瓦希里文" => Some(TargetScript::Latin),
        "俄文" | "乌克兰文" | "保加利亚文" | "塞尔维亚文" => {
            Some(TargetScript::Cyrillic)
        }
        "希腊文" => Some(TargetScript::Greek),
        "希伯来文" => Some(TargetScript::Hebrew),
        "阿拉伯文" | "波斯文" | "乌尔都文" => Some(TargetScript::Arabic),
        "泰文" => Some(TargetScript::Thai),
        "印地文" | "马拉地文" | "尼泊尔文" => Some(TargetScript::Devanagari),
        "孟加拉文" => Some(TargetScript::Bengali),
        "旁遮普文" => Some(TargetScript::Gurmukhi),
        "古吉拉特文" => Some(TargetScript::Gujarati),
        "泰米尔文" => Some(TargetScript::Tamil),
        "泰卢固文" => Some(TargetScript::Telugu),
        "卡纳达文" => Some(TargetScript::Kannada),
        "马拉雅拉姆文" => Some(TargetScript::Malayalam),
        "僧伽罗文" => Some(TargetScript::Sinhala),
        "高棉文" => Some(TargetScript::Khmer),
        "老挝文" => Some(TargetScript::Lao),
        "缅甸文" => Some(TargetScript::Myanmar),
        _ => None,
    }
}

/// 汉字码点（基本区 + 扩展 A/B… + 兼容区）。
fn is_han(codepoint: u32) -> bool {
    (0x4E00..=0x9FFF).contains(&codepoint)
        || (0x3400..=0x4DBF).contains(&codepoint)
        || (0xF900..=0xFAFF).contains(&codepoint)
        || (0x20000..=0x2FA1F).contains(&codepoint)
}

/// 该字符是否属于目标脚本。
fn script_contains(script: TargetScript, ch: char) -> bool {
    let cp = ch as u32;
    match script {
        TargetScript::Han => is_han(cp),
        TargetScript::HanKana => {
            is_han(cp)
                || (0x3040..=0x30FF).contains(&cp)
                || (0x31F0..=0x31FF).contains(&cp)
                || (0xFF66..=0xFF9F).contains(&cp)
        }
        TargetScript::Hangul => {
            (0x1100..=0x11FF).contains(&cp)
                || (0x3130..=0x318F).contains(&cp)
                || (0xA960..=0xA97F).contains(&cp)
                || (0xAC00..=0xD7AF).contains(&cp)
        }
        TargetScript::Latin => {
            ch.is_ascii_alphabetic()
                || (0x00C0..=0x024F).contains(&cp)
                || (0x1E00..=0x1EFF).contains(&cp)
                || (0x2C60..=0x2C7F).contains(&cp)
        }
        TargetScript::Cyrillic => (0x0400..=0x052F).contains(&cp),
        TargetScript::Greek => (0x0370..=0x03FF).contains(&cp) || (0x1F00..=0x1FFF).contains(&cp),
        TargetScript::Hebrew => (0x0590..=0x05FF).contains(&cp),
        TargetScript::Arabic => {
            (0x0600..=0x06FF).contains(&cp)
                || (0x0750..=0x077F).contains(&cp)
                || (0xFB50..=0xFDFF).contains(&cp)
                || (0xFE70..=0xFEFF).contains(&cp)
        }
        TargetScript::Thai => (0x0E00..=0x0E7F).contains(&cp),
        TargetScript::Devanagari => (0x0900..=0x097F).contains(&cp),
        TargetScript::Bengali => (0x0980..=0x09FF).contains(&cp),
        TargetScript::Gurmukhi => (0x0A00..=0x0A7F).contains(&cp),
        TargetScript::Gujarati => (0x0A80..=0x0AFF).contains(&cp),
        TargetScript::Tamil => (0x0B80..=0x0BFF).contains(&cp),
        TargetScript::Telugu => (0x0C00..=0x0C7F).contains(&cp),
        TargetScript::Kannada => (0x0C80..=0x0CFF).contains(&cp),
        TargetScript::Malayalam => (0x0D00..=0x0D7F).contains(&cp),
        TargetScript::Sinhala => (0x0D80..=0x0DFF).contains(&cp),
        TargetScript::Khmer => (0x1780..=0x17FF).contains(&cp),
        TargetScript::Lao => (0x0E80..=0x0EFF).contains(&cp),
        TargetScript::Myanmar => (0x1000..=0x109F).contains(&cp),
    }
}

/// 去除译文常见的包裹：首尾空白、`译文：`/`翻译：`/`Translation:` 前缀、成对引号/反引号。
fn strip_translation_wrappers(text: &str) -> String {
    let mut current = text.trim().to_string();
    for prefix in [
        "译文：",
        "译文:",
        "翻译：",
        "翻译:",
        "Translation:",
        "Translation：",
        "Translated:",
        "Translated text:",
    ] {
        if let Some(rest) = current.strip_prefix(prefix) {
            current = rest.trim().to_string();
        }
    }
    loop {
        let trimmed = current.trim();
        let chars: Vec<char> = trimmed.chars().collect();
        if chars.len() < 2 {
            break;
        }
        let first = chars[0];
        let last = chars[chars.len() - 1];
        let is_pair = matches!(
            (first, last),
            ('"', '"')
                | ('\'', '\'')
                | ('`', '`')
                | ('“', '”')
                | ('‘', '’')
                | ('「', '」')
                | ('『', '』')
                | ('【', '】')
                | ('（', '）')
        );
        if !is_pair {
            break;
        }
        let inner: String = chars[1..chars.len() - 1].iter().collect();
        let trimmed_inner = inner.trim().to_string();
        if trimmed_inner.is_empty() {
            break;
        }
        current = trimmed_inner;
    }
    current
}

/// 归一化空白（合并连续空白 + 去首尾），用于「输出与原文是否完全相同」的严格比较。
fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 判断翻译结果是否**原样复述了原文**（本地小模型未执行翻译）。
///
/// 判定刻意保守（避免误伤合法输出）：
/// 1. 未知目标语言码（含 `auto`）一律不判；
/// 2. 先去掉输出首尾空白、`译文：` 类前缀与包裹引号/反引号，再归一化空白；
/// 3. 仅当归一化后**与原文完全相同**、且输出中**不含任何目标语言脚本字符**时才判为未翻译；
/// 4. 因此「源语言 == 目标语言」的合法原样输出（如中文→中文）不会被误判。
pub fn looks_untranslated(source: &str, output: &str, target_lang: &str) -> bool {
    let Some(script) = target_script(target_lang) else {
        return false;
    };
    let source_normalized = normalize_whitespace(source);
    if source_normalized.is_empty() {
        return false;
    }
    let output_normalized = normalize_whitespace(&strip_translation_wrappers(output));
    if output_normalized.is_empty() || output_normalized != source_normalized {
        return false;
    }
    // 输出与原文完全一致：目标脚本完全缺席 ⇒ 模型只是把原文吐了回来。
    !output_normalized
        .chars()
        .any(|ch| script_contains(script, ch))
}

/// 本地翻译「返回原文（未翻译）」的可操作错误（SA-1099）。
///
/// 供 `translate_with_fallback` 与 `task_runner` 共用：命中复述检测时记失败并继续降级链。
pub fn untranslated_output_error(target_lang: &str) -> AppError {
    AppError::invalid_state(format!(
        "本地翻译模型未执行翻译：输出与原文完全相同（目标语言 {}）",
        prompt_language_name(target_lang)
    ))
    .with_message(
        "本地翻译模型没有翻译（返回了原文）：已自动尝试云端降级；如仍失败，请在设置中更换本地模型或改用云端翻译",
    )
    .with_fallback(vec!["retry", "open_settings"])
}

/// 截断到 `limit` 字符（UTF-8 安全）。
fn truncate_chars(text: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    text.chars().take(limit).collect()
}

/// DashScope Qwen-MT 原生翻译器（契约 v5 §3.4）。
#[derive(Debug, Clone)]
pub struct QwenMtTranslator {
    /// Base URL（`https://dashscope.aliyuncs.com`）。
    pub base_url: String,
    /// DashScope API Key。
    pub api_key: String,
    /// 模型名（`qwen-mt-lite` / `qwen-mt-flash` / `qwen-mt-plus`）。
    pub model: String,
    /// 领域提示（可选，英文；写入 `translation_options.domains`）。
    pub domains: Option<String>,
}

impl QwenMtTranslator {
    /// 新建（无领域提示）。
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            domains: None,
        }
    }

    /// 覆盖领域提示。
    pub fn with_domains(mut self, domains: impl Into<String>) -> Self {
        self.domains = Some(domains.into());
        self
    }

    /// 原生生成端点。
    pub fn generation_url(&self) -> String {
        qwen_mt_generation_url(&self.base_url)
    }

    /// HTTP 客户端。
    pub fn client(&self) -> AppResult<reqwest::Client> {
        crate::platform::http::client_builder_for(&self.base_url)
            .user_agent(concat!("snip-ask/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(QWEN_MT_CONNECT_TIMEOUT)
            .timeout(QWEN_MT_REQUEST_TIMEOUT)
            .build()
            .map_err(|err| AppError::io_failed(format!("创建 Qwen-MT HTTP 客户端失败：{err}")))
    }

    /// 渲染原生请求体（纯函数，便于单测与协议证据）。
    pub fn request_body(&self, request: &TranslateRequest) -> Value {
        let mut options = json!({
            "source_lang": qwen_mt_language_name(request.source_lang_or_auto()),
            "target_lang": qwen_mt_language_name(&request.target_lang),
        });
        let terms: Vec<Value> = request
            .glossary
            .iter()
            .filter(|entry| !entry.from.trim().is_empty() && !entry.to.trim().is_empty())
            .map(|entry| {
                json!({
                    "source": entry.from.trim(),
                    "target": entry.to.trim(),
                })
            })
            .collect();
        if !terms.is_empty() {
            options["terms"] = Value::Array(terms);
        }
        if let Some(domains) = self
            .domains
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            options["domains"] = Value::String(domains.to_string());
        }
        json!({
            "model": self.model,
            "input": {
                "messages": [{ "role": "user", "content": request.text }],
            },
            "parameters": {
                "translation_options": options,
                // 让原生端点按 OpenAI 风格返回 output.choices；不支持时上游忽略，
                // 解析器同时兼容 output.text。
                "result_format": "message",
            },
        })
    }

    /// 解析原生响应：返回 `(译文, input_tokens, output_tokens)`。
    pub fn parse_response(body: &str) -> AppResult<(String, u64, u64)> {
        let value: Value = serde_json::from_str(body).map_err(|err| {
            AppError::new(
                ErrorCode::InvalidState,
                Some(format!(
                    "Qwen-MT 响应不是合法 JSON：{err}；响应片段：{}",
                    truncate_chars(body, 200)
                )),
            )
        })?;
        // 业务错误：DashScope 原生可能以 HTTP 200 + status_code/code 返回。
        let status_code = value
            .get("status_code")
            .and_then(|item| {
                item.as_u64()
                    .or_else(|| item.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(200);
        let code = value
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if status_code != 200 || !code.trim().is_empty() {
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("未知错误")
                .to_string();
            return Err(map_qwen_mt_error_code(code, status_code as u16, &message));
        }
        let output = value.get("output").cloned().unwrap_or(Value::Null);
        let content = output
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .or_else(|| output.get("text").and_then(Value::as_str))
            .unwrap_or_default()
            .trim()
            .to_string();
        if content.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidState,
                Some(format!(
                    "Qwen-MT 未返回译文：{}",
                    truncate_chars(body, ERROR_BODY_LIMIT)
                )),
            ));
        }
        let usage = value.get("usage").cloned().unwrap_or(Value::Null);
        let input_tokens = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .or_else(|| usage.get("prompt_tokens").and_then(Value::as_u64))
            .unwrap_or(0);
        let output_tokens = usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .or_else(|| usage.get("completion_tokens").and_then(Value::as_u64))
            .unwrap_or(0);
        Ok((content, input_tokens, output_tokens))
    }

    /// HTTP 状态码 → 契约错误码。
    pub fn map_http_error(status: u16, body: &str) -> AppError {
        let detail = truncate_chars(body.trim(), ERROR_BODY_LIMIT);
        let with_status =
            |message: &str| format!("Qwen-MT 返回 HTTP {status}（{message}）：{detail}");
        match status {
            401 | 403 => AppError::new(
                ErrorCode::AuthFailed,
                Some(with_status("DashScope Key 无效")),
            )
            .with_message("DashScope API Key 无效或没有权限：请检查设置中的 DashScope Key")
            .with_fallback(vec!["open_settings"]),
            402 => AppError::new(ErrorCode::QuotaExceeded, Some(with_status("额度不足")))
                .with_message("DashScope 账户额度不足：请充值或换用其他翻译后端")
                .with_fallback(vec!["open_settings"]),
            408 => AppError::new(ErrorCode::Timeout, Some(with_status("上游超时")))
                .with_fallback(vec!["retry", "local_translate"]),
            429 => AppError::new(ErrorCode::RateLimited, Some(with_status("触发限流")))
                .with_message("翻译请求过于频繁：请稍后重试")
                .with_fallback(vec!["retry"]),
            _ if (500..600).contains(&status) => AppError::new(
                ErrorCode::NetworkUnavailable,
                Some(with_status("DashScope 服务端错误")),
            )
            .with_fallback(vec!["retry", "local_translate"]),
            400 | 404 | 422 => {
                AppError::new(ErrorCode::InvalidConfig, Some(with_status("请求被拒绝")))
                    .with_message(format!(
                        "Qwen-MT 请求被拒绝（HTTP {status}）：请检查模型名与语言代码"
                    ))
            }
            _ => AppError::new(ErrorCode::InvalidState, Some(with_status("未预期状态"))),
        }
    }

    /// 传输层错误 → 契约错误码。
    pub fn map_transport_error(err: &reqwest::Error) -> AppError {
        if err.is_timeout() {
            AppError::new(ErrorCode::Timeout, Some(format!("Qwen-MT 请求超时：{err}")))
                .with_message("翻译超时：可重试或改用本地翻译")
                .with_fallback(vec!["retry", "local_translate"])
        } else {
            AppError::new(
                ErrorCode::NetworkUnavailable,
                Some(format!("Qwen-MT 网络错误：{err}")),
            )
            .with_message("网络不可用：无法连接 DashScope，可改用本地翻译")
            .with_fallback(vec!["local_translate"])
        }
    }

    /// 真实调用（非流式）：返回 `(译文, TokenUsage)`。
    pub async fn translate_with_usage(
        &self,
        request: &TranslateRequest,
    ) -> AppResult<(String, TokenUsage)> {
        if request.text.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidState,
                Some("待翻译文本为空".to_string()),
            ));
        }
        let client = self.client()?;
        let payload = serde_json::to_string(&self.request_body(request)).map_err(|err| {
            AppError::invalid_config("qwen_mt_request", format!("请求体序列化失败：{err}"))
        })?;
        let response = client
            .post(self.generation_url())
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .bearer_auth(self.api_key.trim())
            .body(payload)
            .send()
            .await
            .map_err(|err| Self::map_transport_error(&err))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(Self::map_http_error(status.as_u16(), &body));
        }
        let (text, input_tokens, output_tokens) = Self::parse_response(&body)?;
        let usage = qwen_mt_usage(&self.model, input_tokens, output_tokens);
        Ok((text, usage))
    }
}

#[async_trait]
impl Translator for QwenMtTranslator {
    async fn translate(&self, request: &TranslateRequest) -> AppResult<String> {
        Ok(self.translate_with_usage(request).await?.0)
    }
}

/// 按真实 token 用量 + `pricing.json` 估算 Qwen-MT 费用（人民币）。
pub fn qwen_mt_usage(model: &str, input_tokens: u64, output_tokens: u64) -> TokenUsage {
    match crate::domain::usage::PricingTable::from_json(crate::resources::PRICING) {
        Ok(table) => table.estimate(model, input_tokens, output_tokens, 0),
        Err(_) => TokenUsage {
            input_tokens,
            output_tokens,
            cached_input_tokens: 0,
            cost_estimate: 0.0,
            currency: crate::contract::types::Currency::Cny,
        },
    }
}

/// DashScope 业务错误码/HTTP 码 → 契约错误码。
fn map_qwen_mt_error_code(code: &str, status: u16, message: &str) -> AppError {
    let lower = code.to_ascii_lowercase();
    let detail = format!("Qwen-MT 错误 {code}（HTTP {status}）：{message}");
    if lower.contains("invalidapikey")
        || lower.contains("unauthorized")
        || lower.contains("accessdenied")
        || lower.contains("authentication")
    {
        return AppError::new(ErrorCode::AuthFailed, Some(detail))
            .with_message("DashScope API Key 无效或没有权限：请检查设置中的 DashScope Key")
            .with_fallback(vec!["open_settings"]);
    }
    if lower.contains("arrearage") || lower.contains("quota") || lower.contains("balance") {
        return AppError::new(ErrorCode::QuotaExceeded, Some(detail))
            .with_message("DashScope 账户额度不足：请充值或换用其他翻译后端")
            .with_fallback(vec!["open_settings"]);
    }
    if lower.contains("throttling") || lower.contains("ratelimit") || status == 429 {
        return AppError::new(ErrorCode::RateLimited, Some(detail))
            .with_message("翻译请求过于频繁：请稍后重试")
            .with_fallback(vec!["retry"]);
    }
    if lower.contains("modelnotfound") || lower.contains("model_not") {
        return AppError::new(ErrorCode::ModelMissing, Some(detail))
            .with_fallback(vec!["download_model", "cloud_translate"]);
    }
    if lower.contains("invalidparameter")
        || lower.contains("invalid_value")
        || lower.contains("badrequest")
        || (400..500).contains(&status)
    {
        return AppError::new(ErrorCode::InvalidConfig, Some(detail)).with_message(format!(
            "Qwen-MT 请求参数被拒绝（{code}）：请检查模型名与语言代码"
        ));
    }
    AppError::new(ErrorCode::InvalidState, Some(detail))
}

/// 通用 LLM 翻译器：OpenAI 兼容网关 + 渲染好的翻译提示词（契约 v5 §3.4）。
pub struct GatewayTranslator {
    /// 实际引擎标签（供应商 id / `local`）。
    engine: String,
    /// 网关（云端 SSE / 本地 llama-server）。
    gateway: std::sync::Arc<dyn crate::services::llm_gateway::LlmGateway>,
    /// 已渲染的翻译提示词（`translate.md` 或 `translate_minimal.md`）。
    prompt: String,
    /// 模式。
    mode: Mode,
}

impl GatewayTranslator {
    /// 新建。
    pub fn new(
        engine: impl Into<String>,
        gateway: std::sync::Arc<dyn crate::services::llm_gateway::LlmGateway>,
        prompt: impl Into<String>,
        mode: Mode,
    ) -> Self {
        Self {
            engine: engine.into(),
            gateway,
            prompt: prompt.into(),
            mode,
        }
    }

    /// 引擎标签。
    pub fn engine(&self) -> &str {
        &self.engine
    }

    /// 流式翻译：把文本增量直接交给 `sink`，返回网关结算结果。
    pub async fn translate_stream(
        &self,
        request: &TranslateRequest,
        sink: &(dyn Fn(crate::services::llm_gateway::StreamChunk) + Send + Sync),
    ) -> AppResult<crate::services::llm_gateway::TaskOutcome> {
        if request.text.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidState,
                Some("待翻译文本为空".to_string()),
            ));
        }
        let llm_request = crate::services::llm_gateway::LlmRequest {
            // 翻译任务使用独立 task/session 占位：网关只关心 engine/prompt/mode。
            task: crate::contract::types::TaskId::from("translate"),
            session: crate::contract::types::SessionId::from("translate"),
            action: crate::contract::types::ActionId::Translate,
            mode: self.mode,
            engine: self.engine.clone(),
            prompt: self.prompt.clone(),
            input_text: request.text.clone(),
            image_data_url: None,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        self.gateway.stream(&llm_request, sink).await
    }
}

#[async_trait]
impl Translator for GatewayTranslator {
    async fn translate(&self, request: &TranslateRequest) -> AppResult<String> {
        let collected = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
        let buffer = collected.clone();
        let sink = move |chunk: crate::services::llm_gateway::StreamChunk| {
            if chunk.kind == crate::contract::types::StreamKind::Text {
                buffer.lock().push_str(&chunk.chunk);
            }
        };
        let outcome = self.translate_stream(request, &sink).await?;
        let text = collected.lock().trim().to_string();
        if text.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidState,
                Some(format!(
                    "翻译后端未返回内容（engine={}，耗时 {}ms）",
                    outcome.engine, outcome.latency_ms
                )),
            ));
        }
        Ok(text)
    }
}

/// 一次翻译尝试：引擎标签 + 后端种类。
pub enum TranslateAttemptKind {
    /// 通用 LLM（可流式）。
    Llm {
        /// 网关。
        gateway: std::sync::Arc<dyn crate::services::llm_gateway::LlmGateway>,
        /// 已渲染提示词。
        prompt: String,
    },
    /// Qwen-MT 原生端点（非流式）。
    QwenMt(QwenMtTranslator),
}

/// 一次翻译尝试。
pub struct TranslateAttempt {
    /// 引擎标签（供应商 id / `local`）。
    pub engine: String,
    /// 后端种类。
    pub kind: TranslateAttemptKind,
}

impl std::fmt::Debug for TranslateAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match &self.kind {
            TranslateAttemptKind::Llm { .. } => "llm",
            TranslateAttemptKind::QwenMt(_) => "qwenMt",
        };
        write!(f, "TranslateAttempt({}, {kind})", self.engine)
    }
}

/// 翻译结果（译文 + 实际引擎 + 用量/费用）。
#[derive(Debug, Clone)]
pub struct TranslationOutcome {
    /// 译文。
    pub text: String,
    /// 实际生效的引擎（供应商 id / `local` / `skeleton`）。
    pub engine: String,
    /// Token 用量与费用（Qwen-MT 按真实 usage；LLM 走网关结算）。
    pub usage: TokenUsage,
}

/// `translate_test` 命令实现（真实配置后端；骨架模式保持契约 §12 占位）。
pub async fn translate_test(
    state: &crate::app::state::AppState,
    app: &tauri::AppHandle,
    config: &Config,
    text: &str,
    target: &str,
) -> AppResult<String> {
    if text.trim().is_empty() {
        return Err(AppError::invalid_config("text", "待翻译文本不能为空"));
    }
    if target.trim().is_empty() {
        return Err(AppError::invalid_config("target", "目标语言不能为空"));
    }
    let request = TranslateRequest::new(text, target, config.modes.translate)
        .with_glossary(config.translate.glossary.clone());
    if state.is_skeleton() {
        let translator = SkeletonTranslator;
        return translator.translate(&request).await;
    }
    Ok(translate_with_config(state, app, config, &request)
        .await?
        .text)
}

/// 按配置执行真实翻译（主后端 → 降级链），返回译文。
pub async fn translate_with_config(
    state: &crate::app::state::AppState,
    app: &tauri::AppHandle,
    config: &Config,
    request: &TranslateRequest,
) -> AppResult<TranslationOutcome> {
    if request.text.trim().is_empty() {
        return Err(AppError::invalid_config("text", "待翻译文本不能为空"));
    }
    if request.target_lang.trim().is_empty() {
        return Err(AppError::invalid_config("target", "目标语言不能为空"));
    }
    let attempts = translate_attempts(state, app, config, request).await?;
    translate_with_fallback(&attempts, request).await
}

/// 逐个尝试翻译后端：任一成功即返回；全部失败返回最后一个可操作错误（带尝试明细）。
pub async fn translate_with_fallback(
    attempts: &[TranslateAttempt],
    request: &TranslateRequest,
) -> AppResult<TranslationOutcome> {
    if attempts.is_empty() {
        return Err(AppError::invalid_config(
            "translate",
            "没有可用的翻译后端：请在设置中启用云端供应商并填写 Key，或下载本地模型",
        ));
    }
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut last_error: Option<AppError> = None;
    for attempt in attempts {
        match &attempt.kind {
            TranslateAttemptKind::Llm { gateway, prompt } => {
                let translator = GatewayTranslator::new(
                    attempt.engine.clone(),
                    gateway.clone(),
                    prompt.clone(),
                    request.mode,
                );
                match translator.translate(request).await {
                    Ok(text) => {
                        // 本地小模型可能把原文原样复述（v1.0.3 用户反馈的根因）：
                        // 仅对 engine=="local" 的 LLM 结果做复述检测，命中则记失败并继续降级链。
                        if attempt.engine == "local"
                            && looks_untranslated(&request.text, &text, &request.target_lang)
                        {
                            let err = untranslated_output_error(&request.target_lang);
                            tracing::warn!(
                                engine = %attempt.engine,
                                code = %err.code,
                                "本地翻译返回原文（未翻译），判定失败并继续降级链"
                            );
                            failures.push((attempt.engine.clone(), err.code.clone()));
                            last_error = Some(err);
                            continue;
                        }
                        // LLM 网关的 token 用量在 translate_stream 的 outcome 里；
                        // 这里只拿到正文，用量由调用方（task_runner）单独结算。
                        return Ok(TranslationOutcome {
                            text,
                            engine: attempt.engine.clone(),
                            usage: TokenUsage {
                                input_tokens: 0,
                                output_tokens: 0,
                                cached_input_tokens: 0,
                                cost_estimate: 0.0,
                                currency: crate::contract::types::Currency::Cny,
                            },
                        });
                    }
                    Err(err) => {
                        tracing::warn!(engine = %attempt.engine, code = %err.code, "翻译后端失败，尝试下一个");
                        failures.push((attempt.engine.clone(), err.code.clone()));
                        last_error = Some(err);
                    }
                }
            }
            TranslateAttemptKind::QwenMt(translator) => {
                match translator.translate_with_usage(request).await {
                    Ok((text, usage)) => {
                        return Ok(TranslationOutcome {
                            text,
                            engine: attempt.engine.clone(),
                            usage,
                        });
                    }
                    Err(err) => {
                        tracing::warn!(engine = %attempt.engine, code = %err.code, "Qwen-MT 翻译失败，尝试下一个");
                        failures.push((attempt.engine.clone(), err.code.clone()));
                        last_error = Some(err);
                    }
                }
            }
        }
    }
    let mut error =
        last_error.unwrap_or_else(|| AppError::invalid_config("translate", "没有可用翻译后端"));
    let chain = failures
        .iter()
        .map(|(engine, code)| format!("{engine}:{code}"))
        .collect::<Vec<_>>()
        .join(", ");
    error.message = format!("全部翻译后端均失败（已尝试：{chain}）");
    Err(error.with_context(json!({ "attempts": failures })))
}

/// 解析本次配置下的翻译尝试顺序（不含网络请求，仅组装；本地引擎会在最后按需拉起）。
pub async fn translate_attempts(
    state: &crate::app::state::AppState,
    app: &tauri::AppHandle,
    config: &Config,
    request: &TranslateRequest,
) -> AppResult<Vec<TranslateAttempt>> {
    let local_only = config.privacy.local_only;
    let cloud_allowed = !local_only;
    let prompt_engine = state.prompt_engine();
    let local_first = config.translate.backend == TranslateBackend::Local;
    let mut attempts: Vec<TranslateAttempt> = Vec::new();
    let mut missing_keys: Vec<String> = Vec::new();

    if local_first {
        // 本地后端：确保引擎就绪（Vulkan → CUDA → CPU 回退链由 LocalEngine 负责）。
        match state.local_engine.ensure_ready(app, config).await {
            Ok(_) => {
                if let Some(attempt) = build_local_attempt(state, config, &prompt_engine, request) {
                    attempts.push(attempt);
                }
            }
            Err(err) => {
                tracing::warn!(code = %err.code, "本地翻译引擎不可用，尝试云端降级");
            }
        }
        if cloud_allowed {
            build_cloud_attempts(
                state,
                config,
                &prompt_engine,
                request,
                &mut attempts,
                &mut missing_keys,
            )
            .await;
        }
    } else {
        if cloud_allowed {
            build_cloud_attempts(
                state,
                config,
                &prompt_engine,
                request,
                &mut attempts,
                &mut missing_keys,
            )
            .await;
        }
        // 云端失败 → 本地（契约 §7.2 降级）；本地引擎按需拉起。
        if !local_only {
            match state.local_engine.ensure_ready(app, config).await {
                Ok(_) => {
                    if let Some(attempt) =
                        build_local_attempt(state, config, &prompt_engine, request)
                    {
                        attempts.push(attempt);
                    }
                }
                Err(err) => {
                    tracing::warn!(code = %err.code, "本地翻译引擎不可用，跳过本地降级");
                }
            }
        }
    }

    if attempts.is_empty() {
        if !missing_keys.is_empty() {
            return Err(AppError::new(
                ErrorCode::AuthFailed,
                Some(format!(
                    "以下供应商已启用但没有 API Key：{}；本地模型也不可用",
                    missing_keys.join("、")
                )),
            )
            .with_message("尚无可用翻译后端：请在设置中填写 API Key，或在引导页下载本地翻译模型")
            .with_fallback(vec!["open_settings", "download_model"]));
        }
        return Err(AppError::invalid_config(
            "translate",
            if local_only {
                "隐私设置仅本地模式，但本地翻译模型未就绪：请先下载并启用本地模型"
            } else {
                "没有可用的翻译后端：请启用云端供应商并填写 Key，或下载本地模型"
            },
        ));
    }
    Ok(attempts)
}

/// 组装云端尝试（按 `cloud_provider_order`；无 Key 的供应商记入 `missing_keys` 后跳过）。
async fn build_cloud_attempts(
    state: &crate::app::state::AppState,
    config: &Config,
    prompt_engine: &crate::services::prompt_engine::PromptEngine,
    request: &TranslateRequest,
    attempts: &mut Vec<TranslateAttempt>,
    missing_keys: &mut Vec<String>,
) {
    for provider in cloud_provider_order(config) {
        let kind_is_qwen = provider_is_qwen_mt(provider.kind, &provider.base_url);
        let prompt = match render_translate_prompt(
            prompt_engine,
            config,
            "translate",
            request,
            request.target_lang.as_str(),
            false,
        ) {
            Ok(prompt) => prompt,
            Err(err) => {
                tracing::warn!(code = %err.code, "翻译提示词渲染失败，使用最小提示词兜底");
                minimal_translate_prompt(&request.text, &request.target_lang)
            }
        };
        let key = match state.credential_service().provider_key(&provider.id).await {
            Ok(key) => key,
            Err(err) => {
                tracing::warn!(provider = %provider.id, code = %err.code, "读取供应商密钥失败，跳过该后端");
                None
            }
        };
        let Some(key) = key.filter(|key| !key.trim().is_empty()) else {
            missing_keys.push(provider.id.clone());
            continue;
        };
        if kind_is_qwen {
            attempts.push(TranslateAttempt {
                engine: provider.id.clone(),
                kind: TranslateAttemptKind::QwenMt(QwenMtTranslator::new(
                    provider.base_url.clone(),
                    key,
                    provider.model.clone(),
                )),
            });
        } else {
            let base_url = format!("{}/v1", provider.base_url.trim_end_matches('/'));
            let gateway = std::sync::Arc::new(crate::services::local_gateway::HttpLlmGateway::new(
                base_url,
                Some(key),
                provider.model.clone(),
            ));
            attempts.push(TranslateAttempt {
                engine: provider.id.clone(),
                kind: TranslateAttemptKind::Llm { gateway, prompt },
            });
        }
    }
}

/// 组装本地尝试（本地引擎必须已就绪；未就绪返回 `None`）。
fn build_local_attempt(
    state: &crate::app::state::AppState,
    config: &Config,
    prompt_engine: &crate::services::prompt_engine::PromptEngine,
    request: &TranslateRequest,
) -> Option<TranslateAttempt> {
    let base_url = state.local_engine.base_url()?;
    // 本地精简模型：术语表非空走 `translate_minimal_glossary`，否则 `translate_minimal`。
    let prompt_key =
        crate::services::action_router::translate_prompt_key_for(config, &request.glossary);
    let prompt = match render_translate_prompt(
        prompt_engine,
        config,
        prompt_key,
        request,
        request.target_lang.as_str(),
        true,
    ) {
        Ok(prompt) => prompt,
        Err(_) => minimal_translate_prompt(&request.text, &request.target_lang),
    };
    let gateway = std::sync::Arc::new(crate::services::local_gateway::HttpLlmGateway::new(
        base_url,
        None,
        config.local.model.clone(),
    ));
    Some(TranslateAttempt {
        engine: "local".to_string(),
        kind: TranslateAttemptKind::Llm { gateway, prompt },
    })
}

/// 云端 provider 的尝试顺序（v1.1.0 起按 provider caps 过滤）。
///
/// `translate.providerId` 优先（但必须已启用且具备翻译能力），其余具备翻译能力
/// （`translate` cap，或 `kind = qwenMt`）的启用供应商按配置顺序。
/// 无翻译能力的供应商**不会**进入云端翻译尝试链——动作可用性同样按该规则置灰。
pub fn cloud_provider_order(config: &Config) -> Vec<&crate::contract::config::Provider> {
    use crate::services::action_router::can_translate;
    let mut ordered: Vec<&crate::contract::config::Provider> = Vec::new();
    if let Some(primary_id) = config.translate.provider_id.as_deref() {
        if let Some(provider) = config.providers.iter().find(|provider| {
            provider.id == primary_id && provider.enabled && can_translate(provider)
        }) {
            ordered.push(provider);
        }
    }
    for provider in config
        .providers
        .iter()
        .filter(|provider| provider.enabled && can_translate(provider))
    {
        if ordered.iter().any(|item| item.id == provider.id) {
            continue;
        }
        ordered.push(provider);
    }
    ordered
}

/// 渲染翻译提示词（`translate.md` / `translate_minimal*.md`，含 `{glossary}`）。
///
/// 语言名策略（v1.0.4 真机实测）：
/// - **本地精简提示词**：`{targetLang}`/`{sourceLang}` 渲染为中文语言名（如 `简体中文`），
///   因为 0.5B/1.5B/HY-MT-1.8B 不认识 `zh-CHS` 这类 BCP-47 码，会原样复述原文；
/// - **云端 `translate.md`**：保持 BCP-47 码不变（云端模型已验证正常，契约里也约定了语言码）。
pub fn render_translate_prompt(
    engine: &crate::services::prompt_engine::PromptEngine,
    config: &Config,
    prompt_key: &str,
    request: &TranslateRequest,
    target_lang: &str,
    is_local: bool,
) -> AppResult<String> {
    let lang = match config.app.language {
        crate::contract::config::Language::ZhCn => "zh-CN",
        crate::contract::config::Language::EnUs => "en-US",
    };
    let glossary = effective_glossary(config, &request.glossary);
    // 只有「本地 + 精简提示词」需要把语言码换成中文语言名；其余（云端、本地 4B 级模型）保持原样。
    let local_minimal = is_local && prompt_key.starts_with("translate_minimal");
    let target_lang_value = if local_minimal {
        prompt_language_name(target_lang)
    } else {
        target_lang.to_string()
    };
    let source_lang_value = if local_minimal {
        prompt_language_name(request.source_lang_or_auto())
    } else {
        request.source_lang_or_auto().to_string()
    };
    let mut vars: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    vars.insert("text", request.text.as_str());
    vars.insert("lang", lang);
    vars.insert("mode", request.mode.as_str());
    vars.insert("targetLang", target_lang_value.as_str());
    vars.insert("sourceLang", source_lang_value.as_str());
    vars.insert("glossary", glossary.as_str());
    vars.insert("question", "");
    vars.insert("context", "");
    engine.render(prompt_key, &vars)
}

/// 本请求实际生效的术语表文本（`{glossary}` 变量值；空串表示无有效术语）。
///
/// 两处来源都判断：请求级 `request_glossary` 非空时优先（与原有渲染口径一致），
/// 否则使用配置级 `config.translate.glossary`。条目在渲染侧过滤（空原文/空译文丢弃）。
pub fn effective_glossary(config: &Config, request_glossary: &[GlossaryEntry]) -> String {
    if request_glossary.is_empty() {
        render_glossary(config)
    } else {
        render_glossary_entries(request_glossary)
    }
}

/// 渲染术语表条目（`原文 => 译文; ...`）。
pub fn render_glossary_entries(entries: &[GlossaryEntry]) -> String {
    entries
        .iter()
        .filter(|entry| !entry.from.trim().is_empty() && !entry.to.trim().is_empty())
        .map(|entry| format!("{} => {}", entry.from.trim(), entry.to.trim()))
        .collect::<Vec<_>>()
        .join("; ")
}

/// 最小可用翻译提示词（提示词模板渲染失败时的兜底，与 `translate_minimal.md` 语义一致）。
///
/// 目标语言同样映射为中文语言名（本地小模型不认 BCP-47 码）。
pub fn minimal_translate_prompt(text: &str, target_lang: &str) -> String {
    format!(
        "把下面的文本翻译成{}，只输出译文：\n\n{text}",
        prompt_language_name(target_lang)
    )
}

/// 翻译提示词中的术语表渲染（`{glossary}` 变量）。
pub fn render_glossary(config: &Config) -> String {
    config
        .translate
        .glossary
        .iter()
        .filter(|entry| !entry.from.is_empty())
        .map(|entry| format!("{} => {}", entry.from, entry.to))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::config::Config;

    #[tokio::test]
    async fn skeleton_translation_prefixes_original_text() {
        let translator = SkeletonTranslator;
        let request = TranslateRequest {
            text: "hello".to_string(),
            source_lang: Some("en".to_string()),
            target_lang: "zh-CHS".to_string(),
            mode: Mode::Quick,
            glossary: Vec::new(),
        };
        assert_eq!(
            translator.translate(&request).await.unwrap(),
            "【骨架模式译文】hello"
        );
    }

    #[tokio::test]
    async fn empty_text_is_rejected() {
        let translator = SkeletonTranslator;
        let request = TranslateRequest {
            text: "  ".to_string(),
            source_lang: None,
            target_lang: "zh-CHS".to_string(),
            mode: Mode::Quick,
            glossary: Vec::new(),
        };
        assert!(translator.translate(&request).await.is_err());
    }

    #[test]
    fn translate_request_helpers() {
        let request = TranslateRequest::new("hello", "zh-CHS", Mode::Quick);
        assert_eq!(request.source_lang_or_auto(), "auto");
        assert!(request.glossary.is_empty());

        let mut request = request;
        request.source_lang = Some("  ".to_string());
        assert_eq!(request.source_lang_or_auto(), "auto");

        let glossary = vec![GlossaryEntry {
            from: "API".to_string(),
            to: "接口".to_string(),
            extra: serde_json::Map::new(),
        }];
        let request = TranslateRequest::new("hello", "zh-CHS", Mode::Quick).with_glossary(glossary);
        assert_eq!(render_glossary_entries(&request.glossary), "API => 接口");
    }

    #[test]
    fn qwen_mt_language_names() {
        assert_eq!(qwen_mt_language_name("auto"), "auto");
        assert_eq!(qwen_mt_language_name("zh-CHS"), "Chinese");
        assert_eq!(qwen_mt_language_name("zh_CN"), "Chinese");
        assert_eq!(qwen_mt_language_name("zh-CHT"), "Traditional Chinese");
        assert_eq!(qwen_mt_language_name("en"), "English");
        assert_eq!(qwen_mt_language_name("ja"), "Japanese");
        assert_eq!(qwen_mt_language_name("ko"), "Korean");
        assert_eq!(qwen_mt_language_name("fr"), "French");
        assert_eq!(qwen_mt_language_name("de"), "German");
        // 未知代码原样透传（不静默猜语言）。
        assert_eq!(qwen_mt_language_name("xx-YY"), "xx-YY");
        assert_eq!(qwen_mt_language_name(""), "auto");
    }

    #[test]
    fn qwen_mt_endpoint_and_kind_detection() {
        assert_eq!(
            qwen_mt_generation_url("https://dashscope.aliyuncs.com"),
            "https://dashscope.aliyuncs.com/api/v1/services/aigc/text-generation/generation"
        );
        assert_eq!(
            qwen_mt_generation_url("https://dashscope.aliyuncs.com/"),
            "https://dashscope.aliyuncs.com/api/v1/services/aigc/text-generation/generation"
        );
        assert_eq!(
            qwen_mt_generation_url("https://dashscope.aliyuncs.com/api/v1/"),
            "https://dashscope.aliyuncs.com/api/v1/services/aigc/text-generation/generation"
        );
        assert!(is_qwen_mt_base_url("https://dashscope.aliyuncs.com"));
        assert!(is_qwen_mt_base_url("https://dashscope-intl.aliyuncs.com"));
        assert!(!is_qwen_mt_base_url("https://api.deepseek.com"));
        assert!(!is_qwen_mt_base_url("not a url"));
        assert!(provider_is_qwen_mt(
            ProviderKind::QwenMt,
            "https://api.deepseek.com"
        ));
        assert!(provider_is_qwen_mt(
            ProviderKind::OpenAiCompatible,
            "https://dashscope.aliyuncs.com"
        ));
        assert!(!provider_is_qwen_mt(
            ProviderKind::OpenAiCompatible,
            "https://api.deepseek.com"
        ));
    }

    #[test]
    fn qwen_mt_request_body_has_native_translation_options() {
        let translator =
            QwenMtTranslator::new("https://dashscope.aliyuncs.com", "sk-test", "qwen-mt-lite");
        let request = TranslateRequest::new("我看到这个视频后没有笑", "English", Mode::Quick)
            .with_glossary(vec![
                GlossaryEntry {
                    from: "视频".to_string(),
                    to: "video".to_string(),
                    extra: serde_json::Map::new(),
                },
                GlossaryEntry {
                    from: "  ".to_string(),
                    to: "ignored".to_string(),
                    extra: serde_json::Map::new(),
                },
            ]);
        let body = translator.request_body(&request);
        assert_eq!(body["model"], "qwen-mt-lite");
        assert_eq!(body["input"]["messages"][0]["role"], "user");
        assert_eq!(
            body["input"]["messages"][0]["content"],
            "我看到这个视频后没有笑"
        );
        let options = &body["parameters"]["translation_options"];
        assert_eq!(options["source_lang"], "auto");
        assert_eq!(options["target_lang"], "English");
        assert_eq!(options["terms"].as_array().unwrap().len(), 1);
        assert_eq!(options["terms"][0]["source"], "视频");
        assert_eq!(options["terms"][0]["target"], "video");
        // 空术语被过滤；无 domains 时不写该字段。
        assert!(options.get("domains").is_none());
    }

    #[test]
    fn qwen_mt_response_parsing() {
        let body = r#"{
          "status_code": 200,
          "request_id": "r1",
          "output": {
            "text": null,
            "finish_reason": "stop",
            "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "I didn't laugh after watching this video."}}],
            "model_name": "qwen-mt-lite"
          },
          "usage": {"input_tokens": 12, "output_tokens": 8, "total_tokens": 20}
        }"#;
        let (text, input, output) = QwenMtTranslator::parse_response(body).unwrap();
        assert_eq!(text, "I didn't laugh after watching this video.");
        assert_eq!((input, output), (12, 8));

        // 兼容原生 output.text 形态。
        let body = r#"{"status_code":200,"output":{"text":"你好"},"usage":{"input_tokens":3,"output_tokens":2}}"#;
        let (text, input, output) = QwenMtTranslator::parse_response(body).unwrap();
        assert_eq!(text, "你好");
        assert_eq!((input, output), (3, 2));

        // 业务错误（HTTP 200 + status_code/code）。
        let error = QwenMtTranslator::parse_response(
            r#"{"status_code":401,"code":"InvalidApiKey","message":"invalid api key"}"#,
        )
        .unwrap_err();
        assert_eq!(error.code, "SA-3002");

        let error = QwenMtTranslator::parse_response(
            r#"{"status_code":400,"code":"InvalidParameter","message":"bad target_lang"}"#,
        )
        .unwrap_err();
        assert_eq!(error.code, "SA-5001");

        // 未返回译文。
        let error =
            QwenMtTranslator::parse_response(r#"{"status_code":200,"output":{}}"#).unwrap_err();
        assert_eq!(error.code, "SA-1099");
    }

    #[test]
    fn qwen_mt_http_error_mapping() {
        assert_eq!(QwenMtTranslator::map_http_error(401, "x").code, "SA-3002");
        assert_eq!(QwenMtTranslator::map_http_error(403, "x").code, "SA-3002");
        assert_eq!(QwenMtTranslator::map_http_error(402, "x").code, "SA-3003");
        assert_eq!(QwenMtTranslator::map_http_error(429, "x").code, "SA-3004");
        assert_eq!(QwenMtTranslator::map_http_error(408, "x").code, "SA-3005");
        assert_eq!(QwenMtTranslator::map_http_error(500, "x").code, "SA-3001");
        assert_eq!(QwenMtTranslator::map_http_error(400, "x").code, "SA-5001");
        let error = QwenMtTranslator::map_http_error(500, &"x".repeat(10_000));
        assert!(error.detail.unwrap().chars().count() <= ERROR_BODY_LIMIT + 200);
    }

    #[test]
    fn cloud_provider_order_prefers_translate_provider() {
        let mut config = Config::default();
        // 契约 v5 默认：deepseek(enabled)、dashscope(qwenMt, disabled)、generic-openai(disabled)。
        let order = cloud_provider_order(&config);
        assert_eq!(order.len(), 1);
        assert_eq!(order[0].id, "deepseek");

        config.providers[1].enabled = true;
        config.translate.provider_id = Some("dashscope".to_string());
        let order = cloud_provider_order(&config);
        assert_eq!(order[0].id, "dashscope");
        assert_eq!(order[1].id, "deepseek");
    }

    #[test]
    fn cloud_provider_order_skips_providers_without_translate_cap() {
        use crate::contract::config::Cap;
        let mut config = Config::default();
        // generic-openai 启用但只声明 vision：即便作为 translate.providerId 也不应出现。
        config.providers[2].enabled = true;
        config.providers[2].caps = vec![Cap::Vision];
        config.translate.provider_id = Some("generic-openai".to_string());
        let order = cloud_provider_order(&config);
        assert_eq!(
            order
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>(),
            vec!["deepseek"]
        );

        // 全部供应商都不具备翻译能力（且无 qwenMt）→ 空链，调用方据此给出可操作错误。
        config.providers[0].caps = vec![Cap::Vision, Cap::Search];
        config.providers[1].enabled = false;
        assert!(cloud_provider_order(&config).is_empty());
    }

    #[test]
    fn fallback_chain_depends_on_backend() {
        let mut config = Config::default();
        assert_eq!(fallback_chain(&config), vec!["local", "another_cloud"]);
        config.translate.backend = TranslateBackend::Local;
        assert_eq!(fallback_chain(&config), vec!["cloud", "another_cloud"]);
    }

    #[test]
    fn glossary_rendering() {
        let mut config = Config::default();
        config.translate.glossary = vec![
            crate::contract::config::GlossaryEntry {
                from: "API".to_string(),
                to: "接口".to_string(),
                extra: serde_json::Map::new(),
            },
            crate::contract::config::GlossaryEntry {
                from: "Rust".to_string(),
                to: "Rust".to_string(),
                extra: serde_json::Map::new(),
            },
        ];
        assert_eq!(render_glossary(&config), "API => 接口; Rust => Rust");
    }

    /* ───────────────────────── v1.0.4：语言名 / 复述检测 ───────────────────────── */

    #[test]
    fn prompt_language_names_are_human_readable() {
        assert_eq!(prompt_language_name("zh"), "简体中文");
        assert_eq!(prompt_language_name("zh-CHS"), "简体中文");
        assert_eq!(prompt_language_name("zh_CN"), "简体中文");
        assert_eq!(prompt_language_name("zh-Hans"), "简体中文");
        assert_eq!(prompt_language_name("zh-CHT"), "繁体中文");
        assert_eq!(prompt_language_name("zh-TW"), "繁体中文");
        assert_eq!(prompt_language_name("en"), "英文");
        assert_eq!(prompt_language_name("en-US"), "英文");
        assert_eq!(prompt_language_name("ja"), "日文");
        assert_eq!(prompt_language_name("ko"), "韩文");
        assert_eq!(prompt_language_name(" ru "), "俄文");
        assert_eq!(prompt_language_name("ar"), "阿拉伯文");
        // 已是中文名 / 未知码原样透传；空串返回空串。
        assert_eq!(prompt_language_name("简体中文"), "简体中文");
        assert_eq!(prompt_language_name(" 英文 "), "英文");
        assert_eq!(prompt_language_name("xx-YY"), "xx-YY");
        assert_eq!(prompt_language_name(""), "");
    }

    fn local_tiny_config() -> Config {
        let mut config = Config::default();
        config.translate.backend = TranslateBackend::Local;
        config.local.model = "qwen2.5-0.5b-instruct-q4".to_string();
        config
    }

    #[test]
    fn local_minimal_prompt_uses_language_name_cloud_keeps_code() {
        let config = local_tiny_config();
        let engine =
            crate::services::prompt_engine::PromptEngine::with_override_dir("C:/not-exists");
        let request = TranslateRequest::new("Hello, world", "zh-CHS", Mode::Quick);

        let key =
            crate::services::action_router::translate_prompt_key_for(&config, &request.glossary);
        assert_eq!(key, "translate_minimal");
        let local = render_translate_prompt(&engine, &config, key, &request, "zh-CHS", true)
            .expect("本地精简提示词渲染应成功");
        assert!(local.contains("简体中文"), "local={local}");
        assert!(
            !local.contains("zh-CHS"),
            "本地提示词不应再出现语言码：{local}"
        );
        assert!(
            !local.contains("术语表"),
            "无术语表时不应出现术语表行：{local}"
        );
        assert!(local.contains("Hello, world"));

        // 云端 translate.md：保持 BCP-47 码（云端模型已验证正常）。
        let cloud =
            render_translate_prompt(&engine, &config, "translate", &request, "zh-CHS", false)
                .expect("云端提示词渲染应成功");
        assert!(cloud.contains("zh-CHS"), "云端提示词应保留语言码：{cloud}");
        assert!(!cloud.contains("简体中文"));
    }

    #[test]
    fn local_minimal_prompt_selects_glossary_template_when_glossary_present() {
        let mut config = local_tiny_config();
        let engine =
            crate::services::prompt_engine::PromptEngine::with_override_dir("C:/not-exists");
        let glossary = vec![GlossaryEntry {
            from: "API".to_string(),
            to: "接口".to_string(),
            extra: serde_json::Map::new(),
        }];
        let request = TranslateRequest::new("Hello, API", "zh-CHS", Mode::Quick)
            .with_glossary(glossary.clone());
        let key =
            crate::services::action_router::translate_prompt_key_for(&config, &request.glossary);
        assert_eq!(key, "translate_minimal_glossary");
        let prompt = render_translate_prompt(&engine, &config, key, &request, "zh-CHS", true)
            .expect("术语表模板渲染应成功");
        assert!(prompt.contains("简体中文"), "prompt={prompt}");
        assert!(
            prompt.contains("术语表（命中时按指定译法）：API => 接口"),
            "prompt={prompt}"
        );
        assert!(!prompt.contains("zh-CHS"));

        // 配置级术语表（请求级为空）同样走术语表模板。
        config.translate.glossary = glossary;
        let empty_request = TranslateRequest::new("Hello, API", "zh-CHS", Mode::Quick);
        assert_eq!(
            crate::services::action_router::translate_prompt_key_for(
                &config,
                &empty_request.glossary
            ),
            "translate_minimal_glossary"
        );
        // 空术语表 → 极简模板（无术语表行）。
        config.translate.glossary.clear();
        let plain = render_translate_prompt(
            &engine,
            &config,
            "translate_minimal",
            &empty_request,
            "zh-CHS",
            true,
        )
        .unwrap();
        assert!(!plain.contains("术语表"));
    }

    #[test]
    fn minimal_translate_prompt_fallback_uses_language_name() {
        let prompt = minimal_translate_prompt("Hello, world", "zh-CHS");
        assert!(prompt.contains("简体中文"), "prompt={prompt}");
        assert!(!prompt.contains("zh-CHS"));
        assert!(prompt.contains("Hello, world"));
    }

    #[test]
    fn looks_untranslated_detects_echo_only() {
        // 英文原文 → 中文目标：原样输出 = 未翻译。
        assert!(looks_untranslated("Hello, world", "Hello, world", "zh-CHS"));
        assert!(looks_untranslated(
            "Hello, world",
            "Hello,  world\n",
            "zh-CHS"
        ));
        // 引号 / 前缀包裹的原文同样命中。
        assert!(looks_untranslated(
            "Hello, world",
            "\"Hello, world\"",
            "zh-CHS"
        ));
        assert!(looks_untranslated(
            "Hello, world",
            "「Hello, world」",
            "zh-CHS"
        ));
        assert!(looks_untranslated(
            "Hello, world",
            "译文：Hello, world",
            "zh-CHS"
        ));
        assert!(looks_untranslated(
            "Hello, world",
            "`Hello, world`",
            "zh-CN"
        ));
        // 真实中文译文不命中。
        assert!(!looks_untranslated("Hello, world", "你好，世界", "zh-CHS"));
        // 中文 → 中文原样输出是合法输出，不命中。
        assert!(!looks_untranslated("你好，世界", "你好，世界", "zh-CHS"));
        // 同语言（英→英）原样输出不命中。
        assert!(!looks_untranslated("Hello", "Hello", "en"));
        // 有改动（追加标点）不算复述。
        assert!(!looks_untranslated("Hello", "Hello!", "zh-CHS"));
        // 目标代码未知 / auto 一律不判（避免误伤）。
        assert!(!looks_untranslated("Hello, world", "Hello, world", "xx-YY"));
        assert!(!looks_untranslated("Hello, world", "Hello, world", "auto"));
        // 空原文不判。
        assert!(!looks_untranslated("", "", "zh-CHS"));
        // 其它目标脚本：俄文原样输出合法（不命中），英文原文复述到俄文目标（命中）。
        assert!(!looks_untranslated("Привет, мир", "Привет, мир", "ru"));
        assert!(looks_untranslated("Hello, world", "Hello, world", "ru"));
        // 日文：假名译文不命中；英文原文复述到日文目标命中。
        assert!(!looks_untranslated("こんにちは", "こんにちは", "ja"));
        assert!(looks_untranslated("Hello", "Hello", "ja"));
    }

    /// 固定输出的测试网关（复述检测 / 降级链单测用）。
    struct FixedGateway(&'static str);

    #[async_trait::async_trait]
    impl LlmGateway for FixedGateway {
        async fn stream(
            &self,
            request: &LlmRequest,
            sink: &(dyn Fn(StreamChunk) + Send + Sync),
        ) -> AppResult<crate::services::llm_gateway::TaskOutcome> {
            sink(StreamChunk {
                kind: StreamKind::Text,
                chunk: self.0.to_string(),
            });
            Ok(crate::services::llm_gateway::TaskOutcome {
                usage: TokenUsage::default(),
                latency_ms: 1,
                engine: request.engine.clone(),
                sources: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn fallback_retries_cloud_after_local_echo() {
        let attempts = vec![
            TranslateAttempt {
                engine: "local".to_string(),
                kind: TranslateAttemptKind::Llm {
                    gateway: Arc::new(FixedGateway("Hello, world")),
                    prompt: "translate_minimal".to_string(),
                },
            },
            TranslateAttempt {
                engine: "deepseek".to_string(),
                kind: TranslateAttemptKind::Llm {
                    gateway: Arc::new(FixedGateway("你好，世界")),
                    prompt: "translate".to_string(),
                },
            },
        ];
        let request = TranslateRequest::new("Hello, world", "zh-CHS", Mode::Quick);
        let outcome = translate_with_fallback(&attempts, &request)
            .await
            .expect("本地回显应判定失败并降级到云端成功");
        assert_eq!(outcome.engine, "deepseek");
        assert_eq!(outcome.text, "你好，世界");
    }

    #[tokio::test]
    async fn local_echo_only_reports_actionable_error() {
        let attempts = vec![TranslateAttempt {
            engine: "local".to_string(),
            kind: TranslateAttemptKind::Llm {
                gateway: Arc::new(FixedGateway("Hello, world")),
                prompt: "translate_minimal".to_string(),
            },
        }];
        let request = TranslateRequest::new("Hello, world", "zh-CHS", Mode::Quick);
        let error = translate_with_fallback(&attempts, &request)
            .await
            .expect_err("只有回显原文的本地后端时应报错，不能伪造成功");
        assert_eq!(error.code, ErrorCode::InvalidState);
        assert!(
            error.message.contains("全部翻译后端均失败"),
            "message={}",
            error.message
        );
        assert_eq!(
            error.fallback,
            vec!["retry".to_string(), "open_settings".to_string()]
        );
        assert!(error.context.is_some());
    }

    #[tokio::test]
    async fn cover_translation_marks_echo_as_failed() {
        let translator = CoverTranslator::with_gateway(
            "local",
            Arc::new(FixedGateway("Hello, world")),
            Mode::Quick,
            "zh-CHS",
        );
        let covered = translator
            .translate_blocks(&[crate::contract::types::CoverBlock {
                id: "b0".to_string(),
                text: "Hello, world".to_string(),
                bbox: crate::contract::types::RectPhys {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                confidence: 0.9,
            }])
            .await;
        assert_eq!(covered.len(), 1);
        assert!(covered[0].failed, "回显原文必须标记失败，不伪造成功");
        assert_eq!(covered[0].translation, "Hello, world");
    }

    #[test]
    fn cover_prompt_uses_language_name() {
        let prompt = cover_prompt("Hello, world", "zh-CHS");
        assert!(prompt.contains("简体中文"), "prompt={prompt}");
        assert!(!prompt.contains("zh-CHS"), "prompt={prompt}");
        assert!(prompt.contains("Hello, world"));
        // 英文目标同样渲染为中文语言名。
        let prompt = cover_prompt("你好", "en");
        assert!(prompt.contains("英文"), "prompt={prompt}");
    }
}

/* ═══════════════════════ 覆盖式翻译（契约 v4 / 研究 §2.4） ═══════════════════════ */

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::contract::types::{ActionId, CoverBlock, CoveredBlock, SessionId, StreamKind, TaskId};
use crate::services::llm_gateway::{LlmGateway, LlmRequest, StreamChunk};

/// 覆盖翻译译文缓存容量（研究 §7.3：会话级 LRU；进程内 512 条足够覆盖多次框选）。
pub const COVER_CACHE_CAPACITY: usize = 512;

/// 单块原文上限（字符）：防止异常 OCR 结果把缓存键/提示词撑爆。
pub const MAX_COVER_BLOCK_CHARS: usize = 4000;

/// 规范化原文：合并连续空白 + 去首尾空白（提高缓存命中率）。
pub fn normalize_cache_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 纯 Rust SHA-1（缓存键 = `sha1(原文 + 语言 + 模型)`，研究 §7.3）。
///
/// 仓库未引入 `sha1` crate（离线环境不新增依赖），此处按 FIPS-180-1 实现，
/// 仅用于非安全场景的缓存键；有已知向量单测。
pub fn sha1_hex(input: &str) -> String {
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let bytes = input.as_bytes();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut message = bytes.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.chunks(64) {
        let mut words = [0u32; 80];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        for (index, word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDCu32),
                _ => (b ^ c ^ d, 0xCA62_C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }

    state.iter().map(|value| format!("{value:08x}")).collect()
}

/// 缓存键：`session:sha1(规范化原文 + 目标语言 + 模型)`。
///
/// 会话前缀保证「新框选（新 session）不会命中旧会话译文」，容量满时按 LRU 淘汰。
pub fn cover_cache_key(session: &SessionId, text: &str, target_lang: &str, model: &str) -> String {
    let payload = format!(
        "{}\u{1}{}\u{1}{}",
        normalize_cache_text(text),
        target_lang,
        model
    );
    format!("{}:{}", session.as_str(), sha1_hex(&payload))
}

/// 覆盖翻译译文 LRU 缓存（进程内、容量有界）。
#[derive(Debug)]
pub struct CoverCache {
    capacity: usize,
    order: VecDeque<String>,
    entries: HashMap<String, String>,
}

impl CoverCache {
    /// 按容量创建。
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    /// 容量。
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 当前条目数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 读取（命中后移动到队尾，实现 LRU）。
    pub fn get(&mut self, key: &str) -> Option<String> {
        let value = self.entries.get(key).cloned()?;
        if let Some(position) = self.order.iter().position(|item| item == key) {
            self.order.remove(position);
        }
        self.order.push_back(key.to_string());
        Some(value)
    }

    /// 写入（已存在则更新并移到队尾；超容量淘汰最久未使用）。
    pub fn insert(&mut self, key: String, translation: String) {
        if let Some(position) = self.order.iter().position(|item| item == &key) {
            self.order.remove(position);
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, translation);
        while self.entries.len() > self.capacity {
            if let Some(evicted) = self.order.pop_front() {
                self.entries.remove(&evicted);
            } else {
                break;
            }
        }
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }
}

impl Default for CoverCache {
    fn default() -> Self {
        Self::new(COVER_CACHE_CAPACITY)
    }
}

/// 覆盖翻译提示词（每块独立请求，便于并行/分批与失败隔离）。
///
/// 与 `translate_minimal.md` 同形态：一行指令 + 空行 + 原文。目标语言渲染为**中文语言名**
/// （本地小模型不认 `zh-CHS` 这类 BCP-47 码，会原样回显原文；v1.0.4 真机实测修复）。
pub fn cover_prompt(text: &str, target_lang: &str) -> String {
    format!(
        "把下面的文本翻译成{}，只输出译文：\n\n{text}",
        prompt_language_name(target_lang)
    )
}

/// 覆盖翻译执行器。
///
/// - `skeleton` 模式：确定性占位译文（`engine = "skeleton"`，明确标注，不伪造真实翻译）；
/// - 真实模式：逐块调用 OpenAI 兼容网关（云端或本地 llama-server），单块失败
///   保留原文并 `failed = true`（研究 §8.2「译文长度/失败不可控」）。
pub struct CoverTranslator {
    engine: String,
    gateway: Option<Arc<dyn LlmGateway>>,
    mode: Mode,
    target_lang: String,
}

impl CoverTranslator {
    /// 骨架模式（确定性占位，engine = "skeleton"）。
    pub fn skeleton() -> Self {
        Self {
            engine: "skeleton".to_string(),
            gateway: None,
            mode: Mode::Quick,
            target_lang: String::new(),
        }
    }

    /// 真实模式：走给定网关。
    pub fn with_gateway(
        engine: impl Into<String>,
        gateway: Arc<dyn LlmGateway>,
        mode: Mode,
        target_lang: impl Into<String>,
    ) -> Self {
        Self {
            engine: engine.into(),
            gateway: Some(gateway),
            mode,
            target_lang: target_lang.into(),
        }
    }

    /// 实际使用的引擎标签（如实标注：`skeleton` / 供应商 id / `local`）。
    pub fn engine(&self) -> &str {
        &self.engine
    }

    /// 是否骨架模式。
    pub fn is_skeleton(&self) -> bool {
        self.gateway.is_none()
    }

    /// 逐块翻译（不缓存；缓存由命令层按 `cover_cache_key` 处理）。
    pub async fn translate_blocks(&self, blocks: &[CoverBlock]) -> Vec<CoveredBlock> {
        let mut results = Vec::with_capacity(blocks.len());
        for block in blocks {
            let source = block.text.clone();
            let Some(gateway) = &self.gateway else {
                results.push(CoveredBlock {
                    id: block.id.clone(),
                    source: source.clone(),
                    translation: SkeletonTranslator::placeholder(&source),
                    failed: false,
                    engine: "skeleton".to_string(),
                });
                continue;
            };

            let prompt = cover_prompt(&source, &self.target_lang);
            let request = LlmRequest {
                task: TaskId::from("translate-cover"),
                session: SessionId::from("cover"),
                action: ActionId::Translate,
                mode: self.mode,
                engine: self.engine.clone(),
                prompt,
                input_text: source.clone(),
                image_data_url: None,
                cancel: Arc::new(AtomicBool::new(false)),
            };
            let collected = Arc::new(parking_lot::Mutex::new(String::new()));
            let sink_buffer = collected.clone();
            let sink = move |chunk: StreamChunk| {
                if chunk.kind == StreamKind::Text {
                    sink_buffer.lock().push_str(&chunk.chunk);
                }
            };

            let outcome = gateway.stream(&request, &sink).await;
            let translation = collected.lock().trim().to_string();
            let request_failed = outcome.is_err() || translation.is_empty();
            // 本地/云端都可能把原文原样吐回（v1.0.3 用户反馈的覆盖翻译路径根因）：
            // 命中复述检测时标记失败并保留原文，**不伪造成功**（不写缓存，见命令层）。
            let echoed =
                !request_failed && looks_untranslated(&source, &translation, &self.target_lang);
            if echoed {
                tracing::warn!(
                    engine = %self.engine,
                    target_lang = %self.target_lang,
                    "覆盖翻译返回原文（未翻译），标记失败并保留原文"
                );
            }
            let failed = request_failed || echoed;
            results.push(CoveredBlock {
                id: block.id.clone(),
                source: source.clone(),
                translation: if failed { source } else { translation },
                failed,
                engine: self.engine.clone(),
            });
        }
        results
    }
}

/// 批处理切分：按 `batch_size` 切成 `[start, end)` 区间（研究 §2.4-3：8–10 块/请求）。
pub fn plan_batches(total: usize, batch_size: usize) -> Vec<(usize, usize)> {
    let size = batch_size.max(1);
    let mut batches = Vec::new();
    let mut start = 0;
    while start < total {
        let end = (start + size).min(total);
        batches.push((start, end));
        start = end;
    }
    batches
}
