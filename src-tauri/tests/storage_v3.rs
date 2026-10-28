use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use douyin_history_archive::{
    compute_collector_hashes, delete_history_by_ids, delete_history_by_query,
    initialize_schema_v2, initialize_schema_v3, initialize_schema_v4, list_history,
    validate_and_import_result,
    CollectorRange, CollectorRecordV2, CollectorSuccessV3, SyncOptions,
};
use rusqlite::{params, Connection};

const MIGRATION_V3: &str = include_str!("../migrations/003_multi_account.sql");
const MIGRATION_V4: &str = include_str!("../migrations/004_identity_conflict.sql");

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

fn index_exists(connection: &Connection, index: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
            [index],
            |row| row.get::<_, i64>(0),
        )
        .expect("index existence")
        == 1
}

fn insert_account(connection: &Connection, uid: &str, sec_uid: &str, profile_key: &str) -> i64 {
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'active', 'now', 'now')",
            params![uid, sec_uid, profile_key],
        )
        .expect("insert account");
    connection.last_insert_rowid()
}

fn collector_record(id: &str, history_time_ms: i64) -> CollectorRecordV2 {
    let watched_utc = DateTime::<Utc>::from_timestamp_millis(history_time_ms).expect("time");
    let shanghai = FixedOffset::east_opt(8 * 60 * 60).expect("offset");
    CollectorRecordV2 {
        aweme_id: id.to_string(),
        aweme_type: Some(0),
        canonical_url: format!("https://www.douyin.com/video/{id}"),
        content_type: "video".to_string(),
        history_time_ms,
        watched_at: watched_utc.to_rfc3339_opts(SecondsFormat::Millis, true),
        watched_date: watched_utc
            .with_timezone(&shanghai)
            .format("%Y-%m-%d")
            .to_string(),
        source_position: 0,
        title: Some("shared title".to_string()),
        author_name: Some("shared author".to_string()),
        cover_url: None,
        like_count: Some(100),
        watch_progress_rate: None,
    }
}

fn collector_result(
    account_id: i64,
    sync_job_id: i64,
    records: Vec<CollectorRecordV2>,
) -> CollectorSuccessV3 {
    let (identity_hash, field_hash) = compute_collector_hashes(&records).expect("hash");
    CollectorSuccessV3 {
        schema_version: 3,
        account_id,
        sync_job_id,
        sync_started_at_ms: 1_783_742_400_000,
        calendar_time_zone: "Asia/Shanghai".to_string(),
        range: CollectorRange::Bottom,
        detected_history_time_unit: "milliseconds".to_string(),
        identity_hash,
        field_hash,
        records,
    }
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

#[test]
fn migration_rebuilds_v2_as_empty_v3_and_enables_foreign_keys() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v2(&mut connection).expect("v2 schema");
    connection
        .execute_batch(
            "
            INSERT INTO videos
              (id, aweme_id, canonical_url, content_type, first_seen_at, last_seen_at)
            VALUES (1, 'old', 'https://www.douyin.com/video/old', 'video', 'x', 'x');
            INSERT INTO sync_jobs (id, started_at, status) VALUES (1, 'x', 'running');
            INSERT INTO watch_records
              (video_id, history_time_ms, watched_at, watched_date, source_position, sync_job_id, created_at, updated_at)
            VALUES (1, 1, 'x', 'x', 0, 1, 'x', 'x');
            ",
        )
        .expect("v2 data");

    initialize_schema_v3(&mut connection).expect("v3 migration");

    assert_eq!(user_version(&connection), 3);
    assert_eq!(count(&connection, "accounts"), 0);
    assert_eq!(count(&connection, "videos"), 0);
    assert_eq!(count(&connection, "watch_records"), 0);
    assert_eq!(count(&connection, "sync_jobs"), 0);
    assert!(foreign_keys_enabled(&connection));
}

