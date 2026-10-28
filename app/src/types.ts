export interface HistoryRecordView {
  id: number;
  awemeId: string;
  url: string;
  contentType: "video" | "note" | "unknown";
  title?: string;
  authorName?: string;
  coverUrl?: string;
  cachedCoverUrl?: string;
  likeCount?: number | null;
  historyTimeMs: number;
  watchedAt: string;
  watchedDate: string;
  sourcePosition: number;
  watchProgressRate?: number | null;
}

export interface ListHistoryInput {
  accountId: number;
  query?: string;
  limit: number;
  offset: number;
}

export interface ListHistoryResult {
  records: HistoryRecordView[];
  total: number;
}

export interface ContentRecordView {
  id: number;
  source: "history" | "likes" | "favorites";
  awemeId: string;
  url: string;
  contentType: "video" | "note" | "unknown";
  title?: string;
  authorName?: string;
  coverUrl?: string;
  cachedCoverUrl?: string;
  likeCount?: number | null;
  sourcePosition?: number | null;
  historyTimeMs?: number | null;
  watchedAt?: string | null;
  watchedDate?: string | null;
  watchProgressRate?: number | null;
  remoteCreatedAtMs?: number | null;
  firstSeenAt?: string | null;
  lastSeenAt?: string | null;
  remoteRemovedAt?: string | null;
}

export type ContentSort =
  | "default"
  | "archivedNewest"
  | "watchedNewest"
  | "watchedOldest"
  | "pageOrder"
  | "remoteNewest"
  | "remoteOldest"
  | "likeCountDesc"
  | "titleAsc"
  | "authorAsc";

export interface ListContentInput {
  accountId: number;
  source: "history" | "likes" | "favorites";
  query?: string;
  includeRemoteRemoved: boolean;
  sort?: ContentSort;
  limit: number;
  offset: number;
}

export interface ListContentResult {
  records: ContentRecordView[];
  total: number;
}

export type AccountScopeV1 =
  | { kind: "all" }
  | { kind: "selected"; accountIds: number[] };

export type LibraryContentSort = ContentSort | "archivedNewest";

export interface LibraryContentQueryV1 {
  accountScope: AccountScopeV1;
  sources: Array<"history" | "likes" | "favorites">;
  query?: string;
  includeRemoteRemoved: boolean;
  sort: LibraryContentSort;
  limit: number;
  offset: number;
}

export interface LibraryContentRecordViewV1 extends ContentRecordView {
  accountId: number;
  accountKey: string;
  accountDisplayName: string;
  firstArchivedAt: string;
}

export interface ListLibraryContentResultV1 {
  records: LibraryContentRecordViewV1[];
  total: number;
}

export type SearchRemoteStateV1 = "active" | "removed" | "all";

export interface SearchContentQueryV1 {
  accountScope: AccountScopeV1;
  sources: Array<"history" | "likes" | "favorites">;
  query?: string;
  contentTypes: Array<"video" | "note" | "unknown">;
  dateFrom?: string;
  dateTo?: string;
  remoteState: SearchRemoteStateV1;
  localFolderId?: number;
  sort: LibraryContentSort;
  limit: number;
  offset: number;
}

export type OverviewSourceFilter = "all" | "history" | "likes" | "favorites";
export type OverviewRange = "last7Days" | "last30Days" | "last90Days" | "all";

export interface OverviewQuery {
  accountId: number;
  source: OverviewSourceFilter;
  range: OverviewRange;
  includeRemoteRemoved: boolean;
}

export interface OverviewScopeQuery {
  accountScope: AccountScope;
  source: OverviewSourceFilter;
  range: OverviewRange;
  includeRemoteRemoved: boolean;
}

export interface OverviewSourceCounts {
  history: number;
  likes: number;
  favorites: number;
}

export interface OverviewTrendPoint extends OverviewSourceCounts {
  date: string;
}

export interface OverviewAuthorCount {
  authorName: string;
  contentCount: number;
}

export interface OverviewContentTypeCount {
  contentType: "video" | "note" | "unknown";
  contentCount: number;
}

