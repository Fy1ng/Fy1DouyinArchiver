use std::{
    collections::BTreeMap,
    error::Error as StdError,
    future::Future,
    io::{Read, Seek, SeekFrom, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use futures_util::StreamExt;
use reqwest::{
    dns::{Addrs, Name, Resolve, Resolving},
    header::{
        HeaderMap, HeaderName, HeaderValue, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG,
        IF_RANGE, LAST_MODIFIED, RANGE,
    },
    redirect, Client, StatusCode,
};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{
    asset_ownership_key, DownloadError, FileObservation, FilesystemRootAccess, RootAccess,
};

const MAX_RESPONSE_HEAD_BYTES: usize = 64 * 1024;
const DEFAULT_MAX_ASSET_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const DEFAULT_MAX_JOB_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;
const MAX_ATTEMPTS: usize = 3;

pub type AddressResolution<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, DownloadError>> + Send + 'a>>;

pub trait AddressPolicy: Send + Sync {
    fn validate_url(&self, url: &Url) -> Result<(), DownloadError>;
    fn resolve<'a>(&'a self, host: &'a str) -> AddressResolution<'a>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PublicInternetPolicy;

impl PublicInternetPolicy {
    pub fn allows_ip(address: IpAddr) -> bool {
        match address {
            IpAddr::V4(address) => public_ipv4(address),
            IpAddr::V6(address) => address
                .to_ipv4_mapped()
                .map(public_ipv4)
                .unwrap_or_else(|| public_ipv6(address)),
        }
    }

    pub fn allows_resolved_ip(host: &str, address: IpAddr) -> bool {
        Self::allows_ip(address) || (trusted_douyin_media_host(host) && fake_proxy_address(address))
    }
}

impl AddressPolicy for PublicInternetPolicy {
    fn validate_url(&self, url: &Url) -> Result<(), DownloadError> {
        if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
            return Err(worker_error(
                "download_address_forbidden",
                "下载地址必须是安全的 HTTPS 公网地址",
            ));
        }
        if let Some(host) = url.host() {
            let literal = match host {
                url::Host::Ipv4(address) => Some(IpAddr::V4(address)),
                url::Host::Ipv6(address) => Some(IpAddr::V6(address)),
                url::Host::Domain(_) => None,
            };
            if literal.is_some_and(|address| !Self::allows_ip(address)) {
                return Err(worker_error(
                    "download_address_forbidden",
                    "下载地址解析到受限网络",
                ));
            }
            Ok(())
        } else {
            Err(worker_error(
                "download_address_forbidden",
                "下载地址缺少主机名",
            ))
        }
    }

    fn resolve<'a>(&'a self, host: &'a str) -> AddressResolution<'a> {
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host, 0))
                .await
                .map_err(|_| worker_error("download_network_failed", "无法解析下载地址"))?;
            let allowed: Vec<_> = addresses
                .filter(|address| Self::allows_resolved_ip(host, address.ip()))
                .collect();
            if allowed.is_empty() {
                Err(worker_error(
                    "download_address_forbidden",
                    "下载地址解析到受限网络",
                ))
            } else {
                Ok(allowed)
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetClass {
    Video,
    Image,
    Audio,
}

#[derive(Debug, Clone, Default)]
pub struct JobTransferQuota {
    written: Arc<AtomicU64>,
}

impl JobTransferQuota {
    fn reserve(&self, bytes: u64, limit: u64) -> Result<(), DownloadError> {
        reserve_job_bytes(&self.written, bytes, limit)
    }

    fn release(&self, bytes: u64) {
        self.written.fetch_sub(bytes, Ordering::AcqRel);
    }
}

#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub asset_id: i64,
    pub aweme_id: String,
    pub stable_key: String,
    pub quality: String,
    pub ownership_key: String,
    pub source_url: String,
    pub source_expires_at: Option<String>,
    pub request_headers: BTreeMap<String, String>,
    pub destination_root: PathBuf,
    pub relative_part_path: PathBuf,
    pub relative_final_path: PathBuf,
    pub expected_size: Option<u64>,
    pub expected_etag: Option<String>,
    pub expected_last_modified: Option<String>,
    pub expected_mime: Option<String>,
    pub asset_class: AssetClass,
    pub job_quota: JobTransferQuota,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedAsset {
    pub bytes: u64,
    pub sha256: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub relative_path: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub struct TransferLimits {
    pub max_asset_bytes: u64,
    pub max_job_bytes: u64,
}

impl Default for TransferLimits {
    fn default() -> Self {
        Self {
            max_asset_bytes: DEFAULT_MAX_ASSET_BYTES,
            max_job_bytes: DEFAULT_MAX_JOB_BYTES,
        }
    }
}

pub struct DownloadWorkerBuilder {
    policy: Arc<dyn AddressPolicy>,
    request_timeout: Duration,
    retry_backoff: Duration,
    limits: TransferLimits,
}

impl DownloadWorkerBuilder {
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn retry_backoff(mut self, backoff: Duration) -> Self {
        self.retry_backoff = backoff;
        self
    }

    pub fn limits(mut self, limits: TransferLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn build(self) -> Result<DownloadWorker, DownloadError> {
        let redirect_policy = self.policy.clone();
        let redirects = redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("download_redirect_limit");
            }
            match redirect_policy.validate_url(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(error) => attempt.error(error),
            }
        });
        let resolver = Arc::new(PolicyResolver {
            policy: self.policy.clone(),
        });
        let client = Client::builder()
            .no_proxy()
            .http1_only()
            .redirect(redirects)
            .dns_resolver(resolver)
            .build()
            .map_err(|_| worker_error("download_worker_unavailable", "无法创建下载客户端"))?;
        Ok(DownloadWorker {
            client,
            policy: self.policy,
            roots: FilesystemRootAccess,
            request_timeout: self.request_timeout,
            retry_backoff: self.retry_backoff,
            limits: self.limits,
        })
    }
}

pub struct DownloadWorker {
    client: Client,
    policy: Arc<dyn AddressPolicy>,
    roots: FilesystemRootAccess,
    request_timeout: Duration,
    retry_backoff: Duration,
    limits: TransferLimits,
}

impl DownloadWorker {
    pub fn builder(policy: Arc<dyn AddressPolicy>) -> DownloadWorkerBuilder {
        DownloadWorkerBuilder {
            policy,
            request_timeout: Duration::from_secs(30),
            retry_backoff: Duration::from_millis(250),
            limits: TransferLimits::default(),
        }
    }

    pub async fn transfer(
        &self,
        request: TransferRequest,
        cancellation: CancellationToken,
    ) -> Result<CompletedAsset, DownloadError> {
        validate_transfer_request(&request)?;
        let url = Url::parse(&request.source_url)
            .map_err(|_| worker_error("download_address_invalid", "下载地址格式无效"))?;
        self.policy.validate_url(&url)?;
        let mut last_error = None;
        let mut retry_validator = RetryValidator::from_request(&request);
        for attempt in 0..MAX_ATTEMPTS {
            match self
                .attempt(
                    &request,
                    url.clone(),
                    &cancellation,
                    retry_validator.as_ref(),
                )
                .await
            {
                Ok(asset) => return Ok(asset),
                Err(mut failure) => {
                    if let Some(update) = failure.validator_update.take() {
                        retry_validator = update;
                    }
                    if failure.retryable && attempt + 1 < MAX_ATTEMPTS {
                        last_error = Some(failure.error);
                        tokio::select! {
                            _ = cancellation.cancelled() => return Err(cancelled_error()),
                            _ = tokio::time::sleep(self.retry_backoff.saturating_mul(1 << attempt)) => {}
                        }
                    } else {
                        return Err(failure.error);
                    }
                }
            }
        }
        Err(last_error
            .unwrap_or_else(|| worker_error("download_network_failed", "下载网络请求失败")))
    }

    async fn attempt(
        &self,
        request: &TransferRequest,
        url: Url,
        cancellation: &CancellationToken,
        retry_validator: Option<&RetryValidator>,
    ) -> Result<CompletedAsset, AttemptFailure> {
        if cancellation.is_cancelled() {
            return Err(AttemptFailure::permanent(cancelled_error()));
        }
        let mut part_guard = self
            .roots
            .guard_for_create(&request.destination_root, &request.relative_part_path)
            .map_err(AttemptFailure::permanent)?;
        let mut existing = match part_guard.inspect().map_err(AttemptFailure::permanent)? {
            FileObservation::Missing => 0,
            FileObservation::File { len } => len,
            FileObservation::NotFile => {
                return Err(AttemptFailure::permanent(worker_error(
                    "download_path_unsafe",
                    "下载临时路径不是普通文件",
                )))
            }
        };
        if existing > 0 && retry_validator.is_none() {
            part_guard.remove().map_err(AttemptFailure::permanent)?;
            drop(part_guard);
            part_guard = self
                .roots
                .guard_for_create(&request.destination_root, &request.relative_part_path)
                .map_err(AttemptFailure::permanent)?;
            existing = 0;
        }
        if existing > self.limits.max_asset_bytes {
            return Err(AttemptFailure::permanent(capacity_error()));
        }

        let mut builder = self.client.get(url).timeout(self.request_timeout);
        for (name, value) in &request.request_headers {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                AttemptFailure::permanent(worker_error(
                    "download_request_invalid",
                    "下载请求头无效",
                ))
            })?;
            if matches!(name, RANGE | IF_RANGE | CONTENT_LENGTH) {
                return Err(AttemptFailure::permanent(worker_error(
                    "download_request_invalid",
                    "下载请求头包含受保护字段",
                )));
            }
            let value = HeaderValue::from_str(value).map_err(|_| {
                AttemptFailure::permanent(worker_error(
                    "download_request_invalid",
                    "下载请求头无效",
                ))
            })?;
            builder = builder.header(name, value);
        }
        if existing > 0 {
            builder = builder.header(RANGE, format!("bytes={existing}-"));
            if let Some(validator) = retry_validator {
                builder = builder.header(IF_RANGE, validator.value());
            }
        }

        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(AttemptFailure::permanent(cancelled_error())),
            response = builder.send() => response.map_err(request_failure)?,
        };
        validate_response_head(response.status(), response.headers())
            .map_err(AttemptFailure::permanent)?;
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(AttemptFailure::permanent(worker_error(
                "download_source_expired",
                "下载地址已过期，需要重新解析",
            )));
        }
        if status != StatusCode::OK && status != StatusCode::PARTIAL_CONTENT {
            return Err(AttemptFailure::permanent(worker_error(
                "download_response_status",
                "下载服务器返回了无效状态",
            )));
        }
        if existing == 0 && status == StatusCode::PARTIAL_CONTENT {
            return Err(AttemptFailure::permanent(worker_error(
                "download_unexpected_partial_response",
                "下载服务器返回了非预期的部分响应",
            )));
        }
        let response_validator = RetryValidator::from_headers(response.headers());
        validate_mime(response.headers(), request).map_err(AttemptFailure::permanent)?;

        let mut append = existing > 0 && status == StatusCode::PARTIAL_CONTENT;
        if append {
            if let Err(error) =
                validate_content_range(response.headers(), existing, request.expected_size)
            {
                part_guard.remove().map_err(AttemptFailure::permanent)?;
                drop(part_guard);
                return Err(
                    AttemptFailure::retryable(error).with_validator_update(response_validator)
                );
            }
            let validator = retry_validator.expect("partial response requires retry validator");
            if let Err(error) = validate_validator(response.headers(), validator) {
                part_guard.remove().map_err(AttemptFailure::permanent)?;
                drop(part_guard);
                return Err(
                    AttemptFailure::retryable(error).with_validator_update(response_validator)
                );
            }
        } else if existing > 0 {
            part_guard.remove().map_err(AttemptFailure::permanent)?;
            drop(part_guard);
            part_guard = self
                .roots
                .guard_for_create(&request.destination_root, &request.relative_part_path)
                .map_err(AttemptFailure::permanent)?;
            append = false;
        }
        let initial = if append { existing } else { 0 };
        let declared = response.content_length();
        if initial
            .checked_add(declared.unwrap_or(0))
            .is_none_or(|total| total > self.limits.max_asset_bytes)
        {
            return Err(AttemptFailure::permanent(capacity_error()));
        }

        let mut signature = Vec::with_capacity(32);
        let mut digest = Sha256::new();
        let mut file: Box<dyn Write + Send + '_> = if append {
            let mut file = part_guard
                .open_existing_for_resume()
                .map_err(AttemptFailure::permanent)?;
            let mut prefix = [0u8; 64 * 1024];
            let mut prefix_len = 0u64;
            loop {
                let read = file.read(&mut prefix).map_err(|_| {
                    AttemptFailure::permanent(worker_error(
                        "download_path_unsafe",
                        "无法读取下载临时文件",
                    ))
                })?;
                if read == 0 {
                    break;
                }
                if signature.len() < 32 {
                    let take = (32 - signature.len()).min(read);
                    signature.extend_from_slice(&prefix[..take]);
                }
                digest.update(&prefix[..read]);
                prefix_len = prefix_len
                    .checked_add(read as u64)
                    .ok_or_else(|| AttemptFailure::permanent(capacity_error()))?;
            }
            if prefix_len != existing {
                return Err(AttemptFailure::permanent(worker_error(
                    "download_path_unsafe",
                    "下载临时文件大小在续传前发生变化",
                )));
            }
            file.seek(SeekFrom::End(0)).map_err(|_| {
                AttemptFailure::permanent(worker_error(
                    "download_path_unsafe",
                    "无法定位下载临时文件",
                ))
            })?;
            file
        } else {
            part_guard.create_new().map_err(AttemptFailure::permanent)?
        };

        let headers = response.headers().clone();
        let mut stream = response.bytes_stream();
        let mut total = initial;
        let mut received = 0u64;
        loop {
            let next = tokio::select! {
                _ = cancellation.cancelled() => {
                    file.flush().ok();
                    return Err(AttemptFailure::permanent(cancelled_error()));
                }
                next = stream.next() => next,
            };
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|_| {
                AttemptFailure::retryable(worker_error("download_network_failed", "下载连接中断"))
                    .with_validator_update(response_validator.clone())
            })?;
            let next_total = total
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| AttemptFailure::permanent(capacity_error()))?;
            if next_total > self.limits.max_asset_bytes {
                return Err(AttemptFailure::permanent(capacity_error()));
            }
            request
                .job_quota
                .reserve(chunk.len() as u64, self.limits.max_job_bytes)
                .map_err(AttemptFailure::permanent)?;
            if signature.len() < 32 {
                let take = (32 - signature.len()).min(chunk.len());
                signature.extend_from_slice(&chunk[..take]);
            }
            digest.update(&chunk);
            if file.write_all(&chunk).is_err() {
                request.job_quota.release(chunk.len() as u64);
                return Err(AttemptFailure::permanent(worker_error(
                    "download_write_failed",
                    "写入下载文件失败",
                )));
            }
            total = next_total;
            received += chunk.len() as u64;
        }
        if declared.is_some_and(|expected| expected != received)
            || request
                .expected_size
                .is_some_and(|expected| expected != total)
            || total == 0
        {
            return Err(AttemptFailure::retryable(worker_error(
                "download_length_invalid",
                "下载文件长度不完整",
            ))
            .with_validator_update(response_validator));
        }
        validate_signature(&signature, request.asset_class).map_err(AttemptFailure::permanent)?;
        file.flush().map_err(|_| {
            AttemptFailure::permanent(worker_error("download_write_failed", "刷新下载文件失败"))
        })?;
        drop(file);
        drop(part_guard);
        self.roots
            .atomic_rename(
                &request.destination_root,
                &request.relative_part_path,
                &request.relative_final_path,
            )
            .map_err(AttemptFailure::permanent)?;
        Ok(CompletedAsset {
            bytes: total,
            sha256: format!("{:x}", digest.finalize()),
            etag: header_text(&headers, ETAG),
            last_modified: header_text(&headers, LAST_MODIFIED),
            relative_path: request.relative_final_path.clone(),
        })
    }
}

