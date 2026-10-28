<script lang="ts">
  import {
    ChevronLeft,
    ChevronRight,
    Folder,
    Grid3X3,
    List,
    Pencil,
    Plus,
    Search,
    Trash2,
    Users,
  } from "lucide-svelte";
  import CoverImage from "./CoverImage.svelte";
  import SelectField from "./controls/SelectField.svelte";
  import {
    createLocalFolder,
    deleteLocalFolder,
    listLocalFolderContent,
    listLocalFolders,
    listRemoteFolderContent,
    listRemoteFolders,
    openContentUrl,
    removeLocalFolderItem,
    renameLocalFolder,
    setLocalFolderScope,
  } from "./app-api";
  import { formatLikeCount } from "./formatters";
  import type {
    AccountView,
    FolderContentRecordView,
    LikeCountFormat,
    LocalFolderScope,
    LocalFolderView,
    RemoteFolderView,
  } from "./types";

  export let kind: "remote" | "local";
  export let accountId: number;
  export let accounts: AccountView[];
  export let likeCountFormat: LikeCountFormat;
  export let coverCacheEnabled = false;

  const PAGE_SIZE = 80;
  let remoteFolders: RemoteFolderView[] = [];
  let localFolders: LocalFolderView[] = [];
  let selectedRemoteFolderId: string | null = null;
  let selectedLocalFolderId: number | null = null;
  let records: FolderContentRecordView[] = [];
  let total = 0;
  let page = 1;
  let query = "";
  let includeRemoteRemoved = false;
  let viewMode: "list" | "grid" = "list";
  let loading = false;
  let mutating = false;
  let errorMessage = "";
  let newFolderOpen = false;
  let newFolderName = "";
  let newFolderScope: LocalFolderScope = "account";
  let loadedContext = "";
  let editingFolderId: number | null = null;
  let renameDraft = "";

  $: totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  $: pageStart = total === 0 ? 0 : (page - 1) * PAGE_SIZE + 1;
  $: pageEnd = Math.min(total, page * PAGE_SIZE);
  $: selectedLocalFolder =
    localFolders.find((folder) => folder.id === selectedLocalFolderId) ?? null;
  $: activeAccountLabel =
    accounts.find((account) => account.id === accountId)?.nickname ?? `账号 ${accountId}`;
  $: if (accountId > 0 && `${kind}:${accountId}` !== loadedContext) {
    loadedContext = `${kind}:${accountId}`;
    page = 1;
    records = [];
    total = 0;
    remoteFolders = [];
    localFolders = [];
    selectedRemoteFolderId = null;
    selectedLocalFolderId = null;
    void loadFolders(true);
  }

  async function loadFolders(resetSelection = false): Promise<void> {
    const requestedAccountId = accountId;
    const requestedKind = kind;
    loading = true;
    errorMessage = "";
    try {
      if (kind === "remote") {
        const folders = await listRemoteFolders(accountId, includeRemoteRemoved);
        if (requestedAccountId !== accountId || requestedKind !== kind) return;
        remoteFolders = folders;
        if (
          resetSelection ||
          !folders.some((folder) => folder.remoteFolderId === selectedRemoteFolderId)
        ) {
          selectedRemoteFolderId = folders[0]?.remoteFolderId ?? null;
        }
      } else {
        const folders = await listLocalFolders(accountId);
        if (requestedAccountId !== accountId || requestedKind !== kind) return;
        localFolders = folders;
        if (resetSelection || !folders.some((folder) => folder.id === selectedLocalFolderId)) {
          selectedLocalFolderId = folders[0]?.id ?? null;
        }
      }
      await loadContent(1);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      if (requestedAccountId === accountId && requestedKind === kind) loading = false;
    }
  }

  async function loadContent(targetPage = page): Promise<void> {
    const requestedAccountId = accountId;
    const requestedKind = kind;
    const requestedFolder =
      kind === "remote" ? selectedRemoteFolderId : selectedLocalFolderId;
    if (requestedFolder == null) {
      records = [];
      total = 0;
      page = 1;
      return;
    }
    loading = true;
    errorMessage = "";
    try {
      const normalizedPage = Math.max(1, Math.floor(targetPage));
      const result =
        kind === "remote"
          ? await listRemoteFolderContent({
              accountId,
              remoteFolderId: requestedFolder as string,
              query,
              includeRemoteRemoved,
              limit: PAGE_SIZE,
              offset: (normalizedPage - 1) * PAGE_SIZE,
            })
          : await listLocalFolderContent({
              accountId,
              folderId: requestedFolder as number,
              query,
              limit: PAGE_SIZE,
              offset: (normalizedPage - 1) * PAGE_SIZE,
            });
      if (
        requestedAccountId !== accountId ||
        requestedKind !== kind ||
        requestedFolder !==
          (kind === "remote" ? selectedRemoteFolderId : selectedLocalFolderId)
      ) return;
      const maxPage = Math.max(1, Math.ceil(result.total / PAGE_SIZE));
      if (normalizedPage > maxPage) {
        await loadContent(maxPage);
        return;
      }
      records = result.records;
      total = result.total;
      page = normalizedPage;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      if (requestedAccountId === accountId && requestedKind === kind) loading = false;
    }
  }

  function selectRemoteFolder(remoteFolderId: string): void {
    if (selectedRemoteFolderId === remoteFolderId) return;
    selectedRemoteFolderId = remoteFolderId;
    void loadContent(1);
  }

  function selectLocalFolder(folderId: number): void {
    if (selectedLocalFolderId === folderId) return;
    selectedLocalFolderId = folderId;
    void loadContent(1);
  }

  function updateQuery(value: string): void {
    query = value;
    void loadContent(1);
  }

  async function toggleRemoteRemoved(checked: boolean): Promise<void> {
    includeRemoteRemoved = checked;
    await loadFolders(false);
  }

  async function submitNewFolder(): Promise<void> {
    if (!newFolderName.trim() || mutating) return;
    mutating = true;
    errorMessage = "";
    try {
      const folder = await createLocalFolder(accountId, newFolderName, newFolderScope);
      newFolderName = "";
      newFolderOpen = false;
      await loadFolders(false);
      selectedLocalFolderId = folder.id;
      await loadContent(1);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      mutating = false;
    }
  }

  function startRenaming(folder: LocalFolderView): void {
    editingFolderId = folder.id;
    renameDraft = folder.name;
  }

  function cancelRenaming(): void {
    editingFolderId = null;
    renameDraft = "";
  }

  async function submitRename(folder: LocalFolderView): Promise<void> {
    const name = renameDraft.trim();
    if (!name || mutating) return;
    if (name === folder.name) {
      cancelRenaming();
      return;
    }
    mutating = true;
    try {
      await renameLocalFolder(accountId, folder.id, name);
      cancelRenaming();
      await loadFolders(false);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      mutating = false;
    }
  }

  async function changeFolderScope(
    folder: LocalFolderView,
    scope: LocalFolderScope,
    ownerAccountId: number | null,
  ): Promise<void> {
    if (mutating) return;
    mutating = true;
    try {
      await setLocalFolderScope(accountId, folder.id, scope, ownerAccountId);
      await loadFolders(false);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      mutating = false;
    }
  }

  async function removeFolder(folder: LocalFolderView): Promise<void> {
    if (!window.confirm(`删除本地收藏夹“${folder.name}”？收藏夹内条目关系会被删除。`)) return;
    mutating = true;
    try {
      await deleteLocalFolder(accountId, folder.id);
      await loadFolders(true);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      mutating = false;
    }
  }

  async function removeItem(record: FolderContentRecordView): Promise<void> {
    if (kind !== "local" || selectedLocalFolderId == null || mutating) return;
    mutating = true;
    try {
      await removeLocalFolderItem(accountId, selectedLocalFolderId, record.awemeId);
      await Promise.all([loadFolders(false), loadContent(page)]);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      mutating = false;
    }
  }

  async function openRecord(record: FolderContentRecordView): Promise<void> {
    try {
      await openContentUrl(record.url);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  function sourceAccountLabel(record: FolderContentRecordView): string {
    return record.sourceAccounts
      .map((account) => account.nickname ?? `账号 ${account.id}`)
      .join("、");
  }
</script>

<section class="folder-page" aria-label={kind === "remote" ? "抖音收藏夹" : "本地收藏夹"}>
  <aside class="folder-sidebar">
    <header>
      <div>
        <h2>{kind === "remote" ? "抖音收藏夹" : "本地收藏夹"}</h2>
        <small>{kind === "remote" ? `${remoteFolders.length} 个分类` : `${localFolders.length} 个收藏夹`}</small>
      </div>
      {#if kind === "local"}
        <button
          class="icon-button"
          type="button"
          title="新建本地收藏夹"
          on:click={() => (newFolderOpen = !newFolderOpen)}
        ><Plus size={16} /></button>
      {/if}
    </header>

    {#if kind === "remote"}
      <label class="removed-filter folder-filter">
        <input
          type="checkbox"
          checked={includeRemoteRemoved}
          on:change={(event) => toggleRemoteRemoved((event.currentTarget as HTMLInputElement).checked)}
        />
        <span>包含已移除分类</span>
      </label>
    {:else if newFolderOpen}
      <form class="new-folder-form" on:submit|preventDefault={submitNewFolder}>
        <input aria-label="收藏夹名称" placeholder="收藏夹名称" bind:value={newFolderName} />
        <SelectField ariaLabel="收藏夹归属" value={newFolderScope} options={[{ value: "account", label: "当前账号" }, { value: "aggregate", label: "多账号聚合" }]} on:change={(event) => (newFolderScope = event.detail.value as LocalFolderScope)} />
        <button class="primary" type="submit" disabled={mutating || !newFolderName.trim()}>创建</button>
      </form>
    {/if}

    <div class="folder-list">
      {#if loading && remoteFolders.length === 0 && localFolders.length === 0}
        <div class="folder-list-empty">读取中...</div>
      {:else if kind === "remote" && remoteFolders.length === 0}
        <div class="folder-list-empty">暂无抖音收藏夹</div>
      {:else if kind === "local" && localFolders.length === 0}
        <div class="folder-list-empty">暂无本地收藏夹</div>
      {:else if kind === "remote"}
        {#each remoteFolders as folder}
          <button
            class:active={selectedRemoteFolderId === folder.remoteFolderId}
            class:remote-removed={folder.remoteRemovedAt != null}
            type="button"
            on:click={() => selectRemoteFolder(folder.remoteFolderId)}
          >
            <Folder size={15} />
            <span><strong>{folder.name}</strong><small>{folder.activeItemCount} / {folder.itemCount} 条</small></span>
          </button>
        {/each}
      {:else}
        {#each localFolders as folder}
          <div class:active={selectedLocalFolderId === folder.id} class="local-folder-row">
            {#if editingFolderId === folder.id}
              <form class="folder-rename-form" on:submit|preventDefault={() => submitRename(folder)}>
                <input aria-label="新收藏夹名称" bind:value={renameDraft} />
                <button class="primary" type="submit" disabled={mutating || !renameDraft.trim()}>保存</button>
                <button type="button" disabled={mutating} on:click={cancelRenaming}>取消</button>
              </form>
            {:else}
              <button type="button" on:click={() => selectLocalFolder(folder.id)}>
                {#if folder.scope === "aggregate"}<Users size={15} />{:else}<Folder size={15} />{/if}
                <span><strong>{folder.name}</strong><small>{folder.scope === "aggregate" ? "多账号聚合" : "当前账号"} · {folder.itemCount} 条</small></span>
              </button>
              <button class="folder-row-action" type="button" title="重命名" disabled={mutating} on:click={() => startRenaming(folder)}><Pencil size={13} /></button>
              <button class="folder-row-action danger-text" type="button" title="删除收藏夹" disabled={mutating} on:click={() => removeFolder(folder)}><Trash2 size={13} /></button>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </aside>

  <div class="folder-content">
    <header class="folder-content-toolbar">
      <label class="search-box">
        <Search size={16} />
        <input
          aria-label="收藏夹内容搜索"
          type="search"
          value={query}
          placeholder="搜索标题或作者"
          on:input={(event) => updateQuery((event.currentTarget as HTMLInputElement).value)}
        />
      </label>
      {#if kind === "local" && selectedLocalFolder}
        <div class="folder-scope-select">
          <span>归属</span>
          <SelectField
            ariaLabel="当前收藏夹归属"
            value={selectedLocalFolder.scope === "aggregate" ? "aggregate" : String(selectedLocalFolder.ownerAccountId)}
            disabled={mutating}
            on:change={(event) => {
              const value = event.detail.value;
              void changeFolderScope(
                selectedLocalFolder,
                value === "aggregate" ? "aggregate" : "account",
                value === "aggregate" ? null : Number(value),
              );
            }}
            options={[{ value: "aggregate", label: "多账号聚合" }, { value: String(accountId), label: activeAccountLabel }]}
          />
        </div>
      {/if}
      <div class="segmented view-mode" aria-label="收藏夹视图">
        <button class:active={viewMode === "list"} type="button" title="列表视图" aria-label="收藏夹列表视图" on:click={() => (viewMode = "list")}><List size={16} /></button>
        <button class:active={viewMode === "grid"} type="button" title="网格视图" aria-label="收藏夹网格视图" on:click={() => (viewMode = "grid")}><Grid3X3 size={16} /></button>
      </div>
    </header>

    {#if errorMessage}<div class="folder-error">{errorMessage}</div>{/if}
    <div class:folder-record-list={viewMode === "list"} class:folder-record-grid={viewMode === "grid"}>
      {#if loading && records.length === 0}
        <div class="empty-state">正在读取收藏夹内容...</div>
      {:else if records.length === 0}
        <div class="empty-state">当前收藏夹没有匹配内容</div>
      {:else}
        {#each records as record}
          <article class:remote-removed={record.remoteRemovedAt != null} class="folder-record">
            {#if viewMode === "grid"}
              <button class="folder-record-cover" type="button" title={`打开${record.title ?? "未命名内容"}`} on:click={() => openRecord(record)}>
                <CoverImage
                  cachedCoverUrl={record.cachedCoverUrl}
                  remoteCoverUrl={record.coverUrl}
                  preferCached={coverCacheEnabled}
                  alt={record.title ?? "内容封面"}
                  size={22}
                />
              </button>
            {:else}
              <div class="cover-slot">
                <CoverImage
                  cachedCoverUrl={record.cachedCoverUrl}
                  remoteCoverUrl={record.coverUrl}
                  preferCached={coverCacheEnabled}
                  alt={record.title ?? "内容封面"}
                  size={18}
                />
              </div>
            {/if}
            <div class="row-main">
              <button class="content-title" type="button" title={record.url} on:click={() => openRecord(record)}>{record.title ?? "未命名内容"}</button>
              <div class="row-meta">
                <span>{record.authorName ?? "未知作者"}</span>
                {#if record.likeCount != null}<span>点赞 {formatLikeCount(record.likeCount, likeCountFormat)}</span>{/if}
                {#if record.sourceAccounts.length > 0}<span>来源 {sourceAccountLabel(record)}</span>{/if}
                {#if record.remoteRemovedAt}<span class="removed-state">已从远端移除</span>{/if}
              </div>
            </div>
            {#if kind === "local"}
              <button class="icon-danger" type="button" title="从本地收藏夹移除" disabled={mutating} on:click={() => removeItem(record)}><Trash2 size={15} /></button>
            {/if}
          </article>
        {/each}
      {/if}
    </div>

    <footer class="pagination-bar" aria-label="收藏夹分页">
      <span>{total === 0 ? "0 条" : `${pageStart}-${pageEnd} / ${total} 条`}</span>
      <div class="pagination-actions">
        <button class="icon-button" type="button" title="上一页" disabled={loading || page <= 1} on:click={() => loadContent(page - 1)}><ChevronLeft size={16} /></button>
        <span>第 {page} / {totalPages} 页</span>
        <button class="icon-button" type="button" title="下一页" disabled={loading || page >= totalPages || total === 0} on:click={() => loadContent(page + 1)}><ChevronRight size={16} /></button>
      </div>
    </footer>
  </div>
</section>
