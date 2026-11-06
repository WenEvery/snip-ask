//! 配置中心（M14）：加载 / 原子写 / 迁移 / 深合并 / 导入导出 / 重置 / 热生效分类。
//!
//! 事件发送由命令层完成（实施架构 §4：`AppHandle` 只出现在 `commands`/`app`/`events`），
//! 本服务只负责状态与文件，返回 `changedKeys` 供命令层广播 `config:changed`。
//!
//! 契约 §4.3：配置落盘为原子写（`*.tmp` → rename），保留上一份有效备份 `config.json.bak`。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::contract::config::{
    merge_patch, migrate, validate, Config, ConfigView, CONFIG_SCHEMA_VERSION,
};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::platform::credential::CredentialRef;
use crate::platform::credential::CredentialStore;
use crate::platform::paths::AppPaths;

/// 加载结果（失败时回退默认配置并记录原因）。
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    /// 生效配置。
    pub config: Config,
    /// 是否发生了损坏配置恢复。
    pub recovered: bool,
    /// 诊断信息（写日志用，不含敏感内容）。
    pub notes: Vec<String>,
}

/// 配置中心。
#[derive(Debug, Clone)]
pub struct ConfigCenter {
    paths: AppPaths,
}

impl ConfigCenter {
    /// 创建配置中心。
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    /// 配置文件路径。
    pub fn config_path(&self) -> &Path {
        &self.paths.config_file
    }

    /// 加载配置（永不失败：损坏时备份并回退默认值）。
    ///
    /// v1.1.0：加载后把 `advanced.proxy` 注入 `platform::http` 的进程级代理
    /// （该配置「需重启应用」生效，因此只在启动加载时应用一次）。
    pub fn load(&self) -> LoadedConfig {
        let loaded = self.load_inner();
        // 测试环境：全局代理是进程级状态，串行化注入以避免并行单测互相干扰（生产无额外开销）。
        #[cfg(test)]
        let _proxy_guard = crate::platform::http::proxy_test_guard();
        crate::platform::http::configure_proxy(loaded.config.advanced.proxy.as_deref());
        loaded
    }

    fn load_inner(&self) -> LoadedConfig {
        let mut notes = Vec::new();
        if !self.paths.config_file.is_file() {
            let config = Config::default();
            if let Err(err) = self.save_atomic(&config) {
                notes.push(format!("首次写入默认配置失败：{err}"));
            }
            return LoadedConfig {
                config,
                recovered: false,
                notes,
            };
        }

        let text = match std::fs::read_to_string(&self.paths.config_file) {
            Ok(text) => text,
            Err(err) => {
                notes.push(format!("读取配置失败（使用默认配置）：{err}"));
                return LoadedConfig {
                    config: Config::default(),
                    recovered: true,
                    notes,
                };
            }
        };

        let parsed = serde_json::from_str::<Value>(&text)
            .map_err(AppError::from)
            .and_then(migrate);
        match parsed {
            Ok(config) => LoadedConfig {
                config,
                recovered: false,
                notes,
            },
            Err(err) => {
                notes.push(format!("配置损坏（使用默认配置）：{err}"));
                let invalid_path = self.backup_invalid_config();
                if let Some(path) = invalid_path {
                    notes.push(format!("已备份损坏配置到 {}", path.display()));
                }
                let config = Config::default();
                if let Err(err) = self.save_atomic(&config) {
                    notes.push(format!("写入默认配置失败：{err}"));
                }
                LoadedConfig {
                    config,
                    recovered: true,
                    notes,
                }
            }
        }
    }

    fn backup_invalid_config(&self) -> Option<PathBuf> {
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
        let target = self
            .paths
            .config_file
            .with_extension(format!("json.invalid-{stamp}"));
        std::fs::rename(&self.paths.config_file, &target)
            .ok()
            .map(|_| target)
    }

