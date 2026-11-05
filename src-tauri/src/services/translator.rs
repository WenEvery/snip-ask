//! 翻译引擎（M11 / 说明书 §4.6 / §7.2）。
//!
//! 云端（通用 LLM / Qwen-MT）与本地（llama.cpp / CT2）分支，回退链：本地失败 → 云端 →
//! 提示；云端失败 → 本地（若已下载）→ 提示。骨架实现固定返回 `【骨架模式译文】` + 原文。
//!
//! TODO(real): 云端 Qwen-MT `translation_options`（source_lang / target_lang）、
//! 本地 llama.cpp OpenAI 兼容端点、CT2 按句返回。

use async_trait::async_trait;

use crate::contract::config::{Config, TranslateBackend};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::Mode;

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

/// `translate_test` 命令的实现（骨架模式固定返回占位译文）。
pub async fn translate_test(config: &Config, text: &str, target: &str) -> AppResult<String> {
    if text.trim().is_empty() {
        return Err(AppError::invalid_config("text", "待翻译文本不能为空"));
    }
    if target.trim().is_empty() {
        return Err(AppError::invalid_config("target", "目标语言不能为空"));
    }
    let translator = SkeletonTranslator;
    let request = TranslateRequest {
        text: text.to_string(),
        source_lang: Some("auto".to_string()),
        target_lang: target.to_string(),
        mode: config.modes.translate,
    };
    // TODO(real): 依据 config.translate.backend 选择云端 / 本地翻译器，并按 fallback_chain 降级。
    translator.translate(&request).await
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
        };
        assert!(translator.translate(&request).await.is_err());
    }

    #[tokio::test]
    async fn translate_test_validates_arguments() {
        let config = Config::default();
        assert!(translate_test(&config, "", "zh-CHS").await.is_err());
        assert!(translate_test(&config, "hello", "").await.is_err());
        assert_eq!(
            translate_test(&config, "hello", "zh-CHS").await.unwrap(),
            "【骨架模式译文】hello"
        );
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
/// TODO(prompt): 真实链路就绪后改走 `prompt_engine` 的 `translate.md` 模板
/// （含 `{glossary}` / `{targetLang}` 变量）；当前先给最小可用提示词。
pub fn cover_prompt(text: &str, target_lang: &str) -> String {
    format!("把下面的文字翻译成 {target_lang}，只输出译文，不要解释：\n{text}")
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
            let failed = outcome.is_err() || translation.is_empty();
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
