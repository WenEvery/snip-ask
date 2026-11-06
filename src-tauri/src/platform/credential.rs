//! 凭据存储（M16 / 契约 §11）。
//!
//! - 目标命名：`SnipAsk/llm/<providerId>`、`SnipAsk/search/<providerId>`、`SnipAsk/mt/<providerId>`；
//! - 生产实现 [`KeyringCredentialStore`] 走 Windows Credential Manager（keyring crate）：
//!   Credential Manager 的 Target 名 = 上面的契约目标（`cmdkey /list` 可见），
//!   User 名 = provider id，Service = `SnipAsk`（spec §3.4）；
//! - 单元测试与运行期兜底用 [`InMemoryCredentialStore`]（进程退出即丢，符合契约 §12）；
//! - 异步 trait 内部的 keyring 同步调用一律放进 `spawn_blocking`，避免阻塞 tokio worker。
//!
//! 安全约束：密钥只在发起请求前读取、用完即弃；不写日志、不进入 Config、不进入事件载荷。
//! 日志与错误 detail 只允许出现「目标名 + 错误类别」，绝不出现明文密钥。

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;

use crate::contract::errors::{AppError, AppResult};

/// keyring service 名（Windows Credential Manager 的 comment 字段 / macOS service）。
pub const KEYRING_SERVICE: &str = "SnipAsk";

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

/// 从凭据目标派生 keyring user（= provider id，target 的最后一段）。
///
/// 例：`SnipAsk/llm/deepseek` → `deepseek`。
pub fn provider_id_for_target(target: &str) -> &str {
    target.rsplit('/').next().unwrap_or(target)
}

/// 目标合法性校验（空目标 / 空 provider id → SA-5001）。
fn validate_target(target: &str) -> AppResult<()> {
    if target.trim().is_empty() {
        return Err(AppError::invalid_config(
            "credential.target",
            "凭据目标不能为空",
        ));
    }
    if provider_id_for_target(target).is_empty() {
        return Err(AppError::invalid_config(
            "credential.target",
            "凭据目标缺少 provider id",
        ));
    }
    Ok(())
}

/// keyring 错误 → 契约错误码。
///
/// - 属性非法 / 超长 → `SA-5001`（provider id 或目标名不合法）；
/// - 平台存储不可访问 / 其他失败 → `SA-5003`（IO 失败；lead 已确认不用 SA-9001）。
///
/// `NoEntry` 由调用方当作「未配置」处理，不进入本函数。
fn map_keyring_error(err: &keyring::Error) -> AppError {
    match err {
        keyring::Error::Invalid(attribute, reason) => AppError::invalid_config(
            "credential.target",
            format!("凭据属性 {attribute} 非法：{reason}"),
        ),
        keyring::Error::TooLong(attribute, limit) => AppError::invalid_config(
            "credential.target",
            format!("凭据属性 {attribute} 超过平台上限 {limit}"),
        ),
        other => AppError::io_failed(format!("Windows Credential Manager 访问失败：{other}")),
    }
}

/// 构造 keyring Entry：Target = 契约目标名，Service = `SnipAsk`，User = provider id。
fn keyring_entry(service: &str, target: &str) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new_with_target(target, service, provider_id_for_target(target))
}

/// 在阻塞线程池执行 keyring 同步调用；JoinError 也映射为 SA-5003。
async fn run_blocking<T, F>(task: F) -> AppResult<T>
where
    F: FnOnce() -> Result<T, keyring::Error> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(map_keyring_error(&err)),
        Err(join) => Err(AppError::io_failed(format!(
            "凭据存储阻塞任务执行失败：{join}"
        ))),
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

/// 内存凭据存储（单测默认实现 / keyring 不可用时的运行期兜底）。
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
            self.entries
                .lock()
                .insert(target.to_string(), secret.to_string());
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

/// Windows Credential Manager 实现（keyring crate，windows-native）。
///
/// - 写入：`CredWriteW`（keyring::Entry::set_password），系统加密后落盘，重启后仍可读；
/// - 读取：`CredReadW`（keyring::Entry::get_password），`NoEntry` → `None`；
/// - 删除：`CredDeleteW`（keyring::Entry::delete_credential），不存在视为成功（幂等）。
///
/// 可选内存兜底（[`KeyringCredentialStore::with_fallback`]）：仅当平台存储**访问失败**
/// （安全策略 / Credential Manager 服务不可用 / 目标名非法）时降级到内存并 `warn`，
/// 保证「配置页不崩、当次会话可用」，绝不把失败伪装成成功写盘。
pub struct KeyringCredentialStore {
    service: String,
    fallback: Option<Arc<dyn CredentialStore>>,
}

