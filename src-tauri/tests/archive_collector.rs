use douyin_history_archive::collector::archive_normalization::{
    normalize_active_remote_folder, normalize_archive_items, normalize_remote_folders,
    ArchiveTimeUnit,
};
use douyin_history_archive::collector::archive_snapshot::{
    archive_bottom_confirmed, archive_snapshots_stable, ArchiveWheelTarget, RawArchiveItem,
    RawArchiveSnapshot, RawRemoteFolder, RemoteFolderPageSnapshot,
};
use douyin_history_archive::collector::archive_sync_engine::{
    archive_snapshot_made_progress, consecutive_known_tail, decide_archive_stop,
    decide_remote_folder_discovery, validate_archive_growth, ArchiveStopDecision, ArchiveStopInput,
    RemoteFolderDiscoveryDecision,
};
use douyin_history_archive::settings_store::ArchiveListRangeV1;
use douyin_history_archive::sync::{
    compute_archive_hashes, ArchiveCompleteStopV1, ArchiveCompletionV1, ArchiveIncompleteReasonV1,
    ArchiveItemV1, ArchiveRunModeV1, RemoteFolderDescriptorV1,
};
use serde_json::json;

#[test]
fn raw_archive_snapshot_decodes_strict_page_state() {
    let snapshot = serde_json::from_value::<RawArchiveSnapshot>(json!({
        "kind": "records",
        "items": [{
            "awemeId": "7650000000000000001",
            "awemeType": 0,
            "remoteCreatedAt": 1786420800000_i64,
            "href": "https://www.douyin.com/video/7650000000000000001",
            "sourcePosition": 0,
            "authorName": "作者",
            "itemTitle": "标题",
            "desc": null,
            "coverUrls": ["https://example.com/cover.webp"],
            "likeCount": 123
        }],
        "remoteFolders": [{
            "remoteFolderId": "folder-a",
            "name": "收藏夹 A",
            "sourcePosition": 0
        }],
        "activeRemoteFolderId": "folder-a",
        "activeRemoteFolderName": "收藏夹 A",
        "domCardCount": 1,
        "domFolderCount": 1,
        "scrollTop": 0.0,
        "scrollHeight": 900.0,
        "clientHeight": 600.0,
        "noMoreVisible": false,
        "loadingVisible": false
    }))
    .expect("strict archive snapshot");

    assert_eq!(snapshot.items.len(), 1);
    assert_eq!(snapshot.remote_folders.len(), 1);
    assert_eq!(snapshot.dom_card_count, 1);
    assert_eq!(snapshot.dom_folder_count, 1);
    assert_eq!(snapshot.active_remote_folder_id, Some(json!("folder-a")));
}

#[test]
fn raw_archive_types_reject_unknown_fields() {
    let item = json!({
        "awemeId": "1",
        "awemeType": null,
        "remoteCreatedAt": null,
        "href": "https://www.douyin.com/video/1",
        "sourcePosition": 0,
        "authorName": null,
        "itemTitle": null,
        "desc": null,
        "coverUrls": null,
        "likeCount": null,
        "unexpected": true
    });
    assert!(serde_json::from_value::<RawArchiveItem>(item).is_err());

    let folder = json!({
        "remoteFolderId": "folder-a",
        "name": "A",
        "sourcePosition": 0,
        "unexpected": true
    });
    assert!(serde_json::from_value::<RawRemoteFolder>(folder).is_err());
}

