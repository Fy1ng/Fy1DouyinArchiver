use douyin_history_archive::collector::contracts::{LoginProgressReason, LoginProgressV1};
use douyin_history_archive::device_settings::BrowserPreferenceV1;
use douyin_history_archive::login_watchdog::{LoginTermination, LoginWatchdog, LoginWatchdogEvent};
use douyin_history_archive::sync_driver::{
    phase_completion_decision, prepare_next_phase, startup_recovery_decision,
    PhaseCompletionDecision, PhaseCompletionEvidence, StartupRecoveryDecision,
};
use douyin_history_archive::sync_orchestration::{
    cas_orchestration, create_orchestration, read_orchestration, request_orchestration_cancel,
    SyncLifecycle, SyncOrchestrationV1, SyncPhase, SyncPhaseEnvelopeV1,
};
use douyin_history_archive::uninstall::{parse_early_process_mode, EarlyProcessModeV1};
use douyin_history_archive::{
    browser_process_matches, build_login_invocation, build_phase_collect_invocation,
    build_recover_login_invocation, parse_browser_process_record, AppPaths, BrowserProcessInfo,
    SyncOptions,
};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const INSTALLER_HOOKS: &str = include_str!("../nsis/installer-hooks.nsh");
const MAIN_SOURCE: &str = include_str!("../src/main.rs");
const TAURI_CONFIG: &str = include_str!("../tauri.conf.json");

