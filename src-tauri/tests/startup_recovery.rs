use douyin_history_archive::{
    migrate_schema_to_v5_at_startup,
    startup_recovery::{
        recover_sync_state_once, remove_shutdown_intent, resolve_startup_recovery_fault,
        write_shutdown_intent, ProcessIdentityProbe, RunningJobRecovery, ShutdownIntentV1,
        StartupRecoveryContext, StartupRecoveryDispositionV1, StartupRecoveryFaultPhaseV1,
        StartupRecoveryFaultV1, StartupRecoveryGate, StartupRecoveryStateV1,
        SystemProcessIdentityProbe, SHUTDOWN_INTENT_FILE,
        SHUTDOWN_INTENT_INVALID_FILE, STARTUP_RECOVERY_FAULT_FILE,
    },
    sync::{
        NewSyncJobV1, QueueStore, StoredSyncJobV1, SyncJobStatusV1, SyncSourceV1, SyncTriggerV1,
        SyncWorkerValidator, TaskKeyV1, TerminalJobUpdateV1, WorkerJobDecision,
    },
    AppPaths,
};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::SystemTime};

const APPLICATION_ID: &str = "lifearchive.douyin-history-archive";
const ROOT_ID: &str = "0123456789abcdef0123456789abcdef";
const NOW: &str = "2026-08-12T08:00:00.000Z";

struct TestRoot {
    root: PathBuf,
    paths: AppPaths,
}

