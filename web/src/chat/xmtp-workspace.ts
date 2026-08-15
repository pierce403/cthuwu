import {
  Client,
  ConsentState,
  Dm,
  IdentifierKind,
  SortDirection,
  type AsyncStreamProxy,
  type Conversation,
  type DecodedMessage,
  type Signer,
} from "@xmtp/browser-sdk";
import { Wallet, getBytes } from "ethers";
import type { AppConfig } from "../config";
import type { StoredIdentity } from "../identity";
import {
  RegistryUnavailableError,
  resolveTentacleAssignment,
  type TentacleAssignment,
} from "./assignment";
import {
  CONTROL_CODECS,
  createJoinControl,
  encodeJoinControl,
  isAssignmentContentType,
  isControlContentType,
  isTypingContentType,
  isTypingControl,
  type AssignmentControl,
} from "./control";
import {
  validateAssignedAcolytesGroup,
  validateAssignedGlobalGroups,
  validateAssignedGroups,
  type GroupForValidation,
} from "./group-validation";
import { createChatUiStateStore, type ChatUiStateStore } from "./storage";
import { readTentacleDisplayHint } from "./tentacle-display";
import {
  CHAT_CHANNELS,
  RETENTION_FROM_NS,
  RETENTION_IN_NS,
  type AssignmentState,
  type ChannelSnapshot,
  type ChannelStatus,
  type ChatChannel,
  type ChatWorkspace,
  type WorkspaceMessage,
  type WorkspaceSnapshot,
} from "./types";

const HISTORY_PAGE_SIZE = 40n;
const MAX_MESSAGES_PER_CHANNEL = 1_000;
const MAX_MESSAGE_BYTES = 16 * 1024;

type XmtpStream = AsyncStreamProxy<unknown>;
type XmtpClient = Client<unknown>;

class DirectVerificationUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DirectVerificationUnavailableError";
  }
}

interface MutableChannel extends ChannelSnapshot {
  historyByConversation: Map<string, { beforeNs?: bigint; hasMore: boolean; pageLimit: bigint }>;
}

export interface WorkspaceOptions {
  storage?: Storage;
  resolveAssignment?: typeof resolveTentacleAssignment;
  releaseDatabaseLease?: () => Promise<void>;
}

export async function createXmtpWorkspace(
  config: AppConfig,
  identity: StoredIdentity,
  options: WorkspaceOptions = {},
): Promise<ChatWorkspace> {
  const { client, releaseDatabaseLease } = await openRegisteredClient(config, identity);
  try {
    const workspace = new XmtpMultiChannelWorkspace(client, config, identity, {
      ...options,
      releaseDatabaseLease,
    });
    await workspace.start();
    return workspace;
  } catch (error) {
    client.close();
    await releaseDatabaseLease();
    throw error;
  }
}

/**
 * Ensure the canonical browser/Acolyte address resolves to an XMTP inbox before it is copied into
 * a Tentacle operator command. This opens no conversation and releases the Browser SDK database
 * lease before the operator console opens its direct workspace.
 */
export async function ensureXmtpIdentityRegistration(
  config: AppConfig,
  identity: StoredIdentity,
): Promise<string> {
  const { client, releaseDatabaseLease } = await openRegisteredClient(config, identity);
  try {
    if (!client.inboxId) throw new Error("XMTP client did not return an inbox ID");
    return client.inboxId;
  } finally {
    client.close();
    await releaseDatabaseLease();
  }
}

async function openRegisteredClient(
  config: AppConfig,
  identity: StoredIdentity,
): Promise<{ client: XmtpClient; releaseDatabaseLease: () => Promise<void> }> {
  const wallet = new Wallet(identity.walletPrivateKey);
  const releaseDatabaseLease = await acquireXmtpDatabaseLease(config.environment, identity.address);
  let client: XmtpClient;
  try {
    client = (await recoverRegisteredClient(() =>
      Client.create(createSigner(wallet), {
      env: config.environment,
      appVersion: "cthuwu-web/0.2.0",
      codecs: [...CONTROL_CODECS],
      // Reopen the Browser SDK's persisted installation before asking XMTP whether it is already
      // registered. A normal reload must not consume another inbox installation slot.
      disableAutoRegister: true,
      }),
    )) as XmtpClient;
  } catch (error) {
    await releaseDatabaseLease();
    throw error;
  }
  return { client, releaseDatabaseLease };
}

interface RegistrationClient {
  isRegistered(): Promise<boolean>;
  register(): Promise<void>;
  revokeAllOtherInstallations(): Promise<void>;
  close(): void;
}

export async function recoverRegisteredClient<T extends RegistrationClient>(
  createClient: () => Promise<T>,
): Promise<T> {
  const client = await createClient();
  try {
    if (!(await client.isRegistered())) {
      try {
        await client.register();
      } catch (error) {
        if (!isInstallationLimitError(error)) throw error;
        await client.revokeAllOtherInstallations();
        await client.register();
      }
    }
    return client;
  } catch (error) {
    client.close();
    throw error;
  }
}

function isInstallationLimitError(error: unknown): boolean {
  return error instanceof Error && /already registered 10\/10 installations/iu.test(error.message);
}

