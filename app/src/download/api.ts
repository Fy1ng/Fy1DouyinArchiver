import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DownloadCreateJobRequest,
  DownloadJobDetail,
  DownloadJobSummary,
  DownloadPreview,
  DownloadSelection,
} from "./types.js";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function createDownloadJob(request: DownloadCreateJobRequest): Promise<DownloadJobDetail> {
  if (!isTauri()) throw new Error("Web 预览模式不能创建下载任务，请在桌面应用中运行。");
  return invoke("download_create_job", { request });
}

export async function listDownloadJobs(): Promise<DownloadJobSummary[]> {
  return isTauri() ? invoke("download_list_jobs") : [];
}

export async function previewDownloadSelection(
  accountId: number,
  selection: DownloadSelection,
): Promise<DownloadPreview> {
  if (!isTauri()) return { candidateCount: 0, contentTypeCounts: {} };
  return invoke("download_preview_selection", { accountId, selection });
}

export async function getDownloadJob(jobId: number): Promise<DownloadJobDetail> {
  if (!isTauri()) throw new Error("Web 预览模式没有下载任务详情。");
  return invoke("download_get_job", { jobId });
}

async function mutateJob(command: string, jobId: number): Promise<DownloadJobDetail> {
  return invoke(command, { jobId });
}

export const pauseDownloadJob = (jobId: number) => mutateJob("download_pause_job", jobId);
export const resumeDownloadJob = (jobId: number) => mutateJob("download_resume_job", jobId);
export const cancelDownloadJob = (jobId: number) => mutateJob("download_cancel_job", jobId);
export const retryFailedDownload = (jobId: number) => mutateJob("download_retry_failed", jobId);

export async function revealDownloadJob(jobId: number): Promise<void> {
  await invoke("download_reveal_path", { jobId, itemId: null });
}

export async function revealDownloadItem(itemId: number): Promise<void> {
  await invoke("download_reveal_path", { jobId: null, itemId });
}

export type DownloadEventRegistration = (
  event: string,
  refresh: () => void,
) => Promise<UnlistenFn>;

export async function registerDownloadRefreshListeners(
  register: DownloadEventRegistration,
  refresh: () => void,
): Promise<UnlistenFn> {
  const unlistenJob = await register("download_job_updated", refresh);
  try {
    const unlistenItem = await register("download_item_updated", refresh);
    return () => {
      unlistenJob();
      unlistenItem();
    };
  } catch (error) {
    unlistenJob();
    throw error;
  }
}

export async function listenForDownloadRefresh(refresh: () => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  return registerDownloadRefreshListeners(
    (event, handler) => listen(event, handler),
    refresh,
  );
}
