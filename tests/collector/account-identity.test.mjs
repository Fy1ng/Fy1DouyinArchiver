import { readFileSync } from "node:fs";
import { join } from "node:path";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

const script = readFileSync(
  join(process.cwd(), "src-tauri", "collector-scripts", "account-identity.js"),
  "utf8",
);

function evaluateIdentity(pageState) {
  const result = runInNewContext(script, {
    ...pageState,
    document: { querySelector: () => null },
    location: { href: "https://www.douyin.com/user/self?showTab=record" },
    URL,
  });
  return result == null ? result : JSON.parse(JSON.stringify(result));
}

describe("account identity collector script", () => {
  it("reads the current SSR_RENDER_DATA app.user.info shape", () => {
    const result = evaluateIdentity({
      SSR_RENDER_DATA: {
        app: {
          user: {
            isLogin: true,
            info: {
              uid: "112233445566",
              secUid: "MS4wLjABAAAAfixture-sec-uid",
              nickname: "Fixture User",
              avatarUrl: "https://p3-pc-sign.douyinpic.com/fixture.jpeg",
            },
          },
        },
      },
    });

    expect(result).toEqual({
      pageUrl: "https://www.douyin.com/user/self?showTab=record",
      state: {
        candidates: [
          {
            uid: "112233445566",
            secUid: "MS4wLjABAAAAfixture-sec-uid",
            nickname: "Fixture User",
            avatarUrl: "https://p3-pc-sign.douyinpic.com/fixture.jpeg",
          },
        ],
      },
    });
  });
});
