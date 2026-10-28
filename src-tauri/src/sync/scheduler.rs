use super::{
    automatic_config_hash, decide_enqueue_for_task_key_tx, deserialize_required_option,
    freeze_task_payload, AutomaticScheduleConfigV1, AutomaticSyncConfigV1,
    BrowserExecutionIdentityV1, SourceCapabilityViewV1, SyncJobStatusV1, SyncSourceV1,
    SyncTriggerV1, TaskDecisionDispositionV1, TaskDecisionInputV1, TaskKeyV1, TerminalAnchorV1,
};
use crate::{
    accounts::portable_account_key_for_id,
    device_settings::BrowserPreferenceV1,
    settings_store::{
        account_settings_for, load_portable_settings, AccountSettingsV2, ArchiveListRangeV1,
        ArchiveSourceSettingsV1, PortableSettingsV2, RemoteFolderScheduleModeV1, SourceScheduleV1,
    },
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration as StdDuration,
};

const SCHEDULER_CURSOR_SCHEMA_VERSION: u32 = 1;
const SCHEDULER_CURSOR_DOMAIN: &[u8] = b"emily.scheduler-instances-cursor.v1\0";
const MAX_REMOTE_FOLDER_INSTANCES: usize = 2_000;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SchedulerWorkerStateV1 {
    Idle,
    Waiting,
    Running,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerWaitOutcomeV1 {
    Woken,
    TimedOut,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerTickResultV1 {
    pub paused: bool,
    pub next_wake_at: Option<String>,
    pub queued_job_ids: Vec<i64>,
}

pub struct SchedulerWorkerConfigV1 {
    idle_wait: StdDuration,
    tick: Arc<dyn Fn() -> Result<SchedulerTickResultV1, String> + Send + Sync>,
}

impl SchedulerWorkerConfigV1 {
    pub fn new(
        idle_wait: StdDuration,
        tick: Arc<dyn Fn() -> Result<SchedulerTickResultV1, String> + Send + Sync>,
    ) -> Self {
        Self {
            idle_wait: idle_wait.max(StdDuration::from_millis(10)),
            tick,
        }
    }
}

pub struct SchedulerWorkerHandle {
    runtime: SchedulerRuntimeStateV1,
    join: Mutex<Option<JoinHandle<Result<(), String>>>>,
}

impl SchedulerWorkerHandle {
    pub fn start(config: SchedulerWorkerConfigV1) -> Result<Self, String> {
        let runtime = SchedulerRuntimeStateV1::new();
        let worker_runtime = runtime.clone();
        let join = thread::Builder::new()
            .name("emily-scheduler-worker".to_string())
            .spawn(move || scheduler_worker_loop(config, worker_runtime))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            runtime,
            join: Mutex::new(Some(join)),
        })
    }

    pub fn snapshot(&self) -> Result<SchedulerRuntimeSnapshotV1, String> {
        self.runtime.snapshot()
    }

    pub fn wake(&self) -> Result<(), String> {
        self.runtime.wake()
    }

    pub fn stop(&self) -> Result<(), String> {
        self.runtime.stop()
    }

    pub fn wait_until_stopped(&self, timeout: StdDuration) -> Result<bool, String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let finished = self
                .join
                .lock()
                .map_err(|_| "scheduler_worker_join_poisoned".to_string())?
                .as_ref()
                .is_none_or(JoinHandle::is_finished);
            if finished {
                return Ok(true);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            thread::sleep((deadline - now).min(StdDuration::from_millis(10)));
        }
    }

    pub fn join(&self) -> Result<(), String> {
        let join = self
            .join
            .lock()
            .map_err(|_| "scheduler_worker_join_poisoned".to_string())?
            .take();
        match join {
            Some(join) => join
                .join()
                .map_err(|_| "scheduler_worker_panicked".to_string())?,
            None => Ok(()),
        }
    }
}

impl Drop for SchedulerWorkerHandle {
    fn drop(&mut self) {
        let _ = self.stop();
        let _ = self.join();
    }
}

fn scheduler_worker_loop(
    config: SchedulerWorkerConfigV1,
    runtime: SchedulerRuntimeStateV1,
) -> Result<(), String> {
    let mut observed_generation = runtime.take_wake_generation()?;
    loop {
        runtime.mark_running()?;
        let wait_duration = match (config.tick)() {
            Ok(result) if result.paused => {
                runtime.mark_paused()?;
                config.idle_wait
            }
            Ok(result) => {
                let wait_duration = scheduler_wait_duration(
                    result.next_wake_at.as_deref(),
                    config.idle_wait,
                    Utc::now(),
                );
                runtime.mark_waiting(result.next_wake_at)?;
                wait_duration
            }
            Err(_) => {
                eprintln!("scheduler_tick_failed");
                runtime.mark_waiting(None)?;
                config.idle_wait
            }
        };
        match runtime.wait_for_change(observed_generation, wait_duration)? {
            SchedulerWaitOutcomeV1::Stopped => return Ok(()),
            SchedulerWaitOutcomeV1::Woken | SchedulerWaitOutcomeV1::TimedOut => {
                observed_generation = runtime.take_wake_generation()?;
            }
        }
    }
}

