import type {
  AccountSettings,
  AppSettingsView,
  CloseBehavior,
  CommitOnboardingStepRequest,
  DesktopStateView,
  SourceCapability,
  SyncRangeKind,
} from "./types";

export interface OnboardingDraft {
  step: 0 | 1 | 2 | 3;
  highestStep: 0 | 1 | 2 | 3;
  portableRevision: number;
  deviceRevision: number;
  dataRoot: string;
  selectedDataRoot: string | null;
  accountDefaults: AccountSettings;
  skippedAccount: boolean;
  syncAfterOnboarding: boolean;
  startupSyncDelayMinutes: number;
  closeBehavior: CloseBehavior;
  launchAtStartup: boolean;
  silentAutostart: boolean;
  trayAvailable: boolean;
}

export interface SourceControlState {
  rangeDisabled: boolean;
  scheduleDisabled: boolean;
  reason: string | null;
}

export function createOnboardingDraft(
  settings: AppSettingsView,
  desktop: DesktopStateView,
): OnboardingDraft {
  return {
    step: desktop.device.onboardingStep,
    highestStep: desktop.device.onboardingStep,
    portableRevision: settings.revision,
    deviceRevision: desktop.device.revision,
    dataRoot: settings.dataRoot,
    selectedDataRoot: null,
    accountDefaults: structuredClone(settings.accountDefaults),
    skippedAccount: false,
    syncAfterOnboarding: false,
    startupSyncDelayMinutes: settings.global.startupSyncDelayMinutes,
    closeBehavior: settings.global.closeBehavior,
    launchAtStartup: desktop.device.launchAtStartup,
    silentAutostart: desktop.device.silentAutostart,
    trayAvailable: desktop.trayAvailable,
  };
}

export function buildOnboardingRequest(
  draft: OnboardingDraft,
): CommitOnboardingStepRequest {
  const request = {
    expectedPortableRevision: draft.portableRevision,
    expectedDeviceRevision: draft.deviceRevision,
  };
  switch (draft.step) {
    case 0:
      return { ...request, step: { kind: "account", skipped: draft.skippedAccount } };
    case 1:
      return {
        ...request,
        step: {
          kind: "syncRange",
          accountDefaults: structuredClone(draft.accountDefaults),
          syncAfterOnboarding: draft.syncAfterOnboarding,
        },
      };
    case 2:
      if (
        !Number.isInteger(draft.startupSyncDelayMinutes) ||
        draft.startupSyncDelayMinutes < 0 ||
        draft.startupSyncDelayMinutes > 120
      ) {
        throw new Error("invalid_startup_sync_delay_minutes");
      }
      return {
        ...request,
        step: {
          kind: "automaticSync",
          accountDefaults: structuredClone(draft.accountDefaults),
          startupSyncDelayMinutes: draft.startupSyncDelayMinutes,
        },
      };
    case 3:
      if (draft.silentAutostart && !draft.launchAtStartup) {
        throw new Error("silent_autostart_requires_autostart");
      }
      if (draft.closeBehavior === "tray" && !draft.trayAvailable) {
        throw new Error("tray_unavailable");
      }
      return {
        ...request,
        step: {
          kind: "desktopBehavior",
          closeBehavior: draft.closeBehavior,
          launchAtStartup: draft.launchAtStartup,
          silentAutostart: draft.silentAutostart,
        },
      };
  }
}

export function applyOnboardingCommit(
  draft: OnboardingDraft,
  portableRevision: number,
  deviceRevision: number,
  onboardingStep: 0 | 1 | 2 | 3,
  completed: boolean,
): OnboardingDraft {
  const nextLocalStep = completed
    ? 3
    : (Math.min(3, draft.step + 1) as 0 | 1 | 2 | 3);
  return {
    ...draft,
    step: nextLocalStep,
    highestStep: Math.max(draft.highestStep, onboardingStep, nextLocalStep) as 0 | 1 | 2 | 3,
    portableRevision,
    deviceRevision,
  };
}

export function stepBack(draft: OnboardingDraft): OnboardingDraft {
  return draft.step <= 0 ? draft : { ...draft, step: (draft.step - 1) as 0 | 1 | 2 | 3 };
}

export function sourceControlState(
  capability: SourceCapability,
  rangeKind: SyncRangeKind,
): SourceControlState {
  return {
    rangeDisabled: !capability.supportedRanges.includes(rangeKind),
    scheduleDisabled: !capability.available,
    reason: capability.available ? null : capability.unavailableReason,
  };
}
