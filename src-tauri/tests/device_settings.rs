use douyin_history_archive::{
    device_settings::{
        initialize_device_settings_store, initialize_onboarding_state, notification_failure_key,
        BrowserPreferenceV1, DeviceAccountSettingsV1, DeviceSettingsStore, DeviceSettingsV1,
    },
    initialize_schema_v4,
    settings_store::PortableSettingsV2,
};
use rusqlite::{params, Connection};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-device-settings-{label}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_settings(root: &PathBuf, value: &DeviceSettingsV1) -> Vec<u8> {
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    fs::write(root.join("device-settings.json"), &bytes).unwrap();
    bytes
}

fn empty_database() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    initialize_schema_v4(&mut connection).unwrap();
    connection
}

#[test]
fn missing_device_settings_use_defaults_and_invalid_files_are_preserved() {
    let root = temp_root("contracts");
    let store = DeviceSettingsStore::open(root.clone()).unwrap();

    assert_eq!(store.load().unwrap(), DeviceSettingsV1::default());
    assert!(!root.join("device-settings.json").exists());

    let malformed = b"{ not-json".to_vec();
    fs::write(root.join("device-settings.json"), &malformed).unwrap();
    assert_eq!(store.load().unwrap_err(), "invalid_device_settings");
    assert_eq!(
        fs::read(root.join("device-settings.json")).unwrap(),
        malformed
    );

    let mut unknown = serde_json::to_value(DeviceSettingsV1::default()).unwrap();
    unknown["extra"] = serde_json::json!(true);
    let unknown_bytes = serde_json::to_vec_pretty(&unknown).unwrap();
    fs::write(root.join("device-settings.json"), &unknown_bytes).unwrap();
    assert_eq!(store.load().unwrap_err(), "invalid_device_settings");
    assert_eq!(
        fs::read(root.join("device-settings.json")).unwrap(),
        unknown_bytes
    );
}

#[test]
fn legacy_device_settings_v1_is_atomically_migrated_without_inventing_backup_state() {
    let root = temp_root("schema-v1-migration");
    let legacy = serde_json::json!({
        "schemaVersion": 1,
        "revision": 7,
        "onboardingCompleted": true,
        "onboardingStep": 3,
        "launchAtStartup": true,
        "silentAutostart": true,
        "schedulerPaused": false,
        "firstTrayHintShown": true,
        "notifiedFailureStreaks": {},
        "accounts": {}
    });
    fs::write(
        root.join("device-settings.json"),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();

    let migrated = DeviceSettingsStore::open(root.clone())
        .unwrap()
        .load()
        .unwrap();

    assert_eq!(migrated.schema_version, 4);
    assert_eq!(migrated.revision, 7);
    assert_eq!(migrated.backup_directory, None);
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("device-settings.json")).unwrap()).unwrap();
    assert_eq!(persisted["schemaVersion"], 4);
    assert_eq!(persisted["backupDirectory"], serde_json::Value::Null);
    assert_eq!(
        persisted["automaticBackup"],
        serde_json::json!({
            "lastAttemptAt": null,
            "lastSuccessAt": null,
            "lastFailureAt": null,
            "lastErrorCode": null
        })
    );
}

#[test]
fn legacy_device_settings_v2_preserves_backup_directory_and_adds_run_state() {
    let root = temp_root("schema-v2-migration");
    let backup_directory = root.join("external-backups");
    let legacy = serde_json::json!({
        "schemaVersion": 2,
        "revision": 9,
        "onboardingCompleted": true,
        "onboardingStep": 3,
        "launchAtStartup": false,
        "silentAutostart": false,
        "schedulerPaused": false,
        "firstTrayHintShown": true,
        "notifiedFailureStreaks": {},
        "backupDirectory": backup_directory,
        "accounts": {}
    });
    fs::write(
        root.join("device-settings.json"),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();

    let migrated = DeviceSettingsStore::open(root.clone())
        .unwrap()
        .load()
        .unwrap();

    assert_eq!(migrated.schema_version, 4);
    assert_eq!(migrated.revision, 9);
    assert_eq!(
        migrated.backup_directory.as_deref(),
        Some(root.join("external-backups").as_path())
    );
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("device-settings.json")).unwrap()).unwrap();
    assert_eq!(persisted["schemaVersion"], 4);
    assert_eq!(
        persisted["automaticBackup"],
        serde_json::json!({
            "lastAttemptAt": null,
            "lastSuccessAt": null,
            "lastFailureAt": null,
            "lastErrorCode": null
        })
    );
}

