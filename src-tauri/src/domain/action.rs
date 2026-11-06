//! 动作注册表与动作菜单可见性（契约 §3 `ActionItem` / §5 / 说明书 M07）。
//!
//! `visible_actions` 是纯函数：输入当前配置与「选区内是否有文字」，输出按配置顺序过滤后的
//! 动作项；不可用项保留在列表中并给出 `disabledReason`（前端置灰 + tooltip）。

use std::collections::HashSet;

use crate::contract::config::Config;
use crate::contract::types::{ActionId, ActionItem, ActionTarget, Mode};

use super::mode::mode_label;

// ---------------------------------------------------------------------------
// 受限模式能力门禁（S-07）
// ---------------------------------------------------------------------------

/// 受限模式下「无云端能力」的可操作提示（设置页路径 + 本地模型备选）。
pub const REASON_NO_CLOUD_KEY: &str = "未配置 API Key：请到设置 → AI 服务填写，或下载本地模型";
/// 有云端 Key 但没有供应商声明「联网搜索」能力时的可操作提示。
pub const REASON_NO_SEARCH_CAP: &str =
    "没有可用联网搜索能力：请到设置 → AI 服务为供应商勾选「联网搜索」，或改用浏览器搜索";
/// 有云端 Key 但没有供应商声明「视觉（看图）」能力时的可操作提示。
pub const REASON_NO_VISION_CAP: &str =
    "没有可用的视觉能力：请到设置 → AI 服务为供应商勾选「视觉（看图）」";
/// 有云端 Key 但没有供应商声明「翻译」能力（且无 Qwen-MT）时的可操作提示。
pub const REASON_NO_TRANSLATE_CAP: &str =
    "没有可用的云端翻译能力：请到设置 → AI 服务为供应商勾选「翻译」，或改用本地翻译";
/// 本地翻译后端但模型未就绪时的可操作提示。
pub const REASON_NO_LOCAL_MODEL: &str = "本地模型未下载：请到设置 → 模型库下载";
/// 本地 OCR 不可用（正常情况下内嵌引擎始终可用，保留兜底文案）。
pub const REASON_NO_LOCAL_OCR: &str = "本地 OCR 不可用：请重新安装应用或到设置 → 高级检查";

/// 云端供应商能力快照（按 `enabled && hasKey` 的 provider `caps` 派生；v1.1.0）。
///
/// `config.providers[].caps` 在 v1.1.0 起真正参与动作可用性：
/// AI 搜索需要至少一个可用供应商具备 `search`、图片搜索需要 `vision`、
/// 云端翻译需要 `translate`（`kind = qwenMt` 视为具备）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CloudCaps {
    /// 视觉（图片搜索主路径）。
    pub vision: bool,
    /// 联网搜索（AI 搜索）。
    pub search: bool,
    /// 云端翻译（`translate` cap，或 `kind = qwenMt`）。
    pub translate: bool,
}

impl CloudCaps {
    /// 合并一个「可用」供应商（调用方需先按 `enabled && hasKey` 过滤）的能力。
    ///
    /// `translate_by_kind` 对应 `provider.kind = qwenMt`：Qwen-MT 专用翻译端点
    /// 即使没有显式 `translate` cap 也具备云端翻译能力（契约 v5 语义）。
    pub fn add_provider(&mut self, caps: &[crate::contract::config::Cap], translate_by_kind: bool) {
        use crate::contract::config::Cap;
        self.vision |= caps.contains(&Cap::Vision);
        self.search |= caps.contains(&Cap::Search);
        self.translate |= caps.contains(&Cap::Translate) || translate_by_kind;
    }
}

/// 环境能力快照（S-07「受限模式」的输入，纯数据、可单测）。
///
/// 由 IPC 命令层在运行时探测后注入；`domain` 层不直接访问凭据/文件系统。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// 存在 `enabled` 且已保存 API Key 的供应商（任意能力）。
    pub cloud_available: bool,
    /// 存在可用供应商具备 `vision` cap。
    pub vision_available: bool,
    /// 存在可用供应商具备 `search` cap。
    pub search_available: bool,
    /// 存在可用供应商具备 `translate` cap（或 `kind = qwenMt`）。
    pub translate_available: bool,
    /// 本地 OCR 引擎可用（骨架模式视为可用）。
    pub local_ocr_available: bool,
    /// 浏览器搜索可用（外部浏览器始终存在）。
    pub browser_search_available: bool,
    /// 已安装至少一个本地模型（`<models_dir>/<id>/.installed`）。
    pub local_model_available: bool,
}

