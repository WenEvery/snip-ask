//! 全局热键（M03 / 契约 §4.3 / §6 `hotkey_probe`）。
//!
//! - 从配置解析热键并注册（`tauri-plugin-global-shortcut`）；
//! - 支持暂停 / 恢复（托盘菜单 `togglePause`）；
//! - `probe` 校验格式、规范化并检测与其它配置项冲突（纯函数，无 OS 访问）；
//! - `probe_os` 在 `probe` 之上做 **OS 级占用探测**：用插件 `register` / `unregister`
//!   临时注册一次（免 `unsafe`），失败即判定被其它程序或系统占用，返回
//!   `conflictWith = "system"`（契约 `HotkeyStatus.conflictWith` 的等价标记，见
//!   `docs/spikes/S-04-*.md` 映射表）。
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

/// 全部热键动作（v1.0.6 起只保留**真实可用**的动作）。
///
/// 已移除：`wordGrab`（需读取其它应用选中文字，未实现）、`translate`（与默认动作重复）、
/// `toggleMode`（无对应运行期状态）。配置字段仍保留兼容，但不再注册/展示。
pub const ACTIONS: [HotkeyAction; 6] = [
    HotkeyAction::Capture,
    HotkeyAction::QuickTranslate,
    HotkeyAction::ImageSearch,
    HotkeyAction::Answer,
    HotkeyAction::TogglePause,
    HotkeyAction::OpenSettings,
];

