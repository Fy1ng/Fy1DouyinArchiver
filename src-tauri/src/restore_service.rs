use crate::{
    accounts::portable_account_key_for_id,
    backup::{extract_backup_archive, BackupArchiveSummaryV1},
    backup_service::create_application_backup,
    bootstrap::{read_bootstrap, write_bootstrap_atomic, BootstrapV1},
    desktop_lifecycle::BackgroundTaskRegistry,
    runtime_paths::RuntimePaths,
    schema::assert_schema_latest,
    settings_store::{
        decode_portable_settings_bytes, load_portable_settings, portable_account_key,
        write_portable_settings_atomic, PortableSettingsV2,
    },
    sync::OperationAdmission,
    AppPaths,
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceRestoreResultV1 {
    pub imported: BackupArchiveSummaryV1,
    pub safety_backup_path: PathBuf,
    pub active_account_id: Option<i64>,
}

pub struct ReplaceRestoreRequestV1<'a> {
    pub runtime_paths: &'a RuntimePaths,
    pub admission: &'a OperationAdmission,
    pub background_tasks: &'a BackgroundTaskRegistry,
    pub archive_path: &'a Path,
    pub now: DateTime<Utc>,
    pub application_version: &'a str,
    pub wait_timeout: Duration,
}

pub struct InspectRestoreRequestV1<'a> {
    pub runtime_paths: &'a RuntimePaths,
    pub background_tasks: &'a BackgroundTaskRegistry,
    pub archive_path: &'a Path,
}

