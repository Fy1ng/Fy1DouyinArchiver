use super::{
    freeze_task_payload, shanghai_cutoff_ms, validate_enqueue_request, ArchiveRunModeV1,
    BrowserExecutionIdentityV1, EnqueueSyncRequestV1, EnqueueSyncResultV1, FrozenTaskPayloadV1,
    NewSyncBatchIssueKindV1, NewSyncBatchIssueV1, NewSyncJobV1, QueueStore, RangeFreezeV1,
    RemoteFolderExecutionOverrideV1, SourceCapabilityViewV1, SyncBatchCountsV1, SyncBatchEntryV1,
    SyncRangeKindV1, SyncScopeV1, SyncSourceV1, SyncTaskPayloadV1, SyncTriggerV1, TaskKeyV1,
};
use crate::settings_store::{
    AccountSettingsV2, ArchiveListRangeV1, HistoryRangeV1, RemoteFolderScheduleModeV1,
};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, TransactionBehavior};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnqueueSettingsSnapshotV1 {
    pub revision: u64,
    pub accounts: BTreeMap<i64, AccountSettingsV2>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnqueueDeviceSnapshotV1 {
    pub browsers: BTreeMap<i64, BrowserExecutionIdentityV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskDecisionDispositionV1 {
    Created,
    Attached,
    Promoted,
    Replaced,
    Conflict,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDecisionBatchTargetV1 {
    pub batch_id: i64,
    pub role: super::BatchJobRoleV1,
    pub phase: super::SyncBatchIssuePhaseV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDecisionInputV1 {
    pub task_key: TaskKeyV1,
    pub trigger: SyncTriggerV1,
    pub queued_at: String,
    pub scheduled_for: Option<String>,
    pub settings_revision: i64,
    pub config_sha256: Option<String>,
    pub payload: FrozenTaskPayloadV1,
    pub batch_targets: Vec<TaskDecisionBatchTargetV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDecisionV1 {
    pub disposition: TaskDecisionDispositionV1,
    pub job: Option<super::StoredSyncJobV1>,
    pub existing_job: Option<super::StoredSyncJobV1>,
}

pub struct EnqueueService<'a> {
    connection: &'a mut Connection,
    now: DateTime<Utc>,
}

impl<'a> EnqueueService<'a> {
    pub fn new(connection: &'a mut Connection) -> Self {
        Self {
            connection,
            now: Utc::now(),
        }
    }

    pub fn at(connection: &'a mut Connection, now: &str) -> Result<Self, String> {
        let now = DateTime::parse_from_rfc3339(now)
            .map_err(|_| "invalid_sync_timestamp".to_string())?
            .with_timezone(&Utc);
        Ok(Self { connection, now })
    }

    pub fn enqueue(
        &mut self,
        request: &EnqueueSyncRequestV1,
        settings: &EnqueueSettingsSnapshotV1,
        device: &EnqueueDeviceSnapshotV1,
        capabilities: &[SourceCapabilityViewV1],
    ) -> Result<EnqueueSyncResultV1, String> {
        validate_enqueue_request(request)?;
        let settings_revision = i64::try_from(settings.revision)
            .map_err(|_| "settings_revision_overflow".to_string())?;
        match request.scope {
            SyncScopeV1::One => enqueue_one(
                self.connection,
                request,
                settings,
                device,
                capabilities,
                settings_revision,
                self.now,
            ),
            SyncScopeV1::CurrentAccountAvailableSources | SyncScopeV1::AllActiveAccounts => {
                enqueue_bulk(
                    self.connection,
                    request,
                    None,
                    settings,
                    device,
                    capabilities,
                    settings_revision,
                    self.now,
                )
            }
        }
    }

    pub fn enqueue_for_account_ids(
        &mut self,
        request: &EnqueueSyncRequestV1,
        account_ids: &[i64],
        settings: &EnqueueSettingsSnapshotV1,
        device: &EnqueueDeviceSnapshotV1,
        capabilities: &[SourceCapabilityViewV1],
    ) -> Result<EnqueueSyncResultV1, String> {
        validate_enqueue_request(request)?;
        if request.scope != SyncScopeV1::AllActiveAccounts
            || account_ids.is_empty()
            || account_ids.iter().any(|account_id| *account_id <= 0)
            || account_ids.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err("account_scope_invalid".to_string());
        }
        let settings_revision = i64::try_from(settings.revision)
            .map_err(|_| "settings_revision_overflow".to_string())?;
        enqueue_bulk(
            self.connection,
            request,
            Some(account_ids),
            settings,
            device,
            capabilities,
            settings_revision,
            self.now,
        )
    }
}

fn enqueue_one(
    connection: &mut Connection,
    request: &EnqueueSyncRequestV1,
    settings: &EnqueueSettingsSnapshotV1,
    device: &EnqueueDeviceSnapshotV1,
    capabilities: &[SourceCapabilityViewV1],
    settings_revision: i64,
    now: DateTime<Utc>,
) -> Result<EnqueueSyncResultV1, String> {
    let account_id = request.account_id.expect("validated account");
    let source = request.source.expect("validated source");
    ensure_active_account(connection, account_id)?;
    let capability = available_capability(capabilities, source)?;
    let payload = match &request.payload {
        Some(payload) => payload.clone(),
        None => build_implicit_payload(
            account_id,
            source,
            request.source_instance_key.as_deref().unwrap_or_default(),
            settings,
            device,
            capabilities,
            now.timestamp_millis(),
        )?,
    };
    validate_payload_capability(&payload, capability)?;
    let queued_at = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let input = TaskDecisionInputV1 {
        task_key: TaskKeyV1 {
            account_id,
            source,
            source_instance_key: request.source_instance_key.clone().unwrap_or_default(),
        },
        trigger: SyncTriggerV1::Manual,
        queued_at,
        scheduled_for: None,
        settings_revision,
        config_sha256: None,
        payload: freeze_task_payload(payload)?,
        batch_targets: Vec::new(),
    };
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let decision = decide_enqueue_for_task_key_tx(&transaction, &input)?;
    if decision.disposition == TaskDecisionDispositionV1::Conflict {
        return Err("sync_task_payload_conflict".to_string());
    }
    let job_id = decision
        .job
        .as_ref()
        .map(|job| job.snapshot.id)
        .ok_or_else(|| "sync_enqueue_failed".to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    let view = QueueStore::get_job_view(connection, job_id)?;
    Ok(EnqueueSyncResultV1 {
        batch_id: None,
        counts: SyncBatchCountsV1 {
            jobs: 1,
            skipped: 0,
            conflicts: 0,
        },
        entries: vec![SyncBatchEntryV1::Job(view)],
        next_cursor: None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BulkTask {
    task_key: TaskKeyV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AggregateSkip {
    account_id: i64,
    reason: &'static str,
}

fn enqueue_bulk(
    connection: &mut Connection,
    request: &EnqueueSyncRequestV1,
    selected_account_ids: Option<&[i64]>,
    settings: &EnqueueSettingsSnapshotV1,
    device: &EnqueueDeviceSnapshotV1,
    capabilities: &[SourceCapabilityViewV1],
    settings_revision: i64,
    now: DateTime<Utc>,
) -> Result<EnqueueSyncResultV1, String> {
    let capability_map = capability_map(capabilities)?;
    let available_sources = capability_map
        .iter()
        .filter_map(|(source, capability)| capability.available.then_some(*source))
        .collect::<Vec<_>>();
    let queued_at = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let account_ids = bulk_account_ids(&transaction, request, selected_account_ids)?;
    if available_sources.is_empty() {
        return Err("no_available_sources".to_string());
    }
    let (tasks, aggregate_skips) =
        expand_bulk_targets(&transaction, &account_ids, &available_sources)?;
    if tasks.is_empty() && aggregate_skips.is_empty() {
        return Err("no_available_sources".to_string());
    }
    if tasks.len() > 5_000 {
        return Err("sync_batch_target_limit_exceeded".to_string());
    }
    let active_batches: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM sync_batches WHERE status IN ('queued', 'running')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if active_batches >= 16 {
        return Err("sync_active_batch_limit_exceeded".to_string());
    }

    let batch_id = QueueStore::create_batch_tx(&transaction, request.scope, &queued_at)?;
    for aggregate in aggregate_skips {
        append_aggregate_skip(
            &transaction,
            batch_id,
            aggregate.account_id,
            aggregate.reason,
            &queued_at,
        )?;
    }
    for task in tasks {
        let capability = capability_map
            .get(&task.task_key.source)
            .copied()
            .ok_or_else(|| "source_unavailable".to_string())?;
        let payload = build_implicit_payload(
            task.task_key.account_id,
            task.task_key.source,
            &task.task_key.source_instance_key,
            settings,
            device,
            capabilities,
            now.timestamp_millis(),
        )?;
        validate_payload_capability(&payload, capability)?;
        decide_enqueue_for_task_key_tx_inner(
            &transaction,
            &TaskDecisionInputV1 {
                task_key: task.task_key,
                trigger: SyncTriggerV1::Manual,
                queued_at: queued_at.clone(),
                scheduled_for: None,
                settings_revision,
                config_sha256: None,
                payload: freeze_task_payload(payload)?,
                batch_targets: vec![TaskDecisionBatchTargetV1 {
                    batch_id,
                    role: super::BatchJobRoleV1::Root,
                    phase: super::SyncBatchIssuePhaseV1::InitialExpansion,
                }],
            },
            false,
        )?;
    }
    QueueStore::recalculate_batch_status_tx(&transaction, batch_id, &queued_at)?;
    transaction.commit().map_err(|error| error.to_string())?;
    let batch = QueueStore::get_batch(connection, batch_id)?;
    let page = QueueStore::list_batch_entries(connection, batch_id, None, 200)?;
    Ok(EnqueueSyncResultV1 {
        batch_id: Some(batch_id),
        counts: batch.counts,
        entries: page.entries,
        next_cursor: Some(page.next_cursor),
    })
}

fn capability_map(
    capabilities: &[SourceCapabilityViewV1],
) -> Result<BTreeMap<SyncSourceV1, &SourceCapabilityViewV1>, String> {
    let mut result = BTreeMap::new();
    for capability in capabilities {
        if result.insert(capability.source, capability).is_some() {
            return Err("invalid_capability_registry".to_string());
        }
    }
    Ok(result)
}

fn bulk_account_ids(
    transaction: &rusqlite::Transaction<'_>,
    request: &EnqueueSyncRequestV1,
    selected_account_ids: Option<&[i64]>,
) -> Result<Vec<i64>, String> {
    match request.scope {
        SyncScopeV1::CurrentAccountAvailableSources => {
            if selected_account_ids.is_some() {
                return Err("account_scope_invalid".to_string());
            }
            let account_id = request.account_id.expect("validated account");
            ensure_active_account(transaction, account_id)?;
            Ok(vec![account_id])
        }
        SyncScopeV1::AllActiveAccounts => {
            if let Some(account_ids) = selected_account_ids {
                for account_id in account_ids {
                    ensure_active_account(transaction, *account_id)
                        .map_err(|_| "account_scope_invalid".to_string())?;
                }
                return Ok(account_ids.to_vec());
            }
            let mut statement = transaction
                .prepare("SELECT id FROM accounts WHERE status = 'active' ORDER BY id")
                .map_err(|error| error.to_string())?;
            let account_ids = statement
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            if account_ids.is_empty() {
                Err("no_active_accounts".to_string())
            } else {
                Ok(account_ids)
            }
        }
        SyncScopeV1::One => Err("invalid_sync_batch_scope".to_string()),
    }
}

fn expand_bulk_targets(
    transaction: &rusqlite::Transaction<'_>,
    account_ids: &[i64],
    available_sources: &[SyncSourceV1],
) -> Result<(Vec<BulkTask>, Vec<AggregateSkip>), String> {
    let mut tasks = Vec::new();
    let mut aggregate_skips = Vec::new();
    for &account_id in account_ids {
        for &source in available_sources {
            if source != SyncSourceV1::RemoteFolder {
                tasks.push(BulkTask {
                    task_key: TaskKeyV1 {
                        account_id,
                        source,
                        source_instance_key: String::new(),
                    },
                });
                continue;
            }
            let count: i64 = transaction
                .query_row(
                    "SELECT COUNT(*) FROM remote_folders
                     WHERE account_id = ?1 AND remote_removed_at IS NULL",
                    [account_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if count > 2_000 {
                aggregate_skips.push(AggregateSkip {
                    account_id,
                    reason: "remote_folder_schedule_limit_exceeded",
                });
                continue;
            }
            let mut statement = transaction
                .prepare(
                    "SELECT remote_folder_id FROM remote_folders
                     WHERE account_id = ?1 AND remote_removed_at IS NULL
                     ORDER BY remote_folder_id",
                )
                .map_err(|error| error.to_string())?;
            let folder_ids = statement
                .query_map([account_id], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            tasks.extend(folder_ids.into_iter().map(|source_instance_key| BulkTask {
                task_key: TaskKeyV1 {
                    account_id,
                    source,
                    source_instance_key,
                },
            }));
        }
    }
    tasks.sort();
    tasks.dedup();
    aggregate_skips.sort_by_key(|skip| skip.account_id);
    Ok((tasks, aggregate_skips))
}

fn append_aggregate_skip(
    transaction: &rusqlite::Transaction<'_>,
    batch_id: i64,
    account_id: i64,
    reason: &str,
    created_at: &str,
) -> Result<(), String> {
    let task_key = TaskKeyV1 {
        account_id,
        source: SyncSourceV1::RemoteFolder,
        source_instance_key: String::new(),
    };
    let issue = NewSyncBatchIssueV1 {
        task_key: task_key.clone(),
        target_kind_aggregate: true,
        phase: super::SyncBatchIssuePhaseV1::InitialExpansion,
        kind: NewSyncBatchIssueKindV1::Skipped {
            reason: reason.to_string(),
            requested_payload_sha256: None,
        },
    };
    let entry_seq = QueueStore::reserve_batch_target_tx(
        transaction,
        batch_id,
        &task_key,
        super::BatchTargetKindV1::Aggregate,
        "issue",
    )?;
    QueueStore::append_reserved_batch_issue_tx(
        transaction,
        batch_id,
        &issue,
        entry_seq,
        created_at,
    )?;
    Ok(())
}

pub fn decide_enqueue_for_task_key_tx(
    transaction: &rusqlite::Transaction<'_>,
    input: &TaskDecisionInputV1,
) -> Result<TaskDecisionV1, String> {
    decide_enqueue_for_task_key_tx_inner(transaction, input, true)
}

fn decide_enqueue_for_task_key_tx_inner(
    transaction: &rusqlite::Transaction<'_>,
    input: &TaskDecisionInputV1,
    recalculate_batches: bool,
) -> Result<TaskDecisionV1, String> {
    validate_decision_input(input)?;
    let targets = sorted_batch_targets(&input.batch_targets)?;
    let candidate = decision_job(input);
    let existing = QueueStore::find_active_job_tx(transaction, &input.task_key)?;

    if targets.len() > 4 && existing.is_none() {
        append_limit_skips(transaction, input, &targets, recalculate_batches)?;
        return Ok(TaskDecisionV1 {
            disposition: TaskDecisionDispositionV1::Skipped,
            job: None,
            existing_job: None,
        });
    }
    if let Some(existing) = &existing {
        let active_associations: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM sync_batch_jobs links
                 JOIN sync_batches batches ON batches.id = links.batch_id
                 WHERE links.job_id = ?1 AND batches.status IN ('queued', 'running')",
                [existing.snapshot.id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let requested = i64::try_from(targets.len())
            .map_err(|_| "sync_job_batch_attachment_limit_exceeded".to_string())?;
        if active_associations + requested > 4 {
            append_limit_conflicts(transaction, input, &targets, existing, recalculate_batches)?;
            return Ok(TaskDecisionV1 {
                disposition: TaskDecisionDispositionV1::Conflict,
                job: None,
                existing_job: Some(existing.clone()),
            });
        }
    }

    let Some(existing) = existing else {
        let stored = QueueStore::enqueue_job_tx(transaction, &candidate)?;
        attach_job_targets(
            transaction,
            input,
            &targets,
            stored.snapshot.id,
            recalculate_batches,
        )?;
        return Ok(TaskDecisionV1 {
            disposition: TaskDecisionDispositionV1::Created,
            job: Some(stored),
            existing_job: None,
        });
    };
    let equivalent = existing.equivalence_sha256 == input.payload.equivalence_sha256;
    let automatic_config_matches = existing.config_sha256 == input.config_sha256;

    match input.trigger {
        SyncTriggerV1::Manual if equivalent => {
            if existing.snapshot.trigger == SyncTriggerV1::Automatic
                && existing.snapshot.status == super::SyncJobStatusV1::Queued
            {
                let promoted = QueueStore::promote_automatic_job_tx(
                    transaction,
                    existing.snapshot.id,
                    &candidate,
                )?;
                attach_job_targets(
                    transaction,
                    input,
                    &targets,
                    promoted.snapshot.id,
                    recalculate_batches,
                )?;
                Ok(TaskDecisionV1 {
                    disposition: TaskDecisionDispositionV1::Promoted,
                    job: Some(promoted),
                    existing_job: Some(existing),
                })
            } else {
                attach_job_targets(
                    transaction,
                    input,
                    &targets,
                    existing.snapshot.id,
                    recalculate_batches,
                )?;
                Ok(TaskDecisionV1 {
                    disposition: TaskDecisionDispositionV1::Attached,
                    job: Some(existing.clone()),
                    existing_job: Some(existing),
                })
            }
        }
        SyncTriggerV1::Manual
            if existing.snapshot.trigger == SyncTriggerV1::Automatic
                && existing.snapshot.status == super::SyncJobStatusV1::Queued =>
        {
            QueueStore::cancel_queued_job_tx(
                transaction,
                existing.snapshot.id,
                &input.queued_at,
                "automatic_replaced_by_manual",
            )?;
            let stored = QueueStore::enqueue_job_tx(transaction, &candidate)?;
            attach_job_targets(
                transaction,
                input,
                &targets,
                stored.snapshot.id,
                recalculate_batches,
            )?;
            Ok(TaskDecisionV1 {
                disposition: TaskDecisionDispositionV1::Replaced,
                job: Some(stored),
                existing_job: Some(existing),
            })
        }
        SyncTriggerV1::Automatic if equivalent && automatic_config_matches => {
            attach_job_targets(
                transaction,
                input,
                &targets,
                existing.snapshot.id,
                recalculate_batches,
            )?;
            Ok(TaskDecisionV1 {
                disposition: TaskDecisionDispositionV1::Attached,
                job: Some(existing.clone()),
                existing_job: Some(existing),
            })
        }
        SyncTriggerV1::Automatic
            if existing.snapshot.trigger == SyncTriggerV1::Automatic
                && existing.snapshot.status == super::SyncJobStatusV1::Queued =>
        {
            QueueStore::cancel_queued_job_tx(
                transaction,
                existing.snapshot.id,
                &input.queued_at,
                "automatic_settings_changed",
            )?;
            let stored = QueueStore::enqueue_job_tx(transaction, &candidate)?;
            attach_job_targets(
                transaction,
                input,
                &targets,
                stored.snapshot.id,
                recalculate_batches,
            )?;
            Ok(TaskDecisionV1 {
                disposition: TaskDecisionDispositionV1::Replaced,
                job: Some(stored),
                existing_job: Some(existing),
            })
        }
        _ => {
            append_payload_conflicts(transaction, input, &targets, &existing, recalculate_batches)?;
            Ok(TaskDecisionV1 {
                disposition: TaskDecisionDispositionV1::Conflict,
                job: None,
                existing_job: Some(existing),
            })
        }
    }
}

fn sorted_batch_targets(
    targets: &[TaskDecisionBatchTargetV1],
) -> Result<Vec<TaskDecisionBatchTargetV1>, String> {
    let mut targets = targets.to_vec();
    targets.sort_by_key(|target| target.batch_id);
    if targets.iter().any(|target| target.batch_id <= 0)
        || targets
            .windows(2)
            .any(|window| window[0].batch_id == window[1].batch_id)
    {
        return Err("invalid_sync_batch_targets".to_string());
    }
    Ok(targets)
}

fn attach_job_targets(
    transaction: &rusqlite::Transaction<'_>,
    input: &TaskDecisionInputV1,
    targets: &[TaskDecisionBatchTargetV1],
    job_id: i64,
    recalculate_batches: bool,
) -> Result<(), String> {
    for target in targets {
        let entry_seq = QueueStore::reserve_batch_target_tx(
            transaction,
            target.batch_id,
            &input.task_key,
            super::BatchTargetKindV1::Task,
            "job",
        )?;
        QueueStore::attach_reserved_job_tx(
            transaction,
            target.batch_id,
            job_id,
            target.role,
            entry_seq,
        )?;
        if recalculate_batches {
            QueueStore::recalculate_batch_status_tx(
                transaction,
                target.batch_id,
                &input.queued_at,
            )?;
        }
    }
    Ok(())
}

fn append_limit_skips(
    transaction: &rusqlite::Transaction<'_>,
    input: &TaskDecisionInputV1,
    targets: &[TaskDecisionBatchTargetV1],
    recalculate_batches: bool,
) -> Result<(), String> {
    for target in targets {
        append_issue(
            transaction,
            target,
            input,
            NewSyncBatchIssueKindV1::Skipped {
                reason: "sync_job_batch_attachment_limit_exceeded".to_string(),
                requested_payload_sha256: Some(input.payload.payload_sha256.clone()),
            },
            recalculate_batches,
        )?;
    }
    Ok(())
}

fn append_limit_conflicts(
    transaction: &rusqlite::Transaction<'_>,
    input: &TaskDecisionInputV1,
    targets: &[TaskDecisionBatchTargetV1],
    existing: &super::StoredSyncJobV1,
    recalculate_batches: bool,
) -> Result<(), String> {
    append_conflicts_with_reason(
        transaction,
        input,
        targets,
        existing,
        "sync_job_batch_attachment_limit_exceeded",
        recalculate_batches,
    )
}

fn append_payload_conflicts(
    transaction: &rusqlite::Transaction<'_>,
    input: &TaskDecisionInputV1,
    targets: &[TaskDecisionBatchTargetV1],
    existing: &super::StoredSyncJobV1,
    recalculate_batches: bool,
) -> Result<(), String> {
    append_conflicts_with_reason(
        transaction,
        input,
        targets,
        existing,
        "sync_task_payload_conflict",
        recalculate_batches,
    )
}

fn append_conflicts_with_reason(
    transaction: &rusqlite::Transaction<'_>,
    input: &TaskDecisionInputV1,
    targets: &[TaskDecisionBatchTargetV1],
    existing: &super::StoredSyncJobV1,
    reason: &str,
    recalculate_batches: bool,
) -> Result<(), String> {
    for target in targets {
        append_issue(
            transaction,
            target,
            input,
            NewSyncBatchIssueKindV1::Conflict {
                reason: reason.to_string(),
                requested_payload_sha256: input.payload.payload_sha256.clone(),
                existing_payload_sha256: existing.snapshot.task_payload_sha256.clone(),
                existing_job_id: existing.snapshot.id,
            },
            recalculate_batches,
        )?;
    }
    Ok(())
}

fn append_issue(
    transaction: &rusqlite::Transaction<'_>,
    target: &TaskDecisionBatchTargetV1,
    input: &TaskDecisionInputV1,
    kind: NewSyncBatchIssueKindV1,
    recalculate_batches: bool,
) -> Result<(), String> {
    let issue = NewSyncBatchIssueV1 {
        task_key: input.task_key.clone(),
        target_kind_aggregate: false,
        phase: target.phase,
        kind,
    };
    let entry_seq = QueueStore::reserve_batch_target_tx(
        transaction,
        target.batch_id,
        &input.task_key,
        super::BatchTargetKindV1::Task,
        "issue",
    )?;
    QueueStore::append_reserved_batch_issue_tx(
        transaction,
        target.batch_id,
        &issue,
        entry_seq,
        &input.queued_at,
    )?;
    if recalculate_batches {
        QueueStore::recalculate_batch_status_tx(transaction, target.batch_id, &input.queued_at)?;
    }
    Ok(())
}

fn validate_decision_input(input: &TaskDecisionInputV1) -> Result<(), String> {
    let frozen = freeze_task_payload(input.payload.payload.clone())?;
    if frozen.payload_json != input.payload.payload_json
        || frozen.payload_sha256 != input.payload.payload_sha256
        || frozen.equivalence_json != input.payload.equivalence_json
        || frozen.equivalence_sha256 != input.payload.equivalence_sha256
        || input.payload.payload.source() != input.task_key.source
        || input.payload.payload.source_instance_key() != input.task_key.source_instance_key
    {
        return Err("invalid_sync_task_decision".to_string());
    }
    Ok(())
}

fn decision_job(input: &TaskDecisionInputV1) -> NewSyncJobV1 {
    NewSyncJobV1 {
        task_key: input.task_key.clone(),
        trigger: input.trigger,
        queued_at: input.queued_at.clone(),
        scheduled_for: input.scheduled_for.clone(),
        settings_revision: input.settings_revision,
        config_sha256: input.config_sha256.clone(),
        payload_json: input.payload.payload_json.clone(),
        payload_sha256: input.payload.payload_sha256.clone(),
        equivalence_sha256: input.payload.equivalence_sha256.clone(),
    }
}

fn ensure_active_account(connection: &Connection, account_id: i64) -> Result<(), String> {
    let active: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1 AND status = 'active')",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if active {
        Ok(())
    } else {
        Err("account_not_active".to_string())
    }
}

fn available_capability(
    capabilities: &[SourceCapabilityViewV1],
    source: SyncSourceV1,
) -> Result<&SourceCapabilityViewV1, String> {
    let capability = capabilities
        .iter()
        .find(|capability| capability.source == source)
        .ok_or_else(|| "source_unavailable".to_string())?;
    if capability.available {
        Ok(capability)
    } else {
        Err("source_unavailable".to_string())
    }
}

fn validate_payload_capability(
    payload: &SyncTaskPayloadV1,
    capability: &SourceCapabilityViewV1,
) -> Result<(), String> {
    let range = match payload {
        SyncTaskPayloadV1::History { range, .. } => match range {
            HistoryRangeV1::Days { .. } => SyncRangeKindV1::Days,
            HistoryRangeV1::Bottom => SyncRangeKindV1::Bottom,
        },
        SyncTaskPayloadV1::Likes { range, .. }
        | SyncTaskPayloadV1::Favorites { range, .. }
        | SyncTaskPayloadV1::RemoteFolder { range, .. } => archive_range_kind(*range),
    };
    if !capability.supported_ranges.contains(&range) {
        return Err(if range == SyncRangeKindV1::RecentDays {
            "time_range_not_supported".to_string()
        } else {
            "range_not_supported".to_string()
        });
    }
    if payload.run_mode() == Some(ArchiveRunModeV1::FullReconcile)
        && !capability.supports_full_reconcile
    {
        return Err("full_reconcile_not_supported".to_string());
    }
    Ok(())
}

pub(super) fn build_implicit_payload(
    account_id: i64,
    source: SyncSourceV1,
    source_instance_key: &str,
    settings: &EnqueueSettingsSnapshotV1,
    device: &EnqueueDeviceSnapshotV1,
    capabilities: &[SourceCapabilityViewV1],
    frozen_at_ms: i64,
) -> Result<SyncTaskPayloadV1, String> {
    let account = settings
        .accounts
        .get(&account_id)
        .ok_or_else(|| "account_settings_missing".to_string())?;
    let browser = device
        .browsers
        .get(&account_id)
        .cloned()
        .ok_or_else(|| "browser_identity_missing".to_string())?;
    Ok(match source {
        SyncSourceV1::History => SyncTaskPayloadV1::History {
            range: account.history.range,
            range_freeze: history_freeze(account.history.range, frozen_at_ms)?,
            scroll_speed: account.scroll_speed,
            hide_browser: account.hide_browser,
            browser,
        },
        SyncSourceV1::Likes => SyncTaskPayloadV1::Likes {
            range: account.likes.range,
            range_freeze: archive_freeze(account.likes.range, frozen_at_ms)?,
            run_mode: ArchiveRunModeV1::Incremental,
            scroll_speed: account.scroll_speed,
            hide_browser: account.hide_browser,
            browser,
        },
        SyncSourceV1::Favorites => SyncTaskPayloadV1::Favorites {
            range: account.favorites.archive.range,
            range_freeze: archive_freeze(account.favorites.archive.range, frozen_at_ms)?,
            run_mode: ArchiveRunModeV1::Incremental,
            scroll_speed: account.scroll_speed,
            hide_browser: account.hide_browser,
            browser,
            discover_remote_folders: capabilities
                .iter()
                .find(|capability| capability.source == SyncSourceV1::Favorites)
                .is_some_and(|capability| capability.supports_remote_folders),
            remote_folder_mode: account.favorites.remote_folder_mode,
            remote_folder_default_range: account.favorites.remote_folder_defaults.range,
            remote_folder_overrides: account
                .favorites
                .remote_folders
                .iter()
                .map(|(id, value)| {
                    (
                        id.clone(),
                        RemoteFolderExecutionOverrideV1 { range: value.range },
                    )
                })
                .collect(),
        },
        SyncSourceV1::RemoteFolder => {
            let range = match account.favorites.remote_folder_mode {
                RemoteFolderScheduleModeV1::Unified => {
                    account.favorites.remote_folder_defaults.range
                }
                RemoteFolderScheduleModeV1::PerFolder => account
                    .favorites
                    .remote_folders
                    .get(source_instance_key)
                    .map(|settings| settings.range)
                    .unwrap_or(account.favorites.remote_folder_defaults.range),
            };
            SyncTaskPayloadV1::RemoteFolder {
                remote_folder_id: source_instance_key.to_string(),
                range,
                range_freeze: archive_freeze(range, frozen_at_ms)?,
                run_mode: ArchiveRunModeV1::Incremental,
                scroll_speed: account.scroll_speed,
                hide_browser: account.hide_browser,
                browser,
            }
        }
    })
}

fn history_freeze(range: HistoryRangeV1, frozen_at_ms: i64) -> Result<RangeFreezeV1, String> {
    Ok(RangeFreezeV1 {
        frozen_at_ms,
        cutoff_ms: match range {
            HistoryRangeV1::Days { days } => Some(shanghai_cutoff_ms(frozen_at_ms, days)?),
            HistoryRangeV1::Bottom => None,
        },
    })
}

fn archive_freeze(range: ArchiveListRangeV1, frozen_at_ms: i64) -> Result<RangeFreezeV1, String> {
    Ok(RangeFreezeV1 {
        frozen_at_ms,
        cutoff_ms: match range {
            ArchiveListRangeV1::RecentDays { days } => {
                Some(shanghai_cutoff_ms(frozen_at_ms, days)?)
            }
            _ => None,
        },
    })
}

fn archive_range_kind(range: ArchiveListRangeV1) -> SyncRangeKindV1 {
    match range {
        ArchiveListRangeV1::All => SyncRangeKindV1::All,
        ArchiveListRangeV1::RecentItems { .. } => SyncRangeKindV1::RecentItems,
        ArchiveListRangeV1::RecordLimit { .. } => SyncRangeKindV1::RecordLimit,
        ArchiveListRangeV1::RecentDays { .. } => SyncRangeKindV1::RecentDays,
    }
}
