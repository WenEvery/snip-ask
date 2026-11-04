//! 全局热键（M03 / 契约 §4.3 / §6 `hotkey_probe`）。
//!
//! - 从配置解析热键并注册（`tauri-plugin-global-shortcut`）；
//! - 支持暂停 / 恢复（托盘菜单 `togglePause`）；
//! - `probe` 校验格式、规范化并检测与其它配置项冲突。
//!
//! 已真实编译验证（Tauri 2.12 / Rust 1.99）: 校验 `tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut,
//! ShortcutState}` 的导出与 `event.state()` 名字（Tauri 2 插件）。

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::contract::config::Config;
use crate::contract::errors::AppResult;
use crate::contract::events::TrayActionPayload;
use crate::contract::types::HotkeyStatus;
use crate::events::emit_tray_action;
use crate::platform::hotkey::{normalize_spec, to_tauri_shortcut, validate_spec};

/// 热键动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    /// 开始框选。
    Capture,
    /// 划词。
    WordGrab,
    /// 快速翻译。
    QuickTranslate,
    /// 图片搜索。
    ImageSearch,
    /// 翻译。
    Translate,
    /// 解答。
    Answer,
    /// 暂停 / 恢复热键。
    TogglePause,
    /// 打开设置。
    OpenSettings,
}

impl HotkeyAction {
    /// 托盘事件 id / 诊断用名称。
    pub fn id(&self) -> &'static str {
        match self {
            HotkeyAction::Capture => "capture",
            HotkeyAction::WordGrab => "wordGrab",
            HotkeyAction::QuickTranslate => "quickTranslate",
            HotkeyAction::ImageSearch => "imageSearch",
            HotkeyAction::Translate => "translate",
            HotkeyAction::Answer => "answer",
            HotkeyAction::TogglePause => "togglePause",
            HotkeyAction::OpenSettings => "openSettings",
        }
    }

    /// 该动作在配置中的热键字符串。
    pub fn config_spec<'a>(&self, config: &'a Config) -> &'a str {
        match self {
            HotkeyAction::Capture => config.hotkeys.capture.as_str(),
            HotkeyAction::WordGrab => config.hotkeys.word_grab.as_str(),
            HotkeyAction::QuickTranslate => config.hotkeys.quick_translate.as_str(),
            HotkeyAction::ImageSearch => config.hotkeys.image_search.as_str(),
            HotkeyAction::Translate => config.hotkeys.translate.as_str(),
            HotkeyAction::Answer => config.hotkeys.answer.as_str(),
            HotkeyAction::TogglePause => config.hotkeys.toggle_pause.as_str(),
            HotkeyAction::OpenSettings => config.hotkeys.open_settings.as_str(),
        }
    }
}

/// 全部热键动作。
pub const ACTIONS: [HotkeyAction; 8] = [
    HotkeyAction::Capture,
    HotkeyAction::WordGrab,
    HotkeyAction::QuickTranslate,
    HotkeyAction::ImageSearch,
    HotkeyAction::Translate,
    HotkeyAction::Answer,
    HotkeyAction::TogglePause,
    HotkeyAction::OpenSettings,
];