fn rust_function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source.find(signature).expect("function signature");
    let body_start = source[start..].find('{').expect("function body") + start;
    let mut depth = 0usize;
    for (offset, character) in source[body_start..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[body_start..=body_start + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated function body: {signature}");
}

#[test]
fn updater_plugin_has_a_deserializable_inert_base_configuration() {
    let config: serde_json::Value = serde_json::from_str(TAURI_CONFIG).expect("tauri config");
    let updater = config
        .pointer("/plugins/updater")
        .cloned()
        .expect("plugins.updater must exist before plugin initialization");
    let updater: tauri_plugin_updater::Config =
        serde_json::from_value(updater).expect("plugins.updater must deserialize");

    assert!(updater.pubkey.is_empty());
    assert!(updater.endpoints.is_empty());
}

#[test]
fn task_15_commands_and_scheduler_lifecycle_are_wired_into_the_gui_runtime() {
    for command in [
        "enqueue_sync_tasks",
        "enqueue_workspace_sync",
        "get_sync_batch",
        "list_sync_batch_entries",
        "get_scheduler_state",
        "list_scheduler_instances",
        "update_account_defaults",
        "commit_onboarding_step",
        "set_autostart",
        "set_browser_preference",
        "choose_backup_directory",
        "set_backup_directory",
        "choose_backup_export_path",
        "export_backup",
        "choose_backup_import_path",
        "inspect_backup_import",
        "restore_backup_replace",
        "prepare_backup_merge",
        "restore_backup_merge",
        "download_create_job",
        "download_preview_selection",
        "download_list_jobs",
        "download_get_job",
        "download_pause_job",
        "download_resume_job",
        "download_cancel_job",
        "download_retry_failed",
        "download_reveal_path",
        "get_update_state",
        "check_for_updates",
        "download_update",
        "install_update",
    ] {
        assert!(
            MAIN_SOURCE.contains(&format!("            {command},")),
            "missing command registration: {command}"
        );
    }

    assert!(
        MAIN_SOURCE.contains(".plugin(tauri_plugin_updater::Builder::new().build())"),
        "native updater plugin must be registered in the desktop runtime"
    );

    let startup = MAIN_SOURCE
        .split_once("fn continue_desktop_startup_runtime")
        .expect("startup function")
        .1;
    let worker_stage = startup
        .split_once("StartupStageV1::Worker =>")
        .expect("worker stage")
        .1;
    let sync_start = worker_stage
        .find("initialize_sync_runtime")
        .expect("sync worker");
    let cover_cache_start = worker_stage
        .find("initialize_cover_cache_runtime")
        .expect("cover cache worker");
    let scheduler_start = worker_stage
        .find("initialize_scheduler_runtime")
        .expect("scheduler worker");
    let automatic_backup_start = worker_stage
        .find("initialize_automatic_backup_runtime")
        .expect("automatic backup worker");
    let download_start = worker_stage
        .find("initialize_download_runtime")
        .expect("download runtime");
    assert!(
        sync_start < cover_cache_start
            && cover_cache_start < scheduler_start
            && scheduler_start < automatic_backup_start
            && automatic_backup_start < download_start,
        "download runtime must start last in the worker chain"
    );

    let exit = MAIN_SOURCE
        .split_once("fn perform_controlled_exit")
        .expect("controlled exit")
        .1;
    let stop_worker = exit
        .split_once("context.stop_worker")
        .expect("stop worker callback")
        .1;
    assert!(
        stop_worker
            .find("stop_download_runtime")
            .expect("download stop")
            < stop_worker
                .find("stop_automatic_backup_runtime")
                .expect("automatic backup stop"),
        "download must stop before the automatic backup worker"
    );
    assert!(
        stop_worker
            .find("stop_automatic_backup_runtime")
            .expect("automatic backup stop")
            < stop_worker
                .find("scheduler_worker")
                .expect("scheduler stop"),
        "automatic backup must stop before the scheduler"
    );
    assert!(
        stop_worker
            .find("scheduler_worker")
            .expect("scheduler stop")
            < stop_worker.find("sync_worker").expect("sync worker stop"),
        "scheduler must stop before the queue worker"
    );
    let wait_runtime = exit
        .split_once("context.wait_runtime")
        .expect("wait runtime callback")
        .1;
    assert!(wait_runtime.contains("automatic_backup_worker"));
    assert!(wait_runtime.contains("wait_download_runtime"));

    let pause = MAIN_SOURCE
        .split_once("fn set_scheduler_paused_runtime")
        .expect("pause runtime")
        .1
        .split_once("fn active_accounts_for_scope")
        .expect("pause runtime end")
        .0;
    assert!(pause.contains("wake_scheduler_and_queue(state)"));

    for (command, signature) in [
        ("begin_account_login", "async fn begin_account_login"),
        ("resume_account_login", "async fn resume_account_login"),
        ("restore_account", "fn restore_account"),
        ("archive_account", "fn archive_account"),
        ("delete_account", "fn delete_account"),
    ] {
        let body = rust_function_body(MAIN_SOURCE, signature);
        assert!(
            body.contains("notify_account_state_changed"),
            "{command} must wake the scheduler after account state changes"
        );
    }
    let notification = rust_function_body(MAIN_SOURCE, "fn notify_account_state_changed");
    assert!(notification.contains("wake_scheduler_and_queue(state)"));

    let worker_stage = startup
        .split_once("StartupStageV1::Worker =>")
        .expect("worker stage")
        .1
        .split_once("_ => Err")
        .expect("worker stage end")
        .0;
    assert!(
        worker_stage.contains("rollback_sync_runtime"),
        "scheduler startup failure must stop the already-started sync worker"
    );
    assert!(
        worker_stage.contains("rollback_scheduler_runtime"),
        "automatic-backup startup failure must stop the already-started scheduler"
    );
    assert!(
        worker_stage.contains("rollback_automatic_backup_runtime"),
        "download startup failure must stop the already-started automatic backup worker"
    );

    let login_completion = rust_function_body(MAIN_SOURCE, "fn complete_durable_login");
    assert!(
        login_completion.contains("active_account_warning"),
        "login completion must not turn an already-created account into a failed login"
    );
}

#[test]
fn backup_commands_use_directory_probe_revision_cas_and_runtime_maintenance() {
    let set_directory = rust_function_body(MAIN_SOURCE, "fn set_backup_directory_runtime");
    assert!(set_directory.contains("probe_backup_directory(path)?"));
    assert!(set_directory.contains("compare_and_swap(expected_device_revision"));
    assert!(set_directory.contains("wake_scheduler_and_queue(state)?"));

    let export = rust_function_body(MAIN_SOURCE, "async fn export_backup");
    assert!(export.contains("run_backup_with_runtime"));
    assert!(export.contains("Duration::from_secs(120)"));
    assert!(export.contains("backup:changed"));

    let inspect = rust_function_body(MAIN_SOURCE, "async fn inspect_backup_import");
    assert!(inspect.contains("inspect_backup_for_restore"));
    assert!(inspect.contains("backup_inspection_task_join_failed"));

    let restore = rust_function_body(MAIN_SOURCE, "async fn restore_backup_replace");
    assert!(restore.contains("restore_backup_replace_service"));
    assert!(restore.contains("Duration::from_secs(120)"));
    assert!(restore.contains("wake_scheduler_and_queue(state.inner())"));
    assert!(restore.contains("restore:changed"));

    let merge_plan = rust_function_body(MAIN_SOURCE, "async fn prepare_backup_merge");
    assert!(merge_plan.contains("prepare_backup_merge_service"));
    assert!(merge_plan.contains("backup_merge_plan_task_join_failed"));

    let merge = rust_function_body(MAIN_SOURCE, "async fn restore_backup_merge");
    assert!(merge.contains("restore_backup_merge_service"));
    assert!(merge.contains("Duration::from_secs(120)"));
    assert!(merge.contains("wake_scheduler_and_queue(state.inner())"));
    assert!(merge.contains("restore:changed"));
}

#[test]
fn uninstall_managed_data_cli_is_strict_and_collector_dispatch_stays_first() {
    use std::ffi::OsString;

    let absolute =
        PathBuf::from(r"C:\Users\Test\AppData\Roaming\lifearchive.douyin-history-archive");
    assert_eq!(
        parse_early_process_mode(&[
            OsString::from("--uninstall-managed-data"),
            OsString::from("--bootstrap-dir"),
            absolute.clone().into_os_string(),
        ])
        .unwrap(),
        EarlyProcessModeV1::UninstallManagedData(absolute)
    );
    for arguments in [
        vec![OsString::from("--uninstall-managed-data")],
        vec![
            OsString::from("--uninstall-managed-data"),
            OsString::from("--bootstrap-dir"),
            OsString::from("relative"),
        ],
        vec![
            OsString::from("--uninstall-managed-data"),
            OsString::from("--bootstrap-dir"),
            OsString::from(r"C:\valid"),
            OsString::from("extra"),
        ],
    ] {
        assert!(parse_early_process_mode(&arguments).is_err());
    }
    assert!(matches!(
        parse_early_process_mode(&[OsString::from("--collector")]).unwrap(),
        EarlyProcessModeV1::Collector(_)
    ));
    assert!(matches!(
        parse_early_process_mode(&[
            OsString::from("--download-resolver"),
            OsString::from("--browser-data-dir"),
            OsString::from(r"C:\Emily"),
        ])
        .unwrap(),
        EarlyProcessModeV1::DownloadResolver(_)
    ));

    let main_body = MAIN_SOURCE.split_once("fn main()").unwrap().1;
    assert!(
        main_body.find("parse_early_process_mode").unwrap()
            < main_body
                .find("GuiInstanceGuard::acquire_application")
                .unwrap()
    );
}

#[test]
fn installer_control_uses_the_application_runtime_cleanup_before_destructive_commit() {
    assert!(MAIN_SOURCE.contains("struct ApplicationInstallerControlRuntime"));
    let runtime_impl = rust_function_body(MAIN_SOURCE, "fn prepare_for_destructive_commit");
    assert!(runtime_impl.contains("prepare_application_runtime_for_uninstall(session)"));

    let main_body = rust_function_body(MAIN_SOURCE, "fn main");
    let installer_branch = main_body
        .split("EarlyProcessModeV1::InstallerControl(command)")
        .nth(1)
        .expect("installer control branch");
    assert!(installer_branch.contains("run_installer_control_with"));
    assert!(installer_branch.contains("ApplicationInstallerControlRuntime"));
    assert!(!installer_branch.contains("run_installer_control(command)"));
}

#[test]
fn uninstall_hook_uses_the_authenticated_installer_control_data_path() {
    assert!(INSTALLER_HOOKS.contains("!macro NSIS_HOOK_PREUNINSTALL"));
    assert!(INSTALLER_HOOKS.contains("--installer-control prepare"));
    assert!(INSTALLER_HOOKS.contains("--installer-control delete-managed-data"));
    assert!(INSTALLER_HOOKS.contains("${If} $DeleteAppDataCheckboxState = 1"));
    assert!(INSTALLER_HOOKS.contains("StrCpy $EmilyDataDisposition \"delete\""));
    assert!(INSTALLER_HOOKS.contains("nsExec::ExecToStack"));
    assert!(!INSTALLER_HOOKS.contains("--uninstall-managed-data"));
    assert!(INSTALLER_HOOKS.contains("Abort"));

    let config: serde_json::Value = serde_json::from_str(TAURI_CONFIG).unwrap();
    assert_eq!(
        config.pointer("/bundle/windows/nsis/installerHooks"),
        Some(&serde_json::Value::String(
            "nsis/installer-hooks.nsh".to_string()
        ))
    );
}

#[test]
fn app_paths_keep_user_data_out_of_resource_and_project_directories() {
    let paths = AppPaths::new(
        PathBuf::from("C:/AppData/Emily"),
        PathBuf::from("C:/Program/Emily"),
    );
    assert_eq!(
        paths.database,
        PathBuf::from("C:/AppData/Emily/history.sqlite")
    );
    assert_eq!(
        paths.browser_profiles,
        PathBuf::from("C:/AppData/Emily/profiles")
    );
    assert_eq!(paths.temp, PathBuf::from("C:/AppData/Emily/tmp"));
    assert_eq!(
        paths.diagnostics,
        PathBuf::from("C:/AppData/Emily/diagnostics")
    );
    assert_eq!(paths.logs, PathBuf::from("C:/AppData/Emily/logs"));
    let profile_key = "0123456789abcdef0123456789abcdef";
    assert_eq!(
        paths.account_root(profile_key).unwrap(),
        PathBuf::from("C:/AppData/Emily/profiles/0123456789abcdef0123456789abcdef")
    );
    assert_eq!(
        paths.account_browser_root(profile_key).unwrap(),
        PathBuf::from("C:/AppData/Emily/profiles/0123456789abcdef0123456789abcdef")
    );
}

#[test]
fn app_paths_reject_invalid_profile_keys() {
    let paths = AppPaths::new(PathBuf::from("data/app"), PathBuf::from("resources"));
    for profile_key in [
        ".",
        "..",
        "profile/child",
        r"profile\child",
        "01234567-89ab-cdef-0123-456789abcdef",
        "0123456789ABCDEF0123456789ABCDEF",
        "0123456789abcdef0123456789abcdeg",
    ] {
        assert_eq!(
            paths.account_root(profile_key),
            Err("invalid_profile_key".to_string()),
            "{profile_key}"
        );
    }
}

#[test]
fn sync_and_login_task_paths_are_scoped_by_account() {
    let paths = AppPaths::new(
        PathBuf::from("C:/AppData/Emily"),
        PathBuf::from("resources"),
    );
    assert_eq!(
        paths.result_path(11, 7),
        PathBuf::from("C:/AppData/Emily/tmp/account-11-sync-7.result.json")
    );
    assert_eq!(
        paths.progress_path(11, 7),
        PathBuf::from("C:/AppData/Emily/tmp/account-11-sync-7.progress.json")
    );
    assert_eq!(
        paths.cancel_path(11, 7),
        PathBuf::from("C:/AppData/Emily/tmp/account-11-sync-7.cancel")
    );
    assert_eq!(
        paths.browser_pid_path(11, 7),
        PathBuf::from("C:/AppData/Emily/tmp/account-11-sync-7.browser.pid")
    );
    assert_eq!(
        paths.diagnostic_path(11, 7),
        PathBuf::from("C:/AppData/Emily/diagnostics/account-11/sync-7.json")
    );
    assert_eq!(
        paths.log_path(11, 7),
        PathBuf::from("C:/AppData/Emily/logs/account-11/sync-7.log")
    );

    let attempt_id = "0123456789abcdef0123456789abcdef";
    assert_eq!(
        paths.login_identity_path(11, attempt_id).unwrap(),
        PathBuf::from(
            "C:/AppData/Emily/tmp/account-11-login-0123456789abcdef0123456789abcdef.identity.json"
        )
    );
    assert_eq!(
        paths.login_cancel_path(11, attempt_id).unwrap(),
        PathBuf::from(
            "C:/AppData/Emily/tmp/account-11-login-0123456789abcdef0123456789abcdef.cancel"
        )
    );
    assert_eq!(
        paths.login_progress_path(11, attempt_id).unwrap(),
        PathBuf::from(
            "C:/AppData/Emily/tmp/account-11-login-0123456789abcdef0123456789abcdef.progress.json"
        )
    );
    assert_eq!(
        paths.login_browser_pid_path(11, attempt_id).unwrap(),
        PathBuf::from(
            "C:/AppData/Emily/tmp/account-11-login-0123456789abcdef0123456789abcdef.browser.pid"
        )
    );
    assert_eq!(
        paths.login_diagnostic_path(11, attempt_id).unwrap(),
        PathBuf::from(
            "C:/AppData/Emily/diagnostics/account-11/login-0123456789abcdef0123456789abcdef.json"
        )
    );
    assert_eq!(
        paths.login_log_path(11, attempt_id).unwrap(),
        PathBuf::from(
            "C:/AppData/Emily/logs/account-11/login-0123456789abcdef0123456789abcdef.log"
        )
    );

    assert_ne!(paths.result_path(11, 7), paths.result_path(12, 7));
    assert_ne!(
        paths.login_identity_path(11, attempt_id).unwrap(),
        paths.login_identity_path(12, attempt_id).unwrap()
    );
}

#[test]
fn phase_envelope_and_paths_bind_every_phase_identity() {
    let paths = AppPaths::new(
        PathBuf::from("C:/AppData/Emily"),
        PathBuf::from("resources"),
    );
    let attempt = "0123456789abcdef0123456789abcdef";
    assert_eq!(
        paths.phase_result_path(11, 7, SyncPhase::Collecting, attempt).unwrap(),
        PathBuf::from("C:/AppData/Emily/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.result.json")
    );
    assert_eq!(
        paths.cancel_path(11, 7),
        PathBuf::from("C:/AppData/Emily/tmp/account-11-sync-7.cancel")
    );
    assert!(paths
        .phase_result_path(11, 7, SyncPhase::Collecting, "bad")
        .is_err());

    let envelope = SyncPhaseEnvelopeV1::new(
        11,
        7,
        SyncPhase::Collecting,
        attempt,
        json!({"identityHash":"a".repeat(64),"fieldHash":"b".repeat(64)}),
    )
    .unwrap();
    envelope
        .validate_context(11, 7, SyncPhase::Collecting, attempt)
        .unwrap();
    assert_eq!(
        envelope.validate_context(12, 7, SyncPhase::Collecting, attempt),
        Err("sync_phase_envelope_mismatch".into())
    );
    let serialized = serde_json::to_string(&envelope).unwrap();
    assert!(serialized.contains("\"payload\":{"));
    assert!(!serialized.starts_with("{\"identityHash\""));
    assert!(
        serde_json::from_value::<SyncPhaseEnvelopeV1<serde_json::Value>>(json!({
            "identityHash": "a".repeat(64)
        }))
        .is_err()
    );
    let mut unknown = serde_json::to_value(&envelope).unwrap();
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<SyncPhaseEnvelopeV1<serde_json::Value>>(unknown).is_err());
}

