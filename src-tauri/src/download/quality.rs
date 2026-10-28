use std::cmp::Ordering;

use super::{DownloadError, DownloadQuality, ResolvedFormat};

pub fn select_format(
    formats: &[ResolvedFormat],
    quality: DownloadQuality,
) -> Result<ResolvedFormat, DownloadError> {
    if formats.is_empty() {
        return Err(DownloadError::media_format_unavailable());
    }

    let cap = match quality {
        DownloadQuality::Highest => None,
        DownloadQuality::Balanced => Some(1080),
        DownloadQuality::SpaceSaver => Some(720),
    };

    let selected = match cap {
        None => formats
            .iter()
            .max_by(|left, right| quality_order(left, right)),
        Some(max_height) => formats
            .iter()
            .filter(|format| format.height <= max_height)
            .max_by(|left, right| quality_order(left, right))
            .or_else(|| {
                formats
                    .iter()
                    .min_by(|left, right| fallback_order(left, right))
            }),
    };

    selected
        .cloned()
        .ok_or_else(DownloadError::media_format_unavailable)
}

fn quality_order(left: &ResolvedFormat, right: &ResolvedFormat) -> Ordering {
    left.height
        .cmp(&right.height)
        .then_with(|| left.width.cmp(&right.width))
        .then_with(|| left.bitrate.cmp(&right.bitrate))
        .then_with(|| codec_preference(&left.codec).cmp(&codec_preference(&right.codec)))
        .then_with(|| right.id.cmp(&left.id))
}

fn fallback_order(left: &ResolvedFormat, right: &ResolvedFormat) -> Ordering {
    left.height
        .cmp(&right.height)
        .then_with(|| left.width.cmp(&right.width))
        .then_with(|| left.bitrate.cmp(&right.bitrate))
        .then_with(|| codec_preference(&right.codec).cmp(&codec_preference(&left.codec)))
        .then_with(|| left.id.cmp(&right.id))
}

fn codec_preference(codec: &str) -> u8 {
    let normalized = codec.to_ascii_lowercase().replace(['.', '-'], "");
    u8::from(normalized.contains("h264") || normalized.contains("avc"))
}
