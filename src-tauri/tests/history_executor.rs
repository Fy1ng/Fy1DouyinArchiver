use chrono::{FixedOffset, SecondsFormat, TimeZone, Utc};
use douyin_history_archive::{
    collector::{normalization::HistoryTimeUnit, sync_engine::finish_result},
    device_settings::BrowserPreferenceV1,
    migrate_schema_to_latest_at_startup,
    settings_store::HistoryRangeV1,
    sync::{
        build_history_collect_invocation, cancel_sync_job, decode_execution_envelope,
        freeze_task_payload, import_execution_envelope, BatchJobRoleV1, BrowserExecutionIdentityV1,
        BrowserIdV1, ExecutorRegistration, ExecutorRegistry, HistoryResultImporter,
        HistorySyncExecutor, NewSyncJobV1, QueueStore, QueuedSyncContext, RangeFreezeV1,
        RunningSyncIdentity, SourceExecutionResultV1, SyncBatchEntryV1, SyncBatchStatusV1,
        SyncExecutionEnvelopeV1, SyncExecutor, SyncJobIssueSeverityV1, SyncJobIssueV1,
        SyncJobStatusV1, SyncScopeV1, SyncSourceV1, SyncTaskPayloadV1, SyncTriggerV1, TaskKeyV1,
        TerminalJobUpdateV1,
    },
    sync_orchestration::SyncPhase,
    AppPaths, CollectorRange, CollectorRecordV2,
};
use rusqlite::{params, Connection};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::SystemTime,
};

const FROZEN_AT_MS: i64 = 1_786_420_800_000;
const SEVEN_DAY_CUTOFF_MS: i64 = 1_785_859_200_000;
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
            "emily-history-executor-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp root");
        let paths = AppPaths::new(root.clone(), root.join("resources"));
        let mut connection = Connection::open(&paths.database).expect("database");
        migrate_schema_to_latest_at_startup(&mut connection).expect("schema");
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

fn history_payload(account_id: i64) -> SyncTaskPayloadV1 {
    SyncTaskPayloadV1::History {
        range: HistoryRangeV1::Days { days: 7 },
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: FROZEN_AT_MS,
            cutoff_ms: Some(SEVEN_DAY_CUTOFF_MS),
        },
        scroll_speed: 5_000,
        hide_browser: true,
        browser: BrowserExecutionIdentityV1 {
            preference: BrowserPreferenceV1::Chrome,
            resolved_browser: BrowserIdV1::Chrome,
            profile_key: format!("{account_id:032x}"),
            profile_generation: "fedcba9876543210fedcba9876543210".to_string(),
            executable_path_sha256: "a".repeat(64),
        },
    }
}

fn enqueue_history(
    connection: &mut Connection,
    account_id: i64,
) -> douyin_history_archive::sync::StoredSyncJobV1 {
    let frozen = freeze_task_payload(history_payload(account_id)).expect("freeze payload");
    let queued = QueueStore::enqueue_job(
        connection,
        &NewSyncJobV1 {
            task_key: TaskKeyV1 {
                account_id,
                source: SyncSourceV1::History,
                source_instance_key: String::new(),
            },
            trigger: SyncTriggerV1::Manual,
            queued_at: "2026-08-12T00:00:00.000Z".to_string(),
            scheduled_for: None,
            settings_revision: 1,
            config_sha256: None,
            payload_json: frozen.payload_json,
            payload_sha256: frozen.payload_sha256,
            equivalence_sha256: frozen.equivalence_sha256,
        },
    )
    .expect("enqueue");
    queued
}

fn enqueue_running_history(
    connection: &mut Connection,
    account_id: i64,
) -> douyin_history_archive::sync::StoredSyncJobV1 {
    let queued = enqueue_history(connection, account_id);
    QueueStore::claim_job(connection, queued.snapshot.id, "2026-08-12T00:00:01.000Z")
        .expect("claim")
}

