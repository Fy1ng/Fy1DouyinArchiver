use douyin_history_archive::accounts::{
    add_local_folder_provenance, archive_account, cancel_pending_account,
    create_account_from_identity, create_pending_account, delete_account_data,
    delete_account_data_with_state_writers, finalize_account_identity,
    finish_sync_with_account_state, list_accounts, load_active_account, restore_account,
    AccountFolderDisposition, AccountIdentity, SyncJobIdentity, SyncTerminalOutcome,
};
use douyin_history_archive::bootstrap::{read_bootstrap, write_bootstrap_atomic, BootstrapV1};
use douyin_history_archive::migrate_schema_to_v5_at_startup;
use douyin_history_archive::sync::{
    BatchJobRoleV1, NewSyncJobV1, QueueStore, SyncBatchStatusV1, SyncScopeV1, SyncSourceV1,
    SyncTriggerV1, TaskKeyV1,
};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("database");
    migrate_schema_to_v5_at_startup(&mut connection).expect("schema");
    connection
}

#[test]
fn successful_new_login_creates_the_first_active_account_directly() {
    let mut connection = database();
    assert!(list_accounts(&connection).unwrap().is_empty());

    let account = create_account_from_identity(
        &mut connection,
        "abcdef0123456789abcdef0123456789",
        AccountIdentity {
            uid: Some("uid-new-login".into()),
            sec_uid: Some("sec-new-login".into()),
            nickname: Some("New Login".into()),
            avatar_url: None,
        },
    )
    .expect("create account after login");

    assert_eq!(account.status, "active");
    assert_eq!(account.profile_key, "abcdef0123456789abcdef0123456789");
    assert_eq!(
        connection
            .query_row(
                "SELECT revision FROM remote_folder_registry_state WHERE account_id = ?1",
                [account.id],
                |row| row.get::<_, i64>(0),
            )
            .expect("registry state"),
        1
    );
    assert_eq!(list_accounts(&connection).unwrap(), vec![account]);
}

#[test]
fn pending_account_creates_registry_state_in_the_same_transaction() {
    let mut connection = database();
    let account = create_pending_account(&mut connection).expect("pending account");
    assert_eq!(
        connection
            .query_row(
                "SELECT revision FROM remote_folder_registry_state WHERE account_id = ?1",
                [account.id],
                |row| row.get::<_, i64>(0),
            )
            .expect("registry state"),
        1
    );
}

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "emily-accounts-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temp dir");
    write_bootstrap_atomic(
        &path,
        &BootstrapV1::new(path.clone(), "0123456789abcdef0123456789abcdef".to_string())
            .expect("bootstrap"),
    )
    .expect("write bootstrap");
    path
}

fn identity(uid: Option<&str>, sec_uid: Option<&str>) -> AccountIdentity {
    AccountIdentity {
        uid: uid.map(str::to_string),
        sec_uid: sec_uid.map(str::to_string),
        nickname: Some(" New Nickname ".to_string()),
        avatar_url: Some("https://example.com/avatar.png".to_string()),
    }
}

fn insert_account(
    connection: &Connection,
    uid: Option<&str>,
    sec_uid: Option<&str>,
    profile_key: &str,
    status: &str,
) -> i64 {
    connection
        .execute(
            "INSERT INTO accounts
             (uid, sec_uid, profile_key, nickname, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'Old Nickname', ?4, 'x', 'x')",
            params![uid, sec_uid, profile_key, status],
        )
        .expect("account");
    connection.last_insert_rowid()
}

fn insert_video(connection: &Connection, aweme_id: &str) -> i64 {
    connection
        .execute(
            "INSERT INTO videos
             (aweme_id, canonical_url, content_type, first_seen_at, last_seen_at)
             VALUES (?1, ?2, 'video', 'x', 'x')",
            params![aweme_id, format!("https://www.douyin.com/video/{aweme_id}")],
        )
        .expect("video");
    connection.last_insert_rowid()
}

fn insert_running_sync_job(connection: &Connection, account_id: i64) -> SyncJobIdentity {
    connection
        .execute(
            "INSERT INTO sync_jobs
             (account_id, source, source_instance_key, trigger, queued_at, queue_seq,
              settings_revision, payload_json, payload_sha256, equivalence_sha256,
              started_at, status)
             VALUES (?1, 'history', '', 'manual', 'queued', 1, 1, '{}', ?2, ?2,
                     'started', 'running')",
            params![account_id, "a".repeat(64)],
        )
        .expect("sync job");
    SyncJobIdentity {
        account_id,
        sync_job_id: connection.last_insert_rowid(),
    }
}

