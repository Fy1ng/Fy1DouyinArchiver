use douyin_history_archive::{
    cover_cache::{
        add_cache_rule_item, cache_relative_path, cleanup_broken_cache_entries,
        clear_cache_rule_items, download_cover, enqueue_all_missing_cover_candidates,
        ensure_pending_cache_task, execute_cache_cleanup, execute_configured_cache_cleanup,
        inspect_cache, list_cache_rule_items, plan_cache_cleanup, preview_configured_cache_cleanup,
        process_cache_task_step_with, read_cached_cover, recover_interrupted_cache_tasks,
        remove_cache_rule_item, request_cache_task_cancel, resume_cache_task, retry_cache_task,
        search_cache_rule_candidates, select_cover_candidates, start_cache_task, store_cover_bytes,
        update_cache_task_progress, CacheCleanupModeV1, CacheCleanupPolicyV1, CacheCleanupRuleV1,
        CacheEntryCandidateV1, CacheTaskKindV1, CacheTaskStatusV1, CacheTaskStepV1,
        CoverCacheWorkerConfigV1, CoverCacheWorkerHandle, DownloadedCoverV1,
    },
    migrate_schema_to_latest_at_startup,
    runtime_paths::RuntimePaths,
    settings_store::{update_global_settings, CoverCacheCleanupModeV1, GlobalSettingsV2},
    AppPaths,
};
use rusqlite::Connection;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration as StdDuration, SystemTime, UNIX_EPOCH},
};

fn temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-cover-cache-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn seed_video(connection: &Connection, id: i64, aweme_id: &str) {
    connection
        .execute(
            "INSERT INTO videos
             (id, aweme_id, canonical_url, content_type, cover_url, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, 'video', ?4, 'first', 'last')",
            rusqlite::params![
                id,
                aweme_id,
                format!("https://www.douyin.com/video/{aweme_id}"),
                format!("https://example.com/{aweme_id}.jpg")
            ],
        )
        .unwrap();
}

#[test]
fn latest_schema_adds_cover_cache_metadata_tasks_and_rules() {
    let mut connection = Connection::open_in_memory().expect("open db");
    migrate_schema_to_latest_at_startup(&mut connection).expect("latest schema");

    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 8);
    for table in [
        "cover_cache",
        "cover_cache_tasks",
        "cover_cache_rules",
        "cover_cache_queue",
    ] {
        let exists: i64 = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get(0),
            )
            .expect("schema object");
        assert_eq!(exists, 1, "missing table {table}");
    }
}

#[test]
fn cache_paths_are_stable_and_cannot_escape_the_covers_directory() {
    assert_eq!(
        cache_relative_path("7654083396108329977", "image/jpeg").unwrap(),
        "76/7654083396108329977.jpg"
    );
    assert_eq!(
        cache_relative_path("note:unsafe/identifier", "image/webp").unwrap(),
        "dc/dc5b2f808b18fb27893934ff81ca072e2e9d4713288edabb317a1eb32379a620.webp"
    );
    assert_eq!(
        cache_relative_path("123", "text/html").unwrap_err(),
        "unsupported_cover_mime"
    );
}

