use crate::{initialize_schema_v4, AppPaths};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{collections::BTreeSet, fs, path::Path};

const SCHEMA_VERSION_V4: i64 = 4;
const SCHEMA_VERSION_V5: i64 = 5;
const SCHEMA_VERSION_V6: i64 = 6;
const SCHEMA_VERSION_V7: i64 = 7;
const SCHEMA_VERSION_V8: i64 = 8;
const MAX_SQLITE_SEQUENCE: i64 = i64::MAX;
const MIGRATION_V5: &str = include_str!("../migrations/005_sync_queue.sql");
const MIGRATION_V6: &str = include_str!("../migrations/006_cover_cache.sql");
const MIGRATION_V7: &str = include_str!("../migrations/007_download_jobs.sql");
const MIGRATION_V8: &str = include_str!("../migrations/008_notifications.sql");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRunningJobIdentity {
    pub account_id: i64,
    pub job_id: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaV5RecoveryWork {
    pub interrupted_v4_jobs: Vec<LegacyRunningJobIdentity>,
}

pub fn migrate_schema_to_latest_at_startup(
    connection: &mut Connection,
) -> Result<SchemaV5RecoveryWork, String> {
    let version = user_version(connection)?;
    let work = match version {
        SCHEMA_VERSION_V6 | SCHEMA_VERSION_V7 | SCHEMA_VERSION_V8 => {
            SchemaV5RecoveryWork::default()
        }
        _ => migrate_schema_to_v5_at_startup(connection)?,
    };
    if user_version(connection)? == SCHEMA_VERSION_V5 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch(MIGRATION_V6)
            .map_err(map_migration_error)?;
        assert_schema_v6_layout(&transaction)?;
        transaction.commit().map_err(|error| error.to_string())?;
    }
    if user_version(connection)? == SCHEMA_VERSION_V6 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch(MIGRATION_V7)
            .map_err(map_migration_error)?;
        assert_schema_v7_layout(&transaction)?;
        transaction
            .pragma_update(None, "user_version", SCHEMA_VERSION_V7)
            .map_err(|error| error.to_string())?;
        if user_version(&transaction)? != SCHEMA_VERSION_V7 {
            return Err("schema_v7_user_version_failed".to_string());
        }
        transaction.commit().map_err(|error| error.to_string())?;
    }
    if user_version(connection)? == SCHEMA_VERSION_V7 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch(MIGRATION_V8)
            .map_err(map_migration_error)?;
        assert_schema_v8_layout(&transaction)?;
        transaction
            .pragma_update(None, "user_version", SCHEMA_VERSION_V8)
            .map_err(|error| error.to_string())?;
        if user_version(&transaction)? != SCHEMA_VERSION_V8 {
            return Err("schema_v8_user_version_failed".to_string());
        }
        transaction.commit().map_err(|error| error.to_string())?;
    }
    assert_schema_latest(connection)?;
    Ok(work)
}

pub fn assert_schema_latest(connection: &Connection) -> Result<(), String> {
    if user_version(connection)? != SCHEMA_VERSION_V8 {
        return Err("schema_not_initialized".to_string());
    }
    assert_schema_v5_layout(connection)?;
    assert_schema_v6_layout(connection)?;
    assert_schema_v7_layout(connection)?;
    assert_schema_v8_layout(connection)
}

pub fn migrate_schema_to_v5_at_startup(
    connection: &mut Connection,
) -> Result<SchemaV5RecoveryWork, String> {
    let version = user_version(connection)?;
    match version {
        SCHEMA_VERSION_V5 => {
            assert_schema_v5(connection)?;
            Ok(SchemaV5RecoveryWork::default())
        }
        0..=3 => {
            initialize_schema_v4(connection)?;
            migrate_schema_v4_to_v5(connection)
        }
        SCHEMA_VERSION_V4 => migrate_schema_v4_to_v5(connection),
        _ => Err(format!("unsupported_schema_version:{version}")),
    }
}

