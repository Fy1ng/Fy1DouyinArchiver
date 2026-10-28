use douyin_history_archive::{
    assert_schema_v5, consume_schema_v5_recovery_work, initialize_schema_v2, initialize_schema_v3,
    initialize_schema_v4, migrate_schema_to_v5_at_startup, rebuild_schema_v5_cleanup_work,
    AppPaths,
};
use rusqlite::{params, Connection, Error};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const MIGRATION_V5: &str = include_str!("../migrations/005_sync_queue.sql");

fn user_version(connection: &Connection) -> i64 {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user version")
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("table count")
}

fn foreign_keys_enabled(connection: &Connection) -> bool {
    connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
        .expect("foreign keys")
        == 1
}

fn foreign_key_errors(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("foreign key check")
}

fn sequence(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = ?1",
            [table],
            |row| row.get(0),
        )
        .expect("sqlite sequence")
}

fn table_exists(connection: &Connection, table: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .expect("table existence")
        == 1
}

fn sqlite_error_message(error: Error) -> String {
    match error {
        Error::SqliteFailure(_, Some(message)) => message,
        other => other.to_string(),
    }
}

fn assert_sql_error(connection: &Connection, sql: &str, expected: &str) {
    assert_eq!(
        sqlite_error_message(connection.execute_batch(sql).unwrap_err()),
        expected
    );
}

fn create_v1_schema(connection: &Connection) {
    connection
        .execute_batch(
            "
            CREATE TABLE videos (
              id INTEGER PRIMARY KEY,
              aweme_id TEXT UNIQUE,
              canonical_url TEXT UNIQUE,
              title TEXT,
              author_name TEXT,
              cover_url TEXT,
              like_count_text TEXT,
              first_seen_at TEXT,
              last_seen_at TEXT
            );
            CREATE TABLE sync_jobs (
              id INTEGER PRIMARY KEY,
              started_at TEXT NOT NULL,
              finished_at TEXT,
              source_file TEXT,
              status TEXT NOT NULL,
              inserted_videos INTEGER NOT NULL DEFAULT 0,
              inserted_watch_records INTEGER NOT NULL DEFAULT 0,
              error_message TEXT
            );
            CREATE TABLE watch_records (
              id INTEGER PRIMARY KEY,
              video_id INTEGER,
              page_group_label TEXT,
              collected_at TEXT,
              source TEXT,
              source_url TEXT,
              capture_order INTEGER,
              sync_job_id INTEGER,
              created_at TEXT,
              UNIQUE(video_id, page_group_label)
            );
            PRAGMA user_version = 1;
            ",
        )
        .expect("v1 schema");
}

fn insert_account(connection: &Connection, id: i64) {
    connection
        .execute(
            "INSERT INTO accounts
             (id, uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'active', 'created', 'updated')",
            params![
                id,
                format!("uid-{id}"),
                format!("sec-{id}"),
                format!("{id:032x}")
            ],
        )
        .expect("insert account");
}

fn seed_v4_graph(connection: &Connection) {
    insert_account(connection, 7);
    connection
        .execute_batch(
            "
            INSERT INTO videos
              (id, aweme_id, canonical_url, content_type, first_seen_at, last_seen_at)
            VALUES (11, '123', 'https://www.douyin.com/video/123', 'video', 'first', 'last');

            INSERT INTO sync_jobs
              (id, account_id, source, started_at, finished_at, status,
               inserted_videos, inserted_records)
            VALUES (41, 7, 'history', 'terminal-start', NULL, 'success', 1, 1);

            INSERT INTO sync_jobs
              (id, account_id, source, started_at, status)
            VALUES (42, 7, 'history', 'running-start', 'running');

            INSERT INTO watch_records
              (id, account_id, video_id, history_time_ms, watched_at, watched_date,
               source_position, sync_job_id, created_at, updated_at)
            VALUES (13, 7, 11, 1000, 'watched', '2026-08-11', 0, 41, 'created', 'updated');

            INSERT INTO user_likes
              (id, account_id, video_id, first_seen_at, last_seen_at)
            VALUES (17, 7, 11, 'first', 'last');

            INSERT INTO favorite_items
              (id, account_id, video_id, first_seen_at, last_seen_at)
            VALUES (19, 7, 11, 'first', 'last');

            INSERT INTO remote_folders
              (id, account_id, remote_folder_id, name, first_seen_at, last_seen_at)
            VALUES (23, 7, 'folder-a', 'Folder A', 'first', 'last');

            INSERT INTO remote_folder_items
              (folder_id, video_id, first_seen_at, last_seen_at)
            VALUES (23, 11, 'first', 'last');

            INSERT INTO local_folders
              (id, scope, owner_account_id, name, created_at, updated_at)
            VALUES (29, 'aggregate', NULL, 'Local', 'created', 'updated');

            INSERT INTO local_folder_items (folder_id, video_id, added_at)
            VALUES (29, 11, 'added');

            INSERT INTO local_folder_item_accounts
              (folder_id, video_id, account_id, added_at)
            VALUES (29, 11, 7, 'added');
            ",
        )
        .expect("seed v4 graph");
}