#[test]
fn schema_v4_preserves_accounts_and_every_child_table() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    let account_id = insert_account(&connection, "uid-a", "sec-a", "profile-a");
    connection
        .execute_batch(&format!(
            "
            INSERT INTO videos
              (id, aweme_id, canonical_url, content_type, first_seen_at, last_seen_at)
            VALUES (1, 'video-a', 'https://www.douyin.com/video/video-a', 'video', 'x', 'x');
            INSERT INTO sync_jobs
              (id, account_id, source, started_at, status)
            VALUES (1, {account_id}, 'history', 'x', 'running');
            INSERT INTO watch_records
              (id, account_id, video_id, history_time_ms, watched_at, watched_date,
               source_position, sync_job_id, created_at, updated_at)
            VALUES (1, {account_id}, 1, 10, 'x', 'x', 0, 1, 'x', 'x');
            INSERT INTO user_likes
              (id, account_id, video_id, first_seen_at, last_seen_at)
            VALUES (1, {account_id}, 1, 'x', 'x');
            INSERT INTO favorite_items
              (id, account_id, video_id, first_seen_at, last_seen_at)
            VALUES (1, {account_id}, 1, 'x', 'x');
            INSERT INTO remote_folders
              (id, account_id, remote_folder_id, name, first_seen_at, last_seen_at)
            VALUES (1, {account_id}, 'remote-a', 'remote', 'x', 'x');
            INSERT INTO remote_folder_items
              (folder_id, video_id, first_seen_at, last_seen_at)
            VALUES (1, 1, 'x', 'x');
            INSERT INTO local_folders
              (id, scope, owner_account_id, name, created_at, updated_at)
            VALUES (1, 'aggregate', NULL, 'local', 'x', 'x');
            INSERT INTO local_folder_items (folder_id, video_id, added_at)
            VALUES (1, 1, 'x');
            INSERT INTO local_folder_item_accounts
              (folder_id, video_id, account_id, added_at)
            VALUES (1, 1, {account_id}, 'x');
            "
        ))
        .expect("seed v3 graph");

    initialize_schema_v4(&mut connection).expect("v4 migration");

    assert_eq!(user_version(&connection), 4);
    assert!(foreign_keys_enabled(&connection));
    assert_eq!(foreign_key_errors(&connection), 0);
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
    ] {
        assert_eq!(count(&connection, table), 1, "lost row from {table}");
    }
    assert_eq!(
        connection
            .query_row(
                "SELECT uid, sec_uid, profile_key, status FROM accounts WHERE id = ?1",
                [account_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
            )
            .expect("preserved account"),
        (
            "uid-a".to_string(),
            "sec-a".to_string(),
            "profile-a".to_string(),
            "active".to_string(),
        )
    );
}

#[test]
fn schema_v4_accepts_identity_conflict_and_rejects_unknown_account_status() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    let account_id = insert_account(&connection, "uid-a", "sec-a", "profile-a");

    initialize_schema_v4(&mut connection).expect("v4 migration");

    connection
        .execute(
            "UPDATE accounts SET status = 'identity_conflict' WHERE id = ?1",
            [account_id],
        )
        .expect("identity conflict status");
    assert!(connection
        .execute(
            "UPDATE accounts SET status = 'unknown' WHERE id = ?1",
            [account_id],
        )
        .is_err());
    assert!(!MIGRATION_V4.contains("BEGIN"));
    assert!(!MIGRATION_V4.contains("COMMIT"));
    assert!(!MIGRATION_V4.contains("PRAGMA foreign_keys"));
}

