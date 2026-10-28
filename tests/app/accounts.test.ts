import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  accountStatusLabel,
  canReloginAccount,
  canSyncAccount,
  isAccountRequestCurrent,
  resetHistoryViewForAccountSwitch,
  sortAccounts,
} from "../../app/src/accounts.js";
import type { AccountView } from "../../app/src/types.js";
import {
  accountLoginButtonLabel,
  accountLoginErrorMessage,
  accountRecoveryButtonLabel,
} from "../../app/src/account-login-ui.js";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const account = (id: number, createdAt: string, status: AccountView["status"]): AccountView => ({
  id,
  profileKey: `profile-${id}`,
  nickname: `账号 ${id}`,
  status,
  createdAt,
  updatedAt: createdAt,
});

describe("账号状态转换", () => {
  it("提供登录中按钮和稳定错误文案", () => {
    expect(accountLoginButtonLabel(false)).toBe("添加账号");
    expect(accountLoginButtonLabel(true)).toBe("等待登录…");
    expect(accountRecoveryButtonLabel(false)).toBe("一键登录");
    expect(accountRecoveryButtonLabel(true)).toBe("等待登录…");
    expect(accountLoginErrorMessage("account_login_browser_closed")).toBe(
      "登录浏览器已关闭，未添加账号",
    );
    expect(accountLoginErrorMessage("account_login_cancelled")).toBe("登录已取消，未添加账号");
  });
  it("当前账号优先，其余账号按创建时间排序", () => {
    const accounts = [
      account(1, "2026-07-10T00:00:00Z", "active"),
      account(2, "2026-07-12T00:00:00Z", "login_expired"),
      account(3, "2026-07-11T00:00:00Z", "active"),
    ];

    expect(sortAccounts(accounts, 3).map((item) => item.id)).toEqual([3, 1, 2]);
  });

  it("切换账号时清空选择并回到第一页", () => {
    expect(resetHistoryViewForAccountSwitch()).toEqual({ currentPage: 1, selectedIds: new Set() });
  });

  it("为登录失效和身份冲突账号提供不同文案与操作权限", () => {
    expect(accountStatusLabel("login_expired")).toBe("登录已失效");
    expect(accountStatusLabel("identity_conflict")).toBe("账号身份冲突");
    expect(accountStatusLabel("pending_login")).toBe("等待登录");
    expect(canSyncAccount("active")).toBe(true);
    expect(canSyncAccount("login_expired")).toBe(false);
    expect(canSyncAccount("identity_conflict")).toBe(false);
    expect(canReloginAccount("active")).toBe(false);
    expect(canReloginAccount("login_expired")).toBe(true);
    expect(canReloginAccount("identity_conflict")).toBe(true);
  });

  it("拒绝账号切换后迟到的历史和同步响应", () => {
    expect(isAccountRequestCurrent(11, 7, 11, 7)).toBe(true);
    expect(isAccountRequestCurrent(11, 7, 12, 7)).toBe(false);
    expect(isAccountRequestCurrent(11, 7, 11, 8)).toBe(false);
  });
});

describe("账号范围 IPC", () => {
  beforeEach(() => {
    invoke.mockReset();
    Object.assign(globalThis, { window: { __TAURI_INTERNALS__: {} } });
  });

  it("为历史、同步、状态、取消和删除传递精确账号任务参数", async () => {
    const api = await import("../../app/src/app-api.js");
    invoke.mockResolvedValue({});

    await api.listHistory({ accountId: 11, query: "标题", limit: 80, offset: 0 });
    await api.startSync(11, { mode: "days", days: 7 });
    await api.getSyncStatus(11, 23);
    await api.cancelSync(11, 23);
    await api.deleteHistoryRecords(11, [3, 4]);
    await api.deleteHistoryByQuery(11, "作者");

    expect(invoke.mock.calls).toEqual([
      ["list_history", { accountId: 11, query: "标题", limit: 80, offset: 0 }],
      ["start_sync", { accountId: 11, options: { mode: "days", days: 7 } }],
      ["get_sync_status", { accountId: 11, jobId: 23 }],
      ["cancel_sync", { accountId: 11, jobId: 23 }],
      ["delete_history_records", { accountId: 11, ids: [3, 4] }],
      ["delete_history_by_query", { accountId: 11, query: "作者" }],
    ]);
  });

  it("序列化全部账号管理命令和收藏夹处置参数", async () => {
    const api = await import("../../app/src/app-api.js");
    invoke.mockResolvedValue({});

    await api.listAccounts(true);
    await api.getActiveAccount();
    await api.beginAccountLogin();
    await api.cancelAccountLogin();
    await api.resumeAccountLogin(11);
    await api.cancelPendingAccount(12);
    await api.restoreAccount(13);
    await api.setActiveAccount(14);
    await api.archiveAccount(15);
    await api.deleteAccount(16, { type: "transferToAccount", targetAccountId: 14 });

    expect(invoke.mock.calls).toEqual([
      ["list_accounts", { includeArchived: true }],
      ["get_active_account"],
      ["begin_account_login"],
      ["cancel_account_login"],
      ["resume_account_login", { accountId: 11 }],
      ["cancel_pending_account", { accountId: 12 }],
      ["restore_account", { accountId: 13 }],
      ["set_active_account", { accountId: 14 }],
      ["archive_account", { accountId: 15 }],
      ["delete_account", { accountId: 16, folderDisposition: { type: "transferToAccount", targetAccountId: 14 } }],
    ]);
  });
});
