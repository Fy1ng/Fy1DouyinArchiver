use douyin_history_archive::{
    device_settings::BrowserPreferenceV1,
    migrate_schema_to_v5_at_startup,
    settings_store::{ArchiveListRangeV1, RemoteFolderScheduleModeV1},
    sync::{
        build_archive_collect_invocation, freeze_task_payload, ArchiveRunModeV1,
        BrowserExecutionIdentityV1, BrowserIdV1, NewSyncJobV1, QueueStore, RangeFreezeV1,
        SyncTaskPayloadV1, SyncTriggerV1, TaskKeyV1,
    },
    sync_orchestration::SyncPhase,
    AppPaths,
};
use rusqlite::{params, Connection};
use std::{collections::BTreeMap, fs, path::PathBuf, time::SystemTime};

const FROZEN_AT_MS: i64 = 1_786_420_800_000;
const CUTOFF_MS: i64 = 1_785_859_200_000;
const ATTEMPT_ID: &str = "0123456789abcdef0123456789abcdef";

struct TestDatabase {
    root: PathBuf,
    paths: AppPaths,
}

impl TestDatabase {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "emily-archive-invocation-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp root");
        let paths = AppPaths::new(root.clone(), root.join("resources"));
        let mut connection = Connection::open(&paths.database).expect("database");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        Self { root, paths }
    }

    fn connect(&self) -> Connection {
        Connection::open(&self.paths.database).expect("database")
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

fn enqueue_running(
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

fn arg_value<'a>(args: &'a [String], name: &str) -> &'a str {
    let index = args.iter().position(|value| value == name).expect("arg");
    args.get(index + 1).expect("arg value")
}

fn has_arg(args: &[String], name: &str) -> bool {
    args.iter().any(|value| value == name)
}

#[test]
fn likes_invocation_uses_archive_page_range_and_run_mode() {
    let database = TestDatabase::new("likes");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 1);
    let job = enqueue_running(
        &mut connection,
        account_id,
        SyncTaskPayloadV1::Likes {
            range: ArchiveListRangeV1::RecentItems { count: 25 },
            range_freeze: RangeFreezeV1 {
                frozen_at_ms: FROZEN_AT_MS,
                cutoff_ms: None,
            },
            run_mode: ArchiveRunModeV1::Incremental,
            scroll_speed: 4_200,
            hide_browser: true,
            browser: browser(account_id),
        },
    );

    let invocation = build_archive_collect_invocation(
        &database.paths,
        std::env::current_exe().expect("executable"),
        &job,
        SyncPhase::Collecting,
        ATTEMPT_ID,
    )
    .expect("likes invocation");

    assert_eq!(arg_value(&invocation.args, "--source"), "likes");
    assert_eq!(arg_value(&invocation.args, "--source-instance-key"), "");
    assert_eq!(
        arg_value(&invocation.args, "--page-url"),
        "https://www.douyin.com/user/self?showTab=like"
    );
    assert_eq!(
        arg_value(&invocation.args, "--archive-range"),
        "recent_items"
    );
    assert_eq!(arg_value(&invocation.args, "--item-count"), "25");
    assert_eq!(arg_value(&invocation.args, "--run-mode"), "incremental");
    assert_eq!(arg_value(&invocation.args, "--scroll-speed"), "4200");
    assert!(invocation.args.iter().any(|value| value == "--headless"));
    assert!(invocation
        .args
        .iter()
        .any(|value| value == "--block-images"));
}

#[test]
fn favorites_invocation_carries_discovery_and_recent_days_cutoff() {
    let database = TestDatabase::new("favorites");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 2);
    let job = enqueue_running(
        &mut connection,
        account_id,
        SyncTaskPayloadV1::Favorites {
            range: ArchiveListRangeV1::RecentDays { days: 7 },
            range_freeze: RangeFreezeV1 {
                frozen_at_ms: FROZEN_AT_MS,
                cutoff_ms: Some(CUTOFF_MS),
            },
            run_mode: ArchiveRunModeV1::Incremental,
            scroll_speed: 5_000,
            hide_browser: false,
            browser: browser(account_id),
            discover_remote_folders: true,
            remote_folder_mode: RemoteFolderScheduleModeV1::Unified,
            remote_folder_default_range: ArchiveListRangeV1::All,
            remote_folder_overrides: BTreeMap::new(),
        },
    );

    let invocation = build_archive_collect_invocation(
        &database.paths,
        std::env::current_exe().expect("executable"),
        &job,
        SyncPhase::Collecting,
        ATTEMPT_ID,
    )
    .expect("favorites invocation");

    assert_eq!(arg_value(&invocation.args, "--source"), "favorites");
    assert_eq!(
        arg_value(&invocation.args, "--page-url"),
        "https://www.douyin.com/user/self?showTab=favorite_collection"
    );
    assert_eq!(
        arg_value(&invocation.args, "--archive-range"),
        "recent_days"
    );
    assert_eq!(arg_value(&invocation.args, "--days"), "7");
    assert_eq!(
        arg_value(&invocation.args, "--cutoff-ms"),
        CUTOFF_MS.to_string()
    );
    assert_eq!(arg_value(&invocation.args, "--run-mode"), "incremental");
    assert!(invocation
        .args
        .iter()
        .any(|value| value == "--discover-remote-folders"));
    assert!(!invocation.args.iter().any(|value| value == "--headless"));
}

