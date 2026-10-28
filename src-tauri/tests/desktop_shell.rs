use douyin_history_archive::{
    device_settings::DeviceSettingsStore,
    desktop_lifecycle::{
        close_window_decision, second_instance_decision, tray_menu_decision, tray_sync_scope,
        CloseWindowDecisionInputV1, CloseWindowDecisionV1, DesktopIntentState,
        GuiActivationListener,
        SecondInstanceDecisionV1, TrayCommandV1, TrayMenuDecisionInputV1, TrayMenuItemIdV1,
    },
    notifications::{
        consecutive_automatic_failure_job_ids, observe_terminal_notification,
        terminal_notification_decision,
        NotificationKindV1, NotificationPolicyDecisionV1, TerminalNotificationInputV1,
    },
    process_lock::GuiActivationEvent,
    settings_store::CloseBehaviorV1,
    sync::{
        NewSyncJobV1, OperationAdmissionMode, QueueStore, SyncJobStatusV1, SyncScopeV1,
        SyncSourceV1, SyncTriggerV1, TaskKeyV1, TerminalJobUpdateV1,
    },
    migrate_schema_to_latest_at_startup,
};
use rusqlite::Connection;
use std::{fs, time::{Duration, SystemTime}};

#[test]
fn second_manual_launch_activates_the_existing_instance_but_autostart_is_silent() {
    assert_eq!(
        second_instance_decision(false),
        SecondInstanceDecisionV1::SignalExisting
    );
    assert_eq!(
        second_instance_decision(true),
        SecondInstanceDecisionV1::ExitSilently
    );
}

#[test]
fn gui_activation_event_is_auto_reset() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let name = format!("Local\\LifeArchiveProject.Emily.TestActivation.{unique}");
    let receiver = GuiActivationEvent::create_named(&name).expect("create activation event");
    let sender = GuiActivationEvent::open_named(&name).expect("open activation event");

    sender.signal().expect("signal existing instance");
    assert!(receiver
        .wait_timeout(Duration::from_millis(100))
        .expect("receive activation"));
    assert!(!receiver
        .wait_timeout(Duration::from_millis(10))
        .expect("auto reset after one receiver"));
}

