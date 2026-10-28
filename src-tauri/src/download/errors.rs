use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadError {
    pub code: String,
    pub message: String,
}

impl DownloadError {
    pub fn media_format_unavailable() -> Self {
        Self {
            code: "media_format_unavailable".to_string(),
            message: "未找到可下载的视频格式".to_string(),
        }
    }

    pub fn invalid_asset_type(asset_type: &str) -> Self {
        Self {
            code: "invalid_asset_type".to_string(),
            message: format!("不支持的下载资源类型：{asset_type}"),
        }
    }
}

impl fmt::Display for DownloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DownloadError {}