impl Capabilities {
    /// 全能力（保持旧 API 行为；单测与兼容调用使用）。
    pub fn all_available() -> Self {
        Self {
            cloud_available: true,
            vision_available: true,
            search_available: true,
            translate_available: true,
            local_ocr_available: true,
            browser_search_available: true,
            local_model_available: true,
        }
    }

    /// 由运行时探测结果派生（浏览器搜索与本地 OCR 视为内建能力）。
    ///
    /// 兼容旧签名：只按「是否有云端 Key」派生，三个云端能力随 `cloud_available`，
    /// 等价于 v1.1.0 之前「有 Key 即可用」的语义。
    pub fn derive(cloud_available: bool, local_model_available: bool) -> Self {
        Self {
            cloud_available,
            vision_available: cloud_available,
            search_available: cloud_available,
            translate_available: cloud_available,
            local_ocr_available: true,
            browser_search_available: true,
            local_model_available,
        }
    }

    /// 由运行时探测结果 + 云端 provider caps 派生（v1.1.0 真实语义）。
    ///
    /// `cloud_available` 仍表示「存在 enabled+hasKey 的供应商」（解答等通用云端动作使用），
    /// `cloud_caps` 则按各供应商声明的能力细化搜索 / 图片搜索 / 云端翻译的可用性。
    pub fn derive_from_cloud_caps(
        cloud_available: bool,
        cloud_caps: CloudCaps,
        local_model_available: bool,
    ) -> Self {
        Self {
            cloud_available,
            vision_available: cloud_caps.vision,
            search_available: cloud_caps.search,
            translate_available: cloud_caps.translate,
            local_ocr_available: true,
            browser_search_available: true,
            local_model_available,
        }
    }

    /// 是否处于受限模式：无任何云端供应商 Key 时进入受限模式
    /// （本地 OCR / 浏览器搜索 / 复制仍可用；本地模型就绪时本地翻译也可用）。
    pub fn restricted(&self) -> bool {
        !self.cloud_available
    }
}

/// 动作描述（静态注册表条目，M20 扩展点）。
#[derive(Debug, Clone, PartialEq)]
pub struct ActionDescriptor {
    /// 动作 ID。
    pub id: ActionId,
    /// 作用目标。
    pub applies_to: ActionTarget,
    /// 默认模式。
    pub default_mode: Mode,
    /// 默认中文标签。
    pub default_label: &'static str,
    /// 关联的配置热键字段（点分路径）；无绑定为 `None`。
    pub shortcut_key: Option<&'static str>,
}

static REGISTRY: [ActionDescriptor; 5] = [
    ActionDescriptor {
        id: ActionId::Search,
        applies_to: ActionTarget::Text,
        default_mode: Mode::Quick,
        default_label: "搜索",
        shortcut_key: Some("hotkeys.wordGrab"),
    },
    ActionDescriptor {
        id: ActionId::ImageSearch,
        applies_to: ActionTarget::Image,
        default_mode: Mode::Quick,
        default_label: "图片搜索",
        shortcut_key: Some("hotkeys.imageSearch"),
    },
    ActionDescriptor {
        id: ActionId::Translate,
        applies_to: ActionTarget::Both,
        default_mode: Mode::Quick,
        default_label: "翻译",
        shortcut_key: Some("hotkeys.translate"),
    },
    ActionDescriptor {
        id: ActionId::Answer,
        applies_to: ActionTarget::Both,
        default_mode: Mode::Quick,
        default_label: "解答",
        shortcut_key: Some("hotkeys.answer"),
    },
    ActionDescriptor {
        id: ActionId::Copy,
        applies_to: ActionTarget::Text,
        default_mode: Mode::Quick,
        default_label: "复制文字",
        shortcut_key: None,
    },
];