#[test]
fn archive_scroll_and_remote_folder_snapshots_decode_strictly() {
    let wheel = serde_json::from_value::<ArchiveWheelTarget>(json!({
        "x": 12.0,
        "y": 34.0,
        "recordCount": 20,
        "scrollTop": 100.0,
        "scrollHeight": 2000.0,
        "clientHeight": 600.0,
        "noMoreVisible": false,
        "loadingVisible": true,
        "canScroll": true,
        "listFound": true
    }))
    .expect("wheel target");
    assert!(wheel.can_scroll);
    assert!(wheel.list_found);

    let folders = serde_json::from_value::<RemoteFolderPageSnapshot>(json!({
        "kind": "folders",
        "folders": [{
            "remoteFolderId": "90071992547409931",
            "name": "稍后观看",
            "sourcePosition": 0
        }],
        "domFolderCount": 1,
        "scrollTop": 0.0,
        "scrollHeight": 600.0,
        "clientHeight": 600.0,
        "noMoreVisible": false,
        "loadingVisible": false,
        "canScroll": false,
        "anchorX": 120.0,
        "anchorY": 80.0
    }))
    .expect("folder page snapshot");
    assert_eq!(folders.folders[0].remote_folder_id, json!("90071992547409931"));
    assert!(!folders.can_scroll);

    let mut unknown = serde_json::to_value(folders).unwrap();
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<RemoteFolderPageSnapshot>(unknown).is_err());
}

#[test]
fn final_archive_window_requires_field_and_scroll_stability() {
    let first = serde_json::from_value::<RawArchiveSnapshot>(json!({
        "kind": "records",
        "items": [{
            "awemeId": "7650000000000000001",
            "awemeType": 0,
            "remoteCreatedAt": 1786420800000_i64,
            "href": "https://www.douyin.com/video/7650000000000000001",
            "sourcePosition": 0,
            "authorName": "作者",
            "itemTitle": null,
            "desc": null,
            "coverUrls": null,
            "likeCount": 123
        }],
        "remoteFolders": [],
        "activeRemoteFolderId": null,
        "activeRemoteFolderName": null,
        "domCardCount": 1,
        "domFolderCount": 0,
        "scrollTop": 400.0,
        "scrollHeight": 1000.0,
        "clientHeight": 600.0,
        "noMoreVisible": true,
        "loadingVisible": false
    }))
    .unwrap();
    let mut second = first.clone();
    second.items[0].item_title = Some(json!("延迟标题"));
    assert!(!archive_snapshots_stable(&first, &second));
    assert!(!archive_bottom_confirmed(&first, &second));

    second.items[0].item_title = None;
    assert!(archive_snapshots_stable(&first, &second));
    assert!(archive_bottom_confirmed(&first, &second));

    second.scroll_top = 200.0;
    assert!(!archive_snapshots_stable(&first, &second));
    assert!(!archive_bottom_confirmed(&first, &second));

    let mut not_at_bottom = first.clone();
    not_at_bottom.scroll_top = 100.0;
    assert!(archive_snapshots_stable(&not_at_bottom, &not_at_bottom));
    assert!(!archive_bottom_confirmed(&not_at_bottom, &not_at_bottom));
}

fn raw_item(id: serde_json::Value, time: serde_json::Value) -> RawArchiveItem {
    RawArchiveItem {
        aweme_id: id,
        aweme_type: Some(json!(0)),
        remote_created_at: Some(time),
        href: "https://douyin.com/video/7650000000000000001?from=like".to_string(),
        source_position: 0,
        author_name: Some(json!("  作者  A  ")),
        item_title: Some(json!("  标题  A  ")),
        desc: Some(json!("备用描述")),
        cover_urls: Some(json!([
            "http://invalid.example/a",
            "https://example.com/a.webp"
        ])),
        like_count: Some(json!(1234)),
    }
}

