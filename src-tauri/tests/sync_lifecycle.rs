use douyin_history_archive::sync::{
    automatic_config_hash, freeze_task_payload, AutomaticScheduleConfigV1, AutomaticSyncConfigV1,
    BasicSyncWorkerValidator, BrowserExecutionIdentityV1, BrowserIdV1, BrowserOperationKind,
    CapabilityRegistry, ContinuationAction, ExecutorRegistration, ExecutorRegistry,
    FakeSyncBackend, NewSyncJobV1, OperationAdmission, OperationAdmissionMode, QueueStore,
    RangeFreezeV1, RuntimeSyncWorkerValidator, StoredSyncJobV1, SyncJobSnapshotV1, SyncJobStatusV1,
    SyncSourceV1, SyncTaskPayloadV1, SyncTriggerV1, SyncWorkerConfig, SyncWorkerHandle,
    SyncWorkerTerminalObserver, SyncWorkerValidator, TaskKeyV1, TerminalJobUpdateV1,
    WorkerJobDecision,
};
use douyin_history_archive::{
    device_settings::BrowserPreferenceV1, migrate_schema_to_v5_at_startup,
    settings_store::HistoryRangeV1,
};
use rusqlite::{params, Connection};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Barrier, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

fn job(id: i64, trigger: SyncTriggerV1) -> SyncJobSnapshotV1 {
    SyncJobSnapshotV1 {
        id,
        task_key: TaskKeyV1 {
            account_id: 7,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
        },
        trigger,
        status: SyncJobStatusV1::Queued,
        task_payload_sha256: "a".repeat(64),
    }
}

struct TestDatabase {
    root: PathBuf,
    path: PathBuf,
}

impl TestDatabase {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "emily-sync-worker-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp root");
        let path = root.join("history.sqlite");
        let mut connection = Connection::open(&path).expect("database");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        Self { root, path }
    }

    fn connect(&self) -> Connection {
        Connection::open(&self.path).expect("database")
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

fn queued_job(
    account_id: i64,
    trigger: SyncTriggerV1,
    queued_at: &str,
    scheduled_for: Option<&str>,
) -> NewSyncJobV1 {
    let frozen = freeze_task_payload(SyncTaskPayloadV1::History {
        range: HistoryRangeV1::Bottom,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: 1_786_507_200_000,
            cutoff_ms: None,
        },
        scroll_speed: 5_000,
        hide_browser: true,
        browser: BrowserExecutionIdentityV1 {
            preference: BrowserPreferenceV1::Auto,
            resolved_browser: BrowserIdV1::Edge,
            profile_key: format!("{account_id:032x}"),
            profile_generation: format!("{:032x}", account_id + 100),
            executable_path_sha256: "b".repeat(64),
        },
    })
    .expect("payload");
    NewSyncJobV1 {
        task_key: TaskKeyV1 {
            account_id,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
        },
        trigger,
        queued_at: queued_at.to_string(),
        scheduled_for: scheduled_for.map(str::to_string),
        settings_revision: 1,
        config_sha256: (trigger == SyncTriggerV1::Automatic).then(|| "c".repeat(64)),
        payload_json: frozen.payload_json,
        payload_sha256: frozen.payload_sha256,
        equivalence_sha256: frozen.equivalence_sha256,
    }
}

fn registry(backend: Arc<FakeSyncBackend>) -> Arc<ExecutorRegistry> {
    Arc::new(
        ExecutorRegistry::new(vec![ExecutorRegistration::new(backend.clone(), backend)])
            .expect("registry"),
    )
}

