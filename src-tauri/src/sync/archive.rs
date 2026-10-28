use super::{
    decide_enqueue_for_task_key_tx, freeze_task_payload, shanghai_cutoff_ms, valid_sha256,
    validate_execution_envelope, validate_source_instance, ArchiveCollectionSuccessV1,
    ArchiveCompleteStopV1, ArchiveCompletionV1, ArchiveIncompleteReasonV1, ArchiveItemV1,
    ArchiveRunModeV1, BatchJobRoleV1, BatchTargetKindV1, FavoriteCollectionSuccessV1,
    ImportCountsV1, NewSyncBatchIssueKindV1, NewSyncBatchIssueV1, QueueStore, RangeFreezeV1,
    RemoteFolderCollectionSuccessV1, RemoteFolderDiscoveryResultV1, RemoteFolderDiscoverySuccessV1,
    SourceExecutionResultV1, StoredSyncJobV1, SyncBatchIssuePhaseV1, SyncExecutionEnvelopeV1,
    SyncJobIssueSeverityV1, SyncJobIssueV1, SyncJobSnapshotV1, SyncJobStatusV1, SyncResultImporter,
    SyncSourceV1, SyncTaskPayloadV1, SyncTriggerV1, TaskDecisionBatchTargetV1, TaskDecisionInputV1,
    TaskKeyV1, TerminalJobUpdateV1,
};
use crate::settings_store::{ArchiveListRangeV1, RemoteFolderScheduleModeV1};
use crate::{
    sync_orchestration::is_simple_uuid, sync_orchestration::SyncPhase, AppPaths,
    CollectorInvocation,
};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, path::PathBuf, sync::Arc};

const MAX_ARCHIVE_RECORDS: usize = 50_000;
const MAX_REMOTE_FOLDER_REGISTRY_RECORDS: i64 = 50_000;
const MAX_BATCH_TASK_TARGETS: i64 = 5_000;
const LIKES_URL: &str = "https://www.douyin.com/user/self?showTab=like";
const FAVORITES_URL: &str = "https://www.douyin.com/user/self?showTab=favorite_collection";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveKnownIdsSnapshotV1 {
    pub schema_version: i64,
    pub account_id: i64,
    pub sync_job_id: i64,
    pub phase: SyncPhase,
    pub phase_attempt_id: String,
    pub source: SyncSourceV1,
    pub source_instance_key: String,
    pub task_payload_sha256: String,
    pub aweme_ids: Vec<String>,
    pub ids_sha256: String,
}

pub fn build_archive_known_ids_snapshot(
    connection: &Connection,
    job: &StoredSyncJobV1,
    phase: SyncPhase,
    phase_attempt_id: &str,
) -> Result<ArchiveKnownIdsSnapshotV1, String> {
    if job.snapshot.status != SyncJobStatusV1::Running
        || !matches!(phase, SyncPhase::Collecting | SyncPhase::RetryingAfterLogin)
        || !is_simple_uuid(phase_attempt_id)
    {
        return Err("invalid_archive_known_ids_identity".to_string());
    }
    let payload = validated_archive_payload(job, job.snapshot.task_key.source)?;
    if payload.archive_range() != Some(ArchiveListRangeV1::All)
        || payload.run_mode() != Some(ArchiveRunModeV1::Incremental)
    {
        return Err("archive_known_ids_not_required".to_string());
    }
    let mut aweme_ids = query_archive_known_ids(
        connection,
        job.snapshot.task_key.account_id,
        job.snapshot.task_key.source,
        &job.snapshot.task_key.source_instance_key,
    )?;
    aweme_ids.sort_unstable();
    aweme_ids.dedup();
    let snapshot = ArchiveKnownIdsSnapshotV1 {
        schema_version: 1,
        account_id: job.snapshot.task_key.account_id,
        sync_job_id: job.snapshot.id,
        phase,
        phase_attempt_id: phase_attempt_id.to_string(),
        source: job.snapshot.task_key.source,
        source_instance_key: job.snapshot.task_key.source_instance_key.clone(),
        task_payload_sha256: job.snapshot.task_payload_sha256.clone(),
        ids_sha256: archive_known_ids_hash(&aweme_ids)?,
        aweme_ids,
    };
    validate_archive_known_ids_snapshot(&snapshot, &job.snapshot, phase, phase_attempt_id)?;
    Ok(snapshot)
}

pub fn validate_archive_known_ids_snapshot(
    snapshot: &ArchiveKnownIdsSnapshotV1,
    job: &SyncJobSnapshotV1,
    phase: SyncPhase,
    phase_attempt_id: &str,
) -> Result<(), String> {
    if snapshot.schema_version != 1
        || snapshot.account_id != job.task_key.account_id
        || snapshot.sync_job_id != job.id
        || snapshot.phase != phase
        || snapshot.phase_attempt_id != phase_attempt_id
        || !is_simple_uuid(&snapshot.phase_attempt_id)
        || snapshot.source != job.task_key.source
        || snapshot.source_instance_key != job.task_key.source_instance_key
        || snapshot.task_payload_sha256 != job.task_payload_sha256
        || !valid_sha256(&snapshot.task_payload_sha256)
        || job.status != SyncJobStatusV1::Running
        || !matches!(
            snapshot.source,
            SyncSourceV1::Likes | SyncSourceV1::Favorites | SyncSourceV1::RemoteFolder
        )
        || validate_source_instance(snapshot.source, &snapshot.source_instance_key).is_err()
    {
        return Err("archive_known_ids_identity_mismatch".to_string());
    }
    let mut previous: Option<&str> = None;
    for aweme_id in &snapshot.aweme_ids {
        if aweme_id.is_empty()
            || aweme_id.len() > 64
            || !aweme_id.bytes().all(|byte| byte.is_ascii_digit())
            || previous.is_some_and(|value| value >= aweme_id.as_str())
        {
            return Err("invalid_archive_known_ids".to_string());
        }
        previous = Some(aweme_id);
    }
    if !valid_sha256(&snapshot.ids_sha256)
        || snapshot.ids_sha256 != archive_known_ids_hash(&snapshot.aweme_ids)?
    {
        return Err("archive_known_ids_hash_mismatch".to_string());
    }
    Ok(())
}

