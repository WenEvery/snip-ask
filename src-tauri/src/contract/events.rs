//! 跨边界事件常量与载荷（契约 §7）。
//!
//! 事件名全部为小写点分形式，由 Rust 侧通过 `AppHandle::emit` 单向推送；
//! 强类型发送助手位于 [`crate::events`]（与 Tauri 解耦，便于替换测试实现）。

use serde::{Deserialize, Serialize};

use super::config::{ConfigView, LocalBackend, LocalDevice};
use super::types::{
    ActionItem, CoveredBlock, DownloadState, EngineStatus, FrozenFrameMeta, OcrBox, RectPhys,
    SessionId, StreamKind, TaskId,
};
pub use super::types::{TaskDonePayload, TaskStartedPayload};

/// IPC 契约版本（出现在 `Stats.contractVersion`）。
pub const IPC_CONTRACT_VERSION: u32 = 1;

/// overlay 窗口就绪、可绘制选框。
pub const EVENT_OVERLAY_READY: &str = "overlay:ready";
/// 选区变化（节流 ≤ 30Hz）。
pub const EVENT_OVERLAY_SELECTION: &str = "overlay:selection";
/// 前置 OCR 状态变化。
pub const EVENT_OCR_STATUS: &str = "ocr:status";
/// 动作菜单数据就绪。
pub const EVENT_MENU_READY: &str = "menu:ready";
/// 任务创建后、首个 delta 之前（脚手架扩展事件）。
pub const EVENT_TASK_STARTED: &str = "task:started";
/// 流式增量。
pub const EVENT_TASK_DELTA: &str = "task:delta";
/// 任务完成。
pub const EVENT_TASK_DONE: &str = "task:done";
/// 任务失败。
pub const EVENT_TASK_ERROR: &str = "task:error";
/// 下载进度（节流 200ms）。
pub const EVENT_DOWNLOAD_PROGRESS: &str = "download:progress";
/// 下载状态变化。
pub const EVENT_DOWNLOAD_STATE: &str = "download:state";
/// 配置热生效。
pub const EVENT_CONFIG_CHANGED: &str = "config:changed";
/// 托盘菜单动作。
pub const EVENT_TRAY_ACTION: &str = "tray:action";
/// 本地推理进程状态。
pub const EVENT_ENGINE_STATUS: &str = "engine:status";
/// 覆盖翻译进度（契约 v4）。
pub const EVENT_TRANSLATE_PROGRESS: &str = "translate:progress";
/// 覆盖翻译完成（契约 v4）。
pub const EVENT_TRANSLATE_READY: &str = "translate:ready";

/// 全部事件名（顺序与契约 §7 一致），供测试与前端名称表核对。
pub const ALL_EVENTS: &[&str] = &[
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
    EVENT_TRANSLATE_PROGRESS,
    EVENT_TRANSLATE_READY,
];

/// `overlay:ready` 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayReadyPayload {
    /// 会话 ID。
    pub session: SessionId,
    /// 虚拟桌面物理矩形。
    pub virtual_rect: RectPhys,
    /// 设备像素比。
    pub dpr: f64,
    /// 冻结帧元信息。
    pub frame: FrozenFrameMeta,
}

/// `overlay:selection` 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySelectionPayload {
    /// 会话 ID。
    pub session: SessionId,
    /// 当前选区（物理像素）。
    pub rect_phys: RectPhys,
}

/// 前置 OCR 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OcrStatusState {
    /// 识别中。
    Scanning,
    /// 完成。
    Done,
    /// 失败。
    Failed,
}

/// `ocr:status` 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrStatusPayload {
    /// 会话 ID。
    pub session: SessionId,
    /// 状态。
    pub state: OcrStatusState,
    /// 命中的文字框（`done` 时可能携带）。
    pub boxes: Option<Vec<OcrBox>>,
}

/// `menu:ready` 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuReadyPayload {
    /// 会话 ID。
    pub session: SessionId,
    /// 动作项。
    pub items: Vec<ActionItem>,
    /// 选区内是否有文字。
    pub has_text: bool,
    /// 选区预览文本。
    pub preview: String,
}

/// `task:delta` 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDeltaPayload {
    /// 任务 ID。
    pub task: TaskId,
    /// 增量类型。
    pub kind: StreamKind,
    /// 增量文本。
    pub chunk: String,
}

/// `task:error` 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskErrorPayload {
    /// 任务 ID。
    pub task: TaskId,
    /// 错误码。
    pub code: String,
    /// 面向用户的中文短句。
    pub message: String,
    /// 是否可重试。
    pub retryable: bool,
    /// 降级动作。
    pub fallback: Vec<String>,
}

/// `download:progress` 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgressPayload {
    /// 模型 id。
    pub model: String,
    /// 已下载字节。
    pub loaded: u64,
    /// 总字节。
    pub total: u64,
    /// 速度（B/s）。
    pub speed_bps: u64,
}

/// `download:state` 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatePayload {
    /// 模型 id。
    pub model: String,
    /// 状态。
    pub state: DownloadState,
}