#[test]
fn keep_and_clean_rules_are_mutually_exclusive_and_capacity_uses_lru() {
    let entries = vec![
        CacheEntryCandidateV1::new(1, 100, "2026-08-01T00:00:00.000Z"),
        CacheEntryCandidateV1::new(2, 100, "2026-08-02T00:00:00.000Z"),
        CacheEntryCandidateV1::new(3, 100, "2026-08-03T00:00:00.000Z"),
    ];
    let keep = CacheCleanupRuleV1 {
        mode: CacheCleanupModeV1::KeepListed,
        video_ids: BTreeSet::from([1]),
    };
    let policy = CacheCleanupPolicyV1 {
        max_bytes: Some(150),
        max_age_days: None,
    };
    assert_eq!(
        plan_cache_cleanup(&entries, &keep, &policy, "2026-08-14T00:00:00.000Z")
            .unwrap()
            .video_ids,
        vec![2, 3]
    );

    let clean = CacheCleanupRuleV1 {
        mode: CacheCleanupModeV1::CleanListed,
        video_ids: BTreeSet::from([2]),
    };
    assert_eq!(
        plan_cache_cleanup(
            &entries,
            &clean,
            &CacheCleanupPolicyV1::default(),
            "2026-08-14T00:00:00.000Z"
        )
        .unwrap()
        .video_ids,
        vec![2]
    );

    assert_eq!(
        plan_cache_cleanup(
            &entries,
            &keep,
            &CacheCleanupPolicyV1::default(),
            "2026-08-14T00:00:00.000Z",
        )
        .unwrap()
        .video_ids,
        vec![2, 3],
        "manual keep-list cleanup must remove every cache entry outside the list",
    );

    let restricted_clean = CacheCleanupRuleV1 {
        mode: CacheCleanupModeV1::CleanListed,
        video_ids: BTreeSet::from([2, 3]),
    };
    assert_eq!(
        plan_cache_cleanup(
            &entries,
            &restricted_clean,
            &CacheCleanupPolicyV1 {
                max_bytes: None,
                max_age_days: Some(11),
            },
            "2026-08-14T00:00:00.000Z",
        )
        .unwrap()
        .video_ids,
        vec![2],
        "clean-list age cleanup must never select an entry outside the list",
    );
    assert_eq!(
        plan_cache_cleanup(
            &entries,
            &CacheCleanupRuleV1 {
                mode: CacheCleanupModeV1::CleanListed,
                video_ids: BTreeSet::from([1]),
            },
            &CacheCleanupPolicyV1 {
                max_bytes: Some(150),
                max_age_days: None,
            },
            "2026-08-14T00:00:00.000Z",
        )
        .unwrap()
        .video_ids,
        vec![1],
        "clean-list capacity cleanup may leave the limit unmet instead of deleting outside the list",
    );
}

#[test]
fn storing_cover_bytes_is_atomic_and_inspection_detects_missing_files() {
    let root = temp_root("store");
    let covers = root.join("covers");
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_video(&connection, 1, "7654083396108329977");

    let stored = store_cover_bytes(
        &connection,
        &covers,
        1,
        "7654083396108329977",
        "https://example.com/cover.jpg",
        "image/jpeg",
        b"cover-bytes",
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();
    assert_eq!(stored.relative_path, "76/7654083396108329977.jpg");
    assert_eq!(
        fs::read(covers.join(&stored.relative_path)).unwrap(),
        b"cover-bytes"
    );
    assert_eq!(
        inspect_cache(&connection, &covers).unwrap().healthy_files,
        1
    );

    fs::remove_file(covers.join(&stored.relative_path)).unwrap();
    let inspection = inspect_cache(&connection, &covers).unwrap();
    assert_eq!(inspection.missing_files, 1);
    assert_eq!(inspection.orphan_files, 0);
}

#[test]
fn cleanup_removes_only_indexed_files_beneath_covers_and_keeps_outside_files() {
    let root = temp_root("cleanup");
    let covers = root.join("covers");
    let outside = root.join("outside.jpg");
    fs::write(&outside, b"outside").unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_video(&connection, 1, "111");
    seed_video(&connection, 2, "222");
    store_cover_bytes(
        &connection,
        &covers,
        1,
        "111",
        "https://example.com/111.jpg",
        "image/jpeg",
        b"one",
        "2026-08-01T00:00:00.000Z",
    )
    .unwrap();
    store_cover_bytes(
        &connection,
        &covers,
        2,
        "222",
        "https://example.com/222.jpg",
        "image/jpeg",
        b"two",
        "2026-08-02T00:00:00.000Z",
    )
    .unwrap();
    connection
        .execute(
            "UPDATE cover_cache SET relative_path = '../outside.jpg' WHERE video_id = 2",
            [],
        )
        .unwrap();

    assert_eq!(
        execute_cache_cleanup(&mut connection, &covers, &[1, 2]).unwrap_err(),
        "cover_cache_path_escape"
    );
    assert!(outside.is_file());
    assert!(covers.join("11/111.jpg").is_file());
}

#[test]
fn broken_cache_cleanup_removes_invalid_indexes_and_orphans_without_touching_outside_files() {
    let root = temp_root("broken-cleanup");
    let covers = root.join("covers");
    let outside = root.join("outside.jpg");
    fs::write(&outside, b"outside").unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_video(&connection, 1, "111");
    seed_video(&connection, 2, "222");
    let missing = store_cover_bytes(
        &connection,
        &covers,
        1,
        "111",
        "https://example.com/111.jpg",
        "image/jpeg",
        b"missing",
        "2026-08-01T00:00:00.000Z",
    )
    .unwrap();
    fs::remove_file(covers.join(missing.relative_path)).unwrap();
    store_cover_bytes(
        &connection,
        &covers,
        2,
        "222",
        "https://example.com/222.jpg",
        "image/jpeg",
        b"corrupt",
        "2026-08-02T00:00:00.000Z",
    )
    .unwrap();
    fs::write(covers.join("22/222.jpg"), b"tampered").unwrap();
    fs::create_dir_all(covers.join("orphan")).unwrap();
    fs::write(covers.join("orphan/file.jpg"), b"orphan").unwrap();

    let result = cleanup_broken_cache_entries(&mut connection, &covers).unwrap();

    assert_eq!(result.deleted_indexes, 2);
    assert_eq!(result.deleted_files, 2);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cover_cache", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0,
    );
    assert!(!covers.join("22/222.jpg").exists());
    assert!(!covers.join("orphan/file.jpg").exists());
    assert!(outside.is_file());
}

