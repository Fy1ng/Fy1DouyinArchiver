use chrono::{DateTime, Utc};
use douyin_history_archive::{
    device_settings::BrowserPreferenceV1, settings_store::PortableSettingsV2,
};
use douyin_history_archive::{
    migrate_schema_to_v5_at_startup,
    settings_store::{
        write_portable_settings_atomic, AccountSettingsV2, ArchiveListRangeV1,
        RemoteFolderScheduleModeV1,
    },
    sync::{
        build_scheduler_state, calculate_next_due, expand_source_instances, freeze_task_payload,
        list_scheduler_instances, list_scheduler_instances_with_settings_loader,
        reconcile_automatic_jobs, AutomaticReconcileInputV1, AutomaticSyncConfigV1,
        BrowserExecutionIdentityV1, BrowserIdV1, NewSyncJobV1, QueueStore, RangeFreezeV1,
        SchedulerAccountInputV1, SchedulerRuntimeSnapshotV1, SchedulerWorkerStateV1,
        SourceCapabilityViewV1, SyncJobStatusV1, SyncRangeKindV1, SyncSourceV1, SyncTaskPayloadV1,
        SyncTriggerV1, TaskKeyV1, TerminalAnchorV1, TerminalJobUpdateV1,
    },
};
use rusqlite::{params, Connection};
use std::{fs, path::PathBuf, time::SystemTime};

const APP_READY: &str = "2026-08-12T00:00:00.000Z";

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema v5");
    connection
}

fn insert_account(connection: &Connection, suffix: usize) -> i64 {
    connection
        .execute(
            "INSERT INTO accounts(uid, profile_key, status, created_at, updated_at)
             VALUES (?1, ?2, 'active', 'x', 'x')",
            params![format!("uid-{suffix}"), format!("{suffix:032x}"),],
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

fn insert_remote_folders(connection: &mut Connection, account_id: i64, count: usize) {
    let transaction = connection.transaction().expect("folder transaction");
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO remote_folders
                 (account_id, remote_folder_id, name, first_seen_at, last_seen_at)
                 VALUES (?1, ?2, ?3, 'x', 'x')",
            )
            .expect("folder statement");
        for index in 0..count {
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
                    SyncRangeKindV1::RecentDays,
                ]
            } else {
                vec![SyncRangeKindV1::Days, SyncRangeKindV1::Bottom]
            },
            supports_full_reconcile: archive,
            supports_remote_folders: source == SyncSourceV1::Favorites,
        }
    })
    .collect()
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

fn timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp")
        .with_timezone(&Utc)
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "emily-sync-scheduler-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

#[test]
fn due_time_uses_startup_delay_or_the_latest_started_terminal_anchor() {
    let app_ready = timestamp(APP_READY);
    assert_eq!(
        calculate_next_due(None, app_ready, 10, 360).unwrap(),
        timestamp("2026-08-12T00:10:00.000Z")
    );

    let manual_anchor = TerminalAnchorV1 {
        job_id: 7,
        status: SyncJobStatusV1::Success,
        started_at: "2026-08-12T09:00:00.000Z".to_string(),
        finished_at: "2026-08-12T10:20:00.000Z".to_string(),
    };
    assert_eq!(
        calculate_next_due(Some(&manual_anchor), app_ready, 10, 360).unwrap(),
        timestamp("2026-08-12T16:20:00.000Z")
    );
    assert_eq!(
        calculate_next_due(Some(&manual_anchor), app_ready, 10, 60).unwrap(),
        timestamp("2026-08-12T11:20:00.000Z")
    );
}

