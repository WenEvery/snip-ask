-- 框选 AI 迁移 002：连续提问历史消息（契约 v5 / spec §3.3）
--
-- 约定：
--   * 时间戳统一为 Unix 毫秒（INTEGER）；
--   * 一轮问答 = 一条 user + 一条 assistant（task_id 关联 tasks 表）；
--   * (task_id, role) 唯一：任务重试/重复落库时按幂等覆盖，不会产生重复轮次；
--   * 历史可关闭（history.enabled = false 时仓储层整体 no-op，不执行任何写入）。
--
-- 读取语义（HistoryRepo）：
--   * history_list  → 按 session_id 聚合：首条 user 消息做 title（≤60 字符），
--                     turns = COUNT(DISTINCT task_id)，started_at 取 sessions.started_at 或首条消息时间；
--   * history_thread→ 按 created_at 正序、同轮 user 在 assistant 之前。

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS messages (
  id         TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  task_id    TEXT NOT NULL,
  action     TEXT NOT NULL,
  mode       TEXT NOT NULL,
  role       TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
  content    TEXT NOT NULL,
  engine     TEXT,
  status     TEXT NOT NULL DEFAULT 'done',
  error_code TEXT,
  created_at INTEGER NOT NULL
);

-- 历史回看：按会话取线程 / 按会话聚合摘要
CREATE INDEX IF NOT EXISTS idx_messages_session_created ON messages (session_id, created_at);
-- 单条消息去重 / 重试覆盖
CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_task_role ON messages (task_id, role);
-- retention 清理
CREATE INDEX IF NOT EXISTS idx_messages_created_at ON messages (created_at);

INSERT OR IGNORE INTO schema_migrations (version, applied_at)
VALUES (2, CAST(strftime('%s', 'now') AS INTEGER) * 1000);
