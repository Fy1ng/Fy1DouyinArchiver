use crate::CollectorRecordV2;
use chrono::{FixedOffset, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

const EARLIEST_HISTORY_MS: i64 = 946_684_800_000;
const FUTURE_TOLERANCE_MS: i64 = 300_000;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawHistoryItem {
    pub aweme_id: Value,
    pub aweme_type: Option<Value>,
    pub history_time: Value,
    pub href: String,
    pub source_position: i64,
    pub author_name: Option<Value>,
    pub item_title: Option<Value>,
    pub desc: Option<Value>,
    pub cover_urls: Option<Value>,
    pub like_count: Option<Value>,
    pub history_progress_rate: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryTimeUnit { Seconds, Milliseconds }

impl HistoryTimeUnit { pub fn as_str(self) -> &'static str { match self { Self::Seconds => "seconds", Self::Milliseconds => "milliseconds" } } }

pub fn normalize_snapshot(items: &[RawHistoryItem], sync_started_at_ms: i64, expected: Option<HistoryTimeUnit>) -> Result<(HistoryTimeUnit, Vec<CollectorRecordV2>), String> {
    let unit = detect_unit(items, sync_started_at_ms, expected)?;
    let records = items.iter().map(|item| normalize_record(item, sync_started_at_ms, unit)).collect::<Result<Vec<_>, _>>()?;
    Ok((unit, records))
}

fn detect_unit(items: &[RawHistoryItem], started: i64, expected: Option<HistoryTimeUnit>) -> Result<HistoryTimeUnit, String> {
    if items.is_empty() { return Ok(expected.unwrap_or(HistoryTimeUnit::Milliseconds)); }
    if let Some(unit) = expected {
        return items.iter().all(|item| valid_time(&item.history_time, unit, started)).then_some(unit).ok_or_else(|| "invalid_history_time".to_string());
    }
    let candidates = [HistoryTimeUnit::Seconds, HistoryTimeUnit::Milliseconds].into_iter().filter(|unit| items.iter().all(|item| valid_time(&item.history_time, *unit, started))).collect::<Vec<_>>();
    if candidates.len() != 1 { return Err("ambiguous_history_time_unit".to_string()); }
    Ok(candidates[0])
}

fn valid_time(value: &Value, unit: HistoryTimeUnit, started: i64) -> bool {
    let Some(raw) = value.as_i64() else { return false; };
    let Some(ms) = (if unit == HistoryTimeUnit::Seconds { raw.checked_mul(1000) } else { Some(raw) }) else { return false; };
    raw > 0 && ms >= EARLIEST_HISTORY_MS && ms <= started + FUTURE_TOLERANCE_MS
}

fn normalize_record(item: &RawHistoryItem, started: i64, unit: HistoryTimeUnit) -> Result<CollectorRecordV2, String> {
    let aweme_id = item.aweme_id.as_str().map(str::to_string).or_else(|| item.aweme_id.as_u64().map(|value| value.to_string())).filter(|value| value.chars().all(|c| c.is_ascii_digit())).ok_or_else(|| "invalid_aweme_id".to_string())?;
    let aweme_type = item.aweme_type.as_ref().and_then(Value::as_i64);
    if !valid_time(&item.history_time, unit, started) { return Err("invalid_history_time".to_string()); }
    let raw_time = item.history_time.as_i64().unwrap();
    let history_time_ms = if unit == HistoryTimeUnit::Seconds { raw_time * 1000 } else { raw_time };
    let (canonical_url, path_type) = normalize_url(&item.href, &aweme_id)?;
    let content_type = resolve_content_type(aweme_type, path_type)?;
    let watched = Utc.timestamp_millis_opt(history_time_ms).single().ok_or_else(|| "invalid_history_time".to_string())?;
    let offset = FixedOffset::east_opt(8 * 3600).unwrap();
    Ok(CollectorRecordV2 {
        aweme_id, aweme_type, canonical_url, content_type, history_time_ms,
        watched_at: watched.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        watched_date: watched.with_timezone(&offset).format("%Y-%m-%d").to_string(),
        source_position: item.source_position,
        title: clean(item.item_title.as_ref()).or_else(|| clean(item.desc.as_ref())),
        author_name: clean(item.author_name.as_ref()),
        cover_url: item.cover_urls.as_ref().and_then(Value::as_array).and_then(|items| items.iter().filter_map(Value::as_str).find_map(|value| Url::parse(value).ok().filter(|url| matches!(url.scheme(), "http" | "https")).map(|url| url.to_string()))),
        like_count: item.like_count.as_ref().and_then(Value::as_i64).filter(|value| *value >= 0),
        watch_progress_rate: normalize_progress(item.history_progress_rate.as_ref()),
    })
}

fn normalize_url(value: &str, aweme_id: &str) -> Result<(String, &'static str), String> {
    let base = Url::parse("https://www.douyin.com").unwrap();
    let mut url = base.join(value).map_err(|_| "invalid_canonical_url".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(|host| host != "douyin.com" && !host.ends_with(".douyin.com")) { return Err("invalid_canonical_url".to_string()); }
    let segments = url.path().trim_end_matches('/').split('/').filter(|part| !part.is_empty()).map(str::to_string).collect::<Vec<_>>();
    let path_type = if segments.len() == 2 && matches!(segments[0].as_str(), "video" | "note") && segments[1].chars().all(|c| c.is_ascii_digit()) {
        if segments[1] != aweme_id { return Err("url_id_mismatch".to_string()); }
        if segments[0] == "video" { "video" } else { "note" }
    } else { "unknown" };
    url.set_scheme("https").ok(); url.set_host(Some("www.douyin.com")).map_err(|_| "invalid_canonical_url".to_string())?; url.set_query(None); url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_string(); url.set_path(if path.is_empty() { "/" } else { &path });
    let mut result = url.to_string(); if path_type != "unknown" { result = result.trim_end_matches('/').to_string(); }
    Ok((result, path_type))
}

fn resolve_content_type(aweme_type: Option<i64>, path: &str) -> Result<String, String> {
    if aweme_type == Some(68) && path == "video" || matches!(aweme_type, Some(0 | 61)) && path == "note" { return Err("content_type_conflict".to_string()); }
    Ok(if path != "unknown" { path } else if aweme_type == Some(68) { "note" } else if matches!(aweme_type, Some(0 | 61)) { "video" } else { "unknown" }.to_string())
}

fn clean(value: Option<&Value>) -> Option<String> { value.and_then(Value::as_str).map(|value| value.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|value| !value.is_empty()) }

fn normalize_progress(value: Option<&Value>) -> Option<f64> {
    let numeric = value.and_then(|value| value.as_f64().or_else(|| value.as_str().and_then(|text| text.trim_end_matches('%').trim().parse::<f64>().ok())))?;
    if !(0.0..=100.0).contains(&numeric) { return None; }
    Some((((if numeric <= 1.0 { numeric } else { numeric / 100.0 }) * 10_000.0).round()) / 10_000.0)
}
