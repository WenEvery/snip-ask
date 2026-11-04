//! 快捷键解析、规范化与 Tauri 插件类型转换（M03 / 契约 §4.3）。
//!
//! 契约 §4.3 正则：`^(Ctrl|Alt|Shift|Win)(\+(Ctrl|Alt|Shift|Win))*\+[A-Z0-9]$`
//! （大小写不敏感，后端规范化为 `Ctrl+Q` 形式，修饰键顺序固定 Ctrl → Alt → Shift → Win）。

use crate::contract::config::{is_valid_hotkey, normalize_hotkey};
use crate::contract::errors::{AppError, AppResult, ErrorCode};

/// 修饰键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    /// Ctrl。
    Ctrl,
    /// Alt。
    Alt,
    /// Shift。
    Shift,
    /// Win。
    Win,
}

impl Modifier {
    /// 契约字面量。
    pub fn as_str(&self) -> &'static str {
        match self {
            Modifier::Ctrl => "Ctrl",
            Modifier::Alt => "Alt",
            Modifier::Shift => "Shift",
            Modifier::Win => "Win",
        }
    }
}

/// 主键（契约仅允许 A-Z / 0-9）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// 字母键（大写存储）。
    Char(char),
    /// 数字键 0–9。
    Digit(u8),
}

impl Key {
    /// 契约字面量。
    pub fn as_str(&self) -> String {
        match self {
            Key::Char(ch) => ch.to_string(),
            Key::Digit(digit) => digit.to_string(),
        }
    }
}

/// 解析后的快捷键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeySpec {
    /// 修饰键（已去重、已排序）。
    pub modifiers: Vec<Modifier>,
    /// 主键。
    pub key: Key,
}

impl HotkeySpec {
    /// 规范化字符串，如 `Ctrl+Shift+Q`。
    pub fn normalized(&self) -> String {
        let mut parts: Vec<String> = self
            .modifiers
            .iter()
            .map(|modifier| modifier.as_str().to_string())
            .collect();
        parts.push(self.key.as_str());
        parts.join("+")
    }
}

/// 解析快捷键；非法返回 `SA-1003`。
pub fn parse_spec(spec: &str) -> AppResult<HotkeySpec> {
    let normalized = normalize_hotkey(spec).ok_or_else(shortcut_error)?;
    let mut segments = normalized.split('+');
    let key_segment = segments.next_back().ok_or_else(shortcut_error)?;
    let mut modifiers = Vec::new();
    for segment in segments {
        let modifier = match segment {
            "Ctrl" => Modifier::Ctrl,
            "Alt" => Modifier::Alt,
            "Shift" => Modifier::Shift,
            "Win" => Modifier::Win,
            _ => return Err(shortcut_error()),
        };
        if !modifiers.contains(&modifier) {
            modifiers.push(modifier);
        }
    }
    let mut key_chars = key_segment.chars();
    let key_char = key_chars.next().ok_or_else(shortcut_error)?;
    if key_chars.next().is_some() {
        return Err(shortcut_error());
    }
    let key = if let Some(digit) = key_char.to_digit(10) {
        if !key_char.is_ascii_digit() {
            return Err(shortcut_error());
        }
        Key::Digit(digit as u8)
    } else if key_char.is_ascii_uppercase() {
        Key::Char(key_char)
    } else {
        return Err(shortcut_error());
    };
    Ok(HotkeySpec { modifiers, key })
}

fn shortcut_error() -> AppError {
    AppError::new(
        ErrorCode::HotkeyConflict,
        Some("快捷键必须匹配 (Ctrl|Alt|Shift|Win)[+...]+[A-Z0-9]".to_string()),
    )
    .with_message("快捷键格式不正确")
    .with_fallback(vec!["change_hotkey", "pause_hotkey"])
}

/// 规范化（非法返回 `None`，空串视为未绑定）。
pub fn normalize_spec(spec: &str) -> Option<String> {
    normalize_hotkey(spec)
}

/// 校验快捷键是否符合契约（空串允许，表示未绑定）。
pub fn validate_spec(spec: &str) -> AppResult<()> {
    if is_valid_hotkey(spec) {
        Ok(())
    } else {
        Err(shortcut_error())
    }
}

