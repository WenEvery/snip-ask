//! 动作路由与降级链（M08 / 说明书 §7.2）。
//!
//! `resolve` 是纯函数：根据配置把「动作 + 模式」解析为实际引擎、降级链、任务种类与
//! 提示词 key。真实网关调用由 `llm_gateway` / `search_gateway` / `translator` 完成。
//!
//! 降级链（说明书 §7.2）：
//! | 动作 | 主路径 | 降级 1 | 降级 2 | 最终 |
//! | 文本搜索 | AI 搜索 | 浏览器搜索 | — | 复制文本 |
//! | 图片搜索 | 视觉 + 联网 | 仅视觉识别 | 浏览器以图搜图 | 复制识别文本 |
//! | 翻译 | 云端 | 本地模型 | 另一云端供应商 | 提示失败 |
//! | 解答 | 云端多模态 | 纯文本 | 本地模型 | 提示失败 |

use crate::contract::config::{Config, GlossaryEntry, SearchMode, TranslateBackend};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{ActionId, EngineId, Mode};
use crate::domain::task::TaskKind;

/// 路由结果。
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedAction {
    /// 主引擎 ID。
    pub engine: EngineId,
    /// 降级链（按顺序尝试）。
    pub fallback: Vec<EngineId>,
    /// 任务种类。
    pub kind: TaskKind,
    /// 提示词模板 key（`prompt_engine` 使用）。
    pub prompt_key: String,
}

/// 动作路由器。
#[derive(Debug, Clone, Copy, Default)]
pub struct ActionRouter;

impl ActionRouter {
    /// 解析动作执行方案。
    pub fn resolve(
        &self,
        config: &Config,
        action: ActionId,
        mode: Mode,
        has_text: bool,
    ) -> AppResult<RoutedAction> {
        let primary_provider = config
            .providers
            .iter()
            .find(|provider| provider.enabled)
            .map(|provider| provider.id.clone());
        let secondary_provider = config
            .providers
            .iter()
            .filter(|provider| provider.enabled)
            .nth(1)
            .map(|provider| provider.id.clone());
        let cloud_engine = primary_provider
            .clone()
            .unwrap_or_else(|| "skeleton".to_string());

        match action {
            ActionId::Search => {
                if config.search.mode == SearchMode::Browser {
                    Ok(RoutedAction {
                        engine: "browser".to_string(),
                        fallback: vec!["copy".to_string()],
                        kind: TaskKind::Search,
                        prompt_key: "text_search".to_string(),
                    })
                } else {
                    // AI 搜索：主引擎为供应商，失败降级到浏览器搜索（最终复制文本）。
                    Ok(RoutedAction {
                        engine: cloud_engine,
                        fallback: vec!["browser".to_string(), "copy".to_string()],
                        kind: TaskKind::Search,
                        prompt_key: "text_search".to_string(),
                    })
                }
            }
            ActionId::ImageSearch => Ok(RoutedAction {
                engine: cloud_engine,
                fallback: vec!["browser".to_string(), "copy".to_string()],
                kind: TaskKind::ImageSearch,
                prompt_key: "image_search".to_string(),
            }),
            ActionId::Translate => match config.translate.backend {
                TranslateBackend::Cloud => {
                    let engine = config.translate.provider_id.clone().unwrap_or(cloud_engine);
                    let mut fallback = vec!["local".to_string()];
                    if let Some(secondary) = secondary_provider {
                        if secondary != engine {
                            fallback.push(secondary);
                        }
                    }
                    Ok(RoutedAction {
                        engine,
                        fallback,
                        kind: TaskKind::Translate,
                        prompt_key: "translate".to_string(),
                    })
                }
                TranslateBackend::Local => {
                    let mut fallback = Vec::new();
                    if let Some(cloud) = primary_provider {
                        fallback.push(cloud);
                    }
                    Ok(RoutedAction {
                        engine: "local".to_string(),
                        fallback,
                        kind: TaskKind::Translate,
                        prompt_key: translate_prompt_key(config).to_string(),
                    })
                }
            },
            ActionId::Answer => Ok(RoutedAction {
                engine: cloud_engine,
                fallback: vec!["local".to_string()],
                kind: TaskKind::Answer,
                prompt_key: "answer".to_string(),
            }),
            ActionId::Copy => {
                if !has_text {
                    return Err(AppError::new(
                        ErrorCode::OcrFailed,
                        Some("选区内没有可复制的文字".to_string()),
                    )
                    .with_fallback(vec!["image_path"]));
                }
                Ok(RoutedAction {
                    engine: "local".to_string(),
                    fallback: Vec::new(),
                    kind: TaskKind::Copy,
                    prompt_key: String::new(),
                })
            }
        }
        .map(|mut routed| {
            // 深度模式在提示词层面表达（模板内 {mode} 变量），引擎选择不变。
            let _ = mode;
            if routed.prompt_key.is_empty() {
                routed.prompt_key = "answer".to_string();
            }
            routed
        })
    }

