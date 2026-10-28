use douyin_history_archive::{
    initialize_schema_v4, list_content, list_library_content, migrate_schema_to_latest_at_startup,
    search_library_content, settings_store::AccountScopeV1, sync::SyncSourceV1, ContentRecordView,
    ContentSortV1, LibraryContentQueryV1, LibraryContentSortV1, SearchContentQueryV1,
    SearchRemoteStateV1,
};
use rusqlite::{params, Connection};

fn fixture() -> (Connection, i64, i64) {
    let mut connection = Connection::open_in_memory().expect("database");
    initialize_schema_v4(&mut connection).expect("schema");
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
    let account_one = 1;
    let account_two = 2;
    connection
        .execute_batch(
            "INSERT INTO videos
               (id, aweme_id, canonical_url, content_type, title, author_name,
                cover_url, like_count, first_seen_at, last_seen_at)
             VALUES
               (1, 'active-like', 'https://www.douyin.com/video/active-like', 'video',
                '保留的喜欢', '作者甲', 'https://example.com/1.jpg', 12000, 'first', 'last'),
               (2, 'removed-like', 'https://www.douyin.com/video/removed-like', 'video',
                '已移除的喜欢', '作者乙', NULL, 20, 'first', 'last'),
               (3, 'favorite', 'https://www.douyin.com/note/favorite', 'note',
                '收藏图文', '作者丙', 'https://example.com/3.jpg', NULL, 'first', 'last'),
               (4, 'history', 'https://www.douyin.com/video/history', 'video',
                '观看历史', '作者丁', NULL, 88, 'first', 'last');",
        )
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO user_likes
               (id, account_id, video_id, first_seen_at, last_seen_at,
                remote_created_at_ms, remote_removed_at, source_position)
             VALUES
               (11, 1, 1, 'first-like', 'last-like', 2000, NULL, 1),
               (12, 1, 2, 'first-removed', 'last-removed', 1000, 'removed-at', 0),
               (13, 2, 1, 'other-first', 'other-last', 2000, NULL, 0);

             INSERT INTO favorite_items
               (id, account_id, video_id, first_seen_at, last_seen_at,
                remote_created_at_ms, source_position)
             VALUES (21, 1, 3, 'first-favorite', 'last-favorite', 3000, 0);

             INSERT INTO watch_records
               (id, account_id, video_id, history_time_ms, watched_at, watched_date,
                source_position, created_at, updated_at)
             VALUES (31, 1, 4, 4000, 'watched-at', '2026-08-13', 0, 'created', 'updated');",
        )
        .unwrap();
    (connection, account_one, account_two)
}

#[test]
fn archive_content_defaults_to_active_relations_and_can_include_removed() {
    let (connection, account_one, _) = fixture();

    let active = list_content(
        &connection,
        account_one,
        SyncSourceV1::Likes,
        None,
        false,
        ContentSortV1::Default,
        80,
        0,
    )
    .expect("active likes");
    assert_eq!(active.total, 1);
    assert_eq!(active.records[0].aweme_id, "active-like");
    assert_eq!(active.records[0].remote_removed_at, None);
    assert_eq!(
        active.records[0].first_seen_at.as_deref(),
        Some("first-like")
    );

    let all = list_content(
        &connection,
        account_one,
        SyncSourceV1::Likes,
        None,
        true,
        ContentSortV1::Default,
        80,
        0,
    )
    .expect("all likes");
    assert_eq!(all.total, 2);
    assert_eq!(
        all.records
            .iter()
            .map(|record| record.aweme_id.as_str())
            .collect::<Vec<_>>(),
        vec!["removed-like", "active-like"]
    );
    assert_eq!(
        all.records[0].remote_removed_at.as_deref(),
        Some("removed-at")
    );
}