#[test]
fn device_settings_v3_migrates_to_v4_with_new_onboarding_step_semantics() {
    for (old_step, expected_step) in [(0, 0), (1, 1), (2, 1), (3, 2)] {
        let root = temp_root(&format!("schema-v3-step-{old_step}"));
        let mut legacy = serde_json::to_value(DeviceSettingsV1::default()).unwrap();
        legacy["schemaVersion"] = serde_json::json!(3);
        legacy["onboardingStep"] = serde_json::json!(old_step);
        fs::write(
            root.join("device-settings.json"),
            serde_json::to_vec_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let migrated = DeviceSettingsStore::open(root.clone()).unwrap().load().unwrap();
        assert_eq!(migrated.schema_version, 4);
        assert_eq!(migrated.onboarding_step, expected_step);
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("device-settings.json")).unwrap()).unwrap();
        assert_eq!(persisted["schemaVersion"], 4);
        assert_eq!(persisted["onboardingStep"], expected_step);
    }

    let root = temp_root("schema-v3-completed");
    let mut completed = serde_json::to_value(DeviceSettingsV1::default()).unwrap();
    completed["schemaVersion"] = serde_json::json!(3);
    completed["onboardingCompleted"] = serde_json::json!(true);
    completed["onboardingStep"] = serde_json::json!(3);
    fs::write(
        root.join("device-settings.json"),
        serde_json::to_vec_pretty(&completed).unwrap(),
    )
    .unwrap();
    let migrated = DeviceSettingsStore::open(root).unwrap().load().unwrap();
    assert!(migrated.onboarding_completed);
    assert_eq!(migrated.onboarding_step, 3);
}

