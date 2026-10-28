use super::{SyncJobSnapshotV1, SyncJobStatusV1, SyncTriggerV1, TaskKeyV1};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, Weak,
    },
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationAdmissionMode {
    Open,
    MaintenancePending,
    MaintenanceRunning,
    ExitPending,
    Exiting,
}

impl Default for OperationAdmissionMode {
    fn default() -> Self {
        Self::Open
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserOperationKind {
    Login,
    Sync,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationAction {
    WriteProgress,
    ImportResult,
    AssociateBatchChild,
    WriteTerminalState,
    StartBrowser,
    EnqueueJob,
    ModifySettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationAdmissionSnapshot {
    pub mode: OperationAdmissionMode,
    pub browser_operation: Option<BrowserOperationKind>,
    pub browser_account_id: Option<i64>,
    pub automatic_paused: bool,
    pub automatic_pause_pending: bool,
    pub continuation_count: usize,
    pub continuation_job_ids: Vec<i64>,
    pub owned_operation_count: usize,
}

#[derive(Clone)]
pub struct OperationAdmission {
    inner: Arc<AdmissionInner>,
}

struct AdmissionInner {
    state: Mutex<AdmissionState>,
    changed: Condvar,
    worker_waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

#[derive(Default)]
struct AdmissionState {
    mode: OperationAdmissionMode,
    next_id: u64,
    dequeue_epoch: u64,
    browser: Option<BrowserEntry>,
    continuations: BTreeMap<u64, ContinuationEntry>,
    owned: BTreeMap<u64, OwnedEntry>,
    source_settings_updates: BTreeSet<TaskKeyV1>,
    account_source_settings_updates: BTreeSet<i64>,
    account_browser_updates: BTreeSet<i64>,
    device_settings_update: bool,
    automatic_paused: bool,
    automatic_pause_pending: bool,
    exit_abort_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DequeueValidationTicket {
    job_id: i64,
    task_key: TaskKeyV1,
    trigger: SyncTriggerV1,
    dequeue_epoch: u64,
}

#[derive(Clone)]
struct BrowserEntry {
    id: u64,
    kind: BrowserOperationKind,
    account_id: Option<i64>,
}

#[derive(Clone)]
struct ContinuationEntry {
    job_id: i64,
    task_key: TaskKeyV1,
}

#[derive(Clone)]
enum OwnedKind {
    Enqueue,
    SourceSettings(TaskKeyV1),
    AccountSourceSettings(i64),
    AccountBrowser(i64),
    DeviceSettings,
    AutomaticPause,
    Maintenance,
    Exit,
}

#[derive(Clone)]
struct OwnedEntry {
    kind: OwnedKind,
    cancelled: Arc<AtomicBool>,
}

impl OperationAdmission {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AdmissionInner {
                state: Mutex::new(AdmissionState::default()),
                changed: Condvar::new(),
                worker_waker: Mutex::new(None),
            }),
        }
    }

    pub fn snapshot(&self) -> Result<OperationAdmissionSnapshot, String> {
        let state = self.lock()?;
        Ok(OperationAdmissionSnapshot {
            mode: state.mode,
            browser_operation: state.browser.as_ref().map(|entry| entry.kind),
            browser_account_id: state.browser.as_ref().and_then(|entry| entry.account_id),
            automatic_paused: state.automatic_paused,
            automatic_pause_pending: state.automatic_pause_pending,
            continuation_count: state.continuations.len(),
            continuation_job_ids: state
                .continuations
                .values()
                .map(|entry| entry.job_id)
                .collect(),
            owned_operation_count: state.owned.len(),
        })
    }

    pub fn set_worker_waker(&self, waker: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        *self
            .inner
            .worker_waker
            .lock()
            .map_err(|_| "operation_admission_poisoned".to_string())? = Some(waker);
        Ok(())
    }

    pub fn begin_browser_operation(
        &self,
        kind: BrowserOperationKind,
        account_id: Option<i64>,
    ) -> Result<BrowserOperationLease, String> {
        if account_id.is_some_and(|value| value <= 0) {
            return Err("invalid_account_id".to_string());
        }
        let mut state = self.lock()?;
        ensure_open(&state)?;
        ensure_browser_available(&state, account_id)?;
        let id = reserve_id(&mut state)?;
        state.browser = Some(BrowserEntry {
            id,
            kind,
            account_id,
        });
        Ok(BrowserOperationLease {
            registration: Arc::new(BrowserRegistration {
                inner: Arc::downgrade(&self.inner),
                id,
            }),
        })
    }

    pub fn begin_enqueue_operation(&self) -> Result<OwnedAdmissionOperation, String> {
        let mut state = self.lock()?;
        ensure_open(&state)?;
        if state
            .owned
            .values()
            .any(|entry| matches!(entry.kind, OwnedKind::AccountBrowser(_)))
        {
            return Err("account_browser_operation_in_progress".to_string());
        }
        register_owned(&self.inner, &mut state, OwnedKind::Enqueue)
    }

    pub fn admit_sync(&self, job: SyncJobSnapshotV1) -> Result<AdmittedSync, String> {
        validate_queued_job(&job)?;
        let mut state = self.lock()?;
        ensure_open(&state)?;
        ensure_browser_available(&state, Some(job.task_key.account_id))?;
        if !can_dequeue_locked(&state, &job) {
            return Err("sync_dequeue_blocked".to_string());
        }

        admit_sync_locked(&self.inner, &mut state, job)
    }

    pub fn begin_dequeue_validation(
        &self,
        job: &SyncJobSnapshotV1,
    ) -> Result<Option<DequeueValidationTicket>, String> {
        validate_queued_job(job)?;
        let state = self.lock()?;
        if !can_dequeue_locked(&state, job) {
            return Ok(None);
        }
        Ok(Some(DequeueValidationTicket {
            job_id: job.id,
            task_key: job.task_key.clone(),
            trigger: job.trigger,
            dequeue_epoch: state.dequeue_epoch,
        }))
    }

    pub fn admit_sync_after_validation(
        &self,
        job: SyncJobSnapshotV1,
        ticket: DequeueValidationTicket,
    ) -> Result<AdmittedSync, String> {
        validate_queued_job(&job)?;
        if ticket.job_id != job.id
            || ticket.task_key != job.task_key
            || ticket.trigger != job.trigger
        {
            return Err("sync_dequeue_validation_mismatch".to_string());
        }
        let mut state = self.lock()?;
        ensure_open(&state)?;
        ensure_browser_available(&state, Some(job.task_key.account_id))?;
        if state.dequeue_epoch != ticket.dequeue_epoch {
            return Err("sync_dequeue_changed".to_string());
        }
        if !can_dequeue_locked(&state, &job) {
            return Err("sync_dequeue_blocked".to_string());
        }

        admit_sync_locked(&self.inner, &mut state, job)
    }

    pub fn can_dequeue(&self, job: &SyncJobSnapshotV1) -> Result<bool, String> {
        validate_queued_job(job)?;
        let state = self.lock()?;
        Ok(can_dequeue_locked(&state, job))
    }

    pub fn begin_source_settings_update(
        &self,
        task_key: TaskKeyV1,
    ) -> Result<OwnedAdmissionOperation, String> {
        if task_key.account_id <= 0 {
            return Err("invalid_task_key".to_string());
        }
        let mut state = self.lock()?;
        ensure_open(&state)?;
        if state.source_settings_updates.contains(&task_key) {
            return Err("settings_update_in_progress".to_string());
        }
        let operation = register_owned(
            &self.inner,
            &mut state,
            OwnedKind::SourceSettings(task_key.clone()),
        )?;
        state.source_settings_updates.insert(task_key);
        Ok(operation)
    }

    pub fn begin_account_source_settings_update(
        &self,
        account_id: i64,
    ) -> Result<OwnedAdmissionOperation, String> {
        if account_id <= 0 {
            return Err("invalid_account_id".to_string());
        }
        let mut state = self.lock()?;
        ensure_open(&state)?;
        if state.account_source_settings_updates.contains(&account_id) {
            return Err("settings_update_in_progress".to_string());
        }
        let operation = register_owned(
            &self.inner,
            &mut state,
            OwnedKind::AccountSourceSettings(account_id),
        )?;
        state.account_source_settings_updates.insert(account_id);
        Ok(operation)
    }

    pub fn begin_account_browser_update(
        &self,
        account_id: i64,
    ) -> Result<OwnedAdmissionOperation, String> {
        if account_id <= 0 {
            return Err("invalid_account_id".to_string());
        }
        let mut state = self.lock()?;
        ensure_open(&state)?;
        if state.account_browser_updates.contains(&account_id)
            || state
                .owned
                .values()
                .any(|entry| matches!(entry.kind, OwnedKind::Enqueue))
            || state
                .browser
                .as_ref()
                .is_some_and(|entry| entry.account_id == Some(account_id))
            || state
                .continuations
                .values()
                .any(|entry| entry.task_key.account_id == account_id)
        {
            return Err("account_browser_operation_in_progress".to_string());
        }
        let operation = register_owned(
            &self.inner,
            &mut state,
            OwnedKind::AccountBrowser(account_id),
        )?;
        state.account_browser_updates.insert(account_id);
        Ok(operation)
    }

    pub fn begin_device_settings_update(&self) -> Result<OwnedAdmissionOperation, String> {
        let mut state = self.lock()?;
        ensure_open(&state)?;
        if state.device_settings_update {
            return Err("settings_update_in_progress".to_string());
        }
        let operation = register_owned(&self.inner, &mut state, OwnedKind::DeviceSettings)?;
        state.device_settings_update = true;
        Ok(operation)
    }

    pub fn begin_automatic_pause_update(&self) -> Result<AutomaticPauseUpdate, String> {
        let mut state = self.lock()?;
        ensure_open(&state)?;
        if state.automatic_pause_pending {
            return Err("settings_update_in_progress".to_string());
        }
        let operation = register_owned(&self.inner, &mut state, OwnedKind::AutomaticPause)?;
        state.automatic_pause_pending = true;
        Ok(AutomaticPauseUpdate {
            admission: self.clone(),
            operation: Some(operation),
        })
    }

    pub fn set_automatic_paused(&self, paused: bool) -> Result<(), String> {
        let mut state = self.lock()?;
        ensure_open(&state)?;
        if state.automatic_pause_pending {
            return Err("settings_update_in_progress".to_string());
        }
        state.automatic_paused = paused;
        bump_dequeue_epoch(&mut state);
        notify_changed(&self.inner);
        Ok(())
    }

    pub fn begin_maintenance(&self) -> Result<MaintenanceOperation, String> {
        let mut state = self.lock()?;
        ensure_open(&state)?;
        if !state.source_settings_updates.is_empty()
            || !state.account_source_settings_updates.is_empty()
            || !state.account_browser_updates.is_empty()
            || state.device_settings_update
            || state.automatic_pause_pending
        {
            return Err("settings_update_in_progress".to_string());
        }
        if state
            .browser
            .as_ref()
            .is_some_and(|entry| {
                matches!(
                    entry.kind,
                    BrowserOperationKind::Login | BrowserOperationKind::Download
                )
            })
        {
            return Err("browser_operation_in_progress".to_string());
        }
        let operation = register_owned(&self.inner, &mut state, OwnedKind::Maintenance)?;
        state.mode = OperationAdmissionMode::MaintenancePending;
        notify_changed(&self.inner);
        Ok(MaintenanceOperation {
            admission: self.clone(),
            operation: Some(operation),
        })
    }

    pub fn begin_exit(&self) -> Result<ExitOperation, String> {
        let mut state = self.lock()?;
        if matches!(
            state.mode,
            OperationAdmissionMode::ExitPending | OperationAdmissionMode::Exiting
        ) {
            return Err("exit_already_requested".to_string());
        }
        let operation = register_owned(&self.inner, &mut state, OwnedKind::Exit)?;
        state.mode = OperationAdmissionMode::ExitPending;
        state.exit_abort_pending = false;
        notify_changed(&self.inner);
        Ok(ExitOperation {
            admission: self.clone(),
            operation: Some(operation),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, AdmissionState>, String> {
        self.inner
            .state
            .lock()
            .map_err(|_| "operation_admission_poisoned".to_string())
    }
}

impl Default for OperationAdmission {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for OperationAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationAdmission")
            .field("snapshot", &self.snapshot().ok())
            .finish()
    }
}

#[derive(Clone)]
pub struct BrowserOperationLease {
    registration: Arc<BrowserRegistration>,
}

struct BrowserRegistration {
    inner: Weak<AdmissionInner>,
    id: u64,
}

impl Drop for BrowserRegistration {
    fn drop(&mut self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let Ok(mut state) = inner.state.lock() else {
            return;
        };
        if state
            .browser
            .as_ref()
            .is_some_and(|entry| entry.id == self.id)
        {
            state.browser = None;
            notify_changed(&inner);
        }
    }
}

impl fmt::Debug for BrowserOperationLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserOperationLease")
            .field("id", &self.registration.id)
            .finish_non_exhaustive()
    }
}

pub struct AdmittedSync {
    browser: BrowserOperationLease,
    continuation: OperationContinuationToken,
}

impl AdmittedSync {
    pub fn continuation(&self) -> OperationContinuationToken {
        self.continuation.clone()
    }

    pub fn browser_lease(&self) -> BrowserOperationLease {
        self.browser.clone()
    }
}

impl fmt::Debug for AdmittedSync {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedSync")
            .field("job_id", &self.continuation.job_id())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct OperationContinuationToken {
    registration: Arc<ContinuationRegistration>,
}

struct ContinuationRegistration {
    inner: Weak<AdmissionInner>,
    id: u64,
    job_id: i64,
    task_key: TaskKeyV1,
}

impl OperationContinuationToken {
    pub fn job_id(&self) -> i64 {
        self.registration.job_id
    }

    pub fn task_key(&self) -> &TaskKeyV1 {
        &self.registration.task_key
    }

    pub fn allows(&self, action: ContinuationAction) -> bool {
        matches!(
            action,
            ContinuationAction::WriteProgress
                | ContinuationAction::ImportResult
                | ContinuationAction::AssociateBatchChild
                | ContinuationAction::WriteTerminalState
        )
    }
}

impl Drop for ContinuationRegistration {
    fn drop(&mut self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let Ok(mut state) = inner.state.lock() else {
            return;
        };
        if state.continuations.remove(&self.id).is_some() {
            notify_changed(&inner);
        }
    }
}

impl fmt::Debug for OperationContinuationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationContinuationToken")
            .field("job_id", &self.job_id())
            .field("task_key", self.task_key())
            .finish_non_exhaustive()
    }
}

pub struct OwnedAdmissionOperation {
    registration: Option<OwnedRegistration>,
    cancellation: OwnedOperationCancellation,
}

#[derive(Clone)]
pub struct OwnedOperationCancellation {
    cancelled: Arc<AtomicBool>,
}

impl OwnedOperationCancellation {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

struct OwnedRegistration {
    inner: Weak<AdmissionInner>,
    id: u64,
}

impl OwnedAdmissionOperation {
    pub fn cancellation_token(&self) -> OwnedOperationCancellation {
        self.cancellation.clone()
    }

    fn finish(&mut self) {
        if let Some(registration) = self.registration.take() {
            finish_owned(&registration);
        }
    }
}

impl Drop for OwnedAdmissionOperation {
    fn drop(&mut self) {
        self.finish();
    }
}

impl fmt::Debug for OwnedAdmissionOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedAdmissionOperation")
            .field(
                "id",
                &self
                    .registration
                    .as_ref()
                    .map(|registration| registration.id),
            )
            .finish_non_exhaustive()
    }
}

pub struct AutomaticPauseUpdate {
    admission: OperationAdmission,
    operation: Option<OwnedAdmissionOperation>,
}

impl AutomaticPauseUpdate {
    pub fn commit(mut self, paused: bool) -> Result<(), String> {
        {
            let mut state = self.admission.lock()?;
            if !state.automatic_pause_pending {
                return Err("automatic_pause_update_mismatch".to_string());
            }
            state.automatic_paused = paused;
        }
        if let Some(mut operation) = self.operation.take() {
            operation.finish();
        }
        Ok(())
    }
}

impl fmt::Debug for AutomaticPauseUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AutomaticPauseUpdate")
            .finish_non_exhaustive()
    }
}

