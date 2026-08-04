import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  initializePwaInstallPrompt,
  type PwaInstallController,
  type PwaInstallElements,
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
    </aside>`;
  return {
    card: document.querySelector<HTMLElement>("#card")!,
    title: document.querySelector<HTMLElement>("#title")!,
    copy: document.querySelector<HTMLElement>("#copy")!,
    action: document.querySelector<HTMLButtonElement>("#action")!,
    dismiss: document.querySelector<HTMLButtonElement>("#dismiss")!,
  };
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
  it("captures the Chromium prompt and invokes it exactly once from the install button", async () => {
    const elements = mountElements();
    controller = initializePwaInstallPrompt(elements, { now: () => NOW });
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

  it("respects an explicit dismissal for thirty days", () => {
    const firstElements = mountElements();
    controller = initializePwaInstallPrompt(firstElements, { now: () => NOW });
    window.dispatchEvent(installEvent().event);
    firstElements.dismiss.click();
    expect(firstElements.card.hidden).toBe(true);
    controller.dispose();

    const secondElements = mountElements();
    controller = initializePwaInstallPrompt(secondElements, { now: () => NOW + 1 });
    const second = installEvent();
    window.dispatchEvent(second.event);
    expect(second.event.defaultPrevented).toBe(true);
    expect(secondElements.card.hidden).toBe(true);
    expect(second.prompt).not.toHaveBeenCalled();
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
    expect(deferred.prompt).not.toHaveBeenCalled();
  });

  it("hides a pending nudge when installation completes elsewhere", () => {
    const elements = mountElements();
    controller = initializePwaInstallPrompt(elements);
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
    });
    await Promise.resolve();
    expect(register).toHaveBeenCalledWith("/sw.js", { scope: "/" });
    expect(elements.card.hidden).toBe(true);
  });
});

describe("PWA assets", () => {
  it("declares a stable standalone manifest with correctly sized icons", () => {
    const publicDirectory = resolve(process.cwd(), "public");
    const manifest = JSON.parse(
      readFileSync(resolve(publicDirectory, "manifest.webmanifest"), "utf8"),
    ) as {
      id: string;
      start_url: string;
      scope: string;
      display: string;
      theme_color: string;
      background_color: string;
      icons: Array<{ src: string; sizes: string; purpose: string }>;
    };

    expect(manifest.id).toBe("/");
    expect(manifest.start_url).toBe("/");
    expect(manifest.scope).toBe("/");
    expect(manifest.display).toBe("standalone");
    expect(manifest.theme_color).toBe("#0b0714");
    expect(manifest.background_color).toBe("#0b0714");
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
  });

  it("keeps the offline worker narrowly scoped to navigation and two public assets", () => {
    const worker = readFileSync(resolve(process.cwd(), "public/sw.js"), "utf8");
    expect(worker).toContain('const OFFLINE_ASSETS = [OFFLINE_PAGE, "/icons/cthuwu-192.png"]');
    expect(worker).toContain('request.mode === "navigate"');
    expect(worker).toContain("url.origin !== self.location.origin");
    expect(worker).not.toContain("xmtp");
    expect(worker).not.toContain("/assets/");
  });
});
