import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

const chat = vi.hoisted(() => ({
  connect: vi.fn(async () => undefined),
  resume: vi.fn(async () => undefined),
  close: vi.fn(async () => undefined),
  initialize: vi.fn(),
}));

vi.mock("./chat/controller", () => ({
  initializeChatController: chat.initialize.mockReturnValue(chat),
}));

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
  });

  it("preserves accessible chat, PWA, identity, and leaderboard landmarks", () => {
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
    expect(document.querySelector("#tentacles")?.getAttribute("aria-labelledby")).toBe("leaderboard-title");
    expect(document.querySelector("#leaderboard-list")?.getAttribute("role")).toBe("list");
    const ontology = document.querySelector("#tentacles")?.textContent?.replace(/\s+/gu, " ");
    expect(ontology).toContain("one human operator");
    expect(ontology).toContain("singular, centerless Cthuwu");
    expect(document.querySelector(".intro")?.textContent).toContain("three trusted XMTP channels");
  });

  it("keeps main as lifecycle coordination for chat, identity, leaderboard, and PWA", async () => {
    await import("./main");
    await vi.waitFor(() => expect(chat.connect).toHaveBeenCalledWith(false));
    expect(chat.initialize).toHaveBeenCalledWith(
      expect.objectContaining({
        environment: "production",
        botAddress: "0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db",
        assignmentRefreshMs: 600_000,
      }),
      expect.objectContaining({ environment: "production" }),
    );
    expect(document.querySelector("#tentacles")).not.toBeNull();

    window.dispatchEvent(new PageTransitionEvent("pagehide", { persisted: true }));
    expect(chat.close).not.toHaveBeenCalled();
    window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true }));
    await vi.waitFor(() => expect(chat.resume).toHaveBeenCalled());
    window.dispatchEvent(new PageTransitionEvent("pagehide"));
    await vi.waitFor(() => expect(chat.close).toHaveBeenCalled());
  });
});
