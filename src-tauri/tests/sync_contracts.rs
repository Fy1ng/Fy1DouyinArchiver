use douyin_history_archive::{
    device_settings::BrowserPreferenceV1,
    settings_store::{ArchiveListRangeV1, HistoryRangeV1, RemoteFolderScheduleModeV1},
    sync::{
        automatic_config_hash, canonical_automatic_config, canonical_equivalence,
        compute_archive_hashes, compute_remote_folder_hashes, freeze_task_payload,
        job_identity_sha256, validate_archive_collection, validate_enqueue_request,
        validate_execution_envelope, validate_remote_folder_discovery, ArchiveCollectionSuccessV1,
        ArchiveCompleteStopV1, ArchiveCompletionV1, ArchiveIncompleteReasonV1, ArchiveItemV1,
        ArchiveRunModeV1, AutomaticScheduleConfigV1, AutomaticSyncConfigV1,
        BrowserExecutionIdentityV1, BrowserIdV1, CapabilityRegistry, EffectiveRangeEquivalenceV1,
        EnqueueSyncRequestV1, ExecutorRegistration, ExecutorRegistry, FakeSyncBackend,
        ImportCountsV1, QueuedSyncContext, RangeFreezeV1, RemoteFolderDescriptorV1,
        RemoteFolderDiscoverySuccessV1, RemoteFolderExecutionOverrideV1,
        RemoteFolderRangeEquivalenceV1, SourceExecutionResultV1, SyncBatchEntryV1,
        SyncExecutionEnvelopeV1, SyncExecutionEquivalenceV1, SyncJobIssueV1, SyncJobSnapshotV1,
        SyncJobStatusV1, SyncJobViewV1, SyncScopeV1, SyncSourceV1, SyncTaskPayloadV1,
        SyncTriggerV1, TaskKeyV1,
    },
    CollectorRange, CollectorSuccessV3,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

const FROZEN_AT_MS: i64 = 1_786_420_800_000;
const SEVEN_DAY_CUTOFF_MS: i64 = 1_785_859_200_000;
const EMPTY_HASH: &str = "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945";

fn browser(
    preference: BrowserPreferenceV1,
    resolved_browser: BrowserIdV1,
) -> BrowserExecutionIdentityV1 {
    BrowserExecutionIdentityV1 {
        preference,
        resolved_browser,
        profile_key: "0123456789abcdef0123456789abcdef".to_string(),
        profile_generation: "fedcba9876543210fedcba9876543210".to_string(),
        executable_path_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_string(),
    }
}

fn history_payload() -> SyncTaskPayloadV1 {
    SyncTaskPayloadV1::History {
        range: HistoryRangeV1::Days { days: 7 },
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: FROZEN_AT_MS,
            cutoff_ms: Some(SEVEN_DAY_CUTOFF_MS),
        },
        scroll_speed: 900,
        hide_browser: true,
        browser: browser(BrowserPreferenceV1::Auto, BrowserIdV1::Edge),
    }
}

fn archive_item() -> ArchiveItemV1 {
    ArchiveItemV1 {
        aweme_id: "123".to_string(),
        canonical_url: "https://www.douyin.com/video/123".to_string(),
        content_type: "video".to_string(),
        source_position: 0,
        remote_created_at_ms: Some(1_786_377_600_000),
        title: Some("Title".to_string()),
        author_name: None,
        cover_url: None,
        like_count: Some(42),
    }
}

fn archive_success() -> ArchiveCollectionSuccessV1 {
    ArchiveCollectionSuccessV1 {
        records: vec![archive_item()],
        identity_hash: "d448812270be9bf4c3f71af7a4a22716a3b4558e4df4d49d8dd88bf585b34e3d"
            .to_string(),
        field_hash: "ac0b7e6422be8bf2657867f53e32cd6e78bf402ee02c59b93a90ff26937f24f1".to_string(),
        completion: ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::Bottom,
        },
    }
}

