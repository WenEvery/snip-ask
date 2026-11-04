//! 内置资源（契约 §10）。
//!
//! 通过 `include_str!` 编译内嵌仓库根 `resources/`；用户可在
//! `%APPDATA%\SnipAsk\prompts\*.md` 覆盖同名提示词（用户 > 内置）。
//!
//! 路径层级：本文件位于 `src-tauri/src/resources/mod.rs`，
//! `../../../resources/...` 指向仓库根 `resources/`。

/// 文本搜索提示词（变量 `{text}` `{lang}` `{mode}`）。
pub const PROMPT_TEXT_SEARCH: &str = include_str!("../../../resources/prompts/text_search.md");

/// 图片搜索提示词（变量 `{lang}` `{mode}`；要求输出「识别结果/检索词/结论/来源」）。
pub const PROMPT_IMAGE_SEARCH: &str =
    include_str!("../../../resources/prompts/image_search.md");

/// 翻译提示词（变量 `{text}` `{targetLang}` `{sourceLang}` `{glossary}`；只输出译文）。
pub const PROMPT_TRANSLATE: &str = include_str!("../../../resources/prompts/translate.md");

/// 解答提示词（变量 `{text}` `{lang}` `{mode}`）。
pub const PROMPT_ANSWER: &str = include_str!("../../../resources/prompts/answer.md");

/// 追问提示词（变量 `{question}` `{context}`）。
pub const PROMPT_FOLLOWUP: &str = include_str!("../../../resources/prompts/followup.md");

/// 模型清单（`{ version, verified, models: ModelManifestEntry[] }`）。
pub const MODELS_MANIFEST: &str = include_str!("../../../resources/models.manifest.json");

/// 价格表（`{ version, currency, usdToCny, models[], search[] }`）。
pub const PRICING: &str = include_str!("../../../resources/pricing.json");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_resources_are_non_empty() {
        for (name, content) in [
            ("text_search", PROMPT_TEXT_SEARCH),
            ("image_search", PROMPT_IMAGE_SEARCH),
            ("translate", PROMPT_TRANSLATE),
            ("answer", PROMPT_ANSWER),
            ("followup", PROMPT_FOLLOWUP),
            ("models.manifest", MODELS_MANIFEST),
            ("pricing", PRICING),
        ] {
            assert!(!content.trim().is_empty(), "内置资源 {name} 为空");
        }
    }

    #[test]
    fn embedded_json_is_parseable() {
        assert!(serde_json::from_str::<serde_json::Value>(MODELS_MANIFEST).is_ok());
        assert!(serde_json::from_str::<serde_json::Value>(PRICING).is_ok());
    }
}
