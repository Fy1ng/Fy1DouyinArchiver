use douyin_history_archive::{
    device_settings::BrowserPreferenceV1,
    migrate_schema_to_v5_at_startup,
    settings_store::ArchiveListRangeV1,
    sync::{
        build_archive_known_ids_snapshot, freeze_task_payload, validate_archive_known_ids_snapshot,
        ArchiveKnownIdsSnapshotV1, ArchiveRunModeV1, BrowserExecutionIdentityV1, BrowserIdV1,
        NewSyncJobV1, QueueStore, RangeFreezeV1, SyncSourceV1, SyncTaskPayloadV1, SyncTriggerV1,
        TaskKeyV1,
    },
    sync_orchestration::SyncPhase,
};
use rusqlite::{params, Connection};

const ATTEMPT_ID: &str = "0123456789abcdef0123456789abcdef";

fn memory_database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    connection
}

fn account(connection: &Connection, suffix: usize) -> i64 {
    connection
        .execute(
            "INSERT INTO accounts(uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'active', 'x', 'x')",
            params![
                format!("uid-{suffix}"),
                format!("sec-{suffix}"),
                format!("{suffix:032x}"),
            ],
        )
        .unwrap();
    let account_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)",
            [account_id],
        )
        .unwrap();
    account_id
}

fn browser(account_id: i64) -> BrowserExecutionIdentityV1 {
    BrowserExecutionIdentityV1 {
        preference: BrowserPreferenceV1::Chrome,
        resolved_browser: BrowserIdV1::Chrome,
        profile_key: format!("{account_id:032x}"),
        profile_generation: "fedcba9876543210fedcba9876543210".to_string(),
        executable_path_sha256: "a".repeat(64),
    }
}

fn running_job(
    connection: &mut Connection,
    account_id: i64,
    payload: SyncTaskPayloadV1,
) -> douyin_history_archive::sync::StoredSyncJobV1 {
    let source = payload.source();
    let source_instance_key = payload.source_instance_key().to_string();
    let frozen = freeze_task_payload(payload).unwrap();
    let queued = QueueStore::enqueue_job(
        connection,
        &NewSyncJobV1 {
            task_key: TaskKeyV1 {
                account_id,
                source,
                source_instance_key,
            },
            trigger: SyncTriggerV1::Manual,
            queued_at: "2026-08-13T00:00:00.000Z".to_string(),
            scheduled_for: None,
            settings_revision: 1,
            config_sha256: None,
            payload_json: frozen.payload_json,
            payload_sha256: frozen.payload_sha256,
            equivalence_sha256: frozen.equivalence_sha256,
        },
    )
    .unwrap();
    QueueStore::claim_job(connection, queued.snapshot.id, "2026-08-13T00:00:01.000Z").unwrap()
}

fn likes_payload(account_id: i64) -> SyncTaskPayloadV1 {
    SyncTaskPayloadV1::Likes {
        range: ArchiveListRangeV1::All,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: 1_786_420_800_000,
            cutoff_ms: None,
        },
        run_mode: ArchiveRunModeV1::Incremental,
        scroll_speed: 5_000,
        hide_browser: true,
        browser: browser(account_id),
    }
}

fn insert_video(connection: &Connection, aweme_id: &str) -> i64 {
    connection
        .execute(
            "INSERT INTO videos
             (aweme_id, canonical_url, content_type, first_seen_at, last_seen_at)
             VALUES (?1, ?2, 'video', 'first', 'last')",
            params![aweme_id, format!("https://www.douyin.com/video/{aweme_id}")],
        )
        .unwrap();
    connection.last_insert_rowid()
}