/// 默认动作注册表（顺序与契约 §3 的 `ActionId` 一致）。
pub fn default_registry() -> &'static [ActionDescriptor] {
    &REGISTRY
}

/// 按 ID 查描述。
pub fn descriptor(action: ActionId) -> Option<&'static ActionDescriptor> {
    REGISTRY.iter().find(|item| item.id == action)
}

/// 动作的当前模式（`config.modes` 中对应字段）。
pub fn mode_for(config: &Config, action: ActionId) -> Mode {
    match action {
        ActionId::Search => config.modes.search,
        ActionId::ImageSearch => config.modes.image_search,
        ActionId::Translate => config.modes.translate,
        ActionId::Answer => config.modes.answer,
        ActionId::Copy => Mode::Quick,
    }
}

/// 动作当前是否可用；不可用时给出中文原因（旧签名：按「全能力」评估隐私规则）。
///
/// 规则（说明书 §7 / 实施架构 §8）：
/// - `privacy.localOnly` 关闭全部云端动作，但保留浏览器搜索、本地翻译、复制；
/// - 文本上传关闭 → 需要云端文本的动作置灰；
/// - 图像上传关闭 → 图片搜索置灰。
pub fn availability(config: &Config, action: ActionId) -> (bool, Option<String>) {
    availability_with_capabilities(config, action, &Capabilities::all_available())
}

/// 动作当前是否可用（含受限模式能力门禁）；不可用时给出可操作中文原因。
///
/// 判定顺序：能力门禁优先，其次隐私开关（两者都不可用时优先暴露「可以怎么修」）。
/// - 无云端 Key → 云动作（AI 搜索 / 图片搜索 / 云端翻译 / 解答）置灰并提示配置路径；
/// - 浏览器搜索、复制不依赖云端能力，始终可用；
/// - 本地翻译后端：本地模型未就绪 → 提示下载；已就绪 → 可用（不受云端 Key 影响）；
/// - 本地 OCR 不可用属于内嵌资源异常，图片类动作给出兜底提示。
pub fn availability_with_capabilities(
    config: &Config,
    action: ActionId,
    caps: &Capabilities,
) -> (bool, Option<String>) {
    // 除「复制」外，所有动作都依赖本地 OCR 的识别结果；OCR 不可用时统一兜底。
    if action != ActionId::Copy && !caps.local_ocr_available {
        return (false, Some(REASON_NO_LOCAL_OCR.to_string()));
    }
    match action {
        ActionId::Copy => (true, None),
        ActionId::Search => {
            // 浏览器搜索不上传选区内容、不依赖云端能力，仅在 AI 搜索模式下受开关约束。
            if config.search.mode == crate::contract::config::SearchMode::Browser {
                (true, None)
            } else if !caps.cloud_available {
                (false, Some(REASON_NO_CLOUD_KEY.to_string()))
            } else if !caps.search_available {
                // v1.1.0：AI 搜索需要至少一个可用供应商声明 `search` 能力。
                (false, Some(REASON_NO_SEARCH_CAP.to_string()))
            } else if config.privacy.local_only {
                (false, Some("已开启仅本地模式，AI 搜索不可用".to_string()))
            } else if !config.privacy.allow_text_upload {
                (false, Some("已关闭文本上传，AI 搜索不可用".to_string()))
            } else {
                (true, None)
            }
        }
        ActionId::ImageSearch => {
            if !caps.cloud_available {
                (false, Some(REASON_NO_CLOUD_KEY.to_string()))
            } else if !caps.vision_available {
                // v1.1.0：图片搜索需要至少一个可用供应商声明 `vision` 能力。
                (false, Some(REASON_NO_VISION_CAP.to_string()))
            } else if config.privacy.local_only {
                (false, Some("已开启仅本地模式，图片搜索不可用".to_string()))
            } else if !config.privacy.allow_image_upload {
                (false, Some("已关闭图像上传，图片搜索不可用".to_string()))
            } else {
                (true, None)
            }
        }
        ActionId::Translate => {
            if config.translate.backend == crate::contract::config::TranslateBackend::Local {
                // 本地翻译不上传文本；但需要本地模型就绪。
                if !caps.local_model_available {
                    (false, Some(REASON_NO_LOCAL_MODEL.to_string()))
                } else {
                    (true, None)
                }
            } else if !caps.cloud_available {
                (false, Some(REASON_NO_CLOUD_KEY.to_string()))
            } else if !caps.translate_available {
                // v1.1.0：云端翻译需要至少一个可用供应商声明 `translate`（或 kind = qwenMt）。
                (false, Some(REASON_NO_TRANSLATE_CAP.to_string()))
            } else if config.privacy.local_only {
                (false, Some("已开启仅本地模式，云端翻译不可用".to_string()))
            } else if !config.privacy.allow_text_upload {
                (false, Some("已关闭文本上传，云端翻译不可用".to_string()))
            } else {
                (true, None)
            }
        }
        ActionId::Answer => {
            if !caps.cloud_available {
                (false, Some(REASON_NO_CLOUD_KEY.to_string()))
            } else if config.privacy.local_only {
                (false, Some("已开启仅本地模式，云端解答不可用".to_string()))
            } else if !config.privacy.allow_text_upload {
                (false, Some("已关闭文本上传，解答不可用".to_string()))
            } else {
                (true, None)
            }
        }
    }
}

