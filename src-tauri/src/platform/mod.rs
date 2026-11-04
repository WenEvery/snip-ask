//! 平台适配层（M19）。
//!
//! 这是**唯一允许未来引入 `unsafe` / Win32 调用的目录**；当前脚手架全部为安全代码，
//! 真实能力以 `// TODO(real): ...` 标注并以 trait 暴露，便于替换与单测。
//!
//! - `paths`：数据目录决策（便携版 / 安装版）；
//! - `capture`：屏幕捕获 trait + 骨架实现；
//! - `coords`：虚拟桌面 / DPI / CSS 像素换算纯函数；
//! - `hotkey`：快捷键解析、规范化、转 Tauri 插件类型；
//! - `autostart`：开机自启（插件封装，骨架模式仅改内存状态）；
//! - `credential`：凭据存储 trait + 内存实现（真实 Credential Manager 为 TODO(real)）；
//! - `process`：本地推理进程管理 trait + 停用实现。

pub mod autostart;
pub mod capture;
pub mod coords;
pub mod credential;
pub mod hotkey;
pub mod paths;
pub mod process;