impl fmt::Debug for KeyringCredentialStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyringCredentialStore")
            .field("service", &self.service)
            .field("has_fallback", &self.fallback.is_some())
            .finish()
    }
}

impl Default for KeyringCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyringCredentialStore {
    /// 真实存储（无兜底；失败直接返回 SA-5001 / SA-5003）。
    pub fn new() -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
            fallback: None,
        }
    }

    /// 真实存储 + 内存兜底（生产接线用；见 [`default_store`]）。
    pub fn with_fallback(fallback: Arc<dyn CredentialStore>) -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
            fallback: Some(fallback),
        }
    }

    /// service 名（诊断用）。
    pub fn service(&self) -> &str {
        &self.service
    }

    /// 是否配置了兜底。
    pub fn has_fallback(&self) -> bool {
        self.fallback.is_some()
    }

    /// 记录一次降级（只含目标名与错误码，不含密钥）。
    fn note_fallback(&self, op: &str, target: &str, error: &AppError) {
        tracing::warn!(
            op,
            target,
            code = %error.code,
            "Credential Manager 不可用，本次操作降级到内存兜底（不写盘）"
        );
    }

    /// keyring 读失败时的兜底读取。
    async fn read_fallback(&self, target: &str) -> AppResult<Option<String>> {
        match &self.fallback {
            Some(fallback) => fallback.get(target).await,
            None => Ok(None),
        }
    }
}

#[async_trait]
impl CredentialStore for KeyringCredentialStore {
    async fn set(&self, target: &str, secret: &str) -> AppResult<()> {
        validate_target(target)?;
        // 空串等价于删除（与 InMemory 实现保持一致）。
        if secret.is_empty() {
            return self.delete(target).await;
        }
        let service = self.service.clone();
        let target_owned = target.to_string();
        let secret_owned = secret.to_string();
        let result = run_blocking(move || {
            keyring_entry(&service, &target_owned)?.set_password(&secret_owned)
        })
        .await;
        match result {
            Ok(()) => {
                // 真实存储已成功：清掉可能残留的内存兜底，避免降级时读到旧值。
                if let Some(fallback) = &self.fallback {
                    let _ = fallback.delete(target).await;
                }
                Ok(())
            }
            Err(error) => match &self.fallback {
                Some(fallback) => {
                    self.note_fallback("set", target, &error);
                    fallback.set(target, secret).await
                }
                None => Err(error),
            },
        }
    }

    async fn get(&self, target: &str) -> AppResult<Option<String>> {
        validate_target(target)?;
        let service = self.service.clone();
        let target_owned = target.to_string();
        let result = run_blocking(move || {
            let entry = keyring_entry(&service, &target_owned)?;
            match entry.get_password() {
                Ok(secret) => Ok(Some(secret)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(err) => Err(err),
            }
        })
        .await;
        match result {
            Ok(secret) => Ok(secret),
            Err(error) => match &self.fallback {
                Some(_) => {
                    self.note_fallback("get", target, &error);
                    self.read_fallback(target).await
                }
                None => Err(error),
            },
        }
    }

    async fn delete(&self, target: &str) -> AppResult<()> {
        validate_target(target)?;
        let service = self.service.clone();
        let target_owned = target.to_string();
        let result = run_blocking(move || {
            let entry = keyring_entry(&service, &target_owned)?;
            match entry.delete_credential() {
                // 幂等：不存在视为删除成功。
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(err) => Err(err),
            }
        })
        .await;
        match result {
            Ok(()) => {
                if let Some(fallback) = &self.fallback {
                    let _ = fallback.delete(target).await;
                }
                Ok(())
            }
            Err(error) => match &self.fallback {
                Some(fallback) => {
                    self.note_fallback("delete", target, &error);
                    fallback.delete(target).await
                }
                None => Err(error),
            },
        }
    }
}

/// 生产环境默认存储：真实 Windows Credential Manager + 内存兜底。
///
/// Lead 在 `AppState::initialize` 里接线即可（`AppState.credentials` 类型不变）：
/// ```ignore
/// let credentials: Arc<dyn CredentialStore> =
///     crate::platform::credential::default_store();
/// ```
pub fn default_store() -> Arc<dyn CredentialStore> {
    Arc::new(KeyringCredentialStore::with_fallback(Arc::new(
        InMemoryCredentialStore::new(),
    )))
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
        assert_eq!(
            provider_id_for_target(&CredentialRef::llm("deepseek").target()),
            "deepseek"
        );
        assert_eq!(
            provider_id_for_target(&CredentialRef::search("bing").target()),
            "bing"
        );
        assert_eq!(
            provider_id_for_target(&CredentialRef::mt("qwen-mt").target()),
            "qwen-mt"
        );
    }

