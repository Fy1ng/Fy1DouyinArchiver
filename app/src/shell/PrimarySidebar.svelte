<script lang="ts">
  import type { Snippet } from "svelte";
  import {
    Bookmark,
    Bell,
    Clock,
    Database,
    FolderOpen,
    Folders,
    Heart,
    Library,
    PanelLeftClose,
    PanelLeftOpen,
    RefreshCw,
    Search,
    Settings,
    Users,
  } from "lucide-svelte";
  import type { AppPage, SourceAvailability } from "./types";

  let {
    activePage,
    appIconUrl,
    collapsed = false,
    sourceAvailability,
    syncHealthText,
    syncing,
    accountSwitcher,
    onSelect,
    onToggleCollapsed,
  }: {
    activePage: AppPage;
    appIconUrl: string;
    collapsed?: boolean;
    sourceAvailability: SourceAvailability;
    syncHealthText: string;
    syncing: boolean;
    accountSwitcher?: Snippet;
    onSelect: (page: AppPage) => void;
    onToggleCollapsed: () => void;
  } = $props();

  function current(page: AppPage): "page" | undefined {
    return activePage === page ? "page" : undefined;
  }
</script>

<aside class="app-sidebar" class:is-collapsed={collapsed} aria-label="应用侧栏">
  <div class="brand-block">
    <span class="brand-mark"><img src={appIconUrl} alt="" aria-hidden="true" draggable="false" /></span>
    <span class="brand-copy"><strong>抖音归档器</strong><small>非官方本地归档工具</small></span>
    <button
      class="icon-button sidebar-collapse"
      type="button"
      title={collapsed ? "展开侧栏" : "折叠侧栏"}
      aria-label={collapsed ? "展开侧栏" : "折叠侧栏"}
      onclick={onToggleCollapsed}
    >
      {#if collapsed}<PanelLeftOpen size={16} />{:else}<PanelLeftClose size={16} />{/if}
    </button>
  </div>

  {#if accountSwitcher}
    <div class="sidebar-account">{@render accountSwitcher()}</div>
  {/if}

  <nav class="side-navigation" aria-label="主导航">
    <button class:active={activePage === "overview"} type="button" title="概览" aria-label="概览" aria-current={current("overview")} onclick={() => onSelect("overview")}><Database size={17} /><span>概览</span></button>
    <div class="nav-group-label">内容</div>
    <button class:active={activePage === "library"} type="button" title="资料库" aria-label="资料库" aria-current={current("library")} onclick={() => onSelect("library")}><Library size={17} /><span>资料库</span></button>
    <button class:active={activePage === "search"} type="button" title="搜索" aria-label="搜索" aria-current={current("search")} onclick={() => onSelect("search")}><Search size={17} /><span>搜索</span></button>
    <button class:active={activePage === "history"} type="button" title="观看历史" aria-label="观看历史" aria-current={current("history")} disabled={!sourceAvailability.history} onclick={() => onSelect("history")}><Clock size={17} /><span>观看历史</span></button>
    <button class:active={activePage === "likes"} type="button" title="喜欢" aria-label="喜欢" aria-current={current("likes")} disabled={!sourceAvailability.likes} onclick={() => onSelect("likes")}><Heart size={17} /><span>喜欢</span></button>
    <button class:active={activePage === "favorites"} type="button" title="收藏" aria-label="收藏" aria-current={current("favorites")} disabled={!sourceAvailability.favorites} onclick={() => onSelect("favorites")}><Bookmark size={17} /><span>收藏</span></button>
    <button class:active={activePage === "remoteFolders"} type="button" title="抖音收藏夹" aria-label="抖音收藏夹" aria-current={current("remoteFolders")} onclick={() => onSelect("remoteFolders")}><Folders size={17} /><span>抖音收藏夹</span></button>
    <button class:active={activePage === "localFolders"} type="button" title="本地收藏夹" aria-label="本地收藏夹" aria-current={current("localFolders")} onclick={() => onSelect("localFolders")}><FolderOpen size={17} /><span>本地收藏夹</span></button>
    <div class="nav-group-label">管理</div>
    <button class:active={activePage === "accounts"} type="button" title="账号管理" aria-label="账号管理" aria-current={current("accounts")} onclick={() => onSelect("accounts")}><Users size={17} /><span>账号管理</span></button>
    <button class:active={activePage === "syncRecords" || activePage === "downloads"} type="button" title="任务中心" aria-label="任务中心" aria-current={activePage === "syncRecords" || activePage === "downloads" ? "page" : undefined} onclick={() => onSelect("syncRecords")}><RefreshCw size={17} /><span>任务中心</span></button>
    <button class:active={activePage === "notifications"} type="button" title="通知中心" aria-label="通知中心" aria-current={current("notifications")} onclick={() => onSelect("notifications")}><Bell size={17} /><span>通知中心</span></button>
    <button class:active={activePage === "settings"} type="button" title="设置" aria-label="设置" aria-current={current("settings")} onclick={() => onSelect("settings")}><Settings size={17} /><span>设置</span></button>
  </nav>

  <div class="sidebar-status" aria-label={`同步健康：${syncHealthText}`}>
    <span class:running={syncing}></span>
    <small>{syncHealthText}</small>
  </div>
</aside>
