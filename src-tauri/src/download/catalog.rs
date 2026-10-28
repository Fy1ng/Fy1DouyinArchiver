use rusqlite::{params_from_iter, types::Value, Connection};
use serde::{Deserialize, Serialize};

use super::{DownloadCandidate, DownloadError};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DownloadSelection {
    Checked { watch_record_ids: Vec<i64> },
    Filtered { query: Option<String> },
    All,
}

pub fn expand_candidates(
    connection: &Connection,
    account_id: i64,
    selection: &DownloadSelection,
) -> Result<Vec<DownloadCandidate>, DownloadError> {
    let (predicate, parameters, filter_context) = selection_filter(account_id, selection);
    let sql = format!(
        r#"
        WITH ranked_candidates AS (
          SELECT
            v.aweme_id,
            v.canonical_url,
            v.content_type,
            v.title,
            v.author_name,
            wr.history_time_ms,
            wr.source_position,
            wr.id AS watch_record_id,
            ROW_NUMBER() OVER (
              PARTITION BY v.aweme_id
              ORDER BY wr.history_time_ms DESC, wr.source_position ASC, wr.id DESC
            ) AS candidate_rank
          FROM watch_records wr
          JOIN videos v ON v.id = wr.video_id
          {predicate}
        )
        SELECT aweme_id, canonical_url, content_type, title, author_name
        FROM ranked_candidates
        WHERE candidate_rank = 1
        ORDER BY history_time_ms DESC, source_position ASC, watch_record_id DESC, aweme_id ASC
        "#
    );
    let mut statement = connection.prepare(&sql).map_err(catalog_error)?;
    let rows = statement
        .query_map(params_from_iter(parameters.iter()), |row| {
            Ok(DownloadCandidate {
                aweme_id: row.get(0)?,
                canonical_url: row.get(1)?,
                content_type: row.get(2)?,
                title: row.get(3)?,
                author_name: row.get(4)?,
                published_at: None,
                filter_context: filter_context.clone(),
            })
        })
        .map_err(catalog_error)?;

    rows.collect::<Result<Vec<_>, _>>().map_err(catalog_error)
}

fn selection_filter(
    account_id: i64,
    selection: &DownloadSelection,
) -> (String, Vec<Value>, Option<String>) {
    match selection {
        DownloadSelection::Checked { watch_record_ids } if watch_record_ids.is_empty() => {
            (
                "WHERE wr.account_id = ?1 AND 0".to_string(),
                vec![Value::Integer(account_id)],
                None,
            )
        }
        DownloadSelection::Checked { watch_record_ids } => {
            let placeholders = (2..=watch_record_ids.len() + 1)
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let mut parameters = vec![Value::Integer(account_id)];
            parameters.extend(
                watch_record_ids
                    .iter()
                    .copied()
                    .map(Value::Integer),
            );
            (
                format!("WHERE wr.account_id = ?1 AND wr.id IN ({placeholders})"),
                parameters,
                None,
            )
        }
        DownloadSelection::Filtered { query } => {
            let trimmed = query
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            match trimmed {
                Some(value) => (
                    "WHERE wr.account_id = ?1 AND (v.title LIKE ?2 OR v.author_name LIKE ?2)"
                        .to_string(),
                    vec![
                        Value::Integer(account_id),
                        Value::Text(format!("%{value}%")),
                    ],
                    Some(value.to_string()),
                ),
                None => (
                    "WHERE wr.account_id = ?1".to_string(),
                    vec![Value::Integer(account_id)],
                    None,
                ),
            }
        }
        DownloadSelection::All => (
            "WHERE wr.account_id = ?1".to_string(),
            vec![Value::Integer(account_id)],
            None,
        ),
    }
}

fn catalog_error(_error: rusqlite::Error) -> DownloadError {
    DownloadError {
        code: "download_catalog_failed".to_string(),
        message: "读取下载候选失败".to_string(),
    }
}
