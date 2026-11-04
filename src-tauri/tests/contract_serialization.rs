//! 契约序列化集成测试（契约 §3–§10 / §15）。
//!
//! 在 CI（windows-latest）执行 `cargo test`；不依赖 Tauri 运行时，只验证
//! `snip_ask_lib::contract` 的 JSON 形状与前端 `src/shared/contract/types.ts` 一致。

use std::collections::BTreeSet;

use snip_ask_lib::contract::config::{
    Cap, Config, ConfigView, FontSize, Language, LocalEngine, ResultPosition, SearchMode, Theme,
    TranslateBackend, UpdateChannel, CONFIG_SCHEMA_VERSION,
};
use snip_ask_lib::contract::errors::{AppError, ErrorCode};
use snip_ask_lib::contract::events::{
    DownloadProgressPayload, DownloadStatePayload, MenuReadyPayload, OcrStatusPayload,
    OcrStatusState, OverlayReadyPayload, OverlaySelectionPayload, TaskDeltaPayload,
    TaskErrorPayload, TaskStartedPayload, TrayActionPayload, UpdateState, UpdateStatusPayload,
    EVENT_CONFIG_CHANGED, EVENT_DOWNLOAD_PROGRESS, EVENT_DOWNLOAD_STATE, EVENT_ENGINE_STATUS,
    EVENT_MENU_READY, EVENT_OCR_STATUS, EVENT_OVERLAY_READY, EVENT_OVERLAY_SELECTION,
    EVENT_TASK_DELTA, EVENT_TASK_DONE, EVENT_TASK_ERROR, EVENT_TASK_STARTED, EVENT_TRAY_ACTION,
    EVENT_UPDATE_STATUS, IPC_CONTRACT_VERSION,
};
use snip_ask_lib::contract::types::{
    ActionId, ActionItem, ActionTarget, DownloadState, EngineStatus, FrozenFrameMeta, HotkeyStatus,
    Mode, ModelInfo, ModelKind, OnboardingState, ProviderDraft, RectPhys, RunActionOptions,
    SelectionResult, SessionId, SourceRef, Stats, StreamKind, TaskDonePayload, TaskId, TestResult,
    TokenUsage, WindowLabel,
};

/// 契约 §7 的 14 个事件名（顺序即契约表顺序）。
const EXPECTED_EVENTS: [&str; 14] = [
    EVENT_OVERLAY_READY,
    EVENT_OVERLAY_SELECTION,
    EVENT_OCR_STATUS,
    EVENT_MENU_READY,
    EVENT_TASK_STARTED,
    EVENT_TASK_DELTA,
    EVENT_TASK_DONE,
    EVENT_TASK_ERROR,
    EVENT_DOWNLOAD_PROGRESS,
    EVENT_DOWNLOAD_STATE,
    EVENT_CONFIG_CHANGED,
    EVENT_TRAY_ACTION,
    EVENT_ENGINE_STATUS,
    EVENT_UPDATE_STATUS,
];

#[test]
fn contract_version_is_one() {
    assert_eq!(IPC_CONTRACT_VERSION, 1);
    assert_eq!(CONFIG_SCHEMA_VERSION, 3);
}

#[test]
fn event_names_are_exactly_the_fourteen_contract_names() {
    let unique: BTreeSet<&str> = EXPECTED_EVENTS.iter().copied().collect();
    assert_eq!(unique.len(), 14);
    let expected: BTreeSet<&str> = [
        "overlay:ready",
        "overlay:selection",
        "ocr:status",
        "menu:ready",
        "task:started",
        "task:delta",
        "task:done",
        "task:error",
        "download:progress",
        "download:state",
        "config:changed",
        "tray:action",
        "engine:status",
        "update:status",
    ]
    .into_iter()
    .collect();
    assert_eq!(unique, expected);
}

