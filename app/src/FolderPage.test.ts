import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import FolderPage from "./FolderPage.svelte";

const api = vi.hoisted(() => ({
  createLocalFolder: vi.fn(),
  deleteLocalFolder: vi.fn(),
  listLocalFolderContent: vi.fn(),
  listLocalFolders: vi.fn(),
  listRemoteFolderContent: vi.fn(),
  listRemoteFolders: vi.fn(),
  openContentUrl: vi.fn(),
  removeLocalFolderItem: vi.fn(),
  renameLocalFolder: vi.fn(),
  setLocalFolderScope: vi.fn(),
}));

vi.mock("./app-api", () => api);

const accounts = [
  {
    id: 11,
    profileKey: "0123456789abcdef0123456789abcdef",
    nickname: "当前账号",
    status: "active" as const,
    createdAt: "2026-08-13T00:00:00Z",
    updatedAt: "2026-08-13T00:00:00Z",
  },
  {
    id: 12,
    profileKey: "fedcba9876543210fedcba9876543210",
    nickname: "第二账号",
    status: "active" as const,
    createdAt: "2026-08-13T00:00:00Z",
    updatedAt: "2026-08-13T00:00:00Z",
  },
];

const localFolder = {
  id: 7,
  scope: "account" as const,
  ownerAccountId: 11,
  name: "本地精选",
  itemCount: 1,
  createdAt: "2026-08-13T00:00:00Z",
  updatedAt: "2026-08-13T00:00:00Z",
};

const record = {
  videoId: 5,
  awemeId: "987",
  url: "https://www.douyin.com/video/987",
  contentType: "video" as const,
  title: "收藏内容",
  authorName: "作者",
  coverUrl: "https://example.com/cover.jpg",
  cachedCoverUrl: "http://cover-cache.localhost/5",
  likeCount: 12000,
  addedAt: "2026-08-13T00:00:00Z",
  firstSeenAt: "2026-08-13T00:00:00Z",
  lastSeenAt: "2026-08-13T00:00:00Z",
  remoteRemovedAt: "2026-08-13T01:00:00Z",
  sourceAccounts: [
    { id: 11, nickname: "当前账号" },
    { id: 12, nickname: "第二账号" },
  ],
};

describe("远端与本地收藏夹页面", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.listLocalFolders.mockResolvedValue([localFolder]);
    api.listLocalFolderContent.mockResolvedValue({ records: [record], total: 1 });
    api.listRemoteFolders.mockResolvedValue([]);
    api.listRemoteFolderContent.mockResolvedValue({ records: [], total: 0 });
    api.removeLocalFolderItem.mockResolvedValue(true);
    api.renameLocalFolder.mockImplementation(async (_accountId, _folderId, name) => ({
      ...localFolder,
      name,
    }));
    api.setLocalFolderScope.mockImplementation(async (_accountId, _folderId, scope, ownerAccountId) => ({
      ...localFolder,
      scope,
      ownerAccountId,
    }));
    api.openContentUrl.mockResolvedValue(undefined);
  });

  afterEach(() => cleanup());

  it("本地收藏夹支持内嵌重命名、归属转换和移除条目", async () => {
    render(FolderPage, { kind: "local", accountId: 11, accounts, likeCountFormat: "wan", coverCacheEnabled: true });

    expect(await screen.findByText("来源 当前账号、第二账号")).toBeVisible();
    await fireEvent.click(screen.getByTitle("重命名"));
    await fireEvent.input(screen.getByLabelText("新收藏夹名称"), { target: { value: "重命名后" } });
    await fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(api.renameLocalFolder).toHaveBeenCalledWith(11, 7, "重命名后"));
    await waitFor(() => expect(api.listLocalFolders).toHaveBeenCalledTimes(2));

    const scope = screen.getByLabelText("当前收藏夹归属");
    await waitFor(() => expect(scope).toBeEnabled());
    await fireEvent.change(scope, { target: { value: "aggregate" } });
    await waitFor(() => expect(api.setLocalFolderScope).toHaveBeenCalledWith(11, 7, "aggregate", null));

    const remove = screen.getByTitle("从本地收藏夹移除");
    await waitFor(() => expect(remove).toBeEnabled());
    await fireEvent.click(remove);
    await waitFor(() => expect(api.removeLocalFolderItem).toHaveBeenCalledWith(11, 7, "987"));
  });

  it("远端收藏夹默认隐藏已移除项并可切换显示", async () => {
    api.listRemoteFolders.mockResolvedValue([{
      id: 3,
      remoteFolderId: "remote-1",
      name: "远端分类",
      firstSeenAt: "2026-08-13T00:00:00Z",
      lastSeenAt: "2026-08-13T00:00:00Z",
      remoteRemovedAt: null,
      itemCount: 1,
      activeItemCount: 1,
    }]);
    api.listRemoteFolderContent.mockResolvedValue({ records: [record], total: 1 });

    render(FolderPage, { kind: "remote", accountId: 11, accounts, likeCountFormat: "wan", coverCacheEnabled: true });
    await waitFor(() => expect(api.listRemoteFolders).toHaveBeenCalledWith(11, false));
    await waitFor(() =>
      expect(api.listRemoteFolderContent).toHaveBeenCalledWith(expect.objectContaining({
        accountId: 11,
        remoteFolderId: "remote-1",
        includeRemoteRemoved: false,
      })),
    );

    await fireEvent.click(screen.getByLabelText("包含已移除分类"));
    await waitFor(() => expect(api.listRemoteFolders).toHaveBeenLastCalledWith(11, true));
  });

  it("列表标题与网格封面均通过系统浏览器打开原内容", async () => {
    render(FolderPage, { kind: "local", accountId: 11, accounts, likeCountFormat: "wan", coverCacheEnabled: true });

    await fireEvent.click(await screen.findByRole("button", { name: "收藏内容" }));
    expect(api.openContentUrl).toHaveBeenCalledWith("https://www.douyin.com/video/987");
    await fireEvent.click(screen.getByRole("button", { name: "收藏夹网格视图" }));
    await fireEvent.click(screen.getByTitle("打开收藏内容"));
    expect(api.openContentUrl).toHaveBeenCalledTimes(2);
  });
});