impl TestRoot {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "emily-startup-recovery-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp root");
        let paths = AppPaths::from_roots(
            root.join("bootstrap"),
            root.join("data"),
            root.join("resources"),
        );
        fs::create_dir_all(&paths.bootstrap_dir).expect("bootstrap directory");
        fs::create_dir_all(&paths.temp).expect("temp directory");
        let mut connection = Connection::open(&paths.database).expect("database");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        Self { root, paths }
    }

    fn connect(&self) -> Connection {
        Connection::open(&self.paths.database).expect("database")
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct FixedProbe(Option<u64>);

impl ProcessIdentityProbe for FixedProbe {
    fn creation_time_100ns(&self, _pid: u32) -> Result<Option<u64>, String> {
        Ok(self.0)
    }
}

struct AllowQueued;

impl SyncWorkerValidator for AllowQueued {
    fn validate(
        &self,
        _connection: &Connection,
        _job: &StoredSyncJobV1,
    ) -> Result<WorkerJobDecision, String> {
        Ok(WorkerJobDecision::Run)
    }
}

struct SelectiveValidator {
    rejected_job_id: i64,
}

impl SyncWorkerValidator for SelectiveValidator {
    fn validate(
        &self,
        _connection: &Connection,
        job: &StoredSyncJobV1,
    ) -> Result<WorkerJobDecision, String> {
        if job.snapshot.id == self.rejected_job_id {
            Ok(WorkerJobDecision::Cancel {
                error_code: "browser_profile_changed".to_string(),
            })
        } else {
            Ok(WorkerJobDecision::Run)
        }
    }
}

struct DatabaseRunningRecovery {
    database: PathBuf,
    calls: Vec<(i64, StartupRecoveryDispositionV1)>,
}

impl DatabaseRunningRecovery {
    fn new(database: PathBuf) -> Self {
        Self {
            database,
            calls: Vec::new(),
        }
    }
}

impl RunningJobRecovery for DatabaseRunningRecovery {
    fn recover_running_job(
        &mut self,
        job: &StoredSyncJobV1,
        disposition: StartupRecoveryDispositionV1,
    ) -> Result<(), String> {
        self.calls.push((job.snapshot.id, disposition));
        let status = match disposition {
            StartupRecoveryDispositionV1::PreserveQueued => SyncJobStatusV1::Failed,
            StartupRecoveryDispositionV1::CancelQueued => SyncJobStatusV1::Cancelled,
        };
        let mut connection = Connection::open(&self.database).map_err(|error| error.to_string())?;
        QueueStore::finish_running_job(
            &mut connection,
            job.snapshot.id,
            &TerminalJobUpdateV1 {
                status,
                finished_at: NOW.to_string(),
                inserted_videos: 0,
                inserted_records: 0,
                error_code: Some("collector_interrupted".to_string()),
                error_message: None,
            },
        )?;
        Ok(())
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

fn enqueue_job(connection: &mut Connection, account_id: i64, marker: char) -> StoredSyncJobV1 {
    QueueStore::enqueue_job(
        connection,
        &NewSyncJobV1 {
            task_key: TaskKeyV1 {
                account_id,
                source: SyncSourceV1::History,
                source_instance_key: String::new(),
            },
            trigger: SyncTriggerV1::Manual,
            queued_at: "2026-08-12T07:00:00.000Z".to_string(),
            scheduled_for: None,
            settings_revision: 1,
            config_sha256: None,
            payload_json: "{}".to_string(),
            payload_sha256: marker.to_string().repeat(64),
            equivalence_sha256: marker.to_string().repeat(64),
        },
    )
    .expect("queue job")
}

fn marker(root_id: &str, creation_time: u64) -> ShutdownIntentV1 {
    ShutdownIntentV1 {
        schema_version: 1,
        application_id: APPLICATION_ID.to_string(),
        root_id: root_id.to_string(),
        pid: 42,
        process_created_at_100ns: creation_time,
        started_at: "2026-08-12T07:59:00.000Z".to_string(),
        cancel_queued: true,
    }
}

#[test]
fn shutdown_intent_write_and_removal_are_atomic_and_idempotent() {
    let test = TestRoot::new("marker-lifecycle");
    let expected = marker(ROOT_ID, 55);

    write_shutdown_intent(&test.paths.bootstrap_dir, &expected).expect("write marker");
    let raw = fs::read(test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE)).expect("read marker");
    assert_eq!(
        serde_json::from_slice::<ShutdownIntentV1>(&raw).expect("parse marker"),
        expected
    );

    remove_shutdown_intent(&test.paths.bootstrap_dir).expect("remove marker");
    remove_shutdown_intent(&test.paths.bootstrap_dir).expect("remove missing marker");
    assert!(!test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE).exists());
}

#[test]
fn shutdown_marker_contract_rejects_unknown_fields() {
    let raw = serde_json::json!({
        "schemaVersion": 1,
        "applicationId": APPLICATION_ID,
        "rootId": ROOT_ID,
        "pid": 42,
        "processCreatedAt100ns": 55,
        "startedAt": "2026-08-12T07:59:00.000Z",
        "cancelQueued": true,
        "unexpected": true
    });

    assert!(serde_json::from_value::<ShutdownIntentV1>(raw).is_err());
}

#[cfg(target_os = "windows")]
#[test]
fn system_probe_reads_the_current_process_creation_time() {
    let observed = SystemProcessIdentityProbe
        .creation_time_100ns(std::process::id())
        .expect("probe")
        .expect("current process exists");

    assert!(observed > 0);
}

fn write_marker(path: &std::path::Path, value: &ShutdownIntentV1) -> Vec<u8> {
    let raw = serde_json::to_vec_pretty(value).expect("marker json");
    fs::write(path, &raw).expect("write marker");
    raw
}

fn context<'a>(
    test: &'a TestRoot,
    probe: &'a dyn ProcessIdentityProbe,
    running: &'a mut dyn RunningJobRecovery,
    validator: &'a dyn SyncWorkerValidator,
) -> StartupRecoveryContext<'a> {
    StartupRecoveryContext {
        paths: &test.paths,
        application_id: APPLICATION_ID,
        root_id: ROOT_ID,
        recovered_at: NOW,
        process_probe: probe,
        running_recovery: running,
        queue_validator: validator,
    }
}