fn wait_for_status(path: &Path, job_id: i64, expected: SyncJobStatusV1) -> StoredSyncJobV1 {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let connection = Connection::open(path).expect("database");
        let job = QueueStore::get_stored_job(&connection, job_id).expect("job");
        if job.snapshot.status == expected {
            return job;
        }
        assert!(
            Instant::now() < deadline,
            "job {job_id} did not reach {expected:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn finish_job(path: &Path, job_id: i64) {
    let mut connection = Connection::open(path).expect("database");
    QueueStore::finish_running_job(
        &mut connection,
        job_id,
        &TerminalJobUpdateV1 {
            status: SyncJobStatusV1::Success,
            finished_at: "2026-08-12T05:00:00.000Z".to_string(),
            inserted_videos: 0,
            inserted_records: 0,
            error_code: None,
            error_message: None,
        },
    )
    .expect("finish job");
}

#[test]
fn browser_leases_and_update_gates_are_mutually_exclusive() {
    let admission = OperationAdmission::new();
    let login = admission
        .begin_browser_operation(BrowserOperationKind::Login, Some(7))
        .expect("login lease");

    assert_eq!(
        admission
            .admit_sync(job(1, SyncTriggerV1::Manual))
            .unwrap_err(),
        "account_operation_in_progress"
    );
    assert_eq!(
        admission.begin_maintenance().unwrap_err(),
        "browser_operation_in_progress"
    );

    drop(login);
    let legacy_sync = admission
        .begin_browser_operation(BrowserOperationKind::Sync, Some(7))
        .expect("legacy sync lease");
    assert_eq!(
        admission
            .begin_browser_operation(BrowserOperationKind::Login, Some(8))
            .unwrap_err(),
        "browser_operation_in_progress"
    );
    drop(legacy_sync);
    let download = admission
        .begin_browser_operation(BrowserOperationKind::Download, None)
        .expect("download lease");
    assert_eq!(
        admission
            .begin_browser_operation(BrowserOperationKind::Login, Some(8))
            .unwrap_err(),
        "browser_operation_in_progress"
    );
    assert_eq!(
        admission.begin_maintenance().unwrap_err(),
        "browser_operation_in_progress"
    );
    drop(download);
    let browser_update = admission
        .begin_account_browser_update(7)
        .expect("browser update gate");
    assert_eq!(
        admission
            .admit_sync(job(2, SyncTriggerV1::Manual))
            .unwrap_err(),
        "account_browser_operation_in_progress"
    );
    drop(browser_update);

    assert!(admission.admit_sync(job(3, SyncTriggerV1::Manual)).is_ok());
}

#[test]
fn pause_and_source_update_barriers_only_block_affected_automatic_jobs() {
    let admission = OperationAdmission::new();
    let manual = job(1, SyncTriggerV1::Manual);
    let automatic = job(2, SyncTriggerV1::Automatic);

    let pause = admission
        .begin_automatic_pause_update()
        .expect("pause update");
    assert!(admission.can_dequeue(&manual).expect("manual admission"));
    assert!(!admission
        .can_dequeue(&automatic)
        .expect("automatic admission"));
    pause.commit(true).expect("commit pause");
    assert!(!admission.can_dequeue(&automatic).expect("paused automatic"));

    admission.set_automatic_paused(false).expect("resume");
    let source_update = admission
        .begin_source_settings_update(automatic.task_key.clone())
        .expect("source update gate");
    assert!(admission.can_dequeue(&manual).expect("manual admission"));
    assert!(!admission
        .can_dequeue(&automatic)
        .expect("matching automatic"));

    let other_source = SyncJobSnapshotV1 {
        task_key: TaskKeyV1 {
            source: SyncSourceV1::Likes,
            ..automatic.task_key.clone()
        },
        ..automatic.clone()
    };
    assert!(admission
        .can_dequeue(&other_source)
        .expect("unaffected automatic"));
    drop(source_update);
    assert!(admission
        .can_dequeue(&automatic)
        .expect("released automatic"));

    let account_update = admission
        .begin_account_source_settings_update(7)
        .expect("account source update");
    assert!(!admission
        .can_dequeue(&automatic)
        .expect("account automatic"));
    assert!(admission.can_dequeue(&manual).expect("account manual"));
    drop(account_update);
}

#[test]
fn manual_enqueue_is_owned_by_exit_and_cannot_start_after_exit_pending() {
    let admission = OperationAdmission::new();
    let enqueue = admission
        .begin_enqueue_operation()
        .expect("begin manual enqueue");
    let cancellation = enqueue.cancellation_token();
    let exit = admission.begin_exit().expect("begin exit");
    exit.request_owned_cancellation().expect("request cancellation");
    assert!(cancellation.is_cancelled());
    assert_eq!(
        exit.wait_until_prepared(Duration::from_millis(10))
            .unwrap_err(),
        "operation_timeout"
    );
    drop(enqueue);
    exit.wait_until_prepared(Duration::from_secs(1))
        .expect("enqueue finished");
    exit.abort().expect("abort exit");

    let exit = admission.begin_exit().expect("begin second exit");
    assert_eq!(
        admission.begin_enqueue_operation().unwrap_err(),
        "operation_admission_closed"
    );
    exit.abort().expect("abort second exit");
}

#[test]
fn browser_preference_updates_and_enqueue_operations_are_mutually_exclusive() {
    let admission = OperationAdmission::new();
    let enqueue = admission
        .begin_enqueue_operation()
        .expect("begin enqueue operation");
    assert_eq!(
        admission.begin_account_browser_update(7).unwrap_err(),
        "account_browser_operation_in_progress"
    );
    drop(enqueue);

    let browser_update = admission
        .begin_account_browser_update(7)
        .expect("begin browser update");
    assert_eq!(
        admission.begin_enqueue_operation().unwrap_err(),
        "account_browser_operation_in_progress"
    );
    drop(browser_update);
    assert!(admission.begin_enqueue_operation().is_ok());
}

#[test]
fn maintenance_registers_placeholder_and_reopens_after_timeout() {
    let admission = OperationAdmission::new();
    let running = admission
        .admit_sync(job(1, SyncTriggerV1::Manual))
        .expect("running sync admission");
    let maintenance = admission.begin_maintenance().expect("maintenance pending");

    let snapshot = admission.snapshot().expect("snapshot");
    assert_eq!(snapshot.mode, OperationAdmissionMode::MaintenancePending);
    assert_eq!(snapshot.owned_operation_count, 1);
    assert_eq!(snapshot.continuation_count, 1);

    assert_eq!(
        maintenance
            .wait_until_ready(Duration::from_millis(20))
            .unwrap_err(),
        "operation_timeout"
    );
    assert_eq!(
        admission.snapshot().expect("reopened snapshot").mode,
        OperationAdmissionMode::Open
    );
    drop(running);
}

#[test]
fn continuation_is_bound_to_one_job_and_has_narrow_permissions() {
    let admission = OperationAdmission::new();
    let running = admission
        .admit_sync(job(42, SyncTriggerV1::Manual))
        .expect("sync admission");
    let continuation = running.continuation();

    assert_eq!(continuation.job_id(), 42);
    assert_eq!(continuation.task_key().account_id, 7);
    for action in [
        ContinuationAction::WriteProgress,
        ContinuationAction::ImportResult,
        ContinuationAction::AssociateBatchChild,
        ContinuationAction::WriteTerminalState,
    ] {
        assert!(continuation.allows(action));
    }
    for action in [
        ContinuationAction::StartBrowser,
        ContinuationAction::EnqueueJob,
        ContinuationAction::ModifySettings,
    ] {
        assert!(!continuation.allows(action));
    }
}

#[test]
fn aborting_exit_waits_for_the_last_owned_operation() {
    let admission = OperationAdmission::new();
    let settings = admission
        .begin_device_settings_update()
        .expect("device settings operation");
    let exit = admission.begin_exit().expect("exit pending");
    assert_eq!(
        admission.snapshot().expect("exit snapshot").mode,
        OperationAdmissionMode::ExitPending
    );

    exit.abort().expect("request exit abort");
    assert_eq!(
        admission.snapshot().expect("still pending").mode,
        OperationAdmissionMode::ExitPending
    );

    drop(settings);
    assert_eq!(
        admission.snapshot().expect("abort finalized").mode,
        OperationAdmissionMode::Open
    );
}

#[test]
fn exit_requests_cooperative_cancellation_for_existing_owned_operations() {
    let admission = OperationAdmission::new();
    let settings = admission
        .begin_device_settings_update()
        .expect("device settings operation");
    let cancellation = settings.cancellation_token();
    let exit = admission.begin_exit().expect("exit pending");

    assert!(!cancellation.is_cancelled());
    assert_eq!(exit.request_owned_cancellation().expect("cancel owners"), 1);
    assert!(cancellation.is_cancelled());

    exit.abort().expect("abort exit");
    drop(settings);
}

#[test]
fn worker_refuses_to_start_until_startup_recovery_clears_running_jobs() {
    let database = TestDatabase::new("startup-running-gate");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 99);
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            account_id,
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:00.000Z",
            None,
        ),
    )
    .expect("queued");
    QueueStore::claim_job(
        &mut connection,
        queued.snapshot.id,
        "2026-08-12T00:00:01.000Z",
    )
    .expect("running");
    drop(connection);

    let backend = Arc::new(FakeSyncBackend::new(SyncSourceV1::History));
    let result = SyncWorkerHandle::start(SyncWorkerConfig::new(
        database.path.clone(),
        OperationAdmission::new(),
        registry(backend),
        Arc::new(BasicSyncWorkerValidator),
    ));

    assert!(matches!(
        result,
        Err(error) if error == "sync_running_job_recovery_required"
    ));
}