#[test]
fn schema_v4_failure_rolls_back_version_and_restores_foreign_keys() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    let account_id = insert_account(&connection, "uid-a", "sec-a", "profile-a");
    connection
        .execute_batch(
            "
            PRAGMA foreign_keys = OFF;
            INSERT INTO sync_jobs
              (id, account_id, source, started_at, status)
            VALUES (1, 999, 'history', 'x', 'running');
            PRAGMA foreign_keys = ON;
            ",
        )
        .expect("inject foreign key failure");
    assert_eq!(foreign_key_errors(&connection), 1);

    assert_eq!(
        initialize_schema_v4(&mut connection).unwrap_err(),
        "foreign_key_check_failed"
    );

    assert_eq!(user_version(&connection), 3);
    assert!(foreign_keys_enabled(&connection));
    assert_eq!(count(&connection, "accounts"), 1);
    assert_eq!(
        connection
            .query_row("SELECT status FROM accounts WHERE id = ?1", [account_id], |row| {
                row.get::<_, String>(0)
            })
            .expect("account status"),
        "active"
    );
}

#[test]
fn migration_matrix_handles_v0_v1_v2_v3_and_rejects_future_versions() {
    for source_version in 0..=2 {
        let mut connection = Connection::open_in_memory().expect("open db");
        match source_version {
            0 => {}
            1 => create_v1_schema(&connection),
            2 => initialize_schema_v2(&mut connection).expect("v2 schema"),
            _ => unreachable!(),
        }

        initialize_schema_v3(&mut connection).expect("supported migration");
        assert_eq!(user_version(&connection), 3);
        assert!(table_exists(&connection, "accounts"));
    }

    let mut current = Connection::open_in_memory().expect("open current db");
    initialize_schema_v3(&mut current).expect("create v3");
    insert_account(&current, "uid-current", "sec-current", "profile-current");
    initialize_schema_v3(&mut current).expect("repeat v3 initialization");
    assert_eq!(count(&current, "accounts"), 1);

    let mut future = Connection::open_in_memory().expect("open future db");
    future
        .execute_batch("CREATE TABLE marker (id INTEGER); PRAGMA user_version = 4;")
        .expect("future schema");
    assert_eq!(
        initialize_schema_v3(&mut future).unwrap_err(),
        "unsupported_schema_version:4"
    );
    assert_eq!(user_version(&future), 4);
    assert!(table_exists(&future, "marker"));
}

#[test]
fn v3_schema_has_required_tables_indexes_and_valid_foreign_keys() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");

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
    ] {
        assert!(table_exists(&connection, table), "missing table {table}");
    }
    for index in [
        "idx_accounts_uid",
        "idx_accounts_sec_uid",
        "idx_videos_canonical_url",
        "idx_watch_records_history_time",
        "idx_sync_jobs_account_status",
        "idx_user_likes_account_removed",
        "idx_favorite_items_account_removed",
        "idx_remote_folders_account_removed",
    ] {
        assert!(index_exists(&connection, index), "missing index {index}");
    }
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("foreign key check"),
        0
    );
    assert!(!MIGRATION_V3.contains("CREATE TABLE IF NOT EXISTS"));
    assert!(!MIGRATION_V3.contains("BEGIN"));
    assert!(!MIGRATION_V3.contains("COMMIT"));
    assert!(!MIGRATION_V3.contains("PRAGMA foreign_keys"));
}

