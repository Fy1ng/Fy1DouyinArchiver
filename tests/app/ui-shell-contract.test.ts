import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function cssRule(source: string, selector: string): string {
  const selectorIndex = source.indexOf(selector);
  expect(selectorIndex).toBeGreaterThanOrEqual(0);
  const openingBrace = source.indexOf("{", selectorIndex);
  const closingBrace = source.indexOf("}", openingBrace);
  return source.slice(openingBrace + 1, closingBrace);
}

describe("desktop shell release contract", () => {
  it("uses the packaged application icon as the document icon", () => {
    const source = readFileSync(resolve("app/index.html"), "utf8");

    expect(source).toContain('rel="icon"');
    expect(source).toContain('/src/assets/app-icon.png');
  });

  it("enforces the supported minimum window", () => {
    const config = JSON.parse(readFileSync(resolve("src-tauri/tauri.conf.json"), "utf8"));
    const mainWindow = config.app.windows[0];

    expect(mainWindow.minWidth).toBe(960);
    expect(mainWindow.minHeight).toBe(640);
  });

  it("defines the approved light and dark visual tokens", () => {
    const source = readFileSync(resolve("app/src/styles.css"), "utf8");
    const light = cssRule(source, ":root {");
    const dark = cssRule(source, ':root[data-theme="dark"]');

    expect(light).toContain("--app-bg: #f6f8fa");
    expect(light).toContain("--surface: #ffffff");
    expect(light).toContain("--accent: #0f9f91");
    expect(light.toLowerCase()).not.toContain("#ff4d6d");
    expect(dark).toContain("--app-bg: #161616");
    expect(dark).toContain("--surface: #1d1d1d");
    expect(dark).toContain("--surface-muted: #242424");
    expect(dark).toContain("--accent: #2dd4bf");
    expect(dark).toContain("--brand-secondary: #fe2c55");
    expect(source).not.toMatch(/#1e2938|#98a7bb|#efb5b5|#d96b6b|#861313|#fff1f1/i);
  });

  it("uses the new-install dark theme before persisted settings load", () => {
    const source = readFileSync(resolve("app/src/App.svelte"), "utf8");

    expect(source).toContain('let themeMode: ThemeMode = "dark";');
  });

  it("disables nonessential motion when Windows requests reduced motion", () => {
    const source = readFileSync(resolve("app/src/styles.css"), "utf8");

    expect(source).toContain("@media (prefers-reduced-motion: reduce)");
    expect(source).toMatch(/transition-duration:\s*0\.01ms/);
  });

  it("keeps every primary navigation action visible at the minimum window height", () => {
    const source = readFileSync(resolve("app/src/styles.css"), "utf8");

    expect(source).toMatch(
      /@media \(max-height: 700px\)[\s\S]*?\.side-navigation button\s*\{[\s\S]*?min-height:\s*var\(--control-hit-compact\)/,
    );
  });

  it("fits full-height task and notification workspaces beneath the application header", () => {
    const source = readFileSync(resolve("app/src/styles.css"), "utf8");

    expect(source).toContain("height: calc(100dvh - var(--titlebar-height) - 102px)");
  });

  it("provides a theme-aware keyboard focus ring for every interactive control", () => {
    const source = readFileSync(resolve("app/src/styles.css"), "utf8");

    expect(source).toMatch(/:focus-visible\s*\{[\s\S]*?outline:\s*2px solid var\(--focus-ring\)/);
    expect(source).toMatch(/:focus-visible\s*\{[\s\S]*?outline-offset:\s*2px/);
    expect(source).toContain("--focus-ring-soft:");
  });

  it("gives release-critical search and range inputs explicit accessible names", () => {
    const appSource = readFileSync(resolve("app/src/App.svelte"), "utf8");
    const librarySource = readFileSync(resolve("app/src/LibraryPage.svelte"), "utf8");
    const folderSource = readFileSync(resolve("app/src/FolderPage.svelte"), "utf8");
    const searchSource = readFileSync(resolve("app/src/SearchPage.svelte"), "utf8");

    expect(appSource).toContain('aria-label="观看历史同步范围天数"');
    expect(appSource).toContain('aria-label="来源内容搜索"');
    expect(librarySource).toContain('aria-label="资料库搜索"');
    expect(folderSource).toContain('aria-label="收藏夹内容搜索"');
    expect(searchSource).toContain('aria-label="全局搜索关键词"');
  });
});
