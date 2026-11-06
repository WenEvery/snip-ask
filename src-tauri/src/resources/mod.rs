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
pub const PROMPT_IMAGE_SEARCH: &str = include_str!("../../../resources/prompts/image_search.md");

/// 翻译提示词（变量 `{text}` `{targetLang}` `{sourceLang}` `{glossary}`；只输出译文）。
pub const PROMPT_TRANSLATE: &str = include_str!("../../../resources/prompts/translate.md");

/// 本地「极小模型」精简翻译提示词（S-05 实测：0.5B 类小模型用长提示词会复述原文）。
/// 变量与 `translate.md` 相同；仅当 `translate.backend = local` 且 `local.model` 属于极小模型档时使用。
pub const PROMPT_TRANSLATE_MINIMAL: &str =
    include_str!("../../../resources/prompts/translate_minimal.md");

/// 本地「极小模型」精简翻译提示词（术语表非空分支）。
///
/// 与 [`PROMPT_TRANSLATE_MINIMAL`] 的唯一区别是把术语表追加到指令行（避免空术语表行被小模型复述）；
/// 仅当本地精简模型 + 有效术语表非空时使用。
pub const PROMPT_TRANSLATE_MINIMAL_GLOSSARY: &str =
    include_str!("../../../resources/prompts/translate_minimal_glossary.md");

/// 解答提示词（变量 `{text}` `{lang}` `{mode}`）。
pub const PROMPT_ANSWER: &str = include_str!("../../../resources/prompts/answer.md");

/// 追问提示词（变量 `{question}` `{context}`）。
pub const PROMPT_FOLLOWUP: &str = include_str!("../../../resources/prompts/followup.md");

/// 模型清单（`{ version, verified, models: ModelManifestEntry[] }`）。
pub const MODELS_MANIFEST: &str = include_str!("../../../resources/models.manifest.json");

/// 价格表（`{ version, currency, usdToCny, models[], search[] }`）。
pub const PRICING: &str = include_str!("../../../resources/pricing.json");

/// 推理后端组件清单（`{ version, verified, backends: BackendManifestEntry[] }`，契约 v4）。
///
/// Vulkan 为默认 GPU 组件（AMD/Intel/NVIDIA 通用）；CUDA 为 NVIDIA 可选组件；CPU 随包。
pub const BACKENDS_MANIFEST: &str = include_str!("../../../resources/backends.manifest.json");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_resources_are_non_empty() {
        for (name, content) in [
            ("text_search", PROMPT_TEXT_SEARCH),
            ("image_search", PROMPT_IMAGE_SEARCH),
            ("translate", PROMPT_TRANSLATE),
            ("translate_minimal", PROMPT_TRANSLATE_MINIMAL),
            (
                "translate_minimal_glossary",
                PROMPT_TRANSLATE_MINIMAL_GLOSSARY,
            ),
            ("answer", PROMPT_ANSWER),
            ("followup", PROMPT_FOLLOWUP),
            ("models.manifest", MODELS_MANIFEST),
            ("backends.manifest", BACKENDS_MANIFEST),
            ("pricing", PRICING),
        ] {
            assert!(!content.trim().is_empty(), "内置资源 {name} 为空");
        }
    }

    #[test]
    fn embedded_json_is_parseable() {
        assert!(serde_json::from_str::<serde_json::Value>(MODELS_MANIFEST).is_ok());
        assert!(serde_json::from_str::<serde_json::Value>(BACKENDS_MANIFEST).is_ok());
        assert!(serde_json::from_str::<serde_json::Value>(PRICING).is_ok());
    }
}
