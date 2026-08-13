import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppConfig } from "../config";
import type { StoredIdentity } from "../identity";
import { initializeChatController } from "./controller";
import type { ChatChannel, ChatWorkspace, WorkspaceSnapshot } from "./types";

const html = readFileSync(resolve(process.cwd(), "index.html"), "utf8");
const config: AppConfig = {
  environment: "production",
  botAddress: "0x4c4e26a4683f6b6d63e19abf0bedd1171b9a6e90",
  baseRpcEndpoint: "https://mainnet.base.org/",
  assignmentRefreshMs: 600_000,
};
const identity = {
  version: 1,
  environment: "production",
  address: "0x1111111111111111111111111111111111111111",
  walletPrivateKey: `0x${"12".repeat(32)}`,
  compatibilityDbKey: `0x${"34".repeat(32)}`,
  createdAt: "2026-08-11T00:00:00.000Z",
} satisfies StoredIdentity;

function mount(): void {
  const parsed = new DOMParser().parseFromString(html, "text/html");
  document.body.innerHTML = parsed.body.innerHTML;
}

function initialSnapshot(): WorkspaceSnapshot {
  const channel = (id: ChatChannel) => ({
    channel: id,
    status: id === "direct" ? "ready" as const : "empty" as const,
    messages: [],
    unread: id === "acolytes" ? 3 : 0,
    hasMore: id === "direct",
    retentionVerified: true,
    readConversationIds: [`${id}-conversation`],
    writeConversationId: `${id}-conversation`,
  });
  return {
    inboxId: "a".repeat(64),
    activeChannel: "direct",
    connected: true,
    assignmentState: "intro-unconfigured",
    assignmentNotice: "Branding routing pending deployment",
    tentacleName: "Intro Tentacle",
    channels: {
      direct: {
        ...channel("direct"),
        messages: [{
          id: "m1",
          conversationId: "direct-conversation",
          senderInboxId: "b".repeat(64),
          sentAtNs: 1_700_000_000_000_000_000n,
          contentType: "xmtp.org/text:1.0",
          text: '<img src=x onerror="boom">',
          mine: false,
        }],
      },
      acolytes: channel("acolytes"),
      global: channel("global"),
    },
  };
}

function fakeWorkspace(start = initialSnapshot()): ChatWorkspace & { emit(): void } {
  let snapshot = start;
  const listeners = new Set<(value: WorkspaceSnapshot) => void>();
  const workspace = {
    inboxId: start.inboxId,
    snapshot: () => snapshot,
    subscribe: vi.fn((listener: (value: WorkspaceSnapshot) => void) => {
      listeners.add(listener);
      listener(snapshot);
      return () => listeners.delete(listener);
    }),
    setActiveChannel: vi.fn((channel: ChatChannel) => {
      snapshot = { ...snapshot, activeChannel: channel };
      workspace.emit();
    }),
    setViewport: vi.fn(),
    savedScrollTop: vi.fn((channel: ChatChannel) => channel === "acolytes" ? 88 : 0),
    loadEarlier: vi.fn(async () => undefined),
    send: vi.fn(async () => undefined),
    revalidateAssignment: vi.fn(async () => undefined),
    close: vi.fn(async () => undefined),
    emit: () => listeners.forEach((listener) => listener(snapshot)),
  } satisfies ChatWorkspace & { emit(): void };
  return workspace;
}

