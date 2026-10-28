use douyin_history_archive::{
    device_settings::BrowserPreferenceV1,
    migrate_schema_to_v5_at_startup,
    settings_store::{AccountSettingsV2, ArchiveListRangeV1},
    sync::{
        decide_enqueue_for_task_key_tx, freeze_task_payload, ArchiveRunModeV1, BatchJobRoleV1,
        BrowserExecutionIdentityV1, BrowserIdV1, EnqueueDeviceSnapshotV1, EnqueueService,
        EnqueueSettingsSnapshotV1, EnqueueSyncRequestV1, NewSyncBatchIssueKindV1,
        NewSyncBatchIssueV1, NewSyncJobV1, QueueStore, RangeFreezeV1, SourceCapabilityViewV1,
        SyncBatchEntryV1, SyncBatchIssuePhaseV1, SyncRangeKindV1, SyncScopeV1, SyncSourceV1,
        SyncTaskPayloadV1, SyncTriggerV1, TaskDecisionBatchTargetV1, TaskDecisionDispositionV1,
        TaskDecisionInputV1, TaskKeyV1,
    },
};
use rusqlite::{params, Connection};
use std::collections::BTreeMap;

const NOW: &str = "2026-08-12T04:00:00.000Z";
const NOW_MS: i64 = 1_786_507_200_000;

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    connection
}

fn insert_account(connection: &Connection, suffix: usize, status: &str) -> i64 {
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'x', 'x')",
            params![
                format!("uid-{suffix}"),
                format!("sec-{suffix}"),
                format!("{suffix:032x}"),
                status,
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

fn insert_remote_folders(connection: &mut Connection, account_id: i64, start: usize, count: usize) {
    let transaction = connection.transaction().expect("folder transaction");
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO remote_folders
                 (account_id, remote_folder_id, name, first_seen_at, last_seen_at)
                 VALUES (?1, ?2, ?3, 'x', 'x')",
            )
            .expect("folder statement");
        for index in start..start + count {
            statement
                .execute(params![
                    account_id,
                    format!("folder-{index:05}"),
                    format!("Folder {index}"),
                ])
                .expect("folder");
        }
    }
    transaction.commit().expect("folder commit");
}

fn browser(account_id: i64) -> BrowserExecutionIdentityV1 {
    BrowserExecutionIdentityV1 {
        preference: BrowserPreferenceV1::Auto,
        resolved_browser: BrowserIdV1::Edge,
        profile_key: format!("{account_id:032x}"),
        profile_generation: format!("{:032x}", account_id + 100),
        executable_path_sha256: "a".repeat(64),
    }
}

fn settings_snapshot(account_ids: &[i64]) -> EnqueueSettingsSnapshotV1 {
    EnqueueSettingsSnapshotV1 {
        revision: 0,
        accounts: account_ids
            .iter()
            .copied()
            .map(|account_id| (account_id, AccountSettingsV2::default()))
            .collect(),
    }
}

fn device_snapshot(account_ids: &[i64]) -> EnqueueDeviceSnapshotV1 {
    EnqueueDeviceSnapshotV1 {
        browsers: account_ids
            .iter()
            .copied()
            .map(|account_id| (account_id, browser(account_id)))
            .collect(),
    }
}

fn capabilities(available: &[SyncSourceV1]) -> Vec<SourceCapabilityViewV1> {
    [
        SyncSourceV1::History,
        SyncSourceV1::Likes,
        SyncSourceV1::Favorites,
        SyncSourceV1::RemoteFolder,
    ]
    .into_iter()
    .map(|source| {
        let archive = source != SyncSourceV1::History;
        SourceCapabilityViewV1 {
            source,
            available: available.contains(&source),
            unavailable_reason: (!available.contains(&source))
                .then(|| "collector_not_implemented".to_string()),
            supported_ranges: if archive {
                vec![
                    SyncRangeKindV1::All,
                    SyncRangeKindV1::RecentItems,
                    SyncRangeKindV1::RecordLimit,
                ]
            } else {
                vec![SyncRangeKindV1::Days, SyncRangeKindV1::Bottom]
            },
            supports_full_reconcile: archive,
            supports_remote_folders: source == SyncSourceV1::Favorites
                && available.contains(&SyncSourceV1::RemoteFolder),
        }
    })
    .collect()
}

fn one(account_id: i64, source: SyncSourceV1) -> EnqueueSyncRequestV1 {
    EnqueueSyncRequestV1 {
        scope: SyncScopeV1::One,
        account_id: Some(account_id),
        source: Some(source),
        source_instance_key: (source == SyncSourceV1::RemoteFolder).then(|| "folder-a".to_string()),
        payload: None,
        full_reconcile_confirmation_sha256: None,
    }
}

