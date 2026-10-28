use douyin_history_archive::migrate_schema_to_v5_at_startup;
use douyin_history_archive::sync::{
    BatchJobRoleV1, NewSyncBatchIssueKindV1, NewSyncBatchIssueV1, NewSyncJobV1, QueueStore,
    SyncBatchEntryV1, SyncBatchIssuePhaseV1, SyncBatchStatusV1, SyncJobIssueSeverityV1,
    SyncJobIssueV1, SyncJobStatusV1, SyncScopeV1, SyncSourceV1, SyncTriggerV1, TaskKeyV1,
    TerminalJobUpdateV1,
};
use rusqlite::{params, Connection};

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    connection
}

fn hash(character: char) -> String {
    std::iter::repeat(character).take(64).collect()
}

fn insert_account(connection: &Connection, suffix: usize) -> i64 {
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'active', 'x', 'x')",
            params![
                format!("uid-{suffix}"),
                format!("sec-{suffix}"),
                format!("{suffix:032x}")
            ],
        )
        .expect("account");
    let account_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)",
            [account_id],
        )
        .expect("registry state");
    account_id
}

fn job(
    account_id: i64,
    source: SyncSourceV1,
    source_instance_key: impl Into<String>,
    trigger: SyncTriggerV1,
    queued_at: &str,
    scheduled_for: Option<&str>,
    marker: char,
) -> NewSyncJobV1 {
    NewSyncJobV1 {
        task_key: TaskKeyV1 {
            account_id,
            source,
            source_instance_key: source_instance_key.into(),
        },
        trigger,
        queued_at: queued_at.to_string(),
        scheduled_for: scheduled_for.map(str::to_string),
        settings_revision: 1,
        config_sha256: (trigger == SyncTriggerV1::Automatic).then(|| hash('f')),
        payload_json: "{}".to_string(),
        payload_sha256: hash(marker),
        equivalence_sha256: hash(marker),
    }
}

fn terminal(status: SyncJobStatusV1, finished_at: &str) -> TerminalJobUpdateV1 {
    TerminalJobUpdateV1 {
        status,
        finished_at: finished_at.to_string(),
        inserted_videos: 0,
        inserted_records: 0,
        error_code: None,
        error_message: None,
    }
}

fn enqueue_test_job(
    connection: &mut Connection,
    account_id: i64,
    source: SyncSourceV1,
    instance: &str,
    marker: char,
) -> douyin_history_archive::sync::StoredSyncJobV1 {
    QueueStore::enqueue_job(
        connection,
        &job(
            account_id,
            source,
            instance,
            SyncTriggerV1::Manual,
            "2026-08-12T02:00:00.000Z",
            None,
            marker,
        ),
    )
    .expect("job")
}

