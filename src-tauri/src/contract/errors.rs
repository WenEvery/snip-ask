//! 对外错误模型（契约 §8）。
//!
//! - [`ErrorCode`]：契约 §8.2 的 20 个错误码常量 + 展示名 / 是否可重试 / 建议降级动作；
//! - [`AppError`]：跨 WebView ↔ Rust 边界的统一错误结构，字段与契约 §8.1 逐字一致；
//! - 命令返回值统一为 [`AppResult`]。
//!
//! 本模块只依赖 serde / std，不依赖其余任何内部模块（契约 §15）。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 契约 §8.2 错误码常量集合。
///
/// 常量命名故意与契约码表保持一致（CamelCase），以便与文档对照阅读。
#[allow(non_upper_case_globals)]
pub struct ErrorCode;

#[allow(non_upper_case_globals)]
impl ErrorCode {
    /// 捕获失败 / 黑屏（DRM、全屏独占、权限）。
    pub const CaptureFailed: &'static str = "SA-1001";
    /// 覆盖层创建失败。
    pub const OverlayFailed: &'static str = "SA-1002";
    /// 热键冲突 / 不可注册。
    pub const HotkeyConflict: &'static str = "SA-1003";
    /// 会话状态非法转移 / 会话不存在。
    pub const InvalidState: &'static str = "SA-1099";
    /// OCR 引擎不可用。
    pub const OcrEngineUnavailable: &'static str = "SA-2001";
    /// 识别失败。
    pub const OcrFailed: &'static str = "SA-2002";
    /// 无网络。
    pub const NetworkUnavailable: &'static str = "SA-3001";
    /// 鉴权失败（401）。
    pub const AuthFailed: &'static str = "SA-3002";
    /// 额度不足（402）。
    pub const QuotaExceeded: &'static str = "SA-3003";
    /// 限流（429）。
    pub const RateLimited: &'static str = "SA-3004";
    /// 超时。
    pub const Timeout: &'static str = "SA-3005";
    /// 本地模型缺失。
    pub const ModelMissing: &'static str = "SA-4001";
    /// 内存不足。
    pub const OutOfMemory: &'static str = "SA-4002";
    /// 下载失败。
    pub const DownloadFailed: &'static str = "SA-4003";
    /// 校验失败。
    pub const ChecksumMismatch: &'static str = "SA-4004";
    /// 配置非法。
    pub const InvalidConfig: &'static str = "SA-5001";
    /// 不支持的操作（如明文导出密钥）。
    pub const UnsupportedOperation: &'static str = "SA-5002";
    /// 文件读写失败。
    pub const IoFailed: &'static str = "SA-5003";
    /// 自启注册失败。
    pub const AutostartFailed: &'static str = "SA-5004";
    /// URL 被安全策略拒绝。
    pub const UrlRejected: &'static str = "SA-5005";
    /// 骨架模式未实现的能力（仅内部降级路径用）。
    pub const SkeletonNotImplemented: &'static str = "SA-9001";

