use chrono::{DateTime, Utc};
use douyin_history_archive::{
    desktop_lifecycle::BackgroundTaskRegistry,
    device_settings::{AutomaticBackupStateV1, DeviceSettingsStore},
    migrate_schema_to_latest_at_startup,
    notifications::NotificationKindV1,
    run_automatic_backup_tick,
    runtime_paths::RuntimePaths,
    settings_store::{BackupSettingsV1, NotificationSettingsV1},
    sync::{BrowserOperationKind, OperationAdmission},
    AppPaths, AutomaticBackupTickStatusV1,
};
use rusqlite::Connection;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn due_automatic_backup_uses_default_directory_prunes_and_notifies() {
    let root = temp_root("success");
    let paths = prepare_paths(&root);
    fs::create_dir_all(&paths.backups).unwrap();
    fs::write(
        paths
            .backups
            .join("emily-auto-20260812-030000-000.emily-backup"),
        b"oldest",
    )
    .unwrap();
    fs::write(
        paths
            .backups
            .join("emily-auto-20260813-030000-000.emily-backup"),
        b"newer",
    )
    .unwrap();
    let runtime = RuntimePaths::new(paths.clone());
    let admission = OperationAdmission::new();
    let background = BackgroundTaskRegistry::default();
    let device = DeviceSettingsStore::open(root.join("system")).unwrap();
    let mut notifications = Vec::new();

    let result = run_automatic_backup_tick(
        &runtime,
        &admission,
        &background,
        &device,
        &enabled_backup_settings(2),
        &NotificationSettingsV1 {
            backup_completed: true,
            ..NotificationSettingsV1::default()
        },
        utc("2026-08-14T19:01:00.000Z"),
        "0.1.0",
        Duration::ZERO,
        |kind| {
            notifications.push(kind);
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(result.status, AutomaticBackupTickStatusV1::Succeeded);
    assert_eq!(result.removed_backups, 1);
    assert_eq!(notifications, vec![NotificationKindV1::BackupCompleted]);
    assert_eq!(
        fs::read_dir(&paths.backups)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().unwrap().is_file())
            .count(),
        2
    );
    assert!(!paths
        .backups
        .join("emily-auto-20260812-030000-000.emily-backup")
        .exists());
    assert!(result
        .destination
        .as_ref()
        .is_some_and(|path| path.is_file()));
    let persisted = device.load().unwrap().automatic_backup;
    assert_eq!(
        persisted.last_success_at.as_deref(),
        Some("2026-08-14T19:01:00.000Z")
    );
    assert_eq!(persisted.last_failure_at, None);
    assert_eq!(background.active_count().unwrap(), 0);
    assert_eq!(admission.snapshot().unwrap().owned_operation_count, 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unavailable_custom_directory_fails_without_default_fallback() {
    let root = temp_root("custom-unavailable");
    let paths = prepare_paths(&root);
    let runtime = RuntimePaths::new(paths.clone());
    let admission = OperationAdmission::new();
    let background = BackgroundTaskRegistry::default();
    let device = DeviceSettingsStore::open(root.join("system")).unwrap();
    let custom = root.join("custom-backups");
    fs::write(&custom, b"not-a-directory").unwrap();
    device.set_backup_directory(Some(custom)).unwrap();
    let mut notifications = Vec::new();

    let result = run_automatic_backup_tick(
        &runtime,
        &admission,
        &background,
        &device,
        &enabled_backup_settings(5),
        &NotificationSettingsV1 {
            backup_failure: false,
            ..NotificationSettingsV1::default()
        },
        utc("2026-08-14T19:01:00.000Z"),
        "0.1.0",
        Duration::ZERO,
        |kind| {
            notifications.push(kind);
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(result.status, AutomaticBackupTickStatusV1::Failed);
    assert_eq!(
        result.error_code.as_deref(),
        Some("backup_directory_unavailable")
    );
    assert_eq!(notifications, vec![NotificationKindV1::BackupFailed]);
    assert!(!paths.backups.exists());
    let persisted = device.load().unwrap().automatic_backup;
    assert_eq!(
        persisted.last_failure_at.as_deref(),
        Some("2026-08-14T19:01:00.000Z")
    );
    assert_eq!(
        persisted.last_error_code.as_deref(),
        Some("backup_directory_unavailable")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn busy_runtime_defers_without_failure_state_or_notification() {
    let root = temp_root("busy");
    let paths = prepare_paths(&root);
    let runtime = RuntimePaths::new(paths.clone());
    let admission = OperationAdmission::new();
    let background = BackgroundTaskRegistry::default();
    let device = DeviceSettingsStore::open(root.join("system")).unwrap();
    let browser = admission
        .begin_browser_operation(BrowserOperationKind::Sync, Some(7))
        .unwrap();
    let mut notifications = Vec::new();

    let result = run_automatic_backup_tick(
        &runtime,
        &admission,
        &background,
        &device,
        &enabled_backup_settings(5),
        &NotificationSettingsV1::default(),
        utc("2026-08-14T19:01:00.000Z"),
        "0.1.0",
        Duration::ZERO,
        |kind| {
            notifications.push(kind);
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(result.status, AutomaticBackupTickStatusV1::Deferred);
    assert_eq!(result.next_wake_at, Some(utc("2026-08-14T19:01:30.000Z")));
    assert_eq!(
        device.load().unwrap().automatic_backup,
        AutomaticBackupStateV1::default()
    );
    assert!(notifications.is_empty());
    assert_eq!(background.active_count().unwrap(), 0);
    assert!(!paths.backups.exists());
    drop(browser);
    fs::remove_dir_all(root).unwrap();
}

fn enabled_backup_settings(retention_count: u32) -> BackupSettingsV1 {
    BackupSettingsV1 {
        enabled: true,
        interval_days: 1,
        execution_time: "03:00".to_string(),
        retention_count,
    }
}

fn prepare_paths(root: &PathBuf) -> AppPaths {
    let data_root = root.join("data");
    fs::create_dir_all(&data_root).unwrap();
    let paths = AppPaths::from_roots(root.join("bootstrap"), data_root, root.join("resources"));
    let mut connection = Connection::open(&paths.database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    drop(connection);
    paths
}

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-automatic-backup-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
