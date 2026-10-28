use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use douyin_history_archive::download::{
    recover_download_jobs, FileObservation, FilesystemRootAccess, GuardedPath, RecoverySummary,
    RootAccess,
};
use douyin_history_archive::migrate_schema_to_latest_at_startup;
use rusqlite::{params, Connection};

use std::{
    io::Write,
    sync::{Arc, Mutex},
};

fn test_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("emily-download-recovery-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn connection() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    connection.execute("INSERT INTO accounts (uid, profile_key, status, created_at, updated_at) VALUES ('download-recovery', '0123456789abcdef0123456789abcdef', 'active', 'now', 'now')", []).unwrap();
    let account_id = connection.last_insert_rowid();
    connection.execute("INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)", [account_id]).unwrap();
    connection
}

fn insert_job(connection: &Connection, root: &Path, state: &str) -> i64 {
    connection
        .execute(
            "INSERT INTO download_jobs
         (account_id, profile_key, created_at, selection_scope, destination_root, requested_quality, state, candidate_count)
         VALUES (1, '0123456789abcdef0123456789abcdef', 'now', 'all', ?1, 'highest', ?2, 1)",
            params![root.to_string_lossy(), state],
        )
        .unwrap();
    connection.last_insert_rowid()
}

fn insert_item(connection: &Connection, job_id: i64, aweme_id: &str, state: &str) -> i64 {
    connection.execute(
        "INSERT INTO download_items
         (job_id, aweme_id, canonical_url, content_type, relative_work_directory, state, created_at, updated_at)
         VALUES (?1, ?2, 'https://example.invalid/video', 'video', ?3, ?4, 'now', 'now')",
        params![job_id, aweme_id, format!("author/{aweme_id}"), state],
    ).unwrap();
    connection.last_insert_rowid()
}

fn insert_asset(
    connection: &Connection,
    item_id: i64,
    state: &str,
    final_path: &str,
    part_path: &str,
    expected_size: Option<i64>,
) -> i64 {
    connection
        .execute(
            "INSERT INTO download_assets
         (item_id, stable_key, asset_type, ordinal, requested_profile,
          relative_final_path, relative_part_path, expected_size, manifest_generation,
          state, created_at, updated_at)
         VALUES (?1, ?2, 'video', 0, 'highest', ?3, ?4, ?5, 0, ?6, 'now', 'now')",
            params![
                item_id,
                format!("video:{item_id}:{final_path}"),
                final_path,
                part_path,
                expected_size,
                state
            ],
        )
        .unwrap();
    connection.last_insert_rowid()
}

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn startup_recovery_applies_the_full_offline_transition_matrix() {
    let root = test_root();
    let mut connection = connection();
    let running_job = insert_job(&connection, &root, "running");
    let resolving_item = insert_item(&connection, running_job, "1001", "resolving");
    let retained = insert_asset(
        &connection,
        resolving_item,
        "downloading",
        "author/1001/video.mp4",
        "author/1001/.asset-1-10.mp4.part",
        Some(8),
    );
    connection
        .execute(
            "UPDATE download_assets SET relative_part_path = ?1 WHERE id = ?2",
            params![
                format!("author/1001/.asset-{retained}-10.mp4.part"),
                retained
            ],
        )
        .unwrap();
    write(
        &root,
        &format!("author/1001/.asset-{retained}-10.mp4.part"),
        b"abc",
    );

    let queued_job = insert_job(&connection, &root, "queued");
    let downloading_item = insert_item(&connection, queued_job, "1002", "downloading");
    let completed = insert_asset(
        &connection,
        downloading_item,
        "queued",
        "author/1002/video.mp4",
        "author/1002/.asset-2-1.mp4.part",
        Some(4),
    );
    write(&root, "author/1002/video.mp4", b"good");

    let paused_item = insert_item(&connection, queued_job, "1003", "paused");
    let missing = insert_asset(
        &connection,
        paused_item,
        "paused",
        "author/1003/video.mp4",
        "author/1003/.asset-3-1.mp4.part",
        None,
    );

    let queued_item = insert_item(&connection, queued_job, "1004", "queued");
    let invalid_final = insert_asset(
        &connection,
        queued_item,
        "queued",
        "author/1004/video.mp4",
        "author/1004/.asset-4-1.mp4.part",
        Some(5),
    );
    write(&root, "author/1004/video.mp4", b"no");

    let unowned = insert_asset(
        &connection,
        queued_item,
        "queued",
        "author/1004/other.mp4",
        "author/1004/video.mp4.part",
        None,
    );
    write(&root, "author/1004/video.mp4.part", b"orphan");

    let summary =
        recover_download_jobs(&mut connection, &FilesystemRootAccess).expect("offline recovery");
    assert_eq!(
        summary,
        RecoverySummary {
            jobs_interrupted: 1,
            items_interrupted: 2,
            assets_interrupted: 1,
            finals_completed: 1,
            finals_failed: 1,
            partials_retained: 1,
            partials_deleted: 1,
            assets_reset: 2,
        }
    );

    let states: (String, String, String, i64) = connection
        .query_row(
            "SELECT j.state, i.state, a.state, a.downloaded_bytes
         FROM download_jobs j JOIN download_items i ON i.job_id=j.id
         JOIN download_assets a ON a.item_id=i.id WHERE a.id=?1",
            [retained],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        states,
        (
            "interrupted".into(),
            "interrupted".into(),
            "interrupted".into(),
            3
        )
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT state FROM download_assets WHERE id=?1",
                [completed],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "completed"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT state FROM download_assets WHERE id=?1",
                [missing],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "queued"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT state FROM download_assets WHERE id=?1",
                [invalid_final],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "failed"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT last_error_code FROM download_assets WHERE id=?1",
                [invalid_final],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "download_asset_validation_failed"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT state FROM download_assets WHERE id=?1",
                [unowned],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "queued"
    );
    assert!(!root.join("author/1004/video.mp4.part").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recovery_leaves_terminal_and_explicitly_paused_rows_unchanged_when_no_reconciliation_is_needed()
{
    let root = test_root();
    let mut connection = connection();
    for state in ["completed", "completed_with_errors", "cancelled", "paused"] {
        let job = insert_job(&connection, &root, state);
        let item_state = if state == "paused" {
            "paused"
        } else {
            "completed"
        };
        insert_item(&connection, job, &format!("9{job}"), item_state);
    }
    assert_eq!(
        recover_download_jobs(&mut connection, &FilesystemRootAccess).unwrap(),
        RecoverySummary::default()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recovery_leaves_failed_assets_unchanged_until_explicit_retry() {
    let root = test_root();
    let mut connection = connection();
    let job = insert_job(&connection, &root, "interrupted");
    let item = insert_item(&connection, job, "7001", "failed");
    let asset = insert_asset(
        &connection,
        item,
        "failed",
        "author/7001/video.mp4",
        "author/7001/.asset-1-1.mp4.part",
        Some(4),
    );
    write(&root, "author/7001/video.mp4", b"good");

    assert_eq!(
        recover_download_jobs(&mut connection, &FilesystemRootAccess).unwrap(),
        RecoverySummary::default()
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT state FROM download_assets WHERE id=?1",
                [asset],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "failed"
    );
    fs::remove_dir_all(root).unwrap();
}

struct RecordingRootAccess {
    events: Arc<Mutex<Vec<String>>>,
}

struct RecordingGuard {
    path: String,
    events: Arc<Mutex<Vec<String>>>,
}

impl GuardedPath for RecordingGuard {
    fn inspect(&self) -> Result<FileObservation, douyin_history_archive::download::DownloadError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("inspect:{}", self.path));
        if self.path.ends_with("video.mp4") {
            Ok(FileObservation::Missing)
        } else {
            Ok(FileObservation::File { len: 6 })
        }
    }

    fn remove(&self) -> Result<(), douyin_history_archive::download::DownloadError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("remove:{}", self.path));
        Ok(())
    }

    fn create_new(
        &self,
    ) -> Result<Box<dyn Write + Send>, douyin_history_archive::download::DownloadError> {
        unreachable!("recovery never creates files")
    }
}

