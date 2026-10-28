import { readFileSync } from "node:fs";
import { join } from "node:path";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

const script = readFileSync(
  join(process.cwd(), "src-tauri", "collector-scripts", "session-probe.js"),
  "utf8",
);
const fixture = JSON.parse(
  readFileSync(
    join(
      process.cwd(),
      "src-tauri",
      "tests",
      "fixtures",
      "home-session-sources.json",
    ),
    "utf8",
  ),
);

function evaluateProbe(pageState, href = "https://www.douyin.com/") {
  const result = runInNewContext(script, {
    ...pageState,
    document: { querySelector: () => null },
    location: { href },
    URL,
  });
  return JSON.parse(JSON.stringify(result));
}

function allowedSources(entries) {
  return {
    SSR_RENDER_DATA: { app: { user: entries[0].value } },
    __INITIAL_STATE__: { user: entries[1].value },
  };
}

describe("Douyin home session probe", () => {
  it("reads authenticated identity only from allowed state paths", () => {
    expect(evaluateProbe({ SSR_RENDER_DATA: fixture.authenticated })).toEqual({
      kind: "authenticated",
      pageUrl: "https://www.douyin.com/",
      identity: {
        uid: "112233445566",
        secUid: "MS4wLjABAAAAfixture-sec-uid",
        nickname: "Fixture User",
        avatarUrl: "https://p3-pc-sign.douyinpic.com/fixture.jpeg",
        sources: ["ssr_render_data"],
      },
    });
  });

  it("returns unauthenticated only for a structurally valid explicit false", () => {
    expect(evaluateProbe({ SSR_RENDER_DATA: fixture.loggedOut })).toEqual({
      kind: "unauthenticated",
      pageUrl: "https://www.douyin.com/",
      source: "ssr_render_data",
    });

    expect(
      evaluateProbe({ SSR_RENDER_DATA: { app: { user: { isLogin: false } } } }),
    ).toEqual({
      kind: "indeterminate",
      pageUrl: "https://www.douyin.com/",
      reason: "no_valid_session_source",
    });
  });

  it("does not join identity sources without a shared stable id", () => {
    expect(
      evaluateProbe(allowedSources(fixture.unlinked)),
    ).toEqual({
      kind: "indeterminate",
      pageUrl: "https://www.douyin.com/",
      reason: "identity_sources_unlinked",
    });
  });

  it("rejects conflicting stable ids and mixed login evidence", () => {
    expect(
      evaluateProbe(allowedSources(fixture.conflicting)),
    ).toEqual({
      kind: "conflicting",
      pageUrl: "https://www.douyin.com/",
    });
    expect(
      evaluateProbe(allowedSources(fixture.mixedLogin)),
    ).toEqual({
      kind: "conflicting",
      pageUrl: "https://www.douyin.com/",
    });
  });

  it.each([
    "http://www.douyin.com/",
    "https://m.douyin.com/",
    "https://www.douyin.com:444/",
    "https://user@www.douyin.com/",
    "https://www.douyin.com/video/1",
    "https://www.douyin.com/#login",
  ])("rejects non-home URL %s", (href) => {
    expect(
      evaluateProbe({ SSR_RENDER_DATA: fixture.authenticated }, href),
    ).toEqual({
      kind: "indeterminate",
      pageUrl: href,
      reason: "invalid_home_url",
    });
  });

  it("allows ordinary home query parameters", () => {
    expect(
      evaluateProbe(
        { SSR_RENDER_DATA: fixture.authenticated },
        "https://www.douyin.com/?enter=home",
      ),
    ).toMatchObject({
      kind: "authenticated",
      pageUrl: "https://www.douyin.com/?enter=home",
    });
  });

  it("reports explicit logout on the exact watching-history page", () => {
    expect(
      evaluateProbe(
        { SSR_RENDER_DATA: fixture.loggedOut },
        "https://www.douyin.com/user/self?showTab=record",
      ),
    ).toEqual({
      kind: "unauthenticated",
      pageUrl: "https://www.douyin.com/user/self?showTab=record",
      source: "ssr_render_data",
    });
  });
});
