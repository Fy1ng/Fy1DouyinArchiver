use douyin_history_archive::collector::browser_discovery::discover_from_candidates;
use douyin_history_archive::collector::browser_process::{
    browser_identity_matches, local_debug_port_is_reachable, BrowserProcessIdentity,
    BrowserProcessRecordV2, ManagedBrowser,
};
use douyin_history_archive::collector::browser_state::{
    browser_profile_path, canonical_windows_path, ensure_account_profile_root,
    ensure_profile_generation, read_browser_selection, read_login_validation,
    read_matching_browser_profile_identity, remove_login_validation_if_matches,
    rewrite_browser_state_without_profile_paths, write_browser_selection, write_login_validation,
    BrowserId, BrowserProfileIdentityV1, BrowserSelection, LoginValidation,
};
use douyin_history_archive::collector::cli::BrowserPreference;
use douyin_history_archive::collector::file_store::{read_json, write_json_atomically};
use douyin_history_archive::collector::profile_constraints::apply_profile_constraints;
use douyin_history_archive::collector::session::{
    browser_launch_arguments, prepare_browser_launch,
};
use douyin_history_archive::collector_process::{
    collector_arguments_sha256, collector_process_identity_matches,
    collector_record_matches_expectation, login_collector_process_identity_matches,
    terminate_collector_if_matches, terminate_login_collector_if_matches,
    CollectorProcessExpectation, CollectorProcessIdentityV1, CollectorProcessObservation,
    LoginCollectorProcessExpectation, LoginCollectorProcessIdentityV1, OwnedCollectorProcess,
};
use douyin_history_archive::sync_orchestration::SyncPhase;
use douyin_history_archive::CollectorInvocation;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Fixture {
    value: String,
}

#[test]
fn login_validation_v2_requires_an_attempt_id_and_conditional_delete_never_removes_a_newer_marker()
{
    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::create_dir_all(&account_root).unwrap();
    let profile = browser_profile_path(&account_root, BrowserId::Edge);
    fs::create_dir_all(&profile).unwrap();
    let selection = BrowserSelection {
        schema_version: 1,
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files\Edge\msedge.exe".into(),
        profile_path: profile.to_string_lossy().into_owned(),
        profile_generation: None,
        selected_at: "2026-07-19T00:00:00.000Z".into(),
    };
    write_browser_selection(&account_root, &selection).unwrap();
    let mut validation = LoginValidation {
        schema_version: 2,
        validation_id: Some("0123456789abcdef0123456789abcdef".into()),
        browser_id: BrowserId::Edge,
        executable_path: selection.executable_path.clone(),
        profile_path: selection.profile_path.clone(),
        profile_generation: None,
        validated_at: "2026-07-19T00:01:00.000Z".into(),
        history_url: "https://www.douyin.com/user/self?showTab=record".into(),
    };
    write_login_validation(&account_root, &selection, &validation).unwrap();
    assert!(!remove_login_validation_if_matches(
        &account_root,
        &selection,
        "ffffffffffffffffffffffffffffffff"
    )
    .unwrap());
    assert!(read_login_validation(&account_root, &selection)
        .unwrap()
        .is_some());
    assert!(remove_login_validation_if_matches(
        &account_root,
        &selection,
        validation.validation_id.as_deref().unwrap()
    )
    .unwrap());
    assert!(read_login_validation(&account_root, &selection)
        .unwrap()
        .is_none());

    validation.validation_id = Some("NOT-A-VALIDATION-ID".into());
    assert_eq!(
        write_login_validation(&account_root, &selection, &validation),
        Err("invalid_login_validation".into())
    );
    fs::remove_dir_all(root).unwrap();
}

fn temp_root() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "emily-native-state-{}-{suffix}-{sequence}",
        std::process::id()
    ))
}

#[test]
fn every_browser_session_disables_extensions_signin_sync_and_background_mode() {
    for headless in [false, true] {
        let args = browser_launch_arguments(Path::new(r"C:\App\profile"), headless);
        for required in [
            "--disable-extensions",
            "--disable-component-extensions-with-background-pages",
            "--disable-default-apps",
            "--disable-sync",
            "--disable-background-mode",
            "--no-first-run",
            "--no-default-browser-check",
        ] {
            assert!(args.iter().any(|arg| arg == required), "{required}");
        }
    }
}

