//! 会话状态机（契约 §5 / 概要设计 §3.3）。
//!
//! ```text
//! idle → freezing → selecting → menu → executing → showingResult → idle
//!              ↘ 取消/失败 → error → idle
//! ```
//!
//! 规则（后端强制，非法转移返回 `SA-1099`）：
//! - 同一时刻仅一个活动会话；新 `capture_freeze` 自动作废旧会话（旧 OCR 缓存作废、旧任务取消）；
//! - `selection_commit` 只在 `selecting` 态合法；`menu_execute` 只在 `menu` 态合法；
//! - `task_followup` 只在 `showingResult` 态合法；
//! - `result_close` / `capture_cancel` 在任意非 idle 态合法。
//!
//! 本模块为纯逻辑（无 IO / 无 tauri），可单元测试。

use serde::{Deserialize, Serialize};

use crate::contract::errors::{AppError, AppResult};
use crate::contract::types::{FrozenFrameMeta, SelectionResult, SessionId, TaskId};

use super::ids::new_session_id;

/// 会话阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SessionPhase {
    /// 空闲。
    #[default]
    Idle,
    /// 捕获中 / 冻结层创建中。
    Freezing,
    /// 选区交互中。
    Selecting,
    /// 动作菜单展示中。
    Menu,
    /// 任务执行中。
    Executing,
    /// 结果展示中。
    ShowingResult,
    /// 异常态。
    Error,
}

impl SessionPhase {
    /// 契约字面量（供日志与诊断）。
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionPhase::Idle => "idle",
            SessionPhase::Freezing => "freezing",
            SessionPhase::Selecting => "selecting",
            SessionPhase::Menu => "menu",
            SessionPhase::Executing => "executing",
            SessionPhase::ShowingResult => "showingResult",
            SessionPhase::Error => "error",
        }
    }
}

/// 一个活动会话。
#[derive(Debug, Clone)]
pub struct Session {
    /// 会话 ID。
    pub id: SessionId,
    /// 当前阶段。
    pub phase: SessionPhase,
    /// 冻结帧元信息。
    pub frame: Option<FrozenFrameMeta>,
    /// 前置 OCR 缓存键（新会话会作废旧键）。
    pub ocr_session_id: Option<String>,
    /// 已提交的选区结果。
    pub selection: Option<SelectionResult>,
    /// 本会话启动过的任务。
    pub task_ids: Vec<TaskId>,
    /// 开始时间（Unix 毫秒）。
    pub started_at: i64,
}

impl Session {
    fn new() -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            id: new_session_id(),
            phase: SessionPhase::Freezing,
            frame: None,
            ocr_session_id: Some(uuid::Uuid::new_v4().to_string()),
            selection: None,
            task_ids: Vec::new(),
            started_at: now,
        }
    }
}

/// 新会话开始结果：旧会话（若有）需要被取消的任务。
#[derive(Debug, Clone)]
pub struct CaptureStart {
    /// 新会话 ID。
    pub session: SessionId,
    /// 被作废的旧会话 ID。
    pub replaced_session: Option<SessionId>,
    /// 旧会话遗留的任务 ID（调用方负责取消）。
    pub replaced_tasks: Vec<TaskId>,
}

impl CaptureStart {
    /// 是否替换了旧会话。
    pub fn replaced(&self) -> bool {
        self.replaced_session.is_some()
    }
}

/// 会话容器（同一时刻最多一个活动会话）。
#[derive(Debug, Default)]
pub struct SessionStore {
    active: Option<Session>,
}

impl SessionStore {
    /// 创建空容器。
    pub fn new() -> Self {
        Self { active: None }
    }

    /// 当前活动会话的只读引用。
    pub fn current(&self) -> Option<&Session> {
        self.active.as_ref()
    }

    /// 当前活动会话 ID。
    pub fn current_id(&self) -> Option<SessionId> {
        self.active.as_ref().map(|session| session.id.clone())
    }

    /// 当前阶段（无会话为 `Idle`）。
    pub fn current_phase(&self) -> SessionPhase {
        self.active
            .as_ref()
            .map(|session| session.phase)
            .unwrap_or(SessionPhase::Idle)
    }

    /// 当前会话的全部任务 ID。
    pub fn active_task_ids(&self) -> Vec<TaskId> {
        self.active
            .as_ref()
            .map(|session| session.task_ids.clone())
            .unwrap_or_default()
    }

    /// 给定会话是否仍是当前活动会话（过期事件一律丢弃）。
    pub fn is_current(&self, session: &SessionId) -> bool {
        self.active
            .as_ref()
            .map(|active| &active.id == session)
            .unwrap_or(false)
    }

    /// 当前选区结果（供 menu_execute / task_followup 取文本）。
    pub fn current_selection(&self) -> Option<SelectionResult> {
        self.active
            .as_ref()
            .and_then(|session| session.selection.clone())
    }

