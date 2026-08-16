import {
  Interface,
  TypedDataEncoder,
  Wallet,
  getAddress,
  getBytes,
  hexlify,
  keccak256,
  toUtf8Bytes,
  verifyTypedData,
  type TypedDataField,
} from "ethers";
import type { AppConfig } from "./config";
import {
  CANONICAL_BRANDING_CONTRACT,
  CANONICAL_BRANDING_RUNTIME_HASH,
} from "./config";
import type { StoredIdentity } from "./identity";
import { isLocalIdentity } from "./identity";
import {
  ALLEGIANCE_HEX,
  BASE_CHAIN_ID,
  IDENTITY_REGISTRY,
  PROTOCOL_V1_HEX,
  UWU_CONTRACT,
  ZERO_ADDRESS,
} from "./leaderboard-types";
import { acolyteName } from "./acolyte-name";
import { createJsonRpcClient, type RpcClient } from "./chat/assignment";

export const BRANDING_RUNTIME_HASH = CANONICAL_BRANDING_RUNTIME_HASH;
export const BRANDING_DOMAIN_NAME = "Cthuwu Acolyte Branding";
export const BRANDING_DOMAIN_VERSION = "1";
export const ACOLYTE_NAME_TRAIT = "Acolyte Name";

const MAX_UINT256 = (1n << 256n) - 1n;
const MIN_SIGNING_WINDOW_SECONDS = 120n;
const OFFER_ID = /^[0-9a-f]{32}$/u;
const ADDRESS = /^0x[0-9a-f]{40}$/u;
const HASH = /^0x[0-9a-f]{64}$/u;
const HEX_BYTES = /^0x(?:[0-9a-f]{2})+$/u;
const DECIMAL = /^(?:0|[1-9][0-9]*)$/u;
const QUANTITY = /^0x(?:0|[1-9a-f][0-9a-f]*)$/u;
const NONEMPTY_CODE = /^0x(?:[0-9a-fA-F]{2})+$/u;

const BRANDING = new Interface([
  "function BASE_CHAIN_ID() view returns (uint256)",
  "function IDENTITY_REGISTRY() view returns (address)",
  "function UWU() view returns (address)",
  "function REGISTRY_VERSION() view returns (string)",
  "function DOMAIN_NAME() view returns (string)",
  "function DOMAIN_VERSION() view returns (string)",
  "function brandingOf(address acolyte) view returns (tuple(uint256 tokenId,address acolyte,address owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 pendingDeclaredPrice,uint256 pendingPriceActivation,uint8 status) result)",
  "function nonces(address acolyte) view returns (uint256)",
  "function weeklyUpkeepForPrice(uint256 price) view returns (uint256)",
  "function consentDigest((address acolyte,address minter,uint256 controllerAgentId,address referrer,uint256 initialDeclaredPrice,uint256 nonce,uint256 deadline) consent) view returns (bytes32)",
  "function customTraitCount(uint256 tokenId) view returns (uint256)",
  "function customTraitAt(uint256 tokenId,uint256 index) view returns (string traitType,string value)",
]);
const REGISTRY = new Interface([
  "function getVersion() view returns (string)",
  "function getAgentWallet(uint256 agentId) view returns (address)",
  "function isAuthorizedOrOwner(address wallet,uint256 agentId) view returns (bool)",
  "function getMetadata(uint256 agentId,string key) view returns (bytes)",
]);
const UWU = new Interface(["function balanceOf(address account) view returns (uint256)"]);
const ERC1271 = new Interface(["function isValidSignature(bytes32 digest,bytes signature) view returns (bytes4)"]);

export interface BrandingOffer {
  type: "offer";
  offerId: string;
  contract: string;
  minter: string;
  controllerAgentId: bigint;
  acolyte: string;
  referrer: string;
  treasury: bigint;
  basisPoints: bigint;
  initialDeclaredPrice: bigint;
  firstWeekUpkeep: bigint;
  nonce: bigint;
  deadline: bigint;
  blockNumber: bigint;
  blockHash: string;
  name: string;
  marker: string;
}

export interface BrandingConsent {
  type: "consent";
  offerId: string;
  contract: string;
  minter: string;
  controllerAgentId: bigint;
  acolyte: string;
  referrer: string;
  initialDeclaredPrice: bigint;
  nonce: bigint;
  deadline: bigint;
  blockNumber: bigint;
  blockHash: string;
  name: string;
  signature: string;
  marker: string;
}

