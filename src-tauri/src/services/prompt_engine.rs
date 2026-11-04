//! 提示词引擎（M20 / 契约 §10）。
//!
//! - 内置模板来自 `crate::resources`（`include_str!` 编译内嵌）；
//! - 用户可在 `%APPDATA%\SnipAsk\prompts\<key>.md` 覆盖同名提示词（用户 > 内置）；
//! - 变量替换：`{text}` `{lang}` `{mode}` `{targetLang}` `{sourceLang}` `{glossary}`
//!   `{question}` `{context}`；缺失变量返回 `SA-5001`，未知花括号片段原样保留。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::contract::errors::{AppError, AppResult};
use crate::platform::paths::AppPaths;
use crate::resources::{
    PROMPT_ANSWER, PROMPT_FOLLOWUP, PROMPT_IMAGE_SEARCH, PROMPT_TEXT_SEARCH, PROMPT_TRANSLATE,
};

/// 支持的变量名。
pub const KNOWN_VARS: &[&str] = &[
    "text",
    "lang",
    "mode",
    "targetLang",
    "sourceLang",
    "glossary",
    "question",
    "context",
];

/// 模板 key → 文件名。
pub const PROMPT_FILES: &[(&str, &str)] = &[
    ("text_search", "text_search.md"),
    ("image_search", "image_search.md"),
    ("translate", "translate.md"),
    ("answer", "answer.md"),
    ("followup", "followup.md"),
];

/// 内置模板。
pub fn builtin_template(key: &str) -> Option<&'static str> {
    match key {
        "text_search" => Some(PROMPT_TEXT_SEARCH),
        "image_search" => Some(PROMPT_IMAGE_SEARCH),
        "translate" => Some(PROMPT_TRANSLATE),
        "answer" => Some(PROMPT_ANSWER),
        "followup" => Some(PROMPT_FOLLOWUP),
        _ => None,
    }
}

/// 纯函数：按变量表替换模板；替换后仍残留已知变量则报错。
pub fn render_template(template: &str, vars: &HashMap<&str, &str>) -> AppResult<String> {
    let mut rendered = template.to_string();
    for (key, value) in vars {
        rendered = rendered.replace(&format!("{{{key}}}"), value);
    }
    for var in KNOWN_VARS {
        if rendered.contains(&format!("{{{var}}}")) {
            return Err(AppError::invalid_config(
                "prompt",
                format!("缺少模板变量 {{{var}}}"),
            ));
        }
    }
    Ok(rendered)
}

/// 提示词引擎（带用户覆盖目录）。
#[derive(Debug, Clone)]
pub struct PromptEngine {
    override_dir: PathBuf,
}

impl PromptEngine {
    /// 由应用路径构造（覆盖目录 = `%APPDATA%\SnipAsk\prompts`）。
    pub fn new(paths: &AppPaths) -> Self {
        Self {
            override_dir: paths.prompts_dir.clone(),
        }
    }

    /// 由覆盖目录直接构造（测试用）。
    pub fn with_override_dir(override_dir: impl Into<PathBuf>) -> Self {
        Self {
            override_dir: override_dir.into(),
        }
    }

    /// 读取模板：用户覆盖 > 内置。
    pub fn load(&self, key: &str) -> AppResult<String> {
        if let Some(file_name) = PROMPT_FILES
            .iter()
            .find(|(prompt_key, _)| *prompt_key == key)
            .map(|(_, file)| *file)
        {
            let user_file = self.override_dir.join(file_name);
            if user_file.is_file() {
                return std::fs::read_to_string(&user_file).map_err(|err| {
                    AppError::io_failed(format!(
                        "读取用户提示词 {} 失败：{err}",
                        user_file.display()
                    ))
                });
            }
        }
        builtin_template(key)
            .map(str::to_string)
            .ok_or_else(|| AppError::invalid_config("prompt", format!("未知提示词 key：{key}")))
    }

    /// 读取并渲染模板。
    pub fn render(&self, key: &str, vars: &HashMap<&str, &str>) -> AppResult<String> {
        let template = self.load(key)?;
        render_template(&template, vars)
    }

    /// 覆盖目录路径。
    pub fn override_dir(&self) -> &Path {
        &self.override_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_templates_are_available() {
        for key in [
            "text_search",
            "image_search",
            "translate",
            "answer",
            "followup",
        ] {
            let template = builtin_template(key).expect("内置模板必须存在");
            assert!(!template.trim().is_empty());
        }
        assert!(builtin_template("missing").is_none());
    }

    #[test]
    fn render_replaces_known_variables() {
        let template = "搜索 {text}（{lang}/{mode}）";
        let mut vars = HashMap::new();
        vars.insert("text", "框选 AI");
        vars.insert("lang", "zh-CN");
        vars.insert("mode", "quick");
        assert_eq!(
            render_template(template, &vars).unwrap(),
            "搜索 框选 AI（zh-CN/quick）"
        );
    }

    #[test]
    fn render_reports_missing_variable() {
        let template = "翻译成 {targetLang}：{text}";
        let mut vars = HashMap::new();
        vars.insert("text", "hello");
        let error = render_template(template, &vars).unwrap_err();
        assert_eq!(error.code, "SA-5001");
        assert!(error.detail.unwrap().contains("targetLang"));
    }

    #[test]
    fn unknown_braces_are_left_untouched() {
        let template = "JSON 示例 {\"a\": 1} 与 {text}";
        let mut vars = HashMap::new();
        vars.insert("text", "x");
        assert_eq!(
            render_template(template, &vars).unwrap(),
            "JSON 示例 {\"a\": 1} 与 x"
        );
    }

    #[test]
    fn unknown_prompt_key_is_rejected() {
        let engine = PromptEngine::with_override_dir("C:/not-exists");
        assert!(engine.load("missing").is_err());
        assert!(engine.load("answer").is_ok());
    }

    #[test]
    fn every_template_renders_with_its_expected_vars() {
        let engine = PromptEngine::with_override_dir("C:/not-exists");
        let mut vars = HashMap::new();
        vars.insert("text", "框选 AI");
        vars.insert("lang", "zh-CN");
        vars.insert("mode", "quick");
        vars.insert("targetLang", "zh-CHS");
        vars.insert("sourceLang", "auto");
        vars.insert("glossary", "");
        vars.insert("question", "为什么？");
        vars.insert("context", "上文");
        for key in [
            "text_search",
            "image_search",
            "translate",
            "answer",
            "followup",
        ] {
            let rendered = engine
                .render(key, &vars)
                .unwrap_or_else(|err| panic!("{key}: {err}"));
            assert!(!rendered.contains("{text}"));
        }
    }
}
