use crate::{
    settings_store::{portable_account_key, validate_account_scope, AccountScopeV1},
    sync::SyncSourceV1,
};
use chrono::NaiveDate;
use rusqlite::{params, params_from_iter, types::Value, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContentSortV1 {
    Default,
    WatchedNewest,
    WatchedOldest,
    PageOrder,
    RemoteNewest,
    RemoteOldest,
    LikeCountDesc,
    TitleAsc,
    AuthorAsc,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContentRecordView {
    pub id: i64,
    pub source: SyncSourceV1,
    pub aweme_id: String,
    pub url: String,
    pub content_type: String,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub cover_url: Option<String>,
    pub cached_cover_url: Option<String>,
    pub like_count: Option<i64>,
    pub source_position: Option<i64>,
    pub history_time_ms: Option<i64>,
    pub watched_at: Option<String>,
    pub watched_date: Option<String>,
    pub watch_progress_rate: Option<f64>,
    pub remote_created_at_ms: Option<i64>,
    pub first_seen_at: Option<String>,
    pub last_seen_at: Option<String>,
    pub remote_removed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListContentResult {
    pub records: Vec<ContentRecordView>,
    pub total: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LibraryContentSortV1 {
    Default,
    ArchivedNewest,
    WatchedNewest,
    WatchedOldest,
    PageOrder,
    RemoteNewest,
    RemoteOldest,
    LikeCountDesc,
    TitleAsc,
    AuthorAsc,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryContentQueryV1 {
    pub account_scope: AccountScopeV1,
    pub sources: Vec<SyncSourceV1>,
    pub query: Option<String>,
    pub include_remote_removed: bool,
    pub sort: LibraryContentSortV1,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SearchRemoteStateV1 {
    Active,
    Removed,
    All,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchContentQueryV1 {
    pub account_scope: AccountScopeV1,
    pub sources: Vec<SyncSourceV1>,
    pub query: Option<String>,
    pub content_types: Vec<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub remote_state: SearchRemoteStateV1,
    pub local_folder_id: Option<i64>,
    pub sort: LibraryContentSortV1,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryContentRecordViewV1 {
    pub id: i64,
    pub account_id: i64,
    pub account_key: String,
    pub account_display_name: String,
    pub source: SyncSourceV1,
    pub aweme_id: String,
    pub url: String,
    pub content_type: String,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub cover_url: Option<String>,
    pub cached_cover_url: Option<String>,
    pub like_count: Option<i64>,
    pub source_position: Option<i64>,
    pub history_time_ms: Option<i64>,
    pub watched_at: Option<String>,
    pub watched_date: Option<String>,
    pub watch_progress_rate: Option<f64>,
    pub remote_created_at_ms: Option<i64>,
    pub first_archived_at: String,
    pub last_seen_at: Option<String>,
    pub remote_removed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListLibraryContentResultV1 {
    pub records: Vec<LibraryContentRecordViewV1>,
    pub total: i64,
}

#[derive(Debug)]
struct LibraryContentRow {
    id: i64,
    account_id: i64,
    uid: Option<String>,
    sec_uid: Option<String>,
    nickname: Option<String>,
    source: String,
    aweme_id: String,
    url: String,
    content_type: String,
    title: Option<String>,
    author_name: Option<String>,
    cover_url: Option<String>,
    like_count: Option<i64>,
    source_position: Option<i64>,
    history_time_ms: Option<i64>,
    watched_at: Option<String>,
    watched_date: Option<String>,
    watch_progress_rate: Option<f64>,
    remote_created_at_ms: Option<i64>,
    first_archived_at: String,
    last_seen_at: Option<String>,
    remote_removed_at: Option<String>,
}

pub fn list_library_content(
    connection: &Connection,
    query: &LibraryContentQueryV1,
) -> Result<ListLibraryContentResultV1, String> {
    let filters = LibraryQueryFilters {
        content_types: Vec::new(),
        date_from: None,
        date_to: None,
        remote_state: if query.include_remote_removed {
            SearchRemoteStateV1::All
        } else {
            SearchRemoteStateV1::Active
        },
        local_folder_id: None,
    };
    list_library_content_with_filters(connection, query, &filters)
}

pub fn search_library_content(
    connection: &Connection,
    query: &SearchContentQueryV1,
) -> Result<ListLibraryContentResultV1, String> {
    let filters = validate_search_filters(connection, query)?;
    let library_query = LibraryContentQueryV1 {
        account_scope: query.account_scope.clone(),
        sources: query.sources.clone(),
        query: query.query.clone(),
        include_remote_removed: query.remote_state == SearchRemoteStateV1::All,
        sort: query.sort,
        limit: query.limit,
        offset: query.offset,
    };
    list_library_content_with_filters(connection, &library_query, &filters)
}

#[derive(Debug)]
struct LibraryQueryFilters {
    content_types: Vec<String>,
    date_from: Option<String>,
    date_to: Option<String>,
    remote_state: SearchRemoteStateV1,
    local_folder_id: Option<i64>,
}

fn list_library_content_with_filters(
    connection: &Connection,
    query: &LibraryContentQueryV1,
    filters: &LibraryQueryFilters,
) -> Result<ListLibraryContentResultV1, String> {
    validate_account_scope(&query.account_scope)?;
    let account_ids = resolve_library_account_ids(connection, &query.account_scope)?;
    let sources = validate_library_sources(&query.sources)?;
    let order_by = library_order_by(query.sort, &sources)?;
    if account_ids.is_empty() {
        return Ok(ListLibraryContentResultV1 {
            records: Vec::new(),
            total: 0,
        });
    }

    let union_sql = library_union_sql(&sources);
    let account_placeholders = std::iter::repeat("?")
        .take(account_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let cte_sql = format!("WITH content_relations AS ({union_sql})");
    let mut predicates = vec![
        format!("relations.account_id IN ({account_placeholders})"),
        "(? IS NULL OR COALESCE(relations.title, '') || ' ' ||
          COALESCE(relations.author_name, '') LIKE ?)"
            .to_string(),
    ];
    match filters.remote_state {
        SearchRemoteStateV1::Active => {
            predicates.push("relations.remote_removed_at IS NULL".to_string())
        }
        SearchRemoteStateV1::Removed => {
            predicates.push("relations.remote_removed_at IS NOT NULL".to_string())
        }
        SearchRemoteStateV1::All => {}
    }
    if !filters.content_types.is_empty() {
        let placeholders = std::iter::repeat("?")
            .take(filters.content_types.len())
            .collect::<Vec<_>>()
            .join(", ");
        predicates.push(format!("relations.content_type IN ({placeholders})"));
    }
    if filters.date_from.is_some() {
        predicates.push("relations.effective_date >= ?".to_string());
    }
    if filters.date_to.is_some() {
        predicates.push("relations.effective_date <= ?".to_string());
    }
    if filters.local_folder_id.is_some() {
        predicates.push(
            "EXISTS (
               SELECT 1
               FROM local_folder_items folder_items
               JOIN local_folders folders ON folders.id = folder_items.folder_id
               LEFT JOIN local_folder_item_accounts provenance
                 ON provenance.folder_id = folder_items.folder_id
                AND provenance.video_id = folder_items.video_id
                AND provenance.account_id = relations.account_id
               WHERE folder_items.folder_id = ?
                 AND folder_items.video_id = relations.video_id
                 AND ((folders.scope = 'account' AND
                       folders.owner_account_id = relations.account_id) OR
                      (folders.scope = 'aggregate' AND provenance.account_id IS NOT NULL))
             )"
            .to_string(),
        );
    }
    let filtered_sql = format!(
        "FROM content_relations relations
         JOIN accounts ON accounts.id = relations.account_id
         WHERE {}",
        predicates.join(" AND ")
    );
    let filter_values = library_filter_values(&account_ids, query.query.as_deref(), filters);
    let count_sql = format!("{cte_sql} SELECT COUNT(*) {filtered_sql}");
    let total = connection
        .query_row(&count_sql, params_from_iter(filter_values.iter()), |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;

    let list_sql = format!(
        "{cte_sql}
         SELECT relations.id, relations.account_id, accounts.uid, accounts.sec_uid,
                accounts.nickname, relations.source, relations.aweme_id, relations.url,
                relations.content_type, relations.title, relations.author_name,
                relations.cover_url, relations.like_count, relations.source_position,
                relations.history_time_ms, relations.watched_at, relations.watched_date,
                relations.watch_progress_rate, relations.remote_created_at_ms,
                relations.first_archived_at, relations.last_seen_at,
                relations.remote_removed_at
         {filtered_sql}
         ORDER BY {order_by}
         LIMIT ? OFFSET ?"
    );
    let mut list_values = filter_values;
    list_values.push(Value::Integer(query.limit.clamp(1, 200) as i64));
    list_values.push(Value::Integer(query.offset as i64));
    let mut statement = connection
        .prepare(&list_sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(list_values.iter()), |row| {
            Ok(LibraryContentRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                uid: row.get(2)?,
                sec_uid: row.get(3)?,
                nickname: row.get(4)?,
                source: row.get(5)?,
                aweme_id: row.get(6)?,
                url: row.get(7)?,
                content_type: row.get(8)?,
                title: row.get(9)?,
                author_name: row.get(10)?,
                cover_url: row.get(11)?,
                like_count: row.get(12)?,
                source_position: row.get(13)?,
                history_time_ms: row.get(14)?,
                watched_at: row.get(15)?,
                watched_date: row.get(16)?,
                watch_progress_rate: row.get(17)?,
                remote_created_at_ms: row.get(18)?,
                first_archived_at: row.get(19)?,
                last_seen_at: row.get(20)?,
                remote_removed_at: row.get(21)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut records = rows
        .into_iter()
        .map(library_record_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    attach_library_cached_cover_urls(connection, &mut records)?;
    Ok(ListLibraryContentResultV1 { records, total })
}

fn resolve_library_account_ids(
    connection: &Connection,
    scope: &AccountScopeV1,
) -> Result<Vec<i64>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id FROM accounts
             WHERE status IN ('active', 'login_expired', 'identity_conflict', 'archived')
             ORDER BY id",
        )
        .map_err(|error| error.to_string())?;
    let readable_ids = statement
        .query_map([], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<i64>, _>>()
        .map_err(|error| error.to_string())?;
    match scope {
        AccountScopeV1::All => Ok(readable_ids),
        AccountScopeV1::Selected { account_ids } => {
            if account_ids
                .iter()
                .all(|account_id| readable_ids.binary_search(account_id).is_ok())
            {
                Ok(account_ids.clone())
            } else {
                Err("account_scope_invalid".to_string())
            }
        }
    }
}

fn validate_library_sources(sources: &[SyncSourceV1]) -> Result<Vec<SyncSourceV1>, String> {
    if sources.is_empty() {
        return Err("invalid_content_sources".to_string());
    }
    let mut normalized = Vec::with_capacity(sources.len());
    for source in sources {
        if *source == SyncSourceV1::RemoteFolder || normalized.contains(source) {
            return Err("invalid_content_sources".to_string());
        }
        normalized.push(*source);
    }
    Ok(normalized)
}

fn library_union_sql(sources: &[SyncSourceV1]) -> String {
    let mut parts = Vec::with_capacity(sources.len());
    if sources.contains(&SyncSourceV1::History) {
        parts.push(
            "SELECT relations.id, relations.account_id, videos.id AS video_id,
                    'history' AS source,
                    videos.aweme_id, videos.canonical_url AS url, videos.content_type,
                    videos.title, videos.author_name, videos.cover_url, videos.like_count,
                    relations.source_position, relations.history_time_ms,
                    relations.watched_at, relations.watched_date,
                    relations.watch_progress_rate, NULL AS remote_created_at_ms,
                    relations.created_at AS first_archived_at,
                    relations.watched_date AS effective_date, NULL AS last_seen_at,
                    NULL AS remote_removed_at
             FROM watch_records relations
             JOIN videos ON videos.id = relations.video_id",
        );
    }
    for source in [SyncSourceV1::Likes, SyncSourceV1::Favorites] {
        if sources.contains(&source) {
            parts.push(if source == SyncSourceV1::Likes {
                "SELECT relations.id, relations.account_id, videos.id AS video_id,
                        'likes' AS source,
                        videos.aweme_id, videos.canonical_url AS url, videos.content_type,
                        videos.title, videos.author_name, videos.cover_url, videos.like_count,
                        relations.source_position, NULL AS history_time_ms,
                        NULL AS watched_at, NULL AS watched_date,
                        NULL AS watch_progress_rate, relations.remote_created_at_ms,
                        relations.first_seen_at AS first_archived_at,
                        SUBSTR(relations.first_seen_at, 1, 10) AS effective_date,
                        relations.last_seen_at,
                        relations.remote_removed_at
                 FROM user_likes relations
                 JOIN videos ON videos.id = relations.video_id"
            } else {
                "SELECT relations.id, relations.account_id, videos.id AS video_id,
                        'favorites' AS source,
                        videos.aweme_id, videos.canonical_url AS url, videos.content_type,
                        videos.title, videos.author_name, videos.cover_url, videos.like_count,
                        relations.source_position, NULL AS history_time_ms,
                        NULL AS watched_at, NULL AS watched_date,
                        NULL AS watch_progress_rate, relations.remote_created_at_ms,
                        relations.first_seen_at AS first_archived_at,
                        SUBSTR(relations.first_seen_at, 1, 10) AS effective_date,
                        relations.last_seen_at,
                        relations.remote_removed_at
                 FROM favorite_items relations
                 JOIN videos ON videos.id = relations.video_id"
            });
        }
    }
    parts.join(" UNION ALL ")
}

fn library_filter_values(
    account_ids: &[i64],
    query: Option<&str>,
    filters: &LibraryQueryFilters,
) -> Vec<Value> {
    let mut values = account_ids
        .iter()
        .copied()
        .map(Value::Integer)
        .collect::<Vec<_>>();
    let like_query = query
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("%{value}%"));
    values.push(like_query.clone().map(Value::Text).unwrap_or(Value::Null));
    values.push(like_query.map(Value::Text).unwrap_or(Value::Null));
    values.extend(filters.content_types.iter().cloned().map(Value::Text));
    if let Some(value) = &filters.date_from {
        values.push(Value::Text(value.clone()));
    }
    if let Some(value) = &filters.date_to {
        values.push(Value::Text(value.clone()));
    }
    if let Some(value) = filters.local_folder_id {
        values.push(Value::Integer(value));
    }
    values
}

fn validate_search_filters(
    connection: &Connection,
    query: &SearchContentQueryV1,
) -> Result<LibraryQueryFilters, String> {
    let mut content_types = query
        .content_types
        .iter()
        .map(|value| value.trim().to_string())
        .collect::<Vec<_>>();
    if content_types
        .iter()
        .any(|value| !matches!(value.as_str(), "video" | "note" | "unknown"))
    {
        return Err("invalid_content_types".to_string());
    }
    content_types.sort();
    content_types.dedup();

    let parse_date = |value: &Option<String>| -> Result<Option<String>, String> {
        let Some(value) = value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map(|_| Some(value.to_string()))
            .map_err(|_| "invalid_content_date".to_string())
    };
    let date_from = parse_date(&query.date_from)?;
    let date_to = parse_date(&query.date_to)?;
    if matches!((&date_from, &date_to), (Some(from), Some(to)) if from > to) {
        return Err("invalid_content_date".to_string());
    }

    if let Some(folder_id) = query.local_folder_id {
        let exists = folder_id > 0
            && connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM local_folders WHERE id = ?1)",
                    [folder_id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|error| error.to_string())?;
        if !exists {
            return Err("local_folder_not_found".to_string());
        }
    }

    Ok(LibraryQueryFilters {
        content_types,
        date_from,
        date_to,
        remote_state: query.remote_state,
        local_folder_id: query.local_folder_id,
    })
}

fn library_order_by(
    sort: LibraryContentSortV1,
    sources: &[SyncSourceV1],
) -> Result<&'static str, String> {
    let only_history = sources.len() == 1 && sources[0] == SyncSourceV1::History;
    let only_archive_sources = sources
        .iter()
        .all(|source| matches!(source, SyncSourceV1::Likes | SyncSourceV1::Favorites));
    match sort {
        LibraryContentSortV1::Default | LibraryContentSortV1::ArchivedNewest => {
            Ok("relations.first_archived_at DESC, relations.source ASC,
             relations.account_id ASC, relations.id DESC")
        }
        LibraryContentSortV1::WatchedNewest if only_history => {
            Ok("relations.history_time_ms DESC, relations.source_position ASC, relations.id DESC")
        }
        LibraryContentSortV1::WatchedOldest if only_history => {
            Ok("relations.history_time_ms ASC, relations.source_position ASC, relations.id ASC")
        }
        LibraryContentSortV1::PageOrder if sources.len() == 1 => Ok(
            "CASE WHEN relations.source_position IS NULL THEN 1 ELSE 0 END,
             relations.source_position ASC, relations.id DESC",
        ),
        LibraryContentSortV1::RemoteNewest if only_archive_sources => Ok(
            "CASE WHEN relations.remote_created_at_ms IS NULL THEN 1 ELSE 0 END,
             relations.remote_created_at_ms DESC, relations.source_position ASC,
             relations.id DESC",
        ),
        LibraryContentSortV1::RemoteOldest if only_archive_sources => Ok(
            "CASE WHEN relations.remote_created_at_ms IS NULL THEN 1 ELSE 0 END,
             relations.remote_created_at_ms ASC, relations.source_position ASC,
             relations.id ASC",
        ),
        LibraryContentSortV1::LikeCountDesc => {
            Ok("CASE WHEN relations.like_count IS NULL THEN 1 ELSE 0 END,
             relations.like_count DESC, relations.first_archived_at DESC,
             relations.id DESC")
        }
        LibraryContentSortV1::TitleAsc => Ok(
            "CASE WHEN NULLIF(TRIM(relations.title), '') IS NULL THEN 1 ELSE 0 END,
             relations.title COLLATE NOCASE ASC, relations.first_archived_at DESC,
             relations.id DESC",
        ),
        LibraryContentSortV1::AuthorAsc => Ok(
            "CASE WHEN NULLIF(TRIM(relations.author_name), '') IS NULL THEN 1 ELSE 0 END,
             relations.author_name COLLATE NOCASE ASC, relations.first_archived_at DESC,
             relations.id DESC",
        ),
        _ => Err("invalid_content_sort_for_sources".to_string()),
    }
}

fn library_record_from_row(row: LibraryContentRow) -> Result<LibraryContentRecordViewV1, String> {
    let account_key = portable_account_key(row.uid.as_deref(), row.sec_uid.as_deref())
        .map_err(|_| "account_scope_invalid".to_string())?;
    let account_display_name = row
        .nickname
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            row.uid
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            row.sec_uid
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .map(str::to_string)
        .unwrap_or_else(|| format!("账号 {}", row.account_id));
    let source = match row.source.as_str() {
        "history" => SyncSourceV1::History,
        "likes" => SyncSourceV1::Likes,
        "favorites" => SyncSourceV1::Favorites,
        _ => return Err("invalid_content_source".to_string()),
    };
    Ok(LibraryContentRecordViewV1 {
        id: row.id,
        account_id: row.account_id,
        account_key,
        account_display_name,
        source,
        aweme_id: row.aweme_id,
        url: row.url,
        content_type: row.content_type,
        title: row.title,
        author_name: row.author_name,
        cover_url: row.cover_url,
        cached_cover_url: None,
        like_count: row.like_count,
        source_position: row.source_position,
        history_time_ms: row.history_time_ms,
        watched_at: row.watched_at,
        watched_date: row.watched_date,
        watch_progress_rate: row.watch_progress_rate,
        remote_created_at_ms: row.remote_created_at_ms,
        first_archived_at: row.first_archived_at,
        last_seen_at: row.last_seen_at,
        remote_removed_at: row.remote_removed_at,
    })
}

fn attach_library_cached_cover_urls(
    connection: &Connection,
    records: &mut [LibraryContentRecordViewV1],
) -> Result<(), String> {
    for record in records {
        record.cached_cover_url =
            crate::cover_cache::cached_cover_url_for_aweme_id(connection, &record.aweme_id)?;
    }
    Ok(())
}

pub fn list_content(
    connection: &Connection,
    account_id: i64,
    source: SyncSourceV1,
    query: Option<&str>,
    include_remote_removed: bool,
    sort: ContentSortV1,
    limit: usize,
    offset: usize,
) -> Result<ListContentResult, String> {
    if account_id <= 0 {
        return Err("invalid_account_id".to_string());
    }
    let trimmed = query.unwrap_or_default().trim();
    let like_query = (!trimmed.is_empty()).then(|| format!("%{trimmed}%"));
    match source {
        SyncSourceV1::History => list_history_content(
            connection,
            account_id,
            like_query.as_deref(),
            sort,
            limit,
            offset,
        ),
        SyncSourceV1::Likes => list_archive_content(
            connection,
            account_id,
            source,
            "user_likes",
            like_query.as_deref(),
            include_remote_removed,
            sort,
            limit,
            offset,
        ),
        SyncSourceV1::Favorites => list_archive_content(
            connection,
            account_id,
            source,
            "favorite_items",
            like_query.as_deref(),
            include_remote_removed,
            sort,
            limit,
            offset,
        ),
        SyncSourceV1::RemoteFolder => Err("unsupported_content_source".to_string()),
    }
}

fn list_history_content(
    connection: &Connection,
    account_id: i64,
    like_query: Option<&str>,
    sort: ContentSortV1,
    limit: usize,
    offset: usize,
) -> Result<ListContentResult, String> {
    let order_by = history_order_by(sort)?;
    let total = connection
        .query_row(
            "SELECT COUNT(*)
             FROM watch_records relations
             JOIN videos ON videos.id = relations.video_id
             WHERE relations.account_id = ?1
               AND (?2 IS NULL OR videos.title LIKE ?2 OR videos.author_name LIKE ?2)",
            params![account_id, like_query],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let list_sql = format!(
        "SELECT relations.id, videos.aweme_id, videos.canonical_url,
                    videos.content_type, videos.title, videos.author_name, videos.cover_url,
                    videos.like_count, relations.source_position, relations.history_time_ms,
                    relations.watched_at, relations.watched_date, relations.watch_progress_rate
             FROM watch_records relations
             JOIN videos ON videos.id = relations.video_id
             WHERE relations.account_id = ?1
               AND (?2 IS NULL OR videos.title LIKE ?2 OR videos.author_name LIKE ?2)
             ORDER BY {order_by}
             LIMIT ?3 OFFSET ?4"
    );
    let mut statement = connection
        .prepare(&list_sql)
        .map_err(|error| error.to_string())?;
    let records = statement
        .query_map(
            params![
                account_id,
                like_query,
                limit.clamp(1, 200) as i64,
                offset as i64
            ],
            |row| {
                Ok(ContentRecordView {
                    id: row.get(0)?,
                    source: SyncSourceV1::History,
                    aweme_id: row.get(1)?,
                    url: row.get(2)?,
                    content_type: row.get(3)?,
                    title: row.get(4)?,
                    author_name: row.get(5)?,
                    cover_url: row.get(6)?,
                    cached_cover_url: None,
                    like_count: row.get(7)?,
                    source_position: row.get(8)?,
                    history_time_ms: row.get(9)?,
                    watched_at: row.get(10)?,
                    watched_date: row.get(11)?,
                    watch_progress_rate: row.get(12)?,
                    remote_created_at_ms: None,
                    first_seen_at: None,
                    last_seen_at: None,
                    remote_removed_at: None,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut records = records;
    attach_cached_cover_urls(connection, &mut records)?;
    Ok(ListContentResult { records, total })
}

#[allow(clippy::too_many_arguments)]
fn list_archive_content(
    connection: &Connection,
    account_id: i64,
    source: SyncSourceV1,
    relation_table: &'static str,
    like_query: Option<&str>,
    include_remote_removed: bool,
    sort: ContentSortV1,
    limit: usize,
    offset: usize,
) -> Result<ListContentResult, String> {
    debug_assert!(matches!(relation_table, "user_likes" | "favorite_items"));
    let order_by = archive_order_by(sort)?;
    let count_sql = format!(
        "SELECT COUNT(*)
         FROM {relation_table} relations
         JOIN videos ON videos.id = relations.video_id
         WHERE relations.account_id = ?1
           AND (?2 IS NULL OR videos.title LIKE ?2 OR videos.author_name LIKE ?2)
           AND (?3 = 1 OR relations.remote_removed_at IS NULL)"
    );
    let total = connection
        .query_row(
            &count_sql,
            params![account_id, like_query, i64::from(include_remote_removed)],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let list_sql = format!(
        "SELECT relations.id, videos.aweme_id, videos.canonical_url,
                videos.content_type, videos.title, videos.author_name, videos.cover_url,
                videos.like_count, relations.source_position, relations.remote_created_at_ms,
                relations.first_seen_at, relations.last_seen_at, relations.remote_removed_at
         FROM {relation_table} relations
         JOIN videos ON videos.id = relations.video_id
         WHERE relations.account_id = ?1
           AND (?2 IS NULL OR videos.title LIKE ?2 OR videos.author_name LIKE ?2)
           AND (?3 = 1 OR relations.remote_removed_at IS NULL)
         ORDER BY {order_by}
         LIMIT ?4 OFFSET ?5"
    );
    let mut statement = connection
        .prepare(&list_sql)
        .map_err(|error| error.to_string())?;
    let records = statement
        .query_map(
            params![
                account_id,
                like_query,
                i64::from(include_remote_removed),
                limit.clamp(1, 200) as i64,
                offset as i64
            ],
            |row| {
                Ok(ContentRecordView {
                    id: row.get(0)?,
                    source,
                    aweme_id: row.get(1)?,
                    url: row.get(2)?,
                    content_type: row.get(3)?,
                    title: row.get(4)?,
                    author_name: row.get(5)?,
                    cover_url: row.get(6)?,
                    cached_cover_url: None,
                    like_count: row.get(7)?,
                    source_position: row.get(8)?,
                    history_time_ms: None,
                    watched_at: None,
                    watched_date: None,
                    watch_progress_rate: None,
                    remote_created_at_ms: row.get(9)?,
                    first_seen_at: row.get(10)?,
                    last_seen_at: row.get(11)?,
                    remote_removed_at: row.get(12)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut records = records;
    attach_cached_cover_urls(connection, &mut records)?;
    Ok(ListContentResult { records, total })
}

fn history_order_by(sort: ContentSortV1) -> Result<&'static str, String> {
    match sort {
        ContentSortV1::Default | ContentSortV1::WatchedNewest => Ok(
            "relations.history_time_ms DESC, relations.source_position ASC, relations.id DESC",
        ),
        ContentSortV1::WatchedOldest => Ok(
            "relations.history_time_ms ASC, relations.source_position ASC, relations.id ASC",
        ),
        ContentSortV1::LikeCountDesc => Ok(
            "CASE WHEN videos.like_count IS NULL THEN 1 ELSE 0 END, videos.like_count DESC, \
             relations.history_time_ms DESC, relations.source_position ASC, relations.id DESC",
        ),
        ContentSortV1::TitleAsc => Ok(
            "CASE WHEN NULLIF(TRIM(videos.title), '') IS NULL THEN 1 ELSE 0 END, \
             videos.title COLLATE NOCASE ASC, relations.history_time_ms DESC, relations.id DESC",
        ),
        ContentSortV1::AuthorAsc => Ok(
            "CASE WHEN NULLIF(TRIM(videos.author_name), '') IS NULL THEN 1 ELSE 0 END, \
             videos.author_name COLLATE NOCASE ASC, relations.history_time_ms DESC, relations.id DESC",
        ),
        ContentSortV1::PageOrder
        | ContentSortV1::RemoteNewest
        | ContentSortV1::RemoteOldest => Err("invalid_content_sort_for_source".to_string()),
    }
}

fn archive_order_by(sort: ContentSortV1) -> Result<&'static str, String> {
    match sort {
        ContentSortV1::Default | ContentSortV1::PageOrder => Ok(
            "CASE WHEN relations.source_position IS NULL THEN 1 ELSE 0 END, \
             relations.source_position ASC, relations.remote_created_at_ms DESC, relations.id DESC",
        ),
        ContentSortV1::RemoteNewest => Ok(
            "CASE WHEN relations.remote_created_at_ms IS NULL THEN 1 ELSE 0 END, \
             relations.remote_created_at_ms DESC, relations.source_position ASC, relations.id DESC",
        ),
        ContentSortV1::RemoteOldest => Ok(
            "CASE WHEN relations.remote_created_at_ms IS NULL THEN 1 ELSE 0 END, \
             relations.remote_created_at_ms ASC, relations.source_position ASC, relations.id ASC",
        ),
        ContentSortV1::LikeCountDesc => Ok(
            "CASE WHEN videos.like_count IS NULL THEN 1 ELSE 0 END, videos.like_count DESC, \
             CASE WHEN relations.source_position IS NULL THEN 1 ELSE 0 END, \
             relations.source_position ASC, relations.id DESC",
        ),
        ContentSortV1::TitleAsc => Ok(
            "CASE WHEN NULLIF(TRIM(videos.title), '') IS NULL THEN 1 ELSE 0 END, \
             videos.title COLLATE NOCASE ASC, relations.source_position ASC, relations.id DESC",
        ),
        ContentSortV1::AuthorAsc => Ok(
            "CASE WHEN NULLIF(TRIM(videos.author_name), '') IS NULL THEN 1 ELSE 0 END, \
             videos.author_name COLLATE NOCASE ASC, relations.source_position ASC, relations.id DESC",
        ),
        ContentSortV1::WatchedNewest | ContentSortV1::WatchedOldest => {
            Err("invalid_content_sort_for_source".to_string())
        }
    }
}

fn attach_cached_cover_urls(
    connection: &Connection,
    records: &mut [ContentRecordView],
) -> Result<(), String> {
    for record in records {
        record.cached_cover_url =
            crate::cover_cache::cached_cover_url_for_aweme_id(connection, &record.aweme_id)?;
    }
    Ok(())
}