    /// 追问路由：沿用当前引擎，降级链与解答一致。
    pub fn resolve_followup(&self, config: &Config, has_text: bool) -> AppResult<RoutedAction> {
        let mut routed = self.resolve(config, ActionId::Answer, config.modes.answer, has_text)?;
        routed.kind = TaskKind::Followup;
        routed.prompt_key = "followup".to_string();
        Ok(routed)
    }
}

/// 需要**精简翻译提示词**（`translate_minimal`）的本地模型。
///
/// 判据（均为真实模型实测）：这些模型用产品 `translate.md` 长提示词时会把原文**复述**
/// 而不是翻译（S-05：Qwen2.5-0.5B 10/10 复述原文；v1.0.3 用户反馈：HY-MT1.5-1.8B 同样复述原文），
/// 换精简提示词后可正常翻译。名单与模型清单 `resources/models.manifest.json` 的
/// `params.profile = "tiny-local"` 对应；HY-MT 为专用翻译模型但 1.8B 规模同样对长提示词敏感。
pub const LOCAL_MODELS_MINIMAL_PROMPT: &[&str] = &[
    "qwen2.5-0.5b-instruct-q4",
    "qwen2.5-1.5b-instruct-q4",
    // HY-MT1.5-1.8B（Q4_K_M）：v1.0.3 实测长提示词下输出原文；精简提示词下正常翻译。
    "hy-mt-1.8b-q4",
];

/// 翻译动作应使用的提示词 key：本地「需精简提示词」的模型 → `translate_minimal`，其余 → `translate`。
pub fn translate_prompt_key(config: &Config) -> &'static str {
    if config.translate.backend == TranslateBackend::Local
        && LOCAL_MODELS_MINIMAL_PROMPT
            .iter()
            .any(|id| config.local.model.eq_ignore_ascii_case(id))
    {
        "translate_minimal"
    } else {
        "translate"
    }
}

/// 翻译提示词 key（含**术语表分支**，供本地精简模型使用）。
///
/// 本地精简模型 + 有效术语表非空 → `translate_minimal_glossary`（把术语表追加到指令行，
/// 实测三模型不会把术语表行复述进译文）；否则退回 [`translate_prompt_key`]。
///
/// 术语表来源有两处，必须都判断：请求级 `request.glossary` 优先（渲染侧口径），
/// 否则看配置级 `config.translate.glossary`；只保留「原文与译文都非空」的条目。
pub fn translate_prompt_key_for(
    config: &Config,
    request_glossary: &[GlossaryEntry],
) -> &'static str {
    let key = translate_prompt_key(config);
    if key == "translate_minimal"
        && !crate::services::translator::effective_glossary(config, request_glossary)
            .trim()
            .is_empty()
    {
        "translate_minimal_glossary"
    } else {
        key
    }
}

