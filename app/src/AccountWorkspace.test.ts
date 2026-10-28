import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import AccountWorkspace from "./AccountWorkspace.svelte";

const accounts = [
  {
    id: 1,
    accountKey: "sec:alpha",
    profileKey: "a".repeat(32),
    nickname: "账号甲",
    status: "active" as const,
    createdAt: "2026-08-01T00:00:00Z",
    updatedAt: "2026-08-01T00:00:00Z",
  },
  {
    id: 2,
    accountKey: "sec:beta",
    profileKey: "b".repeat(32),
    nickname: "账号乙",
    status: "active" as const,
    createdAt: "2026-08-02T00:00:00Z",
    updatedAt: "2026-08-02T00:00:00Z",
  },
];

describe("AccountWorkspace", () => {
  it("shows a dual-pane account workspace and edits aggregate scope without widening it", async () => {
    const onActivate = vi.fn();
    const onSyncPreferenceChange = vi.fn();
    const onSync = vi.fn();
    render(AccountWorkspace, {
      accounts,
      activeAccount: accounts[0],
      syncMode: "aggregate",
      syncAccountScope: { kind: "selected", accountKeys: ["sec:beta"] },
      availableSources: ["history", "likes"],
      busy: false,
      loginBusy: false,
      syncing: false,
      onActivate,
      onAdd: vi.fn(),
      onRelogin: vi.fn(),
      onArchive: vi.fn(),
      onRestore: vi.fn(),
      onDelete: vi.fn(),
      onSyncPreferenceChange,
      onSync,
    });

    expect(screen.getByRole("region", { name: "账号工作台" })).toBeVisible();
    expect(screen.getByText("将同步 1 个账号 · 观看历史、喜欢")).toBeVisible();
    await fireEvent.click(screen.getByRole("button", { name: /账号乙/ }));
    expect(onActivate).toHaveBeenCalledWith(accounts[1]);

    await fireEvent.click(screen.getByRole("checkbox", { name: "聚合同步账号甲" }));
    expect(onSyncPreferenceChange).toHaveBeenCalledWith("aggregate", {
      kind: "selected",
      accountKeys: ["sec:alpha", "sec:beta"],
    });
    await fireEvent.click(screen.getByRole("button", { name: "同步 1 个账号" }));
    expect(onSync).toHaveBeenCalledTimes(1);
  });

  it("marks stale configured scopes for reselection instead of falling back to all", () => {
    render(AccountWorkspace, {
      accounts,
      activeAccount: accounts[0],
      syncMode: "aggregate",
      syncAccountScope: { kind: "selected", accountKeys: ["sec:missing"] },
      availableSources: ["history"],
      busy: false,
      loginBusy: false,
      syncing: false,
      onActivate: vi.fn(),
      onAdd: vi.fn(),
      onRelogin: vi.fn(),
      onArchive: vi.fn(),
      onRestore: vi.fn(),
      onDelete: vi.fn(),
      onSyncPreferenceChange: vi.fn(),
      onSync: vi.fn(),
    });

    expect(screen.getByText("同步账号范围已失效，请重新选择")).toBeVisible();
    expect(screen.getByRole("button", { name: "重新选择账号后同步" })).toBeDisabled();
  });
});
