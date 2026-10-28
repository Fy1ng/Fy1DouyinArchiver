import type { DownloadQuality, DownloadSettings } from "./types.js";

export const DOWNLOAD_SETTINGS_STORAGE_KEY = "douyin-history-download-settings";

export const DEFAULT_DOWNLOAD_SETTINGS: DownloadSettings = {
  defaultDirectory: "",
  defaultQuality: "balanced",
  askDirectoryEachJob: true,
  askQualityEachJob: false,
};

const qualities = new Set<DownloadQuality>(["highest", "balanced", "space_saver"]);

export function normalizeDownloadSettings(value: unknown): DownloadSettings {
  const input = value && typeof value === "object" ? value as Record<string, unknown> : {};
  return {
    defaultDirectory:
      typeof input.defaultDirectory === "string" ? input.defaultDirectory.trim() : "",
    defaultQuality:
      typeof input.defaultQuality === "string" && qualities.has(input.defaultQuality as DownloadQuality)
        ? input.defaultQuality as DownloadQuality
        : DEFAULT_DOWNLOAD_SETTINGS.defaultQuality,
    askDirectoryEachJob:
      typeof input.askDirectoryEachJob === "boolean"
        ? input.askDirectoryEachJob
        : DEFAULT_DOWNLOAD_SETTINGS.askDirectoryEachJob,
    askQualityEachJob:
      typeof input.askQualityEachJob === "boolean"
        ? input.askQualityEachJob
        : DEFAULT_DOWNLOAD_SETTINGS.askQualityEachJob,
  };
}

export function loadDownloadSettings(storage: Pick<Storage, "getItem"> = localStorage): DownloadSettings {
  const raw = storage.getItem(DOWNLOAD_SETTINGS_STORAGE_KEY);
  if (!raw) return { ...DEFAULT_DOWNLOAD_SETTINGS };
  try {
    return normalizeDownloadSettings(JSON.parse(raw));
  } catch {
    return { ...DEFAULT_DOWNLOAD_SETTINGS };
  }
}

export function saveDownloadSettings(
  settings: DownloadSettings,
  storage: Pick<Storage, "setItem"> = localStorage,
): void {
  storage.setItem(DOWNLOAD_SETTINGS_STORAGE_KEY, JSON.stringify(normalizeDownloadSettings(settings)));
}
