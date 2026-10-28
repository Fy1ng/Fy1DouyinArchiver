use super::archive_snapshot::{RawArchiveItem, RawRemoteFolder};
use crate::sync::{ArchiveItemV1, RemoteFolderDescriptorV1};
use serde_json::Value;
use std::collections::HashSet;
use url::Url;

const MAX_ARCHIVE_TIME_MS: i64 = 9_999_999_999_999;
const MAX_ARCHIVE_TIME_SECONDS: i64 = 9_999_999_999;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveTimeUnit {
    Seconds,
    Milliseconds,
}

pub fn normalize_archive_items(
    items: &[RawArchiveItem],
    expected_time_unit: Option<ArchiveTimeUnit>,
) -> Result<(Option<ArchiveTimeUnit>, Vec<ArchiveItemV1>), String> {
    let time_unit = detect_time_unit(items, expected_time_unit)?;
    let mut ids = HashSet::new();
    let records = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            if item.source_position != index as i64 {
                return Err("invalid_source_position".to_string());
            }
            let aweme_id = string_id(&item.aweme_id, "invalid_archive_aweme_id")?;
            if aweme_id.len() > 64 || !ids.insert(aweme_id.clone()) {
                return Err("invalid_archive_aweme_id".to_string());
            }
            let (canonical_url, content_type) = canonical_archive_url(&item.href, &aweme_id)?;
            validate_content_type(item.aweme_type.as_ref(), content_type)?;
            let remote_created_at_ms =
                normalize_archive_time(item.remote_created_at.as_ref(), time_unit)?;
            let cover_url = item.cover_urls.as_ref().and_then(first_https_url);
            let like_count = item
                .like_count
                .as_ref()
                .and_then(Value::as_i64)
                .filter(|value| *value >= 0);
            Ok(ArchiveItemV1 {
                aweme_id,
                canonical_url,
                content_type: content_type.to_string(),
                source_position: item.source_position,
                remote_created_at_ms,
                title: clean_text(item.item_title.as_ref())
                    .or_else(|| clean_text(item.desc.as_ref())),
                author_name: clean_text(item.author_name.as_ref()),
                cover_url,
                like_count,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((time_unit, records))
}

pub fn normalize_remote_folders(
    folders: &[RawRemoteFolder],
) -> Result<Vec<RemoteFolderDescriptorV1>, String> {
    let mut ids = HashSet::new();
    folders
        .iter()
        .enumerate()
        .map(|(index, folder)| {
            if folder.source_position != index as i64 {
                return Err("invalid_source_position".to_string());
            }
            let remote_folder_id = folder
                .remote_folder_id
                .as_str()
                .filter(|value| {
                    !value.is_empty()
                        && value.len() <= 256
                        && value.trim() == *value
                        && !value.chars().any(char::is_control)
                })
                .ok_or_else(|| "invalid_remote_folder_id".to_string())?
                .to_string();
            if !ids.insert(remote_folder_id.clone()) {
                return Err("duplicate_remote_folder_id".to_string());
            }
            let name = clean_text(Some(&folder.name))
                .filter(|value| value.chars().count() <= 256)
                .ok_or_else(|| "invalid_remote_folder_name".to_string())?;
            Ok(RemoteFolderDescriptorV1 {
                remote_folder_id,
                name,
                source_position: folder.source_position,
            })
        })
        .collect()
}

pub fn normalize_active_remote_folder(
    remote_folder_id: Option<&Value>,
    name: Option<&Value>,
    requested_remote_folder_id: &str,
) -> Result<(String, String), String> {
    let remote_folder_id = remote_folder_id
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 256
                && value.trim() == *value
                && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| "remote_folder_not_selected".to_string())?;
    if remote_folder_id != requested_remote_folder_id {
        return Err("remote_folder_not_selected".to_string());
    }
    let name = clean_text(name)
        .filter(|value| value.chars().count() <= 256)
        .ok_or_else(|| "invalid_remote_folder_name".to_string())?;
    Ok((remote_folder_id.to_string(), name))
}

fn detect_time_unit(
    items: &[RawArchiveItem],
    expected: Option<ArchiveTimeUnit>,
) -> Result<Option<ArchiveTimeUnit>, String> {
    let values = items
        .iter()
        .filter_map(|item| item.remote_created_at.as_ref())
        .filter(|value| !value.is_null())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(expected);
    }
    if values.iter().any(|value| value.as_i64().is_none()) {
        return Err("invalid_archive_time".to_string());
    }
    let observed = values
        .iter()
        .map(|value| classify_time(value))
        .collect::<Result<HashSet<_>, _>>()?;
    if observed.len() != 1 {
        return Err("mixed_archive_time_unit".to_string());
    }
    let detected = *observed.iter().next().unwrap();
    if let Some(expected) = expected {
        return (detected == expected)
            .then_some(Some(expected))
            .ok_or_else(|| "mixed_archive_time_unit".to_string());
    }
    Ok(Some(detected))
}

