import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App.svelte";
import type { AppSettingsView } from "./types";

const api = vi.hoisted(() => ({
  archiveAccount: vi.fn(),
  beginAccountLogin: vi.fn(),
  cancelSync: vi.fn(),
  cancelPendingAccount: vi.fn(),
  chooseBackupDirectory: vi.fn(),
  chooseBackupExportPath: vi.fn(),
  chooseBackupImportPath: vi.fn(),
  chooseDataRoot: vi.fn(),
  deleteAccount: vi.fn(),
  deleteHistoryByQuery: vi.fn(),
  deleteHistoryRecords: vi.fn(),
  deleteRetainedDataRoot: vi.fn(),
  getActiveAccount: vi.fn(),
  getAppSettings: vi.fn(),
  getDesktopState: vi.fn(),
  getSchedulerState: vi.fn(),
  getCoverCacheTask: vi.fn(),
  getOverview: vi.fn(),
  getOverviewScope: vi.fn(),
  getUpdateState: vi.fn(),
  checkForUpdates: vi.fn(),
  downloadUpdate: vi.fn(),
  installUpdate: vi.fn(),
  inspectCoverCache: vi.fn(),
  inspectBackupImport: vi.fn(),
  listCoverCacheRules: vi.fn(),
  searchCoverCacheCandidates: vi.fn(),
  addCoverCacheRule: vi.fn(),
  removeCoverCacheRule: vi.fn(),
  clearCoverCacheRules: vi.fn(),
  previewCoverCacheCleanup: vi.fn(),
  runCoverCacheCleanup: vi.fn(),
  cleanupBrokenCoverCache: vi.fn(),
  pauseCoverCacheTask: vi.fn(),
  prepareBackupMerge: vi.fn(),
  resumeCoverCacheTask: vi.fn(),
  cancelCoverCacheTask: vi.fn(),
  retryCoverCacheTask: vi.fn(),
  getStartupRecoveryStatus: vi.fn(),
  getSyncStatus: vi.fn(),
  importLegacySettings: vi.fn(),
  initializeOnboardingDataRoot: vi.fn(),
  addLocalFolderItem: vi.fn(),
  createLocalFolder: vi.fn(),
  listenAppEvent: vi.fn(),
  listAccounts: vi.fn(),
  listContent: vi.fn(),
  listLibraryContent: vi.fn(),
  listLocalFolderContent: vi.fn(),
  listLocalFolders: vi.fn(),
  listRemoteFolderContent: vi.fn(),
  listRemoteFolders: vi.fn(),
  searchLibraryContent: vi.fn(),
  openContentUrl: vi.fn(),
  deleteLocalFolder: vi.fn(),
  removeLocalFolderItem: vi.fn(),
  renameLocalFolder: vi.fn(),
  restoreAccount: vi.fn(),
  restoreBackupMerge: vi.fn(),
  restoreBackupReplace: vi.fn(),
  resolveStartupRecoveryFault: vi.fn(),
  resumeAccountLogin: vi.fn(),
  setActiveAccount: vi.fn(),
  setAutostart: vi.fn(),
  setBackupDirectory: vi.fn(),
  setBrowserPreference: vi.fn(),
  setDataRoot: vi.fn(),
  setLocalFolderScope: vi.fn(),
  setSchedulerPaused: vi.fn(),
  updateAccountSettings: vi.fn(),
  updateGlobalSettings: vi.fn(),
  updateWorkspaceSettings: vi.fn(),
  validateDataRoot: vi.fn(),
  commitOnboardingStep: vi.fn(),
  enqueueSyncTasks: vi.fn(),
  enqueueWorkspaceSync: vi.fn(),
  exportBackup: vi.fn(),
  getSyncBatch: vi.fn(),
  listTaskSummaries: vi.fn(),
  cancelSyncBatch: vi.fn(),
  listNotifications: vi.fn(),
  markAllNotificationsRead: vi.fn(),
  markNotificationRead: vi.fn(),
  resolveNotification: vi.fn(),
  listSchedulerInstances: vi.fn(),
  listSyncBatches: vi.fn(),
  listSyncBatchEntries: vi.fn(),
  takeDesktopIntents: vi.fn(),
  updateAccountDefaults: vi.fn(),
}));

const downloadApi = vi.hoisted(() => ({
  cancelDownloadJob: vi.fn(),
  createDownloadJob: vi.fn(),
  getDownloadJob: vi.fn(),
  listDownloadJobs: vi.fn(),
  listenForDownloadRefresh: vi.fn(),
  pauseDownloadJob: vi.fn(),
  previewDownloadSelection: vi.fn(),
  resumeDownloadJob: vi.fn(),
  retryFailedDownload: vi.fn(),
  revealDownloadItem: vi.fn(),
  revealDownloadJob: vi.fn(),
}));

const dialogApi = vi.hoisted(() => ({
  open: vi.fn(),
}));

vi.mock("./app-api", () => api);
vi.mock("./download/api", () => downloadApi);
vi.mock("@tauri-apps/plugin-dialog", () => dialogApi);

const account = (status: "active" | "login_expired" | "identity_conflict") => ({
  id: 11,
  accountKey: "sec:test-account",
  profileKey: "0123456789abcdef0123456789abcdef",
  nickname: status === "identity_conflict" ? "冲突账号" : "测试账号",
  status,
  createdAt: "2026-07-14T00:00:00Z",
  updatedAt: "2026-07-14T00:00:00Z",
});

const enqueueJobResult = (overrides: Record<string, unknown> = {}) => ({
  batchId: null,
  counts: { jobs: 1, skipped: 0, conflicts: 0 },
  entries: [
    {
      kind: "job" as const,
      data: {
        id: 23,
        accountId: 11,
        source: "history" as const,
        sourceInstanceKey: "",
        trigger: "manual" as const,
        status: "queued" as const,
        queuedAt: "2026-08-12T00:00:00.000Z",
        scheduledFor: null,
        startedAt: null,
        finishedAt: null,
        queuePosition: 1,
        canCancel: true,
        issues: [],
        ...overrides,
      },
    },
  ],
  nextCursor: null,
});

const syncStatus = (
  phase: string,
  overrides: Record<string, unknown> = {},
) => ({
  accountId: 11,
  jobId: 23,
  source: "history" as const,
  status: "running" as const,
  insertedVideos: 0,
  insertedWatchRecords: 0,
  phase,
  recordCount: 0,
  elapsedMs: 1000,
  retryCount: 0,
  canCancel: true,
  ...overrides,
});

const appSettings = (overrides: Record<string, unknown> = {}) => ({
  revision: 0,
  dataRoot: "C:\\Users\\Test\\AppData\\Emily\\Data",
  installDir: "C:\\Program Files\\Emily",
  retainedDataRoots: [],
  legacyImportRequired: false,
  global: {
    theme: "system",
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
  },
  workspace: {
    sync: { mode: "singleAccount", accountScope: { kind: "all" } },
    displayScopes: {
      overview: { kind: "all" },
      library: { kind: "all" },
      search: { kind: "all" },
    },
    overviewLayouts: [],
    contentViews: {
      all: { query: "", page: 1, viewMode: "horizontal", includeRemoteRemoved: false, sort: "archivedNewest" },
      history: { query: "", page: 1, viewMode: "horizontal", includeRemoteRemoved: false, sort: "watchedNewest" },
      likes: { query: "", page: 1, viewMode: "grid", includeRemoteRemoved: false, sort: "pageOrder" },
      favorites: { query: "", page: 1, viewMode: "horizontal", includeRemoteRemoved: false, sort: "pageOrder" },
    },
    viewedAnnualReviewYears: [],
  },
  accountDefaults: {
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
  },
  account: {
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
  },
  ...overrides,
});

const desktopState = (overrides: Record<string, unknown> = {}) => ({
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
  ...overrides,
});

const schedulerState = (overrides: Record<string, unknown> = {}) => ({
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
      available: false,
      unavailableReason: "collector_not_implemented",
      supportedRanges: ["all", "recentItems", "recordLimit"],
      supportsFullReconcile: true,
      supportsRemoteFolders: false,
    })),
  ],
  accounts: [],
  ...overrides,
});

const updateState = (overrides: Record<string, unknown> = {}) => ({
  configured: true,
  phase: "idle" as const,
  currentVersion: "0.1.0",
  availableVersion: null,
  notes: null,
  publishedAt: null,
  downloadedBytes: 0,
  totalBytes: null,
  lastError: null,
  canCheck: true,
  canDownload: false,
  canInstall: false,
  ...overrides,
});

async function selectSettingsPanel(category: string, panel: string): Promise<void> {
  const categoryNavigation = await screen.findByRole("navigation", { name: "设置分类" });
  await fireEvent.click(within(categoryNavigation).getByRole("button", { name: category }));
  const panelNavigation = screen.getByRole("navigation", { name: "设置子项" });
  const panelButton = await within(panelNavigation).findByRole("button", { name: panel });
  await fireEvent.click(panelButton);
}

