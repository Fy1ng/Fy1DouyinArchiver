use crate::{
    device_settings::{notification_failure_key, DeviceSettingsStore},
    sync::{StoredSyncJobV1, SyncJobStatusV1, SyncTriggerV1, TaskKeyV1},
};
use chrono::DateTime;
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub fn consecutive_automatic_failure_job_ids(
    connection: &Connection,
    task_key: &TaskKeyV1,
) -> Result<Vec<i64>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, status
             FROM sync_jobs
             WHERE account_id = ?1 AND source = ?2 AND source_instance_key = ?3
               AND trigger = 'automatic'
               AND status IN ('success', 'partial', 'failed', 'cancelled')
             ORDER BY finished_at DESC, id DESC
             LIMIT 4",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                task_key.account_id,
                task_key.source.sqlite_name(),
                task_key.source_instance_key,
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let mut failures = Vec::new();
    for row in rows {
        let (job_id, status) = row.map_err(|error| error.to_string())?;
        if status != "failed" {
            break;
        }
        failures.push(job_id);
        if failures.len() == 3 {
            break;
        }
    }
    Ok(failures)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKindV1 {
    LoginExpired,
    AutomaticFailureStreak,
    BackupFailed,
    BackupCompleted,
    FirstTrayHide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationMessageV1 {
    pub title: &'static str,
    pub body: &'static str,
}

pub fn notification_message(kind: NotificationKindV1) -> NotificationMessageV1 {
    match kind {
        NotificationKindV1::LoginExpired => NotificationMessageV1 {
            title: "抖音登录已失效",
            body: "自动恢复登录失败，请打开应用重新登录。",
        },
        NotificationKindV1::AutomaticFailureStreak => NotificationMessageV1 {
            title: "自动同步连续失败",
            body: "同一同步任务已连续失败 3 次，请打开应用查看。",
        },
        NotificationKindV1::BackupFailed => NotificationMessageV1 {
            title: "自动备份失败",
            body: "未能完成自动备份，请打开应用检查备份目录和设置。",
        },
        NotificationKindV1::BackupCompleted => NotificationMessageV1 {
            title: "自动备份完成",
            body: "数据库与设置已成功备份。",
        },
        NotificationKindV1::FirstTrayHide => NotificationMessageV1 {
            title: "应用仍在后台运行",
            body: "可通过系统托盘重新打开主界面或退出应用。",
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationPolicyDecisionV1 {
    Keep,
    Reset,
    Notify(NotificationKindV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalNotificationInputV1 {
    pub task_key: TaskKeyV1,
    pub job_id: i64,
    pub trigger: SyncTriggerV1,
    pub status: SyncJobStatusV1,
    pub error_code: Option<String>,
    pub consecutive_failure_job_ids: Vec<i64>,
    pub prior_notified_job_id: Option<i64>,
}

pub fn terminal_notification_decision(
    input: &TerminalNotificationInputV1,
) -> NotificationPolicyDecisionV1 {
    if matches!(
        input.status,
        SyncJobStatusV1::Success | SyncJobStatusV1::Partial | SyncJobStatusV1::Cancelled
    ) {
        return NotificationPolicyDecisionV1::Reset;
    }
    if input.status != SyncJobStatusV1::Failed || input.trigger != SyncTriggerV1::Automatic {
        return NotificationPolicyDecisionV1::Keep;
    }

    if input.error_code.as_deref() == Some("login_recovery_failed") {
        return if input.prior_notified_job_id.is_some() {
            NotificationPolicyDecisionV1::Keep
        } else {
            NotificationPolicyDecisionV1::Notify(NotificationKindV1::LoginExpired)
        };
    }

    let is_third_failure = input.consecutive_failure_job_ids.len() >= 3
        && input.consecutive_failure_job_ids.first() == Some(&input.job_id);
    let already_notified = input.prior_notified_job_id.is_some();
    if is_third_failure && !already_notified {
        NotificationPolicyDecisionV1::Notify(NotificationKindV1::AutomaticFailureStreak)
    } else {
        NotificationPolicyDecisionV1::Keep
    }
}

pub fn observe_terminal_notification(
    connection: &Connection,
    settings: &DeviceSettingsStore,
    job: &StoredSyncJobV1,
    mut notify: impl FnMut(NotificationKindV1) -> Result<(), String>,
) -> Result<(), String> {
    let task_key = &job.snapshot.task_key;
    let key = notification_failure_key(
        task_key.account_id,
        task_key.source.sqlite_name(),
        &task_key.source_instance_key,
    )
    .map_err(|_| "notification_state_update_failed".to_string())?;
    let prior_notified_job_id = settings
        .load()
        .map_err(|_| "notification_state_update_failed".to_string())?
        .notified_failure_streaks
        .get(&key)
        .copied();
    let input = TerminalNotificationInputV1 {
        task_key: task_key.clone(),
        job_id: job.snapshot.id,
        trigger: job.snapshot.trigger,
        status: job.snapshot.status,
        error_code: job.error_code.clone(),
        consecutive_failure_job_ids: consecutive_automatic_failure_job_ids(connection, task_key)
            .map_err(|_| "notification_query_failed".to_string())?,
        prior_notified_job_id,
    };
    match terminal_notification_decision(&input) {
        NotificationPolicyDecisionV1::Keep => Ok(()),
        NotificationPolicyDecisionV1::Reset => {
            if prior_notified_job_id.is_some() {
                settings
                    .set_notified_failure(&key, None)
                    .map_err(|_| "notification_state_update_failed".to_string())?;
            }
            let resolved_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            for kind in [
                NotificationKindV1::LoginExpired,
                NotificationKindV1::AutomaticFailureStreak,
            ] {
                resolve_notification_by_dedupe(
                    connection,
                    &sync_notification_dedupe_key(&key, kind),
                    &resolved_at,
                )?;
            }
            Ok(())
        }
        NotificationPolicyDecisionV1::Notify(kind) => {
            let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            upsert_notification(
                connection,
                notification_insert_for_kind(
                    kind,
                    sync_notification_dedupe_key(&key, kind),
                    Some(task_key.account_id),
                    Some("syncJob"),
                    Some(job.snapshot.id.to_string()),
                    created_at,
                ),
            )?;
            settings
                .set_notified_failure(&key, Some(job.snapshot.id))
                .map_err(|_| "notification_state_update_failed".to_string())?;
            notify(kind).map_err(|_| "notification_delivery_failed".to_string())
        }
    }
}

pub fn notification_insert_for_kind(
    kind: NotificationKindV1,
    dedupe_key: String,
    account_id: Option<i64>,
    related_type: Option<&str>,
    related_id: Option<String>,
    created_at: String,
) -> NotificationInsertV1 {
    let message = notification_message(kind);
    NotificationInsertV1 {
        kind: notification_kind_name(kind).to_string(),
        severity: notification_kind_severity(kind),
        dedupe_key,
        title: message.title.to_string(),
        body: message.body.to_string(),
        account_id,
        related_type: related_type.map(str::to_string),
        related_id,
        created_at,
    }
}

pub fn notification_kind_name(kind: NotificationKindV1) -> &'static str {
    match kind {
        NotificationKindV1::LoginExpired => "loginExpired",
        NotificationKindV1::AutomaticFailureStreak => "automaticFailureStreak",
        NotificationKindV1::BackupFailed => "backupFailed",
        NotificationKindV1::BackupCompleted => "backupCompleted",
        NotificationKindV1::FirstTrayHide => "firstTrayHide",
    }
}

pub fn notification_kind_severity(kind: NotificationKindV1) -> NotificationSeverityV1 {
    match kind {
        NotificationKindV1::LoginExpired | NotificationKindV1::BackupFailed => {
            NotificationSeverityV1::Error
        }
        NotificationKindV1::AutomaticFailureStreak => NotificationSeverityV1::Warning,
        NotificationKindV1::BackupCompleted | NotificationKindV1::FirstTrayHide => {
            NotificationSeverityV1::Info
        }
    }
}

fn sync_notification_dedupe_key(key: &str, kind: NotificationKindV1) -> String {
    format!("sync:{key}:{}", notification_kind_name(kind))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSeverityV1 {
    Info,
    Warning,
    Error,
}

impl NotificationSeverityV1 {
    fn sqlite_name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "info" => Ok(Self::Info),
            "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            _ => Err("invalid_notification_severity".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationInsertV1 {
    pub kind: String,
    pub severity: NotificationSeverityV1,
    pub dedupe_key: String,
    pub title: String,
    pub body: String,
    pub account_id: Option<i64>,
    pub related_type: Option<String>,
    pub related_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationViewV1 {
    pub id: i64,
    pub kind: String,
    pub severity: NotificationSeverityV1,
    pub dedupe_key: String,
    pub title: String,
    pub body: String,
    pub account_id: Option<i64>,
    pub related_type: Option<String>,
    pub related_id: Option<String>,
    pub created_at: String,
    pub read_at: Option<String>,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsPageV1 {
    pub records: Vec<NotificationViewV1>,
    pub total: i64,
    pub unread: i64,
}

pub fn upsert_notification(
    connection: &Connection,
    value: NotificationInsertV1,
) -> Result<NotificationViewV1, String> {
    validate_notification_insert(&value)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO notifications(
               kind, severity, dedupe_key, title, body, account_id,
               related_type, related_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                value.kind,
                value.severity.sqlite_name(),
                value.dedupe_key,
                value.title,
                value.body,
                value.account_id,
                value.related_type,
                value.related_id,
                value.created_at,
            ],
        )
        .map_err(|error| format!("notification_write_failed:{error}"))?;
    load_unresolved_notification_by_dedupe(connection, &value.dedupe_key)?
        .ok_or_else(|| "notification_write_failed".to_string())
}

pub fn list_notifications(
    connection: &Connection,
    include_resolved: bool,
    limit: usize,
    offset: usize,
) -> Result<NotificationsPageV1, String> {
    if limit == 0 || limit > 200 || offset > i64::MAX as usize {
        return Err("invalid_notification_page".to_string());
    }
    let where_clause = if include_resolved {
        ""
    } else {
        "WHERE resolved_at IS NULL"
    };
    let total = connection
        .query_row(
            &format!("SELECT COUNT(*) FROM notifications {where_clause}"),
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let unread = connection
        .query_row(
            "SELECT COUNT(*) FROM notifications WHERE resolved_at IS NULL AND read_at IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT id, kind, severity, dedupe_key, title, body, account_id,
                    related_type, related_id, created_at, read_at, resolved_at
             FROM notifications {where_clause}
             ORDER BY created_at DESC, id DESC LIMIT ?1 OFFSET ?2"
        ))
        .map_err(|error| error.to_string())?;
    let records = statement
        .query_map(params![limit as i64, offset as i64], notification_from_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(NotificationsPageV1 {
        records,
        total,
        unread,
    })
}

pub fn mark_notification_read(
    connection: &Connection,
    notification_id: i64,
    read: bool,
    changed_at: &str,
) -> Result<NotificationViewV1, String> {
    validate_notification_id(notification_id)?;
    validate_notification_timestamp(changed_at)?;
    let changed = connection
        .execute(
            "UPDATE notifications SET read_at = CASE WHEN ?2 THEN ?3 ELSE NULL END WHERE id = ?1",
            params![notification_id, read, changed_at],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("notification_not_found".to_string());
    }
    load_notification(connection, notification_id)?
        .ok_or_else(|| "notification_not_found".to_string())
}

pub fn mark_all_notifications_read(
    connection: &Connection,
    changed_at: &str,
) -> Result<usize, String> {
    validate_notification_timestamp(changed_at)?;
    connection
        .execute(
            "UPDATE notifications SET read_at = ?1 WHERE resolved_at IS NULL AND read_at IS NULL",
            [changed_at],
        )
        .map_err(|error| error.to_string())
}

pub fn resolve_notification(
    connection: &Connection,
    notification_id: i64,
    resolved_at: &str,
) -> Result<NotificationViewV1, String> {
    validate_notification_id(notification_id)?;
    validate_notification_timestamp(resolved_at)?;
    let changed = connection
        .execute(
            "UPDATE notifications
             SET resolved_at = COALESCE(resolved_at, ?2), read_at = COALESCE(read_at, ?2)
             WHERE id = ?1",
            params![notification_id, resolved_at],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("notification_not_found".to_string());
    }
    load_notification(connection, notification_id)?
        .ok_or_else(|| "notification_not_found".to_string())
}

pub fn resolve_notification_by_dedupe(
    connection: &Connection,
    dedupe_key: &str,
    resolved_at: &str,
) -> Result<usize, String> {
    validate_notification_text(dedupe_key, 512)?;
    validate_notification_timestamp(resolved_at)?;
    connection
        .execute(
            "UPDATE notifications
             SET resolved_at = ?2, read_at = COALESCE(read_at, ?2)
             WHERE dedupe_key = ?1 AND resolved_at IS NULL",
            params![dedupe_key, resolved_at],
        )
        .map_err(|error| error.to_string())
}

fn load_notification(
    connection: &Connection,
    notification_id: i64,
) -> Result<Option<NotificationViewV1>, String> {
    connection
        .query_row(
            "SELECT id, kind, severity, dedupe_key, title, body, account_id,
                    related_type, related_id, created_at, read_at, resolved_at
             FROM notifications WHERE id = ?1",
            [notification_id],
            notification_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn load_unresolved_notification_by_dedupe(
    connection: &Connection,
    dedupe_key: &str,
) -> Result<Option<NotificationViewV1>, String> {
    connection
        .query_row(
            "SELECT id, kind, severity, dedupe_key, title, body, account_id,
                    related_type, related_id, created_at, read_at, resolved_at
             FROM notifications WHERE dedupe_key = ?1 AND resolved_at IS NULL",
            [dedupe_key],
            notification_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn notification_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NotificationViewV1> {
    let severity: String = row.get(2)?;
    let severity = NotificationSeverityV1::parse(&severity).map_err(|_| {
        rusqlite::Error::InvalidColumnType(2, "severity".to_string(), rusqlite::types::Type::Text)
    })?;
    Ok(NotificationViewV1 {
        id: row.get(0)?,
        kind: row.get(1)?,
        severity,
        dedupe_key: row.get(3)?,
        title: row.get(4)?,
        body: row.get(5)?,
        account_id: row.get(6)?,
        related_type: row.get(7)?,
        related_id: row.get(8)?,
        created_at: row.get(9)?,
        read_at: row.get(10)?,
        resolved_at: row.get(11)?,
    })
}

fn validate_notification_insert(value: &NotificationInsertV1) -> Result<(), String> {
    validate_notification_text(&value.kind, 64)?;
    validate_notification_text(&value.dedupe_key, 512)?;
    validate_notification_text(&value.title, 200)?;
    validate_notification_text(&value.body, 4_000)?;
    if value.account_id.is_some_and(|id| id <= 0) {
        return Err("invalid_notification_account".to_string());
    }
    if value.related_type.is_some() != value.related_id.is_some() {
        return Err("invalid_notification_relation".to_string());
    }
    if let Some(related_type) = value.related_type.as_deref() {
        validate_notification_text(related_type, 64)?;
    }
    if let Some(related_id) = value.related_id.as_deref() {
        validate_notification_text(related_id, 512)?;
    }
    validate_notification_timestamp(&value.created_at)
}

fn validate_notification_text(value: &str, max_chars: usize) -> Result<(), String> {
    if value.is_empty()
        || value.trim() != value
        || value.chars().count() > max_chars
        || value.chars().any(|character| character.is_control())
    {
        return Err("invalid_notification".to_string());
    }
    Ok(())
}

fn validate_notification_timestamp(value: &str) -> Result<(), String> {
    DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| "invalid_notification_timestamp".to_string())
}

fn validate_notification_id(value: i64) -> Result<(), String> {
    if value <= 0 {
        Err("invalid_notification_id".to_string())
    } else {
        Ok(())
    }
}
