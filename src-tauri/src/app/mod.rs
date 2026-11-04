//! 应用层（M01–M03 / M13）。
//!
//! 依赖方向：`commands → app(state/events) → services → domain`；`app` 持有 Tauri `AppHandle`，
//! 负责生命周期、窗口、托盘、热键等运行时形态。

pub mod hotkeys;
pub mod lifecycle;
pub mod state;
pub mod tray;
pub mod windows;
