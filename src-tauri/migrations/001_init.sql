-- 框选 AI 初始 Schema（契约 §10 / 概要设计说明书 §6.5）
-- 约定：
--   * 时间戳统一为 Unix 毫秒（INTEGER）；
--   * 布尔值用 INTEGER 0/1；
--   * 历史库可关闭（history.enabled = false 时仓储层整体 no-op）。

PRAGMA foreign_keys = ON;

-- 迁移版本表：每次结构性迁移插入一条记录
CREATE TABLE IF NOT EXISTS schema_migrations (
  version    INTEGER PRIMARY KEY,
  applied_at INTEGER NOT NULL
);

INSERT OR IGNORE INTO schema_migrations (version, applied_at)
VALUES (1, CAST(strftime('%s', 'now') AS INTEGER) * 1000);

-- 会话：一次「热键 → 框选 → 动作」生命周期
CREATE TABLE IF NOT EXISTS sessions (
  id         TEXT PRIMARY KEY,
  started_at INTEGER NOT NULL,
  rect       TEXT,
  has_text   INTEGER NOT NULL DEFAULT 0,
  ocr_text   TEXT,
  app_exe    TEXT
);

-- 任务：一次具体的搜索 / 翻译 / 解答请求
CREATE TABLE IF NOT EXISTS tasks (
  id             TEXT PRIMARY KEY,
  session_id     TEXT NOT NULL,
  action         TEXT NOT NULL,
  mode           TEXT NOT NULL,
  engine         TEXT,
  status         TEXT NOT NULL,
  first_token_ms INTEGER,
  total_ms       INTEGER,
  tokens_in      INTEGER,
  tokens_out     INTEGER,
  cost_est       REAL,
  error_code     TEXT,
  created_at     INTEGER NOT NULL
);

-- 按日用量聚合（关闭历史时仅保留用量）
CREATE TABLE IF NOT EXISTS usage_daily (
  day       TEXT PRIMARY KEY,
  tasks     INTEGER NOT NULL DEFAULT 0,
  tokens_in INTEGER NOT NULL DEFAULT 0,
  tokens_out INTEGER NOT NULL DEFAULT 0,
  cost_est  REAL NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_tasks_session_id ON tasks (session_id);
CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks (created_at);
CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON sessions (started_at);