    /// 原子写：`config.json.tmp` → rename，并保留 `config.json.bak`。
    pub fn save_atomic(&self, config: &Config) -> AppResult<()> {
        // 落盘前再校验一次，防止绕过命令层写入非法配置。
        validate(config)?;
        if let Some(parent) = self.paths.config_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(config)?;
        let tmp_path = self.paths.config_file.with_extension("json.tmp");
        std::fs::write(&tmp_path, format!("{text}\n"))?;
        if self.paths.config_file.is_file() {
            let backup_path = self.paths.config_file.with_extension("json.bak");
            if let Err(err) = std::fs::copy(&self.paths.config_file, &backup_path) {
                tracing::warn!(error = %err, "备份上一份配置失败");
            }
        }
        std::fs::rename(&tmp_path, &self.paths.config_file)?;
        Ok(())
    }

    /// 构造 `ConfigView`：providers 换成带密钥状态视图（hasKey / keyMask）。
    pub async fn snapshot(&self, config: &Config, credentials: &dyn CredentialStore) -> ConfigView {
        let mut statuses = Vec::with_capacity(config.providers.len());
        for provider in &config.providers {
            let target = CredentialRef::Llm(provider.id.clone()).target();
            let has_key = credentials.has_key(&target).await.unwrap_or(false);
            let key_mask = if has_key {
                credentials.mask(&target).await.ok().flatten()
            } else {
                None
            };
            statuses.push((has_key, key_mask));
        }
        ConfigView::from_config(config, &statuses)
    }

    /// 深合并 patch → 校验 → 原子持久化；校验失败不修改任何状态。
    ///
    /// 返回新的 `ConfigView` 与 `changedKeys`（点分路径，如 `ui.theme`）。
    pub async fn patch(
        &self,
        config: &mut Config,
        patch: Value,
        credentials: &dyn CredentialStore,
    ) -> AppResult<(ConfigView, Vec<String>)> {
        let old = config.clone();
        let mut candidate = config.clone();
        merge_patch(&mut candidate, patch)?;
        self.save_atomic(&candidate)?;
        let changed = classify_change(&old, &candidate);
        *config = candidate;
        let view = self.snapshot(config, credentials).await;
        Ok((view, changed))
    }

    /// 导入：读取 → 迁移 → 校验 → 整份替换（密钥不在配置中，天然保留本机密钥）。
    pub async fn import(
        &self,
        config: &mut Config,
        path: &Path,
        credentials: &dyn CredentialStore,
    ) -> AppResult<(ConfigView, Vec<String>)> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| AppError::io_failed(format!("读取 {} 失败：{err}", path.display())))?;
        let value: Value = serde_json::from_str(&text)?;
        let candidate = migrate(value)?;
        let old = config.clone();
        self.save_atomic(&candidate)?;
        let changed = classify_change(&old, &candidate);
        *config = candidate;
        let view = self.snapshot(config, credentials).await;
        Ok((view, changed))
    }

    /// 导出：默认写入数据目录 `exports/config-<timestamp>.json`；不含密钥。
    pub fn export(
        &self,
        config: &Config,
        path: Option<&Path>,
        include_secrets: bool,
    ) -> AppResult<PathBuf> {
        if include_secrets {
            return Err(AppError::new(
                ErrorCode::UnsupportedOperation,
                Some("v1 不支持导出明文密钥（契约 §4.4）".to_string()),
            )
            .with_message("不支持导出密钥"));
        }
        let target = match path {
            Some(path) => path.to_path_buf(),
            None => {
                std::fs::create_dir_all(&self.paths.export_dir)?;
                let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
                self.paths.export_dir.join(format!("config-{stamp}.json"))
            }
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(config)?;
        std::fs::write(&target, format!("{text}\n"))?;
        Ok(target)
    }

    /// 重置：`scope = "all"` 或契约 §4.1 顶层分组名。
    pub async fn reset(
        &self,
        config: &mut Config,
        scope: &str,
        credentials: &dyn CredentialStore,
    ) -> AppResult<(ConfigView, Vec<String>)> {
        let patch = reset_patch(scope)?;
        self.patch(config, patch, credentials).await
    }
}