fn create_v5_fixture() -> Connection {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v4(&mut connection).expect("v4 schema");
    insert_account(&connection, 7);
    migrate_schema_to_v5_at_startup(&mut connection).expect("v5 migration");
    connection
}

fn temp_paths(label: &str) -> AppPaths {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-storage-v5-{label}-{}-{nonce}",
        std::process::id()
    ));
    let paths = AppPaths::new(root.clone(), PathBuf::from("resources"));
    for directory in [&paths.temp, &paths.diagnostics, &paths.logs] {
        fs::create_dir_all(directory).expect("task directory");
    }
    paths
}

fn insert_active_job(
    connection: &Connection,
    id: i64,
    source: &str,
    source_instance_key: &str,
    status: &str,
) {
    connection
        .execute(
            "INSERT INTO sync_jobs
             (id, account_id, source, source_instance_key, trigger, queued_at, queue_seq,
              settings_revision, payload_json, payload_sha256, equivalence_sha256,
              started_at, status)
             VALUES (?1, 7, ?2, ?3, 'manual', 'queued', ?1, 1, '{}', ?4, ?4,
                     CASE WHEN ?5 = 'running' THEN 'started' ELSE NULL END, ?5)",
            params![id, source, source_instance_key, "a".repeat(64), status],
        )
        .expect("insert active job");
}

#[test]
fn v0_v1_and_v2_use_the_approved_reset_then_reach_empty_schema_v5() {
    for source_version in 0..=2 {
        let mut connection = Connection::open_in_memory().expect("open db");
        match source_version {
            0 => {}
            1 => create_v1_schema(&connection),
            2 => {
                initialize_schema_v2(&mut connection).expect("v2 schema");
                connection
                    .execute_batch(
                        "
                        INSERT INTO videos
                          (id, aweme_id, canonical_url, content_type, first_seen_at, last_seen_at)
                        VALUES (1, 'old', 'https://www.douyin.com/video/old', 'video', 'x', 'x');
                        INSERT INTO sync_jobs (id, started_at, status)
                        VALUES (1, 'x', 'running');
                        INSERT INTO watch_records
                          (video_id, history_time_ms, watched_at, watched_date, source_position,
                           sync_job_id, created_at, updated_at)
                        VALUES (1, 1, 'x', 'x', 0, 1, 'x', 'x');
                        ",
                    )
                    .expect("seed v2 data");
            }
            _ => unreachable!(),
        }

        let work = migrate_schema_to_v5_at_startup(&mut connection).expect("v5 migration");
        assert!(work.interrupted_v4_jobs.is_empty());
        assert_eq!(user_version(&connection), 5);
        assert_eq!(count(&connection, "accounts"), 0);
        assert_eq!(count(&connection, "videos"), 0);
        assert_eq!(count(&connection, "watch_records"), 0);
        assert_eq!(count(&connection, "sync_jobs"), 0);
        assert_eq!(count(&connection, "remote_folder_registry_state"), 0);
        assert_eq!(foreign_key_errors(&connection), 0);
        assert!(foreign_keys_enabled(&connection));
        assert_schema_v5(&connection).expect("valid v5");
    }
}