fn history_registry(calls: Arc<Mutex<Vec<String>>>) -> ExecutorRegistry {
    let start_calls = Arc::clone(&calls);
    let cancel_calls = calls;
    ExecutorRegistry::new(vec![ExecutorRegistration::new(
        Arc::new(HistorySyncExecutor::new(
            move |context| {
                start_calls
                    .lock()
                    .unwrap()
                    .push(format!("start:{}", context.job.id));
                Ok(RunningSyncIdentity {
                    job_id: context.job.id,
                    process_id: Some(4242),
                })
            },
            move |running| {
                cancel_calls
                    .lock()
                    .unwrap()
                    .push(format!("cancel:{}", running.job_id));
                Ok(())
            },
        )),
        Arc::new(HistoryResultImporter),
    )])
    .unwrap()
}

fn record() -> CollectorRecordV2 {
    let history_time_ms = 1_786_377_600_000;
    let watched = Utc
        .timestamp_millis_opt(history_time_ms)
        .single()
        .expect("timestamp");
    let shanghai = FixedOffset::east_opt(8 * 60 * 60).expect("offset");
    CollectorRecordV2 {
        aweme_id: "7312345678901234567".to_string(),
        aweme_type: Some(0),
        canonical_url: "https://www.douyin.com/video/7312345678901234567".to_string(),
        content_type: "video".to_string(),
        history_time_ms,
        watched_at: watched.to_rfc3339_opts(SecondsFormat::Millis, true),
        watched_date: watched
            .with_timezone(&shanghai)
            .format("%Y-%m-%d")
            .to_string(),
        source_position: 0,
        title: Some("fixture".to_string()),
        author_name: Some("author".to_string()),
        cover_url: Some("https://example.com/cover.jpg".to_string()),
        like_count: Some(42),
        watch_progress_rate: Some(0.5),
    }
}

fn envelope(account_id: i64, job_id: i64, payload_sha256: String) -> SyncExecutionEnvelopeV1 {
    let result = finish_result(
        account_id,
        job_id,
        FROZEN_AT_MS,
        CollectorRange::Days {
            days: 7,
            cutoff_ms: SEVEN_DAY_CUTOFF_MS,
        },
        HistoryTimeUnit::Milliseconds,
        vec![record()],
    )
    .expect("collector result");
    SyncExecutionEnvelopeV1 {
        schema_version: 1,
        account_id,
        job_id,
        source: SyncSourceV1::History,
        source_instance_key: String::new(),
        task_payload_sha256: payload_sha256,
        result: SourceExecutionResultV1::History(result),
    }
}

fn arg_value<'a>(args: &'a [String], name: &str) -> &'a str {
    let index = args.iter().position(|value| value == name).expect(name);
    args.get(index + 1).expect("option value")
}

#[test]
fn history_executor_delegates_start_and_cancel_with_the_exact_job_identity() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let start_calls = Arc::clone(&calls);
    let cancel_calls = Arc::clone(&calls);
    let executor = HistorySyncExecutor::new(
        move |context| {
            start_calls
                .lock()
                .unwrap()
                .push(format!("start:{}", context.job.id));
            Ok(RunningSyncIdentity {
                job_id: context.job.id,
                process_id: Some(4242),
            })
        },
        move |running| {
            cancel_calls
                .lock()
                .unwrap()
                .push(format!("cancel:{}", running.job_id));
            Ok(())
        },
    );
    let context = QueuedSyncContext {
        job: douyin_history_archive::sync::SyncJobSnapshotV1 {
            id: 7,
            task_key: TaskKeyV1 {
                account_id: 11,
                source: SyncSourceV1::History,
                source_instance_key: String::new(),
            },
            trigger: SyncTriggerV1::Manual,
            status: SyncJobStatusV1::Running,
            task_payload_sha256: "a".repeat(64),
        },
    };

    let running = executor.start(context).unwrap();
    executor.cancel(&running).unwrap();

    assert_eq!(executor.source(), SyncSourceV1::History);
    assert_eq!(running.process_id, Some(4242));
    assert_eq!(&*calls.lock().unwrap(), &["start:7", "cancel:7"]);
}