struct PolicyResolver {
    policy: Arc<dyn AddressPolicy>,
}

impl Resolve for PolicyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.policy.clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addresses = policy.resolve(&host).await?;
            let addresses: Addrs = Box::new(addresses.into_iter());
            Ok(addresses)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RetryValidator {
    Etag(String),
    LastModified(String),
}

impl RetryValidator {
    fn from_request(request: &TransferRequest) -> Option<Self> {
        request.expected_etag.clone().map(Self::Etag).or_else(|| {
            request
                .expected_last_modified
                .clone()
                .map(Self::LastModified)
        })
    }

    fn from_headers(headers: &HeaderMap) -> Option<Self> {
        header_text(headers, ETAG)
            .map(Self::Etag)
            .or_else(|| header_text(headers, LAST_MODIFIED).map(Self::LastModified))
    }

    fn value(&self) -> &str {
        match self {
            Self::Etag(value) | Self::LastModified(value) => value,
        }
    }
}

struct AttemptFailure {
    error: DownloadError,
    retryable: bool,
    validator_update: Option<Option<RetryValidator>>,
}

impl AttemptFailure {
    fn permanent(error: DownloadError) -> Self {
        Self {
            error,
            retryable: false,
            validator_update: None,
        }
    }

    fn retryable(error: DownloadError) -> Self {
        Self {
            error,
            retryable: true,
            validator_update: None,
        }
    }