export interface BrandingReceipt {
  type: "receipt";
  offerId: string;
  contract: string;
  tokenId: bigint;
  controllerAgentId: bigint;
  acolyte: string;
  owner: string;
  referrer: string;
  initialDeclaredPrice: bigint;
  nonce: bigint;
  blockNumber: bigint;
  blockHash: string;
  name: string;
  marker: string;
}

export interface BrandingDecline {
  type: "decline";
  offerId: string;
  marker: string;
}

export interface BrandingRequest {
  type: "request";
  referrer: string;
  name: string;
  marker: string;
}

export type BrandingControl = BrandingOffer | BrandingConsent | BrandingReceipt | BrandingDecline | BrandingRequest;

export interface ParsedBrandingMessage {
  text: string;
  control?: BrandingControl;
}

export interface BrandingReview {
  offer: BrandingOffer;
  digest: string;
  domain: {
    name: typeof BRANDING_DOMAIN_NAME;
    version: typeof BRANDING_DOMAIN_VERSION;
    chainId: typeof BASE_CHAIN_ID;
    verifyingContract: string;
  };
}

export interface BrandingConsentDependencies {
  rpc?: RpcClient;
  fetch?: typeof fetch;
  hashCode?: (code: string) => string;
  nowSeconds?: () => bigint;
  signExternal?: (identity: StoredIdentity, review: BrandingReview) => Promise<string>;
}

export function parseBrandingMessage(
  text: string,
  direction: "mine" | "theirs",
): ParsedBrandingMessage {
  const markerMatches = text.match(/\[\[cthuwu:branding-(?:offer|consent|receipt|decline|request):v2;[^\r\n]*\]\]/gu);
  if (!markerMatches || markerMatches.length !== 1) return { text };
  const marker = markerMatches[0]!;
  const position = text.indexOf(marker);
  if (text.slice(position + marker.length).trim() !== "") return { text };
  const prefix = text.slice(0, position);
  // A malformed or unsupported Branding-looking marker alongside a valid terminal marker must
  // never be interpreted partially. Controls are fail-closed: the entire message stays literal.
  if (prefix.includes("[[cthuwu:branding-")) return { text };
  let control: BrandingControl | undefined;
  try {
    if (marker.startsWith("[[cthuwu:branding-offer:v2;")) {
      if (direction !== "theirs") return { text };
      control = parseOffer(marker);
    } else if (marker.startsWith("[[cthuwu:branding-receipt:v2;")) {
      if (direction !== "theirs") return { text };
      control = parseReceipt(marker);
    } else if (marker.startsWith("[[cthuwu:branding-consent:v2;")) {
      if (direction !== "mine" || prefix.trim() !== "") return { text };
      control = parseConsent(marker);
    } else if (marker.startsWith("[[cthuwu:branding-decline:v2;")) {
      if (direction !== "mine" || prefix.trim() !== "") return { text };
      control = parseDecline(marker);
    } else {
      if (direction !== "mine" || prefix.trim() !== "") return { text };
      control = parseRequest(marker);
    }
  } catch {
    return { text };
  }
  return { text: prefix.trimEnd(), control };
}

export function encodeBrandingDecline(offerId: string): string {
  if (!OFFER_ID.test(offerId)) throw new Error("Branding offer ID is invalid");
  return `[[cthuwu:branding-decline:v2;offer=${offerId}]]`;
}

export function encodeBrandingRequest(referrer: string, name: string): string {
  const canonicalReferrer = canonicalAddress(referrer);
  if (canonicalReferrer === ZERO_ADDRESS) throw new Error("Branding referrer cannot be zero");
  if (!name || name.length > 256) throw new Error("Branding name is invalid");
  return `[[cthuwu:branding-request:v2;referrer=${canonicalReferrer};name=${encodeName(name)}]]`;
}

export async function reviewBrandingOffer(
  config: AppConfig,
  identity: StoredIdentity,
  offer: BrandingOffer,
  expectedMinter: string | undefined,
  dependencies: BrandingConsentDependencies = {},
): Promise<BrandingReview> {
  const rpc = dependencies.rpc ?? createJsonRpcClient(
    config.baseRpcEndpoint,
    dependencies.fetch ?? fetch,
  );
  validateOfferBinding(config, identity, offer, expectedMinter);
  return inspectOfferAtBlock(
    rpc,
    offer,
    offer.blockNumber,
    offer.blockHash,
    dependencies.hashCode,
  );
}

