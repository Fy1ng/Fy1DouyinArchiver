use crate::settings_store::{validate_account_scope, AccountScopeV1};
use crate::sync::SyncSourceV1;
use rusqlite::{params_from_iter, types::Value, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskSummaryKindV1 {
    SyncBatch,
    DownloadJob,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskSummaryStatusV1 {
    Queued,
    Running,
    Paused,
    Interrupted,
    Success,
    Partial,
    Failed,
    Completed,
    CompletedWithErrors,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskSummaryV1 {
    pub key: String,
    pub kind: TaskSummaryKindV1,
    pub id: i64,
    pub account_id: Option<i64>,
    pub source: Option<SyncSourceV1>,
    pub status: TaskSummaryStatusV1,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub item_count: i64,
    pub completed_count: i64,
    pub failed_count: i64,
    pub error_code: Option<String>,
    pub can_cancel: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskSummariesPageV1 {
    pub records: Vec<TaskSummaryV1>,
    pub total: u64,
}

pub fn list_task_summaries(
    connection: &Connection,
    account_scope: &AccountScopeV1,
    limit: usize,
    offset: usize,
) -> Result<TaskSummariesPageV1, String> {
    validate_account_scope(account_scope)?;
    if !(1..=200).contains(&limit) {
        return Err("invalid_task_page_limit".to_string());
    }
    let account_ids = resolve_account_ids(connection, account_scope)?;
    if account_ids.is_empty() {
        return Ok(TaskSummariesPageV1 {
            records: Vec::new(),
            total: 0,
        });
    }

    let mut records = list_sync_tasks(connection, &account_ids)?;
    records.extend(list_download_tasks(connection, &account_ids)?);
    records.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.key.cmp(&left.key))
    });
    let total = u64::try_from(records.len()).map_err(|_| "task_count_overflow".to_string())?;
    let records = records.into_iter().skip(offset).take(limit).collect();
    Ok(TaskSummariesPageV1 { records, total })
}

fn resolve_account_ids(
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

fn placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn list_sync_tasks(
    connection: &Connection,
    account_ids: &[i64],
) -> Result<Vec<TaskSummaryV1>, String> {
    let sql = format!(
        "SELECT b.id, b.status, b.created_at, b.finished_at,
                (SELECT CASE WHEN COUNT(DISTINCT account_id) = 1 THEN MIN(account_id) END
                   FROM sync_batch_targets WHERE batch_id = b.id),
                (SELECT CASE WHEN COUNT(DISTINCT source) = 1 THEN MIN(source) END
                   FROM sync_batch_targets WHERE batch_id = b.id),
                (SELECT COUNT(*) FROM sync_batch_jobs WHERE batch_id = b.id),
                (SELECT COUNT(*) FROM sync_batch_issues WHERE batch_id = b.id AND kind = 'conflict'),
                (SELECT COUNT(*) FROM sync_batch_issues WHERE batch_id = b.id AND kind = 'skipped')
         FROM sync_batches b
         WHERE EXISTS (SELECT 1 FROM sync_batch_targets t
                       WHERE t.batch_id = b.id AND t.account_id IN ({}))
         ORDER BY b.created_at DESC, b.id DESC",
        placeholders(account_ids.len())
    );
    let values = account_ids
        .iter()
        .copied()
        .map(Value::Integer)
        .collect::<Vec<_>>();
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            let id: i64 = row.get(0)?;
            let status: String = row.get(1)?;
            let source: Option<String> = row.get(5)?;
            Ok((
                id,
                parse_batch_status(&status),
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                source.map(|value| SyncSourceV1::from_sqlite(&value)),
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(
            |(
                id,
                status,
                created_at,
                finished_at,
                account_id,
                source,
                jobs,
                conflicts,
                skipped,
            )| {
                Ok(TaskSummaryV1 {
                    key: format!("sync:{id}"),
                    kind: TaskSummaryKindV1::SyncBatch,
                    id,
                    account_id,
                    source: source.transpose()?,
                    can_cancel: matches!(
                        status,
                        TaskSummaryStatusV1::Queued | TaskSummaryStatusV1::Running
                    ),
                    status,
                    created_at,
                    finished_at,
                    item_count: jobs + conflicts + skipped,
                    completed_count: if matches!(status, TaskSummaryStatusV1::Success) {
                        jobs
                    } else {
                        0
                    },
                    failed_count: if matches!(
                        status,
                        TaskSummaryStatusV1::Failed | TaskSummaryStatusV1::Partial
                    ) {
                        conflicts
                    } else {
                        0
                    },
                    error_code: None,
                })
            },
        )
        .collect()
}

fn list_download_tasks(
    connection: &Connection,
    account_ids: &[i64],
) -> Result<Vec<TaskSummaryV1>, String> {
    let sql = format!(
        "SELECT id, account_id, state, created_at, completed_at, candidate_count,
                completed_count, failed_count, error_code
         FROM download_jobs WHERE account_id IN ({})",
        placeholders(account_ids.len())
    );
    let values = account_ids
        .iter()
        .copied()
        .map(Value::Integer)
        .collect::<Vec<_>>();
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            let id: i64 = row.get(0)?;
            let state: String = row.get(2)?;
            Ok(TaskSummaryV1 {
                key: format!("download:{id}"),
                kind: TaskSummaryKindV1::DownloadJob,
                id,
                account_id: Some(row.get(1)?),
                source: None,
                status: parse_download_status(&state),
                created_at: row.get(3)?,
                finished_at: row.get(4)?,
                item_count: row.get(5)?,
                completed_count: row.get(6)?,
                failed_count: row.get(7)?,
                error_code: row.get(8)?,
                can_cancel: matches!(
                    state.as_str(),
                    "queued" | "running" | "paused" | "interrupted"
                ),
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>();
    rows.map_err(|error| error.to_string())
}

fn parse_batch_status(value: &str) -> TaskSummaryStatusV1 {
    match value {
        "queued" => TaskSummaryStatusV1::Queued,
        "running" => TaskSummaryStatusV1::Running,
        "success" => TaskSummaryStatusV1::Success,
        "partial" => TaskSummaryStatusV1::Partial,
        "failed" => TaskSummaryStatusV1::Failed,
        _ => TaskSummaryStatusV1::Cancelled,
    }
}

fn parse_download_status(value: &str) -> TaskSummaryStatusV1 {
    match value {
        "queued" => TaskSummaryStatusV1::Queued,
        "running" => TaskSummaryStatusV1::Running,
        "paused" => TaskSummaryStatusV1::Paused,
        "interrupted" => TaskSummaryStatusV1::Interrupted,
        "completed" => TaskSummaryStatusV1::Completed,
        "completed_with_errors" => TaskSummaryStatusV1::CompletedWithErrors,
        _ => TaskSummaryStatusV1::Cancelled,
    }
}
