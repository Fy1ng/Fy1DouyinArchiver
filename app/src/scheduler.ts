import type {
  EnqueueSyncResult,
  SchedulerInstancesPage,
  SchedulerInstanceView,
  SyncBatchEntriesPage,
  SyncBatchEntry,
  SyncBatchStatus,
  SyncJobIssue,
  SyncSource,
} from "./types";

export const ACTIVE_REFRESH_DELAY_MS = 500;
export const IDLE_REFRESH_DELAY_MS = 60_000;

export interface BatchEntriesState {
  batchId: number;
  entries: SyncBatchEntry[];
  cursor: string | null;
  hasMore: boolean;
}

export interface SchedulerInstancesState {
  accountId: number;
  source: SyncSource;
  portableSettingsRevision: number;
  registryRevision: number;
  instances: SchedulerInstanceView[];
  cursor: string | null;
  hasMore: boolean;
}

export function createBatchEntriesState(result: EnqueueSyncResult): BatchEntriesState | null {
  if (result.batchId === null) return null;
  return {
    batchId: result.batchId,
    entries: [...result.entries],
    cursor: result.nextCursor,
    hasMore: result.nextCursor !== null,
  };
}

export function appendBatchEntries(
  state: BatchEntriesState,
  page: SyncBatchEntriesPage,
): BatchEntriesState {
  if (state.batchId !== page.batchId) throw new Error("invalid_batch_cursor");
  return {
    ...state,
    entries: [...state.entries, ...page.entries],
    cursor: page.nextCursor,
    hasMore: page.hasMore,
  };
}

export function batchCursorForRefresh(
  state: BatchEntriesState,
  status: SyncBatchStatus,
  reloadFromStart: boolean,
): string | null {
  if (reloadFromStart) return null;
  return status === "queued" || status === "running" ? state.cursor : null;
}

export function createSchedulerInstancesState(
  page: SchedulerInstancesPage,
): SchedulerInstancesState {
  return {
    accountId: page.accountId,
    source: page.source,
    portableSettingsRevision: page.portableSettingsRevision,
    registryRevision: page.registryRevision,
    instances: [...page.instances],
    cursor: page.nextCursor,
    hasMore: page.hasMore,
  };
}

export function appendSchedulerInstances(
  state: SchedulerInstancesState,
  page: SchedulerInstancesPage,
): SchedulerInstancesState {
  if (
    state.accountId !== page.accountId ||
    state.source !== page.source ||
    state.portableSettingsRevision !== page.portableSettingsRevision ||
    state.registryRevision !== page.registryRevision
  ) {
    throw new Error("scheduler_instances_changed");
  }
  return {
    ...state,
    instances: [...state.instances, ...page.instances],
    cursor: page.nextCursor,
    hasMore: page.hasMore,
  };
}

export function syncIssueMessage(issue: SyncJobIssue): string {
  switch (issue.code) {
    case "archive_safety_record_cap_reached":
      return "已达到安全记录上限，本次仅保留已确认内容";
    case "archive_unconfirmed_end":
    case "history_incomplete":
    case "unconfirmed_end":
      return "未能确认远端列表已结束，已保留本次有效记录";
    case "remote_folder_discovery_incomplete":
      return "收藏夹目录发现未完成，本次仅归档已确认内容";
    default:
      return issue.message?.trim() ||
        (issue.severity === "incomplete"
          ? `同步未完整完成（${issue.code}）`
          : `同步提示（${issue.code}）`);
  }
}

export function refreshDelay(visible: boolean, hasActiveWork: boolean): number | null {
  if (!visible) return null;
  return hasActiveWork ? ACTIVE_REFRESH_DELAY_MS : IDLE_REFRESH_DELAY_MS;
}

export class SchedulerRefreshController {
  private timer: number | null = null;
  private visible = false;
  private hasActiveWork = false;

  constructor(private readonly refresh: () => Promise<void> | void) {}

  update(visible: boolean, hasActiveWork: boolean): void {
    if (this.visible === visible && this.hasActiveWork === hasActiveWork && this.timer !== null) {
      return;
    }
    this.visible = visible;
    this.hasActiveWork = hasActiveWork;
    this.restart();
  }

  async refreshNow(): Promise<void> {
    await this.refresh();
  }

  dispose(): void {
    if (this.timer !== null) window.clearInterval(this.timer);
    this.timer = null;
  }

  private restart(): void {
    this.dispose();
    const delay = refreshDelay(this.visible, this.hasActiveWork);
    if (delay === null) return;
    this.timer = window.setInterval(() => void this.refresh(), delay);
  }
}
