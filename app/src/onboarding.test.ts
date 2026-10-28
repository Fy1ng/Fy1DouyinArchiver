import { describe, expect, it } from "vitest";
import {
  buildOnboardingRequest,
  createOnboardingDraft,
  sourceControlState,
} from "./onboarding";
import type {
  AccountSettings,
  AppSettingsView,
  DesktopStateView,
  SourceCapability,
} from "./types";

const accountDefaults = (): AccountSettings => ({
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
});

const settings = (): AppSettingsView => ({
  revision: 8,
  dataRoot: "C:\\Data",
  installDir: "C:\\Emily",
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
  accountDefaults: accountDefaults(),
  account: accountDefaults(),
});

const desktop = (step: 0 | 1 | 2 | 3): DesktopStateView => ({
  device: {
    revision: 3,
    onboardingCompleted: false,
    onboardingStep: step,
    launchAtStartup: false,
    silentAutostart: false,
    schedulerPaused: false,
    backupDirectory: null,
    accounts: {},
  },
  trayAvailable: true,
  trayError: null,
  autostartError: null,
});

describe("onboarding", () => {
  it.each([0, 1, 2, 3] as const)("resumes from backend step %s", (step) => {
    const draft = createOnboardingDraft(settings(), desktop(step));
    expect(draft.step).toBe(step);
    expect(draft.highestStep).toBe(step);
    expect(draft.dataRoot).toBe("C:\\Data");
  });

  it("moves back within the draft without lowering persisted progress", async () => {
    const { stepBack } = await import("./onboarding");
    const draft = createOnboardingDraft(settings(), desktop(3));
    const previous = stepBack(draft);
    expect(previous.step).toBe(2);
    expect(previous.highestStep).toBe(3);
    expect(stepBack(stepBack(previous)).step).toBe(0);
    expect(stepBack(stepBack(stepBack(previous))).step).toBe(0);
  });

  it("advances one local step when the backend keeps a higher recovery marker", async () => {
    const { applyOnboardingCommit } = await import("./onboarding");
    const draft = createOnboardingDraft(settings(), desktop(3));
    draft.step = 1;
    draft.highestStep = 3;

    const next = applyOnboardingCommit(draft, 9, 4, 3, false);

    expect(next.step).toBe(2);
    expect(next.highestStep).toBe(3);
  });

  it.each([
    [
      0,
      (draft: ReturnType<typeof createOnboardingDraft>) => {
        draft.skippedAccount = true;
      },
      { kind: "account", skipped: true },
    ],
    [
      1,
      (draft: ReturnType<typeof createOnboardingDraft>) => {
        draft.syncAfterOnboarding = true;
      },
      {
        kind: "syncRange",
        accountDefaults: accountDefaults(),
        syncAfterOnboarding: true,
      },
    ],
    [
      2,
      (draft: ReturnType<typeof createOnboardingDraft>) => {
        draft.startupSyncDelayMinutes = 12;
      },
      {
        kind: "automaticSync",
        accountDefaults: accountDefaults(),
        startupSyncDelayMinutes: 12,
      },
    ],
    [
      3,
      (draft: ReturnType<typeof createOnboardingDraft>) => {
        draft.closeBehavior = "tray";
        draft.launchAtStartup = true;
        draft.silentAutostart = true;
      },
      {
        kind: "desktopBehavior",
        closeBehavior: "tray",
        launchAtStartup: true,
        silentAutostart: true,
      },
    ],
  ] as const)("builds the exact tagged request for step %s", (step, prepare, expectedStep) => {
    const draft = createOnboardingDraft(settings(), desktop(step));
    prepare(draft);
    expect(buildOnboardingRequest(draft)).toEqual({
      expectedPortableRevision: 8,
      expectedDeviceRevision: 3,
      step: expectedStep,
    });
  });

  it("disables unavailable source schedules and unsupported time ranges", () => {
    const capability: SourceCapability = {
      source: "likes",
      available: false,
      unavailableReason: "collector_not_implemented",
      supportedRanges: ["all", "recentItems", "recordLimit"],
      supportsFullReconcile: true,
      supportsRemoteFolders: false,
    };

    expect(sourceControlState(capability, "recentDays")).toEqual({
      rangeDisabled: true,
      scheduleDisabled: true,
      reason: "collector_not_implemented",
    });
  });

  it("rejects silent startup when autostart is disabled and tray close when unavailable", () => {
    const draft = createOnboardingDraft(settings(), desktop(3));
    draft.silentAutostart = true;
    expect(() => buildOnboardingRequest(draft)).toThrow("silent_autostart_requires_autostart");

    draft.silentAutostart = false;
    draft.closeBehavior = "tray";
    draft.trayAvailable = false;
    expect(() => buildOnboardingRequest(draft)).toThrow("tray_unavailable");
  });
});
