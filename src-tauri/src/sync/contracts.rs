use crate::{
    device_settings::BrowserPreferenceV1,
    settings_store::{ArchiveListRangeV1, HistoryRangeV1, RemoteFolderScheduleModeV1},
    CollectorSuccessV3,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

pub fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SyncSourceV1 {
    History,
    Likes,
    Favorites,
    RemoteFolder,
}

impl SyncSourceV1 {
    pub fn sqlite_name(self) -> &'static str {
        match self {
            Self::History => "history",
            Self::Likes => "likes",
            Self::Favorites => "favorites",
            Self::RemoteFolder => "remote_folder",
        }
    }

    pub fn from_sqlite(value: &str) -> Result<Self, String> {
        match value {
            "history" => Ok(Self::History),
            "likes" => Ok(Self::Likes),
            "favorites" => Ok(Self::Favorites),
            "remote_folder" => Ok(Self::RemoteFolder),
            _ => Err("invalid_sync_source".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SyncScopeV1 {
    One,
    CurrentAccountAvailableSources,
    AllActiveAccounts,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SyncTriggerV1 {
    Manual,
    Automatic,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SyncJobStatusV1 {
    Queued,
    Running,
    Success,
    Partial,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SyncBatchStatusV1 {
    Queued,
    Running,
    Success,
    Partial,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SyncRangeKindV1 {
    Days,
    Bottom,
    All,
    RecentItems,
    RecordLimit,
    RecentDays,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SyncBatchIssuePhaseV1 {
    InitialExpansion,
    DiscoveryExpansion,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SyncJobIssueSeverityV1 {
    Warning,
    Incomplete,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveRunModeV1 {
    Incremental,
    FullReconcile,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum BrowserIdV1 {
    Edge,
    Chrome,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteFolderExecutionOverrideV1 {
    pub range: ArchiveListRangeV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserExecutionIdentityV1 {
    pub preference: BrowserPreferenceV1,
    pub resolved_browser: BrowserIdV1,
    pub profile_key: String,
    pub profile_generation: String,
    pub executable_path_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RangeFreezeV1 {
    pub frozen_at_ms: i64,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub cutoff_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "source",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SyncTaskPayloadV1 {
    History {
        range: HistoryRangeV1,
        range_freeze: RangeFreezeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
    Likes {
        range: ArchiveListRangeV1,
        range_freeze: RangeFreezeV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
    Favorites {
        range: ArchiveListRangeV1,
        range_freeze: RangeFreezeV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
        discover_remote_folders: bool,
        remote_folder_mode: RemoteFolderScheduleModeV1,
        remote_folder_default_range: ArchiveListRangeV1,
        remote_folder_overrides: BTreeMap<String, RemoteFolderExecutionOverrideV1>,
    },
    RemoteFolder {
        remote_folder_id: String,
        range: ArchiveListRangeV1,
        range_freeze: RangeFreezeV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
}

impl SyncTaskPayloadV1 {
    pub fn source(&self) -> SyncSourceV1 {
        match self {
            Self::History { .. } => SyncSourceV1::History,
            Self::Likes { .. } => SyncSourceV1::Likes,
            Self::Favorites { .. } => SyncSourceV1::Favorites,
            Self::RemoteFolder { .. } => SyncSourceV1::RemoteFolder,
        }
    }

    pub fn source_instance_key(&self) -> &str {
        match self {
            Self::RemoteFolder {
                remote_folder_id, ..
            } => remote_folder_id,
            Self::History { .. } | Self::Likes { .. } | Self::Favorites { .. } => "",
        }
    }

    pub fn run_mode(&self) -> Option<ArchiveRunModeV1> {
        match self {
            Self::History { .. } => None,
            Self::Likes { run_mode, .. }
            | Self::Favorites { run_mode, .. }
            | Self::RemoteFolder { run_mode, .. } => Some(*run_mode),
        }
    }

    pub fn archive_range(&self) -> Option<ArchiveListRangeV1> {
        match self {
            Self::History { .. } => None,
            Self::Likes { range, .. }
            | Self::Favorites { range, .. }
            | Self::RemoteFolder { range, .. } => Some(*range),
        }
    }

    pub fn browser(&self) -> &BrowserExecutionIdentityV1 {
        match self {
            Self::History { browser, .. }
            | Self::Likes { browser, .. }
            | Self::Favorites { browser, .. }
            | Self::RemoteFolder { browser, .. } => browser,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveRangeEquivalenceV1 {
    pub kind: SyncRangeKindV1,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub days: Option<u32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub count: Option<u32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub limit: Option<u32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub cutoff_ms: Option<i64>,
}

impl EffectiveRangeEquivalenceV1 {
    pub fn all() -> Self {
        Self {
            kind: SyncRangeKindV1::All,
            days: None,
            count: None,
            limit: None,
            cutoff_ms: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteFolderRangeEquivalenceV1 {
    pub remote_folder_id: String,
    pub range: EffectiveRangeEquivalenceV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomaticScheduleConfigV1 {
    pub enabled: bool,
    pub interval_minutes: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "source",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AutomaticSyncConfigV1 {
    History {
        schema_version: u32,
        source_instance_key: String,
        schedule: AutomaticScheduleConfigV1,
        range: HistoryRangeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser_preference: BrowserPreferenceV1,
    },
    Likes {
        schema_version: u32,
        source_instance_key: String,
        schedule: AutomaticScheduleConfigV1,
        range: ArchiveListRangeV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser_preference: BrowserPreferenceV1,
    },
    Favorites {
        schema_version: u32,
        source_instance_key: String,
        schedule: AutomaticScheduleConfigV1,
        range: ArchiveListRangeV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser_preference: BrowserPreferenceV1,
        discover_remote_folders: bool,
    },
    RemoteFolder {
        schema_version: u32,
        source_instance_key: String,
        schedule: AutomaticScheduleConfigV1,
        range: ArchiveListRangeV1,
        run_mode: ArchiveRunModeV1,
        remote_folder_mode: RemoteFolderScheduleModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser_preference: BrowserPreferenceV1,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "source",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SyncExecutionEquivalenceV1 {
    History {
        schema_version: u32,
        source_instance_key: String,
        range: EffectiveRangeEquivalenceV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
    Likes {
        schema_version: u32,
        source_instance_key: String,
        range: EffectiveRangeEquivalenceV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
    Favorites {
        schema_version: u32,
        source_instance_key: String,
        range: EffectiveRangeEquivalenceV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
        discover_remote_folders: bool,
        remote_folder_mode: RemoteFolderScheduleModeV1,
        remote_folder_default_range: EffectiveRangeEquivalenceV1,
        remote_folder_overrides: Vec<RemoteFolderRangeEquivalenceV1>,
    },
    RemoteFolder {
        schema_version: u32,
        source_instance_key: String,
        range: EffectiveRangeEquivalenceV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenTaskPayloadV1 {
    pub payload: SyncTaskPayloadV1,
    pub payload_json: String,
    pub payload_sha256: String,
    pub equivalence: SyncExecutionEquivalenceV1,
    pub equivalence_json: String,
    pub equivalence_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskKeyV1 {
    pub account_id: i64,
    pub source: SyncSourceV1,
    pub source_instance_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncJobSnapshotV1 {
    pub id: i64,
    pub task_key: TaskKeyV1,
    pub trigger: SyncTriggerV1,
    pub status: SyncJobStatusV1,
    pub task_payload_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnqueueSyncRequestV1 {
    pub scope: SyncScopeV1,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub account_id: Option<i64>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub source: Option<SyncSourceV1>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub source_instance_key: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub payload: Option<SyncTaskPayloadV1>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub full_reconcile_confirmation_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncBatchCountsV1 {
    pub jobs: u32,
    pub skipped: u32,
    pub conflicts: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncJobIssueV1 {
    pub phase: String,
    pub code: String,
    pub severity: SyncJobIssueSeverityV1,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncJobViewV1 {
    pub id: i64,
    pub account_id: i64,
    pub source: SyncSourceV1,
    pub source_instance_key: String,
    pub trigger: SyncTriggerV1,
    pub status: SyncJobStatusV1,
    pub queued_at: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub scheduled_for: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub started_at: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub finished_at: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub queue_position: Option<u32>,
    pub can_cancel: bool,
    pub issues: Vec<SyncJobIssueV1>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncSkipV1 {
    pub account_id: i64,
    pub source: SyncSourceV1,
    pub source_instance_key: String,
    pub phase: SyncBatchIssuePhaseV1,
    pub reason: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub requested_payload_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncConflictV1 {
    pub account_id: i64,
    pub source: SyncSourceV1,
    pub source_instance_key: String,
    pub phase: SyncBatchIssuePhaseV1,
    pub reason: String,
    pub requested_payload_sha256: String,
    pub existing_payload_sha256: String,
    pub existing_job_at_conflict: SyncJobViewV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum SyncBatchEntryV1 {
    Job(SyncJobViewV1),
    Skipped(SyncSkipV1),
    Conflict(SyncConflictV1),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnqueueSyncResultV1 {
    #[serde(deserialize_with = "deserialize_required_option")]
    pub batch_id: Option<i64>,
    pub counts: SyncBatchCountsV1,
    pub entries: Vec<SyncBatchEntryV1>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncStatusV2 {
    pub account_id: i64,
    pub job_id: i64,
    pub source: SyncSourceV1,
    pub source_instance_key: String,
    pub trigger: SyncTriggerV1,
    pub status: SyncJobStatusV1,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub queue_position: Option<u32>,
    pub inserted_videos: i64,
    pub inserted_records: i64,
    pub inserted_watch_records: i64,
    pub record_count: i64,
    pub elapsed_ms: i64,
    pub retry_count: i64,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub phase: Option<String>,
    pub can_cancel: bool,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub started_at: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub finished_at: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub error_code: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub error_message: Option<String>,
    pub issues: Vec<SyncJobIssueV1>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncBatchViewV1 {
    pub id: i64,
    pub trigger: SyncTriggerV1,
    pub scope: SyncScopeV1,
    pub status: SyncBatchStatusV1,
    pub created_at: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub finished_at: Option<String>,
    pub counts: SyncBatchCountsV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncBatchesPageV1 {
    pub records: Vec<SyncBatchViewV1>,
    pub total: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncBatchEntriesPageV1 {
    pub batch_id: i64,
    pub entries: Vec<SyncBatchEntryV1>,
    pub next_cursor: String,
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveItemV1 {
    pub aweme_id: String,
    pub canonical_url: String,
    pub content_type: String,
    pub source_position: i64,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub remote_created_at_ms: Option<i64>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub title: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub author_name: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub cover_url: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub like_count: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ArchiveCompleteStopV1 {
    Bottom,
    RequestedItemCount {
        count: u32,
    },
    RecordLimit {
        limit: u32,
    },
    DateBoundary {
        cutoff_ms: i64,
        guard_aweme_id: String,
        guard_remote_created_at_ms: i64,
    },
    StableKnownBoundary {
        consecutive_known: u32,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ArchiveIncompleteReasonV1 {
    SafetyRecordCap { cap: u32 },
    UnconfirmedEnd,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "status",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ArchiveCompletionV1 {
    Complete { stop: ArchiveCompleteStopV1 },
    Incomplete { reason: ArchiveIncompleteReasonV1 },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveCollectionSuccessV1 {
    pub records: Vec<ArchiveItemV1>,
    pub identity_hash: String,
    pub field_hash: String,
    pub completion: ArchiveCompletionV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteFolderDescriptorV1 {
    pub remote_folder_id: String,
    pub name: String,
    pub source_position: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteFolderDiscoverySuccessV1 {
    pub folders: Vec<RemoteFolderDescriptorV1>,
    pub identity_hash: String,
    pub field_hash: String,
    pub complete_scan: bool,
    pub bottom_confirmed: bool,
    pub hit_record_limit: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "status",
    content = "data",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum RemoteFolderDiscoveryResultV1 {
    NotRequested,
    Success(RemoteFolderDiscoverySuccessV1),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FavoriteCollectionSuccessV1 {
    pub archive: ArchiveCollectionSuccessV1,
    pub remote_folder_discovery: RemoteFolderDiscoveryResultV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteFolderCollectionSuccessV1 {
    pub remote_folder_id: String,
    pub name: String,
    pub archive: ArchiveCollectionSuccessV1,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(
    tag = "source",
    content = "data",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum SourceExecutionResultV1 {
    History(CollectorSuccessV3),
    Likes(ArchiveCollectionSuccessV1),
    Favorites(FavoriteCollectionSuccessV1),
    RemoteFolder(RemoteFolderCollectionSuccessV1),
}

impl SourceExecutionResultV1 {
    pub fn source(&self) -> SyncSourceV1 {
        match self {
            Self::History(_) => SyncSourceV1::History,
            Self::Likes(_) => SyncSourceV1::Likes,
            Self::Favorites(_) => SyncSourceV1::Favorites,
            Self::RemoteFolder(_) => SyncSourceV1::RemoteFolder,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncExecutionEnvelopeV1 {
    pub schema_version: i64,
    pub account_id: i64,
    pub job_id: i64,
    pub source: SyncSourceV1,
    pub source_instance_key: String,
    pub task_payload_sha256: String,
    pub result: SourceExecutionResultV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceCapabilityViewV1 {
    pub source: SyncSourceV1,
    pub available: bool,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub unavailable_reason: Option<String>,
    pub supported_ranges: Vec<SyncRangeKindV1>,
    pub supports_full_reconcile: bool,
    pub supports_remote_folders: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueuedSyncContext {
    pub job: SyncJobSnapshotV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunningSyncIdentity {
    pub job_id: i64,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub process_id: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportCountsV1 {
    pub inserted_videos: i64,
    pub inserted_records: i64,
    pub inserted_watch_records: i64,
}
