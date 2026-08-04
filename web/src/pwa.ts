const DISMISS_STORAGE_KEY = "cthuwu.pwa.install-dismissed.v1";
const DISMISS_COOLDOWN_MS = 30 * 24 * 60 * 60 * 1000;
const DEFAULT_APPLE_PROMPT_DELAY_MS = 1_500;

interface InstallChoice {
  outcome: "accepted" | "dismissed";
  platform?: string;
}

interface DeferredInstallPrompt extends Event {
  prompt: () => Promise<void>;
  userChoice: Promise<InstallChoice>;
}

interface NavigatorWithStandalone extends Navigator {
  standalone?: boolean;
}

export interface PwaInstallElements {
  card: HTMLElement;
  title: HTMLElement;
  copy: HTMLElement;
  action: HTMLButtonElement;
  dismiss: HTMLButtonElement;
}

export interface PwaInstallOptions {
  enabled?: boolean;
  window?: Window;
  navigator?: Navigator;
  storage?: Storage;
  now?: () => number;
  applePromptDelayMs?: number;
  onBackupRequested?: () => void;
}

export interface PwaInstallController {
  dispose: () => void;
}

export function initializePwaInstallPrompt(
  elements: PwaInstallElements,
  options: PwaInstallOptions = {},
): PwaInstallController {
  const activeWindow = options.window ?? window;
  const activeNavigator = options.navigator ?? navigator;
  const activeStorage = options.storage ?? readStorage(activeWindow);
  const now = options.now ?? Date.now;
  const enabled = options.enabled ?? true;
  let deferredPrompt: DeferredInstallPrompt | undefined;
  let appleTimer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;
  let backupMode = false;

  const hide = (): void => {
    elements.card.hidden = true;
  };

  const onDismiss = (): void => {
    hide();
    deferredPrompt = undefined;
    backupMode = false;
    writeDismissal(activeStorage, now());
  };

  const onInstall = (): void => {
    const prompt = deferredPrompt;
    if (!prompt) return;
    deferredPrompt = undefined;
    elements.action.disabled = true;
    hide();
    void (async () => {
      try {
        await prompt.prompt();
        await prompt.userChoice;
      } catch {
        // Browser install UI is optional and must never interrupt XMTP startup.
      } finally {
        if (!disposed) elements.action.disabled = false;
      }
    })();
  };

  const onAction = (): void => {
    if (backupMode) {
      (options.onBackupRequested ?? hide)();
      return;
    }
    onInstall();
  };

  const onBeforeInstallPrompt = (event: Event): void => {
    event.preventDefault();
    if (
      isStandalone(activeWindow, activeNavigator) ||
      wasDismissedRecently(activeStorage, now()) ||
      !isDeferredInstallPrompt(event)
    ) {
      return;
    }
    deferredPrompt = event;
    backupMode = false;
    renderNativePrompt(elements);
  };

  const onAppInstalled = (): void => {
    deferredPrompt = undefined;
    hide();
  };

  const dispose = (): void => {
    disposed = true;
    if (appleTimer) clearTimeout(appleTimer);
    activeWindow.removeEventListener("beforeinstallprompt", onBeforeInstallPrompt);
    activeWindow.removeEventListener("appinstalled", onAppInstalled);
    elements.action.removeEventListener("click", onAction);
    elements.dismiss.removeEventListener("click", onDismiss);
    deferredPrompt = undefined;
  };

  hide();
  if (!enabled) return { dispose };

  registerServiceWorker(activeNavigator);
  if (isStandalone(activeWindow, activeNavigator)) return { dispose };

  activeWindow.addEventListener("beforeinstallprompt", onBeforeInstallPrompt);
  activeWindow.addEventListener("appinstalled", onAppInstalled);
  elements.action.addEventListener("click", onAction);
  elements.dismiss.addEventListener("click", onDismiss);

  const appleMode = detectAppleInstallMode(activeNavigator);
  if (appleMode && !wasDismissedRecently(activeStorage, now())) {
    appleTimer = setTimeout(() => {
      if (disposed || isStandalone(activeWindow, activeNavigator)) return;
      backupMode = true;
      renderApplePrompt(elements, appleMode);
    }, options.applePromptDelayMs ?? DEFAULT_APPLE_PROMPT_DELAY_MS);
  }

  return { dispose };
}

