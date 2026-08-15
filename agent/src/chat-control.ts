import { randomUUID } from "node:crypto";
import {
  chmod,
  lstat,
  mkdir,
  open,
  readFile,
  rename,
  unlink,
} from "node:fs/promises";
import path from "node:path";
import {
  createPublicClient,
  getAddress,
  hexToString,
  http,
  isAddress,
  parseAbi,
  stringToHex,
  type Address,
  type Hex,
} from "viem";
import { base } from "viem/chains";
import {
  GroupPermissionsOptions,
  PermissionPolicy,
  contentTypeText,
  type CreateGroupOptions,
} from "@xmtp/node-sdk";
import {
  type ContentCodec,
  type ContentTypeId,
  type EncodedContent,
} from "@xmtp/content-type-primitives";
import {
  ALLEGIANCE_VALUE,
  ERC8004_IDENTITY_REGISTRY,
  ERC8004_VERSION,
  PROTOCOL_VALUE,
} from "./erc8004.js";

export const CHAT_ENVIRONMENT = "production" as const;
export const GLOBAL_LOGICAL_CHANNEL_ID = "cthuwu.global.v1" as const;
export const RETENTION_FROM_NS = 1n;
export const RETENTION_IN_NS = 1_209_600_000_000_000n;
export const INTRO_TENTACLE_ADDRESS = getAddress(
  "0x4c4e26a4683f6b6d63e19abf0bedd1171b9a6e90",
);
export const CANONICAL_BRANDING_CONTRACT = getAddress(
  "0xD8c36F13D79a505C7FBDc5F6467eA3cd75E896Da",
);

const MAX_CONTROL_BYTES = 8 * 1024;
const MAX_STATE_BYTES = 256 * 1024;
const MAX_AGENT_URI_BYTES = 8 * 1024;
const MAX_GLOBAL_READ_CONVERSATIONS = 32;
const DEFAULT_BASE_RPC_ENDPOINT = "https://mainnet.base.org";
const CANONICAL_UWU = getAddress("0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07");
const INBOX_OR_CONVERSATION_ID = /^[0-9a-f]{64}$/u;
const REQUEST_ID = /^[0-9a-f]{32}$/u;
const DECIMAL_AGENT_ID = /^(0|[1-9][0-9]{0,77})$/u;
const REVISION = /^(?:0|[1-9][0-9]{0,31}):0x[0-9a-f]{64}$/u;
const INVALID_CONTROL = Object.freeze({ invalidControl: true });

type RecordValue = Record<string, unknown>;

export type JoinControl = {
  type: "cthuwu.join.v1";
  requestId: string;
  environment: "production";
};

export type AssignmentControl = {
  type: "cthuwu.assignment.v1";
  requestId: string;
  environment: "production";
  revision: string;
  tentacleAgentId: string;
  tentacleInboxId: string;
  acolytesGroupId: string;
  global: {
    logicalChannelId: "cthuwu.global.v1";
    readConversationIds: string[];
    writeConversationId: string;
    adminInboxIds: string[];
  };
  retention: {
    fromNs: "1";
    inNs: "1209600000000000";
  };
};

export type ParsedControl =
  | { kind: "invalid" }
  | { kind: "join"; value: JoinControl }
  | { kind: "assignment"; value: AssignmentControl };

export const JOIN_CONTENT_TYPE: ContentTypeId = {
  authorityId: "cthuwu.app",
  typeId: "join",
  versionMajor: 1,
  versionMinor: 0,
};

export const ASSIGNMENT_CONTENT_TYPE: ContentTypeId = {
  authorityId: "cthuwu.app",
  typeId: "assignment",
  versionMajor: 1,
  versionMinor: 0,
};
export const TYPING_CONTENT_TYPE: ContentTypeId = {
  authorityId: "cthuwu.app",
  typeId: "typing",
  versionMajor: 1,
  versionMinor: 0,
};

export type TypingControl = {
  type: "cthuwu.typing.v1";
  active: boolean;
  expiresAtNs: string;
};

function isRecord(value: unknown): value is RecordValue {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(value: RecordValue, expected: readonly string[]): boolean {
  const actual = Object.keys(value).sort();
  const sortedExpected = [...expected].sort();
  return (
    actual.length === sortedExpected.length &&
    actual.every((key, index) => key === sortedExpected[index])
  );
}

function isCanonicalId(value: unknown): value is string {
  return typeof value === "string" && INBOX_OR_CONVERSATION_ID.test(value);
}

function isCanonicalAgentId(value: unknown): value is string {
  if (typeof value !== "string" || !DECIMAL_AGENT_ID.test(value)) {
    return false;
  }
  try {
    return BigInt(value) < 1n << 256n;
  } catch {
    return false;
  }
}

function uniqueCanonicalIds(value: unknown, maximum: number): value is string[] {
  return (
    Array.isArray(value) &&
    value.length > 0 &&
    value.length <= maximum &&
    value.every(isCanonicalId) &&
    new Set(value).size === value.length
  );
}

function parseJoin(value: RecordValue): JoinControl | undefined {
  if (
    !hasExactKeys(value, ["type", "requestId", "environment"]) ||
    value.type !== "cthuwu.join.v1" ||
    typeof value.requestId !== "string" ||
    !REQUEST_ID.test(value.requestId) ||
    value.environment !== CHAT_ENVIRONMENT
  ) {
    return undefined;
  }
  return {
    type: "cthuwu.join.v1",
    requestId: value.requestId,
    environment: CHAT_ENVIRONMENT,
  };
}

export function isJoinControl(value: unknown): value is JoinControl {
  return isRecord(value) && parseJoin(value) !== undefined;
}

function parseAssignment(value: RecordValue): AssignmentControl | undefined {
  if (
    !hasExactKeys(value, [
      "type",
      "requestId",
      "environment",
      "revision",
      "tentacleAgentId",
      "tentacleInboxId",
      "acolytesGroupId",
      "global",
      "retention",
    ]) ||
    value.type !== "cthuwu.assignment.v1" ||
    typeof value.requestId !== "string" ||
    !REQUEST_ID.test(value.requestId) ||
    value.environment !== CHAT_ENVIRONMENT ||
    typeof value.revision !== "string" ||
    !REVISION.test(value.revision) ||
    !isCanonicalAgentId(value.tentacleAgentId) ||
    !isCanonicalId(value.tentacleInboxId) ||
    !isCanonicalId(value.acolytesGroupId) ||
    !isRecord(value.global) ||
    !isRecord(value.retention)
  ) {
    return undefined;
  }
  const global = value.global;
  const retention = value.retention;
  if (
    !hasExactKeys(global, [
      "logicalChannelId",
      "readConversationIds",
      "writeConversationId",
      "adminInboxIds",
    ]) ||
    global.logicalChannelId !== GLOBAL_LOGICAL_CHANNEL_ID ||
    !uniqueCanonicalIds(global.readConversationIds, MAX_GLOBAL_READ_CONVERSATIONS) ||
    !isCanonicalId(global.writeConversationId) ||
    !global.readConversationIds.includes(global.writeConversationId) ||
    !uniqueCanonicalIds(global.adminInboxIds, 32) ||
    !global.adminInboxIds.includes(value.tentacleInboxId) ||
    !hasExactKeys(retention, ["fromNs", "inNs"]) ||
    retention.fromNs !== RETENTION_FROM_NS.toString() ||
    retention.inNs !== RETENTION_IN_NS.toString()
  ) {
    return undefined;
  }
  return {
    type: "cthuwu.assignment.v1",
    requestId: value.requestId,
    environment: CHAT_ENVIRONMENT,
    revision: value.revision,
    tentacleAgentId: value.tentacleAgentId,
    tentacleInboxId: value.tentacleInboxId,
    acolytesGroupId: value.acolytesGroupId,
    global: {
      logicalChannelId: GLOBAL_LOGICAL_CHANNEL_ID,
      readConversationIds: [...global.readConversationIds],
      writeConversationId: global.writeConversationId,
      adminInboxIds: [...global.adminInboxIds],
    },
    retention: {
      fromNs: "1",
      inNs: "1209600000000000",
    },
  };
}

export function parseControlPayload(payload: Uint8Array): ParsedControl {
  if (payload.byteLength > MAX_CONTROL_BYTES) {
    return { kind: "invalid" };
  }
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(payload));
  } catch {
    return { kind: "invalid" };
  }
  if (!isRecord(value)) {
    return { kind: "invalid" };
  }
  const join = parseJoin(value);
  if (join !== undefined) {
    return { kind: "join", value: join };
  }
  const assignment = parseAssignment(value);
  return assignment === undefined
    ? { kind: "invalid" }
    : { kind: "assignment", value: assignment };
}

