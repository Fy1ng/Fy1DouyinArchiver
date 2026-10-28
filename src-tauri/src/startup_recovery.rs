use crate::{
    assert_schema_v5,
    bootstrap::write_json_atomically,
    sync::{QueueStore, StoredSyncJobV1, SyncWorkerValidator, WorkerJobDecision},
    AppPaths,
};
use chrono::DateTime;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fmt, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
};
use uuid::Uuid;

pub const SHUTDOWN_INTENT_FILE: &str = "shutdown-intent.json";
pub const SHUTDOWN_INTENT_INVALID_FILE: &str = "shutdown-intent.invalid.json";
pub const STARTUP_RECOVERY_FAULT_FILE: &str = "startup-recovery-fault.json";
const CONTRACT_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShutdownIntentV1 {
    pub schema_version: i64,
    pub application_id: String,
    pub root_id: String,
    pub pid: u32,
    pub process_created_at_100ns: u64,
    pub started_at: String,
    pub cancel_queued: bool,
}

pub fn write_shutdown_intent(
    bootstrap_dir: &Path,
    marker: &ShutdownIntentV1,
) -> Result<(), String> {
    validate_marker(marker).map_err(|error| error.code().to_string())?;
    write_json_atomically(&bootstrap_dir.join(SHUTDOWN_INTENT_FILE), marker)
}

