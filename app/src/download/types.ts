export type DownloadQuality = "highest" | "balanced" | "space_saver";

export type DownloadSelection =
  | { kind: "checked"; watch_record_ids: number[] }
  | { kind: "filtered"; query?: string }
  | { kind: "all" };

export interface DownloadSettings {
  defaultDirectory: string;
  defaultQuality: DownloadQuality;
  askDirectoryEachJob: boolean;
  askQualityEachJob: boolean;
}

export interface DownloadCreateJobRequest {
  accountId: number;
  selection: DownloadSelection;
  destinationRoot: string;
  requestedQuality: DownloadQuality;
}

export interface DownloadJobSummary {
  id: number;
  accountId: number;
  state: string;
  destinationRoot: string;
  requestedQuality: DownloadQuality;
  candidateCount: number;
  resolvedCount: number;
  completedCount: number;
  skippedCount: number;
  failedCount: number;
  downloadedBytes: number;
  totalBytes: number;
  createdAt: string;
  startedAt?: string;
  errorCode?: string;
}

export interface DownloadPreview {
  candidateCount: number;
  contentTypeCounts: Record<string, number>;
}

export interface DownloadItemSummary {
  id: number;
  awemeId: string;
  title?: string;
  authorName?: string;
  contentType: string;
  state: string;
  completedAssetCount: number;
  totalRequiredAssetCount: number;
  errorCode?: string;
  errorMessage?: string;
}

export interface DownloadJobDetail extends DownloadJobSummary {
  items: DownloadItemSummary[];
  assets: DownloadAssetSummary[];
  currentResolverItemId?: number;
  activeAssetCount: number;
  cancelledAssetCount: number;
  averageBytesPerSecond: number;
}

export interface DownloadAssetSummary {
  id: number;
  itemId: number;
  stableKey: string;
  assetType: string;
  state: string;
  downloadedBytes: number;
  expectedSize?: number;
  retryCount: number;
  lastErrorCode?: string;
  relativeFinalPath: string;
}
