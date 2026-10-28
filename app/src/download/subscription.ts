export type RefreshRegistration = (refresh: () => void) => Promise<() => void>;

export function installDownloadRefreshSubscription(
  register: RefreshRegistration,
  refresh: () => void,
): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  const guardedRefresh = () => {
    if (!disposed) refresh();
  };
  void register(guardedRefresh).then((registered) => {
    if (disposed) registered();
    else unlisten = registered;
  }).catch(() => {});
  return () => {
    if (disposed) return;
    disposed = true;
    unlisten?.();
    unlisten = undefined;
  };
}