/// 配置中的热键字段（用于冲突检测）。
pub fn hotkey_fields(config: &Config) -> [(&'static str, &str); 9] {
    [
        ("hotkeys.capture", config.hotkeys.capture.as_str()),
        ("hotkeys.wordGrab", config.hotkeys.word_grab.as_str()),
        (
            "hotkeys.quickTranslate",
            config.hotkeys.quick_translate.as_str(),
        ),
        ("hotkeys.imageSearch", config.hotkeys.image_search.as_str()),
        ("hotkeys.translate", config.hotkeys.translate.as_str()),
        ("hotkeys.answer", config.hotkeys.answer.as_str()),
        ("hotkeys.toggleMode", config.hotkeys.toggle_mode.as_str()),
        ("hotkeys.togglePause", config.hotkeys.toggle_pause.as_str()),
        (
            "hotkeys.openSettings",
            config.hotkeys.open_settings.as_str(),
        ),
    ]
}

/// 按当前配置注册全部热键（失败只记 warn，不阻断启动）。
pub fn register_all(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<crate::app::state::AppState>();
    if state.is_paused() {
        return Ok(());
    }
    let config = state.snapshot_config();
    for action in ACTIONS {
        let spec = action.config_spec(&config);
        if spec.trim().is_empty() {
            continue;
        }
        match to_tauri_shortcut(spec) {
            Some(shortcut) => {
                if let Err(err) = app.global_shortcut().register(shortcut) {
                    tracing::warn!(
                        hotkey = %spec,
                        action = action.id(),
                        error = %err,
                        "热键注册失败（可能被其它程序占用）"
                    );
                }
            }
            None => {
                tracing::warn!(hotkey = %spec, action = action.id(), "热键格式非法，已跳过注册");
            }
        }
    }
    Ok(())
}

/// 注销全部热键。
pub fn unregister_all(app: &AppHandle) {
    if let Err(err) = app.global_shortcut().unregister_all() {
        tracing::warn!(error = %err, "注销热键失败");
    }
}

/// 配置变更后重新注册。
pub fn refresh(app: &AppHandle) -> AppResult<()> {
    unregister_all(app);
    register_all(app)
}

/// 暂停 / 恢复热键。
pub fn set_paused(app: &AppHandle, paused: bool) -> AppResult<()> {
    let state = app.state::<crate::app::state::AppState>();
    state.set_paused(paused);
    if paused {
        unregister_all(app);
        Ok(())
    } else {
        register_all(app)
    }
}

/// 切换暂停状态，返回切换后的 `paused`。
pub fn toggle_pause(app: &AppHandle) -> bool {
    let state = app.state::<crate::app::state::AppState>();
    let next = !state.is_paused();
    let _ = set_paused(app, next);
    next
}

/// 探测快捷键：格式、规范化、与配置内其它项冲突。
pub fn probe(config: &Config, spec: &str) -> HotkeyStatus {
    let valid = validate_spec(spec).is_ok() && normalize_spec(spec).is_some();
    let normalized = normalize_spec(spec);
    let mut conflict_with = None;
    if let Some(normalized_spec) = normalized.as_deref() {
        for (name, value) in hotkey_fields(config) {
            if normalize_spec(value).as_deref() == Some(normalized_spec) {
                conflict_with = Some(name.to_string());
                break;
            }
        }
    }
    HotkeyStatus {
        spec: spec.to_string(),
        valid,
        available: valid && conflict_with.is_none(),
        normalized,
        conflict_with,
    }
}

/// 当前快捷键配置（供设置页展示）。
pub fn current_bindings(config: &Config) -> Vec<(&'static str, String)> {
    hotkey_fields(config)
        .iter()
        .map(|(name, value)| {
            (
                *name,
                normalize_spec(value).unwrap_or_else(|| (*value).to_string()),
            )
        })
        .collect()
}

/// 全局热键回调（在 `lib.rs` 的插件 Builder 中通过闭包转发）。
pub fn on_shortcut(app: &AppHandle, shortcut: &Shortcut, state: ShortcutState) {
    if state != ShortcutState::Pressed {
        return;
    }
    let app_state = app.state::<crate::app::state::AppState>();
    if app_state.is_paused() {
        return;
    }
    let config = app_state.snapshot_config();
    let Some(action) = match_action(&config, shortcut) else {
        return;
    };
    match action {
        HotkeyAction::Capture => {
            let app = app.clone();
            let _handle = tauri::async_runtime::spawn(async move {
                if let Err(err) = crate::commands::capture::do_capture_freeze(&app).await {
                    tracing::warn!(code = %err.code, message = %err.message, "热键触发框选失败");
                }
            });
        }
        HotkeyAction::OpenSettings => {
            if let Err(err) = crate::app::windows::show_main(app) {
                tracing::warn!(error = %err, "打开设置窗口失败");
            }
            emit_tray_action(
                app,
                &TrayActionPayload {
                    id: "openSettings".to_string(),
                },
            );
        }
        HotkeyAction::TogglePause => {
            let _paused = toggle_pause(app);
            emit_tray_action(
                app,
                &TrayActionPayload {
                    id: HotkeyAction::TogglePause.id().to_string(),
                },
            );
        }
        other => emit_tray_action(
            app,
            &TrayActionPayload {
                id: other.id().to_string(),
            },
        ),
    }
}

fn match_action(config: &Config, shortcut: &Shortcut) -> Option<HotkeyAction> {
    ACTIONS.iter().copied().find(|action| {
        let spec = action.config_spec(config);
        if spec.trim().is_empty() {
            return false;
        }
        to_tauri_shortcut(spec)
            .map(|configured| &configured == shortcut)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_detects_conflict_and_normalizes() {
        let config = Config::default();
        let status = probe(&config, "ctrl+q");
        assert!(status.valid);
        assert_eq!(status.normalized.as_deref(), Some("Ctrl+Q"));
        assert!(!status.available);
        assert_eq!(status.conflict_with.as_deref(), Some("hotkeys.capture"));

        let status = probe(&config, "ctrl+shift+y");
        assert!(status.valid);
        assert!(status.available);
        assert!(status.conflict_with.is_none());

        let status = probe(&config, "not-a-hotkey");
        assert!(!status.valid);
        assert!(!status.available);
        assert!(status.normalized.is_none());
    }

    #[test]
    fn actions_map_to_config_fields() {
        let mut config = Config::default();
        config.hotkeys.open_settings = "win+,".to_string();
        assert_eq!(HotkeyAction::Capture.config_spec(&config), "Ctrl+Q");
        assert_eq!(HotkeyAction::OpenSettings.config_spec(&config), "win+,");
        assert_eq!(ACTIONS.len(), 8);
        assert_eq!(hotkey_fields(&config).len(), 9);
    }
}