#[test]
fn profile_constraints_merge_controlled_preferences_without_replacing_existing_state() {
    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let profile = browser_profile_path(&account_root, BrowserId::Edge);
    fs::create_dir_all(profile.join("Default")).unwrap();
    fs::write(
        profile.join("Default/Preferences"),
        serde_json::to_vec(&json!({"sentinel": "keep", "signin": {"other": true}})).unwrap(),
    )
    .unwrap();
    fs::write(
        profile.join("Local State"),
        serde_json::to_vec(&json!({"sentinel": "keep"})).unwrap(),
    )
    .unwrap();

    let constrained = apply_profile_constraints(&account_root, BrowserId::Edge).unwrap();

    assert_eq!(constrained, profile);
    let preferences: Value = read_json(&profile.join("Default/Preferences")).unwrap();
    assert_eq!(
        preferences.pointer("/signin/allowed_on_next_startup"),
        Some(&json!(false))
    );
    assert_eq!(preferences.pointer("/signin/allowed"), Some(&json!(false)));
    assert_eq!(preferences.pointer("/sync/requested"), Some(&json!(false)));
    assert_eq!(
        preferences.pointer("/sync/has_setup_completed"),
        Some(&json!(false))
    );
    assert_eq!(preferences.pointer("/sentinel"), Some(&json!("keep")));
    assert_eq!(preferences.pointer("/signin/other"), Some(&json!(true)));
    let local_state: Value = read_json(&profile.join("Local State")).unwrap();
    assert_eq!(
        local_state.pointer("/background_mode/enabled"),
        Some(&json!(false))
    );
    assert_eq!(local_state.pointer("/sentinel"), Some(&json!("keep")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn browser_launch_preparation_derives_and_constrains_the_profile_from_account_scope() {
    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::create_dir_all(account_root.parent().unwrap()).unwrap();

    let (profile, args) = prepare_browser_launch(&account_root, BrowserId::Chrome, true).unwrap();

    assert_eq!(
        profile,
        browser_profile_path(&account_root, BrowserId::Chrome)
    );
    assert!(args
        .iter()
        .any(|arg| arg == &format!("--user-data-dir={}", profile.to_string_lossy())));
    let preferences: Value = read_json(&profile.join("Default/Preferences")).unwrap();
    assert_eq!(preferences.pointer("/sync/requested"), Some(&json!(false)));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn profile_constraints_reject_default_junction_without_touching_external_preferences() {
    use std::process::Command;

    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let profile = browser_profile_path(&account_root, BrowserId::Edge);
    let external = root.join("external-default");
    fs::create_dir_all(&profile).unwrap();
    fs::create_dir_all(&external).unwrap();
    let external_preferences = external.join("Preferences");
    let original = br#"{"sentinel":"external"}"#;
    fs::write(&external_preferences, original).unwrap();
    assert!(Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(profile.join("Default"))
        .arg(&external)
        .status()
        .unwrap()
        .success());

    assert_eq!(
        apply_profile_constraints(&account_root, BrowserId::Edge),
        Err("browser_profile_unsafe_link".to_string())
    );
    assert_eq!(fs::read(&external_preferences).unwrap(), original);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn browser_profile_paths_are_isolated_and_windows_paths_are_canonical() {
    let account_a = PathBuf::from(r"C:\App Data\Emily\profiles\aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let account_b = PathBuf::from(r"C:\App Data\Emily\profiles\bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    assert_eq!(
        browser_profile_path(&account_a, BrowserId::Edge),
        account_a.join("edge")
    );
    assert_ne!(
        browser_profile_path(&account_a, BrowserId::Edge),
        browser_profile_path(&account_b, BrowserId::Edge)
    );
    assert_eq!(
        canonical_windows_path(r"C:\App\Profiles\..\Emily\"),
        r"c:\app\emily"
    );
    assert_eq!(
        canonical_windows_path(r"\\?\C:\App\Profiles\..\Emily\"),
        r"c:\app\emily"
    );
    assert_eq!(
        canonical_windows_path(r"\\?\UNC\Server\Share\App\..\Emily\"),
        r"\\server\share\emily"
    );
    assert_eq!(canonical_windows_path(r"C:\用户\Emily"), r"c:\用户\emily");
}

#[test]
fn local_debug_port_liveness_survives_browser_launcher_process_handoff() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    assert!(local_debug_port_is_reachable(port));

    drop(listener);
    assert!(!local_debug_port_is_reachable(port));
}

#[test]
fn profile_generation_is_durable_and_strict_identity_rejects_every_mismatch() {
    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::create_dir_all(&account_root).unwrap();
    let profile = browser_profile_path(&account_root, BrowserId::Edge);
    fs::create_dir_all(&profile).unwrap();

    let generation = ensure_profile_generation(&account_root, BrowserId::Edge).unwrap();
    assert_eq!(generation.len(), 32);
    assert!(generation
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    assert_eq!(
        ensure_profile_generation(&account_root, BrowserId::Edge).unwrap(),
        generation
    );

    let selection = BrowserSelection {
        schema_version: 3,
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files\Edge\msedge.exe".into(),
        profile_path: String::new(),
        profile_generation: Some(generation.clone()),
        selected_at: "2026-08-11T00:00:00.000Z".into(),
    };
    write_browser_selection(&account_root, &selection).unwrap();
    let validation = LoginValidation {
        schema_version: 4,
        validation_id: Some("0123456789abcdef0123456789abcdef".into()),
        browser_id: BrowserId::Edge,
        executable_path: selection.executable_path.clone(),
        profile_path: String::new(),
        profile_generation: Some(generation.clone()),
        validated_at: "2026-08-11T00:01:00.000Z".into(),
        history_url: "https://www.douyin.com/".into(),
    };
    write_login_validation(&account_root, &selection, &validation).unwrap();

    let expected = BrowserProfileIdentityV1 {
        browser_id: BrowserId::Edge,
        executable_path: selection.executable_path.clone(),
        profile_path: profile.to_string_lossy().into_owned(),
        profile_generation: generation.clone(),
    };
    assert_eq!(
        read_matching_browser_profile_identity(&account_root, &expected).unwrap(),
        expected
    );

    for mismatch in [
        BrowserProfileIdentityV1 {
            browser_id: BrowserId::Chrome,
            ..expected.clone()
        },
        BrowserProfileIdentityV1 {
            executable_path: r"C:\Other\msedge.exe".into(),
            ..expected.clone()
        },
        BrowserProfileIdentityV1 {
            profile_path: account_root.join("other").to_string_lossy().into_owned(),
            ..expected.clone()
        },
        BrowserProfileIdentityV1 {
            profile_generation: "ffffffffffffffffffffffffffffffff".into(),
            ..expected.clone()
        },
    ] {
        assert_eq!(
            read_matching_browser_profile_identity(&account_root, &mismatch),
            Err("browser_profile_changed".into())
        );
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_browser_markers_are_upgraded_to_one_profile_generation() {
    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let profile = browser_profile_path(&account_root, BrowserId::Chrome);
    fs::create_dir_all(&profile).unwrap();
    let selection = BrowserSelection {
        schema_version: 2,
        browser_id: BrowserId::Chrome,
        executable_path: r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
        profile_path: String::new(),
        profile_generation: None,
        selected_at: "2026-08-11T00:00:00.000Z".into(),
    };
    write_browser_selection(&account_root, &selection).unwrap();
    write_login_validation(
        &account_root,
        &selection,
        &LoginValidation {
            schema_version: 3,
            validation_id: Some("fedcba9876543210fedcba9876543210".into()),
            browser_id: BrowserId::Chrome,
            executable_path: selection.executable_path.clone(),
            profile_path: String::new(),
            profile_generation: None,
            validated_at: "2026-08-11T00:01:00.000Z".into(),
            history_url: "https://www.douyin.com/".into(),
        },
    )
    .unwrap();

    let upgraded_selection = read_browser_selection(&account_root).unwrap().unwrap();
    assert_eq!(upgraded_selection.schema_version, 3);
    let generation = upgraded_selection.profile_generation.clone().unwrap();
    let upgraded_validation = read_login_validation(&account_root, &upgraded_selection)
        .unwrap()
        .unwrap();
    assert_eq!(upgraded_validation.schema_version, 4);
    assert_eq!(
        upgraded_validation.profile_generation.as_deref(),
        Some(generation.as_str())
    );
    assert!(profile.join("profile-generation.json").is_file());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn first_login_creates_a_missing_account_profile_root() {
    let root = temp_root();
    let profiles = root.join("profiles");
    let account_root = profiles.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::create_dir_all(&profiles).unwrap();

    ensure_account_profile_root(&account_root).expect("create account root");

    assert!(account_root.is_dir());
    assert_eq!(read_browser_selection(&account_root).unwrap(), None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn account_profile_root_must_be_a_directory() {
    let root = temp_root();
    let profiles = root.join("profiles");
    let account_root = profiles.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::create_dir_all(&profiles).unwrap();
    fs::write(&account_root, b"not a directory").unwrap();

    assert_eq!(
        ensure_account_profile_root(&account_root),
        Err("browser_profile_scope_mismatch".to_string())
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selection_round_trips_through_atomic_json() {
    let account_root = temp_root()
        .join("profiles")
        .join("0123456789abcdef0123456789abcdef");
    fs::create_dir_all(&account_root).unwrap();
    let selection = BrowserSelection {
        schema_version: 1,
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
            .to_string(),
        profile_path: browser_profile_path(&account_root, BrowserId::Edge)
            .to_string_lossy()
            .to_string(),
        profile_generation: None,
        selected_at: "2026-07-11T00:00:00.000Z".to_string(),
    };
    write_browser_selection(&account_root, &selection).unwrap();
    let upgraded_selection = read_browser_selection(&account_root).unwrap().unwrap();
    assert_eq!(upgraded_selection.schema_version, 3);
    assert!(upgraded_selection.profile_generation.is_some());
    assert!(account_root.join("browser-selection.json").exists());

    let validation = LoginValidation {
        schema_version: 1,
        validation_id: None,
        browser_id: BrowserId::Edge,
        executable_path: selection.executable_path.clone(),
        profile_path: selection.profile_path.clone(),
        profile_generation: None,
        validated_at: "2026-07-11T00:01:00.000Z".to_string(),
        history_url: "https://www.douyin.com/user/self?showTab=record".to_string(),
    };
    write_login_validation(&account_root, &selection, &validation).unwrap();
    let upgraded_validation = read_login_validation(&account_root, &selection)
        .unwrap()
        .unwrap();
    assert_eq!(upgraded_validation.schema_version, 4);
    assert_eq!(
        upgraded_validation.profile_generation,
        upgraded_selection.profile_generation
    );
    assert!(browser_profile_path(&account_root, BrowserId::Edge)
        .join(".login-validated.json")
        .exists());
    fs::remove_dir_all(account_root.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn schema_v2_v3_browser_state_omits_absolute_profile_paths() {
    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("0123456789abcdef0123456789abcdef");
    let profile = browser_profile_path(&account_root, BrowserId::Edge);
    fs::create_dir_all(&profile).unwrap();
    let selection = BrowserSelection {
        schema_version: 2,
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
        profile_path: String::new(),
        profile_generation: None,
        selected_at: "2026-08-11T00:00:00.000Z".into(),
    };
    write_browser_selection(&account_root, &selection).unwrap();
    let validation = LoginValidation {
        schema_version: 3,
        validation_id: Some("0123456789abcdef0123456789abcdef".into()),
        browser_id: BrowserId::Edge,
        executable_path: selection.executable_path.clone(),
        profile_path: String::new(),
        profile_generation: None,
        validated_at: "2026-08-11T00:01:00.000Z".into(),
        history_url: "https://www.douyin.com/user/self?showTab=record".into(),
    };
    write_login_validation(&account_root, &selection, &validation).unwrap();

    let upgraded_selection = read_browser_selection(&account_root).unwrap().unwrap();
    let upgraded_validation = read_login_validation(&account_root, &selection)
        .unwrap()
        .unwrap();
    assert_eq!(upgraded_selection.schema_version, 3);
    assert_eq!(upgraded_validation.schema_version, 4);
    assert_eq!(
        upgraded_validation.profile_generation,
        upgraded_selection.profile_generation
    );
    for state_path in [
        account_root.join("browser-selection.json"),
        profile.join(".login-validated.json"),
    ] {
        let raw = fs::read_to_string(state_path).unwrap();
        assert!(!raw.contains("profilePath"));
        assert!(!raw.contains(&root.to_string_lossy().to_string()));
    }
    assert!(remove_login_validation_if_matches(
        &account_root,
        &selection,
        validation.validation_id.as_deref().unwrap()
    )
    .unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_browser_state_rewrites_to_path_independent_schemas() {
    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("0123456789abcdef0123456789abcdef");
    let profile = browser_profile_path(&account_root, BrowserId::Chrome);
    fs::create_dir_all(&profile).unwrap();
    let selection = BrowserSelection {
        schema_version: 1,
        browser_id: BrowserId::Chrome,
        executable_path: r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
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
            browser_id: BrowserId::Chrome,
            executable_path: selection.executable_path.clone(),
            profile_path: selection.profile_path.clone(),
            profile_generation: None,
            validated_at: "2026-08-11T00:01:00.000Z".into(),
            history_url: "https://www.douyin.com/user/self?showTab=record".into(),
        },
    )
    .unwrap();

    rewrite_browser_state_without_profile_paths(&account_root).unwrap();

    let rewritten = read_browser_selection(&account_root).unwrap().unwrap();
    assert_eq!(rewritten.schema_version, 3);
    assert!(rewritten.profile_path.is_empty());
    assert!(rewritten.profile_generation.is_some());
    let validation = read_login_validation(&account_root, &rewritten)
        .unwrap()
        .unwrap();
    assert_eq!(validation.schema_version, 4);
    assert!(validation.profile_path.is_empty());
    assert_eq!(
        validation.validation_id.as_deref(),
        Some("fedcba9876543210fedcba9876543210")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selection_and_login_validation_reject_cross_account_profile_paths() {
    let root = temp_root();
    let account_a = root
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let account_b = root
        .join("profiles")
        .join("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    fs::create_dir_all(&account_a).unwrap();
    fs::create_dir_all(&account_b).unwrap();
    let selection = BrowserSelection {
        schema_version: 1,
        browser_id: BrowserId::Chrome,
        executable_path: r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
        profile_path: browser_profile_path(&account_b, BrowserId::Chrome)
            .to_string_lossy()
            .to_string(),
        profile_generation: None,
        selected_at: "2026-07-11T00:00:00.000Z".to_string(),
    };

    assert_eq!(
        write_browser_selection(&account_a, &selection),
        Err("browser_profile_scope_mismatch".to_string())
    );
    let validation = LoginValidation {
        schema_version: 1,
        validation_id: None,
        browser_id: BrowserId::Chrome,
        executable_path: selection.executable_path.clone(),
        profile_path: selection.profile_path.clone(),
        profile_generation: None,
        validated_at: "2026-07-11T00:01:00.000Z".to_string(),
        history_url: "https://www.douyin.com/user/self?showTab=record".to_string(),
    };
    assert_eq!(
        write_login_validation(&account_a, &selection, &validation),
        Err("browser_profile_scope_mismatch".to_string())
    );

    let account_a_selection = BrowserSelection {
        profile_path: browser_profile_path(&account_a, BrowserId::Chrome)
            .to_string_lossy()
            .to_string(),
        ..selection
    };
    write_browser_selection(&account_a, &account_a_selection).unwrap();
    assert_eq!(
        write_login_validation(&account_a, &account_a_selection, &validation),
        Err("browser_profile_scope_mismatch".to_string())
    );
    write_json_atomically(
        &browser_profile_path(&account_a, BrowserId::Chrome).join(".login-validated.json"),
        &validation,
    )
    .unwrap();
    assert_eq!(
        read_login_validation(&account_a, &account_a_selection),
        Err("browser_profile_scope_mismatch".to_string())
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn browser_state_rejects_junction_profile_without_touching_external_target() {
    use std::process::Command;
    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let external = root.join("external");
    fs::create_dir_all(&account_root).unwrap();
    fs::create_dir_all(&external).unwrap();
    assert!(Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(account_root.join("edge"))
        .arg(&external)
        .status()
        .unwrap()
        .success());
    let selection = BrowserSelection {
        schema_version: 1,
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files\Edge\msedge.exe".into(),
        profile_path: account_root.join("edge").to_string_lossy().into_owned(),
        profile_generation: None,
        selected_at: "2026-07-11T00:00:00.000Z".into(),
    };
    assert_eq!(
        write_browser_selection(&account_root, &selection),
        Err("browser_profile_unsafe_link".into())
    );
    assert!(fs::read_dir(&external).unwrap().next().is_none());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn browser_selection_read_revalidates_account_root_junction_before_reading_external_state() {
    use std::process::Command;
    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let external = root.join("external-account");
    fs::create_dir_all(&account_root).unwrap();
    let selection = BrowserSelection {
        schema_version: 1,
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files\Edge\msedge.exe".into(),
        profile_path: account_root.join("edge").to_string_lossy().into_owned(),
        profile_generation: None,
        selected_at: "2026-07-11T00:00:00.000Z".into(),
    };
    write_browser_selection(&account_root, &selection).unwrap();
    fs::rename(&account_root, &external).unwrap();
    fs::write(
        external.join("browser-selection.json"),
        b"external-state-must-not-be-read",
    )
    .unwrap();
    assert!(Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&account_root)
        .arg(&external)
        .status()
        .unwrap()
        .success());

    assert_eq!(
        read_browser_selection(&account_root),
        Err("browser_profile_scope_mismatch".into())
    );
    assert_eq!(
        fs::read(external.join("browser-selection.json")).unwrap(),
        b"external-state-must-not-be-read"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn account_profile_root_rejects_a_junction_parent() {
    use std::process::Command;
    let root = temp_root();
    let profiles = root.join("profiles");
    let external_profiles = root.join("external-profiles");
    let account_root = external_profiles.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::create_dir_all(&account_root).unwrap();
    assert!(Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&profiles)
        .arg(&external_profiles)
        .status()
        .unwrap()
        .success());

    assert_eq!(
        ensure_account_profile_root(&profiles.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")),
        Err("browser_profile_scope_mismatch".into())
    );

    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn login_validation_read_revalidates_profile_junction_before_reading_external_state() {
    use std::process::Command;
    let root = temp_root();
    let account_root = root
        .join("profiles")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::create_dir_all(&account_root).unwrap();
    let selection = BrowserSelection {
        schema_version: 1,
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files\Edge\msedge.exe".into(),
        profile_path: account_root.join("edge").to_string_lossy().into_owned(),
        profile_generation: None,
        selected_at: "2026-07-11T00:00:00.000Z".into(),
    };
    write_browser_selection(&account_root, &selection).unwrap();
    let validation = LoginValidation {
        schema_version: 1,
        validation_id: None,
        browser_id: BrowserId::Edge,
        executable_path: selection.executable_path.clone(),
        profile_path: selection.profile_path.clone(),
        profile_generation: None,
        validated_at: "2026-07-11T00:01:00.000Z".into(),
        history_url: "https://www.douyin.com/user/self?showTab=record".into(),
    };
    write_login_validation(&account_root, &selection, &validation).unwrap();
    let profile = browser_profile_path(&account_root, BrowserId::Edge);
    let external = root.join("external-profile");
    fs::rename(&profile, &external).unwrap();
    fs::write(
        external.join(".login-validated.json"),
        b"external-state-must-not-be-read",
    )
    .unwrap();
    assert!(Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&profile)
        .arg(&external)
        .status()
        .unwrap()
        .success());

    assert_eq!(
        read_login_validation(&account_root, &selection),
        Err("browser_profile_scope_mismatch".into())
    );
    assert_eq!(
        fs::read(external.join(".login-validated.json")).unwrap(),
        b"external-state-must-not-be-read"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn atomic_replace_keeps_only_the_latest_complete_json() {
    let root = temp_root();
    fs::create_dir_all(&root).unwrap();
    let path = root.join("state.json");
    write_json_atomically(
        &path,
        &Fixture {
            value: "first".into(),
        },
    )
    .unwrap();
    write_json_atomically(
        &path,
        &Fixture {
            value: "second".into(),
        },
    )
    .unwrap();
    assert_eq!(read_json::<Fixture>(&path).unwrap().value, "second");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn browser_discovery_prefers_edge_and_honors_forced_chrome() {
    let root = temp_root();
    fs::create_dir_all(&root).unwrap();
    let edge = root.join("msedge.exe");
    let chrome = root.join("chrome.exe");
    fs::write(&edge, b"edge").unwrap();
    fs::write(&chrome, b"chrome").unwrap();

    let automatic = discover_from_candidates(
        BrowserPreference::Auto,
        vec![edge.clone()],
        vec![chrome.clone()],
    )
    .unwrap();
    assert_eq!(automatic.browser_id, BrowserId::Edge);

    let forced =
        discover_from_candidates(BrowserPreference::Chrome, vec![edge], vec![chrome]).unwrap();
    assert_eq!(forced.browser_id, BrowserId::Chrome);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn browser_recovery_requires_creation_time_and_complete_user_data_argument() {
    let record = BrowserProcessRecordV2 {
        schema_version: 2,
        pid: 42,
        creation_time_100ns: "133000000000000000".into(),
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
        profile_path: r"C:\App\profiles\edge".into(),
        user_data_dir_argument: r"--user-data-dir=C:\App\profiles\edge".into(),
        devtools_port: 54321,
        browser_websocket_path: "/devtools/browser/abc".into(),
        started_at: "2026-07-11T00:00:00.000Z".into(),
    };
    let identity = BrowserProcessIdentity {
        pid: 42,
        creation_time_100ns: "133000000000000000".into(),
        executable_path: record.executable_path.clone(),
        command_line: vec!["msedge.exe".into(), record.user_data_dir_argument.clone()],
    };
    let expected_profile = PathBuf::from(r"C:\App\profiles\edge");
    assert!(browser_identity_matches(
        &record,
        &identity,
        &expected_profile
    ));
    let mut reused = identity;
    reused.creation_time_100ns = "133000000000000001".into();
    assert!(!browser_identity_matches(
        &record,
        &reused,
        &expected_profile
    ));

    let mut wrong_record_profile = record.clone();
    wrong_record_profile.profile_path = r"C:\App\profiles\other".into();
    wrong_record_profile.user_data_dir_argument = r"--user-data-dir=C:\App\profiles\other".into();
    assert!(!browser_identity_matches(
        &wrong_record_profile,
        &BrowserProcessIdentity {
            creation_time_100ns: "133000000000000000".into(),
            command_line: vec![
                "msedge.exe".into(),
                r"--user-data-dir=C:\App\profiles\other".into(),
            ],
            ..reused.clone()
        },
        &expected_profile
    ));

    let mut partial_argument = reused;
    partial_argument.creation_time_100ns = "133000000000000000".into();
    partial_argument.command_line = vec![
        "msedge.exe".into(),
        r"--user-data-dir=C:\App\profiles\edge-extra".into(),
    ];
    assert!(!browser_identity_matches(
        &record,
        &partial_argument,
        &expected_profile
    ));
}

#[test]
fn collector_process_identity_hashes_the_complete_argument_vector() {
    let arguments = vec![
        "--collector".to_string(),
        "recover-login".to_string(),
        "--account-id".to_string(),
        "11".to_string(),
    ];

    assert_eq!(
        collector_arguments_sha256(&arguments).unwrap(),
        "de4ae489cd40812978d84963f830aa137eaa4d90a59446c8e45f590ecfc6120e"
    );
}

#[test]
fn collector_process_termination_requires_pid_creation_executable_arguments_and_phase() {
    let arguments = vec![
        "--collector".to_string(),
        "recover-login".to_string(),
        "--account-id".to_string(),
        "11".to_string(),
    ];
    let arguments_sha256 = "de4ae489cd40812978d84963f830aa137eaa4d90a59446c8e45f590ecfc6120e";
    let executable = r"C:\Program Files\Emily\Emily.exe";
    let record = CollectorProcessIdentityV1 {
        schema_version: 1,
        pid: 4242,
        creation_time_100ns: "133000000000000000".to_string(),
        executable_path: executable.to_string(),
        arguments_sha256: arguments_sha256.to_string(),
        account_id: 11,
        sync_job_id: 7,
        phase: SyncPhase::RecoveringLogin,
        phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        started_at: "2026-07-19T12:00:00.000Z".to_string(),
    };
    let expectation = CollectorProcessExpectation {
        account_id: 11,
        sync_job_id: 7,
        phase: SyncPhase::RecoveringLogin,
        phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        executable_path: executable.to_string(),
        arguments_sha256: arguments_sha256.to_string(),
    };
    let observation = CollectorProcessObservation {
        pid: 4242,
        creation_time_100ns: "133000000000000000".to_string(),
        executable_path: executable.to_string(),
        arguments: arguments.clone(),
    };

    assert!(collector_process_identity_matches(
        &record,
        &observation,
        &expectation
    ));

    let mut terminated = false;
    assert!(
        terminate_collector_if_matches(&record, &observation, &expectation, |_| {
            terminated = true;
            Ok(())
        })
        .unwrap()
    );
    assert!(terminated);

    let mut mismatches = Vec::new();
    let mut reused_pid = observation.clone();
    reused_pid.creation_time_100ns = "133000000000000001".to_string();
    mismatches.push((record.clone(), reused_pid, expectation.clone()));

    let mut wrong_executable = observation.clone();
    wrong_executable.executable_path = r"C:\Windows\System32\notepad.exe".to_string();
    mismatches.push((record.clone(), wrong_executable, expectation.clone()));

    let mut wrong_arguments = observation.clone();
    wrong_arguments.arguments.push("--unexpected".to_string());
    mismatches.push((record.clone(), wrong_arguments, expectation.clone()));

    let mut stale_attempt = record.clone();
    stale_attempt.phase_attempt_id = "fedcba9876543210fedcba9876543210".to_string();
    mismatches.push((stale_attempt, observation, expectation));

    for (record, observation, expectation) in mismatches {
        let mut called = false;
        assert!(
            !terminate_collector_if_matches(&record, &observation, &expectation, |_| {
                called = true;
                Ok(())
            })
            .unwrap()
        );
        assert!(!called);
    }
}

#[test]
fn restart_cleanup_rejects_reordered_collector_arguments() {
    let arguments = vec![
        "--collector".to_string(),
        "collect".to_string(),
        "--account-id".to_string(),
        "11".to_string(),
        "--job-id".to_string(),
        "7".to_string(),
    ];
    let executable = r"C:\Program Files\Emily\Emily.exe";
    let arguments_sha256 = collector_arguments_sha256(&arguments).unwrap();
    let record = CollectorProcessIdentityV1 {
        schema_version: 1,
        pid: 4242,
        creation_time_100ns: "133000000000000000".to_string(),
        executable_path: executable.to_string(),
        arguments_sha256: arguments_sha256.clone(),
        account_id: 11,
        sync_job_id: 7,
        phase: SyncPhase::Collecting,
        phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        started_at: "2026-07-19T12:00:00.000Z".to_string(),
    };
    let expectation = CollectorProcessExpectation {
        account_id: 11,
        sync_job_id: 7,
        phase: SyncPhase::Collecting,
        phase_attempt_id: record.phase_attempt_id.clone(),
        executable_path: executable.to_string(),
        arguments_sha256,
    };
    let mut reordered = arguments;
    reordered.swap(2, 4);
    reordered.swap(3, 5);
    let observation = CollectorProcessObservation {
        pid: record.pid,
        creation_time_100ns: record.creation_time_100ns.clone(),
        executable_path: executable.to_string(),
        arguments: reordered,
    };
    let mut terminated = false;

    assert!(
        !terminate_collector_if_matches(&record, &observation, &expectation, |_| {
            terminated = true;
            Ok(())
        },)
        .unwrap()
    );
    assert!(!terminated);
}

#[test]
fn collector_record_must_match_a_rebuilt_invocation_before_process_query() {
    let arguments = vec![
        "--collector".to_string(),
        "collect".to_string(),
        "--account-id".to_string(),
        "11".to_string(),
    ];
    let executable = r"C:\Program Files\Emily\Emily.exe";
    let expectation = CollectorProcessExpectation {
        account_id: 11,
        sync_job_id: 7,
        phase: SyncPhase::Collecting,
        phase_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        executable_path: executable.to_string(),
        arguments_sha256: collector_arguments_sha256(&arguments).unwrap(),
    };
    let mut record = CollectorProcessIdentityV1 {
        schema_version: 1,
        pid: 4242,
        creation_time_100ns: "133000000000000000".to_string(),
        executable_path: executable.to_string(),
        arguments_sha256: expectation.arguments_sha256.clone(),
        account_id: 11,
        sync_job_id: 7,
        phase: SyncPhase::Collecting,
        phase_attempt_id: expectation.phase_attempt_id.clone(),
        started_at: "2026-08-13T00:00:00.000Z".to_string(),
    };

    assert!(collector_record_matches_expectation(&record, &expectation));
    record.arguments_sha256 = "a".repeat(64);
    assert!(!collector_record_matches_expectation(&record, &expectation));
}

#[test]
fn login_collector_termination_requires_attempt_profile_and_complete_arguments() {
    let arguments = vec![
        "--collector".to_string(),
        "login".to_string(),
        "--login-attempt-id".to_string(),
        "0123456789abcdef0123456789abcdef".to_string(),
    ];
    let executable = r"C:\Program Files\Emily\Emily.exe";
    let expectation = LoginCollectorProcessExpectation {
        login_attempt_id: "0123456789abcdef0123456789abcdef".to_string(),
        profile_key: "fedcba9876543210fedcba9876543210".to_string(),
        target_account_id: None,
        executable_path: executable.to_string(),
        arguments_sha256: collector_arguments_sha256(&arguments).unwrap(),
    };
    let record = LoginCollectorProcessIdentityV1 {
        schema_version: 1,
        pid: 4242,
        creation_time_100ns: "133000000000000000".to_string(),
        executable_path: executable.to_string(),
        arguments_sha256: expectation.arguments_sha256.clone(),
        login_attempt_id: expectation.login_attempt_id.clone(),
        profile_key: expectation.profile_key.clone(),
        target_account_id: None,
        started_at: "2026-07-19T12:00:00.000Z".to_string(),
    };
    let observation = CollectorProcessObservation {
        pid: record.pid,
        creation_time_100ns: record.creation_time_100ns.clone(),
        executable_path: executable.to_string(),
        arguments: arguments.clone(),
    };

    assert!(login_collector_process_identity_matches(
        &record,
        &observation,
        &expectation,
    ));
    let mut terminated = false;
    assert!(
        terminate_login_collector_if_matches(&record, &observation, &expectation, |_| {
            terminated = true;
            Ok(())
        },)
        .unwrap()
    );
    assert!(terminated);

    let mut wrong_profile = record.clone();
    wrong_profile.profile_key = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    let mut called = false;
    assert!(!terminate_login_collector_if_matches(
        &wrong_profile,
        &observation,
        &expectation,
        |_| {
            called = true;
            Ok(())
        },
    )
    .unwrap());
    assert!(!called);

    let mut reordered = observation.clone();
    reordered.arguments.swap(0, 1);
    assert!(!login_collector_process_identity_matches(
        &record,
        &reordered,
        &expectation,
    ));
}

#[cfg(windows)]
#[test]
fn collector_process_is_owned_and_identified_before_it_is_resumed() {
    let root = temp_root();
    fs::create_dir_all(&root).unwrap();
    let identity_path = root.join("collector.pid");
    let executable = PathBuf::from(std::env::var_os("ComSpec").unwrap());
    let invocation = CollectorInvocation {
        program: executable,
        args: vec!["/d".to_string(), "/c".to_string(), "exit 0".to_string()],
        current_dir: None,
    };
    let expectation = CollectorProcessExpectation::for_invocation(
        11,
        7,
        SyncPhase::Collecting,
        "0123456789abcdef0123456789abcdef",
        &invocation,
    )
    .unwrap();

    let mut process =
        OwnedCollectorProcess::launch_suspended(&invocation, &identity_path, &expectation).unwrap();

    assert!(identity_path.is_file());
    let identity: CollectorProcessIdentityV1 = read_json(&identity_path).unwrap();
    assert_eq!(identity.pid, process.pid());
    assert_eq!(identity.arguments_sha256, expectation.arguments_sha256);
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(process.try_wait().unwrap().is_none());

    process.resume().unwrap();
    let status = process.wait().unwrap();
    assert!(status.success(), "status={status:?}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn login_collector_process_is_owned_and_identified_before_it_is_resumed() {
    let root = temp_root();
    fs::create_dir_all(&root).unwrap();
    let identity_path = root.join("login-collector.pid");
    let executable = PathBuf::from(std::env::var_os("ComSpec").unwrap());
    let invocation = CollectorInvocation {
        program: executable,
        args: vec!["/d".to_string(), "/c".to_string(), "exit 0".to_string()],
        current_dir: None,
    };
    let expectation = LoginCollectorProcessExpectation::for_invocation(
        "0123456789abcdef0123456789abcdef",
        "fedcba9876543210fedcba9876543210",
        None,
        &invocation,
    )
    .unwrap();

    let mut process =
        OwnedCollectorProcess::launch_login_suspended(&invocation, &identity_path, &expectation)
            .unwrap();

    assert!(identity_path.is_file());
    let identity: LoginCollectorProcessIdentityV1 = read_json(&identity_path).unwrap();
    assert_eq!(identity.pid, process.pid());
    assert_eq!(identity.login_attempt_id, expectation.login_attempt_id);
    assert_eq!(identity.profile_key, expectation.profile_key);
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(process.try_wait().unwrap().is_none());

    process.resume().unwrap();
    let status = process.wait().unwrap();
    assert!(status.success(), "status={status:?}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn managed_browser_termination_stops_the_owned_job() {
    let executable = PathBuf::from(std::env::var_os("ComSpec").unwrap());
    let arguments = vec![
        "/d".to_string(),
        "/c".to_string(),
        "ping.exe -n 30 127.0.0.1 >nul".to_string(),
    ];
    let process = ManagedBrowser::launch(&executable, &arguments, true).unwrap();

    assert!(!process.wait(100));
    process.terminate().unwrap();
    assert!(process.wait(2_000));
}