fn history_payload(
    account_id: i64,
    range: douyin_history_archive::settings_store::HistoryRangeV1,
    frozen_at_ms: i64,
) -> SyncTaskPayloadV1 {
    let cutoff_ms = match range {
        douyin_history_archive::settings_store::HistoryRangeV1::Days { days } => {
            Some(douyin_history_archive::sync::shanghai_cutoff_ms(frozen_at_ms, days).unwrap())
        }
        douyin_history_archive::settings_store::HistoryRangeV1::Bottom => None,
    };
    SyncTaskPayloadV1::History {
        range,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms,
            cutoff_ms,
        },
        scroll_speed: 900,
        hide_browser: false,
        browser: browser(account_id),
    }
}

fn decision(
    account_id: i64,
    source: SyncSourceV1,
    instance: &str,
    trigger: SyncTriggerV1,
    payload: SyncTaskPayloadV1,
    marker: char,
) -> TaskDecisionInputV1 {
    TaskDecisionInputV1 {
        task_key: TaskKeyV1 {
            account_id,
            source,
            source_instance_key: instance.to_string(),
        },
        trigger,
        queued_at: NOW.to_string(),
        scheduled_for: (trigger == SyncTriggerV1::Automatic)
            .then(|| "2026-08-12T05:00:00.000Z".to_string()),
        settings_revision: 0,
        config_sha256: (trigger == SyncTriggerV1::Automatic).then(|| marker.to_string().repeat(64)),
        payload: freeze_task_payload(payload).expect("frozen payload"),
        batch_targets: Vec::new(),
    }
}

fn batch_target(batch_id: i64, role: BatchJobRoleV1) -> TaskDecisionBatchTargetV1 {
    TaskDecisionBatchTargetV1 {
        batch_id,
        role,
        phase: SyncBatchIssuePhaseV1::InitialExpansion,
    }
}

#[test]
fn request_matrix_implicit_payloads_and_revision_zero_are_strict() {
    let mut connection = database();
    let mut service = EnqueueService::at(&mut connection, NOW).expect("service clock");
    let empty_settings = EnqueueSettingsSnapshotV1 {
        revision: 0,
        accounts: BTreeMap::new(),
    };
    let empty_device = EnqueueDeviceSnapshotV1 {
        browsers: BTreeMap::new(),
    };
    let all = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::AllActiveAccounts,
        account_id: None,
        source: None,
        source_instance_key: None,
        payload: None,
        full_reconcile_confirmation_sha256: None,
    };
    assert_eq!(
        service
            .enqueue(&all, &empty_settings, &empty_device, &capabilities(&[]))
            .unwrap_err(),
        "no_active_accounts"
    );
    drop(service);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM sync_batches", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    let account_id = insert_account(&connection, 1, "active");
    let settings = settings_snapshot(&[account_id]);
    let device = device_snapshot(&[account_id]);
    let mut service = EnqueueService::at(&mut connection, NOW).expect("service clock");
    let result = service
        .enqueue(
            &one(account_id, SyncSourceV1::History),
            &settings,
            &device,
            &capabilities(&[SyncSourceV1::History]),
        )
        .expect("implicit history");
    assert_eq!(result.batch_id, None);
    assert_eq!(result.counts.jobs, 1);
    assert_eq!(result.entries.len(), 1);
    drop(service);
    assert_eq!(
        connection
            .query_row(
                "SELECT settings_revision FROM sync_jobs WHERE account_id = ?1",
                [account_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );

    let mut service = EnqueueService::at(&mut connection, NOW).expect("service clock");
    assert_eq!(
        service
            .enqueue(
                &one(account_id, SyncSourceV1::Likes),
                &settings,
                &device,
                &capabilities(&[SyncSourceV1::History]),
            )
            .unwrap_err(),
        "source_unavailable"
    );

    let recent_days = SyncTaskPayloadV1::Likes {
        range: ArchiveListRangeV1::RecentDays { days: 3 },
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: NOW_MS,
            cutoff_ms: Some(douyin_history_archive::sync::shanghai_cutoff_ms(NOW_MS, 3).unwrap()),
        },
        run_mode: ArchiveRunModeV1::Incremental,
        scroll_speed: 900,
        hide_browser: false,
        browser: browser(account_id),
    };
    let request = EnqueueSyncRequestV1 {
        payload: Some(recent_days),
        ..one(account_id, SyncSourceV1::Likes)
    };
    assert_eq!(
        service
            .enqueue(
                &request,
                &settings,
                &device,
                &capabilities(&[SyncSourceV1::Likes]),
            )
            .unwrap_err(),
        "time_range_not_supported"
    );

    let mut implicit_recent_days = settings.clone();
    implicit_recent_days
        .accounts
        .get_mut(&account_id)
        .unwrap()
        .likes
        .range = ArchiveListRangeV1::RecentDays { days: 3 };
    assert_eq!(
        service
            .enqueue(
                &one(account_id, SyncSourceV1::Likes),
                &implicit_recent_days,
                &device,
                &capabilities(&[SyncSourceV1::Likes]),
            )
            .unwrap_err(),
        "time_range_not_supported"
    );
}