#[test]
fn worker_runs_manual_first_and_never_claims_two_jobs() {
    let database = TestDatabase::new("priority");
    let mut connection = database.connect();
    let automatic_account_id = insert_account(&connection, 1);
    let manual_account_id = insert_account(&connection, 11);
    let automatic = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            automatic_account_id,
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:00.000Z",
            Some("2026-08-12T01:00:00.000Z"),
        ),
    )
    .expect("automatic");
    let manual = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            manual_account_id,
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:01.000Z",
            None,
        ),
    )
    .expect("manual");
    drop(connection);

    let admission = OperationAdmission::new();
    let backend = Arc::new(FakeSyncBackend::new(SyncSourceV1::History));
    let worker = SyncWorkerHandle::start(SyncWorkerConfig::new(
        database.path.clone(),
        admission,
        registry(backend.clone()),
        Arc::new(BasicSyncWorkerValidator),
    ))
    .expect("worker");
    worker.wake();

    wait_for_status(&database.path, manual.snapshot.id, SyncJobStatusV1::Running);
    assert_eq!(
        QueueStore::get_stored_job(&database.connect(), automatic.snapshot.id)
            .expect("automatic")
            .snapshot
            .status,
        SyncJobStatusV1::Queued
    );
    assert_eq!(
        worker.running_identity().expect("running").job_id,
        manual.snapshot.id
    );

    finish_job(&database.path, manual.snapshot.id);
    worker.wake();
    wait_for_status(
        &database.path,
        automatic.snapshot.id,
        SyncJobStatusV1::Running,
    );
    finish_job(&database.path, automatic.snapshot.id);
    worker.wake();
    worker.stop();
    worker.join().expect("join");

    assert_eq!(
        backend.calls(),
        vec![
            douyin_history_archive::sync::FakeSyncCall::Start {
                job_id: manual.snapshot.id,
            },
            douyin_history_archive::sync::FakeSyncCall::Start {
                job_id: automatic.snapshot.id,
            },
        ]
    );
}

