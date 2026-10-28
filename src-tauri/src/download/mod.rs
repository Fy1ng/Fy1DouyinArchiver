pub mod browser_adapter;
pub mod catalog;
pub mod contracts;
pub mod errors;
pub mod paths;
pub mod persistence;
pub mod quality;
pub mod queue;
pub mod recovery;
pub mod resolver;
pub mod resolver_ipc;
pub mod service;
pub mod storage;
pub mod worker;

pub use browser_adapter::{parse_resolver_child_args, run_resolver_child, ResolverChildOptions};
pub use catalog::{expand_candidates, DownloadSelection};
pub use contracts::{
    stable_asset_key, DownloadAssetState, DownloadCandidate, DownloadItemState, DownloadJobState,
    DownloadQuality, ResolvedAsset, ResolvedFormat, ResolvedMediaManifest,
};
pub use errors::DownloadError;
pub use paths::{
    asset_ownership_key, build_work_directory, contained_path, partial_filename,
    sanitize_component, FileObservation, FilesystemRootAccess, GuardedPath, ResumeFile, RootAccess,
};
pub use persistence::{DownloadEvent, DownloadJobSnapshot, DownloadMutation, PersistenceHandle};
pub use quality::select_format;
pub use queue::DownloadQueue;
pub use recovery::{recover_download_jobs, RecoverySummary};
pub use resolver::{parse_detail_response, parse_manifest, MediaResolver};
pub use resolver_ipc::{
    read_frame, write_frame, ResolverControl, ResolverRequest, ResolverResponse,
    MAX_RESOLVER_FRAME_SIZE,
};
pub use service::{
    BrowserOperationCoordinator, BrowserOperationLease, DownloadAssetSummary,
    DownloadAssetTransfer, DownloadFaultInjector, DownloadFaultPoint, DownloadItemSummary,
    DownloadJobDetail, DownloadJobResolver, DownloadJobSummary, DownloadPreview,
    DownloadRefreshEvent, DownloadRevealTarget, DownloadRuntimeConfig, DownloadService,
};
pub use storage::{
    create_download_job, reconcile_manifest, AssetAction, CreateDownloadJobRequest,
    DownloadJobOwner,
};
pub use worker::{
    AddressPolicy, AddressResolution, AssetClass, CompletedAsset, DownloadWorker,
    DownloadWorkerBuilder, JobTransferQuota, PublicInternetPolicy, TransferLimits, TransferRequest,
};
