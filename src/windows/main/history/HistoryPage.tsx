/**
 * 主窗口「历史」页（契约 v5）：浏览连续提问的历史线程。
 *
 * 命令：
 * - `history_list`（limit/offset）列出会话摘要（按 startedAt 倒序）；
 * - `history_thread` 读取单个会话的完整问答线程（重启后从本机 SQLite 读取）；
 * - `history_clear` 清空单个会话（传 session）或全部（不传）。
 *
 * 状态：空态 / 加载态 / 错误内联；清空为两段式确认，避免误删。
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { Button, EmptyState, Spinner } from "@/components/ui";
import type { AppError, HistorySession, HistoryThread } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc, toAppError } from "@/shared/ipc";
import {
  filterSessions,
  formatAbsoluteTime,
  formatHistoryTime,
  historyActionLabel,
  historyEngineLabel,
  sortSessions,
  summarizeTitle,
  threadTurnCount,
  turnStatusLabel,
} from "./format";
import "./history.css";

const LIST_LIMIT = 100;

type ConfirmState = { type: "all" } | { type: "one"; session: HistorySession } | null;

function describeError(err: unknown): string {
  const appError: AppError = toAppError(err);
  return `${appError.code} · ${appError.message}`;
}

export default function HistoryPage() {
  const [sessions, setSessions] = useState<HistorySession[]>([]);
  const [listLoading, setListLoading] = useState(true);
  const [listError, setListError] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const [selected, setSelected] = useState<string | null>(null);
  const [thread, setThread] = useState<HistoryThread | null>(null);
  const [threadLoading, setThreadLoading] = useState(false);
  const [threadError, setThreadError] = useState<string | null>(null);

  const [confirm, setConfirm] = useState<ConfirmState>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const loadList = useCallback(async (keepSelection: boolean) => {
    setListLoading(true);
    setListError(null);
    try {
      const list = await ipc.invoke<HistorySession[]>(COMMANDS.historyList, {
        limit: LIST_LIMIT,
        offset: 0,
      });
      const sorted = sortSessions(list ?? []);
      setSessions(sorted);
      if (!keepSelection) {
        setSelected(null);
        setThread(null);
      } else {
        setSelected((current) =>
          current && sorted.some((item) => item.session === current) ? current : null,
        );
      }
    } catch (err) {
      setListError(describeError(err));
    } finally {
      setListLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadList(false);
  }, [loadList]);

  const openThread = useCallback(async (session: HistorySession) => {
    setSelected(session.session);
    setThread(null);
    setThreadError(null);
    setThreadLoading(true);
    try {
      const value = await ipc.invoke<HistoryThread>(COMMANDS.historyThread, {
        session: session.session,
      });
      setThread(value);
    } catch (err) {
      setThreadError(describeError(err));
    } finally {
      setThreadLoading(false);
    }
  }, []);

  const refreshSelection = useCallback(async () => {
    if (!selected) return;
    setThreadError(null);
    setThreadLoading(true);
    try {
      const value = await ipc.invoke<HistoryThread>(COMMANDS.historyThread, { session: selected });
      setThread(value);
    } catch (err) {
      setThreadError(describeError(err));
    } finally {
      setThreadLoading(false);
    }
  }, [selected]);

  const clearSession = async (session: HistorySession) => {
    setBusy(true);
    setNotice(null);
    setListError(null);
    try {
      await ipc.invoke<void>(COMMANDS.historyClear, { session: session.session });
      setNotice(`已清空「${summarizeTitle(session.title)}」的历史记录。`);
      if (selected === session.session) {
        setSelected(null);
        setThread(null);
      }
      await loadList(true);
    } catch (err) {
      setListError(describeError(err));
    } finally {
      setBusy(false);
      setConfirm(null);
    }
  };

  const clearAll = async () => {
    setBusy(true);
    setNotice(null);
    setListError(null);
    try {
      await ipc.invoke<void>(COMMANDS.historyClear, {});
      setNotice("已清空全部历史记录（本机 SQLite）。");
      setSelected(null);
      setThread(null);
      await loadList(false);
    } catch (err) {
      setListError(describeError(err));
    } finally {
      setBusy(false);
      setConfirm(null);
    }
  };

  const visible = useMemo(() => filterSessions(sessions, query), [sessions, query]);
  const selectedSummary = useMemo(
    () => sessions.find((item) => item.session === selected) ?? null,
    [sessions, selected],
  );

  return (
    <div className="history-root">
      <header className="history-header">
        <div>
          <h1 className="history-title">历史</h1>
          <p className="history-subtitle">
            连续提问的历史线程保存在本机 SQLite，不上传；清空后不可恢复。
          </p>
        </div>
        <div className="history-header-actions">
          <Button
            variant="secondary"
            size="sm"
            disabled={listLoading}
            onClick={() => void loadList(true)}
          >
            {listLoading ? "读取中…" : "刷新"}
          </Button>
          {confirm?.type === "all" ? (
            <span className="history-confirm">
              <span className="history-confirm-text">将清空全部历史记录，且不可恢复，确定？</span>
              <Button variant="danger" size="sm" disabled={busy} onClick={() => void clearAll()}>
                {busy ? "清空中…" : "确认清空全部"}
              </Button>
              <Button variant="ghost" size="sm" disabled={busy} onClick={() => setConfirm(null)}>
                取消
              </Button>
            </span>
          ) : (
            <Button
              variant="danger"
              size="sm"
              disabled={busy || sessions.length === 0}
              onClick={() => setConfirm({ type: "all" })}
            >
              清空全部历史
            </Button>
          )}
        </div>
      </header>

      {listError ? (
        <div className="history-error" role="alert">
          <strong>{listError}</strong>
          <Button variant="ghost" size="sm" onClick={() => void loadList(true)}>
            重试
          </Button>
        </div>
      ) : null}
      {notice ? <p className="history-notice">{notice}</p> : null}

      <div className="history-toolbar">
        <input
          className="history-search"
          type="search"
          value={query}
          placeholder="按标题 / 引擎 / 动作筛选…"
          onChange={(event) => setQuery(event.target.value)}
        />
        <span className="history-count">
          {visible.length} / {sessions.length} 个会话
        </span>
      </div>

      <div className="history-layout">
        <section className="history-list" aria-label="历史会话列表">
          {listLoading && sessions.length === 0 ? (
            <div className="history-loading">
              <Spinner />
              <span>正在读取历史会话…</span>
            </div>
          ) : visible.length === 0 ? (
            <EmptyState
              title={sessions.length === 0 ? "还没有历史记录" : "没有匹配的会话"}
              description={
                sessions.length === 0
                  ? "框选并完成一次动作后，会话会自动记录在这里（可在「设置 → 核心」关闭历史）。"
                  : "试试更换筛选关键词。"
              }
            />
          ) : (
            visible.map((session) => {
              const active = session.session === selected;
              return (
                <button
                  key={session.session}
                  type="button"
                  className={`history-item${active ? " is-active" : ""}`}
                  onClick={() => void openThread(session)}
                >
                  <span className="history-item-title">{summarizeTitle(session.title)}</span>
                  <span className="history-item-meta">
                    <span>{formatHistoryTime(session.startedAt)}</span>
                    <span>· {session.turns} 轮</span>
                    <span>· {historyActionLabel(session.action)}</span>
                    <span>· {historyEngineLabel(session.engine)}</span>
                  </span>
                </button>
              );
            })
          )}
        </section>

        <section className="history-detail" aria-label="历史线程详情">
          {!selectedSummary && !threadLoading ? (
            <EmptyState
              title="选择一个会话查看问答"
              description="左侧列出最近的会话，点击可回溯每一轮问题、回答、引擎与时间。"
            />
          ) : null}

          {threadLoading ? (
            <div className="history-loading">
              <Spinner />
              <span>正在读取线程…</span>
            </div>
          ) : null}

          {threadError ? (
            <div className="history-error" role="alert">
              <strong>{threadError}</strong>
              <Button variant="ghost" size="sm" onClick={() => void refreshSelection()}>
                重试
              </Button>
            </div>
          ) : null}

          {selectedSummary ? (
            <div className="history-thread-head">
              <div>
                <h2 className="history-thread-title">{summarizeTitle(selectedSummary.title)}</h2>
                <p className="history-thread-meta">
                  {formatAbsoluteTime(selectedSummary.startedAt)} ·{" "}
                  {historyActionLabel(selectedSummary.action)} · {threadTurnCount(thread)} 轮 ·{" "}
                  {historyEngineLabel(selectedSummary.engine)}
                </p>
              </div>
              <div className="history-thread-actions">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={threadLoading}
                  onClick={() => void refreshSelection()}
                >
                  重新读取
                </Button>
                {confirm?.type === "one" ? (
                  <span className="history-confirm">
                    <span className="history-confirm-text">清空该会话？</span>
                    <Button
                      variant="danger"
                      size="sm"
                      disabled={busy}
                      onClick={() => void clearSession(selectedSummary)}
                    >
                      确认清空
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={busy}
                      onClick={() => setConfirm(null)}
                    >
                      取消
                    </Button>
                  </span>
                ) : (
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={busy}
                    onClick={() => setConfirm({ type: "one", session: selectedSummary })}
                  >
                    清空该会话
                  </Button>
                )}
              </div>
            </div>
          ) : null}

          {thread && selectedSummary ? (
            thread.turns.length === 0 ? (
              <p className="history-hint">该会话没有可展示的问答轮次。</p>
            ) : (
              <div className="history-turns">
                {thread.turns.map((turn, index) => (
                  <article key={turn.task} className={`history-turn is-${turn.status}`}>
                    <div className="history-turn-head">
                      <span className="history-turn-index">第 {index + 1} 轮</span>
                      <span className={`history-turn-status is-${turn.status}`}>
                        {turnStatusLabel(turn.status)}
                      </span>
                      <span className="history-turn-meta">
                        {historyEngineLabel(turn.engine)} · {formatAbsoluteTime(turn.createdAt)}
                      </span>
                    </div>
                    <div className="history-turn-question">
                      <span className="history-turn-role">问</span>
                      <span className="history-turn-question-text">
                        {turn.question || "（首轮框选动作，无追问文本）"}
                      </span>
                    </div>
                    <div className="history-turn-answer">
                      <span className="history-turn-role is-ai">答</span>
                      <pre className="history-turn-answer-text">
                        {turn.answer || "（无回答内容）"}
                      </pre>
                    </div>
                  </article>
                ))}
              </div>
            )
          ) : null}
        </section>
      </div>
    </div>
  );
}
