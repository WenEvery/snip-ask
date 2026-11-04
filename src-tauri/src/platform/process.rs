//! 本地推理进程管理（M12 / 说明书 §5.4）。
//!
//! 真实实现（llama.cpp / Ollama / CTranslate2）待接入；当前仅提供 trait 与停用实现。
//!
//! TODO(real): 进程管理（`std::process::Command`，仅 127.0.0.1 端口探测）
//! 1) 固定端口段 39281–39289 探测空闲；
//! 2) 启动 sidecar 并健康检查 `/v1/models`；
//! 3) 空闲 `local.idleUnloadSec` 后卸载；
//! 4) 退出时确保子进程被回收。

use async_trait::async_trait;

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::EngineStatus;

/// 本地引擎监管者。
#[async_trait]
pub trait EngineSupervisor: Send + Sync {
    /// 当前状态。
    fn status(&self) -> EngineStatus;
    /// 启动引擎。
    async fn start(&self) -> AppResult<()>;
    /// 停止引擎。
    async fn stop(&self) -> AppResult<()>;
}

/// 停用实现：永远 `stopped`，`start` 返回 `SA-9001`（骨架模式默认行为）。
#[derive(Debug, Clone, Copy, Default)]
pub struct StoppedSupervisor;

#[async_trait]
impl EngineSupervisor for StoppedSupervisor {
    fn status(&self) -> EngineStatus {
        EngineStatus::Stopped
    }

    async fn start(&self) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::SkeletonNotImplemented,
            Some("TODO(real): 本地推理引擎启动未接入".to_string()),
        ))
    }

    async fn stop(&self) -> AppResult<()> {
        Ok(())
    }
}
