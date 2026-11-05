//! T3「AI 回答后连续提问」回归测试。
//!
//! 覆盖三块验收（研究 §3.10 / 任务描述）：
//! 1. **F1（P0）回归**：任务完成后 `TaskStore` 已移除记录，旧 `task_followup` 实现第一步
//!    `tasks.session_of(&task)` 必然 `SA-1099`；新实现 `locate_followup_target` 必须成功。
//! 2. **多轮上下文**：`build_followup_context` 必须包含上一轮回答正文（以及选区原文）。
//! 3. **状态机 / parent**：任务失败回到 `showingResult` 后可继续追问；`parent` 语义可序列化。

use std::collections::HashMap;

use snip_ask_lib::commands::action::locate_followup_target;
use snip_ask_lib::contract::errors::AppError;
use snip_ask_lib::contract::events::TaskStartedPayload;
use snip_ask_lib::contract::types::{
    ActionId, Mode, OcrBox, RectPhys, SelectionResult, SessionId, TaskId,
};
use snip_ask_lib::domain::session::{SessionPhase, SessionStore};
use snip_ask_lib::domain::task::{TaskRecord, TaskStore};
use snip_ask_lib::domain::turn::{
    build_followup_context, Turn, TurnStore, FOLLOWUP_CONTEXT_MAX_CHARS,
};
use snip_ask_lib::services::prompt_engine::PromptEngine;

fn rect() -> RectPhys {
    RectPhys {
        x: 10.0,
        y: 20.0,
        width: 300.0,
        height: 80.0,
    }
}

fn selection(session: &SessionId) -> SelectionResult {
    SelectionResult {
        session: session.clone(),
        rect_phys: rect(),
        has_text: true,
        preview: "选区原文：请解释这段代码".to_string(),
        boxes: vec![OcrBox {
            text: "选区原文：请解释这段代码".to_string(),
            confidence: 0.95,
            bbox: rect(),
        }],
    }
}

/// 建一个已提交选区（`menu` 阶段）的会话，返回会话 ID。
fn session_at_menu(store: &mut SessionStore) -> SessionId {
    let start = store.begin_capture();
    let session = start.session;
    store.set_selecting(&session).unwrap();
    store
        .commit_selection(&session, selection(&session))
        .unwrap();
    session
}

fn task(id: &str, session: &SessionId) -> TaskRecord {
    TaskRecord::new(
        TaskId::from(id),
        session.clone(),
        ActionId::Answer,
        Mode::Quick,
    )
}

/// F1 回归（P0）：用户真实时序——回答已完成后再追问。
///
/// 原 bug 路径：`task_runner` 完成后 `tasks.remove(&task)` → `session_of` 返回 `None`
/// → `task_followup` 第一步即 `SA-1099`（研究 §1.2 路径 A）。
#[test]
fn f1_completed_task_followup_succeeds_after_task_store_removal() {
    let mut sessions = SessionStore::new();
    let session = session_at_menu(&mut sessions);
    let first = TaskId::from("t-first");

    // 首轮任务：开始 → 完成（会话回到 showingResult）。
    sessions.begin_task(&session, first.clone()).unwrap();
    sessions.task_finished(&session).unwrap();

    // 模拟 task_runner 的完成路径：先登记，成功后移除（这正是 F1 的根因）。
    let mut tasks = TaskStore::new();
    tasks.insert(task("t-first", &session));
    assert!(tasks.remove(&first).is_some());
    assert!(
        tasks.session_of(&first).is_none(),
        "已完成任务必须不在 TaskStore 中（F1 根因）"
    );

    // —— 旧实现（逐字复刻）：第一步 session_of → 必然 SA-1099 ——
    let legacy: Result<SessionId, AppError> = tasks
        .session_of(&first)
        .ok_or_else(|| AppError::invalid_state(format!("任务 {first} 不存在")));
    let legacy_error = legacy.expect_err("旧实现必须复现原 bug（SA-1099）");
    assert_eq!(legacy_error.code, "SA-1099");
    assert!(
        legacy_error
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("不存在"),
        "原 bug 是「任务不存在」：{:?}",
        legacy_error.detail
    );

    // —— 新实现：当前会话 + task_ids.last() 定位 → 成功，parent = 上一轮任务 ——
    let snapshot = sessions.current().cloned();
    let target = locate_followup_target(snapshot.as_ref(), &first)
        .expect("新实现必须允许已完成任务后的追问");
    assert_eq!(target.session, session);
    assert_eq!(target.parent, first);
    assert_eq!(target.selection.preview, "选区原文：请解释这段代码");
}