pub struct MaintenanceOperation {
    admission: OperationAdmission,
    operation: Option<OwnedAdmissionOperation>,
}

impl MaintenanceOperation {
    pub fn wait_until_ready(
        mut self,
        timeout: Duration,
    ) -> Result<ActiveMaintenanceOperation, String> {
        let state = self.admission.lock()?;
        let (mut state, wait) = self
            .admission
            .inner
            .changed
            .wait_timeout_while(state, timeout, |state| {
                !state.continuations.is_empty() || state.browser.is_some()
            })
            .map_err(|_| "operation_admission_poisoned".to_string())?;
        if wait.timed_out() && (!state.continuations.is_empty() || state.browser.is_some()) {
            drop(state);
            if let Some(mut operation) = self.operation.take() {
                operation.finish();
            }
            return Err("operation_timeout".to_string());
        }
        if state.mode != OperationAdmissionMode::MaintenancePending {
            return Err("maintenance_state_mismatch".to_string());
        }
        state.mode = OperationAdmissionMode::MaintenanceRunning;
        drop(state);
        Ok(ActiveMaintenanceOperation {
            operation: self.operation.take(),
        })
    }
}

impl fmt::Debug for MaintenanceOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaintenanceOperation")
            .finish_non_exhaustive()
    }
}

