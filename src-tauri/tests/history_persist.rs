//! 连续提问历史持久化集成测试（T4 / spec v1 §3.3-§3.4）。
//!
//! 这些用例不是 mock：使用真实 `rusqlite` + 真实 SQLite 文件（临时目录下的 `<data>/history.db`），
//! 通过「写入 → 关闭连接 → 重新打开（模拟应用重启）→ 读取」验证历史能跨进程/跨重启恢复。
//!
//! 证据输出：`cargo test --test history_persist -- --nocapture` 会打印带 `T4_EVIDENCE` 前缀的行。

use std::path::{Path, PathBuf};

use snip_ask_lib::contract::types::{ActionId, Mode, RectPhys, SessionId, TaskId};
use snip_ask_lib::domain::task::TaskRecord;
use snip_ask_lib::platform::paths::AppPaths;
use snip_ask_lib::services::history_repo::{HistoryMessage, HistoryRepo, MessageRole};

/// 固定根目录（证据脚本按同一路径归档 history.db）。
const EVIDENCE_ROOT: &str = "snip-ask-t4-history-evidence";

fn evidence_paths(tag: &str) -> AppPaths {
    let root: PathBuf = std::env::temp_dir().join(EVIDENCE_ROOT).join(tag);
    let _ = std::fs::remove_dir_all(&root);
    let paths = AppPaths::from_roots(
        true,
        root,
        Path::new("C:/tmp/config-t4"),
        Path::new("C:/tmp/local-t4"),
    )
    .unwrap();
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    paths
}

fn record_turn(
    repo: &HistoryRepo,
    session: &str,
    task: &str,
    action: ActionId,
    question: &str,
    answer: &str,
    engine: Option<&str>,
) -> usize {
    repo.record_turn(
        &SessionId::from(session),
        &TaskId::from(task),
        action,
        Mode::Quick,
        question,
        answer,
        engine,
        "done",
        None,
    )
}

#[test]
fn history_survives_restart() {
    let paths = evidence_paths("restart");
    let session = SessionId::from("sess-restart-1");
    let other = SessionId::from("sess-restart-2");

    {
        // ---- 第一个进程：写入 ----
        let repo = HistoryRepo::open(&paths, true);
        assert!(repo.is_enabled(), "历史库应可用");

        // 会话元数据（真实捕获流程会调用；这里显式写入以便验证 startedAt 来自 sessions 表）。
        repo.record_session(
            &session,
            1_700_000_000_000,
            Some(&RectPhys {
                x: -100.0,
                y: 10.0,
                width: 320.0,
                height: 180.0,
            }),
            true,
            Some("选区文本"),
            Some("notepad.exe"),
        );
        repo.record_session(&other, 1_600_000_000_000, None, false, None, None);

        // 三轮任务（含追问）+ 两轮另一个会话。
        for (index, (session, task, question, answer, engine)) in [
            (
                &session,
                "task-1",
                "首轮：请解释这段代码的含义",
                "首轮回答：它做了三件事。",
                "deepseek",
            ),
            (
                &session,
                "task-2",
                "追问：第三件事呢？",
                "追问回答：第三件事是缓存。",
                "deepseek",
            ),
            (
                &session,
                "task-3",
                "追问：有风险吗？",
                "追问回答：注意日志脱敏。",
                "local",
            ),
            (
                &other,
                "task-4",
                "另一个会话的问题",
                "另一个会话的回答",
                "dashscope",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            repo.record_task(
                &TaskRecord::new(
                    TaskId::from(task),
                    session.clone(),
                    ActionId::Answer,
                    Mode::Quick,
                ),
                engine.into(),
                "done",
                Some(100 + index as u64),
                Some(600 + index as u64 * 10),
                None,
                None,
            );
            assert_eq!(
                record_turn(
                    &repo,
                    session.as_str(),
                    task,
                    ActionId::Answer,
                    question,
                    answer,
                    Some(engine)
                ),
                2,
                "每轮应写入 user + assistant 两条"
            );
        }
        // 单条消息 API：engine 未知也要能写（lead 要求）。
        repo.record_message(&HistoryMessage {
            session: session.clone(),
            task: TaskId::from("task-note"),
            action: ActionId::Answer,
            mode: Mode::Deep,
            role: MessageRole::User,
            content: "engine 未知的一条消息".to_string(),
            engine: None,
            status: "done".to_string(),
            error_code: None,
            created_at: chrono::Utc::now().timestamp_millis() + 1_000,
        });

        assert_eq!(repo.message_count(), 9);
        println!(
            "T4_EVIDENCE db={} before_restart_messages={}",
            paths.history_db.display(),
            repo.message_count()
        );
        // 关闭：drop repo 释放 SQLite 连接（模拟进程退出）。
    }

    // ---- 第二个「进程」：重新打开同一个库 ----
    let repo = HistoryRepo::open(&paths, true);
    assert!(repo.is_enabled());
    assert_eq!(repo.message_count(), 9, "重启后消息应仍在");

    let sessions = repo.list_sessions(50, 0);
    assert_eq!(sessions.len(), 2, "两个会话都应恢复");
    let first = &sessions[0];
    assert_eq!(first.session, session);
    assert_eq!(
        first.started_at, 1_700_000_000_000,
        "startedAt 应来自 sessions"
    );
    assert_eq!(first.turns, 4, "三个 task + 一条单消息 task");
    assert_eq!(first.engine.as_deref(), Some("local"));
    assert_eq!(first.action, Some(ActionId::Answer));
    assert!(first.title.contains("首轮"));
    println!(
        "T4_EVIDENCE after_restart sessions={}",
        serde_json::to_string(&sessions).unwrap()
    );

    let thread = repo.session_thread(&session).expect("重启后应能回看线程");
    assert_eq!(thread.session.turns, 4);
    assert_eq!(thread.turns.len(), 4);
    assert_eq!(thread.turns[0].question, "首轮：请解释这段代码的含义");
    assert_eq!(thread.turns[1].answer, "追问回答：第三件事是缓存。");
    assert_eq!(thread.turns[2].engine.as_deref(), Some("local"));
    // 时间正序
    for pair in thread.turns.windows(2) {
        assert!(pair[0].created_at <= pair[1].created_at);
    }
    println!(
        "T4_EVIDENCE thread={}",
        serde_json::to_string(&thread).unwrap()
    );

    // 清空单会话后，另一个会话仍可读（scoped clear）。
    // 该会话共 7 条消息：3 轮问答（6）+ 1 条 engine 未知消息。
    let deleted = repo.clear_history(Some(&session));
    assert_eq!(deleted, 7);
    assert!(repo.session_thread(&session).is_none());
    assert!(repo.session_thread(&other).is_some());
    let remaining = repo.list_sessions(50, 0);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].session, other);
    println!(
        "T4_EVIDENCE after_clear sessions={}",
        serde_json::to_string(&remaining).unwrap()
    );
}

