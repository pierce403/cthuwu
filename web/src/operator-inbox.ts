import { Client, ConsentState, Dm, IdentifierKind, SortDirection, type AsyncStreamProxy, type DecodedMessage } from "@xmtp/browser-sdk";
import { getAddress } from "ethers";
import { verifyPeerInboxState } from "./chat/xmtp-workspace";
import { RETENTION_FROM_NS, RETENTION_IN_NS } from "./chat/types";

export interface OperatorThread {
  id: string;
  wallet: string;
  peerInbox: string;
  label: string;
  saved: boolean;
  unread: number;
  messages: Map<string, DecodedMessage<unknown>>;
  draft: string;
}

/** One SDK database/client, all DMs. A saved contact is never evidence of operator authority. */
export class OperatorInbox {
  readonly threads = new Map<string, OperatorThread>();
  selected?: string;
  connected = false;
  private streams: AsyncStreamProxy<unknown>[] = [];
  private conversations = new Map<string, Dm<unknown>>();
  private opening = new Map<string, Promise<OperatorThread>>();
  private closed = false;
  private resuming?: Promise<void>;
  private generation = 0;
  private retryTimer?: ReturnType<typeof setTimeout>;
  private retryDelay = 1000;
  private referralAcks = new Map<string, { since: bigint; resolve: () => void }>();
  private preferenceKey: string;
  private preferences: Record<string, { label?: string; saved?: boolean; read?: string; draft?: string }> = {};

  constructor(readonly client: Client<unknown>, private storage: Storage, private changed: () => void, private release: () => Promise<void>, namespace: string) {
    this.preferenceKey = `cthuwu.operator-inbox.v1:${namespace}:${client.inboxId}`;
    try { this.preferences = JSON.parse(storage.getItem(this.preferenceKey) ?? "{}"); } catch { /* Recover metadata only. */ }
    if (!this.preferences || typeof this.preferences !== "object" || Array.isArray(this.preferences)) this.preferences = {};
  }

  async resume(): Promise<void> {
    if (this.closed) return;
    if (this.resuming) return this.resuming;
    clearTimeout(this.retryTimer); this.retryTimer = undefined;
    this.resuming = this.synchronize().catch(error => { this.retry(); throw error; }).finally(() => { this.resuming = undefined; });
    return this.resuming;
  }

  private async synchronize(): Promise<void> {
    const generation = ++this.generation;
    let failed = false;
    await Promise.all(this.streams.splice(0).map(stream => stream.return()));
    const failure = () => {
      if (generation !== this.generation || this.closed) return;
      failed = true; this.retry();
    };
    const stream = await this.client.conversations.streamAllMessages({ onFail: failure });
    this.streams.push(stream as AsyncStreamProxy<unknown>);
    const deleted = await this.client.conversations.streamDeletedMessages({ onError: failure, onEnd: failure });
    this.streams.push(deleted as AsyncStreamProxy<unknown>);
    void (async () => {
      try { for await (const message of stream) {
        if (this.closed || generation !== this.generation) break;
        await this.receive(message);
      }} finally { failure(); }
    })().catch(failure);
    void (async () => {
      try { for await (const message of deleted) {
        if (this.closed || generation !== this.generation) break;
        for (const thread of this.threads.values()) { thread.messages.delete(message.id); this.ordered(thread); }
        this.changed();
      }} finally { failure(); }
    })().catch(failure);
    // Establish streams before catch-up so messages arriving during synchronization are retained.
    await this.client.conversations.syncAll();
    const dms = await this.client.conversations.listDms({ limit: 200n });
    for (const dm of dms) {
      if (this.closed || generation !== this.generation) return;
      try {
        const thread = await this.thread(dm);
        await this.history(thread.id);
      } catch { /* Non-EVM/ambiguous/blocked conversations are not Tentacle routes. */ }
    }
    if (!failed) { this.connected = true; this.retryDelay = 1000; }
    this.changed();
  }

  private retry(): void {
    if (this.closed) return;
    this.connected = false; this.changed();
    if (this.retryTimer) return;
    this.retryTimer = setTimeout(() => { this.retryTimer = undefined; void this.resume().catch(() => {}); }, this.retryDelay);
    this.retryDelay = Math.min(this.retryDelay * 2, 30000);
  }

