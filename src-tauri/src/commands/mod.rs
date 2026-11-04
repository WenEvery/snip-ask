//! 命令层（契约 §6 全部 29 个命令）。
//!
//! 每个命令都薄：校验参数 → 调用 service → 返回 contract 类型；错误统一 `AppResult<T>`。
//! 命令注册在 `src/lib.rs` 的 `tauri::generate_handler![...]` 中一次性完成
//! （Tauri 2 要求 handler 列表为编译期宏，故不在本模块提供 `handler()` 函数）。
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

pub mod action;
pub mod capture;
pub mod config;
pub mod models;
pub mod onboarding;
pub mod provider;
pub mod selection;
pub mod system;
pub mod translate;
pub mod window;