#[test]
fn normalizes_archive_items_and_remote_folders() {
    let item = raw_item(json!("7650000000000000001"), json!(1_786_420_800_i64));
    let (unit, records) = normalize_archive_items(&[item], None).expect("archive records");

    assert_eq!(unit, Some(ArchiveTimeUnit::Seconds));
    assert_eq!(records[0].aweme_id, "7650000000000000001");
    assert_eq!(
        records[0].canonical_url,
        "https://www.douyin.com/video/7650000000000000001"
    );
    assert_eq!(records[0].content_type, "video");
    assert_eq!(records[0].remote_created_at_ms, Some(1_786_420_800_000));
    assert_eq!(records[0].author_name.as_deref(), Some("作者 A"));
    assert_eq!(records[0].title.as_deref(), Some("标题 A"));
    assert_eq!(
        records[0].cover_url.as_deref(),
        Some("https://example.com/a.webp")
    );
    assert_eq!(records[0].like_count, Some(1234));

    let folders = normalize_remote_folders(&[
        RawRemoteFolder {
            remote_folder_id: json!("folder-a"),
            name: json!("  收藏夹  A  "),
            source_position: 0,
        },
        RawRemoteFolder {
            remote_folder_id: json!("folder-b"),
            name: json!("收藏夹 B"),
            source_position: 1,
        },
    ])
    .expect("remote folders");
    assert_eq!(folders[0].remote_folder_id, "folder-a");
    assert_eq!(folders[0].name, "收藏夹 A");
}

#[test]
fn article_archive_items_preserve_their_url_as_unknown_content() {
    let mut item = raw_item(json!("7659165382892902650"), json!(1_786_420_800_i64));
    item.aweme_type = Some(json!(163));
    item.href = "https://www.douyin.com/article/7659165382892902650".to_string();

    let (_, records) = normalize_archive_items(&[item], None).expect("article archive record");

    assert_eq!(records[0].aweme_id, "7659165382892902650");
    assert_eq!(
        records[0].canonical_url,
        "https://www.douyin.com/article/7659165382892902650"
    );
    assert_eq!(records[0].content_type, "unknown");
    compute_archive_hashes(&records).expect("article archive hashes");
}

#[test]
fn archive_normalization_rejects_unsafe_ids_mixed_units_and_url_conflicts() {
    let unsafe_numeric = raw_item(
        json!(7_650_000_000_000_000_001_u64),
        json!(1_786_420_800_i64),
    );
    assert_eq!(
        normalize_archive_items(&[unsafe_numeric], None).unwrap_err(),
        "invalid_archive_aweme_id"
    );

    let mut seconds = raw_item(json!("7650000000000000001"), json!(1_786_420_800_i64));
    let mut milliseconds = raw_item(json!("7650000000000000002"), json!(1_786_420_799_000_i64));
    milliseconds.source_position = 1;
    milliseconds.href = "https://www.douyin.com/video/7650000000000000002".to_string();
    assert_eq!(
        normalize_archive_items(&[seconds.clone(), milliseconds], None).unwrap_err(),
        "mixed_archive_time_unit"
    );

    seconds.href = "https://www.douyin.com/video/7650000000000000999".to_string();
    assert_eq!(
        normalize_archive_items(&[seconds], None).unwrap_err(),
        "archive_url_id_mismatch"
    );
}

#[test]
fn archive_time_unit_can_be_absent_then_frozen() {
    let mut without_time = raw_item(json!("7650000000000000001"), serde_json::Value::Null);
    without_time.remote_created_at = None;
    let (unit, records) = normalize_archive_items(&[without_time], None).unwrap();
    assert_eq!(unit, None);
    assert_eq!(records[0].remote_created_at_ms, None);

    let with_time = raw_item(json!("7650000000000000001"), json!(1_786_420_800_i64));
    let (unit, _) = normalize_archive_items(&[with_time], unit).unwrap();
    assert_eq!(unit, Some(ArchiveTimeUnit::Seconds));
}

#[test]
fn archive_growth_preserves_the_loaded_identity_prefix() {
    let previous = vec![archive_record(0, None), archive_record(1, None)];
    let mut current = previous.clone();
    current[0].title = Some("延迟补齐的标题".to_string());
    current.push(archive_record(2, None));
    validate_archive_growth(&previous, &current).expect("field enrichment keeps identity prefix");

    current[1].aweme_id = "7659999999999999999".to_string();
    assert_eq!(
        validate_archive_growth(&previous, &current).unwrap_err(),
        "archive_snapshot_prefix_changed"
    );
}

