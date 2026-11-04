//! 动作注册表与动作菜单可见性（契约 §3 `ActionItem` / §5 / 说明书 M07）。
//!
//! `visible_actions` 是纯函数：输入当前配置与「选区内是否有文字」，输出按配置顺序过滤后的
//! 动作项；不可用项保留在列表中并给出 `disabledReason`（前端置灰 + tooltip）。

use std::collections::HashSet;

use crate::contract::config::Config;
use crate::contract::types::{ActionId, ActionItem, ActionTarget, Mode};

use super::mode::mode_label;

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

/// 动作当前是否可用；不可用时给出中文原因。
///
/// 规则（说明书 §7 / 实施架构 §8）：
/// - `privacy.localOnly` 关闭全部云端动作，但保留浏览器搜索、本地翻译、复制；
/// - 文本上传关闭 → 需要云端文本的动作置灰；
/// - 图像上传关闭 → 图片搜索置灰。
pub fn availability(config: &Config, action: ActionId) -> (bool, Option<String>) {
    match action {
        ActionId::Copy => (true, None),
        ActionId::Search => {
            // 浏览器搜索不上传选区内容，仅在 AI 搜索模式下受开关约束。
            if config.search.mode == crate::contract::config::SearchMode::Browser {
                (true, None)
            } else if config.privacy.local_only {
                (false, Some("已开启仅本地模式，AI 搜索不可用".to_string()))
            } else if !config.privacy.allow_text_upload {
                (false, Some("已关闭文本上传，AI 搜索不可用".to_string()))
            } else {
                (true, None)
            }
        }
        ActionId::ImageSearch => {
            if config.privacy.local_only {
                (false, Some("已开启仅本地模式，图片搜索不可用".to_string()))
            } else if !config.privacy.allow_image_upload {
                (false, Some("已关闭图像上传，图片搜索不可用".to_string()))
            } else {
                (true, None)
            }
        }
        ActionId::Translate => {
            if config.translate.backend == crate::contract::config::TranslateBackend::Local {
                // 本地翻译不上传文本；但需要本地模型就绪（真实实现由 action_router 降级处理）。
                (true, None)
            } else if config.privacy.local_only {
                (false, Some("已开启仅本地模式，云端翻译不可用".to_string()))
            } else if !config.privacy.allow_text_upload {
                (false, Some("已关闭文本上传，云端翻译不可用".to_string()))
            } else {
                (true, None)
            }
        }
        ActionId::Answer => {
            if config.privacy.local_only {
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

/// 根据配置生成动作菜单（契约 §7 `menu.ready` 的 `items`）。
///
/// - `has_text = true` 使用 `actions.menu.text`，否则使用 `actions.menu.image`；
/// - 保留配置顺序，重复项去重；
/// - 不可用动作保留并给出 `disabledReason`。
pub fn visible_actions(config: &Config, has_text: bool) -> Vec<ActionItem> {
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
        let (enabled, disabled_reason) = availability(config, *action);
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
        PrivacyPrefs, SearchMode, SearchPrefs, TranslateBackend, TranslatePrefs,
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
}
