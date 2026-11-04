//! 能力编排层（M04–M18）。
//!
//! 依赖方向：`commands → services → domain`；`services` 依赖 `platform` trait，
//! 不直接感知 Tauri 窗口细节（事件由命令层 / 任务执行器统一发送）。
//!
//! 所有外部能力都有骨架实现（`Skeleton*`），行为见契约 §12；
//! 真实实现以 `// TODO(real): ...` 标注并保持同一 trait。

pub mod action_router;
pub mod capture_service;
pub mod config_center;
pub mod credential_service;
pub mod diagnostics;
pub mod history_repo;
pub mod llm_gateway;
pub mod model_manager;
pub mod ocr_service;
pub mod onboarding_service;
pub mod prompt_engine;
pub mod search_gateway;
pub mod task_runner;
pub mod translator;
pub mod updater_service;
pub mod usage_meter;
