use super::{contracts::{CollectorProgress, COLLECTOR_SCHEMA_VERSION, COLLECTOR_TIME_ZONE}, normalization::{normalize_snapshot, HistoryTimeUnit}, snapshot::RawReactSnapshot};
use crate::{compute_collector_hashes, CollectorRange, CollectorRecordV2, CollectorSuccessV3};
use chrono::{FixedOffset, TimeZone, Utc};

pub fn create_range(mode_bottom: bool, days: u32, started: i64) -> CollectorRange {
    if mode_bottom { return CollectorRange::Bottom; }
    let days = days.clamp(1, 365);
    let offset = FixedOffset::east_opt(8 * 3600).unwrap();
    let date = Utc.timestamp_millis_opt(started).single().unwrap().with_timezone(&offset).date_naive();
    let midnight = offset.from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap()).single().unwrap().timestamp_millis();
    CollectorRange::Days { days, cutoff_ms: midnight - i64::from(days - 1) * 86_400_000 }
}

pub fn normalize_raw_snapshot(raw: RawReactSnapshot, started: i64, expected: Option<HistoryTimeUnit>) -> Result<(HistoryTimeUnit, Vec<CollectorRecordV2>), String> { normalize_snapshot(&raw.items, started, expected) }

pub fn validate_growth(previous: &[CollectorRecordV2], current: &[CollectorRecordV2]) -> Result<(), String> {
    if current.len() < previous.len() { return Err("snapshot_prefix_changed".to_string()); }
    for (index, record) in current.iter().enumerate() {
        if record.source_position != index as i64 { return Err("invalid_source_position".to_string()); }
        if index > 0 && current[index - 1].history_time_ms < record.history_time_ms { return Err("history_time_order_invalid".to_string()); }
        if index < previous.len() && (record.aweme_id != previous[index].aweme_id || record.history_time_ms != previous[index].history_time_ms) { return Err("snapshot_prefix_changed".to_string()); }
    }
    let mut identities = std::collections::HashSet::new();
    if current.iter().any(|record| !identities.insert((&record.aweme_id, record.history_time_ms))) { return Err("duplicate_history_identity".to_string()); }
    Ok(())
}

pub fn range_complete(records: &[CollectorRecordV2], range: &CollectorRange, bottom: bool) -> bool { match range { CollectorRange::Bottom => bottom, CollectorRange::Days { cutoff_ms, .. } => bottom || records.iter().any(|record| record.history_time_ms < *cutoff_ms) } }
pub fn filter_range(records: Vec<CollectorRecordV2>, range: &CollectorRange) -> Vec<CollectorRecordV2> { match range { CollectorRange::Bottom => records, CollectorRange::Days { cutoff_ms, .. } => records.into_iter().filter(|record| record.history_time_ms >= *cutoff_ms).collect() } }

pub fn finish_result(account_id: i64, job_id: i64, started: i64, range: CollectorRange, unit: HistoryTimeUnit, records: Vec<CollectorRecordV2>) -> Result<CollectorSuccessV3, String> {
    let (identity_hash, field_hash) = compute_collector_hashes(&records)?;
    Ok(CollectorSuccessV3 { schema_version: COLLECTOR_SCHEMA_VERSION, account_id, sync_job_id: job_id, sync_started_at_ms: started, calendar_time_zone: COLLECTOR_TIME_ZONE.to_string(), range, detected_history_time_unit: unit.as_str().to_string(), identity_hash, field_hash, records })
}

pub fn progress(account_id: i64, job_id: i64, phase: &str, count: usize, elapsed: i64, retry: i64, can_cancel: bool) -> CollectorProgress { CollectorProgress { schema_version: COLLECTOR_SCHEMA_VERSION, account_id, sync_job_id: job_id, phase: phase.to_string(), record_count: count as i64, elapsed_ms: elapsed.max(0), retry_count: retry, can_cancel } }