#[test]
fn orchestration_cas_rejects_stale_or_wrong_phase_identity() {
    let root = std::env::temp_dir().join(format!("emily-orchestration-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("tmp")).unwrap();
    let paths = AppPaths::new(root.clone(), root.join("resources"));
    let attempt = "0123456789abcdef0123456789abcdef";
    let initial = create_orchestration(
        &paths,
        11,
        7,
        SyncPhase::Collecting,
        attempt,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
        "2026-07-20T00:00:00.000Z",
    )
    .unwrap();
    assert_eq!(initial.revision, 0);
    assert_eq!(initial.lifecycle, SyncLifecycle::Preparing);
    assert_eq!(read_orchestration(&paths, 11, 7).unwrap(), initial);
    let running = initial
        .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
        .unwrap();
    cas_orchestration(&paths, &initial, &running).unwrap();
    assert_eq!(read_orchestration(&paths, 11, 7).unwrap(), running);
    assert_eq!(
        cas_orchestration(&paths, &initial, &running),
        Err("sync_orchestration_conflict".into())
    );

    let mut wrong_lifecycle = running.clone();
    wrong_lifecycle.revision += 1;
    assert_eq!(
        cas_orchestration(&paths, &running, &wrong_lifecycle),
        Err("sync_orchestration_conflict".into())
    );

    let mut wrong_attempt: SyncOrchestrationV1 = running
        .next(SyncLifecycle::Consuming, "2026-07-20T00:00:02.000Z")
        .unwrap();
    wrong_attempt.phase_attempt_id = "ffffffffffffffffffffffffffffffff".into();
    assert_eq!(
        cas_orchestration(&paths, &running, &wrong_attempt),
        Err("sync_orchestration_conflict".into())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn orchestration_cancel_is_durable_and_cannot_cross_consuming() {
    let root = std::env::temp_dir().join(format!(
        "emily-orchestration-cancel-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(root.join("tmp")).unwrap();
    let paths = AppPaths::new(root.clone(), root.join("resources"));
    let attempt = "0123456789abcdef0123456789abcdef";
    let initial = create_orchestration(
        &paths,
        11,
        7,
        SyncPhase::Collecting,
        attempt,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
        "2026-07-20T00:00:00.000Z",
    )
    .unwrap();
    let running = initial
        .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
        .unwrap();
    cas_orchestration(&paths, &initial, &running).unwrap();

    let cancelled =
        request_orchestration_cancel(&paths, &running, "2026-07-20T00:00:02.000Z").unwrap();

    assert_eq!(cancelled.revision, running.revision + 1);
    assert_eq!(cancelled.lifecycle, SyncLifecycle::Running);
    assert_eq!(
        cancelled.cancel_requested_at.as_deref(),
        Some("2026-07-20T00:00:02.000Z")
    );
    assert_eq!(read_orchestration(&paths, 11, 7).unwrap(), cancelled);
    assert_eq!(
        cancelled
            .next(SyncLifecycle::Consuming, "2026-07-20T00:00:03.000Z")
            .unwrap_err(),
        "sync_cancel_requested"
    );

    let second_root = root.join("second");
    fs::create_dir_all(second_root.join("tmp")).unwrap();
    let second_paths = AppPaths::new(second_root.clone(), second_root.join("resources"));
    let second_initial = create_orchestration(
        &second_paths,
        11,
        8,
        SyncPhase::Collecting,
        attempt,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        None,
        "2026-07-20T00:00:00.000Z",
    )
    .unwrap();
    let second_running = second_initial
        .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
        .unwrap();
    cas_orchestration(&second_paths, &second_initial, &second_running).unwrap();
    let consuming = second_running
        .next(SyncLifecycle::Consuming, "2026-07-20T00:00:02.000Z")
        .unwrap();
    cas_orchestration(&second_paths, &second_running, &consuming).unwrap();

    assert_eq!(
        request_orchestration_cancel(&second_paths, &consuming, "2026-07-20T00:00:03.000Z",)
            .unwrap_err(),
        "sync_finalizing"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sync_driver_routes_only_the_first_collect_session_expiry_to_recovery() {
    let initial = orchestration_fixture(SyncPhase::Collecting, SyncLifecycle::Running, 0, None);
    let expired = PhaseCompletionEvidence {
        process_succeeded: false,
        result_ready: false,
        diagnostic_status: Some("failed".to_string()),
        diagnostic_error_code: Some("session_expired".to_string()),
        cancel_file_exists: false,
    };

    assert_eq!(
        phase_completion_decision(&initial, &expired),
        PhaseCompletionDecision::StartRecovery
    );

    let retried = orchestration_fixture(
        SyncPhase::RetryingAfterLogin,
        SyncLifecycle::Running,
        1,
        None,
    );
    assert_eq!(
        phase_completion_decision(&retried, &expired),
        PhaseCompletionDecision::FailInterrupted
    );
}

#[test]
fn sync_driver_honors_cancellation_before_consuming_and_finalizes_after_it() {
    let running = orchestration_fixture(SyncPhase::Collecting, SyncLifecycle::Running, 0, None);
    let cancelled_with_result = PhaseCompletionEvidence {
        process_succeeded: true,
        result_ready: true,
        diagnostic_status: None,
        diagnostic_error_code: None,
        cancel_file_exists: true,
    };
    assert_eq!(
        phase_completion_decision(&running, &cancelled_with_result),
        PhaseCompletionDecision::Cancel
    );

    let consuming = orchestration_fixture(SyncPhase::Collecting, SyncLifecycle::Consuming, 0, None);
    assert_eq!(
        phase_completion_decision(&consuming, &cancelled_with_result),
        PhaseCompletionDecision::Consume
    );
}

#[test]
fn sync_driver_prevents_new_phases_after_persistent_cancellation() {
    let mut collecting =
        orchestration_fixture(SyncPhase::Collecting, SyncLifecycle::Running, 0, None);
    collecting.cancel_requested_at = Some("2026-07-20T00:00:01.000Z".to_string());
    assert_eq!(
        prepare_next_phase(
            &collecting,
            SyncPhase::RecoveringLogin,
            "11111111111111111111111111111111",
            Some(1_784_515_230_000),
            1_784_515_200_000,
            "2026-07-20T00:00:02.000Z",
        ),
        Err("sync_cancel_requested".to_string())
    );

    let recovering = orchestration_fixture(
        SyncPhase::RecoveringLogin,
        SyncLifecycle::Running,
        1,
        Some(1_784_515_230_000),
    );
    let retry = prepare_next_phase(
        &recovering,
        SyncPhase::RetryingAfterLogin,
        "22222222222222222222222222222222",
        None,
        1_784_515_229_999,
        "2026-07-20T00:00:29.999Z",
    )
    .expect("retry preparing");
    assert_eq!(retry.phase, SyncPhase::RetryingAfterLogin);
    assert_eq!(retry.lifecycle, SyncLifecycle::Preparing);
    assert_eq!(retry.login_recovery_count, 1);
    assert_eq!(retry.deadline_at_ms, None);

    assert_eq!(
        prepare_next_phase(
            &recovering,
            SyncPhase::RetryingAfterLogin,
            "33333333333333333333333333333333",
            None,
            1_784_515_230_000,
            "2026-07-20T00:00:30.000Z",
        ),
        Err("login_recovery_failed".to_string())
    );
}

#[test]
fn sync_driver_starts_recovery_and_retry_only_from_the_expected_phase() {
    let collecting = orchestration_fixture(SyncPhase::Collecting, SyncLifecycle::Running, 0, None);
    let recovery = prepare_next_phase(
        &collecting,
        SyncPhase::RecoveringLogin,
        "11111111111111111111111111111111",
        Some(1_784_515_230_000),
        1_784_515_200_000,
        "2026-07-20T00:00:00.000Z",
    )
    .expect("recovery preparing");
    assert_eq!(recovery.revision, collecting.revision + 1);
    assert_eq!(recovery.login_recovery_count, 1);
    assert_eq!(recovery.deadline_at_ms, Some(1_784_515_230_000));

    assert_eq!(
        prepare_next_phase(
            &collecting,
            SyncPhase::RetryingAfterLogin,
            "22222222222222222222222222222222",
            None,
            1_784_515_200_000,
            "2026-07-20T00:00:00.000Z",
        ),
        Err("sync_phase_transition_invalid".to_string())
    );
}

#[test]
fn startup_recovery_decision_is_deterministic_for_every_phase_and_lifecycle() {
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
            for result_ready in [false, true] {
                for cancellation in ["none", "orchestration", "file"] {
                    let mut orchestration = orchestration_fixture(
                        phase,
                        lifecycle,
                        u8::from(phase != SyncPhase::Collecting),
                        None,
                    );
                    orchestration.cancel_requested_at = (cancellation == "orchestration")
                        .then(|| "2026-07-20T00:00:01.000Z".to_string());
                    let cancel_file_exists = cancellation == "file";
                    let expected = if lifecycle == SyncLifecycle::Consuming
                        && matches!(phase, SyncPhase::Collecting | SyncPhase::RetryingAfterLogin)
                        && result_ready
                    {
                        StartupRecoveryDecision::Consume
                    } else if cancellation != "none" {
                        StartupRecoveryDecision::Cancel
                    } else {
                        StartupRecoveryDecision::FailInterrupted
                    };

                    assert_eq!(
                        startup_recovery_decision(
                            &orchestration,
                            result_ready,
                            cancel_file_exists,
                        ),
                        expected,
                        "phase={phase:?} lifecycle={lifecycle:?} result={result_ready} cancellation={cancellation}"
                    );
                }
            }
        }
    }
}

#[test]
fn orchestration_cas_replaces_only_the_expected_running_phase() {
    let root = std::env::temp_dir().join(format!("emily-phase-replace-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("tmp")).unwrap();
    let paths = AppPaths::new(root.clone(), root.join("resources"));
    let collecting = create_orchestration(
        &paths,
        11,
        7,
        SyncPhase::Collecting,
        "0123456789abcdef0123456789abcdef",
        &"a".repeat(64),
        None,
        "2026-07-20T00:00:00.000Z",
    )
    .unwrap();
    let collecting = collecting
        .next(SyncLifecycle::Running, "2026-07-20T00:00:01.000Z")
        .and_then(|running| {
            cas_orchestration(
                &paths,
                &read_orchestration(&paths, 11, 7).unwrap(),
                &running,
            )?;
            Ok(running)
        })
        .unwrap();
    let recovering = prepare_next_phase(
        &collecting,
        SyncPhase::RecoveringLogin,
        "11111111111111111111111111111111",
        Some(1_784_515_230_000),
        1_784_515_200_000,
        "2026-07-20T00:00:02.000Z",
    )
    .unwrap();

    cas_orchestration(&paths, &collecting, &recovering).expect("replace current phase");
    assert_eq!(read_orchestration(&paths, 11, 7).unwrap(), recovering);

    assert_eq!(
        cas_orchestration(&paths, &collecting, &recovering),
        Err("sync_orchestration_conflict".to_string())
    );
    fs::remove_dir_all(root).unwrap();
}

fn orchestration_fixture(
    phase: SyncPhase,
    lifecycle: SyncLifecycle,
    login_recovery_count: u8,
    deadline_at_ms: Option<i64>,
) -> SyncOrchestrationV1 {
    SyncOrchestrationV1 {
        schema_version: 1,
        revision: 4,
        account_id: 11,
        sync_job_id: 7,
        phase,
        phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        lifecycle,
        login_recovery_count,
        cancel_requested_at: None,
        options_hash: "a".repeat(64),
        deadline_at_ms,
        updated_at: "2026-07-20T00:00:00.000Z".to_string(),
    }
}

#[test]
fn login_watchdog_state_machine_bounds_browser_and_collector_shutdown() {
    let started = Instant::now();
    let mut watchdog = LoginWatchdog::new(started, started + Duration::from_secs(600));
    let progress = LoginProgressV1 {
        schema_version: 1,
        account_id: 11,
        login_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        state: "closing".to_string(),
        reason: LoginProgressReason::BrowserClosed,
        observed_at_ms: 1_000,
        deadline_at_ms: 3_000,
    };

    assert_eq!(
        watchdog.observe_progress(&progress, 11, "0123456789abcdef0123456789abcdef", started,),
        LoginWatchdogEvent::BeginShutdown(LoginTermination::BrowserClosed)
    );
    assert_eq!(watchdog.deadline(), started + Duration::from_secs(2));
    assert_eq!(
        watchdog.tick(started + Duration::from_millis(500)),
        LoginWatchdogEvent::ForceBrowser
    );
    assert_eq!(
        watchdog.tick(started + Duration::from_secs(2)),
        LoginWatchdogEvent::ForceCollectorAndRelease
    );
}

#[test]
fn app_paths_remove_windows_verbatim_prefixes_before_invoking_collector() {
    let paths = AppPaths::new(
        PathBuf::from(r"\\?\C:\Users\ExampleUser\AppData\Roaming\Emily"),
        PathBuf::from(r"\\?\C:\Program Files\Emily"),
    );

    assert_eq!(
        paths.result_path(11, 7),
        PathBuf::from(r"C:\Users\ExampleUser\AppData\Roaming\Emily\tmp\account-11-sync-7.result.json")
    );
}

#[test]
fn invocation_uses_current_executable_and_task_files() {
    let paths = AppPaths::new(
        PathBuf::from("C:/AppData/Emily"),
        PathBuf::from("C:/Program/Emily"),
    );
    let options: SyncOptions = serde_json::from_value(json!({
        "mode": "days",
        "days": 5,
        "scrollSpeed": 5_000,
        "hideBrowser": true,
        "blockMedia": false,
        "blockImages": false
    }))
    .expect("legacy options");
    let executable = PathBuf::from("C:/Program/Emily/app.exe");
    let profile_key = "0123456789abcdef0123456789abcdef";
    let invocation = build_phase_collect_invocation(
        &paths,
        executable.clone(),
        11,
        profile_key,
        7,
        SyncPhase::Collecting,
        "0123456789abcdef0123456789abcdef",
        &options,
    )
    .expect("collect invocation");

    assert_eq!(invocation.program, executable);
    assert_eq!(invocation.args[0], "--collector");
    assert_eq!(invocation.args[1], "collect");
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| pair == ["--job-id", "7"]));
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| pair == ["--account-id", "11"]));
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| pair == ["--phase", "collecting"]));
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| pair == ["--phase-attempt-id", "0123456789abcdef0123456789abcdef"]));
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| pair == ["--days", "5"]));
    assert!(invocation.args.iter().any(|arg| arg == "--headless"));
    assert!(invocation.args.iter().any(|arg| arg == "--block-images"));
    assert!(!invocation.args.iter().any(|arg| arg == "--no-block-media"));
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| pair[0] == "--account-profile-root"
            && pair[1]
                == paths
                    .account_browser_root(profile_key)
                    .unwrap()
                    .to_string_lossy()));
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| pair[0] == "--browser-pid-file"
            && pair[1]
                == paths
                    .phase_browser_pid_path(
                        11,
                        7,
                        SyncPhase::Collecting,
                        "0123456789abcdef0123456789abcdef",
                    )
                    .unwrap()
                    .to_string_lossy()));
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| pair[0] == "--collector-identity-file"
            && pair[1]
                == paths
                    .phase_collector_identity_path(
                        11,
                        7,
                        SyncPhase::Collecting,
                        "0123456789abcdef0123456789abcdef",
                    )
                    .unwrap()
                    .to_string_lossy()));
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| pair == ["--browser", "auto"]));
    assert!(!invocation.args.iter().any(|arg| arg == "--profile-dir"));
}