pub struct ActiveMaintenanceOperation {
    operation: Option<OwnedAdmissionOperation>,
}

impl ActiveMaintenanceOperation {
    pub fn finish(mut self) {
        if let Some(mut operation) = self.operation.take() {
            operation.finish();
        }
    }
}

impl fmt::Debug for ActiveMaintenanceOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActiveMaintenanceOperation")
            .finish_non_exhaustive()
    }
}

pub struct ExitOperation {
    admission: OperationAdmission,
    operation: Option<OwnedAdmissionOperation>,
}

impl ExitOperation {
    pub fn request_owned_cancellation(&self) -> Result<usize, String> {
        let registration_id = self
            .operation
            .as_ref()
            .and_then(|operation| operation.registration.as_ref())
            .map(|registration| registration.id)
            .ok_or_else(|| "exit_state_mismatch".to_string())?;
        let state = self.admission.lock()?;
        if state.mode != OperationAdmissionMode::ExitPending || state.exit_abort_pending {
            return Err("exit_state_mismatch".to_string());
        }
        let mut cancelled = 0;
        for (operation_id, entry) in &state.owned {
            if *operation_id != registration_id {
                entry.cancelled.store(true, Ordering::Release);
                cancelled += 1;
            }
        }
        Ok(cancelled)
    }

