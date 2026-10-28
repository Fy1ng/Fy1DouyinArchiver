use super::session_probe::{
    session_probe_from_history_value, session_probe_from_page_value, session_probe_from_value,
    ObservedHomeIdentityV1, SessionProbeV1, DOUYIN_HISTORY_URL,
};
use super::{
    archive_normalization::{
        normalize_active_remote_folder, normalize_archive_items, normalize_remote_folders,
        ArchiveTimeUnit,
    },
    archive_network::{is_remote_folder_list_response, parse_remote_folder_list_response},
    archive_snapshot::{
        archive_bottom_confirmed, archive_snapshots_stable, read_raw_archive_snapshot,
        click_favorite_folders_tab, click_favorite_tab, read_archive_wheel_target_with_deadline,
        read_remote_folder_page_snapshot_with_deadline,
        select_remote_folder_with_name, ArchiveSnapshotOptions,
        RawArchiveSnapshot, RemoteFolderPageSnapshot,
    },
    archive_sync_engine::{
        archive_snapshot_made_progress, consecutive_known_tail, decide_archive_stop,
        decide_remote_folder_discovery, validate_archive_growth, ArchiveStopDecision,
        ArchiveStopInput, RemoteFolderDiscoveryDecision,
    },
    browser_process::BrowserProcessRecordV2,
    browser_runtime::{persist_validation, resolve_browser, BrowserChoice},
    browser_state::{read_matching_browser_profile_identity, BrowserId, BrowserProfileIdentityV1},
    cli::{CollectOptions, CollectRange, CollectorCommand, LoginOptions, RecoverLoginOptions},
    contracts::{
        CollectorDiagnostic, LoginProgressReason, LoginProgressV1, COLLECTOR_SCHEMA_VERSION,
        COLLECTOR_TIME_ZONE,
    },
    deadline::Deadline,
    errors::describe_collector_error,
    file_store::{read_json, write_bytes_atomically, write_json_atomically},
    normalization::HistoryTimeUnit,
    session::{launch_browser_session, launch_browser_session_with_deadline, CdpBrowserSession},
    snapshot::{read_raw_snapshot, RawReactSnapshot},
    sync_engine::{filter_range, finish_result, progress, range_complete, validate_growth},
};
use crate::login_attempt::{
    LoginAttemptEnvelopeV1, LoginDiagnosticPayloadV1, LoginProgressPayloadV1,
};
use crate::{
    sync::{
        browser_executable_path_sha256, shanghai_cutoff_ms, validate_archive_known_ids_snapshot,
        ArchiveCollectionSuccessV1, ArchiveKnownIdsSnapshotV1, ArchiveRunModeV1,
        FavoriteCollectionSuccessV1, RemoteFolderCollectionSuccessV1,
        RemoteFolderDiscoveryResultV1, RemoteFolderDiscoverySuccessV1, SourceExecutionResultV1,
        SyncExecutionEnvelopeV1,
        SyncJobSnapshotV1, SyncJobStatusV1, SyncSourceV1, SyncTriggerV1, TaskKeyV1,
    },
    sync_orchestration::SyncPhaseEnvelopeV1,
    CollectorRange, CollectorSuccessV3,
};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    future::Future,
    time::{Duration, Instant},
};

const LOGIN_BROWSER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);
const LOGIN_BROWSER_FORCE_TIMEOUT: Duration = Duration::from_secs(2);

async fn finish_login_browser_shutdown_with<F, W, T>(
    close: F,
    shutdown_deadline: Deadline,
    mut wait: W,
    mut terminate: T,
) -> Result<(), String>
where
    F: Future<Output = Result<(), String>>,
    W: FnMut(u32) -> bool,
    T: FnMut() -> Result<(), String>,
{
    let _ = close.await;
    let graceful_wait_ms = shutdown_deadline
        .remaining()
        .unwrap_or_default()
        .as_millis()
        .min(u32::MAX as u128) as u32;
    if graceful_wait_ms > 0 && wait(graceful_wait_ms) {
        return Ok(());
    }
    terminate()?;
    let force_wait_ms = LOGIN_BROWSER_FORCE_TIMEOUT
        .as_millis()
        .min(u32::MAX as u128) as u32;
    if wait(force_wait_ms) {
        Ok(())
    } else {
        Err("browser_shutdown_timeout".to_string())
    }
}

pub async fn run_command(command: CollectorCommand) -> Result<(), String> {
    match command {
        CollectorCommand::Collect(options) => run_collection(options).await,
        CollectorCommand::Login(options) => run_login(options).await,
        CollectorCommand::RecoverLogin(options) => run_recover_login(options).await,
    }
}

async fn run_recover_login(options: RecoverLoginOptions) -> Result<(), String> {
    let started = Instant::now();
    let _ = fs::remove_file(&options.result_path);
    let _ = fs::remove_file(&options.progress_path);
    let _ = fs::remove_file(&options.diagnostic_path);
    let _ = fs::remove_file(&options.browser_pid_file);
    let outcome: Result<(), String> = async {
        if !options.headless {
            return Err("recover_login_requires_headless".to_string());
        }
        let deadline = Deadline::from_unix_ms(options.deadline_at_ms)?;
        if options.cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        write_recover_json(
            &options,
            &options.progress_path,
            &progress(
                options.account_id,
                options.sync_job_id,
                "recovering_login",
                0,
                started.elapsed().as_millis() as i64,
                0,
                true,
            ),
        )?;
        let choice = resolve_browser(&options.account_profile_root, options.browser, true)?;
        let session = launch_browser_session_with_deadline(
            &choice.executable_path,
            &options.account_profile_root,
            choice.browser_id,
            true,
            deadline,
        )
        .await?;
        let record = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: session.process.pid,
            creation_time_100ns: session.process.creation_time_100ns.clone(),
            browser_id: choice.browser_id,
            executable_path: choice.executable_path.to_string_lossy().to_string(),
            profile_path: session.profile_path.to_string_lossy().to_string(),
            user_data_dir_argument: session.user_data_dir_argument.clone(),
            devtools_port: session.port,
            browser_websocket_path: session.browser_websocket_path.clone(),
            started_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        };
        write_recover_json(&options, &options.browser_pid_file, &record)?;
        let result = async {
            session
                .page
                .block_resources_with_deadline(false, true, deadline)
                .await?;
            session
                .page
                .navigate_with_deadline(&options.home_url, deadline)
                .await?;
            let identity =
                wait_for_recovered_home_identity(&session.page, deadline, &options.cancel_file)
                    .await?;
            super::login_actions::process_optional_save_login(
                &session.page,
                Duration::from_secs(5),
                deadline,
            )
            .await?;
            if options.cancel_file.exists() {
                return Err("collector_cancelled".to_string());
            }
            let shutdown_deadline = deadline.capped(LOGIN_BROWSER_SHUTDOWN_TIMEOUT)?;
            finish_login_browser_shutdown_with(
                session.page.close_with_deadline(shutdown_deadline),
                shutdown_deadline,
                |wait_ms| session.process.wait(wait_ms),
                || session.process.terminate(),
            )
            .await?;
            write_recover_json(&options, &options.result_path, &identity)
        }
        .await;
        if result.is_err() && deadline.remaining().is_ok() {
            let _ = session.page.close_with_deadline(deadline).await;
        }
        result
    }
    .await;
    if let Err(error) = &outcome {
        let _ = fs::remove_file(&options.result_path);
        let cancelled = error == "collector_cancelled" || options.cancel_file.exists();
        let described = describe_collector_error(error);
        let diagnostic = CollectorDiagnostic {
            schema_version: COLLECTOR_SCHEMA_VERSION,
            account_id: options.account_id,
            sync_job_id: options.sync_job_id,
            status: if cancelled { "cancelled" } else { "failed" }.to_string(),
            phase: if cancelled {
                "cancelled"
            } else {
                "recovering_login"
            }
            .to_string(),
            error_code: if cancelled {
                "collector_cancelled".to_string()
            } else {
                described.code
            },
            message: described.message,
            record_count: 0,
            dom_card_count: 0,
            calendar_time_zone: COLLECTOR_TIME_ZONE.to_string(),
            elapsed_ms: started.elapsed().as_millis() as i64,
            no_progress_ms: 0,
            retry_count: 0,
        };
        let _ = write_recover_json(&options, &options.diagnostic_path, &diagnostic);
    }
    outcome
}

