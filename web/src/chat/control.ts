import type {
  ContentCodec,
  ContentTypeId,
  EncodedContent,
} from "@xmtp/content-type-primitives";
import { RETENTION_FROM_NS, RETENTION_IN_NS } from "./types";

export const JOIN_CONTENT_TYPE = {
  authorityId: "cthuwu.app",
  typeId: "join",
  versionMajor: 1,
  versionMinor: 0,
} as const satisfies ContentTypeId;
export const ASSIGNMENT_CONTENT_TYPE = {
  authorityId: "cthuwu.app",
  typeId: "assignment",
  versionMajor: 1,
  versionMinor: 0,
} as const satisfies ContentTypeId;
export const TYPING_CONTENT_TYPE = {
  authorityId: "cthuwu.app",
  typeId: "typing",
  versionMajor: 1,
  versionMinor: 0,
} as const satisfies ContentTypeId;
const HEX_32 = /^[0-9a-f]{32}$/u;
const HEX_64 = /^[0-9a-f]{64}$/u;
const DECIMAL = /^(0|[1-9][0-9]{0,77})$/u;
const REVISION = /^(0|[1-9][0-9]{0,31}):0x[0-9a-f]{64}$/u;
const MAX_GLOBAL_READ_CONVERSATIONS = 32;
const EXPECTED_RETENTION = Object.freeze({
  fromNs: RETENTION_FROM_NS.toString(),
  inNs: RETENTION_IN_NS.toString(),
});

export interface JoinControl {
  type: "cthuwu.join.v1";
  requestId: string;
  environment: "production";
}

export interface GlobalControlBinding {
  logicalChannelId: "cthuwu.global.v1";
  readConversationIds: string[];
  writeConversationId: string;
  adminInboxIds: string[];
}

export interface AssignmentControl {
  type: "cthuwu.assignment.v1";
  requestId: string;
  environment: "production";
  revision: string;
  tentacleAgentId: string;
  tentacleInboxId: string;
  acolytesGroupId: string;
  global: GlobalControlBinding;
  retention: typeof EXPECTED_RETENTION;
}
export interface TypingControl {
  type: "cthuwu.typing.v1";
  active: boolean;
  expiresAtNs: string;
}

export type ControlMessage = JoinControl | AssignmentControl;
export const INVALID_CONTROL = Object.freeze({ invalidControl: true });

export function createJoinControl(requestId = randomRequestId()): JoinControl {
  if (!HEX_32.test(requestId)) throw new Error("join request ID is invalid");
  return { type: "cthuwu.join.v1", requestId, environment: "production" };
}

export function isJoinControl(value: unknown): value is JoinControl {
  return isRecord(value) && parseJoin(value) !== undefined;
}

export const joinCodec: ContentCodec<unknown> = createCodec(JOIN_CONTENT_TYPE, (value) =>
  parseJoin(value),
);
export const assignmentCodec: ContentCodec<unknown> = createCodec(
  ASSIGNMENT_CONTENT_TYPE,
  (value) => parseAssignment(value),
);
export const typingCodec: ContentCodec<unknown> = createCodec(
  TYPING_CONTENT_TYPE,
  (value) => parseTyping(value),
);
export const CONTROL_CODECS = [joinCodec, assignmentCodec, typingCodec] as const;

export function encodeJoinControl(message: JoinControl): EncodedContent {
  return joinCodec.encode(message);
}

export function isJoinContentType(value: ContentTypeId | undefined): boolean {
  return exactContentType(value, JOIN_CONTENT_TYPE);
}

export function isAssignmentContentType(value: ContentTypeId | undefined): boolean {
  return exactContentType(value, ASSIGNMENT_CONTENT_TYPE);
}

export function isControlContentType(value: ContentTypeId | undefined): boolean {
  return isJoinContentType(value) || isAssignmentContentType(value) || isTypingContentType(value);
}

export function isTypingContentType(value: ContentTypeId | undefined): boolean {
  return exactContentType(value, TYPING_CONTENT_TYPE);
}

export function isTypingControl(value: unknown): value is TypingControl {
  return isRecord(value) && parseTyping(value) !== undefined;
}

function parseJoin(value: Record<string, unknown>): JoinControl | undefined {
  if (!hasExactKeys(value, ["environment", "requestId", "type"])) return undefined;
  if (
    value.type !== "cthuwu.join.v1" ||
    value.environment !== "production" ||
    !isRequestId(value.requestId)
  ) return undefined;
  return {
    type: "cthuwu.join.v1",
    requestId: value.requestId,
    environment: "production",
  };
}

