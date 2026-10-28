use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use douyin_history_archive::{
    download::{
        asset_ownership_key, create_download_job, reconcile_manifest, BrowserOperationCoordinator,
        CompletedAsset, CreateDownloadJobRequest, DownloadAssetTransfer, DownloadFaultInjector,
        DownloadFaultPoint, DownloadJobOwner, DownloadJobResolver, DownloadMutation,
        DownloadQuality, DownloadRuntimeConfig, DownloadSelection, DownloadService,
        FileObservation, FilesystemRootAccess, ResolvedAsset, ResolvedMediaManifest, ResolverRequest,
        RootAccess, TransferRequest,
    },
    migrate_schema_to_latest_at_startup,
};
use rusqlite::{params, Connection};
use tokio_util::sync::CancellationToken;

static NEXT_FIXTURE_ROOT: AtomicU64 = AtomicU64::new(0);
const TEST_PROFILE_KEY: &str = "0123456789abcdef0123456789abcdef";

fn test_owner() -> DownloadJobOwner {
    DownloadJobOwner {
        account_id: 1,
        profile_key: TEST_PROFILE_KEY.to_string(),
    }
}

struct FixtureResolver;

fn fixture_manifest(aweme_id: impl Into<String>) -> ResolvedMediaManifest {
    ResolvedMediaManifest {
        aweme_id: aweme_id.into(),
        content_type: "note".into(),
        assets: (1..=4)
            .map(|ordinal| ResolvedAsset {
                stable_key: format!("image:{ordinal:04}"),
                asset_type: "image".into(),
                ordinal,
                actual_quality: Some("original".into()),
                source_url: format!("https://fixture.invalid/{ordinal}.jpg"),
                request_headers: BTreeMap::new(),
                expected_size: Some(4),
                mime_type: Some("image/jpeg".into()),
                width: Some(1),
                height: Some(1),
                bitrate: None,
                codec: None,
                source_expires_at: None,
            })
            .collect(),
    }
}

impl DownloadJobResolver for FixtureResolver {
    fn resolve<'a>(
        &'a self,
        _service: &'a DownloadService,
        request: ResolverRequest,
        _cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        ResolvedMediaManifest,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move { Ok(fixture_manifest(request.aweme_id)) })
    }
}

#[tokio::test]
async fn preview_selection_reports_backend_deduplicated_count_and_content_types() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    let mut connection = Connection::open(&database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    connection.execute(
        "INSERT INTO accounts (uid, profile_key, status, created_at, updated_at) VALUES ('download-preview','0123456789abcdef0123456789abcdef','active','2026-01-01','2026-01-01')",
        [],
    ).unwrap();
    let account_id = connection.last_insert_rowid();
    connection.execute(
        "INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)",
        [account_id],
    ).unwrap();
    for (aweme_id, content_type, position) in [
        ("1001", "note", 0),
        ("1002", "video", 1),
        ("1003", "unknown", 2),
    ] {
        connection.execute(
            "INSERT INTO videos (aweme_id, canonical_url, content_type, title, author_name, first_seen_at, last_seen_at) VALUES (?1,?2,?3,?4,'author','2026-01-01','2026-01-01')",
            params![aweme_id, format!("https://www.douyin.com/video/{aweme_id}"), content_type, format!("fixture-{aweme_id}")],
        ).unwrap();
        let video_id = connection.last_insert_rowid();
        connection.execute(
            "INSERT INTO watch_records (account_id, video_id, history_time_ms, watched_at, watched_date, source_position, created_at, updated_at) VALUES (?1,?2,100,'2026-01-01','2026-01-01',?3,'2026-01-01','2026-01-01')",
            params![account_id, video_id, position],
        ).unwrap();
        if aweme_id == "1001" {
            connection.execute(
                "INSERT INTO watch_records (account_id, video_id, history_time_ms, watched_at, watched_date, source_position, created_at, updated_at) VALUES (?1,?2,200,'2026-01-02','2026-01-02',3,'2026-01-02','2026-01-02')",
                params![account_id, video_id],
            ).unwrap();
        }
    }
    drop(connection);

    let service = DownloadService::spawn(&database, BrowserOperationCoordinator::default())
        .await
        .unwrap();
    let preview = service
        .preview_selection(account_id, DownloadSelection::All)
        .await
        .unwrap();

    assert_eq!(preview.candidate_count, 3);
    assert_eq!(preview.content_type_counts.get("note"), Some(&1));
    assert_eq!(preview.content_type_counts.get("video"), Some(&1));
    assert_eq!(preview.content_type_counts.get("unknown"), Some(&1));
}

