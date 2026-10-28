use super::{
    automatic_config_hash, current_automatic_source_instance, freeze_task_payload, AdmittedSync,
    BrowserIdV1, ExecutorRegistry, OperationAdmission, QueueStore, QueuedSyncContext,
    RunningSyncIdentity, SourceCapabilityViewV1, StoredSyncJobV1, SyncJobStatusV1,
    SyncTaskPayloadV1, SyncTriggerV1, TerminalJobUpdateV1,
};
use crate::{
    accounts::portable_account_key_for_id,
    collector::browser_state::{
        canonical_windows_path, read_browser_selection, read_login_validation, BrowserId,
    },
    device_settings::{BrowserPreferenceV1, DeviceSettingsStore},
    settings_store::{account_settings_for, load_portable_settings},
};
use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

const MAX_IDLE_WAIT: Duration = Duration::from_secs(30);
const MIN_IDLE_WAIT: Duration = Duration::from_millis(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerJobDecision {
    Run,
    RetainQueued,
    Cancel { error_code: String },
}

pub trait SyncWorkerValidator: Send + Sync {
    fn validate(
        &self,
        connection: &Connection,
        job: &StoredSyncJobV1,
    ) -> Result<WorkerJobDecision, String>;
}

pub trait SyncWorkerTerminalObserver: Send + Sync {
    fn observe(&self, job: &StoredSyncJobV1) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, Default)]
struct NoopTerminalObserver;

