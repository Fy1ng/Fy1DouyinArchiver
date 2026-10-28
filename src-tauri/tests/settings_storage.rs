use douyin_history_archive::bootstrap::{
    parse_bootstrap, read_bootstrap, validate_data_root_marker, write_bootstrap_atomic,
    write_data_root_marker_atomic, BootstrapV1, DataRootMarkerV1, DataRootRefV1,
    RetainedDataRootV1, RetainedLayoutV1,
};
use douyin_history_archive::settings_store::{
    account_settings_for, decode_portable_settings_bytes, import_legacy_frontend_settings,
    load_portable_settings, load_app_settings_view, portable_account_key, remove_account_settings,
    retain_known_account_settings, update_account_defaults, update_account_settings,
    update_global_settings, update_workspace_settings, AccountSettingsV1, AccountSettingsV2,
    ArchiveListRangeV1,
    BackupSettingsV1, CloseBehaviorV1, DetailFieldsV1, GlobalSettingsV2, HistoryRangeV1,
    LegacyFrontendSettingsV1, LikeCountFormatV1, NotificationSettingsV1, PortableSettingsV1,
    PortableSettingsV2, CoverCacheBackfillModeV1, CoverCacheCleanupModeV1,
    RemoteFolderScheduleModeV1, SyncModeV1, ThemeModeV1, UpdateSettingsV1,
};
use douyin_history_archive::{
    accounts::{
        archive_account, delete_account_data, known_portable_account_keys, list_accounts,
        load_active_account, portable_account_key_for_id, set_active_account,
        AccountFolderDisposition,
    },
    initialize_schema_v4,
};
use rusqlite::{params, Connection};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-settings-{label}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn root_id(character: char) -> String {
    std::iter::repeat_n(character, 32).collect()
}

fn valid_portable_v2_json() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 2,
        "revision": 9,
        "legacyFrontendImport": { "completed": false, "fingerprint": null },
        "global": {
            "likeCountFormat": "wan",
            "detailFields": {
                "author": true,
                "watchDate": true,
                "likeCount": true,
                "watchProgress": true
            },
            "closeBehavior": "exit",
            "startupSyncDelayMinutes": 10
        },
        "accountDefaults": {
            "hideBrowser": false,
            "scrollSpeed": 900,
            "history": {
                "range": { "kind": "days", "days": 7 },
                "schedule": { "enabled": false, "intervalMinutes": 360 }
            },
            "likes": {
                "range": { "kind": "all" },
                "schedule": { "enabled": false, "intervalMinutes": 360 }
            },
            "favorites": {
                "archive": {
                    "range": { "kind": "all" },
                    "schedule": { "enabled": false, "intervalMinutes": 360 }
                },
                "remoteFolderMode": "unified",
                "remoteFolderDefaults": {
                    "range": { "kind": "all" },
                    "schedule": { "enabled": false, "intervalMinutes": 360 }
                },
                "remoteFolders": {}
            }
        },
        "accounts": {}
    })
}

fn valid_portable_v3_json() -> serde_json::Value {
    let mut value = valid_portable_v2_json();
    value["schemaVersion"] = serde_json::json!(3);
    value["global"]["coverCache"] = serde_json::json!({
        "enabled": true,
        "backfillMode": "backfillAll",
        "capacityLimitMib": 4096,
        "cleanupByAgeEnabled": true,
        "maxAgeDays": 30,
        "cleanupByCapacityEnabled": true,
        "cleanupListMode": "cleanListed"
    });
    value
}

fn valid_portable_v4_json() -> serde_json::Value {
    let mut value = valid_portable_v3_json();
    value["schemaVersion"] = serde_json::json!(4);
    value["global"]["theme"] = serde_json::json!("dark");
    value
}

fn valid_portable_v5_json() -> serde_json::Value {
    let mut value = valid_portable_v4_json();
    value["schemaVersion"] = serde_json::json!(5);
    value["global"]["backup"] = serde_json::json!({
        "enabled": true,
        "intervalDays": 14,
        "executionTime": "04:30",
        "retentionCount": 8
    });
    value["global"]["notifications"] = serde_json::json!({
        "loginExpired": true,
        "automaticFailureStreak": true,
        "backupFailure": true,
        "syncCompleted": true,
        "backupCompleted": false,
        "automaticFailureThreshold": 5
    });
    value
}

fn valid_portable_v6_json(theme: &str) -> serde_json::Value {
    let mut value = serde_json::to_value(PortableSettingsV2::default()).unwrap();
    value["schemaVersion"] = serde_json::json!(6);
    value["global"]["theme"] = serde_json::json!(theme);
    value.as_object_mut().unwrap().remove("workspace");
    value
}

#[test]
fn new_install_uses_schema_v7_dark_theme_and_explicit_all_account_scopes() {
    let settings = PortableSettingsV2::default();
    let value = serde_json::to_value(settings).unwrap();

    assert_eq!(value["schemaVersion"], 7);
    assert_eq!(value["global"]["theme"], "dark");
    assert_eq!(value["workspace"]["sync"]["mode"], "singleAccount");
    assert_eq!(
        value["workspace"]["displayScopes"],
        serde_json::json!({
            "overview": { "kind": "all" },
            "library": { "kind": "all" },
            "search": { "kind": "all" }
        })
    );
    assert_eq!(value["workspace"]["contentViews"]["all"]["page"], 1);
    assert_eq!(
        value["workspace"]["contentViews"]["history"]["sort"],
        "watchedNewest"
    );
}