pub fn remove_shutdown_intent(bootstrap_dir: &Path) -> Result<(), String> {
    match fs::remove_file(io_path(&bootstrap_dir.join(SHUTDOWN_INTENT_FILE))) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StartupRecoveryFaultPhaseV1 {
    Detected,
    MarkerArchived,
    RecoveryApplied,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StartupRecoveryDispositionV1 {
    PreserveQueued,
    CancelQueued,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupRecoveryFaultV1 {
    pub schema_version: i64,
    pub phase: StartupRecoveryFaultPhaseV1,
    pub reason: String,
    pub marker_snapshot: Option<ShutdownIntentV1>,
    pub marker_raw_sha256: String,
    pub disposition_applied: Option<StartupRecoveryDispositionV1>,
}

#[derive(Debug, Clone)]
pub struct StartupContinuationPermitV1 {
    _nonce: Uuid,
    progress: Arc<(Mutex<StartupContinuationProgress>, Condvar)>,
}

#[derive(Debug, Default)]
struct StartupContinuationProgress {
    next_stage: usize,
    running: bool,
}

impl StartupContinuationPermitV1 {
    pub fn new() -> Self {
        Self {
            _nonce: Uuid::new_v4(),
            progress: Arc::new((Mutex::new(StartupContinuationProgress::default()), Condvar::new())),
        }
    }

    pub(crate) fn claim_next_stage(&self, stage_count: usize) -> Result<Option<usize>, String> {
        let (state, changed) = &*self.progress;
        let mut state = state
            .lock()
            .map_err(|_| "startup_continuation_poisoned".to_string())?;
        while state.running {
            state = changed
                .wait(state)
                .map_err(|_| "startup_continuation_poisoned".to_string())?;
        }
        if state.next_stage >= stage_count {
            return Ok(None);
        }
        state.running = true;
        Ok(Some(state.next_stage))
    }

    pub(crate) fn finish_stage(&self, index: usize, succeeded: bool) -> Result<(), String> {
        let (state, changed) = &*self.progress;
        let mut state = state
            .lock()
            .map_err(|_| "startup_continuation_poisoned".to_string())?;
        if !state.running || state.next_stage != index {
            return Err("startup_continuation_state_mismatch".to_string());
        }
        if succeeded {
            state.next_stage = state
                .next_stage
                .checked_add(1)
                .ok_or_else(|| "startup_continuation_stage_overflow".to_string())?;
        }
        state.running = false;
        changed.notify_all();
        Ok(())
    }
}

impl Default for StartupContinuationPermitV1 {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct RecoveredQueueStateV1 {
    pub queued_job_ids: Vec<i64>,
    pub continuation_permit: StartupContinuationPermitV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupRecoveryStateV1 {
    NotStarted,
    Blocked,
    Recovering,
    Ready,
}

pub struct StartupRecoveryGate {
    state: Mutex<StartupRecoveryStateV1>,
    changed: Condvar,
}

impl Default for StartupRecoveryGate {
    fn default() -> Self {
        Self {
            state: Mutex::new(StartupRecoveryStateV1::NotStarted),
            changed: Condvar::new(),
        }
    }
}

impl StartupRecoveryGate {
    pub fn state(&self) -> Result<StartupRecoveryStateV1, String> {
        self.state
            .lock()
            .map(|state| *state)
            .map_err(|_| "startup_recovery_state_poisoned".to_string())
    }

    pub fn recover_once(
        &self,
        context: StartupRecoveryContext<'_>,
    ) -> Result<RecoveredQueueStateV1, StartupRecoveryGateError> {
        self.begin(StartupRecoveryStateV1::NotStarted)?;
        let recovered = recover_sync_state_once(context);
        self.finish_initial(&recovered);
        recovered
    }

    pub fn resolve_fault(
        &self,
        context: StartupRecoveryContext<'_>,
        disposition: StartupRecoveryDispositionV1,
    ) -> Result<RecoveredQueueStateV1, StartupRecoveryGateError> {
        self.begin(StartupRecoveryStateV1::Blocked)?;
        let recovered = resolve_startup_recovery_fault(context, disposition);
        self.finish_resolution(&recovered);
        recovered
    }

    fn begin(&self, expected: StartupRecoveryStateV1) -> Result<(), StartupRecoveryGateError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StartupRecoveryGateError::new("startup_recovery_state_poisoned"))?;
        if *state != expected {
            let code = match *state {
                StartupRecoveryStateV1::Blocked => "startup_recovery_blocked",
                StartupRecoveryStateV1::Recovering => "startup_recovery_in_progress",
                StartupRecoveryStateV1::Ready => "startup_recovery_already_ready",
                StartupRecoveryStateV1::NotStarted => "startup_recovery_not_blocked",
            };
            return Err(StartupRecoveryGateError::new(code));
        }
        *state = StartupRecoveryStateV1::Recovering;
        self.changed.notify_all();
        Ok(())
    }

    fn finish_initial(&self, result: &Result<RecoveredQueueStateV1, StartupRecoveryGateError>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        *state = match result {
            Ok(_) => StartupRecoveryStateV1::Ready,
            Err(error) if error.is_blocked() => StartupRecoveryStateV1::Blocked,
            Err(_) => StartupRecoveryStateV1::NotStarted,
        };
        self.changed.notify_all();
    }

    fn finish_resolution(&self, result: &Result<RecoveredQueueStateV1, StartupRecoveryGateError>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        *state = if result.is_ok() {
            StartupRecoveryStateV1::Ready
        } else {
            StartupRecoveryStateV1::Blocked
        };
        self.changed.notify_all();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRecoveryGateError {
    code: String,
}

impl StartupRecoveryGateError {
    fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn is_blocked(&self) -> bool {
        matches!(
            self.code.as_str(),
            "startup_recovery_blocked" | "startup_recovery_marker_hash_mismatch"
        )
    }
}

impl fmt::Display for StartupRecoveryGateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.code)
    }
}

impl Error for StartupRecoveryGateError {}

pub trait ProcessIdentityProbe {
    fn creation_time_100ns(&self, pid: u32) -> Result<Option<u64>, String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProcessIdentityProbe;

#[cfg(target_os = "windows")]
impl ProcessIdentityProbe for SystemProcessIdentityProbe {
    fn creation_time_100ns(&self, pid: u32) -> Result<Option<u64>, String> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME},
            System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };

        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            let error = std::io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                Ok(None)
            } else {
                Err("process_identity_query_failed".to_string())
            };
        }
        let mut created: FILETIME = unsafe { std::mem::zeroed() };
        let mut exited: FILETIME = unsafe { std::mem::zeroed() };
        let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
        let mut user: FILETIME = unsafe { std::mem::zeroed() };
        let queried =
            unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
        unsafe { CloseHandle(process) };
        if queried == 0 {
            return Err("process_identity_query_failed".to_string());
        }
        Ok(Some(
            ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
        ))
    }
}

