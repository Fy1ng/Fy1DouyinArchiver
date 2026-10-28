import type { DownloadSelection } from "./types.js";

export type DownloadSelectionInput =
  | { mode: "checked"; checkedIds: Iterable<number> }
  | { mode: "filtered"; query?: string }
  | { mode: "all" };

export function buildDownloadSelection(input: DownloadSelectionInput): DownloadSelection {
  if (input.mode === "checked") {
    return {
      kind: "checked",
      watch_record_ids: [...new Set([...input.checkedIds].filter(Number.isSafeInteger))],
    };
  }
  if (input.mode === "filtered") {
    const query = input.query?.trim();
    return query ? { kind: "filtered", query } : { kind: "filtered" };
  }
  return { kind: "all" };
}
