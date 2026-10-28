function (options) {
  const source = options && typeof options.source === "string" ? options.source : "";
  const error = (code, message) => ({ error: { code, message } });
  const scalar = (value) => value === undefined ? null : value;
  const text = (value) => typeof value === "string" ? value.trim() : "";
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
  const tokenOf = (element) => {
    if (!(element instanceof Element)) return "";
    return [element.id, element.getAttribute("data-e2e"), element.className]
      .filter((value) => typeof value === "string")
      .join(" ")
      .toLowerCase();
  };
  const hotLabels = new Set(["\u70ed\u95e8", "\u63a8\u8350", "\u4e3a\u4f60\u63a8\u8350", "\u731c\u4f60\u559c\u6b22"]);
  const excludedRegion = (element) => {
    let current = element;
    for (let depth = 0; current && depth < 8; depth += 1, current = current.parentElement) {
      if (current.matches("aside,footer")) return true;
      if (current.matches("nav") && /navigation|sidebar|top[-_]nav/.test(tokenOf(current))) return true;
      if (/(^|[-_ ])(hot|popular|recommend|recommended|related|guess|sidebar|navigation)([-_ ]|$)|hotlist|recommendlist/.test(tokenOf(current))) return true;
      for (const sibling of Array.from(current.parentElement?.children || [])) {
        if (sibling === current) break;
        if (hotLabels.has(text(sibling.textContent))) return true;
      }
    }
    return false;
  };
  const contentLinkCache = new WeakMap();
  const contentLink = (element) => {
    if (!(element instanceof Element)) return null;
    if (contentLinkCache.has(element)) return contentLinkCache.get(element);
    const anchors = [];
    if (element.matches("a[href]")) anchors.push(element);
    anchors.push(...Array.from(element.querySelectorAll("a[href]")));
    let first = null;
    for (const anchor of anchors) {
      try {
        const url = new URL(anchor.href, location.href);
        const host = url.hostname.toLowerCase();
        const match = url.pathname.match(/^\/(video|note|article)\/(\d+)\/?$/);
        if ((host === "www.douyin.com" || host === "douyin.com") && match) {
          const result = { anchor, href: url.href, id: match[2] };
          if (!first) {
            first = result;
          } else if (first.id !== result.id) {
            contentLinkCache.set(element, null);
            return null;
          }
        }
      } catch (_) {}
    }
    contentLinkCache.set(element, first);
    return first;
  };
  const sourceExactSelector = source === "likes"
    ? '[data-e2e="user-like-list"]'
    : '[data-e2e="user-favorite-list"]';
  const sourceRoot = document.querySelector(sourceExactSelector)
    || document.querySelector("#douyin-right-container")
    || document.querySelector('[data-e2e="user-detail"]')
    || document.querySelector("main")
    || document.body;

  const listElements = (root) => {
    const selectors = "ul,ol,[role=\"list\"],main,section,div";
    const scoped = root ? [root, ...Array.from(root.querySelectorAll(selectors))] : [];
    const global = [
      ...Array.from(document.querySelectorAll('ul[data-e2e="scroll-list"],ol,[role="list"]')),
      ...Array.from(document.querySelectorAll("main,section,div"))
    ];
    return Array.from(new Set([...scoped, ...global]));
  };
  const listCandidates = listElements(sourceRoot).map((candidate) => {
    const cards = Array.from(candidate.children).filter((element) => contentLink(element));
    const exactList = candidate.matches('ul[data-e2e="scroll-list"]');
    let score = cards.length * 1000;
    if (excludedRegion(candidate)) return null;
    if (!rendered(candidate)) return null;
    const parentCards = candidate.parentElement
      ? Array.from(candidate.parentElement.children).filter((element) => contentLink(element)).length
      : 0;
    if (!exactList && contentLink(candidate) && parentCards > 0) {
      if (parentCards > 1) return null;
      score = -500;
    }
    if (!exactList && contentLink(candidate) && Array.from(candidate.children).some((child) =>
      child.matches("a[href]") && contentLink(child)
    )) return null;
    if (cards.length === 0 && !exactList) return null;
    const allCards = Array.from(candidate.children).filter((element) => contentLink(element));
    if (exactList) score += 50_000;
    if (sourceRoot && sourceRoot.contains(candidate)) score += 10_000;
    if (visible(candidate)) score += 100;
    if (cards.length > 0 && cards.length === candidate.children.length) score += 500;
    score += Math.min(200, candidate.getBoundingClientRect().width);
    return { candidate, cards: allCards, score };
  }).filter(Boolean).sort((left, right) => right.score - left.score);
  if (listCandidates.length > 1 && listCandidates[0].score === listCandidates[1].score) {
    return error("ambiguous_archive_list", "Multiple archive lists have the same source-local score");
  }
  const listEntry = listCandidates[0] || (() => {
    const exactLists = Array.from(document.querySelectorAll('ul[data-e2e="scroll-list"]'))
      .filter((element) => !excludedRegion(element));
    return exactLists.length === 1 ? { candidate: exactLists[0], cards: [], score: 0 } : null;
  })();
  if (!listEntry) return error("archive_list_not_found", "Expected one archive content list");
  const list = listEntry.candidate;
  const cards = listEntry.cards;

  const scrollable = (element) => element instanceof HTMLElement && /(auto|scroll|overlay)/.test(getComputedStyle(element).overflowY);
  const nearestScrollTarget = (anchor) => {
    const route = anchor instanceof Element ? anchor.closest(".route-scroll-container") : null;
    if (scrollable(route)) return route;
    let current = anchor instanceof HTMLElement ? anchor.parentElement : null;
    while (current) {
      if (scrollable(current)) return current;
      current = current.parentElement;
    }
    if (scrollable(sourceRoot)) return sourceRoot;
    return document.scrollingElement;
  };
  const scrollTarget = nearestScrollTarget(sourceRoot || list);
  const visibleText = (scrollTarget ? [scrollTarget, ...scrollTarget.querySelectorAll("*")] : [])
    .filter(visible)
    .map((element) => text(element.textContent));
  const noMoreVisible = visibleText.some((value) => value === "暂时没有更多了");
  const loadingVisible = visibleText.some((value) => /^(加载中|正在加载)/.test(value));
  const metrics = {
    scrollTop: scrollTarget ? scrollTarget.scrollTop : window.scrollY,
    scrollHeight: scrollTarget ? scrollTarget.scrollHeight : document.documentElement.scrollHeight,
    clientHeight: scrollTarget ? scrollTarget.clientHeight : window.innerHeight,
    noMoreVisible,
    loadingVisible
  };

  const fiberFor = (element) => {
    const key = Object.getOwnPropertyNames(element).find((name) => name.startsWith("__reactFiber$"));
    return key ? element[key] : null;
  };
  const idOf = (info) => {
    if (!info || typeof info !== "object") return null;
    for (const key of ["awemeIdStr", "aweme_id_str", "awemeId", "aweme_id", "id"]) {
      const value = info[key];
      if (typeof value === "string" && value.trim() && /^\d+$/.test(value.trim())) return value.trim();
      if (typeof value === "number" && Number.isSafeInteger(value) && value >= 0) return String(value);
    }
    return null;
  };
  const infoFromEntry = (entry) => {
    if (!entry || typeof entry !== "object") return null;
    const props = entry.props && typeof entry.props === "object" ? entry.props : null;
    const candidates = [
      entry,
      props && props.itemInfo,
      props && props.item_info,
      props && props.awemeInfo,
      props && props.aweme_info,
      props && props.aweme,
      props && props.item,
      entry.itemInfo,
      entry.item_info,
      entry.awemeInfo,
      entry.aweme_info,
      entry.aweme,
      entry.item
    ];
    return candidates.find((candidate) => candidate && typeof candidate === "object" && idOf(candidate)) || null;
  };
  const arraysIn = (value, depth, seen, output) => {
    if (!value || typeof value !== "object" || depth > 4 || seen.has(value)) return;
    seen.add(value);
    if (Array.isArray(value)) {
      output.push(value);
      return;
    }
    for (const key of [
      "items", "itemList", "item_list", "awemeList", "aweme_list", "awemeInfos", "aweme_infos",
      "records", "children", "videos", "contentList", "content_list", "feedList", "feed_list",
      "list", "data", "listData", "list_data",
      "memoizedState", "state", "payload"
    ]) {
      if (value[key] && typeof value[key] === "object") arraysIn(value[key], depth + 1, seen, output);
    }
  };
  const itemCandidates = [];
  if (cards.length > 0) {
    let fiber = fiberFor(cards[0]);
    if (!fiber) return error("react_fiber_not_found", "Archive card has no React Fiber property");
    const domIds = cards.map((card) => contentLink(card).id);
    const seenArrays = new Set();
    let depth = 0;
    while (fiber && depth < 100) {
      for (const node of [fiber, fiber.alternate]) {
        if (!node) continue;
        const arrays = [];
        arraysIn(node.memoizedProps, 0, new Set(), arrays);
        arraysIn(node.pendingProps, 0, new Set(), arrays);
        arraysIn(node.memoizedState, 0, new Set(), arrays);
        for (const entries of arrays) {
          if (seenArrays.has(entries) || entries.length !== cards.length) continue;
          seenArrays.add(entries);
          const infos = entries.map(infoFromEntry);
          if (infos.some((info) => !info)) continue;
          const ids = infos.map(idOf);
          if (!ids.every((id, index) => id === domIds[index])) continue;
          const score = infos.reduce((total, info) => total + [
            info.awemeType ?? info.aweme_type,
            info.createTime ?? info.create_time ?? info.publishTime ?? info.publish_time,
            info.authorInfo ?? info.author ?? info.author_info ?? info.author_user_info,
            info.itemTitle ?? info.item_title ?? info.title ?? info.text,
            info.desc,
            info.video,
            info.stats ?? info.statistics
          ].filter((value) => value !== undefined && value !== null).length, 0);
          itemCandidates.push({ infos, ids, score });
        }
      }
      fiber = fiber.return;
      depth += 1;
    }
  }
  const signatures = new Map();
  for (const candidate of itemCandidates) {
    const signature = JSON.stringify(candidate.ids);
    const existing = signatures.get(signature);
    if (!existing || candidate.score > existing.score) signatures.set(signature, candidate);
  }
  if (cards.length > 0 && signatures.size === 0) {
    return error("archive_react_list_not_found", "No React item array matches DOM order and IDs");
  }
  if (signatures.size > 1) return error("ambiguous_archive_react_list", "Multiple React lists match the archive DOM");
  const selected = signatures.size === 1 ? Array.from(signatures.values())[0] : null;
  const items = selected ? selected.infos.map((info, index) => {
    const author = info.authorInfo ?? info.author ?? info.author_info ?? info.author_user_info ?? null;
    const video = info.video ?? null;
    const stats = info.stats ?? info.statistics ?? null;
    return {
      awemeId: idOf(info),
      awemeType: scalar(info.awemeType ?? info.aweme_type),
      remoteCreatedAt: scalar(info.createTime ?? info.create_time ?? info.publishTime ?? info.publish_time),
      href: contentLink(cards[index]).href,
      sourcePosition: index,
      authorName: scalar(author && (author.nickname ?? author.name)),
      itemTitle: scalar(info.itemTitle ?? info.item_title ?? info.title ?? info.text),
      desc: scalar(info.desc),
      coverUrls: scalar(video && (video.coverUrlList ?? video.cover_url_list ?? video.cover ?? video.originCover ?? video.origin_cover)),
      likeCount: scalar(stats && (stats.diggCount ?? stats.digg_count ?? stats.likeCount ?? stats.like_count))
    };
  }) : [];

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
  const folderInfoFromEntry = (entry) => {
    if (!entry || typeof entry !== "object") return null;
    const isFolderObject = (value) => {
      if (!value || typeof value !== "object") return false;
      const marker = [
        "collects_id_str", "collectsIdStr", "collects_id", "collectsId",
        "collectionFolderId", "collection_folder_id", "remoteFolderId", "remote_folder_id",
        "collectionId", "collection_id", "folderId", "folder_id"
      ].some((key) => Object.prototype.hasOwnProperty.call(value, key));
      return (marker || (Object.prototype.hasOwnProperty.call(value, "id") && Object.prototype.hasOwnProperty.call(value, "name")))
        && !!folderId(value) && !!folderName(value);
    };
    const props = entry.props && typeof entry.props === "object" ? entry.props : null;
    const candidates = [
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
    ];
    return candidates.find(isFolderObject) || null;
  };
  const folderArraysIn = (value, depth, seen, output) => {
    if (!value || typeof value !== "object" || depth > 4 || seen.has(value)) return;
    seen.add(value);
    if (Array.isArray(value)) {
      output.push(value);
      return;
    }
    for (const key of [
      "folders", "collections", "collects_list", "collectsList", "collectionList", "collection_list",
      "folderList", "folder_list", "list", "data", "children", "items", "props"
    ]) {
      if (value[key] && typeof value[key] === "object") folderArraysIn(value[key], depth + 1, seen, output);
    }
  };
  const folderControls = () => {
    const elements = Array.from(document.querySelectorAll(
      'button,a,li,div,[role="button"],[role="tab"],[data-e2e],[data-collects-id],[data-collects-id-str],[data-collection-id],[data-folder-id],[data-id]'
    ));
    return elements.filter((element) => {
      if (!visible(element) || contentLink(element)) return false;
      const token = tokenOf(element);
      const label = text(element.textContent);
      return /folder|collect|compilation|favorite/.test(token)
        || element.hasAttribute("data-collects-id")
        || element.hasAttribute("data-collects-id-str")
        || element.hasAttribute("data-collection-id")
        || element.hasAttribute("data-folder-id")
        || element.hasAttribute("data-id")
        || label === "收藏夹"
        || label === "收藏";
    }).slice(0, 1_000);
  };
  const folderElementId = (element) => {
    for (const key of ["data-collects-id-str", "data-collects-id", "data-collection-id", "data-folder-id", "data-id"]) {
      const value = text(element.getAttribute(key));
      if (value) return value;
    }
    const href = element.getAttribute("href");
    if (href) {
      try {
        const url = new URL(href, location.href);
        for (const key of ["collects_id_str", "collects_id", "collectsId", "collection_id", "folder_id"]) {
          const value = text(url.searchParams.get(key));
          if (value) return value;
        }
      } catch (_) {}
    }
    return null;
  };
  const controls = folderControls();
  let remoteFolders = [];
  let domFolderCount = 0;
  let activeRemoteFolderId = null;
  let activeRemoteFolderName = null;
  if ((source === "favorites" && options.discoverRemoteFolders) || source === "remoteFolder") {
    const candidates = [];
    for (const element of controls) {
      let fiber = fiberFor(element);
      let depth = 0;
      while (fiber && depth < 80) {
        for (const node of [fiber, fiber.alternate]) {
          if (!node) continue;
          const arrays = [];
          folderArraysIn(node.memoizedProps, 0, new Set(), arrays);
          folderArraysIn(node.pendingProps, 0, new Set(), arrays);
          for (const entries of arrays) {
            const infos = entries.map(folderInfoFromEntry);
            if (infos.some((info) => !info)) continue;
            const ids = infos.map(folderId);
            if (ids.some((id) => !id) || new Set(ids).size !== ids.length) continue;
            const matching = infos.reduce((count, info) => count + controls.some((candidate) =>
              folderElementId(candidate) === folderId(info) || text(candidate.textContent) === folderName(info)
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
    const orderedCandidates = Array.from(distinct.values()).sort((left, right) => right.score - left.score);
    const selectedFolders = orderedCandidates[0]?.infos || [];
    if (orderedCandidates.length > 1 && orderedCandidates[0].score === orderedCandidates[1].score) {
      return error("ambiguous_remote_folder_react_list", "Multiple React folder lists have the same score");
    }
    if (selectedFolders.length > 0) {
      remoteFolders = selectedFolders.map((info, index) => ({
        remoteFolderId: folderId(info),
        name: folderName(info),
        sourcePosition: index
      }));
    } else {
      const explicit = controls.map((element) => ({
        id: folderElementId(element),
        name: text(element.textContent),
        element
      })).filter((entry) => entry.id && entry.name);
      const ids = new Set();
      remoteFolders = explicit.filter((entry) => !ids.has(entry.id) && ids.add(entry.id)).map((entry, index) => ({
        remoteFolderId: entry.id,
        name: entry.name,
        sourcePosition: index
      }));
    }
    domFolderCount = remoteFolders.length;
    for (const folder of remoteFolders) {
      const element = controls.find((candidate) =>
        folderElementId(candidate) === folder.remoteFolderId || text(candidate.textContent) === folder.name
      );
      if (element && (
        element.getAttribute("aria-selected") === "true"
        || element.getAttribute("aria-current") === "page"
        || element.dataset.active === "true"
        || /(^|[-_ ])(active|selected)([-_ ]|$)/.test(tokenOf(element))
      )) {
        activeRemoteFolderId = folder.remoteFolderId;
        activeRemoteFolderName = folder.name;
        break;
      }
    }
  }
  if (source === "remoteFolder") {
    if (!options.requestedRemoteFolderId || (activeRemoteFolderId !== null && String(activeRemoteFolderId) !== options.requestedRemoteFolderId)) {
      return error("remote_folder_not_selected", "Requested remote folder is not active");
    }
  }
  if (cards.length === 0 && !noMoreVisible) {
    return error("empty_archive_unconfirmed", "Archive list is empty without a no-more signal");
  }
  return {
    kind: cards.length === 0 ? "empty" : "records",
    items,
    remoteFolders,
    activeRemoteFolderId,
    activeRemoteFolderName,
    domCardCount: cards.length,
    domFolderCount,
    ...metrics
  };
}
