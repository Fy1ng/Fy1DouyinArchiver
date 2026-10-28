<script lang="ts">
  import { Film } from "lucide-svelte";

  export let cachedCoverUrl: string | null | undefined = null;
  export let remoteCoverUrl: string | null | undefined = null;
  export let preferCached = false;
  export let alt = "";
  export let size = 18;

  let failedUrls = new Set<string>();
  $: primaryUrl = preferCached ? cachedCoverUrl ?? remoteCoverUrl : remoteCoverUrl;
  $: fallbackUrl = preferCached && cachedCoverUrl ? remoteCoverUrl : null;
  $: visibleUrl = primaryUrl && !failedUrls.has(primaryUrl)
    ? primaryUrl
    : fallbackUrl && !failedUrls.has(fallbackUrl)
      ? fallbackUrl
      : null;

  function handleError(): void {
    if (!visibleUrl) return;
    failedUrls = new Set([...failedUrls, visibleUrl]);
  }
</script>

{#if visibleUrl}
  <img src={visibleUrl} {alt} loading="lazy" on:error={handleError} />
{:else}
  <span class="cover-placeholder" aria-label="无封面"><Film {size} /></span>
{/if}
