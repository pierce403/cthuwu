import { describe, expect, it } from "vitest";
import {
  ASSIGNMENT_CONTENT_TYPE,
  INVALID_CONTROL,
  JOIN_CONTENT_TYPE,
  LIVENESS_JOIN_CONTENT_TYPE,
  LIVENESS_QUERY_CONTENT_TYPE,
  LIVENESS_RESPONSE_CONTENT_TYPE,
  TYPING_CONTENT_TYPE,
  assignmentCodec,
  createJoinControl,
  createLivenessJoinControl,
  createLivenessQueryControl,
  isAssignmentContentType,
  joinCodec,
  livenessJoinCodec,
  livenessQueryCodec,
  livenessResponseCodec,
  typingCodec,
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
    const typing = { type: "cthuwu.typing.v1", active: true, expiresAtNs: "1800000000000000000" } as const;
    expect(typingCodec.decode(typingCodec.encode(typing))).toEqual(typing);
    expect(typingCodec.shouldPush(typing)).toBe(false);
    expect(TYPING_CONTENT_TYPE.typeId).toBe("typing");
    const query = createLivenessQueryControl("42", "1800000000000000000", requestId);
    expect(livenessQueryCodec.decode(livenessQueryCodec.encode(query))).toEqual(query);
    const response = {
      type: "cthuwu.liveness-response.v1", requestId, environment: "production",
      phrase: "fhtagn!", tentacleAgentId: "42",
    } as const;
    expect(livenessResponseCodec.decode(livenessResponseCodec.encode(response))).toEqual(response);
    const livenessJoin = createLivenessJoinControl(requestId, "34".repeat(16));
    expect(livenessJoinCodec.decode(livenessJoinCodec.encode(livenessJoin))).toEqual(livenessJoin);
    expect(livenessQueryCodec.shouldPush(query)).toBe(false);
    expect(livenessResponseCodec.shouldPush(response)).toBe(false);
    expect(livenessJoinCodec.shouldPush(livenessJoin)).toBe(false);
  });

  it("uses three separate exact liveness content types with no text fallback", () => {
    expect(LIVENESS_QUERY_CONTENT_TYPE.typeId).toBe("liveness-query");
    expect(LIVENESS_RESPONSE_CONTENT_TYPE.typeId).toBe("liveness-response");
    expect(LIVENESS_JOIN_CONTENT_TYPE.typeId).toBe("liveness-join");
    expect(livenessQueryCodec.fallback(createLivenessQueryControl("42", "1", requestId))).toBeUndefined();
    expect(() => livenessResponseCodec.encode({
      type: "cthuwu.liveness-response.v1", requestId, environment: "dev",
      phrase: "fhtagn!", tentacleAgentId: "42",
    } as never)).toThrow();
  });

  it("rejects altered liveness phrases, extra keys, noncanonical IDs, and uint64 overflow", () => {
    expect(() => createLivenessQueryControl("42", "18446744073709551616", requestId)).toThrow();
    expect(() => createLivenessQueryControl("042", "1", requestId)).toThrow();
    expect(() => livenessQueryCodec.encode({
      ...createLivenessQueryControl("42", "1", requestId), phrase: "fhtagn!",
    } as never)).toThrow();
    expect(() => livenessJoinCodec.encode({
      ...createLivenessJoinControl(requestId), claim: "trusted",
    } as never)).toThrow();
    const encoded = livenessResponseCodec.encode({
      type: "cthuwu.liveness-response.v1", requestId, environment: "production",
      phrase: "fhtagn!", tentacleAgentId: "42",
    });
    expect(livenessResponseCodec.decode({ ...encoded, type: JOIN_CONTENT_TYPE })).toBe(INVALID_CONTROL);
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