export interface OverviewRecentSync {
  accountId: number;
  source: "history" | "likes" | "favorites";
  status: SyncJobStatus;
  finishedAt: string;
  insertedRecords: number;
  errorCode: string | null;
}

export interface OverviewView {
  uniqueContentCount: number;
  sourceCounts: OverviewSourceCounts;
  undatedArchiveCount: number;
  trend: OverviewTrendPoint[];
  topAuthors: OverviewAuthorCount[];
  contentTypes: OverviewContentTypeCount[];
  recentSync: OverviewRecentSync[];
}

export type AnnualReviewChapterId =
  | "footprint"
  | "rhythm"
  | "streak"
  | "authors"
  | "rewatch"
  | "progress"
  | "archiveSnapshot"
  | "closing";

export interface AnnualReviewItemV1 {
  label: string;
  value: string;
  secondary: string | null;
  rate: number | null;
}

export interface AnnualReviewChapterV1 {
  id: AnnualReviewChapterId;
  title: string;
  summary: string;
  omitted: boolean;
  omittedReason: string | null;
  items: AnnualReviewItemV1[];
}

export interface AnnualReviewViewV1 {
  year: number;
  availableYears: number[];
  chapters: AnnualReviewChapterV1[];
}

export type LocalFolderScope = "account" | "aggregate";

export interface RemoteFolderView {
  id: number;
  remoteFolderId: string;
  name: string;
  firstSeenAt: string;
  lastSeenAt: string;
  remoteRemovedAt: string | null;
  itemCount: number;
  activeItemCount: number;
}

export interface LocalFolderView {
  id: number;
  scope: LocalFolderScope;
  ownerAccountId: number | null;
  name: string;
  itemCount: number;
  createdAt: string;
  updatedAt: string;
}

export interface FolderSourceAccountView {
  id: number;
  nickname: string | null;
}

export interface FolderContentRecordView {
  videoId: number;
  awemeId: string;
  url: string;
  contentType: "video" | "note" | "unknown";
  title: string | null;
  authorName: string | null;
  coverUrl: string | null;
  cachedCoverUrl: string | null;
  likeCount: number | null;
  addedAt: string | null;
  firstSeenAt: string | null;
  lastSeenAt: string | null;
  remoteRemovedAt: string | null;
  sourceAccounts: FolderSourceAccountView[];
}

export interface ListFolderContentResult {
  records: FolderContentRecordView[];
  total: number;
}

export interface LocalFolderItemMutation {
  insertedItem: boolean;
  insertedProvenance: boolean;
}

export interface SyncResult {
  accountId: number;
  jobId: number;
  status: "queued" | "running" | "success" | "partial" | "failed" | "cancelled";
  insertedVideos: number;
  insertedWatchRecords: number;
  message?: string;
}

export type SyncMode = "days" | "bottom";

export interface SyncOptions {
  mode: SyncMode;
  days?: number;
  scrollSpeed?: number;
  hideBrowser?: boolean;
}

export type DetailField = "author" | "watchDate" | "likeCount" | "watchProgress";

export type LikeCountFormat = "exact" | "w" | "wan" | "k" | "qian";

export type DetailFields = Record<DetailField, boolean>;

export type CloseBehavior = "tray" | "exit";
export type ThemeMode = "system" | "light" | "dark";

export type CoverCacheBackfillMode = "newRecordsOnly" | "backfillAll";

export type CoverCacheCleanupMode = "keepListed" | "cleanListed";

export interface CoverCacheSettings {
  enabled: boolean;
  backfillMode: CoverCacheBackfillMode;
  capacityLimitMib: number;
  cleanupByAgeEnabled: boolean;
  maxAgeDays: number;
  cleanupByCapacityEnabled: boolean;
  cleanupListMode: CoverCacheCleanupMode;
}

export interface BackupSettings {
  enabled: boolean;
  intervalDays: number;
  executionTime: string;
  retentionCount: number;
}

export interface NotificationSettings {
  loginExpired: boolean;
  automaticFailureStreak: boolean;
  backupFailure: boolean;
  syncCompleted: boolean;
  backupCompleted: boolean;
  automaticFailureThreshold: number;
  messageDurationSeconds: number;
}

export type NotificationSeverity = "info" | "warning" | "error";

