use crate::{
    settings_store::{ArchiveListRangeV1, HistoryRangeV1},
    sync::{shanghai_cutoff_ms, validate_source_instance, ArchiveRunModeV1, SyncSourceV1},
    sync_orchestration::SyncPhase,
    AppPaths,
};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

const DEFAULT_HISTORY_URL: &str = "https://www.douyin.com/user/self?showTab=record";
const DEFAULT_HOME_URL: &str = "https://www.douyin.com/";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserPreference {
    Auto,
    Edge,
    Chrome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectRange {
    History(HistoryRangeV1),
    Archive(ArchiveListRangeV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectOptions {
    pub account_id: i64,
    pub sync_job_id: i64,
    pub source: SyncSourceV1,
    pub source_instance_key: String,
    pub task_payload_sha256: String,
    pub frozen_at_ms: i64,
    pub cutoff_ms: Option<i64>,
    pub phase: Option<SyncPhase>,
    pub phase_attempt_id: Option<String>,
    pub result_path: PathBuf,
    pub progress_path: PathBuf,
    pub diagnostic_path: PathBuf,
    pub cancel_file: PathBuf,
    pub account_profile_root: PathBuf,
    pub browser_pid_file: PathBuf,
    pub collector_identity_path: Option<PathBuf>,
    pub known_ids_path: Option<PathBuf>,
    pub browser: BrowserPreference,
    pub profile_key: String,
    pub profile_generation: String,
    pub browser_executable_sha256: String,
    pub page_url: String,
    pub range: CollectRange,
    pub run_mode: Option<ArchiveRunModeV1>,
    pub discover_remote_folders: bool,
    pub scroll_speed: u32,
    pub headless: bool,
    pub block_media: bool,
    pub block_images: bool,
    pub hard_timeout_ms: u64,
    pub max_records: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginOptions {
    pub target_account_id: Option<i64>,
    pub login_attempt_id: String,
    pub account_profile_root: PathBuf,
    pub identity_result_path: PathBuf,
    pub progress_path: PathBuf,
    pub diagnostic_path: PathBuf,
    pub cancel_file: PathBuf,
    pub browser_pid_file: PathBuf,
    pub browser: BrowserPreference,
    pub home_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverLoginOptions {
    pub account_id: i64,
    pub sync_job_id: i64,
    pub phase: SyncPhase,
    pub phase_attempt_id: String,
    pub result_path: PathBuf,
    pub progress_path: PathBuf,
    pub diagnostic_path: PathBuf,
    pub cancel_file: PathBuf,
    pub account_profile_root: PathBuf,
    pub browser_pid_file: PathBuf,
    pub collector_identity_path: PathBuf,
    pub browser: BrowserPreference,
    pub home_url: String,
    pub deadline_at_ms: i64,
    pub headless: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectorCommand {
    Collect(CollectOptions),
    Login(LoginOptions),
    RecoverLogin(RecoverLoginOptions),
}

pub fn parse_collector_args(argv: Vec<OsString>) -> Result<CollectorCommand, String> {
    let command = argv
        .first()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "invalid_command".to_string())?;
    if command != "collect" && command != "login" && command != "recover-login" {
        return Err("invalid_command".to_string());
    }
    let flag_options = [
        "--bottom",
        "--headless",
        "--block-images",
        "--block-media",
        "--no-block-media",
        "--discover-remote-folders",
    ];
    let value_options = [
        "--account-id",
        "--target-account-id",
        "--job-id",
        "--login-attempt-id",
        "--browser-data-dir",
        "--result",
        "--progress",
        "--identity-result",
        "--login-progress-file",
        "--diagnostic",
        "--cancel-file",
        "--account-profile-root",
        "--browser-pid-file",
        "--collector-identity-file",
        "--known-ids-file",
        "--browser",
        "--history-url",
        "--page-url",
        "--home-url",
        "--phase",
        "--phase-attempt-id",
        "--deadline-at-ms",
        "--source",
        "--source-instance-key",
        "--task-payload-sha256",
        "--frozen-at-ms",
        "--cutoff-ms",
        "--profile-key",
        "--profile-generation",
        "--browser-executable-sha256",
        "--days",
        "--archive-range",
        "--item-count",
        "--record-limit",
        "--run-mode",
        "--scroll-speed",
        "--hard-timeout-ms",
        "--max-records",
    ];
    let mut values = HashMap::<String, OsString>::new();
    let mut flags = HashSet::<String>::new();
    let mut index = 1;
    while index < argv.len() {
        let option = argv[index]
            .to_str()
            .ok_or_else(|| "invalid_unicode_option".to_string())?;
        if !option.starts_with("--") {
            return Err(format!("unexpected_argument:{option}"));
        }
        if flag_options.contains(&option) {
            flags.insert(option.to_string());
            index += 1;
            continue;
        }
        if !value_options.contains(&option) {
            return Err(format!("unexpected_option:{option}"));
        }
        let Some(value) = argv.get(index + 1) else {
            return Err(format!("missing_option_value:{option}"));
        };
        if value.to_str().is_some_and(|value| value.starts_with("--")) {
            return Err(format!("missing_option_value:{option}"));
        }
        values.insert(option.to_string(), value.clone());
        index += 2;
    }

    let paths = AppPaths::new(
        PathBuf::from(required(&values, "--browser-data-dir")?),
        PathBuf::new(),
    );
    let account_profile_root = PathBuf::from(required(&values, "--account-profile-root")?);
    validate_account_profile_root(&paths, &account_profile_root)?;
    let browser = parse_browser(values.get("--browser"))?;
    let collect_identity_options = [
        "--source",
        "--source-instance-key",
        "--task-payload-sha256",
        "--frozen-at-ms",
        "--cutoff-ms",
        "--profile-key",
        "--profile-generation",
        "--browser-executable-sha256",
    ];
    if command == "login" {
        if collect_identity_options
            .iter()
            .any(|option| values.contains_key(*option))
        {
            return Err("unexpected_collect_option".to_string());
        }
        if values.contains_key("--phase")
            || values.contains_key("--phase-attempt-id")
            || values.contains_key("--deadline-at-ms")
            || values.contains_key("--collector-identity-file")
            || values.contains_key("--known-ids-file")
        {
            return Err("unexpected_sync_phase_option".to_string());
        }
        if values.contains_key("--account-id") {
            return Err("unexpected_option:--account-id".to_string());
        }
        let target_account_id = values
            .get("--target-account-id")
            .map(|value| identifier(value, "invalid_account_id"))
            .transpose()?;
        let login_attempt_id =
            required_simple_uuid(&values, "--login-attempt-id", "invalid_login_attempt_id")?;
        validate_path(
            &values,
            "--identity-result",
            &paths.login_task_identity_path(&login_attempt_id)?,
            "identity-result",
        )?;
        validate_path(
            &values,
            "--login-progress-file",
            &paths.login_task_progress_path(&login_attempt_id)?,
            "login-progress-file",
        )?;
        validate_path(
            &values,
            "--diagnostic",
            &paths.login_task_diagnostic_path(&login_attempt_id)?,
            "diagnostic",
        )?;
        validate_path(
            &values,
            "--cancel-file",
            &paths.login_task_cancel_path(&login_attempt_id)?,
            "cancel-file",
        )?;
        validate_path(
            &values,
            "--browser-pid-file",
            &paths.login_task_browser_pid_path(&login_attempt_id)?,
            "browser-pid-file",
        )?;
        let home_url =
            text_value(values.get("--home-url"))?.unwrap_or_else(|| DEFAULT_HOME_URL.to_string());
        return Ok(CollectorCommand::Login(LoginOptions {
            target_account_id,
            login_attempt_id,
            account_profile_root,
            identity_result_path: PathBuf::from(required(&values, "--identity-result")?),
            progress_path: PathBuf::from(required(&values, "--login-progress-file")?),
            diagnostic_path: PathBuf::from(required(&values, "--diagnostic")?),
            cancel_file: PathBuf::from(required(&values, "--cancel-file")?),
            browser_pid_file: PathBuf::from(required(&values, "--browser-pid-file")?),
            browser,
            home_url,
        }));
    }

    if command == "recover-login" {
        if collect_identity_options
            .iter()
            .any(|option| values.contains_key(*option))
        {
            return Err("unexpected_collect_option".to_string());
        }
        if values.contains_key("--target-account-id")
            || values.contains_key("--login-attempt-id")
            || values.contains_key("--identity-result")
            || values.contains_key("--login-progress-file")
            || values.contains_key("--history-url")
            || values.contains_key("--page-url")
            || values.contains_key("--archive-range")
            || values.contains_key("--item-count")
            || values.contains_key("--record-limit")
            || values.contains_key("--run-mode")
            || values.contains_key("--known-ids-file")
            || flags.contains("--discover-remote-folders")
        {
            return Err("unexpected_login_option".to_string());
        }
        let account_id = identifier(&required(&values, "--account-id")?, "invalid_account_id")?;
        let sync_job_id = identifier(&required(&values, "--job-id")?, "invalid_job_id")?;
        let phase = parse_phase(&required(&values, "--phase")?)?;
        if phase != SyncPhase::RecoveringLogin {
            return Err("recover_login_phase_mismatch".to_string());
        }
        let phase_attempt_id =
            required_simple_uuid(&values, "--phase-attempt-id", "invalid_phase_attempt_id")?;
        let result_path =
            paths.phase_result_path(account_id, sync_job_id, phase, &phase_attempt_id)?;
        let progress_path =
            paths.phase_progress_path(account_id, sync_job_id, phase, &phase_attempt_id)?;
        let diagnostic_path =
            paths.phase_diagnostic_path(account_id, sync_job_id, phase, &phase_attempt_id)?;
        let browser_pid_path =
            paths.phase_browser_pid_path(account_id, sync_job_id, phase, &phase_attempt_id)?;
        let collector_identity_path = paths.phase_collector_identity_path(
            account_id,
            sync_job_id,
            phase,
            &phase_attempt_id,
        )?;
        validate_path(&values, "--result", &result_path, "result")?;
        validate_path(&values, "--progress", &progress_path, "progress")?;
        validate_path(&values, "--diagnostic", &diagnostic_path, "diagnostic")?;
        validate_path(
            &values,
            "--cancel-file",
            &paths.cancel_path(account_id, sync_job_id),
            "cancel-file",
        )?;
        validate_path(
            &values,
            "--browser-pid-file",
            &browser_pid_path,
            "browser-pid-file",
        )?;
        validate_path(
            &values,
            "--collector-identity-file",
            &collector_identity_path,
            "collector-identity",
        )?;
        let deadline_at_ms = required_i64(&values, "--deadline-at-ms", "invalid_deadline_at_ms")?;
        if !flags.contains("--headless") {
            return Err("recover_login_requires_headless".to_string());
        }
        let home_url =
            text_value(values.get("--home-url"))?.unwrap_or_else(|| DEFAULT_HOME_URL.to_string());
        return Ok(CollectorCommand::RecoverLogin(RecoverLoginOptions {
            account_id,
            sync_job_id,
            phase,
            phase_attempt_id,
            result_path: PathBuf::from(required(&values, "--result")?),
            progress_path: PathBuf::from(required(&values, "--progress")?),
            diagnostic_path: PathBuf::from(required(&values, "--diagnostic")?),
            cancel_file: PathBuf::from(required(&values, "--cancel-file")?),
            account_profile_root,
            browser_pid_file: PathBuf::from(required(&values, "--browser-pid-file")?),
            collector_identity_path: PathBuf::from(required(&values, "--collector-identity-file")?),
            browser,
            home_url,
            deadline_at_ms,
            headless: true,
        }));
    }

    if values.contains_key("--target-account-id") || values.contains_key("--home-url") {
        return Err("unexpected_login_option".to_string());
    }
    let account_id = identifier(&required(&values, "--account-id")?, "invalid_account_id")?;
    let sync_job_id = identifier(&required(&values, "--job-id")?, "invalid_job_id")?;
    let phase_identity = match (values.get("--phase"), values.get("--phase-attempt-id")) {
        (Some(phase), Some(_)) => Some((
            parse_phase(phase)?,
            required_simple_uuid(&values, "--phase-attempt-id", "invalid_phase_attempt_id")?,
        )),
        (None, None) => None,
        _ => return Err("incomplete_sync_phase_identity".to_string()),
    };
    let result_path = match &phase_identity {
        Some((phase, attempt)) => {
            paths.phase_result_path(account_id, sync_job_id, *phase, attempt)?
        }
        None => paths.result_path(account_id, sync_job_id),
    };
    let progress_path = match &phase_identity {
        Some((phase, attempt)) => {
            paths.phase_progress_path(account_id, sync_job_id, *phase, attempt)?
        }
        None => paths.progress_path(account_id, sync_job_id),
    };
    let diagnostic_path = match &phase_identity {
        Some((phase, attempt)) => {
            paths.phase_diagnostic_path(account_id, sync_job_id, *phase, attempt)?
        }
        None => paths.diagnostic_path(account_id, sync_job_id),
    };
    let browser_pid_path = match &phase_identity {
        Some((phase, attempt)) => {
            paths.phase_browser_pid_path(account_id, sync_job_id, *phase, attempt)?
        }
        None => paths.browser_pid_path(account_id, sync_job_id),
    };
    let collector_identity_path = match &phase_identity {
        Some((phase, attempt)) => {
            Some(paths.phase_collector_identity_path(account_id, sync_job_id, *phase, attempt)?)
        }
        None => None,
    };
    validate_path(&values, "--result", &result_path, "result")?;
    validate_path(&values, "--progress", &progress_path, "progress")?;
    validate_path(&values, "--diagnostic", &diagnostic_path, "diagnostic")?;
    validate_path(
        &values,
        "--cancel-file",
        &paths.cancel_path(account_id, sync_job_id),
        "cancel-file",
    )?;
    validate_path(
        &values,
        "--browser-pid-file",
        &browser_pid_path,
        "browser-pid-file",
    )?;
    if let Some(expected) = &collector_identity_path {
        validate_path(
            &values,
            "--collector-identity-file",
            expected,
            "collector-identity",
        )?;
    } else if values.contains_key("--collector-identity-file") {
        return Err("unexpected_sync_phase_option".to_string());
    }
    let source = match required_text(&values, "--source")?.as_str() {
        "history" => SyncSourceV1::History,
        "likes" => SyncSourceV1::Likes,
        "favorites" => SyncSourceV1::Favorites,
        "remote_folder" => SyncSourceV1::RemoteFolder,
        _ => return Err("sync_executor_source_mismatch".to_string()),
    };
    let source_instance_key = required_text(&values, "--source-instance-key")?;
    validate_source_instance(source, &source_instance_key)
        .map_err(|_| "sync_executor_source_mismatch".to_string())?;
    let task_payload_sha256 = required_sha256(
        &values,
        "--task-payload-sha256",
        "invalid_task_payload_sha256",
    )?;
    let frozen_at_ms = required_epoch_ms(&values, "--frozen-at-ms", "invalid_frozen_at_ms")?;
    let cutoff_ms = values
        .get("--cutoff-ms")
        .map(|value| epoch_ms(value, "invalid_cutoff_ms"))
        .transpose()?;
    let profile_key = required_simple_uuid(&values, "--profile-key", "invalid_profile_key")?;
    if account_profile_root
        .file_name()
        .and_then(|value| value.to_str())
        != Some(profile_key.as_str())
    {
        return Err("invalid_profile_key".to_string());
    }
    let profile_generation = required_simple_uuid(
        &values,
        "--profile-generation",
        "invalid_profile_generation",
    )?;
    let browser_executable_sha256 = required_sha256(
        &values,
        "--browser-executable-sha256",
        "invalid_browser_executable_sha256",
    )?;
    if browser == BrowserPreference::Auto {
        return Err("invalid_browser_execution_identity".to_string());
    }
    let (page_url, range, run_mode, discover_remote_folders, known_ids_path) = if source
        == SyncSourceV1::History
    {
        if values.contains_key("--page-url")
            || values.contains_key("--archive-range")
            || values.contains_key("--item-count")
            || values.contains_key("--record-limit")
            || values.contains_key("--run-mode")
            || values.contains_key("--known-ids-file")
            || flags.contains("--discover-remote-folders")
        {
            return Err("unexpected_archive_option".to_string());
        }
        let page_url = text_value(values.get("--history-url"))?
            .unwrap_or_else(|| DEFAULT_HISTORY_URL.to_string());
        let range = if flags.contains("--bottom") {
            if cutoff_ms.is_some() || values.contains_key("--days") {
                return Err("range_cutoff_mismatch".to_string());
            }
            HistoryRangeV1::Bottom
        } else {
            let days = integer_or(values.get("--days"), 7, 1, 365)? as u32;
            if cutoff_ms != Some(shanghai_cutoff_ms(frozen_at_ms, days)?) {
                return Err("range_cutoff_mismatch".to_string());
            }
            HistoryRangeV1::Days { days }
        };
        (page_url, CollectRange::History(range), None, false, None)
    } else {
        if values.contains_key("--history-url") || flags.contains("--bottom") {
            return Err("unexpected_history_option".to_string());
        }
        let page_url = required_text(&values, "--page-url")?;
        validate_archive_page_url(source, &page_url)?;
        let range = parse_archive_range(&values, frozen_at_ms, cutoff_ms)?;
        let run_mode = match required_text(&values, "--run-mode")?.as_str() {
            "incremental" => ArchiveRunModeV1::Incremental,
            "full_reconcile" => ArchiveRunModeV1::FullReconcile,
            _ => return Err("invalid_archive_run_mode".to_string()),
        };
        if run_mode == ArchiveRunModeV1::FullReconcile && range != ArchiveListRangeV1::All {
            return Err("full_reconcile_requires_all".to_string());
        }
        let discover_remote_folders = flags.contains("--discover-remote-folders");
        if discover_remote_folders && source != SyncSourceV1::Favorites {
            return Err("remote_folder_discovery_request_mismatch".to_string());
        }
        let known_ids_path = if range == ArchiveListRangeV1::All
            && run_mode == ArchiveRunModeV1::Incremental
        {
            let (phase, attempt_id) = phase_identity
                .as_ref()
                .ok_or_else(|| "archive_known_ids_requires_phase".to_string())?;
            let expected =
                paths.phase_archive_known_ids_path(account_id, sync_job_id, *phase, attempt_id)?;
            validate_path(&values, "--known-ids-file", &expected, "archive-known-ids")?;
            Some(expected)
        } else {
            if values.contains_key("--known-ids-file") {
                return Err("unexpected_archive_known_ids_file".to_string());
            }
            None
        };
        (
            page_url,
            CollectRange::Archive(range),
            Some(run_mode),
            discover_remote_folders,
            known_ids_path,
        )
    };
    Ok(CollectorCommand::Collect(CollectOptions {
        account_id,
        sync_job_id,
        source,
        source_instance_key,
        task_payload_sha256,
        frozen_at_ms,
        cutoff_ms,
        phase: phase_identity.as_ref().map(|value| value.0),
        phase_attempt_id: phase_identity.map(|value| value.1),
        result_path: PathBuf::from(required(&values, "--result")?),
        progress_path: PathBuf::from(required(&values, "--progress")?),
        diagnostic_path: PathBuf::from(required(&values, "--diagnostic")?),
        cancel_file: PathBuf::from(required(&values, "--cancel-file")?),
        account_profile_root,
        browser_pid_file: PathBuf::from(required(&values, "--browser-pid-file")?),
        collector_identity_path,
        known_ids_path,
        browser,
        profile_key,
        profile_generation,
        browser_executable_sha256,
        page_url,
        range,
        run_mode,
        discover_remote_folders,
        scroll_speed: integer_or(values.get("--scroll-speed"), 6_000, 1, u32::MAX as u64)? as u32,
        headless: flags.contains("--headless"),
        block_media: !flags.contains("--no-block-media"),
        block_images: flags.contains("--block-images"),
        hard_timeout_ms: integer_or(values.get("--hard-timeout-ms"), 600_000, 30_000, 600_000)?,
        max_records: integer_or(values.get("--max-records"), 50_000, 1, 50_000)? as usize,
    }))
}

fn parse_phase(value: &OsString) -> Result<SyncPhase, String> {
    match value.to_str() {
        Some("collecting") => Ok(SyncPhase::Collecting),
        Some("recovering_login") => Ok(SyncPhase::RecoveringLogin),
        Some("retrying_after_login") => Ok(SyncPhase::RetryingAfterLogin),
        _ => Err("invalid_sync_phase".to_string()),
    }
}

fn parse_archive_range(
    values: &HashMap<String, OsString>,
    frozen_at_ms: i64,
    cutoff_ms: Option<i64>,
) -> Result<ArchiveListRangeV1, String> {
    let range = match required_text(values, "--archive-range")?.as_str() {
        "all" => {
            reject_archive_range_options(values, &["--days", "--item-count", "--record-limit"])?;
            if cutoff_ms.is_some() {
                return Err("range_cutoff_mismatch".to_string());
            }
            ArchiveListRangeV1::All
        }
        "recent_items" => {
            reject_archive_range_options(values, &["--days", "--record-limit"])?;
            if cutoff_ms.is_some() {
                return Err("range_cutoff_mismatch".to_string());
            }
            ArchiveListRangeV1::RecentItems {
                count: required_u32_in_range(values, "--item-count", 1, 5_000)?,
            }
        }
        "record_limit" => {
            reject_archive_range_options(values, &["--days", "--item-count"])?;
            if cutoff_ms.is_some() {
                return Err("range_cutoff_mismatch".to_string());
            }
            ArchiveListRangeV1::RecordLimit {
                limit: required_u32_in_range(values, "--record-limit", 1, 50_000)?,
            }
        }
        "recent_days" => {
            reject_archive_range_options(values, &["--item-count", "--record-limit"])?;
            let days = required_u32_in_range(values, "--days", 1, 365)?;
            if cutoff_ms != Some(shanghai_cutoff_ms(frozen_at_ms, days)?) {
                return Err("range_cutoff_mismatch".to_string());
            }
            ArchiveListRangeV1::RecentDays { days }
        }
        _ => return Err("invalid_archive_range".to_string()),
    };
    Ok(range)
}

fn reject_archive_range_options(
    values: &HashMap<String, OsString>,
    forbidden: &[&str],
) -> Result<(), String> {
    if forbidden.iter().any(|option| values.contains_key(*option)) {
        Err("invalid_archive_range_options".to_string())
    } else {
        Ok(())
    }
}

fn required_u32_in_range(
    values: &HashMap<String, OsString>,
    key: &str,
    minimum: u32,
    maximum: u32,
) -> Result<u32, String> {
    let value = required_text(values, key)?;
    let parsed = value
        .parse::<u32>()
        .map_err(|_| "invalid_archive_range_value".to_string())?;
    if !(minimum..=maximum).contains(&parsed) {
        return Err("invalid_archive_range_value".to_string());
    }
    Ok(parsed)
}

fn validate_archive_page_url(source: SyncSourceV1, page_url: &str) -> Result<(), String> {
    let expected = match source {
        SyncSourceV1::Likes => "https://www.douyin.com/user/self?showTab=like",
        SyncSourceV1::Favorites | SyncSourceV1::RemoteFolder => {
            "https://www.douyin.com/user/self?showTab=favorite_collection"
        }
        SyncSourceV1::History => return Err("sync_executor_source_mismatch".to_string()),
    };
    if page_url == expected {
        Ok(())
    } else {
        Err("archive_page_url_mismatch".to_string())
    }
}

fn identifier(value: &OsString, error: &str) -> Result<i64, String> {
    let parsed = value
        .to_str()
        .ok_or_else(|| "invalid_unicode_value".to_string())?
        .parse::<u64>()
        .map_err(|_| error.to_string())?;
    if parsed == 0 || parsed > MAX_SAFE_INTEGER {
        return Err(error.to_string());
    }
    Ok(parsed as i64)
}

fn validate_account_profile_root(paths: &AppPaths, value: &Path) -> Result<(), String> {
    let profile_key = value
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "invalid_account_profile_root".to_string())?;
    let expected = paths
        .account_browser_root(profile_key)
        .map_err(|_| "invalid_account_profile_root".to_string())?;
    if value != expected {
        return Err("invalid_account_profile_root".into());
    }
    Ok(())
}

fn validate_path(
    values: &HashMap<String, OsString>,
    option: &str,
    expected: &Path,
    label: &str,
) -> Result<(), String> {
    if PathBuf::from(required(values, option)?) != expected {
        return Err(format!("collector_path_mismatch:{label}"));
    }
    Ok(())
}

fn required(values: &HashMap<String, OsString>, key: &str) -> Result<OsString, String> {
    values
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing_required_option:{key}"))
}

fn required_text(values: &HashMap<String, OsString>, key: &str) -> Result<String, String> {
    required(values, key)?
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| "invalid_unicode_value".to_string())
}

fn required_sha256(
    values: &HashMap<String, OsString>,
    key: &str,
    error: &str,
) -> Result<String, String> {
    let value = required_text(values, key)?;
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(value)
    } else {
        Err(error.to_string())
    }
}

fn required_simple_uuid(
    values: &HashMap<String, OsString>,
    key: &str,
    error: &str,
) -> Result<String, String> {
    let value = required(values, key)?;
    let text = value
        .to_str()
        .ok_or_else(|| "invalid_unicode_value".to_string())?;
    if text.len() == 32
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(text.to_string())
    } else {
        Err(error.to_string())
    }
}

fn required_i64(values: &HashMap<String, OsString>, key: &str, error: &str) -> Result<i64, String> {
    let value = required(values, key)?;
    let text = value
        .to_str()
        .ok_or_else(|| "invalid_unicode_value".to_string())?;
    let parsed = text.parse::<u64>().map_err(|_| error.to_string())?;
    if parsed == 0 || parsed > MAX_SAFE_INTEGER {
        return Err(error.to_string());
    }
    Ok(parsed as i64)
}

fn required_epoch_ms(
    values: &HashMap<String, OsString>,
    key: &str,
    error: &str,
) -> Result<i64, String> {
    epoch_ms(&required(values, key)?, error)
}

fn epoch_ms(value: &OsString, error: &str) -> Result<i64, String> {
    let text = value
        .to_str()
        .ok_or_else(|| "invalid_unicode_value".to_string())?;
    let parsed = text.parse::<u64>().map_err(|_| error.to_string())?;
    if parsed > MAX_SAFE_INTEGER {
        return Err(error.to_string());
    }
    Ok(parsed as i64)
}

fn text_value(value: Option<&OsString>) -> Result<Option<String>, String> {
    value
        .map(|value| {
            value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "invalid_unicode_value".to_string())
        })
        .transpose()
}

fn parse_browser(value: Option<&OsString>) -> Result<BrowserPreference, String> {
    match text_value(value)?.as_deref().unwrap_or("auto") {
        "auto" => Ok(BrowserPreference::Auto),
        "edge" => Ok(BrowserPreference::Edge),
        "chrome" => Ok(BrowserPreference::Chrome),
        value => Err(format!("invalid_browser_preference:{value}")),
    }
}

fn integer_or(
    value: Option<&OsString>,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, String> {
    match value {
        Some(value) => integer(value, minimum, maximum),
        None => Ok(default),
    }
}

fn integer(value: &OsString, minimum: u64, maximum: u64) -> Result<u64, String> {
    let text = value
        .to_str()
        .ok_or_else(|| "invalid_unicode_value".to_string())?;
    let parsed = text
        .parse::<f64>()
        .map_err(|_| format!("invalid_number:{text}"))?;
    if !parsed.is_finite() {
        return Err(format!("invalid_number:{text}"));
    }
    Ok((parsed.floor().max(minimum as f64).min(maximum as f64)) as u64)
}
