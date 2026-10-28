use crate::{collector::file_store::write_json_atomically, settings_store::PortableSettingsV2};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

const DEVICE_SETTINGS_SCHEMA_VERSION_V1: i64 = 1;
const DEVICE_SETTINGS_SCHEMA_VERSION_V2: i64 = 2;
const DEVICE_SETTINGS_SCHEMA_VERSION_V3: i64 = 3;
const DEVICE_SETTINGS_SCHEMA_VERSION: i64 = 4;
const DEVICE_SETTINGS_FILE: &str = "device-settings.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrowserPreferenceV1 {
    Auto,
    Edge,
    Chrome,
}

impl BrowserPreferenceV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Edge => "edge",
            Self::Chrome => "chrome",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceAccountSettingsV1 {
    pub browser_preference: BrowserPreferenceV1,
}

impl Default for DeviceAccountSettingsV1 {
    fn default() -> Self {
        Self {
            browser_preference: BrowserPreferenceV1::Auto,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomaticBackupStateV1 {
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceSettingsV1 {
    pub schema_version: i64,
    pub revision: u64,
    pub onboarding_completed: bool,
    pub onboarding_step: u8,
    pub launch_at_startup: bool,
    pub silent_autostart: bool,
    pub scheduler_paused: bool,
    pub first_tray_hint_shown: bool,
    pub notified_failure_streaks: BTreeMap<String, i64>,
    pub automatic_backup: AutomaticBackupStateV1,
    pub backup_directory: Option<PathBuf>,
    pub accounts: BTreeMap<String, DeviceAccountSettingsV1>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeviceSettingsSchemaV1 {
    schema_version: i64,
    revision: u64,
    onboarding_completed: bool,
    onboarding_step: u8,
    launch_at_startup: bool,
    silent_autostart: bool,
    scheduler_paused: bool,
    first_tray_hint_shown: bool,
    notified_failure_streaks: BTreeMap<String, i64>,
    accounts: BTreeMap<String, DeviceAccountSettingsV1>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeviceSettingsSchemaV2 {
    schema_version: i64,
    revision: u64,
    onboarding_completed: bool,
    onboarding_step: u8,
    launch_at_startup: bool,
    silent_autostart: bool,
    scheduler_paused: bool,
    first_tray_hint_shown: bool,
    notified_failure_streaks: BTreeMap<String, i64>,
    backup_directory: Option<PathBuf>,
    accounts: BTreeMap<String, DeviceAccountSettingsV1>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeviceSettingsSchemaV3 {
    schema_version: i64,
    revision: u64,
    onboarding_completed: bool,
    onboarding_step: u8,
    launch_at_startup: bool,
    silent_autostart: bool,
    scheduler_paused: bool,
    first_tray_hint_shown: bool,
    notified_failure_streaks: BTreeMap<String, i64>,
    automatic_backup: AutomaticBackupStateV1,
    backup_directory: Option<PathBuf>,
    accounts: BTreeMap<String, DeviceAccountSettingsV1>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSettingsVersionEnvelope {
    schema_version: i64,
}

impl From<DeviceSettingsSchemaV1> for DeviceSettingsV1 {
    fn from(value: DeviceSettingsSchemaV1) -> Self {
        let onboarding_step = migrate_onboarding_step(
            value.onboarding_completed,
            value.onboarding_step,
        );
        Self {
            schema_version: DEVICE_SETTINGS_SCHEMA_VERSION,
            revision: value.revision,
            onboarding_completed: value.onboarding_completed,
            onboarding_step,
            launch_at_startup: value.launch_at_startup,
            silent_autostart: value.silent_autostart,
            scheduler_paused: value.scheduler_paused,
            first_tray_hint_shown: value.first_tray_hint_shown,
            notified_failure_streaks: value.notified_failure_streaks,
            automatic_backup: AutomaticBackupStateV1::default(),
            backup_directory: None,
            accounts: value.accounts,
        }
    }
}

impl From<DeviceSettingsSchemaV2> for DeviceSettingsV1 {
    fn from(value: DeviceSettingsSchemaV2) -> Self {
        let onboarding_step = migrate_onboarding_step(
            value.onboarding_completed,
            value.onboarding_step,
        );
        Self {
            schema_version: DEVICE_SETTINGS_SCHEMA_VERSION,
            revision: value.revision,
            onboarding_completed: value.onboarding_completed,
            onboarding_step,
            launch_at_startup: value.launch_at_startup,
            silent_autostart: value.silent_autostart,
            scheduler_paused: value.scheduler_paused,
            first_tray_hint_shown: value.first_tray_hint_shown,
            notified_failure_streaks: value.notified_failure_streaks,
            automatic_backup: AutomaticBackupStateV1::default(),
            backup_directory: value.backup_directory,
            accounts: value.accounts,
        }
    }
}

impl From<DeviceSettingsSchemaV3> for DeviceSettingsV1 {
    fn from(value: DeviceSettingsSchemaV3) -> Self {
        let onboarding_step = migrate_onboarding_step(
            value.onboarding_completed,
            value.onboarding_step,
        );
        Self {
            schema_version: DEVICE_SETTINGS_SCHEMA_VERSION,
            revision: value.revision,
            onboarding_completed: value.onboarding_completed,
            onboarding_step,
            launch_at_startup: value.launch_at_startup,
            silent_autostart: value.silent_autostart,
            scheduler_paused: value.scheduler_paused,
            first_tray_hint_shown: value.first_tray_hint_shown,
            notified_failure_streaks: value.notified_failure_streaks,
            automatic_backup: value.automatic_backup,
            backup_directory: value.backup_directory,
            accounts: value.accounts,
        }
    }
}

fn migrate_onboarding_step(completed: bool, step: u8) -> u8 {
    if completed {
        return 3;
    }
    match step {
        0 => 0,
        1 | 2 => 1,
        3 => 2,
        other => other,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceSettingsViewV1 {
    pub revision: u64,
    pub onboarding_completed: bool,
    pub onboarding_step: u8,
    pub launch_at_startup: bool,
    pub silent_autostart: bool,
    pub scheduler_paused: bool,
    pub backup_directory: Option<PathBuf>,
    pub accounts: BTreeMap<String, DeviceAccountSettingsV1>,
}

impl From<DeviceSettingsV1> for DeviceSettingsViewV1 {
    fn from(value: DeviceSettingsV1) -> Self {
        Self {
            revision: value.revision,
            onboarding_completed: value.onboarding_completed,
            onboarding_step: value.onboarding_step,
            launch_at_startup: value.launch_at_startup,
            silent_autostart: value.silent_autostart,
            scheduler_paused: value.scheduler_paused,
            backup_directory: value.backup_directory,
            accounts: value.accounts,
        }
    }
}

impl Default for DeviceSettingsV1 {
    fn default() -> Self {
        Self {
            schema_version: DEVICE_SETTINGS_SCHEMA_VERSION,
            revision: 0,
            onboarding_completed: false,
            onboarding_step: 0,
            launch_at_startup: false,
            silent_autostart: false,
            scheduler_paused: false,
            first_tray_hint_shown: false,
            notified_failure_streaks: BTreeMap::new(),
            automatic_backup: AutomaticBackupStateV1::default(),
            backup_directory: None,
            accounts: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceSettingsStore {
    path: PathBuf,
    rmw: Arc<Mutex<()>>,
}

impl DeviceSettingsStore {
    pub fn open(system_app_data: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&system_app_data).map_err(|error| error.to_string())?;
        Ok(Self {
            path: system_app_data.join(DEVICE_SETTINGS_FILE),
            rmw: Arc::new(Mutex::new(())),
        })
    }

    pub fn load(&self) -> Result<DeviceSettingsV1, String> {
        let _guard = self.lock()?;
        Ok(self.load_optional_locked()?.unwrap_or_default())
    }

    pub fn load_optional(&self) -> Result<Option<DeviceSettingsV1>, String> {
        let _guard = self.lock()?;
        self.load_optional_locked()
    }

    pub fn compare_and_swap(
        &self,
        expected_revision: u64,
        update: impl FnOnce(&mut DeviceSettingsV1) -> Result<(), String>,
    ) -> Result<DeviceSettingsV1, String> {
        let _guard = self.lock()?;
        let current = self.load_optional_locked()?.unwrap_or_default();
        if current.revision != expected_revision {
            return Err("device_settings_revision_conflict".to_string());
        }

        let mut next = current.clone();
        update(&mut next)?;
        if next.revision != current.revision {
            return Err("invalid_device_settings".to_string());
        }
        next.revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| "device_settings_revision_overflow".to_string())?;
        validate_device_settings(&next)?;
        write_json_atomically(&self.path, &next)?;
        Ok(next)
    }

    pub fn mark_first_tray_hint_shown(&self) -> Result<bool, String> {
        let _guard = self.lock()?;
        let current = self.load_optional_locked()?.unwrap_or_default();
        if current.first_tray_hint_shown {
            return Ok(false);
        }
        let next = update_revision(current, |settings| {
            settings.first_tray_hint_shown = true;
            Ok(())
        })?;
        write_json_atomically(&self.path, &next)?;
        Ok(true)
    }

    pub fn set_scheduler_paused(&self, paused: bool) -> Result<DeviceSettingsV1, String> {
        self.update(|settings| {
            settings.scheduler_paused = paused;
            Ok(())
        })
    }

    pub fn set_account_browser_preference(
        &self,
        profile_key: &str,
        browser_preference: BrowserPreferenceV1,
    ) -> Result<DeviceSettingsV1, String> {
        if !valid_simple_uuid(profile_key) {
            return Err("invalid_profile_key".to_string());
        }
        self.update(|settings| {
            settings
                .accounts
                .entry(profile_key.to_string())
                .or_default()
                .browser_preference = browser_preference;
            Ok(())
        })
    }

    pub fn set_notified_failure(
        &self,
        key: &str,
        job_id: Option<i64>,
    ) -> Result<DeviceSettingsV1, String> {
        if job_id.is_some_and(|value| !valid_notification_state_entry(key, value))
            || (job_id.is_none() && !valid_notification_key(key))
        {
            return Err("invalid_notification_key".to_string());
        }
        self.update(|settings| {
            match job_id {
                Some(job_id) => {
                    settings
                        .notified_failure_streaks
                        .insert(key.to_string(), job_id);
                }
                None => {
                    settings.notified_failure_streaks.remove(key);
                }
            }
            Ok(())
        })
    }

    pub fn set_backup_directory(
        &self,
        backup_directory: Option<PathBuf>,
    ) -> Result<DeviceSettingsV1, String> {
        if let Some(path) = backup_directory.as_deref() {
            validate_backup_directory_path(path)?;
        }
        self.update(move |settings| {
            settings.backup_directory = backup_directory;
            Ok(())
        })
    }

    pub fn record_automatic_backup_attempt(
        &self,
        attempted_at: &str,
    ) -> Result<DeviceSettingsV1, String> {
        let attempted_at = attempted_at.to_string();
        self.update(move |settings| {
            settings.automatic_backup.last_attempt_at = Some(attempted_at);
            settings.automatic_backup.last_failure_at = None;
            settings.automatic_backup.last_error_code = None;
            Ok(())
        })
    }

    pub fn record_automatic_backup_success(
        &self,
        succeeded_at: &str,
    ) -> Result<DeviceSettingsV1, String> {
        let succeeded_at = succeeded_at.to_string();
        self.update(move |settings| {
            settings.automatic_backup.last_attempt_at = Some(succeeded_at.clone());
            settings.automatic_backup.last_success_at = Some(succeeded_at);
            settings.automatic_backup.last_failure_at = None;
            settings.automatic_backup.last_error_code = None;
            Ok(())
        })
    }

    pub fn record_automatic_backup_failure(
        &self,
        failed_at: &str,
        error_code: &str,
    ) -> Result<DeviceSettingsV1, String> {
        let failed_at = failed_at.to_string();
        let error_code = error_code.to_string();
        self.update(move |settings| {
            settings.automatic_backup.last_attempt_at = Some(failed_at.clone());
            settings.automatic_backup.last_failure_at = Some(failed_at);
            settings.automatic_backup.last_error_code = Some(error_code);
            Ok(())
        })
    }

    fn update(
        &self,
        update: impl FnOnce(&mut DeviceSettingsV1) -> Result<(), String>,
    ) -> Result<DeviceSettingsV1, String> {
        let _guard = self.lock()?;
        let current = self.load_optional_locked()?.unwrap_or_default();
        let next = update_revision(current, update)?;
        write_json_atomically(&self.path, &next)?;
        Ok(next)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        self.rmw
            .lock()
            .map_err(|_| "device_settings_lock_poisoned".to_string())
    }

    fn load_optional_locked(&self) -> Result<Option<DeviceSettingsV1>, String> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let version = serde_json::from_slice::<DeviceSettingsVersionEnvelope>(&bytes)
            .map_err(|_| "invalid_device_settings".to_string())?
            .schema_version;
        match version {
            DEVICE_SETTINGS_SCHEMA_VERSION_V1 => {
                let legacy = serde_json::from_slice::<DeviceSettingsSchemaV1>(&bytes)
                    .map_err(|_| "invalid_device_settings".to_string())?;
                validate_device_settings_v1(&legacy)?;
                let migrated = DeviceSettingsV1::from(legacy);
                validate_device_settings(&migrated)?;
                write_json_atomically(&self.path, &migrated)?;
                Ok(Some(migrated))
            }
            DEVICE_SETTINGS_SCHEMA_VERSION_V2 => {
                let legacy = serde_json::from_slice::<DeviceSettingsSchemaV2>(&bytes)
                    .map_err(|_| "invalid_device_settings".to_string())?;
                validate_device_settings_v2(&legacy)?;
                let migrated = DeviceSettingsV1::from(legacy);
                validate_device_settings(&migrated)?;
                write_json_atomically(&self.path, &migrated)?;
                Ok(Some(migrated))
            }
            DEVICE_SETTINGS_SCHEMA_VERSION_V3 => {
                let legacy = serde_json::from_slice::<DeviceSettingsSchemaV3>(&bytes)
                    .map_err(|_| "invalid_device_settings".to_string())?;
                validate_device_settings_v3(&legacy)?;
                let migrated = DeviceSettingsV1::from(legacy);
                validate_device_settings(&migrated)?;
                write_json_atomically(&self.path, &migrated)?;
                Ok(Some(migrated))
            }
            DEVICE_SETTINGS_SCHEMA_VERSION => {
                let value = serde_json::from_slice::<DeviceSettingsV1>(&bytes)
                    .map_err(|_| "invalid_device_settings".to_string())?;
                validate_device_settings(&value)?;
                Ok(Some(value))
            }
            _ => Err("unsupported_device_settings_schema".to_string()),
        }
    }
}

fn update_revision(
    current: DeviceSettingsV1,
    update: impl FnOnce(&mut DeviceSettingsV1) -> Result<(), String>,
) -> Result<DeviceSettingsV1, String> {
    let mut next = current;
    let revision = next.revision;
    update(&mut next)?;
    if next.revision != revision {
        return Err("invalid_device_settings".to_string());
    }
    next.revision = revision
        .checked_add(1)
        .ok_or_else(|| "device_settings_revision_overflow".to_string())?;
    validate_device_settings(&next)?;
    Ok(next)
}

pub fn initialize_onboarding_state(
    connection: &Connection,
    portable: &PortableSettingsV2,
) -> Result<DeviceSettingsV1, String> {
    let prior_database_use: i64 = connection
        .query_row(
            "SELECT
               EXISTS(SELECT 1 FROM accounts)
               OR EXISTS(SELECT 1 FROM watch_records)
               OR EXISTS(SELECT 1 FROM sync_jobs WHERE finished_at IS NOT NULL)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let mut settings = DeviceSettingsV1::default();
    if prior_database_use == 1 || portable.revision > 0 {
        settings.onboarding_completed = true;
        settings.onboarding_step = 3;
    }
    Ok(settings)
}

pub fn initialize_device_settings_store(
    system_app_data: PathBuf,
    connection: &Connection,
    portable: &PortableSettingsV2,
) -> Result<DeviceSettingsStore, String> {
    let store = DeviceSettingsStore::open(system_app_data)?;
    if store.load_optional()?.is_none() {
        let initial = initialize_onboarding_state(connection, portable)?;
        store.compare_and_swap(0, move |settings| {
            *settings = initial;
            Ok(())
        })?;
    }
    Ok(store)
}

pub fn notification_failure_key(
    account_id: i64,
    source: &str,
    source_instance_key: &str,
) -> Result<String, String> {
    if account_id <= 0 || !valid_notification_source(source) {
        return Err("invalid_notification_key".to_string());
    }
    let hash = format!("{:x}", Sha256::digest(source_instance_key.as_bytes()));
    Ok(format!("{account_id}:{source}:{hash}"))
}

fn validate_device_settings(value: &DeviceSettingsV1) -> Result<(), String> {
    if value.schema_version != DEVICE_SETTINGS_SCHEMA_VERSION
        || value.onboarding_step > 3
        || (value.onboarding_completed && value.onboarding_step != 3)
        || (value.silent_autostart && !value.launch_at_startup)
    {
        return Err("invalid_device_settings".to_string());
    }
    for (key, job_id) in &value.notified_failure_streaks {
        if !valid_notification_state_entry(key, *job_id) {
            return Err("invalid_device_settings".to_string());
        }
    }
    if let Some(path) = value.backup_directory.as_deref() {
        validate_backup_directory_path(path)?;
    }
    validate_automatic_backup_state(&value.automatic_backup)?;
    for profile_key in value.accounts.keys() {
        if !valid_simple_uuid(profile_key) {
            return Err("invalid_device_settings".to_string());
        }
    }
    Ok(())
}

fn validate_device_settings_v2(value: &DeviceSettingsSchemaV2) -> Result<(), String> {
    if value.schema_version != DEVICE_SETTINGS_SCHEMA_VERSION_V2
        || value.onboarding_step > 3
        || (value.onboarding_completed && value.onboarding_step != 3)
        || (value.silent_autostart && !value.launch_at_startup)
    {
        return Err("invalid_device_settings".to_string());
    }
    for (key, job_id) in &value.notified_failure_streaks {
        if !valid_notification_state_entry(key, *job_id) {
            return Err("invalid_device_settings".to_string());
        }
    }
    if let Some(path) = value.backup_directory.as_deref() {
        validate_backup_directory_path(path)?;
    }
    for profile_key in value.accounts.keys() {
        if !valid_simple_uuid(profile_key) {
            return Err("invalid_device_settings".to_string());
        }
    }
    Ok(())
}

fn validate_device_settings_v3(value: &DeviceSettingsSchemaV3) -> Result<(), String> {
    if value.schema_version != DEVICE_SETTINGS_SCHEMA_VERSION_V3
        || value.onboarding_step > 3
        || (value.onboarding_completed && value.onboarding_step != 3)
        || (value.silent_autostart && !value.launch_at_startup)
    {
        return Err("invalid_device_settings".to_string());
    }
    for (key, job_id) in &value.notified_failure_streaks {
        if !valid_notification_state_entry(key, *job_id) {
            return Err("invalid_device_settings".to_string());
        }
    }
    if let Some(path) = value.backup_directory.as_deref() {
        validate_backup_directory_path(path)?;
    }
    validate_automatic_backup_state(&value.automatic_backup)?;
    for profile_key in value.accounts.keys() {
        if !valid_simple_uuid(profile_key) {
            return Err("invalid_device_settings".to_string());
        }
    }
    Ok(())
}

fn validate_automatic_backup_state(value: &AutomaticBackupStateV1) -> Result<(), String> {
    let parse = |value: Option<&str>| -> Result<Option<DateTime<Utc>>, String> {
        value
            .map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .map(|value| value.with_timezone(&Utc))
                    .map_err(|_| "invalid_device_settings".to_string())
            })
            .transpose()
    };
    let attempt = parse(value.last_attempt_at.as_deref())?;
    let success = parse(value.last_success_at.as_deref())?;
    let failure = parse(value.last_failure_at.as_deref())?;
    if success.is_some_and(|success| attempt.is_none_or(|attempt| success > attempt))
        || failure.is_some_and(|failure| attempt != Some(failure))
        || value.last_failure_at.is_some() != value.last_error_code.is_some()
        || value
            .last_error_code
            .as_deref()
            .is_some_and(|code| !valid_automatic_backup_error_code(code))
    {
        return Err("invalid_device_settings".to_string());
    }
    Ok(())
}

fn valid_automatic_backup_error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
}

fn validate_device_settings_v1(value: &DeviceSettingsSchemaV1) -> Result<(), String> {
    if value.schema_version != DEVICE_SETTINGS_SCHEMA_VERSION_V1
        || value.onboarding_step > 3
        || (value.onboarding_completed && value.onboarding_step != 3)
        || (value.silent_autostart && !value.launch_at_startup)
    {
        return Err("invalid_device_settings".to_string());
    }
    for (key, job_id) in &value.notified_failure_streaks {
        if !valid_notification_state_entry(key, *job_id) {
            return Err("invalid_device_settings".to_string());
        }
    }
    for profile_key in value.accounts.keys() {
        if !valid_simple_uuid(profile_key) {
            return Err("invalid_device_settings".to_string());
        }
    }
    Ok(())
}

pub fn validate_backup_directory_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err("invalid_backup_directory".to_string());
    }
    if path.parent().is_none() {
        return Err("backup_directory_cannot_be_root".to_string());
    }
    Ok(())
}

fn valid_notification_state_entry(key: &str, job_id: i64) -> bool {
    job_id > 0 && valid_notification_key(key)
}

fn valid_notification_key(key: &str) -> bool {
    let mut parts = key.split(':');
    let account = parts.next().unwrap_or_default();
    let source = parts.next().unwrap_or_default();
    let hash = parts.next().unwrap_or_default();
    parts.next().is_none()
        && account
            .parse::<i64>()
            .ok()
            .is_some_and(|value| value > 0 && value.to_string() == account)
        && valid_notification_source(source)
        && valid_sha256(hash)
}

fn valid_notification_source(value: &str) -> bool {
    matches!(value, "history" | "likes" | "favorites" | "remote_folder")
}

fn valid_simple_uuid(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
