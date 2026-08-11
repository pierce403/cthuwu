import { Bytes, DataSourceContext } from "@graphprotocol/graph-ts";
import {
  afterEach,
  assert,
  clearStore,
  dataSourceMock,
  describe,
  test,
} from "matchstick-as";
import { handleProfile } from "../src/profile";
import { TentacleProfile } from "../generated/schema";

afterEach(() => {
  clearStore();
  dataSourceMock.resetValues();
});

function setProfileContext(id: string): void {
  const context = new DataSourceContext();
  context.setString("profileId", id);
  context.setString("sourceURI", "ipfs://fixture");
  dataSourceMock.setContext(context);
}

describe("bounded registration-v1 files", () => {
  test("accepts a bounded current registration document", () => {
    setProfileContext("ipfs:valid");
    const document =
      '{"type":"https://eips.ethereum.org/EIPS/eip-8004#registration-v1",' +
      '"name":"Fixture Tentacle","description":"Public fixture",' +
      '"image":"ipfs://bafyimage","active":true,"x402Support":false,' +
      '"services":[{"name":"CTHUWU-XMTP","endpoint":"xmtp://0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}],' +
      '"registrations":[{"agentId":7,"agentRegistry":"eip155:8453:0x8004a169fb4a3325136eb29fa0ceb6d2e539a432"}]}';
    handleProfile(Bytes.fromUTF8(document));
    assert.fieldEquals("TentacleProfile", "ipfs:valid", "parseValid", "true");
    assert.fieldEquals("TentacleProfile", "ipfs:valid", "name", "Fixture Tentacle");
    assert.fieldEquals(
      "TentacleProfile",
      "ipfs:valid",
      "xmtpEndpoint",
      "xmtp://0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    );
  });

  test("keeps legacy XMTP name only with a canonical production inbox", () => {
    setProfileContext("ipfs:legacy");
    const document =
      '{"type":"https://eips.ethereum.org/EIPS/eip-8004#registration-v1",' +
      '"name":"Legacy Tentacle","description":"Public fixture",' +
      '"image":"ipfs://bafyimage","active":true,"x402Support":false,' +
      '"services":[{"name":"XMTP","endpoint":"xmtp://abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"}],' +
      '"registrations":[{"agentId":7,"agentRegistry":"eip155:8453:0x8004a169fb4a3325136eb29fa0ceb6d2e539a432"}]}';
    handleProfile(Bytes.fromUTF8(document));
    assert.fieldEquals("TentacleProfile", "ipfs:legacy", "parseValid", "true");
    assert.fieldEquals(
      "TentacleProfile",
      "ipfs:legacy",
      "xmtpEndpoint",
      "xmtp://abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd",
    );
  });

  test("does not persist malformed or non-lowercase XMTP routes", () => {
    setProfileContext("ipfs:xmtp-invalid");
    const document =
      '{"type":"https://eips.ethereum.org/EIPS/eip-8004#registration-v1",' +
      '"name":"Unsafe Route","description":"Public fixture",' +
      '"image":"ipfs://bafyimage","active":true,"x402Support":false,' +
      '"services":[{"name":"CTHUWU-XMTP","endpoint":"xmtp://ABCDEFabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd/path"}],' +
      '"registrations":[{"agentId":7,"agentRegistry":"eip155:8453:0x8004a169fb4a3325136eb29fa0ceb6d2e539a432"}]}';
    handleProfile(Bytes.fromUTF8(document));
    assert.fieldEquals(
      "TentacleProfile",
      "ipfs:xmtp-invalid",
      "parseValid",
      "true",
    );
    const profile = TentacleProfile.load("ipfs:xmtp-invalid");
    assert.assertNotNull(profile);
    assert.assertNull(profile!.xmtpEndpoint);
  });

  test("applies the same bounds to an Arweave file context", () => {
    const context = new DataSourceContext();
    context.setString("profileId", "ar:fixture");
    context.setString("sourceURI", "ar://fixture");
    dataSourceMock.setContext(context);
    const document =
      '{"type":"https://eips.ethereum.org/EIPS/eip-8004#registration-v1",' +
      '"name":"Arweave Tentacle","description":"Public fixture",' +
      '"image":"ar://image","active":true,"x402Support":false,' +
      '"services":[],"registrations":[{"agentId":"8",' +
      '"agentRegistry":"eip155:8453:0x8004a169fb4a3325136eb29fa0ceb6d2e539a432"}]}';
    handleProfile(Bytes.fromUTF8(document));
    assert.fieldEquals("TentacleProfile", "ar:fixture", "parseValid", "true");
  });

  test("rejects malformed, hostile-scheme, and oversized documents safely", () => {
    setProfileContext("ipfs:bad");
    handleProfile(Bytes.fromUTF8("{not-json"));
    assert.fieldEquals("TentacleProfile", "ipfs:bad", "parseValid", "false");

    clearStore();
    setProfileContext("ipfs:scheme");
    const hostile =
      '{"type":"https://eips.ethereum.org/EIPS/eip-8004#registration-v1",' +
      '"name":"Bad","description":"Bad","image":"javascript:alert(1)",' +
      '"active":true,"x402Support":false,"services":[],' +
      '"registrations":[{"agentId":1,"agentRegistry":"eip155:8453:registry"}]}';
    handleProfile(Bytes.fromUTF8(hostile));
    assert.fieldEquals("TentacleProfile", "ipfs:scheme", "parseValid", "false");

    clearStore();
    setProfileContext("ipfs:large");
    handleProfile(new Bytes(32769));
    assert.fieldEquals("TentacleProfile", "ipfs:large", "parseValid", "false");
  });
});