#[test]
fn full_reconcile_invocation_requires_all_and_carries_run_mode() {
    let database = TestDatabase::new("full-reconcile");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 4);
    let job = enqueue_running(
        &mut connection,
        account_id,
        SyncTaskPayloadV1::Likes {
            range: ArchiveListRangeV1::All,
            range_freeze: RangeFreezeV1 {
                frozen_at_ms: FROZEN_AT_MS,
                cutoff_ms: None,
            },
            run_mode: ArchiveRunModeV1::FullReconcile,
            scroll_speed: 5_000,
            hide_browser: true,
            browser: browser(account_id),
        },
    );

    let invocation = build_archive_collect_invocation(
        &database.paths,
        std::env::current_exe().expect("executable"),
        &job,
        SyncPhase::Collecting,
        ATTEMPT_ID,
    )
    .expect("full reconcile invocation");

    assert_eq!(arg_value(&invocation.args, "--archive-range"), "all");
    assert_eq!(arg_value(&invocation.args, "--run-mode"), "full_reconcile");
    assert!(!has_arg(&invocation.args, "--known-ids-file"));
}

#[test]
fn incremental_all_invocation_requires_phase_scoped_known_ids() {
    let database = TestDatabase::new("incremental-all");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 5);
    let job = enqueue_running(
        &mut connection,
        account_id,
        SyncTaskPayloadV1::Likes {
            range: ArchiveListRangeV1::All,
            range_freeze: RangeFreezeV1 {
                frozen_at_ms: FROZEN_AT_MS,
                cutoff_ms: None,
            },
            run_mode: ArchiveRunModeV1::Incremental,
            scroll_speed: 5_000,
            hide_browser: true,
            browser: browser(account_id),
        },
    );

    let invocation = build_archive_collect_invocation(
        &database.paths,
        std::env::current_exe().expect("executable"),
        &job,
        SyncPhase::Collecting,
        ATTEMPT_ID,
    )
    .expect("incremental all invocation");

    assert_eq!(
        PathBuf::from(arg_value(&invocation.args, "--known-ids-file")),
        database
            .paths
            .phase_archive_known_ids_path(
                account_id,
                job.snapshot.id,
                SyncPhase::Collecting,
                ATTEMPT_ID
            )
            .unwrap()
    );
}

#[test]
fn remote_folder_invocation_binds_instance_name_and_record_limit() {
    let database = TestDatabase::new("remote-folder");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 3);
    connection
        .execute(
            "INSERT INTO remote_folders
               (account_id, remote_folder_id, name, first_seen_at, last_seen_at)
             VALUES (?1, 'folder-a', 'Folder A', 'first', 'last')",
            [account_id],
        )
        .expect("remote folder");
    let job = enqueue_running(
        &mut connection,
        account_id,
        SyncTaskPayloadV1::RemoteFolder {
            remote_folder_id: "folder-a".to_string(),
            range: ArchiveListRangeV1::RecordLimit { limit: 200 },
            range_freeze: RangeFreezeV1 {
                frozen_at_ms: FROZEN_AT_MS,
                cutoff_ms: None,
            },
            run_mode: ArchiveRunModeV1::Incremental,
            scroll_speed: 5_000,
            hide_browser: true,
            browser: browser(account_id),
        },
    );

    let invocation = build_archive_collect_invocation(
        &database.paths,
        std::env::current_exe().expect("executable"),
        &job,
        SyncPhase::RetryingAfterLogin,
        ATTEMPT_ID,
    )
    .expect("remote folder invocation");

    assert_eq!(arg_value(&invocation.args, "--source"), "remote_folder");
    assert_eq!(
        arg_value(&invocation.args, "--source-instance-key"),
        "folder-a"
    );
    assert_eq!(
        arg_value(&invocation.args, "--page-url"),
        "https://www.douyin.com/user/self?showTab=favorite_collection"
    );
    assert_eq!(
        arg_value(&invocation.args, "--archive-range"),
        "record_limit"
    );
    assert_eq!(arg_value(&invocation.args, "--record-limit"), "200");
}