    #[test]
    fn mask_rules() {
        assert_eq!(mask_secret("sk-1234567890abcd"), "••••abcd");
        assert_eq!(mask_secret("abcd"), "••••abcd");
        assert_eq!(mask_secret("abc"), "••••");
        assert_eq!(mask_secret(""), "••••");
    }

    #[test]
    fn keyring_error_mapping() {
        let invalid = map_keyring_error(&keyring::Error::Invalid(
            "user".to_string(),
            "too long".to_string(),
        ));
        assert_eq!(
            invalid.code,
            crate::contract::errors::ErrorCode::InvalidConfig
        );

        let too_long = map_keyring_error(&keyring::Error::TooLong("user".to_string(), 513));
        assert_eq!(
            too_long.code,
            crate::contract::errors::ErrorCode::InvalidConfig
        );

        let platform = map_keyring_error(&keyring::Error::PlatformFailure(Box::new(
            std::io::Error::other("credential manager unavailable"),
        )));
        assert_eq!(platform.code, crate::contract::errors::ErrorCode::IoFailed);

        let no_access = map_keyring_error(&keyring::Error::NoStorageAccess(Box::new(
            std::io::Error::other("locked"),
        )));
        assert_eq!(no_access.code, crate::contract::errors::ErrorCode::IoFailed);
    }

    #[test]
    fn keyring_store_wiring_shape() {
        // 只验证接线形状（不发起真实 Credential Manager 读写）。
        let real = KeyringCredentialStore::new();
        assert!(!real.has_fallback());
        assert_eq!(real.service(), KEYRING_SERVICE);

        let with_fallback =
            KeyringCredentialStore::with_fallback(Arc::new(InMemoryCredentialStore::new()));
        assert!(with_fallback.has_fallback());
        assert_eq!(with_fallback.service(), KEYRING_SERVICE);

        // `default_store()` 的返回类型必须是 `Arc<dyn CredentialStore>`。
        let _: Arc<dyn CredentialStore> = default_store();
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
        assert_eq!(
            store.mask(&target).await.unwrap().as_deref(),
            Some("••••abcd")
        );

        store.delete(&target).await.unwrap();
        assert!(!store.has_key(&target).await.unwrap());
    }

    #[tokio::test]
    async fn validate_target_rejects_empty() {
        let store = KeyringCredentialStore::new();
        let err = store.set("", "secret").await.unwrap_err();
        assert_eq!(err.code, crate::contract::errors::ErrorCode::InvalidConfig);
        let err = store.get("SnipAsk/llm/").await.unwrap_err();
        assert_eq!(err.code, crate::contract::errors::ErrorCode::InvalidConfig);
    }

    #[tokio::test]
    async fn fallback_engages_when_keyring_rejects_target() {
        // 构造 keyring 侧必然拒绝的目标（user = provider id 超过 WinCred 上限 513），
        // 验证兜底链路：不触碰真实 Credential Manager，也不把失败伪装成写盘成功。
        let fallback = Arc::new(InMemoryCredentialStore::new());
        let store = KeyringCredentialStore::with_fallback(fallback.clone());
        let target = format!("SnipAsk/llm/{}", "x".repeat(600));
        store.set(&target, "sk-fallback-1234").await.unwrap();
        assert_eq!(
            store.get(&target).await.unwrap().as_deref(),
            Some("sk-fallback-1234")
        );
        assert_eq!(
            store.mask(&target).await.unwrap().as_deref(),
            Some("••••1234")
        );
        assert!(fallback.has_key(&target).await.unwrap());
        store.delete(&target).await.unwrap();
        assert!(!store.has_key(&target).await.unwrap());
    }

