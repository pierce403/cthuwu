import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  detectInstallEnvironment,
  initializePwaInstallPrompt,
  type PwaInstallController,
  type PwaInstallElements,
  type PwaUpdateElements,
} from "./pwa";

const NOW = 1_000_000;
let controller: PwaInstallController | undefined;

function mountElements(): PwaInstallElements {
  document.body.innerHTML = `
    <aside id="card" hidden>
      <p id="title"></p>
      <p id="copy"></p>
      <button id="action" type="button"></button>
      <button id="dismiss" type="button"></button>
      <button id="menu" type="button"></button>
    </aside>`;
  return {
    card: document.querySelector<HTMLElement>("#card")!,
    title: document.querySelector<HTMLElement>("#title")!,
    copy: document.querySelector<HTMLElement>("#copy")!,
    action: document.querySelector<HTMLButtonElement>("#action")!,
    dismiss: document.querySelector<HTMLButtonElement>("#dismiss")!,
    menuAction: document.querySelector<HTMLButtonElement>("#menu")!,
  };
}

function mountUpdateElements(): PwaUpdateElements {
  const card = document.createElement("aside");
  const action = document.createElement("button");
  const dismiss = document.createElement("button");
  card.append(action, dismiss);
  document.body.append(card);
  return { card, action, dismiss };
}

function androidNavigator(extra: Record<string, unknown> = {}): Navigator {
  return {
    userAgent: "Mozilla/5.0 (Linux; Android 17; Pixel 9 Pro) AppleWebKit/537.36 Chrome/140 Mobile Safari/537.36",
    maxTouchPoints: 5,
    ...extra,
  } as unknown as Navigator;
}

function installEvent(outcome: "accepted" | "dismissed" = "accepted"): {
  event: Event;
  prompt: ReturnType<typeof vi.fn>;
} {
  const event = new Event("beforeinstallprompt", { cancelable: true });
  const prompt = vi.fn(async () => undefined);
  Object.defineProperties(event, {
    prompt: { value: prompt },
    userChoice: { value: Promise.resolve({ outcome, platform: "web" }) },
  });
  return { event, prompt };
}

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn(() => ({ matches: false })),
  });
});

afterEach(() => {
  controller?.dispose();
  controller = undefined;
  vi.useRealTimers();
});

