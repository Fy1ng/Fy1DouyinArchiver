<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    collapsed = false,
    children,
  }: {
    collapsed?: boolean;
    children?: Snippet;
  } = $props();
</script>

<div
  class="workspace-shell"
  class:sidebar-collapsed={collapsed}
  data-testid="app-workspace"
>
  <div class="workspace-background" data-modal-background data-testid="workspace-background">
    {@render children?.()}
  </div>
  <div class="modal-host" data-modal-host data-testid="modal-host"></div>
</div>
