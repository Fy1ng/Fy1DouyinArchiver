function (source) {
  const visible = (element) => {
    if (!(element instanceof HTMLElement)) return false;
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return !element.hidden && style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const rendered = (element) => {
    if (!(element instanceof HTMLElement)) return false;
    const style = getComputedStyle(element);
    return !element.hidden && style.display !== "none" && style.visibility !== "hidden";
  };
  const tokenOf = (element) => [element.id, element.getAttribute("data-e2e"), element.className]
    .filter((value) => typeof value === "string")
    .join(" ")
    .toLowerCase();
  const hotLabels = new Set(["\u70ed\u95e8", "\u63a8\u8350", "\u4e3a\u4f60\u63a8\u8350", "\u731c\u4f60\u559c\u6b22"]);
  const excludedRegion = (element) => {
    let current = element;
    for (let depth = 0; current && depth < 8; depth += 1, current = current.parentElement) {
      if (current.matches("aside,footer")) return true;
      if (current.matches("nav") && /navigation|sidebar|top[-_]nav/.test(tokenOf(current))) return true;
      if (/(^|[-_ ])(hot|popular|recommend|recommended|related|guess|sidebar|navigation)([-_ ]|$)|hotlist|recommendlist/.test(tokenOf(current))) return true;
      for (const sibling of Array.from(current.parentElement?.children || [])) {
        if (sibling === current) break;
        if (hotLabels.has((sibling.textContent || "").trim())) return true;
      }
    }
    return false;
  };
  const contentLinkCache = new WeakMap();
  const contentLink = (element) => {
    if (!(element instanceof Element)) return false;
    if (contentLinkCache.has(element)) return contentLinkCache.get(element);
    const anchors = [];
    if (element.matches("a[href]")) anchors.push(element);
    anchors.push(...Array.from(element.querySelectorAll("a[href]")));
    const ids = new Set();
    anchors.forEach((anchor) => {
      try {
        const url = new URL(anchor.href, location.href);
        const match = url.pathname.match(/^\/(video|note|article)\/(\d+)\/?$/);
        if ((url.hostname === "www.douyin.com" || url.hostname === "douyin.com") && match) ids.add(match[2]);
      } catch (_) {}
    });
    const result = ids.size === 1;
    contentLinkCache.set(element, result);
    return result;
  };
  const sourceExact = source === "likes"
    ? document.querySelector('[data-e2e="user-like-list"]')
    : document.querySelector('[data-e2e="user-favorite-list"]');
  const sourceRoot = sourceExact || document.querySelector("#douyin-right-container") || document.querySelector('[data-e2e="user-detail"]') || document.querySelector("main") || document.body;
  const listElements = [
    sourceRoot,
    ...Array.from(sourceRoot.querySelectorAll("ul,ol,[role=\"list\"],main,section,div")),
    ...Array.from(document.querySelectorAll('ul[data-e2e="scroll-list"],ol,[role="list"]'))
  ];
  const lists = Array.from(new Set(listElements)).map((candidate) => {
    if (!(candidate instanceof HTMLElement) || excludedRegion(candidate) || !rendered(candidate)) return null;
    const cards = Array.from(candidate.children).filter(contentLink);
    const exact = candidate.matches('ul[data-e2e="scroll-list"]');
    let score = cards.length * 1000;
    if (cards.length === 0 && !exact) return null;
    const parentCards = candidate.parentElement
      ? Array.from(candidate.parentElement.children).filter((element) => contentLink(element)).length
      : 0;
    if (!exact && contentLink(candidate) && parentCards > 0) {
      if (parentCards > 1) return null;
      score = -500;
    }
    if (!exact && contentLink(candidate) && Array.from(candidate.children).some((child) =>
      child.matches("a[href]") && contentLink(child)
    )) return null;
    if (exact) score += 50_000;
    if (sourceRoot.contains(candidate)) score += 10_000;
    if (visible(candidate)) score += 100;
    if (cards.length > 0 && cards.length === candidate.children.length) score += 500;
    return { candidate, cards, score };
  }).filter(Boolean).sort((left, right) => right.score - left.score);
  const list = lists[0]?.candidate || null;
  const recordCount = lists[0]?.cards.length || 0;
  const scrollable = (element) => element instanceof HTMLElement && /(auto|scroll|overlay)/.test(getComputedStyle(element).overflowY);
  const nearestScrollTarget = (anchor) => {
    let current = anchor instanceof HTMLElement ? anchor : null;
    while (current) {
      if (scrollable(current)) return current;
      current = current.parentElement;
    }
    if (scrollable(sourceRoot)) return sourceRoot;
    const candidates = Array.from(sourceRoot.querySelectorAll("main,section,div,ul,ol"))
      .filter((element) => scrollable(element) && !excludedRegion(element))
      .sort((left, right) => (right.scrollHeight - right.clientHeight) - (left.scrollHeight - left.clientHeight));
    return candidates[0] || document.scrollingElement;
  };
  const target = nearestScrollTarget(list || sourceRoot);
  const scope = target === document.scrollingElement ? document.body : target;
  const scopedElements = [scope, ...scope.querySelectorAll("*")].filter(visible);
  const visibleText = scopedElements.map((element) => (element.textContent || "").trim());
  const targetRect = target === document.scrollingElement
    ? { left: 0, top: 0, right: innerWidth, bottom: innerHeight, width: innerWidth, height: innerHeight }
    : target.getBoundingClientRect();
  const anchorRect = list ? list.getBoundingClientRect() : sourceRoot.getBoundingClientRect();
  const left = Math.max(0, targetRect.left, anchorRect.left);
  const right = Math.min(innerWidth, targetRect.right, anchorRect.right);
  const top = Math.max(0, targetRect.top, anchorRect.top);
  const bottom = Math.min(innerHeight, targetRect.bottom, anchorRect.bottom);
  const x = right > left
    ? left + (right - left) / 2
    : Math.min(innerWidth - 1, Math.max(1, targetRect.left + targetRect.width / 2));
  const y = bottom > top
    ? top + (bottom - top) / 2
    : Math.min(innerHeight - 1, Math.max(1, targetRect.top + targetRect.height / 2));
  const scrollTop = target === document.scrollingElement ? window.scrollY : target.scrollTop;
  const scrollHeight = target === document.scrollingElement ? document.documentElement.scrollHeight : target.scrollHeight;
  const clientHeight = target === document.scrollingElement ? innerHeight : target.clientHeight;
  return {
    x,
    y,
    recordCount,
    scrollTop,
    scrollHeight,
    clientHeight,
    noMoreVisible: visibleText.some((value) => value === "暂时没有更多了"),
    loadingVisible: visibleText.some((value) => /^(加载中|正在加载)/.test(value)),
    canScroll: scrollHeight > clientHeight + 1,
    listFound: !!list
  };
}
