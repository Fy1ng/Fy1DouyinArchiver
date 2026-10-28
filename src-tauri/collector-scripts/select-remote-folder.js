function (request) {
  const requestedRemoteFolderId = typeof request === "string"
    ? request
    : request && request.requestedRemoteFolderId;
  const requestedRemoteFolderName = typeof request === "object" && request
    ? request.requestedRemoteFolderName
    : null;
  const error = (code) => ({ error: code });
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
    return !!Array.from(element.matches("a[href]") ? [element] : [])
      .concat(Array.from(element.querySelectorAll("a[href]")))
      .find((anchor) => {
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
  const clickable = (element) => {
    if (!(element instanceof Element)) return null;
    const direct = element.closest('button,a,[role="button"],[role="tab"],li');
    if (direct && visible(direct)) return direct;
    let current = element;
    for (let depth = 0; depth < 5 && current instanceof HTMLElement; depth += 1) {
      if (visible(current) && (current.tabIndex >= 0 || current.onclick || getComputedStyle(current).cursor === "pointer")) return current;
      current = current.parentElement;
    }
    return visible(element) ? element : null;
  };
  const center = (element) => {
    try {
      element.scrollIntoView({ block: "center", inline: "center" });
    } catch (_) {}
    const rect = element.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0 || rect.right <= 0 || rect.bottom <= 0 || rect.left >= innerWidth || rect.top >= innerHeight) {
      return null;
    }
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  };

  if (typeof requestedRemoteFolderId !== "string" || !requestedRemoteFolderId.trim()) {
    return error("invalid_remote_folder_id");
  }
  const allControls = Array.from(document.querySelectorAll(
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
      || label === "收藏夹"
      || label === "收藏";
  }).slice(0, 1_000);
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

  const direct = allControls.find((element) => explicitId(element) === requestedRemoteFolderId);
  if (direct) {
    const target = center(clickable(direct) || direct);
    return target || error("remote_folder_not_visible");
  }

  for (const element of allControls) {
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
          const ids = infos.map((info) => info ? folderId(info) : null);
          const index = ids.indexOf(requestedRemoteFolderId);
          if (index < 0) continue;
          const siblings = element.parentElement
            ? Array.from(element.parentElement.children).filter(visible)
            : [];
          if (siblings.length === ids.length) {
            const target = center(clickable(siblings[index]) || siblings[index]);
            return target || error("remote_folder_not_visible");
          }
          if (folderName(infos[index]) === text(requestedRemoteFolderName) || index === 0) {
            const target = center(clickable(element) || element);
            return target || error("remote_folder_not_visible");
          }
        }
      }
      fiber = fiber.return;
      depth += 1;
    }
  }

  if (typeof requestedRemoteFolderName === "string" && requestedRemoteFolderName.trim()) {
    const named = allControls.filter((element) => text(element.textContent) === requestedRemoteFolderName.trim());
    if (named.length === 1) {
      const target = center(clickable(named[0]) || named[0]);
      return target || error("remote_folder_not_visible");
    }
  }
  return error("remote_folder_not_found");
}
