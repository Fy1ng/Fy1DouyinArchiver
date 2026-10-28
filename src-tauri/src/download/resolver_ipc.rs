use std::io::{Read, Write};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::{DownloadError, DownloadQuality, ResolvedMediaManifest};

pub const MAX_RESOLVER_FRAME_SIZE: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolverRequest {
    pub request_id: u64,
    pub account_id: i64,
    pub profile_key: String,
    pub aweme_id: String,
    pub canonical_url: String,
    pub quality: DownloadQuality,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ResolverResponse {
    Resolved {
        request_id: u64,
        manifest: ResolvedMediaManifest,
    },
    Error {
        request_id: u64,
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResolverControl {
    Cancel,
    Shutdown,
}

pub fn write_frame<W: Write, T: Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<(), DownloadError> {
    let payload = serde_json::to_vec(message)
        .map_err(|_| frame_error("resolver_frame_invalid_json", "解析器消息无法序列化"))?;
    if payload.len() > MAX_RESOLVER_FRAME_SIZE {
        return Err(frame_error(
            "resolver_frame_too_large",
            "解析器消息超过 8 MiB 限制",
        ));
    }
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .and_then(|_| writer.write_all(&payload))
        .map_err(|_| frame_error("resolver_frame_io_failed", "解析器消息写入失败"))
}

pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, DownloadError> {
    let mut length = [0_u8; 4];
    reader
        .read_exact(&mut length)
        .map_err(|_| frame_error("resolver_frame_io_failed", "解析器消息读取失败"))?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_RESOLVER_FRAME_SIZE {
        return Err(frame_error(
            "resolver_frame_too_large",
            "解析器消息超过 8 MiB 限制",
        ));
    }
    let mut payload = vec![0_u8; length];
    reader
        .read_exact(&mut payload)
        .map_err(|_| frame_error("resolver_frame_io_failed", "解析器消息读取失败"))?;
    let text = String::from_utf8(payload)
        .map_err(|_| frame_error("resolver_frame_invalid_utf8", "解析器消息不是有效 UTF-8"))?;
    serde_json::from_str(&text)
        .map_err(|_| frame_error("resolver_frame_invalid_json", "解析器消息不是有效 JSON"))
}

fn frame_error(code: &str, message: &str) -> DownloadError {
    DownloadError {
        code: code.to_string(),
        message: message.to_string(),
    }
}