export interface NotificationView {
  id: number;
  kind: string;
  severity: NotificationSeverity;
  dedupeKey: string;
  title: string;
  body: string;
  accountId?: number | null;
  relatedType?: string | null;
  relatedId?: string | null;
  createdAt: string;
  readAt?: string | null;
  resolvedAt?: string | null;
}

export interface NotificationsPage {
  records: NotificationView[];
  total: number;
  unread: number;
}

export type TaskSummaryKind = "syncBatch" | "downloadJob";

export type TaskSummaryStatus =
  | "queued"
  | "running"
  | "paused"
  | "interrupted"
  | "success"
  | "partial"
  | "failed"
  | "completed"
  | "completedWithErrors"
  | "cancelled";

export interface TaskSummary {
  key: string;
  kind: TaskSummaryKind;
  id: number;
  accountId: number | null;
  source: SyncSource | null;
  status: TaskSummaryStatus;
  createdAt: string;
  finishedAt: string | null;
  itemCount: number;
  completedCount: number;
  failedCount: number;
  errorCode: string | null;
  canCancel: boolean;
}

export interface TaskSummariesPage {
  records: TaskSummary[];
  total: number;
}

export interface UpdateSettings {
  automaticCheck: boolean;
  automaticDownload: boolean;
}

export type UpdatePhase =
  | "notConfigured"
  | "idle"
  | "checking"
  | "upToDate"
  | "available"
  | "downloading"
  | "downloaded"
  | "installing"
  | "failed";

export interface UpdateStateView {
  configured: boolean;
  phase: UpdatePhase;
  currentVersion: string;
  availableVersion: string | null;
  notes: string | null;
  publishedAt: string | null;
  downloadedBytes: number;
  totalBytes: number | null;
  lastError: string | null;
  canCheck: boolean;
  canDownload: boolean;
  canInstall: boolean;
}

export interface BackupArchiveSummary {
  createdAt: string;
  applicationVersion: string;
  databaseSchemaVersion: number;
  settingsSchemaVersion: number;
  byteSize: number;
}

export interface ReplaceRestoreResult {
  imported: BackupArchiveSummary;
  safetyBackupPath: string;
  activeAccountId: number | null;
}

export interface MergeCounts {
  accounts: number;
  videos: number;
  watchRecords: number;
  likes: number;
  favorites: number;
  remoteFolders: number;
  localFolders: number;
}

export interface LocalFolderMergeConflict {
  id: string;
  scope: "account" | "aggregate";
  accountKey: string | null;
  name: string;
}

export interface SettingsMergeConflict {
  id: string;
  path: string;
  currentValue: unknown;
  backupValue: unknown;
}

export interface MergePlan {
  schemaVersion: number;
  archive: BackupArchiveSummary;
  currentFingerprint: string;
  archiveFingerprint: string;
  planHash: string;
  counts: MergeCounts;
  localFolderConflicts: LocalFolderMergeConflict[];
  settingsConflicts: SettingsMergeConflict[];
}

export type LocalFolderMergeResolution =
  | { action: "merge" }
  | { action: "rename"; name: string }
  | { action: "skip" };

export type SettingValueSource = "current" | "backup";

export type SettingsMergeResolution =
  | { mode: "current" }
  | { mode: "backup" }
  | { mode: "perField"; choices: Record<string, SettingValueSource> };

export interface MergeRestoreResult {
  imported: BackupArchiveSummary;
  merged: MergeCounts;
  safetyBackupPath: string;
}

export type CoverCacheTaskKind = "backfill" | "new_records" | "cleanup";

export type CoverCacheTaskStatus =
  | "queued"
  | "running"
  | "paused"
  | "success"
  | "failed"
  | "cancelled";

export interface CoverCacheTaskView {
  id: number;
  kind: CoverCacheTaskKind;
  status: CoverCacheTaskStatus;
  totalItems: number;
  completedItems: number;
  failedItems: number;
  createdAt: string;
  startedAt: string | null;
  finishedAt: string | null;
  errorCode: string | null;
}

