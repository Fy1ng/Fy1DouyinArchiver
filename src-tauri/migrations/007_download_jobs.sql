CREATE TABLE download_jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  profile_key TEXT NOT NULL,
  created_at TEXT NOT NULL,
  started_at TEXT,
  completed_at TEXT,
  selection_scope TEXT NOT NULL CHECK(selection_scope IN ('selected', 'filtered', 'all')),
  filter_summary TEXT,
  destination_root TEXT NOT NULL,
  requested_quality TEXT NOT NULL CHECK(requested_quality IN ('highest', 'balanced', 'space_saver')),
  state TEXT NOT NULL CHECK(state IN (
    'queued', 'running', 'paused', 'interrupted', 'completed',
    'completed_with_errors', 'cancelled'
  )),
  candidate_count INTEGER NOT NULL DEFAULT 0 CHECK(candidate_count >= 0),
  resolved_count INTEGER NOT NULL DEFAULT 0 CHECK(resolved_count >= 0),
  completed_count INTEGER NOT NULL DEFAULT 0 CHECK(completed_count >= 0),
  skipped_count INTEGER NOT NULL DEFAULT 0 CHECK(skipped_count >= 0),
  failed_count INTEGER NOT NULL DEFAULT 0 CHECK(failed_count >= 0),
  total_bytes INTEGER NOT NULL DEFAULT 0 CHECK(total_bytes >= 0),
  downloaded_bytes INTEGER NOT NULL DEFAULT 0 CHECK(downloaded_bytes >= 0),
  error_code TEXT
);

CREATE TABLE download_items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  job_id INTEGER NOT NULL REFERENCES download_jobs(id) ON DELETE CASCADE,
  aweme_id TEXT NOT NULL,
  canonical_url TEXT NOT NULL,
  content_type TEXT NOT NULL,
  title TEXT,
  author_name TEXT,
  published_at TEXT,
  filter_context TEXT,
  relative_work_directory TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN (
    'queued', 'resolving', 'downloading', 'paused', 'interrupted',
    'completed', 'skipped', 'failed', 'superseded', 'cancelled'
  )),
  resolution_attempt_count INTEGER NOT NULL DEFAULT 0 CHECK(resolution_attempt_count >= 0),
  error_code TEXT,
  error_message TEXT,
  manifest_generation INTEGER NOT NULL DEFAULT 0 CHECK(manifest_generation >= 0),
  completed_asset_count INTEGER NOT NULL DEFAULT 0 CHECK(completed_asset_count >= 0),
  total_required_asset_count INTEGER NOT NULL DEFAULT 0 CHECK(total_required_asset_count >= 0),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT,
  UNIQUE(job_id, aweme_id)
);

CREATE TABLE download_assets (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  item_id INTEGER NOT NULL REFERENCES download_items(id) ON DELETE CASCADE,
  stable_key TEXT NOT NULL,
  asset_type TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
  requested_profile TEXT NOT NULL CHECK(requested_profile IN ('highest', 'balanced', 'space_saver')),
  actual_quality TEXT,
  width INTEGER CHECK(width IS NULL OR width > 0),
  height INTEGER CHECK(height IS NULL OR height > 0),
  bitrate INTEGER CHECK(bitrate IS NULL OR bitrate >= 0),
  codec TEXT,
  mime_type TEXT,
  relative_final_path TEXT NOT NULL,
  relative_part_path TEXT NOT NULL,
  expected_size INTEGER CHECK(expected_size IS NULL OR expected_size >= 0),
  downloaded_bytes INTEGER NOT NULL DEFAULT 0 CHECK(downloaded_bytes >= 0),
  etag TEXT,
  last_modified TEXT,
  validated_size INTEGER CHECK(validated_size IS NULL OR validated_size >= 0),
  validated_sha256 TEXT,
  validated_at TEXT,
  manifest_generation INTEGER NOT NULL CHECK(manifest_generation >= 0),
  required INTEGER NOT NULL DEFAULT 1 CHECK(required IN (0, 1)),
  state TEXT NOT NULL CHECK(state IN (
    'queued', 'downloading', 'paused', 'interrupted', 'completed',
    'skipped', 'failed', 'superseded', 'cancelled'
  )),
  retry_count INTEGER NOT NULL DEFAULT 0 CHECK(retry_count >= 0),
  last_error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT,
  UNIQUE(item_id, stable_key, manifest_generation)
);

CREATE INDEX idx_download_jobs_active
  ON download_jobs(state, created_at)
  WHERE state IN ('queued', 'running', 'paused', 'interrupted');

CREATE INDEX idx_download_items_job_state
  ON download_items(job_id, state, id);

CREATE INDEX idx_download_assets_item_state
  ON download_assets(item_id, state, ordinal);

CREATE INDEX idx_download_assets_historical_reuse
  ON download_assets(stable_key, requested_profile, relative_final_path, state)
  WHERE state IN ('completed', 'skipped');