#[test]
fn cancelling_a_queued_history_job_terminalizes_it_without_starting_the_executor() {
    let database = TestDatabase::new("cancel-queued");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 10);
    let queued = enqueue_history(&mut connection, account_id);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = history_registry(Arc::clone(&calls));

    let status = cancel_sync_job(
        &mut connection,
        &registry,
        account_id,
        queued.snapshot.id,
        None,
        "2026-08-12T00:00:02.000Z",
    )
    .unwrap();

    assert_eq!(status, SyncJobStatusV1::Cancelled);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn cancelling_a_running_history_job_routes_to_the_registered_executor() {
    let database = TestDatabase::new("cancel-running");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 11);
    let running = enqueue_running_history(&mut connection, account_id);
    let identity = RunningSyncIdentity {
        job_id: running.snapshot.id,
        process_id: Some(4242),
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = history_registry(Arc::clone(&calls));

    let status = cancel_sync_job(
        &mut connection,
        &registry,
        account_id,
        running.snapshot.id,
        Some(&identity),
        "2026-08-12T00:00:02.000Z",
    )
    .unwrap();

    assert_eq!(status, SyncJobStatusV1::Running);
    assert_eq!(
        &*calls.lock().unwrap(),
        &[format!("cancel:{}", running.snapshot.id)]
    );
}

#[test]
fn cancelling_a_terminal_history_job_is_idempotent() {
    let database = TestDatabase::new("cancel-terminal");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 12);
    let running = enqueue_running_history(&mut connection, account_id);
    QueueStore::finish_running_job(
        &mut connection,
        running.snapshot.id,
        &TerminalJobUpdateV1 {
            status: SyncJobStatusV1::Failed,
            finished_at: "2026-08-12T00:00:02.000Z".to_string(),
            inserted_videos: 0,
            inserted_records: 0,
            error_code: Some("fixture_failure".to_string()),
            error_message: None,
        },
    )
    .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = history_registry(Arc::clone(&calls));

    let status = cancel_sync_job(
        &mut connection,
        &registry,
        account_id,
        running.snapshot.id,
        None,
        "2026-08-12T00:00:03.000Z",
    )
    .unwrap();

    assert_eq!(status, SyncJobStatusV1::Failed);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn history_invocation_uses_the_frozen_payload_and_browser_identity() {
    let database = TestDatabase::new("invocation");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 1);
    let job = enqueue_running_history(&mut connection, account_id);
    let executable = std::env::current_exe().expect("current exe");

    let invocation = build_history_collect_invocation(
        &database.paths,
        executable.clone(),
        &job,
        SyncPhase::Collecting,
        ATTEMPT_ID,
    )
    .expect("invocation");

    assert_eq!(invocation.program, executable);
    assert_eq!(
        arg_value(&invocation.args, "--account-id"),
        account_id.to_string()
    );
    assert_eq!(
        arg_value(&invocation.args, "--job-id"),
        job.snapshot.id.to_string()
    );
    assert_eq!(arg_value(&invocation.args, "--source"), "history");
    assert_eq!(arg_value(&invocation.args, "--source-instance-key"), "");
    assert_eq!(
        arg_value(&invocation.args, "--task-payload-sha256"),
        job.snapshot.task_payload_sha256
    );
    assert_eq!(
        arg_value(&invocation.args, "--frozen-at-ms"),
        FROZEN_AT_MS.to_string()
    );
    assert_eq!(
        arg_value(&invocation.args, "--cutoff-ms"),
        SEVEN_DAY_CUTOFF_MS.to_string()
    );
    assert_eq!(arg_value(&invocation.args, "--days"), "7");
    assert_eq!(arg_value(&invocation.args, "--scroll-speed"), "5000");
    assert_eq!(arg_value(&invocation.args, "--browser"), "chrome");
    assert_eq!(
        arg_value(&invocation.args, "--profile-key"),
        format!("{account_id:032x}")
    );
    assert_eq!(
        arg_value(&invocation.args, "--profile-generation"),
        "fedcba9876543210fedcba9876543210"
    );
    assert_eq!(
        arg_value(&invocation.args, "--browser-executable-sha256"),
        "a".repeat(64)
    );
    assert!(invocation.args.iter().any(|value| value == "--headless"));
    assert!(invocation
        .args
        .iter()
        .any(|value| value == "--block-images"));
}