#[test]
fn content_queries_isolate_accounts_search_page_and_preserve_source_contracts() {
    let (connection, account_one, account_two) = fixture();

    let other = list_content(
        &connection,
        account_two,
        SyncSourceV1::Likes,
        Some("作者甲"),
        false,
        ContentSortV1::Default,
        1,
        0,
    )
    .expect("other account");
    assert_eq!(other.total, 1);
    assert_eq!(other.records.len(), 1);

    let favorite = list_content(
        &connection,
        account_one,
        SyncSourceV1::Favorites,
        Some("收藏"),
        false,
        ContentSortV1::Default,
        1,
        0,
    )
    .expect("favorite");
    assert_eq!(favorite.total, 1);
    assert_eq!(favorite.records[0].source, SyncSourceV1::Favorites);
    assert_eq!(favorite.records[0].content_type, "note");
    assert_eq!(favorite.records[0].remote_created_at_ms, Some(3000));

    let history = list_content(
        &connection,
        account_one,
        SyncSourceV1::History,
        None,
        false,
        ContentSortV1::Default,
        80,
        0,
    )
    .expect("history");
    assert_eq!(history.total, 1);
    assert_eq!(history.records[0].history_time_ms, Some(4000));
    assert_eq!(
        history.records[0].watched_date.as_deref(),
        Some("2026-08-13")
    );
    assert_eq!(history.records[0].first_seen_at, None);

    assert_eq!(
        list_content(
            &connection,
            account_one,
            SyncSourceV1::RemoteFolder,
            None,
            false,
            ContentSortV1::Default,
            80,
            0,
        )
        .unwrap_err(),
        "unsupported_content_source"
    );
}

#[test]
fn content_record_json_keeps_nullable_source_specific_fields() {
    let record = ContentRecordView {
        id: 1,
        source: SyncSourceV1::Likes,
        aweme_id: "1".to_string(),
        url: "https://www.douyin.com/video/1".to_string(),
        content_type: "video".to_string(),
        title: None,
        author_name: None,
        cover_url: None,
        cached_cover_url: None,
        like_count: None,
        source_position: None,
        history_time_ms: None,
        watched_at: None,
        watched_date: None,
        watch_progress_rate: None,
        remote_created_at_ms: None,
        first_seen_at: Some("first".to_string()),
        last_seen_at: Some("last".to_string()),
        remote_removed_at: None,
    };

    let json = serde_json::to_value(record).unwrap();
    assert_eq!(json["source"], "likes");
    assert!(json["historyTimeMs"].is_null());
    assert_eq!(json["firstSeenAt"], "first");
}

#[test]
fn content_queries_expose_a_local_cover_url_only_for_cached_entries() {
    let (mut connection, account_one, _) = fixture();
    migrate_schema_to_latest_at_startup(&mut connection).expect("latest schema");
    connection
        .execute(
            "INSERT INTO cover_cache
               (video_id, remote_url, relative_path, byte_size, mime_type, sha256,
                downloaded_at, last_accessed_at)
             VALUES (1, 'https://example.com/1.jpg', 'ac/active-like.jpg', 1,
                     'image/jpeg', ?1, 'downloaded', 'accessed')",
            ["a".repeat(64)],
        )
        .unwrap();

    let likes = list_content(
        &connection,
        account_one,
        SyncSourceV1::Likes,
        None,
        false,
        ContentSortV1::Default,
        80,
        0,
    )
    .unwrap();

    assert_eq!(
        likes.records[0].cached_cover_url.as_deref(),
        Some("http://cover-cache.localhost/1"),
    );
    assert_eq!(
        likes.records[0].cover_url.as_deref(),
        Some("https://example.com/1.jpg"),
    );
}

#[test]
fn content_sort_matrix_is_source_specific_and_applies_before_pagination() {
    let (connection, account_one, _) = fixture();
    connection
        .execute_batch(
            "INSERT INTO videos
               (id, aweme_id, canonical_url, content_type, title, author_name,
                cover_url, like_count, first_seen_at, last_seen_at)
             VALUES
               (5, 'history-older', 'https://www.douyin.com/video/history-older', 'video',
                '甲标题', '作者戊', NULL, 500, 'first', 'last'),
               (6, 'like-newer', 'https://www.douyin.com/video/like-newer', 'video',
                '乙标题', '作者己', NULL, 900, 'first', 'last');

             INSERT INTO watch_records
               (id, account_id, video_id, history_time_ms, watched_at, watched_date,
                source_position, created_at, updated_at)
             VALUES (32, 1, 5, 3000, 'older', '2026-08-12', 1, 'created', 'updated');

             INSERT INTO user_likes
               (id, account_id, video_id, first_seen_at, last_seen_at,
                remote_created_at_ms, remote_removed_at, source_position)
             VALUES (14, 1, 6, 'first-newer', 'last-newer', 5000, NULL, 9);",
        )
        .unwrap();

    let oldest_history = list_content(
        &connection,
        account_one,
        SyncSourceV1::History,
        None,
        false,
        ContentSortV1::WatchedOldest,
        1,
        0,
    )
    .unwrap();
    assert_eq!(oldest_history.records[0].aweme_id, "history-older");

    let most_liked_history = list_content(
        &connection,
        account_one,
        SyncSourceV1::History,
        None,
        false,
        ContentSortV1::LikeCountDesc,
        1,
        0,
    )
    .unwrap();
    assert_eq!(most_liked_history.records[0].aweme_id, "history-older");

    let newest_like = list_content(
        &connection,
        account_one,
        SyncSourceV1::Likes,
        None,
        false,
        ContentSortV1::RemoteNewest,
        1,
        0,
    )
    .unwrap();
    assert_eq!(newest_like.records[0].aweme_id, "like-newer");

    assert_eq!(
        list_content(
            &connection,
            account_one,
            SyncSourceV1::Likes,
            None,
            false,
            ContentSortV1::WatchedNewest,
            80,
            0,
        )
        .unwrap_err(),
        "invalid_content_sort_for_source"
    );
    assert_eq!(
        list_content(
            &connection,
            account_one,
            SyncSourceV1::History,
            None,
            false,
            ContentSortV1::PageOrder,
            80,
            0,
        )
        .unwrap_err(),
        "invalid_content_sort_for_source"
    );
}

