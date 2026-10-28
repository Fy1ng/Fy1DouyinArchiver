use crate::collector::{
    browser_process::BrowserProcessRecordV2, browser_runtime::resolve_download_browser,
    cdp::CdpPage, file_store::write_json_atomically, session::launch_download_browser_session,
};
use chrono::{SecondsFormat, Utc};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    parse_detail_response, read_frame, write_frame, DownloadError, ResolverControl,
    ResolverRequest, ResolverResponse,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolverChildOptions {
    pub browser_data_dir: PathBuf,
    pub browser_pid_file: PathBuf,
    pub browser_debug_url: Option<String>,
    pub browser_debug_port: Option<u16>,
}

pub fn parse_resolver_child_args(argv: Vec<OsString>) -> Result<ResolverChildOptions, String> {
    let mut browser_data_dir = None;
    let mut browser_pid_file = None;
    let mut browser_debug_url = None;
    let mut browser_debug_port = None;
    let mut index = 0;
    while index < argv.len() {
        let key = argv[index]
            .to_str()
            .ok_or_else(|| "resolver_invalid_arguments".to_string())?;
        let value = argv
            .get(index + 1)
            .and_then(|value| value.to_str())
            .ok_or_else(|| "resolver_invalid_arguments".to_string())?;
        match key {
            "--browser-data-dir" => browser_data_dir = Some(PathBuf::from(value)),
            "--browser-pid-file" => browser_pid_file = Some(PathBuf::from(value)),
            "--browser-debug-url" => browser_debug_url = Some(value.to_string()),
            "--browser-debug-port" => {
                browser_debug_port = value.parse::<u16>().ok().filter(|port| *port > 0)
            }
            _ => return Err("resolver_invalid_arguments".to_string()),
        }
        index += 2;
    }
    if browser_debug_url.is_some() != browser_debug_port.is_some() {
        return Err("resolver_invalid_arguments".to_string());
    }
    if let Some(debug_url) = &browser_debug_url {
        let parsed =
            url::Url::parse(debug_url).map_err(|_| "resolver_invalid_arguments".to_string())?;
        let loopback = parsed.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
        if parsed.scheme() != "ws" || !loopback {
            return Err("resolver_invalid_arguments".to_string());
        }
    }
    Ok(ResolverChildOptions {
        browser_data_dir: browser_data_dir
            .ok_or_else(|| "resolver_invalid_arguments".to_string())?,
        browser_pid_file: browser_pid_file
            .ok_or_else(|| "resolver_invalid_arguments".to_string())?,
        browser_debug_url,
        browser_debug_port,
    })
}

pub async fn run_resolver_child(options: ResolverChildOptions) -> Result<(), String> {
    let request = read_frame::<_, ResolverRequest>(&mut io::stdin()).map_err(|error| error.code)?;
    let (control_tx, mut control_rx) = mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("resolver-control".into())
        .spawn(move || {
            let mut input = io::stdin();
            while let Ok(control) = read_frame::<_, ResolverControl>(&mut input) {
                if control_tx.send(control).is_err() {
                    break;
                }
            }
        })
        .map_err(|_| "resolver_control_failed".to_string())?;

    let request_id = request.request_id;
    let cancellation = CancellationToken::new();
    let cancel_signal = cancellation.clone();
    tokio::spawn(async move {
        match control_rx.recv().await {
            Some(ResolverControl::Cancel | ResolverControl::Shutdown) | None => {
                cancel_signal.cancel()
            }
        }
    });
    let outcome = resolve_request(&options, request, cancellation).await;
    let response = match outcome {
        Ok(manifest) => ResolverResponse::Resolved {
            request_id,
            manifest,
        },
        Err(error) => ResolverResponse::Error {
            request_id,
            code: error.code,
            message: error.message,
        },
    };
    let mut output = io::stdout();
    let write = write_frame(&mut output, &response)
        .and_then(|_| {
            output
                .flush()
                .map_err(|_| adapter_error("resolver_frame_io_failed"))
        })
        .map_err(|error| error.code);
    let _ = fs::remove_file(&options.browser_pid_file);
    #[cfg(debug_assertions)]
    if write.is_ok() {
        if let Some(marker) = std::env::var_os("EMILY_TEST_RESOLVER_HANG_AFTER_FRAME_PID_FILE") {
            let _ = fs::write(marker, std::process::id().to_string());
            std::thread::sleep(Duration::from_secs(60));
        }
    }
    write
}

