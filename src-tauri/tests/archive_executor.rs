use douyin_history_archive::{
    device_settings::BrowserPreferenceV1,
    migrate_schema_to_latest_at_startup,
    settings_store::{ArchiveListRangeV1, RemoteFolderScheduleModeV1},
    sync::{
        compute_archive_hashes, compute_remote_folder_hashes, freeze_task_payload,
        import_execution_envelope, shanghai_cutoff_ms, ArchiveCollectionSuccessV1,
        ArchiveCompleteStopV1, ArchiveCompletionV1, ArchiveIncompleteReasonV1, ArchiveItemV1,
        ArchiveResultImporter, ArchiveRunModeV1, ArchiveSyncExecutor, BatchJobRoleV1,
        BrowserExecutionIdentityV1, BrowserIdV1, ExecutorRegistration, ExecutorRegistry,
        FavoriteCollectionSuccessV1, NewSyncJobV1, QueueStore, RangeFreezeV1,
        RemoteFolderCollectionSuccessV1, RemoteFolderDescriptorV1, RemoteFolderDiscoveryResultV1,
        RemoteFolderDiscoverySuccessV1, RemoteFolderExecutionOverrideV1, SourceExecutionResultV1,
        SyncExecutionEnvelopeV1, SyncJobStatusV1, SyncScopeV1, SyncSourceV1, SyncTaskPayloadV1,
        SyncTriggerV1, TaskKeyV1,
    },
};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;
use std::{collections::BTreeMap, fs, path::PathBuf, time::SystemTime};

const FROZEN_AT_MS: i64 = 1_786_420_800_000;

struct TestDatabase {
    root: PathBuf,
    database: PathBuf,
}