export async function signBrandingOffer(
  config: AppConfig,
  identity: StoredIdentity,
  review: BrandingReview,
  expectedMinter: string | undefined,
  dependencies: BrandingConsentDependencies = {},
): Promise<BrandingConsent> {
  const rpc = dependencies.rpc ?? createJsonRpcClient(
    config.baseRpcEndpoint,
    dependencies.fetch ?? fetch,
  );
  const offer = review.offer;
  validateOfferBinding(config, identity, offer, expectedMinter);
  const signature = canonicalSignature(isLocalIdentity(identity)
    ? await new Wallet(identity.walletPrivateKey).signTypedData(
        review.domain,
        MINT_CONSENT_TYPES,
        mintConsentValue(offer),
      )
    : await (dependencies.signExternal ?? defaultExternalSigner)(identity, review));

  // The wallet prompt is an unbounded user pause. Re-read every mutable or upgradeable input at a
  // fresh head before releasing the signature to XMTP, while also proving the originally reviewed
  // block has not been replaced.
  const original = await rpc.request("eth_getBlockByNumber", [blockTag(offer.blockNumber), false]);
  const originalHeader = parseBlock(original);
  if (originalHeader.hash !== offer.blockHash || originalHeader.number !== offer.blockNumber) {
    throw new Error("The reviewed Base block changed while the consent was being signed");
  }
  const headNumber = rpcQuantity(await rpc.request("eth_blockNumber", []), "Base head");
  const head = parseBlock(await rpc.request("eth_getBlockByNumber", [blockTag(headNumber), false]));
  if (head.number !== headNumber) throw new Error("Base returned a mismatched fresh head");
  const now = dependencies.nowSeconds?.() ?? BigInt(Math.floor(Date.now() / 1000));
  if (offer.deadline < head.timestamp + MIN_SIGNING_WINDOW_SECONDS ||
      offer.deadline < now + MIN_SIGNING_WINDOW_SECONDS) {
    throw new Error("The Branding consent deadline is too close; ask the Tentacle for a fresh offer");
  }
  const fresh = await inspectOfferAtBlock(
    rpc,
    offer,
    head.number,
    head.hash,
    dependencies.hashCode,
  );
  if (fresh.digest !== review.digest) {
    throw new Error("The canonical Branding consent digest changed while signing");
  }
  await verifyConsentSignature(rpc, identity, fresh, signature, blockTag(head.number));

  const consent: BrandingConsent = {
    type: "consent",
    offerId: offer.offerId,
    contract: offer.contract,
    minter: offer.minter,
    controllerAgentId: offer.controllerAgentId,
    acolyte: offer.acolyte,
    referrer: offer.referrer,
    initialDeclaredPrice: offer.initialDeclaredPrice,
    nonce: offer.nonce,
    deadline: offer.deadline,
    blockNumber: offer.blockNumber,
    blockHash: offer.blockHash,
    name: offer.name,
    signature,
    marker: "",
  };
  consent.marker = encodeConsent(consent);
  return consent;
}

export async function verifyBrandingReceipt(
  config: AppConfig,
  identity: StoredIdentity,
  offer: BrandingOffer,
  receipt: BrandingReceipt,
  dependencies: Pick<BrandingConsentDependencies, "rpc" | "fetch" | "hashCode"> = {},
): Promise<void> {
  validateOfferBinding(config, identity, offer, offer.minter);
  if (
    receipt.offerId !== offer.offerId || receipt.contract !== offer.contract ||
    receipt.controllerAgentId !== offer.controllerAgentId || receipt.acolyte !== offer.acolyte ||
    receipt.owner !== offer.minter || receipt.referrer !== offer.referrer ||
    receipt.initialDeclaredPrice !== offer.initialDeclaredPrice || receipt.nonce !== offer.nonce ||
    receipt.name !== offer.name || receipt.tokenId !== BigInt(offer.acolyte)
  ) {
    throw new Error("The Branding receipt does not match the signed offer");
  }
  const rpc = dependencies.rpc ?? createJsonRpcClient(
    config.baseRpcEndpoint,
    dependencies.fetch ?? fetch,
  );
  const tag = blockTag(receipt.blockNumber);
  const header = parseBlock(await rpc.request("eth_getBlockByNumber", [tag, false]));
  if (header.number !== receipt.blockNumber || header.hash !== receipt.blockHash) {
    throw new Error("The Branding receipt block is no longer canonical");
  }
  await verifyCanonicalDeployment(rpc, offer.contract, tag, dependencies.hashCode);
  const [brandingResult, nonceResult, traitCountResult] = await Promise.all([
    contractCall(rpc, BRANDING, offer.contract, "brandingOf", [offer.acolyte], tag),
    contractCall(rpc, BRANDING, offer.contract, "nonces", [offer.acolyte], tag),
    contractCall(rpc, BRANDING, offer.contract, "customTraitCount", [receipt.tokenId], tag),
  ]);
  const branding = brandingResult[0] as BrandingView;
  if (
    branding.tokenId !== receipt.tokenId || canonicalAddress(branding.acolyte) !== offer.acolyte ||
    canonicalAddress(branding.owner) !== offer.minter ||
    branding.controllerAgentId !== offer.controllerAgentId ||
    canonicalAddress(branding.referrer) !== offer.referrer ||
    nonceResult[0] !== offer.nonce + 1n
  ) {
    throw new Error("The Branding receipt is not confirmed by the exact minted on-chain state");
  }
  const traitCount = uint256(traitCountResult[0], "Branding trait count");
  if (traitCount > 32n) throw new Error("The Branding receipt returned too many traits");
  let nameTraits = 0;
  let exactNameTraits = 0;
  for (let index = 0n; index < traitCount; index += 1n) {
    const trait = await contractCall(rpc, BRANDING, offer.contract, "customTraitAt", [receipt.tokenId, index], tag);
    if (trait[0] !== ACOLYTE_NAME_TRAIT) continue;
    nameTraits += 1;
    if (trait[1] === offer.name) exactNameTraits += 1;
  }
  if (nameTraits !== 1 || exactNameTraits !== 1) {
    throw new Error("The Branding does not contain exactly one canonical Acolyte Name trait");
  }
  const unchanged = parseBlock(await rpc.request("eth_getBlockByNumber", [tag, false]));
  if (unchanged.hash !== header.hash || unchanged.number !== header.number) {
    throw new Error("The Branding receipt block changed during verification");
  }
}