/// 配置中的热键字段（仅**生效中**的动作；用于冲突检测与设置页提示）。
pub fn hotkey_fields(config: &Config) -> [(&'static str, &str); 6] {
    [
        ("hotkeys.capture", config.hotkeys.capture.as_str()),
        (
            "hotkeys.quickTranslate",
            config.hotkeys.quick_translate.as_str(),
        ),
        ("hotkeys.imageSearch", config.hotkeys.image_search.as_str()),
        ("hotkeys.answer", config.hotkeys.answer.as_str()),
        ("hotkeys.togglePause", config.hotkeys.toggle_pause.as_str()),
        (
            "hotkeys.openSettings",
            config.hotkeys.open_settings.as_str(),
        ),
    ]
}

/// 按当前配置注册热键（失败只记 warn，不阻断启动）。
///
/// 暂停状态下只注册 [`HotkeyAction::TogglePause`]：其余热键（含 `capture`）全部释放给
/// 其它程序，而「暂停 / 恢复热键」本身保留，才能用同一个热键恢复（否则暂停后只能托盘恢复）。
pub fn register_all(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<crate::app::state::AppState>();
    let paused = state.is_paused();
    let config = state.snapshot_config();
    for action in actions_for_registration(paused) {
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

/// 需要注册的热键动作集合：暂停时仅保留 `togglePause`（用于热键恢复），其余全部释放。
pub fn actions_for_registration(paused: bool) -> impl Iterator<Item = HotkeyAction> {
    ACTIONS
        .into_iter()
        .filter(move |action| !paused || *action == HotkeyAction::TogglePause)
}

/// 注销全部热键。
pub fn unregister_all(app: &AppHandle) {
    if let Err(err) = app.global_shortcut().unregister_all() {
        tracing::warn!(error = %err, "注销热键失败");
    }
}

/// 配置变更后重新注册（暂停时只恢复 `togglePause`，见 [`register_all`]）。
pub fn refresh(app: &AppHandle) -> AppResult<()> {
    unregister_all(app);
    register_all(app)
}

/// 暂停 / 恢复热键：先全部注销，再按暂停状态注册（暂停时仅 `togglePause`）。
pub fn set_paused(app: &AppHandle, paused: bool) -> AppResult<()> {
    let state = app.state::<crate::app::state::AppState>();
    state.set_paused(paused);
    unregister_all(app);
    register_all(app)
}

/// 切换暂停状态，返回切换后的 `paused`。
pub fn toggle_pause(app: &AppHandle) -> bool {
    let state = app.state::<crate::app::state::AppState>();
    let next = !state.is_paused();
    let _ = set_paused(app, next);
    next
}

/// 与 `normalized` 相同的配置内热键字段（按 `hotkey_fields` 顺序，可能多项）。
///
/// 用途：`probe` 取第一项作为冲突提示；`probe_os` 用数量区分「自探测」与「真重复绑定」。
pub fn conflict_fields(config: &Config, normalized: &str) -> Vec<&'static str> {
    hotkey_fields(config)
        .into_iter()
        .filter(|(_, value)| normalize_spec(value).as_deref() == Some(normalized))
        .map(|(name, _)| name)
        .collect()
}

/// 探测快捷键：格式、规范化、与配置内其它项冲突（不访问 OS）。
pub fn probe(config: &Config, spec: &str) -> HotkeyStatus {
    let valid = validate_spec(spec).is_ok() && normalize_spec(spec).is_some();
    let normalized = normalize_spec(spec);
    let conflict_with = normalized
        .as_deref()
        .and_then(|normalized_spec| conflict_fields(config, normalized_spec).first().copied())
        .map(str::to_string);
    HotkeyStatus {
        spec: spec.to_string(),
        valid,
        available: valid && conflict_with.is_none(),
        normalized,
        conflict_with,
    }
}

/// `HotkeyStatus.conflictWith` 的 OS 占用等价标记（契约字段语义内的扩展值）。
///
/// `"system"`：OS 级 `RegisterHotKey` 被拒绝（其它程序/系统已占用该组合）。
/// 与「配置内冲突」（值为 `hotkeys.*` 字段名）严格区分，前端映射见 S-04 文档。
pub const SYSTEM_CONFLICT: &str = "system";

/// OS 级探测结果（依赖注入，便于单测覆盖全部映射分支）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsProbe {
    /// 本应用当前已注册该绑定（正在生效），视为可用。
    RegisteredByApp,
    /// 键在系统上空闲（临时 register 成功，调用方已立即 unregister）。
    Free,
    /// 键被其它程序或系统占用（临时 register 返回 Err）。
    Occupied,
}

/// 探测快捷键（纯逻辑）：格式 / 规范化 / 配置内冲突 / OS 占用。
///
/// 判定优先级（与 OS 真实状态对齐）：
/// 1. 格式非法 → `valid = false`；
/// 2. 本应用已注册该绑定 → `available = true`（当前生效的键；唯一例外：配置内
///    同一组合被 **≥2 个字段**重复绑定，说明是用户刚录入的重复键，仍报配置内冲突）；
/// 3. 未被本应用注册，则临时 register/unregister：
///    - 失败（`Occupied`）→ `available = false`、`conflictWith = "system"`；
///    - 成功（`Free`）→ 若配置内已有同键绑定（例如应用已暂停、该键未注册）则报
///      配置内冲突字段名，否则 `available = true`。
///
/// 已知局限：`hotkey_probe` 只有 `spec` 入参、拿不到「正在编辑哪个字段」，
/// 因此当探测键恰好被应用当前持有、且只命中另一个动作的绑定时，会按「自探测」
/// 判为可用；真正的重复绑定（同键 ≥2 字段）仍可检出。详见 S-04 文档。
pub fn probe_os_with<F>(config: &Config, spec: &str, os: F) -> HotkeyStatus
where
    F: FnOnce(Shortcut) -> OsProbe,
{
    let mut status = probe(config, spec);
    if !status.valid {
        return status;
    }
    let Some(shortcut) = to_tauri_shortcut(spec) else {
        // 理论上不可达：valid 已保证可转成插件 Shortcut；保险起见按非法处理。
        status.valid = false;
        status.available = false;
        status.normalized = None;
        return status;
    };
    let conflicts = status
        .normalized
        .as_deref()
        .map(|normalized| conflict_fields(config, normalized))
        .unwrap_or_default();
    match os(shortcut) {
        OsProbe::RegisteredByApp => {
            if conflicts.len() >= 2 {
                // 同一组合被多个动作重复绑定：即使当前由本应用注册，也应提示改键。
                status.available = false;
                status.conflict_with = conflicts.first().map(|field| (*field).to_string());
            } else {
                // 自探测 / 唯一绑定：当前生效，视为可用。
                status.available = true;
                status.conflict_with = None;
            }
        }
        OsProbe::Free => {
            if let Some(field) = conflicts.first() {
                status.available = false;
                status.conflict_with = Some((*field).to_string());
            } else {
                status.available = true;
                status.conflict_with = None;
            }
        }
        OsProbe::Occupied => {
            status.available = false;
            status.conflict_with = Some(SYSTEM_CONFLICT.to_string());
        }
    }
    status
}

/// OS 级探测（真实实现）：`is_registered` → 已持有；否则临时 `register` + `unregister`。
///
/// 注意：临时注册会短暂占用该键（毫秒级），随后立即释放；不会误注销应用自身绑定
/// （已注册分支直接返回，不重复 register/unregister）。
pub fn probe_os(app: &AppHandle, config: &Config, spec: &str) -> HotkeyStatus {
    probe_os_with(config, spec, |shortcut| {
        let manager = app.global_shortcut();
        if manager.is_registered(shortcut) {
            return OsProbe::RegisteredByApp;
        }
        let started = std::time::Instant::now();
        match manager.register(shortcut) {
            Ok(()) => {
                let elapsed_us = started.elapsed().as_micros();
                if let Err(err) = manager.unregister(shortcut) {
                    tracing::warn!(error = %err, "OS 级探测：临时注册后注销失败");
                }
                tracing::debug!(spec = %spec, elapsed_us, "OS 级探测：键空闲");
                OsProbe::Free
            }
            Err(err) => {
                let elapsed_us = started.elapsed().as_micros();
                tracing::info!(
                    spec = %spec,
                    elapsed_us,
                    error = %err,
                    "OS 级探测：注册被拒绝（判定为被其它程序/系统占用）"
                );
                OsProbe::Occupied
            }
        }
    })
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
    let config = app_state.snapshot_config();
    let Some(action) = match_action(&config, shortcut) else {
        return;
    };
    // 暂停时其它热键均已释放；只有「暂停 / 恢复热键」保留并用于恢复，见 `register_all`。
    if app_state.is_paused() && action != HotkeyAction::TogglePause {
        return;
    }
    match action {
        HotkeyAction::Capture => {
            app_state.set_pending_action(None);
            let app = app.clone();
            let _handle = tauri::async_runtime::spawn(async move {
                if let Err(err) = crate::commands::capture::do_capture_freeze(&app).await {
                    tracing::warn!(code = %err.code, message = %err.message, "热键触发框选失败");
                }
            });
        }
        HotkeyAction::QuickTranslate => {
            // 剪贴板文本 → 直接翻译任务（不截图、不弹框选）。
            let text = crate::platform::clipboard::get_text();
            if text.trim().is_empty() {
                tracing::warn!("快速翻译热键：剪贴板为空，已忽略");
                return;
            }
            let app = app.clone();
            let _handle = tauri::async_runtime::spawn(async move {
                let state = app.state::<crate::app::state::AppState>();
                if let Err(err) = crate::commands::action::execute_text_action(
                    &app,
                    &state,
                    crate::contract::types::ActionId::Translate,
                    text,
                )
                .await
                {
                    tracing::warn!(
                        code = %err.code,
                        message = %err.message,
                        "快速翻译热键执行失败"
                    );
                }
            });
        }
        HotkeyAction::ImageSearch | HotkeyAction::Answer => {
            // 带动作框选：捕获 → 用户框选 → selection_commit 自动执行该动作。
            let action = if action == HotkeyAction::ImageSearch {
                crate::contract::types::ActionId::ImageSearch
            } else {
                crate::contract::types::ActionId::Answer
            };
            app_state.set_pending_action(Some(action));
            let app = app.clone();
            let _handle = tauri::async_runtime::spawn(async move {
                if let Err(err) = crate::commands::capture::do_capture_freeze(&app).await {
                    app.state::<crate::app::state::AppState>()
                        .set_pending_action(None);
                    tracing::warn!(
                        code = %err.code,
                        message = %err.message,
                        "热键触发框选失败（已清除待执行动作）"
                    );
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
        assert_eq!(ACTIONS.len(), 6);
        assert_eq!(hotkey_fields(&config).len(), 6);
        // v1.0.6：不再注册/展示无效热键（划词翻译 / 翻译选区 / 切换模式）。
        assert!(!ACTIONS.contains(&HotkeyAction::WordGrab));
        assert!(!ACTIONS.contains(&HotkeyAction::Translate));
        let fields: Vec<&str> = hotkey_fields(&config)
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert!(!fields.contains(&"hotkeys.wordGrab"));
        assert!(!fields.contains(&"hotkeys.translate"));
        assert!(!fields.contains(&"hotkeys.toggleMode"));
    }

    fn default_config() -> Config {
        Config::default() // 默认 hotkeys.capture = "Ctrl+Q"，其余为空
    }

    #[test]
    fn probe_os_free_key_is_available() {
        let config = default_config();
        for spec in ["Ctrl+Shift+Y", "Alt+Shift+9", "ctrl+shift+y"] {
            let status = probe_os_with(&config, spec, |_| OsProbe::Free);
            assert!(status.valid, "{spec}");
            assert!(status.available, "{spec}");
            assert_eq!(status.conflict_with, None, "{spec}");
        }
        // 规范化输入
        let status = probe_os_with(&config, "ctrl+shift+y", |_| OsProbe::Free);
        assert_eq!(status.normalized.as_deref(), Some("Ctrl+Shift+Y"));
    }

    #[test]
    fn probe_os_internal_conflict_reported_when_key_not_held() {
        let config = default_config();
        // 键空闲（例如应用已暂停）：Ctrl+Q 仍与 capture 配置重复 → 报字段名，不报 system
        let status = probe_os_with(&config, "ctrl+q", |_| OsProbe::Free);
        assert!(status.valid);
        assert!(!status.available);
        assert_eq!(status.conflict_with.as_deref(), Some("hotkeys.capture"));
        assert_ne!(status.conflict_with.as_deref(), Some(SYSTEM_CONFLICT));
    }

    #[test]
    fn probe_os_registered_binding_is_available() {
        let config = default_config();
        // 应用当前持有 Ctrl+Q（默认 capture 绑定）：自探测视为可用
        let status = probe_os_with(&config, "Ctrl+Q", |_| OsProbe::RegisteredByApp);
        assert!(status.valid);
        assert!(status.available);
        assert_eq!(status.conflict_with, None);
    }

    #[test]
    fn probe_os_occupied_maps_to_system() {
        let config = default_config();
        // 未被应用注册 + OS 注册失败 → 被其它程序占用（含应用启动时注册失败、QQ 先占的场景）
        let status = probe_os_with(&config, "Ctrl+Q", |_| OsProbe::Occupied);
        assert!(status.valid);
        assert!(!status.available);
        assert_eq!(status.conflict_with.as_deref(), Some(SYSTEM_CONFLICT));

        // 无配置内冲突的键同样映射为 system
        let status = probe_os_with(&config, "Alt+Shift+9", |_| OsProbe::Occupied);
        assert!(!status.available);
        assert_eq!(status.conflict_with.as_deref(), Some(SYSTEM_CONFLICT));
    }

    #[test]
    fn probe_os_duplicate_binding_detected_even_if_registered() {
        let mut config = default_config();
        // 同一组合被 capture / imageSearch 重复绑定 → 即使应用当前持有该键，也应报配置内冲突
        config.hotkeys.image_search = "ctrl+q".to_string();
        let status = probe_os_with(&config, "Ctrl+Q", |_| OsProbe::RegisteredByApp);
        assert!(!status.available);
        assert_eq!(status.conflict_with.as_deref(), Some("hotkeys.capture"));

        let status = probe_os_with(&config, "Ctrl+Q", |_| OsProbe::Free);
        assert!(!status.available);
        assert_eq!(status.conflict_with.as_deref(), Some("hotkeys.capture"));
    }

    #[test]
    fn probe_os_invalid_skips_os_probe() {
        let config = default_config();
        let called = std::cell::Cell::new(false);
        let status = probe_os_with(&config, "not-a-hotkey", |_| {
            called.set(true);
            OsProbe::Free
        });
        assert!(!status.valid);
        assert!(!status.available);
        assert!(status.normalized.is_none());
        assert!(!called.get(), "非法格式不应触发 OS 探测");

        let called = std::cell::Cell::new(false);
        let status = probe_os_with(&config, "", |_| {
            called.set(true);
            OsProbe::Free
        });
        assert!(!status.valid);
        assert!(!called.get(), "空串（未绑定）不应触发 OS 探测");
    }

    #[test]
    fn paused_keeps_only_toggle_pause_hotkey() {
        // 暂停时其余热键全部释放（含 capture），仅保留「暂停 / 恢复」热键以便热键恢复。
        let paused: Vec<HotkeyAction> = actions_for_registration(true).collect();
        assert_eq!(paused, vec![HotkeyAction::TogglePause]);
        assert_eq!(actions_for_registration(false).count(), ACTIONS.len());
    }

    #[test]
    fn probe_os_mapping_matrix_is_total() {
        // 覆盖 (格式, 配置内冲突, OS 探测) 的完整映射矩阵，断言 100% 归一到契约语义。
        let mut duplicated = default_config();
        duplicated.hotkeys.image_search = "Ctrl+Q".to_string();
        let cases: [(Config, &str, OsProbe, bool, Option<&str>); 10] = [
            (default_config(), "Ctrl+Shift+Y", OsProbe::Free, true, None),
            (
                default_config(),
                "Ctrl+Shift+Y",
                OsProbe::Occupied,
                false,
                Some(SYSTEM_CONFLICT),
            ),
            (
                default_config(),
                "Ctrl+Shift+Y",
                OsProbe::RegisteredByApp,
                true,
                None,
            ),
            (
                default_config(),
                "Ctrl+Q",
                OsProbe::Free,
                false,
                Some("hotkeys.capture"),
            ),
            (
                default_config(),
                "Ctrl+Q",
                OsProbe::Occupied,
                false,
                Some(SYSTEM_CONFLICT),
            ),
            (
                default_config(),
                "Ctrl+Q",
                OsProbe::RegisteredByApp,
                true,
                None,
            ),
            (
                duplicated.clone(),
                "Ctrl+Q",
                OsProbe::RegisteredByApp,
                false,
                Some("hotkeys.capture"),
            ),
            (
                duplicated,
                "Ctrl+Q",
                OsProbe::Free,
                false,
                Some("hotkeys.capture"),
            ),
            (
                default_config(),
                "Alt+Shift+9",
                OsProbe::Occupied,
                false,
                Some(SYSTEM_CONFLICT),
            ),
            (default_config(), "Alt+Shift+9", OsProbe::Free, true, None),
        ];
        for (config, spec, os, expect_available, expect_conflict) in cases {
            let status = probe_os_with(&config, spec, |_| os);
            assert!(status.valid, "{spec} 应为合法格式");
            assert_eq!(
                status.available, expect_available,
                "spec={spec} os={os:?} 的 available 映射不符"
            );
            assert_eq!(
                status.conflict_with.as_deref(),
                expect_conflict,
                "spec={spec} os={os:?} 的 conflictWith 映射不符"
            );
        }
    }
}
