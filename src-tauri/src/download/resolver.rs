use std::{collections::BTreeMap, future::Future};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::{
    select_format, stable_asset_key, DownloadError, DownloadQuality, ResolvedAsset, ResolvedFormat,
    ResolvedMediaManifest,
};

pub trait MediaResolver {
    fn resolve(
        &self,
        request: super::ResolverRequest,
    ) -> impl Future<Output = Result<ResolvedMediaManifest, DownloadError>> + Send;
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureManifest {
    aweme_id: String,
    content_type: String,
    #[serde(default)]
    video: Option<VideoSource>,
    #[serde(default)]
    images: Vec<MediaSource>,
    #[serde(default)]
    music: Option<MediaSource>,
    #[serde(default)]
    live_photos: Vec<LivePhotoSource>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VideoSource {
    formats: Vec<ResolvedFormat>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LivePhotoSource {
    image: MediaSource,
    video: VideoSource,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MediaSource {
    source_url: String,
    #[serde(default)]
    request_headers: BTreeMap<String, String>,
    #[serde(default)]
    expected_size: Option<u64>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    bitrate: Option<u64>,
    #[serde(default)]
    codec: Option<String>,
    #[serde(default)]
    source_expires_at: Option<String>,
}

pub fn parse_manifest(
    value: Value,
    quality: DownloadQuality,
) -> Result<ResolvedMediaManifest, DownloadError> {
    let raw: FixtureManifest = serde_json::from_value(value).map_err(|_| invalid_manifest())?;
    if raw.aweme_id.is_empty() || !raw.aweme_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_manifest());
    }

    match raw.content_type.as_str() {
        "video"
            if raw.video.is_some()
                && raw.images.is_empty()
                && raw.music.is_none()
                && raw.live_photos.is_empty() => {}
        "image" if raw.video.is_none() && !raw.images.is_empty() && raw.live_photos.is_empty() => {}
        "live_photo"
            if raw.video.is_none() && raw.images.is_empty() && !raw.live_photos.is_empty() => {}
        "video" | "image" | "live_photo" => return Err(invalid_manifest()),
        _ => return Err(unsupported_content()),
    }

    let has_images = !raw.images.is_empty();
    let has_live_photos = !raw.live_photos.is_empty();
    let mut assets = Vec::new();
    if let Some(video) = raw.video {
        assets.push(format_asset(
            select_format(&video.formats, quality)?,
            "video",
            0,
        )?);
    }
    for (index, image) in raw.images.into_iter().enumerate() {
        assets.push(source_asset(image, "image", ordinal(index))?);
    }
    for (index, pair) in raw.live_photos.into_iter().enumerate() {
        let ordinal = ordinal(index);
        assets.push(source_asset(pair.image, "live-image", ordinal)?);
        assets.push(format_asset(
            select_format(&pair.video.formats, quality)?,
            "live-video",
            ordinal,
        )?);
    }
    if let Some(music) = raw.music {
        assets.push(source_asset(music, "audio", 0)?);
    }

    if assets.is_empty() {
        return Err(invalid_manifest());
    }

    let content_type = if has_live_photos {
        "live_photo"
    } else if has_images {
        "image"
    } else {
        "video"
    };
    Ok(ResolvedMediaManifest {
        aweme_id: raw.aweme_id,
        content_type: content_type.to_string(),
        assets,
    })
}

pub fn parse_detail_response(
    value: Value,
    expected_aweme_id: &str,
    quality: DownloadQuality,
) -> Result<ResolvedMediaManifest, DownloadError> {
    let detail = value
        .get("aweme_detail")
        .and_then(Value::as_object)
        .ok_or_else(invalid_manifest)?;
    let aweme_id = string_value(detail.get("aweme_id")).ok_or_else(invalid_manifest)?;
    if aweme_id != expected_aweme_id {
        return Err(DownloadError {
            code: "download_manifest_identity_mismatch".into(),
            message: "媒体响应与请求作品不一致".into(),
        });
    }

    let gallery = detail
        .get("image_post_info")
        .and_then(Value::as_object)
        .and_then(|info| info.get("images").or_else(|| info.get("image_list")))
        .or_else(|| detail.get("images"))
        .and_then(Value::as_array);
    if let Some(images) = gallery.filter(|images| !images.is_empty()) {
        return parse_upstream_gallery(&aweme_id, detail, images, quality);
    }

    let video = detail.get("video").ok_or_else(unsupported_content)?;
    let formats = upstream_video_formats(video)?;
    let asset = format_asset(select_format(&formats, quality)?, "video", 0)?;
    Ok(ResolvedMediaManifest {
        aweme_id,
        content_type: "video".into(),
        assets: vec![asset],
    })
}

fn parse_upstream_gallery(
    aweme_id: &str,
    detail: &serde_json::Map<String, Value>,
    images: &[Value],
    quality: DownloadQuality,
) -> Result<ResolvedMediaManifest, DownloadError> {
    let has_live = images.iter().any(|image| image.get("video").is_some());
    let mut assets = Vec::new();
    for (index, image) in images.iter().enumerate() {
        let ordinal = ordinal(index);
        let image_source = upstream_media_source(image, "image/jpeg")?;
        assets.push(source_asset(
            image_source,
            if has_live { "live-image" } else { "image" },
            ordinal,
        )?);
        if let Some(video) = image.get("video") {
            let formats = upstream_video_formats(video)?;
            assets.push(format_asset(
                select_format(&formats, quality)?,
                "live-video",
                ordinal,
            )?);
        }
    }
    if let Some(music) = detail
        .get("music")
        .and_then(|music| music.get("play_url").or_else(|| music.get("playUrl")))
    {
        if let Ok(source) = upstream_media_source(music, "audio/mpeg") {
            assets.push(source_asset(source, "audio", 0)?);
        }
    }
    Ok(ResolvedMediaManifest {
        aweme_id: aweme_id.to_string(),
        content_type: if has_live { "live_photo" } else { "image" }.into(),
        assets,
    })
}

fn upstream_video_formats(video: &Value) -> Result<Vec<ResolvedFormat>, DownloadError> {
    let mut formats = Vec::new();
    if let Some(entries) = video.get("bit_rate").and_then(Value::as_array) {
        for (index, entry) in entries.iter().enumerate() {
            let Some(address) = entry
                .get("play_addr")
                .or_else(|| entry.get("play_addr_h264"))
                .or_else(|| entry.get("play_addr_265"))
            else {
                continue;
            };
            let Some(source_url) = first_url(address) else {
                continue;
            };
            let codec = if entry.get("is_h265").and_then(Value::as_i64) == Some(1)
                || entry
                    .get("gear_name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.to_ascii_lowercase().contains("h265"))
            {
                "h265"
            } else {
                "h264"
            };
            let id = entry
                .get("gear_name")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| string_value(address.get("uri")))
                .unwrap_or_else(|| format!("format-{index}"));
            formats.push(ResolvedFormat {
                id,
                width: unsigned_u32(address.get("width"))
                    .or_else(|| unsigned_u32(entry.get("width")))
                    .or_else(|| unsigned_u32(video.get("width")))
                    .unwrap_or(0),
                height: unsigned_u32(address.get("height"))
                    .or_else(|| unsigned_u32(entry.get("height")))
                    .or_else(|| unsigned_u32(video.get("height")))
                    .unwrap_or(0),
                bitrate: unsigned_u64(entry.get("bit_rate")).unwrap_or(0),
                codec: codec.into(),
                mime_type: Some("video/mp4".into()),
                expected_size: unsigned_u64(address.get("data_size")),
                source_expires_at: expiry_from_url(&source_url),
                source_url,
                request_headers: BTreeMap::new(),
            });
        }
    }
    if formats.is_empty() {
        let address = video
            .get("play_addr")
            .or_else(|| video.get("play_addr_h264"))
            .or_else(|| video.get("download_addr"))
            .ok_or_else(invalid_manifest)?;
        let source_url = first_url(address).ok_or_else(invalid_manifest)?;
        formats.push(ResolvedFormat {
            id: string_value(address.get("uri")).unwrap_or_else(|| "source".into()),
            width: unsigned_u32(address.get("width"))
                .or_else(|| unsigned_u32(video.get("width")))
                .unwrap_or(0),
            height: unsigned_u32(address.get("height"))
                .or_else(|| unsigned_u32(video.get("height")))
                .unwrap_or(0),
            bitrate: 0,
            codec: "h264".into(),
            mime_type: Some("video/mp4".into()),
            expected_size: unsigned_u64(address.get("data_size")),
            source_expires_at: expiry_from_url(&source_url),
            source_url,
            request_headers: BTreeMap::new(),
        });
    }
    Ok(formats)
}

fn upstream_media_source(value: &Value, default_mime: &str) -> Result<MediaSource, DownloadError> {
    let candidates = [
        value.get("watermark_free_download_url_list"),
        value.get("url_list"),
        value.get("download_url_list"),
        value.get("download_url"),
        value.get("display_image"),
    ];
    let source_container = candidates
        .into_iter()
        .flatten()
        .find(|candidate| first_url(candidate).is_some())
        .ok_or_else(invalid_manifest)?;
    let source_url = first_url(source_container).ok_or_else(invalid_manifest)?;
    Ok(MediaSource {
        request_headers: BTreeMap::new(),
        expected_size: unsigned_u64(source_container.get("data_size"))
            .or_else(|| unsigned_u64(value.get("data_size"))),
        mime_type: Some(default_mime.into()),
        width: unsigned_u32(source_container.get("width"))
            .or_else(|| unsigned_u32(value.get("width"))),
        height: unsigned_u32(source_container.get("height"))
            .or_else(|| unsigned_u32(value.get("height"))),
        bitrate: None,
        codec: None,
        source_expires_at: expiry_from_url(&source_url),
        source_url,
    })
}

fn first_url(value: &Value) -> Option<String> {
    let list = value
        .as_array()
        .or_else(|| value.get("url_list").and_then(Value::as_array))?;
    list.iter()
        .filter_map(Value::as_str)
        .find(|url| url.starts_with("https://"))
        .map(str::to_string)
}

fn string_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn unsigned_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn unsigned_u32(value: Option<&Value>) -> Option<u32> {
    unsigned_u64(value).and_then(|value| u32::try_from(value).ok())
}

fn expiry_from_url(source_url: &str) -> Option<String> {
    let parsed = url::Url::parse(source_url).ok()?;
    let seconds = parsed.query_pairs().find_map(|(key, value)| {
        matches!(key.as_ref(), "x-expires" | "expires" | "expire")
            .then(|| value.parse::<i64>().ok())
            .flatten()
    })?;
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn ordinal(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

fn format_asset(
    format: ResolvedFormat,
    asset_type: &str,
    ordinal: u32,
) -> Result<ResolvedAsset, DownloadError> {
    Ok(ResolvedAsset {
        stable_key: stable_asset_key(asset_type, ordinal)?,
        asset_type: asset_type.to_string(),
        ordinal,
        actual_quality: Some(format.id),
        source_url: format.source_url,
        request_headers: format.request_headers,
        expected_size: format.expected_size,
        mime_type: format.mime_type,
        width: Some(format.width),
        height: Some(format.height),
        bitrate: Some(format.bitrate),
        codec: Some(format.codec),
        source_expires_at: format.source_expires_at,
    })
}

fn source_asset(
    source: MediaSource,
    asset_type: &str,
    ordinal: u32,
) -> Result<ResolvedAsset, DownloadError> {
    Ok(ResolvedAsset {
        stable_key: stable_asset_key(asset_type, ordinal)?,
        asset_type: asset_type.to_string(),
        ordinal,
        actual_quality: None,
        source_url: source.source_url,
        request_headers: source.request_headers,
        expected_size: source.expected_size,
        mime_type: source.mime_type,
        width: source.width,
        height: source.height,
        bitrate: source.bitrate,
        codec: source.codec,
        source_expires_at: source.source_expires_at,
    })
}

fn invalid_manifest() -> DownloadError {
    DownloadError {
        code: "download_manifest_invalid".to_string(),
        message: "媒体清单不完整或格式无效".to_string(),
    }
}

fn unsupported_content() -> DownloadError {
    DownloadError {
        code: "download_unsupported_content".to_string(),
        message: "当前作品没有可识别的原始媒体资源".to_string(),
    }
}
