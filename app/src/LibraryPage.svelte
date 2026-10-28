<script lang="ts">
  import {
    AlertCircle,
    ChevronLeft,
    ChevronRight,
    Grid3X3,
    List,
    RefreshCw,
    Search,
  } from "lucide-svelte";
  import { listLibraryContent, openContentUrl } from "./app-api";
  import { resolvePortableAccountScope } from "./account-scope";
  import CoverImage from "./CoverImage.svelte";
  import SelectField from "./controls/SelectField.svelte";
  import { formatLikeCount, formatWatchProgress } from "./formatters";
  import type {
    AccountView,
    LibraryContentQueryV1,
    LibraryContentRecordViewV1,
    LikeCountFormat,
    PortableAccountScope,
    WorkspaceContentViewPreference,
    WorkspaceSettings,
  } from "./types";
  import type { SourceAvailability } from "./shell/types";

  type LibrarySource = "all" | "history" | "likes" | "favorites";

  export let accounts: AccountView[];
  export let accountScope: PortableAccountScope;
  export let preferences: WorkspaceSettings["contentViews"];
  export let sourceAvailability: SourceAvailability;
  export let likeCountFormat: LikeCountFormat;
  export let coverCacheEnabled: boolean;
  export let onPreferenceChange: (
    source: LibrarySource,
    preference: WorkspaceContentViewPreference,
  ) => void;

  const PAGE_SIZE = 80;
  let activeSource: LibrarySource = "all";
  let localPreferences = structuredClone(preferences);
  let records: LibraryContentRecordViewV1[] = [];
  let total = 0;
  let loading = true;
  let errorMessage = "";
  let lastRequestKey = "";
  let generation = 0;

  $: currentPreference = localPreferences[activeSource];
  $: totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  $: pageStart = total === 0 ? 0 : (currentPreference.page - 1) * PAGE_SIZE + 1;
  $: pageEnd = Math.min(total, currentPreference.page * PAGE_SIZE);
  $: requestKey = JSON.stringify({ accountScope, activeSource, currentPreference });
  $: if (requestKey !== lastRequestKey) {
    lastRequestKey = requestKey;
    void refresh();
  }

  function selectedSources(): LibraryContentQueryV1["sources"] {
    if (activeSource === "all") return ["history", "likes", "favorites"];
    return [activeSource];
  }

  async function refresh(): Promise<void> {
    const requestGeneration = ++generation;
    let runtimeScope: LibraryContentQueryV1["accountScope"];
    try {
      runtimeScope = resolvePortableAccountScope(accountScope, accounts);
    } catch {
      if (requestGeneration === generation) {
        records = [];
        total = 0;
        loading = false;
        errorMessage = "账号范围已失效，请重新选择";
      }
      return;
    }

    loading = true;
    errorMessage = "";
    try {
      const result = await listLibraryContent({
        accountScope: runtimeScope,
        sources: selectedSources(),
        query: currentPreference.query,
        includeRemoteRemoved:
          activeSource !== "history" && currentPreference.includeRemoteRemoved,
        sort: currentPreference.sort,
        limit: PAGE_SIZE,
        offset: (currentPreference.page - 1) * PAGE_SIZE,
      });
      if (requestGeneration !== generation) return;
      const maxPage = Math.max(1, Math.ceil(result.total / PAGE_SIZE));
      if (currentPreference.page > maxPage) {
        updatePreference({ page: maxPage });
        return;
      }
      records = result.records;
      total = result.total;
    } catch (error) {
      if (requestGeneration === generation) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (requestGeneration === generation) loading = false;
    }
  }

  function selectSource(source: LibrarySource): void {
    activeSource = source;
  }

  function updatePreference(patch: Partial<WorkspaceContentViewPreference>): void {
    const next = { ...localPreferences[activeSource], ...patch };
    localPreferences = { ...localPreferences, [activeSource]: next };
    onPreferenceChange(activeSource, next);
  }

  function updateQuery(value: string): void {
    updatePreference({ query: value, page: 1 });
  }

  function goToPage(page: number): void {
    updatePreference({ page: Math.max(1, Math.min(totalPages, page)) });
  }

  function sourceLabel(source: LibraryContentRecordViewV1["source"]): string {
    if (source === "history") return "观看历史";
    if (source === "likes") return "喜欢";
    return "收藏";
  }

  function formatDate(value?: string | null): string {
    if (!value) return "";
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return new Intl.DateTimeFormat("zh-CN", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  }

  async function openRecord(record: LibraryContentRecordViewV1): Promise<void> {
    try {
      await openContentUrl(record.url);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }
</script>

<section class="library-page" aria-label="资料库">
  <nav class="library-source-nav" aria-label="资料库来源">
    <button class:active={activeSource === "all"} type="button" on:click={() => selectSource("all")}>全部内容</button>
    <button class:active={activeSource === "history"} type="button" disabled={!sourceAvailability.history} on:click={() => selectSource("history")}>观看历史</button>
    <button class:active={activeSource === "likes"} type="button" disabled={!sourceAvailability.likes} on:click={() => selectSource("likes")}>喜欢</button>
    <button class:active={activeSource === "favorites"} type="button" disabled={!sourceAvailability.favorites} on:click={() => selectSource("favorites")}>收藏</button>
  </nav>

  <div class="library-toolbar">
    <label class="search-box">
      <Search size={16} />
      <input
        aria-label="资料库搜索"
        type="search"
        value={currentPreference.query}
        placeholder="搜索标题或作者"
        on:input={(event) => updateQuery((event.currentTarget as HTMLInputElement).value)}
      />
    </label>
    <div class="library-toolbar-actions">
      <div class="sort-control">
        <span>排序</span>
        <SelectField
          value={currentPreference.sort}
          ariaLabel="排序"
          options={[
            ...(activeSource === "all" ? [{ value: "archivedNewest", label: "首次归档从新到旧" }] : []),
            ...(activeSource === "history"
              ? [
                  { value: "watchedNewest", label: "观看时间从新到旧" },
                  { value: "watchedOldest", label: "观看时间从旧到新" },
                ]
              : activeSource !== "all"
                ? [
                    { value: "pageOrder", label: "页面采集顺序" },
                    { value: "remoteNewest", label: "远端时间从新到旧" },
                    { value: "remoteOldest", label: "远端时间从旧到新" },
                  ]
                : []),
            { value: "likeCountDesc", label: "点赞量从高到低" },
            { value: "titleAsc", label: "标题" },
            { value: "authorAsc", label: "作者" },
          ]}
          on:change={(event) =>
            updatePreference({
              sort: event.detail.value as WorkspaceContentViewPreference["sort"],
              page: 1,
            })}
        />
      </div>
      {#if activeSource !== "history"}
        <label class="removed-filter">
          <input
            type="checkbox"
            checked={currentPreference.includeRemoteRemoved}
            on:change={(event) =>
              updatePreference({
                includeRemoteRemoved: (event.currentTarget as HTMLInputElement).checked,
                page: 1,
              })}
          />
          <span>包含已从远端移除</span>
        </label>
      {/if}
      <div class="segmented view-mode" role="group" aria-label="内容视图">
        <button
          class:active={currentPreference.viewMode === "horizontal"}
          type="button"
          title="横向列表"
          aria-label="横向列表"
          aria-pressed={currentPreference.viewMode === "horizontal"}
          on:click={() => updatePreference({ viewMode: "horizontal" })}
        ><List size={16} /></button>
        <button
          class:active={currentPreference.viewMode === "grid"}
          type="button"
          title="网格视图"
          aria-label="网格视图"
          aria-pressed={currentPreference.viewMode === "grid"}
          on:click={() => updatePreference({ viewMode: "grid" })}
        ><Grid3X3 size={16} /></button>
      </div>
      <button class="icon-button" type="button" title="刷新资料库" aria-label="刷新资料库" disabled={loading} on:click={refresh}>
        <RefreshCw size={16} class={loading ? "spinning" : ""} />
      </button>
    </div>
  </div>

  {#if errorMessage}
    <div class="page-error"><AlertCircle size={16} /><span>{errorMessage}</span><button type="button" on:click={refresh}>重试</button></div>
  {/if}

  <section
    class="library-content"
    class:is-grid={currentPreference.viewMode === "grid"}
    class:is-horizontal={currentPreference.viewMode === "horizontal"}
    aria-label="资料库内容"
  >
    {#if loading && records.length === 0}
      <div class="empty-state">正在读取本地内容...</div>
    {:else if records.length === 0 && !errorMessage}
      <div class="empty-state">没有匹配的归档内容</div>
    {:else}
      {#each records as record (`${record.accountId}:${record.source}:${record.id}`)}
        <article class="library-item" class:remote-removed={record.remoteRemovedAt != null}>
          <button class="library-cover" type="button" title={`打开${record.title ?? "未命名内容"}`} on:click={() => openRecord(record)}>
            <CoverImage
              cachedCoverUrl={record.cachedCoverUrl}
              remoteCoverUrl={record.coverUrl}
              preferCached={coverCacheEnabled}
              alt={record.title ?? "内容封面"}
              size={22}
            />
          </button>
          <div class="library-item-body">
            <div class="library-badges">
              <span>{record.accountDisplayName}</span>
              <span>来源：{sourceLabel(record.source)}</span>
              {#if record.remoteRemovedAt}<span class="removed-state">已从远端移除</span>{/if}
            </div>
            <button class="content-title" type="button" title={record.url} on:click={() => openRecord(record)}>{record.title ?? "未命名内容"}</button>
            <div class="row-meta">
              {#if record.authorName}<span>{record.authorName}</span>{/if}
              {#if record.source === "history" && record.watchedDate}<span>观看于 {record.watchedDate}</span>{/if}
              {#if record.likeCount != null}<span>点赞 {formatLikeCount(record.likeCount, likeCountFormat)}</span>{/if}
              {#if record.watchProgressRate != null}<span>{formatWatchProgress(record.watchProgressRate)}</span>{/if}
              <span>归档于 {formatDate(record.firstArchivedAt)}</span>
            </div>
          </div>
        </article>
      {/each}
    {/if}
  </section>

  <footer class="pagination-bar" aria-label="分页">
    <span>{total === 0 ? "0 条" : `${pageStart}-${pageEnd} / ${total} 条`}</span>
    <div class="pagination-actions">
      <button class="icon-button" type="button" title="上一页" aria-label="上一页" disabled={loading || currentPreference.page <= 1} on:click={() => goToPage(currentPreference.page - 1)}><ChevronLeft size={16} /></button>
      <span>第 {currentPreference.page} / {totalPages} 页</span>
      <button class="icon-button" type="button" title="下一页" aria-label="下一页" disabled={loading || currentPreference.page >= totalPages || total === 0} on:click={() => goToPage(currentPreference.page + 1)}><ChevronRight size={16} /></button>
    </div>
  </footer>
</section>