/// 路径 B：流式过程中提交（真实后端必须拒绝且给出可读提示，而不是原 bug 的「任务不存在」）。
#[test]
fn followup_while_streaming_is_rejected_with_wait_message() {
    let mut sessions = SessionStore::new();
    let session = session_at_menu(&mut sessions);
    let running = TaskId::from("t-running");
    sessions.begin_task(&session, running.clone()).unwrap();
    assert_eq!(sessions.current_phase(), SessionPhase::Executing);

    let snapshot = sessions.current().cloned();
    let error = locate_followup_target(snapshot.as_ref(), &running).expect_err("流式中不得追问");
    assert_eq!(error.code, "SA-1099");
    assert!(
        error.message.contains("等待"),
        "必须是可读提示：{}",
        error.message
    );
}

/// 无活动会话 / 没有可追问任务：仍返回 SA-1099，但信息可操作。
#[test]
fn followup_without_active_session_is_rejected() {
    let error = locate_followup_target(None, &TaskId::from("t-1")).unwrap_err();
    assert_eq!(error.code, "SA-1099");
    assert!(error.message.contains("没有活动会话"));

    let mut sessions = SessionStore::new();
    let _session = session_at_menu(&mut sessions);
    let snapshot = sessions.current().cloned();
    let error = locate_followup_target(snapshot.as_ref(), &TaskId::from("t-1")).unwrap_err();
    assert_eq!(error.code, "SA-1099");
    assert!(
        error.message.contains("没有可追问的任务") || error.message.contains("menu"),
        "必须是可读提示：{}",
        error.message
    );
    assert_eq!(sessions.current_phase(), SessionPhase::Menu);
}

/// 传入的 `task` 不在本会话中（旧窗口残留 / 已被清理）→ 回退最近一轮，不得必失败。
#[test]
fn unknown_task_id_falls_back_to_latest_task() {
    let mut sessions = SessionStore::new();
    let session = session_at_menu(&mut sessions);
    sessions.begin_task(&session, TaskId::from("t-1")).unwrap();
    sessions.task_finished(&session).unwrap();
    sessions.begin_task(&session, TaskId::from("t-2")).unwrap();
    sessions.task_finished(&session).unwrap();

    let snapshot = sessions.current().cloned();
    let stale = TaskId::from("t-from-previous-window");
    let target = locate_followup_target(snapshot.as_ref(), &stale).expect("未知任务应回退而非失败");
    assert_eq!(target.parent, TaskId::from("t-2"), "parent 必须是最近一轮");
}

/// 属于本会话但不是最近一轮 → 明确拒绝「只能对最近一轮追问」。
#[test]
fn older_task_in_same_session_is_rejected() {
    let mut sessions = SessionStore::new();
    let session = session_at_menu(&mut sessions);
    sessions.begin_task(&session, TaskId::from("t-1")).unwrap();
    sessions.task_finished(&session).unwrap();
    sessions.begin_task(&session, TaskId::from("t-2")).unwrap();
    sessions.task_finished(&session).unwrap();

    let snapshot = sessions.current().cloned();
    let error = locate_followup_target(snapshot.as_ref(), &TaskId::from("t-1")).unwrap_err();
    assert_eq!(error.code, "SA-1099");
    assert!(error.message.contains("最近一轮"));
}

/// 多轮上下文：注入的 `{context}` 必须包含上一轮回答正文（F2 回归）。
#[test]
fn followup_context_contains_previous_turn_answer() {
    let session = SessionId::from("s-context");
    let mut turns = TurnStore::new();
    turns.record(
        &session,
        Turn::now(
            TaskId::from("t-1"),
            "选区原文：请解释这段代码".to_string(),
            "上一轮回答正文：这段代码先解析参数，再做网络请求。".to_string(),
            "deepseek".to_string(),
            None,
        ),
    );

    let context = build_followup_context(
        &turns,
        &session,
        "选区原文：请解释这段代码",
        FOLLOWUP_CONTEXT_MAX_CHARS,
    );
    assert!(context.contains("【框选内容】"), "{context}");
    assert!(context.contains("选区原文：请解释这段代码"), "{context}");
    assert!(
        context.contains("上一轮回答正文：这段代码先解析参数，再做网络请求。"),
        "上一轮回答必须进入 {{context}}：{context}"
    );
    assert!(context.contains("问："), "必须区分上一轮问题：{context}");

    // 追问（第二轮）后，上下文应同时包含首轮与追问轮。
    turns.record(
        &session,
        Turn::now(
            TaskId::from("t-2"),
            "再解释第二点".to_string(),
            "第二轮回答：第二点是异常处理。".to_string(),
            "deepseek".to_string(),
            Some(TaskId::from("t-1")),
        ),
    );
    let context = build_followup_context(&turns, &session, "选区原文", FOLLOWUP_CONTEXT_MAX_CHARS);
    assert!(
        context.contains("第二轮回答：第二点是异常处理。"),
        "{context}"
    );
    assert!(context.contains("上一轮回答正文"), "{context}");

    // 无轮次 / 预算为 0 时不注入上下文（首次追问前的空线程）。
    let empty = build_followup_context(&TurnStore::new(), &session, "", FOLLOWUP_CONTEXT_MAX_CHARS);
    assert!(empty.is_empty());
}

