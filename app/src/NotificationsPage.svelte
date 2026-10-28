<script lang="ts">
  import { AlertCircle, Bell, Check, CheckCheck, ChevronLeft, ChevronRight, RefreshCw, RotateCcw } from "lucide-svelte";
  import { onMount } from "svelte";
  import { listenAppEvent, listNotifications, markAllNotificationsRead, markNotificationRead, resolveNotification } from "./app-api";
  import type { NotificationSeverity, NotificationView } from "./types";

  const PAGE_SIZE = 50;
  let records: NotificationView[] = [];
  let total = 0;
  let unread = 0;
  let offset = 0;
  let includeResolved = false;
  let loading = true;
  let busyId: number | null = null;
  let errorMessage = "";
  let refreshGeneration = 0;
  let refreshPending = false;
  let refreshRunning = false;

  async function refresh(): Promise<void> {
    const generation = ++refreshGeneration;
    loading = true;
    errorMessage = "";
    try {
      const result = await listNotifications(includeResolved, PAGE_SIZE, offset);
      if (generation !== refreshGeneration) return;
      records = result.records;
      total = result.total;
      unread = result.unread;
      if (offset >= total && total > 0) {
        offset = Math.max(0, Math.floor((total - 1) / PAGE_SIZE) * PAGE_SIZE);
        await refresh();
      }
    } catch (error) {
      if (generation === refreshGeneration) errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      if (generation === refreshGeneration) loading = false;
    }
  }

  async function toggleResolved(): Promise<void> {
    includeResolved = !includeResolved;
    offset = 0;
    await refresh();
  }

  async function toggleRead(notification: NotificationView): Promise<void> {
    busyId = notification.id;
    try {
      await markNotificationRead(notification.id, !notification.readAt);
      await refresh();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busyId = null;
    }
  }

  async function markAllRead(): Promise<void> {
    busyId = -1;
    try {
      await markAllNotificationsRead();
      await refresh();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busyId = null;
    }
  }

  async function resolve(notification: NotificationView): Promise<void> {
    busyId = notification.id;
    try {
      await resolveNotification(notification.id);
      await refresh();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busyId = null;
    }
  }

  async function drainRefreshes(): Promise<void> {
    if (refreshRunning) return;
    refreshRunning = true;
    try {
      while (refreshPending) {
        refreshPending = false;
        await refresh();
      }
    } finally {
      refreshRunning = false;
      if (refreshPending) void drainRefreshes();
    }
  }

  function scheduleRefresh(): void {
    refreshPending = true;
    void drainRefreshes();
  }

  onMount(() => {
    void refresh();
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenAppEvent("notification:changed", scheduleRefresh).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  function severityLabel(severity: NotificationSeverity): string {
    return severity === "error" ? "错误" : severity === "warning" ? "提醒" : "信息";
  }

  function formatDate(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return new Intl.DateTimeFormat("zh-CN", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  }
</script>

<section class="notifications-page" aria-label="通知中心">
  <header class="notifications-toolbar">
    <div>
      <div class="notifications-title"><Bell size={18} /><h2>通知中心</h2>{#if unread > 0}<span class="notification-count">{unread}</span>{/if}</div>
      <p>登录、自动同步和备份异常会保留在这里，直到问题恢复。</p>
    </div>
    <div class="notifications-actions">
      <button class="secondary" type="button" on:click={toggleResolved}>{includeResolved ? "隐藏已恢复" : "查看已恢复"}</button>
      {#if unread > 0}<button class="secondary" type="button" disabled={busyId === -1} on:click={markAllRead}><CheckCheck size={15} />全部已读</button>{/if}
      <button class="icon-button" type="button" title="刷新通知" aria-label="刷新通知" disabled={loading} on:click={refresh}><RefreshCw size={16} class={loading ? "spinning" : ""} /></button>
    </div>
  </header>

  {#if errorMessage}<div class="overview-error" role="alert"><AlertCircle size={16} /><span>{errorMessage}</span></div>{/if}
  {#if loading && records.length === 0}
    <div class="overview-empty">正在读取通知...</div>
  {:else if records.length === 0}
    <div class="notifications-empty"><Check size={22} /><strong>{includeResolved ? "暂无通知记录" : "没有待处理通知"}</strong><span>{includeResolved ? "应用运行状态良好。" : "新的登录、同步或备份问题会显示在这里。"}</span></div>
  {:else}
    <div class="notification-list">
      {#each records as notification}
        <article class:unread={!notification.readAt} class:resolved={Boolean(notification.resolvedAt)} class={`severity-${notification.severity}`}>
          <div class="notification-mark"><span title={severityLabel(notification.severity)}></span></div>
          <div class="notification-copy">
            <div class="notification-heading"><strong>{notification.title}</strong><small>{formatDate(notification.createdAt)}</small></div>
            <p>{notification.body}</p>
            <div class="notification-meta"><span>{severityLabel(notification.severity)}</span>{#if notification.accountId}<span>账号 #{notification.accountId}</span>{/if}{#if notification.resolvedAt}<span>已恢复于 {formatDate(notification.resolvedAt)}</span>{/if}</div>
          </div>
          <div class="notification-item-actions">
            <button class="icon-button" type="button" title={notification.readAt ? "标记未读" : "标记已读"} aria-label={notification.readAt ? "标记未读" : "标记已读"} disabled={busyId === notification.id} on:click={() => toggleRead(notification)}>{#if notification.readAt}<RotateCcw size={15} />{:else}<Check size={15} />{/if}</button>
            {#if !notification.resolvedAt}<button class="secondary" type="button" disabled={busyId === notification.id} on:click={() => resolve(notification)}>标记已恢复</button>{/if}
          </div>
        </article>
      {/each}
    </div>
  {/if}

  {#if total > PAGE_SIZE}
    <div class="pagination-bar"><span>第 {Math.floor(offset / PAGE_SIZE) + 1} / {Math.ceil(total / PAGE_SIZE)} 页</span><div class="pagination-actions"><button class="icon-button" type="button" title="上一页" aria-label="上一页" disabled={offset === 0 || loading} on:click={() => { offset = Math.max(0, offset - PAGE_SIZE); void refresh(); }}><ChevronLeft size={15} /></button><button class="icon-button" type="button" title="下一页" aria-label="下一页" disabled={offset + PAGE_SIZE >= total || loading} on:click={() => { offset += PAGE_SIZE; void refresh(); }}><ChevronRight size={15} /></button></div></div>
  {/if}
</section>
