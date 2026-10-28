use crate::{
    settings_store::ArchiveListRangeV1,
    sync::{
        compute_archive_hashes, compute_remote_folder_hashes, ArchiveCollectionSuccessV1,
        ArchiveCompleteStopV1, ArchiveCompletionV1, ArchiveIncompleteReasonV1, ArchiveItemV1,
        ArchiveRunModeV1, RemoteFolderDescriptorV1, RemoteFolderDiscoverySuccessV1,
    },
};
use std::collections::HashSet;

const ARCHIVE_RECORD_CAP: usize = 50_000;
const REMOTE_FOLDER_LIMIT: usize = 1_000;
const STABLE_KNOWN_BOUNDARY: u32 = 20;

#[derive(Debug, Clone)]
pub struct ArchiveStopInput {
    pub range: ArchiveListRangeV1,
    pub run_mode: ArchiveRunModeV1,
    pub cutoff_ms: Option<i64>,
    pub bottom_confirmed: bool,
    pub terminal_without_progress: bool,
    pub consecutive_known: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveStopDecision {
    Continue,
    Finished(ArchiveCollectionSuccessV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteFolderDiscoveryDecision {
    Continue,
    Finished(RemoteFolderDiscoverySuccessV1),
}

pub fn validate_archive_growth(
    previous: &[ArchiveItemV1],
    current: &[ArchiveItemV1],
) -> Result<(), String> {
    if current.len() < previous.len() {
        return Err("archive_snapshot_prefix_changed".to_string());
    }
    let mut ids = HashSet::with_capacity(current.len());
    for (index, record) in current.iter().enumerate() {
        if record.source_position != index as i64 || !ids.insert(record.aweme_id.as_str()) {
            return Err("invalid_source_position".to_string());
        }
        if let Some(expected) = previous.get(index) {
            if record.aweme_id != expected.aweme_id
                || record.canonical_url != expected.canonical_url
                || record.content_type != expected.content_type
            {
                return Err("archive_snapshot_prefix_changed".to_string());
            }
        }
    }
    Ok(())
}

pub fn consecutive_known_tail(records: &[ArchiveItemV1], known_ids: &HashSet<String>) -> u32 {
    records
        .iter()
        .rev()
        .take_while(|record| known_ids.contains(&record.aweme_id))
        .count()
        .min(u32::MAX as usize) as u32
}

pub fn archive_snapshot_made_progress(
    previous: &[ArchiveItemV1],
    current: &[ArchiveItemV1],
) -> bool {
    current.len() > previous.len()
        || current
            .iter()
            .zip(previous)
            .any(|(current, previous)| current != previous)
}

pub fn decide_archive_stop(
    records: Vec<ArchiveItemV1>,
    input: ArchiveStopInput,
) -> Result<ArchiveStopDecision, String> {
    validate_positions(&records)?;
    let (stored_records, completion) = match input.range {
        ArchiveListRangeV1::RecentItems { count } if records.len() >= count as usize => (
            records.into_iter().take(count as usize).collect(),
            Some(ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::RequestedItemCount { count },
            }),
        ),
        ArchiveListRangeV1::RecordLimit { limit } if records.len() >= limit as usize => (
            records.into_iter().take(limit as usize).collect(),
            Some(ArchiveCompletionV1::Complete {
                stop: ArchiveCompleteStopV1::RecordLimit { limit },
            }),
        ),
        ArchiveListRangeV1::RecentDays { .. } => {
            let cutoff_ms = input
                .cutoff_ms
                .ok_or_else(|| "archive_cutoff_missing".to_string())?;
            let guard = first_date_guard(&records, cutoff_ms)?;
            let within_range = records
                .iter()
                .take(guard.map_or(records.len(), |(index, _)| index))
                .take(ARCHIVE_RECORD_CAP)
                .cloned()
                .collect::<Vec<_>>();
            let completion = if let Some((_, guard)) = guard {
                Some(ArchiveCompletionV1::Complete {
                    stop: ArchiveCompleteStopV1::DateBoundary {
                        cutoff_ms,
                        guard_aweme_id: guard.aweme_id.clone(),
                        guard_remote_created_at_ms: guard.remote_created_at_ms.unwrap(),
                    },
                })
            } else if input.bottom_confirmed && records.len() <= ARCHIVE_RECORD_CAP {
                Some(ArchiveCompletionV1::Complete {
                    stop: ArchiveCompleteStopV1::Bottom,
                })
            } else if within_range.len() == ARCHIVE_RECORD_CAP {
                Some(ArchiveCompletionV1::Incomplete {
                    reason: ArchiveIncompleteReasonV1::SafetyRecordCap {
                        cap: ARCHIVE_RECORD_CAP as u32,
                    },
                })
            } else if input.terminal_without_progress {
                Some(ArchiveCompletionV1::Incomplete {
                    reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
                })
            } else {
                None
            };
            (within_range, completion)
        }
        range => {
            let completion = if input.bottom_confirmed && records.len() <= ARCHIVE_RECORD_CAP {
                Some(ArchiveCompletionV1::Complete {
                    stop: ArchiveCompleteStopV1::Bottom,
                })
            } else if matches!(range, ArchiveListRangeV1::All)
                && input.run_mode == ArchiveRunModeV1::Incremental
                && input.consecutive_known >= STABLE_KNOWN_BOUNDARY
                && records.len() < ARCHIVE_RECORD_CAP
            {
                Some(ArchiveCompletionV1::Complete {
                    stop: ArchiveCompleteStopV1::StableKnownBoundary {
                        consecutive_known: input.consecutive_known,
                    },
                })
            } else if records.len() >= ARCHIVE_RECORD_CAP
                && matches!(range, ArchiveListRangeV1::All)
            {
                Some(ArchiveCompletionV1::Incomplete {
                    reason: ArchiveIncompleteReasonV1::SafetyRecordCap {
                        cap: ARCHIVE_RECORD_CAP as u32,
                    },
                })
            } else if input.terminal_without_progress {
                Some(ArchiveCompletionV1::Incomplete {
                    reason: ArchiveIncompleteReasonV1::UnconfirmedEnd,
                })
            } else {
                None
            };
            (
                records.into_iter().take(ARCHIVE_RECORD_CAP).collect(),
                completion,
            )
        }
    };

    let Some(completion) = completion else {
        return Ok(ArchiveStopDecision::Continue);
    };
    Ok(ArchiveStopDecision::Finished(finish_archive(
        stored_records,
        completion,
    )?))
}

pub fn decide_remote_folder_discovery(
    folders: Vec<RemoteFolderDescriptorV1>,
    bottom_confirmed: bool,
    terminal_without_progress: bool,
) -> Result<RemoteFolderDiscoveryDecision, String> {
    let completion = if bottom_confirmed && folders.len() <= REMOTE_FOLDER_LIMIT {
        Some((true, true, false))
    } else if folders.len() >= REMOTE_FOLDER_LIMIT {
        Some((false, false, true))
    } else if terminal_without_progress {
        Some((false, false, false))
    } else {
        None
    };
    let Some((complete_scan, bottom_confirmed, hit_record_limit)) = completion else {
        return Ok(RemoteFolderDiscoveryDecision::Continue);
    };
    let folders = folders
        .into_iter()
        .take(REMOTE_FOLDER_LIMIT)
        .collect::<Vec<_>>();
    let (identity_hash, field_hash) = compute_remote_folder_hashes(&folders)?;
    Ok(RemoteFolderDiscoveryDecision::Finished(
        RemoteFolderDiscoverySuccessV1 {
            folders,
            identity_hash,
            field_hash,
            complete_scan,
            bottom_confirmed,
            hit_record_limit,
        },
    ))
}

fn finish_archive(
    records: Vec<ArchiveItemV1>,
    completion: ArchiveCompletionV1,
) -> Result<ArchiveCollectionSuccessV1, String> {
    let (identity_hash, field_hash) = compute_archive_hashes(&records)?;
    Ok(ArchiveCollectionSuccessV1 {
        records,
        identity_hash,
        field_hash,
        completion,
    })
}

fn first_date_guard(
    records: &[ArchiveItemV1],
    cutoff_ms: i64,
) -> Result<Option<(usize, &ArchiveItemV1)>, String> {
    for (index, record) in records.iter().take(ARCHIVE_RECORD_CAP).enumerate() {
        let remote_created_at_ms = record
            .remote_created_at_ms
            .ok_or_else(|| "archive_time_range_unavailable".to_string())?;
        if remote_created_at_ms < cutoff_ms {
            return Ok(Some((index, record)));
        }
    }
    if let Some(guard) = records.get(ARCHIVE_RECORD_CAP) {
        if guard
            .remote_created_at_ms
            .is_some_and(|value| value < cutoff_ms)
        {
            return Ok(Some((ARCHIVE_RECORD_CAP, guard)));
        }
    }
    Ok(None)
}

fn validate_positions(records: &[ArchiveItemV1]) -> Result<(), String> {
    if records
        .iter()
        .enumerate()
        .any(|(index, record)| record.source_position != index as i64)
    {
        return Err("invalid_source_position".to_string());
    }
    Ok(())
}
