use chrono::DateTime;
use douyin_history_archive::{
    initialize_schema_v4, migrate_schema_to_latest_at_startup, query_overview_at,
    OverviewRangeV1, OverviewSourceFilterV1,
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
               (1, 'shared', 'https://www.douyin.com/video/shared', 'video', '共享内容', '作者甲', 'first', 'last'),
               (2, 'old-history', 'https://www.douyin.com/video/old-history', 'video', '旧历史', '作者甲', 'first', 'last'),
               (3, 'undated-favorite', 'https://www.douyin.com/note/undated-favorite', 'note', '无日期收藏', '作者乙', 'first', 'last'),
               (4, 'removed-like', 'https://www.douyin.com/video/removed-like', 'video', '远端移除', '作者丙', 'first', 'last'),
               (5, 'other-account', 'https://www.douyin.com/video/other-account', 'video', '其他账号', '作者丁', 'first', 'last');

             INSERT INTO watch_records
               (account_id, video_id, history_time_ms, watched_at, watched_date,
                source_position, created_at, updated_at)
             VALUES
               (1, 1, 1786665600000, '2026-08-14T00:00:00Z', '2026-08-14', 0, 'created', 'updated'),
               (1, 2, 1782864000000, '2026-07-01T00:00:00Z', '2026-07-01', 1, 'created', 'updated'),
               (2, 5, 1786665600000, '2026-08-14T00:00:00Z', '2026-08-14', 0, 'created', 'updated');

             INSERT INTO user_likes
               (account_id, video_id, first_seen_at, last_seen_at,
                remote_created_at_ms, remote_removed_at, source_position)
             VALUES
               (1, 1, 'first', 'last', 1786669200000, NULL, 0),
               (1, 4, 'first', 'last', 1786672800000, 'removed', 1),
               (2, 5, 'first', 'last', 1786669200000, NULL, 0);

             INSERT INTO favorite_items
               (account_id, video_id, first_seen_at, last_seen_at,
                remote_created_at_ms, remote_removed_at, source_position)
             VALUES (1, 3, 'first', 'last', NULL, NULL, 0);

             INSERT INTO sync_jobs
               (account_id, source, source_instance_key, trigger, queued_at,
                started_at, finished_at, status, inserted_records)
             VALUES
               (1, 'history', '', 'manual', '2026-08-14T01:00:00Z',
                '2026-08-14T01:00:00Z', '2026-08-14T01:01:00Z', 'success', 2),
               (1, 'likes', '', 'automatic', '2026-08-14T02:00:00Z',
                '2026-08-14T02:00:00Z', '2026-08-14T02:01:00Z', 'failed', 0),
               (2, 'history', '', 'manual', '2026-08-14T03:00:00Z',
                '2026-08-14T03:00:00Z', '2026-08-14T03:01:00Z', 'success', 1);",
        )
        .unwrap();
    let now_ms = DateTime::parse_from_rfc3339("2026-08-14T12:00:00Z")
        .unwrap()
        .timestamp_millis();
    (connection, 1, 2, now_ms)
}

#[test]
fn overview_is_account_scoped_and_deduplicates_content_across_sources() {
    let (connection, account_one, _, now_ms) = fixture();

    let view = query_overview_at(
        &connection,
        account_one,
        OverviewSourceFilterV1::All,
        OverviewRangeV1::All,
        false,
        now_ms,
    )
    .unwrap();

    assert_eq!(view.unique_content_count, 3);
    assert_eq!(view.source_counts.history, 2);
    assert_eq!(view.source_counts.likes, 1);
    assert_eq!(view.source_counts.favorites, 1);
    assert_eq!(view.undated_archive_count, 1);
    assert_eq!(view.top_authors[0].author_name, "作者甲");
    assert_eq!(view.top_authors[0].content_count, 2);
    assert_eq!(view.content_types[0].content_type, "video");
    assert_eq!(view.content_types[0].content_count, 2);
    assert_eq!(view.recent_sync.len(), 2);
    assert!(view.recent_sync.iter().all(|item| item.account_id == account_one));
}

#[test]
fn finite_overview_ranges_exclude_undated_archives_without_fabricating_dates() {
    let (connection, account_one, _, now_ms) = fixture();

    let view = query_overview_at(
        &connection,
        account_one,
        OverviewSourceFilterV1::All,
        OverviewRangeV1::Last7Days,
        false,
        now_ms,
    )
    .unwrap();

    assert_eq!(view.unique_content_count, 1);
    assert_eq!(view.source_counts.history, 1);
    assert_eq!(view.source_counts.likes, 1);
    assert_eq!(view.source_counts.favorites, 0);
    assert_eq!(view.undated_archive_count, 1);
    assert_eq!(view.trend.len(), 1);
    assert_eq!(view.trend[0].date, "2026-08-14");
    assert_eq!(view.trend[0].history, 1);
    assert_eq!(view.trend[0].likes, 1);
    assert_eq!(view.trend[0].favorites, 0);
}

#[test]
fn overview_remote_removed_and_source_filters_are_explicit() {
    let (connection, account_one, _, now_ms) = fixture();

    let active_likes = query_overview_at(
        &connection,
        account_one,
        OverviewSourceFilterV1::Likes,
        OverviewRangeV1::All,
        false,
        now_ms,
    )
    .unwrap();
    assert_eq!(active_likes.unique_content_count, 1);
    assert_eq!(active_likes.source_counts.likes, 1);

    let all_likes = query_overview_at(
        &connection,
        account_one,
        OverviewSourceFilterV1::Likes,
        OverviewRangeV1::All,
        true,
        now_ms,
    )
    .unwrap();
    assert_eq!(all_likes.unique_content_count, 2);
    assert_eq!(all_likes.source_counts.likes, 2);
}

#[test]
fn overview_rejects_invalid_accounts_and_future_reference_times() {
    let (connection, _, _, now_ms) = fixture();
    assert_eq!(
        query_overview_at(
            &connection,
            0,
            OverviewSourceFilterV1::All,
            OverviewRangeV1::All,
            false,
            now_ms,
        )
        .unwrap_err(),
        "invalid_account_id"
    );
    assert_eq!(
        query_overview_at(
            &connection,
            1,
            OverviewSourceFilterV1::All,
            OverviewRangeV1::Last7Days,
            false,
            0,
        )
        .unwrap_err(),
        "invalid_overview_reference_time"
    );
}