/// 降级链的可执行动作名（用于 `task:error.fallback` 与前端提示）。
pub fn fallback_labels(fallback: &[EngineId]) -> Vec<String> {
    fallback.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::config::{SearchPrefs, TranslatePrefs};

    fn router() -> ActionRouter {
        ActionRouter
    }

    #[test]
    fn browser_search_routes_to_browser() {
        let config = Config::default();
        let routed = router()
            .resolve(&config, ActionId::Search, Mode::Quick, true)
            .unwrap();
        assert_eq!(routed.engine, "browser");
        assert_eq!(routed.kind, TaskKind::Search);
        assert_eq!(routed.prompt_key, "text_search");
        assert_eq!(routed.fallback, vec!["copy".to_string()]);
    }

    #[test]
    fn ai_search_routes_to_provider_with_browser_fallback() {
        let mut config = Config::default();
        config.search = SearchPrefs {
            mode: SearchMode::Ai,
            ..SearchPrefs::default()
        };
        let routed = router()
            .resolve(&config, ActionId::Search, Mode::Deep, true)
            .unwrap();
        assert_eq!(routed.engine, "deepseek");
        assert_eq!(
            routed.fallback,
            vec!["browser".to_string(), "copy".to_string()]
        );
    }

    #[test]
    fn translate_cloud_and_local_fallbacks() {
        let config = Config::default();
        let routed = router()
            .resolve(&config, ActionId::Translate, Mode::Quick, true)
            .unwrap();
        assert_eq!(routed.engine, "deepseek");
        assert_eq!(routed.fallback, vec!["local".to_string()]);
        assert_eq!(routed.prompt_key, "translate");

        let mut config = Config::default();
        config.translate = TranslatePrefs {
            backend: TranslateBackend::Local,
            ..TranslatePrefs::default()
        };
        let routed = router()
            .resolve(&config, ActionId::Translate, Mode::Quick, true)
            .unwrap();
        assert_eq!(routed.engine, "local");
        assert_eq!(routed.fallback, vec!["deepseek".to_string()]);
    }

    #[test]
    fn answer_and_image_search_have_expected_chains() {
        let config = Config::default();
        let answer = router()
            .resolve(&config, ActionId::Answer, Mode::Quick, true)
            .unwrap();
        assert_eq!(answer.engine, "deepseek");
        assert_eq!(answer.fallback, vec!["local".to_string()]);
        assert_eq!(answer.prompt_key, "answer");

        let image = router()
            .resolve(&config, ActionId::ImageSearch, Mode::Quick, false)
            .unwrap();
        assert_eq!(image.kind, TaskKind::ImageSearch);
        assert_eq!(image.prompt_key, "image_search");
        assert_eq!(
            image.fallback,
            vec!["browser".to_string(), "copy".to_string()]
        );
    }

    #[test]
    fn copy_requires_text() {
        let config = Config::default();
        let routed = router()
            .resolve(&config, ActionId::Copy, Mode::Quick, true)
            .unwrap();
        assert_eq!(routed.kind, TaskKind::Copy);
        assert!(routed.fallback.is_empty());

        let error = router()
            .resolve(&config, ActionId::Copy, Mode::Quick, false)
            .unwrap_err();
        assert_eq!(error.code, "SA-2002");
    }

    #[test]
    fn local_models_minimal_prompt_includes_tiny_and_hy_mt() {
        // 云端：始终用产品长提示词。
        let config = Config::default();
        assert_eq!(translate_prompt_key(&config), "translate");

        // 本地 + HY-MT1.5-1.8B：v1.0.3 实测长提示词会复述原文，必须用精简提示词。
        let mut config = Config::default();
        config.translate = TranslatePrefs {
            backend: TranslateBackend::Local,
            ..TranslatePrefs::default()
        };
        config.local.model = "hy-mt-1.8b-q4".to_string();
        assert_eq!(translate_prompt_key(&config), "translate_minimal");
        let routed = router()
            .resolve(&config, ActionId::Translate, Mode::Quick, true)
            .unwrap();
        assert_eq!(routed.prompt_key, "translate_minimal");
        assert_eq!(routed.engine, "local");

        // 本地 + 极小模型：精简提示词（S-05 实测避免复述原文）。
        config.local.model = "qwen2.5-0.5b-instruct-q4".to_string();
        assert_eq!(translate_prompt_key(&config), "translate_minimal");
        config.local.model = "qwen2.5-1.5b-instruct-q4".to_string();
        assert_eq!(translate_prompt_key(&config), "translate_minimal");

        // 本地 + 其他模型（4B 级通用 instruct）：长提示词。
        config.local.model = "qwen3-4b-q4".to_string();
        assert_eq!(translate_prompt_key(&config), "translate");

        // 大小写不敏感（配置可能来自用户输入/迁移）。
        config.local.model = "HY-MT-1.8B-Q4".to_string();
        assert_eq!(translate_prompt_key(&config), "translate_minimal");
    }

    #[test]
    fn translate_prompt_key_for_selects_glossary_template() {
        let glossary = vec![GlossaryEntry {
            from: "API".to_string(),
            to: "接口".to_string(),
            extra: serde_json::Map::new(),
        }];

        // 本地精简模型 + 请求级有效术语表 → 术语表模板。
        let mut config = Config::default();
        config.translate = TranslatePrefs {
            backend: TranslateBackend::Local,
            ..TranslatePrefs::default()
        };
        config.local.model = "qwen2.5-0.5b-instruct-q4".to_string();
        assert_eq!(
            translate_prompt_key_for(&config, &glossary),
            "translate_minimal_glossary"
        );
        // 无术语表 → 极简模板。
        assert_eq!(translate_prompt_key_for(&config, &[]), "translate_minimal");
        // 只有空条目 → 等同无有效术语表。
        let empty_entries = vec![GlossaryEntry {
            from: "  ".to_string(),
            to: "".to_string(),
            extra: serde_json::Map::new(),
        }];
        assert_eq!(
            translate_prompt_key_for(&config, &empty_entries),
            "translate_minimal"
        );

        // 配置级术语表（请求级为空）同样触发术语表模板（渲染侧 `{glossary}` 口径一致）。
        config.translate.glossary = glossary.clone();
        assert_eq!(
            translate_prompt_key_for(&config, &[]),
            "translate_minimal_glossary"
        );
        config.translate.glossary.clear();

        // 本地非精简模型 / 云端：始终用产品长提示词（不分支）。
        config.local.model = "qwen3-4b-q4".to_string();
        assert_eq!(translate_prompt_key_for(&config, &glossary), "translate");
        let cloud = Config::default();
        assert_eq!(translate_prompt_key_for(&cloud, &glossary), "translate");
    }

    #[test]
    fn followup_uses_followup_prompt() {
        let config = Config::default();
        let routed = router().resolve_followup(&config, true).unwrap();
        assert_eq!(routed.kind, TaskKind::Followup);
        assert_eq!(routed.prompt_key, "followup");
    }
}