export async function acquireXmtpDatabaseLease(
  environment: string,
  address: string,
): Promise<() => Promise<void>> {
  if (!navigator.locks) return async () => undefined;
  let release!: () => void;
  const held = new Promise<void>((resolve) => { release = resolve; });
  let resolveAcquisition!: (acquired: boolean) => void;
  let rejectAcquisition!: (error: unknown) => void;
  const acquired = new Promise<boolean>((resolve, reject) => {
    rejectAcquisition = reject;
    resolveAcquisition = resolve;
  });
  const lease = navigator.locks.request(
    `cthuwu:xmtp-db:v1:${environment}:${address.toLowerCase()}`,
    { ifAvailable: true, mode: "exclusive" },
    async (lock) => {
      resolveAcquisition(Boolean(lock));
      if (lock) await held;
    },
  ).catch((error: unknown) => {
    rejectAcquisition(error);
  });
  if (!(await acquired)) {
    await lease;
    throw new Error("This XMTP identity is already open in another tab. Close the other Cthuwu tab, then retry.");
  }
  let released = false;
  return async () => {
    if (!released) {
      released = true;
      release();
    }
    await lease;
  };
}

export class XmtpMultiChannelWorkspace implements ChatWorkspace {
  readonly inboxId: string;
  private readonly channels: Record<ChatChannel, MutableChannel>;
  private readonly conversations = new Map<string, Conversation<unknown>>();
  private readonly trustedChannelByConversation = new Map<string, ChatChannel>();
  private readonly listeners = new Set<(snapshot: WorkspaceSnapshot) => void>();
  private readonly uiStore: ChatUiStateStore;
  private readonly atBottom: Record<ChatChannel, boolean> = {
    direct: true,
    acolytes: true,
    global: true,
  };
  private readonly streams: XmtpStream[] = [];
  private readonly resolveAssignment: typeof resolveTentacleAssignment;
  private readonly storage: Storage | undefined;
  private assignmentState: AssignmentState = "checking";
  private assignmentNotice = "Checking the canonical assignment…";
  private tentacleName = "Tentacle";
  private connected = false;
  private closed = false;
  private direct?: Dm<unknown>;
  private currentAssignment?: TentacleAssignment;
  private currentTentacleInboxId?: string;
  private pendingRequestId?: string;
  private pendingGroupAssignment?: AssignmentControl;
  private trustedGroupAssignment?: AssignmentControl;
  private revalidation?: Promise<void>;
  private refreshTimer?: ReturnType<typeof setInterval>;
  private restartingStreams?: Promise<void>;
  private needsStreamRestart = false;
  private streamGeneration = 0;
  private registryFailureCount = 0;
  private nextAutomaticRegistryAttemptAt = 0;
  private readonly typingTimers: Partial<Record<ChatChannel, ReturnType<typeof setTimeout>>> = {};

  constructor(
    private readonly client: XmtpClient,
    private readonly config: AppConfig,
    private readonly identity: StoredIdentity,
    options: WorkspaceOptions,
  ) {
    if (!client.inboxId) throw new Error("XMTP client did not return an inbox ID");
    this.inboxId = client.inboxId;
    this.storage = options.storage ?? safeLocalStorage();
    this.uiStore = createChatUiStateStore(this.storage);
    this.resolveAssignment = options.resolveAssignment ?? resolveTentacleAssignment;
    this.releaseDatabaseLease = options.releaseDatabaseLease;
    this.channels = {
      direct: channel("direct", "loading"),
      acolytes: channel("acolytes", "awaiting-assignment"),
      global: channel("global", "awaiting-assignment"),
    };
  }

  private readonly releaseDatabaseLease?: () => Promise<void>;

  async start(): Promise<void> {
    // Start every required stream before sending the join control message, so a fast assignment
    // response or newly delivered group cannot be missed between sync and subscription.
    await this.startStreams();
    this.connected = true;
    await this.revalidateAssignment("connect");
    this.refreshTimer = setInterval(() => {
      void this.revalidateAssignment("periodic").catch((error) => {
        this.assignmentNotice = error instanceof Error ? error.message : "Assignment refresh failed";
        this.emit();
      });
    }, this.config.assignmentRefreshMs);
    this.emit();
  }

  snapshot(): WorkspaceSnapshot {
    return {
      inboxId: this.inboxId,
      activeChannel: this.uiStore.activeChannel,
      connected: this.connected,
      assignmentState: this.assignmentState,
      assignmentNotice: this.assignmentNotice,
      tentacleName: this.tentacleName,
      ...(this.currentAssignment ? { assignedTentacleAddress: this.currentAssignment.address } : {}),
      channels: Object.fromEntries(
        CHAT_CHANNELS.map((id) => {
          const value = this.channels[id];
          return [
            id,
            {
              channel: value.channel,
              status: value.status,
              messages: [...value.messages],
              unread: value.unread,
              hasMore: value.hasMore,
              retentionVerified: value.retentionVerified,
              typing: value.typing,
              readConversationIds: [...value.readConversationIds],
              ...(value.writeConversationId
                ? { writeConversationId: value.writeConversationId }
                : {}),
              ...(value.error ? { error: value.error } : {}),
            },
          ];
        }),
      ) as Record<ChatChannel, ChannelSnapshot>,
    };
  }

  subscribe(listener: (snapshot: WorkspaceSnapshot) => void): () => void {
    this.listeners.add(listener);
    listener(this.snapshot());
    return () => this.listeners.delete(listener);
  }

  setActiveChannel(channelId: ChatChannel): void {
    this.uiStore.setActiveChannel(channelId);
    this.markRead(channelId);
    this.emit();
  }

  setViewport(channelId: ChatChannel, scrollTop: number, atBottom: boolean): void {
    this.atBottom[channelId] = atBottom;
    this.uiStore.setScrollTop(channelId, scrollTop);
    if (atBottom && this.uiStore.activeChannel === channelId) {
      this.markRead(channelId);
      this.emit();
    }
  }

