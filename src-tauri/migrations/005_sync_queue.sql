CREATE TABLE sync_jobs_v5_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  source TEXT NOT NULL CHECK(source IN ('history', 'likes', 'favorites', 'remote_folder')),
  source_instance_key TEXT NOT NULL,
  trigger TEXT NOT NULL CHECK(trigger IN ('manual', 'automatic')),
  queued_at TEXT NOT NULL,
  queue_seq INTEGER,
  scheduled_for TEXT,
  settings_revision INTEGER,
  config_sha256 TEXT,
  payload_json TEXT,
  payload_sha256 TEXT,
  equivalence_sha256 TEXT,
  started_at TEXT,
  finished_at TEXT,
  status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'success', 'partial', 'failed', 'cancelled')),
  inserted_videos INTEGER NOT NULL DEFAULT 0,
  inserted_records INTEGER NOT NULL DEFAULT 0,
  error_code TEXT,
  error_message TEXT,
  CHECK((status IN ('queued', 'running') AND finished_at IS NULL) OR
        (status IN ('success', 'partial', 'failed', 'cancelled') AND finished_at IS NOT NULL)),
  CHECK(status NOT IN ('queued', 'running') OR
        (queue_seq IS NOT NULL AND settings_revision IS NOT NULL AND
         payload_json IS NOT NULL AND payload_sha256 IS NOT NULL AND
         equivalence_sha256 IS NOT NULL)),
  CHECK(status NOT IN ('queued', 'running') OR
        (trigger = 'manual' AND config_sha256 IS NULL) OR
        (trigger = 'automatic' AND config_sha256 IS NOT NULL))
);

INSERT INTO sync_jobs_v5_new (
  id, account_id, source, source_instance_key, trigger, queued_at,
  queue_seq, scheduled_for, settings_revision, config_sha256,
  payload_json, payload_sha256, equivalence_sha256,
  started_at, finished_at, status, inserted_videos, inserted_records,
  error_code, error_message
)
SELECT
  id,
  account_id,
  source,
  '',
  'manual',
  started_at,
  NULL,
  NULL,
  NULL,
  NULL,
  NULL,
  NULL,
  NULL,
  started_at,
  CASE
    WHEN status = 'running' THEN (SELECT frozen_at FROM temp.emily_schema_v5_context)
    ELSE COALESCE(finished_at, started_at)
  END,
  CASE WHEN status = 'running' THEN 'failed' ELSE status END,
  inserted_videos,
  inserted_records,
  CASE
    WHEN status = 'running' THEN 'schema_upgrade_interrupted_running'
    ELSE error_code
  END,
  error_message
FROM sync_jobs;

DROP TABLE sync_jobs;
ALTER TABLE sync_jobs_v5_new RENAME TO sync_jobs;

DELETE FROM sqlite_sequence WHERE name IN ('sync_jobs', 'sync_jobs_v5_new');
INSERT INTO sqlite_sequence(name, seq)
SELECT 'sync_jobs', seq FROM temp.emily_schema_v5_sequence;

CREATE INDEX idx_sync_jobs_account_status
  ON sync_jobs(account_id, status, id DESC);
CREATE INDEX idx_sync_jobs_account_source
  ON sync_jobs(account_id, source, id DESC);
CREATE UNIQUE INDEX idx_sync_jobs_active_task_key
  ON sync_jobs(account_id, source, source_instance_key)
  WHERE status IN ('queued', 'running');
CREATE UNIQUE INDEX idx_sync_jobs_single_running
  ON sync_jobs((1))
  WHERE status = 'running';
CREATE INDEX idx_sync_jobs_queue
  ON sync_jobs(status, trigger, queue_seq, id);
CREATE INDEX idx_sync_jobs_terminal_task_key
  ON sync_jobs(account_id, source, source_instance_key, finished_at DESC, id DESC)
  WHERE status IN ('success', 'partial', 'failed', 'cancelled');

CREATE TABLE sync_batches (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  trigger TEXT NOT NULL CHECK(trigger = 'manual'),
  scope TEXT NOT NULL CHECK(scope IN ('currentAccountAvailableSources', 'allActiveAccounts')),
  created_at TEXT NOT NULL,
  finished_at TEXT,
  next_entry_seq INTEGER NOT NULL DEFAULT 1 CHECK(next_entry_seq BETWEEN 1 AND 9223372036854775807),
  status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'success', 'partial', 'failed', 'cancelled')),
  CHECK((status IN ('queued', 'running') AND finished_at IS NULL) OR
        (status IN ('success', 'partial', 'failed', 'cancelled') AND finished_at IS NOT NULL))
);

CREATE TABLE sync_batch_targets (
  batch_id INTEGER NOT NULL REFERENCES sync_batches(id) ON DELETE CASCADE,
  account_id INTEGER NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('history', 'likes', 'favorites', 'remote_folder')),
  source_instance_key TEXT NOT NULL,
  target_kind TEXT NOT NULL CHECK(target_kind IN ('task', 'aggregate')),
  resolution_kind TEXT NOT NULL CHECK(resolution_kind IN ('job', 'issue')),
  PRIMARY KEY(batch_id, account_id, source, source_instance_key),
  UNIQUE(batch_id, account_id, source, source_instance_key, resolution_kind),
  CHECK(target_kind = 'task' OR
        (target_kind = 'aggregate' AND resolution_kind = 'issue' AND
         source = 'remote_folder' AND source_instance_key = ''))
);