#[test]
fn portable_v6_migration_preserves_theme_and_adds_workspace_defaults() {
    for theme in ["system", "light", "dark"] {
        let migrated = decode_portable_settings_bytes(
            &serde_json::to_vec(&valid_portable_v6_json(theme)).unwrap(),
        )
        .unwrap();
        let value = serde_json::to_value(migrated).unwrap();

        assert_eq!(value["schemaVersion"], 7);
        assert_eq!(value["global"]["theme"], theme);
        assert_eq!(value["workspace"]["displayScopes"]["overview"]["kind"], "all");
        assert_eq!(value["workspace"]["displayScopes"]["library"]["kind"], "all");
        assert_eq!(value["workspace"]["displayScopes"]["search"]["kind"], "all");
    }
}

#[test]
fn portable_account_scopes_reject_empty_duplicate_and_unsorted_selected_keys() {
    let mut empty = valid_portable_v6_json("dark");
    empty["schemaVersion"] = serde_json::json!(7);
    empty["workspace"] = serde_json::json!({
        "sync": { "mode": "aggregate", "accountScope": { "kind": "selected", "accountKeys": [] } },
        "displayScopes": {
            "overview": { "kind": "all" },
            "library": { "kind": "all" },
            "search": { "kind": "all" }
        },
        "overviewLayouts": [],
        "contentViews": {
            "all": { "query": "", "page": 1, "viewMode": "horizontal", "includeRemoteRemoved": false, "sort": "archivedNewest" },
            "history": { "query": "", "page": 1, "viewMode": "horizontal", "includeRemoteRemoved": false, "sort": "watchedNewest" },
            "likes": { "query": "", "page": 1, "viewMode": "horizontal", "includeRemoteRemoved": false, "sort": "pageOrder" },
            "favorites": { "query": "", "page": 1, "viewMode": "horizontal", "includeRemoteRemoved": false, "sort": "pageOrder" }
        },
        "viewedAnnualReviewYears": []
    });
    assert_eq!(
        decode_portable_settings_bytes(&serde_json::to_vec(&empty).unwrap()).unwrap_err(),
        "invalid_account_scope"
    );

    for keys in [
        vec!["sec:b", "sec:a"],
        vec!["sec:a", "sec:a"],
    ] {
        let mut invalid = empty.clone();
        invalid["workspace"]["sync"]["accountScope"] = serde_json::json!({
            "kind": "selected",
            "accountKeys": keys
        });
        assert_eq!(
            decode_portable_settings_bytes(&serde_json::to_vec(&invalid).unwrap()).unwrap_err(),
            "invalid_account_scope"
        );
    }
}

#[test]
fn workspace_update_normalizes_selected_keys_and_rejects_stale_revisions() {
    let root = temp_root("workspace-cas");
    let current = load_portable_settings(&root).unwrap();
    let mut workspace = current.workspace.clone();
    workspace.display_scopes.library =
        douyin_history_archive::settings_store::PortableAccountScopeV1::Selected {
            account_keys: vec![
                "sec:beta".to_string(),
                "sec:alpha".to_string(),
                "sec:alpha".to_string(),
            ],
        };

    let updated = update_workspace_settings(&root, current.revision, workspace).unwrap();
    let value = serde_json::to_value(&updated.workspace).unwrap();
    assert_eq!(
        value["displayScopes"]["library"]["accountKeys"],
        serde_json::json!(["sec:alpha", "sec:beta"])
    );
    assert_eq!(
        update_workspace_settings(&root, current.revision, updated.workspace.clone()).unwrap_err(),
        "portable_settings_revision_conflict"
    );
    assert_eq!(load_portable_settings(&root).unwrap(), updated);
}

#[test]
fn portable_v2_is_migrated_to_v6_with_network_only_cover_defaults_and_system_theme() {
    let root = temp_root("portable-v2-cover-migration");
    write_settings_json(&root, &valid_portable_v2_json());

    let migrated = load_portable_settings(&root).unwrap();

    assert_eq!(migrated.schema_version, 7);
    assert_eq!(migrated.global.theme, ThemeModeV1::System);
    assert!(!migrated.global.cover_cache.enabled);
    assert_eq!(
        migrated.global.cover_cache.backfill_mode,
        CoverCacheBackfillModeV1::NewRecordsOnly
    );
    assert_eq!(migrated.global.cover_cache.capacity_limit_mib, 2_048);
    assert!(!migrated.global.cover_cache.cleanup_by_age_enabled);
    assert!(!migrated.global.cover_cache.cleanup_by_capacity_enabled);
    assert_eq!(
        migrated.global.cover_cache.cleanup_list_mode,
        CoverCacheCleanupModeV1::KeepListed
    );
}

#[test]
fn portable_v3_is_migrated_to_v6_without_changing_cover_cache_policy() {
    let root = temp_root("portable-v3-theme-migration");
    write_settings_json(&root, &valid_portable_v3_json());

    let migrated = load_portable_settings(&root).unwrap();

    assert_eq!(migrated.schema_version, 7);
    assert_eq!(migrated.global.theme, ThemeModeV1::System);
    assert!(migrated.global.cover_cache.enabled);
    assert_eq!(
        migrated.global.cover_cache.backfill_mode,
        CoverCacheBackfillModeV1::BackfillAll
    );
    assert_eq!(migrated.global.cover_cache.capacity_limit_mib, 4_096);
    assert_eq!(
        migrated.global.cover_cache.cleanup_list_mode,
        CoverCacheCleanupModeV1::CleanListed
    );
}