  savedScrollTop(channelId: ChatChannel): number {
    return this.uiStore.scrollTop(channelId);
  }

  async loadEarlier(channelId: ChatChannel): Promise<void> {
    const current = this.channels[channelId];
    if (!current.hasMore || current.readConversationIds.length === 0) return;
    await this.loadHistory(channelId, true);
  }

  async send(channelId: ChatChannel, text: string): Promise<void> {
    const current = this.channels[channelId];
    const trimmed = text.trim();
    if (
      !this.connected ||
      !current.retentionVerified ||
      (current.status !== "ready" && current.status !== "empty") ||
      !current.writeConversationId ||
      !trimmed
    ) {
      throw new Error("This channel is not ready to send");
    }
    if (new TextEncoder().encode(trimmed).length > MAX_MESSAGE_BYTES) {
      throw new Error("Messages must be 16 KiB or smaller");
    }
    const conversation = this.conversations.get(current.writeConversationId);
    if (!conversation || conversation.id !== current.writeConversationId) {
      throw new Error("The exact write conversation is unavailable");
    }
    if (channelId === "direct") {
      try {
        await repairDirectRetention(conversation);
      } catch (error) {
        current.retentionVerified = false;
        this.blockChannel("direct", "Direct chat could not verify its 14-day message policy");
        this.emit();
        throw error;
      }
    } else {
      await this.revalidateBoundGroupChannel(channelId);
    }
    await conversation.sendText(trimmed);
  }

  async revalidateAssignment(
    reason: "connect" | "resume" | "periodic" | "retry",
  ): Promise<void> {
    if (this.revalidation) return this.revalidation;
    if (
      (reason === "resume" || reason === "periodic") &&
      Date.now() < this.nextAutomaticRegistryAttemptAt
    ) return;
    this.revalidation = this.runRevalidation(reason).finally(() => {
      this.revalidation = undefined;
    });
    return this.revalidation;
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.connected = false;
    this.streamGeneration += 1;
    if (this.refreshTimer) clearInterval(this.refreshTimer);
    for (const channelId of CHAT_CHANNELS) this.clearTyping(channelId, false);
    await Promise.all(this.streams.splice(0).map((stream) => stream.return().then(() => undefined)));
    this.client.close();
    await this.releaseDatabaseLease?.();
    this.emit();
  }

  private async runRevalidation(
    reason: "connect" | "resume" | "periodic" | "retry",
  ): Promise<void> {
    if (this.closed) return;
    if (reason !== "periodic") {
      this.assignmentState = "checking";
      this.assignmentNotice = "Checking the canonical assignment…";
      this.emit();
    }
    try {
      if ((this.needsStreamRestart || !this.connected) && (reason === "resume" || reason === "retry")) {
        await this.restartStreams();
      }
      const assignment = await this.resolveAssignment(this.config, this.identity);
      if (assignment.source === "rotation-verified") {
        this.storage?.setItem(
          `cthuwu.rotation.v1:${this.config.environment}:${this.identity.address}`,
          assignment.address,
        );
      }
      this.registryFailureCount = 0;
      this.nextAutomaticRegistryAttemptAt = 0;
      const changed = routeKey(assignment) !== routeKey(this.currentAssignment);
      if (changed || !this.direct) {
        await this.handoffDirect(assignment);
        this.currentAssignment = assignment;
        this.assignmentState = assignment.source;
        this.assignmentNotice = assignment.notice;
        await this.requestAssignment();
      } else {
        try {
          await this.client.conversations.sync();
        } catch (error) {
          const detail = error instanceof Error ? error.message : "unknown XMTP sync failure";
          throw new DirectVerificationUnavailableError(
            `Direct verification is unavailable: conversation sync failed: ${detail}`,
          );
        }
        await this.verifyCurrentDirect(assignment);
        await this.revalidateBoundGroupsIndependently();
        this.currentAssignment = assignment;
        this.assignmentState = assignment.source;
        this.assignmentNotice = assignment.notice;
        this.restoreVerifiedChannels();
        await this.requestAssignment();
      }
      this.emit();
    } catch (error) {
      if (error instanceof RegistryUnavailableError || error instanceof DirectVerificationUnavailableError) {
        if (error instanceof RegistryUnavailableError) {
          this.registryFailureCount += 1;
          const delay = Math.min(15 * 60_000, 30_000 * 2 ** Math.min(5, this.registryFailureCount - 1));
          this.nextAutomaticRegistryAttemptAt = Date.now() + delay;
        }
        this.assignmentState = error instanceof DirectVerificationUnavailableError
          ? "direct-verification-unavailable"
          : "registry-unavailable";
        this.assignmentNotice = error.message;
        // A configured canonical read outage freezes only Branding-dependent routing. Global remains
        // bound to the previously authenticated logical channel and is never reinterpreted as fallback.
        this.blockChannel("direct", error.message);
        this.blockChannel("acolytes", error.message);
        this.channels.direct.retentionVerified = false;
        this.channels.acolytes.retentionVerified = false;
        this.emit();
        return;
      }
      const message = error instanceof Error ? error.message : "XMTP assignment handoff failed";
      this.assignmentNotice = `Assignment retry required: ${message}`;
      if (!this.direct) {
        this.setChannelIssue("direct", "error", message);
        this.setChannelIssue("acolytes", "error", message);
      } else if (!this.trustedGroupAssignment) {
        this.setChannelIssue("acolytes", "error", message);
      }
      this.emit();
      throw error;
    }
  }

