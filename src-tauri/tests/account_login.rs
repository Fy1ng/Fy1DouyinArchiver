use douyin_history_archive::accounts::{
    create_pending_account, finalize_account_identity, get_account, list_accounts,
    list_command_accounts, restore_account, set_active_account,
    validate_hidden_recovery_identity, AccountIdentity,
};
use douyin_history_archive::collector::contracts::{
    account_identity_from_page_state, AccountIdentityV1,
};
use douyin_history_archive::collector::{
    browser_process::BrowserProcessRecordV2,
    browser_state::{browser_profile_path, BrowserId},
};
use douyin_history_archive::bootstrap::{
    read_bootstrap, write_bootstrap_atomic, BootstrapV1,
};
use douyin_history_archive::{
    cleanup_failed_login, cleanup_failed_login_with, cleanup_login_attempt_files, complete_login_attempt,
    complete_login_attempt_with_active_writer, create_login_attempt,
    migrate_schema_to_v5_at_startup,
    read_login_attempt, recover_login_attempts_with,
    pending_profile_cleanup_retry_delays, retry_pending_profile_cleanups_with,
    run_pending_login_guarded, AppPaths,
};
use douyin_history_archive::login_attempt::{
    compare_and_swap_login_attempt, create_login_attempt_state, read_login_attempt_state,
    LoginAttemptLifecycle,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const ATTEMPT_ID: &str = "0123456789abcdef0123456789abcdef";
const SECOND_ATTEMPT_ID: &str = "fedcba9876543210fedcba9876543210";
const PROFILE_KEY: &str = "abcdef0123456789abcdef0123456789";

#[test]
fn pending_profile_cleanup_retries_are_bounded() {
    assert_eq!(
        pending_profile_cleanup_retry_delays(),
        vec![250, 500, 1_000, 2_000, 4_000]
    );
}

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    connection
}

fn temp_paths(label: &str) -> AppPaths {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("emily-account-login-{label}-{nonce}"));
    fs::create_dir_all(&root).expect("bootstrap root");
    write_bootstrap_atomic(
        &root,
        &BootstrapV1::new(
            root.clone(),
            "0123456789abcdef0123456789abcdef".to_string(),
        )
        .expect("bootstrap"),
    )
    .expect("write bootstrap");
    AppPaths::new(root, PathBuf::new())
}

fn valid_identity_json() -> Value {
    json!({
        "schemaVersion": 1,
        "accountId": 7,
        "loginAttemptId": ATTEMPT_ID,
        "profileKey": PROFILE_KEY,
        "uid": "112233445566",
        "secUid": "MS4wLjABAAAAfixture-sec-uid",
        "nickname": "Fixture User",
        "avatarUrl": "https://p3-pc-sign.douyinpic.com/fixture.jpeg",
        "validatedAt": "2026-07-12T10:00:00.000Z",
        "historyUrl": "https://www.douyin.com/user/self?showTab=record"
    })
}

fn browser_record(profile_path: &std::path::Path) -> BrowserProcessRecordV2 {
    BrowserProcessRecordV2 {
        schema_version: 2,
        pid: 4321,
        creation_time_100ns: "133000000000000000".into(),
        browser_id: BrowserId::Edge,
        executable_path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
        profile_path: profile_path.to_string_lossy().to_string(),
        user_data_dir_argument: format!("--user-data-dir={}", profile_path.to_string_lossy()),
        devtools_port: 9222,
        browser_websocket_path: "/devtools/browser/test".into(),
        started_at: "2026-07-12T00:00:00.000Z".into(),
    }
}

