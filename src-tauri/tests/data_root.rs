use douyin_history_archive::bootstrap::{
    read_bootstrap, write_bootstrap_atomic, write_data_root_marker_atomic, BootstrapV1,
    DataRootMarkerV1, DataRootRefV1, RetainedDataRootV1, RetainedLayoutV1,
};
use douyin_history_archive::collector::browser_state::{
    browser_profile_path, read_browser_selection, read_login_validation,
    rewrite_browser_state_without_profile_paths, write_browser_selection, write_login_validation,
    BrowserId, BrowserSelection, LoginValidation,
};
use douyin_history_archive::data_root::{
    build_data_inventory, delete_retained_data_root, migrate_data_root, normalize_requested_root,
    probe_data_root_permissions, probe_data_root_permissions_with_observer, read_migration_journal,
    recover_interrupted_data_root_migration, resolve_or_initialize_data_root,
    uninstall_managed_data, validate_data_root_request, verify_data_inventory,
    write_migration_journal_atomic, DataRootContext, MigrationFaultPoint, MigrationHooks,
    MigrationJournalV1, MigrationPhaseV1, NormalizedDataRoot,
};
use douyin_history_archive::{runtime_paths::RuntimePaths, AppPaths};
use rusqlite::Connection;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-data-root-{label}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    canonical_display(&root)
}

fn context(root: &Path) -> DataRootContext {
    let current = root.join("current");
    let install = root.join("install");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&install).unwrap();
    DataRootContext {
        app_data_dir: current,
        install_dir: install,
    }
}

