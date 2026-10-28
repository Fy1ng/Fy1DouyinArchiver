use super::contracts::*;
use crate::{
    device_settings::BrowserPreferenceV1,
    settings_store::{ArchiveListRangeV1, HistoryRangeV1, RemoteFolderScheduleModeV1},
};
use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use url::Url;

const SCHEMA_VERSION: u32 = 1;
const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;
const MAX_TIME_MS: i64 = 9_999_999_999_999;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MAX_ARCHIVE_RECORDS: usize = 50_000;
const MAX_REMOTE_FOLDERS: usize = 1_000;

pub fn canonical_payload(payload: &SyncTaskPayloadV1) -> Result<String, String> {
    validate_task_payload(payload)?;
    canonical_json(payload)
}

pub fn canonical_equivalence(equivalence: &SyncExecutionEquivalenceV1) -> Result<String, String> {
    validate_equivalence(equivalence)?;
    canonical_json(equivalence)
}

pub fn canonical_automatic_config(config: &AutomaticSyncConfigV1) -> Result<String, String> {
    validate_automatic_config(config)?;
    canonical_json(config)
}

pub fn freeze_task_payload(input: SyncTaskPayloadV1) -> Result<FrozenTaskPayloadV1, String> {
    let payload_json = canonical_payload(&input)?;
    let payload_sha256 = sha256(payload_json.as_bytes());
    let equivalence = build_equivalence(&input)?;
    let equivalence_json = canonical_equivalence(&equivalence)?;
    let equivalence_sha256 = sha256(equivalence_json.as_bytes());
    Ok(FrozenTaskPayloadV1 {
        payload: input,
        payload_json,
        payload_sha256,
        equivalence,
        equivalence_json,
        equivalence_sha256,
    })
}

pub fn automatic_config_hash(input: &AutomaticSyncConfigV1) -> Result<String, String> {
    Ok(sha256(canonical_automatic_config(input)?.as_bytes()))
}

pub fn job_identity_sha256(job: &SyncJobSnapshotV1) -> Result<String, String> {
    validate_job_snapshot(job)?;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct JobIdentity<'a> {
        schema_version: u32,
        job_id: i64,
        account_id: i64,
        source: SyncSourceV1,
        source_instance_key: &'a str,
        trigger: SyncTriggerV1,
        task_payload_sha256: &'a str,
    }
    let identity = JobIdentity {
        schema_version: SCHEMA_VERSION,
        job_id: job.id,
        account_id: job.task_key.account_id,
        source: job.task_key.source,
        source_instance_key: &job.task_key.source_instance_key,
        trigger: job.trigger,
        task_payload_sha256: &job.task_payload_sha256,
    };
    Ok(sha256(canonical_json(&identity)?.as_bytes()))
}

pub fn validate_enqueue_request(request: &EnqueueSyncRequestV1) -> Result<(), String> {
    match request.scope {
        SyncScopeV1::One => {
            if request.account_id.is_none_or(|value| value <= 0) || request.source.is_none() {
                return Err("invalid_enqueue_request".to_string());
            }
            let source = request.source.expect("checked");
            validate_optional_source_instance(source, request.source_instance_key.as_deref())?;
            if let Some(payload) = &request.payload {
                if payload.source() != source
                    || payload.source_instance_key()
                        != request.source_instance_key.as_deref().unwrap_or_default()
                {
                    return Err("sync_payload_source_mismatch".to_string());
                }
                let frozen = freeze_task_payload(payload.clone())?;
                match payload.run_mode() {
                    Some(ArchiveRunModeV1::FullReconcile) => {
                        if payload.archive_range() != Some(ArchiveListRangeV1::All) {
                            return Err("full_reconcile_requires_all".to_string());
                        }
                        if request.full_reconcile_confirmation_sha256.as_deref()
                            != Some(frozen.payload_sha256.as_str())
                        {
                            return Err("full_reconcile_confirmation_mismatch".to_string());
                        }
                    }
                    _ if request.full_reconcile_confirmation_sha256.is_some() => {
                        return Err("unexpected_full_reconcile_confirmation".to_string())
                    }
                    _ => {}
                }
            } else if request.full_reconcile_confirmation_sha256.is_some() {
                return Err("unexpected_full_reconcile_confirmation".to_string());
            }
        }
        SyncScopeV1::CurrentAccountAvailableSources => {
            if request.account_id.is_none_or(|value| value <= 0)
                || request.source.is_some()
                || request.source_instance_key.is_some()
                || request.payload.is_some()
                || request.full_reconcile_confirmation_sha256.is_some()
            {
                return Err("invalid_enqueue_request".to_string());
            }
        }
        SyncScopeV1::AllActiveAccounts => {
            if request.account_id.is_some()
                || request.source.is_some()
                || request.source_instance_key.is_some()
                || request.payload.is_some()
                || request.full_reconcile_confirmation_sha256.is_some()
            {
                return Err("invalid_enqueue_request".to_string());
            }
        }
    }
    Ok(())
}

