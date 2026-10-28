<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open as openDirectory } from "@tauri-apps/plugin-dialog";
  import appIconUrl from "./assets/app-icon.png";
  import {
    AlertCircle,
    Archive,
    CircleStop,
    ChevronLeft,
    ChevronRight,
    Clock,
    Database,
    Download,
    BookmarkPlus,
    FolderOpen,
    Grid3X3,
    List,
    LogIn,
    RefreshCw,
    Search,
    Trash2,
    Upload,
    UserPlus,
    Users,
    X,
  } from "lucide-svelte";
  import {
    archiveAccount,
    beginAccountLogin,
    cancelSync,
    cancelPendingAccount,
    chooseBackupDirectory,
    chooseBackupExportPath,
    chooseBackupImportPath,
    chooseDataRoot,
    deleteAccount,
    deleteHistoryByQuery,
    deleteHistoryRecords,
    deleteRetainedDataRoot,
    getActiveAccount,
    getAppSettings,
    getDesktopState,
    getSyncBatch,
    getSchedulerState,
    getStartupRecoveryStatus,
    getSyncStatus,
    getUpdateState,
    importLegacySettings,
    initializeOnboardingDataRoot,
    inspectBackupImport,
    checkForUpdates,
    downloadUpdate,
    installUpdate,
    addLocalFolderItem,
    createLocalFolder,
    listenAppEvent,
    listAccounts,
    listContent,
    listLocalFolders,
    listSchedulerInstances,
    listSyncBatchEntries,
    openContentUrl,
    prepareBackupMerge,
    restoreAccount,
    restoreBackupMerge,
    restoreBackupReplace,
    resolveStartupRecoveryFault,
    resumeAccountLogin,
    setActiveAccount,
    setDataRoot,
    commitOnboardingStep,
    enqueueSyncTasks,
    enqueueWorkspaceSync,
    exportBackup,
    setAutostart,
    setBackupDirectory,
    setBrowserPreference,
    setSchedulerPaused,
    takeDesktopIntents,
    updateAccountDefaults,
    updateAccountSettings,
    updateGlobalSettings,
    updateWorkspaceSettings,
    validateDataRoot,
    getCoverCacheTask,
    pauseCoverCacheTask,
    resumeCoverCacheTask,
    cancelCoverCacheTask,
    retryCoverCacheTask,
  } from "./app-api";
  import {
    accountStatusLabel,
    canReloginAccount,
    canSyncAccount,
    isAccountRequestCurrent,
    resetHistoryViewForAccountSwitch,
    sortAccounts,
  } from "./accounts";
  import {
    accountLoginButtonLabel,
    accountLoginErrorMessage,
    accountRecoveryButtonLabel,
  } from "./account-login-ui";
  import { formatCollectorPhase, formatLikeCount, formatWatchProgress } from "./formatters";
  import FolderPage from "./FolderPage.svelte";
  import CoverImage from "./CoverImage.svelte";
  import CoverCacheManager from "./CoverCacheManager.svelte";
  import OverviewPage from "./OverviewPage.svelte";
  import AnnualReviewPage from "./AnnualReviewPage.svelte";
  import TaskCenterPage from "./TaskCenterPage.svelte";
  import NotificationsPage from "./NotificationsPage.svelte";
  import AccountWorkspace from "./AccountWorkspace.svelte";
  import LibraryPage from "./LibraryPage.svelte";
  import SearchPage from "./SearchPage.svelte";
  import SyncRecordsPage from "./SyncRecordsPage.svelte";
  import AccountSwitcher from "./shell/AccountSwitcher.svelte";
  import AppShell from "./shell/AppShell.svelte";
  import PrimarySidebar from "./shell/PrimarySidebar.svelte";
  import TitleBar from "./shell/TitleBar.svelte";
  import CustomScrollbars from "./controls/CustomScrollbars.svelte";
  import ConfirmDialog from "./controls/ConfirmDialog.svelte";
  import SelectField from "./controls/SelectField.svelte";
  import OnboardingWorkspace from "./onboarding/OnboardingWorkspace.svelte";
  import SettingsWorkspace from "./settings-workspace/SettingsWorkspace.svelte";
  import { resolvePortableAccountScope } from "./account-scope";
  import type { SettingsSaveState } from "./settings-workspace/SettingsWorkspace.svelte";
  import type { AppPage } from "./shell/types";
  import DownloadStartDialog from "./download/DownloadStartDialog.svelte";
  import { previewDownloadSelection } from "./download/api";
  import { buildDownloadSelection } from "./download/selection";
  import {
    loadDownloadSettings,
    normalizeDownloadSettings,
    saveDownloadSettings,
  } from "./download/settings";
  import type {
    DownloadJobDetail,
    DownloadPreview,
    DownloadSelection,
    DownloadSettings,
  } from "./download/types";
  import {
    createContentViewStates,
    resetContentViewStatesForAccount,
    updateContentViewState,
    type ContentSource,
  } from "./content-view";
  import {
    applyOnboardingCommit,
    buildOnboardingRequest,
    createOnboardingDraft,
    stepBack,
    type OnboardingDraft,
  } from "./onboarding";
  import {
    SchedulerRefreshController,
    appendBatchEntries,
    appendSchedulerInstances,
    createBatchEntriesState,
    createSchedulerInstancesState,
    syncIssueMessage,
    type BatchEntriesState,
    type SchedulerInstancesState,
  } from "./scheduler";
  import {
    LIKE_COUNT_FORMAT_OPTIONS,
    SettingsRequestGeneration,
    SettingsSaveQueue,
    dataRootErrorMessage,
    loadSettingsWithLegacyImport,
    normalizeLikeCountFormat,
    normalizeScrollSpeed,
    normalizeSyncDays,
  } from "./settings";
  import type {
    AccountView,
    ArchiveListRange,
    AppSettingsView,
    BackupArchiveSummary,
    BackupSettings,
    DataRootValidationView,
    DetailField,
    GlobalSettings,
    ContentSort,
    ContentRecordView,
    CoverCacheBackfillMode,
    CoverCacheCleanupMode,
    CoverCacheSettings,
    CoverCacheTaskView,
    LikeCountFormat,
    LocalFolderMergeResolution,
    LocalFolderScope,
    LocalFolderView,
    MergePlan,
    OverviewRange,
    OverviewSourceFilter,
    NotificationSettings,
    PortableAccountScope,
    AccountSettings,
    BrowserPreference,
    DesktopStateView,
    DefaultSyncMode,
    EnqueueSyncResult,
    SchedulerState,
    SettingValueSource,
    SettingsMergeResolution,
    SchedulerSourceState,
    SourceCapability,
    SyncBatchView,
    SyncJobStatus,
    SyncJobView,
    SyncSource,
    SyncMode,
    SyncOptions,
    SyncStatus,
    ThemeMode,
    UpdateStateView,
    UpdateSettings,
    StartupRecoveryDisposition,
    StartupRecoveryStatusView,
    WorkspaceContentViewPreference,
    WorkspaceSettings,
  } from "./types";

  const CONTENT_PAGE_SIZE = 80;
  type MessageBanner = { kind: "error" | "notice"; text: string };
  const settingsSaveQueue = new SettingsSaveQueue();
  const settingsRequestGeneration = new SettingsRequestGeneration();

  let contentViewStates = createContentViewStates();
  let accounts: AccountView[] = [];
  let activeAccount: AccountView | null = null;
  let accountMenuOpen = false;
  let accountBusy = false;
  let accountLoginBusy = false;
  let currentSyncJobId = 0;
  let records: ContentRecordView[] = [];
  let total = 0;
  let contentRequestGeneration = 0;
  let syncMode: SyncMode = "days";
  let syncDays = 7;
  let scrollSpeed = 6_000;
  let hideBrowser = false;
  let accountSettings: AccountSettings | null = null;
  let closeBehavior: GlobalSettings["closeBehavior"] = "exit";
  let startupSyncDelayMinutes = 5;
  let themeMode: ThemeMode = "dark";
  let sidebarCollapsed = false;
  let likeCountFormat: LikeCountFormat = "wan";
  let coverCacheEnableOpen = false;
  let coverCacheManagerOpen = false;
  let coverCacheSettings: CoverCacheSettings = {
    enabled: false,
    backfillMode: "newRecordsOnly",
    capacityLimitMib: 2_048,
    cleanupByAgeEnabled: false,
    maxAgeDays: 90,
    cleanupByCapacityEnabled: false,
    cleanupListMode: "keepListed",
  };
  let downloadSettings: DownloadSettings = loadDownloadSettings();
  let downloadDialogAccountId: number | null = null;
  let downloadDialogSelection: DownloadSelection | null = null;
  let downloadDialogPreview: DownloadPreview | null = null;
  let downloadPreviewing = false;
  let backupSettings: BackupSettings = {
    enabled: false,
    intervalDays: 7,
    executionTime: "03:00",
    retentionCount: 5,
  };
  let notificationSettings: NotificationSettings = {
    loginExpired: true,
    automaticFailureStreak: true,
    backupFailure: true,
    syncCompleted: false,
    backupCompleted: false,
    automaticFailureThreshold: 3,
    messageDurationSeconds: 5,
  };
  let updateSettings: UpdateSettings = {
    automaticCheck: false,
    automaticDownload: false,
  };
  let updateState: UpdateStateView | null = null;
  let updateBusy = false;
  let updateInstallConfirmOpen = false;
  let backupDirectoryChoosing = false;
  let backupExportConfirmOpen = false;
  let backupExporting = false;
  let backupRestoreBusy = false;
  let backupRestoreCandidate: { path: string; summary: BackupArchiveSummary } | null = null;
  let backupMergePlan: MergePlan | null = null;
  let backupFolderResolutions: Record<string, LocalFolderMergeResolution> = {};
  let backupFolderRenameNames: Record<string, string> = {};
  let backupSettingsResolutionMode: SettingsMergeResolution["mode"] = "current";
  let backupSettingChoices: Record<string, SettingValueSource> = {};
  let coverCacheTask: CoverCacheTaskView | null = null;
  let coverCacheTaskBusy = false;
  let desktopState: DesktopStateView | null = null;
  let schedulerState: SchedulerState | null = null;
  let capabilityBySource: Partial<Record<SyncSource, SourceCapability>> = {};
  let schedulerSourceStateBySource: Partial<Record<SyncSource, SchedulerSourceState>> = {};
  let likesCapability: SourceCapability | null = null;
  let favoritesCapability: SourceCapability | null = null;
  let remoteFoldersSupported = false;
  let activeBatch: SyncBatchView | null = null;
  let activeBatchEntries: BatchEntriesState | null = null;
  let schedulerInstances: SchedulerInstancesState | null = null;
  let batchLoading = false;
  let schedulerInstancesLoading = false;
  let onboardingDraft: OnboardingDraft | null = null;
  let onboardingBusy = false;
  let onboardingBrowserPreference: BrowserPreference = "auto";
  let browserPreference: BrowserPreference = "auto";
  let browserReloginRequired = false;
  let currentSource: ContentSource = "history";
  let activePage: AppPage = "history";
  let overviewSource: OverviewSourceFilter = "all";
  let overviewRange: OverviewRange = "last30Days";
  let overviewIncludeRemoteRemoved = false;
  let annualReviewOpen = false;
  let desktopIntentRevision = 0;
  let windowVisible = typeof document === "undefined" || document.visibilityState !== "hidden";
  let refreshController: SchedulerRefreshController | null = null;
  let detailFields: Record<DetailField, boolean> = {
    author: true,
    watchDate: true,
    likeCount: true,
    watchProgress: true,
  };
  let loading = true;
  let settingsLoading = true;
  let settingsSaveState: SettingsSaveState = "idle";
  let settingsSaveGeneration = 0;
  let settingsSaveFeedbackTimer: number | null = null;
  let confirmedGlobalSettings: GlobalSettings | null = null;
  let confirmedAccountSettings: AccountSettings | null = null;
  let confirmedWorkspaceSettings: WorkspaceSettings | null = null;
  let syncing = false;
  let cancelRequested = false;
  let deleting = false;
  let errorMessage = "";
  let noticeMessage = "";
  let messageBanner: MessageBanner | null = null;
  let messageBannerKey = "";
  let dismissedMessageKey = "";
  let scheduledMessageKey = "";
  let scheduledMessageDurationSeconds = 0;
  let messageDismissTimer: number | null = null;
  let appSettings: AppSettingsView | null = null;
  let workspaceMainElement: HTMLElement | null = null;
  const routeScrollPositions = new Map<AppPage, number>();
  let dataRootValidation: DataRootValidationView | null = null;
  let dataRootChoosing = false;
  let dataRootMigrating = false;
  let retainedRootPendingDelete: string | null = null;
  let retainedRootDeleting = false;
  let startupRecoveryStatus: StartupRecoveryStatusView | null = null;
  let startupRecoveryBusy = false;
  let selectedIds = new Set<number>();
  let folderPickerRecord: ContentRecordView | null = null;
  let folderPickerFolders: LocalFolderView[] = [];
  let folderPickerLoading = false;
  let folderPickerBusy = false;
  let folderPickerCreateOpen = false;
  let folderPickerName = "";
  let folderPickerScope: LocalFolderScope = "account";
  let syncStatus: SyncStatus = {
    accountId: 0,
    jobId: 0,
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

  $: sortedAccounts = sortAccounts(accounts, activeAccount?.id);

  $: statusText =
    syncStatus.status === "queued"
      ? typeof syncStatus.queuePosition === "number"
        ? `等待同步（队列第 ${syncStatus.queuePosition} 位）`
        : "等待同步"
      : syncStatus.status === "running"
        ? "同步中"
      : syncStatus.status === "success"
        ? "上次同步成功"
        : syncStatus.status === "partial"
          ? "上次同步部分完成"
          : syncStatus.status === "failed"
            ? "上次同步失败"
            : syncStatus.status === "cancelled"
              ? "上次同步已取消"
              : "尚未同步";

  $: phaseText = formatCollectorPhase(syncStatus.phase);
  $: terminalSyncError =
    syncStatus.status === "failed" ? (syncStatus.errorMessage ?? "") : "";
  $: messageBanner = errorMessage
    ? { kind: "error", text: errorMessage }
    : terminalSyncError
      ? { kind: "error", text: terminalSyncError }
      : noticeMessage
        ? { kind: "notice", text: noticeMessage }
        : null;
  $: messageBannerKey = messageBanner ? `${messageBanner.kind}:${messageBanner.text}` : "";
  $: scheduleMessageDismissal(messageBannerKey, notificationSettings.messageDurationSeconds);

  $: selectedCount = selectedIds.size;
  $: allVisibleSelected =
    records.length > 0 && records.every((record) => selectedIds.has(record.id));
  $: currentContentView = contentViewStates[currentSource];
  $: contentPageActive = ["history", "likes", "favorites"].includes(activePage);
  $: pageTitle =
    activePage === "overview"
      ? "概览"
      : activePage === "library"
        ? "资料库"
        : activePage === "search"
          ? "搜索"
          : activePage === "accounts"
            ? "账号管理"
      : activePage === "history"
        ? "观看历史"
        : activePage === "likes"
          ? "喜欢"
          : activePage === "favorites"
            ? "收藏"
            : activePage === "remoteFolders"
              ? "抖音收藏夹"
              : activePage === "localFolders"
                ? "本地收藏夹"
                : activePage === "syncRecords"
                  ? "任务中心"
                : activePage === "downloads"
                  ? "任务中心"
                  : activePage === "notifications"
                    ? "通知中心"
                  : "设置";
  $: pageDescription =
    activePage === "overview"
      ? "汇总当前账号的本地归档与同步状态"
      : activePage === "library"
        ? "跨账号浏览观看历史、喜欢和收藏"
        : activePage === "search"
          ? "在本地归档与收藏夹中组合检索"
          : activePage === "accounts"
            ? "管理账号、登录状态与同步策略"
      : contentPageActive
        ? `浏览和管理当前账号的${pageTitle}`
        : activePage === "syncRecords" || activePage === "downloads"
          ? "统一查看同步批次、下载任务与实时状态"
        : activePage === "notifications"
          ? "查看登录、同步和备份问题的持久记录"
        : activePage === "settings"
          ? "调整全局、账号、缓存和数据目录设置"
          : `管理当前账号的${pageTitle}`;
  $: query = currentContentView.query;
  $: currentPage = currentContentView.page;
  $: totalPages = Math.max(1, Math.ceil(total / CONTENT_PAGE_SIZE));
  $: pageStart = total === 0 ? 0 : (currentPage - 1) * CONTENT_PAGE_SIZE + 1;
  $: pageEnd = Math.min(total, currentPage * CONTENT_PAGE_SIZE);
  $: accountSettingsDisabled = syncing || dataRootMigrating || settingsLoading || backupRestoreBusy;
  $: globalSettingsDisabled = syncing || dataRootMigrating || settingsLoading || backupRestoreBusy;
  $: updateOperationBusy = updateBusy || ["checking", "downloading", "installing"].includes(updateState?.phase ?? "");
  $: backupMergeReady = Boolean(
    backupMergePlan
      && backupMergePlan.localFolderConflicts.every((conflict) => {
        const resolution = backupFolderResolutions[conflict.id];
        return resolution?.action === "merge"
          || resolution?.action === "skip"
          || (resolution?.action === "rename" && resolution.name.trim().length > 0);
      })
      && (backupSettingsResolutionMode !== "perField"
        || backupMergePlan.settingsConflicts.every((conflict) => Boolean(backupSettingChoices[conflict.id]))),
  );
  $: currentBackupDirectory =
    desktopState?.device.backupDirectory ??
    (appSettings?.dataRoot ? `${appSettings.dataRoot.replace(/[\\/]+$/, "")}\\backups` : "");
  $: hasActiveRuntimeWork =
    syncing ||
    (schedulerState?.accounts.some((account) =>
      account.sourceStates.some((source) => source.activeJobId !== null),
    ) ?? false);
  $: currentSourceCapability =
    capabilityBySource[currentSource] ?? null;
  $: currentSourceAvailable = currentSourceCapability?.available ?? false;
  $: activeSchedulerAccount =
    schedulerState?.accounts.find((account) => account.accountId === activeAccount?.id) ?? null;
  $: capabilityBySource = Object.fromEntries(
    (schedulerState?.capabilities ?? []).map((capability) => [capability.source, capability]),
  ) as Partial<Record<SyncSource, SourceCapability>>;
  $: availableSyncSources = (schedulerState?.capabilities ?? [])
    .filter((capability) => capability.available)
    .map((capability) => capability.source);
  $: configuredSyncScopeInvalid = appSettings?.workspace.sync.accountScope.kind === "selected"
    && appSettings.workspace.sync.accountScope.accountKeys.some(
      (key) => !accounts.some((account) => account.status === "active" && account.accountKey === key),
    );
  $: schedulerSourceStateBySource = Object.fromEntries(
    (activeSchedulerAccount?.sourceStates ?? []).map((state) => [state.source, state]),
  ) as Partial<Record<SyncSource, SchedulerSourceState>>;
  $: likesCapability = capabilityBySource.likes ?? null;
  $: favoritesCapability = capabilityBySource.favorites ?? null;
  $: remoteFoldersSupported = favoritesCapability?.supportsRemoteFolders ?? false;
  $: refreshController?.update(windowVisible, hasActiveRuntimeWork);
  $: applyThemeMode(themeMode);

  async function refreshContent(
    page = contentViewStates[currentSource].page,
    source: ContentSource = currentSource,
  ): Promise<void> {
    if (!activeAccount) {
      records = [];
      total = 0;
      loading = false;
      return;
    }
    loading = true;
    errorMessage = "";
    const requestGeneration = ++contentRequestGeneration;
    try {
      const accountId = activeAccount.id;
      const targetPage = normalizePage(page);
      const view = contentViewStates[source];
      const contentResult = await listContent({
        accountId,
        source,
        query: view.query,
        includeRemoteRemoved: source !== "history" && view.includeRemoteRemoved,
        sort: view.sort,
        limit: CONTENT_PAGE_SIZE,
        offset: (targetPage - 1) * CONTENT_PAGE_SIZE,
      });
      if (
        requestGeneration !== contentRequestGeneration ||
        source !== currentSource ||
        !isAccountRequestCurrent(accountId, 0, activeAccount?.id, 0)
      ) return;
      const maxPage = Math.max(1, Math.ceil(contentResult.total / CONTENT_PAGE_SIZE));
      if (targetPage > maxPage) {
        const correctedResult = await listContent({
          accountId,
          source,
          query: view.query,
          includeRemoteRemoved: source !== "history" && view.includeRemoteRemoved,
          sort: view.sort,
          limit: CONTENT_PAGE_SIZE,
          offset: (maxPage - 1) * CONTENT_PAGE_SIZE,
        });
        if (
          requestGeneration !== contentRequestGeneration ||
          source !== currentSource ||
          !isAccountRequestCurrent(accountId, 0, activeAccount?.id, 0)
        ) return;
        contentViewStates = updateContentViewState(contentViewStates, source, { page: maxPage });
        records = correctedResult.records;
        total = correctedResult.total;
      } else {
        contentViewStates = updateContentViewState(contentViewStates, source, { page: targetPage });
        records = contentResult.records;
        total = contentResult.total;
      }
    } catch (error) {
      if (requestGeneration === contentRequestGeneration) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (requestGeneration === contentRequestGeneration) loading = false;
    }
  }

  async function applyEnqueueResult(result: EnqueueSyncResult): Promise<void> {
    activeBatchEntries = createBatchEntriesState(result);
    activeBatch = result.batchId === null ? null : await getSyncBatch(result.batchId);
    const job = result.entries.find(
      (entry): entry is { kind: "job"; data: SyncJobView } => entry.kind === "job",
    )?.data;
    if (job) {
      currentSyncJobId = job.id;
      syncStatus = syncStatusFromJob(job);
      syncing = isSyncInProgress(job.status);
    }
  }

  async function enqueueManualSync(scope: "one" | "currentAccountAvailableSources"): Promise<void> {
    if (!activeAccount) return;
    const source = scope === "one" ? currentSource : null;
    if (source !== null && !capabilityBySource[source]?.available) return;
    cancelRequested = false;
    errorMessage = "";
    noticeMessage = "";
    try {
      const result = await enqueueSyncTasks({
        scope,
        accountId: activeAccount.id,
        source,
        sourceInstanceKey: source === null ? null : "",
        payload: null,
        fullReconcileConfirmationSha256: null,
      });
      await applyEnqueueResult(result);
      noticeMessage =
        scope === "one" ? "当前来源已加入同步队列" : "当前账号可用来源已加入同步队列";
      await refreshSchedulerState();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  async function enqueueConfiguredWorkspaceSync(): Promise<void> {
    if (!appSettings) return;
    if (appSettings.workspace.sync.mode === "singleAccount") {
      await enqueueManualSync("currentAccountAvailableSources");
      return;
    }
    cancelRequested = false;
    errorMessage = "";
    noticeMessage = "";
    try {
      const accountScope = resolvePortableAccountScope(
        appSettings.workspace.sync.accountScope,
        accounts.filter((account) => account.status === "active"),
      );
      const result = await enqueueWorkspaceSync(accountScope);
      await applyEnqueueResult(result);
      noticeMessage = `${result.counts.jobs} 个同步任务已加入串行队列`;
      await refreshSchedulerState();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  function syncStatusFromJob(job: SyncJobView): SyncStatus {
    return {
      accountId: job.accountId,
      jobId: job.id,
      source: job.source,
      sourceInstanceKey: job.sourceInstanceKey,
      trigger: job.trigger,
      status: job.status,
      queuePosition: job.queuePosition,
      insertedVideos: 0,
      insertedRecords: 0,
      insertedWatchRecords: 0,
      startedAt: job.startedAt,
      finishedAt: job.finishedAt,
      recordCount: 0,
      elapsedMs: 0,
      retryCount: 0,
      canCancel: job.canCancel,
      issues: job.issues,
    };
  }

  async function refreshActiveBatch(loadMore = false): Promise<void> {
    if (!activeBatchEntries || batchLoading) return;
    batchLoading = true;
    try {
      const batch = await getSyncBatch(activeBatchEntries.batchId);
      activeBatch = batch;
      const cursor = loadMore || batch.status === "queued" || batch.status === "running"
        ? activeBatchEntries.cursor
        : null;
      if (cursor === null && !loadMore) return;
      const page = await listSyncBatchEntries(activeBatchEntries.batchId, cursor, 200);
      activeBatchEntries = appendBatchEntries(activeBatchEntries, page);
    } catch (error) {
      if (errorCode(error) === "invalid_batch_cursor" && activeBatchEntries) {
        const page = await listSyncBatchEntries(activeBatchEntries.batchId, null, 200);
        activeBatchEntries = {
          batchId: page.batchId,
          entries: [...page.entries],
          cursor: page.nextCursor,
          hasMore: page.hasMore,
        };
      } else {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      batchLoading = false;
    }
  }

  async function loadSchedulerInstances(reset = false): Promise<void> {
    if (!activeAccount || schedulerInstancesLoading) return;
    schedulerInstancesLoading = true;
    try {
      const cursor = reset ? null : schedulerInstances?.cursor ?? null;
      const page = await listSchedulerInstances(activeAccount.id, "remoteFolder", cursor, 200);
      schedulerInstances = reset || !schedulerInstances
        ? createSchedulerInstancesState(page)
        : appendSchedulerInstances(schedulerInstances, page);
    } catch (error) {
      if (errorCode(error) === "scheduler_instances_changed") {
        const page = await listSchedulerInstances(activeAccount.id, "remoteFolder", null, 200);
        schedulerInstances = createSchedulerInstancesState(page);
      } else {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      schedulerInstancesLoading = false;
    }
  }

  async function refreshSyncStatus(): Promise<void> {
    if (!activeAccount || currentSyncJobId <= 0) return;
    try {
      const accountId = activeAccount.id;
      const jobId = currentSyncJobId;
      const wasSyncing = syncing;
      const nextStatus = await getSyncStatus(accountId, jobId);
      if (!isAccountRequestCurrent(accountId, jobId, activeAccount?.id, currentSyncJobId)) return;
      syncStatus = nextStatus;
      syncing = isSyncInProgress(syncStatus.status);
      if (wasSyncing && !syncing) {
        await refreshAccounts(activePage === "accounts");
        await refreshContent();
      }
    } catch {
      // 主同步调用会展示最终错误，短暂的轮询失败不覆盖它。
    }
  }

  async function cancelRunningSync(): Promise<void> {
    if (!activeAccount || currentSyncJobId <= 0) return;
    cancelRequested = true;
    errorMessage = "";
    try {
      await cancelSync(activeAccount.id, currentSyncJobId);
      noticeMessage = "正在取消同步...";
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
      cancelRequested = false;
    }
  }

  function buildSyncOptions(): SyncOptions {
    if (syncMode === "bottom") {
      return {
        mode: "bottom",
        scrollSpeed: normalizeScrollSpeed(scrollSpeed),
        hideBrowser,
      };
    }

    return {
      mode: "days",
      days: normalizeSyncDays(syncDays),
      scrollSpeed: normalizeScrollSpeed(scrollSpeed),
      hideBrowser,
    };
  }

  function updateSyncDays(value: string): void {
    syncDays = normalizeSyncDays(Number(value));
    saveAccountSettings();
  }

  function updateScrollSpeed(value: string): void {
    scrollSpeed = normalizeScrollSpeed(Number(value));
    saveAccountSettings();
  }

  function setSyncMode(mode: SyncMode): void {
    syncMode = mode;
    saveAccountSettings();
  }

  function toggleDetailField(field: DetailField, checked: boolean): void {
    detailFields = { ...detailFields, [field]: checked };
    saveGlobalSettings();
  }

  function setHideBrowser(checked: boolean): void {
    hideBrowser = checked;
    saveAccountSettings();
  }

  function setLikeCountFormat(value: LikeCountFormat): void {
    likeCountFormat = value;
    saveGlobalSettings();
  }

  function updateStartupSyncDelay(value: string): void {
    startupSyncDelayMinutes = clampInteger(Number(value), 0, 120, 5);
    saveGlobalSettings();
  }

  function updateCloseBehavior(value: GlobalSettings["closeBehavior"]): void {
    if (value === "tray" && desktopState && !desktopState.trayAvailable) return;
    closeBehavior = value;
    saveGlobalSettings();
  }

  function updateSourceSchedule(
    source: "history" | "likes" | "favorites",
    enabled?: boolean,
    intervalMinutes?: number,
  ): void {
    if (!accountSettings || !capabilityBySource[source]?.available) return;
    const previous = structuredClone(accountSettings);
    const next = structuredClone(accountSettings);
    const schedule =
      source === "history"
        ? next.history.schedule
        : source === "likes"
          ? next.likes.schedule
          : next.favorites.archive.schedule;
    if (typeof enabled === "boolean") schedule.enabled = enabled;
    if (typeof intervalMinutes === "number") {
      schedule.intervalMinutes = clampInteger(intervalMinutes, 15, 43_200, 1_440);
    }
    accountSettings = next;
    void persistAccountSettings(next, previous);
  }

  async function persistAccountSettings(
    settings: AccountSettings,
    rollback: AccountSettings,
  ): Promise<void> {
    const accountId = activeAccount?.id ?? null;
    const generation = beginSettingsSave();
    await settingsSaveQueue.enqueue(async () => {
      try {
        const revision = accountId === null
          ? await updateAccountDefaults(settings)
          : await updateAccountSettings(accountId, settings);
        confirmAccountSettings(settings, revision, accountId);
        await refreshSchedulerState();
        completeSettingsSave(generation);
      } catch (error) {
        if ((activeAccount?.id ?? null) === accountId && generation === settingsSaveGeneration) {
          applyAccountSettingsValues(confirmedAccountSettings ?? rollback);
        }
        failSettingsSave(generation);
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    });
  }

  function archiveSettingsFor(
    settings: AccountSettings,
    source: "likes" | "favorites",
  ) {
    return source === "likes" ? settings.likes : settings.favorites.archive;
  }

  function archiveRangeForKind(
    current: ArchiveListRange,
    kind: ArchiveListRange["kind"],
  ): ArchiveListRange {
    switch (kind) {
      case "all":
        return { kind: "all" };
      case "recentItems":
        return { kind, count: current.kind === kind ? current.count : 500 };
      case "recordLimit":
        return { kind, limit: current.kind === kind ? current.limit : 5_000 };
      case "recentDays":
        return { kind, days: current.kind === kind ? current.days : 7 };
    }
  }

  function updateArchiveRange(
    source: "likes" | "favorites",
    kind: ArchiveListRange["kind"],
  ): void {
    if (!accountSettings || !capabilityBySource[source]?.available) return;
    const previous = structuredClone(accountSettings);
    const next = structuredClone(accountSettings);
    const archive = archiveSettingsFor(next, source);
    archive.range = archiveRangeForKind(archive.range, kind);
    accountSettings = next;
    void persistAccountSettings(next, previous);
  }

  function updateArchiveRangeValue(
    source: "likes" | "favorites",
    value: string,
  ): void {
    if (!accountSettings || !capabilityBySource[source]?.available) return;
    const previous = structuredClone(accountSettings);
    const next = structuredClone(accountSettings);
    const archive = archiveSettingsFor(next, source);
    switch (archive.range.kind) {
      case "recentItems":
        archive.range.count = clampInteger(Number(value), 1, 5_000, 500);
        break;
      case "recordLimit":
        archive.range.limit = clampInteger(Number(value), 1, 50_000, 5_000);
        break;
      case "recentDays":
        archive.range.days = clampInteger(Number(value), 1, 365, 7);
        break;
      case "all":
        return;
    }
    accountSettings = next;
    void persistAccountSettings(next, previous);
  }

  async function toggleSchedulerPaused(): Promise<void> {
    const paused = !(schedulerState?.paused ?? desktopState?.device.schedulerPaused ?? false);
    const generation = beginSettingsSave();
    try {
      await setSchedulerPaused(paused);
      if (desktopState) {
        desktopState = {
          ...desktopState,
          device: { ...desktopState.device, schedulerPaused: paused },
        };
      }
      await refreshSchedulerState();
      completeSettingsSave(generation);
    } catch (error) {
      failSettingsSave(generation);
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  async function updateAutostart(enabled: boolean, silent = desktopState?.device.silentAutostart ?? false): Promise<void> {
    if (!desktopState) return;
    const generation = beginSettingsSave();
    const previous = desktopState;
    desktopState = {
      ...desktopState,
      device: {
        ...desktopState.device,
        launchAtStartup: enabled,
        silentAutostart: enabled && silent,
      },
    };
    try {
      const device = await setAutostart({
        expectedDeviceRevision: previous.device.revision,
        enabled,
        silent: enabled && silent,
      });
      desktopState = { ...desktopState, device };
      completeSettingsSave(generation);
    } catch (error) {
      desktopState = {
        ...previous,
        device: {
          ...previous.device,
          accounts: { ...previous.device.accounts },
        },
      };
      failSettingsSave(generation);
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  async function updateBrowserPreference(value: BrowserPreference): Promise<void> {
    if (!activeAccount || value === browserPreference) return;
    if (!window.confirm("切换浏览器后需要重新登录抖音，是否继续？")) return;
    const generation = beginSettingsSave();
    try {
      const device = await setBrowserPreference({
        accountId: activeAccount.id,
        browserPreference: value,
        confirmRelogin: true,
      });
      if (desktopState) desktopState = { ...desktopState, device };
      browserPreference = value;
      browserReloginRequired = true;
      completeSettingsSave(generation);
    } catch (error) {
      failSettingsSave(generation);
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  function currentGlobalSettings(): GlobalSettings {
    return {
      theme: themeMode,
      likeCountFormat,
      detailFields: { ...detailFields },
      closeBehavior,
      startupSyncDelayMinutes,
      coverCache: { ...coverCacheSettings },
      backup: { ...backupSettings },
      notifications: { ...notificationSettings },
      updates: { ...updateSettings },
    };
  }

  function currentAccountSettings(): AccountSettings {
    const settings = structuredClone(accountSettings ?? appSettings?.account);
    if (!settings) throw new Error("account_settings_unavailable");
    settings.hideBrowser = hideBrowser;
    settings.scrollSpeed = normalizeScrollSpeed(scrollSpeed);
    settings.history.range =
      syncMode === "bottom"
        ? { kind: "bottom" }
        : { kind: "days", days: normalizeSyncDays(syncDays) };
    return settings;
  }

  function saveGlobalSettings(): Promise<void> {
    const settings = currentGlobalSettings();
    if (confirmedGlobalSettings && settingsEqual(settings, confirmedGlobalSettings)) {
      return Promise.resolve();
    }
    const generation = beginSettingsSave();
    return settingsSaveQueue.enqueue(async () => {
      try {
        const revision = await updateGlobalSettings(settings);
        confirmedGlobalSettings = structuredClone(settings);
        if (appSettings) appSettings = { ...appSettings, revision, global: structuredClone(settings) };
        completeSettingsSave(generation);
      } catch (error) {
        if (generation === settingsSaveGeneration && confirmedGlobalSettings) {
          applyGlobalSettingsValues(confirmedGlobalSettings);
        }
        failSettingsSave(generation);
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    });
  }

  function saveWorkspaceSettings(settings: WorkspaceSettings): Promise<void> {
    if (confirmedWorkspaceSettings && settingsEqual(settings, confirmedWorkspaceSettings)) {
      return Promise.resolve();
    }
    const generation = beginSettingsSave();
    return settingsSaveQueue.enqueue(async () => {
      try {
        if (!appSettings) throw new Error("app_settings_unavailable");
        const revision = await updateWorkspaceSettings(appSettings.revision, settings);
        confirmedWorkspaceSettings = structuredClone(settings);
        appSettings = {
          ...appSettings,
          revision,
          workspace:
            generation === settingsSaveGeneration
              ? structuredClone(settings)
              : appSettings.workspace,
        };
        completeSettingsSave(generation);
      } catch (error) {
        if (generation === settingsSaveGeneration && appSettings && confirmedWorkspaceSettings) {
          appSettings = {
            ...appSettings,
            workspace: structuredClone(confirmedWorkspaceSettings),
          };
        }
        failSettingsSave(generation);
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    });
  }

  function updateLibraryPreference(
    source: "all" | "history" | "likes" | "favorites",
    preference: WorkspaceContentViewPreference,
  ): void {
    if (!appSettings) return;
    const workspace = structuredClone(appSettings.workspace);
    workspace.contentViews[source] = structuredClone(preference);
    appSettings = { ...appSettings, workspace };
    void saveWorkspaceSettings(workspace);
  }

  function updateWorkspaceSyncPreference(
    mode: DefaultSyncMode,
    accountScope: PortableAccountScope,
  ): void {
    if (!appSettings) return;
    const workspace = structuredClone(appSettings.workspace);
    workspace.sync = { mode, accountScope: structuredClone(accountScope) };
    appSettings = { ...appSettings, workspace };
    void saveWorkspaceSettings(workspace);
  }

  function setWorkspaceSyncScopeKind(kind: "all" | "selected"): void {
    if (!appSettings) return;
    if (kind === "all") {
      updateWorkspaceSyncPreference(appSettings.workspace.sync.mode, { kind: "all" });
      return;
    }
    const accountKey = activeAccount?.accountKey ?? accounts.find((account) => account.status === "active")?.accountKey;
    if (!accountKey) return;
    updateWorkspaceSyncPreference(appSettings.workspace.sync.mode, {
      kind: "selected",
      accountKeys: [accountKey],
    });
  }

  function toggleWorkspaceSyncAccount(account: AccountView, checked: boolean): void {
    if (!appSettings || !account.accountKey) return;
    const current = appSettings.workspace.sync.accountScope;
    const keys = new Set(current.kind === "selected" ? current.accountKeys : []);
    if (checked) keys.add(account.accountKey);
    else keys.delete(account.accountKey);
    if (keys.size === 0) return;
    updateWorkspaceSyncPreference(appSettings.workspace.sync.mode, {
      kind: "selected",
      accountKeys: [...keys].sort((left, right) => left.localeCompare(right)),
    });
  }

  function updateBackupSettings(patch: Partial<BackupSettings>): void {
    backupSettings = { ...backupSettings, ...patch };
    void saveGlobalSettings();
  }

  function updateDownloadSettings(patch: Partial<DownloadSettings>): void {
    try {
      downloadSettings = normalizeDownloadSettings({ ...downloadSettings, ...patch });
      saveDownloadSettings(downloadSettings);
      errorMessage = "";
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  async function chooseDefaultDownloadDirectory(): Promise<void> {
    try {
      const selected = await openDirectory({
        directory: true,
        multiple: false,
        defaultPath: downloadSettings.defaultDirectory || undefined,
      });
      if (typeof selected === "string") {
        updateDownloadSettings({ defaultDirectory: selected });
      }
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  function updateNotificationSettings(patch: Partial<NotificationSettings>): void {
    notificationSettings = { ...notificationSettings, ...patch };
    void saveGlobalSettings();
  }

  function clearMessageDismissTimer(): void {
    if (messageDismissTimer === null || typeof window === "undefined") return;
    window.clearTimeout(messageDismissTimer);
    messageDismissTimer = null;
  }

  function scheduleMessageDismissal(key: string, durationSeconds: number): void {
    if (!key) {
      clearMessageDismissTimer();
      scheduledMessageKey = "";
      scheduledMessageDurationSeconds = 0;
      dismissedMessageKey = "";
      return;
    }
    const duration = clampInteger(durationSeconds, 1, 300, 5);
    const newMessage = key !== scheduledMessageKey;
    if (!newMessage && duration === scheduledMessageDurationSeconds) return;
    clearMessageDismissTimer();
    scheduledMessageKey = key;
    scheduledMessageDurationSeconds = duration;
    if (newMessage) dismissedMessageKey = "";
    if (dismissedMessageKey === key || typeof window === "undefined") return;
    messageDismissTimer = window.setTimeout(() => {
      if (messageBannerKey === key) dismissedMessageKey = key;
      messageDismissTimer = null;
    }, duration * 1_000);
  }

  function dismissMessageBanner(): void {
    if (!messageBannerKey) return;
    dismissedMessageKey = messageBannerKey;
    clearMessageDismissTimer();
  }

  function updateUpdateSettings(patch: Partial<UpdateSettings>): void {
    const next = { ...updateSettings, ...patch };
    if (!next.automaticCheck) next.automaticDownload = false;
    updateSettings = next;
    void saveGlobalSettings();
  }

  function updateErrorMessage(code: string | null): string {
    switch (code) {
      case "update_not_configured":
        return "当前安装包未配置应用内更新";
      case "invalid_update_endpoint":
      case "invalid_update_configuration":
        return "应用内更新配置无效";
      case "update_check_failed":
        return "检查更新失败，请稍后重试";
      case "update_download_failed":
        return "下载更新失败，请稍后重试";
      case "update_install_failed":
        return "启动更新安装失败";
      case "update_backup_failed":
        return "安装前安全备份失败";
      case "update_shutdown_timeout":
      case "update_shutdown_failed":
        return "后台任务未能安全停止，更新未安装";
      case "update_operation_in_progress":
        return "已有更新操作正在进行";
      case null:
        return "";
      default:
        return code;
    }
  }

  function formatUpdateBytes(bytes: number): string {
    if (bytes < 1_024) return `${bytes.toLocaleString("zh-CN")} B`;
    if (bytes < 1_048_576) return `${(bytes / 1_024).toFixed(1)} KiB`;
    return `${(bytes / 1_048_576).toFixed(1)} MiB`;
  }

  async function refreshUpdateState(): Promise<void> {
    try {
      updateState = await getUpdateState();
    } catch (error) {
      updateState = {
        configured: false,
        phase: "failed",
        currentVersion: "-",
        availableVersion: null,
        notes: null,
        publishedAt: null,
        downloadedBytes: 0,
        totalBytes: null,
        lastError: error instanceof Error ? error.message : String(error),
        canCheck: false,
        canDownload: false,
        canInstall: false,
      };
    }
  }

  async function runUpdateAction(action: "check" | "download" | "install"): Promise<void> {
    if (updateOperationBusy) return;
    updateBusy = true;
    errorMessage = "";
    noticeMessage = "";
    if (action === "install") updateInstallConfirmOpen = false;
    try {
      updateState = action === "check"
        ? await checkForUpdates()
        : action === "download"
          ? await downloadUpdate()
          : await installUpdate();
    } catch (error) {
      const code = error instanceof Error ? error.message : String(error);
      errorMessage = updateErrorMessage(code);
      await refreshUpdateState();
    } finally {
      updateBusy = false;
    }
  }

  async function chooseNewBackupDirectory(): Promise<void> {
    if (!desktopState || backupDirectoryChoosing || backupExporting || backupRestoreBusy) return;
    backupDirectoryChoosing = true;
    errorMessage = "";
    noticeMessage = "";
    try {
      const selected = await chooseBackupDirectory();
      if (!selected) return;
      const previous = desktopState;
      const device = await setBackupDirectory({
        path: selected,
        expectedDeviceRevision: previous.device.revision,
      });
      desktopState = { ...previous, device };
      noticeMessage = "备份目录已更新";
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      backupDirectoryChoosing = false;
    }
  }

  async function confirmBackupExport(): Promise<void> {
    if (backupExporting || backupDirectoryChoosing || backupRestoreBusy) return;
    backupExportConfirmOpen = false;
    backupExporting = true;
    errorMessage = "";
    noticeMessage = "";
    try {
      const destination = await chooseBackupExportPath();
      if (!destination) return;
      await exportBackup(destination);
      noticeMessage = "备份已导出";
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      backupExporting = false;
    }
  }

  async function chooseBackupForRestore(): Promise<void> {
    if (backupRestoreBusy || backupExporting || backupDirectoryChoosing) return;
    backupRestoreBusy = true;
    errorMessage = "";
    noticeMessage = "";
    try {
      const path = await chooseBackupImportPath();
      if (!path) return;
      const summary = await inspectBackupImport(path);
      backupRestoreCandidate = { path, summary };
      resetBackupMergeState();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      backupRestoreBusy = false;
    }
  }

  async function confirmBackupRestore(): Promise<void> {
    if (!backupRestoreCandidate || backupRestoreBusy) return;
    backupRestoreBusy = true;
    errorMessage = "";
    noticeMessage = "";
    const candidate = backupRestoreCandidate;
    try {
      await settingsSaveQueue.enqueue(async () => undefined);
      const restored = await restoreBackupReplace(candidate.path);
      await refreshAfterBackupRestore();
      closeBackupRestoreDialog();
      noticeMessage = `备份恢复完成，恢复前安全备份位于 ${restored.safetyBackupPath}`;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      backupRestoreBusy = false;
    }
  }

  function resetBackupMergeState(): void {
    backupMergePlan = null;
    backupFolderResolutions = {};
    backupFolderRenameNames = {};
    backupSettingsResolutionMode = "current";
    backupSettingChoices = {};
  }

  function closeBackupRestoreDialog(): void {
    backupRestoreCandidate = null;
    resetBackupMergeState();
  }

  async function prepareBackupMergePlan(): Promise<void> {
    if (!backupRestoreCandidate || backupRestoreBusy) return;
    backupRestoreBusy = true;
    errorMessage = "";
    noticeMessage = "";
    try {
      await settingsSaveQueue.enqueue(async () => undefined);
      backupMergePlan = await prepareBackupMerge(backupRestoreCandidate.path);
      backupFolderResolutions = {};
      backupFolderRenameNames = {};
      backupSettingsResolutionMode = "current";
      backupSettingChoices = {};
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      backupRestoreBusy = false;
    }
  }

  function updateBackupFolderResolution(conflictId: string, action: string): void {
    if (action === "merge" || action === "skip") {
      backupFolderResolutions = {
        ...backupFolderResolutions,
        [conflictId]: { action },
      };
      return;
    }
    if (action === "rename") {
      const name = backupFolderRenameNames[conflictId] ?? "";
      backupFolderResolutions = {
        ...backupFolderResolutions,
        [conflictId]: { action: "rename", name },
      };
      return;
    }
    const next = { ...backupFolderResolutions };
    delete next[conflictId];
    backupFolderResolutions = next;
  }

  function updateBackupFolderRename(conflictId: string, name: string): void {
    backupFolderRenameNames = { ...backupFolderRenameNames, [conflictId]: name };
    backupFolderResolutions = {
      ...backupFolderResolutions,
      [conflictId]: { action: "rename", name },
    };
  }

  function updateBackupSettingChoice(conflictId: string, source: string): void {
    if (source !== "current" && source !== "backup") return;
    backupSettingChoices = { ...backupSettingChoices, [conflictId]: source };
  }

  function currentBackupSettingsResolution(): SettingsMergeResolution {
    if (backupSettingsResolutionMode === "backup") return { mode: "backup" };
    if (backupSettingsResolutionMode === "perField") {
      return { mode: "perField", choices: backupSettingChoices };
    }
    return { mode: "current" };
  }

  async function confirmBackupMerge(): Promise<void> {
    if (!backupRestoreCandidate || !backupMergePlan || !backupMergeReady || backupRestoreBusy) return;
    backupRestoreBusy = true;
    errorMessage = "";
    noticeMessage = "";
    const candidate = backupRestoreCandidate;
    const plan = backupMergePlan;
    try {
      await settingsSaveQueue.enqueue(async () => undefined);
      const restored = await restoreBackupMerge(
        candidate.path,
        plan,
        backupFolderResolutions,
        currentBackupSettingsResolution(),
      );
      await refreshAfterBackupRestore();
      closeBackupRestoreDialog();
      noticeMessage = `备份合并完成，恢复前安全备份位于 ${restored.safetyBackupPath}`;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      backupRestoreBusy = false;
    }
  }

  async function refreshAfterBackupRestore(): Promise<void> {
    await refreshAccounts(false);
    await Promise.all([
      refreshAppSettings(activeAccount?.id ?? null),
      refreshDesktopState(),
      refreshSchedulerState(),
      refreshSyncStatus(),
      refreshContent(1),
      refreshCoverCacheTask(),
    ]);
  }

  function formatMergeSettingValue(value: unknown): string {
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
      return String(value);
    }
    return JSON.stringify(value);
  }

  function setThemeMode(value: ThemeMode): void {
    if (themeMode === value) return;
    themeMode = value;
    saveGlobalSettings();
  }

  function applyThemeMode(value: ThemeMode): void {
    if (typeof document === "undefined") return;
    document.documentElement.dataset.theme = value;
    document.documentElement.style.colorScheme = value === "system" ? "light dark" : value;
  }

  function saveAccountSettings(): void {
    const accountId = activeAccount?.id ?? null;
    const settings = currentAccountSettings();
    if (confirmedAccountSettings && settingsEqual(settings, confirmedAccountSettings)) return;
    const generation = beginSettingsSave();
    void settingsSaveQueue.enqueue(async () => {
      try {
        const revision = accountId === null
          ? await updateAccountDefaults(settings)
          : await updateAccountSettings(accountId, settings);
        confirmAccountSettings(settings, revision, accountId);
        completeSettingsSave(generation);
      } catch (error) {
        if ((activeAccount?.id ?? null) === accountId) {
          if (generation === settingsSaveGeneration && confirmedAccountSettings) {
            applyAccountSettingsValues(confirmedAccountSettings);
          }
          errorMessage = error instanceof Error ? error.message : String(error);
        }
        failSettingsSave(generation);
      }
    });
  }

  function beginSettingsSave(): number {
    clearSettingsSaveFeedbackTimer();
    settingsRequestGeneration.begin();
    settingsLoading = false;
    settingsSaveGeneration += 1;
    settingsSaveState = "saving";
    return settingsSaveGeneration;
  }

  function completeSettingsSave(generation: number): void {
    if (generation !== settingsSaveGeneration) return;
    clearSettingsSaveFeedbackTimer();
    settingsSaveState = "saved";
    settingsSaveFeedbackTimer = window.setTimeout(() => {
      if (generation === settingsSaveGeneration && settingsSaveState === "saved") {
        settingsSaveState = "fading";
      }
      settingsSaveFeedbackTimer = null;
    }, 3_000);
  }

  function failSettingsSave(generation: number): void {
    if (generation !== settingsSaveGeneration) return;
    clearSettingsSaveFeedbackTimer();
    settingsSaveState = "error";
  }

  function clearSettingsSaveFeedbackTimer(): void {
    if (settingsSaveFeedbackTimer === null || typeof window === "undefined") return;
    window.clearTimeout(settingsSaveFeedbackTimer);
    settingsSaveFeedbackTimer = null;
  }

  function hideSettingsSaveFeedback(): void {
    if (settingsSaveState === "fading") settingsSaveState = "idle";
  }

  function settingsEqual<T>(left: T, right: T): boolean {
    return JSON.stringify(left) === JSON.stringify(right);
  }

  function confirmAccountSettings(
    settings: AccountSettings,
    revision: number,
    accountId: number | null,
  ): void {
    confirmedAccountSettings = structuredClone(settings);
    if (!appSettings) return;
    appSettings = accountId === null
      ? {
          ...appSettings,
          revision,
          accountDefaults: structuredClone(settings),
          account: structuredClone(settings),
        }
      : { ...appSettings, revision, account: structuredClone(settings) };
  }

  function applyAccountSettingsValues(settings: AccountSettings): void {
    accountSettings = structuredClone(settings);
    syncMode = settings.history.range.kind === "bottom" ? "bottom" : "days";
    syncDays = settings.history.range.kind === "days"
      ? normalizeSyncDays(settings.history.range.days)
      : 7;
    scrollSpeed = normalizeScrollSpeed(settings.scrollSpeed);
    hideBrowser = settings.hideBrowser;
  }

  function applyGlobalSettingsValues(settings: GlobalSettings): void {
    themeMode = settings.theme ?? "system";
    likeCountFormat = normalizeLikeCountFormat(settings.likeCountFormat);
    detailFields = { ...settings.detailFields };
    closeBehavior = settings.closeBehavior;
    startupSyncDelayMinutes = settings.startupSyncDelayMinutes;
    coverCacheSettings = { ...settings.coverCache };
    backupSettings = { ...settings.backup };
    notificationSettings = { ...settings.notifications };
    updateSettings = { ...settings.updates };
  }

  function syncStatusLabel(status: SyncJobStatus | null): string {
    switch (status) {
      case "queued": return "等待中";
      case "running": return "同步中";
      case "success": return "成功";
      case "partial": return "部分完成";
      case "failed": return "失败";
      case "cancelled": return "已取消";
      default: return "暂无";
    }
  }

  function batchEntryLabel(entry: BatchEntriesState["entries"][number]): string {
    if (entry.kind === "job") {
      return `${sourceLabel(entry.data.source)}：${syncStatusLabel(entry.data.status)}`;
    }
    return `${sourceLabel(entry.data.source)}：${entry.data.reason === "collector_not_implemented" ? "采集器尚未实现" : entry.data.reason}`;
  }

  function sourceLabel(source: SyncSource): string {
    if (source === "history") return "观看历史";
    if (source === "likes") return "喜欢";
    if (source === "favorites") return "收藏";
    return "远端收藏夹";
  }

  function clampInteger(value: number, minimum: number, maximum: number, fallback: number): number {
    if (!Number.isFinite(value)) return fallback;
    return Math.max(minimum, Math.min(maximum, Math.floor(value)));
  }

  function errorCode(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  function applyAppSettings(settings: AppSettingsView): void {
    appSettings = settings;
    applyAccountSettingsValues(settings.account);
    applyGlobalSettingsValues(settings.global);
    confirmedAccountSettings = structuredClone(settings.account);
    confirmedGlobalSettings = structuredClone(settings.global);
    confirmedWorkspaceSettings = structuredClone(settings.workspace);
    clearSettingsSaveFeedbackTimer();
    settingsSaveState = "idle";
    dataRootValidation = null;
  }

  async function refreshAppSettings(accountId: number | null): Promise<void> {
    const generation = settingsRequestGeneration.begin();
    settingsLoading = true;
    try {
      const settings = await loadSettingsWithLegacyImport({
        accountId,
        storage: window.localStorage,
        getAppSettings,
        importLegacySettings,
      });
      if (!settingsRequestGeneration.isCurrent(generation)) return;
      const pendingOnboardingRoot = onboardingDraft?.selectedDataRoot ?? null;
      applyAppSettings(settings);
      if (onboardingDraft && pendingOnboardingRoot) {
        dataRootValidation = {
          requestedPath: pendingOnboardingRoot,
          normalizedPath: pendingOnboardingRoot,
          adjustedToInstallData: false,
        };
      }
      if (onboardingDraft && !pendingOnboardingRoot) {
        onboardingDraft = {
          ...onboardingDraft,
          dataRoot: settings.dataRoot,
          portableRevision: settings.revision,
        };
      }
    } catch (error) {
      if (settingsRequestGeneration.isCurrent(generation)) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (settingsRequestGeneration.isCurrent(generation)) settingsLoading = false;
    }
  }

  function handleWorkspaceScroll(): void {
    if (workspaceMainElement) {
      routeScrollPositions.set(activePage, workspaceMainElement.scrollTop);
    }
  }

  function navigateToPage(page: AppPage): void {
    if (page === activePage) return;
    handleWorkspaceScroll();
    activePage = page;
    void restoreRouteScroll(page);
  }

  async function restoreRouteScroll(page: AppPage): Promise<void> {
    await tick();
    if (workspaceMainElement && activePage === page) {
      workspaceMainElement.scrollTop = routeScrollPositions.get(page) ?? 0;
    }
  }

  async function openSettings(): Promise<void> {
    navigateToPage("settings");
    selectedIds = new Set();
    await refreshCoverCacheTask();
  }

  async function openCoverCacheManager(): Promise<void> {
    await settingsSaveQueue.enqueue(async () => undefined);
    coverCacheManagerOpen = true;
  }

  async function refreshCoverCacheTask(): Promise<void> {
    try {
      coverCacheTask = await getCoverCacheTask();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  function requestCoverCacheEnabled(checked: boolean): void {
    if (checked && !coverCacheSettings.enabled) {
      coverCacheEnableOpen = true;
      return;
    }
    coverCacheSettings = { ...coverCacheSettings, enabled: false };
    void saveGlobalSettings().then(refreshCoverCacheTask);
  }

  function enableCoverCache(backfillMode: CoverCacheBackfillMode): void {
    coverCacheSettings = { ...coverCacheSettings, enabled: true, backfillMode };
    coverCacheEnableOpen = false;
    void saveGlobalSettings().then(refreshCoverCacheTask);
  }

  function updateCoverCacheSettings(patch: Partial<CoverCacheSettings>): void {
    coverCacheSettings = { ...coverCacheSettings, ...patch };
    saveGlobalSettings();
  }

  async function runCoverCacheTaskAction(
    action: "pause" | "resume" | "cancel" | "retry",
  ): Promise<void> {
    if (!coverCacheTask || coverCacheTaskBusy) return;
    coverCacheTaskBusy = true;
    try {
      coverCacheTask =
        action === "pause"
          ? await pauseCoverCacheTask(coverCacheTask.id)
          : action === "resume"
            ? await resumeCoverCacheTask(coverCacheTask.id)
            : action === "cancel"
              ? await cancelCoverCacheTask(coverCacheTask.id)
              : await retryCoverCacheTask(coverCacheTask.id);
      if (action === "resume" || action === "retry") {
        window.setTimeout(() => void refreshCoverCacheTask(), 250);
      }
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      coverCacheTaskBusy = false;
    }
  }

  function coverCacheTaskLabel(task: CoverCacheTaskView): string {
    const kind = task.kind === "backfill" ? "补齐全部历史" : "缓存新增封面";
    const status =
      task.status === "running" ? "进行中" :
      task.status === "paused" ? "已暂停" :
      task.status === "success" ? "已完成" :
      task.status === "failed" ? "失败" :
      task.status === "cancelled" ? "已取消" : "等待中";
    return `${kind} · ${status}`;
  }

  function coverCacheProgress(task: CoverCacheTaskView): number {
    if (task.totalItems === 0) return task.status === "success" ? 100 : 0;
    return Math.min(100, Math.round((task.completedItems / task.totalItems) * 100));
  }

  async function chooseNewDataRoot(): Promise<void> {
    if (syncing || dataRootMigrating) return;
    dataRootChoosing = true;
    errorMessage = "";
    noticeMessage = "";
    try {
      const selected = await chooseDataRoot();
      if (!selected) return;
      dataRootValidation = await validateDataRoot(selected);
      if (onboardingDraft && dataRootValidation) {
        onboardingDraft = {
          ...onboardingDraft,
          dataRoot: dataRootValidation.normalizedPath,
          selectedDataRoot: dataRootValidation.normalizedPath,
        };
      }
    } catch (error) {
      dataRootValidation = null;
      errorMessage = dataRootErrorMessage(error);
    } finally {
      dataRootChoosing = false;
    }
  }

  async function migrateDataRoot(): Promise<void> {
    const target = dataRootValidation?.normalizedPath;
    if (!target || dataRootMigrating || syncing) return;
    dataRootMigrating = true;
    errorMessage = "";
    noticeMessage = "";
    try {
      const result = await settingsSaveQueue.enqueue(() => setDataRoot(target));
      noticeMessage = result.retainedSource
        ? `数据已迁移，旧目录保留在 ${result.retainedSource.path}`
        : "数据目录迁移完成";
      await refreshAppSettings(activeAccount?.id ?? null);
      if (onboardingDraft) {
        cancelDataRootSelection();
        onboardingDraft = {
          ...onboardingDraft,
          dataRoot: result.dataRoot,
          portableRevision: appSettings?.revision ?? onboardingDraft.portableRevision,
        };
      }
    } catch (error) {
      errorMessage = dataRootErrorMessage(error);
    } finally {
      dataRootMigrating = false;
    }
  }

  async function confirmRetainedRootDelete(): Promise<void> {
    const path = retainedRootPendingDelete;
    if (!path || retainedRootDeleting || dataRootMigrating) return;
    retainedRootDeleting = true;
    errorMessage = "";
    try {
      await deleteRetainedDataRoot(path);
      retainedRootPendingDelete = null;
      noticeMessage = `已删除旧数据目录 ${path}`;
      await refreshAppSettings(activeAccount?.id ?? null);
    } catch (error) {
      errorMessage = dataRootErrorMessage(error);
    } finally {
      retainedRootDeleting = false;
    }
  }

  function closeDownloadDialog(): void {
    downloadDialogAccountId = null;
    downloadDialogSelection = null;
    downloadDialogPreview = null;
  }

  async function openDownloadDialog(mode: "checked" | "filtered" | "all"): Promise<void> {
    if (!activeAccount || downloadPreviewing) return;
    const accountId = activeAccount.id;
    const selection = buildDownloadSelection(
      mode === "checked"
        ? { mode, checkedIds: selectedIds }
        : mode === "filtered"
          ? { mode, query }
          : { mode },
    );
    if (selection.kind === "checked" && selection.watch_record_ids.length === 0) return;

    downloadPreviewing = true;
    errorMessage = "";
    try {
      const preview = await previewDownloadSelection(accountId, selection);
      if (activeAccount?.id !== accountId) return;
      downloadDialogAccountId = accountId;
      downloadDialogSelection = selection;
      downloadDialogPreview = preview;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      downloadPreviewing = false;
    }
  }

  function handleDownloadCreated(job: DownloadJobDetail): void {
    closeDownloadDialog();
    selectedIds = new Set();
    navigateToPage("syncRecords");
    noticeMessage = `下载任务 #${job.id} 已创建`;
  }

  function toggleRecordSelection(id: number, checked: boolean): void {
    const next = new Set(selectedIds);
    if (checked) {
      next.add(id);
    } else {
      next.delete(id);
    }
    selectedIds = next;
  }

  function toggleVisibleSelection(checked: boolean): void {
    const next = new Set(selectedIds);
    for (const record of records) {
      if (checked) {
        next.add(record.id);
      } else {
        next.delete(record.id);
      }
    }
    selectedIds = next;
  }

  function normalizePage(value: number): number {
    if (!Number.isFinite(value)) {
      return 1;
    }

    return Math.max(1, Math.floor(value));
  }

  function updateQuery(value: string): void {
    contentViewStates = updateContentViewState(contentViewStates, currentSource, {
      query: value,
      page: 1,
    });
    selectedIds = new Set();
    void refreshContent(1);
  }

  function goToPage(page: number): void {
    void refreshContent(page);
  }

  function selectContentSource(source: ContentSource): void {
    navigateToPage(source);
    if (source === currentSource && records.length > 0) return;
    currentSource = source;
    selectedIds = new Set();
    void refreshContent(contentViewStates[source].page, source);
  }

  function selectPage(page: AppPage): void {
    closeDownloadDialog();
    if (page === "accounts") {
      openAccountManager();
      return;
    }
    if (page === "history" || page === "likes" || page === "favorites") {
      selectContentSource(page);
      return;
    }
    navigateToPage(page);
    selectedIds = new Set();
    if (page === "settings") void refreshCoverCacheTask();
  }

  function setContentViewMode(viewMode: "list" | "grid"): void {
    contentViewStates = updateContentViewState(contentViewStates, currentSource, { viewMode });
  }

  function setIncludeRemoteRemoved(includeRemoteRemoved: boolean): void {
    contentViewStates = updateContentViewState(contentViewStates, currentSource, {
      includeRemoteRemoved,
      page: 1,
    });
    void refreshContent(1);
  }

  function setContentSort(sort: ContentSort): void {
    contentViewStates = updateContentViewState(contentViewStates, currentSource, {
      sort,
      page: 1,
    });
    selectedIds = new Set();
    void refreshContent(1);
  }

  async function openRecord(record: ContentRecordView): Promise<void> {
    errorMessage = "";
    try {
      await openContentUrl(record.url);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  async function openFolderPicker(record: ContentRecordView): Promise<void> {
    if (!activeAccount) return;
    folderPickerRecord = record;
    folderPickerFolders = [];
    folderPickerLoading = true;
    folderPickerCreateOpen = false;
    folderPickerName = "";
    errorMessage = "";
    try {
      const accountId = activeAccount.id;
      const folders = await listLocalFolders(accountId);
      if (folderPickerRecord?.awemeId === record.awemeId && activeAccount?.id === accountId) {
        folderPickerFolders = folders;
      }
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
      folderPickerRecord = null;
    } finally {
      folderPickerLoading = false;
    }
  }

  function closeFolderPicker(): void {
    if (folderPickerBusy) return;
    folderPickerRecord = null;
    folderPickerFolders = [];
    folderPickerCreateOpen = false;
    folderPickerName = "";
  }

  async function addRecordToFolder(folder: LocalFolderView): Promise<void> {
    if (!activeAccount || !folderPickerRecord || folderPickerBusy) return;
    folderPickerBusy = true;
    errorMessage = "";
    try {
      const result = await addLocalFolderItem(
        activeAccount.id,
        folder.id,
        folderPickerRecord.awemeId,
      );
      noticeMessage = result.insertedItem || result.insertedProvenance
        ? `已加入本地收藏夹“${folder.name}”`
        : `该内容已在“${folder.name}”中`;
      closeFolderPickerAfterMutation();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      folderPickerBusy = false;
    }
  }

  async function createFolderAndAdd(): Promise<void> {
    const name = folderPickerName.trim();
    if (!activeAccount || !folderPickerRecord || !name || folderPickerBusy) return;
    folderPickerBusy = true;
    errorMessage = "";
    try {
      const folder = await createLocalFolder(activeAccount.id, name, folderPickerScope);
      await addLocalFolderItem(activeAccount.id, folder.id, folderPickerRecord.awemeId);
      noticeMessage = `已新建并加入本地收藏夹“${folder.name}”`;
      closeFolderPickerAfterMutation();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      folderPickerBusy = false;
    }
  }

  function closeFolderPickerAfterMutation(): void {
    folderPickerRecord = null;
    folderPickerFolders = [];
    folderPickerCreateOpen = false;
    folderPickerName = "";
  }

  async function deleteOne(record: ContentRecordView): Promise<void> {
    if (currentSource !== "history") return;
    const name = record.title ?? "未命名视频";
    if (!window.confirm(`删除这条本地观看记录？\n${name}`)) {
      return;
    }
    await deleteIds([record.id]);
  }

  async function deleteSelected(): Promise<void> {
    if (selectedIds.size === 0) {
      return;
    }
    if (!window.confirm(`删除选中的 ${selectedIds.size} 条本地观看记录？`)) {
      return;
    }
    await deleteIds([...selectedIds]);
  }

  async function deleteCurrentScope(): Promise<void> {
    if (total === 0) {
      return;
    }
    const trimmed = query.trim();
    const scope = trimmed ? `当前搜索结果中的 ${total} 条` : `全部 ${total} 条`;
    if (!window.confirm(`删除${scope}本地观看记录？此操作不会删除抖音账号中的记录。`)) {
      return;
    }

    deleting = true;
    errorMessage = "";
    noticeMessage = "";
    try {
      if (!activeAccount) return;
      const result = await deleteHistoryByQuery(activeAccount.id, trimmed || undefined);
      selectedIds = new Set();
      noticeMessage = `已删除 ${result.deletedWatchRecords} 条本地观看记录`;
      await refreshContent();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      deleting = false;
    }
  }

  async function deleteIds(ids: number[]): Promise<void> {
    deleting = true;
    errorMessage = "";
    noticeMessage = "";
    try {
      if (!activeAccount) return;
      const result = await deleteHistoryRecords(activeAccount.id, ids);
      const next = new Set(selectedIds);
      for (const id of ids) {
        next.delete(id);
      }
      selectedIds = next;
      noticeMessage = `已删除 ${result.deletedWatchRecords} 条本地观看记录`;
      await refreshContent();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      deleting = false;
    }
  }

  async function refreshAccounts(includeArchived = activePage === "accounts"): Promise<void> {
    const [nextAccounts, nextActive] = await Promise.all([
      listAccounts(includeArchived),
      getActiveAccount(),
    ]);
    accounts = nextAccounts.filter((account) => account.status !== "pending_login");
    activeAccount = nextActive;
  }

  async function refreshDesktopState(): Promise<void> {
    const next = await getDesktopState();
    desktopState = next;
    browserPreference = activeAccount
      ? (next.device.accounts[activeAccount.profileKey]?.browserPreference ?? "auto")
      : "auto";
  }

  async function refreshSchedulerState(): Promise<void> {
    schedulerState = await getSchedulerState();
  }

  async function refreshRuntimeState(shouldRefreshContent = false): Promise<void> {
    await Promise.all([
      refreshDesktopState(),
      refreshSchedulerState(),
      refreshSyncStatus(),
    ]);
    if (shouldRefreshContent) {
      await Promise.all([refreshAccounts(activePage === "accounts"), refreshContent()]);
    }
  }

  function initializeOnboarding(): void {
    if (!appSettings || !desktopState || desktopState.device.onboardingCompleted) {
      onboardingDraft = null;
      return;
    }
    onboardingDraft = createOnboardingDraft(appSettings, desktopState);
  }

  function updateOnboardingHistoryRange(mode: SyncMode): void {
    if (!onboardingDraft) return;
    const accountDefaults = structuredClone(onboardingDraft.accountDefaults);
    accountDefaults.history.range =
      mode === "bottom"
        ? { kind: "bottom" }
        : {
            kind: "days",
            days:
              accountDefaults.history.range.kind === "days"
                ? accountDefaults.history.range.days
                : 7,
          };
    onboardingDraft = { ...onboardingDraft, accountDefaults };
  }

  function updateOnboardingHistoryDays(value: string): void {
    if (!onboardingDraft) return;
    const accountDefaults = structuredClone(onboardingDraft.accountDefaults);
    accountDefaults.history.range = { kind: "days", days: normalizeSyncDays(value) };
    onboardingDraft = { ...onboardingDraft, accountDefaults };
  }

  function updateOnboardingArchiveRange(
    source: "likes" | "favorites",
    kind: ArchiveListRange["kind"],
  ): void {
    if (!onboardingDraft) return;
    const accountDefaults = structuredClone(onboardingDraft.accountDefaults);
    const archive = archiveSettingsFor(accountDefaults, source);
    archive.range = archiveRangeForKind(archive.range, kind);
    onboardingDraft = { ...onboardingDraft, accountDefaults };
  }

  function updateOnboardingArchiveRangeValue(
    source: "likes" | "favorites",
    value: string,
  ): void {
    if (!onboardingDraft) return;
    const accountDefaults = structuredClone(onboardingDraft.accountDefaults);
    const archive = archiveSettingsFor(accountDefaults, source);
    switch (archive.range.kind) {
      case "recentItems":
        archive.range.count = clampInteger(Number(value), 1, 5_000, 500);
        break;
      case "recordLimit":
        archive.range.limit = clampInteger(Number(value), 1, 50_000, 5_000);
        break;
      case "recentDays":
        archive.range.days = clampInteger(Number(value), 1, 365, 7);
        break;
      case "all":
        return;
    }
    onboardingDraft = { ...onboardingDraft, accountDefaults };
  }

  function updateOnboardingSchedule(
    source: "history" | "likes" | "favorites",
    enabled?: boolean,
    intervalMinutes?: number,
  ): void {
    if (!onboardingDraft) return;
    const accountDefaults = structuredClone(onboardingDraft.accountDefaults);
    const schedule =
      source === "history"
        ? accountDefaults.history.schedule
        : source === "likes"
          ? accountDefaults.likes.schedule
          : accountDefaults.favorites.archive.schedule;
    if (typeof enabled === "boolean") schedule.enabled = enabled;
    if (typeof intervalMinutes === "number") {
      schedule.intervalMinutes = Math.max(15, Math.min(43_200, Math.floor(intervalMinutes)));
    }
    onboardingDraft = { ...onboardingDraft, accountDefaults };
  }

  async function initializeAccounts(): Promise<void> {
    loading = true;
    try {
      await refreshAccounts(false);
      await Promise.all([
        refreshAppSettings(activeAccount?.id ?? null),
        refreshDesktopState(),
        refreshSchedulerState(),
        refreshUpdateState(),
        refreshContent(1),
      ]);
      initializeOnboarding();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
      loading = false;
    }
  }

  async function initializeApp(): Promise<void> {
    try {
      startupRecoveryStatus = await getStartupRecoveryStatus();
      if (startupRecoveryStatus.state === "ready") {
        await initializeAccounts();
      } else {
        loading = false;
        settingsLoading = false;
      }
    } catch (error) {
      startupRecoveryStatus = {
        state: "failed",
        errorCode: error instanceof Error ? error.message : String(error),
        canResolve: false,
        canRetry: false,
      };
      loading = false;
      settingsLoading = false;
    }
  }

  async function resolveStartupRecovery(
    disposition: StartupRecoveryDisposition,
  ): Promise<void> {
    if (startupRecoveryBusy) return;
    startupRecoveryBusy = true;
    errorMessage = "";
    try {
      await resolveStartupRecoveryFault(disposition);
      startupRecoveryStatus = await getStartupRecoveryStatus();
      if (startupRecoveryStatus.state !== "ready") {
        throw new Error(startupRecoveryStatus.errorCode ?? "startup_not_ready");
      }
      await initializeAccounts();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
      startupRecoveryStatus = await getStartupRecoveryStatus().catch(() => startupRecoveryStatus);
    } finally {
      startupRecoveryBusy = false;
    }
  }

  function resetForAccountSwitch(): void {
    const reset = resetHistoryViewForAccountSwitch();
    closeDownloadDialog();
    contentViewStates = resetContentViewStatesForAccount(contentViewStates);
    selectedIds = reset.selectedIds;
    currentSyncJobId = 0;
    syncing = false;
    cancelRequested = false;
    syncStatus = {
      accountId: activeAccount?.id ?? 0,
      jobId: 0,
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

  async function switchAccount(account: AccountView): Promise<void> {
    if (account.id === activeAccount?.id || account.status === "pending_login" || account.status === "archived") return;
    accountBusy = true;
    errorMessage = "";
    try {
      activeAccount = await setActiveAccount(account.id);
      accountMenuOpen = false;
      resetForAccountSwitch();
      await refreshAccounts(activePage === "accounts");
      await refreshAppSettings(activeAccount?.id ?? null);
      await refreshContent(1);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      accountBusy = false;
    }
  }

  async function addAccount(): Promise<void> {
    accountLoginBusy = true;
    accountMenuOpen = false;
    errorMessage = "";
    try {
      activeAccount = await beginAccountLogin(
        onboardingDraft ? onboardingBrowserPreference : undefined,
      );
      resetForAccountSwitch();
      await refreshAccounts(activePage === "accounts");
      await refreshDesktopState();
      if (onboardingDraft && desktopState) {
        onboardingDraft = {
          ...onboardingDraft,
          deviceRevision: desktopState.device.revision,
        };
      }
      await refreshAppSettings(activeAccount?.id ?? null);
      await refreshContent(1);
    } catch (error) {
      errorMessage = accountLoginErrorMessage(error);
      await refreshAccounts(activePage === "accounts").catch(() => undefined);
    } finally {
      accountLoginBusy = false;
    }
  }

  async function submitOnboarding(skippedAccount?: boolean): Promise<void> {
    if (!onboardingDraft || onboardingBusy) return;
    const currentStep = onboardingDraft.step;
    onboardingBusy = true;
    errorMessage = "";
    if (typeof skippedAccount === "boolean") {
      onboardingDraft = { ...onboardingDraft, skippedAccount };
    }
    try {
      if (currentStep === 2 && onboardingDraft.selectedDataRoot) {
        dataRootMigrating = true;
        const result = await initializeOnboardingDataRoot(onboardingDraft.selectedDataRoot);
        dataRootValidation = null;
        await refreshAppSettings(activeAccount?.id ?? null);
        onboardingDraft = {
          ...onboardingDraft,
          dataRoot: result.dataRoot,
          selectedDataRoot: null,
          portableRevision: appSettings?.revision ?? onboardingDraft.portableRevision,
        };
        dataRootMigrating = false;
      }
      const result = await commitOnboardingStep(buildOnboardingRequest(onboardingDraft));
      desktopState = desktopState
        ? { ...desktopState, device: result.device }
        : await getDesktopState();
      onboardingDraft = applyOnboardingCommit(
        onboardingDraft,
        result.portableRevision,
        result.device.revision,
        result.device.onboardingStep,
        result.device.onboardingCompleted,
      );
      if (result.device.onboardingCompleted) {
        const syncAfterOnboarding = onboardingDraft.syncAfterOnboarding;
        onboardingDraft = null;
        await Promise.all([
          refreshAppSettings(activeAccount?.id ?? null),
          refreshSchedulerState(),
        ]);
        if (syncAfterOnboarding && activeAccount && canSyncAccount(activeAccount.status)) {
          try {
            await enqueueSyncTasks({
              scope: "currentAccountAvailableSources",
              accountId: activeAccount.id,
              source: null,
              sourceInstanceKey: null,
              payload: null,
              fullReconcileConfirmationSha256: null,
            });
            noticeMessage = "首次设置已完成，已加入同步队列";
          } catch (error) {
            errorMessage = `首次设置已完成；立即同步失败：${error instanceof Error ? error.message : String(error)}`;
          }
        } else {
          noticeMessage = "首次设置已完成";
        }
      } else if (currentStep === 0) {
        onboardingBrowserPreference = "auto";
      }
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      dataRootMigrating = false;
      onboardingBusy = false;
    }
  }

  function cancelDataRootSelection(): void {
    dataRootValidation = null;
    if (onboardingDraft) {
      onboardingDraft = {
        ...onboardingDraft,
        dataRoot: appSettings?.dataRoot ?? onboardingDraft.dataRoot,
        selectedDataRoot: null,
      };
    }
  }

  function goBackOnboarding(): void {
    if (!onboardingDraft || onboardingBusy || accountLoginBusy || dataRootMigrating) return;
    if (onboardingDraft.step === 2) cancelDataRootSelection();
    onboardingDraft = stepBack(onboardingDraft);
    errorMessage = "";
  }

  async function consumeDesktopIntents(): Promise<void> {
    const intent = await takeDesktopIntents(desktopIntentRevision);
    if (intent.revision <= desktopIntentRevision) return;
    desktopIntentRevision = intent.revision;
    if (intent.openSettings) void openSettings();
  }

  async function resumeLogin(account: AccountView): Promise<void> {
    accountLoginBusy = true;
    errorMessage = "";
    try {
      activeAccount = await resumeAccountLogin(account.id);
      resetForAccountSwitch();
      await refreshAccounts(activePage === "accounts");
      await refreshAppSettings(activeAccount?.id ?? null);
      await refreshContent(1);
    } catch (error) {
      errorMessage = accountLoginErrorMessage(error);
    } finally {
      accountLoginBusy = false;
    }
  }

  async function openAccountManager(): Promise<void> {
    accountMenuOpen = false;
    navigateToPage("accounts");
    accountBusy = true;
    try {
      await refreshAccounts(true);
    } finally {
      accountBusy = false;
    }
  }

  async function manageAccount(account: AccountView, action: "archive" | "restore" | "cancel" | "delete"): Promise<void> {
    if (action === "delete" && !window.confirm(`删除账号“${account.nickname ?? account.uid ?? account.id}”的全部本地数据？`)) return;
    if (action === "archive" && !window.confirm(`归档账号“${account.nickname ?? account.uid ?? account.id}”？`)) return;
    accountBusy = true;
    errorMessage = "";
    try {
      if (action === "archive") await archiveAccount(account.id);
      if (action === "restore") await restoreAccount(account.id);
      if (action === "cancel") await cancelPendingAccount(account.id);
      if (action === "delete") await deleteAccount(account.id, { type: "delete" });
      await refreshAccounts(true);
      resetForAccountSwitch();
      await refreshAppSettings(activeAccount?.id ?? null);
      await refreshContent(1);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      accountBusy = false;
    }
  }

  function formatDate(value?: string): string {
    if (!value) {
      return "-";
    }

    const date = new Date(value);
    if (Number.isNaN(date.getTime())) {
      return value;
    }

    return new Intl.DateTimeFormat("zh-CN", {
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  }

  function watchDateLabel(record: ContentRecordView): string {
    return record.watchedDate || "未知观看日期";
  }

  function formatElapsed(milliseconds: number): string {
    const seconds = Math.max(0, Math.floor(milliseconds / 1000));
    const minutes = Math.floor(seconds / 60);
    const remainder = seconds % 60;
    return minutes > 0 ? `${minutes}分${remainder}秒` : `${remainder}秒`;
  }

  function isSyncInProgress(status: SyncStatus["status"]): boolean {
    return status === "queued" || status === "running";
  }

  initializeApp();
  onMount(() => {
    let mounted = true;
    const unlisteners: Array<() => void> = [];
    refreshController = new SchedulerRefreshController(async () => {
      await Promise.all([refreshSchedulerState(), refreshSyncStatus()]);
    });
    refreshController.update(document.visibilityState !== "hidden", hasActiveRuntimeWork);
    const onVisibilityChange = () => {
      windowVisible = document.visibilityState !== "hidden";
      refreshController?.update(windowVisible, hasActiveRuntimeWork);
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    const coverCachePoll = window.setInterval(() => {
      if (
        activePage === "settings" &&
        coverCacheTask &&
        ["queued", "running"].includes(coverCacheTask.status)
      ) {
        void refreshCoverCacheTask();
      }
    }, 1_000);

    void Promise.all([
      listenAppEvent("desktop:intent-changed", () => void consumeDesktopIntents()),
      listenAppEvent("scheduler:changed", () => {
        void refreshRuntimeState(false);
        if (schedulerInstances) void loadSchedulerInstances(true);
      }),
      listenAppEvent("sync:changed", () => {
        void refreshRuntimeState(true);
        void refreshActiveBatch(false);
      }),
      listenAppEvent("update:changed", () => void refreshUpdateState()),
    ]).then(async (listeners) => {
      if (!mounted) {
        listeners.forEach((unlisten) => unlisten());
        return;
      }
      unlisteners.push(...listeners);
      await consumeDesktopIntents();
    }).catch(() => undefined);

    return () => {
      mounted = false;
      document.removeEventListener("visibilitychange", onVisibilityChange);
      window.clearInterval(coverCachePoll);
      refreshController?.dispose();
      refreshController = null;
      clearMessageDismissTimer();
      clearSettingsSaveFeedbackTimer();
      unlisteners.forEach((unlisten) => unlisten());
    };
  });
</script>

<div class="window-frame">
  <TitleBar />
  <CustomScrollbars />
  <div class="window-content">
{#if startupRecoveryStatus?.state !== "ready"}
  <main class="startup-recovery-shell">
    <section class="startup-recovery-panel" aria-live="polite">
      <AlertCircle size={28} />
      {#if startupRecoveryStatus === null || startupRecoveryStatus.state === "recovering"}
        <h1>正在检查启动状态</h1>
        <p>正在确认本地任务与数据状态，请稍候。</p>
      {:else if startupRecoveryStatus.state === "blocked"}
        <h1>需要确认启动恢复方式</h1>
        <p>上次退出留下了无法自动判定的任务状态。请选择如何处理尚未开始的排队任务。</p>
        <div class="startup-recovery-actions">
          <button
            class="primary"
            type="button"
            disabled={startupRecoveryBusy}
            on:click={() => resolveStartupRecovery("preserveQueued")}
          >
            保留排队任务并恢复
          </button>
          <button
            class="danger"
            type="button"
            disabled={startupRecoveryBusy}
            on:click={() => resolveStartupRecovery("cancelQueued")}
          >
            取消排队任务并恢复
          </button>
        </div>
      {:else}
        <h1>应用启动未完成</h1>
        <p>启动检查失败。错误代码：{startupRecoveryStatus.errorCode ?? "startup_not_ready"}</p>
        {#if startupRecoveryStatus.canRetry}
          <button
            class="primary"
            type="button"
            disabled={startupRecoveryBusy}
            on:click={() => resolveStartupRecovery("preserveQueued")}
          >
            重试启动
          </button>
        {/if}
      {/if}
      {#if errorMessage}
        <div class="startup-recovery-error">{errorMessage}</div>
      {/if}
    </section>
  </main>
{:else}
<main class="app-shell">
  {#if onboardingDraft}
    <OnboardingWorkspace step={onboardingDraft.step} {appIconUrl}>
        {#if errorMessage}
          <div class="onboarding-message error-line" role="alert">
            <AlertCircle size={16} />
            <span>{errorMessage}</span>
          </div>
        {/if}

        {#if onboardingDraft.step === 0}
          <div class="onboarding-content">
            <p>每个抖音账号使用独立的浏览器资料和本地数据。你也可以先跳过，稍后在账号菜单中添加。</p>
            <div class="setting-row">
              <span>登录浏览器</span>
              <SelectField ariaLabel="登录浏览器" bind:value={onboardingBrowserPreference} disabled={onboardingBusy || accountLoginBusy} options={[{ value: "auto", label: "自动选择" }, { value: "edge", label: "Microsoft Edge" }, { value: "chrome", label: "Google Chrome" }]} />
            </div>
            {#if activeAccount}
              <div class="onboarding-account-ready">
                <Users size={18} />
                <div><strong>{activeAccount.nickname ?? activeAccount.uid ?? `账号 ${activeAccount.id}`}</strong><small>账号已就绪</small></div>
              </div>
            {/if}
          </div>
          <footer class="onboarding-actions">
            {#if !activeAccount}
              <button type="button" disabled={onboardingBusy || accountLoginBusy} on:click={() => submitOnboarding(true)}>跳过账号</button>
            {:else}
              <span></span>
            {/if}
            {#if activeAccount}
              <button class="primary" type="button" disabled={onboardingBusy || accountLoginBusy} on:click={() => submitOnboarding(false)}>下一步</button>
            {:else}
              <button class="primary" type="button" disabled={onboardingBusy || accountLoginBusy} on:click={addAccount}><LogIn size={16} />{accountLoginBusy ? "登录中" : "一键登录"}</button>
            {/if}
          </footer>
        {:else if onboardingDraft.step === 1}
          <div class="onboarding-content">
            <div class="source-config-row">
              <div><strong>观看历史</strong><small>当前可用</small></div>
              <div class="segmented">
                <button class:active={onboardingDraft.accountDefaults.history.range.kind === "days"} type="button" on:click={() => updateOnboardingHistoryRange("days")}>最近</button>
                <button class:active={onboardingDraft.accountDefaults.history.range.kind === "bottom"} type="button" on:click={() => updateOnboardingHistoryRange("bottom")}>到底端</button>
              </div>
              {#if onboardingDraft.accountDefaults.history.range.kind === "days"}
                <label class="days-input"><input aria-label="引导观看历史天数" type="number" min="1" max="365" value={onboardingDraft.accountDefaults.history.range.days} on:change={(event) => updateOnboardingHistoryDays((event.currentTarget as HTMLInputElement).value)} /><span>天</span></label>
              {/if}
            </div>
            {#each [
              { source: "likes" as const, label: "喜欢", capability: likesCapability, archive: onboardingDraft.accountDefaults.likes },
              { source: "favorites" as const, label: "收藏", capability: favoritesCapability, archive: onboardingDraft.accountDefaults.favorites.archive },
            ] as item}
              <div class:unavailable={!item.capability?.available} class="source-config-row">
                <div><strong>{item.label}</strong><small>{item.capability?.available ? "当前可用" : item.capability?.unavailableReason === "collector_not_implemented" ? "采集器尚未实现" : "当前不可用"}</small></div>
                <SelectField
                  ariaLabel={`引导${item.label}同步范围`}
                  disabled={!item.capability?.available}
                  value={item.archive.range.kind}
                  on:change={(event) => updateOnboardingArchiveRange(item.source, event.detail.value as ArchiveListRange["kind"])}
                  options={[{ value: "all", label: "全部记录" }, { value: "recentItems", label: "最近若干条" }, { value: "recordLimit", label: "记录上限" }, { value: "recentDays", label: "最近天数", disabled: !item.capability?.supportedRanges.includes("recentDays") }]}
                />
                {#if item.archive.range.kind !== "all"}
                  <label class="interval-input">
                    <input
                      aria-label={`引导${item.label}${item.archive.range.kind === "recentItems" ? "最近条数" : item.archive.range.kind === "recordLimit" ? "记录上限" : "最近天数"}`}
                      type="number"
                      min="1"
                      max={item.archive.range.kind === "recentItems" ? 5000 : item.archive.range.kind === "recordLimit" ? 50000 : 365}
                      value={item.archive.range.kind === "recentItems" ? item.archive.range.count : item.archive.range.kind === "recordLimit" ? item.archive.range.limit : item.archive.range.days}
                      disabled={!item.capability?.available}
                      on:change={(event) => updateOnboardingArchiveRangeValue(item.source, (event.currentTarget as HTMLInputElement).value)}
                    />
                    <span>{item.archive.range.kind === "recentDays" ? "天" : "条"}</span>
                  </label>
                {/if}
              </div>
            {/each}
            <div class="onboarding-schedule-list">
              {#each [
                { source: "history", label: "观看历史" },
                { source: "likes", label: "喜欢" },
                { source: "favorites", label: "收藏" },
              ] as item}
                <div class="schedule-row">
                  <label class="toggle-row">
                    <input
                      aria-label={`${item.label}自动同步`}
                      type="checkbox"
                      checked={item.source === "history" ? onboardingDraft.accountDefaults.history.schedule.enabled : item.source === "likes" ? onboardingDraft.accountDefaults.likes.schedule.enabled : onboardingDraft.accountDefaults.favorites.archive.schedule.enabled}
                      disabled={!capabilityBySource[item.source as SyncSource]?.available}
                      on:change={(event) => updateOnboardingSchedule(item.source as "history" | "likes" | "favorites", (event.currentTarget as HTMLInputElement).checked)}
                    />
                    <span>{item.label}自动同步</span>
                  </label>
                  <label class="interval-input"><input aria-label={`${item.label}同步间隔`} type="number" min="15" max="43200" value={item.source === "history" ? onboardingDraft.accountDefaults.history.schedule.intervalMinutes : item.source === "likes" ? onboardingDraft.accountDefaults.likes.schedule.intervalMinutes : onboardingDraft.accountDefaults.favorites.archive.schedule.intervalMinutes} disabled={!capabilityBySource[item.source as SyncSource]?.available} on:change={(event) => updateOnboardingSchedule(item.source as "history" | "likes" | "favorites", undefined, Number((event.currentTarget as HTMLInputElement).value))} /><span>分钟</span></label>
                </div>
              {/each}
              <label class="setting-row"><span>开机后首次同步延迟</span><div class="interval-input"><input aria-label="开机同步延迟" type="number" min="0" max="120" bind:value={onboardingDraft.startupSyncDelayMinutes} /><span>分钟</span></div></label>
            </div>
          </div>
          <footer class="onboarding-actions">
            <button type="button" disabled={onboardingBusy} on:click={goBackOnboarding}><ChevronLeft size={16} />上一步</button>
            <button class="primary" type="button" disabled={onboardingBusy} on:click={() => submitOnboarding()}>下一步</button>
          </footer>
        {:else if onboardingDraft.step === 2}
          <div class="onboarding-content">
            <div class="onboarding-data-root">
              <label class="data-root-field">
                <span>本地数据位置</span>
                <input aria-label="引导当前数据目录" readonly value={onboardingDraft.dataRoot} />
              </label>
              <button class="secondary" type="button" disabled={dataRootChoosing || dataRootMigrating} on:click={chooseNewDataRoot}>
                <FolderOpen size={15} />{dataRootChoosing ? "选择中" : "选择数据目录"}
              </button>
              {#if dataRootValidation?.adjustedToInstallData}<p class="data-root-hint">已自动使用安装目录下的 Data 文件夹</p>{/if}
              {#if dataRootValidation}<button class="ghost data-root-cancel" type="button" disabled={dataRootMigrating} on:click={cancelDataRootSelection}>恢复默认目录</button>{/if}
            </div>
            <div class="desktop-choice-list">
              <label><input aria-label="关闭时退出应用" type="radio" name="onboarding-close" value="exit" bind:group={onboardingDraft.closeBehavior} />关闭时退出应用</label>
              <label class:disabled={!onboardingDraft.trayAvailable}><input aria-label="关闭时隐藏到托盘" type="radio" name="onboarding-close" value="tray" bind:group={onboardingDraft.closeBehavior} disabled={!onboardingDraft.trayAvailable} />关闭时隐藏到托盘</label>
            </div>
            {#if !onboardingDraft.trayAvailable}<p class="setting-warning">系统托盘当前不可用</p>{/if}
            <label class="toggle-row"><input aria-label="开机自动启动" type="checkbox" bind:checked={onboardingDraft.launchAtStartup} /><span>开机自动启动</span></label>
            <label class="toggle-row"><input aria-label="开机时静默启动" type="checkbox" bind:checked={onboardingDraft.silentAutostart} disabled={!onboardingDraft.launchAtStartup} /><span>开机时静默进入后台</span></label>
          </div>
          <footer class="onboarding-actions">
            <button type="button" disabled={onboardingBusy || dataRootMigrating} on:click={goBackOnboarding}><ChevronLeft size={16} />上一步</button>
            <button class="primary" type="button" disabled={onboardingBusy || dataRootMigrating} on:click={() => submitOnboarding()}>下一步</button>
          </footer>
        {:else}
          <div class="onboarding-content">
            <div class="onboarding-review">
              <h3>设置摘要</h3>
              <dl>
                <div><dt>账号</dt><dd>{activeAccount?.nickname ?? activeAccount?.uid ?? "暂未添加"}</dd></div>
                <div><dt>同步来源</dt><dd>{[onboardingDraft.accountDefaults.history.schedule.enabled, onboardingDraft.accountDefaults.likes.schedule.enabled, onboardingDraft.accountDefaults.favorites.archive.schedule.enabled].filter(Boolean).length} 个已启用自动同步</dd></div>
                <div><dt>数据位置</dt><dd title={onboardingDraft.selectedDataRoot ?? onboardingDraft.dataRoot}>{onboardingDraft.selectedDataRoot ?? onboardingDraft.dataRoot}</dd></div>
                <div><dt>关闭行为</dt><dd>{onboardingDraft.closeBehavior === "tray" ? "隐藏到托盘" : "退出应用"}</dd></div>
                <div><dt>开机启动</dt><dd>{onboardingDraft.launchAtStartup ? onboardingDraft.silentAutostart ? "静默启动" : "显示主窗口" : "关闭"}</dd></div>
              </dl>
            </div>
            <label class="toggle-row"><input aria-label="完成引导后立即同步" type="checkbox" bind:checked={onboardingDraft.syncAfterOnboarding} disabled={!activeAccount} /><span>完成引导后立即同步可用来源一次</span></label>
          </div>
          <footer class="onboarding-actions">
            <button type="button" disabled={onboardingBusy} on:click={goBackOnboarding}><ChevronLeft size={16} />上一步</button>
            <button class="primary" type="button" disabled={onboardingBusy} on:click={() => submitOnboarding()}>{onboardingBusy ? "保存中" : "完成设置"}</button>
          </footer>
        {/if}
    </OnboardingWorkspace>
  {/if}
  {#if !onboardingDraft}
  <AppShell collapsed={sidebarCollapsed}>
    <PrimarySidebar
      {activePage}
      {appIconUrl}
      collapsed={sidebarCollapsed}
      sourceAvailability={{
        history: capabilityBySource.history?.available ?? false,
        likes: capabilityBySource.likes?.available ?? false,
        favorites: capabilityBySource.favorites?.available ?? false,
      }}
      syncHealthText={syncing ? "正在同步" : schedulerState?.paused ? "自动同步已暂停" : "后台服务正常"}
      {syncing}
      onSelect={selectPage}
      onToggleCollapsed={() => (sidebarCollapsed = !sidebarCollapsed)}
    >
      {#snippet accountSwitcher()}
        <AccountSwitcher
          accounts={sortedAccounts}
          {activeAccount}
          open={accountMenuOpen}
          disabled={accountBusy || accountLoginBusy || syncing || dataRootMigrating}
          loginBusy={accountLoginBusy}
          canSyncCurrent={Boolean(activeAccount && canSyncAccount(activeAccount.status))}
          collapsed={sidebarCollapsed}
          onToggle={() => (accountMenuOpen = !accountMenuOpen)}
          onSelect={switchAccount}
          onSyncCurrent={() => {
            accountMenuOpen = false;
            void enqueueManualSync("currentAccountAvailableSources");
          }}
          onAdd={addAccount}
          onManage={openAccountManager}
        />
      {/snippet}
    </PrimarySidebar>
    <div class="workspace-main" data-scrollbar-label="页面内容" bind:this={workspaceMainElement} on:scroll={handleWorkspaceScroll}>
  <header class="topbar">
    <div class="page-heading">
      <h1>{pageTitle}</h1>
      <p>{pageDescription}</p>
    </div>
    <div class="actions">
      {#if activePage === "history"}
      <div class="sync-settings" aria-label="同步范围">
        <div class="segmented">
          <button
            class:active={syncMode === "days"}
            type="button"
            aria-pressed={syncMode === "days"}
            disabled={accountSettingsDisabled}
            on:click={() => setSyncMode("days")}
          >
            最近
          </button>
          <button
            class:active={syncMode === "bottom"}
            type="button"
            aria-pressed={syncMode === "bottom"}
            disabled={accountSettingsDisabled}
            on:click={() => setSyncMode("bottom")}
          >
            到底端
          </button>
        </div>
        {#if syncMode === "days"}
          <label class="days-input">
            <input
              aria-label="观看历史同步范围天数"
              type="number"
              min="1"
              max="365"
              step="1"
              value={syncDays}
              disabled={accountSettingsDisabled}
              on:change={(event) => updateSyncDays((event.currentTarget as HTMLInputElement).value)}
            />
            <span>天</span>
          </label>
        {/if}
      </div>
      {/if}
      {#if syncing}
        <button
          class="danger"
          type="button"
          disabled={!syncStatus.canCancel || cancelRequested}
          on:click={cancelRunningSync}
        >
          <CircleStop size={16} />
          {cancelRequested ? "取消中" : "取消同步"}
        </button>
      {:else if activePage !== "accounts"}
        {#if activeAccount && canReloginAccount(activeAccount.status)}
          <button class="secondary" type="button" disabled={accountBusy || accountLoginBusy || dataRootMigrating} on:click={() => resumeLogin(activeAccount)}>
            <LogIn size={16} />
            {accountRecoveryButtonLabel(accountLoginBusy)}
          </button>
        {:else}
          <button
            class="primary"
            type="button"
            disabled={!activeAccount || !canSyncAccount(activeAccount.status) || (contentPageActive && !currentSourceAvailable) || dataRootMigrating}
            on:click={() => enqueueManualSync(contentPageActive ? "one" : "currentAccountAvailableSources")}
          >
            <RefreshCw size={16} />
            {contentPageActive ? "同步当前来源" : "同步当前账号"}
          </button>
        {/if}
      {/if}
    </div>
  </header>

  {#if !activeAccount && activePage !== "accounts"}
    <section class="account-empty-state">
      <Users size={34} />
      <h2>先添加一个抖音账号</h2>
      <p>每个账号使用独立浏览器资料与本地记录，切换时互不混用。</p>
      <button class="primary" type="button" disabled={accountLoginBusy || dataRootMigrating} on:click={addAccount}>
        <UserPlus size={16} />{accountLoginButtonLabel(accountLoginBusy)}
      </button>
    </section>
  {/if}

  {#if activeAccount && contentPageActive}
  <section class="status-strip" aria-live="polite">
    <div class="metric">
      <Database size={18} />
      <span>本地记录</span>
      <strong>{total}</strong>
    </div>
    <div class="metric">
      <Clock size={18} />
      <span>{syncStatus.status === "running" && phaseText ? phaseText : statusText}</span>
      <strong>{syncing ? syncStatus.recordCount : syncStatus.insertedWatchRecords}</strong>
    </div>
    {#if syncing}
      <div class="metric subtle">
        <span>耗时</span>
        <strong>{formatElapsed(syncStatus.elapsedMs)}</strong>
      </div>
      {#if syncStatus.retryCount > 0}
        <div class="metric subtle">
          <span>重试</span>
          <strong>{syncStatus.retryCount}</strong>
        </div>
      {/if}
    {/if}
    {#if syncStatus.finishedAt}
      <div class="metric subtle">
        <span>完成时间</span>
        <strong>{formatDate(syncStatus.finishedAt)}</strong>
      </div>
    {/if}
  </section>
  {/if}

  {#if messageBanner && messageBannerKey !== dismissedMessageKey}
    <section class={messageBanner.kind === "error" ? "error-line message-line" : "notice-line message-line"} aria-live={messageBanner.kind === "error" ? "assertive" : "polite"}>
      {#if messageBanner.kind === "error"}<AlertCircle size={16} />{/if}
      <span>{messageBanner.text}</span>
      <button class="icon-button message-dismiss" type="button" title="关闭消息提示" aria-label="关闭消息提示" on:click={dismissMessageBanner}><X size={15} /></button>
    </section>
  {/if}

  {#if activePage === "settings"}
    <SettingsWorkspace saveState={settingsSaveState} onSaveFeedbackHidden={hideSettingsSaveFeedback}>
        <div class="settings-section theme-settings" data-settings-category="general" data-settings-panel="appearance" data-settings-panel-label="外观" data-settings-search-target>
          <div class="settings-section-heading">
            <div><h3>外观</h3><p>主题会立即应用到当前窗口。</p></div>
          </div>
          <div class="segmented theme-segmented" aria-label="主题">
            {#each [
              { value: "system" as ThemeMode, label: "跟随系统" },
              { value: "light" as ThemeMode, label: "浅色" },
              { value: "dark" as ThemeMode, label: "深色" },
            ] as option}
              <button class:active={themeMode === option.value} type="button" aria-pressed={themeMode === option.value} disabled={globalSettingsDisabled} on:click={() => setThemeMode(option.value)}>{option.label}</button>
            {/each}
          </div>
        </div>
        {#if appSettings}
          <div class="settings-section workspace-sync-settings" data-settings-category="accounts" data-settings-panel="sync-policy" data-settings-panel-label="默认同步策略" data-settings-search-target>
            <div class="settings-section-heading">
              <div><h3>默认同步策略</h3><p>单账号同步使用当前账号；聚合同步按所选账号依次进入串行队列。</p></div>
            </div>
            <div class="segmented" aria-label="设置默认同步方式">
              <button
                type="button"
                class:active={appSettings.workspace.sync.mode === "singleAccount"}
                aria-pressed={appSettings.workspace.sync.mode === "singleAccount"}
                disabled={globalSettingsDisabled}
                on:click={() => updateWorkspaceSyncPreference("singleAccount", appSettings!.workspace.sync.accountScope)}
              >单账号同步</button>
              <button
                type="button"
                class:active={appSettings.workspace.sync.mode === "aggregate"}
                aria-pressed={appSettings.workspace.sync.mode === "aggregate"}
                disabled={globalSettingsDisabled}
                on:click={() => updateWorkspaceSyncPreference("aggregate", appSettings!.workspace.sync.accountScope)}
              >多账号聚合同步</button>
            </div>
            {#if appSettings.workspace.sync.mode === "aggregate"}
              <div class="segmented compact" aria-label="设置聚合同步范围">
                <button type="button" class:active={appSettings.workspace.sync.accountScope.kind === "all"} aria-pressed={appSettings.workspace.sync.accountScope.kind === "all"} disabled={globalSettingsDisabled} on:click={() => setWorkspaceSyncScopeKind("all")}>全部活跃账号</button>
                <button type="button" class:active={appSettings.workspace.sync.accountScope.kind === "selected"} aria-pressed={appSettings.workspace.sync.accountScope.kind === "selected"} disabled={globalSettingsDisabled} on:click={() => setWorkspaceSyncScopeKind("selected")}>自定义账号</button>
              </div>
              {#if appSettings.workspace.sync.accountScope.kind === "selected"}
                <div class="account-scope-list settings-account-scope-list">
                  {#each accounts.filter((account) => account.status === "active") as account}
                    <label>
                      <input
                        type="checkbox"
                        aria-label={`设置聚合同步${account.nickname ?? account.uid ?? `账号 ${account.id}`}`}
                        checked={Boolean(account.accountKey && appSettings.workspace.sync.accountScope.kind === "selected" && appSettings.workspace.sync.accountScope.accountKeys.includes(account.accountKey))}
                        disabled={globalSettingsDisabled || !account.accountKey}
                        on:change={(event) => toggleWorkspaceSyncAccount(account, (event.currentTarget as HTMLInputElement).checked)}
                      />
                      <span>{account.nickname ?? account.uid ?? `账号 ${account.id}`}</span>
                    </label>
                  {/each}
                </div>
                {#if configuredSyncScopeInvalid}<p class="account-scope-error">同步账号范围已失效，请重新选择</p>{/if}
              {/if}
            {/if}
          </div>
        {/if}
        <div class="settings-grid settings-section" data-settings-category="accounts" data-settings-panel="sync-parameters" data-settings-panel-label="同步参数" data-settings-search-target>
          <label class="setting-row">
            <span>滚动速率</span>
            <input
              type="number"
              min="120"
              step="60"
              value={scrollSpeed}
              disabled={accountSettingsDisabled}
              on:change={(event) => updateScrollSpeed((event.currentTarget as HTMLInputElement).value)}
            />
          </label>

          <label class="toggle-row">
            <input
              type="checkbox"
              checked={hideBrowser}
              disabled={accountSettingsDisabled}
              on:change={(event) => setHideBrowser((event.currentTarget as HTMLInputElement).checked)}
            />
            <span>同步时隐藏浏览器</span>
          </label>
          {#if !activeAccount}
            <p class="setting-note">添加账号后可调整该账号的同步设置。</p>
          {/if}
        </div>

        <div class="settings-section scheduler-settings" data-settings-category="accounts" data-settings-panel="automatic-sync" data-settings-panel-label="自动同步" data-settings-search-target>
          <div class="settings-section-heading">
            <div>
              <h3>自动同步</h3>
              <p>每个来源独立设置范围和间隔，时间状态以后台调度器为准。</p>
            </div>
            <button
              class="secondary"
              type="button"
              disabled={globalSettingsDisabled}
              on:click={toggleSchedulerPaused}
            >
              {schedulerState?.paused ? "恢复自动同步" : "暂停自动同步"}
            </button>
          </div>

          {#each [
            { source: "history" as const, label: "观看历史" },
            { source: "likes" as const, label: "喜欢" },
            { source: "favorites" as const, label: "收藏" },
          ] as item}
            {@const capability = capabilityBySource[item.source] ?? null}
            {@const state = schedulerSourceStateBySource[item.source] ?? null}
            {@const schedule = item.source === "history" ? accountSettings?.history.schedule : item.source === "likes" ? accountSettings?.likes.schedule : accountSettings?.favorites.archive.schedule}
            <div class="source-config-row">
              <div class="source-config-heading">
                <div>
                  <strong>{item.label}</strong>
                  <small>{capability?.available ? "可用" : capability?.unavailableReason === "collector_not_implemented" ? "采集器尚未实现" : "当前不可用"}</small>
                </div>
                <label class:disabled={!capability?.available}>
                  <input
                    aria-label={`${item.label}自动同步`}
                    type="checkbox"
                    checked={schedule?.enabled ?? false}
                    disabled={accountSettingsDisabled || !capability?.available}
                    on:change={(event) => updateSourceSchedule(item.source, (event.currentTarget as HTMLInputElement).checked)}
                  />
                  自动同步
                </label>
              </div>

              {#if item.source === "history"}
                <div class="schedule-row">
                  <div class="segmented source-range-segmented">
                    <button class:active={syncMode === "days"} type="button" aria-pressed={syncMode === "days"} disabled={accountSettingsDisabled || !capability?.available} on:click={() => setSyncMode("days")}>最近天数</button>
                    <button class:active={syncMode === "bottom"} type="button" aria-pressed={syncMode === "bottom"} disabled={accountSettingsDisabled || !capability?.available} on:click={() => setSyncMode("bottom")}>同步到底</button>
                  </div>
                  {#if syncMode === "days"}
                    <label class="interval-input"><input aria-label="观看历史同步天数" type="number" min="1" max="365" value={syncDays} disabled={accountSettingsDisabled || !capability?.available} on:change={(event) => updateSyncDays((event.currentTarget as HTMLInputElement).value)} /><span>天</span></label>
                  {/if}
                  <label class="interval-input"><input aria-label="观看历史同步间隔" type="number" min="15" max="43200" value={schedule?.intervalMinutes ?? 1440} disabled={accountSettingsDisabled || !capability?.available} on:change={(event) => updateSourceSchedule(item.source, undefined, Number((event.currentTarget as HTMLInputElement).value))} /><span>分钟</span></label>
                </div>
              {:else}
                {@const archive = item.source === "likes" ? accountSettings?.likes : accountSettings?.favorites.archive}
                <div class="schedule-row">
                  <SelectField
                    ariaLabel={`${item.label}同步范围`}
                    disabled={accountSettingsDisabled || !capability?.available}
                    value={archive?.range.kind ?? "all"}
                    on:change={(event) => updateArchiveRange(item.source, event.detail.value as ArchiveListRange["kind"])}
                    options={[{ value: "all", label: "全部记录" }, { value: "recentItems", label: "最近若干条" }, { value: "recordLimit", label: "记录上限" }, { value: "recentDays", label: "最近天数", disabled: !capability?.supportedRanges.includes("recentDays") }]}
                  />
                  {#if archive && archive.range.kind !== "all"}
                    <label class="interval-input">
                      <input
                        aria-label={`${item.label}${archive.range.kind === "recentItems" ? "最近条数" : archive.range.kind === "recordLimit" ? "记录上限" : "最近天数"}`}
                        type="number"
                        min="1"
                        max={archive.range.kind === "recentItems" ? 5000 : archive.range.kind === "recordLimit" ? 50000 : 365}
                        value={archive.range.kind === "recentItems" ? archive.range.count : archive.range.kind === "recordLimit" ? archive.range.limit : archive.range.days}
                        disabled={accountSettingsDisabled || !capability?.available}
                        on:change={(event) => updateArchiveRangeValue(item.source, (event.currentTarget as HTMLInputElement).value)}
                      />
                      <span>{archive.range.kind === "recentDays" ? "天" : "条"}</span>
                    </label>
                  {/if}
                  <label class="interval-input"><input aria-label={`${item.label}同步间隔`} type="number" min="15" max="43200" value={schedule?.intervalMinutes ?? 1440} disabled={accountSettingsDisabled || !capability?.available} on:change={(event) => updateSourceSchedule(item.source, undefined, Number((event.currentTarget as HTMLInputElement).value))} /><span>分钟</span></label>
                </div>
              {/if}

              <div class="source-runtime-state">
                <span>上次：{syncStatusLabel(state?.lastStatus ?? null)}{state?.lastFinishedAt ? ` · ${formatDate(state.lastFinishedAt)}` : ""}</span>
                <span>下次：{state?.nextDueAt ? formatDate(state.nextDueAt) : schedule?.enabled ? "等待调度" : "未启用"}</span>
              </div>
              {#if state?.aggregateError || state?.message}
                <p class="setting-warning">{state.aggregateError ?? state.message}</p>
              {/if}
            </div>
          {/each}

          <label class="setting-row">
            <span>开机后首次同步延迟</span>
            <div class="interval-input">
              <input aria-label="开机后首次同步延迟" type="number" min="0" max="120" value={startupSyncDelayMinutes} disabled={globalSettingsDisabled} on:change={(event) => updateStartupSyncDelay((event.currentTarget as HTMLInputElement).value)} />
              <span>分钟</span>
            </div>
          </label>
        </div>

        <div class="settings-section" data-settings-category="general" data-settings-panel="desktop-behavior" data-settings-panel-label="桌面行为" data-settings-search-target>
          <h3>桌面行为</h3>
          <div class="desktop-behavior-grid">
            <div class="segmented close-behavior-segmented">
              <button class:active={closeBehavior === "exit"} type="button" aria-pressed={closeBehavior === "exit"} disabled={globalSettingsDisabled} on:click={() => updateCloseBehavior("exit")}>关闭时退出</button>
              <button class:active={closeBehavior === "tray"} type="button" aria-pressed={closeBehavior === "tray"} disabled={globalSettingsDisabled || !desktopState?.trayAvailable} on:click={() => updateCloseBehavior("tray")}>隐藏到托盘</button>
            </div>
            {#if desktopState && !desktopState.trayAvailable}<p class="setting-warning">系统托盘当前不可用</p>{/if}
            <label class="toggle-row"><input aria-label="开机自动启动" type="checkbox" checked={desktopState?.device.launchAtStartup ?? false} disabled={globalSettingsDisabled || !desktopState} on:change={(event) => updateAutostart((event.currentTarget as HTMLInputElement).checked)} /><span>开机自动启动</span></label>
            <label class="toggle-row"><input aria-label="开机时静默启动" type="checkbox" checked={desktopState?.device.silentAutostart ?? false} disabled={globalSettingsDisabled || !desktopState?.device.launchAtStartup} on:change={(event) => updateAutostart(true, (event.currentTarget as HTMLInputElement).checked)} /><span>开机时静默进入后台</span></label>
          </div>
        </div>

        <div class="settings-section" data-settings-category="accounts" data-settings-panel="browser" data-settings-panel-label="浏览器" data-settings-search-target>
          <h3>当前账号浏览器</h3>
          <div class="segmented browser-segmented">
            {#each [
              { value: "auto" as BrowserPreference, label: "自动" },
              { value: "edge" as BrowserPreference, label: "Edge" },
              { value: "chrome" as BrowserPreference, label: "Chrome" },
            ] as option}
              <button class:active={browserPreference === option.value} type="button" aria-pressed={browserPreference === option.value} disabled={accountSettingsDisabled || !activeAccount} on:click={() => updateBrowserPreference(option.value)}>{option.label}</button>
            {/each}
          </div>
          {#if browserReloginRequired}<p class="setting-warning">重新登录后生效</p>{/if}
        </div>

        {#if remoteFoldersSupported && activeAccount}
          <div class="settings-section" data-settings-category="accounts" data-settings-panel="remote-folders" data-settings-panel-label="远端收藏夹" data-settings-search-target>
            <div class="settings-section-heading">
              <div>
                <h3>远端收藏夹</h3>
                <p>分页读取收藏夹调度配置，不在前端推算到期时间。</p>
                {#if !accountSettings?.favorites.archive.schedule.enabled}
                  <p class="setting-note">仍会同步已知收藏夹，但不会自动发现新收藏夹</p>
                {/if}
              </div>
              <button class="secondary" type="button" disabled={schedulerInstancesLoading} on:click={() => loadSchedulerInstances(true)}>查看远端收藏夹设置</button>
            </div>
            {#if schedulerInstances}
              <div class="scheduler-instance-list">
                {#each schedulerInstances.instances as instance}
                  <div><strong>{instance.name}</strong><span>{instance.schedule.enabled ? `每 ${instance.schedule.intervalMinutes} 分钟` : "未启用"}</span></div>
                {/each}
              </div>
              {#if schedulerInstances.hasMore}
                <button type="button" disabled={schedulerInstancesLoading} on:click={() => loadSchedulerInstances(false)}>加载更多收藏夹</button>
              {/if}
            {/if}
          </div>
        {/if}

        <div class="settings-section" data-settings-category="content" data-settings-panel="like-format" data-settings-panel-label="点赞量格式" data-settings-search-target>
          <h3>点赞量格式</h3>
          <div class="segmented setting-segmented">
            {#each LIKE_COUNT_FORMAT_OPTIONS as option}
              <button
                class:active={likeCountFormat === option.value}
                type="button"
                aria-pressed={likeCountFormat === option.value}
                disabled={globalSettingsDisabled}
                on:click={() => setLikeCountFormat(option.value)}
              >{option.label}</button>
            {/each}
          </div>
        </div>

        <div class="settings-section" data-settings-category="content" data-settings-panel="detail-fields" data-settings-panel-label="详情字段" data-settings-search-target>
          <h3>详情字段</h3>
          <div class="field-options">
            <label><input type="checkbox" checked={detailFields.author} disabled={globalSettingsDisabled} on:change={(event) => toggleDetailField("author", (event.currentTarget as HTMLInputElement).checked)} />作者</label>
            <label><input type="checkbox" checked={detailFields.watchDate} disabled={globalSettingsDisabled} on:change={(event) => toggleDetailField("watchDate", (event.currentTarget as HTMLInputElement).checked)} />观看日期</label>
            <label><input type="checkbox" checked={detailFields.likeCount} disabled={globalSettingsDisabled} on:change={(event) => toggleDetailField("likeCount", (event.currentTarget as HTMLInputElement).checked)} />点赞量</label>
            <label><input type="checkbox" checked={detailFields.watchProgress} disabled={globalSettingsDisabled} on:change={(event) => toggleDetailField("watchProgress", (event.currentTarget as HTMLInputElement).checked)} />观看进度</label>
          </div>
        </div>

        <div class="settings-section download-settings" data-settings-category="downloads" data-settings-panel="video-download" data-settings-panel-label="视频下载" data-settings-search-target>
          <div class="settings-section-heading">
            <div>
              <h3>视频下载</h3>
              <p>手动下载观看历史中的原始视频、图文和实况资源；同步不会自动创建下载任务。</p>
            </div>
          </div>
          <div class="download-settings-grid">
            <label class="setting-row download-directory-setting">
              <span>默认目录</span>
              <input
                aria-label="默认下载目录"
                readonly
                value={downloadSettings.defaultDirectory}
                placeholder="请选择默认下载目录"
              />
              <button class="secondary" type="button" aria-label="选择默认下载目录" on:click={chooseDefaultDownloadDirectory}><FolderOpen size={15} />选择目录</button>
            </label>
            <div class="setting-row">
              <span>默认质量</span>
              <SelectField
                ariaLabel="默认下载质量"
                value={downloadSettings.defaultQuality}
                on:change={(event) => updateDownloadSettings({ defaultQuality: event.detail.value as DownloadSettings["defaultQuality"] })}
                options={[{ value: "highest", label: "最高质量" }, { value: "balanced", label: "均衡" }, { value: "space_saver", label: "节省空间" }]}
              />
            </div>
            <label class="toggle-row">
              <input
                aria-label="每次下载前选择目录"
                type="checkbox"
                checked={downloadSettings.askDirectoryEachJob}
                on:change={(event) => updateDownloadSettings({ askDirectoryEachJob: (event.currentTarget as HTMLInputElement).checked })}
              />
              <span>每次下载前选择目录</span>
            </label>
            <label class="toggle-row">
              <input
                aria-label="每次下载前选择质量"
                type="checkbox"
                checked={downloadSettings.askQualityEachJob}
                on:change={(event) => updateDownloadSettings({ askQualityEachJob: (event.currentTarget as HTMLInputElement).checked })}
              />
              <span>每次下载前选择质量</span>
            </label>
          </div>
        </div>

        <div class="settings-section cover-cache-settings" data-settings-category="storage" data-settings-panel="cover-cache" data-settings-panel-label="封面缓存" data-settings-search-target>
          <div class="settings-section-heading">
            <div>
              <h3>封面缓存</h3>
              <p>{coverCacheSettings.enabled ? "优先读取本地高清封面，失败时自动回退网络。" : "默认联网加载，不在浏览时写入缓存。"}</p>
            </div>
            <button class="secondary" type="button" on:click={openCoverCacheManager}>管理封面缓存</button>
          </div>
          <label class="toggle-row">
            <input
              aria-label="启用本地封面缓存"
              type="checkbox"
              checked={coverCacheSettings.enabled}
              disabled={globalSettingsDisabled}
              on:click|preventDefault={() => requestCoverCacheEnabled(!coverCacheSettings.enabled)}
            />
            <span>启用本地封面缓存</span>
          </label>

          {#if coverCacheSettings.enabled}
            <div class="cover-cache-policy-grid">
              <label class="setting-row"><span>容量上限</span><div class="interval-input"><input aria-label="封面缓存容量上限" type="number" min="1" value={coverCacheSettings.capacityLimitMib} disabled={globalSettingsDisabled} on:change={(event) => updateCoverCacheSettings({ capacityLimitMib: clampInteger(Number((event.currentTarget as HTMLInputElement).value), 1, 1048576, 2048) })} /><span>MiB</span></div></label>
              <label class="toggle-row"><input aria-label="按容量自动清理" type="checkbox" checked={coverCacheSettings.cleanupByCapacityEnabled} disabled={globalSettingsDisabled} on:change={(event) => updateCoverCacheSettings({ cleanupByCapacityEnabled: (event.currentTarget as HTMLInputElement).checked })} /><span>达到容量上限时自动清理</span></label>
              <label class="toggle-row"><input aria-label="按时间自动清理" type="checkbox" checked={coverCacheSettings.cleanupByAgeEnabled} disabled={globalSettingsDisabled} on:change={(event) => updateCoverCacheSettings({ cleanupByAgeEnabled: (event.currentTarget as HTMLInputElement).checked })} /><span>按最后使用时间自动清理</span></label>
              <label class="setting-row"><span>最长保留时间</span><div class="interval-input"><input aria-label="封面缓存最长保留天数" type="number" min="1" value={coverCacheSettings.maxAgeDays} disabled={globalSettingsDisabled || !coverCacheSettings.cleanupByAgeEnabled} on:change={(event) => updateCoverCacheSettings({ maxAgeDays: clampInteger(Number((event.currentTarget as HTMLInputElement).value), 1, 36500, 90) })} /><span>天</span></div></label>
              <div class="segmented cache-list-mode" aria-label="封面缓存名单模式">
                <button class:active={coverCacheSettings.cleanupListMode === "keepListed"} type="button" aria-pressed={coverCacheSettings.cleanupListMode === "keepListed"} disabled={globalSettingsDisabled} on:click={() => updateCoverCacheSettings({ cleanupListMode: "keepListed" as CoverCacheCleanupMode })}>仅保留列表内</button>
                <button class:active={coverCacheSettings.cleanupListMode === "cleanListed"} type="button" aria-pressed={coverCacheSettings.cleanupListMode === "cleanListed"} disabled={globalSettingsDisabled} on:click={() => updateCoverCacheSettings({ cleanupListMode: "cleanListed" as CoverCacheCleanupMode })}>仅清理列表内</button>
              </div>
            </div>
          {/if}

          {#if coverCacheTask}
            <div class="cover-cache-task">
              <div><strong>{coverCacheTaskLabel(coverCacheTask)}</strong><span>{coverCacheTask.completedItems} / {coverCacheTask.totalItems}{coverCacheTask.failedItems > 0 ? ` · 失败 ${coverCacheTask.failedItems}` : ""}</span></div>
              <progress max="100" value={coverCacheProgress(coverCacheTask)} aria-label="封面缓存任务进度"></progress>
              <div class="cover-cache-task-actions">
                {#if coverCacheTask.status === "running"}<button type="button" disabled={coverCacheTaskBusy} on:click={() => runCoverCacheTaskAction("pause")}>暂停</button>{/if}
                {#if coverCacheTask.status === "paused"}<button type="button" disabled={coverCacheTaskBusy} on:click={() => runCoverCacheTaskAction("resume")}>继续</button>{/if}
                {#if ["queued", "running", "paused"].includes(coverCacheTask.status)}<button class="danger" type="button" disabled={coverCacheTaskBusy} on:click={() => runCoverCacheTaskAction("cancel")}>取消任务</button>{/if}
                {#if ["failed", "cancelled"].includes(coverCacheTask.status)}<button type="button" disabled={coverCacheTaskBusy} on:click={() => runCoverCacheTaskAction("retry")}>失败重试</button>{/if}
                <button type="button" disabled={coverCacheTaskBusy} on:click={refreshCoverCacheTask}>刷新</button>
              </div>
            </div>
          {/if}
        </div>

        <div class="settings-section backup-settings" data-settings-category="backup" data-settings-panel="backup-restore" data-settings-panel-label="备份与恢复" data-settings-search-target>
          <div class="settings-section-heading">
            <div>
              <h3>备份与通知</h3>
              <p>备份仅包含数据库与设置，不包含封面、登录资料、日志或下载内容。</p>
            </div>
            <div class="backup-actions">
              <button
                class="secondary"
                type="button"
                disabled={globalSettingsDisabled || backupDirectoryChoosing || backupExporting || !desktopState}
                on:click={chooseNewBackupDirectory}
              >
                <FolderOpen size={15} />
                {backupDirectoryChoosing ? "选择中" : "选择备份目录"}
              </button>
              <button
                class="secondary"
                type="button"
                disabled={globalSettingsDisabled || backupDirectoryChoosing || backupExporting || backupRestoreBusy}
                on:click={() => (backupExportConfirmOpen = true)}
              >
                <Download size={15} />
                {backupExporting ? "导出中" : "导出备份"}
              </button>
              <button
                class="secondary"
                type="button"
                disabled={globalSettingsDisabled || backupDirectoryChoosing || backupExporting || backupRestoreBusy}
                on:click={chooseBackupForRestore}
              >
                <Upload size={15} />
                {backupRestoreBusy ? "处理中" : "导入备份"}
              </button>
            </div>
          </div>

          <label class="toggle-row">
            <input
              aria-label="启用自动备份"
              type="checkbox"
              checked={backupSettings.enabled}
              disabled={globalSettingsDisabled}
              on:change={(event) => updateBackupSettings({ enabled: (event.currentTarget as HTMLInputElement).checked })}
            />
            <span>启用自动备份</span>
          </label>

          <div class="backup-policy-grid">
            <label class="setting-row">
              <span>备份间隔</span>
              <div class="interval-input">
                <input aria-label="自动备份间隔" type="number" min="1" max="365" value={backupSettings.intervalDays} disabled={globalSettingsDisabled || !backupSettings.enabled} on:change={(event) => updateBackupSettings({ intervalDays: clampInteger(Number((event.currentTarget as HTMLInputElement).value), 1, 365, 7) })} />
                <span>天</span>
              </div>
            </label>
            <label class="setting-row">
              <span>执行时间</span>
              <input aria-label="自动备份执行时间" type="time" value={backupSettings.executionTime} disabled={globalSettingsDisabled || !backupSettings.enabled} on:change={(event) => updateBackupSettings({ executionTime: (event.currentTarget as HTMLInputElement).value })} />
            </label>
            <label class="setting-row">
              <span>保留份数</span>
              <div class="interval-input">
                <input aria-label="自动备份保留份数" type="number" min="1" max="100" value={backupSettings.retentionCount} disabled={globalSettingsDisabled || !backupSettings.enabled} on:change={(event) => updateBackupSettings({ retentionCount: clampInteger(Number((event.currentTarget as HTMLInputElement).value), 1, 100, 5) })} />
                <span>份</span>
              </div>
            </label>
          </div>

          <label class="data-root-field">
            <span>当前备份目录{desktopState?.device.backupDirectory ? "" : "（默认）"}</span>
            <input aria-label="当前备份目录" readonly value={currentBackupDirectory} />
          </label>

        </div>

        <div class="settings-section notification-settings" data-settings-category="notifications" data-settings-panel="notification-preferences" data-settings-panel-label="通知偏好" data-settings-search-target>
            <div class="settings-section-heading">
              <div><h3>通知</h3><p>选择需要显示的系统通知和应用内消息停留时间。</p></div>
            </div>
            <div class="field-options backup-notification-options">
              <label><input aria-label="登录失效通知" type="checkbox" checked={notificationSettings.loginExpired} disabled={globalSettingsDisabled} on:change={(event) => updateNotificationSettings({ loginExpired: (event.currentTarget as HTMLInputElement).checked })} />登录失效</label>
              <label><input aria-label="连续自动同步失败通知" type="checkbox" checked={notificationSettings.automaticFailureStreak} disabled={globalSettingsDisabled} on:change={(event) => updateNotificationSettings({ automaticFailureStreak: (event.currentTarget as HTMLInputElement).checked })} />连续自动同步失败</label>
              <label><input aria-label="备份失败通知" type="checkbox" checked={notificationSettings.backupFailure} disabled={globalSettingsDisabled} on:change={(event) => updateNotificationSettings({ backupFailure: (event.currentTarget as HTMLInputElement).checked })} />备份失败</label>
              <label><input aria-label="同步完成通知" type="checkbox" checked={notificationSettings.syncCompleted} disabled={globalSettingsDisabled} on:change={(event) => updateNotificationSettings({ syncCompleted: (event.currentTarget as HTMLInputElement).checked })} />同步完成</label>
              <label><input aria-label="备份完成通知" type="checkbox" checked={notificationSettings.backupCompleted} disabled={globalSettingsDisabled} on:change={(event) => updateNotificationSettings({ backupCompleted: (event.currentTarget as HTMLInputElement).checked })} />备份完成</label>
            </div>
            <label class="setting-row">
              <span>连续自动同步失败阈值</span>
              <div class="interval-input">
                <input aria-label="连续自动同步失败阈值" type="number" min="1" max="20" value={notificationSettings.automaticFailureThreshold} disabled={globalSettingsDisabled || !notificationSettings.automaticFailureStreak} on:change={(event) => updateNotificationSettings({ automaticFailureThreshold: clampInteger(Number((event.currentTarget as HTMLInputElement).value), 1, 20, 3) })} />
                <span>次</span>
              </div>
            </label>
            <label class="setting-row">
              <span>消息提示停留时间</span>
              <div class="interval-input">
                <input aria-label="消息提示停留时间" type="number" min="1" max="300" value={notificationSettings.messageDurationSeconds} disabled={globalSettingsDisabled} on:change={(event) => updateNotificationSettings({ messageDurationSeconds: clampInteger(Number((event.currentTarget as HTMLInputElement).value), 1, 300, 5) })} />
                <span>秒</span>
              </div>
            </label>
        </div>

        <div class="settings-section update-settings" data-settings-category="updates" data-settings-panel="app-update" data-settings-panel-label="应用更新" data-settings-search-target>
          <div class="settings-section-heading">
            <div>
              <h3>应用更新</h3>
              <p>自动下载完成后仍需由你确认安装，更新安装前会先保护本地数据。</p>
            </div>
          </div>
          <label class="toggle-row">
            <input
              aria-label="自动检查更新"
              type="checkbox"
              checked={updateSettings.automaticCheck}
              disabled={globalSettingsDisabled}
              on:change={(event) => updateUpdateSettings({ automaticCheck: (event.currentTarget as HTMLInputElement).checked })}
            />
            <span>启动后自动检查更新</span>
          </label>
          <label class="toggle-row">
            <input
              aria-label="自动下载更新"
              type="checkbox"
              checked={updateSettings.automaticDownload}
              disabled={globalSettingsDisabled || !updateSettings.automaticCheck}
              on:change={(event) => updateUpdateSettings({ automaticDownload: (event.currentTarget as HTMLInputElement).checked })}
            />
            <span>发现新版本后自动下载</span>
          </label>
          {#if updateState}
            <div class="update-runtime" aria-live="polite">
              <div class="update-runtime-copy">
                <strong>
                  {#if updateState.phase === "notConfigured"}
                    未配置应用内更新
                  {:else if updateState.phase === "checking"}
                    正在检查更新
                  {:else if updateState.phase === "upToDate"}
                    当前已是最新版本
                  {:else if updateState.phase === "available"}
                    发现新版本 {updateState.availableVersion}
                  {:else if updateState.phase === "downloading"}
                    正在下载更新
                  {:else if updateState.phase === "downloaded"}
                    更新已下载，等待确认安装
                  {:else if updateState.phase === "installing"}
                    正在启动更新安装
                  {:else if updateState.phase === "failed"}
                    更新操作失败
                  {:else}
                    当前版本 {updateState.currentVersion}
                  {/if}
                </strong>
                {#if updateState.availableVersion && !["available", "downloaded"].includes(updateState.phase)}
                  <span>版本 {updateState.availableVersion}</span>
                {/if}
                {#if updateState.publishedAt}
                  <span>发布于 {formatDate(updateState.publishedAt)}</span>
                {/if}
                {#if updateState.notes}
                  <p>{updateState.notes}</p>
                {/if}
                {#if updateState.phase === "notConfigured"}
                  <p>此构建未提供更新地址和签名公钥，不影响其他功能或手动安装新版。</p>
                {:else if updateState.lastError}
                  <p class="update-error">{updateErrorMessage(updateState.lastError)}</p>
                {/if}
                {#if updateState.phase === "downloading"}
                  <div class="update-progress">
                    <progress
                      aria-label="更新下载进度"
                      max={updateState.totalBytes ?? undefined}
                      value={updateState.totalBytes ? updateState.downloadedBytes : undefined}
                    ></progress>
                    <span>
                      {formatUpdateBytes(updateState.downloadedBytes)}
                      {#if updateState.totalBytes} / {formatUpdateBytes(updateState.totalBytes)}{/if}
                    </span>
                  </div>
                {/if}
              </div>
              <div class="update-actions">
                <button
                  class="secondary"
                  type="button"
                  disabled={updateOperationBusy || !updateState.canCheck}
                  on:click={() => runUpdateAction("check")}
                >
                  <RefreshCw size={15} />
                  检查更新
                </button>
                {#if updateState.canDownload}
                  <button
                    class="primary"
                    type="button"
                    disabled={updateOperationBusy}
                    on:click={() => runUpdateAction("download")}
                  >
                    <Download size={15} />
                    下载更新
                  </button>
                {/if}
                {#if updateState.canInstall}
                  <button
                    class="primary"
                    type="button"
                    disabled={updateOperationBusy}
                    on:click={() => (updateInstallConfirmOpen = true)}
                  >
                    <Upload size={15} />
                    安装更新
                  </button>
                {/if}
              </div>
            </div>
          {/if}
        </div>

        <div class="settings-section data-root-section" data-settings-category="storage" data-settings-panel="data-root" data-settings-panel-label="本地数据位置" data-settings-search-target>
          <div class="settings-section-heading">
            <div>
              <h3>本地数据位置</h3>
              <p>数据库、设置、浏览器资料和缓存使用同一数据目录。</p>
            </div>
            <button
              class="secondary"
              type="button"
              disabled={syncing || dataRootMigrating || dataRootChoosing || settingsLoading}
              on:click={chooseNewDataRoot}
            >
              <FolderOpen size={15} />
              {dataRootChoosing ? "选择中" : "选择数据目录"}
            </button>
          </div>

          <label class="data-root-field">
            <span>当前目录</span>
            <input aria-label="当前数据目录" readonly value={dataRootValidation?.normalizedPath ?? appSettings?.dataRoot ?? ""} />
          </label>

          {#if dataRootValidation}
            <div class="data-root-candidate">
              {#if dataRootValidation.adjustedToInstallData}
                <p class="data-root-hint">已自动使用安装目录下的 Data 文件夹</p>
              {/if}
              <div class="data-root-actions">
                <button
                  class="primary"
                  type="button"
                  disabled={syncing || dataRootMigrating}
                  on:click={migrateDataRoot}
                >
                  <Database size={15} />
                  {dataRootMigrating ? "迁移中" : "迁移数据"}
                </button>
                <button
                  type="button"
                  disabled={dataRootMigrating}
                  on:click={() => (dataRootValidation = null)}
                >取消</button>
              </div>
            </div>
          {/if}

          {#if appSettings?.retainedDataRoots.length}
            <div class="retained-roots">
              <strong>保留的旧数据目录</strong>
              {#each appSettings.retainedDataRoots as retained}
                <div class="retained-root-row">
                  <span title={retained.root.path}>{retained.root.path}</span>
                  <button
                    class="icon-danger"
                    type="button"
                    title="删除旧数据目录"
                    disabled={dataRootMigrating || retainedRootDeleting}
                    on:click={() => (retainedRootPendingDelete = retained.root.path)}
                  >
                    <Trash2 size={15} />
                  </button>
                </div>
              {/each}
            </div>
          {/if}
        </div>
        <div class="settings-section" data-settings-category="diagnostics" data-settings-panel="about" data-settings-panel-label="诊断与关于" data-settings-search-target>
          <div class="settings-section-heading">
            <div><h3>诊断与关于</h3><p>Emily · 抖音内容本地归档</p></div>
          </div>
          <div class="settings-about-grid">
            <span>应用版本</span><strong>{updateState?.currentVersion ?? "0.1.0"}</strong>
            <span>数据架构</span><strong>SQLite schema 8</strong>
          </div>
        </div>
    </SettingsWorkspace>
  {/if}

  {#if backupExportConfirmOpen}
    <div class="modal-backdrop confirmation-backdrop" role="presentation" on:click={() => (backupExportConfirmOpen = false)}>
      <div class="settings-modal confirmation-modal" role="dialog" aria-modal="true" aria-label="导出备份" tabindex="-1" on:click|stopPropagation on:keydown|stopPropagation>
        <header class="modal-header">
          <h2>导出备份</h2>
          <button class="icon-button" type="button" title="关闭" on:click={() => (backupExportConfirmOpen = false)}><X size={16} /></button>
        </header>
        <div class="confirmation-content">
          <p>备份不会加密，任何获得该文件的人都可能读取其中的数据。</p>
          <p>文件包含数据库与应用设置，即完整个人归档；不包含封面、浏览器登录资料、日志或下载内容。</p>
          <div class="confirmation-actions">
            <button type="button" on:click={() => (backupExportConfirmOpen = false)}>取消</button>
            <button class="primary" type="button" on:click={confirmBackupExport}>确认导出</button>
          </div>
        </div>
      </div>
    </div>
  {/if}

  {#if updateInstallConfirmOpen}
    <div class="modal-backdrop confirmation-backdrop" role="presentation" on:click={() => (updateInstallConfirmOpen = false)}>
      <div class="settings-modal confirmation-modal" role="dialog" aria-modal="true" aria-label="确认安装更新" tabindex="-1" on:click|stopPropagation on:keydown|stopPropagation>
        <header class="modal-header">
          <h2>确认安装更新</h2>
          <button class="icon-button" type="button" title="关闭" on:click={() => (updateInstallConfirmOpen = false)}><X size={16} /></button>
        </header>
        <div class="confirmation-content">
          <p>安装前会停止后台任务并创建安全备份。安装程序启动后，应用将退出。</p>
          {#if updateState?.availableVersion}
            <p>准备安装 Emily {updateState.availableVersion}。</p>
          {/if}
          <div class="confirmation-actions">
            <button type="button" on:click={() => (updateInstallConfirmOpen = false)}>取消</button>
            <button class="primary" type="button" on:click={() => runUpdateAction("install")}>确认安装</button>
          </div>
        </div>
      </div>
    </div>
  {/if}

  {#if backupRestoreCandidate}
    <div class="modal-backdrop confirmation-backdrop" role="presentation">
      <div class:merge-modal={backupMergePlan !== null} class="settings-modal confirmation-modal" role="dialog" aria-modal="true" aria-label="导入备份" tabindex="-1" on:click|stopPropagation on:keydown|stopPropagation>
        <header class="modal-header">
          <h2>导入备份</h2>
          <button class="icon-button" type="button" title="关闭" disabled={backupRestoreBusy} on:click={closeBackupRestoreDialog}><X size={16} /></button>
        </header>
        <div class="confirmation-content">
          <dl class="backup-restore-summary">
            <div><dt>创建时间</dt><dd>{backupRestoreCandidate.summary.createdAt}</dd></div>
            <div><dt>应用版本</dt><dd>{backupRestoreCandidate.summary.applicationVersion}</dd></div>
            <div><dt>数据库</dt><dd>数据库 schema {backupRestoreCandidate.summary.databaseSchemaVersion}</dd></div>
            <div><dt>设置</dt><dd>设置 schema {backupRestoreCandidate.summary.settingsSchemaVersion}</dd></div>
            <div><dt>文件大小</dt><dd>{backupRestoreCandidate.summary.byteSize.toLocaleString("zh-CN")} 字节</dd></div>
          </dl>
          <p>恢复前会自动创建安全备份。浏览器资料、登录状态、开机启动和窗口状态仍保留当前设备配置。</p>

          {#if backupMergePlan}
            <section class="backup-merge-section" aria-label="待合并记录">
              <h3>待合并记录</h3>
              <dl class="backup-restore-summary compact">
                <div><dt>账号</dt><dd>{backupMergePlan.counts.accounts}</dd></div>
                <div><dt>视频</dt><dd>{backupMergePlan.counts.videos}</dd></div>
                <div><dt>观看记录</dt><dd>{backupMergePlan.counts.watchRecords}</dd></div>
                <div><dt>喜欢 / 收藏</dt><dd>{backupMergePlan.counts.likes} / {backupMergePlan.counts.favorites}</dd></div>
                <div><dt>远端 / 本地收藏夹</dt><dd>{backupMergePlan.counts.remoteFolders} / {backupMergePlan.counts.localFolders}</dd></div>
              </dl>
            </section>

            {#if backupMergePlan.localFolderConflicts.length > 0}
              <section class="backup-merge-section" aria-label="本地收藏夹冲突">
                <h3>本地收藏夹冲突</h3>
                {#each backupMergePlan.localFolderConflicts as conflict}
                  <div class="backup-merge-conflict-row">
                    <label>
                      <span>{conflict.name}</span>
                      <SelectField
                        ariaLabel={`收藏夹“${conflict.name}”的处理方式`}
                        value={backupFolderResolutions[conflict.id]?.action ?? ""}
                        disabled={backupRestoreBusy}
                        on:change={(event) => updateBackupFolderResolution(conflict.id, event.detail.value)}
                        options={[{ value: "", label: "请选择" }, { value: "merge", label: "合并" }, { value: "rename", label: "重命名导入项" }, { value: "skip", label: "跳过" }]}
                      />
                    </label>
                    {#if backupFolderResolutions[conflict.id]?.action === "rename"}
                      <input
                        aria-label={`收藏夹“${conflict.name}”的新名称`}
                        type="text"
                        value={backupFolderRenameNames[conflict.id] ?? ""}
                        disabled={backupRestoreBusy}
                        on:input={(event) => updateBackupFolderRename(conflict.id, (event.currentTarget as HTMLInputElement).value)}
                      />
                    {/if}
                  </div>
                {/each}
              </section>
            {/if}

            {#if backupMergePlan.settingsConflicts.length > 0}
              <section class="backup-merge-section" aria-label="设置冲突">
                <h3>设置冲突</h3>
                <label class="backup-merge-resolution-row">
                  <span>处理方式</span>
                  <SelectField
                    ariaLabel="设置冲突处理"
                    value={backupSettingsResolutionMode}
                    disabled={backupRestoreBusy}
                    on:change={(event) => (backupSettingsResolutionMode = event.detail.value as SettingsMergeResolution["mode"])}
                    options={[{ value: "current", label: "全部使用当前设置" }, { value: "backup", label: "全部使用备份设置" }, { value: "perField", label: "逐项选择" }]}
                  />
                </label>
                {#if backupSettingsResolutionMode === "perField"}
                  {#each backupMergePlan.settingsConflicts as conflict}
                    <label class="backup-merge-setting-row">
                      <span><strong>{conflict.path}</strong><small>当前：{formatMergeSettingValue(conflict.currentValue)} · 备份：{formatMergeSettingValue(conflict.backupValue)}</small></span>
                      <SelectField
                        ariaLabel={`设置“${conflict.path}”的来源`}
                        value={backupSettingChoices[conflict.id] ?? ""}
                        disabled={backupRestoreBusy}
                        on:change={(event) => updateBackupSettingChoice(conflict.id, event.detail.value)}
                        options={[{ value: "", label: "请选择" }, { value: "current", label: "当前" }, { value: "backup", label: "备份" }]}
                      />
                    </label>
                  {/each}
                {/if}
              </section>
            {/if}

            <p>去重合并保留当前设备运行状态，并按稳定身份合并账号、内容、事件和收藏夹。</p>
            <div class="confirmation-actions">
              <button type="button" disabled={backupRestoreBusy} on:click={resetBackupMergeState}>返回</button>
              <button class="primary" type="button" disabled={backupRestoreBusy || !backupMergeReady} on:click={confirmBackupMerge}>{backupRestoreBusy ? "合并中" : "确认去重合并"}</button>
            </div>
          {:else}
            <p>直接替换会用归档内容替换当前数据库和可移植设置；去重合并会保留当前数据并让你处理收藏夹与设置冲突。</p>
            <div class="confirmation-actions backup-restore-actions">
              <button type="button" disabled={backupRestoreBusy} on:click={closeBackupRestoreDialog}>取消</button>
              <button class="secondary" type="button" disabled={backupRestoreBusy} on:click={prepareBackupMergePlan}>{backupRestoreBusy ? "准备中" : "准备去重合并"}</button>
              <button class="danger" type="button" disabled={backupRestoreBusy} on:click={confirmBackupRestore}>{backupRestoreBusy ? "恢复中" : "确认直接替换"}</button>
            </div>
          {/if}
        </div>
      </div>
    </div>
  {/if}

  {#if retainedRootPendingDelete}
    <div class="modal-backdrop confirmation-backdrop" role="presentation">
      <div
        class="settings-modal confirmation-modal"
        role="dialog"
        aria-modal="true"
        aria-label="删除旧数据目录"
      >
        <header class="modal-header">
          <h2>删除旧数据目录</h2>
          <button
            class="icon-button"
            type="button"
            title="关闭"
            disabled={retainedRootDeleting}
            on:click={() => (retainedRootPendingDelete = null)}
          ><X size={16} /></button>
        </header>
        <div class="confirmation-content">
          <p>将永久删除以下旧数据目录及其中的受管数据：</p>
          <code>{retainedRootPendingDelete}</code>
          <div class="confirmation-actions">
            <button
              type="button"
              disabled={retainedRootDeleting}
              on:click={() => (retainedRootPendingDelete = null)}
            >取消</button>
            <button
              class="danger"
              type="button"
              disabled={retainedRootDeleting}
              on:click={confirmRetainedRootDelete}
            >{retainedRootDeleting ? "删除中" : "确认删除"}</button>
          </div>
        </div>
      </div>
    </div>
  {/if}

  {#if coverCacheEnableOpen}
    <ConfirmDialog
      title="启用封面缓存"
      onClose={() => (coverCacheEnableOpen = false)}
    >
      <p>请选择首次启用后的缓存范围。两种模式都会继续缓存后续同步新增记录。</p>
      {#snippet actions()}
        <button type="button" data-dialog-cancel on:click={() => (coverCacheEnableOpen = false)}>取消</button>
        <button type="button" on:click={() => enableCoverCache("newRecordsOnly")}>仅后续新增</button>
        <button class="primary" type="button" on:click={() => enableCoverCache("backfillAll")}>补齐全部历史</button>
      {/snippet}
    </ConfirmDialog>
  {/if}

  {#if coverCacheManagerOpen}
    <CoverCacheManager mode={coverCacheSettings.cleanupListMode} settings={coverCacheSettings} onClose={() => (coverCacheManagerOpen = false)} />
  {/if}

  {#if folderPickerRecord}
    <div class="modal-backdrop" role="presentation" on:click={closeFolderPicker}>
      <div
        class="settings-modal folder-picker-modal"
        role="dialog"
        aria-modal="true"
        aria-label="加入本地收藏夹"
        tabindex="-1"
        on:click|stopPropagation
        on:keydown|stopPropagation
      >
        <header class="modal-header">
          <div>
            <h2>加入本地收藏夹</h2>
            <small>{folderPickerRecord.title ?? "未命名内容"}</small>
          </div>
          <button class="icon-button" type="button" title="关闭" disabled={folderPickerBusy} on:click={closeFolderPicker}><X size={16} /></button>
        </header>
        <div class="folder-picker-body">
          {#if folderPickerLoading}
            <div class="folder-picker-empty">正在读取收藏夹...</div>
          {:else if folderPickerFolders.length === 0 && !folderPickerCreateOpen}
            <div class="folder-picker-empty">还没有本地收藏夹</div>
          {:else}
            <div class="folder-picker-list">
              {#each folderPickerFolders as folder}
                <button type="button" disabled={folderPickerBusy} on:click={() => addRecordToFolder(folder)}>
                  {#if folder.scope === "aggregate"}<Users size={16} />{:else}<FolderOpen size={16} />{/if}
                  <span><strong>{folder.name}</strong><small>{folder.scope === "aggregate" ? "多账号聚合" : "当前账号"} · {folder.itemCount} 条</small></span>
                </button>
              {/each}
            </div>
          {/if}
          {#if folderPickerCreateOpen}
            <form class="folder-picker-create" on:submit|preventDefault={createFolderAndAdd}>
              <input aria-label="新收藏夹名称" placeholder="收藏夹名称" bind:value={folderPickerName} />
              <SelectField ariaLabel="新收藏夹归属" bind:value={folderPickerScope} options={[{ value: "account", label: "当前账号" }, { value: "aggregate", label: "多账号聚合" }]} />
              <div>
                <button type="button" disabled={folderPickerBusy} on:click={() => (folderPickerCreateOpen = false)}>取消</button>
                <button class="primary" type="submit" disabled={folderPickerBusy || !folderPickerName.trim()}>新建并加入</button>
              </div>
            </form>
          {:else if !folderPickerLoading}
            <button class="secondary folder-picker-new" type="button" on:click={() => (folderPickerCreateOpen = true)}>新建收藏夹</button>
          {/if}
        </div>
      </div>
    </div>
  {/if}

  {#if downloadDialogAccountId !== null && downloadDialogSelection && downloadDialogPreview}
    <DownloadStartDialog
      accountId={downloadDialogAccountId}
      selection={downloadDialogSelection}
      preview={downloadDialogPreview}
      settings={downloadSettings}
      onclose={closeDownloadDialog}
      oncreated={handleDownloadCreated}
    />
  {/if}

  {#if activePage === "accounts" && appSettings}
    <AccountWorkspace
      {accounts}
      {activeAccount}
      syncMode={appSettings.workspace.sync.mode}
      syncAccountScope={appSettings.workspace.sync.accountScope}
      availableSources={availableSyncSources}
      busy={accountBusy || dataRootMigrating}
      loginBusy={accountLoginBusy}
      {syncing}
      onActivate={switchAccount}
      onAdd={addAccount}
      onRelogin={resumeLogin}
      onArchive={(account) => manageAccount(account, "archive")}
      onRestore={(account) => manageAccount(account, "restore")}
      onDelete={(account) => manageAccount(account, "delete")}
      onSyncPreferenceChange={updateWorkspaceSyncPreference}
      onSync={enqueueConfiguredWorkspaceSync}
    />
  {:else if (activePage === "syncRecords" || activePage === "downloads") && appSettings}
    <TaskCenterPage
      {accounts}
      accountScope={{ kind: "all" }}
    />
  {:else if activePage === "notifications"}
    <NotificationsPage />
  {:else if activePage === "library" && appSettings}
    <LibraryPage
      {accounts}
      accountScope={appSettings.workspace.displayScopes.library}
      preferences={appSettings.workspace.contentViews}
      sourceAvailability={{
        history: capabilityBySource.history?.available ?? false,
        likes: capabilityBySource.likes?.available ?? false,
        favorites: capabilityBySource.favorites?.available ?? false,
      }}
      {likeCountFormat}
      coverCacheEnabled={coverCacheSettings.enabled}
      onPreferenceChange={updateLibraryPreference}
    />
  {:else if activePage === "search" && appSettings}
    <SearchPage
      {accounts}
      accountScope={appSettings.workspace.displayScopes.search}
      {likeCountFormat}
      coverCacheEnabled={coverCacheSettings.enabled}
    />
  {:else if activePage === "overview" && annualReviewOpen && appSettings}
    <AnnualReviewPage
      {accounts}
      accountScope={appSettings.workspace.displayScopes.overview}
      onClose={() => (annualReviewOpen = false)}
    />
  {:else if activePage === "overview" && appSettings}
    <OverviewPage
      {accounts}
      accountScope={appSettings.workspace.displayScopes.overview}
      bind:source={overviewSource}
      bind:range={overviewRange}
      bind:includeRemoteRemoved={overviewIncludeRemoteRemoved}
      onOpenAnnualReview={() => (annualReviewOpen = true)}
    />
  {:else if activeAccount && activePage === "remoteFolders"}
    <FolderPage
      kind="remote"
      accountId={activeAccount.id}
      {accounts}
      {likeCountFormat}
      coverCacheEnabled={coverCacheSettings.enabled}
    />
  {:else if activeAccount && activePage === "localFolders"}
    <FolderPage
      kind="local"
      accountId={activeAccount.id}
      {accounts}
      {likeCountFormat}
      coverCacheEnabled={coverCacheSettings.enabled}
    />
  {:else if activeAccount && activePage === "syncRecords"}
    <SyncRecordsPage accountId={activeAccount.id} />
  {:else if activeAccount && contentPageActive}
  <section class="toolbar">
    <label class="search-box">
      <Search size={16} />
      <input
        aria-label="来源内容搜索"
        value={query}
        on:input={(event) => updateQuery((event.currentTarget as HTMLInputElement).value)}
        type="search"
        placeholder="搜索标题或作者"
      />
    </label>
    <div class="content-toolbar-actions">
      <div class="sort-control">
        <span>排序</span>
        <SelectField
          ariaLabel="排序"
          value={currentContentView.sort}
          on:change={(event) => setContentSort(event.detail.value as ContentSort)}
          options={currentSource === "history"
            ? [
                { value: "watchedNewest", label: "观看时间从新到旧" },
                { value: "watchedOldest", label: "观看时间从旧到新" },
                { value: "likeCountDesc", label: "点赞量从高到低" },
                { value: "titleAsc", label: "标题" },
                { value: "authorAsc", label: "作者" },
              ]
            : [
                { value: "pageOrder", label: "页面采集顺序" },
                { value: "remoteNewest", label: "远端时间从新到旧" },
                { value: "remoteOldest", label: "远端时间从旧到新" },
                { value: "likeCountDesc", label: "点赞量从高到低" },
                { value: "titleAsc", label: "标题" },
                { value: "authorAsc", label: "作者" },
              ]}
        />
      </div>
      {#if currentSource !== "history"}
        <label class="removed-filter">
          <input
            type="checkbox"
            checked={currentContentView.includeRemoteRemoved}
            on:change={(event) =>
              setIncludeRemoteRemoved((event.currentTarget as HTMLInputElement).checked)}
          />
          <span>包含已从远端移除</span>
        </label>
      {/if}
      <div class="segmented view-mode" role="group" aria-label="内容视图">
        <button
          class:active={currentContentView.viewMode === "list"}
          type="button"
          title="列表视图"
          aria-label="列表视图"
          aria-pressed={currentContentView.viewMode === "list"}
          on:click={() => setContentViewMode("list")}
        ><List size={16} /></button>
        <button
          class:active={currentContentView.viewMode === "grid"}
          type="button"
          title="网格视图"
          aria-label="网格视图"
          aria-pressed={currentContentView.viewMode === "grid"}
          on:click={() => setContentViewMode("grid")}
        ><Grid3X3 size={16} /></button>
      </div>
      <button class="ghost" type="button" on:click={() => refreshContent()}>刷新</button>
    </div>
  </section>

  {#if currentSource === "history"}
  <section class="selection-bar">
    <label class="select-all">
      <input
        type="checkbox"
        checked={allVisibleSelected}
        disabled={records.length === 0 || loading || deleting}
        on:change={(event) =>
          toggleVisibleSelection((event.currentTarget as HTMLInputElement).checked)}
      />
      <span>全选当前页</span>
    </label>
    <span>{selectedCount > 0 ? `已选择 ${selectedCount} 条` : "未选择记录"}</span>
    <div class="selection-actions">
      <button
        class="secondary"
        type="button"
        disabled={selectedCount === 0 || deleting || downloadPreviewing}
        on:click={() => openDownloadDialog("checked")}
      >
        <Download size={15} />
        下载选中
      </button>
      <button
        class="secondary"
        type="button"
        disabled={total === 0 || deleting || downloadPreviewing}
        on:click={() => openDownloadDialog("filtered")}
      >
        <Download size={15} />
        下载当前筛选
      </button>
      <button
        class="secondary"
        type="button"
        disabled={total === 0 || deleting || downloadPreviewing}
        on:click={() => openDownloadDialog("all")}
      >
        <Download size={15} />
        下载全部历史
      </button>
      <button
        class="danger"
        type="button"
        disabled={selectedCount === 0 || deleting}
        on:click={deleteSelected}
      >
        <Trash2 size={15} />
        删除选中
      </button>
      <button
        class="danger ghost-danger"
        type="button"
        disabled={total === 0 || deleting}
        on:click={deleteCurrentScope}
      >
        <Trash2 size={15} />
        {query.trim() ? "删除筛选" : "删除全部"}
      </button>
    </div>
  </section>
  {/if}

  <section
    class:content-list={currentContentView.viewMode === "list"}
    class:content-grid={currentContentView.viewMode === "grid"}
  >
    {#if loading}
      <div class="empty-state">正在读取本地内容...</div>
    {:else if records.length === 0}
      <div class="empty-state">没有匹配的{sourceLabel(currentSource)}记录</div>
    {:else}
      {#each records as record}
        <article class="content-item" class:remote-removed={record.remoteRemovedAt != null}>
          {#if currentSource === "history"}
            <label class="row-check" aria-label="选择记录">
              <input
                type="checkbox"
                checked={selectedIds.has(record.id)}
                disabled={deleting}
                on:change={(event) =>
                  toggleRecordSelection(record.id, (event.currentTarget as HTMLInputElement).checked)}
              />
            </label>
          {/if}
          {#if currentContentView.viewMode === "grid"}
            <button
              class="cover-slot cover-button"
              type="button"
              title={`打开${record.title ?? "未命名内容"}`}
              on:click={() => openRecord(record)}
            >
              <CoverImage cachedCoverUrl={record.cachedCoverUrl} remoteCoverUrl={record.coverUrl} preferCached={coverCacheSettings.enabled} alt={record.title ?? "内容封面"} size={22} />
            </button>
          {:else}
            <div class="cover-slot">
              <CoverImage cachedCoverUrl={record.cachedCoverUrl} remoteCoverUrl={record.coverUrl} preferCached={coverCacheSettings.enabled} alt={record.title ?? "内容封面"} size={18} />
            </div>
          {/if}
          <div class="row-main">
            <button
              class="content-title"
              type="button"
              title={record.url}
              on:click={() => openRecord(record)}
            >
              {record.title ?? "未命名视频"}
            </button>
            <div class="row-meta">
              {#if detailFields.author}
                <span>{record.authorName ?? "未知作者"}</span>
              {/if}
              {#if currentSource === "history" && detailFields.watchDate}
                <span>{watchDateLabel(record)}</span>
              {/if}
              {#if detailFields.likeCount && record.likeCount != null}
                <span>点赞 {formatLikeCount(record.likeCount, likeCountFormat)}</span>
              {/if}
              {#if currentSource === "history" && detailFields.watchProgress && record.watchProgressRate != null}
                <span>{formatWatchProgress(record.watchProgressRate)}</span>
              {/if}
              {#if currentSource !== "history"}
                <span>首次归档 {formatDate(record.firstSeenAt ?? undefined)}</span>
                <span>最近归档 {formatDate(record.lastSeenAt ?? undefined)}</span>
                {#if record.remoteRemovedAt}
                  <span class="removed-state">已从远端移除</span>
                {/if}
              {/if}
            </div>
          </div>
          <div class="content-item-actions">
            <button
              class="icon-button"
              type="button"
              title="加入本地收藏夹"
              aria-label={`将${record.title ?? "未命名内容"}加入本地收藏夹`}
              on:click={() => openFolderPicker(record)}
            >
              <BookmarkPlus size={15} />
            </button>
          {#if currentSource === "history"}
            <button
              class="icon-danger"
              type="button"
              title="删除这条本地记录"
              disabled={deleting}
              on:click={() => deleteOne(record)}
            >
              <Trash2 size={15} />
            </button>
          {/if}
          </div>
        </article>
      {/each}
    {/if}
  </section>

  <section class="pagination-bar" aria-label="分页">
    <span>{total === 0 ? "0 条" : `${pageStart}-${pageEnd} / ${total} 条`}</span>
    <div class="pagination-actions">
      <button
        class="icon-button"
        type="button"
        title="上一页"
        disabled={loading || currentPage <= 1}
        on:click={() => goToPage(currentPage - 1)}
      >
        <ChevronLeft size={16} />
      </button>
      <span>第 {currentPage} / {totalPages} 页</span>
      <button
        class="icon-button"
        type="button"
        title="下一页"
        disabled={loading || currentPage >= totalPages || total === 0}
        on:click={() => goToPage(currentPage + 1)}
      >
        <ChevronRight size={16} />
      </button>
    </div>
  </section>
  {/if}
    </div>
  </AppShell>
  {/if}
</main>
{/if}
  </div>
</div>
