import { describe, expect, it, vi } from "vitest";
import { GroupPermissionsOptions, contentTypeText } from "@xmtp/node-sdk";
import { getAddress } from "viem";
import {
  ASSIGNMENT_CONTENT_TYPE,
  AssignmentCodec,
  CANONICAL_BRANDING_CONTRACT,
  ChatControlService,
  GLOBAL_LOGICAL_CHANNEL_ID,
  JOIN_CONTENT_TYPE,
  JoinCodec,
  RETENTION_FROM_NS,
  RETENTION_IN_NS,
  acolytesAppData,
  bootstrapGlobalGroup,
  classifyInboundMessage,
  dispatchPersonalText,
  globalAppData,
  isJoinControl,
  parseControlPayload,
  resolveFreshSenderAddress,
  type AssignmentResolution,
  type AssignmentResolver,
  type ChatControlConfig,
  type ChatControlState,
  type ChatStateStore,
  type ConversationLike,
  type GroupDirectory,
  type GroupLike,
  type JoinControl,
} from "./chat-control.js";

const SELF = "aa".repeat(32);
const USER = "bb".repeat(32);
const GLOBAL = "cc".repeat(32);
const ACOLYTES = "dd".repeat(32);
const EVIL = "ee".repeat(32);
const AGENT_ID = "42";
const ADDRESS = getAddress("0x1111111111111111111111111111111111111111");
const REVISION = `50000000:0x${"12".repeat(32)}`;

describe("canonical deployment configuration", () => {
  it("pins the verified Base Branding contract", () => {
    expect(CANONICAL_BRANDING_CONTRACT).toBe(
      getAddress("0xD8c36F13D79a505C7FBDc5F6467eA3cd75E896Da"),
    );
  });
});

const ADMIN_ONLY_POLICY = {
  addMemberPolicy: 2,
  removeMemberPolicy: 2,
  addAdminPolicy: 3,
  removeAdminPolicy: 3,
  updateGroupNamePolicy: 2,
  updateGroupDescriptionPolicy: 2,
  updateGroupImageUrlSquarePolicy: 2,
  updateMessageDisappearingPolicy: 2,
  updateAppDataPolicy: 2,
};

class FakeConversation implements ConversationLike {
  settings: { fromNs: bigint; inNs: bigint } | undefined;
  updates = 0;

  constructor(
    readonly id: string,
    settings?: { fromNs: bigint; inNs: bigint },
  ) {
    this.settings = settings;
  }

  messageDisappearingSettings(): { fromNs: bigint; inNs: bigint } | undefined {
    return this.settings;
  }

  async updateMessageDisappearingSettings(fromNs: bigint, inNs: bigint): Promise<void> {
    this.settings = { fromNs, inNs };
    this.updates += 1;
  }
}

class FakeGroup extends FakeConversation implements GroupLike {
  readonly memberIds = new Set<string>();
  readonly admins = new Set<string>();
  readonly superAdmins = new Set<string>();
  added: string[][] = [];
  removed: string[][] = [];

  constructor(
    id: string,
    readonly appData: string,
    readonly addedByInboxId: string,
    readonly creatorInboxId: string,
    members: string[],
    admins: string[],
    superAdmins: string[],
    settings: { fromNs: bigint; inNs: bigint } | undefined = {
      fromNs: RETENTION_FROM_NS,
      inNs: RETENTION_IN_NS,
    },
    readonly permissionType = GroupPermissionsOptions.AdminOnly,
  ) {
    super(id, settings);
    members.forEach((member) => this.memberIds.add(member));
    admins.forEach((admin) => this.admins.add(admin));
    superAdmins.forEach((admin) => this.superAdmins.add(admin));
  }

  async metadata(): Promise<{ creatorInboxId: string }> {
    return { creatorInboxId: this.creatorInboxId };
  }

  async members(): Promise<Array<{ inboxId: string }>> {
    return [...this.memberIds].map((inboxId) => ({ inboxId }));
  }

  listAdmins(): string[] {
    return [...this.admins];
  }

  listSuperAdmins(): string[] {
    return [...this.superAdmins];
  }

  permissions(): { policyType: number; policySet: typeof ADMIN_ONLY_POLICY } {
    return { policyType: this.permissionType, policySet: { ...ADMIN_ONLY_POLICY } };
  }

  async addMembers(inboxIds: string[]): Promise<void> {
    this.added.push([...inboxIds]);
    inboxIds.forEach((inboxId) => this.memberIds.add(inboxId));
  }