pub fn validate_execution_envelope(
    envelope: &SyncExecutionEnvelopeV1,
    job: &SyncJobSnapshotV1,
) -> Result<(), String> {
    validate_job_snapshot(job)?;
    if envelope.schema_version != 1
        || envelope.account_id != job.task_key.account_id
        || envelope.job_id != job.id
        || envelope.source != job.task_key.source
        || envelope.source_instance_key != job.task_key.source_instance_key
        || envelope.task_payload_sha256 != job.task_payload_sha256
        || envelope.result.source() != envelope.source
        || job.status != SyncJobStatusV1::Running
        || !valid_sha256(&envelope.task_payload_sha256)
    {
        return Err("sync_result_identity_mismatch".to_string());
    }
    validate_source_instance(envelope.source, &envelope.source_instance_key)
        .map_err(|_| "sync_result_identity_mismatch".to_string())?;
    match &envelope.result {
        SourceExecutionResultV1::History(result) => {
            if result.account_id != envelope.account_id || result.sync_job_id != envelope.job_id {
                return Err("sync_result_identity_mismatch".to_string());
            }
            crate::validate_collector_result(result, envelope.job_id)
        }
        SourceExecutionResultV1::Likes(result) => validate_archive_collection(result),
        SourceExecutionResultV1::Favorites(result) => {
            validate_archive_collection(&result.archive)?;
            if let RemoteFolderDiscoveryResultV1::Success(discovery) =
                &result.remote_folder_discovery
            {
                validate_remote_folder_discovery(discovery)?;
            }
            Ok(())
        }
        SourceExecutionResultV1::RemoteFolder(result) => {
            if result.remote_folder_id != envelope.source_instance_key {
                return Err("sync_result_identity_mismatch".to_string());
            }
            validate_remote_folder_id(&result.remote_folder_id)?;
            validate_nonempty_text(&result.name, 256, "invalid_remote_folder_name")?;
            validate_archive_collection(&result.archive)
        }
    }
}

pub fn compute_archive_hashes(records: &[ArchiveItemV1]) -> Result<(String, String), String> {
    validate_archive_records(records)?;
    let identity = records
        .iter()
        .map(|record| (record.aweme_id.as_str(), record.source_position))
        .collect::<Vec<_>>();
    let fields = records
        .iter()
        .map(|record| {
            (
                record.aweme_id.as_str(),
                record.canonical_url.as_str(),
                record.content_type.as_str(),
                record.source_position,
                record.remote_created_at_ms,
                record.title.as_deref(),
                record.author_name.as_deref(),
                record.cover_url.as_deref(),
                record.like_count,
            )
        })
        .collect::<Vec<_>>();
    Ok((
        sha256(canonical_json(&identity)?.as_bytes()),
        sha256(canonical_json(&fields)?.as_bytes()),
    ))
}