#[test]
fn v3_and_v4_preserve_the_complete_graph_job_ids_sequences_and_recovery_work() {
    for source_version in [3, 4] {
        let mut connection = Connection::open_in_memory().expect("open db");
        initialize_schema_v3(&mut connection).expect("v3 schema");
        if source_version == 4 {
            initialize_schema_v4(&mut connection).expect("v4 schema");
        }
        seed_v4_graph(&connection);

        let work = migrate_schema_to_v5_at_startup(&mut connection).expect("v5 migration");

        assert_eq!(user_version(&connection), 5);
        assert!(foreign_keys_enabled(&connection));
        assert_eq!(foreign_key_errors(&connection), 0);
        assert_eq!(
            work.interrupted_v4_jobs
                .iter()
                .map(|job| (job.account_id, job.job_id))
                .collect::<Vec<_>>(),
            vec![(7, 42)]
        );
        for table in [
            "accounts",
            "videos",
            "watch_records",
            "user_likes",
            "favorite_items",
            "remote_folders",
            "remote_folder_items",
            "local_folders",
            "local_folder_items",
            "local_folder_item_accounts",
        ] {
            assert_eq!(count(&connection, table), 1, "lost row from {table}");
        }
        assert_eq!(count(&connection, "sync_jobs"), 2);
        assert_eq!(sequence(&connection, "sync_jobs"), 42);
        assert_eq!(
            connection
                .query_row(
                    "SELECT sync_job_id FROM watch_records WHERE id = 13",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("watch job reference"),
            41
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT trigger, queued_at, queue_seq, scheduled_for, settings_revision,
                            config_sha256, payload_json, payload_sha256, equivalence_sha256,
                            status, finished_at
                     FROM sync_jobs WHERE id = 41",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<i64>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, Option<String>>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, String>(9)?,
                            row.get::<_, Option<String>>(10)?,
                        ))
                    },
                )
                .expect("terminal job"),
            (
                "manual".to_string(),
                "terminal-start".to_string(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                "success".to_string(),
                Some("terminal-start".to_string()),
            )
        );
        let running = connection
            .query_row(
                "SELECT status, finished_at, error_code, queue_seq, payload_json
                 FROM sync_jobs WHERE id = 42",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .expect("interrupted job");
        assert_eq!(running.0, "failed");
        assert!(running.1.is_some());
        assert_eq!(
            running.2.as_deref(),
            Some("schema_upgrade_interrupted_running")
        );
        assert_eq!(running.3, None);
        assert_eq!(running.4, None);
        assert_eq!(
            connection
                .query_row(
                    "SELECT revision FROM remote_folder_registry_state WHERE account_id = 7",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("registry state"),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT next_queue_seq FROM sync_queue_state WHERE singleton = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("queue state"),
            1
        );
        assert_schema_v5(&connection).expect("valid v5");
    }

    assert!(!MIGRATION_V5.lines().any(|line| matches!(
        line.trim(),
        "BEGIN;" | "BEGIN TRANSACTION;" | "BEGIN IMMEDIATE;" | "COMMIT;"
    )));
    assert!(!MIGRATION_V5.contains("PRAGMA foreign_keys"));
    assert!(!MIGRATION_V5.contains("ALTER TABLE sync_jobs RENAME"));
    assert!(MIGRATION_V5.contains("DROP TABLE sync_jobs"));
    assert!(MIGRATION_V5.contains("ALTER TABLE sync_jobs_v5_new RENAME TO sync_jobs"));
}

#[test]
fn v4_failures_rollback_exactly_restore_foreign_keys_and_can_retry() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v4(&mut connection).expect("v4 schema");
    insert_account(&connection, 7);
    connection
        .execute_batch(
            "
            PRAGMA ignore_check_constraints = ON;
            INSERT INTO sync_jobs
              (id, account_id, source, started_at, status)
            VALUES (9, 7, 'invalid', 'started', 'running');
            PRAGMA ignore_check_constraints = OFF;
            ",
        )
        .expect("inject invalid source");

    let error = migrate_schema_to_v5_at_startup(&mut connection).unwrap_err();
    assert!(error.contains("CHECK constraint failed"), "{error}");
    assert_eq!(user_version(&connection), 4);
    assert!(foreign_keys_enabled(&connection));
    assert!(!table_exists(&connection, "sync_batches"));
    assert_eq!(
        connection
            .query_row("SELECT source FROM sync_jobs WHERE id = 9", [], |row| {
                row.get::<_, String>(0)
            })
            .expect("preserved invalid row"),
        "invalid"
    );

    connection
        .execute("UPDATE sync_jobs SET source = 'history' WHERE id = 9", [])
        .expect("repair source");
    migrate_schema_to_v5_at_startup(&mut connection).expect("retry migration");
    assert_eq!(user_version(&connection), 5);
}