#[test]
fn custom_backup_directory_is_device_local_absolute_and_not_a_filesystem_root() {
    let root = temp_root("backup-directory");
    let store = DeviceSettingsStore::open(root.clone()).unwrap();
    let custom = root.join("external-backups");

    let saved = store.set_backup_directory(Some(custom.clone())).unwrap();
    assert_eq!(saved.backup_directory.as_deref(), Some(custom.as_path()));
    assert_eq!(
        store
            .set_backup_directory(Some(PathBuf::from("relative-backups")))
            .unwrap_err(),
        "invalid_backup_directory"
    );
    assert_eq!(
        store
            .set_backup_directory(Some(PathBuf::from(r"C:\")))
            .unwrap_err(),
        "backup_directory_cannot_be_root"
    );
    assert_eq!(
        store.load().unwrap().backup_directory.as_deref(),
        Some(custom.as_path())
    );
    assert_eq!(
        store.set_backup_directory(None).unwrap().backup_directory,
        None
    );
}

#[test]
fn automatic_backup_state_records_attempt_failure_and_success_atomically() {
    let root = temp_root("automatic-backup-state");
    let store = DeviceSettingsStore::open(root.clone()).unwrap();

    let attempted = store
        .record_automatic_backup_attempt("2026-08-15T03:00:00.000Z")
        .unwrap();
    assert_eq!(
        attempted.automatic_backup.last_attempt_at.as_deref(),
        Some("2026-08-15T03:00:00.000Z")
    );
    assert_eq!(attempted.automatic_backup.last_success_at, None);
    assert_eq!(attempted.automatic_backup.last_failure_at, None);

    let failed = store
        .record_automatic_backup_failure("2026-08-15T03:01:00.000Z", "backup_directory_unavailable")
        .unwrap();
    assert_eq!(
        failed.automatic_backup.last_attempt_at.as_deref(),
        Some("2026-08-15T03:01:00.000Z")
    );
    assert_eq!(
        failed.automatic_backup.last_failure_at.as_deref(),
        Some("2026-08-15T03:01:00.000Z")
    );
    assert_eq!(
        failed.automatic_backup.last_error_code.as_deref(),
        Some("backup_directory_unavailable")
    );

    let succeeded = store
        .record_automatic_backup_success("2026-08-15T04:00:00.000Z")
        .unwrap();
    assert_eq!(
        succeeded.automatic_backup.last_attempt_at.as_deref(),
        Some("2026-08-15T04:00:00.000Z")
    );
    assert_eq!(
        succeeded.automatic_backup.last_success_at.as_deref(),
        Some("2026-08-15T04:00:00.000Z")
    );
    assert_eq!(succeeded.automatic_backup.last_failure_at, None);
    assert_eq!(succeeded.automatic_backup.last_error_code, None);

    let persisted_before_invalid = fs::read(root.join("device-settings.json")).unwrap();
    assert_eq!(
        store
            .record_automatic_backup_attempt("not-a-timestamp")
            .unwrap_err(),
        "invalid_device_settings"
    );
    assert_eq!(
        store
            .record_automatic_backup_failure(
                "2026-08-15T05:00:00.000Z",
                "backup failure with details",
            )
            .unwrap_err(),
        "invalid_device_settings"
    );
    assert_eq!(
        fs::read(root.join("device-settings.json")).unwrap(),
        persisted_before_invalid
    );
}

#[test]
fn onboarding_steps_device_accounts_and_cross_field_rules_are_strict() {
    let root = temp_root("validation");
    let store = DeviceSettingsStore::open(root.clone()).unwrap();

    for step in 0..=3 {
        let mut value = DeviceSettingsV1::default();
        value.onboarding_step = step;
        write_settings(&root, &value);
        assert_eq!(store.load().unwrap(), value);
    }

    let mut completed = DeviceSettingsV1::default();
    completed.onboarding_completed = true;
    completed.onboarding_step = 3;
    write_settings(&root, &completed);
    assert_eq!(store.load().unwrap(), completed);

    for invalid in [
        DeviceSettingsV1 {
            onboarding_step: 4,
            ..DeviceSettingsV1::default()
        },
        DeviceSettingsV1 {
            onboarding_completed: true,
            onboarding_step: 2,
            ..DeviceSettingsV1::default()
        },
        DeviceSettingsV1 {
            launch_at_startup: false,
            silent_autostart: true,
            ..DeviceSettingsV1::default()
        },
    ] {
        let original = write_settings(&root, &invalid);
        assert_eq!(store.load().unwrap_err(), "invalid_device_settings");
        assert_eq!(
            fs::read(root.join("device-settings.json")).unwrap(),
            original
        );
    }

    let mut value = DeviceSettingsV1::default();
    value.accounts.insert(
        "0123456789abcdef0123456789abcdef".to_string(),
        DeviceAccountSettingsV1 {
            browser_preference: BrowserPreferenceV1::Chrome,
        },
    );
    write_settings(&root, &value);
    assert_eq!(store.load().unwrap(), value);

    value.accounts = BTreeMap::from([(
        "0123456789ABCDEF0123456789ABCDEF".to_string(),
        DeviceAccountSettingsV1::default(),
    )]);
    write_settings(&root, &value);
    assert_eq!(store.load().unwrap_err(), "invalid_device_settings");
}

#[test]
fn compare_and_swap_is_atomic_detects_conflicts_and_preserves_overflow_files() {
    let root = temp_root("cas");
    let store = Arc::new(DeviceSettingsStore::open(root.clone()).unwrap());
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();

    for step in [1_u8, 2_u8] {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            store.compare_and_swap(0, |settings| {
                settings.onboarding_step = step;
                Ok(())
            })
        }));
    }
    barrier.wait();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["device_settings_revision_conflict"]
    );
    assert_eq!(store.load().unwrap().revision, 1);

    let before_conflict = fs::read(root.join("device-settings.json")).unwrap();
    assert_eq!(
        store.compare_and_swap(0, |_| Ok(())).unwrap_err(),
        "device_settings_revision_conflict"
    );
    assert_eq!(
        fs::read(root.join("device-settings.json")).unwrap(),
        before_conflict
    );

    let overflow = DeviceSettingsV1 {
        revision: u64::MAX,
        ..DeviceSettingsV1::default()
    };
    let overflow_bytes = write_settings(&root, &overflow);
    assert_eq!(
        store.compare_and_swap(u64::MAX, |_| Ok(())).unwrap_err(),
        "device_settings_revision_overflow"
    );
    assert_eq!(
        fs::read(root.join("device-settings.json")).unwrap(),
        overflow_bytes
    );
}

