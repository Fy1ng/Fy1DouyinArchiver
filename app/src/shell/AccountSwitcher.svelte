<script lang="ts">
  import { ChevronDown, RefreshCw, Settings, UserPlus, Users } from "lucide-svelte";
  import { accountStatusLabel } from "../accounts";
  import type { AccountView } from "../types";

  let {
    accounts,
    activeAccount,
    open,
    disabled,
    loginBusy,
    canSyncCurrent,
    collapsed = false,
    onToggle,
    onSelect,
    onSyncCurrent,
    onAdd,
    onManage,
  }: {
    accounts: AccountView[];
    activeAccount: AccountView | null;
    open: boolean;
    disabled: boolean;
    loginBusy: boolean;
    canSyncCurrent: boolean;
    collapsed?: boolean;
    onToggle: () => void;
    onSelect: (account: AccountView) => void | Promise<void>;
    onSyncCurrent: () => void;
    onAdd: () => void | Promise<void>;
    onManage: () => void | Promise<void>;
  } = $props();

  const accountName = (account: AccountView | null): string =>
    account?.nickname ?? account?.uid ?? (account ? `账号 ${account.id}` : "未添加账号");
</script>

<div class="account-control sidebar-account-control" class:is-collapsed={collapsed}>
  <button
    class="account-trigger"
    type="button"
    {disabled}
    title={accountName(activeAccount)}
    aria-label={collapsed ? `切换账号：${accountName(activeAccount)}` : undefined}
    aria-expanded={open}
    onclick={onToggle}
  >
    <span class="account-avatar">
      {#if activeAccount?.avatarUrl}
        <img src={activeAccount.avatarUrl} alt="" />
      {:else}
        <Users size={16} />
      {/if}
    </span>
    <span class="account-trigger-copy">
      <strong>{accountName(activeAccount)}</strong>
      <small>{activeAccount ? accountStatusLabel(activeAccount.status) : "添加抖音账号"}</small>
    </span>
    <ChevronDown size={15} />
  </button>

  {#if open}
    <div class="account-menu">
      {#each accounts.filter((account) => account.status !== "archived") as account}
        <button
          class:active={account.id === activeAccount?.id}
          type="button"
          disabled={disabled || loginBusy}
          onclick={() => onSelect(account)}
        >
          <span>{accountName(account)}</span>
          <small>{accountStatusLabel(account.status)}</small>
        </button>
      {/each}
      <div class="account-menu-actions">
        <button type="button" disabled={!canSyncCurrent || disabled} onclick={onSyncCurrent}><RefreshCw size={15} />同步当前账号全部来源</button>
        <button type="button" disabled={loginBusy || disabled} onclick={onAdd}><UserPlus size={15} />{loginBusy ? "等待浏览器登录" : "添加账号"}</button>
        <button type="button" disabled={disabled || loginBusy} onclick={onManage}><Settings size={15} />管理账号</button>
      </div>
    </div>
  {/if}
</div>