#[test]
fn account_folder_and_watch_constraints_enforce_account_isolation() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    let account_one = insert_account(&connection, "uid-1", "sec-1", "profile-1");
    let account_two = insert_account(&connection, "uid-2", "sec-2", "profile-2");

    for sql in [
        "INSERT INTO accounts (uid, sec_uid, profile_key, status, created_at, updated_at) VALUES ('uid-1', 'sec-3', 'profile-3', 'active', 'x', 'x')",
        "INSERT INTO accounts (uid, sec_uid, profile_key, status, created_at, updated_at) VALUES ('uid-3', 'sec-1', 'profile-4', 'active', 'x', 'x')",
        "INSERT INTO accounts (uid, sec_uid, profile_key, status, created_at, updated_at) VALUES ('uid-4', 'sec-4', 'profile-1', 'active', 'x', 'x')",
        "INSERT INTO local_folders (scope, owner_account_id, name, created_at, updated_at) VALUES ('invalid', NULL, 'bad', 'x', 'x')",
        "INSERT INTO local_folders (scope, owner_account_id, name, created_at, updated_at) VALUES ('account', NULL, 'bad', 'x', 'x')",
        "INSERT INTO local_folders (scope, owner_account_id, name, created_at, updated_at) VALUES ('aggregate', 1, 'bad', 'x', 'x')",
        "INSERT INTO sync_jobs (account_id, source, started_at, status) VALUES (999, 'history', 'x', 'running')",
    ] {
        assert!(connection.execute(sql, []).is_err(), "constraint accepted: {sql}");
    }

    connection
        .execute(
            "INSERT INTO videos
             (id, aweme_id, canonical_url, content_type, first_seen_at, last_seen_at)
             VALUES (1, 'video-1', 'https://www.douyin.com/video/1', 'video', 'x', 'x')",
            [],
        )
        .expect("video");
    connection
        .execute(
            "INSERT INTO watch_records
             (account_id, video_id, history_time_ms, watched_at, watched_date, source_position, created_at, updated_at)
             VALUES (?1, 1, 10, 'x', 'x', 0, 'x', 'x')",
            [account_one],
        )
        .expect("first watch");
    assert!(connection
        .execute(
            "INSERT INTO watch_records
             (account_id, video_id, history_time_ms, watched_at, watched_date, source_position, created_at, updated_at)
             VALUES (?1, 1, 10, 'x', 'x', 0, 'x', 'x')",
            [account_one],
        )
        .is_err());
    connection
        .execute(
            "INSERT INTO watch_records
             (account_id, video_id, history_time_ms, watched_at, watched_date, source_position, created_at, updated_at)
             VALUES (?1, 1, 10, 'x', 'x', 0, 'x', 'x')",
            [account_two],
        )
        .expect("same watch identity for another account");
}

#[test]
fn failed_migration_rolls_back_schema_and_user_version() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v2(&mut connection).expect("v2 schema");
    connection
        .execute_batch(
            "
            CREATE TABLE local_folders (marker TEXT);
            INSERT INTO local_folders VALUES ('preserve-me');
            CREATE VIEW favorite_items AS SELECT 1 AS id;
            ",
        )
        .expect("failure injection");

    assert!(initialize_schema_v3(&mut connection).is_err());

    assert_eq!(user_version(&connection), 2);
    assert_eq!(count(&connection, "local_folders"), 1);
    assert!(table_exists(&connection, "videos"));
    assert!(!table_exists(&connection, "accounts"));
}

#[test]
fn version_three_validation_rejects_partial_schema() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    connection
        .execute("DROP TABLE user_likes", [])
        .expect("drop table");

    assert_eq!(
        initialize_schema_v3(&mut connection).unwrap_err(),
        "schema_v3_missing_table:user_likes"
    );
}

#[test]
fn version_three_validation_rejects_missing_required_column() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    connection
        .execute("ALTER TABLE accounts DROP COLUMN created_at", [])
        .expect("drop required column");

    assert_eq!(
        initialize_schema_v3(&mut connection).unwrap_err(),
        "schema_v3_missing_column:accounts:created_at"
    );
}

#[test]
fn version_three_validation_rejects_missing_previously_unchecked_column() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    connection
        .execute("ALTER TABLE accounts DROP COLUMN nickname", [])
        .expect("drop unchecked column");

    assert_eq!(
        initialize_schema_v3(&mut connection).unwrap_err(),
        "schema_v3_missing_column:accounts:nickname"
    );
}

#[test]
fn version_three_validation_rejects_invalid_not_null_signature() {
    let mut connection = Connection::open_in_memory().expect("open db");
    let damaged_migration = MIGRATION_V3.replace(
        "  canonical_url TEXT NOT NULL,\n",
        "  canonical_url TEXT,\n",
    );
    connection
        .execute_batch(&damaged_migration)
        .expect("damaged v3 schema");

    assert_eq!(
        initialize_schema_v3(&mut connection).unwrap_err(),
        "schema_v3_invalid_column:videos:canonical_url:notnull"
    );
}

