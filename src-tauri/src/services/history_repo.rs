//! 历史与用量仓储（M17 / 契约 §10 / 说明书 §6.5）。
//!
//! - SQLite 路径 `<data>/history.db`，初始 Schema 通过 `include_str!("../../migrations/001_init.sql")` 执行；
//! - `history.enabled = false` 时所有写入 / 查询为 no-op；
//! - 打开失败只记 `warn`，不 panic、不阻断启动（降级为无历史）。
//! - 日志禁止记录 OCR 原文（本模块只写库，不写日志内容）。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{params, Connection};

use crate::contract::types::{RectPhys, SessionId, TaskId, TokenUsage};
use crate::domain::task::TaskRecord;
use crate::platform::paths::AppPaths;

const MIGRATION_SQL: &str = include_str!("../../migrations/001_init.sql");

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

/// 历史仓储。
pub struct HistoryRepo {
    conn: parking_lot::Mutex<Option<Connection>>,
    enabled: AtomicBool,
}

impl HistoryRepo {
    /// 打开历史库；失败时降级为无历史（只记 warn）。
    pub fn open(paths: &AppPaths, enabled: bool) -> Self {
        if !enabled {
            return Self {
                conn: parking_lot::Mutex::new(None),
                enabled: AtomicBool::new(false),
            };
        }
        match Self::try_open(&paths.history_db) {
            Ok(conn) => Self {
                conn: parking_lot::Mutex::new(Some(conn)),
                enabled: AtomicBool::new(true),
            },
            Err(err) => {
                tracing::warn!(error = %err, "历史库打开失败，历史与用量功能降级为禁用");
                Self {
                    conn: parking_lot::Mutex::new(None),
                    enabled: AtomicBool::new(false),
                }
            }
        }
    }

    fn try_open(path: &Path) -> Result<Connection, rusqlite::Error> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(MIGRATION_SQL)?;
        Ok(conn)
    }

    /// 是否启用。
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// 开关（配置热更新时调用）。
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

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
        let rect_text = rect.map(|rect| {
            format!(
                "{},{},{},{}",
                rect.x, rect.y, rect.width, rect.height
            )
        });
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

    fn record_usage_with_conn(&self, conn: &Connection, usage: &TokenUsage) -> rusqlite::Result<()> {
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

    /// 清理：按保留条数与保留天数裁剪（`max_entries = 0` 表示不限）。
    pub fn cleanup(&self, max_entries: u32, retention_days: u32) {
        if !self.is_enabled() {
            return;
        }
        let guard = self.conn.lock();
        let Some(conn) = guard.as_ref() else {
            return;
        };
        if retention_days > 0 {
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
            let _ = conn.execute(
                "DELETE FROM tasks WHERE id NOT IN (
                    SELECT id FROM tasks ORDER BY created_at DESC LIMIT ?1
                 )",
                params![max_entries as i64],
            );
        }
    }

    /// 清空全部历史（设置页「一键清空」）。
    pub fn clear(&self) {
        if !self.is_enabled() {
            return;
        }
        let guard = self.conn.lock();
        if let Some(conn) = guard.as_ref() {
            let _ = conn.execute_batch(
                "DELETE FROM tasks; DELETE FROM sessions; DELETE FROM usage_daily;",
            );
        }
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
        let rows = statement.query_map(params![session.as_str()], |row| {
            row.get::<_, String>(0)
        });
        match rows {
            Ok(rows) => rows.filter_map(|row| row.ok()).map(TaskId::from).collect(),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::types::{ActionId, Mode, RectPhys, TaskId};

    fn repo() -> HistoryRepo {
        let root = std::env::temp_dir().join(format!("snip-ask-history-{}", uuid::Uuid::new_v4()));
        let paths = AppPaths::from_roots(
            true,
            root.clone(),
            Path::new("C:/tmp/config"),
            Path::new("C:/tmp/local"),
        )
        .unwrap();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let repo = HistoryRepo::open(&paths, true);
        assert!(repo.is_enabled(), "历史库应可用");
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
        repo.record_session(&session, 1_700_000_000_000, Some(&rect), true, Some("文本"), Some("notepad.exe"));
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
        assert_eq!(repo.task_ids_for_session(&session), vec![TaskId::from("t-1")]);

        let today = repo.stats_today();
        assert_eq!(today.tasks, 1);
        assert_eq!(today.tokens_in, 120);
        assert_eq!(today.tokens_out, 60);
        assert!((today.cost_est - 0.01).abs() < 1e-9);
        assert_eq!(repo.stats_month().tasks, 1);
    }

    #[test]
    fn disabled_repo_is_noop() {
        let root = std::env::temp_dir().join(format!("snip-ask-history-{}", uuid::Uuid::new_v4()));
        let paths = AppPaths::from_roots(
            true,
            root,
            Path::new("C:/tmp/config"),
            Path::new("C:/tmp/local"),
        )
        .unwrap();
        let repo = HistoryRepo::open(&paths, false);
        assert!(!repo.is_enabled());
        let session = SessionId::from("s-1");
        repo.record_session(&session, 0, None, false, None, None);
        assert_eq!(repo.task_count(), 0);
        assert_eq!(repo.stats_today(), UsageSummary::default());
    }

    #[test]
    fn cleanup_and_clear() {
        let repo = repo();
        let session = SessionId::from("s-1");
        repo.record_session(&session, chrono::Utc::now().timestamp_millis(), None, false, None, None);
        repo.record_task(&task_record(&session, "t-1"), None, "done", None, None, None, None);
        repo.cleanup(1000, 90);
        assert_eq!(repo.task_count(), 1);
        repo.clear();
        assert_eq!(repo.task_count(), 0);
    }
}