describe("PWA install prompt", () => {
  it("uses platform hints and display mode in addition to user-agent parsing", () => {
    const hinted = androidNavigator({
      userAgent: "",
      userAgentData: { mobile: true, platform: "Android" },
    });
    expect(detectInstallEnvironment(window, hinted)).toMatchObject({
      mobile: true,
      standalone: false,
    });
  });

  it("captures the Chromium prompt and invokes it exactly once from the install button", async () => {
    const elements = mountElements();
    controller = initializePwaInstallPrompt(elements, {
      now: () => NOW,
      navigator: androidNavigator(),
    });
    const deferred = installEvent();

    window.dispatchEvent(deferred.event);

    expect(deferred.event.defaultPrevented).toBe(true);
    expect(elements.card.hidden).toBe(false);
    expect(elements.card.dataset.mode).toBe("native");
    expect(elements.action.textContent).toBe("install Cthuwu");

    elements.action.click();
    elements.action.click();
    await vi.waitFor(() => expect(deferred.prompt).toHaveBeenCalledTimes(1));
    expect(elements.card.hidden).toBe(true);
  });

  it("respects an explicit dismissal for seven days", () => {
    const firstElements = mountElements();
    controller = initializePwaInstallPrompt(firstElements, {
      now: () => NOW,
      navigator: androidNavigator(),
    });
    window.dispatchEvent(installEvent().event);
    firstElements.dismiss.click();
    expect(firstElements.card.hidden).toBe(true);
    controller.dispose();

    const secondElements = mountElements();
    sessionStorage.clear();
    controller = initializePwaInstallPrompt(secondElements, {
      now: () => NOW + 6 * 24 * 60 * 60 * 1000,
      navigator: androidNavigator(),
    });
    const second = installEvent();
    window.dispatchEvent(second.event);
    expect(second.event.defaultPrevented).toBe(true);
    expect(secondElements.card.hidden).toBe(true);
    expect(second.prompt).not.toHaveBeenCalled();
  });

  it("lets the permanent settings action reopen install UI despite automatic cooldown", () => {
    localStorage.setItem("cthuwu.pwa.install-dismissed.v1", String(NOW));
    const elements = mountElements();
    const returnFocus = document.createElement("button");
    document.body.append(returnFocus);
    const onMenuRequested = vi.fn();
    controller = initializePwaInstallPrompt(elements, {
      now: () => NOW + 1,
      navigator: androidNavigator(),
      onMenuRequested: () => {
        onMenuRequested();
        return returnFocus;
      },
    });
    window.dispatchEvent(installEvent().event);
    expect(elements.card.hidden).toBe(true);
    elements.menuAction?.click();
    expect(onMenuRequested).toHaveBeenCalledTimes(1);
    expect(elements.card.hidden).toBe(false);
    expect(elements.card.dataset.mode).toBe("native");
    expect(document.activeElement).toBe(elements.action);
    elements.dismiss.click();
    expect(document.activeElement).toBe(returnFocus);
  });

  it("does not automatically show the Chromium card on desktop", () => {
    const elements = mountElements();
    controller = initializePwaInstallPrompt(elements, {
      navigator: {
        userAgent: "Mozilla/5.0 Macintosh AppleWebKit/537.36 Chrome/140 Safari/537.36",
        maxTouchPoints: 0,
      } as Navigator,
    });
    window.dispatchEvent(installEvent().event);
    expect(elements.card.hidden).toBe(true);
  });

  it("never nudges or prompts from an installed standalone window", () => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({ matches: true })),
    });
    const elements = mountElements();
    controller = initializePwaInstallPrompt(elements);
    const deferred = installEvent();
    window.dispatchEvent(deferred.event);
    expect(elements.card.hidden).toBe(true);
    expect(elements.menuAction?.hidden).toBe(true);
    expect(deferred.prompt).not.toHaveBeenCalled();
  });

  it("hides a pending nudge when installation completes elsewhere", () => {
    const elements = mountElements();
    controller = initializePwaInstallPrompt(elements, { navigator: androidNavigator() });
    window.dispatchEvent(installEvent().event);
    expect(elements.card.hidden).toBe(false);
    window.dispatchEvent(new Event("appinstalled"));
    expect(elements.card.hidden).toBe(true);
    elements.action.click();
  });

  it("guides Safari users through backup before Apple's manual install flow", () => {
    vi.useFakeTimers();
    const elements = mountElements();
    const onBackupRequested = vi.fn();
    const safari = {
      userAgent:
        "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1",
      maxTouchPoints: 5,
    } as Navigator;
    controller = initializePwaInstallPrompt(elements, {
      navigator: safari,
      applePromptDelayMs: 100,
      onBackupRequested,
    });

    vi.advanceTimersByTime(100);

    expect(elements.card.hidden).toBe(false);
    expect(elements.card.dataset.mode).toBe("apple");
    expect(elements.copy.textContent).toContain("separate local storage");
    expect(elements.copy.textContent).toContain("Add to Home Screen");
    expect(elements.action.textContent).toBe("back up first");
    elements.action.click();
    expect(onBackupRequested).toHaveBeenCalledTimes(1);
  });

  it("tells iOS browsers that installation must be completed in Safari", () => {
    vi.useFakeTimers();
    const elements = mountElements();
    const chromeIos = {
      userAgent:
        "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 CriOS/140 Mobile/15E148 Safari/604.1",
      maxTouchPoints: 5,
    } as Navigator;
    controller = initializePwaInstallPrompt(elements, {
      navigator: chromeIos,
      applePromptDelayMs: 100,
    });
    vi.advanceTimersByTime(100);
    expect(elements.copy.textContent).toContain("open this site in Safari");
    expect(elements.copy.textContent).toContain("Add to Home Screen");
  });

  it("does not mistake desktop Chromium's Safari compatibility token for Safari", () => {
    vi.useFakeTimers();
    const elements = mountElements();
    const chromium = {
      userAgent:
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/140.0.0.0 Safari/537.36",
      maxTouchPoints: 0,
    } as Navigator;
    controller = initializePwaInstallPrompt(elements, {
      navigator: chromium,
      applePromptDelayMs: 100,
    });

    vi.advanceTimersByTime(100);

    expect(elements.card.hidden).toBe(true);
    expect(elements.card.dataset.mode).toBeUndefined();
  });

  it("contains service-worker registration failures", async () => {
    const elements = mountElements();
    const register = vi.fn().mockRejectedValue(new Error("registration blocked"));
    const navigatorWithServiceWorker = {
      userAgent: "Mozilla/5.0 Chrome/140 Safari/537.36",
      maxTouchPoints: 0,
      serviceWorker: { register },
    } as unknown as Navigator;
    controller = initializePwaInstallPrompt(elements, {
      navigator: navigatorWithServiceWorker,
      secureContext: true,
    });
    await Promise.resolve();
    expect(register).toHaveBeenCalledWith("/sw.js", { scope: "/", updateViaCache: "none" });
    expect(elements.card.hidden).toBe(true);
  });

  it("does not register a service worker outside a secure context", () => {
    const elements = mountElements();
    const register = vi.fn();
    const navigatorWithServiceWorker = androidNavigator({ serviceWorker: { register } });
    controller = initializePwaInstallPrompt(elements, {
      navigator: navigatorWithServiceWorker,
      secureContext: false,
    });
    expect(register).not.toHaveBeenCalled();
  });

  it("offers a controlled reload when a waiting service worker is available", async () => {
    const elements = mountElements();
    const updateElements = mountUpdateElements();
    const waiting = { postMessage: vi.fn() };
    const registration = Object.assign(new EventTarget(), {
      waiting,
      installing: null,
    }) as unknown as ServiceWorkerRegistration;
    const serviceWorker = Object.assign(new EventTarget(), {
      register: vi.fn().mockResolvedValue(registration),
      controller: {},
    }) as unknown as ServiceWorkerContainer;
    const reload = vi.fn();
    controller = initializePwaInstallPrompt(elements, {
      navigator: androidNavigator({ serviceWorker }),
      secureContext: true,
      updateElements,
      reload,
    });

    await vi.waitFor(() => expect(updateElements.card.hidden).toBe(false));
    updateElements.action.click();
    expect(waiting.postMessage).toHaveBeenCalledWith({ type: "SKIP_WAITING" });
    serviceWorker.dispatchEvent(new Event("controllerchange"));
    expect(reload).toHaveBeenCalledTimes(1);
    updateElements.dismiss.click();
    expect(updateElements.card.hidden).toBe(true);
  });

  it("detects an update that was already installing when registration resolved", async () => {
    vi.useFakeTimers();
    const elements = mountElements();
    const updateElements = mountUpdateElements();
    const waiting = { postMessage: vi.fn() };
    const installing = Object.assign(new EventTarget(), { state: "installing" });
    const registration = Object.assign(new EventTarget(), {
      waiting: null,
      installing,
    }) as unknown as ServiceWorkerRegistration;
    const serviceWorker = Object.assign(new EventTarget(), {
      register: vi.fn().mockResolvedValue(registration),
      controller: {},
    }) as unknown as ServiceWorkerContainer;
    controller = initializePwaInstallPrompt(elements, {
      navigator: androidNavigator({ serviceWorker }),
      secureContext: true,
      updateElements,
    });
    await Promise.resolve();

    Object.assign(installing, { state: "installed" });
    Object.assign(registration, { waiting });
    installing.dispatchEvent(new Event("statechange"));
    vi.runOnlyPendingTimers();

    expect(updateElements.card.hidden).toBe(false);
  });
});