#[test]
fn version_three_validation_rejects_invalid_primary_key_signature() {
    let mut connection = Connection::open_in_memory().expect("open db");
    let damaged_migration = MIGRATION_V3.replace(
        "  PRIMARY KEY(folder_id, video_id, account_id),\n",
        "  UNIQUE(folder_id, video_id, account_id),\n",
    );
    connection
        .execute_batch(&damaged_migration)
        .expect("damaged v3 schema");

    assert_eq!(
        initialize_schema_v3(&mut connection).unwrap_err(),
        "schema_v3_invalid_column:local_folder_item_accounts:folder_id:pk"
    );
}

#[test]
fn version_three_validation_rejects_invalid_partial_unique_index() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    connection
        .execute_batch(
            "
            DROP INDEX idx_accounts_uid;
            CREATE INDEX idx_accounts_uid ON accounts(uid);
            ",
        )
        .expect("replace partial unique index");

    assert_eq!(
        initialize_schema_v3(&mut connection).unwrap_err(),
        "schema_v3_invalid_index:idx_accounts_uid"
    );
}

#[test]
fn version_three_validation_rejects_widened_partial_index_predicate() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    connection
        .execute_batch(
            "
            DROP INDEX idx_accounts_uid;
            CREATE UNIQUE INDEX idx_accounts_uid ON accounts(uid)
              WHERE uid IS NOT NULL OR status = 'active';
            ",
        )
        .expect("widen partial unique index");

    assert_eq!(
        initialize_schema_v3(&mut connection).unwrap_err(),
        "schema_v3_invalid_index:idx_accounts_uid"
    );
}

#[test]
fn version_three_validation_rejects_account_identifier_nocase_collation() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    connection
        .execute_batch(
            "
            DROP INDEX idx_accounts_uid;
            CREATE UNIQUE INDEX idx_accounts_uid ON accounts(uid COLLATE NOCASE)
              WHERE uid IS NOT NULL;
            ",
        )
        .expect("replace account uid collation");

    assert_eq!(
        initialize_schema_v3(&mut connection).unwrap_err(),
        "schema_v3_invalid_index:idx_accounts_uid"
    );
}

#[test]
fn version_three_validation_rejects_local_folder_binary_name_collation() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");
    connection
        .execute_batch(
            "
            DROP INDEX idx_local_folders_account_name;
            CREATE UNIQUE INDEX idx_local_folders_account_name
              ON local_folders(owner_account_id, name COLLATE BINARY)
              WHERE scope = 'account';
            ",
        )
        .expect("replace local folder name collation");

    assert_eq!(
        initialize_schema_v3(&mut connection).unwrap_err(),
        "schema_v3_invalid_index:idx_local_folders_account_name"
    );
}

#[test]
fn version_three_validation_rejects_missing_required_unique_index() {
    let mut connection = Connection::open_in_memory().expect("open db");
    let damaged_migration = MIGRATION_V3.replace(
        "  updated_at TEXT NOT NULL,\n  UNIQUE(account_id, video_id, history_time_ms)\n",
        "  updated_at TEXT NOT NULL\n",
    );
    connection
        .execute_batch(&damaged_migration)
        .expect("damaged v3 schema");

    assert_eq!(
        initialize_schema_v3(&mut connection).unwrap_err(),
        "schema_v3_missing_unique_index:watch_records:account_id,video_id,history_time_ms"
    );
}

#[test]
fn history_calls_require_an_explicit_valid_account_id() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("v3 schema");

    assert_eq!(
        list_history(&connection, 0, None, 80, 0).unwrap_err(),
        "invalid_account_id"
    );
    assert_eq!(
        delete_history_by_ids(&mut connection, 0, &[1]).unwrap_err(),
        "invalid_account_id"
    );
    assert_eq!(
        delete_history_by_query(&mut connection, 0, None).unwrap_err(),
        "invalid_account_id"
    );
}