  async add(wallet: string): Promise<string> {
    wallet = getAddress(wallet).toLowerCase();
    if (/^0x0{40}$/.test(wallet)) throw new Error("Tentacle wallet must be nonzero");
    const dm = await this.client.conversations.createDmWithIdentifier({ identifier: wallet, identifierKind: IdentifierKind.Ethereum });
    const thread = await this.thread(dm);
    await verifyPeerInboxState(this.client, thread.peerInbox, wallet);
    await dm.updateConsentState(ConsentState.Allowed);
    thread.saved = true;
    this.persist(thread);
    await this.history(thread.id);
    this.select(thread.id);
    return thread.id;
  }

  select(id: string): void {
    const thread = this.threads.get(id);
    if (!thread) return;
    this.selected = id;
    thread.unread = 0;
    this.persist(thread, true);
    this.changed();
  }

  label(id: string, label: string): void {
    const thread = this.threads.get(id);
    if (!thread) return;
    thread.label = label.trim().slice(0, 80) || thread.wallet;
    thread.saved = true; this.persist(thread); this.changed();
  }

  draft(id: string, value: string): void {
    const thread = this.threads.get(id);
    if (!thread) return;
    thread.draft = value.slice(0, 16384);
    // Keep drafts in memory, including credential provisioning text. Never write them to localStorage.
  }

  async send(id: string, text: string): Promise<void> {
    const thread = this.threads.get(id);
    const dm = this.conversations.get(id);
    if (!thread || !dm || !text.trim() || new TextEncoder().encode(text).length > 16384) throw new Error("Choose a conversation and send at most 16 KiB of text.");
    await verifyPeerInboxState(this.client, thread.peerInbox, thread.wallet);
    await dm.updateConsentState(ConsentState.Allowed);
    const settings = await dm.messageDisappearingSettings();
    if (settings?.fromNs !== RETENTION_FROM_NS || settings?.inNs !== RETENTION_IN_NS) {
      await dm.updateMessageDisappearingSettings(RETENTION_FROM_NS, RETENTION_IN_NS);
    }
    await dm.sendText(text);
    if (thread.draft === text) thread.draft = "";
    await this.history(id);
  }

  async registerReferral(id: string): Promise<void> {
    const thread = this.threads.get(id);
    const dm = this.conversations.get(id);
    if (!thread || !dm) throw new Error("Select a Tentacle first");
    await verifyPeerInboxState(this.client, thread.peerInbox, thread.wallet);
    await dm.updateConsentState(ConsentState.Allowed);
    if (this.referralAcks.has(id)) throw new Error("Referral activation is already pending");
    let timer: ReturnType<typeof setTimeout> | undefined;
    const acknowledgement = new Promise<void>((resolve, reject) => {
      this.referralAcks.set(id, { since: BigInt(Date.now()) * 1_000_000n, resolve });
      timer = setTimeout(() => reject(new Error("The Tentacle did not confirm operator referral activation. Send it a message to check your authority, then retry.")), 30000);
    });
    try {
      await Promise.all([dm.sendText("[[cthuwu:growth-operator-register:v1]]"), acknowledgement]);
    } finally { clearTimeout(timer); this.referralAcks.delete(id); }
  }

  async history(id: string, earlier = false): Promise<void> {
    const dm = this.conversations.get(id);
    const thread = this.threads.get(id);
    if (!dm || !thread) return;
    const ordered = this.ordered(thread);
    const rows = await dm.messages({ limit: 80n, direction: SortDirection.Descending, ...(earlier && ordered[0] ? { sentBeforeNs: ordered[0].sentAtNs } : {}) });
    for (const message of rows) this.insert(thread, message);
    this.changed();
  }

