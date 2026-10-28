use douyin_history_archive::{
    add_local_folder_item, create_local_folder, initialize_schema_v4, list_local_folder_content,
    list_local_folders, list_remote_folder_content, list_remote_folders, remove_local_folder_item,
    rename_local_folder, set_local_folder_scope, LocalFolderScopeV1,
};
use rusqlite::{params, Connection};

fn fixture() -> Connection {
    let mut connection = Connection::open_in_memory().expect("database");
    initialize_schema_v4(&mut connection).expect("schema");
    for (id, uid, nickname) in [(1, "uid-a", "账号甲"), (2, "uid-b", "账号乙")] {
        connection
            .execute(
                "INSERT INTO accounts
                 (id, uid, sec_uid, profile_key, nickname, status, created_at, updated_at)
                 VALUES (?1, ?2, ?2, printf('profile-%d', ?1), ?3, 'active', 'created', 'updated')",
                params![id, uid, nickname],
            )
            .unwrap();
    }
    connection
        .execute_batch(
            "INSERT INTO videos
               (id, aweme_id, canonical_url, content_type, title, author_name,
                cover_url, like_count, first_seen_at, last_seen_at)
             VALUES
               (1, 'shared', 'https://www.douyin.com/video/shared', 'video',
                '共享视频', '作者甲', 'https://example.com/shared.jpg', 12, 'first', 'last'),
               (2, 'only-a', 'https://www.douyin.com/note/only-a', 'note',
                '甲的图文', '作者乙', NULL, NULL, 'first', 'last'),
               (3, 'remote-removed', 'https://www.douyin.com/video/remote-removed', 'video',
                '远端移除', '作者丙', NULL, 1, 'first', 'last');

             INSERT INTO user_likes
               (account_id, video_id, first_seen_at, last_seen_at, source_position)
             VALUES (1, 1, 'first-a', 'last-a', 0),
                    (2, 1, 'first-b', 'last-b', 0),
                    (1, 2, 'first-note', 'last-note', 1);",
        )
        .unwrap();
    connection
}

#[test]
fn local_folders_enforce_scoped_names_and_account_visibility() {
    let mut connection = fixture();
    let account = create_local_folder(
        &mut connection,
        1,
        "  稍后看  ",
        LocalFolderScopeV1::Account,
    )
    .unwrap();
    assert_eq!(account.name, "稍后看");
    assert_eq!(account.owner_account_id, Some(1));

    let aggregate =
        create_local_folder(&mut connection, 1, "稍后看", LocalFolderScopeV1::Aggregate).unwrap();
    assert_eq!(aggregate.owner_account_id, None);
    assert_eq!(
        create_local_folder(&mut connection, 1, "稍后看", LocalFolderScopeV1::Account,)
            .unwrap_err(),
        "local_folder_name_conflict"
    );

    assert_eq!(list_local_folders(&connection, 1).unwrap().len(), 2);
    let account_two = list_local_folders(&connection, 2).unwrap();
    assert_eq!(account_two.len(), 1);
    assert_eq!(account_two[0].id, aggregate.id);
    assert_eq!(
        rename_local_folder(&mut connection, 2, account.id, "越权").unwrap_err(),
        "local_folder_not_found"
    );
}