/// 生成重置 patch（`all` → 完整默认配置；分组 → 该分组默认值）。
pub fn reset_patch(scope: &str) -> AppResult<Value> {
    let defaults = serde_json::to_value(Config::default())?;
    if scope == "all" {
        return Ok(defaults);
    }
    let groups = [
        "app",
        "ui",
        "hotkeys",
        "actions",
        "providers",
        "modes",
        "search",
        "translate",
        "local",
        "ocr",
        "privacy",
        "history",
        "advanced",
    ];
    if !groups.contains(&scope) {
        return Err(AppError::invalid_config(
            "scope",
            format!("未知分组 {scope}；允许 all 或 {groups:?}"),
        ));
    }
    let group_default = defaults
        .get(scope)
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let mut patch = serde_json::Map::new();
    patch.insert(scope.to_string(), group_default);
    Ok(serde_json::Value::Object(patch))
}

/// 对比两份配置，输出变更键（点分路径；数组整体变更输出分组路径）。
pub fn classify_change(old: &Config, new: &Config) -> Vec<String> {
    let old_value = serde_json::to_value(old).unwrap_or(serde_json::Value::Null);
    let new_value = serde_json::to_value(new).unwrap_or(serde_json::Value::Null);
    let mut changed = Vec::new();
    collect_changes("", &old_value, &new_value, &mut changed);
    changed.sort();
    changed.dedup();
    changed
}

fn collect_changes(
    prefix: &str,
    old: &serde_json::Value,
    new: &serde_json::Value,
    out: &mut Vec<String>,
) {
    match (old, new) {
        (serde_json::Value::Object(old_map), serde_json::Value::Object(new_map)) => {
            let mut keys: BTreeSet<&String> = BTreeSet::new();
            keys.extend(old_map.keys());
            keys.extend(new_map.keys());
            for key in keys {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match (old_map.get(key), new_map.get(key)) {
                    (Some(old_value), Some(new_value)) => {
                        collect_changes(&path, old_value, new_value, out)
                    }
                    _ => out.push(path),
                }
            }
        }
        (serde_json::Value::Array(_), serde_json::Value::Array(_)) => {
            if old != new {
                out.push(prefix.to_string());
            }
        }
        _ => {
            if old != new {
                out.push(prefix.to_string());
            }
        }
    }
}

/// 热生效分类（契约 §4.4）：返回该变更需要的生效方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyKind {
    /// 即时生效。
    Immediate,
    /// 需要重新注册（热键 / 自启）。
    ReRegister,
    /// 需要重启本地引擎。
    RestartEngine,
    /// 需要重启应用。
    RestartApp,
}

