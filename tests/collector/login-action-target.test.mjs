import { readFileSync } from "node:fs";
import { join } from "node:path";
import { JSDOM } from "jsdom";
import { describe, expect, it } from "vitest";

const script = readFileSync(
  join(process.cwd(), "src-tauri", "collector-scripts", "login-action-target.js"),
  "utf8",
);

function evaluateTarget(html, action, options = {}) {
  const dom = new JSDOM(html, {
    url: options.url ?? "https://www.douyin.com/",
    runScripts: "outside-only",
  });
  const { document, HTMLElement } = dom.window;
  Object.defineProperty(HTMLElement.prototype, "getBoundingClientRect", {
    configurable: true,
    value() {
      if (this.dataset.hidden === "true") {
        return { x: 0, y: 0, left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 };
      }
      return { x: 20, y: 30, left: 20, top: 30, right: 140, bottom: 70, width: 120, height: 40 };
    },
  });
  document.elementFromPoint = (x, y) => {
    if (options.obscured) {
      return document.querySelector("#overlay");
    }
    if (options.centerObscured && x >= 60 && x <= 100 && y >= 40 && y <= 60) {
      return document.querySelector("#overlay");
    }
    return document.querySelector(options.hitSelector ?? "button");
  };
  const findTarget = dom.window.eval(`(${script})`);
  return JSON.parse(JSON.stringify(findTarget({ action })));
}

describe("guarded Douyin home login actions", () => {
  it("finds the unique exact one-click login button in the top login component", () => {
    expect(
      evaluateTarget(
        '<div data-e2e="top-login-container"><button>一键登录</button></div>',
        "one_click_login",
      ),
    ).toEqual({
      kind: "found",
      action: "one_click_login",
      x: 80,
      y: 50,
      token: "one_click_login:button:80:50",
    });
  });

  it("finds the exact save button in the observed Douyin trust-login dialog", () => {
    expect(
      evaluateTarget(
        '<div class="trust-login-dialog-content"><div class="trust-login-dialog-title">是否保存登录信息？（5）</div><div class="trust-login-dialog-info">下次登录更便捷，可在个人中心关闭</div><div class="trust-login-dialog-button"><button class="semi-button trust-login-dialog-button-cancel" type="button" aria-disabled="false"><span>取消</span></button><button id="save" class="semi-button trust-login-dialog-button-confirm" type="button" aria-disabled="false"><span>保存</span></button></div></div>',
        "save_login",
        { hitSelector: "#save" },
      ),
    ).toMatchObject({ kind: "found", action: "save_login", x: 80, y: 50 });
  });

  it("rejects the save button when its center point is obscured", () => {
    expect(
      evaluateTarget(
        '<div class="trust-login-dialog-content"><div class="trust-login-dialog-title">是否保存登录信息？（5）</div><button id="save" class="trust-login-dialog-button-confirm">保存</button></div><div id="overlay"></div>',
        "save_login",
        { centerObscured: true, hitSelector: "#save" },
      ),
    ).toMatchObject({ kind: "not_found", action: "save_login", reason: "obscured", count: 1 });
  });

  it.each([
    ['<div data-e2e="top-login-container"><button>一键登录抖音</button></div>', "one_click_login"],
    ['<main><button>一键登录</button></main>', "one_click_login"],
    ['<div role="dialog"><p>是否保存登录信息？（5）</p><button>保存</button></div>', "save_login"],
    ['<div class="trust-login-dialog-content"><div class="trust-login-dialog-title">其他提示</div><button class="trust-login-dialog-button-confirm">保存</button></div>', "save_login"],
    ['<div class="trust-login-dialog-content"><div class="trust-login-dialog-title">是否保存登录信息？（5）</div><button class="trust-login-dialog-button-cancel">保存</button></div>', "save_login"],
  ])("rejects similar text or the wrong container", (html, action) => {
    expect(evaluateTarget(html, action)).toEqual({
      kind: "not_found",
      action,
      reason: "no_candidate",
      count: 0,
    });
  });

  it("rejects duplicate, hidden, disabled, and obscured candidates", () => {
    expect(
      evaluateTarget(
        '<div data-e2e="top-login-container"><button>一键登录</button><button>一键登录</button></div>',
        "one_click_login",
      ),
    ).toMatchObject({ kind: "not_found", reason: "ambiguous_candidate", count: 2 });
    expect(
      evaluateTarget(
        '<div data-e2e="top-login-container"><button data-hidden="true">一键登录</button></div>',
        "one_click_login",
      ),
    ).toMatchObject({ kind: "not_found", reason: "no_candidate", count: 0 });
    expect(
      evaluateTarget(
        '<div data-e2e="top-login-container"><button disabled>一键登录</button></div>',
        "one_click_login",
      ),
    ).toMatchObject({ kind: "not_found", reason: "no_candidate", count: 0 });
    expect(
      evaluateTarget(
        '<div data-e2e="top-login-container"><button>一键登录</button></div><div id="overlay"></div>',
        "one_click_login",
        { obscured: true },
      ),
    ).toMatchObject({ kind: "not_found", reason: "obscured", count: 1 });
    expect(
      evaluateTarget(
        '<div class="trust-login-dialog-content"><div class="trust-login-dialog-title">是否保存登录信息？（4）</div><button class="trust-login-dialog-button-confirm">保存</button><button class="trust-login-dialog-button-confirm">保存</button></div>',
        "save_login",
      ),
    ).toMatchObject({ kind: "not_found", reason: "ambiguous_candidate", count: 2 });
  });

  it("rejects actions outside the strict Douyin home URL", () => {
    expect(
      evaluateTarget(
        '<div data-e2e="top-login-container"><button>一键登录</button></div>',
        "one_click_login",
        { url: "https://www.douyin.com/user/self" },
      ),
    ).toMatchObject({ kind: "not_found", reason: "invalid_home_url" });
  });
});