#[test]
fn task_decisions_attach_promote_replace_and_preserve_manual_snapshots() {
    let mut connection = database();
    let account_id = insert_account(&connection, 10, "active");

    let original = freeze_task_payload(history_payload(
        account_id,
        douyin_history_archive::settings_store::HistoryRangeV1::Bottom,
        NOW_MS - 60_000,
    ))
    .unwrap();
    let automatic = QueueStore::enqueue_job(
        &mut connection,
        &NewSyncJobV1 {
            task_key: TaskKeyV1 {
                account_id,
                source: SyncSourceV1::History,
                source_instance_key: String::new(),
            },
            trigger: SyncTriggerV1::Automatic,
            queued_at: "2026-08-12T03:00:00.000Z".to_string(),
            scheduled_for: Some("2026-08-12T05:00:00.000Z".to_string()),
            settings_revision: 0,
            config_sha256: Some("b".repeat(64)),
            payload_json: original.payload_json,
            payload_sha256: original.payload_sha256,
            equivalence_sha256: original.equivalence_sha256,
        },
    )
    .expect("automatic");
    let automatic_queue_seq = automatic.queue_seq;

    let transaction = connection.transaction().unwrap();
    let promoted = decide_enqueue_for_task_key_tx(
        &transaction,
        &decision(
            account_id,
            SyncSourceV1::History,
            "",
            SyncTriggerV1::Manual,
            history_payload(
                account_id,
                douyin_history_archive::settings_store::HistoryRangeV1::Bottom,
                NOW_MS,
            ),
            'c',
        ),
    )
    .expect("promote");
    assert_eq!(promoted.disposition, TaskDecisionDispositionV1::Promoted);
    let promoted_job = promoted.job.expect("promoted job");
    assert_eq!(promoted_job.snapshot.id, automatic.snapshot.id);
    assert_eq!(promoted_job.snapshot.trigger, SyncTriggerV1::Manual);
    assert!(promoted_job.queue_seq > automatic_queue_seq);
    assert_eq!(promoted_job.config_sha256, None);
    transaction.commit().unwrap();

    let attached_input = decision(
        account_id,
        SyncSourceV1::History,
        "",
        SyncTriggerV1::Manual,
        history_payload(
            account_id,
            douyin_history_archive::settings_store::HistoryRangeV1::Bottom,
            NOW_MS + 60_000,
        ),
        'd',
    );
    let requested_payload_hash = attached_input.payload.payload_sha256.clone();
    let transaction = connection.transaction().unwrap();
    let attached = decide_enqueue_for_task_key_tx(&transaction, &attached_input).unwrap();
    assert_eq!(attached.disposition, TaskDecisionDispositionV1::Attached);
    let attached_job = attached.job.expect("attached job");
    assert_eq!(attached_job.snapshot.id, automatic.snapshot.id);
    assert_ne!(
        attached_job.snapshot.task_payload_sha256,
        requested_payload_hash
    );
    transaction.commit().unwrap();

    QueueStore::cancel_queued_job(
        &mut connection,
        automatic.snapshot.id,
        "2026-08-12T04:01:00.000Z",
        "test_reset",
    )
    .unwrap();
    let old = QueueStore::enqueue_job(
        &mut connection,
        &NewSyncJobV1 {
            task_key: TaskKeyV1 {
                account_id,
                source: SyncSourceV1::History,
                source_instance_key: String::new(),
            },
            trigger: SyncTriggerV1::Automatic,
            queued_at: "2026-08-12T04:02:00.000Z".to_string(),
            scheduled_for: Some("2026-08-12T06:00:00.000Z".to_string()),
            settings_revision: 1,
            config_sha256: Some("e".repeat(64)),
            payload_json: attached_input.payload.payload_json,
            payload_sha256: attached_input.payload.payload_sha256,
            equivalence_sha256: attached_input.payload.equivalence_sha256,
        },
    )
    .unwrap();
    let transaction = connection.transaction().unwrap();
    let replaced = decide_enqueue_for_task_key_tx(
        &transaction,
        &decision(
            account_id,
            SyncSourceV1::History,
            "",
            SyncTriggerV1::Manual,
            history_payload(
                account_id,
                douyin_history_archive::settings_store::HistoryRangeV1::Days { days: 7 },
                NOW_MS,
            ),
            'f',
        ),
    )
    .expect("replace automatic");
    assert_eq!(replaced.disposition, TaskDecisionDispositionV1::Replaced);
    assert_ne!(replaced.job.as_ref().unwrap().snapshot.id, old.snapshot.id);
    transaction.commit().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT status FROM sync_jobs WHERE id = ?1",
                [old.snapshot.id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "cancelled"
    );
}