#[test]
fn cache_rule_search_preview_and_execution_share_one_server_side_plan() {
    let root = temp_root("rules");
    let covers = root.join("covers");
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    for (id, aweme_id) in [(1, "111"), (2, "222"), (3, "333")] {
        seed_video(&connection, id, aweme_id);
        connection
            .execute(
                "UPDATE videos SET title = ?2 WHERE id = ?1",
                rusqlite::params![id, format!("video-{aweme_id}")],
            )
            .unwrap();
        store_cover_bytes(
            &connection,
            &covers,
            id,
            aweme_id,
            &format!("https://example.com/{aweme_id}.jpg"),
            "image/jpeg",
            &vec![id as u8; 10],
            &format!("2026-08-0{id}T00:00:00.000Z"),
        )
        .unwrap();
    }

    add_cache_rule_item(&connection, 1, "2026-08-14T00:00:00.000Z").unwrap();
    assert_eq!(list_cache_rule_items(&connection).unwrap()[0].video_id, 1,);
    assert_eq!(
        search_cache_rule_candidates(&connection, "222", 20)
            .unwrap()
            .into_iter()
            .map(|item| item.video_id)
            .collect::<Vec<_>>(),
        vec![2],
    );

    let preview = preview_configured_cache_cleanup(
        &connection,
        CacheCleanupModeV1::KeepListed,
        &CacheCleanupPolicyV1::default(),
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();
    assert_eq!(preview.files, 2);
    assert_eq!(preview.videos, 2);
    assert_eq!(preview.estimated_bytes, 20);

    let result = execute_configured_cache_cleanup(
        &mut connection,
        &covers,
        CacheCleanupModeV1::KeepListed,
        &CacheCleanupPolicyV1::default(),
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();
    assert_eq!(result.deleted_files, 2);
    assert!(covers.join("11/111.jpg").is_file());
    assert!(!covers.join("22/222.jpg").exists());
    assert!(!covers.join("33/333.jpg").exists());

    assert!(remove_cache_rule_item(&connection, 1).unwrap());
    add_cache_rule_item(&connection, 1, "2026-08-14T00:01:00.000Z").unwrap();
    assert_eq!(clear_cache_rule_items(&connection).unwrap(), 1);
}

#[test]
fn cache_task_lifecycle_supports_pause_resume_cancel_retry_and_recovery() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();

    let task = start_cache_task(
        &mut connection,
        CacheTaskKindV1::Backfill,
        3,
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();
    assert_eq!(task.status, CacheTaskStatusV1::Running);
    update_cache_task_progress(&connection, task.id, 1, 0).unwrap();
    let paused =
        douyin_history_archive::cover_cache::pause_cache_task(&connection, task.id).unwrap();
    assert_eq!(paused.status, CacheTaskStatusV1::Paused);
    let resumed = resume_cache_task(&connection, task.id).unwrap();
    assert_eq!(resumed.status, CacheTaskStatusV1::Running);
    let cancelled =
        request_cache_task_cancel(&connection, task.id, "2026-08-14T00:01:00.000Z").unwrap();
    assert_eq!(cancelled.status, CacheTaskStatusV1::Cancelled);

    let retried = retry_cache_task(&mut connection, task.id, "2026-08-14T00:02:00.000Z").unwrap();
    assert_ne!(retried.id, task.id);
    assert_eq!(retried.kind, CacheTaskKindV1::Backfill);
    assert_eq!(retried.status, CacheTaskStatusV1::Running);
    assert_eq!(retried.total_items, 3);

    recover_interrupted_cache_tasks(&connection, "2026-08-14T00:03:00.000Z").unwrap();
    let recovered = douyin_history_archive::cover_cache::get_cache_task(&connection, retried.id)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.status, CacheTaskStatusV1::Failed);
    assert_eq!(
        recovered.error_code.as_deref(),
        Some("cache_task_interrupted")
    );
}

#[test]
fn only_one_active_cache_task_can_exist() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    start_cache_task(
        &mut connection,
        CacheTaskKindV1::Backfill,
        1,
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();
    assert_eq!(
        start_cache_task(
            &mut connection,
            CacheTaskKindV1::NewRecords,
            1,
            "2026-08-14T00:00:01.000Z",
        )
        .unwrap_err(),
        "cover_cache_task_active"
    );
}

#[test]
fn candidate_selection_supports_all_history_and_only_new_sync_records() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    for (id, aweme_id) in [(1, "111"), (2, "222"), (3, "333")] {
        seed_video(&connection, id, aweme_id);
    }
    connection
        .execute(
            "INSERT INTO accounts
             (id, uid, profile_key, status, created_at, updated_at)
             VALUES (7, 'uid-7', '00000000000000000000000000000007', 'active', 'created', 'updated')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO sync_jobs
             (id, account_id, source, source_instance_key, trigger, queued_at,
              settings_revision, payload_json, payload_sha256, equivalence_sha256,
              finished_at, status)
             VALUES (9, 7, 'history', '', 'manual', 'queued', 0, '{}', ?1, ?1,
                     'finished', 'success')",
            ["a".repeat(64)],
        )
        .unwrap();
    for (id, video_id, sync_job_id) in [(1, 1, Some(9)), (2, 2, Some(9)), (3, 3, None)] {
        connection
            .execute(
                "INSERT INTO watch_records
                 (id, account_id, video_id, history_time_ms, watched_at, watched_date,
                  source_position, sync_job_id, created_at, updated_at)
                 VALUES (?1, 7, ?2, ?1, 'watched', '2026-08-14', ?1, ?3, 'created', 'updated')",
                rusqlite::params![id, video_id, sync_job_id],
            )
            .unwrap();
    }

    assert_eq!(
        select_cover_candidates(&connection, None)
            .unwrap()
            .into_iter()
            .map(|item| item.video_id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        select_cover_candidates(&connection, Some(9))
            .unwrap()
            .into_iter()
            .map(|item| item.video_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn backfill_enqueues_every_missing_cover_and_starts_one_pending_task() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    for (id, aweme_id) in [(1, "111"), (2, "222")] {
        seed_video(&connection, id, aweme_id);
    }
    connection
        .execute(
            "INSERT INTO accounts
             (id, uid, profile_key, status, created_at, updated_at)
             VALUES (7, 'uid-7', '00000000000000000000000000000007', 'active', 'created', 'updated')",
            [],
        )
        .unwrap();
    for (id, video_id) in [(1, 1), (2, 2)] {
        connection
            .execute(
                "INSERT INTO watch_records
                 (id, account_id, video_id, history_time_ms, watched_at, watched_date,
                  source_position, created_at, updated_at)
                 VALUES (?1, 7, ?2, ?1, 'watched', '2026-08-14', ?1, 'created', 'updated')",
                rusqlite::params![id, video_id],
            )
            .unwrap();
    }

    assert_eq!(
        enqueue_all_missing_cover_candidates(&mut connection, "2026-08-14T00:00:00.000Z").unwrap(),
        2,
    );
    let task = ensure_pending_cache_task(
        &mut connection,
        CacheTaskKindV1::Backfill,
        "2026-08-14T00:00:01.000Z",
    )
    .unwrap()
    .expect("pending task");
    assert_eq!(task.total_items, 2);
    assert_eq!(task.status, CacheTaskStatusV1::Running);
    assert_eq!(
        ensure_pending_cache_task(
            &mut connection,
            CacheTaskKindV1::Backfill,
            "2026-08-14T00:00:02.000Z",
        )
        .unwrap()
        .unwrap()
        .id,
        task.id,
        "repeated wakeups must reuse the active task",
    );
}

#[test]
fn disabling_cache_drops_pending_candidates_and_cancels_the_active_task() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_video(&connection, 1, "111");
    connection
        .execute(
            "INSERT INTO cover_cache_queue(video_id, queued_at) VALUES (1, ?1)",
            ["2026-08-14T00:00:00.000Z"],
        )
        .unwrap();
    let task = start_cache_task(
        &mut connection,
        CacheTaskKindV1::Backfill,
        1,
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();

    douyin_history_archive::cover_cache::disable_cover_cache_queue(
        &mut connection,
        "2026-08-14T00:01:00.000Z",
    )
    .unwrap();

    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cover_cache_queue", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0,
    );
    assert_eq!(
        douyin_history_archive::cover_cache::get_cache_task(&connection, task.id)
            .unwrap()
            .unwrap()
            .status,
        CacheTaskStatusV1::Cancelled,
    );
}

#[test]
fn native_cover_downloader_accepts_images_and_enforces_the_size_limit() {
    fn serve(body: &'static [u8], content_type: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });
        format!("http://{address}/cover")
    }

    let downloaded = download_cover(&serve(b"image", "image/jpeg"), 16).unwrap();
    assert_eq!(downloaded.mime_type, "image/jpeg");
    assert_eq!(downloaded.bytes, b"image");
    assert_eq!(
        download_cover(&serve(b"too-large", "image/jpeg"), 4).unwrap_err(),
        "cover_download_too_large"
    );
    assert_eq!(
        download_cover(&serve(b"html", "text/html"), 16).unwrap_err(),
        "unsupported_cover_mime"
    );
}

#[test]
fn cache_task_step_downloads_one_candidate_and_finishes_without_blocking_the_sync_queue() {
    let root = temp_root("worker-step");
    let covers = root.join("covers");
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_video(&connection, 1, "7654083396108329977");
    connection
        .execute(
            "INSERT INTO cover_cache_queue(video_id, queued_at) VALUES (1, ?1)",
            ["2026-08-14T00:00:00.000Z"],
        )
        .unwrap();
    let task = start_cache_task(
        &mut connection,
        CacheTaskKindV1::NewRecords,
        1,
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();

    let step = process_cache_task_step_with(
        &mut connection,
        &covers,
        task.id,
        1024,
        "2026-08-14T00:00:01.000Z",
        |_| {
            Ok(DownloadedCoverV1 {
                mime_type: "image/jpeg".to_string(),
                bytes: b"fixture-cover".to_vec(),
            })
        },
    )
    .unwrap();

    assert_eq!(step, CacheTaskStepV1::Finished);
    let task = douyin_history_archive::cover_cache::get_cache_task(&connection, task.id)
        .unwrap()
        .unwrap();
    assert_eq!(task.status, CacheTaskStatusV1::Success);
    assert_eq!(task.completed_items, 1);
    assert_eq!(task.failed_items, 0);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cover_cache_queue", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0,
    );
    assert_eq!(
        fs::read(covers.join("76/7654083396108329977.jpg")).unwrap(),
        b"fixture-cover"
    );
}

#[test]
fn cache_task_step_discards_a_download_when_the_task_is_cancelled_mid_flight() {
    let root = temp_root("cancel-mid-download");
    let covers = root.join("covers");
    let database = root.join("history.sqlite");
    let mut connection = Connection::open(&database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_video(&connection, 1, "7654083396108329977");
    connection
        .execute(
            "INSERT INTO cover_cache_queue(video_id, queued_at) VALUES (1, ?1)",
            ["2026-08-14T00:00:00.000Z"],
        )
        .unwrap();
    let task = start_cache_task(
        &mut connection,
        CacheTaskKindV1::Backfill,
        1,
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();

    let cancel_database = database.clone();
    let step = process_cache_task_step_with(
        &mut connection,
        &covers,
        task.id,
        1024,
        "2026-08-14T00:00:01.000Z",
        |_| {
            let cancel_connection = Connection::open(&cancel_database).unwrap();
            request_cache_task_cancel(&cancel_connection, task.id, "2026-08-14T00:00:00.500Z")
                .unwrap();
            Ok(DownloadedCoverV1 {
                mime_type: "image/jpeg".to_string(),
                bytes: b"must-not-be-stored".to_vec(),
            })
        },
    );

    assert_eq!(step.unwrap(), CacheTaskStepV1::Finished);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cover_cache", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0,
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cover_cache_queue", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1,
    );
    assert!(!covers.join("76/7654083396108329977.jpg").exists());
}

#[test]
fn failed_cache_candidates_remain_retryable_after_three_attempts() {
    let root = temp_root("worker-retry");
    let covers = root.join("covers");
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_video(&connection, 1, "7654083396108329977");
    connection
        .execute(
            "INSERT INTO cover_cache_queue(video_id, queued_at) VALUES (1, ?1)",
            ["2026-08-14T00:00:00.000Z"],
        )
        .unwrap();
    let task = start_cache_task(
        &mut connection,
        CacheTaskKindV1::Backfill,
        1,
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();

    for second in 1..=3 {
        process_cache_task_step_with(
            &mut connection,
            &covers,
            task.id,
            1024,
            &format!("2026-08-14T00:00:0{second}.000Z"),
            |_| Err("network_down".to_string()),
        )
        .unwrap();
    }

    let failed = douyin_history_archive::cover_cache::get_cache_task(&connection, task.id)
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, CacheTaskStatusV1::Failed);
    assert_eq!(failed.failed_items, 1);
    assert_eq!(
        connection
            .query_row(
                "SELECT attempts FROM cover_cache_queue WHERE video_id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        3,
        "the failed candidate must remain in the queue for an explicit retry",
    );

    let retried = retry_cache_task(&mut connection, task.id, "2026-08-14T00:01:00.000Z").unwrap();
    assert_eq!(retried.total_items, 1);
    assert_eq!(
        connection
            .query_row(
                "SELECT attempts FROM cover_cache_queue WHERE video_id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0,
    );

    let step = process_cache_task_step_with(
        &mut connection,
        &covers,
        retried.id,
        1024,
        "2026-08-14T00:01:01.000Z",
        |_| {
            Ok(DownloadedCoverV1 {
                mime_type: "image/jpeg".to_string(),
                bytes: b"retried-cover".to_vec(),
            })
        },
    )
    .unwrap();
    assert_eq!(step, CacheTaskStepV1::Finished);
    assert_eq!(
        douyin_history_archive::cover_cache::get_cache_task(&connection, retried.id)
            .unwrap()
            .unwrap()
            .status,
        CacheTaskStatusV1::Success,
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cover_cache_queue", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0,
    );
}

#[test]
fn cache_worker_downloads_in_background_and_stops_before_data_root_maintenance() {
    let root = temp_root("background-worker");
    let paths = AppPaths::new(root.clone(), PathBuf::new());
    let mut connection = Connection::open(&paths.database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_video(&connection, 1, "7654083396108329977");
    connection
        .execute(
            "INSERT INTO cover_cache_queue(video_id, queued_at) VALUES (1, ?1)",
            ["2026-08-14T00:00:00.000Z"],
        )
        .unwrap();
    start_cache_task(
        &mut connection,
        CacheTaskKindV1::Backfill,
        1,
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();
    drop(connection);

    let runtime_paths = RuntimePaths::new(paths.clone());
    let worker = CoverCacheWorkerHandle::start(
        CoverCacheWorkerConfigV1::new(runtime_paths.clone()).with_downloader(Arc::new(|_| {
            Ok(DownloadedCoverV1 {
                mime_type: "image/jpeg".to_string(),
                bytes: b"worker-cover".to_vec(),
            })
        })),
    )
    .unwrap();
    worker.wake();

    let deadline = std::time::Instant::now() + StdDuration::from_secs(2);
    loop {
        let connection = Connection::open(&paths.database).unwrap();
        let status = douyin_history_archive::cover_cache::latest_cache_task(&connection)
            .unwrap()
            .unwrap()
            .status;
        if status == CacheTaskStatusV1::Success {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "cache worker timed out"
        );
        thread::sleep(StdDuration::from_millis(10));
    }
    assert_eq!(
        fs::read(paths.covers.join("76/7654083396108329977.jpg")).unwrap(),
        b"worker-cover",
    );

    worker.stop();
    worker.join().unwrap();
    let maintenance = runtime_paths.begin_maintenance().unwrap();
    drop(maintenance);
}

#[test]
fn cache_worker_applies_capacity_cleanup_only_when_explicitly_woken() {
    let root = temp_root("automatic-cleanup");
    let paths = AppPaths::new(root.clone(), PathBuf::new());
    let mut connection = Connection::open(&paths.database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    for (id, aweme_id) in [(1, "111"), (2, "222")] {
        seed_video(&connection, id, aweme_id);
        store_cover_bytes(
            &connection,
            &paths.covers,
            id,
            aweme_id,
            &format!("https://example.com/{aweme_id}.jpg"),
            "image/jpeg",
            &vec![id as u8; 700 * 1024],
            &format!("2026-08-0{id}T00:00:00.000Z"),
        )
        .unwrap();
    }
    drop(connection);
    let mut settings = GlobalSettingsV2::default();
    settings.cover_cache.enabled = true;
    settings.cover_cache.cleanup_by_capacity_enabled = true;
    settings.cover_cache.capacity_limit_mib = 1;
    settings.cover_cache.cleanup_list_mode = CoverCacheCleanupModeV1::KeepListed;
    update_global_settings(&paths.data_root, settings).unwrap();

    let runtime_paths = RuntimePaths::new(paths.clone());
    let worker = CoverCacheWorkerHandle::start(
        CoverCacheWorkerConfigV1::new(runtime_paths).with_idle_wait(StdDuration::from_millis(10)),
    )
    .unwrap();
    thread::sleep(StdDuration::from_millis(50));
    assert_eq!(
        Connection::open(&paths.database)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM cover_cache", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2,
        "idle worker must not scan or clean without a wake event",
    );

    worker.wake();
    let deadline = std::time::Instant::now() + StdDuration::from_secs(2);
    loop {
        let count = Connection::open(&paths.database)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM cover_cache", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        if count == 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "automatic cleanup timed out"
        );
        thread::sleep(StdDuration::from_millis(10));
    }
    assert!(!paths.covers.join("11/111.jpg").exists());
    assert!(paths.covers.join("22/222.jpg").is_file());
    worker.stop();
    worker.join().unwrap();
}

#[test]
fn cached_cover_reads_are_verified_and_refresh_last_access_time() {
    let root = temp_root("read");
    let covers = root.join("covers");
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_video(&connection, 1, "123");
    let stored = store_cover_bytes(
        &connection,
        &covers,
        1,
        "123",
        "https://example.com/123.jpg",
        "image/jpeg",
        b"cover",
        "2026-08-14T00:00:00.000Z",
    )
    .unwrap();

    let asset = read_cached_cover(&connection, &covers, 1, "2026-08-14T00:01:00.000Z").unwrap();
    assert_eq!(asset.mime_type, "image/jpeg");
    assert_eq!(asset.bytes, b"cover");
    assert_eq!(
        connection
            .query_row(
                "SELECT last_accessed_at FROM cover_cache WHERE video_id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "2026-08-14T00:01:00.000Z",
    );

    fs::write(covers.join(stored.relative_path), b"tampered").unwrap();
    assert_eq!(
        read_cached_cover(&connection, &covers, 1, "2026-08-14T00:02:00.000Z",).unwrap_err(),
        "cover_cache_corrupt",
    );
}
