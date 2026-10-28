use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use tokio::sync::{mpsc, oneshot};

use crate::{assert_schema_latest, configure_connection};

use super::{
    reconcile_manifest, AssetAction, DownloadError, DownloadItemState, ResolvedMediaManifest,
};

#[derive(Debug, Clone)]
pub enum DownloadMutation {
    StartNext,
    Pause {
        job_id: i64,
    },
    Resume {
        job_id: i64,
    },
    PauseRunning,
    Cancel {
        job_id: i64,
    },
    RetryFailed {
        job_id: i64,
    },
    RecomputeJob {
        job_id: i64,
    },
    PrepareReresolve {
        job_id: i64,
        item_id: i64,
    },
    FailItem {
        job_id: i64,
        item_id: i64,
        error_code: String,
        error_message: String,
    },
    InterruptJob {
        job_id: i64,
        error_code: String,
    },
    StartAsset {
        job_id: i64,
        item_id: i64,
        asset_id: i64,
    },
    CompleteAsset {
        job_id: i64,
        item_id: i64,
        asset_id: i64,
        downloaded_bytes: i64,
        sha256: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
    FailAsset {
        job_id: i64,
        item_id: i64,
        asset_id: i64,
        downloaded_bytes: i64,
        error_code: String,
    },
    FinalizeAssetBatch {
        job_id: i64,
        item_id: i64,
        error_code: Option<String>,
        error_message: Option<String>,
    },
    SetItemState {
        job_id: i64,
        aweme_id: String,
        expected_state: DownloadItemState,
        next_state: DownloadItemState,
        downloaded_bytes: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadJobSnapshot {
    pub id: i64,
    pub state: String,
    pub candidate_count: i64,
    pub resolved_count: i64,
    pub completed_count: i64,
    pub skipped_count: i64,
    pub failed_count: i64,
    pub downloaded_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadEvent {
    job: Option<DownloadJobSnapshot>,
}

impl DownloadEvent {
    pub fn job(&self) -> Option<&DownloadJobSnapshot> {
        self.job.as_ref()
    }
}

#[derive(Clone)]
pub struct PersistenceHandle {
    tx: mpsc::Sender<PersistenceRequest>,
}

enum PersistenceRequest {
    Transition {
        mutation: DownloadMutation,
        reply: oneshot::Sender<Result<DownloadEvent, DownloadError>>,
    },
    Reconcile {
        item_id: i64,
        manifest: ResolvedMediaManifest,
        reply: oneshot::Sender<Result<Vec<AssetAction>, DownloadError>>,
    },
}

impl PersistenceHandle {
    pub async fn spawn(path: impl AsRef<Path>, capacity: usize) -> Result<Self, DownloadError> {
        let path = PathBuf::from(path.as_ref());
        let (tx, mut rx) = mpsc::channel::<PersistenceRequest>(capacity.max(1));
        let (ready_tx, ready_rx) = oneshot::channel();
        std::thread::Builder::new()
            .name("download-persistence".to_string())
            .spawn(move || {
                let connection =
                    Connection::open(path)
                        .map_err(database_error)
                        .and_then(|connection| {
                            configure_connection(&connection).map_err(message_error)?;
                            assert_schema_latest(&connection).map_err(message_error)?;
                            Ok(connection)
                        });
                let mut connection = match connection {
                    Ok(connection) => {
                        let _ = ready_tx.send(Ok(()));
                        connection
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                while let Some(request) = rx.blocking_recv() {
                    match request {
                        PersistenceRequest::Transition { mutation, reply } => {
                            let _ = reply.send(apply(&mut connection, mutation));
                        }
                        PersistenceRequest::Reconcile {
                            item_id,
                            manifest,
                            reply,
                        } => {
                            let _ =
                                reply.send(reconcile_manifest(&mut connection, item_id, &manifest));
                        }
                    }
                }
            })
            .map_err(message_error)?;
        ready_rx.await.map_err(actor_stopped)??;
        Ok(Self { tx })
    }

    pub async fn transition(
        &self,
        mutation: DownloadMutation,
    ) -> Result<DownloadEvent, DownloadError> {
        let (reply, response) = oneshot::channel();
        self.tx
            .send(PersistenceRequest::Transition { mutation, reply })
            .await
            .map_err(|_| actor_stopped(()))?;
        response.await.map_err(actor_stopped)?
    }

    pub async fn reconcile_manifest(
        &self,
        item_id: i64,
        manifest: ResolvedMediaManifest,
    ) -> Result<Vec<AssetAction>, DownloadError> {
        let (reply, response) = oneshot::channel();
        self.tx
            .send(PersistenceRequest::Reconcile {
                item_id,
                manifest,
                reply,
            })
            .await
            .map_err(|_| actor_stopped(()))?;
        response.await.map_err(actor_stopped)?
    }
}

fn apply(
    connection: &mut Connection,
    mutation: DownloadMutation,
) -> Result<DownloadEvent, DownloadError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let target = mutate(&transaction, mutation)?;
    if let Some(item_id) = target.item_id {
        derive_item(&transaction, item_id)?;
    }
    if let Some(job_id) = target.job_id {
        derive_aggregates(&transaction, job_id, target.finalize_job)?;
    }
    let event = DownloadEvent {
        job: target
            .job_id
            .map(|id| snapshot(&transaction, id))
            .transpose()?,
    };
    transaction.commit().map_err(database_error)?;
    Ok(event)
}

struct MutationTarget {
    job_id: Option<i64>,
    item_id: Option<i64>,
    finalize_job: bool,
}

fn job_target(job_id: Option<i64>) -> MutationTarget {
    MutationTarget {
        job_id,
        item_id: None,
        finalize_job: false,
    }
}

fn mutate(
    tx: &Transaction<'_>,
    mutation: DownloadMutation,
) -> Result<MutationTarget, DownloadError> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    match mutation {
        DownloadMutation::StartNext => {
            let active: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM download_jobs WHERE state = 'running'",
                    [],
                    |r| r.get(0),
                )
                .map_err(database_error)?;
            if active != 0 {
                return Ok(job_target(None));
            }
            let id = tx
                .query_row(
                    "SELECT id FROM download_jobs WHERE state = 'queued' ORDER BY id LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .map_err(database_error)?;
            if let Some(id) = id {
                tx.execute("UPDATE download_jobs SET state='running', started_at=COALESCE(started_at, ?2) WHERE id=?1", params![id, now]).map_err(database_error)?;
            }
            Ok(job_target(id))
        }
        DownloadMutation::Pause { job_id } => {
            let result = update_job(tx, job_id, "running", "paused", "UPDATE download_items SET state='paused', updated_at=?2 WHERE job_id=?1 AND state IN ('queued','resolving','downloading')", &now)?;
            tx.execute("UPDATE download_assets SET state='paused', updated_at=?2 WHERE item_id IN (SELECT id FROM download_items WHERE job_id=?1) AND state IN ('queued','downloading')", params![job_id, now]).map_err(database_error)?;
            Ok(job_target(result))
        }
        DownloadMutation::Resume { job_id } => {
            ensure_no_other_running(tx, job_id)?;
            let changed = tx.execute("UPDATE download_jobs SET state='running', started_at=COALESCE(started_at,?2), error_code=NULL WHERE id=?1 AND state IN ('queued','paused','interrupted')", params![job_id, now]).map_err(database_error)?;
            if changed == 0 {
                return Err(invalid_transition());
            }
            tx.execute("UPDATE download_items SET state='queued', error_code=NULL, error_message=NULL, updated_at=?2, completed_at=NULL WHERE job_id=?1 AND state IN ('paused','interrupted')", params![job_id, now]).map_err(database_error)?;
            tx.execute("UPDATE download_assets SET state='queued', last_error_code=NULL, updated_at=?2, completed_at=NULL WHERE item_id IN (SELECT id FROM download_items WHERE job_id=?1) AND state IN ('paused','interrupted')", params![job_id, now]).map_err(database_error)?;
            Ok(job_target(Some(job_id)))
        }
        DownloadMutation::PauseRunning => {
            let job_id = tx
                .query_row(
                    "SELECT id FROM download_jobs WHERE state='running' ORDER BY id LIMIT 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(database_error)?;
            let Some(job_id) = job_id else {
                return Ok(job_target(None));
            };
            tx.execute(
                "UPDATE download_jobs SET state='paused' WHERE id=?1 AND state='running'",
                [job_id],
            )
            .map_err(database_error)?;
            tx.execute("UPDATE download_items SET state='paused', updated_at=?2 WHERE job_id=?1 AND state IN ('queued','resolving','downloading')", params![job_id, now]).map_err(database_error)?;
            tx.execute("UPDATE download_assets SET state='paused', updated_at=?2 WHERE item_id IN (SELECT id FROM download_items WHERE job_id=?1) AND state IN ('queued','downloading')", params![job_id, now]).map_err(database_error)?;
            Ok(job_target(Some(job_id)))
        }
        DownloadMutation::Cancel { job_id } => {
            let changed = tx.execute("UPDATE download_jobs SET state='cancelled', completed_at=?2 WHERE id=?1 AND state IN ('queued','running','paused','interrupted')", params![job_id, now]).map_err(database_error)?;
            if changed == 0 {
                return Err(invalid_transition());
            }
            tx.execute("UPDATE download_items SET state='cancelled', updated_at=?2, completed_at=?2 WHERE job_id=?1 AND state NOT IN ('completed','skipped','failed','superseded','cancelled')", params![job_id, now]).map_err(database_error)?;
            tx.execute("UPDATE download_assets SET state='cancelled', updated_at=?2, completed_at=?2 WHERE item_id IN (SELECT id FROM download_items WHERE job_id=?1) AND state NOT IN ('completed','skipped','failed','superseded','cancelled')", params![job_id, now]).map_err(database_error)?;
            Ok(job_target(Some(job_id)))
        }
        DownloadMutation::RetryFailed { job_id } => {
            ensure_no_other_running(tx, job_id)?;
            let changed = tx.execute("UPDATE download_items SET state='queued', error_code=NULL, error_message=NULL, updated_at=?2 WHERE job_id=?1 AND state='failed'", params![job_id, now]).map_err(database_error)?;
            if changed == 0 {
                return Err(invalid_transition());
            }
            tx.execute("UPDATE download_jobs SET state='running', completed_at=NULL, error_code=NULL WHERE id=?1 AND state NOT IN ('cancelled','completed')", [job_id]).map_err(database_error)?;
            Ok(job_target(Some(job_id)))
        }
        DownloadMutation::RecomputeJob { job_id } => Ok(MutationTarget {
            job_id: Some(job_id),
            item_id: None,
            finalize_job: true,
        }),
        DownloadMutation::PrepareReresolve { job_id, item_id } => {
            let changed = tx.execute("UPDATE download_items SET state='resolving', resolution_attempt_count=resolution_attempt_count+1, error_code=NULL, error_message=NULL, updated_at=?3, completed_at=NULL WHERE id=?1 AND job_id=?2 AND state='downloading' AND EXISTS (SELECT 1 FROM download_jobs WHERE id=?2 AND state='running')", params![item_id, job_id, now]).map_err(database_error)?;
            if changed == 0 {
                return Err(stale_mutation());
            }
            Ok(MutationTarget {
                job_id: Some(job_id),
                item_id: None,
                finalize_job: false,
            })
        }
        DownloadMutation::FailItem {
            job_id,
            item_id,
            error_code,
            error_message,
        } => {
            let changed = tx.execute("UPDATE download_items SET state='failed', error_code=?3, error_message=?4, updated_at=?5, completed_at=?5 WHERE id=?1 AND job_id=?2 AND state IN ('queued','resolving','downloading') AND EXISTS (SELECT 1 FROM download_jobs WHERE id=?2 AND state='running')", params![item_id, job_id, error_code, error_message, now]).map_err(database_error)?;
            if changed == 0 {
                return Err(stale_mutation());
            }
            tx.execute("UPDATE download_assets SET state='failed', last_error_code=?3, updated_at=?4, completed_at=?4 WHERE item_id=?1 AND state IN ('queued','downloading') AND EXISTS (SELECT 1 FROM download_items WHERE id=?1 AND job_id=?2)", params![item_id, job_id, error_code, now]).map_err(database_error)?;
            Ok(MutationTarget {
                job_id: Some(job_id),
                item_id: None,
                finalize_job: true,
            })
        }
        DownloadMutation::InterruptJob { job_id, error_code } => {
            tx.execute("UPDATE download_jobs SET state='interrupted', error_code=?2 WHERE id=?1 AND state IN ('queued','running')", params![job_id, error_code]).map_err(database_error)?;
            tx.execute("UPDATE download_items SET state='interrupted', error_code=COALESCE(error_code,?2), updated_at=?3 WHERE job_id=?1 AND state IN ('resolving','downloading')", params![job_id, error_code, now]).map_err(database_error)?;
            tx.execute("UPDATE download_assets SET state='interrupted', last_error_code=COALESCE(last_error_code,?2), updated_at=?3 WHERE item_id IN (SELECT id FROM download_items WHERE job_id=?1) AND state IN ('queued','downloading')", params![job_id, error_code, now]).map_err(database_error)?;
            Ok(job_target(Some(job_id)))
        }
        DownloadMutation::StartAsset {
            job_id,
            item_id,
            asset_id,
        } => {
            let changed = tx.execute("UPDATE download_assets SET state='downloading', updated_at=?4 WHERE id=?1 AND item_id=?2 AND state='queued' AND EXISTS (SELECT 1 FROM download_jobs WHERE id=?3 AND state='running')", params![asset_id, item_id, job_id, now]).map_err(database_error)?;
            if changed == 0 {
                return Err(stale_mutation());
            }
            Ok(MutationTarget {
                job_id: Some(job_id),
                item_id: None,
                finalize_job: false,
            })
        }
        DownloadMutation::CompleteAsset {
            job_id,
            item_id,
            asset_id,
            downloaded_bytes,
            sha256,
            etag,
            last_modified,
        } => {
            let changed = tx.execute("UPDATE download_assets SET state='completed', downloaded_bytes=?4, validated_size=?4, validated_sha256=?5, validated_at=?6, etag=?7, last_modified=?8, last_error_code=NULL, updated_at=?6, completed_at=?6 WHERE id=?1 AND item_id=?2 AND state='downloading' AND EXISTS (SELECT 1 FROM download_jobs WHERE id=?3 AND state='running')", params![asset_id, item_id, job_id, downloaded_bytes.max(0), sha256, now, etag, last_modified]).map_err(database_error)?;
            if changed == 0 {
                return Err(stale_mutation());
            }
            Ok(MutationTarget {
                job_id: Some(job_id),
                item_id: None,
                finalize_job: false,
            })
        }
        DownloadMutation::FailAsset {
            job_id,
            item_id,
            asset_id,
            downloaded_bytes,
            error_code,
        } => {
            let changed = tx.execute("UPDATE download_assets SET state='failed', downloaded_bytes=?4, last_error_code=?5, updated_at=?6, completed_at=?6 WHERE id=?1 AND item_id=?2 AND state='downloading' AND EXISTS (SELECT 1 FROM download_jobs WHERE id=?3 AND state='running')", params![asset_id, item_id, job_id, downloaded_bytes.max(0), error_code, now]).map_err(database_error)?;
            if changed == 0 {
                return Err(stale_mutation());
            }
            Ok(MutationTarget {
                job_id: Some(job_id),
                item_id: None,
                finalize_job: false,
            })
        }
        DownloadMutation::FinalizeAssetBatch {
            job_id,
            item_id,
            error_code,
            error_message,
        } => {
            let active: i64 = tx.query_row("SELECT COUNT(*) FROM download_items WHERE id=?1 AND job_id=?2 AND state='downloading' AND EXISTS (SELECT 1 FROM download_jobs WHERE id=?2 AND state='running')", params![item_id, job_id], |row| row.get(0)).map_err(database_error)?;
            if active == 0 {
                return Err(stale_mutation());
            }
            if let Some(error_code) = error_code {
                tx.execute("UPDATE download_assets SET state='failed', last_error_code=COALESCE(last_error_code,?2), updated_at=?3, completed_at=?3 WHERE item_id=?1 AND state IN ('queued','downloading')", params![item_id, error_code, now]).map_err(database_error)?;
                tx.execute("UPDATE download_items SET error_code=COALESCE(error_code,?2), error_message=COALESCE(error_message,?3), updated_at=?4 WHERE id=?1", params![item_id, error_code, error_message, now]).map_err(database_error)?;
            }
            Ok(MutationTarget {
                job_id: Some(job_id),
                item_id: Some(item_id),
                finalize_job: true,
            })
        }
        DownloadMutation::SetItemState {
            job_id,
            aweme_id,
            expected_state,
            next_state,
            downloaded_bytes,
        } => {
            let expected_state = item_state(expected_state);
            let next_state = item_state(next_state);
            let changed = tx.execute("UPDATE download_items SET state=?4, updated_at=?5, completed_at=CASE WHEN ?4 IN ('completed','skipped','failed','cancelled') THEN ?5 ELSE NULL END WHERE job_id=?1 AND aweme_id=?2 AND state=?3 AND EXISTS (SELECT 1 FROM download_jobs WHERE id=?1 AND state='running')", params![job_id, aweme_id, expected_state, next_state, now]).map_err(database_error)?;
            if changed == 0 {
                return Err(stale_mutation());
            }
            tx.execute("UPDATE download_assets SET downloaded_bytes=?3 WHERE item_id IN (SELECT id FROM download_items WHERE job_id=?1 AND aweme_id=?2)", params![job_id, aweme_id, downloaded_bytes.max(0)]).map_err(database_error)?;
            Ok(job_target(Some(job_id)))
        }
    }
}

fn derive_item(tx: &Transaction<'_>, item_id: i64) -> Result<(), DownloadError> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    tx.execute("UPDATE download_items SET completed_asset_count=(SELECT COUNT(*) FROM download_assets WHERE item_id=?1 AND required=1 AND state IN ('completed','skipped')), total_required_asset_count=(SELECT COUNT(*) FROM download_assets WHERE item_id=?1 AND required=1), state=CASE WHEN EXISTS(SELECT 1 FROM download_assets WHERE item_id=?1 AND required=1 AND state='failed') THEN 'failed' WHEN NOT EXISTS(SELECT 1 FROM download_assets WHERE item_id=?1 AND required=1 AND state<>'skipped') THEN 'skipped' WHEN NOT EXISTS(SELECT 1 FROM download_assets WHERE item_id=?1 AND required=1 AND state NOT IN ('completed','skipped')) THEN 'completed' ELSE 'downloading' END, updated_at=?2, completed_at=CASE WHEN EXISTS(SELECT 1 FROM download_assets WHERE item_id=?1 AND required=1 AND state='failed') OR NOT EXISTS(SELECT 1 FROM download_assets WHERE item_id=?1 AND required=1 AND state NOT IN ('completed','skipped')) THEN ?2 ELSE NULL END WHERE id=?1", params![item_id, now]).map_err(database_error)?;
    Ok(())
}

fn ensure_no_other_running(tx: &Transaction<'_>, job_id: i64) -> Result<(), DownloadError> {
    let active: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM download_jobs WHERE state='running' AND id<>?1",
            [job_id],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    if active == 0 {
        Ok(())
    } else {
        Err(invalid_transition())
    }
}

fn update_job(
    tx: &Transaction<'_>,
    id: i64,
    from: &str,
    to: &str,
    item_sql: &str,
    now: &str,
) -> Result<Option<i64>, DownloadError> {
    if tx
        .execute(
            "UPDATE download_jobs SET state=?3 WHERE id=?1 AND state=?2",
            params![id, from, to],
        )
        .map_err(database_error)?
        == 0
    {
        return Err(invalid_transition());
    }
    tx.execute(item_sql, params![id, now])
        .map_err(database_error)?;
    Ok(Some(id))
}

fn derive_aggregates(
    tx: &Transaction<'_>,
    id: i64,
    finalize_job: bool,
) -> Result<(), DownloadError> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    tx.execute("UPDATE download_jobs SET resolved_count=(SELECT COUNT(*) FROM download_items WHERE job_id=?1 AND state NOT IN ('queued','resolving')), completed_count=(SELECT COUNT(*) FROM download_items WHERE job_id=?1 AND state='completed'), skipped_count=(SELECT COUNT(*) FROM download_items WHERE job_id=?1 AND state='skipped'), failed_count=(SELECT COUNT(*) FROM download_items WHERE job_id=?1 AND state='failed'), total_bytes=COALESCE((SELECT SUM(a.expected_size) FROM download_assets a JOIN download_items i ON i.id=a.item_id WHERE i.job_id=?1 AND a.required=1),0), downloaded_bytes=COALESCE((SELECT SUM(a.downloaded_bytes) FROM download_assets a JOIN download_items i ON i.id=a.item_id WHERE i.job_id=?1),0), state=CASE WHEN ?3=1 AND state='running' AND NOT EXISTS(SELECT 1 FROM download_items WHERE job_id=?1 AND state NOT IN ('completed','skipped','failed','superseded','cancelled')) THEN CASE WHEN EXISTS(SELECT 1 FROM download_items WHERE job_id=?1 AND state='failed') THEN 'completed_with_errors' ELSE 'completed' END ELSE state END, completed_at=CASE WHEN ?3=1 AND state='running' AND NOT EXISTS(SELECT 1 FROM download_items WHERE job_id=?1 AND state NOT IN ('completed','skipped','failed','superseded','cancelled')) THEN ?2 ELSE completed_at END WHERE id=?1", params![id, now, finalize_job as i64]).map_err(database_error)?;
    Ok(())
}

fn snapshot(tx: &Transaction<'_>, id: i64) -> Result<DownloadJobSnapshot, DownloadError> {
    tx.query_row("SELECT id,state,candidate_count,resolved_count,completed_count,skipped_count,failed_count,downloaded_bytes FROM download_jobs WHERE id=?1", [id], |r| Ok(DownloadJobSnapshot { id:r.get(0)?, state:r.get(1)?, candidate_count:r.get(2)?, resolved_count:r.get(3)?, completed_count:r.get(4)?, skipped_count:r.get(5)?, failed_count:r.get(6)?, downloaded_bytes:r.get(7)? })).map_err(database_error)
}

fn item_state(state: DownloadItemState) -> &'static str {
    match state {
        DownloadItemState::Queued => "queued",
        DownloadItemState::Resolving => "resolving",
        DownloadItemState::Downloading => "downloading",
        DownloadItemState::Paused => "paused",
        DownloadItemState::Interrupted => "interrupted",
        DownloadItemState::Completed => "completed",
        DownloadItemState::Skipped => "skipped",
        DownloadItemState::Failed => "failed",
        DownloadItemState::Superseded => "superseded",
        DownloadItemState::Cancelled => "cancelled",
    }
}
fn database_error(_: rusqlite::Error) -> DownloadError {
    message_error("download persistence failed")
}
fn message_error(message: impl ToString) -> DownloadError {
    DownloadError {
        code: "download_persistence_failed".into(),
        message: message.to_string(),
    }
}
fn actor_stopped<T>(_: T) -> DownloadError {
    message_error("download persistence actor stopped")
}
fn invalid_transition() -> DownloadError {
    DownloadError {
        code: "invalid_download_transition".into(),
        message: "下载任务状态转换无效".into(),
    }
}

fn stale_mutation() -> DownloadError {
    DownloadError {
        code: "stale_download_mutation".into(),
        message: "下载状态已变化，忽略过期更新".into(),
    }
}
