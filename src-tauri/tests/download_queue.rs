use std::{path::PathBuf, sync::Arc};

use douyin_history_archive::{
    download::{
        DownloadEvent, DownloadItemState, DownloadMutation, DownloadQueue, PersistenceHandle,
    },
    migrate_schema_to_latest_at_startup,
};
use rusqlite::{params, Connection};

const TEST_PROFILE_KEY: &str = "0123456789abcdef0123456789abcdef";

fn database_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "douyin-download-queue-{name}-{}-{}.sqlite",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ))
}

fn seed_job(path: &PathBuf, aweme_id: &str) -> i64 {
    let mut connection = Connection::open(path).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    connection.execute("INSERT OR IGNORE INTO accounts (uid, profile_key, status, created_at, updated_at) VALUES ('download-queue', ?1, 'active', 'now', 'now')", [TEST_PROFILE_KEY]).unwrap();
    let account_id: i64 = connection.query_row("SELECT id FROM accounts WHERE profile_key = ?1", [TEST_PROFILE_KEY], |row| row.get(0)).unwrap();
    connection.execute("INSERT OR IGNORE INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)", [account_id]).unwrap();
    connection.execute("INSERT INTO download_jobs (account_id, profile_key, created_at, selection_scope, destination_root, requested_quality, state, candidate_count) VALUES (?1, ?2, 'now', 'selected', 'C:/downloads', 'highest', 'queued', 1)", params![account_id, TEST_PROFILE_KEY]).unwrap();
    let job_id = connection.last_insert_rowid();
    connection.execute("INSERT INTO download_items (job_id, aweme_id, canonical_url, content_type, relative_work_directory, state, created_at, updated_at) VALUES (?1, ?2, ?3, 'video', ?2, 'queued', 'now', 'now')", params![job_id, aweme_id, format!("https://example.com/{aweme_id}")]).unwrap();
    let item_id = connection.last_insert_rowid();
    connection.execute("INSERT INTO download_assets (item_id, stable_key, asset_type, ordinal, requested_profile, relative_final_path, relative_part_path, manifest_generation, state, created_at, updated_at) VALUES (?1, 'video:primary', 'video', 0, 'highest', 'video.mp4', 'video.part', 0, 'queued', 'now', 'now')", [item_id]).unwrap();
    job_id
}

#[tokio::test]
async fn concurrent_start_next_serializes_to_one_running_job() {
    let path = database_path("single-active");
    let first = seed_job(&path, "101");
    let second = seed_job(&path, "102");
    let persistence = PersistenceHandle::spawn(&path, 8).await.unwrap();
    let queue = Arc::new(DownloadQueue::new(persistence));

    let (a, b) = tokio::join!(queue.start_next(), queue.start_next());
    let events = [a.unwrap(), b.unwrap()];
    assert_eq!(
        events.iter().filter(|event| event.job().is_some()).count(),
        1
    );

    let connection = Connection::open(&path).unwrap();
    let running: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM download_jobs WHERE state = 'running'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let queued: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM download_jobs WHERE state = 'queued'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!((running, queued), (1, 1));
    assert_eq!(
        events.iter().find_map(DownloadEvent::job).unwrap().id,
        first
    );
    assert_ne!(first, second);
    drop(connection);
    std::fs::remove_file(path).ok();
}

#[tokio::test]
async fn item_mutations_derive_job_aggregates_in_the_same_transaction() {
    let path = database_path("aggregates");
    let job_id = seed_job(&path, "201");
    let persistence = PersistenceHandle::spawn(&path, 8).await.unwrap();
    DownloadQueue::new(persistence.clone())
        .start_next()
        .await
        .unwrap();
    persistence
        .transition(DownloadMutation::SetItemState {
            job_id,
            aweme_id: "201".into(),
            expected_state: DownloadItemState::Queued,
            next_state: DownloadItemState::Completed,
            downloaded_bytes: 42,
        })
        .await
        .unwrap();
    let event = persistence
        .transition(DownloadMutation::SetItemState {
            job_id,
            aweme_id: "201".into(),
            expected_state: DownloadItemState::Completed,
            next_state: DownloadItemState::Failed,
            downloaded_bytes: 7,
        })
        .await
        .unwrap();
    let job = event.job().unwrap();
    assert_eq!(
        (job.completed_count, job.failed_count, job.downloaded_bytes),
        (0, 1, 7)
    );
    std::fs::remove_file(path).ok();
}

#[tokio::test]
async fn queue_controls_preserve_valid_states_and_retry_only_failed_items() {
    let path = database_path("controls");
    let job_id = seed_job(&path, "301");
    let persistence = PersistenceHandle::spawn(&path, 8).await.unwrap();
    let queue = DownloadQueue::new(persistence.clone());
    queue.start_next().await.unwrap();
    queue.pause(job_id).await.unwrap();
    assert_eq!(
        queue.resume(job_id).await.unwrap().job().unwrap().state,
        "running"
    );
    persistence
        .transition(DownloadMutation::SetItemState {
            job_id,
            aweme_id: "301".into(),
            expected_state: DownloadItemState::Queued,
            next_state: DownloadItemState::Failed,
            downloaded_bytes: 3,
        })
        .await
        .unwrap();
    assert_eq!(
        queue
            .retry_failed(job_id)
            .await
            .unwrap()
            .job()
            .unwrap()
            .failed_count,
        0
    );
    assert_eq!(
        queue.cancel(job_id).await.unwrap().job().unwrap().state,
        "cancelled"
    );
    assert!(queue.resume(job_id).await.is_err());
    std::fs::remove_file(path).ok();
}

