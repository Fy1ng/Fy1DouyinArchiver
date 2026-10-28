(() => {
  const visible = (element) => {
    if (!(element instanceof HTMLElement)) return false;
    const rect = element.getBoundingClientRect();
    const style = window.getComputedStyle(element);
    return !element.hidden && style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const error = (code, message) => ({ error: { code, message } });
  const scrollable = (element) => element instanceof HTMLElement && /(auto|scroll|overlay)/.test(window.getComputedStyle(element).overflowY);
  const nearestScrollTarget = (anchor) => {
    const route = anchor.closest(".route-scroll-container");
    if (scrollable(route)) return route;
    let current = anchor.parentElement;
    while (current) {
      if (scrollable(current)) return current;
      current = current.parentElement;
    }
    return document.scrollingElement;
  };
  const lists = Array.from(document.querySelectorAll('ul[data-e2e="scroll-list"]'));
  if (lists.length !== 1) return error("history_list_not_found", "Expected one history scroll list");
  const list = lists[0];
  const cards = Array.from(list.querySelectorAll(':scope > li.record_card'));
  const scrollTarget = nearestScrollTarget(list);
  const textElements = scrollTarget ? [scrollTarget, ...scrollTarget.querySelectorAll("*")] : [];
  const noMoreVisible = textElements.some((element) => visible(element) && (element.innerText || "").trim() === "暂时没有更多了");
  const loadingVisible = textElements.some((element) => visible(element) && /^(加载中|正在加载)/.test((element.innerText || "").trim()));
  const metrics = {
    scrollTop: scrollTarget ? scrollTarget.scrollTop : window.scrollY,
    scrollHeight: scrollTarget ? scrollTarget.scrollHeight : document.documentElement.scrollHeight,
    clientHeight: scrollTarget ? scrollTarget.clientHeight : window.innerHeight,
    noMoreVisible,
    loadingVisible
  };
  if (cards.length === 0) {
    return noMoreVisible
      ? { kind: "empty", items: [], domCardCount: 0, ...metrics }
      : error("empty_history_unconfirmed", "History list is empty without a no-more signal");
  }

  const fiberKey = Object.getOwnPropertyNames(cards[0]).find((key) => key.startsWith("__reactFiber$"));
  if (!fiberKey) return error("react_fiber_not_found", "History card has no React Fiber property");
  const arrays = [];
  const seenArrays = new Set();
  const inspectProps = (props) => {
    if (!Array.isArray(props) || seenArrays.has(props)) return;
    seenArrays.add(props);
    if (props.length > 0 && props.every((entry) => entry && entry.props && entry.props.itemInfo)) arrays.push(props);
  };
  let fiber = cards[0][fiberKey];
  let depth = 0;
  while (fiber && depth < 80) {
    inspectProps(fiber.memoizedProps);
    inspectProps(fiber.pendingProps);
    if (fiber.alternate) {
      inspectProps(fiber.alternate.memoizedProps);
      inspectProps(fiber.alternate.pendingProps);
    }
    fiber = fiber.return;
    depth += 1;
  }
  const exact = arrays.filter((items) => items.length === cards.length);
  if (exact.length === 0) {
    return error(arrays.length > 0 ? "unsupported_virtualized_list" : "react_list_not_found", "No React item array matches the DOM card count");
  }
  const signatures = new Map();
  for (const items of exact) {
    const signature = JSON.stringify(items.map((entry) => [entry.props.itemInfo.awemeId, entry.props.itemInfo.historyTime]));
    if (!signatures.has(signature)) signatures.set(signature, items);
  }
  if (signatures.size !== 1) return error("ambiguous_react_list", "Multiple distinct React item arrays match the history list");
  const items = Array.from(signatures.values())[0];
  return {
    kind: "records",
    domCardCount: cards.length,
    items: items.map((entry, index) => {
      const info = entry.props.itemInfo;
      const anchor = cards[index].querySelector("a[href]");
      return {
        awemeId: info.awemeId,
        awemeType: info.awemeType,
        historyTime: info.historyTime,
        href: anchor ? anchor.href : "",
        sourcePosition: index,
        authorName: info.authorInfo && info.authorInfo.nickname,
        itemTitle: info.itemTitle,
        desc: info.desc,
        coverUrls: info.video && info.video.coverUrlList,
        likeCount: info.stats && info.stats.diggCount,
        historyProgressRate: info.historyProgressRate
      };
    }),
    ...metrics
  };
})()