export function consentMatchesOffer(consent: BrandingConsent, offer: BrandingOffer): boolean {
  return consent.offerId === offer.offerId && consent.contract === offer.contract &&
    consent.minter === offer.minter && consent.controllerAgentId === offer.controllerAgentId &&
    consent.acolyte === offer.acolyte && consent.referrer === offer.referrer &&
    consent.initialDeclaredPrice === offer.initialDeclaredPrice && consent.nonce === offer.nonce &&
    consent.deadline === offer.deadline && consent.blockNumber === offer.blockNumber &&
    consent.blockHash === offer.blockHash && consent.name === offer.name;
}

function parseOffer(marker: string): BrandingOffer {
  const match = /^\[\[cthuwu:branding-offer:v2;offer=([0-9a-f]{32});contract=(0x[0-9a-f]{40});minter=(0x[0-9a-f]{40});agent=([0-9]+);acolyte=(0x[0-9a-f]{40});referrer=(0x[0-9a-f]{40});treasury=([0-9]+);basis=([0-9]+);price=([0-9]+);upkeep=([0-9]+);nonce=([0-9]+);deadline=([0-9]+);block=([0-9]+);blockHash=(0x[0-9a-f]{64});name=(0x(?:[0-9a-f]{2})+)\]\]$/u.exec(marker);
  if (!match) throw new Error("Malformed Branding offer");
  const offer: BrandingOffer = {
    type: "offer", offerId: requestId(match[1]), contract: address(match[2]), minter: address(match[3]),
    controllerAgentId: decimal(match[4], "controller agent"), acolyte: address(match[5]),
    referrer: address(match[6]), treasury: decimal(match[7], "treasury"),
    basisPoints: decimal(match[8], "basis"), initialDeclaredPrice: decimal(match[9], "price"),
    firstWeekUpkeep: decimal(match[10], "upkeep"), nonce: decimal(match[11], "nonce"),
    deadline: decimal(match[12], "deadline"), blockNumber: decimal(match[13], "block"),
    blockHash: hash(match[14]), name: decodeName(match[15]), marker,
  };
  return offer;
}

function parseConsent(marker: string): BrandingConsent {
  const match = /^\[\[cthuwu:branding-consent:v2;offer=([0-9a-f]{32});contract=(0x[0-9a-f]{40});minter=(0x[0-9a-f]{40});agent=([0-9]+);acolyte=(0x[0-9a-f]{40});referrer=(0x[0-9a-f]{40});price=([0-9]+);nonce=([0-9]+);deadline=([0-9]+);block=([0-9]+);blockHash=(0x[0-9a-f]{64});name=(0x(?:[0-9a-f]{2})+);signature=(0x(?:[0-9a-f]{2})+)\]\]$/u.exec(marker);
  if (!match) throw new Error("Malformed Branding consent");
  return {
    type: "consent", offerId: requestId(match[1]), contract: address(match[2]), minter: address(match[3]),
    controllerAgentId: decimal(match[4], "controller agent"), acolyte: address(match[5]),
    referrer: address(match[6]), initialDeclaredPrice: decimal(match[7], "price"),
    nonce: decimal(match[8], "nonce"), deadline: decimal(match[9], "deadline"),
    blockNumber: decimal(match[10], "block"), blockHash: hash(match[11]), name: decodeName(match[12]),
    signature: canonicalSignature(match[13]), marker,
  };
}

