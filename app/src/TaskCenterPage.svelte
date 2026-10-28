<script lang="ts">
  import { AlertCircle, ChevronLeft, ChevronRight, Download, RefreshCw, RotateCcw, X } from "lucide-svelte";
  import { onMount } from "svelte";
  import {
    cancelSync,
    cancelSyncBatch,
    getSyncBatch,
    listenAppEvent,
    listSyncBatchEntries,
    listTaskSummaries,
  } from "./app-api";
  import { resolvePortableAccountScope } from "./account-scope";
  import {
    cancelDownloadJob,
    getDownloadJob,
    listenForDownloadRefresh,
    pauseDownloadJob,
    resumeDownloadJob,
    retryFailedDownload,
  } from "./download/api";
  import { formatDownloadBytes } from "./download/presentation";
  import { syncIssueMessage } from "./scheduler";
  import type {
    AccountView,
    PortableAccountScope,
    SyncBatchEntry,
    SyncBatchView,
    SyncJobView,
    SyncSource,
    TaskSummary,
    TaskSummaryStatus,
  } from "./types";
  import type { DownloadJobDetail } from "./download/types";

  export let accounts: AccountView[] = [];
  export let accountScope: PortableAccountScope = { kind: "all" };

  const PAGE_SIZE = 30;
  let records: TaskSummary[] = [];
  let total = 0;
  let page = 1;
  let loading = true;
  let errorMessage = "";
  let activeTask: TaskSummary | null = null;
  let activeSyncBatch: SyncBatchView | null = null;
  let activeSyncEntries: SyncBatchEntry[] = [];
  let activeDownload: DownloadJobDetail | null = null;
  let detailLoading = false;
  let requestGeneration = 0;
  let detailGeneration = 0;
  let lastScopeKey = "";
  let refreshPending = false;
  let refreshRunning = false;

  $: totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  $: scopeKey = JSON.stringify(accountScope);
  $: if (scopeKey !== lastScopeKey) {
    lastScopeKey = scopeKey;
    page = 1;
    activeTask = null;
    activeSyncBatch = null;
    activeSyncEntries = [];
    activeDownload = null;
    void refresh();
  }

  async function resolveScope(): Promise<ReturnType<typeof resolvePortableAccountScope>> {
    return resolvePortableAccountScope(accountScope, accounts);
  }

  async function refresh(): Promise<void> {
    const generation = ++requestGeneration;
    loading = true;
    errorMessage = "";
    try {
      const runtimeScope = await resolveScope();
      const result = await listTaskSummaries(runtimeScope, PAGE_SIZE, (page - 1) * PAGE_SIZE);
      if (generation !== requestGeneration) return;
      records = result.records;
      total = result.total;
      if (page > Math.max(1, Math.ceil(total / PAGE_SIZE))) {
        page = Math.max(1, Math.ceil(total / PAGE_SIZE));
        await refresh();
      }
    } catch (error) {
      if (generation === requestGeneration) {
        records = [];
        total = 0;
        errorMessage = error instanceof Error ? error.message : String(error);
        if (errorMessage === "account_scope_invalid") {
          errorMessage = "任务范围已失效，请前往设置重新选择账号";
        }
      }
    } finally {
      if (generation === requestGeneration) loading = false;
    }
  }

  async function goToPage(target: number): Promise<void> {
    page = Math.max(1, Math.min(totalPages, target));
    await refresh();
  }

  async function selectTask(task: TaskSummary): Promise<void> {
    activeTask = task;
    activeSyncBatch = null;
    activeSyncEntries = [];
    activeDownload = null;
    await loadDetail();
  }

  async function loadDetail(): Promise<void> {
    if (!activeTask) return;
    const taskKey = activeTask.key;
    const generation = ++detailGeneration;
    detailLoading = true;
    errorMessage = "";
    try {
      if (activeTask.kind === "syncBatch") {
        const [batch, entries] = await Promise.all([
          getSyncBatch(activeTask.id),
          listSyncBatchEntries(activeTask.id, null, 200),
        ]);
        if (generation !== detailGeneration || activeTask?.key !== taskKey) return;
        activeSyncBatch = batch;
        activeSyncEntries = entries.entries;
      } else {
        const detail = await getDownloadJob(activeTask.id);
        if (generation !== detailGeneration || activeTask?.key !== taskKey) return;
        activeDownload = detail;
      }
    } catch (error) {
      if (generation === detailGeneration && activeTask?.key === taskKey) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (generation === detailGeneration) detailLoading = false;
    }
  }

  async function refreshAll(): Promise<void> {
    await refresh();
    if (activeTask) await loadDetail();
  }

  async function drainRefreshes(): Promise<void> {
    if (refreshRunning) return;
    refreshRunning = true;
    try {
      while (refreshPending) {
        refreshPending = false;
        await refreshAll();
      }
    } finally {
      refreshRunning = false;
      if (refreshPending) void drainRefreshes();
    }
  }

  function scheduleRefresh(): void {
    refreshPending = true;
    void drainRefreshes();
  }

  async function cancelSyncJob(job: SyncJobView): Promise<void> {
    if (!job.canCancel) return;
    await cancelSync(job.accountId, job.id);
    scheduleRefresh();
  }

  async function cancelBatch(): Promise<void> {
    if (!activeSyncBatch) return;
    await cancelSyncBatch(activeSyncBatch.id);
    scheduleRefresh();
  }

  async function mutateDownload(
    action: (id: number) => Promise<DownloadJobDetail>,
  ): Promise<void> {
    if (!activeDownload) return;
    activeDownload = await action(activeDownload.id);
    scheduleRefresh();
  }

  onMount(() => {
    let disposed = false;
    let unlistenSync: (() => void) | undefined;
    let unlistenDownload: (() => void) | undefined;
    void listenAppEvent("sync:changed", scheduleRefresh).then((unlisten) => {
      if (disposed) unlisten();
      else unlistenSync = unlisten;
    }).catch(() => {});
    void listenForDownloadRefresh(scheduleRefresh).then((unlisten) => {
      if (disposed) unlisten();
      else unlistenDownload = unlisten;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlistenSync?.();
      unlistenDownload?.();
    };
  });

  function accountLabel(accountId: number | null): string {
    if (accountId === null) return "多个账号";
    const account = accounts.find((item) => item.id === accountId);
    return account?.nickname ?? account?.uid ?? `账号 #${accountId}`;
  }

  function sourceLabel(source: SyncSource | null): string {
    if (source === "history") return "观看历史";
    if (source === "likes") return "喜欢";
    if (source === "favorites") return "收藏";
    if (source === "remoteFolder") return "抖音收藏夹";
    return "全部来源";
  }

  function statusLabel(status: TaskSummaryStatus): string {
    const labels: Record<TaskSummaryStatus, string> = {
      queued: "等待中",
      running: "进行中",
      paused: "已暂停",
      interrupted: "已中断",
      success: "成功",
      partial: "部分完成",
      failed: "失败",
      completed: "已完成",
      completedWithErrors: "完成但有错误",
      cancelled: "已取消",
    };
    return labels[status];
  }

  function formatDate(value: string | null): string {
    if (!value) return "-";
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return new Intl.DateTimeFormat("zh-CN", {
      year: "2-digit",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  }

  function taskTitle(task: TaskSummary): string {
    return task.kind === "syncBatch" ? `同步批次 #${task.id}` : `下载任务 #${task.id}`;
  }
</script>

<section class="task-center-page" aria-label="任务中心">
  <div class="task-center-toolbar">
    <div>
      <h2>任务中心</h2>
      <p>同步批次与下载任务统一按创建时间排列，详情会随状态事件刷新。</p>
    </div>
    <button class="icon-button" type="button" title="刷新任务中心" aria-label="刷新任务中心" disabled={loading} on:click={refresh}>
      <RefreshCw size={16} class={loading ? "spinning" : ""} />
    </button>
  </div>

  {#if errorMessage}
    <div class="overview-error" role="alert"><AlertCircle size={16} /><span>{errorMessage}</span></div>
  {/if}

  <div class="task-center-layout" class:with-details={activeTask !== null}>
    <div class="task-summary-list">
      <div class="task-summary-heading"><span>任务</span><span>账号 / 来源</span><span>状态</span><span>时间</span></div>
      {#if loading && records.length === 0}
        <div class="overview-empty">正在读取任务...</div>
      {:else if records.length === 0}
        <div class="overview-empty">尚无同步或下载任务</div>
      {:else}
        {#each records as task}
          <button class="task-summary-row" class:active={activeTask?.key === task.key} type="button" on:click={() => selectTask(task)}>
            <span class="task-summary-main">
              {#if task.kind === "downloadJob"}<Download size={15} />{:else}<RotateCcw size={15} />{/if}
              <strong>{taskTitle(task)}</strong>
            </span>
            <span><strong>{accountLabel(task.accountId)}</strong><small>{sourceLabel(task.source)}</small></span>
            <span class:task-failed={task.status === "failed" || task.status === "completedWithErrors"}>{statusLabel(task.status)}</span>
            <span>{formatDate(task.createdAt)}</span>
          </button>
        {/each}
      {/if}
    </div>

    {#if activeTask}
      <aside class="task-center-detail" aria-label="任务详情">
        <header>
          <div><strong>{taskTitle(activeTask)}</strong><small>{accountLabel(activeTask.accountId)} · {sourceLabel(activeTask.source)}</small></div>
          <button class="icon-button" type="button" title="关闭详情" aria-label="关闭详情" on:click={() => { activeTask = null; activeSyncBatch = null; activeDownload = null; }}><X size={15} /></button>
        </header>
        {#if detailLoading && !activeSyncBatch && !activeDownload}
          <div class="overview-empty">正在读取任务详情...</div>
        {:else if activeSyncBatch}
          {#if activeSyncBatch.status === "queued" || activeSyncBatch.status === "running"}
            <div class="task-detail-actions"><button class="danger" type="button" on:click={cancelBatch}>取消整个批次</button></div>
          {/if}
          <div class="task-detail-summary">
            <span>状态<strong>{statusLabel(activeSyncBatch.status)}</strong></span>
            <span>任务<strong>{activeSyncBatch.counts.jobs}</strong></span>
            <span>跳过<strong>{activeSyncBatch.counts.skipped}</strong></span>
            <span>冲突<strong>{activeSyncBatch.counts.conflicts}</strong></span>
          </div>
          <div class="task-detail-list">
            {#each activeSyncEntries as entry}
              {#if entry.kind === "job"}
                <article class:warning={entry.data.issues.length > 0}>
                  <div><strong>{sourceLabel(entry.data.source)}</strong><em>{statusLabel(entry.data.status)}</em></div>
                  <small>{accountLabel(entry.data.accountId)} · {entry.data.sourceInstanceKey || "默认来源"}</small>
                  {#each entry.data.issues as issue}<p>{syncIssueMessage(issue)}</p>{/each}
                  {#if entry.data.canCancel}<button class="secondary" type="button" on:click={() => cancelSyncJob(entry.data)}>取消此任务</button>{/if}
                </article>
              {:else}
                <article class="warning"><div><strong>{sourceLabel(entry.data.source)}</strong><em>{entry.kind === "skipped" ? "已跳过" : "冲突"}</em></div><p>{entry.data.reason}</p></article>
              {/if}
            {/each}
          </div>
        {:else if activeDownload}
          <div class="task-detail-actions">
            {#if activeDownload.state === "running"}<button class="secondary" type="button" on:click={() => mutateDownload(pauseDownloadJob)}>暂停</button>{/if}
            {#if ["paused", "interrupted"].includes(activeDownload.state)}<button class="secondary" type="button" on:click={() => mutateDownload(resumeDownloadJob)}>继续</button>{/if}
            {#if ["queued", "running", "paused", "interrupted"].includes(activeDownload.state)}<button class="danger" type="button" on:click={() => mutateDownload(cancelDownloadJob)}>取消</button>{/if}
            {#if activeDownload.failedCount > 0}<button class="secondary" type="button" on:click={() => mutateDownload(retryFailedDownload)}>重试失败</button>{/if}
          </div>
          <p class="task-detail-meta">{formatDownloadBytes(activeDownload.downloadedBytes)} / {formatDownloadBytes(activeDownload.totalBytes)} · {activeDownload.completedCount}/{activeDownload.candidateCount} 完成 · {activeDownload.failedCount} 失败</p>
          <div class="task-detail-list">
            {#each activeDownload.items as item}
              <article class:warning={item.state === "failed"}>
                <div><strong>{item.title ?? item.awemeId}</strong><em>{item.state}</em></div>
                <small>{item.completedAssetCount}/{item.totalRequiredAssetCount} 个资源{item.errorCode ? ` · ${item.errorCode}` : ""}</small>
              </article>
            {/each}
          </div>
        {/if}
      </aside>
    {/if}
  </div>

  <div class="pagination-bar">
    <span>共 {total} 个任务 · 第 {page} / {totalPages} 页</span>
    <div class="pagination-actions">
      <button class="icon-button" type="button" title="上一页" aria-label="上一页" disabled={loading || page <= 1} on:click={() => goToPage(page - 1)}><ChevronLeft size={16} /></button>
      <button class="icon-button" type="button" title="下一页" aria-label="下一页" disabled={loading || page >= totalPages} on:click={() => goToPage(page + 1)}><ChevronRight size={16} /></button>
    </div>
  </div>
</section>
