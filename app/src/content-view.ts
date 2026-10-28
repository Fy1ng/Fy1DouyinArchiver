import type { ContentSort, SyncSource } from "./types";

export type ContentSource = Exclude<SyncSource, "remoteFolder">;
export type ContentViewMode = "list" | "grid";

export interface ContentViewState {
  query: string;
  page: number;
  viewMode: ContentViewMode;
  includeRemoteRemoved: boolean;
  sort: ContentSort;
}

export type ContentViewStates = Record<ContentSource, ContentViewState>;

function defaultState(sort: ContentSort): ContentViewState {
  return {
    query: "",
    page: 1,
    viewMode: "list",
    includeRemoteRemoved: false,
    sort,
  };
}

export function createContentViewStates(): ContentViewStates {
  return {
    history: defaultState("watchedNewest"),
    likes: defaultState("pageOrder"),
    favorites: defaultState("pageOrder"),
  };
}

export function updateContentViewState(
  states: ContentViewStates,
  source: ContentSource,
  patch: Partial<ContentViewState>,
): ContentViewStates {
  return {
    ...states,
    [source]: { ...states[source], ...patch },
  };
}

export function resetContentViewStatesForAccount(states: ContentViewStates): ContentViewStates {
  return {
    history: { ...states.history, page: 1 },
    likes: { ...states.likes, page: 1 },
    favorites: { ...states.favorites, page: 1 },
  };
}