  async removeMembers(inboxIds: string[]): Promise<void> {
    this.removed.push([...inboxIds]);
    inboxIds.forEach((inboxId) => this.memberIds.delete(inboxId));
  }

  async addAdmin(inboxId: string): Promise<void> {
    this.admins.add(inboxId);
  }
}

class FakeDirectory implements GroupDirectory {
  readonly groups: FakeGroup[];
  createCount = 0;

  constructor(groups: FakeGroup[]) {
    this.groups = groups;
  }

  async sync(): Promise<void> {}

  listGroups(): GroupLike[] {
    return this.groups;
  }

  async getConversationById(id: string): Promise<GroupLike | undefined> {
    return this.groups.find((group) => group.id === id);
  }

  async createGroup(
    inboxIds: string[],
    options: {
      permissions?: GroupPermissionsOptions;
      appData?: string;
      messageDisappearingSettings?: { fromNs: bigint; inNs: bigint };
    },
  ): Promise<GroupLike> {
    this.createCount += 1;
    const id = options.appData === globalAppData() ? GLOBAL : ACOLYTES;
    const group = new FakeGroup(
      id,
      options.appData ?? "",
      SELF,
      SELF,
      [SELF, ...inboxIds],
      [],
      [SELF],
      options.messageDisappearingSettings,
      options.permissions,
    );
    this.groups.push(group);
    return group;
  }
}

class FakeStore implements ChatStateStore {
  saves = 0;

  constructor(public state: ChatControlState) {}

  async load(): Promise<ChatControlState> {
    return structuredClone(this.state);
  }

  async save(state: ChatControlState): Promise<void> {
    this.state = structuredClone(state);
    this.saves += 1;
  }
}

class FakeResolver implements AssignmentResolver {
  resolution: AssignmentResolution = {
    kind: "assigned_here",
    revision: REVISION,
    tentacleAgentId: AGENT_ID,
    tentacleInboxId: SELF,
  };

  async resolve(): Promise<AssignmentResolution> {
    return this.resolution;
  }
}

function freshState(): ChatControlState {
  return {
    version: 1,
    environment: "production",
    tentacleAgentId: AGENT_ID,
    tentacleInboxId: SELF,
    enrollments: [],
  };
}

function config(admins = [SELF]): ChatControlConfig {
  return {
    globalGroupId: GLOBAL,
    globalAdminInboxIds: admins,
    assignmentRevalidateSeconds: 900,
  };
}

function globalGroup(admins = [SELF]): FakeGroup {
  return new FakeGroup(
    GLOBAL,
    globalAppData(),
    SELF,
    SELF,
    admins,
    [],
    admins,
  );
}

function service(options: {
  directory: FakeDirectory;
  store?: FakeStore;
  resolver?: FakeResolver;
  admins?: string[];
  resolveInboxAddress?: (inboxId: string) => Promise<typeof ADDRESS | undefined>;
}): ChatControlService {
  return new ChatControlService({
    directory: options.directory,
    store: options.store ?? new FakeStore(freshState()),
    resolver: options.resolver ?? new FakeResolver(),
    config: config(options.admins),
    selfInboxId: SELF,
    tentacleAgentId: AGENT_ID,
    ...(options.resolveInboxAddress === undefined
      ? {}
      : { resolveInboxAddress: options.resolveInboxAddress }),
  });
}

const JOIN: JoinControl = {
  type: "cthuwu.join.v1",
  requestId: "01".repeat(16),
  environment: "production",
};

