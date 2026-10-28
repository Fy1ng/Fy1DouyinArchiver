import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SearchPage from "./SearchPage.svelte";

const api = vi.hoisted(() => ({
  listLocalFolders: vi.fn(),
  openContentUrl: vi.fn(),
  searchLibraryContent: vi.fn(),
}));

vi.mock("./app-api", () => api);

const accounts = [
  { id: 7, uid: "alpha", accountKey: "sec:alpha", profileKey: "pa", nickname: "账号甲", status: "active", createdAt: "c", updatedAt: "u" },
  { id: 9, uid: "beta", accountKey: "uid:beta", profileKey: "pb", nickname: "账号乙", status: "active", createdAt: "c", updatedAt: "u" },
];

describe("SearchPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.listLocalFolders.mockResolvedValue([
      { id: 31, scope: "aggregate", name: "稍后观看", itemCount: 1, createdAt: "c", updatedAt: "u" },
    ]);
    api.searchLibraryContent.mockResolvedValue({
      records: [{
        id: 1,
        accountId: 7,
        accountKey: "sec:alpha",
        accountDisplayName: "账号甲",
        source: "likes",
        awemeId: "note-one",
        url: "https://www.douyin.com/note/note-one",
        contentType: "note",
        title: "检索结果",
        authorName: "作者",
        coverUrl: null,
        cachedCoverUrl: null,
        likeCount: 10,
        sourcePosition: 0,
        historyTimeMs: null,
        watchedAt: null,
        watchedDate: null,
        watchProgressRate: null,
        remoteCreatedAtMs: null,
        firstArchivedAt: "2026-08-19T00:00:00Z",
        lastSeenAt: "2026-08-19T00:00:00Z",
        remoteRemovedAt: null,
      }],
      total: 1,
    });
    api.openContentUrl.mockResolvedValue(undefined);
  });

  afterEach(() => cleanup());

  it("combines account, source, type, date, remote state, and local folder filters", async () => {
    render(SearchPage, {
      accounts,
      accountScope: { kind: "all" },
      likeCountFormat: "wan",
      coverCacheEnabled: false,
    });
    await waitFor(() => expect(api.searchLibraryContent).toHaveBeenCalled());

    await fireEvent.change(screen.getByLabelText("账号"), { target: { value: "7" } });
    await fireEvent.click(screen.getByLabelText("观看历史"));
    await fireEvent.click(screen.getByLabelText("收藏"));
    await fireEvent.change(screen.getByLabelText("内容类型"), { target: { value: "note" } });
    await fireEvent.input(screen.getByLabelText("开始日期"), { target: { value: "2026-08-01" } });
    await fireEvent.input(screen.getByLabelText("结束日期"), { target: { value: "2026-08-31" } });
    await fireEvent.change(screen.getByLabelText("远端状态"), { target: { value: "removed" } });
    await fireEvent.change(screen.getByLabelText("本地收藏夹"), { target: { value: "31" } });
    await fireEvent.click(screen.getByRole("button", { name: "搜索" }));

    await waitFor(() =>
      expect(api.searchLibraryContent).toHaveBeenLastCalledWith({
        accountScope: { kind: "selected", accountIds: [7] },
        sources: ["likes"],
        query: "",
        contentTypes: ["note"],
        dateFrom: "2026-08-01",
        dateTo: "2026-08-31",
        remoteState: "removed",
        localFolderId: 31,
        sort: "archivedNewest",
        limit: 80,
        offset: 0,
      }),
    );
    expect(await screen.findByText("检索结果")).toBeVisible();
    expect(screen.getAllByText("账号甲")).toHaveLength(2);
    expect(screen.getByText("来源：喜欢")).toBeVisible();
  });

  it("shows a repair state for stale configured account scopes", async () => {
    render(SearchPage, {
      accounts,
      accountScope: { kind: "selected", accountKeys: ["sec:missing"] },
      likeCountFormat: "wan",
      coverCacheEnabled: false,
    });

    expect(await screen.findByText("搜索账号范围已失效，请前往设置重新选择")).toBeVisible();
    expect(api.searchLibraryContent).not.toHaveBeenCalled();
  });
});