CREATE TABLE sync_queue_state (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  next_queue_seq INTEGER NOT NULL CHECK(next_queue_seq BETWEEN 1 AND 9223372036854775807)
);
INSERT INTO sync_queue_state(singleton, next_queue_seq) VALUES (1, 1);

CREATE TABLE remote_folder_registry_state (
  account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9223372036854775807)
);
INSERT INTO remote_folder_registry_state(account_id, revision)
SELECT id, 1 FROM accounts;

CREATE INDEX idx_remote_folders_scheduler
  ON remote_folders(account_id, remote_removed_at, remote_folder_id);

CREATE TABLE sync_batch_jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  batch_id INTEGER NOT NULL REFERENCES sync_batches(id) ON DELETE CASCADE,
  account_id INTEGER NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('history', 'likes', 'favorites', 'remote_folder')),
  source_instance_key TEXT NOT NULL,
  resolution_kind TEXT NOT NULL DEFAULT 'job' CHECK(resolution_kind = 'job'),
  job_id INTEGER REFERENCES sync_jobs(id) ON DELETE SET NULL,
  entry_seq INTEGER NOT NULL,
  job_identity_sha256 TEXT NOT NULL,
  job_view_snapshot_json TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('root', 'discoveredChild', 'attached')),
  UNIQUE(batch_id, account_id, source, source_instance_key),
  UNIQUE(batch_id, job_identity_sha256),
  UNIQUE(batch_id, entry_seq),
  FOREIGN KEY(batch_id, account_id, source, source_instance_key, resolution_kind)
    REFERENCES sync_batch_targets(batch_id, account_id, source, source_instance_key, resolution_kind)
    ON DELETE CASCADE
);
CREATE INDEX idx_sync_batch_jobs_job ON sync_batch_jobs(job_id, batch_id);

CREATE TABLE sync_job_issues (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  job_id INTEGER NOT NULL REFERENCES sync_jobs(id) ON DELETE CASCADE,
  phase TEXT NOT NULL,
  code TEXT NOT NULL,
  severity TEXT NOT NULL CHECK(severity IN ('warning', 'incomplete')),
  message TEXT,
  created_at TEXT NOT NULL,
  UNIQUE(job_id, phase, code)
);

CREATE TABLE sync_batch_issues (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  batch_id INTEGER NOT NULL REFERENCES sync_batches(id) ON DELETE CASCADE,
  entry_seq INTEGER NOT NULL,
  account_id INTEGER NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('history', 'likes', 'favorites', 'remote_folder')),
  source_instance_key TEXT NOT NULL,
  resolution_kind TEXT NOT NULL DEFAULT 'issue' CHECK(resolution_kind = 'issue'),
  phase TEXT NOT NULL CHECK(phase IN ('initialExpansion', 'discoveryExpansion')),
  kind TEXT NOT NULL CHECK(kind IN ('skipped', 'conflict')),
  reason TEXT NOT NULL,
  requested_payload_sha256 TEXT,
  existing_payload_sha256 TEXT,
  existing_job_id INTEGER REFERENCES sync_jobs(id) ON DELETE SET NULL,
  conflicting_job_snapshot_json TEXT,
  created_at TEXT NOT NULL,
  UNIQUE(batch_id, account_id, source, source_instance_key),
  UNIQUE(batch_id, entry_seq),
  CHECK((kind = 'skipped' AND existing_job_id IS NULL AND conflicting_job_snapshot_json IS NULL AND
         existing_payload_sha256 IS NULL) OR
        (kind = 'conflict' AND requested_payload_sha256 IS NOT NULL AND
         existing_payload_sha256 IS NOT NULL AND conflicting_job_snapshot_json IS NOT NULL)),
  FOREIGN KEY(batch_id, account_id, source, source_instance_key, resolution_kind)
    REFERENCES sync_batch_targets(batch_id, account_id, source, source_instance_key, resolution_kind)
    ON DELETE CASCADE
);
CREATE INDEX idx_sync_batch_issues_existing_job
  ON sync_batch_issues(existing_job_id, batch_id);

CREATE TRIGGER reject_terminal_batch_job
BEFORE INSERT ON sync_batch_jobs
WHEN (SELECT status FROM sync_batches WHERE id = NEW.batch_id)
     IN ('success', 'partial', 'failed', 'cancelled')
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_terminal');
END;

CREATE TRIGGER reject_terminal_batch_target
BEFORE INSERT ON sync_batch_targets
WHEN (SELECT status FROM sync_batches WHERE id = NEW.batch_id)
     IN ('success', 'partial', 'failed', 'cancelled')
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_terminal');
END;

CREATE TRIGGER reject_batch_target_update
BEFORE UPDATE ON sync_batch_targets
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_target_immutable');
END;