async fn run_login(options: LoginOptions) -> Result<(), String> {
    let deadline = Deadline::after(Duration::from_secs(600));
    let _ = fs::remove_file(&options.identity_result_path);
    let _ = fs::remove_file(&options.diagnostic_path);
    let _ = fs::remove_file(&options.cancel_file);
    let _ = fs::remove_file(&options.browser_pid_file);
    let _ = fs::remove_file(&options.progress_path);
    let outcome: Result<(), String> = async {
        if options.cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let choice = resolve_browser(&options.account_profile_root, options.browser, false)?;
        let session = launch_browser_session_with_deadline(
            &choice.executable_path,
            &options.account_profile_root,
            choice.browser_id,
            false,
            deadline,
        )
        .await?;
        let record = BrowserProcessRecordV2 {
            schema_version: 2,
            pid: session.process.pid,
            creation_time_100ns: session.process.creation_time_100ns.clone(),
            browser_id: choice.browser_id,
            executable_path: choice.executable_path.to_string_lossy().to_string(),
            profile_path: session.profile_path.to_string_lossy().to_string(),
            user_data_dir_argument: session.user_data_dir_argument.clone(),
            devtools_port: session.port,
            browser_websocket_path: session.browser_websocket_path.clone(),
            started_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        };
        write_json_atomically(&options.browser_pid_file, &record)?;
        let result = async {
            session
                .page
                .block_resources_with_deadline(false, true, deadline)
                .await?;
            session
                .page
                .navigate_with_deadline(&options.home_url, deadline)
                .await?;
            let profile_key = options
                .account_profile_root
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| "invalid_account_profile_root".to_string())?;
            let identity = wait_for_home_identity(
                &session.page,
                Duration::from_secs(600),
                &options.cancel_file,
            )
            .await?;
            super::login_actions::process_optional_save_login(
                &session.page,
                Duration::from_secs(5),
                deadline,
            )
            .await?;
            let envelope = LoginAttemptEnvelopeV1::new(
                &options.login_attempt_id,
                options.target_account_id,
                profile_key,
                identity,
            )?;
            write_json_atomically(&options.identity_result_path, &envelope)
                .map_err(|_| "identity_result_write_failed".to_string())?;
            let shutdown_deadline = deadline.capped(LOGIN_BROWSER_SHUTDOWN_TIMEOUT)?;
            finish_login_browser_shutdown_with(
                session.page.close_with_deadline(shutdown_deadline),
                shutdown_deadline,
                |wait_ms| session.process.wait(wait_ms),
                || session.process.terminate(),
            )
            .await
        }
        .await;
        if let Err(error) = &result {
            let progress = login_progress_envelope_for_error(
                error,
                &options.login_attempt_id,
                options.target_account_id,
                options
                    .account_profile_root
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default(),
                Utc::now().timestamp_millis(),
            );
            if let Ok(progress) = progress {
                let _ = write_json_atomically(&options.progress_path, &progress);
            }
        }
        if result.is_err() {
            session.page.close().await;
        }
        result
    }
    .await;
    let _ = fs::remove_file(&options.cancel_file);
    if let Err(error) = &outcome {
        let _ = fs::remove_file(&options.identity_result_path);
        if !options.progress_path.exists() {
            let progress = login_progress_envelope_for_error(
                error,
                &options.login_attempt_id,
                options.target_account_id,
                options
                    .account_profile_root
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default(),
                Utc::now().timestamp_millis(),
            );
            if let Ok(progress) = progress {
                let _ = write_json_atomically(&options.progress_path, &progress);
            }
        }
        let cancelled = error == "collector_cancelled";
        let described = describe_collector_error(error);
        let payload = LoginDiagnosticPayloadV1 {
            status: if cancelled {
                "cancelled"
            } else if error == "account_identity_unavailable" {
                "timeout"
            } else {
                "failed"
            }
            .to_string(),
            error_code: if cancelled {
                "collector_cancelled".to_string()
            } else {
                described.code
            },
            message: described.message,
        };
        if let Some(profile_key) = options
            .account_profile_root
            .file_name()
            .and_then(|value| value.to_str())
        {
            if let Ok(diagnostic) = LoginAttemptEnvelopeV1::new(
                &options.login_attempt_id,
                options.target_account_id,
                profile_key,
                payload,
            ) {
                let _ = write_json_atomically(&options.diagnostic_path, &diagnostic);
            }
        }
    }
    outcome
}

fn login_progress_envelope_for_error(
    error: &str,
    login_attempt_id: &str,
    target_account_id: Option<i64>,
    profile_key: &str,
    observed_at_ms: i64,
) -> Result<LoginAttemptEnvelopeV1<LoginProgressPayloadV1>, String> {
    let legacy = login_progress_for_error(error, 1, login_attempt_id, observed_at_ms);
    LoginAttemptEnvelopeV1::new(
        login_attempt_id,
        target_account_id,
        profile_key,
        LoginProgressPayloadV1 {
            state: legacy.state,
            reason: legacy.reason,
            observed_at_ms: legacy.observed_at_ms,
            deadline_at_ms: legacy.deadline_at_ms,
        },
    )
}

pub fn login_progress_for_error(
    error: &str,
    account_id: i64,
    login_attempt_id: &str,
    observed_at_ms: i64,
) -> LoginProgressV1 {
    let reason = match error {
        "collector_cancelled" => LoginProgressReason::Cancelled,
        "cdp_target_closed" => LoginProgressReason::BrowserClosed,
        "account_identity_unavailable" => LoginProgressReason::Timeout,
        _ => LoginProgressReason::Failed,
    };
    LoginProgressV1 {
        schema_version: 1,
        account_id,
        login_attempt_id: login_attempt_id.to_string(),
        state: "closing".to_string(),
        reason,
        observed_at_ms,
        deadline_at_ms: observed_at_ms.saturating_add(2_000),
    }
}

pub fn account_identity_script() -> &'static str {
    include_str!("../../collector-scripts/account-identity.js")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecoveryProbeStep {
    Complete(ObservedHomeIdentityV1),
    TryOneClickLogin,
    Wait,
}

fn recovery_probe_step(
    probe: &SessionProbeV1,
    one_click_login_dispatched: bool,
) -> Result<RecoveryProbeStep, String> {
    match probe {
        SessionProbeV1::Authenticated(identity) => {
            Ok(RecoveryProbeStep::Complete(identity.clone()))
        }
        SessionProbeV1::Unauthenticated { .. } if !one_click_login_dispatched => {
            Ok(RecoveryProbeStep::TryOneClickLogin)
        }
        SessionProbeV1::Unauthenticated { .. } | SessionProbeV1::Indeterminate { .. } => {
            Ok(RecoveryProbeStep::Wait)
        }
        SessionProbeV1::Conflicting { .. } => Err("account_identity_ambiguous".to_string()),
    }
}

async fn wait_for_recovered_home_identity(
    page: &super::cdp::CdpPage,
    deadline: Deadline,
    cancel_file: &std::path::Path,
) -> Result<ObservedHomeIdentityV1, String> {
    let mut one_click_login_dispatched = false;
    let mut closed = page.subscribe_closed();
    loop {
        if cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let evaluate = deadline
            .run(page.evaluate::<Value>(include_str!("../../collector-scripts/session-probe.js")));
        tokio::pin!(evaluate);
        let page_state = tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            result = &mut evaluate => result?,
        };
        let step = match session_probe_from_value(&page_state) {
            Ok(probe) => recovery_probe_step(&probe, one_click_login_dispatched)?,
            Err(_) => RecoveryProbeStep::Wait,
        };
        match step {
            RecoveryProbeStep::Complete(identity) => return Ok(identity),
            RecoveryProbeStep::TryOneClickLogin => {
                if super::login_actions::click_login_action_once(
                    page,
                    super::login_actions::LoginAction::OneClickLogin,
                    deadline,
                )
                .await?
                {
                    one_click_login_dispatched = true;
                }
            }
            RecoveryProbeStep::Wait => {}
        }
        tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            result = deadline.run(async {
                page.wait_for_timeout(100).await;
                Ok(())
            }) => result?,
        }
    }
}

async fn wait_for_home_identity(
    page: &super::cdp::CdpPage,
    timeout: Duration,
    cancel_file: &std::path::Path,
) -> Result<ObservedHomeIdentityV1, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut closed = page.subscribe_closed();
    loop {
        if cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let evaluate = page.evaluate(include_str!("../../collector-scripts/session-probe.js"));
        tokio::pin!(evaluate);
        let page_state: Value = tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep_until(deadline) => return Err("account_identity_unavailable".to_string()),
            result = &mut evaluate => result.map_err(map_login_evaluate_error)?,
        };
        if !page_state.is_null() {
            match session_probe_from_value(&page_state) {
                Ok(SessionProbeV1::Authenticated(identity)) => return Ok(identity),
                Ok(SessionProbeV1::Conflicting { .. }) => {
                    return Err("account_identity_ambiguous".to_string())
                }
                Ok(_) | Err(_) => {}
            }
        }
        tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep_until(deadline) => return Err("account_identity_unavailable".to_string()),
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    }
}

fn map_login_evaluate_error(error: String) -> String {
    if error == "cdp_target_closed" {
        "cdp_target_closed".to_string()
    } else {
        error
    }
}