#[cfg(not(target_os = "windows"))]
impl ProcessIdentityProbe for SystemProcessIdentityProbe {
    fn creation_time_100ns(&self, _pid: u32) -> Result<Option<u64>, String> {
        Ok(None)
    }
}

pub trait RunningJobRecovery {
    fn recover_running_job(
        &mut self,
        job: &StoredSyncJobV1,
        disposition: StartupRecoveryDispositionV1,
    ) -> Result<(), String>;
}

pub struct StartupRecoveryContext<'a> {
    pub paths: &'a AppPaths,
    pub application_id: &'a str,
    pub root_id: &'a str,
    pub recovered_at: &'a str,
    pub process_probe: &'a dyn ProcessIdentityProbe,
    pub running_recovery: &'a mut dyn RunningJobRecovery,
    pub queue_validator: &'a dyn SyncWorkerValidator,
}

pub fn recover_sync_state_once(
    mut context: StartupRecoveryContext<'_>,
) -> Result<RecoveredQueueStateV1, StartupRecoveryGateError> {
    let paths = RecoveryPaths::new(&context.paths.bootstrap_dir);
    if is_file(&paths.fault) {
        let fault = read_fault(&paths.fault)?;
        verify_external_evidence(&paths, &fault)?;
        if fault.phase == StartupRecoveryFaultPhaseV1::RecoveryApplied {
            finish_fault_file_cleanup(&paths)?;
            return ready_state(context.paths);
        }
        return Err(StartupRecoveryGateError::new("startup_recovery_blocked"));
    }

    if is_file(&paths.invalid) {
        if let Err(error) = persist_orphan_invalid_fault(&paths) {
            return Err(block_after_durable_fault(&paths, error));
        }
        return Err(StartupRecoveryGateError::new("startup_recovery_blocked"));
    }

    if is_file(&paths.normal) {
        let evidence = read_evidence(&paths.normal)?;
        let parsed = serde_json::from_slice::<ShutdownIntentV1>(&evidence.raw)
            .ok()
            .filter(|marker| validate_marker(marker).is_ok());
        let reason = parsed.as_ref().and_then(|marker| {
            if marker.application_id != context.application_id {
                Some("shutdown_intent_application_mismatch")
            } else if marker.root_id != context.root_id {
                Some("shutdown_intent_root_mismatch")
            } else {
                None
            }
        });
        if parsed.is_none() || reason.is_some() {
            if let Err(error) = persist_invalid_normal_fault(
                &paths,
                &evidence,
                parsed,
                reason.unwrap_or("shutdown_intent_invalid"),
            ) {
                return Err(block_after_durable_fault(&paths, error));
            }
            return Err(StartupRecoveryGateError::new("startup_recovery_blocked"));
        }
        let marker = parsed
            .ok_or_else(|| StartupRecoveryGateError::new("shutdown_intent_invalid"))?;
        let observed = context
            .process_probe
            .creation_time_100ns(marker.pid)
            .map_err(StartupRecoveryGateError::new)?;
        if observed == Some(marker.process_created_at_100ns) {
            return Err(StartupRecoveryGateError::new("shutdown_in_progress"));
        }
        let recovered =
            apply_disposition(&mut context, StartupRecoveryDispositionV1::CancelQueued)?;
        remove_if_exists(&paths.normal)?;
        return Ok(recovered);
    }

    apply_disposition(&mut context, StartupRecoveryDispositionV1::PreserveQueued)
}

fn block_after_durable_fault(
    paths: &RecoveryPaths,
    error: StartupRecoveryGateError,
) -> StartupRecoveryGateError {
    if is_file(&paths.fault) {
        StartupRecoveryGateError::new("startup_recovery_blocked")
    } else {
        error
    }
}