fn enqueue_account_job(
    connection: &mut Connection,
    account_id: i64,
    source: SyncSourceV1,
) -> douyin_history_archive::sync::StoredSyncJobV1 {
    QueueStore::enqueue_job(
        connection,
        &NewSyncJobV1 {
            task_key: TaskKeyV1 {
                account_id,
                source,
                source_instance_key: String::new(),
            },
            trigger: SyncTriggerV1::Manual,
            queued_at: "2026-08-12T04:00:00.000Z".to_string(),
            scheduled_for: None,
            settings_revision: 0,
            config_sha256: None,
            payload_json: "{}".to_string(),
            payload_sha256: "a".repeat(64),
            equivalence_sha256: "b".repeat(64),
        },
    )
    .expect("queued account job")
}

fn account_and_job_status(
    connection: &Connection,
    job: &SyncJobIdentity,
) -> (String, String, Option<String>) {
    let account_status = connection
        .query_row(
            "SELECT status FROM accounts WHERE id = ?1",
            [job.account_id],
            |row| row.get(0),
        )
        .expect("account status");
    let (job_status, error_code) = connection
        .query_row(
            "SELECT status, error_code FROM sync_jobs WHERE id = ?1",
            [job.sync_job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("job status");
    (account_status, job_status, error_code)
}

fn insert_local_folder(
    connection: &Connection,
    scope: &str,
    owner_account_id: Option<i64>,
    name: &str,
) -> i64 {
    connection
        .execute(
            "INSERT INTO local_folders
             (scope, owner_account_id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'x', 'x')",
            params![scope, owner_account_id, name],
        )
        .expect("folder");
    connection.last_insert_rowid()
}

fn add_local_item(connection: &Connection, folder_id: i64, video_id: i64) {
    connection
        .execute(
            "INSERT INTO local_folder_items (folder_id, video_id, added_at)
             VALUES (?1, ?2, 'x')",
            params![folder_id, video_id],
        )
        .expect("folder item");
}

fn scalar_i64(connection: &Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get(0))
        .expect("scalar")
}

#[test]
fn pending_account_uses_lowercase_simple_uuid_and_lists_it() {
    let mut connection = database();
    let account = create_pending_account(&mut connection).expect("pending account");

    assert_eq!(account.status, "pending_login");
    assert_eq!(account.profile_key.len(), 32);
    assert!(account
        .profile_key
        .chars()
        .all(|value| value.is_ascii_digit() || ('a'..='f').contains(&value)));
    assert_eq!(list_accounts(&connection).expect("accounts"), vec![account]);
}

#[test]
fn identity_matrix_completes_unmatched_pending() {
    let mut connection = database();
    let pending = create_pending_account(&mut connection).expect("pending");

    let completed = finalize_account_identity(
        &mut connection,
        pending.id,
        identity(Some(" uid-1 "), Some(" sec-1 ")),
    )
    .expect("complete pending");
    assert_eq!(completed.uid.as_deref(), Some("uid-1"));
    assert_eq!(completed.sec_uid.as_deref(), Some("sec-1"));
    assert_eq!(completed.nickname.as_deref(), Some("New Nickname"));
    assert_eq!(completed.status, "active");
    assert_eq!(completed.profile_key, pending.profile_key);
}

#[test]
fn identity_matrix_accepts_pending_account_matching_its_own_ids() {
    let mut connection = database();
    let pending = create_pending_account(&mut connection).expect("pending");
    connection
        .execute(
            "UPDATE accounts SET uid = 'uid-1', sec_uid = 'sec-1' WHERE id = ?1",
            [pending.id],
        )
        .expect("seed pending identity");

    let completed = finalize_account_identity(
        &mut connection,
        pending.id,
        identity(Some("uid-1"), Some("sec-1")),
    )
    .expect("pending self match");
    assert_eq!(completed.id, pending.id);
    assert_eq!(completed.status, "active");
    assert_eq!(completed.profile_key, pending.profile_key);
}

#[test]
fn identity_matrix_updates_same_active_expired_or_conflicted_account_without_rotating_profile() {
    for status in ["active", "login_expired", "identity_conflict"] {
        let mut connection = database();
        let account_id = insert_account(&connection, Some("uid-1"), None, "stable-profile", status);

        let account = finalize_account_identity(
            &mut connection,
            account_id,
            identity(Some("uid-1"), Some("sec-1")),
        )
        .expect("resume account");

        assert_eq!(account.id, account_id);
        assert_eq!(account.sec_uid.as_deref(), Some("sec-1"));
        assert_eq!(account.status, "active");
        assert_eq!(account.profile_key, "stable-profile");
        assert!(account.last_login_validated_at.is_some());
    }
}

#[test]
fn login_failures_expire_account_and_fail_the_same_running_job() {
    for error_code in ["login_recovery_failed", "session_expired"] {
        let mut connection = database();
        let account_id = insert_account(
            &connection,
            Some("uid-1"),
            Some("sec-1"),
            "profile-1",
            "active",
        );
        let job = insert_running_sync_job(&connection, account_id);

        finish_sync_with_account_state(
            &mut connection,
            job.clone(),
            SyncTerminalOutcome::Failed {
                error_code: error_code.to_string(),
                error_message: Some("login failed".to_string()),
            },
        )
        .expect("finish failed sync");

        assert_eq!(
            account_and_job_status(&connection, &job),
            (
                "login_expired".to_string(),
                "failed".to_string(),
                Some(error_code.to_string()),
            )
        );
    }
}

#[test]
fn identity_conflict_failure_marks_account_and_job_atomically() {
    let mut connection = database();
    let account_id = insert_account(
        &connection,
        Some("uid-1"),
        Some("sec-1"),
        "profile-1",
        "active",
    );
    let job = insert_running_sync_job(&connection, account_id);

    finish_sync_with_account_state(
        &mut connection,
        job.clone(),
        SyncTerminalOutcome::Failed {
            error_code: "account_identity_conflict".to_string(),
            error_message: Some("identity conflict".to_string()),
        },
    )
    .expect("finish conflict");

    assert_eq!(
        account_and_job_status(&connection, &job),
        (
            "identity_conflict".to_string(),
            "failed".to_string(),
            Some("account_identity_conflict".to_string()),
        )
    );
}

#[test]
fn technical_failure_and_cancellation_leave_account_state_unchanged() {
    for outcome in [
        SyncTerminalOutcome::Failed {
            error_code: "cdp_connection_failed".to_string(),
            error_message: Some("technical failure".to_string()),
        },
        SyncTerminalOutcome::Cancelled {
            error_message: Some("cancelled".to_string()),
        },
    ] {
        let mut connection = database();
        let account_id = insert_account(
            &connection,
            Some("uid-1"),
            Some("sec-1"),
            "profile-1",
            "active",
        );
        let job = insert_running_sync_job(&connection, account_id);

        finish_sync_with_account_state(&mut connection, job.clone(), outcome.clone())
            .expect("finish sync");

        let (account_status, job_status, _) = account_and_job_status(&connection, &job);
        assert_eq!(account_status, "active");
        assert_eq!(
            job_status,
            if matches!(outcome, SyncTerminalOutcome::Cancelled { .. }) {
                "cancelled"
            } else {
                "failed"
            }
        );
    }
}

#[test]
fn terminal_account_sync_outcomes_refresh_linked_batch_snapshots() {
    let mut connection = database();
    let account_id = insert_account(
        &connection,
        Some("uid-1"),
        Some("sec-1"),
        "profile-1",
        "active",
    );
    let job = insert_running_sync_job(&connection, account_id);
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T00:00:00.000Z",
    )
    .unwrap();
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        job.sync_job_id,
        BatchJobRoleV1::Root,
        "2026-08-12T00:00:01.000Z",
    )
    .unwrap();

    finish_sync_with_account_state(
        &mut connection,
        job,
        SyncTerminalOutcome::Failed {
            error_code: "cdp_connection_failed".to_string(),
            error_message: Some("technical failure".to_string()),
        },
    )
    .unwrap();

    assert_eq!(
        QueueStore::get_batch(&connection, batch_id).unwrap().status,
        SyncBatchStatusV1::Failed
    );
    let entries = QueueStore::list_batch_entries(&connection, batch_id, None, 200).unwrap();
    let douyin_history_archive::sync::SyncBatchEntryV1::Job(snapshot) = &entries.entries[0] else {
        panic!("expected job snapshot");
    };
    assert_eq!(
        snapshot.status,
        douyin_history_archive::sync::SyncJobStatusV1::Failed
    );
}

