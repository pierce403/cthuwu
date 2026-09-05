import {
  Client,
  ConsentState,
  GroupPermissionsOptions,
  IdentifierKind,
  PermissionPolicy,
} from "@xmtp/browser-sdk";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppConfig } from "../config";
import type { StoredIdentity } from "../identity";
import { RegistryUnavailableError, type TentacleAssignment } from "./assignment";
import {
  ASSIGNMENT_CONTENT_TYPE,
  LIVENESS_RESPONSE_CONTENT_TYPE,
  isJoinControl,
  joinCodec,
  livenessJoinCodec,
  livenessQueryCodec,
  type AssignmentControl,
} from "./control";
import { createChatUiStateStore } from "./storage";
import {
  XmtpMultiChannelWorkspace,
  acquireXmtpDatabaseLease,
  ensureXmtpIdentityRegistration,
  recoverRegisteredClient,
} from "./xmtp-workspace";

const own = "1".repeat(64);
const tentacle = "2".repeat(64);
const directId = "3".repeat(64);
const acolytesId = "4".repeat(64);
const globalId = "5".repeat(64);
const address = "0x2222222222222222222222222222222222222222";
const adminOnlyPolicy = {
  addMemberPolicy: PermissionPolicy.Admin,
  removeMemberPolicy: PermissionPolicy.Admin,
  addAdminPolicy: PermissionPolicy.SuperAdmin,
  removeAdminPolicy: PermissionPolicy.SuperAdmin,
  updateGroupNamePolicy: PermissionPolicy.Admin,
  updateGroupDescriptionPolicy: PermissionPolicy.Admin,
  updateGroupImageUrlSquarePolicy: PermissionPolicy.Admin,
  updateMessageDisappearingPolicy: PermissionPolicy.Admin,
  updateAppDataPolicy: PermissionPolicy.Admin,
};
const config: AppConfig = {
  environment: "production",
  botAddress: address,
  baseRpcEndpoint: "https://mainnet.base.org/",
  assignmentRefreshMs: 3_600_000,
};
const identity = {
  version: 1,
  environment: "production",
  address: "0x1111111111111111111111111111111111111111",
  walletPrivateKey: `0x${"12".repeat(32)}`,
  compatibilityDbKey: `0x${"34".repeat(32)}`,
  createdAt: "2026-08-11T00:00:00.000Z",
} satisfies StoredIdentity;
const intro: TentacleAssignment = {
  source: "intro-unconfigured",
  address,
  notice: "Branding routing pending deployment",
};
const configuredFallback: TentacleAssignment = {
  source: "intro-fallback",
  address,
  brandingStatus: "Expired",
  blockNumber: 123n,
  blockHash: `0x${"6".repeat(64)}`,
  notice: "Expired Branding; using intro Tentacle",
};
const rotated: TentacleAssignment = {
  source: "rotation-verified",
  address,
  wallet: address,
  inboxId: tentacle,
  agentId: "42",
  name: "Fresh Registry Name",
  blockNumber: 123n,
  blockHash: `0x${"6".repeat(64)}`,
  notice: "Eligible Tentacle rotation verified",
};
const anchored: TentacleAssignment = {
  ...rotated,
  source: "anchor-verified",
  notice: "Explicit Tentacle verified",
};
const livenessRequired: TentacleAssignment = {
  source: "liveness-required",
  address,
  brandingStatus: "Unminted",
  blockNumber: 123n,
  blockHash: `0x${"6".repeat(64)}`,
  notice: "Unminted Branding; checking live Tentacles",
};

class FakeStream<T> implements AsyncIterable<T> {
  private values: T[] = [];
  private waiters: Array<(value: IteratorResult<T>) => void> = [];
  private done = false;

  push(value: T): void {
    const waiter = this.waiters.shift();
    if (waiter) waiter({ value, done: false });
    else this.values.push(value);
  }

  [Symbol.asyncIterator](): AsyncIterator<T> {
    return {
      next: async () => {
        const value = this.values.shift();
        if (value !== undefined) return { value, done: false };
        if (this.done) return { value: undefined, done: true };
        return new Promise<IteratorResult<T>>((resolve) => this.waiters.push(resolve));
      },
      return: async () => {
        this.done = true;
        for (const waiter of this.waiters.splice(0)) waiter({ value: undefined, done: true });
        return { value: undefined, done: true };
      },
    };
  }

  async return(): Promise<{ value: undefined; done: boolean }> {
    this.done = true;
    for (const waiter of this.waiters.splice(0)) waiter({ value: undefined, done: true });
    return { value: undefined, done: true };
  }
}

function textMessage(id: string, conversationId: string, sentAtNs: bigint, sender = tentacle) {
  return {
    id,
    conversationId,
    senderInboxId: sender,
    sentAtNs,
    contentType: { authorityId: "xmtp.org", typeId: "text", versionMajor: 1, versionMinor: 0 },
    content: `message ${id}`,
  };
}

function assignment(requestId: string): AssignmentControl {
  return {
    type: "cthuwu.assignment.v1",
    requestId,
    environment: "production",
    // The Tentacle samples its own explicit Base block after the browser's block 123 assignment.
    revision: `124:0x${"7".repeat(64)}`,
    tentacleAgentId: "42",
    tentacleInboxId: tentacle,
    acolytesGroupId: acolytesId,
    global: {
      logicalChannelId: "cthuwu.global.v1",
      readConversationIds: [globalId],
      writeConversationId: globalId,
      adminInboxIds: [tentacle],
    },
    retention: { fromNs: "1", inNs: "1209600000000000" },
  };
}

