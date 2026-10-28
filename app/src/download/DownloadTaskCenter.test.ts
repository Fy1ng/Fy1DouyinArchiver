import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import DownloadTaskCenter from "./DownloadTaskCenter.svelte";
import type { DownloadJobDetail, DownloadJobSummary } from "./types";

const api = vi.hoisted(() => ({
  cancelDownloadJob: vi.fn(),
  getDownloadJob: vi.fn(),
  listDownloadJobs: vi.fn(),
  listenForDownloadRefresh: vi.fn(),
  pauseDownloadJob: vi.fn(),
  resumeDownloadJob: vi.fn(),
  retryFailedDownload: vi.fn(),
  revealDownloadItem: vi.fn(),
  revealDownloadJob: vi.fn(),
}));

vi.mock("./api", () => api);

function summary(state: string, completedCount: number): DownloadJobSummary {
  return {
    id: 9,
    accountId: 11,
    state,
    destinationRoot: "D:\\Downloads",
    requestedQuality: "highest",
    candidateCount: 1,
    resolvedCount: completedCount,
    completedCount,
    skippedCount: 0,
    failedCount: 0,
    downloadedBytes: completedCount ? 1024 : 0,
    totalBytes: 1024,
    createdAt: "2026-08-16T00:00:00.000Z",
  };
}

function detail(state: string, itemState: string, completedCount: number): DownloadJobDetail {
  return {
    ...summary(state, completedCount),
    items: [
      {
        id: 21,
        awemeId: "7123456789012345678",
        title: "测试视频",
        contentType: "video",
        state: itemState,
        completedAssetCount: completedCount,
        totalRequiredAssetCount: 1,
      },
    ],
    assets: [],
    activeAssetCount: completedCount ? 0 : 1,
    cancelledAssetCount: 0,
    averageBytesPerSecond: completedCount ? 512 : 0,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

describe("下载任务中心", () => {
  let emitRefresh: (() => void) | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    emitRefresh = undefined;
    api.listenForDownloadRefresh.mockImplementation(async (handler: () => void) => {
      emitRefresh = handler;
      return vi.fn();
    });
  });

  afterEach(() => cleanup());

  it("状态事件重叠时保留打开的详情并展示最新列表与详情", async () => {
    const staleList = deferred<DownloadJobSummary[]>();
    api.listDownloadJobs
      .mockResolvedValueOnce([summary("running", 0)])
      .mockReturnValueOnce(staleList.promise)
      .mockResolvedValueOnce([summary("completed", 1)]);
    api.getDownloadJob
      .mockResolvedValueOnce(detail("running", "resolving", 0))
      .mockResolvedValueOnce(detail("completed", "completed", 1))
      .mockResolvedValueOnce(detail("running", "resolving", 0));

    render(DownloadTaskCenter);
    await fireEvent.click(await screen.findByRole("button", { name: /#9 · running/ }));
    expect(await screen.findByText("resolving · 0/1 资源")).toBeVisible();
    await waitFor(() => expect(emitRefresh).toBeTypeOf("function"));

    emitRefresh?.();
    await waitFor(() => expect(api.listDownloadJobs).toHaveBeenCalledTimes(2));
    emitRefresh?.();

    expect(await screen.findByRole("button", { name: /#9 · completed/ })).toBeVisible();
    expect(await screen.findByText("completed · 1/1 资源")).toBeVisible();

    staleList.resolve([summary("running", 0)]);
    await staleList.promise;
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(api.getDownloadJob).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("button", { name: /#9 · completed/ })).toBeVisible();
    expect(screen.getByText("completed · 1/1 资源")).toBeVisible();
    expect(screen.getByRole("button", { name: "打开根目录" })).toBeVisible();
  });
});