pub fn validate_archive_collection(result: &ArchiveCollectionSuccessV1) -> Result<(), String> {
    let (identity_hash, field_hash) = compute_archive_hashes(&result.records)?;
    if !valid_sha256(&result.identity_hash)
        || !valid_sha256(&result.field_hash)
        || result.identity_hash != identity_hash
        || result.field_hash != field_hash
    {
        return Err("archive_hash_mismatch".to_string());
    }
    let record_count = result.records.len();
    match &result.completion {
        ArchiveCompletionV1::Complete { stop } => match stop {
            ArchiveCompleteStopV1::Bottom => {}
            ArchiveCompleteStopV1::RequestedItemCount { count }
                if (1..=5_000).contains(count) && record_count == *count as usize => {}
            ArchiveCompleteStopV1::RecordLimit { limit }
                if (1..=50_000).contains(limit) && record_count == *limit as usize => {}
            ArchiveCompleteStopV1::DateBoundary {
                cutoff_ms,
                guard_aweme_id,
                guard_remote_created_at_ms,
            } if *cutoff_ms >= 0
                && valid_aweme_id(guard_aweme_id)
                && *guard_remote_created_at_ms >= 0
                && *guard_remote_created_at_ms < *cutoff_ms
                && result
                    .records
                    .iter()
                    .all(|record| record.aweme_id != *guard_aweme_id) => {}
            ArchiveCompleteStopV1::StableKnownBoundary { consecutive_known }
                if *consecutive_known >= 20 && record_count < MAX_ARCHIVE_RECORDS => {}
            _ => return Err("invalid_archive_completion".to_string()),
        },
        ArchiveCompletionV1::Incomplete { reason } => match reason {
            ArchiveIncompleteReasonV1::SafetyRecordCap { cap }
                if *cap == 50_000 && record_count == MAX_ARCHIVE_RECORDS => {}
            ArchiveIncompleteReasonV1::UnconfirmedEnd if record_count < MAX_ARCHIVE_RECORDS => {}
            _ => return Err("invalid_archive_completion".to_string()),
        },
    }
    Ok(())
}

pub fn compute_remote_folder_hashes(
    folders: &[RemoteFolderDescriptorV1],
) -> Result<(String, String), String> {
    validate_remote_folder_records(folders)?;
    let identity = folders
        .iter()
        .map(|folder| (folder.remote_folder_id.as_str(), folder.source_position))
        .collect::<Vec<_>>();
    let fields = folders
        .iter()
        .map(|folder| {
            (
                folder.remote_folder_id.as_str(),
                folder.name.as_str(),
                folder.source_position,
            )
        })
        .collect::<Vec<_>>();
    Ok((
        sha256(canonical_json(&identity)?.as_bytes()),
        sha256(canonical_json(&fields)?.as_bytes()),
    ))
}

pub fn validate_remote_folder_discovery(
    result: &RemoteFolderDiscoverySuccessV1,
) -> Result<(), String> {
    let (identity_hash, field_hash) = compute_remote_folder_hashes(&result.folders)?;
    if !valid_sha256(&result.identity_hash)
        || !valid_sha256(&result.field_hash)
        || result.identity_hash != identity_hash
        || result.field_hash != field_hash
    {
        return Err("remote_folder_hash_mismatch".to_string());
    }
    let valid_state = match (
        result.complete_scan,
        result.bottom_confirmed,
        result.hit_record_limit,
    ) {
        (true, true, false) => result.folders.len() <= MAX_REMOTE_FOLDERS,
        (false, false, true) => result.folders.len() == MAX_REMOTE_FOLDERS,
        (false, false, false) => result.folders.len() < MAX_REMOTE_FOLDERS,
        _ => false,
    };
    if !valid_state {
        return Err("invalid_remote_folder_discovery_state".to_string());
    }
    Ok(())
}