#[test]
fn gui_activation_listener_stops_and_joins_without_detaching() {
    let name = format!(
        "Local\\LifeArchiveProject.Emily.Gui.Listener.Test.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let receiver = GuiActivationEvent::create_named(&name).expect("create activation event");
    let sender = GuiActivationEvent::open_named(&name).expect("open activation event");
    let activations = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = std::sync::Arc::clone(&activations);
    let listener = GuiActivationListener::start(receiver, move || {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    })
    .expect("start listener");

    sender.signal().expect("signal activation");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while activations.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "activation listener did not receive the signal"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    listener.stop();
    listener.join().expect("join listener");
    assert_eq!(activations.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn desktop_internal_flags_update_atomically_without_losing_each_other() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("emily-desktop-flags-{unique}"));
    let store = DeviceSettingsStore::open(root.clone()).expect("open device settings");

    assert!(store
        .mark_first_tray_hint_shown()
        .expect("persist first tray hint"));
    assert!(!store
        .mark_first_tray_hint_shown()
        .expect("repeat tray hint is idempotent"));
    let paused = store
        .set_scheduler_paused(true)
        .expect("persist scheduler pause");
    let key = douyin_history_archive::device_settings::notification_failure_key(
        42,
        "history",
        "",
    )
    .unwrap();
    let notified = store
        .set_notified_failure(&key, Some(9))
        .expect("persist notification streak");
    assert!(notified.first_tray_hint_shown);
    assert!(notified.scheduler_paused);
    assert_eq!(notified.notified_failure_streaks.get(&key), Some(&9));
    assert!(notified.revision > paused.revision);

    let cleared = store
        .set_notified_failure(&key, None)
        .expect("clear recovered streak");
    assert!(cleared.first_tray_hint_shown);
    assert!(cleared.scheduler_paused);
    assert!(!cleared.notified_failure_streaks.contains_key(&key));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn desktop_intents_survive_pre_mount_events_and_are_consumed_once_per_revision() {
    let intents = DesktopIntentState::default();
    let requested = intents
        .request(true, true)
        .expect("record settings intent before frontend mount");
    assert_eq!(requested.revision, 1);
    assert!(requested.show_main);
    assert!(requested.open_settings);

    let first = intents.take(0).expect("consume pending intent");
    assert_eq!(first, requested);
    assert_eq!(
        intents.take(0).expect("duplicate event is idempotent"),
        douyin_history_archive::desktop_lifecycle::DesktopIntentSnapshotV1 {
            revision: 1,
            show_main: false,
            open_settings: false,
        }
    );

    let next = intents
        .request(true, false)
        .expect("record a later activation");
    assert_eq!(next.revision, 2);
    assert_eq!(intents.take(1).expect("consume later intent"), next);
}

#[test]
fn desktop_intents_coalesce_without_losing_open_settings() {
    let intents = DesktopIntentState::default();
    intents.request(true, true).unwrap();
    intents.request(true, false).unwrap();

    let snapshot = intents.take(0).unwrap();
    assert_eq!(snapshot.revision, 2);
    assert!(snapshot.show_main);
    assert!(snapshot.open_settings);
}

#[test]
fn desktop_intent_pending_snapshot_does_not_consume_startup_activation() {
    let intents = DesktopIntentState::default();
    let requested = intents.request(true, false).expect("request activation");

    assert_eq!(intents.pending().expect("peek pending"), requested);
    assert_eq!(intents.take(0).expect("consume activation"), requested);
}

#[test]
fn close_decision_respects_onboarding_tray_and_system_exit_boundaries() {
    let base = CloseWindowDecisionInputV1 {
        close_behavior: CloseBehaviorV1::Tray,
        tray_available: true,
        onboarding_completed: true,
        first_tray_hint_shown: false,
        system_exit: false,
    };
    assert_eq!(
        close_window_decision(base),
        CloseWindowDecisionV1::HideToTray { show_hint: true }
    );
    assert_eq!(
        close_window_decision(CloseWindowDecisionInputV1 {
            first_tray_hint_shown: true,
            ..base
        }),
        CloseWindowDecisionV1::HideToTray { show_hint: false }
    );
    assert_eq!(
        close_window_decision(CloseWindowDecisionInputV1 {
            tray_available: false,
            ..base
        }),
        CloseWindowDecisionV1::KeepVisibleTrayUnavailable
    );
    assert_eq!(
        close_window_decision(CloseWindowDecisionInputV1 {
            onboarding_completed: false,
            ..base
        }),
        CloseWindowDecisionV1::RequestExit
    );
    assert_eq!(
        close_window_decision(CloseWindowDecisionInputV1 {
            close_behavior: CloseBehaviorV1::Exit,
            ..base
        }),
        CloseWindowDecisionV1::RequestExit
    );
    assert_eq!(
        close_window_decision(CloseWindowDecisionInputV1 {
            system_exit: true,
            ..base
        }),
        CloseWindowDecisionV1::RequestExit
    );
}

#[test]
fn tray_menu_has_six_fixed_items_and_disables_sync_during_non_open_modes() {
    let open = tray_menu_decision(TrayMenuDecisionInputV1 {
        runtime_ready: true,
        current_account_active: true,
        has_active_accounts: true,
        admission_mode: OperationAdmissionMode::Open,
        scheduler_paused: false,
    });
    assert_eq!(open.len(), 6);
    assert_eq!(
        open.iter().map(|item| item.id).collect::<Vec<_>>(),
        vec![
            TrayMenuItemIdV1::OpenMain,
            TrayMenuItemIdV1::SyncCurrentAccount,
            TrayMenuItemIdV1::SyncAllAccounts,
            TrayMenuItemIdV1::PauseAutomatic,
            TrayMenuItemIdV1::OpenSettings,
            TrayMenuItemIdV1::Exit,
        ]
    );
    assert!(open[1].enabled);
    assert!(open[2].enabled);
    assert_eq!(open[3].checked, Some(false));

    for mode in [
        OperationAdmissionMode::MaintenancePending,
        OperationAdmissionMode::MaintenanceRunning,
        OperationAdmissionMode::ExitPending,
        OperationAdmissionMode::Exiting,
    ] {
        let menu = tray_menu_decision(TrayMenuDecisionInputV1 {
            runtime_ready: true,
            current_account_active: true,
            has_active_accounts: true,
            admission_mode: mode,
            scheduler_paused: true,
        });
        assert!(!menu[1].enabled, "mode {mode:?}");
        assert!(!menu[2].enabled, "mode {mode:?}");
        assert_eq!(menu[3].checked, Some(true));
    }

    let no_account = tray_menu_decision(TrayMenuDecisionInputV1 {
        runtime_ready: true,
        current_account_active: false,
        has_active_accounts: false,
        admission_mode: OperationAdmissionMode::Open,
        scheduler_paused: false,
    });
    assert!(!no_account[1].enabled);
    assert!(!no_account[2].enabled);

    let another_account_available = tray_menu_decision(TrayMenuDecisionInputV1 {
        runtime_ready: true,
        current_account_active: false,
        has_active_accounts: true,
        admission_mode: OperationAdmissionMode::Open,
        scheduler_paused: false,
    });
    assert!(!another_account_available[1].enabled);
    assert!(another_account_available[2].enabled);

    let starting = tray_menu_decision(TrayMenuDecisionInputV1 {
        runtime_ready: false,
        current_account_active: true,
        has_active_accounts: true,
        admission_mode: OperationAdmissionMode::Open,
        scheduler_paused: false,
    });
    assert!(!starting[1].enabled);
    assert!(!starting[2].enabled);
    assert!(!starting[3].enabled);
}

#[test]
fn tray_sync_commands_use_the_bulk_queue_scopes() {
    assert_eq!(
        tray_sync_scope(TrayCommandV1::SyncCurrentAccount),
        Some(SyncScopeV1::CurrentAccountAvailableSources)
    );
    assert_eq!(
        tray_sync_scope(TrayCommandV1::SyncAllAccounts),
        Some(SyncScopeV1::AllActiveAccounts)
    );
    assert_eq!(tray_sync_scope(TrayCommandV1::OpenMain), None);
}

fn notification_input(
    job_id: i64,
    status: SyncJobStatusV1,
    error_code: Option<&str>,
    consecutive_failure_job_ids: &[i64],
    prior_notified_job_id: Option<i64>,
) -> TerminalNotificationInputV1 {
    TerminalNotificationInputV1 {
        task_key: TaskKeyV1 {
            account_id: 42,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
        },
        job_id,
        trigger: SyncTriggerV1::Automatic,
        status,
        error_code: error_code.map(str::to_string),
        consecutive_failure_job_ids: consecutive_failure_job_ids.to_vec(),
        prior_notified_job_id,
    }
}

#[test]
fn final_login_expiry_notifies_once_until_the_task_recovers() {
    assert_eq!(
        terminal_notification_decision(&notification_input(
            7,
            SyncJobStatusV1::Failed,
            Some("login_recovery_failed"),
            &[7],
            None,
        )),
        NotificationPolicyDecisionV1::Notify(NotificationKindV1::LoginExpired)
    );
    assert_eq!(
        terminal_notification_decision(&notification_input(
            8,
            SyncJobStatusV1::Failed,
            Some("login_recovery_failed"),
            &[8, 7],
            Some(7),
        )),
        NotificationPolicyDecisionV1::Keep
    );
    assert_eq!(
        terminal_notification_decision(&notification_input(
            9,
            SyncJobStatusV1::Success,
            None,
            &[],
            Some(7),
        )),
        NotificationPolicyDecisionV1::Reset
    );
}

#[test]
fn third_consecutive_automatic_failure_notifies_once_per_task_streak() {
    assert_eq!(
        terminal_notification_decision(&notification_input(
            2,
            SyncJobStatusV1::Failed,
            Some("network_error"),
            &[2, 1],
            None,
        )),
        NotificationPolicyDecisionV1::Keep
    );
    assert_eq!(
        terminal_notification_decision(&notification_input(
            3,
            SyncJobStatusV1::Failed,
            Some("network_error"),
            &[3, 2, 1],
            None,
        )),
        NotificationPolicyDecisionV1::Notify(NotificationKindV1::AutomaticFailureStreak)
    );
    assert_eq!(
        terminal_notification_decision(&notification_input(
            4,
            SyncJobStatusV1::Failed,
            Some("network_error"),
            &[4, 3, 2],
            Some(3),
        )),
        NotificationPolicyDecisionV1::Keep
    );
    assert_eq!(
        terminal_notification_decision(&notification_input(
            7,
            SyncJobStatusV1::Failed,
            Some("network_error"),
            &[7, 6, 5],
            Some(3),
        )),
        NotificationPolicyDecisionV1::Keep
    );
}

#[test]
fn manual_jobs_and_non_failure_terminal_states_do_not_notify() {
    let mut manual = notification_input(
        3,
        SyncJobStatusV1::Failed,
        Some("network_error"),
        &[3, 2, 1],
        None,
    );
    manual.trigger = SyncTriggerV1::Manual;
    assert_eq!(
        terminal_notification_decision(&manual),
        NotificationPolicyDecisionV1::Keep
    );

    for status in [
        SyncJobStatusV1::Success,
        SyncJobStatusV1::Partial,
        SyncJobStatusV1::Cancelled,
    ] {
        assert_eq!(
            terminal_notification_decision(&notification_input(
                5,
                status,
                None,
                &[],
                Some(3),
            )),
            NotificationPolicyDecisionV1::Reset
        );
    }
}

#[test]
fn consecutive_failure_window_uses_the_complete_task_key_and_resets_on_success() {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_schema_to_latest_at_startup(&mut connection).expect("schema");
    connection
        .execute(
            "INSERT INTO accounts(uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES ('uid-notify', 'sec-notify', '00000000000000000000000000000042',
                     'active', 'x', 'x')",
            [],
        )
        .expect("account");
    let account_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)",
            [account_id],
        )
        .expect("registry state");
    let history_key = TaskKeyV1 {
        account_id,
        source: SyncSourceV1::History,
        source_instance_key: String::new(),
    };
    let likes_key = TaskKeyV1 {
        account_id,
        source: SyncSourceV1::Likes,
        source_instance_key: String::new(),
    };

    fn finish(
        connection: &mut Connection,
        task_key: &TaskKeyV1,
        status: SyncJobStatusV1,
        index: i64,
    ) -> i64 {
        let queued = QueueStore::enqueue_job(
            connection,
            &NewSyncJobV1 {
                task_key: task_key.clone(),
                trigger: SyncTriggerV1::Automatic,
                queued_at: format!("2026-08-12T00:00:{index:02}.000Z"),
                scheduled_for: Some(format!("2026-08-12T00:01:{index:02}.000Z")),
                settings_revision: 1,
                config_sha256: Some("c".repeat(64)),
                payload_json: "{}".to_string(),
                payload_sha256: format!("{index:064x}"),
                equivalence_sha256: format!("{index:064x}"),
            },
        )
        .expect("enqueue");
        QueueStore::claim_job(
            connection,
            queued.snapshot.id,
            &format!("2026-08-12T00:02:{index:02}.000Z"),
        )
        .expect("claim");
        QueueStore::finish_running_job(
            connection,
            queued.snapshot.id,
            &TerminalJobUpdateV1 {
                status,
                finished_at: format!("2026-08-12T00:03:{index:02}.000Z"),
                inserted_videos: 0,
                inserted_records: 0,
                error_code: (status == SyncJobStatusV1::Failed)
                    .then(|| "network_error".to_string()),
                error_message: None,
            },
        )
        .expect("finish");
        queued.snapshot.id
    }

    let first = finish(&mut connection, &history_key, SyncJobStatusV1::Failed, 1);
    finish(&mut connection, &likes_key, SyncJobStatusV1::Failed, 2);
    let second = finish(&mut connection, &history_key, SyncJobStatusV1::Failed, 3);
    let third = finish(&mut connection, &history_key, SyncJobStatusV1::Failed, 4);
    assert_eq!(
        consecutive_automatic_failure_job_ids(&connection, &history_key).expect("window"),
        vec![third, second, first]
    );

    finish(&mut connection, &history_key, SyncJobStatusV1::Success, 5);
    assert!(
        consecutive_automatic_failure_job_ids(&connection, &history_key)
            .expect("reset window")
            .is_empty()
    );
}

