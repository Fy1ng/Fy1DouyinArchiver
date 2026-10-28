function (input) {
  const action = input && input.action;
  const definitions = {
    one_click_login: { text: "一键登录", container: "login" },
    save_login: { text: "保存", container: "save_modal" },
  };
  const definition = definitions[action];
  if (!definition) {
    return { kind: "not_found", action: String(action), reason: "invalid_action", count: 0 };
  }

  function validHomeUrl(value) {
    try {
      const url = new URL(value);
      return (
        url.protocol === "https:" &&
        url.hostname === "www.douyin.com" &&
        !url.username &&
        !url.password &&
        !url.port &&
        (url.pathname === "/" || url.pathname === "/jingxuan") &&
        !url.hash
      );
    } catch {
      return false;
    }
  }

  function normalizedText(element) {
    return String(element.textContent || "").trim();
  }

  function visibleAndEnabled(element) {
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return (
      style.display !== "none" &&
      style.visibility !== "hidden" &&
      style.opacity !== "0" &&
      rect.width > 0 &&
      rect.height > 0 &&
      !element.hidden &&
      !element.disabled &&
      element.getAttribute("aria-disabled") !== "true"
    );
  }

  function validLoginContainer(element) {
    return Boolean(
      element.closest(
        '[data-e2e="top-login-container"], [data-e2e="login-panel"], [data-emily-login-root="true"]',
      ),
    );
  }

  function saveLoginCandidates() {
    return [...document.querySelectorAll(".trust-login-dialog-content")].flatMap((dialog) => {
      if (!visibleAndEnabled(dialog)) {
        return [];
      }
      const titles = [...dialog.querySelectorAll(".trust-login-dialog-title")].filter(
        (title) =>
          visibleAndEnabled(title) && normalizedText(title).startsWith("是否保存登录信息？"),
      );
      if (titles.length !== 1) {
        return [];
      }
      return [...dialog.querySelectorAll("button.trust-login-dialog-button-confirm")].filter(
        (button) => normalizedText(button) === "保存" && visibleAndEnabled(button),
      );
    });
  }

  if (!validHomeUrl(location.href)) {
    return { kind: "not_found", action, reason: "invalid_home_url", count: 0 };
  }

  const candidates =
    definition.container === "login"
      ? [...document.querySelectorAll('button, [role="button"]')].filter(
          (element) =>
            normalizedText(element) === definition.text &&
            visibleAndEnabled(element) &&
            validLoginContainer(element),
        )
      : saveLoginCandidates();

  if (candidates.length !== 1) {
    return {
      kind: "not_found",
      action,
      reason: candidates.length ? "ambiguous_candidate" : "no_candidate",
      count: candidates.length,
    };
  }

  const candidate = candidates[0];
  const rect = candidate.getBoundingClientRect();
  const x = Math.round(rect.left + rect.width / 2);
  const y = Math.round(rect.top + rect.height / 2);
  const hit = document.elementFromPoint(x, y);
  if (!hit || (hit !== candidate && !candidate.contains(hit))) {
    return { kind: "not_found", action, reason: "obscured", count: 1 };
  }
  return {
    kind: "found",
    action,
    x,
    y,
    token: `${action}:${candidate.tagName.toLowerCase()}:${x}:${y}`,
  };
}