#[test]
fn expansion_returns_all_capabilities_and_resolves_every_known_folder() {
    let mut settings = AccountSettingsV2::default();
    settings.favorites.archive.schedule.enabled = false;
    settings.favorites.remote_folder_defaults.schedule.enabled = true;
    settings
        .favorites
        .remote_folder_defaults
        .schedule
        .interval_minutes = 180;
    settings.favorites.remote_folders.insert(
        "folder-a".to_string(),
        douyin_history_archive::settings_store::ArchiveSourceSettingsV1 {
            range: ArchiveListRangeV1::RecentItems { count: 17 },
            schedule: douyin_history_archive::settings_store::SourceScheduleV1 {
                enabled: true,
                interval_minutes: 45,
            },
        },
    );
    let registry = vec!["folder-b".to_string(), "folder-a".to_string()];
    let all_capabilities = capabilities(&[
        SyncSourceV1::History,
        SyncSourceV1::Likes,
        SyncSourceV1::Favorites,
        SyncSourceV1::RemoteFolder,
    ]);

    for mode in [
        RemoteFolderScheduleModeV1::Unified,
        RemoteFolderScheduleModeV1::PerFolder,
    ] {
        settings.favorites.remote_folder_mode = mode;
        let expansion = expand_source_instances(
            5,
            &settings,
            BrowserPreferenceV1::Auto,
            &all_capabilities,
            &registry,
        )
        .expect("source expansion");
        assert_eq!(expansion.source_states.len(), 4);
        assert_eq!(
            expansion
                .source_states
                .iter()
                .map(|state| state.source)
                .collect::<Vec<_>>(),
            vec![
                SyncSourceV1::History,
                SyncSourceV1::Likes,
                SyncSourceV1::Favorites,
                SyncSourceV1::RemoteFolder,
            ]
        );
        let folders = expansion
            .instances
            .iter()
            .filter(|instance| instance.task_key.source == SyncSourceV1::RemoteFolder)
            .collect::<Vec<_>>();
        assert_eq!(
            folders
                .iter()
                .map(|instance| instance.task_key.source_instance_key.as_str())
                .collect::<Vec<_>>(),
            vec!["folder-a", "folder-b"]
        );
        match &folders[0].automatic_config {
            AutomaticSyncConfigV1::RemoteFolder {
                range,
                schedule,
                remote_folder_mode,
                ..
            } => {
                let expected_range = if mode == RemoteFolderScheduleModeV1::PerFolder {
                    ArchiveListRangeV1::RecentItems { count: 17 }
                } else {
                    ArchiveListRangeV1::RecentItems { count: 100 }
                };
                let expected_interval = if mode == RemoteFolderScheduleModeV1::PerFolder {
                    45
                } else {
                    180
                };
                assert_eq!(*range, expected_range);
                assert_eq!(schedule.interval_minutes, expected_interval);
                assert_eq!(*remote_folder_mode, mode);
            }
            _ => panic!("remote folder config"),
        }
        match &folders[1].automatic_config {
            AutomaticSyncConfigV1::RemoteFolder {
                range, schedule, ..
            } => {
                assert_eq!(
                    *range,
                    ArchiveListRangeV1::RecentItems { count: 100 }
                );
                assert_eq!(schedule.interval_minutes, 180);
            }
            _ => panic!("remote folder config"),
        }
        assert_eq!(
            expansion
                .source_states
                .iter()
                .find(|state| state.source == SyncSourceV1::RemoteFolder)
                .and_then(|state| state.message.as_deref()),
            Some("favorites_automatic_discovery_disabled")
        );
    }

    let history_only = expand_source_instances(
        5,
        &settings,
        BrowserPreferenceV1::Auto,
        &capabilities(&[SyncSourceV1::History]),
        &registry,
    )
    .expect("history-only expansion");
    assert_eq!(history_only.source_states.len(), 4);
    assert!(history_only
        .instances
        .iter()
        .all(|instance| instance.task_key.source == SyncSourceV1::History));
}