#[test]
fn aggregate_folder_deduplicates_items_and_keeps_source_accounts() {
    let mut connection = fixture();
    let folder = create_local_folder(
        &mut connection,
        1,
        "跨账号精选",
        LocalFolderScopeV1::Aggregate,
    )
    .unwrap();

    let first = add_local_folder_item(&mut connection, 1, folder.id, "shared").unwrap();
    assert!(first.inserted_item);
    assert!(first.inserted_provenance);
    let second = add_local_folder_item(&mut connection, 2, folder.id, "shared").unwrap();
    assert!(!second.inserted_item);
    assert!(second.inserted_provenance);
    assert_eq!(
        add_local_folder_item(&mut connection, 2, folder.id, "only-a").unwrap_err(),
        "account_content_not_found"
    );

    let content = list_local_folder_content(&connection, 1, folder.id, None, 80, 0).unwrap();
    assert_eq!(content.total, 1);
    assert_eq!(
        content.records[0]
            .source_accounts
            .iter()
            .map(|account| account.id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(remove_local_folder_item(&mut connection, 2, folder.id, "shared").unwrap());
    assert_eq!(
        list_local_folder_content(&connection, 1, folder.id, None, 80, 0)
            .unwrap()
            .total,
        0
    );
}

#[test]
fn scope_conversion_preserves_and_clears_provenance_deterministically() {
    let mut connection = fixture();
    let folder =
        create_local_folder(&mut connection, 1, "账号收藏", LocalFolderScopeV1::Account).unwrap();
    add_local_folder_item(&mut connection, 1, folder.id, "only-a").unwrap();

    let aggregate = set_local_folder_scope(
        &mut connection,
        1,
        folder.id,
        LocalFolderScopeV1::Aggregate,
        None,
    )
    .unwrap();
    assert_eq!(aggregate.scope, LocalFolderScopeV1::Aggregate);
    let content = list_local_folder_content(&connection, 2, folder.id, None, 80, 0).unwrap();
    assert_eq!(content.records[0].source_accounts[0].id, 1);

    let transferred = set_local_folder_scope(
        &mut connection,
        2,
        folder.id,
        LocalFolderScopeV1::Account,
        Some(2),
    )
    .unwrap();
    assert_eq!(transferred.owner_account_id, Some(2));
    assert_eq!(
        list_local_folder_content(&connection, 1, folder.id, None, 80, 0).unwrap_err(),
        "local_folder_not_found"
    );
    let provenance: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM local_folder_item_accounts WHERE folder_id = ?1",
            [folder.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(provenance, 0);
}

#[test]
fn remote_folders_and_items_default_to_active_and_can_include_removed() {
    let connection = fixture();
    connection
        .execute_batch(
            "INSERT INTO remote_folders
               (id, account_id, remote_folder_id, name, first_seen_at, last_seen_at)
             VALUES (10, 1, 'folder-a', '旅行', 'folder-first', 'folder-last');
             INSERT INTO remote_folders
               (id, account_id, remote_folder_id, name, first_seen_at, last_seen_at,
                remote_removed_at)
             VALUES (11, 1, 'folder-removed', '已删除分类', 'first', 'last', 'removed');
             INSERT INTO remote_folder_items
               (folder_id, video_id, first_seen_at, last_seen_at)
             VALUES (10, 1, 'item-first', 'item-last');
             INSERT INTO remote_folder_items
               (folder_id, video_id, first_seen_at, last_seen_at, remote_removed_at)
             VALUES (10, 3, 'removed-first', 'removed-last', 'item-removed');",
        )
        .unwrap();

    let active_folders = list_remote_folders(&connection, 1, false).unwrap();
    assert_eq!(active_folders.len(), 1);
    assert_eq!(active_folders[0].item_count, 2);
    assert_eq!(active_folders[0].active_item_count, 1);
    assert_eq!(list_remote_folders(&connection, 1, true).unwrap().len(), 2);

    let active_items =
        list_remote_folder_content(&connection, 1, "folder-a", None, false, 80, 0).unwrap();
    assert_eq!(active_items.total, 1);
    assert_eq!(active_items.records[0].aweme_id, "shared");
    let all_items =
        list_remote_folder_content(&connection, 1, "folder-a", None, true, 80, 0).unwrap();
    assert_eq!(all_items.total, 2);
    assert!(all_items
        .records
        .iter()
        .any(|record| record.remote_removed_at.is_some()));
}

#[test]
fn local_folder_names_and_owner_contracts_are_strict() {
    let mut connection = fixture();
    assert_eq!(
        create_local_folder(&mut connection, 1, "\n", LocalFolderScopeV1::Account).unwrap_err(),
        "invalid_local_folder_name"
    );
    let folder =
        create_local_folder(&mut connection, 1, "转换测试", LocalFolderScopeV1::Account).unwrap();
    assert_eq!(
        set_local_folder_scope(
            &mut connection,
            1,
            folder.id,
            LocalFolderScopeV1::Aggregate,
            Some(1),
        )
        .unwrap_err(),
        "aggregate_folder_owner_forbidden"
    );
    assert_eq!(
        set_local_folder_scope(
            &mut connection,
            1,
            folder.id,
            LocalFolderScopeV1::Account,
            None,
        )
        .unwrap_err(),
        "local_folder_owner_required"
    );
}
