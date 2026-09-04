import { beforeEach, describe, expect, it, vi } from "vitest";
import { Client, ConsentState, Dm, IdentifierKind, type DecodedMessage } from "@xmtp/browser-sdk";
import { OperatorInbox } from "./operator-inbox";
const owner = "a".repeat(64);
const walletA = "0x" + "1".repeat(40);
const walletB = "0x" + "2".repeat(40);
function dm(id: string, peer: string) {
  return Object.assign(Object.create(Dm.prototype), {
    // SDK properties are getters; define the test transport explicitly below.
  }) as Dm<unknown>;
}
function idleStream() {
  let pending: ((value: IteratorResult<DecodedMessage<unknown>>) => void) | undefined;
  return {
    [Symbol.asyncIterator]() { return this; },
    next() { return new Promise<IteratorResult<DecodedMessage<unknown>>>(resolve => { pending = resolve; }); },
    async return() { pending?.({ done: true, value: undefined }); return { done: true as const, value: undefined }; },
    push(value: DecodedMessage<unknown>) { pending?.({ done: false, value }); },
  };
}
function fixture() {
  const make = (id: string, peer: string) => {
    const value = dm(id, peer);
    Object.defineProperties(value, {
      id: { value: id }, peerInboxId: { value: vi.fn(async () => peer) },
      consentState: { value: vi.fn(async () => ConsentState.Allowed) }, updateConsentState: { value: vi.fn(async () => {}) },
      messages: { value: vi.fn(async () => []) }, sendText: { value: vi.fn(async () => "sent") },
      messageDisappearingSettings: { value: vi.fn(async () => undefined) }, updateMessageDisappearingSettings: { value: vi.fn(async () => {}) },
    }); return value;
  };
  const a = make("one", "b".repeat(64)); const b = make("two", "c".repeat(64));
  const streams: ReturnType<typeof idleStream>[] = [];
  const client = {
    inboxId: owner, close: vi.fn(),
    preferences: { fetchInboxStates: vi.fn(async ([peer]: string[]) => [{ inboxId: peer, accountIdentifiers: [{ identifierKind: IdentifierKind.Ethereum, identifier: peer === "b".repeat(64) ? walletA : walletB }] }]) },
    conversations: { syncAll: vi.fn(async () => {}), listDms: vi.fn(async () => [a, b]), streamAllMessages: vi.fn(async () => { const stream = idleStream(); streams.push(stream); return stream; }), streamDeletedMessages: vi.fn(async () => idleStream()), createDmWithIdentifier: vi.fn(async ({ identifier }: { identifier: string }) => identifier === walletA ? a : b), getConversationById: vi.fn(async (id: string) => id === "one" ? a : b) },
  } as unknown as Client<unknown>;
  const inbox = new OperatorInbox(client, localStorage, vi.fn(), async () => {}, "production");
  const message = (id: string, conversationId = "one", content = "hello") => ({ id, conversationId, senderInboxId: conversationId === "one" ? "b".repeat(64) : "c".repeat(64), sentAtNs: BigInt(Date.now()) * 1000000n, sentAt: new Date(), contentType: { typeId: "text" }, content }) as DecodedMessage<unknown>;
  return { a, b, inbox, client, message, streams };
}
describe("multi-Tentacle inbox", () => {
  beforeEach(() => localStorage.clear());
  it("routes messages and drafts independently and deduplicates catch-up", async () => {
    const { inbox, a, b, message } = fixture();
    await inbox.add(walletA); inbox.draft("one", "draft one");
    await inbox.add(walletB); inbox.draft("two", "draft two");
    await inbox.receive(message("m1")); await inbox.receive(message("m1"));
    expect(inbox.threads.get("one")?.unread).toBe(1);
    expect(inbox.selected).toBe("two"); expect(inbox.threads.get("two")?.draft).toBe("draft two");
    await inbox.send("two", "draft two");
    expect(b.sendText).toHaveBeenCalledWith("draft two"); expect(a.sendText).not.toHaveBeenCalled();
    inbox.select("one"); expect(inbox.threads.get("one")?.unread).toBe(0); expect(inbox.threads.get("one")?.draft).toBe("draft one");
    expect(localStorage.getItem(`cthuwu.operator-inbox.v1:production:${owner}`)).not.toContain("draft");
  });
  it("discovers unsolicited DMs without granting a saved or operator relationship", async () => {
    const { inbox, message } = fixture();
    await inbox.receive(message("new", "two"));
    expect(inbox.threads.get("two")?.saved).toBe(false);
    expect(inbox.threads.get("two")?.unread).toBe(1);
    expect(inbox.selected).toBeUndefined();
  });
  it("checks peer binding again before sending and rejects forged sender envelopes", async () => {
    const { inbox, client, message, a } = fixture();
    await inbox.add(walletA);
    await inbox.receive({ ...message("spoof"), senderInboxId: "d".repeat(64) } as DecodedMessage<unknown>);
    expect(inbox.threads.get("one")?.messages.size).toBe(0);
    vi.mocked(client.preferences.fetchInboxStates).mockResolvedValue([]);
    await expect(inbox.send("one", "secret command")).rejects.toThrow();
    expect(a.sendText).not.toHaveBeenCalled();
  });
  it("recovers stream/catch-up overlap and reconnects without losing another draft", async () => {
    const { inbox, b, client, message, streams } = fixture();
    await inbox.resume(); inbox.select("one"); inbox.draft("one", "keep this draft");
    const incoming = message("overlap", "two");
    streams[0]!.push(incoming);
    await vi.waitFor(() => expect(inbox.threads.get("two")?.unread).toBe(1));
    vi.mocked(b.messages).mockResolvedValue([incoming]);
    await inbox.resume();
    expect(inbox.threads.get("two")?.messages.size).toBe(1);
    expect(inbox.threads.get("two")?.unread).toBe(1);
    expect(inbox.threads.get("one")?.draft).toBe("keep this draft");
    await streams[1]!.return();
    await vi.waitFor(() => expect(inbox.connected).toBe(false));
    await vi.waitFor(() => expect(client.conversations.syncAll).toHaveBeenCalledTimes(3), { timeout: 2500 });
    expect(inbox.connected).toBe(true);
    expect(inbox.selected).toBe("one");
    await inbox.close();
  });
  it("requires a fresh authenticated referral acknowledgement", async () => {
    const { inbox, message, a } = fixture(); await inbox.add(walletA);
    const registered = inbox.registerReferral("one");
    await vi.waitFor(() => expect(a.sendText).toHaveBeenCalledWith("[[cthuwu:growth-operator-register:v1]]"));
    await inbox.receive(message("ack", "one", "[[cthuwu:growth-operator-ack:v1]]"));
    await registered;
    expect(inbox.threads.get("one")?.messages.size).toBe(0);
  });
});