#[test]
fn failed_job_update_rolls_back_identity_conflict_account_change() {
    let mut connection = database();
    let account_id = insert_account(
        &connection,
        Some("uid-1"),
        Some("sec-1"),
        "profile-1",
        "active",
    );
    let job = insert_running_sync_job(&connection, account_id);
    connection
        .execute_batch(
            "
            CREATE TRIGGER reject_sync_finish
            BEFORE UPDATE OF status ON sync_jobs
            WHEN NEW.id = OLD.id AND NEW.status = 'failed'
            BEGIN
              SELECT RAISE(ABORT, 'blocked sync finish');
            END;
            ",
        )
        .expect("failure trigger");

    assert!(finish_sync_with_account_state(
        &mut connection,
        job.clone(),
        SyncTerminalOutcome::Failed {
            error_code: "account_identity_conflict".to_string(),
            error_message: Some("identity conflict".to_string()),
        },
    )
    .is_err());

    assert_eq!(
        account_and_job_status(&connection, &job),
        ("active".to_string(), "running".to_string(), None)
    );
}

#[test]
fn identity_matrix_accepts_uid_and_sec_uid_matching_the_same_active_or_expired_account() {
    for status in ["active", "login_expired"] {
        let mut connection = database();
        let account_id = insert_account(
            &connection,
            Some("uid-1"),
            Some("sec-1"),
            "stable-profile",
            status,
        );

        let account = finalize_account_identity(
            &mut connection,
            account_id,
            identity(Some("uid-1"), Some("sec-1")),
        )
        .expect("both ids match the same account");

        assert_eq!(account.id, account_id);
        assert_eq!(account.status, "active");
        assert_eq!(account.profile_key, "stable-profile");
    }
}