#[test]
fn terminal_notification_observer_persists_before_delivery_and_resets_after_recovery() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("emily-terminal-observer-{unique}"));
    let store = DeviceSettingsStore::open(root.clone()).expect("device settings");
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_schema_to_latest_at_startup(&mut connection).expect("schema");
    connection
        .execute(
            "INSERT INTO accounts(uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES ('uid-observer', 'sec-observer', '00000000000000000000000000000052',
                     'active', 'x', 'x')",
            [],
        )
        .expect("account");
    let account_id = connection.last_insert_rowid();
    let task_key = TaskKeyV1 {
        account_id,
        source: SyncSourceV1::History,
        source_instance_key: String::new(),
    };

    let first = finish_automatic_job(
        &mut connection,
        &task_key,
        SyncJobStatusV1::Failed,
        1,
    );
    let second = finish_automatic_job(
        &mut connection,
        &task_key,
        SyncJobStatusV1::Failed,
        2,
    );
    let third = finish_automatic_job(
        &mut connection,
        &task_key,
        SyncJobStatusV1::Failed,
        3,
    );
    assert!(first.snapshot.id < second.snapshot.id);

    let notification_key = douyin_history_archive::device_settings::notification_failure_key(
        account_id,
        "history",
        "",
    )
    .expect("notification key");
    let mut observed_persisted_id = None;
    let delivery_error = observe_terminal_notification(
        &connection,
        &store,
        &third,
        |kind| {
            assert_eq!(kind, NotificationKindV1::AutomaticFailureStreak);
            observed_persisted_id = store
                .load()
                .expect("persisted before notification")
                .notified_failure_streaks
                .get(&notification_key)
                .copied();
            Err("notification backend unavailable".to_string())
        },
    )
    .expect_err("delivery failure remains observable");
    assert_eq!(delivery_error, "notification_delivery_failed");
    assert_eq!(observed_persisted_id, Some(third.snapshot.id));
    assert_eq!(
        store
            .load()
            .expect("persisted delivery dedupe")
            .notified_failure_streaks
            .get(&notification_key),
        Some(&third.snapshot.id)
    );

    let recovered = finish_automatic_job(
        &mut connection,
        &task_key,
        SyncJobStatusV1::Success,
        4,
    );
    observe_terminal_notification(&connection, &store, &recovered, |_| {
        panic!("recovery must not emit a notification")
    })
    .expect("reset notification streak");
    assert!(!store
        .load()
        .expect("reset state")
        .notified_failure_streaks
        .contains_key(&notification_key));
    let _ = fs::remove_dir_all(root);
}