function contentTypeEquals(actual: ContentTypeId | undefined, expected: ContentTypeId): boolean {
  return (
    actual?.authorityId === expected.authorityId &&
    actual.typeId === expected.typeId &&
    actual.versionMajor === expected.versionMajor &&
    actual.versionMinor === expected.versionMinor
  );
}

function encodePayload(
  contentType: ContentTypeId,
  value: JoinControl | AssignmentControl | TypingControl,
): EncodedContent {
  const content = new TextEncoder().encode(JSON.stringify(value));
  if (content.byteLength > MAX_CONTROL_BYTES) {
    throw new Error("control payload exceeds its encoded bound");
  }
  return { type: contentType, parameters: {}, content };
}

function parseTyping(value: RecordValue): TypingControl | undefined {
  if (
    !hasExactKeys(value, ["active", "expiresAtNs", "type"]) ||
    value.type !== "cthuwu.typing.v1" ||
    typeof value.active !== "boolean" ||
    typeof value.expiresAtNs !== "string" ||
    !/^[1-9][0-9]{0,19}$/u.test(value.expiresAtNs)
  ) return undefined;
  return { type: "cthuwu.typing.v1", active: value.active, expiresAtNs: value.expiresAtNs };
}

export class TypingCodec implements ContentCodec<unknown> {
  readonly contentType = TYPING_CONTENT_TYPE;

  encode(content: unknown): EncodedContent {
    const parsed = isRecord(content) ? parseTyping(content) : undefined;
    if (!parsed) throw new Error("invalid cthuwu.typing.v1 content");
    return encodePayload(this.contentType, parsed);
  }

  decode(content: EncodedContent): unknown {
    try {
      assertEncodedEnvelope(content, this.contentType);
      const value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(content.content)) as unknown;
      if (isRecord(value)) return parseTyping(value) ?? INVALID_CONTROL;
    } catch {
      // Hostile typing payloads are ignored by registered clients.
    }
    return INVALID_CONTROL;
  }

  fallback(_content: unknown): undefined { return undefined; }
  shouldPush(_content: unknown): boolean { return false; }
}

function assertEncodedEnvelope(content: EncodedContent, expected: ContentTypeId): void {
  if (
    !contentTypeEquals(content.type, expected) ||
    Object.keys(content.parameters).length !== 0 ||
    content.fallback !== undefined ||
    content.compression !== undefined
  ) {
    throw new Error("control content has unexpected type metadata");
  }
}

export class JoinCodec implements ContentCodec<unknown> {
  readonly contentType = JOIN_CONTENT_TYPE;

  encode(content: unknown): EncodedContent {
    const parsed = isRecord(content) ? parseJoin(content) : undefined;
    if (parsed === undefined) {
      throw new Error("invalid cthuwu.join.v1 content");
    }
    return encodePayload(this.contentType, parsed);
  }

  decode(content: EncodedContent): unknown {
    try {
      assertEncodedEnvelope(content, this.contentType);
      const parsed = parseControlPayload(content.content);
      if (parsed.kind === "join") {
        return parsed.value;
      }
    } catch {
      // Malformed network content is an intercepted invalid control value, not a stream-fatal
      // codec exception. The first middleware checks the exact full content type and shape.
    }
    return INVALID_CONTROL;
  }

  fallback(_content: unknown): undefined {
    return undefined;
  }

  shouldPush(_content: unknown): boolean {
    return false;
  }
}

export class AssignmentCodec implements ContentCodec<unknown> {
  readonly contentType = ASSIGNMENT_CONTENT_TYPE;

  encode(content: unknown): EncodedContent {
    const parsed = isRecord(content) ? parseAssignment(content) : undefined;
    if (parsed === undefined) {
      throw new Error("invalid cthuwu.assignment.v1 content");
    }
    return encodePayload(this.contentType, parsed);
  }

  decode(content: EncodedContent): unknown {
    try {
      assertEncodedEnvelope(content, this.contentType);
      const parsed = parseControlPayload(content.content);
      if (parsed.kind === "assignment") {
        return parsed.value;
      }
    } catch {
      // See JoinCodec.decode: hostile payloads must not tear down the Agent stream.
    }
    return INVALID_CONTROL;
  }

  fallback(_content: unknown): undefined {
    return undefined;
  }

  shouldPush(_content: unknown): boolean {
    return false;
  }
}

export function isExactJoinContentType(value: ContentTypeId): boolean {
  return contentTypeEquals(value, JOIN_CONTENT_TYPE);
}

export function isExactAssignmentContentType(value: ContentTypeId): boolean {
  return contentTypeEquals(value, ASSIGNMENT_CONTENT_TYPE);
}

export function isExactTypingContentType(value: ContentTypeId): boolean {
  return contentTypeEquals(value, TYPING_CONTENT_TYPE);
}

export type InboundDisposition = "control" | "direct" | "group";

export function classifyInboundMessage(
  isDm: boolean,
  contentType: ContentTypeId,
): InboundDisposition {
  if (
    isExactJoinContentType(contentType) ||
    isExactAssignmentContentType(contentType) ||
    isExactTypingContentType(contentType)
  ) {
    return "control";
  }
  return isDm ? "direct" : "group";
}

/**
 * Mirrors the final sidecar-to-Rust dispatch gate. Agent middleware consumes custom controls
 * before events, and this second gate permits only actual DM text to enter the personal bridge.
 */
export function dispatchPersonalText(
  isDm: boolean,
  contentType: ContentTypeId,
  text: string,
  bridge: (text: string) => void,
): boolean {
  if (
    classifyInboundMessage(isDm, contentType) !== "direct" ||
    !contentTypeEquals(contentType, contentTypeText())
  ) {
    return false;
  }
  bridge(text);
  return true;
}

export type AcolytesAppData = {
  app: "cthuwu.chat";
  version: 1;
  environment: "production";
  channel: "acolytes";
  tentacleAgentId: string;
  tentacleInboxId: string;
};

export type GlobalAppData = {
  app: "cthuwu.chat";
  version: 1;
  environment: "production";
  channel: "global";
  logicalChannelId: "cthuwu.global.v1";
  shardId: "primary";
};