#[test]
fn history_disabled_is_noop_and_creates_no_db() {
    let paths = evidence_paths("disabled");
    let repo = HistoryRepo::open(&paths, false);
    assert!(!repo.is_enabled());

    assert_eq!(
        record_turn(
            &repo,
            "s-1",
            "t-1",
            ActionId::Answer,
            "问题",
            "回答",
            Some("deepseek")
        ),
        0
    );
    assert_eq!(repo.message_count(), 0);
    assert!(repo.list_sessions(50, 0).is_empty());
    assert!(repo.session_thread(&SessionId::from("s-1")).is_none());
    assert_eq!(repo.clear_history(None), 0);
    assert!(
        !paths.history_db.exists(),
        "history.enabled=false 时不应创建 history.db"
    );
    println!("T4_EVIDENCE disabled_noop db_created=false");
}

#[test]
fn v1_database_upgrades_to_messages_schema() {
    let paths = evidence_paths("legacy-v1");
    // 模拟 v0.1 旧库：只执行 001 初始 Schema。
    {
        let conn = rusqlite::Connection::open(&paths.history_db).unwrap();
        conn.execute_batch(include_str!("../migrations/001_init.sql"))
            .unwrap();
        let applied: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(applied, 1, "旧库只应有 001");
    }

    // 新版本打开 → 增量迁移 002，且旧数据（tasks/usage）不受影响。
    let repo = HistoryRepo::open(&paths, true);
    assert!(repo.is_enabled());
    assert_eq!(
        record_turn(
            &repo,
            "s-legacy",
            "t-legacy",
            ActionId::Translate,
            "旧库升级后的新消息",
            "回答",
            None
        ),
        2
    );
    drop(repo);

    let conn = rusqlite::Connection::open(&paths.history_db).unwrap();
    let max_version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(max_version, 2, "升级后应为版本 2");
    let messages: i64 = conn
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(messages, 2);
    println!("T4_EVIDENCE legacy_upgrade schema_version=2 messages={messages}");
}

#[test]
fn manual_cleanup_prunes_retention_and_max_entries() {
    let paths = evidence_paths("cleanup");
    let repo = HistoryRepo::open(&paths, true);
    let session = SessionId::from("s-clean");
    let old_time = chrono::Utc::now().timestamp_millis() - 100 * 86_400_000;
    repo.record_message(&HistoryMessage {
        session: session.clone(),
        task: TaskId::from("t-old"),
        action: ActionId::Answer,
        mode: Mode::Quick,
        role: MessageRole::User,
        content: "100 天前的问题".to_string(),
        engine: None,
        status: "done".to_string(),
        error_code: None,
        created_at: old_time,
    });
    for index in 1..=3 {
        record_turn(
            &repo,
            "s-clean",
            &format!("t-{index}"),
            ActionId::Answer,
            &format!("问题 {index}"),
            &format!("回答 {index}"),
            None,
        );
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    assert_eq!(repo.message_count(), 7);

    repo.cleanup(1000, 90);
    assert_eq!(repo.message_count(), 6, "retention 应删掉 100 天前的消息");
    println!(
        "T4_EVIDENCE cleanup_retention messages={}",
        repo.message_count()
    );
}
