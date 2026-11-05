//! 历史与用量仓储（M17 / 契约 §10 / 说明书 §6.5 / spec v1 §3.3-§3.4）。
//!
//! - SQLite 路径 `<data>/history.db`；迁移脚本按版本增量执行（`migrations/001_init.sql` →
//!   `migrations/002_messages.sql`），`schema_migrations` 记录已应用版本，兼容 v0.1 旧库；
//! - `history.enabled = false` 时所有写入 / 查询为 no-op；
//! - 打开失败只记 `warn`，不 panic、不阻断启动（降级为无历史）；
//! - 日志禁止记录 OCR 原文 / 问答正文（本模块只写库，不写日志内容）。
//!
//! 连续提问历史（契约 v5）：
//! - 每轮问答落两条 `messages`（role = `user` / `assistant`），`(task_id, role)` 唯一，重试幂等；
//! - `list_sessions` 按 session 聚合：首条 user 消息做 title（≤60 字符）、turns = 不同 task 数、
//!   engine 取最近一条 assistant 消息的引擎；
//! - `session_thread` 按时间正序返回完整问答线程（重启后由前端回溯展示）。

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rusqlite::{params, Connection};

use crate::contract::types::{
    ActionId, HistorySession, HistoryThread, HistoryTurn, Mode, RectPhys, SessionId, TaskId,
    TokenUsage,
};
use crate::domain::task::TaskRecord;
use crate::domain::turn::truncate_chars;
use crate::platform::paths::AppPaths;

/// 迁移脚本：`(版本, SQL)`，按版本升序增量执行。
///
/// `001` 保留原有「一次性建表」脚本（全部 `IF NOT EXISTS` / `INSERT OR IGNORE`，可重复执行）；
/// `002` 新增 messages 表与索引。
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../../migrations/001_init.sql")),
    (2, include_str!("../../migrations/002_messages.sql")),
];

/// 迁移元数据表（旧库已由 001 建好；损坏/更旧库兜底）。
const SCHEMA_BOOTSTRAP: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (
  version    INTEGER PRIMARY KEY,
  applied_at INTEGER NOT NULL
);";

/// 历史会话标题最大字符数（契约 §3.3：≤60 字；CJK 安全截断，超长补省略号）。
pub const HISTORY_TITLE_MAX_CHARS: usize = 60;

/// 写入多少次消息后触发一次节流清理（保留策略来自最近一次 [`HistoryRepo::cleanup`]）。
const CLEANUP_EVERY_WRITES: u64 = 64;

/// 用量汇总。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageSummary {
    /// 任务数。
    pub tasks: i64,
    /// 输入 token。
    pub tokens_in: i64,
    /// 输出 token。
    pub tokens_out: i64,
    /// 估算费用（人民币）。
    pub cost_est: f64,
}

/// 消息角色（契约 v5 历史线程）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    /// 用户输入（首轮 = 选区原文 / 追问 = 问题）。
    User,
    /// AI 回答。
    Assistant,
}

impl MessageRole {
    /// 数据库字面量。
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        }
    }

    /// 从数据库字面量解析。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(MessageRole::User),
            "assistant" => Some(MessageRole::Assistant),
            _ => None,
        }
    }
}

/// 单条历史消息（history_repo 内部输入结构，非 IPC 契约）。
///
/// T3（task_runner）通常直接用 [`HistoryRepo::record_turn`] 一次写两条；
/// 需要单独写一条（如只记录问题 / 只记录回答）时才用 [`HistoryRepo::record_message`]。
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryMessage {
    /// 所属会话。
    pub session: SessionId,
    /// 所属任务（一轮问答 = 一个 task）。
    pub task: TaskId,
    /// 动作。
    pub action: ActionId,
    /// 模式。
    pub mode: Mode,
    /// 角色。
    pub role: MessageRole,
    /// 正文（问题 / 回答）。
    pub content: String,
    /// 引擎（问题通常为 `None`）。
    pub engine: Option<String>,
    /// 状态：`done` / `error` / `cancelled`。
    pub status: String,
    /// 错误码（失败时）。
    pub error_code: Option<String>,
    /// 时间（Unix 毫秒）。
    pub created_at: i64,
}

impl HistoryMessage {
    /// 构造一条 user 消息（时间取当前）。
    pub fn user(
        session: SessionId,
        task: TaskId,
        action: ActionId,
        mode: Mode,
        content: impl Into<String>,
    ) -> Self {
        Self::new(session, task, action, mode, MessageRole::User, content)
    }

    /// 构造一条 assistant 消息（时间取当前）。
    pub fn assistant(
        session: SessionId,
        task: TaskId,
        action: ActionId,
        mode: Mode,
        content: impl Into<String>,
    ) -> Self {
        Self::new(session, task, action, mode, MessageRole::Assistant, content)
    }

    fn new(
        session: SessionId,
        task: TaskId,
        action: ActionId,
        mode: Mode,
        role: MessageRole,
        content: impl Into<String>,
    ) -> Self {
        Self {
            session,
            task,
            action,
            mode,
            role,
            content: content.into(),
            engine: None,
            status: "done".to_string(),
            error_code: None,
            created_at: chrono::Utc::now().timestamp_millis(),
        }
    }
}

