<script lang="ts">
  import type { Snippet } from "svelte";
  import { X } from "lucide-svelte";
  import Dialog from "./Dialog.svelte";

  let {
    title,
    ariaLabel = title,
    confirmLabel = "确认",
    cancelLabel = "取消",
    danger = false,
    busy = false,
    onClose,
    onConfirm,
    children,
    actions,
  }: {
    title: string;
    ariaLabel?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    danger?: boolean;
    busy?: boolean;
    onClose: () => void;
    onConfirm?: () => void;
    children?: Snippet;
    actions?: Snippet;
  } = $props();
</script>

<Dialog
  {ariaLabel}
  panelClass="settings-modal confirmation-modal"
  backdropClass="confirmation-backdrop"
  initialFocus="[data-dialog-cancel]"
  {busy}
  {onClose}
>
  <header class="modal-header">
    <h2>{title}</h2>
    <button
      class="icon-button"
      type="button"
      aria-label={`关闭${ariaLabel}`}
      data-dialog-close
      disabled={busy}
      onclick={onClose}
    ><X size={16} /></button>
  </header>
  <div class="confirmation-content">
    {@render children?.()}
    <div class="confirmation-actions">
      {#if actions}
        {@render actions()}
      {:else}
        <button type="button" data-dialog-cancel disabled={busy} onclick={onClose}>{cancelLabel}</button>
        <button class={danger ? "danger" : "primary"} type="button" disabled={busy} onclick={onConfirm}>{confirmLabel}</button>
      {/if}
    </div>
  </div>
</Dialog>