export function acolytesAppData(
  tentacleAgentId: string,
  tentacleInboxId: string,
): string {
  if (!isCanonicalAgentId(tentacleAgentId) || !isCanonicalId(tentacleInboxId)) {
    throw new Error("cannot encode untrusted Acolytes appData");
  }
  const value: AcolytesAppData = {
    app: "cthuwu.chat",
    version: 1,
    environment: CHAT_ENVIRONMENT,
    channel: "acolytes",
    tentacleAgentId,
    tentacleInboxId,
  };
  return JSON.stringify(value);
}

export function globalAppData(): string {
  const value: GlobalAppData = {
    app: "cthuwu.chat",
    version: 1,
    environment: CHAT_ENVIRONMENT,
    channel: "global",
    logicalChannelId: GLOBAL_LOGICAL_CHANNEL_ID,
    shardId: "primary",
  };
  return JSON.stringify(value);
}

function strictAppData(actual: string, expected: string): boolean {
  if (Buffer.byteLength(actual, "utf8") > 1024) {
    return false;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(actual);
  } catch {
    return false;
  }
  return isRecord(parsed) && JSON.stringify(parsed) === expected;
}

export type GroupMemberLike = {
  inboxId: string;
};

export type ConversationLike = {
  id: string;
  messageDisappearingSettings(): { fromNs: bigint; inNs: bigint } | undefined;
  updateMessageDisappearingSettings(fromNs: bigint, inNs: bigint): Promise<void>;
};

export type GroupLike = ConversationLike & {
  appData: string;
  addedByInboxId: string;
  metadata(): Promise<{ creatorInboxId: string }>;
  members(): Promise<GroupMemberLike[]>;
  listAdmins(): string[];
  listSuperAdmins(): string[];
  permissions(): {
    policyType: number;
    policySet: {
      addMemberPolicy: number;
      removeMemberPolicy: number;
      addAdminPolicy: number;
      removeAdminPolicy: number;
      updateGroupNamePolicy: number;
      updateGroupDescriptionPolicy: number;
      updateGroupImageUrlSquarePolicy: number;
      updateMessageDisappearingPolicy: number;
      updateAppDataPolicy: number;
    };
  };
  addMembers(inboxIds: string[]): Promise<void>;
  removeMembers(inboxIds: string[]): Promise<void>;
  addAdmin(inboxId: string): Promise<void>;
};

export type GroupDirectory = {
  sync(): Promise<void>;
  listGroups(): GroupLike[];
  getConversationById(id: string): Promise<GroupLike | undefined>;
  createGroup(inboxIds: string[], options: CreateGroupOptions): Promise<GroupLike>;
};

function sameSet(actual: readonly string[], expected: readonly string[]): boolean {
  const a = [...new Set(actual)].sort();
  const b = [...new Set(expected)].sort();
  return a.length === b.length && a.every((value, index) => value === b[index]);
}

async function ensureRetention(conversation: ConversationLike): Promise<void> {
  const current = conversation.messageDisappearingSettings();
  if (current?.fromNs === RETENTION_FROM_NS && current.inNs === RETENTION_IN_NS) {
    return;
  }
  await conversation.updateMessageDisappearingSettings(
    RETENTION_FROM_NS,
    RETENTION_IN_NS,
  );
  const repaired = conversation.messageDisappearingSettings();
  if (repaired?.fromNs !== RETENTION_FROM_NS || repaired.inNs !== RETENTION_IN_NS) {
    throw new Error("XMTP conversation did not retain the required 14-day policy");
  }
}

function hasRequiredRetention(conversation: ConversationLike): boolean {
  const current = conversation.messageDisappearingSettings();
  return current?.fromNs === RETENTION_FROM_NS && current.inNs === RETENTION_IN_NS;
}

function hasAdminOnlyPermissions(group: GroupLike): boolean {
  const permissions = group.permissions();
  const policy = permissions.policySet;
  return (
    permissions.policyType === GroupPermissionsOptions.AdminOnly &&
    policy.addMemberPolicy === PermissionPolicy.Admin &&
    policy.removeMemberPolicy === PermissionPolicy.Admin &&
    policy.addAdminPolicy === PermissionPolicy.SuperAdmin &&
    policy.removeAdminPolicy === PermissionPolicy.SuperAdmin &&
    policy.updateGroupNamePolicy === PermissionPolicy.Admin &&
    policy.updateGroupDescriptionPolicy === PermissionPolicy.Admin &&
    policy.updateGroupImageUrlSquarePolicy === PermissionPolicy.Admin &&
    policy.updateMessageDisappearingPolicy === PermissionPolicy.Admin &&
    policy.updateAppDataPolicy === PermissionPolicy.Admin
  );
}

async function isTrustedAcolytesGroup(
  group: GroupLike,
  selfInboxId: string,
  tentacleAgentId: string,
): Promise<boolean> {
  const metadata = await group.metadata();
  const members = await group.members();
  return (
    strictAppData(group.appData, acolytesAppData(tentacleAgentId, selfInboxId)) &&
    metadata.creatorInboxId === selfInboxId &&
    group.addedByInboxId === selfInboxId &&
    sameSet(group.listAdmins(), []) &&
    sameSet(group.listSuperAdmins(), [selfInboxId]) &&
    hasAdminOnlyPermissions(group) &&
    members.some((member) => member.inboxId === selfInboxId)
  );
}

async function isSelfCreatedWithAppData(
  group: GroupLike,
  selfInboxId: string,
  expectedAppData: string,
): Promise<boolean> {
  if (!strictAppData(group.appData, expectedAppData)) {
    return false;
  }
  const metadata = await group.metadata();
  return (
    metadata.creatorInboxId === selfInboxId && group.addedByInboxId === selfInboxId
  );
}

async function assertAcolytesGroup(
  group: GroupLike,
  selfInboxId: string,
  tentacleAgentId: string,
): Promise<void> {
  if (!(await isTrustedAcolytesGroup(group, selfInboxId, tentacleAgentId))) {
    throw new Error("Acolytes group failed trusted appData, creator, or admin validation");
  }
}

async function validateGlobalGroupBase(
  group: GroupLike,
  groupId: string,
  expectedAdminInboxIds: readonly string[],
): Promise<void> {
  if (
    group.id !== groupId ||
    !strictAppData(group.appData, globalAppData()) ||
    !hasAdminOnlyPermissions(group)
  ) {
    throw new Error("Global group failed exact conversation ID or appData validation");
  }
  const elevated = [...group.listAdmins(), ...group.listSuperAdmins()];
  if (elevated.some((inboxId) => !expectedAdminInboxIds.includes(inboxId))) {
    throw new Error("Global group contains an unexpected elevated admin");
  }
}

async function isRecoverableGlobalCandidate(
  group: GroupLike,
  selfInboxId: string,
  expectedAdminInboxIds: readonly string[],
): Promise<boolean> {
  const metadata = await group.metadata();
  const elevated = [...group.listAdmins(), ...group.listSuperAdmins()];
  return (
    strictAppData(group.appData, globalAppData()) &&
    metadata.creatorInboxId === selfInboxId &&
    group.addedByInboxId === selfInboxId &&
    hasAdminOnlyPermissions(group) &&
    hasRequiredRetention(group) &&
    elevated.every((inboxId) => expectedAdminInboxIds.includes(inboxId))
  );
}