#[tokio::test]
async fn job_detail_exposes_asset_rows_and_backend_aggregate_observability() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let mut connection = Connection::open(&database).unwrap();
    let job_id = create_download_job(
        &mut connection,
        &test_owner(),
        &CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        },
    )
    .unwrap();
    let item_id: i64 = connection
        .query_row(
            "SELECT id FROM download_items WHERE job_id=?1",
            [job_id],
            |row| row.get(0),
        )
        .unwrap();
    let started_at = (chrono::Utc::now() - chrono::Duration::seconds(2))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    connection
        .execute(
            "UPDATE download_jobs SET state='running',started_at=?2 WHERE id=?1",
            params![job_id, started_at],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE download_items SET state='resolving',manifest_generation=1 WHERE id=?1",
            [item_id],
        )
        .unwrap();
    for (stable_key, ordinal, state, downloaded, expected, error) in [
        ("image:0001", 1, "downloading", 1_200, 2_000, None),
        (
            "image:0002",
            2,
            "cancelled",
            50,
            100,
            Some("download_cancelled"),
        ),
        ("music", 3, "skipped", 100, 100, None),
    ] {
        connection.execute(
            "INSERT INTO download_assets (item_id,stable_key,asset_type,ordinal,requested_profile,relative_final_path,relative_part_path,expected_size,downloaded_bytes,manifest_generation,state,last_error_code,created_at,updated_at) VALUES (?1,?2,'image',?3,'balanced',?4,?5,?6,?7,1,?8,?9,'now','now')",
            params![item_id, stable_key, ordinal, format!("{ordinal}.jpg"), format!(".{ordinal}.part"), expected, downloaded, state, error],
        ).unwrap();
    }
    drop(connection);

    let service = DownloadService::spawn(&database, BrowserOperationCoordinator::default())
        .await
        .unwrap();
    service
        .persistence()
        .transition(DownloadMutation::RecomputeJob { job_id })
        .await
        .unwrap();
    let detail = service.get_job(job_id).await.unwrap();

    assert_eq!(detail.job.total_bytes, 2_200);
    assert_eq!(detail.job.downloaded_bytes, 1_350);
    assert_eq!(detail.current_resolver_item_id, Some(item_id));
    assert_eq!(detail.active_asset_count, 1);
    assert_eq!(detail.cancelled_asset_count, 1);
    assert!(detail.average_bytes_per_second > 0);
    assert!(detail.average_bytes_per_second <= 1_250);
    assert_eq!(detail.assets.len(), 3);
    assert_eq!(detail.assets[0].stable_key, "image:0001");
    assert_eq!(
        detail.assets[1].last_error_code.as_deref(),
        Some("download_cancelled")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn load_next_item_failure_interrupts_without_leaving_active_item_state() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let faults = Arc::new(ScriptedFaults::new(DownloadFaultPoint::LoadNextItem, 1));
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FixtureResolver), transfer)
                .with_fault_injector(faults),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();
    let interrupted = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let detail = service.get_job(created.job.id).await.unwrap();
            if detail.job.state == "interrupted" {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("load-next interruption");
    assert_eq!(
        interrupted.job.error_code.as_deref(),
        Some("download_injected_failure")
    );
    assert!(!matches!(
        interrupted.items[0].state.as_str(),
        "resolving" | "downloading"
    ));
}

struct FailingResolver;

impl DownloadJobResolver for FailingResolver {
    fn resolve<'a>(
        &'a self,
        _service: &'a DownloadService,
        _request: ResolverRequest,
        _cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        ResolvedMediaManifest,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async {
            Err(douyin_history_archive::download::DownloadError {
                code: "download_fixture_resolver_failed".into(),
                message: "测试解析失败".into(),
            })
        })
    }
}

struct RecordingTransfer {
    active: AtomicUsize,
    maximum: AtomicUsize,
}

struct OwnershipValidatingTransfer;

impl DownloadAssetTransfer for OwnershipValidatingTransfer {
    fn transfer<'a>(
        &'a self,
        request: TransferRequest,
        _cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        CompletedAsset,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let expected = asset_ownership_key(
                &request.destination_root,
                &request.aweme_id,
                &request.stable_key,
                &request.quality,
            );
            if request.ownership_key != expected {
                return Err(douyin_history_archive::download::DownloadError {
                    code: "download_part_not_owned".into(),
                    message: "测试检测到下载临时文件归属不匹配".into(),
                });
            }
            let guard = FilesystemRootAccess
                .guard_for_create(&request.destination_root, &request.relative_part_path)?;
            if guard.inspect()? != FileObservation::Missing {
                return Err(douyin_history_archive::download::DownloadError {
                    code: "download_fixture_path_not_empty".into(),
                    message: "测试下载路径不是空路径".into(),
                });
            }
            Ok(CompletedAsset {
                bytes: 4,
                sha256: "fixture-sha256".into(),
                etag: None,
                last_modified: None,
                relative_path: request.relative_final_path,
            })
        })
    }
}

