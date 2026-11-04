//! 首启引导（契约 §9）：默认值、步骤编号、草稿合并规则、草稿 → 配置合并。
//!
//! 纯逻辑；文件读写见 `services::onboarding_service`。

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::contract::config::{
    FontSize, Language, Provider, ResultPosition, Theme, TranslateBackend,
};
use crate::contract::errors::{AppError, AppResult};
use crate::contract::types::{
    AiDraft, HotkeysDraft, Mode, OnboardingDraft, OnboardingState, PrivacyDraft,
    TranslateDraft, UiDraft,
};

/// 引导契约版本。
pub const ONBOARDING_VERSION: u32 = 1;
/// 第一步编号（0 欢迎与语言）。
pub const STEP_MIN: i32 = 0;
/// 最后一步编号（5 隐私与完成）。
pub const STEP_MAX: i32 = 5;

/// 六个步骤的标题（契约 §9 固定顺序）。
pub fn step_title(step: i32) -> &'static str {
    match step {
        0 => "欢迎与语言",
        1 => "AI 服务",
        2 => "翻译方式",
        3 => "AI 模式",
        4 => "外观与行为",
        5 => "隐私与完成",
        _ => "未知步骤",
    }
}

/// 新建引导状态（首次启动）。
pub fn new_state(now_iso: &str) -> OnboardingState {
    OnboardingState {
        completed: false,
        skipped: false,
        version: ONBOARDING_VERSION,
        current_step: STEP_MIN,
        started_at: now_iso.to_string(),
        updated_at: now_iso.to_string(),
        data: OnboardingDraft::default(),
    }
}

/// 校验并规范化引导状态（读取磁盘后使用）。
pub fn sanitize(mut state: OnboardingState) -> OnboardingState {
    if state.version == 0 {
        state.version = ONBOARDING_VERSION;
    }
    if state.current_step < STEP_MIN || state.current_step > STEP_MAX {
        state.current_step = STEP_MIN;
    }
    state
}

fn parse_field<T: DeserializeOwned>(data: &Value, field: &str) -> AppResult<Option<T>> {
    match data.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|err| AppError::invalid_config(&format!("data.{field}"), err.to_string())),
    }
}

/// 把某一步提交的 `data` 合并进草稿，并推进/记录 `currentStep`。
///
/// 允许部分字段：缺省字段保持草稿原值。
pub fn apply_step_data(state: &mut OnboardingState, step: i32, data: Value) -> AppResult<()> {
    if !(STEP_MIN..=STEP_MAX).contains(&step) {
        return Err(AppError::invalid_config(
            "step",
            format!("步骤编号必须在 {STEP_MIN}..={STEP_MAX}"),
        ));
    }
    match step {
        0 => {
            if let Some(language) = parse_field::<Language>(&data, "language")? {
                state.data.language = language;
            }
        }
        1 => {
            let draft: AiDraft = serde_json::from_value(data).map_err(|err| {
                AppError::invalid_config("data", format!("AI 服务数据非法：{err}"))
            })?;
            state.data.ai = Some(draft);
        }
        2 => {
            let draft: TranslateDraft = serde_json::from_value(data).map_err(|err| {
                AppError::invalid_config("data", format!("翻译方式数据非法：{err}"))
            })?;
            state.data.translate = Some(draft);
        }
        3 => {
            // 部分字段：search / imageSearch / translate / answer → Mode
            let Some(object) = data.as_object() else {
                return Err(AppError::invalid_config("data", "AI 模式数据必须是对象"));
            };
            for action in ["search", "imageSearch", "translate", "answer"] {
                if let Some(value) = object.get(action) {
                    let mode: Mode = serde_json::from_value(value.clone()).map_err(|err| {
                        AppError::invalid_config(
                            &format!("data.{action}"),
                            format!("模式必须是 quick / deep：{err}"),
                        )
                    })?;
                    state.data.modes.insert(action.to_string(), mode);
                }
            }
        }
        4 => {
            if let Some(theme) = parse_field::<Theme>(&data, "theme")? {
                state.data.ui.theme = theme;
            }
            if let Some(font_size) = parse_field::<FontSize>(&data, "fontSize")? {
                state.data.ui.font_size = font_size;
            }
            if let Some(autostart) = parse_field::<bool>(&data, "autostart")? {
                state.data.ui.autostart = autostart;
            }
            if let Some(position) = parse_field::<ResultPosition>(&data, "resultPosition")? {
                state.data.ui.result_position = position;
            }
            if let Some(capture) = parse_field::<String>(&data, "capture")? {
                state.data.hotkeys.capture = capture;
            }
        }
        5 => {
            if let Some(agreed) = parse_field::<bool>(&data, "agreed")? {
                state.data.privacy.agreed = agreed;
            }
            if let Some(allow) = parse_field::<bool>(&data, "allowTextUpload")? {
                state.data.privacy.allow_text_upload = allow;
            }
            if let Some(allow) = parse_field::<bool>(&data, "allowImageUpload")? {
                state.data.privacy.allow_image_upload = allow;
            }
        }
        _ => {}
    }
    state.current_step = step.clamp(STEP_MIN, STEP_MAX);
    Ok(())
}