    /// 全部错误码（顺序与契约 §8.2 一致），供测试与诊断使用。
    pub const ALL: &'static [&'static str] = &[
        Self::CaptureFailed,
        Self::OverlayFailed,
        Self::HotkeyConflict,
        Self::InvalidState,
        Self::OcrEngineUnavailable,
        Self::OcrFailed,
        Self::NetworkUnavailable,
        Self::AuthFailed,
        Self::QuotaExceeded,
        Self::RateLimited,
        Self::Timeout,
        Self::ModelMissing,
        Self::OutOfMemory,
        Self::DownloadFailed,
        Self::ChecksumMismatch,
        Self::InvalidConfig,
        Self::UnsupportedOperation,
        Self::IoFailed,
        Self::AutostartFailed,
        Self::UrlRejected,
        Self::SkeletonNotImplemented,
    ];

    /// 错误码对应的面向用户中文短句（默认 message）。
    pub fn message(code: &str) -> &'static str {
        match code {
            Self::CaptureFailed => "屏幕捕获失败",
            Self::OverlayFailed => "覆盖层创建失败",
            Self::HotkeyConflict => "快捷键冲突",
            Self::InvalidState => "当前状态不允许该操作",
            Self::OcrEngineUnavailable => "OCR 引擎不可用",
            Self::OcrFailed => "文字识别失败",
            Self::NetworkUnavailable => "网络不可用",
            Self::AuthFailed => "API Key 无效",
            Self::QuotaExceeded => "额度不足",
            Self::RateLimited => "请求过于频繁",
            Self::Timeout => "请求超时",
            Self::ModelMissing => "本地模型缺失",
            Self::OutOfMemory => "内存不足",
            Self::DownloadFailed => "模型下载失败",
            Self::ChecksumMismatch => "文件校验失败",
            Self::InvalidConfig => "配置非法",
            Self::UnsupportedOperation => "不支持的操作",
            Self::IoFailed => "文件读写失败",
            Self::AutostartFailed => "开机自启设置失败",
            Self::UrlRejected => "链接被安全策略拒绝",
            Self::SkeletonNotImplemented => "骨架模式尚未实现该能力",
            _ => "未知错误",
        }
    }

    /// 错误码展示名（契约 §8.2 第二列）。
    pub fn display_name(code: &str) -> &'static str {
        match code {
            Self::CaptureFailed => "CaptureFailed",
            Self::OverlayFailed => "OverlayFailed",
            Self::HotkeyConflict => "HotkeyConflict",
            Self::InvalidState => "InvalidState",
            Self::OcrEngineUnavailable => "OcrEngineUnavailable",
            Self::OcrFailed => "OcrFailed",
            Self::NetworkUnavailable => "NetworkUnavailable",
            Self::AuthFailed => "AuthFailed",
            Self::QuotaExceeded => "QuotaExceeded",
            Self::RateLimited => "RateLimited",
            Self::Timeout => "Timeout",
            Self::ModelMissing => "ModelMissing",
            Self::OutOfMemory => "OutOfMemory",
            Self::DownloadFailed => "DownloadFailed",
            Self::ChecksumMismatch => "ChecksumMismatch",
            Self::InvalidConfig => "InvalidConfig",
            Self::UnsupportedOperation => "UnsupportedOperation",
            Self::IoFailed => "IoFailed",
            Self::AutostartFailed => "AutostartFailed",
            Self::UrlRejected => "UrlRejected",
            Self::SkeletonNotImplemented => "SkeletonNotImplemented",
            _ => "Unknown",
        }
    }

    /// 该错误码是否默认可重试（契约 §8.2 第四列）。
    pub fn retryable(code: &str) -> bool {
        matches!(
            code,
            Self::CaptureFailed
                | Self::OverlayFailed
                | Self::OcrFailed
                | Self::NetworkUnavailable
                | Self::RateLimited
                | Self::Timeout
                | Self::DownloadFailed
                | Self::ChecksumMismatch
                | Self::IoFailed
                | Self::AutostartFailed
        )
    }

    /// 该错误码的默认降级动作（契约 §8.2 第五列）。
    pub fn fallback(code: &str) -> &'static [&'static str] {
        match code {
            Self::CaptureFailed => &["wgc", "retry"],
            Self::OverlayFailed => &["retry"],
            Self::HotkeyConflict => &["change_hotkey", "pause_hotkey"],
            Self::OcrEngineUnavailable => &["image_path"],
            Self::OcrFailed => &["retry", "image_path"],
            Self::NetworkUnavailable => &["browser_search", "local_translate"],
            Self::AuthFailed => &["open_settings"],
            Self::QuotaExceeded => &["open_settings"],
            Self::RateLimited => &["retry"],
            Self::Timeout => &["retry", "browser_search"],
            Self::ModelMissing => &["download_model", "cloud_translate"],
            Self::OutOfMemory => &["light_model", "cloud_translate"],
            Self::DownloadFailed => &["mirror", "manual_download"],
            Self::ChecksumMismatch => &["redownload"],
            Self::InvalidConfig => &["reset_group"],
            Self::AutostartFailed => &["open_settings"],
            _ => &[],
        }
    }
}