CREATE TRIGGER reject_direct_batch_target_delete
BEFORE DELETE ON sync_batch_targets
WHEN EXISTS (SELECT 1 FROM sync_batches WHERE id = OLD.batch_id)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_target_delete_forbidden');
END;

CREATE TRIGGER reject_sync_job_task_key_update
BEFORE UPDATE OF account_id, source, source_instance_key ON sync_jobs
WHEN NEW.account_id <> OLD.account_id
  OR NEW.source <> OLD.source
  OR NEW.source_instance_key <> OLD.source_instance_key
BEGIN
  SELECT RAISE(ABORT, 'sync_job_task_key_immutable');
END;

CREATE TRIGGER reject_initial_null_batch_job
BEFORE INSERT ON sync_batch_jobs
WHEN NEW.job_id IS NULL
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_required');
END;

CREATE TRIGGER reject_batch_job_task_key_mismatch
BEFORE INSERT ON sync_batch_jobs
WHEN NEW.job_id IS NOT NULL AND NOT EXISTS (
  SELECT 1 FROM sync_jobs j
  WHERE j.id = NEW.job_id
    AND j.account_id = NEW.account_id
    AND j.source = NEW.source
    AND j.source_instance_key = NEW.source_instance_key
    AND j.status IN ('queued', 'running')
)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_task_key_mismatch');
END;

CREATE TRIGGER reject_batch_job_task_key_update
BEFORE UPDATE OF account_id, source, source_instance_key ON sync_batch_jobs
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_task_key_immutable');
END;

CREATE TRIGGER reject_batch_job_identity_update
BEFORE UPDATE OF batch_id, entry_seq, role, resolution_kind,
                 job_identity_sha256
ON sync_batch_jobs
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_identity_immutable');
END;

CREATE TRIGGER reject_batch_job_rebind
BEFORE UPDATE OF job_id ON sync_batch_jobs
WHEN NEW.job_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_rebind_forbidden');
END;

CREATE TRIGGER reject_batch_job_manual_unlink
BEFORE UPDATE OF job_id ON sync_batch_jobs
WHEN NEW.job_id IS NULL
  AND OLD.job_id IS NOT NULL
  AND EXISTS (SELECT 1 FROM sync_jobs WHERE id = OLD.job_id)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_manual_unlink_forbidden');
END;

CREATE TRIGGER reject_direct_batch_job_delete
BEFORE DELETE ON sync_batch_jobs
WHEN EXISTS (SELECT 1 FROM sync_batches WHERE id = OLD.batch_id)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_delete_forbidden');
END;

CREATE TRIGGER reject_terminal_batch_issue
BEFORE INSERT ON sync_batch_issues
WHEN (SELECT status FROM sync_batches WHERE id = NEW.batch_id)
     IN ('success', 'partial', 'failed', 'cancelled')
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_terminal');
END;

CREATE TRIGGER reject_conflict_issue_job_identity
BEFORE INSERT ON sync_batch_issues
WHEN NEW.kind = 'conflict' AND (
  NEW.existing_job_id IS NULL OR NOT EXISTS (
    SELECT 1 FROM sync_jobs j
    WHERE j.id = NEW.existing_job_id
      AND j.account_id = NEW.account_id
      AND j.source = NEW.source
      AND j.source_instance_key = NEW.source_instance_key
      AND j.status IN ('queued', 'running')
  )
)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_conflict_job_identity_mismatch');
END;

CREATE TRIGGER reject_batch_issue_evidence_update
BEFORE UPDATE OF batch_id, account_id, source, source_instance_key, resolution_kind,
                 phase, kind, reason, requested_payload_sha256,
                 existing_payload_sha256, conflicting_job_snapshot_json,
                 entry_seq, created_at
ON sync_batch_issues
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_issue_evidence_immutable');
END;

CREATE TRIGGER reject_batch_issue_job_rebind
BEFORE UPDATE OF existing_job_id ON sync_batch_issues
WHEN NEW.existing_job_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_issue_job_rebind_forbidden');
END;

CREATE TRIGGER reject_batch_issue_manual_unlink
BEFORE UPDATE OF existing_job_id ON sync_batch_issues
WHEN NEW.existing_job_id IS NULL
  AND OLD.existing_job_id IS NOT NULL
  AND EXISTS (SELECT 1 FROM sync_jobs WHERE id = OLD.existing_job_id)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_issue_manual_unlink_forbidden');
END;

CREATE TRIGGER reject_direct_batch_issue_delete
BEFORE DELETE ON sync_batch_issues
WHEN EXISTS (SELECT 1 FROM sync_batches WHERE id = OLD.batch_id)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_issue_delete_forbidden');
END;

CREATE TRIGGER reject_terminal_batch_reopen
BEFORE UPDATE OF status ON sync_batches
WHEN OLD.status IN ('success', 'partial', 'failed', 'cancelled')
     AND NEW.status <> OLD.status
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_terminal');
END;

DROP TABLE temp.emily_schema_v5_sequence;
DROP TABLE temp.emily_schema_v5_context;
