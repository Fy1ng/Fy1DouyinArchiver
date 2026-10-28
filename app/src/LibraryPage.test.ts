import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import LibraryPage from "./LibraryPage.svelte";

const api = vi.hoisted(() => ({
  listLibraryContent: vi.fn(),
  openContentUrl: vi.fn(),
}));

vi.mock("./app-api", () => api);

const accounts = [
  {
    id: 7,
    uid: "alpha",
    accountKey: "sec:alpha",
    profileKey: "profile-alpha",
    nickname: "账号甲",
    status: "active",
    createdAt: "created",
    updatedAt: "updated",
  },
  {
    id: 9,
    uid: "beta",
    accountKey: "uid:beta",
    profileKey: "profile-beta",
    nickname: "账号乙",
    status: "active",
    createdAt: "created",
    updatedAt: "updated",
  },
];

const preferences = {
  all: { query: "", page: 1, viewMode: "horizontal" as const, includeRemoteRemoved: false, sort: "archivedNewest" as const },
  history: { query: "", page: 1, viewMode: "horizontal" as const, includeRemoteRemoved: false, sort: "watchedNewest" as const },
  likes: { query: "", page: 1, viewMode: "grid" as const, includeRemoteRemoved: false, sort: "pageOrder" as const },
  favorites: { query: "", page: 1, viewMode: "horizontal" as const, includeRemoteRemoved: false, sort: "pageOrder" as const },
};

const record = (overrides: Record<string, unknown>) => ({
  id: 1,
  accountId: 7,
  accountKey: "sec:alpha",
  accountDisplayName: "账号甲",
  source: "likes" as const,
  awemeId: "same-video",
  url: "https://www.douyin.com/video/same-video",
  contentType: "video" as const,
  title: "同一视频",
  authorName: "作者",
  coverUrl: null,
  cachedCoverUrl: null,
  likeCount: 12_000,
  sourcePosition: 0,
  historyTimeMs: null,
  watchedAt: null,
  watchedDate: null,
  watchProgressRate: null,
  remoteCreatedAtMs: null,
  firstArchivedAt: "2026-08-19T00:00:00Z",
  lastSeenAt: "2026-08-19T00:00:00Z",
  remoteRemovedAt: null,
  ...overrides,
});

describe("LibraryPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.listLibraryContent.mockResolvedValue({
      records: [
        record({}),
        record({ id: 2, accountId: 9, accountKey: "uid:beta", accountDisplayName: "账号乙", source: "favorites" }),
      ],
      total: 2,
    });
    api.openContentUrl.mockResolvedValue(undefined);
  });

  afterEach(() => cleanup());

  it("queries every account and source and keeps duplicate relations visible", async () => {
    render(LibraryPage, {
      accounts,
      accountScope: { kind: "all" },
      preferences,
      sourceAvailability: { history: true, likes: true, favorites: true },
      likeCountFormat: "wan",
      coverCacheEnabled: false,
      onPreferenceChange: vi.fn(),
    });

    await waitFor(() =>
      expect(api.listLibraryContent).toHaveBeenCalledWith({
        accountScope: { kind: "all" },
        sources: ["history", "likes", "favorites"],
        query: "",
        includeRemoteRemoved: false,
        sort: "archivedNewest",
        limit: 80,
        offset: 0,
      }),
    );
    expect(await screen.findAllByText("同一视频")).toHaveLength(2);
    expect(screen.getByText("账号甲")).toBeVisible();
    expect(screen.getByText("账号乙")).toBeVisible();
    expect(screen.getByText("喜欢")).toBeVisible();
    expect(screen.getByText("收藏")).toBeVisible();
  });

  it("switches source tabs and restores that source view preference", async () => {
    const onPreferenceChange = vi.fn();
    render(LibraryPage, {
      accounts,
      accountScope: { kind: "all" },
      preferences,
      sourceAvailability: { history: true, likes: true, favorites: true },
      likeCountFormat: "wan",
      coverCacheEnabled: false,
      onPreferenceChange,
    });
    await waitFor(() => expect(api.listLibraryContent).toHaveBeenCalledTimes(1));

    await fireEvent.click(screen.getByRole("button", { name: "喜欢" }));
    await waitFor(() =>
      expect(api.listLibraryContent).toHaveBeenLastCalledWith(
        expect.objectContaining({ sources: ["likes"], sort: "pageOrder" }),
      ),
    );
    expect(screen.getByRole("region", { name: "资料库内容" })).toHaveClass("is-grid");
  });

  it("surfaces stale account scopes instead of silently querying all accounts", async () => {
    render(LibraryPage, {
      accounts,
      accountScope: { kind: "selected", accountKeys: ["sec:missing"] },
      preferences,
      sourceAvailability: { history: true, likes: true, favorites: true },
      likeCountFormat: "wan",
      coverCacheEnabled: false,
      onPreferenceChange: vi.fn(),
    });

    expect(await screen.findByText("账号范围已失效，请重新选择")).toBeVisible();
    expect(api.listLibraryContent).not.toHaveBeenCalled();
  });

  it("keeps the sort label outside the framed dropdown and closes the view toggle seam", async () => {
    render(LibraryPage, {
      accounts,
      accountScope: { kind: "all" },
      preferences,
      sourceAvailability: { history: true, likes: true, favorites: true },
      likeCountFormat: "wan",
      coverCacheEnabled: false,
      onPreferenceChange: vi.fn(),
    });

    const sort = document.querySelector(".sort-control");
    expect(sort).toBeInTheDocument();
    expect(sort).not.toHaveClass("framed-control");
    expect(sort?.querySelector(".custom-select")).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "内容视图" })).toHaveClass("view-mode");
  });
});
