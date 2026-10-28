function () {
  const error = (code, message) => ({ error: { code, message } });
  const text = (value) => typeof value === "string" ? value.trim() : "";
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
  const contentLink = (element) => {
    if (!(element instanceof Element)) return false;
    return Array.from(element.querySelectorAll("a[href]")).some((anchor) => {
      try {
        const url = new URL(anchor.href, location.href);
        return (url.hostname === "www.douyin.com" || url.hostname === "douyin.com")
          && /^\/(video|note|article)\/\d+\/?$/.test(url.pathname);
      } catch (_) {
        return false;
      }
    });
  };
  const folderId = (value) => {
    if (!value || typeof value !== "object") return null;
    for (const key of [
      "collects_id_str", "collectsIdStr", "collects_id", "collectsId",
      "collectionFolderId", "collection_folder_id", "remoteFolderId", "remote_folder_id",
      "collectionId", "collection_id", "folderId", "folder_id", "id"
    ]) {
      const candidate = value[key];
      if (typeof candidate === "string" && candidate.trim()) return candidate.trim();
      if (typeof candidate === "number" && Number.isSafeInteger(candidate) && candidate >= 0) return String(candidate);
    }
    return null;
  };
  const folderName = (value) => {
    if (!value || typeof value !== "object") return null;
    for (const key of [
      "collects_name", "collectsName", "collectionFolderName", "collection_folder_name",
      "name", "title", "collectionName", "collection_name", "folderName", "folder_name"
    ]) {
      const candidate = text(value[key]);
      if (candidate) return candidate;
    }
    return null;
  };
  const folderInfo = (entry) => {
    if (!entry || typeof entry !== "object") return null;
    const isFolder = (value) => {
      if (!value || typeof value !== "object") return false;
      const hasMarker = [
        "collects_id_str", "collectsIdStr", "collects_id", "collectsId",
        "collectionFolderId", "collection_folder_id", "remoteFolderId", "remote_folder_id",
        "collectionId", "collection_id", "folderId", "folder_id"
      ].some((key) => Object.prototype.hasOwnProperty.call(value, key));
      return (hasMarker || (Object.prototype.hasOwnProperty.call(value, "id") && Object.prototype.hasOwnProperty.call(value, "name")))
        && !!folderId(value) && !!folderName(value);
    };
    const props = entry.props && typeof entry.props === "object" ? entry.props : null;
    return [
      entry,
      props && props.folderInfo,
      props && props.collectionInfo,
      props && props.collectsInfo,
      props && props.item,
      props && props.folder,
      props && props.collection,
      entry.folderInfo,
      entry.collectionInfo,
      entry.collectsInfo,
      entry.folder,
      entry.collection
    ].find(isFolder) || null;
  };
  const arraysIn = (value, depth, seen, output) => {
    if (!value || typeof value !== "object" || depth > 4 || seen.has(value)) return;
    seen.add(value);
    if (Array.isArray(value)) {
      output.push(value);
      return;
    }
    for (const key of [
      "folders", "collections", "collects_list", "collectsList", "collectionList", "collection_list",
      "folderList", "folder_list", "list", "data", "children", "items", "props",
      "memoizedState", "state", "payload"
    ]) {
      if (value[key] && typeof value[key] === "object") arraysIn(value[key], depth + 1, seen, output);
    }
  };
  const fiberFor = (element) => {
    const key = Object.getOwnPropertyNames(element).find((name) => name.startsWith("__reactFiber$"));
    return key ? element[key] : null;
  };
  const controls = Array.from(document.querySelectorAll(
    'button,a,li,div,[role="button"],[role="tab"],[data-e2e],[data-collects-id],[data-collects-id-str],[data-collection-id],[data-folder-id],[data-id]'
  )).filter((element) => {
    if (!visible(element) || contentLink(element)) return false;
    const label = text(element.textContent);
    const token = tokenOf(element);
    return /folder|collect|compilation|favorite/.test(token)
      || !!element.getAttribute("data-collects-id")
      || !!element.getAttribute("data-collects-id-str")
      || !!element.getAttribute("data-collection-id")
      || !!element.getAttribute("data-folder-id")
      || !!element.getAttribute("data-id")
      || (label && element.matches("button,a,[role=\"button\"],[role=\"tab\"],li"));
  }).slice(0, 1_500);
  const explicitId = (element) => [
    element.getAttribute("data-collects-id-str"),
    element.getAttribute("data-collects-id"),
    element.getAttribute("data-collection-id"),
    element.getAttribute("data-folder-id"),
    element.getAttribute("data-id")
  ].map(text).find(Boolean) || (() => {
    const href = element.getAttribute("href");
    if (!href) return null;
    try {
      const url = new URL(href, location.href);
      return ["collects_id_str", "collects_id", "collectsId", "collection_id", "folder_id"]
        .map((key) => text(url.searchParams.get(key)))
        .find(Boolean) || null;
    } catch (_) {
      return null;
    }
  })();
  const candidates = [];
  for (const element of controls) {
    let fiber = fiberFor(element);
    let depth = 0;
    while (fiber && depth < 80) {
      for (const node of [fiber, fiber.alternate]) {
        if (!node) continue;
        const arrays = [];
        arraysIn(node.memoizedProps, 0, new Set(), arrays);
        arraysIn(node.pendingProps, 0, new Set(), arrays);
        arraysIn(node.memoizedState, 0, new Set(), arrays);
        for (const entries of arrays) {
          const infos = entries.map(folderInfo);
          if (infos.some((info) => !info)) continue;
          const ids = infos.map(folderId);
          if (ids.some((id) => !id) || new Set(ids).size !== ids.length) continue;
          const matching = infos.reduce((count, info) => count + controls.some((candidate) =>
            explicitId(candidate) === folderId(info) || text(candidate.textContent) === folderName(info)
          ), 0);
          candidates.push({ infos, score: entries.length * 100 + matching * 1_000 });
        }
      }
      fiber = fiber.return;
      depth += 1;
    }
  }
  const distinct = new Map();
  for (const candidate of candidates) {
    const signature = JSON.stringify(candidate.infos.map((info) => [folderId(info), folderName(info)]));
    const existing = distinct.get(signature);
    if (!existing || candidate.score > existing.score) distinct.set(signature, candidate);
  }
  const ordered = Array.from(distinct.values()).sort((left, right) => right.score - left.score);
  if (ordered.length > 1 && ordered[0].score === ordered[1].score) {
    return error("ambiguous_remote_folder_react_list", "Multiple React folder lists have the same score");
  }
  let folders = ordered[0]?.infos.map((info, index) => ({
    remoteFolderId: folderId(info),
    name: folderName(info),
    sourcePosition: index
  })) || [];
  if (folders.length === 0) {
    const individual = new Map();
    for (const element of controls) {
      let fiber = fiberFor(element);
      let depth = 0;
      while (fiber && depth < 80) {
        for (const node of [fiber, fiber.alternate]) {
          if (!node) continue;
          for (const props of [node.memoizedProps, node.pendingProps]) {
            const info = folderInfo(props);
            if (info && !individual.has(folderId(info))) individual.set(folderId(info), {
              id: folderId(info),
              name: folderName(info)
            });
          }
        }
        fiber = fiber.return;
        depth += 1;
      }
    }
    folders = Array.from(individual.values()).map((entry, index) => ({
      remoteFolderId: entry.id,
      name: entry.name,
      sourcePosition: index
    }));
  }
  if (folders.length === 0) {
    const seen = new Set();
    folders = controls.map((element) => ({
      remoteFolderId: explicitId(element),
      name: text(element.textContent),
      element
    })).filter((entry) => entry.remoteFolderId && entry.name && !seen.has(entry.remoteFolderId) && seen.add(entry.remoteFolderId)).map((entry, index) => ({
      remoteFolderId: entry.remoteFolderId,
      name: entry.name,
      sourcePosition: index
    }));
  }
  const root = document.querySelector('[data-e2e="favorite-collection-list"]')
    || document.querySelector("#douyin-right-container")
    || document.querySelector("main")
    || document.body;
  const findControl = (folder, index) => folder && (controls.find((element) =>
    explicitId(element) === folder.remoteFolderId || text(element.textContent) === folder.name
  ) || (controls.length === folders.length ? controls[index] : null));
  const anchorElement = findControl(folders[0], 0) || root;
  const scrollable = (element) => element instanceof HTMLElement && /(auto|scroll|overlay)/.test(getComputedStyle(element).overflowY);
  let target = anchorElement instanceof HTMLElement ? anchorElement : null;
  while (target && !scrollable(target)) target = target.parentElement;
  if (!target) {
    target = Array.from(root.querySelectorAll("main,section,div,ul,ol"))
      .filter((element) => scrollable(element) && element.scrollHeight > element.clientHeight)
      .sort((left, right) => (right.scrollHeight - right.clientHeight) - (left.scrollHeight - left.clientHeight))[0]
      || document.scrollingElement;
  }
  const scope = target === document.scrollingElement ? document.body : target;
  const visibleText = [scope, ...scope.querySelectorAll("*")].filter(visible).map((element) => text(element.textContent));
  const noMoreVisible = visibleText.some((value) => value === "暂时没有更多了" || /^(暂无|还没有).*收藏夹/.test(value));
  const loadingVisible = visibleText.some((value) => /^(加载中|正在加载)/.test(value));
  const targetRect = target === document.scrollingElement
    ? { left: 0, top: 0, right: innerWidth, bottom: innerHeight, width: innerWidth, height: innerHeight }
    : target.getBoundingClientRect();
  const anchorRect = anchorElement.getBoundingClientRect();
  const left = Math.max(0, targetRect.left, anchorRect.left);
  const right = Math.min(innerWidth, targetRect.right, anchorRect.right);
  const top = Math.max(0, targetRect.top, anchorRect.top);
  const bottom = Math.min(innerHeight, targetRect.bottom, anchorRect.bottom);
  const anchorX = right > left ? left + (right - left) / 2 : Math.min(innerWidth - 1, Math.max(1, targetRect.left + targetRect.width / 2));
  const anchorY = bottom > top ? top + (bottom - top) / 2 : Math.min(innerHeight - 1, Math.max(1, targetRect.top + targetRect.height / 2));
  const scrollTop = target === document.scrollingElement ? window.scrollY : target.scrollTop;
  const scrollHeight = target === document.scrollingElement ? document.documentElement.scrollHeight : target.scrollHeight;
  const clientHeight = target === document.scrollingElement ? innerHeight : target.clientHeight;
  if (folders.length === 0 && !noMoreVisible && !loadingVisible) {
    return error("remote_folder_list_not_found", "No remote folder list is ready");
  }
  return {
    kind: folders.length > 0 ? "folders" : "empty",
    folders,
    domFolderCount: folders.length,
    scrollTop,
    scrollHeight,
    clientHeight,
    noMoreVisible,
    loadingVisible,
    canScroll: scrollHeight > clientHeight + 1,
    anchorX,
    anchorY
  };
}