#[test]
fn queue_sequence_sorting_cas_and_cancellation_anchors_are_durable() {
    let mut connection = database();
    let first_account = insert_account(&connection, 1);
    let second_account = insert_account(&connection, 2);

    let automatic_late = QueueStore::enqueue_job(
        &mut connection,
        &job(
            second_account,
            SyncSourceV1::History,
            "",
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:00.000Z",
            Some("2026-08-12T04:00:00.000Z"),
            'a',
        ),
    )
    .expect("automatic late");
    let manual_first = QueueStore::enqueue_job(
        &mut connection,
        &job(
            first_account,
            SyncSourceV1::History,
            "",
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:01.000Z",
            None,
            'b',
        ),
    )
    .expect("manual first");
    let automatic_early = QueueStore::enqueue_job(
        &mut connection,
        &job(
            first_account,
            SyncSourceV1::RemoteFolder,
            "folder-a",
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:02.000Z",
            Some("2026-08-12T03:00:00.000Z"),
            'c',
        ),
    )
    .expect("automatic early");
    let manual_second = QueueStore::enqueue_job(
        &mut connection,
        &job(
            second_account,
            SyncSourceV1::RemoteFolder,
            "folder-b",
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:03.000Z",
            None,
            'd',
        ),
    )
    .expect("manual second");

    assert_eq!(
        QueueStore::list_queued_jobs(&connection)
            .expect("queue")
            .into_iter()
            .map(|item| item.snapshot.id)
            .collect::<Vec<_>>(),
        vec![
            manual_first.snapshot.id,
            manual_second.snapshot.id,
            automatic_early.snapshot.id,
            automatic_late.snapshot.id,
        ]
    );

    QueueStore::claim_job(
        &mut connection,
        manual_first.snapshot.id,
        "2026-08-12T00:01:00.000Z",
    )
    .expect("claim first");
    assert_eq!(
        QueueStore::claim_job(
            &mut connection,
            manual_second.snapshot.id,
            "2026-08-12T00:01:01.000Z",
        )
        .unwrap_err(),
        "sync_already_running"
    );

    QueueStore::cancel_queued_job(
        &mut connection,
        manual_second.snapshot.id,
        "2026-08-12T00:01:02.000Z",
        "cancelled_before_start",
    )
    .expect("cancel queued");
    assert!(
        QueueStore::latest_terminal_anchor(&connection, &manual_second.snapshot.task_key,)
            .expect("anchor")
            .is_none()
    );

    QueueStore::finish_running_job(
        &mut connection,
        manual_first.snapshot.id,
        &terminal(SyncJobStatusV1::Cancelled, "2026-08-12T00:01:03.000Z"),
    )
    .expect("cancel running");
    let anchor = QueueStore::latest_terminal_anchor(&connection, &manual_first.snapshot.task_key)
        .expect("anchor")
        .expect("started anchor");
    assert_eq!(anchor.status, SyncJobStatusV1::Cancelled);
    assert_eq!(anchor.started_at, "2026-08-12T00:01:00.000Z");

    connection
        .execute(
            "UPDATE sync_queue_state SET next_queue_seq = ?1 WHERE singleton = 1",
            [i64::MAX],
        )
        .expect("exhaust sequence");
    assert_eq!(
        QueueStore::enqueue_job(
            &mut connection,
            &job(
                first_account,
                SyncSourceV1::Likes,
                "",
                SyncTriggerV1::Manual,
                "2026-08-12T00:02:00.000Z",
                None,
                'e',
            ),
        )
        .unwrap_err(),
        "sync_queue_sequence_exhausted"
    );
}

#[test]
fn job_transitions_refresh_batch_snapshots_and_incomplete_issues_force_partial() {
    let mut connection = database();
    let account_id = insert_account(&connection, 10);
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T01:00:00.000Z",
    )
    .expect("batch");
    let stored = QueueStore::enqueue_job(
        &mut connection,
        &job(
            account_id,
            SyncSourceV1::History,
            "",
            SyncTriggerV1::Manual,
            "2026-08-12T01:00:01.000Z",
            None,
            'a',
        ),
    )
    .expect("job");
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        stored.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-12T01:00:02.000Z",
    )
    .expect("attach");
    QueueStore::claim_job(
        &mut connection,
        stored.snapshot.id,
        "2026-08-12T01:00:03.000Z",
    )
    .expect("running");
    QueueStore::append_job_issue(
        &mut connection,
        stored.snapshot.id,
        &SyncJobIssueV1 {
            phase: "collection".to_string(),
            code: "unconfirmed_end".to_string(),
            severity: SyncJobIssueSeverityV1::Incomplete,
            message: Some("incomplete".to_string()),
        },
        "2026-08-12T01:00:04.000Z",
    )
    .expect("issue");
    let finished = QueueStore::finish_running_job(
        &mut connection,
        stored.snapshot.id,
        &terminal(SyncJobStatusV1::Success, "2026-08-12T01:00:05.000Z"),
    )
    .expect("finish");
    assert_eq!(finished.snapshot.status, SyncJobStatusV1::Partial);

    let batch = QueueStore::get_batch(&connection, batch_id).expect("batch view");
    assert_eq!(batch.status, SyncBatchStatusV1::Partial);
    assert_eq!(
        batch.finished_at.as_deref(),
        Some("2026-08-12T01:00:05.000Z")
    );
    let page = QueueStore::list_batch_entries(&connection, batch_id, None, 200).expect("entries");
    let SyncBatchEntryV1::Job(job) = &page.entries[0] else {
        panic!("expected job entry");
    };
    assert_eq!(job.status, SyncJobStatusV1::Partial);
    assert_eq!(job.issues.len(), 1);
}

