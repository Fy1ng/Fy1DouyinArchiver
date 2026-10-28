use chrono::{SecondsFormat, Utc};
use rusqlite::{
    params, Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};

const MAX_FOLDER_NAME_CHARS: usize = 100;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum LocalFolderScopeV1 {
    Account,
    Aggregate,
}

impl LocalFolderScopeV1 {
    fn as_str(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Aggregate => "aggregate",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFolderViewV1 {
    pub id: i64,
    pub remote_folder_id: String,
    pub name: String,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub remote_removed_at: Option<String>,
    pub item_count: i64,
    pub active_item_count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalFolderViewV1 {
    pub id: i64,
    pub scope: LocalFolderScopeV1,
    pub owner_account_id: Option<i64>,
    pub name: String,
    pub item_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FolderSourceAccountViewV1 {
    pub id: i64,
    pub nickname: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FolderContentRecordViewV1 {
    pub video_id: i64,
    pub aweme_id: String,
    pub url: String,
    pub content_type: String,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub cover_url: Option<String>,
    pub cached_cover_url: Option<String>,
    pub like_count: Option<i64>,
    pub added_at: Option<String>,
    pub first_seen_at: Option<String>,
    pub last_seen_at: Option<String>,
    pub remote_removed_at: Option<String>,
    pub source_accounts: Vec<FolderSourceAccountViewV1>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListFolderContentResultV1 {
    pub records: Vec<FolderContentRecordViewV1>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalFolderItemMutationV1 {
    pub inserted_item: bool,
    pub inserted_provenance: bool,
}

pub fn list_remote_folders(
    connection: &Connection,
    account_id: i64,
    include_remote_removed: bool,
) -> Result<Vec<RemoteFolderViewV1>, String> {
    require_account(connection, account_id)?;
    let mut statement = connection
        .prepare(
            "SELECT folders.id, folders.remote_folder_id, folders.name,
                    folders.first_seen_at, folders.last_seen_at, folders.remote_removed_at,
                    COUNT(items.video_id),
                    SUM(CASE WHEN items.video_id IS NOT NULL AND items.remote_removed_at IS NULL
                             THEN 1 ELSE 0 END)
             FROM remote_folders folders
             LEFT JOIN remote_folder_items items ON items.folder_id = folders.id
             WHERE folders.account_id = ?1
               AND (?2 = 1 OR folders.remote_removed_at IS NULL)
             GROUP BY folders.id
             ORDER BY CASE WHEN folders.remote_removed_at IS NULL THEN 0 ELSE 1 END,
                      folders.name COLLATE NOCASE, folders.remote_folder_id",
        )
        .map_err(|error| error.to_string())?;
    let folders = statement
        .query_map(
            params![account_id, i64::from(include_remote_removed)],
            |row| {
                Ok(RemoteFolderViewV1 {
                    id: row.get(0)?,
                    remote_folder_id: row.get(1)?,
                    name: row.get(2)?,
                    first_seen_at: row.get(3)?,
                    last_seen_at: row.get(4)?,
                    remote_removed_at: row.get(5)?,
                    item_count: row.get(6)?,
                    active_item_count: row.get(7)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(folders)
}

pub fn list_local_folders(
    connection: &Connection,
    account_id: i64,
) -> Result<Vec<LocalFolderViewV1>, String> {
    require_account(connection, account_id)?;
    let mut statement = connection
        .prepare(
            "SELECT folders.id, folders.scope, folders.owner_account_id, folders.name,
                    COUNT(items.video_id), folders.created_at, folders.updated_at
             FROM local_folders folders
             LEFT JOIN local_folder_items items ON items.folder_id = folders.id
             WHERE folders.scope = 'aggregate'
                OR (folders.scope = 'account' AND folders.owner_account_id = ?1)
             GROUP BY folders.id
             ORDER BY CASE folders.scope WHEN 'account' THEN 0 ELSE 1 END,
                      folders.name COLLATE NOCASE, folders.id",
        )
        .map_err(|error| error.to_string())?;
    let folders = statement
        .query_map([account_id], local_folder_from_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(folders)
}

pub fn create_local_folder(
    connection: &mut Connection,
    account_id: i64,
    name: &str,
    scope: LocalFolderScopeV1,
) -> Result<LocalFolderViewV1, String> {
    let name = normalize_folder_name(name)?;
    let transaction = immediate_transaction(connection)?;
    require_account(&transaction, account_id)?;
    let now = now_iso();
    let owner_account_id = (scope == LocalFolderScopeV1::Account).then_some(account_id);
    transaction
        .execute(
            "INSERT INTO local_folders
             (scope, owner_account_id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![scope.as_str(), owner_account_id, name, now],
        )
        .map_err(map_folder_database_error)?;
    let folder_id = transaction.last_insert_rowid();
    let folder = local_folder_by_id(&transaction, folder_id, account_id)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(folder)
}

pub fn rename_local_folder(
    connection: &mut Connection,
    account_id: i64,
    folder_id: i64,
    name: &str,
) -> Result<LocalFolderViewV1, String> {
    let name = normalize_folder_name(name)?;
    let transaction = immediate_transaction(connection)?;
    require_folder_access(&transaction, folder_id, account_id)?;
    transaction
        .execute(
            "UPDATE local_folders SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![folder_id, name, now_iso()],
        )
        .map_err(map_folder_database_error)?;
    let folder = local_folder_by_id(&transaction, folder_id, account_id)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(folder)
}

pub fn set_local_folder_scope(
    connection: &mut Connection,
    account_id: i64,
    folder_id: i64,
    scope: LocalFolderScopeV1,
    owner_account_id: Option<i64>,
) -> Result<LocalFolderViewV1, String> {
    let transaction = immediate_transaction(connection)?;
    let current = require_folder_access(&transaction, folder_id, account_id)?;
    let target_owner = match scope {
        LocalFolderScopeV1::Account => {
            let target =
                owner_account_id.ok_or_else(|| "local_folder_owner_required".to_string())?;
            require_account(&transaction, target)?;
            Some(target)
        }
        LocalFolderScopeV1::Aggregate => {
            if owner_account_id.is_some() {
                return Err("aggregate_folder_owner_forbidden".to_string());
            }
            None
        }
    };

    if current.scope == LocalFolderScopeV1::Account && scope == LocalFolderScopeV1::Aggregate {
        let source_owner = current
            .owner_account_id
            .ok_or_else(|| "local_folder_scope_corrupt".to_string())?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO local_folder_item_accounts
                 (folder_id, video_id, account_id, added_at)
                 SELECT items.folder_id, items.video_id, ?2, items.added_at
                 FROM local_folder_items items WHERE items.folder_id = ?1",
                params![folder_id, source_owner],
            )
            .map_err(|error| error.to_string())?;
    } else if current.scope == LocalFolderScopeV1::Aggregate && scope == LocalFolderScopeV1::Account
    {
        transaction
            .execute(
                "DELETE FROM local_folder_item_accounts WHERE folder_id = ?1",
                [folder_id],
            )
            .map_err(|error| error.to_string())?;
    }

    transaction
        .execute(
            "UPDATE local_folders
             SET scope = ?2, owner_account_id = ?3, updated_at = ?4
             WHERE id = ?1",
            params![folder_id, scope.as_str(), target_owner, now_iso()],
        )
        .map_err(map_folder_database_error)?;
    let folder = local_folder_by_id(&transaction, folder_id, account_id)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(folder)
}

pub fn delete_local_folder(
    connection: &mut Connection,
    account_id: i64,
    folder_id: i64,
) -> Result<(), String> {
    let transaction = immediate_transaction(connection)?;
    require_folder_access(&transaction, folder_id, account_id)?;
    transaction
        .execute("DELETE FROM local_folders WHERE id = ?1", [folder_id])
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub fn add_local_folder_item(
    connection: &mut Connection,
    account_id: i64,
    folder_id: i64,
    aweme_id: &str,
) -> Result<LocalFolderItemMutationV1, String> {
    let transaction = immediate_transaction(connection)?;
    let folder = require_folder_access(&transaction, folder_id, account_id)?;
    if folder.scope == LocalFolderScopeV1::Account && folder.owner_account_id != Some(account_id) {
        return Err("local_folder_access_denied".to_string());
    }
    let video_id = account_video_id(&transaction, account_id, aweme_id)?;
    let now = now_iso();
    let inserted_item = transaction
        .execute(
            "INSERT OR IGNORE INTO local_folder_items(folder_id, video_id, added_at)
             VALUES (?1, ?2, ?3)",
            params![folder_id, video_id, now],
        )
        .map_err(|error| error.to_string())?
        == 1;
    let inserted_provenance = if folder.scope == LocalFolderScopeV1::Aggregate {
        transaction
            .execute(
                "INSERT OR IGNORE INTO local_folder_item_accounts
                 (folder_id, video_id, account_id, added_at) VALUES (?1, ?2, ?3, ?4)",
                params![folder_id, video_id, account_id, now],
            )
            .map_err(|error| error.to_string())?
            == 1
    } else {
        false
    };
    transaction
        .execute(
            "UPDATE local_folders SET updated_at = ?2 WHERE id = ?1",
            params![folder_id, now_iso()],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(LocalFolderItemMutationV1 {
        inserted_item,
        inserted_provenance,
    })
}

pub fn remove_local_folder_item(
    connection: &mut Connection,
    account_id: i64,
    folder_id: i64,
    aweme_id: &str,
) -> Result<bool, String> {
    let transaction = immediate_transaction(connection)?;
    require_folder_access(&transaction, folder_id, account_id)?;
    let video_id = transaction
        .query_row(
            "SELECT id FROM videos WHERE aweme_id = ?1",
            [aweme_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(video_id) = video_id else {
        return Ok(false);
    };
    let removed = transaction
        .execute(
            "DELETE FROM local_folder_items WHERE folder_id = ?1 AND video_id = ?2",
            params![folder_id, video_id],
        )
        .map_err(|error| error.to_string())?
        == 1;
    if removed {
        transaction
            .execute(
                "UPDATE local_folders SET updated_at = ?2 WHERE id = ?1",
                params![folder_id, now_iso()],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(removed)
}

pub fn list_remote_folder_content(
    connection: &Connection,
    account_id: i64,
    remote_folder_id: &str,
    query: Option<&str>,
    include_remote_removed: bool,
    limit: usize,
    offset: usize,
) -> Result<ListFolderContentResultV1, String> {
    require_account(connection, account_id)?;
    let folder_id = connection
        .query_row(
            "SELECT id FROM remote_folders WHERE account_id = ?1 AND remote_folder_id = ?2",
            params![account_id, remote_folder_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "remote_folder_not_found".to_string())?;
    let like_query = normalized_query(query);
    let total = connection
        .query_row(
            "SELECT COUNT(*) FROM remote_folder_items items
             JOIN videos ON videos.id = items.video_id
             WHERE items.folder_id = ?1
               AND (?2 IS NULL OR videos.title LIKE ?2 OR videos.author_name LIKE ?2)
               AND (?3 = 1 OR items.remote_removed_at IS NULL)",
            params![folder_id, like_query, i64::from(include_remote_removed)],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(
            "SELECT videos.id, videos.aweme_id, videos.canonical_url, videos.content_type,
                    videos.title, videos.author_name, videos.cover_url, videos.like_count,
                    items.first_seen_at, items.last_seen_at, items.remote_removed_at
             FROM remote_folder_items items
             JOIN videos ON videos.id = items.video_id
             WHERE items.folder_id = ?1
               AND (?2 IS NULL OR videos.title LIKE ?2 OR videos.author_name LIKE ?2)
               AND (?3 = 1 OR items.remote_removed_at IS NULL)
             ORDER BY items.last_seen_at DESC, videos.id DESC LIMIT ?4 OFFSET ?5",
        )
        .map_err(|error| error.to_string())?;
    let source_account = source_account(connection, account_id)?;
    let records = statement
        .query_map(
            params![
                folder_id,
                like_query,
                i64::from(include_remote_removed),
                limit.clamp(1, 200) as i64,
                offset as i64
            ],
            |row| {
                Ok(FolderContentRecordViewV1 {
                    video_id: row.get(0)?,
                    aweme_id: row.get(1)?,
                    url: row.get(2)?,
                    content_type: row.get(3)?,
                    title: row.get(4)?,
                    author_name: row.get(5)?,
                    cover_url: row.get(6)?,
                    cached_cover_url: None,
                    like_count: row.get(7)?,
                    added_at: None,
                    first_seen_at: row.get(8)?,
                    last_seen_at: row.get(9)?,
                    remote_removed_at: row.get(10)?,
                    source_accounts: vec![source_account.clone()],
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut records = records;
    attach_cached_cover_urls(connection, &mut records)?;
    Ok(ListFolderContentResultV1 { records, total })
}

pub fn list_local_folder_content(
    connection: &Connection,
    account_id: i64,
    folder_id: i64,
    query: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<ListFolderContentResultV1, String> {
    let folder = require_folder_access(connection, folder_id, account_id)?;
    let like_query = normalized_query(query);
    let total = connection
        .query_row(
            "SELECT COUNT(*) FROM local_folder_items items
             JOIN videos ON videos.id = items.video_id
             WHERE items.folder_id = ?1
               AND (?2 IS NULL OR videos.title LIKE ?2 OR videos.author_name LIKE ?2)",
            params![folder_id, like_query],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(
            "SELECT videos.id, videos.aweme_id, videos.canonical_url, videos.content_type,
                    videos.title, videos.author_name, videos.cover_url, videos.like_count,
                    items.added_at
             FROM local_folder_items items
             JOIN videos ON videos.id = items.video_id
             WHERE items.folder_id = ?1
               AND (?2 IS NULL OR videos.title LIKE ?2 OR videos.author_name LIKE ?2)
             ORDER BY items.added_at DESC, videos.id DESC LIMIT ?3 OFFSET ?4",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                folder_id,
                like_query,
                limit.clamp(1, 200) as i64,
                offset as i64
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let source_accounts = if folder.scope == LocalFolderScopeV1::Account {
            vec![source_account(
                connection,
                folder
                    .owner_account_id
                    .ok_or_else(|| "local_folder_scope_corrupt".to_string())?,
            )?]
        } else {
            aggregate_source_accounts(connection, folder_id, row.0)?
        };
        records.push(FolderContentRecordViewV1 {
            video_id: row.0,
            aweme_id: row.1,
            url: row.2,
            content_type: row.3,
            title: row.4,
            author_name: row.5,
            cover_url: row.6,
            cached_cover_url: None,
            like_count: row.7,
            added_at: Some(row.8),
            first_seen_at: None,
            last_seen_at: None,
            remote_removed_at: local_item_remote_removed_at(connection, row.0, &source_accounts)?,
            source_accounts,
        });
    }
    attach_cached_cover_urls(connection, &mut records)?;
    Ok(ListFolderContentResultV1 { records, total })
}

fn attach_cached_cover_urls(
    connection: &Connection,
    records: &mut [FolderContentRecordViewV1],
) -> Result<(), String> {
    for record in records {
        record.cached_cover_url =
            crate::cover_cache::cached_cover_url_for_video_id(connection, record.video_id)?;
    }
    Ok(())
}

fn normalized_query(query: Option<&str>) -> Option<String> {
    let trimmed = query.unwrap_or_default().trim();
    (!trimmed.is_empty()).then(|| format!("%{trimmed}%"))
}

fn account_video_id(
    connection: &Connection,
    account_id: i64,
    aweme_id: &str,
) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT videos.id FROM videos
             WHERE videos.aweme_id = ?2 AND (
               EXISTS(SELECT 1 FROM watch_records r
                      WHERE r.account_id = ?1 AND r.video_id = videos.id)
               OR EXISTS(SELECT 1 FROM user_likes r
                         WHERE r.account_id = ?1 AND r.video_id = videos.id)
               OR EXISTS(SELECT 1 FROM favorite_items r
                         WHERE r.account_id = ?1 AND r.video_id = videos.id)
               OR EXISTS(SELECT 1 FROM remote_folder_items r
                         JOIN remote_folders f ON f.id = r.folder_id
                         WHERE f.account_id = ?1 AND r.video_id = videos.id)
             )",
            params![account_id, aweme_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "account_content_not_found".to_string())
}

fn require_account(connection: &Connection, account_id: i64) -> Result<(), String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts
             WHERE id = ?1 AND status IN ('active', 'login_expired', 'identity_conflict'))",
            [account_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err("account_not_found".to_string());
    }
    Ok(())
}

fn require_folder_access(
    connection: &Connection,
    folder_id: i64,
    account_id: i64,
) -> Result<LocalFolderViewV1, String> {
    require_account(connection, account_id)?;
    connection
        .query_row(
            "SELECT folders.id, folders.scope, folders.owner_account_id, folders.name,
                    (SELECT COUNT(*) FROM local_folder_items items
                     WHERE items.folder_id = folders.id),
                    folders.created_at, folders.updated_at
             FROM local_folders folders
             WHERE folders.id = ?1
               AND (folders.scope = 'aggregate' OR folders.owner_account_id = ?2)",
            params![folder_id, account_id],
            local_folder_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "local_folder_not_found".to_string())
}

fn local_folder_by_id(
    connection: &Connection,
    folder_id: i64,
    account_id: i64,
) -> Result<LocalFolderViewV1, String> {
    require_folder_access(connection, folder_id, account_id)
}

fn local_folder_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalFolderViewV1> {
    let scope = match row.get::<_, String>(1)?.as_str() {
        "account" => LocalFolderScopeV1::Account,
        "aggregate" => LocalFolderScopeV1::Aggregate,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(LocalFolderViewV1 {
        id: row.get(0)?,
        scope,
        owner_account_id: row.get(2)?,
        name: row.get(3)?,
        item_count: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn source_account(
    connection: &Connection,
    account_id: i64,
) -> Result<FolderSourceAccountViewV1, String> {
    connection
        .query_row(
            "SELECT id, nickname FROM accounts WHERE id = ?1",
            [account_id],
            |row| {
                Ok(FolderSourceAccountViewV1 {
                    id: row.get(0)?,
                    nickname: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "account_not_found".to_string())
}

fn aggregate_source_accounts(
    connection: &Connection,
    folder_id: i64,
    video_id: i64,
) -> Result<Vec<FolderSourceAccountViewV1>, String> {
    let mut statement = connection
        .prepare(
            "SELECT accounts.id, accounts.nickname
             FROM local_folder_item_accounts provenance
             JOIN accounts ON accounts.id = provenance.account_id
             WHERE provenance.folder_id = ?1 AND provenance.video_id = ?2
             ORDER BY accounts.id",
        )
        .map_err(|error| error.to_string())?;
    let accounts = statement
        .query_map(params![folder_id, video_id], |row| {
            Ok(FolderSourceAccountViewV1 {
                id: row.get(0)?,
                nickname: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(accounts)
}

fn local_item_remote_removed_at(
    connection: &Connection,
    video_id: i64,
    source_accounts: &[FolderSourceAccountViewV1],
) -> Result<Option<String>, String> {
    if source_accounts.is_empty() {
        return Ok(None);
    }
    let mut latest_removed = None;
    for account in source_accounts {
        let (active_or_history, removed): (bool, Option<String>) = connection
            .query_row(
                "SELECT
                   EXISTS(SELECT 1 FROM watch_records r
                          WHERE r.account_id = ?1 AND r.video_id = ?2)
                   OR EXISTS(SELECT 1 FROM user_likes r
                             WHERE r.account_id = ?1 AND r.video_id = ?2
                               AND r.remote_removed_at IS NULL)
                   OR EXISTS(SELECT 1 FROM favorite_items r
                             WHERE r.account_id = ?1 AND r.video_id = ?2
                               AND r.remote_removed_at IS NULL)
                   OR EXISTS(SELECT 1 FROM remote_folder_items r
                             JOIN remote_folders f ON f.id = r.folder_id
                             WHERE f.account_id = ?1 AND r.video_id = ?2
                               AND f.remote_removed_at IS NULL
                               AND r.remote_removed_at IS NULL),
                   MAX(removed_at)
                 FROM (
                   SELECT remote_removed_at AS removed_at FROM user_likes
                    WHERE account_id = ?1 AND video_id = ?2
                   UNION ALL
                   SELECT remote_removed_at FROM favorite_items
                    WHERE account_id = ?1 AND video_id = ?2
                   UNION ALL
                   SELECT r.remote_removed_at FROM remote_folder_items r
                    JOIN remote_folders f ON f.id = r.folder_id
                    WHERE f.account_id = ?1 AND r.video_id = ?2
                 )",
                params![account.id, video_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| error.to_string())?;
        if active_or_history {
            return Ok(None);
        }
        if removed > latest_removed {
            latest_removed = removed;
        }
    }
    Ok(latest_removed)
}

fn normalize_folder_name(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_FOLDER_NAME_CHARS
        || trimmed.chars().any(char::is_control)
    {
        return Err("invalid_local_folder_name".to_string());
    }
    Ok(trimmed.to_string())
}

fn map_folder_database_error(error: rusqlite::Error) -> String {
    if let rusqlite::Error::SqliteFailure(code, _) = &error {
        if code.code == ErrorCode::ConstraintViolation {
            return "local_folder_name_conflict".to_string();
        }
    }
    error.to_string()
}

fn immediate_transaction(connection: &mut Connection) -> Result<Transaction<'_>, String> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