#[test]
fn config_default_serializes_to_contract_json() {
    let value = serde_json::to_value(Config::default()).unwrap();
    // 顶层键与契约 §4.1 完全一致（含 schemaVersion）。
    let keys: BTreeSet<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let expected_keys: BTreeSet<&str> = [
        "schemaVersion",
        "app",
        "ui",
        "hotkeys",
        "actions",
        "providers",
        "modes",
        "search",
        "translate",
        "local",
        "ocr",
        "privacy",
        "history",
        "advanced",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, expected_keys);

    // 关键默认值。
    assert_eq!(value["schemaVersion"], 3);
    assert_eq!(value["app"]["language"], "zh-CN");
    assert_eq!(value["app"]["singleInstance"].as_bool(), Some(true));
    assert_eq!(value["ui"]["theme"], "system");
    assert_eq!(value["ui"]["resultWindow"]["width"], 460.0);
    assert_eq!(value["ui"]["resultWindow"]["height"], 420.0);
    assert_eq!(value["hotkeys"]["capture"], "Ctrl+Q");
    assert_eq!(value["actions"]["menu"]["text"][0], "search");
    assert_eq!(value["actions"]["defaultAction"], serde_json::Value::Null);
    assert_eq!(value["providers"][0]["id"], "deepseek");
    assert_eq!(value["providers"][0]["caps"][0], "vision");
    assert_eq!(value["search"]["mode"], "browser");
    assert_eq!(value["translate"]["targetLang"], "zh-CHS");
    assert_eq!(value["local"]["engine"], "llama.cpp");
    assert_eq!(value["local"]["port"], 39281);
    assert_eq!(value["ocr"]["engine"], "rapidocr");
    assert_eq!(value["privacy"]["localOnly"].as_bool(), Some(false));
    assert_eq!(value["history"]["maxEntries"], 1000);
    assert_eq!(value["advanced"]["skeletonMode"].as_bool(), Some(true));
    assert_eq!(value["translate"]["glossary"].as_array().unwrap().len(), 0);
}

#[test]
fn config_roundtrips_through_json() {
    let config = Config::default();
    let text = serde_json::to_string(&config).unwrap();
    let parsed: Config = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, config);
}

