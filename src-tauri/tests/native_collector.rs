use douyin_history_archive::collector::{
    cli::{parse_collector_args, BrowserPreference, CollectRange, CollectorCommand},
    contracts::{CollectorDiagnostic, LoginDiagnostic, LoginProgressV1},
    errors::describe_collector_error,
    normalization::{normalize_snapshot, HistoryTimeUnit, RawHistoryItem},
    resource_policy::{classify_login_resource, ResourceDecision, ResourceStage},
    session_probe::{
        account_identity_from_home, is_valid_douyin_home_url, session_probe_from_history_value,
        session_probe_from_page_value, session_probe_from_value, SessionProbeV1,
        DOUYIN_HISTORY_URL,
    },
    snapshot::RawReactSnapshot,
    sync_engine::{create_range, finish_result, progress},
};
use douyin_history_archive::sync_orchestration::SyncPhase;
use douyin_history_archive::{
    settings_store::{ArchiveListRangeV1, HistoryRangeV1},
    sync::{ArchiveRunModeV1, SyncSourceV1},
};
use douyin_history_archive::{CollectorRange, CollectorSuccessV3};
use serde_json::json;
use std::ffi::OsString;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn parses_collect_arguments_with_existing_defaults() {
    let command = parse_collector_args(args(&[
        "collect",
        "--account-id",
        "11",
        "--job-id",
        "7",
        "--phase",
        "collecting",
        "--phase-attempt-id",
        "0123456789abcdef0123456789abcdef",
        "--source",
        "history",
        "--source-instance-key",
        "",
        "--task-payload-sha256",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--frozen-at-ms",
        "1786420800000",
        "--cutoff-ms",
        "1785859200000",
        "--browser-data-dir",
        "C:/app",
        "--result",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.result.json",
        "--progress",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.progress.json",
        "--diagnostic",
        "C:/app/diagnostics/account-11/sync-7-collecting-0123456789abcdef0123456789abcdef.json",
        "--cancel-file",
        "C:/app/tmp/account-11-sync-7.cancel",
        "--account-profile-root",
        "C:/app/profiles/0123456789abcdef0123456789abcdef",
        "--browser-pid-file",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.browser.pid",
        "--collector-identity-file",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.collector.pid",
        "--profile-key",
        "0123456789abcdef0123456789abcdef",
        "--profile-generation",
        "fedcba9876543210fedcba9876543210",
        "--browser-executable-sha256",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "--browser",
        "chrome",
        "--headless",
        "--block-images",
    ]))
    .expect("collect args");

    let CollectorCommand::Collect(options) = command else {
        panic!("expected collect command");
    };
    assert_eq!(options.account_id, 11);
    assert_eq!(options.sync_job_id, 7);
    assert_eq!(options.phase, Some(SyncPhase::Collecting));
    assert_eq!(
        options.phase_attempt_id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert_eq!(options.source, SyncSourceV1::History);
    assert_eq!(options.source_instance_key, "");
    assert_eq!(options.task_payload_sha256, "a".repeat(64));
    assert_eq!(options.frozen_at_ms, 1_786_420_800_000);
    assert_eq!(options.cutoff_ms, Some(1_785_859_200_000));
    assert_eq!(options.profile_key, "0123456789abcdef0123456789abcdef");
    assert_eq!(
        options.profile_generation,
        "fedcba9876543210fedcba9876543210"
    );
    assert_eq!(options.browser_executable_sha256, "b".repeat(64));
    assert_eq!(
        options.account_profile_root.to_string_lossy(),
        "C:/app/profiles/0123456789abcdef0123456789abcdef"
    );
    assert_eq!(options.browser, BrowserPreference::Chrome);
    assert_eq!(
        options.range,
        CollectRange::History(HistoryRangeV1::Days { days: 7 })
    );
    assert_eq!(options.scroll_speed, 6_000);
    assert!(options.headless);
    assert!(options.block_images);
    assert!(options.block_media);
    assert_eq!(options.hard_timeout_ms, 600_000);
    assert_eq!(options.max_records, 50_000);
}

#[test]
fn parses_archive_collect_arguments_with_strict_source_range_and_flags() {
    let command = parse_collector_args(args(&[
        "collect",
        "--account-id",
        "11",
        "--job-id",
        "7",
        "--phase",
        "collecting",
        "--phase-attempt-id",
        "0123456789abcdef0123456789abcdef",
        "--source",
        "favorites",
        "--source-instance-key",
        "",
        "--task-payload-sha256",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--frozen-at-ms",
        "1786420800000",
        "--cutoff-ms",
        "1785859200000",
        "--browser-data-dir",
        "C:/app",
        "--result",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.result.json",
        "--progress",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.progress.json",
        "--diagnostic",
        "C:/app/diagnostics/account-11/sync-7-collecting-0123456789abcdef0123456789abcdef.json",
        "--cancel-file",
        "C:/app/tmp/account-11-sync-7.cancel",
        "--account-profile-root",
        "C:/app/profiles/0123456789abcdef0123456789abcdef",
        "--browser-pid-file",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.browser.pid",
        "--collector-identity-file",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.collector.pid",
        "--profile-key",
        "0123456789abcdef0123456789abcdef",
        "--profile-generation",
        "fedcba9876543210fedcba9876543210",
        "--browser-executable-sha256",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "--browser",
        "chrome",
        "--page-url",
        "https://www.douyin.com/user/self?showTab=favorite_collection",
        "--archive-range",
        "recent_days",
        "--days",
        "7",
        "--run-mode",
        "incremental",
        "--discover-remote-folders",
        "--headless",
        "--block-images",
    ]))
    .expect("archive collect args");

    let CollectorCommand::Collect(options) = command else {
        panic!("expected collect command");
    };
    assert_eq!(options.source, SyncSourceV1::Favorites);
    assert_eq!(options.source_instance_key, "");
    assert_eq!(
        options.page_url,
        "https://www.douyin.com/user/self?showTab=favorite_collection"
    );
    assert_eq!(
        options.range,
        CollectRange::Archive(ArchiveListRangeV1::RecentDays { days: 7 })
    );
    assert_eq!(options.cutoff_ms, Some(1_785_859_200_000));
    assert_eq!(options.run_mode, Some(ArchiveRunModeV1::Incremental));
    assert!(options.discover_remote_folders);
    assert_eq!(options.known_ids_path, None);
}

#[test]
fn incremental_all_archive_requires_the_exact_phase_known_ids_path() {
    let base = vec![
        "collect",
        "--account-id",
        "11",
        "--job-id",
        "7",
        "--phase",
        "collecting",
        "--phase-attempt-id",
        "0123456789abcdef0123456789abcdef",
        "--source",
        "likes",
        "--source-instance-key",
        "",
        "--task-payload-sha256",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--frozen-at-ms",
        "1786420800000",
        "--browser-data-dir",
        "C:/app",
        "--result",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.result.json",
        "--progress",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.progress.json",
        "--diagnostic",
        "C:/app/diagnostics/account-11/sync-7-collecting-0123456789abcdef0123456789abcdef.json",
        "--cancel-file",
        "C:/app/tmp/account-11-sync-7.cancel",
        "--account-profile-root",
        "C:/app/profiles/0123456789abcdef0123456789abcdef",
        "--browser-pid-file",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.browser.pid",
        "--collector-identity-file",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.collector.pid",
        "--profile-key",
        "0123456789abcdef0123456789abcdef",
        "--profile-generation",
        "fedcba9876543210fedcba9876543210",
        "--browser-executable-sha256",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "--browser",
        "chrome",
        "--page-url",
        "https://www.douyin.com/user/self?showTab=like",
        "--archive-range",
        "all",
        "--run-mode",
        "incremental",
    ];

    assert_eq!(
        parse_collector_args(args(&base)),
        Err("missing_required_option:--known-ids-file".to_string())
    );

    let mut exact = base.clone();
    exact.extend([
        "--known-ids-file",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.archive-known-ids.json",
    ]);
    let CollectorCommand::Collect(options) = parse_collector_args(args(&exact)).unwrap() else {
        panic!("collect");
    };
    assert_eq!(
        options.known_ids_path.unwrap(),
        std::path::PathBuf::from(
            "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.archive-known-ids.json"
        )
    );

    let mut wrong = base;
    wrong.extend(["--known-ids-file", "C:/app/tmp/wrong.json"]);
    assert_eq!(
        parse_collector_args(args(&wrong)),
        Err("collector_path_mismatch:archive-known-ids".to_string())
    );
}

#[test]
fn archive_collect_rejects_wrong_instance_page_and_full_reconcile_range() {
    let base = [
        "collect",
        "--account-id",
        "11",
        "--job-id",
        "7",
        "--phase",
        "collecting",
        "--phase-attempt-id",
        "0123456789abcdef0123456789abcdef",
        "--source",
        "remote_folder",
        "--source-instance-key",
        "folder-a",
        "--task-payload-sha256",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--frozen-at-ms",
        "1786420800000",
        "--browser-data-dir",
        "C:/app",
        "--result",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.result.json",
        "--progress",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.progress.json",
        "--diagnostic",
        "C:/app/diagnostics/account-11/sync-7-collecting-0123456789abcdef0123456789abcdef.json",
        "--cancel-file",
        "C:/app/tmp/account-11-sync-7.cancel",
        "--account-profile-root",
        "C:/app/profiles/0123456789abcdef0123456789abcdef",
        "--browser-pid-file",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.browser.pid",
        "--collector-identity-file",
        "C:/app/tmp/account-11-sync-7-collecting-0123456789abcdef0123456789abcdef.collector.pid",
        "--profile-key",
        "0123456789abcdef0123456789abcdef",
        "--profile-generation",
        "fedcba9876543210fedcba9876543210",
        "--browser-executable-sha256",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "--browser",
        "chrome",
        "--page-url",
        "https://www.douyin.com/user/self?showTab=favorite_collection",
        "--archive-range",
        "recent_items",
        "--item-count",
        "25",
        "--run-mode",
        "full_reconcile",
    ];

    assert_eq!(
        parse_collector_args(args(&base)),
        Err("full_reconcile_requires_all".to_string())
    );
}

#[test]
fn login_commands_reject_collect_execution_identity_options() {
    assert_eq!(
        parse_collector_args(args(&[
            "login",
            "--login-attempt-id",
            "0123456789abcdef0123456789abcdef",
            "--browser-data-dir",
            "C:/app",
            "--account-profile-root",
            "C:/app/profiles/0123456789abcdef0123456789abcdef",
            "--identity-result",
            "C:/app/tmp/login-0123456789abcdef0123456789abcdef.identity.json",
            "--login-progress-file",
            "C:/app/tmp/login-0123456789abcdef0123456789abcdef.progress.json",
            "--diagnostic",
            "C:/app/diagnostics/login-0123456789abcdef0123456789abcdef.json",
            "--cancel-file",
            "C:/app/tmp/login-0123456789abcdef0123456789abcdef.cancel",
            "--browser-pid-file",
            "C:/app/tmp/login-0123456789abcdef0123456789abcdef.browser.pid",
            "--frozen-at-ms",
            "1786420800000",
        ])),
        Err("unexpected_collect_option".to_string())
    );
}

#[test]
fn rejects_unknown_and_missing_collector_options() {
    assert_eq!(
        parse_collector_args(args(&[
            "login",
            "--account-profile-root",
            "C:/app",
            "--wat"
        ])),
        Err("unexpected_option:--wat".to_string())
    );
    assert_eq!(
        parse_collector_args(args(&["login", "--account-profile-root"])),
        Err("missing_option_value:--account-profile-root".to_string())
    );
}

#[test]
fn phase_identity_options_are_complete_and_use_their_own_error_contract() {
    assert_eq!(
        parse_collector_args(args(&[
            "collect",
            "--account-id",
            "11",
            "--job-id",
            "7",
            "--browser-data-dir",
            "C:/app",
            "--account-profile-root",
            "C:/app/profiles/0123456789abcdef0123456789abcdef",
            "--phase",
            "collecting",
        ])),
        Err("incomplete_sync_phase_identity".to_string())
    );
    assert_eq!(
        parse_collector_args(args(&[
            "collect",
            "--account-id",
            "11",
            "--job-id",
            "7",
            "--browser-data-dir",
            "C:/app",
            "--account-profile-root",
            "C:/app/profiles/0123456789abcdef0123456789abcdef",
            "--phase",
            "collecting",
            "--phase-attempt-id",
            "not-a-phase-attempt",
        ])),
        Err("invalid_phase_attempt_id".to_string())
    );
    assert_eq!(
        parse_collector_args(args(&[
            "login",
            "--browser-data-dir",
            "C:/app",
            "--account-profile-root",
            "C:/app/profiles/0123456789abcdef0123456789abcdef",
            "--phase",
            "collecting",
            "--phase-attempt-id",
            "0123456789abcdef0123456789abcdef",
        ])),
        Err("unexpected_sync_phase_option".to_string())
    );
}

#[test]
fn parses_account_scoped_login_arguments() {
    let command = parse_collector_args(args(&[
        "login",
        "--login-attempt-id",
        "0123456789abcdef0123456789abcdef",
        "--browser-data-dir",
        "C:/app",
        "--account-profile-root",
        "C:/app/profiles/0123456789abcdef0123456789abcdef",
        "--identity-result",
        "C:/app/tmp/login-0123456789abcdef0123456789abcdef.identity.json",
        "--login-progress-file",
        "C:/app/tmp/login-0123456789abcdef0123456789abcdef.progress.json",
        "--diagnostic",
        "C:/app/diagnostics/login-0123456789abcdef0123456789abcdef.json",
        "--cancel-file",
        "C:/app/tmp/login-0123456789abcdef0123456789abcdef.cancel",
        "--browser-pid-file",
        "C:/app/tmp/login-0123456789abcdef0123456789abcdef.browser.pid",
        "--home-url",
        "https://www.douyin.com/",
    ]))
    .expect("login args");

    let CollectorCommand::Login(options) = command else {
        panic!("expected login command");
    };
    assert_eq!(options.target_account_id, None);
    assert_eq!(options.login_attempt_id, "0123456789abcdef0123456789abcdef");
    assert!(options
        .identity_result_path
        .ends_with("login-0123456789abcdef0123456789abcdef.identity.json"));
    assert!(options
        .progress_path
        .ends_with("login-0123456789abcdef0123456789abcdef.progress.json"));
    assert_eq!(options.home_url, "https://www.douyin.com/");
}

#[test]
fn login_progress_rejects_unknown_identity_state_and_unbounded_deadlines() {
    let valid = json!({
        "schemaVersion": 1,
        "accountId": 11,
        "loginAttemptId": "0123456789abcdef0123456789abcdef",
        "state": "closing",
        "reason": "browser_closed",
        "observedAtMs": 1000,
        "deadlineAtMs": 3000
    });
    let progress: LoginProgressV1 = serde_json::from_value(valid.clone()).unwrap();
    assert_eq!(
        progress.validate_for(11, "0123456789abcdef0123456789abcdef"),
        Ok(())
    );

    let mut unknown = valid.clone();
    unknown["unexpected"] = json!(true);
    let mut wrong_account = valid.clone();
    wrong_account["accountId"] = json!(12);
    let mut running = valid.clone();
    running["state"] = json!("running");
    let mut too_long = valid.clone();
    too_long["deadlineAtMs"] = json!(3001);
    let mut backwards = valid;
    backwards["deadlineAtMs"] = json!(999);
    for invalid in [unknown, wrong_account, running, too_long, backwards] {
        match serde_json::from_value::<LoginProgressV1>(invalid) {
            Ok(value) => assert!(value
                .validate_for(11, "0123456789abcdef0123456789abcdef")
                .is_err()),
            Err(_) => {}
        }
    }
}

#[test]
fn login_progress_preserves_browser_cancel_timeout_and_failure_reasons() {
    use douyin_history_archive::collector::{
        contracts::LoginProgressReason, runner::login_progress_for_error,
    };

    for (error, expected) in [
        ("cdp_target_closed", LoginProgressReason::BrowserClosed),
        ("collector_cancelled", LoginProgressReason::Cancelled),
        ("account_identity_unavailable", LoginProgressReason::Timeout),
        ("identity_result_write_failed", LoginProgressReason::Failed),
    ] {
        let progress =
            login_progress_for_error(error, 11, "0123456789abcdef0123456789abcdef", 1_000);
        assert_eq!(progress.reason, expected);
        assert_eq!(progress.state, "closing");
        assert_eq!(progress.deadline_at_ms, 3_000);
    }
}

#[test]
fn collect_and_login_reject_unbound_paths() {
    assert_eq!(
        parse_collector_args(args(&[
            "collect",
            "--account-id",
            "11",
            "--job-id",
            "7",
            "--browser-data-dir",
            "C:/app",
            "--result",
            "C:/outside/result.json",
            "--progress",
            "C:/app/tmp/account-11-sync-7.progress.json",
            "--diagnostic",
            "C:/app/diagnostics/account-11/sync-7.json",
            "--cancel-file",
            "C:/app/tmp/account-11-sync-7.cancel",
            "--account-profile-root",
            "C:/app/profiles/0123456789abcdef0123456789abcdef",
            "--browser-pid-file",
            "C:/app/tmp/account-11-sync-7.browser.pid"
        ])),
        Err("collector_path_mismatch:result".into())
    );
    assert_eq!(
        parse_collector_args(args(&[
            "login",
            "--account-id",
            "11",
            "--login-attempt-id",
            "0123456789abcdef0123456789abcdef",
            "--browser-data-dir",
            "C:/app",
            "--account-profile-root",
            "C:/app/profiles/not-a-profile-key",
            "--identity-result",
            "C:/outside/identity.json",
            "--diagnostic",
            "C:/outside/diagnostic.json",
            "--cancel-file",
            "C:/outside/cancel",
            "--browser-pid-file",
            "C:/outside/browser.pid"
        ])),
        Err("invalid_account_profile_root".into())
    );
}

#[test]
fn collector_contracts_include_account_id_and_schema_three() {
    let result = finish_result(
        11,
        7,
        1_783_742_400_000,
        CollectorRange::Bottom,
        HistoryTimeUnit::Milliseconds,
        Vec::new(),
    )
    .expect("result");
    let serialized = serde_json::to_value(&result).expect("serialize result");
    assert_eq!(result.schema_version, 3);
    assert_eq!(result.account_id, 11);
    assert_eq!(serialized["accountId"], 11);
    let _: CollectorSuccessV3 = serde_json::from_value(serialized).expect("v3 result");

    let progress = progress(11, 7, "collecting", 2, 10, 0, true);
    assert_eq!(progress.schema_version, 3);
    assert_eq!(progress.account_id, 11);
    assert_eq!(serde_json::to_value(progress).unwrap()["accountId"], 11);

    let diagnostic: CollectorDiagnostic = serde_json::from_value(json!({
        "schemaVersion": 3,
        "accountId": 11,
        "syncJobId": 7,
        "status": "failed",
        "phase": "collecting",
        "errorCode": "collector_failed",
        "message": "failed",
        "recordCount": 0,
        "domCardCount": 0,
        "calendarTimeZone": "Asia/Shanghai",
        "elapsedMs": 10,
        "noProgressMs": 0,
        "retryCount": 0
    }))
    .expect("diagnostic");
    assert_eq!(diagnostic.account_id, 11);
}

#[test]
fn login_diagnostic_includes_account_and_attempt_identity() {
    let diagnostic: LoginDiagnostic = serde_json::from_value(json!({
        "schemaVersion": 3,
        "accountId": 11,
        "loginAttemptId": "0123456789abcdef0123456789abcdef",
        "status": "failed",
        "errorCode": "collector_failed",
        "message": "failed"
    }))
    .expect("login diagnostic");

    assert_eq!(diagnostic.account_id, 11);
    assert_eq!(
        diagnostic.login_attempt_id,
        "0123456789abcdef0123456789abcdef"
    );
}

#[test]
fn maps_stable_collector_errors_to_chinese_messages() {
    let description = describe_collector_error("cdp_connection_failed");
    assert_eq!(description.code, "cdp_connection_failed");
    assert_eq!(description.message, "无法连接到浏览器 CDP 服务。");

    let ambiguous = describe_collector_error("account_identity_ambiguous");
    assert_eq!(ambiguous.code, "account_identity_ambiguous");
    assert_eq!(ambiguous.message, "页面中检测到互相冲突的登录账号身份。");

    let expired = describe_collector_error("session_expired");
    assert_eq!(expired.code, "session_expired");
    assert_eq!(expired.message, "抖音登录状态已失效。");

    let opaque = describe_collector_error("包含敏感页面内容");
    assert_eq!(opaque.code, "collector_failed");
    assert_eq!(opaque.message, "采集器运行失败。");
}

#[test]
fn normalizes_seconds_urls_progress_and_shanghai_date() {
    let started = 1_720_368_000_000_i64;
    let item = RawHistoryItem {
        aweme_id: json!("123456"),
        aweme_type: Some(json!(0)),
        history_time: json!(1_720_000_000_i64),
        href: "https://douyin.com/video/123456?x=1".into(),
        source_position: 0,
        author_name: Some(json!(" 作者 ")),
        item_title: Some(json!(" 标题 ")),
        desc: None,
        cover_urls: Some(json!(["https://example.com/cover.jpg"])),
        like_count: Some(json!(123)),
        history_progress_rate: Some(json!("25%")),
    };
    let (unit, records) = normalize_snapshot(&[item], started, None).unwrap();
    assert_eq!(unit, HistoryTimeUnit::Seconds);
    assert_eq!(
        records[0].canonical_url,
        "https://www.douyin.com/video/123456"
    );
    assert_eq!(records[0].watch_progress_rate, Some(0.25));
    assert_eq!(records[0].watched_date, "2024-07-03");
}

#[test]
fn creates_a_shanghai_midnight_day_range() {
    let range = create_range(false, 3, 1_720_627_200_000);
    let CollectorRange::Days { days, cutoff_ms } = range else {
        panic!("days");
    };
    assert_eq!(days, 3);
    assert_eq!(cutoff_ms % 86_400_000, 57_600_000);
}

#[test]
fn raw_history_item_rejects_unknown_fields() {
    let value = json!({
        "awemeId": "123456",
        "historyTime": 1_720_000_000_i64,
        "href": "https://www.douyin.com/video/123456",
        "sourcePosition": 0,
        "unexpected": true
    });

    assert!(serde_json::from_value::<RawHistoryItem>(value).is_err());
}

#[test]
fn raw_react_snapshot_rejects_unknown_fields() {
    let value = json!({
        "kind": "history",
        "items": [],
        "domCardCount": 0,
        "scrollTop": 0.0,
        "scrollHeight": 0.0,
        "clientHeight": 0.0,
        "noMoreVisible": false,
        "loadingVisible": false,
        "unexpected": true
    });

    assert!(serde_json::from_value::<RawReactSnapshot>(value).is_err());
}

#[test]
fn session_probe_accepts_only_the_strict_douyin_home_url() {
    assert!(is_valid_douyin_home_url("https://www.douyin.com/"));
    assert!(is_valid_douyin_home_url("https://www.douyin.com/jingxuan"));
    assert!(is_valid_douyin_home_url(
        "https://www.douyin.com/?enter=home"
    ));
    for value in [
        "http://www.douyin.com/",
        "https://m.douyin.com/",
        "https://www.douyin.com:444/",
        "https://user@www.douyin.com/",
        "https://www.douyin.com/video/1",
        "https://www.douyin.com/jingxuan/",
        "https://www.douyin.com/#login",
    ] {
        assert!(!is_valid_douyin_home_url(value), "{value}");
    }
}

#[test]
fn session_probe_builds_observed_identity_and_rejects_unknown_fields() {
    let value = json!({
        "kind": "authenticated",
        "pageUrl": "https://www.douyin.com/?enter=home",
        "identity": {
            "uid": "112233445566",
            "secUid": "MS4wLjABAAAAfixture-sec-uid",
            "nickname": "Fixture User",
            "avatarUrl": "https://p3-pc-sign.douyinpic.com/fixture.jpeg",
            "sources": ["ssr_render_data"]
        }
    });

    let SessionProbeV1::Authenticated(identity) =
        session_probe_from_value(&value).expect("authenticated probe")
    else {
        panic!("authenticated");
    };
    assert_eq!(identity.page_url, "https://www.douyin.com/?enter=home");
    assert_eq!(identity.uid.as_deref(), Some("112233445566"));
    assert_eq!(identity.sources.len(), 1);

    let mut unknown = value.clone();
    unknown["identity"]["historyUrl"] = json!("https://attacker.invalid/");
    assert_eq!(
        session_probe_from_value(&unknown).unwrap_err(),
        "invalid_session_probe"
    );
}

#[test]
fn rust_owns_the_canonical_history_url_for_home_identity() {
    let probe = session_probe_from_value(&json!({
        "kind": "authenticated",
        "pageUrl": "https://www.douyin.com/",
        "identity": {
            "uid": null,
            "secUid": "MS4wLjABAAAAfixture-sec-uid",
            "nickname": null,
            "avatarUrl": null,
            "sources": ["initial_state_user"]
        }
    }))
    .expect("probe");
    let SessionProbeV1::Authenticated(identity) = probe else {
        panic!("authenticated");
    };

    let account = account_identity_from_home(
        &identity,
        11,
        "0123456789abcdef0123456789abcdef",
        "abcdef0123456789abcdef0123456789",
        "2026-07-19T12:00:00.000Z",
    )
    .expect("account identity");

    assert_eq!(account.history_url, DOUYIN_HISTORY_URL);
}

#[test]
fn recover_login_requires_hidden_phase_scoped_files_and_an_absolute_deadline() {
    let command = parse_collector_args(args(&[
        "recover-login",
        "--account-id",
        "11",
        "--job-id",
        "7",
        "--phase",
        "recovering_login",
        "--phase-attempt-id",
        "0123456789abcdef0123456789abcdef",
        "--browser-data-dir",
        "C:/app",
        "--result",
        "C:/app/tmp/account-11-sync-7-recovering_login-0123456789abcdef0123456789abcdef.result.json",
        "--progress",
        "C:/app/tmp/account-11-sync-7-recovering_login-0123456789abcdef0123456789abcdef.progress.json",
        "--diagnostic",
        "C:/app/diagnostics/account-11/sync-7-recovering_login-0123456789abcdef0123456789abcdef.json",
        "--cancel-file",
        "C:/app/tmp/account-11-sync-7.cancel",
        "--account-profile-root",
        "C:/app/profiles/0123456789abcdef0123456789abcdef",
        "--browser-pid-file",
        "C:/app/tmp/account-11-sync-7-recovering_login-0123456789abcdef0123456789abcdef.browser.pid",
        "--collector-identity-file",
        "C:/app/tmp/account-11-sync-7-recovering_login-0123456789abcdef0123456789abcdef.collector.pid",
        "--deadline-at-ms",
        "1784457630000",
        "--headless",
    ]))
    .expect("recover login args");

    let CollectorCommand::RecoverLogin(options) = command else {
        panic!("expected recover-login command");
    };
    assert_eq!(options.account_id, 11);
    assert_eq!(options.sync_job_id, 7);
    assert_eq!(options.phase, SyncPhase::RecoveringLogin);
    assert_eq!(options.phase_attempt_id, "0123456789abcdef0123456789abcdef");
    assert_eq!(options.deadline_at_ms, 1_784_457_630_000);
    assert!(options.headless);
    assert!(options.result_path.ends_with(
        "account-11-sync-7-recovering_login-0123456789abcdef0123456789abcdef.result.json"
    ));
    assert!(options.collector_identity_path.ends_with(
        "account-11-sync-7-recovering_login-0123456789abcdef0123456789abcdef.collector.pid"
    ));
}

#[test]
fn recover_login_rejects_visible_missing_deadline_and_unscoped_identity_invocations() {
    let base = [
        "recover-login",
        "--account-id",
        "11",
        "--job-id",
        "7",
        "--phase",
        "recovering_login",
        "--phase-attempt-id",
        "0123456789abcdef0123456789abcdef",
        "--browser-data-dir",
        "C:/app",
        "--result",
        "C:/app/tmp/account-11-sync-7-recovering_login-0123456789abcdef0123456789abcdef.result.json",
        "--progress",
        "C:/app/tmp/account-11-sync-7-recovering_login-0123456789abcdef0123456789abcdef.progress.json",
        "--diagnostic",
        "C:/app/diagnostics/account-11/sync-7-recovering_login-0123456789abcdef0123456789abcdef.json",
        "--cancel-file",
        "C:/app/tmp/account-11-sync-7.cancel",
        "--account-profile-root",
        "C:/app/profiles/0123456789abcdef0123456789abcdef",
        "--browser-pid-file",
        "C:/app/tmp/account-11-sync-7-recovering_login-0123456789abcdef0123456789abcdef.browser.pid",
        "--collector-identity-file",
        "C:/app/tmp/account-11-sync-7-recovering_login-0123456789abcdef0123456789abcdef.collector.pid",
    ];

    assert_eq!(
        parse_collector_args(args(&base)),
        Err("missing_required_option:--deadline-at-ms".to_string())
    );

    let mut visible = base.to_vec();
    visible.extend(["--deadline-at-ms", "1784457630000"]);
    assert_eq!(
        parse_collector_args(args(&visible)),
        Err("recover_login_requires_headless".to_string())
    );

    let mut wrong_identity = visible;
    wrong_identity.extend(["--headless"]);
    let identity_index = wrong_identity
        .iter()
        .position(|value| *value == "--collector-identity-file")
        .unwrap()
        + 1;
    wrong_identity[identity_index] = "C:/app/tmp/other.collector.pid";
    assert_eq!(
        parse_collector_args(args(&wrong_identity)),
        Err("collector_path_mismatch:collector-identity".to_string())
    );
}

#[test]
fn history_session_probe_maps_only_explicit_logout_and_rejects_other_pages() {
    let logged_out = json!({
        "kind": "unauthenticated",
        "pageUrl": "https://www.douyin.com/user/self?showTab=record",
        "source": "initial_state_user"
    });
    assert!(matches!(
        session_probe_from_history_value(&logged_out).unwrap(),
        SessionProbeV1::Unauthenticated { .. }
    ));
    let mut wrong_page = logged_out;
    wrong_page["pageUrl"] = json!("https://www.douyin.com/user/self?showTab=like");
    assert_eq!(
        session_probe_from_history_value(&wrong_page),
        Err("invalid_session_probe".to_string())
    );
}

#[test]
fn archive_session_probe_requires_the_exact_task_page() {
    let page_url = "https://www.douyin.com/user/self?showTab=like";
    let logged_out = json!({
        "kind": "unauthenticated",
        "pageUrl": page_url,
        "source": "ssr_render_data"
    });
    assert!(matches!(
        session_probe_from_page_value(&logged_out, page_url).unwrap(),
        SessionProbeV1::Unauthenticated { .. }
    ));

    let wrong_page = json!({
        "kind": "unauthenticated",
        "pageUrl": DOUYIN_HISTORY_URL,
        "source": "ssr_render_data"
    });
    assert_eq!(
        session_probe_from_page_value(&wrong_page, page_url),
        Err("invalid_session_probe".to_string())
    );
}

#[test]
fn login_resource_policy_blocks_media_and_fails_open_for_images() {
    assert_eq!(
        classify_login_resource(
            ResourceStage::Login,
            "Media",
            "https://www.douyin.com/aweme/v1/play/"
        ),
        ResourceDecision::Fail
    );
    for url in [
        "https://www.douyin.com/aweme/v1/web/get-qrcode/",
        "https://verify.zijieapi.com/captcha/image",
        "https://p3-pc-sign.douyinpic.com/avatar.jpeg",
        "https://unknown.example/image.webp",
        "data:image/png;base64,AA==",
        "blob:https://www.douyin.com/id",
        "not a url",
    ] {
        assert_eq!(
            classify_login_resource(ResourceStage::Login, "Image", url),
            ResourceDecision::Continue,
            "{url}"
        );
    }
}

#[test]
fn archive_collector_errors_have_actionable_chinese_messages() {
    for (code, expected) in [
        ("archive_page_not_ready", "归档页面尚未准备完成。"),
        ("favorite_tab_not_found", "未能进入默认收藏内容页。"),
        ("favorite_folder_tab_not_found", "未能进入抖音收藏夹目录。"),
        ("favorite_navigation_not_visible", "收藏页面入口当前不可见。"),
        ("remote_folder_list_not_found", "未能读取抖音收藏夹目录。"),
        ("remote_folder_not_selected", "未能切换到指定的远端收藏夹。"),
        (
            "archive_snapshot_prefix_changed",
            "归档列表顺序在采集过程中发生变化。",
        ),
        ("final_stability_failed", "归档列表在最终复核时仍不稳定。"),
    ] {
        let described = describe_collector_error(code);
        assert_eq!(described.code, code);
        assert_eq!(described.message, expected);
    }
}