#[test]
fn worker_retains_automatic_jobs_behind_pause_login_and_source_gates() {
    let database = TestDatabase::new("gates");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 2);
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            account_id,
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:00.000Z",
            Some("2026-08-12T01:00:00.000Z"),
        ),
    )
    .expect("queued");
    drop(connection);

    let admission = OperationAdmission::new();
    admission.set_automatic_paused(true).expect("pause");
    let worker = SyncWorkerHandle::start(SyncWorkerConfig::new(
        database.path.clone(),
        admission.clone(),
        registry(Arc::new(FakeSyncBackend::new(SyncSourceV1::History))),
        Arc::new(BasicSyncWorkerValidator),
    ))
    .expect("worker");
    worker.wake();
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        QueueStore::get_stored_job(&database.connect(), queued.snapshot.id)
            .expect("paused job")
            .snapshot
            .status,
        SyncJobStatusV1::Queued
    );

    admission.set_automatic_paused(false).expect("resume");
    let login = admission
        .begin_browser_operation(BrowserOperationKind::Login, Some(account_id))
        .expect("login");
    worker.wake();
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        QueueStore::get_stored_job(&database.connect(), queued.snapshot.id)
            .expect("login blocked job")
            .snapshot
            .status,
        SyncJobStatusV1::Queued
    );
    drop(login);

    let source_update = admission
        .begin_source_settings_update(queued.snapshot.task_key.clone())
        .expect("source update");
    worker.wake();
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        QueueStore::get_stored_job(&database.connect(), queued.snapshot.id)
            .expect("source blocked job")
            .snapshot
            .status,
        SyncJobStatusV1::Queued
    );
    drop(source_update);
    worker.wake();
    wait_for_status(&database.path, queued.snapshot.id, SyncJobStatusV1::Running);
    finish_job(&database.path, queued.snapshot.id);
    worker.wake();
    worker.stop();
    worker.join().expect("join");
}

#[test]
fn executor_start_failure_is_persisted_and_releases_admission() {
    let database = TestDatabase::new("launch-failure");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 3);
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            account_id,
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:00.000Z",
            None,
        ),
    )
    .expect("queued");
    drop(connection);

    let admission = OperationAdmission::new();
    let worker = SyncWorkerHandle::start(SyncWorkerConfig::new(
        database.path.clone(),
        admission.clone(),
        registry(Arc::new(FakeSyncBackend::with_launch_error(
            SyncSourceV1::History,
            "collector_launch_failed",
        ))),
        Arc::new(BasicSyncWorkerValidator),
    ))
    .expect("worker");
    worker.wake();

    let failed = wait_for_status(&database.path, queued.snapshot.id, SyncJobStatusV1::Failed);
    assert_eq!(
        failed.error_code.as_deref(),
        Some("collector_launch_failed")
    );
    let snapshot = admission.snapshot().expect("admission");
    assert!(snapshot.browser_operation.is_none());
    assert_eq!(snapshot.continuation_count, 0);
    worker.stop();
    worker.join().expect("join");
}

struct BlockingValidator {
    first: AtomicBool,
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

struct SettingsChangeValidator {
    calls: AtomicUsize,
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl SyncWorkerValidator for SettingsChangeValidator {
    fn validate(
        &self,
        _connection: &Connection,
        _job: &StoredSyncJobV1,
    ) -> Result<WorkerJobDecision, String> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.entered.wait();
            self.release.wait();
            Ok(WorkerJobDecision::Run)
        } else {
            Ok(WorkerJobDecision::Cancel {
                error_code: "automatic_settings_changed".to_string(),
            })
        }
    }
}