export interface CoverCacheInspection {
  healthyFiles: number;
  missingFiles: number;
  orphanFiles: number;
  invalidIndexes: number;
  indexedBytes: number;
  orphanBytes: number;
}

export interface CoverCacheCleanupResult {
  deletedFiles: number;
  deletedIndexes: number;
  releasedBytes: number;
}

export interface CoverCacheRuleItem {
  videoId: number;
  awemeId: string;
  title: string | null;
  authorName: string | null;
  coverUrl: string | null;
  cached: boolean;
}

export interface CoverCacheCleanupPreview {
  files: number;
  videos: number;
  estimatedBytes: number;
  mode: CoverCacheCleanupMode;
}

export type BrowserPreference = "auto" | "edge" | "chrome";

export type SyncSource = "history" | "likes" | "favorites" | "remoteFolder";

export type SyncScope = "one" | "currentAccountAvailableSources" | "allActiveAccounts";

export type SyncTrigger = "manual" | "automatic";

export type SyncJobStatus =
  | "queued"
  | "running"
  | "success"
  | "partial"
  | "failed"
  | "cancelled";

export type SyncBatchStatus = SyncJobStatus;

export type SyncRangeKind =
  | "days"
  | "bottom"
  | "all"
  | "recentItems"
  | "recordLimit"
  | "recentDays";

export type HistoryRange = { kind: "days"; days: number } | { kind: "bottom" };

export type ArchiveListRange =
  | { kind: "all" }
  | { kind: "recentItems"; count: number }
  | { kind: "recordLimit"; limit: number }
  | { kind: "recentDays"; days: number };

export interface SourceSchedule {
  enabled: boolean;
  intervalMinutes: number;
}

export interface HistorySourceSettings {
  range: HistoryRange;
  schedule: SourceSchedule;
}

export interface ArchiveSourceSettings {
  range: ArchiveListRange;
  schedule: SourceSchedule;
}

export interface FavoriteSourceSettings {
  archive: ArchiveSourceSettings;
  remoteFolderMode: "unified" | "perFolder";
  remoteFolderDefaults: ArchiveSourceSettings;
  remoteFolders: Record<string, ArchiveSourceSettings>;
}

export interface GlobalSettings {
  theme: ThemeMode;
  likeCountFormat: LikeCountFormat;
  detailFields: DetailFields;
  closeBehavior: CloseBehavior;
  startupSyncDelayMinutes: number;
  coverCache: CoverCacheSettings;
  backup: BackupSettings;
  notifications: NotificationSettings;
  updates: UpdateSettings;
}

export type AccountScope =
  | { kind: "all" }
  | { kind: "selected"; accountIds: number[] };

export type PortableAccountScope =
  | { kind: "all" }
  | { kind: "selected"; accountKeys: string[] };

export type DefaultSyncMode = "singleAccount" | "aggregate";
export type OverviewModuleId =
  | "syncHealth"
  | "sourceTotals"
  | "recentWatched"
  | "recentArchived"
  | "sevenDayTrend"
  | "annualReviewBanner";
export type OverviewModuleWidth = "half" | "full";
export type WorkspaceContentViewMode = "horizontal" | "grid";

export interface WorkspaceContentViewPreference {
  query: string;
  page: number;
  viewMode: WorkspaceContentViewMode;
  includeRemoteRemoved: boolean;
  sort: Exclude<ContentSort, "default">;
}

export interface WorkspaceSettings {
  sync: {
    mode: DefaultSyncMode;
    accountScope: PortableAccountScope;
  };
  displayScopes: {
    overview: PortableAccountScope;
    library: PortableAccountScope;
    search: PortableAccountScope;
  };
  overviewLayouts: Array<{
    accountScope: PortableAccountScope;
    modules: Array<{
      id: OverviewModuleId;
      visible: boolean;
      width: OverviewModuleWidth;
    }>;
  }>;
  contentViews: Record<
    "all" | "history" | "likes" | "favorites",
    WorkspaceContentViewPreference
  >;
  viewedAnnualReviewYears: number[];
}

export interface AccountSettings {
  hideBrowser: boolean;
  scrollSpeed: number;
  history: HistorySourceSettings;
  likes: ArchiveSourceSettings;
  favorites: FavoriteSourceSettings;
}