#[test]
fn portable_v4_is_migrated_to_v6_with_safe_backup_and_notification_defaults() {
    let root = temp_root("portable-v4-backup-migration");
    write_settings_json(&root, &valid_portable_v4_json());

    let migrated = load_portable_settings(&root).unwrap();

    assert_eq!(migrated.schema_version, 7);
    assert_eq!(migrated.global.theme, ThemeModeV1::Dark);
    assert_eq!(migrated.global.backup, BackupSettingsV1::default());
    assert!(!migrated.global.backup.enabled);
    assert_eq!(
        migrated.global.notifications,
        NotificationSettingsV1::default()
    );
    assert!(migrated.global.notifications.login_expired);
    assert!(migrated.global.notifications.automatic_failure_streak);
    assert!(migrated.global.notifications.backup_failure);
    assert!(!migrated.global.notifications.sync_completed);
    assert!(!migrated.global.notifications.backup_completed);
    assert_eq!(migrated.global.notifications.automatic_failure_threshold, 3);
    assert_eq!(migrated.global.notifications.message_duration_seconds, 5);

    let serialized = serde_json::to_value(migrated).unwrap();
    assert!(serialized["global"]["backup"].get("directory").is_none());
}

#[test]
fn portable_v5_is_migrated_to_v6_with_updates_disabled_by_default() {
    let root = temp_root("portable-v5-update-migration");
    write_settings_json(&root, &valid_portable_v5_json());

    let migrated = load_portable_settings(&root).unwrap();

    assert_eq!(migrated.schema_version, 7);
    assert_eq!(migrated.global.updates, UpdateSettingsV1::default());
    assert!(!migrated.global.updates.automatic_check);
    assert!(!migrated.global.updates.automatic_download);
    assert!(migrated.global.backup.enabled);
    assert_eq!(migrated.global.backup.interval_days, 14);
    assert!(migrated.global.notifications.sync_completed);

    let rewritten: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
    assert_eq!(rewritten["schemaVersion"], 7);
    assert_eq!(rewritten["global"]["updates"]["automaticCheck"], false);
    assert_eq!(rewritten["global"]["updates"]["automaticDownload"], false);
}

#[test]
fn automatic_update_download_requires_automatic_check() {
    let root = temp_root("automatic-update-settings-boundaries");
    let mut settings = load_portable_settings(&root).unwrap();
    settings.global.updates.automatic_download = true;

    assert_eq!(
        update_global_settings(&root, settings.global.clone()).unwrap_err(),
        "automatic_update_download_requires_check"
    );

    settings.global.updates.automatic_check = true;
    update_global_settings(&root, settings.global).unwrap();
}

#[test]
fn backup_and_notification_settings_validate_schedule_and_threshold_boundaries() {
    let root = temp_root("backup-notification-settings-boundaries");
    let mut settings = load_portable_settings(&root).unwrap();

    settings.global.backup.interval_days = 0;
    assert_eq!(
        update_global_settings(&root, settings.global.clone()).unwrap_err(),
        "invalid_backup_interval"
    );
    settings.global.backup.interval_days = 7;
    settings.global.backup.execution_time = "24:00".to_string();
    assert_eq!(
        update_global_settings(&root, settings.global.clone()).unwrap_err(),
        "invalid_backup_execution_time"
    );
    settings.global.backup.execution_time = "03:30".to_string();
    settings.global.backup.retention_count = 0;
    assert_eq!(
        update_global_settings(&root, settings.global.clone()).unwrap_err(),
        "invalid_backup_retention_count"
    );
    settings.global.backup.retention_count = 5;
    settings.global.notifications.automatic_failure_threshold = 0;
    assert_eq!(
        update_global_settings(&root, settings.global.clone()).unwrap_err(),
        "invalid_notification_failure_threshold"
    );
    settings.global.notifications.automatic_failure_threshold = 20;
    settings.global.notifications.message_duration_seconds = 0;
    assert_eq!(
        update_global_settings(&root, settings.global.clone()).unwrap_err(),
        "invalid_notification_message_duration"
    );
    settings.global.notifications.message_duration_seconds = 300;
    update_global_settings(&root, settings.global).unwrap();
}

#[test]
fn cover_cache_settings_validate_capacity_and_age_boundaries() {
    let root = temp_root("cover-cache-settings-boundaries");
    let mut settings = load_portable_settings(&root).unwrap();
    settings.global.cover_cache.enabled = true;
    settings.global.cover_cache.cleanup_by_capacity_enabled = true;
    settings.global.cover_cache.capacity_limit_mib = 0;
    assert_eq!(
        update_global_settings(&root, settings.global.clone()).unwrap_err(),
        "invalid_cover_cache_capacity"
    );

    settings.global.cover_cache.capacity_limit_mib = 1;
    settings.global.cover_cache.cleanup_by_age_enabled = true;
    settings.global.cover_cache.max_age_days = 0;
    assert_eq!(
        update_global_settings(&root, settings.global.clone()).unwrap_err(),
        "invalid_cover_cache_age"
    );

    settings.global.cover_cache.max_age_days = 1;
    update_global_settings(&root, settings.global).unwrap();
}

fn write_settings_json(root: &PathBuf, value: &serde_json::Value) -> Vec<u8> {
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    fs::write(root.join("settings.json"), &bytes).unwrap();
    bytes
}

fn load_v2_error(root: &PathBuf, value: &serde_json::Value) -> String {
    write_settings_json(root, value);
    load_portable_settings(root).unwrap_err()
}

fn assert_v2_loads(root: &PathBuf, value: &serde_json::Value) {
    write_settings_json(root, value);
    load_portable_settings(root).unwrap();
}