#[test]
fn a_late_v5_failure_after_v3_commits_leaves_a_retryable_v4_database() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    insert_account(&connection, 7);
    connection
        .execute_batch(
            "
            PRAGMA ignore_check_constraints = ON;
            INSERT INTO sync_jobs
              (id, account_id, source, started_at, status)
            VALUES (9, 7, 'invalid', 'started', 'failed');
            PRAGMA ignore_check_constraints = OFF;
            ",
        )
        .expect("inject invalid v3 source");

    assert!(migrate_schema_to_v5_at_startup(&mut connection).is_err());
    assert_eq!(user_version(&connection), 4);
    assert!(!table_exists(&connection, "sync_batches"));
    assert!(foreign_keys_enabled(&connection));
    connection
        .execute("UPDATE sync_jobs SET source = 'history' WHERE id = 9", [])
        .expect("repair source");
    migrate_schema_to_v5_at_startup(&mut connection).expect("retry migration");
    assert_eq!(user_version(&connection), 5);
}

#[test]
fn exhausted_legacy_job_sequence_is_rejected_without_mutating_v4() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v4(&mut connection).expect("v4 schema");
    insert_account(&connection, 7);
    connection
        .execute(
            "INSERT INTO sync_jobs
             (id, account_id, source, started_at, finished_at, status)
             VALUES (1, 7, 'history', 'started', 'finished', 'success')",
            [],
        )
        .expect("insert job");
    connection
        .execute(
            "UPDATE sqlite_sequence SET seq = 9223372036854775807 WHERE name = 'sync_jobs'",
            [],
        )
        .expect("exhaust sequence");

    assert_eq!(
        migrate_schema_to_v5_at_startup(&mut connection).unwrap_err(),
        "sync_job_sequence_exhausted"
    );
    assert_eq!(user_version(&connection), 4);
    assert_eq!(sequence(&connection, "sync_jobs"), i64::MAX);
    assert!(!table_exists(&connection, "sync_batches"));
    assert!(foreign_keys_enabled(&connection));
}

#[test]
fn schema_assertion_never_migrates_and_detects_missing_v5_objects() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v4(&mut connection).expect("v4 schema");
    assert_eq!(
        assert_schema_v5(&connection).unwrap_err(),
        "schema_not_initialized"
    );
    assert_eq!(user_version(&connection), 4);

    migrate_schema_to_v5_at_startup(&mut connection).expect("v5 migration");
    connection
        .execute_batch("DROP TRIGGER reject_terminal_batch_reopen")
        .expect("damage trigger");
    assert_eq!(
        assert_schema_v5(&connection).unwrap_err(),
        "schema_v5_missing_trigger:reject_terminal_batch_reopen"
    );
}