  private async handoffDirect(assignment: TentacleAssignment): Promise<void> {
    const priorDirectIds = [...this.channels.direct.readConversationIds];
    const priorAcolytesIds = [...this.channels.acolytes.readConversationIds];
    for (const id of [...priorDirectIds, ...priorAcolytesIds]) {
      this.trustedChannelByConversation.delete(id);
      this.conversations.delete(id);
    }
    this.channels.direct = channel("direct", "loading");
    this.channels.acolytes = channel("acolytes", "awaiting-assignment");
    this.direct = undefined;
    this.currentTentacleInboxId = undefined;
    this.pendingGroupAssignment = undefined;
    this.pendingRequestId = undefined;
    this.emit();

    const disappearing = {
      fromNs: RETENTION_FROM_NS,
      inNs: RETENTION_IN_NS,
    };
    const expectedWallet = isVerifiedTentacle(assignment) ? assignment.wallet : assignment.address;
    if (isVerifiedTentacle(assignment)) {
      await verifyPeerInboxState(this.client, assignment.inboxId, expectedWallet);
    }
    const direct =
      isVerifiedTentacle(assignment)
        ? await this.client.conversations.createDm(assignment.inboxId, {
            messageDisappearingSettings: disappearing,
          })
        : await this.client.conversations.createDmWithIdentifier(
            {
              identifier: assignment.address,
              identifierKind: IdentifierKind.Ethereum,
            },
            { messageDisappearingSettings: disappearing },
          );
    const peerInboxId = await direct.peerInboxId();
    if (isVerifiedTentacle(assignment) && peerInboxId !== assignment.inboxId) {
      throw new RegistryUnavailableError("XMTP did not resolve the canonical controller inbox");
    }
    await verifyPeerInboxState(this.client, peerInboxId, expectedWallet);
    if ((await direct.consentState()) !== ConsentState.Allowed) {
      await direct.updateConsentState(ConsentState.Allowed);
    }
    if ((await direct.consentState()) !== ConsentState.Allowed) {
      throw new RegistryUnavailableError("Direct consent could not be verified as Allowed");
    }
    await repairDirectRetention(direct);
    this.direct = direct;
    this.currentTentacleInboxId = peerInboxId;
    this.conversations.set(direct.id, direct);
    this.trustedChannelByConversation.set(direct.id, "direct");
    this.bindChannel("direct", [direct.id], direct.id, true);
    const hint = readTentacleDisplayHint(
      isVerifiedTentacle(assignment) ? assignment.agentId : undefined,
      this.storage,
    );
    this.tentacleName = hint?.name ?? (isVerifiedTentacle(assignment) ? `Tentacle #${assignment.agentId}` : "Intro Tentacle");
    await this.loadHistory("direct", false);
  }

  private async verifyCurrentDirect(assignment: TentacleAssignment): Promise<void> {
    try {
      if (!this.direct || !this.currentTentacleInboxId) {
        throw new Error("the assigned Direct conversation is unavailable");
      }
      const expectedWallet = isVerifiedTentacle(assignment) ? assignment.wallet : assignment.address;
      const expectedInboxId = isVerifiedTentacle(assignment)
        ? assignment.inboxId
        : this.currentTentacleInboxId;
      const peerInboxId = await this.direct.peerInboxId();
      if (peerInboxId !== this.currentTentacleInboxId || peerInboxId !== expectedInboxId) {
        throw new Error("Direct no longer resolves the assigned Tentacle inbox");
      }
      await verifyPeerInboxState(this.client, peerInboxId, expectedWallet);
      await repairDirectRetention(this.direct);
      if ((await this.direct.consentState()) !== ConsentState.Allowed) {
        await this.direct.updateConsentState(ConsentState.Allowed);
      }
      if ((await this.direct.consentState()) !== ConsentState.Allowed) {
        throw new Error("Direct consent could not be verified as Allowed");
      }
      this.channels.direct.retentionVerified = true;
    } catch (error) {
      const detail = error instanceof Error ? error.message : "unknown XMTP verification failure";
      throw new DirectVerificationUnavailableError(`Direct verification is unavailable: ${detail}`);
    }
  }

  private restoreVerifiedChannels(): void {
    for (const channelId of CHAT_CHANNELS) {
      const current = this.channels[channelId];
      if (current.retentionVerified && current.writeConversationId) {
        current.status = current.messages.length === 0 ? "empty" : "ready";
        delete current.error;
      }
    }
  }

  private async requestAssignment(): Promise<void> {
    if (!this.direct || !this.currentTentacleInboxId) return;
    if (this.currentAssignment?.source === "intro-unconfigured") {
      this.setChannelIssue(
        "acolytes",
        "awaiting-assignment",
        "Branding routing is pending deployment; Acolytes enrollment is not configured",
      );
      this.setChannelIssue(
        "global",
        "awaiting-assignment",
        "Branding routing is pending deployment; Global enrollment is not configured",
      );
      this.emit();
      return;
    }
    const join = createJoinControl();
    this.pendingRequestId = join.requestId;
    await this.direct.send(encodeJoinControl(join), { shouldPush: false });
  }