impl SyncWorkerValidator for BlockingValidator {
    fn validate(
        &self,
        connection: &Connection,
        job: &StoredSyncJobV1,
    ) -> Result<WorkerJobDecision, String> {
        if self.first.swap(false, Ordering::SeqCst) {
            self.entered.wait();
            self.release.wait();
        }
        BasicSyncWorkerValidator.validate(connection, job)
    }
}

#[test]
fn source_gate_wins_the_race_after_selection_but_before_claim() {
    let database = TestDatabase::new("selection-race");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 4);
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            account_id,
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:00.000Z",
            Some("2026-08-12T01:00:00.000Z"),
        ),
    )
    .expect("queued");
    drop(connection);

    let admission = OperationAdmission::new();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let validator = Arc::new(BlockingValidator {
        first: AtomicBool::new(true),
        entered: entered.clone(),
        release: release.clone(),
    });
    let worker = SyncWorkerHandle::start(
        SyncWorkerConfig::new(
            database.path.clone(),
            admission.clone(),
            registry(Arc::new(FakeSyncBackend::new(SyncSourceV1::History))),
            Arc::new(BasicSyncWorkerValidator),
        )
        .with_validator(validator),
    )
    .expect("worker");

    worker.wake();
    entered.wait();
    let source_update = admission
        .begin_source_settings_update(queued.snapshot.task_key.clone())
        .expect("source update");
    release.wait();
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        QueueStore::get_stored_job(&database.connect(), queued.snapshot.id)
            .expect("still queued")
            .snapshot
            .status,
        SyncJobStatusV1::Queued
    );

    drop(source_update);
    worker.wake();
    wait_for_status(&database.path, queued.snapshot.id, SyncJobStatusV1::Running);
    finish_job(&database.path, queued.snapshot.id);
    worker.wake();
    worker.stop();
    worker.join().expect("join");
}

#[test]
fn completed_source_update_forces_final_validation_before_claim() {
    let database = TestDatabase::new("completed-source-update-race");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 8);
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            account_id,
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:00.000Z",
            Some("2026-08-12T01:00:00.000Z"),
        ),
    )
    .expect("queued");
    drop(connection);

    let admission = OperationAdmission::new();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let validator = Arc::new(SettingsChangeValidator {
        calls: AtomicUsize::new(0),
        entered: entered.clone(),
        release: release.clone(),
    });
    let worker = SyncWorkerHandle::start(
        SyncWorkerConfig::new(
            database.path.clone(),
            admission.clone(),
            registry(Arc::new(FakeSyncBackend::new(SyncSourceV1::History))),
            Arc::new(BasicSyncWorkerValidator),
        )
        .with_validator(validator),
    )
    .expect("worker");

    worker.wake();
    entered.wait();
    let source_update = admission
        .begin_source_settings_update(queued.snapshot.task_key.clone())
        .expect("source update");
    drop(source_update);
    release.wait();

    let cancelled = wait_for_status(
        &database.path,
        queued.snapshot.id,
        SyncJobStatusV1::Cancelled,
    );
    assert_eq!(
        cancelled.error_code.as_deref(),
        Some("automatic_settings_changed")
    );
    assert!(worker.running_identity().is_none());
    worker.stop();
    worker.join().expect("join");
}

#[derive(Default)]
struct RecordingTerminalObserver {
    jobs: Mutex<Vec<StoredSyncJobV1>>,
}

impl SyncWorkerTerminalObserver for RecordingTerminalObserver {
    fn observe(&self, job: &StoredSyncJobV1) -> Result<(), String> {
        self.jobs.lock().expect("observer").push(job.clone());
        Ok(())
    }
}

#[derive(Default)]
struct FailingTerminalObserver;

impl SyncWorkerTerminalObserver for FailingTerminalObserver {
    fn observe(&self, _job: &StoredSyncJobV1) -> Result<(), String> {
        Err("notification_infrastructure_failed".to_string())
    }
}

