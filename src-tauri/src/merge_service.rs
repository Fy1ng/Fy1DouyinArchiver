use crate::{
    backup::{extract_backup_archive, BackupArchiveSummaryV1},
    backup_service::create_application_backup,
    bootstrap::{read_bootstrap, BootstrapV1},
    desktop_lifecycle::BackgroundTaskRegistry,
    restore_service::{commit_restore_artifacts_with_rollback, safety_backup_path},
    runtime_paths::RuntimePaths,
    schema::assert_schema_latest,
    settings_store::{
        decode_portable_settings_bytes, load_portable_settings, portable_account_key,
        PortableSettingsV2,
    },
    sync::OperationAdmission,
    AppPaths,
};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

const MERGE_PLAN_SCHEMA_VERSION: i64 = 1;
const PLAN_SNAPSHOT_TIMESTAMP: &str = "2000-01-01T00:00:00.000Z";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MergeCountsV1 {
    pub accounts: i64,
    pub videos: i64,
    pub watch_records: i64,
    pub likes: i64,
    pub favorites: i64,
    pub remote_folders: i64,
    pub local_folders: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalFolderMergeConflictV1 {
    pub id: String,
    pub scope: String,
    pub account_key: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsMergeConflictV1 {
    pub id: String,
    pub path: String,
    pub current_value: Value,
    pub backup_value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MergePlanV1 {
    pub schema_version: i64,
    pub archive: BackupArchiveSummaryV1,
    pub current_fingerprint: String,
    pub archive_fingerprint: String,
    pub plan_hash: String,
    pub counts: MergeCountsV1,
    pub local_folder_conflicts: Vec<LocalFolderMergeConflictV1>,
    pub settings_conflicts: Vec<SettingsMergeConflictV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum LocalFolderMergeResolutionV1 {
    Merge,
    Rename { name: String },
    Skip,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SettingValueSourceV1 {
    Current,
    Backup,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum SettingsMergeResolutionV1 {
    Current,
    Backup,
    PerField {
        choices: BTreeMap<String, SettingValueSourceV1>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MergeRestoreResultV1 {
    pub imported: BackupArchiveSummaryV1,
    pub merged: MergeCountsV1,
    pub safety_backup_path: PathBuf,
}

pub struct MergePlanRequestV1<'a> {
    pub runtime_paths: &'a RuntimePaths,
    pub admission: &'a OperationAdmission,
    pub background_tasks: &'a BackgroundTaskRegistry,
    pub archive_path: &'a Path,
    pub wait_timeout: Duration,
}

pub struct MergeRestoreRequestV1<'a> {
    pub runtime_paths: &'a RuntimePaths,
    pub admission: &'a OperationAdmission,
    pub background_tasks: &'a BackgroundTaskRegistry,
    pub archive_path: &'a Path,
    pub expected_plan: &'a MergePlanV1,
    pub folder_resolutions: &'a BTreeMap<String, LocalFolderMergeResolutionV1>,
    pub settings_resolution: &'a SettingsMergeResolutionV1,
    pub now: DateTime<Utc>,
    pub application_version: &'a str,
    pub wait_timeout: Duration,
}

pub fn prepare_backup_merge(request: MergePlanRequestV1<'_>) -> Result<MergePlanV1, String> {
    let _background_task = request.background_tasks.register()?;
    let _ = request.admission.snapshot()?;
    let paths = request.runtime_paths.acquire()?;
    fs::create_dir_all(&paths.temp).map_err(|error| error.to_string())?;
    let staging_root = paths
        .temp
        .join(format!("merge-plan-{}", Uuid::new_v4().simple()));
    let result = build_merge_plan(&paths, request.archive_path, &staging_root);
    let _ = fs::remove_dir_all(&staging_root);
    result
}

pub fn restore_backup_merge(
    request: MergeRestoreRequestV1<'_>,
) -> Result<MergeRestoreResultV1, String> {
    let _background_task = request.background_tasks.register()?;
    let _admission_maintenance = request
        .admission
        .begin_maintenance()?
        .wait_until_ready(request.wait_timeout)?;
    let paths = request.runtime_paths.acquire()?;
    fs::create_dir_all(&paths.temp).map_err(|error| error.to_string())?;
    let staging_root = paths
        .temp
        .join(format!("merge-{}", Uuid::new_v4().simple()));
    let prepared = prepare_merge(&request, &paths, &staging_root);
    drop(paths);
    let result = prepared.and_then(|prepared| {
        let maintenance = request.runtime_paths.begin_maintenance()?;
        let current_paths = maintenance.current_paths().clone();
        commit_restore_artifacts_with_rollback(
            &current_paths,
            &prepared.merged_database,
            &prepared.merged_settings,
            &prepared.bootstrap,
            &prepared.rollback_database,
            &prepared.rollback_settings,
            &prepared.bootstrap,
        )?;
        maintenance.commit(current_paths)?;
        Ok(MergeRestoreResultV1 {
            imported: prepared.imported_summary,
            merged: prepared.counts,
            safety_backup_path: prepared.safety_backup_path,
        })
    });
    let _ = fs::remove_dir_all(&staging_root);
    result
}

struct PreparedMerge {
    imported_summary: BackupArchiveSummaryV1,
    counts: MergeCountsV1,
    merged_database: PathBuf,
    merged_settings: PortableSettingsV2,
    rollback_database: PathBuf,
    rollback_settings: PortableSettingsV2,
    bootstrap: BootstrapV1,
    safety_backup_path: PathBuf,
}

fn prepare_merge(
    request: &MergeRestoreRequestV1<'_>,
    paths: &AppPaths,
    staging_root: &Path,
) -> Result<PreparedMerge, String> {
    let plan = build_merge_plan(
        paths,
        request.archive_path,
        &staging_root.join("validation"),
    )?;
    if plan.plan_hash != request.expected_plan.plan_hash
        || plan.current_fingerprint != request.expected_plan.current_fingerprint
        || plan.archive_fingerprint != request.expected_plan.archive_fingerprint
    {
        return Err("merge_plan_stale".to_string());
    }
    validate_folder_resolutions(&plan, request.folder_resolutions)?;
    validate_settings_resolution(&plan, request.settings_resolution)?;

    let imported = extract_backup_archive(request.archive_path, &staging_root.join("import"))?;
    let imported_settings = decode_portable_settings_bytes(&imported.settings_bytes)?;
    let current_settings = load_portable_settings(&paths.data_root)?;
    let current_connection =
        Connection::open(&paths.database).map_err(|error| error.to_string())?;
    let imported_connection =
        Connection::open(&imported.database_path).map_err(|error| error.to_string())?;
    let account_keys = canonical_account_keys(&current_connection, &imported_connection)?;
    let normalized_current_settings =
        normalize_settings_account_keys(&current_settings, &account_keys.current_aliases)?;
    let normalized_imported_settings =
        normalize_settings_account_keys(&imported_settings, &account_keys.imported_aliases)?;
    let merged_settings = merge_portable_settings(
        &normalized_current_settings,
        &normalized_imported_settings,
        request.settings_resolution,
    )?;
    let bootstrap =
        read_bootstrap(&paths.bootstrap_dir)?.ok_or_else(|| "bootstrap_missing".to_string())?;

    let safety_backup_path = safety_backup_path(&paths.backups, request.now);
    create_application_backup(
        &paths.database,
        &current_settings,
        &safety_backup_path,
        &request.now.to_rfc3339_opts(SecondsFormat::Millis, true),
        request.application_version,
    )?;
    let rollback = extract_backup_archive(&safety_backup_path, &staging_root.join("rollback"))?;
    let rollback_settings = decode_portable_settings_bytes(&rollback.settings_bytes)?;
    let merged_database = staging_root.join("merged.sqlite");
    fs::copy(&rollback.database_path, &merged_database).map_err(|error| error.to_string())?;
    merge_database(
        &merged_database,
        &imported.database_path,
        request.folder_resolutions,
    )?;

    Ok(PreparedMerge {
        imported_summary: imported.summary,
        counts: plan.counts,
        merged_database,
        merged_settings,
        rollback_database: rollback.database_path,
        rollback_settings,
        bootstrap,
        safety_backup_path,
    })
}

fn build_merge_plan(
    paths: &AppPaths,
    archive_path: &Path,
    staging_root: &Path,
) -> Result<MergePlanV1, String> {
    fs::create_dir_all(staging_root).map_err(|error| error.to_string())?;
    let imported = extract_backup_archive(archive_path, &staging_root.join("import"))?;
    let imported_settings = decode_portable_settings_bytes(&imported.settings_bytes)?;
    let current_settings = load_portable_settings(&paths.data_root)?;
    let current_connection =
        Connection::open(&paths.database).map_err(|error| error.to_string())?;
    assert_schema_latest(&current_connection)?;
    let imported_connection =
        Connection::open(&imported.database_path).map_err(|error| error.to_string())?;
    assert_schema_latest(&imported_connection)?;
    let account_keys = canonical_account_keys(&current_connection, &imported_connection)?;
    let normalized_current_settings =
        normalize_settings_account_keys(&current_settings, &account_keys.current_aliases)?;
    let normalized_imported_settings =
        normalize_settings_account_keys(&imported_settings, &account_keys.imported_aliases)?;

    let current_fingerprint = current_state_fingerprint(paths, &current_settings, staging_root)?;
    let archive_fingerprint = sha256_file(archive_path)?;
    let counts = imported_counts(&imported_connection)?;
    let local_folder_conflicts =
        local_folder_conflicts(&current_connection, &imported_connection, &account_keys)?;
    let settings_conflicts =
        settings_conflicts(&normalized_current_settings, &normalized_imported_settings)?;
    let plan_hash = hash_serializable(&(
        MERGE_PLAN_SCHEMA_VERSION,
        &current_fingerprint,
        &archive_fingerprint,
        &counts,
        &local_folder_conflicts,
        &settings_conflicts,
    ))?;
    Ok(MergePlanV1 {
        schema_version: MERGE_PLAN_SCHEMA_VERSION,
        archive: imported.summary,
        current_fingerprint,
        archive_fingerprint,
        plan_hash,
        counts,
        local_folder_conflicts,
        settings_conflicts,
    })
}

fn current_state_fingerprint(
    paths: &AppPaths,
    settings: &PortableSettingsV2,
    staging_root: &Path,
) -> Result<String, String> {
    let snapshot = staging_root.join("current-state.emily-backup");
    create_application_backup(
        &paths.database,
        settings,
        &snapshot,
        PLAN_SNAPSHOT_TIMESTAMP,
        "merge-plan",
    )?;
    let result = sha256_file(&snapshot);
    let _ = fs::remove_file(snapshot);
    result
}

fn imported_counts(connection: &Connection) -> Result<MergeCountsV1, String> {
    Ok(MergeCountsV1 {
        accounts: table_count(connection, "accounts")?,
        videos: table_count(connection, "videos")?,
        watch_records: table_count(connection, "watch_records")?,
        likes: table_count(connection, "user_likes")?,
        favorites: table_count(connection, "favorite_items")?,
        remote_folders: table_count(connection, "remote_folders")?,
        local_folders: table_count(connection, "local_folders")?,
    })
}

fn table_count(connection: &Connection, table: &str) -> Result<i64, String> {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone)]
struct AccountIdentityRow {
    id: i64,
    uid: Option<String>,
    sec_uid: Option<String>,
}

#[derive(Debug)]
struct CanonicalAccountKeys {
    current_by_id: BTreeMap<i64, String>,
    imported_by_id: BTreeMap<i64, String>,
    current_aliases: BTreeMap<String, String>,
    imported_aliases: BTreeMap<String, String>,
}

fn canonical_account_keys(
    current: &Connection,
    imported: &Connection,
) -> Result<CanonicalAccountKeys, String> {
    let current_rows = read_account_identities(current)?;
    let imported_rows = read_account_identities(imported)?;
    let current_uid = identity_index(&current_rows, |row| row.uid.as_deref())?;
    let current_sec_uid = identity_index(&current_rows, |row| row.sec_uid.as_deref())?;
    let current_by_row = current_rows
        .iter()
        .map(|row| (row.id, row))
        .collect::<BTreeMap<_, _>>();
    let mut current_by_id = BTreeMap::new();
    let mut imported_by_id = BTreeMap::new();
    let mut imported_for_current = BTreeMap::new();

    for imported_row in &imported_rows {
        let mut matches = BTreeSet::new();
        if let Some(uid) = imported_row.uid.as_deref() {
            if let Some(current_id) = current_uid.get(uid) {
                matches.insert(*current_id);
            }
        }
        if let Some(sec_uid) = imported_row.sec_uid.as_deref() {
            if let Some(current_id) = current_sec_uid.get(sec_uid) {
                matches.insert(*current_id);
            }
        }
        if matches.len() > 1 {
            return Err("merge_account_identity_conflict".to_string());
        }
        let canonical = if let Some(current_id) = matches.into_iter().next() {
            if imported_for_current
                .insert(current_id, imported_row.id)
                .is_some()
            {
                return Err("merge_account_identity_conflict".to_string());
            }
            let current_row = current_by_row
                .get(&current_id)
                .ok_or_else(|| "merge_account_identity_conflict".to_string())?;
            if option_values_conflict(current_row.uid.as_deref(), imported_row.uid.as_deref())
                || option_values_conflict(
                    current_row.sec_uid.as_deref(),
                    imported_row.sec_uid.as_deref(),
                )
            {
                return Err("merge_account_identity_conflict".to_string());
            }
            let canonical = portable_account_key(
                current_row.uid.as_deref().or(imported_row.uid.as_deref()),
                current_row
                    .sec_uid
                    .as_deref()
                    .or(imported_row.sec_uid.as_deref()),
            )?;
            current_by_id.insert(current_id, canonical.clone());
            canonical
        } else {
            portable_account_key(imported_row.uid.as_deref(), imported_row.sec_uid.as_deref())?
        };
        imported_by_id.insert(imported_row.id, canonical);
    }
    for current_row in &current_rows {
        current_by_id
            .entry(current_row.id)
            .or_insert(portable_account_key(
                current_row.uid.as_deref(),
                current_row.sec_uid.as_deref(),
            )?);
    }

    let current_aliases = identity_aliases(&current_rows, &current_by_id)?;
    let imported_aliases = identity_aliases(&imported_rows, &imported_by_id)?;
    Ok(CanonicalAccountKeys {
        current_by_id,
        imported_by_id,
        current_aliases,
        imported_aliases,
    })
}

fn read_account_identities(connection: &Connection) -> Result<Vec<AccountIdentityRow>, String> {
    let mut statement = connection
        .prepare("SELECT id, uid, sec_uid FROM accounts ORDER BY id")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(AccountIdentityRow {
                id: row.get(0)?,
                uid: row.get(1)?,
                sec_uid: row.get(2)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn identity_index<'a>(
    rows: &'a [AccountIdentityRow],
    value: impl Fn(&'a AccountIdentityRow) -> Option<&'a str>,
) -> Result<BTreeMap<String, i64>, String> {
    let mut index = BTreeMap::new();
    for row in rows {
        let Some(value) = value(row) else {
            continue;
        };
        if index.insert(value.to_string(), row.id).is_some() {
            return Err("merge_account_identity_conflict".to_string());
        }
    }
    Ok(index)
}

fn option_values_conflict(current: Option<&str>, imported: Option<&str>) -> bool {
    matches!((current, imported), (Some(current), Some(imported)) if current != imported)
}

fn identity_aliases(
    rows: &[AccountIdentityRow],
    canonical_by_id: &BTreeMap<i64, String>,
) -> Result<BTreeMap<String, String>, String> {
    let mut aliases = BTreeMap::new();
    for row in rows {
        let canonical = canonical_by_id
            .get(&row.id)
            .ok_or_else(|| "merge_account_identity_conflict".to_string())?;
        for alias in [
            row.uid
                .as_deref()
                .map(|uid| portable_account_key(Some(uid), None))
                .transpose()?,
            row.sec_uid
                .as_deref()
                .map(|sec_uid| portable_account_key(None, Some(sec_uid)))
                .transpose()?,
        ]
        .into_iter()
        .flatten()
        {
            if aliases
                .insert(alias, canonical.clone())
                .is_some_and(|previous| previous != *canonical)
            {
                return Err("merge_account_identity_conflict".to_string());
            }
        }
    }
    Ok(aliases)
}

fn normalize_settings_account_keys(
    settings: &PortableSettingsV2,
    aliases: &BTreeMap<String, String>,
) -> Result<PortableSettingsV2, String> {
    let mut normalized = settings.clone();
    let accounts = std::mem::take(&mut normalized.accounts);
    for (key, value) in accounts {
        let canonical = aliases.get(&key).cloned().unwrap_or(key);
        if normalized
            .accounts
            .insert(canonical, value.clone())
            .is_some_and(|existing| existing != value)
        {
            return Err("merge_settings_account_alias_conflict".to_string());
        }
    }
    Ok(normalized)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FolderIdentity {
    scope: String,
    account_key: Option<String>,
    normalized_name: String,
}

fn local_folder_conflicts(
    current: &Connection,
    imported: &Connection,
    account_keys: &CanonicalAccountKeys,
) -> Result<Vec<LocalFolderMergeConflictV1>, String> {
    let current_folders = read_folder_identities(current, &account_keys.current_by_id)?;
    let current_keys = current_folders
        .into_iter()
        .map(|(identity, _)| identity)
        .collect::<BTreeSet<_>>();
    let mut conflicts = read_folder_identities(imported, &account_keys.imported_by_id)?
        .into_iter()
        .filter_map(|(identity, name)| {
            current_keys.contains(&identity).then(|| {
                let id = folder_conflict_id(&identity);
                LocalFolderMergeConflictV1 {
                    id,
                    scope: identity.scope,
                    account_key: identity.account_key,
                    name,
                }
            })
        })
        .collect::<Vec<_>>();
    conflicts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(conflicts)
}

fn read_folder_identities(
    connection: &Connection,
    account_keys: &BTreeMap<i64, String>,
) -> Result<Vec<(FolderIdentity, String)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT lf.scope, lf.name, lf.owner_account_id
             FROM local_folders lf
             ORDER BY lf.id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let scope = row.get::<_, String>(0)?;
            let name = row.get::<_, String>(1)?;
            let owner_account_id = row.get::<_, Option<i64>>(2)?;
            Ok((scope, name, owner_account_id))
        })
        .map_err(|error| error.to_string())?
        .map(|row| {
            let (scope, name, owner_account_id) = row.map_err(|error| error.to_string())?;
            let account_key = if scope == "account" {
                Some(
                    account_keys
                        .get(
                            &owner_account_id
                                .ok_or_else(|| "merge_folder_owner_missing".to_string())?,
                        )
                        .cloned()
                        .ok_or_else(|| "merge_folder_owner_missing".to_string())?,
                )
            } else {
                None
            };
            Ok((
                FolderIdentity {
                    scope,
                    account_key,
                    normalized_name: name.to_lowercase(),
                },
                name,
            ))
        })
        .collect();
    rows
}

fn folder_conflict_id(identity: &FolderIdentity) -> String {
    hash_bytes(
        format!(
            "folder\0{}\0{}\0{}",
            identity.scope,
            identity.account_key.as_deref().unwrap_or(""),
            identity.normalized_name
        )
        .as_bytes(),
    )
}

fn settings_conflicts(
    current: &PortableSettingsV2,
    backup: &PortableSettingsV2,
) -> Result<Vec<SettingsMergeConflictV1>, String> {
    let current = serde_json::to_value(current).map_err(|error| error.to_string())?;
    let backup = serde_json::to_value(backup).map_err(|error| error.to_string())?;
    let mut conflicts = Vec::new();
    collect_direct_settings_conflicts(
        "global",
        current
            .get("global")
            .and_then(Value::as_object)
            .ok_or_else(|| "merge_settings_invalid".to_string())?,
        backup
            .get("global")
            .and_then(Value::as_object)
            .ok_or_else(|| "merge_settings_invalid".to_string())?,
        &mut conflicts,
    )?;
    collect_direct_settings_conflicts(
        "accountDefaults",
        current
            .get("accountDefaults")
            .and_then(Value::as_object)
            .ok_or_else(|| "merge_settings_invalid".to_string())?,
        backup
            .get("accountDefaults")
            .and_then(Value::as_object)
            .ok_or_else(|| "merge_settings_invalid".to_string())?,
        &mut conflicts,
    )?;
    let current_accounts = current
        .get("accounts")
        .and_then(Value::as_object)
        .ok_or_else(|| "merge_settings_invalid".to_string())?;
    let backup_accounts = backup
        .get("accounts")
        .and_then(Value::as_object)
        .ok_or_else(|| "merge_settings_invalid".to_string())?;
    for (account_key, backup_account) in backup_accounts {
        let Some(current_account) = current_accounts.get(account_key) else {
            continue;
        };
        collect_direct_settings_conflicts(
            &format!("accounts.{account_key}"),
            current_account
                .as_object()
                .ok_or_else(|| "merge_settings_invalid".to_string())?,
            backup_account
                .as_object()
                .ok_or_else(|| "merge_settings_invalid".to_string())?,
            &mut conflicts,
        )?;
    }
    conflicts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(conflicts)
}

fn collect_direct_settings_conflicts(
    path: &str,
    current: &serde_json::Map<String, Value>,
    backup: &serde_json::Map<String, Value>,
    conflicts: &mut Vec<SettingsMergeConflictV1>,
) -> Result<(), String> {
    for (key, backup_value) in backup {
        let Some(current_value) = current.get(key) else {
            continue;
        };
        if current_value != backup_value {
            let field_path = format!("{path}.{key}");
            let id = setting_conflict_id(&field_path, current_value, backup_value)?;
            conflicts.push(SettingsMergeConflictV1 {
                id,
                path: field_path,
                current_value: current_value.clone(),
                backup_value: backup_value.clone(),
            });
        }
    }
    Ok(())
}

fn setting_conflict_id(path: &str, current: &Value, backup: &Value) -> Result<String, String> {
    hash_serializable(&("setting", path, current, backup))
}

fn validate_folder_resolutions(
    plan: &MergePlanV1,
    resolutions: &BTreeMap<String, LocalFolderMergeResolutionV1>,
) -> Result<(), String> {
    let expected = plan
        .local_folder_conflicts
        .iter()
        .map(|conflict| conflict.id.as_str())
        .collect::<BTreeSet<_>>();
    if expected.iter().any(|id| !resolutions.contains_key(*id)) {
        return Err("merge_folder_resolution_missing".to_string());
    }
    if resolutions.keys().any(|id| !expected.contains(id.as_str())) {
        return Err("merge_folder_resolution_unknown".to_string());
    }
    for resolution in resolutions.values() {
        if let LocalFolderMergeResolutionV1::Rename { name } = resolution {
            validate_folder_name(name)?;
        }
    }
    Ok(())
}

fn validate_settings_resolution(
    plan: &MergePlanV1,
    resolution: &SettingsMergeResolutionV1,
) -> Result<(), String> {
    let SettingsMergeResolutionV1::PerField { choices } = resolution else {
        return Ok(());
    };
    let expected = plan
        .settings_conflicts
        .iter()
        .map(|conflict| conflict.id.as_str())
        .collect::<BTreeSet<_>>();
    if expected.iter().any(|id| !choices.contains_key(*id)) {
        return Err("merge_settings_resolution_missing".to_string());
    }
    if choices.keys().any(|id| !expected.contains(id.as_str())) {
        return Err("merge_settings_resolution_unknown".to_string());
    }
    Ok(())
}

fn merge_portable_settings(
    current: &PortableSettingsV2,
    backup: &PortableSettingsV2,
    resolution: &SettingsMergeResolutionV1,
) -> Result<PortableSettingsV2, String> {
    let mut merged = serde_json::to_value(current).map_err(|error| error.to_string())?;
    let backup_value = serde_json::to_value(backup).map_err(|error| error.to_string())?;
    merge_direct_settings_fields(
        "global",
        merged
            .get_mut("global")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "merge_settings_invalid".to_string())?,
        backup_value
            .get("global")
            .and_then(Value::as_object)
            .ok_or_else(|| "merge_settings_invalid".to_string())?,
        resolution,
    )?;
    merge_direct_settings_fields(
        "accountDefaults",
        merged
            .get_mut("accountDefaults")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "merge_settings_invalid".to_string())?,
        backup_value
            .get("accountDefaults")
            .and_then(Value::as_object)
            .ok_or_else(|| "merge_settings_invalid".to_string())?,
        resolution,
    )?;
    let backup_accounts = backup_value
        .get("accounts")
        .and_then(Value::as_object)
        .ok_or_else(|| "merge_settings_invalid".to_string())?;
    let merged_accounts = merged
        .get_mut("accounts")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "merge_settings_invalid".to_string())?;
    for (account_key, backup_account) in backup_accounts {
        if let Some(current_account) = merged_accounts.get_mut(account_key) {
            merge_direct_settings_fields(
                &format!("accounts.{account_key}"),
                current_account
                    .as_object_mut()
                    .ok_or_else(|| "merge_settings_invalid".to_string())?,
                backup_account
                    .as_object()
                    .ok_or_else(|| "merge_settings_invalid".to_string())?,
                resolution,
            )?;
        } else {
            merged_accounts.insert(account_key.clone(), backup_account.clone());
        }
    }
    merged["revision"] = Value::from(current.revision.max(backup.revision).saturating_add(1));
    let bytes = serde_json::to_vec(&merged).map_err(|error| error.to_string())?;
    decode_portable_settings_bytes(&bytes)
}

fn merge_direct_settings_fields(
    path: &str,
    current: &mut serde_json::Map<String, Value>,
    backup: &serde_json::Map<String, Value>,
    resolution: &SettingsMergeResolutionV1,
) -> Result<(), String> {
    for (key, backup_value) in backup {
        let field_path = format!("{path}.{key}");
        if let Some(current_value) = current.get_mut(key) {
            if current_value == backup_value {
                continue;
            }
            let id = setting_conflict_id(&field_path, current_value, backup_value)?;
            let source = match resolution {
                SettingsMergeResolutionV1::Current => SettingValueSourceV1::Current,
                SettingsMergeResolutionV1::Backup => SettingValueSourceV1::Backup,
                SettingsMergeResolutionV1::PerField { choices } => *choices
                    .get(&id)
                    .ok_or_else(|| "merge_settings_resolution_missing".to_string())?,
            };
            if source == SettingValueSourceV1::Backup {
                *current_value = backup_value.clone();
            }
        } else {
            current.insert(key.clone(), backup_value.clone());
        }
    }
    Ok(())
}

fn merge_database(
    target_path: &Path,
    imported_path: &Path,
    folder_resolutions: &BTreeMap<String, LocalFolderMergeResolutionV1>,
) -> Result<(), String> {
    let mut connection = Connection::open(target_path).map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| error.to_string())?;
    assert_schema_latest(&connection)?;
    connection
        .execute(
            "ATTACH DATABASE ?1 AS imported",
            [imported_path.to_string_lossy().as_ref()],
        )
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    create_mapping_tables(&transaction)?;
    merge_accounts(&transaction)?;
    merge_videos(&transaction)?;
    merge_watch_records(&transaction)?;
    merge_archive_relations(&transaction)?;
    merge_remote_folders(&transaction)?;
    merge_local_folders(&transaction, folder_resolutions)?;
    merge_cover_cache_rules(&transaction)?;
    transaction.commit().map_err(|error| error.to_string())?;
    assert_schema_latest(&connection)?;
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

fn create_mapping_tables(transaction: &Transaction<'_>) -> Result<(), String> {
    transaction
        .execute_batch(
            "CREATE TEMP TABLE merge_account_map(
               imported_id INTEGER PRIMARY KEY,
               target_id INTEGER NOT NULL
             );
             CREATE TEMP TABLE merge_video_map(
               imported_id INTEGER PRIMARY KEY,
               target_id INTEGER NOT NULL
             );
             CREATE TEMP TABLE merge_remote_folder_map(
               imported_id INTEGER PRIMARY KEY,
               target_id INTEGER NOT NULL
             );
             CREATE TEMP TABLE merge_local_folder_map(
               imported_id INTEGER PRIMARY KEY,
               target_id INTEGER NOT NULL
             );",
        )
        .map_err(|error| error.to_string())
}

#[derive(Debug)]
struct ImportedAccount {
    id: i64,
    uid: Option<String>,
    sec_uid: Option<String>,
    nickname: Option<String>,
    avatar_url: Option<String>,
    status: String,
    created_at: String,
    updated_at: String,
}

fn merge_accounts(transaction: &Transaction<'_>) -> Result<(), String> {
    let accounts = {
        let mut statement = transaction
            .prepare(
                "SELECT id, uid, sec_uid, nickname, avatar_url, status, created_at, updated_at
                 FROM imported.accounts ORDER BY id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(ImportedAccount {
                    id: row.get(0)?,
                    uid: row.get(1)?,
                    sec_uid: row.get(2)?,
                    nickname: row.get(3)?,
                    avatar_url: row.get(4)?,
                    status: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for account in accounts {
        portable_account_key(account.uid.as_deref(), account.sec_uid.as_deref())?;
        let uid_match = account
            .uid
            .as_deref()
            .map(|uid| account_id_by_field(transaction, "uid", uid))
            .transpose()?
            .flatten();
        let sec_match = account
            .sec_uid
            .as_deref()
            .map(|sec_uid| account_id_by_field(transaction, "sec_uid", sec_uid))
            .transpose()?
            .flatten();
        let target_id = match (uid_match, sec_match) {
            (Some(uid_id), Some(sec_id)) if uid_id != sec_id => {
                return Err("merge_account_identity_conflict".to_string())
            }
            (Some(id), _) | (_, Some(id)) => {
                transaction
                    .execute(
                        "UPDATE accounts
                         SET uid = COALESCE(uid, ?2),
                             sec_uid = COALESCE(sec_uid, ?3),
                             nickname = CASE
                               WHEN ?4 IS NULL OR trim(?4) = '' THEN nickname
                               WHEN nickname IS NULL OR trim(nickname) = '' OR ?7 >= updated_at THEN ?4
                               ELSE nickname END,
                             avatar_url = CASE
                               WHEN ?5 IS NULL OR trim(?5) = '' THEN avatar_url
                               WHEN avatar_url IS NULL OR trim(avatar_url) = '' OR ?7 >= updated_at THEN ?5
                               ELSE avatar_url END,
                             created_at = MIN(created_at, ?6),
                             updated_at = MAX(updated_at, ?7)
                         WHERE id = ?1",
                        params![
                            id,
                            account.uid,
                            account.sec_uid,
                            account.nickname,
                            account.avatar_url,
                            account.created_at,
                            account.updated_at
                        ],
                    )
                    .map_err(|error| error.to_string())?;
                id
            }
            (None, None) => {
                let profile_key = Uuid::new_v4().simple().to_string();
                let status = if account.status == "archived" {
                    "archived"
                } else {
                    "login_expired"
                };
                transaction
                    .execute(
                        "INSERT INTO accounts
                         (uid, sec_uid, profile_key, nickname, avatar_url, status,
                          created_at, updated_at, last_login_validated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
                        params![
                            account.uid,
                            account.sec_uid,
                            profile_key,
                            account.nickname,
                            account.avatar_url,
                            status,
                            account.created_at,
                            account.updated_at
                        ],
                    )
                    .map_err(|error| error.to_string())?;
                let id = transaction.last_insert_rowid();
                transaction
                    .execute(
                        "INSERT INTO remote_folder_registry_state(account_id, revision)
                         VALUES (?1, 1)",
                        [id],
                    )
                    .map_err(|error| error.to_string())?;
                id
            }
        };
        transaction
            .execute(
                "INSERT INTO merge_account_map(imported_id, target_id) VALUES (?1, ?2)",
                params![account.id, target_id],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn account_id_by_field(
    transaction: &Transaction<'_>,
    field: &str,
    value: &str,
) -> Result<Option<i64>, String> {
    transaction
        .query_row(
            &format!("SELECT id FROM accounts WHERE {field} = ?1"),
            [value],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn merge_videos(transaction: &Transaction<'_>) -> Result<(), String> {
    transaction
        .execute_batch(
            "INSERT INTO videos
             (aweme_id, canonical_url, content_type, title, author_name, cover_url,
              like_count, first_seen_at, last_seen_at)
             SELECT aweme_id, canonical_url, content_type, title, author_name, cover_url,
                    like_count, first_seen_at, last_seen_at
             FROM imported.videos WHERE 1
             ON CONFLICT(aweme_id) DO UPDATE SET
               canonical_url = CASE WHEN excluded.last_seen_at >= videos.last_seen_at
                                    THEN excluded.canonical_url ELSE videos.canonical_url END,
               content_type = CASE WHEN excluded.last_seen_at >= videos.last_seen_at
                                   THEN excluded.content_type ELSE videos.content_type END,
               title = CASE
                 WHEN excluded.title IS NULL OR trim(excluded.title) = '' THEN videos.title
                 WHEN videos.title IS NULL OR trim(videos.title) = '' OR excluded.last_seen_at >= videos.last_seen_at
                   THEN excluded.title ELSE videos.title END,
               author_name = CASE
                 WHEN excluded.author_name IS NULL OR trim(excluded.author_name) = '' THEN videos.author_name
                 WHEN videos.author_name IS NULL OR trim(videos.author_name) = '' OR excluded.last_seen_at >= videos.last_seen_at
                   THEN excluded.author_name ELSE videos.author_name END,
               cover_url = CASE
                 WHEN excluded.cover_url IS NULL OR trim(excluded.cover_url) = '' THEN videos.cover_url
                 WHEN videos.cover_url IS NULL OR trim(videos.cover_url) = '' OR excluded.last_seen_at >= videos.last_seen_at
                   THEN excluded.cover_url ELSE videos.cover_url END,
               like_count = CASE
                 WHEN excluded.like_count IS NULL THEN videos.like_count
                 WHEN videos.like_count IS NULL OR excluded.last_seen_at >= videos.last_seen_at
                   THEN excluded.like_count ELSE videos.like_count END,
               first_seen_at = MIN(videos.first_seen_at, excluded.first_seen_at),
               last_seen_at = MAX(videos.last_seen_at, excluded.last_seen_at);
             INSERT INTO merge_video_map(imported_id, target_id)
             SELECT imported_video.id, target_video.id
             FROM imported.videos imported_video
             JOIN videos target_video ON target_video.aweme_id = imported_video.aweme_id;",
        )
        .map_err(|error| error.to_string())
}

fn merge_watch_records(transaction: &Transaction<'_>) -> Result<(), String> {
    transaction
        .execute_batch(
            "INSERT INTO watch_records
             (account_id, video_id, history_time_ms, watched_at, watched_date,
              source_position, watch_progress_rate, sync_job_id, created_at, updated_at)
             SELECT account_map.target_id, video_map.target_id, record.history_time_ms,
                    record.watched_at, record.watched_date, record.source_position,
                    record.watch_progress_rate, NULL, record.created_at, record.updated_at
             FROM imported.watch_records record
             JOIN merge_account_map account_map ON account_map.imported_id = record.account_id
             JOIN merge_video_map video_map ON video_map.imported_id = record.video_id
             WHERE 1
             ON CONFLICT(account_id, video_id, history_time_ms) DO UPDATE SET
               watched_at = CASE WHEN excluded.updated_at >= watch_records.updated_at
                                 THEN excluded.watched_at ELSE watch_records.watched_at END,
               watched_date = CASE WHEN excluded.updated_at >= watch_records.updated_at
                                   THEN excluded.watched_date ELSE watch_records.watched_date END,
               source_position = CASE WHEN excluded.updated_at >= watch_records.updated_at
                                      THEN excluded.source_position ELSE watch_records.source_position END,
               watch_progress_rate = CASE
                 WHEN excluded.watch_progress_rate IS NULL THEN watch_records.watch_progress_rate
                 WHEN watch_records.watch_progress_rate IS NULL OR excluded.updated_at >= watch_records.updated_at
                   THEN excluded.watch_progress_rate ELSE watch_records.watch_progress_rate END,
               created_at = MIN(watch_records.created_at, excluded.created_at),
               updated_at = MAX(watch_records.updated_at, excluded.updated_at);",
        )
        .map_err(|error| error.to_string())
}

fn merge_archive_relations(transaction: &Transaction<'_>) -> Result<(), String> {
    for table in ["user_likes", "favorite_items"] {
        transaction
            .execute_batch(&format!(
                "INSERT INTO {table}
                 (account_id, video_id, first_seen_at, last_seen_at, remote_created_at_ms,
                  remote_removed_at, source_position)
                 SELECT account_map.target_id, video_map.target_id, item.first_seen_at,
                        item.last_seen_at, item.remote_created_at_ms, item.remote_removed_at,
                        item.source_position
                 FROM imported.{table} item
                 JOIN merge_account_map account_map ON account_map.imported_id = item.account_id
                 JOIN merge_video_map video_map ON video_map.imported_id = item.video_id
                 WHERE 1
                 ON CONFLICT(account_id, video_id) DO UPDATE SET
                   first_seen_at = MIN({table}.first_seen_at, excluded.first_seen_at),
                   last_seen_at = MAX({table}.last_seen_at, excluded.last_seen_at),
                   remote_created_at_ms = CASE
                     WHEN {table}.remote_created_at_ms IS NULL THEN excluded.remote_created_at_ms
                     WHEN excluded.remote_created_at_ms IS NULL THEN {table}.remote_created_at_ms
                     ELSE MIN({table}.remote_created_at_ms, excluded.remote_created_at_ms) END,
                   remote_removed_at = CASE WHEN excluded.last_seen_at >= {table}.last_seen_at
                                            THEN excluded.remote_removed_at ELSE {table}.remote_removed_at END,
                   source_position = CASE WHEN excluded.last_seen_at >= {table}.last_seen_at
                                          THEN excluded.source_position ELSE {table}.source_position END;"
            ))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn merge_remote_folders(transaction: &Transaction<'_>) -> Result<(), String> {
    transaction
        .execute_batch(
            "INSERT INTO remote_folders
             (account_id, remote_folder_id, name, first_seen_at, last_seen_at, remote_removed_at)
             SELECT account_map.target_id, folder.remote_folder_id, folder.name,
                    folder.first_seen_at, folder.last_seen_at, folder.remote_removed_at
             FROM imported.remote_folders folder
             JOIN merge_account_map account_map ON account_map.imported_id = folder.account_id
             WHERE 1
             ON CONFLICT(account_id, remote_folder_id) DO UPDATE SET
               name = CASE WHEN excluded.last_seen_at >= remote_folders.last_seen_at
                           THEN excluded.name ELSE remote_folders.name END,
               first_seen_at = MIN(remote_folders.first_seen_at, excluded.first_seen_at),
               remote_removed_at = CASE WHEN excluded.last_seen_at >= remote_folders.last_seen_at
                                        THEN excluded.remote_removed_at ELSE remote_folders.remote_removed_at END,
               last_seen_at = MAX(remote_folders.last_seen_at, excluded.last_seen_at);
             INSERT INTO merge_remote_folder_map(imported_id, target_id)
             SELECT source.id, target.id
             FROM imported.remote_folders source
             JOIN merge_account_map account_map ON account_map.imported_id = source.account_id
             JOIN remote_folders target
               ON target.account_id = account_map.target_id
              AND target.remote_folder_id = source.remote_folder_id;
             INSERT INTO remote_folder_items
             (folder_id, video_id, first_seen_at, last_seen_at, remote_removed_at)
             SELECT folder_map.target_id, video_map.target_id, item.first_seen_at,
                    item.last_seen_at, item.remote_removed_at
             FROM imported.remote_folder_items item
             JOIN merge_remote_folder_map folder_map ON folder_map.imported_id = item.folder_id
             JOIN merge_video_map video_map ON video_map.imported_id = item.video_id
             WHERE 1
             ON CONFLICT(folder_id, video_id) DO UPDATE SET
               first_seen_at = MIN(remote_folder_items.first_seen_at, excluded.first_seen_at),
               remote_removed_at = CASE WHEN excluded.last_seen_at >= remote_folder_items.last_seen_at
                                        THEN excluded.remote_removed_at ELSE remote_folder_items.remote_removed_at END,
               last_seen_at = MAX(remote_folder_items.last_seen_at, excluded.last_seen_at);",
        )
        .map_err(|error| error.to_string())
}

#[derive(Debug)]
struct ImportedLocalFolder {
    id: i64,
    scope: String,
    owner_account_id: Option<i64>,
    name: String,
    created_at: String,
    updated_at: String,
}

fn merge_local_folders(
    transaction: &Transaction<'_>,
    resolutions: &BTreeMap<String, LocalFolderMergeResolutionV1>,
) -> Result<(), String> {
    let folders = {
        let mut statement = transaction
            .prepare(
                "SELECT id, scope, owner_account_id, name, created_at, updated_at
                 FROM imported.local_folders ORDER BY id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(ImportedLocalFolder {
                    id: row.get(0)?,
                    scope: row.get(1)?,
                    owner_account_id: row.get(2)?,
                    name: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for folder in folders {
        let owner_id = folder
            .owner_account_id
            .map(|id| mapped_id(transaction, "merge_account_map", id))
            .transpose()?;
        let account_key = owner_id
            .map(|id| account_portable_key(transaction, id))
            .transpose()?;
        let identity = FolderIdentity {
            scope: folder.scope.clone(),
            account_key,
            normalized_name: folder.name.to_lowercase(),
        };
        let existing = find_local_folder(transaction, &folder.scope, owner_id, &folder.name)?;
        let target_id = if let Some(existing_id) = existing {
            let conflict_id = folder_conflict_id(&identity);
            match resolutions
                .get(&conflict_id)
                .ok_or_else(|| "merge_folder_resolution_missing".to_string())?
            {
                LocalFolderMergeResolutionV1::Merge => {
                    transaction
                        .execute(
                            "UPDATE local_folders
                             SET created_at = MIN(created_at, ?2),
                                 updated_at = MAX(updated_at, ?3)
                             WHERE id = ?1",
                            params![existing_id, folder.created_at, folder.updated_at],
                        )
                        .map_err(|error| error.to_string())?;
                    Some(existing_id)
                }
                LocalFolderMergeResolutionV1::Rename { name } => {
                    validate_folder_name(name)?;
                    if find_local_folder(transaction, &folder.scope, owner_id, name)?.is_some() {
                        return Err("merge_folder_rename_conflict".to_string());
                    }
                    Some(insert_local_folder(transaction, &folder, owner_id, name)?)
                }
                LocalFolderMergeResolutionV1::Skip => None,
            }
        } else {
            Some(insert_local_folder(
                transaction,
                &folder,
                owner_id,
                &folder.name,
            )?)
        };
        if let Some(target_id) = target_id {
            transaction
                .execute(
                    "INSERT INTO merge_local_folder_map(imported_id, target_id) VALUES (?1, ?2)",
                    params![folder.id, target_id],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    transaction
        .execute_batch(
            "INSERT INTO local_folder_items(folder_id, video_id, added_at)
             SELECT folder_map.target_id, video_map.target_id, item.added_at
             FROM imported.local_folder_items item
             JOIN merge_local_folder_map folder_map ON folder_map.imported_id = item.folder_id
             JOIN merge_video_map video_map ON video_map.imported_id = item.video_id
             WHERE 1
             ON CONFLICT(folder_id, video_id) DO UPDATE SET
               added_at = MIN(local_folder_items.added_at, excluded.added_at);
             INSERT INTO local_folder_item_accounts(folder_id, video_id, account_id, added_at)
             SELECT folder_map.target_id, video_map.target_id, account_map.target_id, item.added_at
             FROM imported.local_folder_item_accounts item
             JOIN merge_local_folder_map folder_map ON folder_map.imported_id = item.folder_id
             JOIN merge_video_map video_map ON video_map.imported_id = item.video_id
             JOIN merge_account_map account_map ON account_map.imported_id = item.account_id
             WHERE 1
             ON CONFLICT(folder_id, video_id, account_id) DO UPDATE SET
               added_at = MIN(local_folder_item_accounts.added_at, excluded.added_at);",
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn mapped_id(transaction: &Transaction<'_>, table: &str, imported_id: i64) -> Result<i64, String> {
    transaction
        .query_row(
            &format!("SELECT target_id FROM {table} WHERE imported_id = ?1"),
            [imported_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn account_portable_key(transaction: &Transaction<'_>, account_id: i64) -> Result<String, String> {
    let (uid, sec_uid) = transaction
        .query_row(
            "SELECT uid, sec_uid FROM accounts WHERE id = ?1",
            [account_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    portable_account_key(uid.as_deref(), sec_uid.as_deref())
}

fn find_local_folder(
    transaction: &Transaction<'_>,
    scope: &str,
    owner_id: Option<i64>,
    name: &str,
) -> Result<Option<i64>, String> {
    if scope == "aggregate" {
        transaction
            .query_row(
                "SELECT id FROM local_folders
                 WHERE scope = 'aggregate' AND name = ?1 COLLATE NOCASE",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    } else {
        transaction
            .query_row(
                "SELECT id FROM local_folders
                 WHERE scope = 'account' AND owner_account_id = ?1
                   AND name = ?2 COLLATE NOCASE",
                params![owner_id, name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }
}

fn insert_local_folder(
    transaction: &Transaction<'_>,
    folder: &ImportedLocalFolder,
    owner_id: Option<i64>,
    name: &str,
) -> Result<i64, String> {
    transaction
        .execute(
            "INSERT INTO local_folders
             (scope, owner_account_id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                folder.scope,
                owner_id,
                name,
                folder.created_at,
                folder.updated_at
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(transaction.last_insert_rowid())
}

fn validate_folder_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 128 {
        return Err("merge_folder_name_invalid".to_string());
    }
    Ok(())
}

fn merge_cover_cache_rules(transaction: &Transaction<'_>) -> Result<(), String> {
    transaction
        .execute_batch(
            "INSERT INTO cover_cache_rules(video_id, created_at)
             SELECT video_map.target_id, rule.created_at
             FROM imported.cover_cache_rules rule
             JOIN merge_video_map video_map ON video_map.imported_id = rule.video_id
             WHERE 1
             ON CONFLICT(video_id) DO UPDATE SET
               created_at = MIN(cover_cache_rules.created_at, excluded.created_at);",
        )
        .map_err(|error| error.to_string())
}

fn hash_serializable(value: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(hash_bytes(&bytes))
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
