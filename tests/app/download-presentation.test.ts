import { describe, expect, it } from "vitest";
import {
  formatDownloadBytes,
  summarizeDownloadContentTypes,
} from "../../app/src/download/presentation.js";

describe("download presentation", () => {
  it("renders backend content-type counts in stable key order", () => {
    expect(
      summarizeDownloadContentTypes({ video: 2, unknown: 1, note: 3 }),
    ).toBe("note 3 · unknown 1 · video 2");
    expect(summarizeDownloadContentTypes({})).toBe("");
  });

  it("formats byte counts for task and asset observability", () => {
    expect(formatDownloadBytes(999)).toBe("999 B");
    expect(formatDownloadBytes(1_536)).toBe("1.5 KiB");
    expect(formatDownloadBytes(2 * 1024 ** 2)).toBe("2.0 MiB");
  });
});
