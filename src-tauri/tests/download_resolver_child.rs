use douyin_history_archive::download::{
    asset_ownership_key, parse_resolver_child_args, partial_filename, AssetClass,
    BrowserOperationCoordinator, DownloadQuality, DownloadService, DownloadWorker,
    JobTransferQuota, PublicInternetPolicy, ResolverChildOptions, ResolverRequest,
    ResolverResponse, TransferRequest,
};
use douyin_history_archive::migrate_schema_to_latest_at_startup;
use futures_util::{SinkExt, StreamExt};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

// Resolver fixture controls use process-wide EMILY_TEST_* environment
// variables inherited by every spawned application child. Serialize all
// resolver-spawning tests so parallel harness execution cannot cross-wire them.
static RESOLVER_PROCESS_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn prepare_database(root: &std::path::Path) -> PathBuf {
    let database = root.join("history.sqlite");
    let mut connection = Connection::open(&database).expect("download database");
    migrate_schema_to_latest_at_startup(&mut connection).expect("latest schema");
    database
}

#[test]
fn resolver_child_uses_anonymous_framed_pipes_without_manifest_files() {
    let options = parse_resolver_child_args(vec![
        OsString::from("--browser-data-dir"),
        OsString::from("C:/Emily"),
        OsString::from("--browser-pid-file"),
        OsString::from("C:/Emily/tmp/download-7.browser.pid"),
    ])
    .expect("resolver options");

    assert_eq!(options.browser_data_dir, PathBuf::from("C:/Emily"));
    assert_eq!(
        options.browser_pid_file,
        PathBuf::from("C:/Emily/tmp/download-7.browser.pid")
    );
    assert_eq!(
        options,
        ResolverChildOptions {
            browser_data_dir: PathBuf::from("C:/Emily"),
            browser_pid_file: PathBuf::from("C:/Emily/tmp/download-7.browser.pid"),
            browser_debug_url: None,
            browser_debug_port: None,
        }
    );

    let rejected = parse_resolver_child_args(vec![
        OsString::from("--browser-data-dir"),
        OsString::from("C:/Emily"),
        OsString::from("--browser-pid-file"),
        OsString::from("C:/Emily/tmp/download-7.browser.pid"),
        OsString::from("--manifest-file"),
        OsString::from("C:/Emily/tmp/sensitive.json"),
    ]);
    assert_eq!(rejected, Err("resolver_invalid_arguments".to_string()));

    let remote_debug = parse_resolver_child_args(vec![
        OsString::from("--browser-data-dir"),
        OsString::from("C:/Emily"),
        OsString::from("--browser-pid-file"),
        OsString::from("C:/Emily/tmp/download-7.browser.pid"),
        OsString::from("--browser-debug-url"),
        OsString::from("ws://example.com/devtools/browser/test"),
        OsString::from("--browser-debug-port"),
        OsString::from("9222"),
    ]);
    assert_eq!(remote_debug, Err("resolver_invalid_arguments".to_string()));
}

#[test]
fn browser_operation_lease_remains_exclusive_until_owner_drops_it() {
    let coordinator = BrowserOperationCoordinator::default();
    let lease = coordinator.acquire().expect("first browser operation");
    assert_eq!(
        coordinator.acquire().unwrap_err(),
        "已有登录、同步或下载解析操作正在运行"
    );
    drop(lease);
    assert!(coordinator.acquire().is_ok());
}