struct MixedTransfer {
    active: AtomicUsize,
    maximum: AtomicUsize,
    calls: AtomicUsize,
    first_wave: tokio::sync::Barrier,
}

impl DownloadAssetTransfer for MixedTransfer {
    fn transfer<'a>(
        &'a self,
        request: TransferRequest,
        _cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        CompletedAsset,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
            let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.maximum.fetch_max(active, Ordering::AcqRel);
            if call <= 3 {
                self.first_wave.wait().await;
            }
            if request.stable_key == "image:0001" {
                tokio::time::sleep(Duration::from_millis(5)).await;
                self.active.fetch_sub(1, Ordering::AcqRel);
                return Err(douyin_history_archive::download::DownloadError {
                    code: "download_fixture_asset_failed".into(),
                    message: "测试资源下载失败".into(),
                });
            }
            tokio::time::sleep(Duration::from_millis(60)).await;
            self.active.fetch_sub(1, Ordering::AcqRel);
            Ok(CompletedAsset {
                bytes: 4,
                sha256: "fixture-sha256".into(),
                etag: None,
                last_modified: None,
                relative_path: request.relative_final_path,
            })
        })
    }
}

struct CountingResolver {
    calls: Arc<AtomicUsize>,
}

impl DownloadJobResolver for CountingResolver {
    fn resolve<'a>(
        &'a self,
        _service: &'a DownloadService,
        request: ResolverRequest,
        _cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        ResolvedMediaManifest,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(async move { Ok(fixture_manifest(request.aweme_id)) })
    }
}

struct CountingTransfer {
    calls: Arc<AtomicUsize>,
}

struct RefreshingResolver {
    calls: Arc<AtomicUsize>,
    always_expired: bool,
    change_identity_on_refresh: bool,
}

impl DownloadJobResolver for RefreshingResolver {
    fn resolve<'a>(
        &'a self,
        _service: &'a DownloadService,
        request: ResolverRequest,
        _cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        ResolvedMediaManifest,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        let call = self.calls.fetch_add(1, Ordering::AcqRel);
        let always_expired = self.always_expired;
        let change_identity_on_refresh = self.change_identity_on_refresh;
        Box::pin(async move {
            let mut manifest = fixture_manifest(request.aweme_id);
            let expiry = if always_expired || call == 0 {
                "2000-01-01T00:00:00Z"
            } else {
                "2999-01-01T00:00:00Z"
            };
            for asset in &mut manifest.assets {
                asset.source_expires_at = Some(expiry.into());
            }
            if change_identity_on_refresh && call > 0 {
                manifest.assets[0].asset_type = "video".into();
            }
            Ok(manifest)
        })
    }
}

struct UnauthorizedOnceTransfer {
    calls: Arc<AtomicUsize>,
}

struct BlockingTransfer {
    started: Arc<tokio::sync::Notify>,
}

impl DownloadAssetTransfer for BlockingTransfer {
    fn transfer<'a>(
        &'a self,
        _request: TransferRequest,
        cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        CompletedAsset,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.started.notify_one();
            cancellation.cancelled().await;
            Err(douyin_history_archive::download::DownloadError {
                code: "download_cancelled".into(),
                message: "下载已取消".into(),
            })
        })
    }
}

impl DownloadAssetTransfer for UnauthorizedOnceTransfer {
    fn transfer<'a>(
        &'a self,
        request: TransferRequest,
        _cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        CompletedAsset,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        let call = self.calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(async move {
            if call == 0 {
                Err(douyin_history_archive::download::DownloadError {
                    code: "download_source_expired".into(),
                    message: "下载地址已过期，需要重新解析".into(),
                })
            } else {
                Ok(CompletedAsset {
                    bytes: 4,
                    sha256: "fixture-sha256".into(),
                    etag: None,
                    last_modified: None,
                    relative_path: request.relative_final_path,
                })
            }
        })
    }
}

impl DownloadAssetTransfer for CountingTransfer {
    fn transfer<'a>(
        &'a self,
        request: TransferRequest,
        _cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        CompletedAsset,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(async move {
            Ok(CompletedAsset {
                bytes: 4,
                sha256: "fixture-sha256".into(),
                etag: None,
                last_modified: None,
                relative_path: request.relative_final_path,
            })
        })
    }
}