fn empty_history_result(account_id: i64, job_id: i64) -> CollectorSuccessV3 {
    CollectorSuccessV3 {
        schema_version: 3,
        account_id,
        sync_job_id: job_id,
        sync_started_at_ms: FROZEN_AT_MS,
        calendar_time_zone: "Asia/Shanghai".to_string(),
        range: CollectorRange::Days {
            days: 7,
            cutoff_ms: SEVEN_DAY_CUTOFF_MS,
        },
        detected_history_time_unit: "milliseconds".to_string(),
        identity_hash: EMPTY_HASH.to_string(),
        field_hash: EMPTY_HASH.to_string(),
        records: vec![],
    }
}

#[test]
fn source_json_and_sqlite_mappings_are_exact() {
    for (source, json_name, sqlite_name) in [
        (SyncSourceV1::History, "history", "history"),
        (SyncSourceV1::Likes, "likes", "likes"),
        (SyncSourceV1::Favorites, "favorites", "favorites"),
        (SyncSourceV1::RemoteFolder, "remoteFolder", "remote_folder"),
    ] {
        assert_eq!(
            serde_json::to_string(&source).unwrap(),
            format!("\"{json_name}\"")
        );
        assert_eq!(source.sqlite_name(), sqlite_name);
        assert_eq!(SyncSourceV1::from_sqlite(sqlite_name).unwrap(), source);
    }
    assert_eq!(
        SyncSourceV1::from_sqlite("remoteFolder").unwrap_err(),
        "invalid_sync_source"
    );
}

#[test]
fn history_payload_and_equivalence_have_stable_canonical_hashes() {
    let frozen = freeze_task_payload(history_payload()).unwrap();
    assert_eq!(
        frozen.payload_json,
        r#"{"source":"history","range":{"kind":"days","days":7},"rangeFreeze":{"frozenAtMs":1786420800000,"cutoffMs":1785859200000},"scrollSpeed":900,"hideBrowser":true,"browser":{"preference":"auto","resolvedBrowser":"edge","profileKey":"0123456789abcdef0123456789abcdef","profileGeneration":"fedcba9876543210fedcba9876543210","executablePathSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#
    );
    assert_eq!(
        frozen.payload_sha256,
        "7e755c955f103c6291a370a08325ca3f4b6aa7853088a0d0d10693d9bbcc001c"
    );
    assert_eq!(
        frozen.equivalence_json,
        r#"{"source":"history","schemaVersion":1,"sourceInstanceKey":"","range":{"kind":"days","days":7,"count":null,"limit":null,"cutoffMs":1785859200000},"scrollSpeed":900,"hideBrowser":true,"browser":{"preference":"auto","resolvedBrowser":"edge","profileKey":"0123456789abcdef0123456789abcdef","profileGeneration":"fedcba9876543210fedcba9876543210","executablePathSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#
    );
    assert_eq!(
        frozen.equivalence_sha256,
        "9dd08581554dde88446dec9a49bb5ab4f3ce8f419ebfe0573041ac5a5a7b68c5"
    );
}

