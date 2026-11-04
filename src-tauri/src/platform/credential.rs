//! 凭据存储（M16 / 契约 §11）。
//!
//! - 目标命名：`SnipAsk/llm/<providerId>`、`SnipAsk/search/<providerId>`、`SnipAsk/mt/<providerId>`；
//! - 骨架模式使用 [`InMemoryCredentialStore`]（进程退出即丢，符合契约 §12）；
//! - 真实 Credential Manager 实现见 [`KeyringCredentialStore`] 的 `TODO(real)`。
//!
//! 安全约束：密钥只在发起请求前读取、用完即弃；不写日志、不进入 Config、不进入事件载荷。

use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;

use crate::contract::errors::{AppError, AppResult, ErrorCode};

/// 凭据引用（决定 Credential Manager Target 命名）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CredentialRef {
    /// LLM 供应商。
    Llm(String),
    /// 搜索服务。
    Search(String),
    /// 专用翻译。
    Mt(String),
}

impl CredentialRef {
    /// LLM 供应商凭据。
    pub fn llm(provider_id: impl Into<String>) -> Self {
        CredentialRef::Llm(provider_id.into())
    }

    /// 搜索服务凭据。
    pub fn search(provider_id: impl Into<String>) -> Self {
        CredentialRef::Search(provider_id.into())
    }

    /// 专用翻译凭据。
    pub fn mt(provider_id: impl Into<String>) -> Self {
        CredentialRef::Mt(provider_id.into())
    }

    /// Credential Manager Target 字符串（契约 §11）。
    pub fn target(&self) -> String {
        match self {
            CredentialRef::Llm(id) => format!("SnipAsk/llm/{id}"),
            CredentialRef::Search(id) => format!("SnipAsk/search/{id}"),
            CredentialRef::Mt(id) => format!("SnipAsk/mt/{id}"),
        }
    }

    /// 用途前缀。
    pub fn scope(&self) -> &'static str {
        match self {
            CredentialRef::Llm(_) => "llm",
            CredentialRef::Search(_) => "search",
            CredentialRef::Mt(_) => "mt",
        }
    }

    /// 供应商 id。
    pub fn provider_id(&self) -> &str {
        match self {
            CredentialRef::Llm(id) | CredentialRef::Search(id) | CredentialRef::Mt(id) => id,
        }
    }
}

impl fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.target())
    }
}

/// 掩码规则（契约 §11）：`"••••" + 后四位`；不足四位或未配置按掩码处理。
pub fn mask_secret(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() < 4 {
        "••••".to_string()
    } else {
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("••••{tail}")
    }
}

/// 凭据存储抽象。
#[async_trait]
pub trait CredentialStore: Send + Sync {
    /// 写入密钥。
    async fn set(&self, target: &str, secret: &str) -> AppResult<()>;
    /// 读取密钥（仅发起请求前调用）。
    async fn get(&self, target: &str) -> AppResult<Option<String>>;
    /// 删除密钥。
    async fn delete(&self, target: &str) -> AppResult<()>;
    /// 掩码（未配置返回 `None`）。
    async fn mask(&self, target: &str) -> AppResult<Option<String>> {
        Ok(self.get(target).await?.as_deref().map(mask_secret))
    }
    /// 是否已配置密钥。
    async fn has_key(&self, target: &str) -> AppResult<bool> {
        Ok(self
            .get(target)
            .await?
            .map(|secret| !secret.is_empty())
            .unwrap_or(false))
    }
}

/// 内存凭据存储（骨架模式默认实现）。
#[derive(Debug, Default)]
pub struct InMemoryCredentialStore {
    entries: parking_lot::Mutex<HashMap<String, String>>,
}

impl InMemoryCredentialStore {
    /// 创建空存储。
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CredentialStore for InMemoryCredentialStore {
    async fn set(&self, target: &str, secret: &str) -> AppResult<()> {
        if secret.is_empty() {
            self.entries.lock().remove(target);
        } else {
            self.entries.lock().insert(target.to_string(), secret.to_string());
        }
        Ok(())
    }

    async fn get(&self, target: &str) -> AppResult<Option<String>> {
        Ok(self.entries.lock().get(target).cloned())
    }

    async fn delete(&self, target: &str) -> AppResult<()> {
        self.entries.lock().remove(target);
        Ok(())
    }
}

/// Windows Credential Manager 实现（真实能力待接入）。
///
/// TODO(real): 使用 `keyring::Entry::new("SnipAsk", target)` 读写
/// `CredWriteW` / `CredReadW`；失败映射为 `SA-5003` 并脱敏。
#[derive(Debug, Default)]
pub struct KeyringCredentialStore {
    _private: (),
}

impl KeyringCredentialStore {
    /// 创建（真实实现接入后改为读取 Credential Manager）。
    pub fn new() -> Self {
        Self { _private: () }
    }
}

#[async_trait]
impl CredentialStore for KeyringCredentialStore {
    async fn set(&self, _target: &str, _secret: &str) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::SkeletonNotImplemented,
            Some("TODO(real): Windows Credential Manager 写入".to_string()),
        ))
    }

    async fn get(&self, _target: &str) -> AppResult<Option<String>> {
        Err(AppError::new(
            ErrorCode::SkeletonNotImplemented,
            Some("TODO(real): Windows Credential Manager 读取".to_string()),
        ))
    }

    async fn delete(&self, _target: &str) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::SkeletonNotImplemented,
            Some("TODO(real): Windows Credential Manager 删除".to_string()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_targets_match_contract() {
        assert_eq!(
            CredentialRef::llm("deepseek").target(),
            "SnipAsk/llm/deepseek"
        );
        assert_eq!(
            CredentialRef::search("bing").target(),
            "SnipAsk/search/bing"
        );
        assert_eq!(CredentialRef::mt("qwen").target(), "SnipAsk/mt/qwen");
        assert_eq!(CredentialRef::llm("x").scope(), "llm");
        assert_eq!(CredentialRef::Mt("x".into()).provider_id(), "x");
    }

    #[test]
    fn mask_rules() {
        assert_eq!(mask_secret("sk-1234567890abcd"), "••••abcd");
        assert_eq!(mask_secret("abcd"), "••••abcd");
        assert_eq!(mask_secret("abc"), "••••");
        assert_eq!(mask_secret(""), "••••");
    }

    #[tokio::test]
    async fn in_memory_store_crud() {
        let store = InMemoryCredentialStore::new();
        let target = CredentialRef::llm("deepseek").target();
        assert!(!store.has_key(&target).await.unwrap());
        assert!(store.get(&target).await.unwrap().is_none());
        assert!(store.mask(&target).await.unwrap().is_none());

        store.set(&target, "sk-1234567890abcd").await.unwrap();
        assert!(store.has_key(&target).await.unwrap());
        assert_eq!(
            store.get(&target).await.unwrap().as_deref(),
            Some("sk-1234567890abcd")
        );
        assert_eq!(store.mask(&target).await.unwrap().as_deref(), Some("••••abcd"));

        store.delete(&target).await.unwrap();
        assert!(!store.has_key(&target).await.unwrap());
    }
}
