DROP TABLE IF EXISTS local_folder_item_accounts;
DROP TABLE IF EXISTS local_folder_items;
DROP TABLE IF EXISTS local_folders;
DROP TABLE IF EXISTS remote_folder_items;
DROP TABLE IF EXISTS remote_folders;
DROP TABLE IF EXISTS favorite_items;
DROP TABLE IF EXISTS user_likes;
DROP TABLE IF EXISTS watch_records;
DROP TABLE IF EXISTS sync_jobs;
DROP TABLE IF EXISTS videos;
DROP TABLE IF EXISTS accounts;

CREATE TABLE accounts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  uid TEXT,
  sec_uid TEXT,
  profile_key TEXT NOT NULL UNIQUE,
  nickname TEXT,
  avatar_url TEXT,
  status TEXT NOT NULL CHECK(status IN ('pending_login', 'active', 'login_expired', 'archived')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_login_validated_at TEXT,
  CHECK(uid IS NULL OR length(trim(uid)) > 0),
  CHECK(sec_uid IS NULL OR length(trim(sec_uid)) > 0),
  CHECK(uid IS NOT NULL OR sec_uid IS NOT NULL OR status = 'pending_login')
);
CREATE UNIQUE INDEX idx_accounts_uid ON accounts(uid) WHERE uid IS NOT NULL;
CREATE UNIQUE INDEX idx_accounts_sec_uid ON accounts(sec_uid) WHERE sec_uid IS NOT NULL;

CREATE TABLE videos (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  aweme_id TEXT NOT NULL UNIQUE,
  canonical_url TEXT NOT NULL,
  content_type TEXT NOT NULL CHECK(content_type IN ('video', 'note', 'unknown')),
  title TEXT,
  author_name TEXT,
  cover_url TEXT,
  like_count INTEGER,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL
);
CREATE INDEX idx_videos_canonical_url ON videos(canonical_url);
CREATE INDEX idx_videos_title ON videos(title);
CREATE INDEX idx_videos_author ON videos(author_name);

CREATE TABLE sync_jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  source TEXT NOT NULL CHECK(source IN ('history', 'likes', 'favorites', 'remote_folder')),
  started_at TEXT NOT NULL,
  finished_at TEXT,
  status TEXT NOT NULL CHECK(status IN ('running', 'success', 'failed', 'cancelled')),
  inserted_videos INTEGER NOT NULL DEFAULT 0,
  inserted_records INTEGER NOT NULL DEFAULT 0,
  error_code TEXT,
  error_message TEXT
);
CREATE INDEX idx_sync_jobs_account_status
  ON sync_jobs(account_id, status, id DESC);
CREATE INDEX idx_sync_jobs_account_source
  ON sync_jobs(account_id, source, id DESC);

CREATE TABLE watch_records (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  history_time_ms INTEGER NOT NULL,
  watched_at TEXT NOT NULL,
  watched_date TEXT NOT NULL,
  source_position INTEGER NOT NULL,
  watch_progress_rate REAL CHECK(watch_progress_rate BETWEEN 0.0 AND 1.0),
  sync_job_id INTEGER REFERENCES sync_jobs(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(account_id, video_id, history_time_ms)
);
CREATE INDEX idx_watch_records_history_time
  ON watch_records(account_id, history_time_ms DESC, source_position ASC, id DESC);

CREATE TABLE user_likes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  remote_created_at_ms INTEGER,
  remote_removed_at TEXT,
  source_position INTEGER,
  UNIQUE(account_id, video_id)
);
CREATE INDEX idx_user_likes_account_removed
  ON user_likes(account_id, remote_removed_at, source_position);

CREATE TABLE favorite_items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  remote_created_at_ms INTEGER,
  remote_removed_at TEXT,
  source_position INTEGER,
  UNIQUE(account_id, video_id)
);
CREATE INDEX idx_favorite_items_account_removed
  ON favorite_items(account_id, remote_removed_at, source_position);

CREATE TABLE remote_folders (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  remote_folder_id TEXT NOT NULL,
  name TEXT NOT NULL,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  remote_removed_at TEXT,
  UNIQUE(account_id, remote_folder_id)
);
CREATE INDEX idx_remote_folders_account_removed
  ON remote_folders(account_id, remote_removed_at);

CREATE TABLE remote_folder_items (
  folder_id INTEGER NOT NULL REFERENCES remote_folders(id) ON DELETE CASCADE,
  video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  remote_removed_at TEXT,
  PRIMARY KEY(folder_id, video_id)
);
CREATE INDEX idx_remote_folder_items_removed
  ON remote_folder_items(remote_removed_at, folder_id);

CREATE TABLE local_folders (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  scope TEXT NOT NULL CHECK(scope IN ('account', 'aggregate')),
  owner_account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
  name TEXT NOT NULL COLLATE NOCASE,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK((scope = 'account' AND owner_account_id IS NOT NULL) OR
        (scope = 'aggregate' AND owner_account_id IS NULL))
);
CREATE UNIQUE INDEX idx_local_folders_account_name
  ON local_folders(owner_account_id, name)
  WHERE scope = 'account';
CREATE UNIQUE INDEX idx_local_folders_aggregate_name
  ON local_folders(name)
  WHERE scope = 'aggregate';

CREATE TABLE local_folder_items (
  folder_id INTEGER NOT NULL REFERENCES local_folders(id) ON DELETE CASCADE,
  video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  added_at TEXT NOT NULL,
  PRIMARY KEY(folder_id, video_id)
);

CREATE TABLE local_folder_item_accounts (
  folder_id INTEGER NOT NULL,
  video_id INTEGER NOT NULL,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  added_at TEXT NOT NULL,
  PRIMARY KEY(folder_id, video_id, account_id),
  FOREIGN KEY(folder_id, video_id) REFERENCES local_folder_items(folder_id, video_id) ON DELETE CASCADE
);
CREATE INDEX idx_local_folder_item_accounts_account
  ON local_folder_item_accounts(account_id, folder_id);

PRAGMA user_version = 3;