function parseReceipt(marker: string): BrandingReceipt {
  const match = /^\[\[cthuwu:branding-receipt:v2;offer=([0-9a-f]{32});contract=(0x[0-9a-f]{40});token=([0-9]+);agent=([0-9]+);acolyte=(0x[0-9a-f]{40});owner=(0x[0-9a-f]{40});referrer=(0x[0-9a-f]{40});price=([0-9]+);nonce=([0-9]+);block=([0-9]+);blockHash=(0x[0-9a-f]{64});name=(0x(?:[0-9a-f]{2})+)\]\]$/u.exec(marker);
  if (!match) throw new Error("Malformed Branding receipt");
  return {
    type: "receipt", offerId: requestId(match[1]), contract: address(match[2]),
    tokenId: decimal(match[3], "token"), controllerAgentId: decimal(match[4], "controller agent"),
    acolyte: address(match[5]), owner: address(match[6]), referrer: address(match[7]),
    initialDeclaredPrice: decimal(match[8], "price"), nonce: decimal(match[9], "nonce"),
    blockNumber: decimal(match[10], "block"), blockHash: hash(match[11]),
    name: decodeName(match[12]), marker,
  };
}

function parseDecline(marker: string): BrandingDecline {
  const match = /^\[\[cthuwu:branding-decline:v2;offer=([0-9a-f]{32})\]\]$/u.exec(marker);
  if (!match) throw new Error("Malformed Branding decline");
  return { type: "decline", offerId: requestId(match[1]), marker };
}

function parseRequest(marker: string): BrandingRequest {
  const match = /^\[\[cthuwu:branding-request:v2;referrer=(0x[0-9a-f]{40});name=(0x(?:[0-9a-f]{2})+)\]\]$/u.exec(marker);
  if (!match) throw new Error("Malformed Branding request");
  return { type: "request", referrer: address(match[1]), name: decodeName(match[2]), marker };
}

function encodeConsent(consent: BrandingConsent): string {
  return `[[cthuwu:branding-consent:v2;offer=${consent.offerId};contract=${consent.contract};minter=${consent.minter};agent=${consent.controllerAgentId};acolyte=${consent.acolyte};referrer=${consent.referrer};price=${consent.initialDeclaredPrice};nonce=${consent.nonce};deadline=${consent.deadline};block=${consent.blockNumber};blockHash=${consent.blockHash};name=${encodeName(consent.name)};signature=${consent.signature}]]`;
}

