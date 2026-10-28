use douyin_history_archive::{
    create_backup_archive, extract_backup_archive, migrate_schema_to_latest_at_startup,
    BackupArchiveCreateRequestV1,
};
use rusqlite::Connection;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn backup_archive_round_trips_only_database_and_portable_settings() {
    let root = temp_root("round-trip");
    let database = root.join("history.sqlite");
    let mut connection = Connection::open(&database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    connection
        .execute(
            "INSERT INTO accounts(uid, profile_key, status, created_at, updated_at)
             VALUES ('uid-backup', '0123456789abcdef0123456789abcdef', 'active', ?1, ?1)",
            ["2026-08-14T00:00:00.000Z"],
        )
        .unwrap();
    let account_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)",
            [account_id],
        )
        .unwrap();
    drop(connection);

    let destination = root.join("manual.emily-backup");
    let settings = br#"{"schemaVersion":4,"revision":7}"#;
    let summary = create_backup_archive(BackupArchiveCreateRequestV1 {
        database_path: &database,
        settings_bytes: settings,
        settings_schema_version: 4,
        destination: &destination,
        created_at: "2026-08-14T00:01:00.000Z",
        application_version: "0.1.0",
    })
    .unwrap();

    assert_eq!(summary.created_at, "2026-08-14T00:01:00.000Z");
    assert_eq!(summary.database_schema_version, 8);
    assert_eq!(summary.settings_schema_version, 4);
    assert!(summary.byte_size > settings.len() as u64);

    let extracted_root = root.join("extracted");
    let extracted = extract_backup_archive(&destination, &extracted_root).unwrap();
    assert_eq!(extracted.settings_bytes, settings);
    assert_eq!(extracted.summary, summary);
    let extracted_database = Connection::open(extracted.database_path).unwrap();
    let uid: String = extracted_database
        .query_row("SELECT uid FROM accounts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(uid, "uid-backup");
    assert_eq!(extracted.manifest.payloads.len(), 2);
    assert_eq!(extracted.manifest.payloads[0].name, "settings.json");
    assert_eq!(extracted.manifest.payloads[1].name, "history.sqlite");
}

#[test]
fn backup_archive_rejects_payload_tampering_and_trailing_data() {
    let root = temp_root("tamper");
    let database = root.join("history.sqlite");
    let mut connection = Connection::open(&database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    drop(connection);
    let destination = root.join("tampered.emily-backup");
    create_backup_archive(BackupArchiveCreateRequestV1 {
        database_path: &database,
        settings_bytes: br#"{"schemaVersion":4}"#,
        settings_schema_version: 4,
        destination: &destination,
        created_at: "2026-08-14T00:01:00.000Z",
        application_version: "0.1.0",
    })
    .unwrap();

    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&destination)
        .unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[byte[0] ^ 0xff]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert_eq!(
        extract_backup_archive(&destination, &root.join("tampered-out")).unwrap_err(),
        "backup_payload_hash_mismatch"
    );

    create_backup_archive(BackupArchiveCreateRequestV1 {
        database_path: &database,
        settings_bytes: br#"{"schemaVersion":4}"#,
        settings_schema_version: 4,
        destination: &destination,
        created_at: "2026-08-14T00:02:00.000Z",
        application_version: "0.1.0",
    })
    .unwrap();
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&destination)
        .unwrap();
    file.write_all(b"unexpected").unwrap();
    file.sync_all().unwrap();
    assert_eq!(
        extract_backup_archive(&destination, &root.join("trailing-out")).unwrap_err(),
        "backup_trailing_data"
    );
}

#[test]
fn backup_archive_rejects_unsupported_database_schema() {
    let root = temp_root("schema");
    let database = root.join("history.sqlite");
    let connection = Connection::open(&database).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);

    assert_eq!(
        create_backup_archive(BackupArchiveCreateRequestV1 {
            database_path: &database,
            settings_bytes: br#"{"schemaVersion":4}"#,
            settings_schema_version: 4,
            destination: &root.join("invalid.emily-backup"),
            created_at: "2026-08-14T00:01:00.000Z",
            application_version: "0.1.0",
        })
        .unwrap_err(),
        "backup_database_schema_unsupported"
    );
}

fn temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-backup-archive-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