struct MockCdp {
    browser_url: String,
    page_port: u16,
    page_connected: Arc<AtomicBool>,
    media_failed: Arc<AtomicBool>,
    media_body_requested: Arc<AtomicBool>,
    browser_closed: Arc<AtomicBool>,
    browser_disconnected: Arc<AtomicBool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl MockCdp {
    async fn start(send_detail: bool) -> Self {
        let browser = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let page = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let browser_address = browser.local_addr().unwrap();
        let page_port = page.local_addr().unwrap().port();
        let page_connected = Arc::new(AtomicBool::new(false));
        let media_failed = Arc::new(AtomicBool::new(false));
        let media_body_requested = Arc::new(AtomicBool::new(false));
        let browser_closed = Arc::new(AtomicBool::new(false));
        let browser_disconnected = Arc::new(AtomicBool::new(false));

        let closed = Arc::clone(&browser_closed);
        let disconnected = Arc::clone(&browser_disconnected);
        let browser_task = tokio::spawn(async move {
            let (stream, _) = browser.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let request: Value = serde_json::from_str(text.as_ref()).unwrap();
                let id = request["id"].as_u64().unwrap();
                let method = request["method"].as_str().unwrap();
                let result = match method {
                    "Target.createTarget" => json!({"targetId":"fixture"}),
                    "Target.closeTarget" => json!({"success":true}),
                    "Browser.close" => {
                        closed.store(true, Ordering::Release);
                        json!({})
                    }
                    _ => json!({}),
                };
                socket
                    .send(Message::Text(
                        json!({"id":id,"result":result}).to_string().into(),
                    ))
                    .await
                    .unwrap();
            }
            disconnected.store(true, Ordering::Release);
        });

        let connected = Arc::clone(&page_connected);
        let failed = Arc::clone(&media_failed);
        let media_body = Arc::clone(&media_body_requested);
        let page_task = tokio::spawn(async move {
            let (stream, _) = page.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            connected.store(true, Ordering::Release);
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let request: Value = serde_json::from_str(text.as_ref()).unwrap();
                let id = request["id"].as_u64().unwrap();
                let method = request["method"].as_str().unwrap();
                match method {
                    "Page.navigate" => {
                        socket
                            .send(Message::Text(
                                json!({"id":id,"result":{}}).to_string().into(),
                            ))
                            .await
                            .unwrap();
                        socket.send(Message::Text(json!({"method":"Fetch.requestPaused","params":{"requestId":"media-paused","resourceType":"Media"}}).to_string().into())).await.unwrap();
                        if send_detail {
                            socket.send(Message::Text(json!({"method":"Network.responseReceived","params":{"requestId":"media-1","type":"Media","response":{"url":"http://fixture/media/video.mp4","status":200,"mimeType":"video/mp4"}}}).to_string().into())).await.unwrap();
                            socket.send(Message::Text(json!({"method":"Network.responseReceived","params":{"requestId":"detail-unrelated","type":"Fetch","response":{"url":"https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id=7123456789012345678&aid=6383","status":200,"mimeType":"application/json"}}}).to_string().into())).await.unwrap();
                            socket.send(Message::Text(json!({"method":"Network.responseReceived","params":{"requestId":"detail-1","type":"Fetch","response":{"url":"https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id=7123456789012345678&aid=1128","status":200,"mimeType":"application/json"}}}).to_string().into())).await.unwrap();
                        }
                        socket
                            .send(Message::Text(
                                json!({"method":"Page.domContentEventFired","params":{}})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap();
                    }
                    "Network.getResponseBody" => {
                        let request_id = request["params"]["requestId"].as_str().unwrap();
                        if request_id == "media-1" {
                            media_body.store(true, Ordering::Release);
                        }
                        let body = if request_id == "detail-unrelated" {
                            json!({"status_code":0,"aweme_list":[]}).to_string()
                        } else {
                            json!({
                                "status_code":0,
                                "extra":{"logid":"sanitized"},
                                "aweme_detail":{
                                    "aweme_id":"7123456789012345678",
                                    "aweme_type":4,
                                    "video":{
                                        "width":1920,
                                        "height":1080,
                                        "bit_rate":[{
                                            "gear_name":"1080p_h264",
                                            "bit_rate":4000000,
                                            "is_h265":0,
                                            "play_addr":{
                                                "url_list":["https://cdn.example.test/video.mp4"],
                                                "width":1920,
                                                "height":1080,
                                                "data_size":1024
                                            }
                                        }]
                                    },
                                    "unrelated_upstream_field":{"ignored":true}
                                }
                            })
                            .to_string()
                        };
                        socket
                            .send(Message::Text(
                                json!({"id":id,"result":{"body":body,"base64Encoded":false}})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap();
                    }
                    "Runtime.evaluate" => {
                        assert_eq!(request["params"]["expression"], "navigator.userAgent");
                        socket
                            .send(Message::Text(
                                json!({
                                    "id": id,
                                    "result": {
                                        "result": {
                                            "type": "string",
                                            "value": "EmilyFixtureBrowser/1.0"
                                        }
                                    }
                                })
                                .to_string()
                                .into(),
                            ))
                            .await
                            .unwrap();
                    }
                    "Fetch.failRequest" => {
                        failed.store(true, Ordering::Release);
                    }
                    _ => {
                        socket
                            .send(Message::Text(
                                json!({"id":id,"result":{}}).to_string().into(),
                            ))
                            .await
                            .unwrap();
                    }
                }
            }
        });

        Self {
            browser_url: format!("ws://{browser_address}/devtools/browser/fixture"),
            page_port,
            page_connected,
            media_failed,
            media_body_requested,
            browser_closed,
            browser_disconnected,
            tasks: vec![browser_task, page_task],
        }
    }
}

impl Drop for MockCdp {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

fn temp_root(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("emily-{label}-{}-{suffix}", std::process::id()))
}

async fn wait_flag(flag: &AtomicBool) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !flag.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("fixture flag");
}

async fn wait_path(path: &std::path::Path) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("fixture path");
}

async fn wait_path_absent(path: &std::path::Path) {
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("fixture path cleanup");
}

async fn wait_lease_available(coordinator: &BrowserOperationCoordinator) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(lease) = coordinator.acquire() {
                drop(lease);
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("browser lease cleanup");
}

fn fixture_request(request_id: u64) -> ResolverRequest {
    ResolverRequest {
        request_id,
        account_id: 1,
        profile_key: "0123456789abcdef0123456789abcdef".into(),
        aweme_id: "7123456789012345678".into(),
        canonical_url: "http://fixture/video/7123456789012345678".into(),
        quality: DownloadQuality::Highest,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn resolver_child_emits_one_manifest_without_downloading_media_body_and_holds_lease() {
    let _resolver_process_env = RESOLVER_PROCESS_ENV.lock().await;
    let fixture = MockCdp::start(true).await;
    let root = temp_root("resolver-child");
    std::fs::create_dir_all(&root).unwrap();
    let coordinator = BrowserOperationCoordinator::default();
    let service = Arc::new(
        DownloadService::spawn(prepare_database(&root), coordinator.clone())
            .await
            .unwrap(),
    );
    let service_task = Arc::clone(&service);
    let browser_url = fixture.browser_url.clone();
    let page_port = fixture.page_port;
    let browser_pid_file = root.join("download-7.browser.pid");
    let resolve = tokio::spawn(async move {
        service_task
            .resolve_with_debug_endpoint(
                PathBuf::from(env!("CARGO_BIN_EXE_douyin-history-archive")).as_path(),
                &root,
                &browser_pid_file,
                fixture_request(7),
                Some((&browser_url, page_port)),
            )
            .await
    });

    wait_flag(&fixture.page_connected).await;
    assert!(
        coordinator.acquire().is_err(),
        "lease released before child exit"
    );
    let response = resolve.await.unwrap().unwrap();
    let ResolverResponse::Resolved {
        request_id,
        manifest,
    } = response
    else {
        panic!("manifest frame");
    };
    assert_eq!(request_id, 7);
    assert_eq!(manifest.aweme_id, "7123456789012345678");
    assert_eq!(manifest.assets.len(), 1);
    assert_eq!(manifest.assets[0].request_headers.len(), 2);
    assert_eq!(
        manifest.assets[0].request_headers.get("Referer"),
        Some(&"https://www.douyin.com/".to_string())
    );
    assert_eq!(
        manifest.assets[0].request_headers.get("User-Agent"),
        Some(&"EmilyFixtureBrowser/1.0".to_string())
    );
    assert!(fixture.media_failed.load(Ordering::Acquire));
    assert!(!fixture.media_body_requested.load(Ordering::Acquire));
    assert!(fixture.browser_closed.load(Ordering::Acquire));
    assert!(
        coordinator.acquire().is_ok(),
        "lease retained after child exit"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_active_resolver_closes_child_and_releases_browser_lease() {
    let _resolver_process_env = RESOLVER_PROCESS_ENV.lock().await;
    let fixture = MockCdp::start(false).await;
    let root = temp_root("resolver-cancel");
    std::fs::create_dir_all(&root).unwrap();
    let coordinator = BrowserOperationCoordinator::default();
    let service = Arc::new(
        DownloadService::spawn(prepare_database(&root), coordinator.clone())
            .await
            .unwrap(),
    );
    let service_task = Arc::clone(&service);
    let browser_url = fixture.browser_url.clone();
    let page_port = fixture.page_port;
    let resolve_root = root.clone();
    let resolve = tokio::spawn(async move {
        service_task
            .resolve_with_debug_endpoint(
                PathBuf::from(env!("CARGO_BIN_EXE_douyin-history-archive")).as_path(),
                &resolve_root,
                &resolve_root.join("download-8.browser.pid"),
                fixture_request(8),
                Some((&browser_url, page_port)),
            )
            .await
    });
    wait_flag(&fixture.page_connected).await;
    service.cancel_active().await.unwrap();
    let response = resolve.await.unwrap().unwrap();
    assert!(
        matches!(response, ResolverResponse::Error { code, .. } if code == "download_cancelled")
    );
    wait_flag(&fixture.browser_closed).await;
    assert!(coordinator.acquire().is_ok());
    assert!(!root.join("download-8.browser.pid").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_force_terminates_a_resolver_that_ignores_cancel_without_leaking_the_lease() {
    let _resolver_process_env = RESOLVER_PROCESS_ENV.lock().await;
    let fixture = MockCdp::start(true).await;
    let root = temp_root("resolver-forced-cancel");
    std::fs::create_dir_all(&root).unwrap();
    let pid_file = root.join("download-10.browser.pid");
    let hung_child_pid_file = root.join("hung-resolver.pid");
    let service_wait_file = root.join("service-waiting.pid");
    std::env::set_var(
        "EMILY_TEST_RESOLVER_HANG_AFTER_FRAME_PID_FILE",
        &hung_child_pid_file,
    );
    std::env::set_var("EMILY_TEST_SERVICE_CHILD_WAIT_MARKER", &service_wait_file);
    let coordinator = BrowserOperationCoordinator::default();
    let service = Arc::new(
        DownloadService::spawn(prepare_database(&root), coordinator.clone())
            .await
            .unwrap(),
    );
    let service_task = Arc::clone(&service);
    let browser_url = fixture.browser_url.clone();
    let page_port = fixture.page_port;
    let resolve_root = root.clone();
    let resolve_pid_file = pid_file.clone();
    let resolve = tokio::spawn(async move {
        service_task
            .resolve_with_debug_endpoint(
                PathBuf::from(env!("CARGO_BIN_EXE_douyin-history-archive")).as_path(),
                &resolve_root,
                &resolve_pid_file,
                fixture_request(10),
                Some((&browser_url, page_port)),
            )
            .await
    });
    wait_flag(&fixture.page_connected).await;
    wait_path(&hung_child_pid_file).await;
    wait_path(&service_wait_file).await;
    std::env::remove_var("EMILY_TEST_RESOLVER_HANG_AFTER_FRAME_PID_FILE");
    std::env::remove_var("EMILY_TEST_SERVICE_CHILD_WAIT_MARKER");
    std::fs::write(&pid_file, b"fixture pid record").unwrap();

    let cancelled =
        tokio::time::timeout(std::time::Duration::from_secs(4), service.cancel_active()).await;
    if cancelled.is_err() {
        if let Ok(pid) = std::fs::read_to_string(&hung_child_pid_file) {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", pid.trim(), "/T", "/F"])
                .output();
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), resolve).await;
    }

    assert!(
        cancelled.is_ok(),
        "forced cancellation exceeded its bounded timeout"
    );
    assert!(cancelled.unwrap().is_ok());
    wait_flag(&fixture.browser_disconnected).await;
    assert!(!pid_file.exists());
    assert!(coordinator.acquire().is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_service_keeps_pid_and_lease_until_child_owner_confirms_exit() {
    let _resolver_process_env = RESOLVER_PROCESS_ENV.lock().await;
    let fixture = MockCdp::start(true).await;
    let root = temp_root("resolver-drop-confirmation");
    std::fs::create_dir_all(&root).unwrap();
    let pid_file = root.join("download-11.browser.pid");
    let hung_child_pid_file = root.join("hung-resolver.pid");
    let service_wait_file = root.join("service-waiting.pid");
    let exit_delay_file = root.join("owner-delaying-exit.pid");
    std::env::set_var(
        "EMILY_TEST_RESOLVER_HANG_AFTER_FRAME_PID_FILE",
        &hung_child_pid_file,
    );
    std::env::set_var("EMILY_TEST_SERVICE_CHILD_WAIT_MARKER", &service_wait_file);
    std::env::set_var("EMILY_TEST_SERVICE_EXIT_CONFIRMATION_DELAY_MS", "4000");
    std::env::set_var("EMILY_TEST_SERVICE_EXIT_DELAY_MARKER", &exit_delay_file);
    let coordinator = BrowserOperationCoordinator::default();
    let service = Arc::new(
        DownloadService::spawn(prepare_database(&root), coordinator.clone())
            .await
            .unwrap(),
    );
    let service_task = Arc::clone(&service);
    let browser_url = fixture.browser_url.clone();
    let page_port = fixture.page_port;
    let resolve_root = root.clone();
    let resolve_pid_file = pid_file.clone();
    let resolve = tokio::spawn(async move {
        service_task
            .resolve_with_debug_endpoint(
                PathBuf::from(env!("CARGO_BIN_EXE_douyin-history-archive")).as_path(),
                &resolve_root,
                &resolve_pid_file,
                fixture_request(11),
                Some((&browser_url, page_port)),
            )
            .await
    });
    wait_flag(&fixture.page_connected).await;
    wait_path(&hung_child_pid_file).await;
    wait_path(&service_wait_file).await;
    std::fs::write(&pid_file, b"fixture pid record").unwrap();

    resolve.abort();
    let _ = resolve.await;
    drop(service);
    wait_flag(&fixture.browser_disconnected).await;
    wait_path(&exit_delay_file).await;

    assert!(pid_file.exists(), "PID removed before owner confirmed exit");
    assert!(
        coordinator.acquire().is_err(),
        "lease released before owner confirmed exit"
    );
    wait_path_absent(&pid_file).await;
    wait_lease_available(&coordinator).await;

    std::env::remove_var("EMILY_TEST_RESOLVER_HANG_AFTER_FRAME_PID_FILE");
    std::env::remove_var("EMILY_TEST_SERVICE_CHILD_WAIT_MARKER");
    std::env::remove_var("EMILY_TEST_SERVICE_EXIT_CONFIRMATION_DELAY_MS");
    std::env::remove_var("EMILY_TEST_SERVICE_EXIT_DELAY_MARKER");
}

#[tokio::test(flavor = "multi_thread")]
async fn write_failure_timeout_keeps_pid_and_lease_until_owner_confirms_exit() {
    let _resolver_process_env = RESOLVER_PROCESS_ENV.lock().await;
    let root = temp_root("resolver-write-failure-confirmation");
    std::fs::create_dir_all(&root).unwrap();
    let pid_file = root.join("download-12.browser.pid");
    let exit_delay_file = root.join("owner-delaying-exit.pid");
    std::fs::write(&pid_file, b"fixture pid record").unwrap();
    std::env::set_var("EMILY_TEST_SERVICE_FAIL_RESOLVER_WRITE", "1");
    std::env::set_var("EMILY_TEST_SERVICE_EXIT_CONFIRMATION_DELAY_MS", "4000");
    std::env::set_var("EMILY_TEST_SERVICE_EXIT_DELAY_MARKER", &exit_delay_file);
    let coordinator = BrowserOperationCoordinator::default();
    let service = DownloadService::spawn(prepare_database(&root), coordinator.clone())
        .await
        .unwrap();

    let result = service
        .resolve(
            PathBuf::from(env!("CARGO_BIN_EXE_douyin-history-archive")).as_path(),
            &root,
            &pid_file,
            fixture_request(12),
        )
        .await;
    std::env::remove_var("EMILY_TEST_SERVICE_FAIL_RESOLVER_WRITE");
    wait_path(&exit_delay_file).await;

    assert!(
        result.is_err(),
        "fixture did not force request write failure"
    );
    assert!(pid_file.exists(), "PID removed before owner confirmed exit");
    assert!(
        coordinator.acquire().is_err(),
        "lease released before owner confirmed exit"
    );
    wait_path_absent(&pid_file).await;
    wait_lease_available(&coordinator).await;
    let stale = service.cancel_active().await.unwrap_err();
    assert_eq!(stale.message, "resolver not active");

    std::env::remove_var("EMILY_TEST_SERVICE_EXIT_CONFIRMATION_DELAY_MS");
    std::env::remove_var("EMILY_TEST_SERVICE_EXIT_DELAY_MARKER");
}

#[cfg(target_os = "windows")]
#[tokio::test(flavor = "multi_thread")]
async fn resolver_crash_before_a_frame_releases_the_lease_and_active_child() {
    let _resolver_process_env = RESOLVER_PROCESS_ENV.lock().await;
    let root = temp_root("resolver-crash");
    std::fs::create_dir_all(&root).unwrap();
    let coordinator = BrowserOperationCoordinator::default();
    let service = DownloadService::spawn(prepare_database(&root), coordinator.clone())
        .await
        .unwrap();
    let result = service
        .resolve(
            PathBuf::from(r"C:\Windows\System32\where.exe").as_path(),
            &root,
            &root.join("download-9.browser.pid"),
            fixture_request(9),
        )
        .await;
    assert!(result.is_err());
    assert!(coordinator.acquire().is_ok());
    assert!(!root.join("download-9.browser.pid").exists());
}

#[cfg(target_os = "windows")]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires EMILY_TEST_DOWNLOAD_BROWSER_DATA_DIR and EMILY_TEST_DOWNLOAD_VIDEO_URL"]
async fn authenticated_public_video_resolves_and_downloads_with_explicit_test_profile() {
    let _resolver_process_env = RESOLVER_PROCESS_ENV.lock().await;
    let browser_data_dir = std::env::var_os("EMILY_TEST_DOWNLOAD_BROWSER_DATA_DIR")
        .map(PathBuf::from)
        .expect("environment blocked: set EMILY_TEST_DOWNLOAD_BROWSER_DATA_DIR to an isolated prepared test profile root");
    assert!(
        browser_data_dir.is_absolute(),
        "environment blocked: test profile root must be absolute"
    );
    assert!(browser_data_dir.join(".emily-download-acceptance-profile").is_file(), "environment blocked: isolated test profile root must contain .emily-download-acceptance-profile");
    let canonical_url = std::env::var("EMILY_TEST_DOWNLOAD_VIDEO_URL").expect(
        "environment blocked: set EMILY_TEST_DOWNLOAD_VIDEO_URL to a public normal-video URL",
    );
    let parsed = url::Url::parse(&canonical_url).expect("acceptance URL must be valid");
    assert_eq!(parsed.scheme(), "https", "acceptance URL must use HTTPS");
    assert!(
        parsed
            .host_str()
            .is_some_and(|host| host == "douyin.com" || host.ends_with(".douyin.com")),
        "acceptance URL must be on douyin.com"
    );
    let aweme_id = parsed
        .path_segments()
        .and_then(Iterator::last)
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .expect("acceptance URL must end in a decimal normal-video ID")
        .to_string();

    let root = temp_root("real-browser-acceptance");
    let destination = root.join("download");
    std::fs::create_dir_all(&destination).unwrap();
    let service = DownloadService::spawn(
        prepare_database(&root),
        BrowserOperationCoordinator::default(),
    )
    .await
    .unwrap();
    let response = service
        .resolve(
            PathBuf::from(env!("CARGO_BIN_EXE_douyin-history-archive")).as_path(),
            &browser_data_dir,
            &root.join("acceptance.browser.pid"),
            ResolverRequest {
                request_id: 90_001,
                account_id: 1,
                profile_key: "0123456789abcdef0123456789abcdef".into(),
                aweme_id: aweme_id.clone(),
                canonical_url,
                quality: DownloadQuality::Highest,
            },
        )
        .await
        .expect("authenticated public video resolution failed");
    let manifest = match response {
        ResolverResponse::Resolved {
            request_id: 90_001,
            manifest,
        } => manifest,
        ResolverResponse::Error { code, message, .. } => panic!("resolver error {code}: {message}"),
        _ => panic!("resolver response request ID mismatch"),
    };
    assert_eq!(
        manifest.content_type, "video",
        "fixture must be a normal video"
    );
    let asset = manifest
        .assets
        .into_iter()
        .find(|asset| asset.asset_type == "video")
        .expect("resolved manifest omitted the normal-video asset");
    let source_host = url::Url::parse(&asset.source_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string));
    let source_addresses = match source_host.as_deref() {
        Some(host) => tokio::net::lookup_host((host, 0))
            .await
            .map(|addresses| {
                addresses
                    .map(|address| (address.ip(), PublicInternetPolicy::allows_ip(address.ip())))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let worker = DownloadWorker::builder(Arc::new(PublicInternetPolicy))
        .build()
        .unwrap();
    let completed = worker
        .transfer(
            TransferRequest {
                asset_id: 1,
                aweme_id: aweme_id.clone(),
                stable_key: asset.stable_key.clone(),
                quality: "highest".into(),
                ownership_key: asset_ownership_key(
                    &destination,
                    &aweme_id,
                    &asset.stable_key,
                    "highest",
                ),
                source_url: asset.source_url,
                source_expires_at: asset.source_expires_at,
                request_headers: asset.request_headers,
                destination_root: destination.clone(),
                relative_part_path: PathBuf::from(partial_filename(1, 1, "mp4")),
                relative_final_path: PathBuf::from("video.mp4"),
                expected_size: asset.expected_size,
                expected_etag: None,
                expected_last_modified: None,
                expected_mime: asset.mime_type,
                asset_class: AssetClass::Video,
                job_quota: JobTransferQuota::default(),
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "resolved public video download failed: host={source_host:?}, addresses={source_addresses:?}, error={error:?}"
            )
        });
    assert!(completed.bytes > 0);
    assert!(destination.join("video.mp4").is_file());
}
