use crate::settings_store::{validate_account_scope, AccountScopeV1};
use crate::sync::{SyncJobStatusV1, SyncSourceV1};
use rusqlite::{params, params_from_iter, types::Value, Connection};
use serde::{Deserialize, Serialize};

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OverviewSourceFilterV1 {
    All,
    History,
    Likes,
    Favorites,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OverviewRangeV1 {
    Last7Days,
    Last30Days,
    Last90Days,
    All,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverviewSourceCountsV1 {
    pub history: i64,
    pub likes: i64,
    pub favorites: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverviewTrendPointV1 {
    pub date: String,
    pub history: i64,
    pub likes: i64,
    pub favorites: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverviewAuthorCountV1 {
    pub author_name: String,
    pub content_count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverviewContentTypeCountV1 {
    pub content_type: String,
    pub content_count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverviewRecentSyncV1 {
    pub account_id: i64,
    pub source: SyncSourceV1,
    pub status: SyncJobStatusV1,
    pub finished_at: String,
    pub inserted_records: i64,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverviewViewV1 {
    pub unique_content_count: i64,
    pub source_counts: OverviewSourceCountsV1,
    pub undated_archive_count: i64,
    pub trend: Vec<OverviewTrendPointV1>,
    pub top_authors: Vec<OverviewAuthorCountV1>,
    pub content_types: Vec<OverviewContentTypeCountV1>,
    pub recent_sync: Vec<OverviewRecentSyncV1>,
}

pub fn query_overview_scope_at(
    connection: &Connection,
    account_scope: &AccountScopeV1,
    source: OverviewSourceFilterV1,
    range: OverviewRangeV1,
    include_remote_removed: bool,
    reference_time_ms: i64,
) -> Result<OverviewViewV1, String> {
    if reference_time_ms <= 0 {
        return Err("invalid_overview_reference_time".to_string());
    }
    validate_account_scope(account_scope)?;
    let account_ids = resolve_overview_account_ids(connection, account_scope)?;
    if account_ids.is_empty() {
        return Ok(OverviewViewV1 {
            unique_content_count: 0,
            source_counts: OverviewSourceCountsV1::default(),
            undated_archive_count: 0,
            trend: Vec::new(),
            top_authors: Vec::new(),
            content_types: Vec::new(),
            recent_sync: Vec::new(),
        });
    }
    let source_name = source_name(source);
    let cutoff = cutoff_ms(range, reference_time_ms)?;
    let removed = i64::from(include_remote_removed);
    let filter = scoped_filter(account_ids.len());
    let values = scoped_values(&account_ids, source_name, removed, cutoff);
    let unique_content_count = connection
        .query_row(
            &format!("{} SELECT COUNT(DISTINCT video_id) FROM relations WHERE {}", scoped_relations_cte(), filter),
            params_from_iter(values.clone()),
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let source_counts = scoped_source_counts(connection, &account_ids, source_name, removed, cutoff)?;
    let undated_archive_count = connection
        .query_row(
            &format!(
                "{} SELECT COUNT(*) FROM relations WHERE {} AND source IN ('likes','favorites') AND event_time_ms IS NULL",
                scoped_relations_cte(), filter
            ),
            params_from_iter(values.clone()),
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let trend = scoped_trend(connection, &account_ids, source_name, removed, cutoff)?;
    let top_authors = scoped_authors(connection, &account_ids, source_name, removed, cutoff)?;
    let content_types = scoped_types(connection, &account_ids, source_name, removed, cutoff)?;
    let recent_sync = scoped_recent_sync(connection, &account_ids)?;
    Ok(OverviewViewV1 {
        unique_content_count,
        source_counts,
        undated_archive_count,
        trend,
        top_authors,
        content_types,
        recent_sync,
    })
}

fn resolve_overview_account_ids(
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
    let readable = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    match scope {
        AccountScopeV1::All => Ok(readable),
        AccountScopeV1::Selected { account_ids }
            if account_ids
                .iter()
                .all(|account_id| readable.binary_search(account_id).is_ok()) =>
        {
            Ok(account_ids.clone())
        }
        AccountScopeV1::Selected { .. } => Err("account_scope_invalid".to_string()),
    }
}

fn scoped_relations_cte() -> &'static str {
    "WITH relations AS (
       SELECT wr.account_id, wr.video_id, 'history' AS source,
              wr.history_time_ms AS event_time_ms, wr.watched_date AS event_date,
              NULL AS remote_removed_at FROM watch_records wr
       UNION ALL
       SELECT ul.account_id, ul.video_id, 'likes' AS source,
              ul.remote_created_at_ms AS event_time_ms,
              CASE WHEN ul.remote_created_at_ms IS NULL THEN NULL ELSE
                date(ul.remote_created_at_ms / 1000, 'unixepoch', '+8 hours') END AS event_date,
              ul.remote_removed_at FROM user_likes ul
       UNION ALL
       SELECT fi.account_id, fi.video_id, 'favorites' AS source,
              fi.remote_created_at_ms AS event_time_ms,
              CASE WHEN fi.remote_created_at_ms IS NULL THEN NULL ELSE
                date(fi.remote_created_at_ms / 1000, 'unixepoch', '+8 hours') END AS event_date,
              fi.remote_removed_at FROM favorite_items fi
     )"
}

fn scoped_filter(account_count: usize) -> String {
    format!(
        "account_id IN ({}) AND (? = 'all' OR source = ?) AND
         (? = 1 OR remote_removed_at IS NULL) AND
         (? IS NULL OR event_time_ms >= ?)",
        std::iter::repeat("?")
            .take(account_count)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn scoped_values(
    account_ids: &[i64],
    source: &str,
    include_removed: i64,
    cutoff: Option<i64>,
) -> Vec<Value> {
    let mut values = account_ids.iter().copied().map(Value::Integer).collect::<Vec<_>>();
    values.extend([Value::Text(source.to_string()), Value::Text(source.to_string())]);
    values.push(Value::Integer(include_removed));
    let cutoff = cutoff.map(Value::Integer).unwrap_or(Value::Null);
    values.extend([cutoff.clone(), cutoff]);
    values
}

fn scoped_source_counts(
    connection: &Connection,
    account_ids: &[i64],
    source: &str,
    include_removed: i64,
    cutoff: Option<i64>,
) -> Result<OverviewSourceCountsV1, String> {
    let sql = format!("{} SELECT source, COUNT(*) FROM relations WHERE {} GROUP BY source", scoped_relations_cte(), scoped_filter(account_ids.len()));
    let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(scoped_values(account_ids, source, include_removed, cutoff)), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut result = OverviewSourceCountsV1::default();
    for (name, count) in rows {
        match name.as_str() {
            "history" => result.history = count,
            "likes" => result.likes = count,
            "favorites" => result.favorites = count,
            _ => return Err("invalid_overview_source".to_string()),
        }
    }
    Ok(result)
}

fn scoped_trend(
    connection: &Connection,
    account_ids: &[i64],
    source: &str,
    include_removed: i64,
    cutoff: Option<i64>,
) -> Result<Vec<OverviewTrendPointV1>, String> {
    let sql = format!(
        "{} SELECT event_date,
          SUM(CASE WHEN source = 'history' THEN 1 ELSE 0 END),
          SUM(CASE WHEN source = 'likes' THEN 1 ELSE 0 END),
          SUM(CASE WHEN source = 'favorites' THEN 1 ELSE 0 END)
         FROM relations WHERE {} AND event_date IS NOT NULL
         GROUP BY event_date ORDER BY event_date ASC",
        scoped_relations_cte(), scoped_filter(account_ids.len())
    );
    let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(scoped_values(account_ids, source, include_removed, cutoff)), |row| {
            Ok(OverviewTrendPointV1 { date: row.get(0)?, history: row.get(1)?, likes: row.get(2)?, favorites: row.get(3)? })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn scoped_authors(
    connection: &Connection,
    account_ids: &[i64],
    source: &str,
    include_removed: i64,
    cutoff: Option<i64>,
) -> Result<Vec<OverviewAuthorCountV1>, String> {
    let sql = format!(
        "{}, selected_videos AS (SELECT DISTINCT video_id FROM relations WHERE {})
         SELECT v.author_name, COUNT(*) FROM selected_videos JOIN videos v ON v.id = selected_videos.video_id
         WHERE NULLIF(TRIM(v.author_name), '') IS NOT NULL
         GROUP BY v.author_name ORDER BY COUNT(*) DESC, v.author_name COLLATE NOCASE ASC LIMIT 10",
        scoped_relations_cte(), scoped_filter(account_ids.len())
    );
    let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(scoped_values(account_ids, source, include_removed, cutoff)), |row| {
            Ok(OverviewAuthorCountV1 { author_name: row.get(0)?, content_count: row.get(1)? })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn scoped_types(
    connection: &Connection,
    account_ids: &[i64],
    source: &str,
    include_removed: i64,
    cutoff: Option<i64>,
) -> Result<Vec<OverviewContentTypeCountV1>, String> {
    let sql = format!(
        "{}, selected_videos AS (SELECT DISTINCT video_id FROM relations WHERE {})
         SELECT v.content_type, COUNT(*) FROM selected_videos JOIN videos v ON v.id = selected_videos.video_id
         GROUP BY v.content_type ORDER BY COUNT(*) DESC, v.content_type ASC",
        scoped_relations_cte(), scoped_filter(account_ids.len())
    );
    let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(scoped_values(account_ids, source, include_removed, cutoff)), |row| {
            Ok(OverviewContentTypeCountV1 { content_type: row.get(0)?, content_count: row.get(1)? })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn scoped_recent_sync(
    connection: &Connection,
    account_ids: &[i64],
) -> Result<Vec<OverviewRecentSyncV1>, String> {
    let placeholders = std::iter::repeat("?").take(account_ids.len()).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT account_id, source, status, finished_at, inserted_records, error_code FROM (
           SELECT account_id, source, status, finished_at, inserted_records, error_code,
             ROW_NUMBER() OVER (PARTITION BY account_id, source ORDER BY finished_at DESC, id DESC) AS source_rank
           FROM sync_jobs WHERE account_id IN ({placeholders})
             AND source IN ('history', 'likes', 'favorites')
             AND status IN ('success', 'partial', 'failed', 'cancelled')
         ) WHERE source_rank = 1 ORDER BY finished_at DESC, account_id ASC, source ASC"
    );
    let values = account_ids.iter().copied().map(Value::Integer).collect::<Vec<_>>();
    let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, Option<String>>(5)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    rows.into_iter().map(|(account_id, source, status, finished_at, inserted_records, error_code)| {
        Ok(OverviewRecentSyncV1 {
            account_id,
            source: SyncSourceV1::from_sqlite(&source)?,
            status: parse_sync_status(&status)?,
            finished_at,
            inserted_records,
            error_code,
        })
    }).collect()
}

pub fn query_overview_at(
    connection: &Connection,
    account_id: i64,
    source: OverviewSourceFilterV1,
    range: OverviewRangeV1,
    include_remote_removed: bool,
    reference_time_ms: i64,
) -> Result<OverviewViewV1, String> {
    if account_id <= 0 {
        return Err("invalid_account_id".to_string());
    }
    if reference_time_ms <= 0 {
        return Err("invalid_overview_reference_time".to_string());
    }
    let cutoff_ms = cutoff_ms(range, reference_time_ms)?;
    let source_name = source_name(source);
    let include_removed = i64::from(include_remote_removed);

    let unique_content_count = connection
        .query_row(
            &format!(
                "{} SELECT COUNT(DISTINCT video_id) FROM selected_relations",
                selected_relations_cte()
            ),
            params![account_id, source_name, include_removed, cutoff_ms],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    let mut source_counts = OverviewSourceCountsV1::default();
    {
        let sql = format!(
            "{} SELECT source, COUNT(*) FROM selected_relations GROUP BY source",
            selected_relations_cte()
        );
        let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(
                params![account_id, source_name, include_removed, cutoff_ms],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        for (source, count) in rows {
            match source.as_str() {
                "history" => source_counts.history = count,
                "likes" => source_counts.likes = count,
                "favorites" => source_counts.favorites = count,
                _ => return Err("invalid_overview_source".to_string()),
            }
        }
    }

    let undated_archive_count = connection
        .query_row(
            &format!(
                "{} SELECT COUNT(*) FROM relations
                 WHERE account_id = ?1
                   AND (?2 = 'all' OR source = ?2)
                   AND source IN ('likes', 'favorites')
                   AND event_time_ms IS NULL
                   AND (?3 = 1 OR remote_removed_at IS NULL)",
                relations_cte()
            ),
            params![account_id, source_name, include_removed],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    let trend = load_trend(
        connection,
        account_id,
        source_name,
        include_removed,
        cutoff_ms,
    )?;
    let top_authors = load_top_authors(
        connection,
        account_id,
        source_name,
        include_removed,
        cutoff_ms,
    )?;
    let content_types = load_content_types(
        connection,
        account_id,
        source_name,
        include_removed,
        cutoff_ms,
    )?;
    let recent_sync = load_recent_sync(connection, account_id)?;

    Ok(OverviewViewV1 {
        unique_content_count,
        source_counts,
        undated_archive_count,
        trend,
        top_authors,
        content_types,
        recent_sync,
    })
}

fn cutoff_ms(range: OverviewRangeV1, reference_time_ms: i64) -> Result<Option<i64>, String> {
    let days = match range {
        OverviewRangeV1::Last7Days => Some(7),
        OverviewRangeV1::Last30Days => Some(30),
        OverviewRangeV1::Last90Days => Some(90),
        OverviewRangeV1::All => None,
    };
    days.map(|days| {
        reference_time_ms
            .checked_sub(days * DAY_MS)
            .ok_or_else(|| "invalid_overview_reference_time".to_string())
    })
    .transpose()
}

fn source_name(source: OverviewSourceFilterV1) -> &'static str {
    match source {
        OverviewSourceFilterV1::All => "all",
        OverviewSourceFilterV1::History => "history",
        OverviewSourceFilterV1::Likes => "likes",
        OverviewSourceFilterV1::Favorites => "favorites",
    }
}

fn relations_cte() -> &'static str {
    "WITH relations AS (
       SELECT relations.account_id, relations.video_id, 'history' AS source,
              relations.history_time_ms AS event_time_ms,
              relations.watched_date AS event_date,
              NULL AS remote_removed_at
       FROM watch_records relations
       UNION ALL
       SELECT relations.account_id, relations.video_id, 'likes' AS source,
              relations.remote_created_at_ms AS event_time_ms,
              CASE WHEN relations.remote_created_at_ms IS NULL THEN NULL
                   ELSE date(relations.remote_created_at_ms / 1000, 'unixepoch', '+8 hours') END AS event_date,
              relations.remote_removed_at
       FROM user_likes relations
       UNION ALL
       SELECT relations.account_id, relations.video_id, 'favorites' AS source,
              relations.remote_created_at_ms AS event_time_ms,
              CASE WHEN relations.remote_created_at_ms IS NULL THEN NULL
                   ELSE date(relations.remote_created_at_ms / 1000, 'unixepoch', '+8 hours') END AS event_date,
              relations.remote_removed_at
       FROM favorite_items relations
     )"
}

fn selected_relations_cte() -> String {
    format!(
        "{}, selected_relations AS (
           SELECT * FROM relations
           WHERE account_id = ?1
             AND (?2 = 'all' OR source = ?2)
             AND (?3 = 1 OR remote_removed_at IS NULL)
             AND (?4 IS NULL OR event_time_ms >= ?4)
         )",
        relations_cte()
    )
}

fn load_trend(
    connection: &Connection,
    account_id: i64,
    source_name: &str,
    include_removed: i64,
    cutoff_ms: Option<i64>,
) -> Result<Vec<OverviewTrendPointV1>, String> {
    let sql = format!(
        "{} SELECT event_date,
                SUM(CASE WHEN source = 'history' THEN 1 ELSE 0 END),
                SUM(CASE WHEN source = 'likes' THEN 1 ELSE 0 END),
                SUM(CASE WHEN source = 'favorites' THEN 1 ELSE 0 END)
         FROM selected_relations
         WHERE event_date IS NOT NULL
         GROUP BY event_date ORDER BY event_date ASC",
        selected_relations_cte()
    );
    let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
    let records = statement
        .query_map(
            params![account_id, source_name, include_removed, cutoff_ms],
            |row| {
                Ok(OverviewTrendPointV1 {
                    date: row.get(0)?,
                    history: row.get(1)?,
                    likes: row.get(2)?,
                    favorites: row.get(3)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(records)
}

fn load_top_authors(
    connection: &Connection,
    account_id: i64,
    source_name: &str,
    include_removed: i64,
    cutoff_ms: Option<i64>,
) -> Result<Vec<OverviewAuthorCountV1>, String> {
    let sql = format!(
        "{}, selected_videos AS (SELECT DISTINCT video_id FROM selected_relations)
         SELECT videos.author_name, COUNT(*)
         FROM selected_videos
         JOIN videos ON videos.id = selected_videos.video_id
         WHERE NULLIF(TRIM(videos.author_name), '') IS NOT NULL
         GROUP BY videos.author_name
         ORDER BY COUNT(*) DESC, videos.author_name COLLATE NOCASE ASC
         LIMIT 10",
        selected_relations_cte()
    );
    let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
    let records = statement
        .query_map(
            params![account_id, source_name, include_removed, cutoff_ms],
            |row| {
                Ok(OverviewAuthorCountV1 {
                    author_name: row.get(0)?,
                    content_count: row.get(1)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(records)
}

fn load_content_types(
    connection: &Connection,
    account_id: i64,
    source_name: &str,
    include_removed: i64,
    cutoff_ms: Option<i64>,
) -> Result<Vec<OverviewContentTypeCountV1>, String> {
    let sql = format!(
        "{}, selected_videos AS (SELECT DISTINCT video_id FROM selected_relations)
         SELECT videos.content_type, COUNT(*)
         FROM selected_videos
         JOIN videos ON videos.id = selected_videos.video_id
         GROUP BY videos.content_type
         ORDER BY COUNT(*) DESC, videos.content_type ASC",
        selected_relations_cte()
    );
    let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
    let records = statement
        .query_map(
            params![account_id, source_name, include_removed, cutoff_ms],
            |row| {
                Ok(OverviewContentTypeCountV1 {
                    content_type: row.get(0)?,
                    content_count: row.get(1)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(records)
}

fn load_recent_sync(
    connection: &Connection,
    account_id: i64,
) -> Result<Vec<OverviewRecentSyncV1>, String> {
    let mut statement = connection
        .prepare(
            "SELECT account_id, source, status, finished_at, inserted_records, error_code
             FROM (
               SELECT account_id, source, status, finished_at, inserted_records, error_code,
                      ROW_NUMBER() OVER (
                        PARTITION BY source
                        ORDER BY finished_at DESC, id DESC
                      ) AS source_rank
               FROM sync_jobs
               WHERE account_id = ?1
                 AND source IN ('history', 'likes', 'favorites')
                 AND status IN ('success', 'partial', 'failed', 'cancelled')
             )
             WHERE source_rank = 1
             ORDER BY finished_at DESC, source ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([account_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(
            |(account_id, source, status, finished_at, inserted_records, error_code)| {
                Ok(OverviewRecentSyncV1 {
                    account_id,
                    source: SyncSourceV1::from_sqlite(&source)?,
                    status: parse_sync_status(&status)?,
                    finished_at,
                    inserted_records,
                    error_code,
                })
            },
        )
        .collect()
}

fn parse_sync_status(value: &str) -> Result<SyncJobStatusV1, String> {
    match value {
        "success" => Ok(SyncJobStatusV1::Success),
        "partial" => Ok(SyncJobStatusV1::Partial),
        "failed" => Ok(SyncJobStatusV1::Failed),
        "cancelled" => Ok(SyncJobStatusV1::Cancelled),
        _ => Err("invalid_sync_job_status".to_string()),
    }
}
