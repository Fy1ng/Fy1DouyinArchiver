use douyin_history_archive::{
    initialize_schema_v4, migrate_schema_to_latest_at_startup, sync::QueueStore,
};
use rusqlite::{params, Connection};

fn fixture() -> Connection {
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
            "INSERT INTO sync_batches
               (id, trigger, scope, created_at, finished_at, status)
             VALUES
               (1, 'manual', 'currentAccountAvailableSources', '2026-08-12T00:00:00Z', NULL, 'queued'),
               (2, 'manual', 'currentAccountAvailableSources', '2026-08-13T00:00:00Z', NULL, 'queued'),
               (3, 'manual', 'allActiveAccounts', '2026-08-14T00:00:00Z', NULL, 'running');

             INSERT INTO sync_batch_targets
               (batch_id, account_id, source, source_instance_key, target_kind, resolution_kind)
             VALUES
               (1, 1, 'history', '', 'task', 'issue'),
               (2, 2, 'likes', '', 'task', 'issue'),
               (3, 1, 'history', '', 'task', 'issue'),
               (3, 2, 'likes', '', 'task', 'issue');

             UPDATE sync_batches
             SET finished_at = '2026-08-12T00:01:00Z', status = 'success'
             WHERE id = 1;
             UPDATE sync_batches
             SET finished_at = '2026-08-13T00:01:00Z', status = 'failed'
             WHERE id = 2;",
        )
        .unwrap();
    connection
}

#[test]
fn sync_batch_history_is_account_scoped_newest_first_and_paginated() {
    let connection = fixture();

    let first = QueueStore::list_batches_for_account(&connection, 1, 1, 0).unwrap();
    assert_eq!(first.total, 2);
    assert_eq!(first.records.len(), 1);
    assert_eq!(first.records[0].id, 3);

    let second = QueueStore::list_batches_for_account(&connection, 1, 1, 1).unwrap();
    assert_eq!(second.total, 2);
    assert_eq!(second.records[0].id, 1);

    let other = QueueStore::list_batches_for_account(&connection, 2, 80, 0).unwrap();
    assert_eq!(other.total, 2);
    assert_eq!(
        other.records.iter().map(|record| record.id).collect::<Vec<_>>(),
        vec![3, 2]
    );
}

#[test]
fn sync_batch_history_validates_account_and_page_size() {
    let connection = fixture();
    assert_eq!(
        QueueStore::list_batches_for_account(&connection, 0, 80, 0).unwrap_err(),
        "invalid_account_id"
    );
    assert_eq!(
        QueueStore::list_batches_for_account(&connection, 1, 0, 0).unwrap_err(),
        "invalid_sync_batch_page_limit"
    );
    assert_eq!(
        QueueStore::list_batches_for_account(&connection, 1, 201, 0).unwrap_err(),
        "invalid_sync_batch_page_limit"
    );
}