function renderNativePrompt(elements: PwaInstallElements): void {
  elements.card.dataset.mode = "native";
  elements.title.textContent = "keep Cthuwu close";
  elements.copy.textContent = "Install a cozy, full-screen portal that is always one tap away.";
  elements.action.textContent = "install Cthuwu";
  elements.action.setAttribute("aria-label", "Install Cthuwu on this device");
  elements.action.disabled = false;
  elements.card.hidden = false;
}

function renderApplePrompt(elements: PwaInstallElements, mode: "ios" | "macos"): void {
  elements.card.dataset.mode = "apple";
  elements.title.textContent = "keep this XMTP identity";
  elements.copy.textContent =
    mode === "ios"
      ? "Apple starts Home Screen apps with separate local storage. Back up first, then tap Share → Add to Home Screen."
      : "Apple starts Dock apps with separate local storage. Back up first, then choose File → Add to Dock.";
  elements.action.textContent = "back up first";
  elements.action.setAttribute("aria-label", "Open identity settings to create an encrypted backup");
  elements.action.disabled = false;
  elements.card.hidden = false;
}

function isDeferredInstallPrompt(event: Event): event is DeferredInstallPrompt {
  const candidate = event as Partial<DeferredInstallPrompt>;
  return (
    typeof candidate.prompt === "function" &&
    typeof (candidate.userChoice as Promise<InstallChoice> | undefined)?.then === "function"
  );
}

function isStandalone(activeWindow: Window, activeNavigator: Navigator): boolean {
  const navigatorStandalone = (activeNavigator as NavigatorWithStandalone).standalone === true;
  return navigatorStandalone || activeWindow.matchMedia?.("(display-mode: standalone)").matches === true;
}

function detectAppleInstallMode(activeNavigator: Navigator): "ios" | "macos" | undefined {
  const userAgent = activeNavigator.userAgent;
  const isSafari =
    /Version\/[\d.]+.*Safari\//i.test(userAgent) &&
    !/(Chrome|Chromium|CriOS|FxiOS|Firefox|Edg|OPR|OPiOS|SamsungBrowser|DuckDuckGo)/i.test(
      userAgent,
    );
  if (!isSafari) return undefined;
  if (/iPhone|iPad|iPod/i.test(userAgent)) return "ios";
  if (/Macintosh/i.test(userAgent) && activeNavigator.maxTouchPoints > 1) return "ios";
  if (/Macintosh/i.test(userAgent)) return "macos";
  return undefined;
}

function readStorage(activeWindow: Window): Storage | undefined {
  try {
    return activeWindow.localStorage;
  } catch {
    return undefined;
  }
}

function wasDismissedRecently(storage: Storage | undefined, now: number): boolean {
  if (!storage) return false;
  try {
    const dismissedAt = Number(storage.getItem(DISMISS_STORAGE_KEY));
    return Number.isFinite(dismissedAt) && dismissedAt > 0 && now - dismissedAt < DISMISS_COOLDOWN_MS;
  } catch {
    return false;
  }
}

function writeDismissal(storage: Storage | undefined, now: number): void {
  if (!storage) return;
  try {
    storage.setItem(DISMISS_STORAGE_KEY, String(now));
  } catch {
    // Install-prompt persistence is optional when storage is unavailable.
  }
}

function registerServiceWorker(activeNavigator: Navigator): void {
  if (!("serviceWorker" in activeNavigator)) return;
  try {
    void activeNavigator.serviceWorker.register("/sw.js", { scope: "/" }).catch(() => undefined);
  } catch {
    // Service-worker support is an enhancement, not a prerequisite for XMTP chat.
  }
}
