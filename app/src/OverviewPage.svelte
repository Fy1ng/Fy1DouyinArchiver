<script lang="ts">
  import { AlertCircle, BarChart3, Clock3, RefreshCw, Users } from "lucide-svelte";
  import { getOverviewScope } from "./app-api";
  import { resolvePortableAccountScope } from "./account-scope";
  import SelectField from "./controls/SelectField.svelte";
  import type {
    AccountView,
    AccountScope,
    OverviewRange,
    OverviewSourceFilter,
    PortableAccountScope,
    OverviewView,
    SyncJobStatus,
  } from "./types";

  export let accounts: AccountView[];
  export let accountScope: PortableAccountScope;
  export let source: OverviewSourceFilter = "all";
  export let range: OverviewRange = "last30Days";
  export let includeRemoteRemoved = false;
  export let onOpenAnnualReview: () => void = () => undefined;

  let view: OverviewView | null = null;
  let loading = true;
  let errorMessage = "";
  let lastRequestKey = "";
  let generation = 0;

  $: requestKey = `${JSON.stringify(accountScope)}:${source}:${range}:${includeRemoteRemoved}`;
  $: if (requestKey !== lastRequestKey) {
    lastRequestKey = requestKey;
    void refresh();
  }
  $: trendMaximum = Math.max(
    1,
    ...(view?.trend.map((point) => point.history + point.likes + point.favorites) ?? [1]),
  );

  async function refresh(): Promise<void> {
    const requestGeneration = ++generation;
    loading = true;
    errorMessage = "";
    let runtimeScope: AccountScope;
    try {
      runtimeScope = resolvePortableAccountScope(accountScope, accounts);
    } catch {
      if (requestGeneration === generation) {
        view = null;
        loading = false;
        errorMessage = "概览账号范围已失效，请前往设置重新选择";
      }
      return;
    }
    try {
      const result = await getOverviewScope({
        accountScope: runtimeScope,
        source,
        range,
        includeRemoteRemoved,
      });
      if (requestGeneration === generation) view = result;
    } catch (error) {
      if (requestGeneration === generation) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (requestGeneration === generation) loading = false;
    }
  }

  function sourceLabel(value: OverviewSourceFilter): string {
    if (value === "history") return "观看历史";
    if (value === "likes") return "喜欢";
    if (value === "favorites") return "收藏";
    return "全部来源";
  }

  function statusLabel(status: SyncJobStatus): string {
    if (status === "success") return "成功";
    if (status === "partial") return "部分完成";
    if (status === "failed") return "失败";
    if (status === "cancelled") return "已取消";
    if (status === "running") return "进行中";
    return "等待中";
  }

  function formatDate(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return new Intl.DateTimeFormat("zh-CN", {
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  }
</script>

<section class="overview-page" aria-label="概览">
  <div class="page-toolbar overview-toolbar">
    <div class="overview-filters">
      <div class="select-field-label">
        <span>来源</span>
        <SelectField
          ariaLabel="来源"
          bind:value={source}
          options={[
            { value: "all", label: "全部来源" },
            { value: "history", label: "观看历史" },
            { value: "likes", label: "喜欢" },
            { value: "favorites", label: "收藏" },
          ]}
        />
      </div>
      <div class="select-field-label">
        <span>时间</span>
        <SelectField
          ariaLabel="时间"
          bind:value={range}
          options={[
            { value: "last7Days", label: "最近 7 天" },
            { value: "last30Days", label: "最近 30 天" },
            { value: "last90Days", label: "最近 90 天" },
            { value: "all", label: "全部时间" },
          ]}
        />
      </div>
      <label class="removed-filter">
        <input type="checkbox" bind:checked={includeRemoteRemoved} />
        <span>包含已从远端移除</span>
      </label>
    </div>
    <button class="icon-button" type="button" title="刷新概览" disabled={loading} on:click={refresh}>
      <RefreshCw size={16} class={loading ? "spinning" : ""} />
    </button>
    <button class="secondary" type="button" on:click={onOpenAnnualReview}>年度回顾</button>
  </div>

  {#if errorMessage}
    <div class="overview-error"><AlertCircle size={16} /><span>{errorMessage}</span></div>
  {:else if loading && !view}
    <div class="empty-state">正在汇总本地归档...</div>
  {:else if view}
    <div class="overview-metrics">
      <div><span>去重内容</span><strong>{view.uniqueContentCount}</strong><small>{sourceLabel(source)}</small></div>
      <div><span>观看记录</span><strong>{view.sourceCounts.history}</strong><small>观看事件</small></div>
      <div><span>喜欢归档</span><strong>{view.sourceCounts.likes}</strong><small>来源关系</small></div>
      <div><span>收藏归档</span><strong>{view.sourceCounts.favorites}</strong><small>来源关系</small></div>
    </div>

    {#if view.undatedArchiveCount > 0 && range !== "all"}
      <div class="overview-note">
        有 {view.undatedArchiveCount} 条喜欢或收藏缺少可靠远端时间，未纳入当前时间范围。
      </div>
    {/if}

    <div class="overview-grid">
      <section class="overview-section trend-section">
        <header><div><BarChart3 size={17} /><h2>每日新增</h2></div><span>{view.trend.length} 天</span></header>
        {#if view.trend.length === 0}
          <div class="overview-empty">当前范围没有可靠时间记录</div>
        {:else}
          <div class="trend-chart" aria-label="每日新增趋势">
            {#each view.trend as point}
              {@const total = point.history + point.likes + point.favorites}
              <div class="trend-column" title={`${point.date} · ${total} 条`}>
                <span class="trend-value">{total}</span>
                <div class="trend-track">
                  <i class="trend-bar" style={`height: ${Math.max(4, (total / trendMaximum) * 100)}%`}></i>
                </div>
                <small>{point.date.slice(5)}</small>
              </div>
            {/each}
          </div>
        {/if}
      </section>

      <section class="overview-section">
        <header><div><Users size={17} /><h2>作者排行</h2></div></header>
        {#if view.topAuthors.length === 0}
          <div class="overview-empty">暂无作者信息</div>
        {:else}
          <ol class="rank-list">
            {#each view.topAuthors as item, index}
              <li><span>{index + 1}</span><strong title={item.authorName}>{item.authorName}</strong><em>{item.contentCount}</em></li>
            {/each}
          </ol>
        {/if}
      </section>

      <section class="overview-section">
        <header><div><BarChart3 size={17} /><h2>内容类型</h2></div></header>
        {#if view.contentTypes.length === 0}
          <div class="overview-empty">暂无内容</div>
        {:else}
          <div class="type-breakdown">
            {#each view.contentTypes as item}
              <div><span>{item.contentType === "video" ? "视频" : item.contentType === "note" ? "图文" : "未知"}</span><strong>{item.contentCount}</strong></div>
            {/each}
          </div>
        {/if}
      </section>

      <section class="overview-section">
        <header><div><Clock3 size={17} /><h2>最近同步</h2></div></header>
        {#if view.recentSync.length === 0}
          <div class="overview-empty">尚无同步记录</div>
        {:else}
          <div class="recent-sync-list">
            {#each view.recentSync as item}
              <div>
                <span><strong>{sourceLabel(item.source)}</strong><small>{formatDate(item.finishedAt)}</small></span>
                <em class:failed={item.status === "failed"}>{statusLabel(item.status)}</em>
              </div>
            {/each}
          </div>
        {/if}
      </section>
    </div>
  {/if}
</section>