#[test]
fn backup_settings_bytes_are_decoded_without_touching_a_data_root() {
    let migrated = decode_portable_settings_bytes(
        &serde_json::to_vec(&valid_portable_v2_json()).unwrap(),
    )
    .unwrap();
    assert_eq!(migrated.schema_version, 7);
    assert_eq!(migrated.revision, 9);

    let current = PortableSettingsV2::default();
    assert_eq!(
        decode_portable_settings_bytes(&serde_json::to_vec(&current).unwrap()).unwrap(),
        current
    );

    let mut unsupported = valid_portable_v2_json();
    unsupported["schemaVersion"] = serde_json::json!(99);
    assert_eq!(
        decode_portable_settings_bytes(&serde_json::to_vec(&unsupported).unwrap()).unwrap_err(),
        "unsupported_portable_settings_schema"
    );
}

#[test]
fn portable_v1_is_atomically_migrated_to_latest_without_enabling_background_sync() {
    let root = temp_root("portable-v1-migration");
    let original = serde_json::json!({
        "schemaVersion": 1,
        "revision": 17,
        "legacyFrontendImport": {
            "completed": true,
            "fingerprint": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "global": {
            "likeCountFormat": "k",
            "detailFields": {
                "author": false,
                "watchDate": true,
                "likeCount": false,
                "watchProgress": true
            }
        },
        "accountDefaults": {
            "syncMode": "bottom",
            "syncDays": 31,
            "hideBrowser": true,
            "scrollSpeed": 4800
        },
        "accounts": {
            "sec:account-a": {
                "syncMode": "days",
                "syncDays": 45,
                "hideBrowser": false,
                "scrollSpeed": 7200
            }
        }
    });
    write_settings_json(&root, &original);

    let migrated = load_portable_settings(&root).unwrap();

    assert_eq!(migrated.schema_version, 7);
    assert_eq!(migrated.global.theme, ThemeModeV1::System);
    assert_eq!(migrated.revision, 17);
    assert_eq!(migrated.global.like_count_format, LikeCountFormatV1::K);
    assert!(!migrated.global.detail_fields.author);
    assert!(!migrated.global.detail_fields.like_count);
    assert_eq!(migrated.global.close_behavior, CloseBehaviorV1::Exit);
    assert_eq!(migrated.global.startup_sync_delay_minutes, 10);
    assert!(migrated.account_defaults.hide_browser);
    assert_eq!(migrated.account_defaults.scroll_speed, 4_800);
    assert_eq!(migrated.account_defaults.history.range, HistoryRangeV1::Bottom);
    assert!(!migrated.account_defaults.history.schedule.enabled);
    assert_eq!(migrated.account_defaults.history.schedule.interval_minutes, 360);
    assert_eq!(
        migrated.account_defaults.likes.range,
        ArchiveListRangeV1::RecentItems { count: 100 }
    );
    assert!(!migrated.account_defaults.likes.schedule.enabled);
    assert_eq!(
        migrated.account_defaults.favorites.remote_folder_mode,
        RemoteFolderScheduleModeV1::Unified
    );
    assert!(!migrated.account_defaults.favorites.archive.schedule.enabled);
    assert!(!migrated
        .account_defaults
        .favorites
        .remote_folder_defaults
        .schedule
        .enabled);
    assert!(migrated
        .account_defaults
        .favorites
        .remote_folders
        .is_empty());

    let account = migrated.accounts.get("sec:account-a").unwrap();
    assert_eq!(account.history.range, HistoryRangeV1::Days { days: 45 });
    assert_eq!(account.scroll_speed, 7_200);
    assert!(!account.hide_browser);
    assert!(!account.history.schedule.enabled);
    assert!(!account.likes.schedule.enabled);
    assert!(!account.favorites.archive.schedule.enabled);

    let persisted: PortableSettingsV2 =
        serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
    assert_eq!(persisted, migrated);
    assert_eq!(load_portable_settings(&root).unwrap(), migrated);

    let invalid_root = temp_root("portable-v1-invalid-preserved");
    let mut invalid = original;
    invalid["accountDefaults"]["syncDays"] = serde_json::json!(0);
    let invalid_bytes = write_settings_json(&invalid_root, &invalid);
    assert_eq!(
        load_portable_settings(&invalid_root).unwrap_err(),
        "invalid_account_settings"
    );
    assert_eq!(fs::read(invalid_root.join("settings.json")).unwrap(), invalid_bytes);
}

#[test]
fn portable_v2_rejects_unknown_fields_and_every_documented_range_boundary() {
    let root = temp_root("portable-v2-boundaries");
    let valid = valid_portable_v2_json();

    for boundary in [0_u32, 120] {
        let mut value = valid.clone();
        value["global"]["startupSyncDelayMinutes"] = serde_json::json!(boundary);
        assert_v2_loads(&root, &value);
    }
    for boundary in [15_u32, 43_200] {
        let mut value = valid.clone();
        value["accountDefaults"]["history"]["schedule"]["intervalMinutes"] =
            serde_json::json!(boundary);
        assert_v2_loads(&root, &value);
    }
    for boundary in [1_u32, 365] {
        let mut value = valid.clone();
        value["accountDefaults"]["history"]["range"] =
            serde_json::json!({ "kind": "days", "days": boundary });
        assert_v2_loads(&root, &value);
    }
    for (kind, field, boundaries) in [
        ("recentItems", "count", [1_u32, 5_000]),
        ("recordLimit", "limit", [1_u32, 50_000]),
        ("recentDays", "days", [1_u32, 365]),
    ] {
        for boundary in boundaries {
            let mut range = serde_json::Map::new();
            range.insert("kind".to_string(), serde_json::json!(kind));
            range.insert(field.to_string(), serde_json::json!(boundary));
            let mut value = valid.clone();
            value["accountDefaults"]["likes"]["range"] = serde_json::Value::Object(range);
            assert_v2_loads(&root, &value);
        }
    }
    for boundary in [120_u32, u32::MAX] {
        let mut value = valid.clone();
        value["accountDefaults"]["scrollSpeed"] = serde_json::json!(boundary);
        assert_v2_loads(&root, &value);
    }

    let mut unknown = valid.clone();
    unknown["global"]["extra"] = serde_json::json!(true);
    assert!(load_v2_error(&root, &unknown).starts_with("invalid_portable_settings:"));

    let mut invalid_startup_delay = valid.clone();
    invalid_startup_delay["global"]["startupSyncDelayMinutes"] = serde_json::json!(121);
    assert_eq!(
        load_v2_error(&root, &invalid_startup_delay),
        "invalid_startup_sync_delay_minutes"
    );

    for invalid in [14_u32, 43_201] {
        let mut value = valid.clone();
        value["accountDefaults"]["history"]["schedule"]["intervalMinutes"] =
            serde_json::json!(invalid);
        assert_eq!(load_v2_error(&root, &value), "invalid_source_schedule");
    }
    for invalid in [0_u32, 366] {
        let mut value = valid.clone();
        value["accountDefaults"]["history"]["range"] =
            serde_json::json!({ "kind": "days", "days": invalid });
        assert_eq!(load_v2_error(&root, &value), "invalid_history_range");
    }
    for (kind, field, invalid_values) in [
        ("recentItems", "count", [0_u32, 5_001]),
        ("recordLimit", "limit", [0_u32, 50_001]),
        ("recentDays", "days", [0_u32, 366]),
    ] {
        for invalid in invalid_values {
            let mut range = serde_json::Map::new();
            range.insert("kind".to_string(), serde_json::json!(kind));
            range.insert(field.to_string(), serde_json::json!(invalid));
            let mut value = valid.clone();
            value["accountDefaults"]["likes"]["range"] = serde_json::Value::Object(range);
            assert_eq!(load_v2_error(&root, &value), "invalid_archive_list_range");
        }
    }

    let mut invalid_scroll_speed = valid.clone();
    invalid_scroll_speed["accountDefaults"]["scrollSpeed"] = serde_json::json!(119);
    assert_eq!(
        load_v2_error(&root, &invalid_scroll_speed),
        "invalid_account_settings"
    );

    let mut unsafe_account = valid.clone();
    unsafe_account["accounts"] = serde_json::json!({
        "profile:device-local": valid["accountDefaults"].clone()
    });
    assert_eq!(
        load_v2_error(&root, &unsafe_account),
        "invalid_portable_account_key"
    );

    let mut invalid_folder = valid.clone();
    invalid_folder["accountDefaults"]["favorites"]["remoteFolders"] = serde_json::json!({
        " bad-folder ": valid["accountDefaults"]["favorites"]["remoteFolderDefaults"].clone()
    });
    assert_eq!(
        load_v2_error(&root, &invalid_folder),
        "invalid_remote_folder_id"
    );

    let mut folders = serde_json::Map::new();
    for index in 0..1_000 {
        folders.insert(
            format!("folder-{index}"),
            valid["accountDefaults"]["favorites"]["remoteFolderDefaults"].clone(),
        );
    }
    let mut maximum_folders = valid.clone();
    maximum_folders["accountDefaults"]["favorites"]["remoteFolders"] =
        serde_json::Value::Object(folders.clone());
    assert_v2_loads(&root, &maximum_folders);

    folders.insert(
        "folder-1000".to_string(),
        valid["accountDefaults"]["favorites"]["remoteFolderDefaults"].clone(),
    );
    let mut too_many_folders = valid.clone();
    too_many_folders["accountDefaults"]["favorites"]["remoteFolders"] =
        serde_json::Value::Object(folders);
    assert_eq!(
        load_v2_error(&root, &too_many_folders),
        "remote_folder_settings_limit_exceeded"
    );

    let original_bytes = write_settings_json(&root, &unknown);
    assert!(load_portable_settings(&root).is_err());
    assert_eq!(fs::read(root.join("settings.json")).unwrap(), original_bytes);
}

#[test]
fn contracts_reject_unknown_fields_relative_roots_and_invalid_account_keys() {
    let root = temp_root("contracts").join("Data");
    let bootstrap = BootstrapV1::new(root.clone(), root_id('a')).unwrap();
    assert_eq!(bootstrap.current_root.path, root);
    assert!(parse_bootstrap(
        r#"{"schemaVersion":1,"currentRoot":{"path":"relative","rootId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"activeAccountId":null,"retainedDataRoots":[]}"#,
    )
    .is_err());
    assert!(parse_bootstrap(
        r#"{"schemaVersion":1,"currentRoot":{"path":"C:\\Data","rootId":"short"},"activeAccountId":null,"retainedDataRoots":[]}"#,
    )
    .is_err());
    assert!(serde_json::from_str::<PortableSettingsV1>(
        r#"{"schemaVersion":1,"revision":0,"legacyFrontendImport":{"completed":false,"fingerprint":null},"global":{"likeCountFormat":"wan","detailFields":{"author":true,"watchDate":true,"likeCount":true,"watchProgress":true}},"accountDefaults":{"syncMode":"days","syncDays":7,"hideBrowser":false,"scrollSpeed":900},"accounts":{},"extra":true}"#,
    )
    .is_err());
    assert!(portable_account_key(None, None).is_err());
    assert!(portable_account_key(Some("  "), None).is_err());
}

#[test]
fn bootstrap_atomic_update_preserves_root_identity_and_retained_roots() {
    let bootstrap_dir = temp_root("bootstrap");
    let current_path = bootstrap_dir.join("Data");
    let retained_path = bootstrap_dir.join("Retained");
    let mut bootstrap = BootstrapV1::new(current_path.clone(), root_id('a')).unwrap();
    bootstrap.retained_data_roots.push(RetainedDataRootV1 {
        root: DataRootRefV1 {
            path: retained_path.clone(),
            root_id: root_id('b'),
        },
        layout: RetainedLayoutV1::Standard,
        legacy_entries: Vec::new(),
    });
    write_bootstrap_atomic(&bootstrap_dir, &bootstrap).unwrap();

    let mut loaded = read_bootstrap(&bootstrap_dir).unwrap().unwrap();
    loaded.active_account_id = Some(42);
    write_bootstrap_atomic(&bootstrap_dir, &loaded).unwrap();
    let reloaded = read_bootstrap(&bootstrap_dir).unwrap().unwrap();

    assert_eq!(reloaded.current_root.path, current_path);
    assert_eq!(reloaded.current_root.root_id, root_id('a'));
    assert_eq!(reloaded.retained_data_roots.len(), 1);
    assert_eq!(reloaded.retained_data_roots[0].root.path, retained_path);
    assert_eq!(reloaded.active_account_id, Some(42));
}

#[test]
fn data_root_marker_must_match_bootstrap_root_id() {
    let root = temp_root("marker");
    let marker = DataRootMarkerV1::new(root_id('a')).unwrap();
    write_data_root_marker_atomic(&root, &marker).unwrap();

    validate_data_root_marker(
        &root,
        &DataRootRefV1 {
            path: root.clone(),
            root_id: root_id('a'),
        },
    )
    .unwrap();
    assert_eq!(
        validate_data_root_marker(
            &root,
            &DataRootRefV1 {
                path: root.clone(),
                root_id: root_id('b'),
            },
        )
        .unwrap_err(),
        "data_root_identity_mismatch"
    );
}

#[test]
fn portable_settings_defaults_and_scope_updates_are_stable() {
    let root = temp_root("portable");
    let defaults = load_portable_settings(&root).unwrap();
    assert_eq!(defaults.global.like_count_format, LikeCountFormatV1::Wan);
    assert_eq!(
        defaults.account_defaults.history.range,
        HistoryRangeV1::Days { days: 7 }
    );
    assert_eq!(defaults.account_defaults.scroll_speed, 6_000);
    assert_eq!(
        defaults.account_defaults.likes.range,
        ArchiveListRangeV1::RecentItems { count: 100 }
    );
    assert_eq!(
        defaults.account_defaults.favorites.archive.range,
        ArchiveListRangeV1::RecentItems { count: 100 }
    );

    let account_key = portable_account_key(Some("uid-a"), Some("sec-a")).unwrap();
    assert_eq!(account_key, "sec:sec-a");
    let updated_account = AccountSettingsV2::from(AccountSettingsV1 {
        sync_mode: SyncModeV1::Bottom,
        sync_days: 30,
        hide_browser: true,
        scroll_speed: 5_000,
    });
    let after_account =
        update_account_settings(&root, &account_key, updated_account.clone()).unwrap();
    assert_eq!(
        after_account.accounts.get(&account_key),
        Some(&updated_account)
    );
    assert_eq!(after_account.global, defaults.global);

    let mut account_defaults = AccountSettingsV2::default();
    account_defaults.history.range = HistoryRangeV1::Days { days: 12 };
    let after_defaults = update_account_defaults(&root, account_defaults.clone()).unwrap();
    assert_eq!(after_defaults.account_defaults, account_defaults);
    assert_eq!(
        after_defaults.accounts.get(&account_key),
        Some(&updated_account)
    );
    assert!(after_defaults.revision > after_account.revision);

    let global = GlobalSettingsV2 {
        like_count_format: LikeCountFormatV1::K,
        detail_fields: DetailFieldsV1 {
            author: false,
            watch_date: true,
            like_count: true,
            watch_progress: false,
        },
        ..GlobalSettingsV2::default()
    };
    let after_global = update_global_settings(&root, global.clone()).unwrap();
    assert_eq!(after_global.global, global);
    assert_eq!(
        after_global.accounts.get(&account_key),
        Some(&updated_account)
    );
    assert!(after_global.revision > after_defaults.revision);
}

#[test]
fn legacy_import_is_idempotent_and_uses_defaults_without_an_account() {
    let root = temp_root("legacy");
    let legacy = LegacyFrontendSettingsV1 {
        sync_mode: SyncModeV1::Bottom,
        sync_days: 12,
        scroll_speed: 4_200,
        hide_browser: true,
        like_count_format: LikeCountFormatV1::W,
        detail_fields: DetailFieldsV1 {
            author: true,
            watch_date: false,
            like_count: true,
            watch_progress: false,
        },
    };

    let first = import_legacy_frontend_settings(&root, None, legacy.clone()).unwrap();
    let second = import_legacy_frontend_settings(&root, None, legacy.clone()).unwrap();
    assert_eq!(first, second);
    assert!(first.legacy_frontend_import.completed);
    assert!(first.legacy_frontend_import.fingerprint.is_some());
    assert_eq!(first.account_defaults.history.range, HistoryRangeV1::Bottom);
    assert!(first.accounts.is_empty());

    let conflicting = LegacyFrontendSettingsV1 {
        sync_days: 13,
        ..legacy
    };
    assert_eq!(
        import_legacy_frontend_settings(&root, None, conflicting).unwrap_err(),
        "legacy_settings_already_imported"
    );
}

#[test]
fn legacy_import_targets_the_stable_remote_account_key() {
    let root = temp_root("legacy-account");
    let key = portable_account_key(Some("uid-a"), Some("sec-a")).unwrap();
    let legacy = LegacyFrontendSettingsV1::default();
    let imported = import_legacy_frontend_settings(&root, Some(&key), legacy).unwrap();

    assert_eq!(
        imported.accounts.get(&key),
        Some(&imported.account_defaults)
    );
    assert!(!imported.accounts.contains_key("profile:device-local"));
}

#[test]
fn account_settings_fall_back_and_orphan_cleanup_is_idempotent() {
    let root = temp_root("account-cleanup");
    let first_key = portable_account_key(Some("uid-a"), Some("sec-a")).unwrap();
    let second_key = portable_account_key(Some("uid-b"), Some("sec-b")).unwrap();
    let first = AccountSettingsV2::from(AccountSettingsV1 {
        sync_mode: SyncModeV1::Bottom,
        sync_days: 14,
        hide_browser: true,
        scroll_speed: 4_000,
    });
    let second = AccountSettingsV2::from(AccountSettingsV1 {
        sync_mode: SyncModeV1::Days,
        sync_days: 21,
        hide_browser: false,
        scroll_speed: 3_000,
    });
    update_account_settings(&root, &first_key, first.clone()).unwrap();
    let before_cleanup = update_account_settings(&root, &second_key, second).unwrap();

    assert_eq!(
        account_settings_for(&before_cleanup, Some(&first_key)).unwrap(),
        first
    );
    assert_eq!(
        account_settings_for(&before_cleanup, None).unwrap(),
        before_cleanup.account_defaults
    );

    let known = BTreeSet::from([first_key.clone()]);
    let cleaned = retain_known_account_settings(&root, &known).unwrap();
    assert_eq!(cleaned.accounts.len(), 1);
    assert!(cleaned.accounts.contains_key(&first_key));
    assert!(cleaned.revision > before_cleanup.revision);

    let unchanged = retain_known_account_settings(&root, &known).unwrap();
    assert_eq!(unchanged.revision, cleaned.revision);

    let removed = remove_account_settings(&root, &first_key).unwrap();
    assert!(removed.accounts.is_empty());
    let removed_again = remove_account_settings(&root, &first_key).unwrap();
    assert_eq!(removed_again.revision, removed.revision);
}

#[test]
fn database_account_ids_resolve_to_remote_portable_keys() {
    let root = temp_root("account-keys");
    let database = root.join("history.sqlite");
    let mut connection = Connection::open(database).unwrap();
    initialize_schema_v4(&mut connection).unwrap();
    let now = "2026-08-11T00:00:00.000Z";
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'active', ?4, ?4)",
            params![
                "uid-a",
                "sec-a",
                "0123456789abcdef0123456789abcdef",
                now
            ],
        )
        .unwrap();
    let first_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO accounts
             (uid, profile_key, status, created_at, updated_at)
             VALUES (?1, ?2, 'archived', ?3, ?3)",
            params![
                "uid-b",
                "fedcba9876543210fedcba9876543210",
                now
            ],
        )
        .unwrap();
    let second_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO accounts
             (profile_key, status, created_at, updated_at)
             VALUES (?1, 'pending_login', ?2, ?2)",
            params!["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", now],
        )
        .unwrap();
    let pending_id = connection.last_insert_rowid();

    assert_eq!(
        portable_account_key_for_id(&connection, first_id).unwrap(),
        "sec:sec-a"
    );
    let account = list_accounts(&connection)
        .unwrap()
        .into_iter()
        .find(|account| account.id == first_id)
        .unwrap();
    assert_eq!(
        serde_json::to_value(account).unwrap()["accountKey"],
        "sec:sec-a"
    );
    assert_eq!(
        portable_account_key_for_id(&connection, second_id).unwrap(),
        "uid:uid-b"
    );
    assert_eq!(
        portable_account_key_for_id(&connection, pending_id).unwrap_err(),
        "portable_account_identity_missing"
    );
    assert_eq!(
        portable_account_key_for_id(&connection, 999).unwrap_err(),
        "account_not_found"
    );
    assert_eq!(
        known_portable_account_keys(&connection).unwrap(),
        BTreeSet::from(["sec:sec-a".to_string(), "uid:uid-b".to_string()])
    );
}