async fn resolve_request(
    options: &ResolverChildOptions,
    request: ResolverRequest,
    cancellation: CancellationToken,
) -> Result<super::ResolvedMediaManifest, DownloadError> {
    if let (Some(browser_url), Some(port)) =
        (&options.browser_debug_url, options.browser_debug_port)
    {
        let page = CdpPage::connect(browser_url, port, true)
            .await
            .map_err(adapter_error)?;
        let result = tokio::select! {
            result = resolve_on_page(&page, &request) => result,
            _ = cancellation.cancelled() => Err(cancelled()),
        };
        page.close().await;
        return result;
    }

    let choice = resolve_download_browser(&options.browser_data_dir).map_err(adapter_error)?;
    let session = launch_download_browser_session(
        &choice.executable_path,
        &choice.profile_path,
        choice.browser_id,
    )
    .await
    .map_err(adapter_error)?;
    let record = BrowserProcessRecordV2 {
        schema_version: 2,
        pid: session.process.pid,
        creation_time_100ns: session.process.creation_time_100ns.clone(),
        browser_id: choice.browser_id,
        executable_path: choice.executable_path.to_string_lossy().to_string(),
        profile_path: choice.profile_path.to_string_lossy().to_string(),
        user_data_dir_argument: session.user_data_dir_argument.clone(),
        devtools_port: session.port,
        browser_websocket_path: session.browser_websocket_path.clone(),
        started_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    };
    write_json_atomically(&options.browser_pid_file, &record).map_err(adapter_error)?;
    let result = tokio::select! {
        result = resolve_on_page(&session.page, &request) => result,
        _ = cancellation.cancelled() => Err(cancelled()),
    };
    session.page.close().await;
    result
}

async fn resolve_on_page(
    page: &CdpPage,
    request: &ResolverRequest,
) -> Result<super::ResolvedMediaManifest, DownloadError> {
    page.enable_network().await.map_err(adapter_error)?;
    page.block_resources(false, true)
        .await
        .map_err(adapter_error)?;
    page.navigate(&request.canonical_url)
        .await
        .map_err(adapter_error)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut last_candidate_error = None;
    loop {
        let response = tokio::time::timeout_at(deadline, page.next_network_response())
            .await
            .map_err(|_| {
                last_candidate_error
                    .clone()
                    .unwrap_or_else(|| adapter_error("resolver_detail_timeout"))
            })?
            .map_err(adapter_error)?;
        if !response.is_json_detail_for(&request.aweme_id) {
            continue;
        }
        let body = match tokio::time::timeout_at(deadline, page.response_body(&response.request_id))
            .await
        {
            Ok(Ok(body)) => body,
            Ok(Err(error)) => {
                last_candidate_error = Some(adapter_error(error));
                continue;
            }
            Err(_) => {
                return Err(last_candidate_error
                    .clone()
                    .unwrap_or_else(|| adapter_error("resolver_detail_timeout")))
            }
        };
        if body.len() > super::MAX_RESOLVER_FRAME_SIZE {
            last_candidate_error = Some(adapter_error("download_manifest_too_large"));
            continue;
        }
        let value: Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(_) => {
                last_candidate_error = Some(adapter_error("download_manifest_invalid"));
                continue;
            }
        };
        match parse_detail_response(value, &request.aweme_id, request.quality) {
            Ok(mut manifest) => {
                let user_agent: String = page
                    .evaluate("navigator.userAgent")
                    .await
                    .map_err(adapter_error)?;
                let headers = browser_request_headers(&user_agent)?;
                for asset in &mut manifest.assets {
                    for (name, value) in &headers {
                        asset
                            .request_headers
                            .entry(name.clone())
                            .or_insert_with(|| value.clone());
                    }
                }
                return Ok(manifest);
            }
            Err(error) => last_candidate_error = Some(error),
        }
    }
}

fn browser_request_headers(user_agent: &str) -> Result<BTreeMap<String, String>, DownloadError> {
    let user_agent = user_agent.trim();
    if user_agent.is_empty() || user_agent.contains(['\r', '\n']) {
        return Err(adapter_error("resolver_browser_identity_invalid"));
    }
    Ok(BTreeMap::from([
        ("Referer".to_string(), "https://www.douyin.com/".to_string()),
        ("User-Agent".to_string(), user_agent.to_string()),
    ]))
}

fn adapter_error(code: impl ToString) -> DownloadError {
    let code = code.to_string();
    DownloadError {
        code,
        message: "媒体解析失败".to_string(),
    }
}

fn cancelled() -> DownloadError {
    DownloadError {
        code: "download_cancelled".into(),
        message: "下载已取消".into(),
    }
}