#[test]
fn scheduler_instance_pages_bind_registry_and_portable_revisions() {
    let mut connection = database();
    let account_id = insert_account(&connection, 3);
    insert_remote_folders(&mut connection, account_id, 205);
    let root = temp_dir("pages");
    let mut portable = PortableSettingsV2 {
        revision: 7,
        ..PortableSettingsV2::default()
    };
    portable
        .accounts
        .insert("uid:uid-3".to_string(), AccountSettingsV2::default());
    write_portable_settings_atomic(&root, &portable).expect("portable settings");

    let first = list_scheduler_instances(
        &mut connection,
        &root,
        account_id,
        SyncSourceV1::RemoteFolder,
        None,
        200,
    )
    .expect("first page");
    assert_eq!(first.instances.len(), 200);
    assert!(first.has_more);
    assert_eq!(first.portable_settings_revision, 7);
    assert_eq!(first.registry_revision, 1);
    assert_eq!(first.instances[0].remote_folder_id, "folder-00000");
    let cursor = first.next_cursor.clone().expect("next cursor");

    let second = list_scheduler_instances(
        &mut connection,
        &root,
        account_id,
        SyncSourceV1::RemoteFolder,
        Some(&cursor),
        200,
    )
    .expect("second page");
    assert_eq!(second.instances.len(), 5);
    assert!(!second.has_more);
    assert_eq!(second.instances[4].remote_folder_id, "folder-00204");

    connection
        .execute(
            "UPDATE remote_folder_registry_state SET revision = revision + 1 WHERE account_id = ?1",
            [account_id],
        )
        .unwrap();
    assert_eq!(
        list_scheduler_instances(
            &mut connection,
            &root,
            account_id,
            SyncSourceV1::RemoteFolder,
            Some(&cursor),
            200,
        )
        .unwrap_err(),
        "scheduler_instances_changed"
    );

    let current = list_scheduler_instances(
        &mut connection,
        &root,
        account_id,
        SyncSourceV1::RemoteFolder,
        None,
        200,
    )
    .expect("current first page");
    portable.revision = 8;
    write_portable_settings_atomic(&root, &portable).expect("changed portable settings");
    assert_eq!(
        list_scheduler_instances(
            &mut connection,
            &root,
            account_id,
            SyncSourceV1::RemoteFolder,
            current.next_cursor.as_deref(),
            200,
        )
        .unwrap_err(),
        "scheduler_instances_changed"
    );

    let mut reads = 0;
    let first_snapshot = portable.clone();
    let mut changed_snapshot = portable.clone();
    changed_snapshot.revision += 1;
    assert_eq!(
        list_scheduler_instances_with_settings_loader(
            &mut connection,
            account_id,
            SyncSourceV1::RemoteFolder,
            None,
            20,
            || {
                reads += 1;
                Ok(if reads == 1 {
                    first_snapshot.clone()
                } else {
                    changed_snapshot.clone()
                })
            },
        )
        .unwrap_err(),
        "scheduler_instances_changed"
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn automatic_reconciliation_is_source_specific_and_never_backfills_missed_periods() {
    let mut connection = database();
    let account_id = insert_account(&connection, 4);
    let mut settings = AccountSettingsV2::default();
    settings.history.range = douyin_history_archive::settings_store::HistoryRangeV1::Bottom;
    settings.history.schedule.enabled = true;
    settings.history.schedule.interval_minutes = 60;
    settings.likes.schedule.enabled = true;
    settings.likes.schedule.interval_minutes = 60;
    let caps = capabilities(&[SyncSourceV1::History, SyncSourceV1::Likes]);

    let manual_payload = freeze_task_payload(SyncTaskPayloadV1::History {
        range: settings.history.range,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: timestamp("2026-08-12T00:30:00.000Z").timestamp_millis(),
            cutoff_ms: None,
        },
        scroll_speed: settings.scroll_speed,
        hide_browser: settings.hide_browser,
        browser: browser(account_id),
    })
    .unwrap();
    let manual = QueueStore::enqueue_job(
        &mut connection,
        &NewSyncJobV1 {
            task_key: TaskKeyV1 {
                account_id,
                source: SyncSourceV1::History,
                source_instance_key: String::new(),
            },
            trigger: SyncTriggerV1::Manual,
            queued_at: "2026-08-12T00:30:00.000Z".to_string(),
            scheduled_for: None,
            settings_revision: 1,
            config_sha256: None,
            payload_json: manual_payload.payload_json,
            payload_sha256: manual_payload.payload_sha256,
            equivalence_sha256: manual_payload.equivalence_sha256,
        },
    )
    .unwrap();
    QueueStore::claim_job(
        &mut connection,
        manual.snapshot.id,
        "2026-08-12T00:40:00.000Z",
    )
    .unwrap();
    QueueStore::finish_running_job(
        &mut connection,
        manual.snapshot.id,
        &TerminalJobUpdateV1 {
            status: SyncJobStatusV1::Success,
            finished_at: "2026-08-12T01:00:00.000Z".to_string(),
            inserted_videos: 0,
            inserted_records: 0,
            error_code: None,
            error_message: None,
        },
    )
    .unwrap();

    let input = |account_settings: AccountSettingsV2, now: &str| AutomaticReconcileInputV1 {
        account_id,
        settings_revision: 1,
        account_settings,
        browser_preference: BrowserPreferenceV1::Auto,
        browser: browser(account_id),
        capabilities: caps.clone(),
        app_ready_at: timestamp(APP_READY),
        startup_delay_minutes: 5,
        now: timestamp(now),
    };

    let mut mismatched_browser = input(settings.clone(), "2026-08-12T04:00:00.000Z");
    mismatched_browser.browser_preference = BrowserPreferenceV1::Edge;
    assert_eq!(
        reconcile_automatic_jobs(&mut connection, &mismatched_browser).unwrap_err(),
        "browser_preference_mismatch"
    );
    assert!(QueueStore::list_queued_jobs(&connection)
        .unwrap()
        .is_empty());

    let first = reconcile_automatic_jobs(
        &mut connection,
        &input(settings.clone(), "2026-08-12T04:00:00.000Z"),
    )
    .expect("first reconciliation");
    assert_eq!(first.created_job_ids.len(), 2);
    assert!(first.cancelled_job_ids.is_empty());
    let history_state = first
        .source_states
        .iter()
        .find(|state| state.source == SyncSourceV1::History)
        .unwrap();
    assert_eq!(history_state.last_status, Some(SyncJobStatusV1::Success));
    assert_eq!(
        history_state.last_finished_at.as_deref(),
        Some("2026-08-12T01:00:00.000Z")
    );
    assert_eq!(
        history_state.next_due_at.as_deref(),
        Some("2026-08-12T02:00:00.000Z")
    );
    assert!(history_state.active_job_id.is_some());

    let repeated = reconcile_automatic_jobs(
        &mut connection,
        &input(settings.clone(), "2026-08-12T08:00:00.000Z"),
    )
    .expect("missed periods remain one job");
    assert!(repeated.created_job_ids.is_empty());
    assert!(repeated.cancelled_job_ids.is_empty());
    let active_before = QueueStore::list_queued_jobs(&connection).unwrap();
    assert_eq!(active_before.len(), 2);
    let history_before = active_before
        .iter()
        .find(|job| job.snapshot.task_key.source == SyncSourceV1::History)
        .unwrap()
        .snapshot
        .id;
    let likes_before = active_before
        .iter()
        .find(|job| job.snapshot.task_key.source == SyncSourceV1::Likes)
        .unwrap()
        .snapshot
        .id;

    settings.history.range =
        douyin_history_archive::settings_store::HistoryRangeV1::Days { days: 14 };
    let changed = reconcile_automatic_jobs(
        &mut connection,
        &input(settings.clone(), "2026-08-12T08:01:00.000Z"),
    )
    .expect("changed history config");
    assert_eq!(changed.cancelled_job_ids, vec![history_before]);
    assert_eq!(changed.created_job_ids.len(), 1);
    let active_after = QueueStore::list_queued_jobs(&connection).unwrap();
    assert_eq!(active_after.len(), 2);
    assert!(active_after
        .iter()
        .any(|job| job.snapshot.id == likes_before));
    let replacement = active_after
        .iter()
        .find(|job| job.snapshot.task_key.source == SyncSourceV1::History)
        .unwrap()
        .snapshot
        .id;
    assert_ne!(replacement, history_before);

    settings.history.schedule.enabled = false;
    let disabled = reconcile_automatic_jobs(
        &mut connection,
        &input(settings, "2026-08-12T08:02:00.000Z"),
    )
    .expect("disabled history");
    assert_eq!(disabled.cancelled_job_ids, vec![replacement]);
    assert!(disabled.created_job_ids.is_empty());
    assert_eq!(QueueStore::list_queued_jobs(&connection).unwrap().len(), 1);
    assert_eq!(
        connection
            .query_row(
                "SELECT error_code FROM sync_jobs WHERE id = ?1",
                [replacement],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "automatic_settings_changed"
    );
}

#[test]
fn scheduler_state_includes_account_login_status_all_capabilities_and_source_runtime_fields() {
    let mut connection = database();
    let active_account = insert_account(&connection, 21);
    let expired_account = insert_account(&connection, 22);
    connection
        .execute(
            "UPDATE accounts SET status = 'login_expired' WHERE id = ?1",
            [expired_account],
        )
        .expect("expire account");

    let mut active_settings = AccountSettingsV2::default();
    active_settings.history.schedule.enabled = true;
    active_settings.history.schedule.interval_minutes = 60;
    let expired_settings = AccountSettingsV2::default();
    let caps = capabilities(&[SyncSourceV1::History]);
    let reconciled = reconcile_automatic_jobs(
        &mut connection,
        &AutomaticReconcileInputV1 {
            account_id: active_account,
            settings_revision: 7,
            account_settings: active_settings.clone(),
            browser_preference: BrowserPreferenceV1::Auto,
            browser: browser(active_account),
            capabilities: caps.clone(),
            app_ready_at: timestamp(APP_READY),
            startup_delay_minutes: 5,
            now: timestamp("2026-08-12T00:05:00.000Z"),
        },
    )
    .expect("queue due history");
    assert_eq!(reconciled.created_job_ids.len(), 1);

    let runtime = SchedulerRuntimeSnapshotV1 {
        worker_state: SchedulerWorkerStateV1::Waiting,
        next_wake_at: Some("2026-08-12T01:05:00.000Z".to_string()),
        wake_generation: 3,
    };
    let state = build_scheduler_state(
        &connection,
        false,
        &runtime,
        &caps,
        &[
            SchedulerAccountInputV1 {
                account_id: active_account,
                account_status: "active".to_string(),
                account_settings: active_settings,
                browser_preference: BrowserPreferenceV1::Auto,
            },
            SchedulerAccountInputV1 {
                account_id: expired_account,
                account_status: "login_expired".to_string(),
                account_settings: expired_settings,
                browser_preference: BrowserPreferenceV1::Edge,
            },
        ],
        timestamp(APP_READY),
        5,
    )
    .expect("scheduler state");

    assert!(!state.paused);
    assert_eq!(state.worker_state, SchedulerWorkerStateV1::Waiting);
    assert_eq!(state.capabilities.len(), 4);
    assert_eq!(
        state
            .capabilities
            .iter()
            .map(|capability| capability.source)
            .collect::<Vec<_>>(),
        vec![
            SyncSourceV1::History,
            SyncSourceV1::Likes,
            SyncSourceV1::Favorites,
            SyncSourceV1::RemoteFolder,
        ]
    );
    assert_eq!(state.accounts.len(), 2);
    let active = state
        .accounts
        .iter()
        .find(|account| account.account_id == active_account)
        .expect("active account state");
    assert_eq!(active.account_status, "active");
    let history = active
        .source_states
        .iter()
        .find(|source| source.source == SyncSourceV1::History)
        .expect("history state");
    assert!(history.enabled);
    assert_eq!(history.interval_minutes, Some(60));
    assert_eq!(history.active_job_id, Some(reconciled.created_job_ids[0]));
    assert_eq!(
        history.next_due_at.as_deref(),
        Some("2026-08-12T00:05:00.000Z")
    );
    let expired = state
        .accounts
        .iter()
        .find(|account| account.account_id == expired_account)
        .expect("expired account state");
    assert_eq!(expired.account_status, "login_expired");
    assert_eq!(expired.source_states.len(), 4);
}
