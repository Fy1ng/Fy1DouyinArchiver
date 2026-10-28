import { describe, expect, test } from "vitest";
import {
  DEFAULT_DOWNLOAD_SETTINGS,
  normalizeDownloadSettings,
} from "../../app/src/download/settings.js";

describe("download settings", () => {
  test("normalizes invalid quality and trims the default directory", () => {
    expect(
      normalizeDownloadSettings({
        defaultDirectory: "  D:\\Archive  ",
        defaultQuality: "original",
      }),
    ).toMatchObject({
      defaultDirectory: "D:\\Archive",
      defaultQuality: "balanced",
    });
  });

  test("falls back to an empty default directory", () => {
    expect(normalizeDownloadSettings({ defaultDirectory: 42 }).defaultDirectory).toBe("");
  });

  test("keeps directory and quality prompts independent", () => {
    expect(
      normalizeDownloadSettings({
        askDirectoryEachJob: false,
        askQualityEachJob: true,
      }),
    ).toEqual({
      ...DEFAULT_DOWNLOAD_SETTINGS,
      askDirectoryEachJob: false,
      askQualityEachJob: true,
    });
  });
});
