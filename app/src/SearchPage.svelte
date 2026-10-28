<script lang="ts">
  import { onMount } from "svelte";
  import {
    AlertCircle,
    ChevronLeft,
    ChevronRight,
    RefreshCw,
    Search,
  } from "lucide-svelte";
  import { listLocalFolders, openContentUrl, searchLibraryContent } from "./app-api";
  import { resolvePortableAccountScope } from "./account-scope";
  import CoverImage from "./CoverImage.svelte";
  import SelectField from "./controls/SelectField.svelte";
  import { formatLikeCount, formatWatchProgress } from "./formatters";
  import type {
    AccountScope,
    AccountView,
    LibraryContentRecordViewV1,
    LikeCountFormat,
    LocalFolderView,
    PortableAccountScope,
    SearchContentQueryV1,
    SearchRemoteStateV1,
  } from "./types";

  export let accounts: AccountView[];
  export let accountScope: PortableAccountScope;
  export let likeCountFormat: LikeCountFormat;
  export let coverCacheEnabled: boolean;

  const PAGE_SIZE = 80;
  let query = "";
  let selectedAccountId = "all";
  let includeHistory = true;
  let includeLikes = true;
  let includeFavorites = true;
  let contentType = "all";
  let dateFrom = "";
  let dateTo = "";
  let remoteState: SearchRemoteStateV1 = "active";
  let localFolderId = "all";
  let folders: LocalFolderView[] = [];
  let records: LibraryContentRecordViewV1[] = [];
  let total = 0;
  let page = 1;
  let loading = false;
  let errorMessage = "";
  let runtimeScope: AccountScope | null = null;
  let visibleAccounts: AccountView[] = [];
  let generation = 0;

  $: totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  $: pageStart = total === 0 ? 0 : (page - 1) * PAGE_SIZE + 1;
  $: pageEnd = Math.min(total, page * PAGE_SIZE);

  onMount(() => {
    try {
      runtimeScope = resolvePortableAccountScope(accountScope, accounts);
      visibleAccounts = runtimeScope.kind === "all"
        ? accounts
        : accounts.filter((account) => runtimeScope?.kind === "selected" && runtimeScope.accountIds.includes(account.id));
      void loadFolders();
      void search(1);
    } catch {
      errorMessage = "搜索账号范围已失效，请前往设置重新选择";
    }
  });

  async function loadFolders(): Promise<void> {
    const results = await Promise.all(
      visibleAccounts.map((account) => listLocalFolders(account.id).catch(() => [])),
    );
    const unique = new Map<number, LocalFolderView>();
    for (const folder of results.flat()) unique.set(folder.id, folder);
    folders = [...unique.values()].sort((left, right) => left.name.localeCompare(right.name, "zh-CN"));
  }

  function selectedSources(): SearchContentQueryV1["sources"] {
    const sources: SearchContentQueryV1["sources"] = [];
    if (includeHistory) sources.push("history");
    if (includeLikes) sources.push("likes");
    if (includeFavorites) sources.push("favorites");
    return sources;
  }

  function effectiveAccountScope(): AccountScope | null {
    if (!runtimeScope) return null;
    if (selectedAccountId === "all") return runtimeScope;
    const accountId = Number(selectedAccountId);
    if (!visibleAccounts.some((account) => account.id === accountId)) return null;
    return { kind: "selected", accountIds: [accountId] };
  }

  async function search(targetPage = 1): Promise<void> {
    const accountScope = effectiveAccountScope();
    const sources = selectedSources();
    if (!accountScope) {
      errorMessage = "搜索账号范围已失效，请前往设置重新选择";
      return;
    }
    if (sources.length === 0) {
      errorMessage = "请至少选择一个来源";
      return;
    }

    const requestGeneration = ++generation;
    loading = true;
    errorMessage = "";
    try {
      const result = await searchLibraryContent({
        accountScope,
        sources,
        query,
        contentTypes: contentType === "all"
          ? []
          : [contentType as "video" | "note" | "unknown"],
        dateFrom: dateFrom || undefined,
        dateTo: dateTo || undefined,
        remoteState,
        localFolderId: localFolderId === "all" ? undefined : Number(localFolderId),
        sort: "archivedNewest",
        limit: PAGE_SIZE,
        offset: (targetPage - 1) * PAGE_SIZE,
      });
      if (requestGeneration !== generation) return;
      records = result.records;
      total = result.total;
      page = Math.min(targetPage, Math.max(1, Math.ceil(result.total / PAGE_SIZE)));
    } catch (error) {
      if (requestGeneration === generation) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (requestGeneration === generation) loading = false;
    }
  }

  function sourceLabel(source: LibraryContentRecordViewV1["source"]): string {
    if (source === "history") return "观看历史";
    if (source === "likes") return "喜欢";
    return "收藏";
  }

  async function openRecord(record: LibraryContentRecordViewV1): Promise<void> {
    try {
      await openContentUrl(record.url);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }
</script>

<section class="search-page" aria-label="搜索">
  <form class="search-filter-panel" on:submit|preventDefault={() => search(1)}>
    <label class="search-query-field">
      <span>关键词</span>
      <span class="search-box"><Search size={16} /><input aria-label="全局搜索关键词" type="search" bind:value={query} placeholder="标题或作者" /></span>
    </label>
    <div class="select-field-label"><span>账号</span><SelectField ariaLabel="账号" bind:value={selectedAccountId} options={[{ value: "all", label: "设置范围内全部账号" }, ...visibleAccounts.map((account) => ({ value: String(account.id), label: account.nickname ?? account.uid ?? `账号 ${account.id}` }))]} /></div>
    <fieldset class="source-filter"><legend>来源</legend><label><input aria-label="观看历史" type="checkbox" bind:checked={includeHistory} />观看历史</label><label><input aria-label="喜欢" type="checkbox" bind:checked={includeLikes} />喜欢</label><label><input aria-label="收藏" type="checkbox" bind:checked={includeFavorites} />收藏</label></fieldset>
    <div class="select-field-label"><span>内容类型</span><SelectField ariaLabel="内容类型" bind:value={contentType} options={[{ value: "all", label: "全部" }, { value: "video", label: "视频" }, { value: "note", label: "图文" }, { value: "unknown", label: "未知" }]} /></div>
    <label><span>开始日期</span><input aria-label="开始日期" type="date" bind:value={dateFrom} /></label>
    <label><span>结束日期</span><input aria-label="结束日期" type="date" bind:value={dateTo} /></label>
    <div class="select-field-label"><span>远端状态</span><SelectField ariaLabel="远端状态" bind:value={remoteState} options={[{ value: "active", label: "仍在远端" }, { value: "removed", label: "已从远端移除" }, { value: "all", label: "全部" }]} /></div>
    <div class="select-field-label"><span>本地收藏夹</span><SelectField ariaLabel="本地收藏夹" bind:value={localFolderId} options={[{ value: "all", label: "全部" }, ...folders.map((folder) => ({ value: String(folder.id), label: folder.name }))]} /></div>
    <button class="primary search-submit" type="submit" disabled={loading}><Search size={16} />搜索</button>
  </form>

  {#if errorMessage}
    <div class="page-error"><AlertCircle size={16} /><span>{errorMessage}</span>{#if runtimeScope}<button type="button" on:click={() => search(page)}>重试</button>{/if}</div>
  {/if}

  <div class="search-result-heading"><span>{total} 条结果</span><button class="icon-button" type="button" title="刷新搜索" aria-label="刷新搜索" disabled={loading || !runtimeScope} on:click={() => search(page)}><RefreshCw size={16} class={loading ? "spinning" : ""} /></button></div>

  <section class="library-content is-horizontal search-results" aria-label="搜索结果">
    {#if loading && records.length === 0}<div class="empty-state">正在搜索本地归档...</div>
    {:else if records.length === 0 && !errorMessage}<div class="empty-state">没有匹配结果</div>
    {:else}
      {#each records as record (`${record.accountId}:${record.source}:${record.id}`)}
        <article class="library-item">
          <button class="library-cover" type="button" title={`打开${record.title ?? "未命名内容"}`} on:click={() => openRecord(record)}><CoverImage cachedCoverUrl={record.cachedCoverUrl} remoteCoverUrl={record.coverUrl} preferCached={coverCacheEnabled} alt={record.title ?? "内容封面"} size={20} /></button>
          <div class="library-item-body">
            <div class="library-badges"><span>{record.accountDisplayName}</span><span>来源：{sourceLabel(record.source)}</span>{#if record.remoteRemovedAt}<span class="removed-state">已从远端移除</span>{/if}</div>
            <button class="content-title" type="button" title={record.url} on:click={() => openRecord(record)}>{record.title ?? "未命名内容"}</button>
            <div class="row-meta">{#if record.authorName}<span>{record.authorName}</span>{/if}{#if record.watchedDate}<span>观看于 {record.watchedDate}</span>{/if}{#if record.likeCount != null}<span>点赞 {formatLikeCount(record.likeCount, likeCountFormat)}</span>{/if}{#if record.watchProgressRate != null}<span>{formatWatchProgress(record.watchProgressRate)}</span>{/if}</div>
          </div>
        </article>
      {/each}
    {/if}
  </section>

  <footer class="pagination-bar" aria-label="搜索分页"><span>{total === 0 ? "0 条" : `${pageStart}-${pageEnd} / ${total} 条`}</span><div class="pagination-actions"><button class="icon-button" type="button" title="上一页" aria-label="上一页" disabled={loading || page <= 1} on:click={() => search(page - 1)}><ChevronLeft size={16} /></button><span>第 {page} / {totalPages} 页</span><button class="icon-button" type="button" title="下一页" aria-label="下一页" disabled={loading || page >= totalPages || total === 0} on:click={() => search(page + 1)}><ChevronRight size={16} /></button></div></footer>
</section>
