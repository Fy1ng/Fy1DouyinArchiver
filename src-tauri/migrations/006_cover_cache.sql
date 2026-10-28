CREATE TABLE cover_cache (
  video_id INTEGER PRIMARY KEY REFERENCES videos(id) ON DELETE CASCADE,
  remote_url TEXT NOT NULL,
  relative_path TEXT NOT NULL UNIQUE,
  byte_size INTEGER NOT NULL CHECK(byte_size >= 0),
  mime_type TEXT NOT NULL CHECK(mime_type IN ('image/jpeg', 'image/png', 'image/webp', 'image/gif')),
  sha256 TEXT NOT NULL CHECK(length(sha256) = 64),
  downloaded_at TEXT NOT NULL,
  last_accessed_at TEXT NOT NULL
);
CREATE INDEX idx_cover_cache_lru ON cover_cache(last_accessed_at, video_id);

CREATE TABLE cover_cache_tasks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL CHECK(kind IN ('backfill', 'new_records', 'cleanup')),
  status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'paused', 'success', 'failed', 'cancelled')),
  total_items INTEGER NOT NULL DEFAULT 0 CHECK(total_items >= 0),
  completed_items INTEGER NOT NULL DEFAULT 0 CHECK(completed_items >= 0 AND completed_items <= total_items),
  failed_items INTEGER NOT NULL DEFAULT 0 CHECK(failed_items >= 0 AND failed_items <= completed_items),
  created_at TEXT NOT NULL,
  started_at TEXT,
  finished_at TEXT,
  error_code TEXT,
  CHECK((status IN ('success', 'failed', 'cancelled') AND finished_at IS NOT NULL) OR
        (status NOT IN ('success', 'failed', 'cancelled') AND finished_at IS NULL))
);
CREATE UNIQUE INDEX idx_cover_cache_single_active_task
  ON cover_cache_tasks((1)) WHERE status IN ('queued', 'running', 'paused');

CREATE TABLE cover_cache_rules (
  video_id INTEGER PRIMARY KEY REFERENCES videos(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL
);

CREATE TABLE cover_cache_queue (
  video_id INTEGER PRIMARY KEY REFERENCES videos(id) ON DELETE CASCADE,
  sync_job_id INTEGER REFERENCES sync_jobs(id) ON DELETE SET NULL,
  queued_at TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
  last_error TEXT
);
CREATE INDEX idx_cover_cache_queue_order ON cover_cache_queue(queued_at, video_id);

PRAGMA user_version = 6;