#[test]
fn every_range_tag_and_remote_folder_payload_round_trip_exactly() {
    for (range, expected) in [
        (
            HistoryRangeV1::Days { days: 7 },
            r#"{"kind":"days","days":7}"#,
        ),
        (HistoryRangeV1::Bottom, r#"{"kind":"bottom"}"#),
    ] {
        assert_eq!(serde_json::to_string(&range).unwrap(), expected);
        assert_eq!(
            serde_json::from_str::<HistoryRangeV1>(expected).unwrap(),
            range
        );
    }

    for (range, expected) in [
        (ArchiveListRangeV1::All, r#"{"kind":"all"}"#),
        (
            ArchiveListRangeV1::RecentItems { count: 500 },
            r#"{"kind":"recentItems","count":500}"#,
        ),
        (
            ArchiveListRangeV1::RecordLimit { limit: 200 },
            r#"{"kind":"recordLimit","limit":200}"#,
        ),
        (
            ArchiveListRangeV1::RecentDays { days: 3 },
            r#"{"kind":"recentDays","days":3}"#,
        ),
    ] {
        assert_eq!(serde_json::to_string(&range).unwrap(), expected);
        assert_eq!(
            serde_json::from_str::<ArchiveListRangeV1>(expected).unwrap(),
            range
        );
    }

    let payload = SyncTaskPayloadV1::RemoteFolder {
        remote_folder_id: "folder-a".to_string(),
        range: ArchiveListRangeV1::RecordLimit { limit: 200 },
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: FROZEN_AT_MS,
            cutoff_ms: None,
        },
        run_mode: ArchiveRunModeV1::Incremental,
        scroll_speed: 900,
        hide_browser: true,
        browser: browser(BrowserPreferenceV1::Auto, BrowserIdV1::Edge),
    };
    let frozen = freeze_task_payload(payload.clone()).unwrap();
    assert_eq!(
        serde_json::from_str::<SyncTaskPayloadV1>(&frozen.payload_json).unwrap(),
        payload
    );
    match frozen.equivalence {
        SyncExecutionEquivalenceV1::RemoteFolder {
            source_instance_key,
            ..
        } => assert_eq!(source_instance_key, "folder-a"),
        _ => panic!("expected remote-folder equivalence"),
    }
}

#[test]
fn automatic_config_and_sorted_favorite_overrides_have_golden_hashes() {
    let likes = AutomaticSyncConfigV1::Likes {
        schema_version: 1,
        source_instance_key: String::new(),
        schedule: AutomaticScheduleConfigV1 {
            enabled: true,
            interval_minutes: 360,
        },
        range: ArchiveListRangeV1::RecentItems { count: 500 },
        run_mode: ArchiveRunModeV1::Incremental,
        scroll_speed: 900,
        hide_browser: false,
        browser_preference: BrowserPreferenceV1::Chrome,
    };
    assert_eq!(
        canonical_automatic_config(&likes).unwrap(),
        r#"{"source":"likes","schemaVersion":1,"sourceInstanceKey":"","schedule":{"enabled":true,"intervalMinutes":360},"range":{"kind":"recentItems","count":500},"runMode":"incremental","scrollSpeed":900,"hideBrowser":false,"browserPreference":"chrome"}"#
    );
    assert_eq!(
        automatic_config_hash(&likes).unwrap(),
        "dd69135150ef6a6d39e32b97a79e1d8ae073f92e58e4f60fcb8fa1a070644013"
    );

    let mut overrides = BTreeMap::new();
    overrides.insert(
        "folder-b".to_string(),
        RemoteFolderExecutionOverrideV1 {
            range: ArchiveListRangeV1::RecordLimit { limit: 200 },
        },
    );
    overrides.insert(
        "folder-a".to_string(),
        RemoteFolderExecutionOverrideV1 {
            range: ArchiveListRangeV1::RecentDays { days: 3 },
        },
    );
    let payload = SyncTaskPayloadV1::Favorites {
        range: ArchiveListRangeV1::RecentItems { count: 25 },
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: FROZEN_AT_MS,
            cutoff_ms: None,
        },
        run_mode: ArchiveRunModeV1::Incremental,
        scroll_speed: 1200,
        hide_browser: true,
        browser: browser(BrowserPreferenceV1::Edge, BrowserIdV1::Edge),
        discover_remote_folders: true,
        remote_folder_mode: RemoteFolderScheduleModeV1::PerFolder,
        remote_folder_default_range: ArchiveListRangeV1::All,
        remote_folder_overrides: overrides,
    };
    let frozen = freeze_task_payload(payload).unwrap();
    assert_eq!(
        frozen.equivalence_json,
        r#"{"source":"favorites","schemaVersion":1,"sourceInstanceKey":"","range":{"kind":"recentItems","days":null,"count":25,"limit":null,"cutoffMs":null},"runMode":"incremental","scrollSpeed":1200,"hideBrowser":true,"browser":{"preference":"edge","resolvedBrowser":"edge","profileKey":"0123456789abcdef0123456789abcdef","profileGeneration":"fedcba9876543210fedcba9876543210","executablePathSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"discoverRemoteFolders":true,"remoteFolderMode":"perFolder","remoteFolderDefaultRange":{"kind":"all","days":null,"count":null,"limit":null,"cutoffMs":null},"remoteFolderOverrides":[{"remoteFolderId":"folder-a","range":{"kind":"recentDays","days":3,"count":null,"limit":null,"cutoffMs":1786204800000}},{"remoteFolderId":"folder-b","range":{"kind":"recordLimit","days":null,"count":null,"limit":200,"cutoffMs":null}}]}"#
    );
    assert_eq!(
        frozen.equivalence_sha256,
        "75da562579e786da272dd781aaf9a5c3bc171264869e6620a422d3b4f695f19a"
    );

    let remote = AutomaticSyncConfigV1::RemoteFolder {
        schema_version: 1,
        source_instance_key: "folder-a".to_string(),
        schedule: AutomaticScheduleConfigV1 {
            enabled: false,
            interval_minutes: 720,
        },
        range: ArchiveListRangeV1::RecordLimit { limit: 200 },
        run_mode: ArchiveRunModeV1::Incremental,
        remote_folder_mode: RemoteFolderScheduleModeV1::PerFolder,
        scroll_speed: 1500,
        hide_browser: true,
        browser_preference: BrowserPreferenceV1::Auto,
    };
    assert_eq!(
        automatic_config_hash(&remote).unwrap(),
        "4c46e3b2c9cf86918e5b161cb65361fcd56b04357fe85df570d6862302a3c5ea"
    );
}