async fn wait_for_cancel_file(path: &std::path::Path) {
    loop {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn run_collection(options: CollectOptions) -> Result<(), String> {
    let started_ms = options.frozen_at_ms;
    let started = Instant::now();
    let _ = fs::remove_file(&options.result_path);
    let _ = fs::remove_file(&options.browser_pid_file);
    let mut last_count = 0;
    let mut last_dom = 0;
    let mut phase = "launching".to_string();
    let mut retries = 0;
    let outcome: Result<(), String> = async {
        if options.cancel_file.exists() { return Err("collector_cancelled".to_string()); }
        let known_ids = load_archive_known_ids(&options)?;
        let choice = resolve_collection_browser(&options)?;
        let session = launch_browser_session(&choice.executable_path, &options.account_profile_root, choice.browser_id, options.headless).await?;
        let record = BrowserProcessRecordV2 { schema_version: 2, pid: session.process.pid, creation_time_100ns: session.process.creation_time_100ns.clone(), browser_id: choice.browser_id, executable_path: choice.executable_path.to_string_lossy().to_string(), profile_path: session.profile_path.to_string_lossy().to_string(), user_data_dir_argument: session.user_data_dir_argument.clone(), devtools_port: session.port, browser_websocket_path: session.browser_websocket_path.clone(), started_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true) };
        write_collect_json(&options, &options.browser_pid_file, &record)?;
        session.page.block_resources(options.block_images, options.block_media).await?;
        if options.source == SyncSourceV1::Favorites && options.discover_remote_folders {
            let _ = session.page.enable_network().await;
        }
        session.page.navigate(&options.page_url).await?;
        if options.source != SyncSourceV1::History {
            run_archive_collection(
                &options,
                &session,
                known_ids.as_ref(),
                started,
                &mut phase,
                &mut retries,
                &mut last_count,
                &mut last_dom,
            )
            .await?;
            persist_validation(&options.account_profile_root, &choice, DOUYIN_HISTORY_URL)?;
            let _ = fs::remove_file(&options.diagnostic_path);
            session.page.close().await;
            return Ok(());
        }
        let mut raw = wait_ready(&session.page, Duration::from_millis(options.hard_timeout_ms.min(60_000)), Some(&options.cancel_file)).await?;
        persist_validation(&options.account_profile_root, &choice, &options.page_url)?;
        let range = collection_range(&options)?;
        let mut expected: Option<HistoryTimeUnit> = None; let mut previous = Vec::new(); let mut last_progress = Instant::now();
        loop {
            if options.cancel_file.exists() { return Err("collector_cancelled".to_string()); }
            if started.elapsed().as_millis() as u64 >= options.hard_timeout_ms { return Err("hard_timeout".to_string()); }
            let (unit, records) = super::normalization::normalize_snapshot(&raw.items, started_ms, expected)?; expected = Some(unit);
            validate_growth(&previous, &records)?; last_count = records.len(); last_dom = raw.dom_card_count as usize;
            if records.len() >= options.max_records { return Err("record_limit_exceeded".to_string()); }
            if records.len() > previous.len() { last_progress = Instant::now(); retries = 0; }
            phase = "validating".into(); write_collect_json(&options, &options.progress_path, &progress(options.account_id, options.sync_job_id, &phase, records.len(), started.elapsed().as_millis() as i64, retries, true))?;
            let bottom = if raw.no_more_visible && !raw.loading_visible { let second = stable_snapshot(&session.page, &options.cancel_file).await?; raw.scroll_top == second.scroll_top && raw.scroll_height == second.scroll_height && second.no_more_visible && !second.loading_visible } else { false };
            if range_complete(&records, &range, bottom) {
                phase = "finalizing".into();
                let final_raw = stable_snapshot(&session.page, &options.cancel_file).await?;
                let (_, final_records) = super::normalization::normalize_snapshot(&final_raw.items, started_ms, expected)?;
                let filtered = filter_range(final_records, &range);
                let result = finish_result(options.account_id, options.sync_job_id, started_ms, range.clone(), unit, filtered)?;
                write_bytes_atomically(
                    &options.result_path,
                    &encode_collection_result(&options, result)?,
                )?;
                let _ = fs::remove_file(&options.diagnostic_path);
                session.page.close().await; return Ok(());
            }
            if last_progress.elapsed() >= Duration::from_secs(30) { retries += 1; if retries > 3 { return Err("state_retry_exhausted".to_string()); } }
            previous = records; phase = "scrolling".into();
            let _: Value = session.page.call_function(
                include_str!("../../collector-scripts/continuous-scroll.js"),
                json!({"startRecordCount":previous.len(),"source":"history","recordSelector":"ul[data-e2e=\"scroll-list\"] > li.record_card","scrollSpeed":options.scroll_speed,"maxDurationMs":30_000}),
            ).await?; raw = read_raw_snapshot(&session.page).await?;
        }
    }.await;
    cleanup_collection_artifacts_after_exit(&options, outcome.is_err());
    if let Err(error) = &outcome {
        let described = describe_collector_error(error);
        let cancelled = error == "collector_cancelled" || options.cancel_file.exists();
        let diagnostic = CollectorDiagnostic {
            schema_version: COLLECTOR_SCHEMA_VERSION,
            account_id: options.account_id,
            sync_job_id: options.sync_job_id,
            status: if cancelled { "cancelled" } else { "failed" }.into(),
            phase: if cancelled { "cancelled".into() } else { phase },
            error_code: if cancelled {
                "collector_cancelled".into()
            } else {
                described.code
            },
            message: described.message,
            record_count: last_count as i64,
            dom_card_count: last_dom as i64,
            calendar_time_zone: COLLECTOR_TIME_ZONE.into(),
            elapsed_ms: started.elapsed().as_millis() as i64,
            no_progress_ms: 0,
            retry_count: retries,
        };
        let _ = write_collect_json(&options, &options.diagnostic_path, &diagnostic);
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn run_archive_collection(
    options: &CollectOptions,
    session: &CdpBrowserSession,
    known_ids: Option<&HashSet<String>>,
    started: Instant,
    phase: &mut String,
    retries: &mut i64,
    last_count: &mut usize,
    last_dom: &mut usize,
) -> Result<(), String> {
    let range = archive_range(options)?;
    let run_mode = options
        .run_mode
        .ok_or_else(|| "invalid_archive_run_mode".to_string())?;
    let empty_known_ids = HashSet::new();
    let known_ids = known_ids.unwrap_or(&empty_known_ids);
    if options.source == SyncSourceV1::Favorites {
        // Douyin renders the default favorites and the folder directory in separate tab states.
        activate_favorites_default_view(&session.page, &options.cancel_file).await?;
    } else if options.source == SyncSourceV1::RemoteFolder {
        activate_favorites_folder_view(&session.page, &options.cancel_file).await?;
        let selected_folder_name = wait_for_remote_folder_selection(
            &session.page,
            &options.source_instance_key,
            &options.page_url,
            Duration::from_millis(options.hard_timeout_ms.min(60_000)),
            &options.cancel_file,
        )
        .await?;
        let (archive, final_raw) = collect_archive_view(
            options,
            session,
            known_ids,
            started,
            range,
            run_mode,
            phase,
            retries,
            last_count,
            last_dom,
        )
        .await?;
        let folder_identity = if final_raw.active_remote_folder_id.is_none()
            && final_raw.active_remote_folder_name.is_none()
        {
            (options.source_instance_key.clone(), selected_folder_name)
        } else {
            normalize_active_remote_folder(
                final_raw.active_remote_folder_id.as_ref(),
                final_raw.active_remote_folder_name.as_ref(),
                &options.source_instance_key,
            )?
        };
        let result = archive_result_for_source(
            options,
            archive,
            None,
            Some(folder_identity),
        )?;
        write_bytes_atomically(
            &options.result_path,
            &encode_source_collection_result(options, result)?,
        )?;
        return Ok(());
    }

    let (archive, final_raw) = collect_archive_view(
        options,
        session,
        known_ids,
        started,
        range,
        run_mode,
        phase,
        retries,
        last_count,
        last_dom,
    )
    .await?;

    let discovery = if options.source == SyncSourceV1::Favorites {
        if options.discover_remote_folders {
            activate_favorites_folder_view(&session.page, &options.cancel_file).await?;
            Some(RemoteFolderDiscoveryResultV1::Success(
                collect_remote_folder_discovery(
                    options,
                    session,
                    started,
                    phase,
                    retries,
                    last_dom,
                )
                .await?,
            ))
        } else {
            Some(RemoteFolderDiscoveryResultV1::NotRequested)
        }
    } else {
        None
    };
    let folder_identity = if options.source == SyncSourceV1::RemoteFolder {
        Some(normalize_active_remote_folder(
            final_raw.active_remote_folder_id.as_ref(),
            final_raw.active_remote_folder_name.as_ref(),
            &options.source_instance_key,
        )?)
    } else {
        None
    };
    let result = archive_result_for_source(options, archive, discovery, folder_identity)?;
    write_bytes_atomically(
        &options.result_path,
        &encode_source_collection_result(options, result)?,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn collect_archive_view(
    options: &CollectOptions,
    session: &CdpBrowserSession,
    known_ids: &HashSet<String>,
    started: Instant,
    range: crate::settings_store::ArchiveListRangeV1,
    run_mode: ArchiveRunModeV1,
    phase: &mut String,
    retries: &mut i64,
    last_count: &mut usize,
    last_dom: &mut usize,
) -> Result<(ArchiveCollectionSuccessV1, RawArchiveSnapshot), String> {
    let snapshot_options = ArchiveSnapshotOptions {
        source: options.source,
        discover_remote_folders: false,
        requested_remote_folder_id: (options.source == SyncSourceV1::RemoteFolder)
            .then(|| options.source_instance_key.clone()),
    };
    let mut raw = wait_archive_ready(
        &session.page,
        &snapshot_options,
        &options.page_url,
        Duration::from_millis(options.hard_timeout_ms.min(60_000)),
        &options.cancel_file,
    )
    .await?;
    let mut expected_time_unit: Option<ArchiveTimeUnit> = None;
    let mut previous = Vec::new();
    let mut last_progress = Instant::now();
    loop {
        ensure_archive_collection_active(options, started)?;
        let (time_unit, records) = normalize_archive_items(&raw.items, expected_time_unit)?;
        expected_time_unit = time_unit;
        validate_archive_growth(&previous, &records)?;
        *last_count = records.len();
        *last_dom = usize::try_from(raw.dom_card_count).unwrap_or(0);
        if archive_snapshot_made_progress(&previous, &records) {
            last_progress = Instant::now();
            *retries = 0;
        }
        *phase = "validating".to_string();
        write_collect_json(
            options,
            &options.progress_path,
            &progress(
                options.account_id,
                options.sync_job_id,
                phase,
                records.len(),
                started.elapsed().as_millis() as i64,
                *retries,
                true,
            ),
        )?;
        let terminal_without_progress = *retries >= 3;
        let should_finalize = matches!(
            decide_archive_stop(
                records.clone(),
                ArchiveStopInput {
                    range,
                    run_mode,
                    cutoff_ms: options.cutoff_ms,
                    bottom_confirmed: false,
                    terminal_without_progress,
                    consecutive_known: consecutive_known_tail(&records, known_ids),
                },
            )?,
            ArchiveStopDecision::Finished(_)
        ) || (raw.no_more_visible && !raw.loading_visible);
        if should_finalize {
            *phase = "finalizing".to_string();
            let (final_raw, bottom_confirmed) =
                stable_archive_snapshot(&session.page, &snapshot_options, &options.cancel_file)
                    .await?;
            let (final_time_unit, final_records) =
                normalize_archive_items(&final_raw.items, expected_time_unit)?;
            expected_time_unit = final_time_unit;
            validate_archive_growth(&records, &final_records)?;
            let final_consecutive_known = consecutive_known_tail(&final_records, known_ids);
            if let ArchiveStopDecision::Finished(archive) = decide_archive_stop(
                final_records,
                ArchiveStopInput {
                    range,
                    run_mode,
                    cutoff_ms: options.cutoff_ms,
                    bottom_confirmed,
                    terminal_without_progress,
                    consecutive_known: final_consecutive_known,
                },
            )? {
                return Ok((archive, final_raw));
            }
            raw = final_raw;
            continue;
        }
        if last_progress.elapsed() >= Duration::from_secs(30) {
            *retries += 1;
        }
        previous = records;
        *phase = "scrolling".to_string();
        run_archive_scroll(options, session, previous.len(), started).await?;
        raw = read_raw_archive_snapshot(&session.page, &snapshot_options).await?;
    }
}

fn archive_result_for_source(
    options: &CollectOptions,
    archive: ArchiveCollectionSuccessV1,
    discovery: Option<RemoteFolderDiscoveryResultV1>,
    folder_identity: Option<(String, String)>,
) -> Result<SourceExecutionResultV1, String> {
    match options.source {
        SyncSourceV1::Likes if discovery.is_none() && folder_identity.is_none() => {
            Ok(SourceExecutionResultV1::Likes(archive))
        }
        SyncSourceV1::Favorites if folder_identity.is_none() => Ok(
            SourceExecutionResultV1::Favorites(FavoriteCollectionSuccessV1 {
                archive,
                remote_folder_discovery: discovery
                    .ok_or_else(|| "remote_folder_discovery_request_mismatch".to_string())?,
            }),
        ),
        SyncSourceV1::RemoteFolder if discovery.is_none() => {
            let (remote_folder_id, name) =
                folder_identity.ok_or_else(|| "remote_folder_not_selected".to_string())?;
            if remote_folder_id != options.source_instance_key {
                return Err("remote_folder_not_selected".to_string());
            }
            Ok(SourceExecutionResultV1::RemoteFolder(
                RemoteFolderCollectionSuccessV1 {
                    remote_folder_id,
                    name,
                    archive,
                },
            ))
        }
        _ => Err("sync_executor_source_mismatch".to_string()),
    }
}

async fn activate_favorites_default_view(
    page: &super::cdp::CdpPage,
    cancel_file: &std::path::Path,
) -> Result<(), String> {
    const PROFILE_ROOT_URL: &str = "https://www.douyin.com/user/self";
    let mut deadline = Instant::now() + Duration::from_secs(15);
    let mut profile_root_fallback_used = false;
    let mut closed = page.subscribe_closed();
    loop {
        if cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        match click_favorite_tab(page).await {
            Ok(()) => {
                page.wait_for_timeout(250).await;
                return Ok(());
            }
            Err(error)
                if matches!(
                    error.as_str(),
                    "favorite_tab_not_found" | "favorite_navigation_not_visible"
                ) && Instant::now() < deadline => {}
            Err(error)
                if matches!(
                    error.as_str(),
                    "favorite_tab_not_found" | "favorite_navigation_not_visible"
                ) && !profile_root_fallback_used =>
            {
                profile_root_fallback_used = true;
                page.navigate(PROFILE_ROOT_URL).await?;
                deadline = Instant::now() + Duration::from_secs(15);
            }
            Err(error) if error == "favorite_tab_not_found" => return Ok(()),
            Err(error) => return Err(error),
        }
        tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep(Duration::from_millis(150)) => {}
            _ = tokio::time::sleep_until(deadline.into()) => {
                if profile_root_fallback_used {
                    return Ok(());
                }
                profile_root_fallback_used = true;
                page.navigate(PROFILE_ROOT_URL).await?;
                deadline = Instant::now() + Duration::from_secs(15);
            }
        }
    }
}

async fn activate_favorites_folder_view(
    page: &super::cdp::CdpPage,
    cancel_file: &std::path::Path,
) -> Result<(), String> {
    const PROFILE_ROOT_URL: &str = "https://www.douyin.com/user/self";
    let mut deadline = Instant::now() + Duration::from_secs(15);
    let mut profile_root_fallback_used = false;
    let mut closed = page.subscribe_closed();
    loop {
        if cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        match click_favorite_folders_tab(page).await {
            Ok(()) => {
                page.wait_for_timeout(250).await;
                return Ok(());
            }
            Err(error)
                if matches!(
                    error.as_str(),
                    "favorite_folder_tab_not_found" | "favorite_navigation_not_visible"
                ) && Instant::now() < deadline => {}
            Err(error)
                if matches!(
                    error.as_str(),
                    "favorite_folder_tab_not_found" | "favorite_navigation_not_visible"
                ) && !profile_root_fallback_used =>
            {
                profile_root_fallback_used = true;
                page.navigate(PROFILE_ROOT_URL).await?;
                let _ = click_favorite_tab(page).await;
                page.wait_for_timeout(250).await;
                deadline = Instant::now() + Duration::from_secs(15);
            }
            Err(error) => return Err(error),
        }
        tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep(Duration::from_millis(150)) => {}
            _ = tokio::time::sleep_until(deadline.into()) => {
                if profile_root_fallback_used {
                    return Err("archive_page_not_ready".to_string());
                }
                profile_root_fallback_used = true;
                page.navigate(PROFILE_ROOT_URL).await?;
                let _ = click_favorite_tab(page).await;
                page.wait_for_timeout(250).await;
                deadline = Instant::now() + Duration::from_secs(15);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn collect_remote_folder_discovery(
    options: &CollectOptions,
    session: &CdpBrowserSession,
    started: Instant,
    phase: &mut String,
    retries: &mut i64,
    last_dom: &mut usize,
) -> Result<RemoteFolderDiscoverySuccessV1, String> {
    let remaining_ms = options
        .hard_timeout_ms
        .saturating_sub(started.elapsed().as_millis() as u64);
    if remaining_ms == 0 {
        return Err("hard_timeout".to_string());
    }
    if let Some(network_result) = try_network_folder_discovery(
        &session.page,
        Duration::from_millis(remaining_ms.min(2_000)),
        &options.cancel_file,
    )
    .await?
    {
        *last_dom = network_result.folders.len();
        return Ok(network_result);
    }
    let timeout = Duration::from_millis(remaining_ms.min(60_000));
    let mut snapshot = wait_remote_folder_ready(
        &session.page,
        timeout,
        &options.cancel_file,
    )
    .await?;
    let mut previous = Vec::new();
    let mut last_progress = Instant::now();
    let mut local_retries = 0_i64;
    loop {
        ensure_archive_collection_active(options, started)?;
        let folders = normalize_remote_folders(&snapshot.folders)?;
        *last_dom = folders.len();
        if folders != previous {
            previous = folders.clone();
            last_progress = Instant::now();
            local_retries = 0;
        }
        let bottom_candidate = remote_folder_bottom_candidate(&snapshot);
        let terminal_without_progress = local_retries >= 3;
        let stop_without_bottom = matches!(
            decide_remote_folder_discovery(
                folders.clone(),
                false,
                terminal_without_progress,
            )?,
            RemoteFolderDiscoveryDecision::Finished(_)
        );
        if bottom_candidate || stop_without_bottom {
            *phase = "finalizing".to_string();
            let (final_snapshot, bottom_confirmed) = stable_remote_folder_snapshot(
                &session.page,
                &options.cancel_file,
                started,
                options.hard_timeout_ms,
            )
            .await?;
            let final_folders = normalize_remote_folders(&final_snapshot.folders)?;
            if let RemoteFolderDiscoveryDecision::Finished(result) =
                decide_remote_folder_discovery(
                    final_folders,
                    bottom_confirmed || remote_folder_bottom_candidate(&final_snapshot),
                    terminal_without_progress,
                )?
            {
                return Ok(result);
            }
            snapshot = final_snapshot;
            continue;
        }
        if last_progress.elapsed() >= Duration::from_secs(30) {
            local_retries += 1;
            *retries = local_retries;
        }
        *phase = "scrolling".to_string();
        scroll_remote_folder_page(
            options,
            session,
            snapshot.folders.len(),
            started,
        )
        .await?;
        let remaining = options
            .hard_timeout_ms
            .saturating_sub(started.elapsed().as_millis() as u64);
        let deadline = Deadline::after(Duration::from_millis(remaining.min(60_000)));
        snapshot = read_remote_folder_page_snapshot_with_deadline(&session.page, deadline).await?;
    }
}

async fn try_network_folder_discovery(
    page: &super::cdp::CdpPage,
    timeout: Duration,
    cancel_file: &std::path::Path,
) -> Result<Option<RemoteFolderDiscoverySuccessV1>, String> {
    let deadline = Instant::now() + timeout;
    let mut closed = page.subscribe_closed();
    loop {
        if cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        let response = tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            result = tokio::time::timeout(remaining.min(Duration::from_millis(200)), page.next_network_response()) => {
                match result {
                    Ok(Ok(response)) => response,
                    Ok(Err(error)) => return Err(error),
                    Err(_) => continue,
                }
            }
        };
        if !is_remote_folder_list_response(&response) {
            continue;
        }
        let body = match tokio::time::timeout(
            remaining.min(Duration::from_secs(2)),
            page.response_body(&response.request_id),
        )
        .await
        {
            Ok(Ok(body)) => body,
            Ok(Err(_)) | Err(_) => continue,
        };
        let parsed = match parse_remote_folder_list_response(&body) {
            Ok(parsed) if !parsed.has_more => parsed,
            Ok(_) | Err(_) => return Ok(None),
        };
        let raw_folders = parsed.folders;
        let folders = normalize_remote_folders(&raw_folders)?;
        return match decide_remote_folder_discovery(folders, true, false)? {
            RemoteFolderDiscoveryDecision::Finished(result) => Ok(Some(result)),
            RemoteFolderDiscoveryDecision::Continue => Ok(None),
        };
    }
}

fn remote_folder_bottom_candidate(snapshot: &RemoteFolderPageSnapshot) -> bool {
    if snapshot.loading_visible {
        return false;
    }
    let at_bottom = snapshot.scroll_top + snapshot.client_height >= snapshot.scroll_height - 1.0;
    snapshot.no_more_visible && at_bottom || !snapshot.can_scroll
}

async fn wait_remote_folder_ready(
    page: &super::cdp::CdpPage,
    timeout: Duration,
    cancel_file: &std::path::Path,
) -> Result<RemoteFolderPageSnapshot, String> {
    let deadline = Instant::now() + timeout;
    let mut closed = page.subscribe_closed();
    loop {
        if cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("archive_page_not_ready".to_string());
        }
        let should_nudge = match read_remote_folder_page_snapshot_with_deadline(
            page,
            Deadline::after(remaining.min(Duration::from_secs(2))),
        )
        .await
        {
            Ok(snapshot)
                if snapshot.kind == "folders"
                    || (snapshot.kind == "empty"
                        && snapshot.no_more_visible
                        && !snapshot.loading_visible) => return Ok(snapshot),
            Ok(_) => true,
            Err(error)
                if matches!(
                    error.as_str(),
                    "remote_folder_list_not_found" | "remote_folder_snapshot_decode_failed"
                ) => true,
            Err(error) => return Err(error),
        };
        if should_nudge {
                if let Ok(target) = read_archive_wheel_target_with_deadline(
                    page,
                    SyncSourceV1::Favorites,
                    Deadline::after(remaining.min(Duration::from_secs(2))),
                )
                .await
                {
                    page.dispatch_mouse_wheel_with_deadline(
                        target.x,
                        target.y,
                        120.0,
                        Deadline::after(remaining.min(Duration::from_secs(2))),
                    )
                        .await?;
                }
        }
        tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep(Duration::from_millis(150)) => {}
            _ = tokio::time::sleep_until(deadline.into()) => return Err("archive_page_not_ready".to_string()),
        }
    }
}

async fn stable_remote_folder_snapshot(
    page: &super::cdp::CdpPage,
    cancel_file: &std::path::Path,
    started: Instant,
    hard_timeout_ms: u64,
) -> Result<(RemoteFolderPageSnapshot, bool), String> {
    for _ in 0..3 {
        if cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let remaining = hard_timeout_ms.saturating_sub(started.elapsed().as_millis() as u64);
        if remaining == 0 {
            return Err("hard_timeout".to_string());
        }
        let first = read_remote_folder_page_snapshot_with_deadline(
            page,
            Deadline::after(Duration::from_millis(remaining.min(2_000))),
        )
        .await?;
        if first.loading_visible {
            page.wait_for_timeout(250).await;
            continue;
        }
        tokio::select! {
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep(Duration::from_millis(750)) => {}
        }
        let second = read_remote_folder_page_snapshot_with_deadline(
            page,
            Deadline::after(Duration::from_millis(remaining.min(2_000))),
        )
        .await?;
        if first == second {
            return Ok((second.clone(), remote_folder_bottom_candidate(&second)));
        }
    }
    Err("final_stability_failed".to_string())
}

async fn scroll_remote_folder_page(
    options: &CollectOptions,
    session: &CdpBrowserSession,
    start_count: usize,
    started: Instant,
) -> Result<(), String> {
    let remaining_ms = options
        .hard_timeout_ms
        .saturating_sub(started.elapsed().as_millis() as u64);
    if remaining_ms == 0 {
        return Err("hard_timeout".to_string());
    }
    let budget = Duration::from_millis(30_000_u64.min(remaining_ms));
    let cycle_reaches_task_deadline = remaining_ms <= 30_000;
    let deadline = Deadline::after(budget);
    let tick = Duration::from_millis(40);
    let mut closed = session.page.subscribe_closed();
    loop {
        if options.cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let snapshot = match read_remote_folder_page_snapshot_with_deadline(&session.page, deadline)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) if error == "collector_deadline_exceeded" => {
                return if cycle_reaches_task_deadline {
                    Err("hard_timeout".to_string())
                } else {
                    Ok(())
                };
            }
            Err(error) => return Err(error),
        };
        if snapshot.folders.len() > start_count {
            return Ok(());
        }
        if snapshot.no_more_visible && !snapshot.loading_visible {
            return Ok(());
        }
        let wheel_result = session
            .page
            .dispatch_mouse_wheel_with_deadline(
                snapshot.anchor_x,
                snapshot.anchor_y,
                (options.scroll_speed as f64 * tick.as_secs_f64()).max(1.0),
                deadline,
            )
            .await;
        if let Err(error) = wheel_result {
            if error == "collector_deadline_exceeded" {
                return if cycle_reaches_task_deadline {
                    Err("hard_timeout".to_string())
                } else {
                    Ok(())
                };
            }
            return Err(error);
        }
        let remaining = match deadline.remaining() {
            Ok(remaining) => remaining,
            Err(_) => {
                return if cycle_reaches_task_deadline {
                    Err("hard_timeout".to_string())
                } else {
                    Ok(())
                };
            }
        };
        let wait = remaining.min(tick);
        tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(&options.cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep(wait) => {
                if wait >= remaining {
                    return if cycle_reaches_task_deadline {
                        Err("hard_timeout".to_string())
                    } else {
                        Ok(())
                    };
                }
            }
        }
    }
}

fn archive_range(
    options: &CollectOptions,
) -> Result<crate::settings_store::ArchiveListRangeV1, String> {
    let CollectRange::Archive(range) = options.range else {
        return Err("sync_executor_source_mismatch".to_string());
    };
    match range {
        crate::settings_store::ArchiveListRangeV1::RecentDays { days } => {
            if options.cutoff_ms != Some(shanghai_cutoff_ms(options.frozen_at_ms, days)?) {
                return Err("range_cutoff_mismatch".to_string());
            }
        }
        _ if options.cutoff_ms.is_some() => return Err("range_cutoff_mismatch".to_string()),
        _ => {}
    }
    Ok(range)
}

fn ensure_archive_collection_active(
    options: &CollectOptions,
    started: Instant,
) -> Result<(), String> {
    if options.cancel_file.exists() {
        Err("collector_cancelled".to_string())
    } else if started.elapsed().as_millis() as u64 >= options.hard_timeout_ms {
        Err("hard_timeout".to_string())
    } else {
        Ok(())
    }
}

async fn wait_for_remote_folder_selection(
    page: &super::cdp::CdpPage,
    requested_id: &str,
    page_url: &str,
    timeout: Duration,
    cancel_file: &std::path::Path,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let mut closed = page.subscribe_closed();
    loop {
        if cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let session_state: Value = page
            .evaluate(include_str!("../../collector-scripts/session-probe.js"))
            .await?;
        if matches!(
            session_probe_from_page_value(&session_state, page_url),
            Ok(SessionProbeV1::Unauthenticated { .. })
        ) {
            return Err("session_expired".to_string());
        }
        let folder_snapshot = read_remote_folder_page_snapshot_with_deadline(
            page,
            Deadline::after(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(2)),
            ),
        )
        .await;
        if let Ok(snapshot) = &folder_snapshot {
            let folders = normalize_remote_folders(&snapshot.folders)?;
            if let Some(folder) = folders
                .iter()
                .find(|folder| folder.remote_folder_id == requested_id)
            {
                let name = folder.name.clone();
                match select_remote_folder_with_name(page, requested_id, &name).await {
                    Ok(()) => return Ok(name),
                    Err(error)
                        if error == "remote_folder_not_selected"
                            && Instant::now() < deadline => {}
                    Err(error) => return Err(error),
                }
            } else if snapshot.kind == "folders"
                || (snapshot.kind == "empty"
                    && snapshot.no_more_visible
                    && !snapshot.loading_visible)
            {
                return Err("remote_folder_not_found".to_string());
            }
        }
        if let Err(error) = &folder_snapshot {
            if !remote_folder_selection_error_is_transient(error)
                && error != "collector_deadline_exceeded"
            {
                return Err(error.clone());
            }
        }
        tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep_until(deadline.into()) => return Err("archive_page_not_ready".to_string()),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

fn remote_folder_selection_error_is_transient(error: &str) -> bool {
    matches!(
        error,
        "remote_folder_list_not_found"
            | "remote_folder_fiber_not_found"
            | "remote_folder_react_list_not_found"
            | "remote_folder_not_visible"
            | "remote_folder_snapshot_decode_failed"
            | "favorite_folder_tab_not_found"
            | "favorite_navigation_not_visible"
            | "archive_list_not_found"
            | "archive_react_list_not_found"
            | "empty_archive_unconfirmed"
    )
}

async fn wait_archive_ready(
    page: &super::cdp::CdpPage,
    snapshot_options: &ArchiveSnapshotOptions,
    page_url: &str,
    timeout: Duration,
    cancel_file: &std::path::Path,
) -> Result<RawArchiveSnapshot, String> {
    let deadline = Instant::now() + timeout;
    let mut stable_empty = 0_u8;
    let mut closed = page.subscribe_closed();
    loop {
        if cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let (snapshot_result, session_result) = tokio::join!(
            read_raw_archive_snapshot(page, snapshot_options),
            page.evaluate::<Value>(include_str!("../../collector-scripts/session-probe.js")),
        );
        let should_nudge = matches!(
            &snapshot_result,
            Err(error) if matches!(
                error.as_str(),
                "archive_list_not_found" | "empty_archive_unconfirmed"
            )
        );
        if let Ok(value) = session_result {
            if matches!(
                session_probe_from_page_value(&value, page_url),
                Ok(SessionProbeV1::Unauthenticated { .. })
            ) {
                return Err("session_expired".to_string());
            }
        }
        match snapshot_result {
            Ok(snapshot) if snapshot.kind == "records" => return Ok(snapshot),
            Ok(snapshot)
                if snapshot.kind == "empty"
                    && snapshot.no_more_visible
                    && !snapshot.loading_visible =>
            {
                stable_empty += 1;
                if stable_empty >= 3 {
                    return Ok(snapshot);
                }
            }
            Ok(_) | Err(_) => stable_empty = 0,
        }
        if should_nudge && Instant::now() < deadline {
            let nudge_deadline = Deadline::after(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(2)),
            );
            if let Ok(target) = read_archive_wheel_target_with_deadline(
                page,
                snapshot_options.source,
                nudge_deadline,
            )
            .await
            {
                if !target.no_more_visible || target.loading_visible {
                    page.dispatch_mouse_wheel_with_deadline(
                        target.x,
                        target.y,
                        120.0,
                        nudge_deadline,
                    )
                    .await?;
                }
            }
        }
        tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep_until(deadline.into()) => return Err("archive_page_not_ready".to_string()),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

async fn stable_archive_snapshot(
    page: &super::cdp::CdpPage,
    options: &ArchiveSnapshotOptions,
    cancel_file: &std::path::Path,
) -> Result<(RawArchiveSnapshot, bool), String> {
    for _ in 0..3 {
        if cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let first = read_raw_archive_snapshot(page, options).await?;
        if first.loading_visible {
            tokio::select! {
                _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
            continue;
        }
        tokio::select! {
            _ = wait_for_cancel_file(cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep(Duration::from_millis(750)) => {}
        }
        let _: Value = page
            .evaluate("new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))")
            .await?;
        let second = read_raw_archive_snapshot(page, options).await?;
        if archive_snapshots_stable(&first, &second) {
            let bottom_confirmed = archive_bottom_confirmed(&first, &second);
            return Ok((second, bottom_confirmed));
        }
    }
    Err("final_stability_failed".to_string())
}

async fn run_archive_scroll(
    options: &CollectOptions,
    session: &CdpBrowserSession,
    record_count: usize,
    started: Instant,
) -> Result<(), String> {
    let remaining_ms = options
        .hard_timeout_ms
        .saturating_sub(started.elapsed().as_millis() as u64);
    if remaining_ms == 0 {
        return Err("hard_timeout".to_string());
    }
    let scroll_budget = Duration::from_millis(30_000_u64.min(remaining_ms));
    let cycle_reaches_task_deadline = remaining_ms <= 30_000;
    let scroll_deadline = Deadline::after(scroll_budget);
    let mut closed = session.page.subscribe_closed();
    let tick = Duration::from_millis(40);
    loop {
        if options.cancel_file.exists() {
            return Err("collector_cancelled".to_string());
        }
        let target = match read_archive_wheel_target_with_deadline(
            &session.page,
            options.source,
            scroll_deadline,
        )
        .await
        {
            Ok(target) => target,
            Err(error)
                if matches!(
                    error.as_str(),
                    "archive_list_not_found"
                        | "scroll_target_not_found"
                        | "archive_scroll_target_invalid"
                ) => {
                    tokio::select! {
                        biased;
                        changed = closed.changed() => {
                            let _ = changed;
                            return Err("cdp_target_closed".to_string());
                        }
                        _ = wait_for_cancel_file(&options.cancel_file) => return Err("collector_cancelled".to_string()),
                        _ = tokio::time::sleep(tick) => {}
                    }
                    continue;
                }
            Err(error) if error == "collector_deadline_exceeded" => {
                return if cycle_reaches_task_deadline {
                    Err("hard_timeout".to_string())
                } else {
                    Ok(())
                };
            }
            Err(error) => return Err(error),
        };
        if target.record_count > record_count {
            return Ok(());
        }
        if target.no_more_visible && !target.loading_visible {
            return Ok(());
        }
        let wheel_result = session
            .page
            .dispatch_mouse_wheel_with_deadline(
                target.x,
                target.y,
                (options.scroll_speed as f64 * tick.as_secs_f64()).max(1.0),
                scroll_deadline,
            )
            .await;
        if let Err(error) = wheel_result {
            if error == "collector_deadline_exceeded" {
                return if cycle_reaches_task_deadline {
                    Err("hard_timeout".to_string())
                } else {
                    Ok(())
                };
            }
            return Err(error);
        }
        let remaining = match scroll_deadline.remaining() {
            Ok(remaining) => remaining,
            Err(_) => {
                return if cycle_reaches_task_deadline {
                    Err("hard_timeout".to_string())
                } else {
                    Ok(())
                };
            }
        };
        let wait_duration = remaining.min(tick);
        tokio::select! {
            biased;
            changed = closed.changed() => {
                let _ = changed;
                return Err("cdp_target_closed".to_string());
            }
            _ = wait_for_cancel_file(&options.cancel_file) => return Err("collector_cancelled".to_string()),
            _ = tokio::time::sleep(wait_duration) => {
                if wait_duration >= remaining {
                    return if cycle_reaches_task_deadline {
                        Err("hard_timeout".to_string())
                    } else {
                        Ok(())
                    };
                }
            }
        }
    }
}

fn load_archive_known_ids(options: &CollectOptions) -> Result<Option<HashSet<String>>, String> {
    let Some(path) = options.known_ids_path.as_deref() else {
        return Ok(None);
    };
    let phase = options
        .phase
        .ok_or_else(|| "archive_known_ids_requires_phase".to_string())?;
    let phase_attempt_id = options
        .phase_attempt_id
        .as_deref()
        .ok_or_else(|| "archive_known_ids_requires_phase".to_string())?;
    let snapshot: ArchiveKnownIdsSnapshotV1 =
        read_json(path).map_err(|_| "invalid_archive_known_ids_file".to_string())?;
    let job = SyncJobSnapshotV1 {
        id: options.sync_job_id,
        task_key: TaskKeyV1 {
            account_id: options.account_id,
            source: options.source,
            source_instance_key: options.source_instance_key.clone(),
        },
        trigger: SyncTriggerV1::Manual,
        status: SyncJobStatusV1::Running,
        task_payload_sha256: options.task_payload_sha256.clone(),
    };
    validate_archive_known_ids_snapshot(&snapshot, &job, phase, phase_attempt_id)?;
    Ok(Some(snapshot.aweme_ids.into_iter().collect()))
}

fn cleanup_collection_artifacts_after_exit(options: &CollectOptions, failed: bool) {
    if failed {
        let _ = fs::remove_file(&options.result_path);
    }
}

fn collection_range(options: &CollectOptions) -> Result<CollectorRange, String> {
    match options.range {
        CollectRange::History(crate::settings_store::HistoryRangeV1::Days { days }) => {
            let cutoff_ms = options
                .cutoff_ms
                .ok_or_else(|| "range_cutoff_mismatch".to_string())?;
            if cutoff_ms != shanghai_cutoff_ms(options.frozen_at_ms, days)? {
                return Err("range_cutoff_mismatch".to_string());
            }
            Ok(CollectorRange::Days { days, cutoff_ms })
        }
        CollectRange::History(crate::settings_store::HistoryRangeV1::Bottom)
            if options.cutoff_ms.is_none() =>
        {
            Ok(CollectorRange::Bottom)
        }
        CollectRange::History(crate::settings_store::HistoryRangeV1::Bottom) => {
            Err("range_cutoff_mismatch".to_string())
        }
        CollectRange::Archive(_) => Err("sync_executor_source_mismatch".to_string()),
    }
}

fn resolve_collection_browser(options: &CollectOptions) -> Result<BrowserChoice, String> {
    let choice = resolve_browser(
        &options.account_profile_root,
        options.browser,
        options.headless,
    )?;
    let expected = BrowserProfileIdentityV1 {
        browser_id: choice.browser_id,
        executable_path: choice.executable_path.to_string_lossy().into_owned(),
        profile_path: choice.profile_path.to_string_lossy().into_owned(),
        profile_generation: options.profile_generation.clone(),
    };
    let current = read_matching_browser_profile_identity(&options.account_profile_root, &expected)?;
    validate_collection_browser_identity(options, &choice, &current)?;
    Ok(choice)
}

fn validate_collection_browser_identity(
    options: &CollectOptions,
    choice: &BrowserChoice,
    current: &BrowserProfileIdentityV1,
) -> Result<(), String> {
    let expected_browser = match options.browser {
        super::cli::BrowserPreference::Edge => BrowserId::Edge,
        super::cli::BrowserPreference::Chrome => BrowserId::Chrome,
        super::cli::BrowserPreference::Auto => {
            return Err("browser_profile_changed".to_string());
        }
    };
    if choice.browser_id != expected_browser
        || current.browser_id != expected_browser
        || current.profile_generation != options.profile_generation
        || browser_executable_path_sha256(&current.executable_path)
            != options.browser_executable_sha256
    {
        return Err("browser_profile_changed".to_string());
    }
    Ok(())
}

fn encode_collection_result(
    options: &CollectOptions,
    result: CollectorSuccessV3,
) -> Result<Vec<u8>, String> {
    encode_source_collection_result(options, SourceExecutionResultV1::History(result))
}

fn encode_source_collection_result(
    options: &CollectOptions,
    result: SourceExecutionResultV1,
) -> Result<Vec<u8>, String> {
    let identity_matches = match &result {
        SourceExecutionResultV1::History(result) => {
            options.source == SyncSourceV1::History
                && options.source_instance_key.is_empty()
                && result.account_id == options.account_id
                && result.sync_job_id == options.sync_job_id
                && result.sync_started_at_ms == options.frozen_at_ms
        }
        SourceExecutionResultV1::Likes(_) => {
            options.source == SyncSourceV1::Likes && options.source_instance_key.is_empty()
        }
        SourceExecutionResultV1::Favorites(_) => {
            options.source == SyncSourceV1::Favorites && options.source_instance_key.is_empty()
        }
        SourceExecutionResultV1::RemoteFolder(result) => {
            options.source == SyncSourceV1::RemoteFolder
                && !options.source_instance_key.is_empty()
                && result.remote_folder_id == options.source_instance_key
        }
    };
    if !identity_matches || result.source() != options.source {
        return Err("sync_result_identity_mismatch".to_string());
    }
    let envelope = SyncExecutionEnvelopeV1 {
        schema_version: 1,
        account_id: options.account_id,
        job_id: options.sync_job_id,
        source: options.source,
        source_instance_key: options.source_instance_key.clone(),
        task_payload_sha256: options.task_payload_sha256.clone(),
        result,
    };
    serde_json::to_vec_pretty(&envelope).map_err(|error| error.to_string())
}

fn write_collect_json<T: Serialize>(
    options: &CollectOptions,
    path: &std::path::Path,
    payload: &T,
) -> Result<(), String> {
    let bytes = encode_collect_payload(options, payload)?;
    write_bytes_atomically(path, &bytes)
}

fn encode_collect_payload<T: Serialize>(
    options: &CollectOptions,
    payload: &T,
) -> Result<Vec<u8>, String> {
    match (options.phase, options.phase_attempt_id.as_deref()) {
        (Some(phase), Some(attempt_id)) => {
            let envelope = SyncPhaseEnvelopeV1::new(
                options.account_id,
                options.sync_job_id,
                phase,
                attempt_id,
                payload,
            )?;
            serde_json::to_vec_pretty(&envelope).map_err(|error| error.to_string())
        }
        (None, None) => serde_json::to_vec_pretty(payload).map_err(|error| error.to_string()),
        _ => Err("incomplete_sync_phase_identity".to_string()),
    }
}

fn encode_recover_login_payload<T: Serialize>(
    options: &RecoverLoginOptions,
    payload: &T,
) -> Result<Vec<u8>, String> {
    let envelope = SyncPhaseEnvelopeV1::new(
        options.account_id,
        options.sync_job_id,
        options.phase,
        &options.phase_attempt_id,
        payload,
    )?;
    serde_json::to_vec_pretty(&envelope).map_err(|error| error.to_string())
}

fn write_recover_json<T: Serialize>(
    options: &RecoverLoginOptions,
    path: &std::path::Path,
    payload: &T,
) -> Result<(), String> {
    let bytes = encode_recover_login_payload(options, payload)?;
    write_bytes_atomically(path, &bytes)
}

async fn wait_ready(
    page: &super::cdp::CdpPage,
    timeout: Duration,
    cancel: Option<&std::path::Path>,
) -> Result<RawReactSnapshot, String> {
    let deadline = Instant::now() + timeout;
    let mut stable_empty = 0;
    loop {
        if cancel.is_some_and(|path| path.exists()) {
            return Err("collector_cancelled".into());
        }
        let (snapshot_result, session_result) = tokio::join!(
            read_raw_snapshot(page),
            page.evaluate::<Value>(include_str!("../../collector-scripts/session-probe.js")),
        );
        if let Ok(value) = session_result {
            if matches!(
                session_probe_from_history_value(&value),
                Ok(SessionProbeV1::Unauthenticated { .. })
            ) {
                return Err("session_expired".into());
            }
        }
        match snapshot_result {
            Ok(snapshot) if snapshot.kind == "records" => return Ok(snapshot),
            Ok(snapshot)
                if snapshot.kind == "empty"
                    && snapshot.no_more_visible
                    && !snapshot.loading_visible =>
            {
                stable_empty += 1;
                if stable_empty >= 3 {
                    return Ok(snapshot);
                }
            }
            Ok(_) | Err(_) => stable_empty = 0,
        }
        if Instant::now() >= deadline {
            return Err("history_page_not_ready".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn stable_snapshot(
    page: &super::cdp::CdpPage,
    cancel: &std::path::Path,
) -> Result<RawReactSnapshot, String> {
    for _ in 0..3 {
        if cancel.exists() {
            return Err("collector_cancelled".into());
        }
        let first = read_raw_snapshot(page).await?;
        if first.loading_visible {
            continue;
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
        let _: Value = page
            .evaluate("new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))")
            .await?;
        let second = read_raw_snapshot(page).await?;
        if !second.loading_visible
            && first.items.len() == second.items.len()
            && first.scroll_top == second.scroll_top
            && first.scroll_height == second.scroll_height
        {
            return Ok(second);
        }
    }
    Err("final_stability_failed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::session_probe::SessionStateSource;
    use crate::sync::{SourceExecutionResultV1, SyncExecutionEnvelopeV1, SyncSourceV1};
    use sha2::Digest;
    use std::path::PathBuf;

    fn collect_options(
        phase: Option<crate::sync_orchestration::SyncPhase>,
        phase_attempt_id: Option<&str>,
    ) -> CollectOptions {
        CollectOptions {
            account_id: 11,
            sync_job_id: 7,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
            task_payload_sha256: "c".repeat(64),
            frozen_at_ms: 1_721_234_567_890,
            cutoff_ms: Some(1_720_713_600_000),
            phase,
            phase_attempt_id: phase_attempt_id.map(str::to_string),
            result_path: PathBuf::from("result.json"),
            progress_path: PathBuf::from("progress.json"),
            diagnostic_path: PathBuf::from("diagnostic.json"),
            cancel_file: PathBuf::from("cancel"),
            account_profile_root: PathBuf::from("profile"),
            browser_pid_file: PathBuf::from("browser.pid"),
            collector_identity_path: phase.map(|_| PathBuf::from("collector.pid")),
            known_ids_path: None,
            browser: super::super::cli::BrowserPreference::Auto,
            profile_key: "0123456789abcdef0123456789abcdef".to_string(),
            profile_generation: "fedcba9876543210fedcba9876543210".to_string(),
            browser_executable_sha256: "d".repeat(64),
            page_url: "https://www.douyin.com/user/self?showTab=record".to_string(),
            range: super::super::cli::CollectRange::History(
                crate::settings_store::HistoryRangeV1::Days { days: 7 },
            ),
            run_mode: None,
            discover_remote_folders: false,
            scroll_speed: 6_000,
            headless: true,
            block_media: true,
            block_images: true,
            hard_timeout_ms: 600_000,
            max_records: 50_000,
        }
    }

    #[test]
    fn phased_collect_output_nests_the_exact_success_payload() {
        let options = collect_options(
            Some(crate::sync_orchestration::SyncPhase::Collecting),
            Some("0123456789abcdef0123456789abcdef"),
        );
        let payload = CollectorSuccessV3 {
            schema_version: 3,
            account_id: 11,
            sync_job_id: 7,
            sync_started_at_ms: 1_721_234_567_890,
            calendar_time_zone: "Asia/Shanghai".to_string(),
            range: CollectorRange::Days {
                days: 7,
                cutoff_ms: 1_720_713_600_000,
            },
            detected_history_time_unit: "milliseconds".to_string(),
            identity_hash: "a".repeat(64),
            field_hash: "b".repeat(64),
            records: Vec::new(),
        };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();

        let encoded = encode_collection_result(&options, payload).unwrap();
        let envelope: SyncExecutionEnvelopeV1 = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(envelope.schema_version, 1);
        assert_eq!(envelope.account_id, 11);
        assert_eq!(envelope.job_id, 7);
        assert_eq!(envelope.source, SyncSourceV1::History);
        assert_eq!(envelope.source_instance_key, "");
        assert_eq!(envelope.task_payload_sha256, "c".repeat(64));
        let SourceExecutionResultV1::History(history) = envelope.result else {
            panic!("expected history result");
        };
        assert_eq!(serde_json::to_vec(&history).unwrap(), payload_bytes);
        assert_eq!(history.identity_hash, "a".repeat(64));
        assert_eq!(history.field_hash, "b".repeat(64));
    }

    #[test]
    fn source_collection_output_accepts_archive_variants_for_the_exact_task() {
        use crate::sync::{
            ArchiveCollectionSuccessV1, ArchiveCompletionV1, ArchiveIncompleteReasonV1,
            FavoriteCollectionSuccessV1, RemoteFolderCollectionSuccessV1,
            RemoteFolderDiscoveryResultV1,
        };

        let archive = ArchiveCollectionSuccessV1 {
            records: Vec::new(),
            identity_hash: "a".repeat(64),
            field_hash: "b".repeat(64),
            completion: ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
            },
        };
        for (source, instance, result) in [
            (
                SyncSourceV1::Likes,
                "",
                SourceExecutionResultV1::Likes(archive.clone()),
            ),
            (
                SyncSourceV1::Favorites,
                "",
                SourceExecutionResultV1::Favorites(FavoriteCollectionSuccessV1 {
                    archive: archive.clone(),
                    remote_folder_discovery: RemoteFolderDiscoveryResultV1::NotRequested,
                }),
            ),
            (
                SyncSourceV1::RemoteFolder,
                "folder-a",
                SourceExecutionResultV1::RemoteFolder(RemoteFolderCollectionSuccessV1 {
                    remote_folder_id: "folder-a".to_string(),
                    name: "收藏夹 A".to_string(),
                    archive: archive.clone(),
                }),
            ),
        ] {
            let mut options = collect_options(
                Some(crate::sync_orchestration::SyncPhase::Collecting),
                Some("0123456789abcdef0123456789abcdef"),
            );
            options.source = source;
            options.source_instance_key = instance.to_string();
            options.range =
                CollectRange::Archive(crate::settings_store::ArchiveListRangeV1::RecentItems {
                    count: 20,
                });
            options.run_mode = Some(crate::sync::ArchiveRunModeV1::Incremental);

            let encoded = encode_source_collection_result(&options, result).unwrap();
            let envelope: SyncExecutionEnvelopeV1 = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(envelope.source, source);
            assert_eq!(envelope.source_instance_key, instance);
            assert_eq!(envelope.result.source(), source);
        }
    }

    #[test]
    fn source_collection_output_rejects_a_result_from_another_source() {
        use crate::sync::{
            ArchiveCollectionSuccessV1, ArchiveCompletionV1, ArchiveIncompleteReasonV1,
        };
        let mut options = collect_options(
            Some(crate::sync_orchestration::SyncPhase::Collecting),
            Some("0123456789abcdef0123456789abcdef"),
        );
        options.source = SyncSourceV1::Likes;
        options.range =
            CollectRange::Archive(crate::settings_store::ArchiveListRangeV1::RecentItems {
                count: 20,
            });
        options.run_mode = Some(crate::sync::ArchiveRunModeV1::Incremental);
        let result = SourceExecutionResultV1::Favorites(crate::sync::FavoriteCollectionSuccessV1 {
            archive: ArchiveCollectionSuccessV1 {
                records: Vec::new(),
                identity_hash: "a".repeat(64),
                field_hash: "b".repeat(64),
                completion: ArchiveCompletionV1::Incomplete {
                    reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
                },
            },
            remote_folder_discovery: crate::sync::RemoteFolderDiscoveryResultV1::NotRequested,
        });

        assert_eq!(
            encode_source_collection_result(&options, result).unwrap_err(),
            "sync_result_identity_mismatch"
        );
    }

    #[test]
    fn archive_result_uses_discovery_and_observed_remote_folder_identity() {
        use crate::sync::{
            ArchiveCollectionSuccessV1, ArchiveCompletionV1, ArchiveIncompleteReasonV1,
            RemoteFolderDiscoveryResultV1, RemoteFolderDiscoverySuccessV1,
        };
        let archive = ArchiveCollectionSuccessV1 {
            records: Vec::new(),
            identity_hash: "a".repeat(64),
            field_hash: "b".repeat(64),
            completion: ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
            },
        };
        let discovery = RemoteFolderDiscoveryResultV1::Success(RemoteFolderDiscoverySuccessV1 {
            folders: Vec::new(),
            identity_hash: "c".repeat(64),
            field_hash: "d".repeat(64),
            complete_scan: true,
            bottom_confirmed: true,
            hit_record_limit: false,
        });

        let mut favorites = collect_options(None, None);
        favorites.source = SyncSourceV1::Favorites;
        favorites.discover_remote_folders = true;
        assert!(matches!(
            archive_result_for_source(&favorites, archive.clone(), Some(discovery), None).unwrap(),
            SourceExecutionResultV1::Favorites(crate::sync::FavoriteCollectionSuccessV1 {
                remote_folder_discovery: RemoteFolderDiscoveryResultV1::Success(_),
                ..
            })
        ));

        let mut folder = collect_options(None, None);
        folder.source = SyncSourceV1::RemoteFolder;
        folder.source_instance_key = "folder-a".to_string();
        assert!(matches!(
            archive_result_for_source(
                &folder,
                archive.clone(),
                None,
                Some(("folder-a".to_string(), "收藏夹 A".to_string()))
            )
            .unwrap(),
            SourceExecutionResultV1::RemoteFolder(crate::sync::RemoteFolderCollectionSuccessV1 {
                remote_folder_id,
                name,
                ..
            }) if remote_folder_id == "folder-a" && name == "收藏夹 A"
        ));
        assert_eq!(
            archive_result_for_source(
                &folder,
                archive,
                None,
                Some(("folder-b".to_string(), "收藏夹 B".to_string()))
            )
            .unwrap_err(),
            "remote_folder_not_selected"
        );
    }

    #[test]
    fn remote_folder_selection_retries_only_transient_page_readiness_errors() {
        for error in [
            "remote_folder_list_not_found",
            "remote_folder_fiber_not_found",
            "remote_folder_react_list_not_found",
            "remote_folder_not_visible",
            "remote_folder_snapshot_decode_failed",
            "favorite_folder_tab_not_found",
            "favorite_navigation_not_visible",
            "archive_list_not_found",
            "archive_react_list_not_found",
            "empty_archive_unconfirmed",
        ] {
            assert!(remote_folder_selection_error_is_transient(error));
        }
        for error in [
            "remote_folder_not_found",
            "ambiguous_remote_folder_react_list",
            "remote_folder_not_selected",
        ] {
            assert!(!remote_folder_selection_error_is_transient(error));
        }
    }

    #[test]
    fn collection_range_uses_the_frozen_payload_boundary() {
        let options = collect_options(
            Some(crate::sync_orchestration::SyncPhase::Collecting),
            Some("0123456789abcdef0123456789abcdef"),
        );

        let range = collection_range(&options).unwrap();

        assert!(matches!(
            range,
            CollectorRange::Days {
                days: 7,
                cutoff_ms: 1_720_713_600_000
            }
        ));
    }

    #[test]
    fn collection_browser_identity_rejects_a_changed_executable_hash() {
        let executable = PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe");
        let profile = PathBuf::from(r"C:\app\profiles\0123456789abcdef0123456789abcdef\chrome");
        let mut options = collect_options(
            Some(crate::sync_orchestration::SyncPhase::Collecting),
            Some("0123456789abcdef0123456789abcdef"),
        );
        options.browser = super::super::cli::BrowserPreference::Chrome;
        options.browser_executable_sha256 =
            crate::sync::browser_executable_path_sha256(&executable.to_string_lossy());
        let choice = super::super::browser_runtime::BrowserChoice {
            browser_id: super::super::browser_state::BrowserId::Chrome,
            executable_path: executable.clone(),
            profile_path: profile.clone(),
        };
        let current = super::super::browser_state::BrowserProfileIdentityV1 {
            browser_id: super::super::browser_state::BrowserId::Chrome,
            executable_path: executable.to_string_lossy().into_owned(),
            profile_path: profile.to_string_lossy().into_owned(),
            profile_generation: options.profile_generation.clone(),
        };

        validate_collection_browser_identity(&options, &choice, &current).unwrap();
        options.browser_executable_sha256 = "e".repeat(64);
        assert_eq!(
            validate_collection_browser_identity(&options, &choice, &current),
            Err("browser_profile_changed".to_string())
        );
    }

    #[test]
    fn archive_known_ids_are_validated_before_collection() {
        let root =
            std::env::temp_dir().join(format!("emily-runner-known-ids-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("known.json");
        let phase = crate::sync_orchestration::SyncPhase::Collecting;
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let mut options = collect_options(Some(phase), Some(attempt_id));
        options.source = SyncSourceV1::Likes;
        options.range = CollectRange::Archive(crate::settings_store::ArchiveListRangeV1::All);
        options.run_mode = Some(crate::sync::ArchiveRunModeV1::Incremental);
        options.known_ids_path = Some(path.clone());
        let snapshot = ArchiveKnownIdsSnapshotV1 {
            schema_version: 1,
            account_id: options.account_id,
            sync_job_id: options.sync_job_id,
            phase,
            phase_attempt_id: attempt_id.to_string(),
            source: SyncSourceV1::Likes,
            source_instance_key: String::new(),
            task_payload_sha256: options.task_payload_sha256.clone(),
            aweme_ids: vec!["7650000000000000001".to_string()],
            ids_sha256: "0".repeat(64),
        };
        write_json_atomically(&path, &snapshot).unwrap();
        assert_eq!(
            load_archive_known_ids(&options).unwrap_err(),
            "archive_known_ids_hash_mismatch"
        );

        let mut valid = snapshot;
        valid.ids_sha256 = format!(
            "{:x}",
            sha2::Sha256::digest(serde_json::to_vec(&valid.aweme_ids).unwrap())
        );
        write_json_atomically(&path, &valid).unwrap();
        assert!(load_archive_known_ids(&options)
            .unwrap()
            .unwrap()
            .contains("7650000000000000001"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_collect_output_stays_flat() {
        let options = collect_options(None, None);
        let payload = progress(11, 7, "scrolling", 23, 456, 1, true);

        let encoded = encode_collect_payload(&options, &payload).unwrap();
        let value: Value = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(value["schemaVersion"], json!(3));
        assert_eq!(value["recordCount"], json!(23));
        assert!(value.get("envelopeVersion").is_none());
        assert!(value.get("payload").is_none());
    }

    #[test]
    fn collect_output_rejects_an_incomplete_phase_identity() {
        let options = collect_options(Some(crate::sync_orchestration::SyncPhase::Collecting), None);

        assert_eq!(
            encode_collect_payload(&options, &json!({"status": "failed"})),
            Err("incomplete_sync_phase_identity".to_string())
        );
    }

    #[test]
    fn collection_exit_keeps_browser_identity_for_tauri_cleanup() {
        let root = std::env::temp_dir().join(format!(
            "emily-collection-artifacts-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut options = collect_options(
            Some(crate::sync_orchestration::SyncPhase::Collecting),
            Some("0123456789abcdef0123456789abcdef"),
        );
        options.result_path = root.join("result.json");
        options.browser_pid_file = root.join("browser.pid");
        fs::write(&options.result_path, b"incomplete").unwrap();
        fs::write(&options.browser_pid_file, b"identity").unwrap();

        cleanup_collection_artifacts_after_exit(&options, true);

        assert!(!options.result_path.exists());
        assert!(options.browser_pid_file.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn login_shutdown_forces_the_owned_browser_after_graceful_timeout() {
        let waits = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from([false, true]),
        ));
        let waits_for_callback = std::sync::Arc::clone(&waits);
        let terminated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let terminated_for_callback = std::sync::Arc::clone(&terminated);

        assert_eq!(
            finish_login_browser_shutdown_with(
                async { Err("cdp_browser_closed".to_string()) },
                Deadline::after(Duration::from_secs(1)),
                move |_| waits_for_callback.lock().unwrap().pop_front().unwrap(),
                move || {
                    terminated_for_callback.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                },
            )
            .await,
            Ok(())
        );
        assert!(terminated.load(std::sync::atomic::Ordering::SeqCst));
        assert!(waits.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn login_shutdown_fails_only_when_the_owned_browser_cannot_be_stopped() {
        assert_eq!(
            finish_login_browser_shutdown_with(
                async { Ok(()) },
                Deadline::after(Duration::from_secs(1)),
                |_| false,
                || Ok(()),
            )
            .await,
            Err("browser_shutdown_timeout".to_string())
        );
    }

    #[test]
    fn recover_login_output_nests_the_observed_home_identity() {
        let attempt_id = "0123456789abcdef0123456789abcdef";
        let options = super::super::cli::RecoverLoginOptions {
            account_id: 11,
            sync_job_id: 7,
            phase: crate::sync_orchestration::SyncPhase::RecoveringLogin,
            phase_attempt_id: attempt_id.to_string(),
            result_path: PathBuf::from("result.json"),
            progress_path: PathBuf::from("progress.json"),
            diagnostic_path: PathBuf::from("diagnostic.json"),
            cancel_file: PathBuf::from("cancel"),
            account_profile_root: PathBuf::from("profile"),
            browser_pid_file: PathBuf::from("browser.pid"),
            collector_identity_path: PathBuf::from("collector.pid"),
            browser: super::super::cli::BrowserPreference::Auto,
            home_url: "https://www.douyin.com/".to_string(),
            deadline_at_ms: 1_784_457_630_000,
            headless: true,
        };
        let identity = ObservedHomeIdentityV1 {
            page_url: "https://www.douyin.com/".to_string(),
            uid: Some("112233445566".to_string()),
            sec_uid: Some("MS4wLjABAAAAfixture-sec-uid".to_string()),
            nickname: Some("fixture".to_string()),
            avatar_url: None,
            sources: vec![SessionStateSource::SsrRenderData],
        };

        let encoded = encode_recover_login_payload(&options, &identity).unwrap();
        let envelope: SyncPhaseEnvelopeV1<ObservedHomeIdentityV1> =
            serde_json::from_slice(&encoded).unwrap();

        envelope
            .validate_context(
                11,
                7,
                crate::sync_orchestration::SyncPhase::RecoveringLogin,
                attempt_id,
            )
            .unwrap();
        assert_eq!(envelope.payload, identity);
    }

    #[test]
    fn recover_login_requests_at_most_one_one_click_login() {
        let unauthenticated = SessionProbeV1::Unauthenticated {
            page_url: "https://www.douyin.com/".to_string(),
            source: SessionStateSource::InitialStateUser,
        };

        assert_eq!(
            recovery_probe_step(&unauthenticated, false),
            Ok(RecoveryProbeStep::TryOneClickLogin)
        );
        assert_eq!(
            recovery_probe_step(&unauthenticated, true),
            Ok(RecoveryProbeStep::Wait)
        );
    }

    #[test]
    fn recover_login_completes_authenticated_identity_and_rejects_conflicts() {
        let identity = ObservedHomeIdentityV1 {
            page_url: "https://www.douyin.com/".to_string(),
            uid: Some("112233445566".to_string()),
            sec_uid: None,
            nickname: None,
            avatar_url: None,
            sources: vec![SessionStateSource::SsrRenderData],
        };

        assert_eq!(
            recovery_probe_step(&SessionProbeV1::Authenticated(identity.clone()), false),
            Ok(RecoveryProbeStep::Complete(identity))
        );
        assert_eq!(
            recovery_probe_step(
                &SessionProbeV1::Conflicting {
                    page_url: "https://www.douyin.com/".to_string(),
                },
                false,
            ),
            Err("account_identity_ambiguous".to_string())
        );
    }

    #[tokio::test]
    async fn recover_login_honors_and_preserves_the_task_cancel_file() {
        let root = std::env::temp_dir().join(format!(
            "emily-recover-login-cancel-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let cancel_file = root.join("task.cancel");
        let diagnostic_path = root.join("diagnostic.json");
        fs::write(&cancel_file, b"cancel").unwrap();
        let options = super::super::cli::RecoverLoginOptions {
            account_id: 11,
            sync_job_id: 7,
            phase: crate::sync_orchestration::SyncPhase::RecoveringLogin,
            phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
            result_path: root.join("result.json"),
            progress_path: root.join("progress.json"),
            diagnostic_path: diagnostic_path.clone(),
            cancel_file: cancel_file.clone(),
            account_profile_root: root.join("profile"),
            browser_pid_file: root.join("browser.pid"),
            collector_identity_path: root.join("collector.pid"),
            browser: super::super::cli::BrowserPreference::Auto,
            home_url: "https://www.douyin.com/".to_string(),
            deadline_at_ms: Utc::now().timestamp_millis() + 30_000,
            headless: true,
        };

        assert_eq!(
            run_command(CollectorCommand::RecoverLogin(options)).await,
            Err("collector_cancelled".to_string())
        );
        assert!(cancel_file.exists());
        let diagnostic: SyncPhaseEnvelopeV1<CollectorDiagnostic> =
            serde_json::from_slice(&fs::read(&diagnostic_path).unwrap()).unwrap();
        diagnostic
            .validate_context(
                11,
                7,
                crate::sync_orchestration::SyncPhase::RecoveringLogin,
                "0123456789abcdef0123456789abcdef",
            )
            .unwrap();
        assert_eq!(diagnostic.payload.status, "cancelled");
        assert_eq!(diagnostic.payload.error_code, "collector_cancelled");

        fs::remove_dir_all(root).unwrap();
    }
}
