CREATE TABLE IF NOT EXISTS sync_jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  started_at TEXT NOT NULL,
  finished_at TEXT,
  source_file TEXT,
  status TEXT NOT NULL,
  inserted_videos INTEGER NOT NULL DEFAULT 0,
  inserted_watch_records INTEGER NOT NULL DEFAULT 0,
  error_message TEXT
);

DROP TABLE IF EXISTS watch_records_v2;
DROP TABLE IF EXISTS videos_v2;

CREATE TABLE videos_v2 (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  aweme_id TEXT NOT NULL UNIQUE,
  canonical_url TEXT NOT NULL UNIQUE,
  content_type TEXT NOT NULL CHECK(content_type IN ('video', 'note', 'unknown')),
  title TEXT,
  author_name TEXT,
  cover_url TEXT,
  like_count INTEGER,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL
);

CREATE TABLE watch_records_v2 (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  video_id INTEGER NOT NULL REFERENCES videos_v2(id) ON DELETE CASCADE,
  history_time_ms INTEGER NOT NULL,
  watched_at TEXT NOT NULL,
  watched_date TEXT NOT NULL,
  source_position INTEGER NOT NULL,
  watch_progress_rate REAL CHECK(watch_progress_rate BETWEEN 0.0 AND 1.0),
  sync_job_id INTEGER REFERENCES sync_jobs(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(video_id, history_time_ms)
);

DROP TABLE IF EXISTS watch_records;
DROP TABLE IF EXISTS videos;

ALTER TABLE videos_v2 RENAME TO videos;
ALTER TABLE watch_records_v2 RENAME TO watch_records;

CREATE INDEX idx_watch_records_history_time ON watch_records(history_time_ms DESC);
CREATE INDEX idx_watch_records_source_position ON watch_records(source_position ASC);
CREATE INDEX idx_videos_title ON videos(title);
CREATE INDEX idx_videos_author ON videos(author_name);
CREATE UNIQUE INDEX idx_watch_records_history_identity
  ON watch_records(video_id, history_time_ms);

UPDATE sync_jobs SET source_file = NULL;

PRAGMA user_version = 2;