#[test]
fn wrong_root_is_faulted_before_marker_is_archived() {
    let test = TestRoot::new("wrong-root");
    let marker_path = test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE);
    let raw = write_marker(
        &marker_path,
        &marker("fedcba9876543210fedcba9876543210", 10),
    );
    let probe = FixedProbe(None);
    let mut running = DatabaseRunningRecovery::new(test.paths.database.clone());

    let error = recover_sync_state_once(context(&test, &probe, &mut running, &AllowQueued))
        .expect_err("identity mismatch must block");

    assert_eq!(error.code(), "startup_recovery_blocked");
    assert!(!marker_path.exists());
    assert!(test
        .paths
        .bootstrap_dir
        .join(SHUTDOWN_INTENT_INVALID_FILE)
        .is_file());
    let fault: StartupRecoveryFaultV1 = serde_json::from_slice(
        &fs::read(test.paths.bootstrap_dir.join(STARTUP_RECOVERY_FAULT_FILE)).expect("fault"),
    )
    .expect("strict fault");
    assert_eq!(fault.phase, StartupRecoveryFaultPhaseV1::MarkerArchived);
    assert_eq!(fault.reason, "shutdown_intent_root_mismatch");
    assert_eq!(
        fault.marker_raw_sha256,
        format!("{:x}", Sha256::digest(&raw))
    );
    assert_eq!(
        fault.marker_snapshot,
        Some(marker("fedcba9876543210fedcba9876543210", 10))
    );
}

#[test]
fn malformed_marker_is_archived_with_a_hash_but_without_a_snapshot() {
    let test = TestRoot::new("malformed");
    let raw = b"{not-json";
    fs::write(test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE), raw).expect("marker");
    let probe = FixedProbe(None);
    let mut running = DatabaseRunningRecovery::new(test.paths.database.clone());

    recover_sync_state_once(context(&test, &probe, &mut running, &AllowQueued))
        .expect_err("malformed evidence blocks");

    let fault: StartupRecoveryFaultV1 = serde_json::from_slice(
        &fs::read(test.paths.bootstrap_dir.join(STARTUP_RECOVERY_FAULT_FILE)).expect("fault"),
    )
    .expect("fault contract");
    assert_eq!(fault.reason, "shutdown_intent_invalid");
    assert_eq!(fault.marker_snapshot, None);
    assert_eq!(
        fault.marker_raw_sha256,
        format!("{:x}", Sha256::digest(raw))
    );
}

#[test]
fn structurally_invalid_root_id_is_not_preserved_as_a_marker_snapshot() {
    let test = TestRoot::new("invalid-root-contract");
    write_marker(
        &test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE),
        &marker("not-a-root-id", 10),
    );
    let probe = FixedProbe(None);
    let mut running = DatabaseRunningRecovery::new(test.paths.database.clone());

    recover_sync_state_once(context(&test, &probe, &mut running, &AllowQueued))
        .expect_err("invalid root contract blocks");

    let fault: StartupRecoveryFaultV1 = serde_json::from_slice(
        &fs::read(
            test.paths
                .bootstrap_dir
                .join(STARTUP_RECOVERY_FAULT_FILE),
        )
        .expect("fault"),
    )
    .expect("fault contract");
    assert_eq!(fault.reason, "shutdown_intent_invalid");
    assert_eq!(fault.marker_snapshot, None);
}

