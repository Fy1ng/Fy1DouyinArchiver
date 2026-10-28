<script lang="ts">
  import {
    Archive,
    Check,
    LogIn,
    RefreshCw,
    Trash2,
    UserPlus,
    Users,
  } from "lucide-svelte";
  import { accountStatusLabel, canReloginAccount } from "./accounts";
  import type {
    AccountView,
    DefaultSyncMode,
    PortableAccountScope,
    SyncSource,
  } from "./types";

  let {
    accounts,
    activeAccount,
    syncMode,
    syncAccountScope,
    availableSources,
    busy,
    loginBusy,
    syncing,
    onActivate,
    onAdd,
    onRelogin,
    onArchive,
    onRestore,
    onDelete,
    onSyncPreferenceChange,
    onSync,
  }: {
    accounts: AccountView[];
    activeAccount: AccountView | null;
    syncMode: DefaultSyncMode;
    syncAccountScope: PortableAccountScope;
    availableSources: SyncSource[];
    busy: boolean;
    loginBusy: boolean;
    syncing: boolean;
    onActivate: (account: AccountView) => void | Promise<void>;
    onAdd: () => void | Promise<void>;
    onRelogin: (account: AccountView) => void | Promise<void>;
    onArchive: (account: AccountView) => void | Promise<void>;
    onRestore: (account: AccountView) => void | Promise<void>;
    onDelete: (account: AccountView) => void | Promise<void>;
    onSyncPreferenceChange: (
      mode: DefaultSyncMode,
      accountScope: PortableAccountScope,
    ) => void;
    onSync: () => void | Promise<void>;
  } = $props();

  let selectedAccountId = $state<number | null>(null);
  const activeAccounts = $derived(accounts.filter((account) => account.status === "active"));
  const selectedAccount = $derived(
    accounts.find((account) => account.id === selectedAccountId) ?? activeAccount ?? accounts[0] ?? null,
  );
  const availableKeys = $derived(
    new Set(activeAccounts.flatMap((account) => account.accountKey ? [account.accountKey] : [])),
  );
  const selectedKeys = $derived(
    syncAccountScope.kind === "selected" ? syncAccountScope.accountKeys : [],
  );
  const scopeInvalid = $derived(
    syncAccountScope.kind === "selected" &&
      (selectedKeys.length === 0 || selectedKeys.some((key) => !availableKeys.has(key))),
  );
  const aggregateAccounts = $derived(
    syncAccountScope.kind === "all"
      ? activeAccounts
      : activeAccounts.filter(
          (account) => account.accountKey && selectedKeys.includes(account.accountKey),
        ),
  );
  const syncAccountCount = $derived(
    syncMode === "singleAccount" ? (activeAccount?.status === "active" ? 1 : 0) : aggregateAccounts.length,
  );
  const sourceNames: Record<SyncSource, string> = {
    history: "观看历史",
    likes: "喜欢",
    favorites: "收藏",
    remoteFolder: "抖音收藏夹",
  };
  const syncSummary = $derived(
    `将同步 ${syncAccountCount} 个账号 · ${availableSources.map((source) => sourceNames[source]).join("、") || "暂无可用来源"}`,
  );

  $effect(() => {
    if (!accounts.some((account) => account.id === selectedAccountId)) {
      selectedAccountId = activeAccount?.id ?? accounts[0]?.id ?? null;
    }
  });

  function selectAccount(account: AccountView): void {
    selectedAccountId = account.id;
    if (account.status !== "archived" && account.id !== activeAccount?.id) void onActivate(account);
  }

  function setMode(mode: DefaultSyncMode): void {
    onSyncPreferenceChange(mode, syncAccountScope);
  }

  function setScopeKind(kind: "all" | "selected"): void {
    if (kind === "all") {
      onSyncPreferenceChange(syncMode, { kind: "all" });
      return;
    }
    const fallback = activeAccount?.accountKey ?? activeAccounts[0]?.accountKey;
    if (!fallback) return;
    onSyncPreferenceChange(syncMode, { kind: "selected", accountKeys: [fallback] });
  }

  function toggleAccount(account: AccountView, checked: boolean): void {
    if (!account.accountKey) return;
    const keys = new Set(scopeInvalid ? [] : selectedKeys);
    if (checked) keys.add(account.accountKey);
    else keys.delete(account.accountKey);
    if (keys.size === 0) return;
    onSyncPreferenceChange(syncMode, {
      kind: "selected",
      accountKeys: [...keys].sort((left, right) => left.localeCompare(right)),
    });
  }
</script>

