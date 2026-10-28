use douyin_history_archive::{
    collector::browser_state::{
        clear_browser_selection, ensure_account_profile_root, ensure_profile_generation,
        read_browser_selection, write_browser_selection, BrowserId, BrowserSelection,
    },
    desktop_ipc::{
        build_desktop_state_view, commit_onboarding_step, update_account_defaults_for_capabilities,
        BeginAccountLoginRequestV1, CommitOnboardingStepRequestV1, OnboardingStepRequestV1,
        SetAutostartRequestV1, SetBrowserPreferenceRequestV1,
    },
    device_settings::{
        BrowserPreferenceV1, DeviceAccountSettingsV1, DeviceSettingsStore,
        DeviceSettingsV1, DeviceSettingsViewV1,
    },
    settings_store::{
        load_portable_settings, AccountSettingsV2, ArchiveListRangeV1, CloseBehaviorV1,
    },
    sync::{
        CapabilityRegistry, EnqueueSyncRequestV1, QueueStore, SchedulerRuntimeStateV1,
        SchedulerTickResultV1, SchedulerWaitOutcomeV1, SchedulerWorkerConfigV1,
        SchedulerWorkerHandle, SchedulerWorkerStateV1, SyncBatchStatusV1, SyncScopeV1,
        SyncSourceV1,
    },
};
use rusqlite::Connection;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

fn temp_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-desktop-ipc-{label}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("temp root");
    root
}

#[test]
fn ipc_requests_are_camel_case_and_reject_unknown_fields() {
    let request = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::One,
        account_id: Some(7),
        source: Some(SyncSourceV1::History),
        source_instance_key: Some(String::new()),
        payload: None,
        full_reconcile_confirmation_sha256: None,
    };
    assert_eq!(
        serde_json::to_value(&request).expect("serialize request"),
        json!({
            "scope": "one",
            "accountId": 7,
            "source": "history",
            "sourceInstanceKey": "",
            "payload": null,
            "fullReconcileConfirmationSha256": null
        })
    );
    assert!(serde_json::from_value::<EnqueueSyncRequestV1>(json!({
        "scope": "one",
        "accountId": 7,
        "source": "history",
        "sourceInstanceKey": "",
        "payload": null,
        "fullReconcileConfirmationSha256": null,
        "unexpected": true
    }))
    .is_err());

    let onboarding = CommitOnboardingStepRequestV1 {
        expected_portable_revision: 3,
        expected_device_revision: 4,
        step: OnboardingStepRequestV1::Account { skipped: true },
    };
    assert_eq!(
        serde_json::to_value(&onboarding).expect("serialize onboarding"),
        json!({
            "expectedPortableRevision": 3,
            "expectedDeviceRevision": 4,
            "step": { "kind": "account", "skipped": true }
        })
    );
    assert!(
        serde_json::from_value::<CommitOnboardingStepRequestV1>(json!({
            "expectedPortableRevision": 3,
            "expectedDeviceRevision": 4,
            "step": { "kind": "account", "skipped": true, "extra": 1 }
        }))
        .is_err()
    );

    let browser = SetBrowserPreferenceRequestV1 {
        account_id: 7,
        browser_preference: BrowserPreferenceV1::Chrome,
        confirm_relogin: true,
    };
    assert_eq!(
        serde_json::to_value(&browser).expect("serialize browser preference"),
        json!({
            "accountId": 7,
            "browserPreference": "chrome",
            "confirmRelogin": true
        })
    );
    assert!(
        serde_json::from_value::<SetBrowserPreferenceRequestV1>(json!({
            "accountId": 7,
            "browserPreference": "edge",
            "confirmRelogin": true,
            "unexpected": true
        }))
        .is_err()
    );

    let login = BeginAccountLoginRequestV1 {
        browser_preference: Some(BrowserPreferenceV1::Edge),
    };
    assert_eq!(
        serde_json::to_value(&login).expect("serialize login request"),
        json!({ "browserPreference": "edge" })
    );
    assert_eq!(
        serde_json::from_value::<BeginAccountLoginRequestV1>(json!({}))
            .expect("optional browser preference"),
        BeginAccountLoginRequestV1::default()
    );
    assert!(serde_json::from_value::<BeginAccountLoginRequestV1>(json!({
        "browserPreference": "chrome",
        "unexpected": true
    }))
    .is_err());

    let autostart = SetAutostartRequestV1 {
        expected_device_revision: 9,
        enabled: true,
        silent: true,
    };
    assert_eq!(
        serde_json::to_value(&autostart).expect("serialize autostart"),
        json!({
            "expectedDeviceRevision": 9,
            "enabled": true,
            "silent": true
        })
    );
    assert!(serde_json::from_value::<SetAutostartRequestV1>(json!({
        "expectedDeviceRevision": 9,
        "enabled": true,
        "silent": true,
        "unexpected": true
    }))
    .is_err());
}

