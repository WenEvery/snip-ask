//! 回环地址 HTTP 客户端的系统代理规避 + `advanced.proxy` 显式代理（v1.0.0 / v1.1.0）。
//!
//! 背景（真机 E2E/门禁实测）：`reqwest` 的 `system-proxy` 会读取 Windows 的 IE/系统代理配置。
//! 部分用户（本机即如此，装了 Steam++/Clash 等）的系统代理**不绕过 127.0.0.1**，导致：
//! - 本地 llama-server（`http://127.0.0.1:<port>`）请求被代理拦截；
//! - 自定义供应商填 `http://127.0.0.1:<port>` 时 `provider_test` 失败；
//! - 依赖回环 mock server 的集成测试在本机假失败。
//!
//! 规则（v1.1.0 扩展）：
//! 1. **URL 主机为回环地址时一律 `no_proxy()`**（`advanced.proxy` 也不作用于回环）；
//! 2. 其余云端目标：配置了 `advanced.proxy` → 显式使用该代理（支持 `http://user:pass@host:port`）；
//!    未配置 → 沿用系统代理（尊重用户网络环境）。
//!
//! `advanced.proxy` 是「需重启应用」配置：启动时由 [`ConfigCenter`](crate::services::config_center)
//! 加载后调用 [`configure_proxy`] 注入进程级全局值；单测可用 [`client_builder_with_proxy`]
//! 显式传入代理（不依赖全局状态）。
//!
//! 覆盖范围：LLM 网关（`local_gateway` / `translator`）、搜索/工具网关（`search_gateway`）、
//! `provider_test`、模型下载（`model_manager`）、后端下载（`backend_manager`）。

use parking_lot::RwLock;
use reqwest::ClientBuilder;

/// 进程级 `advanced.proxy`（`None` = 未配置，云端沿用系统代理）。
static CONFIGURED_PROXY: RwLock<Option<String>> = RwLock::new(None);

/// URL 是否指向回环地址（`127.0.0.1` / `localhost` / `::1` / `[::1]`）。
pub fn is_loopback_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.trim_matches(|ch| ch == '[' || ch == ']');
    host.eq_ignore_ascii_case("127.0.0.1")
        || host.eq_ignore_ascii_case("localhost")
        || host == "::1"
}

/// 校验并规范化 `advanced.proxy`。
///
/// - 空串 / 纯空白 → `Ok(None)`（视为未配置）；
/// - 合法 → `Ok(Some(trimmed))`（`http://host:port` / `https://host:port`，可带 user:pass）；
/// - 非法 → `Err(可操作中文原因)`（由配置校验转换为 `SA-5001`）。
pub fn parse_proxy(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    crate::contract::config::validate_proxy_url(trimmed)?;
    Ok(Some(trimmed.to_string()))
}

/// 注入进程级代理（启动时由配置加载调用；`advanced.proxy` 需重启应用生效）。
///
/// 非法值只记录告警并清空（不 panic）——配置保存路径已由 [`crate::contract::config::validate`]
/// 拦截非法值，这里仅作防御。
pub fn configure_proxy(raw: Option<&str>) {
    let parsed = match raw {
        Some(value) => match parse_proxy(value) {
            Ok(parsed) => parsed,
            Err(reason) => {
                tracing::warn!(reason = %reason, "advanced.proxy 非法，已忽略并使用系统代理");
                None
            }
        },
        None => None,
    };
    *CONFIGURED_PROXY.write() = parsed;
}

/// 当前进程级代理（`None` = 未配置）。
pub fn configured_proxy() -> Option<String> {
    CONFIGURED_PROXY.read().clone()
}

/// 目标 URL 实际应使用的代理：回环地址返回 `None`（永不代理），其余返回配置值。
pub fn proxy_for_url<'a>(base_url: &str, configured: Option<&'a str>) -> Option<&'a str> {
    if is_loopback_url(base_url) {
        return None;
    }
    configured.map(str::trim).filter(|proxy| !proxy.is_empty())
}

/// 按目标 URL 构造客户端 builder：回环 → `no_proxy()`；云端 → 显式代理或系统代理。
pub fn client_builder_for(base_url: &str) -> ClientBuilder {
    let proxy = configured_proxy();
    client_builder_with_proxy(base_url, proxy.as_deref())
}

