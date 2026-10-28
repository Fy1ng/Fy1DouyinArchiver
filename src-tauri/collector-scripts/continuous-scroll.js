async (options) => {
  const visible = (element) => {
    if (!(element instanceof HTMLElement)) return false;
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return !element.hidden && style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const scrollable = (element) => {
    if (!(element instanceof HTMLElement)) return false;
    return /(auto|scroll|overlay)/.test(getComputedStyle(element).overflowY);
  };
  const nearestScrollTarget = (anchor) => {
    const route = anchor instanceof Element ? anchor.closest(".route-scroll-container") : null;
    if (scrollable(route)) return route;
    let current = anchor instanceof HTMLElement ? anchor.parentElement : null;
    while (current) {
      if (scrollable(current)) return current;
      current = current.parentElement;
    }
    return document.scrollingElement;
  };
  const contentLink = (element) => Array.from(element.querySelectorAll("a[href]")).some((anchor) => {
    try {
      const url = new URL(anchor.href, location.href);
      return url.hostname === "www.douyin.com" && /^\/(video|note|article)\/\d+\/?$/.test(url.pathname);
    } catch (_) {
      return false;
    }
  });
  const source = typeof options.source === "string" ? options.source : "";
  const fallbackList = document.querySelector('ul[data-e2e="scroll-list"]');
  const sourceRoot = source === "likes"
    ? document.querySelector('[data-e2e="user-like-list"]')
    : source === "favorites" || source === "remoteFolder"
      ? document.querySelector('[data-e2e="user-favorite-list"]')
      : source === "history"
        ? fallbackList
        : null;
  const anchor = sourceRoot || fallbackList;
  const target = nearestScrollTarget(anchor);
  if (!target) throw new Error("scroll_target_not_found");
  const list = sourceRoot && sourceRoot.matches('ul[data-e2e="scroll-list"]')
    ? sourceRoot
    : sourceRoot?.querySelector('ul[data-e2e="scroll-list"]') || fallbackList;
  const recordSelector = typeof options.recordSelector === "string" && options.recordSelector
    ? options.recordSelector
    : 'ul[data-e2e="scroll-list"] > li.record_card';
  const recordCount = () => {
    if (source === "history") return list ? list.querySelectorAll(":scope > li.record_card").length : 0;
    if (source === "likes" || source === "favorites" || source === "remoteFolder") {
      return list ? Array.from(list.children).filter(contentLink).length : 0;
    }
    return document.querySelectorAll(recordSelector).length;
  };
  const scopedElements = () => [target, ...target.querySelectorAll("*")];
  const noMore = () => scopedElements().some((element) => visible(element) && (element.textContent || "").trim() === "暂时没有更多了");
  const started = performance.now();
  let frameAt = started;
  let moveAt = started;
  let lastTop = target.scrollTop;
  return await new Promise((resolve) => {
    let finished = false;
    const timer = setTimeout(() => finish("timeout"), options.maxDurationMs);
    const finish = (reason) => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      resolve({ reason, recordCount: recordCount(), scrollTop: target.scrollTop, scrollHeight: target.scrollHeight });
    };
    const frame = (now) => {
      const count = recordCount();
      if (count > options.startRecordCount) return finish("list-growth");
      if (noMore()) return finish("no-more");
      if (now - started >= options.maxDurationMs) return finish("timeout");
      const elapsed = Math.min(100, Math.max(0, now - frameAt));
      frameAt = now;
      const maxTop = Math.max(0, target.scrollHeight - target.clientHeight);
      target.scrollTop = Math.min(maxTop, target.scrollTop + options.scrollSpeed * elapsed / 1000);
      if (target.scrollTop > lastTop + 0.25) {
        lastTop = target.scrollTop;
        moveAt = now;
      }
      if (target.scrollTop >= maxTop - 0.5 && now - moveAt >= 600) return finish("bottom-stall");
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  });
}