#[test]
fn desktop_state_view_exposes_only_frontend_initialization_state() {
    let mut persisted = DeviceSettingsV1::default();
    persisted.revision = 1;
    persisted.onboarding_step = 2;
    persisted.launch_at_startup = true;
    persisted.silent_autostart = true;
    persisted.scheduler_paused = true;
    persisted.first_tray_hint_shown = true;
    persisted
        .notified_failure_streaks
        .insert("private".to_string(), 3);
    persisted.accounts.insert(
        "0123456789abcdef0123456789abcdef".to_string(),
        DeviceAccountSettingsV1 {
            browser_preference: BrowserPreferenceV1::Chrome,
        },
    );

    let view = build_desktop_state_view(
        persisted,
        false,
        Some("tray_unavailable".to_string()),
        Some("autostart_repair_failed".to_string()),
    );
    let json = serde_json::to_value(view).expect("serialize desktop state");

    assert_eq!(json["device"]["revision"], 1);
    assert_eq!(json["device"]["onboardingStep"], 2);
    assert_eq!(json["device"]["launchAtStartup"], true);
    assert_eq!(json["device"]["silentAutostart"], true);
    assert_eq!(json["device"]["schedulerPaused"], true);
    assert_eq!(
        json["device"]["accounts"]["0123456789abcdef0123456789abcdef"]
            ["browserPreference"],
        "chrome"
    );
    assert_eq!(json["trayAvailable"], false);
    assert_eq!(json["trayError"], "tray_unavailable");
    assert_eq!(json["autostartError"], "autostart_repair_failed");
    assert!(json["device"].get("firstTrayHintShown").is_none());
    assert!(json["device"].get("notifiedFailureStreaks").is_none());

}

