//! 数据目录决策（实施架构 §7）。
//!
//! - 便携版：存在 `<exe_dir>\portable.flag` → 数据写入 `<exe_dir>\data\`；
//! - 安装版：配置 → `%APPDATA%\SnipAsk`，模型 / 缓存 / 运行时 → `%LOCALAPPDATA%\SnipAsk`。

use std::path::{Path, PathBuf};

use crate::contract::errors::{AppError, AppResult};

/// 应用数据路径集合。
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// 可执行文件所在目录。
    pub exe_dir: PathBuf,
    /// 配置 / 引导 / 日志 / 历史库的根目录。
    pub data_dir: PathBuf,
    /// `config.json`。
    pub config_file: PathBuf,
    /// `onboarding.json`。
    pub onboarding_file: PathBuf,
    /// 用户提示词覆盖目录。
    pub prompts_dir: PathBuf,
    /// 日志目录。
    pub logs_dir: PathBuf,
    /// `history.db`。
    pub history_db: PathBuf,
    /// 本地模型目录。
    pub models_dir: PathBuf,
    /// 截图等临时缓存目录。
    pub cache_dir: PathBuf,
    /// 本地引擎运行时目录。
    pub runtime_dir: PathBuf,
    /// 配置导出默认目录。
    pub export_dir: PathBuf,
    /// 是否便携版。
    pub portable: bool,
}

/// 纯函数：决定数据根目录（便于单测，不触碰文件系统）。
///
/// 返回 `(data_root, local_root)`：
/// - 便携版 → 两者都是 `<exe_dir>\data`；
/// - 安装版 → `(config_dir/SnipAsk, local_dir/SnipAsk)`。
pub fn decide_data_root(
    portable: bool,
    exe_dir: &Path,
    config_dir: &Path,
    local_dir: &Path,
) -> (PathBuf, PathBuf) {
    if portable {
        let root = exe_dir.join("data");
        (root.clone(), root)
    } else {
        (config_dir.join("SnipAsk"), local_dir.join("SnipAsk"))
    }
}

impl AppPaths {
    /// 按当前进程环境解析全部路径。
    pub fn resolve() -> AppResult<Self> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let portable = exe_dir.join("portable.flag").is_file();
        let config_dir = dirs::config_dir()
            .ok_or_else(|| AppError::io_failed("无法解析用户配置目录（%APPDATA%）"))?;
        let local_dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::io_failed("无法解析用户本地数据目录（%LOCALAPPDATA%）"))?;
        Self::from_roots(portable, exe_dir, &config_dir, &local_dir)
    }

    /// 由外部传入的根目录构造（单测 / 便携版测试用）。
    pub fn from_roots(
        portable: bool,
        exe_dir: PathBuf,
        config_dir: &Path,
        local_dir: &Path,
    ) -> AppResult<Self> {
        let (data_dir, local_root) = decide_data_root(portable, &exe_dir, config_dir, local_dir);
        Ok(Self {
            exe_dir,
            config_file: data_dir.join("config.json"),
            onboarding_file: data_dir.join("onboarding.json"),
            prompts_dir: data_dir.join("prompts"),
            logs_dir: data_dir.join("logs"),
            history_db: data_dir.join("history.db"),
            models_dir: local_root.join("models"),
            cache_dir: local_root.join("cache"),
            runtime_dir: local_root.join("runtime"),
            export_dir: data_dir.join("exports"),
            data_dir,
            portable,
        })
    }

    /// 创建所有必需目录（幂等）。
    pub fn ensure(&self) -> AppResult<()> {
        let dirs = [
            &self.data_dir,
            &self.prompts_dir,
            &self.logs_dir,
            &self.cache_dir,
            &self.runtime_dir,
            &self.export_dir,
            &self.models_dir,
        ];
        for dir in dirs {
            std::fs::create_dir_all(dir).map_err(|err| {
                AppError::io_failed(format!("创建目录 {} 失败：{err}", dir.display()))
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_mode_uses_exe_directory() {
        let (data_root, local_root) = decide_data_root(
            true,
            Path::new("D:/apps/SnipAsk"),
            Path::new("C:/Users/u/AppData/Roaming"),
            Path::new("C:/Users/u/AppData/Local"),
        );
        assert_eq!(data_root, PathBuf::from("D:/apps/SnipAsk/data"));
        assert_eq!(local_root, PathBuf::from("D:/apps/SnipAsk/data"));
    }

    #[test]
    fn installed_mode_uses_appdata_and_localappdata() {
        let (data_root, local_root) = decide_data_root(
            false,
            Path::new("C:/Program Files/SnipAsk"),
            Path::new("C:/Users/u/AppData/Roaming"),
            Path::new("C:/Users/u/AppData/Local"),
        );
        assert_eq!(
            data_root,
            PathBuf::from("C:/Users/u/AppData/Roaming/SnipAsk")
        );
        assert_eq!(
            local_root,
            PathBuf::from("C:/Users/u/AppData/Local/SnipAsk")
        );
    }

    #[test]
    fn from_roots_composes_child_paths() {
        let paths = AppPaths::from_roots(
            false,
            PathBuf::from("C:/Program Files/SnipAsk"),
            Path::new("C:/Users/u/AppData/Roaming"),
            Path::new("C:/Users/u/AppData/Local"),
        )
        .unwrap();
        assert_eq!(
            paths.config_file,
            PathBuf::from("C:/Users/u/AppData/Roaming/SnipAsk/config.json")
        );
        assert_eq!(
            paths.history_db,
            PathBuf::from("C:/Users/u/AppData/Roaming/SnipAsk/history.db")
        );
        assert_eq!(
            paths.models_dir,
            PathBuf::from("C:/Users/u/AppData/Local/SnipAsk/models")
        );
        assert!(!paths.portable);
    }
}
