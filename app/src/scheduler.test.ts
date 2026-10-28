import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  SchedulerRefreshController,
  appendBatchEntries,
  appendSchedulerInstances,
  batchCursorForRefresh,
  createBatchEntriesState,
  createSchedulerInstancesState,
  refreshDelay,
  syncIssueMessage,
} from "./scheduler";
import type {
  EnqueueSyncResult,
  SchedulerInstancesPage,
  SyncBatchEntriesPage,
} from "./types";

describe("scheduler refresh policy", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("uses 500ms only while visible work is queued or running", () => {
    expect(refreshDelay(true, true)).toBe(500);
    expect(refreshDelay(true, false)).toBe(60_000);
    expect(refreshDelay(false, true)).toBeNull();
    expect(refreshDelay(false, false)).toBeNull();
  });

  it("restarts when visibility or active work changes and stops cleanly", async () => {
    const refresh = vi.fn(async () => undefined);
    const controller = new SchedulerRefreshController(refresh);
    controller.update(true, true);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(refresh).toHaveBeenCalledTimes(2);

    controller.update(false, true);
    await vi.advanceTimersByTimeAsync(2_000);
    expect(refresh).toHaveBeenCalledTimes(2);

    await controller.refreshNow();
    expect(refresh).toHaveBeenCalledTimes(3);
    controller.dispose();
  });

  it("polls visible idle state no faster than 60 seconds and never polls while hidden", async () => {
    const refresh = vi.fn(async () => undefined);
    const controller = new SchedulerRefreshController(refresh);

    controller.update(true, false);
    await vi.advanceTimersByTimeAsync(59_999);
    expect(refresh).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(refresh).toHaveBeenCalledTimes(1);

    controller.update(false, false);
    await vi.advanceTimersByTimeAsync(120_000);
    expect(refresh).toHaveBeenCalledTimes(1);
    controller.dispose();
  });
});

const enqueueResult = (): EnqueueSyncResult => ({
  batchId: 17,
  counts: { jobs: 1, skipped: 0, conflicts: 0 },
  entries: [
    {
      kind: "job",
      data: {
        id: 31,
        accountId: 11,
        source: "history",
        sourceInstanceKey: "",
        trigger: "manual",
        status: "queued",
        queuedAt: "2026-08-12T00:00:00.000Z",
        scheduledFor: null,
        startedAt: null,
        finishedAt: null,
        queuePosition: 1,
        canCancel: true,
        issues: [],
      },
    },
  ],
  nextCursor: "cursor-1",
});

describe("scheduler pagination state", () => {
  it("keeps probing an exhausted active batch from its last cursor", () => {
    const initial = createBatchEntriesState(enqueueResult());
    if (!initial) throw new Error("expected batch state");
    const exhausted: SyncBatchEntriesPage = {
      batchId: 17,
      entries: [],
      nextCursor: "cursor-1",
      hasMore: false,
    };
    const state = appendBatchEntries(initial, exhausted);

    expect(batchCursorForRefresh(state, "running", false)).toBe("cursor-1");
    expect(batchCursorForRefresh(state, "success", false)).toBeNull();
    expect(batchCursorForRefresh(state, "success", true)).toBeNull();
  });

  it("appends entries discovered after a previously exhausted cursor", () => {
    const initial = createBatchEntriesState(enqueueResult());
    if (!initial) throw new Error("expected batch state");
    const exhausted = appendBatchEntries(initial, {
      batchId: 17,
      entries: [],
      nextCursor: "cursor-1",
      hasMore: false,
    });
    const appended = appendBatchEntries(exhausted, {
      batchId: 17,
      entries: [
        {
          kind: "skipped",
          data: {
            accountId: 11,
            source: "likes",
            sourceInstanceKey: "",
            phase: "discoveryExpansion",
            reason: "collector_not_implemented",
            requestedPayloadSha256: null,
          },
        },
      ],
      nextCursor: "cursor-2",
      hasMore: false,
    });

    expect(appended.entries).toHaveLength(2);
    expect(appended.cursor).toBe("cursor-2");
  });

  it("rejects scheduler instance pages whose frozen revisions changed", () => {
    const first: SchedulerInstancesPage = {
      accountId: 11,
      source: "remoteFolder",
      portableSettingsRevision: 4,
      registryRevision: 8,
      instances: [
        {
          remoteFolderId: "folder-a",
          name: "收藏夹 A",
          range: { kind: "all" },
          schedule: { enabled: true, intervalMinutes: 1_440 },
        },
      ],
      nextCursor: "instance-cursor-1",
      hasMore: true,
    };
    const state = createSchedulerInstancesState(first);

    expect(() =>
      appendSchedulerInstances(state, {
        ...first,
        portableSettingsRevision: 5,
        instances: [],
        nextCursor: null,
        hasMore: false,
      }),
    ).toThrow("scheduler_instances_changed");
  });

  it("renders stable Chinese labels for incomplete sync issues", () => {
    expect(
      syncIssueMessage({
        phase: "collection",
        code: "archive_unconfirmed_end",
        severity: "incomplete",
        message: null,
      }),
    ).toBe("未能确认远端列表已结束，已保留本次有效记录");
    expect(
      syncIssueMessage({
        phase: "discovery",
        code: "remote_folder_discovery_incomplete",
        severity: "incomplete",
        message: null,
      }),
    ).toBe("收藏夹目录发现未完成，本次仅归档已确认内容");
  });
});