pub fn resolve_startup_recovery_fault(
    mut context: StartupRecoveryContext<'_>,
    disposition: StartupRecoveryDispositionV1,
) -> Result<RecoveredQueueStateV1, StartupRecoveryGateError> {
    let paths = RecoveryPaths::new(&context.paths.bootstrap_dir);
    let mut fault = read_fault(&paths.fault)?;
    verify_external_evidence(&paths, &fault)?;

    if fault.phase == StartupRecoveryFaultPhaseV1::RecoveryApplied {
        if fault.disposition_applied != Some(disposition) {
            return Err(StartupRecoveryGateError::new(
                "startup_recovery_disposition_mismatch",
            ));
        }
        finish_fault_file_cleanup(&paths)?;
        return ready_state(context.paths);
    }

    if fault.phase == StartupRecoveryFaultPhaseV1::Detected {
        if is_file(&paths.normal) {
            archive_marker(&paths.normal, &paths.invalid)?;
        }
        fault.phase = StartupRecoveryFaultPhaseV1::MarkerArchived;
        write_fault(&paths.fault, &fault)?;
    }

    let recovered = apply_disposition(&mut context, disposition)?;
    fault.phase = StartupRecoveryFaultPhaseV1::RecoveryApplied;
    fault.disposition_applied = Some(disposition);
    write_fault(&paths.fault, &fault)?;
    finish_fault_file_cleanup(&paths)?;
    Ok(recovered)
}

fn apply_disposition(
    context: &mut StartupRecoveryContext<'_>,
    disposition: StartupRecoveryDispositionV1,
) -> Result<RecoveredQueueStateV1, StartupRecoveryGateError> {
    let connection = open_database(&context.paths.database)?;
    let running =
        QueueStore::list_running_jobs(&connection).map_err(StartupRecoveryGateError::new)?;
    drop(connection);
    for job in &running {
        context
            .running_recovery
            .recover_running_job(job, disposition)
            .map_err(StartupRecoveryGateError::new)?;
    }

    let mut connection = open_database(&context.paths.database)?;
    match disposition {
        StartupRecoveryDispositionV1::CancelQueued => {
            QueueStore::cancel_all_queued_jobs(
                &mut connection,
                context.recovered_at,
                "startup_shutdown_cancelled",
            )
            .map_err(StartupRecoveryGateError::new)?;
        }
        StartupRecoveryDispositionV1::PreserveQueued => {
            for queued in
                QueueStore::list_queued_jobs(&connection).map_err(StartupRecoveryGateError::new)?
            {
                match context
                    .queue_validator
                    .validate(&connection, &queued)
                    .map_err(StartupRecoveryGateError::new)?
                {
                    WorkerJobDecision::Run | WorkerJobDecision::RetainQueued => {}
                    WorkerJobDecision::Cancel { error_code } => {
                        QueueStore::cancel_queued_job(
                            &mut connection,
                            queued.snapshot.id,
                            context.recovered_at,
                            &error_code,
                        )
                        .map_err(StartupRecoveryGateError::new)?;
                    }
                }
            }
        }
    }
    QueueStore::assert_no_running_jobs(&connection).map_err(StartupRecoveryGateError::new)?;
    let queued_job_ids = QueueStore::list_queued_jobs(&connection)
        .map_err(StartupRecoveryGateError::new)?
        .into_iter()
        .map(|job| job.snapshot.id)
        .collect();
    Ok(RecoveredQueueStateV1 {
        queued_job_ids,
        continuation_permit: StartupContinuationPermitV1::new(),
    })
}

fn ready_state(paths: &AppPaths) -> Result<RecoveredQueueStateV1, StartupRecoveryGateError> {
    let connection = open_database(&paths.database)?;
    QueueStore::assert_no_running_jobs(&connection).map_err(StartupRecoveryGateError::new)?;
    let queued_job_ids = QueueStore::list_queued_jobs(&connection)
        .map_err(StartupRecoveryGateError::new)?
        .into_iter()
        .map(|job| job.snapshot.id)
        .collect();
    Ok(RecoveredQueueStateV1 {
        queued_job_ids,
        continuation_permit: StartupContinuationPermitV1::new(),
    })
}