#[test]
fn config_partial_json_fills_defaults() {
    // 缺失字段必须用默认值补齐（契约 §16：后端提供默认值）。
    let partial: Config = serde_json::from_str(r#"{"ui":{"theme":"dark"}}"#).unwrap();
    assert_eq!(partial.ui.theme, Theme::Dark);
    assert_eq!(partial.schema_version, 3);
    assert_eq!(partial.hotkeys.capture, "Ctrl+Q");
    assert_eq!(partial.providers.len(), 1);
}

#[test]
fn config_view_providers_have_key_status_fields() {
    let view = ConfigView::from_config(&Config::default(), &[(true, Some("••••abcd".to_string()))]);
    let value = serde_json::to_value(&view).unwrap();
    assert_eq!(value["providers"][0]["hasKey"].as_bool(), Some(true));
    assert_eq!(value["providers"][0]["keyMask"], "••••abcd");
    assert!(value["providers"][0].get("apiKey").is_none());

    let view = ConfigView::from(&Config::default());
    assert_eq!(
        serde_json::to_value(&view).unwrap()["providers"][0]["hasKey"],
        false
    );
}

#[test]
fn config_enums_serialize_with_contract_literals() {
    let cases: [(serde_json::Value, &str); 9] = [
        (serde_json::to_value(Language::EnUs).unwrap(), "en-US"),
        (serde_json::to_value(Theme::Dark).unwrap(), "dark"),
        (serde_json::to_value(FontSize::Lg).unwrap(), "lg"),
        (
            serde_json::to_value(ResultPosition::Fixed).unwrap(),
            "fixed",
        ),
        (serde_json::to_value(SearchMode::Ai).unwrap(), "ai"),
        (
            serde_json::to_value(TranslateBackend::Local).unwrap(),
            "local",
        ),
        (
            serde_json::to_value(LocalEngine::LlamaCpp).unwrap(),
            "llama.cpp",
        ),
        (
            serde_json::to_value(UpdateChannel::Preview).unwrap(),
            "preview",
        ),
        (serde_json::to_value(Cap::Thinking).unwrap(), "thinking"),
    ];
    for (value, expected) in cases {
        assert_eq!(value, expected);
    }
}

#[test]
fn base_types_use_contract_literals() {
    assert_eq!(serde_json::to_value(Mode::Quick).unwrap(), "quick");
    assert_eq!(serde_json::to_value(Mode::Deep).unwrap(), "deep");
    assert_eq!(
        serde_json::to_value(ActionId::ImageSearch).unwrap(),
        "imageSearch"
    );
    assert_eq!(serde_json::to_value(ActionTarget::Both).unwrap(), "both");
    assert_eq!(
        serde_json::to_value(StreamKind::Reasoning).unwrap(),
        "reasoning"
    );
    assert_eq!(serde_json::to_value(WindowLabel::Result).unwrap(), "result");
    assert_eq!(
        serde_json::to_value(DownloadState::Cancelled).unwrap(),
        "cancelled"
    );
    assert_eq!(serde_json::to_value(EngineStatus::Ready).unwrap(), "ready");
    assert_eq!(
        serde_json::to_value(UpdateState::Checking).unwrap(),
        "checking"
    );
    assert_eq!(
        serde_json::to_value(OcrStatusState::Scanning).unwrap(),
        "scanning"
    );
    assert_eq!(serde_json::to_value(ModelKind::Gguf).unwrap(), "gguf");
}

#[test]
fn app_error_has_six_camel_case_fields() {
    let error = AppError::new(ErrorCode::AuthFailed, Some("401".to_string()))
        .with_fallback(vec!["open_settings"]);
    let value = serde_json::to_value(&error).unwrap();
    let keys: BTreeSet<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "code",
            "message",
            "detail",
            "retryable",
            "fallback",
            "context"
        ]
        .into_iter()
        .collect::<BTreeSet<&str>>()
    );
    assert_eq!(value["code"], "SA-3002");
    assert_eq!(value["message"], "API Key 无效");
    assert_eq!(value["retryable"].as_bool(), Some(false));
    assert_eq!(value["fallback"][0], "open_settings");
    assert!(value["context"].is_null());

    // 20 个错误码全部可构造且 message 非空。
    let all_codes = [
        ErrorCode::CaptureFailed,
        ErrorCode::OverlayFailed,
        ErrorCode::HotkeyConflict,
        ErrorCode::InvalidState,
        ErrorCode::OcrEngineUnavailable,
        ErrorCode::OcrFailed,
        ErrorCode::NetworkUnavailable,
        ErrorCode::AuthFailed,
        ErrorCode::QuotaExceeded,
        ErrorCode::RateLimited,
        ErrorCode::Timeout,
        ErrorCode::ModelMissing,
        ErrorCode::OutOfMemory,
        ErrorCode::DownloadFailed,
        ErrorCode::ChecksumMismatch,
        ErrorCode::InvalidConfig,
        ErrorCode::UnsupportedOperation,
        ErrorCode::IoFailed,
        ErrorCode::AutostartFailed,
        ErrorCode::UrlRejected,
        ErrorCode::SkeletonNotImplemented,
    ];
    for code in all_codes {
        let error = AppError::from_code(code);
        assert_eq!(error.code, code);
        assert!(!error.message.is_empty());
        assert!(code.starts_with("SA-"));
    }
}

#[test]
fn task_payloads_match_contract_shape() {
    let started = TaskStartedPayload {
        task: TaskId::from("task-1"),
        session: SessionId::from("session-1"),
        action: ActionId::Search,
        mode: Mode::Quick,
        engine: "skeleton".to_string(),
    };
    let value = serde_json::to_value(&started).unwrap();
    assert_eq!(value["task"], "task-1");
    assert_eq!(value["session"], "session-1");
    assert_eq!(value["action"], "search");
    assert_eq!(value["mode"], "quick");
    assert_eq!(value["engine"], "skeleton");

    let done = TaskDonePayload {
        task: TaskId::from("task-1"),
        usage: TokenUsage {
            input_tokens: 120,
            output_tokens: 60,
            cached_input_tokens: 0,
            cost_estimate: 0.0,
            currency: Default::default(),
        },
        latency_ms: 812,
        engine: "skeleton".to_string(),
        sources: vec![SourceRef {
            title: "来源".to_string(),
            url: "https://example.com".to_string(),
            site_name: Some("example.com".to_string()),
            snippet: None,
        }],
    };
    let value = serde_json::to_value(&done).unwrap();
    assert_eq!(value["usage"]["inputTokens"], 120);
    assert_eq!(value["usage"]["cachedInputTokens"], 0);
    assert_eq!(value["usage"]["costEstimate"], 0.0);
    assert_eq!(value["usage"]["currency"], "CNY");
    assert_eq!(value["latencyMs"], 812);
    assert_eq!(value["sources"][0]["url"], "https://example.com");
    assert!(value["sources"][0]["snippet"].is_null());
}