#[test]
fn uninstall_recovery_terminalizes_all_active_jobs_in_one_batch_consistent_transaction() {
    let mut connection = database();
    let account_id = insert_account(&connection, 11);
    let running_batch = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-13T01:00:00.000Z",
    )
    .unwrap();
    let running = enqueue_test_job(&mut connection, account_id, SyncSourceV1::History, "", 'a');
    QueueStore::attach_job_to_batch(
        &mut connection,
        running_batch,
        running.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-13T01:00:01.000Z",
    )
    .unwrap();
    QueueStore::claim_job(
        &mut connection,
        running.snapshot.id,
        "2026-08-13T01:00:02.000Z",
    )
    .unwrap();

    let queued_batch = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-13T01:01:00.000Z",
    )
    .unwrap();
    let queued = enqueue_test_job(&mut connection, account_id, SyncSourceV1::Likes, "", 'b');
    QueueStore::attach_job_to_batch(
        &mut connection,
        queued_batch,
        queued.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-13T01:01:01.000Z",
    )
    .unwrap();

    let recovered = QueueStore::recover_for_uninstall(
        &mut connection,
        "2026-08-13T01:02:00.000Z",
        "uninstall_runtime_stopped",
    )
    .unwrap();

    assert_eq!(recovered.running_job_ids, vec![running.snapshot.id]);
    assert_eq!(recovered.queued_job_ids, vec![queued.snapshot.id]);
    let running = QueueStore::get_stored_job(&connection, running.snapshot.id).unwrap();
    assert_eq!(running.snapshot.status, SyncJobStatusV1::Cancelled);
    assert_eq!(
        running.error_code.as_deref(),
        Some("uninstall_runtime_stopped")
    );
    let queued = QueueStore::get_stored_job(&connection, queued.snapshot.id).unwrap();
    assert_eq!(queued.snapshot.status, SyncJobStatusV1::Cancelled);
    assert_eq!(
        queued.error_code.as_deref(),
        Some("uninstall_runtime_stopped")
    );
    assert_eq!(
        QueueStore::get_batch(&connection, running_batch)
            .unwrap()
            .status,
        SyncBatchStatusV1::Cancelled
    );
    assert_eq!(
        QueueStore::get_batch(&connection, queued_batch)
            .unwrap()
            .status,
        SyncBatchStatusV1::Cancelled
    );
    QueueStore::assert_no_active_jobs(&connection).unwrap();
}

#[test]
fn uninstall_recovery_lists_terminalized_jobs_for_a_later_file_cleanup_retry() {
    let mut connection = database();
    let account_id = insert_account(&connection, 12);
    let running = enqueue_test_job(&mut connection, account_id, SyncSourceV1::History, "", 'a');
    QueueStore::claim_job(
        &mut connection,
        running.snapshot.id,
        "2026-08-13T02:00:00.000Z",
    )
    .unwrap();
    let unrelated = enqueue_test_job(&mut connection, account_id, SyncSourceV1::Likes, "", 'b');

    QueueStore::recover_for_uninstall(
        &mut connection,
        "2026-08-13T02:01:00.000Z",
        "uninstall_runtime_stopped",
    )
    .unwrap();
    connection
        .execute(
            "UPDATE sync_jobs SET error_code = 'different_reason' WHERE id = ?1",
            [unrelated.snapshot.id],
        )
        .unwrap();

    assert_eq!(
        QueueStore::list_uninstall_recovery_job_keys(&connection, "uninstall_runtime_stopped")
            .unwrap(),
        vec![(account_id, running.snapshot.id)]
    );
}