fn query_archive_known_ids(
    connection: &Connection,
    account_id: i64,
    source: SyncSourceV1,
    source_instance_key: &str,
) -> Result<Vec<String>, String> {
    let (sql, parameter) = match source {
        SyncSourceV1::Likes => (
            "SELECT videos.aweme_id
             FROM user_likes relations
             JOIN videos ON videos.id = relations.video_id
             WHERE relations.account_id = ?1
             ORDER BY videos.aweme_id ASC",
            account_id,
        ),
        SyncSourceV1::Favorites => (
            "SELECT videos.aweme_id
             FROM favorite_items relations
             JOIN videos ON videos.id = relations.video_id
             WHERE relations.account_id = ?1
             ORDER BY videos.aweme_id ASC",
            account_id,
        ),
        SyncSourceV1::RemoteFolder => {
            let folder_id = connection
                .query_row(
                    "SELECT id FROM remote_folders
                     WHERE account_id = ?1 AND remote_folder_id = ?2",
                    params![account_id, source_instance_key],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "remote_folder_not_found".to_string())?;
            (
                "SELECT videos.aweme_id
                 FROM remote_folder_items relations
                 JOIN videos ON videos.id = relations.video_id
                 WHERE relations.folder_id = ?1
                 ORDER BY videos.aweme_id ASC",
                folder_id,
            )
        }
        SyncSourceV1::History => return Err("sync_executor_source_mismatch".to_string()),
    };
    let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([parameter], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn archive_known_ids_hash(aweme_ids: &[String]) -> Result<String, String> {
    let bytes = serde_json::to_vec(aweme_ids).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

type ArchiveStartFn =
    dyn Fn(super::QueuedSyncContext) -> Result<super::RunningSyncIdentity, String> + Send + Sync;
type ArchiveCancelFn = dyn Fn(&super::RunningSyncIdentity) -> Result<(), String> + Send + Sync;

#[derive(Clone)]
pub struct ArchiveSyncExecutor {
    source: SyncSourceV1,
    start: Arc<ArchiveStartFn>,
    cancel: Arc<ArchiveCancelFn>,
}

impl ArchiveSyncExecutor {
    pub fn new<Start, Cancel>(source: SyncSourceV1, start: Start, cancel: Cancel) -> Self
    where
        Start: Fn(super::QueuedSyncContext) -> Result<super::RunningSyncIdentity, String>
            + Send
            + Sync
            + 'static,
        Cancel: Fn(&super::RunningSyncIdentity) -> Result<(), String> + Send + Sync + 'static,
    {
        Self {
            source,
            start: Arc::new(start),
            cancel: Arc::new(cancel),
        }
    }
}

impl fmt::Debug for ArchiveSyncExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArchiveSyncExecutor")
            .field("source", &self.source)
            .finish()
    }
}

impl super::SyncExecutor for ArchiveSyncExecutor {
    fn source(&self) -> SyncSourceV1 {
        self.source
    }

    fn start(
        &self,
        context: super::QueuedSyncContext,
    ) -> Result<super::RunningSyncIdentity, String> {
        if !matches!(
            self.source,
            SyncSourceV1::Likes | SyncSourceV1::Favorites | SyncSourceV1::RemoteFolder
        ) || context.job.task_key.source != self.source
            || context.job.status != SyncJobStatusV1::Running
            || (self.source == SyncSourceV1::RemoteFolder
                && context.job.task_key.source_instance_key.is_empty())
            || (self.source != SyncSourceV1::RemoteFolder
                && !context.job.task_key.source_instance_key.is_empty())
        {
            return Err("sync_executor_source_mismatch".to_string());
        }
        (self.start)(context)
    }

    fn cancel(&self, running: &super::RunningSyncIdentity) -> Result<(), String> {
        if running.job_id <= 0 {
            return Err("invalid_sync_job_identity".to_string());
        }
        (self.cancel)(running)
    }
}

pub fn build_archive_collect_invocation(
    paths: &AppPaths,
    executable: PathBuf,
    job: &StoredSyncJobV1,
    phase: SyncPhase,
    phase_attempt_id: &str,
) -> Result<CollectorInvocation, String> {
    if !matches!(phase, SyncPhase::Collecting | SyncPhase::RetryingAfterLogin)
        || !is_simple_uuid(phase_attempt_id)
        || !executable.is_absolute()
    {
        return Err("invalid_archive_execution_identity".to_string());
    }
    let payload = validated_archive_payload(job, job.snapshot.task_key.source)?;
    let (range, range_freeze, run_mode, scroll_speed, hide_browser, browser, discover) =
        match &payload {
            SyncTaskPayloadV1::Likes {
                range,
                range_freeze,
                run_mode,
                scroll_speed,
                hide_browser,
                browser,
            }
            | SyncTaskPayloadV1::RemoteFolder {
                range,
                range_freeze,
                run_mode,
                scroll_speed,
                hide_browser,
                browser,
                ..
            } => (
                *range,
                range_freeze,
                *run_mode,
                *scroll_speed,
                *hide_browser,
                browser,
                false,
            ),
            SyncTaskPayloadV1::Favorites {
                range,
                range_freeze,
                run_mode,
                scroll_speed,
                hide_browser,
                browser,
                discover_remote_folders,
                ..
            } => (
                *range,
                range_freeze,
                *run_mode,
                *scroll_speed,
                *hide_browser,
                browser,
                *discover_remote_folders,
            ),
            SyncTaskPayloadV1::History { .. } => {
                return Err("sync_executor_source_mismatch".to_string())
            }
        };
    if run_mode == ArchiveRunModeV1::FullReconcile && range != ArchiveListRangeV1::All {
        return Err("full_reconcile_requires_all".to_string());
    }
    let page_url = match job.snapshot.task_key.source {
        SyncSourceV1::Likes => LIKES_URL,
        SyncSourceV1::Favorites | SyncSourceV1::RemoteFolder => FAVORITES_URL,
        SyncSourceV1::History => return Err("sync_executor_source_mismatch".to_string()),
    };
    let source = match job.snapshot.task_key.source {
        SyncSourceV1::Likes => "likes",
        SyncSourceV1::Favorites => "favorites",
        SyncSourceV1::RemoteFolder => "remote_folder",
        SyncSourceV1::History => return Err("sync_executor_source_mismatch".to_string()),
    };
    let account_profile_root = paths.account_browser_root(&browser.profile_key)?;
    let mut args = vec!["--collector".to_string(), "collect".to_string()];
    args.extend([
        "--account-id".to_string(),
        job.snapshot.task_key.account_id.to_string(),
        "--job-id".to_string(),
        job.snapshot.id.to_string(),
        "--source".to_string(),
        source.to_string(),
        "--source-instance-key".to_string(),
        job.snapshot.task_key.source_instance_key.clone(),
        "--task-payload-sha256".to_string(),
        job.snapshot.task_payload_sha256.clone(),
        "--frozen-at-ms".to_string(),
        range_freeze.frozen_at_ms.to_string(),
        "--phase".to_string(),
        phase.as_str().to_string(),
        "--phase-attempt-id".to_string(),
        phase_attempt_id.to_string(),
        "--browser-data-dir".to_string(),
        paths.app_data.to_string_lossy().to_string(),
        "--result".to_string(),
        paths
            .phase_result_path(
                job.snapshot.task_key.account_id,
                job.snapshot.id,
                phase,
                phase_attempt_id,
            )?
            .to_string_lossy()
            .to_string(),
        "--progress".to_string(),
        paths
            .phase_progress_path(
                job.snapshot.task_key.account_id,
                job.snapshot.id,
                phase,
                phase_attempt_id,
            )?
            .to_string_lossy()
            .to_string(),
        "--diagnostic".to_string(),
        paths
            .phase_diagnostic_path(
                job.snapshot.task_key.account_id,
                job.snapshot.id,
                phase,
                phase_attempt_id,
            )?
            .to_string_lossy()
            .to_string(),
        "--cancel-file".to_string(),
        paths
            .cancel_path(job.snapshot.task_key.account_id, job.snapshot.id)
            .to_string_lossy()
            .to_string(),
        "--account-profile-root".to_string(),
        account_profile_root.to_string_lossy().to_string(),
        "--profile-key".to_string(),
        browser.profile_key.clone(),
        "--profile-generation".to_string(),
        browser.profile_generation.clone(),
        "--browser-executable-sha256".to_string(),
        browser.executable_path_sha256.clone(),
        "--browser-pid-file".to_string(),
        paths
            .phase_browser_pid_path(
                job.snapshot.task_key.account_id,
                job.snapshot.id,
                phase,
                phase_attempt_id,
            )?
            .to_string_lossy()
            .to_string(),
        "--collector-identity-file".to_string(),
        paths
            .phase_collector_identity_path(
                job.snapshot.task_key.account_id,
                job.snapshot.id,
                phase,
                phase_attempt_id,
            )?
            .to_string_lossy()
            .to_string(),
        "--browser".to_string(),
        match browser.resolved_browser {
            super::BrowserIdV1::Edge => "edge",
            super::BrowserIdV1::Chrome => "chrome",
        }
        .to_string(),
        "--page-url".to_string(),
        page_url.to_string(),
        "--archive-range".to_string(),
        match range {
            ArchiveListRangeV1::All => "all",
            ArchiveListRangeV1::RecentItems { .. } => "recent_items",
            ArchiveListRangeV1::RecordLimit { .. } => "record_limit",
            ArchiveListRangeV1::RecentDays { .. } => "recent_days",
        }
        .to_string(),
        "--run-mode".to_string(),
        match run_mode {
            ArchiveRunModeV1::Incremental => "incremental",
            ArchiveRunModeV1::FullReconcile => "full_reconcile",
        }
        .to_string(),
        "--scroll-speed".to_string(),
        scroll_speed.to_string(),
    ]);
    match range {
        ArchiveListRangeV1::All => {
            if range_freeze.cutoff_ms.is_some() {
                return Err("range_cutoff_mismatch".to_string());
            }
        }
        ArchiveListRangeV1::RecentItems { count } => {
            if range_freeze.cutoff_ms.is_some() {
                return Err("range_cutoff_mismatch".to_string());
            }
            args.extend(["--item-count".to_string(), count.to_string()]);
        }
        ArchiveListRangeV1::RecordLimit { limit } => {
            if range_freeze.cutoff_ms.is_some() {
                return Err("range_cutoff_mismatch".to_string());
            }
            args.extend(["--record-limit".to_string(), limit.to_string()]);
        }
        ArchiveListRangeV1::RecentDays { days } => {
            let cutoff_ms = range_freeze
                .cutoff_ms
                .ok_or_else(|| "range_cutoff_mismatch".to_string())?;
            if cutoff_ms != shanghai_cutoff_ms(range_freeze.frozen_at_ms, days)? {
                return Err("range_cutoff_mismatch".to_string());
            }
            args.extend([
                "--days".to_string(),
                days.to_string(),
                "--cutoff-ms".to_string(),
                cutoff_ms.to_string(),
            ]);
        }
    }
    if range == ArchiveListRangeV1::All && run_mode == ArchiveRunModeV1::Incremental {
        args.extend([
            "--known-ids-file".to_string(),
            paths
                .phase_archive_known_ids_path(
                    job.snapshot.task_key.account_id,
                    job.snapshot.id,
                    phase,
                    phase_attempt_id,
                )?
                .to_string_lossy()
                .to_string(),
        ]);
    }
    if discover {
        args.push("--discover-remote-folders".to_string());
    }
    if hide_browser {
        args.push("--headless".to_string());
    }
    args.push("--block-images".to_string());
    Ok(CollectorInvocation {
        program: executable,
        args,
        current_dir: None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveImportOutcomeV1 {
    pub counts: ImportCountsV1,
    pub status: SyncJobStatusV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveResultImporter {
    source: SyncSourceV1,
}

#[derive(Debug, Clone, Copy)]
enum ArchiveRelationScope<'a> {
    Account { table: &'a str, account_id: i64 },
    RemoteFolder { folder_id: i64 },
}

impl ArchiveResultImporter {
    pub fn new(source: SyncSourceV1) -> Self {
        Self { source }
    }

    pub fn import_envelope(
        &self,
        connection: &mut Connection,
        expected_job_id: i64,
        envelope: SyncExecutionEnvelopeV1,
    ) -> Result<ArchiveImportOutcomeV1, String> {
        let imported = (|| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| error.to_string())?;
            let job = QueueStore::get_stored_job(&transaction, expected_job_id)?;
            if job.snapshot.status != SyncJobStatusV1::Running {
                return Err("sync_job_not_running".to_string());
            }
            validate_execution_envelope(&envelope, &job.snapshot)?;
            let counts = self.validate_and_import(&transaction, &job.snapshot, envelope.result)?;
            let finished = QueueStore::finish_running_job_tx(
                &transaction,
                expected_job_id,
                &TerminalJobUpdateV1 {
                    status: SyncJobStatusV1::Success,
                    finished_at: now_iso(),
                    inserted_videos: counts.inserted_videos,
                    inserted_records: counts.inserted_records,
                    error_code: None,
                    error_message: None,
                },
            )?;
            let outcome = ArchiveImportOutcomeV1 {
                counts,
                status: finished.snapshot.status,
            };
            transaction.commit().map_err(|error| error.to_string())?;
            Ok(outcome)
        })();
        if let Err(error) = &imported {
            fail_running_import_if_needed(connection, expected_job_id, error)?;
        }
        imported
    }
}

impl SyncResultImporter for ArchiveResultImporter {
    fn source(&self) -> SyncSourceV1 {
        self.source
    }

    fn validate_and_import(
        &self,
        transaction: &Transaction<'_>,
        job: &SyncJobSnapshotV1,
        result: SourceExecutionResultV1,
    ) -> Result<ImportCountsV1, String> {
        if job.task_key.source != self.source || result.source() != self.source {
            return Err("sync_executor_source_mismatch".to_string());
        }
        let stored = QueueStore::get_stored_job(transaction, job.id)?;
        let payload = validated_archive_payload(&stored, self.source)?;
        match (self.source, result) {
            (SyncSourceV1::Likes, SourceExecutionResultV1::Likes(archive)) => {
                import_archive_result(
                    transaction,
                    &stored,
                    &payload,
                    &archive,
                    ArchiveRelationScope::Account {
                        table: "user_likes",
                        account_id: stored.snapshot.task_key.account_id,
                    },
                )
            }
            (
                SyncSourceV1::Favorites,
                SourceExecutionResultV1::Favorites(FavoriteCollectionSuccessV1 {
                    archive,
                    remote_folder_discovery,
                }),
            ) => {
                validate_discovery_request(&payload, &remote_folder_discovery)?;
                let counts = import_archive_result(
                    transaction,
                    &stored,
                    &payload,
                    &archive,
                    ArchiveRelationScope::Account {
                        table: "favorite_items",
                        account_id: stored.snapshot.task_key.account_id,
                    },
                )?;
                import_remote_folder_discovery(
                    transaction,
                    &stored,
                    &payload,
                    &remote_folder_discovery,
                )?;
                Ok(counts)
            }
            (SyncSourceV1::RemoteFolder, SourceExecutionResultV1::RemoteFolder(collection)) => {
                import_remote_folder_result(transaction, &stored, &payload, &collection)
            }
            _ => Err("sync_executor_source_mismatch".to_string()),
        }
    }
}

fn import_remote_folder_discovery(
    transaction: &Transaction<'_>,
    job: &StoredSyncJobV1,
    payload: &SyncTaskPayloadV1,
    result: &RemoteFolderDiscoveryResultV1,
) -> Result<(), String> {
    let RemoteFolderDiscoveryResultV1::Success(discovery) = result else {
        return Ok(());
    };
    let SyncTaskPayloadV1::Favorites { run_mode, .. } = payload else {
        return Err("sync_executor_source_mismatch".to_string());
    };
    let observed_at = now_iso();
    transaction
        .execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS emily_remote_folder_seen_ids (
               remote_folder_id TEXT PRIMARY KEY
             );
             DELETE FROM temp.emily_remote_folder_seen_ids;",
        )
        .map_err(|error| error.to_string())?;

    let mut changes = Vec::with_capacity(discovery.folders.len());
    let mut new_folder_count = 0_i64;
    let mut registry_changed = false;
    for folder in &discovery.folders {
        let existing = transaction
            .query_row(
                "SELECT name, remote_removed_at FROM remote_folders
                 WHERE account_id = ?1 AND remote_folder_id = ?2",
                params![job.snapshot.task_key.account_id, folder.remote_folder_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        registry_changed |= existing
            .as_ref()
            .is_none_or(|(name, removed_at)| name != &folder.name || removed_at.is_some());
        new_folder_count += i64::from(existing.is_none());
        changes.push((folder, existing.is_none()));
        transaction
            .execute(
                "INSERT INTO temp.emily_remote_folder_seen_ids(remote_folder_id) VALUES (?1)",
                [&folder.remote_folder_id],
            )
            .map_err(|error| error.to_string())?;
    }

    let existing_folder_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM remote_folders WHERE account_id = ?1",
            [job.snapshot.task_key.account_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if existing_folder_count
        .checked_add(new_folder_count)
        .is_none_or(|count| count > MAX_REMOTE_FOLDER_REGISTRY_RECORDS)
    {
        return Err("remote_folder_registry_limit_exceeded".to_string());
    }

    let discovery_complete = discovery_is_complete(discovery);
    let should_mark_removed = *run_mode == ArchiveRunModeV1::FullReconcile && discovery_complete;
    let unseen_active_count = if should_mark_removed {
        transaction
            .query_row(
                "SELECT COUNT(*) FROM remote_folders folders
                 WHERE folders.account_id = ?1 AND folders.remote_removed_at IS NULL
                   AND NOT EXISTS(
                     SELECT 1 FROM temp.emily_remote_folder_seen_ids seen
                     WHERE seen.remote_folder_id = folders.remote_folder_id
                   )",
                [job.snapshot.task_key.account_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())?
    } else {
        0
    };
    registry_changed |= unseen_active_count > 0;
    if registry_changed {
        ensure_remote_folder_revision_available(transaction, job.snapshot.task_key.account_id)?;
    }

    for (folder, is_new) in changes {
        if is_new {
            transaction
                .execute(
                    "INSERT INTO remote_folders
                       (account_id, remote_folder_id, name, first_seen_at, last_seen_at,
                        remote_removed_at)
                     VALUES (?1, ?2, ?3, ?4, ?4, NULL)",
                    params![
                        job.snapshot.task_key.account_id,
                        folder.remote_folder_id,
                        folder.name,
                        observed_at,
                    ],
                )
                .map_err(|error| error.to_string())?;
        } else {
            transaction
                .execute(
                    "UPDATE remote_folders
                     SET name = ?1, last_seen_at = ?2, remote_removed_at = NULL
                     WHERE account_id = ?3 AND remote_folder_id = ?4",
                    params![
                        folder.name,
                        observed_at,
                        job.snapshot.task_key.account_id,
                        folder.remote_folder_id,
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
    }

    if unseen_active_count > 0 {
        let removed_job_ids =
            queued_remote_folder_jobs_not_seen(transaction, job.snapshot.task_key.account_id)?;
        transaction
            .execute(
                "UPDATE remote_folders AS folders
                 SET remote_removed_at = ?1
                 WHERE folders.account_id = ?2 AND folders.remote_removed_at IS NULL
                   AND NOT EXISTS(
                     SELECT 1 FROM temp.emily_remote_folder_seen_ids seen
                     WHERE seen.remote_folder_id = folders.remote_folder_id
                   )",
                params![observed_at, job.snapshot.task_key.account_id],
            )
            .map_err(|error| error.to_string())?;
        for job_id in removed_job_ids {
            QueueStore::cancel_queued_job_tx(
                transaction,
                job_id,
                &observed_at,
                "remote_folder_removed",
            )?;
        }
    }

    if registry_changed {
        transaction
            .execute(
                "UPDATE remote_folder_registry_state
                 SET revision = revision + 1 WHERE account_id = ?1",
                [job.snapshot.task_key.account_id],
            )
            .map_err(|error| error.to_string())?;
    }

    if discovery_complete {
        expand_remote_folder_children(transaction, job, payload, discovery, &observed_at)?;
    }

    if !discovery_complete {
        QueueStore::append_job_issue_tx(
            transaction,
            job.snapshot.id,
            &SyncJobIssueV1 {
                phase: "discovery".to_string(),
                code: "remote_folder_discovery_incomplete".to_string(),
                severity: SyncJobIssueSeverityV1::Incomplete,
                message: None,
            },
            &observed_at,
        )?;
    }
    Ok(())
}

fn queued_remote_folder_jobs_not_seen(
    transaction: &Transaction<'_>,
    account_id: i64,
) -> Result<Vec<i64>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT jobs.id
             FROM sync_jobs jobs
             JOIN remote_folders folders
               ON folders.account_id = jobs.account_id
              AND folders.remote_folder_id = jobs.source_instance_key
             WHERE jobs.account_id = ?1 AND jobs.source = 'remote_folder'
               AND jobs.status = 'queued' AND folders.remote_removed_at IS NULL
               AND NOT EXISTS(
                 SELECT 1 FROM temp.emily_remote_folder_seen_ids seen
                 WHERE seen.remote_folder_id = folders.remote_folder_id
               )
             ORDER BY jobs.id",
        )
        .map_err(|error| error.to_string())?;
    let job_ids = statement
        .query_map([account_id], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(job_ids)
}

fn expand_remote_folder_children(
    transaction: &Transaction<'_>,
    parent: &StoredSyncJobV1,
    payload: &SyncTaskPayloadV1,
    discovery: &RemoteFolderDiscoverySuccessV1,
    queued_at: &str,
) -> Result<(), String> {
    let SyncTaskPayloadV1::Favorites {
        range_freeze,
        scroll_speed,
        hide_browser,
        browser,
        remote_folder_mode,
        remote_folder_default_range,
        remote_folder_overrides,
        ..
    } = payload
    else {
        return Err("sync_executor_source_mismatch".to_string());
    };
    let parent_batch_ids = active_parent_batch_ids(transaction, parent.snapshot.id)?;
    if parent_batch_ids.is_empty() {
        return Ok(());
    }
    let blocked_batches = preflight_discovery_batch_limits(
        transaction,
        parent.snapshot.task_key.account_id,
        &parent_batch_ids,
        discovery,
        queued_at,
    )?;

    for folder in &discovery.folders {
        let task_key = TaskKeyV1 {
            account_id: parent.snapshot.task_key.account_id,
            source: SyncSourceV1::RemoteFolder,
            source_instance_key: folder.remote_folder_id.clone(),
        };
        let mut batch_ids = Vec::new();
        for &batch_id in &parent_batch_ids {
            if !blocked_batches.contains(&batch_id)
                && !remote_folder_expansion_closed(transaction, batch_id)?
                && !batch_has_task_target(transaction, batch_id, &task_key)?
            {
                batch_ids.push(batch_id);
            }
        }
        if batch_ids.is_empty() {
            continue;
        }

        let range = match remote_folder_mode {
            RemoteFolderScheduleModeV1::Unified => *remote_folder_default_range,
            RemoteFolderScheduleModeV1::PerFolder => remote_folder_overrides
                .get(&folder.remote_folder_id)
                .map(|value| value.range)
                .unwrap_or(*remote_folder_default_range),
        };
        let child_payload = SyncTaskPayloadV1::RemoteFolder {
            remote_folder_id: folder.remote_folder_id.clone(),
            range,
            range_freeze: RangeFreezeV1 {
                frozen_at_ms: range_freeze.frozen_at_ms,
                cutoff_ms: match range {
                    ArchiveListRangeV1::RecentDays { days } => {
                        Some(shanghai_cutoff_ms(range_freeze.frozen_at_ms, days)?)
                    }
                    _ => None,
                },
            },
            run_mode: ArchiveRunModeV1::Incremental,
            scroll_speed: *scroll_speed,
            hide_browser: *hide_browser,
            browser: browser.clone(),
        };
        let frozen = freeze_task_payload(child_payload)?;
        let existing = QueueStore::find_active_job_tx(transaction, &task_key)?;
        let role = if existing.as_ref().is_some_and(|job| {
            job.equivalence_sha256 == frozen.equivalence_sha256
                && !(job.snapshot.trigger == SyncTriggerV1::Automatic
                    && job.snapshot.status == SyncJobStatusV1::Queued)
        }) {
            BatchJobRoleV1::Attached
        } else {
            BatchJobRoleV1::DiscoveredChild
        };
        decide_enqueue_for_task_key_tx(
            transaction,
            &TaskDecisionInputV1 {
                task_key,
                trigger: SyncTriggerV1::Manual,
                queued_at: queued_at.to_string(),
                scheduled_for: None,
                settings_revision: parent.settings_revision,
                config_sha256: None,
                payload: frozen,
                batch_targets: batch_ids
                    .into_iter()
                    .map(|batch_id| TaskDecisionBatchTargetV1 {
                        batch_id,
                        role,
                        phase: SyncBatchIssuePhaseV1::DiscoveryExpansion,
                    })
                    .collect(),
            },
        )?;
    }
    Ok(())
}

fn preflight_discovery_batch_limits(
    transaction: &Transaction<'_>,
    account_id: i64,
    parent_batch_ids: &[i64],
    discovery: &RemoteFolderDiscoverySuccessV1,
    changed_at: &str,
) -> Result<Vec<i64>, String> {
    let mut blocked = Vec::new();
    for &batch_id in parent_batch_ids {
        if remote_folder_expansion_closed(transaction, batch_id)? {
            blocked.push(batch_id);
            continue;
        }
        let existing_task_count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM sync_batch_targets
                 WHERE batch_id = ?1 AND target_kind = 'task'",
                [batch_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let mut candidate_count = 0_i64;
        for folder in &discovery.folders {
            let task_key = TaskKeyV1 {
                account_id,
                source: SyncSourceV1::RemoteFolder,
                source_instance_key: folder.remote_folder_id.clone(),
            };
            if !batch_has_task_target(transaction, batch_id, &task_key)? {
                candidate_count += 1;
            }
        }
        if existing_task_count
            .checked_add(candidate_count)
            .is_some_and(|count| count <= MAX_BATCH_TASK_TARGETS)
        {
            continue;
        }
        append_discovery_aggregate_skip(
            transaction,
            batch_id,
            account_id,
            "sync_batch_target_limit_exceeded",
            changed_at,
        )?;
        blocked.push(batch_id);
    }
    Ok(blocked)
}

fn append_discovery_aggregate_skip(
    transaction: &Transaction<'_>,
    batch_id: i64,
    account_id: i64,
    reason: &str,
    changed_at: &str,
) -> Result<(), String> {
    let task_key = TaskKeyV1 {
        account_id,
        source: SyncSourceV1::RemoteFolder,
        source_instance_key: String::new(),
    };
    let issue = NewSyncBatchIssueV1 {
        task_key: task_key.clone(),
        target_kind_aggregate: true,
        phase: SyncBatchIssuePhaseV1::DiscoveryExpansion,
        kind: NewSyncBatchIssueKindV1::Skipped {
            reason: reason.to_string(),
            requested_payload_sha256: None,
        },
    };
    let entry_seq = QueueStore::reserve_batch_target_tx(
        transaction,
        batch_id,
        &task_key,
        BatchTargetKindV1::Aggregate,
        "issue",
    )?;
    QueueStore::append_reserved_batch_issue_tx(
        transaction,
        batch_id,
        &issue,
        entry_seq,
        changed_at,
    )?;
    QueueStore::recalculate_batch_status_tx(transaction, batch_id, changed_at)?;
    Ok(())
}

fn active_parent_batch_ids(
    transaction: &Transaction<'_>,
    parent_job_id: i64,
) -> Result<Vec<i64>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT links.batch_id
             FROM sync_batch_jobs links
             JOIN sync_batches batches ON batches.id = links.batch_id
             WHERE links.job_id = ?1 AND batches.status IN ('queued', 'running')
             ORDER BY links.batch_id",
        )
        .map_err(|error| error.to_string())?;
    let batch_ids = statement
        .query_map([parent_job_id], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(batch_ids)
}

fn remote_folder_expansion_closed(
    transaction: &Transaction<'_>,
    batch_id: i64,
) -> Result<bool, String> {
    transaction
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM sync_batch_issues
               WHERE batch_id = ?1 AND source = 'remote_folder'
                 AND source_instance_key = '' AND kind = 'skipped'
                 AND reason IN (
                   'remote_folder_schedule_limit_exceeded',
                   'sync_batch_target_limit_exceeded'
                 )
             )",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn batch_has_task_target(
    transaction: &Transaction<'_>,
    batch_id: i64,
    task_key: &TaskKeyV1,
) -> Result<bool, String> {
    transaction
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM sync_batch_targets
               WHERE batch_id = ?1 AND account_id = ?2 AND source = ?3
                 AND source_instance_key = ?4
             )",
            params![
                batch_id,
                task_key.account_id,
                task_key.source.sqlite_name(),
                task_key.source_instance_key,
            ],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn discovery_is_complete(discovery: &RemoteFolderDiscoverySuccessV1) -> bool {
    discovery.complete_scan && discovery.bottom_confirmed && !discovery.hit_record_limit
}

fn ensure_remote_folder_revision_available(
    transaction: &Transaction<'_>,
    account_id: i64,
) -> Result<(), String> {
    let revision = transaction
        .query_row(
            "SELECT revision FROM remote_folder_registry_state WHERE account_id = ?1",
            [account_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "remote_folder_registry_state_missing".to_string())?;
    if revision == i64::MAX {
        Err("remote_folder_registry_revision_exhausted".to_string())
    } else {
        Ok(())
    }
}

fn validated_archive_payload(
    job: &StoredSyncJobV1,
    expected_source: SyncSourceV1,
) -> Result<SyncTaskPayloadV1, String> {
    if job.snapshot.task_key.source != expected_source {
        return Err("sync_executor_source_mismatch".to_string());
    }
    let payload = serde_json::from_str::<SyncTaskPayloadV1>(&job.payload_json)
        .map_err(|_| "invalid_sync_job_payload".to_string())?;
    let frozen = freeze_task_payload(payload.clone())?;
    if frozen.payload_json != job.payload_json
        || frozen.payload_sha256 != job.snapshot.task_payload_sha256
        || frozen.equivalence_sha256 != job.equivalence_sha256
        || payload.source() != expected_source
        || payload.source_instance_key() != job.snapshot.task_key.source_instance_key
    {
        return Err("invalid_sync_job_payload".to_string());
    }
    Ok(payload)
}

fn validate_discovery_request(
    payload: &SyncTaskPayloadV1,
    result: &RemoteFolderDiscoveryResultV1,
) -> Result<(), String> {
    let SyncTaskPayloadV1::Favorites {
        discover_remote_folders,
        ..
    } = payload
    else {
        return Err("sync_executor_source_mismatch".to_string());
    };
    match (*discover_remote_folders, result) {
        (false, RemoteFolderDiscoveryResultV1::NotRequested)
        | (true, RemoteFolderDiscoveryResultV1::Success(_)) => Ok(()),
        _ => Err("remote_folder_discovery_request_mismatch".to_string()),
    }
}

fn import_archive_result(
    transaction: &Transaction<'_>,
    job: &StoredSyncJobV1,
    payload: &SyncTaskPayloadV1,
    archive: &ArchiveCollectionSuccessV1,
    relation_scope: ArchiveRelationScope<'_>,
) -> Result<ImportCountsV1, String> {
    let (range, range_freeze, run_mode) = archive_payload_parts(payload)?;
    validate_archive_completion(
        transaction,
        relation_scope,
        range,
        range_freeze.cutoff_ms,
        run_mode,
        archive,
    )?;

    let observed_at = now_iso();
    transaction
        .execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS emily_archive_seen_video_ids (
               video_id INTEGER PRIMARY KEY
             );
             DELETE FROM temp.emily_archive_seen_video_ids;",
        )
        .map_err(|error| error.to_string())?;

    let mut inserted_videos = 0;
    let mut inserted_records = 0;
    for record in &archive.records {
        let existing_video_id: Option<i64> = transaction
            .query_row(
                "SELECT id FROM videos WHERE aweme_id = ?1",
                [&record.aweme_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO videos
                   (aweme_id, canonical_url, content_type, title, author_name, cover_url,
                    like_count, first_seen_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                 ON CONFLICT(aweme_id) DO UPDATE SET
                   canonical_url = excluded.canonical_url,
                   content_type = excluded.content_type,
                   title = COALESCE(excluded.title, videos.title),
                   author_name = COALESCE(excluded.author_name, videos.author_name),
                   cover_url = COALESCE(excluded.cover_url, videos.cover_url),
                   like_count = COALESCE(excluded.like_count, videos.like_count),
                   last_seen_at = excluded.last_seen_at",
                params![
                    record.aweme_id,
                    record.canonical_url,
                    record.content_type,
                    record.title,
                    record.author_name,
                    record.cover_url,
                    record.like_count,
                    observed_at,
                ],
            )
            .map_err(|error| error.to_string())?;
        let video_id: i64 = transaction
            .query_row(
                "SELECT id FROM videos WHERE aweme_id = ?1",
                [&record.aweme_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        inserted_videos += i64::from(existing_video_id.is_none());
        let existing_relation = relation_exists(transaction, relation_scope, video_id)?;
        upsert_archive_relation(transaction, relation_scope, video_id, record, &observed_at)?;
        if record.cover_url.as_deref().is_some_and(|url| !url.trim().is_empty()) {
            crate::cover_cache::enqueue_cover_candidate_best_effort(
                transaction,
                video_id,
                job.snapshot.id,
                &observed_at,
            );
        }
        inserted_records += i64::from(!existing_relation);
        transaction
            .execute(
                "INSERT OR IGNORE INTO temp.emily_archive_seen_video_ids(video_id) VALUES (?1)",
                [video_id],
            )
            .map_err(|error| error.to_string())?;
    }

    if archive_is_incomplete(archive) {
        let code = match archive.completion {
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::SafetyRecordCap { .. },
            } => "archive_safety_record_cap_reached",
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
            } => "archive_unconfirmed_end",
            _ => unreachable!(),
        };
        QueueStore::append_job_issue_tx(
            transaction,
            job.snapshot.id,
            &SyncJobIssueV1 {
                phase: "collection".to_string(),
                code: code.to_string(),
                severity: SyncJobIssueSeverityV1::Incomplete,
                message: None,
            },
            &observed_at,
        )?;
    }

    if run_mode == ArchiveRunModeV1::FullReconcile
        && matches!(
            archive.completion,
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom
            }
        )
    {
        mark_unseen_removed(transaction, relation_scope, &observed_at)?;
    }

    Ok(ImportCountsV1 {
        inserted_videos,
        inserted_records,
        inserted_watch_records: 0,
    })
}

fn import_remote_folder_result(
    transaction: &Transaction<'_>,
    job: &StoredSyncJobV1,
    payload: &SyncTaskPayloadV1,
    result: &RemoteFolderCollectionSuccessV1,
) -> Result<ImportCountsV1, String> {
    let SyncTaskPayloadV1::RemoteFolder {
        remote_folder_id, ..
    } = payload
    else {
        return Err("sync_executor_source_mismatch".to_string());
    };
    if remote_folder_id != &job.snapshot.task_key.source_instance_key
        || result.remote_folder_id != *remote_folder_id
    {
        return Err("sync_result_identity_mismatch".to_string());
    }

    let folder = transaction
        .query_row(
            "SELECT id, name, remote_removed_at
             FROM remote_folders
             WHERE account_id = ?1 AND remote_folder_id = ?2",
            params![job.snapshot.task_key.account_id, remote_folder_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "remote_folder_not_found".to_string())?;
    let registry_changed = folder.1 != result.name || folder.2.is_some();
    if registry_changed {
        ensure_remote_folder_revision_available(transaction, job.snapshot.task_key.account_id)?;
    }

    let observed_at = now_iso();
    transaction
        .execute(
            "UPDATE remote_folders
             SET name = ?1, last_seen_at = ?2, remote_removed_at = NULL
             WHERE id = ?3 AND account_id = ?4 AND remote_folder_id = ?5",
            params![
                result.name,
                observed_at,
                folder.0,
                job.snapshot.task_key.account_id,
                remote_folder_id,
            ],
        )
        .map_err(|error| error.to_string())?;
    if registry_changed {
        transaction
            .execute(
                "UPDATE remote_folder_registry_state
                 SET revision = revision + 1 WHERE account_id = ?1",
                [job.snapshot.task_key.account_id],
            )
            .map_err(|error| error.to_string())?;
    }

    import_archive_result(
        transaction,
        job,
        payload,
        &result.archive,
        ArchiveRelationScope::RemoteFolder {
            folder_id: folder.0,
        },
    )
}

fn archive_payload_parts(
    payload: &SyncTaskPayloadV1,
) -> Result<(ArchiveListRangeV1, &super::RangeFreezeV1, ArchiveRunModeV1), String> {
    match payload {
        SyncTaskPayloadV1::Likes {
            range,
            range_freeze,
            run_mode,
            ..
        }
        | SyncTaskPayloadV1::Favorites {
            range,
            range_freeze,
            run_mode,
            ..
        }
        | SyncTaskPayloadV1::RemoteFolder {
            range,
            range_freeze,
            run_mode,
            ..
        } => Ok((*range, range_freeze, *run_mode)),
        SyncTaskPayloadV1::History { .. } => Err("sync_executor_source_mismatch".to_string()),
    }
}

fn validate_archive_completion(
    transaction: &Transaction<'_>,
    relation_scope: ArchiveRelationScope<'_>,
    range: ArchiveListRangeV1,
    cutoff_ms: Option<i64>,
    run_mode: ArchiveRunModeV1,
    archive: &ArchiveCollectionSuccessV1,
) -> Result<(), String> {
    let count = archive.records.len();
    if run_mode == ArchiveRunModeV1::FullReconcile
        && (!matches!(range, ArchiveListRangeV1::All)
            || !matches!(
                archive.completion,
                ArchiveCompletionV1::Complete {
                    stop: ArchiveCompleteStopV1::Bottom
                }
            ))
    {
        return Err("invalid_archive_completion_for_payload".to_string());
    }
    match (&archive.completion, range) {
        (
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
            ArchiveListRangeV1::All,
        ) if count <= MAX_ARCHIVE_RECORDS => Ok(()),
        (
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::StableKnownBoundary { consecutive_known },
            },
            ArchiveListRangeV1::All,
        ) if run_mode == ArchiveRunModeV1::Incremental && count < MAX_ARCHIVE_RECORDS => {
            let actual = consecutive_known_at_tail(transaction, relation_scope, &archive.records)?;
            if actual == *consecutive_known as usize && actual >= 20 {
                Ok(())
            } else {
                Err("stable_known_boundary_mismatch".to_string())
            }
        }
        (
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::RequestedItemCount { count: requested },
            },
            ArchiveListRangeV1::RecentItems { count: expected },
        ) if requested == &expected && count == expected as usize => Ok(()),
        (
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
            ArchiveListRangeV1::RecentItems { count: expected },
        ) if count < expected as usize => Ok(()),
        (
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::RecordLimit { limit: requested },
            },
            ArchiveListRangeV1::RecordLimit { limit: expected },
        ) if requested == &expected && count == expected as usize => Ok(()),
        (
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
            ArchiveListRangeV1::RecordLimit { limit: expected },
        ) if count < expected as usize => Ok(()),
        (
            ArchiveCompletionV1::Complete {
                stop:
                    ArchiveCompleteStopV1::DateBoundary {
                        cutoff_ms: stop_cutoff,
                        ..
                    },
            },
            ArchiveListRangeV1::RecentDays { .. },
        ) if cutoff_ms == Some(*stop_cutoff) && records_within_cutoff(archive, *stop_cutoff) => {
            Ok(())
        }
        (
            ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::Bottom,
            },
            ArchiveListRangeV1::RecentDays { .. },
        ) if cutoff_ms.is_some_and(|cutoff| records_within_cutoff(archive, cutoff)) => Ok(()),
        (
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::SafetyRecordCap { cap },
            },
            ArchiveListRangeV1::All,
        ) if *cap == MAX_ARCHIVE_RECORDS as u32 && count == MAX_ARCHIVE_RECORDS => Ok(()),
        (
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::SafetyRecordCap { cap },
            },
            ArchiveListRangeV1::RecentDays { .. },
        ) if *cap == MAX_ARCHIVE_RECORDS as u32
            && count == MAX_ARCHIVE_RECORDS
            && cutoff_ms.is_some_and(|cutoff| records_within_cutoff(archive, cutoff)) =>
        {
            Ok(())
        }
        (
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
            },
            ArchiveListRangeV1::All,
        ) if count < MAX_ARCHIVE_RECORDS => Ok(()),
        (
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
            },
            ArchiveListRangeV1::RecentItems { count: expected },
        ) if count < expected as usize => Ok(()),
        (
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
            },
            ArchiveListRangeV1::RecordLimit { limit: expected },
        ) if count < expected as usize => Ok(()),
        (
            ArchiveCompletionV1::Incomplete {
                reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
            },
            ArchiveListRangeV1::RecentDays { .. },
        ) if count < MAX_ARCHIVE_RECORDS
            && cutoff_ms.is_some_and(|cutoff| records_within_cutoff(archive, cutoff)) =>
        {
            Ok(())
        }
        _ => Err("invalid_archive_completion_for_payload".to_string()),
    }
}

