<script lang="ts">
  import Dialog from "./Dialog.svelte";
  import AppShell from "../shell/AppShell.svelte";

  let primaryOpen = false;
  let nestedOpen = false;
</script>

<AppShell>
  <button type="button" onclick={() => (primaryOpen = true)}>打开主弹窗</button>
  {#if primaryOpen}
    <Dialog ariaLabel="主弹窗" onClose={() => (primaryOpen = false)}>
      <h2>主弹窗</h2>
      <button type="button" data-dialog-cancel onclick={() => (primaryOpen = false)}>取消主弹窗</button>
      <button type="button" onclick={() => (nestedOpen = true)}>打开嵌套弹窗</button>
    </Dialog>
  {/if}
  {#if nestedOpen}
    <Dialog ariaLabel="嵌套弹窗" onClose={() => (nestedOpen = false)}>
      <h2>嵌套弹窗</h2>
      <button type="button" data-dialog-cancel onclick={() => (nestedOpen = false)}>取消嵌套弹窗</button>
      <button type="button">嵌套次要操作</button>
    </Dialog>
  {/if}
</AppShell>