#[test]
fn selection_and_action_shapes() {
    let rect = RectPhys {
        x: -10.0,
        y: 20.0,
        width: 30.0,
        height: 40.0,
    };
    let selection = SelectionResult {
        session: SessionId::from("s-1"),
        rect_phys: rect,
        has_text: true,
        preview: "文本".to_string(),
        boxes: vec![],
    };
    let value = serde_json::to_value(&selection).unwrap();
    assert_eq!(value["rectPhys"]["x"], -10.0);
    assert_eq!(value["hasText"].as_bool(), Some(true));

    let item = ActionItem {
        id: ActionId::Answer,
        label: "解答".to_string(),
        target: ActionTarget::Both,
        mode: Mode::Deep,
        enabled: false,
        disabled_reason: Some("已开启仅本地模式".to_string()),
        shortcut: None,
    };
    let value = serde_json::to_value(&item).unwrap();
    assert_eq!(value["id"], "answer");
    assert_eq!(value["disabledReason"], "已开启仅本地模式");
    assert!(value["shortcut"].is_null());
}

#[test]
fn event_payload_shapes() {
    let overlay = OverlayReadyPayload {
        session: SessionId::from("s-1"),
        virtual_rect: RectPhys {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        },
        dpr: 1.5,
        frame: FrozenFrameMeta {
            width: 1920,
            height: 1080,
            data_url: None,
        },
    };
    let value = serde_json::to_value(&overlay).unwrap();
    assert_eq!(value["virtualRect"]["width"], 1920.0);
    assert_eq!(value["dpr"], 1.5);
    assert!(value["frame"]["dataUrl"].is_null());

    let selection = OverlaySelectionPayload {
        session: SessionId::from("s-1"),
        rect_phys: RectPhys::default(),
    };
    assert!(serde_json::to_value(&selection).unwrap()["rectPhys"].is_object());

    let ocr = OcrStatusPayload {
        session: SessionId::from("s-1"),
        state: OcrStatusState::Done,
        boxes: None,
    };
    assert_eq!(
        serde_json::to_value(&ocr).unwrap()["boxes"],
        serde_json::Value::Null
    );

    let menu = MenuReadyPayload {
        session: SessionId::from("s-1"),
        items: vec![],
        has_text: true,
        preview: "预览".to_string(),
    };
    assert_eq!(
        serde_json::to_value(&menu).unwrap()["hasText"].as_bool(),
        Some(true)
    );

    let delta = TaskDeltaPayload {
        task: TaskId::from("t-1"),
        kind: StreamKind::Text,
        chunk: "增量".to_string(),
    };
    assert_eq!(serde_json::to_value(&delta).unwrap()["kind"], "text");

    let error = TaskErrorPayload {
        task: TaskId::from("t-1"),
        code: "SA-3002".to_string(),
        message: "API Key 无效".to_string(),
        retryable: false,
        fallback: vec!["open_settings".to_string()],
    };
    let value = serde_json::to_value(&error).unwrap();
    assert_eq!(value["task"], "t-1");
    assert_eq!(value["code"], "SA-3002");
    assert_eq!(value["fallback"][0], "open_settings");

    let progress = DownloadProgressPayload {
        model: "hy-mt-1.8b-q4".to_string(),
        loaded: 100,
        total: 1000,
        speed_bps: 10,
    };
    assert_eq!(serde_json::to_value(&progress).unwrap()["speedBps"], 10);

    let state = DownloadStatePayload {
        model: "m".to_string(),
        state: DownloadState::Installed,
    };
    assert_eq!(serde_json::to_value(&state).unwrap()["state"], "installed");

    let tray = TrayActionPayload {
        id: "capture".to_string(),
    };
    assert_eq!(serde_json::to_value(&tray).unwrap()["id"], "capture");

    let update = UpdateStatusPayload {
        state: UpdateState::Idle,
        version: None,
        error: None,
    };
    let value = serde_json::to_value(&update).unwrap();
    assert_eq!(value["state"], "idle");
    assert!(value["version"].is_null());
}

