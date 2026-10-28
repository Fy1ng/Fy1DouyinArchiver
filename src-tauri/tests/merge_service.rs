use chrono::{DateTime, Utc};
use douyin_history_archive::{
    bootstrap::{read_bootstrap, write_bootstrap_atomic, BootstrapV1},
    create_application_backup,
    desktop_lifecycle::BackgroundTaskRegistry,
    migrate_schema_to_latest_at_startup, prepare_backup_merge, restore_backup_merge,
    runtime_paths::RuntimePaths,
    settings_store::{
        load_portable_settings, portable_account_key, write_portable_settings_atomic,
        HistoryRangeV1, PortableSettingsV2, ThemeModeV1,
    },
    sync::OperationAdmission,
    AppPaths, LocalFolderMergeResolutionV1, MergePlanRequestV1, MergeRestoreRequestV1,
    SettingValueSourceV1, SettingsMergeResolutionV1,
};
use rusqlite::{params, Connection};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn merge_plan_reports_folder_and_settings_conflicts_without_mutation() {
    let fixture = MergeFixture::new("plan");
    let before_database = fs::read(&fixture.target.database).unwrap();
    let before_settings = fs::read(fixture.target.data_root.join("settings.json")).unwrap();

    let plan = fixture.prepare_plan();

    assert_eq!(plan.counts.accounts, 2);
    assert_eq!(plan.counts.videos, 2);
    assert_eq!(plan.counts.watch_records, 2);
    assert_eq!(plan.local_folder_conflicts.len(), 1);
    assert_eq!(plan.local_folder_conflicts[0].name, "稍后再看");
    assert!(plan
        .settings_conflicts
        .iter()
        .any(|conflict| conflict.path == "global.theme"));
    assert!(plan.settings_conflicts.iter().any(|conflict| {
        conflict.path.ends_with(".scrollSpeed")
            && conflict.path.starts_with("accounts.sec:shared-sec")
    }));
    assert_eq!(fs::read(&fixture.target.database).unwrap(), before_database);
    assert_eq!(
        fs::read(fixture.target.data_root.join("settings.json")).unwrap(),
        before_settings
    );
    assert!(!fixture.target.backups.exists());
    assert_eq!(fixture.background.active_count().unwrap(), 0);
}

