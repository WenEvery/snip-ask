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
import type { HistorySession, HistoryThread } from "@/shared/contract/types";
import { COMMANDS } from "@/shared/contract/names";
import { ipc, toAppError } from "@/shared/ipc";
import { t } from "@/i18n";
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
  return toAppError(err).message;
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
      setNotice(t("已清空「{name}」的记录。", { name: summarizeTitle(session.title, 60, t) }));
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
      setNotice(t("已清空全部历史记录。"));
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

  const visible = useMemo(() => filterSessions(sessions, query, t), [sessions, query]);
  const selectedSummary = useMemo(
    () => sessions.find((item) => item.session === selected) ?? null,
    [sessions, selected],
  );

  return (
    <div className="history-root">
      <header className="history-header">
        <div>
          <h1 className="history-title">{t("历史")}</h1>
          <p className="history-subtitle">
            {t("每次框选的问答都记录在这里，只保存在本机；清空后无法恢复。")}
          </p>
        </div>
        <div className="history-header-actions">
          <Button
            variant="secondary"
            size="sm"
            disabled={listLoading}
            onClick={() => void loadList(true)}
          >
            {listLoading ? t("读取中…") : t("刷新")}
          </Button>
          {confirm?.type === "all" ? (
            <span className="history-confirm">
              <span className="history-confirm-text">
                {t("将清空全部历史记录，且不可恢复，确定？")}
              </span>
              <Button variant="danger" size="sm" disabled={busy} onClick={() => void clearAll()}>
                {busy ? t("清空中…") : t("确认清空全部")}
              </Button>
              <Button variant="ghost" size="sm" disabled={busy} onClick={() => setConfirm(null)}>
                {t("取消")}
              </Button>
            </span>
          ) : (
            <Button
              variant="danger"
              size="sm"
              disabled={busy || sessions.length === 0}
              onClick={() => setConfirm({ type: "all" })}
            >
              {t("清空全部历史")}
            </Button>
          )}
        </div>
      </header>

      {listError ? (
        <div className="history-error" role="alert">
          <strong>{listError}</strong>
          <Button variant="ghost" size="sm" onClick={() => void loadList(true)}>
            {t("重试")}
          </Button>
        </div>
      ) : null}
      {notice ? <p className="history-notice">{notice}</p> : null}

      <div className="history-toolbar">
        <input
          className="history-search"
          type="search"
          value={query}
          placeholder={t("按标题、动作或模型筛选…")}
          onChange={(event) => setQuery(event.target.value)}
        />
        <span className="history-count">
          {t("{visible} / {total} 个会话", { visible: visible.length, total: sessions.length })}
        </span>
      </div>

      <div className="history-layout">
        <section className="history-list" aria-label={t("历史会话列表")}>
          {listLoading && sessions.length === 0 ? (
            <div className="history-loading">
              <Spinner />
              <span>{t("正在读取历史…")}</span>
            </div>
          ) : visible.length === 0 ? (
            <EmptyState
              title={sessions.length === 0 ? t("还没有历史记录") : t("没有找到匹配的记录")}
              description={
                sessions.length === 0
                  ? t("框选并完成一次动作后，记录会自动出现在这里（可在「设置 → 核心」关闭历史）。")
                  : t("换个关键词试试。")
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
                  <span className="history-item-title">{summarizeTitle(session.title, 60, t)}</span>
                  <span className="history-item-meta">
                    <span>{formatHistoryTime(session.startedAt, Date.now(), t)}</span>
                    <span>· {t("{count} 轮", { count: session.turns })}</span>
                    <span>· {historyActionLabel(session.action, t)}</span>
                    <span>· {historyEngineLabel(session.engine, t)}</span>
                  </span>
                </button>
              );
            })
          )}
        </section>

        <section className="history-detail" aria-label={t("历史线程详情")}>
          {!selectedSummary && !threadLoading ? (
            <EmptyState
              title={t("选择一个对话查看内容")}
              description={t(
                "左边是最近的记录，点一下就能看到每一轮的问题、回答、使用的模型和时间。",
              )}
            />
          ) : null}

          {threadLoading ? (
            <div className="history-loading">
              <Spinner />
              <span>{t("正在读取…")}</span>
            </div>
          ) : null}

          {threadError ? (
            <div className="history-error" role="alert">
              <strong>{threadError}</strong>
              <Button variant="ghost" size="sm" onClick={() => void refreshSelection()}>
                {t("重试")}
              </Button>
            </div>
          ) : null}

          {selectedSummary ? (
            <div className="history-thread-head">
              <div>
                <h2 className="history-thread-title">
                  {summarizeTitle(selectedSummary.title, 60, t)}
                </h2>
                <p className="history-thread-meta">
                  {formatAbsoluteTime(selectedSummary.startedAt)} ·{" "}
                  {historyActionLabel(selectedSummary.action, t)} ·{" "}
                  {t("{count} 轮", { count: threadTurnCount(thread) })} ·{" "}
                  {historyEngineLabel(selectedSummary.engine, t)}
                </p>
              </div>
              <div className="history-thread-actions">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={threadLoading}
                  onClick={() => void refreshSelection()}
                >
                  {t("重新读取")}
                </Button>
                {confirm?.type === "one" ? (
                  <span className="history-confirm">
                    <span className="history-confirm-text">{t("清空该会话？")}</span>
                    <Button
                      variant="danger"
                      size="sm"
                      disabled={busy}
                      onClick={() => void clearSession(selectedSummary)}
                    >
                      {t("确认清空")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={busy}
                      onClick={() => setConfirm(null)}
                    >
                      {t("取消")}
                    </Button>
                  </span>
                ) : (
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={busy}
                    onClick={() => setConfirm({ type: "one", session: selectedSummary })}
                  >
                    {t("清空该会话")}
                  </Button>
                )}
              </div>
            </div>
          ) : null}

          {thread && selectedSummary ? (
            thread.turns.length === 0 ? (
              <p className="history-hint">{t("这个对话没有可显示的内容。")}</p>
            ) : (
              <div className="history-turns">
                {thread.turns.map((turn, index) => (
                  <article key={turn.task} className={`history-turn is-${turn.status}`}>
                    <div className="history-turn-head">
                      <span className="history-turn-index">
                        {t("第 {index} 轮", { index: index + 1 })}
                      </span>
                      <span className={`history-turn-status is-${turn.status}`}>
                        {turnStatusLabel(turn.status, t)}
                      </span>
                      <span className="history-turn-meta">
                        {historyEngineLabel(turn.engine, t)} · {formatAbsoluteTime(turn.createdAt)}
                      </span>
                    </div>
                    <div className="history-turn-question">
                      <span className="history-turn-role">{t("问")}</span>
                      <span className="history-turn-question-text">
                        {turn.question || t("（第一次框选，没有追问）")}
                      </span>
                    </div>
                    <div className="history-turn-answer">
                      <span className="history-turn-role is-ai">{t("答")}</span>
                      <pre className="history-turn-answer-text">
                        {turn.answer || t("（无回答内容）")}
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