function parseAssignment(value: Record<string, unknown>): AssignmentControl | undefined {
  if (
    !hasExactKeys(value, [
      "acolytesGroupId",
      "environment",
      "global",
      "requestId",
      "retention",
      "revision",
      "tentacleAgentId",
      "tentacleInboxId",
      "type",
    ]) ||
    value.type !== "cthuwu.assignment.v1" ||
    value.environment !== "production" ||
    !isRequestId(value.requestId) ||
    typeof value.revision !== "string" ||
    !REVISION.test(value.revision) ||
    !isDecimal(value.tentacleAgentId) ||
    !isHex64(value.tentacleInboxId) ||
    !isHex64(value.acolytesGroupId) ||
    !isRecord(value.retention) ||
    !hasExactKeys(value.retention, ["fromNs", "inNs"]) ||
    value.retention.fromNs !== EXPECTED_RETENTION.fromNs ||
    value.retention.inNs !== EXPECTED_RETENTION.inNs ||
    !isRecord(value.global)
  ) {
    return undefined;
  }
  const global = parseGlobalBinding(value.global);
  if (!global || !global.adminInboxIds.includes(value.tentacleInboxId)) return undefined;
  return {
    type: "cthuwu.assignment.v1",
    requestId: value.requestId,
    environment: "production",
    revision: value.revision,
    tentacleAgentId: value.tentacleAgentId,
    tentacleInboxId: value.tentacleInboxId,
    acolytesGroupId: value.acolytesGroupId,
    global,
    retention: EXPECTED_RETENTION,
  };
}

function parseTyping(value: Record<string, unknown>): TypingControl | undefined {
  if (
    !hasExactKeys(value, ["active", "expiresAtNs", "type"]) ||
    value.type !== "cthuwu.typing.v1" ||
    typeof value.active !== "boolean" ||
    typeof value.expiresAtNs !== "string" ||
    !/^[1-9][0-9]{0,19}$/u.test(value.expiresAtNs)
  ) return undefined;
  return { type: "cthuwu.typing.v1", active: value.active, expiresAtNs: value.expiresAtNs };
}

function createCodec<T extends ControlMessage | TypingControl>(
  contentType: ContentTypeId,
  validate: (value: Record<string, unknown>) => T | undefined,
): ContentCodec<unknown> {
  return {
    contentType,
    encode: (content) => {
      const value = isRecord(content) ? validate(content) : undefined;
      if (!value) throw new Error("control content does not match its exact v1 schema");
      return {
        type: contentType,
        parameters: {},
        content: new TextEncoder().encode(JSON.stringify(value)),
      };
    },
    decode: (encoded) => {
      try {
        if (
          !exactContentType(encoded.type, contentType) ||
          encoded.content.length > 8 * 1024 ||
          Object.keys(encoded.parameters).length !== 0 ||
          encoded.fallback !== undefined ||
          encoded.compression !== undefined
        ) {
          return INVALID_CONTROL;
        }
        const parsed = JSON.parse(
          new TextDecoder("utf-8", { fatal: true }).decode(encoded.content),
        ) as unknown;
        if (!isRecord(parsed)) return INVALID_CONTROL;
        return validate(parsed) ?? INVALID_CONTROL;
      } catch {
        // Registered network codecs must never let hostile control bytes tear down the stream.
        return INVALID_CONTROL;
      }
    },
    fallback: () => undefined,
    shouldPush: () => false,
  };
}

function exactContentType(
  value: ContentTypeId | undefined,
  expected: ContentTypeId,
): boolean {
  return Boolean(
    value &&
      value.authorityId === expected.authorityId &&
      value.typeId === expected.typeId &&
      value.versionMajor === expected.versionMajor &&
      value.versionMinor === expected.versionMinor,
  );
}

function parseGlobalBinding(value: Record<string, unknown>): GlobalControlBinding | undefined {
  if (
    !hasExactKeys(value, [
      "adminInboxIds",
      "logicalChannelId",
      "readConversationIds",
      "writeConversationId",
    ]) ||
    value.logicalChannelId !== "cthuwu.global.v1" ||
    !Array.isArray(value.readConversationIds) ||
    value.readConversationIds.length === 0 ||
    value.readConversationIds.length > MAX_GLOBAL_READ_CONVERSATIONS ||
    !Array.isArray(value.adminInboxIds) ||
    value.adminInboxIds.length === 0 ||
    value.adminInboxIds.length > 32 ||
    !isHex64(value.writeConversationId)
  ) {
    return undefined;
  }
  const readConversationIds = uniqueHex64(value.readConversationIds);
  const adminInboxIds = uniqueHex64(value.adminInboxIds);
  if (!readConversationIds || !adminInboxIds || !readConversationIds.includes(value.writeConversationId)) {
    return undefined;
  }
  return {
    logicalChannelId: "cthuwu.global.v1",
    readConversationIds,
    writeConversationId: value.writeConversationId,
    adminInboxIds,
  };
}

function randomRequestId(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

function uniqueHex64(value: unknown[]): string[] | undefined {
  if (!value.every(isHex64)) return undefined;
  const unique = [...new Set(value)];
  return unique.length === value.length ? unique : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isRequestId(value: unknown): value is string {
  return typeof value === "string" && HEX_32.test(value);
}

export function isHex64(value: unknown): value is string {
  return typeof value === "string" && HEX_64.test(value);
}

function isDecimal(value: unknown): value is string {
  return typeof value === "string" && DECIMAL.test(value) && BigInt(value) < 1n << 256n;
}

function hasExactKeys(value: Record<string, unknown>, expected: string[]): boolean {
  const keys = Object.keys(value).sort();
  return keys.length === expected.length && keys.every((key, index) => key === expected[index]);
}