impl TestDatabase {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "emily-archive-executor-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp root");
        let database = root.join("history.sqlite");
        let mut connection = Connection::open(&database).expect("database");
        migrate_schema_to_latest_at_startup(&mut connection).expect("schema");
        Self { root, database }
    }

    fn connect(&self) -> Connection {
        Connection::open(&self.database).expect("database")
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn insert_account(connection: &Connection, suffix: usize) -> i64 {
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

fn browser(account_id: i64) -> BrowserExecutionIdentityV1 {
    BrowserExecutionIdentityV1 {
        preference: BrowserPreferenceV1::Chrome,
        resolved_browser: BrowserIdV1::Chrome,
        profile_key: format!("{account_id:032x}"),
        profile_generation: "fedcba9876543210fedcba9876543210".to_string(),
        executable_path_sha256: "a".repeat(64),
    }
}

fn archive_payload(
    account_id: i64,
    source: SyncSourceV1,
    run_mode: ArchiveRunModeV1,
) -> SyncTaskPayloadV1 {
    let range_freeze = RangeFreezeV1 {
        frozen_at_ms: FROZEN_AT_MS,
        cutoff_ms: None,
    };
    match source {
        SyncSourceV1::Likes => SyncTaskPayloadV1::Likes {
            range: ArchiveListRangeV1::All,
            range_freeze,
            run_mode,
            scroll_speed: 5_000,
            hide_browser: true,
            browser: browser(account_id),
        },
        SyncSourceV1::Favorites => SyncTaskPayloadV1::Favorites {
            range: ArchiveListRangeV1::All,
            range_freeze,
            run_mode,
            scroll_speed: 5_000,
            hide_browser: true,
            browser: browser(account_id),
            discover_remote_folders: false,
            remote_folder_mode: RemoteFolderScheduleModeV1::Unified,
            remote_folder_default_range: ArchiveListRangeV1::All,
            remote_folder_overrides: BTreeMap::new(),
        },
        _ => panic!("unsupported test source"),
    }
}

fn remote_folder_payload(
    account_id: i64,
    remote_folder_id: &str,
    run_mode: ArchiveRunModeV1,
) -> SyncTaskPayloadV1 {
    SyncTaskPayloadV1::RemoteFolder {
        remote_folder_id: remote_folder_id.to_string(),
        range: ArchiveListRangeV1::All,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: FROZEN_AT_MS,
            cutoff_ms: None,
        },
        run_mode,
        scroll_speed: 5_000,
        hide_browser: true,
        browser: browser(account_id),
    }
}

fn enqueue_running_archive(
    connection: &mut Connection,
    account_id: i64,
    source: SyncSourceV1,
    run_mode: ArchiveRunModeV1,
) -> douyin_history_archive::sync::StoredSyncJobV1 {
    let frozen =
        freeze_task_payload(archive_payload(account_id, source, run_mode)).expect("freeze payload");
    let queued = QueueStore::enqueue_job(
        connection,
        &NewSyncJobV1 {
            task_key: TaskKeyV1 {
                account_id,
                source,
                source_instance_key: String::new(),
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
    .expect("enqueue");
    QueueStore::claim_job(connection, queued.snapshot.id, "2026-08-13T00:00:01.000Z")
        .expect("claim")
}

fn enqueue_running_payload(
    connection: &mut Connection,
    account_id: i64,
    payload: SyncTaskPayloadV1,
) -> douyin_history_archive::sync::StoredSyncJobV1 {
    let source = payload.source();
    let source_instance_key = payload.source_instance_key().to_string();
    let frozen = freeze_task_payload(payload).expect("freeze payload");
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
    .expect("enqueue");
    QueueStore::claim_job(connection, queued.snapshot.id, "2026-08-13T00:00:01.000Z")
        .expect("claim")
}

fn item(id: &str, position: i64) -> ArchiveItemV1 {
    ArchiveItemV1 {
        aweme_id: id.to_string(),
        canonical_url: format!("https://www.douyin.com/video/{id}"),
        content_type: "video".to_string(),
        source_position: position,
        remote_created_at_ms: None,
        title: Some(format!("title-{id}")),
        author_name: Some("author".to_string()),
        cover_url: Some("https://example.com/cover.jpg".to_string()),
        like_count: Some(42),
    }
}

fn archive(
    records: Vec<ArchiveItemV1>,
    completion: ArchiveCompletionV1,
) -> ArchiveCollectionSuccessV1 {
    let (identity_hash, field_hash) = compute_archive_hashes(&records).expect("hashes");
    ArchiveCollectionSuccessV1 {
        records,
        identity_hash,
        field_hash,
        completion,
    }
}

fn discovery(folders: Vec<RemoteFolderDescriptorV1>) -> RemoteFolderDiscoveryResultV1 {
    let (identity_hash, field_hash) =
        compute_remote_folder_hashes(&folders).expect("folder hashes");
    RemoteFolderDiscoveryResultV1::Success(RemoteFolderDiscoverySuccessV1 {
        folders,
        identity_hash,
        field_hash,
        complete_scan: true,
        bottom_confirmed: true,
        hit_record_limit: false,
    })
}

fn incomplete_discovery(folders: Vec<RemoteFolderDescriptorV1>) -> RemoteFolderDiscoveryResultV1 {
    let (identity_hash, field_hash) =
        compute_remote_folder_hashes(&folders).expect("folder hashes");
    RemoteFolderDiscoveryResultV1::Success(RemoteFolderDiscoverySuccessV1 {
        folders,
        identity_hash,
        field_hash,
        complete_scan: false,
        bottom_confirmed: false,
        hit_record_limit: false,
    })
}

fn envelope(
    job: &douyin_history_archive::sync::StoredSyncJobV1,
    result: SourceExecutionResultV1,
) -> SyncExecutionEnvelopeV1 {
    SyncExecutionEnvelopeV1 {
        schema_version: 1,
        account_id: job.snapshot.task_key.account_id,
        job_id: job.snapshot.id,
        source: job.snapshot.task_key.source,
        source_instance_key: job.snapshot.task_key.source_instance_key.clone(),
        task_payload_sha256: job.snapshot.task_payload_sha256.clone(),
        result,
    }
}

fn insert_existing_relation(
    connection: &Connection,
    account_id: i64,
    table: &str,
    aweme_id: &str,
    removed: bool,
) {
    connection
        .execute(
            "INSERT INTO videos
             (aweme_id, canonical_url, content_type, title, first_seen_at, last_seen_at)
             VALUES (?1, ?2, 'video', 'old', 'old', 'old')",
            params![aweme_id, format!("https://www.douyin.com/video/{aweme_id}")],
        )
        .expect("video");
    let video_id = connection.last_insert_rowid();
    let sql = format!(
        "INSERT INTO {table}
         (account_id, video_id, first_seen_at, last_seen_at, remote_removed_at, source_position)
         VALUES (?1, ?2, 'first', 'last', ?3, 99)"
    );
    connection
        .execute(
            &sql,
            params![
                account_id,
                video_id,
                removed.then_some("2026-08-01T00:00:00.000Z")
            ],
        )
        .expect("relation");
}

fn insert_remote_folder(
    connection: &Connection,
    account_id: i64,
    remote_folder_id: &str,
    name: &str,
    removed: bool,
) -> i64 {
    connection
        .execute(
            "INSERT INTO remote_folders
               (account_id, remote_folder_id, name, first_seen_at, last_seen_at,
                remote_removed_at)
             VALUES (?1, ?2, ?3, 'first', 'last', ?4)",
            params![
                account_id,
                remote_folder_id,
                name,
                removed.then_some("2026-08-01T00:00:00.000Z"),
            ],
        )
        .expect("remote folder");
    connection.last_insert_rowid()
}

fn insert_remote_folder_item(
    connection: &Connection,
    folder_id: i64,
    aweme_id: &str,
    removed: bool,
) {
    connection
        .execute(
            "INSERT INTO videos
               (aweme_id, canonical_url, content_type, title, first_seen_at, last_seen_at)
             VALUES (?1, ?2, 'video', 'old', 'old', 'old')",
            params![aweme_id, format!("https://www.douyin.com/video/{aweme_id}")],
        )
        .expect("video");
    let video_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_items
               (folder_id, video_id, first_seen_at, last_seen_at, remote_removed_at)
             VALUES (?1, ?2, 'first', 'last', ?3)",
            params![
                folder_id,
                video_id,
                removed.then_some("2026-08-01T00:00:00.000Z"),
            ],
        )
        .expect("remote folder item");
}

fn remote_folder_item_removed_at(
    connection: &Connection,
    folder_id: i64,
    aweme_id: &str,
) -> Option<String> {
    connection
        .query_row(
            "SELECT items.remote_removed_at
             FROM remote_folder_items items
             JOIN videos ON videos.id = items.video_id
             WHERE items.folder_id = ?1 AND videos.aweme_id = ?2",
            params![folder_id, aweme_id],
            |row| row.get(0),
        )
        .optional()
        .expect("remote folder item query")
        .flatten()
}

fn relation_removed_at(
    connection: &Connection,
    table: &str,
    account_id: i64,
    aweme_id: &str,
) -> Option<String> {
    connection
        .query_row(
            &format!(
                "SELECT relations.remote_removed_at
                 FROM {table} relations
                 JOIN videos ON videos.id = relations.video_id
                 WHERE relations.account_id = ?1 AND videos.aweme_id = ?2"
            ),
            params![account_id, aweme_id],
            |row| row.get(0),
        )
        .optional()
        .expect("relation query")
        .flatten()
}

#[test]
fn incremental_remote_folder_import_restores_seen_items_without_removing_unseen_items() {
    let database = TestDatabase::new("remote-folder-incremental");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 10);
    let folder_id = insert_remote_folder(&connection, account_id, "folder-a", "Folder A", false);
    insert_remote_folder_item(&connection, folder_id, "8100000000000000001", true);
    insert_remote_folder_item(&connection, folder_id, "8100000000000000002", false);
    let job = enqueue_running_payload(
        &mut connection,
        account_id,
        remote_folder_payload(account_id, "folder-a", ArchiveRunModeV1::Incremental),
    );
    let result = SourceExecutionResultV1::RemoteFolder(RemoteFolderCollectionSuccessV1 {
        remote_folder_id: "folder-a".to_string(),
        name: "Folder A".to_string(),
        archive: archive(
            vec![
                item("8100000000000000001", 0),
                item("8100000000000000003", 1),
            ],
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
        ),
    });

    let outcome = ArchiveResultImporter::new(SyncSourceV1::RemoteFolder)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("remote folder import");

    assert_eq!(outcome.status, SyncJobStatusV1::Success);
    assert_eq!(outcome.counts.inserted_videos, 1);
    assert_eq!(outcome.counts.inserted_records, 1);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM cover_cache_queue WHERE sync_job_id = ?1",
                [job.snapshot.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2,
        "archive imports should enqueue cover work without making it part of sync success",
    );
    assert_eq!(
        remote_folder_item_removed_at(&connection, folder_id, "8100000000000000001"),
        None
    );
    assert_eq!(
        remote_folder_item_removed_at(&connection, folder_id, "8100000000000000002"),
        None
    );
}

#[test]
fn full_reconcile_remote_folder_marks_only_unseen_items_removed() {
    let database = TestDatabase::new("remote-folder-reconcile");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 11);
    let folder_id = insert_remote_folder(&connection, account_id, "folder-b", "Folder B", false);
    insert_remote_folder_item(&connection, folder_id, "8200000000000000001", false);
    insert_remote_folder_item(&connection, folder_id, "8200000000000000002", false);
    let job = enqueue_running_payload(
        &mut connection,
        account_id,
        remote_folder_payload(account_id, "folder-b", ArchiveRunModeV1::FullReconcile),
    );
    let result = SourceExecutionResultV1::RemoteFolder(RemoteFolderCollectionSuccessV1 {
        remote_folder_id: "folder-b".to_string(),
        name: "Folder B".to_string(),
        archive: archive(
            vec![item("8200000000000000001", 0)],
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
        ),
    });

    ArchiveResultImporter::new(SyncSourceV1::RemoteFolder)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("remote folder reconcile");

    assert_eq!(
        remote_folder_item_removed_at(&connection, folder_id, "8200000000000000001"),
        None
    );
    assert!(remote_folder_item_removed_at(&connection, folder_id, "8200000000000000002").is_some());
}

#[test]
fn incomplete_remote_folder_imports_seen_items_without_removal_and_finishes_partial() {
    let database = TestDatabase::new("remote-folder-incomplete");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 12);
    let folder_id = insert_remote_folder(&connection, account_id, "folder-c", "Folder C", false);
    insert_remote_folder_item(&connection, folder_id, "8300000000000000001", false);
    let job = enqueue_running_payload(
        &mut connection,
        account_id,
        remote_folder_payload(account_id, "folder-c", ArchiveRunModeV1::Incremental),
    );
    let result = SourceExecutionResultV1::RemoteFolder(RemoteFolderCollectionSuccessV1 {
        remote_folder_id: "folder-c".to_string(),
        name: "Folder C".to_string(),
        archive: archive(
            vec![item("8300000000000000002", 0)],
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
            },
        ),
    });

    let outcome = ArchiveResultImporter::new(SyncSourceV1::RemoteFolder)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("incomplete remote folder import");

    assert_eq!(outcome.status, SyncJobStatusV1::Partial);
    assert_eq!(outcome.counts.inserted_videos, 1);
    assert_eq!(outcome.counts.inserted_records, 1);
    assert_eq!(
        remote_folder_item_removed_at(&connection, folder_id, "8300000000000000001"),
        None
    );
    let issue: String = connection
        .query_row(
            "SELECT code FROM sync_job_issues WHERE job_id = ?1",
            [job.snapshot.id],
            |row| row.get(0),
        )
        .expect("incomplete issue");
    assert_eq!(issue, "archive_unconfirmed_end");
}