#[test]
fn detected_phase_replays_marker_archival_before_recovery() {
    let test = TestRoot::new("detected-replay");
    let marker = marker("fedcba9876543210fedcba9876543210", 10);
    let raw = write_marker(
        &test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE),
        &marker,
    );
    let fault = StartupRecoveryFaultV1 {
        schema_version: 1,
        phase: StartupRecoveryFaultPhaseV1::Detected,
        reason: "shutdown_intent_root_mismatch".to_string(),
        marker_snapshot: Some(marker),
        marker_raw_sha256: format!("{:x}", Sha256::digest(&raw)),
        disposition_applied: None,
    };
    fs::write(
        test.paths.bootstrap_dir.join(STARTUP_RECOVERY_FAULT_FILE),
        serde_json::to_vec_pretty(&fault).expect("fault json"),
    )
    .expect("fault");
    let mut connection = test.connect();
    let account_id = insert_account(&connection, 6);
    let queued = enqueue_job(&mut connection, account_id, 'c');
    let probe = FixedProbe(None);
    let mut recovery = DatabaseRunningRecovery::new(test.paths.database.clone());

    resolve_startup_recovery_fault(
        context(&test, &probe, &mut recovery, &AllowQueued),
        StartupRecoveryDispositionV1::CancelQueued,
    )
    .expect("replay detected phase");

    assert_eq!(
        QueueStore::get_stored_job(&test.connect(), queued.snapshot.id)
            .expect("queued")
            .snapshot
            .status,
        SyncJobStatusV1::Cancelled
    );
    assert!(!test
        .paths
        .bootstrap_dir
        .join(STARTUP_RECOVERY_FAULT_FILE)
        .exists());
    assert!(!test
        .paths
        .bootstrap_dir
        .join(SHUTDOWN_INTENT_INVALID_FILE)
        .exists());
}

#[test]
fn matching_live_pid_stops_before_database_recovery() {
    let test = TestRoot::new("live-pid");
    let mut connection = test.connect();
    let account_id = insert_account(&connection, 1);
    let queued = enqueue_job(&mut connection, account_id, 'a');
    let running_job = QueueStore::claim_job(
        &mut connection,
        queued.snapshot.id,
        "2026-08-12T07:30:00.000Z",
    )
    .expect("running");
    write_marker(
        &test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE),
        &marker(ROOT_ID, 55),
    );
    let probe = FixedProbe(Some(55));
    let mut running = DatabaseRunningRecovery::new(test.paths.database.clone());

    let error = recover_sync_state_once(context(&test, &probe, &mut running, &AllowQueued))
        .expect_err("old process is still alive");

    assert_eq!(error.code(), "shutdown_in_progress");
    assert!(running.calls.is_empty());
    assert_eq!(
        QueueStore::get_stored_job(&test.connect(), running_job.snapshot.id)
            .expect("job")
            .snapshot
            .status,
        SyncJobStatusV1::Running
    );
}

#[test]
fn reused_or_missing_pid_applies_cancel_queued_recovery() {
    for (label, observed) in [("missing", None), ("reused", Some(99))] {
        let test = TestRoot::new(label);
        let mut connection = test.connect();
        let account_id = insert_account(&connection, 2);
        let running = enqueue_job(&mut connection, account_id, 'b');
        let running = QueueStore::claim_job(
            &mut connection,
            running.snapshot.id,
            "2026-08-12T07:30:00.000Z",
        )
        .expect("running");
        let queued_account_id = insert_account(&connection, 20);
        let queued = enqueue_job(&mut connection, queued_account_id, 'c');
        write_marker(
            &test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE),
            &marker(ROOT_ID, 55),
        );
        let probe = FixedProbe(observed);
        let mut recovery = DatabaseRunningRecovery::new(test.paths.database.clone());

        let recovered =
            recover_sync_state_once(context(&test, &probe, &mut recovery, &AllowQueued))
                .expect("cancel recovery");

        assert!(recovered.queued_job_ids.is_empty());
        assert_eq!(
            recovery.calls,
            vec![(
                running.snapshot.id,
                StartupRecoveryDispositionV1::CancelQueued
            )]
        );
        let connection = test.connect();
        assert_eq!(
            QueueStore::get_stored_job(&connection, running.snapshot.id)
                .expect("running job")
                .snapshot
                .status,
            SyncJobStatusV1::Cancelled
        );
        assert_eq!(
            QueueStore::get_stored_job(&connection, queued.snapshot.id)
                .expect("queued job")
                .snapshot
                .status,
            SyncJobStatusV1::Cancelled
        );
        assert!(
            QueueStore::latest_terminal_anchor(&connection, &running.snapshot.task_key)
                .expect("running anchor")
                .is_some()
        );
        assert!(
            QueueStore::latest_terminal_anchor(&connection, &queued.snapshot.task_key)
                .expect("queued anchor")
                .is_none()
        );
        assert!(!test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE).exists());
        QueueStore::assert_no_running_jobs(&connection).expect("zero running");
    }
}