#[test]
fn identity_matrix_rejects_conflicting_existing_ids_and_split_matches() {
    let mut connection = database();
    let account_id = insert_account(
        &connection,
        Some("uid-1"),
        Some("sec-1"),
        "profile-1",
        "active",
    );
    assert_eq!(
        finalize_account_identity(
            &mut connection,
            account_id,
            identity(Some("uid-1"), Some("sec-other")),
        )
        .unwrap_err(),
        "account_identity_conflict"
    );

    let second_id = insert_account(
        &connection,
        Some("uid-2"),
        Some("sec-2"),
        "profile-2",
        "active",
    );
    assert_eq!(
        finalize_account_identity(
            &mut connection,
            second_id,
            identity(Some("uid-1"), Some("sec-2")),
        )
        .unwrap_err(),
        "account_identity_conflict"
    );
}

#[test]
fn identity_matrix_rejects_archived_and_existing_account_for_new_pending() {
    let mut connection = database();
    let archived_id = insert_account(
        &connection,
        Some("uid-archived"),
        None,
        "archived-profile",
        "archived",
    );
    assert_eq!(
        finalize_account_identity(
            &mut connection,
            archived_id,
            identity(Some("uid-archived"), None),
        )
        .unwrap_err(),
        "account_archived_requires_restore"
    );

    insert_account(
        &connection,
        Some("uid-existing"),
        Some("sec-existing"),
        "existing-profile",
        "active",
    );
    let pending = create_pending_account(&mut connection).expect("pending");
    assert_eq!(
        finalize_account_identity(
            &mut connection,
            pending.id,
            identity(Some("uid-existing"), Some("sec-existing")),
        )
        .unwrap_err(),
        "account_already_exists"
    );
    let preserved = list_accounts(&connection)
        .expect("accounts")
        .into_iter()
        .find(|account| account.id == pending.id)
        .expect("pending preserved");
    assert_eq!(preserved.status, "pending_login");
    assert_eq!(preserved.profile_key, pending.profile_key);
}

#[test]
fn identity_validation_enforces_lengths_urls_and_nonempty_remote_identity() {
    let mut connection = database();
    let pending = create_pending_account(&mut connection).expect("pending");

    for invalid in [
        AccountIdentity {
            uid: None,
            sec_uid: None,
            nickname: None,
            avatar_url: None,
        },
        identity(Some(&"u".repeat(257)), None),
        AccountIdentity {
            avatar_url: Some("file:///avatar.png".to_string()),
            ..identity(Some("uid"), None)
        },
        AccountIdentity {
            nickname: Some("n".repeat(201)),
            ..identity(Some("uid"), None)
        },
    ] {
        assert_eq!(
            finalize_account_identity(&mut connection, pending.id, invalid).unwrap_err(),
            "invalid_account_identity"
        );
    }
}

