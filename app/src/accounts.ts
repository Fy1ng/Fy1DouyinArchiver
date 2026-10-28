import type { AccountStatus, AccountView } from "./types";

export function sortAccounts(accounts: AccountView[], activeAccountId?: number): AccountView[] {
  return [...accounts].sort((left, right) => {
    if (left.id === activeAccountId) return -1;
    if (right.id === activeAccountId) return 1;
    return left.createdAt.localeCompare(right.createdAt) || left.id - right.id;
  });
}

export function accountStatusLabel(status: AccountStatus): string {
  switch (status) {
    case "active":
      return "已登录";
    case "login_expired":
      return "登录已失效";
    case "identity_conflict":
      return "账号身份冲突";
    case "pending_login":
      return "等待登录";
    case "archived":
      return "已归档";
  }
}

export function canSyncAccount(status: AccountStatus): boolean {
  return status === "active";
}

export function canReloginAccount(status: AccountStatus): boolean {
  return status === "login_expired" || status === "identity_conflict";
}

export function resetHistoryViewForAccountSwitch(): {
  currentPage: number;
  selectedIds: Set<number>;
} {
  return { currentPage: 1, selectedIds: new Set<number>() };
}

export function isAccountRequestCurrent(
  expectedAccountId: number,
  expectedJobId: number,
  currentAccountId?: number,
  currentJobId = 0,
): boolean {
  return expectedAccountId === currentAccountId && expectedJobId === currentJobId;
}