async function reconcileGlobalAdmins(
  group: GroupLike,
  groupId: string,
  admins: readonly string[],
): Promise<void> {
  await validateGlobalGroupBase(group, groupId, admins);
  const members = await group.members();
  const missingMembers = admins.filter(
    (inboxId) => !members.some((member) => member.inboxId === inboxId),
  );
  if (missingMembers.length > 0) {
    await group.addMembers(missingMembers);
  }
  const elevated = new Set([...group.listAdmins(), ...group.listSuperAdmins()]);
  for (const inboxId of admins) {
    if (!elevated.has(inboxId)) {
      await group.addAdmin(inboxId);
    }
  }
  await assertGlobalGroup(group, groupId, admins);
  await ensureRetention(group);
}

async function assertGlobalGroup(
  group: GroupLike,
  groupId: string,
  expectedAdminInboxIds: readonly string[],
): Promise<void> {
  await validateGlobalGroupBase(group, groupId, expectedAdminInboxIds);
  const elevated = [...group.listAdmins(), ...group.listSuperAdmins()];
  if (!sameSet(elevated, expectedAdminInboxIds)) {
    throw new Error("Global group admin set differs from configured authority");
  }
  const members = await group.members();
  const memberIds = members.map((member) => member.inboxId);
  if (!expectedAdminInboxIds.every((inboxId) => memberIds.includes(inboxId))) {
    throw new Error("Global group is missing a configured Tentacle admin member");
  }
}

export type Enrollment = {
  inboxId: string;
  address: Address;
  revision: string;
};

export type ChatControlState = {
  version: 1;
  environment: "production";
  tentacleAgentId: string;
  tentacleInboxId: string;
  acolytesGroupId?: string;
  globalGroupId?: string;
  enrollments: Enrollment[];
};

export interface ChatStateStore {
  load(): Promise<ChatControlState>;
  save(state: ChatControlState): Promise<void>;
}

function parseChatState(
  value: unknown,
  tentacleAgentId: string,
  tentacleInboxId: string,
): ChatControlState {
  if (
    !isRecord(value) ||
    value.version !== 1 ||
    value.environment !== CHAT_ENVIRONMENT ||
    value.tentacleAgentId !== tentacleAgentId ||
    value.tentacleInboxId !== tentacleInboxId ||
    (value.acolytesGroupId !== undefined && !isCanonicalId(value.acolytesGroupId)) ||
    (value.globalGroupId !== undefined && !isCanonicalId(value.globalGroupId)) ||
    !Array.isArray(value.enrollments) ||
    value.enrollments.length > 10_000
  ) {
    throw new Error("persisted chat-control state is invalid or belongs to another Tentacle");
  }
  const enrollments: Enrollment[] = [];
  const seen = new Set<string>();
  for (const candidate of value.enrollments) {
    if (
      !isRecord(candidate) ||
      !hasExactKeys(candidate, ["inboxId", "address", "revision"]) ||
      !isCanonicalId(candidate.inboxId) ||
      typeof candidate.address !== "string" ||
      !isAddress(candidate.address, { strict: true }) ||
      getAddress(candidate.address) === getAddress("0x0000000000000000000000000000000000000000") ||
      typeof candidate.revision !== "string" ||
      !REVISION.test(candidate.revision) ||
      seen.has(candidate.inboxId)
    ) {
      throw new Error("persisted chat-control enrollment is invalid");
    }
    seen.add(candidate.inboxId);
    enrollments.push({
      inboxId: candidate.inboxId,
      address: getAddress(candidate.address),
      revision: candidate.revision,
    });
  }
  return {
    version: 1,
    environment: CHAT_ENVIRONMENT,
    tentacleAgentId,
    tentacleInboxId,
    ...(value.acolytesGroupId === undefined
      ? {}
      : { acolytesGroupId: value.acolytesGroupId }),
    ...(value.globalGroupId === undefined ? {} : { globalGroupId: value.globalGroupId }),
    enrollments,
  };
}

export class FileChatStateStore implements ChatStateStore {
  readonly #directory: string;
  readonly #filePath: string;
  readonly #tentacleAgentId: string;
  readonly #tentacleInboxId: string;

  constructor(dataDir: string, tentacleAgentId: string, tentacleInboxId: string) {
    this.#directory = path.join(path.resolve(dataDir), "state");
    this.#filePath = path.join(this.#directory, "xmtp-chat-control.json");
    this.#tentacleAgentId = tentacleAgentId;
    this.#tentacleInboxId = tentacleInboxId;
  }

