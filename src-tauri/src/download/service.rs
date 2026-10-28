use std::{
    collections::{BTreeMap, HashMap},
    fmt, fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use futures_util::{stream, StreamExt};
use rusqlite::Connection;
use serde::Serialize;
use tokio::{sync::broadcast, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use super::{
    asset_ownership_key, contained_path, create_download_job, expand_candidates, read_frame,
    recover_download_jobs, write_frame, AssetClass, CompletedAsset, CreateDownloadJobRequest,
    DownloadError, DownloadItemState, DownloadJobOwner, DownloadMutation, DownloadQuality,
    DownloadQueue, DownloadSelection, DownloadWorker, FilesystemRootAccess, JobTransferQuota,
    PersistenceHandle, PublicInternetPolicy, RecoverySummary, ResolvedAsset,
    ResolvedMediaManifest, ResolverControl, ResolverRequest, ResolverResponse, TransferRequest,
};
use crate::{
    assert_schema_latest, configure_connection,
    sync::{
        BrowserOperationKind, BrowserOperationLease as SharedBrowserOperationLease,
        OperationAdmission,
    },
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone)]
pub struct BrowserOperationCoordinator {
    admission: OperationAdmission,
}

impl Default for BrowserOperationCoordinator {
    fn default() -> Self {
        Self::from_admission(OperationAdmission::new())
    }
}

impl BrowserOperationCoordinator {
    pub fn from_admission(admission: OperationAdmission) -> Self {
        Self { admission }
    }

    pub fn acquire(&self) -> Result<BrowserOperationLease, String> {
        self.admission
            .begin_browser_operation(BrowserOperationKind::Download, None)
            .map(|lease| BrowserOperationLease { _lease: lease })
            .map_err(|_| "已有登录、同步或下载解析操作正在运行".to_string())
    }
}

pub struct BrowserOperationLease {
    _lease: SharedBrowserOperationLease,
}

impl fmt::Debug for BrowserOperationLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserOperationLease")
            .finish_non_exhaustive()
    }
}

pub struct DownloadService {
    database: PathBuf,
    queue: DownloadQueue,
    persistence: PersistenceHandle,
    cancellations: tokio::sync::Mutex<HashMap<i64, CancellationToken>>,
    active_child: tokio::sync::Mutex<Option<ActiveResolverChild>>,
    workers: tokio::sync::Mutex<Vec<JoinHandle<()>>>,
    browser_operations: BrowserOperationCoordinator,
    runtime: Option<Arc<DownloadRuntimeConfig>>,
    scheduler_running: AtomicBool,
    shutting_down: AtomicBool,
    shutdown: CancellationToken,
    events: broadcast::Sender<DownloadRefreshEvent>,
}

pub type DownloadJobFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, DownloadError>> + Send + 'a>>;

pub trait DownloadJobResolver: Send + Sync {
    fn resolve<'a>(
        &'a self,
        service: &'a DownloadService,
        request: ResolverRequest,
        cancellation: CancellationToken,
    ) -> DownloadJobFuture<'a, ResolvedMediaManifest>;
}

pub trait DownloadAssetTransfer: Send + Sync {
    fn transfer<'a>(
        &'a self,
        request: TransferRequest,
        cancellation: CancellationToken,
    ) -> DownloadJobFuture<'a, CompletedAsset>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownloadFaultPoint {
    QueryRunning,
    StartNext,
    HasRunnableJob,
    LoadNextItem,
    ReconcileManifest,
    LoadTransferRequests,
    StartAsset,
    CompleteAsset,
    FailAsset,
    FinalizeAssetBatch,
    FailItem,
    InterruptJob,
}

pub trait DownloadFaultInjector: Send + Sync {
    fn check(&self, point: DownloadFaultPoint) -> Result<(), DownloadError>;
}

struct NoDownloadFaults;

impl DownloadFaultInjector for NoDownloadFaults {
    fn check(&self, _point: DownloadFaultPoint) -> Result<(), DownloadError> {
        Ok(())
    }
}

impl DownloadAssetTransfer for DownloadWorker {
    fn transfer<'a>(
        &'a self,
        request: TransferRequest,
        cancellation: CancellationToken,
    ) -> DownloadJobFuture<'a, CompletedAsset> {
        Box::pin(DownloadWorker::transfer(self, request, cancellation))
    }
}

pub struct DownloadRuntimeConfig {
    resolver: Arc<dyn DownloadJobResolver>,
    transfer: Arc<dyn DownloadAssetTransfer>,
    faults: Arc<dyn DownloadFaultInjector>,
}

impl DownloadRuntimeConfig {
    pub fn new(
        resolver: Arc<dyn DownloadJobResolver>,
        transfer: Arc<dyn DownloadAssetTransfer>,
    ) -> Self {
        Self {
            resolver,
            transfer,
            faults: Arc::new(NoDownloadFaults),
        }
    }

    pub fn with_fault_injector(mut self, faults: Arc<dyn DownloadFaultInjector>) -> Self {
        self.faults = faults;
        self
    }

    pub fn production(
        executable: PathBuf,
        browser_profiles_root: PathBuf,
        temp_dir: PathBuf,
    ) -> Result<Self, DownloadError> {
        let worker = DownloadWorker::builder(Arc::new(PublicInternetPolicy)).build()?;
        Ok(Self::new(
            Arc::new(ChildDownloadJobResolver {
                executable,
                browser_profiles_root,
                temp_dir,
            }),
            Arc::new(worker),
        ))
    }
}

struct ChildDownloadJobResolver {
    executable: PathBuf,
    browser_profiles_root: PathBuf,
    temp_dir: PathBuf,
}

