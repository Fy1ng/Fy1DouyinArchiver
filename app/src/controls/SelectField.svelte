<script lang="ts">
  import { onMount, createEventDispatcher } from "svelte";
  import { Check, ChevronDown } from "lucide-svelte";

  export type SelectOption = {
    value: string;
    label: string;
    disabled?: boolean;
  };

  export let value = "";
  export let options: SelectOption[] = [];
  export let ariaLabel = "选择项";
  export let disabled = false;
  export let placeholder = "请选择";
  export let onchange: (value: string) => void = () => undefined;
  export let oninput: (value: string) => void = () => undefined;

  const dispatch = createEventDispatcher<{
    change: { value: string };
    input: string;
  }>();

  let root: HTMLDivElement;
  let trigger: HTMLButtonElement;
  let open = false;

  $: selected = options.find((option) => option.value === value) ?? null;

  function commit(nextValue: string): void {
    if (disabled || !options.some((option) => option.value === nextValue && !option.disabled)) return;
    value = nextValue;
    open = false;
    oninput(value);
    onchange(value);
    dispatch("input", value);
    dispatch("change", { value });
  }

  function handleNativeChange(event: Event): void {
    commit((event.currentTarget as HTMLButtonElement).value);
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (disabled) return;
    if (event.key === "Escape") {
      open = false;
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      open = !open;
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const enabled = options.filter((option) => !option.disabled);
    if (enabled.length === 0) return;
    const currentIndex = Math.max(0, enabled.findIndex((option) => option.value === value));
    const nextIndex = event.key === "ArrowDown"
      ? Math.min(enabled.length - 1, currentIndex + 1)
      : Math.max(0, currentIndex - 1);
    commit(enabled[nextIndex].value);
  }

  function handleDocumentPointerdown(event: PointerEvent): void {
    if (open && root && !root.contains(event.target as Node)) open = false;
  }

  onMount(() => {
    Object.defineProperty(trigger, "options", {
      configurable: true,
      get: () => options.map((option) => {
        const nativeOption = document.createElement("option");
        nativeOption.value = option.value;
        nativeOption.disabled = Boolean(option.disabled);
        nativeOption.textContent = option.label;
        return nativeOption;
      }),
    });
    document.addEventListener("pointerdown", handleDocumentPointerdown);
    return () => document.removeEventListener("pointerdown", handleDocumentPointerdown);
  });
</script>

<div class="custom-select" class:is-open={open} class:is-disabled={disabled} bind:this={root}>
  <button
    class="custom-select-trigger"
    type="button"
    aria-label={ariaLabel}
    aria-haspopup="listbox"
    aria-expanded={open}
    value={value}
    bind:this={trigger}
    disabled={disabled}
    on:click={() => (open = !open)}
    on:change={handleNativeChange}
    on:keydown={handleKeydown}
  >
    <span class:placeholder={!selected}>{selected?.label ?? placeholder}</span>
    <ChevronDown size={15} aria-hidden="true" />
  </button>

  {#if open}
    <div class="custom-select-menu" role="listbox" aria-label={ariaLabel}>
      {#each options as option}
        <button
          class="custom-select-option"
          class:active={option.value === value}
          type="button"
          role="option"
          aria-selected={option.value === value}
          disabled={option.disabled}
          on:click={() => commit(option.value)}
        >
          <span>{option.label}</span>
          {#if option.value === value}<Check size={14} aria-hidden="true" />{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>