#[test]
fn archive_field_enrichment_counts_as_collection_progress() {
    let previous = vec![archive_record(0, None)];
    let mut enriched = previous.clone();
    enriched[0].title = Some("延迟补齐的标题".to_string());
    assert!(archive_snapshot_made_progress(&previous, &enriched));
    assert!(!archive_snapshot_made_progress(&enriched, &enriched));
}

#[test]
fn known_boundary_counts_only_the_consecutive_loaded_tail() {
    let records = (0..6)
        .map(|index| archive_record(index, None))
        .collect::<Vec<_>>();
    let known = [
        records[0].aweme_id.clone(),
        records[3].aweme_id.clone(),
        records[4].aweme_id.clone(),
        records[5].aweme_id.clone(),
    ]
    .into_iter()
    .collect();

    assert_eq!(consecutive_known_tail(&records, &known), 3);
}

#[test]
fn active_remote_folder_identity_is_strict_and_matches_the_request() {
    assert_eq!(
        normalize_active_remote_folder(
            Some(&json!("folder-a")),
            Some(&json!(" 收藏夹 A ")),
            "folder-a"
        )
        .unwrap(),
        ("folder-a".to_string(), "收藏夹 A".to_string())
    );
    assert_eq!(
        normalize_active_remote_folder(
            Some(&json!("folder-b")),
            Some(&json!("收藏夹 B")),
            "folder-a"
        )
        .unwrap_err(),
        "remote_folder_not_selected"
    );
}

fn archive_record(index: usize, remote_created_at_ms: Option<i64>) -> ArchiveItemV1 {
    let aweme_id = format!("765{index:016}");
    ArchiveItemV1 {
        canonical_url: format!("https://www.douyin.com/video/{aweme_id}"),
        aweme_id,
        content_type: "video".to_string(),
        source_position: index as i64,
        remote_created_at_ms,
        title: Some(format!("标题 {index}")),
        author_name: Some(format!("作者 {index}")),
        cover_url: Some(format!("https://example.com/{index}.webp")),
        like_count: Some(index as i64),
    }
}

fn stop_input(range: ArchiveListRangeV1) -> ArchiveStopInput {
    ArchiveStopInput {
        range,
        run_mode: ArchiveRunModeV1::Incremental,
        cutoff_ms: None,
        bottom_confirmed: false,
        terminal_without_progress: false,
        consecutive_known: 0,
    }
}

#[test]
fn requested_counts_take_priority_over_bottom() {
    let records = (0..3).map(|index| archive_record(index, None)).collect();
    let mut input = stop_input(ArchiveListRangeV1::RecentItems { count: 3 });
    input.bottom_confirmed = true;
    let ArchiveStopDecision::Finished(result) = decide_archive_stop(records, input).unwrap() else {
        panic!("finished");
    };
    assert_eq!(
        result.completion,
        ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::RequestedItemCount { count: 3 }
        }
    );

    let records = (0..2).map(|index| archive_record(index, None)).collect();
    let mut input = stop_input(ArchiveListRangeV1::RecordLimit { limit: 3 });
    input.bottom_confirmed = true;
    let ArchiveStopDecision::Finished(result) = decide_archive_stop(records, input).unwrap() else {
        panic!("finished");
    };
    assert_eq!(
        result.completion,
        ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::Bottom
        }
    );
}