fn canonical_display(path: &Path) -> PathBuf {
    let canonical = fs::canonicalize(path).unwrap();
    let raw = canonical.to_string_lossy();
    PathBuf::from(raw.strip_prefix(r"\\?\").unwrap_or(&raw))
}

fn release_normalized_target(normalized: &douyin_history_archive::data_root::NormalizedDataRoot) {
    normalized.release_target_ownership().unwrap();
}

#[test]
fn validation_request_probes_without_retaining_a_target_shell() {
    let root = temp_root("validation-view");
    let context = context(&root);
    let expected = context.install_dir.join("Data");
    let expected_normalized = canonical_display(&context.install_dir).join("Data");

    let view = validate_data_root_request(&context, &context.install_dir).unwrap();

    assert_eq!(view.requested_path, context.install_dir);
    assert_eq!(view.normalized_path, expected_normalized);
    assert!(view.adjusted_to_install_data);
    assert!(!expected.exists());
    fs::remove_dir_all(root).unwrap();
}

fn standard_root(root: &Path) -> PathBuf {
    let data = root.join("source");
    fs::create_dir_all(data.join("covers")).unwrap();
    write_data_root_marker_atomic(
        &data,
        &DataRootMarkerV1::new("0123456789abcdef0123456789abcdef".into()).unwrap(),
    )
    .unwrap();
    fs::write(data.join("history.sqlite"), b"database-v1").unwrap();
    fs::write(data.join("settings.json"), br#"{"schemaVersion":1}"#).unwrap();
    fs::write(data.join("covers/cache.bin"), b"cache-v1").unwrap();
    let account_root = data
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let profile = browser_profile_path(&account_root, BrowserId::Edge);
    fs::create_dir_all(&profile).unwrap();
    let selection = BrowserSelection {
        schema_version: 1,
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files\Edge\msedge.exe".into(),
        profile_path: profile.to_string_lossy().into_owned(),
        profile_generation: None,
        selected_at: "2026-08-11T00:00:00.000Z".into(),
    };
    write_browser_selection(&account_root, &selection).unwrap();
    write_login_validation(
        &account_root,
        &selection,
        &LoginValidation {
            schema_version: 2,
            validation_id: Some("fedcba9876543210fedcba9876543210".into()),
            browser_id: BrowserId::Edge,
            executable_path: selection.executable_path.clone(),
            profile_path: selection.profile_path.clone(),
            profile_generation: None,
            validated_at: "2026-08-11T00:01:00.000Z".into(),
            history_url: "https://www.douyin.com/user/self?showTab=record".into(),
        },
    )
    .unwrap();
    data
}

struct MigrationFixture {
    root: PathBuf,
    bootstrap_dir: PathBuf,
    source: DataRootRefV1,
    target: NormalizedDataRoot,
    resource_dir: PathBuf,
}

fn migration_fixture(label: &str) -> MigrationFixture {
    let root = temp_root(label);
    let source_path = standard_root(&root);
    fs::remove_file(source_path.join("history.sqlite")).unwrap();
    let database = Connection::open(source_path.join("history.sqlite")).unwrap();
    database
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE records(id INTEGER PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO records(value) VALUES ('source-record');",
        )
        .unwrap();
    drop(database);

    let source = DataRootRefV1 {
        path: canonical_display(&source_path),
        root_id: "0123456789abcdef0123456789abcdef".into(),
    };
    let bootstrap_dir = root.join("bootstrap");
    fs::create_dir(&bootstrap_dir).unwrap();
    write_bootstrap_atomic(
        &bootstrap_dir,
        &BootstrapV1::new(source.path.clone(), source.root_id.clone()).unwrap(),
    )
    .unwrap();
    let install_dir = root.join("install");
    fs::create_dir(&install_dir).unwrap();
    let target_path = root.join("target");
    fs::create_dir(&target_path).unwrap();
    let target = normalize_requested_root(
        &DataRootContext {
            app_data_dir: source.path.clone(),
            install_dir,
        },
        &target_path,
    )
    .unwrap();
    let resource_dir = root.join("resources");
    fs::create_dir(&resource_dir).unwrap();
    MigrationFixture {
        root,
        bootstrap_dir,
        source,
        target,
        resource_dir,
    }
}

#[test]
fn inventory_hashes_payload_and_browser_state_semantics() {
    let root = temp_root("inventory");
    let source = standard_root(&root);
    let before = build_data_inventory(&source).unwrap();

    assert_eq!(before.file_count, 3);
    assert_eq!(
        before.total_bytes,
        b"database-v1".len() as u64
            + br#"{"schemaVersion":1}"#.len() as u64
            + b"cache-v1".len() as u64
    );
    assert_eq!(
        before.critical_hashes.keys().cloned().collect::<Vec<_>>(),
        vec!["history.sqlite".to_string(), "settings.json".to_string()]
    );
    assert_eq!(before.semantic_browser_hashes.len(), 1);

    let account_root = source
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    rewrite_browser_state_without_profile_paths(&account_root).unwrap();
    let rewritten = build_data_inventory(&source).unwrap();
    assert_eq!(rewritten, before);

    fs::write(source.join("covers/cache.bin"), b"cache-v1-expanded").unwrap();
    let changed = build_data_inventory(&source).unwrap();
    assert_ne!(changed.total_bytes, before.total_bytes);
    assert_eq!(changed.critical_hashes, before.critical_hashes);
    assert_eq!(
        changed.semantic_browser_hashes,
        before.semantic_browser_hashes
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn inventory_verification_rejects_payload_size_and_critical_hash_changes() {
    let root = temp_root("inventory-verification");
    let source = standard_root(&root);
    let expected = build_data_inventory(&source).unwrap();

    verify_data_inventory(&source, &expected).unwrap();
    fs::write(source.join("covers/cache.bin"), b"cache-v1-expanded").unwrap();
    assert_eq!(
        verify_data_inventory(&source, &expected)
            .unwrap_err()
            .code(),
        "data_root_verification_failed"
    );

    fs::write(source.join("covers/cache.bin"), b"cache-v1").unwrap();
    fs::write(source.join("history.sqlite"), b"Database-v1").unwrap();
    assert_eq!(
        verify_data_inventory(&source, &expected)
            .unwrap_err()
            .code(),
        "data_root_verification_failed"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn migration_journal_round_trips_and_rejects_unknown_fields() {
    let root = temp_root("migration-journal");
    let bootstrap_dir = root.join("bootstrap");
    fs::create_dir(&bootstrap_dir).unwrap();
    let source = standard_root(&root);
    let target = root.join("target");
    let staging = root.join(".emily-migration-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.staging");
    let journal = MigrationJournalV1 {
        schema_version: 1,
        migration_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        source: DataRootRefV1 {
            path: source.clone(),
            root_id: "0123456789abcdef0123456789abcdef".into(),
        },
        target: DataRootRefV1 {
            path: target,
            root_id: "fedcba9876543210fedcba9876543210".into(),
        },
        staging,
        target_token: "11111111111111111111111111111111".into(),
        phase: MigrationPhaseV1::Prepared,
        source_inventory: build_data_inventory(&source).unwrap(),
    };

    write_migration_journal_atomic(&bootstrap_dir, &journal).unwrap();
    assert_eq!(
        read_migration_journal(&bootstrap_dir).unwrap(),
        Some(journal)
    );

    let journal_path = bootstrap_dir.join("data-root-migration.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    value["extra"] = serde_json::json!(true);
    fs::write(&journal_path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        read_migration_journal(&bootstrap_dir).unwrap_err().code(),
        "data_root_migration_journal_invalid"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn migration_copies_verifies_and_commits_the_new_root() {
    let fixture = migration_fixture("migration-success");
    let source_inventory = build_data_inventory(&fixture.source.path).unwrap();
    let result = migrate_data_root(
        &fixture.source,
        &fixture.target,
        &fixture.bootstrap_dir,
        &MigrationHooks::default(),
    )
    .unwrap();

    assert_eq!(result.bootstrap.current_root.path, fixture.target.path);
    assert_eq!(result.bootstrap.retained_data_roots.len(), 1);
    assert_eq!(result.bootstrap.retained_data_roots[0].root, fixture.source);
    assert_eq!(
        result.bootstrap.retained_data_roots[0].layout,
        RetainedLayoutV1::Standard
    );
    assert_eq!(
        read_bootstrap(&fixture.bootstrap_dir)
            .unwrap()
            .unwrap()
            .current_root,
        result.bootstrap.current_root
    );
    verify_data_inventory(&fixture.target.path, &source_inventory).unwrap();
    assert!(read_migration_journal(&fixture.bootstrap_dir)
        .unwrap()
        .is_none());

    assert!(!fixture.source.path.join("history.sqlite-wal").exists());
    assert!(!fixture.source.path.join("history.sqlite-shm").exists());
    let retained_database = Connection::open(fixture.source.path.join("history.sqlite")).unwrap();
    let retained_journal_mode: String = retained_database
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retained_journal_mode.to_ascii_lowercase(), "wal");
    drop(retained_database);

    let database = Connection::open(fixture.target.path.join("history.sqlite")).unwrap();
    assert_eq!(
        database
            .query_row("SELECT value FROM records", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "source-record"
    );
    drop(database);
    let account_root = fixture
        .target
        .path
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let selection = read_browser_selection(&account_root).unwrap().unwrap();
    assert_eq!(selection.schema_version, 3);
    assert!(selection.profile_path.is_empty());
    assert!(selection.profile_generation.is_some());
    let validation = read_login_validation(&account_root, &selection)
        .unwrap()
        .unwrap();
    assert_eq!(validation.schema_version, 4);
    assert!(validation.profile_path.is_empty());
    fs::remove_dir_all(fixture.root).unwrap();
}

#[cfg(windows)]
#[test]
fn migration_accepts_a_clean_source_shm_held_without_delete_sharing() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;

    let fixture = migration_fixture("migration-clean-locked-shm");
    let database_path = fixture.source.path.join("history.sqlite");
    let database = Connection::open(&database_path).unwrap();
    database
        .query_row("SELECT value FROM records LIMIT 1", [], |row| row.get::<_, String>(0))
        .unwrap();
    let shm_path = fixture.source.path.join("history.sqlite-shm");
    assert!(shm_path.is_file());
    let held_shm = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&shm_path)
        .unwrap();
    drop(database);

    let result = migrate_data_root(
        &fixture.source,
        &fixture.target,
        &fixture.bootstrap_dir,
        &MigrationHooks::default(),
    )
    .unwrap();

    assert_eq!(result.bootstrap.current_root.path, fixture.target.path);
    assert!(shm_path.is_file());
    assert!(!fixture.target.path.join("history.sqlite-shm").exists());
    assert!(!fixture.target.path.join("history.sqlite-wal").exists());

    drop(held_shm);
    fs::remove_dir_all(fixture.root).unwrap();
}

#[cfg(windows)]
#[test]
fn migration_accepts_a_clean_source_wal_held_without_delete_sharing() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;

    let fixture = migration_fixture("migration-clean-locked-wal");
    let database_path = fixture.source.path.join("history.sqlite");
    let database = Connection::open(&database_path).unwrap();
    database
        .query_row("SELECT value FROM records LIMIT 1", [], |row| row.get::<_, String>(0))
        .unwrap();
    let (busy, _, _): (i64, i64, i64) = database
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(busy, 0);
    let wal_path = fixture.source.path.join("history.sqlite-wal");
    assert_eq!(fs::metadata(&wal_path).unwrap().len(), 0);
    let held_wal = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&wal_path)
        .unwrap();

    let result = migrate_data_root(
        &fixture.source,
        &fixture.target,
        &fixture.bootstrap_dir,
        &MigrationHooks::default(),
    )
    .unwrap();

    assert_eq!(result.bootstrap.current_root.path, fixture.target.path);
    assert!(wal_path.is_file());
    assert!(!fixture.target.path.join("history.sqlite-shm").exists());
    assert!(!fixture.target.path.join("history.sqlite-wal").exists());
    assert_eq!(
        database
            .query_row("SELECT value FROM records LIMIT 1", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "source-record"
    );

    drop(held_wal);
    drop(database);
    fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn migration_faults_before_bootstrap_commit_keep_runtime_on_source() {
    let points = [
        MigrationFaultPoint::BeforeJournalWrite,
        MigrationFaultPoint::DuringCopy,
        MigrationFaultPoint::DuringBrowserStateRewrite,
        MigrationFaultPoint::CheckpointBusy,
        MigrationFaultPoint::BeforeVerification,
        MigrationFaultPoint::AfterTargetShellRemoved,
        MigrationFaultPoint::AfterStagingRename,
        MigrationFaultPoint::BeforeBootstrapWrite,
    ];
    for point in points {
        let fixture = migration_fixture(&format!("migration-fault-{point:?}"));
        let old_paths = AppPaths::from_roots(
            fixture.bootstrap_dir.clone(),
            fixture.source.path.clone(),
            fixture.resource_dir.clone(),
        );
        let runtime = RuntimePaths::new(old_paths.clone());
        let maintenance = runtime.begin_maintenance().unwrap();
        let error = migrate_data_root(
            &fixture.source,
            &fixture.target,
            &fixture.bootstrap_dir,
            &MigrationHooks::interrupt_at(point),
        )
        .unwrap_err();
        assert!(matches!(
            error.code(),
            "data_root_migration_interrupted" | "data_root_checkpoint_busy"
        ));
        let bootstrap = read_bootstrap(&fixture.bootstrap_dir).unwrap().unwrap();
        assert_eq!(bootstrap.current_root, fixture.source, "fault {point:?}");
        assert_eq!(
            runtime.acquire().unwrap_err(),
            "data_root_migration_in_progress"
        );
        drop(maintenance);
        assert_eq!(runtime.acquire().unwrap().data_root, old_paths.data_root);

        let recovered =
            recover_interrupted_data_root_migration(&fixture.bootstrap_dir, &bootstrap).unwrap();
        if matches!(
            point,
            MigrationFaultPoint::AfterTargetShellRemoved
                | MigrationFaultPoint::AfterStagingRename
                | MigrationFaultPoint::BeforeBootstrapWrite
        ) {
            assert_eq!(recovered.current_root.path, fixture.target.path);
            assert!(recovered.current_root.path.join("history.sqlite").is_file());
        } else {
            assert_eq!(recovered.current_root, fixture.source);
            assert!(!fixture.target.path.join("history.sqlite").exists());
        }
        fs::remove_dir_all(fixture.root).unwrap();
    }
}

#[test]
fn migration_fault_after_bootstrap_write_can_commit_runtime_after_recovery() {
    let fixture = migration_fixture("migration-post-bootstrap-fault");
    let old_paths = AppPaths::from_roots(
        fixture.bootstrap_dir.clone(),
        fixture.source.path.clone(),
        fixture.resource_dir.clone(),
    );
    let runtime = RuntimePaths::new(old_paths);
    let maintenance = runtime.begin_maintenance().unwrap();
    assert_eq!(
        migrate_data_root(
            &fixture.source,
            &fixture.target,
            &fixture.bootstrap_dir,
            &MigrationHooks::interrupt_at(MigrationFaultPoint::AfterBootstrapWrite),
        )
        .unwrap_err()
        .code(),
        "data_root_migration_interrupted"
    );
    let bootstrap = read_bootstrap(&fixture.bootstrap_dir).unwrap().unwrap();
    assert_eq!(bootstrap.current_root.path, fixture.target.path);
    assert_eq!(
        runtime.acquire().unwrap_err(),
        "data_root_migration_in_progress"
    );

    let recovered =
        recover_interrupted_data_root_migration(&fixture.bootstrap_dir, &bootstrap).unwrap();
    let new_paths = AppPaths::from_roots(
        fixture.bootstrap_dir.clone(),
        recovered.current_root.path.clone(),
        fixture.resource_dir.clone(),
    );
    maintenance.commit(new_paths.clone()).unwrap();
    assert_eq!(runtime.acquire().unwrap().data_root, new_paths.data_root);
    assert!(read_migration_journal(&fixture.bootstrap_dir)
        .unwrap()
        .is_none());
    fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn retained_standard_root_deletion_requires_exact_identity_and_never_deletes_current() {
    let fixture = migration_fixture("retained-standard");
    let result = migrate_data_root(
        &fixture.source,
        &fixture.target,
        &fixture.bootstrap_dir,
        &MigrationHooks::default(),
    )
    .unwrap();
    assert_eq!(
        delete_retained_data_root(&fixture.bootstrap_dir, &result.bootstrap.current_root)
            .unwrap_err()
            .code(),
        "data_root_is_current"
    );

    let mismatched = DataRootRefV1 {
        path: fixture.source.path.clone(),
        root_id: "11111111111111111111111111111111".into(),
    };
    assert_eq!(
        delete_retained_data_root(&fixture.bootstrap_dir, &mismatched)
            .unwrap_err()
            .code(),
        "data_root_not_retained"
    );

    write_data_root_marker_atomic(
        &fixture.source.path,
        &DataRootMarkerV1::new("22222222222222222222222222222222".into()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        delete_retained_data_root(&fixture.bootstrap_dir, &fixture.source)
            .unwrap_err()
            .code(),
        "data_root_identity_mismatch"
    );
    assert!(fixture.source.path.join("history.sqlite").is_file());
    write_data_root_marker_atomic(
        &fixture.source.path,
        &DataRootMarkerV1::new(fixture.source.root_id.clone()).unwrap(),
    )
    .unwrap();

    let updated = delete_retained_data_root(&fixture.bootstrap_dir, &fixture.source).unwrap();
    assert!(updated.retained_data_roots.is_empty());
    assert!(!fixture.source.path.exists());
    assert!(updated.current_root.path.join("history.sqlite").is_file());
    fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn uninstall_managed_data_deletes_current_and_retained_roots_after_identity_checks() {
    let fixture = migration_fixture("uninstall-managed-data");
    let result = migrate_data_root(
        &fixture.source,
        &fixture.target,
        &fixture.bootstrap_dir,
        &MigrationHooks::default(),
    )
    .unwrap();
    let current = result.bootstrap.current_root.path.clone();
    let retained = result.bootstrap.retained_data_roots[0].root.path.clone();

    uninstall_managed_data(&fixture.bootstrap_dir).unwrap();

    assert!(!current.exists());
    assert!(!retained.exists());
    assert!(read_bootstrap(&fixture.bootstrap_dir).unwrap().is_none());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn uninstall_managed_data_rejects_a_changed_current_marker_before_deleting_anything() {
    let fixture = migration_fixture("uninstall-current-mismatch");
    write_data_root_marker_atomic(
        &fixture.source.path,
        &DataRootMarkerV1::new("fedcba9876543210fedcba9876543210".into()).unwrap(),
    )
    .unwrap();

    assert_eq!(
        uninstall_managed_data(&fixture.bootstrap_dir)
            .unwrap_err()
            .code(),
        "data_root_identity_mismatch"
    );
    assert!(fixture.source.path.exists());
    assert!(read_bootstrap(&fixture.bootstrap_dir).unwrap().is_some());
    release_normalized_target(&fixture.target);
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn retained_legacy_flat_deletion_rejects_unlisted_entries() {
    let root = temp_root("retained-legacy");
    let bootstrap_dir = root.join("bootstrap");
    let current = root.join("current");
    let retained = root.join("legacy-retained");
    fs::create_dir(&bootstrap_dir).unwrap();
    fs::create_dir(&current).unwrap();
    fs::create_dir_all(retained.join("profiles")).unwrap();
    fs::write(current.join("history.sqlite"), b"current").unwrap();
    fs::write(retained.join("history.sqlite"), b"legacy").unwrap();
    fs::write(retained.join("profiles/profile.bin"), b"profile").unwrap();
    fs::write(retained.join("unexpected.txt"), b"keep").unwrap();
    let current_ref = DataRootRefV1 {
        path: canonical_display(&current),
        root_id: "33333333333333333333333333333333".into(),
    };
    let retained_ref = DataRootRefV1 {
        path: canonical_display(&retained),
        root_id: "44444444444444444444444444444444".into(),
    };
    write_data_root_marker_atomic(
        &current_ref.path,
        &DataRootMarkerV1::new(current_ref.root_id.clone()).unwrap(),
    )
    .unwrap();
    write_data_root_marker_atomic(
        &retained_ref.path,
        &DataRootMarkerV1::new(retained_ref.root_id.clone()).unwrap(),
    )
    .unwrap();
    let mut bootstrap =
        BootstrapV1::new(current_ref.path.clone(), current_ref.root_id.clone()).unwrap();
    bootstrap.retained_data_roots.push(RetainedDataRootV1 {
        root: retained_ref.clone(),
        layout: RetainedLayoutV1::LegacyFlat,
        legacy_entries: vec!["history.sqlite".into(), "profiles".into()],
    });
    write_bootstrap_atomic(&bootstrap_dir, &bootstrap).unwrap();

    assert_eq!(
        delete_retained_data_root(&bootstrap_dir, &retained_ref)
            .unwrap_err()
            .code(),
        "data_root_retained_root_changed"
    );
    assert!(retained.join("unexpected.txt").is_file());
    fs::remove_file(retained.join("unexpected.txt")).unwrap();
    let updated = delete_retained_data_root(&bootstrap_dir, &retained_ref).unwrap();
    assert!(updated.retained_data_roots.is_empty());
    assert!(!retained.exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn retained_root_deletion_rejects_internal_junctions() {
    use std::process::Command;

    let fixture = migration_fixture("retained-junction");
    migrate_data_root(
        &fixture.source,
        &fixture.target,
        &fixture.bootstrap_dir,
        &MigrationHooks::default(),
    )
    .unwrap();
    let external = fixture.root.join("external-retained-data");
    let junction = fixture.source.path.join("covers").join("linked");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("keep.txt"), b"keep").unwrap();
    assert!(Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&external)
        .status()
        .unwrap()
        .success());

    assert_eq!(
        delete_retained_data_root(&fixture.bootstrap_dir, &fixture.source)
            .unwrap_err()
            .code(),
        "data_root_reparse_point"
    );
    assert!(external.join("keep.txt").is_file());
    fs::remove_dir(&junction).unwrap();
    fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn initial_resolution_creates_and_reuses_the_default_data_root() {
    let root = temp_root("initial-data-root");
    let bootstrap_dir = root.join("app-data");
    fs::create_dir(&bootstrap_dir).unwrap();

    let first = resolve_or_initialize_data_root(&bootstrap_dir).unwrap();
    assert_eq!(first.current_root.path, bootstrap_dir.join("Data"));
    assert!(first.retained_data_roots.is_empty());
    assert!(first
        .current_root
        .path
        .join(".emily-data-root.json")
        .is_file());
    let second = resolve_or_initialize_data_root(&bootstrap_dir).unwrap();
    assert_eq!(second, first);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn data_root_marker_round_trips_through_a_long_windows_path() {
    let root = temp_root("long-marker");
    let long_root = root
        .join("a".repeat(100))
        .join("b".repeat(100))
        .join("c".repeat(40));
    let verbatim = PathBuf::from(format!(r"\\?\{}", long_root.to_string_lossy()));
    fs::create_dir_all(&verbatim).unwrap();
    let root_ref = DataRootRefV1 {
        path: long_root.clone(),
        root_id: "77777777777777777777777777777777".into(),
    };
    write_data_root_marker_atomic(
        &long_root,
        &DataRootMarkerV1::new(root_ref.root_id.clone()).unwrap(),
    )
    .unwrap();
    douyin_history_archive::bootstrap::validate_data_root_marker(&long_root, &root_ref).unwrap();
    fs::remove_dir_all(verbatim).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_flat_resolution_copies_allowlisted_data_and_retains_original_entries() {
    let root = temp_root("legacy-adoption");
    let bootstrap_dir = root.join("app-data");
    fs::create_dir(&bootstrap_dir).unwrap();
    let source = standard_root(&root);
    fs::remove_file(source.join("history.sqlite")).unwrap();
    let database = Connection::open(source.join("history.sqlite")).unwrap();
    database
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA user_version=4;
             CREATE TABLE legacy_records(id INTEGER PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO legacy_records(value) VALUES ('legacy-record');",
        )
        .unwrap();
    drop(database);
    fs::rename(
        source.join("history.sqlite"),
        bootstrap_dir.join("history.sqlite"),
    )
    .unwrap();
    fs::rename(source.join("profiles"), bootstrap_dir.join("profiles")).unwrap();
    let legacy_account_root = bootstrap_dir
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let legacy_profile = browser_profile_path(&legacy_account_root, BrowserId::Edge);
    let legacy_selection = BrowserSelection {
        schema_version: 1,
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files\Edge\msedge.exe".into(),
        profile_path: legacy_profile.to_string_lossy().into_owned(),
        profile_generation: None,
        selected_at: "2026-08-11T00:00:00.000Z".into(),
    };
    write_browser_selection(&legacy_account_root, &legacy_selection).unwrap();
    write_login_validation(
        &legacy_account_root,
        &legacy_selection,
        &LoginValidation {
            schema_version: 2,
            validation_id: Some("fedcba9876543210fedcba9876543210".into()),
            browser_id: BrowserId::Edge,
            executable_path: legacy_selection.executable_path.clone(),
            profile_path: legacy_selection.profile_path.clone(),
            profile_generation: None,
            validated_at: "2026-08-11T00:01:00.000Z".into(),
            history_url: "https://www.douyin.com/user/self?showTab=record".into(),
        },
    )
    .unwrap();
    fs::create_dir(bootstrap_dir.join("tmp")).unwrap();
    fs::write(bootstrap_dir.join("tmp/task.json"), b"task").unwrap();
    fs::write(
        bootstrap_dir.join("device-settings.json"),
        br#"{"schemaVersion":1,"activeAccountId":7}"#,
    )
    .unwrap();
    fs::write(
        bootstrap_dir.join("browser-selection.json"),
        br#"{"legacy":true}"#,
    )
    .unwrap();
    fs::write(bootstrap_dir.join("unmanaged.txt"), b"leave-me").unwrap();

    let adopted = resolve_or_initialize_data_root(&bootstrap_dir).unwrap();
    assert_eq!(adopted.current_root.path, bootstrap_dir.join("Data"));
    assert_eq!(adopted.active_account_id, Some(7));
    assert!(adopted.current_root.path.join("history.sqlite").is_file());
    let adopted_database =
        Connection::open(adopted.current_root.path.join("history.sqlite")).unwrap();
    assert_eq!(
        adopted_database
            .query_row("SELECT COUNT(*) FROM legacy_records", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        adopted_database
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        4
    );
    drop(adopted_database);
    assert!(adopted.current_root.path.join("tmp/task.json").is_file());
    assert!(adopted
        .current_root
        .path
        .join("device-settings.json")
        .is_file());
    let account_root = adopted
        .current_root
        .path
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let selection = read_browser_selection(&account_root).unwrap().unwrap();
    assert_eq!(selection.schema_version, 3);
    assert!(selection.profile_path.is_empty());
    assert!(selection.profile_generation.is_some());
    let validation = read_login_validation(&account_root, &selection)
        .unwrap()
        .unwrap();
    assert_eq!(validation.validation_id.as_deref(), Some("fedcba9876543210fedcba9876543210"));
    assert_eq!(adopted.retained_data_roots.len(), 1);
    let retained = &adopted.retained_data_roots[0];
    assert_eq!(retained.layout, RetainedLayoutV1::LegacyFlat);
    assert!(retained.root.path.join("history.sqlite").is_file());
    assert!(retained.root.path.join("profiles").is_dir());
    assert!(retained.root.path.join("device-settings.json").is_file());
    assert!(retained.root.path.join("browser-selection.json").is_file());
    assert!(bootstrap_dir.join("unmanaged.txt").is_file());
    assert!(!bootstrap_dir.join("history.sqlite").exists());
    let reopened = resolve_or_initialize_data_root(&bootstrap_dir).unwrap();
    assert_eq!(reopened, adopted);
    let reopened_selection = read_browser_selection(&account_root).unwrap().unwrap();
    assert!(read_login_validation(&account_root, &reopened_selection)
        .unwrap()
        .is_some());

    let retained_root = retained.root.clone();
    fs::rename(retained_root.path.join("tmp"), bootstrap_dir.join("tmp")).unwrap();
    let interrupted = serde_json::json!({
        "schemaVersion": 1,
        "adoptionId": "55555555555555555555555555555555",
        "target": adopted.current_root,
        "staging": bootstrap_dir.join(".emily-legacy-55555555555555555555555555555555.staging"),
        "retained": retained_root,
        "targetToken": "66666666666666666666666666666666",
        "phase": "entriesMoving",
        "sourceInventory": build_data_inventory(&bootstrap_dir.join("Data")).unwrap(),
        "legacyEntries": retained.legacy_entries,
        "activeAccountId": 7
    });
    fs::write(
        bootstrap_dir.join("legacy-data-root-adoption.json"),
        serde_json::to_vec_pretty(&interrupted).unwrap(),
    )
    .unwrap();
    let recovered = resolve_or_initialize_data_root(&bootstrap_dir).unwrap();
    assert_eq!(recovered, adopted);
    assert!(recovered.retained_data_roots[0]
        .root
        .path
        .join("tmp/task.json")
        .is_file());
    assert!(!bootstrap_dir.join("tmp").exists());
    assert!(!bootstrap_dir
        .join("legacy-data-root-adoption.json")
        .exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn inventory_rejects_internal_junctions() {
    use std::process::Command;

    let root = temp_root("inventory-junction");
    let source = standard_root(&root);
    let external = root.join("external");
    let junction = source.join("covers").join("linked");
    fs::create_dir(&external).unwrap();
    assert!(Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&external)
        .status()
        .unwrap()
        .success());

    assert_eq!(
        build_data_inventory(&source).unwrap_err().code(),
        "data_root_reparse_point"
    );
    fs::remove_dir(&junction).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn normalizes_empty_local_targets_and_install_data() {
    let root = temp_root("normalize");
    let context = context(&root);
    let ordinary = root.join("ordinary");
    fs::create_dir(&ordinary).unwrap();

    let normalized = normalize_requested_root(&context, &ordinary).unwrap();
    assert_eq!(normalized.path, canonical_display(&ordinary));
    assert!(normalized.target_existed);
    assert!(!normalized.adjusted_to_install_data);
    assert_eq!(
        normalized.parent_identity.canonical_path,
        canonical_display(&root)
    );
    normalized.verify_target_ownership().unwrap();
    release_normalized_target(&normalized);
    assert!(fs::read_dir(&ordinary).unwrap().next().is_none());

    let verbatim_local = fs::canonicalize(&ordinary).unwrap();
    let normalized_verbatim = normalize_requested_root(&context, &verbatim_local).unwrap();
    assert_eq!(normalized_verbatim.path, canonical_display(&ordinary));
    release_normalized_target(&normalized_verbatim);

    let install_data = normalize_requested_root(&context, &context.install_dir).unwrap();
    assert_eq!(install_data.path, context.install_dir.join("Data"));
    assert!(!install_data.target_existed);
    assert!(install_data.adjusted_to_install_data);
    install_data.verify_target_ownership().unwrap();
    release_normalized_target(&install_data);
    assert!(!install_data.path.exists());

    let uppercase_install = PathBuf::from(context.install_dir.to_string_lossy().to_uppercase());
    let uppercase_data = normalize_requested_root(&context, &uppercase_install).unwrap();
    assert_eq!(uppercase_data.path, context.install_dir.join("Data"));
    assert!(uppercase_data.adjusted_to_install_data);
    release_normalized_target(&uppercase_data);

    fs::create_dir(context.install_dir.join("Data")).unwrap();
    let selected_data =
        normalize_requested_root(&context, &context.install_dir.join("Data")).unwrap();
    assert_eq!(selected_data.path, context.install_dir.join("Data"));
    assert!(selected_data.target_existed);
    assert!(!selected_data.adjusted_to_install_data);
    release_normalized_target(&selected_data);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_invalid_remote_overlapping_and_nonempty_targets() {
    let root = temp_root("reject");
    let context = context(&root);

    assert_eq!(
        normalize_requested_root(&context, Path::new("relative"))
            .unwrap_err()
            .code(),
        "data_root_not_absolute"
    );
    assert_eq!(
        normalize_requested_root(&context, Path::new(r"C:\"))
            .unwrap_err()
            .code(),
        "data_root_is_drive_root"
    );
    assert_eq!(
        normalize_requested_root(&context, Path::new(r"\\server\share\Emily"))
            .unwrap_err()
            .code(),
        "data_root_is_remote"
    );
    assert_eq!(
        normalize_requested_root(&context, &context.app_data_dir)
            .unwrap_err()
            .code(),
        "data_root_overlaps_current"
    );
    let uppercase_current = PathBuf::from(context.app_data_dir.to_string_lossy().to_uppercase());
    assert_eq!(
        normalize_requested_root(&context, &uppercase_current)
            .unwrap_err()
            .code(),
        "data_root_overlaps_current"
    );
    assert_eq!(
        normalize_requested_root(&context, &context.app_data_dir.join("child"))
            .unwrap_err()
            .code(),
        "data_root_overlaps_current"
    );
    assert_eq!(
        normalize_requested_root(&context, context.app_data_dir.parent().unwrap())
            .unwrap_err()
            .code(),
        "data_root_overlaps_current"
    );

    let nonempty = root.join("nonempty");
    fs::create_dir(&nonempty).unwrap();
    fs::write(nonempty.join("unknown.bin"), b"unknown").unwrap();
    assert_eq!(
        normalize_requested_root(&context, &nonempty)
            .unwrap_err()
            .code(),
        "data_root_not_empty"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ownership_release_refuses_a_target_that_changed_after_validation() {
    let root = temp_root("ownership-change");
    let context = context(&root);
    let target = root.join("target");
    fs::create_dir(&target).unwrap();
    let normalized = normalize_requested_root(&context, &target).unwrap();
    fs::write(target.join("foreign.bin"), b"foreign").unwrap();

    assert_eq!(
        normalized.release_target_ownership().unwrap_err().code(),
        "data_root_not_empty"
    );
    assert!(target.exists());
    fs::remove_file(target.join("foreign.bin")).unwrap();
    release_normalized_target(&normalized);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn rejects_non_unicode_and_reparse_targets() {
    use std::{
        os::windows::ffi::{OsStrExt, OsStringExt},
        process::Command,
    };

    let root = temp_root("unsafe-paths");
    let context = context(&root);
    let mut invalid = context
        .install_dir
        .as_os_str()
        .encode_wide()
        .collect::<Vec<_>>();
    invalid.push(b'\\' as u16);
    invalid.push(0xd800);
    let invalid = PathBuf::from(std::ffi::OsString::from_wide(&invalid));
    assert_eq!(
        normalize_requested_root(&context, &invalid)
            .unwrap_err()
            .code(),
        "data_root_not_unicode"
    );

    let external = root.join("external");
    let junction = root.join("junction");
    fs::create_dir(&external).unwrap();
    assert!(Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&external)
        .status()
        .unwrap()
        .success());
    assert_eq!(
        normalize_requested_root(&context, &junction)
            .unwrap_err()
            .code(),
        "data_root_reparse_point"
    );

    let target = root.join("target-with-junction");
    let nested = target.join("nested");
    fs::create_dir(&target).unwrap();
    assert!(Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&nested)
        .arg(&external)
        .status()
        .unwrap()
        .success());
    assert_eq!(
        normalize_requested_root(&context, &target)
            .unwrap_err()
            .code(),
        "data_root_reparse_point"
    );

    fs::remove_dir(&nested).unwrap();
    fs::remove_dir(&junction).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn probe_exercises_files_and_live_sqlite_wal_without_residue() {
    let root = temp_root("probe");
    let target = root.join("target");
    fs::create_dir(&target).unwrap();
    let mut observed = false;

    probe_data_root_permissions_with_observer(&target, |probe_dir| {
        observed = true;
        assert!(probe_dir.join("probe.sqlite-wal").is_file());
        assert!(probe_dir.join("probe.sqlite-shm").is_file());
        Ok(())
    })
    .unwrap();

    assert!(observed);
    assert!(fs::read_dir(&target).unwrap().next().is_none());
    probe_data_root_permissions(&target).unwrap();
    assert!(fs::read_dir(&target).unwrap().next().is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn probe_maps_unusable_roots_to_a_stable_permission_error() {
    let root = temp_root("probe-denied");
    let file = root.join("not-a-directory");
    fs::write(&file, b"occupied").unwrap();

    assert_eq!(
        probe_data_root_permissions(&file).unwrap_err().code(),
        "data_root_permission_denied"
    );
    assert_eq!(fs::read(&file).unwrap(), b"occupied");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn probe_rejects_a_real_read_only_acl_directory() {
    use std::process::Command;

    let root = temp_root("probe-read-only");
    let target = root.join("read-only");
    fs::create_dir(&target).unwrap();
    let user = String::from_utf8(Command::new("whoami").output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_string();
    assert!(!user.is_empty());
    assert!(Command::new("icacls")
        .arg(&target)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("{user}:(OI)(CI)(RX)"))
        .status()
        .unwrap()
        .success());

    let result = probe_data_root_permissions(&target);
    let restored = Command::new("icacls")
        .arg(&target)
        .arg("/grant:r")
        .arg(format!("{user}:(OI)(CI)(F)"))
        .status()
        .unwrap()
        .success();
    assert!(restored);
    assert_eq!(result.unwrap_err().code(), "data_root_permission_denied");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn probe_removes_a_root_that_it_created() {
    let root = temp_root("probe-create");
    let target = root.join("new-target");

    probe_data_root_permissions(&target).unwrap();

    assert!(!target.exists());
    fs::remove_dir_all(root).unwrap();
}