impl Drop for RecordingGuard {
    fn drop(&mut self) {
        self.events
            .lock()
            .unwrap()
            .push(format!("drop:{}", self.path));
    }
}

impl RootAccess for RecordingRootAccess {
    fn guard<'a>(
        &'a self,
        _root: &Path,
        relative: &Path,
    ) -> Result<Box<dyn GuardedPath + 'a>, douyin_history_archive::download::DownloadError> {
        let path = relative.to_string_lossy().replace('\\', "/");
        self.events.lock().unwrap().push(format!("guard:{path}"));
        Ok(Box::new(RecordingGuard {
            path,
            events: Arc::clone(&self.events),
        }))
    }

    fn guard_for_create<'a>(
        &'a self,
        root: &Path,
        relative: &Path,
    ) -> Result<Box<dyn GuardedPath + 'a>, douyin_history_archive::download::DownloadError> {
        self.guard(root, relative)
    }

    fn atomic_rename(
        &self,
        _root: &Path,
        _from: &Path,
        _to: &Path,
    ) -> Result<(), douyin_history_archive::download::DownloadError> {
        unreachable!("recovery never renames files")
    }
}

#[test]
fn recovery_keeps_the_same_guard_alive_across_inspection_and_deletion() {
    let root = test_root();
    let mut connection = connection();
    let job = insert_job(&connection, &root, "queued");
    let item = insert_item(&connection, job, "8001", "queued");
    insert_asset(
        &connection,
        item,
        "queued",
        "video.mp4",
        "unowned.part",
        None,
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let roots = RecordingRootAccess {
        events: Arc::clone(&events),
    };

    recover_download_jobs(&mut connection, &roots).unwrap();

    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "guard:author/8001/video.mp4",
            "inspect:author/8001/video.mp4",
            "drop:author/8001/video.mp4",
            "guard:author/8001/unowned.part",
            "inspect:author/8001/unowned.part",
            "remove:author/8001/unowned.part",
            "drop:author/8001/unowned.part",
        ]
    );
    fs::remove_dir_all(root).unwrap();
}