impl SyncWorkerTerminalObserver for NoopTerminalObserver {
    fn observe(&self, _job: &StoredSyncJobV1) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BasicSyncWorkerValidator;

impl SyncWorkerValidator for BasicSyncWorkerValidator {
    fn validate(
        &self,
        connection: &Connection,
        job: &StoredSyncJobV1,
    ) -> Result<WorkerJobDecision, String> {
        let account_status = connection
            .query_row(
                "SELECT status FROM accounts WHERE id = ?1",
                [job.snapshot.task_key.account_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if account_status.as_deref() != Some("active") {
            return Ok(WorkerJobDecision::Cancel {
                error_code: "sync_account_not_active".to_string(),
            });
        }

        let payload = match serde_json::from_str::<SyncTaskPayloadV1>(&job.payload_json) {
            Ok(payload) => payload,
            Err(_) => {
                return Ok(WorkerJobDecision::Cancel {
                    error_code: "invalid_sync_job_payload".to_string(),
                })
            }
        };
        let frozen = match freeze_task_payload(payload.clone()) {
            Ok(frozen) => frozen,
            Err(_) => {
                return Ok(WorkerJobDecision::Cancel {
                    error_code: "invalid_sync_job_payload".to_string(),
                })
            }
        };
        if frozen.payload_json != job.payload_json
            || frozen.payload_sha256 != job.snapshot.task_payload_sha256
            || frozen.equivalence_sha256 != job.equivalence_sha256
            || payload.source() != job.snapshot.task_key.source
            || payload.source_instance_key() != job.snapshot.task_key.source_instance_key
        {
            return Ok(WorkerJobDecision::Cancel {
                error_code: "invalid_sync_job_payload".to_string(),
            });
        }
        if job.snapshot.trigger == SyncTriggerV1::Automatic
            && (job.config_sha256.is_none() || job.scheduled_for.is_none())
        {
            return Ok(WorkerJobDecision::Cancel {
                error_code: "invalid_automatic_sync_job".to_string(),
            });
        }
        Ok(WorkerJobDecision::Run)
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeSyncWorkerValidator {
    data_root: PathBuf,
    bootstrap_dir: PathBuf,
    capabilities: Vec<SourceCapabilityViewV1>,
}

impl RuntimeSyncWorkerValidator {
    pub fn new(
        data_root: PathBuf,
        bootstrap_dir: PathBuf,
        capabilities: Vec<SourceCapabilityViewV1>,
    ) -> Self {
        Self {
            data_root,
            bootstrap_dir,
            capabilities,
        }
    }
}

impl SyncWorkerValidator for RuntimeSyncWorkerValidator {
    fn validate(
        &self,
        connection: &Connection,
        job: &StoredSyncJobV1,
    ) -> Result<WorkerJobDecision, String> {
        let basic = BasicSyncWorkerValidator.validate(connection, job)?;
        if basic != WorkerJobDecision::Run {
            return Ok(basic);
        }
        if !self.capabilities.iter().any(|capability| {
            capability.source == job.snapshot.task_key.source && capability.available
        }) {
            return Ok(WorkerJobDecision::Cancel {
                error_code: "sync_source_not_available".to_string(),
            });
        }
        let payload = serde_json::from_str::<SyncTaskPayloadV1>(&job.payload_json)
            .map_err(|_| "invalid_sync_job_payload".to_string())?;
        let portable_account_key =
            portable_account_key_for_id(connection, job.snapshot.task_key.account_id)?;
        let profile_key = connection
            .query_row(
                "SELECT profile_key FROM accounts WHERE id = ?1",
                [job.snapshot.task_key.account_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "sync_account_not_active".to_string())?;
        let portable = load_portable_settings(&self.data_root)?;
        let account_settings = account_settings_for(&portable, Some(&portable_account_key))?;
        let device = DeviceSettingsStore::open(self.bootstrap_dir.clone())?.load()?;
        let browser_preference = device
            .accounts
            .get(&profile_key)
            .cloned()
            .unwrap_or_default()
            .browser_preference;

        if job.snapshot.trigger == SyncTriggerV1::Automatic {
            let current = current_automatic_source_instance(
                connection,
                &job.snapshot.task_key,
                &account_settings,
                browser_preference,
                &self.capabilities,
            )?;
            let still_valid = current.is_some_and(|instance| {
                instance.schedule.enabled
                    && automatic_config_hash(&instance.automatic_config)
                        .ok()
                        .as_deref()
                        == job.config_sha256.as_deref()
            });
            if !still_valid {
                return Ok(WorkerJobDecision::Cancel {
                    error_code: "automatic_settings_changed".to_string(),
                });
            }
        }

        if !browser_identity_matches_current(
            &self.data_root,
            &profile_key,
            browser_preference,
            payload.browser(),
        ) {
            return Ok(WorkerJobDecision::Cancel {
                error_code: "browser_profile_changed".to_string(),
            });
        }
        Ok(WorkerJobDecision::Run)
    }
}

fn browser_identity_matches_current(
    data_root: &std::path::Path,
    profile_key: &str,
    browser_preference: BrowserPreferenceV1,
    expected: &super::BrowserExecutionIdentityV1,
) -> bool {
    if expected.profile_key != profile_key || expected.preference != browser_preference {
        return false;
    }
    let account_root = data_root.join("profiles").join(profile_key);
    let Ok(Some(selection)) = read_browser_selection(&account_root) else {
        return false;
    };
    let Ok(Some(_validation)) = read_login_validation(&account_root, &selection) else {
        return false;
    };
    let resolved_browser = match selection.browser_id {
        BrowserId::Edge => BrowserIdV1::Edge,
        BrowserId::Chrome => BrowserIdV1::Chrome,
    };
    let Some(profile_generation) = selection.profile_generation.as_deref() else {
        return false;
    };
    let executable_path_sha256 = browser_executable_path_sha256(&selection.executable_path);
    expected.resolved_browser == resolved_browser
        && expected.profile_generation == profile_generation
        && expected.executable_path_sha256 == executable_path_sha256
}

pub fn browser_executable_path_sha256(executable_path: &str) -> String {
    let canonical = canonical_windows_path(executable_path);
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}

pub struct SyncWorkerConfig {
    database_path: PathBuf,
    admission: OperationAdmission,
    registry: Arc<ExecutorRegistry>,
    validator: Arc<dyn SyncWorkerValidator>,
    terminal_observer: Arc<dyn SyncWorkerTerminalObserver>,
    idle_wait: Duration,
}

impl SyncWorkerConfig {
    pub fn new(
        database_path: PathBuf,
        admission: OperationAdmission,
        registry: Arc<ExecutorRegistry>,
        validator: Arc<dyn SyncWorkerValidator>,
    ) -> Self {
        Self {
            database_path,
            admission,
            registry,
            validator,
            terminal_observer: Arc::new(NoopTerminalObserver),
            idle_wait: MAX_IDLE_WAIT,
        }
    }

    pub fn with_validator(mut self, validator: Arc<dyn SyncWorkerValidator>) -> Self {
        self.validator = validator;
        self
    }

    pub fn with_idle_wait(mut self, idle_wait: Duration) -> Self {
        self.idle_wait = idle_wait;
        self
    }

    pub fn with_terminal_observer(
        mut self,
        observer: Arc<dyn SyncWorkerTerminalObserver>,
    ) -> Self {
        self.terminal_observer = observer;
        self
    }

    pub fn effective_idle_wait(&self) -> Duration {
        self.idle_wait.clamp(MIN_IDLE_WAIT, MAX_IDLE_WAIT)
    }
}

pub struct SyncWorkerHandle {
    signal: Arc<WorkerSignal>,
    running: Arc<Mutex<Option<WorkerRunning>>>,
    join: Mutex<Option<JoinHandle<Result<(), String>>>>,
}

struct WorkerSignal {
    state: Mutex<WorkerSignalState>,
    changed: Condvar,
}

#[derive(Default)]
struct WorkerSignalState {
    wake_generation: u64,
    stop: bool,
    quiesce: bool,
}

struct WorkerRunning {
    identity: RunningSyncIdentity,
    _admitted: AdmittedSync,
}

impl SyncWorkerHandle {
    pub fn start(config: SyncWorkerConfig) -> Result<Self, String> {
        let connection =
            Connection::open(&config.database_path).map_err(|error| error.to_string())?;
        QueueStore::assert_no_running_jobs(&connection)?;
        drop(connection);
        let signal = Arc::new(WorkerSignal {
            state: Mutex::new(WorkerSignalState::default()),
            changed: Condvar::new(),
        });
        let weak_signal = Arc::downgrade(&signal);
        config.admission.set_worker_waker(Arc::new(move || {
            let Some(signal) = weak_signal.upgrade() else {
                return;
            };
            let Ok(mut state) = signal.state.lock() else {
                return;
            };
            state.wake_generation = state.wake_generation.wrapping_add(1);
            signal.changed.notify_all();
        }))?;
        let running = Arc::new(Mutex::new(None));
        let worker_signal = Arc::clone(&signal);
        let worker_running = Arc::clone(&running);
        let join = thread::Builder::new()
            .name("emily-sync-worker".to_string())
            .spawn(move || worker_loop(config, worker_signal, worker_running))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            signal,
            running,
            join: Mutex::new(Some(join)),
        })
    }

    pub fn wake(&self) {
        let Ok(mut state) = self.signal.state.lock() else {
            return;
        };
        state.wake_generation = state.wake_generation.wrapping_add(1);
        self.signal.changed.notify_all();
    }

    pub fn stop(&self) {
        let Ok(mut state) = self.signal.state.lock() else {
            return;
        };
        state.stop = true;
        self.signal.changed.notify_all();
    }

    pub fn quiesce(&self) {
        let Ok(mut state) = self.signal.state.lock() else {
            return;
        };
        state.quiesce = true;
        state.wake_generation = state.wake_generation.wrapping_add(1);
        self.signal.changed.notify_all();
    }

    pub fn join(&self) -> Result<(), String> {
        let join = self
            .join
            .lock()
            .map_err(|_| "sync_worker_join_poisoned".to_string())?
            .take();
        match join {
            Some(join) => join
                .join()
                .map_err(|_| "sync_worker_panicked".to_string())?,
            None => Ok(()),
        }
    }

    pub fn wait_until_stopped(&self, timeout: Duration) -> Result<bool, String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let finished = self
                .join
                .lock()
                .map_err(|_| "sync_worker_join_poisoned".to_string())?
                .as_ref()
                .is_none_or(JoinHandle::is_finished);
            if finished {
                return Ok(true);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            thread::sleep((deadline - now).min(Duration::from_millis(10)));
        }
    }

    pub fn running_identity(&self) -> Option<RunningSyncIdentity> {
        self.running
            .lock()
            .ok()
            .and_then(|running| running.as_ref().map(|running| running.identity.clone()))
    }
}

impl Drop for SyncWorkerHandle {
    fn drop(&mut self) {
        self.stop();
        let _ = self.join();
    }
}

fn worker_loop(
    config: SyncWorkerConfig,
    signal: Arc<WorkerSignal>,
    running: Arc<Mutex<Option<WorkerRunning>>>,
) -> Result<(), String> {
    let idle_wait = config.effective_idle_wait();
    let mut observed_generation = 0;
    loop {
        if should_stop(&signal)? {
            return Ok(());
        }
        let quiescing = should_quiesce(&signal)?;
        if quiescing {
            let connection = Connection::open(&config.database_path)
                .map_err(|error| error.to_string())?;
            release_terminal_running(&connection, &running, config.terminal_observer.as_ref())?;
            if running
                .lock()
                .map_err(|_| "sync_worker_running_poisoned".to_string())?
                .is_none()
            {
                return Ok(());
            }
        } else {
            process_queue_once(&config, &running)?;
        }

        let mut state = signal
            .state
            .lock()
            .map_err(|_| "sync_worker_signal_poisoned".to_string())?;
        if state.stop {
            return Ok(());
        }
        if state.wake_generation == observed_generation {
            let (next, _) = signal
                .changed
                .wait_timeout(state, idle_wait)
                .map_err(|_| "sync_worker_signal_poisoned".to_string())?;
            state = next;
        }
        observed_generation = state.wake_generation;
    }
}

fn should_stop(signal: &WorkerSignal) -> Result<bool, String> {
    signal
        .state
        .lock()
        .map(|state| state.stop)
        .map_err(|_| "sync_worker_signal_poisoned".to_string())
}

fn should_quiesce(signal: &WorkerSignal) -> Result<bool, String> {
    signal
        .state
        .lock()
        .map(|state| state.quiesce)
        .map_err(|_| "sync_worker_signal_poisoned".to_string())
}

fn process_queue_once(
    config: &SyncWorkerConfig,
    running: &Arc<Mutex<Option<WorkerRunning>>>,
) -> Result<(), String> {
    let mut connection =
        Connection::open(&config.database_path).map_err(|error| error.to_string())?;
    release_terminal_running(&connection, running, config.terminal_observer.as_ref())?;
    if running
        .lock()
        .map_err(|_| "sync_worker_running_poisoned".to_string())?
        .is_some()
    {
        return Ok(());
    }
    let database_has_running: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_jobs WHERE status = 'running')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if database_has_running {
        return Ok(());
    }

    for queued in QueueStore::list_queued_jobs(&connection)? {
        let Some(validation_ticket) = config
            .admission
            .begin_dequeue_validation(&queued.snapshot)?
        else {
            continue;
        };
        match config.validator.validate(&connection, &queued)? {
            WorkerJobDecision::Run => {}
            WorkerJobDecision::RetainQueued => continue,
            WorkerJobDecision::Cancel { error_code } => {
                let terminal = QueueStore::cancel_queued_job(
                    &mut connection,
                    queued.snapshot.id,
                    &now_iso(),
                    &error_code,
                )?;
                observe_terminal_best_effort(config.terminal_observer.as_ref(), &terminal);
                continue;
            }
        }
        let Some(executor) = config.registry.executor(queued.snapshot.task_key.source) else {
            let terminal = QueueStore::cancel_queued_job(
                &mut connection,
                queued.snapshot.id,
                &now_iso(),
                "sync_source_not_available",
            )?;
            observe_terminal_best_effort(config.terminal_observer.as_ref(), &terminal);
            continue;
        };
        let admitted = match config
            .admission
            .admit_sync_after_validation(queued.snapshot.clone(), validation_ticket)
        {
            Ok(admitted) => admitted,
            Err(error)
                if matches!(
                    error.as_str(),
                    "account_operation_in_progress"
                        | "browser_operation_in_progress"
                        | "account_browser_operation_in_progress"
                        | "sync_dequeue_blocked"
                        | "sync_dequeue_changed"
                        | "operation_admission_closed"
                ) =>
            {
                continue
            }
            Err(error) => return Err(error),
        };
        let claimed = match QueueStore::claim_job(&mut connection, queued.snapshot.id, &now_iso()) {
            Ok(claimed) => claimed,
            Err(error)
                if matches!(
                    error.as_str(),
                    "sync_job_not_queued" | "sync_already_running"
                ) =>
            {
                drop(admitted);
                continue;
            }
            Err(error) => return Err(error),
        };

        match executor.start(QueuedSyncContext {
            job: claimed.snapshot.clone(),
        }) {
            Ok(identity) => {
                if identity.job_id != claimed.snapshot.id {
                    let terminal = finish_launch_failure(
                        &mut connection,
                        claimed.snapshot.id,
                        "sync_executor_identity_mismatch",
                    )?;
                    observe_terminal_best_effort(config.terminal_observer.as_ref(), &terminal);
                    drop(admitted);
                    continue;
                }
                let mut active = running
                    .lock()
                    .map_err(|_| "sync_worker_running_poisoned".to_string())?;
                if active.is_some() {
                    return Err("sync_worker_running_conflict".to_string());
                }
                *active = Some(WorkerRunning {
                    identity,
                    _admitted: admitted,
                });
                return Ok(());
            }
            Err(error) => {
                let code = if error.trim().is_empty() {
                    "sync_executor_start_failed"
                } else {
                    error.as_str()
                };
                let terminal = finish_launch_failure(&mut connection, claimed.snapshot.id, code)?;
                observe_terminal_best_effort(config.terminal_observer.as_ref(), &terminal);
                drop(admitted);
            }
        }
    }
    Ok(())
}

fn release_terminal_running(
    connection: &Connection,
    running: &Arc<Mutex<Option<WorkerRunning>>>,
    terminal_observer: &dyn SyncWorkerTerminalObserver,
) -> Result<(), String> {
    let job_id = running
        .lock()
        .map_err(|_| "sync_worker_running_poisoned".to_string())?
        .as_ref()
        .map(|running| running.identity.job_id);
    let Some(job_id) = job_id else {
        return Ok(());
    };
    let terminal = QueueStore::get_stored_job(connection, job_id)?;
    if terminal.snapshot.status != SyncJobStatusV1::Running {
        observe_terminal_best_effort(terminal_observer, &terminal);
        running
            .lock()
            .map_err(|_| "sync_worker_running_poisoned".to_string())?
            .take();
    }
    Ok(())
}

fn observe_terminal_best_effort(
    terminal_observer: &dyn SyncWorkerTerminalObserver,
    terminal: &StoredSyncJobV1,
) {
    if terminal_observer.observe(terminal).is_err() {
        eprintln!("sync_terminal_observer_failed");
    }
}

fn finish_launch_failure(
    connection: &mut Connection,
    job_id: i64,
    error_code: &str,
) -> Result<StoredSyncJobV1, String> {
    QueueStore::finish_running_job(
        connection,
        job_id,
        &TerminalJobUpdateV1 {
            status: SyncJobStatusV1::Failed,
            finished_at: now_iso(),
            inserted_videos: 0,
            inserted_records: 0,
            error_code: Some(error_code.to_string()),
            error_message: Some(error_code.to_string()),
        },
    )
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
