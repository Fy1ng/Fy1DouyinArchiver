export type AppPage =
  | "overview"
  | "library"
  | "search"
  | "accounts"
  | "history"
  | "likes"
  | "favorites"
  | "remoteFolders"
  | "localFolders"
  | "syncRecords"
  | "downloads"
  | "notifications"
  | "settings";

export interface SourceAvailability {
  history: boolean;
  likes: boolean;
  favorites: boolean;
}