/// 历史仓储。
pub struct HistoryRepo {
    conn: parking_lot::Mutex<Option<Connection>>,
    enabled: AtomicBool,
    /// 写入计数（触发节流清理）。
    write_count: AtomicU64,
    /// 保留策略打包：`高 32 位 = max_entries`、`低 32 位 = retention_days`；0 = 未设置。
    cleanup_policy: AtomicU64,
}

impl HistoryRepo {
    /// 打开历史库；失败时降级为无历史（只记 warn）。
    pub fn open(paths: &AppPaths, enabled: bool) -> Self {
        if !enabled {
            return Self {
                conn: parking_lot::Mutex::new(None),
                enabled: AtomicBool::new(false),
                write_count: AtomicU64::new(0),
                cleanup_policy: AtomicU64::new(0),
            };
        }
        match Self::try_open(&paths.history_db) {
            Ok(conn) => Self {
                conn: parking_lot::Mutex::new(Some(conn)),
                enabled: AtomicBool::new(true),
                write_count: AtomicU64::new(0),
                cleanup_policy: AtomicU64::new(0),
            },
            Err(err) => {
                tracing::warn!(error = %err, "历史库打开失败，历史与用量功能降级为禁用");
                Self {
                    conn: parking_lot::Mutex::new(None),
                    enabled: AtomicBool::new(false),
                    write_count: AtomicU64::new(0),
                    cleanup_policy: AtomicU64::new(0),
                }
            }
        }
    }

