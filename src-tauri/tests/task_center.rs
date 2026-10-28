use douyin_history_archive::{
    initialize_schema_v4, list_task_summaries, migrate_schema_to_latest_at_startup,
    settings_store::AccountScopeV1,
};
use rusqlite::Connection;

fn fixture() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    initialize_schema_v4(&mut connection).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    connection
        .execute_batch(
            "INSERT INTO accounts (id, uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES (1, 'uid-a', 'sec-a', 'profile-a', 'active', 'created', 'updated'),
                    (2, 'uid-b', 'sec-b', 'profile-b', 'active', 'created', 'updated');
             INSERT INTO sync_batches (id, trigger, scope, created_at, finished_at, status)
             VALUES (10, 'manual', 'currentAccountAvailableSources', '2026-08-19T10:00:00Z', NULL, 'queued'),
                    (11, 'manual', 'allActiveAccounts', '2026-08-19T12:00:00Z', NULL, 'running');
             INSERT INTO sync_batch_targets (batch_id, account_id, source, source_instance_key, target_kind, resolution_kind)
             VALUES (10, 1, 'history', '', 'task', 'job'),
                    (11, 1, 'history', '', 'task', 'job'),
                    (11, 2, 'likes', '', 'task', 'job');
             UPDATE sync_batches SET status = 'success', finished_at = '2026-08-19T10:01:00Z' WHERE id = 10;
             INSERT INTO download_jobs (id, account_id, profile_key, created_at, completed_at, selection_scope, destination_root, requested_quality, state, candidate_count, completed_count, failed_count)
             VALUES (20, 2, 'profile-b', '2026-08-19T11:00:00Z', NULL, 'all', 'D:/media', 'highest', 'running', 3, 1, 0);",
        )
        .unwrap();
    connection
}

#[test]
fn unified_tasks_sort_by_time_and_preserve_aggregate_identity() {
    let connection = fixture();
    let page = list_task_summaries(&connection, &AccountScopeV1::All, 20, 0).unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(
        page.records
            .iter()
            .map(|task| task.key.as_str())
            .collect::<Vec<_>>(),
        vec!["sync:11", "download:20", "sync:10"]
    );
    assert_eq!(page.records[0].account_id, None);
    assert_eq!(page.records[0].source, None);
    assert!(page.records[0].can_cancel);
}

#[test]
fn unified_tasks_reject_stale_scope_and_filter_selected_accounts() {
    let connection = fixture();
    let selected = list_task_summaries(
        &connection,
        &AccountScopeV1::Selected {
            account_ids: vec![2],
        },
        20,
        0,
    )
    .unwrap();
    assert_eq!(
        selected
            .records
            .iter()
            .map(|task| task.key.as_str())
            .collect::<Vec<_>>(),
        vec!["sync:11", "download:20"]
    );
    assert_eq!(
        list_task_summaries(
            &connection,
            &AccountScopeV1::Selected {
                account_ids: vec![99]
            },
            20,
            0
        )
        .unwrap_err(),
        "account_scope_invalid"
    );
}
