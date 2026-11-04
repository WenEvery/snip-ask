//! 模式（快 / 深）相关纯逻辑（契约 §1.6）。

use crate::contract::types::Mode;

/// 从契约字面量解析模式。
pub fn parse_mode(value: &str) -> Option<Mode> {
    Mode::parse(value)
}

/// 模式的用户可见中文名。
pub fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Quick => "快速",
        Mode::Deep => "深度",
    }
}

/// 计算动作的实际模式：显式指定 > 动作配置 > 全局默认。
pub fn effective_mode(
    explicit: Option<Mode>,
    configured: Option<Mode>,
    action_default: Mode,
) -> Mode {
    explicit.or(configured).unwrap_or(action_default)
}

/// 深度模式的 token 预算（契约 §4.1 `modes.deepBudget.maxTokens`）。
pub fn deep_token_budget(configured: u32, quick_default: u32) -> u32 {
    if configured == 0 {
        quick_default
    } else {
        configured.min(8192)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_roundtrip() {
        assert_eq!(parse_mode("quick"), Some(Mode::Quick));
        assert_eq!(parse_mode("deep"), Some(Mode::Deep));
        assert_eq!(parse_mode("Quick"), None);
        assert_eq!(Mode::Quick.toggle(), Mode::Deep);
        assert_eq!(Mode::Deep.toggle(), Mode::Quick);
        assert_eq!(mode_label(Mode::Deep), "深度");
    }

    #[test]
    fn effective_mode_precedence() {
        assert_eq!(
            effective_mode(Some(Mode::Deep), Some(Mode::Quick), Mode::Quick),
            Mode::Deep
        );
        assert_eq!(
            effective_mode(None, Some(Mode::Deep), Mode::Quick),
            Mode::Deep
        );
        assert_eq!(effective_mode(None, None, Mode::Quick), Mode::Quick);
    }

    #[test]
    fn deep_budget_is_clamped() {
        assert_eq!(deep_token_budget(1500, 1024), 1500);
        assert_eq!(deep_token_budget(0, 1024), 1024);
        assert_eq!(deep_token_budget(99_999, 1024), 8192);
    }
}
