<script lang="ts">
  import { onMount } from "svelte";

  type ScrollEntry = {
    target: HTMLElement;
    targetId: string;
    generatedTargetId: boolean;
    track: HTMLDivElement;
    thumb: HTMLButtonElement;
    cleanup: () => void;
  };

  const TRACK_HIT_WIDTH = 24;
  const MIN_THUMB_HEIGHT = 32;
  const ARROW_STEP = 40;

  let entries: ScrollEntry[] = [];
  let scanFrame = 0;
  let resizeObserver: ResizeObserver | null = null;
  let mutationObserver: MutationObserver | null = null;
  let targetSequence = 0;

  function isScrollViewport(element: HTMLElement): boolean {
    if (element === document.body || element === document.documentElement) return false;
    const style = getComputedStyle(element);
    return [style.overflowY, style.overflow].some((value) => value === "auto" || value === "scroll")
      && element.scrollHeight > element.clientHeight + 1;
  }

  function update(entry: ScrollEntry): void {
    const { target, track, thumb } = entry;
    const rect = target.getBoundingClientRect();
    const visibleRatio = target.clientHeight / target.scrollHeight;
    const top = Math.max(3, rect.top + 3);
    const bottom = Math.min(window.innerHeight - 3, rect.bottom - 3);
    const trackHeight = bottom - top;
    if (trackHeight < MIN_THUMB_HEIGHT || rect.width <= 0 || rect.height <= 0 || rect.right <= 0 || rect.left >= window.innerWidth) {
      track.style.display = "none";
      return;
    }
    const thumbHeight = Math.max(MIN_THUMB_HEIGHT, Math.min(trackHeight, trackHeight * visibleRatio));
    const maxScroll = Math.max(0, target.scrollHeight - target.clientHeight);
    const maxOffset = Math.max(0, trackHeight - thumbHeight);
    const offset = maxScroll === 0 ? 0 : (target.scrollTop / maxScroll) * maxOffset;
    track.style.left = `${Math.min(window.innerWidth - TRACK_HIT_WIDTH, Math.max(0, rect.right - TRACK_HIT_WIDTH))}px`;
    track.style.top = `${top}px`;
    track.style.height = `${trackHeight}px`;
    track.style.display = rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.top < window.innerHeight
      ? "block"
      : "none";
    thumb.style.height = `${thumbHeight}px`;
    thumb.style.transform = `translateY(${offset}px)`;
    thumb.setAttribute("aria-valuemax", String(Math.round(maxScroll)));
    thumb.setAttribute("aria-valuenow", String(Math.round(Math.min(maxScroll, Math.max(0, target.scrollTop)))));
    thumb.setAttribute("aria-valuetext", `${Math.round(maxScroll === 0 ? 0 : (target.scrollTop / maxScroll) * 100)}%`);
  }

  function removeEntry(entry: ScrollEntry): void {
    entry.cleanup();
    entry.track.remove();
  }

  function createEntry(target: HTMLElement): ScrollEntry {
    const generatedTargetId = target.id.length === 0;
    if (generatedTargetId) {
      do {
        targetSequence += 1;
        target.id = `emily-scroll-viewport-${targetSequence}`;
      } while (document.getElementById(target.id) !== target);
    }
    const targetId = target.id;
    const targetLabel = target.dataset.scrollbarLabel?.trim()
      || target.getAttribute("aria-label")?.trim()
      || `滚动区域 ${targetSequence}`;
    const track = document.createElement("div");
    const thumb = document.createElement("button");
    track.className = "custom-scrollbar-track";
    track.setAttribute("role", "presentation");
    thumb.className = "custom-scrollbar-thumb";
    thumb.type = "button";
    thumb.setAttribute("role", "scrollbar");
    thumb.setAttribute("aria-label", `${targetLabel}滚动条`);
    thumb.setAttribute("aria-controls", targetId);
    thumb.setAttribute("aria-orientation", "vertical");
    thumb.setAttribute("aria-valuemin", "0");
    track.appendChild(thumb);
    document.body.appendChild(track);

    let dragging = false;
    let startPointer = 0;
    let startScroll = 0;

    const updateEntry = () => update(entry);
    const setScrollTop = (value: number) => {
      const maxScroll = Math.max(0, target.scrollHeight - target.clientHeight);
      target.scrollTop = Math.min(maxScroll, Math.max(0, value));
      updateEntry();
    };
    const onPointerDown = (event: PointerEvent) => {
      event.preventDefault();
      event.stopPropagation();
      dragging = true;
      startPointer = event.clientY;
      startScroll = target.scrollTop;
      thumb.setPointerCapture?.(event.pointerId);
      thumb.classList.add("is-dragging");
    };
    const onPointerMove = (event: PointerEvent) => {
      if (!dragging) return;
      const trackHeight = Math.max(MIN_THUMB_HEIGHT, track.clientHeight);
      const thumbHeight = thumb.offsetHeight;
      const ratio = (target.scrollHeight - target.clientHeight) / Math.max(1, trackHeight - thumbHeight);
      setScrollTop(startScroll + (event.clientY - startPointer) * ratio);
    };
    const onPointerUp = () => {
      dragging = false;
      thumb.classList.remove("is-dragging");
    };
    const onTrackPointerDown = (event: PointerEvent) => {
      if (event.target === thumb) return;
      const rect = track.getBoundingClientRect();
      const ratio = (event.clientY - rect.top) / Math.max(1, rect.height);
      thumb.focus({ preventScroll: true });
      setScrollTop(ratio * (target.scrollHeight - target.clientHeight));
    };
    const onKeyDown = (event: KeyboardEvent) => {
      const maxScroll = Math.max(0, target.scrollHeight - target.clientHeight);
      let nextScroll: number | null = null;
      if (event.key === "ArrowUp") nextScroll = target.scrollTop - ARROW_STEP;
      else if (event.key === "ArrowDown") nextScroll = target.scrollTop + ARROW_STEP;
      else if (event.key === "PageUp") nextScroll = target.scrollTop - target.clientHeight;
      else if (event.key === "PageDown") nextScroll = target.scrollTop + target.clientHeight;
      else if (event.key === "Home") nextScroll = 0;
      else if (event.key === "End") nextScroll = maxScroll;
      if (nextScroll === null) return;
      event.preventDefault();
      event.stopPropagation();
      setScrollTop(nextScroll);
    };
    thumb.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    track.addEventListener("pointerdown", onTrackPointerDown);
    thumb.addEventListener("keydown", onKeyDown);
    target.addEventListener("scroll", updateEntry, { passive: true });

    const entry = {
      target,
      targetId,
      generatedTargetId,
      track,
      thumb,
      cleanup: () => {
        thumb.removeEventListener("pointerdown", onPointerDown);
        window.removeEventListener("pointermove", onPointerMove);
        window.removeEventListener("pointerup", onPointerUp);
        track.removeEventListener("pointerdown", onTrackPointerDown);
        thumb.removeEventListener("keydown", onKeyDown);
        target.removeEventListener("scroll", updateEntry);
        if (generatedTargetId && target.id === targetId) target.removeAttribute("id");
      },
    } satisfies ScrollEntry;
    update(entry);
    return entry;
  }

  function scan(): void {
    scanFrame = 0;
    const candidates = [...document.querySelectorAll<HTMLElement>("*")].filter(isScrollViewport);
    const candidateSet = new Set(candidates);
    for (const entry of entries.filter((item) => !candidateSet.has(item.target))) removeEntry(entry);
    entries = entries.filter((item) => candidateSet.has(item.target));
    for (const candidate of candidates) {
      if (!entries.some((entry) => entry.target === candidate)) entries.push(createEntry(candidate));
    }
    for (const entry of entries) update(entry);
  }

  function scheduleScan(): void {
    if (!scanFrame && typeof requestAnimationFrame === "function") scanFrame = requestAnimationFrame(scan);
  }

  onMount(() => {
    if (typeof document === "undefined" || typeof window === "undefined") return;
    scan();
    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(scheduleScan);
      resizeObserver.observe(document.documentElement);
    }
    if (typeof MutationObserver !== "undefined") {
      mutationObserver = new MutationObserver(scheduleScan);
      mutationObserver.observe(document.body, { childList: true, subtree: true });
    }
    window.addEventListener("resize", scheduleScan);
    document.addEventListener("scroll", scheduleScan, true);
    return () => {
      if (scanFrame) cancelAnimationFrame(scanFrame);
      resizeObserver?.disconnect();
      mutationObserver?.disconnect();
      window.removeEventListener("resize", scheduleScan);
      document.removeEventListener("scroll", scheduleScan, true);
      entries.forEach(removeEntry);
      entries = [];
    };
  });
</script>