    fn try_open(path: &Path) -> Result<Connection, rusqlite::Error> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        Self::apply_migrations(&conn)?;
        Ok(conn)
    }

    /// 增量执行迁移：只应用 `schema_migrations` 里没有的版本。
    fn apply_migrations(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(SCHEMA_BOOTSTRAP)?;
        let applied: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        for (version, sql) in MIGRATIONS {
            if *version <= applied {
                continue;
            }
            conn.execute_batch(sql)?;
        }
        Ok(())
    }

    /// 是否启用。
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// 开关（配置热更新时调用）。
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    // ------------------------------------------------------------------
    // 会话 / 任务 / 用量（既有能力，行为保持不变）
    // ------------------------------------------------------------------

    /// 记录会话开始。
    pub fn record_session(
        &self,
        id: &SessionId,
        started_at: i64,
        rect: Option<&RectPhys>,
        has_text: bool,
        ocr_text: Option<&str>,
        app_exe: Option<&str>,
    ) {
        if !self.is_enabled() {
            return;
        }
        let rect_text =
            rect.map(|rect| format!("{},{},{},{}", rect.x, rect.y, rect.width, rect.height));
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return;
        };
        if let Err(err) = conn.execute(
            "INSERT OR REPLACE INTO sessions (id, started_at, rect, has_text, ocr_text, app_exe)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id.as_str(),
                started_at,
                rect_text,
                if has_text { 1 } else { 0 },
                ocr_text,
                app_exe
            ],
        ) {
            tracing::warn!(error = %err, "写入会话历史失败");
        }
    }

    /// 记录任务结束。
    pub fn record_task(
        &self,
        record: &TaskRecord,
        engine: Option<&str>,
        status: &str,
        first_token_ms: Option<u64>,
        total_ms: Option<u64>,
        usage: Option<&TokenUsage>,
        error_code: Option<&str>,
    ) {
        if !self.is_enabled() {
            return;
        }
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return;
        };
        let created_at = record.started_at;
        let tokens_in = usage.map(|usage| usage.input_tokens as i64);
        let tokens_out = usage.map(|usage| usage.output_tokens as i64);
        let cost_est = usage.map(|usage| usage.cost_estimate);
        if let Err(err) = conn.execute(
            "INSERT OR REPLACE INTO tasks
             (id, session_id, action, mode, engine, status, first_token_ms, total_ms,
              tokens_in, tokens_out, cost_est, error_code, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                record.id.as_str(),
                record.session_id.as_str(),
                record.action.as_str(),
                record.mode.as_str(),
                engine,
                status,
                first_token_ms.map(|value| value as i64),
                total_ms.map(|value| value as i64),
                tokens_in,
                tokens_out,
                cost_est,
                error_code,
                created_at
            ],
        ) {
            tracing::warn!(error = %err, "写入任务历史失败");
            return;
        }
        if let Some(usage) = usage {
            let _ = self.record_usage_with_conn(conn, usage);
        }
    }

    /// 按日累计用量。
    pub fn record_usage(&self, usage: &TokenUsage) {
        if !self.is_enabled() {
            return;
        }
        let guard = self.conn.lock();
        if let Some(conn) = guard.as_ref() {
            let _ = self.record_usage_with_conn(conn, usage);
        }
    }

    fn record_usage_with_conn(
        &self,
        conn: &Connection,
        usage: &TokenUsage,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO usage_daily (day, tasks, tokens_in, tokens_out, cost_est)
             VALUES (date('now', 'localtime'), 1, ?1, ?2, ?3)
             ON CONFLICT(day) DO UPDATE SET
               tasks = tasks + 1,
               tokens_in = tokens_in + excluded.tokens_in,
               tokens_out = tokens_out + excluded.tokens_out,
               cost_est = cost_est + excluded.cost_est",
            params![
                usage.input_tokens as i64,
                usage.output_tokens as i64,
                usage.cost_estimate
            ],
        )
        .map(|_| ())
    }

    /// 今日用量。
    pub fn stats_today(&self) -> UsageSummary {
        self.query_summary("SELECT COALESCE(SUM(tasks),0), COALESCE(SUM(tokens_in),0), COALESCE(SUM(tokens_out),0), COALESCE(SUM(cost_est),0) FROM usage_daily WHERE day = date('now','localtime')")
    }

    /// 本月用量。
    pub fn stats_month(&self) -> UsageSummary {
        self.query_summary("SELECT COALESCE(SUM(tasks),0), COALESCE(SUM(tokens_in),0), COALESCE(SUM(tokens_out),0), COALESCE(SUM(cost_est),0) FROM usage_daily WHERE day >= date('now','start of month','localtime')")
    }

    fn query_summary(&self, sql: &str) -> UsageSummary {
        if !self.is_enabled() {
            return UsageSummary::default();
        }
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return UsageSummary::default();
        };
        let result = conn.query_row(sql, (), |row| {
            Ok(UsageSummary {
                tasks: row.get(0)?,
                tokens_in: row.get(1)?,
                tokens_out: row.get(2)?,
                cost_est: row.get(3)?,
            })
        });
        match result {
            Ok(summary) => summary,
            Err(err) => {
                tracing::warn!(error = %err, "查询用量失败");
                UsageSummary::default()
            }
        }
    }

    // ------------------------------------------------------------------
    // 连续提问历史（契约 v5 / spec §3.3-§3.4）
    // ------------------------------------------------------------------

    /// 记录单条消息（spec §3.4）。
    ///
    /// - `history.enabled = false` → no-op；
    /// - 会话行不存在时自动补建（`sessions` 表），保证历史列表的 `startedAt` 有依据；
    /// - `(task_id, role)` 唯一：重复落库按覆盖处理（幂等）。
    pub fn record_message(&self, message: &HistoryMessage) {
        if !self.is_enabled() {
            return;
        }
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return;
        };
        let written = (|| -> rusqlite::Result<()> {
            let tx = conn.unchecked_transaction()?;
            ensure_session_row(&tx, &message.session, message.created_at)?;
            insert_message_row(&tx, message)?;
            tx.commit()
        })();
        match written {
            Ok(()) => self.after_write(conn),
            Err(err) => tracing::warn!(error = %err, "写入历史消息失败"),
        }
    }

    /// 记录一轮问答（user + assistant 各一条）——T3 `task_runner` 的便捷入口。
    ///
    /// - `question`：首轮 = 选区输入原文；追问 = 用户问题；
    /// - `answer`：累积完整回答（`status = "error"` 时可为空串）；
    /// - `engine`：引擎未知时传 `None`；
    /// - `status`：`done` / `error` / `cancelled`（空串按 `done`）；
    /// - `error_code`：失败时的契约错误码；
    /// - 返回实际写入条数（`0` = 未启用 / 打开失败），`2` = 正常。
    #[allow(clippy::too_many_arguments)]
    pub fn record_turn(
        &self,
        session: &SessionId,
        task: &TaskId,
        action: ActionId,
        mode: Mode,
        question: &str,
        answer: &str,
        engine: Option<&str>,
        status: &str,
        error_code: Option<&str>,
    ) -> usize {
        if !self.is_enabled() {
            return 0;
        }
        let now = chrono::Utc::now().timestamp_millis();
        let status = if status.is_empty() { "done" } else { status };
        let error_code = error_code.map(str::to_string);
        let user = HistoryMessage {
            session: session.clone(),
            task: task.clone(),
            action,
            mode,
            role: MessageRole::User,
            content: question.to_string(),
            engine: None,
            status: status.to_string(),
            error_code: error_code.clone(),
            created_at: now,
        };
        let assistant = HistoryMessage {
            session: session.clone(),
            task: task.clone(),
            action,
            mode,
            role: MessageRole::Assistant,
            content: answer.to_string(),
            engine: engine.map(str::to_string),
            status: status.to_string(),
            error_code,
            created_at: now,
        };
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return 0;
        };
        let written = (|| -> rusqlite::Result<usize> {
            let tx = conn.unchecked_transaction()?;
            ensure_session_row(&tx, session, now)?;
            insert_message_row(&tx, &user)?;
            insert_message_row(&tx, &assistant)?;
            tx.commit()?;
            Ok(2)
        })();
        match written {
            Ok(count) => {
                self.after_write(conn);
                count
            }
            Err(err) => {
                tracing::warn!(error = %err, "写入历史问答失败");
                0
            }
        }
    }

    /// 历史会话列表（按 `startedAt` 倒序；契约 v5 `history_list`）。
    ///
    /// 以 messages 为准聚合（没有问答的会话不展示）；title 取该会话最早一条 user 消息，≤60 字符。
    pub fn list_sessions(&self, limit: u32, offset: u32) -> Vec<HistorySession> {
        if !self.is_enabled() || limit == 0 {
            return Vec::new();
        }
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return Vec::new();
        };
        let mut statement = match conn.prepare(LIST_SESSIONS_SQL) {
            Ok(statement) => statement,
            Err(err) => {
                tracing::warn!(error = %err, "查询历史会话失败");
                return Vec::new();
            }
        };
        let rows = statement.query_map(
            params![i64::from(limit), i64::from(offset)],
            map_session_row,
        );
        match rows {
            Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
            Err(err) => {
                tracing::warn!(error = %err, "查询历史会话失败");
                Vec::new()
            }
        }
    }

    /// 单个会话的完整问答线程（契约 v5 `history_thread`）；不存在 / 未启用 → `None`。
    pub fn session_thread(&self, session: &SessionId) -> Option<HistoryThread> {
        if !self.is_enabled() {
            return None;
        }
        let guard = self.conn.lock();
        let conn = guard.as_ref()?;
        let summary = Self::query_session_summary(conn, session).ok()?;
        let turns = Self::query_turns(conn, session).unwrap_or_default();
        Some(HistoryThread {
            session: summary,
            turns,
        })
    }

    fn query_session_summary(
        conn: &Connection,
        session: &SessionId,
    ) -> rusqlite::Result<HistorySession> {
        conn.query_row(
            SESSION_SUMMARY_SQL,
            params![session.as_str()],
            map_session_row,
        )
    }

    fn query_turns(conn: &Connection, session: &SessionId) -> rusqlite::Result<Vec<HistoryTurn>> {
        let mut statement = conn.prepare(TURNS_SQL)?;
        let mut rows = statement.query(params![session.as_str()])?;
        let mut order: Vec<String> = Vec::new();
        let mut turns: std::collections::HashMap<String, HistoryTurn> =
            std::collections::HashMap::new();
        while let Some(row) = rows.next()? {
            let task_id: String = row.get(0)?;
            let action: String = row.get(1)?;
            let mode: String = row.get(2)?;
            let role: String = row.get(3)?;
            let content: String = row.get(4)?;
            let engine: Option<String> = row.get(5)?;
            let status: String = row.get(6)?;
            let created_at: i64 = row.get(7)?;
            let entry = turns.entry(task_id.clone()).or_insert_with(|| {
                order.push(task_id.clone());
                HistoryTurn {
                    task: TaskId::from(task_id.clone()),
                    action: ActionId::parse(&action).unwrap_or(ActionId::Answer),
                    mode: Mode::parse(&mode).unwrap_or(Mode::Quick),
                    question: String::new(),
                    answer: String::new(),
                    engine: engine.clone(),
                    status: status.clone(),
                    created_at,
                }
            });
            match MessageRole::parse(&role) {
                Some(MessageRole::User) => entry.question = content,
                Some(MessageRole::Assistant) => {
                    entry.answer = content;
                    entry.status = status;
                    if engine.is_some() {
                        entry.engine = engine;
                    }
                }
                None => {}
            }
        }
        Ok(order
            .into_iter()
            .filter_map(|task_id| turns.remove(&task_id))
            .collect())
    }

    /// 清空历史（契约 v5 `history_clear`）。
    ///
    /// - `Some(session)`：清该会话的消息 + 任务 + 会话行（保留按日用量聚合）；
    /// - `None`：清全部历史（messages / tasks / sessions / usage_daily，与设置页「一键清空」一致）；
    /// - 返回删除的 messages 行数；未启用 → 0。
    pub fn clear_history(&self, session: Option<&SessionId>) -> usize {
        if !self.is_enabled() {
            return 0;
        }
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return 0;
        };
        match session {
            Some(id) => {
                let deleted = conn
                    .execute(
                        "DELETE FROM messages WHERE session_id = ?1",
                        params![id.as_str()],
                    )
                    .unwrap_or(0);
                let _ = conn.execute(
                    "DELETE FROM tasks WHERE session_id = ?1",
                    params![id.as_str()],
                );
                let _ = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id.as_str()]);
                deleted
            }
            None => {
                let deleted = conn.execute("DELETE FROM messages", []).unwrap_or(0);
                let _ = conn.execute_batch(
                    "DELETE FROM tasks; DELETE FROM sessions; DELETE FROM usage_daily;",
                );
                deleted
            }
        }
    }

    /// 消息条数（测试 / 诊断用）。
    pub fn message_count(&self) -> i64 {
        if !self.is_enabled() {
            return 0;
        }
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return 0;
        };
        conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap_or(0)
    }

    /// 写入后节流清理（每 [`CLEANUP_EVERY_WRITES`] 条消息一次）。
    fn after_write(&self, conn: &Connection) {
        let count = self.write_count.fetch_add(1, Ordering::Relaxed) + 1;
        if count % CLEANUP_EVERY_WRITES != 0 {
            return;
        }
        let policy = self.cleanup_policy.load(Ordering::Relaxed);
        if policy == 0 {
            return;
        }
        let max_entries = (policy >> 32) as u32;
        let retention_days = policy as u32;
        self.cleanup_with_conn(conn, max_entries, retention_days);
    }

    // ------------------------------------------------------------------
    // 清理
    // ------------------------------------------------------------------

    /// 清理：按保留条数与保留天数裁剪（`max_entries = 0` 表示不限）。
    ///
    /// 同时记录保留策略，供写入路径的节流清理复用（启动 / 退出时由 Lead 调用即可）。
    pub fn cleanup(&self, max_entries: u32, retention_days: u32) {
        self.cleanup_policy.store(
            (u64::from(max_entries) << 32) | u64::from(retention_days),
            Ordering::Relaxed,
        );
        if !self.is_enabled() {
            return;
        }
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return;
        };
        self.cleanup_with_conn(conn, max_entries, retention_days);
    }

    fn cleanup_with_conn(&self, conn: &Connection, max_entries: u32, retention_days: u32) {
        if retention_days > 0 {
            let _ = conn.execute(
                "DELETE FROM messages WHERE created_at < (strftime('%s','now') - ?1*86400) * 1000",
                params![retention_days as i64],
            );
            let _ = conn.execute(
                "DELETE FROM tasks WHERE created_at < (strftime('%s','now') - ?1*86400) * 1000",
                params![retention_days as i64],
            );
            let _ = conn.execute(
                "DELETE FROM sessions WHERE started_at < (strftime('%s','now') - ?1*86400) * 1000",
                params![retention_days as i64],
            );
        }
        if max_entries > 0 {
            // 保留最新 max_entries 轮对应的消息：先按「将被裁剪的 task」删消息，再删 task，
            // 这样既不会把「task 行缺失的孤儿消息」误删，也不会漏删被裁剪轮的正文。
            let _ = conn.execute(
                "DELETE FROM messages WHERE task_id IN (
                    SELECT id FROM tasks WHERE id NOT IN (
                      SELECT id FROM tasks ORDER BY created_at DESC LIMIT ?1
                    )
                 )",
                params![max_entries as i64],
            );
            let _ = conn.execute(
                "DELETE FROM tasks WHERE id NOT IN (
                    SELECT id FROM tasks ORDER BY created_at DESC LIMIT ?1
                 )",
                params![max_entries as i64],
            );
        }
    }

    /// 清空全部历史（设置页「一键清空」；行为与 [`HistoryRepo::clear_history(None)`] 一致）。
    pub fn clear(&self) {
        let _ = self.clear_history(None);
    }

    /// 最近任务数（诊断 / 测试用）。
    pub fn task_count(&self) -> i64 {
        if !self.is_enabled() {
            return 0;
        }
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return 0;
        };
        conn.query_row("SELECT COUNT(*) FROM tasks", (), |row| row.get(0))
            .unwrap_or(0)
    }

    /// 某会话的任务 ID 列表（测试用）。
    pub fn task_ids_for_session(&self, session: &SessionId) -> Vec<TaskId> {
        if !self.is_enabled() {
            return Vec::new();
        }
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return Vec::new();
        };
        let mut statement = match conn.prepare("SELECT id FROM tasks WHERE session_id = ?1") {
            Ok(statement) => statement,
            Err(_) => return Vec::new(),
        };
        let rows = statement.query_map(params![session.as_str()], |row| row.get::<_, String>(0));
        match rows {
            Ok(rows) => rows.filter_map(|row| row.ok()).map(TaskId::from).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// 会话列表聚合 SQL（`?1 = limit`、`?2 = offset`）。
const LIST_SESSIONS_SQL: &str = "
SELECT
  m.session_id,
  COALESCE(
    (SELECT s.started_at FROM sessions s WHERE s.id = m.session_id),
    MIN(m.created_at)
  ) AS started_at,
  (SELECT u.content FROM messages u
     WHERE u.session_id = m.session_id AND u.role = 'user'
     ORDER BY u.created_at ASC, u.rowid ASC LIMIT 1) AS title,
  (SELECT u.action FROM messages u
     WHERE u.session_id = m.session_id AND u.role = 'user'
     ORDER BY u.created_at ASC, u.rowid ASC LIMIT 1) AS action,
  COUNT(DISTINCT m.task_id) AS turns,
  (SELECT a.engine FROM messages a
     WHERE a.session_id = m.session_id AND a.role = 'assistant'
     ORDER BY a.created_at DESC, a.rowid DESC LIMIT 1) AS engine
FROM messages m
GROUP BY m.session_id
ORDER BY started_at DESC
LIMIT ?1 OFFSET ?2
";

/// 单会话摘要 SQL（`?1 = session_id`）。
const SESSION_SUMMARY_SQL: &str = "
SELECT
  m.session_id,
  COALESCE(
    (SELECT s.started_at FROM sessions s WHERE s.id = m.session_id),
    MIN(m.created_at)
  ) AS started_at,
  (SELECT u.content FROM messages u
     WHERE u.session_id = m.session_id AND u.role = 'user'
     ORDER BY u.created_at ASC, u.rowid ASC LIMIT 1) AS title,
  (SELECT u.action FROM messages u
     WHERE u.session_id = m.session_id AND u.role = 'user'
     ORDER BY u.created_at ASC, u.rowid ASC LIMIT 1) AS action,
  COUNT(DISTINCT m.task_id) AS turns,
  (SELECT a.engine FROM messages a
     WHERE a.session_id = m.session_id AND a.role = 'assistant'
     ORDER BY a.created_at DESC, a.rowid DESC LIMIT 1) AS engine
FROM messages m
WHERE m.session_id = ?1
GROUP BY m.session_id
";

/// 线程消息 SQL（`?1 = session_id`）：时间正序，同轮 user 在 assistant 之前。
const TURNS_SQL: &str = "
SELECT task_id, action, mode, role, content, engine, status, created_at
FROM messages
WHERE session_id = ?1
ORDER BY created_at ASC, CASE role WHEN 'user' THEN 0 ELSE 1 END, rowid ASC
";

/// 会话行不存在时补建（`INSERT OR IGNORE`：已有会话的真实 started_at 不被覆盖）。
fn ensure_session_row(
    conn: &Connection,
    session: &SessionId,
    created_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO sessions (id, started_at, has_text) VALUES (?1, ?2, 0)",
        params![session.as_str(), created_at],
    )
    .map(|_| ())
}