<section class="account-workspace" aria-label="账号工作台">
  <aside class="account-workspace-list">
    <header>
      <div><span class="eyebrow">账号</span><h2>{accounts.length} 个本地账号</h2></div>
      <button class="icon-button" type="button" title="添加账号" disabled={loginBusy || busy} onclick={onAdd}><UserPlus size={17} /></button>
    </header>
    <div class="account-workspace-items">
      {#each accounts as account}
        <button
          type="button"
          class:active={account.id === selectedAccount?.id}
          onclick={() => selectAccount(account)}
          aria-label={`${account.nickname ?? account.uid ?? `账号 ${account.id}`}，${accountStatusLabel(account.status)}`}
        >
          <span class="account-avatar">{#if account.avatarUrl}<img src={account.avatarUrl} alt="" />{:else}<Users size={17} />{/if}</span>
          <span><strong>{account.nickname ?? account.uid ?? `账号 ${account.id}`}</strong><small>{accountStatusLabel(account.status)}</small></span>
          {#if account.id === activeAccount?.id}<Check size={15} aria-label="当前账号" />{/if}
        </button>
      {/each}
    </div>
    <button class="secondary account-workspace-add" type="button" disabled={loginBusy || busy} onclick={onAdd}><UserPlus size={16} />{loginBusy ? "等待浏览器登录" : "添加账号"}</button>
  </aside>

  <div class="account-workspace-detail">
    {#if selectedAccount}
      <section class="account-detail-header">
        <div class="account-detail-identity">
          <span class="account-avatar large">{#if selectedAccount.avatarUrl}<img src={selectedAccount.avatarUrl} alt="" />{:else}<Users size={24} />{/if}</span>
          <div><span class="eyebrow">{accountStatusLabel(selectedAccount.status)}</span><h2>{selectedAccount.nickname ?? selectedAccount.uid ?? `账号 ${selectedAccount.id}`}</h2><p>{selectedAccount.uid ?? "抖音账号身份已保存在本地"}</p></div>
        </div>
        <div class="account-detail-actions">
          {#if canReloginAccount(selectedAccount.status)}
            <button type="button" disabled={busy || loginBusy} onclick={() => onRelogin(selectedAccount)}><LogIn size={15} />一键登录</button>
          {/if}
          {#if selectedAccount.status === "archived"}
            <button type="button" disabled={busy || loginBusy} onclick={() => onRestore(selectedAccount)}>恢复账号</button>
          {:else}
            <button type="button" disabled={busy || loginBusy || syncing} onclick={() => onArchive(selectedAccount)}><Archive size={15} />归档</button>
          {/if}
          <button class="danger" type="button" disabled={busy || loginBusy || syncing} onclick={() => onDelete(selectedAccount)}><Trash2 size={15} />删除</button>
        </div>
      </section>
    {/if}

    <section class="account-sync-policy">
      <div class="section-heading"><div><span class="eyebrow">同步策略</span><h3>选择本次同步覆盖的账号</h3></div></div>
      <div class="segmented" aria-label="默认同步方式">
        <button type="button" class:active={syncMode === "singleAccount"} aria-pressed={syncMode === "singleAccount"} disabled={busy || syncing} onclick={() => setMode("singleAccount")}>单账号同步</button>
        <button type="button" class:active={syncMode === "aggregate"} aria-pressed={syncMode === "aggregate"} disabled={busy || syncing} onclick={() => setMode("aggregate")}>多账号聚合同步</button>
      </div>

      {#if syncMode === "aggregate"}
        <div class="segmented compact" aria-label="聚合同步范围">
          <button type="button" class:active={syncAccountScope.kind === "all"} aria-pressed={syncAccountScope.kind === "all"} disabled={busy || syncing} onclick={() => setScopeKind("all")}>全部活跃账号</button>
          <button type="button" class:active={syncAccountScope.kind === "selected"} aria-pressed={syncAccountScope.kind === "selected"} disabled={busy || syncing} onclick={() => setScopeKind("selected")}>自定义账号</button>
        </div>
        {#if syncAccountScope.kind === "selected"}
          <div class="account-scope-list">
            {#each activeAccounts as account}
              <label>
                <input
                  type="checkbox"
                  aria-label={`聚合同步${account.nickname ?? account.uid ?? `账号 ${account.id}`}`}
                  checked={Boolean(account.accountKey && selectedKeys.includes(account.accountKey))}
                  disabled={busy || syncing || !account.accountKey || (!scopeInvalid && selectedKeys.length === 1 && account.accountKey === selectedKeys[0])}
                  onchange={(event) => toggleAccount(account, (event.currentTarget as HTMLInputElement).checked)}
                />
                <span>{account.nickname ?? account.uid ?? `账号 ${account.id}`}</span>
              </label>
            {/each}
          </div>
        {/if}
      {/if}

      {#if scopeInvalid}<p class="account-scope-error">同步账号范围已失效，请重新选择</p>{/if}
      <footer class="account-sync-footer">
        <span>{scopeInvalid ? "配置中的账号已不存在或不可同步" : syncSummary}</span>
        <button
          class="primary"
          type="button"
          disabled={busy || syncing || scopeInvalid || syncAccountCount === 0 || availableSources.length === 0}
          onclick={onSync}
        ><RefreshCw size={16} />{scopeInvalid ? "重新选择账号后同步" : `同步 ${syncAccountCount} 个账号`}</button>
      </footer>
    </section>
  </div>
</section>