fn records_within_cutoff(archive: &ArchiveCollectionSuccessV1, cutoff_ms: i64) -> bool {
    archive.records.iter().all(|record| {
        record
            .remote_created_at_ms
            .is_some_and(|value| value >= cutoff_ms)
    })
}

fn consecutive_known_at_tail(
    transaction: &Transaction<'_>,
    relation_scope: ArchiveRelationScope<'_>,
    records: &[ArchiveItemV1],
) -> Result<usize, String> {
    let mut count = 0;
    for record in records.iter().rev() {
        let known: bool = match relation_scope {
            ArchiveRelationScope::Account { table, account_id } => transaction.query_row(
                &format!(
                    "SELECT EXISTS(
                           SELECT 1 FROM {table} relations
                           JOIN videos ON videos.id = relations.video_id
                           WHERE relations.account_id = ?1 AND videos.aweme_id = ?2
                         )"
                ),
                params![account_id, record.aweme_id],
                |row| row.get(0),
            ),
            ArchiveRelationScope::RemoteFolder { folder_id } => transaction.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM remote_folder_items relations
                   JOIN videos ON videos.id = relations.video_id
                   WHERE relations.folder_id = ?1 AND videos.aweme_id = ?2
                 )",
                params![folder_id, record.aweme_id],
                |row| row.get(0),
            ),
        }
        .map_err(|error| error.to_string())?;
        if !known {
            break;
        }
        count += 1;
    }
    Ok(count)
}