    fn with_validator_update(mut self, validator: Option<RetryValidator>) -> Self {
        self.validator_update = Some(validator);
        self
    }
}

fn validate_transfer_request(request: &TransferRequest) -> Result<(), DownloadError> {
    let expected = asset_ownership_key(
        &request.destination_root,
        &request.aweme_id,
        &request.stable_key,
        &request.quality,
    );
    let owned_name = request
        .relative_part_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with(&format!(".asset-{}-", request.asset_id)) && name.ends_with(".part")
        });
    if request.ownership_key != expected || !owned_name {
        return Err(worker_error(
            "download_part_not_owned",
            "下载临时文件不属于当前资源",
        ));
    }
    Ok(())
}

fn validate_response_head(status: StatusCode, headers: &HeaderMap) -> Result<(), DownloadError> {
    let mut size = status.as_str().len();
    for (name, value) in headers {
        size = size
            .checked_add(name.as_str().len() + value.as_bytes().len() + 4)
            .ok_or_else(header_limit_error)?;
        if size > MAX_RESPONSE_HEAD_BYTES {
            return Err(header_limit_error());
        }
    }
    Ok(())
}

fn validate_mime(headers: &HeaderMap, request: &TransferRequest) -> Result<(), DownloadError> {
    let actual = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let class_matches = match request.asset_class {
        AssetClass::Video => actual.starts_with("video/"),
        AssetClass::Image => actual.starts_with("image/"),
        AssetClass::Audio => actual.starts_with("audio/"),
    };
    let expected_matches = request
        .expected_mime
        .as_ref()
        .map(|expected| actual == expected.to_ascii_lowercase())
        .unwrap_or(true);
    if class_matches && expected_matches {
        Ok(())
    } else {
        Err(worker_error(
            "download_mime_invalid",
            "下载响应不是预期媒体类型",
        ))
    }
}

