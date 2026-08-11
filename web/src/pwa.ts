const DISMISS_STORAGE_KEY = "cthuwu.pwa.install-dismissed.v1";
const SESSION_SHOWN_KEY = "cthuwu.pwa.install-shown.v1";
const DISMISS_COOLDOWN_MS = 7 * 24 * 60 * 60 * 1000;
const DEFAULT_APPLE_PROMPT_DELAY_MS = 1_500;

interface InstallChoice {
  outcome: "accepted" | "dismissed";
  platform?: string;
}

interface DeferredInstallPrompt extends Event {
  prompt: () => Promise<void>;
  userChoice: Promise<InstallChoice>;
}

interface NavigatorWithInstallHints extends Navigator {
  standalone?: boolean;
  userAgentData?: { mobile?: boolean; platform?: string };
}

export interface InstallEnvironment {
  standalone: boolean;
  mobile: boolean;
  ios: boolean;
  safari: boolean;
}

export interface PwaInstallElements {
  card: HTMLElement;
  title: HTMLElement;
  copy: HTMLElement;
  action: HTMLButtonElement;
  dismiss: HTMLButtonElement;
  menuAction?: HTMLButtonElement;
}

export interface PwaUpdateElements {
  card: HTMLElement;
  action: HTMLButtonElement;
  dismiss: HTMLButtonElement;
}

export interface PwaInstallOptions {
  enabled?: boolean;
  window?: Window;
  navigator?: Navigator;
  storage?: Storage;
  sessionStorage?: Storage;
  now?: () => number;
  applePromptDelayMs?: number;
  onBackupRequested?: () => void;
  onMenuRequested?: () => HTMLElement | undefined;
  updateElements?: PwaUpdateElements;
  secureContext?: boolean;
  reload?: () => void;
}

export interface PwaInstallController {
  show: () => void;
  dispose: () => void;
}

