use douyin_history_archive::{
    configure_connection, list_notifications, mark_all_notifications_read, mark_notification_read,
    migrate_schema_to_latest_at_startup, resolve_notification, upsert_notification,
    NotificationInsertV1, NotificationSeverityV1,
};
use rusqlite::Connection;

fn user_version(connection: &Connection) -> i64 {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn startup_migrates_schema_v7_to_v8_and_creates_notification_indexes() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    connection
        .execute_batch("DROP TABLE notifications; PRAGMA user_version = 7;")
        .unwrap();

    migrate_schema_to_latest_at_startup(&mut connection).unwrap();

    assert_eq!(user_version(&connection), 8);
    for object in [
        ("table", "notifications"),
        ("index", "idx_notifications_unresolved_dedupe"),
        ("index", "idx_notifications_created"),
        ("index", "idx_notifications_unread"),
    ] {
        assert_eq!(
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
                    [object.0, object.1],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "missing {} {}",
            object.0,
            object.1
        );
    }
}

#[test]
fn unresolved_notifications_are_deduplicated_until_resolved() {
    let mut connection = Connection::open_in_memory().unwrap();
    configure_connection(&connection).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    connection
        .execute(
            "INSERT INTO accounts(uid, profile_key, status, created_at, updated_at)
             VALUES ('notification-user', '11111111111111111111111111111111', 'active', 'now', 'now')",
            [],
        )
        .unwrap();
    let input = NotificationInsertV1 {
        kind: "loginExpired".to_string(),
        severity: NotificationSeverityV1::Error,
        dedupe_key: "account:1:login".to_string(),
        title: "登录已失效".to_string(),
        body: "请重新登录".to_string(),
        account_id: Some(1),
        related_type: Some("account".to_string()),
        related_id: Some("1".to_string()),
        created_at: "2026-08-19T12:00:00.000Z".to_string(),
    };

    let first = upsert_notification(&connection, input.clone()).unwrap();
    let second = upsert_notification(&connection, input).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(
        list_notifications(&connection, false, 50, 0).unwrap().total,
        1
    );

    mark_notification_read(&connection, first.id, true, "2026-08-19T12:01:00.000Z").unwrap();
    assert_eq!(
        list_notifications(&connection, false, 50, 0)
            .unwrap()
            .records[0]
            .read_at
            .as_deref(),
        Some("2026-08-19T12:01:00.000Z")
    );

    resolve_notification(&connection, first.id, "2026-08-19T12:02:00.000Z").unwrap();
    let third = upsert_notification(
        &connection,
        NotificationInsertV1 {
            kind: "loginExpired".to_string(),
            severity: NotificationSeverityV1::Error,
            dedupe_key: "account:1:login".to_string(),
            title: "登录已失效".to_string(),
            body: "请重新登录".to_string(),
            account_id: Some(1),
            related_type: Some("account".to_string()),
            related_id: Some("1".to_string()),
            created_at: "2026-08-19T12:03:00.000Z".to_string(),
        },
    )
    .unwrap();
    assert_ne!(third.id, first.id);
    assert_eq!(
        list_notifications(&connection, false, 50, 0).unwrap().total,
        1
    );
    assert_eq!(
        list_notifications(&connection, true, 50, 0).unwrap().total,
        2
    );
}

#[test]
fn notification_read_operations_are_atomic_and_paginated() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    for index in 0..3 {
        upsert_notification(
            &connection,
            NotificationInsertV1 {
                kind: "backupFailed".to_string(),
                severity: NotificationSeverityV1::Warning,
                dedupe_key: format!("backup:{index}"),
                title: "备份失败".to_string(),
                body: "检查备份目录".to_string(),
                account_id: None,
                related_type: None,
                related_id: None,
                created_at: format!("2026-08-19T12:0{index}:00.000Z"),
            },
        )
        .unwrap();
    }
    assert_eq!(
        list_notifications(&connection, false, 2, 0)
            .unwrap()
            .records
            .len(),
        2
    );
    assert_eq!(
        list_notifications(&connection, false, 2, 0).unwrap().total,
        3
    );
    mark_all_notifications_read(&connection, "2026-08-19T13:00:00.000Z").unwrap();
    assert!(list_notifications(&connection, false, 50, 0)
        .unwrap()
        .records
        .iter()
        .all(|item| item.read_at.is_some()));
}