#[test]
fn task_decisions_conflict_only_on_the_exact_active_task_key() {
    let mut connection = database();
    let first = insert_account(&connection, 20, "active");
    let second = insert_account(&connection, 21, "active");
    let existing = decision(
        first,
        SyncSourceV1::History,
        "",
        SyncTriggerV1::Manual,
        history_payload(
            first,
            douyin_history_archive::settings_store::HistoryRangeV1::Bottom,
            NOW_MS,
        ),
        'a',
    );
    let transaction = connection.transaction().unwrap();
    let created = decide_enqueue_for_task_key_tx(&transaction, &existing).unwrap();
    let job_id = created.job.unwrap().snapshot.id;
    transaction.commit().unwrap();

    let transaction = connection.transaction().unwrap();
    let conflict = decide_enqueue_for_task_key_tx(
        &transaction,
        &decision(
            first,
            SyncSourceV1::History,
            "",
            SyncTriggerV1::Manual,
            history_payload(
                first,
                douyin_history_archive::settings_store::HistoryRangeV1::Days { days: 3 },
                NOW_MS,
            ),
            'b',
        ),
    )
    .unwrap();
    assert_eq!(conflict.disposition, TaskDecisionDispositionV1::Conflict);
    assert_eq!(conflict.existing_job.unwrap().snapshot.id, job_id);

    let isolated = decide_enqueue_for_task_key_tx(
        &transaction,
        &decision(
            second,
            SyncSourceV1::History,
            "",
            SyncTriggerV1::Manual,
            history_payload(
                second,
                douyin_history_archive::settings_store::HistoryRangeV1::Days { days: 3 },
                NOW_MS,
            ),
            'c',
        ),
    )
    .unwrap();
    assert_eq!(isolated.disposition, TaskDecisionDispositionV1::Created);
    transaction.commit().unwrap();
}

#[test]
fn automatic_config_changes_replace_only_a_queued_automatic_job() {
    let mut connection = database();
    let account_id = insert_account(&connection, 25, "active");
    let first = decision(
        account_id,
        SyncSourceV1::History,
        "",
        SyncTriggerV1::Automatic,
        history_payload(
            account_id,
            douyin_history_archive::settings_store::HistoryRangeV1::Bottom,
            NOW_MS,
        ),
        'a',
    );
    let transaction = connection.transaction().unwrap();
    let old = decide_enqueue_for_task_key_tx(&transaction, &first)
        .unwrap()
        .job
        .unwrap();
    transaction.commit().unwrap();

    let mut changed = first.clone();
    changed.config_sha256 = Some("b".repeat(64));
    let transaction = connection.transaction().unwrap();
    let replaced = decide_enqueue_for_task_key_tx(&transaction, &changed).unwrap();
    assert_eq!(replaced.disposition, TaskDecisionDispositionV1::Replaced);
    let replacement = replaced.job.unwrap();
    assert_ne!(replacement.snapshot.id, old.snapshot.id);
    transaction.commit().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT status, error_code FROM sync_jobs WHERE id = ?1",
                [old.snapshot.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap(),
        (
            "cancelled".to_string(),
            "automatic_settings_changed".to_string(),
        )
    );

    QueueStore::claim_job(
        &mut connection,
        replacement.snapshot.id,
        "2026-08-12T04:01:00.000Z",
    )
    .unwrap();
    let mut changed_again = changed;
    changed_again.config_sha256 = Some("c".repeat(64));
    let transaction = connection.transaction().unwrap();
    let conflict = decide_enqueue_for_task_key_tx(&transaction, &changed_again).unwrap();
    assert_eq!(conflict.disposition, TaskDecisionDispositionV1::Conflict);
    transaction.commit().unwrap();
}