#[test]
fn executor_start_failure_notifies_the_terminal_observer() {
    let database = TestDatabase::new("launch-failure-observer");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 31);
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            account_id,
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:00.000Z",
            Some("2026-08-12T01:00:00.000Z"),
        ),
    )
    .expect("queued");
    drop(connection);

    let observer = Arc::new(RecordingTerminalObserver::default());
    let worker = SyncWorkerHandle::start(
        SyncWorkerConfig::new(
            database.path.clone(),
            OperationAdmission::new(),
            registry(Arc::new(FakeSyncBackend::with_launch_error(
                SyncSourceV1::History,
                "collector_launch_failed",
            ))),
            Arc::new(BasicSyncWorkerValidator),
        )
        .with_terminal_observer(observer.clone()),
    )
    .expect("worker");
    worker.wake();

    wait_for_status(&database.path, queued.snapshot.id, SyncJobStatusV1::Failed);
    let jobs = observer.jobs.lock().expect("observer");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].snapshot.id, queued.snapshot.id);
    assert_eq!(jobs[0].snapshot.status, SyncJobStatusV1::Failed);
    assert_eq!(jobs[0].error_code.as_deref(), Some("collector_launch_failed"));
    drop(jobs);
    worker.stop();
    worker.join().expect("join");
}

#[test]
fn completed_running_job_notifies_the_terminal_observer_before_release() {
    let database = TestDatabase::new("completed-running-observer");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 32);
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            account_id,
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:00.000Z",
            Some("2026-08-12T01:00:00.000Z"),
        ),
    )
    .expect("queued");
    drop(connection);

    let observer = Arc::new(RecordingTerminalObserver::default());
    let worker = SyncWorkerHandle::start(
        SyncWorkerConfig::new(
            database.path.clone(),
            OperationAdmission::new(),
            registry(Arc::new(FakeSyncBackend::new(SyncSourceV1::History))),
            Arc::new(BasicSyncWorkerValidator),
        )
        .with_terminal_observer(observer.clone()),
    )
    .expect("worker");
    worker.wake();
    wait_for_status(&database.path, queued.snapshot.id, SyncJobStatusV1::Running);

    finish_job(&database.path, queued.snapshot.id);
    worker.wake();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if !observer.jobs.lock().expect("observer").is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "terminal observer did not receive the completed running job"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let jobs = observer.jobs.lock().expect("observer");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].snapshot.id, queued.snapshot.id);
    assert_eq!(jobs[0].snapshot.status, SyncJobStatusV1::Success);
    drop(jobs);
    worker.stop();
    worker.join().expect("join");
}

#[test]
fn terminal_observer_failure_does_not_block_running_job_release() {
    let database = TestDatabase::new("terminal-observer-failure-release");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 33);
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            account_id,
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:00.000Z",
            Some("2026-08-12T01:00:00.000Z"),
        ),
    )
    .expect("queued");
    drop(connection);

    let worker = SyncWorkerHandle::start(
        SyncWorkerConfig::new(
            database.path.clone(),
            OperationAdmission::new(),
            registry(Arc::new(FakeSyncBackend::new(SyncSourceV1::History))),
            Arc::new(BasicSyncWorkerValidator),
        )
        .with_terminal_observer(Arc::new(FailingTerminalObserver)),
    )
    .expect("worker");
    worker.wake();
    wait_for_status(&database.path, queued.snapshot.id, SyncJobStatusV1::Running);

    finish_job(&database.path, queued.snapshot.id);
    worker.wake();
    let deadline = Instant::now() + Duration::from_secs(3);
    while worker.running_identity().is_some() {
        assert!(
            Instant::now() < deadline,
            "observer failure kept the terminal job owned by the worker"
        );
        thread::sleep(Duration::from_millis(10));
    }

    worker.stop();
    worker.join().expect("join");
}

#[test]
fn basic_validator_rejects_archived_accounts_and_tampered_payloads() {
    let database = TestDatabase::new("startup-basic-validation");
    let mut connection = database.connect();
    let archived_account_id = insert_account(&connection, 96);
    let archived = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            archived_account_id,
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:00.000Z",
            None,
        ),
    )
    .expect("archived queued");
    connection
        .execute(
            "UPDATE accounts SET status = 'archived' WHERE id = ?1",
            [archived_account_id],
        )
        .expect("archive account");
    let archived = QueueStore::get_stored_job(&connection, archived.snapshot.id).expect("archived");
    assert_eq!(
        BasicSyncWorkerValidator
            .validate(&connection, &archived)
            .expect("validation"),
        WorkerJobDecision::Cancel {
            error_code: "sync_account_not_active".to_string(),
        }
    );

    let active_account_id = insert_account(&connection, 97);
    let valid = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            active_account_id,
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:01.000Z",
            None,
        ),
    )
    .expect("active queued");
    let mut tampered = QueueStore::get_stored_job(&connection, valid.snapshot.id).expect("active");
    tampered.payload_json.push(' ');
    assert_eq!(
        BasicSyncWorkerValidator
            .validate(&connection, &tampered)
            .expect("validation"),
        WorkerJobDecision::Cancel {
            error_code: "invalid_sync_job_payload".to_string(),
        }
    );
}