fn relation_exists(
    transaction: &Transaction<'_>,
    relation_scope: ArchiveRelationScope<'_>,
    video_id: i64,
) -> Result<bool, String> {
    match relation_scope {
        ArchiveRelationScope::Account { table, account_id } => transaction.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM {table} WHERE account_id = ?1 AND video_id = ?2)"
            ),
            params![account_id, video_id],
            |row| row.get(0),
        ),
        ArchiveRelationScope::RemoteFolder { folder_id } => transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM remote_folder_items WHERE folder_id = ?1 AND video_id = ?2
             )",
            params![folder_id, video_id],
            |row| row.get(0),
        ),
    }
    .map_err(|error| error.to_string())
}

fn upsert_archive_relation(
    transaction: &Transaction<'_>,
    relation_scope: ArchiveRelationScope<'_>,
    video_id: i64,
    record: &ArchiveItemV1,
    observed_at: &str,
) -> Result<(), String> {
    match relation_scope {
        ArchiveRelationScope::Account { table, account_id } => transaction.execute(
            &format!("INSERT INTO {table}
                   (account_id, video_id, first_seen_at, last_seen_at,
                    remote_created_at_ms, remote_removed_at, source_position)
                 VALUES (?1, ?2, ?3, ?3, ?4, NULL, ?5)
                 ON CONFLICT(account_id, video_id) DO UPDATE SET
                   last_seen_at = excluded.last_seen_at,
                   remote_created_at_ms = COALESCE(excluded.remote_created_at_ms, {table}.remote_created_at_ms),
                   remote_removed_at = NULL,
                   source_position = excluded.source_position"),
            params![
                account_id,
                video_id,
                observed_at,
                record.remote_created_at_ms,
                record.source_position,
            ],
        ),
        ArchiveRelationScope::RemoteFolder { folder_id } => transaction.execute(
            "INSERT INTO remote_folder_items
               (folder_id, video_id, first_seen_at, last_seen_at, remote_removed_at)
             VALUES (?1, ?2, ?3, ?3, NULL)
             ON CONFLICT(folder_id, video_id) DO UPDATE SET
               last_seen_at = excluded.last_seen_at,
               remote_removed_at = NULL",
            params![folder_id, video_id, observed_at],
        ),
    }
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn mark_unseen_removed(
    transaction: &Transaction<'_>,
    relation_scope: ArchiveRelationScope<'_>,
    removed_at: &str,
) -> Result<(), String> {
    match relation_scope {
        ArchiveRelationScope::Account { table, account_id } => transaction.execute(
            &format!(
                "UPDATE {table}
                 SET remote_removed_at = ?1
                 WHERE account_id = ?2 AND remote_removed_at IS NULL
                   AND NOT EXISTS(
                     SELECT 1 FROM temp.emily_archive_seen_video_ids seen
                     WHERE seen.video_id = {table}.video_id
                   )"
            ),
            params![removed_at, account_id],
        ),
        ArchiveRelationScope::RemoteFolder { folder_id } => transaction.execute(
            "UPDATE remote_folder_items
             SET remote_removed_at = ?1
             WHERE folder_id = ?2 AND remote_removed_at IS NULL
               AND NOT EXISTS(
                 SELECT 1 FROM temp.emily_archive_seen_video_ids seen
                 WHERE seen.video_id = remote_folder_items.video_id
               )",
            params![removed_at, folder_id],
        ),
    }
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn archive_is_incomplete(archive: &ArchiveCollectionSuccessV1) -> bool {
    matches!(archive.completion, ArchiveCompletionV1::Incomplete { .. })
}

fn fail_running_import_if_needed(
    connection: &mut Connection,
    job_id: i64,
    error: &str,
) -> Result<(), String> {
    let status = connection
        .query_row(
            "SELECT status FROM sync_jobs WHERE id = ?1",
            [job_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|query_error| query_error.to_string())?;
    if status.as_deref() != Some("running") {
        return Ok(());
    }
    QueueStore::finish_running_job(
        connection,
        job_id,
        &TerminalJobUpdateV1 {
            status: SyncJobStatusV1::Failed,
            finished_at: now_iso(),
            inserted_videos: 0,
            inserted_records: 0,
            error_code: Some("sync_result_import_failed".to_string()),
            error_message: Some(error.to_string()),
        },
    )?;
    Ok(())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
