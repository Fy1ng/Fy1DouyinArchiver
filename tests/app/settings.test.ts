import { describe, expect, it, vi } from "vitest";
import {
  LEGACY_SETTINGS_KEY,
  LIKE_COUNT_FORMAT_OPTIONS,
  SettingsRequestGeneration,
  SettingsSaveQueue,
  dataRootErrorMessage,
  loadSettingsWithLegacyImport,
  normalizeLikeCountFormat,
  normalizeLegacyFrontendSettings,
} from "../../app/src/settings.js";
import type {
  AccountSettings,
  AppSettingsView,
  LegacyFrontendSettings,
} from "../../app/src/types.js";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

function accountSettings(): AccountSettings {
  return {
    hideBrowser: false,
    scrollSpeed: 900,
    history: {
      range: { kind: "days", days: 7 },
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
  };
}

function settingsView(legacyImportRequired: boolean): AppSettingsView {
  const account = accountSettings();
  return {
    revision: legacyImportRequired ? 0 : 1,
    dataRoot: "C:\\Users\\Test\\AppData\\Emily\\Data",
    installDir: "C:\\Program Files\\Emily",
    retainedDataRoots: [],
    legacyImportRequired,
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
    },
    accountDefaults: structuredClone(account),
    account,
  };
}

function memoryStorage(initial?: string): Storage {
  const values = new Map<string, string>();
  if (initial !== undefined) values.set(LEGACY_SETTINGS_KEY, initial);
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, value),
  };
}

describe("点赞量格式设置", () => {
  it("提供准确数字、W、万、K、千五种选项", () => {
    expect(LIKE_COUNT_FORMAT_OPTIONS).toEqual([
      { value: "exact", label: "准确数字" },
      { value: "w", label: "W" },
      { value: "wan", label: "万" },
      { value: "k", label: "K" },
      { value: "qian", label: "千" },
    ]);
  });

  it("保留有效旧配置并将未知值回退到万", () => {
    expect(normalizeLikeCountFormat("exact")).toBe("exact");
    expect(normalizeLikeCountFormat("wan")).toBe("wan");
    expect(normalizeLikeCountFormat("k")).toBe("k");
    expect(normalizeLikeCountFormat("w")).toBe("w");
    expect(normalizeLikeCountFormat("qian")).toBe("qian");
    expect(normalizeLikeCountFormat("english-k")).toBe("wan");
    expect(normalizeLikeCountFormat(undefined)).toBe("wan");
  });
});