#[test]
fn runtime_validator_cancels_manual_jobs_when_capability_is_unavailable() {
    let database = TestDatabase::new("runtime-capability");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 98);
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            account_id,
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:00.000Z",
            None,
        ),
    )
    .expect("queued");
    let stored = QueueStore::get_stored_job(&connection, queued.snapshot.id).expect("stored");
    let mut capabilities = CapabilityRegistry::stage_3_3().all().to_vec();
    capabilities
        .iter_mut()
        .find(|capability| capability.source == SyncSourceV1::History)
        .expect("history capability")
        .available = false;
    let validator =
        RuntimeSyncWorkerValidator::new(database.root.clone(), database.root.clone(), capabilities);

    assert_eq!(
        validator
            .validate(&connection, &stored)
            .expect("validation"),
        WorkerJobDecision::Cancel {
            error_code: "sync_source_not_available".to_string(),
        }
    );
}

#[test]
fn runtime_validator_cancels_automatic_when_current_config_hash_changed() {
    let database = TestDatabase::new("runtime-config-change");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 9);
    let mut settings = douyin_history_archive::settings_store::AccountSettingsV2::default();
    settings.history.range = HistoryRangeV1::Bottom;
    settings.history.schedule.enabled = true;
    settings.history.schedule.interval_minutes = 15;
    douyin_history_archive::settings_store::update_account_settings(
        &database.root,
        "sec:sec-9",
        settings.clone(),
    )
    .expect("initial settings");
    let config_sha256 = automatic_config_hash(&AutomaticSyncConfigV1::History {
        schema_version: 1,
        source_instance_key: String::new(),
        schedule: AutomaticScheduleConfigV1 {
            enabled: true,
            interval_minutes: 15,
        },
        range: HistoryRangeV1::Bottom,
        scroll_speed: settings.scroll_speed,
        hide_browser: settings.hide_browser,
        browser_preference: BrowserPreferenceV1::Auto,
    })
    .expect("config hash");
    let mut new_job = queued_job(
        account_id,
        SyncTriggerV1::Automatic,
        "2026-08-12T00:00:00.000Z",
        Some("2026-08-12T01:00:00.000Z"),
    );
    new_job.config_sha256 = Some(config_sha256);
    let queued = QueueStore::enqueue_job(&mut connection, &new_job).expect("queued");

    settings.scroll_speed += 250;
    douyin_history_archive::settings_store::update_account_settings(
        &database.root,
        "sec:sec-9",
        settings,
    )
    .expect("changed settings");
    let stored = QueueStore::get_stored_job(&connection, queued.snapshot.id).expect("stored");
    let validator = RuntimeSyncWorkerValidator::new(
        database.root.clone(),
        database.root.clone(),
        CapabilityRegistry::stage_3_3().all().to_vec(),
    );

    assert_eq!(
        validator
            .validate(&connection, &stored)
            .expect("validation"),
        WorkerJobDecision::Cancel {
            error_code: "automatic_settings_changed".to_string(),
        }
    );
}

#[test]
fn runtime_validator_rejects_a_missing_frozen_browser_profile() {
    let database = TestDatabase::new("runtime-browser-change");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 10);
    let mut settings = douyin_history_archive::settings_store::AccountSettingsV2::default();
    settings.history.range = HistoryRangeV1::Bottom;
    settings.history.schedule.enabled = true;
    settings.history.schedule.interval_minutes = 15;
    douyin_history_archive::settings_store::update_account_settings(
        &database.root,
        "sec:sec-10",
        settings.clone(),
    )
    .expect("settings");
    let config_sha256 = automatic_config_hash(&AutomaticSyncConfigV1::History {
        schema_version: 1,
        source_instance_key: String::new(),
        schedule: AutomaticScheduleConfigV1 {
            enabled: true,
            interval_minutes: 15,
        },
        range: HistoryRangeV1::Bottom,
        scroll_speed: settings.scroll_speed,
        hide_browser: settings.hide_browser,
        browser_preference: BrowserPreferenceV1::Auto,
    })
    .expect("config hash");
    let mut new_job = queued_job(
        account_id,
        SyncTriggerV1::Automatic,
        "2026-08-12T00:00:00.000Z",
        Some("2026-08-12T01:00:00.000Z"),
    );
    new_job.config_sha256 = Some(config_sha256);
    let queued = QueueStore::enqueue_job(&mut connection, &new_job).expect("queued");
    let stored = QueueStore::get_stored_job(&connection, queued.snapshot.id).expect("stored");
    let validator = RuntimeSyncWorkerValidator::new(
        database.root.clone(),
        database.root.clone(),
        CapabilityRegistry::stage_3_3().all().to_vec(),
    );

    assert_eq!(
        validator
            .validate(&connection, &stored)
            .expect("validation"),
        WorkerJobDecision::Cancel {
            error_code: "browser_profile_changed".to_string(),
        }
    );
}