  ordered(thread: OperatorThread): DecodedMessage<unknown>[] {
    const expired = BigInt(Date.now()) * 1_000_000n - RETENTION_IN_NS;
    for (const [id, message] of thread.messages) if (message.sentAtNs < expired) thread.messages.delete(id);
    const seen = this.preferences[thread.id]?.read;
    const readAt = typeof seen === "string" && /^\d+$/.test(seen) ? BigInt(seen) : 0n;
    thread.unread = this.selected === thread.id ? 0 : [...thread.messages.values()].filter(message => message.senderInboxId !== this.client.inboxId && message.sentAtNs > readAt).length;
    return [...thread.messages.values()].sort((a, b) => a.sentAtNs < b.sentAtNs ? -1 : a.sentAtNs > b.sentAtNs ? 1 : a.id.localeCompare(b.id));
  }

  private async thread(dm: Dm<unknown>): Promise<OperatorThread> {
    const known = this.threads.get(dm.id);
    if (known) return known;
    const pending = this.opening.get(dm.id);
    if (pending) return pending;
    const promise = this.discover(dm).finally(() => this.opening.delete(dm.id));
    this.opening.set(dm.id, promise);
    return promise;
  }

  private async discover(dm: Dm<unknown>): Promise<OperatorThread> {
    if (await dm.consentState() === ConsentState.Denied) throw new Error("Blocked contact");
    const peerInbox = await dm.peerInboxId();
    const states = await this.client.preferences.fetchInboxStates([peerInbox]);
    const identifiers = states[0]?.accountIdentifiers.filter(item => item.identifierKind === IdentifierKind.Ethereum) ?? [];
    if (states[0]?.inboxId !== peerInbox || identifiers.length !== 1) throw new Error("No unique EVM peer");
    const wallet = getAddress(identifiers[0]!.identifier).toLowerCase();
    const preferences = this.preferences[dm.id] ?? {};
    const thread: OperatorThread = { id: dm.id, wallet, peerInbox, label: typeof preferences.label === "string" ? preferences.label.slice(0, 80) : wallet, saved: preferences.saved === true, messages: new Map(), unread: 0, draft: "" };
    this.conversations.set(dm.id, dm); this.threads.set(dm.id, thread); this.changed();
    return thread;
  }

  async receive(message: DecodedMessage<unknown>): Promise<void> {
    let thread = this.threads.get(message.conversationId);
    if (!thread) {
      const dm = await this.client.conversations.getConversationById(message.conversationId);
      if (!(dm instanceof Dm)) return;
      try { thread = await this.thread(dm); } catch { return; }
    }
    this.insert(thread, message); this.changed();
  }

  private insert(thread: OperatorThread, message: DecodedMessage<unknown>): void {
    if (message.conversationId !== thread.id || typeof message.content !== "string" || message.contentType?.typeId !== "text" || (message.senderInboxId !== thread.peerInbox && message.senderInboxId !== this.client.inboxId) || thread.messages.has(message.id)) return;
    if (message.content === "[[cthuwu:growth-operator-ack:v1]]" && message.senderInboxId === thread.peerInbox) {
      const pending = this.referralAcks.get(thread.id);
      if (pending && message.sentAtNs >= pending.since) pending.resolve();
    }
    if (message.content.startsWith("[[cthuwu:")) return;
    if (message.sentAtNs < BigInt(Date.now()) * 1_000_000n - RETENTION_IN_NS) return;
    thread.messages.set(message.id, message);
    if (thread.messages.size > 1000) thread.messages.delete(this.ordered(thread)[0]!.id);
    this.ordered(thread);
    if (this.selected === thread.id) this.persist(thread, true);
  }

  private persist(thread: OperatorThread, read = false): void {
    const previous = this.preferences[thread.id];
    this.preferences[thread.id] = { label: thread.label, saved: thread.saved, read: read ? (this.ordered(thread).at(-1)?.sentAtNs.toString() ?? previous?.read) : previous?.read };
    try { this.storage.setItem(this.preferenceKey, JSON.stringify(this.preferences)); } catch { /* Metadata storage failure must not lose the live DM. */ }
  }

  async close(): Promise<void> {
    this.closed = true; ++this.generation;
    clearTimeout(this.retryTimer);
    await Promise.all(this.streams.splice(0).map(stream => stream.return()));
    this.client.close(); await this.release();
  }
}