function fixture() {
  const streams = {
    messages: new FakeStream<ReturnType<typeof textMessage>>(),
    groups: new FakeStream<unknown>(),
    deleted: new FakeStream<ReturnType<typeof textMessage>>(),
    probe: new FakeStream<unknown>(),
  };
  const streamHistory = {
    messages: [streams.messages],
    groups: [streams.groups],
    deleted: [streams.deleted],
  };
  const sentControls: unknown[] = [];
  let directPolicy: { fromNs: bigint; inNs: bigint } | undefined;
  let peerAddress = address;
  const directMessages: ReturnType<typeof textMessage>[] = [];
  const direct = {
    id: directId,
    peerInboxId: vi.fn(async () => tentacle),
    consentState: vi.fn(async () => ConsentState.Allowed),
    updateConsentState: vi.fn(async () => undefined),
    messageDisappearingSettings: vi.fn(async () => directPolicy),
    updateMessageDisappearingSettings: vi.fn(async (fromNs: bigint, inNs: bigint) => {
      directPolicy = { fromNs, inNs };
    }),
    messages: vi.fn(async () => directMessages),
    send: vi.fn(async (content: unknown) => { sentControls.push(content); }),
    stream: vi.fn(async () => streams.probe),
    sendText: vi.fn(async () => undefined),
  };
  let globalPolicy = { fromNs: 1n, inNs: 1_209_600_000_000_000n };
  let acolytesPolicy = { fromNs: 1n, inNs: 1_209_600_000_000_000n };
  const history = Array.from({ length: 50 }, (_, index) =>
    textMessage(`history-${index}`, globalId, 1_000n));
  const makeGroup = (id: string, channel: "acolytes" | "global") => {
    let consent = ConsentState.Unknown;
    return {
      id,
      addedByInboxId: tentacle,
      metadata: { creatorInboxId: tentacle },
      appData: JSON.stringify(channel === "acolytes" ? {
        app: "cthuwu.chat", version: 1, environment: "production", channel,
        tentacleAgentId: "42", tentacleInboxId: tentacle,
      } : {
        app: "cthuwu.chat", version: 1, environment: "production", channel,
        logicalChannelId: "cthuwu.global.v1", shardId: "primary",
      }),
      members: vi.fn(async () => [{ inboxId: own }, { inboxId: tentacle }]),
      listAdmins: vi.fn(async () => channel === "global" ? [tentacle] : []),
      listSuperAdmins: vi.fn(async () => channel === "acolytes" ? [tentacle] : []),
      permissions: vi.fn(async () => ({
        policyType: GroupPermissionsOptions.AdminOnly,
        policySet: { ...adminOnlyPolicy },
      })),
      consentState: vi.fn(async () => consent),
      updateConsentState: vi.fn(async (state: ConsentState) => { consent = state; }),
      messageDisappearingSettings: vi.fn(async () => channel === "global" ? globalPolicy : acolytesPolicy),
      sync: vi.fn(async () => undefined),
      messages: vi.fn(async ({ limit }: { limit: bigint }) =>
        channel === "global" ? history.slice(0, Number(limit)) : []),
      sendText: vi.fn(async () => undefined),
    };
  };
  const acolytes = makeGroup(acolytesId, "acolytes");
  const global = makeGroup(globalId, "global");
  let allMessageOptions: { onFail?: () => void } | undefined;
  let messageStreamCalls = 0;
  let groupStreamCalls = 0;
  let deletedStreamCalls = 0;
  const client = {
    inboxId: own,
    conversations: {
      streamAllMessages: vi.fn(async (options: { onFail?: () => void }) => {
        allMessageOptions = options;
        if (messageStreamCalls++ === 0) return streams.messages;
        const replacement = new FakeStream<ReturnType<typeof textMessage>>();
        streamHistory.messages.push(replacement);
        return replacement;
      }),
      streamGroups: vi.fn(async () => {
        if (groupStreamCalls++ === 0) return streams.groups;
        const replacement = new FakeStream<unknown>();
        streamHistory.groups.push(replacement);
        return replacement;
      }),
      streamDeletedMessages: vi.fn(async () => {
        if (deletedStreamCalls++ === 0) return streams.deleted;
        const replacement = new FakeStream<ReturnType<typeof textMessage>>();
        streamHistory.deleted.push(replacement);
        return replacement;
      }),
      createDmWithIdentifier: vi.fn(async () => direct),
      createDm: vi.fn(async () => direct),
      sync: vi.fn(async () => undefined),
      getConversationById: vi.fn(async (id: string) => id === acolytesId ? acolytes : id === globalId ? global : undefined),
    },
    preferences: {
      fetchInboxStates: vi.fn(async () => [{
        inboxId: tentacle,
        recoveryIdentifier: { identifier: peerAddress, identifierKind: IdentifierKind.Ethereum },
        accountIdentifiers: [{ identifier: peerAddress, identifierKind: IdentifierKind.Ethereum }],
        installations: [],
      }]),
    },
    close: vi.fn(),
  };
  return {
    streams, streamHistory, sentControls, direct, acolytes, global, client,
    allMessageOptions: () => allMessageOptions,
    setPeerAddress: (value: string) => { peerAddress = value; },
    breakDirectPolicy: () => { directPolicy = { fromNs: 0n, inNs: 1n }; },
    breakAcolytesPolicy: () => { acolytesPolicy = { fromNs: 0n, inNs: 1n }; },
    restoreAcolytesPolicy: () => { acolytesPolicy = { fromNs: 1n, inNs: 1_209_600_000_000_000n }; },
    breakGlobalPolicy: () => { globalPolicy = { fromNs: 0n, inNs: 1n }; },
    restoreGlobalPolicy: () => { globalPolicy = { fromNs: 1n, inNs: 1_209_600_000_000_000n }; },
  };
}

async function waitFor(check: () => boolean): Promise<void> {
  await vi.waitFor(() => expect(check()).toBe(true));
}

async function startVerified(workspace: XmtpMultiChannelWorkspace): Promise<void> {
  await workspace.start();
  await workspace.revalidateAssignment("connect");
}

