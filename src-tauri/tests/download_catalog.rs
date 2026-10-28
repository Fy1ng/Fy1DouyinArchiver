use std::collections::BTreeSet;

use douyin_history_archive::download::{
    create_download_job, expand_candidates, CreateDownloadJobRequest, DownloadJobOwner,
    DownloadQuality, DownloadSelection,
};
use rusqlite::{params, Connection};

fn test_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("database");
    connection
        .execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE accounts (
              id INTEGER PRIMARY KEY,
              profile_key TEXT NOT NULL UNIQUE
            );

            CREATE TABLE videos (
              id INTEGER PRIMARY KEY,
              aweme_id TEXT NOT NULL,
              canonical_url TEXT NOT NULL,
              content_type TEXT NOT NULL,
              title TEXT,
              author_name TEXT,
              cover_url TEXT,
              like_count INTEGER,
              first_seen_at TEXT NOT NULL,
              last_seen_at TEXT NOT NULL
            );

            CREATE TABLE watch_records (
              id INTEGER PRIMARY KEY,
              account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
              video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
              history_time_ms INTEGER NOT NULL,
              watched_at TEXT NOT NULL,
              watched_date TEXT NOT NULL,
              source_position INTEGER NOT NULL,
              watch_progress_rate REAL,
              sync_job_id INTEGER,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            "#,
        )
        .expect("history schema");
    connection
        .execute_batch(
            r#"
            INSERT INTO accounts(id, profile_key) VALUES
              (1, '11111111111111111111111111111111'),
              (2, '22222222222222222222222222222222');
            "#,
        )
        .expect("accounts");
    connection
        .execute_batch(include_str!("../migrations/007_download_jobs.sql"))
        .expect("download schema");
    connection
}

fn insert_history(
    connection: &Connection,
    video_id: i64,
    watch_record_id: i64,
    aweme_id: &str,
    title: &str,
    author_name: &str,
    history_time_ms: i64,
    source_position: i64,
) {
    insert_history_for_account(
        connection,
        1,
        video_id,
        watch_record_id,
        aweme_id,
        title,
        author_name,
        history_time_ms,
        source_position,
    );
}

fn insert_history_for_account(
    connection: &Connection,
    account_id: i64,
    video_id: i64,
    watch_record_id: i64,
    aweme_id: &str,
    title: &str,
    author_name: &str,
    history_time_ms: i64,
    source_position: i64,
) {
    connection
        .execute(
            "INSERT INTO videos
             (id, aweme_id, canonical_url, content_type, title, author_name, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, 'video', ?4, ?5, 'now', 'now')",
            params![
                video_id,
                aweme_id,
                format!("https://www.douyin.com/video/{aweme_id}"),
                title,
                author_name
            ],
        )
        .expect("video");
    connection
        .execute(
            "INSERT INTO watch_records
             (id, account_id, video_id, history_time_ms, watched_at, watched_date, source_position,
              created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'now', '2026-07-12', ?5, 'now', 'now')",
            params![
                watch_record_id,
                account_id,
                video_id,
                history_time_ms,
                source_position
            ],
        )
        .expect("watch record");
}

fn owner() -> DownloadJobOwner {
    DownloadJobOwner {
        account_id: 1,
        profile_key: "11111111111111111111111111111111".to_string(),
    }
}

fn request(selection: DownloadSelection) -> CreateDownloadJobRequest {
    CreateDownloadJobRequest {
        selection,
        destination_root: "C:/downloads".to_string(),
        requested_quality: DownloadQuality::Highest,
    }
}

#[test]
fn catalog_deduplicates_by_newest_time_then_source_position_then_record_id() {
    let connection = test_connection();
    insert_history(&connection, 1, 1, "1001", "old", "author", 100, 0);
    insert_history(
        &connection,
        2,
        2,
        "1001",
        "newest-worse-position",
        "author",
        200,
        9,
    );
    insert_history(
        &connection,
        3,
        3,
        "1001",
        "newest-lower-id",
        "author",
        200,
        1,
    );
    insert_history(&connection, 4, 4, "1001", "newest", "author", 200, 1);

    let candidates =
        expand_candidates(&connection, 1, &DownloadSelection::All).expect("candidates");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].aweme_id, "1001");
    assert_eq!(candidates[0].title.as_deref(), Some("newest"));
}

