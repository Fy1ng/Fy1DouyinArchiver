use crate::{
    backup::{create_backup_archive, BackupArchiveCreateRequestV1, BackupArchiveSummaryV1},
    desktop_lifecycle::BackgroundTaskRegistry,
    device_settings::{validate_backup_directory_path, DeviceSettingsStore},
    notifications::NotificationKindV1,
    runtime_paths::RuntimePaths,
    settings_store::{
        load_portable_settings, BackupSettingsV1, NotificationSettingsV1, PortableSettingsV2,
    },
    sync::OperationAdmission,
};
use chrono::{DateTime, Days, Duration as ChronoDuration, FixedOffset, TimeZone, Utc};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const AUTOMATIC_BACKUP_PREFIX: &str = "emily-auto-";
const BACKUP_EXTENSION: &str = ".emily-backup";
const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;
const AUTOMATIC_BACKUP_FAILURE_RETRY_HOURS: i64 = 1;
const AUTOMATIC_BACKUP_BUSY_RETRY_SECONDS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomaticBackupTickStatusV1 {
    Disabled,
    Waiting,
    Deferred,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomaticBackupTickResultV1 {
    pub status: AutomaticBackupTickStatusV1,
    pub next_wake_at: Option<DateTime<Utc>>,
    pub destination: Option<PathBuf>,
    pub removed_backups: usize,
    pub error_code: Option<String>,
    pub notification_error: Option<String>,
}

pub fn create_application_backup(
    database_path: &Path,
    settings: &PortableSettingsV2,
    destination: &Path,
    created_at: &str,
    application_version: &str,
) -> Result<BackupArchiveSummaryV1, String> {
    let settings_bytes = serde_json::to_vec_pretty(settings)
        .map_err(|_| "backup_settings_serialize_failed".to_string())?;
    create_backup_archive(BackupArchiveCreateRequestV1 {
        database_path,
        settings_bytes: &settings_bytes,
        settings_schema_version: settings.schema_version,
        destination,
        created_at,
        application_version,
    })
}

pub fn run_backup_with_runtime(
    runtime_paths: &RuntimePaths,
    admission: &OperationAdmission,
    background_tasks: &BackgroundTaskRegistry,
    destination: &Path,
    created_at: &str,
    application_version: &str,
    wait_timeout: Duration,
) -> Result<BackupArchiveSummaryV1, String> {
    run_backup_with_runtime_observed(
        runtime_paths,
        admission,
        background_tasks,
        destination,
        created_at,
        application_version,
        wait_timeout,
        || Ok(()),
    )
}

fn run_backup_with_runtime_observed(
    runtime_paths: &RuntimePaths,
    admission: &OperationAdmission,
    background_tasks: &BackgroundTaskRegistry,
    destination: &Path,
    created_at: &str,
    application_version: &str,
    wait_timeout: Duration,
    on_ready: impl FnOnce() -> Result<(), String>,
) -> Result<BackupArchiveSummaryV1, String> {
    let _background_task = background_tasks.register()?;
    let maintenance = admission
        .begin_maintenance()?
        .wait_until_ready(wait_timeout)?;
    let paths = runtime_paths.acquire()?;
    let settings = load_portable_settings(&paths.data_root)?;
    on_ready()?;
    let result = create_application_backup(
        &paths.database,
        &settings,
        destination,
        created_at,
        application_version,
    );
    drop(paths);
    maintenance.finish();
    result
}

#[allow(clippy::too_many_arguments)]
pub fn run_automatic_backup_tick(
    runtime_paths: &RuntimePaths,
    admission: &OperationAdmission,
    background_tasks: &BackgroundTaskRegistry,
    device_settings: &DeviceSettingsStore,
    backup_settings: &BackupSettingsV1,
    notification_settings: &NotificationSettingsV1,
    now: DateTime<Utc>,
    application_version: &str,
    wait_timeout: Duration,
    mut notify: impl FnMut(NotificationKindV1) -> Result<(), String>,
) -> Result<AutomaticBackupTickResultV1, String> {
    let device = device_settings.load()?;
    let next_wake_at =
        next_automatic_backup_attempt_at(&device.automatic_backup, backup_settings, now)?;
    let Some(next_wake_at) = next_wake_at else {
        return Ok(automatic_backup_tick_result(
            AutomaticBackupTickStatusV1::Disabled,
            None,
        ));
    };
    if next_wake_at > now {
        return Ok(automatic_backup_tick_result(
            AutomaticBackupTickStatusV1::Waiting,
            Some(next_wake_at),
        ));
    }

    let paths = runtime_paths.acquire()?;
    let directory = device
        .backup_directory
        .clone()
        .unwrap_or_else(|| paths.backups.clone());
    drop(paths);
    let attempted_at = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let destination = automatic_backup_destination(&directory, now);
    let result = run_backup_with_runtime_observed(
        runtime_paths,
        admission,
        background_tasks,
        &destination,
        &attempted_at,
        application_version,
        wait_timeout,
        || {
            probe_backup_directory(&directory)?;
            device_settings
                .record_automatic_backup_attempt(&attempted_at)
                .map(|_| ())
        },
    );
    let summary = match result {
        Ok(summary) => summary,
        Err(error) if transient_automatic_backup_error(&error) => {
            return Ok(AutomaticBackupTickResultV1 {
                status: AutomaticBackupTickStatusV1::Deferred,
                next_wake_at: Some(
                    now.checked_add_signed(ChronoDuration::seconds(
                        AUTOMATIC_BACKUP_BUSY_RETRY_SECONDS,
                    ))
                    .ok_or_else(|| "invalid_backup_state".to_string())?,
                ),
                destination: None,
                removed_backups: 0,
                error_code: None,
                notification_error: None,
            });
        }
        Err(error) => {
            return automatic_backup_failure(
                device_settings,
                backup_settings,
                now,
                &attempted_at,
                stable_automatic_backup_error_code(&error),
                Some(destination),
                &mut notify,
            );
        }
    };
    let _ = summary;
    let removed_backups = match prune_automatic_backups(&directory, backup_settings.retention_count)
    {
        Ok(removed) => removed,
        Err(error) => {
            return automatic_backup_failure(
                device_settings,
                backup_settings,
                now,
                &attempted_at,
                stable_automatic_backup_error_code(&error),
                Some(destination),
                &mut notify,
            );
        }
    };
    let state = device_settings.record_automatic_backup_success(&attempted_at)?;
    let notification_error = if notification_settings.backup_completed {
        notify(NotificationKindV1::BackupCompleted).err()
    } else {
        None
    };
    Ok(AutomaticBackupTickResultV1 {
        status: AutomaticBackupTickStatusV1::Succeeded,
        next_wake_at: next_automatic_backup_attempt_at(
            &state.automatic_backup,
            backup_settings,
            now,
        )?,
        destination: Some(destination),
        removed_backups,
        error_code: None,
        notification_error,
    })
}

fn automatic_backup_failure(
    device_settings: &DeviceSettingsStore,
    backup_settings: &BackupSettingsV1,
    now: DateTime<Utc>,
    attempted_at: &str,
    error_code: String,
    destination: Option<PathBuf>,
    notify: &mut impl FnMut(NotificationKindV1) -> Result<(), String>,
) -> Result<AutomaticBackupTickResultV1, String> {
    let state = device_settings.record_automatic_backup_failure(attempted_at, &error_code)?;
    let notification_error = notify(NotificationKindV1::BackupFailed).err();
    Ok(AutomaticBackupTickResultV1 {
        status: AutomaticBackupTickStatusV1::Failed,
        next_wake_at: next_automatic_backup_attempt_at(
            &state.automatic_backup,
            backup_settings,
            now,
        )?,
        destination,
        removed_backups: 0,
        error_code: Some(error_code),
        notification_error,
    })
}

fn automatic_backup_tick_result(
    status: AutomaticBackupTickStatusV1,
    next_wake_at: Option<DateTime<Utc>>,
) -> AutomaticBackupTickResultV1 {
    AutomaticBackupTickResultV1 {
        status,
        next_wake_at,
        destination: None,
        removed_backups: 0,
        error_code: None,
        notification_error: None,
    }
}

fn transient_automatic_backup_error(error: &str) -> bool {
    matches!(
        error,
        "operation_timeout"
            | "settings_update_in_progress"
            | "browser_operation_in_progress"
            | "maintenance_in_progress"
            | "data_root_migration_in_progress"
            | "exit_in_progress"
    )
}

fn stable_automatic_backup_error_code(error: &str) -> String {
    let candidate = error.split(':').next().unwrap_or_default();
    if !candidate.is_empty()
        && candidate.len() <= 128
        && candidate.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
    {
        candidate.to_string()
    } else {
        "automatic_backup_failed".to_string()
    }
}

pub fn probe_backup_directory(path: &Path) -> Result<(), String> {
    validate_backup_directory_path(path)?;
    let created_directory = !path.exists();
    fs::create_dir_all(path).map_err(|_| "backup_directory_unavailable".to_string())?;
    if !path.is_dir() {
        return Err("backup_directory_unavailable".to_string());
    }

    let nonce = unique_nonce()?;
    let probe = path.join(format!(
        ".emily-backup-probe-{}-{nonce}.tmp",
        std::process::id()
    ));
    let renamed = path.join(format!(
        ".emily-backup-probe-{}-{nonce}.renamed",
        std::process::id()
    ));
    let payload = b"emily-backup-directory-probe";
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&probe)
            .map_err(|_| "backup_directory_unavailable".to_string())?;
        file.write_all(payload)
            .and_then(|_| file.sync_all())
            .map_err(|_| "backup_directory_unavailable".to_string())?;
        drop(file);
        if fs::read(&probe).map_err(|_| "backup_directory_unavailable".to_string())? != payload {
            return Err("backup_directory_unavailable".to_string());
        }
        fs::rename(&probe, &renamed).map_err(|_| "backup_directory_unavailable".to_string())?;
        fs::remove_file(&renamed).map_err(|_| "backup_directory_unavailable".to_string())
    })();
    let _ = fs::remove_file(&probe);
    let _ = fs::remove_file(&renamed);
    if result.is_err() && created_directory {
        let _ = fs::remove_dir(path);
    }
    result
}