#[test]
fn stable_known_boundary_is_only_for_incremental_all() {
    let records: Vec<ArchiveItemV1> = (0..25).map(|index| archive_record(index, None)).collect();
    let mut input = stop_input(ArchiveListRangeV1::All);
    input.consecutive_known = 20;
    let ArchiveStopDecision::Finished(result) =
        decide_archive_stop(records.clone(), input).unwrap()
    else {
        panic!("known boundary");
    };
    assert_eq!(
        result.completion,
        ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::StableKnownBoundary {
                consecutive_known: 20
            }
        }
    );

    let mut full = stop_input(ArchiveListRangeV1::All);
    full.run_mode = ArchiveRunModeV1::FullReconcile;
    full.consecutive_known = 25;
    assert!(matches!(
        decide_archive_stop(records, full).unwrap(),
        ArchiveStopDecision::Continue
    ));
}

#[test]
fn confirmed_bottom_is_stronger_than_stable_known_boundary() {
    let records = (0..25).map(|index| archive_record(index, None)).collect();
    let mut input = stop_input(ArchiveListRangeV1::All);
    input.bottom_confirmed = true;
    input.consecutive_known = 20;

    let ArchiveStopDecision::Finished(result) = decide_archive_stop(records, input).unwrap() else {
        panic!("bottom");
    };
    assert_eq!(
        result.completion,
        ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::Bottom
        }
    );
}

#[test]
fn recent_days_uses_guard_without_storing_it() {
    let cutoff = 1_786_000_000_000_i64;
    let mut records = vec![
        archive_record(0, Some(cutoff + 2_000)),
        archive_record(1, Some(cutoff + 1_000)),
        archive_record(2, Some(cutoff - 1)),
    ];
    let guard_id = records[2].aweme_id.clone();
    let mut input = stop_input(ArchiveListRangeV1::RecentDays { days: 7 });
    input.cutoff_ms = Some(cutoff);
    let ArchiveStopDecision::Finished(result) =
        decide_archive_stop(std::mem::take(&mut records), input).unwrap()
    else {
        panic!("date boundary");
    };
    assert_eq!(result.records.len(), 2);
    assert_eq!(
        result.completion,
        ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::DateBoundary {
                cutoff_ms: cutoff,
                guard_aweme_id: guard_id,
                guard_remote_created_at_ms: cutoff - 1,
            }
        }
    );

    let mut missing = vec![archive_record(0, None)];
    let mut input = stop_input(ArchiveListRangeV1::RecentDays { days: 7 });
    input.cutoff_ms = Some(cutoff);
    assert_eq!(
        decide_archive_stop(std::mem::take(&mut missing), input).unwrap_err(),
        "archive_time_range_unavailable"
    );
}

