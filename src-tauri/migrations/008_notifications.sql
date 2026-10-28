CREATE TABLE notifications (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 64),
  severity TEXT NOT NULL CHECK(severity IN ('info', 'warning', 'error')),
  dedupe_key TEXT NOT NULL CHECK(length(dedupe_key) BETWEEN 1 AND 512),
  title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 200),
  body TEXT NOT NULL CHECK(length(body) BETWEEN 1 AND 4000),
  account_id INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
  related_type TEXT,
  related_id TEXT,
  created_at TEXT NOT NULL,
  read_at TEXT,
  resolved_at TEXT
);

CREATE UNIQUE INDEX idx_notifications_unresolved_dedupe
  ON notifications(dedupe_key)
  WHERE resolved_at IS NULL;

CREATE INDEX idx_notifications_created
  ON notifications(created_at DESC, id DESC);

CREATE INDEX idx_notifications_unread
  ON notifications(read_at, created_at DESC, id DESC)
  WHERE resolved_at IS NULL;
