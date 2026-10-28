use super::{
    job_identity_sha256, valid_sha256, SyncBatchCountsV1, SyncBatchEntriesPageV1, SyncBatchEntryV1,
    SyncBatchIssuePhaseV1, SyncBatchStatusV1, SyncBatchViewV1, SyncBatchesPageV1, SyncConflictV1,
    SyncJobIssueSeverityV1, SyncJobIssueV1, SyncJobSnapshotV1, SyncJobStatusV1, SyncJobViewV1,
    SyncScopeV1, SyncSkipV1, SyncSourceV1, SyncTriggerV1, TaskKeyV1,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{cmp::Ordering, collections::HashMap};

const CURSOR_SCHEMA_VERSION: u32 = 1;
const CURSOR_DOMAIN: &[u8] = b"emily.sync-batch-cursor.v1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSyncJobV1 {
    pub task_key: TaskKeyV1,
    pub trigger: SyncTriggerV1,
    pub queued_at: String,
    pub scheduled_for: Option<String>,
    pub settings_revision: i64,
    pub config_sha256: Option<String>,
    pub payload_json: String,
    pub payload_sha256: String,
    pub equivalence_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSyncJobV1 {
    pub snapshot: SyncJobSnapshotV1,
    pub queued_at: String,
    pub queue_seq: i64,
    pub scheduled_for: Option<String>,
    pub settings_revision: i64,
    pub config_sha256: Option<String>,
    pub payload_json: String,
    pub equivalence_sha256: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub inserted_videos: i64,
    pub inserted_records: i64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueSortKeyV1 {
    Manual {
        queue_seq: i64,
        job_id: i64,
    },
    Automatic {
        scheduled_for_ms: i64,
        account_id: i64,
        source: SyncSourceV1,
        source_instance_key: String,
        queue_seq: i64,
        job_id: i64,
    },
}

impl Ord for QueueSortKeyV1 {
    fn cmp(&self, other: &Self) -> Ordering {
        use QueueSortKeyV1::{Automatic, Manual};
        match (self, other) {
            (
                Manual {
                    queue_seq: left_seq,
                    job_id: left_id,
                },
                Manual {
                    queue_seq: right_seq,
                    job_id: right_id,
                },
            ) => (left_seq, left_id).cmp(&(right_seq, right_id)),
            (Manual { .. }, Automatic { .. }) => Ordering::Less,
            (Automatic { .. }, Manual { .. }) => Ordering::Greater,
            (
                Automatic {
                    scheduled_for_ms: left_scheduled,
                    account_id: left_account,
                    source: left_source,
                    source_instance_key: left_instance,
                    queue_seq: left_seq,
                    job_id: left_id,
                },
                Automatic {
                    scheduled_for_ms: right_scheduled,
                    account_id: right_account,
                    source: right_source,
                    source_instance_key: right_instance,
                    queue_seq: right_seq,
                    job_id: right_id,
                },
            ) => (
                left_scheduled,
                left_account,
                left_source,
                left_instance,
                left_seq,
                left_id,
            )
                .cmp(&(
                    right_scheduled,
                    right_account,
                    right_source,
                    right_instance,
                    right_seq,
                    right_id,
                )),
        }
    }
}

impl PartialOrd for QueueSortKeyV1 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalJobUpdateV1 {
    pub status: SyncJobStatusV1,
    pub finished_at: String,
    pub inserted_videos: i64,
    pub inserted_records: i64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallRecoveryResultV1 {
    pub running_job_ids: Vec<i64>,
    pub queued_job_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalAnchorV1 {
    pub job_id: i64,
    pub status: SyncJobStatusV1,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchJobRoleV1 {
    Root,
    DiscoveredChild,
    Attached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchTargetKindV1 {
    Task,
    Aggregate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewSyncBatchIssueKindV1 {
    Skipped {
        reason: String,
        requested_payload_sha256: Option<String>,
    },
    Conflict {
        reason: String,
        requested_payload_sha256: String,
        existing_payload_sha256: String,
        existing_job_id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSyncBatchIssueV1 {
    pub task_key: TaskKeyV1,
    pub target_kind_aggregate: bool,
    pub phase: SyncBatchIssuePhaseV1,
    pub kind: NewSyncBatchIssueKindV1,
}

pub struct QueueStore;

impl QueueStore {
    pub fn enqueue_job(
        connection: &mut Connection,
        input: &NewSyncJobV1,
    ) -> Result<StoredSyncJobV1, String> {
        let transaction = immediate_transaction(connection)?;
        let stored = Self::enqueue_job_tx(&transaction, input)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(stored)
    }

    pub fn enqueue_job_tx(
        transaction: &Transaction<'_>,
        input: &NewSyncJobV1,
    ) -> Result<StoredSyncJobV1, String> {
        validate_new_job(input)?;
        let queue_seq = Self::reserve_queue_sequence_tx(transaction)?;
        transaction
            .execute(
                "INSERT INTO sync_jobs
                 (account_id, source, source_instance_key, trigger, queued_at, queue_seq,
                  scheduled_for, settings_revision, config_sha256, payload_json,
                  payload_sha256, equivalence_sha256, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'queued')",
                params![
                    input.task_key.account_id,
                    input.task_key.source.sqlite_name(),
                    input.task_key.source_instance_key,
                    trigger_name(input.trigger),
                    input.queued_at,
                    queue_seq,
                    input.scheduled_for,
                    input.settings_revision,
                    input.config_sha256,
                    input.payload_json,
                    input.payload_sha256,
                    input.equivalence_sha256,
                ],
            )
            .map_err(map_job_insert_error)?;
        load_stored_job(transaction, transaction.last_insert_rowid())
    }

    pub fn reserve_queue_sequence_tx(transaction: &Transaction<'_>) -> Result<i64, String> {
        let next: i64 = transaction
            .query_row(
                "SELECT next_queue_seq FROM sync_queue_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if next == i64::MAX {
            return Err("sync_queue_sequence_exhausted".to_string());
        }
        transaction
            .execute(
                "UPDATE sync_queue_state SET next_queue_seq = ?1 WHERE singleton = 1",
                [next + 1],
            )
            .map_err(|error| error.to_string())?;
        Ok(next)
    }

    pub fn list_queued_jobs(connection: &Connection) -> Result<Vec<StoredSyncJobV1>, String> {
        let mut statement = connection
            .prepare("SELECT id FROM sync_jobs WHERE status = 'queued'")
            .map_err(|error| error.to_string())?;
        let ids = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let jobs = ids
            .into_iter()
            .map(|id| load_stored_job(connection, id))
            .collect::<Result<Vec<_>, _>>()?;
        let mut keyed = jobs
            .into_iter()
            .map(|job| Ok((queue_sort_key(&job)?, job)))
            .collect::<Result<Vec<_>, String>>()?;
        keyed.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(keyed.into_iter().map(|(_, job)| job).collect())
    }

    pub fn list_running_jobs(connection: &Connection) -> Result<Vec<StoredSyncJobV1>, String> {
        let mut statement = connection
            .prepare("SELECT id FROM sync_jobs WHERE status = 'running' ORDER BY id")
            .map_err(|error| error.to_string())?;
        let ids = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        ids.into_iter()
            .map(|id| load_stored_job(connection, id))
            .collect()
    }

    pub fn assert_no_running_jobs(connection: &Connection) -> Result<(), String> {
        let running_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_jobs WHERE status = 'running')",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if running_exists {
            Err("sync_running_job_recovery_required".to_string())
        } else {
            Ok(())
        }
    }

    pub fn assert_no_active_jobs(connection: &Connection) -> Result<(), String> {
        let active_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_jobs WHERE status IN ('queued', 'running'))",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if active_exists {
            Err("sync_active_jobs_remain".to_string())
        } else {
            Ok(())
        }
    }

    pub fn list_uninstall_recovery_job_keys(
        connection: &Connection,
        error_code: &str,
    ) -> Result<Vec<(i64, i64)>, String> {
        if error_code.trim().is_empty() {
            return Err("invalid_sync_error_code".to_string());
        }
        let mut statement = connection
            .prepare(
                "SELECT account_id, id FROM sync_jobs
                 WHERE status = 'cancelled' AND error_code = ?1
                 ORDER BY id",
            )
            .map_err(|error| error.to_string())?;
        let keys = statement
            .query_map([error_code], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(keys)
    }

    pub fn get_job_view(connection: &Connection, job_id: i64) -> Result<SyncJobViewV1, String> {
        load_job_view(connection, job_id)
    }

    pub fn get_stored_job(connection: &Connection, job_id: i64) -> Result<StoredSyncJobV1, String> {
        load_stored_job(connection, job_id)
    }

    pub fn find_active_job_tx(
        transaction: &Transaction<'_>,
        task_key: &TaskKeyV1,
    ) -> Result<Option<StoredSyncJobV1>, String> {
        Self::find_active_job(transaction, task_key)
    }

    pub fn find_active_job(
        connection: &Connection,
        task_key: &TaskKeyV1,
    ) -> Result<Option<StoredSyncJobV1>, String> {
        validate_task_key(task_key)?;
        connection
            .query_row(
                "SELECT id FROM sync_jobs
                 WHERE account_id = ?1 AND source = ?2 AND source_instance_key = ?3
                   AND status IN ('queued', 'running')",
                params![
                    task_key.account_id,
                    task_key.source.sqlite_name(),
                    task_key.source_instance_key,
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .map(|job_id| load_stored_job(connection, job_id))
            .transpose()
    }

    pub fn promote_automatic_job_tx(
        transaction: &Transaction<'_>,
        job_id: i64,
        input: &NewSyncJobV1,
    ) -> Result<StoredSyncJobV1, String> {
        validate_new_job(input)?;
        if input.trigger != SyncTriggerV1::Manual {
            return Err("invalid_manual_sync_job".to_string());
        }
        let queue_seq = Self::reserve_queue_sequence_tx(transaction)?;
        let updated = transaction
            .execute(
                "UPDATE sync_jobs
                 SET trigger = 'manual', queued_at = ?1, queue_seq = ?2,
                     scheduled_for = NULL, settings_revision = ?3, config_sha256 = NULL,
                     payload_json = ?4, payload_sha256 = ?5, equivalence_sha256 = ?6
                 WHERE id = ?7 AND trigger = 'automatic' AND status = 'queued'
                   AND account_id = ?8 AND source = ?9 AND source_instance_key = ?10",
                params![
                    input.queued_at,
                    queue_seq,
                    input.settings_revision,
                    input.payload_json,
                    input.payload_sha256,
                    input.equivalence_sha256,
                    job_id,
                    input.task_key.account_id,
                    input.task_key.source.sqlite_name(),
                    input.task_key.source_instance_key,
                ],
            )
            .map_err(|error| error.to_string())?;
        if updated != 1 {
            return Err("sync_job_not_promotable".to_string());
        }
        refresh_linked_job_snapshots_and_batches_tx(transaction, job_id, &input.queued_at)?;
        load_stored_job(transaction, job_id)
    }

    pub fn claim_job(
        connection: &mut Connection,
        job_id: i64,
        started_at: &str,
    ) -> Result<StoredSyncJobV1, String> {
        let transaction = immediate_transaction(connection)?;
        let stored = Self::claim_job_tx(&transaction, job_id, started_at)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(stored)
    }

    pub fn claim_job_tx(
        transaction: &Transaction<'_>,
        job_id: i64,
        started_at: &str,
    ) -> Result<StoredSyncJobV1, String> {
        validate_timestamp(started_at)?;
        let running_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_jobs WHERE status = 'running')",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if running_exists {
            return Err("sync_already_running".to_string());
        }
        let updated = transaction
            .execute(
                "UPDATE sync_jobs
                 SET status = 'running', started_at = ?1
                 WHERE id = ?2 AND status = 'queued'",
                params![started_at, job_id],
            )
            .map_err(|error| error.to_string())?;
        if updated != 1 {
            return Err("sync_job_not_queued".to_string());
        }
        refresh_linked_job_snapshots_and_batches_tx(transaction, job_id, started_at)?;
        load_stored_job(transaction, job_id)
    }

    pub fn cancel_queued_job(
        connection: &mut Connection,
        job_id: i64,
        finished_at: &str,
        error_code: &str,
    ) -> Result<StoredSyncJobV1, String> {
        let transaction = immediate_transaction(connection)?;
        let stored = Self::cancel_queued_job_tx(&transaction, job_id, finished_at, error_code)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(stored)
    }

    pub fn cancel_all_queued_jobs(
        connection: &mut Connection,
        finished_at: &str,
        error_code: &str,
    ) -> Result<Vec<i64>, String> {
        validate_timestamp(finished_at)?;
        if error_code.trim().is_empty() {
            return Err("invalid_sync_error_code".to_string());
        }
        let transaction = immediate_transaction(connection)?;
        let job_ids = {
            let mut statement = transaction
                .prepare("SELECT id FROM sync_jobs WHERE status = 'queued' ORDER BY id")
                .map_err(|error| error.to_string())?;
            let ids = statement
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            ids
        };
        for job_id in &job_ids {
            Self::cancel_queued_job_tx(&transaction, *job_id, finished_at, error_code)?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(job_ids)
    }

    pub fn recover_for_uninstall(
        connection: &mut Connection,
        finished_at: &str,
        error_code: &str,
    ) -> Result<UninstallRecoveryResultV1, String> {
        validate_timestamp(finished_at)?;
        if error_code.trim().is_empty() {
            return Err("invalid_sync_error_code".to_string());
        }
        let transaction = immediate_transaction(connection)?;
        let running_job_ids = active_job_ids(&transaction, "running")?;
        let queued_job_ids = active_job_ids(&transaction, "queued")?;
        for job_id in &running_job_ids {
            Self::finish_running_job_tx(
                &transaction,
                *job_id,
                &TerminalJobUpdateV1 {
                    status: SyncJobStatusV1::Cancelled,
                    finished_at: finished_at.to_string(),
                    inserted_videos: 0,
                    inserted_records: 0,
                    error_code: Some(error_code.to_string()),
                    error_message: None,
                },
            )?;
        }
        for job_id in &queued_job_ids {
            Self::cancel_queued_job_tx(&transaction, *job_id, finished_at, error_code)?;
        }
        let active_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_jobs WHERE status IN ('queued', 'running'))",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if active_exists {
            return Err("sync_active_jobs_remain".to_string());
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(UninstallRecoveryResultV1 {
            running_job_ids,
            queued_job_ids,
        })
    }

    pub fn cancel_queued_job_tx(
        transaction: &Transaction<'_>,
        job_id: i64,
        finished_at: &str,
        error_code: &str,
    ) -> Result<StoredSyncJobV1, String> {
        validate_timestamp(finished_at)?;
        if error_code.trim().is_empty() {
            return Err("invalid_sync_error_code".to_string());
        }
        let updated = transaction
            .execute(
                "UPDATE sync_jobs
                 SET status = 'cancelled', finished_at = ?1, error_code = ?2
                 WHERE id = ?3 AND status = 'queued'",
                params![finished_at, error_code, job_id],
            )
            .map_err(|error| error.to_string())?;
        if updated != 1 {
            return Err("sync_job_not_queued".to_string());
        }
        refresh_linked_job_snapshots_and_batches_tx(transaction, job_id, finished_at)?;
        load_stored_job(transaction, job_id)
    }

    pub fn finish_running_job(
        connection: &mut Connection,
        job_id: i64,
        update: &TerminalJobUpdateV1,
    ) -> Result<StoredSyncJobV1, String> {
        let transaction = immediate_transaction(connection)?;
        let stored = Self::finish_running_job_tx(&transaction, job_id, update)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(stored)
    }

    pub fn finish_running_job_tx(
        transaction: &Transaction<'_>,
        job_id: i64,
        update: &TerminalJobUpdateV1,
    ) -> Result<StoredSyncJobV1, String> {
        validate_timestamp(&update.finished_at)?;
        if !is_terminal_job_status(update.status) {
            return Err("invalid_sync_terminal_status".to_string());
        }
        if update.inserted_videos < 0 || update.inserted_records < 0 {
            return Err("invalid_sync_import_counts".to_string());
        }
        let has_incomplete: bool = transaction
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM sync_job_issues
                   WHERE job_id = ?1 AND severity = 'incomplete'
                 )",
                [job_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let status = if has_incomplete && update.status == SyncJobStatusV1::Success {
            SyncJobStatusV1::Partial
        } else {
            update.status
        };
        let updated = transaction
            .execute(
                "UPDATE sync_jobs
                 SET status = ?1, finished_at = ?2,
                     inserted_videos = ?3, inserted_records = ?4,
                     error_code = ?5, error_message = ?6
                 WHERE id = ?7 AND status = 'running'",
                params![
                    job_status_name(status),
                    update.finished_at,
                    update.inserted_videos,
                    update.inserted_records,
                    update.error_code,
                    update.error_message,
                    job_id,
                ],
            )
            .map_err(|error| error.to_string())?;
        if updated != 1 {
            return Err("sync_job_not_running".to_string());
        }
        refresh_linked_job_snapshots_and_batches_tx(transaction, job_id, &update.finished_at)?;
        load_stored_job(transaction, job_id)
    }

    pub fn append_job_issue(
        connection: &mut Connection,
        job_id: i64,
        issue: &SyncJobIssueV1,
        created_at: &str,
    ) -> Result<(), String> {
        let transaction = immediate_transaction(connection)?;
        Self::append_job_issue_tx(&transaction, job_id, issue, created_at)?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn append_job_issue_tx(
        transaction: &Transaction<'_>,
        job_id: i64,
        issue: &SyncJobIssueV1,
        created_at: &str,
    ) -> Result<(), String> {
        validate_timestamp(created_at)?;
        if issue.phase.trim().is_empty() || issue.code.trim().is_empty() {
            return Err("invalid_sync_job_issue".to_string());
        }
        transaction
            .execute(
                "INSERT INTO sync_job_issues(job_id, phase, code, severity, message, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    job_id,
                    issue.phase,
                    issue.code,
                    issue_severity_name(issue.severity),
                    issue.message,
                    created_at,
                ],
            )
            .map_err(|error| error.to_string())?;
        refresh_linked_job_snapshots_and_batches_tx(transaction, job_id, created_at)
    }

    pub fn latest_terminal_anchor(
        connection: &Connection,
        task_key: &TaskKeyV1,
    ) -> Result<Option<TerminalAnchorV1>, String> {
        validate_task_key(task_key)?;
        connection
            .query_row(
                "SELECT id, status, started_at, finished_at
                 FROM sync_jobs
                 WHERE account_id = ?1 AND source = ?2 AND source_instance_key = ?3
                   AND status IN ('success', 'partial', 'failed', 'cancelled')
                   AND started_at IS NOT NULL
                 ORDER BY finished_at DESC, id DESC
                 LIMIT 1",
                params![
                    task_key.account_id,
                    task_key.source.sqlite_name(),
                    task_key.source_instance_key,
                ],
                |row| {
                    let status: String = row.get(1)?;
                    Ok((
                        row.get(0)?,
                        status,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
            .map(|(job_id, status, started_at, finished_at)| {
                Ok(TerminalAnchorV1 {
                    job_id,
                    status: parse_job_status(&status)?,
                    started_at,
                    finished_at,
                })
            })
            .transpose()
    }
}

fn immediate_transaction(connection: &mut Connection) -> Result<Transaction<'_>, String> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())
}

fn active_job_ids(connection: &Connection, status: &str) -> Result<Vec<i64>, String> {
    if !matches!(status, "queued" | "running") {
        return Err("invalid_active_sync_status".to_string());
    }
    let mut statement = connection
        .prepare("SELECT id FROM sync_jobs WHERE status = ?1 ORDER BY id")
        .map_err(|error| error.to_string())?;
    let ids = statement
        .query_map([status], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(ids)
}

fn validate_new_job(input: &NewSyncJobV1) -> Result<(), String> {
    validate_task_key(&input.task_key)?;
    validate_timestamp(&input.queued_at)?;
    if input.settings_revision < 0
        || input.payload_json.is_empty()
        || !valid_sha256(&input.payload_sha256)
        || !valid_sha256(&input.equivalence_sha256)
    {
        return Err("invalid_sync_job_snapshot".to_string());
    }
    match input.trigger {
        SyncTriggerV1::Manual => {
            if input.scheduled_for.is_some() || input.config_sha256.is_some() {
                return Err("invalid_manual_sync_job".to_string());
            }
        }
        SyncTriggerV1::Automatic => {
            let scheduled_for = input
                .scheduled_for
                .as_deref()
                .ok_or_else(|| "invalid_automatic_sync_job".to_string())?;
            validate_timestamp(scheduled_for)?;
            if !input.config_sha256.as_deref().is_some_and(valid_sha256) {
                return Err("invalid_automatic_sync_job".to_string());
            }
        }
    }
    Ok(())
}

fn validate_task_key(task_key: &TaskKeyV1) -> Result<(), String> {
    if task_key.account_id <= 0 {
        return Err("invalid_sync_task_key".to_string());
    }
    match task_key.source {
        SyncSourceV1::History | SyncSourceV1::Likes | SyncSourceV1::Favorites
            if task_key.source_instance_key.is_empty() =>
        {
            Ok(())
        }
        SyncSourceV1::RemoteFolder if !task_key.source_instance_key.trim().is_empty() => Ok(()),
        _ => Err("invalid_sync_task_key".to_string()),
    }
}

fn validate_timestamp(value: &str) -> Result<i64, String> {
    let parsed =
        DateTime::parse_from_rfc3339(value).map_err(|_| "invalid_sync_timestamp".to_string())?;
    Ok(parsed.with_timezone(&Utc).timestamp_millis())
}

fn queue_sort_key(job: &StoredSyncJobV1) -> Result<QueueSortKeyV1, String> {
    Ok(match job.snapshot.trigger {
        SyncTriggerV1::Manual => QueueSortKeyV1::Manual {
            queue_seq: job.queue_seq,
            job_id: job.snapshot.id,
        },
        SyncTriggerV1::Automatic => QueueSortKeyV1::Automatic {
            scheduled_for_ms: validate_timestamp(
                job.scheduled_for
                    .as_deref()
                    .ok_or_else(|| "invalid_automatic_sync_job".to_string())?,
            )?,
            account_id: job.snapshot.task_key.account_id,
            source: job.snapshot.task_key.source,
            source_instance_key: job.snapshot.task_key.source_instance_key.clone(),
            queue_seq: job.queue_seq,
            job_id: job.snapshot.id,
        },
    })
}

fn load_stored_job(connection: &Connection, job_id: i64) -> Result<StoredSyncJobV1, String> {
    connection
        .query_row(
            "SELECT id, account_id, source, source_instance_key, trigger, status,
                    payload_sha256, queued_at, queue_seq, scheduled_for, settings_revision,
                    config_sha256, payload_json, equivalence_sha256,
                    started_at, finished_at, inserted_videos, inserted_records,
                    error_code, error_message
             FROM sync_jobs WHERE id = ?1",
            [job_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, i64>(16)?,
                    row.get::<_, i64>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<String>>(19)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "sync_job_not_found".to_string())
        .and_then(
            |(
                id,
                account_id,
                source,
                source_instance_key,
                trigger,
                status,
                payload_sha256,
                queued_at,
                queue_seq,
                scheduled_for,
                settings_revision,
                config_sha256,
                payload_json,
                equivalence_sha256,
                started_at,
                finished_at,
                inserted_videos,
                inserted_records,
                error_code,
                error_message,
            )| {
                Ok(StoredSyncJobV1 {
                    snapshot: SyncJobSnapshotV1 {
                        id,
                        task_key: TaskKeyV1 {
                            account_id,
                            source: SyncSourceV1::from_sqlite(&source)?,
                            source_instance_key,
                        },
                        trigger: parse_trigger(&trigger)?,
                        status: parse_job_status(&status)?,
                        task_payload_sha256: payload_sha256
                            .ok_or_else(|| "invalid_sync_job_payload".to_string())?,
                    },
                    queued_at,
                    queue_seq: queue_seq
                        .ok_or_else(|| "invalid_sync_job_queue_sequence".to_string())?,
                    scheduled_for,
                    settings_revision: settings_revision
                        .ok_or_else(|| "invalid_sync_job_settings_revision".to_string())?,
                    config_sha256,
                    payload_json: payload_json
                        .ok_or_else(|| "invalid_sync_job_payload".to_string())?,
                    equivalence_sha256: equivalence_sha256
                        .ok_or_else(|| "invalid_sync_job_payload".to_string())?,
                    started_at,
                    finished_at,
                    inserted_videos,
                    inserted_records,
                    error_code,
                    error_message,
                })
            },
        )
}

fn map_job_insert_error(error: rusqlite::Error) -> String {
    let message = error.to_string();
    if message.contains("idx_sync_jobs_active_task_key")
        || message.contains("sync_jobs.account_id, sync_jobs.source, sync_jobs.source_instance_key")
    {
        "sync_task_already_active".to_string()
    } else {
        message
    }
}

fn is_terminal_job_status(status: SyncJobStatusV1) -> bool {
    matches!(
        status,
        SyncJobStatusV1::Success
            | SyncJobStatusV1::Partial
            | SyncJobStatusV1::Failed
            | SyncJobStatusV1::Cancelled
    )
}

fn trigger_name(trigger: SyncTriggerV1) -> &'static str {
    match trigger {
        SyncTriggerV1::Manual => "manual",
        SyncTriggerV1::Automatic => "automatic",
    }
}

fn parse_trigger(value: &str) -> Result<SyncTriggerV1, String> {
    match value {
        "manual" => Ok(SyncTriggerV1::Manual),
        "automatic" => Ok(SyncTriggerV1::Automatic),
        _ => Err("invalid_sync_trigger".to_string()),
    }
}

fn job_status_name(status: SyncJobStatusV1) -> &'static str {
    match status {
        SyncJobStatusV1::Queued => "queued",
        SyncJobStatusV1::Running => "running",
        SyncJobStatusV1::Success => "success",
        SyncJobStatusV1::Partial => "partial",
        SyncJobStatusV1::Failed => "failed",
        SyncJobStatusV1::Cancelled => "cancelled",
    }
}

fn parse_job_status(value: &str) -> Result<SyncJobStatusV1, String> {
    match value {
        "queued" => Ok(SyncJobStatusV1::Queued),
        "running" => Ok(SyncJobStatusV1::Running),
        "success" => Ok(SyncJobStatusV1::Success),
        "partial" => Ok(SyncJobStatusV1::Partial),
        "failed" => Ok(SyncJobStatusV1::Failed),
        "cancelled" => Ok(SyncJobStatusV1::Cancelled),
        _ => Err("invalid_sync_job_status".to_string()),
    }
}

fn issue_severity_name(severity: SyncJobIssueSeverityV1) -> &'static str {
    match severity {
        SyncJobIssueSeverityV1::Warning => "warning",
        SyncJobIssueSeverityV1::Incomplete => "incomplete",
    }
}

fn parse_issue_severity(value: &str) -> Result<SyncJobIssueSeverityV1, String> {
    match value {
        "warning" => Ok(SyncJobIssueSeverityV1::Warning),
        "incomplete" => Ok(SyncJobIssueSeverityV1::Incomplete),
        _ => Err("invalid_sync_job_issue_severity".to_string()),
    }
}

impl QueueStore {
    pub fn create_batch(
        connection: &mut Connection,
        scope: SyncScopeV1,
        created_at: &str,
    ) -> Result<i64, String> {
        let transaction = immediate_transaction(connection)?;
        let batch_id = Self::create_batch_tx(&transaction, scope, created_at)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(batch_id)
    }

    pub fn create_batch_tx(
        transaction: &Transaction<'_>,
        scope: SyncScopeV1,
        created_at: &str,
    ) -> Result<i64, String> {
        validate_timestamp(created_at)?;
        if scope == SyncScopeV1::One {
            return Err("invalid_sync_batch_scope".to_string());
        }
        transaction
            .execute(
                "INSERT INTO sync_batches(trigger, scope, created_at, status)
                 VALUES ('manual', ?1, ?2, 'queued')",
                params![scope_name(scope), created_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(transaction.last_insert_rowid())
    }

    pub fn attach_job_to_batch(
        connection: &mut Connection,
        batch_id: i64,
        job_id: i64,
        role: BatchJobRoleV1,
        changed_at: &str,
    ) -> Result<i64, String> {
        let transaction = immediate_transaction(connection)?;
        let job = load_stored_job(&transaction, job_id)?;
        let entry_seq = Self::reserve_batch_target_tx(
            &transaction,
            batch_id,
            &job.snapshot.task_key,
            BatchTargetKindV1::Task,
            "job",
        )?;
        Self::attach_reserved_job_tx(&transaction, batch_id, job_id, role, entry_seq)?;
        Self::recalculate_batch_status_tx(&transaction, batch_id, changed_at)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(entry_seq)
    }

    pub fn reserve_batch_target_tx(
        transaction: &Transaction<'_>,
        batch_id: i64,
        task_key: &TaskKeyV1,
        target_kind: BatchTargetKindV1,
        resolution_kind: &str,
    ) -> Result<i64, String> {
        validate_batch_target_key(task_key, target_kind)?;
        if !matches!(resolution_kind, "job" | "issue") {
            return Err("invalid_batch_resolution_kind".to_string());
        }
        transaction
            .execute(
                "INSERT INTO sync_batch_targets
                 (batch_id, account_id, source, source_instance_key, target_kind, resolution_kind)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    batch_id,
                    task_key.account_id,
                    task_key.source.sqlite_name(),
                    task_key.source_instance_key,
                    batch_target_kind_name(target_kind),
                    resolution_kind,
                ],
            )
            .map_err(map_batch_mutation_error)?;
        reserve_batch_entry_sequence_tx(transaction, batch_id)
    }

    pub fn attach_reserved_job_tx(
        transaction: &Transaction<'_>,
        batch_id: i64,
        job_id: i64,
        role: BatchJobRoleV1,
        entry_seq: i64,
    ) -> Result<(), String> {
        let stored = load_stored_job(transaction, job_id)?;
        if !matches!(
            stored.snapshot.status,
            SyncJobStatusV1::Queued | SyncJobStatusV1::Running
        ) {
            return Err("sync_batch_job_not_active".to_string());
        }
        let view = load_job_view_without_queue_position(transaction, job_id)?;
        let snapshot_json = serde_json::to_string(&view).map_err(|error| error.to_string())?;
        let identity_sha256 = job_identity_sha256(&stored.snapshot)?;
        transaction
            .execute(
                "INSERT INTO sync_batch_jobs
                 (batch_id, account_id, source, source_instance_key, job_id, entry_seq,
                  job_identity_sha256, job_view_snapshot_json, role)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    batch_id,
                    stored.snapshot.task_key.account_id,
                    stored.snapshot.task_key.source.sqlite_name(),
                    stored.snapshot.task_key.source_instance_key,
                    job_id,
                    entry_seq,
                    identity_sha256,
                    snapshot_json,
                    batch_job_role_name(role),
                ],
            )
            .map_err(map_batch_mutation_error)?;
        Ok(())
    }

    pub fn append_batch_issue(
        connection: &mut Connection,
        batch_id: i64,
        issue: &NewSyncBatchIssueV1,
        changed_at: &str,
    ) -> Result<i64, String> {
        let transaction = immediate_transaction(connection)?;
        let target_kind = if issue.target_kind_aggregate {
            BatchTargetKindV1::Aggregate
        } else {
            BatchTargetKindV1::Task
        };
        let entry_seq = Self::reserve_batch_target_tx(
            &transaction,
            batch_id,
            &issue.task_key,
            target_kind,
            "issue",
        )?;
        Self::append_reserved_batch_issue_tx(&transaction, batch_id, issue, entry_seq, changed_at)?;
        Self::recalculate_batch_status_tx(&transaction, batch_id, changed_at)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(entry_seq)
    }

    pub fn append_reserved_batch_issue_tx(
        transaction: &Transaction<'_>,
        batch_id: i64,
        issue: &NewSyncBatchIssueV1,
        entry_seq: i64,
        created_at: &str,
    ) -> Result<(), String> {
        validate_timestamp(created_at)?;
        let phase = batch_issue_phase_name(issue.phase);
        match &issue.kind {
            NewSyncBatchIssueKindV1::Skipped {
                reason,
                requested_payload_sha256,
            } => {
                if reason.trim().is_empty()
                    || requested_payload_sha256
                        .as_deref()
                        .is_some_and(|value| !valid_sha256(value))
                {
                    return Err("invalid_sync_batch_issue".to_string());
                }
                transaction
                    .execute(
                        "INSERT INTO sync_batch_issues
                         (batch_id, entry_seq, account_id, source, source_instance_key,
                          phase, kind, reason, requested_payload_sha256, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'skipped', ?7, ?8, ?9)",
                        params![
                            batch_id,
                            entry_seq,
                            issue.task_key.account_id,
                            issue.task_key.source.sqlite_name(),
                            issue.task_key.source_instance_key,
                            phase,
                            reason,
                            requested_payload_sha256,
                            created_at,
                        ],
                    )
                    .map_err(map_batch_mutation_error)?;
            }
            NewSyncBatchIssueKindV1::Conflict {
                reason,
                requested_payload_sha256,
                existing_payload_sha256,
                existing_job_id,
            } => {
                if reason.trim().is_empty()
                    || !valid_sha256(requested_payload_sha256)
                    || !valid_sha256(existing_payload_sha256)
                {
                    return Err("invalid_sync_batch_issue".to_string());
                }
                let existing = load_stored_job(transaction, *existing_job_id)?;
                if existing.snapshot.task_key != issue.task_key
                    || existing.snapshot.task_payload_sha256 != *existing_payload_sha256
                    || !matches!(
                        existing.snapshot.status,
                        SyncJobStatusV1::Queued | SyncJobStatusV1::Running
                    )
                {
                    return Err("sync_batch_conflict_job_identity_mismatch".to_string());
                }
                let snapshot = load_job_view(transaction, *existing_job_id)?;
                let snapshot_json =
                    serde_json::to_string(&snapshot).map_err(|error| error.to_string())?;
                transaction
                    .execute(
                        "INSERT INTO sync_batch_issues
                         (batch_id, entry_seq, account_id, source, source_instance_key,
                          phase, kind, reason, requested_payload_sha256,
                          existing_payload_sha256, existing_job_id,
                          conflicting_job_snapshot_json, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'conflict', ?7, ?8, ?9, ?10, ?11, ?12)",
                        params![
                            batch_id,
                            entry_seq,
                            issue.task_key.account_id,
                            issue.task_key.source.sqlite_name(),
                            issue.task_key.source_instance_key,
                            phase,
                            reason,
                            requested_payload_sha256,
                            existing_payload_sha256,
                            existing_job_id,
                            snapshot_json,
                            created_at,
                        ],
                    )
                    .map_err(map_batch_mutation_error)?;
            }
        }
        Ok(())
    }

    pub fn recalculate_batch_status(
        connection: &mut Connection,
        batch_id: i64,
        changed_at: &str,
    ) -> Result<SyncBatchViewV1, String> {
        let transaction = immediate_transaction(connection)?;
        let view = Self::recalculate_batch_status_tx(&transaction, batch_id, changed_at)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(view)
    }

    pub fn recalculate_batch_status_tx(
        transaction: &Transaction<'_>,
        batch_id: i64,
        changed_at: &str,
    ) -> Result<SyncBatchViewV1, String> {
        validate_timestamp(changed_at)?;
        let current = load_batch_row(transaction, batch_id)?;
        let states = load_batch_job_states(transaction, batch_id)?;
        let has_batch_issue: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_batch_issues WHERE batch_id = ?1)",
                [batch_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let status = aggregate_batch_status(&states, has_batch_issue, current.status);
        let finished_at = if is_terminal_batch_status(status) {
            current
                .finished_at
                .clone()
                .or_else(|| Some(changed_at.to_string()))
        } else {
            None
        };
        transaction
            .execute(
                "UPDATE sync_batches SET status = ?1, finished_at = ?2 WHERE id = ?3",
                params![batch_status_name(status), finished_at, batch_id],
            )
            .map_err(map_batch_mutation_error)?;
        load_batch_view(transaction, batch_id)
    }

    pub fn get_batch(connection: &Connection, batch_id: i64) -> Result<SyncBatchViewV1, String> {
        load_batch_view(connection, batch_id)
    }

    pub fn list_batches_for_account(
        connection: &Connection,
        account_id: i64,
        limit: usize,
        offset: usize,
    ) -> Result<SyncBatchesPageV1, String> {
        if account_id <= 0 {
            return Err("invalid_account_id".to_string());
        }
        if !(1..=200).contains(&limit) {
            return Err("invalid_sync_batch_page_limit".to_string());
        }
        let total: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sync_batches batches
                 WHERE EXISTS (
                   SELECT 1 FROM sync_batch_targets targets
                   WHERE targets.batch_id = batches.id AND targets.account_id = ?1
                 )",
                [account_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT batches.id FROM sync_batches batches
                 WHERE EXISTS (
                   SELECT 1 FROM sync_batch_targets targets
                   WHERE targets.batch_id = batches.id AND targets.account_id = ?1
                 )
                 ORDER BY batches.created_at DESC, batches.id DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .map_err(|error| error.to_string())?;
        let ids = statement
            .query_map(params![account_id, limit as i64, offset as i64], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let records = ids
            .into_iter()
            .map(|batch_id| load_batch_view(connection, batch_id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SyncBatchesPageV1 {
            records,
            total: u64::try_from(total).map_err(|_| "sync_batch_count_overflow".to_string())?,
        })
    }

    pub fn list_batch_entries(
        connection: &Connection,
        batch_id: i64,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<SyncBatchEntriesPageV1, String> {
        if !(1..=200).contains(&limit) {
            return Err("invalid_batch_page_limit".to_string());
        }
        let batch = load_batch_row(connection, batch_id)?;
        let last_entry_seq = match cursor {
            Some(value) => decode_batch_cursor(value, batch_id)?,
            None => 0,
        };
        if last_entry_seq < 0 || last_entry_seq >= batch.next_entry_seq {
            return Err("invalid_batch_cursor".to_string());
        }
        let mut statement = connection
            .prepare(
                "SELECT entry_seq, kind, row_id FROM (
                   SELECT entry_seq, 'job' AS kind, id AS row_id
                   FROM sync_batch_jobs
                   WHERE batch_id = ?1 AND entry_seq > ?2
                   UNION ALL
                   SELECT entry_seq, 'issue' AS kind, id AS row_id
                   FROM sync_batch_issues
                   WHERE batch_id = ?1 AND entry_seq > ?2
                 )
                 ORDER BY entry_seq ASC
                 LIMIT ?3",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(
                params![batch_id, last_entry_seq, (limit + 1) as i64],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let has_more = rows.len() > limit;
        let visible = rows.into_iter().take(limit).collect::<Vec<_>>();
        let queue_positions = load_queue_sort_entries(connection)?
            .into_iter()
            .enumerate()
            .map(|(position, (job_id, _))| (job_id, (position + 1) as u32))
            .collect::<HashMap<_, _>>();
        let mut entries = Vec::with_capacity(visible.len());
        let mut next_entry_seq = last_entry_seq;
        for (entry_seq, kind, row_id) in visible {
            next_entry_seq = entry_seq;
            entries.push(match kind.as_str() {
                "job" => load_batch_job_entry(connection, row_id, &queue_positions)?,
                "issue" => load_batch_issue_entry(connection, row_id)?,
                _ => return Err("invalid_sync_batch_entry".to_string()),
            });
        }
        Ok(SyncBatchEntriesPageV1 {
            batch_id,
            entries,
            next_cursor: encode_batch_cursor(batch_id, next_entry_seq)?,
            has_more,
        })
    }
}

#[derive(Debug, Clone)]
struct BatchRow {
    trigger: SyncTriggerV1,
    scope: SyncScopeV1,
    status: SyncBatchStatusV1,
    created_at: String,
    finished_at: Option<String>,
    next_entry_seq: i64,
}

#[derive(Debug, Clone, Copy)]
struct BatchJobState {
    status: SyncJobStatusV1,
    has_incomplete_issue: bool,
}

fn reserve_batch_entry_sequence_tx(
    transaction: &Transaction<'_>,
    batch_id: i64,
) -> Result<i64, String> {
    let next: i64 = transaction
        .query_row(
            "SELECT next_entry_seq FROM sync_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "sync_batch_not_found".to_string())?;
    if next == i64::MAX {
        return Err("sync_batch_entry_sequence_exhausted".to_string());
    }
    transaction
        .execute(
            "UPDATE sync_batches SET next_entry_seq = ?1 WHERE id = ?2",
            params![next + 1, batch_id],
        )
        .map_err(map_batch_mutation_error)?;
    Ok(next)
}

fn validate_batch_target_key(
    task_key: &TaskKeyV1,
    target_kind: BatchTargetKindV1,
) -> Result<(), String> {
    match target_kind {
        BatchTargetKindV1::Task => validate_task_key(task_key),
        BatchTargetKindV1::Aggregate
            if task_key.account_id > 0
                && task_key.source == SyncSourceV1::RemoteFolder
                && task_key.source_instance_key.is_empty() =>
        {
            Ok(())
        }
        BatchTargetKindV1::Aggregate => Err("invalid_sync_batch_target".to_string()),
    }
}

fn batch_target_kind_name(kind: BatchTargetKindV1) -> &'static str {
    match kind {
        BatchTargetKindV1::Task => "task",
        BatchTargetKindV1::Aggregate => "aggregate",
    }
}

fn batch_job_role_name(role: BatchJobRoleV1) -> &'static str {
    match role {
        BatchJobRoleV1::Root => "root",
        BatchJobRoleV1::DiscoveredChild => "discoveredChild",
        BatchJobRoleV1::Attached => "attached",
    }
}

fn scope_name(scope: SyncScopeV1) -> &'static str {
    match scope {
        SyncScopeV1::One => "one",
        SyncScopeV1::CurrentAccountAvailableSources => "currentAccountAvailableSources",
        SyncScopeV1::AllActiveAccounts => "allActiveAccounts",
    }
}

fn parse_scope(value: &str) -> Result<SyncScopeV1, String> {
    match value {
        "one" => Ok(SyncScopeV1::One),
        "currentAccountAvailableSources" => Ok(SyncScopeV1::CurrentAccountAvailableSources),
        "allActiveAccounts" => Ok(SyncScopeV1::AllActiveAccounts),
        _ => Err("invalid_sync_scope".to_string()),
    }
}

fn batch_issue_phase_name(phase: SyncBatchIssuePhaseV1) -> &'static str {
    match phase {
        SyncBatchIssuePhaseV1::InitialExpansion => "initialExpansion",
        SyncBatchIssuePhaseV1::DiscoveryExpansion => "discoveryExpansion",
    }
}

fn parse_batch_issue_phase(value: &str) -> Result<SyncBatchIssuePhaseV1, String> {
    match value {
        "initialExpansion" => Ok(SyncBatchIssuePhaseV1::InitialExpansion),
        "discoveryExpansion" => Ok(SyncBatchIssuePhaseV1::DiscoveryExpansion),
        _ => Err("invalid_sync_batch_issue_phase".to_string()),
    }
}

fn batch_status_name(status: SyncBatchStatusV1) -> &'static str {
    match status {
        SyncBatchStatusV1::Queued => "queued",
        SyncBatchStatusV1::Running => "running",
        SyncBatchStatusV1::Success => "success",
        SyncBatchStatusV1::Partial => "partial",
        SyncBatchStatusV1::Failed => "failed",
        SyncBatchStatusV1::Cancelled => "cancelled",
    }
}

fn parse_batch_status(value: &str) -> Result<SyncBatchStatusV1, String> {
    match value {
        "queued" => Ok(SyncBatchStatusV1::Queued),
        "running" => Ok(SyncBatchStatusV1::Running),
        "success" => Ok(SyncBatchStatusV1::Success),
        "partial" => Ok(SyncBatchStatusV1::Partial),
        "failed" => Ok(SyncBatchStatusV1::Failed),
        "cancelled" => Ok(SyncBatchStatusV1::Cancelled),
        _ => Err("invalid_sync_batch_status".to_string()),
    }
}

fn is_terminal_batch_status(status: SyncBatchStatusV1) -> bool {
    matches!(
        status,
        SyncBatchStatusV1::Success
            | SyncBatchStatusV1::Partial
            | SyncBatchStatusV1::Failed
            | SyncBatchStatusV1::Cancelled
    )
}

fn aggregate_batch_status(
    states: &[BatchJobState],
    has_batch_issue: bool,
    current: SyncBatchStatusV1,
) -> SyncBatchStatusV1 {
    if states
        .iter()
        .any(|state| state.status == SyncJobStatusV1::Running)
    {
        return SyncBatchStatusV1::Running;
    }
    if states
        .iter()
        .any(|state| state.status == SyncJobStatusV1::Queued)
    {
        return SyncBatchStatusV1::Queued;
    }
    if states.is_empty() && !has_batch_issue {
        return current;
    }
    let has_incomplete = has_batch_issue || states.iter().any(|state| state.has_incomplete_issue);
    let all_success = !states.is_empty()
        && states
            .iter()
            .all(|state| state.status == SyncJobStatusV1::Success);
    let all_cancelled = !states.is_empty()
        && states
            .iter()
            .all(|state| state.status == SyncJobStatusV1::Cancelled);
    if all_success && !has_incomplete {
        return SyncBatchStatusV1::Success;
    }
    if all_cancelled && !has_incomplete {
        return SyncBatchStatusV1::Cancelled;
    }
    let has_success_or_partial = states.iter().any(|state| {
        matches!(
            state.status,
            SyncJobStatusV1::Success | SyncJobStatusV1::Partial
        )
    });
    let has_degraded = has_incomplete
        || states.iter().any(|state| {
            matches!(
                state.status,
                SyncJobStatusV1::Partial | SyncJobStatusV1::Failed | SyncJobStatusV1::Cancelled
            )
        });
    if has_success_or_partial && has_degraded {
        SyncBatchStatusV1::Partial
    } else {
        SyncBatchStatusV1::Failed
    }
}

fn load_job_issues(connection: &Connection, job_id: i64) -> Result<Vec<SyncJobIssueV1>, String> {
    let mut statement = connection
        .prepare(
            "SELECT phase, code, severity, message
             FROM sync_job_issues WHERE job_id = ?1 ORDER BY id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([job_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|(phase, code, severity, message)| {
            Ok(SyncJobIssueV1 {
                phase,
                code,
                severity: parse_issue_severity(&severity)?,
                message,
            })
        })
        .collect()
}

fn load_job_view(connection: &Connection, job_id: i64) -> Result<SyncJobViewV1, String> {
    let stored = load_stored_job(connection, job_id)?;
    let queue_position = if stored.snapshot.status == SyncJobStatusV1::Queued {
        load_queue_sort_entries(connection)?
            .iter()
            .position(|(id, _)| *id == job_id)
            .map(|position| (position + 1) as u32)
    } else {
        None
    };
    build_job_view(connection, stored, queue_position)
}

fn load_job_view_without_queue_position(
    connection: &Connection,
    job_id: i64,
) -> Result<SyncJobViewV1, String> {
    let stored = load_stored_job(connection, job_id)?;
    build_job_view(connection, stored, None)
}

fn load_job_view_with_queue_position(
    connection: &Connection,
    job_id: i64,
    queue_position: Option<u32>,
) -> Result<SyncJobViewV1, String> {
    let stored = load_stored_job(connection, job_id)?;
    let queue_position = if stored.snapshot.status == SyncJobStatusV1::Queued {
        queue_position
    } else {
        None
    };
    build_job_view(connection, stored, queue_position)
}

fn build_job_view(
    connection: &Connection,
    stored: StoredSyncJobV1,
    queue_position: Option<u32>,
) -> Result<SyncJobViewV1, String> {
    let job_id = stored.snapshot.id;
    Ok(SyncJobViewV1 {
        id: stored.snapshot.id,
        account_id: stored.snapshot.task_key.account_id,
        source: stored.snapshot.task_key.source,
        source_instance_key: stored.snapshot.task_key.source_instance_key,
        trigger: stored.snapshot.trigger,
        status: stored.snapshot.status,
        queued_at: stored.queued_at,
        scheduled_for: stored.scheduled_for,
        started_at: stored.started_at,
        finished_at: stored.finished_at,
        queue_position,
        can_cancel: matches!(
            stored.snapshot.status,
            SyncJobStatusV1::Queued | SyncJobStatusV1::Running
        ),
        issues: load_job_issues(connection, job_id)?,
    })
}

fn load_queue_sort_entries(connection: &Connection) -> Result<Vec<(i64, QueueSortKeyV1)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, account_id, source, source_instance_key, trigger,
                    scheduled_for, queue_seq
             FROM sync_jobs WHERE status = 'queued'",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut entries = rows
        .into_iter()
        .map(
            |(id, account_id, source, source_instance_key, trigger, scheduled_for, queue_seq)| {
                let source = SyncSourceV1::from_sqlite(&source)?;
                let trigger = parse_trigger(&trigger)?;
                let queue_seq =
                    queue_seq.ok_or_else(|| "invalid_sync_job_queue_sequence".to_string())?;
                let key = match trigger {
                    SyncTriggerV1::Manual => QueueSortKeyV1::Manual {
                        queue_seq,
                        job_id: id,
                    },
                    SyncTriggerV1::Automatic => QueueSortKeyV1::Automatic {
                        scheduled_for_ms: validate_timestamp(
                            scheduled_for
                                .as_deref()
                                .ok_or_else(|| "invalid_automatic_sync_job".to_string())?,
                        )?,
                        account_id,
                        source,
                        source_instance_key,
                        queue_seq,
                        job_id: id,
                    },
                };
                Ok((id, key))
            },
        )
        .collect::<Result<Vec<_>, String>>()?;
    entries.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(entries)
}

fn refresh_linked_job_snapshots_and_batches_tx(
    transaction: &Transaction<'_>,
    job_id: i64,
    changed_at: &str,
) -> Result<(), String> {
    let view = load_job_view(transaction, job_id)?;
    let snapshot_json = serde_json::to_string(&view).map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE sync_batch_jobs SET job_view_snapshot_json = ?1 WHERE job_id = ?2",
            params![snapshot_json, job_id],
        )
        .map_err(|error| error.to_string())?;
    let mut statement = transaction
        .prepare(
            "SELECT DISTINCT batch_id FROM sync_batch_jobs WHERE job_id = ?1 ORDER BY batch_id",
        )
        .map_err(|error| error.to_string())?;
    let batch_ids = statement
        .query_map([job_id], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    for batch_id in batch_ids {
        QueueStore::recalculate_batch_status_tx(transaction, batch_id, changed_at)?;
    }
    Ok(())
}

fn load_batch_row(connection: &Connection, batch_id: i64) -> Result<BatchRow, String> {
    connection
        .query_row(
            "SELECT trigger, scope, status, created_at, finished_at, next_entry_seq
             FROM sync_batches WHERE id = ?1",
            [batch_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "sync_batch_not_found".to_string())
        .and_then(
            |(trigger, scope, status, created_at, finished_at, next_entry_seq)| {
                Ok(BatchRow {
                    trigger: parse_trigger(&trigger)?,
                    scope: parse_scope(&scope)?,
                    status: parse_batch_status(&status)?,
                    created_at,
                    finished_at,
                    next_entry_seq,
                })
            },
        )
}

fn load_batch_view(connection: &Connection, batch_id: i64) -> Result<SyncBatchViewV1, String> {
    let row = load_batch_row(connection, batch_id)?;
    let jobs: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sync_batch_jobs WHERE batch_id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let skipped: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sync_batch_issues WHERE batch_id = ?1 AND kind = 'skipped'",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let conflicts: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sync_batch_issues WHERE batch_id = ?1 AND kind = 'conflict'",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(SyncBatchViewV1 {
        id: batch_id,
        trigger: row.trigger,
        scope: row.scope,
        status: row.status,
        created_at: row.created_at,
        finished_at: row.finished_at,
        counts: SyncBatchCountsV1 {
            jobs: u32::try_from(jobs).map_err(|_| "sync_batch_count_overflow".to_string())?,
            skipped: u32::try_from(skipped).map_err(|_| "sync_batch_count_overflow".to_string())?,
            conflicts: u32::try_from(conflicts)
                .map_err(|_| "sync_batch_count_overflow".to_string())?,
        },
    })
}

fn load_batch_job_states(
    connection: &Connection,
    batch_id: i64,
) -> Result<Vec<BatchJobState>, String> {
    let mut statement = connection
        .prepare(
            "SELECT links.job_id, links.job_view_snapshot_json, jobs.status,
                    EXISTS(
                      SELECT 1 FROM sync_job_issues issues
                      WHERE issues.job_id = links.job_id AND issues.severity = 'incomplete'
                    )
             FROM sync_batch_jobs links
             LEFT JOIN sync_jobs jobs ON jobs.id = links.job_id
             WHERE links.batch_id = ?1 ORDER BY links.entry_seq",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([batch_id], |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|(job_id, snapshot_json, live_status, live_incomplete)| {
            if job_id.is_some() {
                return Ok(BatchJobState {
                    status: parse_job_status(
                        live_status
                            .as_deref()
                            .ok_or_else(|| "invalid_sync_job_snapshot".to_string())?,
                    )?,
                    has_incomplete_issue: live_incomplete,
                });
            }
            let view = serde_json::from_str::<SyncJobViewV1>(&snapshot_json)
                .map_err(|_| "invalid_sync_job_snapshot".to_string())?;
            Ok(BatchJobState {
                status: view.status,
                has_incomplete_issue: view
                    .issues
                    .iter()
                    .any(|issue| issue.severity == SyncJobIssueSeverityV1::Incomplete),
            })
        })
        .collect()
}

fn load_batch_job_entry(
    connection: &Connection,
    association_id: i64,
    queue_positions: &HashMap<i64, u32>,
) -> Result<SyncBatchEntryV1, String> {
    let (job_id, snapshot_json) = connection
        .query_row(
            "SELECT job_id, job_view_snapshot_json FROM sync_batch_jobs WHERE id = ?1",
            [association_id],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "invalid_sync_batch_entry".to_string())?;
    let view = match job_id {
        Some(job_id) => load_job_view_with_queue_position(
            connection,
            job_id,
            queue_positions.get(&job_id).copied(),
        )?,
        None => serde_json::from_str(&snapshot_json)
            .map_err(|_| "invalid_sync_job_snapshot".to_string())?,
    };
    Ok(SyncBatchEntryV1::Job(view))
}

fn load_batch_issue_entry(
    connection: &Connection,
    issue_id: i64,
) -> Result<SyncBatchEntryV1, String> {
    let row = connection
        .query_row(
            "SELECT account_id, source, source_instance_key, phase, kind, reason,
                    requested_payload_sha256, existing_payload_sha256,
                    conflicting_job_snapshot_json
             FROM sync_batch_issues WHERE id = ?1",
            [issue_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "invalid_sync_batch_entry".to_string())?;
    let (
        account_id,
        source,
        source_instance_key,
        phase,
        kind,
        reason,
        requested,
        existing,
        snapshot,
    ) = row;
    let source = SyncSourceV1::from_sqlite(&source)?;
    let phase = parse_batch_issue_phase(&phase)?;
    match kind.as_str() {
        "skipped" => Ok(SyncBatchEntryV1::Skipped(SyncSkipV1 {
            account_id,
            source,
            source_instance_key,
            phase,
            reason,
            requested_payload_sha256: requested,
        })),
        "conflict" => Ok(SyncBatchEntryV1::Conflict(SyncConflictV1 {
            account_id,
            source,
            source_instance_key,
            phase,
            reason,
            requested_payload_sha256: requested
                .ok_or_else(|| "invalid_sync_batch_issue".to_string())?,
            existing_payload_sha256: existing
                .ok_or_else(|| "invalid_sync_batch_issue".to_string())?,
            existing_job_at_conflict: serde_json::from_str(
                snapshot
                    .as_deref()
                    .ok_or_else(|| "invalid_sync_batch_issue".to_string())?,
            )
            .map_err(|_| "invalid_sync_job_snapshot".to_string())?,
        })),
        _ => Err("invalid_sync_batch_issue".to_string()),
    }
}

fn map_batch_mutation_error(error: rusqlite::Error) -> String {
    let message = error.to_string();
    for stable in [
        "sync_batch_terminal",
        "sync_batch_target_immutable",
        "sync_batch_target_delete_forbidden",
        "sync_batch_job_required",
        "sync_batch_job_task_key_mismatch",
        "sync_batch_job_task_key_immutable",
        "sync_batch_job_identity_immutable",
        "sync_batch_job_rebind_forbidden",
        "sync_batch_job_manual_unlink_forbidden",
        "sync_batch_job_delete_forbidden",
        "sync_batch_conflict_job_identity_mismatch",
        "sync_batch_issue_evidence_immutable",
        "sync_batch_issue_job_rebind_forbidden",
        "sync_batch_issue_manual_unlink_forbidden",
        "sync_batch_issue_delete_forbidden",
    ] {
        if message.contains(stable) {
            return stable.to_string();
        }
    }
    message
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchCursorBodyV1 {
    schema_version: u32,
    batch_id: i64,
    last_entry_seq: i64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BatchCursorTokenV1 {
    schema_version: u32,
    batch_id: i64,
    last_entry_seq: i64,
    checksum: String,
}

fn encode_batch_cursor(batch_id: i64, last_entry_seq: i64) -> Result<String, String> {
    let body = BatchCursorBodyV1 {
        schema_version: CURSOR_SCHEMA_VERSION,
        batch_id,
        last_entry_seq,
    };
    let body_json = serde_json::to_string(&body).map_err(|error| error.to_string())?;
    let checksum = cursor_checksum(&body_json);
    let token = BatchCursorTokenV1 {
        schema_version: body.schema_version,
        batch_id,
        last_entry_seq,
        checksum,
    };
    let json = serde_json::to_vec(&token).map_err(|error| error.to_string())?;
    Ok(base64url_encode(&json))
}

fn decode_batch_cursor(value: &str, expected_batch_id: i64) -> Result<i64, String> {
    let decoded = base64url_decode(value).map_err(|_| "invalid_batch_cursor".to_string())?;
    if base64url_encode(&decoded) != value {
        return Err("invalid_batch_cursor".to_string());
    }
    let token: BatchCursorTokenV1 =
        serde_json::from_slice(&decoded).map_err(|_| "invalid_batch_cursor".to_string())?;
    if token.schema_version != CURSOR_SCHEMA_VERSION || token.batch_id != expected_batch_id {
        return Err("invalid_batch_cursor".to_string());
    }
    let body = BatchCursorBodyV1 {
        schema_version: token.schema_version,
        batch_id: token.batch_id,
        last_entry_seq: token.last_entry_seq,
    };
    let body_json = serde_json::to_string(&body).map_err(|_| "invalid_batch_cursor".to_string())?;
    if token.checksum != cursor_checksum(&body_json) {
        return Err("invalid_batch_cursor".to_string());
    }
    Ok(token.last_entry_seq)
}

fn cursor_checksum(body_json: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(CURSOR_DOMAIN);
    digest.update(body_json.as_bytes());
    format!("{:x}", digest.finalize())
}

fn base64url_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity((bytes.len() * 4 + 2) / 3);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
        }
        if chunk.len() > 2 {
            output.push(TABLE[(third & 0x3f) as usize] as char);
        }
    }
    output
}

fn base64url_decode(value: &str) -> Result<Vec<u8>, ()> {
    if value.is_empty() || value.len() % 4 == 1 {
        return Err(());
    }
    let values = value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' => Ok(byte - b'A'),
            b'a'..=b'z' => Ok(byte - b'a' + 26),
            b'0'..=b'9' => Ok(byte - b'0' + 52),
            b'-' => Ok(62),
            b'_' => Ok(63),
            _ => Err(()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Vec::with_capacity(values.len() * 3 / 4);
    for chunk in values.chunks(4) {
        if chunk.len() < 2 {
            return Err(());
        }
        output.push((chunk[0] << 2) | (chunk[1] >> 4));
        if chunk.len() > 2 {
            output.push((chunk[1] << 4) | (chunk[2] >> 2));
        }
        if chunk.len() > 3 {
            output.push((chunk[2] << 6) | chunk[3]);
        }
    }
    Ok(output)
}
