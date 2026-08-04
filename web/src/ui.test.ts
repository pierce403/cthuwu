import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatMessage, ChatSession } from "./transport";

const transport = vi.hoisted(() => ({
  createSession: vi.fn(),
  onMessage: undefined as ((message: ChatMessage) => void) | undefined,
  onError: undefined as ((error: unknown) => void) | undefined,
}));

vi.mock("./xmtp-transport", () => ({
  createXmtpSession: transport.createSession,
}));

const html = readFileSync(resolve(process.cwd(), "index.html"), "utf8");

function mountMarkup(): void {
  const parsed = new DOMParser().parseFromString(html, "text/html");
  document.head.innerHTML = parsed.head.innerHTML;
  document.body.innerHTML = parsed.body.innerHTML;
}

describe("chat interface", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    vi.stubEnv("VITE_XMTP_ENV", "dev");
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    localStorage.clear();
    transport.onMessage = undefined;
    transport.onError = undefined;
    mountMarkup();
  });

  it("keeps the responsive chat and identity controls semantically labeled", () => {
    const log = document.querySelector<HTMLElement>("#messages");
    const message = document.querySelector<HTMLTextAreaElement>("#message");
    const mascot = document.querySelector<HTMLImageElement>(".mascot");
    const close = document.querySelector<HTMLButtonElement>("#identity-close");
    const dialogForm = document.querySelector<HTMLFormElement>("#identity-dialog form");

    expect(document.querySelector("main")).not.toBeNull();
    expect(log?.getAttribute("role")).toBe("log");
    expect(log?.getAttribute("aria-label")).toBe("Conversation with Cthuwu");
    expect(log?.tabIndex).toBe(0);
    expect(message?.getAttribute("aria-describedby")).toContain("composer-error");
    expect(message?.disabled).toBe(true);
    expect(document.querySelector("#status")?.getAttribute("role")).toBe("status");
    expect(document.querySelector("#composer-error")?.getAttribute("role")).toBe("alert");
    expect(document.querySelector("#settings")?.getAttribute("aria-label")).toContain("identity");
    expect(mascot?.alt).toBe("");
    expect(mascot?.width).toBe(1254);
    expect(mascot?.height).toBe(1254);
    expect(close?.type).toBe("button");
    expect(dialogForm?.hasAttribute("method")).toBe(false);
    expect(document.querySelector("#identity-dialog")?.textContent).toContain("unencrypted");
    expect(document.querySelector("#motion-toggle")?.getAttribute("aria-pressed")).toBe("false");
    expect(document.querySelector('meta[property="og:image"]')?.getAttribute("content")).toBe(
      "https://cthuwu.app/cthuwu-og.jpg",
    );
    expect(document.querySelector('meta[property="og:image:width"]')?.getAttribute("content")).toBe(
      "1200",
    );
    expect(document.querySelector('meta[name="twitter:card"]')?.getAttribute("content")).toBe(
      "summary_large_image",
    );
  });

  it("connects with an empty welcome, safely renders text, sends once, and disables on stream loss", async () => {
    let releaseStop: (() => void) | undefined;
    const stop = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          releaseStop = resolve;
        }),
    );
    const close = vi.fn(async () => undefined);
    const send = vi.fn(async () => undefined);
    const fakeSession: ChatSession = {
      inboxId: "inbox_1234567890abcdefghijklmnop",
      history: vi.fn(async () => []),
      stream: vi.fn(async (onMessage, onError) => {
        transport.onMessage = onMessage;
        transport.onError = onError;
        return stop;
      }),
      send,
      close,
    };
    transport.createSession.mockResolvedValue(fakeSession);

    await import("./main");
    await vi.waitFor(() => expect(document.querySelector("#chat")?.getAttribute("data-state")).toBe("connected"));
    expect(transport.createSession).toHaveBeenCalledWith(
      {
        environment: "dev",
        botAddress: "0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db",
      },
      expect.any(Object),
    );

    const input = document.querySelector<HTMLTextAreaElement>("#message");
    const sendButton = document.querySelector<HTMLButtonElement>("#send");
    expect(input?.disabled).toBe(false);
    expect(sendButton?.disabled).toBe(true);
    expect(document.querySelector("[data-welcome]")?.textContent).toContain("what’s on your mind?");

    transport.onMessage?.({ id: "hostile", text: '<img src=x onerror="boom">', mine: false });
    expect(document.querySelector(".message img")).toBeNull();
    expect(document.querySelector(".message")?.textContent).toContain("<img src=x");
    expect(document.querySelector("[data-welcome]")).toBeNull();
    expect(document.querySelector("#mascot-stage")?.classList.contains("is-delighted")).toBe(true);

    if (!input) throw new Error("missing composer");
    input.value = "hello from the browser";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    expect(sendButton?.disabled).toBe(false);
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await vi.waitFor(() => expect(send).toHaveBeenCalledTimes(1));
    expect(send).toHaveBeenCalledWith("hello from the browser");
    expect(input.value).toBe("");

    const motion = document.querySelector<HTMLButtonElement>("#motion-toggle");
    motion?.click();
    expect(document.documentElement.dataset.motion).toBe("paused");
    expect(motion?.getAttribute("aria-pressed")).toBe("true");

    transport.onError?.(new Error("stream ended"));
    await vi.waitFor(() =>
      expect(document.querySelector("#chat")?.getAttribute("data-state")).toBe("retryable-error"),
    );
    expect(input.disabled).toBe(true);
    expect(document.querySelector<HTMLButtonElement>("#connect")?.hidden).toBe(false);
    expect(stop).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();
    releaseStop?.();
    await vi.waitFor(() => expect(close).toHaveBeenCalledTimes(1));
  });
});
