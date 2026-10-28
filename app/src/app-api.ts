import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AccountFolderDisposition,
  AccountSettings,
  AccountScope,
  AccountView,
  AnnualReviewViewV1,
  AppSettingsView,
  BackupArchiveSummary,
  BrowserPreference,
  CommitOnboardingStepRequest,
  CommitOnboardingStepResult,
  ContentRecordView,
  ContentSort,
  CoverCacheCleanupPreview,
  CoverCacheCleanupResult,
  CoverCacheInspection,
  CoverCacheRuleItem,
  CoverCacheTaskView,
  FolderContentRecordView,
  DataRootMigrationView,
  DataRootValidationView,
  DeleteHistoryResult,
  DesktopIntentSnapshot,
  DesktopStateView,
  EnqueueSyncRequest,
  EnqueueSyncResult,
  GlobalSettings,
  HistoryRecordView,
  LegacyFrontendSettings,
  ListHistoryInput,
  ListHistoryResult,
  ListContentInput,
  ListContentResult,
  LibraryContentQueryV1,
  ListLibraryContentResultV1,
  SearchContentQueryV1,
  ListFolderContentResult,
  LocalFolderItemMutation,
  LocalFolderScope,
  LocalFolderView,
  LocalFolderMergeResolution,
  MergePlan,
  MergeRestoreResult,
  OverviewQuery,
  OverviewScopeQuery,
  OverviewView,
  TaskSummariesPage,
  NotificationsPage,
  NotificationView,
  RemoteFolderView,
  ReplaceRestoreResult,
  SchedulerInstancesPage,
  SchedulerState,
  SettingsMergeResolution,
  SetAutostartRequest,
  SetBackupDirectoryRequest,
  SetBrowserPreferenceRequest,
  SyncBatchEntriesPage,
  SyncBatchesPage,
  SyncBatchView,
  SyncOptions,
  SyncResult,
  SyncRangeKind,
  SyncSource,
  SyncStatus,
  StartupRecoveryDisposition,
  StartupRecoveryStatusView,
  UpdateStateView,
  WorkspaceSettings,
} from "./types";

const demoRecords: HistoryRecordView[] = [
  {
    id: 1,
    awemeId: "7654083396108329977",
    url: "https://www.douyin.com/video/7654083396108329977",
    contentType: "video",
    title: "一盘黄焖鸡：没有枪法就玩道具",
    likeCount: 54_000,
    historyTimeMs: 1_783_526_400_000,
    watchedAt: "2026-07-09T00:00:00.000Z",
    watchedDate: "2026-07-09",
    sourcePosition: 0,
    watchProgressRate: 0.42,
  },
  {
    id: 2,
    awemeId: "7652241435384025819",
    url: "https://www.douyin.com/video/7652241435384025819",
    contentType: "video",
    title: "配音片段",
    authorName: "大仁",
    historyTimeMs: 1_783_526_340_000,
    watchedAt: "2026-07-08T23:59:00.000Z",
    watchedDate: "2026-07-09",
    sourcePosition: 1,
  },
];

const demoContentRecords: ContentRecordView[] = demoRecords.map((record) => ({
  ...record,
  source: "history",
}));