struct FailingTransfer;

impl DownloadAssetTransfer for FailingTransfer {
    fn transfer<'a>(
        &'a self,
        _request: TransferRequest,
        _cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        CompletedAsset,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async {
            Err(douyin_history_archive::download::DownloadError {
                code: "download_fixture_transfer_failed".into(),
                message: "测试传输失败".into(),
            })
        })
    }
}

struct ScriptedFaults {
    point: DownloadFaultPoint,
    remaining: AtomicUsize,
    calls: AtomicUsize,
}

struct TimedFaults {
    point: DownloadFaultPoint,
    remaining: AtomicUsize,
    calls: Mutex<Vec<Instant>>,
}

impl TimedFaults {
    fn new(point: DownloadFaultPoint, failures: usize) -> Self {
        Self {
            point,
            remaining: AtomicUsize::new(failures),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl DownloadFaultInjector for TimedFaults {
    fn check(
        &self,
        point: DownloadFaultPoint,
    ) -> Result<(), douyin_history_archive::download::DownloadError> {
        if point != self.point {
            return Ok(());
        }
        self.calls.lock().unwrap().push(Instant::now());
        if self
            .remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            Err(douyin_history_archive::download::DownloadError {
                code: "download_injected_failure".into(),
                message: "注入的下载阶段失败".into(),
            })
        } else {
            Ok(())
        }
    }
}

struct BatchFallbackFaults;

impl DownloadFaultInjector for BatchFallbackFaults {
    fn check(
        &self,
        point: DownloadFaultPoint,
    ) -> Result<(), douyin_history_archive::download::DownloadError> {
        if matches!(
            point,
            DownloadFaultPoint::CompleteAsset
                | DownloadFaultPoint::FinalizeAssetBatch
                | DownloadFaultPoint::FailItem
                | DownloadFaultPoint::InterruptJob
        ) {
            Err(douyin_history_archive::download::DownloadError {
                code: "download_injected_failure".into(),
                message: "注入的下载阶段失败".into(),
            })
        } else {
            Ok(())
        }
    }
}

impl ScriptedFaults {
    fn new(point: DownloadFaultPoint, failures: usize) -> Self {
        Self {
            point,
            remaining: AtomicUsize::new(failures),
            calls: AtomicUsize::new(0),
        }
    }
}

impl DownloadFaultInjector for ScriptedFaults {
    fn check(
        &self,
        point: DownloadFaultPoint,
    ) -> Result<(), douyin_history_archive::download::DownloadError> {
        if point != self.point {
            return Ok(());
        }
        self.calls.fetch_add(1, Ordering::AcqRel);
        let failed = self
            .remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok();
        if failed {
            Err(douyin_history_archive::download::DownloadError {
                code: "download_injected_failure".into(),
                message: "注入的下载阶段失败".into(),
            })
        } else {
            Ok(())
        }
    }
}

impl DownloadAssetTransfer for RecordingTransfer {
    fn transfer<'a>(
        &'a self,
        request: TransferRequest,
        _cancellation: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        CompletedAsset,
                        douyin_history_archive::download::DownloadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.maximum.fetch_max(active, Ordering::AcqRel);
            tokio::time::sleep(Duration::from_millis(40)).await;
            self.active.fetch_sub(1, Ordering::AcqRel);
            Ok(CompletedAsset {
                bytes: 4,
                sha256: "fixture-sha256".into(),
                etag: None,
                last_modified: None,
                relative_path: request.relative_final_path,
            })
        })
    }
}

fn fixture_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "emily-download-service-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

fn seed_history(database: &Path) {
    let mut connection = Connection::open(database).unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    connection.execute("INSERT INTO accounts (uid, profile_key, status, created_at, updated_at) VALUES ('download-service','0123456789abcdef0123456789abcdef','active','2026-01-01','2026-01-01')", []).unwrap();
    let account_id = connection.last_insert_rowid();
    connection.execute("INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)", [account_id]).unwrap();
    connection.execute("INSERT INTO videos (aweme_id, canonical_url, content_type, title, author_name, first_seen_at, last_seen_at) VALUES ('1001','https://www.douyin.com/note/1001','note','fixture','author','2026-01-01','2026-01-01')", []).unwrap();
    let video_id = connection.last_insert_rowid();
    connection.execute("INSERT INTO watch_records (account_id, video_id, history_time_ms, watched_at, watched_date, source_position, created_at, updated_at) VALUES (?1,?2,100,'2026-01-01','2026-01-01',0,'2026-01-01','2026-01-01')", params![account_id, video_id]).unwrap();
}