#[test]
fn known_ids_snapshot_is_source_scoped_strict_and_tamper_evident() {
    let mut connection = memory_database();
    let first_account = account(&connection, 1);
    let second_account = account(&connection, 2);
    let first = insert_video(&connection, "7650000000000000001");
    let removed = insert_video(&connection, "7650000000000000002");
    let other_account = insert_video(&connection, "7650000000000000003");
    connection
        .execute(
            "INSERT INTO user_likes
             (account_id, video_id, first_seen_at, last_seen_at, remote_removed_at)
             VALUES (?1, ?2, 'first', 'last', NULL),
                    (?1, ?3, 'first', 'last', 'removed'),
                    (?4, ?5, 'first', 'last', NULL)",
            params![first_account, first, removed, second_account, other_account],
        )
        .unwrap();
    let job = running_job(&mut connection, first_account, likes_payload(first_account));

    let snapshot =
        build_archive_known_ids_snapshot(&connection, &job, SyncPhase::Collecting, ATTEMPT_ID)
            .expect("snapshot");
    assert_eq!(
        snapshot.aweme_ids,
        vec![
            "7650000000000000001".to_string(),
            "7650000000000000002".to_string(),
        ]
    );
    validate_archive_known_ids_snapshot(
        &snapshot,
        &job.snapshot,
        SyncPhase::Collecting,
        ATTEMPT_ID,
    )
    .expect("valid snapshot");

    let mut tampered = snapshot.clone();
    tampered.aweme_ids.pop();
    assert_eq!(
        validate_archive_known_ids_snapshot(
            &tampered,
            &job.snapshot,
            SyncPhase::Collecting,
            ATTEMPT_ID,
        )
        .unwrap_err(),
        "archive_known_ids_hash_mismatch"
    );

    let mut encoded = serde_json::to_value(snapshot).unwrap();
    encoded
        .as_object_mut()
        .unwrap()
        .insert("unexpected".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<ArchiveKnownIdsSnapshotV1>(encoded).is_err());
}

#[test]
fn remote_folder_known_ids_do_not_cross_folder_boundaries() {
    let mut connection = memory_database();
    let account_id = account(&connection, 3);
    for (folder_id, name) in [("folder-a", "A"), ("folder-b", "B")] {
        connection
            .execute(
                "INSERT INTO remote_folders
                 (account_id, remote_folder_id, name, first_seen_at, last_seen_at)
                 VALUES (?1, ?2, ?3, 'first', 'last')",
                params![account_id, folder_id, name],
            )
            .unwrap();
    }
    let video_a = insert_video(&connection, "7650000000000000010");
    let video_b = insert_video(&connection, "7650000000000000011");
    let folder_a: i64 = connection
        .query_row(
            "SELECT id FROM remote_folders WHERE account_id = ?1 AND remote_folder_id = 'folder-a'",
            [account_id],
            |row| row.get(0),
        )
        .unwrap();
    let folder_b: i64 = connection
        .query_row(
            "SELECT id FROM remote_folders WHERE account_id = ?1 AND remote_folder_id = 'folder-b'",
            [account_id],
            |row| row.get(0),
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO remote_folder_items(folder_id, video_id, first_seen_at, last_seen_at)
             VALUES (?1, ?2, 'first', 'last'), (?3, ?4, 'first', 'last')",
            params![folder_a, video_a, folder_b, video_b],
        )
        .unwrap();
    let job = running_job(
        &mut connection,
        account_id,
        SyncTaskPayloadV1::RemoteFolder {
            remote_folder_id: "folder-a".to_string(),
            range: ArchiveListRangeV1::All,
            range_freeze: RangeFreezeV1 {
                frozen_at_ms: 1_786_420_800_000,
                cutoff_ms: None,
            },
            run_mode: ArchiveRunModeV1::Incremental,
            scroll_speed: 5_000,
            hide_browser: true,
            browser: browser(account_id),
        },
    );

    let snapshot = build_archive_known_ids_snapshot(
        &connection,
        &job,
        SyncPhase::RetryingAfterLogin,
        ATTEMPT_ID,
    )
    .unwrap();
    assert_eq!(snapshot.source, SyncSourceV1::RemoteFolder);
    assert_eq!(snapshot.source_instance_key, "folder-a");
    assert_eq!(snapshot.aweme_ids, vec!["7650000000000000010".to_string()]);
}