#[test]
fn checked_duplicate_ids_create_one_download_item() {
    let mut connection = test_connection();
    insert_history(&connection, 1, 10, "1001", "snapshot", "author", 100, 0);

    let job_id = create_download_job(
        &mut connection,
        &owner(),
        &request(DownloadSelection::Checked {
            watch_record_ids: vec![10, 10, 10],
        }),
    )
    .expect("job");

    let (scope, candidate_count): (String, i64) = connection
        .query_row(
            "SELECT selection_scope, candidate_count FROM download_jobs WHERE id = ?1",
            [job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("job row");
    let item_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM download_items WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .expect("item count");

    assert_eq!(scope, "selected");
    assert_eq!(candidate_count, 1);
    assert_eq!(item_count, 1);
}

#[test]
fn filtered_selection_expands_every_match_without_pagination() {
    let connection = test_connection();
    insert_history(&connection, 1, 1, "1001", "猫一", "作者", 300, 0);
    insert_history(&connection, 2, 2, "1002", "普通", "猫作者", 200, 1);
    insert_history(&connection, 3, 3, "1003", "猫二", "作者", 100, 2);
    insert_history(&connection, 4, 4, "1004", "无关", "作者", 50, 3);

    let candidates = expand_candidates(
        &connection,
        1,
        &DownloadSelection::Filtered {
            query: Some(" 猫 ".to_string()),
        },
    )
    .expect("filtered candidates");
    let ids = candidates
        .iter()
        .map(|candidate| candidate.aweme_id.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(ids, BTreeSet::from(["1001", "1002", "1003"]));
    assert!(candidates
        .iter()
        .all(|candidate| candidate.filter_context.as_deref() == Some("猫")));
}

#[test]
fn job_and_item_snapshots_roll_back_together() {
    let mut connection = test_connection();
    insert_history(&connection, 1, 1, "1001", "first", "author", 200, 0);
    insert_history(&connection, 2, 2, "1002", "second", "author", 100, 1);
    connection
        .execute_batch(
            r#"
            CREATE TRIGGER reject_second_snapshot
            BEFORE INSERT ON download_items
            WHEN NEW.aweme_id = '1002'
            BEGIN
              SELECT RAISE(ABORT, 'forced snapshot failure');
            END;
            "#,
        )
        .expect("trigger");

    let error = create_download_job(&mut connection, &owner(), &request(DownloadSelection::All))
        .expect_err("job creation must fail");

    assert_eq!(error.code, "download_storage_failed");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM download_jobs", [], |row| row
                .get::<_, i64>(0))
            .expect("job count"),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM download_items", [], |row| row
                .get::<_, i64>(0))
            .expect("item count"),
        0
    );
}

#[test]
fn job_items_are_history_independent_snapshots() {
    let mut connection = test_connection();
    insert_history(
        &connection,
        1,
        1,
        "1001",
        "durable snapshot",
        "author",
        100,
        0,
    );
    let job_id = create_download_job(
        &mut connection,
        &owner(),
        &request(DownloadSelection::All),
    )
    .expect("job");

    connection
        .execute("DELETE FROM watch_records", [])
        .expect("delete watch records");
    connection
        .execute("DELETE FROM videos", [])
        .expect("delete videos");

    let snapshot: (String, String, String) = connection
        .query_row(
            "SELECT aweme_id, title, relative_work_directory
             FROM download_items WHERE job_id = ?1",
            [job_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("snapshot");
    assert_eq!(
        snapshot,
        (
            "1001".to_string(),
            "durable snapshot".to_string(),
            "author".to_string() + std::path::MAIN_SEPARATOR_STR + "未知时间_durable snapshot_1001"
        )
    );
}

#[test]
fn catalog_and_job_snapshot_are_isolated_to_the_bound_account() {
    let mut connection = test_connection();
    insert_history(&connection, 1, 1, "1001", "account one", "author", 200, 0);
    insert_history_for_account(
        &connection,
        2,
        2,
        2,
        "2001",
        "account two",
        "author",
        300,
        0,
    );

    let all = expand_candidates(&connection, 1, &DownloadSelection::All).expect("all");
    let checked = expand_candidates(
        &connection,
        1,
        &DownloadSelection::Checked {
            watch_record_ids: vec![1, 2],
        },
    )
    .expect("checked");
    let job_id = create_download_job(
        &mut connection,
        &owner(),
        &request(DownloadSelection::All),
    )
    .expect("job");

    assert_eq!(all.iter().map(|item| item.aweme_id.as_str()).collect::<Vec<_>>(), vec!["1001"]);
    assert_eq!(checked.iter().map(|item| item.aweme_id.as_str()).collect::<Vec<_>>(), vec!["1001"]);
    let binding: (i64, String) = connection
        .query_row(
            "SELECT account_id, profile_key FROM download_jobs WHERE id = ?1",
            [job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("job binding");
    assert_eq!(binding, (1, "11111111111111111111111111111111".to_string()));
}