const demoAccounts: AccountView[] = [
  {
    id: 1,
    uid: "demo-one",
    accountKey: "uid:demo-one",
    profileKey: "demo-account-one",
    nickname: "演示账号一",
    status: "active",
    createdAt: "2026-07-10T00:00:00.000Z",
    updatedAt: "2026-07-10T00:00:00.000Z",
  },
  {
    id: 2,
    uid: "demo-two",
    accountKey: "uid:demo-two",
    profileKey: "demo-account-two",
    nickname: "演示账号二",
    status: "login_expired",
    createdAt: "2026-07-11T00:00:00.000Z",
    updatedAt: "2026-07-11T00:00:00.000Z",
  },
];
let demoActiveAccountId = 1;
let demoSettingsRevision = 0;
let demoLegacyImportRequired = true;
let demoDataRoot = "C:\\Users\\Demo\\AppData\\Roaming\\Emily\\Data";
const demoInstallDir = "C:\\Program Files\\Emily";
let demoRetainedDataRoots: AppSettingsView["retainedDataRoots"] = [];
let demoGlobalSettings: GlobalSettings = {
  theme: "dark",
  likeCountFormat: "wan",
  detailFields: {
    author: true,
    watchDate: true,
    likeCount: true,
    watchProgress: true,
  },
  closeBehavior: "exit",
  startupSyncDelayMinutes: 5,
  coverCache: {
    enabled: false,
    backfillMode: "newRecordsOnly",
    capacityLimitMib: 2_048,
    cleanupByAgeEnabled: false,
    maxAgeDays: 90,
    cleanupByCapacityEnabled: false,
    cleanupListMode: "keepListed",
  },
  backup: {
    enabled: false,
    intervalDays: 7,
    executionTime: "03:00",
    retentionCount: 5,
  },
  notifications: {
    loginExpired: true,
    automaticFailureStreak: true,
    backupFailure: true,
    syncCompleted: false,
    backupCompleted: false,
    automaticFailureThreshold: 3,
    messageDurationSeconds: 5,
  },
  updates: {
    automaticCheck: false,
    automaticDownload: false,
  },
};
let demoWorkspaceSettings: WorkspaceSettings = {
  sync: { mode: "singleAccount", accountScope: { kind: "all" } },
  displayScopes: {
    overview: { kind: "all" },
    library: { kind: "all" },
    search: { kind: "all" },
  },
  overviewLayouts: [],
  contentViews: {
    all: {
      query: "",
      page: 1,
      viewMode: "horizontal",
      includeRemoteRemoved: false,
      sort: "archivedNewest",
    },
    history: {
      query: "",
      page: 1,
      viewMode: "horizontal",
      includeRemoteRemoved: false,
      sort: "watchedNewest",
    },
    likes: {
      query: "",
      page: 1,
      viewMode: "horizontal",
      includeRemoteRemoved: false,
      sort: "pageOrder",
    },
    favorites: {
      query: "",
      page: 1,
      viewMode: "horizontal",
      includeRemoteRemoved: false,
      sort: "pageOrder",
    },
  },
  viewedAnnualReviewYears: [],
};
let demoCoverCacheRules: CoverCacheRuleItem[] = [];
let demoUpdateState: UpdateStateView = {
  configured: false,
  phase: "notConfigured",
  currentVersion: "0.1.0",
  availableVersion: null,
  notes: null,
  publishedAt: null,
  downloadedBytes: 0,
  totalBytes: null,
  lastError: "update_not_configured",
  canCheck: false,
  canDownload: false,
  canInstall: false,
};
const demoAccountDefaults: AccountSettings = {
  hideBrowser: false,
  scrollSpeed: 6000,
  history: {
    range: { kind: "days", days: 7 },
    schedule: { enabled: false, intervalMinutes: 1_440 },
  },
  likes: {
    range: { kind: "recentItems", count: 100 },
    schedule: { enabled: false, intervalMinutes: 1_440 },
  },
  favorites: {
    archive: {
      range: { kind: "recentItems", count: 100 },
      schedule: { enabled: false, intervalMinutes: 1_440 },
    },
    remoteFolderMode: "unified",
    remoteFolderDefaults: {
      range: { kind: "recentItems", count: 100 },
      schedule: { enabled: false, intervalMinutes: 1_440 },
    },
    remoteFolders: {},
  },
};
const demoAccountSettings = new Map<number, AccountSettings>();
let demoDesktopState: DesktopStateView = {
  device: {
    revision: 1,
    onboardingCompleted: true,
    onboardingStep: 3,
    launchAtStartup: false,
    silentAutostart: false,
    schedulerPaused: false,
    backupDirectory: null,
    accounts: {},
  },
  trayAvailable: true,
  trayError: null,
  autostartError: null,
};
const demoSchedulerState: SchedulerState = {
  paused: false,
  workerState: "waiting",
  nextWakeAt: null,
  capabilities: [
    {
      source: "history",
      available: true,
      unavailableReason: null,
      supportedRanges: ["days", "bottom"],
      supportsFullReconcile: false,
      supportsRemoteFolders: false,
    },
    ...(["likes", "favorites", "remoteFolder"] as const).map((source) => ({
      source,
      available: true,
      unavailableReason: null,
      supportedRanges: ["all", "recentItems", "recordLimit"] as SyncRangeKind[],
      supportsFullReconcile: true,
      supportsRemoteFolders: source === "favorites",
    })),
  ],
  accounts: [],
};

export async function getStartupRecoveryStatus(): Promise<StartupRecoveryStatusView> {
  if (isTauri()) return invoke<StartupRecoveryStatusView>("get_startup_recovery_status");
  return { state: "ready", canResolve: false, canRetry: false };
}

export async function resolveStartupRecoveryFault(
  disposition: StartupRecoveryDisposition,
): Promise<void> {
  if (isTauri()) await invoke("resolve_startup_recovery_fault", { disposition });
}

export async function listHistory(input: ListHistoryInput): Promise<ListHistoryResult> {
  if (isTauri()) {
    return invoke<ListHistoryResult>("list_history", {
      accountId: input.accountId,
      query: input.query,
      limit: input.limit,
      offset: input.offset,
    });
  }

  const query = input.query?.trim().toLowerCase();
  const filtered = query
    ? demoRecords.filter((record) =>
        `${record.title ?? ""} ${record.authorName ?? ""}`.toLowerCase().includes(query),
      )
    : demoRecords;

  return {
    records: filtered.slice(input.offset, input.offset + input.limit),
    total: filtered.length,
  };
}

export async function listContent(input: ListContentInput): Promise<ListContentResult> {
  if (isTauri()) {
    return invoke<ListContentResult>("list_content", {
      accountId: input.accountId,
      source: input.source,
      query: input.query,
      includeRemoteRemoved: input.includeRemoteRemoved,
      sort: input.sort ?? "default",
      limit: input.limit,
      offset: input.offset,
    });
  }

  const query = input.query?.trim().toLowerCase();
  const sourceRecords = demoContentRecords.filter((record) => record.source === input.source);
  const filtered = sourceRecords.filter(
    (record) =>
      (input.includeRemoteRemoved || !record.remoteRemovedAt) &&
      (!query ||
        `${record.title ?? ""} ${record.authorName ?? ""}`.toLowerCase().includes(query)),
  );
  const sorted = sortDemoContent(filtered, input.sort ?? "default");
  return {
    records: sorted.slice(input.offset, input.offset + input.limit),
    total: filtered.length,
  };
}

export async function listLibraryContent(
  query: LibraryContentQueryV1,
): Promise<ListLibraryContentResultV1> {
  if (isTauri()) {
    return invoke<ListLibraryContentResultV1>("list_library_content", { query });
  }

  const accountIds = query.accountScope.kind === "all"
    ? demoAccounts.map((account) => account.id)
    : query.accountScope.accountIds;
  const textQuery = query.query?.trim().toLocaleLowerCase("zh-CN");
  const records = demoContentRecords
    .filter((record) => query.sources.includes(record.source))
    .filter((record) => query.includeRemoteRemoved || !record.remoteRemovedAt)
    .filter(
      (record) =>
        !textQuery ||
        `${record.title ?? ""} ${record.authorName ?? ""}`
          .toLocaleLowerCase("zh-CN")
          .includes(textQuery),
    )
    .flatMap((record) =>
      demoAccounts
        .filter((account) => accountIds.includes(account.id))
        .slice(0, 1)
        .map((account) => ({
          ...record,
          accountId: account.id,
          accountKey: account.accountKey ?? `profile:${account.profileKey}`,
          accountDisplayName: account.nickname ?? account.uid ?? `账号 ${account.id}`,
          firstArchivedAt: record.watchedAt ?? "2026-07-10T00:00:00.000Z",
        })),
    );
  const sorted = sortDemoContent(records, query.sort);
  return {
    records: sorted.slice(query.offset, query.offset + query.limit),
    total: sorted.length,
  };
}

