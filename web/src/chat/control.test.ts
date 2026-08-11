import { describe, expect, it } from "vitest";
import {
  ASSIGNMENT_CONTENT_TYPE,
  INVALID_CONTROL,
  JOIN_CONTENT_TYPE,
  assignmentCodec,
  createJoinControl,
  isAssignmentContentType,
  joinCodec,
  type AssignmentControl,
} from "./control";

const requestId = "12".repeat(16);
const tentacle = "a".repeat(64);
const acolytes = "b".repeat(64);
const global = "c".repeat(64);

function assignment(): AssignmentControl {
  return {
    type: "cthuwu.assignment.v1",
    requestId,
    environment: "production",
    revision: `123:${`0x${"d".repeat(64)}`}`,
    tentacleAgentId: "42",
    tentacleInboxId: tentacle,
    acolytesGroupId: acolytes,
    global: {
      logicalChannelId: "cthuwu.global.v1",
      readConversationIds: [global],
      writeConversationId: global,
      adminInboxIds: [tentacle],
    },
    retention: { fromNs: "1", inNs: "1209600000000000" },
  };
}

describe("XMTP v1 control codecs", () => {
  it("round-trips exact custom content without push or text fallback", () => {
    const join = createJoinControl(requestId);
    expect(joinCodec.decode(joinCodec.encode(join))).toEqual(join);
    expect(assignmentCodec.decode(assignmentCodec.encode(assignment()))).toEqual(assignment());
    expect(joinCodec.shouldPush(join)).toBe(false);
    expect(joinCodec.fallback(join)).toBeUndefined();
  });

  it("rejects forged type, environment, authority fields, and versions", () => {
    const forgedType = { ...createJoinControl(requestId), type: "cthuwu.assignment.v1" };
    expect(() => joinCodec.encode(forgedType as never)).toThrow(/exact v1 schema/u);
    const forgedEnvironment = { ...assignment(), environment: "dev" };
    expect(() => assignmentCodec.encode(forgedEnvironment as never)).toThrow(/exact v1 schema/u);
    expect(isAssignmentContentType({ ...ASSIGNMENT_CONTENT_TYPE, versionMinor: 1 })).toBe(false);
    expect(isAssignmentContentType({ ...JOIN_CONTENT_TYPE, authorityId: "evil.app", typeId: "assignment" })).toBe(false);
    const encoded = joinCodec.encode(createJoinControl(requestId));
    expect(joinCodec.decode({ ...encoded, parameters: { claim: "trusted" } })).toBe(INVALID_CONTROL);
    expect(joinCodec.decode({ ...encoded, fallback: "cthuwu.join.v1" })).toBe(INVALID_CONTROL);
    expect(joinCodec.decode({ ...encoded, compression: 0 })).toBe(INVALID_CONTROL);
    expect(joinCodec.decode({ ...encoded, content: new Uint8Array([0xff]) })).toBe(INVALID_CONTROL);
  });

  it("rejects noncanonical revisions and duplicate or unbound Global IDs", () => {
    expect(() => assignmentCodec.encode({ ...assignment(), revision: `01:0x${"d".repeat(64)}` })).toThrow();
    expect(() => assignmentCodec.encode({
      ...assignment(),
      global: { ...assignment().global, readConversationIds: [global, global] },
    })).toThrow();
    expect(() => assignmentCodec.encode({
      ...assignment(),
      global: { ...assignment().global, writeConversationId: "e".repeat(64) },
    })).toThrow();
    expect(() => assignmentCodec.encode({
      ...assignment(),
      global: { ...assignment().global, adminInboxIds: ["e".repeat(64)] },
    })).toThrow();
  });
});