#[test]
fn illegal_ranges_instances_hashes_and_override_order_are_rejected() {
    let invalid_range = EffectiveRangeEquivalenceV1 {
        kind: douyin_history_archive::sync::SyncRangeKindV1::Days,
        days: Some(7),
        count: Some(1),
        limit: None,
        cutoff_ms: Some(SEVEN_DAY_CUTOFF_MS),
    };
    let invalid = SyncExecutionEquivalenceV1::History {
        schema_version: 1,
        source_instance_key: String::new(),
        range: invalid_range,
        scroll_speed: 900,
        hide_browser: false,
        browser: browser(BrowserPreferenceV1::Auto, BrowserIdV1::Edge),
    };
    assert_eq!(
        canonical_equivalence(&invalid).unwrap_err(),
        "invalid_effective_range"
    );

    let mut missing_nulls = serde_json::to_value(SyncExecutionEquivalenceV1::History {
        schema_version: 1,
        source_instance_key: String::new(),
        range: EffectiveRangeEquivalenceV1 {
            kind: douyin_history_archive::sync::SyncRangeKindV1::Bottom,
            days: None,
            count: None,
            limit: None,
            cutoff_ms: None,
        },
        scroll_speed: 900,
        hide_browser: false,
        browser: browser(BrowserPreferenceV1::Auto, BrowserIdV1::Edge),
    })
    .unwrap();
    missing_nulls["range"]
        .as_object_mut()
        .unwrap()
        .remove("days");
    assert!(serde_json::from_value::<SyncExecutionEquivalenceV1>(missing_nulls).is_err());

    let unsorted = SyncExecutionEquivalenceV1::Favorites {
        schema_version: 1,
        source_instance_key: String::new(),
        range: EffectiveRangeEquivalenceV1::all(),
        run_mode: ArchiveRunModeV1::Incremental,
        scroll_speed: 900,
        hide_browser: false,
        browser: browser(BrowserPreferenceV1::Auto, BrowserIdV1::Edge),
        discover_remote_folders: true,
        remote_folder_mode: RemoteFolderScheduleModeV1::PerFolder,
        remote_folder_default_range: EffectiveRangeEquivalenceV1::all(),
        remote_folder_overrides: vec![
            RemoteFolderRangeEquivalenceV1 {
                remote_folder_id: "folder-b".to_string(),
                range: EffectiveRangeEquivalenceV1::all(),
            },
            RemoteFolderRangeEquivalenceV1 {
                remote_folder_id: "folder-a".to_string(),
                range: EffectiveRangeEquivalenceV1::all(),
            },
        ],
    };
    assert_eq!(
        canonical_equivalence(&unsorted).unwrap_err(),
        "invalid_remote_folder_overrides"
    );

    let mut cutoff_mismatch = history_payload();
    if let SyncTaskPayloadV1::History { range_freeze, .. } = &mut cutoff_mismatch {
        range_freeze.cutoff_ms = Some(SEVEN_DAY_CUTOFF_MS + 1);
    }
    assert_eq!(
        freeze_task_payload(cutoff_mismatch).unwrap_err(),
        "range_cutoff_mismatch"
    );

    let mut unsafe_browser = history_payload();
    if let SyncTaskPayloadV1::History { browser, .. } = &mut unsafe_browser {
        browser.executable_path_sha256 = "BAD".to_string();
    }
    assert_eq!(
        freeze_task_payload(unsafe_browser).unwrap_err(),
        "invalid_browser_identity"
    );

    let invalid_automatic = AutomaticSyncConfigV1::Favorites {
        schema_version: 1,
        source_instance_key: "not-empty".to_string(),
        schedule: AutomaticScheduleConfigV1 {
            enabled: true,
            interval_minutes: 360,
        },
        range: ArchiveListRangeV1::All,
        run_mode: ArchiveRunModeV1::FullReconcile,
        scroll_speed: 900,
        hide_browser: false,
        browser_preference: BrowserPreferenceV1::Auto,
        discover_remote_folders: false,
    };
    assert_eq!(
        automatic_config_hash(&invalid_automatic).unwrap_err(),
        "invalid_automatic_config"
    );
}