#[test]
fn login_invocation_uses_the_same_executable_mode() {
    let paths = AppPaths::new(PathBuf::from("data/app"), PathBuf::from("resources"));
    let executable = PathBuf::from("C:/Program/Emily/app.exe");
    let profile_key = "0123456789abcdef0123456789abcdef";
    let attempt_id = "fedcba9876543210fedcba9876543210";
    let login = build_login_invocation(
        &paths,
        executable.clone(),
        None,
        profile_key,
        attempt_id,
        BrowserPreferenceV1::Auto,
    )
    .expect("login invocation");
    assert_eq!(login.program, executable);
    assert_eq!(&login.args[..2], ["--collector", "login"]);
    assert!(login.args.iter().any(|arg| arg == "login"));
    assert!(!login.args.iter().any(|arg| arg == "--account-id"));
    assert!(!login.args.iter().any(|arg| arg == "--target-account-id"));
    assert!(login
        .args
        .windows(2)
        .any(|pair| pair == ["--home-url", "https://www.douyin.com/"]));
    assert!(login
        .args
        .windows(2)
        .any(|pair| pair[0] == "--account-profile-root"
            && pair[1]
                == paths
                    .account_browser_root(profile_key)
                    .unwrap()
                    .to_string_lossy()));
    for option in [
        "--login-attempt-id",
        "--identity-result",
        "--login-progress-file",
        "--diagnostic",
        "--cancel-file",
        "--browser-pid-file",
    ] {
        assert!(login.args.iter().any(|arg| arg == option), "{option}");
    }
    assert!(login
        .args
        .windows(2)
        .any(|pair| pair == ["--browser", "auto"]));
    assert!(!login.args.iter().any(|arg| arg == "--profile-dir"));

    let relogin = build_login_invocation(
        &paths,
        PathBuf::from("C:/Program/Emily/app.exe"),
        Some(11),
        profile_key,
        attempt_id,
        BrowserPreferenceV1::Chrome,
    )
    .expect("relogin invocation");
    assert!(relogin
        .args
        .windows(2)
        .any(|pair| pair == ["--target-account-id", "11"]));
    assert!(relogin
        .args
        .windows(2)
        .any(|pair| pair == ["--browser", "chrome"]));

    assert_eq!(
        paths
            .login_task_collector_identity_path(attempt_id)
            .unwrap(),
        PathBuf::from("data/app/tmp/login-fedcba9876543210fedcba9876543210.collector.pid")
    );
    assert_ne!(
        paths
            .login_task_collector_identity_path(attempt_id)
            .unwrap(),
        paths.login_task_browser_pid_path(attempt_id).unwrap(),
    );
}

