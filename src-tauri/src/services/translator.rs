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
