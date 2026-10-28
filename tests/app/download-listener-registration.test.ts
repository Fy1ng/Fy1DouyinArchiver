import { describe, expect, test, vi } from "vitest";
import { registerDownloadRefreshListeners } from "../../app/src/download/api.js";

describe("download event listener registration", () => {
  test("rolls back the first listener when the second registration fails", async () => {
    const unlistenJob = vi.fn();
    const listen = vi
      .fn()
      .mockResolvedValueOnce(unlistenJob)
      .mockRejectedValueOnce(new Error("item listener failed"));

    await expect(
      registerDownloadRefreshListeners(listen, vi.fn()),
    ).rejects.toThrow("item listener failed");

    expect(unlistenJob).toHaveBeenCalledOnce();
    expect(listen).toHaveBeenCalledTimes(2);
  });
});