  #fresh(): ChatControlState {
    return {
      version: 1,
      environment: CHAT_ENVIRONMENT,
      tentacleAgentId: this.#tentacleAgentId,
      tentacleInboxId: this.#tentacleInboxId,
      enrollments: [],
    };
  }

  async load(): Promise<ChatControlState> {
    await mkdir(this.#directory, { recursive: true, mode: 0o700 });
    await chmod(this.#directory, 0o700);
    try {
      const stat = await lstat(this.#filePath);
      if (!stat.isFile() || stat.isSymbolicLink() || stat.size > MAX_STATE_BYTES) {
        throw new Error("chat-control state is not a bounded regular file");
      }
      await chmod(this.#filePath, 0o600);
      return parseChatState(
        JSON.parse(await readFile(this.#filePath, "utf8")),
        this.#tentacleAgentId,
        this.#tentacleInboxId,
      );
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") {
        return this.#fresh();
      }
      throw error;
    }
  }

  async save(state: ChatControlState): Promise<void> {
    const canonical = parseChatState(
      state,
      this.#tentacleAgentId,
      this.#tentacleInboxId,
    );
    await mkdir(this.#directory, { recursive: true, mode: 0o700 });
    const encoded = `${JSON.stringify(canonical, null, 2)}\n`;
    if (Buffer.byteLength(encoded, "utf8") > MAX_STATE_BYTES) {
      throw new Error("chat-control state exceeds its bound");
    }
    const temporary = `${this.#filePath}.${process.pid}.${randomUUID()}.tmp`;
    const handle = await open(temporary, "wx", 0o600);
    try {
      await handle.writeFile(encoded, "utf8");
      await handle.sync();
    } finally {
      await handle.close();
    }
    try {
      await rename(temporary, this.#filePath);
      await chmod(this.#filePath, 0o600);
    } finally {
      await unlink(temporary).catch(() => undefined);
    }
  }
}

export type AssignmentResolution =
  | {
      kind: "assigned_here";
      revision: string;
      tentacleAgentId: string;
      tentacleInboxId: string;
    }
  | { kind: "assigned_elsewhere"; revision: string }
  | { kind: "registry_unavailable" };

export interface AssignmentResolver {
  resolve(address: Address): Promise<AssignmentResolution>;
}

export type FreshInboxPreferences = {
  fetchInboxStates(inboxIds: string[]): Promise<
    Array<{
      inboxId: string;
      identifiers: Array<{ identifier: string; identifierKind: number }>;
    }>
  >;
};

export async function resolveFreshSenderAddress(
  preferences: FreshInboxPreferences,
  senderInboxId: string,
): Promise<Address | undefined> {
  if (!isCanonicalId(senderInboxId)) {
    return undefined;
  }
  const states = await preferences.fetchInboxStates([senderInboxId]);
  if (states.length !== 1 || states[0]?.inboxId !== senderInboxId) {
    return undefined;
  }
  // IdentifierKind.Ethereum is 0 in the pinned node bindings. Require one exact current Ethereum
  // binding instead of selecting the SDK helper's first cached identifier.
  const ethereum = states[0].identifiers.filter(
    (identifier) => identifier.identifierKind === 0,
  );
  if (
    ethereum.length !== 1 ||
    ethereum[0] === undefined ||
    !isAddress(ethereum[0].identifier, { strict: true })
  ) {
    return undefined;
  }
  const address = getAddress(ethereum[0].identifier);
  return address === getAddress("0x0000000000000000000000000000000000000000")
    ? undefined
    : address;
}

const brandingAbi = parseAbi([
  "function brandingOf(address acolyte) view returns ((uint256 tokenId,address acolyte,address owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 pendingDeclaredPrice,uint256 pendingPriceActivation,uint8 status) result)",
  "function BASE_CHAIN_ID() view returns (uint256)",
  "function IDENTITY_REGISTRY() view returns (address)",
  "function UWU() view returns (address)",
  "function REGISTRY_VERSION() view returns (string)",
]);

const identityAbi = parseAbi([
  "function getVersion() view returns (string)",
  "function tokenURI(uint256 agentId) view returns (string)",
  "function getAgentWallet(uint256 agentId) view returns (address)",
  "function getMetadata(uint256 agentId, string metadataKey) view returns (bytes)",
  "function isAuthorizedOrOwner(address spender, uint256 agentId) view returns (bool)",
]);

type RegistrationSnapshot = {
  agentId: string;
  wallet: Address;
  inboxId: string;
};

export async function loadVerifiedRegistration(
  dataDir: string,
  expectedWallet: Address,
  expectedInboxId: string,
): Promise<RegistrationSnapshot> {
  const snapshotPath = path.join(path.resolve(dataDir), "state", "erc8004-registration.json");
  const stat = await lstat(snapshotPath);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > MAX_STATE_BYTES) {
    throw new Error("ERC-8004 registration snapshot is not a bounded regular file");
  }
  const value: unknown = JSON.parse(await readFile(snapshotPath, "utf8"));
  if (!isRecord(value) || !isRecord(value.last_verified)) {
    throw new Error("Tentacle has no locally verified ERC-8004 identity");
  }
  const verified = value.last_verified;
  if (
    value.chain_id !== 8453 ||
    typeof value.identity_registry !== "string" ||
    !isAddress(value.identity_registry, { strict: true }) ||
    getAddress(value.identity_registry) !== ERC8004_IDENTITY_REGISTRY ||
    value.phase !== "active" ||
    !isCanonicalAgentId(value.confirmed_agent_id) ||
    value.selected_agent_id !== value.confirmed_agent_id ||
    typeof value.tentacle_wallet !== "string" ||
    !isAddress(value.tentacle_wallet, { strict: true }) ||
    getAddress(value.tentacle_wallet) !== expectedWallet ||
    value.xmtp_inbox_id !== expectedInboxId ||
    typeof verified.agent_wallet !== "string" ||
    !isAddress(verified.agent_wallet, { strict: true }) ||
    getAddress(verified.agent_wallet) !== expectedWallet ||
    verified.authorized !== true ||
    verified.declares_tentacle_allegiance !== true ||
    verified.protocol_compatible !== true ||
    verified.wallet_verified !== true
  ) {
    throw new Error("persisted ERC-8004 identity is not active or does not match XMTP");
  }
  return { agentId: value.confirmed_agent_id, wallet: expectedWallet, inboxId: expectedInboxId };
}

function safeRpcEndpoint(value: string | undefined): string {
  const endpoint = new URL(value ?? DEFAULT_BASE_RPC_ENDPOINT);
  if (
    (endpoint.protocol !== "https:" &&
      !(endpoint.protocol === "http:" && ["127.0.0.1", "::1", "localhost"].includes(endpoint.hostname))) ||
    endpoint.username !== "" ||
    endpoint.password !== ""
  ) {
    throw new Error("CTHUWU_RPC_ENDPOINT must be credential-free HTTPS or loopback HTTP");
  }
  return endpoint.toString();
}

function configuredBrandingAddress(value: string | undefined): Address | undefined {
  if (value === undefined || value.trim() === "") {
    return CANONICAL_BRANDING_CONTRACT;
  }
  if (!isAddress(value, { strict: true })) {
    throw new Error("CTHUWU_BRANDING_CONTRACT must be a full EVM address");
  }
  const address = getAddress(value);
  if (address === getAddress("0x0000000000000000000000000000000000000000")) {
    throw new Error("CTHUWU_BRANDING_CONTRACT must not be zero");
  }
  return address;
}

function decodeCanonicalDataJson(uri: string): unknown {
  const prefix = "data:application/json;base64,";
  if (!uri.startsWith(prefix) || Buffer.byteLength(uri, "utf8") > MAX_AGENT_URI_BYTES) {
    throw new Error("data URI is not a bounded application/json base64 value");
  }
  const encoded = uri.slice(prefix.length);
  if (
    encoded.length === 0 ||
    encoded.length % 4 !== 0 ||
    !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/u.test(
      encoded,
    )
  ) {
    throw new Error("data URI base64 is not canonical");
  }
  const decoded = Buffer.from(encoded, "base64");
  if (decoded.toString("base64") !== encoded || decoded.byteLength > MAX_AGENT_URI_BYTES) {
    throw new Error("data URI base64 is not canonical or exceeds its decoded bound");
  }
  try {
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(decoded));
  } catch {
    throw new Error("data URI has invalid UTF-8 JSON");
  }
}

function endpointFromAgentUri(agentUri: string, agentId: string): string {
  let value: unknown;
  try {
    value = decodeCanonicalDataJson(agentUri);
  } catch {
    throw new Error("controller agentURI has invalid registration-v1 JSON");
  }
  if (
    !isRecord(value) ||
    value.type !== "https://eips.ethereum.org/EIPS/eip-8004#registration-v1" ||
    value.active !== true ||
    !Array.isArray(value.services) ||
    !Array.isArray(value.registrations)
  ) {
    throw new Error("controller agentURI is not an active registration-v1 profile");
  }
  const matchingRegistrations = value.registrations.filter(
    (registration) => {
      if (
        !isRecord(registration) ||
        !hasExactKeys(registration, ["agentId", "agentRegistry"]) ||
        registration.agentRegistry !== `eip155:8453:${ERC8004_IDENTITY_REGISTRY}`
      ) {
        return false;
      }
      return (
        (typeof registration.agentId === "number" &&
          Number.isSafeInteger(registration.agentId) &&
          registration.agentId >= 0 &&
          BigInt(registration.agentId) === BigInt(agentId)) ||
        registration.agentId === agentId
      );
    },
  );
  const xmtpServices = value.services
    .filter(
      (service): service is RecordValue =>
        isRecord(service) &&
        hasExactKeys(service, ["name", "endpoint", "version"]) &&
        service.name === "CTHUWU-XMTP" &&
        service.version === "1" &&
        typeof service.endpoint === "string",
    );
  const manifestServices = value.services.filter(
    (service): service is RecordValue =>
      isRecord(service) &&
      hasExactKeys(service, ["name", "endpoint", "version"]) &&
      service.name === "CTHUWU" &&
      typeof service.endpoint === "string" &&
      service.version === "1",
  );
  if (
    matchingRegistrations.length !== 1 ||
    xmtpServices.length !== 1 ||
    manifestServices.length !== 1
  ) {
    throw new Error("controller profile lacks one exact production XMTP service");
  }
  const endpoint = xmtpServices[0]?.endpoint;
  if (typeof endpoint !== "string" || !endpoint.startsWith("xmtp://")) {
    throw new Error("controller XMTP endpoint is invalid");
  }
  const inboxId = endpoint.slice("xmtp://".length);
  if (!isCanonicalId(inboxId)) {
    throw new Error("controller XMTP endpoint is not a canonical inbox ID");
  }

  const manifestUri = manifestServices[0]?.endpoint;
  if (typeof manifestUri !== "string") {
    throw new Error("controller profile lacks its bounded CTHUWU manifest");
  }
  let manifest: unknown;
  try {
    manifest = decodeCanonicalDataJson(manifestUri);
  } catch {
    throw new Error("controller CTHUWU manifest is invalid");
  }
  if (
    !isRecord(manifest) ||
    !hasExactKeys(manifest, [
      "schemaVersion",
      "protocol",
      "tentacleId",
      "erc8004",
      "xmtp",
      "capabilities",
    ]) ||
    manifest.schemaVersion !== 1 ||
    manifest.protocol !== 1 ||
    typeof manifest.tentacleId !== "string" ||
    manifest.tentacleId.length === 0 ||
    Buffer.byteLength(manifest.tentacleId, "utf8") > 128 ||
    !isRecord(manifest.erc8004) ||
    !hasExactKeys(manifest.erc8004, ["chainId", "registry", "agentId"]) ||
    manifest.erc8004.chainId !== 8453 ||
    manifest.erc8004.registry !== ERC8004_IDENTITY_REGISTRY ||
    manifest.erc8004.agentId !== agentId ||
    !isRecord(manifest.xmtp) ||
    !hasExactKeys(manifest.xmtp, ["environment", "endpoint"]) ||
    manifest.xmtp.environment !== CHAT_ENVIRONMENT ||
    manifest.xmtp.endpoint !== endpoint ||
    !Array.isArray(manifest.capabilities) ||
    manifest.capabilities.length > 16 ||
    !manifest.capabilities.every(
      (capability) =>
        typeof capability === "string" &&
        capability.length > 0 &&
        Buffer.byteLength(capability, "utf8") <= 64,
    ) ||
    !manifest.capabilities.includes("direct-xmtp-messaging")
  ) {
    throw new Error("controller CTHUWU manifest does not bind its canonical identity and endpoint");
  }
  return inboxId;
}

function exactMetadata(value: Hex, expected: string): boolean {
  try {
    return value === stringToHex(expected) && hexToString(value) === expected;
  } catch {
    return false;
  }
}

type ChainClient = ReturnType<typeof createAssignmentClient>;

function createAssignmentClient(rpcEndpoint: string) {
  return createPublicClient({
    chain: base,
    transport: http(rpcEndpoint, { timeout: 20_000, retryCount: 1, batch: true }),
  });
}

export class CanonicalAssignmentResolver implements AssignmentResolver {
  readonly #client: ChainClient;
  readonly #brandingAddress: Address | undefined;
  readonly #local: RegistrationSnapshot;

  constructor(options: {
    rpcEndpoint?: string;
    brandingContract?: string;
    localRegistration: RegistrationSnapshot;
  }) {
    this.#client = createAssignmentClient(safeRpcEndpoint(options.rpcEndpoint));
    this.#brandingAddress = configuredBrandingAddress(options.brandingContract);
    this.#local = options.localRegistration;
  }

  async #verifyAgent(
    agentId: string,
    wallet: Address,
    blockNumber: bigint,
  ): Promise<string> {
    const id = BigInt(agentId);
    const [version, agentWallet, authorized, allegiance, protocol, agentUri] =
      await Promise.all([
        this.#client.readContract({
          address: ERC8004_IDENTITY_REGISTRY,
          abi: identityAbi,
          functionName: "getVersion",
          blockNumber,
        }),
        this.#client.readContract({
          address: ERC8004_IDENTITY_REGISTRY,
          abi: identityAbi,
          functionName: "getAgentWallet",
          args: [id],
          blockNumber,
        }),
        this.#client.readContract({
          address: ERC8004_IDENTITY_REGISTRY,
          abi: identityAbi,
          functionName: "isAuthorizedOrOwner",
          args: [wallet, id],
          blockNumber,
        }),
        this.#client.readContract({
          address: ERC8004_IDENTITY_REGISTRY,
          abi: identityAbi,
          functionName: "getMetadata",
          args: [id, "cthuwu.allegiance"],
          blockNumber,
        }),
        this.#client.readContract({
          address: ERC8004_IDENTITY_REGISTRY,
          abi: identityAbi,
          functionName: "getMetadata",
          args: [id, "cthuwu.protocol"],
          blockNumber,
        }),
        this.#client.readContract({
          address: ERC8004_IDENTITY_REGISTRY,
          abi: identityAbi,
          functionName: "tokenURI",
          args: [id],
          blockNumber,
        }),
      ]);
    if (
      version !== ERC8004_VERSION ||
      getAddress(agentWallet) !== wallet ||
      !authorized ||
      !exactMetadata(allegiance, ALLEGIANCE_VALUE) ||
      !exactMetadata(protocol, PROTOCOL_VALUE)
    ) {
      throw new Error("controller failed canonical ERC-8004 wallet/allegiance/protocol checks");
    }
    return endpointFromAgentUri(agentUri, agentId);
  }

  async resolve(address: Address): Promise<AssignmentResolution> {
    try {
      if ((await this.#client.getChainId()) !== 8453) {
        return { kind: "registry_unavailable" };
      }
      const observed = await this.#client.getBlock();
      const blockNumber = observed.number;
      const revision = `${blockNumber}:${observed.hash}`;
      let assignedAgentId = this.#local.agentId;
      let assignedWallet = this.#local.wallet;
      let endpoint: string | undefined;
      let assignedElsewhere = false;

      if (this.#brandingAddress === undefined) {
        if (this.#local.wallet !== INTRO_TENTACLE_ADDRESS) {
          assignedElsewhere = true;
        } else {
          endpoint = await this.#verifyAgent(assignedAgentId, assignedWallet, blockNumber);
        }
      } else {
        const code = await this.#client.getCode({
          address: this.#brandingAddress,
          blockNumber,
        });
        if (code === undefined || code === "0x") {
          return { kind: "registry_unavailable" };
        }
        const [branding, brandingChain, brandingRegistry, brandingUwu, brandingVersion] =
          await Promise.all([
            this.#client.readContract({
              address: this.#brandingAddress,
              abi: brandingAbi,
              functionName: "brandingOf",
              args: [address],
              blockNumber,
            }),
            this.#client.readContract({
              address: this.#brandingAddress,
              abi: brandingAbi,
              functionName: "BASE_CHAIN_ID",
              blockNumber,
            }),
            this.#client.readContract({
              address: this.#brandingAddress,
              abi: brandingAbi,
              functionName: "IDENTITY_REGISTRY",
              blockNumber,
            }),
            this.#client.readContract({
              address: this.#brandingAddress,
              abi: brandingAbi,
              functionName: "UWU",
              blockNumber,
            }),
            this.#client.readContract({
              address: this.#brandingAddress,
              abi: brandingAbi,
              functionName: "REGISTRY_VERSION",
              blockNumber,
            }),
          ]);
        if (
          brandingChain !== 8453n ||
          getAddress(brandingRegistry) !== ERC8004_IDENTITY_REGISTRY ||
          getAddress(brandingUwu) !== CANONICAL_UWU ||
          brandingVersion !== ERC8004_VERSION ||
          getAddress(branding.acolyte) !== address ||
          branding.tokenId !== BigInt(address)
        ) {
          return { kind: "registry_unavailable" };
        }
        if (branding.status === 4) {
          return { kind: "registry_unavailable" };
        }
        if (branding.status === 1) {
          assignedAgentId = branding.controllerAgentId.toString();
          assignedWallet = getAddress(branding.owner);
          if (
            assignedWallet ===
            getAddress("0x0000000000000000000000000000000000000000")
          ) {
            return { kind: "registry_unavailable" };
          }
          endpoint = await this.#verifyAgent(assignedAgentId, assignedWallet, blockNumber);
        } else {
          if (![0, 2, 3].includes(branding.status)) {
            return { kind: "registry_unavailable" };
          }
          if (this.#local.wallet !== INTRO_TENTACLE_ADDRESS) {
            assignedElsewhere = true;
          } else {
            assignedAgentId = this.#local.agentId;
            assignedWallet = this.#local.wallet;
            endpoint = await this.#verifyAgent(assignedAgentId, assignedWallet, blockNumber);
          }
        }
      }

      const canonical = await this.#client.getBlock({ blockNumber });
      if (canonical.hash !== observed.hash) {
        return { kind: "registry_unavailable" };
      }
      if (assignedElsewhere) {
        return { kind: "assigned_elsewhere", revision };
      }
      if (
        assignedAgentId !== this.#local.agentId ||
        assignedWallet !== this.#local.wallet ||
        endpoint !== this.#local.inboxId
      ) {
        return { kind: "assigned_elsewhere", revision };
      }
      return {
        kind: "assigned_here",
        revision,
        tentacleAgentId: assignedAgentId,
        tentacleInboxId: endpoint,
      };
    } catch {
      return { kind: "registry_unavailable" };
    }
  }
}