#[test]
fn worker_idle_wait_is_capped_at_thirty_seconds() {
    let database = TestDatabase::new("idle-cap");
    let config = SyncWorkerConfig::new(
        database.path.clone(),
        OperationAdmission::new(),
        Arc::new(ExecutorRegistry::new(Vec::new()).expect("empty registry")),
        Arc::new(BasicSyncWorkerValidator),
    )
    .with_idle_wait(Duration::from_secs(120));
    assert_eq!(config.effective_idle_wait(), Duration::from_secs(30));
}

#[test]
fn maintenance_queue_sweep_cancels_every_queued_job_in_one_call() {
    let database = TestDatabase::new("maintenance-sweep");
    let mut connection = database.connect();
    let first_account = insert_account(&connection, 5);
    let second_account = insert_account(&connection, 6);
    let first = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            first_account,
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:00.000Z",
            None,
        ),
    )
    .expect("first");
    let second = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            second_account,
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:01.000Z",
            Some("2026-08-12T01:00:00.000Z"),
        ),
    )
    .expect("second");

    let cancelled = QueueStore::cancel_all_queued_jobs(
        &mut connection,
        "2026-08-12T02:00:00.000Z",
        "maintenance_cancelled_before_start",
    )
    .expect("sweep");
    assert_eq!(cancelled, vec![first.snapshot.id, second.snapshot.id]);
    for job_id in cancelled {
        let stored = QueueStore::get_stored_job(&connection, job_id).expect("stored");
        assert_eq!(stored.snapshot.status, SyncJobStatusV1::Cancelled);
        assert_eq!(
            stored.error_code.as_deref(),
            Some("maintenance_cancelled_before_start")
        );
    }
}

#[test]
fn failed_pause_write_releases_the_barrier_and_wakes_the_worker() {
    let database = TestDatabase::new("pause-rollback-wake");
    let mut connection = database.connect();
    let account_id = insert_account(&connection, 7);
    let queued = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            account_id,
            SyncTriggerV1::Automatic,
            "2026-08-12T00:00:00.000Z",
            Some("2026-08-12T01:00:00.000Z"),
        ),
    )
    .expect("queued");
    drop(connection);

    let admission = OperationAdmission::new();
    let failed_write = admission
        .begin_automatic_pause_update()
        .expect("pause pending");
    let worker = SyncWorkerHandle::start(SyncWorkerConfig::new(
        database.path.clone(),
        admission,
        registry(Arc::new(FakeSyncBackend::new(SyncSourceV1::History))),
        Arc::new(BasicSyncWorkerValidator),
    ))
    .expect("worker");
    thread::sleep(Duration::from_millis(50));
    drop(failed_write);

    wait_for_status(&database.path, queued.snapshot.id, SyncJobStatusV1::Running);
    finish_job(&database.path, queued.snapshot.id);
    worker.wake();
    worker.stop();
    worker.join().expect("join");
}

#[test]
fn quiescing_worker_finishes_the_running_job_without_dequeueing_the_next_job() {
    let database = TestDatabase::new("quiesce");
    let mut connection = database.connect();
    let first_account = insert_account(&connection, 70);
    let second_account = insert_account(&connection, 71);
    let first = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            first_account,
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:00.000Z",
            None,
        ),
    )
    .expect("first");
    let second = QueueStore::enqueue_job(
        &mut connection,
        &queued_job(
            second_account,
            SyncTriggerV1::Manual,
            "2026-08-12T00:00:01.000Z",
            None,
        ),
    )
    .expect("second");
    drop(connection);

    let worker = SyncWorkerHandle::start(SyncWorkerConfig::new(
        database.path.clone(),
        OperationAdmission::new(),
        registry(Arc::new(FakeSyncBackend::new(SyncSourceV1::History))),
        Arc::new(BasicSyncWorkerValidator),
    ))
    .expect("worker");
    worker.wake();
    wait_for_status(&database.path, first.snapshot.id, SyncJobStatusV1::Running);

    worker.quiesce();
    finish_job(&database.path, first.snapshot.id);
    worker.wake();
    assert!(worker
        .wait_until_stopped(Duration::from_secs(3))
        .expect("bounded worker wait"));
    worker.join().expect("quiesced join");

    assert_eq!(
        QueueStore::get_stored_job(&database.connect(), second.snapshot.id)
            .expect("second stored")
            .snapshot
            .status,
        SyncJobStatusV1::Queued
    );
    assert!(worker.running_identity().is_none());
}