/// 转换为 global-shortcut 插件的 `Shortcut`（供注册用）。
///
/// TODO(api-check): 校验 `tauri_plugin_global_shortcut::{Modifiers, Code, Shortcut}`
/// 的实际导出名与 `Shortcut::new(Option<Modifiers>, Code)` 签名；插件版本见 Cargo.toml `"2"`。
pub fn to_tauri_shortcut(spec: &str) -> Option<tauri_plugin_global_shortcut::Shortcut> {
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

    let parsed = parse_spec(spec).ok()?;
    let mut modifiers = Modifiers::empty();
    for modifier in &parsed.modifiers {
        modifiers |= match modifier {
            Modifier::Ctrl => Modifiers::CONTROL,
            Modifier::Alt => Modifiers::ALT,
            Modifier::Shift => Modifiers::SHIFT,
            Modifier::Win => Modifiers::SUPER,
        };
    }
    let code = match parsed.key {
        Key::Char(ch) => code_for_letter(ch)?,
        Key::Digit(digit) => code_for_digit(digit)?,
    };
    Some(Shortcut::new(Some(modifiers), code))
}

fn code_for_letter(ch: char) -> Option<tauri_plugin_global_shortcut::Code> {
    use tauri_plugin_global_shortcut::Code;
    Some(match ch.to_ascii_uppercase() {
        'A' => Code::KeyA,
        'B' => Code::KeyB,
        'C' => Code::KeyC,
        'D' => Code::KeyD,
        'E' => Code::KeyE,
        'F' => Code::KeyF,
        'G' => Code::KeyG,
        'H' => Code::KeyH,
        'I' => Code::KeyI,
        'J' => Code::KeyJ,
        'K' => Code::KeyK,
        'L' => Code::KeyL,
        'M' => Code::KeyM,
        'N' => Code::KeyN,
        'O' => Code::KeyO,
        'P' => Code::KeyP,
        'Q' => Code::KeyQ,
        'R' => Code::KeyR,
        'S' => Code::KeyS,
        'T' => Code::KeyT,
        'U' => Code::KeyU,
        'V' => Code::KeyV,
        'W' => Code::KeyW,
        'X' => Code::KeyX,
        'Y' => Code::KeyY,
        'Z' => Code::KeyZ,
        _ => return None,
    })
}

fn code_for_digit(digit: u8) -> Option<tauri_plugin_global_shortcut::Code> {
    use tauri_plugin_global_shortcut::Code;
    Some(match digit {
        0 => Code::Digit0,
        1 => Code::Digit1,
        2 => Code::Digit2,
        3 => Code::Digit3,
        4 => Code::Digit4,
        5 => Code::Digit5,
        6 => Code::Digit6,
        7 => Code::Digit7,
        8 => Code::Digit8,
        9 => Code::Digit9,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legal_specs() {
        let spec = parse_spec("Ctrl+Q").unwrap();
        assert_eq!(spec.modifiers, vec![Modifier::Ctrl]);
        assert_eq!(spec.key, Key::Char('Q'));
        assert_eq!(spec.normalized(), "Ctrl+Q");

        let spec = parse_spec("ctrl+shift+1").unwrap();
        assert_eq!(spec.modifiers, vec![Modifier::Ctrl, Modifier::Shift]);
        assert_eq!(spec.key, Key::Digit(1));
        assert_eq!(spec.normalized(), "Ctrl+Shift+1");

        assert_eq!(normalize_spec("WIN+alt+a").as_deref(), Some("Alt+Win+A"));
        assert_eq!(normalize_spec("").as_deref(), None);
    }

    #[test]
    fn rejects_illegal_specs() {
        for bad in ["", "Q", "Ctrl", "Ctrl+", "Ctrl+F1", "Ctrl+QQ", "Ctrl++Q", "Meta+Q"] {
            assert!(parse_spec(bad).is_err(), "应当拒绝：{bad}");
        }
        assert!(validate_spec("Ctrl+Q").is_ok());
        assert!(validate_spec("").is_ok());
        assert_eq!(validate_spec("Q").unwrap_err().code, "SA-1003");
    }

    #[test]
    fn converts_to_tauri_shortcut() {
        assert!(to_tauri_shortcut("Ctrl+Q").is_some());
        assert!(to_tauri_shortcut("Alt+Shift+9").is_some());
        assert!(to_tauri_shortcut("bad").is_none());
    }
}