export type ChatControlConfig = {
  globalGroupId: string;
  globalAdminInboxIds: string[];
  assignmentRevalidateSeconds: number;
};

export function parseGlobalAdminInboxIds(
  value: string | undefined,
  selfInboxId: string,
): string[] {
  if (!isCanonicalId(selfInboxId)) {
    throw new Error("local XMTP inbox ID is not canonical");
  }
  const suppliedAdmins = (value ?? "")
    .split(",")
    .map((candidate) => candidate.trim())
    .filter((candidate) => candidate !== "");
  const admins = [...new Set([selfInboxId, ...suppliedAdmins])].sort();
  if (admins.length > 32 || !admins.every(isCanonicalId)) {
    throw new Error("CTHUWU_GLOBAL_ADMIN_INBOX_IDS contains an invalid inbox ID");
  }
  return admins;
}

export function parseChatControlConfig(
  environment: NodeJS.ProcessEnv,
  selfInboxId: string,
): ChatControlConfig {
  const groupId = environment.CTHUWU_GLOBAL_GROUP_ID;
  if (!isCanonicalId(groupId)) {
    throw new Error("CTHUWU_GLOBAL_GROUP_ID must be one canonical production group ID");
  }
  const admins = parseGlobalAdminInboxIds(
    environment.CTHUWU_GLOBAL_ADMIN_INBOX_IDS,
    selfInboxId,
  );
  const rawInterval = environment.CTHUWU_ASSIGNMENT_REVALIDATE_SECONDS ?? "900";
  if (!/^[0-9]{1,8}$/u.test(rawInterval)) {
    throw new Error("CTHUWU_ASSIGNMENT_REVALIDATE_SECONDS must be an integer");
  }
  const interval = Number(rawInterval);
  if (!Number.isSafeInteger(interval) || interval < 60 || interval > 86_400) {
    throw new Error("CTHUWU_ASSIGNMENT_REVALIDATE_SECONDS must be between 60 and 86400");
  }
  return {
    globalGroupId: groupId,
    globalAdminInboxIds: admins,
    assignmentRevalidateSeconds: interval,
  };
}