async fn wait_terminal_job(
    service: &DownloadService,
    job_id: i64,
) -> douyin_history_archive::download::DownloadJobDetail {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let detail = service.get_job(job_id).await.unwrap();
            if matches!(
                detail.job.state.as_str(),
                "completed" | "completed_with_errors" | "cancelled" | "interrupted"
            ) {
                return detail;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("download job terminal state")
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_sources_force_exactly_one_identity_checked_reresolution() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let transfer_calls = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(
                Arc::new(RefreshingResolver {
                    calls: Arc::clone(&resolver_calls),
                    always_expired: false,
                    change_identity_on_refresh: false,
                }),
                Arc::new(CountingTransfer {
                    calls: Arc::clone(&transfer_calls),
                }),
            ),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();

    let detail = wait_terminal_job(&service, created.job.id).await;
    assert_eq!(detail.job.state, "completed");
    assert_eq!(resolver_calls.load(Ordering::Acquire), 2);
    assert_eq!(transfer_calls.load(Ordering::Acquire), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_expired_manifest_fails_without_a_refresh_loop() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let transfer_calls = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(
                Arc::new(RefreshingResolver {
                    calls: Arc::clone(&resolver_calls),
                    always_expired: true,
                    change_identity_on_refresh: false,
                }),
                Arc::new(CountingTransfer {
                    calls: Arc::clone(&transfer_calls),
                }),
            ),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();

    let detail = wait_terminal_job(&service, created.job.id).await;
    assert_eq!(detail.job.state, "completed_with_errors");
    assert_eq!(
        detail.items[0].error_code.as_deref(),
        Some("download_source_expired")
    );
    assert_eq!(resolver_calls.load(Ordering::Acquire), 2);
    assert_eq!(transfer_calls.load(Ordering::Acquire), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn forced_refresh_rejects_a_changed_stable_asset_identity() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let transfer_calls = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(
                Arc::new(RefreshingResolver {
                    calls: Arc::clone(&resolver_calls),
                    always_expired: false,
                    change_identity_on_refresh: true,
                }),
                Arc::new(CountingTransfer {
                    calls: Arc::clone(&transfer_calls),
                }),
            ),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();

    let detail = wait_terminal_job(&service, created.job.id).await;
    assert_eq!(detail.job.state, "completed_with_errors");
    assert_eq!(
        detail.items[0].error_code.as_deref(),
        Some("download_refresh_identity_changed")
    );
    assert_eq!(resolver_calls.load(Ordering::Acquire), 2);
    assert_eq!(transfer_calls.load(Ordering::Acquire), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn authorization_expiry_refreshes_once_and_reuses_already_completed_assets() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let transfer_calls = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(
                Arc::new(CountingResolver {
                    calls: Arc::clone(&resolver_calls),
                }),
                Arc::new(UnauthorizedOnceTransfer {
                    calls: Arc::clone(&transfer_calls),
                }),
            ),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();

    let detail = wait_terminal_job(&service, created.job.id).await;
    assert_eq!(detail.job.state, "completed");
    assert_eq!(resolver_calls.load(Ordering::Acquire), 2);
    assert_eq!(transfer_calls.load(Ordering::Acquire), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_job_runs_to_completion_with_at_most_three_asset_transfers() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    let destination = root.join("downloads");
    seed_history(&database);
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FixtureResolver), transfer.clone()),
        )
        .await
        .unwrap(),
    );
    let mut events = service.subscribe();

    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: destination.to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();

    let completed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let detail = service.get_job(created.job.id).await.unwrap();
            if detail.job.state == "completed" {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("job completion");

    assert_eq!(completed.job.completed_count, 1);
    assert_eq!(completed.items[0].state, "completed");
    assert_eq!(completed.items[0].completed_asset_count, 4);
    assert_eq!(transfer.maximum.load(Ordering::Acquire), 3);
    assert!(
        events.try_recv().is_ok(),
        "background execution emitted no refresh event"
    );
    let connection = Connection::open(&database).unwrap();
    let completed_assets: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM download_assets WHERE state='completed' AND downloaded_bytes=4",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(completed_assets, 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn transfer_ownership_uses_the_existing_destination_root_and_nested_item_paths() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    let destination = root.join("downloads");
    seed_history(&database);
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(
                Arc::new(FixtureResolver),
                Arc::new(OwnershipValidatingTransfer),
            ),
        )
        .await
        .unwrap(),
    );

    let created = service
        .create_job(
            test_owner(),
            CreateDownloadJobRequest {
                selection: DownloadSelection::All,
                destination_root: destination.to_string_lossy().into_owned(),
                requested_quality: DownloadQuality::Balanced,
            },
        )
        .await
        .unwrap();
    let completed = wait_terminal_job(&service, created.job.id).await;

    assert_eq!(completed.job.state, "completed");
    assert_eq!(completed.job.failed_count, 0);
    assert_eq!(completed.items[0].state, "completed");
    let connection = Connection::open(&database).unwrap();
    let relative_work_directory: String = connection
        .query_row(
            "SELECT relative_work_directory FROM download_items WHERE job_id=?1",
            [created.job.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(destination.join(relative_work_directory).is_dir());
}

#[tokio::test(flavor = "multi_thread")]
async fn mixed_asset_batch_waits_for_every_transfer_before_terminal_derivation() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let transfer = Arc::new(MixedTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
        first_wave: tokio::sync::Barrier::new(3),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FixtureResolver), transfer.clone()),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();

    let detail = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let detail = service.get_job(created.job.id).await.unwrap();
            if detail.job.state == "completed_with_errors" {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let detail = match detail {
        Ok(detail) => detail,
        Err(_) => {
            let detail = service.get_job(created.job.id).await.unwrap();
            let connection = Connection::open(&database).unwrap();
            let states = connection
                .prepare("SELECT state,last_error_code FROM download_assets ORDER BY ordinal,id")
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            panic!("mixed batch did not converge: detail={detail:?}, assets={states:?}");
        }
    };
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert_eq!(transfer.calls.load(Ordering::Acquire), 4);
    assert_eq!(transfer.maximum.load(Ordering::Acquire), 3);
    assert_eq!(detail.items[0].state, "failed");
    assert_eq!(
        detail.items[0].error_code.as_deref(),
        Some("download_fixture_asset_failed")
    );
    assert_eq!(
        detail.items[0].error_message.as_deref(),
        Some("测试资源下载失败")
    );
    let connection = Connection::open(&database).unwrap();
    let states = connection
        .prepare("SELECT state FROM download_assets ORDER BY ordinal,id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(states.len(), 4);
    assert!(states
        .iter()
        .all(|state| matches!(state.as_str(), "completed" | "failed")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_queued_job_stays_offline_until_explicit_resume() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let mut connection = Connection::open(&database).unwrap();
    let job_id = create_download_job(
        &mut connection,
        &test_owner(),
        &CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        },
    )
    .unwrap();
    drop(connection);
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let transfer_calls = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(
                Arc::new(CountingResolver {
                    calls: Arc::clone(&resolver_calls),
                }),
                Arc::new(CountingTransfer {
                    calls: Arc::clone(&transfer_calls),
                }),
            ),
        )
        .await
        .unwrap(),
    );

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(service.get_job(job_id).await.unwrap().job.state, "queued");
    assert_eq!(resolver_calls.load(Ordering::Acquire), 0);
    service.resume_job(job_id).await.unwrap();
    let detail = wait_terminal_job(&service, job_id).await;
    assert_eq!(detail.job.state, "completed");
    assert_eq!(resolver_calls.load(Ordering::Acquire), 1);
    assert_eq!(transfer_calls.load(Ordering::Acquire), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn graceful_shutdown_cancels_work_and_durably_pauses_all_active_rows() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let started = Arc::new(tokio::sync::Notify::new());
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(
                Arc::new(FixtureResolver),
                Arc::new(BlockingTransfer {
                    started: Arc::clone(&started),
                }),
            ),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .expect("asset transfer start");

    service.request_shutdown().await.unwrap();
    assert!(service
        .wait_for_shutdown(Duration::from_secs(5))
        .await
        .unwrap());
    service.request_shutdown().await.unwrap();
    let detail = service.get_job(created.job.id).await.unwrap();
    assert_eq!(detail.job.state, "paused");
    assert!(detail.items.iter().all(|item| item.state == "paused"));
    let connection = Connection::open(&database).unwrap();
    let active_assets: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM download_assets a JOIN download_items i ON i.id=a.item_id WHERE i.job_id=?1 AND a.state IN ('queued','downloading')",
            [created.job.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_assets, 0);
    let paused_assets: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM download_assets a JOIN download_items i ON i.id=a.item_id WHERE i.job_id=?1 AND a.state='paused'",
            [created.job.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(paused_assets, 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_startup_recovery_does_not_touch_network_until_manual_resume() {
    let root = fixture_root();
    let destination = root.join("downloads");
    std::fs::create_dir_all(&destination).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let mut connection = Connection::open(&database).unwrap();
    let job_id = create_download_job(
        &mut connection,
        &test_owner(),
        &CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: destination.to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        },
    )
    .unwrap();
    let item_id: i64 = connection
        .query_row(
            "SELECT id FROM download_items WHERE job_id=?1",
            [job_id],
            |row| row.get(0),
        )
        .unwrap();
    reconcile_manifest(&mut connection, item_id, &fixture_manifest("1001")).unwrap();
    connection
        .execute(
            "UPDATE download_jobs SET state='running' WHERE id=?1",
            [job_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE download_items SET state='downloading' WHERE id=?1",
            [item_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE download_assets SET state='downloading' WHERE item_id=?1",
            [item_id],
        )
        .unwrap();
    drop(connection);

    DownloadService::recover_offline(&database).unwrap();
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let transfer_calls = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(
                Arc::new(CountingResolver {
                    calls: resolver_calls.clone(),
                }),
                Arc::new(CountingTransfer {
                    calls: transfer_calls.clone(),
                }),
            ),
        )
        .await
        .unwrap(),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    let recovered = service.get_job(job_id).await.unwrap();
    assert_eq!(recovered.job.state, "interrupted");
    assert_eq!(recovered.items[0].state, "interrupted");
    assert_eq!(resolver_calls.load(Ordering::Acquire), 0);
    assert_eq!(transfer_calls.load(Ordering::Acquire), 0);

    service.resume_job(job_id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if service.get_job(job_id).await.unwrap().job.state == "completed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("manual resume completion");
    assert_eq!(resolver_calls.load(Ordering::Acquire), 1);
    assert_eq!(transfer_calls.load(Ordering::Acquire), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn resolver_failure_persists_item_error_and_converges_the_job() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FailingResolver), transfer),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();

    let failed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let detail = service.get_job(created.job.id).await.unwrap();
            if detail.job.state != "running" && detail.job.state != "queued" {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("failure convergence");
    assert_eq!(failed.job.state, "completed_with_errors");
    assert_eq!(failed.items[0].state, "failed");
    assert_eq!(
        failed.items[0].error_code.as_deref(),
        Some("download_fixture_resolver_failed")
    );
    assert_eq!(
        failed.items[0].error_message.as_deref(),
        Some("测试解析失败")
    );
}

async fn run_fault_case(
    point: DownloadFaultPoint,
    transfer: Arc<dyn DownloadAssetTransfer>,
) -> douyin_history_archive::download::DownloadJobDetail {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let faults = Arc::new(ScriptedFaults::new(point, 1));
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FixtureResolver), transfer)
                .with_fault_injector(faults),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let detail = service.get_job(created.job.id).await.unwrap();
            if !matches!(detail.job.state.as_str(), "queued" | "running") {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("fault convergence")
}

#[tokio::test(flavor = "multi_thread")]
async fn reconcile_request_and_start_asset_failures_converge_to_item_failed() {
    for point in [
        DownloadFaultPoint::ReconcileManifest,
        DownloadFaultPoint::LoadTransferRequests,
        DownloadFaultPoint::StartAsset,
    ] {
        let transfer = Arc::new(RecordingTransfer {
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
        });
        let detail = run_fault_case(point, transfer).await;
        assert_eq!(detail.job.state, "completed_with_errors", "{point:?}");
        assert_eq!(detail.items[0].state, "failed", "{point:?}");
        assert_eq!(
            detail.items[0].error_code.as_deref(),
            Some("download_injected_failure"),
            "{point:?}"
        );
        assert_eq!(
            detail.items[0].error_message.as_deref(),
            Some("注入的下载阶段失败"),
            "{point:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_and_fail_asset_persistence_failures_converge_to_item_failed() {
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let complete = run_fault_case(DownloadFaultPoint::CompleteAsset, transfer).await;
    assert_eq!(complete.job.state, "completed_with_errors");
    assert_eq!(complete.items[0].state, "failed");
    let failed = run_fault_case(DownloadFaultPoint::FailAsset, Arc::new(FailingTransfer)).await;
    assert_eq!(failed.job.state, "completed_with_errors");
    assert_eq!(failed.items[0].state, "failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_item_persistence_failure_interrupts_the_job() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let faults = Arc::new(ScriptedFaults::new(DownloadFaultPoint::FailItem, 1));
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FailingResolver), transfer)
                .with_fault_injector(faults),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();
    let interrupted = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let detail = service.get_job(created.job.id).await.unwrap();
            if detail.job.state == "interrupted" {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("job interruption");
    assert_eq!(interrupted.items[0].state, "interrupted");
    assert_eq!(
        interrupted.job.error_code.as_deref(),
        Some("download_failure_persistence_failed")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn actor_interrupt_failure_uses_atomic_direct_fallback() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FixtureResolver), transfer)
                .with_fault_injector(Arc::new(BatchFallbackFaults)),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();
    let detail = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let detail = service.get_job(created.job.id).await.unwrap();
            if detail.job.state == "interrupted" {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("direct fallback interruption");
    assert_eq!(detail.items[0].state, "interrupted");
    let connection = Connection::open(&database).unwrap();
    let asset_counts: (i64, i64) = connection
        .query_row(
            "SELECT COUNT(*),SUM(CASE WHEN state='interrupted' THEN 1 ELSE 0 END) FROM download_assets",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(asset_counts, (4, 4));
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduler_query_failure_recovers_after_more_than_the_initial_budget() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let faults = Arc::new(ScriptedFaults::new(DownloadFaultPoint::QueryRunning, 5));
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FixtureResolver), transfer)
                .with_fault_injector(faults.clone()),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if service.get_job(created.job.id).await.unwrap().job.state == "completed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("scheduler recovery without an external wake");
    assert!(faults.calls.load(Ordering::Acquire) >= 6);
}

#[tokio::test(flavor = "multi_thread")]
async fn persistent_scheduler_query_failure_retries_at_a_bounded_rate() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let faults = Arc::new(ScriptedFaults::new(
        DownloadFaultPoint::QueryRunning,
        usize::MAX,
    ));
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FixtureResolver), transfer)
                .with_fault_injector(faults.clone()),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(260)).await;
    let calls = faults.calls.load(Ordering::Acquire);
    assert!(
        (4..=8).contains(&calls),
        "unbounded or stopped retry count: {calls}"
    );
    assert!(matches!(
        service
            .get_job(created.job.id)
            .await
            .unwrap()
            .job
            .state
            .as_str(),
        "queued" | "running"
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn single_handoff_query_failure_sleeps_then_exits_without_external_wake() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let faults = Arc::new(TimedFaults::new(DownloadFaultPoint::HasRunnableJob, 1));
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FixtureResolver), transfer)
                .with_fault_injector(faults.clone()),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let completed = service.get_job(created.job.id).await.unwrap().job.state == "completed";
            if completed && faults.calls.lock().unwrap().len() >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("handoff recovery and exit");
    let calls = faults.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(
        calls[1].duration_since(calls[0]) >= Duration::from_millis(20),
        "handoff retried without backoff: {:?}",
        calls[1].duration_since(calls[0])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn persistent_handoff_query_failure_retries_at_a_bounded_rate() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let faults = Arc::new(ScriptedFaults::new(
        DownloadFaultPoint::HasRunnableJob,
        usize::MAX,
    ));
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FixtureResolver), transfer)
                .with_fault_injector(faults.clone()),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while faults.calls.load(Ordering::Acquire) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first handoff query");
    let baseline = faults.calls.load(Ordering::Acquire);
    tokio::time::sleep(Duration::from_millis(260)).await;
    let additional = faults.calls.load(Ordering::Acquire) - baseline;
    assert!(
        (3..=7).contains(&additional),
        "handoff retry rate was unbounded or stopped: {additional}"
    );
    assert_eq!(
        service.get_job(created.job.id).await.unwrap().job.state,
        "completed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn persistent_start_next_failure_keeps_its_own_capped_backoff() {
    let root = fixture_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("history.sqlite");
    seed_history(&database);
    let faults = Arc::new(TimedFaults::new(DownloadFaultPoint::StartNext, usize::MAX));
    let transfer = Arc::new(RecordingTransfer {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    });
    let service = Arc::new(
        DownloadService::spawn_with_runtime(
            &database,
            BrowserOperationCoordinator::default(),
            DownloadRuntimeConfig::new(Arc::new(FixtureResolver), transfer)
                .with_fault_injector(faults.clone()),
        )
        .await
        .unwrap(),
    );
    let created = service
        .create_job(test_owner(), CreateDownloadJobRequest {
            selection: DownloadSelection::All,
            destination_root: root.join("downloads").to_string_lossy().into_owned(),
            requested_quality: DownloadQuality::Balanced,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if faults.calls.lock().unwrap().len() >= 4 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("StartNext retries did not continue");
    let calls = faults.calls.lock().unwrap().clone();
    let minimum_gaps = [20, 40, 80];
    for (index, minimum_ms) in minimum_gaps.into_iter().enumerate() {
        let gap = calls[index + 1].duration_since(calls[index]);
        assert!(
            gap >= Duration::from_millis(minimum_ms),
            "StartNext retry {index} skipped backoff: {gap:?}"
        );
    }
    assert_eq!(
        service.get_job(created.job.id).await.unwrap().job.state,
        "queued"
    );
}
