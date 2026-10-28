use crate::{
    process_lock::GuiActivationEvent,
    settings_store::CloseBehaviorV1,
    sync::{ExitOperation, OperationAdmission, OperationAdmissionMode, SyncScopeV1},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub use crate::startup_recovery::StartupContinuationPermitV1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondInstanceDecisionV1 {
    SignalExisting,
    ExitSilently,
}

pub fn second_instance_decision(autostart: bool) -> SecondInstanceDecisionV1 {
    if autostart {
        SecondInstanceDecisionV1::ExitSilently
    } else {
        SecondInstanceDecisionV1::SignalExisting
    }
}

pub struct GuiActivationListener {
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<Result<(), String>>>>,
}

impl GuiActivationListener {
    pub fn start(
        event: GuiActivationEvent,
        on_activation: impl Fn() + Send + 'static,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("emily-gui-activation".to_string())
            .spawn(move || {
                while !stop_for_thread.load(Ordering::Acquire) {
                    if event.wait_timeout(Duration::from_millis(100))? {
                        on_activation();
                    }
                }
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub fn join(&self) -> Result<(), String> {
        let thread = self
            .thread
            .lock()
            .map_err(|_| "gui_activation_listener_poisoned".to_string())?
            .take();
        match thread {
            Some(thread) => thread
                .join()
                .map_err(|_| "gui_activation_listener_panicked".to_string())?,
            None => Ok(()),
        }
    }
}

impl Drop for GuiActivationListener {
    fn drop(&mut self) {
        self.stop();
        let _ = self.join();
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopIntentSnapshotV1 {
    pub revision: u64,
    pub show_main: bool,
    pub open_settings: bool,
}

#[derive(Debug, Default, Clone)]
pub struct DesktopIntentState {
    inner: Arc<Mutex<DesktopIntentSnapshotV1>>,
}

impl Default for DesktopIntentSnapshotV1 {
    fn default() -> Self {
        Self {
            revision: 0,
            show_main: false,
            open_settings: false,
        }
    }
}

impl DesktopIntentState {
    pub fn request(
        &self,
        show_main: bool,
        open_settings: bool,
    ) -> Result<DesktopIntentSnapshotV1, String> {
        if !show_main && !open_settings {
            return Err("desktop_intent_empty".to_string());
        }
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "desktop_intent_state_poisoned".to_string())?;
        state.revision = state
            .revision
            .checked_add(1)
            .ok_or_else(|| "desktop_intent_revision_overflow".to_string())?;
        state.show_main |= show_main;
        state.open_settings |= open_settings;
        Ok(*state)
    }

    pub fn take(&self, last_seen_revision: u64) -> Result<DesktopIntentSnapshotV1, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "desktop_intent_state_poisoned".to_string())?;
        if last_seen_revision >= state.revision {
            return Ok(DesktopIntentSnapshotV1 {
                revision: state.revision,
                show_main: false,
                open_settings: false,
            });
        }
        let snapshot = *state;
        state.show_main = false;
        state.open_settings = false;
        Ok(snapshot)
    }

    pub fn pending(&self) -> Result<DesktopIntentSnapshotV1, String> {
        self.inner
            .lock()
            .map(|state| *state)
            .map_err(|_| "desktop_intent_state_poisoned".to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseWindowDecisionV1 {
    HideToTray { show_hint: bool },
    KeepVisibleTrayUnavailable,
    RequestExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseWindowDecisionInputV1 {
    pub close_behavior: CloseBehaviorV1,
    pub tray_available: bool,
    pub onboarding_completed: bool,
    pub first_tray_hint_shown: bool,
    pub system_exit: bool,
}

pub fn close_window_decision(input: CloseWindowDecisionInputV1) -> CloseWindowDecisionV1 {
    if input.system_exit
        || !input.onboarding_completed
        || input.close_behavior == CloseBehaviorV1::Exit
    {
        return CloseWindowDecisionV1::RequestExit;
    }
    if !input.tray_available {
        return CloseWindowDecisionV1::KeepVisibleTrayUnavailable;
    }
    CloseWindowDecisionV1::HideToTray {
        show_hint: !input.first_tray_hint_shown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuItemIdV1 {
    OpenMain,
    SyncCurrentAccount,
    SyncAllAccounts,
    PauseAutomatic,
    OpenSettings,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayMenuItemDecisionV1 {
    pub id: TrayMenuItemIdV1,
    pub enabled: bool,
    pub checked: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayMenuDecisionInputV1 {
    pub runtime_ready: bool,
    pub current_account_active: bool,
    pub has_active_accounts: bool,
    pub admission_mode: OperationAdmissionMode,
    pub scheduler_paused: bool,
}

pub fn tray_menu_decision(input: TrayMenuDecisionInputV1) -> Vec<TrayMenuItemDecisionV1> {
    let open = input.runtime_ready && input.admission_mode == OperationAdmissionMode::Open;
    vec![
        TrayMenuItemDecisionV1 {
            id: TrayMenuItemIdV1::OpenMain,
            enabled: true,
            checked: None,
        },
        TrayMenuItemDecisionV1 {
            id: TrayMenuItemIdV1::SyncCurrentAccount,
            enabled: open && input.current_account_active,
            checked: None,
        },
        TrayMenuItemDecisionV1 {
            id: TrayMenuItemIdV1::SyncAllAccounts,
            enabled: open && input.has_active_accounts,
            checked: None,
        },
        TrayMenuItemDecisionV1 {
            id: TrayMenuItemIdV1::PauseAutomatic,
            enabled: open,
            checked: Some(input.scheduler_paused),
        },
        TrayMenuItemDecisionV1 {
            id: TrayMenuItemIdV1::OpenSettings,
            enabled: true,
            checked: None,
        },
        TrayMenuItemDecisionV1 {
            id: TrayMenuItemIdV1::Exit,
            enabled: true,
            checked: None,
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommandV1 {
    OpenMain,
    SyncCurrentAccount,
    SyncAllAccounts,
    PauseAutomatic,
    OpenSettings,
    Exit,
}

pub fn tray_sync_scope(command: TrayCommandV1) -> Option<SyncScopeV1> {
    match command {
        TrayCommandV1::SyncCurrentAccount => Some(SyncScopeV1::CurrentAccountAvailableSources),
        TrayCommandV1::SyncAllAccounts => Some(SyncScopeV1::AllActiveAccounts),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StartupStageV1 {
    UninstallRecovery,
    Bootstrap,
    Schema,
    SchemaCleanup,
    SyncRecovery,
    NoRunningAssertion,
    DeviceAndOnboarding,
    TrayAndWindow,
    Worker,
}

impl StartupStageV1 {
    pub const PRE_READY: [Self; 6] = [
        Self::UninstallRecovery,
        Self::Bootstrap,
        Self::Schema,
        Self::SchemaCleanup,
        Self::SyncRecovery,
        Self::NoRunningAssertion,
    ];

    pub const POST_READY: [Self; 3] = [
        Self::DeviceAndOnboarding,
        Self::TrayAndWindow,
        Self::Worker,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopStartupState {
    NotStarted,
    Recovering,
    Blocked,
    Ready,
}

type StartupCallback<'a> = Box<dyn FnMut() -> Result<(), String> + 'a>;
type SyncRecoveryCallback<'a> =
    Box<dyn FnMut() -> Result<Option<StartupContinuationPermitV1>, String> + 'a>;

pub struct DesktopStartupContext<'a> {
    state: DesktopStartupState,
    stages: BTreeMap<StartupStageV1, StartupCallback<'a>>,
    sync_recovery: Option<SyncRecoveryCallback<'a>>,
}

impl<'a> DesktopStartupContext<'a> {
    pub fn new() -> Self {
        Self {
            state: DesktopStartupState::NotStarted,
            stages: BTreeMap::new(),
            sync_recovery: None,
        }
    }

    pub fn set_stage<F>(&mut self, stage: StartupStageV1, callback: F)
    where
        F: FnMut() -> Result<(), String> + 'a,
    {
        assert_ne!(stage, StartupStageV1::SyncRecovery);
        self.stages.insert(stage, Box::new(callback));
    }

    pub fn set_sync_recovery<F>(&mut self, callback: F)
    where
        F: FnMut() -> Result<Option<StartupContinuationPermitV1>, String> + 'a,
    {
        self.sync_recovery = Some(Box::new(callback));
    }

    pub fn state(&self) -> DesktopStartupState {
        self.state
    }

    fn run(&mut self, stage: StartupStageV1) -> Result<(), String> {
        self.stages
            .get_mut(&stage)
            .ok_or_else(|| "desktop_startup_stage_missing".to_string())?()
    }
}

impl Default for DesktopStartupContext<'_> {
    fn default() -> Self {
        Self::new()
    }
}

pub fn recover_desktop_startup_once(
    context: &mut DesktopStartupContext<'_>,
) -> Result<Option<StartupContinuationPermitV1>, String> {
    if context.state != DesktopStartupState::NotStarted {
        return Err("desktop_startup_already_started".to_string());
    }
    context.state = DesktopStartupState::Recovering;
    for stage in [
        StartupStageV1::UninstallRecovery,
        StartupStageV1::Bootstrap,
        StartupStageV1::Schema,
        StartupStageV1::SchemaCleanup,
    ] {
        if let Err(error) = context.run(stage) {
            context.state = DesktopStartupState::Blocked;
            return Err(error);
        }
    }
    let permit = match context
        .sync_recovery
        .as_mut()
        .ok_or_else(|| "desktop_startup_stage_missing".to_string())?()
    {
        Ok(Some(permit)) => permit,
        Ok(None) => {
            context.state = DesktopStartupState::Blocked;
            return Ok(None);
        }
        Err(error) => {
            context.state = DesktopStartupState::Blocked;
            return Err(error);
        }
    };
    if let Err(error) = context.run(StartupStageV1::NoRunningAssertion) {
        context.state = DesktopStartupState::Blocked;
        return Err(error);
    }
    context.state = DesktopStartupState::Ready;
    Ok(Some(permit))
}

pub fn continue_desktop_startup_from_ready<F>(
    permit: StartupContinuationPermitV1,
    mut run_stage: F,
) -> Result<(), String>
where
    F: FnMut(StartupStageV1) -> Result<(), String>,
{
    loop {
        let Some(index) = permit.claim_next_stage(StartupStageV1::POST_READY.len())? else {
            return Ok(());
        };
        let stage = StartupStageV1::POST_READY[index];
        match run_stage(stage) {
            Ok(()) => permit.finish_stage(index, true)?,
            Err(error) => {
                permit.finish_stage(index, false)?;
                return Err(error);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCompletionV1 {
    Clean,
    HardTimeout,
}

pub struct ControlledExitContext<'a> {
    pub admission: OperationAdmission,
    pub owned_operation_timeout: Duration,
    pub process_cleanup_grace: Duration,
    pub total_timeout: Duration,
    pub before_owned_wait: Box<dyn FnMut() -> Result<(), String> + 'a>,
    pub write_marker: Box<dyn FnMut() -> Result<(), String> + 'a>,
    pub stop_worker: Box<dyn FnMut() -> Result<(), String> + 'a>,
    pub cancel_running: Box<dyn FnMut() -> Result<(), String> + 'a>,
    pub wait_runtime: Box<dyn FnMut(Duration) -> Result<bool, String> + 'a>,
    pub force_process_cleanup: Box<dyn FnMut() -> Result<(), String> + 'a>,
    pub sweep_queued: Box<dyn FnMut() -> Result<(), String> + 'a>,
    pub assert_idle: Box<dyn FnMut() -> Result<(), String> + 'a>,
    pub cleanup_task_files: Box<dyn FnMut() -> Result<(), String> + 'a>,
    pub remove_marker: Box<dyn FnMut() -> Result<(), String> + 'a>,
    pub best_effort_cleanup: Box<dyn FnMut() -> Result<(), String> + 'a>,
}

impl ControlledExitContext<'_> {
    pub fn new(admission: OperationAdmission) -> Self {
        Self {
            admission,
            owned_operation_timeout: Duration::from_secs(2),
            process_cleanup_grace: Duration::from_secs(2),
            total_timeout: Duration::from_secs(10),
            before_owned_wait: Box::new(|| Ok(())),
            write_marker: Box::new(|| Ok(())),
            stop_worker: Box::new(|| Ok(())),
            cancel_running: Box::new(|| Ok(())),
            wait_runtime: Box::new(|_| Ok(true)),
            force_process_cleanup: Box::new(|| Ok(())),
            sweep_queued: Box::new(|| Ok(())),
            assert_idle: Box::new(|| Ok(())),
            cleanup_task_files: Box::new(|| Ok(())),
            remove_marker: Box::new(|| Ok(())),
            best_effort_cleanup: Box::new(|| Ok(())),
        }
    }
}

pub fn abort_exit_before_marker(exit: ExitOperation, _error: &str) -> Result<(), String> {
    exit.abort()
}

pub fn request_app_exit(
    context: &mut ControlledExitContext<'_>,
) -> Result<ExitCompletionV1, String> {
    let started = Instant::now();
    let exit = context.admission.begin_exit()?;
    exit.request_owned_cancellation()?;
    let prepare_timeout = context
        .total_timeout
        .saturating_sub(started.elapsed())
        .min(context.owned_operation_timeout);
    if let Err(error) = exit.wait_until_prepared(prepare_timeout) {
        abort_exit_before_marker(exit, &error)?;
        return Err(error);
    }
    if let Err(error) = (context.before_owned_wait)() {
        abort_exit_before_marker(exit, &error)?;
        return Err(error);
    }
    if let Err(error) = (context.write_marker)() {
        abort_exit_before_marker(exit, &error)?;
        return Err(error);
    }
    exit.commit_marker()?;

    if (context.stop_worker)().is_err() || (context.cancel_running)().is_err() {
        (context.best_effort_cleanup)()?;
        return Ok(ExitCompletionV1::HardTimeout);
    }
    let graceful_wait = context
        .total_timeout
        .saturating_sub(started.elapsed())
        .min(context.process_cleanup_grace);
    if !graceful_wait.is_zero() && matches!((context.wait_runtime)(graceful_wait), Ok(true)) {
        // Runtime reached a terminal state during the cooperative grace period.
    } else if (context.force_process_cleanup)().is_err() {
        (context.best_effort_cleanup)()?;
        return Ok(ExitCompletionV1::HardTimeout);
    } else {
        let remaining = context.total_timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() || !matches!((context.wait_runtime)(remaining), Ok(true)) {
            (context.best_effort_cleanup)()?;
            return Ok(ExitCompletionV1::HardTimeout);
        }
    }
    if (context.sweep_queued)().is_err()
        || (context.assert_idle)().is_err()
        || (context.cleanup_task_files)().is_err()
        || (context.remove_marker)().is_err()
    {
        (context.best_effort_cleanup)()?;
        return Ok(ExitCompletionV1::HardTimeout);
    }
    Ok(ExitCompletionV1::Clean)
}

#[derive(Debug, Default, Clone)]
pub struct BackgroundTaskRegistry {
    inner: Arc<BackgroundTaskRegistryInner>,
}

#[derive(Debug, Default)]
struct BackgroundTaskRegistryInner {
    state: Mutex<BackgroundTaskRegistryState>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct BackgroundTaskRegistryState {
    active: usize,
    closed: bool,
}

impl BackgroundTaskRegistry {
    pub fn register(&self) -> Result<BackgroundTaskRegistration, String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "background_task_registry_poisoned".to_string())?;
        if state.closed {
            return Err("background_task_registry_closed".to_string());
        }
        state.active = state
            .active
            .checked_add(1)
            .ok_or_else(|| "background_task_registry_overflow".to_string())?;
        Ok(BackgroundTaskRegistration {
            inner: Some(Arc::clone(&self.inner)),
        })
    }

    pub fn active_count(&self) -> Result<usize, String> {
        self.inner
            .state
            .lock()
            .map(|state| state.active)
            .map_err(|_| "background_task_registry_poisoned".to_string())
    }

    pub fn close(&self) -> Result<(), String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "background_task_registry_poisoned".to_string())?;
        state.closed = true;
        self.inner.changed.notify_all();
        Ok(())
    }

    pub fn wait_until_idle(&self, timeout: Duration) -> Result<bool, String> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "background_task_registry_poisoned".to_string())?;
        let (state, wait) = self
            .inner
            .changed
            .wait_timeout_while(state, timeout, |state| state.active > 0)
            .map_err(|_| "background_task_registry_poisoned".to_string())?;
        Ok(state.active == 0 && !wait.timed_out())
    }
}

pub struct BackgroundTaskRegistration {
    inner: Option<Arc<BackgroundTaskRegistryInner>>,
}

impl Drop for BackgroundTaskRegistration {
    fn drop(&mut self) {
        let Some(inner) = self.inner.take() else {
            return;
        };
        let Ok(mut state) = inner.state.lock() else {
            return;
        };
        state.active = state.active.saturating_sub(1);
        inner.changed.notify_all();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitRequestDecisionV1 {
    PreventAndStart,
    Prevent,
    Allow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ExitRequestStateV1 {
    #[default]
    Idle,
    Cleaning,
    Complete,
}

#[derive(Debug, Default)]
pub struct ExitRequestGate {
    state: Mutex<ExitRequestStateV1>,
}

impl ExitRequestGate {
    pub fn on_exit_requested(
        &self,
        normal_runtime_ready: bool,
    ) -> Result<ExitRequestDecisionV1, String> {
        if !normal_runtime_ready {
            return Ok(ExitRequestDecisionV1::Allow);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "exit_request_state_poisoned".to_string())?;
        match *state {
            ExitRequestStateV1::Idle => {
                *state = ExitRequestStateV1::Cleaning;
                Ok(ExitRequestDecisionV1::PreventAndStart)
            }
            ExitRequestStateV1::Cleaning => Ok(ExitRequestDecisionV1::Prevent),
            ExitRequestStateV1::Complete => Ok(ExitRequestDecisionV1::Allow),
        }
    }

    pub fn mark_cleanup_complete(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "exit_request_state_poisoned".to_string())?;
        if *state != ExitRequestStateV1::Cleaning {
            return Err("exit_request_state_mismatch".to_string());
        }
        *state = ExitRequestStateV1::Complete;
        Ok(())
    }

    pub fn abort_request(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "exit_request_state_poisoned".to_string())?;
        if *state != ExitRequestStateV1::Cleaning {
            return Err("exit_request_state_mismatch".to_string());
        }
        *state = ExitRequestStateV1::Idle;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupWindowDecisionV1 {
    ShowMain,
    KeepHidden,
    ShowRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupWindowDecisionInputV1 {
    pub recovery_blocked: bool,
    pub autostart: bool,
    pub silent_autostart: bool,
    pub onboarding_completed: bool,
    pub tray_available: bool,
}

pub fn startup_window_decision(input: StartupWindowDecisionInputV1) -> StartupWindowDecisionV1 {
    if input.recovery_blocked {
        StartupWindowDecisionV1::ShowRecovery
    } else if input.autostart
        && input.silent_autostart
        && input.onboarding_completed
        && input.tray_available
    {
        StartupWindowDecisionV1::KeepHidden
    } else {
        StartupWindowDecisionV1::ShowMain
    }
}

#[derive(Debug, Default, Clone)]
pub struct StartupActionLog {
    entries: Arc<Mutex<Vec<StartupStageV1>>>,
}

impl StartupActionLog {
    pub fn record(&self, stage: StartupStageV1) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(stage);
        }
    }

    pub fn entries(&self) -> Vec<StartupStageV1> {
        self.entries
            .lock()
            .map(|entries| entries.clone())
            .unwrap_or_default()
    }
}