    pub fn wait_until_prepared(&self, timeout: Duration) -> Result<(), String> {
        let registration = self
            .operation
            .as_ref()
            .and_then(|operation| operation.registration.as_ref())
            .ok_or_else(|| "exit_state_mismatch".to_string())?;
        let state = self.admission.lock()?;
        let (state, wait) = self
            .admission
            .inner
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.mode == OperationAdmissionMode::ExitPending
                    && state
                        .owned
                        .keys()
                        .any(|operation_id| *operation_id != registration.id)
            })
            .map_err(|_| "operation_admission_poisoned".to_string())?;
        let other_owned = state
            .owned
            .keys()
            .any(|operation_id| *operation_id != registration.id);
        if wait.timed_out() && other_owned {
            Err("operation_timeout".to_string())
        } else if state.mode != OperationAdmissionMode::ExitPending || state.exit_abort_pending {
            Err("exit_state_mismatch".to_string())
        } else {
            Ok(())
        }
    }

    pub fn commit_marker(mut self) -> Result<(), String> {
        {
            let mut state = self.admission.lock()?;
            if state.mode != OperationAdmissionMode::ExitPending || state.exit_abort_pending {
                return Err("exit_state_mismatch".to_string());
            }
            let registration_id = self
                .operation
                .as_ref()
                .and_then(|operation| operation.registration.as_ref())
                .map(|registration| registration.id)
                .ok_or_else(|| "exit_state_mismatch".to_string())?;
            if state
                .owned
                .keys()
                .any(|operation_id| *operation_id != registration_id)
            {
                return Err("owned_operation_in_progress".to_string());
            }
            state.mode = OperationAdmissionMode::Exiting;
        }
        if let Some(mut operation) = self.operation.take() {
            operation.finish();
        }
        Ok(())
    }

    pub fn abort(mut self) -> Result<(), String> {
        {
            let mut state = self.admission.lock()?;
            if state.mode != OperationAdmissionMode::ExitPending {
                return Err("exit_state_mismatch".to_string());
            }
            state.exit_abort_pending = true;
        }
        if let Some(mut operation) = self.operation.take() {
            operation.finish();
        }
        Ok(())
    }
}

