import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const projectRoot = resolve(process.cwd());
const styles = readFileSync(resolve(projectRoot, "app/src/styles.css"), "utf8");
const appSource = readFileSync(resolve(projectRoot, "app/src/App.svelte"), "utf8");
const productionSvelte = [
  "app/src/App.svelte",
  "app/src/SearchPage.svelte",
  "app/src/LibraryPage.svelte",
  "app/src/OverviewPage.svelte",
  "app/src/AnnualReviewPage.svelte",
  "app/src/FolderPage.svelte",
  "app/src/download/DownloadStartDialog.svelte",
].map((path) => readFileSync(resolve(projectRoot, path), "utf8")).join("\n");
const tauriConfig = JSON.parse(
  readFileSync(resolve(projectRoot, "src-tauri/tauri.conf.json"), "utf8"),
) as { app: { windows: Array<{ decorations?: boolean }> } };

describe("stage 3.7 UI contracts", () => {
  it("uses the Douyin palette and a custom scrollbar skin", () => {
    expect(styles).toContain("--app-bg: #161616");
    expect(styles).toContain("--brand-action: #d21b46");
    expect(styles).toContain("--selection: #fe2c55");
    expect(styles).toContain("overflow-x: hidden");
    expect(styles).toContain("::-webkit-scrollbar");
    expect(styles).toContain("appearance: none");
  });

  it("uses a frameless native window and renders the custom titlebar", () => {
    expect(tauriConfig.app.windows[0]?.decorations).toBe(false);
    expect(appSource).toContain("<TitleBar />");
  });

  it("uses the shared hand-drawn dropdown instead of visible native selects", () => {
    expect(productionSvelte).not.toContain("<select");
    expect(appSource).toContain("<SelectField");
    expect(styles).toContain(".custom-select-menu");
    expect(styles).toContain(".custom-scrollbar-thumb");
  });

  it("gives custom scrollbars accessible hit geometry and one search-page scroll owner", () => {
    expect(styles).toMatch(/\.custom-scrollbar-track\s*\{[^}]*width:\s*24px/s);
    expect(styles).toMatch(/\.custom-scrollbar-track::before\s*\{[^}]*width:\s*12px/s);
    expect(styles).toMatch(/\.custom-scrollbar-thumb\s*\{[^}]*width:\s*24px/s);
    expect(styles).toMatch(/\.custom-scrollbar-thumb::before\s*\{[^}]*width:\s*12px/s);
    expect(styles).toMatch(/\.search-page\s*\{[^}]*height:\s*auto/s);
    expect(styles).toMatch(/\.search-results\s*\{[^}]*overflow:\s*visible/s);
    expect(styles).toMatch(/\.library-content\.search-results\s*\{[^}]*overflow:\s*visible/s);
  });

  it("uses discrete readable type and interaction-size tokens", () => {
    expect(styles).toContain("--text-xs: 12px");
    expect(styles).toContain("--text-sm: 13px");
    expect(styles).toContain("--text-base: 14px");
    expect(styles).toContain("--control-hit-compact: 32px");
    expect(styles).toContain("--control-hit-default: 36px");
    expect(styles).toContain("--choice-control-size: 18px");
    expect(styles).not.toMatch(/font-size:\s*(?:9|10|11)px/);
    expect(styles).toMatch(/small\s*\{[^}]*font-size:\s*var\(--text-xs\)/s);
    expect(styles).toMatch(/input\[type="checkbox"\],[\s\S]*?width:\s*var\(--choice-control-size\)/);
    expect(styles).toMatch(/label:has\(> input\[type="checkbox"\]\),[\s\S]*?min-height:\s*var\(--control-hit-compact\)/);
    expect(styles).toMatch(/\.sidebar-collapse\s*\{[^}]*width:\s*var\(--control-hit-compact\)[^}]*min-height:\s*var\(--control-hit-compact\)/s);
    expect(styles).toMatch(/\.message-dismiss\s*\{[^}]*width:\s*var\(--control-hit-compact\)[^}]*height:\s*var\(--control-hit-compact\)/s);
  });

  it("keeps toolbar labels unframed and input shells evenly padded", () => {
    expect(styles).toContain(".sort-control {\n  border: 0;");
    expect(styles).toContain(".interval-input:focus-within");
    expect(styles).toContain(".days-input:focus-within");
  });

  it("renders compound controls with only their outer shell", () => {
    expect(styles).toContain(".days-input.days-input > input");
    expect(styles).toContain(".interval-input.interval-input > input");
    expect(styles).toContain(".search-box.search-box > input");
    expect(styles).toContain(".settings-search.settings-search > input");
    expect(styles).toContain(".cover-cache-search.cover-cache-search > label > input");
    expect(styles).toContain("border: 0;");
    expect(styles).toContain("background: transparent;");
    expect(styles).toContain("field-sizing: content;");
  });

  it("keeps choice and numeric controls compact with bounded hit areas", () => {
    expect(styles).toContain("label:has(> input[type=\"checkbox\"])");
    expect(styles).toContain("inline-size: fit-content;");
    expect(styles).toContain(".segmented {");
    expect(styles).toContain("width: fit-content;");
    expect(styles).toContain("input[type=\"number\"]");
    expect(styles).toContain("field-sizing: content;");
  });

  it("keeps the search source group horizontal and non-shrinking", () => {
    expect(styles).toContain(".search-filter-panel .source-filter");
    expect(styles).toContain("flex-wrap: nowrap;");
    expect(styles).toContain("flex: 0 0 auto;");
  });

  it("renders directory values as picker-only read-only fields", () => {
    expect(appSource).toContain('aria-label="默认下载目录"');
    expect(appSource).toContain('aria-label="默认下载目录"\n                readonly');
    expect(productionSvelte).toContain('aria-label="下载目标目录"');
    expect(productionSvelte).toContain('aria-label="下载目标目录" bind:value={destinationRoot} readonly');
  });
});