#[test]
fn portable_settings_reject_recent_days_without_reliable_time_capability() {
    let root = temp_root("recent-days");
    let before = load_portable_settings(&root).expect("defaults");
    let mut defaults = AccountSettingsV2::default();
    defaults.likes.range = ArchiveListRangeV1::RecentDays { days: 3 };

    assert_eq!(
        update_account_defaults_for_capabilities(
            &root,
            defaults,
            CapabilityRegistry::stage_3_3().all(),
        )
        .expect_err("unsupported time range"),
        "time_range_not_supported"
    );
    assert_eq!(
        load_portable_settings(&root).expect("unchanged settings"),
        before
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn onboarding_commit_advances_once_and_keeps_portable_and_device_scopes_separate() {
    let root = temp_root("onboarding");
    let store = DeviceSettingsStore::open(root.clone()).expect("device store");
    let initial = store.load().expect("initial device settings");
    let request = CommitOnboardingStepRequestV1 {
        expected_portable_revision: 0,
        expected_device_revision: initial.revision,
        step: OnboardingStepRequestV1::Account { skipped: true },
    };

    let first = commit_onboarding_step(
        &root,
        &store,
        &request,
        CapabilityRegistry::stage_3_3().all(),
    )
    .expect("first commit");
    assert_eq!(first.portable_revision, 0);
    assert_eq!(first.device.onboarding_step, 1);
    assert!(!first.device.onboarding_completed);
    assert_eq!(
        first.device,
        DeviceSettingsViewV1::from(store.load().expect("persisted device"))
    );

    let repeated = commit_onboarding_step(
        &root,
        &store,
        &request,
        CapabilityRegistry::stage_3_3().all(),
    )
    .expect("idempotent repeat");
    assert_eq!(repeated, first);
    assert_eq!(load_portable_settings(&root).unwrap().revision, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn onboarding_commits_all_four_steps_in_order_and_invalid_settings_do_not_advance() {
    let root = temp_root("onboarding-four-steps");
    let store = DeviceSettingsStore::open(root.clone()).expect("device store");
    let capabilities = CapabilityRegistry::stage_3_3();

    let account = commit_onboarding_step(
        &root,
        &store,
        &CommitOnboardingStepRequestV1 {
            expected_portable_revision: 0,
            expected_device_revision: 0,
            step: OnboardingStepRequestV1::Account { skipped: true },
        },
        capabilities.all(),
    )
    .expect("account step");
    assert_eq!(account.device.onboarding_step, 1);

    let mut ranges = AccountSettingsV2::default();
    ranges.history.range = douyin_history_archive::settings_store::HistoryRangeV1::Bottom;
    let range = commit_onboarding_step(
        &root,
        &store,
        &CommitOnboardingStepRequestV1 {
            expected_portable_revision: account.portable_revision,
            expected_device_revision: account.device.revision,
            step: OnboardingStepRequestV1::SyncRange {
                account_defaults: ranges.clone(),
                sync_after_onboarding: true,
            },
        },
        capabilities.all(),
    )
    .expect("range step");
    assert_eq!(range.device.onboarding_step, 2);
    assert_eq!(
        load_portable_settings(&root)
            .expect("portable after range")
            .account_defaults
            .history
            .range,
        ranges.history.range
    );

    let mut unsupported = ranges.clone();
    unsupported.likes.range = ArchiveListRangeV1::RecentDays { days: 2 };
    let before_failure = store.load().expect("before invalid schedule");
    assert_eq!(
        commit_onboarding_step(
            &root,
            &store,
            &CommitOnboardingStepRequestV1 {
                expected_portable_revision: range.portable_revision,
                expected_device_revision: range.device.revision,
                step: OnboardingStepRequestV1::AutomaticSync {
                    account_defaults: unsupported,
                    startup_sync_delay_minutes: 5,
                },
            },
            capabilities.all(),
        )
        .expect_err("unsupported settings must fail"),
        "time_range_not_supported"
    );
    assert_eq!(store.load().expect("step unchanged"), before_failure);

    let automatic = commit_onboarding_step(
        &root,
        &store,
        &CommitOnboardingStepRequestV1 {
            expected_portable_revision: range.portable_revision,
            expected_device_revision: range.device.revision,
            step: OnboardingStepRequestV1::AutomaticSync {
                account_defaults: ranges,
                startup_sync_delay_minutes: 5,
            },
        },
        capabilities.all(),
    )
    .expect("automatic step");
    assert_eq!(automatic.device.onboarding_step, 3);
    assert!(!automatic.device.onboarding_completed);
    assert_eq!(
        load_portable_settings(&root)
            .expect("portable after automatic")
            .global
            .startup_sync_delay_minutes,
        5
    );

    let mut revised_ranges = load_portable_settings(&root)
        .expect("portable before revisiting range")
        .account_defaults;
    revised_ranges.history.range =
        douyin_history_archive::settings_store::HistoryRangeV1::Days { days: 30 };
    let revisited = commit_onboarding_step(
        &root,
        &store,
        &CommitOnboardingStepRequestV1 {
            expected_portable_revision: automatic.portable_revision,
            expected_device_revision: automatic.device.revision,
            step: OnboardingStepRequestV1::SyncRange {
                account_defaults: revised_ranges.clone(),
                sync_after_onboarding: false,
            },
        },
        capabilities.all(),
    )
    .expect("revisit committed range step");
    assert_eq!(revisited.device.onboarding_step, 3);
    assert!(!revisited.device.onboarding_completed);
    assert_eq!(
        load_portable_settings(&root)
            .expect("portable after revisiting range")
            .account_defaults,
        revised_ranges
    );

    let desktop = commit_onboarding_step(
        &root,
        &store,
        &CommitOnboardingStepRequestV1 {
            expected_portable_revision: revisited.portable_revision,
            expected_device_revision: revisited.device.revision,
            step: OnboardingStepRequestV1::DesktopBehavior {
                close_behavior: CloseBehaviorV1::Tray,
                launch_at_startup: false,
                silent_autostart: false,
            },
        },
        capabilities.all(),
    )
    .expect("desktop step");
    assert_eq!(desktop.device.onboarding_step, 3);
    assert!(desktop.device.onboarding_completed);
    assert_eq!(
        load_portable_settings(&root)
            .expect("portable after desktop")
            .global
            .close_behavior,
        CloseBehaviorV1::Tray
    );

    let repeated = commit_onboarding_step(
        &root,
        &store,
        &CommitOnboardingStepRequestV1 {
            expected_portable_revision: automatic.portable_revision,
            expected_device_revision: automatic.device.revision,
            step: OnboardingStepRequestV1::DesktopBehavior {
                close_behavior: CloseBehaviorV1::Tray,
                launch_at_startup: false,
                silent_autostart: false,
            },
        },
        capabilities.all(),
    )
    .expect("identical desktop retry is idempotent");
    assert_eq!(repeated, desktop);

    let stale_different = commit_onboarding_step(
        &root,
        &store,
        &CommitOnboardingStepRequestV1 {
            expected_portable_revision: automatic.portable_revision,
            expected_device_revision: automatic.device.revision,
            step: OnboardingStepRequestV1::DesktopBehavior {
                close_behavior: CloseBehaviorV1::Exit,
                launch_at_startup: false,
                silent_autostart: false,
            },
        },
        capabilities.all(),
    )
    .expect_err("different stale request is not idempotent");
    assert_eq!(stale_different, "onboarding_request_conflict");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn onboarding_retries_after_portable_commit_without_device_progress() {
    let root = temp_root("onboarding-partial-commit");
    let store = DeviceSettingsStore::open(root.clone()).expect("device store");
    let account = commit_onboarding_step(
        &root,
        &store,
        &CommitOnboardingStepRequestV1 {
            expected_portable_revision: 0,
            expected_device_revision: 0,
            step: OnboardingStepRequestV1::Account { skipped: true },
        },
        CapabilityRegistry::stage_3_3().all(),
    )
    .expect("account step");
    let mut defaults = AccountSettingsV2::default();
    defaults.history.range = douyin_history_archive::settings_store::HistoryRangeV1::Bottom;
    let request = CommitOnboardingStepRequestV1 {
        expected_portable_revision: account.portable_revision,
        expected_device_revision: account.device.revision,
        step: OnboardingStepRequestV1::SyncRange {
            account_defaults: defaults.clone(),
            sync_after_onboarding: false,
        },
    };

    update_account_defaults_for_capabilities(
        &root,
        defaults,
        CapabilityRegistry::stage_3_3().all(),
    )
    .expect("simulate portable commit before interruption");

    let retried = commit_onboarding_step(
        &root,
        &store,
        &request,
        CapabilityRegistry::stage_3_3().all(),
    )
    .expect("retry identical request");
    assert_eq!(retried.device.onboarding_step, 2);
    assert_eq!(retried.portable_revision, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scheduler_runtime_state_tracks_worker_lifecycle_without_a_gui_runtime() {
    let state = SchedulerRuntimeStateV1::new();
    assert_eq!(
        state.snapshot().unwrap().worker_state,
        SchedulerWorkerStateV1::Idle
    );
    state
        .mark_waiting(Some("2026-08-12T01:00:00.000Z".to_string()))
        .unwrap();
    let waiting = state.snapshot().unwrap();
    assert_eq!(waiting.worker_state, SchedulerWorkerStateV1::Waiting);
    assert_eq!(
        waiting.next_wake_at.as_deref(),
        Some("2026-08-12T01:00:00.000Z")
    );
    state.wake().unwrap();
    assert!(state.take_wake_generation().unwrap() > 0);
    state.stop().unwrap();
    assert_eq!(
        state.snapshot().unwrap().worker_state,
        SchedulerWorkerStateV1::Stopped
    );
}

#[test]
fn scheduler_wait_is_interrupted_by_wake_and_stop() {
    let state = SchedulerRuntimeStateV1::new();
    let initial_generation = state.take_wake_generation().expect("generation");
    let waiter = state.clone();
    let started = Instant::now();
    let wake_thread = thread::spawn(move || {
        waiter
            .wait_for_change(initial_generation, Duration::from_secs(5))
            .expect("wait for wake")
    });
    thread::sleep(Duration::from_millis(30));
    state.wake().expect("wake scheduler");
    assert_eq!(
        wake_thread.join().expect("join wake waiter"),
        SchedulerWaitOutcomeV1::Woken
    );
    assert!(started.elapsed() < Duration::from_secs(1));

    let generation = state.take_wake_generation().expect("next generation");
    let stop_waiter = state.clone();
    let stop_thread = thread::spawn(move || {
        stop_waiter
            .wait_for_change(generation, Duration::from_secs(5))
            .expect("wait for stop")
    });
    thread::sleep(Duration::from_millis(30));
    state.stop().expect("stop scheduler");
    assert_eq!(
        stop_thread.join().expect("join stop waiter"),
        SchedulerWaitOutcomeV1::Stopped
    );
}

#[test]
fn changing_browser_keeps_isolated_profiles_and_only_clears_the_selection_pointer() {
    let root = temp_root("browser-selection");
    let account_root = root.join("account");
    ensure_account_profile_root(&account_root).expect("account profile root");
    let edge_generation =
        ensure_profile_generation(&account_root, BrowserId::Edge).expect("edge generation");
    let chrome_generation =
        ensure_profile_generation(&account_root, BrowserId::Chrome).expect("chrome generation");
    let selection = BrowserSelection {
        schema_version: 3,
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files\Edge\msedge.exe".to_string(),
        profile_path: String::new(),
        profile_generation: Some(edge_generation),
        selected_at: "2026-08-12T00:00:00.000Z".to_string(),
    };
    write_browser_selection(&account_root, &selection).expect("selection");
    let edge_profile = account_root.join("edge");
    let chrome_profile = account_root.join("chrome");

    assert!(clear_browser_selection(&account_root).expect("clear selection"));
    assert_eq!(
        read_browser_selection(&account_root).expect("read selection"),
        None
    );
    assert!(edge_profile.join("profile-generation.json").is_file());
    assert!(chrome_profile.join("profile-generation.json").is_file());
    assert_eq!(
        fs::read_to_string(chrome_profile.join("profile-generation.json"))
            .expect("chrome generation file")
            .contains(&chrome_generation),
        true
    );
    assert!(!clear_browser_selection(&account_root).expect("idempotent clear"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn scheduler_worker_runs_ticks_wakes_early_and_stops_cleanly() {
    let ticks = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&ticks);
    let worker = SchedulerWorkerHandle::start(SchedulerWorkerConfigV1::new(
        Duration::from_secs(5),
        Arc::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(SchedulerTickResultV1 {
                paused: false,
                next_wake_at: None,
                queued_job_ids: Vec::new(),
            })
        }),
    ))
    .expect("start scheduler worker");

    let deadline = Instant::now() + Duration::from_secs(1);
    while ticks.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(ticks.load(Ordering::SeqCst), 1);

    worker.wake().expect("wake worker");
    let deadline = Instant::now() + Duration::from_secs(1);
    while ticks.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(ticks.load(Ordering::SeqCst), 2);

    worker.stop().expect("stop worker");
    assert!(worker
        .wait_until_stopped(Duration::from_secs(1))
        .expect("wait for scheduler"));
    worker.join().expect("join scheduler");
    assert_eq!(
        worker.snapshot().expect("stopped snapshot").worker_state,
        SchedulerWorkerStateV1::Stopped
    );
}

#[test]
fn scheduler_worker_survives_a_tick_error_and_runs_again() {
    let ticks = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&ticks);
    let worker = SchedulerWorkerHandle::start(SchedulerWorkerConfigV1::new(
        Duration::from_millis(20),
        Arc::new(move || {
            let current = observed.fetch_add(1, Ordering::SeqCst);
            if current == 0 {
                return Err("transient_scheduler_tick_failed".to_string());
            }
            Ok(SchedulerTickResultV1 {
                paused: false,
                next_wake_at: None,
                queued_job_ids: Vec::new(),
            })
        }),
    ))
    .expect("start scheduler worker");

    let deadline = Instant::now() + Duration::from_secs(1);
    while ticks.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ticks.load(Ordering::SeqCst) >= 2);

    worker.stop().expect("stop worker");
    assert!(worker
        .wait_until_stopped(Duration::from_secs(1))
        .expect("wait for scheduler"));
    worker.join().expect("join scheduler");
}

#[test]
fn scheduler_worker_wakes_at_the_reported_due_time_before_the_idle_limit() {
    let ticks = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&ticks);
    let worker = SchedulerWorkerHandle::start(SchedulerWorkerConfigV1::new(
        Duration::from_secs(5),
        Arc::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(SchedulerTickResultV1 {
                paused: false,
                next_wake_at: Some(
                    (chrono::Utc::now() + chrono::Duration::milliseconds(60))
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                ),
                queued_job_ids: Vec::new(),
            })
        }),
    ))
    .expect("start scheduler worker");

    let deadline = Instant::now() + Duration::from_secs(1);
    while ticks.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ticks.load(Ordering::SeqCst) >= 2);
    worker.stop().expect("stop worker");
    assert!(worker
        .wait_until_stopped(Duration::from_secs(1))
        .expect("wait for scheduler"));
    worker.join().expect("join scheduler");
}

#[test]
fn batch_queries_preserve_first_page_and_cursor_contracts() {
    let mut connection = Connection::open_in_memory().expect("database");
    douyin_history_archive::migrate_schema_to_v5_at_startup(&mut connection).expect("schema v5");
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T00:00:00.000Z",
    )
    .expect("batch");

    let summary = QueueStore::get_batch(&connection, batch_id).expect("summary");
    assert_eq!(summary.id, batch_id);
    assert_eq!(summary.status, SyncBatchStatusV1::Queued);
    let page =
        QueueStore::list_batch_entries(&connection, batch_id, None, 200).expect("first page");
    assert!(page.entries.is_empty());
    assert!(!page.has_more);
    assert!(!page.next_cursor.is_empty());
    let other_batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::AllActiveAccounts,
        "2026-08-12T00:00:01.000Z",
    )
    .expect("other batch");
    assert_eq!(
        QueueStore::list_batch_entries(&connection, other_batch_id, Some(&page.next_cursor), 200)
            .expect_err("cursor is bound to one batch"),
        "invalid_batch_cursor"
    );
}