    /// 开始新的捕获会话；自动作废旧会话并返回其任务 ID（调用方负责取消）。
    pub fn begin_capture(&mut self) -> CaptureStart {
        let mut replaced_session = None;
        let mut replaced_tasks = Vec::new();
        if let Some(old) = self.active.take() {
            replaced_session = Some(old.id);
            replaced_tasks = old.task_ids;
        }
        let session = Session::new();
        let session_id = session.id.clone();
        self.active = Some(session);
        CaptureStart {
            session: session_id,
            replaced_session,
            replaced_tasks,
        }
    }

    /// 冻结完成 → 进入选区交互（`freezing → selecting`）。
    pub fn set_selecting(&mut self, session: &SessionId) -> AppResult<()> {
        let active = self.get_mut(session)?;
        ensure_phase(active, &[SessionPhase::Freezing])?;
        active.phase = SessionPhase::Selecting;
        Ok(())
    }

    /// 附着冻结帧元信息（不改变阶段）。
    pub fn attach_frame(&mut self, session: &SessionId, frame: FrozenFrameMeta) -> AppResult<()> {
        let active = self.get_mut(session)?;
        active.frame = Some(frame);
        Ok(())
    }

    /// 提交选区（`selecting → menu`）。
    pub fn commit_selection(
        &mut self,
        session: &SessionId,
        selection: SelectionResult,
    ) -> AppResult<()> {
        let active = self.get_mut(session)?;
        ensure_phase(active, &[SessionPhase::Selecting])?;
        active.selection = Some(selection);
        active.phase = SessionPhase::Menu;
        Ok(())
    }

    /// 显式进入菜单阶段（`selecting | menu → menu`，幂等）。
    pub fn enter_menu(&mut self, session: &SessionId) -> AppResult<()> {
        let active = self.get_mut(session)?;
        ensure_phase(active, &[SessionPhase::Selecting, SessionPhase::Menu])?;
        active.phase = SessionPhase::Menu;
        Ok(())
    }

    /// 重置选区、回到选区交互（`menu | selecting → selecting`，复用 OCR 缓存）。
    pub fn reset_selection(&mut self, session: &SessionId) -> AppResult<()> {
        let active = self.get_mut(session)?;
        ensure_phase(active, &[SessionPhase::Menu, SessionPhase::Selecting])?;
        active.selection = None;
        active.phase = SessionPhase::Selecting;
        Ok(())
    }

    /// 开始任务（`menu | showingResult → executing`）。
    pub fn begin_task(&mut self, session: &SessionId, task: TaskId) -> AppResult<()> {
        let active = self.get_mut(session)?;
        ensure_phase(active, &[SessionPhase::Menu, SessionPhase::ShowingResult])?;
        active.phase = SessionPhase::Executing;
        active.task_ids.push(task);
        Ok(())
    }

    /// 任务完成（`executing → showingResult`）。
    pub fn task_finished(&mut self, session: &SessionId) -> AppResult<()> {
        let active = self.get_mut(session)?;
        ensure_phase(active, &[SessionPhase::Executing])?;
        active.phase = SessionPhase::ShowingResult;
        Ok(())
    }

    /// 任务失败（`executing → error`）。
    pub fn task_failed(&mut self, session: &SessionId) -> AppResult<()> {
        let active = self.get_mut(session)?;
        ensure_phase(active, &[SessionPhase::Executing])?;
        active.phase = SessionPhase::Error;
        Ok(())
    }

    /// 取消 / 关闭会话（任意非 idle 态 → idle）。
    pub fn cancel(&mut self, session: &SessionId) -> AppResult<()> {
        self.get_mut(session)?;
        self.active = None;
        Ok(())
    }

    /// 关闭会话并释放选区 / 冻结帧（任意非 idle 态 → idle）。
    pub fn close(&mut self, session: &SessionId) -> AppResult<()> {
        self.cancel(session)
    }

    fn get_mut(&mut self, session: &SessionId) -> AppResult<&mut Session> {
        match self.active.as_mut() {
            Some(active) if &active.id == session => Ok(active),
            Some(active) => Err(AppError::invalid_state(format!(
                "会话 {} 不是当前活动会话（当前为 {}）",
                session, active.id
            ))),
            None => Err(AppError::invalid_state("当前没有活动会话")),
        }
    }
}

