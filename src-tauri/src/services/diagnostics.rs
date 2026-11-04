//! 诊断（M18 / 契约 §6 `app_stats`）。
//!
//! `memoryMb` 骨架模式返回 0（TODO(real)：`GetProcessMemoryInfo`）。

use crate::contract::events::IPC_CONTRACT_VERSION;
use crate::contract::types::{EngineStatus, SessionId, Stats};

/// 应用版本（`CARGO_PKG_VERSION`）。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 收集诊断信息。
pub fn collect(
    active_session: Option<SessionId>,
    active_tasks: u32,
    local_engine: EngineStatus,
    models_installed: u32,
    skeleton_mode: bool,
    uptime_sec: u64,
) -> Stats {
    Stats {
        version: APP_VERSION.to_string(),
        contract_version: IPC_CONTRACT_VERSION,
        uptime_sec,
        memory_mb: memory_mb(),
        active_session,
        active_tasks,
        local_engine,
        models_installed,
        skeleton_mode,
    }
}

/// 进程内存占用（MB）。
///
/// TODO(real): `K32GetProcessMemoryInfo` 读取 `WorkingSetSize`。
pub fn memory_mb() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_shape_is_consistent() {
        let stats = collect(
            Some(SessionId::from("s-1")),
            2,
            EngineStatus::Stopped,
            3,
            true,
            42,
        );
        assert_eq!(stats.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(stats.contract_version, 1);
        assert_eq!(stats.uptime_sec, 42);
        assert_eq!(stats.memory_mb, 0);
        assert_eq!(stats.active_tasks, 2);
        assert_eq!(stats.models_installed, 3);
        assert!(stats.skeleton_mode);
        let value = serde_json::to_value(&stats).unwrap();
        assert_eq!(value["contractVersion"], 1);
        assert_eq!(value["activeSession"], "s-1");
        assert_eq!(value["localEngine"], "stopped");
        assert_eq!(value["skeletonMode"].as_bool(), Some(true));
    }
}