#[test]
fn ordinary_crash_preserves_only_valid_queued_jobs_and_fails_running() {
    let test = TestRoot::new("ordinary-crash");
    let mut connection = test.connect();
    let account_id = insert_account(&connection, 3);
    let running = enqueue_job(&mut connection, account_id, 'd');
    let running = QueueStore::claim_job(
        &mut connection,
        running.snapshot.id,
        "2026-08-12T07:30:00.000Z",
    )
    .expect("running");
    let retained_account_id = insert_account(&connection, 30);
    let rejected_account_id = insert_account(&connection, 31);
    let retained = enqueue_job(&mut connection, retained_account_id, 'e');
    let rejected = enqueue_job(&mut connection, rejected_account_id, 'f');
    let probe = FixedProbe(None);
    let mut recovery = DatabaseRunningRecovery::new(test.paths.database.clone());
    let validator = SelectiveValidator {
        rejected_job_id: rejected.snapshot.id,
    };

    let recovered = recover_sync_state_once(context(&test, &probe, &mut recovery, &validator))
        .expect("ordinary recovery");

    assert_eq!(recovered.queued_job_ids, vec![retained.snapshot.id]);
    assert_eq!(
        recovery.calls,
        vec![(
            running.snapshot.id,
            StartupRecoveryDispositionV1::PreserveQueued
        )]
    );
    let connection = test.connect();
    assert_eq!(
        QueueStore::get_stored_job(&connection, running.snapshot.id)
            .expect("running")
            .snapshot
            .status,
        SyncJobStatusV1::Failed
    );
    let rejected = QueueStore::get_stored_job(&connection, rejected.snapshot.id).expect("rejected");
    assert_eq!(rejected.snapshot.status, SyncJobStatusV1::Cancelled);
    assert_eq!(
        rejected.error_code.as_deref(),
        Some("browser_profile_changed")
    );
    QueueStore::assert_no_running_jobs(&connection).expect("zero running");
}

#[test]
fn orphan_invalid_marker_rebuilds_a_blocking_fault_and_fault_only_resolution_is_idempotent() {
    let test = TestRoot::new("orphan-invalid");
    let invalid_path = test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_INVALID_FILE);
    write_marker(
        &invalid_path,
        &marker("fedcba9876543210fedcba9876543210", 10),
    );
    let mut connection = test.connect();
    let account_id = insert_account(&connection, 4);
    let queued = enqueue_job(&mut connection, account_id, 'a');
    let probe = FixedProbe(None);
    let mut recovery = DatabaseRunningRecovery::new(test.paths.database.clone());

    let error = recover_sync_state_once(context(&test, &probe, &mut recovery, &AllowQueued))
        .expect_err("orphan evidence must block");
    assert_eq!(error.code(), "startup_recovery_blocked");
    fs::remove_file(&invalid_path).expect("simulate fault-only crash point");

    let recovered = resolve_startup_recovery_fault(
        context(&test, &probe, &mut recovery, &AllowQueued),
        StartupRecoveryDispositionV1::PreserveQueued,
    )
    .expect("fault-only recovery");

    assert_eq!(recovered.queued_job_ids, vec![queued.snapshot.id]);
    assert!(!test
        .paths
        .bootstrap_dir
        .join(STARTUP_RECOVERY_FAULT_FILE)
        .exists());
}