describe("custom control codecs", () => {
  it("round-trips strict join and assignment content with no fallback or push", () => {
    const joinCodec = new JoinCodec();
    const encodedJoin = joinCodec.encode(JOIN);
    expect(encodedJoin.type).toEqual(JOIN_CONTENT_TYPE);
    expect(joinCodec.decode(encodedJoin)).toEqual(JOIN);
    expect(joinCodec.fallback(JOIN)).toBeUndefined();
    expect(joinCodec.shouldPush(JOIN)).toBe(false);

    const assignmentCodec = new AssignmentCodec();
    const assignment = {
      type: "cthuwu.assignment.v1",
      requestId: JOIN.requestId,
      environment: "production",
      revision: REVISION,
      tentacleAgentId: AGENT_ID,
      tentacleInboxId: SELF,
      acolytesGroupId: ACOLYTES,
      global: {
        logicalChannelId: GLOBAL_LOGICAL_CHANNEL_ID,
        readConversationIds: [GLOBAL],
        writeConversationId: GLOBAL,
        adminInboxIds: [SELF],
      },
      retention: { fromNs: "1", inNs: "1209600000000000" },
    } as const;
    const encodedAssignment = assignmentCodec.encode(assignment);
    expect(encodedAssignment.type).toEqual(ASSIGNMENT_CONTENT_TYPE);
    expect(assignmentCodec.decode(encodedAssignment)).toEqual(assignment);
  });

  it("rejects forged claims, unknown fields, and changed content-type versions", () => {
    const forged = new TextEncoder().encode(
      JSON.stringify({ ...JOIN, senderAddress: ADDRESS }),
    );
    expect(parseControlPayload(forged)).toEqual({ kind: "invalid" });

    const codec = new JoinCodec();
    const encoded = codec.encode(JOIN);
    expect(
      isJoinControl(
        codec.decode({
          ...encoded,
          type: { ...JOIN_CONTENT_TYPE, versionMinor: 1 },
        }),
      ),
    ).toBe(false);
    expect(
      isJoinControl(codec.decode({ ...encoded, parameters: { sender: USER } })),
    ).toBe(false);
  });

  it("supports the bounded 32-read and 32-admin Global assignment", () => {
    const readConversationIds = Array.from({ length: 32 }, (_, index) =>
      (index + 1).toString(16).padStart(2, "0").repeat(32),
    );
    const adminInboxIds = [
      SELF,
      ...Array.from({ length: 31 }, (_, index) =>
        (index + 64).toString(16).padStart(2, "0").repeat(32),
      ),
    ];
    const assignment = {
      type: "cthuwu.assignment.v1",
      requestId: JOIN.requestId,
      environment: "production",
      revision: REVISION,
      tentacleAgentId: AGENT_ID,
      tentacleInboxId: SELF,
      acolytesGroupId: ACOLYTES,
      global: {
        logicalChannelId: GLOBAL_LOGICAL_CHANNEL_ID,
        readConversationIds,
        writeConversationId: readConversationIds[0],
        adminInboxIds,
      },
      retention: { fromNs: "1", inNs: "1209600000000000" },
    } as const;
    const codec = new AssignmentCodec();
    const encoded = codec.encode(assignment);
    expect(encoded.content.byteLength).toBeGreaterThan(4 * 1024);
    expect(codec.decode(encoded)).toEqual(assignment);
  });

  it("classifies all group traffic away from personal inference", () => {
    expect(classifyInboundMessage(false, contentTypeText())).toBe("group");
    expect(classifyInboundMessage(false, JOIN_CONTENT_TYPE)).toBe("control");
    expect(classifyInboundMessage(true, contentTypeText())).toBe("direct");
  });

  it("dispatches only DM text to the personal bridge", () => {
    const bridge = vi.fn<(text: string) => void>();
    expect(dispatchPersonalText(true, JOIN_CONTENT_TYPE, "join", bridge)).toBe(false);
    expect(
      dispatchPersonalText(true, ASSIGNMENT_CONTENT_TYPE, "assignment", bridge),
    ).toBe(false);
    expect(dispatchPersonalText(false, contentTypeText(), "group", bridge)).toBe(false);
    expect(dispatchPersonalText(true, contentTypeText(), "direct", bridge)).toBe(true);
    expect(bridge).toHaveBeenCalledOnce();
    expect(bridge).toHaveBeenCalledWith("direct");
  });
});

describe("fresh sender authentication", () => {
  it("accepts exactly one network-fresh Ethereum identifier", async () => {
    await expect(
      resolveFreshSenderAddress(
        {
          fetchInboxStates: async () => [
            {
              inboxId: USER,
              identifiers: [
                { identifier: "passkey", identifierKind: 1 },
                { identifier: ADDRESS, identifierKind: 0 },
              ],
            },
          ],
        },
        USER,
      ),
    ).resolves.toBe(ADDRESS);
  });

  it("rejects a spoofed/multi-address inbox state", async () => {
    await expect(
      resolveFreshSenderAddress(
        {
          fetchInboxStates: async () => [
            {
              inboxId: USER,
              identifiers: [
                { identifier: ADDRESS, identifierKind: 0 },
                {
                  identifier: "0x2222222222222222222222222222222222222222",
                  identifierKind: 0,
                },
              ],
            },
          ],
        },
        USER,
      ),
    ).resolves.toBeUndefined();
  });
});