#[test]
fn account_identity_contract_rejects_unknown_missing_or_oversized_identity() {
    let identity: AccountIdentityV1 =
        serde_json::from_value(valid_identity_json()).expect("valid identity");
    identity
        .validate_for(7, ATTEMPT_ID, PROFILE_KEY)
        .expect("matching context");

    let mut unknown = valid_identity_json();
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<AccountIdentityV1>(unknown).is_err());

    for (field, replacement, error) in [
        ("uid", json!(null), "account_identity_unavailable"),
        ("accountId", json!(8), "account_identity_mismatch"),
        (
            "loginAttemptId",
            json!("fedcba9876543210fedcba9876543210"),
            "account_identity_mismatch",
        ),
        (
            "profileKey",
            json!("fedcba9876543210fedcba9876543210"),
            "account_identity_mismatch",
        ),
        (
            "historyUrl",
            json!("https://example.com/user/self?showTab=record"),
            "invalid_account_identity",
        ),
        (
            "avatarUrl",
            json!("file:///tmp/avatar.png"),
            "invalid_account_identity",
        ),
        (
            "nickname",
            json!("x".repeat(201)),
            "invalid_account_identity",
        ),
        ("secUid", json!("x".repeat(257)), "invalid_account_identity"),
        ("uid", json!(" 112233445566"), "invalid_account_identity"),
        (
            "secUid",
            json!("MS4wLjABAAAAfixture-sec-uid "),
            "invalid_account_identity",
        ),
        (
            "nickname",
            json!(" Fixture User"),
            "invalid_account_identity",
        ),
        (
            "avatarUrl",
            json!("https://p3-pc-sign.douyinpic.com/fixture.jpeg "),
            "invalid_account_identity",
        ),
    ] {
        let mut value = valid_identity_json();
        value[field] = replacement;
        if field == "uid" {
            value["secUid"] = json!(null);
        }
        let parsed: AccountIdentityV1 = serde_json::from_value(value).expect("shape");
        assert_eq!(
            parsed.validate_for(7, ATTEMPT_ID, PROFILE_KEY).unwrap_err(),
            error,
            "field {field}"
        );
    }
}

#[test]
fn page_identity_rejects_conflicting_allowed_sources() {
    let page = json!({
        "pageUrl": "https://www.douyin.com/user/self?showTab=record",
        "state": {
            "candidates": [
                {
                    "uid": "112233445566",
                    "secUid": "MS4wLjABAAAAfixture-sec-uid",
                    "nickname": "Fixture User",
                    "avatarUrl": "https://p3-pc-sign.douyinpic.com/fixture.jpeg"
                },
                {
                    "uid": "998877665544",
                    "secUid": "MS4wLjABAAAAother-sec-uid",
                    "nickname": "Other User",
                    "avatarUrl": "https://p3-pc-sign.douyinpic.com/other.jpeg"
                }
            ]
        }
    });

    assert_eq!(
        account_identity_from_page_state(
            &page,
            7,
            ATTEMPT_ID,
            PROFILE_KEY,
            "2026-07-12T10:00:00.000Z"
        )
        .unwrap_err(),
        "account_identity_ambiguous"
    );
}

#[test]
fn page_identity_fixture_reads_only_verified_self_account_state() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/account-identity-self-state.json"))
            .expect("fixture");

    let identity = account_identity_from_page_state(
        &fixture,
        7,
        ATTEMPT_ID,
        PROFILE_KEY,
        "2026-07-12T10:00:00.000Z",
    )
    .expect("self identity");

    assert_eq!(identity.uid.as_deref(), Some("112233445566"));
    assert_eq!(
        identity.sec_uid.as_deref(),
        Some("MS4wLjABAAAAfixture-sec-uid")
    );
    assert_eq!(identity.nickname.as_deref(), Some("Fixture User"));

    let nickname_only = json!({
        "pageUrl": "https://www.douyin.com/user/self?showTab=record",
        "state": {"user": {"isLogin": true, "userInfo": {"nickname": "Not identity"}}}
    });
    assert_eq!(
        account_identity_from_page_state(
            &nickname_only,
            7,
            ATTEMPT_ID,
            PROFILE_KEY,
            "2026-07-12T10:00:00.000Z"
        )
        .unwrap_err(),
        "account_identity_unavailable"
    );
}

