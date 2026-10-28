use super::contracts::{
    ImportCountsV1, QueuedSyncContext, RunningSyncIdentity, SourceExecutionResultV1,
    SyncJobSnapshotV1, SyncJobStatusV1, SyncSourceV1,
};
use super::QueueStore;
use chrono::{SecondsFormat, Utc};
use rusqlite::Connection;
use rusqlite::Transaction;
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
};

pub trait SyncExecutor: Send + Sync {
    fn source(&self) -> SyncSourceV1;

    fn start(&self, context: QueuedSyncContext) -> Result<RunningSyncIdentity, String>;

    fn cancel(&self, running: &RunningSyncIdentity) -> Result<(), String>;
}

pub trait SyncResultImporter: Send + Sync {
    fn source(&self) -> SyncSourceV1;

    fn validate_and_import(
        &self,
        transaction: &Transaction<'_>,
        job: &SyncJobSnapshotV1,
        result: SourceExecutionResultV1,
    ) -> Result<ImportCountsV1, String>;
}

pub struct ExecutorRegistration {
    executor: Arc<dyn SyncExecutor>,
    importer: Arc<dyn SyncResultImporter>,
}

impl ExecutorRegistration {
    pub fn new(executor: Arc<dyn SyncExecutor>, importer: Arc<dyn SyncResultImporter>) -> Self {
        Self { executor, importer }
    }
}

pub struct ExecutorRegistry {
    registrations: BTreeMap<SyncSourceV1, ExecutorRegistration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncImportOutcomeV1 {
    pub counts: ImportCountsV1,
    pub status: SyncJobStatusV1,
}

impl ExecutorRegistry {
    pub fn new(registrations: Vec<ExecutorRegistration>) -> Result<Self, String> {
        let mut by_source = BTreeMap::new();
        for registration in registrations {
            let source = registration.executor.source();
            if registration.importer.source() != source {
                return Err("sync_executor_source_mismatch".to_string());
            }
            if by_source.insert(source, registration).is_some() {
                return Err("duplicate_sync_executor".to_string());
            }
        }
        Ok(Self {
            registrations: by_source,
        })
    }

    pub fn executor(&self, source: SyncSourceV1) -> Option<&dyn SyncExecutor> {
        self.registrations
            .get(&source)
            .map(|registration| registration.executor.as_ref())
    }

    pub fn importer(&self, source: SyncSourceV1) -> Option<&dyn SyncResultImporter> {
        self.registrations
            .get(&source)
            .map(|registration| registration.importer.as_ref())
    }
}

impl fmt::Debug for ExecutorRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutorRegistry")
            .field("sources", &self.registrations.keys().collect::<Vec<_>>())
            .finish()
    }
}