impl DownloadJobResolver for ChildDownloadJobResolver {
    fn resolve<'a>(
        &'a self,
        service: &'a DownloadService,
        request: ResolverRequest,
        cancellation: CancellationToken,
    ) -> DownloadJobFuture<'a, ResolvedMediaManifest> {
        Box::pin(async move {
            let pid = self
                .temp_dir
                .join(format!("download-{}.browser.pid", request.request_id));
            let browser_data_dir = self.browser_profiles_root.join(&request.profile_key);
            let resolve = service.resolve(
                &self.executable,
                &browser_data_dir,
                &pid,
                request.clone(),
            );
            tokio::pin!(resolve);
            let response = tokio::select! {
                response = &mut resolve => response?,
                _ = cancellation.cancelled() => {
                    let _ = service.cancel_active().await;
                    return Err(download_error("download_cancelled", "下载已暂停或取消"));
                }
            };
            match response {
                ResolverResponse::Resolved {
                    request_id,
                    manifest,
                } if request_id == request.request_id => Ok(manifest),
                ResolverResponse::Error { code, message, .. } => {
                    Err(DownloadError { code, message })
                }
                _ => Err(download_error(
                    "download_resolver_mismatch",
                    "解析器响应与请求不匹配",
                )),
            }
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRefreshEvent {
    pub job_id: i64,
    pub item_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJobSummary {
    pub id: i64,
    pub account_id: i64,
    pub state: String,
    pub destination_root: String,
    pub requested_quality: String,
    pub candidate_count: i64,
    pub resolved_count: i64,
    pub completed_count: i64,
    pub skipped_count: i64,
    pub failed_count: i64,
    pub downloaded_bytes: i64,
    pub total_bytes: i64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPreview {
    pub candidate_count: i64,
    pub content_type_counts: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadItemSummary {
    pub id: i64,
    pub aweme_id: String,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub content_type: String,
    pub state: String,
    pub completed_asset_count: i64,
    pub total_required_asset_count: i64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadAssetSummary {
    pub id: i64,
    pub item_id: i64,
    pub stable_key: String,
    pub asset_type: String,
    pub state: String,
    pub downloaded_bytes: i64,
    pub expected_size: Option<i64>,
    pub retry_count: i64,
    pub last_error_code: Option<String>,
    pub relative_final_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJobDetail {
    #[serde(flatten)]
    pub job: DownloadJobSummary,
    pub items: Vec<DownloadItemSummary>,
    pub assets: Vec<DownloadAssetSummary>,
    pub current_resolver_item_id: Option<i64>,
    pub active_asset_count: i64,
    pub cancelled_asset_count: i64,
    pub average_bytes_per_second: i64,
}

pub struct DownloadRevealTarget {
    pub path: PathBuf,
    pub select: bool,
}

struct ActiveResolverChild {
    request_id: u64,
    input: Arc<Mutex<ChildStdin>>,
    control: std::sync::mpsc::Sender<ChildControl>,
    state: tokio::sync::watch::Receiver<ChildOwnerState>,
}

struct PendingItem {
    id: i64,
    account_id: i64,
    profile_key: String,
    aweme_id: String,
    canonical_url: String,
    quality: DownloadQuality,
    destination_root: PathBuf,
    relative_work_directory: PathBuf,
}

struct TransferRow {
    asset_id: i64,
    stable_key: String,
    relative_part_path: PathBuf,
    relative_final_path: PathBuf,
    expected_size: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
    mime_type: Option<String>,
}

enum ChildControl {
    Kill,
}

#[derive(Debug, Clone)]
enum ChildOwnerState {
    Running,
    TerminationFailed(DownloadError),
    Exited,
}

trait ResolverProcessControl: Send + 'static {
    fn try_wait(&mut self) -> std::io::Result<bool>;
    fn kill(&mut self) -> std::io::Result<()>;
    fn wait(&mut self) -> std::io::Result<()>;
}

impl ResolverProcessControl for Child {
    fn try_wait(&mut self) -> std::io::Result<bool> {
        Child::try_wait(self).map(|status| status.is_some())
    }
    fn kill(&mut self) -> std::io::Result<()> {
        Child::kill(self)
    }
    fn wait(&mut self) -> std::io::Result<()> {
        Child::wait(self).map(|_| ())
    }
}

impl DownloadService {
    pub fn recover_offline(database: impl AsRef<Path>) -> Result<RecoverySummary, DownloadError> {
        let mut connection = open_service_database(database.as_ref())?;
        recover_download_jobs(&mut connection, &FilesystemRootAccess)
    }

    pub async fn spawn(
        database: impl AsRef<Path>,
        browser_operations: BrowserOperationCoordinator,
    ) -> Result<Self, DownloadError> {
        Self::spawn_inner(database, browser_operations, None).await
    }

    pub async fn spawn_with_runtime(
        database: impl AsRef<Path>,
        browser_operations: BrowserOperationCoordinator,
        runtime: DownloadRuntimeConfig,
    ) -> Result<Self, DownloadError> {
        Self::spawn_inner(database, browser_operations, Some(Arc::new(runtime))).await
    }

    async fn spawn_inner(
        database: impl AsRef<Path>,
        browser_operations: BrowserOperationCoordinator,
        runtime: Option<Arc<DownloadRuntimeConfig>>,
    ) -> Result<Self, DownloadError> {
        let database = PathBuf::from(database.as_ref());
        let persistence = PersistenceHandle::spawn(&database, 64).await?;
        let (events, _) = broadcast::channel(128);
        Ok(Self {
            database,
            queue: DownloadQueue::new(persistence.clone()),
            persistence,
            cancellations: tokio::sync::Mutex::new(HashMap::new()),
            active_child: tokio::sync::Mutex::new(None),
            workers: tokio::sync::Mutex::new(Vec::new()),
            browser_operations,
            runtime,
            scheduler_running: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            shutdown: CancellationToken::new(),
            events,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DownloadRefreshEvent> {
        self.events.subscribe()
    }

    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub fn queue(&self) -> &DownloadQueue {
        &self.queue
    }
    pub fn persistence(&self) -> &PersistenceHandle {
        &self.persistence
    }

    pub async fn create_job(
        self: &Arc<Self>,
        owner: DownloadJobOwner,
        mut request: CreateDownloadJobRequest,
    ) -> Result<DownloadJobDetail, DownloadError> {
        let root = validate_destination_root(Path::new(&request.destination_root))?;
        request.destination_root = root.to_string_lossy().into_owned();
        let database = self.database.clone();
        let job_id = tokio::task::spawn_blocking(move || {
            let mut connection = open_service_database(&database)?;
            create_download_job(&mut connection, &owner, &request)
        })
        .await
        .map_err(service_error)??;
        self.publish(job_id, None);
        self.ensure_scheduler().await;
        self.get_job(job_id).await
    }

    pub async fn list_jobs(&self) -> Result<Vec<DownloadJobSummary>, DownloadError> {
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || {
            let connection = open_service_database(&database)?;
            query_jobs(&connection)
        })
        .await
        .map_err(service_error)?
    }

    pub async fn preview_selection(
        &self,
        account_id: i64,
        selection: DownloadSelection,
    ) -> Result<DownloadPreview, DownloadError> {
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || {
            let connection = open_service_database(&database)?;
            let candidates = expand_candidates(&connection, account_id, &selection)?;
            let mut content_type_counts = BTreeMap::new();
            for candidate in &candidates {
                *content_type_counts
                    .entry(candidate.content_type.clone())
                    .or_insert(0) += 1;
            }
            Ok(DownloadPreview {
                candidate_count: candidates.len() as i64,
                content_type_counts,
            })
        })
        .await
        .map_err(service_error)?
    }

    pub async fn get_job(&self, job_id: i64) -> Result<DownloadJobDetail, DownloadError> {
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || {
            let connection = open_service_database(&database)?;
            query_job(&connection, job_id)
        })
        .await
        .map_err(service_error)?
    }

    pub async fn pause_job(&self, job_id: i64) -> Result<DownloadJobDetail, DownloadError> {
        self.queue.pause(job_id).await?;
        self.cancel_job_work(job_id).await;
        self.publish(job_id, None);
        self.get_job(job_id).await
    }

    pub async fn resume_job(
        self: &Arc<Self>,
        job_id: i64,
    ) -> Result<DownloadJobDetail, DownloadError> {
        self.replace_cancellation(job_id).await;
        self.queue.resume(job_id).await?;
        self.publish(job_id, None);
        self.ensure_scheduler().await;
        self.get_job(job_id).await
    }

    pub async fn cancel_job(&self, job_id: i64) -> Result<DownloadJobDetail, DownloadError> {
        self.queue.cancel(job_id).await?;
        self.cancel_job_work(job_id).await;
        self.remove_job_partials(job_id).await?;
        self.publish(job_id, None);
        self.get_job(job_id).await
    }

    pub async fn retry_failed(
        self: &Arc<Self>,
        job_id: i64,
    ) -> Result<DownloadJobDetail, DownloadError> {
        self.replace_cancellation(job_id).await;
        self.queue.retry_failed(job_id).await?;
        self.publish(job_id, None);
        self.ensure_scheduler().await;
        self.get_job(job_id).await
    }

    pub async fn request_shutdown(&self) -> Result<(), DownloadError> {
        if self
            .shutting_down
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        self.shutdown.cancel();
        {
            let cancellations = self.cancellations.lock().await;
            for cancellation in cancellations.values() {
                cancellation.cancel();
            }
        }
        let _ = self.cancel_active().await;
        let event = self
            .persistence
            .transition(DownloadMutation::PauseRunning)
            .await?;
        if let Some(job) = event.job() {
            self.publish(job.id, None);
        }
        Ok(())
    }

    pub async fn wait_for_shutdown(&self, timeout: Duration) -> Result<bool, DownloadError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let workers = {
            let mut workers = self.workers.lock().await;
            std::mem::take(&mut *workers)
        };
        let mut workers = workers.into_iter();
        let mut pending = Vec::new();
        while let Some(mut worker) = workers.next() {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero()
                || tokio::time::timeout(remaining, &mut worker).await.is_err()
            {
                pending.push(worker);
                pending.extend(workers);
                break;
            }
        }
        if !pending.is_empty() {
            self.workers.lock().await.extend(pending);
            return Ok(false);
        }
        Ok(true)
    }

    pub async fn shutdown_gracefully(&self) -> Result<(), DownloadError> {
        self.request_shutdown().await?;
        let _ = self.wait_for_shutdown(Duration::from_secs(5)).await?;
        Ok(())
    }

    pub async fn reveal_target(
        &self,
        job_id: Option<i64>,
        item_id: Option<i64>,
    ) -> Result<DownloadRevealTarget, DownloadError> {
        if job_id.is_some() == item_id.is_some() {
            return Err(download_error(
                "download_reveal_invalid",
                "必须且只能指定一个下载任务或条目 ID",
            ));
        }
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || {
            let connection = open_service_database(&database)?;
            query_reveal_target(&connection, job_id, item_id)
        })
        .await
        .map_err(service_error)?
    }

    pub async fn cancellation_for(&self, job_id: i64) -> CancellationToken {
        self.cancellations
            .lock()
            .await
            .entry(job_id)
            .or_default()
            .clone()
    }

    pub async fn spawn_worker<F>(&self, worker: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.workers.lock().await.push(tokio::spawn(worker));
    }

    async fn ensure_scheduler(self: &Arc<Self>) {
        if self.shutting_down.load(Ordering::Acquire)
            || self.runtime.is_none()
            || self
                .scheduler_running
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        let service = Arc::clone(self);
        let handle = tokio::spawn(async move {
            let mut handoff_retry_delay = Duration::from_millis(25);
            'worker: loop {
                service.scheduler_loop().await;
                loop {
                    match service.has_runnable_job().await {
                        Ok(true) => {
                            handoff_retry_delay = Duration::from_millis(25);
                            continue 'worker;
                        }
                        Err(_) => {
                            scheduler_retry(&mut handoff_retry_delay).await;
                        }
                        Ok(false) => {
                            handoff_retry_delay = Duration::from_millis(25);
                            service.scheduler_running.store(false, Ordering::Release);
                            match service.has_runnable_job().await {
                                Ok(false) => break 'worker,
                                Ok(true) => {
                                    if service
                                        .scheduler_running
                                        .compare_exchange(
                                            false,
                                            true,
                                            Ordering::AcqRel,
                                            Ordering::Acquire,
                                        )
                                        .is_err()
                                    {
                                        break 'worker;
                                    }
                                    continue 'worker;
                                }
                                Err(_) => {
                                    if service
                                        .scheduler_running
                                        .compare_exchange(
                                            false,
                                            true,
                                            Ordering::AcqRel,
                                            Ordering::Acquire,
                                        )
                                        .is_err()
                                    {
                                        break 'worker;
                                    }
                                    scheduler_retry(&mut handoff_retry_delay).await;
                                }
                            }
                        }
                    }
                }
            }
        });
        self.workers.lock().await.push(handle);
    }

    async fn scheduler_loop(self: &Arc<Self>) {
        let mut query_retry_delay = Duration::from_millis(25);
        let mut start_next_retry_delay = Duration::from_millis(25);
        loop {
            if self.shutting_down.load(Ordering::Acquire) {
                break;
            }
            let running = match self.query_running_job().await {
                Ok(running) => {
                    query_retry_delay = Duration::from_millis(25);
                    running
                }
                Err(_) => {
                    scheduler_retry(&mut query_retry_delay).await;
                    continue;
                }
            };
            let job_id = if let Some(job_id) = running {
                Some(job_id)
            } else {
                let start_next = match self.check_fault(DownloadFaultPoint::StartNext) {
                    Ok(()) => self.queue.start_next().await,
                    Err(error) => Err(error),
                };
                match start_next {
                    Ok(event) => {
                        start_next_retry_delay = Duration::from_millis(25);
                        let job_id = event.job().map(|job| job.id);
                        if let Some(job_id) = job_id {
                            self.publish(job_id, None);
                        }
                        job_id
                    }
                    Err(_) => {
                        scheduler_retry(&mut start_next_retry_delay).await;
                        continue;
                    }
                }
            };
            let Some(job_id) = job_id else { break };
            self.process_job(job_id).await;
        }
    }

    async fn process_job(self: &Arc<Self>, job_id: i64) {
        let Some(runtime) = self.runtime.clone() else {
            return;
        };
        let cancellation = self.replace_cancellation(job_id).await;
        let job_quota = JobTransferQuota::default();
        'job_items: loop {
            if cancellation.is_cancelled() || self.shutting_down.load(Ordering::Acquire) {
                break;
            }
            let item = match self.load_next_item(job_id).await {
                Ok(Some(item)) => item,
                Ok(None) => {
                    if let Err(error) = self
                        .persistence
                        .transition(DownloadMutation::RecomputeJob { job_id })
                        .await
                    {
                        self.interrupt_job(job_id, &error.code).await;
                    } else {
                        self.publish(job_id, None);
                    }
                    break;
                }
                Err(error) => {
                    self.interrupt_job(job_id, &error.code).await;
                    break;
                }
            };
            if let Err(error) = self
                .persistence
                .transition(DownloadMutation::SetItemState {
                    job_id,
                    aweme_id: item.aweme_id.clone(),
                    expected_state: DownloadItemState::Queued,
                    next_state: DownloadItemState::Resolving,
                    downloaded_bytes: 0,
                })
                .await
            {
                self.converge_item_failure(job_id, item.id, error).await;
                break;
            }
            self.publish(job_id, Some(item.id));
            let mut original_manifest: Option<ResolvedMediaManifest> = None;
            let mut forced_refresh_used = false;
            'resolve_item: loop {
                let manifest = runtime
                    .resolver
                    .resolve(
                        self,
                        ResolverRequest {
                            request_id: item.id as u64,
                            account_id: item.account_id,
                            profile_key: item.profile_key.clone(),
                            aweme_id: item.aweme_id.clone(),
                            canonical_url: item.canonical_url.clone(),
                            quality: item.quality,
                        },
                        cancellation.clone(),
                    )
                    .await;
                let manifest = match manifest {
                    Ok(manifest) => manifest,
                    Err(_) if cancellation.is_cancelled() => break 'job_items,
                    Err(error) => {
                        self.converge_item_failure(job_id, item.id, error).await;
                        continue 'job_items;
                    }
                };
                if let Some(original) = &original_manifest {
                    if !same_manifest_identity(original, &manifest) {
                        self.converge_item_failure(
                            job_id,
                            item.id,
                            download_error(
                                "download_refresh_identity_changed",
                                "重新解析后的媒体身份发生变化",
                            ),
                        )
                        .await;
                        continue 'job_items;
                    }
                } else {
                    original_manifest = Some(manifest.clone());
                }
                let reconcile = match self.check_fault(DownloadFaultPoint::ReconcileManifest) {
                    Ok(()) => {
                        self.persistence
                            .reconcile_manifest(item.id, manifest.clone())
                            .await
                    }
                    Err(error) => Err(error),
                };
                if let Err(error) = reconcile {
                    self.converge_item_failure(job_id, item.id, error).await;
                    continue 'job_items;
                }
                let _ = self
                    .persistence
                    .transition(DownloadMutation::RecomputeJob { job_id })
                    .await;
                self.publish(job_id, Some(item.id));
                let transfers = match self
                    .load_transfer_requests(&item, &manifest, &job_quota)
                    .await
                {
                    Ok(transfers) => transfers,
                    Err(error) => {
                        self.converge_item_failure(job_id, item.id, error).await;
                        continue 'job_items;
                    }
                };
                if transfers.is_empty() {
                    let _ = self
                        .persistence
                        .transition(DownloadMutation::RecomputeJob { job_id })
                        .await;
                    self.publish(job_id, Some(item.id));
                    continue 'job_items;
                }
                let mut outcomes =
                    stream::iter(transfers.into_iter().enumerate().map(|(order, request)| {
                        let service = Arc::clone(self);
                        let transfer = Arc::clone(&runtime.transfer);
                        let cancellation = cancellation.clone();
                        async move {
                            (
                                order,
                                service
                                    .run_asset_transfer(
                                        job_id,
                                        item.id,
                                        transfer,
                                        request,
                                        cancellation,
                                    )
                                    .await,
                            )
                        }
                    }))
                    .buffer_unordered(3)
                    .collect::<Vec<_>>()
                    .await;
                if cancellation.is_cancelled() {
                    break 'job_items;
                }
                outcomes.sort_by_key(|(order, _)| *order);
                let source_refresh_required = outcomes.iter().any(|(_, error)| {
                    error
                        .as_ref()
                        .is_some_and(|error| error.code == "download_source_expired")
                });
                if source_refresh_required && !forced_refresh_used {
                    match self
                        .persistence
                        .transition(DownloadMutation::PrepareReresolve {
                            job_id,
                            item_id: item.id,
                        })
                        .await
                    {
                        Ok(_) => {
                            forced_refresh_used = true;
                            self.publish(job_id, Some(item.id));
                            continue 'resolve_item;
                        }
                        Err(error) => {
                            self.converge_item_failure(job_id, item.id, error).await;
                            continue 'job_items;
                        }
                    }
                }
                let first_error = outcomes.into_iter().find_map(|(_, error)| error);
                let (error_code, error_message) = first_error
                    .map(|error| (Some(error.code), Some(error.message)))
                    .unwrap_or((None, None));
                let finalization = match self.check_fault(DownloadFaultPoint::FinalizeAssetBatch) {
                    Ok(()) => {
                        self.persistence
                            .transition(DownloadMutation::FinalizeAssetBatch {
                                job_id,
                                item_id: item.id,
                                error_code,
                                error_message,
                            })
                            .await
                    }
                    Err(error) => Err(error),
                };
                if let Err(error) = finalization {
                    self.converge_item_failure(job_id, item.id, error).await;
                } else {
                    self.publish(job_id, Some(item.id));
                }
                continue 'job_items;
            }
        }
        self.cancellations.lock().await.remove(&job_id);
    }

    async fn run_asset_transfer(
        &self,
        job_id: i64,
        item_id: i64,
        transfer: Arc<dyn DownloadAssetTransfer>,
        request: TransferRequest,
        cancellation: CancellationToken,
    ) -> Option<DownloadError> {
        let asset_id = request.asset_id;
        if source_is_expired(request.source_expires_at.as_deref()) {
            return Some(download_error(
                "download_source_expired",
                "下载地址已过期，需要重新解析",
            ));
        }
        let part_path = request.destination_root.join(&request.relative_part_path);
        let start = match self.check_fault(DownloadFaultPoint::StartAsset) {
            Ok(()) => {
                self.persistence
                    .transition(DownloadMutation::StartAsset {
                        job_id,
                        item_id,
                        asset_id,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        if let Err(error) = start {
            return Some(error);
        }
        self.publish(job_id, Some(item_id));
        match transfer.transfer(request, cancellation.clone()).await {
            Ok(completed) => {
                let complete = match self.check_fault(DownloadFaultPoint::CompleteAsset) {
                    Ok(()) => {
                        self.persistence
                            .transition(DownloadMutation::CompleteAsset {
                                job_id,
                                item_id,
                                asset_id,
                                downloaded_bytes: completed.bytes as i64,
                                sha256: completed.sha256,
                                etag: completed.etag,
                                last_modified: completed.last_modified,
                            })
                            .await
                    }
                    Err(error) => Err(error),
                };
                if let Err(error) = complete {
                    return Some(error);
                }
            }
            Err(error) if !cancellation.is_cancelled() => {
                let bytes = fs::metadata(part_path)
                    .map(|metadata| metadata.len() as i64)
                    .unwrap_or(0);
                let fail = match self.check_fault(DownloadFaultPoint::FailAsset) {
                    Ok(()) => {
                        self.persistence
                            .transition(DownloadMutation::FailAsset {
                                job_id,
                                item_id,
                                asset_id,
                                downloaded_bytes: bytes,
                                error_code: error.code.clone(),
                            })
                            .await
                    }
                    Err(error) => Err(error),
                };
                if let Err(persistence_error) = fail {
                    return Some(persistence_error);
                }
                self.publish(job_id, Some(item_id));
                return Some(error);
            }
            Err(_) => return None,
        }
        self.publish(job_id, Some(item_id));
        None
    }

    async fn converge_item_failure(&self, job_id: i64, item_id: i64, error: DownloadError) {
        let result = match self.check_fault(DownloadFaultPoint::FailItem) {
            Ok(()) => {
                self.persistence
                    .transition(DownloadMutation::FailItem {
                        job_id,
                        item_id,
                        error_code: error.code.clone(),
                        error_message: error.message,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        if result.is_err() {
            self.interrupt_job(job_id, "download_failure_persistence_failed")
                .await;
        }
        self.publish(job_id, Some(item_id));
    }

    async fn interrupt_job(&self, job_id: i64, error_code: &str) {
        let interrupted = match self.check_fault(DownloadFaultPoint::InterruptJob) {
            Ok(()) => {
                self.persistence
                    .transition(DownloadMutation::InterruptJob {
                        job_id,
                        error_code: error_code.to_string(),
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        if interrupted.is_err() {
            let _ = emergency_interrupt_job(&self.database, job_id, error_code);
        }
        self.publish(job_id, None);
    }

    fn publish(&self, job_id: i64, item_id: Option<i64>) {
        let _ = self.events.send(DownloadRefreshEvent { job_id, item_id });
    }

    fn check_fault(&self, point: DownloadFaultPoint) -> Result<(), DownloadError> {
        if let Some(runtime) = &self.runtime {
            runtime.faults.check(point)
        } else {
            Ok(())
        }
    }

    async fn replace_cancellation(&self, job_id: i64) -> CancellationToken {
        let token = CancellationToken::new();
        if let Some(previous) = self
            .cancellations
            .lock()
            .await
            .insert(job_id, token.clone())
        {
            previous.cancel();
        }
        token
    }

    async fn cancel_job_work(&self, job_id: i64) {
        if let Some(token) = self.cancellations.lock().await.get(&job_id).cloned() {
            token.cancel();
        }
        let _ = self.cancel_active().await;
    }

    async fn query_running_job(&self) -> Result<Option<i64>, DownloadError> {
        self.check_fault(DownloadFaultPoint::QueryRunning)?;
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || {
            use rusqlite::OptionalExtension;
            let connection = open_service_database(&database)?;
            connection
                .query_row(
                    "SELECT id FROM download_jobs WHERE state='running' ORDER BY id LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| download_error("download_storage_failed", "无法读取运行中的下载任务"))
        })
        .await
        .map_err(service_error)?
    }

    async fn has_runnable_job(&self) -> Result<bool, DownloadError> {
        self.check_fault(DownloadFaultPoint::HasRunnableJob)?;
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || {
            let connection = open_service_database(&database)?;
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM download_jobs WHERE state IN ('queued','running')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| download_error("download_storage_failed", "无法读取待运行下载任务"))?;
            Ok(count > 0)
        })
        .await
        .map_err(service_error)?
    }

    async fn load_next_item(&self, job_id: i64) -> Result<Option<PendingItem>, DownloadError> {
        self.check_fault(DownloadFaultPoint::LoadNextItem)?;
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || {
            use rusqlite::OptionalExtension;
            let connection = open_service_database(&database)?;
            connection.query_row("SELECT i.id,j.account_id,j.profile_key,i.aweme_id,i.canonical_url,j.requested_quality,j.destination_root,i.relative_work_directory FROM download_items i JOIN download_jobs j ON j.id=i.job_id WHERE i.job_id=?1 AND i.state='queued' AND j.state='running' ORDER BY i.id LIMIT 1", [job_id], |row| {
                let quality: String = row.get(5)?;
                Ok(PendingItem {
                    id: row.get(0)?, account_id: row.get(1)?, profile_key: row.get(2)?,
                    aweme_id: row.get(3)?, canonical_url: row.get(4)?,
                    quality: parse_quality(&quality), destination_root: PathBuf::from(row.get::<_, String>(6)?),
                    relative_work_directory: PathBuf::from(row.get::<_, String>(7)?),
                })
            }).optional().map_err(|_| download_error("download_storage_failed", "无法读取待下载条目"))
        }).await.map_err(service_error)?
    }

    async fn load_transfer_requests(
        &self,
        item: &PendingItem,
        manifest: &ResolvedMediaManifest,
        job_quota: &JobTransferQuota,
    ) -> Result<Vec<TransferRequest>, DownloadError> {
        self.check_fault(DownloadFaultPoint::LoadTransferRequests)?;
        let database = self.database.clone();
        let item_id = item.id;
        let rows = tokio::task::spawn_blocking(move || {
            let connection = open_service_database(&database)?;
            let mut statement = connection.prepare("SELECT id,stable_key,relative_part_path,relative_final_path,expected_size,etag,last_modified,mime_type FROM download_assets WHERE item_id=?1 AND required=1 AND state='queued' ORDER BY ordinal,id").map_err(|_| download_error("download_storage_failed", "无法读取待下载资源"))?;
            let rows = statement.query_map([item_id], |row| Ok(TransferRow {
                asset_id: row.get(0)?, stable_key: row.get(1)?, relative_part_path: PathBuf::from(row.get::<_, String>(2)?),
                relative_final_path: PathBuf::from(row.get::<_, String>(3)?), expected_size: row.get::<_, Option<i64>>(4)?.map(|value| value as u64),
                etag: row.get(5)?, last_modified: row.get(6)?, mime_type: row.get(7)?,
            })).map_err(|_| download_error("download_storage_failed", "无法读取待下载资源"))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|_| download_error("download_storage_failed", "无法解析待下载资源"))
        }).await.map_err(service_error)??;
        let quality = quality_name(item.quality).to_string();
        rows.into_iter()
            .map(|row| {
                let asset = manifest
                    .assets
                    .iter()
                    .find(|asset| asset.stable_key == row.stable_key)
                    .ok_or_else(|| {
                        download_error("download_manifest_mismatch", "下载资源与解析清单不匹配")
                    })?;
                Ok(TransferRequest {
                    asset_id: row.asset_id,
                    aweme_id: item.aweme_id.clone(),
                    stable_key: row.stable_key,
                    quality: quality.clone(),
                    ownership_key: asset_ownership_key(
                        &item.destination_root,
                        &item.aweme_id,
                        &asset.stable_key,
                        &quality,
                    ),
                    source_url: asset.source_url.clone(),
                    source_expires_at: asset.source_expires_at.clone(),
                    request_headers: asset.request_headers.clone(),
                    destination_root: item.destination_root.clone(),
                    relative_part_path: item
                        .relative_work_directory
                        .join(row.relative_part_path),
                    relative_final_path: item
                        .relative_work_directory
                        .join(row.relative_final_path),
                    expected_size: row.expected_size,
                    expected_etag: row.etag,
                    expected_last_modified: row.last_modified,
                    expected_mime: row.mime_type.or_else(|| asset.mime_type.clone()),
                    asset_class: asset_class(asset),
                    job_quota: job_quota.clone(),
                })
            })
            .collect()
    }

    async fn remove_job_partials(&self, job_id: i64) -> Result<(), DownloadError> {
        let database = self.database.clone();
        tokio::task::spawn_blocking(move || {
            let connection = open_service_database(&database)?;
            let root: String = connection.query_row("SELECT destination_root FROM download_jobs WHERE id=?1", [job_id], |row| row.get(0)).map_err(|_| download_error("download_job_not_found", "下载任务不存在"))?;
            let root = PathBuf::from(root);
            let mut statement = connection.prepare("SELECT i.relative_work_directory,a.relative_part_path FROM download_assets a JOIN download_items i ON i.id=a.item_id WHERE i.job_id=?1 AND a.state='cancelled'").map_err(|_| download_error("download_storage_failed", "无法读取下载临时文件"))?;
            let paths = statement.query_map([job_id], |row| Ok(PathBuf::from(row.get::<_, String>(0)?).join(row.get::<_, String>(1)?))).map_err(|_| download_error("download_storage_failed", "无法读取下载临时文件"))?;
            for relative in paths {
                let path = contained_path(&root, &relative.map_err(|_| download_error("download_storage_failed", "无法解析下载临时文件"))?)?;
                if path.exists() { fs::remove_file(path).map_err(|_| download_error("download_part_cleanup_failed", "无法删除取消任务的临时文件"))?; }
            }
            Ok(())
        }).await.map_err(service_error)?
    }

    pub async fn resolve(
        &self,
        executable: &Path,
        browser_data_dir: &Path,
        browser_pid_file: &Path,
        request: ResolverRequest,
    ) -> Result<ResolverResponse, DownloadError> {
        self.resolve_with_debug_endpoint(
            executable,
            browser_data_dir,
            browser_pid_file,
            request,
            None,
        )
        .await
    }

    pub async fn resolve_with_debug_endpoint(
        &self,
        executable: &Path,
        browser_data_dir: &Path,
        browser_pid_file: &Path,
        request: ResolverRequest,
        debug_endpoint: Option<(&str, u16)>,
    ) -> Result<ResolverResponse, DownloadError> {
        let lease = self.browser_operations.acquire().map_err(service_error)?;
        let mut command = Command::new(executable);
        command
            .arg("--download-resolver")
            .arg("--browser-data-dir")
            .arg(browser_data_dir)
            .arg("--browser-pid-file")
            .arg(browser_pid_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some((url, port)) = debug_endpoint {
            command
                .arg("--browser-debug-url")
                .arg(url)
                .arg("--browser-debug-port")
                .arg(port.to_string());
        }
        #[cfg(target_os = "windows")]
        command.creation_flags(CREATE_NO_WINDOW);
        let mut child = command.spawn().map_err(service_error)?;
        let Some(input) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(service_error("resolver stdin unavailable"));
        };
        let Some(mut output) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(service_error("resolver stdout unavailable"));
        };
        let input = Arc::new(Mutex::new(input));
        let (control, mut state) = spawn_child_owner(child, browser_pid_file.to_path_buf(), lease);
        {
            let mut active = self.active_child.lock().await;
            if active
                .as_ref()
                .is_some_and(|active| matches!(*active.state.borrow(), ChildOwnerState::Exited))
            {
                *active = None;
            }
            if active.is_some() {
                let _ = control.send(ChildControl::Kill);
                return Err(service_error("resolver already active"));
            }
            *active = Some(ActiveResolverChild {
                request_id: request.request_id,
                input: Arc::clone(&input),
                control,
                state: state.clone(),
            });
        }
        let write_input = Arc::clone(&input);
        let request_to_write = request.clone();
        #[cfg(debug_assertions)]
        let force_write_failure =
            std::env::var_os("EMILY_TEST_SERVICE_FAIL_RESOLVER_WRITE").is_some();
        #[cfg(not(debug_assertions))]
        let force_write_failure = false;
        let write_result = if force_write_failure {
            Err(service_error("resolver fixture write failure"))
        } else {
            tokio::task::spawn_blocking(move || {
                let mut input = write_input
                    .lock()
                    .map_err(|_| service_error("resolver stdin lock poisoned"))?;
                write_frame(&mut *input, &request_to_write)
            })
            .await
            .map_err(service_error)?
        };
        if let Err(error) = write_result {
            self.kill_active(request.request_id).await;
            return Err(error);
        }
        let response =
            tokio::task::spawn_blocking(move || read_frame::<_, ResolverResponse>(&mut output))
                .await
                .map_err(service_error)
                .and_then(|response| response);
        match wait_for_owner_state(&mut state, None, true).await {
            Some(ChildOwnerState::Exited) => {
                let _ = self.take_active(request.request_id).await;
                response
            }
            Some(ChildOwnerState::TerminationFailed(error)) => Err(error),
            Some(ChildOwnerState::Running) | None => Err(termination_error()),
        }
    }

    pub async fn cancel_active(&self) -> Result<(), DownloadError> {
        let (request_id, input, control, mut owner_state) = {
            let mut active = self.active_child.lock().await;
            if active
                .as_ref()
                .is_some_and(|active| matches!(*active.state.borrow(), ChildOwnerState::Exited))
            {
                *active = None;
            }
            let active = active
                .as_ref()
                .ok_or_else(|| service_error("resolver not active"))?;
            (
                active.request_id,
                Arc::clone(&active.input),
                active.control.clone(),
                active.state.clone(),
            )
        };
        tokio::task::spawn_blocking(move || {
            let mut input = input
                .lock()
                .map_err(|_| service_error("resolver stdin lock poisoned"))?;
            write_frame(&mut *input, &ResolverControl::Cancel)
        })
        .await
        .map_err(service_error)??;
        match wait_for_owner_state(&mut owner_state, Some(Duration::from_secs(2)), true).await {
            Some(ChildOwnerState::Exited) => {
                let _ = self.take_active(request_id).await;
                return Ok(());
            }
            Some(ChildOwnerState::TerminationFailed(error)) => return Err(error),
            Some(ChildOwnerState::Running) | None => {}
        }
        owner_state.borrow_and_update();
        control
            .send(ChildControl::Kill)
            .map_err(|_| termination_error())?;
        match wait_for_owner_state(&mut owner_state, Some(Duration::from_secs(3)), false).await {
            Some(ChildOwnerState::Exited) => {
                let _ = self.take_active(request_id).await;
                Ok(())
            }
            Some(ChildOwnerState::TerminationFailed(error)) => Err(error),
            Some(ChildOwnerState::Running) | None => {
                Err(service_error("resolver child termination timed out"))
            }
        }
    }

    async fn kill_active(&self, request_id: u64) {
        let owner = {
            let active = self.active_child.lock().await;
            active
                .as_ref()
                .filter(|active| active.request_id == request_id)
                .map(|active| (active.control.clone(), active.state.clone()))
        };
        if let Some((control, mut state)) = owner {
            state.borrow_and_update();
            if control.send(ChildControl::Kill).is_err() {
                return;
            }
            if matches!(
                wait_for_owner_state(&mut state, Some(Duration::from_secs(3)), false).await,
                Some(ChildOwnerState::Exited)
            ) {
                let _ = self.take_active(request_id).await;
            }
        }
    }

    async fn take_active(&self, request_id: u64) -> Option<ActiveResolverChild> {
        let mut active = self.active_child.lock().await;
        if active
            .as_ref()
            .is_some_and(|active| active.request_id == request_id)
        {
            active.take()
        } else {
            None
        }
    }
}

impl Drop for DownloadService {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_child.try_lock() {
            if let Some(active) = active.take() {
                let _ = active.control.send(ChildControl::Kill);
            }
        }
        if let Ok(mut workers) = self.workers.try_lock() {
            for worker in workers.drain(..) {
                worker.abort();
            }
        }
        if let Ok(cancellations) = self.cancellations.try_lock() {
            for cancellation in cancellations.values() {
                cancellation.cancel();
            }
        }
    }
}

fn spawn_child_owner(
    child: Child,
    browser_pid_file: PathBuf,
    lease: BrowserOperationLease,
) -> (
    std::sync::mpsc::Sender<ChildControl>,
    tokio::sync::watch::Receiver<ChildOwnerState>,
) {
    spawn_child_owner_with_process(child, browser_pid_file, lease)
}

fn spawn_child_owner_with_process<P: ResolverProcessControl>(
    mut process: P,
    browser_pid_file: PathBuf,
    lease: BrowserOperationLease,
) -> (
    std::sync::mpsc::Sender<ChildControl>,
    tokio::sync::watch::Receiver<ChildOwnerState>,
) {
    let (control, controls) = std::sync::mpsc::channel();
    let (state_tx, state) = tokio::sync::watch::channel(ChildOwnerState::Running);
    tokio::task::spawn_blocking(move || {
        #[cfg(debug_assertions)]
        if let Some(marker) = std::env::var_os("EMILY_TEST_SERVICE_CHILD_WAIT_MARKER") {
            let _ = fs::write(marker, std::process::id().to_string());
        }
        let mut recovery = false;
        loop {
            let control = if recovery {
                match controls.recv() {
                    Ok(control) => Some(control),
                    Err(_) => loop {
                        std::thread::park();
                    },
                }
            } else {
                match controls.try_recv() {
                    Ok(control) => Some(control),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(ChildControl::Kill),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                }
            };
            let observation = if matches!(control, Some(ChildControl::Kill)) {
                process.kill().and_then(|_| process.wait()).map(|_| true)
            } else {
                process.try_wait()
            };
            match observation {
                Ok(true) => break,
                Ok(false) => {
                    recovery = false;
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => {
                    let _ = state_tx.send(ChildOwnerState::TerminationFailed(termination_error()));
                    recovery = true;
                }
            }
        }
        #[cfg(debug_assertions)]
        if let Ok(milliseconds) = std::env::var("EMILY_TEST_SERVICE_EXIT_CONFIRMATION_DELAY_MS") {
            if let Ok(milliseconds) = milliseconds.parse::<u64>() {
                if let Some(marker) = std::env::var_os("EMILY_TEST_SERVICE_EXIT_DELAY_MARKER") {
                    let _ = fs::write(marker, std::process::id().to_string());
                }
                std::thread::sleep(Duration::from_millis(milliseconds));
            }
        }
        let _ = fs::remove_file(browser_pid_file);
        drop(lease);
        let _ = state_tx.send(ChildOwnerState::Exited);
    });
    (control, state)
}

async fn wait_for_owner_state(
    state: &mut tokio::sync::watch::Receiver<ChildOwnerState>,
    duration: Option<Duration>,
    include_current: bool,
) -> Option<ChildOwnerState> {
    let wait = async {
        if include_current && !matches!(*state.borrow(), ChildOwnerState::Running) {
            return Some(state.borrow().clone());
        }
        while state.changed().await.is_ok() {
            if !matches!(*state.borrow(), ChildOwnerState::Running) {
                return Some(state.borrow().clone());
            }
        }
        None
    };
    match duration {
        Some(duration) => tokio::time::timeout(duration, wait).await.ok().flatten(),
        None => wait.await,
    }
}

fn service_error(error: impl ToString) -> DownloadError {
    DownloadError {
        code: "download_resolver_failed".into(),
        message: error.to_string(),
    }
}

fn download_error(code: &str, message: &str) -> DownloadError {
    DownloadError {
        code: code.into(),
        message: message.into(),
    }
}

fn parse_quality(value: &str) -> DownloadQuality {
    match value {
        "highest" => DownloadQuality::Highest,
        "space_saver" => DownloadQuality::SpaceSaver,
        _ => DownloadQuality::Balanced,
    }
}

fn quality_name(value: DownloadQuality) -> &'static str {
    match value {
        DownloadQuality::Highest => "highest",
        DownloadQuality::Balanced => "balanced",
        DownloadQuality::SpaceSaver => "space_saver",
    }
}

fn asset_class(asset: &ResolvedAsset) -> AssetClass {
    match asset.asset_type.as_str() {
        "image" | "live-image" => AssetClass::Image,
        "audio" => AssetClass::Audio,
        _ => AssetClass::Video,
    }
}

fn source_is_expired(value: Option<&str>) -> bool {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|expires| expires <= chrono::Utc::now())
}

fn same_manifest_identity(
    original: &ResolvedMediaManifest,
    refreshed: &ResolvedMediaManifest,
) -> bool {
    original.aweme_id == refreshed.aweme_id
        && original.content_type == refreshed.content_type
        && original.assets.len() == refreshed.assets.len()
        && original
            .assets
            .iter()
            .zip(&refreshed.assets)
            .all(|(left, right)| {
                left.stable_key == right.stable_key
                    && left.asset_type == right.asset_type
                    && left.ordinal == right.ordinal
            })
}

fn open_service_database(path: &Path) -> Result<Connection, DownloadError> {
    let connection = Connection::open(path)
        .map_err(|_| download_error("download_storage_failed", "无法打开下载数据库"))?;
    configure_connection(&connection)
        .map_err(|_| download_error("download_storage_failed", "无法配置下载数据库"))?;
    assert_schema_latest(&connection)
        .map_err(|_| download_error("download_storage_failed", "下载数据库版本不兼容"))?;
    Ok(connection)
}

async fn scheduler_retry(delay: &mut Duration) {
    tokio::time::sleep(*delay).await;
    *delay = next_scheduler_retry_delay(*delay);
}

fn next_scheduler_retry_delay(delay: Duration) -> Duration {
    Duration::from_millis((delay.as_millis() as u64 * 2).min(400))
}

fn emergency_interrupt_job(
    path: &Path,
    job_id: i64,
    error_code: &str,
) -> Result<(), DownloadError> {
    let mut last_error = None;
    for _ in 0..3 {
        match open_service_database(path).and_then(|mut connection| {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| {
                    download_error("download_failure_persistence_failed", "无法开始下载中断事务")
                })?;
            let now = chrono::Utc::now().to_rfc3339();
            transaction.execute("UPDATE download_jobs SET state='interrupted', error_code=?2 WHERE id=?1 AND state IN ('queued','running')", rusqlite::params![job_id, error_code]).map_err(|_| download_error("download_failure_persistence_failed", "无法保存下载失败状态"))?;
            transaction.execute("UPDATE download_items SET state='interrupted', error_code=COALESCE(error_code,?2), error_message=COALESCE(error_message,'下载任务已中断'), updated_at=?3 WHERE job_id=?1 AND state IN ('resolving','downloading')", rusqlite::params![job_id, error_code, now]).map_err(|_| download_error("download_failure_persistence_failed", "无法保存下载条目中断状态"))?;
            transaction.execute("UPDATE download_assets SET state='interrupted', last_error_code=COALESCE(last_error_code,?2), updated_at=?3 WHERE item_id IN (SELECT id FROM download_items WHERE job_id=?1) AND state IN ('queued','downloading')", rusqlite::params![job_id, error_code, now]).map_err(|_| download_error("download_failure_persistence_failed", "无法保存下载资源中断状态"))?;
            transaction.commit().map_err(|_| {
                download_error("download_failure_persistence_failed", "无法提交下载中断事务")
            })
        }) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(last_error.expect("emergency interrupt records an error"))
}

fn validate_destination_root(root: &Path) -> Result<PathBuf, DownloadError> {
    if !root.is_absolute() {
        return Err(download_error(
            "download_destination_invalid",
            "下载目录必须是绝对路径",
        ));
    }
    fs::create_dir_all(root).map_err(|_| {
        download_error("download_destination_unavailable", "无法创建或访问下载目录")
    })?;
    let root = root
        .canonicalize()
        .map_err(|_| download_error("download_destination_unavailable", "无法验证下载目录"))?;
    if !root.is_dir() {
        return Err(download_error(
            "download_destination_invalid",
            "下载目标不是目录",
        ));
    }
    Ok(root)
}

fn query_jobs(connection: &Connection) -> Result<Vec<DownloadJobSummary>, DownloadError> {
    let mut statement = connection
        .prepare("SELECT id,account_id,state,destination_root,requested_quality,candidate_count,resolved_count,completed_count,skipped_count,failed_count,downloaded_bytes,total_bytes,created_at,started_at,error_code FROM download_jobs ORDER BY id DESC")
        .map_err(|_| download_error("download_storage_failed", "无法读取下载任务"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(DownloadJobSummary {
                id: row.get(0)?,
                account_id: row.get(1)?,
                state: row.get(2)?,
                destination_root: row.get(3)?,
                requested_quality: row.get(4)?,
                candidate_count: row.get(5)?,
                resolved_count: row.get(6)?,
                completed_count: row.get(7)?,
                skipped_count: row.get(8)?,
                failed_count: row.get(9)?,
                downloaded_bytes: row.get(10)?,
                total_bytes: row.get(11)?,
                created_at: row.get(12)?,
                started_at: row.get(13)?,
                error_code: row.get(14)?,
            })
        })
        .map_err(|_| download_error("download_storage_failed", "无法读取下载任务"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| download_error("download_storage_failed", "无法解析下载任务"))
}

fn query_job(connection: &Connection, job_id: i64) -> Result<DownloadJobDetail, DownloadError> {
    let job = connection
        .query_row("SELECT id,account_id,state,destination_root,requested_quality,candidate_count,resolved_count,completed_count,skipped_count,failed_count,downloaded_bytes,total_bytes,created_at,started_at,error_code FROM download_jobs WHERE id=?1", [job_id], |row| {
            Ok(DownloadJobSummary {
                id: row.get(0)?, account_id: row.get(1)?, state: row.get(2)?, destination_root: row.get(3)?,
                requested_quality: row.get(4)?, candidate_count: row.get(5)?,
                resolved_count: row.get(6)?, completed_count: row.get(7)?,
                skipped_count: row.get(8)?, failed_count: row.get(9)?,
                downloaded_bytes: row.get(10)?, total_bytes: row.get(11)?, created_at: row.get(12)?, started_at: row.get(13)?, error_code: row.get(14)?,
            })
        })
        .map_err(|_| download_error("download_job_not_found", "下载任务不存在"))?;
    let mut statement = connection
        .prepare("SELECT id,aweme_id,title,author_name,content_type,state,completed_asset_count,total_required_asset_count,error_code,error_message FROM download_items WHERE job_id=?1 ORDER BY id")
        .map_err(|_| download_error("download_storage_failed", "无法读取下载条目"))?;
    let items = statement
        .query_map([job_id], |row| {
            Ok(DownloadItemSummary {
                id: row.get(0)?,
                aweme_id: row.get(1)?,
                title: row.get(2)?,
                author_name: row.get(3)?,
                content_type: row.get(4)?,
                state: row.get(5)?,
                completed_asset_count: row.get(6)?,
                total_required_asset_count: row.get(7)?,
                error_code: row.get(8)?,
                error_message: row.get(9)?,
            })
        })
        .map_err(|_| download_error("download_storage_failed", "无法读取下载条目"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| download_error("download_storage_failed", "无法解析下载条目"))?;
    let mut statement = connection
        .prepare("SELECT a.id,a.item_id,a.stable_key,a.asset_type,a.state,a.downloaded_bytes,a.expected_size,a.retry_count,a.last_error_code,a.relative_final_path FROM download_assets a JOIN download_items i ON i.id=a.item_id WHERE i.job_id=?1 AND a.required=1 ORDER BY a.item_id,a.ordinal,a.id")
        .map_err(|_| download_error("download_storage_failed", "无法读取下载资源"))?;
    let assets = statement
        .query_map([job_id], |row| {
            Ok(DownloadAssetSummary {
                id: row.get(0)?,
                item_id: row.get(1)?,
                stable_key: row.get(2)?,
                asset_type: row.get(3)?,
                state: row.get(4)?,
                downloaded_bytes: row.get(5)?,
                expected_size: row.get(6)?,
                retry_count: row.get(7)?,
                last_error_code: row.get(8)?,
                relative_final_path: row.get(9)?,
            })
        })
        .map_err(|_| download_error("download_storage_failed", "无法读取下载资源"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| download_error("download_storage_failed", "无法解析下载资源"))?;
    let current_resolver_item_id = items
        .iter()
        .find(|item| item.state == "resolving")
        .map(|item| item.id);
    let active_asset_count = assets
        .iter()
        .filter(|asset| asset.state == "downloading")
        .count() as i64;
    let cancelled_asset_count = assets
        .iter()
        .filter(|asset| asset.state == "cancelled")
        .count() as i64;
    let transferred_bytes: i64 = assets
        .iter()
        .filter(|asset| asset.state != "skipped")
        .map(|asset| asset.downloaded_bytes.max(0))
        .sum();
    let average_bytes_per_second = job
        .started_at
        .as_deref()
        .and_then(|started| chrono::DateTime::parse_from_rfc3339(started).ok())
        .map(|started| {
            (chrono::Utc::now() - started.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(1)
        })
        .map_or(0, |seconds| transferred_bytes / seconds);
    Ok(DownloadJobDetail {
        job,
        items,
        assets,
        current_resolver_item_id,
        active_asset_count,
        cancelled_asset_count,
        average_bytes_per_second,
    })
}

fn query_reveal_target(
    connection: &Connection,
    job_id: Option<i64>,
    item_id: Option<i64>,
) -> Result<DownloadRevealTarget, DownloadError> {
    let (root, relative, select) = if let Some(job_id) = job_id {
        let root: String = connection
            .query_row(
                "SELECT destination_root FROM download_jobs WHERE id=?1",
                [job_id],
                |row| row.get(0),
            )
            .map_err(|_| download_error("download_job_not_found", "下载任务不存在"))?;
        (PathBuf::from(root), None, false)
    } else {
        let item_id = item_id.expect("validated reveal item id");
        let (root, relative): (String, String) = connection
            .query_row("SELECT j.destination_root,i.relative_work_directory FROM download_items i JOIN download_jobs j ON j.id=i.job_id WHERE i.id=?1", [item_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|_| download_error("download_item_not_found", "下载条目不存在"))?;
        (PathBuf::from(root), Some(PathBuf::from(relative)), true)
    };
    let root = root
        .canonicalize()
        .map_err(|_| download_error("download_path_missing", "下载目录不存在"))?;
    let path = match relative {
        Some(relative) => contained_path(&root, &relative)?,
        None => root.clone(),
    };
    let path = path
        .canonicalize()
        .map_err(|_| download_error("download_path_missing", "下载路径不存在"))?;
    if !path.starts_with(&root) {
        return Err(download_error(
            "download_path_unsafe",
            "下载路径超出任务目录",
        ));
    }
    Ok(DownloadRevealTarget { path, select })
}

fn termination_error() -> DownloadError {
    DownloadError {
        code: "download_resolver_termination_failed".into(),
        message: "无法确认解析器子进程已退出".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn scheduler_retry_delay_doubles_and_caps_at_four_hundred_milliseconds() {
        let mut delay = Duration::from_millis(25);
        let mut observed = Vec::new();
        for _ in 0..6 {
            delay = next_scheduler_retry_delay(delay);
            observed.push(delay);
        }
        assert_eq!(
            observed,
            vec![
                Duration::from_millis(50),
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::from_millis(400),
                Duration::from_millis(400),
                Duration::from_millis(400),
            ]
        );
    }

    struct KillFailsOnce {
        kill_calls: Arc<AtomicUsize>,
    }

    impl ResolverProcessControl for KillFailsOnce {
        fn try_wait(&mut self) -> std::io::Result<bool> {
            Ok(false)
        }
        fn kill(&mut self) -> std::io::Result<()> {
            if self.kill_calls.fetch_add(1, Ordering::AcqRel) == 0 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "kill denied",
                ))
            } else {
                Ok(())
            }
        }
        fn wait(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct TryWaitFailsOnce {
        try_calls: Arc<AtomicUsize>,
    }

    impl ResolverProcessControl for TryWaitFailsOnce {
        fn try_wait(&mut self) -> std::io::Result<bool> {
            if self.try_calls.fetch_add(1, Ordering::AcqRel) == 0 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "try_wait failed",
                ))
            } else {
                Ok(false)
            }
        }
        fn kill(&mut self) -> std::io::Result<()> {
            Ok(())
        }
        fn wait(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn fixture_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("emily-owner-{label}-{}", std::process::id()))
    }

    async fn wait_state(
        state: &mut tokio::sync::watch::Receiver<ChildOwnerState>,
        predicate: impl Fn(&ChildOwnerState) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if predicate(&state.borrow().clone()) {
                    return;
                }
                state.changed().await.expect("owner state sender");
            }
        })
        .await
        .expect("owner state timeout");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kill_error_retains_recovery_ownership_until_confirmed_retry_exit() {
        let pid = fixture_path("kill-error.pid");
        fs::write(&pid, b"pid").unwrap();
        let coordinator = BrowserOperationCoordinator::default();
        let lease = coordinator.acquire().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let (control, mut state) = spawn_child_owner_with_process(
            KillFailsOnce {
                kill_calls: Arc::clone(&calls),
            },
            pid.clone(),
            lease,
        );

        control.send(ChildControl::Kill).unwrap();
        wait_state(&mut state, |state| {
            matches!(state, ChildOwnerState::TerminationFailed(_))
        })
        .await;
        let ChildOwnerState::TerminationFailed(error) = state.borrow().clone() else {
            unreachable!()
        };
        assert_eq!(error.code, "download_resolver_termination_failed");
        assert!(pid.exists());
        assert!(coordinator.acquire().is_err());
        let after_failure = calls.load(Ordering::Acquire);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            calls.load(Ordering::Acquire),
            after_failure,
            "owner busy-looped after failure"
        );

        control.send(ChildControl::Kill).unwrap();
        wait_state(&mut state, |state| matches!(state, ChildOwnerState::Exited)).await;
        assert!(!pid.exists());
        assert!(coordinator.acquire().is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn try_wait_error_retains_recovery_ownership_until_confirmed_kill_exit() {
        let pid = fixture_path("try-wait-error.pid");
        fs::write(&pid, b"pid").unwrap();
        let coordinator = BrowserOperationCoordinator::default();
        let lease = coordinator.acquire().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let (control, mut state) = spawn_child_owner_with_process(
            TryWaitFailsOnce {
                try_calls: Arc::clone(&calls),
            },
            pid.clone(),
            lease,
        );

        wait_state(&mut state, |state| {
            matches!(state, ChildOwnerState::TerminationFailed(_))
        })
        .await;
        let ChildOwnerState::TerminationFailed(error) = state.borrow().clone() else {
            unreachable!()
        };
        assert_eq!(error.code, "download_resolver_termination_failed");
        assert!(pid.exists());
        assert!(coordinator.acquire().is_err());
        let after_failure = calls.load(Ordering::Acquire);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            calls.load(Ordering::Acquire),
            after_failure,
            "owner busy-looped after failure"
        );

        control.send(ChildControl::Kill).unwrap();
        wait_state(&mut state, |state| matches!(state, ChildOwnerState::Exited)).await;
        assert!(!pid.exists());
        assert!(coordinator.acquire().is_ok());
    }

    #[test]
    fn destination_snapshot_rejects_relative_paths() {
        let error = validate_destination_root(Path::new("downloads")).unwrap_err();
        assert_eq!(error.code, "download_destination_invalid");
    }

    #[test]
    fn destination_snapshot_creates_and_canonicalizes_an_absolute_directory() {
        let root =
            std::env::temp_dir().join(format!("emily-service-destination-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let validated = validate_destination_root(&root).unwrap();
        assert!(validated.is_absolute());
        assert!(validated.is_dir());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn emergency_interrupt_rolls_back_job_item_and_asset_together() {
        let database = fixture_path("emergency-interrupt.sqlite");
        let _ = fs::remove_file(&database);
        let mut connection = Connection::open(&database).unwrap();
        crate::migrate_schema_to_latest_at_startup(&mut connection).unwrap();
        connection.execute("INSERT INTO accounts (uid, profile_key, status, created_at, updated_at) VALUES ('emergency-interrupt','0123456789abcdef0123456789abcdef','active','now','now')", []).unwrap();
        let account_id = connection.last_insert_rowid();
        connection.execute("INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)", [account_id]).unwrap();
        connection.execute("INSERT INTO download_jobs (account_id,profile_key,created_at,selection_scope,destination_root,requested_quality,state,candidate_count) VALUES (?1,'0123456789abcdef0123456789abcdef','now','all','C:/downloads','balanced','running',1)", [account_id]).unwrap();
        let job_id = connection.last_insert_rowid();
        connection.execute("INSERT INTO download_items (job_id,aweme_id,canonical_url,content_type,relative_work_directory,state,created_at,updated_at) VALUES (?1,'1001','https://www.douyin.com/note/1001','note','1001','downloading','now','now')", [job_id]).unwrap();
        let item_id = connection.last_insert_rowid();
        connection.execute("INSERT INTO download_assets (item_id,stable_key,asset_type,ordinal,requested_profile,relative_final_path,relative_part_path,manifest_generation,state,created_at,updated_at) VALUES (?1,'image:0001','image',1,'balanced','1.jpg','.1.jpg.part',1,'downloading','now','now')", [item_id]).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_emergency_item BEFORE UPDATE OF state ON download_items WHEN NEW.state='interrupted' BEGIN SELECT RAISE(ABORT, 'reject item interrupt'); END;").unwrap();
        drop(connection);

        assert!(
            emergency_interrupt_job(&database, job_id, "download_failure_persistence_failed")
                .is_err()
        );

        let connection = Connection::open(&database).unwrap();
        let states: (String, String, String) = connection
            .query_row(
                "SELECT j.state,i.state,a.state FROM download_jobs j JOIN download_items i ON i.job_id=j.id JOIN download_assets a ON a.item_id=i.id WHERE j.id=?1",
                [job_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            states,
            ("running".into(), "downloading".into(), "downloading".into())
        );
        drop(connection);
        let _ = fs::remove_file(database);
    }
}
