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
import { isLocalIdentity, type StoredIdentity } from "../identity";
import { encodeReferralAttribution } from "../onboarding-links";
import {
  RegistryUnavailableError,
  resolveTentacleAssignment,
  verifyLivenessCandidate,
  type TentacleAssignment,
} from "./assignment";
import {
  CONTROL_CODECS,
  createJoinControl,
  createLivenessJoinControl,
  createLivenessQueryControl,
  encodeJoinControl,
  encodeLivenessJoinControl,
  encodeLivenessQueryControl,
  isAssignmentContentType,
  isControlContentType,
  isLivenessResponseContentType,
  isLivenessResponseControl,
  isTypingContentType,
  isTypingControl,
  type AssignmentControl,
} from "./control";
import {
  loadLivenessCandidates,
  type LivenessCandidate,
  type LivenessLogger,
} from "./liveness";
import {
  validateAssignedAcolytesGroup,
  validateAssignedGlobalGroups,
  validateAssignedGroups,
  type GroupForValidation,
} from "./group-validation";
import { createChatUiStateStore, type ChatUiStateStore } from "./storage";
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
const REFERRAL_RETRY_COOLDOWN_MS = 30_000;
const MAX_MESSAGE_BYTES = 16 * 1024;
const LIVENESS_WINDOW_MS = 15_000;

type XmtpStream = AsyncStreamProxy<unknown>;
type XmtpClient = Client<unknown>;
type ProbeDm = Dm<unknown> & {
  stream(options?: { onError?: (error: unknown) => void; onFail?: () => void }): Promise<AsyncStreamProxy<DecodedMessage<unknown>>>;
};

interface ProbeDiagnostics {
  responsesObserved: number;
  streamFailures: number;
  rejections: Record<string, number>;
}

interface PreparedLivenessProbe {
  candidate: LivenessCandidate;
  conversationId: string;
  dm: ProbeDm;
  stream: AsyncStreamProxy<DecodedMessage<unknown>>;
  diagnostics: ProbeDiagnostics;
}

interface LivenessProbe extends PreparedLivenessProbe {
  requestId: string;
  expiresAtNs: bigint;
}

interface LivenessWinner {
  candidate: LivenessCandidate;
  requestId: string;
}

class DirectVerificationUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DirectVerificationUnavailableError";
  }
}

class LivenessUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "LivenessUnavailableError";
  }
}

class LivenessPreparationCancelledError extends Error {
  constructor() {
    super("liveness candidate preparation was cancelled");
    this.name = "LivenessPreparationCancelledError";
  }
}

interface MutableChannel extends ChannelSnapshot {
  historyByConversation: Map<string, { beforeNs?: bigint; hasMore: boolean; pageLimit: bigint }>;
}

