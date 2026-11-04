//! 任务登记与取消（说明书 §3.1 M08 / 实施架构 §5.2）。
//!
//! 取消语义用 `Arc<AtomicBool>` 表达（等价于 CancellationToken 的最小实现），
//! 任务执行器在每个流式块之间检查，不再推送已取消任务的事件。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::contract::types::{ActionId, Mode, SessionId, TaskId};

/// 任务种类（决定路由到哪个网关）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskKind {
    /// 文本搜索（AI 或浏览器）。
    Search,
    /// 图片搜索。
    ImageSearch,
    /// 翻译。
    Translate,
    /// 解答。
    Answer,
    /// 复制文字。
    Copy,
    /// 追问。
    Followup,
}

impl TaskKind {
    /// 契约字面量。
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskKind::Search => "search",
            TaskKind::ImageSearch => "imageSearch",
            TaskKind::Translate => "translate",
            TaskKind::Answer => "answer",
            TaskKind::Copy => "copy",
            TaskKind::Followup => "followup",
        }
    }

    /// 由动作推导任务种类。
    pub fn from_action(action: ActionId) -> Self {
        match action {
            ActionId::Search => TaskKind::Search,
            ActionId::ImageSearch => TaskKind::ImageSearch,
            ActionId::Translate => TaskKind::Translate,
            ActionId::Answer => TaskKind::Answer,
            ActionId::Copy => TaskKind::Copy,
        }
    }
}

/// 任务记录。
#[derive(Debug, Clone)]
pub struct TaskRecord {
    /// 任务 ID。
    pub id: TaskId,
    /// 所属会话。
    pub session_id: SessionId,
    /// 动作。
    pub action: ActionId,
    /// 模式。
    pub mode: Mode,
    /// 开始时间（Unix 毫秒）。
    pub started_at: i64,
    /// 取消标记。
    pub cancelled: Arc<AtomicBool>,
}

impl TaskRecord {
    /// 创建任务记录。
    pub fn new(
        id: TaskId,
        session_id: SessionId,
        action: ActionId,
        mode: Mode,
    ) -> Self {
        Self {
            id,
            session_id,
            action,
            mode,
            started_at: chrono::Utc::now().timestamp_millis(),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 取消令牌（交给流式执行器）。
    pub fn cancel_token(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    /// 标记取消。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// 是否已取消。
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// 任务表。
#[derive(Debug, Default)]
pub struct TaskStore {
    tasks: HashMap<TaskId, TaskRecord>,
}

impl TaskStore {
    /// 创建空任务表。
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    /// 插入任务记录。
    pub fn insert(&mut self, record: TaskRecord) {
        self.tasks.insert(record.id.clone(), record);
    }

    /// 查询任务记录。
    pub fn get(&self, task: &TaskId) -> Option<&TaskRecord> {
        self.tasks.get(task)
    }

    /// 任务所属会话。
    pub fn session_of(&self, task: &TaskId) -> Option<SessionId> {
        self.tasks.get(task).map(|record| record.session_id.clone())
    }

    /// 任务的动作。
    pub fn action_of(&self, task: &TaskId) -> Option<ActionId> {
        self.tasks.get(task).map(|record| record.action)
    }

    /// 取取消令牌。
    pub fn cancel_token(&self, task: &TaskId) -> Option<Arc<AtomicBool>> {
        self.tasks.get(task).map(TaskRecord::cancel_token)
    }

    /// 取消任务；返回是否命中了活动任务。
    pub fn cancel(&mut self, task: &TaskId) -> bool {
        match self.tasks.get_mut(task) {
            Some(record) => {
                record.cancel();
                true
            }
            None => false,
        }
    }

    /// 取消某会话的全部任务（新会话 / 关闭结果窗时调用）。
    pub fn cancel_session(&mut self, session: &SessionId) -> Vec<TaskId> {
        let mut cancelled = Vec::new();
        for record in self.tasks.values_mut() {
            if &record.session_id == session {
                record.cancel();
                cancelled.push(record.id.clone());
            }
        }
        cancelled
    }

    /// 移除任务记录。
    pub fn remove(&mut self, task: &TaskId) -> Option<TaskRecord> {
        self.tasks.remove(task)
    }

    /// 活动任务数。
    pub fn active_count(&self) -> usize {
        self.tasks.len()
    }

    /// 全部任务 ID。
    pub fn ids(&self) -> Vec<TaskId> {
        self.tasks.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, session: &str) -> TaskRecord {
        TaskRecord::new(
            TaskId::from(id),
            SessionId::from(session),
            ActionId::Answer,
            Mode::Quick,
        )
    }

    #[test]
    fn insert_get_cancel_remove() {
        let mut store = TaskStore::new();
        let task = record("t-1", "s-1");
        let token = task.cancel_token();
        store.insert(task);
        assert_eq!(store.active_count(), 1);
        assert_eq!(store.session_of(&TaskId::from("t-1")).unwrap(), SessionId::from("s-1"));
        assert!(!token.load(Ordering::SeqCst));

        assert!(store.cancel(&TaskId::from("t-1")));
        assert!(token.load(Ordering::SeqCst));
        assert!(store.get(&TaskId::from("t-1")).unwrap().is_cancelled());
        assert!(!store.cancel(&TaskId::from("missing")));

        assert!(store.remove(&TaskId::from("t-1")).is_some());
        assert_eq!(store.active_count(), 0);
    }

    #[test]
    fn cancel_session_cancels_only_that_session() {
        let mut store = TaskStore::new();
        store.insert(record("t-1", "s-1"));
        store.insert(record("t-2", "s-1"));
        store.insert(record("t-3", "s-2"));

        let cancelled = store.cancel_session(&SessionId::from("s-1"));
        assert_eq!(cancelled.len(), 2);
        assert!(store.get(&TaskId::from("t-3")).unwrap().is_cancelled() == false);
        assert!(store.get(&TaskId::from("t-1")).unwrap().is_cancelled());
    }

    #[test]
    fn task_kind_mapping() {
        assert_eq!(TaskKind::from_action(ActionId::Search), TaskKind::Search);
        assert_eq!(
            TaskKind::from_action(ActionId::ImageSearch),
            TaskKind::ImageSearch
        );
        assert_eq!(TaskKind::from_action(ActionId::Copy), TaskKind::Copy);
        assert_eq!(TaskKind::Followup.as_str(), "followup");
    }
}