export function initializePwaInstallPrompt(
  elements: PwaInstallElements,
  options: PwaInstallOptions = {},
): PwaInstallController {
  const activeWindow = options.window ?? window;
  const activeNavigator = options.navigator ?? navigator;
  const activeStorage = options.storage ?? readStorage(activeWindow, "localStorage");
  const activeSessionStorage =
    options.sessionStorage ?? readStorage(activeWindow, "sessionStorage");
  const now = options.now ?? Date.now;
  const enabled = options.enabled ?? true;
  let environment = detectInstallEnvironment(activeWindow, activeNavigator);
  let deferredPrompt: DeferredInstallPrompt | undefined;
  let appleTimer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;
  let backupMode = false;
  let returnFocus: HTMLElement | undefined;
  let serviceWorkerCleanup = (): void => undefined;

  const hide = (): void => {
    elements.card.hidden = true;
  };

  const show = (): void => {
    environment = detectInstallEnvironment(activeWindow, activeNavigator);
    if (environment.standalone) return;
    if (deferredPrompt) {
      backupMode = false;
      renderNativePrompt(elements);
    } else if (environment.ios) {
      backupMode = true;
      renderApplePrompt(elements, environment.safari);
    } else {
      backupMode = false;
      renderBrowserHelp(elements);
    }
    (elements.action.disabled ? elements.dismiss : elements.action).focus({ preventScroll: true });
  };

  const onMenuAction = (): void => {
    returnFocus = options.onMenuRequested?.() ?? elements.menuAction;
    show();
  };

  const onDismiss = (): void => {
    hide();
    backupMode = false;
    writeTimestamp(activeStorage, DISMISS_STORAGE_KEY, now());
    if (returnFocus) returnFocus.focus({ preventScroll: true });
    else elements.dismiss.blur();
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
        const choice = await prompt.userChoice;
        if (choice.outcome === "dismissed") {
          writeTimestamp(activeStorage, DISMISS_STORAGE_KEY, now());
          if (returnFocus) returnFocus.focus({ preventScroll: true });
          else elements.action.blur();
        }
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

  const showAutomatically = (render: () => void): void => {
    if (
      environment.standalone ||
      !environment.mobile ||
      wasDismissedRecently(activeStorage, now()) ||
      readTimestamp(activeSessionStorage, SESSION_SHOWN_KEY) !== undefined
    ) {
      return;
    }
    writeTimestamp(activeSessionStorage, SESSION_SHOWN_KEY, now());
    render();
  };

  const onBeforeInstallPrompt = (event: Event): void => {
    if (!isDeferredInstallPrompt(event)) return;
    environment = detectInstallEnvironment(activeWindow, activeNavigator);
    if (environment.ios) return;
    event.preventDefault();
    deferredPrompt = event;
    showAutomatically(() => {
      backupMode = false;
      renderNativePrompt(elements);
    });
  };

  const onAppInstalled = (): void => {
    deferredPrompt = undefined;
    hide();
    elements.action.blur();
    if (elements.menuAction) elements.menuAction.hidden = true;
  };

  const dispose = (): void => {
    disposed = true;
    if (appleTimer) clearTimeout(appleTimer);
    activeWindow.removeEventListener("beforeinstallprompt", onBeforeInstallPrompt);
    activeWindow.removeEventListener("appinstalled", onAppInstalled);
    elements.action.removeEventListener("click", onAction);
    elements.dismiss.removeEventListener("click", onDismiss);
    elements.menuAction?.removeEventListener("click", onMenuAction);
    serviceWorkerCleanup();
    deferredPrompt = undefined;
  };

  hide();
  options.updateElements && hideUpdate(options.updateElements);
  if (!enabled) return { show, dispose };

  serviceWorkerCleanup = registerServiceWorker(activeWindow, activeNavigator, options);
  if (environment.standalone) {
    if (elements.menuAction) elements.menuAction.hidden = true;
    return { show, dispose };
  }

  activeWindow.addEventListener("beforeinstallprompt", onBeforeInstallPrompt);
  activeWindow.addEventListener("appinstalled", onAppInstalled);
  elements.action.addEventListener("click", onAction);
  elements.dismiss.addEventListener("click", onDismiss);
  elements.menuAction?.addEventListener("click", onMenuAction);

  if (environment.ios && !wasDismissedRecently(activeStorage, now())) {
    appleTimer = setTimeout(() => {
      environment = detectInstallEnvironment(activeWindow, activeNavigator);
      showAutomatically(() => {
        backupMode = true;
        renderApplePrompt(elements, environment.safari);
      });
    }, options.applePromptDelayMs ?? DEFAULT_APPLE_PROMPT_DELAY_MS);
  }

  return { show, dispose };
}

export function detectInstallEnvironment(
  activeWindow: Window,
  activeNavigator: Navigator,
): InstallEnvironment {
  const hints = activeNavigator as NavigatorWithInstallHints;
  const userAgent = activeNavigator.userAgent ?? "";
  const ios =
    /iPhone|iPad|iPod/iu.test(userAgent) ||
    (/Macintosh/iu.test(userAgent) && activeNavigator.maxTouchPoints > 1);
  const safari =
    /Version\/[\d.]+.*Safari\//iu.test(userAgent) &&
    !/(CriOS|FxiOS|EdgiOS|OPiOS|DuckDuckGo|Chrome|Chromium|Edg|OPR|SamsungBrowser)/iu.test(
      userAgent,
    );
  const coarsePointer = activeWindow.matchMedia?.("(pointer: coarse)").matches === true;
  const mobile =
    hints.userAgentData?.mobile === true ||
    ios ||
    (/Android|Mobile/iu.test(userAgent) && (coarsePointer || activeNavigator.maxTouchPoints > 0));
  const standalone =
    hints.standalone === true ||
    activeWindow.matchMedia?.("(display-mode: standalone)").matches === true ||
    activeWindow.matchMedia?.("(display-mode: fullscreen)").matches === true;
  return { standalone, mobile, ios, safari };
}

function renderNativePrompt(elements: PwaInstallElements): void {
  elements.card.dataset.mode = "native";
  elements.title.textContent = "keep Cthuwu close";
  elements.copy.textContent = "Install the portal for a full-screen home-screen app.";
  elements.action.textContent = "install Cthuwu";
  elements.action.setAttribute("aria-label", "Install Cthuwu on this device");
  elements.action.disabled = false;
  elements.card.hidden = false;
}

function renderApplePrompt(elements: PwaInstallElements, safari: boolean): void {
  elements.card.dataset.mode = "apple";
  elements.title.textContent = "back up before installing";
  elements.copy.textContent = safari
    ? "An installed Safari app may use separate local storage. Back up your identity, then tap Share → Add to Home Screen."
    : "An installed Safari app may use separate local storage. Back up, open this site in Safari, then tap Share → Add to Home Screen.";
  elements.action.textContent = "back up first";
  elements.action.setAttribute("aria-label", "Open identity settings to create an encrypted backup");
  elements.action.disabled = false;
  elements.card.hidden = false;
}

function renderBrowserHelp(elements: PwaInstallElements): void {
  elements.card.dataset.mode = "help";
  elements.title.textContent = "install Cthuwu";
  elements.copy.textContent = "Use your browser menu’s Install app or Add to Home Screen action.";
  elements.action.textContent = "waiting for browser";
  elements.action.setAttribute("aria-label", "Browser install action is not currently available");
  elements.action.disabled = true;
  elements.card.hidden = false;
}

function isDeferredInstallPrompt(event: Event): event is DeferredInstallPrompt {
  const candidate = event as Partial<DeferredInstallPrompt>;
  return (
    typeof candidate.prompt === "function" &&
    typeof (candidate.userChoice as Promise<InstallChoice> | undefined)?.then === "function"
  );
}

function readStorage(activeWindow: Window, key: "localStorage" | "sessionStorage"): Storage | undefined {
  try {
    return activeWindow[key];
  } catch {
    return undefined;
  }
}

function wasDismissedRecently(storage: Storage | undefined, now: number): boolean {
  const dismissedAt = readTimestamp(storage, DISMISS_STORAGE_KEY);
  return dismissedAt !== undefined && now - dismissedAt < DISMISS_COOLDOWN_MS;
}

function readTimestamp(storage: Storage | undefined, key: string): number | undefined {
  if (!storage) return undefined;
  try {
    const timestamp = Number(storage.getItem(key));
    return Number.isFinite(timestamp) && timestamp > 0 ? timestamp : undefined;
  } catch {
    return undefined;
  }
}

function writeTimestamp(storage: Storage | undefined, key: string, now: number): void {
  if (!storage) return;
  try {
    storage.setItem(key, String(now));
  } catch {
    // Prompt persistence is optional when storage is unavailable.
  }
}

function registerServiceWorker(
  activeWindow: Window,
  activeNavigator: Navigator,
  options: PwaInstallOptions,
): () => void {
  if (!("serviceWorker" in activeNavigator)) return () => undefined;
  const secure = options.secureContext ?? activeWindow.isSecureContext === true;
  if (!secure) return () => undefined;
  const updateElements = options.updateElements;
  const cleanups: Array<() => void> = [];
  let disposed = false;
  try {
    void activeNavigator.serviceWorker
      .register("/sw.js", { scope: "/", updateViaCache: "none" })
      .then((registration) => {
        if (!updateElements || disposed) return;
        let observedInstalling: ServiceWorker | undefined;
        const showWaiting = (): void => {
          if (!registration.waiting) return;
          updateElements.card.hidden = false;
        };
        showWaiting();
        const onUpdateFound = (): void => {
          const worker = registration.installing;
          if (!worker || worker === observedInstalling) return;
          observedInstalling = worker;
          const onStateChange = (): void => {
            if (worker.state === "installed" && activeNavigator.serviceWorker.controller) {
              // The registration's waiting slot can update just after the worker state event.
              const timer = activeWindow.setTimeout(showWaiting, 0);
              cleanups.push(() => activeWindow.clearTimeout(timer));
            }
          };
          worker.addEventListener("statechange", onStateChange);
          cleanups.push(() => worker.removeEventListener("statechange", onStateChange));
        };
        registration.addEventListener("updatefound", onUpdateFound);
        // Registration can resolve after updatefound has already fired.
        onUpdateFound();
        const reload = options.reload ?? (() => activeWindow.location.reload());
        let reloadRequested = false;
        const onControllerChange = (): void => {
          if (reloadRequested) reload();
        };
        activeNavigator.serviceWorker.addEventListener("controllerchange", onControllerChange);
        const onReload = (): void => {
          if (!registration.waiting) return;
          reloadRequested = true;
          registration.waiting.postMessage({ type: "SKIP_WAITING" });
        };
        const onDismissUpdate = (): void => hideUpdate(updateElements);
        updateElements.action.addEventListener("click", onReload);
        updateElements.dismiss.addEventListener("click", onDismissUpdate);
        cleanups.push(
          () => registration.removeEventListener("updatefound", onUpdateFound),
          () => activeNavigator.serviceWorker.removeEventListener("controllerchange", onControllerChange),
          () => updateElements.action.removeEventListener("click", onReload),
          () => updateElements.dismiss.removeEventListener("click", onDismissUpdate),
        );
      })
      .catch(() => undefined);
  } catch {
    // Service-worker support is an enhancement, not a prerequisite for XMTP chat.
  }
  return () => {
    disposed = true;
    for (const cleanup of cleanups.splice(0)) cleanup();
  };
}

function hideUpdate(elements: PwaUpdateElements): void {
  elements.card.hidden = true;
}