#[test]
fn batch_status_matrix_matches_jobs_and_blocking_issues() {
    let mut connection = database();
    let account_id = insert_account(&connection, 20);

    let queued_batch = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T02:00:00.000Z",
    )
    .unwrap();
    let queued = enqueue_test_job(&mut connection, account_id, SyncSourceV1::History, "", 'a');
    QueueStore::attach_job_to_batch(
        &mut connection,
        queued_batch,
        queued.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-12T02:00:01.000Z",
    )
    .unwrap();
    assert_eq!(
        QueueStore::get_batch(&connection, queued_batch)
            .unwrap()
            .status,
        SyncBatchStatusV1::Queued
    );

    let running_batch = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T02:01:00.000Z",
    )
    .unwrap();
    let running = enqueue_test_job(&mut connection, account_id, SyncSourceV1::Likes, "", 'b');
    QueueStore::attach_job_to_batch(
        &mut connection,
        running_batch,
        running.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-12T02:01:01.000Z",
    )
    .unwrap();
    QueueStore::claim_job(
        &mut connection,
        running.snapshot.id,
        "2026-08-12T02:01:02.000Z",
    )
    .unwrap();
    assert_eq!(
        QueueStore::get_batch(&connection, running_batch)
            .unwrap()
            .status,
        SyncBatchStatusV1::Running
    );
    QueueStore::finish_running_job(
        &mut connection,
        running.snapshot.id,
        &terminal(SyncJobStatusV1::Success, "2026-08-12T02:01:03.000Z"),
    )
    .unwrap();
    assert_eq!(
        QueueStore::get_batch(&connection, running_batch)
            .unwrap()
            .status,
        SyncBatchStatusV1::Success
    );

    let cancelled_batch = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::AllActiveAccounts,
        "2026-08-12T02:02:00.000Z",
    )
    .unwrap();
    let cancelled = enqueue_test_job(
        &mut connection,
        account_id,
        SyncSourceV1::Favorites,
        "",
        'c',
    );
    QueueStore::attach_job_to_batch(
        &mut connection,
        cancelled_batch,
        cancelled.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-12T02:02:01.000Z",
    )
    .unwrap();
    QueueStore::cancel_queued_job(
        &mut connection,
        cancelled.snapshot.id,
        "2026-08-12T02:02:02.000Z",
        "user_cancelled",
    )
    .unwrap();
    assert_eq!(
        QueueStore::get_batch(&connection, cancelled_batch)
            .unwrap()
            .status,
        SyncBatchStatusV1::Cancelled
    );

    let failed_batch = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::AllActiveAccounts,
        "2026-08-12T02:03:00.000Z",
    )
    .unwrap();
    QueueStore::append_batch_issue(
        &mut connection,
        failed_batch,
        &NewSyncBatchIssueV1 {
            task_key: TaskKeyV1 {
                account_id,
                source: SyncSourceV1::RemoteFolder,
                source_instance_key: "".to_string(),
            },
            target_kind_aggregate: true,
            phase: SyncBatchIssuePhaseV1::InitialExpansion,
            kind: NewSyncBatchIssueKindV1::Skipped {
                reason: "remote_folder_schedule_limit_exceeded".to_string(),
                requested_payload_sha256: None,
            },
        },
        "2026-08-12T02:03:01.000Z",
    )
    .unwrap();
    assert_eq!(
        QueueStore::get_batch(&connection, failed_batch)
            .unwrap()
            .status,
        SyncBatchStatusV1::Failed
    );
}

#[test]
fn batch_entries_share_sequence_page_at_two_hundred_and_bind_the_cursor() {
    let mut connection = database();
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::AllActiveAccounts,
        "2026-08-12T03:00:00.000Z",
    )
    .expect("batch");
    let empty =
        QueueStore::list_batch_entries(&connection, batch_id, None, 200).expect("empty page");
    assert!(empty.entries.is_empty());
    assert!(!empty.has_more);
    assert!(!empty.next_cursor.is_empty());

    for index in 0..100 {
        let account_id = insert_account(&connection, 1000 + index);
        let stored = QueueStore::enqueue_job(
            &mut connection,
            &job(
                account_id,
                SyncSourceV1::History,
                "",
                SyncTriggerV1::Manual,
                "2026-08-12T03:00:01.000Z",
                None,
                'a',
            ),
        )
        .unwrap();
        QueueStore::attach_job_to_batch(
            &mut connection,
            batch_id,
            stored.snapshot.id,
            BatchJobRoleV1::Root,
            "2026-08-12T03:00:02.000Z",
        )
        .unwrap();
    }
    let issue_account = insert_account(&connection, 5000);
    QueueStore::append_batch_issue(
        &mut connection,
        batch_id,
        &NewSyncBatchIssueV1 {
            task_key: TaskKeyV1 {
                account_id: issue_account,
                source: SyncSourceV1::Likes,
                source_instance_key: "".to_string(),
            },
            target_kind_aggregate: false,
            phase: SyncBatchIssuePhaseV1::InitialExpansion,
            kind: NewSyncBatchIssueKindV1::Skipped {
                reason: "collector_not_implemented".to_string(),
                requested_payload_sha256: Some(hash('b')),
            },
        },
        "2026-08-12T03:00:03.000Z",
    )
    .unwrap();
    for index in 100..200 {
        let account_id = insert_account(&connection, 1000 + index);
        let stored = QueueStore::enqueue_job(
            &mut connection,
            &job(
                account_id,
                SyncSourceV1::History,
                "",
                SyncTriggerV1::Manual,
                "2026-08-12T03:00:04.000Z",
                None,
                'c',
            ),
        )
        .unwrap();
        QueueStore::attach_job_to_batch(
            &mut connection,
            batch_id,
            stored.snapshot.id,
            BatchJobRoleV1::Root,
            "2026-08-12T03:00:05.000Z",
        )
        .unwrap();
    }

    let first =
        QueueStore::list_batch_entries(&connection, batch_id, Some(&empty.next_cursor), 200)
            .expect("first page");
    assert_eq!(first.entries.len(), 200);
    assert!(first.has_more);
    assert!(first
        .entries
        .iter()
        .any(|entry| matches!(entry, SyncBatchEntryV1::Skipped(_))));
    let second =
        QueueStore::list_batch_entries(&connection, batch_id, Some(&first.next_cursor), 200)
            .expect("second page");
    assert_eq!(second.entries.len(), 1);
    assert!(!second.has_more);
    assert!(!second.next_cursor.is_empty());

    let other_batch = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T03:01:00.000Z",
    )
    .unwrap();
    assert_eq!(
        QueueStore::list_batch_entries(&connection, other_batch, Some(&first.next_cursor), 200,)
            .unwrap_err(),
        "invalid_batch_cursor"
    );
    let mut tampered = first.next_cursor.into_bytes();
    let last = tampered.len() - 1;
    tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered).unwrap();
    assert_eq!(
        QueueStore::list_batch_entries(&connection, batch_id, Some(&tampered), 200).unwrap_err(),
        "invalid_batch_cursor"
    );
}