#[test]
fn provider_draft_and_test_result_shapes() {
    let draft: ProviderDraft = serde_json::from_str(
        r#"{"id":"deepseek","name":"DeepSeek","baseUrl":"https://api.deepseek.com","model":"deepseek-flash","caps":["vision"],"apiKey":null}"#,
    )
    .unwrap();
    assert!(draft.temperature.is_none());
    assert!(draft.max_tokens.is_none());
    assert!(draft.api_key.is_none());

    let result = TestResult {
        ok: false,
        latency_ms: None,
        model: None,
        status: None,
        error: Some(AppError::from_code(ErrorCode::Timeout)),
    };
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["ok"].as_bool(), Some(false));
    assert!(value["latencyMs"].is_null());
    assert_eq!(value["error"]["code"], "SA-3005");
}

#[test]
fn onboarding_and_model_shapes() {
    let state: OnboardingState = serde_json::from_str(r#"{"completed":false,"skipped":false,"version":1,"currentStep":0,"startedAt":"","updatedAt":""}"#).unwrap();
    assert_eq!(state.current_step, 0);
    let value = serde_json::to_value(&state).unwrap();
    assert_eq!(value["currentStep"], 0);
    assert_eq!(value["data"]["hotkeys"]["capture"], "Ctrl+Q");
    assert!(value["data"]["ai"].is_null());

    let model = ModelInfo {
        id: "hy-mt-1.8b-q4".to_string(),
        name: "HY-MT".to_string(),
        kind: ModelKind::Gguf,
        size_bytes: 100,
        sha256: "00".to_string(),
        license: "Apache-2.0".to_string(),
        min_ram_mb: 4096,
        urls: vec![],
        params: serde_json::json!({"quant":"Q4_K_M"}),
        installed: false,
        installed_bytes: 0,
        download_state: DownloadState::Queued,
    };
    let value = serde_json::to_value(&model).unwrap();
    assert_eq!(value["sizeBytes"], 100);
    assert_eq!(value["minRamMB"], 4096);
    assert_eq!(value["downloadState"], "queued");
}

#[test]
fn stats_and_overlay_types_camel_case() {
    let stats = Stats {
        version: "0.1.0".to_string(),
        contract_version: 1,
        uptime_sec: 5,
        memory_mb: 0,
        active_session: None,
        active_tasks: 0,
        local_engine: EngineStatus::Stopped,
        models_installed: 0,
        skeleton_mode: true,
    };
    let value = serde_json::to_value(&stats).unwrap();
    assert_eq!(value["contractVersion"], 1);
    assert_eq!(value["uptimeSec"], 5);
    assert_eq!(value["memoryMb"], 0);
    assert_eq!(value["activeSession"], serde_json::Value::Null);
    assert_eq!(value["localEngine"], "stopped");
    assert_eq!(value["modelsInstalled"], 0);
    assert_eq!(value["skeletonMode"].as_bool(), Some(true));

    let hotkey = HotkeyStatus {
        spec: "ctrl+q".to_string(),
        valid: true,
        available: false,
        normalized: Some("Ctrl+Q".to_string()),
        conflict_with: Some("hotkeys.capture".to_string()),
    };
    let value = serde_json::to_value(&hotkey).unwrap();
    assert_eq!(value["normalized"], "Ctrl+Q");
    assert_eq!(value["conflictWith"], "hotkeys.capture");

    let options = RunActionOptions::default();
    let value = serde_json::to_value(&options).unwrap();
    assert!(value["text"].is_null());
    assert!(value["targetLang"].is_null());
}

#[test]
fn ids_are_transparent_strings() {
    let session = SessionId::from("6ba7b810-9dad-41d1-80b4-00c04fd430c8");
    assert_eq!(
        serde_json::to_value(&session).unwrap(),
        "6ba7b810-9dad-41d1-80b4-00c04fd430c8"
    );
    let task: TaskId = serde_json::from_str("\"task-1\"").unwrap();
    assert_eq!(task.as_str(), "task-1");
}
