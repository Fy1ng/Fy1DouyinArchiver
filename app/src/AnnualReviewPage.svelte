<script lang="ts">
  import { AlertCircle, ChevronLeft, ChevronRight, RefreshCw, X } from "lucide-svelte";
  import { getAnnualReview } from "./app-api";
  import { resolvePortableAccountScope } from "./account-scope";
  import SelectField from "./controls/SelectField.svelte";
  import type { AccountView, AnnualReviewChapterV1, AnnualReviewViewV1, PortableAccountScope } from "./types";

  export let accounts: AccountView[];
  export let accountScope: PortableAccountScope;
  export let onClose: () => void;

  let year: number | undefined;
  let view: AnnualReviewViewV1 | null = null;
  let chapterIndex = 0;
  let loading = true;
  let errorMessage = "";
  let lastRequestKey = "";
  let generation = 0;

  $: requestKey = `${JSON.stringify(accountScope)}:${year ?? "default"}`;
  $: if (requestKey !== lastRequestKey) {
    lastRequestKey = requestKey;
    void refresh();
  }
  $: currentChapter = view?.chapters[chapterIndex] ?? null;

  async function refresh(): Promise<void> {
    const requestGeneration = ++generation;
    try {
      const runtimeScope = resolvePortableAccountScope(accountScope, accounts);
      loading = true;
      errorMessage = "";
      const result = await getAnnualReview(runtimeScope, year);
      if (requestGeneration !== generation) return;
      view = result;
      chapterIndex = Math.min(chapterIndex, Math.max(0, result.chapters.length - 1));
    } catch (error) {
      if (requestGeneration === generation) {
        view = null;
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (requestGeneration === generation) loading = false;
    }
  }

  function selectChapter(index: number): void {
    if (!view) return;
    chapterIndex = Math.max(0, Math.min(index, view.chapters.length - 1));
  }

  function changeYear(nextValue: string): void {
    const value = Number(nextValue);
    year = Number.isFinite(value) ? value : undefined;
    chapterIndex = 0;
  }

  function chapterLabel(chapter: AnnualReviewChapterV1): string {
    return chapter.omitted ? `${chapter.title}（内容稀疏）` : chapter.title;
  }
</script>

<section class="annual-review-page" aria-label="年度回顾">
  <header class="annual-review-header">
    <div>
      <span class="eyebrow">Emily archive</span>
      <h1>{view?.year ?? "年度"} 年度回顾</h1>
      <p>只使用本地归档的观看历史与当前关系快照。</p>
    </div>
    <div class="annual-review-actions">
      <div class="select-field-label annual-year-field"><span>回顾年份</span><SelectField ariaLabel="回顾年份" value={String(view?.year ?? year ?? "")} options={(view?.availableYears ?? (year ? [year] : [])).map((availableYear) => ({ value: String(availableYear), label: String(availableYear) }))} disabled={loading} on:change={(event) => changeYear(event.detail.value)} /></div>
      <button class="icon-button" type="button" title="刷新年度回顾" aria-label="刷新年度回顾" on:click={refresh} disabled={loading}><RefreshCw size={16} /></button>
      <button class="icon-button" type="button" title="关闭年度回顾" aria-label="关闭年度回顾" on:click={onClose}><X size={16} /></button>
    </div>
  </header>

  {#if errorMessage}
    <div class="overview-error"><AlertCircle size={16} /><span>{errorMessage}</span></div>
  {:else if loading && !view}
    <div class="empty-state">正在整理年度回顾...</div>
  {:else if view && currentChapter}
    <div class="annual-review-body">
      <nav class="annual-review-toc" aria-label="年度回顾目录">
        {#each view.chapters as chapter, index}
          <button type="button" class:active={index === chapterIndex} on:click={() => selectChapter(index)}>{chapterLabel(chapter)}</button>
        {/each}
      </nav>
      <article class="annual-review-chapter">
        <div class="chapter-index">{String(chapterIndex + 1).padStart(2, "0")} / {String(view.chapters.length).padStart(2, "0")}</div>
        <h2>{currentChapter.title}</h2>
        <p class="chapter-summary">{currentChapter.summary}</p>
        {#if currentChapter.omitted}
          <div class="annual-review-omitted">本章已省略：<span>{currentChapter.omittedReason}</span></div>
        {:else if currentChapter.items.length > 0}
          <div class="annual-review-items">
            {#each currentChapter.items as item}
              <div class="annual-review-item"><span>{item.label}</span><strong>{item.value}</strong>{#if item.secondary}<small>{item.secondary}</small>{/if}</div>
            {/each}
          </div>
        {/if}
      </article>
    </div>
    <footer class="annual-review-footer">
      <button type="button" disabled={chapterIndex === 0} on:click={() => selectChapter(chapterIndex - 1)}><ChevronLeft size={16} />上一章</button>
      <span>{currentChapter.title}</span>
      <button type="button" disabled={chapterIndex === view.chapters.length - 1} on:click={() => selectChapter(chapterIndex + 1)}>下一章<ChevronRight size={16} /></button>
    </footer>
  {/if}
</section>

<style>
  .annual-review-page { display: flex; flex-direction: column; gap: 20px; min-height: 100%; padding: 28px; }
  .annual-review-header, .annual-review-footer { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
  .annual-review-header h1 { margin: 4px 0; font-size: 28px; }
  .annual-review-header p, .chapter-summary { margin: 0; color: var(--text-muted); }
  .eyebrow, .chapter-index { color: var(--accent); font-size: 12px; text-transform: uppercase; }
  .annual-review-actions { display: flex; align-items: center; gap: 8px; }
  .annual-review-actions .select-field-label { display: flex; align-items: center; gap: 8px; color: var(--text-muted); }
  .annual-review-body { display: grid; grid-template-columns: minmax(160px, 220px) 1fr; gap: 28px; flex: 1; }
  .annual-review-toc { display: flex; flex-direction: column; gap: 4px; border-right: 1px solid var(--border); padding-right: 16px; }
  .annual-review-toc button { border: 0; border-radius: 6px; padding: 10px 12px; color: var(--text-muted); background: transparent; text-align: left; cursor: pointer; }
  .annual-review-toc button.active { color: var(--accent); background: color-mix(in srgb, var(--accent) 12%, transparent); }
  .annual-review-chapter { max-width: 760px; padding: 12px 0; }
  .annual-review-chapter h2 { margin: 10px 0 6px; font-size: 36px; }
  .annual-review-items { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; margin-top: 32px; }
  .annual-review-item { display: flex; flex-direction: column; gap: 6px; border-top: 1px solid var(--border); padding-top: 12px; }
  .annual-review-item span, .annual-review-item small { color: var(--text-muted); }
  .annual-review-item strong { font-size: 24px; }
  .annual-review-omitted { margin-top: 28px; border-left: 3px solid var(--accent); padding: 12px 16px; color: var(--text-muted); }
  .annual-review-footer { border-top: 1px solid var(--border); padding-top: 14px; }
  .annual-review-footer button { display: inline-flex; align-items: center; gap: 6px; border: 0; background: transparent; color: var(--text-muted); cursor: pointer; }
  .annual-review-footer button:disabled { opacity: .45; cursor: default; }
  @media (max-width: 760px) { .annual-review-body { grid-template-columns: 1fr; } .annual-review-toc { flex-direction: row; overflow-x: auto; border-right: 0; border-bottom: 1px solid var(--border); padding: 0 0 8px; } .annual-review-toc button { white-space: nowrap; } .annual-review-header { align-items: flex-start; flex-direction: column; } }
</style>