/// 根据变更键判断生效方式（命令层据此触发重新注册 / 重启引擎）。
pub fn classify_apply(changed_keys: &[String]) -> Vec<ApplyKind> {
    let mut kinds = Vec::new();
    for key in changed_keys {
        let kind = if key.starts_with("hotkeys.") || key == "app.autostart" {
            ApplyKind::ReRegister
        } else if key.starts_with("local.")
            || key == "translate.backend"
            || key.starts_with("translate.providerId")
        {
            ApplyKind::RestartEngine
        } else if key == "advanced.proxy" {
            ApplyKind::RestartApp
        } else {
            ApplyKind::Immediate
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    kinds
}

/// 配置 schema 版本（命令层诊断）。
pub fn schema_version() -> u32 {
    CONFIG_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::config::Theme;
    use crate::platform::credential::InMemoryCredentialStore;

    fn center() -> ConfigCenter {
        let paths = AppPaths::from_roots(
            true,
            std::env::temp_dir().join("snip-ask-config-test"),
            Path::new("C:/tmp/config"),
            Path::new("C:/tmp/local"),
        )
        .unwrap();
        ConfigCenter::new(paths)
    }

    #[test]
    fn classify_change_reports_leaf_paths() {
        let old = Config::default();
        let mut new = Config::default();
        new.ui.theme = Theme::Dark;
        new.hotkeys.capture = "Alt+Space".to_string();
        new.privacy.local_only = true;
        let changed = classify_change(&old, &new);
        assert!(changed.contains(&"ui.theme".to_string()));
        assert!(changed.contains(&"hotkeys.capture".to_string()));
        assert!(changed.contains(&"privacy.localOnly".to_string()));
        assert!(!changed.contains(&"app".to_string()));

        // 数组整体变更只报告分组路径。
        let mut new = Config::default();
        new.providers[0].model = "deepseek-chat".to_string();
        let changed = classify_change(&old, &new);
        assert!(changed.contains(&"providers".to_string()));
    }

    #[test]
    fn reset_patch_for_group_and_all() {
        let mut config = Config::default();
        config.ui.theme = Theme::Dark;
        let patch = reset_patch("ui").unwrap();
        merge_patch(&mut config, patch).unwrap();
        assert_eq!(config.ui.theme, Theme::System);

        assert!(reset_patch("unknown").is_err());
        assert!(reset_patch("all").is_ok());
    }

    #[test]
    fn apply_classification_matches_contract() {
        let kinds = classify_apply(&[
            "ui.theme".to_string(),
            "hotkeys.capture".to_string(),
            "local.port".to_string(),
            "advanced.proxy".to_string(),
        ]);
        assert!(kinds.contains(&ApplyKind::Immediate));
        assert!(kinds.contains(&ApplyKind::ReRegister));
        assert!(kinds.contains(&ApplyKind::RestartEngine));
        assert!(kinds.contains(&ApplyKind::RestartApp));
    }

    #[tokio::test]
    async fn save_then_load_roundtrip() {
        let paths = AppPaths::from_roots(
            true,
            std::env::temp_dir().join(format!("snip-ask-cfg-{}", uuid::Uuid::new_v4())),
            Path::new("C:/tmp/config"),
            Path::new("C:/tmp/local"),
        )
        .unwrap();
        let _ = std::fs::create_dir_all(&paths.data_dir);
        let center = ConfigCenter::new(paths.clone());
        let mut config = Config::default();
        config.ui.theme = Theme::Dark;
        center.save_atomic(&config).unwrap();
        assert!(paths.config_file.is_file());

        let loaded = center.load();
        assert!(!loaded.recovered);
        assert_eq!(loaded.config.ui.theme, Theme::Dark);

        // 损坏配置 → 备份 + 回退默认。
        std::fs::write(&paths.config_file, "{not json").unwrap();
        let loaded = center.load();
        assert!(loaded.recovered);
        assert_eq!(loaded.config, Config::default());
        assert!(!loaded.notes.is_empty());

        let _ = std::fs::remove_dir_all(&paths.data_dir);
    }

    #[tokio::test]
    async fn patch_persists_and_reports_changed_keys() {
        let paths = AppPaths::from_roots(
            true,
            std::env::temp_dir().join(format!("snip-ask-cfg-{}", uuid::Uuid::new_v4())),
            Path::new("C:/tmp/config"),
            Path::new("C:/tmp/local"),
        )
        .unwrap();
        let _ = std::fs::create_dir_all(&paths.data_dir);
        let center = ConfigCenter::new(paths.clone());
        let credentials = InMemoryCredentialStore::new();
        let mut config = Config::default();
        let (view, changed) = center
            .patch(
                &mut config,
                serde_json::json!({ "ui": { "theme": "dark" } }),
                &credentials,
            )
            .await
            .unwrap();
        assert_eq!(view.ui.theme, Theme::Dark);
        assert_eq!(changed, vec!["ui.theme".to_string()]);
        assert_eq!(config.ui.theme, Theme::Dark);

        // 非法 patch 不修改状态。
        let err = center
            .patch(
                &mut config,
                serde_json::json!({ "ui": { "theme": 1 } }),
                &credentials,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "SA-5001");
        assert_eq!(config.ui.theme, Theme::Dark);

        let _ = std::fs::remove_dir_all(&paths.data_dir);
    }

    #[test]
    fn export_rejects_secrets() {
        let center = center();
        let error = center.export(&Config::default(), None, true).unwrap_err();
        assert_eq!(error.code, "SA-5002");
    }
}
