import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const styles = readFileSync(resolve(process.cwd(), "app/src/styles.css"), "utf8");

function readToken(block: string, name: string): string {
  const match = block.match(new RegExp(`${name}\\s*:\\s*([^;]+);`));
  if (!match) throw new Error(`Missing ${name}`);
  return match[1].trim().toLowerCase();
}

function readBlock(selector: string): string {
  const start = styles.indexOf(`${selector} {`);
  if (start < 0) throw new Error(`Missing block ${selector}`);
  const end = styles.indexOf("}\n", start);
  if (end < 0) throw new Error(`Unclosed block ${selector}`);
  return styles.slice(start, end + 1);
}

function channel(value: string, offset: number): number {
  return Number.parseInt(value.slice(offset, offset + 2), 16) / 255;
}

function relativeLuminance(hex: string): number {
  const normalized = hex.replace("#", "");
  const linear = (component: number) =>
    component <= 0.03928 ? component / 12.92 : ((component + 0.055) / 1.055) ** 2.4;
  return (
    0.2126 * linear(channel(normalized, 0)) +
    0.7152 * linear(channel(normalized, 2)) +
    0.0722 * linear(channel(normalized, 4))
  );
}

function contrastRatio(foreground: string, background: string): number {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

function blendHex(foreground: string, background: string, opacity: number): string {
  const foregroundHex = foreground.replace("#", "");
  const backgroundHex = background.replace("#", "");
  const blended = [0, 2, 4].map((offset) =>
    Math.round(
      channel(foregroundHex, offset) * 255 * opacity +
        channel(backgroundHex, offset) * 255 * (1 - opacity),
    )
      .toString(16)
      .padStart(2, "0"),
  );
  return `#${blended.join("")}`;
}

describe("UI semantic color contracts", () => {
  it("keeps action, brand, selection, and focus meanings separate", () => {
    const light = readBlock(":root");
    const dark = readBlock(':root[data-theme="dark"]');
    const systemDark = readBlock(':root[data-theme="system"]');

    for (const block of [light, dark, systemDark]) {
      expect(readToken(block, "--brand-action")).toBe("#d21b46");
      expect(readToken(block, "--selection")).toBe("#fe2c55");
      expect(readToken(block, "--focus-ring")).not.toBe(readToken(block, "--selection"));
      expect(readToken(block, "--scrollbar-thumb")).not.toBe(readToken(block, "--selection"));
      expect(readToken(block, "--scrollbar-thumb-hover")).not.toBe(readToken(block, "--brand-action"));
    }

    expect(styles).toMatch(/\.primary\s*\{[\s\S]*background:\s*var\(--action-primary-bg\)/);
    expect(styles).toMatch(/\.primary:hover:not\(:disabled\)\s*\{[\s\S]*background:\s*var\(--action-primary-bg-hover\)/);
  });

  it("keeps enabled primary-button text at WCAG AA contrast in light and dark themes", () => {
    const light = readBlock(":root");
    const dark = readBlock(':root[data-theme="dark"]');
    const cases = [
      {
        name: "light",
        block: light,
        background: "#ffffff",
      },
      {
        name: "dark",
        block: dark,
        background: "#0b0d10",
      },
    ];

    for (const { name, block, background } of cases) {
      const foreground = readToken(block, "--action-primary-fg");
      const normal = readToken(block, "--action-primary-bg");
      const hover = readToken(block, "--action-primary-bg-hover");
      expect(contrastRatio(foreground, normal), `${name} normal`).toBeGreaterThanOrEqual(4.5);
      expect(contrastRatio(foreground, hover), `${name} hover`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("keeps disabled primary-button text distinguishable from its composited background", () => {
    const cases = [
      { block: readBlock(":root"), surface: "#ffffff" },
      { block: readBlock(':root[data-theme="dark"]'), surface: "#1d1d1d" },
    ];
    const opacity = 0.72;

    expect(styles).toContain(".primary:disabled {\n  opacity: 0.72;");
    for (const { block, surface } of cases) {
      const foreground = blendHex(readToken(block, "--action-primary-fg"), surface, opacity);
      const background = blendHex(readToken(block, "--action-primary-bg"), surface, opacity);
      expect(contrastRatio(foreground, background)).toBeGreaterThanOrEqual(3);
    }
  });
});