#[test]
fn fault_resolution_rejects_changed_marker_bytes_without_touching_jobs() {
    let test = TestRoot::new("hash-mismatch");
    let marker_path = test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE);
    write_marker(
        &marker_path,
        &marker("fedcba9876543210fedcba9876543210", 10),
    );
    let mut connection = test.connect();
    let account_id = insert_account(&connection, 5);
    let queued = enqueue_job(&mut connection, account_id, 'b');
    let probe = FixedProbe(None);
    let mut recovery = DatabaseRunningRecovery::new(test.paths.database.clone());
    recover_sync_state_once(context(&test, &probe, &mut recovery, &AllowQueued))
        .expect_err("fault expected");
    let invalid_path = test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_INVALID_FILE);
    fs::write(&invalid_path, b"different evidence").expect("tamper evidence");

    let error = resolve_startup_recovery_fault(
        context(&test, &probe, &mut recovery, &AllowQueued),
        StartupRecoveryDispositionV1::CancelQueued,
    )
    .expect_err("hash mismatch must fail closed");

    assert_eq!(error.code(), "startup_recovery_marker_hash_mismatch");
    assert_eq!(
        QueueStore::get_stored_job(&test.connect(), queued.snapshot.id)
            .expect("queued")
            .snapshot
            .status,
        SyncJobStatusV1::Queued
    );
}

#[test]
fn recovery_applied_phase_only_finishes_file_cleanup() {
    let test = TestRoot::new("recovery-applied");
    let fault = StartupRecoveryFaultV1 {
        schema_version: 1,
        phase: StartupRecoveryFaultPhaseV1::RecoveryApplied,
        reason: "shutdown_intent_root_mismatch".to_string(),
        marker_snapshot: Some(marker("fedcba9876543210fedcba9876543210", 10)),
        marker_raw_sha256: "a".repeat(64),
        disposition_applied: Some(StartupRecoveryDispositionV1::CancelQueued),
    };
    fs::write(
        test.paths.bootstrap_dir.join(STARTUP_RECOVERY_FAULT_FILE),
        serde_json::to_vec_pretty(&fault).expect("fault json"),
    )
    .expect("fault");
    let probe = FixedProbe(None);
    let mut recovery = DatabaseRunningRecovery::new(test.paths.database.clone());

    let recovered = recover_sync_state_once(context(&test, &probe, &mut recovery, &AllowQueued))
        .expect("cleanup replay");

    assert!(recovered.queued_job_ids.is_empty());
    assert!(recovery.calls.is_empty());
    assert!(!test
        .paths
        .bootstrap_dir
        .join(STARTUP_RECOVERY_FAULT_FILE)
        .exists());
}

#[test]
fn startup_gate_can_resolve_a_durable_fault_without_restarting_the_process() {
    let test = TestRoot::new("gate");
    write_marker(
        &test.paths.bootstrap_dir.join(SHUTDOWN_INTENT_FILE),
        &marker("fedcba9876543210fedcba9876543210", 10),
    );
    let probe = FixedProbe(None);
    let mut recovery = DatabaseRunningRecovery::new(test.paths.database.clone());
    let gate = StartupRecoveryGate::default();
    assert_eq!(
        gate.state().expect("state"),
        StartupRecoveryStateV1::NotStarted
    );

    gate.recover_once(context(&test, &probe, &mut recovery, &AllowQueued))
        .expect_err("fault blocks startup");
    assert_eq!(
        gate.state().expect("state"),
        StartupRecoveryStateV1::Blocked
    );

    gate.resolve_fault(
        context(&test, &probe, &mut recovery, &AllowQueued),
        StartupRecoveryDispositionV1::PreserveQueued,
    )
    .expect("same-process resolution");
    assert_eq!(gate.state().expect("state"), StartupRecoveryStateV1::Ready);
}