pub fn assert_schema_v5(connection: &Connection) -> Result<(), String> {
    if !matches!(
        user_version(connection)?,
        SCHEMA_VERSION_V5 | SCHEMA_VERSION_V6 | SCHEMA_VERSION_V7 | SCHEMA_VERSION_V8
    ) {
        return Err("schema_not_initialized".to_string());
    }
    assert_schema_v5_layout(connection)
}

pub fn rebuild_schema_v5_cleanup_work(
    connection: &Connection,
    paths: &AppPaths,
) -> Result<SchemaV5RecoveryWork, String> {
    assert_schema_v5(connection)?;
    let mut statement = connection
        .prepare(
            "SELECT account_id, id
             FROM sync_jobs
             WHERE error_code = 'schema_upgrade_interrupted_running'
             ORDER BY id ASC",
        )
        .map_err(|error| error.to_string())?;
    let identities = statement
        .query_map([], |row| {
            Ok(LegacyRunningJobIdentity {
                account_id: row.get(0)?,
                job_id: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|identity| cleanup_evidence_exists(paths, identity))
        .collect();
    Ok(SchemaV5RecoveryWork {
        interrupted_v4_jobs: identities,
    })
}

pub fn consume_schema_v5_recovery_work<F>(
    work: &SchemaV5RecoveryWork,
    mut cleanup: F,
) -> Result<(), String>
where
    F: FnMut(&LegacyRunningJobIdentity) -> Result<(), String>,
{
    for identity in &work.interrupted_v4_jobs {
        cleanup(identity)?;
    }
    Ok(())
}

fn migrate_schema_v4_to_v5(connection: &mut Connection) -> Result<SchemaV5RecoveryWork, String> {
    let interrupted_v4_jobs = load_running_v4_jobs(connection)?;
    let original_sequence = sqlite_sequence(connection, "sync_jobs")?;
    if original_sequence == Some(MAX_SQLITE_SEQUENCE) {
        return Err("sync_job_sequence_exhausted".to_string());
    }
    let original_foreign_keys = foreign_keys_enabled(connection)?;
    set_foreign_keys(connection, false)?;

    let migration = (|| {
        let frozen_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch(
                "CREATE TEMP TABLE emily_schema_v5_context (frozen_at TEXT NOT NULL);
                 CREATE TEMP TABLE emily_schema_v5_sequence (seq INTEGER NOT NULL);",
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO temp.emily_schema_v5_context(frozen_at) VALUES (?1)",
                [&frozen_at],
            )
            .map_err(|error| error.to_string())?;
        if let Some(sequence) = original_sequence {
            transaction
                .execute(
                    "INSERT INTO temp.emily_schema_v5_sequence(seq) VALUES (?1)",
                    [sequence],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction
            .execute_batch(MIGRATION_V5)
            .map_err(map_migration_error)?;
        assert_schema_v5_layout(&transaction)?;
        transaction
            .pragma_update(None, "user_version", SCHEMA_VERSION_V5)
            .map_err(|error| error.to_string())?;
        if user_version(&transaction)? != SCHEMA_VERSION_V5 {
            return Err("schema_v5_user_version_failed".to_string());
        }
        if sqlite_sequence(&transaction, "sync_jobs")? != original_sequence {
            return Err("schema_v5_sequence_mismatch".to_string());
        }
        transaction.commit().map_err(|error| error.to_string())
    })();

    let restore = set_foreign_keys(connection, original_foreign_keys)
        .and_then(|_| verify_foreign_keys_state(connection, original_foreign_keys));
    match (migration, restore) {
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(restore_error)) => Err(format!("{error};{restore_error}")),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => {
            assert_schema_v5(connection)?;
            Ok(SchemaV5RecoveryWork {
                interrupted_v4_jobs,
            })
        }
    }
}

fn load_running_v4_jobs(connection: &Connection) -> Result<Vec<LegacyRunningJobIdentity>, String> {
    let mut statement = connection
        .prepare(
            "SELECT account_id, id
             FROM sync_jobs
             WHERE status = 'running'
             ORDER BY id ASC",
        )
        .map_err(|error| error.to_string())?;
    let jobs = statement
        .query_map([], |row| {
            Ok(LegacyRunningJobIdentity {
                account_id: row.get(0)?,
                job_id: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(jobs)
}

fn assert_schema_v5_layout(connection: &Connection) -> Result<(), String> {
    for table in [
        "accounts",
        "videos",
        "sync_jobs",
        "watch_records",
        "user_likes",
        "favorite_items",
        "remote_folders",
        "remote_folder_items",
        "local_folders",
        "local_folder_items",
        "local_folder_item_accounts",
        "sync_batches",
        "sync_batch_targets",
        "sync_queue_state",
        "remote_folder_registry_state",
        "sync_batch_jobs",
        "sync_job_issues",
        "sync_batch_issues",
    ] {
        assert_schema_object(connection, "table", table)?;
    }
    for index in [
        "idx_sync_jobs_account_status",
        "idx_sync_jobs_account_source",
        "idx_sync_jobs_active_task_key",
        "idx_sync_jobs_single_running",
        "idx_sync_jobs_queue",
        "idx_sync_jobs_terminal_task_key",
        "idx_remote_folders_scheduler",
        "idx_sync_batch_jobs_job",
        "idx_sync_batch_issues_existing_job",
    ] {
        assert_schema_object(connection, "index", index)?;
    }
    for trigger in [
        "reject_terminal_batch_job",
        "reject_terminal_batch_target",
        "reject_batch_target_update",
        "reject_direct_batch_target_delete",
        "reject_sync_job_task_key_update",
        "reject_initial_null_batch_job",
        "reject_batch_job_task_key_mismatch",
        "reject_batch_job_task_key_update",
        "reject_batch_job_identity_update",
        "reject_batch_job_rebind",
        "reject_batch_job_manual_unlink",
        "reject_direct_batch_job_delete",
        "reject_terminal_batch_issue",
        "reject_conflict_issue_job_identity",
        "reject_batch_issue_evidence_update",
        "reject_batch_issue_job_rebind",
        "reject_batch_issue_manual_unlink",
        "reject_direct_batch_issue_delete",
        "reject_terminal_batch_reopen",
    ] {
        assert_schema_object(connection, "trigger", trigger)?;
    }

    assert_columns(
        connection,
        "sync_jobs",
        &[
            "id",
            "account_id",
            "source",
            "source_instance_key",
            "trigger",
            "queued_at",
            "queue_seq",
            "scheduled_for",
            "settings_revision",
            "config_sha256",
            "payload_json",
            "payload_sha256",
            "equivalence_sha256",
            "started_at",
            "finished_at",
            "status",
            "inserted_videos",
            "inserted_records",
            "error_code",
            "error_message",
        ],
    )?;
    assert_columns(
        connection,
        "sync_batch_targets",
        &[
            "batch_id",
            "account_id",
            "source",
            "source_instance_key",
            "target_kind",
            "resolution_kind",
        ],
    )?;
    assert_columns(
        connection,
        "sync_batch_jobs",
        &[
            "id",
            "batch_id",
            "account_id",
            "source",
            "source_instance_key",
            "resolution_kind",
            "job_id",
            "entry_seq",
            "job_identity_sha256",
            "job_view_snapshot_json",
            "role",
        ],
    )?;

    let sync_jobs_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'sync_jobs'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if sync_jobs_sql.contains("sync_jobs_v5_new") {
        return Err("schema_v5_temporary_sync_jobs_reference".to_string());
    }
    assert_foreign_key_targets(connection)?;
    assert_foreign_key_check(connection)?;

    let queue_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sync_queue_state
             WHERE singleton = 1 AND next_queue_seq >= 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if queue_rows != 1 {
        return Err("schema_v5_invalid_queue_state".to_string());
    }
    let account_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let registry_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM remote_folder_registry_state",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if account_count != registry_count {
        return Err("schema_v5_registry_state_mismatch".to_string());
    }
    Ok(())
}

fn assert_schema_v6_layout(connection: &Connection) -> Result<(), String> {
    for table in [
        "cover_cache",
        "cover_cache_tasks",
        "cover_cache_rules",
        "cover_cache_queue",
    ] {
        assert_schema_object(connection, "table", table)?;
    }
    for index in [
        "idx_cover_cache_lru",
        "idx_cover_cache_single_active_task",
        "idx_cover_cache_queue_order",
    ] {
        assert_schema_object(connection, "index", index)?;
    }
    assert_foreign_key_check(connection)
}

fn assert_schema_v7_layout(connection: &Connection) -> Result<(), String> {
    for table in ["download_jobs", "download_items", "download_assets"] {
        assert_schema_object(connection, "table", table)?;
    }
    for index in [
        "idx_download_jobs_active",
        "idx_download_items_job_state",
        "idx_download_assets_item_state",
        "idx_download_assets_historical_reuse",
    ] {
        assert_schema_object(connection, "index", index)?;
    }
    assert_columns(
        connection,
        "download_jobs",
        &[
            "id",
            "account_id",
            "profile_key",
            "created_at",
            "started_at",
            "completed_at",
            "selection_scope",
            "filter_summary",
            "destination_root",
            "requested_quality",
            "state",
            "candidate_count",
            "resolved_count",
            "completed_count",
            "skipped_count",
            "failed_count",
            "total_bytes",
            "downloaded_bytes",
            "error_code",
        ],
    )?;
    let account_target = connection
        .query_row(
            "SELECT \"table\" FROM pragma_foreign_key_list('download_jobs') WHERE \"from\" = 'account_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if account_target.as_deref() != Some("accounts") {
        return Err("schema_v7_missing_download_account_foreign_key".to_string());
    }
    assert_foreign_key_check(connection)
}

fn assert_schema_v8_layout(connection: &Connection) -> Result<(), String> {
    assert_schema_object(connection, "table", "notifications")?;
    for index in [
        "idx_notifications_unresolved_dedupe",
        "idx_notifications_created",
        "idx_notifications_unread",
    ] {
        assert_schema_object(connection, "index", index)?;
    }
    assert_columns(
        connection,
        "notifications",
        &[
            "id",
            "kind",
            "severity",
            "dedupe_key",
            "title",
            "body",
            "account_id",
            "related_type",
            "related_id",
            "created_at",
            "read_at",
            "resolved_at",
        ],
    )?;
    let account_target = connection
        .query_row(
            "SELECT \"table\" FROM pragma_foreign_key_list('notifications') WHERE \"from\" = 'account_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if account_target.as_deref() != Some("accounts") {
        return Err("schema_v8_missing_notification_account_foreign_key".to_string());
    }
    assert_foreign_key_check(connection)
}

fn assert_schema_object(
    connection: &Connection,
    object_type: &str,
    name: &str,
) -> Result<(), String> {
    let exists: i64 = connection
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2
             )",
            params![object_type, name],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if exists != 1 {
        return Err(format!("schema_v5_missing_{object_type}:{name}"));
    }
    Ok(())
}

fn assert_columns(connection: &Connection, table: &str, expected: &[&str]) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| error.to_string())?;
    let actual = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if actual != expected {
        return Err(format!("schema_v5_column_mismatch:{table}"));
    }
    Ok(())
}

fn assert_foreign_key_targets(connection: &Connection) -> Result<(), String> {
    let tables = [
        "accounts",
        "videos",
        "sync_jobs",
        "watch_records",
        "user_likes",
        "favorite_items",
        "remote_folders",
        "remote_folder_items",
        "local_folders",
        "local_folder_items",
        "local_folder_item_accounts",
        "sync_batches",
        "sync_batch_targets",
        "sync_queue_state",
        "remote_folder_registry_state",
        "sync_batch_jobs",
        "sync_job_issues",
        "sync_batch_issues",
    ];
    let mut targets = BTreeSet::new();
    for table in tables {
        let mut statement = connection
            .prepare(&format!("PRAGMA foreign_key_list({table})"))
            .map_err(|error| error.to_string())?;
        for target in statement
            .query_map([], |row| row.get::<_, String>(2))
            .map_err(|error| error.to_string())?
        {
            let target = target.map_err(|error| error.to_string())?;
            if target.contains("_v5_new") {
                return Err(format!("schema_v5_temporary_foreign_key:{table}"));
            }
            targets.insert((table.to_string(), target));
        }
    }
    for expected in [
        ("watch_records".to_string(), "sync_jobs".to_string()),
        (
            "sync_batch_jobs".to_string(),
            "sync_batch_targets".to_string(),
        ),
        (
            "sync_batch_issues".to_string(),
            "sync_batch_targets".to_string(),
        ),
        ("sync_job_issues".to_string(), "sync_jobs".to_string()),
    ] {
        if !targets.contains(&expected) {
            return Err(format!(
                "schema_v5_missing_foreign_key:{}:{}",
                expected.0, expected.1
            ));
        }
    }
    Ok(())
}

fn assert_foreign_key_check(connection: &Connection) -> Result<(), String> {
    let violation = connection
        .query_row(
            "SELECT \"table\" FROM pragma_foreign_key_check LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if violation.is_some() {
        return Err("foreign_key_check_failed".to_string());
    }
    Ok(())
}

fn user_version(connection: &Connection) -> Result<i64, String> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())
}

fn sqlite_sequence(connection: &Connection, table: &str) -> Result<Option<i64>, String> {
    connection
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn foreign_keys_enabled(connection: &Connection) -> Result<bool, String> {
    connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
        .map(|value| value == 1)
        .map_err(|error| error.to_string())
}

fn set_foreign_keys(connection: &Connection, enabled: bool) -> Result<(), String> {
    connection
        .pragma_update(None, "foreign_keys", enabled)
        .map_err(|error| error.to_string())
}

fn verify_foreign_keys_state(connection: &Connection, expected: bool) -> Result<(), String> {
    if foreign_keys_enabled(connection)? != expected {
        return Err("foreign_keys_state_restore_failed".to_string());
    }
    Ok(())
}

fn map_migration_error(error: rusqlite::Error) -> String {
    match error {
        rusqlite::Error::SqliteFailure(_, Some(message)) => message,
        other => other.to_string(),
    }
}

fn cleanup_evidence_exists(paths: &AppPaths, identity: &LegacyRunningJobIdentity) -> bool {
    let account_id = identity.account_id;
    let job_id = identity.job_id;
    if [
        paths.result_path(account_id, job_id),
        paths.progress_path(account_id, job_id),
        paths.cancel_path(account_id, job_id),
        paths.browser_pid_path(account_id, job_id),
        paths.diagnostic_path(account_id, job_id),
        paths.log_path(account_id, job_id),
        paths.orchestration_path(account_id, job_id),
    ]
    .iter()
    .any(|path| path.is_file())
    {
        return true;
    }
    directory_has_regular_prefix(&paths.temp, &format!("account-{account_id}-sync-{job_id}-"))
        || directory_has_regular_prefix(
            &paths.diagnostics.join(format!("account-{account_id}")),
            &format!("sync-{job_id}-"),
        )
        || directory_has_regular_prefix(
            &paths.logs.join(format!("account-{account_id}")),
            &format!("sync-{job_id}-"),
        )
}

fn directory_has_regular_prefix(directory: &Path, prefix: &str) -> bool {
    fs::read_dir(directory).ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_type()
                .ok()
                .is_some_and(|file_type| file_type.is_file())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(prefix))
        })
    })
}
