<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import {
    handleDialogKeydown,
    registerDialog,
    requestDialogClose,
    type DialogRegistration,
  } from "./dialog-stack";

  let {
    ariaLabel,
    panelClass = "settings-modal",
    backdropClass = "",
    initialFocus,
    busy = false,
    closeOnBackdrop = true,
    closeOnEscape = true,
    onClose,
    children,
  }: {
    ariaLabel: string;
    panelClass?: string;
    backdropClass?: string;
    initialFocus?: string;
    busy?: boolean;
    closeOnBackdrop?: boolean;
    closeOnEscape?: boolean;
    onClose: () => void;
    children?: Snippet;
  } = $props();

  let backdrop: HTMLElement;
  let panel: HTMLElement;
  let registration: DialogRegistration | null = null;

  function portal(node: HTMLElement): { destroy: () => void } {
    const workspace = node.closest<HTMLElement>('[data-testid="app-workspace"]');
    const host = workspace?.querySelector<HTMLElement>("[data-modal-host]");
    if (host) host.appendChild(node);
    return {
      destroy: () => {
        node.remove();
      },
    };
  }

  function handleBackdropClick(event: MouseEvent): void {
    if (!closeOnBackdrop || event.target !== event.currentTarget) return;
    requestDialogClose(registration);
  }

  onMount(() => {
    registration = registerDialog({
      backdrop,
      panel,
      initialFocus,
      canClose: () => !busy,
      closeOnEscape: () => closeOnEscape,
      onClose: () => onClose(),
    });
    return () => {
      registration?.destroy();
      registration = null;
    };
  });
</script>

<div
  bind:this={backdrop}
  class={`modal-backdrop ${backdropClass}`.trim()}
  data-dialog-layer
  role="presentation"
  use:portal
  onclick={handleBackdropClick}
>
  <div
    bind:this={panel}
    class={panelClass}
    role="dialog"
    aria-modal="true"
    aria-label={ariaLabel}
    tabindex="-1"
    onclick={(event) => event.stopPropagation()}
    onkeydown={(event) => handleDialogKeydown(registration, event)}
  >
    {@render children?.()}
  </div>
</div>
