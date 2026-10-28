import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import OverviewPage from "./OverviewPage.svelte";

const api = vi.hoisted(() => ({
  getOverviewScope: vi.fn(),
}));

vi.mock("./app-api", () => api);

const overview = {
  uniqueContentCount: 9,
  sourceCounts: { history: 6, likes: 4, favorites: 3 },
  undatedArchiveCount: 2,
  trend: [
    { date: "2026-08-12", history: 2, likes: 1, favorites: 0 },
    { date: "2026-08-13", history: 1, likes: 0, favorites: 1 },
  ],
  topAuthors: [{ authorName: "测试作者", contentCount: 4 }],
  contentTypes: [
    { contentType: "video" as const, contentCount: 7 },
    { contentType: "note" as const, contentCount: 2 },
  ],
  recentSync: [
    {
      accountId: 11,
      source: "history" as const,
      status: "success" as const,
      finishedAt: "2026-08-13T08:00:00.000Z",
      insertedRecords: 3,
      errorCode: null,
    },
  ],
};

describe("账号概览页面", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.getOverviewScope.mockResolvedValue(overview);
  });

  afterEach(() => cleanup());

  it("按默认范围汇总跨来源指标与可靠时间提示", async () => {
    render(OverviewPage, {
      accounts: [{ id: 11, accountKey: "uid:test" }],
      accountScope: { kind: "all" },
    });

    await waitFor(() =>
      expect(api.getOverviewScope).toHaveBeenCalledWith({
        accountScope: { kind: "all" },
        source: "all",
        range: "last30Days",
        includeRemoteRemoved: false,
      }),
    );
    expect(await screen.findByText("9")).toBeVisible();
    expect(screen.getByText(/有 2 条喜欢或收藏缺少可靠远端时间/)).toBeVisible();
    expect(screen.getByRole("region", { name: "概览" })).toHaveTextContent("测试作者");
    expect(screen.getByLabelText("每日新增趋势")).toBeVisible();
    expect(screen.getByText("成功")).toBeVisible();
  });

  it("切换来源、时间与远端移除筛选后重新查询", async () => {
    render(OverviewPage, {
      accounts: [{ id: 11, accountKey: "uid:test" }],
      accountScope: { kind: "selected", accountKeys: ["uid:test"] },
    });
    await waitFor(() => expect(api.getOverviewScope).toHaveBeenCalledTimes(1));

    await fireEvent.change(screen.getByLabelText("来源"), { target: { value: "likes" } });
    await fireEvent.change(screen.getByLabelText("时间"), { target: { value: "all" } });
    await fireEvent.click(screen.getByLabelText("包含已从远端移除"));

    await waitFor(() =>
      expect(api.getOverviewScope).toHaveBeenLastCalledWith({
        accountScope: { kind: "selected", accountIds: [11] },
        source: "likes",
        range: "all",
        includeRemoteRemoved: true,
      }),
    );
    expect(screen.queryByText(/缺少可靠远端时间/)).not.toBeInTheDocument();
  });

  it("账号范围失效时不回退为全部账号", async () => {
    render(OverviewPage, {
      accounts: [{ id: 11, accountKey: "uid:test" }],
      accountScope: { kind: "selected", accountKeys: ["uid:missing"] },
    });

    expect(await screen.findByText("概览账号范围已失效，请前往设置重新选择")).toBeVisible();
    expect(api.getOverviewScope).not.toHaveBeenCalled();
  });
});