fn validate_content_range(
    headers: &HeaderMap,
    start: u64,
    expected_total: Option<u64>,
) -> Result<(), DownloadError> {
    let value = headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| worker_error("download_range_invalid", "下载续传范围无效"))?;
    let value = value
        .strip_prefix("bytes ")
        .ok_or_else(|| worker_error("download_range_invalid", "下载续传范围无效"))?;
    let (range, total) = value
        .split_once('/')
        .ok_or_else(|| worker_error("download_range_invalid", "下载续传范围无效"))?;
    let (actual_start, end) = range
        .split_once('-')
        .ok_or_else(|| worker_error("download_range_invalid", "下载续传范围无效"))?;
    let actual_start = actual_start
        .parse::<u64>()
        .map_err(|_| worker_error("download_range_invalid", "下载续传范围无效"))?;
    let end = end
        .parse::<u64>()
        .map_err(|_| worker_error("download_range_invalid", "下载续传范围无效"))?;
    let total = total
        .parse::<u64>()
        .map_err(|_| worker_error("download_range_invalid", "下载续传范围无效"))?;
    if actual_start != start
        || end < start
        || end.checked_add(1) != Some(total)
        || expected_total.is_some_and(|v| v != total)
    {
        Err(worker_error(
            "download_range_invalid",
            "下载续传范围与临时文件不兼容",
        ))
    } else {
        Ok(())
    }
}