/// `config:changed` 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigChangedPayload {
    /// 新配置视图。
    pub config: ConfigView,
    /// 变更键（点分路径，如 `ui.theme`）。
    pub changed_keys: Vec<String>,
}

/// `tray:action` 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayActionPayload {
    /// 动作 id（`capture` / `openSettings` / `rerunOnboarding` / `togglePause` / `quit`）。
    pub id: String,
}

/// `engine:status` 载荷（契约 v4 扩展：设备 / 后端 / 回退原因 / offload 层数）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatusPayload {
    /// 本地引擎状态。
    pub local: EngineStatus,
    /// 实际使用的计算设备（`auto`/`cpu`/`gpu` 的**解析结果**仍是这三值之一）。
    #[serde(default)]
    pub device: Option<LocalDevice>,
    /// 实际使用的后端（含 CPU）。
    #[serde(default)]
    pub backend: Option<LocalBackend>,
    /// 回退原因（如「Vulkan 后端未安装，已回退 CPU」）。
    #[serde(default)]
    pub fallback_reason: Option<String>,
    /// 实际 offload 到 GPU 的层数。
    #[serde(default)]
    pub offload_layers: Option<u32>,
    /// 设备名（如 `AMD Radeon RX 7800 XT`）。
    #[serde(default)]
    pub device_name: Option<String>,
    /// 监听端口。
    #[serde(default)]
    pub port: Option<u16>,
}

/// `translate:progress` 载荷（契约 v4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateProgressPayload {
    /// 会话 ID。
    pub session: SessionId,
    /// 已处理块数。
    pub done: u32,
    /// 总块数。
    pub total: u32,
}

/// `translate:ready` 载荷（契约 v4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateReadyPayload {
    /// 会话 ID。
    pub session: SessionId,
    /// 逐块结果。
    pub blocks: Vec<CoveredBlock>,
    /// 实际使用的引擎。
    pub engine: String,
}

/// 任务相关事件复用 [`super::types`] 的载荷。
pub type TaskStarted = TaskStartedPayload;
/// 任务完成事件载荷别名。
pub type TaskDone = TaskDonePayload;

#[cfg(test)]
mod tests {
    use super::super::types::{ActionId, ActionTarget, Mode, TokenUsage};
    use super::*;

    #[test]
    fn event_names_match_contract() {
        assert_eq!(ALL_EVENTS.len(), 15);
        assert_eq!(EVENT_OVERLAY_READY, "overlay:ready");
        assert_eq!(EVENT_OVERLAY_SELECTION, "overlay:selection");
        assert_eq!(EVENT_OCR_STATUS, "ocr:status");
        assert_eq!(EVENT_MENU_READY, "menu:ready");
        assert_eq!(EVENT_TASK_STARTED, "task:started");
        assert_eq!(EVENT_TASK_DELTA, "task:delta");
        assert_eq!(EVENT_TASK_DONE, "task:done");
        assert_eq!(EVENT_TASK_ERROR, "task:error");
        assert_eq!(EVENT_DOWNLOAD_PROGRESS, "download:progress");
        assert_eq!(EVENT_DOWNLOAD_STATE, "download:state");
        assert_eq!(EVENT_CONFIG_CHANGED, "config:changed");
        assert_eq!(EVENT_TRAY_ACTION, "tray:action");
        assert_eq!(EVENT_ENGINE_STATUS, "engine:status");
        assert_eq!(EVENT_TRANSLATE_PROGRESS, "translate:progress");
        assert_eq!(EVENT_TRANSLATE_READY, "translate:ready");
        assert_eq!(IPC_CONTRACT_VERSION, 1);
    }

    #[test]
    fn event_payloads_use_camel_case_and_literals() {
        let payload = OcrStatusPayload {
            session: SessionId::from("s-1"),
            state: OcrStatusState::Scanning,
            boxes: None,
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["session"], "s-1");
        assert_eq!(value["state"], "scanning");
        assert!(value["boxes"].is_null());

        let payload = MenuReadyPayload {
            session: SessionId::from("s-1"),
            items: vec![ActionItem {
                id: ActionId::Search,
                label: "搜索".to_string(),
                target: ActionTarget::Text,
                mode: Mode::Quick,
                enabled: true,
                disabled_reason: None,
                shortcut: None,
            }],
            has_text: true,
            preview: "预览".to_string(),
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["hasText"].as_bool(), Some(true));
        assert_eq!(value["items"][0]["id"], "search");
        assert_eq!(value["items"][0]["disabledReason"], serde_json::Value::Null);

        let payload = TaskDeltaPayload {
            task: TaskId::from("t-1"),
            kind: StreamKind::Text,
            chunk: "hello".to_string(),
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["kind"], "text");

        let payload = TaskDonePayload {
            task: TaskId::from("t-1"),
            usage: TokenUsage::default(),
            latency_ms: 12,
            engine: "skeleton".to_string(),
            sources: Vec::new(),
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["usage"]["currency"], "CNY");
        assert_eq!(value["latencyMs"], 12);
    }
}
