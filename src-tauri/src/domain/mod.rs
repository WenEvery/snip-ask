//! 纯逻辑领域层：无 IO、无 tauri、无 `std::process`，全部可单元测试。
//!
//! 依赖方向：`commands → services → domain`；`domain` 不依赖 `services` / `commands` /
//! `platform`。契约类型来自 `crate::contract`（同一 crate 内的最底层契约层）。

pub mod action;
pub mod ids;
pub mod mode;
pub mod models;
pub mod onboarding;
pub mod rect;
pub mod session;
pub mod task;
pub mod turn;
pub mod usage;
