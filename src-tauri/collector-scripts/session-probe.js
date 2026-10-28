(() => {
  function readPath(root, path) {
    let value = root;
    for (const segment of path) {
      if (!value || typeof value !== "object") return null;
      value = value[segment];
    }
    return value;
  }

  function parseRenderData() {
    const raw = globalThis.RENDER_DATA;
    if (typeof raw !== "string" || raw.length > 2_000_000) return null;
    try {
      return JSON.parse(decodeURIComponent(raw));
    } catch {
      return null;
    }
  }

  function reactRootProps() {
    const root = document.querySelector("#root");
    if (!root) return null;
    const key = Object.keys(root).find((name) => name.startsWith("__reactProps$"));
    return key ? root[key] : null;
  }

  function sourceNodes() {
    return [
      ["ssr_render_data", readPath(globalThis.SSR_RENDER_DATA, ["app", "user"])],
      ["initial_state_user", readPath(globalThis.__INITIAL_STATE__, ["user"])],
      ["initial_state_state_user", readPath(globalThis.__INITIAL_STATE__, ["state", "user"])],
      ["router_data", readPath(globalThis._ROUTER_DATA, ["loaderData", "user"])],
      ["next_data", readPath(globalThis.__NEXT_DATA__, ["props", "pageProps", "user"])],
      ["render_data", readPath(parseRenderData(), ["app", "user"])],
      ["react_root_props", readPath(reactRootProps(), ["children", "props", "user"])],
    ];
  }

  function normalizedString(value) {
    if (typeof value !== "string") return null;
    const trimmed = value.trim();
    return trimmed || null;
  }

  function loginFlag(node) {
    const camel = node.isLogin;
    const snake = node.is_login;
    if (typeof camel === "boolean" && typeof snake === "boolean" && camel !== snake) {
      return null;
    }
    if (typeof camel === "boolean") return camel;
    if (typeof snake === "boolean") return snake;
    return null;
  }

  function identityObject(node) {
    const candidates = [
      node.info,
      node.userInfo,
      node.user_info,
      node.currentUser,
      node.current_user,
    ];
    return candidates.find((value) => value && typeof value === "object") ?? null;
  }

  function sourceEvidence(source, node) {
    if (!node || typeof node !== "object") return null;
    const loggedIn = loginFlag(node);
    const info = identityObject(node);
    if (loggedIn === null || !info) return null;
    if (!loggedIn) return { source, loggedIn: false };
    const uid = normalizedString(info.uid);
    const secUid = normalizedString(info.secUid ?? info.sec_uid);
    if (!uid && !secUid) return null;
    return {
      source,
      loggedIn: true,
      uid,
      secUid,
      nickname: normalizedString(info.nickname ?? info.nickName),
      avatarUrl: normalizedString(info.avatarUrl ?? info.avatar_url),
    };
  }

  function validProbeUrl(value) {
    try {
      const url = new URL(value);
      return (
        url.protocol === "https:" &&
        url.hostname === "www.douyin.com" &&
        !url.username &&
        !url.password &&
        (!url.port || url.port === "443") &&
        (url.pathname === "/" ||
          url.pathname === "/jingxuan" ||
          (url.pathname === "/user/self" && ["record", "like", "favorite_collection"].includes(url.searchParams.get("showTab")))) &&
        !url.hash
      );
    } catch {
      return false;
    }
  }

  function identitiesConflict(left, right) {
    return (
      (left.uid && right.uid && left.uid !== right.uid) ||
      (left.secUid && right.secUid && left.secUid !== right.secUid)
    );
  }

  function identitiesOverlap(left, right) {
    return Boolean(
      (left.uid && right.uid && left.uid === right.uid) ||
        (left.secUid && right.secUid && left.secUid === right.secUid),
    );
  }

  function identitiesAreLinked(identities) {
    if (identities.length < 2) return true;
    const visited = new Set([0]);
    let changed = true;
    while (changed) {
      changed = false;
      for (let index = 0; index < identities.length; index += 1) {
        if (visited.has(index)) continue;
        if ([...visited].some((other) => identitiesOverlap(identities[index], identities[other]))) {
          visited.add(index);
          changed = true;
        }
      }
    }
    return visited.size === identities.length;
  }

  const rawPageUrl = String(location.href);
  if (!validProbeUrl(rawPageUrl)) {
    return { kind: "indeterminate", pageUrl: rawPageUrl, reason: "invalid_home_url" };
  }
  const pageUrl = new URL(rawPageUrl).href;
  const evidence = sourceNodes()
    .map(([source, node]) => sourceEvidence(source, node))
    .filter(Boolean);
  if (!evidence.length) {
    return { kind: "indeterminate", pageUrl, reason: "no_valid_session_source" };
  }
  const authenticated = evidence.filter((item) => item.loggedIn);
  const unauthenticated = evidence.filter((item) => !item.loggedIn);
  if (authenticated.length && unauthenticated.length) {
    return { kind: "conflicting", pageUrl };
  }
  if (!authenticated.length) {
    return { kind: "unauthenticated", pageUrl, source: unauthenticated[0].source };
  }
  for (let left = 0; left < authenticated.length; left += 1) {
    for (let right = left + 1; right < authenticated.length; right += 1) {
      if (identitiesConflict(authenticated[left], authenticated[right])) {
        return { kind: "conflicting", pageUrl };
      }
    }
  }
  if (!identitiesAreLinked(authenticated)) {
    return { kind: "indeterminate", pageUrl, reason: "identity_sources_unlinked" };
  }
  const first = (name) => authenticated.find((item) => item[name])?.[name] ?? null;
  return {
    kind: "authenticated",
    pageUrl,
    identity: {
      uid: first("uid"),
      secUid: first("secUid"),
      nickname: first("nickname"),
      avatarUrl: first("avatarUrl"),
      sources: authenticated.map((item) => item.source),
    },
  };
})()