#[test]
fn cancel_pending_and_restore_account_obey_status_and_profile_boundaries() {
    let mut connection = database();
    let root = temp_dir("cancel");
    let profiles = root.join("profiles");
    fs::create_dir_all(&profiles).expect("profiles");
    let pending = create_pending_account(&mut connection).expect("pending");
    let profile = profiles.join(&pending.profile_key);
    fs::create_dir_all(profile.join("edge")).expect("profile");

    cancel_pending_account(&mut connection, pending.id, &profiles).expect("cancel pending");
    assert!(!profile.exists());
    assert!(list_accounts(&connection).expect("accounts").is_empty());

    let archived_id = insert_account(
        &connection,
        Some("uid-archived"),
        None,
        "restore-profile",
        "archived",
    );
    assert_eq!(
        restore_account(&mut connection, archived_id)
            .expect("restore")
            .status,
        "login_expired"
    );

    let outside = root.join("outside");
    fs::create_dir_all(&outside).expect("outside");
    let unsafe_id = insert_account(&connection, None, None, "../outside", "pending_login");
    assert_eq!(
        cancel_pending_account(&mut connection, unsafe_id, &profiles).unwrap_err(),
        "profile_path_not_controlled"
    );
    assert!(outside.exists());
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn archive_current_account_selects_active_fallback() {
    let mut connection = database();
    let first = insert_account(&connection, Some("uid-1"), None, "profile-1", "active");
    let second = insert_account(&connection, Some("uid-2"), None, "profile-2", "active");
    let app_data = temp_dir("archive");
    douyin_history_archive::bootstrap::update_active_account(&app_data, Some(first))
        .expect("active account");

    archive_account(&mut connection, &app_data, first).expect("archive");

    assert_eq!(
        read_bootstrap(&app_data)
            .expect("bootstrap")
            .expect("bootstrap value")
            .active_account_id,
        Some(second)
    );
    fs::remove_dir_all(app_data).expect("cleanup");
}

#[test]
fn archive_cancels_queued_jobs_and_rejects_accounts_with_running_jobs() {
    let mut connection = database();
    let queued_account = insert_account(
        &connection,
        Some("uid-queued"),
        None,
        "queued-profile",
        "active",
    );
    let running_account = insert_account(
        &connection,
        Some("uid-running"),
        None,
        "running-profile",
        "active",
    );
    let app_data = temp_dir("archive-queue");
    douyin_history_archive::bootstrap::update_active_account(&app_data, Some(queued_account))
        .expect("active account");
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T04:00:00.000Z",
    )
    .unwrap();
    let queued = enqueue_account_job(&mut connection, queued_account, SyncSourceV1::History);
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        queued.snapshot.id,
        douyin_history_archive::sync::BatchJobRoleV1::Root,
        "2026-08-12T04:00:00.000Z",
    )
    .unwrap();

    archive_account(&mut connection, &app_data, queued_account).expect("archive queued account");
    assert_eq!(
        connection
            .query_row(
                "SELECT status, error_code FROM sync_jobs WHERE id = ?1",
                [queued.snapshot.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap(),
        ("cancelled".to_string(), "account_archived".to_string())
    );
    assert_eq!(
        QueueStore::get_batch(&connection, batch_id).unwrap().status,
        douyin_history_archive::sync::SyncBatchStatusV1::Cancelled
    );

    let running = enqueue_account_job(&mut connection, running_account, SyncSourceV1::History);
    QueueStore::claim_job(
        &mut connection,
        running.snapshot.id,
        "2026-08-12T04:01:00.000Z",
    )
    .unwrap();
    assert_eq!(
        archive_account(&mut connection, &app_data, running_account).unwrap_err(),
        "account_sync_running"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT status FROM accounts WHERE id = ?1",
                [running_account],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "active"
    );
    fs::remove_dir_all(app_data).expect("cleanup");
}

#[test]
fn delete_cancels_queued_jobs_and_preserves_the_final_batch_snapshot() {
    let mut connection = database();
    let account_id = insert_account(
        &connection,
        Some("uid-delete-queue"),
        None,
        "delete-queue-profile",
        "active",
    );
    let root = temp_dir("delete-queue");
    let profiles = root.join("profiles");
    fs::create_dir_all(profiles.join("delete-queue-profile")).expect("profile");
    douyin_history_archive::bootstrap::update_active_account(&root, Some(account_id))
        .expect("active account");
    let batch_id = QueueStore::create_batch(
        &mut connection,
        SyncScopeV1::CurrentAccountAvailableSources,
        "2026-08-12T04:00:00.000Z",
    )
    .unwrap();
    let queued = enqueue_account_job(&mut connection, account_id, SyncSourceV1::History);
    QueueStore::attach_job_to_batch(
        &mut connection,
        batch_id,
        queued.snapshot.id,
        douyin_history_archive::sync::BatchJobRoleV1::Root,
        "2026-08-12T04:00:00.000Z",
    )
    .unwrap();

    delete_account_data(
        &mut connection,
        &root,
        &root,
        &profiles,
        account_id,
        AccountFolderDisposition::Delete,
    )
    .expect("delete account");
    assert_eq!(
        connection
            .query_row(
                "SELECT job_id, json_extract(job_view_snapshot_json, '$.status')
                 FROM sync_batch_jobs WHERE batch_id = ?1",
                [batch_id],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap(),
        (None, "cancelled".to_string())
    );
    assert_eq!(
        QueueStore::get_batch(&connection, batch_id).unwrap().status,
        douyin_history_archive::sync::SyncBatchStatusV1::Cancelled
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn provenance_is_only_allowed_for_aggregate_folders() {
    let mut connection = database();
    let account_id = insert_account(&connection, Some("uid-1"), None, "profile-1", "active");
    let video_id = insert_video(&connection, "video-1");
    let account_folder = insert_local_folder(&connection, "account", Some(account_id), "Mine");
    add_local_item(&connection, account_folder, video_id);

    assert_eq!(
        add_local_folder_provenance(&mut connection, account_folder, video_id, account_id,)
            .unwrap_err(),
        "local_folder_provenance_requires_aggregate"
    );
}

#[test]
fn delete_converts_account_folder_to_aggregate_with_owner_provenance() {
    let mut connection = database();
    let account_id = insert_account(&connection, Some("uid-1"), None, "profile-1", "active");
    let video_id = insert_video(&connection, "video-1");
    let folder_id = insert_local_folder(&connection, "account", Some(account_id), "Mine");
    add_local_item(&connection, folder_id, video_id);
    connection
        .execute_batch(
            "CREATE TEMP TABLE provenance_audit (account_id INTEGER NOT NULL);
             CREATE TEMP TRIGGER audit_provenance_insert
             AFTER INSERT ON main.local_folder_item_accounts
             BEGIN
               INSERT INTO provenance_audit (account_id) VALUES (NEW.account_id);
             END;",
        )
        .expect("provenance audit");
    let app_data = temp_dir("convert");

    delete_account_data(
        &mut connection,
        &app_data,
        &app_data,
        &app_data.join("profiles"),
        account_id,
        AccountFolderDisposition::ConvertToAggregate,
    )
    .expect("delete and convert");

    assert_eq!(
        connection
            .query_row(
                "SELECT scope, owner_account_id FROM local_folders WHERE id = ?1",
                [folder_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .expect("folder"),
        ("aggregate".to_string(), None)
    );
    assert_eq!(
        scalar_i64(
            &connection,
            "SELECT COUNT(*) FROM local_folder_item_accounts"
        ),
        0,
        "deleting the account removes its aggregate provenance after conversion"
    );
    assert_eq!(
        scalar_i64(&connection, "SELECT COUNT(*) FROM provenance_audit"),
        1,
        "conversion writes the original owner provenance before account deletion"
    );
    assert_eq!(
        scalar_i64(&connection, "SELECT COUNT(*) FROM local_folder_items"),
        1
    );
    assert_eq!(scalar_i64(&connection, "SELECT COUNT(*) FROM videos"), 1);
    fs::remove_dir_all(app_data).expect("cleanup");
}

#[test]
fn delete_transfers_account_folder_without_writing_provenance() {
    let mut connection = database();
    let source = insert_account(&connection, Some("uid-1"), None, "profile-1", "active");
    let target = insert_account(&connection, Some("uid-2"), None, "profile-2", "active");
    let video_id = insert_video(&connection, "video-1");
    let folder_id = insert_local_folder(&connection, "account", Some(source), "Mine");
    add_local_item(&connection, folder_id, video_id);
    let app_data = temp_dir("transfer");

    delete_account_data(
        &mut connection,
        &app_data,
        &app_data,
        &app_data.join("profiles"),
        source,
        AccountFolderDisposition::TransferToAccount {
            target_account_id: target,
        },
    )
    .expect("delete and transfer");

    assert_eq!(
        connection
            .query_row(
                "SELECT owner_account_id FROM local_folders WHERE id = ?1",
                [folder_id],
                |row| row.get::<_, i64>(0),
            )
            .expect("owner"),
        target
    );
    assert_eq!(
        scalar_i64(
            &connection,
            "SELECT COUNT(*) FROM local_folder_item_accounts"
        ),
        0
    );
    fs::remove_dir_all(app_data).expect("cleanup");
}

#[test]
fn delete_removes_only_deleted_account_aggregate_provenance_and_keeps_item() {
    let mut connection = database();
    let source = insert_account(&connection, Some("uid-1"), None, "profile-1", "active");
    let other = insert_account(&connection, Some("uid-2"), None, "profile-2", "active");
    let video_id = insert_video(&connection, "video-1");
    let folder_id = insert_local_folder(&connection, "aggregate", None, "Shared");
    add_local_item(&connection, folder_id, video_id);
    for account_id in [source, other] {
        connection
            .execute(
                "INSERT INTO local_folder_item_accounts
                 (folder_id, video_id, account_id, added_at) VALUES (?1, ?2, ?3, 'x')",
                params![folder_id, video_id, account_id],
            )
            .expect("provenance");
    }
    let app_data = temp_dir("provenance-delete");

    delete_account_data(
        &mut connection,
        &app_data,
        &app_data,
        &app_data.join("profiles"),
        source,
        AccountFolderDisposition::Delete,
    )
    .expect("delete");

    assert_eq!(
        scalar_i64(
            &connection,
            "SELECT COUNT(*) FROM local_folder_item_accounts"
        ),
        1
    );
    assert_eq!(
        scalar_i64(&connection, "SELECT COUNT(*) FROM local_folder_items"),
        1
    );
    fs::remove_dir_all(app_data).expect("cleanup");
}

#[test]
fn delete_rejects_folder_name_conflicts_and_invalid_transfer_target() {
    let mut connection = database();
    let source = insert_account(&connection, Some("uid-1"), None, "profile-1", "active");
    let target = insert_account(&connection, Some("uid-2"), None, "profile-2", "active");
    insert_local_folder(&connection, "account", Some(source), "Same");
    insert_local_folder(&connection, "account", Some(target), "Same");
    let app_data = temp_dir("conflict");
    assert_eq!(
        delete_account_data(
            &mut connection,
            &app_data,
            &app_data,
            &app_data.join("profiles"),
            source,
            AccountFolderDisposition::TransferToAccount {
                target_account_id: target,
            },
        )
        .unwrap_err(),
        "local_folder_name_conflict"
    );

    let archived = insert_account(&connection, Some("uid-3"), None, "profile-3", "archived");
    assert_eq!(
        delete_account_data(
            &mut connection,
            &app_data,
            &app_data,
            &app_data.join("profiles"),
            source,
            AccountFolderDisposition::TransferToAccount {
                target_account_id: archived,
            },
        )
        .unwrap_err(),
        "invalid_transfer_account"
    );
    fs::remove_dir_all(app_data).expect("cleanup");
}

#[test]
fn database_commit_survives_bootstrap_write_failure_and_repairs_on_next_read() {
    let mut connection = database();
    let account_id = insert_account(&connection, Some("uid-1"), None, "profile-1", "active");
    let app_data = temp_dir("settings-failure");
    douyin_history_archive::bootstrap::update_active_account(&app_data, Some(account_id))
        .expect("active account");

    assert_eq!(
        delete_account_data_with_state_writers(
            &mut connection,
            &app_data,
            &app_data,
            &app_data.join("profiles"),
            account_id,
            AccountFolderDisposition::Delete,
            |_, _| Err("injected bootstrap write failure".to_string()),
            |_, _| Ok(()),
        )
        .unwrap_err(),
        "bootstrap_update_failed"
    );
    assert_eq!(scalar_i64(&connection, "SELECT COUNT(*) FROM accounts"), 0);
    assert_eq!(load_active_account(&app_data, &connection).unwrap(), None);
    assert_eq!(
        read_bootstrap(&app_data)
            .unwrap()
            .unwrap()
            .active_account_id,
        None
    );
    fs::remove_dir_all(app_data).expect("cleanup");
}

#[test]
fn profile_cleanup_happens_after_commit_and_failure_is_reported() {
    let mut connection = database();
    let account_id = insert_account(&connection, Some("uid-1"), None, "profile-1", "active");
    let root = temp_dir("profile-cleanup");
    let profiles = root.join("profiles");
    fs::create_dir_all(&profiles).expect("profiles");
    fs::write(profiles.join("profile-1"), b"not a directory").expect("profile file");

    assert_eq!(
        delete_account_data(
            &mut connection,
            &root,
            &root,
            &profiles,
            account_id,
            AccountFolderDisposition::Delete,
        )
        .unwrap_err(),
        "profile_cleanup_failed"
    );
    assert_eq!(scalar_i64(&connection, "SELECT COUNT(*) FROM accounts"), 0);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn delete_cleans_only_unreferenced_videos() {
    let mut connection = database();
    let source = insert_account(&connection, Some("uid-1"), None, "profile-1", "active");
    let other = insert_account(&connection, Some("uid-2"), None, "profile-2", "active");
    let orphan = insert_video(&connection, "orphan");
    let retained = insert_video(&connection, "retained");
    connection
        .execute(
            "INSERT INTO user_likes
             (account_id, video_id, first_seen_at, last_seen_at)
             VALUES (?1, ?2, 'x', 'x')",
            params![other, retained],
        )
        .expect("like");
    connection
        .execute(
            "INSERT INTO favorite_items
             (account_id, video_id, first_seen_at, last_seen_at)
             VALUES (?1, ?2, 'x', 'x')",
            params![source, orphan],
        )
        .expect("favorite");
    let app_data = temp_dir("orphan-cleanup");

    delete_account_data(
        &mut connection,
        &app_data,
        &app_data,
        &app_data.join("profiles"),
        source,
        AccountFolderDisposition::Delete,
    )
    .expect("delete");

    assert_eq!(scalar_i64(&connection, "SELECT COUNT(*) FROM videos"), 1);
    assert_eq!(
        connection
            .query_row("SELECT id FROM videos", [], |row| row.get::<_, i64>(0))
            .expect("retained video"),
        retained
    );
    fs::remove_dir_all(app_data).expect("cleanup");
}

#[test]
fn transfer_disposition_deserializes_target_account_id_from_camel_case() {
    let disposition: AccountFolderDisposition =
        serde_json::from_str(r#"{"type":"transferToAccount","targetAccountId":42}"#)
            .expect("transfer disposition");

    assert_eq!(
        disposition,
        AccountFolderDisposition::TransferToAccount {
            target_account_id: 42,
        }
    );
}

#[test]
fn controlled_profile_path_must_be_a_direct_child() {
    let mut connection = database();
    let root = temp_dir("direct-child");
    let profiles = root.join("profiles");
    fs::create_dir_all(&profiles).expect("profiles");
    let account_id = insert_account(&connection, None, None, "nested/profile", "pending_login");

    assert_eq!(
        cancel_pending_account(&mut connection, account_id, &profiles).unwrap_err(),
        "profile_path_not_controlled"
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn controlled_profile_path_rejects_symlink_or_reparse_directory() {
    let mut connection = database();
    let root = temp_dir("profile-link");
    let profiles = root.join("profiles");
    let outside = root.join("outside-profile");
    fs::create_dir_all(&profiles).expect("profiles");
    fs::create_dir_all(&outside).expect("outside profile");
    fs::write(outside.join("sentinel.txt"), b"keep").expect("sentinel");
    let profile = profiles.join("linked-profile");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &profile).expect("profile symlink");
    #[cfg(windows)]
    {
        let output = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&profile)
            .arg(&outside)
            .output()
            .expect("create profile junction");
        assert!(
            output.status.success(),
            "mklink failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let account_id = insert_account(&connection, None, None, "linked-profile", "pending_login");
    assert_eq!(
        cancel_pending_account(&mut connection, account_id, &profiles).unwrap_err(),
        "profile_path_not_controlled"
    );
    assert_eq!(scalar_i64(&connection, "SELECT COUNT(*) FROM accounts"), 1);
    assert!(outside.join("sentinel.txt").exists());

    #[cfg(unix)]
    fs::remove_file(&profile).expect("remove profile symlink");
    #[cfg(windows)]
    fs::remove_dir(&profile).expect("remove profile junction");
    fs::remove_dir_all(root).expect("cleanup");
}

#[allow(dead_code)]
fn assert_path_exists(path: &Path) {
    assert!(path.exists(), "{} should exist", path.display());
}