/// 标记完成 / 跳过。
pub fn finish(state: &mut OnboardingState, skip: bool, now_iso: &str) {
    state.completed = !skip;
    state.skipped = skip;
    state.version = ONBOARDING_VERSION;
    state.updated_at = now_iso.to_string();
    state.data.finished_at = Some(now_iso.to_string());
}

/// 把草稿中已填项合并进配置（`onboarding_finish({skip:false})`）。
///
/// 只合并「用户明确填过」的字段；返回是否发生了修改。
pub fn merge_into_config(state: &OnboardingState, config: &mut crate::contract::config::Config) -> bool {
    let draft = &state.data;
    let mut changed = false;

    if config.app.language != draft.language {
        config.app.language = draft.language;
        changed = true;
    }
    if config.app.autostart != draft.ui.autostart {
        config.app.autostart = draft.ui.autostart;
        changed = true;
    }
    if config.ui.theme != draft.ui.theme {
        config.ui.theme = draft.ui.theme;
        changed = true;
    }
    if config.ui.font_size != draft.ui.font_size {
        config.ui.font_size = draft.ui.font_size;
        changed = true;
    }
    if config.ui.result_window.position != draft.ui.result_position {
        config.ui.result_window.position = draft.ui.result_position;
        changed = true;
    }

    // 热键：仅当草稿非空且合法时覆盖。
    let capture = draft.hotkeys.capture.trim();
    if !capture.is_empty()
        && crate::contract::config::is_valid_hotkey(capture)
        && config.hotkeys.capture != capture
    {
        if let Some(normalized) = crate::contract::config::normalize_hotkey(capture) {
            config.hotkeys.capture = normalized;
            changed = true;
        }
    }

    // AI 服务：更新同 id 供应商，否则新增。
    if let Some(ai) = &draft.ai {
        let id = ai.provider_id.trim().to_lowercase();
        let valid_id = !id.is_empty()
            && id.len() <= 32
            && id
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_');
        if valid_id {
            let existing = config
                .providers
                .iter()
                .position(|provider| provider.id == id);
            match existing {
                Some(index) => {
                    let provider = &mut config.providers[index];
                    if !ai.base_url.is_empty() && provider.base_url != ai.base_url {
                        provider.base_url = ai.base_url.clone();
                        changed = true;
                    }
                    if !ai.model.is_empty() && provider.model != ai.model {
                        provider.model = ai.model.clone();
                        changed = true;
                    }
                }
                None if !ai.base_url.is_empty() => {
                    config.providers.push(Provider {
                        id: id.clone(),
                        name: if ai.provider_id.is_empty() {
                            id.clone()
                        } else {
                            ai.provider_id.clone()
                        },
                        base_url: ai.base_url.clone(),
                        model: ai.model.clone(),
                        ..Provider::default()
                    });
                    changed = true;
                }
                None => {}
            }
        }
    }

    // 翻译方式。
    if let Some(translate) = &draft.translate {
        if config.translate.backend != translate.backend {
            config.translate.backend = translate.backend;
            changed = true;
        }
        match translate.backend {
            TranslateBackend::Local => {
                if let Some(model_id) = &translate.model_id {
                    if !model_id.is_empty() && &config.local.model != model_id {
                        config.local.model = model_id.clone();
                        changed = true;
                    }
                }
            }
            TranslateBackend::Cloud => {
                if !translate.cloud_model.is_empty()
                    && config.translate.model != translate.cloud_model
                {
                    config.translate.model = translate.cloud_model.clone();
                    changed = true;
                }
            }
        }
    }

    // 各动作默认模式。
    for (key, value) in &draft.modes {
        let slot = match key.as_str() {
            "search" => Some(&mut config.modes.search),
            "imageSearch" => Some(&mut config.modes.image_search),
            "translate" => Some(&mut config.modes.translate),
            "answer" => Some(&mut config.modes.answer),
            _ => None,
        };
        if let Some(slot) = slot {
            if *slot != *value {
                *slot = *value;
                changed = true;
            }
        }
    }

    // 隐私：仅合并两个上传开关（localOnly 由设置页管理）。
    if config.privacy.allow_text_upload != draft.privacy.allow_text_upload {
        config.privacy.allow_text_upload = draft.privacy.allow_text_upload;
        changed = true;
    }
    if config.privacy.allow_image_upload != draft.privacy.allow_image_upload {
        config.privacy.allow_image_upload = draft.privacy.allow_image_upload;
        changed = true;
    }

    changed
}

