import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppConfig } from "../config";
import { CANONICAL_BRANDING_CONTRACT } from "../config";
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
    typing: false,
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
    assignedTentacleAddress: config.botAddress,
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
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn(async () => undefined) },
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

    document.querySelector<HTMLButtonElement>("#copy-referral")?.click();
    await vi.waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      `${location.origin}/#t=${config.botAddress}&r=${identity.address}`,
    ));
    expect(document.querySelector("#referral-status")?.textContent).toBe("referral link copied");

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

  it("starts a fresh workspace when the user retries a completed liveness race", async () => {
    const failedSnapshot = {
      ...initialSnapshot(),
      assignmentState: "liveness-unavailable" as const,
      assignmentNotice: "No ranked Tentacle answered the one-time liveness check",
    };
    const first = fakeWorkspace(failedSnapshot);
    const second = fakeWorkspace();
    const createWorkspace = vi.fn()
      .mockResolvedValueOnce(first)
      .mockResolvedValueOnce(second);
    const controller = initializeChatController(config, identity, { createWorkspace });
    await controller.connect();

    document.querySelector<HTMLButtonElement>("#connect")?.click();
    await vi.waitFor(() => expect(createWorkspace).toHaveBeenCalledTimes(2));
    expect(first.revalidateAssignment).not.toHaveBeenCalled();
    expect(first.close).toHaveBeenCalledOnce();
    await controller.close();
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
    const dialog = document.querySelector<HTMLDialogElement>("#branding-offer")!;
    const showModal = vi.fn(() => dialog.setAttribute("open", ""));
    Object.defineProperty(dialog, "showModal", {
      configurable: true,
      value: showModal,
    });
    const start = initialSnapshot();
    const name = "Ainsworth-Clavering of Ambercroft";
    const nameHex = `0x${Array.from(new TextEncoder().encode(name), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
    const offerId = "12".repeat(16);
    const blockHash = `0x${"a".repeat(64)}`;
    const offerMarker = `[[cthuwu:branding-offer:v2;offer=${offerId};contract=${CANONICAL_BRANDING_CONTRACT};minter=${config.botAddress};agent=42;acolyte=${identity.address};referrer=${referrer};treasury=1000;basis=1000;price=100;upkeep=1;nonce=0;deadline=2000000000;block=123;blockHash=${blockHash};name=${nameHex}]]`;
    const consentMarker = `[[cthuwu:branding-consent:v2;offer=${offerId};contract=${CANONICAL_BRANDING_CONTRACT};minter=${config.botAddress};agent=42;acolyte=${identity.address};referrer=${referrer};price=100;nonce=0;deadline=2000000000;block=123;blockHash=${blockHash};name=${nameHex};signature=0x11]]`;
    start.channels.direct.messages[0] = {
      ...start.channels.direct.messages[0]!,
      text: `thanks for sharing\n[[cthuwu:reward:v1;status=pending;amount=8]]\n${offerMarker}`,
    };
    const workspace = fakeWorkspace(start);
    const offer = (await import("../branding-consent")).parseBrandingMessage(offerMarker, "theirs").control;
    if (!offer || offer.type !== "offer") throw new Error("fixture offer did not parse");
    const review = {
      offer,
      digest: `0x${"b".repeat(64)}`,
      domain: { name: "Cthuwu Acolyte Branding", version: "1", chainId: 8453, verifyingContract: CANONICAL_BRANDING_CONTRACT },
    } as const;
    const consent = (await import("../branding-consent")).parseBrandingMessage(consentMarker, "mine").control;
    if (!consent || consent.type !== "consent") throw new Error("fixture consent did not parse");
    const controller = initializeChatController({ ...config, brandingContract: CANONICAL_BRANDING_CONTRACT, referrer }, identity, {
      createWorkspace: vi.fn(async () => workspace),
      reviewBrandingOffer: vi.fn(async () => review),
      signBrandingOffer: vi.fn(async () => consent),
      nowSeconds: () => 1_900_000_000n,
    });
    await controller.connect();

    expect(document.querySelector(".message")?.textContent).toContain("thanks for sharing");
    expect(document.querySelector(".message")?.textContent).not.toContain("cthuwu:reward");
    expect(document.querySelector("#reward-status")?.textContent).toContain("8 UWU reward queued");
    expect(document.querySelector("#branding-offer")?.hasAttribute("open")).toBe(true);
    expect(showModal).toHaveBeenCalledTimes(1);
    expect(document.querySelector("#branding-price")?.textContent).toContain("100 base units");
    expect(document.querySelector("#branding-referrer")?.textContent).toBe(referrer);
    const brandingReview = document.querySelector<HTMLButtonElement>("#branding-review")!;
    expect(brandingReview.hidden).toBe(false);
    expect(brandingReview.textContent).toContain("Branding offer");
    dialog.removeAttribute("open");
    brandingReview.click();
    expect(showModal).toHaveBeenCalledTimes(2);
    expect(dialog.hasAttribute("open")).toBe(true);

    document.querySelector<HTMLButtonElement>("#branding-accept")?.click();
    await vi.waitFor(() => expect(document.querySelector("#branding-accept")?.textContent).toBe("sign exact consent"));
    expect(workspace.send).not.toHaveBeenCalled();
    document.querySelector<HTMLButtonElement>("#branding-accept")?.click();
    await vi.waitFor(() => expect(workspace.send).toHaveBeenCalledWith("direct", consentMarker));
    expect(document.querySelector("#branding-name")?.textContent).toBe(name);
    expect(document.querySelector("#branding-offer")?.hasAttribute("open")).toBe(true);
    expect(localStorage.getItem(`cthuwu:branding-offer:v2:${offerId}`)).toBeNull();
    await controller.close();
  });

  it("retries the same receipt after a transient Base verification failure", async () => {
    const start = initialSnapshot();
    const referrer = "0x2222222222222222222222222222222222222222";
    const name = "Ainsworth-Clavering of Ambercroft";
    const nameHex = `0x${Array.from(new TextEncoder().encode(name), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
    const offerId = "34".repeat(16);
    const offerBlockHash = `0x${"a".repeat(64)}`;
    const receiptBlockHash = `0x${"b".repeat(64)}`;
    const offerMarker = `[[cthuwu:branding-offer:v2;offer=${offerId};contract=${CANONICAL_BRANDING_CONTRACT};minter=${config.botAddress};agent=42;acolyte=${identity.address};referrer=${referrer};treasury=1000;basis=1000;price=100;upkeep=1;nonce=0;deadline=2000000000;block=123;blockHash=${offerBlockHash};name=${nameHex}]]`;
    const receiptMarker = `[[cthuwu:branding-receipt:v2;offer=${offerId};contract=${CANONICAL_BRANDING_CONTRACT};token=${BigInt(identity.address)};agent=42;acolyte=${identity.address};owner=${config.botAddress};referrer=${referrer};price=100;nonce=0;block=130;blockHash=${receiptBlockHash};name=${nameHex}]]`;
    start.channels.direct.messages = [
      {
        ...start.channels.direct.messages[0]!,
        id: "offer",
        text: offerMarker,
      },
      {
        ...start.channels.direct.messages[0]!,
        id: "receipt",
        sentAtNs: start.channels.direct.messages[0]!.sentAtNs + 1n,
        text: receiptMarker,
      },
    ];
    const workspace = fakeWorkspace(start);
    const verifyReceipt = vi.fn()
      .mockRejectedValueOnce(new Error("Base RPC unavailable"))
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce(new Error("receipt no longer matches the offer"));
    const controller = initializeChatController(
      { ...config, brandingContract: CANONICAL_BRANDING_CONTRACT },
      identity,
      {
        createWorkspace: vi.fn(async () => workspace),
        verifyBrandingReceipt: verifyReceipt,
      },
    );
    await controller.connect();

    await vi.waitFor(() => {
      expect(document.querySelector("#branding-status")?.textContent).toContain(
        "Receipt verification failed",
      );
    });
    const accept = document.querySelector<HTMLButtonElement>("#branding-accept")!;
    expect(accept.textContent).toBe("retry receipt verification");

    accept.click();
    await vi.waitFor(() => expect(verifyReceipt).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => {
      expect(document.querySelector("#branding-status")?.textContent).toContain(
        "Routing eligibility is checked separately",
      );
    });
    expect(accept.textContent).toBe("done");

    const changedOfferMarker = offerMarker.replace(
      "treasury=1000;basis=1000;price=100;upkeep=1",
      "treasury=2000;basis=1000;price=200;upkeep=1",
    );
    start.channels.direct.messages.push(
      {
        ...start.channels.direct.messages[0]!,
        id: "changed-offer",
        sentAtNs: start.channels.direct.messages[1]!.sentAtNs + 1n,
        text: changedOfferMarker,
      },
      {
        ...start.channels.direct.messages[1]!,
        id: "old-receipt-again",
        sentAtNs: start.channels.direct.messages[1]!.sentAtNs + 2n,
      },
    );
    workspace.emit();
    await vi.waitFor(() => expect(verifyReceipt).toHaveBeenCalledTimes(3));
    await vi.waitFor(() => {
      expect(document.querySelector("#branding-status")?.textContent).toContain(
        "receipt no longer matches the offer",
      );
    });
    await controller.close();
  });

  it("renders privileged transcripts literally on the operator surface", async () => {
    const start = initialSnapshot();
    start.activeChannel = "global";
    start.channels.direct.messages[0] = {
      ...start.channels.direct.messages[0]!,
      text: "tool output [[cthuwu:reward:v1;status=confirmed;amount=8]] [[cthuwu:branding-offer:v1;treasury=0x3e8;price=0x64;upkeep=0x1]]",
    };
    const workspace = fakeWorkspace(start);
    const controller = initializeChatController(config, identity, {
      surface: "operator",
      brandingOffers: false,
      createWorkspace: vi.fn(async () => workspace),
    });
    await controller.connect();

    expect(document.querySelector(".message")?.textContent).toContain("[[cthuwu:reward:v1");
    expect(document.querySelector(".message .sender")?.textContent).toContain("Target Tentacle");
    expect(document.querySelector("#chat-name")?.textContent).toContain("Target Tentacle");
    expect(document.querySelector("#status")?.textContent).toContain("Verified direct XMTP route");
    expect(document.querySelector("#status")?.textContent).not.toContain("Branding routing");
    expect(document.querySelector("#branding-offer")?.hasAttribute("open")).toBe(false);
    expect(document.querySelector<HTMLElement>("#activity-card")?.hidden).toBe(true);
    expect(document.querySelector<HTMLButtonElement>("#branding-review")?.hidden).toBe(true);
    expect(workspace.setActiveChannel).toHaveBeenCalledWith("direct");

    document.querySelector<HTMLButtonElement>("#tab-direct")?.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
    );
    document.querySelector<HTMLButtonElement>("#tab-acolytes")?.click();
    expect(workspace.setActiveChannel).not.toHaveBeenCalledWith("acolytes");
    expect(workspace.setActiveChannel).not.toHaveBeenCalledWith("global");
    await controller.close();
  });

  it("does not call a disconnected or failed operator route verified", async () => {
    const start = initialSnapshot();
    start.connected = false;
    start.channels.direct.status = "error";
    start.channels.direct.error = "Direct history verification failed";
    const workspace = fakeWorkspace(start);
    const controller = initializeChatController(config, identity, {
      surface: "operator",
      brandingOffers: false,
      createWorkspace: vi.fn(async () => workspace),
    });
    await controller.connect();

    expect(document.querySelector("#status")?.textContent).toContain("Direct history verification failed");
    expect(document.querySelector("#status")?.textContent).not.toContain("Verified direct XMTP route");
    await controller.close();
  });
});