describe("统一设置迁移协调", () => {
  const legacyRaw = JSON.stringify({
    syncMode: "bottom",
    syncDays: 12,
    scrollSpeed: 4_200,
    hideBrowser: true,
    likeCountFormat: "k",
    detailFields: { author: false, watchDate: true },
  });

  it("只在 Rust 确认导入完成后删除 legacy localStorage", async () => {
    const storage = memoryStorage(legacyRaw);
    const getAppSettings = vi
      .fn()
      .mockResolvedValueOnce(settingsView(true))
      .mockResolvedValueOnce(settingsView(false));
    const importLegacySettings = vi.fn().mockResolvedValue(1);

    const loaded = await loadSettingsWithLegacyImport({
      accountId: 11,
      storage,
      getAppSettings,
      importLegacySettings,
    });

    expect(importLegacySettings).toHaveBeenCalledWith(11, {
      syncMode: "bottom",
      syncDays: 12,
      scrollSpeed: 4_200,
      hideBrowser: true,
      likeCountFormat: "k",
      detailFields: {
        author: false,
        watchDate: true,
        likeCount: true,
        watchProgress: true,
      },
    });
    expect(loaded.legacyImportRequired).toBe(false);
    expect(storage.getItem(LEGACY_SETTINGS_KEY)).toBeNull();
  });

  it("Rust 已提交但 IPC 响应丢失时通过再次读取确认并清理 legacy key", async () => {
    const storage = memoryStorage(legacyRaw);
    const getAppSettings = vi
      .fn()
      .mockResolvedValueOnce(settingsView(true))
      .mockResolvedValueOnce(settingsView(false));
    const importLegacySettings = vi.fn().mockRejectedValue(new Error("ipc response lost"));

    const loaded = await loadSettingsWithLegacyImport({
      accountId: null,
      storage,
      getAppSettings,
      importLegacySettings,
    });

    expect(loaded.legacyImportRequired).toBe(false);
    expect(storage.getItem(LEGACY_SETTINGS_KEY)).toBeNull();
  });

  it("导入失败且后端仍未完成时保留 legacy key", async () => {
    const storage = memoryStorage(legacyRaw);
    const getAppSettings = vi.fn().mockResolvedValue(settingsView(true));
    const importLegacySettings = vi.fn().mockRejectedValue(new Error("disk full"));

    await expect(
      loadSettingsWithLegacyImport({
        accountId: 11,
        storage,
        getAppSettings,
        importLegacySettings,
      }),
    ).rejects.toThrow("disk full");
    expect(storage.getItem(LEGACY_SETTINGS_KEY)).toBe(legacyRaw);
  });

  it("规范化旧设置并拒绝损坏 JSON", () => {
    expect(normalizeLegacyFrontendSettings(legacyRaw)?.syncDays).toBe(12);
    expect(normalizeLegacyFrontendSettings("not-json")).toBeNull();
  });

  it("保存队列严格串行且请求代次会淘汰迟到响应", async () => {
    const queue = new SettingsSaveQueue();
    const order: string[] = [];
    let releaseFirst!: () => void;
    const firstGate = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    const first = queue.enqueue(async () => {
      order.push("first:start");
      await firstGate;
      order.push("first:end");
    });
    const second = queue.enqueue(async () => {
      order.push("second");
    });
    await Promise.resolve();
    expect(order).toEqual(["first:start"]);
    releaseFirst();
    await Promise.all([first, second]);
    expect(order).toEqual(["first:start", "first:end", "second"]);

    const generation = new SettingsRequestGeneration();
    const oldRequest = generation.begin();
    const currentRequest = generation.begin();
    expect(generation.isCurrent(oldRequest)).toBe(false);
    expect(generation.isCurrent(currentRequest)).toBe(true);
  });

  it("将数据根错误映射为稳定中文提示", () => {
    expect(dataRootErrorMessage("data_root_is_drive_root")).toContain("磁盘根目录");
    expect(dataRootErrorMessage("data_root_is_remote")).toContain("网络");
    expect(dataRootErrorMessage("data_root_permission_denied")).toContain("权限");
  });
});

describe("统一设置与数据根 IPC", () => {
  it("使用稳定命令名和 camelCase 参数", async () => {
    Object.assign(globalThis, { window: { __TAURI_INTERNALS__: {} } });
    invoke.mockReset().mockResolvedValue({});
    const api = await import("../../app/src/app-api.js");
    const global = settingsView(false).global;
    const account = settingsView(false).account;
    const legacy: LegacyFrontendSettings = {
      syncMode: "days",
      syncDays: 7,
      hideBrowser: account.hideBrowser,
      scrollSpeed: account.scrollSpeed,
      likeCountFormat: global.likeCountFormat,
      detailFields: global.detailFields,
    };

    await api.getAppSettings(null);
    await api.updateGlobalSettings(global);
    await api.updateAccountSettings(11, account);
    await api.importLegacySettings(null, legacy);
    await api.chooseDataRoot();
    await api.validateDataRoot("C:\\Archive");
    await api.setDataRoot("C:\\Archive");
    await api.deleteRetainedDataRoot("C:\\OldData");
    await api.getDesktopState();
    await api.takeDesktopIntents(4);
    await api.getSchedulerState();
    await api.setSchedulerPaused(true);
    await api.beginAccountLogin("chrome");

    expect(invoke.mock.calls).toEqual([
      ["get_app_settings", { accountId: null }],
      ["update_global_settings", { settings: global }],
      ["update_account_settings", { accountId: 11, settings: account }],
      ["import_legacy_settings", { accountId: null, legacy }],
      ["choose_data_root"],
      ["validate_data_root", { path: "C:\\Archive" }],
      ["set_data_root", { path: "C:\\Archive" }],
      ["delete_retained_data_root", { path: "C:\\OldData" }],
      ["get_desktop_state"],
      ["take_desktop_intents", { lastSeenRevision: 4 }],
      ["get_scheduler_state"],
      ["set_scheduler_paused", { paused: true }],
      ["begin_account_login", { request: { browserPreference: "chrome" } }],
    ]);
  });
});
