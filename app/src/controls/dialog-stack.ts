const FOCUSABLE_SELECTOR = [
  "button:not([disabled])",
  "a[href]",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

interface DialogEntry {
  backdrop: HTMLElement;
  panel: HTMLElement;
  trigger: HTMLElement | null;
  initialFocus?: string;
  canClose: () => boolean;
  closeOnEscape: () => boolean;
  onClose: () => void;
}

export interface DialogRegistration {
  entry: DialogEntry;
  destroy: () => void;
}

interface BackgroundState {
  element: HTMLElement;
  ariaHidden: string | null;
  inert: boolean;
}

const stack: DialogEntry[] = [];
let backgroundState: BackgroundState | null = null;
let lastInteractionTarget: HTMLElement | null = null;
let interactionGeneration = 0;

function rememberInteractionTarget(target: EventTarget | null): void {
  if (!(target instanceof HTMLElement)) return;
  lastInteractionTarget = target;
  const generation = ++interactionGeneration;
  window.setTimeout(() => {
    if (generation === interactionGeneration) lastInteractionTarget = null;
  }, 0);
}

if (typeof document !== "undefined") {
  document.addEventListener("pointerdown", (event) => rememberInteractionTarget(event.target), true);
  document.addEventListener("click", (event) => rememberInteractionTarget(event.target), true);
  document.addEventListener(
    "keydown",
    () => {
      if (document.activeElement instanceof HTMLElement && document.activeElement !== document.body) {
        rememberInteractionTarget(document.activeElement);
      }
    },
    true,
  );
}

function setInert(element: HTMLElement, inert: boolean): void {
  (element as HTMLElement & { inert: boolean }).inert = inert;
  if (inert) element.setAttribute("inert", "");
  else element.removeAttribute("inert");
}

function findBackground(entry: DialogEntry): HTMLElement | null {
  const host = entry.backdrop.closest<HTMLElement>("[data-modal-host]");
  return host?.parentElement?.querySelector<HTMLElement>("[data-modal-background]") ?? null;
}

function syncLayers(): void {
  if (stack.length > 0 && backgroundState === null) {
    const background = findBackground(stack[0]);
    if (background) {
      backgroundState = {
        element: background,
        ariaHidden: background.getAttribute("aria-hidden"),
        inert: Boolean((background as HTMLElement & { inert?: boolean }).inert),
      };
      setInert(background, true);
      background.setAttribute("aria-hidden", "true");
    }
    document.body.classList.add("modal-open");
  }

  stack.forEach((entry, index) => {
    const active = index === stack.length - 1;
    setInert(entry.backdrop, !active);
    if (active) entry.backdrop.removeAttribute("aria-hidden");
    else entry.backdrop.setAttribute("aria-hidden", "true");
  });

  if (stack.length === 0) {
    if (backgroundState) {
      setInert(backgroundState.element, backgroundState.inert);
      if (backgroundState.ariaHidden === null) backgroundState.element.removeAttribute("aria-hidden");
      else backgroundState.element.setAttribute("aria-hidden", backgroundState.ariaHidden);
      backgroundState = null;
    }
    document.body.classList.remove("modal-open");
  }
}

function focusableElements(panel: HTMLElement): HTMLElement[] {
  return [...panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)].filter(
    (element) => !element.hasAttribute("hidden") && element.getAttribute("aria-hidden") !== "true",
  );
}

function focusInitial(entry: DialogEntry): void {
  if (stack.at(-1) !== entry) return;
  const requested = entry.initialFocus ? entry.panel.querySelector<HTMLElement>(entry.initialFocus) : null;
  const target =
    requested ??
    entry.panel.querySelector<HTMLElement>("[data-dialog-cancel]") ??
    entry.panel.querySelector<HTMLElement>("[data-dialog-close]") ??
    focusableElements(entry.panel)[0] ??
    entry.panel;
  target.focus({ preventScroll: true });
}

export function registerDialog(
  entry: Omit<DialogEntry, "trigger">,
): DialogRegistration {
  const registered: DialogEntry = {
    ...entry,
    trigger:
      lastInteractionTarget?.isConnected
        ? lastInteractionTarget
        : document.activeElement instanceof HTMLElement && document.activeElement !== document.body
          ? document.activeElement
          : null,
  };
  stack.push(registered);
  syncLayers();
  queueMicrotask(() => focusInitial(registered));

  return {
    entry: registered,
    destroy: () => {
      const index = stack.indexOf(registered);
      if (index >= 0) stack.splice(index, 1);
      setInert(registered.backdrop, false);
      registered.backdrop.removeAttribute("aria-hidden");
      syncLayers();
      const restoreTarget = registered.trigger;
      queueMicrotask(() => {
        if (restoreTarget?.isConnected) restoreTarget.focus({ preventScroll: true });
        else if (stack.length > 0) focusInitial(stack.at(-1)!);
      });
    },
  };
}

export function isTopDialog(registration: DialogRegistration | null): boolean {
  return registration !== null && stack.at(-1) === registration.entry;
}

export function requestDialogClose(registration: DialogRegistration | null): void {
  if (registration === null || stack.at(-1) !== registration.entry || !registration.entry.canClose()) return;
  registration.entry.onClose();
}

export function handleDialogKeydown(
  registration: DialogRegistration | null,
  event: KeyboardEvent,
): void {
  if (registration === null || stack.at(-1) !== registration.entry) return;
  const entry = registration.entry;
  if (event.key === "Escape") {
    if (!entry.canClose() || !entry.closeOnEscape()) return;
    event.preventDefault();
    event.stopPropagation();
    entry.onClose();
    return;
  }
  if (event.key !== "Tab") return;

  const focusable = focusableElements(entry.panel);
  if (focusable.length === 0) {
    event.preventDefault();
    entry.panel.focus({ preventScroll: true });
    return;
  }
  const first = focusable[0];
  const last = focusable.at(-1)!;
  const active = document.activeElement;
  if (event.shiftKey && (active === first || !entry.panel.contains(active))) {
    event.preventDefault();
    last.focus({ preventScroll: true });
  } else if (!event.shiftKey && (active === last || !entry.panel.contains(active))) {
    event.preventDefault();
    first.focus({ preventScroll: true });
  }
}
