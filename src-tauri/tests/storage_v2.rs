use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use douyin_history_archive::{
    compute_collector_hashes, delete_history_by_ids, initialize_schema_v2, initialize_schema_v3,
    list_history, validate_and_import_result, CollectorRange, CollectorRecordV2,
    CollectorSuccessV3,
};
use rusqlite::Connection;

fn record(id: &str, history_time_ms: i64, source_position: i64) -> CollectorRecordV2 {
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
        source_position,
        title: Some("标题".to_string()),
        author_name: Some("作者".to_string()),
        cover_url: None,
        like_count: Some(100),
        watch_progress_rate: Some(0.5),
    }
}

fn result(records: Vec<CollectorRecordV2>) -> CollectorSuccessV3 {
    let (identity_hash, field_hash) = compute_collector_hashes(&records).expect("hash records");
    CollectorSuccessV3 {
        schema_version: 3,
        account_id: 1,
        sync_job_id: 1,
        sync_started_at_ms: 1_783_742_400_000,
        calendar_time_zone: "Asia/Shanghai".to_string(),
        range: CollectorRange::Bottom,
        detected_history_time_unit: "milliseconds".to_string(),
        identity_hash,
        field_hash,
        records,
    }
}

fn refresh_hashes(result: &mut CollectorSuccessV3) {
    let (identity_hash, field_hash) = compute_collector_hashes(&result.records).expect("hash");
    result.identity_hash = identity_hash;
    result.field_hash = field_hash;
}

fn initialized_v3() -> (Connection, i64) {
    let mut connection = Connection::open_in_memory().expect("open db");
    initialize_schema_v3(&mut connection).expect("schema");
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES ('uid-test', 'sec-test', 'profile-test', 'active', 'x', 'x')",
            [],
        )
        .expect("account");
    let account_id = connection.last_insert_rowid();
    (connection, account_id)
}

fn import_error(result: &CollectorSuccessV3) -> String {
    let (mut connection, account_id) = initialized_v3();
    connection
        .execute(
            "INSERT INTO sync_jobs (id, account_id, source, started_at, status)
             VALUES (1, ?1, 'history', 'x', 'running')",
            [account_id],
        )
        .expect("job");
    validate_and_import_result(&mut connection, result, 1).unwrap_err()
}

#[test]
fn collector_result_deserializes_camel_case_day_range() {
    let raw = r#"{
        "schemaVersion": 3,
        "accountId": 1,
        "syncJobId": 1,
        "syncStartedAtMs": 1783742400000,
        "calendarTimeZone": "Asia/Shanghai",
        "range": {
            "mode": "days",
            "days": 7,
            "cutoffMs": 1783180800000
        },
        "detectedHistoryTimeUnit": "milliseconds",
        "identityHash": "0000000000000000000000000000000000000000000000000000000000000000",
        "fieldHash": "0000000000000000000000000000000000000000000000000000000000000000",
        "records": []
    }"#;

    let parsed: CollectorSuccessV3 = serde_json::from_str(raw).expect("camelCase result");

    assert!(matches!(
        parsed.range,
        CollectorRange::Days {
            days: 7,
            cutoff_ms: 1_783_180_800_000
        }
    ));
}

#[test]
fn migration_rebuilds_legacy_tables_and_preserves_sync_jobs() {
    let mut connection = Connection::open_in_memory().expect("open db");
    connection
        .execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE videos (id INTEGER PRIMARY KEY, aweme_id TEXT UNIQUE, canonical_url TEXT UNIQUE, title TEXT, author_name TEXT, cover_url TEXT, like_count_text TEXT, first_seen_at TEXT, last_seen_at TEXT);
            CREATE TABLE sync_jobs (id INTEGER PRIMARY KEY, started_at TEXT NOT NULL, finished_at TEXT, source_file TEXT, status TEXT NOT NULL, inserted_videos INTEGER NOT NULL DEFAULT 0, inserted_watch_records INTEGER NOT NULL DEFAULT 0, error_message TEXT);
            CREATE TABLE watch_records (id INTEGER PRIMARY KEY, video_id INTEGER, page_group_label TEXT, collected_at TEXT, source TEXT, source_url TEXT, capture_order INTEGER, sync_job_id INTEGER, created_at TEXT, UNIQUE(video_id, page_group_label));
            INSERT INTO videos VALUES (1, '1', 'https://www.douyin.com/video/1', NULL, NULL, NULL, NULL, 'x', 'x');
            INSERT INTO sync_jobs VALUES (1, 'x', NULL, 'old.json', 'success', 1, 1, NULL);
            INSERT INTO watch_records VALUES (1, 1, '昨天', 'x', 'dom', '', 1, 1, 'x');
            PRAGMA user_version = 1;
            ",
        )
        .expect("legacy schema");

    initialize_schema_v2(&mut connection).expect("migrate");

    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM videos", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM watch_records", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT source_file FROM sync_jobs WHERE id = 1",
                [],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
}