    // ------------------------------------------------------------------
    // 真实 Windows Credential Manager 证据测试（默认 `#[ignore]`）。
    //
    // 运行方式（每个命令都是一次独立进程，用于证明跨进程持久化）：
    //   cargo test --lib keyring_real_write -- --ignored --exact --nocapture
    //   cmdkey /list | findstr SnipAsk
    //   cargo test --lib keyring_real_read_persisted -- --ignored --exact --nocapture
    //   cargo test --lib keyring_real_delete -- --ignored --exact --nocapture
    //
    // 测试密钥是**假 Key**（sk-t4-*），输出只有掩码，绝不打印明文。
    // ------------------------------------------------------------------

    const REAL_TARGET: &str = "SnipAsk/llm/t4-evidence";
    const REAL_SECRET: &str = "sk-t4-evidence-1234";

    #[tokio::test]
    #[ignore = "写真实 Windows Credential Manager，需 --ignored 手动运行"]
    async fn keyring_real_write() {
        let store = KeyringCredentialStore::new();
        store.delete(REAL_TARGET).await.unwrap();
        store.set(REAL_TARGET, REAL_SECRET).await.unwrap();
        let read_back = store.get(REAL_TARGET).await.unwrap();
        assert_eq!(
            read_back.as_deref(),
            Some(REAL_SECRET),
            "写入后应能立即读回"
        );
        assert!(store.has_key(REAL_TARGET).await.unwrap());
        println!(
            "KEYRING_WRITE_OK target={REAL_TARGET} has_key=true mask={}",
            store.mask(REAL_TARGET).await.unwrap().unwrap_or_default()
        );
    }

    #[tokio::test]
    #[ignore = "读取上一个进程写入的真实 Windows Credential Manager 条目，需 --ignored 手动运行"]
    async fn keyring_real_read_persisted() {
        // 本测试不写入：证明「另一个进程写入后，重启进程仍能读到」。
        let store = KeyringCredentialStore::new();
        let read_back = store.get(REAL_TARGET).await.unwrap();
        assert_eq!(
            read_back.as_deref(),
            Some(REAL_SECRET),
            "模拟重启（新进程）后应从 Credential Manager 读回原值"
        );
        println!(
            "KEYRING_READ_OK（新进程读取） target={REAL_TARGET} has_key=true mask={}",
            store.mask(REAL_TARGET).await.unwrap().unwrap_or_default()
        );
    }

    #[tokio::test]
    #[ignore = "删除真实 Windows Credential Manager 条目并验证已消失，需 --ignored 手动运行"]
    async fn keyring_real_delete() {
        let store = KeyringCredentialStore::new();
        store.delete(REAL_TARGET).await.unwrap();
        assert_eq!(store.get(REAL_TARGET).await.unwrap(), None);
        assert!(!store.has_key(REAL_TARGET).await.unwrap());
        // 幂等：再删一次也成功。
        store.delete(REAL_TARGET).await.unwrap();
        println!("KEYRING_DELETE_OK target={REAL_TARGET} get=None has_key=false");
    }

    #[tokio::test]
    #[ignore = "读取 cmdkey 写入的真实 Windows Credential Manager 条目，需 --ignored 手动运行"]
    async fn keyring_real_read_cmdkey_written() {
        // 由 `.verify/v1/storage/run-storage-evidence.ps1` 先用 cmdkey 写入
        // target=SnipAsk/llm/t4-cmdkey、user=t4-cmdkey、pass=sk-t4-cmdkey-9876，
        // 再用本测试经 keyring 读回，证明两个独立工具共享同一 Windows 凭据存储。
        let target = "SnipAsk/llm/t4-cmdkey";
        let store = KeyringCredentialStore::new();
        let read_back = store.get(target).await.unwrap();
        assert_eq!(
            read_back.as_deref(),
            Some("sk-t4-cmdkey-9876"),
            "keyring 应读到 cmdkey 写入的同名凭据"
        );
        println!(
            "KEYRING_CMDKEY_INTEROP_OK target={target} mask={}",
            store.mask(target).await.unwrap().unwrap_or_default()
        );
        store.delete(target).await.unwrap();
    }
}
