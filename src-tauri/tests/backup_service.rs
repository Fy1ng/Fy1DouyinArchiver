use chrono::{DateTime, Utc};
use douyin_history_archive::{
    automatic_backup_destination, create_application_backup,
    device_settings::AutomaticBackupStateV1,
    extract_backup_archive, manual_backup_file_name, migrate_schema_to_latest_at_startup,
    next_automatic_backup_at, next_automatic_backup_attempt_at, probe_backup_directory,
    prune_automatic_backups,
    settings_store::{BackupSettingsV1, PortableSettingsV2},
};
use rusqlite::Connection;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn application_backup_round_trips_current_portable_settings() {
    let root = temp_root("application");
    let database = root.join("history.sqlite");
    let mut connection = Connection::open(&database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    drop(connection);

    let mut settings = PortableSettingsV2::default();
    settings.revision = 12;
    settings.global.backup.enabled = true;
    settings.global.backup.interval_days = 3;
    let destination = root.join("manual.emily-backup");

    create_application_backup(
        &database,
        &settings,
        &destination,
        "2026-08-14T03:00:00.000+08:00",
        "0.1.0",
    )
    .unwrap();
    let extracted = extract_backup_archive(&destination, &root.join("extracted")).unwrap();
    let restored: PortableSettingsV2 = serde_json::from_slice(&extracted.settings_bytes).unwrap();

    assert_eq!(restored, settings);
    assert_eq!(extracted.manifest.settings_schema_version, 7);
}

#[test]
fn backup_directory_probe_leaves_no_files_and_rejects_roots() {
    let root = temp_root("probe");
    let directory = root.join("backups");

    probe_backup_directory(&directory).unwrap();

    assert!(directory.is_dir());
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
    assert_eq!(
        probe_backup_directory(PathBuf::from(r"C:\").as_path()).unwrap_err(),
        "backup_directory_cannot_be_root"
    );
}

#[test]
fn automatic_backup_pruning_only_removes_matching_regular_files() {
    let root = temp_root("retention");
    for name in [
        "emily-auto-20260810-030000-000.emily-backup",
        "emily-auto-20260811-030000-000.emily-backup",
        "emily-auto-20260812-030000-000.emily-backup",
        "manual.emily-backup",
        "emily-auto-invalid.emily-backup",
    ] {
        fs::write(root.join(name), name).unwrap();
    }
    fs::create_dir(root.join("emily-auto-20260809-030000-000.emily-backup")).unwrap();

    let removed = prune_automatic_backups(&root, 2).unwrap();

    assert_eq!(removed, 1);
    assert!(!root
        .join("emily-auto-20260810-030000-000.emily-backup")
        .exists());
    assert!(root
        .join("emily-auto-20260811-030000-000.emily-backup")
        .is_file());
    assert!(root
        .join("emily-auto-20260812-030000-000.emily-backup")
        .is_file());
    assert!(root.join("manual.emily-backup").is_file());
    assert!(root.join("emily-auto-invalid.emily-backup").is_file());
    assert!(root
        .join("emily-auto-20260809-030000-000.emily-backup")
        .is_dir());
}

#[test]
fn automatic_backup_names_and_schedule_use_shanghai_calendar_time() {
    let instant = utc("2026-08-13T19:00:00.123Z");
    assert_eq!(
        automatic_backup_destination(PathBuf::from(r"C:\Backups").as_path(), instant),
        PathBuf::from(r"C:\Backups\emily-auto-20260814-030000-123.emily-backup")
    );
    assert_eq!(
        manual_backup_file_name(instant),
        "emily-backup-20260814-030000-123.emily-backup"
    );

    let settings = BackupSettingsV1 {
        enabled: true,
        interval_days: 3,
        execution_time: "03:00".to_string(),
        retention_count: 5,
    };
    let due = next_automatic_backup_at(
        Some("2026-08-10T19:01:00.000Z"),
        &settings,
        utc("2026-08-13T18:00:00.000Z"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(due, utc("2026-08-13T19:00:00.000Z"));
    assert_eq!(
        next_automatic_backup_at(None, &BackupSettingsV1::default(), Utc::now()).unwrap(),
        None
    );
}

#[test]
fn failed_or_interrupted_automatic_backup_retries_after_one_hour() {
    let settings = BackupSettingsV1 {
        enabled: true,
        interval_days: 3,
        execution_time: "03:00".to_string(),
        retention_count: 5,
    };
    let failed = AutomaticBackupStateV1 {
        last_attempt_at: Some("2026-08-13T19:01:00.000Z".to_string()),
        last_success_at: Some("2026-08-10T19:01:00.000Z".to_string()),
        last_failure_at: Some("2026-08-13T19:01:00.000Z".to_string()),
        last_error_code: Some("backup_directory_unavailable".to_string()),
    };
    assert_eq!(
        next_automatic_backup_attempt_at(&failed, &settings, utc("2026-08-13T19:10:00.000Z"),)
            .unwrap(),
        Some(utc("2026-08-13T20:01:00.000Z"))
    );

    let interrupted = AutomaticBackupStateV1 {
        last_attempt_at: Some("2026-08-13T21:15:00.000Z".to_string()),
        last_success_at: failed.last_success_at.clone(),
        last_failure_at: None,
        last_error_code: None,
    };
    assert_eq!(
        next_automatic_backup_attempt_at(&interrupted, &settings, utc("2026-08-13T21:20:00.000Z"),)
            .unwrap(),
        Some(utc("2026-08-13T22:15:00.000Z"))
    );

    let succeeded = AutomaticBackupStateV1 {
        last_attempt_at: Some("2026-08-13T19:01:00.000Z".to_string()),
        last_success_at: Some("2026-08-13T19:01:00.000Z".to_string()),
        last_failure_at: None,
        last_error_code: None,
    };
    assert_eq!(
        next_automatic_backup_attempt_at(&succeeded, &settings, utc("2026-08-13T20:00:00.000Z"),)
            .unwrap(),
        Some(utc("2026-08-16T19:00:00.000Z"))
    );
    assert_eq!(
        next_automatic_backup_attempt_at(
            &AutomaticBackupStateV1::default(),
            &BackupSettingsV1::default(),
            utc("2026-08-13T20:00:00.000Z"),
        )
        .unwrap(),
        None
    );
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
        "emily-backup-service-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