#[test]
fn batch_entry_reads_live_jobs_then_falls_back_to_the_last_snapshot() {
    let mut connection = database();
    let account_id = insert_account(&connection, 90);
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T04:00:00.000Z",
    )
    .unwrap();
    let stored = QueueStore::enqueue_job(
        &mut connection,
        &job(
            account_id,
            SyncSourceV1::History,
            "",
            SyncTriggerV1::Manual,
            "2026-08-12T04:00:01.000Z",
            None,
            'a',
        ),
    )
    .unwrap();
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        stored.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-12T04:00:02.000Z",
    )
    .unwrap();
    connection
        .execute(
            "UPDATE sync_batch_jobs SET job_view_snapshot_json = ?1 WHERE batch_id = ?2",
            params![
                r#"{"id":1,"accountId":1,"source":"history","sourceInstanceKey":"","trigger":"manual","status":"failed","queuedAt":"x","scheduledFor":null,"startedAt":null,"finishedAt":"x","queuePosition":null,"canCancel":false,"issues":[]}"#,
                batch_id
            ],
        )
        .unwrap();
    let live = QueueStore::list_batch_entries(&connection, batch_id, None, 10).unwrap();
    let SyncBatchEntryV1::Job(live_job) = &live.entries[0] else {
        panic!("job");
    };
    assert_eq!(live_job.status, SyncJobStatusV1::Queued);

    QueueStore::claim_job(
        &mut connection,
        stored.snapshot.id,
        "2026-08-12T04:00:03.000Z",
    )
    .unwrap();
    QueueStore::finish_running_job(
        &mut connection,
        stored.snapshot.id,
        &terminal(SyncJobStatusV1::Success, "2026-08-12T04:00:04.000Z"),
    )
    .unwrap();
    connection
        .execute("DELETE FROM accounts WHERE id = ?1", [account_id])
        .unwrap();
    let snapshot = QueueStore::list_batch_entries(&connection, batch_id, None, 10).unwrap();
    let SyncBatchEntryV1::Job(snapshot_job) = &snapshot.entries[0] else {
        panic!("job");
    };
    assert_eq!(snapshot_job.status, SyncJobStatusV1::Success);
    assert_eq!(snapshot_job.id, stored.snapshot.id);
}

