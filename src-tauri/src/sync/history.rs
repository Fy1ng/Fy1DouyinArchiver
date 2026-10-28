use super::{
    freeze_task_payload, validate_execution_envelope, BrowserIdV1, ImportCountsV1, QueueStore,
    QueuedSyncContext, RunningSyncIdentity, SourceExecutionResultV1, StoredSyncJobV1,
    SyncExecutionEnvelopeV1, SyncExecutor, SyncJobSnapshotV1, SyncJobStatusV1, SyncResultImporter,
    SyncSourceV1, SyncTaskPayloadV1, TerminalJobUpdateV1,
};
use crate::{
    import_history_result_tx,
    settings_store::HistoryRangeV1,
    sync_orchestration::{is_simple_uuid, SyncPhase},
    AppPaths, CollectorInvocation,
};
use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::{fmt, path::PathBuf, sync::Arc};

type HistoryStartFn =
    dyn Fn(QueuedSyncContext) -> Result<RunningSyncIdentity, String> + Send + Sync;
type HistoryCancelFn = dyn Fn(&RunningSyncIdentity) -> Result<(), String> + Send + Sync;

#[derive(Clone)]
pub struct HistorySyncExecutor {
    start: Arc<HistoryStartFn>,
    cancel: Arc<HistoryCancelFn>,
}

impl HistorySyncExecutor {
    pub fn new<Start, Cancel>(start: Start, cancel: Cancel) -> Self
    where
        Start: Fn(QueuedSyncContext) -> Result<RunningSyncIdentity, String> + Send + Sync + 'static,
        Cancel: Fn(&RunningSyncIdentity) -> Result<(), String> + Send + Sync + 'static,
    {
        Self {
            start: Arc::new(start),
            cancel: Arc::new(cancel),
        }
    }
}

impl fmt::Debug for HistorySyncExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("HistorySyncExecutor").finish()
    }
}

impl SyncExecutor for HistorySyncExecutor {
    fn source(&self) -> SyncSourceV1 {
        SyncSourceV1::History
    }

    fn start(&self, context: QueuedSyncContext) -> Result<RunningSyncIdentity, String> {
        if context.job.task_key.source != SyncSourceV1::History
            || !context.job.task_key.source_instance_key.is_empty()
            || context.job.status != SyncJobStatusV1::Running
        {
            return Err("sync_executor_source_mismatch".to_string());
        }
        (self.start)(context)
    }

    fn cancel(&self, running: &RunningSyncIdentity) -> Result<(), String> {
        if running.job_id <= 0 {
            return Err("invalid_sync_job_identity".to_string());
        }
        (self.cancel)(running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryImportOutcomeV1 {
    pub counts: ImportCountsV1,
    pub status: SyncJobStatusV1,
}

pub fn decode_execution_envelope(bytes: &[u8]) -> Result<SyncExecutionEnvelopeV1, String> {
    serde_json::from_slice(bytes).map_err(|_| "invalid_sync_execution_envelope".to_string())
}

pub fn build_history_collect_invocation(
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
        return Err("invalid_history_execution_identity".to_string());
    }
    let payload = validated_history_payload(job)?;
    let SyncTaskPayloadV1::History {
        range,
        range_freeze,
        scroll_speed,
        hide_browser,
        browser,
    } = payload
    else {
        return Err("sync_executor_source_mismatch".to_string());
    };
    let account_profile_root = paths.account_browser_root(&browser.profile_key)?;
    let mut args = vec!["--collector".to_string(), "collect".to_string()];
    args.extend([
        "--account-id".to_string(),
        job.snapshot.task_key.account_id.to_string(),
        "--job-id".to_string(),
        job.snapshot.id.to_string(),
        "--source".to_string(),
        "history".to_string(),
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
        browser.profile_key,
        "--profile-generation".to_string(),
        browser.profile_generation,
        "--browser-executable-sha256".to_string(),
        browser.executable_path_sha256,
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
            BrowserIdV1::Edge => "edge",
            BrowserIdV1::Chrome => "chrome",
        }
        .to_string(),
        "--scroll-speed".to_string(),
        scroll_speed.to_string(),
    ]);
    match range {
        HistoryRangeV1::Days { days } => {
            let cutoff_ms = range_freeze
                .cutoff_ms
                .ok_or_else(|| "range_cutoff_mismatch".to_string())?;
            args.extend([
                "--days".to_string(),
                days.to_string(),
                "--cutoff-ms".to_string(),
                cutoff_ms.to_string(),
            ]);
        }
        HistoryRangeV1::Bottom => {
            if range_freeze.cutoff_ms.is_some() {
                return Err("range_cutoff_mismatch".to_string());
            }
            args.push("--bottom".to_string());
        }
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

#[derive(Debug, Default, Clone, Copy)]
pub struct HistoryResultImporter;

impl HistoryResultImporter {
    pub fn import_envelope(
        &self,
        connection: &mut Connection,
        expected_job_id: i64,
        envelope: SyncExecutionEnvelopeV1,
    ) -> Result<HistoryImportOutcomeV1, String> {
        let imported = (|| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| error.to_string())?;
            let job = QueueStore::get_stored_job(&transaction, expected_job_id)?;
            if job.snapshot.status != SyncJobStatusV1::Running {
                return Err("sync_job_not_running".to_string());
            }
            validated_history_payload(&job)?;
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
            let outcome = HistoryImportOutcomeV1 {
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

impl SyncResultImporter for HistoryResultImporter {
    fn source(&self) -> SyncSourceV1 {
        SyncSourceV1::History
    }

    fn validate_and_import(
        &self,
        transaction: &Transaction<'_>,
        job: &SyncJobSnapshotV1,
        result: SourceExecutionResultV1,
    ) -> Result<ImportCountsV1, String> {
        if job.task_key.source != SyncSourceV1::History {
            return Err("sync_executor_source_mismatch".to_string());
        }
        let SourceExecutionResultV1::History(result) = result else {
            return Err("sync_executor_source_mismatch".to_string());
        };
        let summary =
            import_history_result_tx(transaction, &result, job.id, job.task_key.account_id)?;
        Ok(ImportCountsV1 {
            inserted_videos: summary.inserted_videos,
            inserted_records: summary.inserted_watch_records,
            inserted_watch_records: summary.inserted_watch_records,
        })
    }
}

fn validated_history_payload(job: &StoredSyncJobV1) -> Result<SyncTaskPayloadV1, String> {
    if job.snapshot.task_key.source != SyncSourceV1::History
        || !job.snapshot.task_key.source_instance_key.is_empty()
    {
        return Err("sync_executor_source_mismatch".to_string());
    }
    let payload = serde_json::from_str::<SyncTaskPayloadV1>(&job.payload_json)
        .map_err(|_| "invalid_sync_job_payload".to_string())?;
    let frozen = freeze_task_payload(payload.clone())?;
    if frozen.payload_json != job.payload_json
        || frozen.payload_sha256 != job.snapshot.task_payload_sha256
        || frozen.equivalence_sha256 != job.equivalence_sha256
        || payload.source() != job.snapshot.task_key.source
        || payload.source_instance_key() != job.snapshot.task_key.source_instance_key
    {
        return Err("invalid_sync_job_payload".to_string());
    }
    Ok(payload)
}

fn fail_running_import_if_needed(
    connection: &mut Connection,
    job_id: i64,
    error: &str,
) -> Result<(), String> {
    let Ok(job) = QueueStore::get_stored_job(connection, job_id) else {
        return Ok(());
    };
    if job.snapshot.status != SyncJobStatusV1::Running {
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
