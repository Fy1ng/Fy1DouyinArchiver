use douyin_history_archive::{
    desktop_lifecycle::BackgroundTaskRegistry,
    migrate_schema_to_latest_at_startup,
    runtime_paths::RuntimePaths,
    sync::{BrowserOperationKind, OperationAdmission},
    AppPaths,
};
use rusqlite::Connection;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn runtime_backup_uses_maintenance_and_releases_every_registration() {
    let root = temp_root("success");
    let paths = prepare_paths(&root);
    let runtime = RuntimePaths::new(paths.clone());
    let admission = OperationAdmission::new();
    let background = BackgroundTaskRegistry::default();
    let destination = root.join("export.emily-backup");

    let summary = douyin_history_archive::run_backup_with_runtime(
        &runtime,
        &admission,
        &background,
        &destination,
        "2026-08-14T03:00:00.000+08:00",
        "0.1.0",
        Duration::from_secs(1),
    )
    .unwrap();

    assert!(destination.is_file());
    assert!(summary.byte_size > 0);
    assert_eq!(background.active_count().unwrap(), 0);
    assert_eq!(admission.snapshot().unwrap().owned_operation_count, 0);
    assert_eq!(runtime.acquire().unwrap().database, paths.database);
}

#[test]
fn runtime_backup_times_out_while_a_browser_operation_is_active_without_writing() {
    let root = temp_root("busy");
    let paths = prepare_paths(&root);
    let runtime = RuntimePaths::new(paths);
    let admission = OperationAdmission::new();
    let background = BackgroundTaskRegistry::default();
    let browser = admission
        .begin_browser_operation(BrowserOperationKind::Sync, Some(7))
        .unwrap();
    let destination = root.join("blocked.emily-backup");

    assert_eq!(
        douyin_history_archive::run_backup_with_runtime(
            &runtime,
            &admission,
            &background,
            &destination,
            "2026-08-14T03:00:00.000+08:00",
            "0.1.0",
            Duration::ZERO,
        )
        .unwrap_err(),
        "operation_timeout"
    );
    assert!(!destination.exists());
    assert_eq!(background.active_count().unwrap(), 0);
    drop(browser);
    assert!(runtime.acquire().is_ok());
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

fn temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-backup-runtime-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