#[test]
fn public_sync_options_reject_unknown_fields() {
    let raw = r#"{"mode":"days","days":7,"unexpected":true}"#;

    assert!(serde_json::from_str::<SyncOptions>(raw).is_err());
}

#[test]
fn explicit_account_history_queries_and_deletes_are_isolated() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("schema");
    let account_one = insert_account(&connection, "uid-a", "sec-a", "profile-a");
    let account_two = insert_account(&connection, "uid-b", "sec-b", "profile-b");
    connection
        .execute_batch(
            "INSERT INTO videos
               (id, aweme_id, canonical_url, content_type, title, first_seen_at, last_seen_at)
             VALUES (1, 'shared', 'https://www.douyin.com/video/shared', 'video', 'shared title', 'x', 'x');",
        )
        .expect("video");
    for (account_id, history_time_ms) in [(account_one, 10_i64), (account_two, 20_i64)] {
        connection
            .execute(
                "INSERT INTO watch_records
                 (account_id, video_id, history_time_ms, watched_at, watched_date,
                  source_position, created_at, updated_at)
                 VALUES (?1, 1, ?2, 'x', 'x', 0, 'x', 'x')",
                params![account_id, history_time_ms],
            )
            .expect("watch");
    }

    let first = list_history(&connection, account_one, Some("shared"), 80, 0).expect("list a");
    let second = list_history(&connection, account_two, None, 80, 0).expect("list b");
    assert_eq!(first.total, 1);
    assert_eq!(second.total, 1);
    assert_ne!(first.records[0].id, second.records[0].id);

    let deleted = delete_history_by_ids(&mut connection, account_one, &[first.records[0].id])
        .expect("delete a");
    assert_eq!(deleted.deleted_watch_records, 1);
    assert_eq!(deleted.deleted_videos, 0);
    assert_eq!(list_history(&connection, account_one, None, 80, 0).unwrap().total, 0);
    assert_eq!(list_history(&connection, account_two, None, 80, 0).unwrap().total, 1);

    connection
        .execute_batch(
            "INSERT INTO videos
               (id, aweme_id, canonical_url, content_type, title, first_seen_at, last_seen_at)
             VALUES (2, 'query-only', 'https://www.douyin.com/video/query-only', 'video', 'query-only', 'x', 'x');",
        )
        .unwrap();
    for account_id in [account_one, account_two] {
        connection
            .execute(
                "INSERT INTO watch_records
                 (account_id, video_id, history_time_ms, watched_at, watched_date,
                  source_position, created_at, updated_at)
                 VALUES (?1, 2, 30, 'x', 'x', 0, 'x', 'x')",
                [account_id],
            )
            .unwrap();
    }

    let deleted = delete_history_by_query(&mut connection, account_one, Some("query-only"))
        .expect("delete account query");
    assert_eq!(deleted.deleted_watch_records, 1);
    assert_eq!(list_history(&connection, account_one, Some("query-only"), 80, 0).unwrap().total, 0);
    assert_eq!(list_history(&connection, account_two, Some("query-only"), 80, 0).unwrap().total, 1);
}

#[test]
fn imports_the_same_remote_event_once_per_account_while_sharing_video_metadata() {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("schema");
    let account_one = insert_account(&connection, "uid-a", "sec-a", "profile-a");
    let account_two = insert_account(&connection, "uid-b", "sec-b", "profile-b");
    let history_time_ms = 1_783_700_000_000_i64;

    for (job_id, account_id) in [(1_i64, account_one), (2_i64, account_two)] {
        connection
            .execute(
                "INSERT INTO sync_jobs (id, account_id, source, started_at, status)
                 VALUES (?1, ?2, 'history', 'x', 'running')",
                params![job_id, account_id],
            )
            .unwrap();
        let result = collector_result(
            account_id,
            job_id,
            vec![collector_record("7312345678901234567", history_time_ms)],
        );
        validate_and_import_result(&mut connection, &result, job_id).expect("import");
    }

    assert_eq!(count(&connection, "videos"), 1);
    assert_eq!(count(&connection, "watch_records"), 2);
    assert_eq!(list_history(&connection, account_one, None, 80, 0).unwrap().total, 1);
    assert_eq!(list_history(&connection, account_two, None, 80, 0).unwrap().total, 1);
}