#[test]
fn login_attempt_file_is_bound_to_account_attempt_and_profile_and_cleans_task_files() {
    let paths = temp_paths("attempt");
    fs::create_dir_all(&paths.temp).expect("temp");

    let attempt = create_login_attempt(&paths, 7, ATTEMPT_ID, PROFILE_KEY).expect("create attempt");
    assert_eq!(read_login_attempt(&paths, 7, ATTEMPT_ID).unwrap(), attempt);

    fs::write(
        paths.login_identity_path(7, ATTEMPT_ID).unwrap(),
        b"identity",
    )
    .expect("identity");
    fs::write(paths.login_cancel_path(7, ATTEMPT_ID).unwrap(), b"cancel").expect("cancel");
    fs::write(
        paths.login_browser_pid_path(7, ATTEMPT_ID).unwrap(),
        b"pid",
    )
    .expect("pid");
    cleanup_login_attempt_files(&paths, 7, ATTEMPT_ID).expect("cleanup");

    assert!(!paths.login_attempt_path(7, ATTEMPT_ID).unwrap().exists());
    assert!(!paths.login_identity_path(7, ATTEMPT_ID).unwrap().exists());
    assert!(!paths.login_cancel_path(7, ATTEMPT_ID).unwrap().exists());
    assert!(!paths
        .login_browser_pid_path(7, ATTEMPT_ID)
        .unwrap()
        .exists());
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn command_account_views_and_active_state_follow_task_four_status_rules() {
    let paths = temp_paths("commands");
    fs::create_dir_all(&paths.app_data).expect("app data");
    let mut connection = database();
    let pending = create_pending_account(&mut connection).expect("pending");
    let active = finalize_account_identity(
        &mut connection,
        pending.id,
        AccountIdentity {
            uid: Some("uid-active".into()),
            sec_uid: None,
            nickname: None,
            avatar_url: None,
        },
    )
    .expect("active");
    connection
        .execute(
            "INSERT INTO accounts (uid, profile_key, status, created_at, updated_at) VALUES (?1, ?2, ?3, 'x', 'x')",
            params!["uid-expired", "11111111111111111111111111111111", "login_expired"],
        )
        .expect("expired");
    let expired_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO accounts (uid, profile_key, status, created_at, updated_at) VALUES (?1, ?2, ?3, 'x', 'x')",
            params!["uid-conflict", "33333333333333333333333333333333", "identity_conflict"],
        )
        .expect("identity conflict");
    let conflict_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO accounts (uid, profile_key, status, created_at, updated_at) VALUES (?1, ?2, 'archived', 'x', 'x')",
            params!["uid-archived", "22222222222222222222222222222222"],
        )
        .expect("archived");
    let archived_id = connection.last_insert_rowid();
    let management_pending = create_pending_account(&mut connection).expect("management pending");

    let visible = list_command_accounts(&connection, false).expect("visible accounts");
    assert_eq!(
        visible.iter().map(|item| item.id).collect::<Vec<_>>(),
        vec![active.id, expired_id, conflict_id]
    );
    let managed = list_command_accounts(&connection, true).unwrap();
    assert_eq!(managed.len(), 4);
    assert!(!managed
        .iter()
        .any(|account| account.id == management_pending.id));
    assert!(list_accounts(&connection)
        .unwrap()
        .iter()
        .any(|account| account.id == management_pending.id));

    set_active_account(&paths.bootstrap_dir, &connection, expired_id).expect("select expired");
    set_active_account(&paths.bootstrap_dir, &connection, conflict_id).expect("select conflict");
    assert_eq!(
        get_account(&connection, expired_id)
            .unwrap()
            .unwrap()
            .status,
        "login_expired"
    );
    assert_eq!(
        set_active_account(&paths.bootstrap_dir, &connection, archived_id).unwrap_err(),
        "account_not_selectable"
    );

    let restored = restore_account(&mut connection, archived_id).expect("restore");
    assert_eq!(restored.status, "login_expired");
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn hidden_recovery_requires_a_shared_matching_id_without_filling_missing_ids() {
    let connection = database();
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, nickname, status, created_at, updated_at)
             VALUES ('uid-local', NULL, ?1, 'Local Nickname', 'active', 'x', 'x')",
            [PROFILE_KEY],
        )
        .expect("account");
    let account_id = connection.last_insert_rowid();

    let validated = validate_hidden_recovery_identity(
        &connection,
        account_id,
        AccountIdentity {
            uid: Some("uid-local".to_string()),
            sec_uid: Some("sec-remote".to_string()),
            nickname: Some("Remote Nickname".to_string()),
            avatar_url: None,
        },
    )
    .expect("matching recovery identity");

    assert_eq!(validated.uid.as_deref(), Some("uid-local"));
    assert_eq!(validated.sec_uid, None);
    assert_eq!(validated.nickname.as_deref(), Some("Local Nickname"));
    let persisted = get_account(&connection, account_id).unwrap().unwrap();
    assert_eq!(persisted.sec_uid, None);
    assert_eq!(persisted.nickname.as_deref(), Some("Local Nickname"));

    for remote in [
        AccountIdentity {
            uid: Some("uid-other".to_string()),
            sec_uid: Some("sec-remote".to_string()),
            nickname: None,
            avatar_url: None,
        },
        AccountIdentity {
            uid: None,
            sec_uid: Some("sec-remote".to_string()),
            nickname: None,
            avatar_url: None,
        },
    ] {
        assert_eq!(
            validate_hidden_recovery_identity(&connection, account_id, remote).unwrap_err(),
            "account_identity_conflict"
        );
    }
}