#[test]
fn execution_envelope_decoder_rejects_legacy_unknown_and_mismatched_roots() {
    let database = TestDatabase::new("envelope");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 2);
    let job = enqueue_running_history(&mut connection, account_id);
    let valid = envelope(
        account_id,
        job.snapshot.id,
        job.snapshot.task_payload_sha256.clone(),
    );
    let bytes = serde_json::to_vec(&valid).expect("json");
    assert_eq!(
        decode_execution_envelope(&bytes).expect("decode").job_id,
        job.snapshot.id
    );

    let SourceExecutionResultV1::History(raw_history) = &valid.result else {
        unreachable!();
    };
    assert!(decode_execution_envelope(&serde_json::to_vec(raw_history).unwrap()).is_err());

    let mut unknown = serde_json::to_value(&valid).unwrap();
    unknown["extra"] = json!(true);
    assert!(decode_execution_envelope(&serde_json::to_vec(&unknown).unwrap()).is_err());

    let mut wrong_source = serde_json::to_value(&valid).unwrap();
    wrong_source["source"] = json!("likes");
    assert!(HistoryResultImporter::default()
        .import_envelope(
            &mut connection,
            job.snapshot.id,
            decode_execution_envelope(&serde_json::to_vec(&wrong_source).unwrap()).unwrap(),
        )
        .is_err());

    let mut tampered = valid;
    tampered.task_payload_sha256 = "b".repeat(64);
    assert!(HistoryResultImporter::default()
        .import_envelope(&mut connection, job.snapshot.id, tampered)
        .is_err());
}

#[test]
fn history_importer_is_atomic_deduplicates_events_and_refreshes_batches() {
    let database = TestDatabase::new("import");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 3);
    let first = enqueue_running_history(&mut connection, account_id);
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T00:00:02.000Z",
    )
    .expect("batch");
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        first.snapshot.id,
        BatchJobRoleV1::Root,
        "2026-08-12T00:00:03.000Z",
    )
    .expect("attach");

    let outcome = HistoryResultImporter::default()
        .import_envelope(
            &mut connection,
            first.snapshot.id,
            envelope(
                account_id,
                first.snapshot.id,
                first.snapshot.task_payload_sha256.clone(),
            ),
        )
        .expect("first import");
    assert_eq!(outcome.counts.inserted_videos, 1);
    assert_eq!(outcome.counts.inserted_records, 1);
    assert_eq!(outcome.counts.inserted_watch_records, 1);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM cover_cache_queue WHERE sync_job_id = ?1",
                [first.snapshot.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1,
        "a successful history import should only enqueue cover work inside its own transaction",
    );
    assert_eq!(outcome.status, SyncJobStatusV1::Success);
    assert_eq!(
        QueueStore::get_batch(&connection, batch_id).unwrap().status,
        SyncBatchStatusV1::Success
    );
    let page = QueueStore::list_batch_entries(&connection, batch_id, None, 200).unwrap();
    let SyncBatchEntryV1::Job(snapshot) = &page.entries[0] else {
        panic!("expected job entry");
    };
    assert_eq!(snapshot.status, SyncJobStatusV1::Success);

    let second = enqueue_running_history(&mut connection, account_id);
    let repeated = HistoryResultImporter::default()
        .import_envelope(
            &mut connection,
            second.snapshot.id,
            envelope(
                account_id,
                second.snapshot.id,
                second.snapshot.task_payload_sha256.clone(),
            ),
        )
        .expect("repeat import");
    assert_eq!(repeated.counts.inserted_videos, 0);
    assert_eq!(repeated.counts.inserted_records, 0);
    assert_eq!(repeated.counts.inserted_watch_records, 0);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM watch_records", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn registry_import_entrypoint_routes_history_and_terminalizes_the_job() {
    let database = TestDatabase::new("registry-import");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 31);
    let job = enqueue_running_history(&mut connection, account_id);
    let registry = history_registry(Arc::new(Mutex::new(Vec::new())));

    let outcome = import_execution_envelope(
        &registry,
        &mut connection,
        job.snapshot.id,
        envelope(
            account_id,
            job.snapshot.id,
            job.snapshot.task_payload_sha256.clone(),
        ),
    )
    .expect("registry import");

    assert_eq!(outcome.status, SyncJobStatusV1::Success);
    assert_eq!(outcome.counts.inserted_watch_records, 1);
    assert_eq!(
        QueueStore::get_stored_job(&connection, job.snapshot.id)
            .unwrap()
            .snapshot
            .status,
        SyncJobStatusV1::Success
    );
}