#[test]
fn merge_restore_deduplicates_archives_and_applies_explicit_conflict_resolutions() {
    let fixture = MergeFixture::new("success");
    let plan = fixture.prepare_plan();
    let folder_resolutions = plan
        .local_folder_conflicts
        .iter()
        .map(|conflict| (conflict.id.clone(), LocalFolderMergeResolutionV1::Merge))
        .collect::<BTreeMap<_, _>>();
    let field_choices = plan
        .settings_conflicts
        .iter()
        .map(|conflict| {
            let source = if conflict.path == "global.theme" || conflict.path.ends_with(".history") {
                SettingValueSourceV1::Backup
            } else {
                SettingValueSourceV1::Current
            };
            (conflict.id.clone(), source)
        })
        .collect::<BTreeMap<_, _>>();

    let result = fixture.restore(
        &plan,
        folder_resolutions,
        SettingsMergeResolutionV1::PerField {
            choices: field_choices,
        },
    );

    assert!(result.safety_backup_path.is_file());
    let connection = Connection::open(&fixture.target.database).unwrap();
    assert_eq!(count(&connection, "accounts"), 2);
    assert_eq!(count(&connection, "videos"), 3);
    assert_eq!(count(&connection, "watch_records"), 2);
    assert_eq!(count(&connection, "user_likes"), 1);
    assert_eq!(count(&connection, "favorite_items"), 1);
    assert_eq!(count(&connection, "remote_folders"), 1);
    assert_eq!(count(&connection, "remote_folder_items"), 2);
    assert_eq!(count(&connection, "local_folders"), 1);
    assert_eq!(count(&connection, "local_folder_items"), 2);
    assert_eq!(count(&connection, "local_folder_item_accounts"), 2);
    assert_eq!(count(&connection, "cover_cache_rules"), 2);
    let shared = connection
        .query_row(
            "SELECT profile_key, status, nickname, created_at, updated_at
             FROM accounts WHERE sec_uid = 'shared-sec'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(shared.0, "33333333333333333333333333333333");
    assert_eq!(shared.1, "active");
    assert_eq!(shared.2.as_deref(), Some("来源更新账号"));
    assert_eq!(shared.3, "2026-08-10T00:00:00.000Z");
    assert_eq!(shared.4, "2026-08-15T00:00:00.000Z");
    let imported = connection
        .query_row(
            "SELECT profile_key, status FROM accounts WHERE sec_uid = 'new-sec'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap();
    assert_ne!(imported.0, "44444444444444444444444444444444");
    assert_eq!(imported.1, "login_expired");
    let shared_video = connection
        .query_row(
            "SELECT title, author_name, cover_url, like_count, first_seen_at, last_seen_at
             FROM videos WHERE aweme_id = 'shared-video'",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(shared_video.0.as_deref(), Some("当前标题"));
    assert_eq!(shared_video.1.as_deref(), Some("来源作者"));
    assert_eq!(
        shared_video.2.as_deref(),
        Some("https://example.com/shared.jpg")
    );
    assert_eq!(shared_video.3, Some(900));
    assert_eq!(shared_video.4, "2026-08-10T00:00:00.000Z");
    assert_eq!(shared_video.5, "2026-08-15T00:00:00.000Z");
    assert_eq!(
        connection
            .query_row(
                "SELECT watch_progress_rate FROM watch_records wr
                 JOIN videos v ON v.id = wr.video_id
                 WHERE v.aweme_id = 'shared-video'",
                [],
                |row| row.get::<_, Option<f64>>(0),
            )
            .unwrap(),
        Some(0.8)
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT name FROM remote_folders WHERE remote_folder_id = 'shared-folder'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "来源收藏夹"
    );
    drop(connection);

    let settings = load_portable_settings(&fixture.target.data_root).unwrap();
    assert_eq!(settings.global.theme, ThemeModeV1::Light);
    let shared_key = portable_account_key(Some("shared-user"), Some("shared-sec")).unwrap();
    assert_eq!(settings.accounts[&shared_key].scroll_speed, 900);
    assert_eq!(
        settings.accounts[&shared_key].history.range,
        HistoryRangeV1::Days { days: 30 }
    );
    let new_key = portable_account_key(Some("new-user"), Some("new-sec")).unwrap();
    assert_eq!(settings.accounts[&new_key].scroll_speed, 1500);
    assert_eq!(fixture.background.active_count().unwrap(), 0);
    assert_eq!(
        fixture.admission.snapshot().unwrap().owned_operation_count,
        0
    );
}

#[test]
fn stale_or_unresolved_merge_plan_never_creates_a_safety_backup() {
    let unresolved = MergeFixture::new("unresolved");
    let unresolved_plan = unresolved.prepare_plan();
    assert_eq!(
        restore_backup_merge(MergeRestoreRequestV1 {
            runtime_paths: &unresolved.runtime,
            admission: &unresolved.admission,
            background_tasks: &unresolved.background,
            archive_path: &unresolved.archive,
            expected_plan: &unresolved_plan,
            folder_resolutions: &BTreeMap::new(),
            settings_resolution: &SettingsMergeResolutionV1::Current,
            now: utc("2026-08-15T09:00:00.000Z"),
            application_version: "0.1.0",
            wait_timeout: Duration::ZERO,
        })
        .unwrap_err(),
        "merge_folder_resolution_missing"
    );
    assert!(!unresolved.target.backups.exists());

    let stale = MergeFixture::new("stale");
    let stale_plan = stale.prepare_plan();
    let connection = Connection::open(&stale.target.database).unwrap();
    connection
        .execute(
            "INSERT INTO videos
             (aweme_id, canonical_url, content_type, first_seen_at, last_seen_at)
             VALUES ('late-video', 'https://www.douyin.com/video/late-video', 'video', ?1, ?1)",
            ["2026-08-15T08:30:00.000Z"],
        )
        .unwrap();
    drop(connection);
    let folder_resolutions = stale_plan
        .local_folder_conflicts
        .iter()
        .map(|conflict| (conflict.id.clone(), LocalFolderMergeResolutionV1::Merge))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        restore_backup_merge(MergeRestoreRequestV1 {
            runtime_paths: &stale.runtime,
            admission: &stale.admission,
            background_tasks: &stale.background,
            archive_path: &stale.archive,
            expected_plan: &stale_plan,
            folder_resolutions: &folder_resolutions,
            settings_resolution: &SettingsMergeResolutionV1::Current,
            now: utc("2026-08-15T09:00:00.000Z"),
            application_version: "0.1.0",
            wait_timeout: Duration::ZERO,
        })
        .unwrap_err(),
        "merge_plan_stale"
    );
    assert!(!stale.target.backups.exists());
    let connection = Connection::open(&stale.target.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM videos WHERE aweme_id = 'late-video'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn merge_restore_supports_folder_rename_and_skip() {
    let renamed = MergeFixture::new("rename");
    let renamed_plan = renamed.prepare_plan();
    let rename_resolutions = renamed_plan
        .local_folder_conflicts
        .iter()
        .map(|conflict| {
            (
                conflict.id.clone(),
                LocalFolderMergeResolutionV1::Rename {
                    name: "导入稍后再看".to_string(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    renamed.restore(
        &renamed_plan,
        rename_resolutions,
        SettingsMergeResolutionV1::Current,
    );
    let connection = Connection::open(&renamed.target.database).unwrap();
    assert_eq!(count(&connection, "local_folders"), 2);
    assert_eq!(count(&connection, "local_folder_items"), 2);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM local_folders WHERE name = '导入稍后再看'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    drop(connection);

    let skipped = MergeFixture::new("skip");
    let skipped_plan = skipped.prepare_plan();
    let skip_resolutions = skipped_plan
        .local_folder_conflicts
        .iter()
        .map(|conflict| (conflict.id.clone(), LocalFolderMergeResolutionV1::Skip))
        .collect::<BTreeMap<_, _>>();
    skipped.restore(
        &skipped_plan,
        skip_resolutions,
        SettingsMergeResolutionV1::Current,
    );
    let connection = Connection::open(&skipped.target.database).unwrap();
    assert_eq!(count(&connection, "local_folders"), 1);
    assert_eq!(count(&connection, "local_folder_items"), 1);
}

#[test]
fn merge_restore_supports_whole_settings_source_choices() {
    for (label, resolution, expected_theme, expected_speed, expected_days) in [
        (
            "settings-current",
            SettingsMergeResolutionV1::Current,
            ThemeModeV1::Dark,
            900,
            7,
        ),
        (
            "settings-backup",
            SettingsMergeResolutionV1::Backup,
            ThemeModeV1::Light,
            1200,
            30,
        ),
    ] {
        let fixture = MergeFixture::new(label);
        let plan = fixture.prepare_plan();
        let folder_resolutions = plan
            .local_folder_conflicts
            .iter()
            .map(|conflict| (conflict.id.clone(), LocalFolderMergeResolutionV1::Merge))
            .collect::<BTreeMap<_, _>>();
        fixture.restore(&plan, folder_resolutions, resolution);
        let settings = load_portable_settings(&fixture.target.data_root).unwrap();
        let shared_key = portable_account_key(Some("shared-user"), Some("shared-sec")).unwrap();
        assert_eq!(settings.global.theme, expected_theme);
        assert_eq!(settings.accounts[&shared_key].scroll_speed, expected_speed);
        assert_eq!(
            settings.accounts[&shared_key].history.range,
            HistoryRangeV1::Days {
                days: expected_days
            }
        );
        assert!(settings
            .accounts
            .contains_key(&portable_account_key(Some("new-user"), Some("new-sec")).unwrap()));
    }
}

#[test]
fn merge_normalizes_uid_only_current_account_to_the_enriched_portable_key() {
    let fixture = MergeFixture::new("identity-enrichment");
    let connection = Connection::open(&fixture.target.database).unwrap();
    connection
        .execute(
            "UPDATE accounts SET sec_uid = NULL WHERE uid = 'shared-user'",
            [],
        )
        .unwrap();
    drop(connection);
    let mut settings = load_portable_settings(&fixture.target.data_root).unwrap();
    let old_key = portable_account_key(Some("shared-user"), Some("shared-sec")).unwrap();
    let current = settings.accounts.remove(&old_key).unwrap();
    let uid_key = portable_account_key(Some("shared-user"), None).unwrap();
    settings.accounts.insert(uid_key.clone(), current);
    write_portable_settings_atomic(&fixture.target.data_root, &settings).unwrap();

    let plan = fixture.prepare_plan();
    assert_eq!(plan.local_folder_conflicts.len(), 1);
    assert!(plan
        .settings_conflicts
        .iter()
        .any(|conflict| conflict.path == "accounts.sec:shared-sec.scrollSpeed"));
    let folder_resolutions = plan
        .local_folder_conflicts
        .iter()
        .map(|conflict| (conflict.id.clone(), LocalFolderMergeResolutionV1::Merge))
        .collect::<BTreeMap<_, _>>();
    fixture.restore(
        &plan,
        folder_resolutions,
        SettingsMergeResolutionV1::Current,
    );

    let settings = load_portable_settings(&fixture.target.data_root).unwrap();
    assert!(!settings.accounts.contains_key(&uid_key));
    assert_eq!(settings.accounts[&old_key].scroll_speed, 900);
}

struct MergeFixture {
    root: PathBuf,
    target: AppPaths,
    archive: PathBuf,
    runtime: RuntimePaths,
    admission: OperationAdmission,
    background: BackgroundTaskRegistry,
}

impl MergeFixture {
    fn new(label: &str) -> Self {
        let root = temp_root(label);
        let source = prepare_data_root(&root.join("source"), 'a');
        let target = prepare_data_root(&root.join("target"), 'b');
        let archive = root.join("source.emily-backup");
        seed_source(&source, &archive);
        seed_target(&target);
        Self {
            runtime: RuntimePaths::new(target.clone()),
            admission: OperationAdmission::new(),
            background: BackgroundTaskRegistry::default(),
            root,
            target,
            archive,
        }
    }

    fn prepare_plan(&self) -> douyin_history_archive::MergePlanV1 {
        prepare_backup_merge(MergePlanRequestV1 {
            runtime_paths: &self.runtime,
            admission: &self.admission,
            background_tasks: &self.background,
            archive_path: &self.archive,
            wait_timeout: Duration::ZERO,
        })
        .unwrap()
    }

    fn restore(
        &self,
        plan: &douyin_history_archive::MergePlanV1,
        folder_resolutions: BTreeMap<String, LocalFolderMergeResolutionV1>,
        settings_resolution: SettingsMergeResolutionV1,
    ) -> douyin_history_archive::MergeRestoreResultV1 {
        restore_backup_merge(MergeRestoreRequestV1 {
            runtime_paths: &self.runtime,
            admission: &self.admission,
            background_tasks: &self.background,
            archive_path: &self.archive,
            expected_plan: plan,
            folder_resolutions: &folder_resolutions,
            settings_resolution: &settings_resolution,
            now: utc("2026-08-15T09:00:00.000Z"),
            application_version: "0.1.0",
            wait_timeout: Duration::ZERO,
        })
        .unwrap()
    }
}

impl Drop for MergeFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn seed_target(paths: &AppPaths) {
    let connection = Connection::open(&paths.database).unwrap();
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, nickname, status, created_at, updated_at,
              last_login_validated_at)
             VALUES ('shared-user', 'shared-sec', ?1, '当前账号', 'active', ?2, ?2, ?2)",
            params![
                "33333333333333333333333333333333",
                "2026-08-12T00:00:00.000Z"
            ],
        )
        .unwrap();
    let account_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)",
            [account_id],
        )
        .unwrap();
    insert_video(
        &connection,
        "shared-video",
        Some("当前标题"),
        None,
        None,
        Some(100),
        "2026-08-12T00:00:00.000Z",
        "2026-08-12T00:00:00.000Z",
    );
    let shared_video_id = connection.last_insert_rowid();
    insert_video(
        &connection,
        "current-video",
        Some("当前独有"),
        None,
        None,
        None,
        "2026-08-12T00:00:00.000Z",
        "2026-08-12T00:00:00.000Z",
    );
    connection
        .execute(
            "INSERT INTO watch_records
             (account_id, video_id, history_time_ms, watched_at, watched_date,
              source_position, watch_progress_rate, created_at, updated_at)
             VALUES (?1, ?2, 1786410000000, ?3, '2026-08-11', 2, 0.2, ?3, ?3)",
            params![account_id, shared_video_id, "2026-08-12T00:00:00.000Z"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO user_likes
             (account_id, video_id, first_seen_at, last_seen_at, source_position)
             VALUES (?1, ?2, ?3, ?3, 2)",
            params![account_id, shared_video_id, "2026-08-12T00:00:00.000Z"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO remote_folders
             (account_id, remote_folder_id, name, first_seen_at, last_seen_at)
             VALUES (?1, 'shared-folder', '当前收藏夹', ?2, ?2)",
            params![account_id, "2026-08-12T00:00:00.000Z"],
        )
        .unwrap();
    let remote_folder_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_items
             (folder_id, video_id, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![
                remote_folder_id,
                shared_video_id,
                "2026-08-12T00:00:00.000Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO local_folders(scope, owner_account_id, name, created_at, updated_at)
             VALUES ('account', ?1, '稍后再看', ?2, ?2)",
            params![account_id, "2026-08-12T00:00:00.000Z"],
        )
        .unwrap();
    let folder_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO local_folder_items(folder_id, video_id, added_at) VALUES (?1, ?2, ?3)",
            params![folder_id, shared_video_id, "2026-08-12T00:00:00.000Z"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO local_folder_item_accounts(folder_id, video_id, account_id, added_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                folder_id,
                shared_video_id,
                account_id,
                "2026-08-12T00:00:00.000Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cover_cache_rules(video_id, created_at) VALUES (?1, ?2)",
            params![shared_video_id, "2026-08-12T00:00:00.000Z"],
        )
        .unwrap();
    drop(connection);
    fs::create_dir_all(
        paths
            .account_root("33333333333333333333333333333333")
            .unwrap(),
    )
    .unwrap();
    let mut bootstrap = read_bootstrap(&paths.bootstrap_dir).unwrap().unwrap();
    bootstrap.active_account_id = Some(account_id);
    write_bootstrap_atomic(&paths.bootstrap_dir, &bootstrap).unwrap();
    let mut settings = PortableSettingsV2::default();
    settings.revision = 10;
    settings.global.theme = ThemeModeV1::Dark;
    let key = portable_account_key(Some("shared-user"), Some("shared-sec")).unwrap();
    let mut account = settings.account_defaults.clone();
    account.scroll_speed = 900;
    account.history.range = HistoryRangeV1::Days { days: 7 };
    settings.accounts.insert(key, account);
    write_portable_settings_atomic(&paths.data_root, &settings).unwrap();
}

fn seed_source(paths: &AppPaths, archive: &Path) {
    let connection = Connection::open(&paths.database).unwrap();
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, nickname, avatar_url, status, created_at, updated_at)
             VALUES ('shared-user', 'shared-sec', ?1, '来源更新账号',
                     'https://example.com/avatar.jpg', 'active', ?2, ?3)",
            params![
                "22222222222222222222222222222222",
                "2026-08-10T00:00:00.000Z",
                "2026-08-15T00:00:00.000Z"
            ],
        )
        .unwrap();
    let shared_account_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)",
            [shared_account_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, nickname, status, created_at, updated_at)
             VALUES ('new-user', 'new-sec', ?1, '新账号', 'active', ?2, ?2)",
            params![
                "44444444444444444444444444444444",
                "2026-08-14T00:00:00.000Z"
            ],
        )
        .unwrap();
    let new_account_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)",
            [new_account_id],
        )
        .unwrap();
    insert_video(
        &connection,
        "shared-video",
        None,
        Some("来源作者"),
        Some("https://example.com/shared.jpg"),
        Some(900),
        "2026-08-10T00:00:00.000Z",
        "2026-08-15T00:00:00.000Z",
    );
    let shared_video_id = connection.last_insert_rowid();
    insert_video(
        &connection,
        "imported-video",
        Some("来源独有"),
        Some("来源作者"),
        None,
        Some(10),
        "2026-08-14T00:00:00.000Z",
        "2026-08-14T00:00:00.000Z",
    );
    let imported_video_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO watch_records
             (account_id, video_id, history_time_ms, watched_at, watched_date,
              source_position, watch_progress_rate, created_at, updated_at)
             VALUES (?1, ?2, 1786410000000, ?3, '2026-08-11', 1, 0.8, ?4, ?3)",
            params![
                shared_account_id,
                shared_video_id,
                "2026-08-15T00:00:00.000Z",
                "2026-08-10T00:00:00.000Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO watch_records
             (account_id, video_id, history_time_ms, watched_at, watched_date,
              source_position, created_at, updated_at)
             VALUES (?1, ?2, 1786670000000, ?3, '2026-08-14', 1, ?3, ?3)",
            params![
                new_account_id,
                imported_video_id,
                "2026-08-14T00:00:00.000Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO user_likes
             (account_id, video_id, first_seen_at, last_seen_at, source_position)
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![
                shared_account_id,
                shared_video_id,
                "2026-08-10T00:00:00.000Z",
                "2026-08-15T00:00:00.000Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO favorite_items
             (account_id, video_id, first_seen_at, last_seen_at, source_position)
             VALUES (?1, ?2, ?3, ?3, 1)",
            params![
                new_account_id,
                imported_video_id,
                "2026-08-14T00:00:00.000Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO remote_folders
             (account_id, remote_folder_id, name, first_seen_at, last_seen_at)
             VALUES (?1, 'shared-folder', '来源收藏夹', ?2, ?3)",
            params![
                shared_account_id,
                "2026-08-10T00:00:00.000Z",
                "2026-08-15T00:00:00.000Z"
            ],
        )
        .unwrap();
    let remote_folder_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO remote_folder_items
             (folder_id, video_id, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![
                remote_folder_id,
                imported_video_id,
                "2026-08-14T00:00:00.000Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO local_folders(scope, owner_account_id, name, created_at, updated_at)
             VALUES ('account', ?1, '稍后再看', ?2, ?3)",
            params![
                shared_account_id,
                "2026-08-10T00:00:00.000Z",
                "2026-08-15T00:00:00.000Z"
            ],
        )
        .unwrap();
    let folder_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO local_folder_items(folder_id, video_id, added_at) VALUES (?1, ?2, ?3)",
            params![folder_id, imported_video_id, "2026-08-14T00:00:00.000Z"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO local_folder_item_accounts(folder_id, video_id, account_id, added_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                folder_id,
                imported_video_id,
                shared_account_id,
                "2026-08-14T00:00:00.000Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cover_cache_rules(video_id, created_at) VALUES (?1, ?2)",
            params![imported_video_id, "2026-08-14T00:00:00.000Z"],
        )
        .unwrap();
    drop(connection);
    let mut settings = PortableSettingsV2::default();
    settings.revision = 5;
    settings.global.theme = ThemeModeV1::Light;
    let shared_key = portable_account_key(Some("shared-user"), Some("shared-sec")).unwrap();
    let mut shared = settings.account_defaults.clone();
    shared.scroll_speed = 1200;
    shared.history.range = HistoryRangeV1::Days { days: 30 };
    settings.accounts.insert(shared_key, shared);
    let new_key = portable_account_key(Some("new-user"), Some("new-sec")).unwrap();
    let mut new_account = settings.account_defaults.clone();
    new_account.scroll_speed = 1500;
    settings.accounts.insert(new_key, new_account);
    write_portable_settings_atomic(&paths.data_root, &settings).unwrap();
    create_application_backup(
        &paths.database,
        &settings,
        archive,
        "2026-08-15T07:00:00.000Z",
        "0.1.0",
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn insert_video(
    connection: &Connection,
    aweme_id: &str,
    title: Option<&str>,
    author: Option<&str>,
    cover: Option<&str>,
    like_count: Option<i64>,
    first_seen_at: &str,
    last_seen_at: &str,
) {
    connection
        .execute(
            "INSERT INTO videos
             (aweme_id, canonical_url, content_type, title, author_name, cover_url,
              like_count, first_seen_at, last_seen_at)
             VALUES (?1, ?2, 'video', ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                aweme_id,
                format!("https://www.douyin.com/video/{aweme_id}"),
                title,
                author,
                cover,
                like_count,
                first_seen_at,
                last_seen_at
            ],
        )
        .unwrap();
}

fn prepare_data_root(root: &Path, root_character: char) -> AppPaths {
    let paths = AppPaths::from_roots(
        root.join("system"),
        root.join("data"),
        root.join("resources"),
    );
    fs::create_dir_all(&paths.data_root).unwrap();
    fs::create_dir_all(&paths.bootstrap_dir).unwrap();
    let mut connection = Connection::open(&paths.database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    drop(connection);
    let bootstrap = BootstrapV1::new(
        paths.data_root.clone(),
        std::iter::repeat_n(root_character, 32).collect(),
    )
    .unwrap();
    write_bootstrap_atomic(&paths.bootstrap_dir, &bootstrap).unwrap();
    write_portable_settings_atomic(&paths.data_root, &PortableSettingsV2::default()).unwrap();
    paths
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "em-merge-{label}-{:x}-{nonce:x}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
