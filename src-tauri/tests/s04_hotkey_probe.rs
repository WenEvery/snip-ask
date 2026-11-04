//! S-04：`hotkey_probe` 的 OS 级占用探测映射（跨 crate 边界的公开 API 验证）。
//!
//! 真实 OS 注册由 `applab` 真机验证覆盖；这里用注入闭包覆盖「成功 / 占用 / 已注册」
//! 三种结果到契约 `HotkeyStatus` 的映射，保证映射 100% 符合契约语义、且
//! `conflictWith` 的两种冲突类型（`hotkeys.*` 字段名 vs `system`）互不混淆。

use snip_ask_lib::app::hotkeys::{probe_os_with, OsProbe, SYSTEM_CONFLICT};
use snip_ask_lib::contract::config::Config;
use snip_ask_lib::contract::types::HotkeyStatus;

fn default_config() -> Config {
    Config::default()
}

#[test]
fn system_conflict_serializes_as_camel_case_field() {
    let config = default_config();
    let status = probe_os_with(&config, "Ctrl+Q", |_| OsProbe::Occupied);
    assert!(!status.available);
    assert_eq!(status.conflict_with.as_deref(), Some(SYSTEM_CONFLICT));
    assert_eq!(SYSTEM_CONFLICT, "system");

    let json = serde_json::to_value(&status).expect("HotkeyStatus 应可序列化");
    assert_eq!(json["conflictWith"], "system");
    assert_eq!(json["available"], false);
    assert_eq!(json["valid"], true);
    assert_eq!(json["normalized"], "Ctrl+Q");
    assert_eq!(json["spec"], "Ctrl+Q");
}

#[test]
fn config_conflict_keeps_field_name() {
    let config = default_config(); // hotkeys.capture = "Ctrl+Q"
    let status = probe_os_with(&config, "ctrl+q", |_| OsProbe::Free);
    assert!(!status.available);
    // 配置内冲突必须保持字段名，不能被 system 覆盖
    assert_eq!(status.conflict_with.as_deref(), Some("hotkeys.capture"));
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["conflictWith"], "hotkeys.capture");
}

#[test]
fn occupied_takes_precedence_over_config_conflict() {
    // 模拟「QQ 已先占 Ctrl+Q，应用启动注册失败」：键未被本应用注册且 OS 注册失败，
    // 此时用户更需要的提示是「被其它程序占用」。
    let config = default_config();
    let status = probe_os_with(&config, "Ctrl+Q", |_| OsProbe::Occupied);
    assert!(!status.available);
    assert_eq!(status.conflict_with.as_deref(), Some(SYSTEM_CONFLICT));
}

#[test]
fn registered_binding_is_available() {
    let config = default_config();
    let status = probe_os_with(&config, "Ctrl+Q", |_| OsProbe::RegisteredByApp);
    assert!(status.available);
    assert!(status.conflict_with.is_none());
}

#[test]
fn candidate_combinations_available_when_os_free() {
    let config = default_config();
    for spec in ["Ctrl+Shift+Q", "Ctrl+Shift+Y", "Alt+Shift+9"] {
        let status: HotkeyStatus = probe_os_with(&config, spec, |_| OsProbe::Free);
        assert!(status.valid, "{spec}");
        assert!(status.available, "{spec}");
        assert!(status.conflict_with.is_none(), "{spec}");
    }
}

#[test]
fn invalid_spec_short_circuits_without_os_probe() {
    let config = default_config();
    let called = std::cell::Cell::new(false);
    let status = probe_os_with(&config, "Ctrl+F1", |_| {
        called.set(true);
        OsProbe::Free
    });
    assert!(!status.valid);
    assert!(!status.available);
    assert!(status.normalized.is_none());
    assert!(!called.get());
}