#[test]
fn task_keys_single_running_targets_and_batch_entries_enforce_exact_invariants() {
    let connection = create_v5_fixture();
    insert_active_job(&connection, 1, "history", "", "running");
    assert!(connection
        .execute(
            "INSERT INTO sync_jobs
             (id, account_id, source, source_instance_key, trigger, queued_at, queue_seq,
              settings_revision, payload_json, payload_sha256, equivalence_sha256,
              started_at, status)
             VALUES (2, 7, 'likes', '', 'manual', 'queued', 2, 1, '{}', ?1, ?1,
                     'started', 'running')",
            ["b".repeat(64)],
        )
        .is_err());
    assert_sql_error(
        &connection,
        "UPDATE sync_jobs SET source = 'likes' WHERE id = 1",
        "sync_job_task_key_immutable",
    );

    connection
        .execute_batch(
            "
            INSERT INTO sync_batches
              (id, trigger, scope, created_at, status)
            VALUES (10, 'manual', 'currentAccountAvailableSources', 'created', 'running');
            INSERT INTO sync_batch_targets
              (batch_id, account_id, source, source_instance_key, target_kind, resolution_kind)
            VALUES (10, 7, 'history', '', 'task', 'job');
            INSERT INTO sync_batch_jobs
              (id, batch_id, account_id, source, source_instance_key, job_id, entry_seq,
               job_identity_sha256, job_view_snapshot_json, role)
            VALUES (20, 10, 7, 'history', '', 1, 1, 'identity', '{}', 'root');
            ",
        )
        .expect("seed batch job");

    assert_sql_error(
        &connection,
        "UPDATE sync_batch_targets SET target_kind = 'aggregate' WHERE batch_id = 10",
        "sync_batch_target_immutable",
    );
    assert_sql_error(
        &connection,
        "DELETE FROM sync_batch_targets WHERE batch_id = 10",
        "sync_batch_target_delete_forbidden",
    );
    assert_sql_error(
        &connection,
        "INSERT INTO sync_batch_jobs
         (batch_id, account_id, source, source_instance_key, job_id, entry_seq,
          job_identity_sha256, job_view_snapshot_json, role)
         VALUES (10, 7, 'history', '', NULL, 2, 'null-job', '{}', 'root')",
        "sync_batch_job_required",
    );
    assert_sql_error(
        &connection,
        "UPDATE sync_batch_jobs SET source = 'likes' WHERE id = 20",
        "sync_batch_job_task_key_immutable",
    );
    assert_sql_error(
        &connection,
        "UPDATE sync_batch_jobs SET entry_seq = 2 WHERE id = 20",
        "sync_batch_job_identity_immutable",
    );
    assert_sql_error(
        &connection,
        "UPDATE sync_batch_jobs SET job_id = 1 WHERE id = 20",
        "sync_batch_job_rebind_forbidden",
    );
    assert_sql_error(
        &connection,
        "UPDATE sync_batch_jobs SET job_id = NULL WHERE id = 20",
        "sync_batch_job_manual_unlink_forbidden",
    );
    assert_sql_error(
        &connection,
        "DELETE FROM sync_batch_jobs WHERE id = 20",
        "sync_batch_job_delete_forbidden",
    );

    connection
        .execute("DELETE FROM sync_jobs WHERE id = 1", [])
        .expect("parent job delete");
    assert_eq!(
        connection
            .query_row(
                "SELECT job_id FROM sync_batch_jobs WHERE id = 20",
                [],
                |row| { row.get::<_, Option<i64>>(0) }
            )
            .expect("retained association"),
        None
    );
    connection
        .execute("DELETE FROM sync_batches WHERE id = 10", [])
        .expect("batch cascade");
    assert_eq!(count(&connection, "sync_batch_targets"), 0);
    assert_eq!(count(&connection, "sync_batch_jobs"), 0);
}

