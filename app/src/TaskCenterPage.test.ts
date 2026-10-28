import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import TaskCenterPage from "./TaskCenterPage.svelte";

const api = vi.hoisted(() => ({
  cancelSync: vi.fn(),
  cancelSyncBatch: vi.fn(),
  getSyncBatch: vi.fn(),
  listenAppEvent: vi.fn(),
  listSyncBatchEntries: vi.fn(),
  listTaskSummaries: vi.fn(),
}));
const downloadApi = vi.hoisted(() => ({
  cancelDownloadJob: vi.fn(),
  getDownloadJob: vi.fn(),
  listenForDownloadRefresh: vi.fn(),
  pauseDownloadJob: vi.fn(),
  resumeDownloadJob: vi.fn(),
  retryFailedDownload: vi.fn(),
}));

vi.mock("./app-api", () => api);
vi.mock("./download/api", () => downloadApi);

const accounts = [{
  id: 11,
  accountKey: "sec:a",
  profileKey: "profile-a",
  nickname: "测试账号",
  status: "active" as const,
  createdAt: "2026-08-19T00:00:00Z",
  updatedAt: "2026-08-19T00:00:00Z",
}];

const syncTask = {
  key: "sync:7",
  kind: "syncBatch" as const,
  id: 7,
  accountId: 11,
  source: "history" as const,
  status: "running" as const,
  createdAt: "2026-08-19T12:00:00Z",
  finishedAt: null,
  itemCount: 1,
  completedCount: 0,
  failedCount: 0,
  errorCode: null,
  canCancel: true,
};

const downloadTask = {
  key: "download:9",
  kind: "downloadJob" as const,
  id: 9,
  accountId: 11,
  source: null,
  status: "completed" as const,
  createdAt: "2026-08-19T11:00:00Z",
  finishedAt: "2026-08-19T11:01:00Z",
  itemCount: 1,
  completedCount: 1,
  failedCount: 0,
  errorCode: null,
  canCancel: false,
};

describe("统一任务中心", () => {
  let eventHandlers = new Map<string, () => void>();

  beforeEach(() => {
    vi.clearAllMocks();
    eventHandlers = new Map();
    api.listenAppEvent.mockImplementation(async (name: string, handler: () => void) => {
      eventHandlers.set(name, handler);
      return vi.fn();
    });
    downloadApi.listenForDownloadRefresh.mockResolvedValue(vi.fn());
    api.listTaskSummaries.mockResolvedValue({ records: [syncTask, downloadTask], total: 2 });
    api.getSyncBatch.mockResolvedValue({
      id: 7,
      trigger: "manual",
      scope: "currentAccountAvailableSources",
      status: "running",
      createdAt: syncTask.createdAt,
      finishedAt: null,
      counts: { jobs: 1, skipped: 0, conflicts: 0 },
    });
    api.listSyncBatchEntries.mockResolvedValue({ batchId: 7, entries: [], nextCursor: "", hasMore: false });
    downloadApi.getDownloadJob.mockResolvedValue({
      ...downloadTask,
      destinationRoot: "D:/media",
      requestedQuality: "highest",
      candidateCount: 1,
      resolvedCount: 1,
      skippedCount: 0,
      failedCount: 0,
      downloadedBytes: 100,
      totalBytes: 100,
      items: [],
      assets: [],
      activeAssetCount: 0,
      cancelledAssetCount: 0,
      averageBytesPerSecond: 100,
    });
  });

  afterEach(() => cleanup());

  it("按运行时账号范围加载同步与下载任务并打开对应详情", async () => {
    render(TaskCenterPage, { accounts, accountScope: { kind: "selected", accountKeys: ["sec:a"] } });
    await waitFor(() => expect(api.listTaskSummaries).toHaveBeenCalledWith({ kind: "selected", accountIds: [11] }, 30, 0));
    expect(await screen.findByText("同步批次 #7")).toBeVisible();
    expect(screen.getByText("下载任务 #9")).toBeVisible();

    await fireEvent.click(screen.getByRole("button", { name: /下载任务 #9/ }));
    await waitFor(() => expect(downloadApi.getDownloadJob).toHaveBeenCalledWith(9));
    expect(await screen.findByText(/1\/1 完成/)).toBeVisible();
  });

  it("范围失效时不扩大为全部账号", async () => {
    render(TaskCenterPage, { accounts, accountScope: { kind: "selected", accountKeys: ["missing"] } });
    expect(await screen.findByText("任务范围已失效，请前往设置重新选择账号")).toBeVisible();
    expect(api.listTaskSummaries).not.toHaveBeenCalled();
  });

  it("状态事件刷新列表和详情时不关闭当前抽屉", async () => {
    render(TaskCenterPage, { accounts, accountScope: { kind: "all" } });
    await fireEvent.click(await screen.findByRole("button", { name: /同步批次 #7/ }));
    await waitFor(() => expect(api.getSyncBatch).toHaveBeenCalledWith(7));

    api.listTaskSummaries.mockResolvedValue({ records: [{ ...syncTask, status: "success", canCancel: false }], total: 1 });
    api.getSyncBatch.mockResolvedValue({
      id: 7,
      trigger: "manual",
      scope: "currentAccountAvailableSources",
      status: "success",
      createdAt: syncTask.createdAt,
      finishedAt: "2026-08-19T12:01:00Z",
      counts: { jobs: 1, skipped: 0, conflicts: 0 },
    });
    await waitFor(() => expect(eventHandlers.has("sync:changed")).toBe(true));
    eventHandlers.get("sync:changed")?.();

    await waitFor(() => expect(api.listTaskSummaries).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("成功")).toBeVisible();
    expect(screen.getByRole("region", { name: "任务中心" })).toBeVisible();
  });
});