#[test]
fn explicit_full_reconcile_requires_and_accepts_the_complete_payload_hash() {
    let mut connection = database();
    let account_id = insert_account(&connection, 26, "active");
    let payload = SyncTaskPayloadV1::Likes {
        range: ArchiveListRangeV1::All,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: NOW_MS,
            cutoff_ms: None,
        },
        run_mode: ArchiveRunModeV1::FullReconcile,
        scroll_speed: 900,
        hide_browser: false,
        browser: browser(account_id),
    };
    let confirmation = freeze_task_payload(payload.clone()).unwrap().payload_sha256;
    let request = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::One,
        account_id: Some(account_id),
        source: Some(SyncSourceV1::Likes),
        source_instance_key: None,
        payload: Some(payload),
        full_reconcile_confirmation_sha256: Some(confirmation),
    };
    let settings = settings_snapshot(&[account_id]);
    let device = device_snapshot(&[account_id]);
    let mut service = EnqueueService::at(&mut connection, NOW).unwrap();
    let result = service
        .enqueue(
            &request,
            &settings,
            &device,
            &capabilities(&[SyncSourceV1::Likes]),
        )
        .expect("full reconcile");
    assert_eq!(result.counts.jobs, 1);
    drop(service);
    assert_eq!(
        connection
            .query_row(
                "SELECT json_extract(payload_json, '$.runMode') FROM sync_jobs",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "fullReconcile"
    );

    let mut invalid = request;
    invalid.full_reconcile_confirmation_sha256 = Some("f".repeat(64));
    let mut service = EnqueueService::at(&mut connection, NOW).unwrap();
    assert_eq!(
        service
            .enqueue(
                &invalid,
                &settings,
                &device,
                &capabilities(&[SyncSourceV1::Likes]),
            )
            .unwrap_err(),
        "full_reconcile_confirmation_mismatch"
    );
}

#[test]
fn batch_decisions_reuse_one_job_and_persist_conflicts_for_every_target() {
    let mut connection = database();
    let account_id = insert_account(&connection, 30, "active");
    let first_batch = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        NOW,
    )
    .unwrap();
    let second_batch =
        QueueStore::create_batch(&mut connection, SyncScopeV1::AllActiveAccounts, NOW).unwrap();
    let mut input = decision(
        account_id,
        SyncSourceV1::History,
        "",
        SyncTriggerV1::Manual,
        history_payload(
            account_id,
            douyin_history_archive::settings_store::HistoryRangeV1::Bottom,
            NOW_MS,
        ),
        'a',
    );
    input.batch_targets = vec![
        batch_target(first_batch, BatchJobRoleV1::Root),
        batch_target(second_batch, BatchJobRoleV1::Root),
    ];
    let transaction = connection.transaction().unwrap();
    let created = decide_enqueue_for_task_key_tx(&transaction, &input).unwrap();
    assert_eq!(created.disposition, TaskDecisionDispositionV1::Created);
    let job_id = created.job.unwrap().snapshot.id;
    transaction.commit().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sync_batch_jobs WHERE job_id = ?1",
                [job_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );

    let third_batch = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        NOW,
    )
    .unwrap();
    let fourth_batch =
        QueueStore::create_batch(&mut connection, SyncScopeV1::AllActiveAccounts, NOW).unwrap();
    let mut conflict_input = decision(
        account_id,
        SyncSourceV1::History,
        "",
        SyncTriggerV1::Manual,
        history_payload(
            account_id,
            douyin_history_archive::settings_store::HistoryRangeV1::Days { days: 7 },
            NOW_MS,
        ),
        'b',
    );
    conflict_input.batch_targets = vec![
        batch_target(third_batch, BatchJobRoleV1::Root),
        batch_target(fourth_batch, BatchJobRoleV1::Root),
    ];
    let transaction = connection.transaction().unwrap();
    let conflict = decide_enqueue_for_task_key_tx(&transaction, &conflict_input).unwrap();
    assert_eq!(conflict.disposition, TaskDecisionDispositionV1::Conflict);
    transaction.commit().unwrap();
    for batch_id in [third_batch, fourth_batch] {
        let page = QueueStore::list_batch_entries(&connection, batch_id, None, 200).unwrap();
        assert!(matches!(
            page.entries.as_slice(),
            [SyncBatchEntryV1::Conflict(_)]
        ));
    }
}

