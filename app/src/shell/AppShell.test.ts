import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import AppShell from "./AppShell.svelte";

describe("AppShell", () => {
  afterEach(() => cleanup());

  it("exposes a stable workspace and collapsed-sidebar state", () => {
    render(AppShell, { collapsed: true });

    expect(screen.getByTestId("app-workspace")).toHaveClass("workspace-shell", "sidebar-collapsed");
    expect(screen.getByTestId("workspace-background")).toBeInTheDocument();
    expect(screen.getByTestId("modal-host")).toBeInTheDocument();
  });
});