export interface WorkspaceOptions {
  storage?: Storage;
  resolveAssignment?: typeof resolveTentacleAssignment;
  loadLivenessCandidates?: typeof loadLivenessCandidates;
  verifyLivenessCandidate?: typeof verifyLivenessCandidate;
  livenessWindowMs?: number;
  nowMs?: () => number;
  logger?: LivenessLogger;
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

export async function openRegisteredClient(
  config: AppConfig,
  identity: StoredIdentity,
): Promise<{ client: XmtpClient; releaseDatabaseLease: () => Promise<void> }> {
  const releaseDatabaseLease = await acquireXmtpDatabaseLease(config.environment, identity.address);
  let client: XmtpClient;
  try {
    const signer = isLocalIdentity(identity)
      ? createSigner(new Wallet(identity.walletPrivateKey))
      : await (await import("../wallet-connector")).createExternalSigner(identity);
    client = (await recoverRegisteredClient(() =>
      Client.create(signer, {
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
  private readonly loadCandidates: typeof loadLivenessCandidates;
  private readonly verifyLiveCandidate: typeof verifyLivenessCandidate;
  private readonly livenessWindowMs: number;
  private readonly nowMs: () => number;
  private readonly logger: LivenessLogger;
  private readonly storage: Storage | undefined;
  private assignmentState: AssignmentState = "checking";
  private assignmentNotice = "Checking the canonical assignment…";
  private verificationWarning?: string;
  private tentacleName = "Tentacle";
  private connected = false;
  private closed = false;
  private direct?: Dm<unknown>;
  private currentAssignment?: TentacleAssignment;
  private currentTentacleInboxId?: string;
  private pendingRequestId?: string;
  private pendingLivenessRequestId?: string;
  private retainedRotationAddress?: string;
  private unpersistedLivenessCandidate?: LivenessCandidate;
  private livenessAttempted = false;
  private livenessFailure?: string;
  private livenessPreProbeFailure?: string;
  private livenessProbeInFlight?: Promise<TentacleAssignment>;
  private pendingGroupAssignment?: AssignmentControl;
  private trustedGroupAssignment?: AssignmentControl;
  private revalidation?: Promise<void>;
  private refreshTimer?: ReturnType<typeof setInterval>;
  private restartingStreams?: Promise<void>;
  private needsStreamRestart = false;
  private streamGeneration = 0;
  private registryFailureCount = 0;
  private nextAutomaticRegistryAttemptAt = 0;
  private referralAcknowledged = false;
  private lastReferralAttemptAt = 0;
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
    this.loadCandidates = options.loadLivenessCandidates ?? loadLivenessCandidates;
    this.verifyLiveCandidate = options.verifyLivenessCandidate ?? verifyLivenessCandidate;
    this.livenessWindowMs = options.livenessWindowMs ?? LIVENESS_WINDOW_MS;
    this.nowMs = options.nowMs ?? Date.now;
    this.logger = options.logger ?? console;
    this.retainedRotationAddress = config.rotationAnchor;
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
    const provisional: TentacleAssignment = {
      source: "unverified",
      address: this.config.tentacleAnchor ?? this.retainedRotationAddress ?? this.config.botAddress,
      notice: "XMTP connected · checking Tentacle registration in the background…",
    };
    await this.handoffDirect(provisional);
    if (this.closed) return;
    this.currentAssignment = provisional;
    this.assignmentState = "unverified";
    this.assignmentNotice = provisional.notice;
    void this.revalidateAssignment("connect").catch(() => undefined);
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
      verificationWarning: this.verificationWarning,
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
      await this.sendReferralAttributionIfNeeded().catch(() => undefined);
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
      this.assignmentNotice = this.direct ? "XMTP connected · checking Base in the background…" : "Checking the canonical assignment…";
      this.emit();
    }
    if (reason === "retry") {
      this.livenessPreProbeFailure = undefined;
    }
    try {
      if ((this.needsStreamRestart || !this.connected) && (reason === "resume" || reason === "retry")) {
        await this.restartStreams();
      }
      let assignment = await this.resolveAssignment(
        { ...this.config, rotationAnchor: this.retainedRotationAddress },
        this.identity,
      );
      if (this.closed) return;
      if (assignment.source === "liveness-required") {
        assignment = await this.selectLiveAssignment(reason);
      } else if (assignment.source === "anchor-verified" && this.config.tentacleAnchor && !this.pendingLivenessRequestId) {
        const candidate = candidateFromVerified(assignment);
        try {
          const verified = await this.runLivenessProbe([candidate]);
          if (verified.source === "rotation-verified") {
            assignment = {
              ...verified,
              source: "anchor-verified",
              notice: `Deep-linked Tentacle canonically verified at Base block ${verified.blockNumber}`,
            };
          }
        } catch (error) {
          if (error instanceof LivenessUnavailableError) {
            throw new LivenessUnavailableError(
              "The deep-linked Tentacle did not answer the liveness check. It may be offline or unreachable.",
            );
          }
          throw error;
        }
      }
      if (this.closed) return;
      if (this.currentAssignment?.source === "unverified" &&
          assignment.address.toLowerCase() !== this.currentAssignment.address.toLowerCase()) {
        this.assignmentState = "unverified";
        this.assignmentNotice = "XMTP connected · current target is not the canonical assignment";
        this.verificationWarning = `Base routing points to ${assignment.address}. Your current conversation has been kept open; no messages were moved or resent.`;
        this.emit();
        return;
      }
      this.verificationWarning = undefined;
      this.registryFailureCount = 0;
      this.nextAutomaticRegistryAttemptAt = 0;
      const promotingCurrentTarget = this.currentAssignment?.source === "unverified" &&
        assignment.address.toLowerCase() === this.currentAssignment.address.toLowerCase();
      const changed = !promotingCurrentTarget && routeKey(assignment) !== routeKey(this.currentAssignment);
      if (changed || !this.direct) {
        await this.handoffDirect(assignment);
        if (this.closed) return;
        this.currentAssignment = assignment;
        this.assignmentState = assignment.source;
        this.assignmentNotice = assignment.notice;
        await this.requestAssignment();
        if (assignment.source === "rotation-verified" && this.pendingLivenessRequestId) {
          this.persistRotation(assignment.address);
          this.retainedRotationAddress = assignment.address;
          this.unpersistedLivenessCandidate = undefined;
        }
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
        if (this.closed) return;
        this.currentAssignment = assignment;
        this.tentacleName = isVerifiedTentacle(assignment) ? assignment.name : assignment.source === "unverified" ? "XMTP contact · unverified" : "Intro Tentacle";
        this.assignmentState = assignment.source;
        this.assignmentNotice = assignment.notice;
        this.restoreVerifiedChannels();
        await this.requestAssignment();
        if (assignment.source === "rotation-verified" && this.pendingLivenessRequestId) {
          this.persistRotation(assignment.address);
          this.retainedRotationAddress = assignment.address;
          this.unpersistedLivenessCandidate = undefined;
        }
      }
      await this.sendReferralAttributionIfNeeded();
      this.emit();
    } catch (error) {
      if (this.closed) return;
      if (
        error instanceof RegistryUnavailableError ||
        error instanceof DirectVerificationUnavailableError ||
        error instanceof LivenessUnavailableError
      ) {
        if (error instanceof RegistryUnavailableError) {
          this.registryFailureCount += 1;
          const delay = Math.min(15 * 60_000, 30_000 * 2 ** Math.min(5, this.registryFailureCount - 1));
          this.nextAutomaticRegistryAttemptAt = Date.now() + delay;
        }
        this.assignmentState = error instanceof DirectVerificationUnavailableError
          ? "direct-verification-unavailable"
          : error instanceof LivenessUnavailableError
            ? "liveness-unavailable"
            : "registry-unavailable";
        this.assignmentNotice = error.message;
        // Base availability is not an XMTP authorization or retention check. Keep a verified
        // direct conversation usable and surface registry/liveness failures outside the transcript.
        if (!(error instanceof DirectVerificationUnavailableError) && this.direct) {
          this.verificationWarning = `Tentacle verification could not be completed. ${error.message}. You can keep chatting; registration is not confirmed.`;
          this.assignmentNotice = "XMTP connected · Tentacle verification unavailable";
          this.blockChannel("acolytes", error.message);
          this.channels.acolytes.retentionVerified = false;
          this.emit();
          return;
        }
        this.blockChannel("direct", error.message);
        this.blockChannel("acolytes", error.message);
        this.channels.direct.retentionVerified = false;
        this.channels.acolytes.retentionVerified = false;
        this.emit();
        return;
      }
      const message = error instanceof Error ? error.message : "XMTP assignment handoff failed";
      this.assignmentNotice = `Assignment retry required: ${message}`;
      if (this.direct) {
        this.verificationWarning = `Tentacle verification failed: ${message}. Your XMTP conversation remains open.`;
      }
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

  private async selectLiveAssignment(
    reason: "connect" | "resume" | "periodic" | "retry",
  ): Promise<TentacleAssignment> {
    if (this.unpersistedLivenessCandidate) {
      return this.verifyLiveCandidate(this.config, this.identity, this.unpersistedLivenessCandidate);
    }
    if (this.livenessAttempted) {
      throw new LivenessUnavailableError(
        this.livenessFailure ?? "The one-time live Tentacle check has already completed without a responder",
      );
    }
    if (this.livenessPreProbeFailure && reason !== "retry") {
      throw new RegistryUnavailableError(this.livenessPreProbeFailure);
    }
    if (this.livenessProbeInFlight) return this.livenessProbeInFlight;
    if (reason === "retry") this.livenessPreProbeFailure = undefined;
    this.livenessProbeInFlight = this.runLivenessProbe()
      .catch((error: unknown) => {
        if (!this.livenessAttempted) {
          this.livenessPreProbeFailure = error instanceof Error
            ? error.message
            : "Live Tentacle selection failed before a liveness query could be sent";
        }
        throw error;
      })
      .finally(() => {
        this.livenessProbeInFlight = undefined;
      });
    return this.livenessProbeInFlight;
  }

  private async runLivenessProbe(
    candidatesToProbe?: LivenessCandidate[],
  ): Promise<TentacleAssignment> {
    const candidates = candidatesToProbe ?? (await this.loadCandidates(
      this.config,
      this.identity.address,
      this.inboxId,
      this.storage,
      { logger: this.logger, now: this.nowMs },
    ));
    if (candidates.length === 0) {
      throw new RegistryUnavailableError("No eligible ranked Tentacle is available for the liveness check");
    }
    this.assignmentNotice = `Preparing a private “fhtagn?” check for ${candidates.length} ranked Tentacle${candidates.length === 1 ? "" : "s"}…`;
    this.emit();
    this.logger.info("[cthuwu-liveness] preparing ranked probes", {
      candidateCount: candidates.length,
      candidates: candidates.map(candidateDiagnostic),
    });

    const preparationCancellation = { cancelled: false };
    const prepared: PreparedLivenessProbe[] = [];
    const preparationTasks = candidates.map(async (candidate) => {
      this.logger.debug("[cthuwu-liveness] preparing candidate", candidateDiagnostic(candidate));
      try {
        const probe = await this.prepareLivenessProbe(
          candidate,
          () => preparationCancellation.cancelled,
        );
        if (preparationCancellation.cancelled) {
          await probe.stream.return().catch(() => undefined);
          return;
        }
        prepared.push(probe);
        this.logger.debug("[cthuwu-liveness] candidate prepared", candidateDiagnostic(candidate));
      } catch (error) {
        if (error instanceof LivenessPreparationCancelledError) {
          this.logger.debug("[cthuwu-liveness] late candidate preparation cancelled", candidateDiagnostic(candidate));
          return;
        }
        this.logger.warn("[cthuwu-liveness] candidate preparation failed", {
          ...candidateDiagnostic(candidate),
          reason: safeDiagnosticReason(error),
        });
      }
    });
    let preparationTimedOut = false;
    let preparationTimeoutId: ReturnType<typeof setTimeout> | undefined;
    const preparationTimeout = new Promise<void>((resolve) => {
      preparationTimeoutId = setTimeout(() => {
        preparationTimedOut = true;
        preparationCancellation.cancelled = true;
        resolve();
      }, this.livenessWindowMs);
    });
    await Promise.race([Promise.allSettled(preparationTasks), preparationTimeout]);
    if (preparationTimeoutId) clearTimeout(preparationTimeoutId);
    preparationCancellation.cancelled = true;
    if (preparationTimedOut) {
      this.logger.warn("[cthuwu-liveness] candidate preparation window ended", {
        candidateCount: candidates.length,
        preparedCount: prepared.length,
      });
    }
    if (prepared.length === 0) {
      throw new RegistryUnavailableError(
        `Live Tentacle selection could not prepare any probe (0/${candidates.length} candidates ready); see the browser console for safe diagnostics`,
      );
    }

    this.assignmentNotice = `Whispering “fhtagn?” to ${prepared.length} ranked Tentacle${prepared.length === 1 ? "" : "s"}…`;
    this.emit();
    const startedAtMs = Math.floor(this.nowMs());
    if (!Number.isSafeInteger(startedAtMs) || startedAtMs < 0) {
      await closeProbeStreams(prepared);
      throw new RegistryUnavailableError("The browser clock is unavailable for the liveness check");
    }
    const deadlineAtMs = startedAtMs + this.livenessWindowMs;
    const expiresAtNs = BigInt(deadlineAtMs) * 1_000_000n;
    let resolveWinner!: (winner: LivenessWinner) => void;
    const winner = new Promise<LivenessWinner>((resolve) => { resolveWinner = resolve; });
    let settled = false;
    const accept = (value: LivenessWinner): void => {
      if (settled) return;
      settled = true;
      this.livenessAttempted = true;
      resolveWinner(value);
    };

    const probes = prepared.map((candidate) => {
      const query = createLivenessQueryControl(candidate.candidate.agentId, expiresAtNs.toString());
      return { ...candidate, requestId: query.requestId, expiresAtNs, query };
    });
    const consumers = probes.map(({ query: _query, ...probe }) =>
      this.consumeLivenessProbe(probe, accept));
    let windowClosed = false;
    let publishedCount = 0;
    let pendingSendCount = probes.length;
    let resolveNoPublish!: () => void;
    const noPublish = new Promise<void>((resolve) => { resolveNoPublish = resolve; });
    let timeoutId: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<"timeout">((resolve) => {
      timeoutId = setTimeout(
        () => resolve("timeout"),
        Math.max(0, deadlineAtMs - Math.floor(this.nowMs())),
      );
    });
    const sendTasks = probes.map(async ({ query, ...probe }) => {
      try {
        await probe.dm.send(encodeLivenessQueryControl(query), { shouldPush: false });
        const withinResponseWindow =
          !windowClosed && Math.floor(this.nowMs()) <= deadlineAtMs;
        if (withinResponseWindow) {
          publishedCount += 1;
          this.livenessAttempted = true;
        }
        this.logger.info("[cthuwu-liveness] query published", {
          ...candidateDiagnostic(probe.candidate),
          withinResponseWindow,
        });
      } catch (error) {
        this.logger.warn("[cthuwu-liveness] query publish failed", {
          ...candidateDiagnostic(probe.candidate),
          reason: safeDiagnosticReason(error),
        });
        await probe.stream.return().catch(() => undefined);
      } finally {
        pendingSendCount -= 1;
        if (pendingSendCount === 0 && publishedCount === 0 && !settled && !windowClosed) {
          resolveNoPublish();
        }
      }
    });
    const outcome = await Promise.race([
      winner.then((value) => ({ type: "winner" as const, value })),
      timeout.then(() => ({ type: "timeout" as const })),
      noPublish.then(() => ({ type: "no-publish" as const })),
    ]);
    windowClosed = true;
    if (timeoutId) clearTimeout(timeoutId);
    await closeProbeStreams(prepared);
    await Promise.allSettled(consumers);
    if (outcome.type === "no-publish") {
      await Promise.allSettled(sendTasks);
      throw new RegistryUnavailableError(
        `Live Tentacle selection could not publish a liveness query (0/${prepared.length} prepared probes sent); see the browser console for safe diagnostics`,
      );
    }
    if (outcome.type !== "winner") {
      if (pendingSendCount > 0) {
        // A send that outlives the response window may still cross XMTP, but its embedded query is
        // already expired and therefore inert. Treat this workspace's bounded attempt as consumed.
        this.livenessAttempted = true;
      }
      const responsesObserved = prepared.reduce(
        (total, probe) => total + probe.diagnostics.responsesObserved,
        0,
      );
      const rejections = mergeRejectionCounts(prepared);
      this.logger.warn("[cthuwu-liveness] ranked probe window ended without a winner", {
        candidateCount: candidates.length,
        preparedCount: prepared.length,
        publishedCount,
        responsesObserved,
        streamFailures: prepared.reduce(
          (total, probe) => total + probe.diagnostics.streamFailures,
          0,
        ),
        rejections,
      });
      this.livenessFailure =
        `No ranked Tentacle answered with a compatible liveness response (${publishedCount}/${candidates.length} probes sent; ${responsesObserved} response${responsesObserved === 1 ? "" : "s"} observed). Tentacles may be offline or need the current sidecar.`;
      throw new LivenessUnavailableError(this.livenessFailure);
    }
    const selected = outcome.value;
    this.logger.info("[cthuwu-liveness] accepted ranked response", candidateDiagnostic(selected.candidate));
    this.pendingLivenessRequestId = selected.requestId;
    this.unpersistedLivenessCandidate = selected.candidate;
    return this.verifyLiveCandidate(this.config, this.identity, selected.candidate);
  }

  private async prepareLivenessProbe(
    candidate: LivenessCandidate,
    cancelled: () => boolean,
  ): Promise<PreparedLivenessProbe> {
    await verifyPeerInboxState(this.client, candidate.inboxId, candidate.wallet);
    if (cancelled()) throw new LivenessPreparationCancelledError();
    if (this.closed) throw new Error("the workspace closed before DM creation");
    const disappearing = { fromNs: RETENTION_FROM_NS, inNs: RETENTION_IN_NS };
    const dm = await this.client.conversations.createDm(candidate.inboxId, {
      messageDisappearingSettings: disappearing,
    });
    if (cancelled()) throw new LivenessPreparationCancelledError();
    if (await dm.peerInboxId() !== candidate.inboxId) {
      throw new Error("XMTP resolved a different liveness candidate inbox");
    }
    if (cancelled()) throw new LivenessPreparationCancelledError();
    await repairDirectRetention(dm);
    if (cancelled()) throw new LivenessPreparationCancelledError();
    if (this.closed) throw new Error("the workspace closed before stream creation");
    const diagnostics: ProbeDiagnostics = {
      responsesObserved: 0,
      streamFailures: 0,
      rejections: {},
    };
    const stream = await (dm as ProbeDm).stream({
      onError: (error) => {
        diagnostics.streamFailures += 1;
        this.logger.warn("[cthuwu-liveness] candidate stream error", {
          ...candidateDiagnostic(candidate),
          reason: safeDiagnosticReason(error),
        });
      },
      onFail: () => {
        diagnostics.streamFailures += 1;
        this.logger.warn("[cthuwu-liveness] candidate stream failed", candidateDiagnostic(candidate));
      },
    });
    if (cancelled() || this.closed) {
      await stream.return();
      if (cancelled()) throw new LivenessPreparationCancelledError();
      throw new Error("the workspace closed before the candidate was ready");
    }
    return {
      candidate,
      conversationId: dm.id,
      dm: dm as ProbeDm,
      stream,
      diagnostics,
    };
  }

  private async consumeLivenessProbe(
    probe: LivenessProbe,
    accept: (winner: LivenessWinner) => void,
  ): Promise<void> {
    try {
      for await (const message of probe.stream) {
        // A probe DM can also carry the query's own echo or ordinary existing traffic. Only an
        // actual liveness-response envelope counts as an observed response in diagnostics.
        if (!isLivenessResponseContentType(message.contentType)) continue;
        probe.diagnostics.responsesObserved += 1;
        const rejection = livenessRejectionReason(message, probe, this.nowMs);
        if (rejection) {
          const count = (probe.diagnostics.rejections[rejection] ?? 0) + 1;
          probe.diagnostics.rejections[rejection] = count;
          if (count === 1) {
            this.logger.debug("[cthuwu-liveness] rejected candidate response", {
              ...candidateDiagnostic(probe.candidate),
              reason: rejection,
            });
          }
          continue;
        }
        accept({ candidate: probe.candidate, requestId: probe.requestId });
        return;
      }
    } catch (error) {
      probe.diagnostics.streamFailures += 1;
      this.logger.warn("[cthuwu-liveness] candidate stream stopped", {
        ...candidateDiagnostic(probe.candidate),
        reason: safeDiagnosticReason(error),
      });
      // Another candidate may still answer. A dead probe is not permission to select it.
    }
  }

  private persistRotation(address: string): void {
    if (!this.storage) return;
    const key = `cthuwu.rotation.v1:${this.config.environment}:${this.identity.address}`;
    this.storage.setItem(key, address);
    if (this.storage.getItem(key) !== address) throw new Error("The live Tentacle choice could not be persisted");
  }

  private clearRetainedRotation(): void {
    this.retainedRotationAddress = undefined;
    if (this.storage) {
      const key = `cthuwu.rotation.v1:${this.config.environment}:${this.identity.address}`;
      this.storage.removeItem(key);
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
    if (this.closed) return;
    this.direct = direct;
    this.currentTentacleInboxId = peerInboxId;
    this.conversations.set(direct.id, direct);
    this.trustedChannelByConversation.set(direct.id, "direct");
    this.bindChannel("direct", [direct.id], direct.id, true);
    this.tentacleName = isVerifiedTentacle(assignment) ? assignment.name : assignment.source === "unverified" ? "XMTP contact · unverified" : "Intro Tentacle";
    await this.loadHistory("direct", false);
    // The browser-local first-valid pin is only a hint until the authenticated Direct sender binds
    // it at the Tentacle. Retry after handoff/reconnect until an authenticated terminal ACK appears
    // in Direct history; the Tentacle's durable first-write rule makes retries idempotent.
    if (assignment.source !== "unverified") await this.sendReferralAttributionIfNeeded(true);
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
    if (this.currentAssignment?.source === "anchor-verified") {
      this.blockChannel(
        "acolytes",
        "Explicit Tentacle links are Direct-only while your Acolyte is Unminted; use automatic live selection or mint Branding to join Acolytes",
      );
      this.blockChannel(
        "global",
        "Explicit Tentacle links are Direct-only while your Acolyte is Unminted; Global enrollment is unavailable on this route",
      );
      this.emit();
      return;
    }
    if (this.currentAssignment?.source === "intro-unconfigured" || this.currentAssignment?.source === "unverified") {
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
    if (this.pendingLivenessRequestId && this.currentAssignment?.source === "rotation-verified") {
      const join = createLivenessJoinControl(this.pendingLivenessRequestId);
      this.pendingRequestId = join.requestId;
      await this.direct.send(encodeLivenessJoinControl(join), { shouldPush: false });
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
      await this.sendReferralAttributionIfNeeded(true);
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
    this.noteReferralAcknowledgement(channelId, decoded);
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
      this.pendingLivenessRequestId = undefined;
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
      for (const message of decoded) this.noteReferralAcknowledgement(channelId, message);
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
    if (current.status !== "policy-blocked") current.status = "ready";
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

  private referralAcknowledgementKey(peerInboxId: string): string {
    return `cthuwu.referral-ack.v1:${this.config.environment}:${this.identity.address.toLowerCase()}:${peerInboxId}`;
  }

  private noteReferralAcknowledgement(
    channelId: ChatChannel,
    message: WorkspaceMessage,
  ): void {
    if (
      !this.config.referrer ||
      message.mine ||
      channelId !== "direct" ||
      message.senderInboxId !== this.currentTentacleInboxId ||
      !/(?:^|\n)\[\[cthuwu:referral-attribution-ack:v1;status=(?:accepted|immutable|direct);referrer=(?:0x[0-9a-f]{40}|none)\]\]$/u
        .test(message.text)
    ) {
      return;
    }
    this.referralAcknowledged = true;
    if (!this.currentTentacleInboxId) return;
    try {
      this.storage?.setItem(
        this.referralAcknowledgementKey(this.currentTentacleInboxId),
        "acknowledged",
      );
    } catch {
      // The in-memory acknowledgement still prevents retries for this workspace.
    }
  }

  private async sendReferralAttributionIfNeeded(force = false): Promise<void> {
    if (this.currentAssignment?.source === "unverified" || !this.config.referrer || !this.direct || !this.currentTentacleInboxId) return;
    let persisted = false;
    try {
      persisted = this.storage?.getItem(
        this.referralAcknowledgementKey(this.currentTentacleInboxId),
      ) === "acknowledged";
    } catch {
      persisted = false;
    }
    if (this.referralAcknowledged || persisted) {
      this.referralAcknowledged = true;
      return;
    }
    const now = this.nowMs();
    if (!force && now - this.lastReferralAttemptAt < REFERRAL_RETRY_COOLDOWN_MS) return;
    await this.direct.sendText(encodeReferralAttribution(this.config.referrer));
    this.lastReferralAttemptAt = now;
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

function livenessRejectionReason(
  message: DecodedMessage<unknown>,
  probe: LivenessProbe,
  nowMs: () => number,
): string | undefined {
  if (message.conversationId !== probe.conversationId) return "conversation-mismatch";
  if (message.senderInboxId !== probe.candidate.inboxId) return "sender-mismatch";
  if (!isLivenessResponseContentType(message.contentType)) return "content-type-mismatch";
  if (!isLivenessResponseControl(message.content)) return "invalid-control";
  if (message.content.requestId !== probe.requestId) return "request-mismatch";
  if (message.content.tentacleAgentId !== probe.candidate.agentId) return "agent-mismatch";
  // The response is bound to an unguessable fresh request and must arrive before the local
  // deadline. Do not compare sentAtNs to the browser clock in either direction: the responder is a
  // different host and ordinary NTP skew must not turn a live Tentacle into a false negative.
  const currentMs = Math.floor(nowMs());
  if (!Number.isSafeInteger(currentMs) || currentMs < 0) return "browser-clock-unavailable";
  if (BigInt(currentMs) * 1_000_000n > probe.expiresAtNs) return "received-after-local-deadline";
  return undefined;
}

function candidateDiagnostic(candidate: LivenessCandidate): Record<string, unknown> {
  return {
    rank: candidate.rank,
    agentId: candidate.agentId,
    directoryBlock: candidate.blockNumber,
  };
}

function safeDiagnosticReason(error: unknown): string {
  const value = error instanceof Error ? `${error.name}: ${error.message}` : "Unknown error";
  return value
    .replace(/https?:\/\/[^\s)\]}]+/giu, "<redacted-endpoint>")
    .replace(/(?:0x)?[0-9a-f]{64}/giu, "<redacted-id>")
    .replace(/[\r\n]+/gu, " ")
    .slice(0, 512);
}

async function closeProbeStreams(probes: PreparedLivenessProbe[]): Promise<void> {
  await Promise.allSettled(probes.map((probe) => probe.stream.return()));
}

function mergeRejectionCounts(probes: PreparedLivenessProbe[]): Record<string, number> {
  const merged: Record<string, number> = {};
  for (const probe of probes) {
    for (const [reason, count] of Object.entries(probe.diagnostics.rejections)) {
      merged[reason] = (merged[reason] ?? 0) + count;
    }
  }
  return merged;
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

function candidateFromVerified(
  assignment: Extract<TentacleAssignment, { source: "rotation-verified" | "anchor-verified" }>,
): LivenessCandidate {
  return {
    wallet: assignment.wallet,
    agentId: assignment.agentId,
    inboxId: assignment.inboxId,
    name: assignment.name,
    rank: 1,
    blockNumber: assignment.blockNumber.toString(),
    blockHash: assignment.blockHash,
  };
}

export async function verifyPeerInboxState(
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