#[tokio::test]
async fn failed_commit_returns_no_event_and_rolls_back() {
    let path = database_path("rollback");
    let job_id = seed_job(&path, "401");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_running BEFORE UPDATE OF state ON download_jobs WHEN NEW.state = 'running' BEGIN SELECT RAISE(ABORT, 'reject'); END;").unwrap();
    drop(connection);
    let persistence = PersistenceHandle::spawn(&path, 8).await.unwrap();
    assert!(persistence
        .transition(DownloadMutation::StartNext)
        .await
        .is_err());
    let connection = Connection::open(&path).unwrap();
    let state: String = connection
        .query_row(
            "SELECT state FROM download_jobs WHERE id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "queued");
    std::fs::remove_file(path).ok();
}

#[tokio::test]
async fn resume_cannot_create_a_second_running_job() {
    let path = database_path("resume-single-active");
    let first = seed_job(&path, "501");
    seed_job(&path, "502");
    let queue = DownloadQueue::new(PersistenceHandle::spawn(&path, 8).await.unwrap());
    queue.start_next().await.unwrap();
    queue.pause(first).await.unwrap();
    queue.start_next().await.unwrap();
    assert!(queue.resume(first).await.is_err());
    let connection = Connection::open(&path).unwrap();
    let running: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM download_jobs WHERE state='running'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(running, 1);
    std::fs::remove_file(path).ok();
}

#[tokio::test]
async fn pause_resume_and_cancel_transition_assets_consistently() {
    let path = database_path("asset-controls");
    let job_id = seed_job(&path, "601");
    let persistence = PersistenceHandle::spawn(&path, 8).await.unwrap();
    let queue = DownloadQueue::new(persistence.clone());
    queue.start_next().await.unwrap();
    persistence
        .transition(DownloadMutation::SetItemState {
            job_id,
            aweme_id: "601".into(),
            expected_state: DownloadItemState::Queued,
            next_state: DownloadItemState::Downloading,
            downloaded_bytes: 5,
        })
        .await
        .unwrap();
    let connection = Connection::open(&path).unwrap();
    connection.execute("UPDATE download_assets SET state='downloading' WHERE item_id IN (SELECT id FROM download_items WHERE job_id=?1)", [job_id]).unwrap();
    drop(connection);

    queue.pause(job_id).await.unwrap();
    assert_states(&path, job_id, "paused", "paused", "paused");
    queue.resume(job_id).await.unwrap();
    assert_states(&path, job_id, "running", "queued", "queued");

    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE download_items SET state='failed' WHERE job_id=?1",
            [job_id],
        )
        .unwrap();
    connection.execute("UPDATE download_assets SET state='failed' WHERE item_id IN (SELECT id FROM download_items WHERE job_id=?1)", [job_id]).unwrap();
    drop(connection);
    queue.cancel(job_id).await.unwrap();
    assert_states(&path, job_id, "cancelled", "failed", "failed");
    std::fs::remove_file(path).ok();
}

#[tokio::test]
async fn late_item_mutation_after_pause_is_rejected_without_event() {
    let path = database_path("stale-mutation");
    let job_id = seed_job(&path, "701");
    let persistence = PersistenceHandle::spawn(&path, 8).await.unwrap();
    let queue = DownloadQueue::new(persistence.clone());
    queue.start_next().await.unwrap();
    queue.pause(job_id).await.unwrap();
    let error = persistence
        .transition(DownloadMutation::SetItemState {
            job_id,
            aweme_id: "701".into(),
            expected_state: DownloadItemState::Downloading,
            next_state: DownloadItemState::Completed,
            downloaded_bytes: 99,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "stale_download_mutation");
    assert_states(&path, job_id, "paused", "paused", "paused");
    std::fs::remove_file(path).ok();
}

#[tokio::test(flavor = "current_thread")]
async fn sqlite_busy_wait_does_not_block_the_tokio_runtime_thread() {
    let path = database_path("dedicated-thread");
    seed_job(&path, "801");
    let persistence = PersistenceHandle::spawn(&path, 8).await.unwrap();
    let blocker = Connection::open(&path).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let transition =
        tokio::spawn(async move { persistence.transition(DownloadMutation::StartNext).await });
    let started = std::time::Instant::now();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    blocker.execute_batch("ROLLBACK").unwrap();
    transition.await.unwrap().unwrap();
    std::fs::remove_file(path).ok();
}

fn assert_states(path: &PathBuf, job_id: i64, job: &str, item: &str, asset: &str) {
    let connection = Connection::open(path).unwrap();
    let states: (String, String, String) = connection.query_row(
        "SELECT j.state, i.state, a.state FROM download_jobs j JOIN download_items i ON i.job_id=j.id JOIN download_assets a ON a.item_id=i.id WHERE j.id=?1",
        [job_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert_eq!(states, (job.into(), item.into(), asset.into()));
}