async function inspectOfferAtBlock(
  rpc: RpcClient,
  offer: BrandingOffer,
  number: bigint,
  expectedHash: string,
  hashCode: ((code: string) => string) | undefined,
): Promise<BrandingReview> {
  const chain = rpcQuantity(await rpc.request("eth_chainId", []), "Base chain ID");
  if (chain !== BigInt(BASE_CHAIN_ID)) throw new Error("The configured RPC is not Base mainnet");
  const tag = blockTag(number);
  const header = parseBlock(await rpc.request("eth_getBlockByNumber", [tag, false]));
  if (header.number !== number || header.hash !== expectedHash) {
    throw new Error("The Branding offer is not pinned to the canonical Base block");
  }
  if (offer.deadline < header.timestamp + MIN_SIGNING_WINDOW_SECONDS) {
    throw new Error("The Branding consent deadline is too close; ask the Tentacle for a fresh offer");
  }
  await verifyCanonicalDeployment(rpc, offer.contract, tag, hashCode);
  const consentValue = mintConsentValue(offer);
  const [brandingResult, nonceResult, upkeepResult, digestResult, treasuryResult,
    registryVersion, agentWallet, authorized, allegiance, protocol] = await Promise.all([
    contractCall(rpc, BRANDING, offer.contract, "brandingOf", [offer.acolyte], tag),
    contractCall(rpc, BRANDING, offer.contract, "nonces", [offer.acolyte], tag),
    contractCall(rpc, BRANDING, offer.contract, "weeklyUpkeepForPrice", [offer.initialDeclaredPrice], tag),
    contractCall(rpc, BRANDING, offer.contract, "consentDigest", [consentValue], tag),
    contractCall(rpc, UWU, UWU_CONTRACT, "balanceOf", [offer.minter], tag),
    contractCall(rpc, REGISTRY, IDENTITY_REGISTRY, "getVersion", [], tag),
    contractCall(rpc, REGISTRY, IDENTITY_REGISTRY, "getAgentWallet", [offer.controllerAgentId], tag),
    contractCall(rpc, REGISTRY, IDENTITY_REGISTRY, "isAuthorizedOrOwner", [offer.minter, offer.controllerAgentId], tag),
    contractCall(rpc, REGISTRY, IDENTITY_REGISTRY, "getMetadata", [offer.controllerAgentId, "cthuwu.allegiance"], tag),
    contractCall(rpc, REGISTRY, IDENTITY_REGISTRY, "getMetadata", [offer.controllerAgentId, "cthuwu.protocol"], tag),
  ]);
  const branding = brandingResult[0] as BrandingView;
  if (
    branding.tokenId !== BigInt(offer.acolyte) || canonicalAddress(branding.acolyte) !== offer.acolyte ||
    canonicalAddress(branding.owner) !== ZERO_ADDRESS || branding.status !== 0n ||
    branding.controllerAgentId !== 0n || canonicalAddress(branding.referrer) !== ZERO_ADDRESS ||
    branding.declaredPrice !== 0n || branding.paidThrough !== 0n ||
    branding.pendingDeclaredPrice !== 0n || branding.pendingPriceActivation !== 0n
  ) throw new Error("The Acolyte Branding is no longer exactly Unminted");
  if (nonceResult[0] !== offer.nonce) throw new Error("The Branding consent nonce changed");
  if (treasuryResult[0] !== offer.treasury) throw new Error("The Tentacle treasury changed after its offer");
  if (upkeepResult[0] !== offer.firstWeekUpkeep) throw new Error("The offered upkeep is not canonical");
  if (
    registryVersion[0] !== "2.0.0" || canonicalAddress(agentWallet[0]) !== offer.minter ||
    authorized[0] !== true || String(allegiance[0]).toLowerCase() !== ALLEGIANCE_HEX ||
    String(protocol[0]).toLowerCase() !== PROTOCOL_V1_HEX
  ) throw new Error("The offered controller is not an eligible canonical Tentacle");
  const domain = {
    name: BRANDING_DOMAIN_NAME,
    version: BRANDING_DOMAIN_VERSION,
    chainId: BASE_CHAIN_ID,
    verifyingContract: getAddress(offer.contract),
  } as const;
  const digest = TypedDataEncoder.hash(domain, MINT_CONSENT_TYPES, consentValue).toLowerCase();
  if (String(digestResult[0]).toLowerCase() !== digest) {
    throw new Error("The Branding contract returned a different consent digest");
  }
  const unchanged = parseBlock(await rpc.request("eth_getBlockByNumber", [tag, false]));
  if (unchanged.hash !== header.hash || unchanged.number !== header.number) {
    throw new Error("The Branding offer block changed during verification");
  }
  return { offer, digest, domain };
}

async function verifyCanonicalDeployment(
  rpc: RpcClient,
  contract: string,
  tag: string,
  hashCode: ((code: string) => string) | undefined,
): Promise<void> {
  const [brandingCode, registryCode, uwuCode] = await Promise.all([
    rpc.request("eth_getCode", [contract, tag]),
    rpc.request("eth_getCode", [IDENTITY_REGISTRY, tag]),
    rpc.request("eth_getCode", [UWU_CONTRACT, tag]),
  ]);
  if (typeof brandingCode !== "string" || !NONEMPTY_CODE.test(brandingCode) ||
      (hashCode ?? keccak256)(brandingCode).toLowerCase() !== BRANDING_RUNTIME_HASH) {
    throw new Error("The canonical Branding runtime is unavailable or changed");
  }
  if (typeof registryCode !== "string" || !NONEMPTY_CODE.test(registryCode) ||
      typeof uwuCode !== "string" || !NONEMPTY_CODE.test(uwuCode)) {
    throw new Error("A canonical Branding dependency is unavailable");
  }
  const [chain, registry, uwu, version, domainName, domainVersion] = await Promise.all([
    contractCall(rpc, BRANDING, contract, "BASE_CHAIN_ID", [], tag),
    contractCall(rpc, BRANDING, contract, "IDENTITY_REGISTRY", [], tag),
    contractCall(rpc, BRANDING, contract, "UWU", [], tag),
    contractCall(rpc, BRANDING, contract, "REGISTRY_VERSION", [], tag),
    contractCall(rpc, BRANDING, contract, "DOMAIN_NAME", [], tag),
    contractCall(rpc, BRANDING, contract, "DOMAIN_VERSION", [], tag),
  ]);
  if (chain[0] !== BigInt(BASE_CHAIN_ID) || canonicalAddress(registry[0]) !== IDENTITY_REGISTRY ||
      canonicalAddress(uwu[0]) !== UWU_CONTRACT || version[0] !== "2.0.0" ||
      domainName[0] !== BRANDING_DOMAIN_NAME || domainVersion[0] !== BRANDING_DOMAIN_VERSION) {
    throw new Error("The Branding deployment does not match the canonical production configuration");
  }
}