#[test]
fn issue_evidence_terminal_append_and_composite_foreign_keys_are_immutable() {
    let connection = create_v5_fixture();
    insert_active_job(&connection, 1, "favorites", "", "queued");
    connection
        .execute_batch(
            "
            INSERT INTO sync_batches
              (id, trigger, scope, created_at, status)
            VALUES (10, 'manual', 'allActiveAccounts', 'created', 'queued');
            INSERT INTO sync_batch_targets
              (batch_id, account_id, source, source_instance_key, target_kind, resolution_kind)
            VALUES (10, 7, 'favorites', '', 'task', 'issue');
            INSERT INTO sync_batch_issues
              (id, batch_id, entry_seq, account_id, source, source_instance_key, phase,
               kind, reason, requested_payload_sha256, existing_payload_sha256,
               existing_job_id, conflicting_job_snapshot_json, created_at)
            VALUES (30, 10, 1, 7, 'favorites', '', 'initialExpansion', 'conflict',
                    'conflict', 'requested', 'existing', 1, '{}', 'created');
            ",
        )
        .expect("seed conflict issue");

    assert_sql_error(
        &connection,
        "UPDATE sync_batch_issues SET reason = 'changed' WHERE id = 30",
        "sync_batch_issue_evidence_immutable",
    );
    assert_sql_error(
        &connection,
        "UPDATE sync_batch_issues SET existing_job_id = 1 WHERE id = 30",
        "sync_batch_issue_job_rebind_forbidden",
    );
    assert_sql_error(
        &connection,
        "UPDATE sync_batch_issues SET existing_job_id = NULL WHERE id = 30",
        "sync_batch_issue_manual_unlink_forbidden",
    );
    assert_sql_error(
        &connection,
        "DELETE FROM sync_batch_issues WHERE id = 30",
        "sync_batch_issue_delete_forbidden",
    );

    connection
        .execute("DELETE FROM sync_jobs WHERE id = 1", [])
        .expect("delete conflict job");
    assert_eq!(
        connection
            .query_row(
                "SELECT existing_job_id FROM sync_batch_issues WHERE id = 30",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .expect("retained conflict"),
        None
    );

    connection
        .execute(
            "UPDATE sync_batches SET status = 'failed', finished_at = 'finished' WHERE id = 10",
            [],
        )
        .expect("finish batch");
    assert_sql_error(
        &connection,
        "INSERT INTO sync_batch_targets
         (batch_id, account_id, source, source_instance_key, target_kind, resolution_kind)
         VALUES (10, 7, 'history', '', 'task', 'issue')",
        "sync_batch_terminal",
    );
    assert_sql_error(
        &connection,
        "UPDATE sync_batches SET status = 'running', finished_at = NULL WHERE id = 10",
        "sync_batch_terminal",
    );

    let mut isolated = create_v5_fixture();
    insert_active_job(&isolated, 2, "history", "", "queued");
    isolated
        .execute(
            "INSERT INTO sync_batches
             (id, trigger, scope, created_at, status)
             VALUES (11, 'manual', 'currentAccountAvailableSources', 'created', 'queued')",
            [],
        )
        .expect("insert batch");
    let transaction = isolated.transaction().expect("transaction");
    transaction
        .execute_batch("PRAGMA defer_foreign_keys = ON")
        .expect("defer foreign keys");
    transaction
        .execute(
            "INSERT INTO sync_batch_jobs
             (batch_id, account_id, source, source_instance_key, job_id, entry_seq,
              job_identity_sha256, job_view_snapshot_json, role)
             VALUES (11, 7, 'history', '', 2, 1, 'identity', '{}', 'root')",
            [],
        )
        .expect("deferred child insert");
    assert!(transaction.commit().is_err());
}

#[test]
fn interrupted_upgrade_cleanup_is_rebuilt_until_identity_files_are_removed() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v4(&mut connection).expect("v4 schema");
    insert_account(&connection, 7);
    connection
        .execute(
            "INSERT INTO sync_jobs
             (id, account_id, source, started_at, status)
             VALUES (42, 7, 'history', 'started', 'running')",
            [],
        )
        .expect("running job");
    let immediate = migrate_schema_to_v5_at_startup(&mut connection).expect("v5 migration");
    assert_eq!(immediate.interrupted_v4_jobs.len(), 1);

    let paths = temp_paths("recovery");
    let evidence = paths.progress_path(7, 42);
    fs::write(&evidence, b"progress").expect("recovery evidence");
    let rebuilt = rebuild_schema_v5_cleanup_work(&connection, &paths).expect("rebuild work");
    assert_eq!(rebuilt, immediate);

    assert_eq!(
        consume_schema_v5_recovery_work(&rebuilt, |_| Err("cleanup_failed".to_string()))
            .unwrap_err(),
        "cleanup_failed"
    );
    assert!(evidence.is_file());
    assert_eq!(
        rebuild_schema_v5_cleanup_work(&connection, &paths)
            .expect("retry work")
            .interrupted_v4_jobs
            .len(),
        1
    );

    consume_schema_v5_recovery_work(&rebuilt, |_| {
        fs::remove_file(&evidence).map_err(|error| error.to_string())
    })
    .expect("cleanup succeeds");
    assert!(rebuild_schema_v5_cleanup_work(&connection, &paths)
        .expect("empty work")
        .interrupted_v4_jobs
        .is_empty());
    let _ = fs::remove_dir_all(paths.data_root);
}
