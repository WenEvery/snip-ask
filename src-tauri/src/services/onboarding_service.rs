//! 引导状态服务（M15）：`onboarding.json` 读写（原子写）、草稿更新、完成合并。
//!
//! 引导状态独立持久化于 `%APPDATA%\SnipAsk\onboarding.json`，与 `config.json` 分离
//! （契约 §9），避免污染配置 Schema。

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::contract::errors::AppResult;
use crate::contract::types::OnboardingState;
use crate::domain::onboarding;
use crate::platform::paths::AppPaths;

/// 当前时间（RFC3339，UTC）。
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 引导状态服务。
#[derive(Debug, Clone)]
pub struct OnboardingService {
    path: PathBuf,
}

impl OnboardingService {
    /// 由应用路径构造。
    pub fn new(paths: &AppPaths) -> Self {
        Self {
            path: paths.onboarding_file.clone(),
        }
    }

    /// 由文件路径直接构造（测试用）。
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 引导文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 读取引导状态；文件缺失 / 损坏时返回新建状态（不阻断启动）。
    pub fn load(&self) -> OnboardingState {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => match serde_json::from_str::<OnboardingState>(&text) {
                Ok(state) => onboarding::sanitize(state),
                Err(err) => {
                    tracing::warn!(error = %err, "引导状态损坏，使用默认引导状态");
                    onboarding::new_state(&now_iso())
                }
            },
            Err(_) => onboarding::new_state(&now_iso()),
        }
    }

    /// 原子写：`onboarding.json.tmp` → rename。
    pub fn save_atomic(&self, state: &OnboardingState) -> AppResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(state)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, format!("{text}\n"))?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// 写入某一步的草稿数据并即时保存。
    pub fn update(&self, state: &mut OnboardingState, step: i32, data: Value) -> AppResult<()> {
        onboarding::apply_step_data(state, step, data)?;
        state.updated_at = now_iso();
        self.save_atomic(state)
    }

    /// 完成 / 跳过引导并保存。
    pub fn finish(&self, state: &mut OnboardingState, skip: bool) -> AppResult<()> {
        onboarding::finish(state, skip, &now_iso());
        self.save_atomic(state)
    }
}

/// 读取引导状态（命令层便捷函数）。
pub fn load_state(paths: &AppPaths) -> OnboardingState {
    OnboardingService::new(paths).load()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::config::Language;

    fn temp_service() -> OnboardingService {
        let dir =
            std::env::temp_dir().join(format!("snip-ask-onboarding-{}", uuid::Uuid::new_v4()));
        OnboardingService::with_path(dir.join("onboarding.json"))
    }

    #[test]
    fn missing_file_yields_fresh_state() {
        let service = temp_service();
        let state = service.load();
        assert!(!state.completed);
        assert_eq!(state.current_step, 0);
        assert_eq!(state.version, 1);
    }

    #[test]
    fn update_saves_and_roundtrips() {
        let service = temp_service();
        let mut state = service.load();
        service
            .update(&mut state, 0, serde_json::json!({ "language": "en-US" }))
            .unwrap();
        let loaded = service.load();
        assert_eq!(loaded.data.language, Language::EnUs);
        assert_eq!(loaded.current_step, 0);
        assert!(!loaded.started_at.is_empty());

        if let Some(parent) = service.path().parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn finish_marks_state_and_persists() {
        let service = temp_service();
        let mut state = service.load();
        service.finish(&mut state, true).unwrap();
        let loaded = service.load();
        assert!(loaded.skipped);
        assert!(!loaded.completed);
        assert!(loaded.data.finished_at.is_some());
        if let Some(parent) = service.path().parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn invalid_step_is_rejected() {
        let service = temp_service();
        let mut state = service.load();
        let error = service
            .update(&mut state, 42, serde_json::json!({}))
            .unwrap_err();
        assert_eq!(error.code, "SA-5001");
        let _ = std::fs::remove_dir_all(service.path().parent().unwrap_or(Path::new(".")));
    }

    #[test]
    fn corrupt_file_falls_back_to_default() {
        let service = temp_service();
        if let Some(parent) = service.path().parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(service.path(), "{bad json").unwrap();
        let state = service.load();
        assert!(!state.completed);
        let _ = std::fs::remove_dir_all(service.path().parent().unwrap_or(Path::new(".")));
    }
}