#[test]
fn remote_folder_import_renames_and_restores_registry_with_one_revision_increment() {
    let database = TestDatabase::new("remote-folder-registry-update");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 13);
    insert_remote_folder(&connection, account_id, "folder-d", "Old Name", true);
    let job = enqueue_running_payload(
        &mut connection,
        account_id,
        remote_folder_payload(account_id, "folder-d", ArchiveRunModeV1::Incremental),
    );
    let result = SourceExecutionResultV1::RemoteFolder(RemoteFolderCollectionSuccessV1 {
        remote_folder_id: "folder-d".to_string(),
        name: "New Name".to_string(),
        archive: archive(
            Vec::new(),
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
        ),
    });

    ArchiveResultImporter::new(SyncSourceV1::RemoteFolder)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("remote folder registry update");

    let (name, removed_at, revision): (String, Option<String>, i64) = connection
        .query_row(
            "SELECT folders.name, folders.remote_removed_at, state.revision
             FROM remote_folders folders
             JOIN remote_folder_registry_state state
               ON state.account_id = folders.account_id
             WHERE folders.account_id = ?1 AND folders.remote_folder_id = 'folder-d'",
            [account_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("updated registry");
    assert_eq!(name, "New Name");
    assert_eq!(removed_at, None);
    assert_eq!(revision, 2);
}

#[test]
fn remote_folder_payload_id_mismatch_is_rejected_without_archive_writes() {
    let database = TestDatabase::new("remote-folder-payload-mismatch");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 14);
    insert_remote_folder(&connection, account_id, "folder-e", "Folder E", false);
    let frozen = freeze_task_payload(remote_folder_payload(
        account_id,
        "folder-other",
        ArchiveRunModeV1::Incremental,
    ))
    .expect("mismatched payload");
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &NewSyncJobV1 {
            task_key: TaskKeyV1 {
                account_id,
                source: SyncSourceV1::RemoteFolder,
                source_instance_key: "folder-e".to_string(),
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
    .expect("queued mismatched payload");
    let job = QueueStore::claim_job(
        &mut connection,
        queued.snapshot.id,
        "2026-08-13T00:00:01.000Z",
    )
    .expect("claim mismatched payload");
    let result = SourceExecutionResultV1::RemoteFolder(RemoteFolderCollectionSuccessV1 {
        remote_folder_id: "folder-e".to_string(),
        name: "Folder E".to_string(),
        archive: archive(
            vec![item("8400000000000000001", 0)],
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
        ),
    });

    let error = ArchiveResultImporter::new(SyncSourceV1::RemoteFolder)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect_err("payload mismatch");

    assert_eq!(error, "invalid_sync_job_payload");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM videos", [], |row| row
                .get::<_, i64>(0))
            .expect("video count"),
        0
    );
    let status: String = connection
        .query_row(
            "SELECT status FROM sync_jobs WHERE id = ?1",
            [job.snapshot.id],
            |row| row.get(0),
        )
        .expect("job status");
    assert_eq!(status, "failed");
}