#[test]
fn active_account_is_stored_in_bootstrap_and_stale_ids_are_repaired() {
    let bootstrap_dir = temp_root("active-bootstrap");
    let data_root = bootstrap_dir.join("Data");
    fs::create_dir_all(&data_root).unwrap();
    let bootstrap = BootstrapV1::new(data_root.clone(), root_id('c')).unwrap();
    write_bootstrap_atomic(&bootstrap_dir, &bootstrap).unwrap();

    let mut connection = Connection::open(data_root.join("history.sqlite")).unwrap();
    initialize_schema_v4(&mut connection).unwrap();
    let now = "2026-08-11T00:00:00.000Z";
    connection
        .execute(
            "INSERT INTO accounts
             (uid, profile_key, status, created_at, updated_at)
             VALUES (?1, ?2, 'active', ?3, ?3)",
            params![
                "uid-active",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                now
            ],
        )
        .unwrap();
    let account_id = connection.last_insert_rowid();

    let selected = set_active_account(&bootstrap_dir, &connection, account_id).unwrap();
    assert_eq!(selected.id, account_id);
    assert_eq!(
        read_bootstrap(&bootstrap_dir)
            .unwrap()
            .unwrap()
            .active_account_id,
        Some(account_id)
    );
    assert_eq!(
        load_active_account(&bootstrap_dir, &connection)
            .unwrap()
            .unwrap()
            .id,
        account_id
    );

    connection
        .execute("DELETE FROM accounts WHERE id = ?1", [account_id])
        .unwrap();
    assert_eq!(load_active_account(&bootstrap_dir, &connection).unwrap(), None);
    let repaired = read_bootstrap(&bootstrap_dir).unwrap().unwrap();
    assert_eq!(repaired.active_account_id, None);
    assert_eq!(repaired.current_root, bootstrap.current_root);
}