#[test]
fn library_content_keeps_account_and_source_relations_and_uses_archive_order() {
    let (connection, account_one, account_two) = fixture();
    connection
        .execute_batch(
            "UPDATE watch_records SET created_at = '2026-08-10T00:00:00Z';
             UPDATE user_likes SET first_seen_at = CASE id
               WHEN 11 THEN '2026-08-13T00:00:00Z'
               WHEN 12 THEN '2026-08-11T00:00:00Z'
               ELSE '2026-08-12T00:00:00Z' END;
             UPDATE favorite_items SET first_seen_at = '2026-08-14T00:00:00Z';
             INSERT INTO favorite_items
               (id, account_id, video_id, first_seen_at, last_seen_at,
                remote_created_at_ms, source_position)
             VALUES (22, 1, 1, '2026-08-15T00:00:00Z', '2026-08-15T00:00:00Z', NULL, 1);",
        )
        .unwrap();

    let result = list_library_content(
        &connection,
        &LibraryContentQueryV1 {
            account_scope: AccountScopeV1::All,
            sources: vec![
                SyncSourceV1::History,
                SyncSourceV1::Likes,
                SyncSourceV1::Favorites,
            ],
            query: None,
            include_remote_removed: false,
            sort: LibraryContentSortV1::ArchivedNewest,
            limit: 80,
            offset: 0,
        },
    )
    .expect("cross-account library");

    assert_eq!(result.total, 5);
    assert_eq!(result.records[0].source, SyncSourceV1::Favorites);
    assert_eq!(result.records[0].aweme_id, "active-like");
    assert_eq!(result.records[0].account_id, account_one);
    assert_eq!(result.records[0].account_key, "sec:uid-a");
    assert_eq!(result.records[0].account_display_name, "uid-a");
    assert_eq!(result.records[0].first_archived_at, "2026-08-15T00:00:00Z");

    let duplicate_relations = result
        .records
        .iter()
        .filter(|record| record.aweme_id == "active-like")
        .map(|record| (record.account_id, record.source))
        .collect::<Vec<_>>();
    assert_eq!(
        duplicate_relations,
        vec![
            (account_one, SyncSourceV1::Favorites),
            (account_one, SyncSourceV1::Likes),
            (account_two, SyncSourceV1::Likes),
        ]
    );
}

#[test]
fn library_content_preserves_real_watch_time_without_forging_archive_times() {
    let (connection, account_one, _) = fixture();
    let result = list_library_content(
        &connection,
        &LibraryContentQueryV1 {
            account_scope: AccountScopeV1::Selected {
                account_ids: vec![account_one],
            },
            sources: vec![SyncSourceV1::History, SyncSourceV1::Likes],
            query: None,
            include_remote_removed: false,
            sort: LibraryContentSortV1::ArchivedNewest,
            limit: 80,
            offset: 0,
        },
    )
    .expect("mixed source library");

    let history = result
        .records
        .iter()
        .find(|record| record.source == SyncSourceV1::History)
        .unwrap();
    assert_eq!(history.history_time_ms, Some(4000));
    assert_eq!(history.watched_date.as_deref(), Some("2026-08-13"));

    let like = result
        .records
        .iter()
        .find(|record| record.source == SyncSourceV1::Likes)
        .unwrap();
    assert_eq!(like.history_time_ms, None);
    assert_eq!(like.watched_at, None);
    assert_eq!(like.watched_date, None);
    assert_eq!(like.first_archived_at, "first-like");
}

