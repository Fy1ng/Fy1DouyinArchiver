use std::collections::BTreeMap;

use serde::{de, Deserialize, Deserializer, Serialize};

use super::errors::DownloadError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadQuality {
    Highest,
    Balanced,
    SpaceSaver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadJobState {
    Queued,
    Running,
    Paused,
    Interrupted,
    Completed,
    CompletedWithErrors,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadItemState {
    Queued,
    Resolving,
    Downloading,
    Paused,
    Interrupted,
    Completed,
    Skipped,
    Failed,
    Superseded,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadAssetState {
    Queued,
    Downloading,
    Paused,
    Interrupted,
    Completed,
    Skipped,
    Failed,
    Superseded,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadCandidate {
    #[serde(deserialize_with = "deserialize_aweme_id")]
    pub aweme_id: String,
    pub canonical_url: String,
    pub content_type: String,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub published_at: Option<String>,
    pub filter_context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedFormat {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub bitrate: u64,
    pub codec: String,
    pub mime_type: Option<String>,
    pub expected_size: Option<u64>,
    pub source_url: String,
    #[serde(default)]
    pub request_headers: BTreeMap<String, String>,
    pub source_expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedAsset {
    pub stable_key: String,
    pub asset_type: String,
    pub ordinal: u32,
    pub actual_quality: Option<String>,
    pub source_url: String,
    #[serde(default)]
    pub request_headers: BTreeMap<String, String>,
    pub expected_size: Option<u64>,
    pub mime_type: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bitrate: Option<u64>,
    pub codec: Option<String>,
    pub source_expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedMediaManifest {
    #[serde(deserialize_with = "deserialize_aweme_id")]
    pub aweme_id: String,
    pub content_type: String,
    pub assets: Vec<ResolvedAsset>,
}

pub fn stable_asset_key(asset_type: &str, ordinal: u32) -> Result<String, DownloadError> {
    match asset_type {
        "video" if ordinal == 0 => Ok("video:primary".to_string()),
        "audio" if ordinal == 0 => Ok("audio:music".to_string()),
        "image" | "live-image" | "live-video" if ordinal > 0 => {
            Ok(format!("{asset_type}:{ordinal:04}"))
        }
        _ => Err(DownloadError::invalid_asset_type(asset_type)),
    }
}

fn deserialize_aweme_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty() || !value.chars().all(|character| character.is_ascii_digit()) {
        return Err(de::Error::custom("awemeId 必须是非空十进制字符串"));
    }
    Ok(value)
}