fn scheduler_wait_duration(
    next_wake_at: Option<&str>,
    maximum: StdDuration,
    now: DateTime<Utc>,
) -> StdDuration {
    let Some(next_wake_at) = next_wake_at else {
        return maximum;
    };
    let Ok(next_wake_at) = DateTime::parse_from_rfc3339(next_wake_at) else {
        return maximum;
    };
    let remaining_ms = next_wake_at
        .with_timezone(&Utc)
        .signed_duration_since(now)
        .num_milliseconds();
    if remaining_ms <= 0 {
        StdDuration::from_millis(10).min(maximum)
    } else {
        StdDuration::from_millis(remaining_ms as u64)
            .max(StdDuration::from_millis(10))
            .min(maximum)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulerRuntimeSnapshotV1 {
    pub worker_state: SchedulerWorkerStateV1,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub next_wake_at: Option<String>,
    pub wake_generation: u64,
}

#[derive(Debug, Clone)]
pub struct SchedulerRuntimeStateV1 {
    inner: Arc<SchedulerRuntimeInnerV1>,
}

#[derive(Debug)]
struct SchedulerRuntimeInnerV1 {
    state: Mutex<SchedulerRuntimeSnapshotV1>,
    changed: Condvar,
}

impl Default for SchedulerRuntimeStateV1 {
    fn default() -> Self {
        Self::new()
    }
}

impl SchedulerRuntimeStateV1 {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SchedulerRuntimeInnerV1 {
                state: Mutex::new(SchedulerRuntimeSnapshotV1 {
                    worker_state: SchedulerWorkerStateV1::Idle,
                    next_wake_at: None,
                    wake_generation: 0,
                }),
                changed: Condvar::new(),
            }),
        }
    }

    pub fn snapshot(&self) -> Result<SchedulerRuntimeSnapshotV1, String> {
        self.lock().map(|state| state.clone())
    }

    pub fn mark_idle(&self) -> Result<(), String> {
        self.update_state(SchedulerWorkerStateV1::Idle, None)
    }

    pub fn mark_waiting(&self, next_wake_at: Option<String>) -> Result<(), String> {
        self.update_state(SchedulerWorkerStateV1::Waiting, next_wake_at)
    }

    pub fn mark_running(&self) -> Result<(), String> {
        self.update_state(SchedulerWorkerStateV1::Running, None)
    }

    pub fn mark_paused(&self) -> Result<(), String> {
        self.update_state(SchedulerWorkerStateV1::Paused, None)
    }

    pub fn wake(&self) -> Result<(), String> {
        let mut state = self.lock()?;
        state.wake_generation = state
            .wake_generation
            .checked_add(1)
            .ok_or_else(|| "scheduler_wake_generation_overflow".to_string())?;
        self.inner.changed.notify_all();
        Ok(())
    }

    pub fn take_wake_generation(&self) -> Result<u64, String> {
        self.lock().map(|state| state.wake_generation)
    }

    pub fn wait_for_change(
        &self,
        observed_generation: u64,
        timeout: StdDuration,
    ) -> Result<SchedulerWaitOutcomeV1, String> {
        let state = self.lock()?;
        if state.worker_state == SchedulerWorkerStateV1::Stopped {
            return Ok(SchedulerWaitOutcomeV1::Stopped);
        }
        if state.wake_generation != observed_generation {
            return Ok(SchedulerWaitOutcomeV1::Woken);
        }
        let (state, wait) = self
            .inner
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.worker_state != SchedulerWorkerStateV1::Stopped
                    && state.wake_generation == observed_generation
            })
            .map_err(|_| "scheduler_runtime_lock_poisoned".to_string())?;
        if state.worker_state == SchedulerWorkerStateV1::Stopped {
            Ok(SchedulerWaitOutcomeV1::Stopped)
        } else if state.wake_generation != observed_generation {
            Ok(SchedulerWaitOutcomeV1::Woken)
        } else if wait.timed_out() {
            Ok(SchedulerWaitOutcomeV1::TimedOut)
        } else {
            Ok(SchedulerWaitOutcomeV1::Woken)
        }
    }

    pub fn stop(&self) -> Result<(), String> {
        {
            let mut state = self.lock()?;
            state.worker_state = SchedulerWorkerStateV1::Stopped;
            state.next_wake_at = None;
            state.wake_generation = state
                .wake_generation
                .checked_add(1)
                .ok_or_else(|| "scheduler_wake_generation_overflow".to_string())?;
        }
        self.inner.changed.notify_all();
        Ok(())
    }

    fn update_state(
        &self,
        worker_state: SchedulerWorkerStateV1,
        next_wake_at: Option<String>,
    ) -> Result<(), String> {
        let mut state = self.lock()?;
        if state.worker_state == SchedulerWorkerStateV1::Stopped {
            return Err("scheduler_stopped".to_string());
        }
        state.worker_state = worker_state;
        state.next_wake_at = next_wake_at;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SchedulerRuntimeSnapshotV1>, String> {
        self.inner
            .state
            .lock()
            .map_err(|_| "scheduler_runtime_lock_poisoned".to_string())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulerSourceStateV1 {
    pub source: SyncSourceV1,
    pub available: bool,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub unavailable_reason: Option<String>,
    pub enabled: bool,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub interval_minutes: Option<u32>,
    pub instance_count: u32,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub aggregate_error: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub message: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub last_status: Option<SyncJobStatusV1>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub last_finished_at: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub next_due_at: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub active_job_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulerAccountStateV1 {
    pub account_id: i64,
    pub account_status: String,
    pub source_states: Vec<SchedulerSourceStateV1>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulerStateV1 {
    pub paused: bool,
    pub worker_state: SchedulerWorkerStateV1,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub next_wake_at: Option<String>,
    pub capabilities: Vec<SourceCapabilityViewV1>,
    pub accounts: Vec<SchedulerAccountStateV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedSourceInstanceV1 {
    pub task_key: TaskKeyV1,
    pub schedule: SourceScheduleV1,
    pub automatic_config: AutomaticSyncConfigV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedAccountSourcesV1 {
    pub source_states: Vec<SchedulerSourceStateV1>,
    pub instances: Vec<ExpandedSourceInstanceV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerAccountInputV1 {
    pub account_id: i64,
    pub account_status: String,
    pub account_settings: AccountSettingsV2,
    pub browser_preference: BrowserPreferenceV1,
}

#[derive(Debug, Clone)]
pub struct AutomaticReconcileInputV1 {
    pub account_id: i64,
    pub settings_revision: u64,
    pub account_settings: AccountSettingsV2,
    pub browser_preference: BrowserPreferenceV1,
    pub browser: BrowserExecutionIdentityV1,
    pub capabilities: Vec<SourceCapabilityViewV1>,
    pub app_ready_at: DateTime<Utc>,
    pub startup_delay_minutes: u32,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomaticReconcileResultV1 {
    pub created_job_ids: Vec<i64>,
    pub cancelled_job_ids: Vec<i64>,
    pub next_wake_at: Option<String>,
    pub source_states: Vec<SchedulerSourceStateV1>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulerInstanceViewV1 {
    pub remote_folder_id: String,
    pub name: String,
    pub range: ArchiveListRangeV1,
    pub schedule: SourceScheduleV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulerInstancesPageV1 {
    pub account_id: i64,
    pub source: SyncSourceV1,
    pub portable_settings_revision: u64,
    pub registry_revision: i64,
    pub instances: Vec<SchedulerInstanceViewV1>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn build_scheduler_state(
    connection: &Connection,
    paused: bool,
    runtime: &SchedulerRuntimeSnapshotV1,
    capabilities: &[SourceCapabilityViewV1],
    accounts: &[SchedulerAccountInputV1],
    app_ready_at: DateTime<Utc>,
    startup_delay_minutes: u32,
) -> Result<SchedulerStateV1, String> {
    let capability_map = ordered_capabilities(capabilities)?;
    let ordered_capabilities = all_sources()
        .into_iter()
        .map(|source| {
            capability_map
                .get(&source)
                .cloned()
                .ok_or_else(|| "invalid_capability_registry".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut account_states = Vec::with_capacity(accounts.len());
    for account in accounts {
        if account.account_id <= 0 || !valid_account_status(&account.account_status) {
            return Err("invalid_scheduler_account".to_string());
        }
        let remote_folder_ids = active_remote_folder_ids(connection, account.account_id)?;
        let mut expansion = expand_source_instances(
            account.account_id,
            &account.account_settings,
            account.browser_preference,
            &ordered_capabilities,
            &remote_folder_ids,
        )?;
        populate_source_states(
            connection,
            account.account_id,
            app_ready_at,
            startup_delay_minutes,
            &mut expansion.source_states,
        )?;
        account_states.push(SchedulerAccountStateV1 {
            account_id: account.account_id,
            account_status: account.account_status.clone(),
            source_states: expansion.source_states,
        });
    }
    account_states.sort_by_key(|account| account.account_id);
    Ok(SchedulerStateV1 {
        paused,
        worker_state: runtime.worker_state,
        next_wake_at: runtime.next_wake_at.clone(),
        capabilities: ordered_capabilities,
        accounts: account_states,
    })
}

pub fn calculate_next_due(
    last_terminal: Option<&TerminalAnchorV1>,
    app_ready_at: DateTime<Utc>,
    startup_delay_minutes: u32,
    interval_minutes: u32,
) -> Result<DateTime<Utc>, String> {
    if startup_delay_minutes > 120 || !(15..=43_200).contains(&interval_minutes) {
        return Err("invalid_scheduler_interval".to_string());
    }
    let (anchor, minutes) = match last_terminal {
        Some(anchor) => (
            DateTime::parse_from_rfc3339(&anchor.finished_at)
                .map_err(|_| "invalid_sync_timestamp".to_string())?
                .with_timezone(&Utc),
            interval_minutes,
        ),
        None => (app_ready_at, startup_delay_minutes),
    };
    anchor
        .checked_add_signed(Duration::minutes(i64::from(minutes)))
        .ok_or_else(|| "scheduler_time_overflow".to_string())
}

pub fn expand_source_instances(
    account_id: i64,
    account: &AccountSettingsV2,
    browser_preference: BrowserPreferenceV1,
    capabilities: &[SourceCapabilityViewV1],
    remote_folder_ids: &[String],
) -> Result<ExpandedAccountSourcesV1, String> {
    if account_id <= 0 {
        return Err("account_not_active".to_string());
    }
    let capabilities = ordered_capabilities(capabilities)?;
    let supports_remote_folders = capabilities
        .get(&SyncSourceV1::Favorites)
        .is_some_and(|value| value.available && value.supports_remote_folders)
        && capabilities
            .get(&SyncSourceV1::RemoteFolder)
            .is_some_and(|value| value.available);
    let mut folder_ids = remote_folder_ids.to_vec();
    folder_ids.sort();
    folder_ids.dedup();
    for folder_id in &folder_ids {
        validate_remote_folder_id(folder_id)?;
    }

    let mut source_states = Vec::with_capacity(4);
    let mut instances = Vec::new();
    for source in all_sources() {
        let capability = capabilities
            .get(&source)
            .expect("ordered capability registry");
        let (enabled, interval_minutes, instance_count, aggregate_error, message) = match source {
            SyncSourceV1::History => (
                account.history.schedule.enabled,
                Some(account.history.schedule.interval_minutes),
                1,
                None,
                None,
            ),
            SyncSourceV1::Likes => (
                account.likes.schedule.enabled,
                Some(account.likes.schedule.interval_minutes),
                1,
                None,
                None,
            ),
            SyncSourceV1::Favorites => (
                account.favorites.archive.schedule.enabled,
                Some(account.favorites.archive.schedule.interval_minutes),
                1,
                None,
                None,
            ),
            SyncSourceV1::RemoteFolder => {
                let over_limit = folder_ids.len() > MAX_REMOTE_FOLDER_INSTANCES;
                let enabled = !over_limit
                    && folder_ids.iter().any(|folder_id| {
                        effective_remote_folder_settings(account, folder_id)
                            .is_ok_and(|settings| settings.schedule.enabled)
                    });
                let interval = (account.favorites.remote_folder_mode
                    == RemoteFolderScheduleModeV1::Unified)
                    .then_some(
                        account
                            .favorites
                            .remote_folder_defaults
                            .schedule
                            .interval_minutes,
                    );
                (
                    enabled,
                    interval,
                    u32::try_from(folder_ids.len())
                        .map_err(|_| "remote_folder_schedule_limit_exceeded".to_string())?,
                    over_limit.then(|| "remote_folder_schedule_limit_exceeded".to_string()),
                    (supports_remote_folders
                        && !folder_ids.is_empty()
                        && !account.favorites.archive.schedule.enabled)
                        .then(|| "favorites_automatic_discovery_disabled".to_string()),
                )
            }
        };
        source_states.push(SchedulerSourceStateV1 {
            source,
            available: capability.available,
            unavailable_reason: capability.unavailable_reason.clone(),
            enabled,
            interval_minutes,
            instance_count,
            aggregate_error,
            message,
            last_status: None,
            last_finished_at: None,
            next_due_at: None,
            active_job_id: None,
        });
        if !capability.available {
            continue;
        }
        match source {
            SyncSourceV1::History => {
                instances.push(expanded_history(account_id, account, browser_preference)?)
            }
            SyncSourceV1::Likes => {
                instances.push(expanded_likes(account_id, account, browser_preference)?)
            }
            SyncSourceV1::Favorites => instances.push(expanded_favorites(
                account_id,
                account,
                browser_preference,
                supports_remote_folders,
            )?),
            SyncSourceV1::RemoteFolder if folder_ids.len() <= MAX_REMOTE_FOLDER_INSTANCES => {
                for folder_id in &folder_ids {
                    instances.push(expanded_remote_folder(
                        account_id,
                        account,
                        browser_preference,
                        folder_id,
                    )?);
                }
            }
            SyncSourceV1::RemoteFolder => {}
        }
    }
    instances.sort_by(|left, right| left.task_key.cmp(&right.task_key));
    Ok(ExpandedAccountSourcesV1 {
        source_states,
        instances,
    })
}

pub fn reconcile_automatic_jobs(
    connection: &mut Connection,
    input: &AutomaticReconcileInputV1,
) -> Result<AutomaticReconcileResultV1, String> {
    if input.browser.preference != input.browser_preference {
        return Err("browser_preference_mismatch".to_string());
    }
    let settings_revision = i64::try_from(input.settings_revision)
        .map_err(|_| "settings_revision_overflow".to_string())?;
    let mut expansion = expand_source_instances(
        input.account_id,
        &input.account_settings,
        input.browser_preference,
        &input.capabilities,
        &[],
    )?;
    let base_candidates = expansion
        .instances
        .iter()
        .map(|instance| (instance.task_key.clone(), instance))
        .collect::<BTreeMap<_, _>>();
    let queued_at = input.now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    ensure_active_account(&transaction, input.account_id)?;

    let remote_folder_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM remote_folders
             WHERE account_id = ?1 AND remote_removed_at IS NULL",
            [input.account_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let remote_folder_count = usize::try_from(remote_folder_count)
        .map_err(|_| "remote_folder_schedule_limit_exceeded".to_string())?;
    let capability_map = ordered_capabilities(&input.capabilities)?;
    let remote_available = capability_map
        .get(&SyncSourceV1::RemoteFolder)
        .is_some_and(|capability| capability.available);
    let supports_remote_folders = capability_map
        .get(&SyncSourceV1::Favorites)
        .is_some_and(|capability| capability.available && capability.supports_remote_folders)
        && remote_available;
    let remote_state = expansion
        .source_states
        .iter_mut()
        .find(|state| state.source == SyncSourceV1::RemoteFolder)
        .expect("remote source state");
    remote_state.instance_count = u32::try_from(remote_folder_count)
        .map_err(|_| "remote_folder_schedule_limit_exceeded".to_string())?;
    remote_state.aggregate_error = (remote_folder_count > MAX_REMOTE_FOLDER_INSTANCES)
        .then(|| "remote_folder_schedule_limit_exceeded".to_string());
    remote_state.message = (supports_remote_folders
        && remote_folder_count > 0
        && !input.account_settings.favorites.archive.schedule.enabled)
        .then(|| "favorites_automatic_discovery_disabled".to_string());
    remote_state.enabled = false;

    let mut statement = transaction
        .prepare(
            "SELECT id, source, source_instance_key, config_sha256
             FROM sync_jobs
             WHERE account_id = ?1 AND trigger = 'automatic' AND status = 'queued'
             ORDER BY source, source_instance_key, id",
        )
        .map_err(|error| error.to_string())?;
    let queued_automatic = statement
        .query_map([input.account_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);

    let mut cancelled_job_ids = Vec::new();
    for (job_id, source, source_instance_key, stored_hash) in queued_automatic {
        let task_key = TaskKeyV1 {
            account_id: input.account_id,
            source: SyncSourceV1::from_sqlite(&source)?,
            source_instance_key,
        };
        let remote_candidate = if task_key.source == SyncSourceV1::RemoteFolder
            && remote_available
            && remote_folder_count <= MAX_REMOTE_FOLDER_INSTANCES
        {
            let exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM remote_folders
                       WHERE account_id = ?1 AND remote_folder_id = ?2
                         AND remote_removed_at IS NULL
                     )",
                    params![input.account_id, task_key.source_instance_key],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            exists
                .then(|| {
                    expanded_remote_folder(
                        input.account_id,
                        &input.account_settings,
                        input.browser_preference,
                        &task_key.source_instance_key,
                    )
                })
                .transpose()?
        } else {
            None
        };
        let candidate = if task_key.source == SyncSourceV1::RemoteFolder {
            remote_candidate.as_ref()
        } else {
            base_candidates.get(&task_key).copied()
        };
        let still_valid = candidate.is_some_and(|candidate| {
            candidate.schedule.enabled
                && automatic_config_hash(&candidate.automatic_config)
                    .ok()
                    .as_deref()
                    == stored_hash.as_deref()
        });
        if !still_valid {
            super::QueueStore::cancel_queued_job_tx(
                &transaction,
                job_id,
                &queued_at,
                "automatic_settings_changed",
            )?;
            cancelled_job_ids.push(job_id);
        }
    }

    let settings_snapshot = super::EnqueueSettingsSnapshotV1 {
        revision: input.settings_revision,
        accounts: BTreeMap::from([(input.account_id, input.account_settings.clone())]),
    };
    let device_snapshot = super::EnqueueDeviceSnapshotV1 {
        browsers: BTreeMap::from([(input.account_id, input.browser.clone())]),
    };
    let mut created_job_ids = Vec::new();
    let mut next_wake: Option<DateTime<Utc>> = None;
    for candidate in expansion.instances.iter() {
        reconcile_candidate_tx(
            &transaction,
            candidate,
            input,
            settings_revision,
            &settings_snapshot,
            &device_snapshot,
            &queued_at,
            &mut created_job_ids,
            &mut next_wake,
        )?;
    }
    if remote_available && remote_folder_count <= MAX_REMOTE_FOLDER_INSTANCES {
        let mut last_remote_folder_id = String::new();
        loop {
            let remote_folder_id = transaction
                .query_row(
                    "SELECT remote_folder_id FROM remote_folders
                     WHERE account_id = ?1 AND remote_removed_at IS NULL
                       AND remote_folder_id > ?2
                     ORDER BY remote_folder_id
                     LIMIT 1",
                    params![input.account_id, last_remote_folder_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            let Some(remote_folder_id) = remote_folder_id else {
                break;
            };
            let candidate = expanded_remote_folder(
                input.account_id,
                &input.account_settings,
                input.browser_preference,
                &remote_folder_id,
            )?;
            if candidate.schedule.enabled {
                remote_state.enabled = true;
            }
            reconcile_candidate_tx(
                &transaction,
                &candidate,
                input,
                settings_revision,
                &settings_snapshot,
                &device_snapshot,
                &queued_at,
                &mut created_job_ids,
                &mut next_wake,
            )?;
            last_remote_folder_id = remote_folder_id;
        }
    }
    populate_base_source_states_tx(&transaction, input, &mut expansion.source_states)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(AutomaticReconcileResultV1 {
        created_job_ids,
        cancelled_job_ids,
        next_wake_at: next_wake.map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true)),
        source_states: expansion.source_states,
    })
}

pub fn cancel_stale_automatic_jobs(
    connection: &mut Connection,
    account_id: i64,
    account_settings: &AccountSettingsV2,
    browser_preference: BrowserPreferenceV1,
    capabilities: &[SourceCapabilityViewV1],
    finished_at: &str,
) -> Result<Vec<i64>, String> {
    if account_id <= 0 {
        return Err("account_not_active".to_string());
    }
    DateTime::parse_from_rfc3339(finished_at).map_err(|_| "invalid_sync_timestamp".to_string())?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let remote_folder_ids = {
        let mut statement = transaction
            .prepare(
                "SELECT remote_folder_id FROM remote_folders
                 WHERE account_id = ?1 AND remote_removed_at IS NULL
                 ORDER BY remote_folder_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([account_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let expansion = expand_source_instances(
        account_id,
        account_settings,
        browser_preference,
        capabilities,
        &remote_folder_ids,
    )?;
    let current = expansion
        .instances
        .into_iter()
        .map(|instance| {
            let hash = automatic_config_hash(&instance.automatic_config)?;
            Ok((instance.task_key, (instance.schedule.enabled, hash)))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let queued = {
        let mut statement = transaction
            .prepare(
                "SELECT id, source, source_instance_key, config_sha256
                 FROM sync_jobs
                 WHERE account_id = ?1 AND trigger = 'automatic' AND status = 'queued'
                 ORDER BY id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([account_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let mut cancelled_job_ids = Vec::new();
    for (job_id, source, source_instance_key, stored_hash) in queued {
        let task_key = TaskKeyV1 {
            account_id,
            source: SyncSourceV1::from_sqlite(&source)?,
            source_instance_key,
        };
        let still_valid = current
            .get(&task_key)
            .is_some_and(|(enabled, hash)| *enabled && stored_hash.as_deref() == Some(hash));
        if !still_valid {
            super::QueueStore::cancel_queued_job_tx(
                &transaction,
                job_id,
                finished_at,
                "automatic_settings_changed",
            )?;
            cancelled_job_ids.push(job_id);
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(cancelled_job_ids)
}

pub fn current_automatic_source_instance(
    connection: &Connection,
    task_key: &TaskKeyV1,
    account_settings: &AccountSettingsV2,
    browser_preference: BrowserPreferenceV1,
    capabilities: &[SourceCapabilityViewV1],
) -> Result<Option<ExpandedSourceInstanceV1>, String> {
    let remote_folder_ids = if task_key.source == SyncSourceV1::RemoteFolder {
        connection
            .query_row(
                "SELECT remote_folder_id FROM remote_folders
                 WHERE account_id = ?1 AND remote_folder_id = ?2
                   AND remote_removed_at IS NULL",
                params![task_key.account_id, task_key.source_instance_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    Ok(expand_source_instances(
        task_key.account_id,
        account_settings,
        browser_preference,
        capabilities,
        &remote_folder_ids,
    )?
    .instances
    .into_iter()
    .find(|instance| instance.task_key == *task_key))
}

#[allow(clippy::too_many_arguments)]
fn reconcile_candidate_tx(
    transaction: &rusqlite::Transaction<'_>,
    candidate: &ExpandedSourceInstanceV1,
    input: &AutomaticReconcileInputV1,
    settings_revision: i64,
    settings_snapshot: &super::EnqueueSettingsSnapshotV1,
    device_snapshot: &super::EnqueueDeviceSnapshotV1,
    queued_at: &str,
    created_job_ids: &mut Vec<i64>,
    next_wake: &mut Option<DateTime<Utc>>,
) -> Result<(), String> {
    if !candidate.schedule.enabled
        || super::QueueStore::find_active_job_tx(transaction, &candidate.task_key)?.is_some()
    {
        return Ok(());
    }
    let anchor = super::QueueStore::latest_terminal_anchor(transaction, &candidate.task_key)?;
    let due = calculate_next_due(
        anchor.as_ref(),
        input.app_ready_at,
        input.startup_delay_minutes,
        candidate.schedule.interval_minutes,
    )?;
    if due > input.now {
        *next_wake = Some(next_wake.map_or(due, |current| current.min(due)));
        return Ok(());
    }
    let payload = super::enqueue::build_implicit_payload(
        input.account_id,
        candidate.task_key.source,
        &candidate.task_key.source_instance_key,
        settings_snapshot,
        device_snapshot,
        &input.capabilities,
        input.now.timestamp_millis(),
    )?;
    let decision = decide_enqueue_for_task_key_tx(
        transaction,
        &TaskDecisionInputV1 {
            task_key: candidate.task_key.clone(),
            trigger: SyncTriggerV1::Automatic,
            queued_at: queued_at.to_string(),
            scheduled_for: Some(due.to_rfc3339_opts(SecondsFormat::Millis, true)),
            settings_revision,
            config_sha256: Some(automatic_config_hash(&candidate.automatic_config)?),
            payload: freeze_task_payload(payload)?,
            batch_targets: Vec::new(),
        },
    )?;
    if matches!(
        decision.disposition,
        TaskDecisionDispositionV1::Created | TaskDecisionDispositionV1::Replaced
    ) {
        created_job_ids.push(
            decision
                .job
                .ok_or_else(|| "sync_enqueue_failed".to_string())?
                .snapshot
                .id,
        );
    }
    Ok(())
}

fn populate_base_source_states_tx(
    transaction: &rusqlite::Transaction<'_>,
    input: &AutomaticReconcileInputV1,
    states: &mut [SchedulerSourceStateV1],
) -> Result<(), String> {
    populate_source_states(
        transaction,
        input.account_id,
        input.app_ready_at,
        input.startup_delay_minutes,
        states,
    )
}

fn populate_source_states(
    connection: &Connection,
    account_id: i64,
    app_ready_at: DateTime<Utc>,
    startup_delay_minutes: u32,
    states: &mut [SchedulerSourceStateV1],
) -> Result<(), String> {
    for source in [
        SyncSourceV1::History,
        SyncSourceV1::Likes,
        SyncSourceV1::Favorites,
    ] {
        let state = states
            .iter_mut()
            .find(|state| state.source == source)
            .expect("base source state");
        let task_key = TaskKeyV1 {
            account_id,
            source,
            source_instance_key: String::new(),
        };
        let anchor = super::QueueStore::latest_terminal_anchor(connection, &task_key)?;
        state.last_status = anchor.as_ref().map(|anchor| anchor.status);
        state.last_finished_at = anchor.as_ref().map(|anchor| anchor.finished_at.clone());
        state.next_due_at = state
            .interval_minutes
            .map(|interval_minutes| {
                calculate_next_due(
                    anchor.as_ref(),
                    app_ready_at,
                    startup_delay_minutes,
                    interval_minutes,
                )
                .map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true))
            })
            .transpose()?;
        state.active_job_id =
            super::QueueStore::find_active_job(connection, &task_key)?.map(|job| job.snapshot.id);
    }
    Ok(())
}

fn active_remote_folder_ids(
    connection: &Connection,
    account_id: i64,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT remote_folder_id FROM remote_folders
             WHERE account_id = ?1 AND remote_removed_at IS NULL
             ORDER BY remote_folder_id",
        )
        .map_err(|error| error.to_string())?;
    let folder_ids = statement
        .query_map([account_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(folder_ids)
}

fn valid_account_status(value: &str) -> bool {
    matches!(value, "active" | "login_expired" | "identity_conflict")
}

pub fn list_scheduler_instances(
    connection: &mut Connection,
    settings_root: &Path,
    account_id: i64,
    source: SyncSourceV1,
    cursor: Option<&str>,
    limit: usize,
) -> Result<SchedulerInstancesPageV1, String> {
    list_scheduler_instances_with_settings_loader(
        connection,
        account_id,
        source,
        cursor,
        limit,
        || load_portable_settings(settings_root),
    )
}

pub fn list_scheduler_instances_with_settings_loader<F>(
    connection: &mut Connection,
    account_id: i64,
    source: SyncSourceV1,
    cursor: Option<&str>,
    limit: usize,
    mut load_settings: F,
) -> Result<SchedulerInstancesPageV1, String>
where
    F: FnMut() -> Result<PortableSettingsV2, String>,
{
    if account_id <= 0 || source != SyncSourceV1::RemoteFolder || !(1..=200).contains(&limit) {
        return Err("invalid_scheduler_instances_request".to_string());
    }
    let portable = load_settings()?;
    let portable_revision = portable.revision;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|error| error.to_string())?;
    let portable_key = portable_account_key_for_id(&transaction, account_id)?;
    let account = account_settings_for(&portable, Some(&portable_key))?;
    let registry_revision: i64 = transaction
        .query_row(
            "SELECT revision FROM remote_folder_registry_state WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "account_not_found".to_string())?;
    let last_folder_id = match cursor {
        Some(cursor) => {
            let decoded = decode_scheduler_cursor(cursor)?;
            if decoded.account_id != account_id
                || decoded.source != source
                || decoded.registry_revision != registry_revision
                || decoded.portable_settings_revision != portable_revision
            {
                return Err("scheduler_instances_changed".to_string());
            }
            decoded.last_remote_folder_id
        }
        None => String::new(),
    };
    let mut statement = transaction
        .prepare(
            "SELECT remote_folder_id, name
             FROM remote_folders
             WHERE account_id = ?1 AND remote_removed_at IS NULL
               AND remote_folder_id > ?2
             ORDER BY remote_folder_id
             LIMIT ?3",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![account_id, last_folder_id, (limit + 1) as i64],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    let has_more = rows.len() > limit;
    let mut instances = Vec::with_capacity(rows.len().min(limit));
    for (remote_folder_id, name) in rows.into_iter().take(limit) {
        validate_remote_folder_id(&remote_folder_id)?;
        let effective = effective_remote_folder_settings(&account, &remote_folder_id)?;
        instances.push(SchedulerInstanceViewV1 {
            remote_folder_id,
            name,
            range: effective.range,
            schedule: effective.schedule,
        });
    }
    transaction.commit().map_err(|error| error.to_string())?;
    if load_settings()?.revision != portable_revision {
        return Err("scheduler_instances_changed".to_string());
    }
    let next_cursor = if has_more {
        let last_remote_folder_id = instances
            .last()
            .ok_or_else(|| "invalid_scheduler_cursor".to_string())?
            .remote_folder_id
            .clone();
        Some(encode_scheduler_cursor(&SchedulerCursorPayloadV1 {
            schema_version: SCHEDULER_CURSOR_SCHEMA_VERSION,
            account_id,
            source,
            registry_revision,
            portable_settings_revision: portable_revision,
            last_remote_folder_id,
        })?)
    } else {
        None
    };
    Ok(SchedulerInstancesPageV1 {
        account_id,
        source,
        portable_settings_revision: portable_revision,
        registry_revision,
        instances,
        next_cursor,
        has_more,
    })
}

fn expanded_history(
    account_id: i64,
    account: &AccountSettingsV2,
    browser_preference: BrowserPreferenceV1,
) -> Result<ExpandedSourceInstanceV1, String> {
    expanded(
        TaskKeyV1 {
            account_id,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
        },
        account.history.schedule.clone(),
        AutomaticSyncConfigV1::History {
            schema_version: 1,
            source_instance_key: String::new(),
            schedule: automatic_schedule(&account.history.schedule),
            range: account.history.range,
            scroll_speed: account.scroll_speed,
            hide_browser: account.hide_browser,
            browser_preference,
        },
    )
}

fn expanded_likes(
    account_id: i64,
    account: &AccountSettingsV2,
    browser_preference: BrowserPreferenceV1,
) -> Result<ExpandedSourceInstanceV1, String> {
    expanded(
        TaskKeyV1 {
            account_id,
            source: SyncSourceV1::Likes,
            source_instance_key: String::new(),
        },
        account.likes.schedule.clone(),
        AutomaticSyncConfigV1::Likes {
            schema_version: 1,
            source_instance_key: String::new(),
            schedule: automatic_schedule(&account.likes.schedule),
            range: account.likes.range,
            run_mode: super::ArchiveRunModeV1::Incremental,
            scroll_speed: account.scroll_speed,
            hide_browser: account.hide_browser,
            browser_preference,
        },
    )
}

fn expanded_favorites(
    account_id: i64,
    account: &AccountSettingsV2,
    browser_preference: BrowserPreferenceV1,
    supports_remote_folders: bool,
) -> Result<ExpandedSourceInstanceV1, String> {
    expanded(
        TaskKeyV1 {
            account_id,
            source: SyncSourceV1::Favorites,
            source_instance_key: String::new(),
        },
        account.favorites.archive.schedule.clone(),
        AutomaticSyncConfigV1::Favorites {
            schema_version: 1,
            source_instance_key: String::new(),
            schedule: automatic_schedule(&account.favorites.archive.schedule),
            range: account.favorites.archive.range,
            run_mode: super::ArchiveRunModeV1::Incremental,
            scroll_speed: account.scroll_speed,
            hide_browser: account.hide_browser,
            browser_preference,
            discover_remote_folders: supports_remote_folders,
        },
    )
}

fn expanded_remote_folder(
    account_id: i64,
    account: &AccountSettingsV2,
    browser_preference: BrowserPreferenceV1,
    remote_folder_id: &str,
) -> Result<ExpandedSourceInstanceV1, String> {
    let effective = effective_remote_folder_settings(account, remote_folder_id)?;
    expanded(
        TaskKeyV1 {
            account_id,
            source: SyncSourceV1::RemoteFolder,
            source_instance_key: remote_folder_id.to_string(),
        },
        effective.schedule.clone(),
        AutomaticSyncConfigV1::RemoteFolder {
            schema_version: 1,
            source_instance_key: remote_folder_id.to_string(),
            schedule: automatic_schedule(&effective.schedule),
            range: effective.range,
            run_mode: super::ArchiveRunModeV1::Incremental,
            remote_folder_mode: account.favorites.remote_folder_mode,
            scroll_speed: account.scroll_speed,
            hide_browser: account.hide_browser,
            browser_preference,
        },
    )
}

fn expanded(
    task_key: TaskKeyV1,
    schedule: SourceScheduleV1,
    automatic_config: AutomaticSyncConfigV1,
) -> Result<ExpandedSourceInstanceV1, String> {
    automatic_config_hash(&automatic_config)?;
    Ok(ExpandedSourceInstanceV1 {
        task_key,
        schedule,
        automatic_config,
    })
}

fn automatic_schedule(schedule: &SourceScheduleV1) -> AutomaticScheduleConfigV1 {
    AutomaticScheduleConfigV1 {
        enabled: schedule.enabled,
        interval_minutes: schedule.interval_minutes,
    }
}

fn effective_remote_folder_settings(
    account: &AccountSettingsV2,
    remote_folder_id: &str,
) -> Result<ArchiveSourceSettingsV1, String> {
    validate_remote_folder_id(remote_folder_id)?;
    Ok(match account.favorites.remote_folder_mode {
        RemoteFolderScheduleModeV1::Unified => account.favorites.remote_folder_defaults.clone(),
        RemoteFolderScheduleModeV1::PerFolder => account
            .favorites
            .remote_folders
            .get(remote_folder_id)
            .cloned()
            .unwrap_or_else(|| account.favorites.remote_folder_defaults.clone()),
    })
}

fn ordered_capabilities(
    capabilities: &[SourceCapabilityViewV1],
) -> Result<BTreeMap<SyncSourceV1, SourceCapabilityViewV1>, String> {
    let mut ordered = BTreeMap::new();
    for capability in capabilities {
        if ordered
            .insert(capability.source, capability.clone())
            .is_some()
        {
            return Err("invalid_capability_registry".to_string());
        }
    }
    if all_sources()
        .iter()
        .any(|source| !ordered.contains_key(source))
    {
        return Err("invalid_capability_registry".to_string());
    }
    Ok(ordered)
}

fn all_sources() -> [SyncSourceV1; 4] {
    [
        SyncSourceV1::History,
        SyncSourceV1::Likes,
        SyncSourceV1::Favorites,
        SyncSourceV1::RemoteFolder,
    ]
}

fn ensure_active_account(connection: &Connection, account_id: i64) -> Result<(), String> {
    let active: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1 AND status = 'active')",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if active {
        Ok(())
    } else {
        Err("account_not_active".to_string())
    }
}

fn validate_remote_folder_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err("invalid_remote_folder_id".to_string())
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SchedulerCursorPayloadV1 {
    schema_version: u32,
    account_id: i64,
    source: SyncSourceV1,
    registry_revision: i64,
    portable_settings_revision: u64,
    last_remote_folder_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SchedulerCursorEnvelopeV1 {
    payload: SchedulerCursorPayloadV1,
    sha256: String,
}

fn encode_scheduler_cursor(payload: &SchedulerCursorPayloadV1) -> Result<String, String> {
    if payload.schema_version != SCHEDULER_CURSOR_SCHEMA_VERSION
        || payload.account_id <= 0
        || payload.source != SyncSourceV1::RemoteFolder
        || payload.registry_revision <= 0
        || payload.last_remote_folder_id.is_empty()
    {
        return Err("invalid_scheduler_cursor".to_string());
    }
    let payload_json = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    digest.update(SCHEDULER_CURSOR_DOMAIN);
    digest.update(&payload_json);
    let envelope = SchedulerCursorEnvelopeV1 {
        payload: payload.clone(),
        sha256: format!("{:x}", digest.finalize()),
    };
    let json = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
    Ok(base64url_encode(&json))
}

fn decode_scheduler_cursor(value: &str) -> Result<SchedulerCursorPayloadV1, String> {
    let decoded = base64url_decode(value).map_err(|_| "invalid_scheduler_cursor".to_string())?;
    if base64url_encode(&decoded) != value {
        return Err("invalid_scheduler_cursor".to_string());
    }
    let envelope = serde_json::from_slice::<SchedulerCursorEnvelopeV1>(&decoded)
        .map_err(|_| "invalid_scheduler_cursor".to_string())?;
    let encoded = encode_scheduler_cursor(&envelope.payload)?;
    if encoded != value {
        return Err("invalid_scheduler_cursor".to_string());
    }
    Ok(envelope.payload)
}

fn base64url_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity((bytes.len() * 4).div_ceil(3));
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let first = bytes[index];
        let second = bytes[index + 1];
        let third = bytes[index + 2];
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        output.push(TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
        output.push(TABLE[(third & 0x3f) as usize] as char);
        index += 3;
    }
    match bytes.len() - index {
        1 => {
            let first = bytes[index];
            output.push(TABLE[(first >> 2) as usize] as char);
            output.push(TABLE[((first & 0x03) << 4) as usize] as char);
        }
        2 => {
            let first = bytes[index];
            let second = bytes[index + 1];
            output.push(TABLE[(first >> 2) as usize] as char);
            output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
            output.push(TABLE[((second & 0x0f) << 2) as usize] as char);
        }
        _ => {}
    }
    output
}

fn base64url_decode(value: &str) -> Result<Vec<u8>, ()> {
    if value.is_empty() || value.len() % 4 == 1 {
        return Err(());
    }
    let values = value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' => Ok(byte - b'A'),
            b'a'..=b'z' => Ok(byte - b'a' + 26),
            b'0'..=b'9' => Ok(byte - b'0' + 52),
            b'-' => Ok(62),
            b'_' => Ok(63),
            _ => Err(()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Vec::with_capacity(values.len() * 3 / 4);
    for chunk in values.chunks(4) {
        if chunk.len() < 2 {
            return Err(());
        }
        output.push((chunk[0] << 2) | (chunk[1] >> 4));
        if chunk.len() >= 3 {
            output.push((chunk[1] << 4) | (chunk[2] >> 2));
        }
        if chunk.len() == 4 {
            output.push((chunk[2] << 6) | chunk[3]);
        }
    }
    Ok(output)
}
