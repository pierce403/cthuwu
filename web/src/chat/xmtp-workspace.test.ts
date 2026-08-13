import {
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
  isJoinControl,
  joinCodec,
  type AssignmentControl,
} from "./control";
import { createChatUiStateStore } from "./storage";
import { XmtpMultiChannelWorkspace, acquireXmtpDatabaseLease, recoverRegisteredClient } from "./xmtp-workspace";

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
  brandingStatus: "Unminted",
  blockNumber: 123n,
  blockHash: `0x${"6".repeat(64)}`,
  notice: "Unminted Branding; using intro Tentacle",
};
const rotated: TentacleAssignment = {
  source: "rotation-verified",
  address,
  wallet: address,
  inboxId: tentacle,
  agentId: "42",
  blockNumber: 123n,
  blockHash: `0x${"6".repeat(64)}`,
  notice: "Eligible Tentacle rotation verified",
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

  it("fails before OPFS open when another tab owns the identity database lease", async () => {
    let occupied = false;
    const request = vi.fn(async (_name: string, _options: LockOptions, callback: (lock: Lock | null) => Promise<void>) => {
      if (occupied) return callback(null);
      occupied = true;
      return callback({} as Lock);
    });
    vi.stubGlobal("navigator", { ...navigator, locks: { request } });
    const release = await acquireXmtpDatabaseLease("production", identity.address);
    await expect(acquireXmtpDatabaseLease("production", identity.address)).rejects.toThrow(/another tab/u);
    release();
  });

  it("keeps unconfigured deployment on legacy intro Direct without inventing group bindings", async () => {
    const value = fixture();
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => intro),
      storage: localStorage,
    });
    await workspace.start();
    expect(value.sentControls).toEqual([]);
    expect(workspace.snapshot().channels.direct.retentionVerified).toBe(true);
    expect(workspace.snapshot().channels.acolytes.status).toBe("awaiting-assignment");
    expect(workspace.snapshot().channels.global.readConversationIds).toEqual([]);
    await workspace.close();
  });

  it("persists a verified unbranded rotation choice for browser continuity", async () => {
    const value = fixture();
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => rotated),
      storage: localStorage,
    });
    await workspace.start();
    expect(localStorage.getItem(`cthuwu.rotation.v1:production:${identity.address}`)).toBe(address);
    await workspace.close();
  });

  it("accepts an authenticated later revision, routes trusted IDs, paginates, and deletes", async () => {
    const value = fixture();
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: vi.fn(async () => configuredFallback),
      storage: localStorage,
    });
    await workspace.start();
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

    value.streams.messages.push(textMessage("direct-in", directId, 1_500n));
    value.streams.messages.push(textMessage("acolytes-in", acolytesId, 1_501n));
    value.streams.messages.push(textMessage("global-in", globalId, 1_502n));
    await waitFor(() =>
      workspace.snapshot().channels.direct.messages.some(({ id }) => id === "direct-in") &&
      workspace.snapshot().channels.acolytes.messages.some(({ id }) => id === "acolytes-in") &&
      workspace.snapshot().channels.global.messages.some(({ id }) => id === "global-in"));
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
    await workspace.start();
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

  it("backs off automatic registry retries after an outage while preserving explicit retry", async () => {
    const value = fixture();
    const resolver = vi.fn(async () => { throw new RegistryUnavailableError("Base RPC rate limited"); });
    const workspace = new XmtpMultiChannelWorkspace(value.client as never, config, identity, {
      resolveAssignment: resolver,
      storage: localStorage,
    });
    await workspace.start();
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
    await workspace.start();
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
    await workspace.start();
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
    await workspace.start();
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
    await workspace.start();
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
    await workspace.start();
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
