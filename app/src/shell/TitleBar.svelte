<script lang="ts">
  import { Minus, Square, X } from "lucide-svelte";
  import appIconUrl from "../assets/app-icon.png";

  async function withCurrentWindow(action: "drag" | "minimize" | "maximize" | "close") {
    if (!("__TAURI_INTERNALS__" in window)) return;
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const currentWindow = getCurrentWindow();
    if (action === "drag") await currentWindow.startDragging();
    if (action === "minimize") await currentWindow.minimize();
    if (action === "maximize") await currentWindow.toggleMaximize();
    if (action === "close") await currentWindow.close();
  }

  function startDragging(event: MouseEvent) {
    if (event.button !== 0 || event.detail > 1 || (event.target as HTMLElement).closest("button")) return;
    void withCurrentWindow("drag");
  }

  function toggleMaximize(event: MouseEvent) {
    if ((event.target as HTMLElement).closest("button")) return;
    void withCurrentWindow("maximize");
  }
</script>

<header
  class="window-titlebar"
  role="presentation"
  data-tauri-drag-region
  on:mousedown={startDragging}
  on:dblclick={toggleMaximize}
>
  <div class="window-title" data-tauri-drag-region>
    <img src={appIconUrl} alt="" aria-hidden="true" draggable="false" />
    <span data-tauri-drag-region>Fy1DouyinArchiver</span>
  </div>
  <div class="window-controls" aria-label="窗口控制">
    <button type="button" title="最小化" aria-label="最小化" on:click={() => void withCurrentWindow("minimize")}><Minus size={15} strokeWidth={1.7} /></button>
    <button type="button" title="最大化或还原" aria-label="最大化或还原" on:click={() => void withCurrentWindow("maximize")}><Square size={12} strokeWidth={1.7} /></button>
    <button class="window-close" type="button" title="关闭" aria-label="关闭" on:click={() => void withCurrentWindow("close")}><X size={16} strokeWidth={1.7} /></button>
  </div>
</header>