export class ChatControlService {
  readonly #directory: GroupDirectory;
  readonly #store: ChatStateStore;
  readonly #resolver: AssignmentResolver;
  readonly #config: ChatControlConfig;
  readonly #selfInboxId: string;
  readonly #tentacleAgentId: string;
  readonly #resolveInboxAddress:
    | ((inboxId: string) => Promise<Address | undefined>)
    | undefined;
  #serial: Promise<void> = Promise.resolve();

  constructor(options: {
    directory: GroupDirectory;
    store: ChatStateStore;
    resolver: AssignmentResolver;
    config: ChatControlConfig;
    selfInboxId: string;
    tentacleAgentId: string;
    resolveInboxAddress?: (inboxId: string) => Promise<Address | undefined>;
  }) {
    this.#directory = options.directory;
    this.#store = options.store;
    this.#resolver = options.resolver;
    this.#config = options.config;
    this.#selfInboxId = options.selfInboxId;
    this.#tentacleAgentId = options.tentacleAgentId;
    this.#resolveInboxAddress = options.resolveInboxAddress;
  }

  async #exclusive<T>(operation: () => Promise<T>): Promise<T> {
    const previous = this.#serial;
    let release: () => void = () => undefined;
    this.#serial = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;
    try {
      return await operation();
    } finally {
      release();
    }
  }

  async #acolytesGroup(state: ChatControlState): Promise<GroupLike> {
    await this.#directory.sync();
    if (state.acolytesGroupId !== undefined) {
      const persisted = await this.#directory.getConversationById(state.acolytesGroupId);
      if (persisted === undefined) {
        throw new Error("persisted Acolytes group is unavailable; refusing replacement");
      }
      await assertAcolytesGroup(persisted, this.#selfInboxId, this.#tentacleAgentId);
      await ensureRetention(persisted);
      return persisted;
    }
    const candidates: GroupLike[] = [];
    for (const group of this.#directory.listGroups()) {
      if (
        await isSelfCreatedWithAppData(
          group,
          this.#selfInboxId,
          acolytesAppData(this.#tentacleAgentId, this.#selfInboxId),
        )
      ) {
        candidates.push(group);
      }
    }
    if (candidates.length > 1) {
      throw new Error("multiple self-created Acolytes groups exist; refusing ambiguous recovery");
    }
    const recovered = candidates[0];
    if (
      recovered !== undefined &&
      (!hasRequiredRetention(recovered) ||
        !(await isTrustedAcolytesGroup(
          recovered,
          this.#selfInboxId,
          this.#tentacleAgentId,
        )))
    ) {
      throw new Error(
        "self-created Acolytes group requires operator repair; refusing a competing replacement",
      );
    }
    const group =
      recovered ??
      (await this.#directory.createGroup([], {
        permissions: GroupPermissionsOptions.AdminOnly,
        groupName: "Cthuwu Acolytes",
        groupDescription: "Acolytes assigned to this Tentacle",
        appData: acolytesAppData(this.#tentacleAgentId, this.#selfInboxId),
        messageDisappearingSettings: {
          fromNs: RETENTION_FROM_NS,
          inNs: RETENTION_IN_NS,
        },
      }));
    await assertAcolytesGroup(group, this.#selfInboxId, this.#tentacleAgentId);
    await ensureRetention(group);
    state.acolytesGroupId = group.id;
    await this.#store.save(state);
    return group;
  }

  async #globalGroup(): Promise<GroupLike> {
    await this.#directory.sync();
    const group = await this.#directory.getConversationById(this.#config.globalGroupId);
    if (group === undefined) {
      throw new Error("configured Global group is unavailable");
    }
    await assertGlobalGroup(
      group,
      this.#config.globalGroupId,
      this.#config.globalAdminInboxIds,
    );
    await ensureRetention(group);
    return group;
  }

  async enroll(options: {
    join: JoinControl;
    senderInboxId: string;
    senderAddress: string | undefined;
    directConversation: ConversationLike;
  }): Promise<AssignmentControl | undefined> {
    return this.#exclusive(async () => {
      if (
        !isCanonicalId(options.senderInboxId) ||
        options.senderAddress === undefined ||
        !isAddress(options.senderAddress, { strict: true })
      ) {
        return undefined;
      }
      const senderAddress = getAddress(options.senderAddress);
      const resolution = await this.#resolver.resolve(senderAddress);
      if (
        resolution.kind !== "assigned_here" ||
        resolution.tentacleAgentId !== this.#tentacleAgentId ||
        resolution.tentacleInboxId !== this.#selfInboxId
      ) {
        return undefined;
      }
      await ensureRetention(options.directConversation);
      const state = await this.#store.load();
      if (
        state.globalGroupId !== undefined &&
        state.globalGroupId !== this.#config.globalGroupId
      ) {
        throw new Error("configured Global group conflicts with persisted binding");
      }
      const acolytes = await this.#acolytesGroup(state);
      const global = await this.#globalGroup();
      for (const group of [acolytes, global]) {
        const members = await group.members();
        if (!members.some((member) => member.inboxId === options.senderInboxId)) {
          await group.addMembers([options.senderInboxId]);
        }
      }
      state.globalGroupId = global.id;
      const enrollment: Enrollment = {
        inboxId: options.senderInboxId,
        address: senderAddress,
        revision: resolution.revision,
      };
      state.enrollments = [
        ...state.enrollments.filter((current) => current.inboxId !== enrollment.inboxId),
        enrollment,
      ].sort((left, right) => left.inboxId.localeCompare(right.inboxId));
      await this.#store.save(state);
      return {
        type: "cthuwu.assignment.v1",
        requestId: options.join.requestId,
        environment: CHAT_ENVIRONMENT,
        revision: resolution.revision,
        tentacleAgentId: this.#tentacleAgentId,
        tentacleInboxId: this.#selfInboxId,
        acolytesGroupId: acolytes.id,
        global: {
          logicalChannelId: GLOBAL_LOGICAL_CHANNEL_ID,
          readConversationIds: [global.id],
          writeConversationId: global.id,
          adminInboxIds: [...this.#config.globalAdminInboxIds],
        },
        retention: {
          fromNs: "1",
          inNs: "1209600000000000",
        },
      };
    });
  }

  async pruneMovedAssignments(): Promise<number> {
    return this.#exclusive(async () => {
      const state = await this.#store.load();
      if (state.acolytesGroupId === undefined) {
        return 0;
      }
      const group = await this.#directory.getConversationById(state.acolytesGroupId);
      if (group === undefined) {
        throw new Error("persisted Acolytes group is unavailable during reassignment sweep");
      }
      await assertAcolytesGroup(group, this.#selfInboxId, this.#tentacleAgentId);
      const removed: string[] = [];
      const retained: Enrollment[] = [];
      const byInbox = new Map(
        state.enrollments.map((enrollment) => [enrollment.inboxId, enrollment]),
      );
      const members = await group.members();
      for (const member of members) {
        if (member.inboxId === this.#selfInboxId) {
          continue;
        }
        const persisted = byInbox.get(member.inboxId);
        if (persisted === undefined) {
          // Group authority comes from authenticated enrollment state, not from an out-of-band
          // group add. Unknown members fail closed and may rejoin through the authenticated DM
          // control path.
          removed.push(member.inboxId);
          continue;
        }
        const address =
          this.#resolveInboxAddress === undefined
            ? persisted.address
            : await this.#resolveInboxAddress(member.inboxId);
        if (address === undefined) {
          // A completed fresh lookup that no longer has exactly one Ethereum identifier is a
          // positive loss of the authenticated binding. Transport failures throw and abort the
          // whole sweep, preserving the group until the next bounded retry.
          removed.push(member.inboxId);
          continue;
        }
        const resolution = await this.#resolver.resolve(address);
        if (resolution.kind === "registry_unavailable") {
          retained.push(persisted);
        } else if (
          resolution.kind === "assigned_here" &&
          resolution.tentacleAgentId === this.#tentacleAgentId &&
          resolution.tentacleInboxId === this.#selfInboxId
        ) {
          retained.push({
            inboxId: member.inboxId,
            address,
            revision: resolution.revision,
          });
        } else {
          removed.push(member.inboxId);
        }
      }
      if (removed.length > 0) {
        const present = removed.filter((inboxId) =>
          members.some((member) => member.inboxId === inboxId),
        );
        if (present.length > 0) {
          await group.removeMembers(present);
        }
      }
      state.enrollments = retained;
      await this.#store.save(state);
      return removed.length;
    });
  }
}

