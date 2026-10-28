import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import AccountSwitcher from "./AccountSwitcher.svelte";
import type { AccountView } from "../types";

const accounts: AccountView[] = [
  {
    id: 11,
    accountKey: "uid:current",
    profileKey: "current-profile",
    nickname: "当前账号",
    status: "active",
    createdAt: "2026-08-01T00:00:00Z",
    updatedAt: "2026-08-01T00:00:00Z",
  },
  {
    id: 12,
    accountKey: "uid:other",
    profileKey: "other-profile",
    nickname: "另一个账号",
    status: "active",
    createdAt: "2026-08-02T00:00:00Z",
    updatedAt: "2026-08-02T00:00:00Z",
  },
];

describe("AccountSwitcher", () => {
  afterEach(() => cleanup());

  it("shows the current account and switches from the controlled menu", async () => {
    const onSelect = vi.fn();
    render(AccountSwitcher, {
      accounts,
      activeAccount: accounts[0],
      open: true,
      disabled: false,
      loginBusy: false,
      canSyncCurrent: true,
      collapsed: false,
      onToggle: vi.fn(),
      onSelect,
      onSyncCurrent: vi.fn(),
      onAdd: vi.fn(),
      onManage: vi.fn(),
    });

    expect(screen.getByRole("button", { expanded: true })).toHaveTextContent("当前账号");
    await fireEvent.click(screen.getByRole("button", { name: /另一个账号/ }));
    expect(onSelect).toHaveBeenCalledWith(accounts[1]);
  });

  it("retains an accessible account identity when collapsed", () => {
    render(AccountSwitcher, {
      accounts,
      activeAccount: accounts[0],
      open: false,
      disabled: false,
      loginBusy: false,
      canSyncCurrent: true,
      collapsed: true,
      onToggle: vi.fn(),
      onSelect: vi.fn(),
      onSyncCurrent: vi.fn(),
      onAdd: vi.fn(),
      onManage: vi.fn(),
    });

    expect(screen.getByRole("button", { name: "切换账号：当前账号" })).toHaveAttribute("title", "当前账号");
  });
});