pub fn import_execution_envelope(
    registry: &ExecutorRegistry,
    connection: &mut Connection,
    expected_job_id: i64,
    envelope: super::SyncExecutionEnvelopeV1,
) -> Result<SyncImportOutcomeV1, String> {
    let imported = (|| {
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let job = QueueStore::get_stored_job(&transaction, expected_job_id)?;
        if job.snapshot.status != SyncJobStatusV1::Running {
            return Err("sync_job_not_running".to_string());
        }
        super::validate_execution_envelope(&envelope, &job.snapshot)?;
        let importer = registry
            .importer(job.snapshot.task_key.source)
            .ok_or_else(|| "sync_source_not_available".to_string())?;
        let counts = importer.validate_and_import(&transaction, &job.snapshot, envelope.result)?;
        let finished = QueueStore::finish_running_job_tx(
            &transaction,
            expected_job_id,
            &super::TerminalJobUpdateV1 {
                status: SyncJobStatusV1::Success,
                finished_at: sync_import_now_iso(),
                inserted_videos: counts.inserted_videos,
                inserted_records: counts.inserted_records,
                error_code: None,
                error_message: None,
            },
        )?;
        let outcome = SyncImportOutcomeV1 {
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
        &super::TerminalJobUpdateV1 {
            status: SyncJobStatusV1::Failed,
            finished_at: sync_import_now_iso(),
            inserted_videos: 0,
            inserted_records: 0,
            error_code: Some("sync_result_import_failed".to_string()),
            error_message: Some(error.to_string()),
        },
    )?;
    Ok(())
}

fn sync_import_now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn cancel_sync_job(
    connection: &mut Connection,
    registry: &ExecutorRegistry,
    account_id: i64,
    job_id: i64,
    running: Option<&RunningSyncIdentity>,
    finished_at: &str,
) -> Result<SyncJobStatusV1, String> {
    let job = QueueStore::get_stored_job(connection, job_id)?;
    if account_id <= 0 || job.snapshot.task_key.account_id != account_id {
        return Err("sync_job_mismatch".to_string());
    }
    match job.snapshot.status {
        SyncJobStatusV1::Queued => Ok(QueueStore::cancel_queued_job(
            connection,
            job_id,
            finished_at,
            "cancelled_by_user",
        )?
        .snapshot
        .status),
        SyncJobStatusV1::Running => {
            let running = running
                .filter(|running| running.job_id == job_id)
                .ok_or_else(|| "sync_worker_running_mismatch".to_string())?;
            let executor = registry
                .executor(job.snapshot.task_key.source)
                .ok_or_else(|| "sync_source_not_available".to_string())?;
            executor.cancel(running)?;
            Ok(SyncJobStatusV1::Running)
        }
        status => Ok(status),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeSyncCall {
    Start { job_id: i64 },
    Cancel { job_id: i64 },
    Import { job_id: i64, source: SyncSourceV1 },
}

#[derive(Debug)]
pub struct FakeSyncBackend {
    source: SyncSourceV1,
    launch_error: Option<String>,
    calls: Mutex<Vec<FakeSyncCall>>,
}

impl FakeSyncBackend {
    pub fn new(source: SyncSourceV1) -> Self {
        Self {
            source,
            launch_error: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn with_launch_error(source: SyncSourceV1, error: impl Into<String>) -> Self {
        Self {
            source,
            launch_error: Some(error.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<FakeSyncCall> {
        self.calls
            .lock()
            .expect("fake sync backend mutex poisoned")
            .clone()
    }

    fn record(&self, call: FakeSyncCall) -> Result<(), String> {
        self.calls
            .lock()
            .map_err(|_| "fake_sync_backend_poisoned".to_string())?
            .push(call);
        Ok(())
    }
}

impl SyncExecutor for FakeSyncBackend {
    fn source(&self) -> SyncSourceV1 {
        self.source
    }

    fn start(&self, context: QueuedSyncContext) -> Result<RunningSyncIdentity, String> {
        if context.job.task_key.source != self.source {
            return Err("sync_executor_source_mismatch".to_string());
        }
        self.record(FakeSyncCall::Start {
            job_id: context.job.id,
        })?;
        if let Some(error) = &self.launch_error {
            return Err(error.clone());
        }
        Ok(RunningSyncIdentity {
            job_id: context.job.id,
            process_id: None,
        })
    }

    fn cancel(&self, running: &RunningSyncIdentity) -> Result<(), String> {
        self.record(FakeSyncCall::Cancel {
            job_id: running.job_id,
        })
    }
}

impl SyncResultImporter for FakeSyncBackend {
    fn source(&self) -> SyncSourceV1 {
        self.source
    }

    fn validate_and_import(
        &self,
        _transaction: &Transaction<'_>,
        job: &SyncJobSnapshotV1,
        result: SourceExecutionResultV1,
    ) -> Result<ImportCountsV1, String> {
        if job.task_key.source != self.source || result.source() != self.source {
            return Err("sync_executor_source_mismatch".to_string());
        }
        self.record(FakeSyncCall::Import {
            job_id: job.id,
            source: result.source(),
        })?;
        Ok(ImportCountsV1::default())
    }
}