/// 动作绑定的快捷键（规范化后）；未绑定返回 `None`。
pub fn shortcut_for(config: &Config, action: ActionId) -> Option<String> {
    let raw = match action {
        ActionId::Search => &config.hotkeys.word_grab,
        ActionId::ImageSearch => &config.hotkeys.image_search,
        ActionId::Translate => {
            if config.hotkeys.translate.is_empty() {
                &config.hotkeys.quick_translate
            } else {
                &config.hotkeys.translate
            }
        }
        ActionId::Answer => &config.hotkeys.answer,
        ActionId::Copy => return None,
    };
    crate::contract::config::normalize_hotkey(raw)
}

/// 根据配置生成动作菜单（契约 §7 `menu:ready` 的 `items`）。
///
/// - `has_text = true` 使用 `actions.menu.text`，否则使用 `actions.menu.image`；
/// - 保留配置顺序，重复项去重；
/// - 不可用动作保留并给出 `disabledReason`。
///
/// 旧签名：按「全能力」评估，保持兼容行为；运行时（`selection_commit`）请使用
/// [`visible_actions_with_capabilities`] 注入真实能力快照。
pub fn visible_actions(config: &Config, has_text: bool) -> Vec<ActionItem> {
    visible_actions_with_capabilities(config, has_text, &Capabilities::all_available())
}

/// 同 [`visible_actions`]，但按运行时能力快照（受限模式门禁）计算可用性。
pub fn visible_actions_with_capabilities(
    config: &Config,
    has_text: bool,
    caps: &Capabilities,
) -> Vec<ActionItem> {
    let menu = if has_text {
        &config.actions.menu.text
    } else {
        &config.actions.menu.image
    };
    let mut seen: HashSet<ActionId> = HashSet::new();
    let mut items = Vec::new();
    for action in menu {
        if !seen.insert(*action) {
            continue;
        }
        let Some(desc) = descriptor(*action) else {
            continue;
        };
        let (enabled, disabled_reason) = availability_with_capabilities(config, *action, caps);
        items.push(ActionItem {
            id: *action,
            label: desc.default_label.to_string(),
            target: desc.applies_to,
            mode: mode_for(config, *action),
            enabled,
            disabled_reason,
            shortcut: shortcut_for(config, *action),
        });
    }
    items
}