  private async startStreams(): Promise<void> {
    const generation = ++this.streamGeneration;
    const allMessages = await this.client.conversations.streamAllMessages({
      consentStates: [ConsentState.Allowed],
      onFail: () => this.streamFailed("The XMTP message stream needs a reconnect", generation),
    });
    const newGroups = await this.client.conversations.streamGroups({
      onFail: () => this.streamFailed("The XMTP group stream needs a reconnect", generation),
    });
    const deletedMessages = await this.client.conversations.streamDeletedMessages({
      onError: () => this.streamFailed("The XMTP expiry stream needs a reconnect", generation),
      onEnd: () => this.streamFailed("The XMTP expiry stream needs a reconnect", generation),
    });
    this.streams.push(allMessages as XmtpStream, newGroups as XmtpStream, deletedMessages as XmtpStream);
    void this.consume(allMessages, (message) => this.handleMessage(message), generation);
    void this.consume(newGroups, (group) => this.handleNewGroup(group), generation);
    void this.consume(deletedMessages, (message) => this.handleDeletedMessage(message), generation);
  }

  private async restartStreams(): Promise<void> {
    if (this.restartingStreams) return this.restartingStreams;
    this.restartingStreams = (async () => {
      this.streamGeneration += 1;
      await Promise.all(this.streams.splice(0).map((stream) => stream.return().then(() => undefined)));
      await this.startStreams();
      await this.client.conversations.sync();
      this.needsStreamRestart = false;
      this.connected = true;
      this.emit();
    })().finally(() => {
      this.restartingStreams = undefined;
    });
    return this.restartingStreams;
  }

  private async consume<T>(
    stream: AsyncIterable<T>,
    handler: (value: T) => Promise<void> | void,
    generation: number,
  ): Promise<void> {
    try {
      for await (const value of stream) {
        if (this.closed) break;
        await handler(value);
      }
      if (!this.closed) this.streamFailed("The XMTP stream ended; reconnect to catch up safely", generation);
    } catch {
      if (!this.closed) this.streamFailed("The XMTP stream stopped; reconnect to catch up safely", generation);
    }
  }

  private async handleMessage(message: DecodedMessage<unknown>): Promise<void> {
    const channelId = this.trustedChannelByConversation.get(message.conversationId);
    if (!channelId) return;
    if (isControlContentType(message.contentType)) {
      if (isAssignmentContentType(message.contentType)) await this.handleAssignment(message);
      if (isTypingContentType(message.contentType)) this.handleTyping(message, channelId);
      return;
    }
    const decoded = decodeChatMessage(message, this.inboxId);
    if (!decoded) return;
    this.addMessage(channelId, decoded, true);
    if (!decoded.mine) this.clearTyping(channelId);
  }

  private handleTyping(message: DecodedMessage<unknown>, channelId: ChatChannel): void {
    if (
      !isTypingControl(message.content) ||
      message.senderInboxId === this.inboxId ||
      (channelId === "direct" && message.senderInboxId !== this.currentTentacleInboxId)
    ) return;
    let expiresAtNs: bigint;
    try { expiresAtNs = BigInt(message.content.expiresAtNs); } catch { return; }
    const remainingMs = Number((expiresAtNs - BigInt(Date.now()) * 1_000_000n) / 1_000_000n);
    if (!message.content.active || remainingMs <= 0) {
      this.clearTyping(channelId);
      return;
    }
    this.clearTyping(channelId, false);
    this.channels[channelId].typing = true;
    this.typingTimers[channelId] = setTimeout(() => this.clearTyping(channelId), Math.min(remainingMs, 30_000));
    this.emit();
  }

  private clearTyping(channelId: ChatChannel, emit = true): void {
    const timer = this.typingTimers[channelId];
    if (timer) clearTimeout(timer);
    delete this.typingTimers[channelId];
    if (!this.channels[channelId].typing) return;
    this.channels[channelId].typing = false;
    if (emit) this.emit();
  }

  private async handleAssignment(message: DecodedMessage<unknown>): Promise<void> {
    if (
      !this.direct ||
      message.conversationId !== this.direct.id ||
      message.senderInboxId !== this.currentTentacleInboxId ||
      !this.pendingRequestId ||
      !isAssignmentControl(message.content)
    ) {
      return;
    }
    const assignment = message.content;
    if (
      assignment.requestId !== this.pendingRequestId ||
      assignment.tentacleInboxId !== this.currentTentacleInboxId ||
      !assignment.global.adminInboxIds.includes(assignment.tentacleInboxId)
    ) {
      return;
    }
    if (
      this.currentAssignment && isVerifiedTentacle(this.currentAssignment) &&
      assignment.tentacleAgentId !== this.currentAssignment.agentId
    ) {
      return;
    }
    this.pendingGroupAssignment = assignment;
    await this.tryBindAssignedGroups();
  }

  private async handleNewGroup(group: Conversation<unknown>): Promise<void> {
    const pending = this.pendingGroupAssignment;
    if (!pending) return;
    if (
      group.id !== pending.acolytesGroupId &&
      !pending.global.readConversationIds.includes(group.id)
    ) {
      return;
    }
    await this.tryBindAssignedGroups();
  }

  private handleDeletedMessage(message: DecodedMessage<unknown>): void {
    const channelId = this.trustedChannelByConversation.get(message.conversationId);
    if (!channelId) return;
    let changed = false;
    const current = this.channels[channelId];
    const next = current.messages.filter((item) => item.id !== message.id);
    if (next.length !== current.messages.length) {
      current.messages = next;
      current.status = next.length === 0 && current.retentionVerified ? "empty" : current.status;
      current.unread = next.filter(
        (item) => !item.mine && this.isUnread(channelId, item),
      ).length;
      changed = true;
    }
    if (changed) this.emit();
  }