#[test]
fn enqueue_request_requires_exact_source_instance_and_full_reconcile_confirmation() {
    let payload = SyncTaskPayloadV1::Likes {
        range: ArchiveListRangeV1::All,
        range_freeze: RangeFreezeV1 {
            frozen_at_ms: FROZEN_AT_MS,
            cutoff_ms: None,
        },
        run_mode: ArchiveRunModeV1::FullReconcile,
        scroll_speed: 900,
        hide_browser: false,
        browser: browser(BrowserPreferenceV1::Auto, BrowserIdV1::Edge),
    };
    let payload_hash = freeze_task_payload(payload.clone()).unwrap().payload_sha256;
    let valid = EnqueueSyncRequestV1 {
        scope: SyncScopeV1::One,
        account_id: Some(1),
        source: Some(SyncSourceV1::Likes),
        source_instance_key: Some(String::new()),
        payload: Some(payload.clone()),
        full_reconcile_confirmation_sha256: Some(payload_hash.clone()),
    };
    validate_enqueue_request(&valid).unwrap();

    for invalid in [
        EnqueueSyncRequestV1 {
            full_reconcile_confirmation_sha256: None,
            ..valid.clone()
        },
        EnqueueSyncRequestV1 {
            source: Some(SyncSourceV1::Favorites),
            ..valid.clone()
        },
        EnqueueSyncRequestV1 {
            scope: SyncScopeV1::CurrentAccountAvailableSources,
            ..valid.clone()
        },
    ] {
        assert!(validate_enqueue_request(&invalid).is_err());
    }

    let non_all = match payload {
        SyncTaskPayloadV1::Likes {
            range_freeze,
            run_mode,
            scroll_speed,
            hide_browser,
            browser,
            ..
        } => SyncTaskPayloadV1::Likes {
            range: ArchiveListRangeV1::RecentItems { count: 1 },
            range_freeze,
            run_mode,
            scroll_speed,
            hide_browser,
            browser,
        },
        _ => unreachable!(),
    };
    let invalid = EnqueueSyncRequestV1 {
        payload: Some(non_all),
        full_reconcile_confirmation_sha256: Some(payload_hash),
        ..valid
    };
    assert_eq!(
        validate_enqueue_request(&invalid).unwrap_err(),
        "full_reconcile_requires_all"
    );
}