/// 动作的模式中文名（结果窗「快速 / 深度」标注）。
pub fn action_mode_label(config: &Config, action: ActionId) -> &'static str {
    mode_label(mode_for(config, action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::config::{
        Cap, PrivacyPrefs, SearchMode, SearchPrefs, TranslateBackend, TranslatePrefs,
    };

    #[test]
    fn default_order_matches_contract_menu() {
        let config = Config::default();
        let items = visible_actions(&config, true);
        let ids: Vec<ActionId> = items.iter().map(|item| item.id).collect();
        assert_eq!(
            ids,
            vec![
                ActionId::Search,
                ActionId::Translate,
                ActionId::Answer,
                ActionId::Copy
            ]
        );
        assert!(items.iter().all(|item| item.enabled));
        assert_eq!(items[0].label, "搜索");
        assert_eq!(items[0].target, ActionTarget::Text);
        assert_eq!(items[0].mode, Mode::Quick);
        assert_eq!(items[0].shortcut, None);

        let items = visible_actions(&config, false);
        let ids: Vec<ActionId> = items.iter().map(|item| item.id).collect();
        assert_eq!(
            ids,
            vec![ActionId::ImageSearch, ActionId::Answer, ActionId::Translate]
        );
    }

    #[test]
    fn local_only_disables_cloud_actions_but_keeps_browser_and_copy() {
        let mut config = Config::default();
        config.privacy = PrivacyPrefs {
            allow_text_upload: true,
            allow_image_upload: true,
            local_only: true,
            extra: serde_json::Map::new(),
        };
        // 浏览器搜索 + 复制仍可用。
        let items = visible_actions(&config, true);
        assert!(
            items
                .iter()
                .find(|item| item.id == ActionId::Search)
                .unwrap()
                .enabled
        );
        assert!(
            items
                .iter()
                .find(|item| item.id == ActionId::Copy)
                .unwrap()
                .enabled
        );
        let translate = items
            .iter()
            .find(|item| item.id == ActionId::Translate)
            .unwrap();
        assert!(!translate.enabled);
        assert!(translate
            .disabled_reason
            .as_deref()
            .unwrap()
            .contains("仅本地"));

        // AI 搜索模式下搜索也置灰。
        config.search = SearchPrefs {
            mode: SearchMode::Ai,
            ..SearchPrefs::default()
        };
        let items = visible_actions(&config, true);
        let search = items
            .iter()
            .find(|item| item.id == ActionId::Search)
            .unwrap();
        assert!(!search.enabled);
    }

    #[test]
    fn upload_switches_disable_actions() {
        let mut config = Config::default();
        config.privacy.allow_text_upload = false;
        config.search = SearchPrefs {
            mode: SearchMode::Ai,
            ..SearchPrefs::default()
        };
        let items = visible_actions(&config, true);
        let search = items
            .iter()
            .find(|item| item.id == ActionId::Search)
            .unwrap();
        assert!(!search.enabled);
        assert!(search
            .disabled_reason
            .as_deref()
            .unwrap()
            .contains("文本上传"));

        let mut config = Config::default();
        config.privacy.allow_image_upload = false;
        let items = visible_actions(&config, false);
        let image_search = items
            .iter()
            .find(|item| item.id == ActionId::ImageSearch)
            .unwrap();
        assert!(!image_search.enabled);
    }

    #[test]
    fn local_translate_backend_is_available_in_local_only_mode() {
        let mut config = Config::default();
        config.privacy.local_only = true;
        config.translate = TranslatePrefs {
            backend: TranslateBackend::Local,
            ..TranslatePrefs::default()
        };
        let items = visible_actions(&config, true);
        let translate = items
            .iter()
            .find(|item| item.id == ActionId::Translate)
            .unwrap();
        assert!(translate.enabled);
    }

    #[test]
    fn menu_dedupes_and_uses_configured_shortcut() {
        let mut config = Config::default();
        config.actions.menu.text = vec![ActionId::Answer, ActionId::Answer, ActionId::Search];
        config.hotkeys.answer = "ctrl+shift+a".to_string();
        let items = visible_actions(&config, true);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, ActionId::Answer);
        assert_eq!(items[0].shortcut.as_deref(), Some("Ctrl+Shift+A"));
    }

    fn restricted() -> Capabilities {
        Capabilities::derive(false, false)
    }

    fn item(items: &[ActionItem], id: ActionId) -> &ActionItem {
        items.iter().find(|item| item.id == id).unwrap()
    }

    #[test]
    fn restricted_mode_disables_cloud_actions_but_keeps_browser_search_and_copy() {
        let config = Config::default();
        let items = visible_actions_with_capabilities(&config, true, &restricted());
        // 默认搜索模式为 browser：搜索（浏览器）与复制仍可用。
        assert!(item(&items, ActionId::Search).enabled);
        assert!(item(&items, ActionId::Copy).enabled);
        // 云动作置灰，且原因可操作（含设置页路径 / 本地模型备选）。
        for id in [ActionId::Translate, ActionId::Answer] {
            let entry = item(&items, id);
            assert!(!entry.enabled, "{id:?} 应禁用");
            let reason = entry.disabled_reason.as_deref().unwrap();
            assert!(
                reason.contains("设置 → AI 服务"),
                "原因缺少设置路径：{reason}"
            );
            assert!(
                reason.contains("下载本地模型"),
                "原因缺少本地模型备选：{reason}"
            );
        }
        // 图片菜单：图片搜索同样被禁用。
        let image_items = visible_actions_with_capabilities(&config, false, &restricted());
        let image_search = item(&image_items, ActionId::ImageSearch);
        assert!(!image_search.enabled);
        assert!(image_search
            .disabled_reason
            .as_deref()
            .unwrap()
            .contains("设置 → AI 服务"));
    }

    #[test]
    fn restricted_mode_keeps_local_translate_when_model_installed() {
        let mut config = Config::default();
        config.translate = TranslatePrefs {
            backend: TranslateBackend::Local,
            ..TranslatePrefs::default()
        };
        // 无模型 → 禁用并提示下载。
        let items = visible_actions_with_capabilities(&config, true, &restricted());
        let translate = item(&items, ActionId::Translate);
        assert!(!translate.enabled);
        assert!(translate
            .disabled_reason
            .as_deref()
            .unwrap()
            .contains("模型库下载"));
        // 模型已安装 → 本地翻译可用（无需云端 Key）。
        let caps = Capabilities::derive(false, true);
        let items = visible_actions_with_capabilities(&config, true, &caps);
        assert!(item(&items, ActionId::Translate).enabled);
    }

    #[test]
    fn cloud_key_restores_cloud_actions() {
        let config = Config::default();
        let caps = Capabilities::derive(true, false);
        let items = visible_actions_with_capabilities(&config, true, &caps);
        assert!(item(&items, ActionId::Translate).enabled);
        assert!(item(&items, ActionId::Answer).enabled);
        // 默认隐私开关全开，故全可用。
        assert!(items.iter().all(|entry| entry.enabled));
    }

    #[test]
    fn local_ocr_unavailable_disables_text_actions_except_copy() {
        let config = Config::default();
        let caps = Capabilities {
            local_ocr_available: false,
            ..Capabilities::all_available()
        };
        let items = visible_actions_with_capabilities(&config, true, &caps);
        assert!(!item(&items, ActionId::Search).enabled);
        assert!(!item(&items, ActionId::Translate).enabled);
        assert!(item(&items, ActionId::Copy).enabled);
    }

    #[test]
    fn legacy_visible_actions_ignores_capability_gate() {
        // 兼容性：旧签名不注入能力快照，行为与改造前一致（全部可用）。
        let config = Config::default();
        let items = visible_actions(&config, true);
        assert!(items.iter().all(|entry| entry.enabled));
        assert!(Capabilities::all_available().cloud_available);
        assert!(!restricted().cloud_available);
        assert!(restricted().restricted());
    }

    /* ───────────── v1.1.0：provider caps 参与动作可用性 ───────────── */

    fn cloud_caps(vision: bool, search: bool, translate: bool) -> Capabilities {
        Capabilities::derive_from_cloud_caps(
            true,
            CloudCaps {
                vision,
                search,
                translate,
            },
            false,
        )
    }

    #[test]
    fn cloud_caps_merges_provider_caps_and_qwen_mt_kind() {
        let mut caps = CloudCaps::default();
        caps.add_provider(&[Cap::Vision], false);
        assert!(caps.vision && !caps.search && !caps.translate);
        caps.add_provider(&[Cap::Translate], false);
        assert!(caps.translate);
        // qwenMt 即使没有显式 translate cap 也视为可翻译（契约 v5 语义）。
        let mut only_kind = CloudCaps::default();
        only_kind.add_provider(&[], true);
        assert!(only_kind.translate && !only_kind.vision && !only_kind.search);
    }

    #[test]
    fn ai_search_requires_search_cap_when_cloud_key_exists() {
        let mut config = Config::default();
        config.search = SearchPrefs {
            mode: SearchMode::Ai,
            ..SearchPrefs::default()
        };
        // 有 Key 但没有供应商声明 search → 禁用并提示勾选能力。
        let items =
            visible_actions_with_capabilities(&config, true, &cloud_caps(true, false, true));
        let search = item(&items, ActionId::Search);
        assert!(!search.enabled);
        assert!(search
            .disabled_reason
            .as_deref()
            .unwrap()
            .contains("联网搜索"));
        // 具备 search cap → 可用。
        let items = visible_actions_with_capabilities(&config, true, &cloud_caps(true, true, true));
        assert!(item(&items, ActionId::Search).enabled);
        // 浏览器搜索不受 caps 影响。
        let mut browser = Config::default();
        browser.search = SearchPrefs {
            mode: SearchMode::Browser,
            ..SearchPrefs::default()
        };
        let items =
            visible_actions_with_capabilities(&browser, true, &cloud_caps(false, false, false));
        assert!(item(&items, ActionId::Search).enabled);
    }

    #[test]
    fn image_search_requires_vision_cap_when_cloud_key_exists() {
        let config = Config::default();
        let items =
            visible_actions_with_capabilities(&config, false, &cloud_caps(false, true, true));
        let image_search = item(&items, ActionId::ImageSearch);
        assert!(!image_search.enabled);
        assert!(image_search
            .disabled_reason
            .as_deref()
            .unwrap()
            .contains("视觉"));
        let items =
            visible_actions_with_capabilities(&config, false, &cloud_caps(true, false, false));
        assert!(item(&items, ActionId::ImageSearch).enabled);
    }

    #[test]
    fn cloud_translate_requires_translate_cap() {
        let config = Config::default();
        let items =
            visible_actions_with_capabilities(&config, true, &cloud_caps(true, true, false));
        let translate = item(&items, ActionId::Translate);
        assert!(!translate.enabled);
        assert!(translate
            .disabled_reason
            .as_deref()
            .unwrap()
            .contains("翻译"));
        let items = visible_actions_with_capabilities(&config, true, &cloud_caps(true, true, true));
        assert!(item(&items, ActionId::Translate).enabled);
    }

    #[test]
    fn local_translate_ignores_translate_cap() {
        let mut config = Config::default();
        config.translate = TranslatePrefs {
            backend: TranslateBackend::Local,
            ..TranslatePrefs::default()
        };
        let caps = Capabilities::derive_from_cloud_caps(
            true,
            CloudCaps::default(),
            true, // 本地模型已安装
        );
        let items = visible_actions_with_capabilities(&config, true, &caps);
        assert!(item(&items, ActionId::Translate).enabled);
        // 未安装本地模型 → 仍按本地模型缺失提示。
        let caps = Capabilities::derive_from_cloud_caps(true, CloudCaps::default(), false);
        let items = visible_actions_with_capabilities(&config, true, &caps);
        let translate = item(&items, ActionId::Translate);
        assert!(!translate.enabled);
        assert!(translate
            .disabled_reason
            .as_deref()
            .unwrap()
            .contains("本地模型未下载"));
    }

    #[test]
    fn answer_ignores_specific_caps_but_needs_cloud_key() {
        let config = Config::default();
        let items =
            visible_actions_with_capabilities(&config, true, &cloud_caps(false, false, false));
        // 解答是通用云端能力：不要求 vision/search/translate 任一 cap。
        assert!(item(&items, ActionId::Answer).enabled);
        let no_key = Capabilities::derive_from_cloud_caps(false, CloudCaps::default(), false);
        let items = visible_actions_with_capabilities(&config, true, &no_key);
        assert!(!item(&items, ActionId::Answer).enabled);
    }
}