/// 状态机（研究 §3.9-①）：任务失败 → 回到 showingResult（不是终局 error），之后可追问/重答。
#[test]
fn task_failure_returns_to_showing_result_and_allows_followup() {
    let mut sessions = SessionStore::new();
    let session = session_at_menu(&mut sessions);
    sessions.begin_task(&session, TaskId::from("t-1")).unwrap();
    sessions.task_failed(&session).unwrap();
    assert_eq!(
        sessions.current_phase(),
        SessionPhase::ShowingResult,
        "失败必须回结果态而不是终局 error"
    );

    // 失败后仍可追问：定位成功 + 状态机允许 begin_task。
    let snapshot = sessions.current().cloned();
    let target = locate_followup_target(snapshot.as_ref(), &TaskId::from("t-1"))
        .expect("失败后必须允许继续追问");
    assert_eq!(target.parent, TaskId::from("t-1"));
    sessions
        .begin_task(&session, TaskId::from("t-2"))
        .expect("失败后必须允许再次执行任务");
    assert_eq!(sessions.current_phase(), SessionPhase::Executing);
    sessions.task_finished(&session).unwrap();
    assert_eq!(sessions.current_phase(), SessionPhase::ShowingResult);

    // error 阶段保留可达（不可恢复错误），且能关闭回 idle。
    sessions.enter_error(&session).unwrap();
    assert_eq!(sessions.current_phase(), SessionPhase::Error);
    sessions.cancel(&session).unwrap();
    assert_eq!(sessions.current_phase(), SessionPhase::Idle);
}

/// 契约 v4：`task:started.parent` 序列化语义（首轮/重答 null，追问 = 上一轮任务 id）。
#[test]
fn task_started_payload_carries_parent() {
    let payload = TaskStartedPayload {
        task: TaskId::from("t-2"),
        session: SessionId::from("s-1"),
        action: ActionId::Answer,
        mode: Mode::Quick,
        engine: "skeleton".to_string(),
        parent: Some(TaskId::from("t-1")),
    };
    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value["parent"], "t-1");
    assert_eq!(value["task"], "t-2");

    let first_round = TaskStartedPayload {
        task: TaskId::from("t-1"),
        session: SessionId::from("s-1"),
        action: ActionId::Answer,
        mode: Mode::Quick,
        engine: "skeleton".to_string(),
        parent: None,
    };
    assert!(serde_json::to_value(&first_round).unwrap()["parent"].is_null());
}

/// 端到端（提示词层）：把组装好的 `{context}` 交给**真实** `followup.md` 模板渲染，
/// 断言上一轮回答正文确实进入了发给模型的提示词（F2 回归的最终交付物）。
///
/// 这里用无覆盖目录的 `PromptEngine`（内置模板），与运行时 `state.prompt_engine()` 同一实现。
#[test]
fn followup_prompt_template_receives_previous_answer() {
    let engine = PromptEngine::with_override_dir(
        std::env::temp_dir().join("snip-ask-t3-followup-no-override"),
    );
    let session = SessionId::from("s-prompt");
    let mut turns = TurnStore::new();
    turns.record(
        &session,
        Turn::now(
            TaskId::from("t-1"),
            "选区原文：请解释这段代码".to_string(),
            "上一轮回答：这段代码先解析参数，再发起网络请求。".to_string(),
            "deepseek".to_string(),
            None,
        ),
    );

    let context = build_followup_context(
        &turns,
        &session,
        "选区原文：请解释这段代码",
        FOLLOWUP_CONTEXT_MAX_CHARS,
    );
    let mut vars: HashMap<&str, &str> = HashMap::new();
    vars.insert("question", "再解释第二点");
    vars.insert("context", context.as_str());
    let prompt = engine
        .render("followup", &vars)
        .expect("followup 模板必须可渲染");

    assert!(
        prompt.contains("上一轮回答：这段代码先解析参数，再发起网络请求。"),
        "上一轮回答必须进入最终提示词：{prompt}"
    );
    assert!(
        prompt.contains("再解释第二点"),
        "本次追问必须进入 {{question}}"
    );
    assert!(prompt.contains("选区原文：请解释这段代码"));
    assert!(
        !prompt.contains("# 用途"),
        "维护注释头必须被 prompt_engine 剔除"
    );
    assert!(
        prompt.contains("不要重复上一轮"),
        "模板的多轮约束必须生效（证明上下文语义回归）"
    );
}