#[test]
fn onboarding_compatibility_distinguishes_new_installs_and_prior_use() {
    let portable = PortableSettingsV2::default();
    let connection = empty_database();
    let fresh = initialize_onboarding_state(&connection, &portable).unwrap();
    assert!(!fresh.onboarding_completed);
    assert_eq!(fresh.onboarding_step, 0);

    let mut used_portable = portable.clone();
    used_portable.revision = 1;
    let used = initialize_onboarding_state(&connection, &used_portable).unwrap();
    assert!(used.onboarding_completed);
    assert_eq!(used.onboarding_step, 3);

    let connection = empty_database();
    connection
        .execute(
            "INSERT INTO accounts
             (uid, profile_key, status, created_at, updated_at)
             VALUES ('uid-a', '0123456789abcdef0123456789abcdef', 'active', ?1, ?1)",
            ["2026-08-11T00:00:00.000Z"],
        )
        .unwrap();
    assert!(
        initialize_onboarding_state(&connection, &portable)
            .unwrap()
            .onboarding_completed
    );

    let connection = empty_database();
    connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    connection
        .execute(
            "INSERT INTO watch_records
             (account_id, video_id, history_time_ms, watched_at, watched_date,
              source_position, created_at, updated_at)
             VALUES (99, 99, 1786406400000, ?1, '2026-08-11', 0, ?1, ?1)",
            ["2026-08-11T00:00:00.000Z"],
        )
        .unwrap();
    assert!(
        initialize_onboarding_state(&connection, &portable)
            .unwrap()
            .onboarding_completed
    );

    let connection = empty_database();
    connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    connection
        .execute(
            "INSERT INTO sync_jobs
             (account_id, source, started_at, finished_at, status)
             VALUES (99, 'history', ?1, ?1, 'success')",
            params!["2026-08-11T00:00:00.000Z"],
        )
        .unwrap();
    assert!(
        initialize_onboarding_state(&connection, &portable)
            .unwrap()
            .onboarding_completed
    );
}

#[test]
fn startup_initialization_writes_only_missing_device_settings() {
    let root = temp_root("startup");
    let connection = empty_database();
    let portable = PortableSettingsV2::default();

    let store = initialize_device_settings_store(root.clone(), &connection, &portable).unwrap();
    let initial = store.load().unwrap();
    assert_eq!(initial.revision, 1);
    assert!(!initial.onboarding_completed);

    let customized = store
        .compare_and_swap(initial.revision, |settings| {
            settings.onboarding_step = 2;
            settings.launch_at_startup = true;
            Ok(())
        })
        .unwrap();

    let mut used_portable = portable;
    used_portable.revision = 10;
    let reopened =
        initialize_device_settings_store(root.clone(), &connection, &used_portable).unwrap();
    assert_eq!(reopened.load().unwrap(), customized);
}

#[test]
fn notification_failure_keys_hash_source_instances_without_persisting_remote_ids() {
    let empty = notification_failure_key(42, "history", "").unwrap();
    assert_eq!(
        empty,
        "42:history:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );

    let remote_id = "remote-folder-sensitive-id";
    let key = notification_failure_key(42, "remote_folder", remote_id).unwrap();
    assert!(key.starts_with("42:remote_folder:"));
    assert!(!key.contains(remote_id));
    assert_eq!(key.rsplit(':').next().unwrap().len(), 64);
    assert_eq!(
        notification_failure_key(0, "history", "").unwrap_err(),
        "invalid_notification_key"
    );
    assert_eq!(
        notification_failure_key(42, "unknown", "").unwrap_err(),
        "invalid_notification_key"
    );
}

#[test]
fn login_preference_is_added_only_after_successful_account_claim() {
    let root = temp_root("login-preference");
    let store = DeviceSettingsStore::open(root).unwrap();
    let profile_key = "0123456789abcdef0123456789abcdef";

    let saved = store
        .set_account_browser_preference(profile_key, BrowserPreferenceV1::Chrome)
        .unwrap();
    assert_eq!(
        saved.accounts.get(profile_key),
        Some(&DeviceAccountSettingsV1 {
            browser_preference: BrowserPreferenceV1::Chrome,
        })
    );

    assert_eq!(
        store
            .set_account_browser_preference("not-a-profile-key", BrowserPreferenceV1::Edge)
            .unwrap_err(),
        "invalid_profile_key"
    );
    assert_eq!(
        store.load().unwrap().accounts.len(),
        1,
        "a failed preference write must not create an orphan entry"
    );
}
