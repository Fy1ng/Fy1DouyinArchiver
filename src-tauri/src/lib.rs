use chrono::{DateTime, FixedOffset, SecondsFormat, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use url::Url;

pub mod accounts;
pub mod annual_review;
pub mod autostart;
pub mod backup;
pub mod backup_service;
pub mod bootstrap;
pub mod collector;
pub mod collector_process;
pub mod content;
pub mod content_url;
pub mod cover_cache;
pub mod data_root;
pub mod desktop_ipc;
pub mod desktop_lifecycle;
pub mod device_settings;
pub mod download;
pub mod folders;
pub mod login_attempt;
pub mod login_watchdog;
pub mod merge_service;
pub mod notifications;
pub mod overview;
pub mod process_lock;
pub mod restore_service;
pub mod runtime_paths;
pub mod schema;
pub mod settings_store;
pub mod startup_recovery;
pub mod sync;
pub mod sync_driver;
pub mod sync_orchestration;
pub mod task_center;
pub mod uninstall;
pub mod update_runtime;

pub use annual_review::*;
pub use task_center::*;
pub use backup::*;
pub use backup_service::*;
pub use content::{
    list_content, list_library_content, search_library_content, ContentRecordView, ContentSortV1,
    LibraryContentQueryV1, LibraryContentRecordViewV1, LibraryContentSortV1, ListContentResult,
    ListLibraryContentResultV1, SearchContentQueryV1, SearchRemoteStateV1,
};
pub use content_url::{open_content_url, validate_content_url};
pub use folders::*;
pub use merge_service::*;
pub use notifications::*;
pub use overview::*;
pub use restore_service::*;

pub use schema::{
    assert_schema_latest, assert_schema_v5, consume_schema_v5_recovery_work,
    migrate_schema_to_latest_at_startup, migrate_schema_to_v5_at_startup,
    rebuild_schema_v5_cleanup_work, LegacyRunningJobIdentity, SchemaV5RecoveryWork,
};

const SCHEMA_VERSION_V2: i64 = 2;
const SCHEMA_VERSION_V3: i64 = 3;
const SCHEMA_VERSION_V4: i64 = 4;
const MIGRATION_V2: &str = include_str!("../migrations/002_react_history.sql");
const MIGRATION_V3: &str = include_str!("../migrations/003_multi_account.sql");
const MIGRATION_V4: &str = include_str!("../migrations/004_identity_conflict.sql");
const EARLIEST_HISTORY_MS: i64 = 946_684_800_000;
const FUTURE_TOLERANCE_MS: i64 = 5 * 60 * 1_000;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectorRecordV2 {
    pub aweme_id: String,
    pub aweme_type: Option<i64>,
    pub canonical_url: String,
    pub content_type: String,
    pub history_time_ms: i64,
    pub watched_at: String,
    pub watched_date: String,
    pub source_position: i64,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub cover_url: Option<String>,
    pub like_count: Option<i64>,
    pub watch_progress_rate: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
pub enum CollectorRange {
    Days {
        days: u32,
        #[serde(rename = "cutoffMs")]
        cutoff_ms: i64,
    },
    Bottom,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectorSuccessV3 {
    pub schema_version: i64,
    pub account_id: i64,
    pub sync_job_id: i64,
    pub sync_started_at_ms: i64,
    pub calendar_time_zone: String,
    pub range: CollectorRange,
    pub detected_history_time_unit: String,
    pub identity_hash: String,
    pub field_hash: String,
    pub records: Vec<CollectorRecordV2>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportSummary {
    pub inserted_videos: i64,
    pub inserted_watch_records: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecordView {
    pub id: i64,
    pub aweme_id: String,
    pub url: String,
    pub content_type: String,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub cover_url: Option<String>,
    pub cached_cover_url: Option<String>,
    pub like_count: Option<i64>,
    pub history_time_ms: i64,
    pub watched_at: String,
    pub watched_date: String,
    pub source_position: i64,
    pub watch_progress_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListHistoryResult {
    pub records: Vec<HistoryRecordView>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteHistoryResult {
    pub deleted_watch_records: i64,
    pub deleted_videos: i64,
    pub remaining_watch_records: i64,
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub bootstrap_dir: PathBuf,
    pub data_root: PathBuf,
    pub app_data: PathBuf,
    pub resource_dir: PathBuf,
    pub database: PathBuf,
    pub browser_profiles: PathBuf,
    pub covers: PathBuf,
    pub backups: PathBuf,
    pub temp: PathBuf,
    pub diagnostics: PathBuf,
    pub logs: PathBuf,
    pub downloads: PathBuf,
}

impl AppPaths {
    pub fn new(app_data: PathBuf, resource_dir: PathBuf) -> Self {
        Self::from_roots(app_data.clone(), app_data, resource_dir)
    }

    pub fn from_roots(bootstrap_dir: PathBuf, data_root: PathBuf, resource_dir: PathBuf) -> Self {
        let bootstrap_dir = node_compatible_path(bootstrap_dir);
        let data_root = node_compatible_path(data_root);
        let resource_dir = node_compatible_path(resource_dir);
        Self {
            database: data_root.join("history.sqlite"),
            browser_profiles: data_root.join("profiles"),
            covers: data_root.join("covers"),
            backups: data_root.join("backups"),
            temp: data_root.join("tmp"),
            diagnostics: data_root.join("diagnostics"),
            logs: data_root.join("logs"),
            downloads: data_root.join("downloads"),
            bootstrap_dir,
            app_data: data_root.clone(),
            data_root,
            resource_dir,
        }
    }

    pub fn account_root(&self, profile_key: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(profile_key, "invalid_profile_key")?;
        Ok(self.browser_profiles.join(profile_key))
    }

    pub fn account_browser_root(&self, profile_key: &str) -> Result<PathBuf, String> {
        self.account_root(profile_key)
    }

    pub fn result_path(&self, account_id: i64, job_id: i64) -> PathBuf {
        self.temp
            .join(format!("account-{account_id}-sync-{job_id}.result.json"))
    }

    pub fn orchestration_path(&self, account_id: i64, job_id: i64) -> PathBuf {
        self.temp.join(format!(
            "account-{account_id}-sync-{job_id}.orchestration.json"
        ))
    }

    pub fn phase_result_path(
        &self,
        account_id: i64,
        job_id: i64,
        phase: sync_orchestration::SyncPhase,
        phase_attempt_id: &str,
    ) -> Result<PathBuf, String> {
        self.phase_temp_path(account_id, job_id, phase, phase_attempt_id, "result.json")
    }

    pub fn phase_progress_path(
        &self,
        account_id: i64,
        job_id: i64,
        phase: sync_orchestration::SyncPhase,
        phase_attempt_id: &str,
    ) -> Result<PathBuf, String> {
        self.phase_temp_path(account_id, job_id, phase, phase_attempt_id, "progress.json")
    }

    pub fn phase_archive_known_ids_path(
        &self,
        account_id: i64,
        job_id: i64,
        phase: sync_orchestration::SyncPhase,
        phase_attempt_id: &str,
    ) -> Result<PathBuf, String> {
        self.phase_temp_path(
            account_id,
            job_id,
            phase,
            phase_attempt_id,
            "archive-known-ids.json",
        )
    }

    pub fn phase_browser_pid_path(
        &self,
        account_id: i64,
        job_id: i64,
        phase: sync_orchestration::SyncPhase,
        phase_attempt_id: &str,
    ) -> Result<PathBuf, String> {
        self.phase_temp_path(account_id, job_id, phase, phase_attempt_id, "browser.pid")
    }

    pub fn phase_collector_identity_path(
        &self,
        account_id: i64,
        job_id: i64,
        phase: sync_orchestration::SyncPhase,
        phase_attempt_id: &str,
    ) -> Result<PathBuf, String> {
        self.phase_temp_path(account_id, job_id, phase, phase_attempt_id, "collector.pid")
    }

    pub fn phase_diagnostic_path(
        &self,
        account_id: i64,
        job_id: i64,
        phase: sync_orchestration::SyncPhase,
        phase_attempt_id: &str,
    ) -> Result<PathBuf, String> {
        sync_orchestration::is_simple_uuid(phase_attempt_id)
            .then(|| {
                self.diagnostics
                    .join(format!("account-{account_id}"))
                    .join(format!(
                        "sync-{job_id}-{}-{phase_attempt_id}.json",
                        phase.as_str()
                    ))
            })
            .ok_or_else(|| "invalid_phase_attempt_id".to_string())
    }

    pub fn phase_log_path(
        &self,
        account_id: i64,
        job_id: i64,
        phase: sync_orchestration::SyncPhase,
        phase_attempt_id: &str,
    ) -> Result<PathBuf, String> {
        sync_orchestration::is_simple_uuid(phase_attempt_id)
            .then(|| {
                self.logs
                    .join(format!("account-{account_id}"))
                    .join(format!(
                        "sync-{job_id}-{}-{phase_attempt_id}.log",
                        phase.as_str()
                    ))
            })
            .ok_or_else(|| "invalid_phase_attempt_id".to_string())
    }

    fn phase_temp_path(
        &self,
        account_id: i64,
        job_id: i64,
        phase: sync_orchestration::SyncPhase,
        phase_attempt_id: &str,
        suffix: &str,
    ) -> Result<PathBuf, String> {
        if account_id <= 0 || job_id <= 0 || !sync_orchestration::is_simple_uuid(phase_attempt_id) {
            return Err("invalid_phase_identity".to_string());
        }
        Ok(self.temp.join(format!(
            "account-{account_id}-sync-{job_id}-{}-{phase_attempt_id}.{suffix}",
            phase.as_str()
        )))
    }

    pub fn progress_path(&self, account_id: i64, job_id: i64) -> PathBuf {
        self.temp
            .join(format!("account-{account_id}-sync-{job_id}.progress.json"))
    }

    pub fn cancel_path(&self, account_id: i64, job_id: i64) -> PathBuf {
        self.temp
            .join(format!("account-{account_id}-sync-{job_id}.cancel"))
    }

    pub fn diagnostic_path(&self, account_id: i64, job_id: i64) -> PathBuf {
        self.diagnostics
            .join(format!("account-{account_id}"))
            .join(format!("sync-{job_id}.json"))
    }

    pub fn browser_pid_path(&self, account_id: i64, job_id: i64) -> PathBuf {
        self.temp
            .join(format!("account-{account_id}-sync-{job_id}.browser.pid"))
    }

    pub fn log_path(&self, account_id: i64, job_id: i64) -> PathBuf {
        self.logs
            .join(format!("account-{account_id}"))
            .join(format!("sync-{job_id}.log"))
    }

    pub fn login_identity_path(
        &self,
        account_id: i64,
        attempt_id: &str,
    ) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.temp.join(format!(
            "account-{account_id}-login-{attempt_id}.identity.json"
        )))
    }

    pub fn login_attempt_path(&self, account_id: i64, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.temp.join(format!(
            "account-{account_id}-login-{attempt_id}.attempt.json"
        )))
    }

    pub fn login_cancel_path(&self, account_id: i64, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self
            .temp
            .join(format!("account-{account_id}-login-{attempt_id}.cancel")))
    }

    pub fn login_progress_path(
        &self,
        account_id: i64,
        attempt_id: &str,
    ) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.temp.join(format!(
            "account-{account_id}-login-{attempt_id}.progress.json"
        )))
    }

    pub fn login_browser_pid_path(
        &self,
        account_id: i64,
        attempt_id: &str,
    ) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.temp.join(format!(
            "account-{account_id}-login-{attempt_id}.browser.pid"
        )))
    }

    pub fn login_diagnostic_path(
        &self,
        account_id: i64,
        attempt_id: &str,
    ) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self
            .diagnostics
            .join(format!("account-{account_id}"))
            .join(format!("login-{attempt_id}.json")))
    }

    pub fn login_log_path(&self, account_id: i64, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self
            .logs
            .join(format!("account-{account_id}"))
            .join(format!("login-{attempt_id}.log")))
    }

    pub fn pending_profile_cleanup_path(&self, profile_key: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(profile_key, "invalid_profile_key")?;
        Ok(self
            .temp
            .join(format!("pending-profile-cleanup-{profile_key}.json")))
    }

    pub fn login_attempt_state_path(&self, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self
            .temp
            .join(format!("login-attempt-{attempt_id}.state.json")))
    }

    pub fn login_task_identity_path(&self, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.temp.join(format!("login-{attempt_id}.identity.json")))
    }

    pub fn login_task_progress_path(&self, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.temp.join(format!("login-{attempt_id}.progress.json")))
    }

    pub fn login_task_cancel_path(&self, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.temp.join(format!("login-{attempt_id}.cancel")))
    }

    pub fn login_task_browser_pid_path(&self, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.temp.join(format!("login-{attempt_id}.browser.pid")))
    }

    pub fn login_task_collector_identity_path(&self, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.temp.join(format!("login-{attempt_id}.collector.pid")))
    }

    pub fn login_task_diagnostic_path(&self, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.diagnostics.join(format!("login-{attempt_id}.json")))
    }

    pub fn login_task_log_path(&self, attempt_id: &str) -> Result<PathBuf, String> {
        validate_simple_uuid(attempt_id, "invalid_login_attempt_id")?;
        Ok(self.logs.join(format!("login-{attempt_id}.log")))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginAttemptV1 {
    pub schema_version: i64,
    pub account_id: i64,
    pub login_attempt_id: String,
    pub profile_key: String,
    pub started_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingProfileCleanupV1 {
    pub schema_version: i64,
    pub profile_key: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCleanupReport {
    pub primary_error: Option<String>,
    pub deferred_profile: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedLogin {
    pub account: accounts::AccountView,
    pub active_account_warning: Option<String>,
}

pub fn create_login_attempt(
    paths: &AppPaths,
    account_id: i64,
    login_attempt_id: &str,
    profile_key: &str,
) -> Result<LoginAttemptV1, String> {
    if account_id <= 0 {
        return Err("invalid_account_id".to_string());
    }
    validate_simple_uuid(login_attempt_id, "invalid_login_attempt_id")?;
    validate_simple_uuid(profile_key, "invalid_profile_key")?;
    let attempt = LoginAttemptV1 {
        schema_version: 1,
        account_id,
        login_attempt_id: login_attempt_id.to_string(),
        profile_key: profile_key.to_string(),
        started_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    };
    collector::file_store::write_json_atomically(
        &paths.login_attempt_path(account_id, login_attempt_id)?,
        &attempt,
    )?;
    Ok(attempt)
}

pub fn read_login_attempt(
    paths: &AppPaths,
    account_id: i64,
    login_attempt_id: &str,
) -> Result<LoginAttemptV1, String> {
    let attempt: LoginAttemptV1 =
        collector::file_store::read_json(&paths.login_attempt_path(account_id, login_attempt_id)?)?;
    if attempt.schema_version != 1
        || attempt.account_id != account_id
        || attempt.login_attempt_id != login_attempt_id
        || !matches!(
            validate_simple_uuid(&attempt.profile_key, "invalid_profile_key"),
            Ok(())
        )
        || DateTime::parse_from_rfc3339(&attempt.started_at).is_err()
    {
        return Err("invalid_login_attempt".to_string());
    }
    Ok(attempt)
}

pub fn cleanup_login_attempt_files(
    paths: &AppPaths,
    account_id: i64,
    login_attempt_id: &str,
) -> Result<(), String> {
    for path in [
        paths.login_attempt_path(account_id, login_attempt_id)?,
        paths.login_identity_path(account_id, login_attempt_id)?,
        paths.login_progress_path(account_id, login_attempt_id)?,
        paths.login_cancel_path(account_id, login_attempt_id)?,
        paths.login_browser_pid_path(account_id, login_attempt_id)?,
    ] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

pub fn cleanup_failed_login_with<R>(
    paths: &AppPaths,
    connection: &mut Connection,
    account_id: i64,
    login_attempt_id: &str,
    mut remove_profile: R,
) -> LoginCleanupReport
where
    R: FnMut(&Path) -> Result<(), String>,
{
    let mut report = LoginCleanupReport {
        primary_error: None,
        deferred_profile: false,
    };
    let account = match accounts::get_account(connection, account_id) {
        Ok(account) => account,
        Err(error) => {
            report.primary_error = Some(error);
            None
        }
    };
    let pending = account.filter(|account| account.status == "pending_login");
    let mut row_deleted = false;
    if pending.is_some() {
        match connection.execute(
            "DELETE FROM accounts WHERE id = ?1 AND status = 'pending_login'",
            [account_id],
        ) {
            Ok(1) => row_deleted = true,
            Ok(_) => {
                record_first_error(&mut report.primary_error, "account_not_pending".to_string())
            }
            Err(error) => record_first_error(&mut report.primary_error, error.to_string()),
        }
    }
    if let Err(error) = cleanup_login_attempt_files(paths, account_id, login_attempt_id) {
        record_first_error(&mut report.primary_error, error);
    }
    if let Some(account) = pending.filter(|_| row_deleted) {
        let profile = match paths.account_root(&account.profile_key) {
            Ok(profile) => profile,
            Err(error) => {
                record_first_error(&mut report.primary_error, error);
                return report;
            }
        };
        if profile.exists() {
            if let Err(error) = remove_profile(&profile) {
                record_first_error(&mut report.primary_error, error);
                let marker = PendingProfileCleanupV1 {
                    schema_version: 1,
                    profile_key: account.profile_key.clone(),
                    created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
                };
                match paths.pending_profile_cleanup_path(&account.profile_key) {
                    Ok(path) => {
                        match collector::file_store::write_json_atomically(&path, &marker) {
                            Ok(()) => report.deferred_profile = true,
                            Err(error) => record_first_error(&mut report.primary_error, error),
                        }
                    }
                    Err(error) => record_first_error(&mut report.primary_error, error),
                }
            }
        }
    }
    report
}

pub fn cleanup_failed_login(
    paths: &AppPaths,
    connection: &mut Connection,
    account_id: i64,
    login_attempt_id: &str,
) -> LoginCleanupReport {
    cleanup_failed_login_with(paths, connection, account_id, login_attempt_id, |profile| {
        remove_controlled_cleanup_profile(paths, profile)
    })
}

pub async fn run_pending_login_guarded<T, F>(
    paths: &AppPaths,
    account_id: i64,
    login_attempt_id: &str,
    operation: F,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    let result = operation.await;
    if result.is_err() {
        let mut deferred_profile = false;
        if let Ok(mut connection) = Connection::open(&paths.database) {
            let _ = assert_schema_v5(&connection);
            deferred_profile =
                cleanup_failed_login(paths, &mut connection, account_id, login_attempt_id)
                    .deferred_profile;
        }
        if deferred_profile {
            let paths = paths.clone();
            tokio::spawn(async move {
                for delay_ms in pending_profile_cleanup_retry_delays() {
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    let _ = retry_pending_profile_cleanups(&paths);
                    let has_markers = fs::read_dir(&paths.temp)
                        .ok()
                        .into_iter()
                        .flatten()
                        .filter_map(Result::ok)
                        .any(|entry| {
                            entry.file_name().to_str().is_some_and(|name| {
                                name.starts_with("pending-profile-cleanup-")
                                    && name.ends_with(".json")
                            })
                        });
                    if !has_markers {
                        break;
                    }
                }
            });
        }
    }
    result
}

pub fn pending_profile_cleanup_retry_delays() -> Vec<u64> {
    vec![250, 500, 1_000, 2_000, 4_000]
}

pub fn retry_pending_profile_cleanups(paths: &AppPaths) -> Result<(), String> {
    retry_pending_profile_cleanups_with(paths, |profile| {
        remove_controlled_cleanup_profile(paths, profile)
    })
}

pub fn retry_pending_profile_cleanups_with<R>(
    paths: &AppPaths,
    mut remove_profile: R,
) -> Result<(), String>
where
    R: FnMut(&Path) -> Result<(), String>,
{
    let entries = match fs::read_dir(&paths.temp) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("pending-profile-cleanup-") || !name.ends_with(".json") {
            continue;
        }
        let marker =
            match collector::file_store::read_json::<PendingProfileCleanupV1>(&entry.path()) {
                Ok(marker)
                    if marker.schema_version == 1
                        && validate_simple_uuid(&marker.profile_key, "invalid_profile_key")
                            .is_ok()
                        && paths
                            .pending_profile_cleanup_path(&marker.profile_key)
                            .ok()
                            .as_deref()
                            == Some(entry.path().as_path()) =>
                {
                    marker
                }
                _ => {
                    let _ = fs::remove_file(entry.path());
                    continue;
                }
            };
        let profile = paths.account_root(&marker.profile_key)?;
        if !profile.exists() || remove_profile(&profile).is_ok() {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}

fn record_first_error(target: &mut Option<String>, error: String) {
    if target.is_none() {
        *target = Some(error);
    }
}

pub fn complete_login_attempt(
    paths: &AppPaths,
    account_id: i64,
    login_attempt_id: &str,
) -> Result<accounts::AccountView, String> {
    complete_login_attempt_with_active_writer(
        paths,
        account_id,
        login_attempt_id,
        |directory, active_account_id| {
            bootstrap::update_active_account(directory, active_account_id).map(|_| ())
        },
    )
    .map(|completed| completed.account)
}

pub fn complete_login_attempt_with_active_writer<W>(
    paths: &AppPaths,
    account_id: i64,
    login_attempt_id: &str,
    mut write_settings: W,
) -> Result<CompletedLogin, String>
where
    W: FnMut(&Path, Option<i64>) -> Result<(), String>,
{
    let attempt = read_login_attempt(paths, account_id, login_attempt_id)?;
    let identity: collector::contracts::AccountIdentityV1 = collector::file_store::read_json(
        &paths.login_identity_path(account_id, login_attempt_id)?,
    )?;
    let identity = identity.validated_for(account_id, login_attempt_id, &attempt.profile_key)?;

    let mut connection = Connection::open(&paths.database).map_err(|error| error.to_string())?;
    assert_schema_v5(&connection)?;
    let account = accounts::finalize_account_identity(
        &mut connection,
        account_id,
        accounts::AccountIdentity {
            uid: identity.uid,
            sec_uid: identity.sec_uid,
            nickname: identity.nickname,
            avatar_url: identity.avatar_url,
        },
    )?;
    let mut active_account_warning = None;
    if write_settings(&paths.bootstrap_dir, Some(account_id)).is_err()
        && write_settings(&paths.bootstrap_dir, Some(account_id)).is_err()
    {
        active_account_warning = Some("bootstrap_update_failed".to_string());
    }
    let _ = cleanup_login_attempt_files(paths, account_id, login_attempt_id);
    Ok(CompletedLogin {
        account,
        active_account_warning,
    })
}

pub fn recover_login_attempts_with<F>(
    paths: &AppPaths,
    connection: &mut Connection,
    mut terminate: F,
) -> Result<(), String>
where
    F: FnMut(&collector::browser_process::BrowserProcessRecordV2, &Path) -> Result<(), String>,
{
    let entries = match fs::read_dir(&paths.temp) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(".attempt.json") {
            continue;
        }
        let Ok(attempt) = collector::file_store::read_json::<LoginAttemptV1>(&entry.path()) else {
            continue;
        };
        if paths
            .login_attempt_path(attempt.account_id, &attempt.login_attempt_id)
            .ok()
            .as_deref()
            != Some(entry.path().as_path())
        {
            continue;
        }
        let Ok(attempt) = read_login_attempt(paths, attempt.account_id, &attempt.login_attempt_id)
        else {
            continue;
        };
        let account = accounts::get_account(connection, attempt.account_id)?;
        if let Ok(contents) = fs::read_to_string(
            paths.login_browser_pid_path(attempt.account_id, &attempt.login_attempt_id)?,
        ) {
            if let Ok(record) = serde_json::from_str::<
                collector::browser_process::BrowserProcessRecordV2,
            >(&contents)
            {
                if let Some(account) = account
                    .as_ref()
                    .filter(|account| account.profile_key == attempt.profile_key)
                {
                    let account_root = paths.account_browser_root(&account.profile_key)?;
                    let expected_profile = collector::browser_state::browser_profile_path(
                        &account_root,
                        record.browser_id,
                    );
                    if collector::browser_process::browser_record_matches_expected_profile(
                        &record,
                        &expected_profile,
                    ) {
                        let _ = terminate(&record, &expected_profile);
                    }
                }
            }
        }
        cleanup_login_attempt_files(paths, attempt.account_id, &attempt.login_attempt_id)?;
    }
    let pending_accounts = accounts::list_accounts(connection)?
        .into_iter()
        .filter(|account| account.status == "pending_login")
        .collect::<Vec<_>>();
    for account in pending_accounts {
        let _ = cleanup_failed_login_with(
            paths,
            connection,
            account.id,
            "00000000000000000000000000000000",
            |profile| remove_controlled_cleanup_profile(paths, profile),
        );
    }
    retry_pending_profile_cleanups_with(paths, |profile| {
        remove_controlled_cleanup_profile(paths, profile)
    })?;
    Ok(())
}

fn remove_controlled_cleanup_profile(paths: &AppPaths, profile: &Path) -> Result<(), String> {
    let parent = profile
        .parent()
        .ok_or_else(|| "profile_path_not_controlled".to_string())?;
    if parent != paths.browser_profiles || !profile.exists() {
        return if profile.exists() {
            Err("profile_path_not_controlled".to_string())
        } else {
            Ok(())
        };
    }
    let metadata =
        fs::symlink_metadata(profile).map_err(|_| "profile_path_not_controlled".to_string())?;
    if metadata.file_type().is_symlink() || is_cleanup_reparse_point(&metadata) {
        return Err("profile_path_not_controlled".to_string());
    }
    let canonical_root = fs::canonicalize(&paths.browser_profiles)
        .map_err(|_| "profile_path_not_controlled".to_string())?;
    let canonical_profile =
        fs::canonicalize(profile).map_err(|_| "profile_path_not_controlled".to_string())?;
    if canonical_profile.parent() != Some(canonical_root.as_path()) {
        return Err("profile_path_not_controlled".to_string());
    }
    fs::remove_dir_all(profile).map_err(|_| "profile_cleanup_failed".to_string())
}

#[cfg(windows)]
fn is_cleanup_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_cleanup_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn validate_simple_uuid(value: &str, error: &str) -> Result<(), String> {
    if value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(error.to_string())
    }
}

fn node_compatible_path(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = raw.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncOptions {
    pub mode: String,
    pub days: Option<u32>,
    pub scroll_speed: Option<u32>,
    pub hide_browser: Option<bool>,
    pub block_media: Option<bool>,
    pub block_images: Option<bool>,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            mode: "days".to_string(),
            days: Some(7),
            scroll_speed: Some(900),
            hide_browser: Some(false),
            block_media: None,
            block_images: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectorInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub current_dir: Option<PathBuf>,
}

pub fn sync_options_hash(options: &SyncOptions) -> Result<String, String> {
    let bytes = serde_json::to_vec(options).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserProcessRecord {
    pub schema_version: i64,
    pub pid: u32,
    pub browser_id: String,
    pub executable_path: String,
    pub profile_path: String,
    pub started_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserProcessInfo {
    pub executable_path: String,
    pub command_line: String,
}

pub fn build_collect_invocation(
    paths: &AppPaths,
    executable: PathBuf,
    account_id: i64,
    profile_key: &str,
    job_id: i64,
    options: &SyncOptions,
) -> Result<CollectorInvocation, String> {
    let account_profile_root = paths.account_browser_root(profile_key)?;
    let mut args = vec!["--collector".to_string()];
    args.extend([
        "collect".to_string(),
        "--account-id".to_string(),
        account_id.to_string(),
        "--job-id".to_string(),
        job_id.to_string(),
        "--browser-data-dir".to_string(),
        paths.app_data.to_string_lossy().to_string(),
        "--result".to_string(),
        paths
            .result_path(account_id, job_id)
            .to_string_lossy()
            .to_string(),
        "--progress".to_string(),
        paths
            .progress_path(account_id, job_id)
            .to_string_lossy()
            .to_string(),
        "--diagnostic".to_string(),
        paths
            .diagnostic_path(account_id, job_id)
            .to_string_lossy()
            .to_string(),
        "--cancel-file".to_string(),
        paths
            .cancel_path(account_id, job_id)
            .to_string_lossy()
            .to_string(),
        "--account-profile-root".to_string(),
        account_profile_root.to_string_lossy().to_string(),
        "--browser-pid-file".to_string(),
        paths
            .browser_pid_path(account_id, job_id)
            .to_string_lossy()
            .to_string(),
        "--browser".to_string(),
        "auto".to_string(),
        "--scroll-speed".to_string(),
        options.scroll_speed.unwrap_or(6_000).max(1).to_string(),
    ]);
    append_sync_options(&mut args, options);
    Ok(CollectorInvocation {
        program: executable,
        args,
        current_dir: None,
    })
}

pub fn build_phase_collect_invocation(
    paths: &AppPaths,
    executable: PathBuf,
    account_id: i64,
    profile_key: &str,
    job_id: i64,
    phase: sync_orchestration::SyncPhase,
    phase_attempt_id: &str,
    options: &SyncOptions,
) -> Result<CollectorInvocation, String> {
    let account_profile_root = paths.account_browser_root(profile_key)?;
    let mut args = vec!["--collector".to_string()];
    args.extend([
        "collect".to_string(),
        "--account-id".to_string(),
        account_id.to_string(),
        "--job-id".to_string(),
        job_id.to_string(),
        "--phase".to_string(),
        phase.as_str().to_string(),
        "--phase-attempt-id".to_string(),
        phase_attempt_id.to_string(),
        "--browser-data-dir".to_string(),
        paths.app_data.to_string_lossy().to_string(),
        "--result".to_string(),
        paths
            .phase_result_path(account_id, job_id, phase, phase_attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--progress".to_string(),
        paths
            .phase_progress_path(account_id, job_id, phase, phase_attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--diagnostic".to_string(),
        paths
            .phase_diagnostic_path(account_id, job_id, phase, phase_attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--cancel-file".to_string(),
        paths
            .cancel_path(account_id, job_id)
            .to_string_lossy()
            .to_string(),
        "--account-profile-root".to_string(),
        account_profile_root.to_string_lossy().to_string(),
        "--browser-pid-file".to_string(),
        paths
            .phase_browser_pid_path(account_id, job_id, phase, phase_attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--collector-identity-file".to_string(),
        paths
            .phase_collector_identity_path(account_id, job_id, phase, phase_attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--browser".to_string(),
        "auto".to_string(),
        "--scroll-speed".to_string(),
        options.scroll_speed.unwrap_or(6_000).max(1).to_string(),
    ]);
    append_sync_options(&mut args, options);
    Ok(CollectorInvocation {
        program: executable,
        args,
        current_dir: None,
    })
}

pub fn build_recover_login_invocation(
    paths: &AppPaths,
    executable: PathBuf,
    account_id: i64,
    profile_key: &str,
    job_id: i64,
    phase_attempt_id: &str,
    deadline_at_ms: i64,
) -> Result<CollectorInvocation, String> {
    if deadline_at_ms <= 0 {
        return Err("invalid_deadline_at_ms".to_string());
    }
    let phase = sync_orchestration::SyncPhase::RecoveringLogin;
    let account_profile_root = paths.account_browser_root(profile_key)?;
    let args = vec![
        "--collector".to_string(),
        "recover-login".to_string(),
        "--account-id".to_string(),
        account_id.to_string(),
        "--job-id".to_string(),
        job_id.to_string(),
        "--phase".to_string(),
        phase.as_str().to_string(),
        "--phase-attempt-id".to_string(),
        phase_attempt_id.to_string(),
        "--browser-data-dir".to_string(),
        paths.app_data.to_string_lossy().to_string(),
        "--result".to_string(),
        paths
            .phase_result_path(account_id, job_id, phase, phase_attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--progress".to_string(),
        paths
            .phase_progress_path(account_id, job_id, phase, phase_attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--diagnostic".to_string(),
        paths
            .phase_diagnostic_path(account_id, job_id, phase, phase_attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--cancel-file".to_string(),
        paths
            .cancel_path(account_id, job_id)
            .to_string_lossy()
            .to_string(),
        "--account-profile-root".to_string(),
        account_profile_root.to_string_lossy().to_string(),
        "--browser-pid-file".to_string(),
        paths
            .phase_browser_pid_path(account_id, job_id, phase, phase_attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--collector-identity-file".to_string(),
        paths
            .phase_collector_identity_path(account_id, job_id, phase, phase_attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--browser".to_string(),
        "auto".to_string(),
        "--home-url".to_string(),
        collector::session_probe::DOUYIN_HOME_URL.to_string(),
        "--deadline-at-ms".to_string(),
        deadline_at_ms.to_string(),
        "--headless".to_string(),
    ];
    Ok(CollectorInvocation {
        program: executable,
        args,
        current_dir: None,
    })
}

fn append_sync_options(args: &mut Vec<String>, options: &SyncOptions) {
    if options.mode == "bottom" {
        args.push("--bottom".to_string());
    } else {
        args.extend([
            "--days".to_string(),
            options.days.unwrap_or(7).clamp(1, 365).to_string(),
        ]);
    }
    if options.hide_browser.unwrap_or(false) {
        args.push("--headless".to_string());
    }
    args.push("--block-images".to_string());
}

pub fn build_login_invocation(
    paths: &AppPaths,
    executable: PathBuf,
    target_account_id: Option<i64>,
    profile_key: &str,
    attempt_id: &str,
    browser_preference: crate::device_settings::BrowserPreferenceV1,
) -> Result<CollectorInvocation, String> {
    let account_profile_root = paths.account_browser_root(profile_key)?;
    let mut args = vec!["--collector".to_string()];
    args.extend([
        "login".to_string(),
        "--login-attempt-id".to_string(),
        attempt_id.to_string(),
        "--browser-data-dir".to_string(),
        paths.app_data.to_string_lossy().to_string(),
        "--account-profile-root".to_string(),
        account_profile_root.to_string_lossy().to_string(),
        "--identity-result".to_string(),
        paths
            .login_task_identity_path(attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--login-progress-file".to_string(),
        paths
            .login_task_progress_path(attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--diagnostic".to_string(),
        paths
            .login_task_diagnostic_path(attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--cancel-file".to_string(),
        paths
            .login_task_cancel_path(attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--browser-pid-file".to_string(),
        paths
            .login_task_browser_pid_path(attempt_id)?
            .to_string_lossy()
            .to_string(),
        "--browser".to_string(),
        browser_preference.as_str().to_string(),
        "--home-url".to_string(),
        collector::session_probe::DOUYIN_HOME_URL.to_string(),
    ]);
    if let Some(target_account_id) = target_account_id {
        if target_account_id <= 0 {
            return Err("invalid_account_id".to_string());
        }
        args.extend([
            "--target-account-id".to_string(),
            target_account_id.to_string(),
        ]);
    }
    Ok(CollectorInvocation {
        program: executable,
        args,
        current_dir: None,
    })
}

pub fn parse_browser_process_record(contents: &str) -> Option<BrowserProcessRecord> {
    let record: BrowserProcessRecord = serde_json::from_str(contents).ok()?;
    let expected_executable = match record.browser_id.as_str() {
        "edge" => "msedge.exe",
        "chrome" => "chrome.exe",
        _ => return None,
    };
    if record.schema_version != 1
        || record.pid == 0
        || !Path::new(&record.executable_path).is_absolute()
        || !Path::new(&record.profile_path).is_absolute()
        || Path::new(&record.executable_path)
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| !value.eq_ignore_ascii_case(expected_executable))
            .unwrap_or(true)
        || DateTime::parse_from_rfc3339(&record.started_at).is_err()
    {
        return None;
    }
    Some(record)
}

pub fn browser_process_matches(
    record: &BrowserProcessRecord,
    process: &BrowserProcessInfo,
) -> bool {
    let expected_executable = normalize_windows_text(&record.executable_path);
    let actual_executable = normalize_windows_text(&process.executable_path);
    let command_line = normalize_windows_text(&process.command_line);
    let profile_path = normalize_windows_text(&record.profile_path);
    expected_executable == actual_executable
        && command_line.contains("--user-data-dir=")
        && command_line.contains(&profile_path)
}

fn normalize_windows_text(value: &str) -> String {
    value.replace('/', "\\").to_lowercase()
}

pub fn configure_connection(connection: &Connection) -> Result<(), String> {
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())
}

pub fn initialize_schema_v2(connection: &mut Connection) -> Result<(), String> {
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .map_err(|error| error.to_string())?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version < SCHEMA_VERSION_V2 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch(MIGRATION_V2)
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
    }
    assert_schema_v2(connection)
}

pub fn initialize_schema_v3(connection: &mut Connection) -> Result<(), String> {
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .map_err(|error| error.to_string())?;
    assert_foreign_keys_enabled(connection)?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    match version {
        SCHEMA_VERSION_V3 => assert_schema_v3(connection),
        0..=SCHEMA_VERSION_V2 => {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| error.to_string())?;
            transaction
                .execute_batch(MIGRATION_V3)
                .map_err(|error| error.to_string())?;
            assert_schema_v3(&transaction)?;
            transaction.commit().map_err(|error| error.to_string())
        }
        _ => Err(format!("unsupported_schema_version:{version}")),
    }
}

pub fn initialize_schema_v4(connection: &mut Connection) -> Result<(), String> {
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .map_err(|error| error.to_string())?;
    assert_foreign_keys_enabled(connection)?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    match version {
        SCHEMA_VERSION_V4 => assert_schema_v4(connection),
        SCHEMA_VERSION_V3 => migrate_schema_v3_to_v4(connection),
        0..=SCHEMA_VERSION_V2 => {
            initialize_schema_v3(connection)?;
            migrate_schema_v3_to_v4(connection)
        }
        _ => Err(format!("unsupported_schema_version:{version}")),
    }
}

fn migrate_schema_v3_to_v4(connection: &mut Connection) -> Result<(), String> {
    connection
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(|error| error.to_string())?;
    let migration = (|| {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch(MIGRATION_V4)
            .map_err(|error| error.to_string())?;
        assert_schema_v4(&transaction)?;
        transaction.commit().map_err(|error| error.to_string())
    })();
    let restore = connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| error.to_string())
        .and_then(|_| assert_foreign_keys_enabled(connection));
    match (migration, restore) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(_), Ok(_)) => Ok(()),
    }
}

pub fn compute_collector_hashes(records: &[CollectorRecordV2]) -> Result<(String, String), String> {
    let identity: Vec<Value> = records
        .iter()
        .map(|record| json!([record.aweme_id, record.history_time_ms]))
        .collect();
    let fields: Vec<Value> = records
        .iter()
        .map(|record| {
            json!([
                record.aweme_id,
                record.aweme_type,
                record.canonical_url,
                record.content_type,
                record.history_time_ms,
                record.watched_at,
                record.watched_date,
                record.source_position,
                record.title,
                record.author_name,
                record.cover_url,
                record.like_count,
                record.watch_progress_rate
            ])
        })
        .collect();
    Ok((
        sha256(&serde_json::to_string(&identity).map_err(|error| error.to_string())?),
        sha256(&serde_json::to_string(&fields).map_err(|error| error.to_string())?),
    ))
}

pub fn validate_and_import_result(
    connection: &mut Connection,
    result: &CollectorSuccessV3,
    expected_sync_job_id: i64,
) -> Result<ImportSummary, String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let job: Option<(i64, String)> = transaction
        .query_row(
            "SELECT account_id, status FROM sync_jobs WHERE id = ?1",
            [expected_sync_job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((account_id, job_status)) = job else {
        return Err("sync_job_not_running".to_string());
    };
    if job_status != "running" {
        return Err("sync_job_not_running".to_string());
    }
    if result.account_id != account_id {
        return Err("sync_account_mismatch".to_string());
    }

    let summary = import_history_result_tx(&transaction, result, expected_sync_job_id, account_id)?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let updated_jobs = transaction
        .execute(
            "
            UPDATE sync_jobs
            SET status = 'success', finished_at = ?2, inserted_videos = ?3,
                inserted_records = ?4, error_code = NULL, error_message = NULL
            WHERE id = ?1 AND account_id = ?5 AND status = 'running'
            ",
            params![
                expected_sync_job_id,
                now,
                summary.inserted_videos,
                summary.inserted_watch_records,
                account_id
            ],
        )
        .map_err(|error| error.to_string())?;
    if updated_jobs != 1 {
        return Err("sync_job_mismatch".to_string());
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(summary)
}

pub fn import_history_result_tx(
    transaction: &Transaction<'_>,
    result: &CollectorSuccessV3,
    expected_sync_job_id: i64,
    expected_account_id: i64,
) -> Result<ImportSummary, String> {
    validate_collector_result(result, expected_sync_job_id)?;
    if result.account_id != expected_account_id {
        return Err("sync_account_mismatch".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut summary = ImportSummary::default();
    for record in &result.records {
        let existing_video: Option<i64> = transaction
            .query_row(
                "SELECT id FROM videos WHERE aweme_id = ?1",
                [&record.aweme_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "
                INSERT INTO videos
                  (aweme_id, canonical_url, content_type, title, author_name, cover_url, like_count, first_seen_at, last_seen_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                ON CONFLICT(aweme_id) DO UPDATE SET
                  canonical_url = excluded.canonical_url,
                  content_type = excluded.content_type,
                  title = COALESCE(excluded.title, videos.title),
                  author_name = COALESCE(excluded.author_name, videos.author_name),
                  cover_url = COALESCE(excluded.cover_url, videos.cover_url),
                  like_count = COALESCE(excluded.like_count, videos.like_count),
                  last_seen_at = excluded.last_seen_at
                ",
                params![
                    record.aweme_id,
                    record.canonical_url,
                    record.content_type,
                    record.title,
                    record.author_name,
                    record.cover_url,
                    record.like_count,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
        let video_id: i64 = transaction
            .query_row(
                "SELECT id FROM videos WHERE aweme_id = ?1",
                [&record.aweme_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if existing_video.is_none() {
            summary.inserted_videos += 1;
        }
        if record
            .cover_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
        {
            crate::cover_cache::enqueue_cover_candidate_best_effort(
                transaction,
                video_id,
                expected_sync_job_id,
                &now,
            );
        }
        let existing_watch: Option<i64> = transaction
            .query_row(
                "SELECT id FROM watch_records WHERE account_id = ?1 AND video_id = ?2 AND history_time_ms = ?3",
                params![expected_account_id, video_id, record.history_time_ms],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "
                INSERT INTO watch_records
                  (account_id, video_id, history_time_ms, watched_at, watched_date, source_position, watch_progress_rate, sync_job_id, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                ON CONFLICT(account_id, video_id, history_time_ms) DO UPDATE SET
                  source_position = excluded.source_position,
                  watch_progress_rate = COALESCE(excluded.watch_progress_rate, watch_records.watch_progress_rate),
                  sync_job_id = excluded.sync_job_id,
                  updated_at = excluded.updated_at
                ",
                params![
                    expected_account_id,
                    video_id,
                    record.history_time_ms,
                    record.watched_at,
                    record.watched_date,
                    record.source_position,
                    record.watch_progress_rate,
                    expected_sync_job_id,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
        if existing_watch.is_none() {
            summary.inserted_watch_records += 1;
        }
    }
    Ok(summary)
}

pub fn list_history(
    connection: &Connection,
    account_id: i64,
    query: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<ListHistoryResult, String> {
    validate_account_id(account_id)?;
    let trimmed = query.unwrap_or_default().trim();
    let like_query = if trimmed.is_empty() {
        None
    } else {
        Some(format!("%{trimmed}%"))
    };
    let total: i64 = connection
        .query_row(
            "
            SELECT COUNT(*)
            FROM watch_records wr
            JOIN videos v ON v.id = wr.video_id
            WHERE wr.account_id = ?1
              AND (?2 IS NULL OR v.title LIKE ?2 OR v.author_name LIKE ?2)
            ",
            params![account_id, like_query.as_deref()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(
            "
            SELECT
              wr.id, v.aweme_id, v.canonical_url, v.content_type, v.title,
              v.author_name, v.cover_url, v.like_count, wr.history_time_ms,
              wr.watched_at, wr.watched_date, wr.source_position, wr.watch_progress_rate
            FROM watch_records wr
            JOIN videos v ON v.id = wr.video_id
            WHERE wr.account_id = ?1
              AND (?2 IS NULL OR v.title LIKE ?2 OR v.author_name LIKE ?2)
            ORDER BY wr.history_time_ms DESC, wr.source_position ASC, wr.id DESC
            LIMIT ?3 OFFSET ?4
            ",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                account_id,
                like_query,
                limit.clamp(1, 200) as i64,
                offset as i64
            ],
            |row| {
                Ok(HistoryRecordView {
                    id: row.get(0)?,
                    aweme_id: row.get(1)?,
                    url: row.get(2)?,
                    content_type: row.get(3)?,
                    title: row.get(4)?,
                    author_name: row.get(5)?,
                    cover_url: row.get(6)?,
                    cached_cover_url: None,
                    like_count: row.get(7)?,
                    history_time_ms: row.get(8)?,
                    watched_at: row.get(9)?,
                    watched_date: row.get(10)?,
                    source_position: row.get(11)?,
                    watch_progress_rate: row.get(12)?,
                })
            },
        )
        .map_err(|error| error.to_string())?;
    let mut records = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for record in &mut records {
        record.cached_cover_url =
            crate::cover_cache::cached_cover_url_for_aweme_id(connection, &record.aweme_id)?;
    }
    Ok(ListHistoryResult { records, total })
}

pub fn delete_history_by_ids(
    connection: &mut Connection,
    account_id: i64,
    ids: &[i64],
) -> Result<DeleteHistoryResult, String> {
    validate_account_id(account_id)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let mut deleted_watch_records = 0_i64;
    for id in ids {
        deleted_watch_records += transaction
            .execute(
                "DELETE FROM watch_records WHERE id = ?1 AND account_id = ?2",
                params![id, account_id],
            )
            .map_err(|error| error.to_string())? as i64;
    }
    let deleted_videos = transaction
        .execute(
            "
            DELETE FROM videos
            WHERE id NOT IN (SELECT video_id FROM watch_records)
              AND id NOT IN (SELECT video_id FROM user_likes)
              AND id NOT IN (SELECT video_id FROM favorite_items)
              AND id NOT IN (SELECT video_id FROM remote_folder_items)
              AND id NOT IN (SELECT video_id FROM local_folder_items)
            ",
            [],
        )
        .map_err(|error| error.to_string())? as i64;
    let remaining_watch_records: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM watch_records WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(DeleteHistoryResult {
        deleted_watch_records,
        deleted_videos,
        remaining_watch_records,
    })
}

pub fn delete_history_by_query(
    connection: &mut Connection,
    account_id: i64,
    query: Option<&str>,
) -> Result<DeleteHistoryResult, String> {
    validate_account_id(account_id)?;
    let trimmed = query.unwrap_or_default().trim();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let deleted_watch_records = if trimmed.is_empty() {
        transaction
            .execute(
                "DELETE FROM watch_records WHERE account_id = ?1",
                [account_id],
            )
            .map_err(|error| error.to_string())? as i64
    } else {
        transaction
            .execute(
                "
                DELETE FROM watch_records
                WHERE id IN (
                  SELECT wr.id FROM watch_records wr
                  JOIN videos v ON v.id = wr.video_id
                  WHERE wr.account_id = ?1
                    AND (v.title LIKE ?2 OR v.author_name LIKE ?2)
                )
                ",
                params![account_id, format!("%{trimmed}%")],
            )
            .map_err(|error| error.to_string())? as i64
    };
    let deleted_videos = transaction
        .execute(
            "
            DELETE FROM videos
            WHERE id NOT IN (SELECT video_id FROM watch_records)
              AND id NOT IN (SELECT video_id FROM user_likes)
              AND id NOT IN (SELECT video_id FROM favorite_items)
              AND id NOT IN (SELECT video_id FROM remote_folder_items)
              AND id NOT IN (SELECT video_id FROM local_folder_items)
            ",
            [],
        )
        .map_err(|error| error.to_string())? as i64;
    let remaining_watch_records: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM watch_records WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(DeleteHistoryResult {
        deleted_watch_records,
        deleted_videos,
        remaining_watch_records,
    })
}

fn validate_account_id(account_id: i64) -> Result<(), String> {
    if account_id <= 0 {
        Err("invalid_account_id".to_string())
    } else {
        Ok(())
    }
}

pub fn validate_collector_result(
    result: &CollectorSuccessV3,
    expected_sync_job_id: i64,
) -> Result<(), String> {
    if result.schema_version != SCHEMA_VERSION_V3 {
        return Err("invalid_result_schema_version".to_string());
    }
    if result.account_id <= 0 {
        return Err("invalid_account_id".to_string());
    }
    if result.sync_job_id != expected_sync_job_id {
        return Err("sync_job_id_mismatch".to_string());
    }
    if result.calendar_time_zone != "Asia/Shanghai" {
        return Err("calendar_time_zone_mismatch".to_string());
    }
    if result.detected_history_time_unit != "seconds"
        && result.detected_history_time_unit != "milliseconds"
    {
        return Err("invalid_history_time_unit".to_string());
    }
    validate_sync_started_at(result.sync_started_at_ms)?;
    validate_range(&result.range, result.sync_started_at_ms)?;
    validate_records(&result.records, result.sync_started_at_ms)?;
    let (identity_hash, field_hash) = compute_collector_hashes(&result.records)?;
    if identity_hash != result.identity_hash {
        return Err("identity_hash_mismatch".to_string());
    }
    if field_hash != result.field_hash {
        return Err("field_hash_mismatch".to_string());
    }
    if let CollectorRange::Days { cutoff_ms, .. } = &result.range {
        if result
            .records
            .iter()
            .any(|record| record.history_time_ms < *cutoff_ms)
        {
            return Err("range_record_out_of_bounds".to_string());
        }
    }
    Ok(())
}

fn validate_sync_started_at(sync_started_at_ms: i64) -> Result<(), String> {
    if sync_started_at_ms < EARLIEST_HISTORY_MS
        || sync_started_at_ms > MAX_SAFE_INTEGER
        || DateTime::<Utc>::from_timestamp_millis(sync_started_at_ms).is_none()
    {
        return Err("invalid_sync_started_at".to_string());
    }
    Ok(())
}

fn validate_range(range: &CollectorRange, sync_started_at_ms: i64) -> Result<(), String> {
    let CollectorRange::Days { days, cutoff_ms } = range else {
        return Ok(());
    };
    if !(1..=365).contains(days) {
        return Err("invalid_range_days".to_string());
    }
    let offset = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS)
        .ok_or_else(|| "invalid_calendar_time_zone".to_string())?;
    let sync_utc = DateTime::<Utc>::from_timestamp_millis(sync_started_at_ms)
        .ok_or_else(|| "invalid_sync_started_at".to_string())?;
    let local_date = sync_utc.with_timezone(&offset).date_naive();
    let local_midnight = offset
        .from_local_datetime(
            &local_date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| "range_cutoff_mismatch".to_string())?,
        )
        .single()
        .ok_or_else(|| "range_cutoff_mismatch".to_string())?
        .timestamp_millis();
    let expected_cutoff = local_midnight
        .checked_sub(i64::from(*days - 1) * 86_400_000)
        .ok_or_else(|| "range_cutoff_mismatch".to_string())?;
    if *cutoff_ms != expected_cutoff {
        return Err("range_cutoff_mismatch".to_string());
    }
    Ok(())
}

fn validate_records(records: &[CollectorRecordV2], sync_started_at_ms: i64) -> Result<(), String> {
    let mut identities = HashSet::new();
    for (index, record) in records.iter().enumerate() {
        if record.source_position != index as i64 {
            return Err("invalid_source_position".to_string());
        }
        if record.aweme_id.is_empty()
            || !record
                .aweme_id
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            return Err("invalid_aweme_id".to_string());
        }
        if record
            .aweme_type
            .is_some_and(|value| value.unsigned_abs() > MAX_SAFE_INTEGER as u64)
        {
            return Err("invalid_aweme_type".to_string());
        }
        validate_record_time(record, sync_started_at_ms)?;
        if index > 0 && records[index - 1].history_time_ms < record.history_time_ms {
            return Err("history_time_order_invalid".to_string());
        }
        if !identities.insert((record.aweme_id.clone(), record.history_time_ms)) {
            return Err("duplicate_history_identity".to_string());
        }
        validate_url(record)?;
        if let Some(cover_url) = &record.cover_url {
            let cover = Url::parse(cover_url).map_err(|_| "invalid_cover_url".to_string())?;
            if cover.scheme() != "http" && cover.scheme() != "https" {
                return Err("invalid_cover_url".to_string());
            }
        }
        if let Some(like_count) = record.like_count {
            if !(0..=MAX_SAFE_INTEGER).contains(&like_count) {
                return Err("invalid_like_count".to_string());
            }
        }
        if let Some(progress) = record.watch_progress_rate {
            if !(0.0..=1.0).contains(&progress) {
                return Err("invalid_watch_progress_rate".to_string());
            }
        }
    }
    Ok(())
}

fn validate_record_time(record: &CollectorRecordV2, sync_started_at_ms: i64) -> Result<(), String> {
    if record.history_time_ms < EARLIEST_HISTORY_MS
        || record.history_time_ms > sync_started_at_ms + FUTURE_TOLERANCE_MS
    {
        return Err("invalid_history_time".to_string());
    }
    let watched_utc = DateTime::<Utc>::from_timestamp_millis(record.history_time_ms)
        .ok_or_else(|| "invalid_history_time".to_string())?;
    let expected_watched_at = watched_utc.to_rfc3339_opts(SecondsFormat::Millis, true);
    if record.watched_at != expected_watched_at {
        return Err("invalid_watched_at".to_string());
    }
    let offset = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS)
        .ok_or_else(|| "invalid_calendar_time_zone".to_string())?;
    let expected_watched_date = watched_utc
        .with_timezone(&offset)
        .format("%Y-%m-%d")
        .to_string();
    if record.watched_date != expected_watched_date {
        return Err("invalid_watched_date".to_string());
    }
    Ok(())
}

fn validate_url(record: &CollectorRecordV2) -> Result<(), String> {
    let url = Url::parse(&record.canonical_url).map_err(|_| "invalid_canonical_url".to_string())?;
    let host = url
        .host_str()
        .ok_or_else(|| "invalid_canonical_url".to_string())?;
    if url.scheme() != "https"
        || host != "www.douyin.com"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid_canonical_url".to_string());
    }
    let segments: Vec<_> = url
        .path_segments()
        .ok_or_else(|| "invalid_canonical_url".to_string())?
        .collect();
    let path_type = if segments.len() == 2 && (segments[0] == "video" || segments[0] == "note") {
        if segments[1] != record.aweme_id {
            return Err("url_id_mismatch".to_string());
        }
        segments[0]
    } else {
        "unknown"
    };
    let expected_content_type = match (path_type, record.aweme_type) {
        ("video", _) => "video",
        ("note", _) => "note",
        ("unknown", Some(68)) => "note",
        ("unknown", Some(0 | 61)) => "video",
        _ => "unknown",
    };
    if record.content_type != expected_content_type {
        return Err("content_type_conflict".to_string());
    }
    Ok(())
}

fn assert_schema_v2(connection: &Connection) -> Result<(), String> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version != SCHEMA_VERSION_V2 {
        return Err(format!("unsupported_schema_version:{version}"));
    }
    assert_foreign_key_check(connection)
}

fn assert_schema_v3(connection: &Connection) -> Result<(), String> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version != SCHEMA_VERSION_V3 {
        return Err(format!("unsupported_schema_version:{version}"));
    }
    assert_schema_v3_layout(connection)
}

fn assert_schema_v4(connection: &Connection) -> Result<(), String> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version != SCHEMA_VERSION_V4 {
        return Err(format!("unsupported_schema_version:{version}"));
    }
    assert_schema_v3_layout(connection)
}

fn assert_schema_v3_layout(connection: &Connection) -> Result<(), String> {
    for table in [
        "accounts",
        "videos",
        "sync_jobs",
        "watch_records",
        "user_likes",
        "favorite_items",
        "remote_folders",
        "remote_folder_items",
        "local_folders",
        "local_folder_items",
        "local_folder_item_accounts",
    ] {
        let exists: i64 = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if exists != 1 {
            return Err(format!("schema_v3_missing_table:{table}"));
        }
    }
    for (table, columns) in [
        (
            "accounts",
            &[
                ("id", false, 1),
                ("uid", false, 0),
                ("sec_uid", false, 0),
                ("profile_key", true, 0),
                ("nickname", false, 0),
                ("avatar_url", false, 0),
                ("status", true, 0),
                ("created_at", true, 0),
                ("updated_at", true, 0),
                ("last_login_validated_at", false, 0),
            ][..],
        ),
        (
            "videos",
            &[
                ("id", false, 1),
                ("aweme_id", true, 0),
                ("canonical_url", true, 0),
                ("content_type", true, 0),
                ("title", false, 0),
                ("author_name", false, 0),
                ("cover_url", false, 0),
                ("like_count", false, 0),
                ("first_seen_at", true, 0),
                ("last_seen_at", true, 0),
            ][..],
        ),
        (
            "sync_jobs",
            &[
                ("id", false, 1),
                ("account_id", true, 0),
                ("source", true, 0),
                ("started_at", true, 0),
                ("finished_at", false, 0),
                ("status", true, 0),
                ("inserted_videos", true, 0),
                ("inserted_records", true, 0),
                ("error_code", false, 0),
                ("error_message", false, 0),
            ][..],
        ),
        (
            "watch_records",
            &[
                ("id", false, 1),
                ("account_id", true, 0),
                ("video_id", true, 0),
                ("history_time_ms", true, 0),
                ("watched_at", true, 0),
                ("watched_date", true, 0),
                ("source_position", true, 0),
                ("watch_progress_rate", false, 0),
                ("sync_job_id", false, 0),
                ("created_at", true, 0),
                ("updated_at", true, 0),
            ][..],
        ),
        (
            "user_likes",
            &[
                ("id", false, 1),
                ("account_id", true, 0),
                ("video_id", true, 0),
                ("first_seen_at", true, 0),
                ("last_seen_at", true, 0),
                ("remote_created_at_ms", false, 0),
                ("remote_removed_at", false, 0),
                ("source_position", false, 0),
            ][..],
        ),
        (
            "favorite_items",
            &[
                ("id", false, 1),
                ("account_id", true, 0),
                ("video_id", true, 0),
                ("first_seen_at", true, 0),
                ("last_seen_at", true, 0),
                ("remote_created_at_ms", false, 0),
                ("remote_removed_at", false, 0),
                ("source_position", false, 0),
            ][..],
        ),
        (
            "remote_folders",
            &[
                ("id", false, 1),
                ("account_id", true, 0),
                ("remote_folder_id", true, 0),
                ("name", true, 0),
                ("first_seen_at", true, 0),
                ("last_seen_at", true, 0),
                ("remote_removed_at", false, 0),
            ][..],
        ),
        (
            "remote_folder_items",
            &[
                ("folder_id", true, 1),
                ("video_id", true, 2),
                ("first_seen_at", true, 0),
                ("last_seen_at", true, 0),
                ("remote_removed_at", false, 0),
            ][..],
        ),
        (
            "local_folders",
            &[
                ("id", false, 1),
                ("scope", true, 0),
                ("owner_account_id", false, 0),
                ("name", true, 0),
                ("created_at", true, 0),
                ("updated_at", true, 0),
            ][..],
        ),
        (
            "local_folder_items",
            &[
                ("folder_id", true, 1),
                ("video_id", true, 2),
                ("added_at", true, 0),
            ][..],
        ),
        (
            "local_folder_item_accounts",
            &[
                ("folder_id", true, 1),
                ("video_id", true, 2),
                ("account_id", true, 3),
                ("added_at", true, 0),
            ][..],
        ),
    ] {
        assert_table_signature(connection, table, columns)?;
    }
    for (table, columns) in [
        ("accounts", &[("profile_key", "BINARY")][..]),
        ("videos", &[("aweme_id", "BINARY")][..]),
        (
            "watch_records",
            &[
                ("account_id", "BINARY"),
                ("video_id", "BINARY"),
                ("history_time_ms", "BINARY"),
            ][..],
        ),
        (
            "user_likes",
            &[("account_id", "BINARY"), ("video_id", "BINARY")][..],
        ),
        (
            "favorite_items",
            &[("account_id", "BINARY"), ("video_id", "BINARY")][..],
        ),
        (
            "remote_folders",
            &[("account_id", "BINARY"), ("remote_folder_id", "BINARY")][..],
        ),
        (
            "remote_folder_items",
            &[("folder_id", "BINARY"), ("video_id", "BINARY")][..],
        ),
        (
            "local_folder_items",
            &[("folder_id", "BINARY"), ("video_id", "BINARY")][..],
        ),
        (
            "local_folder_item_accounts",
            &[
                ("folder_id", "BINARY"),
                ("video_id", "BINARY"),
                ("account_id", "BINARY"),
            ][..],
        ),
    ] {
        assert_unique_index(connection, table, columns)?;
    }
    for (table, index, columns, predicate) in [
        (
            "accounts",
            "idx_accounts_uid",
            &[("uid", "BINARY")][..],
            "uid is not null",
        ),
        (
            "accounts",
            "idx_accounts_sec_uid",
            &[("sec_uid", "BINARY")][..],
            "sec_uid is not null",
        ),
        (
            "local_folders",
            "idx_local_folders_account_name",
            &[("owner_account_id", "BINARY"), ("name", "NOCASE")][..],
            "scope = 'account'",
        ),
        (
            "local_folders",
            "idx_local_folders_aggregate_name",
            &[("name", "NOCASE")][..],
            "scope = 'aggregate'",
        ),
    ] {
        assert_partial_unique_index(connection, table, index, columns, predicate)?;
    }
    assert_foreign_key_check(connection)
}

fn assert_table_signature(
    connection: &Connection,
    table: &str,
    expected_columns: &[(&str, bool, i64)],
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(3)? == 1,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    for (name, expected_not_null, expected_pk) in expected_columns {
        let Some((_, actual_not_null, actual_pk)) = columns
            .iter()
            .find(|(actual_name, _, _)| actual_name == name)
        else {
            return Err(format!("schema_v3_missing_column:{table}:{name}"));
        };
        if actual_not_null != expected_not_null {
            return Err(format!("schema_v3_invalid_column:{table}:{name}:notnull"));
        }
        if actual_pk != expected_pk {
            return Err(format!("schema_v3_invalid_column:{table}:{name}:pk"));
        }
    }
    for (name, _, _) in &columns {
        if !expected_columns
            .iter()
            .any(|(expected_name, _, _)| expected_name == name)
        {
            return Err(format!("schema_v3_unexpected_column:{table}:{name}"));
        }
    }
    Ok(())
}

fn assert_unique_index(
    connection: &Connection,
    table: &str,
    expected_columns: &[(&str, &str)],
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA index_list({table})"))
        .map_err(|error| error.to_string())?;
    let indexes = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? == 1,
                row.get::<_, i64>(4)? == 1,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (index, unique, partial) in indexes {
        if unique
            && !partial
            && index_signature_matches(&index_columns(connection, &index)?, expected_columns)
        {
            return Ok(());
        }
    }
    Err(format!(
        "schema_v3_missing_unique_index:{table}:{}",
        expected_columns
            .iter()
            .map(|(column, _)| *column)
            .collect::<Vec<_>>()
            .join(",")
    ))
}

fn assert_partial_unique_index(
    connection: &Connection,
    table: &str,
    index: &str,
    expected_columns: &[(&str, &str)],
    expected_predicate: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA index_list({table})"))
        .map_err(|error| error.to_string())?;
    let indexes = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? == 1,
                row.get::<_, i64>(4)? == 1,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let metadata = indexes
        .into_iter()
        .find(|(name, _, _)| name == index)
        .map(|(_, unique, partial)| (unique, partial));
    let sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [index],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let actual_predicate = sql.as_deref().and_then(normalized_where_predicate);
    let expected_predicate = normalize_sql_fragment(expected_predicate);
    if metadata != Some((true, true))
        || !index_signature_matches(&index_columns(connection, index)?, expected_columns)
        || actual_predicate.as_deref() != Some(expected_predicate.as_str())
    {
        return Err(format!("schema_v3_invalid_index:{index}"));
    }
    Ok(())
}

fn index_columns(connection: &Connection, index: &str) -> Result<Vec<(String, String)>, String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA index_xinfo({index})"))
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)? == 1,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(columns
        .into_iter()
        .filter(|(_, _, key)| *key)
        .map(|(name, collation, _)| (name.unwrap_or_default(), collation.unwrap_or_default()))
        .collect())
}

fn index_signature_matches(actual: &[(String, String)], expected: &[(&str, &str)]) -> bool {
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(
            |((actual_name, actual_collation), (expected_name, expected_collation))| {
                actual_name == expected_name && actual_collation == expected_collation
            },
        )
}

fn normalized_where_predicate(sql: &str) -> Option<String> {
    let lowercase = sql.to_ascii_lowercase();
    let bytes = lowercase.as_bytes();
    let start = lowercase.match_indices("where").find_map(|(start, _)| {
        let before_is_identifier =
            start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
        let end = start + "where".len();
        let after_is_identifier =
            end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
        (!before_is_identifier && !after_is_identifier).then_some(end)
    })?;
    Some(normalize_sql_fragment(&sql[start..]))
}

fn normalize_sql_fragment(fragment: &str) -> String {
    let mut normalized = String::new();
    let mut quoted = false;
    let mut characters = fragment.trim().trim_end_matches(';').chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\'' {
            normalized.push(character);
            if quoted && characters.peek() == Some(&'\'') {
                normalized.push(characters.next().expect("peeked quote"));
            } else {
                quoted = !quoted;
            }
        } else if quoted {
            normalized.push(character);
        } else if !character.is_ascii_whitespace() {
            normalized.push(character.to_ascii_lowercase());
        }
    }
    normalized
}

fn assert_foreign_keys_enabled(connection: &Connection) -> Result<(), String> {
    let enabled: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if enabled != 1 {
        return Err("foreign_keys_not_enabled".to_string());
    }
    Ok(())
}

fn assert_foreign_key_check(connection: &Connection) -> Result<(), String> {
    let foreign_key_errors: i64 = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if foreign_key_errors != 0 {
        return Err("foreign_key_check_failed".to_string());
    }
    Ok(())
}

fn sha256(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
