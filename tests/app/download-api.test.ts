import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("下载 IPC", () => {
  beforeEach(() => {
    invoke.mockReset().mockResolvedValue({});
    Object.assign(globalThis, { window: { __TAURI_INTERNALS__: {} } });
  });

  it("预览和创建任务都携带当前账号", async () => {
    const { createDownloadJob, previewDownloadSelection } = await import(
      "../../app/src/download/api.js"
    );
    const selection = { kind: "checked" as const, watch_record_ids: [3, 7] };

    await previewDownloadSelection(11, selection);
    await createDownloadJob({
      accountId: 11,
      selection,
      destinationRoot: "D:\\Douyin",
      requestedQuality: "balanced",
    });

    expect(invoke).toHaveBeenNthCalledWith(1, "download_preview_selection", {
      accountId: 11,
      selection,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "download_create_job", {
      request: {
        accountId: 11,
        selection,
        destinationRoot: "D:\\Douyin",
        requestedQuality: "balanced",
      },
    });
  });
});