fn build_equivalence(payload: &SyncTaskPayloadV1) -> Result<SyncExecutionEquivalenceV1, String> {
    match payload {
        SyncTaskPayloadV1::History {
            range,
            range_freeze,
            scroll_speed,
            hide_browser,
            browser,
        } => Ok(SyncExecutionEquivalenceV1::History {
            schema_version: SCHEMA_VERSION,
            source_instance_key: String::new(),
            range: effective_history_range(*range, range_freeze)?,
            scroll_speed: *scroll_speed,
            hide_browser: *hide_browser,
            browser: browser.clone(),
        }),
        SyncTaskPayloadV1::Likes {
            range,
            range_freeze,
            run_mode,
            scroll_speed,
            hide_browser,
            browser,
        } => Ok(SyncExecutionEquivalenceV1::Likes {
            schema_version: SCHEMA_VERSION,
            source_instance_key: String::new(),
            range: effective_archive_range(*range, range_freeze.frozen_at_ms)?,
            run_mode: *run_mode,
            scroll_speed: *scroll_speed,
            hide_browser: *hide_browser,
            browser: browser.clone(),
        }),
        SyncTaskPayloadV1::Favorites {
            range,
            range_freeze,
            run_mode,
            scroll_speed,
            hide_browser,
            browser,
            discover_remote_folders,
            remote_folder_mode,
            remote_folder_default_range,
            remote_folder_overrides,
        } => {
            let overrides = remote_folder_overrides
                .iter()
                .map(|(remote_folder_id, value)| {
                    Ok(RemoteFolderRangeEquivalenceV1 {
                        remote_folder_id: remote_folder_id.clone(),
                        range: effective_archive_range(value.range, range_freeze.frozen_at_ms)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(SyncExecutionEquivalenceV1::Favorites {
                schema_version: SCHEMA_VERSION,
                source_instance_key: String::new(),
                range: effective_archive_range(*range, range_freeze.frozen_at_ms)?,
                run_mode: *run_mode,
                scroll_speed: *scroll_speed,
                hide_browser: *hide_browser,
                browser: browser.clone(),
                discover_remote_folders: *discover_remote_folders,
                remote_folder_mode: *remote_folder_mode,
                remote_folder_default_range: effective_archive_range(
                    *remote_folder_default_range,
                    range_freeze.frozen_at_ms,
                )?,
                remote_folder_overrides: overrides,
            })
        }
        SyncTaskPayloadV1::RemoteFolder {
            remote_folder_id,
            range,
            range_freeze,
            run_mode,
            scroll_speed,
            hide_browser,
            browser,
        } => Ok(SyncExecutionEquivalenceV1::RemoteFolder {
            schema_version: SCHEMA_VERSION,
            source_instance_key: remote_folder_id.clone(),
            range: effective_archive_range(*range, range_freeze.frozen_at_ms)?,
            run_mode: *run_mode,
            scroll_speed: *scroll_speed,
            hide_browser: *hide_browser,
            browser: browser.clone(),
        }),
    }
}

fn validate_task_payload(payload: &SyncTaskPayloadV1) -> Result<(), String> {
    match payload {
        SyncTaskPayloadV1::History {
            range,
            range_freeze,
            scroll_speed,
            browser,
            ..
        } => {
            validate_scroll_speed(*scroll_speed)?;
            validate_browser_identity(browser)?;
            validate_history_range(*range)?;
            validate_range_freeze(range_freeze)?;
            effective_history_range(*range, range_freeze)?;
        }
        SyncTaskPayloadV1::Likes {
            range,
            range_freeze,
            scroll_speed,
            browser,
            ..
        } => {
            validate_archive_range(*range)?;
            validate_range_freeze(range_freeze)?;
            validate_archive_freeze(*range, range_freeze)?;
            validate_scroll_speed(*scroll_speed)?;
            validate_browser_identity(browser)?;
        }
        SyncTaskPayloadV1::Favorites {
            range,
            range_freeze,
            scroll_speed,
            browser,
            remote_folder_mode,
            remote_folder_default_range,
            remote_folder_overrides,
            ..
        } => {
            validate_archive_range(*range)?;
            validate_range_freeze(range_freeze)?;
            validate_archive_freeze(*range, range_freeze)?;
            validate_scroll_speed(*scroll_speed)?;
            validate_browser_identity(browser)?;
            validate_archive_range(*remote_folder_default_range)?;
            if remote_folder_overrides.len() > 1_000
                || (*remote_folder_mode == RemoteFolderScheduleModeV1::Unified
                    && !remote_folder_overrides.is_empty())
            {
                return Err("invalid_remote_folder_overrides".to_string());
            }
            for (remote_folder_id, value) in remote_folder_overrides {
                validate_remote_folder_id(remote_folder_id)?;
                validate_archive_range(value.range)?;
            }
        }
        SyncTaskPayloadV1::RemoteFolder {
            remote_folder_id,
            range,
            range_freeze,
            scroll_speed,
            browser,
            ..
        } => {
            validate_remote_folder_id(remote_folder_id)?;
            validate_archive_range(*range)?;
            validate_range_freeze(range_freeze)?;
            validate_archive_freeze(*range, range_freeze)?;
            validate_scroll_speed(*scroll_speed)?;
            validate_browser_identity(browser)?;
        }
    }
    Ok(())
}

fn validate_equivalence(value: &SyncExecutionEquivalenceV1) -> Result<(), String> {
    match value {
        SyncExecutionEquivalenceV1::History {
            schema_version,
            source_instance_key,
            range,
            scroll_speed,
            browser,
            ..
        } => {
            validate_common_equivalence(
                *schema_version,
                SyncSourceV1::History,
                source_instance_key,
                range,
                *scroll_speed,
                browser,
            )?;
        }
        SyncExecutionEquivalenceV1::Likes {
            schema_version,
            source_instance_key,
            range,
            scroll_speed,
            browser,
            ..
        } => {
            validate_common_equivalence(
                *schema_version,
                SyncSourceV1::Likes,
                source_instance_key,
                range,
                *scroll_speed,
                browser,
            )?;
        }
        SyncExecutionEquivalenceV1::Favorites {
            schema_version,
            source_instance_key,
            range,
            scroll_speed,
            browser,
            remote_folder_mode,
            remote_folder_default_range,
            remote_folder_overrides,
            ..
        } => {
            validate_common_equivalence(
                *schema_version,
                SyncSourceV1::Favorites,
                source_instance_key,
                range,
                *scroll_speed,
                browser,
            )?;
            validate_effective_range(remote_folder_default_range)?;
            if remote_folder_overrides.len() > 1_000
                || (*remote_folder_mode == RemoteFolderScheduleModeV1::Unified
                    && !remote_folder_overrides.is_empty())
            {
                return Err("invalid_remote_folder_overrides".to_string());
            }
            let mut previous: Option<&str> = None;
            for value in remote_folder_overrides {
                validate_remote_folder_id(&value.remote_folder_id)?;
                validate_effective_range(&value.range)?;
                if previous.is_some_and(|previous| {
                    previous.as_bytes() >= value.remote_folder_id.as_bytes()
                }) {
                    return Err("invalid_remote_folder_overrides".to_string());
                }
                previous = Some(&value.remote_folder_id);
            }
        }
        SyncExecutionEquivalenceV1::RemoteFolder {
            schema_version,
            source_instance_key,
            range,
            scroll_speed,
            browser,
            ..
        } => {
            validate_common_equivalence(
                *schema_version,
                SyncSourceV1::RemoteFolder,
                source_instance_key,
                range,
                *scroll_speed,
                browser,
            )?;
        }
    }
    Ok(())
}

fn validate_common_equivalence(
    schema_version: u32,
    source: SyncSourceV1,
    source_instance_key: &str,
    range: &EffectiveRangeEquivalenceV1,
    scroll_speed: u32,
    browser: &BrowserExecutionIdentityV1,
) -> Result<(), String> {
    if schema_version != SCHEMA_VERSION {
        return Err("invalid_equivalence_schema".to_string());
    }
    validate_source_instance(source, source_instance_key)?;
    validate_effective_range(range)?;
    validate_scroll_speed(scroll_speed)?;
    validate_browser_identity(browser)
}

fn validate_automatic_config(config: &AutomaticSyncConfigV1) -> Result<(), String> {
    let result = match config {
        AutomaticSyncConfigV1::History {
            schema_version,
            source_instance_key,
            schedule,
            range,
            scroll_speed,
            ..
        } => {
            validate_automatic_common(
                *schema_version,
                SyncSourceV1::History,
                source_instance_key,
                schedule,
                *scroll_speed,
            )?;
            validate_history_range(*range)
        }
        AutomaticSyncConfigV1::Likes {
            schema_version,
            source_instance_key,
            schedule,
            range,
            run_mode,
            scroll_speed,
            ..
        } => validate_automatic_archive(
            *schema_version,
            SyncSourceV1::Likes,
            source_instance_key,
            schedule,
            *range,
            *run_mode,
            *scroll_speed,
        ),
        AutomaticSyncConfigV1::Favorites {
            schema_version,
            source_instance_key,
            schedule,
            range,
            run_mode,
            scroll_speed,
            ..
        } => validate_automatic_archive(
            *schema_version,
            SyncSourceV1::Favorites,
            source_instance_key,
            schedule,
            *range,
            *run_mode,
            *scroll_speed,
        ),
        AutomaticSyncConfigV1::RemoteFolder {
            schema_version,
            source_instance_key,
            schedule,
            range,
            run_mode,
            scroll_speed,
            ..
        } => validate_automatic_archive(
            *schema_version,
            SyncSourceV1::RemoteFolder,
            source_instance_key,
            schedule,
            *range,
            *run_mode,
            *scroll_speed,
        ),
    };
    result.map_err(|_| "invalid_automatic_config".to_string())
}

fn validate_automatic_archive(
    schema_version: u32,
    source: SyncSourceV1,
    source_instance_key: &str,
    schedule: &AutomaticScheduleConfigV1,
    range: ArchiveListRangeV1,
    run_mode: ArchiveRunModeV1,
    scroll_speed: u32,
) -> Result<(), String> {
    validate_automatic_common(
        schema_version,
        source,
        source_instance_key,
        schedule,
        scroll_speed,
    )?;
    validate_archive_range(range)?;
    if run_mode != ArchiveRunModeV1::Incremental {
        return Err("invalid_automatic_config".to_string());
    }
    Ok(())
}

fn validate_automatic_common(
    schema_version: u32,
    source: SyncSourceV1,
    source_instance_key: &str,
    schedule: &AutomaticScheduleConfigV1,
    scroll_speed: u32,
) -> Result<(), String> {
    if schema_version != SCHEMA_VERSION || !(15..=43_200).contains(&schedule.interval_minutes) {
        return Err("invalid_automatic_config".to_string());
    }
    validate_source_instance(source, source_instance_key)?;
    validate_scroll_speed(scroll_speed)
}

fn validate_job_snapshot(job: &SyncJobSnapshotV1) -> Result<(), String> {
    if job.id <= 0 || !valid_sha256(&job.task_payload_sha256) {
        return Err("invalid_sync_job_snapshot".to_string());
    }
    validate_task_key(&job.task_key)
}

pub fn validate_task_key(task_key: &TaskKeyV1) -> Result<(), String> {
    if task_key.account_id <= 0 {
        return Err("invalid_task_key".to_string());
    }
    validate_source_instance(task_key.source, &task_key.source_instance_key)
        .map_err(|_| "invalid_task_key".to_string())
}

pub fn validate_source_instance(source: SyncSourceV1, value: &str) -> Result<(), String> {
    match source {
        SyncSourceV1::RemoteFolder => validate_remote_folder_id(value),
        SyncSourceV1::History | SyncSourceV1::Likes | SyncSourceV1::Favorites
            if value.is_empty() =>
        {
            Ok(())
        }
        _ => Err("invalid_source_instance".to_string()),
    }
}

fn validate_optional_source_instance(
    source: SyncSourceV1,
    value: Option<&str>,
) -> Result<(), String> {
    match source {
        SyncSourceV1::RemoteFolder => {
            validate_remote_folder_id(value.ok_or_else(|| "invalid_source_instance".to_string())?)
        }
        _ if value.is_none_or(str::is_empty) => Ok(()),
        _ => Err("invalid_source_instance".to_string()),
    }
}

fn validate_browser_identity(value: &BrowserExecutionIdentityV1) -> Result<(), String> {
    let preference_matches = match value.preference {
        BrowserPreferenceV1::Auto => true,
        BrowserPreferenceV1::Edge => value.resolved_browser == BrowserIdV1::Edge,
        BrowserPreferenceV1::Chrome => value.resolved_browser == BrowserIdV1::Chrome,
    };
    if !preference_matches
        || !valid_simple_uuid(&value.profile_key)
        || !valid_simple_uuid(&value.profile_generation)
        || !valid_sha256(&value.executable_path_sha256)
    {
        return Err("invalid_browser_identity".to_string());
    }
    Ok(())
}

fn validate_effective_range(value: &EffectiveRangeEquivalenceV1) -> Result<(), String> {
    let valid = match value.kind {
        SyncRangeKindV1::Days => {
            value.days.is_some_and(|days| (1..=365).contains(&days))
                && value.count.is_none()
                && value.limit.is_none()
                && value.cutoff_ms.is_some()
        }
        SyncRangeKindV1::Bottom | SyncRangeKindV1::All => {
            value.days.is_none()
                && value.count.is_none()
                && value.limit.is_none()
                && value.cutoff_ms.is_none()
        }
        SyncRangeKindV1::RecentItems => {
            value.days.is_none()
                && value
                    .count
                    .is_some_and(|count| (1..=5_000).contains(&count))
                && value.limit.is_none()
                && value.cutoff_ms.is_none()
        }
        SyncRangeKindV1::RecordLimit => {
            value.days.is_none()
                && value.count.is_none()
                && value
                    .limit
                    .is_some_and(|limit| (1..=50_000).contains(&limit))
                && value.cutoff_ms.is_none()
        }
        SyncRangeKindV1::RecentDays => {
            value.days.is_some_and(|days| (1..=365).contains(&days))
                && value.count.is_none()
                && value.limit.is_none()
                && value.cutoff_ms.is_some()
        }
    };
    if !valid {
        return Err("invalid_effective_range".to_string());
    }
    Ok(())
}

fn effective_history_range(
    range: HistoryRangeV1,
    freeze: &RangeFreezeV1,
) -> Result<EffectiveRangeEquivalenceV1, String> {
    match range {
        HistoryRangeV1::Days { days } => {
            let expected = shanghai_cutoff_ms(freeze.frozen_at_ms, days)?;
            if freeze.cutoff_ms != Some(expected) {
                return Err("range_cutoff_mismatch".to_string());
            }
            Ok(EffectiveRangeEquivalenceV1 {
                kind: SyncRangeKindV1::Days,
                days: Some(days),
                count: None,
                limit: None,
                cutoff_ms: Some(expected),
            })
        }
        HistoryRangeV1::Bottom if freeze.cutoff_ms.is_none() => Ok(EffectiveRangeEquivalenceV1 {
            kind: SyncRangeKindV1::Bottom,
            days: None,
            count: None,
            limit: None,
            cutoff_ms: None,
        }),
        HistoryRangeV1::Bottom => Err("range_cutoff_mismatch".to_string()),
    }
}

fn effective_archive_range(
    range: ArchiveListRangeV1,
    frozen_at_ms: i64,
) -> Result<EffectiveRangeEquivalenceV1, String> {
    validate_archive_range(range)?;
    Ok(match range {
        ArchiveListRangeV1::All => EffectiveRangeEquivalenceV1::all(),
        ArchiveListRangeV1::RecentItems { count } => EffectiveRangeEquivalenceV1 {
            kind: SyncRangeKindV1::RecentItems,
            days: None,
            count: Some(count),
            limit: None,
            cutoff_ms: None,
        },
        ArchiveListRangeV1::RecordLimit { limit } => EffectiveRangeEquivalenceV1 {
            kind: SyncRangeKindV1::RecordLimit,
            days: None,
            count: None,
            limit: Some(limit),
            cutoff_ms: None,
        },
        ArchiveListRangeV1::RecentDays { days } => EffectiveRangeEquivalenceV1 {
            kind: SyncRangeKindV1::RecentDays,
            days: Some(days),
            count: None,
            limit: None,
            cutoff_ms: Some(shanghai_cutoff_ms(frozen_at_ms, days)?),
        },
    })
}

fn validate_archive_freeze(
    range: ArchiveListRangeV1,
    freeze: &RangeFreezeV1,
) -> Result<(), String> {
    match range {
        ArchiveListRangeV1::RecentDays { days } => {
            if freeze.cutoff_ms != Some(shanghai_cutoff_ms(freeze.frozen_at_ms, days)?) {
                return Err("range_cutoff_mismatch".to_string());
            }
        }
        _ if freeze.cutoff_ms.is_some() => return Err("range_cutoff_mismatch".to_string()),
        _ => {}
    }
    Ok(())
}

pub fn shanghai_cutoff_ms(frozen_at_ms: i64, days: u32) -> Result<i64, String> {
    if !(1..=365).contains(&days) {
        return Err("invalid_range_days".to_string());
    }
    let offset = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS)
        .ok_or_else(|| "range_cutoff_mismatch".to_string())?;
    let frozen = DateTime::<Utc>::from_timestamp_millis(frozen_at_ms)
        .ok_or_else(|| "invalid_range_freeze".to_string())?;
    let date = frozen.with_timezone(&offset).date_naive();
    let midnight = offset
        .from_local_datetime(
            &date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| "range_cutoff_mismatch".to_string())?,
        )
        .single()
        .ok_or_else(|| "range_cutoff_mismatch".to_string())?
        .timestamp_millis();
    midnight
        .checked_sub(i64::from(days - 1) * 86_400_000)
        .ok_or_else(|| "range_cutoff_mismatch".to_string())
}

fn validate_range_freeze(value: &RangeFreezeV1) -> Result<(), String> {
    if value.frozen_at_ms < 0
        || value.frozen_at_ms > MAX_SAFE_INTEGER
        || DateTime::<Utc>::from_timestamp_millis(value.frozen_at_ms).is_none()
    {
        return Err("invalid_range_freeze".to_string());
    }
    Ok(())
}

fn validate_history_range(value: HistoryRangeV1) -> Result<(), String> {
    match value {
        HistoryRangeV1::Days { days } if !(1..=365).contains(&days) => {
            Err("invalid_history_range".to_string())
        }
        _ => Ok(()),
    }
}

fn validate_archive_range(value: ArchiveListRangeV1) -> Result<(), String> {
    match value {
        ArchiveListRangeV1::All => Ok(()),
        ArchiveListRangeV1::RecentItems { count } if (1..=5_000).contains(&count) => Ok(()),
        ArchiveListRangeV1::RecordLimit { limit } if (1..=50_000).contains(&limit) => Ok(()),
        ArchiveListRangeV1::RecentDays { days } if (1..=365).contains(&days) => Ok(()),
        _ => Err("invalid_archive_list_range".to_string()),
    }
}

fn validate_scroll_speed(value: u32) -> Result<(), String> {
    if value < 120 {
        return Err("invalid_scroll_speed".to_string());
    }
    Ok(())
}

fn validate_archive_records(records: &[ArchiveItemV1]) -> Result<(), String> {
    if records.len() > MAX_ARCHIVE_RECORDS {
        return Err("archive_record_limit_exceeded".to_string());
    }
    let mut ids = HashSet::new();
    for (index, record) in records.iter().enumerate() {
        if record.source_position != index as i64 {
            return Err("invalid_source_position".to_string());
        }
        if !valid_aweme_id(&record.aweme_id) || !ids.insert(record.aweme_id.as_str()) {
            return Err("invalid_archive_aweme_id".to_string());
        }
        let expected_url = match record.content_type.as_str() {
            "video" => format!("https://www.douyin.com/video/{}", record.aweme_id),
            "note" => format!("https://www.douyin.com/note/{}", record.aweme_id),
            "unknown" => format!("https://www.douyin.com/article/{}", record.aweme_id),
            _ => return Err("invalid_archive_content_type".to_string()),
        };
        if record.canonical_url != expected_url {
            return Err("invalid_archive_url".to_string());
        }
        if record
            .remote_created_at_ms
            .is_some_and(|value| !(0..=MAX_TIME_MS).contains(&value))
            || record.like_count.is_some_and(|value| value < 0)
        {
            return Err("invalid_archive_numeric_field".to_string());
        }
        validate_optional_text(record.title.as_deref(), 2_048, "invalid_archive_title")?;
        validate_optional_text(record.author_name.as_deref(), 256, "invalid_archive_author")?;
        if let Some(cover_url) = record.cover_url.as_deref() {
            if cover_url.len() > 4_096
                || Url::parse(cover_url)
                    .ok()
                    .is_none_or(|url| url.scheme() != "https")
            {
                return Err("invalid_archive_cover_url".to_string());
            }
        }
    }
    Ok(())
}

fn validate_remote_folder_records(folders: &[RemoteFolderDescriptorV1]) -> Result<(), String> {
    if folders.len() > MAX_REMOTE_FOLDERS {
        return Err("remote_folder_discovery_limit_exceeded".to_string());
    }
    let mut ids = HashSet::new();
    for (index, folder) in folders.iter().enumerate() {
        if folder.source_position != index as i64 {
            return Err("invalid_source_position".to_string());
        }
        validate_remote_folder_id(&folder.remote_folder_id)?;
        if !ids.insert(folder.remote_folder_id.as_str()) {
            return Err("duplicate_remote_folder_id".to_string());
        }
        validate_nonempty_text(&folder.name, 256, "invalid_remote_folder_name")?;
    }
    Ok(())
}

fn validate_remote_folder_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err("invalid_remote_folder_id".to_string());
    }
    Ok(())
}

fn validate_nonempty_text(value: &str, max: usize, error: &str) -> Result<(), String> {
    if value.is_empty() || value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(error.to_string());
    }
    Ok(())
}

fn validate_optional_text(value: Option<&str>, max: usize, error: &str) -> Result<(), String> {
    if value.is_some_and(|value| value.chars().count() > max || value.chars().any(char::is_control))
    {
        return Err(error.to_string());
    }
    Ok(())
}

fn valid_aweme_id(value: &str) -> bool {
    (1..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_simple_uuid(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