fn open_database(path: &Path) -> Result<Connection, StartupRecoveryGateError> {
    let connection =
        Connection::open(path).map_err(|error| StartupRecoveryGateError::new(error.to_string()))?;
    assert_schema_v5(&connection).map_err(StartupRecoveryGateError::new)?;
    Ok(connection)
}

struct RecoveryPaths {
    normal: PathBuf,
    invalid: PathBuf,
    fault: PathBuf,
}

impl RecoveryPaths {
    fn new(directory: &Path) -> Self {
        Self {
            normal: directory.join(SHUTDOWN_INTENT_FILE),
            invalid: directory.join(SHUTDOWN_INTENT_INVALID_FILE),
            fault: directory.join(STARTUP_RECOVERY_FAULT_FILE),
        }
    }
}

struct MarkerEvidence {
    raw: Vec<u8>,
    sha256: String,
}

fn read_evidence(path: &Path) -> Result<MarkerEvidence, StartupRecoveryGateError> {
    let raw = fs::read(io_path(path))
        .map_err(|error| StartupRecoveryGateError::new(error.to_string()))?;
    let sha256 = format!("{:x}", Sha256::digest(&raw));
    Ok(MarkerEvidence { raw, sha256 })
}

fn persist_invalid_normal_fault(
    paths: &RecoveryPaths,
    evidence: &MarkerEvidence,
    marker_snapshot: Option<ShutdownIntentV1>,
    reason: &str,
) -> Result<(), StartupRecoveryGateError> {
    let mut fault = StartupRecoveryFaultV1 {
        schema_version: CONTRACT_SCHEMA_VERSION,
        phase: StartupRecoveryFaultPhaseV1::Detected,
        reason: reason.to_string(),
        marker_snapshot,
        marker_raw_sha256: evidence.sha256.clone(),
        disposition_applied: None,
    };
    write_fault(&paths.fault, &fault)?;
    archive_marker(&paths.normal, &paths.invalid)?;
    fault.phase = StartupRecoveryFaultPhaseV1::MarkerArchived;
    write_fault(&paths.fault, &fault)
}

fn persist_orphan_invalid_fault(paths: &RecoveryPaths) -> Result<(), StartupRecoveryGateError> {
    let evidence = read_evidence(&paths.invalid)?;
    let marker_snapshot = serde_json::from_slice::<ShutdownIntentV1>(&evidence.raw)
        .ok()
        .filter(|marker| validate_marker(marker).is_ok());
    let reason = marker_snapshot
        .as_ref()
        .map(|_| "orphan_shutdown_intent_invalid")
        .unwrap_or("shutdown_intent_invalid");
    let mut fault = StartupRecoveryFaultV1 {
        schema_version: CONTRACT_SCHEMA_VERSION,
        phase: StartupRecoveryFaultPhaseV1::Detected,
        reason: reason.to_string(),
        marker_snapshot,
        marker_raw_sha256: evidence.sha256,
        disposition_applied: None,
    };
    write_fault(&paths.fault, &fault)?;
    fault.phase = StartupRecoveryFaultPhaseV1::MarkerArchived;
    write_fault(&paths.fault, &fault)
}

fn read_fault(path: &Path) -> Result<StartupRecoveryFaultV1, StartupRecoveryGateError> {
    let raw = fs::read(io_path(path))
        .map_err(|error| StartupRecoveryGateError::new(error.to_string()))?;
    let fault = serde_json::from_slice::<StartupRecoveryFaultV1>(&raw)
        .map_err(|_| StartupRecoveryGateError::new("startup_recovery_fault_invalid"))?;
    validate_fault(&fault)?;
    Ok(fault)
}

