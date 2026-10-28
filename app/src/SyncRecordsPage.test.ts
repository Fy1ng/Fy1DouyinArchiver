import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SyncRecordsPage from "./SyncRecordsPage.svelte";

const api = vi.hoisted(() => ({
  getSyncBatch: vi.fn(),
  listenAppEvent: vi.fn(),
  listSyncBatchEntries: vi.fn(),
  listSyncBatches: vi.fn(),
}));

vi.mock("./app-api", () => api);

const batch = {
  id: 77,
  trigger: "manual" as const,
  scope: "currentAccountAvailableSources" as const,
  status: "success" as const,
  createdAt: "2026-08-13T08:00:00.000Z",
  finishedAt: "2026-08-13T08:01:00.000Z",
  counts: { jobs: 1, skipped: 1, conflicts: 0 },
};

describe("同步记录页面", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.listenAppEvent.mockResolvedValue(vi.fn());
    api.listSyncBatches.mockResolvedValue({ records: [batch], total: 31 });
    api.getSyncBatch.mockResolvedValue(batch);
    api.listSyncBatchEntries.mockResolvedValue({
      batchId: 77,
      entries: [
        {
          kind: "job",
          data: {
            id: 23,
            accountId: 11,
            source: "history",
            sourceInstanceKey: "",
            trigger: "manual",
            status: "success",
            queuedAt: "2026-08-13T08:00:00.000Z",
            scheduledFor: null,
            startedAt: "2026-08-13T08:00:01.000Z",
            finishedAt: "2026-08-13T08:01:00.000Z",
            queuePosition: null,
            canCancel: false,
            issues: [],
          },
        },
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
      nextCursor: "",
      hasMore: false,
    });
  });

  afterEach(() => cleanup());

  it("分页读取账号同步批次", async () => {
    render(SyncRecordsPage, { accountId: 11 });

    await waitFor(() => expect(api.listSyncBatches).toHaveBeenCalledWith(11, 30, 0));
    expect(await screen.findByText("共 31 个同步批次")).toBeVisible();
    await fireEvent.click(screen.getByTitle("下一页"));
    await waitFor(() => expect(api.listSyncBatches).toHaveBeenLastCalledWith(11, 30, 30));
  });

  it("打开批次详情并呈现任务与跳过原因", async () => {
    render(SyncRecordsPage, { accountId: 11 });
    await fireEvent.click(await screen.findByRole("button", { name: "详情" }));

    await waitFor(() => expect(api.listSyncBatchEntries).toHaveBeenCalledWith(77, null, 200));
    expect(await screen.findByText("批次 77")).toBeVisible();
    expect(screen.getByText("观看历史")).toBeVisible();
    expect(screen.getByText("已跳过")).toBeVisible();
    expect(screen.getByText("collector_not_implemented")).toBeVisible();
    expect(api.getSyncBatch).toHaveBeenCalledWith(77);
  });

  it("同步状态事件刷新列表和当前详情且不关闭详情", async () => {
    const handlers = new Map<string, () => void>();
    const runningBatch = {
      ...batch,
      status: "running" as const,
      finishedAt: null,
      counts: { jobs: 1, skipped: 0, conflicts: 0 },
    };
    const completedBatch = {
      ...batch,
      status: "success" as const,
      counts: { jobs: 2, skipped: 0, conflicts: 0 },
    };
    api.listenAppEvent.mockImplementation(async (name: string, handler: () => void) => {
      handlers.set(name, handler);
      return vi.fn();
    });
    api.listSyncBatches.mockResolvedValue({ records: [runningBatch], total: 1 });
    api.getSyncBatch.mockResolvedValue(runningBatch);
    api.listSyncBatchEntries.mockResolvedValue({
      batchId: 77,
      entries: [
        {
          kind: "job",
          data: {
            id: 23,
            accountId: 11,
            source: "history",
            sourceInstanceKey: "",
            trigger: "manual",
            status: "running",
            queuedAt: "2026-08-13T08:00:00.000Z",
            scheduledFor: null,
            startedAt: "2026-08-13T08:00:01.000Z",
            finishedAt: null,
            queuePosition: null,
            canCancel: true,
            issues: [],
          },
        },
      ],
      nextCursor: "",
      hasMore: false,
    });

    render(SyncRecordsPage, { accountId: 11 });
    await fireEvent.click(await screen.findByRole("button", { name: "详情" }));
    expect(await screen.findByText("批次 77")).toBeVisible();
    await waitFor(() => expect(handlers.has("sync:changed")).toBe(true));

    api.listSyncBatches.mockResolvedValue({ records: [completedBatch], total: 1 });
    api.getSyncBatch.mockResolvedValue(completedBatch);
    api.listSyncBatchEntries.mockResolvedValue({
      batchId: 77,
      entries: [
        {
          kind: "job",
          data: {
            id: 23,
            accountId: 11,
            source: "history",
            sourceInstanceKey: "",
            trigger: "manual",
            status: "success",
            queuedAt: "2026-08-13T08:00:00.000Z",
            scheduledFor: null,
            startedAt: "2026-08-13T08:00:01.000Z",
            finishedAt: "2026-08-13T08:01:00.000Z",
            queuePosition: null,
            canCancel: false,
            issues: [],
          },
        },
      ],
      nextCursor: "",
      hasMore: false,
    });

    handlers.get("sync:changed")?.();

    await waitFor(() => expect(api.listSyncBatches).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(api.listSyncBatchEntries).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("批次 77")).toBeVisible();
    expect(screen.getAllByText("成功")).toHaveLength(3);
    expect(screen.getByText("2", { selector: ".sync-batch-summary strong" })).toBeVisible();
  });
});
