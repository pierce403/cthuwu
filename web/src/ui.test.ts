import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

const chat = vi.hoisted(() => ({
  connect: vi.fn(async () => undefined),
  resume: vi.fn(async () => undefined),
  close: vi.fn(async () => undefined),
  initialize: vi.fn(),
}));

const balances = vi.hoisted(() => ({
  fetch: vi.fn(async () => ({
    blockNumber: 123n,
    formattedEth: "1.25",
    formattedUwu: "2,500",
    level: "3.40",
  })),
}));

vi.mock("./chat/controller", () => ({
  initializeChatController: chat.initialize.mockReturnValue(chat),
}));

vi.mock("./account-balances", () => ({ fetchAccountBalances: balances.fetch }));

const html = readFileSync(resolve(process.cwd(), "index.html"), "utf8");

function mountMarkup(): void {
  const parsed = new DOMParser().parseFromString(html, "text/html");
  document.head.innerHTML = parsed.head.innerHTML;
  document.body.innerHTML = parsed.body.innerHTML;
}

describe("application shell", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    chat.initialize.mockReturnValue(chat);
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    localStorage.clear();
    mountMarkup();
    const dialog = document.querySelector<HTMLDialogElement>("#identity-dialog");
    if (dialog) dialog.showModal = vi.fn(() => dialog.setAttribute("open", ""));
  });

  it("loads a same-block Base balance snapshot only when identity settings open", async () => {
    await import("./main");
    expect(balances.fetch).not.toHaveBeenCalled();
    document.querySelector<HTMLButtonElement>("#settings")?.click();
    await vi.waitFor(() => expect(balances.fetch).toHaveBeenCalledTimes(1));
    expect(document.querySelector("#identity-eth-balance")?.textContent).toBe("1.25 ETH");
    expect(document.querySelector("#identity-uwu-balance")?.textContent).toBe("2,500 UWU");
    expect(document.querySelector("#identity-level")?.textContent).toBe("3.40");
    expect(document.querySelector("#identity-balance-state")?.textContent).toContain("123");
  });

  it("preserves accessible chat, PWA, identity, and route navigation landmarks", () => {
    const log = document.querySelector<HTMLElement>("#messages");
    const message = document.querySelector<HTMLTextAreaElement>("#message");
    const tabs = [...document.querySelectorAll<HTMLElement>('[role="tab"]')];
    expect(document.querySelector("main")).not.toBeNull();
    expect(log?.getAttribute("role")).toBe("log");
    expect(log?.tabIndex).toBe(0);
    expect(message?.getAttribute("aria-describedby")).toContain("composer-error");
    expect(message?.disabled).toBe(true);
    expect(document.querySelector('[role="tablist"]')?.getAttribute("aria-label")).toBe("Cthuwu channels");
    expect(tabs.map((tab) => tab.textContent?.trim())).toEqual(["Direct0", "Acolytes0", "Global0"]);
    expect(tabs.map((tab) => tab.getAttribute("aria-selected"))).toEqual(["true", "false", "false"]);
    expect(document.querySelector("#channel-panel")?.getAttribute("role")).toBe("tabpanel");
    expect(document.querySelector("#status")?.getAttribute("role")).toBe("status");
    expect(document.querySelector("#composer-error")?.getAttribute("role")).toBe("alert");
    expect(document.querySelector("#settings")?.getAttribute("aria-label")).toContain("identity");
    expect(document.querySelector<HTMLImageElement>(".mascot")?.alt).toBe("");
    expect(document.querySelector("#identity-dialog")?.textContent).toContain("unencrypted");
    expect(document.querySelector('link[rel="manifest"]')?.getAttribute("href")).toBe("/manifest.webmanifest");
    const csp = document.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute("content");
    expect(csp).toContain("img-src 'self' data:");
    expect(csp).not.toMatch(/img-src[^;]*https:/u);
    expect(document.querySelector("#tentacles")).toBeNull();
    expect(document.querySelector<HTMLAnchorElement>('a[href="/tentacles/"]')).not.toBeNull();
    expect(document.querySelector<HTMLAnchorElement>('a[href="/acolytes/"]')).not.toBeNull();
    expect(document.querySelector<HTMLAnchorElement>('a[href="/operator/"]')).not.toBeNull();
    expect(document.querySelector<HTMLAnchorElement>('a[href="https://github.com/pierce403/cthuwu"]')?.textContent).toContain("GitHub");
    expect(document.querySelector("#identity-dialog")?.textContent).toContain("Base balances");
    expect(document.querySelector("#identity-dialog")?.textContent).toContain("export encrypted key");
    expect(document.querySelector("#identity-level")).not.toBeNull();
    expect(document.querySelector(".intro")?.textContent).toContain("three trusted XMTP channels");
  });

  it("keeps main as lifecycle coordination for chat, identity, balances, and PWA", async () => {
    await import("./main");
    await vi.waitFor(() => expect(chat.connect).toHaveBeenCalledWith(false));
    expect(chat.initialize).toHaveBeenCalledWith(
      expect.objectContaining({
        environment: "production",
        botAddress: "0x4c4e26a4683f6b6d63e19abf0bedd1171b9a6e90",
        assignmentRefreshMs: 600_000,
      }),
      expect.objectContaining({ environment: "production" }),
    );
    expect(document.querySelector("#tentacles")).toBeNull();

    window.dispatchEvent(new PageTransitionEvent("pagehide", { persisted: true }));
    expect(chat.close).not.toHaveBeenCalled();
    window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true }));
    await vi.waitFor(() => expect(chat.resume).toHaveBeenCalled());
    window.dispatchEvent(new PageTransitionEvent("pagehide"));
    await vi.waitFor(() => expect(chat.close).toHaveBeenCalled());
  });
});
