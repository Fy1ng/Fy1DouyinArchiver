#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use chrono::{SecondsFormat, Utc};
use douyin_history_archive::accounts::{
    archive_account as archive_account_storage,
    cancel_pending_account as cancel_pending_account_storage, create_account_from_identity,
    delete_account_data, finalize_account_identity, finish_sync_with_account_state,
    get_account as get_account_storage, known_portable_account_keys, list_command_accounts,
    load_active_account as load_active_account_storage, portable_account_key_for_id,
    remove_unclaimed_profile, restore_account as restore_account_storage,
    set_active_account as set_active_account_storage, AccountFolderDisposition, AccountIdentity,
    AccountView, SyncJobIdentity, SyncTerminalOutcome,
};
use douyin_history_archive::autostart::{
    repair_autostart_on_startup, set_autostart_transactional_at_revision,
};
use douyin_history_archive::bootstrap::{
    read_bootstrap, validate_data_root_marker, DataRootRefV1, APPLICATION_ID,
};
use douyin_history_archive::collector::session_probe::{
    is_valid_douyin_home_url, ObservedHomeIdentityV1, DOUYIN_HISTORY_URL,
};
use douyin_history_archive::collector::{
    browser_process::{
        browser_identity_matches, browser_record_matches_expected_profile,
        local_debug_port_is_reachable, BrowserProcessIdentity, BrowserProcessRecordV2,
    },
    browser_runtime::BrowserChoice,
    browser_state::{
        clear_browser_selection, read_browser_selection, read_login_validation,
        write_browser_selection, BrowserId,
    },
    contracts::CollectorDiagnostic,
    file_store::write_json_atomically,
};
use douyin_history_archive::collector_process::{
    collector_process_identity_matches, collector_record_matches_expectation,
    login_collector_process_identity_matches, login_collector_record_matches_expectation,
    terminate_collector_if_matches, terminate_login_collector_if_matches,
    CollectorProcessExpectation, CollectorProcessIdentityV1, CollectorProcessObservation,
    LoginCollectorProcessExpectation, LoginCollectorProcessIdentityV1, OwnedCollectorProcess,
};
use douyin_history_archive::cover_cache::{
    clear_cache_rule_items as clear_cache_rule_items_storage,
    cleanup_broken_cache_entries, disable_cover_cache_queue,
    enqueue_all_missing_cover_candidates, ensure_pending_cache_task,
    execute_configured_cache_cleanup, inspect_cache, latest_cache_task,
    list_cache_rule_items as list_cache_rule_items_storage, pause_cache_task,
    preview_configured_cache_cleanup, read_cached_cover, remove_cache_rule_item,
    recover_interrupted_cache_tasks, request_cache_task_cancel, resume_cache_task,
    retry_cache_task,
    search_cache_rule_candidates as search_cache_rule_candidates_storage,
    add_cache_rule_item as add_cache_rule_item_storage, CacheCleanupModeV1,
    CacheCleanupPolicyV1, CacheCleanupPreviewV1, CacheCleanupResultV1,
    CacheInspectionV1, CacheRuleItemV1, CacheTaskKindV1, CacheTaskViewV1,
    CoverCacheWorkerConfigV1, CoverCacheWorkerHandle,
};
use douyin_history_archive::data_root::{
    delete_retained_data_root as delete_retained_data_root_storage, migrate_data_root,
    normalize_requested_root, probe_data_root_permissions, resolve_or_initialize_data_root,
    uninstall_managed_data, validate_data_root_request, DataRootContext, DataRootMigrationView,
    DataRootValidationView, MigrationHooks, OnboardingDataRootView,
};
use douyin_history_archive::desktop_ipc::{
    build_desktop_state_view, commit_onboarding_step_with_device_commit,
    update_account_defaults_for_capabilities, BeginAccountLoginRequestV1,
    CommitOnboardingStepRequestV1, CommitOnboardingStepResultV1, DesktopStateViewV1,
    OnboardingStepRequestV1, SetAutostartRequestV1, SetBrowserPreferenceRequestV1,
};
use douyin_history_archive::desktop_lifecycle::{
    close_window_decision, continue_desktop_startup_from_ready, recover_desktop_startup_once,
    second_instance_decision, startup_window_decision, tray_menu_decision, BackgroundTaskRegistry,
    CloseWindowDecisionInputV1, CloseWindowDecisionV1, DesktopIntentSnapshotV1, DesktopIntentState,
    DesktopStartupContext, ExitCompletionV1, ExitRequestDecisionV1, ExitRequestGate,
    GuiActivationListener, SecondInstanceDecisionV1, StartupStageV1, StartupWindowDecisionInputV1,
    StartupWindowDecisionV1, TrayMenuDecisionInputV1, TrayMenuItemIdV1,
};
use douyin_history_archive::device_settings::{
    initialize_device_settings_store, BrowserPreferenceV1, DeviceSettingsStore,
    DeviceSettingsViewV1,
};
use douyin_history_archive::download::{
    BrowserOperationCoordinator as DownloadBrowserOperationCoordinator, CreateDownloadJobRequest,
    DownloadError, DownloadJobDetail, DownloadJobOwner, DownloadJobSummary, DownloadPreview,
    DownloadQuality, DownloadRuntimeConfig, DownloadSelection, DownloadService,
};
use douyin_history_archive::login_attempt::{
    compare_and_swap_login_attempt, read_login_attempt_state, LoginAttemptEnvelopeV1,
    LoginAttemptLifecycle, LoginAttemptStateV1, LoginDiagnosticPayloadV1, LoginProgressPayloadV1,
};
use douyin_history_archive::login_watchdog::{LoginTermination, LoginWatchdog, LoginWatchdogEvent};
use douyin_history_archive::merge_service::{
    prepare_backup_merge as prepare_backup_merge_service,
    restore_backup_merge as restore_backup_merge_service, LocalFolderMergeResolutionV1,
    MergePlanRequestV1, MergePlanV1, MergeRestoreRequestV1, MergeRestoreResultV1,
    SettingsMergeResolutionV1,
};
use douyin_history_archive::notifications::{
    list_notifications as list_notifications_storage,
    mark_all_notifications_read as mark_all_notifications_read_storage,
    mark_notification_read as mark_notification_read_storage, notification_message,
    notification_insert_for_kind, observe_terminal_notification,
    resolve_notification as resolve_notification_storage, resolve_notification_by_dedupe,
    upsert_notification, NotificationKindV1, NotificationsPageV1, NotificationViewV1,
};
use douyin_history_archive::process_lock::{
    BootstrapProcessGuard, GuiActivationEvent, GuiInstanceGuard,
};
use douyin_history_archive::runtime_paths::{BackgroundPathLease, PathLease, RuntimePaths};
use douyin_history_archive::restore_service::{
    inspect_backup_for_restore, restore_backup_replace as restore_backup_replace_service,
    InspectRestoreRequestV1, ReplaceRestoreRequestV1, ReplaceRestoreResultV1,
};
use douyin_history_archive::settings_store::{
    account_settings_for, import_legacy_frontend_settings, load_app_settings_view,
    load_portable_settings, update_account_settings as update_account_settings_storage,
    update_global_settings as update_global_settings_storage, AccountScopeV1, AccountSettingsV2,
    AppSettingsView, CoverCacheBackfillModeV1,
    CoverCacheCleanupModeV1, GlobalSettingsV2, HistoryRangeV1, LegacyFrontendSettingsV1,
    NotificationSettingsV1, PortableSettingsV2, WorkspaceSettingsV1,
    update_workspace_settings as update_workspace_settings_storage, validate_account_scope,
};
use douyin_history_archive::startup_recovery::{
    remove_shutdown_intent, write_shutdown_intent, ProcessIdentityProbe, RunningJobRecovery,
    ShutdownIntentV1, StartupContinuationPermitV1, StartupRecoveryContext,
    StartupRecoveryDispositionV1, StartupRecoveryGate, StartupRecoveryStateV1,
    SystemProcessIdentityProbe,
};
use douyin_history_archive::sync::{
    browser_executable_path_sha256, build_archive_collect_invocation,
    build_archive_known_ids_snapshot, build_history_collect_invocation, build_scheduler_state,
    cancel_stale_automatic_jobs, cancel_sync_job, decode_execution_envelope, freeze_task_payload,
    import_execution_envelope, list_scheduler_instances as list_scheduler_instances_storage,
    reconcile_automatic_jobs, validate_execution_envelope, ArchiveResultImporter, ArchiveRunModeV1,
    ArchiveSyncExecutor, AutomaticReconcileInputV1, BrowserExecutionIdentityV1, BrowserIdV1,
    BrowserOperationKind, BrowserOperationLease, CapabilityRegistry, EnqueueDeviceSnapshotV1,
    EnqueueService, EnqueueSettingsSnapshotV1, EnqueueSyncRequestV1, EnqueueSyncResultV1,
    ExecutorRegistration, ExecutorRegistry, HistoryResultImporter, HistorySyncExecutor,
    OperationAdmission, QueueStore, QueuedSyncContext, RangeFreezeV1,
    RunningSyncIdentity as QueueRunningSyncIdentity, RuntimeSyncWorkerValidator,
    SchedulerAccountInputV1, SchedulerInstancesPageV1, SchedulerStateV1, SchedulerTickResultV1,
    SchedulerWorkerConfigV1, SchedulerWorkerHandle, StoredSyncJobV1, SyncBatchEntriesPageV1,
    SyncBatchEntryV1, SyncBatchStatusV1, SyncBatchViewV1, SyncExecutionEnvelopeV1,
    SyncJobStatusV1, SyncScopeV1, SyncSourceV1, SyncStatusV2, SyncTaskPayloadV1,
    SyncWorkerConfig, SyncWorkerHandle, SyncWorkerTerminalObserver,
};
use douyin_history_archive::sync_driver::{
    phase_completion_decision, prepare_next_phase, startup_recovery_decision,
    PhaseCompletionDecision, PhaseCompletionEvidence, StartupRecoveryDecision,
};
use douyin_history_archive::sync_orchestration::{
    cas_orchestration, create_orchestration, read_orchestration, request_orchestration_cancel,
    SyncLifecycle, SyncOrchestrationV1, SyncPhase, SyncPhaseEnvelopeV1,
};
use douyin_history_archive::uninstall::{
    compiled_product_identity, recover_uninstall_state_before_bootstrap,
    run_installer_control_with, run_package_manifest_builder, run_uninstall_guard,
    uninstall_coordination_dir, ExecutableIdentityV1, InstallerControlRuntime,
    InstallerGuardProcessV1, NativeInstallerControlRuntime, NativeUninstallSystem,
    ProductIdentityV1, UninstallSessionV1,
};
use douyin_history_archive::update_runtime::{NativeUpdateRuntime, UpdateStateViewV1};
use douyin_history_archive::{
    add_local_folder_item as add_local_folder_item_storage, assert_schema_latest, assert_schema_v5,
    build_login_invocation, build_recover_login_invocation, consume_schema_v5_recovery_work,
    create_local_folder as create_local_folder_storage, delete_history_by_ids,
    delete_history_by_query as delete_history_by_query_storage,
    delete_local_folder as delete_local_folder_storage, list_content as list_content_storage,
    list_history as list_history_storage, list_library_content as list_library_content_storage,
    list_local_folder_content as list_local_folder_content_storage,
    list_local_folders as list_local_folders_storage,
    list_remote_folder_content as list_remote_folder_content_storage,
    list_remote_folders as list_remote_folders_storage, manual_backup_file_name,
    migrate_schema_to_latest_at_startup, open_content_url as open_content_url_in_browser,
    probe_backup_directory, rebuild_schema_v5_cleanup_work, recover_login_attempts_with,
    remove_local_folder_item as remove_local_folder_item_storage,
    rename_local_folder as rename_local_folder_storage, run_automatic_backup_tick,
    run_backup_with_runtime, search_library_content as search_library_content_storage,
    set_local_folder_scope as set_local_folder_scope_storage, AppPaths,
    AutomaticBackupTickStatusV1, BackupArchiveSummaryV1, BrowserProcessInfo, CollectorInvocation,
    AnnualReviewQueryV1, AnnualReviewViewV1, ContentSortV1, DeleteHistoryResult,
    LegacyRunningJobIdentity, LibraryContentQueryV1, ListHistoryResult,
    ListLibraryContentResultV1, LocalFolderScopeV1, OverviewRangeV1, OverviewSourceFilterV1,
    OverviewViewV1, SchemaV5RecoveryWork, SearchContentQueryV1, SyncOptions,
};
use douyin_history_archive::task_center::{
    list_task_summaries as list_task_summaries_storage, TaskSummariesPageV1,
};
#[cfg(test)]
use rusqlite::params;
use rusqlite::{Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
#[cfg(test)]
use std::process::Child;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;
use uuid::Uuid;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const TRAY_ID: &str = "emily-main-tray";
const TRAY_MENU_OPEN_MAIN: &str = "tray-open-main";
const TRAY_MENU_SYNC_CURRENT: &str = "tray-sync-current";
const TRAY_MENU_SYNC_ALL: &str = "tray-sync-all";
const TRAY_MENU_PAUSE: &str = "tray-pause-automatic";
const TRAY_MENU_OPEN_SETTINGS: &str = "tray-open-settings";
const TRAY_MENU_EXIT: &str = "tray-exit";

#[derive(Clone)]
struct TrayMenuHandles {
    sync_current: tauri::menu::MenuItem<tauri::Wry>,
    sync_all: tauri::menu::MenuItem<tauri::Wry>,
    pause_automatic: tauri::menu::CheckMenuItem<tauri::Wry>,
}

#[derive(Default)]
struct AppState {
    running: Mutex<Option<RunningSync>>,
    running_login: Mutex<Option<RunningLogin>>,
    operation_admission: OperationAdmission,
    runtime_paths: OnceLock<RuntimePaths>,
    device_settings: OnceLock<DeviceSettingsStore>,
    app_ready_at: OnceLock<chrono::DateTime<Utc>>,
    desktop_settings_tx: Mutex<()>,
    autostart_error: Mutex<Option<String>>,
    bootstrap_write: Arc<Mutex<()>>,
    sync_registry: OnceLock<Arc<ExecutorRegistry>>,
    sync_worker: OnceLock<SyncWorkerHandle>,
    scheduler_worker: OnceLock<SchedulerWorkerHandle>,
    automatic_backup_worker: OnceLock<SchedulerWorkerHandle>,
    cover_cache_worker: OnceLock<CoverCacheWorkerHandle>,
    download_service: OnceLock<Arc<DownloadService>>,
    update_runtime: NativeUpdateRuntime,
    startup_recovery: StartupRecoveryGate,
    startup_continuation_permit: Mutex<Option<StartupContinuationPermitV1>>,
    background_tasks: BackgroundTaskRegistry,
    exit_request: ExitRequestGate,
    started_by_autostart: OnceLock<bool>,
    tray_available: AtomicBool,
    tray_menu: Mutex<Option<TrayMenuHandles>>,
    tray_error: Mutex<Option<String>>,
    desktop_intents: DesktopIntentState,
    activation_event: Mutex<Option<GuiActivationEvent>>,
    activation_listener: Mutex<Option<GuiActivationListener>>,
    normal_runtime_ready: AtomicBool,
    startup_error: Mutex<Option<String>>,
}

struct RuntimeTerminalObserver {
    app: AppHandle,
}

impl SyncWorkerTerminalObserver for RuntimeTerminalObserver {
    fn observe(&self, job: &StoredSyncJobV1) -> Result<(), String> {
        let notification_result = (|| {
            let state = self.app.state::<AppState>();
            let paths = state.acquire_paths().map_err(|_| {
                eprintln!("notification_query_failed");
                "notification_query_failed".to_string()
            })?;
            let connection = open_database(&paths.database).map_err(|_| {
                eprintln!("notification_query_failed");
                "notification_query_failed".to_string()
            })?;
            let settings = state.device_settings.get().ok_or_else(|| {
                eprintln!("notification_state_update_failed");
                "notification_state_update_failed".to_string()
            })?;
            let result = observe_terminal_notification(&connection, settings, job, |kind| {
                let notification_settings = load_portable_settings(&paths.data_root)
                    .map_err(|_| "notification_delivery_failed".to_string())?
                    .global
                    .notifications;
                if system_notification_enabled(&notification_settings, kind) {
                    send_system_notification(&self.app, kind)
                } else {
                    Ok(())
                }
            });
            let _ = self.app.emit(NOTIFICATION_CHANGED_EVENT, ());
            result.map_err(|error| {
                let code = match error.as_str() {
                    "notification_query_failed" => "notification_query_failed",
                    "notification_delivery_failed" => "notification_delivery_failed",
                    _ => "notification_state_update_failed",
                };
                eprintln!("{code}");
                code.to_string()
            })
        })();
        let state = self.app.state::<AppState>();
        observe_terminal_cover_cache_best_effort(state.inner(), job);
        if let Some(worker) = state.scheduler_worker.get() {
            let _ = worker.wake();
        }
        let _ = self.app.emit("sync:changed", job.snapshot.id);
        let _ = self.app.emit("scheduler:changed", ());
        notification_result
    }
}

fn observe_terminal_cover_cache_best_effort(state: &AppState, job: &StoredSyncJobV1) {
    if !matches!(job.snapshot.status, SyncJobStatusV1::Success | SyncJobStatusV1::Partial) {
        return;
    }
    let result = (|| {
        let paths = state.acquire_paths()?;
        let portable = load_portable_settings(&paths.data_root)?;
        let mut connection = open_database(&paths.database)?;
        assert_schema_latest(&connection)?;
        if !portable.global.cover_cache.enabled {
            disable_cover_cache_queue(&mut connection, &now_iso())?;
            return Ok(());
        }
        ensure_pending_cache_task(
            &mut connection,
            CacheTaskKindV1::NewRecords,
            &now_iso(),
        )?;
        Ok::<(), String>(())
    })();
    if let Err(error) = result {
        eprintln!("cover_cache_sync_observer_failed:{error}");
    }
    if let Some(worker) = state.cover_cache_worker.get() {
        worker.wake();
    }
}

impl AppState {
    fn initialize_runtime_paths(&self, paths: AppPaths) -> Result<(), String> {
        self.runtime_paths
            .set(RuntimePaths::new(paths))
            .map_err(|_| "runtime_paths_already_initialized".to_string())
    }

    fn initialize_device_settings(&self, store: DeviceSettingsStore) -> Result<(), String> {
        self.device_settings
            .set(store)
            .map_err(|_| "device_settings_already_initialized".to_string())
    }

    fn record_autostart_error(&self, error: Option<String>) {
        if let Ok(mut slot) = self.autostart_error.lock() {
            *slot = error;
        }
    }

    fn acquire_paths(&self) -> Result<PathLease, String> {
        self.runtime_paths
            .get()
            .ok_or_else(|| "runtime_paths_not_initialized".to_string())?
            .acquire()
    }

    fn runtime_paths(&self) -> Result<RuntimePaths, String> {
        self.runtime_paths
            .get()
            .cloned()
            .ok_or_else(|| "runtime_paths_not_initialized".to_string())
    }

    fn app_ready_at(&self) -> chrono::DateTime<Utc> {
        self.app_ready_at.get_or_init(Utc::now).to_owned()
    }

    fn record_startup_error(&self, error: Option<String>) {
        if let Ok(mut slot) = self.startup_error.lock() {
            *slot = error;
        }
    }
}

fn require_normal_runtime(state: &AppState) -> Result<(), String> {
    if state.normal_runtime_ready.load(Ordering::Acquire) {
        Ok(())
    } else {
        Err("startup_not_ready".to_string())
    }
}

fn send_system_notification(app: &AppHandle, kind: NotificationKindV1) -> Result<(), String> {
    let message = notification_message(kind);
    app.notification()
        .builder()
        .title(message.title)
        .body(message.body)
        .show()
        .map_err(|_| "notification_delivery_failed".to_string())
}

fn system_notification_enabled(
    settings: &NotificationSettingsV1,
    kind: NotificationKindV1,
) -> bool {
    match kind {
        NotificationKindV1::LoginExpired => settings.login_expired,
        NotificationKindV1::AutomaticFailureStreak => settings.automatic_failure_streak,
        NotificationKindV1::BackupFailed => settings.backup_failure,
        NotificationKindV1::BackupCompleted => settings.backup_completed,
        NotificationKindV1::FirstTrayHide => true,
    }
}

fn show_main_window(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main_window_missing".to_string())?;
    window
        .show()
        .map_err(|_| "main_window_show_failed".to_string())?;
    window
        .unminimize()
        .map_err(|_| "main_window_restore_failed".to_string())?;
    window
        .set_focus()
        .map_err(|_| "main_window_focus_failed".to_string())
}

fn publish_desktop_intent(
    app: &AppHandle,
    show_main: bool,
    open_settings: bool,
) -> Result<DesktopIntentSnapshotV1, String> {
    let state = app.state::<AppState>();
    let snapshot = state.desktop_intents.request(show_main, open_settings)?;
    if show_main {
        show_main_window(app)?;
    }
    app.emit("desktop:intent-changed", snapshot)
        .map_err(|_| "desktop_intent_emit_failed".to_string())?;
    Ok(snapshot)
}

fn set_scheduler_paused_runtime(
    app: &AppHandle,
    state: &AppState,
    paused: bool,
) -> Result<(), String> {
    let update = state.operation_admission.begin_automatic_pause_update()?;
    state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?
        .set_scheduler_paused(paused)?;
    update.commit(paused)?;
    wake_scheduler_and_queue(state)?;
    refresh_tray_menu(app).map_err(|_| "tray_menu_refresh_failed".to_string())?;
    app.emit("scheduler:changed", paused)
        .map_err(|_| "scheduler_event_emit_failed".to_string())
}

fn active_accounts_for_scope(
    state: &AppState,
    paths: &AppPaths,
    connection: &Connection,
    scope: SyncScopeV1,
) -> Result<Vec<AccountView>, String> {
    match scope {
        SyncScopeV1::CurrentAccountAvailableSources => {
            let _bootstrap_write = state
                .bootstrap_write
                .lock()
                .map_err(|_| "bootstrap_lock_poisoned".to_string())?;
            let account = load_active_account_storage(&paths.bootstrap_dir, connection)?
                .filter(|account| account.status == "active")
                .ok_or_else(|| "no_active_account".to_string())?;
            Ok(vec![account])
        }
        SyncScopeV1::AllActiveAccounts => {
            let accounts = list_command_accounts(connection, false)?
                .into_iter()
                .filter(|account| account.status == "active")
                .collect::<Vec<_>>();
            if accounts.is_empty() {
                Err("no_active_accounts".to_string())
            } else {
                Ok(accounts)
            }
        }
        SyncScopeV1::One => Err("invalid_sync_batch_scope".to_string()),
    }
}

fn build_manual_enqueue_snapshots(
    state: &AppState,
    paths: &AppPaths,
    connection: &Connection,
    portable: &PortableSettingsV2,
    accounts: &[AccountView],
) -> Result<(EnqueueSettingsSnapshotV1, EnqueueDeviceSnapshotV1), String> {
    let device = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?
        .load()?;
    let mut settings = std::collections::BTreeMap::new();
    let mut browsers = std::collections::BTreeMap::new();
    for account in accounts {
        let account_key = portable_account_key_for_id(connection, account.id)?;
        settings.insert(
            account.id,
            account_settings_for(portable, Some(&account_key))?,
        );
        let preference = device
            .accounts
            .get(&account.profile_key)
            .cloned()
            .unwrap_or_default()
            .browser_preference;
        let browser =
            current_browser_execution_identity(paths, connection, account.id, preference)?
                .ok_or_else(|| "login_validation_required".to_string())?;
        browsers.insert(account.id, browser);
    }
    Ok((
        EnqueueSettingsSnapshotV1 {
            revision: portable.revision,
            accounts: settings,
        },
        EnqueueDeviceSnapshotV1 { browsers },
    ))
}

fn enqueue_tray_scope(state: &AppState, scope: SyncScopeV1) -> Result<EnqueueSyncResultV1, String> {
    let _operation = state.operation_admission.begin_enqueue_operation()?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let portable = load_portable_settings(&paths.data_root)?;
    let accounts = active_accounts_for_scope(state, &paths, &connection, scope)?;
    let (settings, device) =
        build_manual_enqueue_snapshots(state, &paths, &connection, &portable, &accounts)?;
    let request = EnqueueSyncRequestV1 {
        scope,
        account_id: (scope == SyncScopeV1::CurrentAccountAvailableSources)
            .then_some(accounts[0].id),
        source: None,
        source_instance_key: None,
        payload: None,
        full_reconcile_confirmation_sha256: None,
    };
    let result = EnqueueService::new(&mut connection).enqueue(
        &request,
        &settings,
        &device,
        CapabilityRegistry::stage_3_3().all(),
    )?;
    state
        .sync_worker
        .get()
        .ok_or_else(|| "sync_worker_not_initialized".to_string())?
        .wake();
    Ok(result)
}

fn accounts_for_enqueue_request(
    state: &AppState,
    paths: &AppPaths,
    connection: &Connection,
    request: &EnqueueSyncRequestV1,
) -> Result<Vec<AccountView>, String> {
    if request.scope != SyncScopeV1::One {
        return active_accounts_for_scope(state, paths, connection, request.scope);
    }
    let account_id = request
        .account_id
        .ok_or_else(|| "invalid_sync_enqueue_request".to_string())?;
    let account = get_account_storage(connection, account_id)?
        .filter(|account| account.status == "active")
        .ok_or_else(|| "account_not_active".to_string())?;
    Ok(vec![account])
}

fn enqueue_sync_tasks_runtime(
    state: &AppState,
    request: &EnqueueSyncRequestV1,
) -> Result<EnqueueSyncResultV1, String> {
    let operation = state.operation_admission.begin_enqueue_operation()?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let portable = load_portable_settings(&paths.data_root)?;
    let accounts = accounts_for_enqueue_request(state, &paths, &connection, request)?;
    let (settings, device) =
        build_manual_enqueue_snapshots(state, &paths, &connection, &portable, &accounts)?;
    let result = EnqueueService::new(&mut connection).enqueue(
        request,
        &settings,
        &device,
        CapabilityRegistry::stage_3_3().all(),
    )?;
    drop(connection);
    drop(paths);
    drop(operation);
    state
        .sync_worker
        .get()
        .ok_or_else(|| "sync_worker_not_initialized".to_string())?
        .wake();
    if let Some(worker) = state.scheduler_worker.get() {
        worker.wake()?;
    }
    Ok(result)
}

fn workspace_sync_accounts(
    connection: &Connection,
    account_scope: &AccountScopeV1,
) -> Result<Vec<AccountView>, String> {
    validate_account_scope(account_scope)?;
    match account_scope {
        AccountScopeV1::All => {
            let accounts = list_command_accounts(connection, false)?
                .into_iter()
                .filter(|account| account.status == "active")
                .collect::<Vec<_>>();
            if accounts.is_empty() {
                Err("no_active_accounts".to_string())
            } else {
                Ok(accounts)
            }
        }
        AccountScopeV1::Selected { account_ids } => account_ids
            .iter()
            .map(|account_id| {
                get_account_storage(connection, *account_id)
                    .map_err(|_| "account_scope_invalid".to_string())?
                    .filter(|account| account.status == "active")
                    .ok_or_else(|| "account_scope_invalid".to_string())
            })
            .collect(),
    }
}

fn enqueue_workspace_sync_runtime(
    state: &AppState,
    account_scope: &AccountScopeV1,
) -> Result<EnqueueSyncResultV1, String> {
    let operation = state.operation_admission.begin_enqueue_operation()?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let portable = load_portable_settings(&paths.data_root)?;
    let accounts = workspace_sync_accounts(&connection, account_scope)?;
    let account_ids = accounts
        .iter()
        .map(|account| account.id)
        .collect::<Vec<_>>();
    let (settings, device) =
        build_manual_enqueue_snapshots(state, &paths, &connection, &portable, &accounts)?;
    let request = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::AllActiveAccounts,
        account_id: None,
        source: None,
        source_instance_key: None,
        payload: None,
        full_reconcile_confirmation_sha256: None,
    };
    let result = EnqueueService::new(&mut connection).enqueue_for_account_ids(
        &request,
        &account_ids,
        &settings,
        &device,
        CapabilityRegistry::stage_3_3().all(),
    )?;
    drop(connection);
    drop(paths);
    drop(operation);
    state
        .sync_worker
        .get()
        .ok_or_else(|| "sync_worker_not_initialized".to_string())?
        .wake();
    if let Some(worker) = state.scheduler_worker.get() {
        worker.wake()?;
    }
    Ok(result)
}

fn spawn_tray_enqueue(app: &AppHandle, scope: SyncScopeV1) {
    let app_for_thread = app.clone();
    let spawn = thread::Builder::new()
        .name("emily-tray-enqueue".to_string())
        .spawn(move || {
            let state = app_for_thread.state::<AppState>();
            match enqueue_tray_scope(state.inner(), scope) {
                Ok(result) => {
                    let _ = app_for_thread.emit("sync:changed", result);
                }
                Err(_) => eprintln!("tray_sync_enqueue_failed"),
            }
        });
    if spawn.is_err() {
        eprintln!("tray_sync_enqueue_start_failed");
    }
}

fn tray_menu_input(state: &AppState) -> Result<TrayMenuDecisionInputV1, String> {
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    let current_account_active = {
        let _bootstrap_write = state
            .bootstrap_write
            .lock()
            .map_err(|_| "bootstrap_lock_poisoned".to_string())?;
        load_active_account_storage(&paths.bootstrap_dir, &connection)?
            .is_some_and(|account| account.status == "active")
    };
    let has_active_accounts = list_command_accounts(&connection, false)?
        .iter()
        .any(|account| account.status == "active");
    let admission = state.operation_admission.snapshot()?;
    let scheduler_paused = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?
        .load()?
        .scheduler_paused;
    Ok(TrayMenuDecisionInputV1 {
        runtime_ready: state.normal_runtime_ready.load(Ordering::Acquire),
        current_account_active,
        has_active_accounts,
        admission_mode: admission.mode,
        scheduler_paused,
    })
}

fn refresh_tray_menu(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let handles = state
        .tray_menu
        .lock()
        .map_err(|_| "tray_menu_state_poisoned".to_string())?
        .clone()
        .ok_or_else(|| "tray_menu_unavailable".to_string())?;
    for item in tray_menu_decision(tray_menu_input(state.inner())?) {
        match item.id {
            TrayMenuItemIdV1::SyncCurrentAccount => handles
                .sync_current
                .set_enabled(item.enabled)
                .map_err(|_| "tray_menu_refresh_failed".to_string())?,
            TrayMenuItemIdV1::SyncAllAccounts => handles
                .sync_all
                .set_enabled(item.enabled)
                .map_err(|_| "tray_menu_refresh_failed".to_string())?,
            TrayMenuItemIdV1::PauseAutomatic => {
                handles
                    .pause_automatic
                    .set_enabled(item.enabled)
                    .map_err(|_| "tray_menu_refresh_failed".to_string())?;
                handles
                    .pause_automatic
                    .set_checked(item.checked.unwrap_or(false))
                    .map_err(|_| "tray_menu_refresh_failed".to_string())?;
            }
            TrayMenuItemIdV1::OpenMain
            | TrayMenuItemIdV1::OpenSettings
            | TrayMenuItemIdV1::Exit => {}
        }
    }
    Ok(())
}

fn handle_tray_menu_event(app: &AppHandle, id: &str) {
    match id {
        TRAY_MENU_OPEN_MAIN => {
            if publish_desktop_intent(app, true, false).is_err() {
                eprintln!("desktop_intent_publish_failed");
            }
        }
        TRAY_MENU_SYNC_CURRENT => {
            spawn_tray_enqueue(app, SyncScopeV1::CurrentAccountAvailableSources);
        }
        TRAY_MENU_SYNC_ALL => {
            spawn_tray_enqueue(app, SyncScopeV1::AllActiveAccounts);
        }
        TRAY_MENU_PAUSE => {
            let state = app.state::<AppState>();
            let paused = state
                .device_settings
                .get()
                .and_then(|store| store.load().ok())
                .is_some_and(|settings| settings.scheduler_paused);
            if set_scheduler_paused_runtime(app, state.inner(), !paused).is_err() {
                eprintln!("scheduler_pause_update_failed");
            }
        }
        TRAY_MENU_OPEN_SETTINGS => {
            if publish_desktop_intent(app, true, true).is_err() {
                eprintln!("desktop_intent_publish_failed");
            }
        }
        TRAY_MENU_EXIT => request_app_exit_runtime(app),
        _ => {}
    }
}

fn create_tray(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let open_main = MenuItem::with_id(app, TRAY_MENU_OPEN_MAIN, "打开主界面", true, None::<&str>)
        .map_err(|_| "tray_menu_create_failed".to_string())?;
    let sync_current = MenuItem::with_id(
        app,
        TRAY_MENU_SYNC_CURRENT,
        "立即同步当前账号",
        false,
        None::<&str>,
    )
    .map_err(|_| "tray_menu_create_failed".to_string())?;
    let sync_all = MenuItem::with_id(app, TRAY_MENU_SYNC_ALL, "同步全部账号", false, None::<&str>)
        .map_err(|_| "tray_menu_create_failed".to_string())?;
    let pause_automatic = CheckMenuItem::with_id(
        app,
        TRAY_MENU_PAUSE,
        "暂停自动同步",
        true,
        false,
        None::<&str>,
    )
    .map_err(|_| "tray_menu_create_failed".to_string())?;
    let open_settings = MenuItem::with_id(app, TRAY_MENU_OPEN_SETTINGS, "设置", true, None::<&str>)
        .map_err(|_| "tray_menu_create_failed".to_string())?;
    let exit = MenuItem::with_id(app, TRAY_MENU_EXIT, "退出", true, None::<&str>)
        .map_err(|_| "tray_menu_create_failed".to_string())?;
    let menu = Menu::with_items(
        app,
        &[
            &open_main,
            &sync_current,
            &sync_all,
            &pause_automatic,
            &open_settings,
            &exit,
        ],
    )
    .map_err(|_| "tray_menu_create_failed".to_string())?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| "tray_icon_missing".to_string())?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(douyin_history_archive::uninstall::PRODUCT_NAME)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_tray_menu_event(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => {
                if publish_desktop_intent(tray.app_handle(), true, false).is_err() {
                    eprintln!("desktop_intent_publish_failed");
                }
            }
            TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Down,
                ..
            } => {
                if refresh_tray_menu(tray.app_handle()).is_err() {
                    eprintln!("tray_menu_refresh_failed");
                }
            }
            _ => {}
        })
        .build(app)
        .map_err(|_| "tray_unavailable".to_string())?;
    *state
        .tray_menu
        .lock()
        .map_err(|_| "tray_menu_state_poisoned".to_string())? = Some(TrayMenuHandles {
        sync_current,
        sync_all,
        pause_automatic,
    });
    state.tray_available.store(true, Ordering::Release);
    if refresh_tray_menu(app).is_err() {
        eprintln!("tray_menu_refresh_failed");
    }
    Ok(())
}

fn initialize_activation_listener(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let event = state
        .activation_event
        .lock()
        .map_err(|_| "gui_activation_event_poisoned".to_string())?
        .take()
        .ok_or_else(|| "gui_activation_event_missing".to_string())?;
    let app_for_activation = app.clone();
    let listener = GuiActivationListener::start(event, move || {
        if publish_desktop_intent(&app_for_activation, true, false).is_err() {
            eprintln!("desktop_activation_failed");
        }
    })?;
    *state
        .activation_listener
        .lock()
        .map_err(|_| "gui_activation_listener_poisoned".to_string())? = Some(listener);
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StartupRecoveryStatusView {
    state: &'static str,
    error_code: Option<String>,
    can_resolve: bool,
    can_retry: bool,
}

fn startup_recovery_status(
    normal_runtime_ready: bool,
    recovery_state: StartupRecoveryStateV1,
    startup_error: Option<String>,
) -> StartupRecoveryStatusView {
    if normal_runtime_ready {
        return StartupRecoveryStatusView {
            state: "ready",
            error_code: None,
            can_resolve: false,
            can_retry: false,
        };
    }
    match recovery_state {
        StartupRecoveryStateV1::Blocked => StartupRecoveryStatusView {
            state: "blocked",
            error_code: startup_error.or_else(|| Some("startup_recovery_blocked".to_string())),
            can_resolve: true,
            can_retry: false,
        },
        StartupRecoveryStateV1::Recovering => StartupRecoveryStatusView {
            state: "recovering",
            error_code: None,
            can_resolve: false,
            can_retry: false,
        },
        StartupRecoveryStateV1::Ready => StartupRecoveryStatusView {
            state: "failed",
            error_code: startup_error
                .or_else(|| Some("startup_continuation_incomplete".to_string())),
            can_resolve: false,
            can_retry: true,
        },
        StartupRecoveryStateV1::NotStarted => StartupRecoveryStatusView {
            state: "failed",
            error_code: startup_error.or_else(|| Some("startup_not_ready".to_string())),
            can_resolve: false,
            can_retry: false,
        },
    }
}

fn startup_recovery_status_from_state(
    state: &AppState,
) -> Result<StartupRecoveryStatusView, String> {
    let startup_error = state
        .startup_error
        .lock()
        .map_err(|_| "startup_error_poisoned".to_string())?
        .clone();
    Ok(startup_recovery_status(
        state.normal_runtime_ready.load(Ordering::Acquire),
        state.startup_recovery.state()?,
        startup_error,
    ))
}

#[tauri::command]
fn get_startup_recovery_status(
    state: State<'_, AppState>,
) -> Result<StartupRecoveryStatusView, String> {
    startup_recovery_status_from_state(state.inner())
}

#[tauri::command]
fn take_desktop_intents(
    state: State<'_, AppState>,
    last_seen_revision: u64,
) -> Result<DesktopIntentSnapshotV1, String> {
    state.desktop_intents.take(last_seen_revision)
}

#[tauri::command]
fn set_scheduler_paused(
    app: AppHandle,
    state: State<'_, AppState>,
    paused: bool,
) -> Result<(), String> {
    require_normal_runtime(state.inner())?;
    set_scheduler_paused_runtime(&app, state.inner(), paused)
}

#[tauri::command]
fn get_desktop_state(state: State<'_, AppState>) -> Result<DesktopStateViewV1, String> {
    require_normal_runtime(state.inner())?;
    let device = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?
        .load()?;
    let tray_error = state
        .tray_error
        .lock()
        .map_err(|_| "tray_error_poisoned".to_string())?
        .clone();
    let autostart_error = state
        .autostart_error
        .lock()
        .map_err(|_| "autostart_error_poisoned".to_string())?
        .clone();
    Ok(build_desktop_state_view(
        device,
        state.tray_available.load(Ordering::Acquire),
        tray_error,
        autostart_error,
    ))
}

#[tauri::command]
fn enqueue_sync_tasks(
    app: AppHandle,
    state: State<'_, AppState>,
    request: EnqueueSyncRequestV1,
) -> Result<EnqueueSyncResultV1, String> {
    require_normal_runtime(state.inner())?;
    let result = enqueue_sync_tasks_runtime(state.inner(), &request)?;
    app.emit("sync:changed", &result)
        .map_err(|_| "sync_event_emit_failed".to_string())?;
    app.emit("scheduler:changed", ())
        .map_err(|_| "scheduler_event_emit_failed".to_string())?;
    Ok(result)
}

#[tauri::command]
fn enqueue_workspace_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    account_scope: AccountScopeV1,
) -> Result<EnqueueSyncResultV1, String> {
    require_normal_runtime(state.inner())?;
    let result = enqueue_workspace_sync_runtime(state.inner(), &account_scope)?;
    app.emit("sync:changed", &result)
        .map_err(|_| "sync_event_emit_failed".to_string())?;
    app.emit("scheduler:changed", ())
        .map_err(|_| "scheduler_event_emit_failed".to_string())?;
    Ok(result)
}

#[tauri::command]
fn get_sync_batch(state: State<'_, AppState>, batch_id: i64) -> Result<SyncBatchViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    QueueStore::get_batch(&connection, batch_id)
}

#[tauri::command]
fn list_sync_batch_entries(
    state: State<'_, AppState>,
    batch_id: i64,
    cursor: Option<String>,
    limit: usize,
) -> Result<SyncBatchEntriesPageV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    QueueStore::list_batch_entries(&connection, batch_id, cursor.as_deref(), limit)
}

#[tauri::command]
fn list_sync_batches(
    state: State<'_, AppState>,
    account_id: i64,
    limit: usize,
    offset: usize,
) -> Result<douyin_history_archive::sync::SyncBatchesPageV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    require_account_status(
        &connection,
        account_id,
        &["active", "login_expired", "identity_conflict", "archived"],
    )?;
    QueueStore::list_batches_for_account(&connection, account_id, limit, offset)
}

#[tauri::command]
fn list_task_summaries(
    state: State<'_, AppState>,
    account_scope: AccountScopeV1,
    limit: usize,
    offset: usize,
) -> Result<TaskSummariesPageV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    list_task_summaries_storage(&connection, &account_scope, limit, offset)
}

#[tauri::command]
fn cancel_sync_batch(
    app: AppHandle,
    state: State<'_, AppState>,
    batch_id: i64,
) -> Result<(), String> {
    require_normal_runtime(state.inner())?;
    if batch_id <= 0 {
        return Err("invalid_sync_batch_id".to_string());
    }
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    let batch = QueueStore::get_batch(&connection, batch_id)?;
    if !matches!(batch.status, SyncBatchStatusV1::Queued | SyncBatchStatusV1::Running) {
        return Err("sync_batch_not_cancellable".to_string());
    }
    let job_rows = {
        let mut statement = connection
            .prepare(
                "SELECT jobs.id, jobs.account_id
                 FROM sync_batch_jobs links
                 JOIN sync_jobs jobs ON jobs.id = links.job_id
                 WHERE links.batch_id = ?1 AND jobs.status IN ('queued', 'running')
                 ORDER BY links.entry_seq",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([batch_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>();
        rows.map_err(|error| error.to_string())?
    };
    let registry = state
        .sync_registry
        .get()
        .ok_or_else(|| "sync_registry_not_initialized".to_string())?;
    let worker = state
        .sync_worker
        .get()
        .ok_or_else(|| "sync_worker_not_initialized".to_string())?;
    for (job_id, account_id) in job_rows {
        let running = worker.running_identity();
        cancel_sync_job(
            &mut connection,
            registry,
            account_id,
            job_id,
            running.as_ref(),
            &now_iso(),
        )?;
    }
    QueueStore::recalculate_batch_status(&mut connection, batch_id, &now_iso())?;
    worker.wake();
    app.emit("sync:changed", batch_id)
        .map_err(|_| "sync_event_emit_failed".to_string())?;
    Ok(())
}

#[tauri::command]
fn get_scheduler_state(state: State<'_, AppState>) -> Result<SchedulerStateV1, String> {
    require_normal_runtime(state.inner())?;
    scheduler_state_runtime(state.inner())
}

#[tauri::command]
fn list_scheduler_instances(
    state: State<'_, AppState>,
    account_id: i64,
    source: SyncSourceV1,
    cursor: Option<String>,
    limit: usize,
) -> Result<SchedulerInstancesPageV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    list_scheduler_instances_storage(
        &mut connection,
        &paths.data_root,
        account_id,
        source,
        cursor.as_deref(),
        limit,
    )
}

#[tauri::command]
fn update_account_defaults(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AccountSettingsV2,
) -> Result<u64, String> {
    require_normal_runtime(state.inner())?;
    let revision = update_account_defaults_runtime(state.inner(), settings)?;
    app.emit("scheduler:changed", revision)
        .map_err(|_| "scheduler_event_emit_failed".to_string())?;
    Ok(revision)
}

#[tauri::command]
fn commit_onboarding_step(
    app: AppHandle,
    state: State<'_, AppState>,
    request: CommitOnboardingStepRequestV1,
) -> Result<CommitOnboardingStepResultV1, String> {
    require_normal_runtime(state.inner())?;
    let result = commit_onboarding_step_runtime(state.inner(), &request)?;
    refresh_tray_menu(&app).map_err(|_| "tray_menu_refresh_failed".to_string())?;
    app.emit("scheduler:changed", ())
        .map_err(|_| "scheduler_event_emit_failed".to_string())?;
    Ok(result)
}

#[tauri::command]
fn set_autostart(
    app: AppHandle,
    state: State<'_, AppState>,
    request: SetAutostartRequestV1,
) -> Result<DeviceSettingsViewV1, String> {
    require_normal_runtime(state.inner())?;
    let result = set_autostart_runtime(state.inner(), &request)?;
    app.emit("scheduler:changed", ())
        .map_err(|_| "scheduler_event_emit_failed".to_string())?;
    Ok(result)
}

#[tauri::command]
fn set_browser_preference(
    app: AppHandle,
    state: State<'_, AppState>,
    request: SetBrowserPreferenceRequestV1,
) -> Result<DeviceSettingsViewV1, String> {
    require_normal_runtime(state.inner())?;
    let result = set_browser_preference_runtime(state.inner(), &request)?;
    app.emit("scheduler:changed", ())
        .map_err(|_| "scheduler_event_emit_failed".to_string())?;
    Ok(result)
}

fn request_app_exit_runtime(app: &AppHandle) {
    let state = app.state::<AppState>();
    match state
        .exit_request
        .on_exit_requested(state.normal_runtime_ready.load(Ordering::Acquire))
    {
        Ok(ExitRequestDecisionV1::PreventAndStart) => start_controlled_exit(app.clone()),
        Ok(ExitRequestDecisionV1::Allow) => app.exit(0),
        Ok(ExitRequestDecisionV1::Prevent) | Err(_) => {}
    }
}

#[tauri::command]
fn request_app_exit(app: AppHandle) -> Result<(), String> {
    request_app_exit_runtime(&app);
    Ok(())
}

fn get_app_settings_from_state(
    state: &AppState,
    account_id: Option<i64>,
    install_dir: PathBuf,
) -> Result<AppSettingsView, String> {
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let known_account_keys = known_portable_account_keys(&connection)?;
    let account_key = account_id
        .map(|account_id| portable_account_key_for_id(&connection, account_id))
        .transpose()?;
    let bootstrap =
        read_bootstrap(&paths.bootstrap_dir)?.ok_or_else(|| "bootstrap_missing".to_string())?;
    if !same_windows_path(&bootstrap.current_root.path, &paths.data_root) {
        return Err("data_root_source_mismatch".to_string());
    }
    load_app_settings_view(
        &paths.data_root,
        account_key.as_deref(),
        &known_account_keys,
        paths.data_root.clone(),
        install_dir,
        bootstrap.retained_data_roots,
    )
}

fn update_global_settings_from_state(
    state: &AppState,
    settings: GlobalSettingsV2,
) -> Result<u64, String> {
    let operation = state.operation_admission.begin_device_settings_update()?;
    let paths = state.acquire_paths()?;
    let previous = load_portable_settings(&paths.data_root)?.global;
    let revision = update_global_settings_storage(&paths.data_root, settings.clone())?.revision;
    let mut connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    reconcile_cover_cache_settings(&mut connection, &previous, &settings, &now_iso())?;
    drop(paths);
    drop(operation);
    wake_scheduler_and_queue(state)?;
    if let Some(worker) = state.cover_cache_worker.get() {
        worker.wake();
    }
    Ok(revision)
}

const AUTOMATIC_BACKUP_FAILURE_NOTIFICATION_KEY: &str = "backup:automatic:failed";
const NOTIFICATION_CHANGED_EVENT: &str = "notification:changed";

fn deliver_runtime_notification(
    app: &AppHandle,
    state: &AppState,
    kind: NotificationKindV1,
) -> Result<(), String> {
    if kind == NotificationKindV1::FirstTrayHide {
        return send_system_notification(app, kind);
    }
    let paths = state.acquire_paths()?;
    if kind == NotificationKindV1::BackupFailed {
        let connection = open_database(&paths.database)?;
        assert_schema_latest(&connection)?;
        upsert_notification(
            &connection,
            notification_insert_for_kind(
                kind,
                AUTOMATIC_BACKUP_FAILURE_NOTIFICATION_KEY.to_string(),
                None,
                None,
                None,
                now_iso(),
            ),
        )?;
        drop(connection);
        app.emit(NOTIFICATION_CHANGED_EVENT, ())
            .map_err(|_| "notification_event_emit_failed".to_string())?;
    }
    let notification_settings = load_portable_settings(&paths.data_root)?.global.notifications;
    drop(paths);
    if system_notification_enabled(&notification_settings, kind) {
        send_system_notification(app, kind)
    } else {
        Ok(())
    }
}

fn resolve_automatic_backup_failure_notification(
    app: &AppHandle,
    state: &AppState,
) -> Result<(), String> {
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    let changed = resolve_notification_by_dedupe(
        &connection,
        AUTOMATIC_BACKUP_FAILURE_NOTIFICATION_KEY,
        &now_iso(),
    )?;
    if changed > 0 {
        app.emit(NOTIFICATION_CHANGED_EVENT, ())
            .map_err(|_| "notification_event_emit_failed".to_string())?;
    }
    Ok(())
}

fn list_notifications_from_state(
    state: &AppState,
    include_resolved: bool,
    limit: usize,
    offset: usize,
) -> Result<NotificationsPageV1, String> {
    require_normal_runtime(state)?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    list_notifications_storage(&connection, include_resolved, limit, offset)
}

fn mark_notification_read_from_state(
    state: &AppState,
    notification_id: i64,
    read: bool,
    changed_at: &str,
    emit: impl FnOnce(&'static str) -> Result<(), String>,
) -> Result<NotificationViewV1, String> {
    require_normal_runtime(state)?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    let view =
        mark_notification_read_storage(&connection, notification_id, read, changed_at)?;
    drop(connection);
    drop(paths);
    emit(NOTIFICATION_CHANGED_EVENT)?;
    Ok(view)
}

fn mark_all_notifications_read_from_state(
    state: &AppState,
    changed_at: &str,
    emit: impl FnOnce(&'static str) -> Result<(), String>,
) -> Result<usize, String> {
    require_normal_runtime(state)?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    let changed = mark_all_notifications_read_storage(&connection, changed_at)?;
    drop(connection);
    drop(paths);
    emit(NOTIFICATION_CHANGED_EVENT)?;
    Ok(changed)
}

fn resolve_notification_from_state(
    state: &AppState,
    notification_id: i64,
    resolved_at: &str,
    emit: impl FnOnce(&'static str) -> Result<(), String>,
) -> Result<NotificationViewV1, String> {
    require_normal_runtime(state)?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    let view = resolve_notification_storage(&connection, notification_id, resolved_at)?;
    drop(connection);
    drop(paths);
    emit(NOTIFICATION_CHANGED_EVENT)?;
    Ok(view)
}

#[tauri::command]
fn list_notifications(
    state: State<'_, AppState>,
    include_resolved: bool,
    limit: usize,
    offset: usize,
) -> Result<NotificationsPageV1, String> {
    list_notifications_from_state(state.inner(), include_resolved, limit, offset)
}

#[tauri::command]
fn mark_notification_read(
    app: AppHandle,
    state: State<'_, AppState>,
    notification_id: i64,
    read: bool,
) -> Result<NotificationViewV1, String> {
    mark_notification_read_from_state(
        state.inner(),
        notification_id,
        read,
        &now_iso(),
        |event| {
            app.emit(event, notification_id)
                .map_err(|_| "notification_event_emit_failed".to_string())
        },
    )
}

#[tauri::command]
fn mark_all_notifications_read(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    mark_all_notifications_read_from_state(state.inner(), &now_iso(), |event| {
        app.emit(event, ())
            .map_err(|_| "notification_event_emit_failed".to_string())
    })
}

#[tauri::command]
fn resolve_notification(
    app: AppHandle,
    state: State<'_, AppState>,
    notification_id: i64,
) -> Result<NotificationViewV1, String> {
    resolve_notification_from_state(state.inner(), notification_id, &now_iso(), |event| {
        app.emit(event, notification_id)
            .map_err(|_| "notification_event_emit_failed".to_string())
    })
}

fn update_workspace_settings_from_state(
    state: &AppState,
    expected_revision: u64,
    settings: WorkspaceSettingsV1,
) -> Result<u64, String> {
    let operation = state.operation_admission.begin_device_settings_update()?;
    let paths = state.acquire_paths()?;
    let revision = update_workspace_settings_storage(
        &paths.data_root,
        expected_revision,
        settings,
    )?
    .revision;
    drop(paths);
    drop(operation);
    Ok(revision)
}

fn reconcile_cover_cache_settings(
    connection: &mut Connection,
    previous: &GlobalSettingsV2,
    next: &GlobalSettingsV2,
    now: &str,
) -> Result<(), String> {
    if !next.cover_cache.enabled {
        return disable_cover_cache_queue(connection, now);
    }
    let start_backfill = next.cover_cache.backfill_mode == CoverCacheBackfillModeV1::BackfillAll
        && (!previous.cover_cache.enabled
            || previous.cover_cache.backfill_mode != CoverCacheBackfillModeV1::BackfillAll);
    if start_backfill {
        enqueue_all_missing_cover_candidates(connection, now)?;
    }
    ensure_pending_cache_task(
        connection,
        if start_backfill {
            CacheTaskKindV1::Backfill
        } else {
            CacheTaskKindV1::NewRecords
        },
        now,
    )?;
    Ok(())
}

fn update_account_settings_from_state(
    state: &AppState,
    account_id: i64,
    settings: AccountSettingsV2,
) -> Result<u64, String> {
    let operation = state
        .operation_admission
        .begin_account_source_settings_update(account_id)?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let account_key = portable_account_key_for_id(&connection, account_id)?;
    let profile_key = account_profile_key(&connection, account_id)?;
    let updated = update_account_settings_storage(&paths.data_root, &account_key, settings)?;
    let browser_preference = state
        .device_settings
        .get()
        .map(|store| store.load())
        .transpose()?
        .and_then(|device| device.accounts.get(&profile_key).cloned())
        .unwrap_or_default()
        .browser_preference;
    let account_settings = account_settings_for(&updated, Some(&account_key))?;
    let capability_registry = CapabilityRegistry::stage_3_3();
    let app_ready_at = state.app_ready_at();
    let now = Utc::now();
    if let Some(browser) =
        current_browser_execution_identity(&paths, &connection, account_id, browser_preference)?
    {
        reconcile_automatic_jobs(
            &mut connection,
            &AutomaticReconcileInputV1 {
                account_id,
                settings_revision: updated.revision,
                account_settings,
                browser_preference,
                browser,
                capabilities: capability_registry.all().to_vec(),
                app_ready_at,
                startup_delay_minutes: updated.global.startup_sync_delay_minutes,
                now,
            },
        )?;
    } else {
        cancel_stale_automatic_jobs(
            &mut connection,
            account_id,
            &account_settings,
            browser_preference,
            capability_registry.all(),
            &now.to_rfc3339_opts(SecondsFormat::Millis, true),
        )?;
    }
    let revision = updated.revision;
    drop(connection);
    drop(paths);
    drop(operation);
    wake_scheduler_and_queue(state)?;
    Ok(revision)
}

fn wake_scheduler_and_queue(state: &AppState) -> Result<(), String> {
    if let Some(worker) = state.scheduler_worker.get() {
        worker.wake()?;
    }
    if let Some(worker) = state.automatic_backup_worker.get() {
        worker.wake()?;
    }
    if let Some(worker) = state.sync_worker.get() {
        worker.wake();
    }
    Ok(())
}

fn update_account_defaults_runtime(
    state: &AppState,
    settings: AccountSettingsV2,
) -> Result<u64, String> {
    let operation = state.operation_admission.begin_device_settings_update()?;
    let paths = state.acquire_paths()?;
    let updated = update_account_defaults_for_capabilities(
        &paths.data_root,
        settings,
        CapabilityRegistry::stage_3_3().all(),
    )?;
    drop(paths);
    drop(operation);
    wake_scheduler_and_queue(state)?;
    Ok(updated.revision)
}

fn set_autostart_runtime(
    state: &AppState,
    request: &SetAutostartRequestV1,
) -> Result<DeviceSettingsViewV1, String> {
    let operation = state.operation_admission.begin_device_settings_update()?;
    let store = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?;
    let executable =
        std::env::current_exe().map_err(|_| "invalid_autostart_executable".to_string())?;
    let updated = set_autostart_transactional_at_revision(
        store,
        &state.desktop_settings_tx,
        &executable,
        request.expected_device_revision,
        request.enabled,
        request.silent,
        |_| Ok(()),
    )?;
    drop(operation);
    Ok(updated.into())
}

fn set_backup_directory_runtime(
    state: &AppState,
    path: Option<PathBuf>,
    expected_device_revision: u64,
) -> Result<DeviceSettingsViewV1, String> {
    let operation = state.operation_admission.begin_device_settings_update()?;
    if let Some(path) = path.as_deref() {
        probe_backup_directory(path)?;
    }
    let _transaction = state
        .desktop_settings_tx
        .lock()
        .map_err(|_| "desktop_settings_transaction_poisoned".to_string())?;
    let store = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?;
    let updated = store.compare_and_swap(expected_device_revision, move |settings| {
        settings.backup_directory = path;
        Ok(())
    })?;
    drop(_transaction);
    drop(operation);
    wake_scheduler_and_queue(state)?;
    Ok(updated.into())
}

fn account_has_active_job(connection: &Connection, account_id: i64) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM sync_jobs
               WHERE account_id = ?1 AND status IN ('queued', 'running')
             )",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn set_browser_preference_runtime(
    state: &AppState,
    request: &SetBrowserPreferenceRequestV1,
) -> Result<DeviceSettingsViewV1, String> {
    if !request.confirm_relogin {
        return Err("browser_relogin_confirmation_required".to_string());
    }
    let operation = state
        .operation_admission
        .begin_account_browser_update(request.account_id)?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    require_account_status(
        &connection,
        request.account_id,
        &["active", "login_expired", "identity_conflict"],
    )?;
    if account_has_active_job(&connection, request.account_id)? {
        return Err("account_browser_operation_in_progress".to_string());
    }
    let profile_key = account_profile_key(&connection, request.account_id)?;
    drop(connection);
    let store = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?;
    let current = store.load()?;
    let current_preference = current
        .accounts
        .get(&profile_key)
        .cloned()
        .unwrap_or_default()
        .browser_preference;
    if current_preference == request.browser_preference {
        return Ok(current.into());
    }

    let account_root = paths.account_root(&profile_key)?;
    let previous_selection = read_browser_selection(&account_root)?;
    clear_browser_selection(&account_root)?;
    let updated = store.compare_and_swap(current.revision, |settings| {
        settings
            .accounts
            .entry(profile_key.clone())
            .or_default()
            .browser_preference = request.browser_preference;
        Ok(())
    });
    let updated = match updated {
        Ok(updated) => updated,
        Err(error) => {
            if let Some(selection) = previous_selection {
                write_browser_selection(&account_root, &selection)
                    .map_err(|_| "browser_preference_state_uncertain".to_string())?;
            }
            return Err(error);
        }
    };
    drop(paths);
    drop(operation);
    wake_scheduler_and_queue(state)?;
    Ok(updated.into())
}

fn commit_onboarding_step_runtime(
    state: &AppState,
    request: &CommitOnboardingStepRequestV1,
) -> Result<CommitOnboardingStepResultV1, String> {
    let operation = state.operation_admission.begin_device_settings_update()?;
    let paths = state.acquire_paths()?;
    let store = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?;
    let executable =
        std::env::current_exe().map_err(|_| "invalid_autostart_executable".to_string())?;
    let result = commit_onboarding_step_with_device_commit(
        &paths.data_root,
        store,
        request,
        CapabilityRegistry::stage_3_3().all(),
        |expected_revision, step, next_step, completes_onboarding| match step {
            OnboardingStepRequestV1::DesktopBehavior {
                launch_at_startup,
                silent_autostart,
                ..
            } => set_autostart_transactional_at_revision(
                store,
                &state.desktop_settings_tx,
                &executable,
                expected_revision,
                *launch_at_startup,
                *silent_autostart,
                |settings| {
                    settings.onboarding_step = next_step;
                    settings.onboarding_completed = completes_onboarding;
                    Ok(())
                },
            ),
            _ => store.compare_and_swap(expected_revision, |settings| {
                settings.onboarding_step = next_step;
                settings.onboarding_completed = completes_onboarding;
                Ok(())
            }),
        },
    )?;
    drop(paths);
    drop(operation);
    wake_scheduler_and_queue(state)?;
    Ok(result)
}

fn current_browser_execution_identity(
    paths: &AppPaths,
    connection: &Connection,
    account_id: i64,
    preference: douyin_history_archive::device_settings::BrowserPreferenceV1,
) -> Result<Option<BrowserExecutionIdentityV1>, String> {
    let profile_key = account_profile_key(connection, account_id)?;
    let account_root = paths.account_root(&profile_key)?;
    if !account_root.is_dir() {
        return Ok(None);
    }
    let Some(selection) = read_browser_selection(&account_root)? else {
        return Ok(None);
    };
    if read_login_validation(&account_root, &selection)?.is_none() {
        return Ok(None);
    }
    let resolved_browser = match selection.browser_id {
        BrowserId::Edge => BrowserIdV1::Edge,
        BrowserId::Chrome => BrowserIdV1::Chrome,
    };
    let preference_matches = match preference {
        douyin_history_archive::device_settings::BrowserPreferenceV1::Auto => true,
        douyin_history_archive::device_settings::BrowserPreferenceV1::Edge => {
            resolved_browser == BrowserIdV1::Edge
        }
        douyin_history_archive::device_settings::BrowserPreferenceV1::Chrome => {
            resolved_browser == BrowserIdV1::Chrome
        }
    };
    if !preference_matches {
        return Ok(None);
    }
    let profile_generation = selection
        .profile_generation
        .ok_or_else(|| "browser_profile_changed".to_string())?;
    Ok(Some(BrowserExecutionIdentityV1 {
        preference,
        resolved_browser,
        profile_key,
        profile_generation,
        executable_path_sha256: browser_executable_path_sha256(&selection.executable_path),
    }))
}

fn import_legacy_settings_from_state(
    state: &AppState,
    account_id: Option<i64>,
    legacy: LegacyFrontendSettingsV1,
) -> Result<u64, String> {
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let account_key = account_id
        .map(|account_id| portable_account_key_for_id(&connection, account_id))
        .transpose()?;
    import_legacy_frontend_settings(&paths.data_root, account_key.as_deref(), legacy)
        .map(|value| value.revision)
}

fn sync_options_from_account_settings(settings: &AccountSettingsV2) -> SyncOptions {
    let (mode, days) = match settings.history.range {
        HistoryRangeV1::Days { days } => ("days".to_string(), Some(days)),
        HistoryRangeV1::Bottom => ("bottom".to_string(), None),
    };
    SyncOptions {
        mode,
        days,
        scroll_speed: Some(settings.scroll_speed),
        hide_browser: Some(settings.hide_browser),
        block_media: None,
        block_images: None,
    }
}

fn current_install_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "install_dir_unavailable".to_string())
}

fn same_windows_path(left: &Path, right: &Path) -> bool {
    let normalize = |path: &Path| {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    normalize(left) == normalize(right)
}

fn validate_data_root_from_state(
    state: &AppState,
    requested: &Path,
    install_dir: &Path,
) -> Result<DataRootValidationView, String> {
    let paths = state.acquire_paths()?;
    validate_data_root_request(
        &DataRootContext {
            app_data_dir: paths.data_root.clone(),
            install_dir: install_dir.to_path_buf(),
        },
        requested,
    )
    .map_err(|error| error.code().to_string())
}

#[cfg(test)]
fn set_data_root_from_state(
    state: &AppState,
    requested: PathBuf,
    install_dir: PathBuf,
) -> Result<DataRootMigrationView, String> {
    set_data_root_with_runtime(
        state.runtime_paths()?,
        state.operation_admission.clone(),
        Arc::clone(&state.bootstrap_write),
        requested,
        install_dir,
    )
}

#[cfg(test)]
fn initialize_onboarding_data_root_from_state(
    state: &AppState,
    requested: PathBuf,
    install_dir: PathBuf,
) -> Result<OnboardingDataRootView, String> {
    let device_settings = state
        .device_settings
        .get()
        .cloned()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?;
    initialize_onboarding_data_root_with_runtime(
        state.runtime_paths()?,
        state.operation_admission.clone(),
        Arc::clone(&state.bootstrap_write),
        requested,
        install_dir,
        device_settings,
    )
}

fn set_data_root_with_runtime(
    runtime_paths: RuntimePaths,
    operation_admission: OperationAdmission,
    bootstrap_write: Arc<Mutex<()>>,
    requested: PathBuf,
    install_dir: PathBuf,
) -> Result<DataRootMigrationView, String> {
    set_data_root_with_runtime_guarded(
        runtime_paths,
        operation_admission,
        bootstrap_write,
        requested,
        install_dir,
        None,
    )
}

fn initialize_onboarding_data_root_with_runtime(
    runtime_paths: RuntimePaths,
    operation_admission: OperationAdmission,
    bootstrap_write: Arc<Mutex<()>>,
    requested: PathBuf,
    install_dir: PathBuf,
    device_settings: DeviceSettingsStore,
) -> Result<OnboardingDataRootView, String> {
    let migration = set_data_root_with_runtime_guarded(
        runtime_paths,
        operation_admission,
        bootstrap_write,
        requested,
        install_dir,
        Some(device_settings),
    )?;
    Ok(OnboardingDataRootView {
        data_root: migration.data_root,
    })
}

fn set_data_root_with_runtime_guarded(
    runtime_paths: RuntimePaths,
    operation_admission: OperationAdmission,
    bootstrap_write: Arc<Mutex<()>>,
    requested: PathBuf,
    install_dir: PathBuf,
    onboarding_store: Option<DeviceSettingsStore>,
) -> Result<DataRootMigrationView, String> {
    let _admission_maintenance = operation_admission
        .begin_maintenance()?
        .wait_until_ready(Duration::from_secs(30))?;
    if let Some(store) = onboarding_store.as_ref() {
        if store.load()?.onboarding_completed {
            return Err("onboarding_already_completed".to_string());
        }
    }
    {
        let paths = runtime_paths.acquire()?;
        let mut connection = open_database(&paths.database)?;
        assert_schema_v5(&connection)?;
        QueueStore::cancel_all_queued_jobs(
            &mut connection,
            &now_iso(),
            "maintenance_cancelled_before_start",
        )?;
    }
    let maintenance = runtime_paths.begin_maintenance()?;
    let current_paths = maintenance.current_paths().clone();
    let _bootstrap_process = BootstrapProcessGuard::acquire(&current_paths.bootstrap_dir)?;
    let _bootstrap_write = bootstrap_write
        .lock()
        .map_err(|_| "bootstrap_lock_poisoned".to_string())?;
    let context = DataRootContext {
        app_data_dir: current_paths.data_root.clone(),
        install_dir,
    };
    let normalized =
        normalize_requested_root(&context, &requested).map_err(|error| error.code().to_string())?;
    if let Err(error) = probe_data_root_permissions(&normalized.path) {
        let _ = normalized.release_target_ownership();
        return Err(error.code().to_string());
    }
    let bootstrap = match read_bootstrap(&current_paths.bootstrap_dir) {
        Ok(Some(bootstrap)) => bootstrap,
        Ok(None) => {
            let _ = normalized.release_target_ownership();
            return Err("bootstrap_missing".to_string());
        }
        Err(error) => {
            let _ = normalized.release_target_ownership();
            return Err(error);
        }
    };
    if !same_windows_path(&bootstrap.current_root.path, &current_paths.data_root) {
        let _ = normalized.release_target_ownership();
        return Err("data_root_source_mismatch".to_string());
    }
    let retained_source = bootstrap.current_root.clone();
    let migration = migrate_data_root(
        &retained_source,
        &normalized,
        &current_paths.bootstrap_dir,
        &MigrationHooks::default(),
    )
    .map_err(|error| error.code().to_string())?;
    let new_paths = AppPaths::from_roots(
        current_paths.bootstrap_dir,
        migration.bootstrap.current_root.path.clone(),
        current_paths.resource_dir,
    );
    let data_root = new_paths.data_root.clone();
    maintenance.commit(new_paths)?;
    Ok(DataRootMigrationView {
        data_root,
        retained_source,
    })
}

fn delete_retained_data_root_with_runtime(
    runtime_paths: RuntimePaths,
    bootstrap_write: Arc<Mutex<()>>,
    requested: PathBuf,
) -> Result<(), String> {
    let paths = runtime_paths.acquire()?;
    let _bootstrap_process = BootstrapProcessGuard::acquire(&paths.bootstrap_dir)?;
    let _bootstrap_write = bootstrap_write
        .lock()
        .map_err(|_| "bootstrap_lock_poisoned".to_string())?;
    let bootstrap =
        read_bootstrap(&paths.bootstrap_dir)?.ok_or_else(|| "bootstrap_missing".to_string())?;
    let expected: DataRootRefV1 = bootstrap
        .retained_data_roots
        .iter()
        .find(|retained| same_windows_path(&retained.root.path, &requested))
        .map(|retained| retained.root.clone())
        .ok_or_else(|| "data_root_not_retained".to_string())?;
    delete_retained_data_root_storage(&paths.bootstrap_dir, &expected)
        .map(|_| ())
        .map_err(|error| error.code().to_string())
}

#[tauri::command]
fn get_app_settings(
    state: State<'_, AppState>,
    account_id: Option<i64>,
) -> Result<AppSettingsView, String> {
    require_normal_runtime(state.inner())?;
    get_app_settings_from_state(state.inner(), account_id, current_install_dir()?)
}

#[tauri::command]
fn update_global_settings(
    state: State<'_, AppState>,
    settings: GlobalSettingsV2,
) -> Result<u64, String> {
    require_normal_runtime(state.inner())?;
    update_global_settings_from_state(state.inner(), settings)
}

#[tauri::command]
fn update_workspace_settings(
    state: State<'_, AppState>,
    expected_revision: u64,
    settings: WorkspaceSettingsV1,
) -> Result<u64, String> {
    require_normal_runtime(state.inner())?;
    update_workspace_settings_from_state(state.inner(), expected_revision, settings)
}

fn cover_cache_policy(settings: &GlobalSettingsV2) -> CacheCleanupPolicyV1 {
    CacheCleanupPolicyV1 {
        max_bytes: settings
            .cover_cache
            .cleanup_by_capacity_enabled
            .then(|| settings.cover_cache.capacity_limit_mib.saturating_mul(1024 * 1024)),
        max_age_days: settings
            .cover_cache
            .cleanup_by_age_enabled
            .then_some(settings.cover_cache.max_age_days),
    }
}

fn cover_cache_mode(settings: &GlobalSettingsV2) -> CacheCleanupModeV1 {
    match settings.cover_cache.cleanup_list_mode {
        CoverCacheCleanupModeV1::KeepListed => CacheCleanupModeV1::KeepListed,
        CoverCacheCleanupModeV1::CleanListed => CacheCleanupModeV1::CleanListed,
    }
}

#[tauri::command]
fn get_cover_cache_task(state: State<'_, AppState>) -> Result<Option<CacheTaskViewV1>, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    latest_cache_task(&connection)
}

#[tauri::command]
fn pause_cover_cache_task(
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<CacheTaskViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    pause_cache_task(&connection, task_id)
}

#[tauri::command]
fn resume_cover_cache_task(
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<CacheTaskViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    let task = resume_cache_task(&connection, task_id)?;
    if let Some(worker) = state.cover_cache_worker.get() {
        worker.wake();
    }
    Ok(task)
}

#[tauri::command]
fn cancel_cover_cache_task(
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<CacheTaskViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    request_cache_task_cancel(&connection, task_id, &now_iso())
}

#[tauri::command]
fn retry_cover_cache_task(
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<CacheTaskViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    let task = retry_cache_task(&mut connection, task_id, &now_iso())?;
    if let Some(worker) = state.cover_cache_worker.get() {
        worker.wake();
    }
    Ok(task)
}

#[tauri::command]
fn inspect_cover_cache(state: State<'_, AppState>) -> Result<CacheInspectionV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    inspect_cache(&connection, &paths.covers)
}

#[tauri::command]
fn list_cover_cache_rules(state: State<'_, AppState>) -> Result<Vec<CacheRuleItemV1>, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    list_cache_rule_items_storage(&connection)
}

#[tauri::command]
fn search_cover_cache_candidates(
    state: State<'_, AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<CacheRuleItemV1>, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    search_cache_rule_candidates_storage(&connection, &query, limit)
}

#[tauri::command]
fn add_cover_cache_rule(state: State<'_, AppState>, video_id: i64) -> Result<(), String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    add_cache_rule_item_storage(&connection, video_id, &now_iso())
}

#[tauri::command]
fn remove_cover_cache_rule(state: State<'_, AppState>, video_id: i64) -> Result<bool, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    remove_cache_rule_item(&connection, video_id)
}

#[tauri::command]
fn clear_cover_cache_rules(state: State<'_, AppState>) -> Result<u64, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    clear_cache_rule_items_storage(&connection)
}

#[tauri::command]
fn preview_cover_cache_cleanup(
    state: State<'_, AppState>,
) -> Result<CacheCleanupPreviewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    let settings = load_portable_settings(&paths.data_root)?.global;
    preview_configured_cache_cleanup(
        &connection,
        cover_cache_mode(&settings),
        &cover_cache_policy(&settings),
        &now_iso(),
    )
}

#[tauri::command]
fn run_cover_cache_cleanup(
    state: State<'_, AppState>,
) -> Result<CacheCleanupResultV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    let settings = load_portable_settings(&paths.data_root)?.global;
    execute_configured_cache_cleanup(
        &mut connection,
        &paths.covers,
        cover_cache_mode(&settings),
        &cover_cache_policy(&settings),
        &now_iso(),
    )
}

#[tauri::command]
fn cleanup_broken_cover_cache(
    state: State<'_, AppState>,
) -> Result<CacheCleanupResultV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    cleanup_broken_cache_entries(&mut connection, &paths.covers)
}

#[tauri::command]
fn update_account_settings(
    state: State<'_, AppState>,
    account_id: i64,
    settings: AccountSettingsV2,
) -> Result<u64, String> {
    require_normal_runtime(state.inner())?;
    update_account_settings_from_state(state.inner(), account_id, settings)
}

#[tauri::command]
fn import_legacy_settings(
    state: State<'_, AppState>,
    account_id: Option<i64>,
    legacy: LegacyFrontendSettingsV1,
) -> Result<u64, String> {
    require_normal_runtime(state.inner())?;
    import_legacy_settings_from_state(state.inner(), account_id, legacy)
}

#[tauri::command]
async fn choose_data_root(state: State<'_, AppState>) -> Result<Option<PathBuf>, String> {
    require_normal_runtime(state.inner())?;
    tauri::async_runtime::spawn_blocking(|| rfd::FileDialog::new().pick_folder())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn choose_backup_directory(state: State<'_, AppState>) -> Result<Option<PathBuf>, String> {
    require_normal_runtime(state.inner())?;
    tauri::async_runtime::spawn_blocking(|| rfd::FileDialog::new().pick_folder())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_backup_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    path: Option<PathBuf>,
    expected_device_revision: u64,
) -> Result<DeviceSettingsViewV1, String> {
    require_normal_runtime(state.inner())?;
    let updated =
        set_backup_directory_runtime(state.inner(), path, expected_device_revision)?;
    let _ = app.emit("desktop:changed", ());
    Ok(updated)
}

#[tauri::command]
async fn choose_backup_export_path(
    state: State<'_, AppState>,
) -> Result<Option<PathBuf>, String> {
    require_normal_runtime(state.inner())?;
    let file_name = manual_backup_file_name(Utc::now());
    tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .add_filter(format!("{} 备份", douyin_history_archive::uninstall::PRODUCT_NAME), &["emily-backup"])
            .set_file_name(file_name)
            .save_file()
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn export_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    destination: PathBuf,
) -> Result<BackupArchiveSummaryV1, String> {
    require_normal_runtime(state.inner())?;
    let runtime_paths = state.runtime_paths()?;
    let admission = state.operation_admission.clone();
    let background_tasks = state.background_tasks.clone();
    let created_at = now_iso();
    let summary = tauri::async_runtime::spawn_blocking(move || {
        run_backup_with_runtime(
            &runtime_paths,
            &admission,
            &background_tasks,
            &destination,
            &created_at,
            env!("CARGO_PKG_VERSION"),
            Duration::from_secs(120),
        )
    })
    .await
    .map_err(|_| "backup_task_join_failed".to_string())??;
    let _ = app.emit("backup:changed", &summary);
    Ok(summary)
}

#[tauri::command]
async fn choose_backup_import_path(
    state: State<'_, AppState>,
) -> Result<Option<PathBuf>, String> {
    require_normal_runtime(state.inner())?;
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter(format!("{} 备份", douyin_history_archive::uninstall::PRODUCT_NAME), &["emily-backup"])
            .pick_file()
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn inspect_backup_import(
    state: State<'_, AppState>,
    archive_path: PathBuf,
) -> Result<BackupArchiveSummaryV1, String> {
    require_normal_runtime(state.inner())?;
    let runtime_paths = state.runtime_paths()?;
    let background_tasks = state.background_tasks.clone();
    tauri::async_runtime::spawn_blocking(move || {
        inspect_backup_for_restore(InspectRestoreRequestV1 {
            runtime_paths: &runtime_paths,
            background_tasks: &background_tasks,
            archive_path: &archive_path,
        })
    })
    .await
    .map_err(|_| "backup_inspection_task_join_failed".to_string())?
}

#[tauri::command]
async fn restore_backup_replace(
    app: AppHandle,
    state: State<'_, AppState>,
    archive_path: PathBuf,
) -> Result<ReplaceRestoreResultV1, String> {
    require_normal_runtime(state.inner())?;
    let runtime_paths = state.runtime_paths()?;
    let admission = state.operation_admission.clone();
    let background_tasks = state.background_tasks.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        restore_backup_replace_service(ReplaceRestoreRequestV1 {
            runtime_paths: &runtime_paths,
            admission: &admission,
            background_tasks: &background_tasks,
            archive_path: &archive_path,
            now: Utc::now(),
            application_version: env!("CARGO_PKG_VERSION"),
            wait_timeout: Duration::from_secs(120),
        })
    })
    .await
    .map_err(|_| "backup_restore_task_join_failed".to_string())??;
    let _ = wake_scheduler_and_queue(state.inner());
    let _ = app.emit("restore:changed", &result);
    let _ = app.emit("desktop:changed", ());
    Ok(result)
}

#[tauri::command]
async fn prepare_backup_merge(
    state: State<'_, AppState>,
    archive_path: PathBuf,
) -> Result<MergePlanV1, String> {
    require_normal_runtime(state.inner())?;
    let runtime_paths = state.runtime_paths()?;
    let admission = state.operation_admission.clone();
    let background_tasks = state.background_tasks.clone();
    tauri::async_runtime::spawn_blocking(move || {
        prepare_backup_merge_service(MergePlanRequestV1 {
            runtime_paths: &runtime_paths,
            admission: &admission,
            background_tasks: &background_tasks,
            archive_path: &archive_path,
            wait_timeout: Duration::from_secs(120),
        })
    })
    .await
    .map_err(|_| "backup_merge_plan_task_join_failed".to_string())?
}

#[tauri::command]
async fn restore_backup_merge(
    app: AppHandle,
    state: State<'_, AppState>,
    archive_path: PathBuf,
    expected_plan: MergePlanV1,
    folder_resolutions: BTreeMap<String, LocalFolderMergeResolutionV1>,
    settings_resolution: SettingsMergeResolutionV1,
) -> Result<MergeRestoreResultV1, String> {
    require_normal_runtime(state.inner())?;
    let runtime_paths = state.runtime_paths()?;
    let admission = state.operation_admission.clone();
    let background_tasks = state.background_tasks.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        restore_backup_merge_service(MergeRestoreRequestV1 {
            runtime_paths: &runtime_paths,
            admission: &admission,
            background_tasks: &background_tasks,
            archive_path: &archive_path,
            expected_plan: &expected_plan,
            folder_resolutions: &folder_resolutions,
            settings_resolution: &settings_resolution,
            now: Utc::now(),
            application_version: env!("CARGO_PKG_VERSION"),
            wait_timeout: Duration::from_secs(120),
        })
    })
    .await
    .map_err(|_| "backup_merge_task_join_failed".to_string())??;
    let _ = wake_scheduler_and_queue(state.inner());
    let _ = app.emit("restore:changed", &result);
    let _ = app.emit("desktop:changed", ());
    Ok(result)
}

#[tauri::command]
fn validate_data_root(
    state: State<'_, AppState>,
    path: PathBuf,
) -> Result<DataRootValidationView, String> {
    require_normal_runtime(state.inner())?;
    validate_data_root_from_state(state.inner(), &path, &current_install_dir()?)
}

#[tauri::command]
async fn set_data_root(
    state: State<'_, AppState>,
    path: PathBuf,
) -> Result<DataRootMigrationView, String> {
    require_normal_runtime(state.inner())?;
    let runtime_paths = state.runtime_paths()?;
    let operation_admission = state.operation_admission.clone();
    let bootstrap_write = Arc::clone(&state.bootstrap_write);
    let install_dir = current_install_dir()?;
    let migration = tauri::async_runtime::spawn_blocking(move || {
        set_data_root_with_runtime(
            runtime_paths,
            operation_admission,
            bootstrap_write,
            path,
            install_dir,
        )
    })
    .await
    .map_err(|error| error.to_string())??;
    if let Some(worker) = state.cover_cache_worker.get() {
        worker.wake();
    }
    Ok(migration)
}

#[tauri::command]
async fn initialize_onboarding_data_root(
    state: State<'_, AppState>,
    path: PathBuf,
) -> Result<OnboardingDataRootView, String> {
    require_normal_runtime(state.inner())?;
    let runtime_paths = state.runtime_paths()?;
    let operation_admission = state.operation_admission.clone();
    let bootstrap_write = Arc::clone(&state.bootstrap_write);
    let device_settings = state
        .device_settings
        .get()
        .cloned()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?;
    let install_dir = current_install_dir()?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        initialize_onboarding_data_root_with_runtime(
            runtime_paths,
            operation_admission,
            bootstrap_write,
            path,
            install_dir,
            device_settings,
        )
    })
    .await
    .map_err(|error| error.to_string())??;
    if let Some(worker) = state.cover_cache_worker.get() {
        worker.wake();
    }
    Ok(result)
}

#[tauri::command]
async fn delete_retained_data_root(
    state: State<'_, AppState>,
    path: PathBuf,
) -> Result<(), String> {
    require_normal_runtime(state.inner())?;
    let runtime_paths = state.runtime_paths()?;
    let bootstrap_write = Arc::clone(&state.bootstrap_write);
    tauri::async_runtime::spawn_blocking(move || {
        delete_retained_data_root_with_runtime(runtime_paths, bootstrap_write, path)
    })
    .await
    .map_err(|error| error.to_string())?
}

struct RunningSync {
    _path_lease: Option<BackgroundPathLease>,
    _browser_lease: Option<BrowserOperationLease>,
    paths: AppPaths,
    account_id: i64,
    job_id: i64,
    profile_key: String,
    executable: PathBuf,
    phase: SyncPhase,
    phase_attempt_id: String,
    child: Option<Arc<Mutex<OwnedCollectorProcess>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunningSyncIdentity {
    account_id: i64,
    job_id: i64,
    phase: SyncPhase,
    phase_attempt_id: String,
}

impl RunningSync {
    fn identity(&self) -> RunningSyncIdentity {
        RunningSyncIdentity {
            account_id: self.account_id,
            job_id: self.job_id,
            phase: self.phase,
            phase_attempt_id: self.phase_attempt_id.clone(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncResult {
    account_id: i64,
    job_id: i64,
    status: String,
    inserted_videos: i64,
    inserted_watch_records: i64,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProgressFile {
    schema_version: i64,
    account_id: i64,
    sync_job_id: i64,
    phase: String,
    record_count: i64,
    elapsed_ms: i64,
    retry_count: i64,
    can_cancel: bool,
}

type DiagnosticFile = CollectorDiagnostic;

#[tauri::command]
fn list_history(
    state: State<'_, AppState>,
    account_id: i64,
    query: Option<String>,
    limit: usize,
    offset: usize,
) -> Result<ListHistoryResult, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    require_account_status(
        &connection,
        account_id,
        &["active", "login_expired", "identity_conflict"],
    )?;
    list_history_storage(&connection, account_id, query.as_deref(), limit, offset)
}

#[tauri::command]
fn list_content(
    state: State<'_, AppState>,
    account_id: i64,
    source: SyncSourceV1,
    query: Option<String>,
    include_remote_removed: bool,
    sort: Option<ContentSortV1>,
    limit: usize,
    offset: usize,
) -> Result<douyin_history_archive::ListContentResult, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    require_account_status(
        &connection,
        account_id,
        &["active", "login_expired", "identity_conflict"],
    )?;
    list_content_storage(
        &connection,
        account_id,
        source,
        query.as_deref(),
        include_remote_removed,
        sort.unwrap_or(ContentSortV1::Default),
        limit,
        offset,
    )
}

#[tauri::command]
fn get_overview(
    state: State<'_, AppState>,
    account_id: i64,
    source: OverviewSourceFilterV1,
    range: OverviewRangeV1,
    include_remote_removed: bool,
) -> Result<OverviewViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    require_account_status(
        &connection,
        account_id,
        &["active", "login_expired", "identity_conflict", "archived"],
    )?;
    douyin_history_archive::query_overview_at(
        &connection,
        account_id,
        source,
        range,
        include_remote_removed,
        Utc::now().timestamp_millis(),
    )
}

#[tauri::command]
fn get_overview_scope(
    state: State<'_, AppState>,
    account_scope: AccountScopeV1,
    source: OverviewSourceFilterV1,
    range: OverviewRangeV1,
    include_remote_removed: bool,
) -> Result<OverviewViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    douyin_history_archive::query_overview_scope_at(
        &connection,
        &account_scope,
        source,
        range,
        include_remote_removed,
        Utc::now().timestamp_millis(),
    )
}

#[tauri::command]
fn get_annual_review(
    state: State<'_, AppState>,
    query: AnnualReviewQueryV1,
) -> Result<AnnualReviewViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    douyin_history_archive::query_annual_review(&connection, query)
}

#[tauri::command]
fn open_content_url(url: String) -> Result<(), String> {
    open_content_url_in_browser(&url)
}

fn emit_update_state(app: &AppHandle, runtime: &NativeUpdateRuntime) {
    if let Ok(snapshot) = runtime.snapshot() {
        let _ = app.emit("update:changed", snapshot);
    }
}

fn build_native_updater(
    app: &AppHandle,
    runtime: &NativeUpdateRuntime,
) -> Result<tauri_plugin_updater::Updater, String> {
    let configuration = runtime.configuration()?;
    app.updater_builder()
        .endpoints(vec![configuration.endpoint])
        .map_err(|_| "invalid_update_endpoint".to_string())?
        .pubkey(configuration.pubkey)
        .build()
        .map_err(|_| "invalid_update_configuration".to_string())
}

async fn run_update_check(
    app: &AppHandle,
    runtime: &NativeUpdateRuntime,
) -> Result<UpdateStateViewV1, String> {
    runtime.begin_check()?;
    emit_update_state(app, runtime);
    let updater = match build_native_updater(app, runtime) {
        Ok(updater) => updater,
        Err(error) => {
            runtime.fail(error.clone())?;
            emit_update_state(app, runtime);
            return Err(error);
        }
    };
    match updater.check().await {
        Ok(update) => runtime.complete_check(update)?,
        Err(_) => {
            let error = "update_check_failed".to_string();
            runtime.fail(error.clone())?;
            emit_update_state(app, runtime);
            return Err(error);
        }
    }
    emit_update_state(app, runtime);
    runtime.snapshot()
}

async fn run_update_download(
    app: &AppHandle,
    runtime: &NativeUpdateRuntime,
) -> Result<UpdateStateViewV1, String> {
    let update = runtime.begin_download()?;
    emit_update_state(app, runtime);
    let progress_runtime = runtime.clone();
    let progress_app = app.clone();
    let mut last_emit = Instant::now();
    let result = update
        .download(
            move |chunk, total| {
                let _ = progress_runtime.record_download_chunk(chunk, total);
                if last_emit.elapsed() >= Duration::from_millis(250) {
                    emit_update_state(&progress_app, &progress_runtime);
                    last_emit = Instant::now();
                }
            },
            || {},
        )
        .await;
    match result {
        Ok(bytes) => runtime.complete_download(bytes)?,
        Err(_) => {
            let error = "update_download_failed".to_string();
            runtime.fail(error.clone())?;
            emit_update_state(app, runtime);
            return Err(error);
        }
    }
    emit_update_state(app, runtime);
    runtime.snapshot()
}

fn spawn_automatic_update_check(app: &AppHandle) {
    let state = app.state::<AppState>();
    let settings = state
        .acquire_paths()
        .and_then(|paths| load_portable_settings(&paths.data_root));
    let Ok(settings) = settings else {
        eprintln!("automatic_update_settings_unavailable");
        return;
    };
    if !settings.global.updates.automatic_check {
        return;
    }
    let automatic_download = settings.global.updates.automatic_download;
    let runtime = state.update_runtime.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if run_update_check(&app, &runtime).await.is_ok()
            && automatic_download
            && runtime
                .snapshot()
                .map(|snapshot| snapshot.can_download)
                .unwrap_or(false)
        {
            let _ = run_update_download(&app, &runtime).await;
        }
    });
}

fn create_pre_update_safety_backup(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let operation_maintenance = state
        .operation_admission
        .begin_maintenance()?
        .wait_until_ready(Duration::from_secs(30))?;
    let runtime_paths = state.runtime_paths()?;
    let path_maintenance = runtime_paths.begin_maintenance()?;
    let paths = path_maintenance.current_paths().clone();
    fs::create_dir_all(&paths.backups).map_err(|_| "update_backup_failed".to_string())?;
    let settings = load_portable_settings(&paths.data_root)
        .map_err(|_| "update_backup_failed".to_string())?;
    let now = Utc::now();
    let file_name = manual_backup_file_name(now).replacen(
        "emily-backup-",
        "emily-pre-update-",
        1,
    );
    let destination = paths.backups.join(file_name);
    douyin_history_archive::create_application_backup(
        &paths.database,
        &settings,
        &destination,
        &now.to_rfc3339_opts(SecondsFormat::Millis, true),
        &app.package_info().version.to_string(),
    )
    .map_err(|_| "update_backup_failed".to_string())?;
    drop(path_maintenance);
    operation_maintenance.finish();
    Ok(())
}

fn prepare_update_install(app: &AppHandle) -> Result<(), String> {
    create_pre_update_safety_backup(app)?;
    match perform_controlled_exit(app)? {
        ExitCompletionV1::Clean => Ok(()),
        ExitCompletionV1::HardTimeout => Err("update_shutdown_timeout".to_string()),
    }
}

#[tauri::command]
fn get_update_state(state: State<'_, AppState>) -> Result<UpdateStateViewV1, String> {
    state.update_runtime.snapshot()
}

#[tauri::command]
async fn check_for_updates(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateStateViewV1, String> {
    require_normal_runtime(state.inner())?;
    let runtime = state.update_runtime.clone();
    run_update_check(&app, &runtime).await
}

#[tauri::command]
async fn download_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateStateViewV1, String> {
    require_normal_runtime(state.inner())?;
    let runtime = state.update_runtime.clone();
    run_update_download(&app, &runtime).await
}

#[tauri::command]
async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateStateViewV1, String> {
    require_normal_runtime(state.inner())?;
    let runtime = state.update_runtime.clone();
    let (update, bytes) = runtime.begin_install()?;
    emit_update_state(&app, &runtime);
    let app_for_prepare = app.clone();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        prepare_update_install(&app_for_prepare)
    })
    .await
    .map_err(|_| "update_shutdown_failed".to_string())?;
    if let Err(error) = prepared {
        runtime.restore_install_payload(bytes, error.clone())?;
        emit_update_state(&app, &runtime);
        return Err(error);
    }
    if update.install(&bytes).is_err() {
        let error = "update_install_failed".to_string();
        runtime.restore_install_payload(bytes, error.clone())?;
        emit_update_state(&app, &runtime);
        return Err(error);
    }
    runtime.snapshot()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DownloadCreateJobCommandRequest {
    account_id: i64,
    selection: DownloadSelection,
    destination_root: String,
    requested_quality: DownloadQuality,
}

fn download_command_error(code: impl Into<String>, message: impl Into<String>) -> DownloadError {
    DownloadError {
        code: code.into(),
        message: message.into(),
    }
}

fn active_download_service(state: &AppState) -> Result<Arc<DownloadService>, DownloadError> {
    require_normal_runtime(state).map_err(|error| {
        download_command_error("download_runtime_unavailable", error)
    })?;
    state.download_service.get().cloned().ok_or_else(|| {
        download_command_error("download_runtime_unavailable", "下载服务尚未初始化")
    })
}

fn download_owner_for_account(
    state: &AppState,
    account_id: i64,
) -> Result<DownloadJobOwner, DownloadError> {
    let paths = state
        .acquire_paths()
        .map_err(|error| download_command_error("download_storage_failed", error))?;
    let connection = open_database(&paths.database)
        .map_err(|error| download_command_error("download_storage_failed", error))?;
    require_account_status(&connection, account_id, &["active"])
        .map_err(|error| download_command_error("download_account_unavailable", error))?;
    let profile_key = account_profile_key(&connection, account_id)
        .map_err(|error| download_command_error("download_account_unavailable", error))?;
    Ok(DownloadJobOwner {
        account_id,
        profile_key,
    })
}

#[tauri::command]
async fn download_create_job(
    state: State<'_, AppState>,
    request: DownloadCreateJobCommandRequest,
) -> Result<DownloadJobDetail, DownloadError> {
    let owner = download_owner_for_account(state.inner(), request.account_id)?;
    active_download_service(state.inner())?
        .create_job(
            owner,
            CreateDownloadJobRequest {
                selection: request.selection,
                destination_root: request.destination_root,
                requested_quality: request.requested_quality,
            },
        )
        .await
}

#[tauri::command]
async fn download_preview_selection(
    state: State<'_, AppState>,
    account_id: i64,
    selection: DownloadSelection,
) -> Result<DownloadPreview, DownloadError> {
    let paths = state
        .acquire_paths()
        .map_err(|error| download_command_error("download_storage_failed", error))?;
    let connection = open_database(&paths.database)
        .map_err(|error| download_command_error("download_storage_failed", error))?;
    require_account_status(
        &connection,
        account_id,
        &["active", "login_expired", "identity_conflict", "archived"],
    )
    .map_err(|error| download_command_error("download_account_unavailable", error))?;
    drop(connection);
    drop(paths);
    active_download_service(state.inner())?
        .preview_selection(account_id, selection)
        .await
}

#[tauri::command]
async fn download_list_jobs(
    state: State<'_, AppState>,
) -> Result<Vec<DownloadJobSummary>, DownloadError> {
    active_download_service(state.inner())?.list_jobs().await
}

#[tauri::command]
async fn download_get_job(
    state: State<'_, AppState>,
    job_id: i64,
) -> Result<DownloadJobDetail, DownloadError> {
    active_download_service(state.inner())?.get_job(job_id).await
}

#[tauri::command]
async fn download_pause_job(
    state: State<'_, AppState>,
    job_id: i64,
) -> Result<DownloadJobDetail, DownloadError> {
    active_download_service(state.inner())?.pause_job(job_id).await
}

#[tauri::command]
async fn download_resume_job(
    state: State<'_, AppState>,
    job_id: i64,
) -> Result<DownloadJobDetail, DownloadError> {
    active_download_service(state.inner())?.resume_job(job_id).await
}

#[tauri::command]
async fn download_cancel_job(
    state: State<'_, AppState>,
    job_id: i64,
) -> Result<DownloadJobDetail, DownloadError> {
    active_download_service(state.inner())?.cancel_job(job_id).await
}

#[tauri::command]
async fn download_retry_failed(
    state: State<'_, AppState>,
    job_id: i64,
) -> Result<DownloadJobDetail, DownloadError> {
    active_download_service(state.inner())?
        .retry_failed(job_id)
        .await
}

#[tauri::command]
async fn download_reveal_path(
    state: State<'_, AppState>,
    job_id: Option<i64>,
    item_id: Option<i64>,
) -> Result<(), DownloadError> {
    let target = active_download_service(state.inner())?
        .reveal_target(job_id, item_id)
        .await?;
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("explorer.exe");
        if target.select {
            command.arg(format!("/select,{}", target.path.display()));
        } else {
            command.arg(&target.path);
        }
        command
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|_| {
                download_command_error("download_reveal_failed", "无法打开资源管理器")
            })?;
    }
    #[cfg(not(target_os = "windows"))]
    let _ = target;
    Ok(())
}

#[tauri::command]
fn list_remote_folders(
    state: State<'_, AppState>,
    account_id: i64,
    include_remote_removed: bool,
) -> Result<Vec<douyin_history_archive::RemoteFolderViewV1>, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    list_remote_folders_storage(&connection, account_id, include_remote_removed)
}

#[tauri::command]
fn list_local_folders(
    state: State<'_, AppState>,
    account_id: i64,
) -> Result<Vec<douyin_history_archive::LocalFolderViewV1>, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    list_local_folders_storage(&connection, account_id)
}

#[tauri::command]
fn create_local_folder(
    state: State<'_, AppState>,
    account_id: i64,
    name: String,
    scope: LocalFolderScopeV1,
) -> Result<douyin_history_archive::LocalFolderViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    create_local_folder_storage(&mut connection, account_id, &name, scope)
}

#[tauri::command]
fn rename_local_folder(
    state: State<'_, AppState>,
    account_id: i64,
    folder_id: i64,
    name: String,
) -> Result<douyin_history_archive::LocalFolderViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    rename_local_folder_storage(&mut connection, account_id, folder_id, &name)
}

#[tauri::command]
fn set_local_folder_scope(
    state: State<'_, AppState>,
    account_id: i64,
    folder_id: i64,
    scope: LocalFolderScopeV1,
    owner_account_id: Option<i64>,
) -> Result<douyin_history_archive::LocalFolderViewV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    set_local_folder_scope_storage(
        &mut connection,
        account_id,
        folder_id,
        scope,
        owner_account_id,
    )
}

#[tauri::command]
fn delete_local_folder(
    state: State<'_, AppState>,
    account_id: i64,
    folder_id: i64,
) -> Result<(), String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    delete_local_folder_storage(&mut connection, account_id, folder_id)
}

#[tauri::command]
fn add_local_folder_item(
    state: State<'_, AppState>,
    account_id: i64,
    folder_id: i64,
    aweme_id: String,
) -> Result<douyin_history_archive::LocalFolderItemMutationV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    add_local_folder_item_storage(&mut connection, account_id, folder_id, &aweme_id)
}

#[tauri::command]
fn remove_local_folder_item(
    state: State<'_, AppState>,
    account_id: i64,
    folder_id: i64,
    aweme_id: String,
) -> Result<bool, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    remove_local_folder_item_storage(&mut connection, account_id, folder_id, &aweme_id)
}

#[tauri::command]
fn list_remote_folder_content(
    state: State<'_, AppState>,
    account_id: i64,
    remote_folder_id: String,
    query: Option<String>,
    include_remote_removed: bool,
    limit: usize,
    offset: usize,
) -> Result<douyin_history_archive::ListFolderContentResultV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    list_remote_folder_content_storage(
        &connection,
        account_id,
        &remote_folder_id,
        query.as_deref(),
        include_remote_removed,
        limit,
        offset,
    )
}

#[tauri::command]
fn list_local_folder_content(
    state: State<'_, AppState>,
    account_id: i64,
    folder_id: i64,
    query: Option<String>,
    limit: usize,
    offset: usize,
) -> Result<douyin_history_archive::ListFolderContentResultV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    list_local_folder_content_storage(
        &connection,
        account_id,
        folder_id,
        query.as_deref(),
        limit,
        offset,
    )
}

#[tauri::command]
fn delete_history_records(
    state: State<'_, AppState>,
    account_id: i64,
    ids: Vec<i64>,
) -> Result<DeleteHistoryResult, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    require_account_status(
        &connection,
        account_id,
        &["active", "login_expired", "identity_conflict"],
    )?;
    delete_history_by_ids(&mut connection, account_id, &ids)
}

#[tauri::command]
fn delete_history_by_query(
    state: State<'_, AppState>,
    account_id: i64,
    query: Option<String>,
) -> Result<DeleteHistoryResult, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    require_account_status(
        &connection,
        account_id,
        &["active", "login_expired", "identity_conflict"],
    )?;
    delete_history_by_query_storage(&mut connection, account_id, query.as_deref())
}

#[tauri::command]
fn get_sync_status(
    state: State<'_, AppState>,
    account_id: i64,
    job_id: i64,
) -> Result<SyncStatusV2, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    require_account_status(
        &connection,
        account_id,
        &["active", "login_expired", "identity_conflict"],
    )?;
    let mut status = load_sync_status_for_account(&connection, account_id, job_id)?;
    if let Some(running) = running_sync_identity(&state.running)? {
        let (lifecycle, progress) =
            read_running_sync_progress(&paths, &running).unwrap_or((SyncLifecycle::Running, None));
        status_with_running_progress(
            &mut status,
            (account_id, job_id),
            &running,
            lifecycle,
            progress,
        )?;
    }
    Ok(status)
}

struct RunningLogin {
    _browser_lease: Option<BrowserOperationLease>,
    paths: AppPaths,
    account_id: Option<i64>,
    profile_key: String,
    attempt_id: String,
    cancel_path: PathBuf,
    child: Arc<Mutex<OwnedCollectorProcess>>,
}

#[derive(Clone)]
struct LoginTarget {
    account_id: Option<i64>,
    profile_key: String,
    browser_preference: douyin_history_archive::device_settings::BrowserPreferenceV1,
}

fn prepare_new_login_target(
    paths: &AppPaths,
    attempt_id: &str,
    updated_at: &str,
    browser_preference: douyin_history_archive::device_settings::BrowserPreferenceV1,
) -> Result<LoginTarget, String> {
    let profile_key = Uuid::new_v4().simple().to_string();
    douyin_history_archive::login_attempt::create_login_attempt_state(
        paths,
        attempt_id,
        &profile_key,
        None,
        updated_at,
    )?;
    Ok(LoginTarget {
        account_id: None,
        profile_key,
        browser_preference,
    })
}

fn cleanup_durable_login_files(paths: &AppPaths, attempt_id: &str) {
    let candidates = [
        paths.login_attempt_state_path(attempt_id),
        paths.login_task_identity_path(attempt_id),
        paths.login_task_progress_path(attempt_id),
        paths.login_task_cancel_path(attempt_id),
        paths.login_task_browser_pid_path(attempt_id),
        paths.login_task_collector_identity_path(attempt_id),
        paths.login_task_diagnostic_path(attempt_id),
    ];
    for path in candidates.into_iter().flatten() {
        let _ = fs::remove_file(path);
    }
}

fn cleanup_durable_login_attempt(paths: &AppPaths, target: &LoginTarget, attempt_id: &str) {
    let candidates = [
        paths.login_attempt_state_path(attempt_id),
        paths.login_task_identity_path(attempt_id),
        paths.login_task_progress_path(attempt_id),
        paths.login_task_cancel_path(attempt_id),
        paths.login_task_browser_pid_path(attempt_id),
        paths.login_task_collector_identity_path(attempt_id),
    ];
    for path in candidates.into_iter().flatten() {
        let _ = fs::remove_file(path);
    }
    if target.account_id.is_none() {
        let _ = remove_unclaimed_profile(&paths.browser_profiles, &target.profile_key);
    }
}

fn complete_durable_login(
    paths: &AppPaths,
    target: &LoginTarget,
    attempt_id: &str,
    bootstrap_write: &Mutex<()>,
) -> Result<CompletedDurableLogin, String> {
    let envelope: LoginAttemptEnvelopeV1<ObservedHomeIdentityV1> =
        read_json(&paths.login_task_identity_path(attempt_id)?)?;
    envelope.validate_context(attempt_id, target.account_id, &target.profile_key)?;
    let record: BrowserProcessRecordV2 =
        read_json(&paths.login_task_browser_pid_path(attempt_id)?)?;
    let account_root = paths.account_browser_root(&target.profile_key)?;
    let expected_profile = douyin_history_archive::collector::browser_state::browser_profile_path(
        &account_root,
        record.browser_id,
    );
    if !browser_record_matches_expected_profile(&record, &expected_profile) {
        return Err("browser_profile_scope_mismatch".to_string());
    }
    if query_process_creation_time_100ns(record.pid).as_deref()
        == Some(record.creation_time_100ns.as_str())
    {
        return Err("browser_still_running".to_string());
    }
    let choice = BrowserChoice {
        browser_id: record.browser_id,
        executable_path: PathBuf::from(&record.executable_path),
        profile_path: expected_profile,
    };
    douyin_history_archive::collector::browser_runtime::persist_validation_with_id(
        &account_root,
        &choice,
        DOUYIN_HISTORY_URL,
        Some(attempt_id),
    )?;

    let identity = AccountIdentity {
        uid: envelope.payload.uid,
        sec_uid: envelope.payload.sec_uid,
        nickname: envelope.payload.nickname,
        avatar_url: envelope.payload.avatar_url,
    };
    let account_result = (|| {
        let mut connection = open_database(&paths.database)?;
        assert_schema_v5(&connection)?;
        if let Some(account_id) = target.account_id {
            let current = get_account_storage(&connection, account_id)?
                .ok_or_else(|| "account_not_found".to_string())?;
            if current.profile_key != target.profile_key {
                return Err("account_identity_mismatch".to_string());
            }
            finalize_account_identity(&mut connection, account_id, identity)
        } else {
            create_account_from_identity(&mut connection, &target.profile_key, identity)
        }
    })();
    let account = match account_result {
        Ok(account) => account,
        Err(error) => {
            if let Ok(Some(selection)) =
                douyin_history_archive::collector::browser_state::read_browser_selection(
                    &account_root,
                )
            {
                let _ = douyin_history_archive::collector::browser_state::remove_login_validation_if_matches(
                    &account_root,
                    &selection,
                    attempt_id,
                );
            }
            return Err(error);
        }
    };
    let mut active_account_warning = None;
    match bootstrap_write.lock() {
        Ok(_bootstrap_write) => match open_database(&paths.database).and_then(|connection| {
            set_active_account_storage(&paths.bootstrap_dir, &connection, account.id)
        }) {
            Ok(_) => {}
            Err(_) => active_account_warning = Some("bootstrap_update_failed".to_string()),
        },
        Err(_) => active_account_warning = Some("bootstrap_update_failed".to_string()),
    }
    cleanup_durable_login_files(paths, attempt_id);
    Ok(CompletedDurableLogin {
        account,
        active_account_warning,
    })
}

struct CompletedDurableLogin {
    account: AccountView,
    active_account_warning: Option<String>,
}

fn running_sync_identity(
    running: &Mutex<Option<RunningSync>>,
) -> Result<Option<RunningSyncIdentity>, String> {
    Ok(running
        .lock()
        .map_err(|_| "同步状态锁已损坏".to_string())?
        .as_ref()
        .map(RunningSync::identity))
}

fn read_running_sync_progress(
    paths: &AppPaths,
    running: &RunningSyncIdentity,
) -> Result<(SyncLifecycle, Option<ProgressFile>), String> {
    let orchestration = read_orchestration(paths, running.account_id, running.job_id)?;
    validate_running_orchestration(&orchestration, running)?;
    let path = paths.phase_progress_path(
        running.account_id,
        running.job_id,
        running.phase,
        &running.phase_attempt_id,
    )?;
    if !path.exists() {
        return Ok((orchestration.lifecycle, None));
    }
    Ok((
        orchestration.lifecycle,
        read_sync_phase_payload(&path, &orchestration).ok(),
    ))
}

fn status_with_running_progress(
    status: &mut SyncStatusV2,
    requested: (i64, i64),
    running: &RunningSyncIdentity,
    lifecycle: SyncLifecycle,
    progress: Option<ProgressFile>,
) -> Result<(), String> {
    if (running.account_id, running.job_id) != requested {
        return Ok(());
    }
    if let Some(progress) = progress.filter(|progress| {
        progress.schema_version == 3
            && progress.account_id == running.account_id
            && progress.sync_job_id == running.job_id
    }) {
        status.account_id = running.account_id;
        status.job_id = running.job_id;
        status.status = douyin_history_archive::sync::SyncJobStatusV1::Running;
        status.phase = Some(progress.phase);
        status.record_count = progress.record_count;
        status.elapsed_ms = progress.elapsed_ms;
        status.retry_count = progress.retry_count;
        status.can_cancel = progress.can_cancel;
    } else {
        status.account_id = running.account_id;
        status.job_id = running.job_id;
        status.status = douyin_history_archive::sync::SyncJobStatusV1::Running;
        status.phase = Some(match running.phase {
            SyncPhase::Collecting => "launching".to_string(),
            phase => phase.as_str().to_string(),
        });
        status.can_cancel = true;
    }
    if lifecycle == SyncLifecycle::Consuming {
        status.phase = Some("finalizing".to_string());
        status.can_cancel = false;
    }
    Ok(())
}

fn load_sync_status_for_account(
    connection: &Connection,
    account_id: i64,
    job_id: i64,
) -> Result<SyncStatusV2, String> {
    let view = QueueStore::get_job_view(connection, job_id).map_err(|error| {
        if error == "sync_job_not_found" {
            "sync_job_mismatch".to_string()
        } else {
            error
        }
    })?;
    if view.account_id != account_id {
        return Err("sync_job_mismatch".to_string());
    }
    let stored = QueueStore::get_stored_job(connection, job_id)?;
    Ok(SyncStatusV2 {
        account_id,
        job_id,
        source: view.source,
        source_instance_key: view.source_instance_key,
        trigger: view.trigger,
        status: view.status,
        queue_position: view.queue_position,
        inserted_videos: stored.inserted_videos,
        inserted_records: stored.inserted_records,
        inserted_watch_records: if view.source == SyncSourceV1::History {
            stored.inserted_records
        } else {
            0
        },
        record_count: 0,
        elapsed_ms: 0,
        retry_count: 0,
        phase: None,
        can_cancel: view.can_cancel,
        started_at: view.started_at,
        finished_at: view.finished_at,
        error_code: stored.error_code,
        error_message: stored.error_message,
        issues: view.issues,
    })
}

fn notify_account_state_changed(app: &AppHandle, state: &AppState) {
    if wake_scheduler_and_queue(state).is_err() {
        eprintln!("scheduler_wake_failed");
    }
    let _ = app.emit("sync:changed", ());
    let _ = app.emit("scheduler:changed", ());
}

#[tauri::command]
async fn begin_account_login(
    app: AppHandle,
    state: State<'_, AppState>,
    request: Option<BeginAccountLoginRequestV1>,
) -> Result<AccountView, String> {
    require_normal_runtime(state.inner())?;
    let operation = begin_browser_operation(state.inner(), BrowserOperationKind::Login, None)?;
    ensure_no_running_sync(&state)?;
    let paths = state.acquire_paths()?;
    prepare_app_directories(&paths)?;
    let attempt_id = Uuid::new_v4().simple().to_string();
    let target = prepare_new_login_target(
        &paths,
        &attempt_id,
        &now_iso(),
        request
            .and_then(|request| request.browser_preference)
            .unwrap_or(douyin_history_archive::device_settings::BrowserPreferenceV1::Auto),
    )?;
    let result = run_account_login(
        &paths,
        state.inner(),
        target.clone(),
        &attempt_id,
        operation,
    )
    .await;
    if result.is_err() {
        cleanup_durable_login_attempt(&paths, &target, &attempt_id);
    }
    if result.is_ok() {
        notify_account_state_changed(&app, state.inner());
    }
    result
}

#[tauri::command]
async fn resume_account_login(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: i64,
) -> Result<AccountView, String> {
    require_normal_runtime(state.inner())?;
    let operation =
        begin_browser_operation(state.inner(), BrowserOperationKind::Login, Some(account_id))?;
    ensure_no_running_sync(&state)?;
    let paths = state.acquire_paths()?;
    prepare_app_directories(&paths)?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let account = get_account_storage(&connection, account_id)?
        .ok_or_else(|| "account_not_found".to_string())?;
    if !account_can_resume_login(&account.status) {
        return Err("account_not_resumable".to_string());
    }
    drop(connection);
    let attempt_id = Uuid::new_v4().simple().to_string();
    let profile_key = account.profile_key;
    let browser_preference = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?
        .load()?
        .accounts
        .get(&profile_key)
        .cloned()
        .unwrap_or_default()
        .browser_preference;
    let target = LoginTarget {
        account_id: Some(account.id),
        profile_key,
        browser_preference,
    };
    douyin_history_archive::login_attempt::create_login_attempt_state(
        &paths,
        &attempt_id,
        &target.profile_key,
        target.account_id,
        &now_iso(),
    )?;
    let result = run_account_login(
        &paths,
        state.inner(),
        target.clone(),
        &attempt_id,
        operation,
    )
    .await;
    if result.is_err() {
        cleanup_durable_login_attempt(&paths, &target, &attempt_id);
    }
    if result.is_ok() {
        notify_account_state_changed(&app, state.inner());
    }
    result
}

async fn run_account_login(
    paths: &AppPaths,
    state: &AppState,
    target: LoginTarget,
    attempt_id: &str,
    operation: BrowserOperationLease,
) -> Result<AccountView, String> {
    let _background_task = state.background_tasks.register()?;
    let preparing = read_login_attempt_state(paths, attempt_id)?;
    let running_state = preparing.next(LoginAttemptLifecycle::Running, &now_iso())?;
    compare_and_swap_login_attempt(paths, &preparing, &running_state)?;
    let invocation = build_login_invocation(
        paths,
        std::env::current_exe().map_err(|error| error.to_string())?,
        target.account_id,
        &target.profile_key,
        attempt_id,
        target.browser_preference,
    )?;
    let child = match spawn_login_task_collector(paths, &target, attempt_id, invocation) {
        Ok(child) => child,
        Err(error) => {
            write_login_failure_log(paths, attempt_id, &error);
            return Err(error);
        }
    };
    let child = Arc::new(Mutex::new(child));
    {
        let mut running = state
            .running_login
            .lock()
            .map_err(|_| "登录状态锁已损坏".to_string())?;
        *running = Some(RunningLogin {
            _browser_lease: Some(operation.clone()),
            paths: paths.clone(),
            account_id: target.account_id,
            profile_key: target.profile_key.clone(),
            attempt_id: attempt_id.to_string(),
            cancel_path: paths.login_task_cancel_path(attempt_id)?,
            child: Arc::clone(&child),
        });
        if let Err(error) = child
            .lock()
            .map_err(|_| "登录进程锁已损坏".to_string())?
            .resume()
        {
            *running = None;
            return Err(error);
        }
    }
    let (exit_status, termination) =
        wait_for_login_child_with_watchdog(paths, &target, attempt_id, &child).await?;
    if let Ok(mut running) = state.running_login.lock() {
        if running.as_ref().is_some_and(|item| {
            item.account_id == target.account_id && item.attempt_id == attempt_id
        }) {
            *running = None;
        }
    }
    if exit_status.success() {
        let current = read_login_attempt_state(paths, attempt_id)?;
        let finalizing = current.next(LoginAttemptLifecycle::Finalizing, &now_iso())?;
        compare_and_swap_login_attempt(paths, &current, &finalizing)?;
        let completed = complete_durable_login(paths, &target, attempt_id, &state.bootstrap_write)?;
        if completed.active_account_warning.is_some() {
            eprintln!("bootstrap_update_failed");
        }
        if target.account_id.is_none() {
            let persisted = state.device_settings.get().map_or_else(
                || Err("device_settings_not_initialized".to_string()),
                |store| {
                    store.set_account_browser_preference(
                        &target.profile_key,
                        target.browser_preference,
                    )
                },
            );
            if persisted.is_err() {
                eprintln!("device_settings_update_failed");
            }
        }
        return Ok(completed.account);
    }
    let diagnostic = read_json::<LoginAttemptEnvelopeV1<LoginDiagnosticPayloadV1>>(
        &paths.login_task_diagnostic_path(attempt_id)?,
    )
    .ok()
    .filter(|diagnostic| {
        diagnostic
            .validate_context(attempt_id, target.account_id, &target.profile_key)
            .is_ok()
    });
    let diagnostic_error = diagnostic.map(|diagnostic| diagnostic.payload.error_code);
    let error = resolve_login_failure_code(termination, diagnostic_error);
    write_login_failure_log(paths, attempt_id, &error);
    Err(error)
}

#[tauri::command]
fn list_library_content(
    state: State<'_, AppState>,
    query: LibraryContentQueryV1,
) -> Result<ListLibraryContentResultV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    list_library_content_storage(&connection, &query)
}

#[tauri::command]
fn search_library_content(
    state: State<'_, AppState>,
    query: SearchContentQueryV1,
) -> Result<ListLibraryContentResultV1, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_latest(&connection)?;
    search_library_content_storage(&connection, &query)
}

fn resolve_login_failure_code(
    termination: Option<LoginTermination>,
    diagnostic_error: Option<String>,
) -> String {
    match termination {
        Some(LoginTermination::Cancelled) => "account_login_cancelled".to_string(),
        Some(LoginTermination::BrowserClosed) => "account_login_browser_closed".to_string(),
        Some(LoginTermination::Timeout) => "account_identity_unavailable".to_string(),
        Some(LoginTermination::Failed) | None => diagnostic_error
            .filter(|error| !error.trim().is_empty())
            .unwrap_or_else(|| "account_login_failed".to_string()),
    }
}

fn write_login_failure_log(paths: &AppPaths, attempt_id: &str, error: &str) {
    let safe_error = if error.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || "_:-".contains(character)
    }) {
        error
    } else {
        "account_login_failed"
    };
    let Ok(path) = paths.login_task_log_path(attempt_id) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(
        path,
        format!("timestamp={} error_code={safe_error}\n", now_iso()),
    );
}

fn login_identity_observed_for_shutdown(
    paths: &AppPaths,
    target: &LoginTarget,
    attempt_id: &str,
) -> bool {
    let Ok(path) = paths.login_task_identity_path(attempt_id) else {
        return false;
    };
    let Ok(envelope) = read_json::<LoginAttemptEnvelopeV1<ObservedHomeIdentityV1>>(&path) else {
        return false;
    };
    envelope
        .validate_context(attempt_id, target.account_id, &target.profile_key)
        .is_ok()
        && is_valid_douyin_home_url(&envelope.payload.page_url)
        && (envelope.payload.uid.is_some() || envelope.payload.sec_uid.is_some())
        && !envelope.payload.sources.is_empty()
}

async fn wait_for_login_child_with_watchdog(
    paths: &AppPaths,
    target: &LoginTarget,
    attempt_id: &str,
    child: &Arc<Mutex<OwnedCollectorProcess>>,
) -> Result<(ExitStatus, Option<LoginTermination>), String> {
    let started = Instant::now();
    let mut watchdog = LoginWatchdog::new(started, started + Duration::from_secs(600));
    let mut termination = None;
    loop {
        if let Some(status) = child
            .lock()
            .map_err(|_| "登录进程锁已损坏".to_string())?
            .try_wait()
            .map_err(|error| error.to_string())?
        {
            return Ok((status, termination));
        }

        let now = Instant::now();
        let event = read_json::<LoginAttemptEnvelopeV1<LoginProgressPayloadV1>>(
            &paths.login_task_progress_path(attempt_id)?,
        )
        .ok()
        .filter(|progress| {
            progress
                .validate_context(attempt_id, target.account_id, &target.profile_key)
                .is_ok()
        })
        .map(|progress| watchdog.observe_payload(&progress.payload, now))
        .filter(|event| *event != LoginWatchdogEvent::None)
        .or_else(|| {
            let browser_closed =
                recorded_login_task_browser_is_alive(paths, &target.profile_key, attempt_id)
                    .is_some_and(|alive| !alive);
            (browser_closed
                && !login_identity_observed_for_shutdown(paths, target, attempt_id))
            .then(|| watchdog.observe_browser_exit(now))
        })
        .unwrap_or_else(|| watchdog.tick(now));

        match event {
            LoginWatchdogEvent::None => {}
            LoginWatchdogEvent::BeginShutdown(reason) => {
                termination = Some(reason);
                let _ = fs::write(paths.login_task_cancel_path(attempt_id)?, b"cancel");
            }
            LoginWatchdogEvent::ForceBrowser => {
                let _ = terminate_recorded_login_task_browser_with(
                    paths,
                    &target.profile_key,
                    attempt_id,
                    terminate_recorded_browser_v2,
                );
            }
            LoginWatchdogEvent::ForceCollectorAndRelease => {
                let child = child.lock().map_err(|_| "登录进程锁已损坏".to_string())?;
                let _ = child.kill();
                let status = child.wait().map_err(|error| error.to_string())?;
                let _ = terminate_recorded_login_task_browser_with(
                    paths,
                    &target.profile_key,
                    attempt_id,
                    terminate_recorded_browser_v2,
                );
                return Ok((status, termination.or(Some(LoginTermination::Failed))));
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn recorded_login_task_browser_is_alive(
    paths: &AppPaths,
    profile_key: &str,
    attempt_id: &str,
) -> Option<bool> {
    let record: BrowserProcessRecordV2 =
        read_json(&paths.login_task_browser_pid_path(attempt_id).ok()?).ok()?;
    let account_root = paths.account_browser_root(profile_key).ok()?;
    let expected_profile = douyin_history_archive::collector::browser_state::browser_profile_path(
        &account_root,
        record.browser_id,
    );
    if !browser_record_matches_expected_profile(&record, &expected_profile) {
        return None;
    }
    match query_process_creation_time_100ns(record.pid) {
        Some(creation_time) if creation_time == record.creation_time_100ns => Some(true),
        Some(_) => None,
        None if local_debug_port_is_reachable(record.devtools_port) => Some(true),
        None => Some(false),
    }
}

#[tauri::command]
fn list_accounts(
    state: State<'_, AppState>,
    include_archived: Option<bool>,
) -> Result<Vec<AccountView>, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    list_command_accounts(&connection, include_archived.unwrap_or(false))
}

#[tauri::command]
fn get_active_account(state: State<'_, AppState>) -> Result<Option<AccountView>, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let _bootstrap_write = state
        .bootstrap_write
        .lock()
        .map_err(|_| "bootstrap_lock_poisoned".to_string())?;
    load_active_account_storage(&paths.bootstrap_dir, &connection)
}

#[tauri::command]
fn set_active_account(state: State<'_, AppState>, account_id: i64) -> Result<AccountView, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let _bootstrap_write = state
        .bootstrap_write
        .lock()
        .map_err(|_| "bootstrap_lock_poisoned".to_string())?;
    set_active_account_storage(&paths.bootstrap_dir, &connection, account_id)
}

#[tauri::command]
async fn cancel_account_login(state: State<'_, AppState>) -> Result<(), String> {
    require_normal_runtime(state.inner())?;
    cancel_running_login_task(state.inner(), None).await
}

#[tauri::command]
async fn cancel_pending_account(state: State<'_, AppState>, account_id: i64) -> Result<(), String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    cancel_running_login_task(state.inner(), Some(account_id)).await?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    cancel_pending_account_storage(&mut connection, account_id, &paths.browser_profiles)
}

#[tauri::command]
fn restore_account(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: i64,
) -> Result<AccountView, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let account = restore_account_storage(&mut connection, account_id)?;
    drop(connection);
    drop(paths);
    notify_account_state_changed(&app, state.inner());
    Ok(account)
}

#[tauri::command]
fn archive_account(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: i64,
) -> Result<(), String> {
    require_normal_runtime(state.inner())?;
    let _operation =
        begin_browser_operation(state.inner(), BrowserOperationKind::Login, Some(account_id))?;
    ensure_account_not_running(state.inner(), account_id)?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let _bootstrap_write = state
        .bootstrap_write
        .lock()
        .map_err(|_| "bootstrap_lock_poisoned".to_string())?;
    archive_account_storage(&mut connection, &paths.bootstrap_dir, account_id)?;
    drop(_bootstrap_write);
    drop(connection);
    drop(paths);
    drop(_operation);
    notify_account_state_changed(&app, state.inner());
    Ok(())
}

#[tauri::command]
fn delete_account(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: i64,
    folder_disposition: AccountFolderDisposition,
) -> Result<(), String> {
    require_normal_runtime(state.inner())?;
    let _operation =
        begin_browser_operation(state.inner(), BrowserOperationKind::Login, Some(account_id))?;
    ensure_account_not_running(state.inner(), account_id)?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let _bootstrap_write = state
        .bootstrap_write
        .lock()
        .map_err(|_| "bootstrap_lock_poisoned".to_string())?;
    delete_account_data(
        &mut connection,
        &paths.bootstrap_dir,
        &paths.data_root,
        &paths.browser_profiles,
        account_id,
        folder_disposition,
    )?;
    drop(_bootstrap_write);
    drop(connection);
    drop(paths);
    drop(_operation);
    notify_account_state_changed(&app, state.inner());
    Ok(())
}

fn sync_profile_key(job: &StoredSyncJobV1) -> Result<String, String> {
    let payload = serde_json::from_str::<SyncTaskPayloadV1>(&job.payload_json)
        .map_err(|_| "invalid_sync_job_payload".to_string())?;
    let frozen = freeze_task_payload(payload.clone())?;
    if frozen.payload_json != job.payload_json
        || frozen.payload_sha256 != job.snapshot.task_payload_sha256
        || frozen.equivalence_sha256 != job.equivalence_sha256
        || payload.source() != job.snapshot.task_key.source
        || payload.source_instance_key() != job.snapshot.task_key.source_instance_key
    {
        return Err("invalid_sync_job_payload".to_string());
    }
    Ok(payload.browser().profile_key.clone())
}

fn build_collect_invocation_for_job(
    paths: &AppPaths,
    executable: PathBuf,
    job: &StoredSyncJobV1,
    phase: SyncPhase,
    phase_attempt_id: &str,
) -> Result<CollectorInvocation, String> {
    match job.snapshot.task_key.source {
        SyncSourceV1::History => {
            build_history_collect_invocation(paths, executable, job, phase, phase_attempt_id)
        }
        SyncSourceV1::Likes | SyncSourceV1::Favorites | SyncSourceV1::RemoteFolder => {
            build_archive_collect_invocation(paths, executable, job, phase, phase_attempt_id)
        }
    }
}

fn prepare_collect_input_files(
    paths: &AppPaths,
    connection: &Connection,
    job: &StoredSyncJobV1,
    phase: SyncPhase,
    phase_attempt_id: &str,
) -> Result<(), String> {
    if job.snapshot.task_key.source == SyncSourceV1::History {
        return Ok(());
    }
    let payload = serde_json::from_str::<SyncTaskPayloadV1>(&job.payload_json)
        .map_err(|_| "invalid_sync_job_payload".to_string())?;
    if payload.archive_range()
        == Some(douyin_history_archive::settings_store::ArchiveListRangeV1::All)
        && payload.run_mode() == Some(ArchiveRunModeV1::Incremental)
    {
        let snapshot = build_archive_known_ids_snapshot(connection, job, phase, phase_attempt_id)?;
        write_json_atomically(
            &paths.phase_archive_known_ids_path(
                job.snapshot.task_key.account_id,
                job.snapshot.id,
                phase,
                phase_attempt_id,
            )?,
            &snapshot,
        )?;
    }
    Ok(())
}

fn start_queued_sync_execution(
    app: &AppHandle,
    context: QueuedSyncContext,
) -> Result<QueueRunningSyncIdentity, String> {
    let state = app.state::<AppState>();
    if state
        .running
        .lock()
        .map_err(|_| "同步状态锁已损坏".to_string())?
        .is_some()
    {
        return Err("sync_worker_running_conflict".to_string());
    }
    let path_lease = state.acquire_paths()?;
    let paths = (*path_lease).clone();
    prepare_app_directories(&paths)?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let job = QueueStore::get_stored_job(&connection, context.job.id)?;
    if job.snapshot != context.job
        || job.snapshot.status != douyin_history_archive::sync::SyncJobStatusV1::Running
    {
        return Err("sync_job_not_running".to_string());
    }
    let profile_key = sync_profile_key(&job)?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let phase = SyncPhase::Collecting;
    let phase_attempt_id = Uuid::new_v4().simple().to_string();
    let invocation = build_collect_invocation_for_job(
        &paths,
        executable.clone(),
        &job,
        phase,
        &phase_attempt_id,
    )?;
    prepare_collect_input_files(&paths, &connection, &job, phase, &phase_attempt_id)?;
    let identity = RunningSyncIdentity {
        account_id: context.job.task_key.account_id,
        job_id: context.job.id,
        phase,
        phase_attempt_id: phase_attempt_id.clone(),
    };
    let preparing = match create_orchestration(
        &paths,
        context.job.task_key.account_id,
        context.job.id,
        phase,
        &phase_attempt_id,
        &context.job.task_payload_sha256,
        None,
        &now_iso(),
    ) {
        Ok(preparing) => preparing,
        Err(error) => {
            cleanup_phase_files(&paths, &identity);
            return Err(error);
        }
    };
    let child = match spawn_collector(&paths, &identity, invocation) {
        Ok(child) => child,
        Err(error) => {
            cleanup_task_files(&paths, &identity);
            return Err(error);
        }
    };
    let process_id = child.pid();
    let child = Arc::new(Mutex::new(child));
    if let Err(error) = register_running_phase(
        &state.running,
        &paths,
        &preparing,
        RunningSync {
            _path_lease: Some(path_lease),
            _browser_lease: None,
            paths: paths.clone(),
            account_id: identity.account_id,
            job_id: identity.job_id,
            profile_key,
            executable,
            phase,
            phase_attempt_id,
            child: Some(Arc::clone(&child)),
        },
    ) {
        if let Ok(child) = child.lock() {
            let _ = child.kill();
        }
        cleanup_task_files(&paths, &identity);
        return Err(error);
    }
    if let Err(error) =
        spawn_sync_phase_monitor(app.clone(), paths.clone(), identity.clone(), child)
    {
        let _ = request_sync_cancellation(&paths, &identity, &now_iso());
        terminate_recorded_sync_browser_with(&paths, &identity, terminate_recorded_browser_v2);
        cleanup_phase_files(&paths, &identity);
        let _ = finish_live_sync_failure(
            state.inner(),
            &paths,
            identity.account_id,
            identity.job_id,
            "sync_monitor_start_failed",
            &error,
        );
        return Err(error);
    }
    Ok(QueueRunningSyncIdentity {
        job_id: context.job.id,
        process_id: Some(process_id),
    })
}

fn cancel_queued_sync_execution(
    app: &AppHandle,
    running_identity: &QueueRunningSyncIdentity,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let cleanup_registration = state.background_tasks.register()?;
    let (paths, identity, child) = {
        let running = state
            .running
            .lock()
            .map_err(|_| "同步状态锁已损坏".to_string())?;
        let running = running
            .as_ref()
            .filter(|running| running.job_id == running_identity.job_id)
            .ok_or_else(|| "sync_job_mismatch".to_string())?;
        let identity = running.identity();
        request_sync_cancellation(&running.paths, &identity, &now_iso())?;
        (
            running.paths.clone(),
            identity,
            running.child.as_ref().map(Arc::clone),
        )
    };
    thread::Builder::new()
        .name("emily-sync-cancel-cleanup".to_string())
        .spawn(move || {
            let _cleanup_registration = cleanup_registration;
            thread::sleep(Duration::from_secs(2));
            if let Some(child) = child {
                if let Ok(child) = child.lock() {
                    if child.try_wait().ok().flatten().is_none() {
                        let _ = child.kill();
                    }
                }
            }
            terminate_recorded_sync_browser_with(&paths, &identity, terminate_recorded_browser_v2);
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
async fn start_sync(
    state: State<'_, AppState>,
    account_id: i64,
    options: Option<SyncOptions>,
) -> Result<SyncResult, String> {
    require_normal_runtime(state.inner())?;
    let paths = state.acquire_paths()?;
    prepare_app_directories(&paths)?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    require_account_status(&connection, account_id, &["active"])?;
    let profile_key = account_profile_key(&connection, account_id)?;
    let account_key = portable_account_key_for_id(&connection, account_id)?;
    let portable = load_portable_settings(&paths.data_root)?;
    let account_settings = account_settings_for(&portable, Some(&account_key))?;
    let options = match options {
        Some(options) => options,
        None => sync_options_from_account_settings(&account_settings),
    };
    let frozen_at_ms = Utc::now().timestamp_millis();
    let (range, cutoff_ms) = match options.mode.as_str() {
        "days" => {
            let days = options
                .days
                .ok_or_else(|| "invalid_range_days".to_string())?;
            if !(1..=365).contains(&days) {
                return Err("invalid_range_days".to_string());
            }
            (
                HistoryRangeV1::Days { days },
                Some(douyin_history_archive::sync::shanghai_cutoff_ms(
                    frozen_at_ms,
                    days,
                )?),
            )
        }
        "bottom" if options.days.is_none() => (HistoryRangeV1::Bottom, None),
        _ => return Err("invalid_sync_mode".to_string()),
    };
    let scroll_speed = options
        .scroll_speed
        .unwrap_or(account_settings.scroll_speed);
    if scroll_speed == 0 {
        return Err("invalid_scroll_speed".to_string());
    }
    let hide_browser = options
        .hide_browser
        .unwrap_or(account_settings.hide_browser);
    let device = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?
        .load()?;
    let browser_preference = device
        .accounts
        .get(&profile_key)
        .cloned()
        .unwrap_or_default()
        .browser_preference;
    let browser =
        current_browser_execution_identity(&paths, &connection, account_id, browser_preference)?
            .ok_or_else(|| "login_validation_required".to_string())?;
    let payload = SyncTaskPayloadV1::History {
        range,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms,
            cutoff_ms,
        },
        scroll_speed,
        hide_browser,
        browser: browser.clone(),
    };
    let settings_snapshot = EnqueueSettingsSnapshotV1 {
        revision: portable.revision,
        accounts: std::collections::BTreeMap::from([(account_id, account_settings)]),
    };
    let device_snapshot = EnqueueDeviceSnapshotV1 {
        browsers: std::collections::BTreeMap::from([(account_id, browser)]),
    };
    let request = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::One,
        account_id: Some(account_id),
        source: Some(SyncSourceV1::History),
        source_instance_key: Some(String::new()),
        payload: Some(payload),
        full_reconcile_confirmation_sha256: None,
    };
    let capabilities = CapabilityRegistry::stage_3_3();
    let enqueued = EnqueueService::new(&mut connection).enqueue(
        &request,
        &settings_snapshot,
        &device_snapshot,
        capabilities.all(),
    )?;
    let job = enqueued
        .entries
        .into_iter()
        .find_map(|entry| match entry {
            SyncBatchEntryV1::Job(job) => Some(job),
            _ => None,
        })
        .ok_or_else(|| "sync_enqueue_failed".to_string())?;
    state
        .sync_worker
        .get()
        .ok_or_else(|| "sync_worker_not_initialized".to_string())?
        .wake();
    active_sync_result(account_id, job.id, job.status)
}

fn account_can_resume_login(status: &str) -> bool {
    matches!(status, "login_expired" | "identity_conflict")
}

fn register_running_phase(
    slot: &Mutex<Option<RunningSync>>,
    paths: &AppPaths,
    preparing: &SyncOrchestrationV1,
    running_sync: RunningSync,
) -> Result<(), String> {
    let mut running = slot.lock().map_err(|_| "同步状态锁已损坏".to_string())?;
    *running = Some(running_sync);
    let next = preparing.next(SyncLifecycle::Running, &now_iso());
    if let Err(error) = next.and_then(|next| cas_orchestration(paths, preparing, &next)) {
        *running = None;
        return Err(error);
    }
    let resume = running
        .as_ref()
        .ok_or_else(|| "同步阶段进程未注册".to_string())?
        .child
        .as_ref()
        .ok_or_else(|| "同步阶段进程未注册".to_string())?
        .lock()
        .map_err(|_| "同步子进程锁已损坏".to_string())?
        .resume();
    if let Err(error) = resume {
        *running = None;
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn cancel_sync(
    state: State<'_, AppState>,
    account_id: i64,
    job_id: i64,
) -> Result<(), String> {
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let registry = state
        .sync_registry
        .get()
        .ok_or_else(|| "sync_registry_not_initialized".to_string())?;
    let worker = state
        .sync_worker
        .get()
        .ok_or_else(|| "sync_worker_not_initialized".to_string())?;
    let running = worker.running_identity();
    cancel_sync_job(
        &mut connection,
        registry,
        account_id,
        job_id,
        running.as_ref(),
        &now_iso(),
    )?;
    worker.wake();
    Ok(())
}

#[tauri::command]
fn resolve_startup_recovery_fault(
    app: AppHandle,
    state: State<'_, AppState>,
    disposition: StartupRecoveryDispositionV1,
) -> Result<(), String> {
    let paths = state.acquire_paths()?;
    let permit = resolve_sync_runtime_state(state.inner(), &paths, disposition)?;
    drop(paths);
    match continue_desktop_startup_runtime(&app, state.inner(), permit) {
        Ok(()) => {
            state.record_startup_error(None);
            Ok(())
        }
        Err(error) => {
            state.record_startup_error(Some(error.clone()));
            Err(error)
        }
    }
}

fn initialize_sync_runtime(
    app: &AppHandle,
    state: &AppState,
    paths: &AppPaths,
) -> Result<(), String> {
    let registry = state
        .sync_registry
        .get()
        .cloned()
        .ok_or_else(|| "sync_registry_not_initialized".to_string())?;
    let validator = Arc::new(RuntimeSyncWorkerValidator::new(
        paths.data_root.clone(),
        paths.bootstrap_dir.clone(),
        CapabilityRegistry::stage_3_3().all().to_vec(),
    ));
    let terminal_observer: Arc<dyn SyncWorkerTerminalObserver> =
        Arc::new(RuntimeTerminalObserver { app: app.clone() });
    let worker = SyncWorkerHandle::start(
        SyncWorkerConfig::new(
            paths.database.clone(),
            state.operation_admission.clone(),
            registry,
            validator,
        )
        .with_terminal_observer(terminal_observer),
    )?;
    state.sync_worker.set(worker).map_err(|worker| {
        worker.stop();
        let _ = worker.join();
        "sync_worker_already_initialized".to_string()
    })?;
    state
        .sync_worker
        .get()
        .ok_or_else(|| "sync_worker_not_initialized".to_string())?
        .wake();
    Ok(())
}

fn initialize_sync_registry(app: &AppHandle, state: &AppState) -> Result<(), String> {
    if state.sync_registry.get().is_some() {
        return Ok(());
    }
    let start_app = app.clone();
    let cancel_app = app.clone();
    let registry = build_sync_registry(
        Arc::new(move |context| start_queued_sync_execution(&start_app, context)),
        Arc::new(move |running| cancel_queued_sync_execution(&cancel_app, running)),
    )?;
    state
        .sync_registry
        .set(Arc::new(registry))
        .map_err(|_| "sync_registry_already_initialized".to_string())
}

type RuntimeSyncStart =
    dyn Fn(QueuedSyncContext) -> Result<QueueRunningSyncIdentity, String> + Send + Sync;
type RuntimeSyncCancel = dyn Fn(&QueueRunningSyncIdentity) -> Result<(), String> + Send + Sync;

fn build_sync_registry(
    start: Arc<RuntimeSyncStart>,
    cancel: Arc<RuntimeSyncCancel>,
) -> Result<ExecutorRegistry, String> {
    let history_start = Arc::clone(&start);
    let history_cancel = Arc::clone(&cancel);
    let mut registrations = vec![ExecutorRegistration::new(
        Arc::new(HistorySyncExecutor::new(
            move |context| history_start(context),
            move |running| history_cancel(running),
        )),
        Arc::new(HistoryResultImporter),
    )];
    for source in [
        SyncSourceV1::Likes,
        SyncSourceV1::Favorites,
        SyncSourceV1::RemoteFolder,
    ] {
        let start = Arc::clone(&start);
        let cancel = Arc::clone(&cancel);
        registrations.push(ExecutorRegistration::new(
            Arc::new(ArchiveSyncExecutor::new(
                source,
                move |context| start(context),
                move |running| cancel(running),
            )),
            Arc::new(ArchiveResultImporter::new(source)),
        ));
    }
    ExecutorRegistry::new(registrations)
}

#[cfg(test)]
fn test_sync_registry() -> ExecutorRegistry {
    build_sync_registry(
        Arc::new(|_| unreachable!("test importer does not launch")),
        Arc::new(|_| unreachable!("test importer does not cancel")),
    )
    .expect("test sync registry")
}

fn scheduler_account_inputs(
    connection: &Connection,
    portable: &PortableSettingsV2,
    device: &douyin_history_archive::device_settings::DeviceSettingsV1,
) -> Result<Vec<SchedulerAccountInputV1>, String> {
    list_command_accounts(connection, false)?
        .into_iter()
        .map(|account| {
            let account_key = portable_account_key_for_id(connection, account.id)?;
            let account_settings = account_settings_for(portable, Some(&account_key))?;
            let browser_preference = device
                .accounts
                .get(&account.profile_key)
                .cloned()
                .unwrap_or_default()
                .browser_preference;
            Ok(SchedulerAccountInputV1 {
                account_id: account.id,
                account_status: account.status,
                account_settings,
                browser_preference,
            })
        })
        .collect()
}

fn merge_scheduler_wake(
    current: &mut Option<chrono::DateTime<Utc>>,
    candidate: Option<&str>,
) -> Result<(), String> {
    let Some(candidate) = candidate else {
        return Ok(());
    };
    let candidate = chrono::DateTime::parse_from_rfc3339(candidate)
        .map_err(|_| "invalid_scheduler_timestamp".to_string())?
        .with_timezone(&Utc);
    *current = Some(current.map_or(candidate, |value| value.min(candidate)));
    Ok(())
}

fn scheduler_tick_runtime(
    app: &AppHandle,
    state: &AppState,
) -> Result<SchedulerTickResultV1, String> {
    let _operation = state.operation_admission.begin_enqueue_operation()?;
    let paths = state.acquire_paths()?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let portable = load_portable_settings(&paths.data_root)?;
    let device = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?
        .load()?;
    if device.scheduler_paused {
        return Ok(SchedulerTickResultV1 {
            paused: true,
            next_wake_at: None,
            queued_job_ids: Vec::new(),
        });
    }

    let accounts = scheduler_account_inputs(&connection, &portable, &device)?;
    let capabilities = CapabilityRegistry::stage_3_3();
    let now = Utc::now();
    let mut queued_job_ids = Vec::new();
    let mut next_wake = None;
    let mut changed = false;
    for account in accounts
        .iter()
        .filter(|account| account.account_status == "active")
    {
        let browser = match current_browser_execution_identity(
            &paths,
            &connection,
            account.account_id,
            account.browser_preference,
        ) {
            Ok(Some(browser)) => browser,
            Ok(None) => continue,
            Err(_) => {
                eprintln!("scheduler_account_identity_invalid");
                continue;
            }
        };
        let result = reconcile_automatic_jobs(
            &mut connection,
            &AutomaticReconcileInputV1 {
                account_id: account.account_id,
                settings_revision: portable.revision,
                account_settings: account.account_settings.clone(),
                browser_preference: account.browser_preference,
                browser,
                capabilities: capabilities.all().to_vec(),
                app_ready_at: state.app_ready_at(),
                startup_delay_minutes: portable.global.startup_sync_delay_minutes,
                now,
            },
        )?;
        merge_scheduler_wake(&mut next_wake, result.next_wake_at.as_deref())?;
        changed |= !result.created_job_ids.is_empty() || !result.cancelled_job_ids.is_empty();
        queued_job_ids.extend(result.created_job_ids);
    }

    if !queued_job_ids.is_empty() {
        state
            .sync_worker
            .get()
            .ok_or_else(|| "sync_worker_not_initialized".to_string())?
            .wake();
    }
    if changed {
        let _ = app.emit("sync:changed", ());
        let _ = app.emit("scheduler:changed", ());
    }
    Ok(SchedulerTickResultV1 {
        paused: false,
        next_wake_at: next_wake.map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true)),
        queued_job_ids,
    })
}

fn automatic_backup_tick_runtime_with(
    state: &AppState,
    now: chrono::DateTime<Utc>,
    wait_timeout: Duration,
    notify: impl FnMut(NotificationKindV1) -> Result<(), String>,
) -> Result<SchedulerTickResultV1, String> {
    let runtime_paths = state.runtime_paths()?;
    let portable = {
        let paths = runtime_paths.acquire()?;
        load_portable_settings(&paths.data_root)?
    };
    let device_settings = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?;
    let result = run_automatic_backup_tick(
        &runtime_paths,
        &state.operation_admission,
        &state.background_tasks,
        device_settings,
        &portable.global.backup,
        &portable.global.notifications,
        now,
        env!("CARGO_PKG_VERSION"),
        wait_timeout,
        notify,
    )?;
    if let Some(error) = result.notification_error.as_deref() {
        eprintln!("automatic_backup_notification_failed:{error}");
    }
    if result.status == AutomaticBackupTickStatusV1::Failed {
        eprintln!(
            "automatic_backup_failed:{}",
            result
                .error_code
                .as_deref()
                .unwrap_or("automatic_backup_failed")
        );
    }
    Ok(SchedulerTickResultV1 {
        paused: false,
        next_wake_at: result
            .next_wake_at
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true)),
        queued_job_ids: Vec::new(),
    })
}

fn initialize_scheduler_runtime(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let app_for_tick = app.clone();
    let worker = SchedulerWorkerHandle::start(SchedulerWorkerConfigV1::new(
        Duration::from_secs(30),
        Arc::new(move || {
            let state = app_for_tick.state::<AppState>();
            scheduler_tick_runtime(&app_for_tick, state.inner())
        }),
    ))?;
    state.scheduler_worker.set(worker).map_err(|worker| {
        let _ = worker.stop();
        let _ = worker.join();
        "scheduler_worker_already_initialized".to_string()
    })?;
    Ok(())
}

fn initialize_automatic_backup_runtime_with(
    state: &AppState,
    tick: Arc<dyn Fn() -> Result<SchedulerTickResultV1, String> + Send + Sync>,
) -> Result<(), String> {
    let worker =
        SchedulerWorkerHandle::start(SchedulerWorkerConfigV1::new(Duration::from_secs(30), tick))?;
    state
        .automatic_backup_worker
        .set(worker)
        .map_err(|worker| {
            let _ = worker.stop();
            let _ = worker.join();
            "automatic_backup_worker_already_initialized".to_string()
        })?;
    Ok(())
}

fn initialize_automatic_backup_runtime(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let app_for_tick = app.clone();
    initialize_automatic_backup_runtime_with(
        state,
        Arc::new(move || {
            let state = app_for_tick.state::<AppState>();
            let result = automatic_backup_tick_runtime_with(
                state.inner(),
                Utc::now(),
                Duration::ZERO,
                |kind| deliver_runtime_notification(&app_for_tick, state.inner(), kind),
            )?;
            let backup_state = state
                .device_settings
                .get()
                .ok_or_else(|| "device_settings_not_initialized".to_string())?
                .load()?
                .automatic_backup;
            if backup_state.last_failure_at.is_none()
                && backup_state.last_success_at.is_some()
                && backup_state.last_success_at == backup_state.last_attempt_at
            {
                resolve_automatic_backup_failure_notification(&app_for_tick, state.inner())?;
            }
            Ok(result)
        }),
    )
}

fn stop_automatic_backup_runtime(state: &AppState) -> Result<(), String> {
    if let Some(worker) = state.automatic_backup_worker.get() {
        worker.stop()?;
    }
    Ok(())
}

fn join_automatic_backup_runtime(state: &AppState) -> Result<(), String> {
    if let Some(worker) = state.automatic_backup_worker.get() {
        worker.join()?;
    }
    Ok(())
}

fn rollback_automatic_backup_runtime(state: &AppState) {
    let _ = stop_automatic_backup_runtime(state);
    let _ = join_automatic_backup_runtime(state);
}

fn download_runtime_error(error: DownloadError) -> String {
    format!("{}:{}", error.code, error.message)
}

fn initialize_download_runtime(
    app: &AppHandle,
    state: &AppState,
    paths: &AppPaths,
) -> Result<(), String> {
    DownloadService::recover_offline(&paths.database).map_err(download_runtime_error)?;
    let runtime = DownloadRuntimeConfig::production(
        std::env::current_exe().map_err(|error| error.to_string())?,
        paths.browser_profiles.clone(),
        paths.temp.clone(),
    )
    .map_err(download_runtime_error)?;
    let service = Arc::new(
        tauri::async_runtime::block_on(DownloadService::spawn_with_runtime(
            &paths.database,
            DownloadBrowserOperationCoordinator::from_admission(
                state.operation_admission.clone(),
            ),
            runtime,
        ))
        .map_err(download_runtime_error)?,
    );
    let mut events = service.subscribe();
    let shutdown = service.shutdown_token();
    let app_for_events = app.clone();
    tauri::async_runtime::block_on(service.spawn_worker(async move {
        loop {
            let event = tokio::select! {
                _ = shutdown.cancelled() => break,
                event = events.recv() => event,
            };
            match event {
                Ok(event) => {
                    let _ = app_for_events.emit("download_job_updated", &event);
                    if event.item_id.is_some() {
                        let _ = app_for_events.emit("download_item_updated", &event);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));
    if let Err(service) = state.download_service.set(service) {
        let _ = tauri::async_runtime::block_on(service.request_shutdown());
        let _ = tauri::async_runtime::block_on(service.wait_for_shutdown(Duration::from_secs(5)));
        return Err("download_service_already_initialized".to_string());
    }
    Ok(())
}

fn stop_download_runtime(state: &AppState) -> Result<(), String> {
    if let Some(service) = state.download_service.get() {
        tauri::async_runtime::block_on(service.request_shutdown())
            .map_err(download_runtime_error)?;
    }
    Ok(())
}

fn wait_download_runtime(state: &AppState, timeout: Duration) -> Result<bool, String> {
    state.download_service.get().map_or(Ok(true), |service| {
        tauri::async_runtime::block_on(service.wait_for_shutdown(timeout))
            .map_err(download_runtime_error)
    })
}

fn initialize_cover_cache_runtime(state: &AppState) -> Result<(), String> {
    let runtime_paths = state.runtime_paths()?;
    {
        let paths = runtime_paths.acquire()?;
        let mut connection = open_database(&paths.database)?;
        assert_schema_latest(&connection)?;
        recover_interrupted_cache_tasks(&connection, &now_iso())?;
        let portable = load_portable_settings(&paths.data_root)?;
        if portable.global.cover_cache.enabled {
            if portable.global.cover_cache.backfill_mode == CoverCacheBackfillModeV1::BackfillAll {
                enqueue_all_missing_cover_candidates(&mut connection, &now_iso())?;
            }
            ensure_pending_cache_task(
                &mut connection,
                if portable.global.cover_cache.backfill_mode
                    == CoverCacheBackfillModeV1::BackfillAll
                {
                    CacheTaskKindV1::Backfill
                } else {
                    CacheTaskKindV1::NewRecords
                },
                &now_iso(),
            )?;
        } else {
            disable_cover_cache_queue(&mut connection, &now_iso())?;
        }
    }
    let worker = CoverCacheWorkerHandle::start(CoverCacheWorkerConfigV1::new(runtime_paths))?;
    state.cover_cache_worker.set(worker).map_err(|worker| {
        worker.stop();
        let _ = worker.join();
        "cover_cache_worker_already_initialized".to_string()
    })?;
    if let Some(worker) = state.cover_cache_worker.get() {
        worker.wake();
    }
    Ok(())
}

fn rollback_cover_cache_runtime(state: &AppState) {
    if let Some(worker) = state.cover_cache_worker.get() {
        worker.stop();
        let _ = worker.join();
    }
}

fn rollback_scheduler_runtime(state: &AppState) {
    if let Some(worker) = state.scheduler_worker.get() {
        let _ = worker.stop();
        let _ = worker.join();
    }
}

fn rollback_sync_runtime(state: &AppState) {
    if let Some(worker) = state.sync_worker.get() {
        worker.stop();
        let _ = worker.join();
    }
}

fn scheduler_state_runtime(state: &AppState) -> Result<SchedulerStateV1, String> {
    let paths = state.acquire_paths()?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let portable = load_portable_settings(&paths.data_root)?;
    let device = state
        .device_settings
        .get()
        .ok_or_else(|| "device_settings_not_initialized".to_string())?
        .load()?;
    let accounts = scheduler_account_inputs(&connection, &portable, &device)?;
    let runtime = state
        .scheduler_worker
        .get()
        .ok_or_else(|| "scheduler_worker_not_initialized".to_string())?
        .snapshot()?;
    build_scheduler_state(
        &connection,
        device.scheduler_paused,
        &runtime,
        CapabilityRegistry::stage_3_3().all(),
        &accounts,
        state.app_ready_at(),
        portable.global.startup_sync_delay_minutes,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhaseLaunchBarrier {
    BeforePreparing,
    AfterPreparing,
    AfterSpawn,
    BeforeResume,
}

impl PhaseLaunchBarrier {
    #[cfg(test)]
    fn as_str(self) -> &'static str {
        match self {
            Self::BeforePreparing => "before-preparing",
            Self::AfterPreparing => "after-preparing",
            Self::AfterSpawn => "after-spawn",
            Self::BeforeResume => "before-resume",
        }
    }
}

fn launch_next_phase_with<F>(
    slot: &Mutex<Option<RunningSync>>,
    paths: &AppPaths,
    current: &SyncOrchestrationV1,
    preparing: &SyncOrchestrationV1,
    invocation: CollectorInvocation,
    mut barrier: F,
) -> Result<(RunningSyncIdentity, Arc<Mutex<OwnedCollectorProcess>>), String>
where
    F: FnMut(PhaseLaunchBarrier, &SyncOrchestrationV1) -> Result<(), String>,
{
    let current_identity = RunningSyncIdentity {
        account_id: current.account_id,
        job_id: current.sync_job_id,
        phase: current.phase,
        phase_attempt_id: current.phase_attempt_id.clone(),
    };
    let next_identity = RunningSyncIdentity {
        account_id: preparing.account_id,
        job_id: preparing.sync_job_id,
        phase: preparing.phase,
        phase_attempt_id: preparing.phase_attempt_id.clone(),
    };
    let mut running = slot.lock().map_err(|_| "同步状态锁已损坏".to_string())?;
    let task = running
        .as_mut()
        .filter(|task| task.identity() == current_identity)
        .ok_or_else(|| "sync_orchestration_mismatch".to_string())?;

    barrier(PhaseLaunchBarrier::BeforePreparing, current)?;
    ensure_phase_launch_allowed(paths, current)?;
    cas_orchestration(paths, current, preparing)?;
    task.phase = preparing.phase;
    task.phase_attempt_id = preparing.phase_attempt_id.clone();
    task.child = None;

    barrier(PhaseLaunchBarrier::AfterPreparing, preparing)?;
    ensure_phase_launch_allowed(paths, preparing)?;
    let child = spawn_collector(paths, &next_identity, invocation)?;

    barrier(PhaseLaunchBarrier::AfterSpawn, preparing)?;
    if let Err(error) = ensure_phase_launch_allowed(paths, preparing) {
        let _ = child.kill();
        return Err(error);
    }
    let child = Arc::new(Mutex::new(child));
    task.child = Some(Arc::clone(&child));
    let running_state = preparing.next(SyncLifecycle::Running, &now_iso())?;
    if let Err(error) = cas_orchestration(paths, preparing, &running_state) {
        if let Ok(child) = child.lock() {
            let _ = child.kill();
        }
        task.child = None;
        return Err(error);
    }

    barrier(PhaseLaunchBarrier::BeforeResume, &running_state)?;
    if let Err(error) = ensure_phase_launch_allowed(paths, &running_state) {
        if let Ok(child) = child.lock() {
            let _ = child.kill();
        }
        task.child = None;
        return Err(error);
    }
    if let Err(error) = child
        .lock()
        .map_err(|_| "同步子进程锁已损坏".to_string())?
        .resume()
    {
        task.child = None;
        return Err(error);
    }
    Ok((next_identity, child))
}

fn ensure_phase_launch_allowed(
    paths: &AppPaths,
    expected: &SyncOrchestrationV1,
) -> Result<(), String> {
    let current = read_orchestration(paths, expected.account_id, expected.sync_job_id)?;
    if current.cancel_requested_at.is_some()
        || paths
            .cancel_path(expected.account_id, expected.sync_job_id)
            .exists()
    {
        return Err("sync_cancel_requested".to_string());
    }
    if current != *expected {
        return Err("sync_orchestration_conflict".to_string());
    }
    let connection = open_database(&paths.database)?;
    let status = connection
        .query_row(
            "SELECT account_id, status FROM sync_jobs WHERE id = ?1",
            [expected.sync_job_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "sync_job_mismatch".to_string())?;
    if status.0 != expected.account_id || status.1 != "running" {
        return Err("sync_job_mismatch".to_string());
    }
    Ok(())
}

struct ObservedPhaseCompletion {
    decision: PhaseCompletionDecision,
    diagnostic: Option<CollectorDiagnostic>,
    recovery_identity: Option<ObservedHomeIdentityV1>,
}

fn observe_phase_completion(
    paths: &AppPaths,
    identity: &RunningSyncIdentity,
    process_succeeded: bool,
) -> Result<ObservedPhaseCompletion, String> {
    let orchestration = read_orchestration(paths, identity.account_id, identity.job_id)?;
    validate_running_orchestration(&orchestration, identity)?;
    let result_path = paths.phase_result_path(
        identity.account_id,
        identity.job_id,
        identity.phase,
        &identity.phase_attempt_id,
    )?;
    let (result_ready, recovery_identity) = match identity.phase {
        SyncPhase::Collecting | SyncPhase::RetryingAfterLogin => (
            read_sync_execution_envelope(paths, &result_path, &orchestration).is_ok(),
            None,
        ),
        SyncPhase::RecoveringLogin => {
            match read_sync_phase_payload::<ObservedHomeIdentityV1>(&result_path, &orchestration) {
                Ok(identity) => (true, Some(identity)),
                Err(_) => (false, None),
            }
        }
    };
    let diagnostic = paths
        .phase_diagnostic_path(
            identity.account_id,
            identity.job_id,
            identity.phase,
            &identity.phase_attempt_id,
        )
        .ok()
        .and_then(|path| read_sync_phase_payload::<CollectorDiagnostic>(&path, &orchestration).ok())
        .filter(|value| {
            value.schema_version == 3
                && value.account_id == identity.account_id
                && value.sync_job_id == identity.job_id
        });
    let evidence = PhaseCompletionEvidence {
        process_succeeded,
        result_ready,
        diagnostic_status: diagnostic.as_ref().map(|value| value.status.clone()),
        diagnostic_error_code: diagnostic.as_ref().map(|value| value.error_code.clone()),
        cancel_file_exists: paths
            .cancel_path(identity.account_id, identity.job_id)
            .exists(),
    };
    Ok(ObservedPhaseCompletion {
        decision: phase_completion_decision(&orchestration, &evidence),
        diagnostic,
        recovery_identity,
    })
}

fn complete_hidden_recovery(
    paths: &AppPaths,
    profile_key: &str,
    orchestration: &SyncOrchestrationV1,
    identity: ObservedHomeIdentityV1,
) -> Result<(), String> {
    if orchestration.phase != SyncPhase::RecoveringLogin
        || orchestration.lifecycle != SyncLifecycle::Running
        || orchestration.login_recovery_count != 1
        || orchestration
            .deadline_at_ms
            .is_none_or(|deadline| Utc::now().timestamp_millis() >= deadline)
    {
        return Err("login_recovery_failed".to_string());
    }
    if !is_valid_douyin_home_url(&identity.page_url) {
        return Err("account_identity_conflict".to_string());
    }
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    douyin_history_archive::accounts::validate_hidden_recovery_identity(
        &connection,
        orchestration.account_id,
        AccountIdentity {
            uid: identity.uid,
            sec_uid: identity.sec_uid,
            nickname: identity.nickname,
            avatar_url: identity.avatar_url,
        },
    )?;

    let browser_path = paths.phase_browser_pid_path(
        orchestration.account_id,
        orchestration.sync_job_id,
        orchestration.phase,
        &orchestration.phase_attempt_id,
    )?;
    let record: BrowserProcessRecordV2 = read_sync_phase_payload(&browser_path, orchestration)?;
    let account_root = paths.account_browser_root(profile_key)?;
    let expected_profile = douyin_history_archive::collector::browser_state::browser_profile_path(
        &account_root,
        record.browser_id,
    );
    if !browser_record_matches_expected_profile(&record, &expected_profile) {
        return Err("browser_profile_scope_mismatch".to_string());
    }
    if query_process_creation_time_100ns(record.pid).as_deref()
        == Some(record.creation_time_100ns.as_str())
    {
        return Err("browser_still_running".to_string());
    }
    let choice = BrowserChoice {
        browser_id: record.browser_id,
        executable_path: PathBuf::from(record.executable_path),
        profile_path: expected_profile,
    };
    douyin_history_archive::collector::browser_runtime::persist_validation_with_id(
        &account_root,
        &choice,
        DOUYIN_HISTORY_URL,
        Some(&orchestration.phase_attempt_id),
    )
}

fn terminal_phase_error(
    phase: SyncPhase,
    diagnostic: Option<&CollectorDiagnostic>,
) -> (String, String) {
    let Some(diagnostic) = diagnostic else {
        return (
            "collector_interrupted".to_string(),
            "同步进程异常退出".to_string(),
        );
    };
    match (phase, diagnostic.error_code.as_str()) {
        (SyncPhase::RecoveringLogin, "collector_deadline_exceeded") => (
            "login_recovery_failed".to_string(),
            "自动恢复登录失败，请重新登录。".to_string(),
        ),
        (SyncPhase::RecoveringLogin, "account_identity_ambiguous") => (
            "account_identity_conflict".to_string(),
            "自动恢复检测到账号身份冲突。".to_string(),
        ),
        _ => (diagnostic.error_code.clone(), diagnostic.message.clone()),
    }
}

#[cfg(test)]
fn finalize_sync(
    paths: &AppPaths,
    identity: &RunningSyncIdentity,
    process_success: bool,
) -> Result<SyncResult, String> {
    let registry = test_sync_registry();
    let gate = linearize_sync_finalization(paths, identity);
    finalize_sync_with_gate(paths, &registry, identity, process_success, gate)
}

enum SyncFinalizationGate {
    Cancelled(SyncResult),
    Consuming(SyncOrchestrationV1),
}

fn linearize_sync_finalization(
    paths: &AppPaths,
    identity: &RunningSyncIdentity,
) -> Result<SyncFinalizationGate, String> {
    let orchestration = read_orchestration(paths, identity.account_id, identity.job_id)?;
    validate_running_orchestration(&orchestration, identity)?;
    if orchestration.cancel_requested_at.is_some()
        || paths
            .cancel_path(identity.account_id, identity.job_id)
            .exists()
    {
        persist_sync_terminal_outcome(
            paths,
            identity.account_id,
            identity.job_id,
            SyncTerminalOutcome::Cancelled {
                error_message: Some("同步已取消".to_string()),
            },
        )?;
        return Ok(SyncFinalizationGate::Cancelled(SyncResult {
            account_id: identity.account_id,
            job_id: identity.job_id,
            status: "cancelled".to_string(),
            inserted_videos: 0,
            inserted_watch_records: 0,
            message: Some("同步已取消".to_string()),
        }));
    }
    let consuming = orchestration.next(SyncLifecycle::Consuming, &now_iso())?;
    cas_orchestration(paths, &orchestration, &consuming)?;
    Ok(SyncFinalizationGate::Consuming(consuming))
}

fn finalize_sync_with_gate(
    paths: &AppPaths,
    registry: &ExecutorRegistry,
    identity: &RunningSyncIdentity,
    process_success: bool,
    gate: Result<SyncFinalizationGate, String>,
) -> Result<SyncResult, String> {
    let result = gate.and_then(|gate| {
        complete_sync_finalization(paths, registry, identity, process_success, gate)
    });
    if let Err(error) = &result {
        let _ = persist_sync_terminal_outcome(
            paths,
            identity.account_id,
            identity.job_id,
            SyncTerminalOutcome::Failed {
                error_code: "collector_interrupted".to_string(),
                error_message: Some(error.clone()),
            },
        );
    }
    result
}

fn complete_sync_finalization(
    paths: &AppPaths,
    registry: &ExecutorRegistry,
    identity: &RunningSyncIdentity,
    process_success: bool,
    gate: SyncFinalizationGate,
) -> Result<SyncResult, String> {
    let consuming = match gate {
        SyncFinalizationGate::Cancelled(result) => return Ok(result),
        SyncFinalizationGate::Consuming(orchestration) => orchestration,
    };
    if process_success {
        let result_path = paths.phase_result_path(
            identity.account_id,
            identity.job_id,
            identity.phase,
            &identity.phase_attempt_id,
        )?;
        let envelope = read_sync_execution_envelope(paths, &result_path, &consuming)?;
        let mut connection = open_database(&paths.database)?;
        assert_schema_v5(&connection)?;
        match import_execution_envelope(registry, &mut connection, identity.job_id, envelope) {
            Ok(outcome) => Ok(SyncResult {
                account_id: identity.account_id,
                job_id: identity.job_id,
                status: match outcome.status {
                    douyin_history_archive::sync::SyncJobStatusV1::Success => "success",
                    douyin_history_archive::sync::SyncJobStatusV1::Partial => "partial",
                    _ => return Err("invalid_sync_import_status".to_string()),
                }
                .to_string(),
                inserted_videos: outcome.counts.inserted_videos,
                inserted_watch_records: outcome.counts.inserted_watch_records,
                message: None,
            }),
            Err(error) => Ok(failed_result(identity.account_id, identity.job_id, &error)),
        }
    } else {
        let diagnostic_path = paths.phase_diagnostic_path(
            identity.account_id,
            identity.job_id,
            identity.phase,
            &identity.phase_attempt_id,
        )?;
        let diagnostic = read_sync_phase_payload::<DiagnosticFile>(&diagnostic_path, &consuming)
            .ok()
            .filter(|value| {
                value.schema_version == 3
                    && value.account_id == identity.account_id
                    && value.sync_job_id == identity.job_id
            });
        let status = failed_sync_status(
            diagnostic.as_ref().map(|value| value.status.as_str()),
            paths
                .cancel_path(identity.account_id, identity.job_id)
                .exists(),
        );
        let message = diagnostic
            .as_ref()
            .map(|value| value.message.clone())
            .unwrap_or_else(|| {
                if status == "cancelled" {
                    "同步已取消".to_string()
                } else {
                    paths
                        .phase_log_path(
                            identity.account_id,
                            identity.job_id,
                            identity.phase,
                            &identity.phase_attempt_id,
                        )
                        .map(|path| read_log_tail(&path))
                        .unwrap_or_else(|_| "同步进程异常退出".to_string())
                }
            });
        if status == "cancelled" {
            persist_sync_terminal_outcome(
                paths,
                identity.account_id,
                identity.job_id,
                SyncTerminalOutcome::Cancelled {
                    error_message: Some(message.clone()),
                },
            )?;
        } else {
            let (error_code, message) = terminal_phase_error(identity.phase, diagnostic.as_ref());
            persist_sync_terminal_outcome(
                paths,
                identity.account_id,
                identity.job_id,
                SyncTerminalOutcome::Failed {
                    error_code,
                    error_message: Some(message.clone()),
                },
            )?;
            return Ok(SyncResult {
                account_id: identity.account_id,
                job_id: identity.job_id,
                status: status.to_string(),
                inserted_videos: 0,
                inserted_watch_records: 0,
                message: Some(message),
            });
        }
        Ok(SyncResult {
            account_id: identity.account_id,
            job_id: identity.job_id,
            status: status.to_string(),
            inserted_videos: 0,
            inserted_watch_records: 0,
            message: Some(message),
        })
    }
}

fn failed_sync_status(diagnostic_status: Option<&str>, cancel_file_exists: bool) -> &'static str {
    if diagnostic_status == Some("cancelled") || cancel_file_exists {
        "cancelled"
    } else {
        "failed"
    }
}

fn request_sync_cancellation(
    paths: &AppPaths,
    identity: &RunningSyncIdentity,
    requested_at: &str,
) -> Result<(), String> {
    let orchestration = read_orchestration(paths, identity.account_id, identity.job_id)?;
    validate_running_orchestration(&orchestration, identity)?;
    request_orchestration_cancel(paths, &orchestration, requested_at)?;
    let _ = fs::write(
        paths.cancel_path(identity.account_id, identity.job_id),
        b"cancel\n",
    );
    Ok(())
}

fn resolve_initial_paths(app: &AppHandle) -> Result<AppPaths, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?;
    resolve_initial_paths_from(app_data, resource_dir)
}

fn resolve_initial_paths_from(
    app_data: PathBuf,
    resource_dir: PathBuf,
) -> Result<AppPaths, String> {
    let _bootstrap_guard = BootstrapProcessGuard::acquire(&app_data)?;
    let bootstrap =
        resolve_or_initialize_data_root(&app_data).map_err(|error| error.code().to_string())?;
    Ok(AppPaths::from_roots(
        app_data,
        bootstrap.current_root.path,
        resource_dir,
    ))
}

fn prepare_app_directories(paths: &AppPaths) -> Result<(), String> {
    for directory in [
        &paths.app_data,
        &paths.browser_profiles,
        &paths.covers,
        &paths.temp,
        &paths.diagnostics,
        &paths.logs,
    ] {
        fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn recover_login_runtime_state(paths: &AppPaths) -> Result<(), String> {
    let current_executable = std::env::current_exe().map_err(|error| error.to_string())?;
    recover_durable_login_attempts_with(
        paths,
        &current_executable,
        terminate_recorded_login_collector,
        terminate_recorded_browser_v2,
    )?;
    let mut connection = open_database(&paths.database)?;
    recover_login_attempts_with(paths, &mut connection, terminate_recorded_browser_v2)?;
    Ok(())
}

fn uninstall_runtime_paths(
    session: &douyin_history_archive::uninstall::UninstallSessionV1,
) -> Result<AppPaths, String> {
    let bootstrap = read_bootstrap(&session.device_settings_dir)
        .map_err(|_| "uninstall_runtime_bootstrap_invalid".to_string())?
        .ok_or_else(|| "uninstall_runtime_bootstrap_missing".to_string())?;
    if bootstrap.current_root.root_id != session.root_id {
        return Err("uninstall_runtime_root_mismatch".to_string());
    }
    validate_data_root_marker(&bootstrap.current_root.path, &bootstrap.current_root)
        .map_err(|_| "uninstall_runtime_root_mismatch".to_string())?;
    Ok(AppPaths::from_roots(
        session.device_settings_dir.clone(),
        bootstrap.current_root.path,
        PathBuf::new(),
    ))
}

fn prepare_application_runtime_for_uninstall(session: &UninstallSessionV1) -> Result<(), String> {
    prepare_application_runtime_for_uninstall_with(
        session,
        preflight_recorded_login_collector_strict,
        preflight_recorded_sync_collector_strict,
        preflight_recorded_browser_v2_strict,
        terminate_preflighted_login_collector,
        terminate_preflighted_sync_collector,
        terminate_preflighted_browser_v2,
    )
}

#[derive(Debug, Default)]
struct ApplicationInstallerControlRuntime {
    native: NativeInstallerControlRuntime,
}

impl InstallerControlRuntime for ApplicationInstallerControlRuntime {
    fn start_guard(
        &self,
        coordination_dir: &Path,
        authentication: &douyin_history_archive::uninstall::GuardAuthenticationV1,
        expected_guard_product: &ProductIdentityV1,
        prepare_request: Option<&douyin_history_archive::uninstall::GuardPrepareRequestV1>,
    ) -> Result<InstallerGuardProcessV1, String> {
        self.native.start_guard(
            coordination_dir,
            authentication,
            expected_guard_product,
            prepare_request,
        )
    }

    fn assert_guard_alive(
        &self,
        authentication: &douyin_history_archive::uninstall::GuardAuthenticationV1,
    ) -> Result<(), String> {
        self.native.assert_guard_alive(authentication)
    }

    fn prepare_for_destructive_commit(&self, session: &UninstallSessionV1) -> Result<(), String> {
        prepare_application_runtime_for_uninstall(session)
    }

    fn signal_guard_finished(
        &self,
        coordination_dir: &Path,
        authentication: &douyin_history_archive::uninstall::GuardAuthenticationV1,
    ) -> Result<(), String> {
        self.native
            .signal_guard_finished(coordination_dir, authentication)
    }

    fn signal_guard_aborted(
        &self,
        coordination_dir: &Path,
        authentication: &douyin_history_archive::uninstall::GuardAuthenticationV1,
    ) -> Result<(), String> {
        self.native
            .signal_guard_aborted(coordination_dir, authentication)
    }

    fn delete_managed_data(&self, bootstrap_dir: &Path) -> Result<(), String> {
        self.native.delete_managed_data(bootstrap_dir)
    }
}

#[derive(Debug, Clone)]
struct TrustedSyncProcessCleanupV1 {
    collector: CollectorProcessIdentityV1,
    collector_expectation: CollectorProcessExpectation,
    browser: Option<(BrowserProcessRecordV2, PathBuf)>,
}

#[derive(Debug, Clone)]
struct TrustedLoginProcessCleanupV1 {
    collector: Option<(
        LoginCollectorProcessIdentityV1,
        LoginCollectorProcessExpectation,
    )>,
    browser: Option<(BrowserProcessRecordV2, PathBuf)>,
}

#[derive(Debug, Clone)]
enum LoginCleanupFilesV1 {
    Durable { attempt_id: String },
    Legacy { account_id: i64, attempt_id: String },
}

#[derive(Debug, Clone)]
struct TrustedLoginCleanupV1 {
    process: TrustedLoginProcessCleanupV1,
    files: LoginCleanupFilesV1,
}

fn prepare_application_runtime_for_uninstall_with<LT, CT, BT, LP, CP, BP, L, C, B>(
    session: &UninstallSessionV1,
    preflight_login_collector: LP,
    preflight_sync_collector: CP,
    preflight_browser: BP,
    mut terminate_login_collector: L,
    mut terminate_sync_collector: C,
    mut terminate_browser: B,
) -> Result<(), String>
where
    LP: FnMut(
        &LoginCollectorProcessIdentityV1,
        &LoginCollectorProcessExpectation,
    ) -> Result<Option<LT>, String>,
    CP: FnMut(
        &CollectorProcessIdentityV1,
        &CollectorProcessExpectation,
    ) -> Result<Option<CT>, String>,
    BP: FnMut(&BrowserProcessRecordV2, &Path) -> Result<Option<BT>, String>,
    L: FnMut(
        &LoginCollectorProcessIdentityV1,
        &LoginCollectorProcessExpectation,
        LT,
    ) -> Result<(), String>,
    C: FnMut(&CollectorProcessIdentityV1, &CollectorProcessExpectation, CT) -> Result<(), String>,
    B: FnMut(&BrowserProcessRecordV2, &Path, BT) -> Result<(), String>,
{
    let paths = uninstall_runtime_paths(session)?;
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let running_jobs = QueueStore::list_running_jobs(&connection)?;
    let queued_jobs = QueueStore::list_queued_jobs(&connection)?;
    let active_jobs = running_jobs
        .iter()
        .chain(queued_jobs.iter())
        .map(|job| (job.snapshot.task_key.account_id, job.snapshot.id))
        .collect::<Vec<_>>();
    let mut cleanup_jobs =
        QueueStore::list_uninstall_recovery_job_keys(&connection, "uninstall_runtime_stopped")?;
    cleanup_jobs.extend(active_jobs.iter().copied());
    cleanup_jobs.sort_unstable();
    cleanup_jobs.dedup();
    let login_cleanups =
        trusted_login_cleanups_for_uninstall(&paths, &session.main_executable, &connection)?;
    let sync_cleanups = running_jobs
        .iter()
        .map(|job| trusted_sync_cleanup_for_uninstall(&paths, session, job))
        .collect::<Result<Vec<_>, String>>()?;

    stop_trusted_processes_for_uninstall_with(
        &login_cleanups,
        &sync_cleanups,
        preflight_login_collector,
        preflight_sync_collector,
        preflight_browser,
        &mut terminate_login_collector,
        &mut terminate_sync_collector,
        &mut terminate_browser,
    )?;
    for cleanup in &login_cleanups {
        remove_login_cleanup_files(&paths, &cleanup.files)?;
    }

    QueueStore::recover_for_uninstall(&mut connection, &now_iso(), "uninstall_runtime_stopped")?;
    QueueStore::assert_no_active_jobs(&connection)?;
    drop(connection);
    for (account_id, job_id) in cleanup_jobs {
        cleanup_recovered_job_files_strict(&paths, account_id, job_id)?;
    }
    Ok(())
}

fn stop_trusted_processes_for_uninstall_with<LT, CT, BT, LP, CP, BP, L, C, B>(
    login_cleanups: &[TrustedLoginCleanupV1],
    sync_cleanups: &[TrustedSyncProcessCleanupV1],
    mut preflight_login_collector: LP,
    mut preflight_sync_collector: CP,
    mut preflight_browser: BP,
    mut terminate_login_collector: L,
    mut terminate_sync_collector: C,
    mut terminate_browser: B,
) -> Result<(), String>
where
    LP: FnMut(
        &LoginCollectorProcessIdentityV1,
        &LoginCollectorProcessExpectation,
    ) -> Result<Option<LT>, String>,
    CP: FnMut(
        &CollectorProcessIdentityV1,
        &CollectorProcessExpectation,
    ) -> Result<Option<CT>, String>,
    BP: FnMut(&BrowserProcessRecordV2, &Path) -> Result<Option<BT>, String>,
    L: FnMut(
        &LoginCollectorProcessIdentityV1,
        &LoginCollectorProcessExpectation,
        LT,
    ) -> Result<(), String>,
    C: FnMut(&CollectorProcessIdentityV1, &CollectorProcessExpectation, CT) -> Result<(), String>,
    B: FnMut(&BrowserProcessRecordV2, &Path, BT) -> Result<(), String>,
{
    let mut login_tokens = Vec::with_capacity(login_cleanups.len());
    for cleanup in login_cleanups {
        let collector_token = cleanup
            .process
            .collector
            .as_ref()
            .map(|(record, expectation)| preflight_login_collector(record, expectation))
            .transpose()?
            .flatten();
        let browser_token = cleanup
            .process
            .browser
            .as_ref()
            .map(|(record, expected_profile)| preflight_browser(record, expected_profile))
            .transpose()?
            .flatten();
        login_tokens.push((collector_token, browser_token));
    }
    let mut sync_tokens = Vec::with_capacity(sync_cleanups.len());
    for cleanup in sync_cleanups {
        let collector_token =
            preflight_sync_collector(&cleanup.collector, &cleanup.collector_expectation)?;
        let browser_token = cleanup
            .browser
            .as_ref()
            .map(|(record, expected_profile)| preflight_browser(record, expected_profile))
            .transpose()?
            .flatten();
        sync_tokens.push((collector_token, browser_token));
    }

    for (cleanup, (collector_token, browser_token)) in login_cleanups.iter().zip(login_tokens) {
        if let (Some((record, expectation)), Some(token)) =
            (&cleanup.process.collector, collector_token)
        {
            terminate_login_collector(record, expectation, token)?;
        }
        if let (Some((record, expected_profile)), Some(token)) =
            (&cleanup.process.browser, browser_token)
        {
            terminate_browser(record, expected_profile, token)?;
        }
    }
    for (cleanup, (collector_token, browser_token)) in sync_cleanups.iter().zip(sync_tokens) {
        if let Some(token) = collector_token {
            terminate_sync_collector(&cleanup.collector, &cleanup.collector_expectation, token)?;
        }
        if let (Some((record, expected_profile)), Some(token)) = (&cleanup.browser, browser_token) {
            terminate_browser(record, expected_profile, token)?;
        }
    }
    Ok(())
}

fn trusted_sync_cleanup_for_uninstall(
    paths: &AppPaths,
    session: &UninstallSessionV1,
    job: &StoredSyncJobV1,
) -> Result<TrustedSyncProcessCleanupV1, String> {
    let orchestration =
        read_orchestration(paths, job.snapshot.task_key.account_id, job.snapshot.id)
            .map_err(|_| "uninstall_runtime_orchestration_invalid".to_string())?;
    if orchestration.options_hash != job.snapshot.task_payload_sha256 {
        return Err("uninstall_runtime_orchestration_mismatch".to_string());
    }
    let browser_identity = frozen_browser_identity(job)?;
    let invocation = match orchestration.phase {
        SyncPhase::Collecting | SyncPhase::RetryingAfterLogin => build_collect_invocation_for_job(
            paths,
            session.main_executable.clone(),
            job,
            orchestration.phase,
            &orchestration.phase_attempt_id,
        ),
        SyncPhase::RecoveringLogin => build_recover_login_invocation(
            paths,
            session.main_executable.clone(),
            job.snapshot.task_key.account_id,
            &browser_identity.profile_key,
            job.snapshot.id,
            &orchestration.phase_attempt_id,
            orchestration
                .deadline_at_ms
                .ok_or_else(|| "uninstall_runtime_orchestration_mismatch".to_string())?,
        ),
    }
    .map_err(|_| "uninstall_runtime_invocation_invalid".to_string())?;
    let collector_expectation = CollectorProcessExpectation::for_invocation(
        job.snapshot.task_key.account_id,
        job.snapshot.id,
        orchestration.phase,
        &orchestration.phase_attempt_id,
        &invocation,
    )?;
    let collector_path = paths.phase_collector_identity_path(
        job.snapshot.task_key.account_id,
        job.snapshot.id,
        orchestration.phase,
        &orchestration.phase_attempt_id,
    )?;
    let collector: CollectorProcessIdentityV1 = read_json(&collector_path)
        .map_err(|_| "uninstall_runtime_collector_identity_invalid".to_string())?;
    if !collector_record_matches_expectation(&collector, &collector_expectation) {
        return Err("uninstall_runtime_collector_identity_mismatch".to_string());
    }
    let browser_path = paths.phase_browser_pid_path(
        job.snapshot.task_key.account_id,
        job.snapshot.id,
        orchestration.phase,
        &orchestration.phase_attempt_id,
    )?;
    let browser = if browser_path.is_file() {
        let record: BrowserProcessRecordV2 = read_sync_phase_payload(&browser_path, &orchestration)
            .map_err(|_| "uninstall_runtime_browser_identity_invalid".to_string())?;
        let expected_browser = match browser_identity.resolved_browser {
            BrowserIdV1::Edge => BrowserId::Edge,
            BrowserIdV1::Chrome => BrowserId::Chrome,
        };
        let expected_profile =
            douyin_history_archive::collector::browser_state::browser_profile_path(
                &paths.account_browser_root(&browser_identity.profile_key)?,
                expected_browser,
            );
        if record.browser_id != expected_browser
            || browser_executable_path_sha256(&record.executable_path)
                != browser_identity.executable_path_sha256
            || !browser_record_matches_expected_profile(&record, &expected_profile)
        {
            return Err("uninstall_runtime_browser_identity_mismatch".to_string());
        }
        Some((record, expected_profile))
    } else {
        None
    };
    Ok(TrustedSyncProcessCleanupV1 {
        collector,
        collector_expectation,
        browser,
    })
}

fn frozen_browser_identity(job: &StoredSyncJobV1) -> Result<BrowserExecutionIdentityV1, String> {
    let payload = serde_json::from_str::<SyncTaskPayloadV1>(&job.payload_json)
        .map_err(|_| "uninstall_runtime_job_payload_invalid".to_string())?;
    let frozen = freeze_task_payload(payload.clone())
        .map_err(|_| "uninstall_runtime_job_payload_invalid".to_string())?;
    if frozen.payload_json != job.payload_json
        || frozen.payload_sha256 != job.snapshot.task_payload_sha256
        || frozen.equivalence_sha256 != job.equivalence_sha256
        || payload.source() != job.snapshot.task_key.source
        || payload.source_instance_key() != job.snapshot.task_key.source_instance_key
    {
        return Err("uninstall_runtime_job_payload_invalid".to_string());
    }
    Ok(payload.browser().clone())
}

fn trusted_login_cleanups_for_uninstall(
    paths: &AppPaths,
    current_executable: &Path,
    connection: &Connection,
) -> Result<Vec<TrustedLoginCleanupV1>, String> {
    let entries = match fs::read_dir(&paths.temp) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("uninstall_runtime_login_scan_failed".to_string()),
    };
    let mut durable_states = Vec::new();
    let mut legacy_attempts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| "uninstall_runtime_login_scan_failed".to_string())?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|_| "uninstall_runtime_login_scan_failed".to_string())?
            .is_file()
        {
            continue;
        }
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| "uninstall_runtime_login_identity_invalid".to_string())?;
        if name.starts_with("login-attempt-") && name.ends_with(".state.json") {
            durable_states.push(path);
        } else if name.starts_with("account-") && name.ends_with(".attempt.json") {
            legacy_attempts.push(path);
        }
    }
    durable_states.sort();
    legacy_attempts.sort();

    let mut cleanups = Vec::new();
    let mut durable_ids = std::collections::BTreeSet::new();
    for state_path in durable_states {
        let attempt_id = durable_attempt_id_from_state_path(&state_path)?;
        if paths.login_attempt_state_path(&attempt_id)? != state_path {
            return Err("uninstall_runtime_login_identity_invalid".to_string());
        }
        let state = read_login_attempt_state(paths, &attempt_id)
            .map_err(|_| "uninstall_runtime_login_identity_invalid".to_string())?;
        if let Some(account_id) = state.target_account_id {
            let account = get_account_storage(connection, account_id)?
                .ok_or_else(|| "uninstall_runtime_login_identity_mismatch".to_string())?;
            if account.profile_key != state.profile_key {
                return Err("uninstall_runtime_login_identity_mismatch".to_string());
            }
        }
        let collector =
            trusted_durable_login_collector(paths, current_executable, &state, &attempt_id)?;
        let browser = trusted_login_browser(
            paths,
            &state.profile_key,
            &paths.login_task_browser_pid_path(&attempt_id)?,
        )?;
        durable_ids.insert(attempt_id.clone());
        cleanups.push(TrustedLoginCleanupV1 {
            process: TrustedLoginProcessCleanupV1 { collector, browser },
            files: LoginCleanupFilesV1::Durable { attempt_id },
        });
    }

    let mut legacy_ids = std::collections::BTreeSet::new();
    for attempt_path in legacy_attempts {
        let (account_id, attempt_id) = legacy_attempt_identity_from_path(&attempt_path)?;
        if paths.login_attempt_path(account_id, &attempt_id)? != attempt_path {
            return Err("uninstall_runtime_login_identity_invalid".to_string());
        }
        let attempt = douyin_history_archive::read_login_attempt(paths, account_id, &attempt_id)
            .map_err(|_| "uninstall_runtime_login_identity_invalid".to_string())?;
        let account = get_account_storage(connection, account_id)?
            .ok_or_else(|| "uninstall_runtime_login_identity_mismatch".to_string())?;
        if account.profile_key != attempt.profile_key {
            return Err("uninstall_runtime_login_identity_mismatch".to_string());
        }
        let browser = trusted_login_browser(
            paths,
            &attempt.profile_key,
            &paths.login_browser_pid_path(account_id, &attempt_id)?,
        )?;
        legacy_ids.insert((account_id, attempt_id.clone()));
        cleanups.push(TrustedLoginCleanupV1 {
            process: TrustedLoginProcessCleanupV1 {
                collector: None,
                browser,
            },
            files: LoginCleanupFilesV1::Legacy {
                account_id,
                attempt_id,
            },
        });
    }
    validate_no_orphan_login_process_records(paths, &durable_ids, &legacy_ids)?;
    Ok(cleanups)
}

fn durable_attempt_id_from_state_path(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .and_then(|name| name.strip_prefix("login-attempt-"))
        .and_then(|name| name.strip_suffix(".state.json"))
        .filter(|value| is_simple_uuid_text(value))
        .map(str::to_string)
        .ok_or_else(|| "uninstall_runtime_login_identity_invalid".to_string())
}

fn legacy_attempt_identity_from_path(path: &Path) -> Result<(i64, String), String> {
    let body = path
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(|name| name.strip_prefix("account-"))
        .and_then(|name| name.strip_suffix(".attempt.json"))
        .ok_or_else(|| "uninstall_runtime_login_identity_invalid".to_string())?;
    let (account_id, attempt_id) = body
        .split_once("-login-")
        .ok_or_else(|| "uninstall_runtime_login_identity_invalid".to_string())?;
    let account_id = account_id
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "uninstall_runtime_login_identity_invalid".to_string())?;
    if !is_simple_uuid_text(attempt_id) {
        return Err("uninstall_runtime_login_identity_invalid".to_string());
    }
    Ok((account_id, attempt_id.to_string()))
}

fn trusted_durable_login_collector(
    paths: &AppPaths,
    current_executable: &Path,
    state: &LoginAttemptStateV1,
    attempt_id: &str,
) -> Result<
    Option<(
        LoginCollectorProcessIdentityV1,
        LoginCollectorProcessExpectation,
    )>,
    String,
> {
    let path = paths.login_task_collector_identity_path(attempt_id)?;
    if !path.is_file() {
        return Ok(None);
    }
    let record: LoginCollectorProcessIdentityV1 = read_json(&path)
        .map_err(|_| "uninstall_runtime_login_collector_identity_invalid".to_string())?;
    let expectation = trusted_login_collector_expectations(paths, current_executable, state)?
        .into_iter()
        .find(|expectation| login_collector_record_matches_expectation(&record, expectation))
        .ok_or_else(|| "uninstall_runtime_login_collector_identity_mismatch".to_string())?;
    Ok(Some((record, expectation)))
}

fn trusted_login_browser(
    paths: &AppPaths,
    profile_key: &str,
    record_path: &Path,
) -> Result<Option<(BrowserProcessRecordV2, PathBuf)>, String> {
    if !record_path.is_file() {
        return Ok(None);
    }
    let record: BrowserProcessRecordV2 = read_json(record_path)
        .map_err(|_| "uninstall_runtime_login_browser_identity_invalid".to_string())?;
    let expected_profile = douyin_history_archive::collector::browser_state::browser_profile_path(
        &paths.account_browser_root(profile_key)?,
        record.browser_id,
    );
    if !browser_record_matches_expected_profile(&record, &expected_profile) {
        return Err("uninstall_runtime_login_browser_identity_mismatch".to_string());
    }
    Ok(Some((record, expected_profile)))
}

fn validate_no_orphan_login_process_records(
    paths: &AppPaths,
    durable_ids: &std::collections::BTreeSet<String>,
    legacy_ids: &std::collections::BTreeSet<(i64, String)>,
) -> Result<(), String> {
    let entries = match fs::read_dir(&paths.temp) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("uninstall_runtime_login_scan_failed".to_string()),
    };
    for entry in entries {
        let entry = entry.map_err(|_| "uninstall_runtime_login_scan_failed".to_string())?;
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| "uninstall_runtime_login_identity_invalid".to_string())?;
        if let Some(attempt_id) = name
            .strip_prefix("login-")
            .and_then(|value| value.strip_suffix(".collector.pid"))
            .or_else(|| {
                name.strip_prefix("login-")
                    .and_then(|value| value.strip_suffix(".browser.pid"))
            })
        {
            if !is_simple_uuid_text(attempt_id) || !durable_ids.contains(attempt_id) {
                return Err("uninstall_runtime_login_state_missing".to_string());
            }
        }
        if name.starts_with("account-")
            && name.contains("-login-")
            && name.ends_with(".browser.pid")
        {
            let synthetic = PathBuf::from(
                name.strip_suffix(".browser.pid")
                    .map(|value| format!("{value}.attempt.json"))
                    .ok_or_else(|| "uninstall_runtime_login_identity_invalid".to_string())?,
            );
            let identity = legacy_attempt_identity_from_path(&synthetic)?;
            if !legacy_ids.contains(&identity) {
                return Err("uninstall_runtime_login_state_missing".to_string());
            }
        }
    }
    Ok(())
}

fn remove_login_cleanup_files(paths: &AppPaths, files: &LoginCleanupFilesV1) -> Result<(), String> {
    let candidates = match files {
        LoginCleanupFilesV1::Durable { attempt_id } => vec![
            paths.login_attempt_state_path(attempt_id)?,
            paths.login_task_identity_path(attempt_id)?,
            paths.login_task_progress_path(attempt_id)?,
            paths.login_task_cancel_path(attempt_id)?,
            paths.login_task_browser_pid_path(attempt_id)?,
            paths.login_task_collector_identity_path(attempt_id)?,
            paths.login_task_diagnostic_path(attempt_id)?,
            paths.login_task_log_path(attempt_id)?,
        ],
        LoginCleanupFilesV1::Legacy {
            account_id,
            attempt_id,
        } => vec![
            paths.login_attempt_path(*account_id, attempt_id)?,
            paths.login_identity_path(*account_id, attempt_id)?,
            paths.login_progress_path(*account_id, attempt_id)?,
            paths.login_cancel_path(*account_id, attempt_id)?,
            paths.login_browser_pid_path(*account_id, attempt_id)?,
            paths.login_diagnostic_path(*account_id, attempt_id)?,
            paths.login_log_path(*account_id, attempt_id)?,
        ],
    };
    for path in candidates {
        remove_file_if_exists_strict(&path)?;
    }
    Ok(())
}

fn recover_durable_login_attempts_with<C, B>(
    paths: &AppPaths,
    current_executable: &Path,
    mut terminate_collector: C,
    mut terminate_browser: B,
) -> Result<(), String>
where
    C: FnMut(
        &LoginCollectorProcessIdentityV1,
        &LoginCollectorProcessExpectation,
    ) -> Result<(), String>,
    B: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let entries = match fs::read_dir(&paths.temp) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    let mut state_paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("login-attempt-") && name.ends_with(".state.json")
                })
        })
        .collect::<Vec<_>>();
    state_paths.sort();

    for state_path in state_paths {
        let name = state_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "invalid_login_attempt_state".to_string())?;
        let attempt_id = name
            .strip_prefix("login-attempt-")
            .and_then(|name| name.strip_suffix(".state.json"))
            .filter(|value| is_simple_uuid_text(value))
            .ok_or_else(|| "invalid_login_attempt_state".to_string())?;
        if paths.login_attempt_state_path(attempt_id)? != state_path {
            return Err("login_attempt_state_mismatch".to_string());
        }
        let state = read_login_attempt_state(paths, attempt_id)?;

        let collector_path = paths.login_task_collector_identity_path(attempt_id)?;
        if collector_path.is_file() {
            let record: LoginCollectorProcessIdentityV1 = read_json(&collector_path)
                .map_err(|_| "invalid_login_collector_process_identity".to_string())?;
            let expectation =
                trusted_login_collector_expectations(paths, current_executable, &state)?
                    .into_iter()
                    .find(|expectation| {
                        login_collector_record_matches_expectation(&record, expectation)
                    })
                    .ok_or_else(|| "login_collector_process_identity_mismatch".to_string())?;
            terminate_collector(&record, &expectation)?;
        }

        let browser_path = paths.login_task_browser_pid_path(attempt_id)?;
        if browser_path.is_file() {
            let record: BrowserProcessRecordV2 = read_json(&browser_path)
                .map_err(|_| "invalid_browser_process_record".to_string())?;
            let expected_profile =
                douyin_history_archive::collector::browser_state::browser_profile_path(
                    &paths.account_browser_root(&state.profile_key)?,
                    record.browser_id,
                );
            if !browser_record_matches_expected_profile(&record, &expected_profile) {
                return Err("browser_profile_scope_mismatch".to_string());
            }
            terminate_browser(&record, &expected_profile)?;
        }

        remove_durable_login_files(paths, attempt_id)?;
        if state.target_account_id.is_none() {
            remove_unclaimed_profile(&paths.browser_profiles, &state.profile_key)?;
        }
    }
    Ok(())
}

fn trusted_login_collector_expectations(
    paths: &AppPaths,
    current_executable: &Path,
    state: &LoginAttemptStateV1,
) -> Result<Vec<LoginCollectorProcessExpectation>, String> {
    [
        BrowserPreferenceV1::Auto,
        BrowserPreferenceV1::Edge,
        BrowserPreferenceV1::Chrome,
    ]
    .into_iter()
    .map(|browser_preference| {
        let invocation = build_login_invocation(
            paths,
            current_executable.to_path_buf(),
            state.target_account_id,
            &state.profile_key,
            &state.login_attempt_id,
            browser_preference,
        )?;
        LoginCollectorProcessExpectation::for_invocation(
            &state.login_attempt_id,
            &state.profile_key,
            state.target_account_id,
            &invocation,
        )
    })
    .collect()
}

fn remove_durable_login_files(paths: &AppPaths, attempt_id: &str) -> Result<(), String> {
    for path in [
        paths.login_attempt_state_path(attempt_id)?,
        paths.login_task_identity_path(attempt_id)?,
        paths.login_task_progress_path(attempt_id)?,
        paths.login_task_cancel_path(attempt_id)?,
        paths.login_task_browser_pid_path(attempt_id)?,
        paths.login_task_collector_identity_path(attempt_id)?,
        paths.login_task_diagnostic_path(attempt_id)?,
    ] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

struct RuntimeRunningJobRecovery<'a> {
    paths: &'a AppPaths,
    registry: &'a ExecutorRegistry,
}

impl RunningJobRecovery for RuntimeRunningJobRecovery<'_> {
    fn recover_running_job(
        &mut self,
        job: &StoredSyncJobV1,
        disposition: StartupRecoveryDispositionV1,
    ) -> Result<(), String> {
        recover_interrupted_job_with_registry(
            self.paths,
            self.registry,
            job.snapshot.task_key.account_id,
            job.snapshot.id,
            Some(disposition),
            terminate_recorded_browser_v2,
        )
    }
}

fn startup_recovery_context<'a>(
    paths: &'a AppPaths,
    root_id: &'a str,
    recovered_at: &'a str,
    probe: &'a dyn ProcessIdentityProbe,
    running_recovery: &'a mut dyn RunningJobRecovery,
    queue_validator: &'a RuntimeSyncWorkerValidator,
) -> StartupRecoveryContext<'a> {
    StartupRecoveryContext {
        paths,
        application_id: APPLICATION_ID,
        root_id,
        recovered_at,
        process_probe: probe,
        running_recovery,
        queue_validator,
    }
}

fn store_startup_continuation(
    state: &AppState,
    permit: &StartupContinuationPermitV1,
) -> Result<(), String> {
    let mut slot = state
        .startup_continuation_permit
        .lock()
        .map_err(|_| "startup_continuation_permit_poisoned".to_string())?;
    *slot = Some(permit.clone());
    Ok(())
}

fn load_startup_continuation(
    state: &AppState,
) -> Result<Option<StartupContinuationPermitV1>, String> {
    state
        .startup_continuation_permit
        .lock()
        .map(|permit| permit.clone())
        .map_err(|_| "startup_continuation_permit_poisoned".to_string())
}

fn recover_sync_runtime_state(
    state: &AppState,
    paths: &AppPaths,
) -> Result<Option<StartupContinuationPermitV1>, String> {
    let bootstrap =
        read_bootstrap(&paths.bootstrap_dir)?.ok_or_else(|| "bootstrap_missing".to_string())?;
    let validator = RuntimeSyncWorkerValidator::new(
        paths.data_root.clone(),
        paths.bootstrap_dir.clone(),
        CapabilityRegistry::stage_3_3().all().to_vec(),
    );
    let probe = SystemProcessIdentityProbe;
    let registry = state
        .sync_registry
        .get()
        .ok_or_else(|| "sync_registry_not_initialized".to_string())?;
    let mut running_recovery = RuntimeRunningJobRecovery { paths, registry };
    let recovered_at = now_iso();
    match state
        .startup_recovery
        .recover_once(startup_recovery_context(
            paths,
            &bootstrap.current_root.root_id,
            &recovered_at,
            &probe,
            &mut running_recovery,
            &validator,
        )) {
        Ok(recovered) => {
            let permit = recovered.continuation_permit;
            store_startup_continuation(state, &permit)?;
            Ok(Some(permit))
        }
        Err(error) if error.is_blocked() => Ok(None),
        Err(error) => Err(error.code().to_string()),
    }
}

fn resolve_sync_runtime_state(
    state: &AppState,
    paths: &AppPaths,
    disposition: StartupRecoveryDispositionV1,
) -> Result<StartupContinuationPermitV1, String> {
    if state.startup_recovery.state()? == StartupRecoveryStateV1::Ready {
        return load_startup_continuation(state)?
            .ok_or_else(|| "startup_continuation_permit_missing".to_string());
    }
    let bootstrap =
        read_bootstrap(&paths.bootstrap_dir)?.ok_or_else(|| "bootstrap_missing".to_string())?;
    let validator = RuntimeSyncWorkerValidator::new(
        paths.data_root.clone(),
        paths.bootstrap_dir.clone(),
        CapabilityRegistry::stage_3_3().all().to_vec(),
    );
    let probe = SystemProcessIdentityProbe;
    let registry = state
        .sync_registry
        .get()
        .ok_or_else(|| "sync_registry_not_initialized".to_string())?;
    let mut running_recovery = RuntimeRunningJobRecovery { paths, registry };
    let recovered_at = now_iso();
    let recovered = state
        .startup_recovery
        .resolve_fault(
            startup_recovery_context(
                paths,
                &bootstrap.current_root.root_id,
                &recovered_at,
                &probe,
                &mut running_recovery,
                &validator,
            ),
            disposition,
        )
        .map_err(|error| error.code().to_string())?;
    let permit = recovered.continuation_permit;
    store_startup_continuation(state, &permit)?;
    Ok(permit)
}

fn recover_desktop_startup_runtime(
    app: &AppHandle,
    state: &AppState,
    coordination_dir: &Path,
    executable_identity: &ExecutableIdentityV1,
) -> Result<Option<StartupContinuationPermitV1>, String> {
    let resolved_paths = RefCell::new(None::<AppPaths>);
    let migration_work = RefCell::new(None::<SchemaV5RecoveryWork>);
    let mut context = DesktopStartupContext::new();

    context.set_stage(StartupStageV1::UninstallRecovery, || {
        recover_uninstall_state_before_bootstrap(coordination_dir, executable_identity).map(|_| ())
    });
    context.set_stage(StartupStageV1::Bootstrap, || {
        let paths = resolve_initial_paths(app)?;
        prepare_app_directories(&paths)?;
        state.initialize_runtime_paths(paths.clone())?;
        *resolved_paths.borrow_mut() = Some(paths);
        Ok(())
    });
    context.set_stage(StartupStageV1::Schema, || {
        let paths = resolved_paths
            .borrow()
            .clone()
            .ok_or_else(|| "runtime_paths_not_initialized".to_string())?;
        let mut connection = open_database(&paths.database)?;
        *migration_work.borrow_mut() = Some(migrate_schema_to_latest_at_startup(&mut connection)?);
        Ok(())
    });
    context.set_stage(StartupStageV1::SchemaCleanup, || {
        let paths = resolved_paths
            .borrow()
            .clone()
            .ok_or_else(|| "runtime_paths_not_initialized".to_string())?;
        let migration_cleanup = migration_work
            .borrow_mut()
            .take()
            .ok_or_else(|| "schema_v5_cleanup_work_missing".to_string())?;
        consume_schema_v5_recovery_work(&migration_cleanup, |identity| {
            cleanup_schema_upgrade_job(&paths, identity)
        })?;
        let connection = open_database(&paths.database)?;
        let rebuilt_cleanup = rebuild_schema_v5_cleanup_work(&connection, &paths)?;
        consume_schema_v5_recovery_work(&rebuilt_cleanup, |identity| {
            cleanup_schema_upgrade_job(&paths, identity)
        })?;
        if !rebuild_schema_v5_cleanup_work(&connection, &paths)?
            .interrupted_v4_jobs
            .is_empty()
        {
            return Err("schema_v5_cleanup_incomplete".to_string());
        }
        Ok(())
    });
    context.set_sync_recovery(|| {
        let paths = resolved_paths
            .borrow()
            .clone()
            .ok_or_else(|| "runtime_paths_not_initialized".to_string())?;
        recover_sync_runtime_state(state, &paths)
    });
    context.set_stage(StartupStageV1::NoRunningAssertion, || {
        let paths = resolved_paths
            .borrow()
            .clone()
            .ok_or_else(|| "runtime_paths_not_initialized".to_string())?;
        let connection = open_database(&paths.database)?;
        QueueStore::assert_no_running_jobs(&connection)
    });

    recover_desktop_startup_once(&mut context)
}

fn apply_startup_window_decision(
    app: &AppHandle,
    decision: StartupWindowDecisionV1,
) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main_window_missing".to_string())?;
    match decision {
        StartupWindowDecisionV1::KeepHidden => window.hide().map_err(|error| error.to_string()),
        StartupWindowDecisionV1::ShowMain | StartupWindowDecisionV1::ShowRecovery => {
            window.show().map_err(|error| error.to_string())?;
            window.set_focus().map_err(|error| error.to_string())
        }
    }
}

fn continue_desktop_startup_runtime(
    app: &AppHandle,
    state: &AppState,
    permit: StartupContinuationPermitV1,
) -> Result<(), String> {
    let paths = state.runtime_paths()?.acquire()?;
    let paths = (*paths).clone();
    continue_desktop_startup_from_ready(permit, |stage| match stage {
        StartupStageV1::DeviceAndOnboarding => {
            recover_login_runtime_state(&paths)?;
            if state.device_settings.get().is_none() {
                let connection = open_database(&paths.database)?;
                let portable = load_portable_settings(&paths.data_root)?;
                let device_settings = initialize_device_settings_store(
                    paths.bootstrap_dir.clone(),
                    &connection,
                    &portable,
                )?;
                let autostart_error = std::env::current_exe()
                    .map_err(|_| "invalid_autostart_executable".to_string())
                    .and_then(|executable| {
                        repair_autostart_on_startup(
                            &device_settings,
                            &state.desktop_settings_tx,
                            &executable,
                        )
                    })
                    .err();
                state.record_autostart_error(autostart_error);
                state.initialize_device_settings(device_settings)?;
            }
            Ok(())
        }
        StartupStageV1::TrayAndWindow => {
            let device = state
                .device_settings
                .get()
                .ok_or_else(|| "device_settings_not_initialized".to_string())?
                .load()?;
            let activation_uninitialized = state
                .activation_listener
                .lock()
                .map(|slot| slot.is_none())
                .map_err(|_| "gui_activation_listener_poisoned".to_string())?;
            if activation_uninitialized {
                initialize_activation_listener(app, state)?;
            }
            if let Err(error) = create_tray(app, state) {
                state.tray_available.store(false, Ordering::Release);
                if let Ok(mut tray_error) = state.tray_error.lock() {
                    *tray_error = Some("tray_unavailable".to_string());
                }
                let _ = error;
                eprintln!("tray_unavailable");
                apply_startup_window_decision(app, StartupWindowDecisionV1::ShowMain)?;
                return Ok(());
            }
            let decision = startup_window_decision(StartupWindowDecisionInputV1 {
                recovery_blocked: false,
                autostart: state.started_by_autostart.get().copied().unwrap_or(false),
                silent_autostart: device.silent_autostart,
                onboarding_completed: device.onboarding_completed,
                tray_available: state.tray_available.load(Ordering::Acquire),
            });
            apply_startup_window_decision(app, decision)
        }
        StartupStageV1::Worker => {
            let scheduler_paused = state
                .device_settings
                .get()
                .ok_or_else(|| "device_settings_not_initialized".to_string())?
                .load()?
                .scheduler_paused;
            state
                .operation_admission
                .set_automatic_paused(scheduler_paused)?;
            initialize_sync_runtime(app, state, &paths)?;
            if let Err(error) = initialize_cover_cache_runtime(state) {
                rollback_sync_runtime(state);
                return Err(error);
            }
            if let Err(error) = initialize_scheduler_runtime(app, state) {
                rollback_cover_cache_runtime(state);
                rollback_sync_runtime(state);
                return Err(error);
            }
            if let Err(error) = initialize_automatic_backup_runtime(app, state) {
                rollback_scheduler_runtime(state);
                rollback_cover_cache_runtime(state);
                rollback_sync_runtime(state);
                return Err(error);
            }
            if let Err(error) = initialize_download_runtime(app, state, &paths) {
                rollback_automatic_backup_runtime(state);
                rollback_scheduler_runtime(state);
                rollback_cover_cache_runtime(state);
                rollback_sync_runtime(state);
                return Err(error);
            }
            state.normal_runtime_ready.store(true, Ordering::Release);
            spawn_automatic_update_check(app);
            if refresh_tray_menu(app).is_err() {
                eprintln!("tray_menu_refresh_failed");
            }
            Ok(())
        }
        _ => Err("desktop_startup_stage_mismatch".to_string()),
    })
}

fn current_process_creation_time_100ns() -> Result<u64, String> {
    SystemProcessIdentityProbe
        .creation_time_100ns(std::process::id())?
        .ok_or_else(|| "process_identity_query_failed".to_string())
}

fn request_running_sync_on_exit(state: &AppState) -> Result<(), String> {
    let running = {
        let running = state
            .running
            .lock()
            .map_err(|_| "同步状态锁已损坏".to_string())?;
        running
            .as_ref()
            .map(|task| (task.paths.clone(), task.identity()))
    };
    if let Some((paths, identity)) = running {
        request_sync_cancellation(&paths, &identity, &now_iso())?;
    }
    Ok(())
}

fn best_effort_cleanup_running_processes(state: &AppState) {
    let sync_child = state.running.lock().ok().and_then(|running| {
        running
            .as_ref()
            .and_then(|task| task.child.as_ref().map(Arc::clone))
    });
    if let Some(child) = sync_child {
        if let Ok(child) = child.lock() {
            let _ = child.kill();
        }
    }
    force_cleanup_running_login(state);
}

fn perform_controlled_exit(app: &AppHandle) -> Result<ExitCompletionV1, String> {
    let state = app.state::<AppState>();
    let stable_paths = Arc::new(Mutex::new(None::<AppPaths>));
    let mut context = douyin_history_archive::desktop_lifecycle::ControlledExitContext::new(
        state.operation_admission.clone(),
    );
    context.owned_operation_timeout = Duration::from_secs(2);
    context.total_timeout = Duration::from_secs(10);

    {
        let state = state.inner();
        let stable_paths = Arc::clone(&stable_paths);
        context.before_owned_wait = Box::new(move || {
            let lease = state.acquire_paths()?;
            let paths = (*lease).clone();
            let bootstrap = read_bootstrap(&paths.bootstrap_dir)?
                .ok_or_else(|| "bootstrap_missing".to_string())?;
            if !same_windows_path(&bootstrap.current_root.path, &paths.data_root) {
                return Err("data_root_source_mismatch".to_string());
            }
            *stable_paths
                .lock()
                .map_err(|_| "shutdown_paths_poisoned".to_string())? = Some(paths);
            Ok(())
        });
    }
    {
        let stable_paths = Arc::clone(&stable_paths);
        context.write_marker = Box::new(move || {
            let paths = stable_paths
                .lock()
                .map_err(|_| "shutdown_paths_poisoned".to_string())?
                .clone()
                .ok_or_else(|| "shutdown_paths_unavailable".to_string())?;
            let bootstrap = read_bootstrap(&paths.bootstrap_dir)?
                .ok_or_else(|| "bootstrap_missing".to_string())?;
            write_shutdown_intent(
                &paths.bootstrap_dir,
                &ShutdownIntentV1 {
                    schema_version: 1,
                    application_id: APPLICATION_ID.to_string(),
                    root_id: bootstrap.current_root.root_id,
                    pid: std::process::id(),
                    process_created_at_100ns: current_process_creation_time_100ns()?,
                    started_at: now_iso(),
                    cancel_queued: true,
                },
            )
        });
    }
    {
        let state = state.inner();
        context.stop_worker = Box::new(move || {
            state.background_tasks.close()?;
            if let Some(listener) = state
                .activation_listener
                .lock()
                .map_err(|_| "gui_activation_listener_poisoned".to_string())?
                .as_ref()
            {
                listener.stop();
            }
            stop_download_runtime(state)?;
            stop_automatic_backup_runtime(state)?;
            if let Some(worker) = state.scheduler_worker.get() {
                worker.stop()?;
            }
            if let Some(worker) = state.sync_worker.get() {
                worker.quiesce();
            }
            if let Some(worker) = state.cover_cache_worker.get() {
                worker.stop();
            }
            Ok(())
        });
    }
    {
        let state = state.inner();
        context.cancel_running = Box::new(move || {
            request_running_sync_on_exit(state)?;
            let running_login =
                state.running_login.lock().ok().and_then(|running| {
                    running.as_ref().map(|running| running.cancel_path.clone())
                });
            if let Some(cancel_path) = running_login {
                fs::write(cancel_path, b"cancel\n").map_err(|error| error.to_string())?;
            }
            Ok(())
        });
    }
    {
        let state = state.inner();
        context.wait_runtime = Box::new(move |timeout| {
            let deadline = Instant::now() + timeout;
            if !state.background_tasks.wait_until_idle(timeout)? {
                return Ok(false);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if !wait_download_runtime(state, remaining)? {
                return Ok(false);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Some(worker) = state.automatic_backup_worker.get() {
                if !worker.wait_until_stopped(remaining)? {
                    return Ok(false);
                }
                join_automatic_backup_runtime(state)?;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Some(worker) = state.scheduler_worker.get() {
                if !worker.wait_until_stopped(remaining)? {
                    return Ok(false);
                }
                worker.join()?;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Some(worker) = state.sync_worker.get() {
                if !worker.wait_until_stopped(remaining)? {
                    return Ok(false);
                }
                worker.join()?;
            }
            if let Some(worker) = state.cover_cache_worker.get() {
                worker.join()?;
            }
            if let Some(listener) = state
                .activation_listener
                .lock()
                .map_err(|_| "gui_activation_listener_poisoned".to_string())?
                .as_ref()
            {
                listener.join()?;
            }
            let admission = state.operation_admission.snapshot()?;
            Ok(admission.continuation_count == 0)
        });
    }
    {
        let stable_paths = Arc::clone(&stable_paths);
        context.sweep_queued = Box::new(move || {
            let paths = stable_paths
                .lock()
                .map_err(|_| "shutdown_paths_poisoned".to_string())?
                .clone()
                .ok_or_else(|| "shutdown_paths_unavailable".to_string())?;
            let mut connection = open_database(&paths.database)?;
            QueueStore::cancel_all_queued_jobs(
                &mut connection,
                &now_iso(),
                "controlled_exit_cancelled_before_start",
            )?;
            Ok(())
        });
    }
    {
        let stable_paths = Arc::clone(&stable_paths);
        context.assert_idle = Box::new(move || {
            let paths = stable_paths
                .lock()
                .map_err(|_| "shutdown_paths_poisoned".to_string())?
                .clone()
                .ok_or_else(|| "shutdown_paths_unavailable".to_string())?;
            let connection = open_database(&paths.database)?;
            QueueStore::assert_no_running_jobs(&connection)?;
            if !QueueStore::list_queued_jobs(&connection)?.is_empty() {
                return Err("sync_queued_job_cleanup_required".to_string());
            }
            Ok(())
        });
    }
    {
        let state = state.inner();
        let stable_paths = Arc::clone(&stable_paths);
        context.cleanup_task_files = Box::new(move || {
            let paths = stable_paths
                .lock()
                .map_err(|_| "shutdown_paths_poisoned".to_string())?
                .clone()
                .ok_or_else(|| "shutdown_paths_unavailable".to_string())?;
            for job in QueueStore::list_running_jobs(&open_database(&paths.database)?)? {
                cleanup_recovered_job_files(
                    &paths,
                    job.snapshot.task_key.account_id,
                    job.snapshot.id,
                );
            }
            if let Ok(mut running) = state.running.lock() {
                *running = None;
            }
            if let Ok(mut running_login) = state.running_login.lock() {
                *running_login = None;
            }
            Ok(())
        });
    }
    {
        let stable_paths = Arc::clone(&stable_paths);
        context.remove_marker = Box::new(move || {
            let paths = stable_paths
                .lock()
                .map_err(|_| "shutdown_paths_poisoned".to_string())?
                .clone()
                .ok_or_else(|| "shutdown_paths_unavailable".to_string())?;
            remove_shutdown_intent(&paths.bootstrap_dir)
        });
    }
    {
        let state = state.inner();
        context.force_process_cleanup = Box::new(move || {
            best_effort_cleanup_running_processes(state);
            Ok(())
        });
    }
    {
        let state = state.inner();
        context.best_effort_cleanup = Box::new(move || {
            best_effort_cleanup_running_processes(state);
            Ok(())
        });
    }

    douyin_history_archive::desktop_lifecycle::request_app_exit(&mut context)
}

fn start_controlled_exit(app: AppHandle) {
    let state = app.state::<AppState>();
    let app_for_thread = app.clone();
    let spawn = thread::Builder::new()
        .name("emily-controlled-exit".to_string())
        .spawn(move || {
            let result = perform_controlled_exit(&app_for_thread);
            match result {
                Ok(ExitCompletionV1::Clean) => {
                    if app_for_thread
                        .state::<AppState>()
                        .exit_request
                        .mark_cleanup_complete()
                        .is_ok()
                    {
                        app_for_thread.exit(0);
                    } else {
                        app_for_thread.exit(1);
                    }
                }
                Ok(ExitCompletionV1::HardTimeout) => std::process::exit(1),
                Err(_) => {
                    let _ = app_for_thread
                        .state::<AppState>()
                        .exit_request
                        .abort_request();
                }
            }
        });
    if spawn.is_err() {
        let _ = state.exit_request.abort_request();
    }
}

fn handle_main_window_close(app: &AppHandle, api: &tauri::CloseRequestApi) {
    let state = app.state::<AppState>();
    if !state.normal_runtime_ready.load(Ordering::Acquire) {
        return;
    }
    let decision = (|| {
        let paths = state.acquire_paths()?;
        let portable = load_portable_settings(&paths.data_root)?;
        let device = state
            .device_settings
            .get()
            .ok_or_else(|| "device_settings_not_initialized".to_string())?
            .load()?;
        Ok::<_, String>(close_window_decision(CloseWindowDecisionInputV1 {
            close_behavior: portable.global.close_behavior,
            tray_available: state.tray_available.load(Ordering::Acquire),
            onboarding_completed: device.onboarding_completed,
            first_tray_hint_shown: device.first_tray_hint_shown,
            system_exit: false,
        }))
    })();
    match decision {
        Ok(CloseWindowDecisionV1::HideToTray { show_hint }) => {
            api.prevent_close();
            if let Some(window) = app.get_webview_window("main") {
                if window.hide().is_err() {
                    eprintln!("main_window_hide_failed");
                    let _ = window.show();
                    return;
                }
            }
            if show_hint {
                let mark_result = state
                    .device_settings
                    .get()
                    .ok_or_else(|| "device_settings_not_initialized".to_string())
                    .and_then(DeviceSettingsStore::mark_first_tray_hint_shown);
                if mark_result.is_err() {
                    eprintln!("tray_hint_state_update_failed");
                } else if send_system_notification(app, NotificationKindV1::FirstTrayHide).is_err()
                {
                    eprintln!("notification_delivery_failed");
                }
            }
        }
        Ok(CloseWindowDecisionV1::KeepVisibleTrayUnavailable) => {
            api.prevent_close();
            eprintln!("tray_unavailable");
            let _ = show_main_window(app);
        }
        Ok(CloseWindowDecisionV1::RequestExit) => {
            api.prevent_close();
            request_app_exit_runtime(app);
        }
        Err(_) => {
            api.prevent_close();
            eprintln!("close_window_decision_failed");
            let _ = show_main_window(app);
        }
    }
}

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

fn require_account_status(
    connection: &Connection,
    account_id: i64,
    allowed: &[&str],
) -> Result<(), String> {
    let status = connection
        .query_row(
            "SELECT status FROM accounts WHERE id = ?1",
            [account_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "account_not_found".to_string())?;
    if allowed.iter().any(|allowed| *allowed == status) {
        Ok(())
    } else {
        Err("account_not_available".to_string())
    }
}

fn account_profile_key(connection: &Connection, account_id: i64) -> Result<String, String> {
    connection
        .query_row(
            "SELECT profile_key FROM accounts WHERE id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "account_not_found".to_string())
}

#[cfg(test)]
fn start_sync_job(connection: &mut Connection, account_id: i64) -> Result<i64, String> {
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let running_exists: i64 = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_jobs WHERE status = 'running')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if running_exists == 1 {
        return Err("sync_already_running".to_string());
    }
    let queue_seq: i64 = transaction
        .query_row(
            "SELECT next_queue_seq FROM sync_queue_state WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if queue_seq == i64::MAX {
        return Err("sync_queue_sequence_exhausted".to_string());
    }
    transaction
        .execute(
            "UPDATE sync_queue_state SET next_queue_seq = ?1 WHERE singleton = 1",
            [queue_seq + 1],
        )
        .map_err(|error| error.to_string())?;
    let started_at = now_iso();
    let compatibility_hash = "0".repeat(64);
    transaction
        .execute(
            "INSERT INTO sync_jobs
             (account_id, source, source_instance_key, trigger, queued_at, queue_seq,
              settings_revision, payload_json, payload_sha256, equivalence_sha256,
              started_at, status)
             VALUES (?1, 'history', '', 'manual', ?2, ?3, 0, '{}', ?4, ?4, ?2, 'running')",
            params![account_id, started_at, queue_seq, compatibility_hash],
        )
        .map_err(|error| error.to_string())?;
    let job_id = transaction.last_insert_rowid();
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(job_id)
}

#[cfg(test)]
fn recover_interrupted_sync_jobs(connection: &Connection) -> Result<Vec<(i64, i64)>, String> {
    let jobs = {
        let mut statement = connection
            .prepare("SELECT account_id, id FROM sync_jobs WHERE status = 'running' ORDER BY id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    };
    Ok(jobs)
}

#[cfg(test)]
fn recover_interrupted_job_with<F>(
    paths: &AppPaths,
    account_id: i64,
    job_id: i64,
    terminate: F,
) -> Result<(), String>
where
    F: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let registry = test_sync_registry();
    recover_interrupted_job_with_registry(paths, &registry, account_id, job_id, None, terminate)
}

#[cfg(test)]
fn recover_interrupted_job_with_disposition<F>(
    paths: &AppPaths,
    account_id: i64,
    job_id: i64,
    disposition: douyin_history_archive::startup_recovery::StartupRecoveryDispositionV1,
    terminate: F,
) -> Result<(), String>
where
    F: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let registry = test_sync_registry();
    recover_interrupted_job_with_registry(
        paths,
        &registry,
        account_id,
        job_id,
        Some(disposition),
        terminate,
    )
}

fn recover_interrupted_job_with_registry<F>(
    paths: &AppPaths,
    registry: &ExecutorRegistry,
    account_id: i64,
    job_id: i64,
    explicit_disposition: Option<
        douyin_history_archive::startup_recovery::StartupRecoveryDispositionV1,
    >,
    mut terminate: F,
) -> Result<(), String>
where
    F: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let recovery = (|| {
        let mut connection = open_database(&paths.database)?;
        assert_schema_v5(&connection)?;
        let job = connection
            .query_row(
                "SELECT account_id, status FROM sync_jobs WHERE id = ?1",
                [job_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "sync_job_mismatch".to_string())?;
        if job.0 != account_id || job.1 != "running" {
            return Err("sync_job_mismatch".to_string());
        }

        let orchestration = match read_orchestration(paths, account_id, job_id) {
            Ok(orchestration) => orchestration,
            Err(_) => {
                let outcome = match explicit_disposition {
                    Some(
                        douyin_history_archive::startup_recovery::StartupRecoveryDispositionV1::CancelQueued,
                    ) => SyncTerminalOutcome::Cancelled {
                        error_message: Some("同步已取消".to_string()),
                    },
                    _ => SyncTerminalOutcome::Failed {
                        error_code: "collector_interrupted".to_string(),
                        error_message: Some("同步任务因应用退出或异常中断".to_string()),
                    },
                };
                return finish_sync_with_account_state(
                    &mut connection,
                    SyncJobIdentity {
                        account_id,
                        sync_job_id: job_id,
                    },
                    outcome,
                );
            }
        };
        let identity = RunningSyncIdentity {
            account_id,
            job_id,
            phase: orchestration.phase,
            phase_attempt_id: orchestration.phase_attempt_id.clone(),
        };
        validate_running_orchestration(&orchestration, &identity)?;
        terminate_recorded_sync_collector(paths, &identity);
        terminate_recorded_sync_browser_with(paths, &identity, &mut terminate);

        let result = if matches!(
            identity.phase,
            SyncPhase::Collecting | SyncPhase::RetryingAfterLogin
        ) {
            paths
                .phase_result_path(
                    account_id,
                    job_id,
                    identity.phase,
                    &identity.phase_attempt_id,
                )
                .ok()
                .and_then(|path| read_sync_execution_envelope(paths, &path, &orchestration).ok())
        } else {
            None
        };
        let baseline_decision = startup_recovery_decision(
            &orchestration,
            result.is_some(),
            paths.cancel_path(account_id, job_id).exists(),
        );
        let decision = if baseline_decision == StartupRecoveryDecision::Consume {
            baseline_decision
        } else if let Some(disposition) = explicit_disposition {
            match disposition {
                douyin_history_archive::startup_recovery::StartupRecoveryDispositionV1::PreserveQueued => {
                    StartupRecoveryDecision::FailInterrupted
                }
                douyin_history_archive::startup_recovery::StartupRecoveryDispositionV1::CancelQueued => {
                    StartupRecoveryDecision::Cancel
                }
            }
        } else {
            baseline_decision
        };
        match decision {
            StartupRecoveryDecision::Consume => {
                let envelope = result.ok_or_else(|| "collector_interrupted".to_string())?;
                import_execution_envelope(registry, &mut connection, job_id, envelope).map(|_| ())
            }
            StartupRecoveryDecision::Cancel => finish_sync_with_account_state(
                &mut connection,
                SyncJobIdentity {
                    account_id,
                    sync_job_id: job_id,
                },
                SyncTerminalOutcome::Cancelled {
                    error_message: Some("同步已取消".to_string()),
                },
            ),
            StartupRecoveryDecision::FailInterrupted => finish_sync_with_account_state(
                &mut connection,
                SyncJobIdentity {
                    account_id,
                    sync_job_id: job_id,
                },
                SyncTerminalOutcome::Failed {
                    error_code: "collector_interrupted".to_string(),
                    error_message: Some("同步任务因应用退出或异常中断".to_string()),
                },
            ),
        }
    })();
    cleanup_recovered_job_files(paths, account_id, job_id);
    recovery
}

#[cfg(test)]
fn mark_sync_job(
    connection: &Connection,
    account_id: i64,
    job_id: i64,
    status: &str,
    error_message: Option<&str>,
) -> Result<(), String> {
    let updated = connection
        .execute(
            "UPDATE sync_jobs SET status = ?3, finished_at = ?4, error_message = ?5
             WHERE id = ?1 AND account_id = ?2 AND status = 'running'",
            params![job_id, account_id, status, now_iso(), error_message],
        )
        .map_err(|error| error.to_string())?;
    if updated == 1 {
        Ok(())
    } else {
        Err("sync_job_mismatch".to_string())
    }
}

#[cfg(test)]
fn fail_started_sync(
    connection: &Connection,
    account_id: i64,
    job_id: i64,
    message: &str,
) -> Result<SyncResult, String> {
    mark_sync_job(connection, account_id, job_id, "failed", Some(message))?;
    Ok(failed_result(account_id, job_id, message))
}

fn spawn_collector(
    paths: &AppPaths,
    identity: &RunningSyncIdentity,
    invocation: CollectorInvocation,
) -> Result<OwnedCollectorProcess, String> {
    let log_path = paths.phase_log_path(
        identity.account_id,
        identity.job_id,
        identity.phase,
        &identity.phase_attempt_id,
    )?;
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let collector_identity_path = paths.phase_collector_identity_path(
        identity.account_id,
        identity.job_id,
        identity.phase,
        &identity.phase_attempt_id,
    )?;
    let expectation = CollectorProcessExpectation::for_invocation(
        identity.account_id,
        identity.job_id,
        identity.phase,
        &identity.phase_attempt_id,
        &invocation,
    )?;
    OwnedCollectorProcess::launch_suspended(&invocation, &collector_identity_path, &expectation)
}

fn spawn_login_task_collector(
    paths: &AppPaths,
    target: &LoginTarget,
    attempt_id: &str,
    invocation: CollectorInvocation,
) -> Result<OwnedCollectorProcess, String> {
    let log_path = paths.login_task_log_path(attempt_id)?;
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let expectation = LoginCollectorProcessExpectation::for_invocation(
        attempt_id,
        &target.profile_key,
        target.account_id,
        &invocation,
    )?;
    OwnedCollectorProcess::launch_login_suspended(
        &invocation,
        &paths.login_task_collector_identity_path(attempt_id)?,
        &expectation,
    )
}

fn terminate_recorded_login_task_browser_with<F>(
    paths: &AppPaths,
    profile_key: &str,
    attempt_id: &str,
    mut terminate: F,
) -> Result<(), String>
where
    F: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let path = paths.login_task_browser_pid_path(attempt_id)?;
    let Ok(contents) = fs::read_to_string(path) else {
        return Ok(());
    };
    let Ok(record) = serde_json::from_str::<BrowserProcessRecordV2>(&contents) else {
        return Ok(());
    };
    let account_root = paths.account_browser_root(profile_key)?;
    let expected_profile = douyin_history_archive::collector::browser_state::browser_profile_path(
        &account_root,
        record.browser_id,
    );
    if !browser_record_matches_expected_profile(&record, &expected_profile) {
        return Ok(());
    }
    terminate(&record, &expected_profile)
}

async fn cancel_running_login_task(
    state: &AppState,
    expected_account_id: Option<i64>,
) -> Result<(), String> {
    let running = {
        let running = state
            .running_login
            .lock()
            .map_err(|_| "登录状态锁已损坏".to_string())?;
        running
            .as_ref()
            .filter(|running| {
                expected_account_id
                    .map(|account_id| running.account_id == Some(account_id))
                    .unwrap_or(true)
            })
            .map(|running| {
                (
                    running.paths.clone(),
                    running.profile_key.clone(),
                    running.attempt_id.clone(),
                    running.cancel_path.clone(),
                    Arc::clone(&running.child),
                )
            })
    };
    let Some((paths, profile_key, attempt_id, cancel_path, child)) = running else {
        return Ok(());
    };
    tauri::async_runtime::spawn_blocking(move || {
        cancel_login_task_process_with(
            &paths,
            &profile_key,
            &attempt_id,
            &cancel_path,
            &child,
            Duration::from_secs(2),
            terminate_recorded_browser_v2,
        )
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(())
}

fn cancel_login_task_process_with<F>(
    paths: &AppPaths,
    profile_key: &str,
    attempt_id: &str,
    cancel_path: &Path,
    child: &Arc<Mutex<OwnedCollectorProcess>>,
    wait: Duration,
    terminate: F,
) -> Result<(), String>
where
    F: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let mut first_error = fs::write(cancel_path, b"cancel\n")
        .map_err(|error| error.to_string())
        .err();
    if first_error.is_none() {
        thread::sleep(wait);
    }
    let collector_exited = match child.lock() {
        Ok(child) => match child.try_wait() {
            Ok(status) => status.is_some(),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error.to_string());
                }
                false
            }
        },
        Err(_) => {
            if first_error.is_none() {
                first_error = Some("登录子进程锁已损坏".to_string());
            }
            false
        }
    };
    let result = finish_login_cancellation_with(
        collector_exited,
        || {
            let child = child.lock().map_err(|_| "登录子进程锁已损坏".to_string())?;
            if child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_none()
            {
                child.kill().map_err(|error| error.to_string())?;
            }
            Ok(())
        },
        || terminate_recorded_login_task_browser_with(paths, profile_key, attempt_id, terminate),
        || {
            cleanup_durable_login_files(paths, attempt_id);
            Ok(())
        },
    );
    match first_error {
        Some(error) => Err(error),
        None => result,
    }
}

#[cfg(test)]
fn terminate_recorded_login_browser_with<F>(
    paths: &AppPaths,
    account_id: i64,
    profile_key: &str,
    attempt_id: &str,
    mut terminate: F,
) -> Result<(), String>
where
    F: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let path = paths.login_browser_pid_path(account_id, attempt_id)?;
    let Ok(contents) = fs::read_to_string(path) else {
        return Ok(());
    };
    let Ok(record) = serde_json::from_str::<BrowserProcessRecordV2>(&contents) else {
        return Ok(());
    };
    let account_root = paths.account_browser_root(profile_key)?;
    let expected_profile = douyin_history_archive::collector::browser_state::browser_profile_path(
        &account_root,
        record.browser_id,
    );
    if !douyin_history_archive::collector::browser_process::browser_record_matches_expected_profile(
        &record,
        &expected_profile,
    ) {
        return Ok(());
    }
    terminate(&record, &expected_profile)
}

fn finish_login_cancellation_with<S, T, C>(
    collector_exited: bool,
    stop_collector: S,
    terminate_browser: T,
    cleanup: C,
) -> Result<(), String>
where
    S: FnOnce() -> Result<(), String>,
    T: FnOnce() -> Result<(), String>,
    C: FnOnce() -> Result<(), String>,
{
    let mut first_error = None;
    if !collector_exited {
        if let Err(error) = stop_collector() {
            first_error = Some(error);
        }
    }
    if let Err(error) = terminate_browser() {
        if first_error.is_none() {
            first_error = Some(error);
        }
    }
    if let Err(error) = cleanup() {
        if first_error.is_none() {
            first_error = Some(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
fn cancel_login_process_with<F>(
    paths: &AppPaths,
    account_id: i64,
    profile_key: &str,
    attempt_id: &str,
    cancel_path: &Path,
    child: &Arc<Mutex<Child>>,
    wait: Duration,
    terminate: F,
) -> Result<(), String>
where
    F: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let mut first_error = fs::write(cancel_path, b"cancel\n")
        .map_err(|error| error.to_string())
        .err();
    if first_error.is_none() {
        thread::sleep(wait);
    }
    let collector_exited = match child.lock() {
        Ok(mut child) => match child.try_wait() {
            Ok(status) => status.is_some(),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error.to_string());
                }
                false
            }
        },
        Err(_) => {
            if first_error.is_none() {
                first_error = Some("登录子进程锁已损坏".to_string());
            }
            false
        }
    };
    let result = finish_login_cancellation_with(
        collector_exited,
        || {
            let mut child = child.lock().map_err(|_| "登录子进程锁已损坏".to_string())?;
            if child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_none()
            {
                child.kill().map_err(|error| error.to_string())?;
            }
            Ok(())
        },
        || {
            terminate_recorded_login_browser_with(
                paths,
                account_id,
                profile_key,
                attempt_id,
                terminate,
            )
        },
        || douyin_history_archive::cleanup_login_attempt_files(paths, account_id, attempt_id),
    );
    match first_error {
        Some(error) => Err(error),
        None => result,
    }
}

fn terminate_recorded_browser_v2(
    record: &BrowserProcessRecordV2,
    expected_profile: &Path,
) -> Result<(), String> {
    let Some(process) = query_browser_process(record.pid) else {
        return Ok(());
    };
    let identity = BrowserProcessIdentity {
        pid: record.pid,
        creation_time_100ns: query_process_creation_time_100ns(record.pid)
            .ok_or_else(|| format!("浏览器进程 {} 创建时间不可用", record.pid))?,
        executable_path: process.executable_path,
        command_line: split_windows_command_line(&process.command_line),
    };
    if !browser_identity_matches(record, &identity, expected_profile) {
        return Err(format!("浏览器进程 {} 身份不匹配", record.pid));
    }
    terminate_process_tree(record.pid)
}

fn terminate_preflighted_browser_v2(
    record: &BrowserProcessRecordV2,
    _expected_profile: &Path,
    token: UninstallProcessToken,
) -> Result<(), String> {
    terminate_preflighted_process(record.pid, token)
}

fn preflight_recorded_browser_v2_strict(
    record: &BrowserProcessRecordV2,
    expected_profile: &Path,
) -> Result<Option<UninstallProcessToken>, String> {
    let expected_creation = record
        .creation_time_100ns
        .parse::<u64>()
        .map_err(|_| "uninstall_runtime_browser_identity_invalid".to_string())?;
    let Some(token) = open_uninstall_process_token(record.pid, expected_creation)? else {
        return Ok(None);
    };
    let process = match query_browser_process(record.pid) {
        Some(process) => process,
        None if token.has_exited()? => return Ok(None),
        None => return Err("uninstall_runtime_browser_identity_query_failed".to_string()),
    };
    let identity = BrowserProcessIdentity {
        pid: record.pid,
        creation_time_100ns: expected_creation.to_string(),
        executable_path: process.executable_path,
        command_line: split_windows_command_line(&process.command_line),
    };
    if !browser_identity_matches(record, &identity, expected_profile) {
        return Err("uninstall_runtime_browser_identity_mismatch".to_string());
    }
    Ok(Some(token))
}

fn split_windows_command_line(value: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut characters = value.chars().peekable();
    let mut quoted = false;
    while let Some(character) = characters.next() {
        if character.is_whitespace() && !quoted {
            if !current.is_empty() {
                arguments.push(std::mem::take(&mut current));
            }
            continue;
        }
        if character == '"' {
            quoted = !quoted;
            continue;
        }
        if character == '\\' {
            let mut slashes = 1;
            while characters.peek() == Some(&'\\') {
                characters.next();
                slashes += 1;
            }
            if characters.peek() == Some(&'"') {
                current.push_str(&"\\".repeat(slashes / 2));
                if slashes % 2 == 0 {
                    characters.next();
                    quoted = !quoted;
                } else {
                    characters.next();
                    current.push('"');
                }
            } else {
                current.push_str(&"\\".repeat(slashes));
            }
            continue;
        }
        current.push(character);
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    arguments
}

#[cfg(target_os = "windows")]
fn query_process_creation_time_100ns(pid: u32) -> Option<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exited: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    unsafe { CloseHandle(process) };
    if ok == 0 {
        return None;
    }
    Some((((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64).to_string())
}

#[cfg(not(target_os = "windows"))]
fn query_process_creation_time_100ns(_pid: u32) -> Option<String> {
    None
}

fn force_cleanup_running_login(state: &AppState) {
    let running = state.running_login.lock().ok().and_then(|running| {
        running.as_ref().map(|running| {
            (
                running.paths.clone(),
                running.attempt_id.clone(),
                Arc::clone(&running.child),
            )
        })
    });
    let Some((paths, attempt_id, child)) = running else {
        return;
    };
    if let Ok(child) = child.lock() {
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
        }
    }
    cleanup_durable_login_files(&paths, &attempt_id);
}

#[cfg(test)]
fn wait_for_child(
    child: &Arc<Mutex<OwnedCollectorProcess>>,
) -> Result<std::process::ExitStatus, String> {
    loop {
        let status = child
            .lock()
            .map_err(|_| "同步子进程锁已损坏".to_string())?
            .try_wait()
            .map_err(|error| error.to_string())?;
        if let Some(status) = status {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_phase_child_with(
    paths: &AppPaths,
    identity: &RunningSyncIdentity,
    child: &Arc<Mutex<OwnedCollectorProcess>>,
    cleanup_grace: Duration,
) -> Result<(std::process::ExitStatus, Option<String>), String> {
    let recovery_deadline = if identity.phase == SyncPhase::RecoveringLogin {
        let orchestration = read_orchestration(paths, identity.account_id, identity.job_id)?;
        validate_running_orchestration(&orchestration, identity)?;
        orchestration.deadline_at_ms
    } else {
        None
    };
    loop {
        if let Some(status) = child
            .lock()
            .map_err(|_| "同步子进程锁已损坏".to_string())?
            .try_wait()
            .map_err(|error| error.to_string())?
        {
            return Ok((status, None));
        }
        if recovery_deadline.is_some_and(|deadline| Utc::now().timestamp_millis() >= deadline) {
            terminate_recorded_sync_browser_with(paths, identity, terminate_recorded_browser_v2);
            let cleanup_deadline = Instant::now() + cleanup_grace;
            loop {
                if let Some(status) = child
                    .lock()
                    .map_err(|_| "同步子进程锁已损坏".to_string())?
                    .try_wait()
                    .map_err(|error| error.to_string())?
                {
                    return Ok((status, Some("login_recovery_failed".to_string())));
                }
                if Instant::now() >= cleanup_deadline {
                    let child = child.lock().map_err(|_| "同步子进程锁已损坏".to_string())?;
                    child.kill()?;
                    let status = child.wait()?;
                    terminate_recorded_sync_browser_with(
                        paths,
                        identity,
                        terminate_recorded_browser_v2,
                    );
                    return Ok((status, Some("login_recovery_failed".to_string())));
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[derive(Clone)]
struct RunningSyncContext {
    profile_key: String,
    executable: PathBuf,
}

fn spawn_sync_phase_monitor(
    app: AppHandle,
    paths: AppPaths,
    identity: RunningSyncIdentity,
    child: Arc<Mutex<OwnedCollectorProcess>>,
) -> Result<(), String> {
    let registration = app.state::<AppState>().background_tasks.register()?;
    let wait_paths = paths.clone();
    let wait_identity = identity.clone();
    tauri::async_runtime::spawn(async move {
        let _registration = registration;
        let completion = tauri::async_runtime::spawn_blocking(move || {
            wait_for_phase_child_with(&wait_paths, &wait_identity, &child, Duration::from_secs(2))
        })
        .await
        .ok()
        .and_then(Result::ok);
        let process_succeeded = completion
            .as_ref()
            .is_some_and(|(status, _)| status.success());
        let forced_error = completion.and_then(|(_, forced_error)| forced_error);
        let _ =
            handle_live_phase_completion(&app, &paths, &identity, process_succeeded, forced_error);
        if let Some(worker) = app.state::<AppState>().sync_worker.get() {
            worker.wake();
        }
    });
    Ok(())
}

fn handle_live_phase_completion(
    app: &AppHandle,
    paths: &AppPaths,
    identity: &RunningSyncIdentity,
    process_succeeded: bool,
    forced_error: Option<String>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let context = running_sync_context(state.inner(), identity)?;
    if let Some(error) = forced_error {
        let orchestration = read_orchestration(paths, identity.account_id, identity.job_id)?;
        let cancelled = orchestration.cancel_requested_at.is_some()
            || paths
                .cancel_path(identity.account_id, identity.job_id)
                .exists();
        release_finished_sync_phase(state.inner(), identity)?;
        terminate_recorded_sync_browser_with(paths, identity, terminate_recorded_browser_v2);
        cleanup_phase_files(paths, identity);
        return if cancelled {
            finish_live_sync_cancelled(
                state.inner(),
                paths,
                identity.account_id,
                identity.job_id,
                "同步已取消",
            )
        } else {
            finish_phase_transition_error(
                state.inner(),
                paths,
                identity.account_id,
                identity.job_id,
                &error,
            )
        };
    }
    let observed = match observe_phase_completion(paths, identity, process_succeeded) {
        Ok(observed) => observed,
        Err(error) => {
            release_finished_sync_phase(state.inner(), identity)?;
            terminate_recorded_sync_browser_with(paths, identity, terminate_recorded_browser_v2);
            cleanup_phase_files(paths, identity);
            return finish_live_sync_failure(
                state.inner(),
                paths,
                identity.account_id,
                identity.job_id,
                "collector_interrupted",
                &error,
            );
        }
    };
    if observed.decision == PhaseCompletionDecision::StartRetry {
        let current = read_orchestration(paths, identity.account_id, identity.job_id)?;
        let recovered_identity = observed
            .recovery_identity
            .clone()
            .ok_or_else(|| "login_recovery_failed".to_string())?;
        if let Err(error) =
            complete_hidden_recovery(paths, &context.profile_key, &current, recovered_identity)
        {
            release_finished_sync_phase(state.inner(), identity)?;
            terminate_recorded_sync_browser_with(paths, identity, terminate_recorded_browser_v2);
            cleanup_phase_files(paths, identity);
            return finish_phase_transition_error(
                state.inner(),
                paths,
                identity.account_id,
                identity.job_id,
                &error,
            );
        }
    }
    release_finished_sync_phase(state.inner(), identity)?;
    if observed.decision != PhaseCompletionDecision::StartRetry {
        terminate_recorded_sync_browser_with(paths, identity, terminate_recorded_browser_v2);
    }

    match observed.decision {
        PhaseCompletionDecision::Consume => {
            let gate = {
                let running = state
                    .running
                    .lock()
                    .map_err(|_| "同步状态锁已损坏".to_string())?;
                if !running
                    .as_ref()
                    .is_some_and(|task| task.identity() == *identity)
                {
                    return Err("sync_orchestration_mismatch".to_string());
                }
                linearize_sync_finalization(paths, identity)
            };
            let registry = state
                .sync_registry
                .get()
                .ok_or_else(|| "sync_registry_not_initialized".to_string())?;
            let result =
                finalize_sync_with_gate(paths, registry, identity, process_succeeded, gate);
            cleanup_phase_files(paths, identity);
            cleanup_live_sync_task(state.inner(), paths, identity.account_id, identity.job_id);
            result.map(|_| ())
        }
        PhaseCompletionDecision::Cancel => {
            cleanup_phase_files(paths, identity);
            finish_live_sync_cancelled(
                state.inner(),
                paths,
                identity.account_id,
                identity.job_id,
                "同步已取消",
            )
        }
        PhaseCompletionDecision::FailInterrupted => {
            let (error_code, message) =
                terminal_phase_error(identity.phase, observed.diagnostic.as_ref());
            cleanup_phase_files(paths, identity);
            finish_live_sync_failure(
                state.inner(),
                paths,
                identity.account_id,
                identity.job_id,
                &error_code,
                &message,
            )
        }
        PhaseCompletionDecision::StartRecovery => {
            start_live_recovery_phase(app, paths, identity, context)
        }
        PhaseCompletionDecision::StartRetry => {
            start_live_retry_phase(app, paths, identity, context)
        }
    }
}

fn running_sync_context(
    state: &AppState,
    identity: &RunningSyncIdentity,
) -> Result<RunningSyncContext, String> {
    let running = state
        .running
        .lock()
        .map_err(|_| "同步状态锁已损坏".to_string())?;
    let task = running
        .as_ref()
        .filter(|task| task.identity() == *identity)
        .ok_or_else(|| "sync_orchestration_mismatch".to_string())?;
    Ok(RunningSyncContext {
        profile_key: task.profile_key.clone(),
        executable: task.executable.clone(),
    })
}

fn release_finished_sync_phase(
    state: &AppState,
    identity: &RunningSyncIdentity,
) -> Result<RunningSyncContext, String> {
    let mut running = state
        .running
        .lock()
        .map_err(|_| "同步状态锁已损坏".to_string())?;
    let task = running
        .as_mut()
        .filter(|task| task.identity() == *identity)
        .ok_or_else(|| "sync_orchestration_mismatch".to_string())?;
    task.child.take();
    Ok(RunningSyncContext {
        profile_key: task.profile_key.clone(),
        executable: task.executable.clone(),
    })
}

fn start_live_recovery_phase(
    app: &AppHandle,
    paths: &AppPaths,
    completed: &RunningSyncIdentity,
    context: RunningSyncContext,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let current = read_orchestration(paths, completed.account_id, completed.job_id)?;
    validate_running_orchestration(&current, completed)?;
    let now_ms = Utc::now().timestamp_millis();
    let next_attempt_id = Uuid::new_v4().simple().to_string();
    let deadline_at_ms = now_ms.saturating_add(30_000);
    let preparing = match prepare_next_phase(
        &current,
        SyncPhase::RecoveringLogin,
        &next_attempt_id,
        Some(deadline_at_ms),
        now_ms,
        &now_iso(),
    ) {
        Ok(preparing) => preparing,
        Err(error) => {
            cleanup_phase_files(paths, completed);
            return finish_phase_transition_error(
                state.inner(),
                paths,
                completed.account_id,
                completed.job_id,
                &error,
            );
        }
    };
    let invocation = match build_recover_login_invocation(
        paths,
        context.executable,
        completed.account_id,
        &context.profile_key,
        completed.job_id,
        &next_attempt_id,
        deadline_at_ms,
    ) {
        Ok(invocation) => invocation,
        Err(error) => {
            cleanup_phase_files(paths, completed);
            return finish_phase_transition_error(
                state.inner(),
                paths,
                completed.account_id,
                completed.job_id,
                &error,
            );
        }
    };
    match launch_next_phase_with(
        &state.running,
        paths,
        &current,
        &preparing,
        invocation,
        |_, _| Ok(()),
    ) {
        Ok((identity, child)) => {
            cleanup_phase_files(paths, completed);
            if let Err(error) =
                spawn_sync_phase_monitor(app.clone(), paths.clone(), identity.clone(), child)
            {
                let _ = request_sync_cancellation(paths, &identity, &now_iso());
                terminate_recorded_sync_browser_with(
                    paths,
                    &identity,
                    terminate_recorded_browser_v2,
                );
                cleanup_phase_files(paths, &identity);
                finish_phase_transition_error(
                    state.inner(),
                    paths,
                    completed.account_id,
                    completed.job_id,
                    &error,
                )?;
                return Err(error);
            }
            Ok(())
        }
        Err(error) => {
            cleanup_phase_files(paths, completed);
            finish_phase_transition_error(
                state.inner(),
                paths,
                completed.account_id,
                completed.job_id,
                &error,
            )
        }
    }
}

fn start_live_retry_phase(
    app: &AppHandle,
    paths: &AppPaths,
    completed: &RunningSyncIdentity,
    context: RunningSyncContext,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let current = read_orchestration(paths, completed.account_id, completed.job_id)?;
    validate_running_orchestration(&current, completed)?;
    let next_attempt_id = Uuid::new_v4().simple().to_string();
    let now_ms = Utc::now().timestamp_millis();
    let preparing = match prepare_next_phase(
        &current,
        SyncPhase::RetryingAfterLogin,
        &next_attempt_id,
        None,
        now_ms,
        &now_iso(),
    ) {
        Ok(preparing) => preparing,
        Err(error) => {
            cleanup_phase_files(paths, completed);
            return finish_phase_transition_error(
                state.inner(),
                paths,
                completed.account_id,
                completed.job_id,
                &error,
            );
        }
    };
    let invocation = match (|| {
        let connection = open_database(&paths.database)?;
        assert_schema_v5(&connection)?;
        let job = QueueStore::get_stored_job(&connection, completed.job_id)?;
        if job.snapshot.task_key.account_id != completed.account_id
            || job.snapshot.status != douyin_history_archive::sync::SyncJobStatusV1::Running
        {
            return Err("sync_job_mismatch".to_string());
        }
        let invocation = build_collect_invocation_for_job(
            paths,
            context.executable,
            &job,
            SyncPhase::RetryingAfterLogin,
            &next_attempt_id,
        )?;
        prepare_collect_input_files(
            paths,
            &connection,
            &job,
            SyncPhase::RetryingAfterLogin,
            &next_attempt_id,
        )?;
        Ok(invocation)
    })() {
        Ok(invocation) => invocation,
        Err(error) => {
            cleanup_phase_files(paths, completed);
            return finish_phase_transition_error(
                state.inner(),
                paths,
                completed.account_id,
                completed.job_id,
                &error,
            );
        }
    };
    let next_identity = RunningSyncIdentity {
        account_id: completed.account_id,
        job_id: completed.job_id,
        phase: SyncPhase::RetryingAfterLogin,
        phase_attempt_id: next_attempt_id.clone(),
    };
    match launch_next_phase_with(
        &state.running,
        paths,
        &current,
        &preparing,
        invocation,
        |_, _| Ok(()),
    ) {
        Ok((identity, child)) => {
            cleanup_phase_files(paths, completed);
            if let Err(error) =
                spawn_sync_phase_monitor(app.clone(), paths.clone(), identity.clone(), child)
            {
                let _ = request_sync_cancellation(paths, &identity, &now_iso());
                terminate_recorded_sync_browser_with(
                    paths,
                    &identity,
                    terminate_recorded_browser_v2,
                );
                cleanup_phase_files(paths, &identity);
                finish_phase_transition_error(
                    state.inner(),
                    paths,
                    completed.account_id,
                    completed.job_id,
                    &error,
                )?;
                return Err(error);
            }
            Ok(())
        }
        Err(error) => {
            cleanup_phase_files(paths, completed);
            cleanup_phase_files(paths, &next_identity);
            finish_phase_transition_error(
                state.inner(),
                paths,
                completed.account_id,
                completed.job_id,
                &error,
            )
        }
    }
}

fn finish_phase_transition_error(
    state: &AppState,
    paths: &AppPaths,
    account_id: i64,
    job_id: i64,
    error: &str,
) -> Result<(), String> {
    if error == "sync_cancel_requested" {
        return finish_live_sync_cancelled(state, paths, account_id, job_id, "同步已取消");
    }
    let message = match error {
        "login_recovery_failed" => "自动恢复登录失败，请重新登录。".to_string(),
        "account_identity_conflict" => "自动恢复检测到账号身份冲突。".to_string(),
        _ => douyin_history_archive::collector::errors::describe_collector_error(error).message,
    };
    finish_live_sync_failure(state, paths, account_id, job_id, error, &message)
}

fn persist_sync_terminal_outcome(
    paths: &AppPaths,
    account_id: i64,
    job_id: i64,
    outcome: SyncTerminalOutcome,
) -> Result<(), String> {
    let mut connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    finish_sync_with_account_state(
        &mut connection,
        SyncJobIdentity {
            account_id,
            sync_job_id: job_id,
        },
        outcome,
    )
}

fn finish_live_sync_cancelled(
    state: &AppState,
    paths: &AppPaths,
    account_id: i64,
    job_id: i64,
    message: &str,
) -> Result<(), String> {
    let mut running = state
        .running
        .lock()
        .map_err(|_| "同步状态锁已损坏".to_string())?;
    if !running
        .as_ref()
        .is_some_and(|task| task.account_id == account_id && task.job_id == job_id)
    {
        return Err("sync_orchestration_mismatch".to_string());
    }
    persist_sync_terminal_outcome(
        paths,
        account_id,
        job_id,
        SyncTerminalOutcome::Cancelled {
            error_message: Some(message.to_string()),
        },
    )?;
    cleanup_live_sync_files(paths, account_id, job_id);
    *running = None;
    Ok(())
}

fn finish_live_sync_failure(
    state: &AppState,
    paths: &AppPaths,
    account_id: i64,
    job_id: i64,
    error_code: &str,
    message: &str,
) -> Result<(), String> {
    let mut running = state
        .running
        .lock()
        .map_err(|_| "同步状态锁已损坏".to_string())?;
    if !running
        .as_ref()
        .is_some_and(|task| task.account_id == account_id && task.job_id == job_id)
    {
        return Err("sync_orchestration_mismatch".to_string());
    }
    let cancellation_persisted = read_orchestration(paths, account_id, job_id)
        .ok()
        .is_some_and(|orchestration| orchestration.cancel_requested_at.is_some())
        || paths.cancel_path(account_id, job_id).exists();
    let outcome = if cancellation_persisted {
        SyncTerminalOutcome::Cancelled {
            error_message: Some("同步已取消".to_string()),
        }
    } else {
        SyncTerminalOutcome::Failed {
            error_code: error_code.to_string(),
            error_message: Some(message.to_string()),
        }
    };
    persist_sync_terminal_outcome(paths, account_id, job_id, outcome)?;
    cleanup_live_sync_files(paths, account_id, job_id);
    *running = None;
    Ok(())
}

fn cleanup_live_sync_task(state: &AppState, paths: &AppPaths, account_id: i64, job_id: i64) {
    cleanup_live_sync_files(paths, account_id, job_id);
    if let Ok(mut running) = state.running.lock() {
        if running
            .as_ref()
            .is_some_and(|task| task.account_id == account_id && task.job_id == job_id)
        {
            *running = None;
        }
    }
}

fn cleanup_live_sync_files(paths: &AppPaths, account_id: i64, job_id: i64) {
    if let Ok(orchestration) = read_orchestration(paths, account_id, job_id) {
        let identity = RunningSyncIdentity {
            account_id,
            job_id,
            phase: orchestration.phase,
            phase_attempt_id: orchestration.phase_attempt_id,
        };
        terminate_recorded_sync_browser_with(paths, &identity, terminate_recorded_browser_v2);
        cleanup_task_files(paths, &identity);
    } else {
        let _ = fs::remove_file(paths.cancel_path(account_id, job_id));
        let _ = fs::remove_file(paths.orchestration_path(account_id, job_id));
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

fn read_sync_phase_payload<T: DeserializeOwned>(
    path: &Path,
    orchestration: &SyncOrchestrationV1,
) -> Result<T, String> {
    let envelope: SyncPhaseEnvelopeV1<T> = read_json(path)?;
    envelope.validate_context(
        orchestration.account_id,
        orchestration.sync_job_id,
        orchestration.phase,
        &orchestration.phase_attempt_id,
    )?;
    Ok(envelope.payload)
}

fn validate_running_orchestration(
    orchestration: &SyncOrchestrationV1,
    identity: &RunningSyncIdentity,
) -> Result<(), String> {
    if orchestration.account_id != identity.account_id
        || orchestration.sync_job_id != identity.job_id
        || orchestration.phase != identity.phase
        || orchestration.phase_attempt_id != identity.phase_attempt_id
    {
        return Err("sync_orchestration_mismatch".to_string());
    }
    Ok(())
}

fn read_log_tail(path: &Path) -> String {
    let Ok(mut file) = File::open(path) else {
        return "同步进程异常退出".to_string();
    };
    let mut raw = String::new();
    let _ = file.read_to_string(&mut raw);
    raw.chars()
        .rev()
        .take(2_000)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

fn cleanup_task_files(paths: &AppPaths, identity: &RunningSyncIdentity) {
    cleanup_phase_files(paths, identity);
    if read_orchestration(paths, identity.account_id, identity.job_id)
        .ok()
        .is_some_and(|orchestration| {
            validate_running_orchestration(&orchestration, identity).is_ok()
        })
    {
        let _ = fs::remove_file(paths.cancel_path(identity.account_id, identity.job_id));
        let _ = fs::remove_file(paths.orchestration_path(identity.account_id, identity.job_id));
    }
}

fn cleanup_phase_files(paths: &AppPaths, identity: &RunningSyncIdentity) {
    let mut candidates = Vec::new();
    for path in [
        paths.phase_result_path(
            identity.account_id,
            identity.job_id,
            identity.phase,
            &identity.phase_attempt_id,
        ),
        paths.phase_progress_path(
            identity.account_id,
            identity.job_id,
            identity.phase,
            &identity.phase_attempt_id,
        ),
        paths.phase_archive_known_ids_path(
            identity.account_id,
            identity.job_id,
            identity.phase,
            &identity.phase_attempt_id,
        ),
        paths.phase_browser_pid_path(
            identity.account_id,
            identity.job_id,
            identity.phase,
            &identity.phase_attempt_id,
        ),
        paths.phase_collector_identity_path(
            identity.account_id,
            identity.job_id,
            identity.phase,
            &identity.phase_attempt_id,
        ),
        paths.phase_diagnostic_path(
            identity.account_id,
            identity.job_id,
            identity.phase,
            &identity.phase_attempt_id,
        ),
        paths.phase_log_path(
            identity.account_id,
            identity.job_id,
            identity.phase,
            &identity.phase_attempt_id,
        ),
    ] {
        if let Ok(path) = path {
            candidates.push(path);
        }
    }
    for path in candidates {
        let _ = fs::remove_file(path);
    }
}

fn cleanup_legacy_task_files(paths: &AppPaths, account_id: i64, job_id: i64) {
    for path in [
        paths.result_path(account_id, job_id),
        paths.progress_path(account_id, job_id),
        paths.cancel_path(account_id, job_id),
        paths.browser_pid_path(account_id, job_id),
        paths.diagnostic_path(account_id, job_id),
        paths.log_path(account_id, job_id),
    ] {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
fn cleanup_recovered_job_with<F>(paths: &AppPaths, account_id: i64, job_id: i64, mut terminate: F)
where
    F: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    if let Ok(orchestration) = read_orchestration(paths, account_id, job_id) {
        let identity = RunningSyncIdentity {
            account_id,
            job_id,
            phase: orchestration.phase,
            phase_attempt_id: orchestration.phase_attempt_id,
        };
        terminate_recorded_sync_browser_with(paths, &identity, &mut terminate);
    } else {
        terminate_legacy_sync_browser_with(paths, account_id, job_id, &mut terminate);
    }
    cleanup_recovered_job_files(paths, account_id, job_id);
}

fn cleanup_recovered_job_files(paths: &AppPaths, account_id: i64, job_id: i64) {
    if let Ok(orchestration) = read_orchestration(paths, account_id, job_id) {
        cleanup_task_files(
            paths,
            &RunningSyncIdentity {
                account_id,
                job_id,
                phase: orchestration.phase,
                phase_attempt_id: orchestration.phase_attempt_id,
            },
        );
    } else {
        cleanup_legacy_task_files(paths, account_id, job_id);
    }
    remove_regular_files_with_prefix(&paths.temp, &format!("account-{account_id}-sync-{job_id}-"));
    remove_regular_files_with_prefix(
        &paths.diagnostics.join(format!("account-{account_id}")),
        &format!("sync-{job_id}-"),
    );
    remove_regular_files_with_prefix(
        &paths.logs.join(format!("account-{account_id}")),
        &format!("sync-{job_id}-"),
    );
    let _ = fs::remove_file(paths.cancel_path(account_id, job_id));
    let _ = fs::remove_file(paths.orchestration_path(account_id, job_id));
}

fn cleanup_recovered_job_files_strict(
    paths: &AppPaths,
    account_id: i64,
    job_id: i64,
) -> Result<(), String> {
    remove_matching_regular_files_strict(
        &paths.temp,
        &format!("account-{account_id}-sync-{job_id}-"),
    )?;
    remove_matching_regular_files_strict(
        &paths.diagnostics.join(format!("account-{account_id}")),
        &format!("sync-{job_id}-"),
    )?;
    remove_matching_regular_files_strict(
        &paths.logs.join(format!("account-{account_id}")),
        &format!("sync-{job_id}-"),
    )?;
    for path in [
        paths.result_path(account_id, job_id),
        paths.progress_path(account_id, job_id),
        paths.cancel_path(account_id, job_id),
        paths.browser_pid_path(account_id, job_id),
        paths.diagnostic_path(account_id, job_id),
        paths.log_path(account_id, job_id),
        paths.orchestration_path(account_id, job_id),
    ] {
        remove_file_if_exists_strict(&path)?;
    }
    assert_no_matching_regular_files(&paths.temp, &format!("account-{account_id}-sync-{job_id}-"))?;
    assert_no_matching_regular_files(
        &paths.diagnostics.join(format!("account-{account_id}")),
        &format!("sync-{job_id}-"),
    )?;
    assert_no_matching_regular_files(
        &paths.logs.join(format!("account-{account_id}")),
        &format!("sync-{job_id}-"),
    )?;
    Ok(())
}

fn remove_file_if_exists_strict(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("uninstall_runtime_file_cleanup_failed".to_string()),
    }
}

fn remove_matching_regular_files_strict(directory: &Path, prefix: &str) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("uninstall_runtime_file_cleanup_failed".to_string()),
    };
    for entry in entries {
        let entry = entry.map_err(|_| "uninstall_runtime_file_cleanup_failed".to_string())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "uninstall_runtime_file_cleanup_failed".to_string())?;
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| "uninstall_runtime_file_cleanup_failed".to_string())?;
        if name.starts_with(prefix) {
            if !file_type.is_file() {
                return Err("uninstall_runtime_file_cleanup_failed".to_string());
            }
            remove_file_if_exists_strict(&entry.path())?;
        }
    }
    Ok(())
}

fn assert_no_matching_regular_files(directory: &Path, prefix: &str) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("uninstall_runtime_file_cleanup_failed".to_string()),
    };
    for entry in entries {
        let entry = entry.map_err(|_| "uninstall_runtime_file_cleanup_failed".to_string())?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(prefix))
        {
            return Err("uninstall_runtime_file_cleanup_failed".to_string());
        }
    }
    Ok(())
}

fn cleanup_schema_upgrade_job(
    paths: &AppPaths,
    identity: &LegacyRunningJobIdentity,
) -> Result<(), String> {
    if let Ok(orchestration) = read_orchestration(paths, identity.account_id, identity.job_id) {
        let running = RunningSyncIdentity {
            account_id: identity.account_id,
            job_id: identity.job_id,
            phase: orchestration.phase,
            phase_attempt_id: orchestration.phase_attempt_id,
        };
        terminate_recorded_sync_collector(paths, &running);
        terminate_recorded_sync_browser_with(paths, &running, terminate_recorded_browser_v2);
    } else {
        terminate_legacy_sync_browser_with(
            paths,
            identity.account_id,
            identity.job_id,
            terminate_recorded_browser_v2,
        );
    }
    cleanup_recovered_job_files(paths, identity.account_id, identity.job_id);
    Ok(())
}

fn remove_regular_files_with_prefix(directory: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        if name.to_str().is_some_and(|name| name.starts_with(prefix)) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn terminate_legacy_sync_browser_with<F>(
    paths: &AppPaths,
    account_id: i64,
    job_id: i64,
    mut terminate: F,
) where
    F: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let Ok(contents) = fs::read_to_string(paths.browser_pid_path(account_id, job_id)) else {
        return;
    };
    let Ok(record) = serde_json::from_str::<BrowserProcessRecordV2>(&contents) else {
        return;
    };
    let Ok(expected_profile) = expected_sync_browser_profile(paths, account_id, record.browser_id)
    else {
        return;
    };
    if browser_record_matches_expected_profile(&record, &expected_profile) {
        let _ = terminate(&record, &expected_profile);
    }
}

fn terminate_recorded_sync_browser_with<F>(
    paths: &AppPaths,
    identity: &RunningSyncIdentity,
    mut terminate: F,
) where
    F: FnMut(&BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let Ok(orchestration) = read_orchestration(paths, identity.account_id, identity.job_id) else {
        return;
    };
    if validate_running_orchestration(&orchestration, identity).is_err() {
        return;
    }
    let Ok(path) = paths.phase_browser_pid_path(
        identity.account_id,
        identity.job_id,
        identity.phase,
        &identity.phase_attempt_id,
    ) else {
        return;
    };
    let Ok(record) = read_sync_phase_payload::<BrowserProcessRecordV2>(&path, &orchestration)
    else {
        return;
    };
    let Ok(expected_profile) =
        expected_sync_browser_profile(paths, identity.account_id, record.browser_id)
    else {
        return;
    };
    if browser_record_matches_expected_profile(&record, &expected_profile) {
        let _ = terminate(&record, &expected_profile);
    }
}

fn terminate_recorded_sync_collector(paths: &AppPaths, identity: &RunningSyncIdentity) {
    let Ok(orchestration) = read_orchestration(paths, identity.account_id, identity.job_id) else {
        return;
    };
    if validate_running_orchestration(&orchestration, identity).is_err() {
        return;
    }
    let Ok(path) = paths.phase_collector_identity_path(
        identity.account_id,
        identity.job_id,
        identity.phase,
        &identity.phase_attempt_id,
    ) else {
        return;
    };
    let Ok(record) = read_json::<CollectorProcessIdentityV1>(&path) else {
        return;
    };
    let expectation = CollectorProcessExpectation {
        account_id: identity.account_id,
        sync_job_id: identity.job_id,
        phase: identity.phase,
        phase_attempt_id: identity.phase_attempt_id.clone(),
        executable_path: record.executable_path.clone(),
        arguments_sha256: record.arguments_sha256.clone(),
    };
    let Some(observation) = query_collector_process(&record) else {
        return;
    };
    let _ =
        terminate_collector_if_matches(&record, &observation, &expectation, terminate_process_tree);
}

fn terminate_recorded_login_collector(
    record: &LoginCollectorProcessIdentityV1,
    expectation: &LoginCollectorProcessExpectation,
) -> Result<(), String> {
    let Some(observation) = query_process_observation(record.pid) else {
        return Ok(());
    };
    if terminate_login_collector_if_matches(
        record,
        &observation,
        expectation,
        terminate_process_tree,
    )? {
        Ok(())
    } else if observation.creation_time_100ns != record.creation_time_100ns {
        Ok(())
    } else {
        Err("login_collector_process_identity_mismatch".to_string())
    }
}

fn terminate_preflighted_login_collector(
    record: &LoginCollectorProcessIdentityV1,
    _expectation: &LoginCollectorProcessExpectation,
    token: UninstallProcessToken,
) -> Result<(), String> {
    terminate_preflighted_process(record.pid, token)
}

fn terminate_preflighted_sync_collector(
    record: &CollectorProcessIdentityV1,
    _expectation: &CollectorProcessExpectation,
    token: UninstallProcessToken,
) -> Result<(), String> {
    terminate_preflighted_process(record.pid, token)
}

fn preflight_recorded_login_collector_strict(
    record: &LoginCollectorProcessIdentityV1,
    expectation: &LoginCollectorProcessExpectation,
) -> Result<Option<UninstallProcessToken>, String> {
    let expected_creation = record
        .creation_time_100ns
        .parse::<u64>()
        .map_err(|_| "uninstall_runtime_login_collector_identity_invalid".to_string())?;
    let Some(token) = open_uninstall_process_token(record.pid, expected_creation)? else {
        return Ok(None);
    };
    let observation = match query_process_observation_with_creation(record.pid, expected_creation) {
        Some(observation) => observation,
        None if token.has_exited()? => return Ok(None),
        None => return Err("uninstall_runtime_login_collector_identity_query_failed".to_string()),
    };
    if !login_collector_process_identity_matches(record, &observation, expectation) {
        return Err("uninstall_runtime_login_collector_identity_mismatch".to_string());
    }
    Ok(Some(token))
}

fn preflight_recorded_sync_collector_strict(
    record: &CollectorProcessIdentityV1,
    expectation: &CollectorProcessExpectation,
) -> Result<Option<UninstallProcessToken>, String> {
    let expected_creation = record
        .creation_time_100ns
        .parse::<u64>()
        .map_err(|_| "uninstall_runtime_collector_identity_invalid".to_string())?;
    let Some(token) = open_uninstall_process_token(record.pid, expected_creation)? else {
        return Ok(None);
    };
    let observation = match query_process_observation_with_creation(record.pid, expected_creation) {
        Some(observation) => observation,
        None if token.has_exited()? => return Ok(None),
        None => return Err("uninstall_runtime_collector_identity_query_failed".to_string()),
    };
    if !collector_process_identity_matches(record, &observation, expectation) {
        return Err("uninstall_runtime_collector_identity_mismatch".to_string());
    }
    Ok(Some(token))
}

fn query_collector_process(
    record: &CollectorProcessIdentityV1,
) -> Option<CollectorProcessObservation> {
    query_process_observation(record.pid)
}

fn query_process_observation(pid: u32) -> Option<CollectorProcessObservation> {
    let creation_time_100ns = query_process_creation_time_100ns(pid)?;
    query_process_observation_with_creation(pid, creation_time_100ns.parse().ok()?)
}

fn query_process_observation_with_creation(
    pid: u32,
    creation_time_100ns: u64,
) -> Option<CollectorProcessObservation> {
    let process = query_browser_process(pid)?;
    let mut arguments = split_windows_command_line(&process.command_line);
    let executable_argument = arguments.first()?;
    if douyin_history_archive::collector::browser_state::canonical_windows_path(executable_argument)
        != douyin_history_archive::collector::browser_state::canonical_windows_path(
            &process.executable_path,
        )
    {
        return None;
    }
    arguments.remove(0);
    Some(CollectorProcessObservation {
        pid,
        creation_time_100ns: creation_time_100ns.to_string(),
        executable_path: process.executable_path,
        arguments,
    })
}

#[cfg(target_os = "windows")]
struct UninstallProcessToken {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(target_os = "windows")]
impl UninstallProcessToken {
    fn has_exited(&self) -> Result<bool, String> {
        use windows_sys::Win32::{
            Foundation::{WAIT_FAILED, WAIT_OBJECT_0},
            System::Threading::WaitForSingleObject,
        };
        match unsafe { WaitForSingleObject(self.handle, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            windows_sys::Win32::Foundation::WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err("process_identity_query_failed".to_string()),
            _ => Err("process_identity_query_failed".to_string()),
        }
    }

    fn wait_for_exit(&self) -> Result<(), String> {
        use windows_sys::Win32::{
            Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::WaitForSingleObject,
        };
        match unsafe { WaitForSingleObject(self.handle, 2_000) } {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err("uninstall_runtime_process_still_running".to_string()),
            WAIT_FAILED => Err("process_identity_query_failed".to_string()),
            _ => Err("process_identity_query_failed".to_string()),
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for UninstallProcessToken {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
    }
}

#[cfg(target_os = "windows")]
fn open_uninstall_process_token(
    pid: u32,
    expected_creation_time_100ns: u64,
) -> Result<Option<UninstallProcessToken>, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER},
        Storage::FileSystem::SYNCHRONIZE,
        System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let handle = unsafe { OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            Ok(None)
        } else {
            Err("process_identity_query_failed".to_string())
        };
    }
    match process_creation_time_from_handle(handle) {
        Ok(observed) if observed == expected_creation_time_100ns => {
            Ok(Some(UninstallProcessToken { handle }))
        }
        Ok(_) => {
            unsafe { CloseHandle(handle) };
            Ok(None)
        }
        Err(error) => {
            unsafe { CloseHandle(handle) };
            Err(error)
        }
    }
}

#[cfg(target_os = "windows")]
fn process_creation_time_from_handle(
    handle: windows_sys::Win32::Foundation::HANDLE,
) -> Result<u64, String> {
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::GetProcessTimes};
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exited: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err("process_identity_query_failed".to_string());
    }
    Ok(((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64)
}

#[cfg(target_os = "windows")]
fn terminate_preflighted_process(pid: u32, token: UninstallProcessToken) -> Result<(), String> {
    if token.has_exited()? {
        return Ok(());
    }
    terminate_process_tree(pid)?;
    token.wait_for_exit()
}

fn is_simple_uuid_text(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn expected_sync_browser_profile(
    paths: &AppPaths,
    account_id: i64,
    browser_id: BrowserId,
) -> Result<PathBuf, String> {
    let connection = open_database(&paths.database)?;
    let profile_key = account_profile_key(&connection, account_id)?;
    let account_root = paths.account_browser_root(&profile_key)?;
    Ok(
        douyin_history_archive::collector::browser_state::browser_profile_path(
            &account_root,
            browser_id,
        ),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowsProcessInfo {
    executable_path: Option<String>,
    command_line: Option<String>,
}

fn query_browser_process(pid: u32) -> Option<BrowserProcessInfo> {
    let script = format!(
        "$OutputEncoding=[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); \
         $p=Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}' -ErrorAction SilentlyContinue; \
         if ($null -ne $p) {{ [pscustomobject]@{{ executablePath=$p.ExecutablePath; commandLine=$p.CommandLine }} | ConvertTo-Json -Compress }}"
    );
    let output = hidden_command(Path::new("powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let info: WindowsProcessInfo = serde_json::from_str(raw.trim()).ok()?;
    Some(BrowserProcessInfo {
        executable_path: info.executable_path?,
        command_line: info.command_line?,
    })
}

fn terminate_process_tree(pid: u32) -> Result<(), String> {
    let pid = pid.to_string();
    let status = hidden_command(Path::new("taskkill.exe"))
        .args(["/PID", pid.as_str(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("无法终止浏览器进程树 {pid}"))
    }
}

fn ensure_no_running_sync(state: &State<'_, AppState>) -> Result<(), String> {
    if state
        .running
        .lock()
        .map_err(|_| "同步状态锁已损坏".to_string())?
        .is_some()
    {
        Err("已有同步任务正在运行".to_string())
    } else {
        Ok(())
    }
}

fn ensure_account_not_running(state: &AppState, account_id: i64) -> Result<(), String> {
    let sync_matches = state
        .running
        .lock()
        .map_err(|_| "同步状态锁已损坏".to_string())?
        .as_ref()
        .is_some_and(|running| running.account_id == account_id);
    let login_matches = state
        .running_login
        .lock()
        .map_err(|_| "登录状态锁已损坏".to_string())?
        .as_ref()
        .is_some_and(|running| running.account_id == Some(account_id));
    if sync_matches || login_matches {
        Err("account_operation_in_progress".to_string())
    } else {
        Ok(())
    }
}

fn begin_browser_operation(
    state: &AppState,
    kind: BrowserOperationKind,
    account_id: Option<i64>,
) -> Result<BrowserOperationLease, String> {
    state
        .operation_admission
        .begin_browser_operation(kind, account_id)
        .map_err(|error| match error.as_str() {
            "browser_operation_in_progress" => "已有登录或同步操作正在运行".to_string(),
            _ => error,
        })
}

fn read_sync_execution_envelope(
    paths: &AppPaths,
    result_path: &Path,
    orchestration: &SyncOrchestrationV1,
) -> Result<SyncExecutionEnvelopeV1, String> {
    let bytes = fs::read(result_path).map_err(|error| error.to_string())?;
    let envelope = decode_execution_envelope(&bytes)?;
    let connection = open_database(&paths.database)?;
    assert_schema_v5(&connection)?;
    let job = QueueStore::get_stored_job(&connection, orchestration.sync_job_id)?;
    if orchestration.account_id != job.snapshot.task_key.account_id
        || orchestration.options_hash != job.snapshot.task_payload_sha256
        || job.snapshot.status != douyin_history_archive::sync::SyncJobStatusV1::Running
    {
        return Err("sync_result_identity_mismatch".to_string());
    }
    validate_execution_envelope(&envelope, &job.snapshot)?;
    Ok(envelope)
}

fn failed_result(account_id: i64, job_id: i64, message: &str) -> SyncResult {
    SyncResult {
        account_id,
        job_id,
        status: "failed".to_string(),
        inserted_videos: 0,
        inserted_watch_records: 0,
        message: Some(message.to_string()),
    }
}

fn active_sync_result(
    account_id: i64,
    job_id: i64,
    status: SyncJobStatusV1,
) -> Result<SyncResult, String> {
    let status = match status {
        SyncJobStatusV1::Queued => "queued",
        SyncJobStatusV1::Running => "running",
        _ => return Err("sync_job_not_active".to_string()),
    };
    Ok(SyncResult {
        account_id,
        job_id,
        status: status.to_string(),
        inserted_videos: 0,
        inserted_watch_records: 0,
        message: None,
    })
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::{
        account_can_resume_login, automatic_backup_tick_runtime_with, begin_browser_operation,
        build_history_collect_invocation, build_login_invocation, cancel_login_process_with,
        cleanup_durable_login_attempt, cleanup_recovered_job_files, cleanup_recovered_job_with,
        cleanup_task_files, compiled_product_identity, complete_hidden_recovery, fail_started_sync,
        failed_sync_status, finalize_sync, finish_login_cancellation_with,
        finish_phase_transition_error, get_app_settings_from_state,
        initialize_automatic_backup_runtime_with, initialize_onboarding_data_root_from_state,
        join_automatic_backup_runtime, launch_next_phase_with, linearize_sync_finalization,
        list_notifications_from_state, load_sync_status_for_account,
        login_identity_observed_for_shutdown, mark_all_notifications_read_from_state,
        mark_notification_read_from_state,
        observe_phase_completion, open_database,
        prepare_app_directories, prepare_application_runtime_for_uninstall,
        prepare_application_runtime_for_uninstall_with, prepare_new_login_target, read_bootstrap,
        read_json, read_running_sync_progress, read_sync_execution_envelope,
        read_sync_phase_payload, recorded_login_task_browser_is_alive,
        recover_durable_login_attempts_with, recover_interrupted_job_with,
        recover_interrupted_job_with_disposition, recover_interrupted_sync_jobs,
        register_running_phase, request_sync_cancellation, require_account_status,
        require_normal_runtime, resolve_initial_paths_from, resolve_login_failure_code,
        resolve_notification_from_state, running_sync_identity, set_data_root_from_state,
        spawn_collector, start_sync_job, startup_recovery_status,
        status_with_running_progress, stop_automatic_backup_runtime,
        stop_trusted_processes_for_uninstall_with, sync_options_from_account_settings,
        system_notification_enabled,
        terminal_phase_error, terminate_recorded_browser_v2, terminate_recorded_login_browser_with,
        terminate_recorded_sync_browser_with, test_sync_registry,
        update_account_settings_from_state, update_global_settings_from_state,
        validate_data_root_from_state, wait_for_child, wait_for_phase_child_with,
        wake_scheduler_and_queue, write_login_failure_log, AppState, BrowserOperationKind,
        BrowserPreferenceV1,
        DeviceSettingsStore, LoginCleanupFilesV1, LoginCollectorProcessExpectation,
        LoginTermination, LoginTarget,
        NotificationKindV1, PhaseLaunchBarrier, ProgressFile, RunningSync, RunningSyncIdentity,
        StartupRecoveryStateV1, SyncFinalizationGate, TrustedLoginCleanupV1,
        TrustedLoginProcessCleanupV1, TrustedSyncProcessCleanupV1,
    };
    use douyin_history_archive::bootstrap::{
        write_bootstrap_atomic, write_data_root_marker_atomic, BootstrapV1, DataRootMarkerV1,
    };
    use douyin_history_archive::collector::{
        browser_process::BrowserProcessRecordV2,
        browser_state::{
            browser_profile_path, ensure_account_profile_root, ensure_profile_generation,
            read_browser_selection, read_login_validation, write_browser_selection,
            write_login_validation, BrowserId, BrowserSelection, LoginValidation,
        },
        contracts::CollectorDiagnostic,
        normalization::HistoryTimeUnit,
        session_probe::{ObservedHomeIdentityV1, SessionStateSource},
        sync_engine::finish_result,
    };
    use douyin_history_archive::collector_process::{
        CollectorProcessExpectation, CollectorProcessIdentityV1, LoginCollectorProcessIdentityV1,
    };
    use douyin_history_archive::login_attempt::{
        create_login_attempt_state, LoginAttemptEnvelopeV1,
    };
    use douyin_history_archive::notifications::{
        upsert_notification, NotificationInsertV1, NotificationSeverityV1,
    };
    use douyin_history_archive::settings_store::{
        AccountSettingsV1, AccountSettingsV2, ArchiveListRangeV1, GlobalSettingsV2, HistoryRangeV1,
        LikeCountFormatV1, NotificationSettingsV1, SyncModeV1,
    };
    use douyin_history_archive::sync::{
        automatic_config_hash, compute_archive_hashes, freeze_task_payload,
        ArchiveCollectionSuccessV1, ArchiveCompleteStopV1, ArchiveCompletionV1, ArchiveItemV1,
        ArchiveRunModeV1, AutomaticScheduleConfigV1, AutomaticSyncConfigV1,
        BrowserExecutionIdentityV1, BrowserIdV1, CapabilityRegistry, NewSyncJobV1, QueueStore,
        RangeFreezeV1, RuntimeSyncWorkerValidator, SchedulerTickResultV1, SchedulerWorkerConfigV1,
        SchedulerWorkerHandle, SourceExecutionResultV1, StoredSyncJobV1, SyncExecutionEnvelopeV1,
        SyncJobStatusV1, SyncSourceV1, SyncStatusV2, SyncTaskPayloadV1, SyncTriggerV1,
        SyncWorkerValidator, TaskKeyV1, WorkerJobDecision,
    };
    use douyin_history_archive::sync_driver::{
        phase_completion_decision, PhaseCompletionDecision, PhaseCompletionEvidence,
    };
    use douyin_history_archive::sync_orchestration::{
        cas_orchestration, create_orchestration, read_orchestration, request_orchestration_cancel,
        SyncLifecycle, SyncOrchestrationV1, SyncPhase, SyncPhaseEnvelopeV1,
    };
    use douyin_history_archive::uninstall::{
        parse_early_process_mode, DeviceAutostartSnapshotV1, EarlyProcessModeV1,
        GuardAuthenticationV1, PackageManifestV1, UninstallPhaseV1, UninstallSessionV1,
        UserDataDispositionV1,
    };
    use douyin_history_archive::{
        create_login_attempt, migrate_schema_to_latest_at_startup, migrate_schema_to_v5_at_startup,
        AppPaths, CollectorInvocation, CollectorRange,
    };
    use rusqlite::{params, Connection};
    use std::{
        fs,
        net::TcpListener,
        path::PathBuf,
        process::Command,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
        time::{Duration, SystemTime},
    };

    #[test]
    fn login_watchdog_keeps_live_cdp_browser_after_launcher_process_handoff() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-login-cdp-liveness-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let profile_key = "0123456789abcdef0123456789abcdef";
        let attempt_id = "fedcba9876543210fedcba9876543210";
        let account_root = paths.account_browser_root(profile_key).unwrap();
        ensure_account_profile_root(&account_root).unwrap();
        let profile = browser_profile_path(&account_root, BrowserId::Edge);
        fs::create_dir_all(&profile).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let record = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: u32::MAX,
            creation_time_100ns: "1".to_string(),
            browser_id: BrowserId::Edge,
            executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
                .to_string(),
            profile_path: profile.to_string_lossy().into_owned(),
            user_data_dir_argument: format!("--user-data-dir={}", profile.to_string_lossy()),
            devtools_port: port,
            browser_websocket_path: "/devtools/browser/fixture".to_string(),
            started_at: "2026-08-16T00:00:00.000Z".to_string(),
        };
        douyin_history_archive::collector::file_store::write_json_atomically(
            &paths.login_task_browser_pid_path(attempt_id).unwrap(),
            &record,
        )
        .unwrap();

        assert_eq!(
            recorded_login_task_browser_is_alive(&paths, profile_key, attempt_id),
            Some(true)
        );
        drop(listener);
        assert_eq!(
            recorded_login_task_browser_is_alive(&paths, profile_key, attempt_id),
            Some(false)
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn observed_login_identity_marks_browser_shutdown_as_expected() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-login-observed-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let attempt_id = "fedcba9876543210fedcba9876543210";
        let target = LoginTarget {
            account_id: None,
            profile_key: "0123456789abcdef0123456789abcdef".to_string(),
            browser_preference: BrowserPreferenceV1::Auto,
        };
        let envelope = LoginAttemptEnvelopeV1::new(
            attempt_id,
            target.account_id,
            &target.profile_key,
            ObservedHomeIdentityV1 {
                page_url: "https://www.douyin.com/".to_string(),
                uid: Some("uid-test".to_string()),
                sec_uid: Some("sec-test".to_string()),
                nickname: Some("fixture".to_string()),
                avatar_url: None,
                sources: vec![SessionStateSource::InitialStateUser],
            },
        )
        .unwrap();
        douyin_history_archive::collector::file_store::write_json_atomically(
            &paths.login_task_identity_path(attempt_id).unwrap(),
            &envelope,
        )
        .unwrap();

        assert!(login_identity_observed_for_shutdown(
            &paths, &target, attempt_id
        ));
        let mismatched = LoginTarget {
            profile_key: "11111111111111111111111111111111".to_string(),
            ..target.clone()
        };
        assert!(!login_identity_observed_for_shutdown(
            &paths,
            &mismatched,
            attempt_id
        ));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_login_cleanup_preserves_the_sanitized_diagnostic() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-login-diagnostic-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let attempt_id = "abcdef0123456789abcdef0123456789";
        let target = LoginTarget {
            account_id: None,
            profile_key: "0123456789abcdef0123456789abcdef".to_string(),
            browser_preference: BrowserPreferenceV1::Auto,
        };
        let diagnostic_path = paths.login_task_diagnostic_path(attempt_id).unwrap();
        let identity_path = paths.login_task_identity_path(attempt_id).unwrap();
        let log_path = paths.login_task_log_path(attempt_id).unwrap();
        fs::write(&diagnostic_path, b"sanitized diagnostic").unwrap();
        fs::write(&identity_path, b"provisional identity").unwrap();
        write_login_failure_log(&paths, attempt_id, "browser_shutdown_timeout");

        cleanup_durable_login_attempt(&paths, &target, attempt_id);

        assert!(diagnostic_path.exists());
        assert!(log_path.exists());
        assert!(fs::read_to_string(log_path)
            .unwrap()
            .contains("error_code=browser_shutdown_timeout"));
        assert!(!identity_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_login_returns_the_collector_diagnostic_code() {
        assert_eq!(
            resolve_login_failure_code(
                Some(LoginTermination::Failed),
                Some("browser_shutdown_timeout".to_string()),
            ),
            "browser_shutdown_timeout"
        );
    }

    #[test]
    fn app_state_initializes_one_bootstrap_managed_runtime_path() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let app_data = std::env::temp_dir().join(format!("emily-runtime-state-{unique}"));
        let resources = app_data.join("resources");
        let paths = resolve_initial_paths_from(app_data.clone(), resources.clone()).unwrap();
        assert_eq!(paths.bootstrap_dir, app_data);
        assert_eq!(paths.data_root, paths.bootstrap_dir.join("Data"));
        assert_eq!(paths.resource_dir, resources);
        assert!(paths.data_root.join(".emily-data-root.json").is_file());
        assert!(paths.bootstrap_dir.join("bootstrap.json").is_file());

        let state = AppState::default();
        assert_eq!(
            state.acquire_paths().unwrap_err(),
            "runtime_paths_not_initialized"
        );
        state.initialize_runtime_paths(paths.clone()).unwrap();
        assert_eq!(state.acquire_paths().unwrap().data_root, paths.data_root);
        assert_eq!(
            state.initialize_runtime_paths(paths).unwrap_err(),
            "runtime_paths_already_initialized"
        );
        let _ = fs::remove_dir_all(app_data);
    }

    #[test]
    fn notification_commands_round_trip_and_emit_change_events() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-notification-ipc-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        let connection = prepare_latest_database(&paths);
        let first = upsert_notification(
            &connection,
            NotificationInsertV1 {
                kind: "loginExpired".to_string(),
                severity: NotificationSeverityV1::Error,
                dedupe_key: "ipc:first".to_string(),
                title: "登录已失效".to_string(),
                body: "请重新登录".to_string(),
                account_id: None,
                related_type: None,
                related_id: None,
                created_at: "2026-08-19T12:00:00.000Z".to_string(),
            },
        )
        .unwrap();
        drop(connection);

        let state = AppState::default();
        state.initialize_runtime_paths(paths.clone()).unwrap();
        state.normal_runtime_ready.store(true, Ordering::Release);
        assert_eq!(
            list_notifications_from_state(&state, false, 50, 0)
                .unwrap()
                .total,
            1
        );

        let mut events = Vec::new();
        let read = mark_notification_read_from_state(
            &state,
            first.id,
            true,
            "2026-08-19T12:01:00.000Z",
            |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(read.read_at.as_deref(), Some("2026-08-19T12:01:00.000Z"));
        let unread = mark_notification_read_from_state(
            &state,
            first.id,
            false,
            "2026-08-19T12:02:00.000Z",
            |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();
        assert!(unread.read_at.is_none());

        let connection = open_database(&paths.database).unwrap();
        upsert_notification(
            &connection,
            NotificationInsertV1 {
                kind: "backupFailed".to_string(),
                severity: NotificationSeverityV1::Warning,
                dedupe_key: "ipc:second".to_string(),
                title: "备份失败".to_string(),
                body: "请检查备份目录".to_string(),
                account_id: None,
                related_type: None,
                related_id: None,
                created_at: "2026-08-19T12:03:00.000Z".to_string(),
            },
        )
        .unwrap();
        drop(connection);
        assert_eq!(
            mark_all_notifications_read_from_state(
                &state,
                "2026-08-19T12:04:00.000Z",
                |event| {
                    events.push(event);
                    Ok(())
                },
            )
            .unwrap(),
            2
        );
        let resolved = resolve_notification_from_state(
            &state,
            first.id,
            "2026-08-19T12:05:00.000Z",
            |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            resolved.resolved_at.as_deref(),
            Some("2026-08-19T12:05:00.000Z")
        );
        assert_eq!(
            list_notifications_from_state(&state, false, 50, 0)
                .unwrap()
                .total,
            1
        );
        assert_eq!(
            list_notifications_from_state(&state, true, 50, 0)
                .unwrap()
                .total,
            2
        );
        assert_eq!(events, vec!["notification:changed"; 4]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn system_notification_delivery_respects_each_user_setting() {
        let mut settings = NotificationSettingsV1::default();
        settings.login_expired = false;
        settings.automatic_failure_streak = false;
        settings.backup_failure = false;
        settings.backup_completed = true;

        assert!(!system_notification_enabled(
            &settings,
            NotificationKindV1::LoginExpired
        ));
        assert!(!system_notification_enabled(
            &settings,
            NotificationKindV1::AutomaticFailureStreak
        ));
        assert!(!system_notification_enabled(
            &settings,
            NotificationKindV1::BackupFailed
        ));
        assert!(system_notification_enabled(
            &settings,
            NotificationKindV1::BackupCompleted
        ));
        assert!(system_notification_enabled(
            &settings,
            NotificationKindV1::FirstTrayHide
        ));
    }

    #[test]
    fn automatic_backup_runtime_tick_returns_the_next_worker_wake() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-backup-main-tick-{unique}"));
        let paths = AppPaths::from_roots(
            root.join("bootstrap"),
            root.join("data"),
            root.join("resources"),
        );
        prepare_latest_database(&paths);
        let state = AppState::default();
        state.initialize_runtime_paths(paths.clone()).unwrap();
        state
            .initialize_device_settings(
                DeviceSettingsStore::open(paths.bootstrap_dir.clone()).unwrap(),
            )
            .unwrap();
        let mut global = GlobalSettingsV2::default();
        global.backup.enabled = true;
        global.backup.interval_days = 1;
        global.backup.execution_time = "03:00".to_string();
        global.notifications.backup_completed = true;
        update_global_settings_from_state(&state, global).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-14T19:01:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut notifications = Vec::new();

        let result = automatic_backup_tick_runtime_with(&state, now, Duration::ZERO, |kind| {
            notifications.push(kind);
            Ok(())
        })
        .unwrap();

        assert!(!result.paused);
        assert!(result.queued_job_ids.is_empty());
        assert_eq!(
            result.next_wake_at.as_deref(),
            Some("2026-08-15T19:00:00.000Z")
        );
        assert_eq!(notifications, vec![NotificationKindV1::BackupCompleted]);
        assert_eq!(fs::read_dir(&paths.backups).unwrap().count(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn shared_settings_wake_reaches_the_automatic_backup_worker() {
        let ticks = Arc::new(AtomicUsize::new(0));
        let tick_counter = Arc::clone(&ticks);
        let worker = SchedulerWorkerHandle::start(SchedulerWorkerConfigV1::new(
            Duration::from_secs(60),
            Arc::new(move || {
                tick_counter.fetch_add(1, Ordering::SeqCst);
                Ok(SchedulerTickResultV1 {
                    paused: false,
                    next_wake_at: None,
                    queued_job_ids: Vec::new(),
                })
            }),
        ))
        .unwrap();
        for _ in 0..100 {
            if ticks.load(Ordering::SeqCst) >= 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let baseline = ticks.load(Ordering::SeqCst);
        assert!(baseline >= 1);
        let state = AppState::default();
        assert!(state.automatic_backup_worker.set(worker).is_ok());

        wake_scheduler_and_queue(&state).unwrap();

        for _ in 0..100 {
            if ticks.load(Ordering::SeqCst) > baseline {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(ticks.load(Ordering::SeqCst) > baseline);
        let worker = state.automatic_backup_worker.get().unwrap();
        worker.stop().unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn automatic_backup_worker_lifecycle_is_owned_once_and_joined() {
        let ticks = Arc::new(AtomicUsize::new(0));
        let tick_counter = Arc::clone(&ticks);
        let state = AppState::default();

        initialize_automatic_backup_runtime_with(
            &state,
            Arc::new(move || {
                tick_counter.fetch_add(1, Ordering::SeqCst);
                Ok(SchedulerTickResultV1 {
                    paused: false,
                    next_wake_at: None,
                    queued_job_ids: Vec::new(),
                })
            }),
        )
        .unwrap();
        assert_eq!(
            initialize_automatic_backup_runtime_with(
                &state,
                Arc::new(|| {
                    Ok(SchedulerTickResultV1 {
                        paused: false,
                        next_wake_at: None,
                        queued_job_ids: Vec::new(),
                    })
                }),
            )
            .unwrap_err(),
            "automatic_backup_worker_already_initialized"
        );
        for _ in 0..100 {
            if ticks.load(Ordering::SeqCst) >= 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(ticks.load(Ordering::SeqCst) >= 1);

        stop_automatic_backup_runtime(&state).unwrap();
        join_automatic_backup_runtime(&state).unwrap();
    }

    #[test]
    fn compatibility_sync_options_follow_the_history_settings_range() {
        let mut account = AccountSettingsV2::default();
        account.history.range = HistoryRangeV1::Days { days: 23 };
        account.scroll_speed = 4_800;
        account.hide_browser = true;

        let days = sync_options_from_account_settings(&account);
        assert_eq!(days.mode, "days");
        assert_eq!(days.days, Some(23));
        assert_eq!(days.scroll_speed, Some(4_800));
        assert_eq!(days.hide_browser, Some(true));

        account.history.range = HistoryRangeV1::Bottom;
        let bottom = sync_options_from_account_settings(&account);
        assert_eq!(bottom.mode, "bottom");
        assert_eq!(bottom.days, None);
        assert_eq!(bottom.scroll_speed, Some(4_800));
        assert_eq!(bottom.hide_browser, Some(true));
    }

    #[test]
    fn settings_command_helpers_resolve_scope_and_obey_maintenance() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let app_data = std::env::temp_dir().join(format!("emily-settings-command-{unique}"));
        let install_dir = app_data.join("install");
        fs::create_dir_all(&install_dir).unwrap();
        let paths = resolve_initial_paths_from(app_data.clone(), install_dir.clone()).unwrap();
        let connection = prepare_latest_database(&paths);
        let account_id = insert_active_account(&connection);
        drop(connection);

        let state = AppState::default();
        state.initialize_runtime_paths(paths.clone()).unwrap();
        let initial =
            get_app_settings_from_state(&state, Some(account_id), install_dir.clone()).unwrap();
        assert_eq!(
            initial.account.history.range,
            HistoryRangeV1::Days { days: 7 }
        );

        let account = AccountSettingsV2::from(AccountSettingsV1 {
            sync_mode: SyncModeV1::Bottom,
            sync_days: 30,
            hide_browser: true,
            scroll_speed: 5_000,
        });
        update_account_settings_from_state(&state, account_id, account.clone()).unwrap();
        let mut global = GlobalSettingsV2::default();
        global.like_count_format = LikeCountFormatV1::K;
        update_global_settings_from_state(&state, global.clone()).unwrap();
        let updated =
            get_app_settings_from_state(&state, Some(account_id), install_dir.clone()).unwrap();
        assert_eq!(updated.account, account);
        assert_eq!(updated.global, global);
        assert_eq!(
            update_account_settings_from_state(&state, 999, AccountSettingsV2::default(),)
                .unwrap_err(),
            "account_not_found"
        );

        let maintenance = state
            .runtime_paths
            .get()
            .unwrap()
            .begin_maintenance()
            .unwrap();
        assert_eq!(
            get_app_settings_from_state(&state, Some(account_id), install_dir).unwrap_err(),
            "data_root_migration_in_progress"
        );
        assert_eq!(
            update_global_settings_from_state(&state, GlobalSettingsV2::default()).unwrap_err(),
            "data_root_migration_in_progress"
        );
        drop(maintenance);
        fs::remove_dir_all(app_data).unwrap();
    }

    #[test]
    fn disabling_source_settings_cancels_the_queued_automatic_before_gate_release() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let app_data = std::env::temp_dir().join(format!("emily-settings-reconcile-{unique}"));
        let install_dir = app_data.join("install");
        fs::create_dir_all(&install_dir).unwrap();
        let paths = resolve_initial_paths_from(app_data.clone(), install_dir).unwrap();
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);

        let mut enabled = AccountSettingsV2::default();
        enabled.history.range = HistoryRangeV1::Bottom;
        enabled.history.schedule.enabled = true;
        enabled.history.schedule.interval_minutes = 15;
        douyin_history_archive::settings_store::update_account_settings(
            &paths.data_root,
            "sec:sec-test",
            enabled.clone(),
        )
        .unwrap();
        let config_sha256 = automatic_config_hash(&AutomaticSyncConfigV1::History {
            schema_version: 1,
            source_instance_key: String::new(),
            schedule: AutomaticScheduleConfigV1 {
                enabled: true,
                interval_minutes: 15,
            },
            range: enabled.history.range,
            scroll_speed: enabled.scroll_speed,
            hide_browser: enabled.hide_browser,
            browser_preference: douyin_history_archive::device_settings::BrowserPreferenceV1::Auto,
        })
        .unwrap();
        let frozen = freeze_task_payload(SyncTaskPayloadV1::History {
            range: enabled.history.range,
            range_freeze: RangeFreezeV1 {
                frozen_at_ms: 1_786_507_200_000,
                cutoff_ms: None,
            },
            scroll_speed: enabled.scroll_speed,
            hide_browser: enabled.hide_browser,
            browser: BrowserExecutionIdentityV1 {
                preference: douyin_history_archive::device_settings::BrowserPreferenceV1::Auto,
                resolved_browser: BrowserIdV1::Edge,
                profile_key: "0123456789abcdef0123456789abcdef".to_string(),
                profile_generation: "1123456789abcdef0123456789abcdef".to_string(),
                executable_path_sha256: "b".repeat(64),
            },
        })
        .unwrap();
        let queued = QueueStore::enqueue_job(
            &mut connection,
            &NewSyncJobV1 {
                task_key: TaskKeyV1 {
                    account_id,
                    source: SyncSourceV1::History,
                    source_instance_key: String::new(),
                },
                trigger: SyncTriggerV1::Automatic,
                queued_at: "2026-08-12T00:00:00.000Z".to_string(),
                scheduled_for: Some("2026-08-12T01:00:00.000Z".to_string()),
                settings_revision: 1,
                config_sha256: Some(config_sha256),
                payload_json: frozen.payload_json,
                payload_sha256: frozen.payload_sha256,
                equivalence_sha256: frozen.equivalence_sha256,
            },
        )
        .unwrap();
        drop(connection);

        let state = AppState::default();
        state.initialize_runtime_paths(paths.clone()).unwrap();
        enabled.history.schedule.enabled = false;
        update_account_settings_from_state(&state, account_id, enabled).unwrap();

        let stored = QueueStore::get_stored_job(
            &open_database(&paths.database).unwrap(),
            queued.snapshot.id,
        )
        .unwrap();
        assert_eq!(stored.snapshot.status, SyncJobStatusV1::Cancelled);
        assert_eq!(
            stored.error_code.as_deref(),
            Some("automatic_settings_changed")
        );
        assert_eq!(
            state
                .operation_admission
                .snapshot()
                .unwrap()
                .owned_operation_count,
            0
        );
        fs::remove_dir_all(app_data).unwrap();
    }

    #[test]
    fn source_settings_update_rebuilds_at_most_one_current_automatic_snapshot() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let app_data = std::env::temp_dir().join(format!("emily-settings-rebuild-{unique}"));
        let install_dir = app_data.join("install");
        fs::create_dir_all(&install_dir).unwrap();
        let paths = resolve_initial_paths_from(app_data.clone(), install_dir).unwrap();
        let connection = prepare_latest_database(&paths);
        let account_id = insert_active_account(&connection);
        drop(connection);
        prepare_test_browser_state(&paths, "0123456789abcdef0123456789abcdef");

        let state = AppState::default();
        state.initialize_runtime_paths(paths.clone()).unwrap();
        let mut global = GlobalSettingsV2::default();
        global.startup_sync_delay_minutes = 0;
        update_global_settings_from_state(&state, global).unwrap();

        let mut enabled = AccountSettingsV2::default();
        enabled.history.range = HistoryRangeV1::Bottom;
        enabled.history.schedule.enabled = true;
        enabled.history.schedule.interval_minutes = 15;
        update_account_settings_from_state(&state, account_id, enabled.clone()).unwrap();

        let connection = open_database(&paths.database).unwrap();
        let first = QueueStore::list_queued_jobs(&connection).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].snapshot.task_key.source, SyncSourceV1::History);
        let validator = RuntimeSyncWorkerValidator::new(
            paths.data_root.clone(),
            paths.bootstrap_dir.clone(),
            CapabilityRegistry::stage_3_3().all().to_vec(),
        );
        assert_eq!(
            validator.validate(&connection, &first[0]).unwrap(),
            WorkerJobDecision::Run
        );
        let first_id = first[0].snapshot.id;
        drop(connection);

        enabled.scroll_speed += 250;
        update_account_settings_from_state(&state, account_id, enabled).unwrap();

        let connection = open_database(&paths.database).unwrap();
        let queued = QueueStore::list_queued_jobs(&connection).unwrap();
        assert_eq!(queued.len(), 1);
        assert_ne!(queued[0].snapshot.id, first_id);
        let replaced = QueueStore::get_stored_job(&connection, first_id).unwrap();
        assert_eq!(replaced.snapshot.status, SyncJobStatusV1::Cancelled);
        assert_eq!(
            replaced.error_code.as_deref(),
            Some("automatic_settings_changed")
        );
        drop(connection);
        fs::remove_dir_all(app_data).unwrap();
    }

    #[test]
    fn data_root_command_helpers_validate_then_switch_runtime_atomically() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-data-root-command-{unique}"));
        let app_data = root.join("bootstrap");
        let install_dir = root.join("install");
        let target = root.join("custom-data");
        fs::create_dir_all(&install_dir).unwrap();
        let paths = resolve_initial_paths_from(app_data.clone(), install_dir.clone()).unwrap();
        let connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        connection
            .execute(
                "INSERT INTO sync_jobs
                 (account_id, source, source_instance_key, trigger, queued_at, queue_seq,
                  settings_revision, payload_json, payload_sha256, equivalence_sha256, status)
                 VALUES (?1, 'history', '', 'manual', '2026-08-12T00:00:00.000Z', 1,
                         1, '{}', ?2, ?2, 'queued')",
                params![account_id, "a".repeat(64)],
            )
            .unwrap();
        let queued_job_id = connection.last_insert_rowid();
        let source = paths.data_root.clone();
        let state = AppState::default();
        state.initialize_runtime_paths(paths).unwrap();

        let validation = validate_data_root_from_state(&state, &target, &install_dir).unwrap();
        assert_eq!(
            fs::canonicalize(validation.normalized_path.parent().unwrap()).unwrap(),
            fs::canonicalize(target.parent().unwrap()).unwrap()
        );
        assert_eq!(validation.normalized_path.file_name(), target.file_name());
        assert!(!target.exists());

        let migrated = set_data_root_from_state(&state, target.clone(), install_dir).unwrap();
        assert_eq!(
            fs::canonicalize(&migrated.data_root).unwrap(),
            fs::canonicalize(&target).unwrap()
        );
        assert_eq!(migrated.retained_source.path, source);
        assert_eq!(state.acquire_paths().unwrap().data_root, migrated.data_root);
        let bootstrap = read_bootstrap(&app_data).unwrap().unwrap();
        assert_eq!(bootstrap.current_root.path, migrated.data_root);
        assert_eq!(bootstrap.retained_data_roots.len(), 1);
        let migrated_database = open_database(&target.join("history.sqlite")).unwrap();
        assert_eq!(
            migrated_database
                .query_row(
                    "SELECT status, error_code FROM sync_jobs WHERE id = ?1",
                    [queued_job_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .unwrap(),
            (
                "cancelled".to_string(),
                "maintenance_cancelled_before_start".to_string(),
            )
        );
        drop(migrated_database);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn onboarding_data_root_initialization_preserves_first_run_state_and_rejects_completed_setup() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("emily-onboarding-data-root-command-{unique}"));
        let app_data = root.join("bootstrap");
        let install_dir = root.join("install");
        let target = root.join("custom-data");
        let rejected_target = root.join("rejected-data");
        fs::create_dir_all(&install_dir).unwrap();
        let paths = resolve_initial_paths_from(app_data.clone(), install_dir.clone()).unwrap();
        let connection = prepare_v5_database(&paths);
        let profile_key = "0123456789abcdef0123456789abcdef";
        let account_id = insert_named_active_account(
            &connection,
            "uid-onboarding",
            "sec-onboarding",
            profile_key,
        );
        drop(connection);
        let profile_marker = paths
            .account_browser_root(profile_key)
            .unwrap()
            .join("edge")
            .join("Default")
            .join("Cookies");
        fs::create_dir_all(profile_marker.parent().unwrap()).unwrap();
        fs::write(&profile_marker, b"login-state").unwrap();
        fs::write(paths.data_root.join("settings.json"), b"first-run-settings").unwrap();

        let state = AppState::default();
        state.initialize_runtime_paths(paths.clone()).unwrap();
        state
            .initialize_device_settings(DeviceSettingsStore::open(app_data.clone()).unwrap())
            .unwrap();

        let initialized =
            initialize_onboarding_data_root_from_state(&state, target.clone(), install_dir.clone())
                .unwrap();
        assert_eq!(
            fs::canonicalize(&initialized.data_root).unwrap(),
            fs::canonicalize(&target).unwrap()
        );
        let migrated_database = open_database(&target.join("history.sqlite")).unwrap();
        assert_eq!(
            migrated_database
                .query_row(
                    "SELECT COUNT(*) FROM accounts WHERE id = ?1",
                    [account_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        drop(migrated_database);
        assert_eq!(
            fs::read(
                target
                    .join("profiles")
                    .join(profile_key)
                    .join("edge")
                    .join("Default")
                    .join("Cookies")
            )
            .unwrap(),
            b"login-state"
        );
        assert_eq!(
            fs::read(target.join("settings.json")).unwrap(),
            b"first-run-settings"
        );

        let store = state.device_settings.get().unwrap();
        let current = store.load().unwrap();
        store
            .compare_and_swap(current.revision, |device| {
                device.onboarding_step = 3;
                device.onboarding_completed = true;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            initialize_onboarding_data_root_from_state(
                &state,
                rejected_target.clone(),
                install_dir,
            )
            .unwrap_err(),
            "onboarding_already_completed"
        );
        assert!(!rejected_target.exists());
        fs::remove_dir_all(root).unwrap();
    }

    fn insert_active_account(connection: &Connection) -> i64 {
        insert_named_active_account(
            connection,
            "uid-test",
            "sec-test",
            "0123456789abcdef0123456789abcdef",
        )
    }

    fn insert_named_active_account(
        connection: &Connection,
        uid: &str,
        sec_uid: &str,
        profile_key: &str,
    ) -> i64 {
        connection
            .execute(
                "INSERT INTO accounts
                 (uid, sec_uid, profile_key, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'active', 'x', 'x')",
                params![uid, sec_uid, profile_key],
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

    fn enqueue_history_job(
        connection: &mut Connection,
        account_id: i64,
        frozen_at_ms: i64,
    ) -> StoredSyncJobV1 {
        enqueue_history_job_with_browser(
            connection,
            account_id,
            frozen_at_ms,
            BrowserExecutionIdentityV1 {
                preference: douyin_history_archive::device_settings::BrowserPreferenceV1::Edge,
                resolved_browser: BrowserIdV1::Edge,
                profile_key: "0123456789abcdef0123456789abcdef".to_string(),
                profile_generation: "1123456789abcdef0123456789abcdef".to_string(),
                executable_path_sha256: "b".repeat(64),
            },
        )
    }

    fn enqueue_history_job_with_browser(
        connection: &mut Connection,
        account_id: i64,
        frozen_at_ms: i64,
        browser: BrowserExecutionIdentityV1,
    ) -> StoredSyncJobV1 {
        let frozen = freeze_task_payload(SyncTaskPayloadV1::History {
            range: HistoryRangeV1::Bottom,
            range_freeze: RangeFreezeV1 {
                frozen_at_ms,
                cutoff_ms: None,
            },
            scroll_speed: 5_000,
            hide_browser: false,
            browser,
        })
        .unwrap();
        QueueStore::enqueue_job(
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
        .unwrap()
    }

    fn enqueue_running_history_job(
        connection: &mut Connection,
        account_id: i64,
        frozen_at_ms: i64,
    ) -> StoredSyncJobV1 {
        let queued = enqueue_history_job(connection, account_id, frozen_at_ms);
        QueueStore::claim_job(connection, queued.snapshot.id, "2026-08-12T00:00:01.000Z").unwrap()
    }

    fn enqueue_running_history_job_with_browser(
        connection: &mut Connection,
        account_id: i64,
        frozen_at_ms: i64,
        browser: BrowserExecutionIdentityV1,
    ) -> StoredSyncJobV1 {
        let queued =
            enqueue_history_job_with_browser(connection, account_id, frozen_at_ms, browser);
        QueueStore::claim_job(connection, queued.snapshot.id, "2026-08-12T00:00:01.000Z").unwrap()
    }

    fn enqueue_running_likes_job(
        connection: &mut Connection,
        account_id: i64,
        frozen_at_ms: i64,
    ) -> StoredSyncJobV1 {
        let frozen = freeze_task_payload(SyncTaskPayloadV1::Likes {
            range: ArchiveListRangeV1::All,
            range_freeze: RangeFreezeV1 {
                frozen_at_ms,
                cutoff_ms: None,
            },
            run_mode: ArchiveRunModeV1::Incremental,
            scroll_speed: 5_000,
            hide_browser: false,
            browser: BrowserExecutionIdentityV1 {
                preference: BrowserPreferenceV1::Edge,
                resolved_browser: BrowserIdV1::Edge,
                profile_key: "0123456789abcdef0123456789abcdef".to_string(),
                profile_generation: "1123456789abcdef0123456789abcdef".to_string(),
                executable_path_sha256: "b".repeat(64),
            },
        })
        .unwrap();
        let queued = QueueStore::enqueue_job(
            connection,
            &NewSyncJobV1 {
                task_key: TaskKeyV1 {
                    account_id,
                    source: SyncSourceV1::Likes,
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
        .unwrap();
        QueueStore::claim_job(connection, queued.snapshot.id, "2026-08-12T00:00:01.000Z").unwrap()
    }

    fn likes_envelope(job: &StoredSyncJobV1, aweme_id: &str) -> SyncExecutionEnvelopeV1 {
        let records = vec![ArchiveItemV1 {
            aweme_id: aweme_id.to_string(),
            canonical_url: format!("https://www.douyin.com/video/{aweme_id}"),
            content_type: "video".to_string(),
            source_position: 0,
            remote_created_at_ms: None,
            title: Some("测试喜欢视频".to_string()),
            author_name: Some("测试作者".to_string()),
            cover_url: Some("https://example.com/cover.jpg".to_string()),
            like_count: Some(42),
        }];
        let (identity_hash, field_hash) = compute_archive_hashes(&records).unwrap();
        SyncExecutionEnvelopeV1 {
            schema_version: 1,
            account_id: job.snapshot.task_key.account_id,
            job_id: job.snapshot.id,
            source: SyncSourceV1::Likes,
            source_instance_key: String::new(),
            task_payload_sha256: job.snapshot.task_payload_sha256.clone(),
            result: SourceExecutionResultV1::Likes(ArchiveCollectionSuccessV1 {
                records,
                identity_hash,
                field_hash,
                completion: ArchiveCompletionV1::Complete {
                    stop: ArchiveCompleteStopV1::Bottom,
                },
            }),
        }
    }

    fn prepare_v5_database(paths: &AppPaths) -> Connection {
        prepare_app_directories(paths).expect("directories");
        let mut connection = open_database(&paths.database).expect("database");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        connection
    }

    fn prepare_latest_database(paths: &AppPaths) -> Connection {
        let mut connection = prepare_v5_database(paths);
        migrate_schema_to_latest_at_startup(&mut connection).expect("latest schema");
        connection
    }

    fn uninstall_session_for(paths: &AppPaths, root_id: &str) -> UninstallSessionV1 {
        let executable = std::env::current_exe().unwrap();
        let product = compiled_product_identity();
        UninstallSessionV1 {
            schema_version: 1,
            authentication: GuardAuthenticationV1 {
                nonce: "abcdef0123456789abcdef0123456789".to_string(),
                installer_pid: 1,
                installer_created_at_100ns: 1,
            },
            root_id: root_id.to_string(),
            old_run_value: None,
            old_device_autostart: DeviceAutostartSnapshotV1::default(),
            phase: UninstallPhaseV1::Prepared,
            device_settings_dir: paths.bootstrap_dir.clone(),
            install_dir: executable.parent().unwrap().to_path_buf(),
            main_executable: executable,
            application_id: product.application_id.clone(),
            product_name: product.product_name.clone(),
            old_build_id: product.build_id.clone(),
            old_semantic_version: product.semantic_version.clone(),
            architecture: product.architecture.clone(),
            protected_paths: vec![paths.data_root.clone()],
            user_data_disposition: UserDataDispositionV1::Retain,
            package_manifest: PackageManifestV1 {
                schema_version: 1,
                application_id: product.application_id,
                product_name: product.product_name,
                build_id: product.build_id,
                semantic_version: product.semantic_version,
                architecture: product.architecture,
                files: Vec::new(),
            },
            package_manifest_sha256: "a".repeat(64),
            prepared_at: "2026-08-13T00:00:00.000Z".to_string(),
            updated_at: "2026-08-13T00:00:01.000Z".to_string(),
        }
    }

    fn running_recovery_fixture(
        label: &str,
    ) -> (
        AppPaths,
        i64,
        i64,
        SyncOrchestrationV1,
        Mutex<Option<RunningSync>>,
        PathBuf,
    ) {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-{label}-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let job_id = start_sync_job(&mut connection, account_id).unwrap();
        let collecting_attempt = "0123456789abcdef0123456789abcdef";
        let collecting = create_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            collecting_attempt,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let collecting = collecting
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(
            &paths,
            &read_orchestration(&paths, account_id, job_id).unwrap(),
            &collecting,
        )
        .unwrap();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let recovery_attempt = "11111111111111111111111111111111";
        let recovering = douyin_history_archive::sync_driver::prepare_next_phase(
            &collecting,
            SyncPhase::RecoveringLogin,
            recovery_attempt,
            Some(now_ms + 30_000),
            now_ms,
            "2026-07-20T00:00:02.000Z",
        )
        .unwrap();
        cas_orchestration(&paths, &collecting, &recovering).unwrap();
        let recovering = recovering
            .next(SyncLifecycle::Running, "2026-07-20T00:00:03.000Z")
            .unwrap();
        cas_orchestration(
            &paths,
            &read_orchestration(&paths, account_id, job_id).unwrap(),
            &recovering,
        )
        .unwrap();
        let slot = Mutex::new(Some(RunningSync {
            _path_lease: None,
            _browser_lease: None,
            paths: paths.clone(),
            account_id,
            job_id,
            profile_key: "0123456789abcdef0123456789abcdef".to_string(),
            executable: std::env::current_exe().unwrap(),
            phase: SyncPhase::RecoveringLogin,
            phase_attempt_id: recovery_attempt.to_string(),
            child: None,
        }));
        (paths, account_id, job_id, recovering, slot, root)
    }

    fn startup_recovery_fixture(label: &str) -> (PathBuf, AppPaths, Connection, i64, i64) {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-{label}-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::from("C:/unused-resources"));
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let job_id = start_sync_job(&mut connection, account_id).unwrap();
        (root, paths, connection, account_id, job_id)
    }

    fn write_startup_orchestration(
        paths: &AppPaths,
        account_id: i64,
        job_id: i64,
        phase: SyncPhase,
        lifecycle: SyncLifecycle,
        attempt_id: &str,
        cancelled: bool,
    ) -> SyncOrchestrationV1 {
        let connection = open_database(&paths.database).unwrap();
        let options_hash = QueueStore::get_stored_job(&connection, job_id)
            .unwrap()
            .snapshot
            .task_payload_sha256;
        let orchestration = SyncOrchestrationV1 {
            schema_version: 1,
            revision: 4,
            account_id,
            sync_job_id: job_id,
            phase,
            phase_attempt_id: attempt_id.to_string(),
            lifecycle,
            login_recovery_count: u8::from(phase != SyncPhase::Collecting),
            cancel_requested_at: cancelled.then(|| "2026-07-20T00:00:05.000Z".to_string()),
            options_hash,
            deadline_at_ms: (phase == SyncPhase::RecoveringLogin)
                .then(|| chrono::Utc::now().timestamp_millis() + 30_000),
            updated_at: "2026-07-20T00:00:04.000Z".to_string(),
        };
        fs::write(
            paths.orchestration_path(account_id, job_id),
            serde_json::to_vec(&orchestration).unwrap(),
        )
        .unwrap();
        if cancelled {
            fs::write(paths.cancel_path(account_id, job_id), b"cancel\n").unwrap();
        }
        orchestration
    }

    fn write_empty_startup_result(
        paths: &AppPaths,
        orchestration: &SyncOrchestrationV1,
    ) -> PathBuf {
        let result = finish_result(
            orchestration.account_id,
            orchestration.sync_job_id,
            1_783_742_400_000,
            CollectorRange::Bottom,
            HistoryTimeUnit::Milliseconds,
            Vec::new(),
        )
        .unwrap();
        let path = paths
            .phase_result_path(
                orchestration.account_id,
                orchestration.sync_job_id,
                orchestration.phase,
                &orchestration.phase_attempt_id,
            )
            .unwrap();
        let connection = open_database(&paths.database).unwrap();
        let job = QueueStore::get_stored_job(&connection, orchestration.sync_job_id).unwrap();
        let envelope = SyncExecutionEnvelopeV1 {
            schema_version: 1,
            account_id: orchestration.account_id,
            job_id: orchestration.sync_job_id,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
            task_payload_sha256: job.snapshot.task_payload_sha256,
            result: SourceExecutionResultV1::History(result),
        };
        fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        path
    }

    #[test]
    fn early_process_modes_are_split_before_gui_initialization() {
        let arguments = vec![
            std::ffi::OsString::from("--collector"),
            std::ffi::OsString::from("recover-login"),
            std::ffi::OsString::from("--headless"),
        ];

        assert!(matches!(
            parse_early_process_mode(&arguments).unwrap(),
            EarlyProcessModeV1::Collector(values) if values == vec![
                std::ffi::OsString::from("recover-login"),
                std::ffi::OsString::from("--headless"),
            ]
        ));
        let resolver_arguments = vec![
            std::ffi::OsString::from("--download-resolver"),
            std::ffi::OsString::from("--browser-data-dir"),
            std::ffi::OsString::from("C:/Emily"),
        ];
        assert!(matches!(
            parse_early_process_mode(&resolver_arguments).unwrap(),
            EarlyProcessModeV1::DownloadResolver(values) if values == vec![
                std::ffi::OsString::from("--browser-data-dir"),
                std::ffi::OsString::from("C:/Emily"),
            ]
        ));
        assert!(matches!(
            parse_early_process_mode(&[std::ffi::OsString::from("--gui")]).unwrap(),
            EarlyProcessModeV1::Gui
        ));
    }

    #[test]
    fn sync_driver_cancellation_wins_when_a_running_result_arrives() {
        let orchestration = SyncOrchestrationV1 {
            schema_version: 1,
            revision: 2,
            account_id: 11,
            sync_job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            lifecycle: SyncLifecycle::Running,
            login_recovery_count: 0,
            cancel_requested_at: Some("2026-07-20T00:00:01.000Z".to_string()),
            options_hash: "a".repeat(64),
            deadline_at_ms: None,
            updated_at: "2026-07-20T00:00:01.000Z".to_string(),
        };
        let evidence = PhaseCompletionEvidence {
            process_succeeded: true,
            result_ready: true,
            diagnostic_status: None,
            diagnostic_error_code: None,
            cancel_file_exists: false,
        };

        assert_eq!(
            phase_completion_decision(&orchestration, &evidence),
            PhaseCompletionDecision::Cancel
        );
    }

    #[test]
    fn next_phase_never_resumes_after_cancellation_at_any_launch_barrier() {
        for cancel_at in [
            PhaseLaunchBarrier::BeforePreparing,
            PhaseLaunchBarrier::AfterPreparing,
            PhaseLaunchBarrier::AfterSpawn,
            PhaseLaunchBarrier::BeforeResume,
        ] {
            let unique = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "emily-phase-barrier-{}-{unique}",
                cancel_at.as_str()
            ));
            let paths = AppPaths::new(root.clone(), PathBuf::new());
            let mut connection = prepare_v5_database(&paths);
            let account_id = insert_active_account(&connection);
            let job_id = start_sync_job(&mut connection, account_id).unwrap();
            let collecting_attempt = "0123456789abcdef0123456789abcdef";
            let collecting = create_orchestration(
                &paths,
                account_id,
                job_id,
                SyncPhase::Collecting,
                collecting_attempt,
                &"a".repeat(64),
                None,
                "2026-07-20T00:00:00.000Z",
            )
            .unwrap();
            let collecting = collecting
                .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
                .unwrap();
            cas_orchestration(
                &paths,
                &read_orchestration(&paths, account_id, job_id).unwrap(),
                &collecting,
            )
            .unwrap();
            let recovery_attempt = "11111111111111111111111111111111";
            let recovering = douyin_history_archive::sync_driver::prepare_next_phase(
                &collecting,
                SyncPhase::RecoveringLogin,
                recovery_attempt,
                Some(1_784_515_230_000),
                1_784_515_200_000,
                "2026-07-20T00:00:02.000Z",
            )
            .unwrap();
            let resumed_marker = root.join("resumed.txt");
            let invocation = CollectorInvocation {
                program: PathBuf::from(std::env::var_os("ComSpec").unwrap()),
                args: vec![
                    "/d".to_string(),
                    "/c".to_string(),
                    format!("echo resumed>\"{}\"", resumed_marker.display()),
                ],
                current_dir: None,
            };
            let slot = Mutex::new(Some(RunningSync {
                _path_lease: None,
                _browser_lease: None,
                paths: paths.clone(),
                account_id,
                job_id,
                profile_key: "0123456789abcdef0123456789abcdef".to_string(),
                executable: std::env::current_exe().unwrap(),
                phase: SyncPhase::Collecting,
                phase_attempt_id: collecting_attempt.to_string(),
                child: None,
            }));

            let result = launch_next_phase_with(
                &slot,
                &paths,
                &collecting,
                &recovering,
                invocation,
                |barrier, state| {
                    if barrier == cancel_at {
                        request_orchestration_cancel(&paths, state, "2026-07-20T00:00:03.000Z")?;
                    }
                    Ok(())
                },
            );

            assert_eq!(
                result.err().as_deref(),
                Some("sync_cancel_requested"),
                "{cancel_at:?}"
            );
            std::thread::sleep(Duration::from_millis(100));
            assert!(!resumed_marker.exists(), "{cancel_at:?}");
            assert!(read_orchestration(&paths, account_id, job_id)
                .unwrap()
                .cancel_requested_at
                .is_some());
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn retry_never_spawns_after_recovery_success_when_cancellation_is_persisted() {
        let (paths, account_id, job_id, recovering, slot, root) =
            running_recovery_fixture("retry-after-cancel");
        let now_ms = chrono::Utc::now().timestamp_millis();
        let retrying = douyin_history_archive::sync_driver::prepare_next_phase(
            &recovering,
            SyncPhase::RetryingAfterLogin,
            "22222222222222222222222222222222",
            None,
            now_ms,
            "2026-07-20T00:00:04.000Z",
        )
        .unwrap();
        let resumed_marker = root.join("retry-resumed.txt");
        let invocation = CollectorInvocation {
            program: PathBuf::from(std::env::var_os("ComSpec").unwrap()),
            args: vec![
                "/d".to_string(),
                "/c".to_string(),
                format!("echo resumed>\"{}\"", resumed_marker.display()),
            ],
            current_dir: None,
        };
        request_orchestration_cancel(&paths, &recovering, "2026-07-20T00:00:05.000Z").unwrap();

        let result =
            launch_next_phase_with(&slot, &paths, &recovering, &retrying, invocation, |_, _| {
                Ok(())
            });

        assert_eq!(result.err().as_deref(), Some("sync_cancel_requested"));
        std::thread::sleep(Duration::from_millis(100));
        assert!(!resumed_marker.exists());
        assert!(read_orchestration(&paths, account_id, job_id)
            .unwrap()
            .cancel_requested_at
            .is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn persistent_cancellation_beats_recovery_failure_terminalization() {
        let (paths, account_id, job_id, recovering, slot, root) =
            running_recovery_fixture("cancel-beats-recovery-failure");
        request_orchestration_cancel(&paths, &recovering, "2026-07-20T00:00:05.000Z").unwrap();
        let state = AppState {
            running: slot,
            ..AppState::default()
        };

        finish_phase_transition_error(&state, &paths, account_id, job_id, "login_recovery_failed")
            .unwrap();

        let connection = open_database(&paths.database).unwrap();
        let (account_status, job_status, error_code) = connection
            .query_row(
                "SELECT a.status, sj.status, sj.error_code
                 FROM sync_jobs sj JOIN accounts a ON a.id = sj.account_id
                 WHERE sj.id = ?1",
                [job_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(account_status, "active");
        assert_eq!(job_status, "cancelled");
        assert_eq!(error_code.as_deref(), Some("collector_cancelled"));
        assert!(state.running.lock().unwrap().is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn completed_collect_phase_reads_session_expiry_from_its_bound_diagnostic() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-expired-phase-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).unwrap();
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            11,
            7,
            SyncPhase::Collecting,
            attempt_id,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let running = preparing
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running).unwrap();
        let diagnostic = CollectorDiagnostic {
            schema_version: 3,
            account_id: 11,
            sync_job_id: 7,
            status: "failed".to_string(),
            phase: "collecting".to_string(),
            error_code: "session_expired".to_string(),
            message: "抖音登录状态已失效。".to_string(),
            record_count: 0,
            dom_card_count: 0,
            calendar_time_zone: "Asia/Shanghai".to_string(),
            elapsed_ms: 10,
            no_progress_ms: 0,
            retry_count: 0,
        };
        let envelope =
            SyncPhaseEnvelopeV1::new(11, 7, SyncPhase::Collecting, attempt_id, diagnostic).unwrap();
        let diagnostic_path = paths
            .phase_diagnostic_path(11, 7, SyncPhase::Collecting, attempt_id)
            .unwrap();
        fs::create_dir_all(diagnostic_path.parent().unwrap()).unwrap();
        fs::write(diagnostic_path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        let identity = RunningSyncIdentity {
            account_id: 11,
            job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };

        let observed = observe_phase_completion(&paths, &identity, false).unwrap();

        assert_eq!(observed.decision, PhaseCompletionDecision::StartRecovery);
        assert_eq!(
            observed
                .diagnostic
                .as_ref()
                .map(|value| value.error_code.as_str()),
            Some("session_expired")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hidden_recovery_persists_validation_only_after_identity_and_shutdown_checks() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-recovery-marker-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let job_id = start_sync_job(&mut connection, account_id).unwrap();
        let collecting_attempt = "0123456789abcdef0123456789abcdef";
        let collecting = create_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            collecting_attempt,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let collecting = collecting
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(
            &paths,
            &read_orchestration(&paths, account_id, job_id).unwrap(),
            &collecting,
        )
        .unwrap();
        let recovery_attempt = "11111111111111111111111111111111";
        let recovering = douyin_history_archive::sync_driver::prepare_next_phase(
            &collecting,
            SyncPhase::RecoveringLogin,
            recovery_attempt,
            Some(chrono::Utc::now().timestamp_millis() + 30_000),
            chrono::Utc::now().timestamp_millis(),
            "2026-07-20T00:00:02.000Z",
        )
        .unwrap();
        cas_orchestration(&paths, &collecting, &recovering).unwrap();
        let recovering = recovering
            .next(SyncLifecycle::Running, "2026-07-20T00:00:03.000Z")
            .unwrap();
        cas_orchestration(
            &paths,
            &read_orchestration(&paths, account_id, job_id).unwrap(),
            &recovering,
        )
        .unwrap();

        let account_root = paths
            .account_browser_root("0123456789abcdef0123456789abcdef")
            .unwrap();
        let profile = douyin_history_archive::collector::browser_state::browser_profile_path(
            &account_root,
            BrowserId::Edge,
        );
        fs::create_dir_all(&profile).unwrap();
        let executable = root.join("msedge.exe");
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let browser = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: u32::MAX,
            creation_time_100ns: "123".to_string(),
            browser_id: BrowserId::Edge,
            executable_path: executable.to_string_lossy().to_string(),
            profile_path: profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!("--user-data-dir={}", profile.display()),
            devtools_port: 9222,
            browser_websocket_path: "/devtools/browser/fixture".to_string(),
            started_at: "2026-07-20T00:00:02.000Z".to_string(),
        };
        let browser_envelope = SyncPhaseEnvelopeV1::new(
            account_id,
            job_id,
            SyncPhase::RecoveringLogin,
            recovery_attempt,
            browser,
        )
        .unwrap();
        fs::write(
            paths
                .phase_browser_pid_path(
                    account_id,
                    job_id,
                    SyncPhase::RecoveringLogin,
                    recovery_attempt,
                )
                .unwrap(),
            serde_json::to_vec(&browser_envelope).unwrap(),
        )
        .unwrap();
        let identity = ObservedHomeIdentityV1 {
            page_url: "https://www.douyin.com/".to_string(),
            uid: Some("uid-test".to_string()),
            sec_uid: Some("sec-test".to_string()),
            nickname: Some("fixture".to_string()),
            avatar_url: None,
            sources: vec![SessionStateSource::SsrRenderData],
        };

        complete_hidden_recovery(
            &paths,
            "0123456789abcdef0123456789abcdef",
            &recovering,
            identity,
        )
        .unwrap();

        let selection = read_browser_selection(&account_root)
            .unwrap()
            .expect("selection");
        let validation = read_login_validation(&account_root, &selection)
            .unwrap()
            .expect("validation");
        assert_eq!(validation.validation_id.as_deref(), Some(recovery_attempt));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hidden_recovery_rejects_noncanonical_home_urls_before_marker_validation() {
        let (paths, _, _, recovering, _, root) = running_recovery_fixture("recovery-home-url");

        for page_url in [
            "http://www.douyin.com/",
            "https://user@www.douyin.com/",
            "https://www.douyin.com:444/",
            "https://www.douyin.com/#fragment",
        ] {
            let identity = ObservedHomeIdentityV1 {
                page_url: page_url.to_string(),
                uid: Some("uid-test".to_string()),
                sec_uid: Some("sec-test".to_string()),
                nickname: Some("fixture".to_string()),
                avatar_url: None,
                sources: vec![SessionStateSource::SsrRenderData],
            };

            assert_eq!(
                complete_hidden_recovery(
                    &paths,
                    "0123456789abcdef0123456789abcdef",
                    &recovering,
                    identity,
                ),
                Err("account_identity_conflict".to_string()),
                "{page_url}"
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_phase_error_changes_account_state_only_for_login_and_identity_failures() {
        let deadline = CollectorDiagnostic {
            schema_version: 3,
            account_id: 11,
            sync_job_id: 7,
            status: "failed".to_string(),
            phase: "recovering_login".to_string(),
            error_code: "collector_deadline_exceeded".to_string(),
            message: "collector_deadline_exceeded".to_string(),
            record_count: 0,
            dom_card_count: 0,
            calendar_time_zone: "Asia/Shanghai".to_string(),
            elapsed_ms: 30_000,
            no_progress_ms: 0,
            retry_count: 0,
        };
        assert_eq!(
            terminal_phase_error(SyncPhase::RecoveringLogin, Some(&deadline)).0,
            "login_recovery_failed"
        );

        let ambiguous = CollectorDiagnostic {
            error_code: "account_identity_ambiguous".to_string(),
            ..deadline.clone()
        };
        assert_eq!(
            terminal_phase_error(SyncPhase::RecoveringLogin, Some(&ambiguous)).0,
            "account_identity_conflict"
        );

        let technical = CollectorDiagnostic {
            error_code: "cdp_connection_failed".to_string(),
            message: "无法连接到浏览器 CDP 服务。".to_string(),
            ..deadline.clone()
        };
        assert_eq!(
            terminal_phase_error(SyncPhase::RecoveringLogin, Some(&technical)).0,
            "cdp_connection_failed"
        );

        let retry_expired = CollectorDiagnostic {
            phase: "retrying_after_login".to_string(),
            error_code: "session_expired".to_string(),
            message: "抖音登录状态已失效。".to_string(),
            ..deadline
        };
        assert_eq!(
            terminal_phase_error(SyncPhase::RetryingAfterLogin, Some(&retry_expired)).0,
            "session_expired"
        );
    }

    #[test]
    fn recovery_watchdog_enforces_the_absolute_deadline_without_marking_user_cancel() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-recovery-deadline-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).unwrap();
        let collecting_attempt = "0123456789abcdef0123456789abcdef";
        let collecting = create_orchestration(
            &paths,
            11,
            7,
            SyncPhase::Collecting,
            collecting_attempt,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let collecting = collecting
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(
            &paths,
            &read_orchestration(&paths, 11, 7).unwrap(),
            &collecting,
        )
        .unwrap();
        let recovery_attempt = "11111111111111111111111111111111";
        let expired_at_ms = chrono::Utc::now().timestamp_millis() - 1;
        let recovering = douyin_history_archive::sync_driver::prepare_next_phase(
            &collecting,
            SyncPhase::RecoveringLogin,
            recovery_attempt,
            Some(expired_at_ms),
            expired_at_ms - 1,
            "2026-07-20T00:00:02.000Z",
        )
        .unwrap();
        cas_orchestration(&paths, &collecting, &recovering).unwrap();
        let recovering = recovering
            .next(SyncLifecycle::Running, "2026-07-20T00:00:03.000Z")
            .unwrap();
        cas_orchestration(
            &paths,
            &read_orchestration(&paths, 11, 7).unwrap(),
            &recovering,
        )
        .unwrap();
        let identity = RunningSyncIdentity {
            account_id: 11,
            job_id: 7,
            phase: SyncPhase::RecoveringLogin,
            phase_attempt_id: recovery_attempt.to_string(),
        };
        let invocation = CollectorInvocation {
            program: PathBuf::from(std::env::var_os("ComSpec").unwrap()),
            args: vec![
                "/d".to_string(),
                "/c".to_string(),
                "ping -n 30 127.0.0.1 >nul".to_string(),
            ],
            current_dir: None,
        };
        let child = Arc::new(Mutex::new(
            spawn_collector(&paths, &identity, invocation).unwrap(),
        ));
        child.lock().unwrap().resume().unwrap();

        let (_, forced_error) =
            wait_for_phase_child_with(&paths, &identity, &child, Duration::from_millis(10))
                .unwrap();

        assert_eq!(forced_error.as_deref(), Some("login_recovery_failed"));
        assert!(!paths.cancel_path(11, 7).exists());
        assert!(child.lock().unwrap().try_wait().unwrap().is_some());
        let _ = fs::remove_dir_all(root);
    }

    fn suspended_test_collector(
        label: &str,
    ) -> (
        AppPaths,
        Arc<Mutex<douyin_history_archive::collector_process::OwnedCollectorProcess>>,
        PathBuf,
    ) {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-{label}-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).unwrap();
        let identity = RunningSyncIdentity {
            account_id: 11,
            job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        };
        let invocation = CollectorInvocation {
            program: PathBuf::from(std::env::var_os("ComSpec").unwrap()),
            args: vec!["/d".to_string(), "/c".to_string(), "exit 0".to_string()],
            current_dir: None,
        };
        let child = spawn_collector(&paths, &identity, invocation).unwrap();
        (paths, Arc::new(Mutex::new(child)), root)
    }

    #[test]
    fn preparing_a_new_login_does_not_insert_a_pending_account() {
        let root = std::env::temp_dir().join(format!(
            "emily-new-login-target-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).unwrap();
        let mut connection = Connection::open(&paths.database).unwrap();
        migrate_schema_to_v5_at_startup(&mut connection).unwrap();

        let target = prepare_new_login_target(
            &paths,
            "0123456789abcdef0123456789abcdef",
            "2026-07-19T12:00:00.000Z",
            douyin_history_archive::device_settings::BrowserPreferenceV1::Chrome,
        )
        .expect("new login target");

        assert_eq!(target.account_id, None);
        assert_eq!(target.profile_key.len(), 32);
        assert_eq!(
            target.browser_preference,
            douyin_history_archive::device_settings::BrowserPreferenceV1::Chrome
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM accounts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn forced_cancel_without_diagnostic_is_still_cancelled() {
        assert_eq!(failed_sync_status(None, true), "cancelled");
        assert_eq!(failed_sync_status(Some("cancelled"), false), "cancelled");
        assert_eq!(failed_sync_status(Some("failed"), false), "failed");
    }

    #[test]
    fn phase_payload_reader_rejects_raw_and_stale_outputs() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-phase-reader-{unique}"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("progress.json");
        let orchestration = SyncOrchestrationV1 {
            schema_version: 1,
            revision: 1,
            account_id: 11,
            sync_job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            lifecycle: SyncLifecycle::Running,
            login_recovery_count: 0,
            cancel_requested_at: None,
            options_hash: "a".repeat(64),
            deadline_at_ms: None,
            updated_at: "2026-07-20T00:00:00.000Z".to_string(),
        };
        let progress = serde_json::json!({
            "schemaVersion": 3,
            "accountId": 11,
            "syncJobId": 7,
            "phase": "scrolling",
            "recordCount": 23,
            "elapsedMs": 456,
            "retryCount": 1,
            "canCancel": true
        });
        let envelope = SyncPhaseEnvelopeV1::new(
            11,
            7,
            SyncPhase::Collecting,
            "0123456789abcdef0123456789abcdef",
            progress.clone(),
        )
        .unwrap();
        fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();

        let decoded: ProgressFile =
            read_sync_phase_payload(&path, &orchestration).expect("current phase payload");
        assert_eq!(decoded.record_count, 23);

        fs::write(&path, serde_json::to_vec(&progress).unwrap()).unwrap();
        assert!(read_sync_phase_payload::<ProgressFile>(&path, &orchestration).is_err());

        let stale = SyncPhaseEnvelopeV1::new(
            11,
            7,
            SyncPhase::Collecting,
            "fedcba9876543210fedcba9876543210",
            progress,
        )
        .unwrap();
        fs::write(&path, serde_json::to_vec(&stale).unwrap()).unwrap();
        assert_eq!(
            read_sync_phase_payload::<ProgressFile>(&path, &orchestration)
                .err()
                .as_deref(),
            Some("sync_phase_envelope_mismatch")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn running_progress_uses_only_the_current_phase_envelope() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-phase-progress-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).unwrap();
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            11,
            7,
            SyncPhase::Collecting,
            attempt_id,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let running_state = preparing
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running_state).unwrap();
        let payload = serde_json::json!({
            "schemaVersion": 3,
            "accountId": 11,
            "syncJobId": 7,
            "phase": "scrolling",
            "recordCount": 23,
            "elapsedMs": 456,
            "retryCount": 1,
            "canCancel": true
        });
        fs::write(
            paths
                .phase_progress_path(11, 7, SyncPhase::Collecting, attempt_id)
                .unwrap(),
            serde_json::to_vec(
                &SyncPhaseEnvelopeV1::new(11, 7, SyncPhase::Collecting, attempt_id, payload)
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        fs::write(
            paths.progress_path(11, 7),
            br#"{"schemaVersion":3,"accountId":11,"syncJobId":7,"phase":"legacy","recordCount":99,"elapsedMs":1,"retryCount":0,"canCancel":true}"#,
        )
        .unwrap();
        let running = RunningSyncIdentity {
            account_id: 11,
            job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };

        let (lifecycle, progress) = read_running_sync_progress(&paths, &running).unwrap();
        let progress = progress.expect("phase progress");

        assert_eq!(lifecycle, SyncLifecycle::Running);
        assert_eq!(progress.phase, "scrolling");
        assert_eq!(progress.record_count, 23);

        let consuming = running_state
            .next(SyncLifecycle::Consuming, "2026-07-20T00:00:02.000Z")
            .unwrap();
        cas_orchestration(&paths, &running_state, &consuming).unwrap();
        fs::write(
            paths
                .phase_progress_path(11, 7, SyncPhase::Collecting, attempt_id)
                .unwrap(),
            b"{ invalid progress",
        )
        .unwrap();
        let (lifecycle, progress) = read_running_sync_progress(&paths, &running).unwrap();
        assert_eq!(lifecycle, SyncLifecycle::Consuming);
        assert!(progress.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_phase_cleanup_keeps_the_current_task_state() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-stale-cleanup-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).unwrap();
        let old_attempt = "0123456789abcdef0123456789abcdef";
        let current_attempt = "fedcba9876543210fedcba9876543210";
        let current = SyncOrchestrationV1 {
            schema_version: 1,
            revision: 4,
            account_id: 11,
            sync_job_id: 7,
            phase: SyncPhase::RetryingAfterLogin,
            phase_attempt_id: current_attempt.to_string(),
            lifecycle: SyncLifecycle::Running,
            login_recovery_count: 1,
            cancel_requested_at: None,
            options_hash: "a".repeat(64),
            deadline_at_ms: None,
            updated_at: "2026-07-20T00:00:00.000Z".to_string(),
        };
        fs::write(
            paths.orchestration_path(11, 7),
            serde_json::to_vec(&current).unwrap(),
        )
        .unwrap();
        fs::write(paths.cancel_path(11, 7), b"cancel\n").unwrap();
        let old_result = paths
            .phase_result_path(11, 7, SyncPhase::Collecting, old_attempt)
            .unwrap();
        fs::write(&old_result, b"old phase").unwrap();
        let old_identity = RunningSyncIdentity {
            account_id: 11,
            job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: old_attempt.to_string(),
        };

        cleanup_task_files(&paths, &old_identity);

        assert!(!old_result.exists());
        assert!(paths.orchestration_path(11, 7).exists());
        assert!(paths.cancel_path(11, 7).exists());
        let persisted = read_orchestration(&paths, 11, 7).unwrap();
        assert_eq!(persisted.phase, SyncPhase::RetryingAfterLogin);
        assert_eq!(persisted.phase_attempt_id, current_attempt);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn forced_login_cancel_stops_collector_before_browser_and_always_cleans() {
        let events = Mutex::new(Vec::new());

        finish_login_cancellation_with(
            false,
            || {
                events.lock().unwrap().push("collector");
                Ok(())
            },
            || {
                events.lock().unwrap().push("browser");
                Ok(())
            },
            || {
                events.lock().unwrap().push("cleanup");
                Ok(())
            },
        )
        .expect("cancel");

        assert_eq!(
            *events.lock().unwrap(),
            vec!["collector", "browser", "cleanup"]
        );
    }

    #[test]
    fn cooperative_login_cancel_still_terminates_browser_and_cleans() {
        let events = Mutex::new(Vec::new());

        finish_login_cancellation_with(
            true,
            || {
                events.lock().unwrap().push("collector");
                Ok(())
            },
            || {
                events.lock().unwrap().push("browser");
                Ok(())
            },
            || {
                events.lock().unwrap().push("cleanup");
                Ok(())
            },
        )
        .expect("cancel");

        assert_eq!(*events.lock().unwrap(), vec!["browser", "cleanup"]);
    }

    #[test]
    fn durable_login_recovery_stops_collector_then_browser_before_cleanup() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-durable-login-recovery-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let profile_key = "fedcba9876543210fedcba9876543210";
        create_login_attempt_state(
            &paths,
            attempt_id,
            profile_key,
            None,
            "2026-08-13T00:00:00.000Z",
        )
        .unwrap();
        let executable = PathBuf::from(r"C:\Program Files\Emily\Emily.exe");
        let invocation = build_login_invocation(
            &paths,
            executable.clone(),
            None,
            profile_key,
            attempt_id,
            BrowserPreferenceV1::Auto,
        )
        .unwrap();
        let expectation = LoginCollectorProcessExpectation::for_invocation(
            attempt_id,
            profile_key,
            None,
            &invocation,
        )
        .unwrap();
        let collector = LoginCollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4242,
            creation_time_100ns: "133000000000000000".to_string(),
            executable_path: executable.to_string_lossy().to_string(),
            arguments_sha256: expectation.arguments_sha256,
            login_attempt_id: attempt_id.to_string(),
            profile_key: profile_key.to_string(),
            target_account_id: None,
            started_at: "2026-08-13T00:00:01.000Z".to_string(),
        };
        fs::write(
            paths
                .login_task_collector_identity_path(attempt_id)
                .unwrap(),
            serde_json::to_vec(&collector).unwrap(),
        )
        .unwrap();
        let expected_profile =
            douyin_history_archive::collector::browser_state::browser_profile_path(
                &paths.account_browser_root(profile_key).unwrap(),
                BrowserId::Edge,
            );
        let browser = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: 4343,
            creation_time_100ns: "133000000000000001".to_string(),
            browser_id: BrowserId::Edge,
            executable_path: r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".to_string(),
            profile_path: expected_profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!("--user-data-dir={}", expected_profile.display()),
            devtools_port: 9222,
            browser_websocket_path: "/devtools/browser/test".to_string(),
            started_at: "2026-08-13T00:00:02.000Z".to_string(),
        };
        fs::write(
            paths.login_task_browser_pid_path(attempt_id).unwrap(),
            serde_json::to_vec(&browser).unwrap(),
        )
        .unwrap();
        let events = Mutex::new(Vec::new());

        recover_durable_login_attempts_with(
            &paths,
            &executable,
            |record, expectation| {
                assert_eq!(record.login_attempt_id, expectation.login_attempt_id);
                events.lock().unwrap().push("collector");
                Ok(())
            },
            |record, profile| {
                assert_eq!(record.pid, 4343);
                assert_eq!(profile, expected_profile);
                events.lock().unwrap().push("browser");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(*events.lock().unwrap(), vec!["collector", "browser"]);
        assert!(!paths.login_attempt_state_path(attempt_id).unwrap().exists());
        assert!(!paths
            .login_task_collector_identity_path(attempt_id)
            .unwrap()
            .exists());
        assert!(!paths
            .login_task_browser_pid_path(attempt_id)
            .unwrap()
            .exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_login_recovery_keeps_evidence_when_collector_cleanup_fails() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-durable-login-failure-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let profile_key = "fedcba9876543210fedcba9876543210";
        create_login_attempt_state(
            &paths,
            attempt_id,
            profile_key,
            Some(7),
            "2026-08-13T00:00:00.000Z",
        )
        .unwrap();
        let executable = PathBuf::from(r"C:\Program Files\Emily\Emily.exe");
        let invocation = build_login_invocation(
            &paths,
            executable.clone(),
            Some(7),
            profile_key,
            attempt_id,
            BrowserPreferenceV1::Auto,
        )
        .unwrap();
        let expectation = LoginCollectorProcessExpectation::for_invocation(
            attempt_id,
            profile_key,
            Some(7),
            &invocation,
        )
        .unwrap();
        let collector = LoginCollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4242,
            creation_time_100ns: "133000000000000000".to_string(),
            executable_path: executable.to_string_lossy().to_string(),
            arguments_sha256: expectation.arguments_sha256,
            login_attempt_id: attempt_id.to_string(),
            profile_key: profile_key.to_string(),
            target_account_id: Some(7),
            started_at: "2026-08-13T00:00:01.000Z".to_string(),
        };
        let collector_path = paths
            .login_task_collector_identity_path(attempt_id)
            .unwrap();
        fs::write(&collector_path, serde_json::to_vec(&collector).unwrap()).unwrap();
        let browser_called = std::cell::Cell::new(false);

        assert_eq!(
            recover_durable_login_attempts_with(
                &paths,
                &executable,
                |_record, _expectation| Err("collector_cleanup_failed".to_string()),
                |_record, _profile| {
                    browser_called.set(true);
                    Ok(())
                },
            ),
            Err("collector_cleanup_failed".to_string())
        );
        assert!(!browser_called.get());
        assert!(paths.login_attempt_state_path(attempt_id).unwrap().exists());
        assert!(collector_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_login_recovery_rejects_a_self_asserted_argument_hash() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-durable-login-hash-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let profile_key = "fedcba9876543210fedcba9876543210";
        create_login_attempt_state(
            &paths,
            attempt_id,
            profile_key,
            None,
            "2026-08-13T00:00:00.000Z",
        )
        .unwrap();
        let executable = PathBuf::from(r"C:\Program Files\Emily\Emily.exe");
        let collector = LoginCollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4242,
            creation_time_100ns: "133000000000000000".to_string(),
            executable_path: executable.to_string_lossy().to_string(),
            arguments_sha256: "a".repeat(64),
            login_attempt_id: attempt_id.to_string(),
            profile_key: profile_key.to_string(),
            target_account_id: None,
            started_at: "2026-08-13T00:00:01.000Z".to_string(),
        };
        let collector_path = paths
            .login_task_collector_identity_path(attempt_id)
            .unwrap();
        fs::write(&collector_path, serde_json::to_vec(&collector).unwrap()).unwrap();
        let termination_called = std::cell::Cell::new(false);

        assert_eq!(
            recover_durable_login_attempts_with(
                &paths,
                &executable,
                |_record, _expectation| {
                    termination_called.set(true);
                    Ok(())
                },
                |_record, _profile| Ok(()),
            ),
            Err("login_collector_process_identity_mismatch".to_string())
        );
        assert!(!termination_called.get());
        assert!(collector_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_runtime_rejects_root_mismatch_before_database_or_file_mutation() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-uninstall-root-{unique}"));
        let bootstrap_dir = root.join("bootstrap");
        let data_root = root.join("Data");
        let paths = AppPaths::from_roots(bootstrap_dir.clone(), data_root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let trusted_root_id = "0123456789abcdef0123456789abcdef";
        write_bootstrap_atomic(
            &bootstrap_dir,
            &BootstrapV1::new(data_root.clone(), trusted_root_id.to_string()).unwrap(),
        )
        .unwrap();
        write_data_root_marker_atomic(
            &data_root,
            &DataRootMarkerV1::new(trusted_root_id.to_string()).unwrap(),
        )
        .unwrap();
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let queued = enqueue_history_job(&mut connection, account_id, 1_786_420_800_000);
        let evidence = paths.temp.join("uninstall-evidence.keep");
        fs::write(&evidence, b"keep").unwrap();
        let executable = std::env::current_exe().unwrap();
        let product = compiled_product_identity();
        let session = UninstallSessionV1 {
            schema_version: 1,
            authentication: GuardAuthenticationV1 {
                nonce: "abcdef0123456789abcdef0123456789".to_string(),
                installer_pid: 1,
                installer_created_at_100ns: 1,
            },
            root_id: "fedcba9876543210fedcba9876543210".to_string(),
            old_run_value: None,
            old_device_autostart: DeviceAutostartSnapshotV1::default(),
            phase: UninstallPhaseV1::Prepared,
            device_settings_dir: bootstrap_dir.clone(),
            install_dir: executable.parent().unwrap().to_path_buf(),
            main_executable: executable,
            application_id: product.application_id.clone(),
            product_name: product.product_name.clone(),
            old_build_id: product.build_id.clone(),
            old_semantic_version: product.semantic_version.clone(),
            architecture: product.architecture.clone(),
            protected_paths: vec![data_root.clone()],
            user_data_disposition: UserDataDispositionV1::Retain,
            package_manifest: PackageManifestV1 {
                schema_version: 1,
                application_id: product.application_id,
                product_name: product.product_name,
                build_id: product.build_id,
                semantic_version: product.semantic_version,
                architecture: product.architecture,
                files: Vec::new(),
            },
            package_manifest_sha256: "a".repeat(64),
            prepared_at: "2026-08-13T00:00:00.000Z".to_string(),
            updated_at: "2026-08-13T00:00:01.000Z".to_string(),
        };

        assert_eq!(
            prepare_application_runtime_for_uninstall(&session),
            Err("uninstall_runtime_root_mismatch".to_string())
        );
        assert_eq!(
            QueueStore::get_stored_job(&connection, queued.snapshot.id)
                .unwrap()
                .snapshot
                .status,
            SyncJobStatusV1::Queued
        );
        assert!(evidence.is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_runtime_cancels_queued_jobs_before_cleaning_their_task_files() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-uninstall-queued-{unique}"));
        let bootstrap_dir = root.join("bootstrap");
        let data_root = root.join("Data");
        let paths = AppPaths::from_roots(bootstrap_dir.clone(), data_root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let root_id = "0123456789abcdef0123456789abcdef";
        write_bootstrap_atomic(
            &bootstrap_dir,
            &BootstrapV1::new(data_root.clone(), root_id.to_string()).unwrap(),
        )
        .unwrap();
        write_data_root_marker_atomic(
            &data_root,
            &DataRootMarkerV1::new(root_id.to_string()).unwrap(),
        )
        .unwrap();
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let queued = enqueue_history_job(&mut connection, account_id, 1_786_420_800_000);
        let stale_progress = paths.progress_path(account_id, queued.snapshot.id);
        fs::write(&stale_progress, b"stale").unwrap();
        let executable = std::env::current_exe().unwrap();
        let product = compiled_product_identity();
        let session = UninstallSessionV1 {
            schema_version: 1,
            authentication: GuardAuthenticationV1 {
                nonce: "abcdef0123456789abcdef0123456789".to_string(),
                installer_pid: 1,
                installer_created_at_100ns: 1,
            },
            root_id: root_id.to_string(),
            old_run_value: None,
            old_device_autostart: DeviceAutostartSnapshotV1::default(),
            phase: UninstallPhaseV1::Prepared,
            device_settings_dir: bootstrap_dir,
            install_dir: executable.parent().unwrap().to_path_buf(),
            main_executable: executable,
            application_id: product.application_id.clone(),
            product_name: product.product_name.clone(),
            old_build_id: product.build_id.clone(),
            old_semantic_version: product.semantic_version.clone(),
            architecture: product.architecture.clone(),
            protected_paths: vec![data_root],
            user_data_disposition: UserDataDispositionV1::Retain,
            package_manifest: PackageManifestV1 {
                schema_version: 1,
                application_id: product.application_id,
                product_name: product.product_name,
                build_id: product.build_id,
                semantic_version: product.semantic_version,
                architecture: product.architecture,
                files: Vec::new(),
            },
            package_manifest_sha256: "a".repeat(64),
            prepared_at: "2026-08-13T00:00:00.000Z".to_string(),
            updated_at: "2026-08-13T00:00:01.000Z".to_string(),
        };

        prepare_application_runtime_for_uninstall(&session).unwrap();

        let stored = QueueStore::get_stored_job(&connection, queued.snapshot.id).unwrap();
        assert_eq!(stored.snapshot.status, SyncJobStatusV1::Cancelled);
        assert_eq!(
            stored.error_code.as_deref(),
            Some("uninstall_runtime_stopped")
        );
        QueueStore::assert_no_active_jobs(&connection).unwrap();
        assert!(!stale_progress.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_runtime_rejects_a_running_collector_identity_not_derived_from_the_frozen_job() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-uninstall-running-{unique}"));
        let bootstrap_dir = root.join("bootstrap");
        let data_root = root.join("Data");
        let paths = AppPaths::from_roots(bootstrap_dir.clone(), data_root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let root_id = "0123456789abcdef0123456789abcdef";
        write_bootstrap_atomic(
            &bootstrap_dir,
            &BootstrapV1::new(data_root.clone(), root_id.to_string()).unwrap(),
        )
        .unwrap();
        write_data_root_marker_atomic(
            &data_root,
            &DataRootMarkerV1::new(root_id.to_string()).unwrap(),
        )
        .unwrap();
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let running = enqueue_running_history_job(&mut connection, account_id, 1_786_420_800_000);
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            running.snapshot.id,
            SyncPhase::Collecting,
            attempt_id,
            &running.snapshot.task_payload_sha256,
            None,
            "2026-08-13T00:00:00.000Z",
        )
        .unwrap();
        let active = preparing
            .next(SyncLifecycle::Running, "2026-08-13T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &active).unwrap();
        let executable = std::env::current_exe().unwrap();
        let invocation = build_history_collect_invocation(
            &paths,
            executable.clone(),
            &running,
            SyncPhase::Collecting,
            attempt_id,
        )
        .unwrap();
        let expectation = CollectorProcessExpectation::for_invocation(
            account_id,
            running.snapshot.id,
            SyncPhase::Collecting,
            attempt_id,
            &invocation,
        )
        .unwrap();
        let record = CollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4242,
            creation_time_100ns: "133000000000000000".to_string(),
            executable_path: expectation.executable_path,
            arguments_sha256: "a".repeat(64),
            account_id,
            sync_job_id: running.snapshot.id,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
            started_at: "2026-08-13T00:00:02.000Z".to_string(),
        };
        let identity_path = paths
            .phase_collector_identity_path(
                account_id,
                running.snapshot.id,
                SyncPhase::Collecting,
                attempt_id,
            )
            .unwrap();
        fs::write(&identity_path, serde_json::to_vec(&record).unwrap()).unwrap();
        let product = compiled_product_identity();
        let session = UninstallSessionV1 {
            schema_version: 1,
            authentication: GuardAuthenticationV1 {
                nonce: "abcdef0123456789abcdef0123456789".to_string(),
                installer_pid: 1,
                installer_created_at_100ns: 1,
            },
            root_id: root_id.to_string(),
            old_run_value: None,
            old_device_autostart: DeviceAutostartSnapshotV1::default(),
            phase: UninstallPhaseV1::Prepared,
            device_settings_dir: bootstrap_dir,
            install_dir: executable.parent().unwrap().to_path_buf(),
            main_executable: executable,
            application_id: product.application_id.clone(),
            product_name: product.product_name.clone(),
            old_build_id: product.build_id.clone(),
            old_semantic_version: product.semantic_version.clone(),
            architecture: product.architecture.clone(),
            protected_paths: vec![data_root],
            user_data_disposition: UserDataDispositionV1::Retain,
            package_manifest: PackageManifestV1 {
                schema_version: 1,
                application_id: product.application_id,
                product_name: product.product_name,
                build_id: product.build_id,
                semantic_version: product.semantic_version,
                architecture: product.architecture,
                files: Vec::new(),
            },
            package_manifest_sha256: "a".repeat(64),
            prepared_at: "2026-08-13T00:00:00.000Z".to_string(),
            updated_at: "2026-08-13T00:00:01.000Z".to_string(),
        };
        let collector_called = std::cell::Cell::new(false);

        assert_eq!(
            prepare_application_runtime_for_uninstall_with(
                &session,
                |_record, _expectation| Ok(Some(1)),
                |_record, _expectation| Ok(Some(2)),
                |_record, _profile| Ok(Some(3)),
                |_record, _expectation, _token| {
                    panic!("login collector callback should not run")
                },
                |_record, _expectation, _token| {
                    collector_called.set(true);
                    Ok(())
                },
                |_record, _profile, _token| Ok(()),
            ),
            Err("uninstall_runtime_collector_identity_mismatch".to_string())
        );
        assert!(!collector_called.get());
        assert_eq!(
            QueueStore::get_stored_job(&connection, running.snapshot.id)
                .unwrap()
                .snapshot
                .status,
            SyncJobStatusV1::Running
        );
        assert!(identity_path.is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_runtime_stops_a_trusted_running_collector_before_terminalizing_the_job() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-uninstall-running-ok-{unique}"));
        let bootstrap_dir = root.join("bootstrap");
        let data_root = root.join("Data");
        let paths = AppPaths::from_roots(bootstrap_dir.clone(), data_root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let root_id = "0123456789abcdef0123456789abcdef";
        write_bootstrap_atomic(
            &bootstrap_dir,
            &BootstrapV1::new(data_root.clone(), root_id.to_string()).unwrap(),
        )
        .unwrap();
        write_data_root_marker_atomic(
            &data_root,
            &DataRootMarkerV1::new(root_id.to_string()).unwrap(),
        )
        .unwrap();
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let running = enqueue_running_history_job(&mut connection, account_id, 1_786_420_800_000);
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            running.snapshot.id,
            SyncPhase::Collecting,
            attempt_id,
            &running.snapshot.task_payload_sha256,
            None,
            "2026-08-13T00:00:00.000Z",
        )
        .unwrap();
        let active = preparing
            .next(SyncLifecycle::Running, "2026-08-13T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &active).unwrap();
        let executable = std::env::current_exe().unwrap();
        let invocation = build_history_collect_invocation(
            &paths,
            executable.clone(),
            &running,
            SyncPhase::Collecting,
            attempt_id,
        )
        .unwrap();
        let expectation = CollectorProcessExpectation::for_invocation(
            account_id,
            running.snapshot.id,
            SyncPhase::Collecting,
            attempt_id,
            &invocation,
        )
        .unwrap();
        let record = CollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4242,
            creation_time_100ns: "133000000000000000".to_string(),
            executable_path: expectation.executable_path.clone(),
            arguments_sha256: expectation.arguments_sha256.clone(),
            account_id,
            sync_job_id: running.snapshot.id,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
            started_at: "2026-08-13T00:00:02.000Z".to_string(),
        };
        let identity_path = paths
            .phase_collector_identity_path(
                account_id,
                running.snapshot.id,
                SyncPhase::Collecting,
                attempt_id,
            )
            .unwrap();
        fs::write(&identity_path, serde_json::to_vec(&record).unwrap()).unwrap();
        let product = compiled_product_identity();
        let session = UninstallSessionV1 {
            schema_version: 1,
            authentication: GuardAuthenticationV1 {
                nonce: "abcdef0123456789abcdef0123456789".to_string(),
                installer_pid: 1,
                installer_created_at_100ns: 1,
            },
            root_id: root_id.to_string(),
            old_run_value: None,
            old_device_autostart: DeviceAutostartSnapshotV1::default(),
            phase: UninstallPhaseV1::Prepared,
            device_settings_dir: bootstrap_dir,
            install_dir: executable.parent().unwrap().to_path_buf(),
            main_executable: executable,
            application_id: product.application_id.clone(),
            product_name: product.product_name.clone(),
            old_build_id: product.build_id.clone(),
            old_semantic_version: product.semantic_version.clone(),
            architecture: product.architecture.clone(),
            protected_paths: vec![data_root],
            user_data_disposition: UserDataDispositionV1::Retain,
            package_manifest: PackageManifestV1 {
                schema_version: 1,
                application_id: product.application_id,
                product_name: product.product_name,
                build_id: product.build_id,
                semantic_version: product.semantic_version,
                architecture: product.architecture,
                files: Vec::new(),
            },
            package_manifest_sha256: "a".repeat(64),
            prepared_at: "2026-08-13T00:00:00.000Z".to_string(),
            updated_at: "2026-08-13T00:00:01.000Z".to_string(),
        };
        let collector_called = std::cell::Cell::new(false);

        prepare_application_runtime_for_uninstall_with(
            &session,
            |_record, _expectation| Ok(Some(1)),
            |_record, _expectation| Ok(Some(2)),
            |_record, _profile| Ok(Some(3)),
            |_record, _expectation, _token| panic!("login collector callback should not run"),
            |observed, rebuilt, _token| {
                assert_eq!(observed, &record);
                assert_eq!(rebuilt, &expectation);
                assert_eq!(
                    QueueStore::get_stored_job(&connection, running.snapshot.id)
                        .unwrap()
                        .snapshot
                        .status,
                    SyncJobStatusV1::Running
                );
                assert!(identity_path.is_file());
                collector_called.set(true);
                Ok(())
            },
            |_record, _profile, _token| panic!("browser callback should not run"),
        )
        .unwrap();

        assert!(collector_called.get());
        let stored = QueueStore::get_stored_job(&connection, running.snapshot.id).unwrap();
        assert_eq!(stored.snapshot.status, SyncJobStatusV1::Cancelled);
        assert_eq!(
            stored.error_code.as_deref(),
            Some("uninstall_runtime_stopped")
        );
        QueueStore::assert_no_active_jobs(&connection).unwrap();
        assert!(!identity_path.exists());
        assert!(!paths
            .orchestration_path(account_id, running.snapshot.id)
            .exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_runtime_uses_the_frozen_browser_identity_after_the_account_profile_changes() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-uninstall-frozen-browser-{unique}"));
        let bootstrap_dir = root.join("bootstrap");
        let data_root = root.join("Data");
        let paths = AppPaths::from_roots(bootstrap_dir.clone(), data_root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let root_id = "0123456789abcdef0123456789abcdef";
        write_bootstrap_atomic(
            &bootstrap_dir,
            &BootstrapV1::new(data_root.clone(), root_id.to_string()).unwrap(),
        )
        .unwrap();
        write_data_root_marker_atomic(
            &data_root,
            &DataRootMarkerV1::new(root_id.to_string()).unwrap(),
        )
        .unwrap();
        let mut connection = prepare_v5_database(&paths);
        let frozen_profile_key = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let account_id = insert_named_active_account(
            &connection,
            "uid-frozen",
            "sec-frozen",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        let browser_executable = root.join("msedge.exe");
        fs::write(&browser_executable, b"fixture").unwrap();
        let browser_hash = douyin_history_archive::sync::browser_executable_path_sha256(
            &browser_executable.to_string_lossy(),
        );
        let running = enqueue_running_history_job_with_browser(
            &mut connection,
            account_id,
            1_786_420_800_000,
            BrowserExecutionIdentityV1 {
                preference: douyin_history_archive::device_settings::BrowserPreferenceV1::Edge,
                resolved_browser: BrowserIdV1::Edge,
                profile_key: frozen_profile_key.to_string(),
                profile_generation: "cccccccccccccccccccccccccccccccc".to_string(),
                executable_path_sha256: browser_hash,
            },
        );
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            running.snapshot.id,
            SyncPhase::Collecting,
            attempt_id,
            &running.snapshot.task_payload_sha256,
            None,
            "2026-08-13T00:00:00.000Z",
        )
        .unwrap();
        let active = preparing
            .next(SyncLifecycle::Running, "2026-08-13T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &active).unwrap();
        let executable = std::env::current_exe().unwrap();
        let invocation = build_history_collect_invocation(
            &paths,
            executable.clone(),
            &running,
            SyncPhase::Collecting,
            attempt_id,
        )
        .unwrap();
        let expectation = CollectorProcessExpectation::for_invocation(
            account_id,
            running.snapshot.id,
            SyncPhase::Collecting,
            attempt_id,
            &invocation,
        )
        .unwrap();
        let collector = CollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4242,
            creation_time_100ns: "133000000000000000".to_string(),
            executable_path: expectation.executable_path,
            arguments_sha256: expectation.arguments_sha256,
            account_id,
            sync_job_id: running.snapshot.id,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
            started_at: "2026-08-13T00:00:02.000Z".to_string(),
        };
        fs::write(
            paths
                .phase_collector_identity_path(
                    account_id,
                    running.snapshot.id,
                    SyncPhase::Collecting,
                    attempt_id,
                )
                .unwrap(),
            serde_json::to_vec(&collector).unwrap(),
        )
        .unwrap();
        let expected_profile = paths
            .account_browser_root(frozen_profile_key)
            .unwrap()
            .join("edge");
        fs::create_dir_all(&expected_profile).unwrap();
        let browser = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: 4343,
            creation_time_100ns: "133000000000000001".to_string(),
            browser_id: BrowserId::Edge,
            executable_path: browser_executable.to_string_lossy().to_string(),
            profile_path: expected_profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!("--user-data-dir={}", expected_profile.display()),
            devtools_port: 9222,
            browser_websocket_path: "/devtools/browser/fixture".to_string(),
            started_at: "2026-08-13T00:00:02.000Z".to_string(),
        };
        let browser_envelope = SyncPhaseEnvelopeV1::new(
            account_id,
            running.snapshot.id,
            SyncPhase::Collecting,
            attempt_id,
            browser.clone(),
        )
        .unwrap();
        fs::write(
            paths
                .phase_browser_pid_path(
                    account_id,
                    running.snapshot.id,
                    SyncPhase::Collecting,
                    attempt_id,
                )
                .unwrap(),
            serde_json::to_vec(&browser_envelope).unwrap(),
        )
        .unwrap();
        let product = compiled_product_identity();
        let session = UninstallSessionV1 {
            schema_version: 1,
            authentication: GuardAuthenticationV1 {
                nonce: "abcdef0123456789abcdef0123456789".to_string(),
                installer_pid: 1,
                installer_created_at_100ns: 1,
            },
            root_id: root_id.to_string(),
            old_run_value: None,
            old_device_autostart: DeviceAutostartSnapshotV1::default(),
            phase: UninstallPhaseV1::Prepared,
            device_settings_dir: bootstrap_dir,
            install_dir: executable.parent().unwrap().to_path_buf(),
            main_executable: executable,
            application_id: product.application_id.clone(),
            product_name: product.product_name.clone(),
            old_build_id: product.build_id.clone(),
            old_semantic_version: product.semantic_version.clone(),
            architecture: product.architecture.clone(),
            protected_paths: vec![data_root],
            user_data_disposition: UserDataDispositionV1::Retain,
            package_manifest: PackageManifestV1 {
                schema_version: 1,
                application_id: product.application_id,
                product_name: product.product_name,
                build_id: product.build_id,
                semantic_version: product.semantic_version,
                architecture: product.architecture,
                files: Vec::new(),
            },
            package_manifest_sha256: "a".repeat(64),
            prepared_at: "2026-08-13T00:00:00.000Z".to_string(),
            updated_at: "2026-08-13T00:00:01.000Z".to_string(),
        };
        let observed_profile = std::cell::RefCell::new(None);

        prepare_application_runtime_for_uninstall_with(
            &session,
            |_record, _expectation| Ok(Some(1)),
            |_record, _expectation| Ok(Some(2)),
            |_record, _profile| Ok(Some(3)),
            |_record, _expectation, _token| panic!("login collector callback should not run"),
            |_record, _expectation, _token| Ok(()),
            |observed, profile, _token| {
                assert_eq!(observed, &browser);
                observed_profile.replace(Some(profile.to_path_buf()));
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(observed_profile.into_inner(), Some(expected_profile));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_runtime_validates_every_process_identity_before_terminating_any_process() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-uninstall-validate-first-{unique}"));
        let bootstrap_dir = root.join("bootstrap");
        let data_root = root.join("Data");
        let paths = AppPaths::from_roots(bootstrap_dir.clone(), data_root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let root_id = "0123456789abcdef0123456789abcdef";
        write_bootstrap_atomic(
            &bootstrap_dir,
            &BootstrapV1::new(data_root.clone(), root_id.to_string()).unwrap(),
        )
        .unwrap();
        write_data_root_marker_atomic(
            &data_root,
            &DataRootMarkerV1::new(root_id.to_string()).unwrap(),
        )
        .unwrap();
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let browser_executable = root.join("msedge.exe");
        fs::write(&browser_executable, b"fixture").unwrap();
        let running = enqueue_running_history_job_with_browser(
            &mut connection,
            account_id,
            1_786_420_800_000,
            BrowserExecutionIdentityV1 {
                preference: douyin_history_archive::device_settings::BrowserPreferenceV1::Edge,
                resolved_browser: BrowserIdV1::Edge,
                profile_key: "0123456789abcdef0123456789abcdef".to_string(),
                profile_generation: "1123456789abcdef0123456789abcdef".to_string(),
                executable_path_sha256: "d".repeat(64),
            },
        );
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            running.snapshot.id,
            SyncPhase::Collecting,
            attempt_id,
            &running.snapshot.task_payload_sha256,
            None,
            "2026-08-13T00:00:00.000Z",
        )
        .unwrap();
        let active = preparing
            .next(SyncLifecycle::Running, "2026-08-13T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &active).unwrap();
        let executable = std::env::current_exe().unwrap();
        let invocation = build_history_collect_invocation(
            &paths,
            executable.clone(),
            &running,
            SyncPhase::Collecting,
            attempt_id,
        )
        .unwrap();
        let expectation = CollectorProcessExpectation::for_invocation(
            account_id,
            running.snapshot.id,
            SyncPhase::Collecting,
            attempt_id,
            &invocation,
        )
        .unwrap();
        let collector = CollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4242,
            creation_time_100ns: "133000000000000000".to_string(),
            executable_path: expectation.executable_path,
            arguments_sha256: expectation.arguments_sha256,
            account_id,
            sync_job_id: running.snapshot.id,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
            started_at: "2026-08-13T00:00:02.000Z".to_string(),
        };
        fs::write(
            paths
                .phase_collector_identity_path(
                    account_id,
                    running.snapshot.id,
                    SyncPhase::Collecting,
                    attempt_id,
                )
                .unwrap(),
            serde_json::to_vec(&collector).unwrap(),
        )
        .unwrap();
        let expected_profile = paths
            .account_browser_root("0123456789abcdef0123456789abcdef")
            .unwrap()
            .join("edge");
        fs::create_dir_all(&expected_profile).unwrap();
        let browser = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: 4343,
            creation_time_100ns: "133000000000000001".to_string(),
            browser_id: BrowserId::Edge,
            executable_path: browser_executable.to_string_lossy().to_string(),
            profile_path: expected_profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!("--user-data-dir={}", expected_profile.display()),
            devtools_port: 9222,
            browser_websocket_path: "/devtools/browser/fixture".to_string(),
            started_at: "2026-08-13T00:00:02.000Z".to_string(),
        };
        fs::write(
            paths
                .phase_browser_pid_path(
                    account_id,
                    running.snapshot.id,
                    SyncPhase::Collecting,
                    attempt_id,
                )
                .unwrap(),
            serde_json::to_vec(
                &SyncPhaseEnvelopeV1::new(
                    account_id,
                    running.snapshot.id,
                    SyncPhase::Collecting,
                    attempt_id,
                    browser,
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let product = compiled_product_identity();
        let session = UninstallSessionV1 {
            schema_version: 1,
            authentication: GuardAuthenticationV1 {
                nonce: "abcdef0123456789abcdef0123456789".to_string(),
                installer_pid: 1,
                installer_created_at_100ns: 1,
            },
            root_id: root_id.to_string(),
            old_run_value: None,
            old_device_autostart: DeviceAutostartSnapshotV1::default(),
            phase: UninstallPhaseV1::Prepared,
            device_settings_dir: bootstrap_dir,
            install_dir: executable.parent().unwrap().to_path_buf(),
            main_executable: executable,
            application_id: product.application_id.clone(),
            product_name: product.product_name.clone(),
            old_build_id: product.build_id.clone(),
            old_semantic_version: product.semantic_version.clone(),
            architecture: product.architecture.clone(),
            protected_paths: vec![data_root],
            user_data_disposition: UserDataDispositionV1::Retain,
            package_manifest: PackageManifestV1 {
                schema_version: 1,
                application_id: product.application_id,
                product_name: product.product_name,
                build_id: product.build_id,
                semantic_version: product.semantic_version,
                architecture: product.architecture,
                files: Vec::new(),
            },
            package_manifest_sha256: "a".repeat(64),
            prepared_at: "2026-08-13T00:00:00.000Z".to_string(),
            updated_at: "2026-08-13T00:00:01.000Z".to_string(),
        };
        let collector_called = std::cell::Cell::new(false);
        let browser_called = std::cell::Cell::new(false);

        assert_eq!(
            prepare_application_runtime_for_uninstall_with(
                &session,
                |_record, _expectation| Ok(Some(1)),
                |_record, _expectation| Ok(Some(2)),
                |_record, _profile| Ok(Some(3)),
                |_record, _expectation, _token| {
                    panic!("login collector callback should not run")
                },
                |_record, _expectation, _token| {
                    collector_called.set(true);
                    Ok(())
                },
                |_record, _profile, _token| {
                    browser_called.set(true);
                    Ok(())
                },
            ),
            Err("uninstall_runtime_browser_identity_mismatch".to_string())
        );
        assert!(!collector_called.get());
        assert!(!browser_called.get());
        assert_eq!(
            QueueStore::get_stored_job(&connection, running.snapshot.id)
                .unwrap()
                .snapshot
                .status,
            SyncJobStatusV1::Running
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_runtime_preflights_every_os_process_before_terminating_any_process() {
        let profile = PathBuf::from(r"C:\Users\fixture\AppData\Local\Emily\profile\edge");
        let login_expectation = LoginCollectorProcessExpectation {
            login_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            profile_key: "fedcba9876543210fedcba9876543210".to_string(),
            target_account_id: None,
            executable_path: r"C:\Program Files\Emily\Emily.exe".to_string(),
            arguments_sha256: "a".repeat(64),
        };
        let login_collector = LoginCollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4101,
            creation_time_100ns: "133000000000000001".to_string(),
            executable_path: login_expectation.executable_path.clone(),
            arguments_sha256: login_expectation.arguments_sha256.clone(),
            login_attempt_id: login_expectation.login_attempt_id.clone(),
            profile_key: login_expectation.profile_key.clone(),
            target_account_id: None,
            started_at: "2026-08-13T00:00:00.000Z".to_string(),
        };
        let sync_expectation = CollectorProcessExpectation {
            account_id: 7,
            sync_job_id: 11,
            phase: SyncPhase::Collecting,
            phase_attempt_id: "1123456789abcdef0123456789abcdef".to_string(),
            executable_path: r"C:\Program Files\Emily\Emily.exe".to_string(),
            arguments_sha256: "b".repeat(64),
        };
        let sync_collector = CollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4102,
            creation_time_100ns: "133000000000000002".to_string(),
            executable_path: sync_expectation.executable_path.clone(),
            arguments_sha256: sync_expectation.arguments_sha256.clone(),
            account_id: sync_expectation.account_id,
            sync_job_id: sync_expectation.sync_job_id,
            phase: sync_expectation.phase,
            phase_attempt_id: sync_expectation.phase_attempt_id.clone(),
            started_at: "2026-08-13T00:00:01.000Z".to_string(),
        };
        let browser = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: 4103,
            creation_time_100ns: "133000000000000003".to_string(),
            browser_id: BrowserId::Edge,
            executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
                .to_string(),
            profile_path: profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!("--user-data-dir={}", profile.display()),
            devtools_port: 9222,
            browser_websocket_path: "/devtools/browser/fixture".to_string(),
            started_at: "2026-08-13T00:00:02.000Z".to_string(),
        };
        let login_cleanups = vec![TrustedLoginCleanupV1 {
            process: TrustedLoginProcessCleanupV1 {
                collector: Some((login_collector, login_expectation)),
                browser: None,
            },
            files: LoginCleanupFilesV1::Durable {
                attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            },
        }];
        let sync_cleanups = vec![TrustedSyncProcessCleanupV1 {
            collector: sync_collector,
            collector_expectation: sync_expectation,
            browser: Some((browser, profile)),
        }];
        let events = std::cell::RefCell::new(Vec::new());

        assert_eq!(
            stop_trusted_processes_for_uninstall_with(
                &login_cleanups,
                &sync_cleanups,
                |_record, _expectation| {
                    events.borrow_mut().push("preflight-login");
                    Ok(Some(1))
                },
                |_record, _expectation| {
                    events.borrow_mut().push("preflight-sync");
                    Ok(Some(2))
                },
                |_record, _profile| {
                    events.borrow_mut().push("preflight-browser");
                    Err::<Option<u64>, _>(
                        "uninstall_runtime_browser_identity_query_failed".to_string(),
                    )
                },
                |_record, _expectation, _token| {
                    events.borrow_mut().push("terminate-login");
                    Ok(())
                },
                |_record, _expectation, _token| {
                    events.borrow_mut().push("terminate-sync");
                    Ok(())
                },
                |_record, _profile, _token| {
                    events.borrow_mut().push("terminate-browser");
                    Ok(())
                },
            ),
            Err("uninstall_runtime_browser_identity_query_failed".to_string())
        );
        assert_eq!(
            events.into_inner(),
            vec!["preflight-login", "preflight-sync", "preflight-browser"]
        );
    }

    #[test]
    fn uninstall_runtime_does_not_terminate_a_process_that_was_stale_during_preflight() {
        let expectation = CollectorProcessExpectation {
            account_id: 7,
            sync_job_id: 11,
            phase: SyncPhase::Collecting,
            phase_attempt_id: "1123456789abcdef0123456789abcdef".to_string(),
            executable_path: r"C:\Program Files\Emily\Emily.exe".to_string(),
            arguments_sha256: "b".repeat(64),
        };
        let collector = CollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4102,
            creation_time_100ns: "133000000000000002".to_string(),
            executable_path: expectation.executable_path.clone(),
            arguments_sha256: expectation.arguments_sha256.clone(),
            account_id: expectation.account_id,
            sync_job_id: expectation.sync_job_id,
            phase: expectation.phase,
            phase_attempt_id: expectation.phase_attempt_id.clone(),
            started_at: "2026-08-13T00:00:01.000Z".to_string(),
        };
        let sync_cleanups = vec![TrustedSyncProcessCleanupV1 {
            collector,
            collector_expectation: expectation,
            browser: None,
        }];
        let terminated = std::cell::Cell::new(false);

        stop_trusted_processes_for_uninstall_with(
            &[],
            &sync_cleanups,
            |_record, _expectation| Ok(Some(1)),
            |_record, _expectation| Ok(Option::<u64>::None),
            |_record, _profile| Ok(Some(2)),
            |_record, _expectation, _token| Ok(()),
            |_record, _expectation, _token| {
                terminated.set(true);
                Ok(())
            },
            |_record, _profile, _token| Ok(()),
        )
        .unwrap();

        assert!(!terminated.get());
    }

    #[test]
    fn uninstall_runtime_terminates_durable_and_legacy_logins_before_cleaning_files() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-uninstall-logins-{unique}"));
        let bootstrap_dir = root.join("bootstrap");
        let data_root = root.join("Data");
        let paths = AppPaths::from_roots(bootstrap_dir.clone(), data_root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let root_id = "0123456789abcdef0123456789abcdef";
        write_bootstrap_atomic(
            &bootstrap_dir,
            &BootstrapV1::new(data_root.clone(), root_id.to_string()).unwrap(),
        )
        .unwrap();
        write_data_root_marker_atomic(
            &data_root,
            &DataRootMarkerV1::new(root_id.to_string()).unwrap(),
        )
        .unwrap();
        let connection = prepare_v5_database(&paths);
        let legacy_profile_key = "22222222222222222222222222222222";
        let legacy_account_id = insert_named_active_account(
            &connection,
            "uid-legacy-login",
            "sec-legacy-login",
            legacy_profile_key,
        );
        let session = uninstall_session_for(&paths, root_id);

        let durable_attempt_id = "0123456789abcdef0123456789abcdef";
        let durable_profile_key = "11111111111111111111111111111111";
        create_login_attempt_state(
            &paths,
            durable_attempt_id,
            durable_profile_key,
            None,
            "2026-08-13T00:00:00.000Z",
        )
        .unwrap();
        let durable_invocation = build_login_invocation(
            &paths,
            session.main_executable.clone(),
            None,
            durable_profile_key,
            durable_attempt_id,
            BrowserPreferenceV1::Auto,
        )
        .unwrap();
        let durable_expectation = LoginCollectorProcessExpectation::for_invocation(
            durable_attempt_id,
            durable_profile_key,
            None,
            &durable_invocation,
        )
        .unwrap();
        let durable_collector = LoginCollectorProcessIdentityV1 {
            schema_version: 1,
            pid: 4101,
            creation_time_100ns: "133000000000000001".to_string(),
            executable_path: durable_expectation.executable_path.clone(),
            arguments_sha256: durable_expectation.arguments_sha256.clone(),
            login_attempt_id: durable_attempt_id.to_string(),
            profile_key: durable_profile_key.to_string(),
            target_account_id: None,
            started_at: "2026-08-13T00:00:01.000Z".to_string(),
        };
        fs::write(
            paths
                .login_task_collector_identity_path(durable_attempt_id)
                .unwrap(),
            serde_json::to_vec(&durable_collector).unwrap(),
        )
        .unwrap();
        let durable_profile =
            douyin_history_archive::collector::browser_state::browser_profile_path(
                &paths.account_browser_root(durable_profile_key).unwrap(),
                BrowserId::Edge,
            );
        fs::create_dir_all(&durable_profile).unwrap();
        let durable_browser = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: 4102,
            creation_time_100ns: "133000000000000002".to_string(),
            browser_id: BrowserId::Edge,
            executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
                .to_string(),
            profile_path: durable_profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!("--user-data-dir={}", durable_profile.display()),
            devtools_port: 9222,
            browser_websocket_path: "/devtools/browser/durable".to_string(),
            started_at: "2026-08-13T00:00:02.000Z".to_string(),
        };
        fs::write(
            paths
                .login_task_browser_pid_path(durable_attempt_id)
                .unwrap(),
            serde_json::to_vec(&durable_browser).unwrap(),
        )
        .unwrap();
        let durable_files = vec![
            paths.login_attempt_state_path(durable_attempt_id).unwrap(),
            paths.login_task_identity_path(durable_attempt_id).unwrap(),
            paths.login_task_progress_path(durable_attempt_id).unwrap(),
            paths.login_task_cancel_path(durable_attempt_id).unwrap(),
            paths
                .login_task_browser_pid_path(durable_attempt_id)
                .unwrap(),
            paths
                .login_task_collector_identity_path(durable_attempt_id)
                .unwrap(),
            paths
                .login_task_diagnostic_path(durable_attempt_id)
                .unwrap(),
            paths.login_task_log_path(durable_attempt_id).unwrap(),
        ];
        for path in &durable_files {
            if !path.exists() {
                fs::write(path, b"durable-evidence").unwrap();
            }
        }

        let legacy_attempt_id = "fedcba9876543210fedcba9876543210";
        create_login_attempt(
            &paths,
            legacy_account_id,
            legacy_attempt_id,
            legacy_profile_key,
        )
        .unwrap();
        let legacy_profile = douyin_history_archive::collector::browser_state::browser_profile_path(
            &paths.account_browser_root(legacy_profile_key).unwrap(),
            BrowserId::Chrome,
        );
        fs::create_dir_all(&legacy_profile).unwrap();
        let legacy_browser = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: 4103,
            creation_time_100ns: "133000000000000003".to_string(),
            browser_id: BrowserId::Chrome,
            executable_path: r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
            profile_path: legacy_profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!("--user-data-dir={}", legacy_profile.display()),
            devtools_port: 9223,
            browser_websocket_path: "/devtools/browser/legacy".to_string(),
            started_at: "2026-08-13T00:00:03.000Z".to_string(),
        };
        fs::write(
            paths
                .login_browser_pid_path(legacy_account_id, legacy_attempt_id)
                .unwrap(),
            serde_json::to_vec(&legacy_browser).unwrap(),
        )
        .unwrap();
        let legacy_files = vec![
            paths
                .login_attempt_path(legacy_account_id, legacy_attempt_id)
                .unwrap(),
            paths
                .login_identity_path(legacy_account_id, legacy_attempt_id)
                .unwrap(),
            paths
                .login_progress_path(legacy_account_id, legacy_attempt_id)
                .unwrap(),
            paths
                .login_cancel_path(legacy_account_id, legacy_attempt_id)
                .unwrap(),
            paths
                .login_browser_pid_path(legacy_account_id, legacy_attempt_id)
                .unwrap(),
            paths
                .login_diagnostic_path(legacy_account_id, legacy_attempt_id)
                .unwrap(),
            paths
                .login_log_path(legacy_account_id, legacy_attempt_id)
                .unwrap(),
        ];
        for path in &legacy_files {
            if !path.exists() {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"legacy-evidence").unwrap();
            }
        }

        let events = std::cell::RefCell::new(Vec::new());
        prepare_application_runtime_for_uninstall_with(
            &session,
            |record, expectation| {
                assert_eq!(record, &durable_collector);
                assert_eq!(expectation, &durable_expectation);
                events.borrow_mut().push("preflight-durable-collector");
                Ok(Some(record.pid))
            },
            |_record, _expectation| panic!("sync collector preflight should not run"),
            |record, profile| {
                match record.pid {
                    4102 => {
                        assert_eq!(profile, durable_profile);
                        events.borrow_mut().push("preflight-durable-browser");
                    }
                    4103 => {
                        assert_eq!(profile, legacy_profile);
                        events.borrow_mut().push("preflight-legacy-browser");
                    }
                    _ => panic!("unexpected login browser"),
                }
                Ok(Some(record.pid))
            },
            |record, expectation, token| {
                assert_eq!(record, &durable_collector);
                assert_eq!(expectation, &durable_expectation);
                assert_eq!(token, 4101);
                events.borrow_mut().push("terminate-durable-collector");
                Ok(())
            },
            |_record, _expectation, _token: u32| {
                panic!("sync collector termination should not run")
            },
            |record, profile, token| {
                assert_eq!(token, record.pid);
                match record.pid {
                    4102 => {
                        assert_eq!(profile, durable_profile);
                        events.borrow_mut().push("terminate-durable-browser");
                    }
                    4103 => {
                        assert_eq!(profile, legacy_profile);
                        events.borrow_mut().push("terminate-legacy-browser");
                    }
                    _ => panic!("unexpected login browser"),
                }
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            events.into_inner(),
            vec![
                "preflight-durable-collector",
                "preflight-durable-browser",
                "preflight-legacy-browser",
                "terminate-durable-collector",
                "terminate-durable-browser",
                "terminate-legacy-browser",
            ]
        );
        for path in durable_files.into_iter().chain(legacy_files) {
            assert!(
                !path.exists(),
                "login evidence was not removed: {}",
                path.display()
            );
        }
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_runtime_rejects_corrupt_login_identity_before_any_process_callback() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-uninstall-login-corrupt-{unique}"));
        let bootstrap_dir = root.join("bootstrap");
        let data_root = root.join("Data");
        let paths = AppPaths::from_roots(bootstrap_dir.clone(), data_root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let root_id = "0123456789abcdef0123456789abcdef";
        write_bootstrap_atomic(
            &bootstrap_dir,
            &BootstrapV1::new(data_root.clone(), root_id.to_string()).unwrap(),
        )
        .unwrap();
        write_data_root_marker_atomic(
            &data_root,
            &DataRootMarkerV1::new(root_id.to_string()).unwrap(),
        )
        .unwrap();
        let connection = prepare_v5_database(&paths);
        let session = uninstall_session_for(&paths, root_id);

        let valid_attempt_id = "0123456789abcdef0123456789abcdef";
        let valid_profile_key = "11111111111111111111111111111111";
        create_login_attempt_state(
            &paths,
            valid_attempt_id,
            valid_profile_key,
            None,
            "2026-08-13T00:00:00.000Z",
        )
        .unwrap();
        let invocation = build_login_invocation(
            &paths,
            session.main_executable.clone(),
            None,
            valid_profile_key,
            valid_attempt_id,
            BrowserPreferenceV1::Auto,
        )
        .unwrap();
        let expectation = LoginCollectorProcessExpectation::for_invocation(
            valid_attempt_id,
            valid_profile_key,
            None,
            &invocation,
        )
        .unwrap();
        fs::write(
            paths
                .login_task_collector_identity_path(valid_attempt_id)
                .unwrap(),
            serde_json::to_vec(&LoginCollectorProcessIdentityV1 {
                schema_version: 1,
                pid: 4201,
                creation_time_100ns: "133000000000000011".to_string(),
                executable_path: expectation.executable_path,
                arguments_sha256: expectation.arguments_sha256,
                login_attempt_id: valid_attempt_id.to_string(),
                profile_key: valid_profile_key.to_string(),
                target_account_id: None,
                started_at: "2026-08-13T00:00:01.000Z".to_string(),
            })
            .unwrap(),
        )
        .unwrap();

        let corrupt_attempt_id = "fedcba9876543210fedcba9876543210";
        let corrupt_profile_key = "22222222222222222222222222222222";
        create_login_attempt_state(
            &paths,
            corrupt_attempt_id,
            corrupt_profile_key,
            None,
            "2026-08-13T00:00:02.000Z",
        )
        .unwrap();
        fs::write(
            paths
                .login_task_collector_identity_path(corrupt_attempt_id)
                .unwrap(),
            b"not-json",
        )
        .unwrap();

        let callbacks = std::cell::Cell::new(0usize);
        assert_eq!(
            prepare_application_runtime_for_uninstall_with(
                &session,
                |_record, _expectation| {
                    callbacks.set(callbacks.get() + 1);
                    Ok(Some(1u8))
                },
                |_record, _expectation| {
                    callbacks.set(callbacks.get() + 1);
                    Ok(Some(2u8))
                },
                |_record, _profile| {
                    callbacks.set(callbacks.get() + 1);
                    Ok(Some(3u8))
                },
                |_record, _expectation, _token| {
                    callbacks.set(callbacks.get() + 1);
                    Ok(())
                },
                |_record, _expectation, _token| {
                    callbacks.set(callbacks.get() + 1);
                    Ok(())
                },
                |_record, _profile, _token| {
                    callbacks.set(callbacks.get() + 1);
                    Ok(())
                },
            ),
            Err("uninstall_runtime_login_collector_identity_invalid".to_string())
        );
        assert_eq!(callbacks.get(), 0);
        assert!(paths
            .login_attempt_state_path(valid_attempt_id)
            .unwrap()
            .is_file());
        assert!(paths
            .login_attempt_state_path(corrupt_attempt_id)
            .unwrap()
            .is_file());
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_runtime_retries_terminal_job_file_cleanup_on_the_next_attempt() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-uninstall-cleanup-retry-{unique}"));
        let bootstrap_dir = root.join("bootstrap");
        let data_root = root.join("Data");
        let paths = AppPaths::from_roots(bootstrap_dir.clone(), data_root.clone(), PathBuf::new());
        prepare_app_directories(&paths).unwrap();
        let root_id = "0123456789abcdef0123456789abcdef";
        write_bootstrap_atomic(
            &bootstrap_dir,
            &BootstrapV1::new(data_root.clone(), root_id.to_string()).unwrap(),
        )
        .unwrap();
        write_data_root_marker_atomic(
            &data_root,
            &DataRootMarkerV1::new(root_id.to_string()).unwrap(),
        )
        .unwrap();
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let queued = enqueue_history_job(&mut connection, account_id, 1_786_420_800_000);
        let stale_progress = paths.progress_path(account_id, queued.snapshot.id);
        fs::write(&stale_progress, b"stale").unwrap();
        let blocking_directory = paths.temp.join(format!(
            "account-{account_id}-sync-{}-blocking",
            queued.snapshot.id
        ));
        fs::create_dir_all(&blocking_directory).unwrap();
        let session = uninstall_session_for(&paths, root_id);

        assert_eq!(
            prepare_application_runtime_for_uninstall_with(
                &session,
                |_record, _expectation| Ok(Some(1u8)),
                |_record, _expectation| Ok(Some(2u8)),
                |_record, _profile| Ok(Some(3u8)),
                |_record, _expectation, _token| Ok(()),
                |_record, _expectation, _token| Ok(()),
                |_record, _profile, _token| Ok(()),
            ),
            Err("uninstall_runtime_file_cleanup_failed".to_string())
        );
        let terminal = QueueStore::get_stored_job(&connection, queued.snapshot.id).unwrap();
        assert_eq!(terminal.snapshot.status, SyncJobStatusV1::Cancelled);
        assert_eq!(
            terminal.error_code.as_deref(),
            Some("uninstall_runtime_stopped")
        );
        assert!(blocking_directory.is_dir());

        fs::remove_dir(&blocking_directory).unwrap();
        let retry_evidence = paths.temp.join(format!(
            "account-{account_id}-sync-{}-retry.tmp",
            queued.snapshot.id
        ));
        fs::write(&retry_evidence, b"retry").unwrap();
        prepare_application_runtime_for_uninstall_with(
            &session,
            |_record, _expectation| Ok(Some(1u8)),
            |_record, _expectation| Ok(Some(2u8)),
            |_record, _profile| Ok(Some(3u8)),
            |_record, _expectation, _token| Ok(()),
            |_record, _expectation, _token| Ok(()),
            |_record, _profile, _token| Ok(()),
        )
        .unwrap();

        assert!(!retry_evidence.exists());
        assert!(!stale_progress.exists());
        let terminal = QueueStore::get_stored_job(&connection, queued.snapshot.id).unwrap();
        assert_eq!(terminal.snapshot.status, SyncJobStatusV1::Cancelled);
        assert_eq!(
            terminal.error_code.as_deref(),
            Some("uninstall_runtime_stopped")
        );
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn login_browser_termination_without_pid_file_is_a_no_op() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-login-no-pid-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        let mut termination_called = false;

        terminate_recorded_login_browser_with(
            &paths,
            7,
            "abcdef0123456789abcdef0123456789",
            "0123456789abcdef0123456789abcdef",
            |_, _| {
                termination_called = true;
                Ok(())
            },
        )
        .expect("missing pid file should be safe");

        assert!(!termination_called);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recorded_login_browser_that_already_exited_is_a_no_op() {
        let expected_profile = PathBuf::from("C:/profiles/account/browser/edge");
        let record = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: u32::MAX,
            creation_time_100ns: "1".to_string(),
            browser_id: BrowserId::Edge,
            executable_path: "C:/Program Files/Microsoft/Edge/Application/msedge.exe".to_string(),
            profile_path: expected_profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!(
                "--user-data-dir={}",
                expected_profile.to_string_lossy()
            ),
            devtools_port: 9222,
            browser_websocket_path: "/devtools/browser/test".to_string(),
            started_at: "2026-07-12T00:00:00.000Z".to_string(),
        };

        terminate_recorded_browser_v2(&record, &expected_profile)
            .expect("an exited browser should already be clean");
    }

    #[test]
    fn poisoned_login_child_lock_still_cleans_attempt_files() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-login-cancel-poison-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).expect("temp");
        let account_id = 7;
        let profile_key = "abcdef0123456789abcdef0123456789";
        let attempt_id = "0123456789abcdef0123456789abcdef";
        for path in [
            paths.login_attempt_path(account_id, attempt_id).unwrap(),
            paths.login_identity_path(account_id, attempt_id).unwrap(),
            paths.login_cancel_path(account_id, attempt_id).unwrap(),
            paths
                .login_browser_pid_path(account_id, attempt_id)
                .unwrap(),
        ] {
            fs::write(path, b"test").expect("attempt file");
        }
        let child = Arc::new(Mutex::new(
            Command::new("cmd")
                .args(["/c", "exit", "0"])
                .spawn()
                .expect("child"),
        ));
        let poison = Arc::clone(&child);
        let _ = std::thread::spawn(move || {
            let _guard = poison.lock().unwrap();
            panic!("poison child lock");
        })
        .join();

        assert!(cancel_login_process_with(
            &paths,
            account_id,
            profile_key,
            attempt_id,
            &paths.login_cancel_path(account_id, attempt_id).unwrap(),
            &child,
            Duration::ZERO,
            |_, _| Ok(()),
        )
        .is_err());

        for path in [
            paths.login_attempt_path(account_id, attempt_id).unwrap(),
            paths.login_identity_path(account_id, attempt_id).unwrap(),
            paths.login_cancel_path(account_id, attempt_id).unwrap(),
            paths
                .login_browser_pid_path(account_id, attempt_id)
                .unwrap(),
        ] {
            assert!(!path.exists(), "{}", path.display());
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_requires_a_known_active_account() {
        let mut connection = Connection::open_in_memory().expect("db");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");

        assert_eq!(
            require_account_status(&connection, 99, &["active"]).unwrap_err(),
            "account_not_found"
        );
    }

    #[test]
    fn identity_conflict_account_remains_readable_and_can_use_visible_relogin() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate_schema_to_v5_at_startup(&mut connection).unwrap();
        let account_id = insert_active_account(&connection);
        connection
            .execute(
                "UPDATE accounts SET status = 'identity_conflict' WHERE id = ?1",
                [account_id],
            )
            .unwrap();

        assert!(account_can_resume_login("identity_conflict"));
        assert!(require_account_status(
            &connection,
            account_id,
            &["active", "login_expired", "identity_conflict"]
        )
        .is_ok());
        assert_eq!(
            require_account_status(&connection, account_id, &["active"]),
            Err("account_not_available".to_string())
        );
    }

    #[test]
    fn sync_status_rejects_missing_job_for_account() {
        let mut connection = Connection::open_in_memory().expect("db");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");

        let account_id = insert_active_account(&connection);
        assert_eq!(
            load_sync_status_for_account(&connection, account_id, 99).unwrap_err(),
            "sync_job_mismatch"
        );
    }

    #[test]
    fn sync_status_helper_uses_the_supplied_account_snapshot() {
        let mut connection = Connection::open_in_memory().expect("db");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        let first_account = insert_active_account(&connection);
        let second_account = insert_named_active_account(
            &connection,
            "uid-second",
            "sec-second",
            "fedcba9876543210fedcba9876543210",
        );
        let first = enqueue_running_history_job(&mut connection, first_account, 1_786_420_800_000);
        QueueStore::finish_running_job(
            &mut connection,
            first.snapshot.id,
            &douyin_history_archive::sync::TerminalJobUpdateV1 {
                status: SyncJobStatusV1::Success,
                finished_at: "2026-08-12T00:00:02.000Z".to_string(),
                inserted_videos: 4,
                inserted_records: 5,
                error_code: None,
                error_message: None,
            },
        )
        .unwrap();
        let second =
            enqueue_running_history_job(&mut connection, second_account, 1_786_420_800_000);
        QueueStore::finish_running_job(
            &mut connection,
            second.snapshot.id,
            &douyin_history_archive::sync::TerminalJobUpdateV1 {
                status: SyncJobStatusV1::Failed,
                finished_at: "2026-08-12T00:00:03.000Z".to_string(),
                inserted_videos: 8,
                inserted_records: 9,
                error_code: Some("fixture_failure".to_string()),
                error_message: None,
            },
        )
        .unwrap();

        let status = load_sync_status_for_account(&connection, first_account, first.snapshot.id)
            .expect("status for supplied account");

        assert_eq!(status.status, SyncJobStatusV1::Success);
        assert_eq!(status.inserted_videos, 4);
        assert_eq!(status.inserted_watch_records, 5);
    }

    #[test]
    fn sync_status_rejects_a_job_owned_by_another_account() {
        let mut connection = Connection::open_in_memory().expect("db");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        let first_account = insert_active_account(&connection);
        let second_account = insert_named_active_account(
            &connection,
            "uid-second",
            "sec-second",
            "fedcba9876543210fedcba9876543210",
        );
        let job_id = start_sync_job(&mut connection, second_account).expect("job");

        assert_eq!(
            load_sync_status_for_account(&connection, first_account, job_id).unwrap_err(),
            "sync_job_mismatch"
        );
    }

    #[test]
    fn database_allows_only_one_running_sync_job() {
        let mut connection = Connection::open_in_memory().expect("db");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        let first_account = insert_active_account(&connection);
        let second_account = insert_named_active_account(
            &connection,
            "uid-second",
            "sec-second",
            "fedcba9876543210fedcba9876543210",
        );
        start_sync_job(&mut connection, first_account).expect("first job");

        assert_eq!(
            start_sync_job(&mut connection, second_account).unwrap_err(),
            "sync_already_running"
        );
    }

    #[test]
    fn concurrent_database_connections_create_only_one_running_sync_job() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let database = std::env::temp_dir().join(format!("emily-sync-race-{unique}.sqlite"));
        let mut setup = Connection::open(&database).expect("db");
        migrate_schema_to_v5_at_startup(&mut setup).expect("schema");
        let first_account = insert_active_account(&setup);
        let second_account = insert_named_active_account(
            &setup,
            "uid-second",
            "sec-second",
            "fedcba9876543210fedcba9876543210",
        );
        drop(setup);

        let barrier = Arc::new(std::sync::Barrier::new(2));
        let handles = [first_account, second_account].map(|account_id| {
            let database = database.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let mut connection = Connection::open(database).expect("db");
                barrier.wait();
                start_sync_job(&mut connection, account_id)
            })
        });
        let results = handles.map(|handle| handle.join().expect("thread"));

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .collect::<Vec<_>>(),
            vec![&"sync_already_running".to_string()]
        );
        let connection = Connection::open(&database).expect("db");
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sync_jobs WHERE status = 'running'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        drop(connection);
        let _ = fs::remove_file(database);
    }

    #[test]
    fn running_identity_is_copied_as_one_account_job_snapshot() {
        let (paths, child, root) = suspended_test_collector("running-identity");
        let running = Mutex::new(Some(RunningSync {
            _path_lease: None,
            _browser_lease: None,
            paths,
            account_id: 11,
            job_id: 7,
            profile_key: "0123456789abcdef0123456789abcdef".to_string(),
            executable: std::env::current_exe().unwrap(),
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            child: Some(child),
        }));
        assert_eq!(
            running_sync_identity(&running).unwrap(),
            Some(RunningSyncIdentity {
                account_id: 11,
                job_id: 7,
                phase: SyncPhase::Collecting,
                phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            })
        );
        drop(running);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn phased_collector_is_identified_while_suspended_then_resumed_after_registration() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-owned-collector-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).unwrap();
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            11,
            7,
            SyncPhase::Collecting,
            attempt_id,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let identity = RunningSyncIdentity {
            account_id: 11,
            job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };
        let invocation = CollectorInvocation {
            program: PathBuf::from(std::env::var_os("ComSpec").unwrap()),
            args: vec!["/d".to_string(), "/c".to_string(), "exit 0".to_string()],
            current_dir: None,
        };

        let child = spawn_collector(&paths, &identity, invocation).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(paths
            .phase_collector_identity_path(11, 7, SyncPhase::Collecting, attempt_id,)
            .unwrap()
            .is_file());
        assert!(child.try_wait().unwrap().is_none());
        let child = Arc::new(Mutex::new(child));
        let slot = Mutex::new(None);

        register_running_phase(
            &slot,
            &paths,
            &preparing,
            RunningSync {
                _path_lease: None,
                _browser_lease: None,
                paths: paths.clone(),
                account_id: 11,
                job_id: 7,
                profile_key: "0123456789abcdef0123456789abcdef".to_string(),
                executable: std::env::current_exe().unwrap(),
                phase: SyncPhase::Collecting,
                phase_attempt_id: attempt_id.to_string(),
                child: Some(Arc::clone(&child)),
            },
        )
        .unwrap();

        assert!(wait_for_child(&child).unwrap().success());
        assert_eq!(
            read_orchestration(&paths, 11, 7).unwrap().lifecycle,
            SyncLifecycle::Running
        );
        drop(slot);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn poisoned_running_slot_returns_to_the_startup_cleanup_path() {
        let slot = Arc::new(Mutex::new(None));
        let poison = Arc::clone(&slot);
        let _ = std::thread::spawn(move || {
            let _guard = poison.lock().unwrap();
            panic!("poison running slot");
        })
        .join();
        let (paths, child, root) = suspended_test_collector("poisoned-running-slot");
        let running = RunningSync {
            _path_lease: None,
            _browser_lease: None,
            paths: paths.clone(),
            account_id: 11,
            job_id: 7,
            profile_key: "0123456789abcdef0123456789abcdef".to_string(),
            executable: std::env::current_exe().unwrap(),
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            child: Some(child),
        };
        let orchestration = SyncOrchestrationV1 {
            schema_version: 1,
            revision: 0,
            account_id: 11,
            sync_job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            lifecycle: SyncLifecycle::Preparing,
            login_recovery_count: 0,
            cancel_requested_at: None,
            options_hash: "a".repeat(64),
            deadline_at_ms: None,
            updated_at: "2026-07-20T00:00:00.000Z".to_string(),
        };
        assert_eq!(
            register_running_phase(&slot, &paths, &orchestration, running).unwrap_err(),
            "同步状态锁已损坏"
        );
        let _ = fs::remove_dir_all(root);
    }

    fn sync_status_fixture(account_id: i64, job_id: i64, status: SyncJobStatusV1) -> SyncStatusV2 {
        SyncStatusV2 {
            account_id,
            job_id,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
            trigger: SyncTriggerV1::Manual,
            status,
            queue_position: None,
            inserted_videos: 0,
            inserted_records: 0,
            inserted_watch_records: 0,
            record_count: 0,
            elapsed_ms: 0,
            retry_count: 0,
            phase: None,
            can_cancel: false,
            started_at: None,
            finished_at: None,
            error_code: None,
            error_message: None,
            issues: Vec::new(),
        }
    }

    #[test]
    fn running_progress_does_not_block_a_queued_job_for_another_account() {
        let mut status = sync_status_fixture(12, 8, SyncJobStatusV1::Queued);
        status.queue_position = Some(2);
        status.can_cancel = true;
        status.inserted_videos = 2;
        status.inserted_records = 3;
        status.inserted_watch_records = 3;
        status.record_count = 3;
        status.elapsed_ms = 4;
        let expected = status.clone();
        let progress = ProgressFile {
            schema_version: 3,
            account_id: 11,
            sync_job_id: 7,
            phase: "scrolling".into(),
            record_count: 99,
            elapsed_ms: 100,
            retry_count: 1,
            can_cancel: true,
        };
        let running = RunningSyncIdentity {
            account_id: 11,
            job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        };
        status_with_running_progress(
            &mut status,
            (12, 8),
            &running,
            SyncLifecycle::Running,
            Some(progress),
        )
        .unwrap();

        assert_eq!(status, expected);
    }

    #[test]
    fn running_progress_does_not_block_another_queued_job_for_the_same_account() {
        let mut status = sync_status_fixture(11, 6, SyncJobStatusV1::Queued);
        status.queue_position = Some(1);
        status.can_cancel = true;
        let expected = status.clone();
        let progress = ProgressFile {
            schema_version: 3,
            account_id: 11,
            sync_job_id: 7,
            phase: "scrolling".into(),
            record_count: 99,
            elapsed_ms: 100,
            retry_count: 1,
            can_cancel: true,
        };

        let running = RunningSyncIdentity {
            account_id: 11,
            job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        };
        status_with_running_progress(
            &mut status,
            (11, 6),
            &running,
            SyncLifecycle::Running,
            Some(progress),
        )
        .unwrap();

        assert_eq!(status, expected);
    }

    #[test]
    fn consuming_status_disables_cancel_even_with_stale_progress() {
        let mut status = sync_status_fixture(11, 7, SyncJobStatusV1::Running);
        status.can_cancel = true;
        let running = RunningSyncIdentity {
            account_id: 11,
            job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        };
        let progress = ProgressFile {
            schema_version: 3,
            account_id: 11,
            sync_job_id: 7,
            phase: "scrolling".into(),
            record_count: 99,
            elapsed_ms: 100,
            retry_count: 1,
            can_cancel: true,
        };

        status_with_running_progress(
            &mut status,
            (11, 7),
            &running,
            SyncLifecycle::Consuming,
            Some(progress),
        )
        .unwrap();

        assert_eq!(status.phase.as_deref(), Some("finalizing"));
        assert!(!status.can_cancel);
    }

    #[test]
    fn collector_spawn_failure_finishes_the_started_job() {
        let mut connection = Connection::open_in_memory().expect("db");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        let account_id = insert_active_account(&connection);
        let job_id = start_sync_job(&mut connection, account_id).expect("job");

        let result =
            fail_started_sync(&connection, account_id, job_id, "runtime_missing").expect("failure");

        assert_eq!(result.status, "failed");
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM sync_jobs WHERE id = ?1",
                    [job_id],
                    |row| row.get::<_, String>(0),
                )
                .expect("status"),
            "failed"
        );
    }

    #[test]
    fn newly_started_sync_returns_the_actual_active_task_status() {
        for (status, expected) in [
            (SyncJobStatusV1::Queued, "queued"),
            (SyncJobStatusV1::Running, "running"),
        ] {
            let result = crate::active_sync_result(11, 7, status).expect("active result");

            assert_eq!(result.account_id, 11);
            assert_eq!(result.job_id, 7);
            assert_eq!(result.status, expected);
            assert_eq!(result.inserted_videos, 0);
            assert_eq!(result.inserted_watch_records, 0);
            assert!(result.message.is_none());
        }
    }

    #[test]
    fn sync_job_updates_reject_wrong_identity_and_non_running_status() {
        let mut connection = Connection::open_in_memory().expect("db");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        let account_id = insert_active_account(&connection);
        let job_id = start_sync_job(&mut connection, account_id).expect("job");

        assert_eq!(
            crate::mark_sync_job(
                &connection,
                account_id + 1,
                job_id,
                "failed",
                Some("wrong account")
            )
            .unwrap_err(),
            "sync_job_mismatch"
        );
        assert_eq!(
            crate::mark_sync_job(
                &connection,
                account_id,
                job_id + 1,
                "failed",
                Some("wrong job")
            )
            .unwrap_err(),
            "sync_job_mismatch"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM sync_jobs WHERE id = ?1",
                    [job_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "running"
        );

        crate::mark_sync_job(&connection, account_id, job_id, "success", None).expect("finish");
        assert_eq!(
            crate::mark_sync_job(
                &connection,
                account_id,
                job_id,
                "failed",
                Some("late failure")
            )
            .unwrap_err(),
            "sync_job_mismatch"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM sync_jobs WHERE id = ?1",
                    [job_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "success"
        );
    }

    #[test]
    fn result_decode_failure_finishes_the_started_job() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-finalize-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::from("C:/unused-resources"));
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let job_id = start_sync_job(&mut connection, account_id).expect("job");
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            attempt_id,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let running = preparing
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running).unwrap();
        fs::write(
            paths
                .phase_result_path(account_id, job_id, SyncPhase::Collecting, attempt_id)
                .unwrap(),
            b"{ invalid json",
        )
        .expect("invalid result");
        let identity = RunningSyncIdentity {
            account_id,
            job_id,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };

        assert!(finalize_sync(&paths, &identity, true).is_err());

        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM sync_jobs WHERE id = ?1",
                    [job_id],
                    |row| row.get::<_, String>(0),
                )
                .expect("status"),
            "failed"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn phase_finalize_imports_only_the_current_attempt_after_consuming_cas() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-phase-finalize-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::from("C:/unused-resources"));
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let frozen_at_ms = 1_783_742_400_000;
        let job = enqueue_running_history_job(&mut connection, account_id, frozen_at_ms);
        let job_id = job.snapshot.id;
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            attempt_id,
            &job.snapshot.task_payload_sha256,
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let running = preparing
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running).unwrap();
        let result = finish_result(
            account_id,
            job_id,
            frozen_at_ms,
            CollectorRange::Bottom,
            HistoryTimeUnit::Milliseconds,
            Vec::new(),
        )
        .unwrap();
        let envelope = SyncExecutionEnvelopeV1 {
            schema_version: 1,
            account_id,
            job_id,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
            task_payload_sha256: job.snapshot.task_payload_sha256,
            result: SourceExecutionResultV1::History(result),
        };
        fs::write(
            paths
                .phase_result_path(account_id, job_id, SyncPhase::Collecting, attempt_id)
                .unwrap(),
            serde_json::to_vec(&envelope).unwrap(),
        )
        .unwrap();
        let identity = RunningSyncIdentity {
            account_id,
            job_id,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };

        let finalized = finalize_sync(&paths, &identity, true).expect("phase finalize");

        assert_eq!(finalized.status, "success");
        assert_eq!(
            read_orchestration(&paths, account_id, job_id)
                .unwrap()
                .lifecycle,
            SyncLifecycle::Consuming
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM sync_jobs WHERE id = ?1",
                    [job_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "success"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn phase_finalize_routes_likes_through_the_archive_importer() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-likes-finalize-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::from("C:/unused-resources"));
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let job = enqueue_running_likes_job(&mut connection, account_id, 1_786_420_800_000);
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            job.snapshot.id,
            SyncPhase::Collecting,
            attempt_id,
            &job.snapshot.task_payload_sha256,
            None,
            "2026-08-12T00:00:00.000Z",
        )
        .unwrap();
        let running = preparing
            .next(SyncLifecycle::Running, "2026-08-12T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running).unwrap();
        let result_path = paths
            .phase_result_path(
                account_id,
                job.snapshot.id,
                SyncPhase::Collecting,
                attempt_id,
            )
            .unwrap();
        fs::write(
            &result_path,
            serde_json::to_vec(&likes_envelope(&job, "7100000000000000001")).unwrap(),
        )
        .unwrap();
        let identity = RunningSyncIdentity {
            account_id,
            job_id: job.snapshot.id,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };

        let finalized = finalize_sync(&paths, &identity, true).expect("likes finalize");

        assert_eq!(finalized.status, "success");
        assert_eq!(finalized.inserted_watch_records, 0);
        assert_eq!(
            connection
                .query_row(
                    "SELECT status, inserted_records FROM sync_jobs WHERE id = ?1",
                    [job.snapshot.id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap(),
            ("success".to_string(), 1)
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM user_likes WHERE account_id = ?1",
                    [account_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn persisted_cancel_wins_before_phase_result_consumption() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-phase-cancel-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::from("C:/unused-resources"));
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let job_id = start_sync_job(&mut connection, account_id).expect("job");
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            attempt_id,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let running = preparing
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running).unwrap();
        request_orchestration_cancel(&paths, &running, "2026-07-20T00:00:02.000Z").unwrap();
        let result = finish_result(
            account_id,
            job_id,
            1_783_742_400_000,
            CollectorRange::Bottom,
            HistoryTimeUnit::Milliseconds,
            Vec::new(),
        )
        .unwrap();
        fs::write(
            paths
                .phase_result_path(account_id, job_id, SyncPhase::Collecting, attempt_id)
                .unwrap(),
            serde_json::to_vec(
                &SyncPhaseEnvelopeV1::new(
                    account_id,
                    job_id,
                    SyncPhase::Collecting,
                    attempt_id,
                    result,
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let identity = RunningSyncIdentity {
            account_id,
            job_id,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };

        let finalized = finalize_sync(&paths, &identity, true).expect("cancelled finalize");

        assert_eq!(finalized.status, "cancelled");
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM sync_jobs WHERE id = ?1",
                    [job_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "cancelled"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM watch_records", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            read_orchestration(&paths, account_id, job_id)
                .unwrap()
                .lifecycle,
            SyncLifecycle::Running
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn consuming_gate_makes_late_cancel_report_finalizing() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-consuming-gate-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).unwrap();
        let mut connection = open_database(&paths.database).unwrap();
        migrate_schema_to_v5_at_startup(&mut connection).unwrap();
        let account_id = insert_active_account(&connection);
        let job_id = start_sync_job(&mut connection, account_id).unwrap();
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            attempt_id,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let running = preparing
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running).unwrap();
        let identity = RunningSyncIdentity {
            account_id,
            job_id,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };

        let gate = linearize_sync_finalization(&paths, &identity).unwrap();

        assert!(matches!(gate, SyncFinalizationGate::Consuming(_)));
        assert_eq!(
            request_sync_cancellation(&paths, &identity, "2026-07-20T00:00:02.000Z",).unwrap_err(),
            "sync_finalizing"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_cancel_survives_task_cancel_file_write_failure() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-cancel-write-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).unwrap();
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            11,
            7,
            SyncPhase::Collecting,
            attempt_id,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let running = preparing
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running).unwrap();
        fs::create_dir_all(paths.cancel_path(11, 7)).unwrap();
        let identity = RunningSyncIdentity {
            account_id: 11,
            job_id: 7,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };

        request_sync_cancellation(&paths, &identity, "2026-07-20T00:00:02.000Z")
            .expect("durable cancellation");

        assert_eq!(
            read_orchestration(&paths, 11, 7)
                .unwrap()
                .cancel_requested_at
                .as_deref(),
            Some("2026-07-20T00:00:02.000Z")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_recovery_lists_only_interrupted_jobs_before_evidence_check() {
        let mut connection = Connection::open_in_memory().expect("db");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        let account_id = insert_active_account(&connection);
        connection
            .execute(
                "INSERT INTO sync_jobs
                 (id, account_id, source, source_instance_key, trigger, queued_at, queue_seq,
                  settings_revision, payload_json, payload_sha256, equivalence_sha256,
                  started_at, status)
                 VALUES (1, ?1, 'history', '', 'manual', 'x', 1, 1, '{}', ?2, ?2,
                         'x', 'running')",
                params![account_id, "a".repeat(64)],
            )
            .expect("running job");
        connection
            .execute(
                "INSERT INTO sync_jobs
                 (id, account_id, source, source_instance_key, trigger, queued_at,
                  started_at, finished_at, status)
                 VALUES (2, ?1, 'history', '', 'manual', 'x', 'x', 'y', 'success')",
                [account_id],
            )
            .expect("finished job");

        let recovered = recover_interrupted_sync_jobs(&connection).expect("recover");

        assert_eq!(recovered, vec![(account_id, 1)]);
        assert_eq!(
            connection
                .query_row("SELECT status FROM sync_jobs WHERE id = 1", [], |row| {
                    row.get::<_, String>(0)
                },)
                .expect("recovered status"),
            "running"
        );
        assert_eq!(
            connection
                .query_row("SELECT status FROM sync_jobs WHERE id = 2", [], |row| {
                    row.get::<_, String>(0)
                },)
                .expect("finished status"),
            "success"
        );
    }

    #[test]
    fn startup_recovery_consumes_only_current_collect_or_retry_results_in_consuming() {
        for phase in [
            SyncPhase::Collecting,
            SyncPhase::RecoveringLogin,
            SyncPhase::RetryingAfterLogin,
        ] {
            for lifecycle in [
                SyncLifecycle::Preparing,
                SyncLifecycle::Running,
                SyncLifecycle::Consuming,
            ] {
                let label = format!("startup-matrix-{}-{lifecycle:?}", phase.as_str());
                let (root, paths, connection, account_id, job_id) =
                    startup_recovery_fixture(&label);
                let orchestration = write_startup_orchestration(
                    &paths,
                    account_id,
                    job_id,
                    phase,
                    lifecycle,
                    "0123456789abcdef0123456789abcdef",
                    false,
                );
                if phase != SyncPhase::RecoveringLogin {
                    write_empty_startup_result(&paths, &orchestration);
                }

                recover_interrupted_job_with(&paths, account_id, job_id, |_, _| Ok(())).unwrap();

                let status = connection
                    .query_row(
                        "SELECT status FROM sync_jobs WHERE id = ?1",
                        [job_id],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap();
                let expected = if lifecycle == SyncLifecycle::Consuming
                    && phase != SyncPhase::RecoveringLogin
                {
                    "success"
                } else {
                    "failed"
                };
                assert_eq!(status, expected, "{phase:?} {lifecycle:?}");
                let _ = fs::remove_dir_all(root);
            }
        }
    }

    #[test]
    fn startup_recovery_consumes_a_strict_likes_envelope_in_consuming() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-likes-recovery-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::from("C:/unused-resources"));
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let job = enqueue_running_likes_job(&mut connection, account_id, 1_786_420_800_000);
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let orchestration = write_startup_orchestration(
            &paths,
            account_id,
            job.snapshot.id,
            SyncPhase::Collecting,
            SyncLifecycle::Consuming,
            attempt_id,
            false,
        );
        let result_path = paths
            .phase_result_path(
                account_id,
                job.snapshot.id,
                orchestration.phase,
                &orchestration.phase_attempt_id,
            )
            .unwrap();
        fs::write(
            &result_path,
            serde_json::to_vec(&likes_envelope(&job, "7100000000000000002")).unwrap(),
        )
        .unwrap();

        recover_interrupted_job_with(&paths, account_id, job.snapshot.id, |_, _| Ok(()))
            .expect("likes recovery");

        assert_eq!(
            connection
                .query_row(
                    "SELECT status, inserted_records FROM sync_jobs WHERE id = ?1",
                    [job.snapshot.id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap(),
            ("success".to_string(), 1)
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM user_likes WHERE account_id = ?1",
                    [account_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert!(!result_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_disposition_controls_running_fallback_without_a_result() {
        for (label, disposition, expected) in [
            (
                "startup-explicit-preserve",
                douyin_history_archive::startup_recovery::StartupRecoveryDispositionV1::PreserveQueued,
                "failed",
            ),
            (
                "startup-explicit-cancel",
                douyin_history_archive::startup_recovery::StartupRecoveryDispositionV1::CancelQueued,
                "cancelled",
            ),
        ] {
            let (root, paths, connection, account_id, job_id) =
                startup_recovery_fixture(label);

            recover_interrupted_job_with_disposition(
                &paths,
                account_id,
                job_id,
                disposition,
                |_, _| Ok(()),
            )
            .expect("recovery");

            assert_eq!(
                connection
                    .query_row(
                        "SELECT status FROM sync_jobs WHERE id = ?1",
                        [job_id],
                        |row| row.get::<_, String>(0),
                    )
                    .expect("status"),
                expected
            );
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn startup_recovery_honors_cancel_before_a_running_result() {
        let (root, paths, connection, account_id, job_id) =
            startup_recovery_fixture("startup-cancel-before-result");
        let orchestration = write_startup_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            SyncLifecycle::Running,
            "0123456789abcdef0123456789abcdef",
            true,
        );
        write_empty_startup_result(&paths, &orchestration);

        recover_interrupted_job_with(&paths, account_id, job_id, |_, _| Ok(())).unwrap();

        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM sync_jobs WHERE id = ?1",
                    [job_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "cancelled"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_recovery_ignores_a_stale_prior_attempt_result() {
        let (root, paths, connection, account_id, job_id) =
            startup_recovery_fixture("startup-stale-result");
        let current_attempt = "0123456789abcdef0123456789abcdef";
        let stale_attempt = "fedcba9876543210fedcba9876543210";
        write_startup_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            SyncLifecycle::Consuming,
            current_attempt,
            false,
        );
        let stale_result = finish_result(
            account_id,
            job_id,
            1_783_742_400_000,
            CollectorRange::Bottom,
            HistoryTimeUnit::Milliseconds,
            Vec::new(),
        )
        .unwrap();
        let stale_path = paths
            .phase_result_path(account_id, job_id, SyncPhase::Collecting, stale_attempt)
            .unwrap();
        fs::write(
            &stale_path,
            serde_json::to_vec(
                &SyncPhaseEnvelopeV1::new(
                    account_id,
                    job_id,
                    SyncPhase::Collecting,
                    stale_attempt,
                    stale_result,
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();

        recover_interrupted_job_with(&paths, account_id, job_id, |_, _| Ok(())).unwrap();

        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM sync_jobs WHERE id = ?1",
                    [job_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "failed"
        );
        assert!(!stale_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_recovery_never_starts_retry_from_a_recovery_marker() {
        let (root, paths, connection, account_id, job_id) =
            startup_recovery_fixture("startup-recovery-marker");
        let attempt_id = "0123456789abcdef0123456789abcdef";
        write_startup_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::RecoveringLogin,
            SyncLifecycle::Running,
            attempt_id,
            false,
        );
        let account_root = paths
            .account_browser_root("0123456789abcdef0123456789abcdef")
            .unwrap();
        let executable = root.join("msedge.exe");
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let choice = douyin_history_archive::collector::browser_runtime::BrowserChoice {
            browser_id: BrowserId::Edge,
            executable_path: executable,
            profile_path: account_root.join("edge"),
        };
        douyin_history_archive::collector::browser_runtime::persist_validation_with_id(
            &account_root,
            &choice,
            douyin_history_archive::collector::session_probe::DOUYIN_HISTORY_URL,
            Some(attempt_id),
        )
        .unwrap();

        recover_interrupted_job_with(&paths, account_id, job_id, |_, _| Ok(())).unwrap();

        assert_eq!(
            connection
                .query_row(
                    "SELECT status, error_code FROM sync_jobs WHERE id = ?1",
                    [job_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .unwrap(),
            ("failed".to_string(), "collector_interrupted".to_string())
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM accounts WHERE id = ?1",
                    [account_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "active"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM sync_jobs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        let selection = read_browser_selection(&account_root)
            .unwrap()
            .expect("browser selection");
        let marker = read_login_validation(&account_root, &selection)
            .unwrap()
            .expect("login validation");
        assert_eq!(marker.validation_id.as_deref(), Some(attempt_id));
        assert!(!paths.orchestration_path(account_id, job_id).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_recovery_rejects_legacy_results_without_valid_orchestration() {
        for corrupt in [false, true] {
            let (root, paths, connection, account_id, job_id) =
                startup_recovery_fixture(if corrupt {
                    "startup-corrupt-orchestration"
                } else {
                    "startup-missing-orchestration"
                });
            let result = finish_result(
                account_id,
                job_id,
                1_783_742_400_000,
                CollectorRange::Bottom,
                HistoryTimeUnit::Milliseconds,
                Vec::new(),
            )
            .unwrap();
            fs::write(
                paths.result_path(account_id, job_id),
                serde_json::to_vec(&result).unwrap(),
            )
            .unwrap();
            if corrupt {
                fs::write(paths.orchestration_path(account_id, job_id), b"{not-json").unwrap();
            }
            let expected_profile = paths
                .account_browser_root("0123456789abcdef0123456789abcdef")
                .unwrap()
                .join("edge");
            fs::create_dir_all(&expected_profile).unwrap();
            let legacy_browser = BrowserProcessRecordV2 {
                schema_version: 2,
                pid: 4321,
                creation_time_100ns: "123".into(),
                browser_id: BrowserId::Edge,
                executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
                    .into(),
                profile_path: expected_profile.to_string_lossy().to_string(),
                user_data_dir_argument: format!("--user-data-dir={}", expected_profile.display()),
                devtools_port: 9222,
                browser_websocket_path: "/devtools/browser/test".into(),
                started_at: "2026-07-11T00:00:00.000Z".into(),
            };
            fs::write(
                paths.browser_pid_path(account_id, job_id),
                serde_json::to_vec(&legacy_browser).unwrap(),
            )
            .unwrap();
            let mut terminated_legacy = false;

            recover_interrupted_job_with(&paths, account_id, job_id, |_, _| {
                terminated_legacy = true;
                Ok(())
            })
            .unwrap();

            assert!(!terminated_legacy, "corrupt={corrupt}");
            assert_eq!(
                connection
                    .query_row(
                        "SELECT status FROM sync_jobs WHERE id = ?1",
                        [job_id],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                "failed"
            );
            let _ = fs::remove_dir_all(root);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn startup_recovery_terminates_the_exact_current_collector() {
        let (root, paths, _, account_id, job_id) =
            startup_recovery_fixture("startup-current-collector");
        let orchestration = write_startup_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            SyncLifecycle::Running,
            "0123456789abcdef0123456789abcdef",
            false,
        );
        let identity = RunningSyncIdentity {
            account_id,
            job_id,
            phase: orchestration.phase,
            phase_attempt_id: orchestration.phase_attempt_id.clone(),
        };
        let invocation = CollectorInvocation {
            program: PathBuf::from(std::env::var_os("ComSpec").unwrap()),
            args: vec![
                "/d".to_string(),
                "/c".to_string(),
                "ping -n 30 127.0.0.1 >nul".to_string(),
            ],
            current_dir: None,
        };
        let mut child = spawn_collector(&paths, &identity, invocation).unwrap();
        child.resume().unwrap();

        recover_interrupted_job_with(&paths, account_id, job_id, |_, _| Ok(())).unwrap();

        assert!(child.try_wait().unwrap().is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn startup_recovery_does_not_terminate_a_collector_with_reused_identity() {
        let (root, paths, _, account_id, job_id) =
            startup_recovery_fixture("startup-reused-collector");
        let orchestration = write_startup_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            SyncLifecycle::Running,
            "0123456789abcdef0123456789abcdef",
            false,
        );
        let identity = RunningSyncIdentity {
            account_id,
            job_id,
            phase: orchestration.phase,
            phase_attempt_id: orchestration.phase_attempt_id.clone(),
        };
        let invocation = CollectorInvocation {
            program: PathBuf::from(std::env::var_os("ComSpec").unwrap()),
            args: vec![
                "/d".to_string(),
                "/c".to_string(),
                "ping -n 30 127.0.0.1 >nul".to_string(),
            ],
            current_dir: None,
        };
        let mut child = spawn_collector(&paths, &identity, invocation).unwrap();
        child.resume().unwrap();
        let collector_path = paths
            .phase_collector_identity_path(
                account_id,
                job_id,
                orchestration.phase,
                &orchestration.phase_attempt_id,
            )
            .unwrap();
        let mut record: CollectorProcessIdentityV1 = read_json(&collector_path).unwrap();
        record.creation_time_100ns.push('1');
        fs::write(&collector_path, serde_json::to_vec(&record).unwrap()).unwrap();

        recover_interrupted_job_with(&paths, account_id, job_id, |_, _| Ok(())).unwrap();

        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_recovery_does_not_consume_a_running_phase_result() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-phase-recovery-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::from("C:/unused-resources"));
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let job_id = start_sync_job(&mut connection, account_id).expect("job");
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            attempt_id,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let running = preparing
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running).unwrap();
        let result = finish_result(
            account_id,
            job_id,
            1_783_742_400_000,
            CollectorRange::Bottom,
            HistoryTimeUnit::Milliseconds,
            Vec::new(),
        )
        .unwrap();
        let result_path = paths
            .phase_result_path(account_id, job_id, SyncPhase::Collecting, attempt_id)
            .unwrap();
        fs::write(
            &result_path,
            serde_json::to_vec(
                &SyncPhaseEnvelopeV1::new(
                    account_id,
                    job_id,
                    SyncPhase::Collecting,
                    attempt_id,
                    result,
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let expected_profile = paths
            .account_browser_root("0123456789abcdef0123456789abcdef")
            .unwrap()
            .join("edge");
        fs::create_dir_all(&expected_profile).unwrap();
        let record = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: 4321,
            creation_time_100ns: "123".into(),
            browser_id: BrowserId::Edge,
            executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
            profile_path: expected_profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!("--user-data-dir={}", expected_profile.display()),
            devtools_port: 9222,
            browser_websocket_path: "/devtools/browser/test".into(),
            started_at: "2026-07-11T00:00:00.000Z".into(),
        };
        fs::write(
            paths
                .phase_browser_pid_path(account_id, job_id, SyncPhase::Collecting, attempt_id)
                .unwrap(),
            serde_json::to_vec(
                &SyncPhaseEnvelopeV1::new(
                    account_id,
                    job_id,
                    SyncPhase::Collecting,
                    attempt_id,
                    record,
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let mut terminated = false;

        recover_interrupted_job_with(&paths, account_id, job_id, |_, _| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT status FROM sync_jobs WHERE id = ?1",
                        [job_id],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                "running"
            );
            assert!(result_path.exists());
            terminated = true;
            Ok(())
        })
        .expect("recover interrupted phase");

        assert!(terminated);
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM sync_jobs WHERE id = ?1",
                    [job_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "failed"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn browser_operations_are_mutually_exclusive() {
        let state = AppState::default();
        let first = begin_browser_operation(&state, BrowserOperationKind::Login, None)
            .expect("first operation");
        assert!(begin_browser_operation(&state, BrowserOperationKind::Login, None).is_err());
        drop(first);
        assert!(begin_browser_operation(&state, BrowserOperationKind::Login, None).is_ok());
    }

    #[test]
    fn ordinary_commands_are_blocked_until_normal_runtime_is_ready() {
        let state = AppState::default();
        assert_eq!(
            require_normal_runtime(&state).expect_err("startup gate"),
            "startup_not_ready"
        );
        state
            .normal_runtime_ready
            .store(true, std::sync::atomic::Ordering::Release);
        require_normal_runtime(&state).expect("runtime ready");
    }

    #[test]
    fn startup_recovery_status_exposes_only_durable_faults_as_resolvable() {
        let failed = startup_recovery_status(
            false,
            StartupRecoveryStateV1::NotStarted,
            Some("bootstrap_invalid".to_string()),
        );
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.error_code.as_deref(), Some("bootstrap_invalid"));
        assert!(!failed.can_resolve);
        assert!(!failed.can_retry);

        let blocked = startup_recovery_status(
            false,
            StartupRecoveryStateV1::Blocked,
            Some("startup_recovery_blocked".to_string()),
        );
        assert_eq!(blocked.state, "blocked");
        assert_eq!(
            blocked.error_code.as_deref(),
            Some("startup_recovery_blocked")
        );
        assert!(blocked.can_resolve);
        assert!(!blocked.can_retry);

        let ready = startup_recovery_status(
            true,
            StartupRecoveryStateV1::Blocked,
            Some("startup_recovery_blocked".to_string()),
        );
        assert_eq!(ready.state, "ready");
        assert_eq!(ready.error_code, None);
        assert!(!ready.can_resolve);
        assert!(!ready.can_retry);

        let retryable = startup_recovery_status(
            false,
            StartupRecoveryStateV1::Ready,
            Some("device_settings_invalid".to_string()),
        );
        assert_eq!(retryable.state, "failed");
        assert_eq!(
            retryable.error_code.as_deref(),
            Some("device_settings_invalid")
        );
        assert!(!retryable.can_resolve);
        assert!(retryable.can_retry);
    }

    #[test]
    fn queued_sync_status_reports_queue_identity_and_position() {
        let mut connection = Connection::open_in_memory().expect("db");
        migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        let account_id = insert_active_account(&connection);
        let queued = enqueue_history_job(&mut connection, account_id, 1_786_420_800_000);

        let status = load_sync_status_for_account(&connection, account_id, queued.snapshot.id)
            .expect("queued status");

        assert_eq!(status.status, SyncJobStatusV1::Queued);
        assert_eq!(status.source, SyncSourceV1::History);
        assert_eq!(status.source_instance_key, "");
        assert_eq!(status.trigger, SyncTriggerV1::Manual);
        assert_eq!(status.queue_position, Some(1));
        assert!(status.can_cancel);
    }

    #[test]
    fn runtime_registry_registers_every_available_source_for_execution_and_import() {
        let registry = test_sync_registry();
        for source in [
            SyncSourceV1::History,
            SyncSourceV1::Likes,
            SyncSourceV1::Favorites,
            SyncSourceV1::RemoteFolder,
        ] {
            assert!(registry.executor(source).is_some(), "executor {source:?}");
            assert!(registry.importer(source).is_some(), "importer {source:?}");
        }
    }

    #[test]
    fn completed_collect_phase_recognizes_the_strict_execution_envelope() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-strict-envelope-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        let mut connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let job_id = start_sync_job(&mut connection, account_id).unwrap();
        let task_payload_sha256 = QueueStore::get_stored_job(&connection, job_id)
            .unwrap()
            .snapshot
            .task_payload_sha256;
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            job_id,
            SyncPhase::Collecting,
            attempt_id,
            &task_payload_sha256,
            None,
            "2026-08-12T00:00:00.000Z",
        )
        .unwrap();
        let running = preparing
            .next(SyncLifecycle::Running, "2026-08-12T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running).unwrap();
        let result = finish_result(
            account_id,
            job_id,
            1_786_420_800_000,
            CollectorRange::Bottom,
            HistoryTimeUnit::Milliseconds,
            Vec::new(),
        )
        .unwrap();
        let envelope = SyncExecutionEnvelopeV1 {
            schema_version: 1,
            account_id,
            job_id,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
            task_payload_sha256,
            result: SourceExecutionResultV1::History(result),
        };
        let result_path = paths
            .phase_result_path(account_id, job_id, SyncPhase::Collecting, attempt_id)
            .unwrap();
        fs::write(&result_path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        let identity = RunningSyncIdentity {
            account_id,
            job_id,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };

        read_sync_execution_envelope(&paths, &result_path, &running)
            .expect("strict execution envelope");
        let observed = observe_phase_completion(&paths, &identity, true).unwrap();

        assert_eq!(observed.decision, PhaseCompletionDecision::Consume);
        let _ = fs::remove_dir_all(root);
    }

    fn prepare_test_browser_state(paths: &AppPaths, profile_key: &str) {
        let account_root = paths.account_root(profile_key).unwrap();
        ensure_account_profile_root(&account_root).unwrap();
        let generation = ensure_profile_generation(&account_root, BrowserId::Edge).unwrap();
        let executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let selection = BrowserSelection {
            schema_version: 3,
            browser_id: BrowserId::Edge,
            executable_path: executable.clone(),
            profile_path: String::new(),
            profile_generation: Some(generation.clone()),
            selected_at: "2026-08-12T00:00:00.000Z".to_string(),
        };
        write_browser_selection(&account_root, &selection).unwrap();
        write_login_validation(
            &account_root,
            &selection,
            &LoginValidation {
                schema_version: 4,
                validation_id: Some("2123456789abcdef0123456789abcdef".to_string()),
                browser_id: BrowserId::Edge,
                executable_path: executable,
                profile_path: String::new(),
                profile_generation: Some(generation),
                validated_at: "2026-08-12T00:00:00.000Z".to_string(),
                history_url: "https://www.douyin.com/".to_string(),
            },
        )
        .unwrap();
    }

    #[test]
    fn running_sync_owns_the_browser_lease_until_terminal_cleanup() {
        let (paths, child, root) = suspended_test_collector("browser-lease-lifecycle");
        let state = AppState::default();
        let lease = state
            .operation_admission
            .begin_browser_operation(BrowserOperationKind::Sync, Some(11))
            .expect("sync browser lease");
        *state.running.lock().unwrap() = Some(RunningSync {
            _path_lease: None,
            _browser_lease: Some(lease),
            paths,
            account_id: 11,
            job_id: 7,
            profile_key: "0123456789abcdef0123456789abcdef".to_string(),
            executable: std::env::current_exe().unwrap(),
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            child: Some(child),
        });

        assert_eq!(
            state
                .operation_admission
                .begin_browser_operation(BrowserOperationKind::Login, Some(12))
                .unwrap_err(),
            "browser_operation_in_progress"
        );
        state.running.lock().unwrap().take();
        assert!(state
            .operation_admission
            .begin_browser_operation(BrowserOperationKind::Login, Some(12))
            .is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn browser_operation_conflict_preserves_the_account_error_contract() {
        let state = AppState::default();
        let _operation = begin_browser_operation(&state, BrowserOperationKind::Login, Some(11))
            .expect("reserve operation");

        assert_eq!(
            begin_browser_operation(&state, BrowserOperationKind::Login, Some(11))
                .err()
                .as_deref(),
            Some("account_operation_in_progress")
        );
        assert_eq!(
            begin_browser_operation(&state, BrowserOperationKind::Login, Some(12))
                .err()
                .as_deref(),
            Some("已有登录或同步操作正在运行")
        );
    }

    #[test]
    fn account_mutation_rejects_the_account_with_a_running_sync() {
        let (paths, child, root) = suspended_test_collector("account-mutation");
        let state = AppState::default();
        *state.running.lock().unwrap() = Some(RunningSync {
            _path_lease: None,
            _browser_lease: None,
            paths,
            account_id: 11,
            job_id: 7,
            profile_key: "0123456789abcdef0123456789abcdef".to_string(),
            executable: std::env::current_exe().unwrap(),
            phase: SyncPhase::Collecting,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            child: Some(child),
        });

        assert_eq!(
            crate::ensure_account_not_running(&state, 11).unwrap_err(),
            "account_operation_in_progress"
        );
        assert_eq!(crate::ensure_account_not_running(&state, 12), Ok(()));
        drop(state);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovered_jobs_terminate_recorded_browser_before_deleting_pid_file() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-browser-recovery-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::from("C:/unused-resources"));
        let connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let expected_profile = paths
            .account_browser_root("0123456789abcdef0123456789abcdef")
            .unwrap()
            .join("edge");
        fs::create_dir_all(&expected_profile).unwrap();
        let record = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: 4321,
            creation_time_100ns: "123".into(),
            browser_id: BrowserId::Edge,
            executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
            profile_path: expected_profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!("--user-data-dir={}", expected_profile.display()),
            devtools_port: 9222,
            browser_websocket_path: "/devtools/browser/test".into(),
            started_at: "2026-07-11T00:00:00.000Z".into(),
        };
        fs::write(
            paths.browser_pid_path(account_id, 9),
            serde_json::to_vec(&record).unwrap(),
        )
        .expect("pid");
        let mut terminated = None;

        cleanup_recovered_job_with(&paths, account_id, 9, |record, profile| {
            terminated = Some((record.pid, profile.to_path_buf()));
            Ok(())
        });

        assert_eq!(terminated, Some((4321, expected_profile)));
        assert!(!paths.browser_pid_path(account_id, 9).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovered_job_cleanup_removes_every_phase_attempt_file() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-recovery-cleanup-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::new());
        fs::create_dir_all(&paths.temp).unwrap();
        fs::create_dir_all(paths.diagnostics.join("account-11")).unwrap();
        fs::create_dir_all(paths.logs.join("account-11")).unwrap();
        let current_attempt = "0123456789abcdef0123456789abcdef";
        let stale_attempt = "fedcba9876543210fedcba9876543210";
        let current = SyncOrchestrationV1 {
            schema_version: 1,
            revision: 1,
            account_id: 11,
            sync_job_id: 7,
            phase: SyncPhase::RetryingAfterLogin,
            phase_attempt_id: current_attempt.to_string(),
            lifecycle: SyncLifecycle::Running,
            login_recovery_count: 1,
            cancel_requested_at: None,
            options_hash: "a".repeat(64),
            deadline_at_ms: None,
            updated_at: "2026-07-20T00:00:00.000Z".to_string(),
        };
        fs::write(
            paths.orchestration_path(11, 7),
            serde_json::to_vec(&current).unwrap(),
        )
        .unwrap();
        fs::write(paths.cancel_path(11, 7), b"cancel\n").unwrap();
        let current_result = paths
            .phase_result_path(11, 7, SyncPhase::RetryingAfterLogin, current_attempt)
            .unwrap();
        let stale_progress = paths
            .phase_progress_path(11, 7, SyncPhase::Collecting, stale_attempt)
            .unwrap();
        let stale_diagnostic = paths
            .phase_diagnostic_path(11, 7, SyncPhase::Collecting, stale_attempt)
            .unwrap();
        let stale_log = paths
            .phase_log_path(11, 7, SyncPhase::Collecting, stale_attempt)
            .unwrap();
        for path in [
            &current_result,
            &stale_progress,
            &stale_diagnostic,
            &stale_log,
        ] {
            fs::write(path, b"phase artifact").unwrap();
        }

        cleanup_recovered_job_files(&paths, 11, 7);

        for path in [
            current_result,
            stale_progress,
            stale_diagnostic,
            stale_log,
            paths.cancel_path(11, 7),
            paths.orchestration_path(11, 7),
        ] {
            assert!(!path.exists(), "{}", path.display());
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancellation_terminates_only_the_requested_account_job_browser() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emily-browser-cancel-{unique}"));
        let paths = AppPaths::new(root.clone(), PathBuf::from("C:/unused-resources"));
        let connection = prepare_v5_database(&paths);
        let account_id = insert_active_account(&connection);
        let expected_profile = paths
            .account_browser_root("0123456789abcdef0123456789abcdef")
            .unwrap()
            .join("edge");
        fs::create_dir_all(&expected_profile).unwrap();
        let record = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: 4321,
            creation_time_100ns: "123".into(),
            browser_id: BrowserId::Edge,
            executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
            profile_path: expected_profile.to_string_lossy().to_string(),
            user_data_dir_argument: format!("--user-data-dir={}", expected_profile.display()),
            devtools_port: 9222,
            browser_websocket_path: "/devtools/browser/test".into(),
            started_at: "2026-07-11T00:00:00.000Z".into(),
        };
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let preparing = create_orchestration(
            &paths,
            account_id,
            9,
            SyncPhase::Collecting,
            attempt_id,
            &"a".repeat(64),
            None,
            "2026-07-20T00:00:00.000Z",
        )
        .unwrap();
        let running = preparing
            .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
            .unwrap();
        cas_orchestration(&paths, &preparing, &running).unwrap();
        let envelope =
            SyncPhaseEnvelopeV1::new(account_id, 9, SyncPhase::Collecting, attempt_id, &record)
                .unwrap();
        let current_pid_path = paths
            .phase_browser_pid_path(account_id, 9, SyncPhase::Collecting, attempt_id)
            .unwrap();
        fs::write(&current_pid_path, serde_json::to_vec(&envelope).unwrap()).expect("account pid");
        let stale_attempt = "fedcba9876543210fedcba9876543210";
        let stale_envelope =
            SyncPhaseEnvelopeV1::new(account_id, 9, SyncPhase::Collecting, stale_attempt, &record)
                .unwrap();
        let stale_pid_path = paths
            .phase_browser_pid_path(account_id, 9, SyncPhase::Collecting, stale_attempt)
            .unwrap();
        fs::write(
            &stale_pid_path,
            serde_json::to_vec(&stale_envelope).unwrap(),
        )
        .expect("stale pid");
        let mut terminated = None;
        let identity = RunningSyncIdentity {
            account_id,
            job_id: 9,
            phase: SyncPhase::Collecting,
            phase_attempt_id: attempt_id.to_string(),
        };

        terminate_recorded_sync_browser_with(&paths, &identity, |record, profile| {
            terminated = Some((record.pid, profile.to_path_buf()));
            Ok(())
        });

        assert_eq!(terminated, Some((4321, expected_profile)));
        assert!(stale_pid_path.exists());
        let _ = fs::remove_dir_all(root);
    }
}

fn hidden_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn cover_cache_protocol_response(
    app: &AppHandle,
    request: &tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let response = || -> Result<tauri::http::Response<Vec<u8>>, String> {
        let method = request.method();
        if method != tauri::http::Method::GET && method != tauri::http::Method::HEAD {
            return Err("cover_cache_method_not_allowed".to_string());
        }
        let path = request.uri().path().trim_matches('/');
        if path.is_empty() || path.contains('/') || !path.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("invalid_cover_video_id".to_string());
        }
        let video_id = path
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| "invalid_cover_video_id".to_string())?;
        let state = app.state::<AppState>();
        let paths = state.acquire_paths()?;
        let connection = open_database(&paths.database)?;
        assert_schema_latest(&connection)?;
        let asset = read_cached_cover(&connection, &paths.covers, video_id, &now_iso())?;
        tauri::http::Response::builder()
            .status(tauri::http::StatusCode::OK)
            .header(tauri::http::header::CONTENT_TYPE, asset.mime_type)
            .header(tauri::http::header::CACHE_CONTROL, "private, max-age=300")
            .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(if method == tauri::http::Method::HEAD {
                Vec::new()
            } else {
                asset.bytes
            })
            .map_err(|error| error.to_string())
    };
    response().unwrap_or_else(|error| {
        let status = match error.as_str() {
            "cover_cache_missing" => tauri::http::StatusCode::NOT_FOUND,
            "cover_cache_method_not_allowed" => tauri::http::StatusCode::METHOD_NOT_ALLOWED,
            "invalid_cover_video_id" => tauri::http::StatusCode::BAD_REQUEST,
            _ => tauri::http::StatusCode::INTERNAL_SERVER_ERROR,
        };
        tauri::http::Response::builder()
            .status(status)
            .header(tauri::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(error.into_bytes())
            .expect("static cover cache protocol response")
    })
}

fn main() {
    use douyin_history_archive::uninstall::{parse_early_process_mode, EarlyProcessModeV1};

    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let early_mode = match parse_early_process_mode(&arguments) {
        Ok(mode) => mode,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    match early_mode {
        EarlyProcessModeV1::Collector(arguments) => {
            let result = douyin_history_archive::collector::cli::parse_collector_args(arguments)
                .and_then(|command| {
                    tokio::runtime::Runtime::new()
                        .map_err(|error| error.to_string())?
                        .block_on(douyin_history_archive::collector::runner::run_command(
                            command,
                        ))
                });
            if let Err(error) = result {
                eprintln!(
                    "{}",
                    douyin_history_archive::collector::errors::describe_collector_error(&error)
                        .message
                );
                std::process::exit(1);
            }
            return;
        }
        EarlyProcessModeV1::DownloadResolver(arguments) => {
            let result = douyin_history_archive::download::parse_resolver_child_args(arguments)
                .and_then(|options| {
                    tokio::runtime::Runtime::new()
                        .map_err(|error| error.to_string())?
                        .block_on(douyin_history_archive::download::run_resolver_child(options))
                });
            if let Err(error) = result {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return;
        }
        EarlyProcessModeV1::UninstallManagedData(bootstrap_dir) => {
            if let Err(error) = uninstall_managed_data(&bootstrap_dir) {
                eprintln!("{}", error.code());
                std::process::exit(1);
            }
            return;
        }
        EarlyProcessModeV1::UninstallGuard(_) => {
            if let Err(error) = run_uninstall_guard(&arguments[1..]) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return;
        }
        EarlyProcessModeV1::InstallerControl(command) => {
            match run_installer_control_with(
                command,
                &NativeUninstallSystem,
                &ApplicationInstallerControlRuntime::default(),
            ) {
                Ok(output) => println!("{output}"),
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            }
            return;
        }
        EarlyProcessModeV1::PackageManifestBuild(arguments) => {
            if let Err(error) = run_package_manifest_builder(&arguments) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return;
        }
        EarlyProcessModeV1::Gui => {}
    }

    let started_by_autostart = arguments
        .iter()
        .any(|argument| argument.to_string_lossy() == "--autostart");
    let activation_event = match GuiActivationEvent::create_application() {
        Ok(event) => event,
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    };
    let _gui_instance = match GuiInstanceGuard::acquire_application() {
        Ok(guard) => guard,
        Err(error) if error == "gui_instance_already_running" => {
            match second_instance_decision(started_by_autostart) {
                SecondInstanceDecisionV1::SignalExisting => {
                    if GuiActivationEvent::open_application()
                        .and_then(|event| event.signal())
                        .is_err()
                    {
                        eprintln!("desktop_activation_failed");
                    }
                }
                SecondInstanceDecisionV1::ExitSilently => {}
            }
            return;
        }
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::default())
        .register_uri_scheme_protocol("cover-cache", |context, request| {
            cover_cache_protocol_response(context.app_handle(), &request)
        })
        .setup(move |app| {
            let local_data_dir = app
                .path()
                .local_data_dir()
                .map_err(|error| error.to_string())?;
            let coordination_dir = uninstall_coordination_dir(&local_data_dir)?;
            let current_executable = std::env::current_exe().map_err(|error| error.to_string())?;
            let install_dir = current_executable
                .parent()
                .ok_or_else(|| "invalid_current_executable".to_string())?
                .to_path_buf();
            let state = app.state::<AppState>();
            initialize_sync_registry(app.handle(), state.inner())?;
            state
                .started_by_autostart
                .set(started_by_autostart)
                .map_err(|_| "startup_mode_already_initialized".to_string())?;
            *state
                .activation_event
                .lock()
                .map_err(|_| "gui_activation_event_poisoned".to_string())? = Some(activation_event);
            let executable_identity = ExecutableIdentityV1 {
                install_dir,
                main_executable: current_executable,
                product: compiled_product_identity(),
            };
            match recover_desktop_startup_runtime(
                app.handle(),
                state.inner(),
                &coordination_dir,
                &executable_identity,
            ) {
                Ok(Some(permit)) => {
                    if let Err(error) =
                        continue_desktop_startup_runtime(app.handle(), state.inner(), permit)
                    {
                        state.record_startup_error(Some(error));
                        apply_startup_window_decision(
                            app.handle(),
                            StartupWindowDecisionV1::ShowRecovery,
                        )?;
                    }
                }
                Ok(None) => {
                    state.record_startup_error(Some("startup_recovery_blocked".to_string()));
                    apply_startup_window_decision(
                        app.handle(),
                        StartupWindowDecisionV1::ShowRecovery,
                    )?;
                }
                Err(error) => {
                    state.record_startup_error(Some(error));
                    apply_startup_window_decision(
                        app.handle(),
                        StartupWindowDecisionV1::ShowRecovery,
                    )?;
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_history,
            list_content,
            list_library_content,
            search_library_content,
            get_overview,
            get_overview_scope,
            get_annual_review,
            open_content_url,
            list_remote_folders,
            list_local_folders,
            create_local_folder,
            rename_local_folder,
            set_local_folder_scope,
            delete_local_folder,
            add_local_folder_item,
            remove_local_folder_item,
            list_remote_folder_content,
            list_local_folder_content,
            get_sync_status,
            enqueue_sync_tasks,
            enqueue_workspace_sync,
            get_sync_batch,
            list_sync_batches,
            list_task_summaries,
            cancel_sync_batch,
            list_sync_batch_entries,
            list_notifications,
            mark_notification_read,
            mark_all_notifications_read,
            resolve_notification,
            get_scheduler_state,
            list_scheduler_instances,
            update_account_defaults,
            commit_onboarding_step,
            set_autostart,
            set_browser_preference,
            begin_account_login,
            cancel_account_login,
            resume_account_login,
            list_accounts,
            get_active_account,
            set_active_account,
            get_app_settings,
            update_global_settings,
            update_workspace_settings,
            update_account_settings,
            import_legacy_settings,
            get_cover_cache_task,
            pause_cover_cache_task,
            resume_cover_cache_task,
            cancel_cover_cache_task,
            retry_cover_cache_task,
            inspect_cover_cache,
            list_cover_cache_rules,
            search_cover_cache_candidates,
            add_cover_cache_rule,
            remove_cover_cache_rule,
            clear_cover_cache_rules,
            preview_cover_cache_cleanup,
            run_cover_cache_cleanup,
            cleanup_broken_cover_cache,
            choose_backup_directory,
            set_backup_directory,
            choose_backup_export_path,
            export_backup,
            choose_backup_import_path,
            inspect_backup_import,
            restore_backup_replace,
            prepare_backup_merge,
            restore_backup_merge,
            download_create_job,
            download_preview_selection,
            download_list_jobs,
            download_get_job,
            download_pause_job,
            download_resume_job,
            download_cancel_job,
            download_retry_failed,
            download_reveal_path,
            choose_data_root,
            validate_data_root,
            set_data_root,
            initialize_onboarding_data_root,
            delete_retained_data_root,
            cancel_pending_account,
            restore_account,
            archive_account,
            delete_account,
            delete_history_records,
            delete_history_by_query,
            start_sync,
            cancel_sync,
            get_startup_recovery_status,
            resolve_startup_recovery_fault,
            get_desktop_state,
            take_desktop_intents,
            set_scheduler_paused,
            get_update_state,
            check_for_updates,
            download_update,
            install_update,
            request_app_exit
        ])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                handle_main_window_close(window.app_handle(), api);
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let state = app.state::<AppState>();
                match state
                    .exit_request
                    .on_exit_requested(state.normal_runtime_ready.load(Ordering::Acquire))
                {
                    Ok(ExitRequestDecisionV1::PreventAndStart) => {
                        api.prevent_exit();
                        start_controlled_exit(app.clone());
                    }
                    Ok(ExitRequestDecisionV1::Prevent) | Err(_) => api.prevent_exit(),
                    Ok(ExitRequestDecisionV1::Allow) => {}
                }
            }
        });
}
