<script lang="ts">
  import { onMount, tick, type Snippet } from "svelte";
  import {
    Bell,
    CircleHelp,
    Database,
    Download,
    MonitorCog,
    Palette,
    RefreshCw,
    Search,
    Users,
  } from "lucide-svelte";

  export type SettingsCategory =
    | "general"
    | "accounts"
    | "content"
    | "storage"
    | "downloads"
    | "backup"
    | "notifications"
    | "updates"
    | "diagnostics";
  export type SettingsSaveState = "idle" | "saving" | "saved" | "fading" | "error";
  type SettingsPanel = {
    id: string;
    label: string;
    category: SettingsCategory;
  };

  const categories = [
    { id: "general", label: "常规与外观", icon: Palette },
    { id: "accounts", label: "账号与同步", icon: Users },
    { id: "content", label: "内容显示", icon: MonitorCog },
    { id: "storage", label: "存储与缓存", icon: Database },
    { id: "downloads", label: "下载", icon: Download },
    { id: "backup", label: "备份与恢复", icon: RefreshCw },
    { id: "notifications", label: "通知", icon: Bell },
    { id: "updates", label: "更新", icon: RefreshCw },
    { id: "diagnostics", label: "诊断与关于", icon: CircleHelp },
  ] as const;

  let {
    saveState = "idle",
    onSaveFeedbackHidden,
    children,
  }: {
    saveState?: SettingsSaveState;
    onSaveFeedbackHidden?: () => void;
    children?: Snippet;
  } = $props();

  let activeCategory = $state<SettingsCategory>("general");
  let activePanel = $state("");
  let panels = $state<SettingsPanel[]>([]);
  let query = $state("");
  let contentRoot: HTMLElement;
  let highlightTimer: number | null = null;

  function categorySections(): HTMLElement[] {
    return contentRoot
      ? Array.from(contentRoot.querySelectorAll<HTMLElement>("[data-settings-category]"))
      : [];
  }

  const activePanels = $derived(
    panels.filter((panel) => panel.category === activeCategory),
  );

  function refreshPanels(): void {
    const discovered = categorySections().flatMap((section) => {
      const category = section.dataset.settingsCategory as SettingsCategory | undefined;
      const id = section.dataset.settingsPanel;
      const label = section.dataset.settingsPanelLabel;
      return category && id && label ? [{ category, id, label }] : [];
    });
    panels = discovered;
    if (!discovered.some((panel) => panel.id === activePanel && panel.category === activeCategory)) {
      activePanel = discovered.find((panel) => panel.category === activeCategory)?.id ?? "";
    }
    syncPanelVisibility();
  }

  function syncPanelVisibility(): void {
    for (const section of categorySections()) {
      const visible = section.dataset.settingsCategory === activeCategory
        && section.dataset.settingsPanel === activePanel;
      section.hidden = !visible;
    }
  }

  async function selectCategory(category: SettingsCategory): Promise<void> {
    activeCategory = category;
    activePanel = panels.find((panel) => panel.category === category)?.id ?? "";
    query = "";
    await tick();
    syncPanelVisibility();
  }

  async function selectPanel(panel: SettingsPanel): Promise<void> {
    activeCategory = panel.category;
    activePanel = panel.id;
    query = "";
    await tick();
    syncPanelVisibility();
  }

  async function searchSettings(value: string): Promise<void> {
    query = value;
    const normalized = value.trim().toLocaleLowerCase("zh-CN");
    if (!normalized) return;

    const match = categorySections().find((section) =>
      (section.textContent ?? "").toLocaleLowerCase("zh-CN").includes(normalized),
    );
    if (!match) return;

    const category = match.dataset.settingsCategory as SettingsCategory | undefined;
    const panel = match.dataset.settingsPanel;
    if (!category || !panel || !categories.some((item) => item.id === category)) return;
    activeCategory = category;
    activePanel = panel;
    await tick();
    syncPanelVisibility();
    match.classList.add("settings-search-hit");
    if (highlightTimer !== null) window.clearTimeout(highlightTimer);
    highlightTimer = window.setTimeout(() => {
      match.classList.remove("settings-search-hit");
      highlightTimer = null;
    }, 1_800);
  }

  onMount(() => {
    refreshPanels();
    const observer = new MutationObserver(refreshPanels);
    observer.observe(contentRoot, { childList: true });
    return () => {
      observer.disconnect();
      if (highlightTimer !== null) window.clearTimeout(highlightTimer);
    };
  });

  const saveLabel = $derived(
    saveState === "saving"
      ? "保存中"
      : saveState === "error"
        ? "保存失败"
        : saveState === "saved" || saveState === "fading"
          ? "已保存"
          : "",
  );
</script>

<section class="settings-workspace" aria-label="设置">
  <header class="settings-workspace-toolbar">
    <label class="settings-search">
      <Search size={16} />
      <input
        type="search"
        aria-label="搜索设置"
        placeholder="搜索设置"
        value={query}
        oninput={(event) => searchSettings((event.currentTarget as HTMLInputElement).value)}
      />
    </label>
    <span
      class:error={saveState === "error"}
      class:fading={saveState === "fading"}
      class="settings-save-status"
      aria-live="polite"
      onanimationend={() => saveState === "fading" && onSaveFeedbackHidden?.()}
    >
      {saveLabel}
    </span>
  </header>

  <div class="settings-workspace-body">
    <div class="settings-navigation-rail">
      <nav class="settings-category-nav" aria-label="设置分类">
        {#each categories as category}
          {@const Icon = category.icon}
          <button
            type="button"
            class:active={activeCategory === category.id}
            aria-current={activeCategory === category.id ? "page" : undefined}
            onclick={() => selectCategory(category.id)}
          >
            <Icon size={16} />
            <span>{category.label}</span>
          </button>
        {/each}
      </nav>
      <nav class="settings-panel-nav" aria-label="设置子项">
        {#each activePanels as panel}
          <button
            type="button"
            class:active={activePanel === panel.id}
            aria-current={activePanel === panel.id ? "page" : undefined}
            onclick={() => selectPanel(panel)}
          >{panel.label}</button>
        {/each}
      </nav>
    </div>

    <div class="settings-workspace-content" bind:this={contentRoot}>
      {@render children?.()}
    </div>
  </div>
</section>