#[test]
fn archiving_the_active_account_selects_a_bootstrap_fallback() {
    let bootstrap_dir = temp_root("archive-bootstrap");
    let data_root = bootstrap_dir.join("Data");
    fs::create_dir_all(&data_root).unwrap();
    write_bootstrap_atomic(
        &bootstrap_dir,
        &BootstrapV1::new(data_root.clone(), root_id('d')).unwrap(),
    )
    .unwrap();
    let mut connection = Connection::open(data_root.join("history.sqlite")).unwrap();
    initialize_schema_v4(&mut connection).unwrap();
    let now = "2026-08-11T00:00:00.000Z";
    for (uid, profile_key) in [
        ("uid-first", "cccccccccccccccccccccccccccccccc"),
        ("uid-second", "dddddddddddddddddddddddddddddddd"),
    ] {
        connection
            .execute(
                "INSERT INTO accounts
                 (uid, profile_key, status, created_at, updated_at)
                 VALUES (?1, ?2, 'active', ?3, ?3)",
                params![uid, profile_key, now],
            )
            .unwrap();
    }
    let first_id = 1;
    let second_id = 2;
    set_active_account(&bootstrap_dir, &connection, first_id).unwrap();

    archive_account(&mut connection, &bootstrap_dir, first_id).unwrap();

    assert_eq!(
        read_bootstrap(&bootstrap_dir)
            .unwrap()
            .unwrap()
            .active_account_id,
        Some(second_id)
    );
}