fn finish_automatic_job(
    connection: &mut Connection,
    task_key: &TaskKeyV1,
    status: SyncJobStatusV1,
    index: i64,
) -> douyin_history_archive::sync::StoredSyncJobV1 {
    let queued = QueueStore::enqueue_job(
        connection,
        &NewSyncJobV1 {
            task_key: task_key.clone(),
            trigger: SyncTriggerV1::Automatic,
            queued_at: format!("2026-08-12T01:00:{index:02}.000Z"),
            scheduled_for: Some(format!("2026-08-12T01:01:{index:02}.000Z")),
            settings_revision: 1,
            config_sha256: Some("d".repeat(64)),
            payload_json: "{}".to_string(),
            payload_sha256: format!("{:064x}", index + 100),
            equivalence_sha256: format!("{:064x}", index + 100),
        },
    )
    .expect("enqueue");
    QueueStore::claim_job(
        connection,
        queued.snapshot.id,
        &format!("2026-08-12T01:02:{index:02}.000Z"),
    )
    .expect("claim");
    QueueStore::finish_running_job(
        connection,
        queued.snapshot.id,
        &TerminalJobUpdateV1 {
            status,
            finished_at: format!("2026-08-12T01:03:{index:02}.000Z"),
            inserted_videos: 0,
            inserted_records: 0,
            error_code: (status == SyncJobStatusV1::Failed)
                .then(|| "network_error".to_string()),
            error_message: None,
        },
    )
    .expect("finish")
}
