//! 命令层（契约 §6 全部 29 个命令）。
//!
//! 每个命令都薄：校验参数 → 调用 service → 返回 contract 类型；错误统一 `AppResult<T>`。
//! 命令注册在 `src/lib.rs` 的 `tauri::generate_handler![...]` 中一次性完成
//! （Tauri 2 要求 handler 列表为编译期宏，故不在本模块提供 `handler()` 函数）。
//!
//! 本模块同时 `pub use` 全部命令，便于从 `commands::xxx` 直接引用与测试；
//! `generate_handler!` 仍使用 `commands::<域>::<命令>` 完整路径。
//!
//! 命令清单：
//! 1. `capture_freeze` / 2. `capture_cancel`
//! 3. `selection_commit` / 4. `selection_reset`
//! 5. `menu_execute` / 6. `task_cancel` / 7. `task_followup` / 8. `result_close`
//! 9. `config_get` / 10. `config_patch` / 11. `config_import` / 12. `config_export` / 13. `config_reset`
//! 14. `onboarding_state` / 15. `onboarding_update` / 16. `onboarding_finish`
//! 17. `provider_test`
//! 18. `models_list` / 19. `models_download` / 20. `models_cancel` / 21. `models_remove`
//! 22. `translate_test`
//! 23. `hotkey_probe` / 24. `set_autostart` / 25. `open_external` / 26. `app_stats`
//! 27. `window_show` / 28. `window_hide` / 29. `app_quit`
//!
//! 契约 v4 新增（30–38，详见 `docs/接口契约.md` §6）：
//! 30. `local_gpu_probe`（Vulkan 优先的真实探测，60s 缓存）
//! 31. `local_engine_status` / 32. `local_engine_start` / 33. `local_engine_stop`
//! 34. `backends_list` / 35. `backends_download` / 36. `backends_cancel` / 37. `backends_remove`
//! 38. `translate_cover`（覆盖式翻译，结构化块输入输出）

pub mod action;
pub mod capture;
pub mod config;
pub mod cover;
pub mod local;
pub mod models;
pub mod onboarding;
pub mod provider;
pub mod selection;
pub mod system;
pub mod translate;
pub mod window;

pub use action::{menu_execute, result_close, task_cancel, task_followup};
pub use capture::{capture_cancel, capture_freeze};
pub use config::{config_export, config_get, config_import, config_patch, config_reset};
pub use cover::translate_cover;
pub use local::{
    backends_cancel, backends_download, backends_list, backends_remove, local_engine_start,
    local_engine_status, local_engine_stop, local_gpu_probe,
};
pub use models::{models_cancel, models_download, models_list, models_remove};
pub use onboarding::{onboarding_finish, onboarding_state, onboarding_update};
pub use provider::provider_test;
pub use selection::{selection_commit, selection_reset};
pub use system::{app_quit, app_stats, hotkey_probe, open_external, set_autostart};
pub use translate::translate_test;
pub use window::{window_hide, window_show};