#[test]
fn app_settings_view_cleans_orphans_and_selects_the_requested_scope() {
    let root = temp_root("settings-view");
    let account_key = portable_account_key(Some("uid-view"), Some("sec-view")).unwrap();
    let orphan_key = portable_account_key(Some("uid-orphan"), Some("sec-orphan")).unwrap();
    let account = AccountSettingsV2::from(AccountSettingsV1 {
        sync_mode: SyncModeV1::Bottom,
        sync_days: 45,
        hide_browser: true,
        scroll_speed: 7_500,
    });
    update_account_settings(&root, &account_key, account.clone()).unwrap();
    update_account_settings(
        &root,
        &orphan_key,
        AccountSettingsV2::from(AccountSettingsV1 {
            sync_days: 99,
            ..AccountSettingsV1::default()
        }),
    )
    .unwrap();
    let retained = RetainedDataRootV1 {
        root: DataRootRefV1 {
            path: root.join("OldData"),
            root_id: root_id('e'),
        },
        layout: RetainedLayoutV1::Standard,
        legacy_entries: Vec::new(),
    };

    let view = load_app_settings_view(
        &root,
        Some(&account_key),
        &BTreeSet::from([account_key.clone()]),
        root.join("Data"),
        root.join("Install"),
        vec![retained.clone()],
    )
    .unwrap();

    assert_eq!(view.data_root, root.join("Data"));
    assert_eq!(view.install_dir, root.join("Install"));
    assert_eq!(view.retained_data_roots, vec![retained]);
    assert!(view.legacy_import_required);
    assert_eq!(view.account, account);
    assert_eq!(view.account_defaults, AccountSettingsV2::default());
    let persisted = load_portable_settings(&root).unwrap();
    assert_eq!(persisted.accounts.len(), 1);
    assert!(persisted.accounts.contains_key(&account_key));
    assert_eq!(view.revision, persisted.revision);
}

