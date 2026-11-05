//! 回环地址 HTTP 客户端的系统代理规避（v1.0.0）。
//!
//! 背景（真机 E2E/门禁实测）：`reqwest` 的 `system-proxy` 会读取 Windows 的 IE/系统代理配置。
//! 部分用户（本机即如此，装了 Steam++/Clash 等）的系统代理**不绕过 127.0.0.1**，导致：
//! - 本地 llama-server（`http://127.0.0.1:<port>`）请求被代理拦截；
//! - 自定义供应商填 `http://127.0.0.1:<port>` 时 `provider_test` 失败；
//! - 依赖回环 mock server 的集成测试在本机假失败。
//!
//! 规则：**URL 主机为回环地址时一律 `no_proxy()`**；其余（真实云端）保留系统代理，
//! 尊重用户网络环境。纯函数便于单测。

use reqwest::ClientBuilder;

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

/// 按目标 URL 构造客户端 builder：回环 → `no_proxy()`，云端 → 沿用系统代理。
pub fn client_builder_for(base_url: &str) -> ClientBuilder {
    let builder = reqwest::Client::builder();
    if is_loopback_url(base_url) {
        builder.no_proxy()
    } else {
        builder
    }
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
    fn builder_is_constructible_for_both_kinds() {
        // 只验证构造路径可走通（代理开关无法在单测断言）。
        assert!(client_builder_for("http://127.0.0.1:1234").build().is_ok());
        assert!(client_builder_for("https://api.deepseek.com")
            .build()
            .is_ok());
    }
}