#[test]
fn orphan_cleanup_preserves_every_future_source_reference() {
    for source in ["likes", "favorites", "remote_folder", "local_folder"] {
        let mut connection = Connection::open_in_memory().expect("open db");
        initialize_schema_v3(&mut connection).expect("schema");
        let account_id = insert_account(&connection, "uid-a", "sec-a", "profile-a");
        connection
            .execute_batch(
                "INSERT INTO videos
                   (id, aweme_id, canonical_url, content_type, first_seen_at, last_seen_at)
                 VALUES (1, 'shared', 'https://www.douyin.com/video/shared', 'video', 'x', 'x');",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO watch_records
                 (id, account_id, video_id, history_time_ms, watched_at, watched_date,
                  source_position, created_at, updated_at)
                 VALUES (1, ?1, 1, 10, 'x', 'x', 0, 'x', 'x')",
                [account_id],
            )
            .unwrap();
        match source {
            "likes" => {
                connection.execute("INSERT INTO user_likes (account_id, video_id, first_seen_at, last_seen_at) VALUES (?1, 1, 'x', 'x')", [account_id]).unwrap();
            }
            "favorites" => {
                connection.execute("INSERT INTO favorite_items (account_id, video_id, first_seen_at, last_seen_at) VALUES (?1, 1, 'x', 'x')", [account_id]).unwrap();
            }
            "remote_folder" => {
                connection.execute("INSERT INTO remote_folders (id, account_id, remote_folder_id, name, first_seen_at, last_seen_at) VALUES (1, ?1, 'folder', 'folder', 'x', 'x')", [account_id]).unwrap();
                connection.execute("INSERT INTO remote_folder_items (folder_id, video_id, first_seen_at, last_seen_at) VALUES (1, 1, 'x', 'x')", []).unwrap();
            }
            "local_folder" => {
                connection.execute("INSERT INTO local_folders (id, scope, owner_account_id, name, created_at, updated_at) VALUES (1, 'account', ?1, 'folder', 'x', 'x')", [account_id]).unwrap();
                connection.execute("INSERT INTO local_folder_items (folder_id, video_id, added_at) VALUES (1, 1, 'x')", []).unwrap();
            }
            _ => unreachable!(),
        }

        let deleted = delete_history_by_ids(&mut connection, account_id, &[1]).unwrap();
        assert_eq!(deleted.deleted_videos, 0, "source {source}");
        assert_eq!(count(&connection, "videos"), 1, "source {source}");

        match source {
            "likes" => {
                connection.execute("DELETE FROM user_likes", []).unwrap();
            }
            "favorites" => {
                connection.execute("DELETE FROM favorite_items", []).unwrap();
            }
            "remote_folder" => {
                connection.execute("DELETE FROM remote_folder_items", []).unwrap();
            }
            "local_folder" => {
                connection.execute("DELETE FROM local_folder_items", []).unwrap();
            }
            _ => unreachable!(),
        }
        connection
            .execute(
                "INSERT INTO watch_records
                 (id, account_id, video_id, history_time_ms, watched_at, watched_date,
                  source_position, created_at, updated_at)
                 VALUES (2, ?1, 1, 20, 'x', 'x', 0, 'x', 'x')",
                [account_id],
            )
            .unwrap();
        let deleted = delete_history_by_ids(&mut connection, account_id, &[2]).unwrap();
        assert_eq!(deleted.deleted_videos, 1, "source {source}");
        assert_eq!(count(&connection, "videos"), 0, "source {source}");
    }
}
