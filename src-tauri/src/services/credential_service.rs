//! 凭据服务（M16）：Provider 密钥 CRUD + 掩码 + 目标命名。
//!
//! 密钥不进入 `Config` / 事件载荷 / 日志；`ConfigView.providers[].keyMask` 由本服务提供。

use std::sync::Arc;

use crate::contract::errors::AppResult;
use crate::platform::credential::{CredentialRef, CredentialStore};

/// 凭据服务。
#[derive(Clone)]
pub struct CredentialService {
    store: Arc<dyn CredentialStore>,
}

impl CredentialService {
    /// 注入凭据存储（骨架模式为 `InMemoryCredentialStore`）。
    pub fn new(store: Arc<dyn CredentialStore>) -> Self {
        Self { store }
    }

    /// 底层存储（配置中心构造 `ConfigView` 时直接使用）。
    pub fn store(&self) -> &Arc<dyn CredentialStore> {
        &self.store
    }

    /// LLM 供应商凭据目标（`SnipAsk/llm/<providerId>`）。
    pub fn target_for_llm(provider_id: &str) -> String {
        CredentialRef::Llm(provider_id.to_string()).target()
    }

    /// 掩码规则：`••••` + 后四位。
    pub fn mask(secret: &str) -> String {
        crate::platform::credential::mask_secret(secret)
    }

    /// 写入供应商密钥（空串等价于删除）。
    pub async fn set_provider_key(&self, provider_id: &str, secret: &str) -> AppResult<()> {
        let target = Self::target_for_llm(provider_id);
        if secret.is_empty() {
            self.store.delete(&target).await
        } else {
            self.store.set(&target, secret).await
        }
    }

    /// 删除供应商密钥。
    pub async fn delete_provider_key(&self, provider_id: &str) -> AppResult<()> {
        self.store.delete(&Self::target_for_llm(provider_id)).await
    }

    /// 读取供应商密钥（仅发起请求前调用）。
    pub async fn provider_key(&self, provider_id: &str) -> AppResult<Option<String>> {
        self.store.get(&Self::target_for_llm(provider_id)).await
    }

    /// 供应商密钥掩码；未配置返回 `None`。
    pub async fn provider_key_mask(&self, provider_id: &str) -> AppResult<Option<String>> {
        self.store.mask(&Self::target_for_llm(provider_id)).await
    }

    /// 是否已配置供应商密钥。
    pub async fn has_provider_key(&self, provider_id: &str) -> AppResult<bool> {
        self.store.has_key(&Self::target_for_llm(provider_id)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::credential::InMemoryCredentialStore;

    #[tokio::test]
    async fn provider_key_lifecycle() {
        let service = CredentialService::new(Arc::new(InMemoryCredentialStore::new()));
        assert!(!service.has_provider_key("deepseek").await.unwrap());
        assert!(service.provider_key("deepseek").await.unwrap().is_none());
        assert!(service
            .provider_key_mask("deepseek")
            .await
            .unwrap()
            .is_none());

        service
            .set_provider_key("deepseek", "sk-1234567890abcd")
            .await
            .unwrap();
        assert!(service.has_provider_key("deepseek").await.unwrap());
        assert_eq!(
            service
                .provider_key_mask("deepseek")
                .await
                .unwrap()
                .as_deref(),
            Some("••••abcd")
        );

        service.delete_provider_key("deepseek").await.unwrap();
        assert!(!service.has_provider_key("deepseek").await.unwrap());
    }

    #[test]
    fn target_and_mask_helpers() {
        assert_eq!(
            CredentialService::target_for_llm("deepseek"),
            "SnipAsk/llm/deepseek"
        );
        assert_eq!(CredentialService::mask("sk-abcdefgh"), "••••efgh");
    }
}