#[test]
fn recover_login_invocation_is_hidden_deadline_bounded_and_phase_scoped() {
    let paths = AppPaths::new(
        PathBuf::from("C:/AppData/Emily"),
        PathBuf::from("C:/Program/Emily"),
    );
    let executable = PathBuf::from("C:/Program/Emily/app.exe");
    let attempt_id = "0123456789abcdef0123456789abcdef";

    let invocation = build_recover_login_invocation(
        &paths,
        executable.clone(),
        11,
        "fedcba9876543210fedcba9876543210",
        7,
        attempt_id,
        1_784_457_630_000,
    )
    .unwrap();

    assert_eq!(invocation.program, executable);
    assert_eq!(&invocation.args[..2], ["--collector", "recover-login"]);
    for pair in [
        ["--account-id", "11"],
        ["--job-id", "7"],
        ["--phase", "recovering_login"],
        ["--phase-attempt-id", attempt_id],
        ["--deadline-at-ms", "1784457630000"],
        ["--home-url", "https://www.douyin.com/"],
    ] {
        assert!(invocation.args.windows(2).any(|window| window == pair));
    }
    assert!(invocation.args.iter().any(|value| value == "--headless"));
    for option in [
        "--result",
        "--progress",
        "--diagnostic",
        "--cancel-file",
        "--browser-pid-file",
        "--collector-identity-file",
    ] {
        assert!(
            invocation.args.iter().any(|value| value == option),
            "{option}"
        );
    }
    assert!(!invocation.args.iter().any(|value| value == "--history-url"));
}

#[test]
fn browser_process_record_requires_matching_executable_and_profile() {
    let record = parse_browser_process_record(
        r#"{"schemaVersion":1,"pid":4321,"browserId":"edge","executablePath":"C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe","profilePath":"C:\\App\\profiles\\edge","startedAt":"2026-07-11T00:00:00.000Z"}"#,
    )
    .expect("record");
    let matching = BrowserProcessInfo {
        executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
            .to_string(),
        command_line: r#"msedge.exe "--user-data-dir=C:\App\profiles\edge" --headless=new"#
            .to_string(),
    };
    assert!(browser_process_matches(&record, &matching));
    assert!(!browser_process_matches(
        &record,
        &BrowserProcessInfo {
            executable_path: "C:\\Windows\\System32\\notepad.exe".to_string(),
            command_line: matching.command_line.clone(),
        }
    ));
    assert!(parse_browser_process_record("4321").is_none());
}