/// 插入 / 覆盖一条消息（`(task_id, role)` 唯一 → 重试幂等）。
fn insert_message_row(conn: &Connection, message: &HistoryMessage) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT OR REPLACE INTO messages
         (id, session_id, task_id, action, mode, role, content, engine, status, error_code, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            uuid::Uuid::new_v4().to_string(),
            message.session.as_str(),
            message.task.as_str(),
            message.action.as_str(),
            message.mode.as_str(),
            message.role.as_str(),
            message.content,
            message.engine,
            message.status,
            message.error_code,
            message.created_at
        ],
    )
}

/// 摘要行映射：title 去多余空白并做 ≤60 字符截断。
fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistorySession> {
    let session: String = row.get(0)?;
    let started_at: i64 = row.get(1)?;
    let title: Option<String> = row.get(2)?;
    let action: Option<String> = row.get(3)?;
    let turns: i64 = row.get(4)?;
    let engine: Option<String> = row.get(5)?;
    Ok(HistorySession {
        session: SessionId::from(session),
        started_at,
        title: normalize_title(title.as_deref().unwrap_or_default()),
        action: action.as_deref().and_then(ActionId::parse),
        turns: u32::try_from(turns.max(0)).unwrap_or(u32::MAX),
        engine,
    })
}

/// 标题规范化：空白折叠为单空格，CJK 安全截断到 [`HISTORY_TITLE_MAX_CHARS`]。
fn normalize_title(raw: &str) -> String {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(normalized.trim(), HISTORY_TITLE_MAX_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::types::{ActionId, Mode, RectPhys, TaskId};

    fn temp_paths() -> AppPaths {
        let root = std::env::temp_dir().join(format!("snip-ask-history-{}", uuid::Uuid::new_v4()));
        AppPaths::from_roots(
            true,
            root,
            Path::new("C:/tmp/config"),
            Path::new("C:/tmp/local"),
        )
        .unwrap()
    }

    fn repo() -> HistoryRepo {
        open_repo(true)
    }

    fn open_repo(enabled: bool) -> HistoryRepo {
        let paths = temp_paths();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let repo = HistoryRepo::open(&paths, enabled);
        if enabled {
            assert!(repo.is_enabled(), "历史库应可用");
        }
        repo
    }

    fn task_record(session: &SessionId, id: &str) -> TaskRecord {
        TaskRecord::new(
            TaskId::from(id),
            session.clone(),
            ActionId::Answer,
            Mode::Quick,
        )
    }

    fn record_turn(
        repo: &HistoryRepo,
        session: &str,
        task: &str,
        question: &str,
        answer: &str,
        engine: Option<&str>,
    ) -> usize {
        repo.record_turn(
            &SessionId::from(session),
            &TaskId::from(task),
            ActionId::Answer,
            Mode::Quick,
            question,
            answer,
            engine,
            "done",
            None,
        )
    }

    // ---------------- 既有行为（回归） ----------------

    #[test]
    fn records_sessions_and_tasks() {
        let repo = repo();
        let session = SessionId::from("s-1");
        let rect = RectPhys {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        repo.record_session(
            &session,
            1_700_000_000_000,
            Some(&rect),
            true,
            Some("文本"),
            Some("notepad.exe"),
        );
        let record = task_record(&session, "t-1");
        repo.record_task(
            &record,
            Some("deepseek"),
            "done",
            Some(120),
            Some(800),
            Some(&TokenUsage {
                input_tokens: 120,
                output_tokens: 60,
                cached_input_tokens: 0,
                cost_estimate: 0.01,
                ..TokenUsage::default()
            }),
            None,
        );
        assert_eq!(repo.task_count(), 1);
        assert_eq!(
            repo.task_ids_for_session(&session),
            vec![TaskId::from("t-1")]
        );

        let today = repo.stats_today();
        assert_eq!(today.tasks, 1);
        assert_eq!(today.tokens_in, 120);
        assert_eq!(today.tokens_out, 60);
        assert!((today.cost_est - 0.01).abs() < 1e-9);
        assert_eq!(repo.stats_month().tasks, 1);
    }

    #[test]
    fn disabled_repo_is_noop() {
        let paths = temp_paths();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let repo = HistoryRepo::open(&paths, false);
        assert!(!repo.is_enabled());
        let session = SessionId::from("s-1");
        repo.record_session(&session, 0, None, false, None, None);
        assert_eq!(repo.task_count(), 0);

        // 历史消息 API 也必须全部 no-op（spec §3.4）。
        assert_eq!(
            record_turn(&repo, "s-1", "t-1", "问题", "回答", Some("deepseek")),
            0
        );
        assert_eq!(repo.message_count(), 0);
        assert!(repo.list_sessions(50, 0).is_empty());
        assert!(repo.session_thread(&session).is_none());
        assert_eq!(repo.clear_history(None), 0);
        assert_eq!(repo.stats_today(), UsageSummary::default());
        // 关闭历史时不应创建库文件。
        assert!(!paths.history_db.exists());
    }

    #[test]
    fn cleanup_and_clear() {
        let repo = repo();
        let session = SessionId::from("s-1");
        repo.record_session(
            &session,
            chrono::Utc::now().timestamp_millis(),
            None,
            false,
            None,
            None,
        );
        repo.record_task(
            &task_record(&session, "t-1"),
            None,
            "done",
            None,
            None,
            None,
            None,
        );
        record_turn(&repo, "s-1", "t-1", "问题", "回答", None);
        repo.cleanup(1000, 90);
        assert_eq!(repo.task_count(), 1);
        assert_eq!(repo.message_count(), 2);
        repo.clear();
        assert_eq!(repo.task_count(), 0);
        assert_eq!(repo.message_count(), 0);
    }

    // ---------------- 迁移 ----------------

    #[test]
    fn migrations_apply_incrementally_and_are_idempotent() {
        let paths = temp_paths();
        std::fs::create_dir_all(&paths.data_dir).unwrap();

        {
            let repo = HistoryRepo::open(&paths, true);
            assert!(repo.is_enabled());
            // 首次打开应已应用 001 + 002（先查元数据，再释放连接锁）。
            let (versions, messages_table) = {
                let guard = repo.conn.lock();
                let conn = guard.as_ref().unwrap();
                let versions: Vec<i64> = {
                    let mut stmt = conn
                        .prepare("SELECT version FROM schema_migrations ORDER BY version")
                        .unwrap();
                    let rows = stmt.query_map([], |row| row.get::<_, i64>(0)).unwrap();
                    rows.filter_map(|row| row.ok()).collect()
                };
                let messages_table: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'messages'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                (versions, messages_table)
            };
            assert_eq!(versions, vec![1, 2]);
            assert_eq!(messages_table, 1);
            record_turn(&repo, "s-1", "t-1", "问题", "回答", Some("deepseek"));
        }

        // 模拟重启：再次打开同一个库，迁移不得重复执行 / 报错，历史仍可读。
        let repo = HistoryRepo::open(&paths, true);
        assert!(repo.is_enabled());
        let sessions = repo.list_sessions(50, 0);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session, SessionId::from("s-1"));
        assert_eq!(repo.message_count(), 2);
    }

    // ---------------- 聚合 / title / 线程 ----------------

    #[test]
    fn list_sessions_aggregates_turns_and_truncates_title() {
        let repo = repo();
        let long_question = format!("请解释这段代码{}", "很长的选区文本".repeat(20));
        record_turn(
            &repo,
            "s-1",
            "t-1",
            &long_question,
            "第一轮回答",
            Some("deepseek"),
        );
        record_turn(&repo, "s-1", "t-2", "继续追问", "第二轮回答", Some("local"));
        record_turn(&repo, "s-2", "t-3", "另一个会话的问题", "回答", None);

        let sessions = repo.list_sessions(50, 0);
        assert_eq!(sessions.len(), 2);
        let first = sessions
            .iter()
            .find(|session| session.session == SessionId::from("s-1"))
            .expect("s-1 应出现在历史列表");
        assert_eq!(first.turns, 2, "turns = 不同 task 数");
        assert_eq!(first.action, Some(ActionId::Answer));
        assert_eq!(
            first.engine.as_deref(),
            Some("local"),
            "engine 取最近一条 assistant 消息"
        );
        assert!(first.started_at > 0);
        assert!(
            first.title.chars().count() <= HISTORY_TITLE_MAX_CHARS,
            "title 必须 ≤60 字符：{}",
            first.title
        );
        assert!(first.title.ends_with('…'), "超长 title 应有省略号");
        assert!(first.title.contains("请解释这段代码"));

        // 分页：limit / offset 生效。
        let page1 = repo.list_sessions(1, 0);
        assert_eq!(page1.len(), 1);
        let page2 = repo.list_sessions(1, 1);
        assert_eq!(page2.len(), 1);
        assert_ne!(page1[0].session, page2[0].session);
        assert_eq!(repo.list_sessions(0, 0).len(), 0);
    }

    #[test]
    fn list_sessions_orders_by_started_at_desc() {
        let repo = repo();
        let old = SessionId::from("s-old");
        let new = SessionId::from("s-new");
        repo.record_session(&old, 1_600_000_000_000, None, false, None, None);
        repo.record_session(&new, 1_700_000_000_000, None, false, None, None);
        record_turn(&repo, "s-old", "t-old", "旧问题", "旧回答", None);
        record_turn(&repo, "s-new", "t-new", "新问题", "新回答", None);

        let sessions = repo.list_sessions(50, 0);
        assert_eq!(sessions[0].session, new);
        assert_eq!(sessions[1].session, old);
        assert_eq!(sessions[0].started_at, 1_700_000_000_000);
    }

    #[test]
    fn session_thread_returns_ordered_turns() {
        let repo = repo();
        let session = SessionId::from("s-thread");
        record_turn(
            &repo,
            "s-thread",
            "t-1",
            "首轮问题",
            "首轮回答",
            Some("deepseek"),
        );
        std::thread::sleep(std::time::Duration::from_millis(3));
        record_turn(
            &repo,
            "s-thread",
            "t-2",
            "追问问题",
            "追问回答",
            Some("local"),
        );

        let thread = repo.session_thread(&session).expect("会话应存在");
        assert_eq!(thread.session.session, session);
        assert_eq!(thread.session.turns, 2);
        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].task, TaskId::from("t-1"));
        assert_eq!(thread.turns[0].question, "首轮问题");
        assert_eq!(thread.turns[0].answer, "首轮回答");
        assert_eq!(thread.turns[0].engine.as_deref(), Some("deepseek"));
        assert_eq!(thread.turns[0].status, "done");
        assert_eq!(thread.turns[1].task, TaskId::from("t-2"));
        assert_eq!(thread.turns[1].question, "追问问题");
        assert!(thread.turns[0].created_at <= thread.turns[1].created_at);

        assert!(repo
            .session_thread(&SessionId::from("s-not-exist"))
            .is_none());
    }

    #[test]
    fn record_message_creates_missing_session_row() {
        let repo = repo();
        let session = SessionId::from("s-auto");
        // 不先 record_session，直接写消息：sessions 行应自动补建。
        let message = HistoryMessage {
            session: session.clone(),
            task: TaskId::from("t-auto"),
            action: ActionId::Search,
            mode: Mode::Deep,
            role: MessageRole::User,
            content: "自动补建会话".to_string(),
            engine: None,
            status: "done".to_string(),
            error_code: None,
            created_at: 1_700_000_123_456,
        };
        repo.record_message(&message);

        // 先确认 sessions 行已补建（拿到值后立刻释放连接锁，避免与后续查询互锁）。
        let started_at: i64 = {
            let guard = repo.conn.lock();
            let conn = guard.as_ref().unwrap();
            conn.query_row(
                "SELECT started_at FROM sessions WHERE id = ?1",
                params![session.as_str()],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(started_at, 1_700_000_123_456);

        let list = repo.list_sessions(50, 0);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "自动补建会话");
        assert_eq!(list[0].started_at, 1_700_000_123_456);
        assert_eq!(list[0].turns, 1);
    }

    #[test]
    fn record_turn_is_idempotent_per_task() {
        let repo = repo();
        record_turn(&repo, "s-1", "t-1", "问题", "回答", None);
        // 同一 task 重试：应覆盖而不是产生第 3 条消息。
        record_turn(&repo, "s-1", "t-1", "问题", "回答（重试）", None);
        assert_eq!(repo.message_count(), 2);
        let thread = repo.session_thread(&SessionId::from("s-1")).unwrap();
        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].answer, "回答（重试）");
    }

    #[test]
    fn clear_history_scoped_and_all() {
        let repo = repo();
        record_turn(&repo, "s-1", "t-1", "问题一", "回答一", None);
        record_turn(&repo, "s-2", "t-2", "问题二", "回答二", None);
        assert_eq!(repo.message_count(), 4);

        let deleted = repo.clear_history(Some(&SessionId::from("s-1")));
        assert_eq!(deleted, 2);
        assert_eq!(repo.message_count(), 2);
        assert!(repo.session_thread(&SessionId::from("s-1")).is_none());
        assert!(repo.session_thread(&SessionId::from("s-2")).is_some());

        let deleted = repo.clear_history(None);
        assert_eq!(deleted, 2);
        assert_eq!(repo.message_count(), 0);
        assert!(repo.list_sessions(50, 0).is_empty());
        assert_eq!(repo.task_count(), 0);
    }

    // ---------------- retention / maxEntries ----------------

    #[test]
    fn retention_cleanup_removes_old_messages() {
        let repo = repo();
        let old_time = chrono::Utc::now().timestamp_millis() - 100 * 86_400_000;
        let session = SessionId::from("s-old");
        repo.record_session(&session, old_time, None, false, None, None);
        let old_message = HistoryMessage {
            session: session.clone(),
            task: TaskId::from("t-old"),
            action: ActionId::Answer,
            mode: Mode::Quick,
            role: MessageRole::User,
            content: "很久以前的问题".to_string(),
            engine: None,
            status: "done".to_string(),
            error_code: None,
            created_at: old_time,
        };
        repo.record_message(&old_message);
        record_turn(&repo, "s-new", "t-new", "新问题", "新回答", None);
        assert_eq!(repo.message_count(), 3);

        repo.cleanup(1000, 90);
        assert_eq!(repo.message_count(), 2, "100 天前的消息应被 retention 清掉");
        let sessions = repo.list_sessions(50, 0);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session, SessionId::from("s-new"));
    }

    #[test]
    fn max_entries_cleanup_keeps_newest_turns() {
        let repo = repo();
        let session = SessionId::from("s-max");
        for index in 1..=3 {
            let task_id = format!("t-{index}");
            repo.record_task(
                &task_record(&session, &task_id),
                None,
                "done",
                None,
                None,
                None,
                None,
            );
            record_turn(
                &repo,
                "s-max",
                &task_id,
                &format!("问题 {index}"),
                &format!("回答 {index}"),
                None,
            );
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        assert_eq!(repo.task_count(), 3);
        assert_eq!(repo.message_count(), 6);

        repo.cleanup(2, 0);
        assert_eq!(repo.task_count(), 2, "tasks 只保留最新 2 条");
        assert_eq!(repo.message_count(), 4, "消息跟随最新 2 轮");
        let thread = repo.session_thread(&session).unwrap();
        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].question, "问题 2");
        assert_eq!(thread.turns[1].question, "问题 3");
    }
}
