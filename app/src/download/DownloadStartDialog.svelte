<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { X } from "lucide-svelte";
  import Dialog from "../controls/Dialog.svelte";
  import SelectField from "../controls/SelectField.svelte";
  import { createDownloadJob } from "./api";
  import { summarizeDownloadContentTypes } from "./presentation";
  import type { DownloadJobDetail, DownloadPreview, DownloadQuality, DownloadSelection, DownloadSettings } from "./types";

  export let accountId: number;
  export let selection: DownloadSelection;
  export let preview: DownloadPreview;
  export let settings: DownloadSettings;
  export let onclose: () => void;
  export let oncreated: (job: DownloadJobDetail) => void;

  let destinationRoot = settings.defaultDirectory;
  let requestedQuality: DownloadQuality = settings.defaultQuality;
  let submitting = false;
  let errorMessage = "";
  $: contentTypeSummary = summarizeDownloadContentTypes(preview.contentTypeCounts);

  async function chooseDirectory(): Promise<void> {
    const selected = await open({ directory: true, multiple: false, defaultPath: destinationRoot || undefined });
    if (typeof selected === "string") destinationRoot = selected;
  }

  async function submit(): Promise<void> {
    if (!destinationRoot.trim()) {
      errorMessage = "请选择下载目录";
      return;
    }
    submitting = true;
    errorMessage = "";
    try {
      const job = await createDownloadJob({
        accountId,
        selection,
        destinationRoot: destinationRoot.trim(),
        requestedQuality,
      });
      oncreated(job);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      submitting = false;
    }
  }
</script>

<Dialog ariaLabel="开始下载" panelClass="download-dialog" busy={submitting} onClose={onclose}>
    <header class="modal-header"><h2>开始批量下载</h2><button class="icon-button" type="button" title="关闭" aria-label="关闭开始下载" data-dialog-close on:click={onclose}><X size={16} /></button></header>
    <p>后端已按抖音 ID 去重：共 <strong>{preview.candidateCount}</strong> 个作品{contentTypeSummary ? `（${contentTypeSummary}）` : ""}。</p>
    <label class="setting-row"><span>保存到</span><input aria-label="下载目标目录" bind:value={destinationRoot} readonly placeholder="选择下载目录" /><button class="secondary" type="button" on:click={chooseDirectory}>选择文件夹</button></label>
    <div class="setting-row"><span>质量</span><SelectField ariaLabel="下载质量" bind:value={requestedQuality} disabled={!settings.askQualityEachJob} options={[{ value: "highest", label: "最高质量" }, { value: "balanced", label: "均衡" }, { value: "space_saver", label: "节省空间" }]} /></div>
    <p class="muted">目录：{settings.askDirectoryEachJob ? "每次任务确认" : "使用默认值"}；质量：{settings.askQualityEachJob ? "每次任务确认" : "使用默认值"}。</p>
    <p class="muted">已完整验证且目标相同的历史资源可被跳过；任务覆盖不会修改默认设置。</p>
    {#if errorMessage}<div class="error-line">{errorMessage}</div>{/if}
    <div class="dialog-actions"><button class="ghost" data-dialog-cancel on:click={onclose}>取消</button><button class="primary" disabled={submitting} on:click={submit}>{submitting ? "创建中" : "创建任务"}</button></div>
</Dialog>
