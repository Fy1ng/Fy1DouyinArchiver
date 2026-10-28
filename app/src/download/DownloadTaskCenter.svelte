<script lang="ts">
  import { onMount } from "svelte";
  import { cancelDownloadJob, getDownloadJob, listDownloadJobs, listenForDownloadRefresh, pauseDownloadJob, resumeDownloadJob, retryFailedDownload, revealDownloadItem, revealDownloadJob } from "./api";
  import { formatDownloadBytes } from "./presentation";
  import { installDownloadRefreshSubscription } from "./subscription";
  import type { DownloadJobDetail, DownloadJobSummary } from "./types";

  let jobs: DownloadJobSummary[] = [];
  let detail: DownloadJobDetail | null = null;
  let errorMessage = "";
  let refreshGeneration = 0;

  async function refresh(): Promise<void> {
    const requestGeneration = ++refreshGeneration;
    try {
      const nextJobs = await listDownloadJobs();
      if (requestGeneration !== refreshGeneration) return;
      jobs = nextJobs;
      const detailId = detail?.id;
      if (!detailId) return;
      const nextDetail = await getDownloadJob(detailId);
      if (requestGeneration !== refreshGeneration || detail?.id !== detailId) return;
      detail = nextDetail;
    } catch (error) {
      if (requestGeneration === refreshGeneration) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    }
  }

  async function selectJob(id: number): Promise<void> {
    detail = await getDownloadJob(id);
  }

  async function mutate(action: (id: number) => Promise<DownloadJobDetail>): Promise<void> {
    if (!detail) return;
    detail = await action(detail.id);
    await refresh();
  }

  onMount(() => {
    void refresh();
    return installDownloadRefreshSubscription(
      listenForDownloadRefresh,
      () => void refresh(),
    );
  });
</script>

<section class="task-center" aria-label="下载任务中心">
  <header><div><h2>下载任务</h2><p>事件只触发刷新，任务状态始终从后端重新读取。</p></div><button class="ghost" on:click={refresh}>刷新</button></header>
  {#if errorMessage}<div class="error-line">{errorMessage}</div>{/if}
  {#if jobs.length === 0}<div class="empty-state compact">暂无下载任务</div>{/if}
  <div class="task-list">
    {#each jobs as job}
      <button class:active={detail?.id === job.id} on:click={() => selectJob(job.id)}><strong>#{job.id} · {job.state}</strong><span>账号 #{job.accountId} · {job.completedCount}/{job.candidateCount} 完成 · {job.skippedCount} 跳过 · {job.failedCount} 失败</span></button>
    {/each}
  </div>
  {#if detail}
    <div class="task-detail">
      <div class="task-actions"><button on:click={() => mutate(pauseDownloadJob)}>暂停</button><button on:click={() => mutate(resumeDownloadJob)}>启动 / 继续</button><button on:click={() => mutate(cancelDownloadJob)}>取消</button><button on:click={() => mutate(retryFailedDownload)}>重试失败</button><button on:click={() => revealDownloadJob(detail!.id)}>打开根目录</button></div>
      <p class="muted">{formatDownloadBytes(detail.downloadedBytes)} / {formatDownloadBytes(detail.totalBytes)} · 平均 {formatDownloadBytes(detail.averageBytesPerSecond)}/s · {detail.activeAssetCount} 个资源传输中 · {detail.cancelledAssetCount} 个已取消</p>
      {#each detail.items as item}
        <div class="task-item"><div><strong>{item.title ?? item.awemeId}{detail.currentResolverItemId === item.id ? " · 正在解析" : ""}</strong><span>{item.state} · {item.completedAssetCount}/{item.totalRequiredAssetCount} 资源</span></div><button class="ghost" on:click={() => revealDownloadItem(item.id)}>打开目录</button></div>
        {#each detail.assets.filter((asset) => asset.itemId === item.id) as asset}
          <div class="task-item asset-row"><div><strong>{asset.stableKey}</strong><span>{asset.state} · {formatDownloadBytes(asset.downloadedBytes)}{asset.expectedSize ? ` / ${formatDownloadBytes(asset.expectedSize)}` : ""}{asset.lastErrorCode ? ` · ${asset.lastErrorCode}` : ""}</span></div><span>{asset.relativeFinalPath}</span></div>
        {/each}
      {/each}
    </div>
  {/if}
</section>