#[test]
fn strict_serde_rejects_unknown_fields_at_every_contract_layer() {
    let payload = serde_json::to_value(history_payload()).unwrap();
    let mutations: Vec<Box<dyn Fn(&mut Value)>> = vec![
        Box::new(|value| value["extra"] = json!(true)),
        Box::new(|value| value["range"]["extra"] = json!(true)),
        Box::new(|value| value["rangeFreeze"]["extra"] = json!(true)),
        Box::new(|value| value["browser"]["extra"] = json!(true)),
    ];
    for mutate in mutations {
        let mut value = payload.clone();
        mutate(&mut value);
        assert!(serde_json::from_value::<SyncTaskPayloadV1>(value).is_err());
    }

    let issue = SyncJobIssueV1 {
        phase: "collect".to_string(),
        code: "warning".to_string(),
        severity: douyin_history_archive::sync::SyncJobIssueSeverityV1::Warning,
        message: None,
    };
    let job = SyncJobViewV1 {
        id: 1,
        account_id: 1,
        source: SyncSourceV1::History,
        source_instance_key: String::new(),
        trigger: SyncTriggerV1::Manual,
        status: SyncJobStatusV1::Queued,
        queued_at: "2026-08-11T00:00:00.000Z".to_string(),
        scheduled_for: None,
        started_at: None,
        finished_at: None,
        queue_position: Some(1),
        can_cancel: true,
        issues: vec![issue],
    };
    let entry = SyncBatchEntryV1::Job(job);
    let mut value = serde_json::to_value(entry).unwrap();
    value["data"]["issues"][0]["extra"] = json!(true);
    assert!(serde_json::from_value::<SyncBatchEntryV1>(value).is_err());

    let mut record = serde_json::to_value(archive_item()).unwrap();
    record["extra"] = json!(true);
    assert!(serde_json::from_value::<ArchiveItemV1>(record).is_err());

    let frozen = freeze_task_payload(history_payload()).unwrap();
    let mut envelope = serde_json::to_value(SyncExecutionEnvelopeV1 {
        schema_version: 1,
        account_id: 7,
        job_id: 9,
        source: SyncSourceV1::History,
        source_instance_key: String::new(),
        task_payload_sha256: frozen.payload_sha256,
        result: SourceExecutionResultV1::History(empty_history_result(7, 9)),
    })
    .unwrap();
    envelope["extra"] = json!(true);
    assert!(serde_json::from_value::<SyncExecutionEnvelopeV1>(envelope).is_err());

    let mut result =
        serde_json::to_value(SourceExecutionResultV1::Likes(archive_success())).unwrap();
    result["data"]["extra"] = json!(true);
    assert!(serde_json::from_value::<SourceExecutionResultV1>(result).is_err());
}