#[test]
fn safety_cap_preserves_bottom_and_date_boundary_priority() {
    let cutoff = 1_786_000_000_000_i64;
    let records = (0..50_000)
        .map(|index| archive_record(index, Some(cutoff + 1_000)))
        .collect::<Vec<_>>();
    let mut all_bottom = stop_input(ArchiveListRangeV1::All);
    all_bottom.bottom_confirmed = true;
    let ArchiveStopDecision::Finished(result) =
        decide_archive_stop(records.clone(), all_bottom).unwrap()
    else {
        panic!("bottom");
    };
    assert!(matches!(
        result.completion,
        ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::Bottom
        }
    ));

    let mut all_capped = stop_input(ArchiveListRangeV1::All);
    all_capped.terminal_without_progress = true;
    let ArchiveStopDecision::Finished(result) =
        decide_archive_stop(records.clone(), all_capped).unwrap()
    else {
        panic!("cap");
    };
    assert!(matches!(
        result.completion,
        ArchiveCompletionV1::Incomplete {
            reason: ArchiveIncompleteReasonV1::SafetyRecordCap { cap: 50_000 }
        }
    ));

    let mut with_guard = records;
    with_guard.push(archive_record(50_000, Some(cutoff - 1)));
    let mut recent = stop_input(ArchiveListRangeV1::RecentDays { days: 7 });
    recent.cutoff_ms = Some(cutoff);
    let ArchiveStopDecision::Finished(result) = decide_archive_stop(with_guard, recent).unwrap()
    else {
        panic!("guard");
    };
    assert_eq!(result.records.len(), 50_000);
    assert!(matches!(
        result.completion,
        ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::DateBoundary { .. }
        }
    ));

    let mut guard_still_in_range = (0..50_001)
        .map(|index| archive_record(index, Some(cutoff + 1_000)))
        .collect::<Vec<_>>();
    guard_still_in_range.push(archive_record(50_001, Some(cutoff - 1)));
    let mut recent = stop_input(ArchiveListRangeV1::RecentDays { days: 7 });
    recent.cutoff_ms = Some(cutoff);
    let ArchiveStopDecision::Finished(result) =
        decide_archive_stop(guard_still_in_range, recent).unwrap()
    else {
        panic!("cap after in-range guard");
    };
    assert_eq!(result.records.len(), 50_000);
    assert!(matches!(
        result.completion,
        ArchiveCompletionV1::Incomplete {
            reason: ArchiveIncompleteReasonV1::SafetyRecordCap { cap: 50_000 }
        }
    ));

    let mut missing_guard = (0..50_000)
        .map(|index| archive_record(index, Some(cutoff + 1_000)))
        .collect::<Vec<_>>();
    missing_guard.push(archive_record(50_000, None));
    let mut recent = stop_input(ArchiveListRangeV1::RecentDays { days: 7 });
    recent.cutoff_ms = Some(cutoff);
    let ArchiveStopDecision::Finished(result) = decide_archive_stop(missing_guard, recent).unwrap()
    else {
        panic!("unreliable guard");
    };
    assert!(matches!(
        result.completion,
        ArchiveCompletionV1::Incomplete {
            reason: ArchiveIncompleteReasonV1::SafetyRecordCap { cap: 50_000 }
        }
    ));

    let over_cap_bottom = (0..50_001)
        .map(|index| archive_record(index, Some(cutoff + 1_000)))
        .collect::<Vec<_>>();
    let mut recent = stop_input(ArchiveListRangeV1::RecentDays { days: 7 });
    recent.cutoff_ms = Some(cutoff);
    recent.bottom_confirmed = true;
    let ArchiveStopDecision::Finished(result) =
        decide_archive_stop(over_cap_bottom, recent).unwrap()
    else {
        panic!("bottom beyond cap");
    };
    assert!(matches!(
        result.completion,
        ArchiveCompletionV1::Incomplete {
            reason: ArchiveIncompleteReasonV1::SafetyRecordCap { cap: 50_000 }
        }
    ));
}

#[test]
fn unconfirmed_end_and_remote_folder_discovery_have_stable_states() {
    let records = vec![archive_record(0, None)];
    let mut input = stop_input(ArchiveListRangeV1::All);
    input.terminal_without_progress = true;
    let ArchiveStopDecision::Finished(result) = decide_archive_stop(records, input).unwrap() else {
        panic!("unconfirmed");
    };
    assert!(matches!(
        result.completion,
        ArchiveCompletionV1::Incomplete {
            reason: ArchiveIncompleteReasonV1::UnconfirmedEnd
        }
    ));

    let folders = vec![RemoteFolderDescriptorV1 {
        remote_folder_id: "folder-a".to_string(),
        name: "收藏夹 A".to_string(),
        source_position: 0,
    }];
    let RemoteFolderDiscoveryDecision::Finished(complete) =
        decide_remote_folder_discovery(folders.clone(), true, false).unwrap()
    else {
        panic!("complete discovery");
    };
    assert!(complete.complete_scan);
    assert!(complete.bottom_confirmed);
    assert!(!complete.hit_record_limit);

    let RemoteFolderDiscoveryDecision::Finished(incomplete) =
        decide_remote_folder_discovery(folders, false, true).unwrap()
    else {
        panic!("incomplete discovery");
    };
    assert!(!incomplete.complete_scan);
    assert!(!incomplete.bottom_confirmed);
    assert!(!incomplete.hit_record_limit);
}