export interface LegacyFrontendSettings {
  syncMode: SyncMode;
  syncDays: number;
  hideBrowser: boolean;
  scrollSpeed: number;
  likeCountFormat: LikeCountFormat;
  detailFields: DetailFields;
}

export interface DataRootRef {
  path: string;
  rootId: string;
}

export interface RetainedDataRoot {
  root: DataRootRef;
  layout: "standard" | "legacyFlat";
  legacyEntries: string[];
}

export interface AppSettingsView {
  revision: number;
  dataRoot: string;
  installDir: string;
  retainedDataRoots: RetainedDataRoot[];
  legacyImportRequired: boolean;
  global: GlobalSettings;
  workspace: WorkspaceSettings;
  accountDefaults: AccountSettings;
  account: AccountSettings;
}

export interface DeviceAccountSettings {
  browserPreference: BrowserPreference;
}

export interface DeviceSettingsView {
  revision: number;
  onboardingCompleted: boolean;
  onboardingStep: 0 | 1 | 2 | 3;
  launchAtStartup: boolean;
  silentAutostart: boolean;
  schedulerPaused: boolean;
  backupDirectory: string | null;
  accounts: Record<string, DeviceAccountSettings>;
}

export interface SetBackupDirectoryRequest {
  path: string | null;
  expectedDeviceRevision: number;
}

export interface DesktopStateView {
  device: DeviceSettingsView;
  trayAvailable: boolean;
  trayError: string | null;
  autostartError: string | null;
}

export interface DesktopIntentSnapshot {
  revision: number;
  showMain: boolean;
  openSettings: boolean;
}

export interface SourceCapability {
  source: SyncSource;
  available: boolean;
  unavailableReason: string | null;
  supportedRanges: SyncRangeKind[];
  supportsFullReconcile: boolean;
  supportsRemoteFolders: boolean;
}

export type SchedulerWorkerState = "idle" | "waiting" | "running" | "paused" | "stopped";

export interface SchedulerSourceState {
  source: SyncSource;
  available: boolean;
  unavailableReason: string | null;
  enabled: boolean;
  intervalMinutes: number | null;
  instanceCount: number;
  aggregateError: string | null;
  message: string | null;
  lastStatus: SyncJobStatus | null;
  lastFinishedAt: string | null;
  nextDueAt: string | null;
  activeJobId: number | null;
}

export interface SchedulerAccountState {
  accountId: number;
  accountStatus: string;
  sourceStates: SchedulerSourceState[];
}

export interface SchedulerState {
  paused: boolean;
  workerState: SchedulerWorkerState;
  nextWakeAt: string | null;
  capabilities: SourceCapability[];
  accounts: SchedulerAccountState[];
}

export interface SchedulerInstanceView {
  remoteFolderId: string;
  name: string;
  range: ArchiveListRange;
  schedule: SourceSchedule;
}

export interface SchedulerInstancesPage {
  accountId: number;
  source: SyncSource;
  portableSettingsRevision: number;
  registryRevision: number;
  instances: SchedulerInstanceView[];
  nextCursor: string | null;
  hasMore: boolean;
}

export interface SyncBatchCounts {
  jobs: number;
  skipped: number;
  conflicts: number;
}

export interface SyncJobView {
  id: number;
  accountId: number;
  source: SyncSource;
  sourceInstanceKey: string;
  trigger: SyncTrigger;
  status: SyncJobStatus;
  queuedAt: string;
  scheduledFor: string | null;
  startedAt: string | null;
  finishedAt: string | null;
  queuePosition: number | null;
  canCancel: boolean;
  issues: SyncJobIssue[];
}

export interface SyncSkip {
  accountId: number;
  source: SyncSource;
  sourceInstanceKey: string;
  phase: "initialExpansion" | "discoveryExpansion";
  reason: string;
  requestedPayloadSha256: string | null;
}

export interface SyncConflict extends Omit<SyncSkip, "reason" | "requestedPayloadSha256"> {
  reason: string;
  requestedPayloadSha256: string;
  existingPayloadSha256: string;
  existingJobAtConflict: SyncJobView;
}