impl fmt::Debug for ExitOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExitOperation")
            .finish_non_exhaustive()
    }
}

fn validate_queued_job(job: &SyncJobSnapshotV1) -> Result<(), String> {
    if job.id <= 0 || job.task_key.account_id <= 0 || job.status != SyncJobStatusV1::Queued {
        return Err("invalid_sync_job_snapshot".to_string());
    }
    Ok(())
}

fn ensure_open(state: &AdmissionState) -> Result<(), String> {
    if state.mode == OperationAdmissionMode::Open {
        Ok(())
    } else {
        Err("operation_admission_closed".to_string())
    }
}

fn ensure_browser_available(
    state: &AdmissionState,
    requested_account_id: Option<i64>,
) -> Result<(), String> {
    if state
        .account_browser_updates
        .iter()
        .any(|account_id| Some(*account_id) == requested_account_id)
    {
        return Err("account_browser_operation_in_progress".to_string());
    }
    if let Some(current) = &state.browser {
        if requested_account_id.is_some() && current.account_id == requested_account_id {
            Err("account_operation_in_progress".to_string())
        } else {
            Err("browser_operation_in_progress".to_string())
        }
    } else {
        Ok(())
    }
}

fn can_dequeue_locked(state: &AdmissionState, job: &SyncJobSnapshotV1) -> bool {
    if state.mode != OperationAdmissionMode::Open
        || state.browser.is_some()
        || state.device_settings_update
        || state
            .account_browser_updates
            .contains(&job.task_key.account_id)
    {
        return false;
    }
    if job.trigger == SyncTriggerV1::Automatic {
        !state.automatic_paused
            && !state.automatic_pause_pending
            && !state
                .account_source_settings_updates
                .contains(&job.task_key.account_id)
            && !state.source_settings_updates.contains(&job.task_key)
    } else {
        true
    }
}

