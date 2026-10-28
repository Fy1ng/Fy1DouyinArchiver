<script lang="ts">
  import { AlertCircle, ChevronLeft, ChevronRight, RefreshCw, X } from "lucide-svelte";
  import { onMount } from "svelte";
  import { getSyncBatch, listenAppEvent, listSyncBatchEntries, listSyncBatches } from "./app-api";
  import { syncIssueMessage } from "./scheduler";
  import type { SyncBatchEntry, SyncBatchView, SyncJobStatus, SyncSource } from "./types";

  export let accountId: number;

  const PAGE_SIZE = 30;
  let records: SyncBatchView[] = [];
  let total = 0;
  let page = 1;
  let loading = true;
  let errorMessage = "";
  let activeBatch: SyncBatchView | null = null;
  let activeEntries: SyncBatchEntry[] = [];
  let nextCursor: string | null = null;
  let hasMoreEntries = false;
  let detailsLoading = false;
  let lastAccountId = 0;
  let generation = 0;
  let detailsGeneration = 0;
  let eventRefreshPending = false;
  let eventRefreshRunning = false;

  $: totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  $: if (accountId !== lastAccountId) {
    lastAccountId = accountId;
    page = 1;
    activeBatch = null;
    void refresh();
  }

  async function refresh(): Promise<void> {
    const requestGeneration = ++generation;
    loading = true;
    errorMessage = "";
    try {
      const result = await listSyncBatches(accountId, PAGE_SIZE, (page - 1) * PAGE_SIZE);
      if (requestGeneration !== generation) return;
      records = result.records;
      total = result.total;
      if (page > Math.max(1, Math.ceil(total / PAGE_SIZE))) {
        page = Math.max(1, Math.ceil(total / PAGE_SIZE));
        await refresh();
      }
    } catch (error) {
      if (requestGeneration === generation) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (requestGeneration === generation) loading = false;
    }
  }

  async function goToPage(target: number): Promise<void> {
    page = Math.max(1, Math.min(totalPages, target));
    await refresh();
  }

  async function openBatch(batch: SyncBatchView): Promise<void> {
    activeBatch = batch;
    activeEntries = [];
    nextCursor = null;
    hasMoreEntries = false;
    await loadEntries(true);
  }

  async function loadEntries(reset: boolean, replaceInFlight = false): Promise<void> {
    if (!activeBatch || (detailsLoading && !replaceInFlight)) return;
    const requestGeneration = ++detailsGeneration;
    detailsLoading = true;
    errorMessage = "";
    try {
      const batchId = activeBatch.id;
      const result = await listSyncBatchEntries(batchId, reset ? null : nextCursor, 200);
      if (requestGeneration !== detailsGeneration || activeBatch?.id !== batchId) return;
      activeEntries = reset ? result.entries : [...activeEntries, ...result.entries];
      nextCursor = result.nextCursor || null;
      hasMoreEntries = result.hasMore;
      const nextBatch = await getSyncBatch(batchId);
      if (requestGeneration !== detailsGeneration || activeBatch?.id !== batchId) return;
      activeBatch = nextBatch;
    } catch (error) {
      if (requestGeneration === detailsGeneration) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (requestGeneration === detailsGeneration) detailsLoading = false;
    }
  }

  async function drainEventRefreshes(): Promise<void> {
    if (eventRefreshRunning) return;
    eventRefreshRunning = true;
    try {
      while (eventRefreshPending) {
        eventRefreshPending = false;
        await refresh();
        if (activeBatch) await loadEntries(true, true);
      }
    } finally {
      eventRefreshRunning = false;
      if (eventRefreshPending) void drainEventRefreshes();
    }
  }

  function scheduleEventRefresh(): void {
    eventRefreshPending = true;
    void drainEventRefreshes();
  }

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenAppEvent("sync:changed", scheduleEventRefresh).then((registered) => {
      if (disposed) registered();
      else unlisten = registered;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
      unlisten = undefined;
    };
  });

  function sourceLabel(source: SyncSource): string {
    if (source === "history") return "观看历史";
    if (source === "likes") return "喜欢";
    if (source === "favorites") return "收藏";
    return "抖音收藏夹";
  }

  function statusLabel(status: SyncJobStatus): string {
    if (status === "success") return "成功";
    if (status === "partial") return "部分完成";
    if (status === "failed") return "失败";
    if (status === "cancelled") return "已取消";
    if (status === "running") return "进行中";
    return "等待中";
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
</script>

<section class="sync-records-page" aria-label="同步记录">
  <div class="page-toolbar">
    <span>共 {total} 个同步批次</span>
    <button class="icon-button" type="button" title="刷新同步记录" disabled={loading} on:click={refresh}>
      <RefreshCw size={16} class={loading ? "spinning" : ""} />
    </button>
  </div>

  {#if errorMessage}
    <div class="overview-error"><AlertCircle size={16} /><span>{errorMessage}</span></div>
  {/if}

  <div class="sync-record-layout" class:with-details={activeBatch !== null}>
    <div class="sync-batch-table">
      <div class="sync-batch-row sync-batch-heading">
        <span>时间</span><span>范围</span><span>任务</span><span>结果</span><span></span>
      </div>
      {#if loading && records.length === 0}
        <div class="overview-empty">正在读取同步记录...</div>
      {:else if records.length === 0}
        <div class="overview-empty">尚无同步批次</div>
      {:else}
        {#each records as batch}
          <div class="sync-batch-row" class:active={activeBatch?.id === batch.id}>
            <span>{formatDate(batch.createdAt)}</span>
            <span>{batch.scope === "allActiveAccounts" ? "全部账号" : "当前账号"}</span>
            <span>{batch.counts.jobs} 个任务</span>
            <span class:failed={batch.status === "failed"}>{statusLabel(batch.status)}</span>
            <button type="button" on:click={() => openBatch(batch)}>详情</button>
          </div>
        {/each}
      {/if}
    </div>

    {#if activeBatch}
      <aside class="sync-batch-details">
        <header>
          <div><strong>批次 {activeBatch.id}</strong><small>{formatDate(activeBatch.createdAt)}</small></div>
          <button class="icon-button" type="button" title="关闭详情" on:click={() => (activeBatch = null)}><X size={15} /></button>
        </header>
        <div class="sync-batch-summary">
          <span>状态<strong>{statusLabel(activeBatch.status)}</strong></span>
          <span>任务<strong>{activeBatch.counts.jobs}</strong></span>
          <span>跳过<strong>{activeBatch.counts.skipped}</strong></span>
          <span>冲突<strong>{activeBatch.counts.conflicts}</strong></span>
        </div>
        <div class="sync-entry-list">
          {#if detailsLoading && activeEntries.length === 0}
            <div class="overview-empty">正在读取批次明细...</div>
          {:else}
            {#each activeEntries as entry}
              <article class:warning={entry.kind !== "job" || (entry.kind === "job" && entry.data.issues.length > 0)}>
                {#if entry.kind === "job"}
                  <div><strong>{sourceLabel(entry.data.source)}</strong><em>{statusLabel(entry.data.status)}</em></div>
                  <small>{entry.data.sourceInstanceKey || "默认来源"}</small>
                  {#each entry.data.issues as issue}<p>{syncIssueMessage(issue)}</p>{/each}
                {:else}
                  <div><strong>{sourceLabel(entry.data.source)}</strong><em>{entry.kind === "skipped" ? "已跳过" : "冲突"}</em></div>
                  <p>{entry.data.reason}</p>
                {/if}
              </article>
            {/each}
          {/if}
        </div>
        {#if hasMoreEntries}
          <button class="secondary sync-entry-more" type="button" disabled={detailsLoading} on:click={() => loadEntries(false)}>加载更多明细</button>
        {/if}
      </aside>
    {/if}
  </div>

  <div class="pagination-bar">
    <span>第 {page} / {totalPages} 页</span>
    <div class="pagination-actions">
      <button class="icon-button" type="button" title="上一页" disabled={loading || page <= 1} on:click={() => goToPage(page - 1)}><ChevronLeft size={16} /></button>
      <button class="icon-button" type="button" title="下一页" disabled={loading || page >= totalPages} on:click={() => goToPage(page + 1)}><ChevronRight size={16} /></button>
    </div>
  </div>
</section>
