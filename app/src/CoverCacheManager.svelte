<script lang="ts">
  import { Search, Trash2, X } from "lucide-svelte";
  import {
    addCoverCacheRule,
    cleanupBrokenCoverCache,
    clearCoverCacheRules,
    inspectCoverCache,
    listCoverCacheRules,
    previewCoverCacheCleanup,
    removeCoverCacheRule,
    runCoverCacheCleanup,
    searchCoverCacheCandidates,
  } from "./app-api";
  import CoverImage from "./CoverImage.svelte";
  import ConfirmDialog from "./controls/ConfirmDialog.svelte";
  import Dialog from "./controls/Dialog.svelte";
  import type {
    CoverCacheCleanupMode,
    CoverCacheCleanupPreview,
    CoverCacheInspection,
    CoverCacheRuleItem,
    CoverCacheSettings,
  } from "./types";

  export let mode: CoverCacheCleanupMode;
  export let settings: CoverCacheSettings;
  export let onClose: () => void;

  let inspection: CoverCacheInspection | null = null;
  let preview: CoverCacheCleanupPreview | null = null;
  let rules: CoverCacheRuleItem[] = [];
  let candidates: CoverCacheRuleItem[] = [];
  let selectedRuleIds = new Set<number>();
  let query = "";
  let loading = true;
  let busy = false;
  let errorMessage = "";
  let noticeMessage = "";
  let pendingConfirmation: "clearRules" | "runCleanup" | null = null;

  $: listName = mode === "keepListed" ? "封面保留列表" : "封面清理列表";
  $: addLabel = `加入${listName}`;
  $: strategyParts = [
    settings.cleanupByCapacityEnabled ? `容量 ${formatBytes(settings.capacityLimitMib * 1024 * 1024)}` : null,
    settings.cleanupByAgeEnabled ? `最长 ${settings.maxAgeDays} 天` : null,
  ].filter((value): value is string => value !== null);
  $: strategyLabel = `${listName} · ${strategyParts.length > 0 ? strategyParts.join(" · ") : "按名单手动清理"}`;

  void refreshAll();

  async function refreshAll(): Promise<void> {
    loading = true;
    errorMessage = "";
    try {
      [inspection, rules, preview] = await Promise.all([
        inspectCoverCache(),
        listCoverCacheRules(),
        previewCoverCacheCleanup(),
      ]);
      selectedRuleIds = new Set(
        [...selectedRuleIds].filter((id) => rules.some((item) => item.videoId === id)),
      );
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      loading = false;
    }
  }

  async function searchCandidates(): Promise<void> {
    busy = true;
    errorMessage = "";
    try {
      candidates = await searchCoverCacheCandidates(query, 50);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }

  async function addRule(item: CoverCacheRuleItem): Promise<void> {
    busy = true;
    try {
      await addCoverCacheRule(item.videoId);
      candidates = candidates.filter((candidate) => candidate.videoId !== item.videoId);
      await refreshAll();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }

  async function removeRules(ids: number[]): Promise<void> {
    if (ids.length === 0) return;
    busy = true;
    try {
      await Promise.all(ids.map((id) => removeCoverCacheRule(id)));
      selectedRuleIds = new Set();
      await refreshAll();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }

  function requestClearRules(): void {
    if (rules.length > 0) pendingConfirmation = "clearRules";
  }

  async function clearRules(): Promise<void> {
    if (rules.length === 0) return;
    busy = true;
    try {
      await clearCoverCacheRules();
      selectedRuleIds = new Set();
      await refreshAll();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }

  function requestCleanup(): void {
    if (preview && preview.files > 0) pendingConfirmation = "runCleanup";
  }

  async function runCleanup(): Promise<void> {
    if (!preview || preview.files === 0) return;
    busy = true;
    try {
      const result = await runCoverCacheCleanup();
      noticeMessage = `已清理 ${result.deletedFiles} 个文件，释放 ${formatBytes(result.releasedBytes)}`;
      await refreshAll();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }

  async function confirmPendingAction(): Promise<void> {
    const action = pendingConfirmation;
    if (action === null) return;
    if (action === "clearRules") await clearRules();
    else await runCleanup();
    pendingConfirmation = null;
  }

  async function cleanBroken(): Promise<void> {
    busy = true;
    try {
      const result = await cleanupBrokenCoverCache();
      noticeMessage = `已清理 ${result.deletedIndexes} 个失效索引和 ${result.deletedFiles} 个文件`;
      await refreshAll();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }

  function toggleSelected(videoId: number, checked: boolean): void {
    const next = new Set(selectedRuleIds);
    if (checked) next.add(videoId);
    else next.delete(videoId);
    selectedRuleIds = next;
  }

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
    const mib = bytes / (1024 * 1024);
    return `${Number.isInteger(mib) ? mib : mib.toFixed(1)} MB`;
  }
</script>

<Dialog ariaLabel="封面缓存清理" panelClass="settings-modal cover-cache-modal" backdropClass="cover-cache-backdrop" {busy} {onClose}>
    <header class="modal-header">
      <div><h2>封面缓存清理</h2><small>{strategyLabel}</small></div>
      <button class="icon-button" type="button" title="关闭封面缓存清理" aria-label="关闭封面缓存清理" data-dialog-close on:click={onClose}><X size={16} /></button>
    </header>

    <div class="cover-cache-summary">
      <span>有效文件 <strong>{inspection?.healthyFiles ?? 0}</strong></span>
      <span>缓存占用 <strong>{formatBytes(inspection?.indexedBytes ?? 0)}</strong></span>
      <span>异常项 <strong>{(inspection?.missingFiles ?? 0) + (inspection?.invalidIndexes ?? 0) + (inspection?.orphanFiles ?? 0)}</strong></span>
      <span>预计释放 {formatBytes(preview?.estimatedBytes ?? 0)}</span>
    </div>

    {#if errorMessage}<div class="cover-cache-message error">{errorMessage}</div>{/if}
    {#if noticeMessage}<div class="cover-cache-message notice">{noticeMessage}</div>{/if}

    <section class="cover-cache-section">
      <div class="cover-cache-section-heading">
        <div><h3>{listName}</h3><p>{mode === "keepListed" ? "列表内封面不会被时间或容量策略删除。" : "自动和手动清理仅处理列表内封面。"}</p></div>
        <div class="cover-cache-actions">
          <button type="button" disabled={busy || selectedRuleIds.size === 0} on:click={() => removeRules([...selectedRuleIds])}>移除选中</button>
          <button class="danger" type="button" disabled={busy || rules.length === 0} on:click={requestClearRules}>清空列表</button>
        </div>
      </div>
      <div class="cover-cache-rule-list">
        {#if loading}<div class="cover-cache-empty">读取中...</div>
        {:else if rules.length === 0}<div class="cover-cache-empty">列表为空</div>
        {:else}{#each rules as item}
          <article class="cover-cache-rule-item">
            <input aria-label={`选择${item.title ?? item.awemeId}`} type="checkbox" checked={selectedRuleIds.has(item.videoId)} on:change={(event) => toggleSelected(item.videoId, (event.currentTarget as HTMLInputElement).checked)} />
            <div class="cover-cache-rule-cover"><CoverImage remoteCoverUrl={item.coverUrl} preferCached={false} alt={item.title ?? item.awemeId} /></div>
            <div><strong>{item.title ?? item.awemeId}</strong><small>{item.authorName ?? "未知作者"}{item.cached ? " · 已缓存" : ""}</small></div>
            <button class="icon-button" type="button" title={`从${listName}移除`} disabled={busy} on:click={() => removeRules([item.videoId])}><Trash2 size={15} /></button>
          </article>
        {/each}{/if}
      </div>
    </section>

    <section class="cover-cache-section">
      <div class="cover-cache-section-heading"><div><h3>搜索并添加</h3><p>搜索列表外的本地内容，不会触发封面下载。</p></div></div>
      <form class="cover-cache-search" on:submit|preventDefault={searchCandidates}>
        <label><Search size={15} /><input aria-label="搜索规则列表外内容" type="search" bind:value={query} placeholder="标题、作者或内容 ID" /></label>
        <button type="submit" disabled={busy}>搜索内容</button>
      </form>
      <div class="cover-cache-candidates">
        {#each candidates as item}
          <article><span><strong>{item.title ?? item.awemeId}</strong><small>{item.authorName ?? "未知作者"}</small></span><button type="button" disabled={busy} on:click={() => addRule(item)}>{addLabel}</button></article>
        {/each}
      </div>
    </section>

    <footer class="cover-cache-footer">
      <button type="button" disabled={busy} on:click={cleanBroken}>清理损坏项</button>
      <span>{preview?.files ?? 0} 个文件 · {preview?.videos ?? 0} 条内容</span>
      <button class="danger" type="button" disabled={busy || !preview || preview.files === 0} on:click={requestCleanup}>执行清理</button>
    </footer>
</Dialog>

{#if pendingConfirmation}
  <ConfirmDialog
    title={pendingConfirmation === "clearRules" ? `清空${listName}` : "执行封面缓存清理"}
    confirmLabel={pendingConfirmation === "clearRules" ? "确认清空" : "确认清理"}
    danger
    {busy}
    onClose={() => (pendingConfirmation = null)}
    onConfirm={() => void confirmPendingAction()}
  >
    <p>{pendingConfirmation === "clearRules" ? `将移除${listName}中的全部规则。` : `将按当前策略清理 ${preview?.files ?? 0} 个封面文件。`}</p>
  </ConfirmDialog>
{/if}
