(() => {
  const readPath = (root, path) => {
    let value = root;
    for (const segment of path) {
      if (!value || typeof value !== "object") return null;
      value = value[segment];
    }
    return value;
  };

  const parseRenderData = () => {
    const raw = globalThis.RENDER_DATA;
    if (typeof raw !== "string" || raw.length > 2_000_000) return null;
    try {
      return JSON.parse(decodeURIComponent(raw));
    } catch {
      return null;
    }
  };

  const reactRootProps = () => {
    const root = document.querySelector("#root");
    if (!root) return null;
    const key = Object.keys(root).find((name) => name.startsWith("__reactProps$"));
    return key ? root[key] : null;
  };

  const identitySourcePaths = () => {
    const fixture = globalThis.__EMILY_ACCOUNT_IDENTITY_FIXTURE__;
    if (fixture) {
      return [
        [fixture, ["state", "user"]],
        [fixture, ["user"]],
        [fixture, []],
      ];
    }
    return [
      [globalThis.SSR_RENDER_DATA, ["app", "user"]],
      [globalThis.__INITIAL_STATE__, ["user"]],
      [globalThis.__INITIAL_STATE__, ["state", "user"]],
      [globalThis._ROUTER_DATA, ["loaderData", "user"]],
      [globalThis.__NEXT_DATA__, ["props", "pageProps", "user"]],
      [parseRenderData(), ["app", "user"]],
      [reactRootProps(), ["children", "props", "user"]],
    ];
  };

  const normalizedString = (value) => {
    if (typeof value !== "string") return null;
    const trimmed = value.trim();
    return trimmed || null;
  };

  const candidateFromSource = (source) => {
    if (!source || typeof source !== "object") return null;
    const loggedIn = source.isLogin === true || source.is_login === true;
    const userInfo =
      source.info ?? source.userInfo ?? source.user_info ?? source.currentUser ?? source.current_user;
    if (!loggedIn || !userInfo || typeof userInfo !== "object") return null;
    const uid = normalizedString(userInfo.uid);
    const secUid = normalizedString(userInfo.secUid ?? userInfo.sec_uid);
    if (!uid && !secUid) return null;
    return {
      uid,
      secUid,
      nickname: normalizedString(userInfo.nickname ?? userInfo.nickName),
      avatarUrl: normalizedString(userInfo.avatarUrl ?? userInfo.avatar_url),
    };
  };

  let pageUrl;
  try {
    const url = new URL(location.href);
    if (
      url.hostname !== "www.douyin.com" ||
      url.pathname !== "/user/self" ||
      url.searchParams.get("showTab") !== "record"
    ) return null;
    pageUrl = url.href;
  } catch {
    return null;
  }

  const candidates = identitySourcePaths()
    .map(([root, path]) => candidateFromSource(readPath(root, path)))
    .filter(Boolean);
  if (!candidates.length) return null;

  const uidValues = new Set(candidates.map((candidate) => candidate.uid).filter(Boolean));
  const secUidValues = new Set(candidates.map((candidate) => candidate.secUid).filter(Boolean));
  if (uidValues.size > 1 || secUidValues.size > 1) {
    return { pageUrl, error: "account_identity_ambiguous" };
  }
  return { pageUrl, state: { candidates } };
})()
