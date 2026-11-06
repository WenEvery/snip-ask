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
    PROMPT_TRANSLATE_MINIMAL, PROMPT_TRANSLATE_MINIMAL_GLOSSARY,
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
    ("translate_minimal", "translate_minimal.md"),
    (
        "translate_minimal_glossary",
        "translate_minimal_glossary.md",
    ),
    ("answer", "answer.md"),
    ("followup", "followup.md"),
];

/// 内置模板。
pub fn builtin_template(key: &str) -> Option<&'static str> {
    match key {
        "text_search" => Some(PROMPT_TEXT_SEARCH),
        "image_search" => Some(PROMPT_IMAGE_SEARCH),
        "translate" => Some(PROMPT_TRANSLATE),
        "translate_minimal" => Some(PROMPT_TRANSLATE_MINIMAL),
        "translate_minimal_glossary" => Some(PROMPT_TRANSLATE_MINIMAL_GLOSSARY),
        "answer" => Some(PROMPT_ANSWER),
        "followup" => Some(PROMPT_FOLLOWUP),
        _ => None,
    }
}

/// 剔除模板开头的维护注释块（模板约定：「开头行为维护注释，渲染时由 prompt_engine 剔除」）。
///
/// 规则：从文件头开始，跳过以 `#` 开头的行与空行，直到第一个正文行；正文之后原样保留。
/// 这样发给模型的提示词不含 `# 用途 / # 变量 / # 版本 / ==== 正文 ====` 等维护信息（对小模型尤其重要，
/// S-05 实测长注释会让 0.5B 类小模型分心/复述原文）。
/// 纯函数，便于单测；用户覆盖的同名提示词同样生效。
pub fn strip_maintenance_header(template: &str) -> String {
    let mut body_started = false;
    let mut body = String::new();
    for line in template.lines() {
        if !body_started {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }
            body_started = true;
        }
        body.push_str(line);
        body.push('\n');
    }
    body.trim_end().to_string()
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

    /// 读取并渲染模板（先剔除维护注释头，再替换变量）。
    pub fn render(&self, key: &str, vars: &HashMap<&str, &str>) -> AppResult<String> {
        let template = self.load(key)?;
        let body = strip_maintenance_header(&template);
        render_template(&body, vars)
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
            "translate_minimal",
            "translate_minimal_glossary",
            "answer",
            "followup",
        ] {
            let rendered = engine
                .render(key, &vars)
                .unwrap_or_else(|err| panic!("{key}: {err}"));
            assert!(!rendered.contains("{text}"));
            // 维护注释头必须被剔除，不能发给模型。
            assert!(
                !rendered.trim_start().starts_with('#'),
                "{key} 渲染结果仍以 # 开头：{}",
                rendered.lines().next().unwrap_or_default()
            );
            assert!(
                !rendered.contains("以下为发送给模型的正文"),
                "{key} 保留了正文分隔注释"
            );
        }
    }

    #[test]
    fn strip_maintenance_header_keeps_body_only() {
        let template = "# 用途：示例\n# 变量：{text}\n\n# ==== 以下为发送给模型的正文 ====\n【角色】翻译。\n\n{text}\n";
        assert_eq!(
            strip_maintenance_header(template),
            "【角色】翻译。\n\n{text}"
        );
        // 没有维护头（用户覆盖）时原样保留正文。
        assert_eq!(strip_maintenance_header("正文 {text}"), "正文 {text}");
        assert_eq!(strip_maintenance_header(""), "");
    }

    #[test]
    fn translate_minimal_template_is_small_and_minimal() {
        let engine = PromptEngine::with_override_dir("C:/not-exists");
        let mut vars = HashMap::new();
        vars.insert("text", "你好");
        vars.insert("targetLang", "英文");
        vars.insert("sourceLang", "简体中文");
        vars.insert("glossary", "");
        let rendered = engine.render("translate_minimal", &vars).unwrap();
        assert!(rendered.contains("英文"));
        assert!(rendered.contains("你好"));
        assert!(!rendered.contains('#'));
        // 极简版提示词不再出现源语言/术语表行（小模型会复述这些行）。
        assert!(!rendered.contains("术语表"), "rendered={rendered}");
        assert!(!rendered.contains("简体中文"), "rendered={rendered}");
        // 精简提示词应显著短于产品长提示词（实测长提示词会让 0.5B 类小模型复述原文）。
        let full = engine.render("translate", &vars).unwrap();
        assert!(
            rendered.chars().count() * 3 < full.chars().count(),
            "精简提示词应显著短于 translate.md（minimal={} full={}）",
            rendered.chars().count(),
            full.chars().count()
        );
    }

    #[test]
    fn translate_minimal_glossary_template_only_adds_inline_glossary() {
        let engine = PromptEngine::with_override_dir("C:/not-exists");
        let mut vars = HashMap::new();
        vars.insert("text", "Hello, world");
        vars.insert("targetLang", "简体中文");
        vars.insert("sourceLang", "英文");
        vars.insert("glossary", "world => 世界; Hello => 你好");
        let rendered = engine.render("translate_minimal_glossary", &vars).unwrap();
        assert!(rendered.contains("简体中文"));
        assert!(rendered.contains("术语表（命中时按指定译法）：world => 世界; Hello => 你好"));
        assert!(rendered.contains("Hello, world"));
        assert!(!rendered.contains('#'));
        // 术语表必须与指令同一段（中间不能有空行），否则小模型会复述该行。
        let first_line = rendered.lines().next().unwrap_or_default();
        assert!(first_line.contains("术语表"));
        // 无术语表时仍选择极简模板（见 action_router/translator 的选择逻辑），这里只确保两模板同源极简。
        let plain = engine.render("translate_minimal", &vars).unwrap();
        assert!(!plain.contains("术语表"));
        assert!(rendered.chars().count() > plain.chars().count());
    }
}