struct PreparedReplace {
    imported_summary: BackupArchiveSummaryV1,
    imported_database: PathBuf,
    imported_settings: PortableSettingsV2,
    rollback_database: PathBuf,
    rollback_settings: PortableSettingsV2,
    original_bootstrap: BootstrapV1,
    restored_bootstrap: BootstrapV1,
    safety_backup_path: PathBuf,
    active_account_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreCommitStage {
    AfterDatabase,
    AfterSettings,
    AfterBootstrap,
}

pub fn inspect_backup_for_restore(
    request: InspectRestoreRequestV1<'_>,
) -> Result<BackupArchiveSummaryV1, String> {
    let _background_task = request.background_tasks.register()?;
    let paths = request.runtime_paths.acquire()?;
    fs::create_dir_all(&paths.temp).map_err(|error| error.to_string())?;
    let staging_root = paths
        .temp
        .join(format!("restore-inspect-{}", Uuid::new_v4().simple()));
    let result = (|| {
        let imported = extract_backup_archive(request.archive_path, &staging_root)?;
        decode_portable_settings_bytes(&imported.settings_bytes)?;
        Ok(imported.summary)
    })();
    let _ = fs::remove_dir_all(&staging_root);
    result
}

pub fn restore_backup_replace(
    request: ReplaceRestoreRequestV1<'_>,
) -> Result<ReplaceRestoreResultV1, String> {
    let _background_task = request.background_tasks.register()?;
    let _admission_maintenance = request
        .admission
        .begin_maintenance()?
        .wait_until_ready(request.wait_timeout)?;
    let paths = request.runtime_paths.acquire()?;
    fs::create_dir_all(&paths.temp).map_err(|error| error.to_string())?;
    let staging_root = paths
        .temp
        .join(format!("restore-{}", Uuid::new_v4().simple()));
    let prepared = prepare_replace(&request, &paths, &staging_root);
    drop(paths);
    let result = prepared.and_then(|prepared| {
        let maintenance = request.runtime_paths.begin_maintenance()?;
        let current_paths = maintenance.current_paths().clone();
        commit_replace_with_rollback(
            &current_paths,
            &prepared.imported_database,
            &prepared.imported_settings,
            &prepared.restored_bootstrap,
            &prepared.rollback_database,
            &prepared.rollback_settings,
            &prepared.original_bootstrap,
            |_| Ok(()),
        )?;
        maintenance.commit(current_paths)?;
        Ok(ReplaceRestoreResultV1 {
            imported: prepared.imported_summary,
            safety_backup_path: prepared.safety_backup_path,
            active_account_id: prepared.active_account_id,
        })
    });
    let _ = fs::remove_dir_all(&staging_root);
    result
}

fn prepare_replace(
    request: &ReplaceRestoreRequestV1<'_>,
    paths: &AppPaths,
    staging_root: &Path,
) -> Result<PreparedReplace, String> {
    let imported = extract_backup_archive(request.archive_path, &staging_root.join("import"))?;
    let imported_settings = decode_portable_settings_bytes(&imported.settings_bytes)?;
    let original_bootstrap =
        read_bootstrap(&paths.bootstrap_dir)?.ok_or_else(|| "bootstrap_missing".to_string())?;
    let current_settings = load_portable_settings(&paths.data_root)?;
    let current_active_key = current_active_account_key(paths, &original_bootstrap)?;
    let active_account_id = prepare_imported_database(
        paths,
        &imported.database_path,
        current_active_key.as_deref(),
        request.now,
    )?;

    let safety_backup_path = safety_backup_path(&paths.backups, request.now);
    create_application_backup(
        &paths.database,
        &current_settings,
        &safety_backup_path,
        &request
            .now
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        request.application_version,
    )?;
    let rollback = extract_backup_archive(&safety_backup_path, &staging_root.join("rollback"))?;
    let rollback_settings = decode_portable_settings_bytes(&rollback.settings_bytes)?;
    let mut restored_bootstrap = original_bootstrap.clone();
    restored_bootstrap.active_account_id = active_account_id;
    Ok(PreparedReplace {
        imported_summary: imported.summary,
        imported_database: imported.database_path,
        imported_settings,
        rollback_database: rollback.database_path,
        rollback_settings,
        original_bootstrap,
        restored_bootstrap,
        safety_backup_path,
        active_account_id,
    })
}

fn current_active_account_key(
    paths: &AppPaths,
    bootstrap: &BootstrapV1,
) -> Result<Option<String>, String> {
    let Some(account_id) = bootstrap.active_account_id else {
        return Ok(None);
    };
    let connection = Connection::open(&paths.database).map_err(|error| error.to_string())?;
    portable_account_key_for_id(&connection, account_id)
        .map(Some)
        .or_else(|error| (error == "account_not_found").then_some(None).ok_or(error))
}

fn prepare_imported_database(
    current_paths: &AppPaths,
    database_path: &Path,
    current_active_key: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Option<i64>, String> {
    let mut connection = Connection::open(database_path).map_err(|error| error.to_string())?;
    assert_schema_latest(&connection)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let timestamp = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    transaction
        .execute("DELETE FROM cover_cache_queue", [])
        .map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM cover_cache", [])
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE cover_cache_tasks
             SET status = 'cancelled', finished_at = ?1,
                 error_code = COALESCE(error_code, 'restore_cancelled_cache_task')
             WHERE status IN ('queued', 'running', 'paused')",
            [&timestamp],
        )
        .map_err(|error| error.to_string())?;

    let mut statement = transaction
        .prepare("SELECT id, profile_key FROM accounts WHERE status = 'active' ORDER BY id")
        .map_err(|error| error.to_string())?;
    let profiles = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    for (account_id, profile_key) in profiles {
        let profile_exists = current_paths
            .account_root(&profile_key)
            .is_ok_and(|path| path.is_dir());
        if !profile_exists {
            transaction
                .execute(
                    "UPDATE accounts
                     SET status = 'login_expired', updated_at = ?2
                     WHERE id = ?1 AND status = 'active'",
                    rusqlite::params![account_id, timestamp],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    let active_account_id = match current_active_key {
        Some(key) => imported_account_id_for_key(&transaction, key)?,
        None => None,
    };
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(active_account_id)
}

fn imported_account_id_for_key(
    connection: &Connection,
    expected: &str,
) -> Result<Option<i64>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, uid, sec_uid
             FROM accounts
             WHERE status IN ('active', 'login_expired', 'identity_conflict')
             ORDER BY id",
        )
        .map_err(|error| error.to_string())?;
    let accounts = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for account in accounts {
        let (account_id, uid, sec_uid) = account.map_err(|error| error.to_string())?;
        if portable_account_key(uid.as_deref(), sec_uid.as_deref())
            .ok()
            .as_deref()
            == Some(expected)
        {
            return Ok(Some(account_id));
        }
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn commit_replace_with_rollback(
    paths: &AppPaths,
    imported_database: &Path,
    imported_settings: &PortableSettingsV2,
    restored_bootstrap: &BootstrapV1,
    rollback_database: &Path,
    rollback_settings: &PortableSettingsV2,
    original_bootstrap: &BootstrapV1,
    mut hook: impl FnMut(RestoreCommitStage) -> Result<(), String>,
) -> Result<(), String> {
    let commit = commit_replace_with_hook(
        paths,
        imported_database,
        imported_settings,
        restored_bootstrap,
        &mut hook,
    );
    if let Err(error) = commit {
        rollback_replace(
            paths,
            rollback_database,
            rollback_settings,
            original_bootstrap,
        )?;
        return Err(error);
    }
    Ok(())
}

pub(crate) fn commit_restore_artifacts_with_rollback(
    paths: &AppPaths,
    imported_database: &Path,
    imported_settings: &PortableSettingsV2,
    restored_bootstrap: &BootstrapV1,
    rollback_database: &Path,
    rollback_settings: &PortableSettingsV2,
    original_bootstrap: &BootstrapV1,
) -> Result<(), String> {
    commit_replace_with_rollback(
        paths,
        imported_database,
        imported_settings,
        restored_bootstrap,
        rollback_database,
        rollback_settings,
        original_bootstrap,
        |_| Ok(()),
    )
}

fn commit_replace_with_hook(
    paths: &AppPaths,
    imported_database: &Path,
    imported_settings: &PortableSettingsV2,
    bootstrap: &BootstrapV1,
    hook: &mut impl FnMut(RestoreCommitStage) -> Result<(), String>,
) -> Result<(), String> {
    remove_database_sidecars(&paths.database);
    replace_file(imported_database, &paths.database)?;
    hook(RestoreCommitStage::AfterDatabase)?;
    write_portable_settings_atomic(&paths.data_root, imported_settings)?;
    hook(RestoreCommitStage::AfterSettings)?;
    write_bootstrap_atomic(&paths.bootstrap_dir, bootstrap)?;
    hook(RestoreCommitStage::AfterBootstrap)?;
    validate_committed_restore(paths)
}

fn rollback_replace(
    paths: &AppPaths,
    rollback_database: &Path,
    rollback_settings: &PortableSettingsV2,
    bootstrap: &BootstrapV1,
) -> Result<(), String> {
    let result = (|| {
        remove_database_sidecars(&paths.database);
        replace_file(rollback_database, &paths.database)?;
        write_portable_settings_atomic(&paths.data_root, rollback_settings)?;
        write_bootstrap_atomic(&paths.bootstrap_dir, bootstrap)?;
        validate_committed_restore(paths)
    })();
    result.map_err(|_| "restore_rollback_failed".to_string())
}

fn validate_committed_restore(paths: &AppPaths) -> Result<(), String> {
    let connection = Connection::open(&paths.database).map_err(|error| error.to_string())?;
    assert_schema_latest(&connection)?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if integrity != "ok" {
        return Err("backup_database_integrity_failed".to_string());
    }
    load_portable_settings(&paths.data_root)?;
    read_bootstrap(&paths.bootstrap_dir)?.ok_or_else(|| "bootstrap_missing".to_string())?;
    Ok(())
}

pub(crate) fn safety_backup_path(directory: &Path, now: DateTime<Utc>) -> PathBuf {
    directory.join(format!(
        "emily-before-restore-{}-{}.emily-backup",
        now.format("%Y%m%d-%H%M%S-%3f"),
        Uuid::new_v4().simple()
    ))
}

fn remove_database_sidecars(database_path: &Path) {
    let raw = database_path.to_string_lossy();
    let _ = fs::remove_file(format!("{raw}-wal"));
    let _ = fs::remove_file(format!("{raw}-shm"));
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{commit_replace_with_rollback, RestoreCommitStage};
    use crate::{
        bootstrap::{read_bootstrap, write_bootstrap_atomic, BootstrapV1},
        migrate_schema_to_latest_at_startup,
        settings_store::{
            load_portable_settings, write_portable_settings_atomic, PortableSettingsV2,
        },
        AppPaths,
    };
    use rusqlite::Connection;
    use std::{fs, path::PathBuf};
    use uuid::Uuid;

    #[test]
    fn partial_replace_failure_rolls_database_settings_and_bootstrap_back() {
        let root = std::env::temp_dir().join(format!(
            "emily-restore-rollback-{}",
            Uuid::new_v4().simple()
        ));
        let paths = AppPaths::from_roots(
            root.join("system"),
            root.join("data"),
            root.join("resources"),
        );
        fs::create_dir_all(&paths.data_root).unwrap();
        fs::create_dir_all(&paths.bootstrap_dir).unwrap();
        create_database(&paths.database, "original");
        let rollback_database = root.join("rollback.sqlite");
        fs::copy(&paths.database, &rollback_database).unwrap();
        let imported_database = root.join("imported.sqlite");
        create_database(&imported_database, "imported");
        let mut original_settings = PortableSettingsV2::default();
        original_settings.revision = 7;
        write_portable_settings_atomic(&paths.data_root, &original_settings).unwrap();
        let mut imported_settings = PortableSettingsV2::default();
        imported_settings.revision = 11;
        let original_bootstrap = BootstrapV1::new(
            paths.data_root.clone(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        )
        .unwrap();
        write_bootstrap_atomic(&paths.bootstrap_dir, &original_bootstrap).unwrap();
        let mut restored_bootstrap = original_bootstrap.clone();
        restored_bootstrap.active_account_id = Some(7);

        assert_eq!(
            commit_replace_with_rollback(
                &paths,
                &imported_database,
                &imported_settings,
                &restored_bootstrap,
                &rollback_database,
                &original_settings,
                &original_bootstrap,
                |stage| {
                    if stage == RestoreCommitStage::AfterSettings {
                        Err("injected_restore_failure".to_string())
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err(),
            "injected_restore_failure"
        );

        let connection = Connection::open(&paths.database).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT aweme_id FROM videos", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "original"
        );
        assert_eq!(
            load_portable_settings(&paths.data_root).unwrap().revision,
            7
        );
        assert_eq!(
            read_bootstrap(&paths.bootstrap_dir)
                .unwrap()
                .unwrap()
                .active_account_id,
            None
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    fn create_database(path: &PathBuf, aweme_id: &str) {
        let mut connection = Connection::open(path).unwrap();
        migrate_schema_to_latest_at_startup(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO videos
                 (aweme_id, canonical_url, content_type, first_seen_at, last_seen_at)
                 VALUES (?1, ?2, 'video', ?3, ?3)",
                rusqlite::params![
                    aweme_id,
                    format!("https://www.douyin.com/video/{aweme_id}"),
                    "2026-08-15T00:00:00.000Z"
                ],
            )
            .unwrap();
    }
}