export async function searchLibraryContent(
  query: SearchContentQueryV1,
): Promise<ListLibraryContentResultV1> {
  if (isTauri()) {
    return invoke<ListLibraryContentResultV1>("search_library_content", { query });
  }
  return listLibraryContent({
    accountScope: query.accountScope,
    sources: query.sources,
    query: query.query,
    includeRemoteRemoved: query.remoteState !== "active",
    sort: query.sort,
    limit: query.limit,
    offset: query.offset,
  });
}

function sortDemoContent<T extends ContentRecordView>(records: T[], sort: ContentSort): T[] {
  const sorted = [...records];
  const text = (value?: string | null) => value?.trim().toLocaleLowerCase("zh-CN") ?? "";
  const archivedAt = (record: T) =>
    "firstArchivedAt" in record ? Date.parse(String(record.firstArchivedAt)) || 0 : 0;
  sorted.sort((left, right) => {
    switch (sort) {
      case "archivedNewest":
        return archivedAt(right) - archivedAt(left);
      case "watchedOldest":
        return (left.historyTimeMs ?? 0) - (right.historyTimeMs ?? 0);
      case "remoteNewest":
        return (right.remoteCreatedAtMs ?? -1) - (left.remoteCreatedAtMs ?? -1);
      case "remoteOldest":
        return (left.remoteCreatedAtMs ?? Number.MAX_SAFE_INTEGER) -
          (right.remoteCreatedAtMs ?? Number.MAX_SAFE_INTEGER);
      case "likeCountDesc":
        return (right.likeCount ?? -1) - (left.likeCount ?? -1);
      case "titleAsc":
        return text(left.title).localeCompare(text(right.title), "zh-CN");
      case "authorAsc":
        return text(left.authorName).localeCompare(text(right.authorName), "zh-CN");
      case "pageOrder":
        return (left.sourcePosition ?? Number.MAX_SAFE_INTEGER) -
          (right.sourcePosition ?? Number.MAX_SAFE_INTEGER);
      case "default":
        return left.source === "history"
          ? (right.historyTimeMs ?? 0) - (left.historyTimeMs ?? 0)
          : (left.sourcePosition ?? Number.MAX_SAFE_INTEGER) -
              (right.sourcePosition ?? Number.MAX_SAFE_INTEGER);
      case "watchedNewest":
      default:
        return (right.historyTimeMs ?? 0) - (left.historyTimeMs ?? 0);
    }
  });
  return sorted;
}

export async function getOverview(input: OverviewQuery): Promise<OverviewView> {
  if (isTauri()) {
    return invoke<OverviewView>("get_overview", {
      accountId: input.accountId,
      source: input.source,
      range: input.range,
      includeRemoteRemoved: input.includeRemoteRemoved,
    });
  }
  const records = demoContentRecords.filter(
    (record) => input.source === "all" || record.source === input.source,
  );
  return {
    uniqueContentCount: new Set(records.map((record) => record.awemeId)).size,
    sourceCounts: {
      history: records.filter((record) => record.source === "history").length,
      likes: records.filter((record) => record.source === "likes").length,
      favorites: records.filter((record) => record.source === "favorites").length,
    },
    undatedArchiveCount: records.filter(
      (record) => record.source !== "history" && record.remoteCreatedAtMs == null,
    ).length,
    trend: [],
    topAuthors: [],
    contentTypes: [],
    recentSync: [],
  };
}

export async function getOverviewScope(input: OverviewScopeQuery): Promise<OverviewView> {
  if (isTauri()) {
    return invoke<OverviewView>("get_overview_scope", {
      accountScope: input.accountScope,
      source: input.source,
      range: input.range,
      includeRemoteRemoved: input.includeRemoteRemoved,
    });
  }
  return getOverview({
    accountId: input.accountScope.kind === "selected" ? input.accountScope.accountIds[0] ?? 1 : 1,
    source: input.source,
    range: input.range,
    includeRemoteRemoved: input.includeRemoteRemoved,
  });
}

export async function getAnnualReview(
  accountScope: AccountScope,
  year?: number,
): Promise<AnnualReviewViewV1> {
  if (isTauri()) {
    return invoke<AnnualReviewViewV1>("get_annual_review", {
      query: { accountScope, year: year ?? null },
    });
  }
  const resolvedYear = year ?? new Date().getFullYear();
  return {
    year: resolvedYear,
    availableYears: [resolvedYear],
    chapters: [
      {
        id: "footprint",
        title: "足迹",
        summary: "演示数据中的观看足迹",
        omitted: false,
        omittedReason: null,
        items: [{ label: "观看事件", value: String(demoRecords.length), secondary: null, rate: null }],
      },
      {
        id: "closing",
        title: "规则结语",
        summary: "年度回顾只使用本地归档数据。",
        omitted: false,
        omittedReason: null,
        items: [],
      },
    ],
  };
}

