use crate::{
    bootstrap::{read_bootstrap, update_active_account},
    settings_store::{portable_account_key, remove_account_settings},
    sync::{QueueStore, SyncJobStatusV1, TerminalJobUpdateV1},
};
use chrono::{SecondsFormat, Utc};
use rusqlite::{
    params, Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::{collections::BTreeSet, fs};
use url::Url;
use uuid::Uuid;

const MAX_REMOTE_ID_BYTES: usize = 256;
const MAX_NICKNAME_CHARS: usize = 200;
const MAX_AVATAR_URL_BYTES: usize = 2048;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: i64,
    pub uid: Option<String>,
    pub sec_uid: Option<String>,
    pub account_key: Option<String>,
    pub profile_key: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_login_validated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountIdentity {
    pub uid: Option<String>,
    pub sec_uid: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncJobIdentity {
    pub account_id: i64,
    pub sync_job_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncTerminalOutcome {
    Failed {
        error_code: String,
        error_message: Option<String>,
    },
    Cancelled {
        error_message: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum AccountFolderDisposition {
    Delete,
    ConvertToAggregate,
    TransferToAccount {
        #[serde(rename = "targetAccountId")]
        target_account_id: i64,
    },
}

pub fn create_pending_account(connection: &mut Connection) -> Result<AccountView, String> {
    let transaction = immediate_transaction(connection)?;
    let now = now_iso();
    let profile_key = Uuid::new_v4().simple().to_string();
    transaction
        .execute(
            "INSERT INTO accounts
             (profile_key, status, created_at, updated_at)
             VALUES (?1, 'pending_login', ?2, ?2)",
            params![profile_key, now],
        )
        .map_err(|error| error.to_string())?;
    let account_id = transaction.last_insert_rowid();
    insert_remote_folder_registry_state(&transaction, account_id)?;
    let account = account_by_id(&transaction, account_id)?.expect("inserted account");
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(account)
}

pub fn create_account_from_identity(
    connection: &mut Connection,
    profile_key: &str,
    identity: AccountIdentity,
) -> Result<AccountView, String> {
    let identity = normalize_identity(identity)?;
    if !is_simple_uuid(profile_key) {
        return Err("invalid_profile_key".to_string());
    }
    let transaction = immediate_transaction(connection)?;
    if matching_account_id(&transaction, "uid", identity.uid.as_deref())?.is_some()
        || matching_account_id(&transaction, "sec_uid", identity.sec_uid.as_deref())?.is_some()
    {
        return Err("account_already_exists".to_string());
    }
    let now = now_iso();
    transaction
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, nickname, avatar_url, status,
              created_at, updated_at, last_login_validated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6, ?6)",
            params![
                identity.uid,
                identity.sec_uid,
                profile_key,
                identity.nickname,
                identity.avatar_url,
                now
            ],
        )
        .map_err(map_identity_database_error)?;
    let account_id = transaction.last_insert_rowid();
    insert_remote_folder_registry_state(&transaction, account_id)?;
    let account = account_by_id(&transaction, account_id)?.expect("inserted account");
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(account)
}

fn insert_remote_folder_registry_state(
    transaction: &rusqlite::Transaction<'_>,
    account_id: i64,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)",
            [account_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn finalize_account_identity(
    connection: &mut Connection,
    account_id: i64,
    identity: AccountIdentity,
) -> Result<AccountView, String> {
    let identity = normalize_identity(identity)?;
    let transaction = immediate_transaction(connection)?;
    let current =
        account_by_id(&transaction, account_id)?.ok_or_else(|| "account_not_found".to_string())?;
    let uid_match = matching_account_id(&transaction, "uid", identity.uid.as_deref())?;
    let sec_uid_match = matching_account_id(&transaction, "sec_uid", identity.sec_uid.as_deref())?;

    let mut matched_ids = [uid_match, sec_uid_match]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    matched_ids.sort_unstable();
    matched_ids.dedup();
    if matched_ids.len() > 1 {
        return Err("account_identity_conflict".to_string());
    }
    if let Some(matched_id) = matched_ids.first().copied() {
        let matched = account_by_id(&transaction, matched_id)?
            .ok_or_else(|| "account_not_found".to_string())?;
        if matched.status == "archived" || current.status == "archived" {
            return Err("account_archived_requires_restore".to_string());
        }
        if current.status == "pending_login" && matched_id != current.id {
            return Err("account_already_exists".to_string());
        }
        if matched_id != current.id {
            return Err("account_identity_conflict".to_string());
        }
    } else if current.status == "archived" {
        return Err("account_archived_requires_restore".to_string());
    }

    if current.status != "pending_login"
        && current.status != "active"
        && current.status != "login_expired"
        && current.status != "identity_conflict"
    {
        return Err("invalid_account_status".to_string());
    }
    let uid = merge_remote_id(current.uid.as_deref(), identity.uid.as_deref())?;
    let sec_uid = merge_remote_id(current.sec_uid.as_deref(), identity.sec_uid.as_deref())?;
    let now = now_iso();
    transaction
        .execute(
            "UPDATE accounts
             SET uid = ?2, sec_uid = ?3, nickname = ?4, avatar_url = ?5,
                 status = 'active', updated_at = ?6, last_login_validated_at = ?6
             WHERE id = ?1",
            params![
                account_id,
                uid,
                sec_uid,
                identity.nickname,
                identity.avatar_url,
                now
            ],
        )
        .map_err(map_identity_database_error)?;
    let account = account_by_id(&transaction, account_id)?.expect("updated account");
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(account)
}

pub fn list_accounts(connection: &Connection) -> Result<Vec<AccountView>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, uid, sec_uid, profile_key, nickname, avatar_url, status,
                    created_at, updated_at, last_login_validated_at
             FROM accounts ORDER BY id",
        )
        .map_err(|error| error.to_string())?;
    let accounts = statement
        .query_map([], account_from_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(accounts)
}

pub fn list_command_accounts(
    connection: &Connection,
    include_archived: bool,
) -> Result<Vec<AccountView>, String> {
    Ok(list_accounts(connection)?
        .into_iter()
        .filter(|account| {
            account.status == "active"
                || account.status == "login_expired"
                || account.status == "identity_conflict"
                || (include_archived && account.status == "archived")
        })
        .collect())
}

pub fn get_account(
    connection: &Connection,
    account_id: i64,
) -> Result<Option<AccountView>, String> {
    connection
        .query_row(
            "SELECT id, uid, sec_uid, profile_key, nickname, avatar_url, status,
                    created_at, updated_at, last_login_validated_at
             FROM accounts WHERE id = ?1",
            [account_id],
            account_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}

pub fn portable_account_key_for_id(
    connection: &Connection,
    account_id: i64,
) -> Result<String, String> {
    let identity = connection
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
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "account_not_found".to_string())?;
    portable_account_key(identity.0.as_deref(), identity.1.as_deref())
}

pub fn known_portable_account_keys(connection: &Connection) -> Result<BTreeSet<String>, String> {
    let mut statement = connection
        .prepare("SELECT uid, sec_uid FROM accounts ORDER BY id")
        .map_err(|error| error.to_string())?;
    let identities = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut keys = BTreeSet::new();
    for identity in identities {
        let (uid, sec_uid) = identity.map_err(|error| error.to_string())?;
        if let Ok(key) = portable_account_key(uid.as_deref(), sec_uid.as_deref()) {
            keys.insert(key);
        }
    }
    Ok(keys)
}

pub fn set_active_account(
    bootstrap_dir: &Path,
    connection: &Connection,
    account_id: i64,
) -> Result<AccountView, String> {
    let account =
        get_account(connection, account_id)?.ok_or_else(|| "account_not_found".to_string())?;
    if account.status != "active"
        && account.status != "login_expired"
        && account.status != "identity_conflict"
    {
        return Err("account_not_selectable".to_string());
    }
    update_active_account(bootstrap_dir, Some(account_id))
        .map_err(|_| "bootstrap_update_failed".to_string())?;
    Ok(account)
}

pub fn load_active_account(
    bootstrap_dir: &Path,
    connection: &Connection,
) -> Result<Option<AccountView>, String> {
    let bootstrap =
        read_bootstrap(bootstrap_dir)?.ok_or_else(|| "bootstrap_missing".to_string())?;
    let Some(account_id) = bootstrap.active_account_id else {
        return Ok(None);
    };
    let account = get_account(connection, account_id)?;
    if account.as_ref().is_some_and(|account| {
        matches!(
            account.status.as_str(),
            "active" | "login_expired" | "identity_conflict"
        )
    }) {
        return Ok(account);
    }
    update_active_account(bootstrap_dir, None)
        .map_err(|_| "bootstrap_update_failed".to_string())?;
    Ok(None)
}

pub fn restore_account(
    connection: &mut Connection,
    account_id: i64,
) -> Result<AccountView, String> {
    let transaction = immediate_transaction(connection)?;
    let changed = transaction
        .execute(
            "UPDATE accounts SET status = 'login_expired', updated_at = ?2
             WHERE id = ?1 AND status = 'archived'",
            params![account_id, now_iso()],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("account_not_archived".to_string());
    }
    let account = account_by_id(&transaction, account_id)?.expect("restored account");
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(account)
}

pub fn validate_hidden_recovery_identity(
    connection: &Connection,
    account_id: i64,
    identity: AccountIdentity,
) -> Result<AccountView, String> {
    let identity = normalize_identity(identity)?;
    let account =
        get_account(connection, account_id)?.ok_or_else(|| "account_not_found".to_string())?;
    if account.status != "active" {
        return Err("invalid_account_status".to_string());
    }
    let mut shared_identity = false;
    for (local, remote) in [
        (account.uid.as_deref(), identity.uid.as_deref()),
        (account.sec_uid.as_deref(), identity.sec_uid.as_deref()),
    ] {
        if let (Some(local), Some(remote)) = (local, remote) {
            shared_identity = true;
            if local != remote {
                return Err("account_identity_conflict".to_string());
            }
        }
    }
    if !shared_identity {
        return Err("account_identity_conflict".to_string());
    }
    Ok(account)
}

pub fn finish_sync_with_account_state(
    connection: &mut Connection,
    job: SyncJobIdentity,
    outcome: SyncTerminalOutcome,
) -> Result<(), String> {
    if job.account_id <= 0 || job.sync_job_id <= 0 {
        return Err("invalid_sync_job_identity".to_string());
    }
    let transaction = immediate_transaction(connection)?;
    let job_status = transaction
        .query_row(
            "SELECT account_id, status FROM sync_jobs WHERE id = ?1",
            [job.sync_job_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "sync_job_mismatch".to_string())?;
    if job_status.0 != job.account_id || job_status.1 != "running" {
        return Err("sync_job_mismatch".to_string());
    }
    let account = account_by_id(&transaction, job.account_id)?
        .ok_or_else(|| "account_not_found".to_string())?;
    let (terminal_status, error_code, error_message, requested_account_status) = match outcome {
        SyncTerminalOutcome::Failed {
            error_code,
            error_message,
        } => {
            if error_code.is_empty() {
                return Err("invalid_sync_outcome".to_string());
            }
            let account_status = match error_code.as_str() {
                "login_recovery_failed" | "session_expired" => Some("login_expired"),
                "account_identity_conflict" => Some("identity_conflict"),
                _ => None,
            };
            (
                SyncJobStatusV1::Failed,
                Some(error_code),
                error_message,
                account_status,
            )
        }
        SyncTerminalOutcome::Cancelled { error_message } => (
            SyncJobStatusV1::Cancelled,
            Some("collector_cancelled".to_string()),
            error_message,
            None,
        ),
    };
    let now = now_iso();
    if let Some(requested_status) = requested_account_status {
        if account.status == "pending_login" || account.status == "archived" {
            return Err("invalid_account_status".to_string());
        }
        let next_status = if account.status == "identity_conflict" {
            "identity_conflict"
        } else {
            requested_status
        };
        let updated = transaction
            .execute(
                "UPDATE accounts SET status = ?2, updated_at = ?3 WHERE id = ?1",
                params![job.account_id, next_status, now],
            )
            .map_err(|error| error.to_string())?;
        if updated != 1 {
            return Err("account_state_mismatch".to_string());
        }
    }
    QueueStore::finish_running_job_tx(
        &transaction,
        job.sync_job_id,
        &TerminalJobUpdateV1 {
            status: terminal_status,
            finished_at: now,
            inserted_videos: 0,
            inserted_records: 0,
            error_code,
            error_message,
        },
    )?;
    transaction.commit().map_err(|error| error.to_string())
}

pub fn cancel_pending_account(
    connection: &mut Connection,
    account_id: i64,
    profiles_root: &Path,
) -> Result<(), String> {
    let transaction = immediate_transaction(connection)?;
    let account =
        account_by_id(&transaction, account_id)?.ok_or_else(|| "account_not_found".to_string())?;
    if account.status != "pending_login" {
        return Err("account_not_pending".to_string());
    }
    let profile_path = controlled_profile_path(profiles_root, &account.profile_key)?;
    transaction
        .execute("DELETE FROM accounts WHERE id = ?1", [account_id])
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    remove_profile_directory(profiles_root, &profile_path)
}

pub fn remove_unclaimed_profile(profiles_root: &Path, profile_key: &str) -> Result<(), String> {
    let profile_path = controlled_profile_path(profiles_root, profile_key)?;
    remove_profile_directory(profiles_root, &profile_path)
}

pub fn archive_account(
    connection: &mut Connection,
    bootstrap_dir: &Path,
    account_id: i64,
) -> Result<(), String> {
    archive_account_with_active_writer(
        connection,
        bootstrap_dir,
        account_id,
        |directory, active_account_id| {
            update_active_account(directory, active_account_id).map(|_| ())
        },
    )
}

pub fn archive_account_with_active_writer<W>(
    connection: &mut Connection,
    bootstrap_dir: &Path,
    account_id: i64,
    write_active_account: W,
) -> Result<(), String>
where
    W: FnOnce(&Path, Option<i64>) -> Result<(), String>,
{
    let current_active_account_id = read_bootstrap(bootstrap_dir)?
        .ok_or_else(|| "bootstrap_missing".to_string())?
        .active_account_id;
    let transaction = immediate_transaction(connection)?;
    let changed_at = now_iso();
    prepare_account_queue_mutation_tx(&transaction, account_id, "account_archived", &changed_at)?;
    let changed = transaction
        .execute(
            "UPDATE accounts SET status = 'archived', updated_at = ?2 WHERE id = ?1",
            params![account_id, changed_at],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("account_not_found".to_string());
    }
    let fallback = fallback_active_account_id(&transaction, account_id)?;
    transaction.commit().map_err(|error| error.to_string())?;

    if current_active_account_id == Some(account_id) {
        write_active_account(bootstrap_dir, fallback)
            .map_err(|_| "bootstrap_update_failed".to_string())?;
    }
    Ok(())
}

pub fn add_local_folder_provenance(
    connection: &mut Connection,
    folder_id: i64,
    video_id: i64,
    account_id: i64,
) -> Result<(), String> {
    let transaction = immediate_transaction(connection)?;
    let scope = transaction
        .query_row(
            "SELECT scope FROM local_folders WHERE id = ?1",
            [folder_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "local_folder_not_found".to_string())?;
    if scope != "aggregate" {
        return Err("local_folder_provenance_requires_aggregate".to_string());
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO local_folder_item_accounts
             (folder_id, video_id, account_id, added_at) VALUES (?1, ?2, ?3, ?4)",
            params![folder_id, video_id, account_id, now_iso()],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub fn delete_account_data(
    connection: &mut Connection,
    bootstrap_dir: &Path,
    data_root: &Path,
    profiles_root: &Path,
    account_id: i64,
    disposition: AccountFolderDisposition,
) -> Result<(), String> {
    delete_account_data_with_state_writers(
        connection,
        bootstrap_dir,
        data_root,
        profiles_root,
        account_id,
        disposition,
        |directory, active_account_id| {
            update_active_account(directory, active_account_id).map(|_| ())
        },
        |root, key| remove_account_settings(root, key).map(|_| ()),
    )
}

pub fn delete_account_data_with_state_writers<W, R>(
    connection: &mut Connection,
    bootstrap_dir: &Path,
    data_root: &Path,
    profiles_root: &Path,
    account_id: i64,
    disposition: AccountFolderDisposition,
    write_active_account: W,
    remove_settings: R,
) -> Result<(), String>
where
    W: FnOnce(&Path, Option<i64>) -> Result<(), String>,
    R: FnOnce(&Path, &str) -> Result<(), String>,
{
    let current_active_account_id = read_bootstrap(bootstrap_dir)?
        .ok_or_else(|| "bootstrap_missing".to_string())?
        .active_account_id;
    let portable_key = portable_account_key_for_id(connection, account_id)?;
    let transaction = immediate_transaction(connection)?;
    let account =
        account_by_id(&transaction, account_id)?.ok_or_else(|| "account_not_found".to_string())?;
    let profile_path = controlled_profile_path(profiles_root, &account.profile_key)?;
    let fallback = fallback_active_account_id(&transaction, account_id)?;
    let changed_at = now_iso();
    prepare_account_queue_mutation_tx(&transaction, account_id, "account_deleted", &changed_at)?;

    apply_folder_disposition(&transaction, account_id, disposition)?;
    transaction
        .execute("DELETE FROM accounts WHERE id = ?1", [account_id])
        .map_err(|error| error.to_string())?;
    delete_orphan_videos(&transaction)?;
    transaction.commit().map_err(|error| error.to_string())?;

    let active_account_id = if current_active_account_id == Some(account_id) {
        fallback
    } else {
        current_active_account_id
    };
    let active_update = write_active_account(bootstrap_dir, active_account_id)
        .map_err(|_| "bootstrap_update_failed".to_string());
    let _ = remove_settings(data_root, &portable_key);
    active_update?;
    remove_profile_directory(profiles_root, &profile_path)
}

fn apply_folder_disposition(
    transaction: &Transaction<'_>,
    account_id: i64,
    disposition: AccountFolderDisposition,
) -> Result<(), String> {
    match disposition {
        AccountFolderDisposition::Delete => Ok(()),
        AccountFolderDisposition::ConvertToAggregate => {
            if folder_name_conflict(transaction, account_id, None)? {
                return Err("local_folder_name_conflict".to_string());
            }
            transaction
                .execute(
                    "INSERT INTO local_folder_item_accounts
                     (folder_id, video_id, account_id, added_at)
                     SELECT lfi.folder_id, lfi.video_id, ?1, lfi.added_at
                     FROM local_folder_items lfi
                     JOIN local_folders lf ON lf.id = lfi.folder_id
                     WHERE lf.scope = 'account' AND lf.owner_account_id = ?1",
                    [account_id],
                )
                .map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "UPDATE local_folders SET scope = 'aggregate', owner_account_id = NULL,
                     updated_at = ?2 WHERE scope = 'account' AND owner_account_id = ?1",
                    params![account_id, now_iso()],
                )
                .map_err(|error| error.to_string())?;
            Ok(())
        }
        AccountFolderDisposition::TransferToAccount { target_account_id } => {
            if target_account_id == account_id
                || !account_is_active(transaction, target_account_id)?
            {
                return Err("invalid_transfer_account".to_string());
            }
            if folder_name_conflict(transaction, account_id, Some(target_account_id))? {
                return Err("local_folder_name_conflict".to_string());
            }
            transaction
                .execute(
                    "UPDATE local_folders SET owner_account_id = ?2, updated_at = ?3
                     WHERE scope = 'account' AND owner_account_id = ?1",
                    params![account_id, target_account_id, now_iso()],
                )
                .map_err(|error| error.to_string())?;
            Ok(())
        }
    }
}

fn folder_name_conflict(
    transaction: &Transaction<'_>,
    source_account_id: i64,
    target_account_id: Option<i64>,
) -> Result<bool, String> {
    if let Some(target_account_id) = target_account_id {
        transaction
            .query_row(
                "SELECT EXISTS(
           SELECT 1 FROM local_folders source
           JOIN local_folders target ON target.name = source.name COLLATE NOCASE
           WHERE source.scope = 'account' AND source.owner_account_id = ?1
             AND target.scope = 'account' AND target.owner_account_id = ?2
         )",
                params![source_account_id, target_account_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| error.to_string())
    } else {
        transaction
            .query_row(
                "SELECT EXISTS(
           SELECT 1 FROM local_folders source
           JOIN local_folders target ON target.name = source.name COLLATE NOCASE
           WHERE source.scope = 'account' AND source.owner_account_id = ?1
             AND target.scope = 'aggregate'
         )",
                [source_account_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| error.to_string())
    }
}

fn delete_orphan_videos(transaction: &Transaction<'_>) -> Result<(), String> {
    transaction
        .execute(
            "DELETE FROM videos
             WHERE id NOT IN (SELECT video_id FROM watch_records)
               AND id NOT IN (SELECT video_id FROM user_likes)
               AND id NOT IN (SELECT video_id FROM favorite_items)
               AND id NOT IN (SELECT video_id FROM remote_folder_items)
               AND id NOT IN (SELECT video_id FROM local_folder_items)",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn fallback_active_account_id(
    transaction: &Transaction<'_>,
    excluded_account_id: i64,
) -> Result<Option<i64>, String> {
    transaction
        .query_row(
            "SELECT id FROM accounts
             WHERE status = 'active' AND id <> ?1 ORDER BY id LIMIT 1",
            [excluded_account_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn account_is_active(transaction: &Transaction<'_>, account_id: i64) -> Result<bool, String> {
    transaction
        .query_row(
            "SELECT status = 'active' FROM accounts WHERE id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .optional()
        .map(|value| value.unwrap_or(false))
        .map_err(|error| error.to_string())
}

fn matching_account_id(
    transaction: &Transaction<'_>,
    column: &str,
    value: Option<&str>,
) -> Result<Option<i64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    transaction
        .query_row(
            &format!("SELECT id FROM accounts WHERE {column} = ?1"),
            [value],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn merge_remote_id(
    existing: Option<&str>,
    incoming: Option<&str>,
) -> Result<Option<String>, String> {
    match (existing, incoming) {
        (Some(existing), Some(incoming)) if existing != incoming => {
            Err("account_identity_conflict".to_string())
        }
        (Some(existing), _) => Ok(Some(existing.to_string())),
        (None, Some(incoming)) => Ok(Some(incoming.to_string())),
        (None, None) => Ok(None),
    }
}

fn normalize_identity(identity: AccountIdentity) -> Result<AccountIdentity, String> {
    let uid = normalize_optional(identity.uid);
    let sec_uid = normalize_optional(identity.sec_uid);
    if uid.is_none() && sec_uid.is_none() {
        return Err("invalid_account_identity".to_string());
    }
    if uid
        .as_ref()
        .is_some_and(|value| value.len() > MAX_REMOTE_ID_BYTES)
        || sec_uid
            .as_ref()
            .is_some_and(|value| value.len() > MAX_REMOTE_ID_BYTES)
    {
        return Err("invalid_account_identity".to_string());
    }
    let nickname = normalize_optional(identity.nickname);
    if nickname
        .as_ref()
        .is_some_and(|value| value.chars().count() > MAX_NICKNAME_CHARS)
    {
        return Err("invalid_account_identity".to_string());
    }
    let avatar_url = normalize_optional(identity.avatar_url);
    if let Some(value) = avatar_url.as_deref() {
        let valid = value.len() <= MAX_AVATAR_URL_BYTES
            && Url::parse(value)
                .ok()
                .is_some_and(|url| matches!(url.scheme(), "http" | "https"));
        if !valid {
            return Err("invalid_account_identity".to_string());
        }
    }
    Ok(AccountIdentity {
        uid,
        sec_uid,
        nickname,
        avatar_url,
    })
}

fn is_simple_uuid(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn controlled_profile_path(profiles_root: &Path, profile_key: &str) -> Result<PathBuf, String> {
    let mut components = Path::new(profile_key).components();
    let valid = matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
        && profile_key != "."
        && profile_key != "..";
    if !valid {
        return Err("profile_path_not_controlled".to_string());
    }
    let profile_path = profiles_root.join(profile_key);
    validate_existing_profile_path(profiles_root, &profile_path)?;
    Ok(profile_path)
}

fn validate_existing_profile_path(
    profiles_root: &Path,
    profile_path: &Path,
) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(profile_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("profile_path_not_controlled".to_string()),
    };
    if metadata.file_type().is_symlink() || is_windows_reparse_point(&metadata) {
        return Err("profile_path_not_controlled".to_string());
    }
    let canonical_root =
        fs::canonicalize(profiles_root).map_err(|_| "profile_path_not_controlled".to_string())?;
    let canonical_target =
        fs::canonicalize(profile_path).map_err(|_| "profile_path_not_controlled".to_string())?;
    if canonical_target.parent() != Some(canonical_root.as_path()) {
        return Err("profile_path_not_controlled".to_string());
    }
    Ok(true)
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn remove_profile_directory(profiles_root: &Path, profile_path: &Path) -> Result<(), String> {
    if !validate_existing_profile_path(profiles_root, profile_path)? {
        return Ok(());
    }
    fs::remove_dir_all(profile_path).map_err(|_| "profile_cleanup_failed".to_string())
}

fn account_by_id(
    transaction: &Transaction<'_>,
    account_id: i64,
) -> Result<Option<AccountView>, String> {
    transaction
        .query_row(
            "SELECT id, uid, sec_uid, profile_key, nickname, avatar_url, status,
                    created_at, updated_at, last_login_validated_at
             FROM accounts WHERE id = ?1",
            [account_id],
            account_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn account_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccountView> {
    let uid: Option<String> = row.get(1)?;
    let sec_uid: Option<String> = row.get(2)?;
    let account_key = portable_account_key(uid.as_deref(), sec_uid.as_deref()).ok();
    Ok(AccountView {
        id: row.get(0)?,
        uid,
        sec_uid,
        account_key,
        profile_key: row.get(3)?,
        nickname: row.get(4)?,
        avatar_url: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        last_login_validated_at: row.get(9)?,
    })
}

fn immediate_transaction(connection: &mut Connection) -> Result<Transaction<'_>, String> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())
}

fn prepare_account_queue_mutation_tx(
    transaction: &Transaction<'_>,
    account_id: i64,
    cancellation_code: &str,
    changed_at: &str,
) -> Result<(), String> {
    let running: bool = transaction
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM sync_jobs WHERE account_id = ?1 AND status = 'running'
             )",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if running {
        return Err("account_sync_running".to_string());
    }
    let mut statement = transaction
        .prepare("SELECT id FROM sync_jobs WHERE account_id = ?1 AND status = 'queued' ORDER BY id")
        .map_err(|error| error.to_string())?;
    let job_ids = statement
        .query_map([account_id], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    for job_id in job_ids {
        QueueStore::cancel_queued_job_tx(transaction, job_id, changed_at, cancellation_code)?;
    }
    Ok(())
}

fn map_identity_database_error(error: rusqlite::Error) -> String {
    match error {
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == ErrorCode::ConstraintViolation =>
        {
            "account_identity_conflict".to_string()
        }
        error => error.to_string(),
    }
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