fn admit_sync_locked(
    inner: &Arc<AdmissionInner>,
    state: &mut AdmissionState,
    job: SyncJobSnapshotV1,
) -> Result<AdmittedSync, String> {
    let browser_id = reserve_id(state)?;
    state.browser = Some(BrowserEntry {
        id: browser_id,
        kind: BrowserOperationKind::Sync,
        account_id: Some(job.task_key.account_id),
    });
    let continuation_id = match reserve_id(state) {
        Ok(id) => id,
        Err(error) => {
            state.browser = None;
            return Err(error);
        }
    };
    state.continuations.insert(
        continuation_id,
        ContinuationEntry {
            job_id: job.id,
            task_key: job.task_key.clone(),
        },
    );

    Ok(AdmittedSync {
        browser: BrowserOperationLease {
            registration: Arc::new(BrowserRegistration {
                inner: Arc::downgrade(inner),
                id: browser_id,
            }),
        },
        continuation: OperationContinuationToken {
            registration: Arc::new(ContinuationRegistration {
                inner: Arc::downgrade(inner),
                id: continuation_id,
                job_id: job.id,
                task_key: job.task_key,
            }),
        },
    })
}

fn reserve_id(state: &mut AdmissionState) -> Result<u64, String> {
    state.next_id = state
        .next_id
        .checked_add(1)
        .ok_or_else(|| "operation_admission_id_exhausted".to_string())?;
    Ok(state.next_id)
}

