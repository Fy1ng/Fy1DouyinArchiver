use douyin_history_archive::download::{
    select_format, stable_asset_key, DownloadAssetState, DownloadCandidate, DownloadError,
    DownloadItemState, DownloadJobState, DownloadQuality, ResolvedFormat,
};
use serde_json::json;

fn format(id: &str, width: u32, height: u32, bitrate: u64, codec: &str) -> ResolvedFormat {
    ResolvedFormat {
        id: id.to_string(),
        width,
        height,
        bitrate,
        codec: codec.to_string(),
        mime_type: Some("video/mp4".to_string()),
        expected_size: None,
        source_url: format!("https://media.example/{id}"),
        request_headers: Default::default(),
        source_expires_at: None,
    }
}

#[test]
fn download_contracts_use_camel_case_and_string_backed_enums() {
    let candidate = DownloadCandidate {
        aweme_id: "123456".to_string(),
        canonical_url: "https://www.douyin.com/video/123456".to_string(),
        content_type: "video".to_string(),
        title: Some("标题".to_string()),
        author_name: Some("作者".to_string()),
        published_at: None,
        filter_context: Some("最近七天".to_string()),
    };

    let value = serde_json::to_value(candidate).expect("serialize candidate");

    assert_eq!(value["awemeId"], "123456");
    assert_eq!(value["canonicalUrl"], "https://www.douyin.com/video/123456");
    assert_eq!(value["authorName"], "作者");
    assert_eq!(
        serde_json::to_value(DownloadQuality::SpaceSaver).unwrap(),
        "space_saver"
    );
    assert_eq!(
        serde_json::to_value(DownloadJobState::CompletedWithErrors).unwrap(),
        "completed_with_errors"
    );
    assert_eq!(
        serde_json::to_value(DownloadItemState::Resolving).unwrap(),
        "resolving"
    );
    assert_eq!(
        serde_json::to_value(DownloadAssetState::Superseded).unwrap(),
        "superseded"
    );
}

#[test]
fn external_contracts_reject_unknown_fields_and_non_decimal_aweme_ids() {
    let base = json!({
        "awemeId": "123456",
        "canonicalUrl": "https://www.douyin.com/video/123456",
        "contentType": "video",
        "title": null,
        "authorName": null,
        "publishedAt": null,
        "filterContext": null
    });
    let mut unknown = base.clone();
    unknown["temporaryUrl"] = json!("https://should-not-persist.example");
    assert!(serde_json::from_value::<DownloadCandidate>(unknown).is_err());

    let mut invalid_id = base;
    invalid_id["awemeId"] = json!("12x34");
    assert!(serde_json::from_value::<DownloadCandidate>(invalid_id).is_err());
}

#[test]
fn balanced_prefers_best_h264_at_or_below_1080p() {
    let formats = vec![
        format("2160p", 3840, 2160, 12_000, "h265"),
        format("1080p-h265", 1920, 1080, 6_000, "h265"),
        format("1080p-h264", 1920, 1080, 6_000, "h264"),
    ];

    assert_eq!(
        select_format(&formats, DownloadQuality::Balanced)
            .unwrap()
            .id,
        "1080p-h264"
    );
}

#[test]
fn space_saver_selects_the_best_format_at_or_below_720p() {
    let formats = vec![
        format("1080p", 1920, 1080, 6_000, "h264"),
        format("720p", 1280, 720, 3_000, "h264"),
        format("480p", 854, 480, 1_500, "h264"),
    ];

    assert_eq!(
        select_format(&formats, DownloadQuality::SpaceSaver)
            .unwrap()
            .id,
        "720p"
    );
}

#[test]
fn capped_profiles_fall_back_to_the_smallest_format_above_the_cap() {
    let formats = vec![
        format("2160p", 3840, 2160, 12_000, "h265"),
        format("1440p", 2560, 1440, 8_000, "h264"),
    ];

    assert_eq!(
        select_format(&formats, DownloadQuality::Balanced)
            .unwrap()
            .id,
        "1440p"
    );
}

#[test]
fn highest_selects_the_largest_format_deterministically() {
    let formats = vec![
        format("1080p", 1920, 1080, 6_000, "h264"),
        format("2160p", 3840, 2160, 12_000, "h265"),
    ];

    assert_eq!(
        select_format(&formats, DownloadQuality::Highest)
            .unwrap()
            .id,
        "2160p"
    );
}

#[test]
fn asset_keys_are_stable_and_independent_of_source_urls() {
    assert_eq!(stable_asset_key("video", 0).unwrap(), "video:primary");
    assert_eq!(stable_asset_key("image", 1).unwrap(), "image:0001");
    assert_eq!(
        stable_asset_key("live-image", 12).unwrap(),
        "live-image:0012"
    );
    assert_eq!(
        stable_asset_key("live-video", 2).unwrap(),
        "live-video:0002"
    );
    assert_eq!(stable_asset_key("audio", 0).unwrap(), "audio:music");
}

#[test]
fn download_errors_expose_stable_codes_and_chinese_descriptions() {
    let error = select_format(&[], DownloadQuality::Highest).expect_err("empty formats");

    assert_eq!(error.code, "media_format_unavailable");
    assert_eq!(error.message, "未找到可下载的视频格式");
    assert_eq!(error.to_string(), "未找到可下载的视频格式");
    assert!(error.message.chars().any(|character| !character.is_ascii()));
    assert_eq!(
        stable_asset_key("unknown", 0),
        Err(DownloadError::invalid_asset_type("unknown"))
    );
}