#[test]
fn remote_folder_import_failure_rolls_back_registry_and_archive_changes() {
    let database = TestDatabase::new("remote-folder-rollback");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 15);
    insert_remote_folder(&connection, account_id, "folder-f", "Old Name", true);
    let job = enqueue_running_payload(
        &mut connection,
        account_id,
        remote_folder_payload(account_id, "folder-f", ArchiveRunModeV1::FullReconcile),
    );
    let result = SourceExecutionResultV1::RemoteFolder(RemoteFolderCollectionSuccessV1 {
        remote_folder_id: "folder-f".to_string(),
        name: "New Name".to_string(),
        archive: archive(
            vec![item("8500000000000000001", 0)],
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
            },
        ),
    });

    let error = ArchiveResultImporter::new(SyncSourceV1::RemoteFolder)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect_err("invalid full reconcile completion");

    assert_eq!(error, "invalid_archive_completion_for_payload");
    let (name, removed_at, revision): (String, Option<String>, i64) = connection
        .query_row(
            "SELECT folders.name, folders.remote_removed_at, state.revision
             FROM remote_folders folders
             JOIN remote_folder_registry_state state
               ON state.account_id = folders.account_id
             WHERE folders.account_id = ?1 AND folders.remote_folder_id = 'folder-f'",
            [account_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("rolled back registry");
    assert_eq!(name, "Old Name");
    assert!(removed_at.is_some());
    assert_eq!(revision, 1);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM videos", [], |row| row
                .get::<_, i64>(0))
            .expect("video count"),
        0
    );
    let status: String = connection
        .query_row(
            "SELECT status FROM sync_jobs WHERE id = ?1",
            [job.snapshot.id],
            |row| row.get(0),
        )
        .expect("job status");
    assert_eq!(status, "failed");
}

#[test]
fn incremental_likes_restore_seen_items_without_marking_unseen_items_removed() {
    let database = TestDatabase::new("likes-incremental");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 1);
    insert_existing_relation(
        &connection,
        account_id,
        "user_likes",
        "7100000000000000001",
        true,
    );
    insert_existing_relation(
        &connection,
        account_id,
        "user_likes",
        "7100000000000000002",
        false,
    );
    let job = enqueue_running_archive(
        &mut connection,
        account_id,
        SyncSourceV1::Likes,
        ArchiveRunModeV1::Incremental,
    );
    let result = SourceExecutionResultV1::Likes(archive(
        vec![
            item("7100000000000000001", 0),
            item("7100000000000000003", 1),
        ],
        ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::Bottom,
        },
    ));

    let outcome = ArchiveResultImporter::new(SyncSourceV1::Likes)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("incremental import");

    assert_eq!(outcome.status, SyncJobStatusV1::Success);
    assert_eq!(outcome.counts.inserted_videos, 1);
    assert_eq!(outcome.counts.inserted_records, 1);
    assert_eq!(
        relation_removed_at(&connection, "user_likes", account_id, "7100000000000000001"),
        None
    );
    assert_eq!(
        relation_removed_at(&connection, "user_likes", account_id, "7100000000000000002"),
        None
    );
}