#[test]
fn archive_and_folder_contracts_freeze_hashes_positions_urls_and_completion_tags() {
    let item = archive_item();
    assert_eq!(
        compute_archive_hashes(&[item.clone()]).unwrap(),
        (
            "d448812270be9bf4c3f71af7a4a22716a3b4558e4df4d49d8dd88bf585b34e3d".to_string(),
            "ac0b7e6422be8bf2657867f53e32cd6e78bf402ee02c59b93a90ff26937f24f1".to_string(),
        )
    );
    validate_archive_collection(&archive_success()).unwrap();
    assert_eq!(
        serde_json::to_string(&ArchiveCompletionV1::Complete {
            stop: ArchiveCompleteStopV1::DateBoundary {
                cutoff_ms: SEVEN_DAY_CUTOFF_MS,
                guard_aweme_id: "999".to_string(),
                guard_remote_created_at_ms: SEVEN_DAY_CUTOFF_MS - 1,
            },
        })
        .unwrap(),
        r#"{"status":"complete","data":{"stop":{"kind":"dateBoundary","data":{"cutoffMs":1785859200000,"guardAwemeId":"999","guardRemoteCreatedAtMs":1785859199999}}}}"#
    );
    assert_eq!(
        serde_json::to_string(&ArchiveCompletionV1::Incomplete {
            reason: ArchiveIncompleteReasonV1::SafetyRecordCap { cap: 50_000 },
        })
        .unwrap(),
        r#"{"status":"incomplete","data":{"reason":{"kind":"safetyRecordCap","data":{"cap":50000}}}}"#
    );

    let mut bad_position = archive_success();
    bad_position.records[0].source_position = 1;
    assert_eq!(
        validate_archive_collection(&bad_position).unwrap_err(),
        "invalid_source_position"
    );
    let mut bad_url = archive_success();
    bad_url.records[0].canonical_url = "https://www.douyin.com/video/999".to_string();
    assert_eq!(
        validate_archive_collection(&bad_url).unwrap_err(),
        "invalid_archive_url"
    );

    let folders = vec![RemoteFolderDescriptorV1 {
        remote_folder_id: "folder-a".to_string(),
        name: "Folder A".to_string(),
        source_position: 0,
    }];
    assert_eq!(
        compute_remote_folder_hashes(&folders).unwrap(),
        (
            "28045004ce0bd95e5f70124d0616709eb6b4e0ebdca6069ac19e66abe589c2f9".to_string(),
            "80a9466fe3e3f32a7c79db83a3089cbce9a42c8698cf898540f7600beb666f7e".to_string(),
        )
    );
    let discovery = RemoteFolderDiscoverySuccessV1 {
        folders,
        identity_hash: "28045004ce0bd95e5f70124d0616709eb6b4e0ebdca6069ac19e66abe589c2f9"
            .to_string(),
        field_hash: "80a9466fe3e3f32a7c79db83a3089cbce9a42c8698cf898540f7600beb666f7e".to_string(),
        complete_scan: true,
        bottom_confirmed: true,
        hit_record_limit: false,
    };
    validate_remote_folder_discovery(&discovery).unwrap();
    let mut invalid = discovery;
    invalid.hit_record_limit = true;
    assert_eq!(
        validate_remote_folder_discovery(&invalid).unwrap_err(),
        "invalid_remote_folder_discovery_state"
    );
}

#[test]
fn execution_envelope_is_bound_to_the_complete_job_identity() {
    let frozen = freeze_task_payload(history_payload()).unwrap();
    let job = SyncJobSnapshotV1 {
        id: 9,
        task_key: TaskKeyV1 {
            account_id: 7,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
        },
        trigger: SyncTriggerV1::Manual,
        status: SyncJobStatusV1::Running,
        task_payload_sha256: frozen.payload_sha256.clone(),
    };
    assert_eq!(
        job_identity_sha256(&job).unwrap(),
        "dabba5a988663c53fe9ea43bfed00978313bc9f356e3bb2fc0652e023a78a277"
    );
    let envelope = SyncExecutionEnvelopeV1 {
        schema_version: 1,
        account_id: 7,
        job_id: 9,
        source: SyncSourceV1::History,
        source_instance_key: String::new(),
        task_payload_sha256: frozen.payload_sha256,
        result: SourceExecutionResultV1::History(empty_history_result(7, 9)),
    };
    validate_execution_envelope(&envelope, &job).unwrap();

    let mut wrong = envelope;
    wrong.source_instance_key = "wrong".to_string();
    assert_eq!(
        validate_execution_envelope(&wrong, &job).unwrap_err(),
        "sync_result_identity_mismatch"
    );
}

