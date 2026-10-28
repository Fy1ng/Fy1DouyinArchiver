import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import AnnualReviewPage from "./AnnualReviewPage.svelte";

const api = vi.hoisted(() => ({ getAnnualReview: vi.fn() }));
vi.mock("./app-api", () => api);

const view = {
  year: 2026,
  availableYears: [2026, 2025],
  chapters: [
    {
      id: "footprint",
      title: "足迹",
      summary: "在 2 个日子留下记录",
      omitted: false,
      omittedReason: null,
      items: [{ label: "观看事件", value: "7", secondary: null, rate: null }],
    },
    {
      id: "authors",
      title: "作者",
      summary: "观看最多的作者",
      omitted: true,
      omittedReason: "作者字段为空",
      items: [],
    },
    {
      id: "closing",
      title: "规则结语",
      summary: "只使用本地归档",
      omitted: false,
      omittedReason: null,
      items: [],
    },
  ],
};

describe("年度回顾页面", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.getAnnualReview.mockResolvedValue(view);
  });
  afterEach(() => cleanup());

  it("按账号范围加载并通过目录和页底按钮切换章节", async () => {
    render(AnnualReviewPage, {
      accounts: [{ id: 11, accountKey: "uid:test" }],
      accountScope: { kind: "all" },
      onClose: vi.fn(),
    });

    await waitFor(() => expect(api.getAnnualReview).toHaveBeenCalledWith({ kind: "all" }, undefined));
    expect(await screen.findByRole("heading", { name: "足迹" })).toBeVisible();
    await fireEvent.click(screen.getByRole("button", { name: "下一章" }));
    expect(screen.getByRole("heading", { name: "作者" })).toBeVisible();
    expect(screen.getByText("作者字段为空")).toBeVisible();
    await fireEvent.click(screen.getByRole("button", { name: "规则结语" }));
    expect(screen.getByRole("heading", { name: "规则结语" })).toBeVisible();
  });

  it("切换年份后重新读取且不拦截滚轮", async () => {
    render(AnnualReviewPage, {
      accounts: [{ id: 11, accountKey: "uid:test" }],
      accountScope: { kind: "all" },
      onClose: vi.fn(),
    });
    await screen.findByRole("heading", { name: "足迹" });
    await fireEvent.change(screen.getByLabelText("回顾年份"), { target: { value: "2025" } });
    await waitFor(() => expect(api.getAnnualReview).toHaveBeenLastCalledWith({ kind: "all" }, 2025));
    const wheel = new WheelEvent("wheel", { cancelable: true });
    screen.getByRole("region", { name: "年度回顾" }).dispatchEvent(wheel);
    expect(wheel.defaultPrevented).toBe(false);
  });
});