fn validate_validator(headers: &HeaderMap, expected: &RetryValidator) -> Result<(), DownloadError> {
    let actual = match expected {
        RetryValidator::Etag(_) => header_text(headers, ETAG),
        RetryValidator::LastModified(_) => header_text(headers, LAST_MODIFIED),
    };
    if actual.as_deref() != Some(expected.value()) {
        return Err(worker_error(
            "download_validator_changed",
            "下载资源验证标识已变化",
        ));
    }
    Ok(())
}

fn validate_signature(bytes: &[u8], class: AssetClass) -> Result<(), DownloadError> {
    let valid = match class {
        AssetClass::Video => {
            bytes.len() >= 12 && (&bytes[4..8] == b"ftyp" || bytes.starts_with(b"\x1aE\xdf\xa3"))
        }
        AssetClass::Image => {
            bytes.starts_with(b"\xff\xd8\xff")
                || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                || bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")
        }
        AssetClass::Audio => {
            bytes.starts_with(b"ID3")
                || bytes.starts_with(b"OggS")
                || bytes.starts_with(b"fLaC")
                || bytes.get(4..8) == Some(b"ftyp")
        }
    };
    if valid {
        Ok(())
    } else {
        Err(worker_error(
            "download_signature_invalid",
            "下载内容不是有效媒体文件",
        ))
    }
}