fn classify_time(value: &Value) -> Result<ArchiveTimeUnit, String> {
    let Some(raw) = value.as_i64().filter(|value| *value >= 0) else {
        return Err("invalid_archive_time".to_string());
    };
    if raw <= MAX_ARCHIVE_TIME_SECONDS {
        Ok(ArchiveTimeUnit::Seconds)
    } else if raw <= MAX_ARCHIVE_TIME_MS {
        Ok(ArchiveTimeUnit::Milliseconds)
    } else {
        Err("invalid_archive_time".to_string())
    }
}

fn valid_time(value: &Value, unit: ArchiveTimeUnit) -> bool {
    classify_time(value).is_ok_and(|detected| detected == unit)
}

fn normalize_archive_time(
    value: Option<&Value>,
    unit: Option<ArchiveTimeUnit>,
) -> Result<Option<i64>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let unit = unit.ok_or_else(|| "invalid_archive_time".to_string())?;
    if !valid_time(value, unit) {
        return Err("invalid_archive_time".to_string());
    }
    let raw = value.as_i64().unwrap();
    Ok(Some(match unit {
        ArchiveTimeUnit::Seconds => raw * 1_000,
        ArchiveTimeUnit::Milliseconds => raw,
    }))
}

fn string_id(value: &Value, error: &str) -> Result<String, String> {
    value
        .as_str()
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .map(str::to_string)
        .ok_or_else(|| error.to_string())
}

fn canonical_archive_url(value: &str, aweme_id: &str) -> Result<(String, &'static str), String> {
    let base = Url::parse("https://www.douyin.com").unwrap();
    let url = base
        .join(value)
        .map_err(|_| "invalid_archive_url".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url
            .host_str()
            .is_none_or(|host| host != "douyin.com" && !host.ends_with(".douyin.com"))
    {
        return Err("invalid_archive_url".to_string());
    }
    let parts = url.path().trim_matches('/').split('/').collect::<Vec<_>>();
    if parts.len() != 2 || !matches!(parts[0], "video" | "note" | "article") {
        return Err("invalid_archive_url".to_string());
    }
    if parts[1] != aweme_id {
        return Err("archive_url_id_mismatch".to_string());
    }
    let (content_type, canonical_path) = match parts[0] {
        "video" => ("video", "video"),
        "note" => ("note", "note"),
        "article" => ("unknown", "article"),
        _ => unreachable!(),
    };
    Ok((
        format!("https://www.douyin.com/{canonical_path}/{aweme_id}"),
        content_type,
    ))
}

fn validate_content_type(value: Option<&Value>, path_type: &str) -> Result<(), String> {
    let aweme_type = value.and_then(Value::as_i64);
    if (aweme_type == Some(68) && path_type == "video")
        || (matches!(aweme_type, Some(0 | 61)) && path_type == "note")
    {
        Err("archive_content_type_conflict".to_string())
    } else {
        Ok(())
    }
}

fn clean_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|value| !value.is_empty())
}

fn first_https_url(value: &Value) -> Option<String> {
    let candidates = value
        .as_array()
        .map(|values| values.as_slice())
        .unwrap_or_else(|| std::slice::from_ref(value));
    candidates.iter().find_map(|candidate| {
        candidate
            .as_str()
            .and_then(|value| Url::parse(value).ok())
            .filter(|url| url.scheme() == "https" && url.as_str().len() <= 4_096)
            .map(|url| url.to_string())
    })
}