/// 统一错误结构（契约 §8.1）。
///
/// 序列化为 camelCase；`Option` 字段序列化为 `null`（不加 skip，保证前端字段恒在）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    /// 错误码，如 `"SA-3002"`。
    pub code: String,
    /// 面向用户的中文短句。
    pub message: String,
    /// 技术细节（脱敏后）。
    pub detail: Option<String>,
    /// 是否可重试。
    pub retryable: bool,
    /// 可尝试的降级动作，如 `["browser_search"]`。
    pub fallback: Vec<String>,
    /// 附加上下文（脱敏）。
    pub context: Option<serde_json::Value>,
}

impl AppError {
    /// 按错误码构造，自动补全 message / retryable / fallback。
    pub fn new(code: &str, detail: Option<String>) -> Self {
        Self {
            code: code.to_string(),
            message: ErrorCode::message(code).to_string(),
            detail,
            retryable: ErrorCode::retryable(code),
            fallback: ErrorCode::fallback(code)
                .iter()
                .map(|item| (*item).to_string())
                .collect(),
            context: None,
        }
    }

    /// 仅按错误码构造（无技术细节）。
    pub fn from_code(code: &str) -> Self {
        Self::new(code, None)
    }

    /// 覆盖面向用户的 message。
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// 补全技术细节。
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// 覆盖降级动作列表。
    pub fn with_fallback<I, S>(mut self, fallback: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.fallback = fallback.into_iter().map(|item| item.into()).collect();
        self
    }

    /// 覆盖是否可重试。
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// 附加结构化上下文。
    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = Some(context);
        self
    }

    /// 会话状态非法转移（SA-1099）。
    pub fn invalid_state(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidState, Some(detail.into()))
    }

    /// 配置非法（SA-5001），并带上出错字段名。
    pub fn invalid_config(field: &str, reason: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::InvalidConfig,
            Some(format!("字段 {}：{}", field, reason.into())),
        )
        .with_message(format!("配置非法：{}", field))
        .with_context(serde_json::json!({ "field": field }))
    }

    /// 文件读写失败（SA-5003）。
    pub fn io_failed(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::IoFailed, Some(detail.into()))
    }

    /// 骨架模式未实现（SA-9001）。
    pub fn skeleton_not_implemented(detail: impl Into<String>) -> Self {
        Self::new(ErrorCode::SkeletonNotImplemented, Some(detail.into()))
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)?;
        if let Some(detail) = &self.detail {
            write!(f, "（{}）", detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::new(ErrorCode::IoFailed, Some(err.to_string()))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::new(ErrorCode::InvalidConfig, Some(err.to_string()))
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::new(ErrorCode::IoFailed, Some(err.to_string()))
    }
}

/// 命令 / 服务的统一返回类型。
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_table_has_all_twenty_codes() {
        assert_eq!(ErrorCode::ALL.len(), 21);
        assert_eq!(ErrorCode::CaptureFailed, "SA-1001");
        assert_eq!(ErrorCode::InvalidState, "SA-1099");
        assert_eq!(ErrorCode::SkeletonNotImplemented, "SA-9001");
        for code in ErrorCode::ALL {
            assert!(!ErrorCode::message(code).is_empty());
            assert_ne!(ErrorCode::display_name(code), "Unknown");
        }
    }

    #[test]
    fn app_error_serializes_camel_case_with_six_fields() {
        let error = AppError::new(ErrorCode::Timeout, Some("请求超过 30s".into()))
            .with_fallback(vec!["retry", "browser_search"])
            .with_context(serde_json::json!({ "host": "api.deepseek.com" }));
        let value = serde_json::to_value(&error).unwrap();
        assert_eq!(value["code"], "SA-3005");
        assert_eq!(value["message"], "请求超时");
        assert_eq!(value["detail"], "请求超过 30s");
        assert_eq!(value["retryable"].as_bool(), Some(true));
        assert_eq!(value["fallback"][0], "retry");
        assert_eq!(value["context"]["host"], "api.deepseek.com");
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 6);
    }

    #[test]
    fn app_error_option_fields_serialize_as_null() {
        let error = AppError::from_code(ErrorCode::InvalidState);
        let value = serde_json::to_value(&error).unwrap();
        assert!(value["detail"].is_null());
        assert!(value["context"].is_null());
        assert_eq!(value["fallback"].as_array().unwrap().len(), 0);
    }
}