export type SyncBatchEntry =
  | { kind: "job"; data: SyncJobView }
  | { kind: "skipped"; data: SyncSkip }
  | { kind: "conflict"; data: SyncConflict };

export interface EnqueueSyncRequest {
  scope: SyncScope;
  accountId: number | null;
  source: SyncSource | null;
  sourceInstanceKey: string | null;
  payload: unknown | null;
  fullReconcileConfirmationSha256: string | null;
}

export interface EnqueueSyncResult {
  batchId: number | null;
  counts: SyncBatchCounts;
  entries: SyncBatchEntry[];
  nextCursor: string | null;
}

export interface SyncBatchView {
  id: number;
  trigger: SyncTrigger;
  scope: SyncScope;
  status: SyncBatchStatus;
  createdAt: string;
  finishedAt: string | null;
  counts: SyncBatchCounts;
}

export interface SyncBatchesPage {
  records: SyncBatchView[];
  total: number;
}

export interface SyncBatchEntriesPage {
  batchId: number;
  entries: SyncBatchEntry[];
  nextCursor: string;
  hasMore: boolean;
}

export type OnboardingStepRequest =
  | { kind: "account"; skipped: boolean }
  | {
      kind: "syncRange";
      accountDefaults: AccountSettings;
      syncAfterOnboarding: boolean;
    }
  | {
      kind: "automaticSync";
      accountDefaults: AccountSettings;
      startupSyncDelayMinutes: number;
    }
  | {
      kind: "desktopBehavior";
      closeBehavior: CloseBehavior;
      launchAtStartup: boolean;
      silentAutostart: boolean;
    };

export interface CommitOnboardingStepRequest {
  expectedPortableRevision: number;
  expectedDeviceRevision: number;
  step: OnboardingStepRequest;
}

export interface CommitOnboardingStepResult {
  portableRevision: number;
  device: DeviceSettingsView;
}

export interface SetAutostartRequest {
  expectedDeviceRevision: number;
  enabled: boolean;
  silent: boolean;
}

export interface SetBrowserPreferenceRequest {
  accountId: number;
  browserPreference: BrowserPreference;
  confirmRelogin: boolean;
}

export interface DataRootValidationView {
  requestedPath: string;
  normalizedPath: string;
  adjustedToInstallData: boolean;
}

export interface DataRootMigrationView {
  dataRoot: string;
  retainedSource: DataRootRef;
}

export interface SyncStatus {
  accountId: number;
  jobId: number;
  source: SyncSource;
  sourceInstanceKey: string;
  trigger: SyncTrigger;
  status: "idle" | SyncJobStatus;
  queuePosition?: number | null;
  insertedVideos: number;
  insertedRecords: number;
  insertedWatchRecords: number;
  startedAt?: string | null;
  finishedAt?: string | null;
  errorCode?: string | null;
  errorMessage?: string | null;
  phase?: string | null;
  recordCount: number;
  elapsedMs: number;
  retryCount: number;
  canCancel: boolean;
  issues: SyncJobIssue[];
}

export interface SyncJobIssue {
  phase: string;
  code: string;
  severity: "warning" | "incomplete";
  message?: string | null;
}

export type AccountStatus =
  | "pending_login"
  | "active"
  | "login_expired"
  | "identity_conflict"
  | "archived";

export interface AccountView {
  id: number;
  uid?: string;
  secUid?: string;
  accountKey?: string;
  profileKey: string;
  nickname?: string;
  avatarUrl?: string;
  status: AccountStatus;
  lastLoginValidatedAt?: string;
  createdAt: string;
  updatedAt: string;
}

export type AccountFolderDisposition =
  | { type: "delete" }
  | { type: "convertToAggregate" }
  | { type: "transferToAccount"; targetAccountId: number };

export interface DeleteHistoryResult {
  deletedWatchRecords: number;
  deletedVideos: number;
  remainingWatchRecords: number;
}

export type StartupRecoveryDisposition = "preserveQueued" | "cancelQueued";

export interface StartupRecoveryStatusView {
  state: "ready" | "blocked" | "recovering" | "failed";
  errorCode?: string | null;
  canResolve: boolean;
  canRetry: boolean;
}