export async function openContentUrl(url: string): Promise<void> {
  if (isTauri()) {
    await invoke("open_content_url", { url });
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

export async function listRemoteFolders(
  accountId: number,
  includeRemoteRemoved: boolean,
): Promise<RemoteFolderView[]> {
  if (isTauri()) {
    return invoke<RemoteFolderView[]>("list_remote_folders", {
      accountId,
      includeRemoteRemoved,
    });
  }
  return [];
}

export async function listLocalFolders(accountId: number): Promise<LocalFolderView[]> {
  if (isTauri()) return invoke<LocalFolderView[]>("list_local_folders", { accountId });
  return [];
}

export async function createLocalFolder(
  accountId: number,
  name: string,
  scope: LocalFolderScope,
): Promise<LocalFolderView> {
  if (isTauri()) return invoke<LocalFolderView>("create_local_folder", { accountId, name, scope });
  throw new Error("Web 预览模式不支持创建本地收藏夹。");
}

export async function renameLocalFolder(
  accountId: number,
  folderId: number,
  name: string,
): Promise<LocalFolderView> {
  if (isTauri()) return invoke<LocalFolderView>("rename_local_folder", { accountId, folderId, name });
  throw new Error("Web 预览模式不支持重命名本地收藏夹。");
}

export async function setLocalFolderScope(
  accountId: number,
  folderId: number,
  scope: LocalFolderScope,
  ownerAccountId: number | null,
): Promise<LocalFolderView> {
  if (isTauri()) {
    return invoke<LocalFolderView>("set_local_folder_scope", {
      accountId,
      folderId,
      scope,
      ownerAccountId,
    });
  }
  throw new Error("Web 预览模式不支持修改本地收藏夹归属。");
}

export async function deleteLocalFolder(accountId: number, folderId: number): Promise<void> {
  if (isTauri()) await invoke("delete_local_folder", { accountId, folderId });
}

export async function addLocalFolderItem(
  accountId: number,
  folderId: number,
  awemeId: string,
): Promise<LocalFolderItemMutation> {
  if (isTauri()) {
    return invoke<LocalFolderItemMutation>("add_local_folder_item", {
      accountId,
      folderId,
      awemeId,
    });
  }
  throw new Error("Web 预览模式不支持添加本地收藏。");
}

export async function removeLocalFolderItem(
  accountId: number,
  folderId: number,
  awemeId: string,
): Promise<boolean> {
  if (isTauri()) {
    return invoke<boolean>("remove_local_folder_item", { accountId, folderId, awemeId });
  }
  return false;
}

export async function listRemoteFolderContent(input: {
  accountId: number;
  remoteFolderId: string;
  query?: string;
  includeRemoteRemoved: boolean;
  limit: number;
  offset: number;
}): Promise<ListFolderContentResult> {
  if (isTauri()) return invoke<ListFolderContentResult>("list_remote_folder_content", input);
  return { records: [], total: 0 };
}

export async function listLocalFolderContent(input: {
  accountId: number;
  folderId: number;
  query?: string;
  limit: number;
  offset: number;
}): Promise<ListFolderContentResult> {
  if (isTauri()) return invoke<ListFolderContentResult>("list_local_folder_content", input);
  return { records: [], total: 0 };
}

export async function startSync(accountId: number, options: SyncOptions): Promise<SyncResult> {
  if (isTauri()) {
    return invoke<SyncResult>("start_sync", { accountId, options });
  }

  return {
    accountId,
    jobId: 0,
    status: "failed",
    insertedVideos: 0,
    insertedWatchRecords: 0,
    message: "Web 预览模式不能启动同步，请在 Tauri 桌面壳中运行。",
  };
}

export async function getSyncStatus(accountId: number, jobId: number): Promise<SyncStatus> {
  if (isTauri()) {
    return invoke<SyncStatus>("get_sync_status", { accountId, jobId });
  }

  return {
    accountId,
    jobId,
    source: "history",
    sourceInstanceKey: "",
    trigger: "manual",
    status: "idle",
    insertedVideos: 0,
    insertedRecords: 0,
    insertedWatchRecords: 0,
    recordCount: 0,
    elapsedMs: 0,
    retryCount: 0,
    canCancel: false,
    issues: [],
  };
}

export async function cancelSync(accountId: number, jobId: number): Promise<void> {
  if (isTauri()) {
    await invoke("cancel_sync", { accountId, jobId });
  }
}

export async function listAccounts(includeArchived = false): Promise<AccountView[]> {
  if (isTauri()) {
    return invoke<AccountView[]>("list_accounts", { includeArchived });
  }
  return demoAccounts.filter((account) => includeArchived || account.status !== "archived");
}

export async function getActiveAccount(): Promise<AccountView | null> {
  if (isTauri()) return invoke<AccountView | null>("get_active_account");
  return demoAccounts.find((account) => account.id === demoActiveAccountId) ?? null;
}

export async function beginAccountLogin(
  browserPreference?: BrowserPreference,
): Promise<AccountView> {
  if (isTauri()) {
    return browserPreference === undefined
      ? invoke<AccountView>("begin_account_login")
      : invoke<AccountView>("begin_account_login", {
          request: { browserPreference },
        });
  }
  throw new Error("Web 预览模式不会模拟抖音登录成功。");
}

export async function cancelAccountLogin(): Promise<void> {
  if (isTauri()) await invoke("cancel_account_login");
}

export async function resumeAccountLogin(accountId: number): Promise<AccountView> {
  if (isTauri()) return invoke<AccountView>("resume_account_login", { accountId });
  throw new Error("Web 预览模式不会模拟抖音登录成功。");
}

export async function cancelPendingAccount(accountId: number): Promise<void> {
  if (isTauri()) await invoke("cancel_pending_account", { accountId });
}

export async function restoreAccount(accountId: number): Promise<AccountView> {
  if (isTauri()) return invoke<AccountView>("restore_account", { accountId });
  throw new Error("Web 预览模式不支持恢复账号。");
}

export async function setActiveAccount(accountId: number): Promise<AccountView> {
  if (isTauri()) return invoke<AccountView>("set_active_account", { accountId });
  const account = demoAccounts.find((item) => item.id === accountId);
  if (!account) throw new Error("account_not_found");
  demoActiveAccountId = accountId;
  return account;
}

export async function archiveAccount(accountId: number): Promise<void> {
  if (isTauri()) await invoke("archive_account", { accountId });
}

export async function deleteAccount(
  accountId: number,
  folderDisposition: AccountFolderDisposition,
): Promise<void> {
  if (isTauri()) await invoke("delete_account", { accountId, folderDisposition });
}

export async function deleteHistoryRecords(
  accountId: number,
  ids: number[],
): Promise<DeleteHistoryResult> {
  if (isTauri()) {
    return invoke<DeleteHistoryResult>("delete_history_records", { accountId, ids });
  }

  return {
    deletedWatchRecords: ids.length,
    deletedVideos: ids.length,
    remainingWatchRecords: Math.max(0, demoRecords.length - ids.length),
  };
}

export async function deleteHistoryByQuery(
  accountId: number,
  query?: string,
): Promise<DeleteHistoryResult> {
  if (isTauri()) {
    return invoke<DeleteHistoryResult>("delete_history_by_query", { accountId, query });
  }

  const trimmed = query?.trim().toLowerCase();
  const count = trimmed
    ? demoRecords.filter((record) =>
        `${record.title ?? ""} ${record.authorName ?? ""}`.toLowerCase().includes(trimmed),
      ).length
    : demoRecords.length;
  return {
    deletedWatchRecords: count,
    deletedVideos: count,
    remainingWatchRecords: Math.max(0, demoRecords.length - count),
  };
}

export async function getAppSettings(accountId: number | null): Promise<AppSettingsView> {
  if (isTauri()) return invoke<AppSettingsView>("get_app_settings", { accountId });
  return {
    revision: demoSettingsRevision,
    dataRoot: demoDataRoot,
    installDir: demoInstallDir,
    retainedDataRoots: demoRetainedDataRoots.map((item) => ({
      ...item,
      root: { ...item.root },
      legacyEntries: [...item.legacyEntries],
    })),
    legacyImportRequired: demoLegacyImportRequired,
    global: {
      ...demoGlobalSettings,
      detailFields: { ...demoGlobalSettings.detailFields },
      coverCache: { ...demoGlobalSettings.coverCache },
      backup: { ...demoGlobalSettings.backup },
      notifications: { ...demoGlobalSettings.notifications },
      updates: { ...demoGlobalSettings.updates },
    },
    workspace: structuredClone(demoWorkspaceSettings),
    accountDefaults: cloneAccountSettings(demoAccountDefaults),
    account: cloneAccountSettings(
      accountId === null
        ? demoAccountDefaults
        : (demoAccountSettings.get(accountId) ?? demoAccountDefaults),
    ),
  };
}

export async function updateGlobalSettings(settings: GlobalSettings): Promise<number> {
  if (isTauri()) return invoke<number>("update_global_settings", { settings });
  demoGlobalSettings = {
    ...settings,
    detailFields: { ...settings.detailFields },
    coverCache: { ...settings.coverCache },
    backup: { ...settings.backup },
    notifications: { ...settings.notifications },
    updates: { ...settings.updates },
  };
  demoSettingsRevision += 1;
  return demoSettingsRevision;
}

export async function updateWorkspaceSettings(
  expectedRevision: number,
  settings: WorkspaceSettings,
): Promise<number> {
  if (isTauri()) {
    return invoke<number>("update_workspace_settings", { expectedRevision, settings });
  }
  if (expectedRevision !== demoSettingsRevision) {
    throw new Error("portable_settings_revision_conflict");
  }
  demoWorkspaceSettings = structuredClone(settings);
  demoSettingsRevision += 1;
  return demoSettingsRevision;
}

export async function listNotifications(
  includeResolved: boolean,
  limit: number,
  offset: number,
): Promise<NotificationsPage> {
  if (isTauri()) {
    return invoke<NotificationsPage>("list_notifications", {
      includeResolved,
      limit,
      offset,
    });
  }
  return { records: [], total: 0, unread: 0 };
}

export async function markNotificationRead(
  notificationId: number,
  read: boolean,
): Promise<NotificationView> {
  if (isTauri()) {
    return invoke<NotificationView>("mark_notification_read", { notificationId, read });
  }
  throw new Error("notification_not_found");
}

export async function markAllNotificationsRead(): Promise<number> {
  if (isTauri()) return invoke<number>("mark_all_notifications_read");
  return 0;
}

export async function resolveNotification(notificationId: number): Promise<NotificationView> {
  if (isTauri()) {
    return invoke<NotificationView>("resolve_notification", { notificationId });
  }
  throw new Error("notification_not_found");
}

export async function getUpdateState(): Promise<UpdateStateView> {
  if (isTauri()) return invoke<UpdateStateView>("get_update_state");
  return structuredClone(demoUpdateState);
}

export async function checkForUpdates(): Promise<UpdateStateView> {
  if (isTauri()) return invoke<UpdateStateView>("check_for_updates");
  return structuredClone(demoUpdateState);
}

export async function downloadUpdate(): Promise<UpdateStateView> {
  if (isTauri()) return invoke<UpdateStateView>("download_update");
  return structuredClone(demoUpdateState);
}

export async function installUpdate(): Promise<UpdateStateView> {
  if (isTauri()) return invoke<UpdateStateView>("install_update");
  return structuredClone(demoUpdateState);
}

export async function getCoverCacheTask(): Promise<CoverCacheTaskView | null> {
  if (isTauri()) return invoke<CoverCacheTaskView | null>("get_cover_cache_task");
  return null;
}

export async function pauseCoverCacheTask(taskId: number): Promise<CoverCacheTaskView> {
  if (isTauri()) return invoke<CoverCacheTaskView>("pause_cover_cache_task", { taskId });
  throw new Error("desktop_runtime_required");
}

export async function resumeCoverCacheTask(taskId: number): Promise<CoverCacheTaskView> {
  if (isTauri()) return invoke<CoverCacheTaskView>("resume_cover_cache_task", { taskId });
  throw new Error("desktop_runtime_required");
}

export async function cancelCoverCacheTask(taskId: number): Promise<CoverCacheTaskView> {
  if (isTauri()) return invoke<CoverCacheTaskView>("cancel_cover_cache_task", { taskId });
  throw new Error("desktop_runtime_required");
}

export async function retryCoverCacheTask(taskId: number): Promise<CoverCacheTaskView> {
  if (isTauri()) return invoke<CoverCacheTaskView>("retry_cover_cache_task", { taskId });
  throw new Error("desktop_runtime_required");
}

export async function inspectCoverCache(): Promise<CoverCacheInspection> {
  if (isTauri()) return invoke<CoverCacheInspection>("inspect_cover_cache");
  return {
    healthyFiles: 0,
    missingFiles: 0,
    orphanFiles: 0,
    invalidIndexes: 0,
    indexedBytes: 0,
    orphanBytes: 0,
  };
}

export async function listCoverCacheRules(): Promise<CoverCacheRuleItem[]> {
  if (isTauri()) return invoke<CoverCacheRuleItem[]>("list_cover_cache_rules");
  return structuredClone(demoCoverCacheRules);
}

export async function searchCoverCacheCandidates(
  query: string,
  limit = 50,
): Promise<CoverCacheRuleItem[]> {
  if (isTauri()) {
    return invoke<CoverCacheRuleItem[]>("search_cover_cache_candidates", { query, limit });
  }
  const normalized = query.trim().toLowerCase();
  return demoContentRecords
    .filter((record) => !demoCoverCacheRules.some((item) => item.awemeId === record.awemeId))
    .filter((record) =>
      `${record.awemeId} ${record.title ?? ""} ${record.authorName ?? ""}`
        .toLowerCase()
        .includes(normalized),
    )
    .slice(0, limit)
    .map((record) => ({
      videoId: record.id,
      awemeId: record.awemeId,
      title: record.title ?? null,
      authorName: record.authorName ?? null,
      coverUrl: record.coverUrl ?? null,
      cached: Boolean(record.cachedCoverUrl),
    }));
}

export async function addCoverCacheRule(videoId: number): Promise<void> {
  if (isTauri()) return invoke<void>("add_cover_cache_rule", { videoId });
  if (!demoCoverCacheRules.some((item) => item.videoId === videoId)) {
    const record = demoContentRecords.find((item) => item.id === videoId);
    if (record) {
      demoCoverCacheRules.push({
        videoId,
        awemeId: record.awemeId,
        title: record.title ?? null,
        authorName: record.authorName ?? null,
        coverUrl: record.coverUrl ?? null,
        cached: Boolean(record.cachedCoverUrl),
      });
    }
  }
}

export async function removeCoverCacheRule(videoId: number): Promise<boolean> {
  if (isTauri()) return invoke<boolean>("remove_cover_cache_rule", { videoId });
  const length = demoCoverCacheRules.length;
  demoCoverCacheRules = demoCoverCacheRules.filter((item) => item.videoId !== videoId);
  return demoCoverCacheRules.length !== length;
}

export async function clearCoverCacheRules(): Promise<number> {
  if (isTauri()) return invoke<number>("clear_cover_cache_rules");
  const count = demoCoverCacheRules.length;
  demoCoverCacheRules = [];
  return count;
}

export async function previewCoverCacheCleanup(): Promise<CoverCacheCleanupPreview> {
  if (isTauri()) return invoke<CoverCacheCleanupPreview>("preview_cover_cache_cleanup");
  return {
    files: 0,
    videos: 0,
    estimatedBytes: 0,
    mode: demoGlobalSettings.coverCache.cleanupListMode,
  };
}

export async function runCoverCacheCleanup(): Promise<CoverCacheCleanupResult> {
  if (isTauri()) return invoke<CoverCacheCleanupResult>("run_cover_cache_cleanup");
  return { deletedFiles: 0, deletedIndexes: 0, releasedBytes: 0 };
}

export async function cleanupBrokenCoverCache(): Promise<CoverCacheCleanupResult> {
  if (isTauri()) return invoke<CoverCacheCleanupResult>("cleanup_broken_cover_cache");
  return { deletedFiles: 0, deletedIndexes: 0, releasedBytes: 0 };
}

export async function updateAccountSettings(
  accountId: number,
  settings: AccountSettings,
): Promise<number> {
  if (isTauri()) {
    return invoke<number>("update_account_settings", { accountId, settings });
  }
  demoAccountSettings.set(accountId, cloneAccountSettings(settings));
  demoSettingsRevision += 1;
  return demoSettingsRevision;
}

export async function importLegacySettings(
  accountId: number | null,
  legacy: LegacyFrontendSettings,
): Promise<number> {
  if (isTauri()) return invoke<number>("import_legacy_settings", { accountId, legacy });
  demoGlobalSettings = {
    ...demoGlobalSettings,
    likeCountFormat: legacy.likeCountFormat,
    detailFields: { ...legacy.detailFields },
  };
  const account = cloneAccountSettings(demoAccountDefaults);
  account.history.range =
    legacy.syncMode === "bottom"
      ? { kind: "bottom" }
      : { kind: "days", days: legacy.syncDays };
  account.hideBrowser = legacy.hideBrowser;
  account.scrollSpeed = legacy.scrollSpeed;
  if (accountId !== null) demoAccountSettings.set(accountId, account);
  demoLegacyImportRequired = false;
  demoSettingsRevision += 1;
  return demoSettingsRevision;
}

export async function chooseDataRoot(): Promise<string | null> {
  if (isTauri()) return invoke<string | null>("choose_data_root");
  return null;
}

export async function chooseBackupDirectory(): Promise<string | null> {
  if (isTauri()) return invoke<string | null>("choose_backup_directory");
  return null;
}

export async function setBackupDirectory(
  request: SetBackupDirectoryRequest,
): Promise<DesktopStateView["device"]> {
  if (isTauri()) {
    return invoke<DesktopStateView["device"]>("set_backup_directory", {
      path: request.path,
      expectedDeviceRevision: request.expectedDeviceRevision,
    });
  }
  demoDesktopState.device = {
    ...demoDesktopState.device,
    revision: demoDesktopState.device.revision + 1,
    backupDirectory: request.path,
  };
  return structuredClone(demoDesktopState.device);
}

export async function chooseBackupExportPath(): Promise<string | null> {
  if (isTauri()) return invoke<string | null>("choose_backup_export_path");
  return null;
}

export async function exportBackup(destination: string): Promise<BackupArchiveSummary> {
  if (isTauri()) return invoke<BackupArchiveSummary>("export_backup", { destination });
  return {
    createdAt: new Date().toISOString(),
    applicationVersion: "0.1.0",
    databaseSchemaVersion: 7,
    settingsSchemaVersion: 6,
    byteSize: 0,
  };
}

export async function chooseBackupImportPath(): Promise<string | null> {
  if (isTauri()) return invoke<string | null>("choose_backup_import_path");
  return null;
}

export async function inspectBackupImport(archivePath: string): Promise<BackupArchiveSummary> {
  if (isTauri()) {
    return invoke<BackupArchiveSummary>("inspect_backup_import", { archivePath });
  }
  return {
    createdAt: new Date().toISOString(),
    applicationVersion: "0.1.0",
    databaseSchemaVersion: 7,
    settingsSchemaVersion: 6,
    byteSize: 0,
  };
}

export async function restoreBackupReplace(archivePath: string): Promise<ReplaceRestoreResult> {
  if (isTauri()) {
    return invoke<ReplaceRestoreResult>("restore_backup_replace", { archivePath });
  }
  return {
    imported: await inspectBackupImport(archivePath),
    safetyBackupPath: "demo-backups/emily-before-restore.emily-backup",
    activeAccountId: null,
  };
}

export async function prepareBackupMerge(archivePath: string): Promise<MergePlan> {
  if (isTauri()) {
    return invoke<MergePlan>("prepare_backup_merge", { archivePath });
  }
  const archive = await inspectBackupImport(archivePath);
  return {
    schemaVersion: 1,
    archive,
    currentFingerprint: "demo-current",
    archiveFingerprint: "demo-archive",
    planHash: "demo-plan",
    counts: {
      accounts: 0,
      videos: 0,
      watchRecords: 0,
      likes: 0,
      favorites: 0,
      remoteFolders: 0,
      localFolders: 0,
    },
    localFolderConflicts: [],
    settingsConflicts: [],
  };
}

export async function restoreBackupMerge(
  archivePath: string,
  expectedPlan: MergePlan,
  folderResolutions: Record<string, LocalFolderMergeResolution>,
  settingsResolution: SettingsMergeResolution,
): Promise<MergeRestoreResult> {
  if (isTauri()) {
    return invoke<MergeRestoreResult>("restore_backup_merge", {
      archivePath,
      expectedPlan,
      folderResolutions,
      settingsResolution,
    });
  }
  return {
    imported: expectedPlan.archive,
    merged: expectedPlan.counts,
    safetyBackupPath: "demo-backups/emily-before-restore.emily-backup",
  };
}

export async function validateDataRoot(path: string): Promise<DataRootValidationView> {
  if (isTauri()) return invoke<DataRootValidationView>("validate_data_root", { path });
  const adjustedToInstallData = path.toLowerCase() === demoInstallDir.toLowerCase();
  return {
    requestedPath: path,
    normalizedPath: adjustedToInstallData ? `${demoInstallDir}\\Data` : path,
    adjustedToInstallData,
  };
}

export async function setDataRoot(path: string): Promise<DataRootMigrationView> {
  if (isTauri()) return invoke<DataRootMigrationView>("set_data_root", { path });
  const retainedSource = {
    path: demoDataRoot,
    rootId: "0123456789abcdef0123456789abcdef",
  };
  demoRetainedDataRoots = [
    ...demoRetainedDataRoots,
    { root: retainedSource, layout: "standard", legacyEntries: [] },
  ];
  demoDataRoot = path;
  return { dataRoot: path, retainedSource };
}

export async function initializeOnboardingDataRoot(path: string): Promise<{ dataRoot: string }> {
  if (isTauri()) {
    return invoke<{ dataRoot: string }>("initialize_onboarding_data_root", { path });
  }
  demoDataRoot = path;
  return { dataRoot: path };
}

export async function deleteRetainedDataRoot(path: string): Promise<void> {
  if (isTauri()) {
    await invoke("delete_retained_data_root", { path });
    return;
  }
  demoRetainedDataRoots = demoRetainedDataRoots.filter((item) => item.root.path !== path);
}

export async function getDesktopState(): Promise<DesktopStateView> {
  if (isTauri()) return invoke<DesktopStateView>("get_desktop_state");
  return structuredClone(demoDesktopState);
}

export async function takeDesktopIntents(
  lastSeenRevision: number,
): Promise<DesktopIntentSnapshot> {
  if (isTauri()) {
    return invoke<DesktopIntentSnapshot>("take_desktop_intents", { lastSeenRevision });
  }
  return { revision: lastSeenRevision, showMain: false, openSettings: false };
}

export async function getSchedulerState(): Promise<SchedulerState> {
  if (isTauri()) return invoke<SchedulerState>("get_scheduler_state");
  return structuredClone(demoSchedulerState);
}

export async function listSchedulerInstances(
  accountId: number,
  source: SyncSource,
  cursor: string | null,
  limit: number,
): Promise<SchedulerInstancesPage> {
  if (isTauri()) {
    return invoke<SchedulerInstancesPage>("list_scheduler_instances", {
      accountId,
      source,
      cursor,
      limit,
    });
  }
  return {
    accountId,
    source,
    portableSettingsRevision: demoSettingsRevision,
    registryRevision: 0,
    instances: [],
    nextCursor: null,
    hasMore: false,
  };
}

export async function enqueueSyncTasks(
  request: EnqueueSyncRequest,
): Promise<EnqueueSyncResult> {
  if (isTauri()) return invoke<EnqueueSyncResult>("enqueue_sync_tasks", { request });
  return {
    batchId: null,
    counts: { jobs: 0, skipped: 1, conflicts: 0 },
    entries: [
      {
        kind: "skipped",
        data: {
          accountId: request.accountId ?? 0,
          source: request.source ?? "history",
          sourceInstanceKey: request.sourceInstanceKey ?? "",
          phase: "initialExpansion",
          reason: "desktop_runtime_required",
          requestedPayloadSha256: null,
        },
      },
    ],
    nextCursor: null,
  };
}

export async function enqueueWorkspaceSync(
  accountScope: AccountScope,
): Promise<EnqueueSyncResult> {
  if (isTauri()) {
    return invoke<EnqueueSyncResult>("enqueue_workspace_sync", { accountScope });
  }
  return {
    batchId: null,
    counts: { jobs: 0, skipped: 1, conflicts: 0 },
    entries: [],
    nextCursor: null,
  };
}

export async function getSyncBatch(batchId: number): Promise<SyncBatchView> {
  if (isTauri()) return invoke<SyncBatchView>("get_sync_batch", { batchId });
  throw new Error("desktop_runtime_required");
}

export async function listSyncBatches(
  accountId: number,
  limit: number,
  offset: number,
): Promise<SyncBatchesPage> {
  if (isTauri()) {
    return invoke<SyncBatchesPage>("list_sync_batches", { accountId, limit, offset });
  }
  return { records: [], total: 0 };
}

export async function listTaskSummaries(
  accountScope: AccountScope,
  limit: number,
  offset: number,
): Promise<TaskSummariesPage> {
  if (isTauri()) {
    return invoke<TaskSummariesPage>("list_task_summaries", {
      accountScope,
      limit,
      offset,
    });
  }
  return { records: [], total: 0 };
}

export async function cancelSyncBatch(batchId: number): Promise<void> {
  if (isTauri()) {
    await invoke("cancel_sync_batch", { batchId });
  }
}

export async function listSyncBatchEntries(
  batchId: number,
  cursor: string | null,
  limit: number,
): Promise<SyncBatchEntriesPage> {
  if (isTauri()) {
    return invoke<SyncBatchEntriesPage>("list_sync_batch_entries", {
      batchId,
      cursor,
      limit,
    });
  }
  return { batchId, entries: [], nextCursor: cursor ?? "", hasMore: false };
}

export async function updateAccountDefaults(settings: AccountSettings): Promise<number> {
  if (isTauri()) return invoke<number>("update_account_defaults", { settings });
  Object.assign(demoAccountDefaults, cloneAccountSettings(settings));
  demoSettingsRevision += 1;
  return demoSettingsRevision;
}

export async function commitOnboardingStep(
  request: CommitOnboardingStepRequest,
): Promise<CommitOnboardingStepResult> {
  if (isTauri()) {
    return invoke<CommitOnboardingStepResult>("commit_onboarding_step", { request });
  }
  const completed = request.step.kind === "desktopBehavior";
  const nextStep = completed
    ? 3
    : request.step.kind === "account"
      ? 1
      : request.step.kind === "syncRange"
        ? 2
        : 3;
  demoDesktopState = {
    ...demoDesktopState,
    device: {
      ...demoDesktopState.device,
      revision: demoDesktopState.device.revision + 1,
      onboardingStep: nextStep,
      onboardingCompleted: completed,
    },
  };
  return {
    portableRevision: request.expectedPortableRevision + (request.step.kind === "account" ? 0 : 1),
    device: structuredClone(demoDesktopState.device),
  };
}

export async function setAutostart(request: SetAutostartRequest): Promise<DesktopStateView["device"]> {
  if (isTauri()) return invoke<DesktopStateView["device"]>("set_autostart", { request });
  demoDesktopState.device = {
    ...demoDesktopState.device,
    revision: demoDesktopState.device.revision + 1,
    launchAtStartup: request.enabled,
    silentAutostart: request.enabled && request.silent,
  };
  return structuredClone(demoDesktopState.device);
}

export async function setBrowserPreference(
  request: SetBrowserPreferenceRequest,
): Promise<DesktopStateView["device"]> {
  if (isTauri()) {
    return invoke<DesktopStateView["device"]>("set_browser_preference", { request });
  }
  const account = demoAccounts.find((item) => item.id === request.accountId);
  if (account) {
    demoDesktopState.device.accounts[account.profileKey] = {
      browserPreference: request.browserPreference,
    };
  }
  demoDesktopState.device.revision += 1;
  return structuredClone(demoDesktopState.device);
}

export async function setSchedulerPaused(paused: boolean): Promise<void> {
  if (isTauri()) await invoke("set_scheduler_paused", { paused });
  demoDesktopState.device.schedulerPaused = paused;
  demoSchedulerState.paused = paused;
  demoSchedulerState.workerState = paused ? "paused" : "waiting";
}

export async function requestAppExit(): Promise<void> {
  if (isTauri()) await invoke("request_app_exit");
}

export type AppEventName =
  | "desktop:intent-changed"
  | "scheduler:changed"
  | "sync:changed"
  | "notification:changed"
  | "update:changed";

export async function listenAppEvent(
  event: AppEventName,
  handler: () => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen(event, () => handler());
}

function cloneAccountSettings(settings: AccountSettings): AccountSettings {
  return structuredClone(settings);
}

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export * from "./download/api.js";