describe("PWA assets", () => {
  it("declares a stable standalone manifest with correctly sized icons", () => {
    const publicDirectory = resolve(process.cwd(), "public");
    const manifest = JSON.parse(
      readFileSync(resolve(publicDirectory, "manifest.webmanifest"), "utf8"),
    ) as {
      id: string;
      name: string;
      short_name: string;
      description: string;
      start_url: string;
      scope: string;
      display: string;
      display_override: string[];
      theme_color: string;
      background_color: string;
      prefer_related_applications: boolean;
      icons: Array<{ src: string; sizes: string; purpose: string }>;
    };

    expect(manifest.id).toBe("/");
    expect(manifest.name).toBe("Cthuwu — Tentacle Portal");
    expect(manifest.short_name).toBe("Cthuwu");
    expect(manifest.description).toContain("Tentacles that compose Cthuwu");
    expect(manifest.start_url).toBe("/");
    expect(manifest.scope).toBe("/");
    expect(manifest.display).toBe("standalone");
    expect(manifest.display_override).toContain("standalone");
    expect(manifest.theme_color).toBe("#0b0714");
    expect(manifest.background_color).toBe("#0b0714");
    expect(manifest.prefer_related_applications).toBe(false);
    expect(manifest.icons.map(({ sizes, purpose }) => ({ sizes, purpose }))).toEqual([
      { sizes: "192x192", purpose: "any" },
      { sizes: "512x512", purpose: "any" },
      { sizes: "512x512", purpose: "maskable" },
    ]);

    for (const icon of manifest.icons) {
      const bytes = readFileSync(resolve(publicDirectory, icon.src.slice(1)));
      expect(bytes.subarray(1, 4).toString()).toBe("PNG");
      const [expectedWidth, expectedHeight] = icon.sizes.split("x").map(Number);
      expect(bytes.readUInt32BE(16)).toBe(expectedWidth);
      expect(bytes.readUInt32BE(20)).toBe(expectedHeight);
    }

    for (const [asset, expectedSize] of [
      ["icons/apple-touch-icon.png", 180],
      ["icons/favicon-32.png", 32],
    ] as const) {
      const bytes = readFileSync(resolve(publicDirectory, asset));
      expect(bytes.subarray(1, 4).toString()).toBe("PNG");
      expect(bytes.readUInt32BE(16)).toBe(expectedSize);
      expect(bytes.readUInt32BE(20)).toBe(expectedSize);
    }
  });

  it("keeps the offline worker narrowly scoped and never caches GraphQL or XMTP", () => {
    const worker = readFileSync(resolve(process.cwd(), "public/sw.js"), "utf8");
    expect(worker).toContain('"/offline-leaderboard.js"');
    expect(worker).toContain('request.mode === "navigate"');
    expect(worker).toContain("url.origin !== self.location.origin");
    expect(worker).not.toContain("xmtp");
    expect(worker).not.toContain("/assets/");
    expect(worker).not.toContain("graphql");
    expect(worker).toContain("SKIP_WAITING");
    const offline = readFileSync(resolve(process.cwd(), "public/offline.html"), "utf8");
    expect(offline).toContain('rel="manifest"');
    expect(offline).toContain('src="/offline-leaderboard.js"');
  });
});
