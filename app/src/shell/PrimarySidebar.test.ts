import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import PrimarySidebar from "./PrimarySidebar.svelte";

describe("PrimarySidebar", () => {
  afterEach(() => cleanup());

  it("renders the brand, navigation selection, and sync health", () => {
    render(PrimarySidebar, {
      activePage: "history",
      appIconUrl: "/app-icon.png",
      collapsed: false,
      sourceAvailability: { history: true, likes: true, favorites: false },
      syncHealthText: "自动同步已暂停",
      syncing: false,
      onSelect: vi.fn(),
      onToggleCollapsed: vi.fn(),
    });

    expect(screen.getByText("抖音归档器")).toBeVisible();
    expect(screen.getByRole("navigation", { name: "主导航" })).toBeVisible();
    expect(screen.getByRole("button", { name: "资料库" })).toBeVisible();
    expect(screen.getByRole("button", { name: "搜索" })).toBeVisible();
    expect(screen.getByRole("button", { name: "账号管理" })).toBeVisible();
    expect(screen.getByRole("button", { name: "观看历史" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "收藏" })).toBeDisabled();
    expect(screen.getByText("自动同步已暂停")).toBeVisible();
  });

  it("keeps icon buttons named by tooltips when collapsed", async () => {
    const onToggleCollapsed = vi.fn();
    render(PrimarySidebar, {
      activePage: "overview",
      appIconUrl: "/app-icon.png",
      collapsed: true,
      sourceAvailability: { history: true, likes: true, favorites: true },
      syncHealthText: "后台服务正常",
      syncing: false,
      onSelect: vi.fn(),
      onToggleCollapsed,
    });

    expect(screen.getByRole("complementary")).toHaveClass("is-collapsed");
    expect(screen.getByRole("button", { name: "概览" })).toHaveAttribute("title", "概览");
    await fireEvent.click(screen.getByRole("button", { name: "展开侧栏" }));
    expect(onToggleCollapsed).toHaveBeenCalledOnce();
  });
});
