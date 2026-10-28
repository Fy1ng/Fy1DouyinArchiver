function (view) {
  const expected = view === "folders" ? "收藏夹" : view === "favorites" ? "收藏" : "";
  const error = view === "folders" ? "favorite_folder_tab_not_found" : "favorite_tab_not_found";
  if (!expected) return { error: "invalid_favorite_view" };
  const text = (value) => typeof value === "string" ? value.replace(/\s+/g, "").trim() : "";
  const visible = (element) => {
    if (!(element instanceof HTMLElement)) return false;
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return !element.hidden && style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const tokenOf = (element) => [element.id, element.getAttribute("data-e2e"), element.className]
    .filter((value) => typeof value === "string")
    .join(" ")
    .toLowerCase();
  const hasContentLink = (element) => Array.from(element.querySelectorAll("a[href]")).some((anchor) => {
    try {
      const url = new URL(anchor.href, location.href);
      return (url.hostname === "www.douyin.com" || url.hostname === "douyin.com")
        && /^\/(video|note|article)\/\d+\/?$/.test(url.pathname);
    } catch (_) {
      return false;
    }
  });
  const clickable = (element) => {
    if (!(element instanceof Element)) return null;
    const direct = element.closest('button,a,[role="tab"],[role="button"],li');
    if (direct && visible(direct)) return direct;
    let current = element;
    for (let depth = 0; depth < 5 && current instanceof HTMLElement; depth += 1) {
      if (visible(current) && (current.tabIndex >= 0 || current.onclick || getComputedStyle(current).cursor === "pointer")) return current;
      current = current.parentElement;
    }
    return visible(element) ? element : null;
  };
  const candidates = [];
  if (view === "folders" || view === "favorites") {
    const stableSelectors = view === "folders"
      ? [
        "#semiTabfavorite_collection",
        '[data-e2e="favorite-collection-tab"]',
        '[data-e2e="favorite-collection"]'
      ]
      : [
        "#semiTabfavorite",
        '[data-e2e="user-favorite-tab"]',
        '[data-e2e="favorite-tab"]'
      ];
    for (const selector of stableSelectors) {
      const element = document.querySelector(selector);
      if (element) candidates.push(element);
    }
  }
  const controls = Array.from(document.querySelectorAll(
    'button,a,[role="tab"],[role="button"],li,span,div'
  ));
  for (const element of controls) {
    if (!visible(element) || text(element.textContent) !== expected) continue;
    if (view === "favorites" && hasContentLink(element)) continue;
    const token = tokenOf(element);
    if (view === "folders" && !(
      /collect|folder|compilation/.test(token)
      || element.getAttribute("role") === "tab"
      || element.id === "semiTabfavorite_collection"
    )) continue;
    if (element.matches("div") && !/tab|favorite|collect|folder|compilation/.test(token)) continue;
    candidates.push(element);
  }
  const unique = Array.from(new Set(candidates)).map(clickable).filter(Boolean);
  if (unique.length === 0) return { error };
  const ranked = unique.map((element) => {
    const rect = element.getBoundingClientRect();
    const token = tokenOf(element);
    let score = 0;
    if (element.id === "semiTabfavorite_collection") score += 10_000;
    if (element.getAttribute("role") === "tab") score += 1_000;
    if (element.matches("button,a,li")) score += 500;
    if (/favorite|collect|folder|compilation/.test(token)) score += 250;
    if (element.closest("main,#douyin-right-container,[data-e2e=\"user-detail\"]")) score += 100;
    score -= Math.min(200, rect.width * rect.height / 10000);
    return { element, rect, score };
  }).sort((left, right) => right.score - left.score);
  const best = ranked[0].element;
  try {
    best.scrollIntoView({ block: "center", inline: "center" });
  } catch (_) {}
  const rect = best.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0 || rect.right <= 0 || rect.bottom <= 0 || rect.left >= innerWidth || rect.top >= innerHeight) {
    return { error: "favorite_navigation_not_visible" };
  }
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
}
