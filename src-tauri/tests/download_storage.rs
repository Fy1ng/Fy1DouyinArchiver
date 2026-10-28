use douyin_history_archive::{
    assert_schema_latest, configure_connection, migrate_schema_to_latest_at_startup,
};
use rusqlite::Connection;

fn user_version(connection: &Connection) -> i64 {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user version")
}

fn object_exists(connection: &Connection, object_type: &str, name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
            [object_type, name],
            |row| row.get(0),
        )
        .expect("schema object lookup")
}

fn table_columns(connection: &Connection, table: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("table info");
    statement
        .query_map([], |row| row.get(1))
        .expect("columns")
        .collect::<Result<Vec<_>, _>>()
        .expect("column rows")
}

fn create_v6_database(connection: &mut Connection) {
    migrate_schema_to_latest_at_startup(connection).expect("latest schema fixture");
    connection
        .execute_batch(
            "DROP TABLE notifications;
             DROP TABLE download_assets;
             DROP TABLE download_items;
             DROP TABLE download_jobs;
             PRAGMA user_version = 6;",
        )
        .expect("schema v6 fixture");
}

#[test]
fn download_migration_leaves_user_version_to_the_rust_registry() {
    let sql = include_str!("../migrations/007_download_jobs.sql");
    assert!(!sql.to_ascii_lowercase().contains("pragma user_version"));
}

#[test]
fn startup_registry_migrates_v6_through_v8_with_download_tables_and_indexes() {
    let mut connection = Connection::open_in_memory().expect("database");
    create_v6_database(&mut connection);

    assert!(assert_schema_latest(&connection).is_err());
    migrate_schema_to_latest_at_startup(&mut connection).expect("latest schema migration");

    assert_eq!(user_version(&connection), 8);
    assert_schema_latest(&connection).expect("latest schema");
    for table in ["download_jobs", "download_items", "download_assets"] {
        assert!(
            object_exists(&connection, "table", table),
            "missing {table}"
        );
    }
    for index in [
        "idx_download_jobs_active",
        "idx_download_items_job_state",
        "idx_download_assets_item_state",
        "idx_download_assets_historical_reuse",
    ] {
        assert!(
            object_exists(&connection, "index", index),
            "missing {index}"
        );
    }
    let columns = table_columns(&connection, "download_jobs");
    assert!(columns.contains(&"account_id".to_string()));
    assert!(columns.contains(&"profile_key".to_string()));
    let account_target: String = connection
        .query_row(
            "SELECT \"table\" FROM pragma_foreign_key_list('download_jobs') WHERE \"from\" = 'account_id'",
            [],
            |row| row.get(0),
        )
        .expect("download account foreign key");
    assert_eq!(account_target, "accounts");
}

#[test]
fn startup_registry_is_idempotent_and_preserves_download_rows() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_schema_to_latest_at_startup(&mut connection).expect("first initialization");
    connection
        .execute(
            "INSERT INTO accounts(profile_key, status, created_at, updated_at)
             VALUES ('11111111111111111111111111111111', 'pending_login', 'now', 'now')",
            [],
        )
        .expect("account");
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (1, 1)",
            [],
        )
        .expect("registry state");
    connection
        .execute(
            "INSERT INTO download_jobs (
                account_id, profile_key, created_at, selection_scope, destination_root, requested_quality, state,
                candidate_count, resolved_count, completed_count, skipped_count,
                failed_count, downloaded_bytes
             ) VALUES (1, '11111111111111111111111111111111', 'now', 'selected', 'C:/downloads', 'highest', 'queued', 0, 0, 0, 0, 0, 0)",
            [],
        )
        .expect("job");

    migrate_schema_to_latest_at_startup(&mut connection).expect("second initialization");

    assert_eq!(user_version(&connection), 8);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM download_jobs", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("job count"),
        1
    );
}

#[test]
fn startup_registry_initializes_an_empty_database_through_v8() {
    let mut connection = Connection::open_in_memory().expect("database");

    migrate_schema_to_latest_at_startup(&mut connection).expect("schema");

    assert_eq!(user_version(&connection), 8);
    assert_schema_latest(&connection).expect("latest schema");
}

#[test]
fn startup_registry_rejects_future_versions_without_modifying_them() {
    let mut connection = Connection::open_in_memory().expect("database");
    connection
        .execute_batch("PRAGMA user_version = 9;")
        .expect("future schema");

    let error =
        migrate_schema_to_latest_at_startup(&mut connection).expect_err("future version must fail");

    assert_eq!(error, "unsupported_schema_version:9");
    assert_eq!(user_version(&connection), 9);
    assert!(!object_exists(&connection, "table", "download_jobs"));
}

#[test]
fn connection_configuration_enables_foreign_keys_and_busy_timeout() {
    let connection = Connection::open_in_memory().expect("database");

    configure_connection(&connection).expect("configure");

    assert_eq!(
        connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .expect("foreign keys"),
        1
    );
    assert_eq!(
        connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .expect("busy timeout"),
        5_000
    );
}

#[test]
fn download_tables_do_not_reference_history_tables() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_schema_to_latest_at_startup(&mut connection).expect("schema");

    for table in ["download_jobs", "download_items", "download_assets"] {
        let mut statement = connection
            .prepare(&format!("PRAGMA foreign_key_list({table})"))
            .expect("foreign key statement");
        let referenced_tables = statement
            .query_map([], |row| row.get::<_, String>(2))
            .expect("foreign keys")
            .collect::<Result<Vec<_>, _>>()
            .expect("foreign key rows");
        assert!(!referenced_tables
            .iter()
            .any(|referenced| referenced == "videos" || referenced == "watch_records"));
    }
}