#[test]
fn hidden_recovery_rejects_non_active_account_states() {
    for status in ["login_expired", "identity_conflict"] {
        let connection = database();
        connection
            .execute(
                "INSERT INTO accounts
                 (uid, profile_key, status, created_at, updated_at)
                 VALUES ('uid-local', ?1, ?2, 'x', 'x')",
                params![PROFILE_KEY, status],
            )
            .expect("account");
        let account_id = connection.last_insert_rowid();

        assert_eq!(
            validate_hidden_recovery_identity(
                &connection,
                account_id,
                AccountIdentity {
                    uid: Some("uid-local".to_string()),
                    sec_uid: None,
                    nickname: None,
                    avatar_url: None,
                },
            )
            .unwrap_err(),
            "invalid_account_status"
        );
    }
}

#[test]
fn duplicate_and_split_identity_failures_preserve_both_profiles() {
    let paths = temp_paths("conflict");
    fs::create_dir_all(&paths.browser_profiles).expect("profiles");
    let mut connection = database();
    connection
        .execute(
            "INSERT INTO accounts (uid, sec_uid, profile_key, status, created_at, updated_at) VALUES ('uid-1', 'sec-1', ?1, 'active', 'x', 'x')",
            ["11111111111111111111111111111111"],
        )
        .expect("existing");
    connection
        .execute(
            "INSERT INTO accounts (uid, sec_uid, profile_key, status, created_at, updated_at) VALUES ('uid-2', 'sec-2', ?1, 'active', 'x', 'x')",
            ["22222222222222222222222222222222"],
        )
        .expect("second");
    let pending = create_pending_account(&mut connection).expect("pending");
    for key in [
        "11111111111111111111111111111111",
        "22222222222222222222222222222222",
        pending.profile_key.as_str(),
    ] {
        fs::create_dir_all(paths.account_root(key).unwrap()).expect("profile");
    }

    assert_eq!(
        finalize_account_identity(
            &mut connection,
            pending.id,
            AccountIdentity {
                uid: Some("uid-1".into()),
                sec_uid: None,
                nickname: None,
                avatar_url: None
            }
        )
        .unwrap_err(),
        "account_already_exists"
    );
    assert_eq!(
        finalize_account_identity(
            &mut connection,
            pending.id,
            AccountIdentity {
                uid: Some("uid-1".into()),
                sec_uid: Some("sec-2".into()),
                nickname: None,
                avatar_url: None
            }
        )
        .unwrap_err(),
        "account_identity_conflict"
    );
    assert!(paths.account_root(&pending.profile_key).unwrap().exists());
    assert!(paths
        .account_root("11111111111111111111111111111111")
        .unwrap()
        .exists());
    assert!(paths
        .account_root("22222222222222222222222222222222")
        .unwrap()
        .exists());
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn collector_identity_script_reads_verified_state_without_nickname_fallback() {
    let script = douyin_history_archive::collector::runner::account_identity_script();
    assert!(script.contains("isLogin"));
    assert!(script.contains("uid"));
    assert!(script.contains("secUid"));
    assert!(script.contains("__INITIAL_STATE__"));
    assert!(script.contains("identitySourcePaths"));
    assert!(script.contains("__EMILY_ACCOUNT_IDENTITY_FIXTURE__"));
    assert!(!script.contains("nickname || uid"));
    assert!(!script.contains("nickname || secUid"));
    assert!(!script.contains("__reactFiber$"));
    assert!(!script.contains("Object.values"));
    assert!(!script.contains("queue.shift"));
}

#[test]
fn completing_login_rejects_whitespace_identity_before_duplicate_matching() {
    let paths = temp_paths("whitespace-duplicate");
    fs::create_dir_all(&paths.temp).expect("temp");
    let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    connection
        .execute(
            "INSERT INTO accounts (uid, profile_key, status, created_at, updated_at) VALUES ('112233445566', ?1, 'active', 'x', 'x')",
            ["11111111111111111111111111111111"],
        )
        .expect("existing account");
    let pending = create_pending_account(&mut connection).expect("pending");
    create_login_attempt(&paths, pending.id, ATTEMPT_ID, &pending.profile_key).expect("attempt");
    let mut identity = valid_identity_json();
    identity["accountId"] = json!(pending.id);
    identity["profileKey"] = json!(pending.profile_key);
    identity["uid"] = json!(" 112233445566 ");
    fs::write(
        paths.login_identity_path(pending.id, ATTEMPT_ID).unwrap(),
        serde_json::to_vec(&identity).unwrap(),
    )
    .expect("identity");
    drop(connection);

    assert_eq!(
        complete_login_attempt(&paths, pending.id, ATTEMPT_ID).unwrap_err(),
        "invalid_account_identity"
    );
    let connection = Connection::open(&paths.database).expect("database");
    assert_eq!(
        get_account(&connection, pending.id)
            .unwrap()
            .unwrap()
            .status,
        "pending_login"
    );
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn completing_login_revalidates_context_finalizes_account_and_sets_active_bootstrap_state() {
    let paths = temp_paths("complete");
    fs::create_dir_all(&paths.temp).expect("temp");
    let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    let pending = create_pending_account(&mut connection).expect("pending");
    create_login_attempt(&paths, pending.id, ATTEMPT_ID, &pending.profile_key).expect("attempt");
    let mut identity = valid_identity_json();
    identity["accountId"] = json!(pending.id);
    identity["profileKey"] = json!(pending.profile_key);
    fs::write(
        paths.login_identity_path(pending.id, ATTEMPT_ID).unwrap(),
        serde_json::to_vec(&identity).unwrap(),
    )
    .expect("identity");
    drop(connection);

    let account = complete_login_attempt(&paths, pending.id, ATTEMPT_ID).expect("complete login");
    assert_eq!(account.status, "active");
    assert_eq!(account.uid.as_deref(), Some("112233445566"));
    assert!(!paths
        .login_attempt_path(pending.id, ATTEMPT_ID)
        .unwrap()
        .exists());
    assert!(!paths
        .login_identity_path(pending.id, ATTEMPT_ID)
        .unwrap()
        .exists());

    assert_eq!(
        read_bootstrap(&paths.bootstrap_dir)
            .expect("bootstrap")
            .expect("bootstrap value")
            .active_account_id,
        Some(pending.id)
    );
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn login_completion_boundary_keeps_active_account_when_bootstrap_updates_fail() {
    let paths = temp_paths("completion-warning");
    fs::create_dir_all(&paths.temp).expect("temp");
    let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    let pending = create_pending_account(&mut connection).expect("pending");
    create_login_attempt(&paths, pending.id, ATTEMPT_ID, &pending.profile_key).expect("attempt");
    let mut identity = valid_identity_json();
    identity["accountId"] = json!(pending.id);
    identity["profileKey"] = json!(pending.profile_key);
    fs::write(
        paths.login_identity_path(pending.id, ATTEMPT_ID).unwrap(),
        serde_json::to_vec(&identity).unwrap(),
    )
    .expect("identity");
    let profile = paths.account_root(&pending.profile_key).unwrap();
    fs::create_dir_all(&profile).expect("profile");
    drop(connection);
    let mut writes = 0;

    let completed = complete_login_attempt_with_active_writer(
        &paths,
        pending.id,
        ATTEMPT_ID,
        |_path, _active_account_id| {
            writes += 1;
            Err("disk full".to_string())
        },
    )
    .expect("database activation remains successful");

    assert_eq!(writes, 2);
    assert_eq!(completed.account.status, "active");
    assert_eq!(
        completed.active_account_warning.as_deref(),
        Some("bootstrap_update_failed")
    );
    let connection = Connection::open(&paths.database).expect("database");
    assert_eq!(get_account(&connection, pending.id).unwrap().unwrap().status, "active");
    assert!(profile.exists());
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn completing_login_rejects_mismatched_identity_without_changing_account_state() {
    let paths = temp_paths("mismatch");
    fs::create_dir_all(&paths.temp).expect("temp");
    let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    let pending = create_pending_account(&mut connection).expect("pending");
    create_login_attempt(&paths, pending.id, ATTEMPT_ID, &pending.profile_key).expect("attempt");
    fs::write(
        paths.login_identity_path(pending.id, ATTEMPT_ID).unwrap(),
        serde_json::to_vec(&valid_identity_json()).unwrap(),
    )
    .expect("identity");
    drop(connection);

    assert_eq!(
        complete_login_attempt(&paths, pending.id, ATTEMPT_ID).unwrap_err(),
        "account_identity_mismatch"
    );
    let connection = Connection::open(&paths.database).expect("database");
    assert_eq!(
        get_account(&connection, pending.id)
            .unwrap()
            .unwrap()
            .status,
        "pending_login"
    );
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn startup_recovery_terminates_only_recorded_attempts_and_removes_incomplete_identity() {
    let paths = temp_paths("recovery");
    fs::create_dir_all(&paths.temp).expect("temp");
    let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    connection
        .execute(
            "INSERT INTO accounts (id, profile_key, status, created_at, updated_at) VALUES (7, ?1, 'pending_login', 'x', 'x')",
            [PROFILE_KEY],
        )
        .expect("account");
    create_login_attempt(&paths, 7, ATTEMPT_ID, PROFILE_KEY).expect("attempt");
    fs::write(
        paths.login_identity_path(7, ATTEMPT_ID).unwrap(),
        b"incomplete",
    )
    .expect("identity");
    let expected_profile = browser_profile_path(
        &paths.account_browser_root(PROFILE_KEY).unwrap(),
        BrowserId::Edge,
    );
    fs::write(
        paths.login_browser_pid_path(7, ATTEMPT_ID).unwrap(),
        serde_json::to_vec(&browser_record(&expected_profile)).unwrap(),
    )
    .expect("pid");
    let mut terminated = Vec::new();

    recover_login_attempts_with(&paths, &mut connection, |record, profile| {
        terminated.push((record.pid, profile.to_path_buf()));
        Ok(())
    })
    .expect("recover");

    assert_eq!(terminated, vec![(4321, expected_profile)]);
    assert!(!paths.login_attempt_path(7, ATTEMPT_ID).unwrap().exists());
    assert!(!paths.login_identity_path(7, ATTEMPT_ID).unwrap().exists());
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn startup_recovery_removes_pending_account_even_without_attempt_files() {
    let paths = temp_paths("recovery-pending-without-attempt");
    fs::create_dir_all(&paths.temp).expect("temp");
    let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    let pending = create_pending_account(&mut connection).expect("pending");
    let profile = paths.account_root(&pending.profile_key).unwrap();
    fs::create_dir_all(&profile).expect("profile");

    recover_login_attempts_with(&paths, &mut connection, |_record, _profile| Ok(()))
        .expect("recover");

    assert!(get_account(&connection, pending.id).unwrap().is_none());
    assert!(!profile.exists());
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn failed_login_deletes_pending_row_and_defers_locked_profile_cleanup() {
    let paths = temp_paths("deferred-profile-cleanup");
    fs::create_dir_all(&paths.temp).expect("temp");
    fs::create_dir_all(&paths.browser_profiles).expect("profiles");
    let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    let pending = create_pending_account(&mut connection).expect("pending");
    let profile = paths.account_root(&pending.profile_key).unwrap();
    fs::create_dir_all(&profile).expect("profile");

    let report = cleanup_failed_login_with(
        &paths,
        &mut connection,
        pending.id,
        ATTEMPT_ID,
        |_profile| Err("sharing_violation".to_string()),
    );

    assert!(get_account(&connection, pending.id).unwrap().is_none());
    assert!(report.deferred_profile);
    assert_eq!(report.primary_error.as_deref(), Some("sharing_violation"));
    let marker = paths
        .pending_profile_cleanup_path(&pending.profile_key)
        .unwrap();
    assert!(marker.exists());
    assert!(profile.exists());

    retry_pending_profile_cleanups_with(&paths, |path| {
        fs::remove_dir_all(path).map_err(|error| error.to_string())
    })
    .expect("retry");
    assert!(!profile.exists());
    assert!(!marker.exists());
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn failed_login_cleanup_never_deletes_an_already_active_account_or_profile() {
    let paths = temp_paths("active-cleanup-guard");
    fs::create_dir_all(&paths.temp).expect("temp");
    fs::create_dir_all(&paths.browser_profiles).expect("profiles");
    let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    let pending = create_pending_account(&mut connection).expect("pending");
    let active = finalize_account_identity(
        &mut connection,
        pending.id,
        AccountIdentity {
            uid: Some("active-uid".to_string()),
            sec_uid: None,
            nickname: None,
            avatar_url: None,
        },
    )
    .expect("active");
    let profile = paths.account_root(&active.profile_key).unwrap();
    fs::create_dir_all(&profile).expect("profile");

    let report = cleanup_failed_login(&paths, &mut connection, active.id, ATTEMPT_ID);

    assert!(report.primary_error.is_none());
    assert_eq!(get_account(&connection, active.id).unwrap().unwrap().status, "active");
    assert!(profile.exists());
    let _ = fs::remove_dir_all(paths.app_data);
}

#[tokio::test]
async fn pending_login_guard_cleans_every_prelaunch_failure_stage() {
    for stage in ["create_attempt", "build_invocation", "create_log", "spawn_collector"] {
        let paths = temp_paths(stage);
        fs::create_dir_all(&paths.temp).expect("temp");
        fs::create_dir_all(&paths.browser_profiles).expect("profiles");
        let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
        let pending = create_pending_account(&mut connection).expect("pending");
        let profile = paths.account_root(&pending.profile_key).unwrap();
        fs::create_dir_all(&profile).expect("profile");
        drop(connection);

        let result: Result<(), String> = run_pending_login_guarded(
            &paths,
            pending.id,
            ATTEMPT_ID,
            async { Err(stage.to_string()) },
        )
        .await;

        assert_eq!(result, Err(stage.to_string()));
        let connection = Connection::open(&paths.database).expect("database");
        assert!(get_account(&connection, pending.id).unwrap().is_none());
        assert!(!profile.exists());
        let _ = fs::remove_dir_all(paths.app_data);
    }
}

#[test]
fn startup_recovery_ignores_corrupt_and_profile_mismatched_pid_records() {
    let paths = temp_paths("unsafe-recovery");
    fs::create_dir_all(&paths.temp).expect("temp");
    let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    connection
        .execute(
            "INSERT INTO accounts (id, profile_key, status, created_at, updated_at) VALUES (7, ?1, 'pending_login', 'x', 'x')",
            [PROFILE_KEY],
        )
        .expect("account");
    create_login_attempt(&paths, 7, ATTEMPT_ID, PROFILE_KEY).expect("first attempt");
    create_login_attempt(&paths, 7, SECOND_ATTEMPT_ID, PROFILE_KEY).expect("second attempt");

    fs::write(
        paths.login_browser_pid_path(7, ATTEMPT_ID).unwrap(),
        serde_json::to_vec(&browser_record(std::path::Path::new(
            r"C:\Other\account\edge",
        )))
        .unwrap(),
    )
    .expect("mismatched pid");
    fs::write(
        paths
            .login_browser_pid_path(7, SECOND_ATTEMPT_ID)
            .unwrap(),
        b"{ damaged pid record",
    )
    .expect("damaged pid");
    let mut terminated = Vec::new();

    recover_login_attempts_with(&paths, &mut connection, |record, _| {
        terminated.push(record.pid);
        Ok(())
    })
    .expect("recover");

    assert!(terminated.is_empty());
    for attempt_id in [ATTEMPT_ID, SECOND_ATTEMPT_ID] {
        assert!(!paths.login_attempt_path(7, attempt_id).unwrap().exists());
        assert!(!paths
            .login_browser_pid_path(7, attempt_id)
            .unwrap()
            .exists());
    }
    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn durable_login_attempt_does_not_create_an_account_and_rejects_stale_revision() {
    let paths = temp_paths("durable-state");
    fs::create_dir_all(&paths.temp).expect("temp");
    let mut connection = Connection::open(&paths.database).expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");

    let preparing = create_login_attempt_state(
        &paths,
        ATTEMPT_ID,
        PROFILE_KEY,
        None,
        "2026-07-19T12:00:00.000Z",
    )
    .expect("create attempt state");

    assert_eq!(preparing.revision, 0);
    assert_eq!(preparing.lifecycle, LoginAttemptLifecycle::Preparing);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );

    let running = preparing
        .next(LoginAttemptLifecycle::Running, "2026-07-19T12:00:01.000Z")
        .expect("next state");
    compare_and_swap_login_attempt(&paths, &preparing, &running).expect("advance state");
    assert_eq!(read_login_attempt_state(&paths, ATTEMPT_ID).unwrap(), running);
    assert_eq!(
        compare_and_swap_login_attempt(&paths, &preparing, &running).unwrap_err(),
        "login_attempt_state_conflict"
    );

    let _ = fs::remove_dir_all(paths.app_data);
}

#[test]
fn relogin_attempt_is_bound_to_the_existing_account_and_profile() {
    let paths = temp_paths("durable-relogin");
    fs::create_dir_all(&paths.temp).expect("temp");
    let state = create_login_attempt_state(
        &paths,
        SECOND_ATTEMPT_ID,
        PROFILE_KEY,
        Some(42),
        "2026-07-19T12:00:00.000Z",
    )
    .expect("relogin state");

    assert_eq!(state.target_account_id, Some(42));
    assert_eq!(state.profile_key, PROFILE_KEY);
    assert_eq!(state.login_attempt_id, SECOND_ATTEMPT_ID);
    let _ = fs::remove_dir_all(paths.app_data);
}
