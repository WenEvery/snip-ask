//! 强类型事件发送助手（契约 §7）。
//!
//! 前端通过 `listen(name, cb)` 订阅；后端只在 Rust 侧 `AppHandle::emit`，前端不得回发同名事件。
//! 这里提供 14 个事件的一一对应助手，避免字符串散落各处。
//!
//! 另提供 [`Throttle`]：`overlay:selection` ≤ 30Hz、`download:progress` ≤ 200ms。

use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::contract::events::{
    ConfigChangedPayload, DownloadProgressPayload, DownloadStatePayload, EngineStatusPayload,
    MenuReadyPayload, OcrStatusPayload, OverlayReadyPayload, OverlaySelectionPayload,
    TaskDeltaPayload, TaskDonePayload, TaskErrorPayload, TaskStartedPayload, TrayActionPayload,
    UpdateState, UpdateStatusPayload, EVENT_CONFIG_CHANGED, EVENT_DOWNLOAD_PROGRESS,
    EVENT_DOWNLOAD_STATE, EVENT_ENGINE_STATUS, EVENT_MENU_READY, EVENT_OCR_STATUS,
    EVENT_OVERLAY_READY, EVENT_OVERLAY_SELECTION, EVENT_TASK_DELTA, EVENT_TASK_DONE,
    EVENT_TASK_ERROR, EVENT_TASK_STARTED, EVENT_TRAY_ACTION, EVENT_UPDATE_STATUS,
};
use crate::contract::types::EngineStatus;

/// `overlay:ready`
pub fn emit_overlay_ready(app: &AppHandle, payload: &OverlayReadyPayload) {
    tracing::debug!(target: "snip_ask_lib::trace", "emit overlay:ready session={}", payload.session);
    if let Err(err) = app.emit(EVENT_OVERLAY_READY, payload) {
        tracing::warn!(target: "snip_ask_lib::trace", "emit overlay:ready 失败：{err}");
    }
}

/// `overlay:selection`
pub fn emit_overlay_selection(app: &AppHandle, payload: &OverlaySelectionPayload) {
    let _ = app.emit(EVENT_OVERLAY_SELECTION, payload);
}

/// `ocr:status`
pub fn emit_ocr_status(app: &AppHandle, payload: &OcrStatusPayload) {
    let _ = app.emit(EVENT_OCR_STATUS, payload);
}

/// `menu:ready`
pub fn emit_menu_ready(app: &AppHandle, payload: &MenuReadyPayload) {
    let _ = app.emit(EVENT_MENU_READY, payload);
}

/// `task:started`（脚手架扩展事件）
pub fn emit_task_started(app: &AppHandle, payload: &TaskStartedPayload) {
    let _ = app.emit(EVENT_TASK_STARTED, payload);
}

/// `task:delta`
pub fn emit_task_delta(app: &AppHandle, payload: &TaskDeltaPayload) {
    let _ = app.emit(EVENT_TASK_DELTA, payload);
}

/// `task:done`
pub fn emit_task_done(app: &AppHandle, payload: &TaskDonePayload) {
    let _ = app.emit(EVENT_TASK_DONE, payload);
}

/// `task:error`
pub fn emit_task_error(app: &AppHandle, payload: &TaskErrorPayload) {
    let _ = app.emit(EVENT_TASK_ERROR, payload);
}

/// `download:progress`
pub fn emit_download_progress(app: &AppHandle, payload: &DownloadProgressPayload) {
    let _ = app.emit(EVENT_DOWNLOAD_PROGRESS, payload);
}

/// `download:state`
pub fn emit_download_state(app: &AppHandle, payload: &DownloadStatePayload) {
    let _ = app.emit(EVENT_DOWNLOAD_STATE, payload);
}

/// `config:changed`
pub fn emit_config_changed(app: &AppHandle, payload: &ConfigChangedPayload) {
    let _ = app.emit(EVENT_CONFIG_CHANGED, payload);
}

/// `tray:action`
pub fn emit_tray_action(app: &AppHandle, payload: &TrayActionPayload) {
    let _ = app.emit(EVENT_TRAY_ACTION, payload);
}

/// `engine:status`
pub fn emit_engine_status(app: &AppHandle, status: EngineStatus) {
    let _ = app.emit(EVENT_ENGINE_STATUS, EngineStatusPayload { local: status });
}

/// `update:status`
pub fn emit_update_status(
    app: &AppHandle,
    state: UpdateState,
    version: Option<String>,
    error: Option<String>,
) {
    let _ = app.emit(
        EVENT_UPDATE_STATUS,
        UpdateStatusPayload {
            state,
            version,
            error,
        },
    );
}

/// 简易节流器（单线程时间戳比较，足够覆盖事件节流场景）。
#[derive(Debug)]
pub struct Throttle {
    interval: Duration,
    last: parking_lot::Mutex<Option<Instant>>,
}

impl Throttle {
    /// 按时间间隔创建。
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: parking_lot::Mutex::new(None),
        }
    }

    /// 按频率（Hz）创建，如 30Hz → 33ms。
    pub fn new_hz(hz: u32) -> Self {
        let hz = hz.max(1);
        Self::new(Duration::from_millis(1000 / u64::from(hz)))
    }

    /// 是否允许本次发送（并记录时间）。
    pub fn allow(&self) -> bool {
        let mut last = self.last.lock();
        let now = Instant::now();
        match *last {
            Some(previous) if now.duration_since(previous) < self.interval => false,
            _ => {
                *last = Some(now);
                true
            }
        }
    }

    /// 重置（新会话时避免沿用旧节流状态）。
    pub fn reset(&self) {
        *self.last.lock() = None;
    }
}

/// 选区事件节流：≤ 30Hz（契约 §7）。
pub fn overlay_selection_throttle() -> Throttle {
    Throttle::new_hz(30)
}

/// 下载进度节流：≤ 200ms（契约 §7）。
pub fn download_progress_throttle() -> Throttle {
    Throttle::new(Duration::from_millis(200))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_limits_frequency() {
        let throttle = Throttle::new(Duration::from_millis(50));
        assert!(throttle.allow());
        assert!(!throttle.allow());
        assert!(!throttle.allow());
        throttle.reset();
        assert!(throttle.allow());
    }

    #[test]
    fn new_hz_30_means_about_33ms() {
        let throttle = Throttle::new_hz(30);
        assert!(throttle.allow());
        assert!(!throttle.allow());
    }
}