export async function bootstrapGlobalGroup(options: {
  action: "create" | "inspect";
  directory: GroupDirectory;
  selfInboxId: string;
  adminInboxIds: string[];
  configuredGroupId?: string;
}): Promise<{
  groupId: string;
  adminInboxIds: string[];
  created: boolean;
  recovered: boolean;
}> {
  const admins = [...new Set([options.selfInboxId, ...options.adminInboxIds])].sort();
  if (!admins.every(isCanonicalId)) {
    throw new Error("Global bootstrap admin list contains an invalid inbox ID");
  }
  await options.directory.sync();
  if (options.action === "create") {
    if (options.configuredGroupId !== undefined) {
      throw new Error("refusing to create a competing Global group while one is configured");
    }
    const candidates: GroupLike[] = [];
    for (const group of options.directory.listGroups()) {
      if (
        await isSelfCreatedWithAppData(
          group,
          options.selfInboxId,
          globalAppData(),
        )
      ) {
        candidates.push(group);
      }
    }
    if (candidates.length > 1) {
      throw new Error("multiple self-created Global groups exist; refusing ambiguous recovery");
    }
    const recovered = candidates[0];
    if (recovered !== undefined) {
      if (!(await isRecoverableGlobalCandidate(recovered, options.selfInboxId, admins))) {
        throw new Error(
          "self-created Global group requires operator repair or inspect; refusing a competing replacement",
        );
      }
      await reconcileGlobalAdmins(recovered, recovered.id, admins);
      return {
        groupId: recovered.id,
        adminInboxIds: admins,
        created: false,
        recovered: true,
      };
    }
    const group = await options.directory.createGroup(
      admins.filter((inboxId) => inboxId !== options.selfInboxId),
      {
        permissions: GroupPermissionsOptions.AdminOnly,
        groupName: "Cthuwu Global",
        groupDescription: "Cthuwu-wide acolyte channel",
        appData: globalAppData(),
        messageDisappearingSettings: {
          fromNs: RETENTION_FROM_NS,
          inNs: RETENTION_IN_NS,
        },
      },
    );
    for (const inboxId of admins) {
      if (inboxId !== options.selfInboxId && !group.listAdmins().includes(inboxId)) {
        await group.addAdmin(inboxId);
      }
    }
    await assertGlobalGroup(group, group.id, admins);
    await ensureRetention(group);
    return {
      groupId: group.id,
      adminInboxIds: admins,
      created: true,
      recovered: false,
    };
  }
  if (!isCanonicalId(options.configuredGroupId)) {
    throw new Error("Global inspect requires CTHUWU_GLOBAL_GROUP_ID");
  }
  const group = await options.directory.getConversationById(options.configuredGroupId);
  if (group === undefined) {
    throw new Error("configured Global group is unavailable");
  }
  await reconcileGlobalAdmins(group, options.configuredGroupId, admins);
  return {
    groupId: group.id,
    adminInboxIds: admins,
    created: false,
    recovered: false,
  };
}