fn ensure_phase(session: &Session, allowed: &[SessionPhase]) -> AppResult<()> {
    if allowed.contains(&session.phase) {
        Ok(())
    } else {
        Err(AppError::invalid_state(format!(
            "会话处于 {} 阶段，不允许该操作（允许：{}）",
            session.phase.as_str(),
            allowed
                .iter()
                .map(SessionPhase::as_str)
                .collect::<Vec<_>>()
                .join(" / ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::types::{OcrBox, RectPhys};

    fn selection(session: &SessionId) -> SelectionResult {
        SelectionResult {
            session: session.clone(),
            rect_phys: RectPhys {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 40.0,
            },
            has_text: true,
            preview: "预览".to_string(),
            boxes: vec![OcrBox {
                text: "预览".to_string(),
                confidence: 0.9,
                bbox: RectPhys {
                    x: 10.0,
                    y: 20.0,
                    width: 30.0,
                    height: 40.0,
                },
            }],
        }
    }

    #[test]
    fn legal_flow() {
        let mut store = SessionStore::new();
        assert_eq!(store.current_phase(), SessionPhase::Idle);

        let start = store.begin_capture();
        assert_eq!(store.current_phase(), SessionPhase::Freezing);
        let session = start.session.clone();
        assert!(!start.replaced());

        store
            .attach_frame(
                &session,
                FrozenFrameMeta {
                    width: 1920,
                    height: 1080,
                    data_url: None,
                },
            )
            .unwrap();
        store.set_selecting(&session).unwrap();
        store
            .commit_selection(&session, selection(&session))
            .unwrap();
        assert_eq!(store.current_phase(), SessionPhase::Menu);

        let task = TaskId::from("task-1");
        store.begin_task(&session, task.clone()).unwrap();
        assert_eq!(store.current_phase(), SessionPhase::Executing);
        assert_eq!(store.active_task_ids(), vec![task]);

        store.task_finished(&session).unwrap();
        assert_eq!(store.current_phase(), SessionPhase::ShowingResult);

        // 追问：showingResult → executing
        let followup = TaskId::from("task-2");
        store.begin_task(&session, followup.clone()).unwrap();
        store.task_finished(&session).unwrap();
        assert_eq!(store.active_task_ids().len(), 2);

        store.close(&session).unwrap();
        assert_eq!(store.current_phase(), SessionPhase::Idle);
        assert!(store.current().is_none());
    }

    #[test]
    fn illegal_transitions_return_sa_1099() {
        let mut store = SessionStore::new();
        let idle = SessionId::from("idle-session");
        assert_eq!(store.cancel(&idle).unwrap_err().code, "SA-1099");

        let start = store.begin_capture();
        let session = start.session;
        assert_eq!(
            store
                .commit_selection(&session, selection(&session))
                .unwrap_err()
                .code,
            "SA-1099"
        );
        assert_eq!(
            store
                .begin_task(&session, TaskId::from("t"))
                .unwrap_err()
                .code,
            "SA-1099"
        );

        store.set_selecting(&session).unwrap();
        // 重复 set_selecting 非法
        assert_eq!(store.set_selecting(&session).unwrap_err().code, "SA-1099");

        store
            .commit_selection(&session, selection(&session))
            .unwrap();
        // menu 态不能再提交选区
        assert_eq!(
            store
                .commit_selection(&session, selection(&session))
                .unwrap_err()
                .code,
            "SA-1099"
        );

        // 会话 ID 不匹配
        assert_eq!(
            store
                .begin_task(&SessionId::from("other"), TaskId::from("t"))
                .unwrap_err()
                .code,
            "SA-1099"
        );
    }

    #[test]
    fn new_capture_invalidates_old_session_and_returns_tasks() {
        let mut store = SessionStore::new();
        let first = store.begin_capture();
        let session = first.session;
        store.set_selecting(&session).unwrap();
        store
            .commit_selection(&session, selection(&session))
            .unwrap();
        store
            .begin_task(&session, TaskId::from("old-task"))
            .unwrap();

        let second = store.begin_capture();
        assert!(second.replaced());
        assert_eq!(second.replaced_session.as_ref().unwrap(), &session);
        assert_eq!(second.replaced_tasks, vec![TaskId::from("old-task")]);
        assert!(!store.is_current(&session));
        assert!(store.is_current(&second.session));

        assert_eq!(store.task_finished(&session).unwrap_err().code, "SA-1099");
    }

    #[test]
    fn reset_selection_returns_to_selecting() {
        let mut store = SessionStore::new();
        let start = store.begin_capture();
        let session = start.session;
        store.set_selecting(&session).unwrap();
        store
            .commit_selection(&session, selection(&session))
            .unwrap();
        store.reset_selection(&session).unwrap();
        assert_eq!(store.current_phase(), SessionPhase::Selecting);
        assert!(store.current().unwrap().selection.is_none());
    }

    #[test]
    fn task_failure_enters_error_then_can_close() {
        let mut store = SessionStore::new();
        let start = store.begin_capture();
        let session = start.session;
        store.set_selecting(&session).unwrap();
        store
            .commit_selection(&session, selection(&session))
            .unwrap();
        store.begin_task(&session, TaskId::from("t")).unwrap();
        store.task_failed(&session).unwrap();
        assert_eq!(store.current_phase(), SessionPhase::Error);
        store.cancel(&session).unwrap();
        assert_eq!(store.current_phase(), SessionPhase::Idle);
    }

    #[test]
    fn phase_literals_match_contract() {
        assert_eq!(SessionPhase::Idle.as_str(), "idle");
        assert_eq!(SessionPhase::ShowingResult.as_str(), "showingResult");
    }
}