async function verifyConsentSignature(
  rpc: RpcClient,
  identity: StoredIdentity,
  review: BrandingReview,
  signature: string,
  tag: string,
): Promise<void> {
  const code = await rpc.request("eth_getCode", [identity.address, tag]);
  if (typeof code !== "string" || !/^0x(?:[0-9a-fA-F]{2})*$/u.test(code)) {
    throw new Error("Base returned invalid Acolyte account code");
  }
  const contractWallet = code !== "0x";
  if (!isLocalIdentity(identity) && (identity.signerType === "SCW") !== contractWallet) {
    throw new Error("The connected wallet type changed; reconnect it before signing");
  }
  if (!contractWallet) {
    const recovered = verifyTypedData(
      review.domain,
      MINT_CONSENT_TYPES,
      mintConsentValue(review.offer),
      signature,
    ).toLowerCase();
    if (recovered !== identity.address) throw new Error("The wallet signature does not match this Acolyte");
    return;
  }
  const response = await ethCall(
    rpc,
    identity.address,
    ERC1271.encodeFunctionData("isValidSignature", [review.digest, signature]),
    tag,
  );
  let result: unknown;
  try {
    result = ERC1271.decodeFunctionResult("isValidSignature", response)[0];
  } catch {
    throw new Error("The smart account rejected the Branding signature");
  }
  if (String(result).toLowerCase() !== "0x1626ba7e") {
    throw new Error("The smart account rejected the Branding signature");
  }
}

function validateOfferBinding(
  config: AppConfig,
  identity: StoredIdentity,
  offer: BrandingOffer,
  expectedMinter: string | undefined,
): void {
  if (offer.contract !== CANONICAL_BRANDING_CONTRACT || config.brandingContract !== offer.contract) {
    throw new Error("The offer does not target the canonical Branding contract");
  }
  if (offer.acolyte !== identity.address || offer.acolyte === ZERO_ADDRESS) {
    throw new Error("The Branding offer targets another Acolyte");
  }
  if (expectedMinter && offer.minter !== canonicalAddress(expectedMinter)) {
    throw new Error("The Branding offer does not match the assigned Tentacle");
  }
  if (offer.minter === ZERO_ADDRESS || offer.referrer === ZERO_ADDRESS ||
      offer.controllerAgentId === 0n || offer.treasury === 0n ||
      offer.initialDeclaredPrice === 0n || offer.firstWeekUpkeep === 0n ||
      offer.blockNumber === 0n || offer.deadline === 0n) {
    throw new Error("The Branding offer contains a forbidden zero value");
  }
  if (offer.basisPoints < 500n || offer.basisPoints > 2_000n) {
    throw new Error("The Branding price basis must be between 5% and 20%");
  }
  if (offer.treasury > MAX_UINT256 / offer.basisPoints ||
      offer.treasury * offer.basisPoints / 10_000n !== offer.initialDeclaredPrice) {
    throw new Error("The Branding offer price does not match its treasury basis");
  }
  if (offer.initialDeclaredPrice > (MAX_UINT256 - 9_999n) / 10n ||
      (offer.initialDeclaredPrice * 10n + 9_999n) / 10_000n !== offer.firstWeekUpkeep) {
    throw new Error("The Branding offer upkeep is not the upward-rounded 0.1%");
  }
  if (offer.name !== acolyteName(identity.address)) {
    throw new Error("The Branding offer does not contain this Acolyte's deterministic name");
  }
  if (config.referrer && offer.referrer !== canonicalAddress(config.referrer)) {
    throw new Error("The Branding offer does not preserve the pinned referrer");
  }
}

const MINT_CONSENT_TYPES: Record<string, TypedDataField[]> = {
  MintConsent: [
    { name: "acolyte", type: "address" },
    { name: "minter", type: "address" },
    { name: "controllerAgentId", type: "uint256" },
    { name: "referrer", type: "address" },
    { name: "initialDeclaredPrice", type: "uint256" },
    { name: "nonce", type: "uint256" },
    { name: "deadline", type: "uint256" },
  ],
};