#[test]
fn registry_import_entrypoint_routes_likes_and_terminalizes_the_job() {
    let database = TestDatabase::new("registry-likes-import");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 71);
    let job = enqueue_running_archive(
        &mut connection,
        account_id,
        SyncSourceV1::Likes,
        ArchiveRunModeV1::Incremental,
    );
    let registry = ExecutorRegistry::new(vec![ExecutorRegistration::new(
        Arc::new(ArchiveSyncExecutor::new(
            SyncSourceV1::Likes,
            |_| unreachable!("import test does not launch"),
            |_| unreachable!("import test does not cancel"),
        )),
        Arc::new(ArchiveResultImporter::new(SyncSourceV1::Likes)),
    )])
    .unwrap();
    let result = archive(
        vec![item("7100000000000000071", 0)],
        ArchiveCompletionV1::Incomplete {
            reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
        },
    );

    let outcome = import_execution_envelope(
        &registry,
        &mut connection,
        job.snapshot.id,
        envelope(&job, SourceExecutionResultV1::Likes(result)),
    )
    .expect("registry likes import");

    assert_eq!(outcome.status, SyncJobStatusV1::Partial);
    assert_eq!(outcome.counts.inserted_records, 1);
    assert_eq!(
        QueueStore::get_stored_job(&connection, job.snapshot.id)
            .unwrap()
            .snapshot
            .status,
        SyncJobStatusV1::Partial
    );
}

#[test]
fn full_reconcile_likes_marks_only_unseen_relations_removed() {
    let database = TestDatabase::new("likes-reconcile");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 2);
    insert_existing_relation(
        &connection,
        account_id,
        "user_likes",
        "7200000000000000001",
        false,
    );
    insert_existing_relation(
        &connection,
        account_id,
        "user_likes",
        "7200000000000000002",
        false,
    );
    let job = enqueue_running_archive(
        &mut connection,
        account_id,
        SyncSourceV1::Likes,
        ArchiveRunModeV1::FullReconcile,
    );
    let result = SourceExecutionResultV1::Likes(archive(
        vec![item("7200000000000000001", 0)],
        ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::Bottom,
        },
    ));

    ArchiveResultImporter::new(SyncSourceV1::Likes)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("full reconcile");

    assert_eq!(
        relation_removed_at(&connection, "user_likes", account_id, "7200000000000000001"),
        None
    );
    assert!(
        relation_removed_at(&connection, "user_likes", account_id, "7200000000000000002").is_some()
    );
}

#[test]
fn incomplete_favorites_import_seen_items_without_remote_removal_and_finish_partial() {
    let database = TestDatabase::new("favorites-incomplete");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 3);
    insert_existing_relation(
        &connection,
        account_id,
        "favorite_items",
        "7300000000000000001",
        false,
    );
    let job = enqueue_running_archive(
        &mut connection,
        account_id,
        SyncSourceV1::Favorites,
        ArchiveRunModeV1::Incremental,
    );
    let result = SourceExecutionResultV1::Favorites(FavoriteCollectionSuccessV1 {
        archive: archive(
            vec![item("7300000000000000002", 0)],
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
            },
        ),
        remote_folder_discovery: RemoteFolderDiscoveryResultV1::NotRequested,
    });

    let outcome = ArchiveResultImporter::new(SyncSourceV1::Favorites)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("incomplete import");

    assert_eq!(outcome.status, SyncJobStatusV1::Partial);
    assert_eq!(outcome.counts.inserted_records, 1);
    assert_eq!(
        relation_removed_at(
            &connection,
            "favorite_items",
            account_id,
            "7300000000000000001"
        ),
        None
    );
    let issue: String = connection
        .query_row(
            "SELECT code FROM sync_job_issues WHERE job_id = ?1",
            [job.snapshot.id],
            |row| row.get(0),
        )
        .expect("incomplete issue");
    assert_eq!(issue, "archive_unconfirmed_end");
}

