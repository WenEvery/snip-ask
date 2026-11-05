//! 多轮追问的会话内轮次存储（契约 v4 / 研究 §3.2-2）。
//!
//! 只在内存中保存「本会话已完成的问答」，供 `task_followup` 把最近 N 轮注入
//! `followup.md` 的 `{context}`；**不落库**（研究 §3：持久化属完整版，需 DB 迁移与隐私评审）。
//!
//! 纯逻辑、无 IO，可单测。

use std::collections::HashMap;

use crate::contract::types::{SessionId, TaskId};

/// 追问上下文总预算（字符）。研究 §3.7 建议 ≤ 6000，这里取 5000（中文约 0.6 token/字）。
pub const FOLLOWUP_CONTEXT_MAX_CHARS: usize = 5000;

/// 追问上下文里选区原文的最大字符数（研究 §3.7 建议 2000）。
pub const FOLLOWUP_SELECTION_MAX_CHARS: usize = 2000;

/// 按字符数截断（CJK 安全），超长时补省略号。
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let mut truncated: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

/// 一轮已完成的问答。
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// 任务 ID。
    pub task: TaskId,
    /// 首轮 = 选区文本；追问 = 用户问题。
    pub question: String,
    /// 回答正文（流式累积后的完整文本）。
    pub answer: String,
    /// 实际使用的引擎。
    pub engine: String,
    /// 创建时间（Unix 毫秒）。
    pub created_at: i64,
    /// 父任务（首轮为 `None`）。
    pub parent: Option<TaskId>,
}

impl Turn {
    /// 用当前时间构造。
    pub fn now(
        task: TaskId,
        question: String,
        answer: String,
        engine: String,
        parent: Option<TaskId>,
    ) -> Self {
        Self {
            task,
            question,
            answer,
            engine,
            created_at: chrono::Utc::now().timestamp_millis(),
            parent,
        }
    }
}

/// 每会话的轮次表（内存）。
#[derive(Debug, Default)]
pub struct TurnStore {
    sessions: HashMap<SessionId, Vec<Turn>>,
}

impl TurnStore {
    /// 空表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 开启新线程：清空该会话历史（首轮任务 / 重答 / 新框选时调用）。
    pub fn begin(&mut self, session: &SessionId) {
        self.sessions.remove(session);
    }

    /// 追加一轮问答。
    pub fn record(&mut self, session: &SessionId, turn: Turn) {
        self.sessions.entry(session.clone()).or_default().push(turn);
    }

    /// 该会话的轮次快照。
    pub fn turns(&self, session: &SessionId) -> Vec<Turn> {
        self.sessions.get(session).cloned().unwrap_or_default()
    }

    /// 最近一次已完成任务 ID（追问定位用）。
    pub fn last_task(&self, session: &SessionId) -> Option<TaskId> {
        self.sessions
            .get(session)
            .and_then(|turns| turns.last())
            .map(|turn| turn.task.clone())
    }

    /// 追问上下文：最近若干轮「问 + 答」，按 `max_chars` 从旧到新裁剪（保留最新）。
    pub fn context_for_followup(&self, session: &SessionId, max_chars: usize) -> String {
        let turns = self.turns(session);
        if turns.is_empty() || max_chars == 0 {
            return String::new();
        }
        let mut blocks: Vec<String> = turns
            .iter()
            .enumerate()
            .map(|(index, turn)| {
                format!(
                    "【第 {} 轮】\n问：{}\n答：{}",
                    index + 1,
                    turn.question.trim(),
                    turn.answer.trim()
                )
            })
            .collect();
        // 从最新往旧累加，超预算即停止，最后反转回时间顺序。
        let mut selected: Vec<String> = Vec::new();
        let mut used = 0usize;
        for block in blocks.iter().rev() {
            let cost = block.chars().count() + 2;
            if used + cost > max_chars && !selected.is_empty() {
                break;
            }
            used += cost;
            selected.push(block.clone());
        }
        selected.reverse();
        blocks.clear();
        selected.join("\n\n")
    }

    /// 清空该会话（重答/关闭结果窗时调用）。
    pub fn clear(&mut self, session: &SessionId) {
        self.sessions.remove(session);
    }

    /// 丢弃空会话（关闭会话时调用）。
    pub fn drop_session(&mut self, session: &SessionId) {
        self.sessions.remove(session);
    }

    /// 只保留指定会话的历史（新框选/首轮任务时清理过期会话，避免内存常驻）。
    pub fn prune_except(&mut self, keep: &SessionId) {
        self.sessions.retain(|session, _| session == keep);
    }
}