#[test]
fn cancellation_and_import_failures_never_leave_partial_source_writes() {
    let database = TestDatabase::new("races");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 4);
    let cancelled = enqueue_running_history(&mut connection, account_id);
    QueueStore::finish_running_job(
        &mut connection,
        cancelled.snapshot.id,
        &TerminalJobUpdateV1 {
            status: SyncJobStatusV1::Cancelled,
            finished_at: "2026-08-12T00:00:02.000Z".to_string(),
            inserted_videos: 0,
            inserted_records: 0,
            error_code: Some("cancelled_by_user".to_string()),
            error_message: None,
        },
    )
    .expect("cancel");
    assert!(HistoryResultImporter::default()
        .import_envelope(
            &mut connection,
            cancelled.snapshot.id,
            envelope(
                account_id,
                cancelled.snapshot.id,
                cancelled.snapshot.task_payload_sha256.clone(),
            ),
        )
        .is_err());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM videos", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    let failed = enqueue_running_history(&mut connection, account_id);
    QueueStore::append_job_issue(
        &mut connection,
        failed.snapshot.id,
        &SyncJobIssueV1 {
            phase: "collection".to_string(),
            code: "informational_warning".to_string(),
            severity: SyncJobIssueSeverityV1::Warning,
            message: None,
        },
        "2026-08-12T00:00:03.000Z",
    )
    .expect("warning");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_history_import
             BEFORE INSERT ON watch_records
             BEGIN SELECT RAISE(ABORT, 'forced history import failure'); END;",
        )
        .expect("trigger");
    assert!(HistoryResultImporter::default()
        .import_envelope(
            &mut connection,
            failed.snapshot.id,
            envelope(
                account_id,
                failed.snapshot.id,
                failed.snapshot.task_payload_sha256.clone(),
            ),
        )
        .is_err());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM videos", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        QueueStore::get_stored_job(&connection, failed.snapshot.id)
            .unwrap()
            .snapshot
            .status,
        SyncJobStatusV1::Failed
    );
    connection
        .execute_batch("DROP TRIGGER fail_history_import;")
        .unwrap();

    let incomplete = enqueue_running_history(&mut connection, account_id);
    QueueStore::append_job_issue(
        &mut connection,
        incomplete.snapshot.id,
        &SyncJobIssueV1 {
            phase: "collection".to_string(),
            code: "history_incomplete".to_string(),
            severity: SyncJobIssueSeverityV1::Incomplete,
            message: None,
        },
        "2026-08-12T00:00:04.000Z",
    )
    .expect("incomplete");
    let outcome = HistoryResultImporter::default()
        .import_envelope(
            &mut connection,
            incomplete.snapshot.id,
            envelope(
                account_id,
                incomplete.snapshot.id,
                incomplete.snapshot.task_payload_sha256.clone(),
            ),
        )
        .expect("partial import");
    assert_eq!(outcome.status, SyncJobStatusV1::Partial);
}

#[test]
fn incomplete_issues_only_degrade_successful_jobs() {
    let database = TestDatabase::new("terminal-status");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 5);

    for (index, status) in [SyncJobStatusV1::Failed, SyncJobStatusV1::Cancelled]
        .into_iter()
        .enumerate()
    {
        let job = enqueue_running_history(&mut connection, account_id);
        QueueStore::append_job_issue(
            &mut connection,
            job.snapshot.id,
            &SyncJobIssueV1 {
                phase: "collection".to_string(),
                code: "history_incomplete".to_string(),
                severity: SyncJobIssueSeverityV1::Incomplete,
                message: None,
            },
            "2026-08-12T00:00:04.000Z",
        )
        .expect("incomplete issue");

        let finished = QueueStore::finish_running_job(
            &mut connection,
            job.snapshot.id,
            &TerminalJobUpdateV1 {
                status,
                finished_at: format!("2026-08-12T00:00:0{}.000Z", index + 5),
                inserted_videos: 0,
                inserted_records: 0,
                error_code: Some("technical_terminal".to_string()),
                error_message: None,
            },
        )
        .expect("finish terminal job");

        assert_eq!(finished.snapshot.status, status);
    }
}
