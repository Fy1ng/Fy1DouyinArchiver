use chrono::DateTime;
use douyin_history_archive::settings_store::AccountScopeV1;
use douyin_history_archive::{
    initialize_schema_v4, migrate_schema_to_latest_at_startup, query_annual_review_at,
    query_overview_scope_at, AnnualReviewQueryV1, OverviewRangeV1, OverviewSourceFilterV1,
};
use rusqlite::{params, Connection};

fn fixture() -> (Connection, i64, i64, i64) {
    let mut connection = Connection::open_in_memory().expect("database");
    initialize_schema_v4(&mut connection).expect("schema v4");
    migrate_schema_to_latest_at_startup(&mut connection).expect("latest schema");
    for (uid, profile) in [("uid-a", "profile-a"), ("uid-b", "profile-b")] {
        connection
            .execute(
                "INSERT INTO accounts
                 (uid, sec_uid, profile_key, status, created_at, updated_at)
                 VALUES (?1, ?1, ?2, 'active', 'created', 'updated')",
                params![uid, profile],
            )
            .unwrap();
    }
    connection
        .execute_batch(
            "INSERT INTO videos
               (id, aweme_id, canonical_url, content_type, title, author_name,
                first_seen_at, last_seen_at)
             VALUES
               (1, 'one', 'https://www.douyin.com/video/one', 'video', '视频一', '作者甲', 'first', 'last'),
               (2, 'two', 'https://www.douyin.com/video/two', 'note', '图文二', '作者甲', 'first', 'last'),
               (3, 'three', 'https://www.douyin.com/video/three', 'video', '视频三', '', 'first', 'last'),
               (4, 'four', 'https://www.douyin.com/video/four', 'video', '视频四', '作者乙', 'first', 'last');

             INSERT INTO watch_records
               (account_id, video_id, history_time_ms, watched_at, watched_date,
                source_position, watch_progress_rate, created_at, updated_at)
             VALUES
               (1, 1, 1786665600000, '2026-08-14T00:00:00Z', '2026-08-14', 0, 0.95, '2026-08-14T00:00:00Z', 'updated'),
               (1, 1, 1786665601000, '2026-08-14T00:00:01Z', '2026-08-14', 1, 0.40, '2026-08-14T00:00:01Z', 'updated'),
               (1, 2, 1786579200000, '2026-08-13T00:00:00Z', '2026-08-13', 2, NULL, '2026-08-13T00:00:00Z', 'updated'),
               (1, 3, 1751328000000, '2025-07-01T00:00:00Z', '2025-07-01', 3, 0.10, '2025-07-01T00:00:00Z', 'updated'),
               (2, 4, 1786665600000, '2026-08-14T00:00:00Z', '2026-08-14', 0, 0.75, '2026-08-14T00:00:00Z', 'updated');

             INSERT INTO user_likes
               (account_id, video_id, first_seen_at, last_seen_at, remote_created_at_ms, remote_removed_at, source_position)
             VALUES (1, 3, '2026-08-14T00:00:00Z', '2026-08-14T00:00:00Z', NULL, NULL, 0);

             INSERT INTO favorite_items
               (account_id, video_id, first_seen_at, last_seen_at, remote_created_at_ms, remote_removed_at, source_position)
             VALUES (2, 2, '2026-08-14T00:00:00Z', '2026-08-14T00:00:00Z', NULL, NULL, 0);",
        )
        .unwrap();
    let now_ms = DateTime::parse_from_rfc3339("2026-08-14T12:00:00Z")
        .unwrap()
        .timestamp_millis();
    (connection, 1, 2, now_ms)
}

#[test]
fn scoped_overview_aggregates_all_accounts_but_rejects_stale_selected_scope() {
    let (connection, account_one, account_two, now_ms) = fixture();
    let all = query_overview_scope_at(
        &connection,
        &AccountScopeV1::All,
        OverviewSourceFilterV1::All,
        OverviewRangeV1::All,
        false,
        now_ms,
    )
    .unwrap();
    assert_eq!(all.unique_content_count, 4);
    assert_eq!(all.source_counts.history, 5);
    assert_eq!(all.source_counts.likes, 1);
    assert_eq!(all.source_counts.favorites, 1);
    assert_eq!(all.recent_sync.len(), 0);

    let selected = query_overview_scope_at(
        &connection,
        &AccountScopeV1::Selected {
            account_ids: vec![account_one, account_two],
        },
        OverviewSourceFilterV1::History,
        OverviewRangeV1::All,
        false,
        now_ms,
    )
    .unwrap();
    assert_eq!(selected.source_counts.history, 5);

    connection
        .execute("DELETE FROM accounts WHERE id = ?1", [account_two])
        .unwrap();
    assert_eq!(
        query_overview_scope_at(
            &connection,
            &AccountScopeV1::Selected {
                account_ids: vec![account_one, account_two],
            },
            OverviewSourceFilterV1::All,
            OverviewRangeV1::All,
            false,
            now_ms,
        )
        .unwrap_err(),
        "account_scope_invalid"
    );
}

#[test]
fn annual_review_falls_back_to_latest_history_year_and_preserves_sparse_semantics() {
    let (connection, account_one, _, now_ms) = fixture();
    let review = query_annual_review_at(
        &connection,
        AnnualReviewQueryV1 {
            account_scope: AccountScopeV1::Selected {
                account_ids: vec![account_one],
            },
            year: None,
        },
        now_ms,
    )
    .unwrap();

    assert_eq!(review.year, 2026);
    assert!(review.available_years.contains(&2025));
    assert_eq!(review.chapters.len(), 8);
    assert_eq!(review.chapters[0].id, "footprint");
    assert_eq!(review.chapters[1].id, "rhythm");
    assert_eq!(review.chapters[2].id, "streak");
    assert_eq!(review.chapters[3].id, "authors");
    assert_eq!(review.chapters[4].id, "rewatch");
    assert_eq!(review.chapters[5].id, "progress");
    assert_eq!(review.chapters[6].id, "archiveSnapshot");
    assert_eq!(review.chapters[7].id, "closing");
    assert!(review.chapters[3]
        .items
        .iter()
        .any(|item| item.label == "作者甲"));
    assert!(!review.chapters[3]
        .items
        .iter()
        .any(|item| item.label.is_empty()));
    assert!(review.chapters[4]
        .items
        .iter()
        .any(|item| item.label == "视频一"));
    assert!(review.chapters[5]
        .items
        .iter()
        .any(|item| item.label == "未记录"));
    assert!(review.chapters[6]
        .items
        .iter()
        .any(|item| item.label == "喜欢"));
}

#[test]
fn explicit_empty_year_is_returned_without_fabricating_history() {
    let (connection, account_one, _, now_ms) = fixture();
    let review = query_annual_review_at(
        &connection,
        AnnualReviewQueryV1 {
            account_scope: AccountScopeV1::Selected {
                account_ids: vec![account_one],
            },
            year: Some(2024),
        },
        now_ms,
    )
    .unwrap();
    assert_eq!(review.year, 2024);
    assert!(review.chapters[0].omitted);
    assert!(review.chapters[0].omitted_reason.is_some());
}