describe("idempotent channel enrollment", () => {
  it("creates one Acolytes group, adds members once, and repairs 14-day Direct policy", async () => {
    const global = globalGroup();
    const directory = new FakeDirectory([global]);
    const direct = new FakeConversation("11".repeat(32));
    const control = service({ directory });

    const first = await control.enroll({
      join: JOIN,
      senderInboxId: USER,
      senderAddress: ADDRESS,
      directConversation: direct,
    });
    const second = await control.enroll({
      join: JOIN,
      senderInboxId: USER,
      senderAddress: ADDRESS,
      directConversation: direct,
    });

    expect(first).toEqual(second);
    expect(first?.tentacleAgentId).toBe(AGENT_ID);
    expect(first?.global.adminInboxIds).toEqual([SELF]);
    expect(directory.createCount).toBe(1);
    const acolytes = directory.groups.find((group) => group.id === ACOLYTES);
    expect(acolytes?.memberIds.has(USER)).toBe(true);
    expect(acolytes?.added).toEqual([[USER]]);
    expect(global.added).toEqual([[USER]]);
    expect(direct.settings).toEqual({
      fromNs: RETENTION_FROM_NS,
      inNs: RETENTION_IN_NS,
    });
    expect(direct.updates).toBe(1);
  });

  it("ignores copied appData/name spoof groups during recovery", async () => {
    const spoof = new FakeGroup(
      EVIL,
      acolytesAppData(AGENT_ID, SELF),
      EVIL,
      EVIL,
      [EVIL],
      [],
      [EVIL],
    );
    const wrongAppData = new FakeGroup(
      "99".repeat(32),
      JSON.stringify({ app: "cthuwu.chat", channel: "acolytes" }),
      SELF,
      SELF,
      [SELF],
      [],
      [SELF],
    );
    const directory = new FakeDirectory([globalGroup(), spoof, wrongAppData]);
    await service({ directory }).enroll({
      join: JOIN,
      senderInboxId: USER,
      senderAddress: ADDRESS,
      directConversation: new FakeConversation("11".repeat(32)),
    });
    expect(directory.createCount).toBe(1);
    expect(spoof.memberIds.has(USER)).toBe(false);
    expect(wrongAppData.memberIds.has(USER)).toBe(false);
  });

  it("refuses a persisted group whose appData or authority was spoofed", async () => {
    const spoof = new FakeGroup(
      ACOLYTES,
      acolytesAppData(AGENT_ID, SELF),
      EVIL,
      EVIL,
      [EVIL],
      [],
      [EVIL],
    );
    const state = { ...freshState(), acolytesGroupId: ACOLYTES };
    const directory = new FakeDirectory([globalGroup(), spoof]);
    await expect(
      service({ directory, store: new FakeStore(state) }).enroll({
        join: JOIN,
        senderInboxId: USER,
        senderAddress: ADDRESS,
        directConversation: new FakeConversation("11".repeat(32)),
      }),
    ).rejects.toThrow(/trusted appData/u);
    expect(directory.createCount).toBe(0);
  });

  it("refuses to replace a drifted self-created Acolytes recovery candidate", async () => {
    const drifted = new FakeGroup(
      ACOLYTES,
      acolytesAppData(AGENT_ID, SELF),
      SELF,
      SELF,
      [SELF],
      [],
      [SELF],
      { fromNs: 0n, inNs: RETENTION_IN_NS },
    );
    const directory = new FakeDirectory([globalGroup(), drifted]);
    await expect(
      service({ directory }).enroll({
        join: JOIN,
        senderInboxId: USER,
        senderAddress: ADDRESS,
        directConversation: new FakeConversation("11".repeat(32)),
      }),
    ).rejects.toThrow(/operator repair/u);
    expect(directory.createCount).toBe(0);
  });

  it("removes a reassigned acolyte but freezes removal on registry outage", async () => {
    const resolver = new FakeResolver();
    const directory = new FakeDirectory([globalGroup()]);
    const store = new FakeStore(freshState());
    const control = service({ directory, resolver, store });
    await control.enroll({
      join: JOIN,
      senderInboxId: USER,
      senderAddress: ADDRESS,
      directConversation: new FakeConversation("11".repeat(32)),
    });
    const acolytes = directory.groups.find((group) => group.id === ACOLYTES);

    resolver.resolution = { kind: "registry_unavailable" };
    await expect(control.pruneMovedAssignments()).resolves.toBe(0);
    expect(acolytes?.memberIds.has(USER)).toBe(true);

    resolver.resolution = { kind: "assigned_elsewhere", revision: REVISION };
    await expect(control.pruneMovedAssignments()).resolves.toBe(1);
    expect(acolytes?.memberIds.has(USER)).toBe(false);
    expect(acolytes?.removed).toEqual([[USER]]);
  });

  it("removes an out-of-band group member missing from authenticated enrollment state", async () => {
    const resolver = new FakeResolver();
    const acolytes = new FakeGroup(
      ACOLYTES,
      acolytesAppData(AGENT_ID, SELF),
      SELF,
      SELF,
      [SELF, USER],
      [],
      [SELF],
    );
    const state = { ...freshState(), acolytesGroupId: ACOLYTES };
    const control = service({
      directory: new FakeDirectory([globalGroup(), acolytes]),
      store: new FakeStore(state),
      resolver,
    });
    await expect(control.pruneMovedAssignments()).resolves.toBe(1);
    expect(acolytes.memberIds.has(USER)).toBe(false);
    expect(resolver.resolution.kind).toBe("assigned_here");
  });
});