/// 组装 `followup.md` 的 `{context}`：**选区原文 + 最近 N 轮问答**（研究 §3.7 MVP）。
///
/// - 选区最多 [`FOLLOWUP_SELECTION_MAX_CHARS`] 字；
/// - 轮次按「从新到旧」分配剩余预算（[`TurnStore::context_for_followup`]），保证最新一轮不被裁掉；
/// - 总长不超过 `max_chars`（0 表示不注入上下文）。
///
/// 纯函数，便于单测；`{question}` 仍是本次追问，不在此处。
pub fn build_followup_context(
    store: &TurnStore,
    session: &SessionId,
    selection: &str,
    max_chars: usize,
) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let selection_text = truncate_chars(selection.trim(), FOLLOWUP_SELECTION_MAX_CHARS);
    let selection_part = if selection_text.is_empty() {
        String::new()
    } else {
        format!("【框选内容】\n{selection_text}")
    };
    let turns_header = "【最近的问答】";
    let overhead = selection_part.chars().count() + turns_header.chars().count() + 2;
    let turns_budget = max_chars.saturating_sub(overhead);
    let turns = store.context_for_followup(session, turns_budget);
    match (selection_part.is_empty(), turns.is_empty()) {
        (true, true) => String::new(),
        (false, true) => truncate_chars(&selection_part, max_chars),
        (true, false) => truncate_chars(&format!("{turns_header}\n{turns}"), max_chars),
        (false, false) => truncate_chars(
            &format!("{selection_part}\n\n{turns_header}\n{turns}"),
            max_chars,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(index: usize, question: &str, answer: &str) -> Turn {
        Turn::now(
            TaskId::from(format!("t-{index}")),
            question.to_string(),
            answer.to_string(),
            "deepseek".to_string(),
            if index == 0 {
                None
            } else {
                Some(TaskId::from(format!("t-{}", index - 1)))
            },
        )
    }

    #[test]
    fn records_and_reports_last_task() {
        let mut store = TurnStore::new();
        let session = SessionId::from("s-1");
        store.begin(&session);
        assert_eq!(store.last_task(&session), None);
        store.record(&session, turn(0, "选区文本", "首轮回答"));
        store.record(&session, turn(1, "继续解释", "追问回答"));
        assert_eq!(store.turns(&session).len(), 2);
        assert_eq!(store.last_task(&session), Some(TaskId::from("t-1")));
    }

    #[test]
    fn context_keeps_latest_turns_within_budget() {
        let mut store = TurnStore::new();
        let session = SessionId::from("s-2");
        store.record(&session, turn(0, "q0", &"a0".repeat(50)));
        store.record(&session, turn(1, "q1", &"a1".repeat(50)));
        store.record(&session, turn(2, "q2", &"a2".repeat(50)));
        let full = store.context_for_followup(&session, 100_000);
        assert!(full.contains("第 1 轮") && full.contains("第 3 轮"));
        assert!(full.find("第 1 轮") < full.find("第 3 轮"));

        let truncated = store.context_for_followup(&session, 120);
        assert!(
            truncated.contains("第 3 轮"),
            "必须保留最新一轮：{truncated}"
        );
        assert!(!truncated.contains("第 1 轮"));
        assert!(truncated.chars().count() < full.chars().count());
    }

    #[test]
    fn begin_and_clear_reset_session() {
        let mut store = TurnStore::new();
        let session = SessionId::from("s-3");
        store.record(&session, turn(0, "q", "a"));
        store.clear(&session);
        assert!(store.turns(&session).is_empty());
        store.record(&session, turn(0, "q", "a"));
        store.begin(&session);
        assert!(store.turns(&session).is_empty());
    }

    #[test]
    fn prune_except_keeps_only_current_session() {
        let mut store = TurnStore::new();
        store.record(&SessionId::from("s-old"), turn(0, "q", "a"));
        store.record(&SessionId::from("s-new"), turn(1, "q2", "a2"));
        store.prune_except(&SessionId::from("s-new"));
        assert!(store.turns(&SessionId::from("s-old")).is_empty());
        assert_eq!(store.turns(&SessionId::from("s-new")).len(), 1);
    }

    #[test]
    fn followup_context_contains_previous_answer() {
        let mut store = TurnStore::new();
        let session = SessionId::from("s-4");
        store.record(
            &session,
            turn(
                0,
                "选区原文：请解释这段代码",
                "上一轮回答：这段代码做了三件事……",
            ),
        );
        let context = build_followup_context(&store, &session, "选区原文：请解释这段代码", 5000);
        assert!(context.contains("【框选内容】"), "{context}");
        assert!(
            context.contains("上一轮回答：这段代码做了三件事"),
            "{context}"
        );
        assert!(context.contains("第 1 轮"), "{context}");
    }

    #[test]
    fn followup_context_truncates_selection_and_respects_budget() {
        let mut store = TurnStore::new();
        let session = SessionId::from("s-5");
        store.record(&session, turn(0, "q0", &"a0".repeat(200)));
        let long_selection = "选区".repeat(3000);
        let context = build_followup_context(
            &store,
            &session,
            &long_selection,
            FOLLOWUP_CONTEXT_MAX_CHARS,
        );
        assert!(
            context.chars().count() <= FOLLOWUP_CONTEXT_MAX_CHARS,
            "{}",
            context.chars().count()
        );
        // 最新一轮回答必须保留（预算优先给轮次）。
        assert!(context.contains("第 1 轮"));

        let empty = build_followup_context(&TurnStore::new(), &session, "", 5000);
        assert!(empty.is_empty());
        assert!(build_followup_context(&store, &session, "选区", 0).is_empty());
    }
}
