import { describe, expect, it, vi } from "vitest";
import { Interface } from "ethers";
import {
  BRANDING_RUNTIME_HASH,
  consentMatchesOffer,
  encodeBrandingDecline,
  encodeBrandingRequest,
  parseBrandingMessage,
  reviewBrandingOffer,
  verifyBrandingReceipt,
  type BrandingOffer,
} from "./branding-consent";
import {
  CANONICAL_BRANDING_CONTRACT,
  CANONICAL_BRANDING_RUNTIME_HASH,
  type AppConfig,
} from "./config";
import type { StoredIdentity } from "./identity";
import { acolyteName } from "./acolyte-name";

const identity = {
  version: 1,
  environment: "production",
  address: "0x1111111111111111111111111111111111111111",
  walletPrivateKey: `0x${"12".repeat(32)}`,
  compatibilityDbKey: `0x${"34".repeat(32)}`,
  createdAt: "2026-08-11T00:00:00.000Z",
} satisfies StoredIdentity;
const minter = "0x2222222222222222222222222222222222222222";
const referrer = "0x3333333333333333333333333333333333333333";
const offerId = "12".repeat(16);
const blockHash = `0x${"a".repeat(64)}`;
const name = acolyteName(identity.address);
const nameHex = `0x${Array.from(new TextEncoder().encode(name), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
const config: AppConfig = {
  environment: "production",
  botAddress: minter,
  baseRpcEndpoint: "https://rpc.example/",
  brandingContract: CANONICAL_BRANDING_CONTRACT,
  assignmentRefreshMs: 600_000,
};
const brandingInterface = new Interface([
  "function BASE_CHAIN_ID() view returns (uint256)",
  "function IDENTITY_REGISTRY() view returns (address)",
  "function UWU() view returns (address)",
  "function REGISTRY_VERSION() view returns (string)",
  "function DOMAIN_NAME() view returns (string)",
  "function DOMAIN_VERSION() view returns (string)",
  "function brandingOf(address acolyte) view returns (tuple(uint256 tokenId,address acolyte,address owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 pendingDeclaredPrice,uint256 pendingPriceActivation,uint8 status) result)",
  "function nonces(address acolyte) view returns (uint256)",
  "function customTraitCount(uint256 tokenId) view returns (uint256)",
  "function customTraitAt(uint256 tokenId,uint256 index) view returns (string traitType,string value)",
]);

function offerMarker(overrides: Partial<Record<
  "offer" | "contract" | "minter" | "agent" | "acolyte" | "referrer" | "treasury" |
  "basis" | "price" | "upkeep" | "nonce" | "deadline" | "block" | "blockHash" | "name",
  string
>> = {}): string {
  const value = {
    offer: offerId, contract: CANONICAL_BRANDING_CONTRACT, minter, agent: "42",
    acolyte: identity.address, referrer, treasury: "10000", basis: "1000", price: "1000",
    upkeep: "1", nonce: "0", deadline: "2000000000", block: "123", blockHash, name: nameHex,
    ...overrides,
  };
  return `[[cthuwu:branding-offer:v2;offer=${value.offer};contract=${value.contract};minter=${value.minter};agent=${value.agent};acolyte=${value.acolyte};referrer=${value.referrer};treasury=${value.treasury};basis=${value.basis};price=${value.price};upkeep=${value.upkeep};nonce=${value.nonce};deadline=${value.deadline};block=${value.block};blockHash=${value.blockHash};name=${value.name}]]`;
}

function parsedOffer(marker = offerMarker()): BrandingOffer {
  const control = parseBrandingMessage(marker, "theirs").control;
  if (!control || control.type !== "offer") throw new Error("offer fixture did not parse");
  return control;
}

describe("Acolyte Branding consent controls", () => {
  it("pins the reviewed production runtime hash", () => {
    expect(BRANDING_RUNTIME_HASH).toBe(CANONICAL_BRANDING_RUNTIME_HASH);
  });

  it("parses one exact ordered v2 offer only from the Tentacle and keeps its prose", () => {
    const marker = offerMarker();
    const parsed = parseBrandingMessage(`an exact invitation\n${marker}`, "theirs");
    expect(parsed.text).toBe("an exact invitation");
    expect(parsed.control).toMatchObject({
      type: "offer", offerId, contract: CANONICAL_BRANDING_CONTRACT, minter,
      controllerAgentId: 42n, acolyte: identity.address, referrer,
      treasury: 10_000n, basisPoints: 1_000n, initialDeclaredPrice: 1_000n,
      firstWeekUpkeep: 1n, nonce: 0n, deadline: 2_000_000_000n,
      blockNumber: 123n, blockHash, name,
    });
    expect(parseBrandingMessage(marker, "mine")).toEqual({ text: marker });
  });

  it("leaves malformed, reordered, duplicated, embedded, or nonterminal controls literal", () => {
    const marker = offerMarker();
    expect(parseBrandingMessage(marker.replace(";contract=", ";extra=1;contract="), "theirs")).toEqual({
      text: marker.replace(";contract=", ";extra=1;contract="),
    });
    expect(parseBrandingMessage(`${marker}\n${marker}`, "theirs")).toEqual({ text: `${marker}\n${marker}` });
    expect(parseBrandingMessage(`${marker}\nordinary suffix`, "theirs")).toEqual({ text: `${marker}\nordinary suffix` });
    expect(parseBrandingMessage(`ordinary ${marker} tail`, "theirs")).toEqual({ text: `ordinary ${marker} tail` });
    expect(parseBrandingMessage(offerMarker({ offer: "AB".repeat(16) }), "theirs").control).toBeUndefined();
  });

  it("recovers an exact outbound consent from Direct history without localStorage", () => {
    const offer = parsedOffer();
    const marker = `[[cthuwu:branding-consent:v2;offer=${offerId};contract=${CANONICAL_BRANDING_CONTRACT};minter=${minter};agent=42;acolyte=${identity.address};referrer=${referrer};price=1000;nonce=0;deadline=2000000000;block=123;blockHash=${blockHash};name=${nameHex};signature=0x11]]`;
    const parsed = parseBrandingMessage(marker, "mine");
    expect(parsed.text).toBe("");
    expect(parsed.control?.type).toBe("consent");
    if (!parsed.control || parsed.control.type !== "consent") throw new Error("consent did not parse");
    expect(consentMatchesOffer(parsed.control, offer)).toBe(true);
    expect(parseBrandingMessage(`I agree\n${marker}`, "mine")).toEqual({ text: `I agree\n${marker}` });
    expect(parseBrandingMessage(marker, "theirs")).toEqual({ text: marker });
  });

  it("encodes sole strict decline and referrer/name replacement requests", () => {
    const decline = encodeBrandingDecline(offerId);
    expect(parseBrandingMessage(decline, "mine")).toMatchObject({
      text: "", control: { type: "decline", offerId },
    });
    const request = encodeBrandingRequest(referrer, name);
    expect(request).toBe(`[[cthuwu:branding-request:v2;referrer=${referrer};name=${nameHex}]]`);
    expect(parseBrandingMessage(request, "mine")).toMatchObject({
      text: "", control: { type: "request", referrer, name },
    });
    expect(() => encodeBrandingDecline("0".repeat(31))).toThrow();
  });

  it.each([
    ["wrong assigned route", offerMarker(), "0x4444444444444444444444444444444444444444"],
    ["wrong acolyte", offerMarker({ acolyte: "0x4444444444444444444444444444444444444444" }), minter],
    ["bad basis", offerMarker({ basis: "499", price: "499" }), minter],
    ["bad price arithmetic", offerMarker({ price: "999" }), minter],
    ["bad upkeep arithmetic", offerMarker({ upkeep: "2" }), minter],
    ["wrong deterministic name", offerMarker({ name: "0x626f677573" }), minter],
  ])("rejects %s before making a Base request", async (_label, marker, expectedMinter) => {
    const rpc = { request: vi.fn(async () => { throw new Error("must not call RPC"); }) };
    await expect(reviewBrandingOffer(config, identity, parsedOffer(marker), expectedMinter, { rpc }))
      .rejects.toThrow();
    expect(rpc.request).not.toHaveBeenCalled();
  });

  it("refuses to sign an offer that replaces a fragment-pinned referrer", async () => {
    const rpc = { request: vi.fn() };
    await expect(reviewBrandingOffer({ ...config, referrer: identity.address }, identity, parsedOffer(), minter, { rpc }))
      .rejects.toThrow(/pinned referrer/u);
    expect(rpc.request).not.toHaveBeenCalled();
  });

  it("rejects a mismatched receipt before consulting Base", async () => {
    const offer = parsedOffer();
    const receiptMarker = `[[cthuwu:branding-receipt:v2;offer=${offerId};contract=${CANONICAL_BRANDING_CONTRACT};token=${BigInt(identity.address)};agent=42;acolyte=${identity.address};owner=0x4444444444444444444444444444444444444444;referrer=${referrer};price=1000;nonce=0;block=130;blockHash=${blockHash};name=${nameHex}]]`;
    const receipt = parseBrandingMessage(receiptMarker, "theirs").control;
    if (!receipt || receipt.type !== "receipt") throw new Error("receipt did not parse");
    const rpc = { request: vi.fn() };
    await expect(verifyBrandingReceipt(config, identity, offer, receipt, { rpc }))
      .rejects.toThrow(/does not match/u);
    expect(rpc.request).not.toHaveBeenCalled();
  });

  it("confirms a delayed name repair after expiry and repricing from immutable mint state", async () => {
    const offer = parsedOffer();
    const receiptBlock = 130n;
    const receiptMarker = `[[cthuwu:branding-receipt:v2;offer=${offerId};contract=${CANONICAL_BRANDING_CONTRACT};token=${BigInt(identity.address)};agent=42;acolyte=${identity.address};owner=${minter};referrer=${referrer};price=1000;nonce=0;block=${receiptBlock};blockHash=${blockHash};name=${nameHex}]]`;
    const receipt = parseBrandingMessage(receiptMarker, "theirs").control;
    if (!receipt || receipt.type !== "receipt") throw new Error("receipt did not parse");
    const rpc = {
      request: vi.fn(async (method: string, params: unknown[]): Promise<unknown> => {
        if (method === "eth_getBlockByNumber") {
          return { number: "0x82", hash: blockHash, timestamp: "0x3e8" };
        }
        if (method === "eth_getCode") return "0x6000";
        if (method !== "eth_call") throw new Error(`unexpected ${method}`);
        const call = params[0] as { data: string };
        const selector = call.data.slice(0, 10);
        if (selector === brandingInterface.getFunction("BASE_CHAIN_ID")!.selector) {
          return brandingInterface.encodeFunctionResult("BASE_CHAIN_ID", [8453n]);
        }
        if (selector === brandingInterface.getFunction("IDENTITY_REGISTRY")!.selector) {
          return brandingInterface.encodeFunctionResult("IDENTITY_REGISTRY", [
            "0x8004A169FB4a3325136EB29fA0ceB6D2e539a432",
          ]);
        }
        if (selector === brandingInterface.getFunction("UWU")!.selector) {
          return brandingInterface.encodeFunctionResult("UWU", [
            "0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07",
          ]);
        }
        if (selector === brandingInterface.getFunction("REGISTRY_VERSION")!.selector) {
          return brandingInterface.encodeFunctionResult("REGISTRY_VERSION", ["2.0.0"]);
        }
        if (selector === brandingInterface.getFunction("DOMAIN_NAME")!.selector) {
          return brandingInterface.encodeFunctionResult("DOMAIN_NAME", ["Cthuwu Acolyte Branding"]);
        }
        if (selector === brandingInterface.getFunction("DOMAIN_VERSION")!.selector) {
          return brandingInterface.encodeFunctionResult("DOMAIN_VERSION", ["1"]);
        }
        if (selector === brandingInterface.getFunction("brandingOf")!.selector) {
          return brandingInterface.encodeFunctionResult("brandingOf", [[
            BigInt(identity.address), identity.address, minter, 42n, referrer,
            2_000n, 999n, 0n, 0n, 2,
          ]]);
        }
        if (selector === brandingInterface.getFunction("nonces")!.selector) {
          return brandingInterface.encodeFunctionResult("nonces", [1n]);
        }
        if (selector === brandingInterface.getFunction("customTraitCount")!.selector) {
          return brandingInterface.encodeFunctionResult("customTraitCount", [1n]);
        }
        if (selector === brandingInterface.getFunction("customTraitAt")!.selector) {
          return brandingInterface.encodeFunctionResult("customTraitAt", ["Acolyte Name", name]);
        }
        throw new Error("unexpected Branding call");
      }),
    };
    await expect(verifyBrandingReceipt(config, identity, offer, receipt, {
      rpc,
      hashCode: () => CANONICAL_BRANDING_RUNTIME_HASH,
    })).resolves.toBeUndefined();
  });

  it("rejects a receipt that replaces the signed initial price before consulting Base", async () => {
    const offer = parsedOffer();
    const receiptMarker = `[[cthuwu:branding-receipt:v2;offer=${offerId};contract=${CANONICAL_BRANDING_CONTRACT};token=${BigInt(identity.address)};agent=42;acolyte=${identity.address};owner=${minter};referrer=${referrer};price=2000;nonce=0;block=130;blockHash=${blockHash};name=${nameHex}]]`;
    const receipt = parseBrandingMessage(receiptMarker, "theirs").control;
    if (!receipt || receipt.type !== "receipt") throw new Error("receipt did not parse");
    const rpc = { request: vi.fn() };
    await expect(verifyBrandingReceipt(config, identity, offer, receipt, { rpc }))
      .rejects.toThrow(/does not match/u);
    expect(rpc.request).not.toHaveBeenCalled();
  });
});