function mintConsentValue(offer: BrandingOffer): Record<string, string | bigint> {
  return {
    acolyte: getAddress(offer.acolyte), minter: getAddress(offer.minter),
    controllerAgentId: offer.controllerAgentId, referrer: getAddress(offer.referrer),
    initialDeclaredPrice: offer.initialDeclaredPrice, nonce: offer.nonce, deadline: offer.deadline,
  };
}

async function defaultExternalSigner(identity: StoredIdentity, review: BrandingReview): Promise<string> {
  if (isLocalIdentity(identity)) throw new Error("Local Branding consent must use the local signer");
  const { signExternalTypedData } = await import("./wallet-connector");
  return signExternalTypedData(identity, review.domain, MINT_CONSENT_TYPES, mintConsentValue(review.offer));
}

interface BrandingView {
  tokenId: bigint;
  acolyte: string;
  owner: string;
  controllerAgentId: bigint;
  referrer: string;
  declaredPrice: bigint;
  paidThrough: bigint;
  pendingDeclaredPrice: bigint;
  pendingPriceActivation: bigint;
  status: bigint;
}

async function contractCall(
  rpc: RpcClient,
  abi: Interface,
  to: string,
  functionName: string,
  args: readonly unknown[],
  tag: string,
): Promise<ReturnType<Interface["decodeFunctionResult"]>> {
  const data = abi.encodeFunctionData(functionName, args);
  const result = await ethCall(rpc, to, data, tag);
  try {
    return abi.decodeFunctionResult(functionName, result);
  } catch {
    throw new Error(`Canonical ${functionName} returned malformed data`);
  }
}

async function ethCall(rpc: RpcClient, to: string, data: string, tag: string): Promise<string> {
  const result = await rpc.request("eth_call", [{ to, data }, tag]);
  if (typeof result !== "string" || !/^0x(?:[0-9a-fA-F]{2})*$/u.test(result)) {
    throw new Error("Base RPC returned an invalid eth_call result");
  }
  return result;
}

function parseBlock(value: unknown): { number: bigint; hash: string; timestamp: bigint } {
  if (!isRecord(value)) throw new Error("Base block header is invalid");
  return {
    number: rpcQuantity(value.number, "Base block number"),
    hash: hash(value.hash),
    timestamp: rpcQuantity(value.timestamp, "Base block timestamp"),
  };
}

function rpcQuantity(value: unknown, label: string): bigint {
  if (typeof value !== "string" || !QUANTITY.test(value)) throw new Error(`${label} is invalid`);
  return BigInt(value);
}

function uint256(value: unknown, label: string): bigint {
  if (typeof value !== "bigint" || value < 0n || value > MAX_UINT256) {
    throw new Error(`${label} is invalid`);
  }
  return value;
}

function decimal(value: string | undefined, label: string): bigint {
  if (!value || !DECIMAL.test(value)) throw new Error(`${label} is not canonical decimal`);
  const parsed = BigInt(value);
  if (parsed > MAX_UINT256) throw new Error(`${label} exceeds uint256`);
  return parsed;
}

function address(value: string | undefined): string {
  if (!value || !ADDRESS.test(value)) throw new Error("Branding address is not canonical lowercase");
  return canonicalAddress(value);
}

function canonicalAddress(value: unknown): string {
  if (typeof value !== "string") throw new Error("Branding address is invalid");
  const normalized = getAddress(value).toLowerCase();
  if (!ADDRESS.test(normalized)) throw new Error("Branding address is invalid");
  return normalized;
}

function requestId(value: string | undefined): string {
  if (!value || !OFFER_ID.test(value)) throw new Error("Branding offer ID is invalid");
  return value;
}

function hash(value: unknown): string {
  if (typeof value !== "string" || !HASH.test(value)) throw new Error("Branding block hash is invalid");
  return value;
}

function canonicalSignature(value: string): string {
  if (!HEX_BYTES.test(value) || value.length > 16_386) throw new Error("Wallet returned an invalid signature");
  return value;
}

function encodeName(name: string): string {
  return hexlify(toUtf8Bytes(name)).toLowerCase();
}

function decodeName(value: string | undefined): string {
  if (!value || !HEX_BYTES.test(value) || value.length > 514) throw new Error("Branding name is invalid");
  let decoded: string;
  try {
    decoded = new TextDecoder("utf-8", { fatal: true }).decode(getBytes(value));
  } catch {
    throw new Error("Branding name is not valid UTF-8");
  }
  if (!decoded || encodeName(decoded) !== value) throw new Error("Branding name is not canonical UTF-8 hex");
  return decoded;
}

function blockTag(value: bigint): string {
  return `0x${value.toString(16)}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