/// 草稿默认热键（供单测）。
pub fn default_draft_hotkeys() -> HotkeysDraft {
    HotkeysDraft::default()
}

/// 草稿默认隐私（供单测）。
pub fn default_draft_privacy() -> PrivacyDraft {
    PrivacyDraft::default()
}

/// 草稿默认外观（供单测）。
pub fn default_draft_ui() -> UiDraft {
    UiDraft::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::config::Config;

    #[test]
    fn default_state_matches_contract() {
        let state = new_state("2026-11-04T00:00:00Z");
        assert!(!state.completed);
        assert!(!state.skipped);
        assert_eq!(state.version, 1);
        assert_eq!(state.current_step, 0);
        assert_eq!(state.data.hotkeys.capture, "Ctrl+Q");
        assert_eq!(state.data.language, Language::ZhCn);
        assert_eq!(STEP_MAX, 5);
        assert_eq!(step_title(3), "AI 模式");
    }

    #[test]
    fn apply_step_data_merges_partial_fields() {
        let mut state = new_state("2026-11-04T00:00:00Z");
        apply_step_data(&mut state, 0, serde_json::json!({ "language": "en-US" })).unwrap();
        assert_eq!(state.data.language, Language::EnUs);

        apply_step_data(
            &mut state,
            1,
            serde_json::json!({
                "providerId": "deepseek",
                "baseUrl": "https://api.deepseek.com",
                "model": "deepseek-flash",
                "tested": true
            }),
        )
        .unwrap();
        assert_eq!(state.data.ai.as_ref().unwrap().model, "deepseek-flash");

        apply_step_data(
            &mut state,
            3,
            serde_json::json!({ "search": "deep", "answer": "quick" }),
        )
        .unwrap();
        assert_eq!(state.data.modes.get("search"), Some(&Mode::Deep));
        assert_eq!(state.current_step, 3);

        apply_step_data(
            &mut state,
            4,
            serde_json::json!({ "theme": "dark", "fontSize": "lg", "autostart": true, "capture": "alt+space" }),
        )
        .unwrap();
        assert_eq!(state.data.ui.theme, Theme::Dark);
        assert_eq!(state.data.ui.font_size, FontSize::Lg);
        assert!(state.data.ui.autostart);
        assert_eq!(state.data.hotkeys.capture, "alt+space");

        apply_step_data(
            &mut state,
            5,
            serde_json::json!({ "agreed": true, "allowImageUpload": false }),
        )
        .unwrap();
        assert!(state.data.privacy.agreed);
        assert!(!state.data.privacy.allow_image_upload);

        assert!(apply_step_data(&mut state, 9, serde_json::json!({})).is_err());
    }

    #[test]
    fn merge_into_config_applies_draft() {
        let mut state = new_state("2026-11-04T00:00:00Z");
        state.data.language = Language::EnUs;
        state.data.ui.theme = Theme::Dark;
        state.data.ui.autostart = true;
        state.data.hotkeys.capture = "Alt+Space".to_string();
        state.data.ai = Some(AiDraft {
            provider_id: "deepseek".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            model: "deepseek-chat".to_string(),
            tested: true,
        });
        state.data.modes.insert("search".to_string(), Mode::Deep);
        state.data.modes.insert("answer".to_string(), Mode::Deep);
        state.data.privacy.allow_text_upload = false;

        let mut config = Config::default();
        assert!(merge_into_config(&state, &mut config));
        assert_eq!(config.app.language, Language::EnUs);
        assert_eq!(config.ui.theme, Theme::Dark);
        assert!(config.app.autostart);
        assert_eq!(config.hotkeys.capture, "Alt+Space");
        assert_eq!(config.providers[0].model, "deepseek-chat");
        assert_eq!(config.modes.search, Mode::Deep);
        assert_eq!(config.modes.answer, Mode::Deep);
        assert!(!config.privacy.allow_text_upload);

        // 再次合并不产生变化。
        assert!(!merge_into_config(&state, &mut config));
    }

    #[test]
    fn finish_marks_completed_or_skipped() {
        let mut state = new_state("2026-11-04T00:00:00Z");
        finish(&mut state, false, "2026-11-04T01:00:00Z");
        assert!(state.completed);
        assert!(!state.skipped);
        assert_eq!(
            state.data.finished_at.as_deref(),
            Some("2026-11-04T01:00:00Z")
        );

        let mut state = new_state("2026-11-04T00:00:00Z");
        finish(&mut state, true, "2026-11-04T01:00:00Z");
        assert!(!state.completed);
        assert!(state.skipped);
    }

    #[test]
    fn sanitize_clamps_step() {
        let mut state = new_state("2026-11-04T00:00:00Z");
        state.current_step = 99;
        let state = sanitize(state);
        assert_eq!(state.current_step, STEP_MIN);
    }
}