describe("Global bootstrap", () => {
  it("creates only by explicit command and sets authorized Tentacle admins", async () => {
    const otherAdmin = "f1".repeat(32);
    const directory = new FakeDirectory([]);
    const result = await bootstrapGlobalGroup({
      action: "create",
      directory,
      selfInboxId: SELF,
      adminInboxIds: [otherAdmin],
    });
    expect(result).toEqual({
      groupId: GLOBAL,
      adminInboxIds: [SELF, otherAdmin].sort(),
      created: true,
      recovered: false,
    });
    expect(directory.groups[0]?.admins.has(otherAdmin)).toBe(true);
    await expect(
      bootstrapGlobalGroup({
        action: "create",
        directory,
        selfInboxId: SELF,
        adminInboxIds: [otherAdmin],
      }),
    ).resolves.toEqual({
      groupId: GLOBAL,
      adminInboxIds: [SELF, otherAdmin].sort(),
      created: false,
      recovered: true,
    });
    expect(directory.createCount).toBe(1);
    await expect(
      bootstrapGlobalGroup({
        action: "create",
        directory,
        selfInboxId: SELF,
        adminInboxIds: [],
        configuredGroupId: GLOBAL,
      }),
    ).rejects.toThrow(/competing Global/u);
  });

  it("inspect reconciles missing configured admins but rejects unexpected elevation", async () => {
    const otherAdmin = "f1".repeat(32);
    const group = globalGroup();
    const directory = new FakeDirectory([group]);
    await bootstrapGlobalGroup({
      action: "inspect",
      directory,
      selfInboxId: SELF,
      adminInboxIds: [otherAdmin],
      configuredGroupId: GLOBAL,
    });
    expect(group.memberIds.has(otherAdmin)).toBe(true);
    expect(group.admins.has(otherAdmin)).toBe(true);

    group.admins.add(EVIL);
    await expect(
      bootstrapGlobalGroup({
        action: "inspect",
        directory,
        selfInboxId: SELF,
        adminInboxIds: [otherAdmin],
        configuredGroupId: GLOBAL,
      }),
    ).rejects.toThrow(/unexpected elevated admin/u);
  });

  it("blocks replacement of a drifted self-created Global but ignores copied appData", async () => {
    const drifted = new FakeGroup(
      GLOBAL,
      globalAppData(),
      SELF,
      SELF,
      [SELF],
      [],
      [SELF],
      { fromNs: 0n, inNs: RETENTION_IN_NS },
    );
    const blocked = new FakeDirectory([drifted]);
    await expect(
      bootstrapGlobalGroup({
        action: "create",
        directory: blocked,
        selfInboxId: SELF,
        adminInboxIds: [],
      }),
    ).rejects.toThrow(/operator repair or inspect/u);
    expect(blocked.createCount).toBe(0);

    const copied = new FakeGroup(
      EVIL,
      globalAppData(),
      EVIL,
      EVIL,
      [EVIL],
      [],
      [EVIL],
    );
    const ignored = new FakeDirectory([copied]);
    await expect(
      bootstrapGlobalGroup({
        action: "create",
        directory: ignored,
        selfInboxId: SELF,
        adminInboxIds: [],
      }),
    ).resolves.toMatchObject({ groupId: GLOBAL, created: true, recovered: false });
    expect(ignored.createCount).toBe(1);
  });
});