  private async tryBindAssignedGroups(): Promise<void> {
    const assignment = this.pendingGroupAssignment;
    if (!assignment) return;
    try {
      await this.client.conversations.sync();
      const ids = [assignment.acolytesGroupId, ...assignment.global.readConversationIds];
      const groups = new Map<string, GroupForValidation>();
      for (const id of ids) {
        const conversation = await this.client.conversations.getConversationById(id);
        if (!isGroupConversation(conversation) || conversation.id !== id) {
          throw new Error("assigned group has not arrived yet");
        }
        await conversation.sync();
        groups.set(id, conversation);
      }
      const validated = await validateAssignedGroups(
        assignment,
        this.inboxId,
        groups,
        this.currentAssignment && isVerifiedTentacle(this.currentAssignment)
          ? this.currentAssignment.agentId
          : undefined,
      );
      await ensureAllowedGroupConsent([
        validated.acolytes,
        ...validated.global.values(),
      ]);
      this.channels.acolytes.retentionVerified = true;
      this.channels.global.retentionVerified = true;
      for (const id of this.channels.acolytes.readConversationIds) {
        this.trustedChannelByConversation.delete(id);
        this.conversations.delete(id);
      }
      for (const id of this.channels.global.readConversationIds) {
        this.trustedChannelByConversation.delete(id);
        this.conversations.delete(id);
      }
      this.conversations.set(validated.acolytes.id, validated.acolytes as unknown as Conversation<unknown>);
      this.trustedChannelByConversation.set(validated.acolytes.id, "acolytes");
      for (const [id, group] of validated.global) {
        this.conversations.set(id, group as unknown as Conversation<unknown>);
        this.trustedChannelByConversation.set(id, "global");
      }
      this.bindChannel("acolytes", [assignment.acolytesGroupId], assignment.acolytesGroupId, true);
      this.bindChannel(
        "global",
        assignment.global.readConversationIds,
        assignment.global.writeConversationId,
        true,
      );
      this.channels.global.messages = this.channels.global.messages.filter((message) =>
        assignment.global.readConversationIds.includes(message.conversationId),
      );
      this.pendingGroupAssignment = undefined;
      this.pendingRequestId = undefined;
      this.trustedGroupAssignment = assignment;
      await Promise.all([this.loadHistory("acolytes", false), this.loadHistory("global", false)]);
    } catch (error) {
      const message = error instanceof Error ? error.message : "assigned group validation failed";
      const waiting = message.includes("has not arrived yet") || message.includes("not available");
      this.setChannelIssue("acolytes", waiting ? "awaiting-assignment" : "error", message);
      const retainedGlobal = this.channels.global;
      if (
        !retainedGlobal.retentionVerified ||
        !retainedGlobal.writeConversationId ||
        retainedGlobal.readConversationIds.length === 0
      ) {
        this.setChannelIssue("global", waiting ? "awaiting-assignment" : "error", message);
      }
      this.emit();
    }
  }

  private async revalidateBoundGroupsIndependently(): Promise<void> {
    const assignment = this.trustedGroupAssignment;
    if (!assignment) return;
    await Promise.allSettled([
      this.revalidateBoundGroupChannel("acolytes"),
      this.revalidateBoundGroupChannel("global"),
    ]);
  }

  private async revalidateBoundGroupChannel(
    channelId: "acolytes" | "global",
  ): Promise<void> {
    const assignment = this.trustedGroupAssignment;
    try {
      if (!assignment) throw new Error("trusted group assignment is unavailable");
      const expectedAgentId = this.currentAssignment && isVerifiedTentacle(this.currentAssignment)
        ? this.currentAssignment.agentId
        : undefined;
      if (channelId === "acolytes") {
        const conversation = this.conversations.get(assignment.acolytesGroupId);
        if (!isGroupConversation(conversation) || conversation.id !== assignment.acolytesGroupId) {
          throw new Error("trusted Acolytes group is unavailable");
        }
        await conversation.sync();
        const validated = await validateAssignedAcolytesGroup(
          assignment,
          this.inboxId,
          conversation,
          expectedAgentId,
        );
        await ensureAllowedGroupConsent([validated]);
      } else {
        const groups = new Map<string, GroupForValidation>();
        for (const id of assignment.global.readConversationIds) {
          const conversation = this.conversations.get(id);
          if (!isGroupConversation(conversation) || conversation.id !== id) {
            throw new Error("trusted Global group is unavailable");
          }
          await conversation.sync();
          groups.set(id, conversation);
        }
        const validated = await validateAssignedGlobalGroups(
          assignment,
          this.inboxId,
          groups,
        );
        await ensureAllowedGroupConsent([...validated.values()]);
      }
      this.channels[channelId].retentionVerified = true;
      this.restoreVerifiedChannels();
    } catch (error) {
      const message = error instanceof Error ? error.message : "trusted group validation failed";
      this.channels[channelId].retentionVerified = false;
      this.blockChannel(channelId, message);
      this.emit();
      throw error;
    }
  }