pub fn automatic_backup_destination(directory: &Path, now: DateTime<Utc>) -> PathBuf {
    directory.join(backup_file_name(AUTOMATIC_BACKUP_PREFIX, now))
}

pub fn manual_backup_file_name(now: DateTime<Utc>) -> String {
    backup_file_name("emily-backup-", now)
}

fn backup_file_name(prefix: &str, now: DateTime<Utc>) -> String {
    let shanghai = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS).expect("valid offset");
    let local = now.with_timezone(&shanghai);
    format!(
        "{prefix}{}-{:03}{BACKUP_EXTENSION}",
        local.format("%Y%m%d-%H%M%S"),
        local.timestamp_subsec_millis()
    )
}

pub fn prune_automatic_backups(directory: &Path, retention_count: u32) -> Result<usize, String> {
    if retention_count == 0 {
        return Err("invalid_backup_retention_count".to_string());
    }
    let entries =
        fs::read_dir(directory).map_err(|_| "backup_directory_unavailable".to_string())?;
    let mut backups = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| "backup_directory_unavailable".to_string())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "backup_directory_unavailable".to_string())?;
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if valid_automatic_backup_name(&name) {
            backups.push((name, entry.path()));
        }
    }
    backups.sort_by(|left, right| right.0.cmp(&left.0));
    let mut removed = 0;
    for (_, path) in backups.into_iter().skip(retention_count as usize) {
        fs::remove_file(path).map_err(|_| "backup_retention_cleanup_failed".to_string())?;
        removed += 1;
    }
    Ok(removed)
}