describe("three-channel chat controller", () => {
  beforeEach(() => {
    mount();
    localStorage.clear();
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    Object.defineProperty(HTMLElement.prototype, "scrollTo", {
      configurable: true,
      value: vi.fn(),
    });
  });

  it("renders text safely and sends through the exact active tab", async () => {
    const workspace = fakeWorkspace();
    const controller = initializeChatController(config, identity, {
      createWorkspace: vi.fn(async () => workspace),
    });
    await controller.connect();

    expect(document.querySelector(".message img")).toBeNull();
    expect(document.querySelector(".message")?.textContent).toContain("<img src=x");
    expect(document.querySelector("#unread-acolytes")?.textContent).toBe("3");
    expect(document.querySelector<HTMLElement>("#unread-acolytes")?.hidden).toBe(false);
    expect(document.querySelector("#retention-notice")?.textContent).toContain("after 14 days");

    document.querySelector<HTMLButtonElement>("#tab-direct")?.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
    );
    expect(workspace.setActiveChannel).toHaveBeenCalledWith("acolytes");
    expect(document.activeElement?.id).toBe("tab-acolytes");
    expect(document.querySelector("#tab-acolytes")?.getAttribute("aria-selected")).toBe("true");
    expect(document.querySelector("#channel-panel")?.getAttribute("aria-labelledby")).toBe("tab-acolytes");

    const input = document.querySelector<HTMLTextAreaElement>("#message")!;
    input.value = "hello group";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector<HTMLFormElement>("#composer")?.requestSubmit();
    await vi.waitFor(() => expect(workspace.send).toHaveBeenCalledWith("acolytes", "hello group"));
    expect(input.value).toBe("");

    document.querySelector<HTMLButtonElement>("#tab-direct")?.click();
    document.querySelector<HTMLButtonElement>("#load-earlier")?.click();
    await vi.waitFor(() => expect(workspace.loadEarlier).toHaveBeenCalledWith("direct"));
  });

  it("restores per-tab scroll and revalidates on resume", async () => {
    const workspace = fakeWorkspace();
    const controller = initializeChatController(config, identity, {
      createWorkspace: vi.fn(async () => workspace),
    });
    await controller.connect();
    document.querySelector<HTMLButtonElement>("#tab-acolytes")?.click();
    expect(document.querySelector<HTMLDivElement>("#messages")?.scrollTop).toBe(88);
    await controller.resume();
    expect(workspace.revalidateAssignment).toHaveBeenCalledWith("resume");
    await controller.close();
    expect(workspace.close).toHaveBeenCalledOnce();
  });

  it("keeps existing message nodes stable across every composer keystroke", async () => {
    const workspace = fakeWorkspace();
    const controller = initializeChatController(config, identity, {
      createWorkspace: vi.fn(async () => workspace),
    });
    await controller.connect();

    const messages = document.querySelector<HTMLDivElement>("#messages")!;
    const bubble = messages.querySelector<HTMLElement>(".message")!;
    const input = document.querySelector<HTMLTextAreaElement>("#message")!;
    const send = document.querySelector<HTMLButtonElement>('#composer button[type="submit"]')!;
    const mutations: MutationRecord[] = [];
    const observer = new MutationObserver((records) => mutations.push(...records));
    observer.observe(messages, { childList: true, subtree: true });

    for (const value of ["h", "he", "hel", "hell", "hello"]) {
      input.value = value;
      input.dispatchEvent(new Event("input", { bubbles: true }));
      expect(messages.querySelector(".message")).toBe(bubble);
      expect(send.disabled).toBe(false);
    }
    await Promise.resolve();
    observer.disconnect();

    expect(mutations).toEqual([]);
    expect(messages.querySelector(".message")).toBe(bubble);
    await controller.close();
  });

  it("renders reward state and presents a one-time Branding decision modal", async () => {
    const referrer = "0x2222222222222222222222222222222222222222";
    const start = initialSnapshot();
    start.channels.direct.messages[0] = {
      ...start.channels.direct.messages[0]!,
      text: "thanks for sharing\n[[cthuwu:reward:v1;status=pending;amount=8]]\n[[cthuwu:branding-offer:v1;treasury=0x3e8;price=0x64;upkeep=0x1]]",
    };
    const workspace = fakeWorkspace(start);
    const controller = initializeChatController({ ...config, referrer }, identity, {
      createWorkspace: vi.fn(async () => workspace),
    });
    await controller.connect();

    expect(document.querySelector(".message")?.textContent).toContain("thanks for sharing");
    expect(document.querySelector(".message")?.textContent).not.toContain("cthuwu:reward");
    expect(document.querySelector("#reward-status")?.textContent).toContain("8 UWU reward queued");
    expect(document.querySelector("#branding-offer")?.hasAttribute("open")).toBe(true);
    expect(document.querySelector("#branding-price")?.textContent).toBe("100 base units");
    expect(document.querySelector("#branding-referrer")?.textContent).toBe(referrer);

    document.querySelector<HTMLButtonElement>("#branding-accept")?.click();
    await vi.waitFor(() => expect(workspace.send).toHaveBeenCalledWith(
      "direct",
      `I accept the Acolyte Branding offer shown in the Cthuwu app. Use referrer ${referrer} in the exact mint consent.`,
    ));
    expect(document.querySelector("#branding-offer")?.hasAttribute("open")).toBe(false);
    expect(localStorage.getItem("cthuwu:branding-offer:v1:m1")).toBe("accepted");
    await controller.close();
  });
});