#[test]
fn failed_mutations_roll_back_queue_and_batch_sequence_reservations() {
    let mut connection = database();
    let account_id = insert_account(&connection, 91);
    QueueStore::enqueue_job(
        &mut connection,
        &job(
            account_id,
            SyncSourceV1::History,
            "",
            SyncTriggerV1::Manual,
            "2026-08-12T05:00:00.000Z",
            None,
            'a',
        ),
    )
    .unwrap();
    assert_eq!(
        QueueStore::enqueue_job(
            &mut connection,
            &job(
                account_id,
                SyncSourceV1::History,
                "",
                SyncTriggerV1::Manual,
                "2026-08-12T05:00:01.000Z",
                None,
                'b',
            ),
        )
        .unwrap_err(),
        "sync_task_already_active"
    );
    let next = QueueStore::enqueue_job(
        &mut connection,
        &job(
            account_id,
            SyncSourceV1::Likes,
            "",
            SyncTriggerV1::Manual,
            "2026-08-12T05:00:02.000Z",
            None,
            'c',
        ),
    )
    .unwrap();
    assert_eq!(next.queue_seq, 2);

    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::AllActiveAccounts,
        "2026-08-12T05:01:00.000Z",
    )
    .unwrap();
    connection
        .execute(
            "UPDATE sync_batches SET next_entry_seq = ?1 WHERE id = ?2",
            params![i64::MAX, batch_id],
        )
        .unwrap();
    assert_eq!(
        QueueStore::append_batch_issue(
            &mut connection,
            batch_id,
            &NewSyncBatchIssueV1 {
                task_key: TaskKeyV1 {
                    account_id,
                    source: SyncSourceV1::Favorites,
                    source_instance_key: "".to_string(),
                },
                target_kind_aggregate: false,
                phase: SyncBatchIssuePhaseV1::InitialExpansion,
                kind: NewSyncBatchIssueKindV1::Skipped {
                    reason: "collector_not_implemented".to_string(),
                    requested_payload_sha256: Some(hash('d')),
                },
            },
            "2026-08-12T05:01:01.000Z",
        )
        .unwrap_err(),
        "sync_batch_entry_sequence_exhausted"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sync_batch_targets WHERE batch_id = ?1",
                [batch_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn conflict_entries_keep_the_conflict_time_snapshot_and_stable_issue_order() {
    let mut connection = database();
    let account_id = insert_account(&connection, 92);
    let existing = QueueStore::enqueue_job(
        &mut connection,
        &job(
            account_id,
            SyncSourceV1::History,
            "",
            SyncTriggerV1::Manual,
            "2026-08-12T06:00:00.000Z",
            None,
            'a',
        ),
    )
    .unwrap();
    let root = QueueStore::enqueue_job(
        &mut connection,
        &job(
            account_id,
            SyncSourceV1::Likes,
            "",
            SyncTriggerV1::Manual,
            "2026-08-12T06:00:01.000Z",
            None,
            'b',
        ),
    )
    .unwrap();
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T06:00:02.000Z",
    )
    .unwrap();
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        root.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-12T06:00:03.000Z",
    )
    .unwrap();
    QueueStore::append_batch_issue(
        &mut connection,
        batch_id,
        &NewSyncBatchIssueV1 {
            task_key: TaskKeyV1 {
                account_id,
                source: SyncSourceV1::Favorites,
                source_instance_key: "".to_string(),
            },
            target_kind_aggregate: false,
            phase: SyncBatchIssuePhaseV1::InitialExpansion,
            kind: NewSyncBatchIssueKindV1::Skipped {
                reason: "collector_not_implemented".to_string(),
                requested_payload_sha256: Some(hash('c')),
            },
        },
        "2026-08-12T06:00:04.000Z",
    )
    .unwrap();
    QueueStore::append_batch_issue(
        &mut connection,
        batch_id,
        &NewSyncBatchIssueV1 {
            task_key: existing.snapshot.task_key.clone(),
            target_kind_aggregate: false,
            phase: SyncBatchIssuePhaseV1::InitialExpansion,
            kind: NewSyncBatchIssueKindV1::Conflict {
                reason: "sync_task_payload_conflict".to_string(),
                requested_payload_sha256: hash('d'),
                existing_payload_sha256: existing.snapshot.task_payload_sha256.clone(),
                existing_job_id: existing.snapshot.id,
            },
        },
        "2026-08-12T06:00:05.000Z",
    )
    .unwrap();

    QueueStore::claim_job(
        &mut connection,
        existing.snapshot.id,
        "2026-08-12T06:00:06.000Z",
    )
    .unwrap();
    QueueStore::finish_running_job(
        &mut connection,
        existing.snapshot.id,
        &terminal(SyncJobStatusV1::Success, "2026-08-12T06:00:07.000Z"),
    )
    .unwrap();

    let page = QueueStore::list_batch_entries(&connection, batch_id, None, 10).unwrap();
    assert!(matches!(page.entries[0], SyncBatchEntryV1::Job(_)));
    assert!(matches!(page.entries[1], SyncBatchEntryV1::Skipped(_)));
    let SyncBatchEntryV1::Conflict(conflict) = &page.entries[2] else {
        panic!("conflict");
    };
    assert_eq!(
        conflict.existing_job_at_conflict.status,
        SyncJobStatusV1::Queued
    );
    assert_eq!(
        conflict.existing_payload_sha256,
        existing.snapshot.task_payload_sha256
    );
}