pub fn next_automatic_backup_at(
    last_success_at: Option<&str>,
    settings: &BackupSettingsV1,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, String> {
    if !settings.enabled {
        return Ok(None);
    }
    if !(1..=365).contains(&settings.interval_days) {
        return Err("invalid_backup_interval".to_string());
    }
    let (hour, minute) = parse_execution_time(&settings.execution_time)?;
    let shanghai = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS).expect("valid offset");
    let date = match last_success_at {
        Some(value) => DateTime::parse_from_rfc3339(value)
            .map_err(|_| "invalid_backup_state".to_string())?
            .with_timezone(&shanghai)
            .date_naive()
            .checked_add_days(Days::new(settings.interval_days.into()))
            .ok_or_else(|| "invalid_backup_state".to_string())?,
        None => now.with_timezone(&shanghai).date_naive(),
    };
    let local = shanghai
        .from_local_datetime(
            &date
                .and_hms_opt(hour.into(), minute.into(), 0)
                .ok_or_else(|| "invalid_backup_execution_time".to_string())?,
        )
        .single()
        .ok_or_else(|| "invalid_backup_execution_time".to_string())?;
    Ok(Some(local.with_timezone(&Utc)))
}

pub fn next_automatic_backup_attempt_at(
    state: &crate::device_settings::AutomaticBackupStateV1,
    settings: &BackupSettingsV1,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, String> {
    let scheduled = next_automatic_backup_at(state.last_success_at.as_deref(), settings, now)?;
    let Some(scheduled) = scheduled else {
        return Ok(None);
    };
    let Some(last_attempt) = state.last_attempt_at.as_deref() else {
        return Ok(Some(scheduled));
    };
    let last_attempt = DateTime::parse_from_rfc3339(last_attempt)
        .map_err(|_| "invalid_backup_state".to_string())?
        .with_timezone(&Utc);
    let last_success = state
        .last_success_at
        .as_deref()
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| "invalid_backup_state".to_string())
        })
        .transpose()?;
    if last_success.is_some_and(|success| success > last_attempt) {
        return Err("invalid_backup_state".to_string());
    }
    if last_success == Some(last_attempt) {
        return Ok(Some(scheduled));
    }
    let retry = last_attempt
        .checked_add_signed(ChronoDuration::hours(AUTOMATIC_BACKUP_FAILURE_RETRY_HOURS))
        .ok_or_else(|| "invalid_backup_state".to_string())?;
    Ok(Some(scheduled.max(retry)))
}

fn parse_execution_time(value: &str) -> Result<(u8, u8), String> {
    let bytes = value.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 2 || byte.is_ascii_digit())
    {
        return Err("invalid_backup_execution_time".to_string());
    }
    let hour = (bytes[0] - b'0') * 10 + (bytes[1] - b'0');
    let minute = (bytes[3] - b'0') * 10 + (bytes[4] - b'0');
    if hour >= 24 || minute >= 60 {
        return Err("invalid_backup_execution_time".to_string());
    }
    Ok((hour, minute))
}

fn valid_automatic_backup_name(value: &str) -> bool {
    let Some(timestamp) = value
        .strip_prefix(AUTOMATIC_BACKUP_PREFIX)
        .and_then(|value| value.strip_suffix(BACKUP_EXTENSION))
    else {
        return false;
    };
    timestamp.len() == 19
        && timestamp.as_bytes()[8] == b'-'
        && timestamp.as_bytes()[15] == b'-'
        && timestamp
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 8 | 15) || byte.is_ascii_digit())
}

fn unique_nonce() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|_| "backup_clock_invalid".to_string())
}
