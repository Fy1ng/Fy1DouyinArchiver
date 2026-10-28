import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

describe("通知中心 IPC", () => {
  beforeEach(() => {
    invoke.mockReset().mockResolvedValue({});
    Object.assign(globalThis, { window: { __TAURI_INTERNALS__: {} } });
  });

  it("列表、已读和恢复操作使用固定命令与 camelCase 参数", async () => {
    const {
      listNotifications,
      markNotificationRead,
      markAllNotificationsRead,
      resolveNotification,
    } = await import("../../app/src/app-api.js");

    await listNotifications(false, 50, 10);
    await markNotificationRead(7, true);
    await markAllNotificationsRead();
    await resolveNotification(7);

    expect(invoke).toHaveBeenNthCalledWith(1, "list_notifications", {
      includeResolved: false,
      limit: 50,
      offset: 10,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "mark_notification_read", {
      notificationId: 7,
      read: true,
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "mark_all_notifications_read");
    expect(invoke).toHaveBeenNthCalledWith(4, "resolve_notification", {
      notificationId: 7,
    });
  });
});