#[test]
fn valid_result_is_imported_atomically_and_repeated_events_are_upserted() {
    let (mut connection, account_id) = initialized_v3();
    connection
        .execute(
            "INSERT INTO sync_jobs (id, account_id, source, started_at, status)
             VALUES (1, ?1, 'history', 'x', 'running')",
            [account_id],
        )
        .unwrap();
    let first = result(vec![record("1001", 1_783_731_600_000, 0)]);

    let summary = validate_and_import_result(&mut connection, &first, 1).expect("import");
    assert_eq!(summary.inserted_videos, 1);
    assert_eq!(summary.inserted_watch_records, 1);

    connection
        .execute("UPDATE sync_jobs SET status = 'running' WHERE id = 1", [])
        .unwrap();
    let repeated = result(vec![CollectorRecordV2 {
        like_count: Some(200),
        watch_progress_rate: Some(0.75),
        ..record("1001", 1_783_731_600_000, 0)
    }]);
    validate_and_import_result(&mut connection, &repeated, 1).expect("repeat import");

    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM watch_records", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT like_count FROM videos", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        200
    );
    assert_eq!(
        connection
            .query_row("SELECT watch_progress_rate FROM watch_records", [], |row| {
                row.get::<_, f64>(0)
            })
            .unwrap(),
        0.75
    );
}

#[test]
fn invalid_hash_leaves_content_tables_unchanged() {
    let (mut connection, account_id) = initialized_v3();
    connection
        .execute(
            "INSERT INTO sync_jobs (id, account_id, source, started_at, status)
             VALUES (1, ?1, 'history', 'x', 'running')",
            [account_id],
        )
        .unwrap();
    let mut invalid = result(vec![record("1001", 1_783_731_600_000, 0)]);
    invalid.field_hash = "0".repeat(64);

    assert_eq!(
        validate_and_import_result(&mut connection, &invalid, 1).unwrap_err(),
        "field_hash_mismatch"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM videos", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM watch_records", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn collector_result_account_must_match_sync_job_account() {
    let (mut connection, account_id) = initialized_v3();
    connection
        .execute(
            "INSERT INTO sync_jobs (id, account_id, source, started_at, status)
             VALUES (1, ?1, 'history', 'x', 'running')",
            [account_id],
        )
        .unwrap();
    let mut mismatched = result(vec![record("1001", 1_783_731_600_000, 0)]);
    mismatched.account_id = account_id + 1;

    assert_eq!(
        validate_and_import_result(&mut connection, &mismatched, 1).unwrap_err(),
        "sync_account_mismatch"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM videos", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn rust_revalidates_time_range_and_canonical_url_contracts() {
    let mut invalid_watched_at = result(vec![record("1001", 1_783_731_600_000, 0)]);
    invalid_watched_at.records[0].watched_at = "2026-07-11T01:00:01.000Z".to_string();
    refresh_hashes(&mut invalid_watched_at);
    assert_eq!(import_error(&invalid_watched_at), "invalid_watched_at");

    let mut invalid_range = result(vec![record("1001", 1_783_731_600_000, 0)]);
    invalid_range.range = CollectorRange::Days {
        days: 2,
        cutoff_ms: 0,
    };
    assert_eq!(import_error(&invalid_range), "range_cutoff_mismatch");

    let mut invalid_url = result(vec![record("1001", 1_783_731_600_000, 0)]);
    invalid_url.records[0].canonical_url =
        "https://www.douyin.com/video/1001?modal_id=1001".to_string();
    refresh_hashes(&mut invalid_url);
    assert_eq!(import_error(&invalid_url), "invalid_canonical_url");
}

#[test]
fn history_query_uses_precise_time_order_and_delete_cleans_orphan_videos() {
    let (mut connection, account_id) = initialized_v3();
    connection
        .execute(
            "INSERT INTO sync_jobs (id, account_id, source, started_at, status)
             VALUES (1, ?1, 'history', 'x', 'running')",
            [account_id],
        )
        .unwrap();
    let records = vec![
        CollectorRecordV2 {
            title: Some("可搜索标题".to_string()),
            ..record("1002", 1_783_731_700_000, 0)
        },
        record("1001", 1_783_731_600_000, 1),
    ];
    validate_and_import_result(&mut connection, &result(records), 1).expect("import");

    let listed = list_history(&connection, account_id, None, 80, 0).expect("list");
    assert_eq!(listed.total, 2);
    assert_eq!(listed.records[0].aweme_id, "1002");
    assert_eq!(listed.records[0].watched_date, "2026-07-11");
    let filtered = list_history(&connection, account_id, Some("可搜索"), 80, 0).expect("filter");
    assert_eq!(filtered.total, 1);

    let deleted = delete_history_by_ids(&mut connection, account_id, &[listed.records[0].id])
        .expect("delete");
    assert_eq!(deleted.deleted_watch_records, 1);
    assert_eq!(deleted.deleted_videos, 1);
    assert_eq!(deleted.remaining_watch_records, 1);
}