fn register_owned(
    inner: &Arc<AdmissionInner>,
    state: &mut AdmissionState,
    kind: OwnedKind,
) -> Result<OwnedAdmissionOperation, String> {
    let id = reserve_id(state)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    state.owned.insert(
        id,
        OwnedEntry {
            kind,
            cancelled: Arc::clone(&cancelled),
        },
    );
    bump_dequeue_epoch(state);
    Ok(OwnedAdmissionOperation {
        registration: Some(OwnedRegistration {
            inner: Arc::downgrade(inner),
            id,
        }),
        cancellation: OwnedOperationCancellation { cancelled },
    })
}

fn finish_owned(registration: &OwnedRegistration) {
    let Some(inner) = registration.inner.upgrade() else {
        return;
    };
    let Ok(mut state) = inner.state.lock() else {
        return;
    };
    let Some(entry) = state.owned.remove(&registration.id) else {
        return;
    };
    match entry.kind {
        OwnedKind::Enqueue => {}
        OwnedKind::SourceSettings(task_key) => {
            state.source_settings_updates.remove(&task_key);
        }
        OwnedKind::AccountSourceSettings(account_id) => {
            state.account_source_settings_updates.remove(&account_id);
        }
        OwnedKind::AccountBrowser(account_id) => {
            state.account_browser_updates.remove(&account_id);
        }
        OwnedKind::DeviceSettings => state.device_settings_update = false,
        OwnedKind::AutomaticPause => state.automatic_pause_pending = false,
        OwnedKind::Maintenance => {
            if matches!(
                state.mode,
                OperationAdmissionMode::MaintenancePending
                    | OperationAdmissionMode::MaintenanceRunning
            ) {
                state.mode = OperationAdmissionMode::Open;
            }
        }
        OwnedKind::Exit => {}
    }
    if state.mode == OperationAdmissionMode::ExitPending
        && state.exit_abort_pending
        && state.owned.is_empty()
    {
        state.mode = OperationAdmissionMode::Open;
        state.exit_abort_pending = false;
    }
    bump_dequeue_epoch(&mut state);
    notify_changed(&inner);
}

fn bump_dequeue_epoch(state: &mut AdmissionState) {
    state.dequeue_epoch = state.dequeue_epoch.wrapping_add(1);
}

fn notify_changed(inner: &AdmissionInner) {
    inner.changed.notify_all();
    let waker = inner
        .worker_waker
        .lock()
        .ok()
        .and_then(|waker| waker.clone());
    if let Some(waker) = waker {
        waker();
    }
}