#[test]
fn batch_attachment_limit_is_all_or_none_for_existing_and_new_jobs() {
    let mut connection = database();
    let existing_account = insert_account(&connection, 40, "active");
    let new_account = insert_account(&connection, 41, "active");
    let existing_batches = (0..5)
        .map(|index| {
            QueueStore::create_batch(
                &mut connection,
                if index % 2 == 0 {
                    SyncScopeV1::CurrentAccountAvailableSources
                } else {
                    SyncScopeV1::AllActiveAccounts
                },
                NOW,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let new_batches = (5..10)
        .map(|index| {
            QueueStore::create_batch(
                &mut connection,
                if index % 2 == 0 {
                    SyncScopeV1::CurrentAccountAvailableSources
                } else {
                    SyncScopeV1::AllActiveAccounts
                },
                NOW,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();

    let mut existing_input = decision(
        existing_account,
        SyncSourceV1::History,
        "",
        SyncTriggerV1::Manual,
        history_payload(
            existing_account,
            douyin_history_archive::settings_store::HistoryRangeV1::Bottom,
            NOW_MS,
        ),
        'c',
    );
    existing_input.batch_targets = existing_batches[..4]
        .iter()
        .copied()
        .map(|batch_id| batch_target(batch_id, BatchJobRoleV1::Root))
        .collect();
    let transaction = connection.transaction().unwrap();
    decide_enqueue_for_task_key_tx(&transaction, &existing_input).unwrap();
    transaction.commit().unwrap();

    let mut too_many_existing = existing_input;
    too_many_existing.batch_targets =
        vec![batch_target(existing_batches[4], BatchJobRoleV1::Attached)];
    let transaction = connection.transaction().unwrap();
    let limited_existing =
        decide_enqueue_for_task_key_tx(&transaction, &too_many_existing).unwrap();
    assert_eq!(
        limited_existing.disposition,
        TaskDecisionDispositionV1::Conflict
    );
    transaction.commit().unwrap();

    let mut too_many_new = decision(
        new_account,
        SyncSourceV1::History,
        "",
        SyncTriggerV1::Manual,
        history_payload(
            new_account,
            douyin_history_archive::settings_store::HistoryRangeV1::Bottom,
            NOW_MS,
        ),
        'd',
    );
    too_many_new.batch_targets = new_batches
        .iter()
        .copied()
        .map(|batch_id| batch_target(batch_id, BatchJobRoleV1::Root))
        .collect();
    let transaction = connection.transaction().unwrap();
    let limited_new = decide_enqueue_for_task_key_tx(&transaction, &too_many_new).unwrap();
    assert_eq!(limited_new.disposition, TaskDecisionDispositionV1::Skipped);
    transaction.commit().unwrap();

    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sync_jobs WHERE account_id = ?1",
                [new_account],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    for batch_id in existing_batches.iter().take(4) {
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sync_batch_jobs WHERE batch_id = ?1",
                    [batch_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
    }
    for batch_id in [existing_batches[4]].into_iter().chain(new_batches) {
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sync_batch_issues
                     WHERE batch_id = ?1 AND reason = 'sync_job_batch_attachment_limit_exceeded'",
                    [batch_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
    }
}

#[test]
fn bulk_expansion_ignores_schedule_flags_and_orders_all_available_targets() {
    let mut connection = database();
    let first = insert_account(&connection, 50, "active");
    let second = insert_account(&connection, 51, "active");
    let _archived = insert_account(&connection, 52, "archived");
    for index in [1, 0] {
        connection
            .execute(
                "INSERT INTO remote_folders
                 (account_id, remote_folder_id, name, first_seen_at, last_seen_at)
                 VALUES (?1, ?2, ?3, 'x', 'x')",
                params![
                    first,
                    format!("folder-{index:05}"),
                    format!("Folder {index}"),
                ],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO remote_folders
             (account_id, remote_folder_id, name, first_seen_at, last_seen_at, remote_removed_at)
             VALUES (?1, 'removed', 'Removed', 'x', 'x', 'x')",
            [first],
        )
        .unwrap();

    let settings = settings_snapshot(&[first, second]);
    let device = device_snapshot(&[first, second]);
    let all_capabilities = capabilities(&[
        SyncSourceV1::History,
        SyncSourceV1::Likes,
        SyncSourceV1::Favorites,
        SyncSourceV1::RemoteFolder,
    ]);
    let current = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::CurrentAccountAvailableSources,
        account_id: Some(first),
        source: None,
        source_instance_key: None,
        payload: None,
        full_reconcile_confirmation_sha256: None,
    };
    let mut service = EnqueueService::at(&mut connection, NOW).unwrap();
    let result = service
        .enqueue(&current, &settings, &device, &all_capabilities)
        .expect("current account bulk");
    assert_eq!(result.counts.jobs, 5);
    assert_eq!(result.counts.skipped, 0);
    assert_eq!(result.entries.len(), 5);
    let batch_id = result.batch_id.expect("batch");
    drop(service);
    let keys = connection
        .prepare(
            "SELECT source, source_instance_key FROM sync_jobs
             WHERE account_id = ?1 ORDER BY queue_seq",
        )
        .unwrap()
        .query_map([first], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        keys,
        vec![
            ("history".to_string(), String::new()),
            ("likes".to_string(), String::new()),
            ("favorites".to_string(), String::new()),
            ("remote_folder".to_string(), "folder-00000".to_string()),
            ("remote_folder".to_string(), "folder-00001".to_string()),
        ]
    );
    assert_eq!(
        QueueStore::get_batch(&connection, batch_id)
            .unwrap()
            .counts
            .jobs,
        5
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sync_jobs
                 WHERE account_id = ?1 AND source <> 'history'
                   AND json_extract(payload_json, '$.runMode') = 'incremental'",
                [first],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        4
    );

    let all = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::AllActiveAccounts,
        account_id: None,
        source: None,
        source_instance_key: None,
        payload: None,
        full_reconcile_confirmation_sha256: None,
    };
    let mut service = EnqueueService::at(&mut connection, "2026-08-12T04:01:00.000Z").unwrap();
    let result = service
        .enqueue(
            &all,
            &settings,
            &device,
            &capabilities(&[SyncSourceV1::History]),
        )
        .expect("all active accounts");
    assert_eq!(result.counts.jobs, 2);
    assert_eq!(result.counts.conflicts, 0);
    drop(service);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sync_jobs WHERE account_id = ?1",
                [second],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    let before = connection
        .query_row("SELECT COUNT(*) FROM sync_batches", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    let mut service = EnqueueService::at(&mut connection, "2026-08-12T04:02:00.000Z").unwrap();
    assert_eq!(
        service
            .enqueue(&current, &settings, &device, &capabilities(&[]))
            .unwrap_err(),
        "no_available_sources"
    );
    drop(service);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM sync_batches", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn bulk_limits_are_checked_before_creating_or_partially_expanding_a_batch() {
    let mut connection = database();
    let first = insert_account(&connection, 60, "active");
    let settings = settings_snapshot(&[first]);
    let device = device_snapshot(&[first]);
    let current = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::CurrentAccountAvailableSources,
        account_id: Some(first),
        source: None,
        source_instance_key: None,
        payload: None,
        full_reconcile_confirmation_sha256: None,
    };
    for _ in 0..16 {
        QueueStore::create_batch(
            &mut connection,
            SyncScopeV1::CurrentAccountAvailableSources,
            NOW,
        )
        .unwrap();
    }
    let mut service = EnqueueService::at(&mut connection, NOW).unwrap();
    assert_eq!(
        service
            .enqueue(
                &current,
                &settings,
                &device,
                &capabilities(&[SyncSourceV1::History]),
            )
            .unwrap_err(),
        "sync_active_batch_limit_exceeded"
    );
    drop(service);

    connection.execute("DELETE FROM sync_batches", []).unwrap();
    insert_remote_folders(&mut connection, first, 0, 2_001);
    let mut service = EnqueueService::at(&mut connection, NOW).unwrap();
    let aggregate = service
        .enqueue(
            &current,
            &settings,
            &device,
            &capabilities(&[SyncSourceV1::History, SyncSourceV1::RemoteFolder]),
        )
        .expect("aggregate remote folder skip");
    assert_eq!(aggregate.counts.jobs, 1);
    assert_eq!(aggregate.counts.skipped, 1);
    drop(service);
    assert_eq!(
        connection
            .query_row(
                "SELECT target_kind, source_instance_key, reason FROM sync_batch_targets targets
                 JOIN sync_batch_issues issues USING(batch_id, account_id, source, source_instance_key)
                 WHERE targets.batch_id = ?1",
                [aggregate.batch_id.unwrap()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap(),
        (
            "aggregate".to_string(),
            String::new(),
            "remote_folder_schedule_limit_exceeded".to_string(),
        )
    );

    let mut connection = database();
    let accounts = [
        insert_account(&connection, 70, "active"),
        insert_account(&connection, 71, "active"),
        insert_account(&connection, 72, "active"),
    ];
    insert_remote_folders(&mut connection, accounts[0], 0, 1_667);
    insert_remote_folders(&mut connection, accounts[1], 2_000, 1_667);
    insert_remote_folders(&mut connection, accounts[2], 4_000, 1_667);
    let settings = settings_snapshot(&accounts);
    let device = device_snapshot(&accounts);
    let all = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::AllActiveAccounts,
        account_id: None,
        source: None,
        source_instance_key: None,
        payload: None,
        full_reconcile_confirmation_sha256: None,
    };
    let mut service = EnqueueService::at(&mut connection, NOW).unwrap();
    assert_eq!(
        service
            .enqueue(
                &all,
                &settings,
                &device,
                &capabilities(&[SyncSourceV1::RemoteFolder]),
            )
            .unwrap_err(),
        "sync_batch_target_limit_exceeded"
    );
    drop(service);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM sync_batches", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM sync_jobs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn selected_account_aggregate_only_expands_requested_active_accounts() {
    let mut connection = database();
    let first = insert_account(&connection, 301, "active");
    let second = insert_account(&connection, 302, "active");
    let third = insert_account(&connection, 303, "active");
    let account_ids = [first, second, third];
    let request = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::AllActiveAccounts,
        account_id: None,
        source: None,
        source_instance_key: None,
        payload: None,
        full_reconcile_confirmation_sha256: None,
    };
    let settings = settings_snapshot(&account_ids);
    let device = device_snapshot(&account_ids);

    let result = EnqueueService::at(&mut connection, NOW)
        .unwrap()
        .enqueue_for_account_ids(
            &request,
            &[first, third],
            &settings,
            &device,
            &capabilities(&[SyncSourceV1::History, SyncSourceV1::Likes]),
        )
        .expect("selected aggregate");

    assert_eq!(result.counts.jobs, 4);
    let mut selected = connection
        .prepare(
            "SELECT DISTINCT account_id FROM sync_batch_targets
             WHERE batch_id = ?1 ORDER BY account_id",
        )
        .unwrap()
        .query_map([result.batch_id.unwrap()], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(selected, vec![first, third]);

    selected.push(99_999);
    let before = connection
        .query_row("SELECT COUNT(*) FROM sync_batches", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    let error = EnqueueService::at(&mut connection, "2026-08-12T04:05:00.000Z")
        .unwrap()
        .enqueue_for_account_ids(
            &request,
            &selected,
            &settings,
            &device,
            &capabilities(&[SyncSourceV1::History]),
        )
        .unwrap_err();
    assert_eq!(error, "account_scope_invalid");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM sync_batches", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn exactly_five_thousand_bulk_targets_are_accepted_and_page_at_two_hundred() {
    let mut connection = database();
    let accounts = [
        insert_account(&connection, 80, "active"),
        insert_account(&connection, 81, "active"),
        insert_account(&connection, 82, "active"),
    ];
    insert_remote_folders(&mut connection, accounts[0], 0, 2_000);
    insert_remote_folders(&mut connection, accounts[1], 3_000, 1_500);
    insert_remote_folders(&mut connection, accounts[2], 5_000, 1_500);
    let settings = settings_snapshot(&accounts);
    let device = device_snapshot(&accounts);
    let all = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::AllActiveAccounts,
        account_id: None,
        source: None,
        source_instance_key: None,
        payload: None,
        full_reconcile_confirmation_sha256: None,
    };
    let mut service = EnqueueService::at(&mut connection, NOW).unwrap();
    let result = service
        .enqueue(
            &all,
            &settings,
            &device,
            &capabilities(&[SyncSourceV1::RemoteFolder]),
        )
        .expect("five thousand targets");
    assert_eq!(result.counts.jobs, 5_000);
    assert_eq!(result.entries.len(), 200);
    assert!(result.next_cursor.is_some());
    let batch_id = result.batch_id.unwrap();
    drop(service);
    QueueStore::append_batch_issue(
        &mut connection,
        batch_id,
        &NewSyncBatchIssueV1 {
            task_key: TaskKeyV1 {
                account_id: accounts[0],
                source: SyncSourceV1::RemoteFolder,
                source_instance_key: String::new(),
            },
            target_kind_aggregate: true,
            phase: SyncBatchIssuePhaseV1::DiscoveryExpansion,
            kind: NewSyncBatchIssueKindV1::Skipped {
                reason: "sync_batch_target_limit_exceeded".to_string(),
                requested_payload_sha256: None,
            },
        },
        "2026-08-12T04:01:00.000Z",
    )
    .expect("aggregate after five thousand tasks");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM sync_jobs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        5_000
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM sync_batch_targets", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        5_001
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sync_batch_targets WHERE target_kind = 'task'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        5_000
    );
}