describe("账号登录恢复界面", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.removeItem("douyin-history-download-settings");
    api.listAccounts.mockResolvedValue([]);
    api.getActiveAccount.mockResolvedValue(null);
    api.getAppSettings.mockResolvedValue(appSettings());
    api.getDesktopState.mockResolvedValue(desktopState());
    api.getSchedulerState.mockResolvedValue(schedulerState());
    api.getCoverCacheTask.mockResolvedValue(null);
    api.getOverview.mockResolvedValue({
      uniqueContentCount: 0,
      sourceCounts: { history: 0, likes: 0, favorites: 0 },
      undatedArchiveCount: 0,
      trend: [],
      topAuthors: [],
      contentTypes: [],
      recentSync: [],
    });
    api.getOverviewScope.mockResolvedValue({
      uniqueContentCount: 0,
      sourceCounts: { history: 0, likes: 0, favorites: 0 },
      undatedArchiveCount: 0,
      trend: [],
      topAuthors: [],
      contentTypes: [],
      recentSync: [],
    });
    api.getUpdateState.mockResolvedValue(updateState());
    api.checkForUpdates.mockResolvedValue(
      updateState({
        phase: "available",
        availableVersion: "0.2.0",
        notes: "稳定性更新",
        publishedAt: "2026-08-15T12:00:00Z",
        canDownload: true,
      }),
    );
    api.downloadUpdate.mockResolvedValue(
      updateState({
        phase: "downloaded",
        availableVersion: "0.2.0",
        notes: "稳定性更新",
        publishedAt: "2026-08-15T12:00:00Z",
        downloadedBytes: 4_096,
        totalBytes: 4_096,
        canCheck: true,
        canInstall: true,
      }),
    );
    api.installUpdate.mockResolvedValue(
      updateState({
        phase: "installing",
        availableVersion: "0.2.0",
        canCheck: false,
        canInstall: false,
      }),
    );
    api.listSyncBatches.mockResolvedValue({ records: [], total: 0 });
    api.listTaskSummaries.mockResolvedValue({ records: [], total: 0 });
    api.listNotifications.mockResolvedValue({ records: [], total: 0, unread: 0 });
    api.inspectCoverCache.mockResolvedValue({
      healthyFiles: 0,
      missingFiles: 0,
      orphanFiles: 0,
      invalidIndexes: 0,
      indexedBytes: 0,
      orphanBytes: 0,
    });
    api.listCoverCacheRules.mockResolvedValue([]);
    api.searchCoverCacheCandidates.mockResolvedValue([]);
    api.previewCoverCacheCleanup.mockResolvedValue({
      files: 0,
      videos: 0,
      estimatedBytes: 0,
      mode: "keepListed",
    });
    api.runCoverCacheCleanup.mockResolvedValue({
      deletedFiles: 0,
      deletedIndexes: 0,
      releasedBytes: 0,
    });
    api.cleanupBrokenCoverCache.mockResolvedValue({
      deletedFiles: 0,
      deletedIndexes: 0,
      releasedBytes: 0,
    });
    api.getStartupRecoveryStatus.mockResolvedValue({
      state: "ready",
      canResolve: false,
      canRetry: false,
    });
    api.importLegacySettings.mockResolvedValue(1);
    api.initializeOnboardingDataRoot.mockResolvedValue({
      dataRoot: "C:\\Archive",
    });
    api.listContent.mockResolvedValue({ records: [], total: 0 });
    api.listLibraryContent.mockResolvedValue({ records: [], total: 0 });
    api.listLocalFolders.mockResolvedValue([]);
    api.listRemoteFolders.mockResolvedValue([]);
    api.listLocalFolderContent.mockResolvedValue({ records: [], total: 0 });
    api.listRemoteFolderContent.mockResolvedValue({ records: [], total: 0 });
    api.searchLibraryContent.mockResolvedValue({ records: [], total: 0 });
    api.addLocalFolderItem.mockResolvedValue({
      insertedItem: true,
      insertedProvenance: true,
    });
    api.openContentUrl.mockResolvedValue(undefined);
    api.getSyncStatus.mockResolvedValue({
      ...syncStatus("idle"),
      status: "queued",
      queuePosition: 1,
    });
    api.updateGlobalSettings.mockResolvedValue(1);
    api.updateWorkspaceSettings.mockResolvedValue(1);
    api.updateAccountSettings.mockResolvedValue(1);
    api.chooseBackupDirectory.mockResolvedValue(null);
    api.chooseBackupExportPath.mockResolvedValue(null);
    api.chooseBackupImportPath.mockResolvedValue(null);
    api.exportBackup.mockResolvedValue({
      createdAt: "2026-08-15T03:00:00.000+08:00",
      applicationVersion: "0.1.0",
      databaseSchemaVersion: 7,
      settingsSchemaVersion: 6,
      byteSize: 4_096,
    });
    api.inspectBackupImport.mockResolvedValue({
      createdAt: "2026-08-15T03:00:00.000+08:00",
      applicationVersion: "0.1.0",
      databaseSchemaVersion: 7,
      settingsSchemaVersion: 6,
      byteSize: 4_096,
    });
    api.restoreBackupReplace.mockResolvedValue({
      imported: {
        createdAt: "2026-08-15T03:00:00.000+08:00",
        applicationVersion: "0.1.0",
        databaseSchemaVersion: 7,
        settingsSchemaVersion: 6,
        byteSize: 4_096,
      },
      safetyBackupPath: "D:\\EmilyBackups\\emily-before-restore.emily-backup",
      activeAccountId: null,
    });
    api.prepareBackupMerge.mockResolvedValue({
      schemaVersion: 1,
      archive: {
        createdAt: "2026-08-15T03:00:00.000+08:00",
        applicationVersion: "0.1.0",
        databaseSchemaVersion: 7,
        settingsSchemaVersion: 6,
        byteSize: 4_096,
      },
      currentFingerprint: "a".repeat(64),
      archiveFingerprint: "b".repeat(64),
      planHash: "c".repeat(64),
      counts: {
        accounts: 2,
        videos: 3,
        watchRecords: 4,
        likes: 1,
        favorites: 1,
        remoteFolders: 1,
        localFolders: 1,
      },
      localFolderConflicts: [],
      settingsConflicts: [],
    });
    api.restoreBackupMerge.mockResolvedValue({
      imported: {
        createdAt: "2026-08-15T03:00:00.000+08:00",
        applicationVersion: "0.1.0",
        databaseSchemaVersion: 7,
        settingsSchemaVersion: 6,
        byteSize: 4_096,
      },
      merged: {
        accounts: 2,
        videos: 3,
        watchRecords: 4,
        likes: 1,
        favorites: 1,
        remoteFolders: 1,
        localFolders: 1,
      },
      safetyBackupPath: "D:\\EmilyBackups\\emily-before-restore.emily-backup",
    });
    api.deleteRetainedDataRoot.mockResolvedValue(undefined);
    api.listenAppEvent.mockResolvedValue(vi.fn());
    api.takeDesktopIntents.mockResolvedValue({
      revision: 0,
      showMain: false,
      openSettings: false,
    });
    api.commitOnboardingStep.mockResolvedValue({
      portableRevision: 1,
      device: {
        revision: 2,
        onboardingCompleted: false,
        onboardingStep: 1,
        launchAtStartup: false,
        silentAutostart: false,
        schedulerPaused: false,
        accounts: {},
      },
    });
    api.setSchedulerPaused.mockResolvedValue(undefined);
    api.setAutostart.mockImplementation(async (request: { enabled: boolean; silent: boolean }) => ({
      ...desktopState().device,
      revision: 2,
      launchAtStartup: request.enabled,
      silentAutostart: request.enabled && request.silent,
    }));
    api.setBackupDirectory.mockImplementation(async ({ path }: { path: string | null }) => ({
      ...desktopState().device,
      revision: 2,
      backupDirectory: path,
    }));
    api.setBrowserPreference.mockResolvedValue({
      ...desktopState().device,
      revision: 2,
      accounts: {
        "0123456789abcdef0123456789abcdef": { browserPreference: "chrome" },
      },
    });
    api.getSyncBatch.mockResolvedValue({
      id: 77,
      trigger: "manual",
      scope: "currentAccountAvailableSources",
      status: "running",
      createdAt: "2026-08-12T00:00:00.000Z",
      finishedAt: null,
      counts: { jobs: 1, skipped: 0, conflicts: 0 },
    });
    api.listSyncBatchEntries.mockResolvedValue({
      batchId: 77,
      entries: [],
      nextCursor: "batch-cursor",
      hasMore: false,
    });
    api.listSchedulerInstances.mockResolvedValue({
      accountId: 11,
      source: "remoteFolder",
      portableSettingsRevision: 0,
      registryRevision: 0,
      instances: [],
      nextCursor: null,
      hasMore: false,
    });
    api.enqueueSyncTasks.mockResolvedValue(enqueueJobResult());
    api.enqueueWorkspaceSync.mockResolvedValue(enqueueJobResult());
    downloadApi.previewDownloadSelection.mockResolvedValue({
      candidateCount: 1,
      contentTypeCounts: { video: 1 },
    });
    downloadApi.listDownloadJobs.mockResolvedValue([]);
    downloadApi.listenForDownloadRefresh.mockResolvedValue(vi.fn());
    dialogApi.open.mockResolvedValue(null);
  });

  afterEach(() => cleanup());

  it("启动恢复阻塞时只显示明确恢复操作且不调用普通 IPC", async () => {
    api.getStartupRecoveryStatus.mockResolvedValue({
      state: "blocked",
      errorCode: "startup_recovery_blocked",
      canResolve: true,
      canRetry: false,
    });

    render(App);

    expect(await screen.findByRole("heading", { name: "需要确认启动恢复方式" })).toBeVisible();
    expect(screen.getByRole("button", { name: "保留排队任务并恢复" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "取消排队任务并恢复" })).toBeEnabled();
    expect(api.listAccounts).not.toHaveBeenCalled();
    expect(api.getAppSettings).not.toHaveBeenCalled();
  });

  it.each([
    ["保留排队任务并恢复", "preserveQueued"],
    ["取消排队任务并恢复", "cancelQueued"],
  ] as const)("%s 后在同一进程继续正常初始化", async (buttonName, disposition) => {
    api.getStartupRecoveryStatus
      .mockResolvedValueOnce({
        state: "blocked",
        errorCode: "startup_recovery_blocked",
        canResolve: true,
        canRetry: false,
      })
      .mockResolvedValueOnce({ state: "ready", canResolve: false, canRetry: false });
    api.resolveStartupRecoveryFault.mockResolvedValue(undefined);

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: buttonName }));

    await waitFor(() =>
      expect(api.resolveStartupRecoveryFault).toHaveBeenCalledWith(disposition),
    );
    await waitFor(() => expect(api.listAccounts).toHaveBeenCalled());
    expect(screen.queryByRole("heading", { name: "需要确认启动恢复方式" })).not.toBeInTheDocument();
  });

  it("无账号时仍可打开设置", async () => {
    render(App);
    await waitFor(() => expect(api.getActiveAccount).toHaveBeenCalled());

    await fireEvent.click(screen.getByTitle("设置"));

    expect(screen.getByRole("region", { name: "设置" })).toBeVisible();
    expect(api.getAppSettings).toHaveBeenCalledWith(null);
  });

  it("全新安装显示四步引导并从后端步骤继续", async () => {
    api.getDesktopState.mockResolvedValue(
      desktopState({
        device: {
          revision: 4,
          onboardingCompleted: false,
          onboardingStep: 0,
          launchAtStartup: false,
          silentAutostart: false,
          schedulerPaused: false,
          accounts: {},
        },
      }),
    );

    render(App);

    expect(await screen.findByRole("dialog", { name: "首次设置" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "账号登录" })).toBeVisible();
    expect(screen.queryByLabelText("应用侧栏")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "一键登录" })).toBeEnabled();
    await fireEvent.click(screen.getByRole("button", { name: "跳过账号" }));
    await waitFor(() =>
      expect(api.commitOnboardingStep).toHaveBeenCalledWith({
        expectedPortableRevision: 0,
        expectedDeviceRevision: 4,
        step: { kind: "account", skipped: true },
      }),
    );
  });

  it("首次引导登录成功后使用刷新后的设备修订提交下一步", async () => {
    const active = account("active");
    const initialDesktop = desktopState({
      device: {
        ...desktopState().device,
        revision: 4,
        onboardingCompleted: false,
        onboardingStep: 0,
      },
    });
    const refreshedDesktop = desktopState({
      device: {
        ...initialDesktop.device,
        revision: 5,
        accounts: {
          [active.profileKey]: { browserPreference: "auto" },
        },
      },
    });
    api.getDesktopState
      .mockResolvedValueOnce(initialDesktop)
      .mockResolvedValue(refreshedDesktop);
    api.listAccounts.mockResolvedValueOnce([]).mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValueOnce(null).mockResolvedValue(active);
    api.beginAccountLogin.mockResolvedValue(active);

    render(App);

    await fireEvent.click(await screen.findByRole("button", { name: "一键登录" }));
    const next = await screen.findByRole("button", { name: "下一步" });
    await waitFor(() => expect(next).toBeEnabled());
    await fireEvent.click(next);

    await waitFor(() =>
      expect(api.commitOnboardingStep).toHaveBeenCalledWith({
        expectedPortableRevision: 0,
        expectedDeviceRevision: 5,
        step: { kind: "account", skipped: false },
      }),
    );
  });

  it("首次引导已有活动账号时隐藏跳过账号", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.getDesktopState.mockResolvedValue(
      desktopState({
        device: {
          ...desktopState().device,
          revision: 5,
          onboardingCompleted: false,
          onboardingStep: 0,
        },
      }),
    );

    render(App);

    expect(await screen.findByText("账号已就绪")).toBeVisible();
    expect(screen.queryByRole("button", { name: "跳过账号" })).not.toBeInTheDocument();
  });

  it("首次引导第二步合并同步来源、范围、自动同步和周期", async () => {
    api.getDesktopState.mockResolvedValue(
      desktopState({
        device: {
          ...desktopState().device,
          onboardingCompleted: false,
          onboardingStep: 1,
        },
      }),
    );

    render(App);

    expect(await screen.findByRole("heading", { name: "同步设置" })).toBeVisible();
    expect(screen.getByLabelText("引导观看历史天数")).toBeVisible();
    expect(screen.getByLabelText("观看历史同步间隔")).toBeVisible();
    expect(screen.queryByLabelText("完成引导后立即同步")).not.toBeInTheDocument();
  });

  it("首次引导允许回退到已提交步骤而不产生新的提交", async () => {
    api.getDesktopState.mockResolvedValue(
      desktopState({
        device: {
          ...desktopState().device,
          onboardingCompleted: false,
          onboardingStep: 1,
        },
      }),
    );

    render(App);

    expect(await screen.findByRole("heading", { name: "同步设置" })).toBeVisible();
    await fireEvent.click(screen.getByRole("button", { name: /上一步/ }));

    expect(await screen.findByRole("heading", { name: "账号登录" })).toBeVisible();
    expect(api.commitOnboardingStep).not.toHaveBeenCalled();
  });

  it("首次引导选择数据目录后在下一步自动初始化且不显示迁移操作", async () => {
    api.getDesktopState.mockResolvedValue(
      desktopState({
        device: {
          ...desktopState().device,
          onboardingCompleted: false,
          onboardingStep: 2,
        },
      }),
    );
    api.chooseDataRoot.mockResolvedValue("C:\\Archive");
    api.validateDataRoot.mockResolvedValue({
      requestedPath: "C:\\Archive",
      normalizedPath: "C:\\Archive",
      adjustedToInstallData: false,
    });

    render(App);

    expect(await screen.findByRole("heading", { name: "数据与桌面" })).toBeVisible();
    await fireEvent.click(screen.getByRole("button", { name: "选择数据目录" }));
    expect(await screen.findByLabelText("引导当前数据目录")).toHaveValue("C:\\Archive");
    expect(screen.queryByText("目标目录")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "迁移数据" })).not.toBeInTheDocument();
    await fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    await waitFor(() =>
      expect(api.initializeOnboardingDataRoot).toHaveBeenCalledWith("C:\\Archive"),
    );
    await waitFor(() => expect(api.commitOnboardingStep).toHaveBeenCalled());
    expect(api.initializeOnboardingDataRoot.mock.invocationCallOrder[0]).toBeLessThan(
      api.commitOnboardingStep.mock.invocationCallOrder[0],
    );
    expect(await screen.findByRole("heading", { name: "确认设置" })).toBeVisible();
    expect(screen.getByText("C:\\Archive")).toBeVisible();
  });

  it("首次引导第三步配置数据位置和桌面行为", async () => {
    api.getDesktopState.mockResolvedValue(
      desktopState({
        device: {
          ...desktopState().device,
          onboardingCompleted: false,
          onboardingStep: 2,
        },
      }),
    );

    render(App);

    expect(await screen.findByRole("heading", { name: "数据与桌面" })).toBeVisible();
    expect(screen.getByLabelText("引导当前数据目录")).toHaveValue(
      "C:\\Users\\Test\\AppData\\Emily\\Data",
    );
    expect(screen.getByLabelText("关闭时退出应用")).toBeVisible();
    expect(screen.getByLabelText("开机自动启动")).toBeVisible();
  });

  it("首次引导第四步复核设置并选择是否立即同步", async () => {
    api.getDesktopState.mockResolvedValue(
      desktopState({
        device: {
          ...desktopState().device,
          onboardingCompleted: false,
          onboardingStep: 3,
        },
      }),
    );

    render(App);

    expect(await screen.findByRole("heading", { name: "确认设置" })).toBeVisible();
    expect(screen.getByText("设置摘要")).toBeVisible();
    expect(screen.getByLabelText("完成引导后立即同步")).toBeVisible();
  });

  it("设置页提供九类导航并可搜索定位设置项", async () => {
    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));

    const settingsNavigation = await screen.findByRole("navigation", { name: "设置分类" });
    expect(settingsNavigation).toBeVisible();
    expect(settingsNavigation.children).toHaveLength(9);
    expect(screen.getByRole("navigation", { name: "设置子项" })).toBeVisible();
    expect(screen.getByRole("button", { name: "外观" })).toHaveAttribute("aria-current", "page");
    expect(screen.queryByLabelText("消息提示停留时间")).not.toBeVisible();

    await fireEvent.input(screen.getByRole("searchbox", { name: "搜索设置" }), {
      target: { value: "消息提示停留时间" },
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "通知" })).toHaveAttribute("aria-current", "page"),
    );
    expect(screen.getByRole("button", { name: "通知偏好" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByLabelText("消息提示停留时间").closest(".settings-section")).toHaveClass("settings-search-hit");
  });

  it("设置页初始化和未发生变更时不显示已保存", async () => {
    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));

    expect(screen.queryByText("已保存")).not.toBeInTheDocument();
    const systemTheme = screen.getByRole("button", { name: "跟随系统" });
    await waitFor(() => expect(systemTheme).toHaveAttribute("aria-pressed", "true"));
    api.updateGlobalSettings.mockClear();
    await fireEvent.click(systemTheme);
    expect(api.updateGlobalSettings).not.toHaveBeenCalled();
    expect(screen.queryByText("已保存")).not.toBeInTheDocument();
  });

  it("为每个一级页面分别恢复滚动位置", async () => {
    render(App);
    await waitFor(() => expect(api.getDesktopState).toHaveBeenCalled());
    const viewport = document.querySelector<HTMLElement>(".workspace-main");
    expect(viewport).not.toBeNull();

    viewport!.scrollTop = 280;
    await fireEvent.scroll(viewport!);
    await fireEvent.click(screen.getByTitle("设置"));
    await waitFor(() => expect(viewport!.scrollTop).toBe(0));

    viewport!.scrollTop = 96;
    await fireEvent.scroll(viewport!);
    await fireEvent.click(screen.getByTitle("观看历史"));
    await waitFor(() => expect(viewport!.scrollTop).toBe(280));

    await fireEvent.click(screen.getByTitle("设置"));
    await waitFor(() => expect(viewport!.scrollTop).toBe(96));
  });

  it("已使用用户不重复显示首次引导", async () => {
    render(App);
    await waitFor(() => expect(api.getDesktopState).toHaveBeenCalled());
    expect(screen.queryByRole("dialog", { name: "首次设置" })).not.toBeInTheDocument();
  });

  it("侧栏使用应用品牌图片", async () => {
    render(App);
    await waitFor(() => expect(api.getDesktopState).toHaveBeenCalled());

    const brandImage = document.querySelector<HTMLImageElement>(".brand-mark img");
    expect(brandImage).toBeInTheDocument();
    expect(brandImage?.src).toContain("app-icon.png");
  });

  it("同步当前来源通过统一队列提交 scope one", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);

    render(App);

    const syncCurrent = await screen.findByRole("button", { name: "同步当前来源" });
    await waitFor(() => expect(syncCurrent).toBeEnabled());
    await fireEvent.click(syncCurrent);
    await waitFor(() =>
      expect(api.enqueueSyncTasks).toHaveBeenCalledWith({
        scope: "one",
        accountId: 11,
        source: "history",
        sourceInstanceKey: "",
        payload: null,
        fullReconcileConfirmationSha256: null,
      }),
    );
  });

  it("同步当前账号全部来源使用批量 scope 且不受自动开关影响", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);

    render(App);

    await fireEvent.click(await screen.findByRole("button", { name: /测试账号/ }));
    await fireEvent.click(
      screen.getByRole("button", { name: "同步当前账号全部来源" }),
    );
    await waitFor(() =>
      expect(api.enqueueSyncTasks).toHaveBeenCalledWith({
        scope: "currentAccountAvailableSources",
        accountId: 11,
        source: null,
        sourceInstanceKey: null,
        payload: null,
        fullReconcileConfirmationSha256: null,
      }),
    );
  });

  it("账号工作台按稳定账号范围提交聚合同步", async () => {
    const active = account("active");
    const second = {
      ...active,
      id: 12,
      accountKey: "sec:second-account",
      profileKey: "1123456789abcdef0123456789abcdef",
      nickname: "第二账号",
    };
    api.listAccounts.mockResolvedValue([active, second]);
    api.getActiveAccount.mockResolvedValue(active);
    const settings = appSettings() as unknown as AppSettingsView;
    settings.workspace.sync = {
      mode: "aggregate",
      accountScope: { kind: "selected", accountKeys: ["sec:second-account"] },
    };
    api.getAppSettings.mockResolvedValue(settings);

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "账号管理" }));
    const sync = await screen.findByRole("button", { name: "同步 1 个账号" });
    await fireEvent.click(sync);

    await waitFor(() =>
      expect(api.enqueueWorkspaceSync).toHaveBeenCalledWith({
        kind: "selected",
        accountIds: [12],
      }),
    );
  });

  it("设置页保存来源调度、启动延迟并暂停自动同步", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.getSchedulerState.mockResolvedValue(
      schedulerState({
        accounts: [
          {
            accountId: 11,
            accountStatus: "active",
            sourceStates: [
              {
                source: "history",
                available: true,
                unavailableReason: null,
                enabled: false,
                intervalMinutes: 1_440,
                instanceCount: 1,
                aggregateError: null,
                message: null,
                lastStatus: "partial",
                lastFinishedAt: "2026-08-12T01:00:00.000Z",
                nextDueAt: "2026-08-13T01:00:00.000Z",
                activeJobId: null,
              },
            ],
          },
        ],
      }),
    );

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    await selectSettingsPanel("账号与同步", "自动同步");

    expect(await screen.findByText(/上次：部分完成/)).toBeVisible();
    await fireEvent.click(screen.getByLabelText("观看历史自动同步"));
    await waitFor(() =>
      expect(api.updateAccountSettings).toHaveBeenCalledWith(
        11,
        expect.objectContaining({
          history: expect.objectContaining({
            schedule: { enabled: true, intervalMinutes: 1_440 },
          }),
        }),
      ),
    );
    await selectSettingsPanel("常规与外观", "桌面行为");
    await fireEvent.change(screen.getByLabelText("开机后首次同步延迟"), {
      target: { value: "12" },
    });
    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenCalledWith(
        expect.objectContaining({ startupSyncDelayMinutes: 12 }),
      ),
    );
    let finishSchedulerSave!: () => void;
    api.setSchedulerPaused.mockImplementationOnce(
      () => new Promise<void>((resolve) => {
        finishSchedulerSave = resolve;
      }),
    );
    await selectSettingsPanel("账号与同步", "自动同步");
    await fireEvent.click(screen.getByRole("button", { name: "暂停自动同步" }));
    await waitFor(() => expect(api.setSchedulerPaused).toHaveBeenCalledWith(true));
    expect(await screen.findByText("保存中")).toBeVisible();
    finishSchedulerSave();
    expect(await screen.findByText("已保存")).toBeVisible();
  });

  it("设置页保存自动备份周期、执行时间和保留份数", async () => {
    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));

    const automaticBackup = screen.getByLabelText("启用自动备份");
    await waitFor(() => expect(automaticBackup).toBeEnabled());
    await fireEvent.click(automaticBackup);
    await waitFor(() => expect(automaticBackup).toBeChecked());
    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({
          backup: {
            enabled: true,
            intervalDays: 7,
            executionTime: "03:00",
            retentionCount: 5,
          },
        }),
      ),
    );

    await fireEvent.change(screen.getByLabelText("自动备份间隔"), {
      target: { value: "14" },
    });
    await fireEvent.change(screen.getByLabelText("自动备份执行时间"), {
      target: { value: "04:30" },
    });
    await fireEvent.change(screen.getByLabelText("自动备份保留份数"), {
      target: { value: "8" },
    });

    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({
          backup: {
            enabled: true,
            intervalDays: 14,
            executionTime: "04:30",
            retentionCount: 8,
          },
        }),
      ),
    );
  });

  it("设置页保存通知开关、连续失败阈值和消息停留时间", async () => {
    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));

    const syncCompleted = screen.getByLabelText("同步完成通知");
    await waitFor(() => expect(syncCompleted).toBeEnabled());
    await fireEvent.click(syncCompleted);
    await waitFor(() => expect(syncCompleted).toBeChecked());
    await fireEvent.change(screen.getByLabelText("连续自动同步失败阈值"), {
      target: { value: "5" },
    });
    await fireEvent.change(screen.getByLabelText("消息提示停留时间"), {
      target: { value: "12" },
    });

    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({
          notifications: {
            loginExpired: true,
            automaticFailureStreak: true,
            backupFailure: true,
            syncCompleted: true,
            backupCompleted: false,
            automaticFailureThreshold: 5,
            messageDurationSeconds: 12,
          },
        }),
      ),
    );
  });

  it("设置页保存自动更新偏好并保持下载依赖检查", async () => {
    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));

    const automaticCheck = screen.getByLabelText("自动检查更新");
    const automaticDownload = screen.getByLabelText("自动下载更新");
    await waitFor(() => expect(automaticCheck).toBeEnabled());
    expect(automaticCheck).not.toBeChecked();
    expect(automaticDownload).toBeDisabled();

    await fireEvent.click(automaticCheck);
    await waitFor(() => expect(automaticDownload).toBeEnabled());
    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({
          updates: {
            automaticCheck: true,
            automaticDownload: false,
          },
        }),
      ),
    );

    await fireEvent.click(automaticDownload);
    await waitFor(() => expect(automaticDownload).toBeChecked());
    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({
          updates: {
            automaticCheck: true,
            automaticDownload: true,
          },
        }),
      ),
    );

    await fireEvent.click(automaticCheck);
    await waitFor(() => expect(automaticDownload).toBeDisabled());
    expect(automaticDownload).not.toBeChecked();
    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({
          updates: {
            automaticCheck: false,
            automaticDownload: false,
          },
        }),
      ),
    );
  });

  it("手动检查下载后必须确认才安装更新", async () => {
    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    await selectSettingsPanel("更新", "应用更新");

    await fireEvent.click(await screen.findByRole("button", { name: "检查更新" }));
    await waitFor(() => expect(api.checkForUpdates).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("发现新版本 0.2.0")).toBeVisible();
    expect(screen.getByText("稳定性更新")).toBeVisible();

    await fireEvent.click(screen.getByRole("button", { name: "下载更新" }));
    await waitFor(() => expect(api.downloadUpdate).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("更新已下载，等待确认安装")).toBeVisible();

    await fireEvent.click(screen.getByRole("button", { name: "安装更新" }));
    expect(api.installUpdate).not.toHaveBeenCalled();
    const dialog = screen.getByRole("dialog", { name: "确认安装更新" });
    expect(dialog).toHaveTextContent("安装前会停止后台任务并创建安全备份");

    await fireEvent.click(screen.getByRole("button", { name: "确认安装" }));
    await waitFor(() => expect(api.installUpdate).toHaveBeenCalledTimes(1));
  });

  it("备份目录使用设备修订写入且导出前明确确认未加密风险", async () => {
    api.getDesktopState.mockResolvedValue(
      desktopState({
        device: {
          ...desktopState().device,
          backupDirectory: "D:\\EmilyBackups",
        },
      }),
    );
    api.chooseBackupDirectory.mockResolvedValue("E:\\Archives");
    api.chooseBackupExportPath.mockResolvedValue("E:\\Exports\\emily-backup.emily-backup");

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    await selectSettingsPanel("通知", "通知偏好");
    const messageDuration = screen.getByLabelText("消息提示停留时间");
    await waitFor(() => expect(messageDuration).toBeEnabled());
    await fireEvent.change(messageDuration, {
      target: { value: "1" },
    });
    await waitFor(() => expect(messageDuration).toHaveValue(1));
    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({
          notifications: expect.objectContaining({ messageDurationSeconds: 1 }),
        }),
      ),
    );

    await selectSettingsPanel("备份与恢复", "备份与恢复");
    const backupDirectory = screen.getByLabelText("当前备份目录");
    await waitFor(() => expect(backupDirectory).toHaveValue("D:\\EmilyBackups"));
    await fireEvent.click(screen.getByRole("button", { name: "选择备份目录" }));
    await waitFor(() =>
      expect(api.setBackupDirectory).toHaveBeenCalledWith({
        path: "E:\\Archives",
        expectedDeviceRevision: 1,
      }),
    );
    expect(backupDirectory).toHaveValue("E:\\Archives");

    await fireEvent.click(screen.getByRole("button", { name: "导出备份" }));
    const dialog = screen.getByRole("dialog", { name: "导出备份" });
    expect(dialog).toHaveTextContent("备份不会加密");
    expect(dialog).toHaveTextContent("完整个人归档");
    expect(api.chooseBackupExportPath).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole("button", { name: "确认导出" }));
    await waitFor(() =>
      expect(api.exportBackup).toHaveBeenCalledWith(
        "E:\\Exports\\emily-backup.emily-backup",
      ),
    );
    expect(await screen.findByText(/备份已导出/)).toBeVisible();
    await waitFor(
      () => expect(screen.queryByText(/备份已导出/)).not.toBeInTheDocument(),
      { timeout: 1_500 },
    );
  });

  it("导入备份先检查摘要再确认直接替换并刷新全部数据", async () => {
    const active = account("active");
    const archivePath = "E:\\Imports\\emily-backup.emily-backup";
    const restoreResult = {
      imported: {
        createdAt: "2026-08-15T03:00:00.000+08:00",
        applicationVersion: "0.1.0",
        databaseSchemaVersion: 7,
        settingsSchemaVersion: 6,
        byteSize: 4_096,
      },
      safetyBackupPath: "D:\\EmilyBackups\\emily-before-restore.emily-backup",
      activeAccountId: active.id,
    };
    let completeRestore: ((value: typeof restoreResult) => void) | undefined;
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.chooseBackupImportPath.mockResolvedValue(archivePath);
    api.restoreBackupReplace.mockImplementation(
      () =>
        new Promise<typeof restoreResult>((resolve) => {
          completeRestore = resolve;
        }),
    );

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    await selectSettingsPanel("备份与恢复", "备份与恢复");
    const importButton = screen.getByRole("button", { name: "导入备份" });
    await waitFor(() => expect(importButton).toBeEnabled());
    await fireEvent.click(importButton);

    await waitFor(() => expect(api.inspectBackupImport).toHaveBeenCalledWith(archivePath));
    const dialog = screen.getByRole("dialog", { name: "导入备份" });
    expect(dialog).toHaveTextContent("直接替换");
    expect(dialog).toHaveTextContent("2026-08-15");
    expect(dialog).toHaveTextContent("数据库 schema 7");
    expect(dialog).toHaveTextContent("恢复前会自动创建安全备份");
    expect(dialog).toHaveTextContent("去重合并会保留当前数据");
    expect(api.restoreBackupReplace).not.toHaveBeenCalled();
    const scrollSpeed = screen.getByLabelText("滚动速率");
    await waitFor(() => expect(scrollSpeed).toBeEnabled());

    api.listAccounts.mockClear();
    api.getActiveAccount.mockClear();
    api.getAppSettings.mockClear();
    api.getDesktopState.mockClear();
    api.getSchedulerState.mockClear();
    api.getSyncStatus.mockClear();
    api.listContent.mockClear();
    await fireEvent.click(screen.getByRole("button", { name: "确认直接替换" }));

    await waitFor(() => expect(api.restoreBackupReplace).toHaveBeenCalledWith(archivePath));
    expect(scrollSpeed).toBeDisabled();
    completeRestore?.(restoreResult);
    await waitFor(() => {
      expect(api.listAccounts).toHaveBeenCalled();
      expect(api.getActiveAccount).toHaveBeenCalled();
      expect(api.getAppSettings).toHaveBeenCalled();
      expect(api.getDesktopState).toHaveBeenCalled();
      expect(api.getSchedulerState).toHaveBeenCalled();
      expect(api.listContent).toHaveBeenCalled();
    });
    expect(await screen.findByText(/备份恢复完成/)).toBeVisible();
  });

  it("导入备份可生成去重合并计划并提交显式冲突决议", async () => {
    const active = account("active");
    const archivePath = "E:\\Imports\\emily-backup.emily-backup";
    const plan = {
      schemaVersion: 1,
      archive: {
        createdAt: "2026-08-15T03:00:00.000+08:00",
        applicationVersion: "0.1.0",
        databaseSchemaVersion: 7,
        settingsSchemaVersion: 6,
        byteSize: 4_096,
      },
      currentFingerprint: "a".repeat(64),
      archiveFingerprint: "b".repeat(64),
      planHash: "c".repeat(64),
      counts: {
        accounts: 2,
        videos: 3,
        watchRecords: 4,
        likes: 1,
        favorites: 1,
        remoteFolders: 1,
        localFolders: 1,
      },
      localFolderConflicts: [
        {
          id: "folder-1",
          scope: "account",
          accountKey: "sec:shared-sec",
          name: "稍后再看",
        },
      ],
      settingsConflicts: [
        {
          id: "setting-1",
          path: "global.theme",
          currentValue: "dark",
          backupValue: "light",
        },
      ],
    };
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.chooseBackupImportPath.mockResolvedValue(archivePath);
    api.prepareBackupMerge.mockResolvedValue(plan);

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    await selectSettingsPanel("备份与恢复", "备份与恢复");
    const importButton = screen.getByRole("button", { name: "导入备份" });
    await waitFor(() => expect(importButton).toBeEnabled());
    await fireEvent.click(importButton);
    await waitFor(() => expect(api.inspectBackupImport).toHaveBeenCalledWith(archivePath));

    await fireEvent.click(screen.getByRole("button", { name: "准备去重合并" }));
    await waitFor(() => expect(api.prepareBackupMerge).toHaveBeenCalledWith(archivePath));
    expect(await screen.findByText("待合并记录")).toBeVisible();
    expect(screen.getByText("本地收藏夹冲突")).toBeVisible();
    await fireEvent.change(screen.getByLabelText("收藏夹“稍后再看”的处理方式"), {
      target: { value: "merge" },
    });
    await fireEvent.change(screen.getByLabelText("设置冲突处理"), {
      target: { value: "backup" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "确认去重合并" }));

    await waitFor(() =>
      expect(api.restoreBackupMerge).toHaveBeenCalledWith(
        archivePath,
        plan,
        { "folder-1": { action: "merge" } },
        { mode: "backup" },
      ),
    );
    expect(await screen.findByText(/备份合并完成/)).toBeVisible();
  });

  it("喜欢范围支持数量设置并禁用不受 capability 支持的最近天数", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.getSchedulerState.mockResolvedValue(
      schedulerState({
        capabilities: schedulerState().capabilities.map((capability) =>
          capability.source === "likes"
            ? {
                ...capability,
                available: true,
                unavailableReason: null,
                supportedRanges: ["all", "recentItems", "recordLimit"],
              }
            : capability,
        ),
      }),
    );

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    await selectSettingsPanel("账号与同步", "自动同步");

    const range = await screen.findByLabelText("喜欢同步范围");
    const recentDays = Array.from((range as HTMLSelectElement).options).find(
      (option) => option.value === "recentDays",
    );
    expect(recentDays).toBeDisabled();
    await fireEvent.change(range, { target: { value: "recentItems" } });
    await fireEvent.change(await screen.findByLabelText("喜欢最近条数"), {
      target: { value: "321" },
    });

    await waitFor(() =>
      expect(api.updateAccountSettings).toHaveBeenLastCalledWith(
        11,
        expect.objectContaining({
          likes: expect.objectContaining({
            range: { kind: "recentItems", count: 321 },
          }),
        }),
      ),
    );
  });

  it("后端拒绝最近天数时显示错误并恢复原范围", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.getSchedulerState.mockResolvedValue(
      schedulerState({
        capabilities: schedulerState().capabilities.map((capability) =>
          capability.source === "likes"
            ? {
                ...capability,
                available: true,
                unavailableReason: null,
                supportedRanges: ["all", "recentItems", "recordLimit", "recentDays"],
              }
            : capability,
        ),
      }),
    );
    api.updateAccountSettings.mockRejectedValueOnce(new Error("time_range_not_supported"));

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    await selectSettingsPanel("账号与同步", "自动同步");

    const range = await screen.findByLabelText("喜欢同步范围");
    await fireEvent.change(range, { target: { value: "recentDays" } });

    expect(await screen.findByText("time_range_not_supported")).toBeVisible();
    await waitFor(() => expect(range).toHaveValue("recentItems"));
  });

  it("桌面行为设置使用设备事务且浏览器偏好提示重新登录", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    let finishAutostartSave!: (value: ReturnType<typeof desktopState>["device"]) => void;
    api.setAutostart.mockImplementationOnce(
      () => new Promise((resolve) => {
        finishAutostartSave = resolve;
      }),
    );

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    await selectSettingsPanel("常规与外观", "桌面行为");

    await fireEvent.click(screen.getByLabelText("开机自动启动"));
    await waitFor(() =>
      expect(api.setAutostart).toHaveBeenCalledWith({
        expectedDeviceRevision: 1,
        enabled: true,
        silent: false,
      }),
    );
    expect(finishAutostartSave).toBeTypeOf("function");
    expect(await screen.findByText("保存中")).toBeVisible();
    finishAutostartSave({
      ...desktopState().device,
      revision: 2,
      launchAtStartup: true,
    });
    expect(await screen.findByText("已保存")).toBeVisible();

    let finishBrowserSave!: (value: ReturnType<typeof desktopState>["device"]) => void;
    api.setBrowserPreference.mockImplementationOnce(
      () => new Promise((resolve) => {
        finishBrowserSave = resolve;
      }),
    );
    await selectSettingsPanel("账号与同步", "浏览器");
    await fireEvent.click(screen.getByRole("button", { name: "Chrome" }));
    await waitFor(() =>
      expect(api.setBrowserPreference).toHaveBeenCalledWith({
        accountId: 11,
        browserPreference: "chrome",
        confirmRelogin: true,
      }),
    );
    expect(await screen.findByText("保存中")).toBeVisible();
    finishBrowserSave({
      ...desktopState().device,
      revision: 3,
      launchAtStartup: true,
      accounts: {
        "0123456789abcdef0123456789abcdef": { browserPreference: "chrome" },
      },
    });
    expect(await screen.findByText("已保存")).toBeVisible();
    expect(await screen.findByText("重新登录后生效")).toBeVisible();
  });

  it("开机启动写入失败时恢复开关状态", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.setAutostart.mockRejectedValueOnce(new Error("autostart_write_failed"));

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    const checkbox = screen.getByLabelText("开机自动启动");
    await fireEvent.click(checkbox);

    expect(await screen.findByText("autostart_write_failed")).toBeVisible();
    expect(await screen.findByText("保存失败")).toBeVisible();
    await waitFor(() => expect(checkbox).not.toBeChecked());
  });

  it("批次首屏后继续分页，并在活动批次耗尽后响应事件追加", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    const handlers = new Map<string, () => void>();
    api.listenAppEvent.mockImplementation(async (name: string, handler: () => void) => {
      handlers.set(name, handler);
      return vi.fn();
    });
    api.enqueueSyncTasks.mockResolvedValue({
      batchId: 77,
      counts: { jobs: 1, skipped: 0, conflicts: 0 },
      entries: [
        {
          kind: "job",
          data: {
            id: 23,
            accountId: 11,
            source: "history",
            sourceInstanceKey: "",
            trigger: "manual",
            status: "queued",
            queuedAt: "2026-08-12T00:00:00.000Z",
            scheduledFor: null,
            startedAt: null,
            finishedAt: null,
            queuePosition: 1,
            canCancel: true,
            issues: [],
          },
        },
      ],
      nextCursor: "cursor-1",
    });
    api.listSyncBatchEntries
      .mockResolvedValueOnce({
        batchId: 77,
        entries: [],
        nextCursor: "cursor-1",
        hasMore: false,
      })
      .mockResolvedValueOnce({
        batchId: 77,
        entries: [
          {
            kind: "skipped",
            data: {
              accountId: 11,
              source: "likes",
              sourceInstanceKey: "",
              phase: "discoveryExpansion",
              reason: "collector_not_implemented",
              requestedPayloadSha256: null,
            },
          },
        ],
        nextCursor: "cursor-2",
        hasMore: false,
      });

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: /测试账号/ }));
    await fireEvent.click(screen.getByRole("button", { name: "同步当前账号全部来源" }));
    await waitFor(() => expect(handlers.has("sync:changed")).toBe(true));
    handlers.get("sync:changed")?.();
    await waitFor(() =>
      expect(api.listSyncBatchEntries).toHaveBeenCalledWith(77, "cursor-1", 200),
    );

    handlers.get("sync:changed")?.();
    await waitFor(() => expect(api.listSyncBatchEntries).toHaveBeenCalledTimes(2));
  });

  it("批次游标失效时从第一页重新加载", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    const handlers = new Map<string, () => void>();
    api.listenAppEvent.mockImplementation(async (name: string, handler: () => void) => {
      handlers.set(name, handler);
      return vi.fn();
    });
    api.enqueueSyncTasks.mockResolvedValue({
      batchId: 77,
      counts: { jobs: 1, skipped: 0, conflicts: 0 },
      entries: [],
      nextCursor: "stale-cursor",
    });
    api.listSyncBatchEntries
      .mockRejectedValueOnce(new Error("invalid_batch_cursor"))
      .mockResolvedValueOnce({
        batchId: 77,
        entries: [
          {
            kind: "skipped",
            data: {
              accountId: 11,
              source: "likes",
              sourceInstanceKey: "",
              phase: "initialExpansion",
              reason: "collector_not_implemented",
              requestedPayloadSha256: null,
            },
          },
        ],
        nextCursor: "fresh-cursor",
        hasMore: false,
      });

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: /测试账号/ }));
    await fireEvent.click(screen.getByRole("button", { name: "同步当前账号全部来源" }));
    await waitFor(() => expect(handlers.has("sync:changed")).toBe(true));
    handlers.get("sync:changed")?.();

    await waitFor(() => {
      expect(api.listSyncBatchEntries).toHaveBeenNthCalledWith(1, 77, "stale-cursor", 200);
      expect(api.listSyncBatchEntries).toHaveBeenNthCalledWith(2, 77, null, 200);
    });
  });

  it("远端收藏夹分页在 revision 失效时自动从第一页重载", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.getSchedulerState.mockResolvedValue(
      schedulerState({
        capabilities: [
          ...schedulerState().capabilities.filter((item) => item.source !== "favorites" && item.source !== "remoteFolder"),
          {
            source: "favorites",
            available: true,
            unavailableReason: null,
            supportedRanges: ["all", "recentItems", "recordLimit"],
            supportsFullReconcile: true,
            supportsRemoteFolders: true,
          },
          {
            source: "remoteFolder",
            available: true,
            unavailableReason: null,
            supportedRanges: ["all", "recentItems", "recordLimit"],
            supportsFullReconcile: true,
            supportsRemoteFolders: true,
          },
        ],
      }),
    );
    api.listSchedulerInstances
      .mockResolvedValueOnce({
        accountId: 11,
        source: "remoteFolder",
        portableSettingsRevision: 4,
        registryRevision: 8,
        instances: [
          {
            remoteFolderId: "folder-a",
            name: "收藏夹 A",
            range: { kind: "all" },
            schedule: { enabled: true, intervalMinutes: 1_440 },
          },
        ],
        nextCursor: "folder-cursor",
        hasMore: true,
      })
      .mockRejectedValueOnce(new Error("scheduler_instances_changed"))
      .mockResolvedValueOnce({
        accountId: 11,
        source: "remoteFolder",
        portableSettingsRevision: 5,
        registryRevision: 9,
        instances: [
          {
            remoteFolderId: "folder-b",
            name: "收藏夹 B",
            range: { kind: "all" },
            schedule: { enabled: false, intervalMinutes: 1_440 },
          },
        ],
        nextCursor: null,
        hasMore: false,
      });

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    await selectSettingsPanel("账号与同步", "远端收藏夹");
    await fireEvent.click(await screen.findByRole("button", { name: "查看远端收藏夹设置" }));
    expect(await screen.findByText("收藏夹 A")).toBeVisible();
    await fireEvent.click(screen.getByRole("button", { name: "加载更多收藏夹" }));
    expect(await screen.findByText("收藏夹 B")).toBeVisible();
    expect(api.listSchedulerInstances).toHaveBeenLastCalledWith(11, "remoteFolder", null, 200);
  });

  it("托盘不可用时引导不能选择关闭到托盘", async () => {
    api.getDesktopState.mockResolvedValue(
      desktopState({
        trayAvailable: false,
        trayError: "tray_unavailable",
        device: {
          revision: 4,
          onboardingCompleted: false,
          onboardingStep: 2,
          launchAtStartup: false,
          silentAutostart: false,
          schedulerPaused: false,
          accounts: {},
        },
      }),
    );

    render(App);

    expect(await screen.findByRole("heading", { name: "数据与桌面" })).toBeVisible();
    expect(screen.getByLabelText("关闭时隐藏到托盘")).toBeDisabled();
    expect(screen.getByText("系统托盘当前不可用")).toBeVisible();
  });

  it("监听建立后通过持久桌面意图打开设置", async () => {
    api.takeDesktopIntents.mockResolvedValue({
      revision: 7,
      showMain: true,
      openSettings: true,
    });

    render(App);

    expect(await screen.findByRole("region", { name: "设置" })).toBeVisible();
    expect(api.listenAppEvent).toHaveBeenCalledWith("desktop:intent-changed", expect.any(Function));
    expect(api.takeDesktopIntents).toHaveBeenCalledWith(0);
  });

  it("侧栏在概览与同步记录独立页面间切换", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.listTaskSummaries.mockResolvedValue({ records: [], total: 0 });

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "概览" }));
    expect(await screen.findByRole("region", { name: "概览" })).toBeVisible();
    await waitFor(() =>
      expect(api.getOverviewScope).toHaveBeenCalledWith({
        accountScope: { kind: "all" },
        source: "all",
        range: "last30Days",
        includeRemoteRemoved: false,
      }),
    );

    await fireEvent.click(screen.getByRole("button", { name: "任务中心" }));
    expect(await screen.findByRole("region", { name: "任务中心" })).toBeVisible();
    await waitFor(() => expect(api.listTaskSummaries).toHaveBeenCalledWith({ kind: "all" }, 30, 0));
  });

  it("主题设置即时应用并持久化", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    const darkTheme = screen.getByRole("button", { name: "深色" });
    await waitFor(() => expect(darkTheme).toBeEnabled());
    await fireEvent.click(darkTheme);

    await waitFor(() => expect(document.documentElement).toHaveAttribute("data-theme", "dark"));
    expect(document.documentElement.style.colorScheme).toBe("dark");
    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({ theme: "dark" }),
      ),
    );
  });

  it("普通设置只在后端确认后显示已保存", async () => {
    let finishSave!: (revision: number) => void;
    api.updateGlobalSettings.mockImplementationOnce(
      () => new Promise<number>((resolve) => {
        finishSave = resolve;
      }),
    );

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    const darkTheme = screen.getByRole("button", { name: "深色" });
    await waitFor(() => expect(darkTheme).toBeEnabled());
    await fireEvent.click(darkTheme);

    expect(await screen.findByText("保存中")).toBeVisible();
    finishSave(2);
    expect(await screen.findByText("已保存")).toBeVisible();
  });

  it("已保存提示停留三秒后淡出", async () => {
    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    const timeoutSpy = vi.spyOn(window, "setTimeout");

    const darkTheme = screen.getByRole("button", { name: "深色" });
    await waitFor(() => expect(screen.getByRole("button", { name: "跟随系统" })).toHaveAttribute("aria-pressed", "true"));
    await fireEvent.click(darkTheme);
    const saved = await screen.findByText("已保存");
    const hideCall = timeoutSpy.mock.calls.find(([, delay]) => delay === 3_000);
    expect(hideCall).toBeDefined();

    const hideCallback = hideCall?.[0];
    expect(hideCallback).toBeTypeOf("function");
    if (typeof hideCallback === "function") hideCallback();
    await waitFor(() => expect(saved).toHaveClass("fading"));
    await fireEvent.animationEnd(saved);
    expect(screen.queryByText("已保存")).not.toBeInTheDocument();
    timeoutSpy.mockRestore();
  });

  it("普通设置保存失败时显示失败并恢复后端已确认值", async () => {
    api.updateGlobalSettings.mockRejectedValueOnce(new Error("settings_write_failed"));

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    const darkTheme = screen.getByRole("button", { name: "深色" });
    await waitFor(() => expect(darkTheme).toBeEnabled());
    await fireEvent.click(darkTheme);

    expect(await screen.findByText("保存失败")).toBeVisible();
    expect(document.documentElement).toHaveAttribute("data-theme", "system");
    expect(screen.getByRole("button", { name: "跟随系统" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("scheduler 与 sync 后端事件触发即时刷新", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    const handlers = new Map<string, () => void>();
    api.listenAppEvent.mockImplementation(async (name: string, handler: () => void) => {
      handlers.set(name, handler);
      return vi.fn();
    });
    render(App);
    await waitFor(() => expect(api.getSchedulerState).toHaveBeenCalledTimes(1));

    handlers.get("scheduler:changed")?.();
    await waitFor(() => expect(api.getSchedulerState).toHaveBeenCalledTimes(2));
    handlers.get("sync:changed")?.();
    await waitFor(() => expect(api.getSchedulerState).toHaveBeenCalledTimes(3));
    expect(api.listContent).toHaveBeenCalled();
  });

  it("切换喜欢来源后使用统一内容查询并默认隐藏远端移除记录", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.getSchedulerState.mockResolvedValue(
      schedulerState({
        capabilities: schedulerState().capabilities.map((capability) => ({
          ...capability,
          available: capability.source !== "remoteFolder",
          unavailableReason: capability.source === "remoteFolder" ? "not_applicable" : null,
        })),
      }),
    );

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "喜欢" }));

    await waitFor(() =>
      expect(api.listContent).toHaveBeenLastCalledWith({
        accountId: 11,
        source: "likes",
        query: "",
        includeRemoteRemoved: false,
        sort: "pageOrder",
        limit: 80,
        offset: 0,
      }),
    );
    expect(screen.queryByText("删除选中")).not.toBeInTheDocument();

    await fireEvent.click(screen.getByLabelText("包含已从远端移除"));
    await waitFor(() =>
      expect(api.listContent).toHaveBeenLastCalledWith(
        expect.objectContaining({ source: "likes", includeRemoteRemoved: true }),
      ),
    );
  });

  it("各来源保留独立搜索状态且账号切换时只重置页码", async () => {
    const first = account("active");
    const second = { ...first, id: 12, nickname: "第二账号", profileKey: "fedcba9876543210fedcba9876543210" };
    let selectedAccount = first;
    api.listAccounts.mockResolvedValue([first, second]);
    api.getActiveAccount.mockImplementation(async () => selectedAccount);
    api.setActiveAccount.mockImplementation(async () => {
      selectedAccount = second;
      return second;
    });
    api.getSchedulerState.mockResolvedValue(
      schedulerState({
        capabilities: schedulerState().capabilities.map((capability) => ({
          ...capability,
          available: capability.source !== "remoteFolder",
          unavailableReason: capability.source === "remoteFolder" ? "not_applicable" : null,
        })),
      }),
    );

    render(App);
    const search = await screen.findByPlaceholderText("搜索标题或作者");
    await fireEvent.input(search, { target: { value: "历史词" } });
    await fireEvent.click(screen.getByRole("button", { name: "喜欢" }));
    expect(search).toHaveValue("");
    await fireEvent.input(search, { target: { value: "喜欢词" } });
    await fireEvent.click(screen.getByRole("button", { name: "观看历史" }));
    expect(search).toHaveValue("历史词");

    await fireEvent.click(screen.getByRole("button", { name: /测试账号/ }));
    await fireEvent.click(screen.getByRole("button", { name: /第二账号/ }));
    await waitFor(() => expect(api.setActiveAccount).toHaveBeenCalledWith(12));
    expect(search).toHaveValue("历史词");
    await waitFor(() =>
      expect(api.listContent).toHaveBeenLastCalledWith(
        expect.objectContaining({ accountId: 12, source: "history", query: "历史词", offset: 0 }),
      ),
    );
  });

  it("列表标题与网格封面都通过系统浏览器命令打开原内容", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.listContent.mockResolvedValue({
      records: [{
        id: 1,
        source: "history",
        awemeId: "123",
        url: "https://www.douyin.com/video/123",
        contentType: "video",
        title: "示例视频",
        authorName: "作者",
        coverUrl: "https://example.com/cover.jpg",
        likeCount: 100,
        watchedDate: "2026-08-13",
      }],
      total: 1,
    });

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "示例视频" }));
    expect(api.openContentUrl).toHaveBeenCalledWith("https://www.douyin.com/video/123");

    await fireEvent.click(screen.getByRole("button", { name: "网格视图" }));
    await fireEvent.click(screen.getByTitle("打开示例视频"));
    expect(api.openContentUrl).toHaveBeenCalledTimes(2);
  });

  it("观看历史可按选中、当前筛选和全部历史创建下载预览", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.listContent.mockResolvedValue({
      records: [
        {
          id: 1,
          source: "history",
          awemeId: "123",
          url: "https://www.douyin.com/video/123",
          contentType: "video",
          title: "示例视频 A",
          authorName: "作者 A",
          watchedDate: "2026-08-13",
        },
        {
          id: 2,
          source: "history",
          awemeId: "456",
          url: "https://www.douyin.com/video/456",
          contentType: "video",
          title: "示例视频 B",
          authorName: "作者 B",
          watchedDate: "2026-08-12",
        },
      ],
      total: 2,
    });

    render(App);
    const recordChecks = await screen.findAllByLabelText("选择记录");
    await fireEvent.click(recordChecks[0]);
    await fireEvent.click(screen.getByRole("button", { name: "下载选中" }));
    await waitFor(() =>
      expect(downloadApi.previewDownloadSelection).toHaveBeenLastCalledWith(11, {
        kind: "checked",
        watch_record_ids: [1],
      }),
    );
    const downloadDialog = screen.getByRole("dialog", { name: "开始下载" });
    const downloadCancel = screen.getByRole("button", { name: "取消" });
    expect(downloadDialog).toBeVisible();
    await waitFor(() => expect(downloadCancel).toHaveFocus());
    expect(screen.getByTestId("workspace-background")).toHaveAttribute("inert");
    expect(screen.getByTestId("modal-host")).toContainElement(downloadDialog);
    await fireEvent.click(downloadCancel);
    expect(screen.getByTestId("workspace-background")).not.toHaveAttribute("inert");

    await fireEvent.input(screen.getByPlaceholderText("搜索标题或作者"), {
      target: { value: "作者 A" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "下载当前筛选" }));
    await waitFor(() =>
      expect(downloadApi.previewDownloadSelection).toHaveBeenLastCalledWith(11, {
        kind: "filtered",
        query: "作者 A",
      }),
    );
    await fireEvent.click(screen.getByRole("button", { name: "取消" }));

    await fireEvent.click(screen.getByRole("button", { name: "下载全部历史" }));
    await waitFor(() =>
      expect(downloadApi.previewDownloadSelection).toHaveBeenLastCalledWith(11, {
        kind: "all",
      }),
    );
  });

  it("管理导航打开跨账号下载任务中心", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.listTaskSummaries.mockResolvedValue({
      records: [{
        key: "download:9",
        kind: "downloadJob",
        id: 9,
        accountId: 11,
        source: null,
        status: "running",
        createdAt: "2026-08-15T00:00:00Z",
        finishedAt: null,
        itemCount: 2,
        completedCount: 1,
        failedCount: 0,
        errorCode: null,
        canCancel: true,
      }],
      total: 1,
    });

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "任务中心" }));

    expect(await screen.findByRole("region", { name: "任务中心" })).toBeVisible();
    expect(await screen.findByText(/下载任务 #9/)).toBeVisible();
    await waitFor(() => expect(api.listTaskSummaries).toHaveBeenCalled());
  });

  it("设置页保存下载目录、质量和每次询问选项", async () => {
    localStorage.setItem(
      "douyin-history-download-settings",
      JSON.stringify({
        defaultDirectory: "D:\\Douyin",
        defaultQuality: "balanced",
        askDirectoryEachJob: true,
        askQualityEachJob: false,
      }),
    );

    render(App);
    await fireEvent.click(await screen.findByTitle("设置"));
    await selectSettingsPanel("下载", "视频下载");

    const directory = await screen.findByLabelText("默认下载目录");
    const quality = screen.getByLabelText("默认下载质量");
    const askDirectory = screen.getByLabelText("每次下载前选择目录");
    const askQuality = screen.getByLabelText("每次下载前选择质量");
    expect(directory).toHaveValue("D:\\Douyin");
    expect(directory).toHaveAttribute("readonly");
    expect(quality).toHaveValue("balanced");
    expect(askDirectory).toBeChecked();
    expect(askQuality).not.toBeChecked();

    dialogApi.open.mockResolvedValueOnce("E:\\Archive");
    await fireEvent.click(screen.getByRole("button", { name: "选择默认下载目录" }));
    await waitFor(() => expect(directory).toHaveValue("E:\\Archive"));
    await fireEvent.change(quality, { target: { value: "highest" } });
    await fireEvent.click(askDirectory);
    await fireEvent.click(askQuality);

    await waitFor(() =>
      expect(JSON.parse(localStorage.getItem("douyin-history-download-settings") ?? "{}")).toEqual({
        defaultDirectory: "E:\\Archive",
        defaultQuality: "highest",
        askDirectoryEachJob: false,
        askQualityEachJob: true,
      }),
    );
  });

  it("从普通内容记录加入已有本地收藏夹", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.listContent.mockResolvedValue({
      records: [{
        id: 1,
        source: "history",
        awemeId: "123",
        url: "https://www.douyin.com/video/123",
        contentType: "video",
        title: "示例视频",
      }],
      total: 1,
    });
    api.listLocalFolders.mockResolvedValue([{
      id: 9,
      scope: "aggregate",
      ownerAccountId: null,
      name: "跨账号精选",
      itemCount: 3,
      createdAt: "2026-08-13T00:00:00Z",
      updatedAt: "2026-08-13T00:00:00Z",
    }]);

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "将示例视频加入本地收藏夹" }));
    expect(await screen.findByRole("dialog", { name: "加入本地收藏夹" })).toBeVisible();
    expect(api.listLocalFolders).toHaveBeenCalledWith(11);

    await fireEvent.click(screen.getByRole("button", { name: /跨账号精选/ }));
    await waitFor(() => expect(api.addLocalFolderItem).toHaveBeenCalledWith(11, 9, "123"));
    expect(await screen.findByText("已加入本地收藏夹“跨账号精选”")).toBeVisible();
  });

  it("可在收藏浮窗中新建账号专属收藏夹并立即加入", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.listContent.mockResolvedValue({
      records: [{
        id: 2,
        source: "likes",
        awemeId: "456",
        url: "https://www.douyin.com/note/456",
        contentType: "note",
        title: "图文内容",
      }],
      total: 1,
    });
    api.createLocalFolder.mockResolvedValue({
      id: 10,
      scope: "account",
      ownerAccountId: 11,
      name: "稍后阅读",
      itemCount: 0,
      createdAt: "2026-08-13T00:00:00Z",
      updatedAt: "2026-08-13T00:00:00Z",
    });

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "将图文内容加入本地收藏夹" }));
    await fireEvent.click(await screen.findByRole("button", { name: "新建收藏夹" }));
    await fireEvent.input(screen.getByLabelText("新收藏夹名称"), { target: { value: "稍后阅读" } });
    await fireEvent.click(screen.getByRole("button", { name: "新建并加入" }));

    await waitFor(() => expect(api.createLocalFolder).toHaveBeenCalledWith(11, "稍后阅读", "account"));
    expect(api.addLocalFolderItem).toHaveBeenCalledWith(11, 10, "456");
  });

  it("抖音收藏夹与本地收藏夹入口按当前账号读取且默认隐藏远端移除", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);

    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "抖音收藏夹" }));
    await waitFor(() => expect(api.listRemoteFolders).toHaveBeenCalledWith(11, false));

    await fireEvent.click(screen.getByRole("button", { name: "本地收藏夹" }));
    await waitFor(() => expect(api.listLocalFolders).toHaveBeenCalledWith(11));
  });

  it("从后端初始化设置并按全局与账号 scope 串行保存", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.getAppSettings.mockResolvedValue(
      appSettings({
        global: {
          likeCountFormat: "exact",
          detailFields: {
            author: false,
            watchDate: true,
            likeCount: true,
            watchProgress: false,
          },
        },
        account: {
          hideBrowser: true,
          scrollSpeed: 3_200,
          history: {
            range: { kind: "bottom" },
            schedule: { enabled: false, intervalMinutes: 1_440 },
          },
          likes: {
            range: { kind: "all" },
            schedule: { enabled: false, intervalMinutes: 1_440 },
          },
          favorites: {
            archive: {
              range: { kind: "all" },
              schedule: { enabled: false, intervalMinutes: 1_440 },
            },
            remoteFolderMode: "unified",
            remoteFolderDefaults: {
              range: { kind: "all" },
              schedule: { enabled: false, intervalMinutes: 1_440 },
            },
            remoteFolders: {},
          },
        },
      }),
    );
    render(App);
    await waitFor(() => expect(api.getAppSettings).toHaveBeenCalledWith(11));

    await fireEvent.click(screen.getByTitle("设置"));
    await selectSettingsPanel("账号与同步", "同步参数");
    expect(screen.getByLabelText("滚动速率")).toHaveValue(3_200);
    expect(screen.getByLabelText("同步时隐藏浏览器")).toBeChecked();
    await selectSettingsPanel("内容显示", "点赞量格式");
    expect(screen.getByRole("button", { name: "准确数字" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await fireEvent.click(screen.getByRole("button", { name: "K" }));
    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenCalledWith(
        expect.objectContaining({ likeCountFormat: "k" }),
      ),
    );
    await selectSettingsPanel("账号与同步", "同步参数");
    await fireEvent.change(screen.getByLabelText("滚动速率"), {
      target: { value: "4800" },
    });
    await waitFor(() =>
      expect(api.updateAccountSettings).toHaveBeenCalledWith(
        11,
        expect.objectContaining({
          scrollSpeed: 4_800,
          history: expect.objectContaining({ range: { kind: "bottom" } }),
        }),
      ),
    );
  });

  it("开启封面缓存时要求选择后续新增或补齐全部历史", async () => {
    render(App);
    await waitFor(() => expect(api.getAppSettings).toHaveBeenCalled());
    await fireEvent.click(screen.getByTitle("设置"));

    const cacheToggle = screen.getByLabelText("启用本地封面缓存");
    api.updateGlobalSettings.mockClear();
    await fireEvent.click(cacheToggle);
    expect(screen.getByRole("dialog", { name: "启用封面缓存" })).toBeVisible();
    expect(cacheToggle).not.toBeChecked();
    expect(api.updateGlobalSettings).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog", { name: "启用封面缓存" })).not.toBeInTheDocument();
    expect(cacheToggle).not.toBeChecked();
    expect(api.updateGlobalSettings).not.toHaveBeenCalled();

    await fireEvent.click(cacheToggle);
    await fireEvent.click(screen.getByRole("button", { name: "补齐全部历史" }));
    await waitFor(() =>
      expect(api.updateGlobalSettings).toHaveBeenCalledWith(
        expect.objectContaining({
          coverCache: expect.objectContaining({
            enabled: true,
            backfillMode: "backfillAll",
          }),
        }),
      ),
    );
    expect(cacheToggle).toBeChecked();
  });

  it("封面缓存清理浮窗展示预览并可搜索添加规则", async () => {
    api.inspectCoverCache.mockResolvedValue({
      healthyFiles: 3,
      missingFiles: 1,
      orphanFiles: 1,
      invalidIndexes: 1,
      indexedBytes: 3_145_728,
      orphanBytes: 512,
    });
    api.listCoverCacheRules.mockResolvedValue([
      {
        videoId: 7,
        awemeId: "777",
        title: "保留内容",
        authorName: "作者甲",
        coverUrl: "https://example.com/777.jpg",
        cached: true,
      },
    ]);
    api.searchCoverCacheCandidates.mockResolvedValue([
      {
        videoId: 8,
        awemeId: "888",
        title: "搜索结果",
        authorName: "作者乙",
        coverUrl: "https://example.com/888.jpg",
        cached: false,
      },
    ]);
    api.previewCoverCacheCleanup.mockResolvedValue({
      files: 2,
      videos: 2,
      estimatedBytes: 2_097_152,
      mode: "keepListed",
    });
    api.addCoverCacheRule.mockResolvedValue(undefined);

    render(App);
    await waitFor(() => expect(api.getAppSettings).toHaveBeenCalled());
    await fireEvent.click(screen.getByTitle("设置"));
    await selectSettingsPanel("存储与缓存", "封面缓存");
    await fireEvent.click(screen.getByRole("button", { name: "管理封面缓存" }));

    const cacheManager = await screen.findByRole("dialog", { name: "封面缓存清理" });
    expect(cacheManager).toBeVisible();
    expect(screen.getByText("封面保留列表 · 按名单手动清理")).toBeVisible();
    expect(await screen.findByText("预计释放 2 MB")).toBeVisible();
    expect(screen.getByText("保留内容")).toBeVisible();

    const clearRules = screen.getByRole("button", { name: "清空列表" });
    clearRules.focus();
    await fireEvent.click(clearRules);
    const clearConfirmation = screen.getByRole("dialog", { name: "清空封面保留列表" });
    expect(cacheManager.closest("[data-dialog-layer]")).toHaveAttribute("inert");
    await fireEvent.keyDown(clearConfirmation, { key: "Escape" });
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "清空封面保留列表" })).not.toBeInTheDocument(),
    );
    expect(clearRules).toHaveFocus();
    expect(cacheManager.closest("[data-dialog-layer]")).not.toHaveAttribute("inert");

    await fireEvent.input(screen.getByLabelText("搜索规则列表外内容"), {
      target: { value: "搜索" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "搜索内容" }));
    expect(await screen.findByText("搜索结果")).toBeVisible();
    await fireEvent.click(screen.getByRole("button", { name: "加入封面保留列表" }));
    await waitFor(() => expect(api.addCoverCacheRule).toHaveBeenCalledWith(8));
  });

  it("选择安装目录时展示 Data 调整并由用户明确迁移", async () => {
    api.chooseDataRoot.mockResolvedValue("C:\\Program Files\\Emily");
    api.validateDataRoot.mockResolvedValue({
      requestedPath: "C:\\Program Files\\Emily",
      normalizedPath: "C:\\Program Files\\Emily\\Data",
      adjustedToInstallData: true,
    });
    api.setDataRoot.mockResolvedValue({
      dataRoot: "C:\\Program Files\\Emily\\Data",
      retainedSource: {
        path: "C:\\Users\\Test\\AppData\\Emily\\Data",
        rootId: "0123456789abcdef0123456789abcdef",
      },
    });
    api.getAppSettings
      .mockResolvedValueOnce(appSettings())
      .mockResolvedValue(appSettings({ dataRoot: "C:\\Program Files\\Emily\\Data" }));
    render(App);
    await waitFor(() => expect(api.getAppSettings).toHaveBeenCalled());
    await fireEvent.click(screen.getByTitle("设置"));
    await selectSettingsPanel("存储与缓存", "本地数据位置");

    await fireEvent.click(screen.getByRole("button", { name: "选择数据目录" }));
    expect(await screen.findByLabelText("当前数据目录")).toHaveValue("C:\\Program Files\\Emily\\Data");
    expect(screen.queryByText("目标目录")).not.toBeInTheDocument();
    expect(screen.getByText("已自动使用安装目录下的 Data 文件夹")).toBeVisible();

    await fireEvent.click(screen.getByRole("button", { name: "迁移数据" }));
    await waitFor(() =>
      expect(api.setDataRoot).toHaveBeenCalledWith("C:\\Program Files\\Emily\\Data"),
    );
  });

  it("迁移期间锁定同步与设置控件", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.chooseDataRoot.mockResolvedValue("C:\\Archive");
    api.validateDataRoot.mockResolvedValue({
      requestedPath: "C:\\Archive",
      normalizedPath: "C:\\Archive",
      adjustedToInstallData: false,
    });
    let finishMigration!: (value: unknown) => void;
    api.setDataRoot.mockImplementation(
      () => new Promise((resolve) => {
        finishMigration = resolve;
      }),
    );
    render(App);
    await waitFor(() => expect(api.getAppSettings).toHaveBeenCalledWith(11));
    await fireEvent.click(screen.getByTitle("设置"));
    await selectSettingsPanel("存储与缓存", "本地数据位置");
    await fireEvent.click(screen.getByRole("button", { name: "选择数据目录" }));
    await fireEvent.click(await screen.findByRole("button", { name: "迁移数据" }));

    expect(screen.getByRole("button", { name: "同步当前账号" })).toBeDisabled();
    expect(screen.getByLabelText("滚动速率")).toBeDisabled();
    finishMigration({
      dataRoot: "C:\\Archive",
      retainedSource: {
        path: "C:\\Users\\Test\\AppData\\Emily\\Data",
        rootId: "0123456789abcdef0123456789abcdef",
      },
    });
  });

  it("迁移会等待此前的设置保存完成", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.chooseDataRoot.mockResolvedValue("C:\\Archive");
    api.validateDataRoot.mockResolvedValue({
      requestedPath: "C:\\Archive",
      normalizedPath: "C:\\Archive",
      adjustedToInstallData: false,
    });
    let finishSave!: (value: number) => void;
    api.updateGlobalSettings.mockImplementation(
      () => new Promise((resolve) => {
        finishSave = resolve;
      }),
    );
    api.setDataRoot.mockResolvedValue({
      dataRoot: "C:\\Archive",
      retainedSource: {
        path: "C:\\Users\\Test\\AppData\\Emily\\Data",
        rootId: "0123456789abcdef0123456789abcdef",
      },
    });
    render(App);
    await waitFor(() => expect(api.getAppSettings).toHaveBeenCalledWith(11));
    await fireEvent.click(screen.getByTitle("设置"));
    await selectSettingsPanel("内容显示", "点赞量格式");
    await fireEvent.click(screen.getByRole("button", { name: "K" }));
    await waitFor(() => expect(api.updateGlobalSettings).toHaveBeenCalled());
    await selectSettingsPanel("存储与缓存", "本地数据位置");
    await fireEvent.click(screen.getByRole("button", { name: "选择数据目录" }));
    await fireEvent.click(await screen.findByRole("button", { name: "迁移数据" }));

    expect(api.setDataRoot).not.toHaveBeenCalled();
    finishSave(1);
    await waitFor(() => expect(api.setDataRoot).toHaveBeenCalledWith("C:\\Archive"));
  });

  it("删除旧数据目录使用独立确认浮层", async () => {
    api.getAppSettings.mockResolvedValue(
      appSettings({
        retainedDataRoots: [
          {
            root: {
              path: "C:\\OldData",
              rootId: "fedcba9876543210fedcba9876543210",
            },
            layout: "standard",
            legacyEntries: [],
          },
        ],
      }),
    );
    render(App);
    await waitFor(() => expect(api.getAppSettings).toHaveBeenCalled());
    await fireEvent.click(screen.getByTitle("设置"));
    await fireEvent.click(screen.getByTitle("删除旧数据目录"));

    expect(screen.getByRole("dialog", { name: "删除旧数据目录" })).toBeVisible();
    await fireEvent.click(screen.getByRole("button", { name: "确认删除" }));
    await waitFor(() =>
      expect(api.deleteRetainedDataRoot).toHaveBeenCalledWith("C:\\OldData"),
    );
  });

  it("账号管理不会渲染 pending 登录", async () => {
    api.listAccounts.mockResolvedValue([
      {
        id: 99,
        profileKey: "0123456789abcdef0123456789abcdef",
        nickname: "PENDING-SHOULD-NOT-RENDER",
        status: "pending_login",
        createdAt: "2026-07-14T00:00:00Z",
        updatedAt: "2026-07-14T00:00:00Z",
      },
    ]);
    render(App);
    await waitFor(() => expect(api.listAccounts).toHaveBeenCalled());
    await fireEvent.click(screen.getByRole("button", { name: /未添加账号/ }));
    await fireEvent.click(screen.getByRole("button", { name: "管理账号" }));
    await waitFor(() => expect(api.listAccounts).toHaveBeenCalledWith(true));

    expect(screen.queryByText("PENDING-SHOULD-NOT-RENDER")).not.toBeInTheDocument();
  });

  it("浏览器关闭后显示稳定错误并恢复添加按钮", async () => {
    api.beginAccountLogin.mockRejectedValueOnce("account_login_browser_closed");
    render(App);
    await waitFor(() => expect(api.getActiveAccount).toHaveBeenCalled());
    const add = screen.getByRole("button", { name: "添加账号" });

    await fireEvent.click(add);

    expect(await screen.findByText("登录浏览器已关闭，未添加账号")).toBeVisible();
    expect(await screen.findByRole("button", { name: "添加账号" })).toBeEnabled();
    await fireEvent.click(screen.getByRole("button", { name: "关闭消息提示" }));
    expect(screen.queryByText("登录浏览器已关闭，未添加账号")).not.toBeInTheDocument();
  });

  it("身份冲突账号禁止同步但保留一键登录、归档和删除", async () => {
    const conflicted = account("identity_conflict");
    api.listAccounts.mockResolvedValue([conflicted]);
    api.getActiveAccount.mockResolvedValue(conflicted);
    render(App);

    expect(await screen.findByRole("button", { name: "一键登录" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "同步当前来源" })).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole("button", { name: /冲突账号/ }));
    await fireEvent.click(screen.getByRole("button", { name: "管理账号" }));

    expect(await screen.findByRole("region", { name: "账号工作台" })).toBeVisible();
    expect(screen.getAllByRole("button", { name: "一键登录" }).length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "归档" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "删除" })).toBeEnabled();
  });

  it.each([
    ["recovering_login", "正在恢复登录"],
    ["retrying_after_login", "登录已恢复，正在继续同步"],
  ])("%s 阶段不显示首次登录失效错误且仍可取消", async (phase, label) => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.enqueueSyncTasks.mockResolvedValue(
      enqueueJobResult({ status: "running", queuePosition: null }),
    );
    api.getSyncStatus.mockResolvedValue(
      syncStatus(phase, { errorMessage: "session_expired" }),
    );
    render(App);
    await waitFor(() => expect(api.getActiveAccount).toHaveBeenCalled());

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "同步当前来源" })).toBeEnabled(),
    );
    await fireEvent.click(screen.getByRole("button", { name: "同步当前来源" }));

    await waitFor(() => expect(api.enqueueSyncTasks).toHaveBeenCalled());
    await waitFor(() => expect(api.getSyncStatus).toHaveBeenCalledWith(11, 23));
    expect(await screen.findByText(label)).toBeVisible();
    expect(screen.getByRole("button", { name: "取消同步" })).toBeEnabled();
    expect(screen.queryByText("session_expired")).not.toBeInTheDocument();
  });

  it("排队中的同步保持可取消并显示队列位置", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.enqueueSyncTasks.mockResolvedValue(enqueueJobResult({ queuePosition: 2 }));
    api.getSyncStatus.mockResolvedValue(
      syncStatus("queued", {
        status: "queued",
        queuePosition: 2,
        canCancel: true,
      }),
    );
    render(App);
    await waitFor(() => expect(api.getActiveAccount).toHaveBeenCalled());

    const syncButton = await screen.findByRole("button", { name: "同步当前来源" });
    await waitFor(() => expect(syncButton).toBeEnabled());
    await fireEvent.click(syncButton);

    expect(await screen.findByText("等待同步（队列第 2 位）")).toBeVisible();
    expect(screen.getByRole("button", { name: "取消同步" })).toBeEnabled();
  });

  it("部分完成作为独立终态呈现", async () => {
    const active = account("active");
    api.listAccounts.mockResolvedValue([active]);
    api.getActiveAccount.mockResolvedValue(active);
    api.enqueueSyncTasks.mockResolvedValue(
      enqueueJobResult({ status: "running", queuePosition: null }),
    );
    api.getSyncStatus.mockResolvedValue(
      syncStatus("finalizing", {
        status: "partial",
        canCancel: false,
      }),
    );
    render(App);
    await waitFor(() => expect(api.getActiveAccount).toHaveBeenCalled());

    const syncButton = await screen.findByRole("button", { name: "同步当前来源" });
    await waitFor(() => expect(syncButton).toBeEnabled());
    await fireEvent.click(syncButton);

    expect(await screen.findByText("上次同步部分完成")).toBeVisible();
    expect(screen.getByRole("button", { name: "同步当前来源" })).toBeEnabled();
  });

  it.each([
    ["login_expired", "自动恢复登录失败，请重新登录。", "登录已失效"],
    ["identity_conflict", "自动恢复检测到账号身份冲突。", "账号身份冲突"],
  ] as const)("最终 %s 状态在事务完成后单独呈现", async (status, message, label) => {
    const active = account("active");
    const terminal = account(status);
    api.listAccounts.mockResolvedValueOnce([active]).mockResolvedValue([terminal]);
    api.getActiveAccount.mockResolvedValueOnce(active).mockResolvedValue(terminal);
    api.enqueueSyncTasks.mockResolvedValue(
      enqueueJobResult({ status: "running", queuePosition: null }),
    );
    api.getSyncStatus.mockResolvedValue(
      syncStatus("failed", {
        status: "failed",
        errorMessage: message,
        canCancel: false,
      }),
    );
    render(App);
    await waitFor(() => expect(api.getActiveAccount).toHaveBeenCalledTimes(1));

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "同步当前来源" })).toBeEnabled(),
    );
    await fireEvent.click(screen.getByRole("button", { name: "同步当前来源" }));

    await waitFor(() => expect(api.enqueueSyncTasks).toHaveBeenCalled());
    await waitFor(() => expect(api.getSyncStatus).toHaveBeenCalledWith(11, 23));
    expect(await screen.findByText(message)).toBeVisible();
    expect(await screen.findByText(label)).toBeVisible();
    expect(screen.getByRole("button", { name: "一键登录" })).toBeEnabled();
  });
});