  private async loadHistory(channelId: ChatChannel, earlier: boolean): Promise<void> {
    const current = this.channels[channelId];
    const ids = current.readConversationIds;
    if (ids.length === 0) return;
    if (!earlier) {
      current.messages = [];
      current.historyByConversation.clear();
      for (const id of ids) current.historyByConversation.set(id, { hasMore: true, pageLimit: HISTORY_PAGE_SIZE });
      current.status = "loading";
      this.emit();
    }
    try {
      const batches = await Promise.all(
        ids.map(async (id) => {
          const conversation = this.conversations.get(id);
          if (!conversation || conversation.id !== id) throw new Error("trusted conversation unavailable");
          let cursor = current.historyByConversation.get(id) ?? {
            hasMore: true,
            pageLimit: HISTORY_PAGE_SIZE,
          };
          if (earlier && !cursor.hasMore) return { raw: [] as DecodedMessage<unknown>[] };
          let limit = earlier ? cursor.pageLimit : HISTORY_PAGE_SIZE;
          let raw = await conversation.messages({
            limit,
            direction: SortDirection.Descending,
            ...(earlier && cursor?.beforeNs
              ? { sentBeforeNs: cursor.beforeNs + 1n }
              : {}),
          });
          raw = raw.filter((message) => message.conversationId === id);
          while (
            earlier && cursor.beforeNs && BigInt(raw.length) === limit &&
            raw.every((message) => current.messages.some((known) => known.id === String(message.id))) &&
            limit < BigInt(MAX_MESSAGES_PER_CHANNEL)
          ) {
            limit = limit * 2n > BigInt(MAX_MESSAGES_PER_CHANNEL)
              ? BigInt(MAX_MESSAGES_PER_CHANNEL)
              : limit * 2n;
            raw = await conversation.messages({
              limit,
              direction: SortDirection.Descending,
              sentBeforeNs: cursor.beforeNs + 1n,
            });
            raw = raw.filter((message) => message.conversationId === id);
          }
          const oldest = raw.reduce<bigint | undefined>(
            (value, message) => value === undefined || message.sentAtNs < value ? message.sentAtNs : value,
            undefined,
          );
          current.historyByConversation.set(id, {
            ...(oldest !== undefined ? { beforeNs: oldest } : {}),
            hasMore: BigInt(raw.length) === limit &&
              !(limit === BigInt(MAX_MESSAGES_PER_CHANNEL) &&
                raw.every((message) => current.messages.some((known) => known.id === String(message.id)))),
            pageLimit: oldest !== undefined && cursor.beforeNs !== undefined && oldest < cursor.beforeNs
              ? HISTORY_PAGE_SIZE
              : limit,
          });
          return { raw };
        }),
      );
      const decoded = batches
        .flatMap(({ raw }) => raw)
        .flatMap((message) => {
          if (isControlContentType(message.contentType)) return [];
          const value = decodeChatMessage(message, this.inboxId);
          return value ? [value] : [];
        })
        .sort(compareMessages);
      current.messages = mergeMessages(current.messages, decoded).slice(-MAX_MESSAGES_PER_CHANNEL);
      current.hasMore = [...current.historyByConversation.values()].some(({ hasMore }) => hasMore);
      current.status = current.messages.length === 0 ? "empty" : "ready";
      delete current.error;
      if (this.uiStore.activeChannel === channelId && this.atBottom[channelId]) {
        this.markRead(channelId);
      } else {
        current.unread = current.messages.filter(
          (message) => !message.mine && this.isUnread(channelId, message),
        ).length;
      }
      this.emit();
    } catch (error) {
      this.setChannelIssue(
        channelId,
        "error",
        error instanceof Error ? error.message : "message history is unavailable",
      );
      this.emit();
    }
  }

  private addMessage(channelId: ChatChannel, message: WorkspaceMessage, live: boolean): void {
    const current = this.channels[channelId];
    if (!current.readConversationIds.includes(message.conversationId)) return;
    if (current.messages.some((item) => item.id === message.id)) return;
    current.messages = mergeMessages(current.messages, [message]).slice(-MAX_MESSAGES_PER_CHANNEL);
    current.status = "ready";
    if (
      !message.mine &&
      live &&
      (this.uiStore.activeChannel !== channelId || !this.atBottom[channelId])
    ) {
      current.unread += 1;
    } else if (this.uiStore.activeChannel === channelId) {
      this.markRead(channelId);
    }
    this.emit();
  }

  private bindChannel(
    channelId: ChatChannel,
    readConversationIds: string[],
    writeConversationId: string,
    retentionVerified: boolean,
  ): void {
    const current = this.channels[channelId];
    current.readConversationIds = [...readConversationIds];
    current.writeConversationId = writeConversationId;
    current.retentionVerified = retentionVerified;
    current.historyByConversation = new Map(
      readConversationIds.map((id) => [id, { hasMore: true, pageLimit: HISTORY_PAGE_SIZE }]),
    );
    current.status = "loading";
    delete current.error;
  }

  private blockChannel(channelId: ChatChannel, error: string): void {
    const current = this.channels[channelId];
    current.status = "policy-blocked";
    current.error = error;
  }

  private setChannelIssue(channelId: ChatChannel, status: ChannelStatus, error: string): void {
    const current = this.channels[channelId];
    current.status = status;
    current.error = error;
  }

  private markRead(channelId: ChatChannel): void {
    const current = this.channels[channelId];
    const newest = current.messages.at(-1)?.sentAtNs;
    if (newest) {
      this.uiStore.setReadAt(
        channelId,
        newest,
        current.messages.filter((message) => message.sentAtNs === newest).map((message) => message.id),
      );
    }
    current.unread = 0;
  }

  private isUnread(channelId: ChatChannel, message: WorkspaceMessage): boolean {
    const cursor = this.uiStore.readCursor(channelId);
    return message.sentAtNs > cursor.sentAtNs ||
      (message.sentAtNs === cursor.sentAtNs && !cursor.messageIds.has(message.id));
  }

  private streamFailed(message: string, generation = this.streamGeneration): void {
    if (this.closed || generation !== this.streamGeneration) return;
    this.needsStreamRestart = true;
    this.connected = false;
    for (const channelId of CHAT_CHANNELS) this.blockChannel(channelId, message);
    this.emit();
  }