/// 同 [`client_builder_for`]，但显式传入代理（下载 / 单测等无全局状态的调用方使用）。
///
/// 回环目标始终 `no_proxy()`；云端目标代理地址非法时回退系统代理（记录告警，不 panic）。
pub fn client_builder_with_proxy(base_url: &str, proxy: Option<&str>) -> ClientBuilder {
    let builder = reqwest::Client::builder();
    if is_loopback_url(base_url) {
        return builder.no_proxy();
    }
    let Some(proxy_url) = proxy_for_url(base_url, proxy) else {
        return builder;
    };
    match reqwest::Proxy::all(proxy_url) {
        Ok(proxy) => builder.proxy(proxy),
        Err(err) => {
            tracing::warn!(error = %err, "advanced.proxy 无法应用，已回退系统代理");
            builder
        }
    }
}

/// 测试串行化锁：全局代理是进程级状态，避免并行单测互相覆盖。
#[cfg(test)]
static PROXY_TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// 获取测试串行化锁（`ConfigCenter::load` 与代理单测共用，保证「注入 → 断言」原子性）。
#[cfg(test)]
pub(crate) fn proxy_test_guard() -> parking_lot::MutexGuard<'static, ()> {
    PROXY_TEST_LOCK.lock()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_loopback_hosts() {
        assert!(is_loopback_url("http://127.0.0.1:39281"));
        assert!(is_loopback_url(
            "http://127.0.0.1:39281/v1/chat/completions"
        ));
        assert!(is_loopback_url("http://localhost:1420/"));
        assert!(is_loopback_url("http://[::1]:8080/"));
        assert!(!is_loopback_url("https://api.deepseek.com"));
        assert!(!is_loopback_url("https://127.0.0.1.example.com"));
        assert!(!is_loopback_url("not a url"));
    }

    #[test]
    fn parses_and_validates_proxy_urls() {
        // 空 / 空白 → 未配置。
        assert_eq!(parse_proxy("").unwrap(), None);
        assert_eq!(parse_proxy("   ").unwrap(), None);
        // 合法：http / https / user:pass。
        assert_eq!(
            parse_proxy(" http://127.0.0.1:7890 ").unwrap().as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert!(parse_proxy("https://proxy.local:8443").unwrap().is_some());
        assert!(parse_proxy("http://user:pass@proxy.local:8080")
            .unwrap()
            .is_some());
        // 非法：缺 scheme / 非 HTTP(S) / 无主机。
        assert!(parse_proxy("127.0.0.1:7890").is_err());
        assert!(parse_proxy("socks5://127.0.0.1:1080").is_err());
        assert!(parse_proxy("http://").is_err());
    }

    #[test]
    fn loopback_never_uses_configured_proxy() {
        let proxy = Some("http://127.0.0.1:7890");
        assert_eq!(proxy_for_url("http://127.0.0.1:39281", proxy), None);
        assert_eq!(proxy_for_url("http://localhost:1420/", proxy), None);
        assert_eq!(proxy_for_url("http://[::1]:8080/", proxy), None);
        // 云端：返回配置的代理；未配置 → None。
        assert_eq!(
            proxy_for_url("https://api.deepseek.com", proxy),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(proxy_for_url("https://api.deepseek.com", None), None);
        assert_eq!(proxy_for_url("https://api.deepseek.com", Some("  ")), None);
    }

    #[test]
    fn builder_is_constructible_for_both_kinds() {
        // 只验证构造路径可走通（代理开关无法在单测断言）。
        for url in ["http://127.0.0.1:1234", "https://api.deepseek.com"] {
            assert!(
                client_builder_with_proxy(url, Some("http://127.0.0.1:7890"))
                    .build()
                    .is_ok()
            );
            assert!(client_builder_with_proxy(url, None).build().is_ok());
            assert!(client_builder_for(url).build().is_ok());
        }
        // 非法代理地址不 panic（回退系统代理）。
        assert!(
            client_builder_with_proxy("https://api.deepseek.com", Some("not a url"))
                .build()
                .is_ok()
        );
    }

    #[test]
    fn global_proxy_can_be_injected_and_cleared() {
        // `ConfigCenter::load` 的代理注入与本测试共用同一把锁，保证断言原子性。
        let _guard = proxy_test_guard();
        configure_proxy(Some("http://127.0.0.1:7890"));
        assert_eq!(configured_proxy().as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(
            proxy_for_url("https://api.deepseek.com", configured_proxy().as_deref()),
            Some("http://127.0.0.1:7890")
        );
        // 回环目标即使配置了代理也不使用。
        assert_eq!(
            proxy_for_url("http://127.0.0.1:39281", configured_proxy().as_deref()),
            None
        );
        // 非法值被拒绝 → 不生效（不 panic）。
        configure_proxy(Some("not a url"));
        assert_eq!(configured_proxy(), None);
        // user:pass 代理可注入。
        configure_proxy(Some("http://user:pass@proxy.local:8080"));
        assert!(configured_proxy().is_some());
        configure_proxy(None);
        assert_eq!(configured_proxy(), None);
    }
}