#[test]
fn stage_capabilities_are_fixed_and_executor_registry_rejects_duplicates() {
    let capabilities = CapabilityRegistry::stage_3_3();
    let all = capabilities.all();
    assert_eq!(all.len(), 4);
    assert_eq!(
        all.iter().map(|value| value.source).collect::<Vec<_>>(),
        vec![
            SyncSourceV1::History,
            SyncSourceV1::Likes,
            SyncSourceV1::Favorites,
            SyncSourceV1::RemoteFolder,
        ]
    );
    let history = capabilities.get(SyncSourceV1::History).unwrap();
    assert!(history.available);
    assert_eq!(
        history.supported_ranges,
        vec![
            douyin_history_archive::sync::SyncRangeKindV1::Days,
            douyin_history_archive::sync::SyncRangeKindV1::Bottom,
        ]
    );
    assert!(!history.supports_full_reconcile);
    for source in [
        SyncSourceV1::Likes,
        SyncSourceV1::Favorites,
        SyncSourceV1::RemoteFolder,
    ] {
        let capability = capabilities.get(source).unwrap();
        assert!(capability.available);
        assert_eq!(capability.unavailable_reason, None);
        assert_eq!(
            capability.supported_ranges,
            vec![
                douyin_history_archive::sync::SyncRangeKindV1::All,
                douyin_history_archive::sync::SyncRangeKindV1::RecentItems,
                douyin_history_archive::sync::SyncRangeKindV1::RecordLimit,
            ]
        );
        assert!(capability.supports_full_reconcile);
        assert_eq!(
            capability.supports_remote_folders,
            source == SyncSourceV1::Favorites
        );
    }
    assert!(capabilities.supports_remote_folders());

    let backend = Arc::new(FakeSyncBackend::new(SyncSourceV1::History));
    let registry = ExecutorRegistry::new(vec![ExecutorRegistration::new(
        backend.clone(),
        backend.clone(),
    )])
    .unwrap();
    let frozen = freeze_task_payload(history_payload()).unwrap();
    let job = SyncJobSnapshotV1 {
        id: 11,
        task_key: TaskKeyV1 {
            account_id: 7,
            source: SyncSourceV1::History,
            source_instance_key: String::new(),
        },
        trigger: SyncTriggerV1::Manual,
        status: SyncJobStatusV1::Queued,
        task_payload_sha256: frozen.payload_sha256,
    };
    let running = registry
        .executor(SyncSourceV1::History)
        .unwrap()
        .start(QueuedSyncContext { job: job.clone() })
        .unwrap();
    registry
        .executor(SyncSourceV1::History)
        .unwrap()
        .cancel(&running)
        .unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    let transaction = connection.transaction().unwrap();
    let counts = registry
        .importer(SyncSourceV1::History)
        .unwrap()
        .validate_and_import(
            &transaction,
            &job,
            SourceExecutionResultV1::History(empty_history_result(7, 11)),
        )
        .unwrap();
    assert_eq!(counts, ImportCountsV1::default());
    assert_eq!(backend.calls().len(), 3);

    let failing = Arc::new(FakeSyncBackend::with_launch_error(
        SyncSourceV1::History,
        "fake_launch_error",
    ));
    let failing_registry = ExecutorRegistry::new(vec![ExecutorRegistration::new(
        failing.clone(),
        failing.clone(),
    )])
    .unwrap();
    assert_eq!(
        failing_registry
            .executor(SyncSourceV1::History)
            .unwrap()
            .start(QueuedSyncContext { job: job.clone() })
            .unwrap_err(),
        "fake_launch_error"
    );
    assert_eq!(failing.calls().len(), 1);

    let duplicate = Arc::new(FakeSyncBackend::new(SyncSourceV1::History));
    assert_eq!(
        ExecutorRegistry::new(vec![
            ExecutorRegistration::new(backend.clone(), backend),
            ExecutorRegistration::new(duplicate.clone(), duplicate),
        ])
        .unwrap_err(),
        "duplicate_sync_executor"
    );
}