fn reserve_job_bytes(counter: &AtomicU64, bytes: u64, limit: u64) -> Result<(), DownloadError> {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        let Some(next) = current.checked_add(bytes) else {
            return Err(capacity_error());
        };
        if next > limit {
            return Err(capacity_error());
        }
        match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Ok(()),
            Err(actual) => current = actual,
        }
    }
}

fn header_text(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn public_ipv4(address: Ipv4Addr) -> bool {
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_unspecified()
        || address.is_broadcast()
        || address.octets()[0] == 0
        || address.octets()[0] >= 224
        || address.octets()[0] == 100 && (64..=127).contains(&address.octets()[1])
        || address.octets()[0] == 192 && address.octets()[1] == 0 && address.octets()[2] == 0
        || address.octets()[0] == 192 && address.octets()[1] == 0 && address.octets()[2] == 2
        || address.octets()[0] == 198 && matches!(address.octets()[1], 18 | 19)
        || address.octets()[0] == 198 && address.octets()[1] == 51 && address.octets()[2] == 100
        || address.octets()[0] == 203 && address.octets()[1] == 0 && address.octets()[2] == 113)
}

fn public_ipv6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    !(address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

fn fake_proxy_address(address: IpAddr) -> bool {
    let address = match address {
        IpAddr::V4(address) => Some(address),
        IpAddr::V6(address) => address.to_ipv4_mapped(),
    };
    address.is_some_and(|address| {
        let octets = address.octets();
        octets[0] == 198 && matches!(octets[1], 18 | 19)
    })
}

fn trusted_douyin_media_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    ["douyinvod.com", "douyinpic.com", "byteimg.com"]
        .into_iter()
        .any(|suffix| host == suffix || host.ends_with(&format!(".{suffix}")))
}

fn worker_error(code: &str, message: &str) -> DownloadError {
    DownloadError {
        code: code.to_string(),
        message: message.to_string(),
    }
}

fn capacity_error() -> DownloadError {
    worker_error("download_capacity_exceeded", "下载数据超过安全容量限制")
}

fn header_limit_error() -> DownloadError {
    worker_error("download_headers_too_large", "下载响应头超过安全限制")
}

fn cancelled_error() -> DownloadError {
    worker_error("download_cancelled", "下载已取消")
}

fn request_failure(error: reqwest::Error) -> AttemptFailure {
    let mut source: Option<&(dyn StdError + 'static)> = Some(&error);
    while let Some(current) = source {
        if let Some(download) = current.downcast_ref::<DownloadError>() {
            return AttemptFailure::permanent(download.clone());
        }
        source = current.source();
    }
    if error.is_redirect() {
        AttemptFailure::permanent(worker_error(
            "download_redirect_limit",
            "下载重定向超过安全限制",
        ))
    } else {
        AttemptFailure::retryable(worker_error("download_network_failed", "下载网络请求失败"))
    }
}
