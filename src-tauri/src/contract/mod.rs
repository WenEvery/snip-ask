//! 对外契约模块（契约 §15）。
//!
//! - `errors`：错误码与 [`errors::AppError`]；
//! - `types`：跨边界基础类型；
//! - `config`：配置模型、校验、迁移、深合并；
//! - `events`：事件名常量与事件载荷。
//!
//! 本模块**不依赖**其余内部模块，任何层都可以依赖它；命令签名只允许使用这里的类型。

pub mod config;
pub mod errors;
pub mod events;
pub mod types;

pub use config::*;
pub use errors::*;
pub use events::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_reexports_are_usable() {
        // 保证各子模块的关键符号能从 contract 根直接使用。
        let _: AppError = AppError::from_code(ErrorCode::Timeout);
        let _: Config = Config::default();
        let _: SessionId = SessionId::from("s".to_string());
        let _: TaskId = TaskId::from("t");
        let _: DownloadId = DownloadId::from("d");
        assert_eq!(IPC_CONTRACT_VERSION, 1);
    }
}