fn write_fault(
    path: &Path,
    fault: &StartupRecoveryFaultV1,
) -> Result<(), StartupRecoveryGateError> {
    validate_fault(fault)?;
    write_json_atomically(path, fault).map_err(StartupRecoveryGateError::new)
}

fn validate_marker(marker: &ShutdownIntentV1) -> Result<(), StartupRecoveryGateError> {
    if marker.schema_version != CONTRACT_SCHEMA_VERSION
        || marker.application_id.trim().is_empty()
        || !valid_root_id(&marker.root_id)
        || marker.pid == 0
        || marker.process_created_at_100ns == 0
        || !marker.cancel_queued
        || DateTime::parse_from_rfc3339(&marker.started_at).is_err()
    {
        return Err(StartupRecoveryGateError::new("shutdown_intent_invalid"));
    }
    Ok(())
}

fn valid_root_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_fault(fault: &StartupRecoveryFaultV1) -> Result<(), StartupRecoveryGateError> {
    let valid_hash = fault.marker_raw_sha256.len() == 64
        && fault
            .marker_raw_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    let disposition_valid = match fault.phase {
        StartupRecoveryFaultPhaseV1::Detected | StartupRecoveryFaultPhaseV1::MarkerArchived => {
            fault.disposition_applied.is_none()
        }
        StartupRecoveryFaultPhaseV1::RecoveryApplied => fault.disposition_applied.is_some(),
    };
    if fault.schema_version != CONTRACT_SCHEMA_VERSION
        || fault.reason.trim().is_empty()
        || !valid_hash
        || !disposition_valid
        || fault
            .marker_snapshot
            .as_ref()
            .is_some_and(|marker| validate_marker(marker).is_err())
    {
        return Err(StartupRecoveryGateError::new(
            "startup_recovery_fault_invalid",
        ));
    }
    Ok(())
}

fn verify_external_evidence(
    paths: &RecoveryPaths,
    fault: &StartupRecoveryFaultV1,
) -> Result<(), StartupRecoveryGateError> {
    for path in [&paths.normal, &paths.invalid] {
        if is_file(path) && read_evidence(path)?.sha256 != fault.marker_raw_sha256 {
            return Err(StartupRecoveryGateError::new(
                "startup_recovery_marker_hash_mismatch",
            ));
        }
    }
    Ok(())
}

fn finish_fault_file_cleanup(paths: &RecoveryPaths) -> Result<(), StartupRecoveryGateError> {
    remove_if_exists(&paths.normal)?;
    remove_if_exists(&paths.invalid)?;
    remove_if_exists(&paths.fault)
}

fn remove_if_exists(path: &Path) -> Result<(), StartupRecoveryGateError> {
    match fs::remove_file(io_path(path)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(StartupRecoveryGateError::new(error.to_string())),
    }
}

fn is_file(path: &Path) -> bool {
    fs::metadata(io_path(path))
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

fn archive_marker(source: &Path, target: &Path) -> Result<(), StartupRecoveryGateError> {
    if is_file(target) {
        let source_hash = read_evidence(source)?.sha256;
        let target_hash = read_evidence(target)?.sha256;
        if source_hash != target_hash {
            return Err(StartupRecoveryGateError::new(
                "startup_recovery_marker_hash_mismatch",
            ));
        }
        return remove_if_exists(source);
    }
    move_file_write_through(source, target).map_err(StartupRecoveryGateError::new)
}

#[cfg(target_os = "windows")]
fn move_file_write_through(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let source = io_path(source);
    let target = io_path(target);
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn io_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy().replace('/', "\\");
    if value.starts_with(r"\\?\") {
        return PathBuf::from(value);
    }
    if let Some(rest) = value.strip_prefix(r"\\") {
        return PathBuf::from(format!(r"\\?\UNC\{rest}"));
    }
    if path.is_absolute() {
        return PathBuf::from(format!(r"\\?\{value}"));
    }
    path.to_path_buf()
}

#[cfg(not(target_os = "windows"))]
fn io_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(not(target_os = "windows"))]
fn move_file_write_through(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|error| error.to_string())
}