#[test]
fn complete_favorites_discovery_reconciles_registry_and_increments_revision_once() {
    let database = TestDatabase::new("favorites-discovery-registry");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 4);
    connection
        .execute_batch(&format!(
            "INSERT INTO remote_folders
               (account_id, remote_folder_id, name, first_seen_at, last_seen_at, remote_removed_at)
             VALUES
               ({account_id}, 'folder-a', 'Old A', 'first-a', 'last-a', 'removed-a'),
               ({account_id}, 'folder-b', 'Folder B', 'first-b', 'last-b', NULL);"
        ))
        .expect("existing folders");
    let payload = SyncTaskPayloadV1::Favorites {
        range: ArchiveListRangeV1::All,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: FROZEN_AT_MS,
            cutoff_ms: None,
        },
        run_mode: ArchiveRunModeV1::FullReconcile,
        scroll_speed: 5_000,
        hide_browser: true,
        browser: browser(account_id),
        discover_remote_folders: true,
        remote_folder_mode: RemoteFolderScheduleModeV1::Unified,
        remote_folder_default_range: ArchiveListRangeV1::All,
        remote_folder_overrides: BTreeMap::new(),
    };
    let job = enqueue_running_payload(&mut connection, account_id, payload);
    let result = SourceExecutionResultV1::Favorites(FavoriteCollectionSuccessV1 {
        archive: archive(
            Vec::new(),
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
        ),
        remote_folder_discovery: discovery(vec![
            RemoteFolderDescriptorV1 {
                remote_folder_id: "folder-a".to_string(),
                name: "Folder A".to_string(),
                source_position: 0,
            },
            RemoteFolderDescriptorV1 {
                remote_folder_id: "folder-c".to_string(),
                name: "Folder C".to_string(),
                source_position: 1,
            },
        ]),
    });

    let outcome = ArchiveResultImporter::new(SyncSourceV1::Favorites)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("complete discovery import");

    assert_eq!(outcome.status, SyncJobStatusV1::Success);
    let folders = connection
        .prepare(
            "SELECT remote_folder_id, name, first_seen_at, remote_removed_at
             FROM remote_folders WHERE account_id = ?1 ORDER BY remote_folder_id",
        )
        .expect("folder query")
        .query_map([account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .expect("folder rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("folder values");
    assert_eq!(folders.len(), 3);
    assert_eq!(
        folders[0],
        (
            "folder-a".to_string(),
            "Folder A".to_string(),
            "first-a".to_string(),
            None,
        )
    );
    assert_eq!(folders[1].0, "folder-b");
    assert_eq!(folders[1].1, "Folder B");
    assert_eq!(folders[1].2, "first-b");
    assert!(folders[1].3.is_some());
    assert_eq!(folders[2].0, "folder-c");
    assert_eq!(folders[2].1, "Folder C");
    assert!(!folders[2].2.is_empty());
    assert_eq!(folders[2].3, None);
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM remote_folder_registry_state WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .expect("registry revision");
    assert_eq!(revision, 2);
}

#[test]
fn incomplete_favorites_discovery_upserts_seen_folders_without_removal_and_finishes_partial() {
    let database = TestDatabase::new("favorites-discovery-incomplete");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 5);
    connection
        .execute(
            "INSERT INTO remote_folders
               (account_id, remote_folder_id, name, first_seen_at, last_seen_at)
             VALUES (?1, 'folder-old', 'Old Folder', 'first-old', 'last-old')",
            [account_id],
        )
        .expect("old folder");
    let mut payload = archive_payload(
        account_id,
        SyncSourceV1::Favorites,
        ArchiveRunModeV1::Incremental,
    );
    let SyncTaskPayloadV1::Favorites {
        discover_remote_folders,
        ..
    } = &mut payload
    else {
        unreachable!();
    };
    *discover_remote_folders = true;
    let job = enqueue_running_payload(&mut connection, account_id, payload);
    let result = SourceExecutionResultV1::Favorites(FavoriteCollectionSuccessV1 {
        archive: archive(
            vec![item("7400000000000000001", 0)],
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
        ),
        remote_folder_discovery: incomplete_discovery(vec![RemoteFolderDescriptorV1 {
            remote_folder_id: "folder-new".to_string(),
            name: "New Folder".to_string(),
            source_position: 0,
        }]),
    });

    let outcome = ArchiveResultImporter::new(SyncSourceV1::Favorites)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("incomplete discovery import");

    assert_eq!(outcome.status, SyncJobStatusV1::Partial);
    let folders: Vec<(String, Option<String>)> = connection
        .prepare(
            "SELECT remote_folder_id, remote_removed_at FROM remote_folders
             WHERE account_id = ?1 ORDER BY remote_folder_id",
        )
        .expect("folder query")
        .query_map([account_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("folder rows")
        .collect::<Result<_, _>>()
        .expect("folder values");
    assert_eq!(
        folders,
        vec![
            ("folder-new".to_string(), None),
            ("folder-old".to_string(), None),
        ]
    );
    let issue: String = connection
        .query_row(
            "SELECT code FROM sync_job_issues
             WHERE job_id = ?1 AND phase = 'discovery'",
            [job.snapshot.id],
            |row| row.get(0),
        )
        .expect("discovery issue");
    assert_eq!(issue, "remote_folder_discovery_incomplete");
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM remote_folder_registry_state WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .expect("registry revision");
    assert_eq!(revision, 2);
}

#[test]
fn unchanged_complete_favorites_discovery_does_not_increment_registry_revision() {
    let database = TestDatabase::new("favorites-discovery-unchanged");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 6);
    connection
        .execute(
            "INSERT INTO remote_folders
               (account_id, remote_folder_id, name, first_seen_at, last_seen_at)
             VALUES (?1, 'folder-a', 'Folder A', 'first-a', 'last-a')",
            [account_id],
        )
        .expect("existing folder");
    let mut payload = archive_payload(
        account_id,
        SyncSourceV1::Favorites,
        ArchiveRunModeV1::Incremental,
    );
    let SyncTaskPayloadV1::Favorites {
        discover_remote_folders,
        ..
    } = &mut payload
    else {
        unreachable!();
    };
    *discover_remote_folders = true;
    let job = enqueue_running_payload(&mut connection, account_id, payload);
    let result = SourceExecutionResultV1::Favorites(FavoriteCollectionSuccessV1 {
        archive: archive(
            Vec::new(),
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
        ),
        remote_folder_discovery: discovery(vec![RemoteFolderDescriptorV1 {
            remote_folder_id: "folder-a".to_string(),
            name: "Folder A".to_string(),
            source_position: 0,
        }]),
    });

    ArchiveResultImporter::new(SyncSourceV1::Favorites)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("unchanged discovery import");

    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM remote_folder_registry_state WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .expect("registry revision");
    assert_eq!(revision, 1);
}

#[test]
fn complete_favorites_discovery_creates_frozen_remote_folder_child_for_parent_batch() {
    let database = TestDatabase::new("favorites-discovery-child");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 7);
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-13T00:00:00.000Z",
    )
    .expect("batch");
    let mut overrides = BTreeMap::new();
    overrides.insert(
        "folder-new".to_string(),
        RemoteFolderExecutionOverrideV1 {
            range: ArchiveListRangeV1::RecentDays { days: 7 },
        },
    );
    let payload = SyncTaskPayloadV1::Favorites {
        range: ArchiveListRangeV1::All,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: FROZEN_AT_MS,
            cutoff_ms: None,
        },
        run_mode: ArchiveRunModeV1::Incremental,
        scroll_speed: 7_777,
        hide_browser: false,
        browser: browser(account_id),
        discover_remote_folders: true,
        remote_folder_mode: RemoteFolderScheduleModeV1::PerFolder,
        remote_folder_default_range: ArchiveListRangeV1::All,
        remote_folder_overrides: overrides,
    };
    let job = enqueue_running_payload(&mut connection, account_id, payload);
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        job.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-13T00:00:02.000Z",
    )
    .expect("attach parent");
    let result = SourceExecutionResultV1::Favorites(FavoriteCollectionSuccessV1 {
        archive: archive(
            Vec::new(),
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
        ),
        remote_folder_discovery: discovery(vec![RemoteFolderDescriptorV1 {
            remote_folder_id: "folder-new".to_string(),
            name: "New Folder".to_string(),
            source_position: 0,
        }]),
    });

    ArchiveResultImporter::new(SyncSourceV1::Favorites)
        .import_envelope(&mut connection, job.snapshot.id, envelope(&job, result))
        .expect("discovery child import");

    let (child_id, trigger, status, settings_revision, config_sha256, payload_json) = connection
        .query_row(
            "SELECT id, trigger, status, settings_revision, config_sha256, payload_json
             FROM sync_jobs
             WHERE account_id = ?1 AND source = 'remote_folder'
               AND source_instance_key = 'folder-new'",
            [account_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .expect("remote folder child");
    assert_eq!(trigger, "manual");
    assert_eq!(status, "queued");
    assert_eq!(settings_revision, 1);
    assert_eq!(config_sha256, None);
    assert_eq!(
        serde_json::from_str::<SyncTaskPayloadV1>(&payload_json).expect("child payload"),
        SyncTaskPayloadV1::RemoteFolder {
            remote_folder_id: "folder-new".to_string(),
            range: ArchiveListRangeV1::RecentDays { days: 7 },
            range_freeze: RangeFreezeV1 {
                frozen_at_ms: FROZEN_AT_MS,
                cutoff_ms: Some(shanghai_cutoff_ms(FROZEN_AT_MS, 7).expect("cutoff")),
            },
            run_mode: ArchiveRunModeV1::Incremental,
            scroll_speed: 7_777,
            hide_browser: false,
            browser: browser(account_id),
        }
    );
    let role: String = connection
        .query_row(
            "SELECT role FROM sync_batch_jobs WHERE batch_id = ?1 AND job_id = ?2",
            params![batch_id, child_id],
            |row| row.get(0),
        )
        .expect("child batch role");
    assert_eq!(role, "discoveredChild");
    let batch_status: String = connection
        .query_row(
            "SELECT status FROM sync_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .expect("batch status");
    assert_eq!(batch_status, "queued");
}

#[test]
fn complete_reconcile_cancels_queued_jobs_for_folders_removed_from_remote() {
    let database = TestDatabase::new("favorites-discovery-removal-cancel");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 8);
    connection
        .execute(
            "INSERT INTO remote_folders
               (account_id, remote_folder_id, name, first_seen_at, last_seen_at)
             VALUES (?1, 'folder-old', 'Old Folder', 'first-old', 'last-old')",
            [account_id],
        )
        .expect("existing folder");
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-13T00:00:00.000Z",
    )
    .expect("batch");
    let parent_payload = SyncTaskPayloadV1::Favorites {
        range: ArchiveListRangeV1::All,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: FROZEN_AT_MS,
            cutoff_ms: None,
        },
        run_mode: ArchiveRunModeV1::FullReconcile,
        scroll_speed: 5_000,
        hide_browser: true,
        browser: browser(account_id),
        discover_remote_folders: true,
        remote_folder_mode: RemoteFolderScheduleModeV1::Unified,
        remote_folder_default_range: ArchiveListRangeV1::All,
        remote_folder_overrides: BTreeMap::new(),
    };
    let parent = enqueue_running_payload(&mut connection, account_id, parent_payload);
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        parent.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-13T00:00:02.000Z",
    )
    .expect("attach parent");
    let child_payload = freeze_task_payload(SyncTaskPayloadV1::RemoteFolder {
        remote_folder_id: "folder-old".to_string(),
        range: ArchiveListRangeV1::All,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: FROZEN_AT_MS,
            cutoff_ms: None,
        },
        run_mode: ArchiveRunModeV1::Incremental,
        scroll_speed: 5_000,
        hide_browser: true,
        browser: browser(account_id),
    })
    .expect("child payload");
    let child = QueueStore::enqueue_job(
        &mut connection,
        &NewSyncJobV1 {
            task_key: TaskKeyV1 {
                account_id,
                source: SyncSourceV1::RemoteFolder,
                source_instance_key: "folder-old".to_string(),
            },
            trigger: SyncTriggerV1::Manual,
            queued_at: "2026-08-13T00:00:03.000Z".to_string(),
            scheduled_for: None,
            settings_revision: 1,
            config_sha256: None,
            payload_json: child_payload.payload_json,
            payload_sha256: child_payload.payload_sha256,
            equivalence_sha256: child_payload.equivalence_sha256,
        },
    )
    .expect("queued child");
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        child.snapshot.id,
        BatchJobRoleV1::DiscoveredChild,
        "2026-08-13T00:00:04.000Z",
    )
    .expect("attach child");
    let result = SourceExecutionResultV1::Favorites(FavoriteCollectionSuccessV1 {
        archive: archive(
            Vec::new(),
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
        ),
        remote_folder_discovery: discovery(Vec::new()),
    });

    ArchiveResultImporter::new(SyncSourceV1::Favorites)
        .import_envelope(
            &mut connection,
            parent.snapshot.id,
            envelope(&parent, result),
        )
        .expect("complete discovery import");

    let removed_at: Option<String> = connection
        .query_row(
            "SELECT remote_removed_at FROM remote_folders
             WHERE account_id = ?1 AND remote_folder_id = 'folder-old'",
            [account_id],
            |row| row.get(0),
        )
        .expect("removed folder");
    assert!(removed_at.is_some());
    let (child_status, child_error): (String, Option<String>) = connection
        .query_row(
            "SELECT status, error_code FROM sync_jobs WHERE id = ?1",
            [child.snapshot.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("child status");
    assert_eq!(child_status, "cancelled");
    assert_eq!(child_error.as_deref(), Some("remote_folder_removed"));
    let batch_status: String = connection
        .query_row(
            "SELECT status FROM sync_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .expect("batch status");
    assert_eq!(batch_status, "partial");
}

#[test]
fn discovery_batch_target_limit_is_all_or_none() {
    let database = TestDatabase::new("favorites-discovery-batch-limit");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 9);
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-13T00:00:00.000Z",
    )
    .expect("batch");
    let mut payload = archive_payload(
        account_id,
        SyncSourceV1::Favorites,
        ArchiveRunModeV1::Incremental,
    );
    let SyncTaskPayloadV1::Favorites {
        discover_remote_folders,
        ..
    } = &mut payload
    else {
        unreachable!();
    };
    *discover_remote_folders = true;
    let parent = enqueue_running_payload(&mut connection, account_id, payload);
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        parent.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-13T00:00:02.000Z",
    )
    .expect("attach parent");
    connection
        .execute_batch(&format!(
            "WITH RECURSIVE seq(value) AS (
               SELECT 0
               UNION ALL
               SELECT value + 1 FROM seq WHERE value < 4997
             )
             INSERT INTO sync_batch_targets
               (batch_id, account_id, source, source_instance_key,
                target_kind, resolution_kind)
             SELECT {batch_id}, {account_id}, 'remote_folder',
                    printf('existing-%05d', value), 'task', 'job'
             FROM seq;"
        ))
        .expect("existing batch targets");
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sync_batch_targets
                 WHERE batch_id = ?1 AND target_kind = 'task'",
                [batch_id],
                |row| row.get::<_, i64>(0),
            )
            .expect("existing target count"),
        4_999
    );
    let result = SourceExecutionResultV1::Favorites(FavoriteCollectionSuccessV1 {
        archive: archive(
            Vec::new(),
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
        ),
        remote_folder_discovery: discovery(vec![
            RemoteFolderDescriptorV1 {
                remote_folder_id: "folder-new-a".to_string(),
                name: "New A".to_string(),
                source_position: 0,
            },
            RemoteFolderDescriptorV1 {
                remote_folder_id: "folder-new-b".to_string(),
                name: "New B".to_string(),
                source_position: 1,
            },
        ]),
    });

    ArchiveResultImporter::new(SyncSourceV1::Favorites)
        .import_envelope(
            &mut connection,
            parent.snapshot.id,
            envelope(&parent, result),
        )
        .expect("limited discovery import");

    let task_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sync_batch_targets
             WHERE batch_id = ?1 AND target_kind = 'task'",
            [batch_id],
            |row| row.get(0),
        )
        .expect("task count");
    assert_eq!(task_count, 4_999);
    let child_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sync_jobs
             WHERE account_id = ?1 AND source = 'remote_folder'
               AND source_instance_key IN ('folder-new-a', 'folder-new-b')",
            [account_id],
            |row| row.get(0),
        )
        .expect("child count");
    assert_eq!(child_count, 0);
    let aggregate: (String, String, String) = connection
        .query_row(
            "SELECT targets.target_kind, issues.source_instance_key, issues.reason
             FROM sync_batch_targets targets
             JOIN sync_batch_issues issues
               ON issues.batch_id = targets.batch_id
              AND issues.account_id = targets.account_id
              AND issues.source = targets.source
              AND issues.source_instance_key = targets.source_instance_key
             WHERE targets.batch_id = ?1 AND targets.target_kind = 'aggregate'",
            [batch_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("aggregate skip");
    assert_eq!(
        aggregate,
        (
            "aggregate".to_string(),
            String::new(),
            "sync_batch_target_limit_exceeded".to_string(),
        )
    );
}