#[test]
fn library_content_rejects_invalid_or_stale_account_scopes() {
    let (connection, _, _) = fixture();
    let query = |account_scope| LibraryContentQueryV1 {
        account_scope,
        sources: vec![SyncSourceV1::Likes],
        query: None,
        include_remote_removed: false,
        sort: LibraryContentSortV1::ArchivedNewest,
        limit: 80,
        offset: 0,
    };

    assert_eq!(
        list_library_content(
            &connection,
            &query(AccountScopeV1::Selected {
                account_ids: vec![2, 1],
            }),
        )
        .unwrap_err(),
        "invalid_account_scope"
    );
    assert_eq!(
        list_library_content(
            &connection,
            &query(AccountScopeV1::Selected {
                account_ids: vec![999],
            }),
        )
        .unwrap_err(),
        "account_scope_invalid"
    );
}

#[test]
fn search_content_combines_type_date_remote_and_local_folder_filters() {
    let (connection, account_one, _) = fixture();
    connection
        .execute_batch(
            "UPDATE favorite_items SET first_seen_at = '2026-08-14T00:00:00Z' WHERE id = 21;
             INSERT INTO local_folders
               (id, scope, owner_account_id, name, created_at, updated_at)
             VALUES (1, 'account', 1, '图文收藏', 'created', 'updated');
             INSERT INTO local_folder_items(folder_id, video_id, added_at)
             VALUES (1, 3, 'added');",
        )
        .unwrap();

    let result = search_library_content(
        &connection,
        &SearchContentQueryV1 {
            account_scope: AccountScopeV1::Selected {
                account_ids: vec![account_one],
            },
            sources: vec![
                SyncSourceV1::History,
                SyncSourceV1::Likes,
                SyncSourceV1::Favorites,
            ],
            query: None,
            content_types: vec!["note".to_string()],
            date_from: Some("2026-08-01".to_string()),
            date_to: Some("2026-08-31".to_string()),
            remote_state: SearchRemoteStateV1::Active,
            local_folder_id: Some(1),
            sort: LibraryContentSortV1::ArchivedNewest,
            limit: 80,
            offset: 0,
        },
    )
    .expect("filtered search");

    assert_eq!(result.total, 1);
    assert_eq!(result.records[0].aweme_id, "favorite");
    assert_eq!(result.records[0].content_type, "note");
}

#[test]
fn search_content_can_show_only_remote_removed_relations() {
    let (connection, account_one, _) = fixture();
    let result = search_library_content(
        &connection,
        &SearchContentQueryV1 {
            account_scope: AccountScopeV1::Selected {
                account_ids: vec![account_one],
            },
            sources: vec![SyncSourceV1::Likes, SyncSourceV1::Favorites],
            query: None,
            content_types: Vec::new(),
            date_from: None,
            date_to: None,
            remote_state: SearchRemoteStateV1::Removed,
            local_folder_id: None,
            sort: LibraryContentSortV1::ArchivedNewest,
            limit: 80,
            offset: 0,
        },
    )
    .expect("removed search");

    assert_eq!(result.total, 1);
    assert_eq!(result.records[0].aweme_id, "removed-like");
}

#[test]
fn search_content_rejects_invalid_dates_types_and_folders() {
    let (connection, _, _) = fixture();
    let mut query = SearchContentQueryV1 {
        account_scope: AccountScopeV1::All,
        sources: vec![SyncSourceV1::History],
        query: None,
        content_types: vec!["live".to_string()],
        date_from: None,
        date_to: None,
        remote_state: SearchRemoteStateV1::All,
        local_folder_id: None,
        sort: LibraryContentSortV1::ArchivedNewest,
        limit: 80,
        offset: 0,
    };
    assert_eq!(
        search_library_content(&connection, &query).unwrap_err(),
        "invalid_content_types"
    );

    query.content_types.clear();
    query.date_from = Some("2026-99-99".to_string());
    assert_eq!(
        search_library_content(&connection, &query).unwrap_err(),
        "invalid_content_date"
    );

    query.date_from = None;
    query.local_folder_id = Some(999);
    assert_eq!(
        search_library_content(&connection, &query).unwrap_err(),
        "local_folder_not_found"
    );
}
