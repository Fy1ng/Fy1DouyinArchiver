use crate::bootstrap::{write_json_atomically, RetainedDataRootV1};
use serde::{de::IgnoredAny, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Mutex,
};

const SETTINGS_SCHEMA_VERSION_V1: i64 = 1;
const SETTINGS_SCHEMA_VERSION_V2: i64 = 2;
const SETTINGS_SCHEMA_VERSION_V3: i64 = 3;
const SETTINGS_SCHEMA_VERSION_V4: i64 = 4;
const SETTINGS_SCHEMA_VERSION_V5: i64 = 5;
const SETTINGS_SCHEMA_VERSION_V6: i64 = 6;
const SETTINGS_SCHEMA_VERSION: i64 = 7;
const SETTINGS_FILE: &str = "settings.json";
static SETTINGS_RMW_LOCK: Mutex<()> = Mutex::new(());

pub const DEFAULT_INTERVAL_MINUTES: u32 = 360;
pub const DEFAULT_STARTUP_SYNC_DELAY_MINUTES: u32 = 10;
pub const DEFAULT_MESSAGE_DURATION_SECONDS: u32 = 5;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LikeCountFormatV1 {
    Exact,
    W,
    Wan,
    K,
    Qian,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetailFieldsV1 {
    pub author: bool,
    pub watch_date: bool,
    pub like_count: bool,
    pub watch_progress: bool,
}

impl Default for DetailFieldsV1 {
    fn default() -> Self {
        Self {
            author: true,
            watch_date: true,
            like_count: true,
            watch_progress: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlobalSettingsV1 {
    pub like_count_format: LikeCountFormatV1,
    pub detail_fields: DetailFieldsV1,
}

impl Default for GlobalSettingsV1 {
    fn default() -> Self {
        Self {
            like_count_format: LikeCountFormatV1::Wan,
            detail_fields: DetailFieldsV1::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CloseBehaviorV1 {
    Tray,
    Exit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeModeV1 {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CoverCacheBackfillModeV1 {
    NewRecordsOnly,
    BackfillAll,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CoverCacheCleanupModeV1 {
    KeepListed,
    CleanListed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverCacheSettingsV1 {
    pub enabled: bool,
    pub backfill_mode: CoverCacheBackfillModeV1,
    pub capacity_limit_mib: u64,
    pub cleanup_by_age_enabled: bool,
    pub max_age_days: u32,
    pub cleanup_by_capacity_enabled: bool,
    pub cleanup_list_mode: CoverCacheCleanupModeV1,
}

impl Default for CoverCacheSettingsV1 {
    fn default() -> Self {
        Self {
            enabled: false,
            backfill_mode: CoverCacheBackfillModeV1::NewRecordsOnly,
            capacity_limit_mib: 2_048,
            cleanup_by_age_enabled: false,
            max_age_days: 90,
            cleanup_by_capacity_enabled: false,
            cleanup_list_mode: CoverCacheCleanupModeV1::KeepListed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupSettingsV1 {
    pub enabled: bool,
    pub interval_days: u32,
    pub execution_time: String,
    pub retention_count: u32,
}

impl Default for BackupSettingsV1 {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_days: 7,
            execution_time: "03:00".to_string(),
            retention_count: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationSettingsV1 {
    pub login_expired: bool,
    pub automatic_failure_streak: bool,
    pub backup_failure: bool,
    pub sync_completed: bool,
    pub backup_completed: bool,
    pub automatic_failure_threshold: u32,
    #[serde(default = "default_message_duration_seconds")]
    pub message_duration_seconds: u32,
}

impl Default for NotificationSettingsV1 {
    fn default() -> Self {
        Self {
            login_expired: true,
            automatic_failure_streak: true,
            backup_failure: true,
            sync_completed: false,
            backup_completed: false,
            automatic_failure_threshold: 3,
            message_duration_seconds: DEFAULT_MESSAGE_DURATION_SECONDS,
        }
    }
}

fn default_message_duration_seconds() -> u32 {
    DEFAULT_MESSAGE_DURATION_SECONDS
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateSettingsV1 {
    pub automatic_check: bool,
    pub automatic_download: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlobalSettingsV2 {
    pub theme: ThemeModeV1,
    pub like_count_format: LikeCountFormatV1,
    pub detail_fields: DetailFieldsV1,
    pub close_behavior: CloseBehaviorV1,
    pub startup_sync_delay_minutes: u32,
    pub cover_cache: CoverCacheSettingsV1,
    pub backup: BackupSettingsV1,
    pub notifications: NotificationSettingsV1,
    pub updates: UpdateSettingsV1,
}

impl Default for GlobalSettingsV2 {
    fn default() -> Self {
        let legacy = GlobalSettingsV1::default();
        Self {
            theme: ThemeModeV1::Dark,
            like_count_format: legacy.like_count_format,
            detail_fields: legacy.detail_fields,
            close_behavior: CloseBehaviorV1::Exit,
            startup_sync_delay_minutes: DEFAULT_STARTUP_SYNC_DELAY_MINUTES,
            cover_cache: CoverCacheSettingsV1::default(),
            backup: BackupSettingsV1::default(),
            notifications: NotificationSettingsV1::default(),
            updates: UpdateSettingsV1::default(),
        }
    }
}

impl From<GlobalSettingsV1> for GlobalSettingsV2 {
    fn from(value: GlobalSettingsV1) -> Self {
        Self {
            theme: ThemeModeV1::System,
            like_count_format: value.like_count_format,
            detail_fields: value.detail_fields,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SyncModeV1 {
    Days,
    Bottom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSettingsV1 {
    pub sync_mode: SyncModeV1,
    pub sync_days: u32,
    pub hide_browser: bool,
    pub scroll_speed: u32,
}

impl Default for AccountSettingsV1 {
    fn default() -> Self {
        Self {
            sync_mode: SyncModeV1::Days,
            sync_days: 7,
            hide_browser: false,
            scroll_speed: 6_000,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum HistoryRangeV1 {
    Days { days: u32 },
    Bottom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ArchiveListRangeV1 {
    All,
    RecentItems { count: u32 },
    RecordLimit { limit: u32 },
    RecentDays { days: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceScheduleV1 {
    pub enabled: bool,
    pub interval_minutes: u32,
}

impl Default for SourceScheduleV1 {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: DEFAULT_INTERVAL_MINUTES,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistorySourceSettingsV1 {
    pub range: HistoryRangeV1,
    pub schedule: SourceScheduleV1,
}

impl Default for HistorySourceSettingsV1 {
    fn default() -> Self {
        Self {
            range: HistoryRangeV1::Days { days: 7 },
            schedule: SourceScheduleV1::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveSourceSettingsV1 {
    pub range: ArchiveListRangeV1,
    pub schedule: SourceScheduleV1,
}

impl Default for ArchiveSourceSettingsV1 {
    fn default() -> Self {
        Self {
            range: ArchiveListRangeV1::RecentItems { count: 100 },
            schedule: SourceScheduleV1::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RemoteFolderScheduleModeV1 {
    Unified,
    PerFolder,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FavoriteSourceSettingsV1 {
    pub archive: ArchiveSourceSettingsV1,
    pub remote_folder_mode: RemoteFolderScheduleModeV1,
    pub remote_folder_defaults: ArchiveSourceSettingsV1,
    pub remote_folders: BTreeMap<String, ArchiveSourceSettingsV1>,
}

impl Default for FavoriteSourceSettingsV1 {
    fn default() -> Self {
        Self {
            archive: ArchiveSourceSettingsV1::default(),
            remote_folder_mode: RemoteFolderScheduleModeV1::Unified,
            remote_folder_defaults: ArchiveSourceSettingsV1::default(),
            remote_folders: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSettingsV2 {
    pub hide_browser: bool,
    pub scroll_speed: u32,
    pub history: HistorySourceSettingsV1,
    pub likes: ArchiveSourceSettingsV1,
    pub favorites: FavoriteSourceSettingsV1,
}

impl Default for AccountSettingsV2 {
    fn default() -> Self {
        Self::from(AccountSettingsV1::default())
    }
}

impl From<AccountSettingsV1> for AccountSettingsV2 {
    fn from(value: AccountSettingsV1) -> Self {
        let range = match value.sync_mode {
            SyncModeV1::Days => HistoryRangeV1::Days {
                days: value.sync_days,
            },
            SyncModeV1::Bottom => HistoryRangeV1::Bottom,
        };
        Self {
            hide_browser: value.hide_browser,
            scroll_speed: value.scroll_speed,
            history: HistorySourceSettingsV1 {
                range,
                schedule: SourceScheduleV1::default(),
            },
            likes: ArchiveSourceSettingsV1::default(),
            favorites: FavoriteSourceSettingsV1::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AccountScopeV1 {
    All,
    Selected { account_ids: Vec<i64> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PortableAccountScopeV1 {
    All,
    Selected { account_keys: Vec<String> },
}

impl Default for PortableAccountScopeV1 {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DefaultSyncModeV1 {
    SingleAccount,
    Aggregate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncWorkspaceSettingsV1 {
    pub mode: DefaultSyncModeV1,
    pub account_scope: PortableAccountScopeV1,
}

impl Default for SyncWorkspaceSettingsV1 {
    fn default() -> Self {
        Self {
            mode: DefaultSyncModeV1::SingleAccount,
            account_scope: PortableAccountScopeV1::All,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisplayScopesV1 {
    pub overview: PortableAccountScopeV1,
    pub library: PortableAccountScopeV1,
    pub search: PortableAccountScopeV1,
}

impl Default for DisplayScopesV1 {
    fn default() -> Self {
        Self {
            overview: PortableAccountScopeV1::All,
            library: PortableAccountScopeV1::All,
            search: PortableAccountScopeV1::All,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OverviewModuleIdV1 {
    SyncHealth,
    SourceTotals,
    RecentWatched,
    RecentArchived,
    SevenDayTrend,
    AnnualReviewBanner,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OverviewModuleWidthV1 {
    Half,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverviewModuleSettingsV1 {
    pub id: OverviewModuleIdV1,
    pub visible: bool,
    pub width: OverviewModuleWidthV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedOverviewLayoutV1 {
    pub account_scope: PortableAccountScopeV1,
    pub modules: Vec<OverviewModuleSettingsV1>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContentViewModeV1 {
    Horizontal,
    Grid,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContentSortPreferenceV1 {
    ArchivedNewest,
    WatchedNewest,
    WatchedOldest,
    PageOrder,
    RemoteNewest,
    RemoteOldest,
    LikeCountDesc,
    TitleAsc,
    AuthorAsc,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentViewPreferenceV1 {
    pub query: String,
    pub page: u32,
    pub view_mode: ContentViewModeV1,
    pub include_remote_removed: bool,
    pub sort: ContentSortPreferenceV1,
}

impl ContentViewPreferenceV1 {
    fn new(sort: ContentSortPreferenceV1) -> Self {
        Self {
            query: String::new(),
            page: 1,
            view_mode: ContentViewModeV1::Horizontal,
            include_remote_removed: false,
            sort,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentViewPreferencesV1 {
    pub all: ContentViewPreferenceV1,
    pub history: ContentViewPreferenceV1,
    pub likes: ContentViewPreferenceV1,
    pub favorites: ContentViewPreferenceV1,
}

impl Default for ContentViewPreferencesV1 {
    fn default() -> Self {
        Self {
            all: ContentViewPreferenceV1::new(ContentSortPreferenceV1::ArchivedNewest),
            history: ContentViewPreferenceV1::new(ContentSortPreferenceV1::WatchedNewest),
            likes: ContentViewPreferenceV1::new(ContentSortPreferenceV1::PageOrder),
            favorites: ContentViewPreferenceV1::new(ContentSortPreferenceV1::PageOrder),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSettingsV1 {
    pub sync: SyncWorkspaceSettingsV1,
    pub display_scopes: DisplayScopesV1,
    pub overview_layouts: Vec<ScopedOverviewLayoutV1>,
    pub content_views: ContentViewPreferencesV1,
    pub viewed_annual_review_years: BTreeSet<i32>,
}

impl Default for WorkspaceSettingsV1 {
    fn default() -> Self {
        Self {
            sync: SyncWorkspaceSettingsV1::default(),
            display_scopes: DisplayScopesV1::default(),
            overview_layouts: Vec::new(),
            content_views: ContentViewPreferencesV1::default(),
            viewed_annual_review_years: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyImportStateV1 {
    pub completed: bool,
    pub fingerprint: Option<String>,
}

impl Default for LegacyImportStateV1 {
    fn default() -> Self {
        Self {
            completed: false,
            fingerprint: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortableSettingsV1 {
    pub schema_version: i64,
    pub revision: u64,
    pub legacy_frontend_import: LegacyImportStateV1,
    pub global: GlobalSettingsV1,
    pub account_defaults: AccountSettingsV1,
    pub accounts: BTreeMap<String, AccountSettingsV1>,
}

impl Default for PortableSettingsV1 {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION_V1,
            revision: 0,
            legacy_frontend_import: LegacyImportStateV1::default(),
            global: GlobalSettingsV1::default(),
            account_defaults: AccountSettingsV1::default(),
            accounts: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortableSettingsV2 {
    pub schema_version: i64,
    pub revision: u64,
    pub legacy_frontend_import: LegacyImportStateV1,
    pub global: GlobalSettingsV2,
    pub workspace: WorkspaceSettingsV1,
    pub account_defaults: AccountSettingsV2,
    pub accounts: BTreeMap<String, AccountSettingsV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableSettingsSchemaV2 {
    schema_version: i64,
    revision: u64,
    legacy_frontend_import: LegacyImportStateV1,
    global: GlobalSettingsSchemaV2,
    account_defaults: AccountSettingsV2,
    accounts: BTreeMap<String, AccountSettingsV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableSettingsSchemaV3 {
    schema_version: i64,
    revision: u64,
    legacy_frontend_import: LegacyImportStateV1,
    global: GlobalSettingsSchemaV3,
    account_defaults: AccountSettingsV2,
    accounts: BTreeMap<String, AccountSettingsV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableSettingsSchemaV4 {
    schema_version: i64,
    revision: u64,
    legacy_frontend_import: LegacyImportStateV1,
    global: GlobalSettingsSchemaV4,
    account_defaults: AccountSettingsV2,
    accounts: BTreeMap<String, AccountSettingsV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableSettingsSchemaV5 {
    schema_version: i64,
    revision: u64,
    legacy_frontend_import: LegacyImportStateV1,
    global: GlobalSettingsSchemaV5,
    account_defaults: AccountSettingsV2,
    accounts: BTreeMap<String, AccountSettingsV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableSettingsSchemaV6 {
    schema_version: i64,
    revision: u64,
    legacy_frontend_import: LegacyImportStateV1,
    global: GlobalSettingsV2,
    account_defaults: AccountSettingsV2,
    accounts: BTreeMap<String, AccountSettingsV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GlobalSettingsSchemaV2 {
    like_count_format: LikeCountFormatV1,
    detail_fields: DetailFieldsV1,
    close_behavior: CloseBehaviorV1,
    startup_sync_delay_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GlobalSettingsSchemaV3 {
    like_count_format: LikeCountFormatV1,
    detail_fields: DetailFieldsV1,
    close_behavior: CloseBehaviorV1,
    startup_sync_delay_minutes: u32,
    cover_cache: CoverCacheSettingsV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GlobalSettingsSchemaV4 {
    theme: ThemeModeV1,
    like_count_format: LikeCountFormatV1,
    detail_fields: DetailFieldsV1,
    close_behavior: CloseBehaviorV1,
    startup_sync_delay_minutes: u32,
    cover_cache: CoverCacheSettingsV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GlobalSettingsSchemaV5 {
    theme: ThemeModeV1,
    like_count_format: LikeCountFormatV1,
    detail_fields: DetailFieldsV1,
    close_behavior: CloseBehaviorV1,
    startup_sync_delay_minutes: u32,
    cover_cache: CoverCacheSettingsV1,
    backup: BackupSettingsV1,
    notifications: NotificationSettingsV1,
}

impl From<PortableSettingsSchemaV2> for PortableSettingsV2 {
    fn from(value: PortableSettingsSchemaV2) -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            revision: value.revision,
            legacy_frontend_import: value.legacy_frontend_import,
            global: GlobalSettingsV2 {
                theme: ThemeModeV1::System,
                like_count_format: value.global.like_count_format,
                detail_fields: value.global.detail_fields,
                close_behavior: value.global.close_behavior,
                startup_sync_delay_minutes: value.global.startup_sync_delay_minutes,
                cover_cache: CoverCacheSettingsV1::default(),
                backup: BackupSettingsV1::default(),
                notifications: NotificationSettingsV1::default(),
                updates: UpdateSettingsV1::default(),
            },
            workspace: WorkspaceSettingsV1::default(),
            account_defaults: value.account_defaults,
            accounts: value.accounts,
        }
    }
}

impl From<PortableSettingsSchemaV3> for PortableSettingsV2 {
    fn from(value: PortableSettingsSchemaV3) -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            revision: value.revision,
            legacy_frontend_import: value.legacy_frontend_import,
            global: GlobalSettingsV2 {
                theme: ThemeModeV1::System,
                like_count_format: value.global.like_count_format,
                detail_fields: value.global.detail_fields,
                close_behavior: value.global.close_behavior,
                startup_sync_delay_minutes: value.global.startup_sync_delay_minutes,
                cover_cache: value.global.cover_cache,
                backup: BackupSettingsV1::default(),
                notifications: NotificationSettingsV1::default(),
                updates: UpdateSettingsV1::default(),
            },
            workspace: WorkspaceSettingsV1::default(),
            account_defaults: value.account_defaults,
            accounts: value.accounts,
        }
    }
}

impl From<PortableSettingsSchemaV4> for PortableSettingsV2 {
    fn from(value: PortableSettingsSchemaV4) -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            revision: value.revision,
            legacy_frontend_import: value.legacy_frontend_import,
            global: GlobalSettingsV2 {
                theme: value.global.theme,
                like_count_format: value.global.like_count_format,
                detail_fields: value.global.detail_fields,
                close_behavior: value.global.close_behavior,
                startup_sync_delay_minutes: value.global.startup_sync_delay_minutes,
                cover_cache: value.global.cover_cache,
                backup: BackupSettingsV1::default(),
                notifications: NotificationSettingsV1::default(),
                updates: UpdateSettingsV1::default(),
            },
            workspace: WorkspaceSettingsV1::default(),
            account_defaults: value.account_defaults,
            accounts: value.accounts,
        }
    }
}

impl From<PortableSettingsSchemaV5> for PortableSettingsV2 {
    fn from(value: PortableSettingsSchemaV5) -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            revision: value.revision,
            legacy_frontend_import: value.legacy_frontend_import,
            global: GlobalSettingsV2 {
                theme: value.global.theme,
                like_count_format: value.global.like_count_format,
                detail_fields: value.global.detail_fields,
                close_behavior: value.global.close_behavior,
                startup_sync_delay_minutes: value.global.startup_sync_delay_minutes,
                cover_cache: value.global.cover_cache,
                backup: value.global.backup,
                notifications: value.global.notifications,
                updates: UpdateSettingsV1::default(),
            },
            workspace: WorkspaceSettingsV1::default(),
            account_defaults: value.account_defaults,
            accounts: value.accounts,
        }
    }
}

impl From<PortableSettingsSchemaV6> for PortableSettingsV2 {
    fn from(value: PortableSettingsSchemaV6) -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            revision: value.revision,
            legacy_frontend_import: value.legacy_frontend_import,
            global: value.global,
            workspace: WorkspaceSettingsV1::default(),
            account_defaults: value.account_defaults,
            accounts: value.accounts,
        }
    }
}

impl Default for PortableSettingsV2 {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            revision: 0,
            legacy_frontend_import: LegacyImportStateV1::default(),
            global: GlobalSettingsV2::default(),
            workspace: WorkspaceSettingsV1::default(),
            account_defaults: AccountSettingsV2::default(),
            accounts: BTreeMap::new(),
        }
    }
}

impl From<PortableSettingsV1> for PortableSettingsV2 {
    fn from(value: PortableSettingsV1) -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            revision: value.revision,
            legacy_frontend_import: value.legacy_frontend_import,
            global: value.global.into(),
            workspace: WorkspaceSettingsV1::default(),
            account_defaults: value.account_defaults.into(),
            accounts: value
                .accounts
                .into_iter()
                .map(|(key, account)| (key, account.into()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyFrontendSettingsV1 {
    pub sync_mode: SyncModeV1,
    pub sync_days: u32,
    pub scroll_speed: u32,
    pub hide_browser: bool,
    pub like_count_format: LikeCountFormatV1,
    pub detail_fields: DetailFieldsV1,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsView {
    pub revision: u64,
    pub data_root: PathBuf,
    pub install_dir: PathBuf,
    pub retained_data_roots: Vec<RetainedDataRootV1>,
    pub legacy_import_required: bool,
    pub global: GlobalSettingsV2,
    pub workspace: WorkspaceSettingsV1,
    pub account_defaults: AccountSettingsV2,
    pub account: AccountSettingsV2,
}

impl Default for LegacyFrontendSettingsV1 {
    fn default() -> Self {
        let account = AccountSettingsV1::default();
        let global = GlobalSettingsV1::default();
        Self {
            sync_mode: account.sync_mode,
            sync_days: account.sync_days,
            scroll_speed: account.scroll_speed,
            hide_browser: account.hide_browser,
            like_count_format: global.like_count_format,
            detail_fields: global.detail_fields,
        }
    }
}

pub fn portable_account_key(uid: Option<&str>, sec_uid: Option<&str>) -> Result<String, String> {
    if let Some(value) = normalized_identity(sec_uid) {
        return Ok(format!("sec:{value}"));
    }
    if let Some(value) = normalized_identity(uid) {
        return Ok(format!("uid:{value}"));
    }
    Err("portable_account_identity_missing".to_string())
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsVersionEnvelope {
    schema_version: i64,
    revision: IgnoredAny,
    legacy_frontend_import: IgnoredAny,
    global: IgnoredAny,
    #[serde(default)]
    workspace: Option<IgnoredAny>,
    account_defaults: IgnoredAny,
    accounts: IgnoredAny,
}

fn settings_schema_version(bytes: &[u8]) -> Result<i64, String> {
    serde_json::from_slice::<SettingsVersionEnvelope>(bytes)
        .map(|value| value.schema_version)
        .map_err(invalid_portable_settings)
}

fn invalid_portable_settings(error: serde_json::Error) -> String {
    format!("invalid_portable_settings:{error}")
}

pub fn load_portable_settings(root: &Path) -> Result<PortableSettingsV2, String> {
    let _guard = settings_rmw_guard()?;
    load_portable_settings_locked(root)
}

fn load_portable_settings_locked(root: &Path) -> Result<PortableSettingsV2, String> {
    let path = root.join(SETTINGS_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(PortableSettingsV2::default())
        }
        Err(error) => return Err(error.to_string()),
    };
    let schema_version = settings_schema_version(&bytes)?;
    let value = decode_portable_settings_bytes_with_version(&bytes, schema_version)?;
    if schema_version != SETTINGS_SCHEMA_VERSION {
        write_portable_settings_atomic_locked(root, &value)?;
    }
    Ok(value)
}

pub fn decode_portable_settings_bytes(bytes: &[u8]) -> Result<PortableSettingsV2, String> {
    decode_portable_settings_bytes_with_version(bytes, settings_schema_version(bytes)?)
}

fn decode_portable_settings_bytes_with_version(
    bytes: &[u8],
    schema_version: i64,
) -> Result<PortableSettingsV2, String> {
    match schema_version {
        SETTINGS_SCHEMA_VERSION_V1 => {
            let legacy = serde_json::from_slice::<PortableSettingsV1>(&bytes)
                .map_err(invalid_portable_settings)?;
            validate_settings_v1(&legacy)?;
            let migrated = PortableSettingsV2::from(legacy);
            validate_settings(&migrated)?;
            Ok(migrated)
        }
        SETTINGS_SCHEMA_VERSION_V2 => {
            let legacy = serde_json::from_slice::<PortableSettingsSchemaV2>(&bytes)
                .map_err(invalid_portable_settings)?;
            let migrated = PortableSettingsV2::from(legacy);
            validate_settings(&migrated)?;
            Ok(migrated)
        }
        SETTINGS_SCHEMA_VERSION_V3 => {
            let legacy = serde_json::from_slice::<PortableSettingsSchemaV3>(&bytes)
                .map_err(invalid_portable_settings)?;
            let migrated = PortableSettingsV2::from(legacy);
            validate_settings(&migrated)?;
            Ok(migrated)
        }
        SETTINGS_SCHEMA_VERSION_V4 => {
            let legacy = serde_json::from_slice::<PortableSettingsSchemaV4>(&bytes)
                .map_err(invalid_portable_settings)?;
            let migrated = PortableSettingsV2::from(legacy);
            validate_settings(&migrated)?;
            Ok(migrated)
        }
        SETTINGS_SCHEMA_VERSION_V5 => {
            let legacy = serde_json::from_slice::<PortableSettingsSchemaV5>(&bytes)
                .map_err(invalid_portable_settings)?;
            let migrated = PortableSettingsV2::from(legacy);
            validate_settings(&migrated)?;
            Ok(migrated)
        }
        SETTINGS_SCHEMA_VERSION_V6 => {
            let legacy = serde_json::from_slice::<PortableSettingsSchemaV6>(&bytes)
                .map_err(invalid_portable_settings)?;
            let migrated = PortableSettingsV2::from(legacy);
            validate_settings(&migrated)?;
            Ok(migrated)
        }
        SETTINGS_SCHEMA_VERSION => {
            let value = serde_json::from_slice::<PortableSettingsV2>(&bytes)
                .map_err(invalid_portable_settings)?;
            validate_settings(&value)?;
            Ok(value)
        }
        _ => Err("unsupported_portable_settings_schema".to_string()),
    }
}

pub fn write_portable_settings_atomic(
    root: &Path,
    value: &PortableSettingsV2,
) -> Result<(), String> {
    let _guard = settings_rmw_guard()?;
    write_portable_settings_atomic_locked(root, value)
}

pub fn compare_and_swap_portable_settings(
    root: &Path,
    expected_revision: u64,
    update: impl FnOnce(&mut PortableSettingsV2) -> Result<(), String>,
) -> Result<PortableSettingsV2, String> {
    let _guard = settings_rmw_guard()?;
    let mut settings = load_portable_settings_locked(root)?;
    if settings.revision != expected_revision {
        return Err("portable_settings_revision_conflict".to_string());
    }
    update(&mut settings)?;
    if settings.revision != expected_revision {
        return Err("invalid_portable_settings".to_string());
    }
    increment_revision(&mut settings)?;
    write_portable_settings_atomic_locked(root, &settings)?;
    Ok(settings)
}

fn write_portable_settings_atomic_locked(
    root: &Path,
    value: &PortableSettingsV2,
) -> Result<(), String> {
    validate_settings(value)?;
    write_json_atomically(&root.join(SETTINGS_FILE), value)
}

pub fn update_global_settings(
    root: &Path,
    value: GlobalSettingsV2,
) -> Result<PortableSettingsV2, String> {
    validate_global_settings(&value)?;
    let _guard = settings_rmw_guard()?;
    let mut settings = load_portable_settings_locked(root)?;
    settings.global = value;
    increment_revision(&mut settings)?;
    write_portable_settings_atomic_locked(root, &settings)?;
    Ok(settings)
}

pub fn update_workspace_settings(
    root: &Path,
    expected_revision: u64,
    value: WorkspaceSettingsV1,
) -> Result<PortableSettingsV2, String> {
    let value = normalize_workspace_settings(value)?;
    compare_and_swap_portable_settings(root, expected_revision, move |settings| {
        settings.workspace = value;
        Ok(())
    })
}

pub fn update_account_defaults(
    root: &Path,
    value: AccountSettingsV2,
) -> Result<PortableSettingsV2, String> {
    validate_account_settings(&value)?;
    let _guard = settings_rmw_guard()?;
    let mut settings = load_portable_settings_locked(root)?;
    settings.account_defaults = value;
    increment_revision(&mut settings)?;
    write_portable_settings_atomic_locked(root, &settings)?;
    Ok(settings)
}

pub fn update_account_settings(
    root: &Path,
    portable_account_key: &str,
    value: AccountSettingsV2,
) -> Result<PortableSettingsV2, String> {
    validate_account_key(portable_account_key)?;
    validate_account_settings(&value)?;
    let _guard = settings_rmw_guard()?;
    let mut settings = load_portable_settings_locked(root)?;
    settings
        .accounts
        .insert(portable_account_key.to_string(), value);
    increment_revision(&mut settings)?;
    write_portable_settings_atomic_locked(root, &settings)?;
    Ok(settings)
}

pub fn account_settings_for(
    settings: &PortableSettingsV2,
    portable_account_key: Option<&str>,
) -> Result<AccountSettingsV2, String> {
    validate_settings(settings)?;
    let Some(key) = portable_account_key else {
        return Ok(settings.account_defaults.clone());
    };
    validate_account_key(key)?;
    Ok(settings
        .accounts
        .get(key)
        .cloned()
        .unwrap_or_else(|| settings.account_defaults.clone()))
}

pub fn retain_known_account_settings(
    root: &Path,
    known_account_keys: &BTreeSet<String>,
) -> Result<PortableSettingsV2, String> {
    for key in known_account_keys {
        validate_account_key(key)?;
    }
    let _guard = settings_rmw_guard()?;
    let mut settings = load_portable_settings_locked(root)?;
    let previous_len = settings.accounts.len();
    settings
        .accounts
        .retain(|key, _| known_account_keys.contains(key));
    if settings.accounts.len() == previous_len {
        return Ok(settings);
    }
    increment_revision(&mut settings)?;
    write_portable_settings_atomic_locked(root, &settings)?;
    Ok(settings)
}

pub fn remove_account_settings(
    root: &Path,
    portable_account_key: &str,
) -> Result<PortableSettingsV2, String> {
    validate_account_key(portable_account_key)?;
    let _guard = settings_rmw_guard()?;
    let mut settings = load_portable_settings_locked(root)?;
    if settings.accounts.remove(portable_account_key).is_none() {
        return Ok(settings);
    }
    increment_revision(&mut settings)?;
    write_portable_settings_atomic_locked(root, &settings)?;
    Ok(settings)
}

pub fn load_app_settings_view(
    root: &Path,
    portable_account_key: Option<&str>,
    known_account_keys: &BTreeSet<String>,
    data_root: PathBuf,
    install_dir: PathBuf,
    retained_data_roots: Vec<RetainedDataRootV1>,
) -> Result<AppSettingsView, String> {
    let settings = retain_known_account_settings(root, known_account_keys)?;
    let account = account_settings_for(&settings, portable_account_key)?;
    Ok(AppSettingsView {
        revision: settings.revision,
        data_root,
        install_dir,
        retained_data_roots,
        legacy_import_required: !settings.legacy_frontend_import.completed,
        global: settings.global,
        workspace: settings.workspace,
        account_defaults: settings.account_defaults,
        account,
    })
}

pub fn import_legacy_frontend_settings(
    root: &Path,
    portable_account_key: Option<&str>,
    legacy: LegacyFrontendSettingsV1,
) -> Result<PortableSettingsV2, String> {
    validate_legacy_settings(&legacy)?;
    if let Some(key) = portable_account_key {
        validate_account_key(key)?;
    }
    let fingerprint = legacy_fingerprint(&legacy)?;
    let _guard = settings_rmw_guard()?;
    let mut settings = load_portable_settings_locked(root)?;
    if settings.legacy_frontend_import.completed {
        if settings.legacy_frontend_import.fingerprint.as_deref() == Some(&fingerprint) {
            return Ok(settings);
        }
        return Err("legacy_settings_already_imported".to_string());
    }

    let account = AccountSettingsV2::from(AccountSettingsV1 {
        sync_mode: legacy.sync_mode,
        sync_days: legacy.sync_days,
        hide_browser: legacy.hide_browser,
        scroll_speed: legacy.scroll_speed,
    });
    settings.global.like_count_format = legacy.like_count_format;
    settings.global.detail_fields = legacy.detail_fields;
    settings.account_defaults = account.clone();
    if let Some(key) = portable_account_key {
        settings.accounts.insert(key.to_string(), account);
    }
    settings.legacy_frontend_import = LegacyImportStateV1 {
        completed: true,
        fingerprint: Some(fingerprint),
    };
    increment_revision(&mut settings)?;
    write_portable_settings_atomic_locked(root, &settings)?;
    Ok(settings)
}

fn settings_rmw_guard() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    SETTINGS_RMW_LOCK
        .lock()
        .map_err(|_| "settings_lock_poisoned".to_string())
}

fn increment_revision(settings: &mut PortableSettingsV2) -> Result<(), String> {
    settings.revision = settings
        .revision
        .checked_add(1)
        .ok_or_else(|| "settings_revision_overflow".to_string())?;
    Ok(())
}

fn validate_settings(value: &PortableSettingsV2) -> Result<(), String> {
    if value.schema_version != SETTINGS_SCHEMA_VERSION {
        return Err("unsupported_portable_settings_schema".to_string());
    }
    validate_global_settings(&value.global)?;
    validate_workspace_settings(&value.workspace)?;
    validate_account_settings(&value.account_defaults)?;
    validate_legacy_import_state(&value.legacy_frontend_import)?;
    for (key, account) in &value.accounts {
        validate_account_key(key)?;
        validate_account_settings(account)?;
    }
    Ok(())
}

pub fn normalize_portable_account_scope(
    scope: PortableAccountScopeV1,
) -> Result<PortableAccountScopeV1, String> {
    match scope {
        PortableAccountScopeV1::All => Ok(PortableAccountScopeV1::All),
        PortableAccountScopeV1::Selected { mut account_keys } => {
            for key in &account_keys {
                validate_account_key(key).map_err(|_| "invalid_account_scope".to_string())?;
            }
            account_keys.sort();
            account_keys.dedup();
            if account_keys.is_empty() {
                return Err("invalid_account_scope".to_string());
            }
            Ok(PortableAccountScopeV1::Selected { account_keys })
        }
    }
}

fn validate_portable_account_scope(scope: &PortableAccountScopeV1) -> Result<(), String> {
    let normalized = normalize_portable_account_scope(scope.clone())?;
    if &normalized != scope {
        return Err("invalid_account_scope".to_string());
    }
    Ok(())
}

pub fn validate_account_scope(scope: &AccountScopeV1) -> Result<(), String> {
    match scope {
        AccountScopeV1::All => Ok(()),
        AccountScopeV1::Selected { account_ids } => {
            if account_ids.is_empty()
                || account_ids.iter().any(|id| *id <= 0)
                || account_ids.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err("invalid_account_scope".to_string());
            }
            Ok(())
        }
    }
}

fn validate_workspace_settings(value: &WorkspaceSettingsV1) -> Result<(), String> {
    validate_portable_account_scope(&value.sync.account_scope)?;
    validate_portable_account_scope(&value.display_scopes.overview)?;
    validate_portable_account_scope(&value.display_scopes.library)?;
    validate_portable_account_scope(&value.display_scopes.search)?;

    if value.overview_layouts.len() > 64 {
        return Err("invalid_overview_layouts".to_string());
    }
    for (index, layout) in value.overview_layouts.iter().enumerate() {
        validate_portable_account_scope(&layout.account_scope)?;
        if value.overview_layouts[..index]
            .iter()
            .any(|prior| prior.account_scope == layout.account_scope)
            || layout.modules.len() > 16
        {
            return Err("invalid_overview_layouts".to_string());
        }
        for (module_index, module) in layout.modules.iter().enumerate() {
            if layout.modules[..module_index]
                .iter()
                .any(|prior| prior.id == module.id)
                || (module.id == OverviewModuleIdV1::SyncHealth && !module.visible)
            {
                return Err("invalid_overview_layouts".to_string());
            }
        }
    }

    for preference in [
        &value.content_views.all,
        &value.content_views.history,
        &value.content_views.likes,
        &value.content_views.favorites,
    ] {
        if preference.page == 0
            || preference.query.chars().count() > 500
            || preference.query.chars().any(char::is_control)
        {
            return Err("invalid_content_view_preference".to_string());
        }
    }
    if value
        .viewed_annual_review_years
        .iter()
        .any(|year| !(2000..=9999).contains(year))
    {
        return Err("invalid_annual_review_year".to_string());
    }
    Ok(())
}

fn normalize_workspace_settings(
    mut value: WorkspaceSettingsV1,
) -> Result<WorkspaceSettingsV1, String> {
    value.sync.account_scope = normalize_portable_account_scope(value.sync.account_scope)?;
    value.display_scopes.overview =
        normalize_portable_account_scope(value.display_scopes.overview)?;
    value.display_scopes.library =
        normalize_portable_account_scope(value.display_scopes.library)?;
    value.display_scopes.search =
        normalize_portable_account_scope(value.display_scopes.search)?;
    for layout in &mut value.overview_layouts {
        layout.account_scope = normalize_portable_account_scope(layout.account_scope.clone())?;
    }
    validate_workspace_settings(&value)?;
    Ok(value)
}

fn validate_settings_v1(value: &PortableSettingsV1) -> Result<(), String> {
    if value.schema_version != SETTINGS_SCHEMA_VERSION_V1 {
        return Err("unsupported_portable_settings_schema".to_string());
    }
    validate_account_settings_v1(&value.account_defaults)?;
    validate_legacy_import_state(&value.legacy_frontend_import)?;
    for (key, account) in &value.accounts {
        validate_account_key(key)?;
        validate_account_settings_v1(account)?;
    }
    Ok(())
}

fn validate_legacy_import_state(value: &LegacyImportStateV1) -> Result<(), String> {
    if value.completed != value.fingerprint.is_some() {
        return Err("invalid_legacy_import_state".to_string());
    }
    if value
        .fingerprint
        .as_deref()
        .is_some_and(|fingerprint| !valid_sha256(fingerprint))
    {
        return Err("invalid_legacy_import_state".to_string());
    }
    Ok(())
}

fn validate_legacy_settings(value: &LegacyFrontendSettingsV1) -> Result<(), String> {
    validate_account_settings_v1(&AccountSettingsV1 {
        sync_mode: value.sync_mode,
        sync_days: value.sync_days,
        hide_browser: value.hide_browser,
        scroll_speed: value.scroll_speed,
    })
}

fn validate_global_settings(value: &GlobalSettingsV2) -> Result<(), String> {
    if value.startup_sync_delay_minutes > 120 {
        return Err("invalid_startup_sync_delay_minutes".to_string());
    }
    if value.cover_cache.cleanup_by_capacity_enabled && value.cover_cache.capacity_limit_mib == 0 {
        return Err("invalid_cover_cache_capacity".to_string());
    }
    if value.cover_cache.cleanup_by_age_enabled && value.cover_cache.max_age_days == 0 {
        return Err("invalid_cover_cache_age".to_string());
    }
    if !(1..=365).contains(&value.backup.interval_days) {
        return Err("invalid_backup_interval".to_string());
    }
    if !valid_execution_time(&value.backup.execution_time) {
        return Err("invalid_backup_execution_time".to_string());
    }
    if !(1..=100).contains(&value.backup.retention_count) {
        return Err("invalid_backup_retention_count".to_string());
    }
    if !(1..=20).contains(&value.notifications.automatic_failure_threshold) {
        return Err("invalid_notification_failure_threshold".to_string());
    }
    if !(1..=300).contains(&value.notifications.message_duration_seconds) {
        return Err("invalid_notification_message_duration".to_string());
    }
    if value.updates.automatic_download && !value.updates.automatic_check {
        return Err("automatic_update_download_requires_check".to_string());
    }
    Ok(())
}

fn valid_execution_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 2 || byte.is_ascii_digit())
    {
        return false;
    }
    let hour = (bytes[0] - b'0') * 10 + (bytes[1] - b'0');
    let minute = (bytes[3] - b'0') * 10 + (bytes[4] - b'0');
    hour < 24 && minute < 60
}

fn validate_account_settings(value: &AccountSettingsV2) -> Result<(), String> {
    if value.scroll_speed < 120 {
        return Err("invalid_account_settings".to_string());
    }
    validate_history_source_settings(&value.history)?;
    validate_archive_source_settings(&value.likes)?;
    validate_favorite_source_settings(&value.favorites)
}

fn validate_account_settings_v1(value: &AccountSettingsV1) -> Result<(), String> {
    if !(1..=365).contains(&value.sync_days) || value.scroll_speed < 120 {
        return Err("invalid_account_settings".to_string());
    }
    Ok(())
}

fn validate_history_source_settings(value: &HistorySourceSettingsV1) -> Result<(), String> {
    match value.range {
        HistoryRangeV1::Days { days } if !(1..=365).contains(&days) => {
            return Err("invalid_history_range".to_string())
        }
        HistoryRangeV1::Days { .. } | HistoryRangeV1::Bottom => {}
    }
    validate_source_schedule(&value.schedule)
}

fn validate_archive_source_settings(value: &ArchiveSourceSettingsV1) -> Result<(), String> {
    match value.range {
        ArchiveListRangeV1::All => {}
        ArchiveListRangeV1::RecentItems { count } if !(1..=5_000).contains(&count) => {
            return Err("invalid_archive_list_range".to_string())
        }
        ArchiveListRangeV1::RecordLimit { limit } if !(1..=50_000).contains(&limit) => {
            return Err("invalid_archive_list_range".to_string())
        }
        ArchiveListRangeV1::RecentDays { days } if !(1..=365).contains(&days) => {
            return Err("invalid_archive_list_range".to_string())
        }
        ArchiveListRangeV1::RecentItems { .. }
        | ArchiveListRangeV1::RecordLimit { .. }
        | ArchiveListRangeV1::RecentDays { .. } => {}
    }
    validate_source_schedule(&value.schedule)
}

fn validate_favorite_source_settings(value: &FavoriteSourceSettingsV1) -> Result<(), String> {
    validate_archive_source_settings(&value.archive)?;
    validate_archive_source_settings(&value.remote_folder_defaults)?;
    if value.remote_folders.len() > 1_000 {
        return Err("remote_folder_settings_limit_exceeded".to_string());
    }
    for (remote_folder_id, settings) in &value.remote_folders {
        validate_remote_folder_id(remote_folder_id)?;
        validate_archive_source_settings(settings)?;
    }
    Ok(())
}

fn validate_source_schedule(value: &SourceScheduleV1) -> Result<(), String> {
    if !(15..=43_200).contains(&value.interval_minutes) {
        return Err("invalid_source_schedule".to_string());
    }
    Ok(())
}

fn validate_remote_folder_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err("invalid_remote_folder_id".to_string());
    }
    Ok(())
}

fn validate_account_key(value: &str) -> Result<(), String> {
    let identity = value
        .strip_prefix("sec:")
        .or_else(|| value.strip_prefix("uid:"))
        .ok_or_else(|| "invalid_portable_account_key".to_string())?;
    if normalized_identity(Some(identity)).as_deref() != Some(identity) {
        return Err("invalid_portable_account_key".to_string());
    }
    Ok(())
}

fn normalized_identity(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty()
        || value.len() > 512
        || value.chars().any(|character| character.is_control())
    {
        return None;
    }
    Some(value.to_string())
}

fn legacy_fingerprint(value: &LegacyFrontendSettingsV1) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