#[test]
fn deleting_an_account_clears_bootstrap_and_portable_settings() {
    let bootstrap_dir = temp_root("delete-account-state");
    let data_root = bootstrap_dir.join("Data");
    let profiles_root = data_root.join("profiles");
    fs::create_dir_all(&profiles_root).unwrap();
    write_bootstrap_atomic(
        &bootstrap_dir,
        &BootstrapV1::new(data_root.clone(), root_id('f')).unwrap(),
    )
    .unwrap();
    let mut connection = Connection::open(data_root.join("history.sqlite")).unwrap();
    initialize_schema_v4(&mut connection).unwrap();
    let now = "2026-08-11T00:00:00.000Z";
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'active', ?4, ?4)",
            params![
                "uid-delete",
                "sec-delete",
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                now
            ],
        )
        .unwrap();
    let account_id = connection.last_insert_rowid();
    let key = portable_account_key_for_id(&connection, account_id).unwrap();
    update_account_settings(&data_root, &key, AccountSettingsV2::default()).unwrap();
    set_active_account(&bootstrap_dir, &connection, account_id).unwrap();

    delete_account_data(
        &mut connection,
        &bootstrap_dir,
        &data_root,
        &profiles_root,
        account_id,
        AccountFolderDisposition::Delete,
    )
    .unwrap();

    assert_eq!(
        read_bootstrap(&bootstrap_dir)
            .unwrap()
            .unwrap()
            .active_account_id,
        None
    );
    assert!(load_portable_settings(&data_root).unwrap().accounts.is_empty());
}
