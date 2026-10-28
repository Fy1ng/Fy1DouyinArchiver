use chrono::{DateTime, Utc};
use douyin_history_archive::{
    bootstrap::{read_bootstrap, write_bootstrap_atomic, BootstrapV1},
    create_application_backup,
    desktop_lifecycle::BackgroundTaskRegistry,
    extract_backup_archive, inspect_backup_for_restore, migrate_schema_to_latest_at_startup,
    restore_backup_replace,
    runtime_paths::RuntimePaths,
    settings_store::{
        load_portable_settings, write_portable_settings_atomic, LikeCountFormatV1,
        PortableSettingsV2,
    },
    sync::OperationAdmission,
    AppPaths, InspectRestoreRequestV1, ReplaceRestoreRequestV1,
};
use rusqlite::{params, Connection};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn replace_restore_recovers_archives_remaps_active_account_and_creates_safety_backup() {
    let root = temp_root("replace-success");
    let source = prepare_data_root(&root.join("source"), 'a');
    let target = prepare_data_root(&root.join("target"), 'b');
    let archive = root.join("source.emily-backup");
    let source_account_id = seed_source_archive(&source, &archive);
    let target_account_id = seed_target_state(&target);
    assert_ne!(source_account_id, target_account_id);
    let runtime = RuntimePaths::new(target.clone());
    let admission = OperationAdmission::new();
    let background = BackgroundTaskRegistry::default();

    let restored = restore_backup_replace(ReplaceRestoreRequestV1 {
        runtime_paths: &runtime,
        admission: &admission,
        background_tasks: &background,
        archive_path: &archive,
        now: utc("2026-08-15T08:00:00.000Z"),
        application_version: "0.1.0",
        wait_timeout: Duration::ZERO,
    })
    .unwrap();

    assert!(restored.safety_backup_path.is_file());
    assert_eq!(restored.active_account_id, Some(source_account_id));
    let connection = Connection::open(&target.database).unwrap();
    assert_eq!(count(&connection, "accounts"), 2);
    assert_eq!(count(&connection, "watch_records"), 1);
    assert_eq!(count(&connection, "local_folders"), 1);
    assert_eq!(count(&connection, "cover_cache"), 0);
    assert_eq!(
        connection
            .query_row(
                "SELECT status FROM accounts WHERE id = ?1",
                [source_account_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "login_expired"
    );
    assert_eq!(
        read_bootstrap(&target.bootstrap_dir)
            .unwrap()
            .unwrap()
            .active_account_id,
        Some(source_account_id)
    );
    let settings = load_portable_settings(&target.data_root).unwrap();
    assert_eq!(settings.revision, 3);
    assert_eq!(settings.global.like_count_format, LikeCountFormatV1::Exact);

    let safety =
        extract_backup_archive(&restored.safety_backup_path, &root.join("safety-extracted"))
            .unwrap();
    let safety_connection = Connection::open(safety.database_path).unwrap();
    assert_eq!(count(&safety_connection, "accounts"), 1);
    assert_eq!(count(&safety_connection, "watch_records"), 0);
    assert_eq!(background.active_count().unwrap(), 0);
    assert_eq!(admission.snapshot().unwrap().owned_operation_count, 0);
    drop(safety_connection);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_replace_archive_leaves_database_settings_and_bootstrap_unchanged() {
    let root = temp_root("replace-invalid");
    let source = prepare_data_root(&root.join("source"), 'c');
    let target = prepare_data_root(&root.join("target"), 'd');
    let archive = root.join("tampered.emily-backup");
    seed_source_archive(&source, &archive);
    seed_target_state(&target);
    let mut bytes = fs::read(&archive).unwrap();
    *bytes.last_mut().unwrap() ^= 0x01;
    fs::write(&archive, bytes).unwrap();
    let before_database = fs::read(&target.database).unwrap();
    let before_settings = fs::read(target.data_root.join("settings.json")).unwrap();
    let before_bootstrap = fs::read(target.bootstrap_dir.join("bootstrap.json")).unwrap();
    let runtime = RuntimePaths::new(target.clone());
    let admission = OperationAdmission::new();
    let background = BackgroundTaskRegistry::default();

    assert_eq!(
        restore_backup_replace(ReplaceRestoreRequestV1 {
            runtime_paths: &runtime,
            admission: &admission,
            background_tasks: &background,
            archive_path: &archive,
            now: utc("2026-08-15T08:00:00.000Z"),
            application_version: "0.1.0",
            wait_timeout: Duration::ZERO,
        })
        .unwrap_err(),
        "backup_payload_hash_mismatch"
    );

    assert_eq!(fs::read(&target.database).unwrap(), before_database);
    assert_eq!(
        fs::read(target.data_root.join("settings.json")).unwrap(),
        before_settings
    );
    assert_eq!(
        fs::read(target.bootstrap_dir.join("bootstrap.json")).unwrap(),
        before_bootstrap
    );
    assert!(!target.backups.exists());
    assert_eq!(background.active_count().unwrap(), 0);
    assert_eq!(admission.snapshot().unwrap().owned_operation_count, 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_inspection_validates_archive_without_mutating_current_state() {
    let root = temp_root("inspect-valid");
    let source = prepare_data_root(&root.join("source"), 'e');
    let target = prepare_data_root(&root.join("target"), 'f');
    let archive = root.join("source.emily-backup");
    seed_source_archive(&source, &archive);
    seed_target_state(&target);
    let before_database = fs::read(&target.database).unwrap();
    let before_settings = fs::read(target.data_root.join("settings.json")).unwrap();
    let before_bootstrap = fs::read(target.bootstrap_dir.join("bootstrap.json")).unwrap();
    let runtime = RuntimePaths::new(target.clone());
    let background = BackgroundTaskRegistry::default();

    let summary = inspect_backup_for_restore(InspectRestoreRequestV1 {
        runtime_paths: &runtime,
        background_tasks: &background,
        archive_path: &archive,
    })
    .unwrap();

    assert_eq!(summary.created_at, "2026-08-15T07:00:00.000Z");
    assert_eq!(summary.application_version, "0.1.0");
    assert_eq!(summary.database_schema_version, 8);
    assert_eq!(summary.settings_schema_version, 7);
    assert!(summary.byte_size > 0);
    assert_eq!(fs::read(&target.database).unwrap(), before_database);
    assert_eq!(
        fs::read(target.data_root.join("settings.json")).unwrap(),
        before_settings
    );
    assert_eq!(
        fs::read(target.bootstrap_dir.join("bootstrap.json")).unwrap(),
        before_bootstrap
    );
    assert!(!target.backups.exists());
    assert_eq!(background.active_count().unwrap(), 0);
    if target.temp.exists() {
        assert_eq!(fs::read_dir(&target.temp).unwrap().count(), 0);
    }
    fs::remove_dir_all(root).unwrap();
}

fn seed_source_archive(paths: &AppPaths, archive: &Path) -> i64 {
    let connection = Connection::open(&paths.database).unwrap();
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, nickname, status, created_at, updated_at)
             VALUES ('source-other', 'source-other-sec', ?1, '其他账号', 'archived', ?2, ?2)",
            params![
                "11111111111111111111111111111111",
                "2026-08-10T00:00:00.000Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision)
             VALUES (?1, 1)",
            [connection.last_insert_rowid()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, nickname, status, created_at, updated_at)
             VALUES ('shared-user', 'shared-sec', ?1, '来源账号', 'active', ?2, ?2)",
            params![
                "22222222222222222222222222222222",
                "2026-08-11T00:00:00.000Z"
            ],
        )
        .unwrap();
    let account_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision)
             VALUES (?1, 1)",
            [account_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO videos
             (aweme_id, canonical_url, content_type, title, cover_url, first_seen_at, last_seen_at)
             VALUES ('source-video', 'https://www.douyin.com/video/source-video', 'video',
                     '来源视频', 'https://example.com/source.jpg', ?1, ?1)",
            ["2026-08-11T00:00:00.000Z"],
        )
        .unwrap();
    let video_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO watch_records
             (account_id, video_id, history_time_ms, watched_at, watched_date,
              source_position, created_at, updated_at)
             VALUES (?1, ?2, 1786410000000, ?3, '2026-08-11', 1, ?3, ?3)",
            params![account_id, video_id, "2026-08-11T01:00:00.000Z"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO local_folders(scope, owner_account_id, name, created_at, updated_at)
             VALUES ('account', ?1, '稍后再看', ?2, ?2)",
            params![account_id, "2026-08-11T01:00:00.000Z"],
        )
        .unwrap();
    let folder_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO local_folder_items(folder_id, video_id, added_at)
             VALUES (?1, ?2, ?3)",
            params![folder_id, video_id, "2026-08-11T01:00:00.000Z"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cover_cache
             (video_id, remote_url, relative_path, byte_size, mime_type, sha256,
              downloaded_at, last_accessed_at)
             VALUES (?1, 'https://example.com/source.jpg', 'aa/source.jpg', 10,
                     'image/jpeg', ?2, ?3, ?3)",
            params![video_id, "a".repeat(64), "2026-08-11T01:00:00.000Z"],
        )
        .unwrap();
    drop(connection);
    let mut settings = PortableSettingsV2::default();
    settings.revision = 3;
    settings.global.like_count_format = LikeCountFormatV1::Exact;
    write_portable_settings_atomic(&paths.data_root, &settings).unwrap();
    create_application_backup(
        &paths.database,
        &settings,
        archive,
        "2026-08-15T07:00:00.000Z",
        "0.1.0",
    )
    .unwrap();
    account_id
}

fn seed_target_state(paths: &AppPaths) -> i64 {
    let connection = Connection::open(&paths.database).unwrap();
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, nickname, status, created_at, updated_at)
             VALUES ('shared-user', 'shared-sec', ?1, '当前设备账号', 'active', ?2, ?2)",
            params![
                "33333333333333333333333333333333",
                "2026-08-12T00:00:00.000Z"
            ],
        )
        .unwrap();
    let account_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision)
             VALUES (?1, 1)",
            [account_id],
        )
        .unwrap();
    drop(connection);
    let mut bootstrap = read_bootstrap(&paths.bootstrap_dir).unwrap().unwrap();
    bootstrap.active_account_id = Some(account_id);
    write_bootstrap_atomic(&paths.bootstrap_dir, &bootstrap).unwrap();
    fs::create_dir_all(
        paths
            .account_root("33333333333333333333333333333333")
            .unwrap(),
    )
    .unwrap();
    let mut settings = PortableSettingsV2::default();
    settings.revision = 11;
    write_portable_settings_atomic(&paths.data_root, &settings).unwrap();
    account_id
}

fn prepare_data_root(root: &Path, root_character: char) -> AppPaths {
    let paths = AppPaths::from_roots(
        root.join("system"),
        root.join("data"),
        root.join("resources"),
    );
    fs::create_dir_all(&paths.data_root).unwrap();
    fs::create_dir_all(&paths.bootstrap_dir).unwrap();
    let mut connection = Connection::open(&paths.database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    drop(connection);
    let bootstrap = BootstrapV1::new(
        paths.data_root.clone(),
        std::iter::repeat_n(root_character, 32).collect(),
    )
    .unwrap();
    write_bootstrap_atomic(&paths.bootstrap_dir, &bootstrap).unwrap();
    write_portable_settings_atomic(&paths.data_root, &PortableSettingsV2::default()).unwrap();
    paths
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
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
        "emily-restore-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
