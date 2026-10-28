import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import NotificationsPage from "./NotificationsPage.svelte";

const api = vi.hoisted(() => ({
  listenAppEvent: vi.fn(),
  listNotifications: vi.fn(),
  markAllNotificationsRead: vi.fn(),
  markNotificationRead: vi.fn(),
  resolveNotification: vi.fn(),
}));

vi.mock("./app-api", () => api);

const notification = {
  id: 4,
  kind: "loginExpired",
  severity: "error" as const,
  dedupeKey: "login:11",
  title: "抖音登录已失效",
  body: "请重新登录。",
  accountId: 11,
  relatedType: null,
  relatedId: null,
  createdAt: "2026-08-19T10:00:00Z",
  readAt: null,
  resolvedAt: null,
};

describe("通知中心", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.listenAppEvent.mockResolvedValue(vi.fn());
    api.listNotifications.mockResolvedValue({ records: [notification], total: 1, unread: 1 });
    api.markNotificationRead.mockResolvedValue({ ...notification, readAt: "2026-08-19T11:00:00Z" });
    api.markAllNotificationsRead.mockResolvedValue(1);
    api.resolveNotification.mockResolvedValue({ ...notification, resolvedAt: "2026-08-19T11:00:00Z" });
  });

  afterEach(() => cleanup());

  it("默认读取未恢复通知并支持单条已读与恢复", async () => {
    render(NotificationsPage);
    await waitFor(() => expect(api.listNotifications).toHaveBeenCalledWith(false, 50, 0));
    expect(await screen.findByText("抖音登录已失效")).toBeVisible();
    await fireEvent.click(screen.getByRole("button", { name: "标记已读" }));
    await waitFor(() => expect(api.markNotificationRead).toHaveBeenCalledWith(4, true));
    await fireEvent.click(screen.getByRole("button", { name: "标记已恢复" }));
    await waitFor(() => expect(api.resolveNotification).toHaveBeenCalledWith(4));
  });

  it("支持切换已恢复列表和全部已读", async () => {
    render(NotificationsPage);
    await screen.findByText("抖音登录已失效");
    await fireEvent.click(screen.getByRole("button", { name: "全部已读" }));
    expect(api.markAllNotificationsRead).toHaveBeenCalledOnce();
    await fireEvent.click(screen.getByRole("button", { name: "查看已恢复" }));
    await waitFor(() => expect(api.listNotifications).toHaveBeenLastCalledWith(true, 50, 0));
  });
});
