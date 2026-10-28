use std::{
    collections::BTreeMap,
    future::Future,
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

use douyin_history_archive::download::{
    asset_ownership_key, partial_filename, AddressPolicy, AssetClass, DownloadError,
    DownloadWorker, JobTransferQuota, PublicInternetPolicy, TransferLimits, TransferRequest,
};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;
use url::Url;

const MP4: &[u8] = b"\0\0\0\x18ftypisom\0\0\0\0isomiso2media-payload-for-worker-tests";

type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, DownloadError>> + Send + 'a>>;

#[derive(Clone)]
struct TestAddressPolicy {
    allowed: SocketAddr,
}

impl AddressPolicy for TestAddressPolicy {
    fn validate_url(&self, url: &Url) -> Result<(), DownloadError> {
        if url.scheme() == "http"
            && url.host_str() == Some("fixture.test")
            && url.port() == Some(self.allowed.port())
        {
            Ok(())
        } else {
            Err(DownloadError {
                code: "download_address_forbidden".into(),
                message: "测试地址不在精确白名单中".into(),
            })
        }
    }

    fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a> {
        Box::pin(async move {
            if host == "fixture.test" {
                Ok(vec![self.allowed])
            } else {
                Err(DownloadError {
                    code: "download_address_forbidden".into(),
                    message: "测试主机不在精确白名单中".into(),
                })
            }
        })
    }
}

#[derive(Debug, Clone)]
struct RequestRecord {
    target: String,
    headers: BTreeMap<String, String>,
}

#[derive(Clone)]
struct ResponseSpec {
    status: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    delay_before_headers: Duration,
    split_at: Option<usize>,
    delay_between_chunks: Duration,
}

struct Fixture {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<RequestRecord>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start(
        handler: impl Fn(RequestRecord, usize) -> ResponseSpec + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let handler = Arc::new(handler);
        let task = tokio::spawn(async move {
            let mut sequence = 0usize;
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                sequence += 1;
                let handler = handler.clone();
                let captured = captured.clone();
                tokio::spawn(async move {
                    let mut stream = stream;
                    let Some(request) = read_request(&mut stream).await else {
                        return;
                    };
                    captured.lock().unwrap().push(request.clone());
                    let response = handler(request, sequence);
                    write_response(&mut stream, response).await;
                });
            }
        });
        Self {
            address,
            requests,
            task,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://fixture.test:{}{path}", self.address.port())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<RequestRecord> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 1024];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") && bytes.len() < 70_000 {
        let read = stream.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    let text = String::from_utf8(bytes).ok()?;
    let mut lines = text.split("\r\n");
    let target = lines.next()?.split_whitespace().nth(1)?.to_string();
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    Some(RequestRecord { target, headers })
}

async fn write_response(stream: &mut TcpStream, spec: ResponseSpec) {
    tokio::time::sleep(spec.delay_before_headers).await;
    let mut head = format!("HTTP/1.1 {}\r\n", spec.status);
    for (name, value) in spec.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(&value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes()).await;
    if let Some(split) = spec.split_at {
        let split = split.min(spec.body.len());
        let _ = stream.write_all(&spec.body[..split]).await;
        tokio::time::sleep(spec.delay_between_chunks).await;
        let _ = stream.write_all(&spec.body[split..]).await;
    } else {
        let _ = stream.write_all(&spec.body).await;
    }
    let _ = stream.shutdown().await;
}

fn response(status: &'static str, content_type: &'static str, body: &[u8]) -> ResponseSpec {
    ResponseSpec {
        status,
        headers: vec![
            ("Content-Type", content_type.into()),
            ("Content-Length", body.len().to_string()),
            ("Connection", "close".into()),
        ],
        body: body.to_vec(),
        delay_before_headers: Duration::ZERO,
        split_at: None,
        delay_between_chunks: Duration::ZERO,
    }
}

struct TestPaths {
    root: PathBuf,
    part: PathBuf,
    final_path: PathBuf,
}

impl TestPaths {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "emily-worker-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        let directory = PathBuf::from("author/work");
        std::fs::create_dir_all(root.join(&directory)).unwrap();
        Self {
            root,
            part: directory.join(partial_filename(7, 42, "mp4")),
            final_path: directory.join("video.mp4"),
        }
    }
}

impl Drop for TestPaths {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn request(paths: &TestPaths, source_url: String) -> TransferRequest {
    TransferRequest {
        asset_id: 7,
        aweme_id: "123".into(),
        stable_key: "video:primary".into(),
        quality: "highest".into(),
        ownership_key: asset_ownership_key(&paths.root, "123", "video:primary", "highest"),
        source_url,
        source_expires_at: None,
        request_headers: BTreeMap::new(),
        destination_root: paths.root.clone(),
        relative_part_path: paths.part.clone(),
        relative_final_path: paths.final_path.clone(),
        expected_size: Some(MP4.len() as u64),
        expected_etag: None,
        expected_last_modified: None,
        expected_mime: Some("video/mp4".into()),
        asset_class: AssetClass::Video,
        job_quota: JobTransferQuota::default(),
    }
}

fn worker(fixture: &Fixture) -> DownloadWorker {
    DownloadWorker::builder(Arc::new(TestAddressPolicy {
        allowed: fixture.address,
    }))
    .request_timeout(Duration::from_millis(300))
    .retry_backoff(Duration::from_millis(5))
    .build()
    .unwrap()
}

#[tokio::test]
async fn streams_valid_media_to_an_owned_part_and_atomically_finalizes_it() {
    let fixture = Fixture::start(|request, _| {
        assert_eq!(request.target, "/full");
        response("200 OK", "video/mp4", MP4)
    })
    .await;
    let paths = TestPaths::new();

    let completed = worker(&fixture)
        .transfer(
            request(&paths, fixture.url("/full")),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(completed.bytes, MP4.len() as u64);
    assert_eq!(completed.sha256, format!("{:x}", Sha256::digest(MP4)));
    assert_eq!(
        std::fs::read(paths.root.join(&paths.final_path)).unwrap(),
        MP4
    );
    assert!(!paths.root.join(&paths.part).exists());
}

#[tokio::test]
async fn authorization_status_is_reported_as_a_refreshable_source_expiry() {
    let fixture = Fixture::start(|_, _| response("403 Forbidden", "text/plain", b"expired")).await;
    let paths = TestPaths::new();
    let error = worker(&fixture)
        .transfer(
            request(&paths, fixture.url("/expired")),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, "download_source_expired");
    assert!(!paths.root.join(&paths.part).exists());
}

#[tokio::test]
async fn resumes_with_range_and_if_range_when_the_validator_is_compatible() {
    let prefix = 24usize;
    let fixture = Fixture::start(move |request, _| {
        assert_eq!(
            request.headers.get("range"),
            Some(&format!("bytes={prefix}-"))
        );
        assert_eq!(
            request.headers.get("if-range").map(String::as_str),
            Some("\"v1\"")
        );
        let mut spec = response("206 Partial Content", "video/mp4", &MP4[prefix..]);
        spec.headers.push((
            "Content-Range",
            format!("bytes {prefix}-{}/{}", MP4.len() - 1, MP4.len()),
        ));
        spec.headers.push(("ETag", "\"v1\"".into()));
        spec
    })
    .await;
    let paths = TestPaths::new();
    std::fs::write(paths.root.join(&paths.part), &MP4[..prefix]).unwrap();
    let mut transfer = request(&paths, fixture.url("/range"));
    transfer.expected_etag = Some("\"v1\"".into());

    let completed = worker(&fixture)
        .transfer(transfer, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(completed.sha256, format!("{:x}", Sha256::digest(MP4)));
    assert_eq!(
        std::fs::read(paths.root.join(&paths.final_path)).unwrap(),
        MP4
    );
}

#[tokio::test]
async fn rejects_partial_response_to_a_request_that_did_not_send_range_before_writing() {
    let body = &MP4[..16];
    let fixture = Fixture::start(move |request, _| {
        assert!(!request.headers.contains_key("range"));
        let mut spec = response("206 Partial Content", "video/mp4", body);
        spec.headers
            .push(("Content-Range", format!("bytes 0-15/{}", body.len())));
        spec
    })
    .await;
    let paths = TestPaths::new();
    let mut transfer = request(&paths, fixture.url("/unsolicited-partial"));
    transfer.expected_size = None;

    let error = worker(&fixture)
        .transfer(transfer, CancellationToken::new())
        .await
        .unwrap_err();

    assert_eq!(error.code, "download_unexpected_partial_response");
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    assert!(!paths.root.join(&paths.part).exists());
    assert!(!paths.root.join(&paths.final_path).exists());
}

#[tokio::test]
async fn ignored_range_discards_the_partial_and_restarts_from_the_full_response() {
    let fixture = Fixture::start(|request, _| {
        assert!(request.headers.contains_key("range"));
        response("200 OK", "video/mp4", MP4)
    })
    .await;
    let paths = TestPaths::new();
    std::fs::write(paths.root.join(&paths.part), &MP4[..20]).unwrap();
    let mut transfer = request(&paths, fixture.url("/ignored"));
    transfer.expected_etag = Some("\"v1\"".into());

    worker(&fixture)
        .transfer(transfer, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(
        std::fs::read(paths.root.join(&paths.final_path)).unwrap(),
        MP4
    );
}

#[tokio::test]
async fn invalid_content_range_discards_the_partial_and_restarts_without_range() {
    let prefix = 20usize;
    let fixture = Fixture::start(move |request, sequence| {
        if sequence == 1 {
            assert!(request.headers.contains_key("range"));
            let mut spec = response("206 Partial Content", "video/mp4", &MP4[prefix..]);
            spec.headers.push((
                "Content-Range",
                format!("bytes {}-{}/{}", prefix + 1, MP4.len() - 1, MP4.len()),
            ));
            spec
        } else {
            assert!(!request.headers.contains_key("range"));
            response("200 OK", "video/mp4", MP4)
        }
    })
    .await;
    let paths = TestPaths::new();
    std::fs::write(paths.root.join(&paths.part), &MP4[..prefix]).unwrap();
    let mut transfer = request(&paths, fixture.url("/invalid-range"));
    transfer.expected_etag = Some("\"v1\"".into());

    worker(&fixture)
        .transfer(transfer, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(fixture.requests.lock().unwrap().len(), 2);
    assert_eq!(
        std::fs::read(paths.root.join(&paths.final_path)).unwrap(),
        MP4
    );
}

#[tokio::test]
async fn changed_etag_discards_the_partial_and_restarts_without_range() {
    let prefix = 20usize;
    let fixture = Fixture::start(move |request, sequence| {
        if sequence == 1 {
            assert!(request.headers.contains_key("range"));
            let mut spec = response("206 Partial Content", "video/mp4", &MP4[prefix..]);
            spec.headers.push((
                "Content-Range",
                format!("bytes {prefix}-{}/{}", MP4.len() - 1, MP4.len()),
            ));
            spec.headers.push(("ETag", "\"v2\"".into()));
            spec
        } else {
            assert!(!request.headers.contains_key("range"));
            let mut spec = response("200 OK", "video/mp4", MP4);
            spec.headers.push(("ETag", "\"v2\"".into()));
            spec
        }
    })
    .await;
    let paths = TestPaths::new();
    std::fs::write(paths.root.join(&paths.part), &MP4[..prefix]).unwrap();
    let mut transfer = request(&paths, fixture.url("/etag"));
    transfer.expected_etag = Some("\"v1\"".into());

    worker(&fixture)
        .transfer(transfer, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(fixture.requests.lock().unwrap().len(), 2);
    assert_eq!(
        std::fs::read(paths.root.join(&paths.final_path)).unwrap(),
        MP4
    );
}

#[tokio::test]
async fn follows_validated_redirects_and_stops_after_five() {
    let fixture = Fixture::start(|request, _| match request.target.as_str() {
        "/redirect" => ResponseSpec {
            status: "302 Found",
            headers: vec![("Location", "/full".into()), ("Content-Length", "0".into())],
            body: Vec::new(),
            delay_before_headers: Duration::ZERO,
            split_at: None,
            delay_between_chunks: Duration::ZERO,
        },
        "/full" => response("200 OK", "video/mp4", MP4),
        "/loop" => ResponseSpec {
            status: "302 Found",
            headers: vec![("Location", "/loop".into()), ("Content-Length", "0".into())],
            body: Vec::new(),
            delay_before_headers: Duration::ZERO,
            split_at: None,
            delay_between_chunks: Duration::ZERO,
        },
        other => panic!("unexpected route: {other}"),
    })
    .await;
    let paths = TestPaths::new();
    worker(&fixture)
        .transfer(
            request(&paths, fixture.url("/redirect")),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(paths.root.join(&paths.final_path)).unwrap(),
        MP4
    );

    let loop_paths = TestPaths::new();
    let error = worker(&fixture)
        .transfer(
            request(&loop_paths, fixture.url("/loop")),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "download_redirect_limit");
    assert!(!loop_paths.root.join(&loop_paths.final_path).exists());
}

#[tokio::test]
async fn rejects_a_redirect_target_before_connecting_when_policy_denies_it() {
    let port = Arc::new(Mutex::new(0u16));
    let target_port = port.clone();
    let fixture = Fixture::start(move |_, _| ResponseSpec {
        status: "302 Found",
        headers: vec![
            (
                "Location",
                format!("http://127.0.0.1:{}/full", *target_port.lock().unwrap()),
            ),
            ("Content-Length", "0".into()),
        ],
        body: Vec::new(),
        delay_before_headers: Duration::ZERO,
        split_at: None,
        delay_between_chunks: Duration::ZERO,
    })
    .await;
    *port.lock().unwrap() = fixture.address.port();
    let paths = TestPaths::new();

    let error = worker(&fixture)
        .transfer(
            request(&paths, fixture.url("/blocked-redirect")),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, "download_address_forbidden");
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    assert!(!paths.root.join(&paths.final_path).exists());
}

#[tokio::test]
async fn html_and_json_challenges_never_become_final_media() {
    for (path, mime, body) in [
        (
            "/html",
            "video/mp4",
            b"<!doctype html><html>challenge</html>".as_slice(),
        ),
        (
            "/json",
            "application/json",
            b"{\"error\":\"challenge\"}".as_slice(),
        ),
    ] {
        let fixture = Fixture::start(move |_, _| response("200 OK", mime, body)).await;
        let paths = TestPaths::new();
        let mut transfer = request(&paths, fixture.url(path));
        transfer.expected_size = Some(body.len() as u64);
        let error = worker(&fixture)
            .transfer(transfer, CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(
            error.code.as_str(),
            "download_mime_invalid" | "download_signature_invalid"
        ));
        assert!(!paths.root.join(&paths.final_path).exists());
    }
}

#[tokio::test]
async fn retries_timeout_twice_then_succeeds_on_the_third_attempt() {
    let fixture = Fixture::start(|_, sequence| {
        let mut spec = response("200 OK", "video/mp4", MP4);
        if sequence < 3 {
            spec.delay_before_headers = Duration::from_millis(500);
        }
        spec
    })
    .await;
    let paths = TestPaths::new();

    worker(&fixture)
        .transfer(
            request(&paths, fixture.url("/timeout")),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(fixture.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn retries_truncated_disconnects_with_ranges_and_caps_attempts_at_three() {
    let fixture = Fixture::start(|request, sequence| {
        let start = request
            .headers
            .get("range")
            .and_then(|value| value.strip_prefix("bytes="))
            .and_then(|value| value.strip_suffix('-'))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let sent = if sequence < 3 { 8 } else { MP4.len() - start };
        let body = &MP4[start..start + sent];
        let mut spec = response(
            if start == 0 {
                "200 OK"
            } else {
                "206 Partial Content"
            },
            "video/mp4",
            body,
        );
        let declared = MP4.len() - start;
        spec.headers
            .iter_mut()
            .find(|(name, _)| *name == "Content-Length")
            .unwrap()
            .1 = declared.to_string();
        spec.headers.push(("ETag", "\"v1\"".into()));
        if start > 0 {
            spec.headers.push((
                "Content-Range",
                format!("bytes {start}-{}/{}", MP4.len() - 1, MP4.len()),
            ));
        }
        spec
    })
    .await;
    let paths = TestPaths::new();
    let mut transfer = request(&paths, fixture.url("/disconnect"));
    transfer.expected_etag = Some("\"v1\"".into());

    worker(&fixture)
        .transfer(transfer, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(fixture.requests.lock().unwrap().len(), 3);
    assert_eq!(
        std::fs::read(paths.root.join(&paths.final_path)).unwrap(),
        MP4
    );
}

#[tokio::test]
async fn disconnect_without_any_validator_restarts_from_zero_instead_of_range_appending() {
    let mut changed = MP4.to_vec();
    for byte in &mut changed[12..] {
        *byte ^= 0x5a;
    }
    let changed = Arc::new(changed);
    let served = changed.clone();
    let fixture = Fixture::start(move |request, sequence| {
        if sequence == 1 {
            assert!(!request.headers.contains_key("range"));
            let mut spec = response("200 OK", "video/mp4", &MP4[..20]);
            spec.headers
                .iter_mut()
                .find(|(name, _)| *name == "Content-Length")
                .unwrap()
                .1 = MP4.len().to_string();
            return spec;
        }
        if let Some(range) = request.headers.get("range") {
            let start = range
                .strip_prefix("bytes=")
                .and_then(|value| value.strip_suffix('-'))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap();
            let mut spec = response("206 Partial Content", "video/mp4", &served[start..]);
            spec.headers.push((
                "Content-Range",
                format!("bytes {start}-{}/{}", served.len() - 1, served.len()),
            ));
            spec
        } else {
            response("200 OK", "video/mp4", &served)
        }
    })
    .await;
    let paths = TestPaths::new();

    worker(&fixture)
        .transfer(
            request(&paths, fixture.url("/unvalidated-disconnect")),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].headers.contains_key("range"));
    assert_eq!(
        std::fs::read(paths.root.join(&paths.final_path)).unwrap(),
        *changed
    );
}

#[tokio::test]
async fn response_etag_discovered_before_disconnect_enables_safe_range_retry() {
    let prefix = 20usize;
    let fixture = Fixture::start(move |request, sequence| {
        if sequence == 1 {
            assert!(!request.headers.contains_key("range"));
            let mut spec = response("200 OK", "video/mp4", &MP4[..prefix]);
            spec.headers
                .iter_mut()
                .find(|(name, _)| *name == "Content-Length")
                .unwrap()
                .1 = MP4.len().to_string();
            spec.headers.push(("ETag", "\"discovered-v1\"".into()));
            return spec;
        }
        if request.headers.contains_key("range") {
            let mut spec = response("206 Partial Content", "video/mp4", &MP4[prefix..]);
            spec.headers.push((
                "Content-Range",
                format!("bytes {prefix}-{}/{}", MP4.len() - 1, MP4.len()),
            ));
            spec.headers.push(("ETag", "\"discovered-v1\"".into()));
            spec
        } else {
            response("200 OK", "video/mp4", MP4)
        }
    })
    .await;
    let paths = TestPaths::new();

    worker(&fixture)
        .transfer(
            request(&paths, fixture.url("/discovered-validator")),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].headers.get("range").map(String::as_str),
        Some("bytes=20-")
    );
    assert_eq!(
        requests[1].headers.get("if-range").map(String::as_str),
        Some("\"discovered-v1\"")
    );
    assert_eq!(
        std::fs::read(paths.root.join(&paths.final_path)).unwrap(),
        MP4
    );
}

#[tokio::test]
async fn cancellation_flushes_and_retains_the_owned_partial() {
    let fixture = Fixture::start(|_, _| {
        let mut spec = response("200 OK", "video/mp4", MP4);
        spec.split_at = Some(24);
        spec.delay_between_chunks = Duration::from_secs(2);
        spec
    })
    .await;
    let paths = TestPaths::new();
    let token = CancellationToken::new();
    let cancel = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
    });

    let error = worker(&fixture)
        .transfer(request(&paths, fixture.url("/cancel")), token)
        .await
        .unwrap_err();

    assert_eq!(error.code, "download_cancelled");
    let part = std::fs::read(paths.root.join(&paths.part)).unwrap();
    assert!(!part.is_empty());
    assert!(MP4.starts_with(&part));
    assert!(!paths.root.join(&paths.final_path).exists());
}

#[tokio::test]
async fn accepts_exactly_64k_of_response_head_and_rejects_one_byte_more_before_writing() {
    let base = 3 + (12 + 9 + 4) + (14 + MP4.len().to_string().len() + 4) + (10 + 5 + 4) + (5 + 4);
    let boundary_pad = 64 * 1024 - base;
    for (extra, should_pass) in [(0usize, true), (1usize, false)] {
        let pad = "x".repeat(boundary_pad + extra);
        let fixture = Fixture::start(move |_, _| {
            let mut spec = response("200 OK", "video/mp4", MP4);
            spec.headers.push(("X-Pad", pad.clone()));
            spec
        })
        .await;
        let paths = TestPaths::new();
        let result = worker(&fixture)
            .transfer(
                request(&paths, fixture.url("/headers")),
                CancellationToken::new(),
            )
            .await;
        if should_pass {
            result.unwrap();
            assert!(paths.root.join(&paths.final_path).exists());
        } else {
            assert_eq!(result.unwrap_err().code, "download_headers_too_large");
            assert!(!paths.root.join(&paths.part).exists());
            assert!(!paths.root.join(&paths.final_path).exists());
        }
    }
}

#[tokio::test]
async fn asset_and_job_limits_stop_before_excess_bytes_are_written() {
    for (limits, expected_part_len) in [
        (
            TransferLimits {
                max_asset_bytes: (MP4.len() - 1) as u64,
                max_job_bytes: u64::MAX,
            },
            None,
        ),
        (
            TransferLimits {
                max_asset_bytes: u64::MAX,
                max_job_bytes: (MP4.len() - 1) as u64,
            },
            Some(0),
        ),
    ] {
        let fixture = Fixture::start(|_, _| response("200 OK", "video/mp4", MP4)).await;
        let paths = TestPaths::new();
        let limited = DownloadWorker::builder(Arc::new(TestAddressPolicy {
            allowed: fixture.address,
        }))
        .limits(limits)
        .build()
        .unwrap();

        let error = limited
            .transfer(
                request(&paths, fixture.url("/limit")),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();

        assert_eq!(error.code, "download_capacity_exceeded");
        assert!(!paths.root.join(&paths.final_path).exists());
        assert_eq!(
            std::fs::metadata(paths.root.join(&paths.part))
                .ok()
                .map(|metadata| metadata.len()),
            expected_part_len,
        );
    }
}

#[tokio::test]
async fn job_limit_is_shared_within_one_job_but_resets_for_the_next_job() {
    let fixture = Fixture::start(|_, _| response("200 OK", "video/mp4", MP4)).await;
    let limited = DownloadWorker::builder(Arc::new(TestAddressPolicy {
        allowed: fixture.address,
    }))
    .limits(TransferLimits {
        max_asset_bytes: u64::MAX,
        max_job_bytes: MP4.len() as u64,
    })
    .build()
    .unwrap();

    let first_job = JobTransferQuota::default();
    let first_paths = TestPaths::new();
    let mut first_request = request(&first_paths, fixture.url("/job-one-first"));
    first_request.job_quota = first_job.clone();
    limited
        .transfer(first_request, CancellationToken::new())
        .await
        .unwrap();

    let second_paths = TestPaths::new();
    let mut second_request = request(&second_paths, fixture.url("/job-one-second"));
    second_request.job_quota = first_job;
    assert_eq!(
        limited
            .transfer(second_request, CancellationToken::new())
            .await
            .unwrap_err()
            .code,
        "download_capacity_exceeded"
    );

    let next_job_paths = TestPaths::new();
    let mut next_job_request = request(&next_job_paths, fixture.url("/job-two"));
    next_job_request.job_quota = JobTransferQuota::default();
    limited
        .transfer(next_job_request, CancellationToken::new())
        .await
        .unwrap();
}

#[test]
fn public_policy_rejects_reserved_a_aaaa_and_ipv4_mapped_ipv6() {
    for address in [
        "127.0.0.1",
        "10.0.0.1",
        "169.254.1.1",
        "224.0.0.1",
        "0.0.0.0",
        "::1",
        "fe80::1",
        "fc00::1",
        "ff02::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
    ] {
        assert!(
            !PublicInternetPolicy::allows_ip(address.parse().unwrap()),
            "{address}"
        );
    }
    assert!(PublicInternetPolicy::allows_ip("1.1.1.1".parse().unwrap()));
    assert!(PublicInternetPolicy::allows_ip(
        "2606:4700:4700::1111".parse().unwrap()
    ));
}

#[test]
fn public_policy_allows_proxy_fake_ip_only_for_trusted_douyin_media_hosts() {
    let fake_ip = "198.18.0.109".parse().unwrap();
    assert!(!PublicInternetPolicy::allows_ip(fake_ip));
    for host in [
        "v26-web.douyinvod.com",
        "p3-sign.douyinpic.com",
        "tos-cn-i-0813c001.byteimg.com",
    ] {
        assert!(PublicInternetPolicy::allows_resolved_ip(host, fake_ip));
    }
    for host in [
        "example.com",
        "douyinvod.com.example.com",
        "notdouyinpic.com",
    ] {
        assert!(!PublicInternetPolicy::allows_resolved_ip(host, fake_ip));
    }
    assert!(PublicInternetPolicy
        .validate_url(&Url::parse("https://198.18.0.109/video.mp4").unwrap())
        .is_err());
}

#[test]
fn fixture_policy_rejects_any_other_port() {
    let policy = TestAddressPolicy {
        allowed: "127.0.0.1:12345".parse().unwrap(),
    };
    assert!(policy
        .validate_url(&Url::parse("http://fixture.test:12346/full").unwrap())
        .is_err());
}