describe("multi-channel XMTP workspace", () => {
  beforeEach(() => localStorage.clear());

  it("reuses an installation and closes failed registration", async () => {
    const existing = { isRegistered: vi.fn(async () => true), register: vi.fn(), revokeAllOtherInstallations: vi.fn(), close: vi.fn() };
    await expect(recoverRegisteredClient(async () => existing)).resolves.toBe(existing);
    expect(existing.register).not.toHaveBeenCalled();
    const failed = {
      isRegistered: vi.fn(async () => false),
      register: vi.fn(async () => { throw new Error("installation limit"); }),
      revokeAllOtherInstallations: vi.fn(),
      close: vi.fn(),
    };
    await expect(recoverRegisteredClient(async () => failed)).rejects.toThrow("installation limit");
    expect(failed.close).toHaveBeenCalledOnce();
  });

  it("revokes stale installations once when a new installation finds the inbox full", async () => {
    const client = {
      isRegistered: vi.fn(async () => false),
      register: vi.fn()
        .mockRejectedValueOnce(new Error("InboxID abc has already registered 10/10 installations. Please revoke existing installations first."))
        .mockResolvedValueOnce(undefined),
      revokeAllOtherInstallations: vi.fn(async () => undefined),
      close: vi.fn(),
    };
    await expect(recoverRegisteredClient(async () => client)).resolves.toBe(client);
    expect(client.revokeAllOtherInstallations).toHaveBeenCalledOnce();
    expect(client.register).toHaveBeenCalledTimes(2);
  });

  it("registers the Acolyte inbox without opening a conversation and closes the client", async () => {
    let occupied = false;
    const request = vi.fn(async (_name: string, _options: LockOptions, callback: (lock: Lock | null) => Promise<void>) => {
      if (occupied) return callback(null);
      occupied = true;
      try {
        return await callback({} as Lock);
      } finally {
        occupied = false;
      }
    });
    vi.stubGlobal("navigator", { ...navigator, locks: { request } });
    const client = {
      inboxId: own,
      isRegistered: vi.fn(async () => false),
      register: vi.fn(async () => undefined),
      revokeAllOtherInstallations: vi.fn(async () => undefined),
      close: vi.fn(),
      conversations: { createDmWithIdentifier: vi.fn(), createDm: vi.fn() },
    };
    const create = vi.spyOn(Client, "create").mockResolvedValue(client as never);

    await expect(ensureXmtpIdentityRegistration(config, identity)).resolves.toBe(own);
    expect(client.register).toHaveBeenCalledOnce();
    expect(client.conversations.createDmWithIdentifier).not.toHaveBeenCalled();
    expect(client.conversations.createDm).not.toHaveBeenCalled();
    expect(client.close).toHaveBeenCalledOnce();
    expect(occupied).toBe(false);
    const reacquired = await acquireXmtpDatabaseLease("production", identity.address);
    await reacquired();
    create.mockRestore();
  });

  it("fails before OPFS open when another tab owns the identity database lease", async () => {
    let occupied = false;
    const request = vi.fn(async (_name: string, _options: LockOptions, callback: (lock: Lock | null) => Promise<void>) => {
      if (occupied) return callback(null);
      occupied = true;
      try {
        return await callback({} as Lock);
      } finally {
        occupied = false;
      }
    });
    vi.stubGlobal("navigator", { ...navigator, locks: { request } });
    const release = await acquireXmtpDatabaseLease("production", identity.address);
    await expect(acquireXmtpDatabaseLease("production", identity.address)).rejects.toThrow(/another tab/u);
    await release();
    const reacquired = await acquireXmtpDatabaseLease("production", identity.address);
    await reacquired();
  });

  it("keeps unconfigured deployment on legacy intro Direct without inventing group bindings", async () => {
    const value = fixture();
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => intro),
      storage: localStorage,
    });
    await startVerified(workspace);
    expect(value.client.conversations.createDmWithIdentifier).toHaveBeenCalledWith(
      { identifier: address, identifierKind: IdentifierKind.Ethereum },
      { messageDisappearingSettings: { fromNs: 1n, inNs: 1_209_600_000_000_000n } },
    );
    expect(value.client.conversations.createDm).not.toHaveBeenCalled();
    expect(value.client.preferences.fetchInboxStates).toHaveBeenCalled();
    expect(value.sentControls).toEqual([]);
    expect(workspace.snapshot().channels.direct.retentionVerified).toBe(true);
    expect(workspace.snapshot().channels.acolytes.status).toBe("awaiting-assignment");
    expect(workspace.snapshot().channels.global.readConversationIds).toEqual([]);
    await workspace.close();
  });

  it("keeps a fresh Unminted explicit Tentacle link Direct-only without sending an ordinary join", async () => {
    const value = fixture();
    let resolved = anchored;
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => resolved),
      storage: localStorage,
    });
    await startVerified(workspace);
    expect(value.client.conversations.createDmWithIdentifier).toHaveBeenCalledWith(
      { identifier: address, identifierKind: IdentifierKind.Ethereum },
      { messageDisappearingSettings: { fromNs: 1n, inNs: 1_209_600_000_000_000n } },
    );
    expect(value.sentControls).toEqual([]);
    expect(workspace.snapshot().channels.direct.retentionVerified).toBe(true);
    expect(workspace.snapshot().channels.acolytes).toMatchObject({
      status: "policy-blocked",
      error: expect.stringMatching(/Direct-only.*Acolytes/u),
    });
    expect(workspace.snapshot().channels.global).toMatchObject({
      status: "policy-blocked",
      error: expect.stringMatching(/Direct-only.*Global/u),
    });
    resolved = { ...anchored, name: "Freshly Renamed Tentacle" };
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot().tentacleName).toBe("Freshly Renamed Tentacle");
    expect(value.sentControls).toEqual([]);
    await workspace.close();
  });

  it("retries referral attribution after restart until the authenticated Tentacle ACKs it", async () => {
    const referrer = "0x3333333333333333333333333333333333333333";
    const referredConfig = { ...config, referrer };
    const first = fixture();
    const workspace = new XmtpMultiChannelWorkspace(
      first.client as never,
      referredConfig,
      identity,
      {
        resolveAssignment: vi.fn(async () => anchored),
        storage: localStorage,
      },
    );
    await startVerified(workspace);
    expect(first.direct.sendText).toHaveBeenCalledWith(
      `[[cthuwu:referral-attribution:v1;referrer=${referrer}]]`,
    );
    await workspace.revalidateAssignment("retry");
    expect(first.direct.sendText).toHaveBeenCalledTimes(1);
    await workspace.close();

    const unacknowledged = fixture();
    const recovered = new XmtpMultiChannelWorkspace(
      unacknowledged.client as never,
      referredConfig,
      identity,
      {
        resolveAssignment: vi.fn(async () => anchored),
        storage: localStorage,
      },
    );
    await startVerified(recovered);
    expect(unacknowledged.direct.sendText).toHaveBeenCalledWith(
      `[[cthuwu:referral-attribution:v1;referrer=${referrer}]]`,
    );
    unacknowledged.streams.messages.push({
      ...textMessage("referral-ack", directId, 1_700_000_000_000_000_001n),
      content: `[[cthuwu:referral-attribution-ack:v1;status=accepted;referrer=${referrer}]]`,
    });
    const acknowledgementKey =
      `cthuwu.referral-ack.v1:production:${identity.address.toLowerCase()}:${tentacle}`;
    await vi.waitFor(() => expect(localStorage.getItem(acknowledgementKey)).toBe("acknowledged"));
    await recovered.close();

    const acknowledged = fixture();
    const finalRestart = new XmtpMultiChannelWorkspace(
      acknowledged.client as never,
      referredConfig,
      identity,
      {
        resolveAssignment: vi.fn(async () => anchored),
        storage: localStorage,
      },
    );
    await startVerified(finalRestart);
    expect(acknowledged.direct.sendText).not.toHaveBeenCalled();
    await finalRestart.close();
  });

  it("probes explicit deep-linked tentacle with liveness query before connecting as anchor-verified", async () => {
    const value = fixture();
    const anchorConfig = { ...config, tentacleAnchor: address };
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, anchorConfig, identity, {
      resolveAssignment: vi.fn(async () => anchored),
      verifyLivenessCandidate: vi.fn(async () => rotated),
      livenessWindowMs: 200,
      nowMs: () => 1_800_000_000_000,
      storage: localStorage,
    });
    const starting = startVerified(workspace);
    await waitFor(() => value.sentControls.length === 1);
    const query = livenessQueryCodec.decode(value.sentControls[0] as never) as {
      requestId: string;
    };
    value.streams.probe.push({
      ...textMessage("alive", directId, 1_800_000_000_001_000_000n),
      contentType: LIVENESS_RESPONSE_CONTENT_TYPE,
      content: {
        type: "cthuwu.liveness-response.v1",
        requestId: query.requestId,
        environment: "production",
        phrase: "fhtagn!",
        tentacleAgentId: "42",
      },
    });
    await starting;
    expect(workspace.snapshot().assignmentState).toBe("anchor-verified");
    expect(workspace.snapshot().channels.direct.retentionVerified).toBe(true);
    await workspace.close();
  });

  it("warns when a deep-linked Tentacle does not answer ping pong while keeping Direct usable", async () => {
    const value = fixture();
    const anchorConfig = { ...config, tentacleAnchor: address };
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, anchorConfig, identity, {
      resolveAssignment: vi.fn(async () => anchored),
      livenessWindowMs: 20,
      nowMs: () => 1_800_000_000_000,
      storage: localStorage,
    });
    await startVerified(workspace);
    expect(workspace.snapshot()).toMatchObject({
      assignmentState: "liveness-unavailable",
      verificationWarning: expect.stringMatching(/The deep-linked Tentacle did not answer the liveness check/u),
    });
    expect(workspace.snapshot().channels.direct.retentionVerified).toBe(true);
    await workspace.close();
  });

  it("does not persist a snapshot-only rotation without a successful liveness join", async () => {
    const value = fixture();
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => rotated),
      storage: localStorage,
    });
    await startVerified(workspace);
    expect(localStorage.getItem(`cthuwu.rotation.v1:production:${identity.address}`)).toBeNull();
    await workspace.close();
  });

  it("races an exact non-push liveness query, verifies the first response, persists after join, and replaces a stale cache name", async () => {
    const value = fixture();
    const candidate = {
      wallet: address,
      agentId: "42",
      inboxId: tentacle,
      name: "Stale Cached Name",
      rank: 1,
      blockNumber: "122",
      blockHash: `0x${"8".repeat(64)}`,
    };
    const resolveAssignment = vi.fn(async (current: AppConfig) =>
      current.rotationAnchor ? rotated : livenessRequired);
    const verifyCandidate = vi.fn(async () => rotated);
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment,
      loadLivenessCandidates: vi.fn(async () => [candidate]),
      verifyLivenessCandidate: verifyCandidate,
      livenessWindowMs: 200,
      nowMs: () => 1_800_000_000_000,
      storage: localStorage,
    });
    const starting = startVerified(workspace);
    await waitFor(() => value.sentControls.length === 1);
    const query = livenessQueryCodec.decode(value.sentControls[0] as never) as {
      requestId: string; expiresAtNs: string; targetAgentId: string;
    };
    expect(query).toMatchObject({
      targetAgentId: "42",
      expiresAtNs: "1800000000200000000",
    });
    expect(value.direct.send).toHaveBeenNthCalledWith(1, value.sentControls[0], { shouldPush: false });
    value.streams.probe.push({
      ...textMessage("alive", directId, 1_800_000_000_001_000_000n),
      contentType: LIVENESS_RESPONSE_CONTENT_TYPE,
      content: {
        type: "cthuwu.liveness-response.v1",
        requestId: query.requestId,
        environment: "production",
        phrase: "fhtagn!",
        tentacleAgentId: "42",
      },
    });
    await starting;
    expect(verifyCandidate).toHaveBeenCalledWith(config, identity, candidate);
    expect(workspace.snapshot().tentacleName).toBe(rotated.name);
    expect(localStorage.getItem(`cthuwu.rotation.v1:production:${identity.address}`)).toBe(address);
    expect(value.sentControls).toHaveLength(2);
    const firstJoin = livenessJoinCodec.decode(value.sentControls[1] as never) as {
      requestId: string; livenessRequestId: string;
    };
    expect(firstJoin.livenessRequestId).toBe(query.requestId);

    await workspace.revalidateAssignment("retry");
    const secondJoin = livenessJoinCodec.decode(value.sentControls[2] as never) as {
      requestId: string; livenessRequestId: string;
    };
    expect(secondJoin.livenessRequestId).toBe(query.requestId);
    expect(secondJoin.requestId).not.toBe(firstJoin.requestId);

    value.streams.messages.push({
      ...textMessage("assignment-live", directId, 3n),
      contentType: ASSIGNMENT_CONTENT_TYPE,
      content: assignment(secondJoin.requestId),
    } as never);
    await waitFor(() => workspace.snapshot().channels.acolytes.status !== "awaiting-assignment");
    await workspace.revalidateAssignment("retry");
    expect(isJoinControl(joinCodec.decode(value.sentControls[3] as never))).toBe(true);
    await workspace.close();
  });

  it("prepares the DM stream before starting the response window and never publishes from an expired setup timer", async () => {
    const value = fixture();
    const candidate = {
      wallet: address,
      agentId: "42",
      inboxId: tentacle,
      name: "Vhoorl-of-the-Patient-Tide",
      rank: 1,
      blockNumber: "122",
      blockHash: `0x${"8".repeat(64)}`,
    };
    let releaseStream!: (stream: FakeStream<unknown>) => void;
    value.direct.stream.mockImplementationOnce(async () =>
      new Promise<FakeStream<unknown>>((resolve) => { releaseStream = resolve; }));
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => livenessRequired),
      loadLivenessCandidates: vi.fn(async () => [candidate]),
      verifyLivenessCandidate: vi.fn(async () => rotated),
      livenessWindowMs: 250,
      nowMs: () => 1_800_000_000_000,
      storage: localStorage,
    });

    const starting = startVerified(workspace);
    await waitFor(() => value.direct.stream.mock.calls.length === 1);
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(value.sentControls).toEqual([]);

    releaseStream(value.streams.probe);
    await waitFor(() => value.sentControls.length === 1);
    const query = livenessQueryCodec.decode(value.sentControls[0] as never) as {
      requestId: string; expiresAtNs: string;
    };
    await new Promise((resolve) => setTimeout(resolve, 150));
    value.streams.probe.push({
      ...textMessage("alive-after-setup", directId, BigInt(query.expiresAtNs) - 1n),
      contentType: LIVENESS_RESPONSE_CONTENT_TYPE,
      content: {
        type: "cthuwu.liveness-response.v1",
        requestId: query.requestId,
        environment: "production",
        phrase: "fhtagn!",
        tentacleAgentId: "42",
      },
    });

    await starting;
    expect(workspace.snapshot().assignmentState).toBe("rotation-verified");
    await workspace.close();
  });

  it("bounds a stuck preparation and closes a stream that arrives after cancellation without publishing", async () => {
    const value = fixture();
    const candidate = {
      wallet: address,
      agentId: "42",
      inboxId: tentacle,
      name: "Vhoorl-of-the-Lost-Stream",
      rank: 1,
      blockNumber: "122",
      blockHash: `0x${"8".repeat(64)}`,
    };
    const lateStream = new FakeStream<unknown>();
    const closeLateStream = vi.spyOn(lateStream, "return");
    let releaseStream!: (stream: FakeStream<unknown>) => void;
    value.direct.stream.mockImplementationOnce(async () =>
      new Promise<FakeStream<unknown>>((resolve) => { releaseStream = resolve; }));
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => livenessRequired),
      loadLivenessCandidates: vi.fn(async () => [candidate]),
      verifyLivenessCandidate: vi.fn(async () => rotated),
      livenessWindowMs: 20,
      storage: localStorage,
    });

    await startVerified(workspace);
    expect(workspace.snapshot().assignmentState).toBe("registry-unavailable");
    expect(value.sentControls).toEqual([]);

    releaseStream(lateStream);
    await waitFor(() => closeLateStream.mock.calls.length === 1);
    expect(value.sentControls).toEqual([]);
    await workspace.close();
  });

  it("redacts endpoints and full inbox IDs from candidate failure diagnostics", async () => {
    const value = fixture();
    const candidate = {
      wallet: address,
      agentId: "42",
      inboxId: tentacle,
      name: "Vhoorl-of-the-Quiet-Log",
      rank: 1,
      blockNumber: "122",
      blockHash: `0x${"8".repeat(64)}`,
    };
    value.client.preferences.fetchInboxStates.mockResolvedValueOnce([
      { inboxId: tentacle, recoveryIdentifier: { identifier: address, identifierKind: IdentifierKind.Ethereum }, installations: [], accountIdentifiers: [{ identifier: address, identifierKind: IdentifierKind.Ethereum }] },
    ]).mockRejectedValueOnce(
      new Error(`failed https://rpc.example/private-key for inbox ${tentacle}`),
    );
    const logger = {
      debug: vi.fn(),
      info: vi.fn(),
      warn: vi.fn(),
    };
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => livenessRequired),
      loadLivenessCandidates: vi.fn(async () => [candidate]),
      verifyLivenessCandidate: vi.fn(async () => rotated),
      livenessWindowMs: 20,
      logger,
      storage: localStorage,
    });

    await startVerified(workspace);
    const diagnostics = JSON.stringify(logger.warn.mock.calls);
    expect(diagnostics).toContain("<redacted-endpoint>");
    expect(diagnostics).toContain("<redacted-id>");
    expect(diagnostics).not.toContain("rpc.example/private-key");
    expect(diagnostics).not.toContain(tentacle);
    await workspace.close();
  });

  it("times out a stuck query publish while its expired late send remains inert", async () => {
    const value = fixture();
    const candidate = {
      wallet: address,
      agentId: "42",
      inboxId: tentacle,
      name: "Vhoorl-of-the-Stalled-Query",
      rank: 1,
      blockNumber: "122",
      blockHash: `0x${"8".repeat(64)}`,
    };
    let releaseSend!: () => void;
    value.direct.send.mockImplementationOnce(async () =>
      new Promise<void>((resolve) => { releaseSend = resolve; }));
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => livenessRequired),
      loadLivenessCandidates: vi.fn(async () => [candidate]),
      verifyLivenessCandidate: vi.fn(async () => rotated),
      livenessWindowMs: 20,
      storage: localStorage,
    });

    const starting = startVerified(workspace);
    await waitFor(() => value.direct.send.mock.calls.length === 1);
    await starting;
    expect(workspace.snapshot()).toMatchObject({
      assignmentState: "liveness-unavailable",
      verificationWarning: expect.stringMatching(/0\/1 probes sent/u),
    });

    releaseSend();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(workspace.snapshot().assignmentState).toBe("liveness-unavailable");
    await workspace.close();
  });

  it("accepts a correctly bound response when the responder clock is behind the browser", async () => {
    const value = fixture();
    const candidate = {
      wallet: address,
      agentId: "42",
      inboxId: tentacle,
      name: "Vhoorl-of-the-Clockless-Tide",
      rank: 1,
      blockNumber: "122",
      blockHash: `0x${"8".repeat(64)}`,
    };
    const verifyCandidate = vi.fn(async () => rotated);
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => livenessRequired),
      loadLivenessCandidates: vi.fn(async () => [candidate]),
      verifyLivenessCandidate: verifyCandidate,
      livenessWindowMs: 200,
      nowMs: () => 1_800_000_000_000,
      storage: localStorage,
    });

    const starting = startVerified(workspace);
    await waitFor(() => value.sentControls.length === 1);
    const query = livenessQueryCodec.decode(value.sentControls[0] as never) as {
      requestId: string;
    };
    value.streams.probe.push({
      ...textMessage("clock-skewed-alive", directId, 1_799_999_995_000_000_000n),
      contentType: LIVENESS_RESPONSE_CONTENT_TYPE,
      content: {
        type: "cthuwu.liveness-response.v1",
        requestId: query.requestId,
        environment: "production",
        phrase: "fhtagn!",
        tentacleAgentId: "42",
      },
    });

    await starting;
    expect(verifyCandidate).toHaveBeenCalledOnce();
    expect(workspace.snapshot().assignmentState).toBe("rotation-verified");
    await workspace.close();
  });

  it("accepts a correctly bound response when the responder clock is ahead of the browser", async () => {
    const value = fixture();
    const candidate = {
      wallet: address,
      agentId: "42",
      inboxId: tentacle,
      name: "Vhoorl-of-the-Future-Tide",
      rank: 1,
      blockNumber: "122",
      blockHash: `0x${"8".repeat(64)}`,
    };
    const verifyCandidate = vi.fn(async () => rotated);
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => livenessRequired),
      loadLivenessCandidates: vi.fn(async () => [candidate]),
      verifyLivenessCandidate: verifyCandidate,
      livenessWindowMs: 200,
      nowMs: () => 1_800_000_000_000,
      storage: localStorage,
    });

    const starting = startVerified(workspace);
    await waitFor(() => value.sentControls.length === 1);
    const query = livenessQueryCodec.decode(value.sentControls[0] as never) as {
      requestId: string; expiresAtNs: string;
    };
    value.streams.probe.push({
      ...textMessage(
        "clock-skewed-future-alive",
        directId,
        BigInt(query.expiresAtNs) + 5_000_000_000n,
      ),
      contentType: LIVENESS_RESPONSE_CONTENT_TYPE,
      content: {
        type: "cthuwu.liveness-response.v1",
        requestId: query.requestId,
        environment: "production",
        phrase: "fhtagn!",
        tentacleAgentId: "42",
      },
    });

    await starting;
    expect(verifyCandidate).toHaveBeenCalledOnce();
    expect(workspace.snapshot().assignmentState).toBe("rotation-verified");
    await workspace.close();
  });

  it("keeps a no-response liveness warning explicit while retaining the initial Direct route", async () => {
    const value = fixture();
    const candidate = {
      wallet: address,
      agentId: "42",
      inboxId: tentacle,
      name: "Vhoorl-of-the-Quiet-Tide",
      rank: 1,
      blockNumber: "122",
      blockHash: `0x${"8".repeat(64)}`,
    };
    const verifyCandidate = vi.fn(async () => rotated);
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => livenessRequired),
      loadLivenessCandidates: vi.fn(async () => [candidate]),
      verifyLivenessCandidate: verifyCandidate,
      livenessWindowMs: 20,
      nowMs: () => 1_800_000_000_000,
      storage: localStorage,
    });

    const starting = startVerified(workspace);
    await waitFor(() => value.sentControls.length === 1);
    value.streams.probe.push(
      textMessage("ordinary-probe-dm-traffic", directId, 1_800_000_000_001_000_000n),
    );
    await starting;
    expect(workspace.snapshot()).toMatchObject({
      assignmentState: "liveness-unavailable",
      verificationWarning: expect.stringMatching(
        /No ranked Tentacle answered.*1\/1 probes sent; 0 responses observed.*current sidecar/u,
      ),
    });
    expect(value.sentControls).toHaveLength(1);
    expect(verifyCandidate).not.toHaveBeenCalled();
    expect(value.client.conversations.createDmWithIdentifier).toHaveBeenCalledTimes(1);
    expect(localStorage.getItem(`cthuwu.rotation.v1:production:${identity.address}`)).toBeNull();

    await workspace.revalidateAssignment("retry");
    expect(value.sentControls).toHaveLength(1);
    expect(value.client.conversations.createDmWithIdentifier).toHaveBeenCalledTimes(1);
    await workspace.close();
  });

  it("allows only an explicit retry to initiate the first probe after a pre-probe failure", async () => {
    const value = fixture();
    const candidate = {
      wallet: address,
      agentId: "42",
      inboxId: tentacle,
      name: "Vhoorl-of-the-Quiet-Tide",
      rank: 1,
      blockNumber: "122",
      blockHash: `0x${"8".repeat(64)}`,
    };
    const loadCandidates = vi.fn()
      .mockRejectedValueOnce(new Error("directory offline"))
      .mockResolvedValue([candidate]);
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => livenessRequired),
      loadLivenessCandidates: loadCandidates,
      verifyLivenessCandidate: vi.fn(async () => rotated),
      livenessWindowMs: 20,
      nowMs: () => 1_800_000_000_000,
      storage: localStorage,
    });

    await expect(startVerified(workspace)).rejects.toThrow(/directory offline/u);
    await workspace.revalidateAssignment("periodic");
    await workspace.revalidateAssignment("resume");
    expect(loadCandidates).toHaveBeenCalledTimes(1);
    expect(value.sentControls).toHaveLength(0);

    await workspace.revalidateAssignment("retry");
    expect(loadCandidates).toHaveBeenCalledTimes(2);
    expect(value.sentControls).toHaveLength(1);
    await workspace.close();
  });

  it("rejects a liveness response received after the browser's local deadline", async () => {
    const value = fixture();
    const candidate = {
      wallet: address,
      agentId: "42",
      inboxId: tentacle,
      name: "Vhoorl-of-the-Quiet-Tide",
      rank: 1,
      blockNumber: "122",
      blockHash: `0x${"8".repeat(64)}`,
    };
    let now = 1_800_000_000_000;
    const verifyCandidate = vi.fn(async () => rotated);
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => livenessRequired),
      loadLivenessCandidates: vi.fn(async () => [candidate]),
      verifyLivenessCandidate: verifyCandidate,
      livenessWindowMs: 200,
      nowMs: () => now,
      storage: localStorage,
    });
    const starting = startVerified(workspace);
    await waitFor(() => value.sentControls.length === 1);
    const query = livenessQueryCodec.decode(value.sentControls[0] as never) as {
      requestId: string;
    };
    now += 201;
    value.streams.probe.push({
      ...textMessage("received-late", directId, 1_800_000_000_000_000_000n),
      contentType: LIVENESS_RESPONSE_CONTENT_TYPE,
      content: {
        type: "cthuwu.liveness-response.v1",
        requestId: query.requestId,
        environment: "production",
        phrase: "fhtagn!",
        tentacleAgentId: "42",
      },
    });
    await starting;
    expect(verifyCandidate).not.toHaveBeenCalled();
    expect(workspace.snapshot()).toMatchObject({
      assignmentState: "liveness-unavailable",
      verificationWarning: expect.stringMatching(/1 response observed/u),
    });
    await workspace.close();
  });

  it("accepts an authenticated later revision, routes trusted IDs, paginates, and deletes", async () => {
    const value = fixture();
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => configuredFallback),
      storage: localStorage,
    });
    await startVerified(workspace);
    expect(value.client.conversations.streamAllMessages.mock.invocationCallOrder[0]).toBeLessThan(
      value.direct.send.mock.invocationCallOrder[0]!,
    );
    const join = joinCodec.decode(value.sentControls[0] as never);
    if (!isJoinControl(join)) throw new Error("join control did not decode");
    value.streams.messages.push({
      ...textMessage("control", directId, 2n),
      contentType: ASSIGNMENT_CONTENT_TYPE,
      content: assignment(join.requestId),
    } as never);
    await vi.waitFor(() => expect(workspace.snapshot().channels.global).toMatchObject({
      retentionVerified: true,
    }));
    expect(value.acolytes.updateConsentState).toHaveBeenCalledWith(ConsentState.Allowed);
    expect(value.global.updateConsentState).toHaveBeenCalledWith(ConsentState.Allowed);
    await expect(value.acolytes.consentState()).resolves.toBe(ConsentState.Allowed);
    await expect(value.global.consentState()).resolves.toBe(ConsentState.Allowed);
    expect(workspace.snapshot().channels.global.readConversationIds).toEqual([globalId]);
    expect(workspace.snapshot().channels.global.writeConversationId).toBe(globalId);
    expect(workspace.snapshot().channels.global.messages).toHaveLength(40);

    value.streams.messages.push({
      ...textMessage("typing", directId, 1_400n),
      contentType: { authorityId: "cthuwu.app", typeId: "typing", versionMajor: 1, versionMinor: 0 },
      content: {
        type: "cthuwu.typing.v1",
        active: true,
        expiresAtNs: (BigInt(Date.now() + 15_000) * 1_000_000n).toString(),
      },
    } as never);
    await vi.waitFor(() => expect(workspace.snapshot().channels.direct.typing).toBe(true));
    expect(workspace.snapshot().channels.direct.messages.some(({ id }) => id === "typing")).toBe(false);

    value.streams.messages.push(textMessage("direct-in", directId, 1_500n));
    value.streams.messages.push(textMessage("acolytes-in", acolytesId, 1_501n));
    value.streams.messages.push(textMessage("global-in", globalId, 1_502n));
    await waitFor(() =>
      workspace.snapshot().channels.direct.messages.some(({ id }) => id === "direct-in") &&
      workspace.snapshot().channels.acolytes.messages.some(({ id }) => id === "acolytes-in") &&
      workspace.snapshot().channels.global.messages.some(({ id }) => id === "global-in"));
    expect(workspace.snapshot().channels.direct.typing).toBe(false);
    await workspace.send("direct", "direct out");
    await workspace.send("acolytes", "acolytes out");
    await workspace.send("global", "global out");
    expect(value.direct.sendText).toHaveBeenCalledWith("direct out");
    expect(value.acolytes.sendText).toHaveBeenCalledWith("acolytes out");
    expect(value.global.sendText).toHaveBeenCalledWith("global out");

    workspace.setActiveChannel("global");
    workspace.setActiveChannel("acolytes");
    value.streams.messages.push(textMessage("trusted", globalId, 2_000n));
    value.streams.messages.push(textMessage("spoofed", "9".repeat(64), 2_001n));
    await vi.waitFor(() => expect(workspace.snapshot().channels.global).toMatchObject({
      unread: 1,
    }));
    expect(workspace.snapshot().channels.global.messages.some(({ id }) => id === "spoofed")).toBe(false);

    await workspace.loadEarlier("global");
    expect(workspace.snapshot().channels.global.messages).toHaveLength(52);
    value.streams.deleted.push(textMessage("trusted", globalId, 2_000n));
    await waitFor(() => !workspace.snapshot().channels.global.messages.some(({ id }) => id === "trusted"));
    expect(workspace.snapshot().channels.global.unread).toBe(0);
    await workspace.close();
  });

  it("freshly enforces retention before exact-conversation send and reconnects all streams on resume", async () => {
    const value = fixture();
    const resolver = vi.fn(async () => configuredFallback);
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: resolver,
      storage: localStorage,
    });
    await startVerified(workspace);
    const join = joinCodec.decode(value.sentControls[0] as never);
    if (!isJoinControl(join)) throw new Error("join control did not decode");
    value.streams.messages.push({
      ...textMessage("control", directId, 2n),
      contentType: ASSIGNMENT_CONTENT_TYPE,
      content: assignment(join.requestId),
    } as never);
    await waitFor(() => workspace.snapshot().channels.global.retentionVerified);
    await workspace.send("global", "hello global");
    expect(value.global.sendText).toHaveBeenCalledWith("hello global");
    value.breakGlobalPolicy();
    await expect(workspace.send("global", "unsafe")).rejects.toThrow(/policy/u);
    expect(workspace.snapshot().channels.global.status).toBe("policy-blocked");
    value.restoreGlobalPolicy();

    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot().channels.acolytes).toMatchObject({
      retentionVerified: true,
      status: expect.stringMatching(/^(?:empty|ready)$/u),
    });
    expect(workspace.snapshot().channels.global).toMatchObject({
      retentionVerified: true,
      status: expect.stringMatching(/^(?:empty|ready)$/u),
    });

    value.allMessageOptions()?.onFail?.();
    expect(workspace.snapshot().connected).toBe(false);
    await workspace.revalidateAssignment("resume");
    expect(value.client.conversations.streamAllMessages).toHaveBeenCalledTimes(2);
    expect(value.client.conversations.streamGroups).toHaveBeenCalledTimes(2);
    expect(value.client.conversations.streamDeletedMessages).toHaveBeenCalledTimes(2);
    expect(value.client.conversations.sync).toHaveBeenCalled();
    expect(workspace.snapshot().connected).toBe(true);
    await Promise.resolve();
    await Promise.resolve();
    expect(workspace.snapshot().connected).toBe(true);
    expect(value.streamHistory.messages).toHaveLength(2);
    expect(value.streamHistory.groups).toHaveLength(2);
    expect(value.streamHistory.deleted).toHaveLength(2);
    await workspace.close();
  });

  it("starts and sends while Base is pending, then keeps Direct usable after RPC failure", async () => {
    const value = fixture();
    let rejectCheck!: (error: Error) => void;
    const check = new Promise<TentacleAssignment>((_resolve, reject) => { rejectCheck = reject; });
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: () => check,
    });
    await workspace.start();
    expect(workspace.snapshot().channels.direct.retentionVerified).toBe(true);
    expect(workspace.snapshot().tentacleName).toContain("unverified");
    await workspace.send("direct", "hello before Base replies");
    expect(value.direct.sendText).toHaveBeenCalledWith("hello before Base replies");
    rejectCheck(new RegistryUnavailableError("Base RPC offline"));
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot().verificationWarning).toContain("Base RPC offline");
    await workspace.send("direct", "still connected");
    expect(value.direct.sendText).toHaveBeenCalledWith("still connected");
    expect(value.sentControls).toEqual([]);
    await workspace.close();
  });

  it("does not redirect an open chat when a late Base result names a different target", async () => {
    const value = fixture();
    let finish!: (assignment: TentacleAssignment) => void;
    const check = new Promise<TentacleAssignment>((resolve) => { finish = resolve; });
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: () => check,
    });
    await workspace.start();
    finish({ ...rotated, address: "0x9999999999999999999999999999999999999999" });
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot().assignedTentacleAddress).toBe(address);
    expect(workspace.snapshot().verificationWarning).toContain("no messages were moved");
    expect(value.client.conversations.createDm).not.toHaveBeenCalled();
    await workspace.send("direct", "same recipient");
    await workspace.close();
  });

  it("ignores a Base result delivered after the workspace closes", async () => {
    const value = fixture();
    let finish!: (assignment: TentacleAssignment) => void;
    const check = new Promise<TentacleAssignment>((resolve) => { finish = resolve; });
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: () => check,
    });
    await workspace.start();
    await workspace.close();
    finish(rotated);
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot().connected).toBe(false);
    expect(value.sentControls).toEqual([]);
    expect(value.client.conversations.createDm).not.toHaveBeenCalled();
  });

  it("backs off automatic registry retries after an outage while preserving explicit retry", async () => {
    const value = fixture();
    const resolver = vi.fn(async () => { throw new RegistryUnavailableError("Base RPC rate limited"); });
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: resolver,
      storage: localStorage,
    });
    await startVerified(workspace);
    expect(resolver).toHaveBeenCalledTimes(1);
    await workspace.revalidateAssignment("resume");
    await workspace.revalidateAssignment("periodic");
    expect(resolver).toHaveBeenCalledTimes(1);
    await workspace.revalidateAssignment("retry");
    expect(resolver).toHaveBeenCalledTimes(2);
    await workspace.close();
  });

  it("blocks drifted Acolytes independently while Global stays ready and sendable", async () => {
    const value = fixture();
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => configuredFallback),
      storage: localStorage,
    });
    await startVerified(workspace);
    const join = joinCodec.decode(value.sentControls[0] as never);
    if (!isJoinControl(join)) throw new Error("join control did not decode");
    value.streams.messages.push({
      ...textMessage("control", directId, 2n),
      contentType: ASSIGNMENT_CONTENT_TYPE,
      content: assignment(join.requestId),
    } as never);
    await waitFor(() => workspace.snapshot().channels.global.retentionVerified);

    value.breakAcolytesPolicy();
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot().channels.acolytes).toMatchObject({
      retentionVerified: false,
      status: "policy-blocked",
    });
    expect(workspace.snapshot().channels.global).toMatchObject({
      retentionVerified: true,
      status: expect.stringMatching(/^(?:empty|ready)$/u),
    });
    await workspace.send("global", "Global remains healthy");
    expect(value.global.sendText).toHaveBeenCalledWith("Global remains healthy");
    value.streams.messages.push(textMessage("blocked-acolytes-in", acolytesId, 2_001n));
    await waitFor(() => workspace.snapshot().channels.acolytes.messages.some(
      ({ id }) => id === "blocked-acolytes-in",
    ));
    expect(workspace.snapshot().channels.acolytes.status).toBe("policy-blocked");
    await expect(workspace.send("acolytes", "must stay blocked")).rejects.toThrow(/not ready/u);
    expect(workspace.snapshot().channels.global.retentionVerified).toBe(true);
    await workspace.close();
  });

  it("fresh-verifies unchanged Direct wallet binding and repairs retention before recovery", async () => {
    const value = fixture();
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => configuredFallback),
      storage: localStorage,
    });
    await startVerified(workspace);
    const join = joinCodec.decode(value.sentControls[0] as never);
    if (!isJoinControl(join)) throw new Error("join control did not decode");
    value.streams.messages.push({
      ...textMessage("control", directId, 2n),
      contentType: ASSIGNMENT_CONTENT_TYPE,
      content: assignment(join.requestId),
    } as never);
    await waitFor(() => workspace.snapshot().channels.global.retentionVerified);

    value.breakDirectPolicy();
    value.direct.updateMessageDisappearingSettings.mockRejectedValueOnce(new Error("repair failed"));
    await expect(workspace.send("direct", "blocked until repair")).rejects.toThrow(/repair failed/u);
    expect(workspace.snapshot().channels.direct).toMatchObject({
      retentionVerified: false,
      status: "policy-blocked",
    });
    await workspace.revalidateAssignment("retry");
    expect(value.direct.updateMessageDisappearingSettings).toHaveBeenCalledWith(
      1n,
      1_209_600_000_000_000n,
    );
    expect(workspace.snapshot().channels.direct).toMatchObject({
      retentionVerified: true,
      status: expect.stringMatching(/^(?:empty|ready)$/u),
    });

    value.setPeerAddress("0x9999999999999999999999999999999999999999");
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot()).toMatchObject({
      assignmentState: "direct-verification-unavailable",
      channels: {
        direct: { retentionVerified: false, status: "policy-blocked" },
        acolytes: { retentionVerified: false, status: "policy-blocked" },
        global: { retentionVerified: true },
      },
    });

    value.setPeerAddress(address);
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot().channels.direct).toMatchObject({
      retentionVerified: true,
      status: expect.stringMatching(/^(?:empty|ready)$/u),
    });
    expect(workspace.snapshot().channels.acolytes).toMatchObject({
      retentionVerified: true,
      status: expect.stringMatching(/^(?:empty|ready)$/u),
    });

    value.client.conversations.sync.mockRejectedValueOnce(new Error("conversation sync offline"));
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot()).toMatchObject({
      assignmentState: "direct-verification-unavailable",
      channels: {
        direct: { retentionVerified: false, status: "policy-blocked" },
        acolytes: { retentionVerified: false, status: "policy-blocked" },
        global: { retentionVerified: true },
      },
    });
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot().channels.direct.retentionVerified).toBe(true);
    expect(workspace.snapshot().channels.acolytes.retentionVerified).toBe(true);

    value.client.preferences.fetchInboxStates.mockRejectedValueOnce(new Error("inbox RPC offline"));
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot()).toMatchObject({
      assignmentState: "direct-verification-unavailable",
      channels: {
        direct: { retentionVerified: false, status: "policy-blocked" },
        acolytes: { retentionVerified: false, status: "policy-blocked" },
      },
    });
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot().channels.direct.retentionVerified).toBe(true);
    expect(workspace.snapshot().channels.acolytes.retentionVerified).toBe(true);

    value.direct.consentState.mockRejectedValueOnce(new Error("consent RPC offline"));
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot()).toMatchObject({
      assignmentState: "direct-verification-unavailable",
      channels: {
        direct: { retentionVerified: false, status: "policy-blocked" },
        acolytes: { retentionVerified: false, status: "policy-blocked" },
      },
    });
    await workspace.revalidateAssignment("retry");
    expect(workspace.snapshot().channels.direct.retentionVerified).toBe(true);
    expect(workspace.snapshot().channels.acolytes.retentionVerified).toBe(true);
    expect(value.client.preferences.fetchInboxStates.mock.calls.length).toBeGreaterThanOrEqual(3);
    await workspace.close();
  });

  it("restores same-timestamp unread IDs and recomputes them after deletion", async () => {
    createChatUiStateStore(localStorage).setReadAt("global", 1_000n, ["history-0"]);
    const value = fixture();
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => configuredFallback),
      storage: localStorage,
    });
    await startVerified(workspace);
    const join = joinCodec.decode(value.sentControls[0] as never);
    if (!isJoinControl(join)) throw new Error("join control did not decode");
    value.streams.messages.push({
      ...textMessage("control", directId, 2n),
      contentType: ASSIGNMENT_CONTENT_TYPE,
      content: assignment(join.requestId),
    } as never);
    await waitFor(() => workspace.snapshot().channels.global.retentionVerified);
    expect(workspace.snapshot().channels.global.unread).toBe(39);

    value.streams.deleted.push(textMessage("history-1", globalId, 1_000n));
    await waitFor(() => workspace.snapshot().channels.global.unread === 38);
    expect(workspace.snapshot().channels.global.messages.some(({ id }) => id === "history-1")).toBe(false);
    await workspace.close();
  });

  it("filters spoofed conversation IDs returned by trusted history APIs", async () => {
    const value = fixture();
    value.global.messages.mockResolvedValueOnce([
      textMessage("history-spoof", "9".repeat(64), 2_000n),
      ...Array.from({ length: 39 }, (_, index) => textMessage(`trusted-history-${index}`, globalId, 1_000n)),
    ]);
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => configuredFallback),
      storage: localStorage,
    });
    await startVerified(workspace);
    const join = joinCodec.decode(value.sentControls[0] as never);
    if (!isJoinControl(join)) throw new Error("join control did not decode");
    value.streams.messages.push({
      ...textMessage("control", directId, 2n),
      contentType: ASSIGNMENT_CONTENT_TYPE,
      content: assignment(join.requestId),
    } as never);
    await waitFor(() => workspace.snapshot().channels.global.retentionVerified);
    expect(workspace.snapshot().channels.global.messages.some(({ id }) => id === "history-spoof")).toBe(false);
    expect(workspace.snapshot().channels.global.messages).toHaveLength(39);
    await workspace.close();
  });

  it("retries a failed transactional handoff on the new controller while retaining Global", async () => {
    const value = fixture();
    let resolved: TentacleAssignment = configuredFallback;
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => resolved),
      storage: localStorage,
    });
    await startVerified(workspace);
    const join = joinCodec.decode(value.sentControls[0] as never);
    if (!isJoinControl(join)) throw new Error("join control did not decode");
    value.streams.messages.push({
      ...textMessage("control", directId, 2n),
      contentType: ASSIGNMENT_CONTENT_TYPE,
      content: assignment(join.requestId),
    } as never);
    await waitFor(() => workspace.snapshot().channels.global.retentionVerified);
    resolved = {
      source: "branding-active",
      address,
      wallet: address,
      inboxId: tentacle,
      agentId: "43",
      name: "Fresh Controller Name",
      blockNumber: 124n,
      blockHash: `0x${"7".repeat(64)}`,
      notice: "Active Branding verified",
    };
    value.client.conversations.createDm.mockRejectedValueOnce(new Error("new Direct unavailable"));
    await expect(workspace.revalidateAssignment("retry")).rejects.toThrow(/new Direct unavailable/u);
    expect(workspace.snapshot().channels.direct).toMatchObject({
      status: "error",
      readConversationIds: [],
    });
    expect(workspace.snapshot().channels.acolytes).toMatchObject({
      status: "error",
      readConversationIds: [],
    });
    expect(workspace.snapshot().channels.global.readConversationIds).toEqual([globalId]);

    await workspace.revalidateAssignment("retry");
    expect(value.client.conversations.createDm).toHaveBeenCalledWith(
      tentacle,
      expect.objectContaining({ messageDisappearingSettings: expect.any(Object) }),
    );
    expect(value.client.conversations.createDm).toHaveBeenCalledTimes(2);
    expect(workspace.snapshot().channels.direct.readConversationIds).toEqual([directId]);
    expect(workspace.snapshot().channels.acolytes.readConversationIds).toEqual([]);
    expect(workspace.snapshot().channels.global.readConversationIds).toEqual([globalId]);
    expect(workspace.snapshot().channels.global.writeConversationId).toBe(globalId);

    const nextJoin = joinCodec.decode(value.sentControls.at(-1) as never);
    if (!isJoinControl(nextJoin)) throw new Error("new join control did not decode");
    const missingAcolytesId = "8".repeat(64);
    value.streams.messages.push({
      ...textMessage("new-control", directId, 3n),
      contentType: ASSIGNMENT_CONTENT_TYPE,
      content: {
        ...assignment(nextJoin.requestId),
        revision: `124:0x${"7".repeat(64)}`,
        tentacleAgentId: "43",
        acolytesGroupId: missingAcolytesId,
      },
    } as never);
    await waitFor(() => value.client.conversations.getConversationById.mock.calls.some(
      ([id]: [string]) => id === missingAcolytesId,
    ));
    expect(workspace.snapshot().channels.acolytes.status).toBe("awaiting-assignment");
    expect(workspace.snapshot().channels.global).toMatchObject({
      readConversationIds: [globalId],
      writeConversationId: globalId,
      retentionVerified: true,
      status: expect.stringMatching(/^(?:empty|ready)$/u),
    });
    await workspace.send("global", "retained while new Acolytes is pending");
    expect(value.global.sendText).toHaveBeenCalledWith("retained while new Acolytes is pending");
    await workspace.close();
  });
});