  private emit(): void {
    const snapshot = this.snapshot();
    for (const listener of this.listeners) listener(snapshot);
  }
}

async function repairDirectRetention(conversation: Conversation<unknown>): Promise<void> {
  let policy = await conversation.messageDisappearingSettings();
  if (policy?.fromNs !== RETENTION_FROM_NS || policy.inNs !== RETENTION_IN_NS) {
    await conversation.updateMessageDisappearingSettings(RETENTION_FROM_NS, RETENTION_IN_NS);
    policy = await conversation.messageDisappearingSettings();
  }
  if (policy?.fromNs !== RETENTION_FROM_NS || policy.inNs !== RETENTION_IN_NS) {
    throw new Error("Direct chat could not verify its 14-day message policy");
  }
}

function decodeChatMessage(
  message: DecodedMessage<unknown>,
  ownInboxId: string,
): WorkspaceMessage | undefined {
  if (message.contentType.typeId !== "text" || typeof message.content !== "string") return undefined;
  if (new TextEncoder().encode(message.content).length > MAX_MESSAGE_BYTES) return undefined;
  return {
    id: String(message.id),
    conversationId: message.conversationId,
    senderInboxId: message.senderInboxId,
    sentAtNs: message.sentAtNs,
    contentType: `${message.contentType.authorityId}/${message.contentType.typeId}:${message.contentType.versionMajor}.${message.contentType.versionMinor}`,
    text: message.content,
    mine: message.senderInboxId === ownInboxId,
  };
}

function isAssignmentControl(value: unknown): value is AssignmentControl {
  return (
    typeof value === "object" &&
    value !== null &&
    "type" in value &&
    value.type === "cthuwu.assignment.v1"
  );
}

function channel(channelId: ChatChannel, status: ChannelStatus): MutableChannel {
  return {
    channel: channelId,
    status,
    messages: [],
    unread: 0,
    hasMore: false,
    retentionVerified: false,
    typing: false,
    readConversationIds: [],
    historyByConversation: new Map(),
  };
}

function mergeMessages(existing: WorkspaceMessage[], incoming: WorkspaceMessage[]): WorkspaceMessage[] {
  const byId = new Map(existing.map((message) => [message.id, message]));
  for (const message of incoming) byId.set(message.id, message);
  return [...byId.values()].sort(compareMessages);
}

function compareMessages(left: WorkspaceMessage, right: WorkspaceMessage): number {
  if (left.sentAtNs !== right.sentAtNs) return left.sentAtNs < right.sentAtNs ? -1 : 1;
  return left.id.localeCompare(right.id);
}

function routeKey(assignment: TentacleAssignment | undefined): string | undefined {
  if (!assignment) return undefined;
  return isVerifiedTentacle(assignment)
    ? `agent:${assignment.agentId}:inbox:${assignment.inboxId}`
    : `address:${assignment.address}`;
}

function isVerifiedTentacle(assignment: TentacleAssignment): assignment is Extract<TentacleAssignment, { source: "branding-active" | "anchor-verified" | "rotation-verified" }> {
  return assignment.source === "branding-active" || assignment.source === "anchor-verified" || assignment.source === "rotation-verified";
}

async function verifyPeerInboxState(
  client: XmtpClient,
  inboxId: string,
  expectedWallet: string,
): Promise<void> {
  const states = await client.preferences.fetchInboxStates([inboxId]);
  const state = states.length === 1 && states[0]?.inboxId === inboxId ? states[0] : undefined;
  const ethereumIdentifiers = state?.accountIdentifiers.filter(
    (identifier) => identifier.identifierKind === IdentifierKind.Ethereum,
  ) ?? [];
  if (
    !state || ethereumIdentifiers.length !== 1 ||
    ethereumIdentifiers[0]?.identifier.toLowerCase() !== expectedWallet.toLowerCase()
  ) {
    throw new RegistryUnavailableError("XMTP inbox state does not bind the expected Tentacle wallet");
  }
}

function createSigner(wallet: Wallet): Signer {
  return {
    type: "EOA",
    getIdentifier: () => ({
      identifier: wallet.address.toLowerCase(),
      identifierKind: IdentifierKind.Ethereum,
    }),
    signMessage: async (message: string) => getBytes(await wallet.signMessage(message)),
  };
}

function safeLocalStorage(): Storage | undefined {
  try {
    return localStorage;
  } catch {
    return undefined;
  }
}

function isGroupConversation(value: unknown): value is Conversation<unknown> & GroupForValidation {
  return Boolean(
    value && typeof value === "object" &&
    "id" in value && typeof value.id === "string" &&
    "members" in value && typeof value.members === "function" &&
    "listAdmins" in value && typeof value.listAdmins === "function" &&
    "listSuperAdmins" in value && typeof value.listSuperAdmins === "function" &&
    "permissions" in value && typeof value.permissions === "function" &&
    "consentState" in value && typeof value.consentState === "function" &&
    "updateConsentState" in value && typeof value.updateConsentState === "function",
  );
}

async function ensureAllowedGroupConsent(groups: GroupForValidation[]): Promise<void> {
  for (const group of groups) {
    if ((await group.consentState()) !== ConsentState.Allowed) {
      await group.updateConsentState(ConsentState.Allowed);
    }
    if ((await group.consentState()) !== ConsentState.Allowed) {
      throw new Error("assigned group consent could not be verified as Allowed");
    }
  }
}
