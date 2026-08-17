import { createHash, randomUUID } from "node:crypto";
import { chmod, link, lstat, open, readFile, rename, unlink } from "node:fs/promises";
import path from "node:path";
import {
  ContractFunctionRevertedError,
  createPublicClient,
  createWalletClient,
  decodeEventLog,
  encodeFunctionData,
  getAddress,
  hashTypedData,
  hexToBytes,
  hexToString,
  http,
  isAddress,
  isHex,
  keccak256,
  pad,
  parseAbi,
  stringToHex,
  toHex,
  verifyTypedData,
  type Address,
  type Hex,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { base } from "viem/chains";
import { publicActionsL2 } from "viem/op-stack";
import type { LoadedIdentity } from "./identity.js";
import { resolveOperatorIdentity } from "./operator-identity.js";

export const ERC8004_CHAIN_ID = 8453;
export const ERC8004_IDENTITY_REGISTRY = getAddress(
  "0x8004A169FB4a3325136EB29fA0ceB6D2e539a432",
);
export const ERC8004_REPUTATION_REGISTRY = getAddress(
  "0x8004BAa17C55a88189AE136b182e5fdA19dE9b63",
);
export const ERC8004_IDENTITY_IMPLEMENTATION = getAddress(
  "0x7274e874CA62410a93Bd8bf61c69d8045E399c02",
);
export const ERC8004_REPUTATION_IMPLEMENTATION = getAddress(
  "0x16e0FA7f7C56B9a767E34B192B51f921BE31dA34",
);
export const ERC8004_REVISION =
  "erc-8004-contracts@68fc6765761a10fb26f0692df21c8a6f9d12b1be";
export const ERC8004_VERSION = "2.0.0";
export const ERC8004_START_BLOCK = 41_663_783n;

export const BRANDING_CONTRACT = getAddress(
  "0xD8c36F13D79a505C7FBDc5F6467eA3cd75E896Da",
);
export const BRANDING_RUNTIME_CODE_HASH =
  "0x3a22b742a570dc2d030edf2bd82dceda1e068a297e2c36883f97b7b66ed4ef2d" as Hex;
export const CANONICAL_UWU = getAddress(
  "0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07",
);
export const ACOLYTE_NAME_TRAIT = "Acolyte Name";
export const ACOLYTE_NAME_SCHEME = "acolyte-v1";

export const ALLEGIANCE_KEY = "cthuwu.allegiance";
export const ALLEGIANCE_VALUE = "uwu-tentacle-v1";
export const PROTOCOL_KEY = "cthuwu.protocol";
export const PROTOCOL_VALUE = "1";
export const TENTACLE_ID_KEY = "cthuwu.tentacle-id";

const ZERO_ADDRESS = "0x0000000000000000000000000000000000000000";
const EIP1967_IMPLEMENTATION_SLOT =
  "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";
const PROXY_CODE_HASH =
  "0xd0e45b1d89fa9b6cc7e97c1f155d64180e5c232aaccf9900ef9d4fd738c02b41";
const IDENTITY_IMPLEMENTATION_CODE_HASH =
  "0xa5f9624ea85e45b3f4b8558581f03bfb3e6cefab278d7bf0500ec9bd065dc16f";
const REPUTATION_IMPLEMENTATION_CODE_HASH =
  "0x38602de97f1bd86f0a4729f7f3c0a78b1d27892e6eb581272cce5504a68fd00b";
const MAX_FRAME_BYTES = 32 * 1024;
const MAX_URI_BYTES = 8 * 1024;
const MAX_METADATA_VALUE_BYTES = 256;
const MAX_TENTACLE_ID_BYTES = 128;
const MAX_CANDIDATES = 64;
const MAX_DISCOVERY_IDS = 256;
const MAX_DISCOVERY_LOGS = 4_096;
const MAX_DISCOVERY_RAW_LOGS_PER_CHUNK = 4_096;
const MAX_OPERATOR_OWNERS = 64;
// Base's public mainnet RPC rejects eth_getLogs ranges wider than 10,000
// blocks. Keep this inclusive range pinned to that real production limit.
const LOG_BLOCK_SPAN = 10_000n;
const RECENT_DISCOVERY_BLOCKS = 20_000n;
const TEST_RECENT_DISCOVERY_BLOCKS_ENV =
  "CTHUWU_TEST_ERC8004_RECENT_DISCOVERY_BLOCKS";
const DISCOVERY_CONCURRENCY = 5;
const RPC_RETRY_ATTEMPTS = 3;
const RPC_RETRY_BASE_DELAY_MS = 1_100;
const L1_FEE_THROTTLE_MS = 1_100;
// Real canonical-Base estimation for the bounded registration-v1 data URI can exceed
// 800k gas; 2m remains a strict per-call ceiling and covers the verified profile size.
const DEFAULT_MAX_GAS_PER_TRANSACTION = 2_000_000n;
const DEFAULT_MAX_FEE_PER_GAS_WEI = 10_000_000_000n;
const DEFAULT_SAFETY_BPS = 12_500n;
const DEFAULT_RESERVE_WEI = 50_000_000_000_000n;
const MAX_SIGNER_JOURNAL_BYTES = 4 * 1024;
const MAX_DISCOVERY_JOURNAL_BYTES = 64 * 1024;
const MAX_SIGNATURE_BYTES = 8 * 1024;
const BRANDING_DOMAIN_NAME = "Cthuwu Acolyte Branding";
const BRANDING_DOMAIN_VERSION = "1";
const BRANDING_REGISTRY_VERSION = "2.0.0";
const UWU_DECIMALS = 18;
const ERC1271_MAGIC_VALUE = "0x1626ba7e";

const brandingAbi = parseAbi([
  "event BrandingMinted(uint256 indexed tokenId, address indexed acolyte, address indexed owner, uint256 controllerAgentId, address referrer, uint256 declaredPrice, uint256 paidThrough, uint256 firstUpkeep)",
  "event CustomTraitUpdated(uint256 indexed tokenId, string traitType, string value)",
  "function BASE_CHAIN_ID() view returns (uint256)",
  "function IDENTITY_REGISTRY() view returns (address)",
  "function UWU() view returns (address)",
  "function REGISTRY_VERSION() view returns (string)",
  "function DOMAIN_NAME() view returns (string)",
  "function DOMAIN_VERSION() view returns (string)",
  "function nonces(address acolyte) view returns (uint256)",
  "function weeklyUpkeepForPrice(uint256 price) pure returns (uint256)",
  "function consentDigest((address acolyte,address minter,uint256 controllerAgentId,address referrer,uint256 initialDeclaredPrice,uint256 nonce,uint256 deadline) consent) view returns (bytes32)",
  "function mintBranding((address acolyte,address minter,uint256 controllerAgentId,address referrer,uint256 initialDeclaredPrice,uint256 nonce,uint256 deadline) consent, bytes signature) returns (uint256 tokenId)",
  "function brandingOf(address acolyte) view returns ((uint256 tokenId,address acolyte,address owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 pendingDeclaredPrice,uint256 pendingPriceActivation,uint8 status) result)",
  "function customTraitCount(uint256 tokenId) view returns (uint256)",
  "function customTraitAt(uint256 tokenId, uint256 index) view returns (string traitType, string value)",
  "function setCustomTrait(uint256 tokenId, string traitType, string value)",
]);

const erc20Abi = parseAbi([
  "function decimals() view returns (uint8)",
  "function balanceOf(address account) view returns (uint256)",
  "function allowance(address owner, address spender) view returns (uint256)",
  "function approve(address spender, uint256 amount) returns (bool)",
]);

const erc1271Abi = parseAbi([
  "function isValidSignature(bytes32 hash, bytes signature) view returns (bytes4 magicValue)",
]);

const identityAbi = parseAbi([
  "error ERC721NonexistentToken(uint256 tokenId)",
  "event Registered(uint256 indexed agentId, string agentURI, address indexed owner)",
  "event Transfer(address indexed from, address indexed to, uint256 indexed tokenId)",
  "event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId)",
  "event ApprovalForAll(address indexed owner, address indexed operator, bool approved)",
  "event MetadataSet(uint256 indexed agentId, string indexed indexedMetadataKey, string metadataKey, bytes metadataValue)",
  "function getVersion() pure returns (string)",
  "function ownerOf(uint256 tokenId) view returns (address)",
  "function tokenURI(uint256 tokenId) view returns (string)",
  "function getAgentWallet(uint256 agentId) view returns (address)",
  "function getMetadata(uint256 agentId, string metadataKey) view returns (bytes)",
  "function isAuthorizedOrOwner(address spender, uint256 agentId) view returns (bool)",
  "function isApprovedForAll(address owner, address operator) view returns (bool)",
  "function register() returns (uint256 agentId)",
  "function setAgentURI(uint256 agentId, string newURI)",
  "function setMetadata(uint256 agentId, string metadataKey, bytes metadataValue)",
  "function setAgentWallet(uint256 agentId, address newWallet, uint256 deadline, bytes signature)",
]);

const reputationAbi = parseAbi([
  "function getVersion() pure returns (string)",
  "function getIdentityRegistry() view returns (address)",
]);

type ReadOperation =
  | { type: "inspect_registry" }
  | { type: "resolve_inbox"; wallet: string }
  | {
      type: "transaction_nonce";
      wallet: string;
      observedBlockNumber?: string;
      observedBlockHash?: Hex;
    }
  | { type: "inspect_agent"; agentId: string; wallet: string }
  | {
      type: "discover";
      wallet: string;
      registrationNonce?: string;
      registrationActionId?: string;
      scope: "recent" | "exhaustive";
      tentacleId?: string;
      xmtpInboxId?: string;
      checkpoint?: DiscoveryCheckpointReference;
    }
  | { type: "receipt"; transactionHash: string }
  | {
      type: "funding_estimate";
      wallet: string;
      agentId?: string;
      agentURI: string;
      includeAgentUri: boolean;
      includeWalletVerification: boolean;
      metadata: Array<{ key: string; value: string }>;
    };

export type DiscoveryCheckpointReference = {
  version: 1;
  fingerprint: string;
};

export type MintAuthorization = {
  version: 1;
  chainId: 8453;
  registry: Address;
  wallet: Address;
  tentacleId: string;
  xmtpInboxId: string;
  fromBlock: string;
  throughBlock: string;
  throughBlockHash: Hex;
  candidateSetHash: Hex;
  fingerprint: string;
};

type WriteIntent =
  | { type: "register" }
  | { type: "set_agent_uri"; agentId: string; agentURI: string }
  | { type: "set_metadata"; agentId: string; key: string; value: string }
  | { type: "set_agent_wallet"; agentId: string };

type WriteOperation =
  | { type: "register"; nonce: string; mintAuthorization?: MintAuthorization }
  | (Exclude<WriteIntent, { type: "register" }> & { nonce: string });

export type BrandingInspectOperation = {
  type: "branding_inspect";
  acolyte: Address;
  controllerAgentId: string;
  referrer: Address;
  treasuryBalance: string;
  priceBasisPoints: number;
  initialDeclaredPrice: string;
  acolyteName: string;
};

export type CompleteBrandingOperation = {
  type: "complete_branding";
  acolyte: Address;
  minter: Address;
  controllerAgentId: string;
  referrer: Address;
  treasuryBalance: string;
  priceBasisPoints: number;
  initialDeclaredPrice: string;
  nonce: string;
  deadline: string;
  offerBlockNumber: string;
  offerBlockHash: Hex;
  signature: Hex;
  acolyteName: string;
};

type BrandingOperation = BrandingInspectOperation | CompleteBrandingOperation;

export type PreparedErc8004Transaction = {
  chainId: 8453;
  to: Address;
  value: 0n;
  data: Hex;
};

export type Erc8004Request = {
  version: 1;
  actionId: string;
  operation: ReadOperation | WriteOperation | BrandingOperation;
};

export type Erc8004Response =
  | { version: 1; actionId: string; ok: true; result: unknown }
  | {
      version: 1;
      actionId: string;
      ok: false;
      recoverable: boolean;
      code: string;
      message: string;
    };

export function assertProductionIdentity(
  identity: LoadedIdentity,
  expectedWallet?: string,
): Address {
  if (identity.environment !== "production") {
    throw new PermanentSignerError(
      "identity_environment",
      "ERC-8004 signing and production-inbox publication require the persistent XMTP production identity",
    );
  }
  const derived = getAddress(privateKeyToAccount(identity.walletKey).address);
  if (derived !== getAddress(identity.walletAddress)) {
    throw new PermanentSignerError(
      "identity_mismatch",
      "persistent identity wallet derivation changed",
    );
  }
  if (expectedWallet !== undefined && derived !== getAddress(expectedWallet)) {
    throw new PermanentSignerError(
      "identity_mismatch",
      "requested Tentacle wallet is not the persistent XMTP production signer",
    );
  }
  return derived;
}

type PublicClient = ReturnType<typeof createRegistryPublicClient>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function exactKeys(record: Record<string, unknown>, keys: readonly string[]): void {
  const actual = Object.keys(record).sort();
  const expected = [...keys].sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new PermanentSignerError("invalid_request", "request contains missing or unknown fields");
  }
}

function boundedString(value: unknown, name: string, maximum: number): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new PermanentSignerError("invalid_request", `${name} must be a nonempty string`);
  }
  if (Buffer.byteLength(value, "utf8") > maximum || /[\u0000-\u001f\u007f]/u.test(value)) {
    throw new PermanentSignerError("invalid_request", `${name} is oversized or contains control characters`);
  }
  return value;
}

function decimalId(value: unknown): string {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]{0,77})$/u.test(value)) {
    throw new PermanentSignerError("invalid_request", "agentId must be a canonical uint256 decimal string");
  }
  if (BigInt(value) >= 1n << 256n) {
    throw new PermanentSignerError("invalid_request", "agentId exceeds uint256");
  }
  return value;
}

function walletAddress(value: unknown): Address {
  if (typeof value !== "string" || !isAddress(value, { strict: true })) {
    throw new PermanentSignerError("invalid_request", "wallet must be a full EVM address");
  }
  const address = getAddress(value);
  if (address === ZERO_ADDRESS) {
    throw new PermanentSignerError("invalid_request", "wallet must not be the zero address");
  }
  return address;
}

function uint256Decimal(
  value: unknown,
  name: string,
  options: { positive?: boolean } = {},
): string {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]{0,77})$/u.test(value)) {
    throw new PermanentSignerError(
      "invalid_request",
      `${name} must be a canonical uint256 decimal string`,
    );
  }
  const parsed = BigInt(value);
  if (parsed >= 1n << 256n || (options.positive === true && parsed === 0n)) {
    throw new PermanentSignerError(
      "invalid_request",
      `${name} is outside its required uint256 range`,
    );
  }
  return value;
}

function brandingPriceBasisPoints(value: unknown): number {
  if (
    typeof value !== "number" ||
    !Number.isInteger(value) ||
    value < 500 ||
    value > 2_000
  ) {
    throw new PermanentSignerError(
      "invalid_request",
      "priceBasisPoints must be an integer between 500 and 2000",
    );
  }
  return value;
}

function validateBrandingQuote(
  treasuryBalance: string,
  priceBasisPoints: number,
  initialDeclaredPrice: string,
): void {
  const product = BigInt(treasuryBalance) * BigInt(priceBasisPoints);
  if (product >= 1n << 256n) {
    throw new PermanentSignerError(
      "branding_quote",
      "treasury-derived Branding price calculation exceeds uint256",
    );
  }
  const expected = product / 10_000n;
  if (expected === 0n || expected.toString() !== initialDeclaredPrice) {
    throw new PermanentSignerError(
      "branding_quote",
      "initialDeclaredPrice does not match the exact treasury balance and bounded basis-point quote",
    );
  }
}

function boundedHex(value: unknown, name: string, maximumBytes: number): Hex {
  if (
    typeof value !== "string" ||
    !/^0x(?:[0-9a-fA-F]{2})+$/u.test(value) ||
    (value.length - 2) / 2 > maximumBytes
  ) {
    throw new PermanentSignerError(
      "invalid_request",
      `${name} must be nonempty bounded hexadecimal bytes`,
    );
  }
  return value.toLowerCase() as Hex;
}

const ACOLYTE_NAME_FIRST = [
  "Ainsworth", "Ashcombe", "Bellingham", "Blackwood", "Cavendish", "Cholmondeley",
  "Davenport", "Devereux", "Eversleigh", "Fairfax", "Featherstone", "Fitzwilliam",
  "Fortescue", "Gainsborough", "Harrington", "Hawthorne", "Kensington", "Langford",
  "Marlborough", "Montague", "Pemberton", "Ravenscroft", "Sinclair", "Somerset",
  "Stanhope", "Thackeray", "Wainwright", "Weatherby", "Wellington", "Westcott",
  "Whitcombe", "Winchester", "Abberley", "Adderley", "Alvingham", "Bancroft",
  "Barrington", "Beauchamp", "Beresford", "Brabazon", "Broughton", "Buckhurst",
  "Cadogan", "Chatterton", "Chetwynd", "Coleridge", "Digby", "Edgeworth", "Frobisher",
  "Granville", "Hardwick", "Hesketh", "Lascelles", "Mandeville", "Mortimer", "Neville",
  "Paget", "Rawdon", "Rockingham", "Sherborne", "Trelawney", "Waldegrave", "Wentworth",
  "Wyndham",
] as const;

const ACOLYTE_NAME_SECOND = [
  "Arbuthnot", "Bramwell", "Carrington", "Chadwick", "Clavering", "Cumberland",
  "Darlington", "Ellsworth", "Farnsworth", "Fetherstonhaugh", "Godolphin", "Grantham",
  "Hargreaves", "Kingsley", "Loxley", "Marchbanks", "Molesworth", "Northcott", "Ormsby",
  "Ponsonby", "Radcliffe", "Sackville", "Smythe", "Tavistock", "Templeton", "Uxbridge",
  "Vane", "Walsingham", "Wetherell", "Whittington", "Wickham", "Worthing", "Acton",
  "Blandford", "Boswell", "Bridgeman", "Bulwer", "Calthorpe", "Chichester", "Coningsby",
  "Delamere", "Denham", "Dorrington", "Eddington", "Fane", "Fitzalan", "Grafton",
  "Grosvenor", "Harcourt", "Ingleby", "Jermyn", "Kettering", "Lowther", "Marwood",
  "Painswick", "Quenby", "Rivington", "SaintJohn", "Strathmore", "Tichborne", "Underhill",
  "Vernon", "Wrottesley", "Yelverton",
] as const;

const ACOLYTE_ESTATE_PREFIX = [
  "Alder", "Amber", "Apple", "Ash", "Barrow", "Beech", "Bel", "Birch", "Black", "Blen",
  "Blythe", "Bracken", "Bram", "Briar", "Bright", "Broad", "Buck", "Cedar", "Charn",
  "Clear", "Cold", "Crow", "Deep", "Dun", "East", "Elder", "Elm", "Ever", "Fair",
  "Fern", "Fleet", "Fox", "Glen", "Gold", "Grand", "Green", "Grey", "Hart", "Hazel",
  "High", "Holly", "Honey", "Ivy", "Kings", "Lang", "Little", "Long", "Low", "Maple",
  "Marsh", "Mere", "Mill", "Nether", "North", "Oak", "Pen", "Pine", "Raven", "Red",
  "Rose", "Silver", "South", "Stan", "Wych",
] as const;

const ACOLYTE_ESTATE_SUFFIX = [
  "abbey", "bank", "borough", "bourne", "bridge", "brook", "bury", "castle", "chester",
  "cliff", "combe", "court", "croft", "dale", "den", "field", "ford", "gate", "grove",
  "hall", "ham", "haven", "heath", "hill", "holm", "hurst", "ington", "land", "leigh",
  "manor", "marsh", "meadow", "mere", "mill", "minster", "moor", "mount", "park", "pool",
  "port", "ridge", "rose", "stead", "stoke", "stone", "thorp", "ton", "vale", "view",
  "ville", "wall", "water", "way", "well", "wick", "wood", "worth", "yard", "end", "fen",
  "green", "lodge", "priory", "quay",
] as const;

export function deriveAcolyteName(address: string): string {
  const canonical = getAddress(address);
  const digest = hexToBytes(keccak256(canonical));
  const index = (offset: number, length: number): number =>
    ((digest[offset]! << 8) | digest[offset + 1]!) % length;
  return `${ACOLYTE_NAME_FIRST[index(0, ACOLYTE_NAME_FIRST.length)]}-${ACOLYTE_NAME_SECOND[index(2, ACOLYTE_NAME_SECOND.length)]} of ${ACOLYTE_ESTATE_PREFIX[index(4, ACOLYTE_ESTATE_PREFIX.length)]}${ACOLYTE_ESTATE_SUFFIX[index(6, ACOLYTE_ESTATE_SUFFIX.length)]}`;
}

function validatedAcolyteName(value: unknown, acolyte: Address): string {
  const name = boundedString(value, "acolyteName", 256);
  if (name !== deriveAcolyteName(acolyte)) {
    throw new PermanentSignerError(
      "acolyte_name",
      `acolyteName must match the deterministic ${ACOLYTE_NAME_SCHEME} value`,
    );
  }
  return name;
}

function transactionHash(value: unknown): Hex {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]{64}$/u.test(value)) {
    throw new PermanentSignerError("invalid_request", "transactionHash must be 32 bytes");
  }
  return value as Hex;
}

function transactionNonce(value: unknown): string {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]{0,19})$/u.test(value)) {
    throw new PermanentSignerError(
      "invalid_request",
      "nonce must be a canonical nonnegative decimal string",
    );
  }
  const nonce = BigInt(value);
  if (nonce > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new PermanentSignerError(
      "invalid_request",
      "nonce exceeds the signer transport's exact integer range",
    );
  }
  return value;
}

function fingerprint(value: unknown, label = "fingerprint"): string {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/u.test(value)) {
    throw new PermanentSignerError("invalid_request", `${label} must be 32 lowercase hex bytes`);
  }
  return value;
}

function xmtpInboxId(value: unknown): string {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/u.test(value)) {
    throw new PermanentSignerError("invalid_request", "xmtpInboxId must be a canonical production inbox ID");
  }
  return value;
}

function discoveryCheckpointReference(value: unknown): DiscoveryCheckpointReference {
  if (!isRecord(value)) {
    throw new PermanentSignerError("invalid_request", "discovery checkpoint must be an object");
  }
  exactKeys(value, ["version", "fingerprint"]);
  if (value.version !== 1) {
    throw new PermanentSignerError("invalid_request", "unsupported discovery checkpoint version");
  }
  return { version: 1, fingerprint: fingerprint(value.fingerprint, "checkpoint fingerprint") };
}

function mintAuthorization(value: unknown): MintAuthorization {
  if (!isRecord(value)) {
    throw new PermanentSignerError("invalid_request", "mintAuthorization must be an object");
  }
  exactKeys(value, [
    "version", "chainId", "registry", "wallet", "tentacleId", "xmtpInboxId",
    "fromBlock", "throughBlock", "throughBlockHash", "candidateSetHash", "fingerprint",
  ]);
  if (value.version !== 1 || value.chainId !== ERC8004_CHAIN_ID) {
    throw new PermanentSignerError("invalid_request", "mintAuthorization targets an unsupported chain or version");
  }
  const registry = walletAddress(value.registry);
  if (registry !== ERC8004_IDENTITY_REGISTRY) {
    throw new PermanentSignerError("invalid_request", "mintAuthorization targets another registry");
  }
  const tentacleId = boundedString(value.tentacleId, "tentacleId", MAX_TENTACLE_ID_BYTES);
  validateMetadata(TENTACLE_ID_KEY, tentacleId);
  return {
    version: 1,
    chainId: ERC8004_CHAIN_ID,
    registry,
    wallet: walletAddress(value.wallet),
    tentacleId,
    xmtpInboxId: xmtpInboxId(value.xmtpInboxId),
    fromBlock: decimalId(value.fromBlock),
    throughBlock: decimalId(value.throughBlock),
    throughBlockHash: transactionHash(value.throughBlockHash),
    candidateSetHash: transactionHash(value.candidateSetHash),
    fingerprint: fingerprint(value.fingerprint, "mint authorization fingerprint"),
  };
}

export function assertTransactionNonceWindow(
  requested: string,
  pending: number,
  latest: number,
  exactAllocationExists = false,
): number {
  const nonce = Number(transactionNonce(requested));
  if (!Number.isSafeInteger(pending) || !Number.isSafeInteger(latest) || pending < latest) {
    throw new PermanentSignerError(
      "nonce_state",
      "provider returned an invalid signer nonce window",
    );
  }
  if (nonce < latest) {
    throw new RecoverableSignerError(
      "nonce_consumed",
      "the persisted registry transaction nonce has already been confirmed",
    );
  }
  if (nonce > pending) {
    throw new PermanentSignerError(
      "nonce_gap",
      "registry transaction nonce is above the production signer's pending nonce",
    );
  }
  if (nonce < pending && !exactAllocationExists) {
    throw new PermanentSignerError(
      "nonce_unallocated",
      "refusing to replace a pending wallet transaction without an exact durable signer action allocation",
    );
  }
  return nonce;
}

function metadataPair(value: unknown): { key: string; value: string } {
  if (!isRecord(value)) {
    throw new PermanentSignerError("invalid_request", "metadata entry must be an object");
  }
  exactKeys(value, ["key", "value"]);
  const key = boundedString(value.key, "metadata key", 64);
  if (typeof value.value !== "string") {
    throw new PermanentSignerError("invalid_request", "metadata value must be a UTF-8 string");
  }
  validateMetadata(key, value.value);
  return { key, value: value.value };
}

function validateMetadata(key: string, value: string): void {
  if (![ALLEGIANCE_KEY, PROTOCOL_KEY, TENTACLE_ID_KEY].includes(key)) {
    throw new PermanentSignerError("metadata_key", "metadata key is outside the cthuwu allowlist");
  }
  const bytes = Buffer.byteLength(value, "utf8");
  if (bytes > MAX_METADATA_VALUE_BYTES || /[\u0000-\u001f\u007f]/u.test(value)) {
    throw new PermanentSignerError("metadata_value", "metadata value is oversized or contains control characters");
  }
  if (key === ALLEGIANCE_KEY && value !== "" && value !== ALLEGIANCE_VALUE) {
    throw new PermanentSignerError("metadata_value", "allegiance may only be exact opt-in bytes or empty bytes");
  }
  if (key === PROTOCOL_KEY && value !== "" && value !== PROTOCOL_VALUE) {
    throw new PermanentSignerError("metadata_value", "protocol may only be exact version 1 bytes or empty bytes");
  }
  if (key === TENTACLE_ID_KEY && bytes > MAX_TENTACLE_ID_BYTES) {
    throw new PermanentSignerError("metadata_value", "Tentacle ID exceeds its public bound");
  }
}

export function parseErc8004Request(value: unknown): Erc8004Request {
  if (!isRecord(value)) {
    throw new PermanentSignerError("invalid_request", "request must be an object");
  }
  exactKeys(value, ["version", "actionId", "operation"]);
  if (value.version !== 1) {
    throw new PermanentSignerError("invalid_request", "unsupported request version");
  }
  const actionId = boundedString(value.actionId, "actionId", 128);
  if (!/^[a-zA-Z0-9:_-]+$/u.test(actionId)) {
    throw new PermanentSignerError("invalid_request", "actionId contains unsupported characters");
  }
  if (!isRecord(value.operation) || typeof value.operation.type !== "string") {
    throw new PermanentSignerError("invalid_request", "operation must be a tagged object");
  }
  const operation = value.operation;
  switch (operation.type) {
    case "inspect_registry":
      exactKeys(operation, ["type"]);
      return { version: 1, actionId, operation: { type: "inspect_registry" } };
    case "resolve_inbox":
      exactKeys(operation, ["type", "wallet"]);
      return {
        version: 1,
        actionId,
        operation: { type: "resolve_inbox", wallet: walletAddress(operation.wallet) },
      };
    case "transaction_nonce":
      if (
        (operation.observedBlockNumber === undefined) !==
        (operation.observedBlockHash === undefined)
      ) {
        throw new PermanentSignerError(
          "invalid_request",
          "observed discovery block number and hash must be provided together",
        );
      }
      exactKeys(
        operation,
        operation.observedBlockNumber === undefined
          ? ["type", "wallet"]
          : ["type", "wallet", "observedBlockNumber", "observedBlockHash"],
      );
      return {
        version: 1,
        actionId,
        operation: {
          type: "transaction_nonce",
          wallet: walletAddress(operation.wallet),
          ...(operation.observedBlockNumber === undefined
            ? {}
            : {
                observedBlockNumber: decimalId(operation.observedBlockNumber),
                observedBlockHash: transactionHash(operation.observedBlockHash),
              }),
        },
      };
    case "inspect_agent":
      exactKeys(operation, ["type", "agentId", "wallet"]);
      return {
        version: 1,
        actionId,
        operation: { type: "inspect_agent", agentId: decimalId(operation.agentId), wallet: walletAddress(operation.wallet) },
      };
    case "discover":
      if ((operation.tentacleId === undefined) !== (operation.xmtpInboxId === undefined)) {
        throw new PermanentSignerError(
          "invalid_request",
          "Tentacle ID and production XMTP inbox must be provided together for identity classification",
        );
      }
      if (operation.registrationActionId !== undefined && operation.registrationNonce === undefined) {
        throw new PermanentSignerError(
          "invalid_request",
          "registrationActionId requires the exact persisted registration nonce",
        );
      }
      exactKeys(
        operation,
        [
          "type", "wallet", "scope",
          ...(operation.registrationNonce === undefined ? [] : ["registrationNonce"]),
          ...(operation.registrationActionId === undefined ? [] : ["registrationActionId"]),
          ...(operation.tentacleId === undefined ? [] : ["tentacleId", "xmtpInboxId"]),
          ...(operation.checkpoint === undefined ? [] : ["checkpoint"]),
        ],
      );
      if (operation.scope !== "recent" && operation.scope !== "exhaustive") {
        throw new PermanentSignerError("invalid_request", "discovery scope must be recent or exhaustive");
      }
      if (operation.checkpoint !== undefined && operation.scope !== "exhaustive") {
        throw new PermanentSignerError("invalid_request", "discovery checkpoints are exhaustive-only");
      }
      const discoveredTentacleId = operation.tentacleId === undefined
        ? undefined
        : boundedString(operation.tentacleId, "tentacleId", MAX_TENTACLE_ID_BYTES);
      if (discoveredTentacleId !== undefined) {
        validateMetadata(TENTACLE_ID_KEY, discoveredTentacleId);
      }
      return {
        version: 1,
        actionId,
        operation: {
          type: "discover",
          wallet: walletAddress(operation.wallet),
          scope: operation.scope,
          ...(operation.tentacleId === undefined
            ? {}
            : {
                tentacleId: discoveredTentacleId as string,
                xmtpInboxId: xmtpInboxId(operation.xmtpInboxId),
              }),
          ...(operation.checkpoint === undefined
            ? {}
            : { checkpoint: discoveryCheckpointReference(operation.checkpoint) }),
          ...(operation.registrationNonce === undefined
            ? {}
            : { registrationNonce: transactionNonce(operation.registrationNonce) }),
          ...(operation.registrationActionId === undefined
            ? {}
            : {
                registrationActionId: boundedString(
                  operation.registrationActionId,
                  "registrationActionId",
                  128,
                ),
              }),
        },
      };
    case "receipt":
      exactKeys(operation, ["type", "transactionHash"]);
      return { version: 1, actionId, operation: { type: "receipt", transactionHash: transactionHash(operation.transactionHash) } };
    case "funding_estimate": {
      const keys = operation.agentId === undefined
        ? ["type", "wallet", "agentURI", "includeAgentUri", "includeWalletVerification", "metadata"]
        : ["type", "wallet", "agentId", "agentURI", "includeAgentUri", "includeWalletVerification", "metadata"];
      exactKeys(operation, keys);
      if (
        typeof operation.includeAgentUri !== "boolean" ||
        typeof operation.includeWalletVerification !== "boolean" ||
        !Array.isArray(operation.metadata) ||
        operation.metadata.length > 3
      ) {
        throw new PermanentSignerError("invalid_request", "funding estimate fields are invalid");
      }
      const parsed: ReadOperation = {
        type: "funding_estimate",
        wallet: walletAddress(operation.wallet),
        agentURI: uri(operation.agentURI),
        includeAgentUri: operation.includeAgentUri,
        includeWalletVerification: operation.includeWalletVerification,
        metadata: operation.metadata.map(metadataPair),
        ...(operation.agentId === undefined ? {} : { agentId: decimalId(operation.agentId) }),
      };
      return { version: 1, actionId, operation: parsed };
    }
    case "branding_inspect": {
      exactKeys(operation, [
        "type",
        "acolyte",
        "controllerAgentId",
        "referrer",
        "treasuryBalance",
        "priceBasisPoints",
        "initialDeclaredPrice",
        "acolyteName",
      ]);
      const acolyte = walletAddress(operation.acolyte);
      const treasuryBalance = uint256Decimal(operation.treasuryBalance, "treasuryBalance");
      const priceBasisPoints = brandingPriceBasisPoints(operation.priceBasisPoints);
      const initialDeclaredPrice = uint256Decimal(
        operation.initialDeclaredPrice,
        "initialDeclaredPrice",
        { positive: true },
      );
      validateBrandingQuote(treasuryBalance, priceBasisPoints, initialDeclaredPrice);
      return {
        version: 1,
        actionId,
        operation: {
          type: "branding_inspect",
          acolyte,
          controllerAgentId: decimalId(operation.controllerAgentId),
          referrer: walletAddress(operation.referrer),
          treasuryBalance,
          priceBasisPoints,
          initialDeclaredPrice,
          acolyteName: validatedAcolyteName(operation.acolyteName, acolyte),
        },
      };
    }
    case "complete_branding": {
      exactKeys(operation, [
        "type",
        "acolyte",
        "minter",
        "controllerAgentId",
        "referrer",
        "treasuryBalance",
        "priceBasisPoints",
        "initialDeclaredPrice",
        "nonce",
        "deadline",
        "offerBlockNumber",
        "offerBlockHash",
        "signature",
        "acolyteName",
      ]);
      const acolyte = walletAddress(operation.acolyte);
      const treasuryBalance = uint256Decimal(operation.treasuryBalance, "treasuryBalance");
      const priceBasisPoints = brandingPriceBasisPoints(operation.priceBasisPoints);
      const initialDeclaredPrice = uint256Decimal(
        operation.initialDeclaredPrice,
        "initialDeclaredPrice",
        { positive: true },
      );
      validateBrandingQuote(treasuryBalance, priceBasisPoints, initialDeclaredPrice);
      return {
        version: 1,
        actionId,
        operation: {
          type: "complete_branding",
          acolyte,
          minter: walletAddress(operation.minter),
          controllerAgentId: decimalId(operation.controllerAgentId),
          referrer: walletAddress(operation.referrer),
          treasuryBalance,
          priceBasisPoints,
          initialDeclaredPrice,
          nonce: uint256Decimal(operation.nonce, "nonce"),
          deadline: uint256Decimal(operation.deadline, "deadline", { positive: true }),
          offerBlockNumber: uint256Decimal(
            operation.offerBlockNumber,
            "offerBlockNumber",
          ),
          offerBlockHash: transactionHash(operation.offerBlockHash),
          signature: boundedHex(operation.signature, "signature", MAX_SIGNATURE_BYTES),
          acolyteName: validatedAcolyteName(operation.acolyteName, acolyte),
        },
      };
    }
    case "register":
      exactKeys(
        operation,
        operation.mintAuthorization === undefined
          ? ["type", "nonce"]
          : ["type", "nonce", "mintAuthorization"],
      );
      return {
        version: 1,
        actionId,
        operation: {
          type: "register",
          nonce: transactionNonce(operation.nonce),
          ...(operation.mintAuthorization === undefined
            ? {}
            : { mintAuthorization: mintAuthorization(operation.mintAuthorization) }),
        },
      };
    case "set_agent_uri":
      exactKeys(operation, ["type", "agentId", "agentURI", "nonce"]);
      return { version: 1, actionId, operation: { type: "set_agent_uri", agentId: decimalId(operation.agentId), agentURI: uri(operation.agentURI), nonce: transactionNonce(operation.nonce) } };
    case "set_metadata":
      exactKeys(operation, ["type", "agentId", "key", "value", "nonce"]);
      return { version: 1, actionId, operation: { type: "set_metadata", agentId: decimalId(operation.agentId), ...metadataPair({ key: operation.key, value: operation.value }), nonce: transactionNonce(operation.nonce) } };
    case "set_agent_wallet":
      exactKeys(operation, ["type", "agentId", "nonce"]);
      return { version: 1, actionId, operation: { type: "set_agent_wallet", agentId: decimalId(operation.agentId), nonce: transactionNonce(operation.nonce) } };
    default:
      throw new PermanentSignerError("operation", "unsupported ERC-8004 operation");
  }
}

function uri(value: unknown): string {
  return boundedString(value, "agentURI", MAX_URI_BYTES);
}

class PermanentSignerError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

class RecoverableSignerError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

type Sleep = (milliseconds: number) => Promise<void>;

const sleep: Sleep = async (milliseconds) => {
  await new Promise<void>((resolve) => setTimeout(resolve, milliseconds));
};

function rpcErrorText(error: unknown): string {
  const fragments: string[] = [];
  const visited = new Set<unknown>();
  let current: unknown = error;
  for (let depth = 0; depth < 5 && current !== undefined; depth += 1) {
    if (visited.has(current)) break;
    visited.add(current);
    if (current instanceof Error) {
      fragments.push(current.name, current.message);
      current = current.cause;
      continue;
    }
    if (!isRecord(current)) break;
    for (const key of ["name", "message", "shortMessage", "details", "code", "status"]) {
      const value = current[key];
      if (typeof value === "string" || typeof value === "number") {
        fragments.push(String(value));
      }
    }
    current = current.cause;
  }
  return fragments.join(" ").toLowerCase();
}

export function isTransientRpcError(error: unknown): boolean {
  const description = rpcErrorText(error);
  return [
    "over rate limit",
    "rate limit",
    "too many requests",
    "429",
    "timeout",
    "timed out",
    "network",
    "fetch failed",
    "connection",
    "socket",
    "econn",
    "service unavailable",
    "temporarily unavailable",
    "backend",
    "502",
    "503",
    "504",
  ].some((marker) => description.includes(marker));
}

export async function withBoundedRpcRetry<T>(
  operation: () => Promise<T>,
  options: {
    attempts?: number;
    baseDelayMs?: number;
    sleep?: Sleep;
  } = {},
): Promise<T> {
  const attempts = options.attempts ?? RPC_RETRY_ATTEMPTS;
  const baseDelayMs = options.baseDelayMs ?? RPC_RETRY_BASE_DELAY_MS;
  const wait = options.sleep ?? sleep;
  if (!Number.isSafeInteger(attempts) || attempts < 1 || attempts > 5) {
    throw new PermanentSignerError("configuration", "RPC retry attempts are outside the bounded range");
  }
  if (!Number.isSafeInteger(baseDelayMs) || baseDelayMs < 0 || baseDelayMs > 10_000) {
    throw new PermanentSignerError("configuration", "RPC retry delay is outside the bounded range");
  }
  let lastError: unknown;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      return await operation();
    } catch (error) {
      lastError = error;
      if (!isTransientRpcError(error) || attempt + 1 === attempts) throw error;
      await wait(baseDelayMs * 2 ** attempt);
    }
  }
  throw lastError;
}

export async function sumThrottledL1Fees<T>(
  requests: readonly T[],
  estimate: (request: T) => Promise<bigint>,
  options: {
    throttleMs?: number;
    sleep?: Sleep;
  } = {},
): Promise<bigint> {
  const throttleMs = options.throttleMs ?? L1_FEE_THROTTLE_MS;
  const wait = options.sleep ?? sleep;
  if (!Number.isSafeInteger(throttleMs) || throttleMs < 0 || throttleMs > 10_000) {
    throw new PermanentSignerError("configuration", "L1 fee throttle is outside the bounded range");
  }
  let total = 0n;
  for (const request of requests) {
    // The canonical GasPriceOracle is queried once per exact calldata payload. Public Base
    // providers apply request budgets independently of viem's transport retry behavior.
    await wait(throttleMs);
    const fee = await withBoundedRpcRetry(() => estimate(request), { sleep: wait });
    if (fee < 0n) {
      throw new RecoverableSignerError("l1_fee_estimate", "provider returned a negative Base L1 data fee");
    }
    total += fee;
  }
  return total;
}

export async function sumL1FeesWithConservativeFallback<T>(
  requests: readonly T[],
  estimate: (request: T) => Promise<bigint>,
  conservativeFeePerTransaction: bigint,
  options: {
    throttleMs?: number;
    sleep?: Sleep;
  } = {},
): Promise<{ fee: bigint; exact: boolean }> {
  if (conservativeFeePerTransaction < 0n) {
    throw new PermanentSignerError(
      "configuration",
      "conservative L1 fee allowance cannot be negative",
    );
  }
  try {
    return {
      fee: await sumThrottledL1Fees(requests, estimate, options),
      exact: true,
    };
  } catch (error) {
    if (error instanceof PermanentSignerError) throw error;
    // Some otherwise usable Base providers do not expose the GasPriceOracle
    // getL1Fee call used by viem. Funding reconciliation may conservatively
    // reserve the full configured L2 execution ceiling once more for every
    // pending transaction. The write path still performs its own estimateGas,
    // fee-cap, signing, and broadcast checks, so this fallback cannot authorize
    // a transaction that the exact execution path rejects.
    return {
      fee: conservativeFeePerTransaction * BigInt(requests.length),
      exact: false,
    };
  }
}

type SignerNonceAllocation = {
  version: 1;
  chainId: 8453;
  registry: Address;
  wallet: Address;
  nonce: string;
  actionId: string;
  fingerprint: string;
};

function signerActionFingerprint(
  actionId: string,
  operation: WriteOperation,
  wallet: Address,
): string {
  return createHash("sha256")
    .update(JSON.stringify({
      version: 1,
      actionId,
      chainId: ERC8004_CHAIN_ID,
      registry: ERC8004_IDENTITY_REGISTRY,
      wallet,
      operation,
    }))
    .digest("hex");
}

async function installSignerAllocation(
  target: string,
  contents: string,
): Promise<boolean> {
  const temporary = `${target}.${process.pid}.${randomUUID()}.tmp`;
  const handle = await open(temporary, "wx", 0o600);
  try {
    await handle.writeFile(contents, { encoding: "utf8" });
    await handle.sync();
  } finally {
    await handle.close();
  }
  try {
    await link(temporary, target);
    await chmod(target, 0o600);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EEXIST") return false;
    throw error;
  } finally {
    await unlink(temporary).catch(() => undefined);
  }
}

function parseSignerAllocation(raw: string): SignerNonceAllocation {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    throw new PermanentSignerError(
      "signer_journal",
      "ERC-8004 signer nonce allocation is not valid JSON",
    );
  }
  if (!isRecord(value)) {
    throw new PermanentSignerError(
      "signer_journal",
      "ERC-8004 signer nonce allocation has an invalid shape",
    );
  }
  exactKeys(value, [
    "version",
    "chainId",
    "registry",
    "wallet",
    "nonce",
    "actionId",
    "fingerprint",
  ]);
  if (
    value.version !== 1 ||
    value.chainId !== ERC8004_CHAIN_ID ||
    value.registry !== ERC8004_IDENTITY_REGISTRY ||
    typeof value.wallet !== "string" ||
    typeof value.nonce !== "string" ||
    typeof value.actionId !== "string" ||
    typeof value.fingerprint !== "string" ||
    !/^[0-9a-f]{64}$/u.test(value.fingerprint)
  ) {
    throw new PermanentSignerError(
      "signer_journal",
      "ERC-8004 signer nonce allocation is incomplete or targets another deployment",
    );
  }
  return {
    version: 1,
    chainId: ERC8004_CHAIN_ID,
    registry: ERC8004_IDENTITY_REGISTRY,
    wallet: walletAddress(value.wallet),
    nonce: transactionNonce(value.nonce),
    actionId: boundedString(value.actionId, "journal actionId", 128),
    fingerprint: value.fingerprint,
  };
}

/**
 * Atomically allocates one production signer nonce to one exact typed action before broadcast.
 * Returns true only when the exact allocation already existed, which is the sole condition that
 * permits replacing a nonce below the provider's current pending nonce.
 */
export async function authorizeSignerNonce(
  identity: LoadedIdentity,
  actionId: string,
  operation: WriteOperation,
  requestedNonce: number,
  pendingNonce: number,
): Promise<boolean> {
  const wallet = assertProductionIdentity(identity);
  if (!Number.isSafeInteger(requestedNonce) || !Number.isSafeInteger(pendingNonce)) {
    throw new PermanentSignerError("nonce_state", "signer nonce is outside the exact integer range");
  }
  const stateDirectory = path.dirname(identity.identityPath);
  const directoryStat = await lstat(stateDirectory);
  if (!directoryStat.isDirectory() || directoryStat.isSymbolicLink()) {
    throw new PermanentSignerError(
      "signer_journal",
      "persistent identity state directory is not a private regular directory",
    );
  }
  await chmod(stateDirectory, 0o700);
  const nonce = requestedNonce.toString();
  const fingerprint = signerActionFingerprint(actionId, operation, wallet);
  const allocation: SignerNonceAllocation = {
    version: 1,
    chainId: ERC8004_CHAIN_ID,
    registry: ERC8004_IDENTITY_REGISTRY,
    wallet,
    nonce,
    actionId,
    fingerprint,
  };
  const encoded = `${JSON.stringify(allocation)}\n`;
  if (Buffer.byteLength(encoded, "utf8") > MAX_SIGNER_JOURNAL_BYTES) {
    throw new PermanentSignerError("signer_journal", "signer nonce allocation exceeds its bound");
  }
  const allocationPath = path.join(
    stateDirectory,
    `erc8004-signer-nonce-v1-${nonce}.json`,
  );
  let existed = true;
  if (requestedNonce === pendingNonce) {
    existed = !(await installSignerAllocation(allocationPath, encoded));
  }
  let stat;
  try {
    stat = await lstat(allocationPath);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      throw new PermanentSignerError(
        "nonce_unallocated",
        "refusing to replace a pending wallet transaction without an exact durable signer action allocation",
      );
    }
    throw error;
  }
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > MAX_SIGNER_JOURNAL_BYTES) {
    throw new PermanentSignerError(
      "signer_journal",
      "signer nonce allocation is not a bounded regular file",
    );
  }
  await chmod(allocationPath, 0o600);
  const persisted = parseSignerAllocation(await readFile(allocationPath, "utf8"));
  if (
    persisted.wallet !== allocation.wallet ||
    persisted.nonce !== allocation.nonce ||
    persisted.actionId !== allocation.actionId ||
    persisted.fingerprint !== allocation.fingerprint
  ) {
    throw new PermanentSignerError(
      "nonce_allocation_conflict",
      "signer nonce is durably allocated to another ERC-8004 action",
    );
  }
  return existed;
}

type BrandingSignerAllocation = {
  version: 2;
  chainId: 8453;
  wallet: Address;
  nonce: string;
  actionId: string;
  phase: BrandingTransactionPhase;
  destination: Address;
  calldataHash: Hex;
  fingerprint: string;
};

export type BrandingTransactionPhase = "approve" | "mint" | "name_trait";

function brandingSignerFingerprint(
  actionId: string,
  phase: BrandingTransactionPhase,
  wallet: Address,
  nonce: string,
  destination: Address,
  calldataHash: Hex,
): string {
  return createHash("sha256")
    .update(JSON.stringify({
      version: 2,
      actionId,
      chainId: ERC8004_CHAIN_ID,
      wallet,
      nonce,
      phase,
      destination,
      calldataHash,
    }))
    .digest("hex");
}

function parseBrandingSignerAllocation(raw: string): BrandingSignerAllocation {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    throw new PermanentSignerError(
      "signer_journal",
      "shared signer nonce allocation is not valid JSON",
    );
  }
  if (!isRecord(value)) {
    throw new PermanentSignerError(
      "signer_journal",
      "shared signer nonce allocation has an invalid shape",
    );
  }
  exactKeys(value, [
    "version",
    "chainId",
    "wallet",
    "nonce",
    "actionId",
    "phase",
    "destination",
    "calldataHash",
    "fingerprint",
  ]);
  if (
    value.version !== 2 ||
    value.chainId !== ERC8004_CHAIN_ID ||
    (value.phase !== "approve" && value.phase !== "mint" && value.phase !== "name_trait") ||
    typeof value.fingerprint !== "string" ||
    !/^[0-9a-f]{64}$/u.test(value.fingerprint)
  ) {
    throw new PermanentSignerError(
      "signer_journal",
      "shared signer nonce allocation is incomplete or has another protocol version",
    );
  }
  return {
    version: 2,
    chainId: ERC8004_CHAIN_ID,
    wallet: walletAddress(value.wallet),
    nonce: transactionNonce(value.nonce),
    actionId: boundedString(value.actionId, "journal actionId", 128),
    phase: value.phase,
    destination: walletAddress(value.destination),
    calldataHash: transactionHash(value.calldataHash),
    fingerprint: value.fingerprint,
  };
}

/**
 * Reserves the one currently executable EOA nonce for one closed Branding phase. The same
 * `erc8004-signer-nonce-v1-*` namespace is intentionally shared with registry writes, so a
 * pending registration or unrelated wallet transaction cannot be replaced by Branding.
 */
export async function authorizeBrandingSignerNonce(
  identity: LoadedIdentity,
  actionId: string,
  phase: BrandingTransactionPhase,
  destination: Address,
  data: Hex,
  pendingNonce: number,
  latestNonce: number,
): Promise<{ nonce: number; existed: boolean }> {
  const wallet = assertProductionIdentity(identity);
  if (
    !Number.isSafeInteger(pendingNonce) ||
    !Number.isSafeInteger(latestNonce) ||
    latestNonce < 0 ||
    pendingNonce < latestNonce ||
    pendingNonce > latestNonce + 1
  ) {
    throw new RecoverableSignerError(
      "signer_busy",
      "production signer has an unmatched pending nonce window",
    );
  }
  const stateDirectory = path.dirname(identity.identityPath);
  const directoryStat = await lstat(stateDirectory);
  if (!directoryStat.isDirectory() || directoryStat.isSymbolicLink()) {
    throw new PermanentSignerError(
      "signer_journal",
      "persistent identity state directory is not a private regular directory",
    );
  }
  await chmod(stateDirectory, 0o700);
  const nonce = latestNonce.toString();
  const calldataHash = keccak256(data);
  const allocation: BrandingSignerAllocation = {
    version: 2,
    chainId: ERC8004_CHAIN_ID,
    wallet,
    nonce,
    actionId,
    phase,
    destination,
    calldataHash,
    fingerprint: brandingSignerFingerprint(
      actionId,
      phase,
      wallet,
      nonce,
      destination,
      calldataHash,
    ),
  };
  const encoded = `${JSON.stringify(allocation)}\n`;
  if (Buffer.byteLength(encoded, "utf8") > MAX_SIGNER_JOURNAL_BYTES) {
    throw new PermanentSignerError("signer_journal", "shared signer nonce allocation exceeds its bound");
  }
  const allocationPath = path.join(
    stateDirectory,
    `erc8004-signer-nonce-v1-${nonce}.json`,
  );
  let existed = true;
  if (pendingNonce === latestNonce) {
    existed = !(await installSignerAllocation(allocationPath, encoded));
  }
  let stat;
  try {
    stat = await lstat(allocationPath);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      throw new RecoverableSignerError(
        "signer_busy",
        "production signer has an unmatched pending transaction",
      );
    }
    throw error;
  }
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > MAX_SIGNER_JOURNAL_BYTES) {
    throw new PermanentSignerError(
      "signer_journal",
      "shared signer nonce allocation is not a bounded regular file",
    );
  }
  await chmod(allocationPath, 0o600);
  let persisted: BrandingSignerAllocation;
  try {
    persisted = parseBrandingSignerAllocation(await readFile(allocationPath, "utf8"));
  } catch (error) {
    // A v1 registry allocation at this exact nonce is valid shared-signer state, but it never
    // authorizes a Branding replacement.
    if (pendingNonce > latestNonce) {
      throw new RecoverableSignerError(
        "signer_busy",
        "production signer nonce is reserved by another exact action",
      );
    }
    throw error;
  }
  if (
    persisted.wallet !== allocation.wallet ||
    persisted.nonce !== allocation.nonce ||
    persisted.actionId !== allocation.actionId ||
    persisted.phase !== allocation.phase ||
    persisted.destination !== allocation.destination ||
    persisted.calldataHash !== allocation.calldataHash ||
    persisted.fingerprint !== allocation.fingerprint
  ) {
    throw new RecoverableSignerError(
      "signer_busy",
      "production signer nonce is reserved by another exact action",
    );
  }
  return { nonce: latestNonce, existed };
}

function createRegistryPublicClient(rpcEndpoint: string) {
  let endpoint: URL;
  try {
    endpoint = new URL(rpcEndpoint);
  } catch (error) {
    throw new PermanentSignerError("rpc_endpoint", "RPC endpoint is not a valid URL");
  }
  if (endpoint.protocol !== "https:" && !(endpoint.protocol === "http:" && ["127.0.0.1", "::1", "localhost"].includes(endpoint.hostname))) {
    throw new PermanentSignerError("rpc_endpoint", "RPC endpoint must use HTTPS");
  }
  if (endpoint.username !== "" || endpoint.password !== "") {
    throw new PermanentSignerError("rpc_endpoint", "RPC endpoint must not contain URL credentials");
  }
  return createPublicClient({
    chain: base,
    transport: http(endpoint.toString(), {
      timeout: 20_000,
      retryCount: 1,
      batch: true,
    }),
  }).extend(publicActionsL2());
}

function implementationFromSlot(slot: Hex | undefined): Address {
  if (slot === undefined || !/^0x[0-9a-fA-F]{64}$/u.test(slot)) {
    throw new PermanentSignerError("registry_proxy", "registry has no valid EIP-1967 implementation slot");
  }
  return getAddress(`0x${slot.slice(-40)}`);
}

async function verifyCode(
  publicClient: PublicClient,
  address: Address,
  expectedHash: Hex,
  label: string,
  blockNumber: bigint,
): Promise<number> {
  const code = await publicClient.getCode({ address, blockNumber });
  if (code === undefined || code === "0x") {
    throw new PermanentSignerError("registry_code", `${label} has no deployed bytecode`);
  }
  if (keccak256(code) !== expectedHash) {
    throw new PermanentSignerError("registry_code", `${label} bytecode does not match the pinned deployment`);
  }
  return (code.length - 2) / 2;
}

export async function verifyCanonicalDeployment(
  publicClient: PublicClient,
  pinnedObservation?: { number: bigint; hash: Hex },
): Promise<Record<string, unknown>> {
  const chainId = await publicClient.getChainId();
  if (chainId !== ERC8004_CHAIN_ID) {
    throw new PermanentSignerError("wrong_chain", `RPC reported chain ${chainId}; canonical Base is ${ERC8004_CHAIN_ID}`);
  }
  const observedBlock = pinnedObservation === undefined
    ? await publicClient.getBlock()
    : await publicClient.getBlock({ blockNumber: pinnedObservation.number });
  const observedBlockNumber = observedBlock.number;
  const observedBlockHash = observedBlock.hash;
  if (
    pinnedObservation !== undefined &&
    (observedBlockNumber !== pinnedObservation.number || observedBlockHash !== pinnedObservation.hash)
  ) {
    throw new RecoverableSignerError(
      "registry_reorg",
      "the pinned registry observation block is no longer canonical",
    );
  }
  const [identityProxyCodeBytes, reputationProxyCodeBytes] = await Promise.all([
    verifyCode(publicClient, ERC8004_IDENTITY_REGISTRY, PROXY_CODE_HASH, "Identity Registry proxy", observedBlockNumber),
    verifyCode(publicClient, ERC8004_REPUTATION_REGISTRY, PROXY_CODE_HASH, "Reputation Registry proxy", observedBlockNumber),
  ]);
  const [identitySlot, reputationSlot] = await Promise.all([
    publicClient.getStorageAt({ address: ERC8004_IDENTITY_REGISTRY, slot: EIP1967_IMPLEMENTATION_SLOT, blockNumber: observedBlockNumber }),
    publicClient.getStorageAt({ address: ERC8004_REPUTATION_REGISTRY, slot: EIP1967_IMPLEMENTATION_SLOT, blockNumber: observedBlockNumber }),
  ]);
  const identityImplementation = implementationFromSlot(identitySlot);
  const reputationImplementation = implementationFromSlot(reputationSlot);
  if (identityImplementation !== ERC8004_IDENTITY_IMPLEMENTATION || reputationImplementation !== ERC8004_REPUTATION_IMPLEMENTATION) {
    throw new PermanentSignerError("registry_proxy", "registry proxy implementation does not match the pinned deployment");
  }
  const [identityImplementationCodeBytes, reputationImplementationCodeBytes] = await Promise.all([
    verifyCode(publicClient, identityImplementation, IDENTITY_IMPLEMENTATION_CODE_HASH, "Identity Registry implementation", observedBlockNumber),
    verifyCode(publicClient, reputationImplementation, REPUTATION_IMPLEMENTATION_CODE_HASH, "Reputation Registry implementation", observedBlockNumber),
  ]);
  const [identityVersion, reputationVersion, linkedIdentity] = await Promise.all([
    publicClient.readContract({ address: ERC8004_IDENTITY_REGISTRY, abi: identityAbi, functionName: "getVersion", blockNumber: observedBlockNumber }),
    publicClient.readContract({ address: ERC8004_REPUTATION_REGISTRY, abi: reputationAbi, functionName: "getVersion", blockNumber: observedBlockNumber }),
    publicClient.readContract({ address: ERC8004_REPUTATION_REGISTRY, abi: reputationAbi, functionName: "getIdentityRegistry", blockNumber: observedBlockNumber }),
  ]);
  if (identityVersion !== ERC8004_VERSION || reputationVersion !== ERC8004_VERSION || getAddress(linkedIdentity) !== ERC8004_IDENTITY_REGISTRY) {
    throw new PermanentSignerError("registry_interface", "registry version or cross-registry binding is incompatible");
  }
  const canonicalBlock = await publicClient.getBlock({ blockNumber: observedBlockNumber });
  if (canonicalBlock.hash !== observedBlockHash) {
    throw new RecoverableSignerError(
      "registry_reorg",
      "the registry deployment observation block changed while it was being verified",
    );
  }
  return {
    chainId,
    identityRegistry: ERC8004_IDENTITY_REGISTRY,
    reputationRegistry: ERC8004_REPUTATION_REGISTRY,
    identityImplementation,
    reputationImplementation,
    identityVersion,
    reputationVersion,
    reputationIdentityRegistry: getAddress(linkedIdentity),
    identityProxyCodeBytes,
    reputationProxyCodeBytes,
    identityImplementationCodeBytes,
    reputationImplementationCodeBytes,
    interfaceRevision: "registration-v1",
    interfaceComplete: true,
    pinnedRevision: ERC8004_REVISION,
    blockNumber: observedBlockNumber.toString(),
    blockHash: observedBlockHash,
  };
}

function decodeMetadata(value: Hex): { hex: Hex; utf8: string | null } {
  try {
    return { hex: value, utf8: hexToString(value) };
  } catch {
    return { hex: value, utf8: null };
  }
}

function isCanonicalOwnerOfNotFound(error: unknown, agentId: bigint): boolean {
  const visited = new Set<unknown>();
  let current: unknown = error;
  for (let depth = 0; depth < 8 && current !== undefined; depth += 1) {
    if (visited.has(current)) return false;
    visited.add(current);
    if (current instanceof ContractFunctionRevertedError) {
      const decoded = current.data;
      return (
        decoded?.errorName === "ERC721NonexistentToken" &&
        Array.isArray(decoded.args) &&
        decoded.args.length === 1 &&
        decoded.args[0] === agentId
      );
    }
    if (current instanceof Error) {
      current = current.cause;
      continue;
    }
    if (!isRecord(current)) return false;
    current = current.cause;
  }
  return false;
}

export async function inspectAgent(publicClient: PublicClient, agentId: string, wallet: Address): Promise<Record<string, unknown>> {
  const observedBlock = await publicClient.getBlock();
  const blockNumber = observedBlock.number;
  const result = await inspectAgentAtBlock(publicClient, agentId, wallet, blockNumber, observedBlock.hash);
  const canonicalBlock = await publicClient.getBlock({ blockNumber });
  if (canonicalBlock.hash !== observedBlock.hash) {
    throw new RecoverableSignerError(
      "agent_reorg",
      "the agent observation block changed while it was being read",
    );
  }
  return result;
}

async function inspectAgentAtBlock(
  publicClient: PublicClient,
  agentId: string,
  wallet: Address,
  blockNumber: bigint,
  blockHash: Hex,
): Promise<Record<string, unknown>> {
  const id = BigInt(agentId);
  let owner: Address;
  try {
    owner = await publicClient.readContract({
      address: ERC8004_IDENTITY_REGISTRY,
      abi: identityAbi,
      functionName: "ownerOf",
      args: [id],
      blockNumber,
    });
  } catch (error) {
    if (!isCanonicalOwnerOfNotFound(error, id)) throw error;
    return {
      agentId,
      agentExists: false,
      authority: "canonical-base-ownerOf",
      observedBlock: blockNumber.toString(),
      observedBlockHash: blockHash,
    };
  }
  const [agentURI, agentWallet, allegiance, protocol, tentacleId, authorized] = await Promise.all([
    publicClient.readContract({ address: ERC8004_IDENTITY_REGISTRY, abi: identityAbi, functionName: "tokenURI", args: [id], blockNumber }),
    publicClient.readContract({ address: ERC8004_IDENTITY_REGISTRY, abi: identityAbi, functionName: "getAgentWallet", args: [id], blockNumber }),
    publicClient.readContract({ address: ERC8004_IDENTITY_REGISTRY, abi: identityAbi, functionName: "getMetadata", args: [id, ALLEGIANCE_KEY], blockNumber }),
    publicClient.readContract({ address: ERC8004_IDENTITY_REGISTRY, abi: identityAbi, functionName: "getMetadata", args: [id, PROTOCOL_KEY], blockNumber }),
    publicClient.readContract({ address: ERC8004_IDENTITY_REGISTRY, abi: identityAbi, functionName: "getMetadata", args: [id, TENTACLE_ID_KEY], blockNumber }),
    publicClient.readContract({ address: ERC8004_IDENTITY_REGISTRY, abi: identityAbi, functionName: "isAuthorizedOrOwner", args: [wallet, id], blockNumber }),
  ]);
  if (Buffer.byteLength(agentURI, "utf8") > MAX_URI_BYTES) {
    throw new PermanentSignerError("hostile_profile", "on-chain agentURI exceeds the supported bound");
  }
  const allegianceValue = decodeMetadata(allegiance);
  const protocolValue = decodeMetadata(protocol);
  const tentacleIdValue = decodeMetadata(tentacleId);
  return {
    agentId,
    owner: getAddress(owner),
    agentURI,
    agentWallet: getAddress(agentWallet),
    authorized,
    allegiance: allegianceValue,
    protocol: protocolValue,
    tentacleId: tentacleIdValue,
    declaresTentacleAllegiance: allegiance === stringToHex(ALLEGIANCE_VALUE),
    protocolCompatible: protocol === stringToHex(PROTOCOL_VALUE),
    walletVerified: getAddress(agentWallet) === wallet && getAddress(agentWallet) !== ZERO_ADDRESS,
    observedBlock: blockNumber.toString(),
    observedBlockHash: blockHash,
  };
}

type DiscoveryLog = {
  eventName: "Registered" | "Transfer" | "Approval" | "ApprovalForAll" | "MetadataSet";
  transactionHash?: Hex;
  args: {
    agentId?: bigint;
    tokenId?: bigint;
    owner?: Address;
    to?: Address;
    operator?: Address;
    approved?: Address | boolean;
    metadataKey?: string;
    metadataValue?: Hex;
  };
};

type WalletRegistration = {
  agentId: string;
  transactionHash: Hex;
};

type DiscoveryClassificationContext = {
  tentacleId: string;
  xmtpInboxId: string;
};

type DiscoveryJournal = {
  version: 1;
  chainId: 8453;
  registry: Address;
  wallet: Address;
  tentacleId: string;
  xmtpInboxId: string;
  fromBlock: string;
  throughBlock: string;
  throughBlockHash: Hex;
  associatedAgentIds: string[];
  walletRegistrations: WalletRegistration[];
  operatorOwners: Address[];
  checkpointFingerprint: string;
  mintAuthorization?: MintAuthorization;
};

type DiscoveryInternalState = {
  associatedAgentIds: string[];
  walletRegistrations: WalletRegistration[];
  operatorOwners: Address[];
  classification?: DiscoveryClassificationContext;
};

const DISCOVERY_INTERNAL = Symbol("cthuwu.discovery-internal");
type DiscoveryResultWithInternal = Record<string, unknown> & {
  [DISCOVERY_INTERNAL]: DiscoveryInternalState;
};

const DISCOVERY_EVENT_TOPICS = {
  Registered: keccak256(stringToHex("Registered(uint256,string,address)")),
  Transfer: keccak256(stringToHex("Transfer(address,address,uint256)")),
  Approval: keccak256(stringToHex("Approval(address,address,uint256)")),
  ApprovalForAll: keccak256(stringToHex("ApprovalForAll(address,address,bool)")),
  MetadataSet: keccak256(stringToHex("MetadataSet(uint256,string,string,bytes)")),
} as const;
const AGENT_WALLET_METADATA_TOPIC = keccak256(stringToHex("agentWallet"));

function decodedDiscoveryLog(value: unknown): DiscoveryLog {
  if (!isRecord(value) || typeof value.data !== "string" || !isHex(value.data)) {
    throw new RecoverableSignerError("rpc_response", "provider returned a malformed ERC-8004 log");
  }
  if (
    !Array.isArray(value.topics) ||
    value.topics.length < 2 ||
    value.topics.length > 4 ||
    value.topics.some((topic) => typeof topic !== "string" || !/^0x[0-9a-fA-F]{64}$/u.test(topic))
  ) {
    throw new RecoverableSignerError("rpc_response", "provider returned malformed ERC-8004 log topics");
  }
  const decoded = decodeEventLog({
    abi: identityAbi,
    data: value.data,
    topics: value.topics as [Hex, ...Hex[]],
    strict: true,
  });
  if (
    !["Registered", "Transfer", "Approval", "ApprovalForAll", "MetadataSet"].includes(
      decoded.eventName,
    ) ||
    !isRecord(decoded.args)
  ) {
    throw new RecoverableSignerError("rpc_response", "provider returned an unexpected ERC-8004 event");
  }
  const args = decoded.args as Record<string, unknown>;
  const result: DiscoveryLog = {
    eventName: decoded.eventName as DiscoveryLog["eventName"],
    ...(typeof value.transactionHash === "string" && /^0x[0-9a-fA-F]{64}$/u.test(value.transactionHash)
      ? { transactionHash: value.transactionHash as Hex }
      : {}),
    args: {},
  };
  if (typeof args.agentId === "bigint") result.args.agentId = args.agentId;
  if (typeof args.tokenId === "bigint") result.args.tokenId = args.tokenId;
  if (typeof args.owner === "string" && isAddress(args.owner, { strict: true })) {
    result.args.owner = getAddress(args.owner);
  }
  if (typeof args.to === "string" && isAddress(args.to, { strict: true })) {
    result.args.to = getAddress(args.to);
  }
  if (typeof args.operator === "string" && isAddress(args.operator, { strict: true })) {
    result.args.operator = getAddress(args.operator);
  }
  if (typeof args.approved === "boolean") {
    result.args.approved = args.approved;
  } else if (typeof args.approved === "string" && isAddress(args.approved, { strict: true })) {
    result.args.approved = getAddress(args.approved);
  }
  if (typeof args.metadataKey === "string") result.args.metadataKey = args.metadataKey;
  if (typeof args.metadataValue === "string" && isHex(args.metadataValue)) {
    result.args.metadataValue = args.metadataValue;
  }
  return result;
}

async function getLogsChunked(
  publicClient: PublicClient,
  eventTopics: readonly Hex[],
  topic2Values: readonly Hex[],
  accept: (log: DiscoveryLog) => boolean,
  budget: { logs: number },
  observedBlockNumber: bigint,
  firstBlock = ERC8004_START_BLOCK,
): Promise<readonly DiscoveryLog[]> {
  const ranges: Array<{ fromBlock: bigint; toBlock: bigint }> = [];
  for (
    let fromBlock = firstBlock;
    fromBlock <= observedBlockNumber;
    fromBlock += LOG_BLOCK_SPAN
  ) {
    ranges.push({
      fromBlock,
      toBlock:
        fromBlock + LOG_BLOCK_SPAN - 1n > observedBlockNumber
          ? observedBlockNumber
          : fromBlock + LOG_BLOCK_SPAN - 1n,
    });
  }
  const chunks: DiscoveryLog[][] = Array.from({ length: ranges.length }, () => []);
  let nextRange = 0;
  let cancelled = false;
  let hasFailure = false;
  let firstFailure: unknown;
  const worker = async (workerIndex: number): Promise<void> => {
    try {
      for (;;) {
        if (cancelled) return;
        const index = nextRange;
        nextRange += 1;
        const range = ranges[index];
        if (range === undefined) return;
        let response: unknown;
        try {
          response = await withBoundedRpcRetry(
            () =>
              publicClient.request({
                method: "eth_getLogs",
                params: [
                  {
                    address: ERC8004_IDENTITY_REGISTRY,
                    topics: [eventTopics, null, topic2Values],
                    fromBlock: toHex(range.fromBlock),
                    toBlock: toHex(range.toBlock),
                  },
                ],
              } as never),
            {
              attempts: 5,
              baseDelayMs: RPC_RETRY_BASE_DELAY_MS + workerIndex * 100,
            },
          );
        } catch (error) {
          if (isTransientRpcError(error)) {
            throw new RecoverableSignerError(
              "discovery_rpc_busy",
              "Base RPC remained rate-limited or unavailable after bounded identity-discovery retries",
            );
          }
          throw error;
        }
        if (!Array.isArray(response)) {
          throw new RecoverableSignerError("rpc_response", "provider returned a malformed eth_getLogs response");
        }
        if (response.length > MAX_DISCOVERY_RAW_LOGS_PER_CHUNK) {
          throw new PermanentSignerError(
            "candidate_limit",
            "one bounded identity discovery chunk returned too many logs",
          );
        }
        const decoded = response.map(decodedDiscoveryLog).filter(accept);
        budget.logs += decoded.length;
        if (budget.logs > MAX_DISCOVERY_LOGS) {
          throw new PermanentSignerError(
            "candidate_limit",
            "bounded identity discovery log budget was exceeded; select an agent ID explicitly",
          );
        }
        chunks[index] = decoded;
      }
    } catch (error) {
      // Stop assigning new chunks immediately, but do not reject while sibling workers still
      // own in-flight RPC promises. Awaiting all workers keeps the one-frame sidecar from
      // emitting a typed error while invisible discovery work holds the process open.
      cancelled = true;
      if (!hasFailure) {
        hasFailure = true;
        firstFailure = error;
      }
    }
  };
  await Promise.all(
    Array.from(
      { length: Math.min(DISCOVERY_CONCURRENCY, ranges.length) },
      (_unused, index) => worker(index),
    ),
  );
  if (hasFailure) throw firstFailure;
  return chunks.flat();
}

function addDiscoveryId(ids: Set<string>, value: bigint | undefined): void {
  if (value === undefined) return;
  ids.add(value.toString());
  if (ids.size > MAX_DISCOVERY_IDS) {
    throw new PermanentSignerError(
      "candidate_limit",
      "wallet has too many historical identity associations for bounded discovery; select an agent ID explicitly",
    );
  }
}

function decodeBoundedDataJson(value: string): Record<string, unknown> | undefined {
  const prefix = "data:application/json;base64,";
  if (!value.startsWith(prefix) || Buffer.byteLength(value, "utf8") > MAX_URI_BYTES) return undefined;
  try {
    const decoded = Buffer.from(value.slice(prefix.length), "base64");
    if (decoded.length > MAX_URI_BYTES) return undefined;
    const parsed: unknown = JSON.parse(decoded.toString("utf8"));
    return isRecord(parsed) ? parsed : undefined;
  } catch {
    return undefined;
  }
}

function profileIdentityEvidence(agentURI: unknown): { tentacleIds: Set<string>; inboxIds: Set<string> } {
  const tentacleIds = new Set<string>();
  const inboxIds = new Set<string>();
  if (typeof agentURI !== "string") return { tentacleIds, inboxIds };
  const visit = (record: Record<string, unknown>, depth: number): void => {
    if (typeof record.tentacleId === "string" && Buffer.byteLength(record.tentacleId, "utf8") <= MAX_TENTACLE_ID_BYTES) {
      tentacleIds.add(record.tentacleId);
    }
    if (isRecord(record.xmtp) && typeof record.xmtp.endpoint === "string") {
      const match = /^xmtp:\/\/([0-9a-f]{64})$/u.exec(record.xmtp.endpoint);
      if (match) inboxIds.add(match[1]!);
    }
    if (!Array.isArray(record.services) || record.services.length > 32) return;
    for (const serviceValue of record.services) {
      if (!isRecord(serviceValue)) continue;
      const endpoint = serviceValue.endpoint ?? serviceValue.uri;
      if (typeof endpoint !== "string") continue;
      if (serviceValue.name === "CTHUWU-XMTP" || serviceValue.name === "XMTP") {
        const match = /^xmtp:\/\/([0-9a-f]{64})$/u.exec(endpoint);
        if (match) inboxIds.add(match[1]!);
      }
      if (depth === 0 && serviceValue.name === "CTHUWU") {
        const manifest = decodeBoundedDataJson(endpoint);
        if (manifest) visit(manifest, 1);
      }
    }
  };
  const profile = decodeBoundedDataJson(agentURI);
  if (profile) visit(profile, 0);
  return { tentacleIds, inboxIds };
}

export function classifyDiscoveredAgent(
  candidate: Record<string, unknown>,
  context: DiscoveryClassificationContext,
): Record<string, unknown> {
  const evidence: string[] = [];
  const metadata = isRecord(candidate.tentacleId) ? candidate.tentacleId : undefined;
  const metadataTentacleId = typeof metadata?.utf8 === "string" && metadata.utf8.length > 0
    ? metadata.utf8
    : undefined;
  const profile = profileIdentityEvidence(candidate.agentURI);
  const exactMetadataTentacle = metadataTentacleId === context.tentacleId;
  const exactProfileTentacle = profile.tentacleIds.has(context.tentacleId);
  const exactXmtp = profile.inboxIds.has(context.xmtpInboxId);
  const conflictingTentacleId =
    (metadataTentacleId !== undefined && metadataTentacleId !== context.tentacleId) ||
    [...profile.tentacleIds].some((value) => value !== context.tentacleId);
  const eligibleMarkers =
    candidate.declaresTentacleAllegiance === true && candidate.protocolCompatible === true;
  const currentRelationship = candidate.authorized === true || candidate.walletVerified === true;
  if (exactMetadataTentacle) evidence.push("exact-tentacle-id");
  if (exactProfileTentacle) evidence.push("exact-profile-tentacle-id");
  if (exactXmtp) evidence.push("exact-xmtp-endpoint");
  if (eligibleMarkers && currentRelationship) evidence.push("legacy-allegiance");
  if (currentRelationship && evidence.length === 0) evidence.push("wallet-only");
  const sameTentacle = currentRelationship && !conflictingTentacleId &&
    (exactMetadataTentacle || exactProfileTentacle || (eligibleMarkers && exactXmtp));
  const provenUnrelated = conflictingTentacleId &&
    !exactMetadataTentacle && !exactProfileTentacle && !exactXmtp;
  // A bare current wallet/authorization relationship is indistinguishable from a just-mined
  // register() whose local journals and metadata reconciliation were lost. It must block mint
  // without being adopted. Only explicit conflicting durable identity provenance can prove that
  // a same-operator identity belongs to another Tentacle.
  const ambiguousTentacle = !sameTentacle && !provenUnrelated &&
    (currentRelationship || eligibleMarkers || exactMetadataTentacle || exactProfileTentacle || exactXmtp);
  return { ...candidate, identityEvidence: evidence, sameTentacle, ambiguousTentacle };
}

export async function discoverAgents(
  publicClient: PublicClient,
  wallet: Address,
  registrationNonce?: string,
  scope: "recent" | "exhaustive" = "exhaustive",
  context?: DiscoveryClassificationContext,
  checkpoint?: DiscoveryJournal,
  observation: "finalized" | "latest" = "finalized",
): Promise<unknown> {
  // Recovery decisions must never compare logs from one RPC head with a nonce from another.
  // Ordinary/checkpoint discovery uses finalized. The last signer-bound refresh uses latest so
  // an externally-created wallet association above finalized cannot hide from the mint gate.
  // Both modes echo and recheck an exact canonical number/hash.
  const observedBlock = await publicClient.getBlock({ blockTag: observation });
  if (observedBlock.hash === null || observedBlock.number < ERC8004_START_BLOCK) {
    throw new RecoverableSignerError(
      "discovery_observation",
      `provider did not return a usable ${observation} ERC-8004 discovery block`,
    );
  }
  const observedBlockNumber = observedBlock.number;
  const observedBlockHash = observedBlock.hash;
  let recentDiscoveryBlocks = RECENT_DISCOVERY_BLOCKS;
  if (scope === "recent" && process.env.NODE_ENV === "test") {
    const override = process.env[TEST_RECENT_DISCOVERY_BLOCKS_ENV];
    if (override !== undefined) {
      if (!/^[1-9][0-9]{0,4}$/u.test(override)) {
        throw new PermanentSignerError(
          "configuration",
          `${TEST_RECENT_DISCOVERY_BLOCKS_ENV} must be a canonical positive integer`,
        );
      }
      recentDiscoveryBlocks = BigInt(override);
      if (recentDiscoveryBlocks > RECENT_DISCOVERY_BLOCKS) {
        throw new PermanentSignerError(
          "configuration",
          `${TEST_RECENT_DISCOVERY_BLOCKS_ENV} cannot exceed the production 20,000-block window`,
        );
      }
    }
  }
  const requestedFirstBlock = scope === "recent"
    ? observedBlockNumber - ERC8004_START_BLOCK + 1n > recentDiscoveryBlocks
      ? observedBlockNumber - recentDiscoveryBlocks + 1n
      : ERC8004_START_BLOCK
    : ERC8004_START_BLOCK;
  if (
    checkpoint !== undefined &&
    (scope !== "exhaustive" ||
      context === undefined ||
      checkpoint.wallet !== wallet ||
      checkpoint.tentacleId !== context.tentacleId ||
      checkpoint.xmtpInboxId !== context.xmtpInboxId ||
      checkpoint.fromBlock !== ERC8004_START_BLOCK.toString() ||
      BigInt(checkpoint.throughBlock) > observedBlockNumber)
  ) {
    throw new RecoverableSignerError(
      "discovery_checkpoint",
      "durable identity-discovery checkpoint is incompatible with this exhaustive request",
    );
  }
  if (checkpoint !== undefined) {
    const canonicalCheckpointBlock = await publicClient.getBlock({
      blockNumber: BigInt(checkpoint.throughBlock),
    });
    if (canonicalCheckpointBlock.hash !== checkpoint.throughBlockHash) {
      throw new RecoverableSignerError(
        "discovery_checkpoint_reorg",
        "durable identity-discovery checkpoint is no longer canonical",
      );
    }
  }
  const firstBlock = checkpoint === undefined
    ? requestedFirstBlock
    : BigInt(checkpoint.throughBlock) + 1n;
  const budget = { logs: 0 };
  const walletTopic = pad(wallet, { size: 32 });
  const combined = await getLogsChunked(
    publicClient,
    Object.values(DISCOVERY_EVENT_TOPICS),
    [walletTopic, AGENT_WALLET_METADATA_TOPIC],
    (log) => {
      switch (log.eventName) {
        case "Registered":
          return log.args.owner === wallet;
        case "Transfer":
          return log.args.to === wallet;
        case "Approval":
          return log.args.approved === wallet;
        case "ApprovalForAll":
          return log.args.operator === wallet;
        case "MetadataSet": {
          const value = log.args.metadataValue;
          return (
            log.args.metadataKey === "agentWallet" &&
            typeof value === "string" &&
            /^0x[0-9a-fA-F]{40}$/u.test(value) &&
            getAddress(value) === wallet
          );
        }
      }
    },
    budget,
    observedBlockNumber,
    firstBlock,
  );
  const registered: DiscoveryLog[] = [];
  const transferred: DiscoveryLog[] = [];
  const approved: DiscoveryLog[] = [];
  const walletMetadata: DiscoveryLog[] = [];
  const operatorEvents: DiscoveryLog[] = [];
  for (const log of combined) {
    switch (log.eventName) {
      case "Registered":
        if (log.args.owner === wallet) registered.push(log);
        break;
      case "Transfer":
        if (log.args.to === wallet) transferred.push(log);
        break;
      case "Approval":
        if (log.args.approved === wallet) approved.push(log);
        break;
      case "ApprovalForAll":
        if (log.args.operator === wallet) operatorEvents.push(log);
        break;
      case "MetadataSet":
        if (log.args.metadataKey === "agentWallet") walletMetadata.push(log);
        break;
    }
  }
  const ids = new Set<string>(checkpoint?.associatedAgentIds ?? []);
  for (const log of registered) addDiscoveryId(ids, log.args.agentId);
  for (const log of transferred) addDiscoveryId(ids, log.args.tokenId);
  for (const log of approved) addDiscoveryId(ids, log.args.tokenId);
  for (const log of walletMetadata) {
    const value = log.args.metadataValue;
    if (
      log.args.metadataKey === "agentWallet" &&
      typeof value === "string" &&
      /^0x[0-9a-fA-F]{40}$/u.test(value) &&
      getAddress(value) === wallet
    ) {
      addDiscoveryId(ids, log.args.agentId);
    }
  }

  const operatorOwners = new Set<Address>(checkpoint?.operatorOwners ?? []);
  const ownersGrantedInCurrentRange = new Set<Address>();
  for (const log of operatorEvents) {
    if (log.args.owner !== undefined) {
      const owner = getAddress(log.args.owner);
      operatorOwners.add(owner);
      if (log.args.approved === true) ownersGrantedInCurrentRange.add(owner);
    }
    if (operatorOwners.size > MAX_OPERATOR_OWNERS) {
      throw new PermanentSignerError(
        "candidate_limit",
        "wallet has too many historical ERC-721 operator owners for bounded discovery",
      );
    }
  }
  for (const owner of operatorOwners) {
    const currentlyApproved = await publicClient.readContract({
      address: ERC8004_IDENTITY_REGISTRY,
      abi: identityAbi,
      functionName: "isApprovedForAll",
      args: [owner, wallet],
      blockNumber: observedBlockNumber,
    });
    if (!currentlyApproved) continue;
    // A new or re-granted blanket approval can expose identities that the owner registered
    // before this checkpoint. Existing checkpoint owners without a new grant were already
    // exhaustively enumerated when their association became current, so only they may advance
    // incrementally. This branch is rare and remains bounded by the canonical log budget.
    const ownerFirstBlock = ownersGrantedInCurrentRange.has(owner)
      ? ERC8004_START_BLOCK
      : firstBlock;
    const ownerLogs = await getLogsChunked(
      publicClient,
      [DISCOVERY_EVENT_TOPICS.Registered, DISCOVERY_EVENT_TOPICS.Transfer],
      [pad(owner, { size: 32 })],
      (log) =>
        (log.eventName === "Registered" && log.args.owner === owner) ||
        (log.eventName === "Transfer" && log.args.to === owner),
      budget,
      observedBlockNumber,
      ownerFirstBlock,
    );
    for (const log of ownerLogs) {
      if (log.eventName === "Registered" && log.args.owner === owner) {
        addDiscoveryId(ids, log.args.agentId);
      }
      if (log.eventName === "Transfer" && log.args.to === owner) {
        addDiscoveryId(ids, log.args.tokenId);
      }
    }
  }
  const candidates = [];
  for (const id of [...ids].sort((left, right) => (BigInt(left) < BigInt(right) ? -1 : 1))) {
    const inspected = await inspectAgentAtBlock(
      publicClient,
      id,
      wallet,
      observedBlockNumber,
      observedBlockHash,
    );
    // A historical association can name a token that canonical ownerOf now proves does not
    // exist. This typed result is authoritative for that candidate only; all provider failures
    // still abort discovery and therefore remain incapable of authorizing registration.
    if (inspected.agentExists === false) continue;
    const candidate = context === undefined
      ? inspected
      : classifyDiscoveredAgent(inspected, context);
    if (
      candidate.authorized === true ||
      candidate.walletVerified === true ||
      candidate.sameTentacle === true ||
      candidate.ambiguousTentacle === true
    ) candidates.push(candidate);
    if (candidates.length > MAX_CANDIDATES) {
      throw new PermanentSignerError(
        "candidate_limit",
        "wallet has too many current candidate identities for bounded discovery; select an agent ID explicitly",
      );
    }
  }
  const canonicalBlock = await publicClient.getBlock({ blockNumber: observedBlockNumber });
  if (canonicalBlock.hash !== observedBlockHash) {
    throw new RecoverableSignerError(
      "discovery_reorg",
      `the ${observation} discovery block changed while candidate discovery was running`,
    );
  }
  const walletRegistrationMap = new Map<string, WalletRegistration>();
  for (const registration of checkpoint?.walletRegistrations ?? []) {
    walletRegistrationMap.set(`${registration.agentId}:${registration.transactionHash}`, registration);
  }
  for (const log of registered) {
    if (log.transactionHash === undefined || log.args.agentId === undefined) {
      throw new RecoverableSignerError(
        "discovery_outcome",
        "provider omitted transaction provenance for a registration event",
      );
    }
    const registration = {
      agentId: log.args.agentId.toString(),
      transactionHash: log.transactionHash,
    };
    walletRegistrationMap.set(`${registration.agentId}:${registration.transactionHash}`, registration);
  }
  const walletRegistrations = [...walletRegistrationMap.values()].sort((left, right) =>
    BigInt(left.agentId) < BigInt(right.agentId) ? -1 :
      BigInt(left.agentId) > BigInt(right.agentId) ? 1 :
        left.transactionHash.localeCompare(right.transactionHash),
  );
  if (walletRegistrations.length > MAX_DISCOVERY_IDS) {
    throw new PermanentSignerError(
      "candidate_limit",
      "wallet has too many historical registrations for bounded recovery",
    );
  }
  const matchedRegistrationAgentIds = new Set<string>();
  if (registrationNonce !== undefined) {
    const requestedNonce = Number(transactionNonce(registrationNonce));
    for (const registration of walletRegistrations) {
      const transaction = await publicClient.getTransaction({
        hash: registration.transactionHash,
      });
      if (
        getAddress(transaction.from) === wallet &&
        transaction.nonce === requestedNonce
      ) {
        matchedRegistrationAgentIds.add(registration.agentId);
      }
    }
    if (matchedRegistrationAgentIds.size > 1) {
      throw new PermanentSignerError(
        "discovery_outcome",
        "one signer nonce unexpectedly produced multiple ERC-8004 identities",
      );
    }
  }
  const coverageFromBlock = checkpoint?.fromBlock ?? requestedFirstBlock.toString();
  const result = {
    complete: scope === "exhaustive" && coverageFromBlock === ERC8004_START_BLOCK.toString(),
    rangeComplete: true,
    scope,
    source: checkpoint === undefined ? "canonical-logs" : "canonical-logs-checkpoint",
    fromBlock: coverageFromBlock,
    observedBlockNumber: observedBlockNumber.toString(),
    observedBlockHash,
    coverage: {
      fromBlock: coverageFromBlock,
      throughBlock: observedBlockNumber.toString(),
      throughBlockHash: observedBlockHash,
    },
    matchedRegistrationAgentIds: [...matchedRegistrationAgentIds],
    candidates,
  } as unknown as DiscoveryResultWithInternal;
  Object.defineProperty(result, DISCOVERY_INTERNAL, {
    value: {
      associatedAgentIds: [...ids].sort((left, right) => BigInt(left) < BigInt(right) ? -1 : 1),
      walletRegistrations,
      operatorOwners: [...operatorOwners].sort(),
      ...(context === undefined ? {} : { classification: context }),
    } satisfies DiscoveryInternalState,
    enumerable: false,
  });
  return result;
}

function sha256Hex(value: unknown): string {
  return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}

function discoveryJournalPath(identity: LoadedIdentity): string {
  return path.join(path.dirname(identity.identityPath), "erc8004-discovery-v1.json");
}

async function replacePrivateFile(target: string, contents: string): Promise<void> {
  const directory = path.dirname(target);
  const directoryStat = await lstat(directory);
  if (!directoryStat.isDirectory() || directoryStat.isSymbolicLink()) {
    throw new PermanentSignerError(
      "discovery_journal",
      "identity-discovery state directory is not a private regular directory",
    );
  }
  await chmod(directory, 0o700);
  const temporary = `${target}.${process.pid}.${randomUUID()}.tmp`;
  const handle = await open(temporary, "wx", 0o600);
  try {
    await handle.writeFile(contents, { encoding: "utf8" });
    await handle.sync();
  } finally {
    await handle.close();
  }
  try {
    await rename(temporary, target);
    await chmod(target, 0o600);
  } finally {
    await unlink(temporary).catch(() => undefined);
  }
}

function candidateSetHash(candidates: unknown): Hex {
  if (!Array.isArray(candidates)) {
    throw new PermanentSignerError("discovery_journal", "candidate set is unavailable");
  }
  const bounded = candidates.map((value) => {
    if (!isRecord(value)) {
      throw new PermanentSignerError("discovery_journal", "candidate set is malformed");
    }
    return {
      agentId: value.agentId,
      owner: value.owner,
      agentURI: value.agentURI,
      agentWallet: value.agentWallet,
      authorized: value.authorized,
      walletVerified: value.walletVerified,
      allegiance: value.allegiance,
      protocol: value.protocol,
      tentacleId: value.tentacleId,
      sameTentacle: value.sameTentacle,
      ambiguousTentacle: value.ambiguousTentacle,
    };
  });
  return `0x${sha256Hex(bounded)}`;
}

function buildDiscoveryJournal(
  result: DiscoveryResultWithInternal,
  wallet: Address,
  context: DiscoveryClassificationContext,
): DiscoveryJournal {
  const coverage = result.coverage;
  if (!isRecord(coverage)) {
    throw new PermanentSignerError("discovery_journal", "complete discovery omitted coverage");
  }
  const base = {
    version: 1 as const,
    chainId: ERC8004_CHAIN_ID as 8453,
    registry: ERC8004_IDENTITY_REGISTRY,
    wallet,
    tentacleId: context.tentacleId,
    xmtpInboxId: context.xmtpInboxId,
    fromBlock: decimalId(coverage.fromBlock),
    throughBlock: decimalId(coverage.throughBlock),
    throughBlockHash: transactionHash(coverage.throughBlockHash),
    associatedAgentIds: result[DISCOVERY_INTERNAL].associatedAgentIds,
    walletRegistrations: result[DISCOVERY_INTERNAL].walletRegistrations,
    operatorOwners: result[DISCOVERY_INTERNAL].operatorOwners,
  };
  const checkpointFingerprint = sha256Hex(base);
  let mint: MintAuthorization | undefined;
  const candidates = Array.isArray(result.candidates) ? result.candidates : [];
  const blocked = candidates.some((candidate) =>
    isRecord(candidate) && (candidate.sameTentacle === true || candidate.ambiguousTentacle === true));
  if (
    result.complete === true &&
    base.fromBlock === ERC8004_START_BLOCK.toString() &&
    !blocked
  ) {
    const authorizationBase = {
      version: 1 as const,
      chainId: ERC8004_CHAIN_ID as 8453,
      registry: ERC8004_IDENTITY_REGISTRY,
      wallet,
      tentacleId: context.tentacleId,
      xmtpInboxId: context.xmtpInboxId,
      fromBlock: base.fromBlock,
      throughBlock: base.throughBlock,
      throughBlockHash: base.throughBlockHash,
      candidateSetHash: candidateSetHash(candidates),
    };
    mint = { ...authorizationBase, fingerprint: sha256Hex(authorizationBase) };
  }
  return {
    ...base,
    checkpointFingerprint,
    ...(mint === undefined ? {} : { mintAuthorization: mint }),
  };
}

function parseDiscoveryJournal(raw: string): DiscoveryJournal {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    throw new RecoverableSignerError("discovery_checkpoint", "identity-discovery checkpoint is not valid JSON");
  }
  if (!isRecord(value)) {
    throw new RecoverableSignerError("discovery_checkpoint", "identity-discovery checkpoint has an invalid shape");
  }
  const expectedKeys = [
    "version", "chainId", "registry", "wallet", "tentacleId", "xmtpInboxId", "fromBlock",
    "throughBlock", "throughBlockHash", "associatedAgentIds", "walletRegistrations",
    "operatorOwners", "checkpointFingerprint",
    ...(value.mintAuthorization === undefined ? [] : ["mintAuthorization"]),
  ];
  try {
    exactKeys(value, expectedKeys);
    if (value.version !== 1 || value.chainId !== ERC8004_CHAIN_ID) throw new Error("version");
    const registry = walletAddress(value.registry);
    if (registry !== ERC8004_IDENTITY_REGISTRY) throw new Error("registry");
    if (!Array.isArray(value.associatedAgentIds) || value.associatedAgentIds.length > MAX_DISCOVERY_IDS ||
        value.associatedAgentIds.some((id) => typeof id !== "string" || decimalId(id) !== id) ||
        new Set(value.associatedAgentIds).size !== value.associatedAgentIds.length) throw new Error("ids");
    if (!Array.isArray(value.walletRegistrations) || value.walletRegistrations.length > MAX_DISCOVERY_IDS) throw new Error("registrations");
    const walletRegistrations = value.walletRegistrations.map((entry) => {
      if (!isRecord(entry)) throw new Error("registration");
      exactKeys(entry, ["agentId", "transactionHash"]);
      return { agentId: decimalId(entry.agentId), transactionHash: transactionHash(entry.transactionHash) };
    });
    if (!Array.isArray(value.operatorOwners) || value.operatorOwners.length > MAX_OPERATOR_OWNERS) throw new Error("owners");
    const operatorOwners = value.operatorOwners.map(walletAddress);
    const parsed = {
      version: 1 as const,
      chainId: ERC8004_CHAIN_ID as 8453,
      registry,
      wallet: walletAddress(value.wallet),
      tentacleId: boundedString(value.tentacleId, "tentacleId", MAX_TENTACLE_ID_BYTES),
      xmtpInboxId: xmtpInboxId(value.xmtpInboxId),
      fromBlock: decimalId(value.fromBlock),
      throughBlock: decimalId(value.throughBlock),
      throughBlockHash: transactionHash(value.throughBlockHash),
      associatedAgentIds: value.associatedAgentIds as string[],
      walletRegistrations,
      operatorOwners,
    };
    const checkpointFingerprint = fingerprint(value.checkpointFingerprint, "checkpoint fingerprint");
    if (sha256Hex(parsed) !== checkpointFingerprint) throw new Error("fingerprint");
    return {
      ...parsed,
      checkpointFingerprint,
      ...(value.mintAuthorization === undefined
        ? {}
        : { mintAuthorization: mintAuthorization(value.mintAuthorization) }),
    };
  } catch (error) {
    if (error instanceof RecoverableSignerError) throw error;
    throw new RecoverableSignerError(
      "discovery_checkpoint",
      "identity-discovery checkpoint is incomplete or does not match its provenance",
    );
  }
}

async function readDiscoveryJournal(identity: LoadedIdentity): Promise<DiscoveryJournal> {
  const target = discoveryJournalPath(identity);
  const stat = await lstat(target);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > MAX_DISCOVERY_JOURNAL_BYTES) {
    throw new RecoverableSignerError(
      "discovery_checkpoint",
      "identity-discovery checkpoint is not a bounded regular file",
    );
  }
  await chmod(target, 0o600);
  return parseDiscoveryJournal(await readFile(target, "utf8"));
}

async function persistDiscoveryJournal(
  identity: LoadedIdentity,
  journal: DiscoveryJournal,
): Promise<void> {
  const encoded = `${JSON.stringify(journal)}\n`;
  if (Buffer.byteLength(encoded, "utf8") > MAX_DISCOVERY_JOURNAL_BYTES) {
    throw new PermanentSignerError("discovery_journal", "identity-discovery checkpoint exceeds its bound");
  }
  await replacePrivateFile(discoveryJournalPath(identity), encoded);
}

function mintAuthorizationBase(value: MintAuthorization): Omit<MintAuthorization, "fingerprint"> {
  const { fingerprint: _fingerprint, ...base } = value;
  return base;
}

function assertMintAuthorizationFingerprint(value: MintAuthorization): void {
  if (sha256Hex(mintAuthorizationBase(value)) !== value.fingerprint) {
    throw new PermanentSignerError(
      "mint_authorization",
      "mint authorization does not match its canonical discovery fingerprint",
    );
  }
}

async function readNonceAtCanonicalCoverage(
  publicClient: PublicClient,
  wallet: Address,
  blockNumber: bigint,
  blockHash: Hex,
): Promise<number> {
  const before = await publicClient.getBlock({ blockNumber });
  if (before.hash !== blockHash) {
    throw new RecoverableSignerError(
      "discovery_reorg",
      "complete discovery coverage is no longer canonical",
    );
  }
  const rawNonce: unknown = await publicClient.request({
    method: "eth_getTransactionCount",
    params: [wallet, { blockHash, requireCanonical: true }],
  } as never);
  if (
    typeof rawNonce !== "string" ||
    !/^0x(?:0|[1-9a-fA-F][0-9a-fA-F]*)$/u.test(rawNonce)
  ) {
    throw new RecoverableSignerError(
      "registration_nonce_state",
      "provider returned an invalid nonce at complete discovery coverage",
    );
  }
  const parsed = BigInt(rawNonce);
  if (parsed > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new PermanentSignerError(
      "registration_nonce_state",
      "production wallet nonce exceeds the exact signer range",
    );
  }
  const after = await publicClient.getBlock({ blockNumber });
  if (after.hash !== blockHash) {
    throw new RecoverableSignerError(
      "discovery_reorg",
      "complete discovery coverage changed during the nonce read",
    );
  }
  return Number(parsed);
}

type MintAuthorizationAllocation = {
  version: 1;
  fingerprint: string;
  actionId: string;
  nonce: string;
};

function mintAuthorizationAllocationPath(identity: LoadedIdentity): string {
  return path.join(
    path.dirname(identity.identityPath),
    "erc8004-registration-mint-v1.json",
  );
}

function parseMintAuthorizationAllocation(raw: string): MintAuthorizationAllocation {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    throw new PermanentSignerError(
      "mint_authorization",
      "registration mint authorization journal is not valid JSON",
    );
  }
  if (!isRecord(value)) {
    throw new PermanentSignerError(
      "mint_authorization",
      "registration mint authorization journal has an invalid shape",
    );
  }
  exactKeys(value, ["version", "fingerprint", "actionId", "nonce"]);
  if (value.version !== 1) {
    throw new PermanentSignerError(
      "mint_authorization",
      "registration mint authorization journal has another protocol version",
    );
  }
  return {
    version: 1,
    fingerprint: fingerprint(value.fingerprint, "allocated mint authorization fingerprint"),
    actionId: boundedString(value.actionId, "allocated registration actionId", 128),
    nonce: transactionNonce(value.nonce),
  };
}

async function readMintAuthorizationAllocation(
  identity: LoadedIdentity,
): Promise<MintAuthorizationAllocation | undefined> {
  const target = mintAuthorizationAllocationPath(identity);
  let stat;
  try {
    stat = await lstat(target);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > MAX_SIGNER_JOURNAL_BYTES) {
    throw new PermanentSignerError(
      "mint_authorization",
      "registration mint authorization journal is not a bounded regular file",
    );
  }
  await chmod(target, 0o600);
  return parseMintAuthorizationAllocation(await readFile(target, "utf8"));
}

export async function selectDiscoveryMintAuthorization(
  identity: LoadedIdentity,
  candidate: MintAuthorization | undefined,
  registrationActionId?: string,
  registrationNonce?: string,
): Promise<MintAuthorization | undefined> {
  if (candidate === undefined) return undefined;
  const allocation = await readMintAuthorizationAllocation(identity);
  if (allocation === undefined) return candidate;
  return registrationActionId === allocation.actionId && registrationNonce === allocation.nonce
    ? candidate
    : undefined;
}

export function buildPublicDiscoveryResult(
  discovered: Record<string, unknown>,
  checkpoint: { version: 1; fingerprint: string; throughBlock: string; throughBlockHash: Hex },
  mintAuthorization: MintAuthorization | undefined,
): Record<string, unknown> {
  const result = { ...discovered };
  // The internal discovery result is deliberately optimistic so it can be journaled. The
  // allocation decision is a separate owner-only boundary; absence or mismatch must remove,
  // not merely decline to overwrite, the optimistic proof.
  delete result.mintAuthorization;
  result.checkpoint = checkpoint;
  if (mintAuthorization !== undefined) result.mintAuthorization = mintAuthorization;
  return result;
}

async function bindMintAuthorizationUse(
  identity: LoadedIdentity,
  authorization: MintAuthorization,
  actionId: string,
  nonce: string,
): Promise<void> {
  const target = mintAuthorizationAllocationPath(identity);
  const allocation: MintAuthorizationAllocation = {
    version: 1 as const,
    fingerprint: authorization.fingerprint,
    actionId,
    nonce,
  };
  const encoded = `${JSON.stringify(allocation)}\n`;
  const installed = await installSignerAllocation(target, encoded);
  if (!installed) {
    const existing = await readMintAuthorizationAllocation(identity);
    if (
      existing === undefined ||
      existing.actionId !== allocation.actionId ||
      existing.nonce !== allocation.nonce
    ) {
      throw new PermanentSignerError(
        "mint_authorization_reused",
        "complete discovery already authorized another registration action",
      );
    }
  }
}

export async function authorizeRegistrationMint(
  publicClient: PublicClient,
  identity: LoadedIdentity,
  actionId: string,
  operation: Extract<WriteOperation, { type: "register" }>,
): Promise<void> {
  const authorization = operation.mintAuthorization;
  if (authorization === undefined) {
    throw new PermanentSignerError(
      "mint_authorization",
      "register requires positively complete historical identity discovery",
    );
  }
  const wallet = assertProductionIdentity(identity, authorization.wallet);
  assertMintAuthorizationFingerprint(authorization);
  if (
    authorization.chainId !== ERC8004_CHAIN_ID ||
    authorization.registry !== ERC8004_IDENTITY_REGISTRY ||
    authorization.fromBlock !== ERC8004_START_BLOCK.toString()
  ) {
    throw new PermanentSignerError(
      "mint_authorization",
      "mint authorization does not cover the canonical ERC-8004 deployment history",
    );
  }
  const allocation = await readMintAuthorizationAllocation(identity);
  const exactReplay = allocation !== undefined &&
    allocation.actionId === actionId &&
    allocation.nonce === operation.nonce;
  if (allocation !== undefined && !exactReplay) {
    throw new PermanentSignerError(
      "mint_authorization_reused",
      "complete discovery already authorized another registration action",
    );
  }
  const journal = await readDiscoveryJournal(identity);
  if (
    journal.wallet !== wallet ||
    journal.tentacleId !== authorization.tentacleId ||
    journal.xmtpInboxId !== authorization.xmtpInboxId ||
    BigInt(journal.throughBlock) < BigInt(authorization.throughBlock) ||
    (!exactReplay && (
      journal.mintAuthorization === undefined ||
      JSON.stringify(journal.mintAuthorization) !== JSON.stringify(authorization)
    ))
  ) {
    throw new PermanentSignerError(
      "mint_authorization",
      "register authorization does not match the owner-only discovery journal",
    );
  }
  const canonical = await publicClient.getBlock({ blockNumber: BigInt(authorization.throughBlock) });
  if (canonical.hash !== authorization.throughBlockHash) {
    throw new RecoverableSignerError(
      "mint_authorization_reorg",
      "the complete identity-discovery authorization block is no longer canonical",
    );
  }
  const context = {
    tentacleId: authorization.tentacleId,
    xmtpInboxId: authorization.xmtpInboxId,
  };
  const refreshed = await discoverAgents(
    publicClient,
    wallet,
    undefined,
    "exhaustive",
    context,
    journal,
    "latest",
  ) as DiscoveryResultWithInternal;
  const candidates = Array.isArray(refreshed.candidates) ? refreshed.candidates : [];
  if (candidates.some((candidate) =>
    isRecord(candidate) && (candidate.sameTentacle === true || candidate.ambiguousTentacle === true))) {
    throw new RecoverableSignerError(
      "mint_authorization_stale",
      "canonical discovery found an existing or ambiguous Cthuwu identity; refusing to register",
    );
  }
  const coverageBlock = BigInt(decimalId(refreshed.observedBlockNumber));
  const coverageHash = transactionHash(refreshed.observedBlockHash);
  if (!exactReplay) {
    const [pendingNonce, coveredNonce] = await Promise.all([
      publicClient.getTransactionCount({ address: wallet, blockTag: "pending" }),
      readNonceAtCanonicalCoverage(publicClient, wallet, coverageBlock, coverageHash),
    ]);
    if (
      !Number.isSafeInteger(pendingNonce) ||
      !Number.isSafeInteger(coveredNonce) ||
      pendingNonce < coveredNonce
    ) {
      throw new RecoverableSignerError(
        "registration_nonce_state",
        "provider returned an invalid production wallet nonce window",
      );
    }
    if (pendingNonce !== coveredNonce) {
      throw new RecoverableSignerError(
        "registration_nonce_uncertain",
        "a transaction confirmed or pending beyond complete discovery coverage blocks first ERC-8004 registration",
      );
    }
    assertTransactionNonceWindow(operation.nonce, pendingNonce, coveredNonce, false);
  }
  const currentHead = await publicClient.getBlock({ blockTag: "latest" });
  if (currentHead.number !== coverageBlock || currentHead.hash !== coverageHash) {
    throw new RecoverableSignerError(
      "registration_head_advanced",
      "canonical Base advanced after complete discovery; retry before ERC-8004 registration",
    );
  }
  // Allocate this proof globally before the signer can be reached. The fixed owner-only
  // journal permits an exact lost-response replay, but no second proof/action/nonce can ever
  // authorize another registration from this durable installation.
  await bindMintAuthorizationUse(identity, authorization, actionId, operation.nonce);
  const refreshedJournal = buildDiscoveryJournal(refreshed, wallet, context);
  await persistDiscoveryJournal(identity, refreshedJournal);
}

function envBigInt(name: string, fallback: bigint): bigint {
  const value = process.env[name];
  if (value === undefined) return fallback;
  if (!/^(0|[1-9][0-9]{0,30})$/u.test(value)) {
    throw new PermanentSignerError("configuration", `${name} must be a canonical nonnegative integer`);
  }
  return BigInt(value);
}

async function feeParameters(publicClient: PublicClient): Promise<{ maxFeePerGas: bigint; maxPriorityFeePerGas: bigint }> {
  const fees = await publicClient.estimateFeesPerGas();
  const maxFeePerGas = fees.maxFeePerGas;
  const maxPriorityFeePerGas = fees.maxPriorityFeePerGas;
  if (maxFeePerGas === undefined || maxPriorityFeePerGas === undefined) {
    throw new RecoverableSignerError("fee_estimate", "provider did not return EIP-1559 fee estimates");
  }
  const ceiling = envBigInt("CTHUWU_ERC8004_MAX_FEE_PER_GAS_WEI", DEFAULT_MAX_FEE_PER_GAS_WEI);
  if (maxFeePerGas > ceiling) {
    throw new RecoverableSignerError("fee_ceiling", "current Base fee exceeds the configured signing ceiling");
  }
  return { maxFeePerGas, maxPriorityFeePerGas };
}

function calldataFor(operation: WriteIntent, wallet: Address, nowSeconds: bigint): { data: Hex; functionName: "register" | "setAgentURI" | "setMetadata" | "setAgentWallet"; args: readonly unknown[] } {
  switch (operation.type) {
    case "register":
      return { data: encodeFunctionData({ abi: identityAbi, functionName: "register" }), functionName: "register", args: [] };
    case "set_agent_uri": {
      const args = [BigInt(operation.agentId), operation.agentURI] as const;
      return { data: encodeFunctionData({ abi: identityAbi, functionName: "setAgentURI", args }), functionName: "setAgentURI", args };
    }
    case "set_metadata": {
      const args = [BigInt(operation.agentId), operation.key, stringToHex(operation.value)] as const;
      return { data: encodeFunctionData({ abi: identityAbi, functionName: "setMetadata", args }), functionName: "setMetadata", args };
    }
    case "set_agent_wallet": {
      // Signature is populated by the write path. A zeroed placeholder is only used for bounded
      // calldata/L1 estimation and can never be broadcast. Its 65-byte length keeps the Base L1
      // data estimate representative without exposing or creating a signature.
      const placeholderSignature = `0x${"00".repeat(65)}` as Hex;
      const args = [BigInt(operation.agentId), wallet, nowSeconds + 300n, placeholderSignature] as const;
      return { data: encodeFunctionData({ abi: identityAbi, functionName: "setAgentWallet", args }), functionName: "setAgentWallet", args };
    }
  }
}

export function prepareErc8004Transaction(
  operation: WriteIntent,
  wallet: Address,
  nowSeconds: bigint,
  walletSignature?: Hex,
): PreparedErc8004Transaction {
  let data: Hex;
  if (operation.type === "set_agent_wallet") {
    if (walletSignature === undefined) {
      throw new PermanentSignerError("signature", "agent-wallet publication requires its scoped EIP-712 signature");
    }
    data = encodeFunctionData({
      abi: identityAbi,
      functionName: "setAgentWallet",
      args: [BigInt(operation.agentId), wallet, nowSeconds + 240n, walletSignature],
    });
  } else {
    data = calldataFor(operation, wallet, nowSeconds).data;
  }
  if (!isAllowedCalldata(data)) {
    throw new PermanentSignerError("calldata", "prepared calldata is outside the ERC-8004 allowlist");
  }
  return { chainId: ERC8004_CHAIN_ID, to: ERC8004_IDENTITY_REGISTRY, value: 0n, data };
}

async function fundingEstimate(publicClient: PublicClient, operation: Extract<ReadOperation, { type: "funding_estimate" }>): Promise<unknown> {
  const wallet = getAddress(operation.wallet);
  const writes: WriteIntent[] = [];
  if (operation.agentId === undefined) writes.push({ type: "register" });
  const estimateId = operation.agentId ?? "0";
  if (operation.includeAgentUri) {
    writes.push({ type: "set_agent_uri", agentId: estimateId, agentURI: operation.agentURI });
  }
  if (operation.includeWalletVerification) writes.push({ type: "set_agent_wallet", agentId: estimateId });
  for (const entry of operation.metadata) writes.push({ type: "set_metadata", agentId: estimateId, ...entry });
  const { maxFeePerGas, maxPriorityFeePerGas } = await feeParameters(publicClient);
  const gasCeiling = envBigInt("CTHUWU_ERC8004_MAX_GAS_PER_TRANSACTION", DEFAULT_MAX_GAS_PER_TRANSACTION);
  let executionGas = 0n;
  let exactOperations = 0;
  const now = BigInt(Math.floor(Date.now() / 1000));
  const l1Calls: Hex[] = [];
  for (const write of writes) {
    const call = calldataFor(write, wallet, now);
    let gas = gasCeiling;
    if (
      write.type === "register" ||
      (operation.agentId !== undefined && write.type !== "set_agent_wallet")
    ) {
      try {
        gas = await publicClient.estimateGas({ account: wallet, to: ERC8004_IDENTITY_REGISTRY, data: call.data, value: 0n });
        exactOperations += 1;
      } catch {
        // Some providers reject eth_estimateGas for an otherwise valid authorized registry write.
        // Funding estimation is allowed to remain conservative: the configured per-transaction
        // ceiling plus safety factor is safer and more useful than discarding the independently
        // observable wallet balance. The actual write path still estimates and fails closed before
        // signing, so this fallback cannot authorize or broadcast a reverting call.
      }
    }
    if (gas > gasCeiling) throw new RecoverableSignerError("gas_ceiling", "estimated registry gas exceeds the configured ceiling");
    executionGas += gas;
    l1Calls.push(call.data);
  }
  const conservativeL1FeePerTransaction = gasCeiling * maxFeePerGas;
  const l1Estimate = await sumL1FeesWithConservativeFallback(
    l1Calls,
    (data) =>
      publicClient.estimateL1Fee({
        account: wallet,
        to: ERC8004_IDENTITY_REGISTRY,
        data,
      }),
    conservativeL1FeePerTransaction,
  );
  const l1Fee = l1Estimate.fee;
  const balance = await publicClient.getBalance({ address: wallet });
  const safetyBps = envBigInt("CTHUWU_ERC8004_GAS_SAFETY_BPS", DEFAULT_SAFETY_BPS);
  if (safetyBps < 10_000n || safetyBps > 50_000n) {
    throw new PermanentSignerError("configuration", "gas safety factor must be between 10000 and 50000 basis points");
  }
  const reserve = envBigInt("CTHUWU_ERC8004_POST_REGISTRATION_RESERVE_WEI", DEFAULT_RESERVE_WEI);
  const estimatedCost = executionGas * maxFeePerGas + l1Fee;
  const targetBalance = (estimatedCost * safetyBps + 9_999n) / 10_000n + reserve;
  return {
    wallet,
    balanceWei: balance.toString(),
    estimatedCostWei: estimatedCost.toString(),
    targetBalanceWei: targetBalance.toString(),
    shortfallWei: (targetBalance > balance ? targetBalance - balance : 0n).toString(),
    executionGas: executionGas.toString(),
    l1DataFeeWei: l1Fee.toString(),
    l1DataFeeExact: l1Estimate.exact,
    maxFeePerGasWei: maxFeePerGas.toString(),
    maxPriorityFeePerGasWei: maxPriorityFeePerGas.toString(),
    safetyBps: safetyBps.toString(),
    reserveWei: reserve.toString(),
    exactOperations,
    conservativeOperations: writes.length - exactOperations,
  };
}

async function inspectReceipt(publicClient: PublicClient, hash: Hex): Promise<unknown> {
  let receipt;
  try {
    receipt = await publicClient.getTransactionReceipt({ hash });
  } catch (error) {
    const name = error instanceof Error ? error.name : "";
    if (name.includes("TransactionReceiptNotFound")) return { status: "pending", transactionHash: hash };
    throw error;
  }
  let agentId: string | null = null;
  for (const log of receipt.logs) {
    if (getAddress(log.address) !== ERC8004_IDENTITY_REGISTRY) continue;
    try {
      const decoded = decodeEventLog({ abi: identityAbi, data: log.data, topics: log.topics, strict: true });
      if (decoded.eventName === "Registered") agentId = decoded.args.agentId.toString();
    } catch {
      // Other canonical registry logs are irrelevant to registration receipt extraction.
    }
  }
  const block = await publicClient.getBlock({ blockNumber: receipt.blockNumber });
  return {
    status: receipt.status === "success" ? "success" : "reverted",
    transactionHash: receipt.transactionHash,
    blockNumber: receipt.blockNumber.toString(),
    blockHash: receipt.blockHash,
    canonicalBlockHash: block.hash,
    confirmations: (await publicClient.getBlockNumber()) - receipt.blockNumber + 1n,
    agentId,
  };
}

export async function readSignerNonceState(
  publicClient: PublicClient,
  wallet: Address,
  observation?: { observedBlockNumber: string; observedBlockHash: Hex },
): Promise<Record<string, unknown>> {
  let observed: { observedBlockNumber: string; observedBlockHash: Hex } | undefined;
  let latestNonce: number;
  if (observation !== undefined) {
    const blockNumber = BigInt(observation.observedBlockNumber);
    const expectedHash = observation.observedBlockHash;
    const before = await publicClient.getBlock({ blockNumber });
    if (before.hash !== expectedHash) {
      throw new RecoverableSignerError(
        "discovery_reorg",
        "the requested discovery block is no longer canonical",
      );
    }
    // EIP-1898 binds the account nonce to the exact block hash rather than merely a block
    // number, closing inconsistent-head races behind load-balanced RPC endpoints.
    const rawNonce: unknown = await publicClient.request({
      method: "eth_getTransactionCount",
      params: [wallet, { blockHash: expectedHash, requireCanonical: true }],
    } as never);
    if (
      typeof rawNonce !== "string" ||
      !/^0x(?:0|[1-9a-fA-F][0-9a-fA-F]*)$/u.test(rawNonce)
    ) {
      throw new RecoverableSignerError(
        "nonce_state",
        "provider returned an invalid EIP-1898 confirmed nonce",
      );
    }
    const parsedNonce = BigInt(rawNonce);
    if (parsedNonce > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new PermanentSignerError(
        "nonce_state",
        "provider returned a confirmed nonce outside the exact integer range",
      );
    }
    latestNonce = Number(parsedNonce);
    const after = await publicClient.getBlock({ blockNumber });
    if (after.hash !== expectedHash) {
      throw new RecoverableSignerError(
        "discovery_reorg",
        "the requested discovery block changed during the nonce read",
      );
    }
    observed = observation;
  } else {
    latestNonce = await publicClient.getTransactionCount({
      address: wallet,
      blockTag: "latest",
    });
  }
  const pendingNonce = await publicClient.getTransactionCount({
    address: wallet,
    blockTag: "pending",
  });
  if (pendingNonce < latestNonce) {
    throw new RecoverableSignerError(
      "nonce_state",
      "provider returned a pending nonce below the confirmed nonce",
    );
  }
  return {
    wallet,
    chainId: ERC8004_CHAIN_ID,
    registry: ERC8004_IDENTITY_REGISTRY,
    pendingNonce: pendingNonce.toString(),
    latestNonce: latestNonce.toString(),
    ...observed,
  };
}

async function executeWrite(
  publicClient: PublicClient,
  identity: LoadedIdentity,
  actionId: string,
  operation: WriteOperation,
  rpcEndpoint: string,
): Promise<unknown> {
  assertProductionIdentity(identity);
  if (operation.type === "register") {
    await authorizeRegistrationMint(publicClient, identity, actionId, operation);
  }
  const account = privateKeyToAccount(identity.walletKey);
  const fees = await feeParameters(publicClient);
  const gasCeiling = envBigInt("CTHUWU_ERC8004_MAX_GAS_PER_TRANSACTION", DEFAULT_MAX_GAS_PER_TRANSACTION);
  const walletClient = createWalletClient({ account, chain: base, transport: http(rpcEndpoint, { timeout: 20_000, retryCount: 0 }) });
  const id = operation.type === "register" ? undefined : BigInt(operation.agentId);
  let prepared: PreparedErc8004Transaction;
  if (operation.type === "set_agent_wallet") {
    const owner = await publicClient.readContract({ address: ERC8004_IDENTITY_REGISTRY, abi: identityAbi, functionName: "ownerOf", args: [id as bigint] });
    const deadline = BigInt(Math.floor(Date.now() / 1000) + 240);
    const signature = await account.signTypedData({
      domain: { name: "ERC8004IdentityRegistry", version: "1", chainId: ERC8004_CHAIN_ID, verifyingContract: ERC8004_IDENTITY_REGISTRY },
      types: { AgentWalletSet: [
        { name: "agentId", type: "uint256" },
        { name: "newWallet", type: "address" },
        { name: "owner", type: "address" },
        { name: "deadline", type: "uint256" },
      ] },
      primaryType: "AgentWalletSet",
      message: { agentId: id as bigint, newWallet: account.address, owner, deadline },
    });
    const data = encodeFunctionData({
      abi: identityAbi,
      functionName: "setAgentWallet",
      args: [id as bigint, account.address, deadline, signature],
    });
    if (!isAllowedCalldata(data)) {
      throw new PermanentSignerError("calldata", "prepared calldata is outside the ERC-8004 allowlist");
    }
    prepared = { chainId: ERC8004_CHAIN_ID, to: ERC8004_IDENTITY_REGISTRY, value: 0n, data };
  } else {
    prepared = prepareErc8004Transaction(
      operation,
      account.address,
      BigInt(Math.floor(Date.now() / 1000)),
    );
  }
  const gas = await publicClient.estimateGas({ account, to: prepared.to, data: prepared.data, value: prepared.value });
  if (gas > gasCeiling) throw new RecoverableSignerError("gas_ceiling", "estimated registry gas exceeds the configured ceiling");
  const [pending, latest] = await Promise.all([
    publicClient.getTransactionCount({ address: account.address, blockTag: "pending" }),
    publicClient.getTransactionCount({ address: account.address, blockTag: "latest" }),
  ]);
  const requestedNonce = assertTransactionNonceWindow(
    transactionNonce(operation.nonce),
    pending,
    latest,
    true,
  );
  const allocationExisted = await authorizeSignerNonce(
    identity,
    actionId,
    operation,
    requestedNonce,
    pending,
  );
  const exactNonce = assertTransactionNonceWindow(
    operation.nonce,
    pending,
    latest,
    allocationExisted,
  );
  const hash = await walletClient.sendTransaction({
    account,
    chain: base,
    to: prepared.to,
    data: prepared.data,
    value: prepared.value,
    gas,
    ...fees,
    nonce: exactNonce,
  });
  return {
    transactionHash: hash,
    wallet: account.address,
    chainId: ERC8004_CHAIN_ID,
    registry: ERC8004_IDENTITY_REGISTRY,
    valueWei: "0",
    transactionNonce: operation.nonce,
  };
}

type CanonicalBrandingObservation = {
  blockNumber: bigint;
  blockHash: Hex;
  blockTimestamp: bigint;
};

type BrandingStatusName =
  | "unminted"
  | "active"
  | "expired"
  | "ineligible"
  | "registry_unavailable";

type BrandingView = {
  tokenId: bigint;
  acolyte: Address;
  owner: Address;
  controllerAgentId: bigint;
  referrer: Address;
  declaredPrice: bigint;
  paidThrough: bigint;
  pendingDeclaredPrice: bigint;
  pendingPriceActivation: bigint;
  status: number;
};

type BrandingSnapshot = {
  observation: CanonicalBrandingObservation;
  minter: Address;
  tokenId: bigint;
  branding: BrandingView;
  brandingStatus: BrandingStatusName;
  currentConsentNonce: bigint;
  firstWeekUpkeep: bigint;
  ethBalance: bigint;
  uwuBalance: bigint;
  allowance: bigint;
  nameTrait: string | null;
};

type BrandingFunding = {
  ethTarget: bigint;
  ethShortfall: bigint;
  uwuTarget: bigint;
  uwuShortfall: bigint;
  estimatedCost: bigint;
  executionGas: bigint;
  l1DataFee: bigint;
  l1DataFeeExact: boolean;
  maxFeePerGas: bigint;
  maxPriorityFeePerGas: bigint;
  safetyBps: bigint;
  reserve: bigint;
  exactOperations: BrandingTransactionPhase[];
  conservativeOperations: BrandingTransactionPhase[];
};

type PreparedBrandingPhase = {
  operation: BrandingTransactionPhase;
  to: Address;
  data: Hex;
};

export function isAllowedBrandingTransaction(
  operation: BrandingTransactionPhase,
  to: string,
  data: string,
): boolean {
  if (!isAddress(to, { strict: true }) || !isHex(data) || data.length < 10) return false;
  const selector = data.slice(0, 10);
  if (operation === "approve") {
    return (
      getAddress(to) === CANONICAL_UWU &&
      selector ===
        encodeFunctionData({
          abi: erc20Abi,
          functionName: "approve",
          args: [BRANDING_CONTRACT, 0n],
        }).slice(0, 10)
    );
  }
  if (getAddress(to) !== BRANDING_CONTRACT) return false;
  if (operation === "mint") {
    return selector === encodeFunctionData({
      abi: brandingAbi,
      functionName: "mintBranding",
      args: [{
        acolyte: BRANDING_CONTRACT,
        minter: BRANDING_CONTRACT,
        controllerAgentId: 0n,
        referrer: BRANDING_CONTRACT,
        initialDeclaredPrice: 1n,
        nonce: 0n,
        deadline: 1n,
      }, "0x00"],
    }).slice(0, 10);
  }
  return selector === encodeFunctionData({
    abi: brandingAbi,
    functionName: "setCustomTrait",
    args: [0n, ACOLYTE_NAME_TRAIT, "x"],
  }).slice(0, 10);
}

const BRANDING_STATUS_NAMES: readonly BrandingStatusName[] = [
  "unminted",
  "active",
  "expired",
  "ineligible",
  "registry_unavailable",
];

function requiredBlockHash(value: Hex | null): Hex {
  if (value === null || !/^0x[0-9a-fA-F]{64}$/u.test(value)) {
    throw new RecoverableSignerError(
      "branding_observation",
      "Base RPC did not return a canonical Branding block hash",
    );
  }
  return value.toLowerCase() as Hex;
}

function lowerAddress(value: string): string {
  return getAddress(value).toLowerCase();
}

export async function verifyCanonicalBrandingDeployment(
  publicClient: PublicClient,
  pinnedObservation?: { number: bigint; hash: Hex },
): Promise<CanonicalBrandingObservation> {
  const block = pinnedObservation === undefined
    ? await publicClient.getBlock()
    : await publicClient.getBlock({ blockNumber: pinnedObservation.number });
  const blockHash = requiredBlockHash(block.hash);
  if (
    pinnedObservation !== undefined &&
    (block.number !== pinnedObservation.number || blockHash !== pinnedObservation.hash.toLowerCase())
  ) {
    throw new RecoverableSignerError(
      "branding_reorg",
      "the signed Branding observation block is no longer canonical",
    );
  }
  await verifyCanonicalDeployment(publicClient, { number: block.number, hash: blockHash });
  const [runtime, uwuCode] = await Promise.all([
    publicClient.getCode({ address: BRANDING_CONTRACT, blockNumber: block.number }),
    publicClient.getCode({ address: CANONICAL_UWU, blockNumber: block.number }),
  ]);
  if (runtime === undefined || runtime === "0x" || keccak256(runtime) !== BRANDING_RUNTIME_CODE_HASH) {
    throw new PermanentSignerError(
      "branding_code",
      "canonical Branding runtime bytecode does not match the pinned Base deployment",
    );
  }
  if (uwuCode === undefined || uwuCode === "0x") {
    throw new PermanentSignerError("uwu_code", "canonical UWU has no deployed bytecode");
  }
  const [chainId, registry, uwu, registryVersion, domainName, domainVersion, decimals] =
    await Promise.all([
      publicClient.readContract({
        address: BRANDING_CONTRACT,
        abi: brandingAbi,
        functionName: "BASE_CHAIN_ID",
        blockNumber: block.number,
      }),
      publicClient.readContract({
        address: BRANDING_CONTRACT,
        abi: brandingAbi,
        functionName: "IDENTITY_REGISTRY",
        blockNumber: block.number,
      }),
      publicClient.readContract({
        address: BRANDING_CONTRACT,
        abi: brandingAbi,
        functionName: "UWU",
        blockNumber: block.number,
      }),
      publicClient.readContract({
        address: BRANDING_CONTRACT,
        abi: brandingAbi,
        functionName: "REGISTRY_VERSION",
        blockNumber: block.number,
      }),
      publicClient.readContract({
        address: BRANDING_CONTRACT,
        abi: brandingAbi,
        functionName: "DOMAIN_NAME",
        blockNumber: block.number,
      }),
      publicClient.readContract({
        address: BRANDING_CONTRACT,
        abi: brandingAbi,
        functionName: "DOMAIN_VERSION",
        blockNumber: block.number,
      }),
      publicClient.readContract({
        address: CANONICAL_UWU,
        abi: erc20Abi,
        functionName: "decimals",
        blockNumber: block.number,
      }),
    ]);
  if (
    chainId !== BigInt(ERC8004_CHAIN_ID) ||
    getAddress(registry) !== ERC8004_IDENTITY_REGISTRY ||
    getAddress(uwu) !== CANONICAL_UWU ||
    registryVersion !== BRANDING_REGISTRY_VERSION ||
    domainName !== BRANDING_DOMAIN_NAME ||
    domainVersion !== BRANDING_DOMAIN_VERSION ||
    decimals !== UWU_DECIMALS
  ) {
    throw new PermanentSignerError(
      "branding_dependencies",
      "canonical Branding dependencies, domain, or version do not match the pinned deployment",
    );
  }
  const canonical = await publicClient.getBlock({ blockNumber: block.number });
  if (requiredBlockHash(canonical.hash) !== blockHash) {
    throw new RecoverableSignerError(
      "branding_reorg",
      "the Branding deployment observation changed while it was verified",
    );
  }
  return {
    blockNumber: block.number,
    blockHash,
    blockTimestamp: block.timestamp,
  };
}

async function verifyBrandingController(
  publicClient: PublicClient,
  minter: Address,
  agentId: string,
  blockNumber: bigint,
): Promise<void> {
  const id = BigInt(agentId);
  const [agentWallet, authorized, allegiance, protocol] = await Promise.all([
    publicClient.readContract({
      address: ERC8004_IDENTITY_REGISTRY,
      abi: identityAbi,
      functionName: "getAgentWallet",
      args: [id],
      blockNumber,
    }),
    publicClient.readContract({
      address: ERC8004_IDENTITY_REGISTRY,
      abi: identityAbi,
      functionName: "isAuthorizedOrOwner",
      args: [minter, id],
      blockNumber,
    }),
    publicClient.readContract({
      address: ERC8004_IDENTITY_REGISTRY,
      abi: identityAbi,
      functionName: "getMetadata",
      args: [id, ALLEGIANCE_KEY],
      blockNumber,
    }),
    publicClient.readContract({
      address: ERC8004_IDENTITY_REGISTRY,
      abi: identityAbi,
      functionName: "getMetadata",
      args: [id, PROTOCOL_KEY],
      blockNumber,
    }),
  ]);
  if (
    getAddress(agentWallet) !== minter ||
    !authorized ||
    allegiance !== stringToHex(ALLEGIANCE_VALUE) ||
    protocol !== stringToHex(PROTOCOL_VALUE)
  ) {
    throw new PermanentSignerError(
      "branding_ineligible",
      "local production signer is not the exact eligible controller for this Branding",
    );
  }
}

async function readAcolyteNameTrait(
  publicClient: PublicClient,
  tokenId: bigint,
  blockNumber: bigint,
): Promise<string | null> {
  const count = await publicClient.readContract({
    address: BRANDING_CONTRACT,
    abi: brandingAbi,
    functionName: "customTraitCount",
    args: [tokenId],
    blockNumber,
  });
  if (count > 32n) {
    throw new PermanentSignerError(
      "branding_metadata",
      "Branding contains more custom traits than the canonical contract permits",
    );
  }
  for (let index = 0n; index < count; index += 1n) {
    const [traitType, value] = await publicClient.readContract({
      address: BRANDING_CONTRACT,
      abi: brandingAbi,
      functionName: "customTraitAt",
      args: [tokenId, index],
      blockNumber,
    });
    if (
      Buffer.byteLength(traitType, "utf8") > 64 ||
      Buffer.byteLength(value, "utf8") > 256
    ) {
      throw new PermanentSignerError(
        "branding_metadata",
        "Branding custom trait exceeds the canonical metadata bounds",
      );
    }
    if (traitType === ACOLYTE_NAME_TRAIT) return value;
  }
  return null;
}

async function readBrandingSnapshot(
  publicClient: PublicClient,
  operation: BrandingOperation,
  minter: Address,
): Promise<BrandingSnapshot> {
  const observation = await verifyCanonicalBrandingDeployment(publicClient);
  const initialDeclaredPrice = BigInt(operation.initialDeclaredPrice);
  const [rawBranding, currentConsentNonce, firstWeekUpkeep, ethBalance, uwuBalance, allowance] =
    await Promise.all([
      publicClient.readContract({
        address: BRANDING_CONTRACT,
        abi: brandingAbi,
        functionName: "brandingOf",
        args: [operation.acolyte],
        blockNumber: observation.blockNumber,
      }),
      publicClient.readContract({
        address: BRANDING_CONTRACT,
        abi: brandingAbi,
        functionName: "nonces",
        args: [operation.acolyte],
        blockNumber: observation.blockNumber,
      }),
      publicClient.readContract({
        address: BRANDING_CONTRACT,
        abi: brandingAbi,
        functionName: "weeklyUpkeepForPrice",
        args: [initialDeclaredPrice],
        blockNumber: observation.blockNumber,
      }),
      publicClient.getBalance({ address: minter, blockNumber: observation.blockNumber }),
      publicClient.readContract({
        address: CANONICAL_UWU,
        abi: erc20Abi,
        functionName: "balanceOf",
        args: [minter],
        blockNumber: observation.blockNumber,
      }),
      publicClient.readContract({
        address: CANONICAL_UWU,
        abi: erc20Abi,
        functionName: "allowance",
        args: [minter, BRANDING_CONTRACT],
        blockNumber: observation.blockNumber,
      }),
    ]);
  const branding = rawBranding as BrandingView;
  const tokenId = BigInt(operation.acolyte);
  const brandingStatus = BRANDING_STATUS_NAMES[branding.status];
  if (
    brandingStatus === undefined ||
    branding.tokenId !== tokenId ||
    getAddress(branding.acolyte) !== operation.acolyte
  ) {
    throw new PermanentSignerError(
      "branding_state",
      "canonical Branding returned an inconsistent subject or status",
    );
  }
  if (brandingStatus === "unminted") {
    // Inspection reports the fresh canonical balance so Rust can durably supersede an invitation
    // whose unsigned quote went stale. Once a consent is signed, completion stays byte-exact and
    // refuses any treasury drift before minting.
    if (
      operation.type === "complete_branding" &&
      uwuBalance.toString() !== operation.treasuryBalance
    ) {
      throw new RecoverableSignerError(
        "branding_treasury_changed",
        "the production minter UWU balance changed after the exact Branding quote was created",
      );
    }
    const expectedFirstWeekUpkeep = (initialDeclaredPrice * 10n + 9_999n) / 10_000n;
    if (firstWeekUpkeep !== expectedFirstWeekUpkeep) {
      throw new PermanentSignerError(
        "branding_upkeep",
        "canonical Branding returned an upkeep amount inconsistent with its exact upward-rounding rule",
      );
    }
    if (
      getAddress(branding.owner) !== ZERO_ADDRESS ||
      branding.controllerAgentId !== 0n ||
      getAddress(branding.referrer) !== ZERO_ADDRESS ||
      branding.declaredPrice !== 0n ||
      branding.paidThrough !== 0n
    ) {
      throw new PermanentSignerError(
        "branding_state",
        "unminted Branding returned nonzero lifecycle state",
      );
    }
    await verifyBrandingController(
      publicClient,
      minter,
      operation.controllerAgentId,
      observation.blockNumber,
    );
  } else {
    if (
      getAddress(branding.owner) !== minter ||
      branding.controllerAgentId.toString() !== operation.controllerAgentId ||
      getAddress(branding.referrer) !== operation.referrer
    ) {
      throw new PermanentSignerError(
        "branding_conflict",
        "existing Branding owner, controller, or immutable referrer conflicts with this exact workflow",
      );
    }
    if (
      operation.type === "complete_branding" &&
      currentConsentNonce !== BigInt(operation.nonce) + 1n
    ) {
      throw new PermanentSignerError(
        "branding_nonce",
        "existing Branding did not consume this exact mint consent nonce",
      );
    }
  }
  const nameTrait = brandingStatus === "unminted"
    ? null
    : await readAcolyteNameTrait(publicClient, tokenId, observation.blockNumber);
  const canonical = await publicClient.getBlock({ blockNumber: observation.blockNumber });
  if (requiredBlockHash(canonical.hash) !== observation.blockHash) {
    throw new RecoverableSignerError(
      "branding_reorg",
      "Branding state changed canonical blocks while it was read",
    );
  }
  return {
    observation,
    minter,
    tokenId,
    branding,
    brandingStatus,
    currentConsentNonce,
    firstWeekUpkeep,
    ethBalance,
    uwuBalance,
    allowance,
    nameTrait,
  };
}

function mintConsentFor(
  operation: CompleteBrandingOperation,
): {
  acolyte: Address;
  minter: Address;
  controllerAgentId: bigint;
  referrer: Address;
  initialDeclaredPrice: bigint;
  nonce: bigint;
  deadline: bigint;
} {
  return {
    acolyte: operation.acolyte,
    minter: operation.minter,
    controllerAgentId: BigInt(operation.controllerAgentId),
    referrer: operation.referrer,
    initialDeclaredPrice: BigInt(operation.initialDeclaredPrice),
    nonce: BigInt(operation.nonce),
    deadline: BigInt(operation.deadline),
  };
}

export function brandingConsentDigest(operation: CompleteBrandingOperation): Hex {
  return hashTypedData({
    domain: {
      name: BRANDING_DOMAIN_NAME,
      version: BRANDING_DOMAIN_VERSION,
      chainId: ERC8004_CHAIN_ID,
      verifyingContract: BRANDING_CONTRACT,
    },
    types: {
      MintConsent: [
        { name: "acolyte", type: "address" },
        { name: "minter", type: "address" },
        { name: "controllerAgentId", type: "uint256" },
        { name: "referrer", type: "address" },
        { name: "initialDeclaredPrice", type: "uint256" },
        { name: "nonce", type: "uint256" },
        { name: "deadline", type: "uint256" },
      ],
    },
    primaryType: "MintConsent",
    message: mintConsentFor(operation),
  });
}

async function verifyMintConsent(
  publicClient: PublicClient,
  operation: CompleteBrandingOperation,
  snapshot: BrandingSnapshot,
): Promise<void> {
  if (snapshot.brandingStatus !== "unminted") return;
  if (snapshot.currentConsentNonce.toString() !== operation.nonce) {
    throw new PermanentSignerError(
      "branding_nonce",
      "mint consent nonce no longer matches canonical Branding state",
    );
  }
  const deadline = BigInt(operation.deadline);
  if (deadline <= snapshot.observation.blockTimestamp) {
    throw new PermanentSignerError(
      "branding_deadline",
      "mint consent expired at the fresh pre-mint block",
    );
  }
  if (deadline < snapshot.observation.blockTimestamp + 120n) {
    // A previously submitted exact mint may still confirm before the signed deadline. Keep the
    // durable consent retryable until the chain either proves the mint or makes inclusion
    // impossible; only the expired branch above may discard that authority.
    throw new RecoverableSignerError(
      "branding_deadline_close",
      "mint consent has less than 120 seconds remaining at the fresh pre-mint block",
    );
  }
  const digest = brandingConsentDigest(operation);
  const contractDigest = await publicClient.readContract({
    address: BRANDING_CONTRACT,
    abi: brandingAbi,
    functionName: "consentDigest",
    args: [mintConsentFor(operation)],
    blockNumber: snapshot.observation.blockNumber,
  });
  if (contractDigest.toLowerCase() !== digest.toLowerCase()) {
    throw new PermanentSignerError(
      "branding_digest",
      "local and canonical contract EIP-712 consent digests disagree",
    );
  }
  const acolyteCode = await publicClient.getCode({
    address: operation.acolyte,
    blockNumber: snapshot.observation.blockNumber,
  });
  let valid = false;
  if (acolyteCode === undefined || acolyteCode === "0x") {
    valid = await verifyTypedData({
      address: operation.acolyte,
      domain: {
        name: BRANDING_DOMAIN_NAME,
        version: BRANDING_DOMAIN_VERSION,
        chainId: ERC8004_CHAIN_ID,
        verifyingContract: BRANDING_CONTRACT,
      },
      types: {
        MintConsent: [
          { name: "acolyte", type: "address" },
          { name: "minter", type: "address" },
          { name: "controllerAgentId", type: "uint256" },
          { name: "referrer", type: "address" },
          { name: "initialDeclaredPrice", type: "uint256" },
          { name: "nonce", type: "uint256" },
          { name: "deadline", type: "uint256" },
        ],
      },
      primaryType: "MintConsent",
      message: mintConsentFor(operation),
      signature: operation.signature,
    }).catch(() => false);
  } else {
    const magic = await publicClient.readContract({
      address: operation.acolyte,
      abi: erc1271Abi,
      functionName: "isValidSignature",
      args: [digest, operation.signature],
      blockNumber: snapshot.observation.blockNumber,
    }).catch(() => "0x" as Hex);
    valid = magic.toLowerCase() === ERC1271_MAGIC_VALUE;
  }
  if (!valid) {
    throw new PermanentSignerError(
      "branding_signature",
      "mint consent signature is not valid for the exact acolyte EOA or ERC-1271 wallet",
    );
  }
  const canonical = await publicClient.getBlock({ blockNumber: snapshot.observation.blockNumber });
  if (requiredBlockHash(canonical.hash) !== snapshot.observation.blockHash) {
    throw new RecoverableSignerError(
      "branding_reorg",
      "the mint-consent verification block changed before execution",
    );
  }
}

function brandingPhases(
  operation: BrandingOperation,
  snapshot: BrandingSnapshot,
): PreparedBrandingPhase[] {
  if (snapshot.brandingStatus !== "unminted") {
    return snapshot.nameTrait === operation.acolyteName
      ? []
      : [{
          operation: "name_trait",
          to: BRANDING_CONTRACT,
          data: encodeFunctionData({
            abi: brandingAbi,
            functionName: "setCustomTrait",
            args: [snapshot.tokenId, ACOLYTE_NAME_TRAIT, operation.acolyteName],
          }),
        }];
  }
  const phases: PreparedBrandingPhase[] = [];
  if (snapshot.allowance < snapshot.firstWeekUpkeep) {
    phases.push({
      operation: "approve",
      to: CANONICAL_UWU,
      data: encodeFunctionData({
        abi: erc20Abi,
        functionName: "approve",
        args: [BRANDING_CONTRACT, snapshot.firstWeekUpkeep],
      }),
    });
  }
  const complete = operation.type === "complete_branding" ? operation : undefined;
  const placeholderSignature = `0x${"00".repeat(65)}` as Hex;
  const consent = complete === undefined
    ? {
        acolyte: operation.acolyte,
        minter: snapshot.minter,
        controllerAgentId: BigInt(operation.controllerAgentId),
        referrer: operation.referrer,
        initialDeclaredPrice: BigInt(operation.initialDeclaredPrice),
        nonce: snapshot.currentConsentNonce,
        deadline: snapshot.observation.blockTimestamp + 600n,
      }
    : mintConsentFor(complete);
  phases.push({
    operation: "mint",
    to: BRANDING_CONTRACT,
    data: encodeFunctionData({
      abi: brandingAbi,
      functionName: "mintBranding",
      args: [consent, complete?.signature ?? placeholderSignature],
    }),
  });
  phases.push({
    operation: "name_trait",
    to: BRANDING_CONTRACT,
    data: encodeFunctionData({
      abi: brandingAbi,
      functionName: "setCustomTrait",
      args: [snapshot.tokenId, ACOLYTE_NAME_TRAIT, operation.acolyteName],
    }),
  });
  return phases;
}

async function brandingFunding(
  publicClient: PublicClient,
  snapshot: BrandingSnapshot,
  phases: readonly PreparedBrandingPhase[],
): Promise<BrandingFunding> {
  const safetyBps = envBigInt("CTHUWU_ERC8004_GAS_SAFETY_BPS", DEFAULT_SAFETY_BPS);
  if (safetyBps < 10_000n || safetyBps > 50_000n) {
    throw new PermanentSignerError(
      "configuration",
      "gas safety factor must be between 10000 and 50000 basis points",
    );
  }
  const reserve = envBigInt(
    "CTHUWU_ERC8004_POST_REGISTRATION_RESERVE_WEI",
    DEFAULT_RESERVE_WEI,
  );
  if (phases.length === 0) {
    return {
      ethTarget: 0n,
      ethShortfall: 0n,
      uwuTarget: 0n,
      uwuShortfall: 0n,
      estimatedCost: 0n,
      executionGas: 0n,
      l1DataFee: 0n,
      l1DataFeeExact: true,
      maxFeePerGas: 0n,
      maxPriorityFeePerGas: 0n,
      safetyBps,
      reserve,
      exactOperations: [],
      conservativeOperations: [],
    };
  }
  const { maxFeePerGas, maxPriorityFeePerGas } = await feeParameters(publicClient);
  const gasCeiling = envBigInt(
    "CTHUWU_ERC8004_MAX_GAS_PER_TRANSACTION",
    DEFAULT_MAX_GAS_PER_TRANSACTION,
  );
  let executionGas = 0n;
  const exactOperations: BrandingTransactionPhase[] = [];
  const conservativeOperations: BrandingTransactionPhase[] = [];
  for (const phase of phases) {
    let gas = gasCeiling;
    try {
      gas = await publicClient.estimateGas({
        account: snapshot.minter,
        to: phase.to,
        data: phase.data,
        value: 0n,
      });
      exactOperations.push(phase.operation);
    } catch {
      // A later phase may be impossible to estimate until the prior exact transaction confirms.
      // Keep the strict configured ceiling instead of understating the funding requirement.
      conservativeOperations.push(phase.operation);
    }
    if (gas > gasCeiling) {
      throw new RecoverableSignerError(
        "gas_ceiling",
        "estimated Branding gas exceeds the configured per-transaction ceiling",
      );
    }
    executionGas += gas;
  }
  const conservativeL1FeePerTransaction = gasCeiling * maxFeePerGas;
  const l1Estimate = await sumL1FeesWithConservativeFallback(
    phases,
    (phase) =>
      publicClient.estimateL1Fee({
        account: snapshot.minter,
        to: phase.to,
        data: phase.data,
      }),
    conservativeL1FeePerTransaction,
  );
  const estimatedCost = executionGas * maxFeePerGas + l1Estimate.fee;
  const ethTarget = (estimatedCost * safetyBps + 9_999n) / 10_000n + reserve;
  const uwuTarget = phases.some((phase) => phase.operation === "mint")
    ? snapshot.firstWeekUpkeep
    : 0n;
  return {
    ethTarget,
    ethShortfall: ethTarget > snapshot.ethBalance ? ethTarget - snapshot.ethBalance : 0n,
    uwuTarget,
    uwuShortfall: uwuTarget > snapshot.uwuBalance ? uwuTarget - snapshot.uwuBalance : 0n,
    estimatedCost,
    executionGas,
    l1DataFee: l1Estimate.fee,
    l1DataFeeExact: l1Estimate.exact,
    maxFeePerGas,
    maxPriorityFeePerGas,
    safetyBps,
    reserve,
    exactOperations,
    conservativeOperations,
  };
}

function brandingResult(
  kind: "branding_inspection" | "branding_completion",
  operation: BrandingOperation,
  snapshot: BrandingSnapshot,
  phases: readonly PreparedBrandingPhase[],
  funding: BrandingFunding,
  transactionHashes: readonly { operation: BrandingTransactionPhase; transactionHash: Hex }[],
): Record<string, unknown> {
  const funded = funding.ethShortfall === 0n && funding.uwuShortfall === 0n;
  const disposition = phases.length === 0
    ? "complete"
    : !funded
      ? "funding_required"
      : snapshot.brandingStatus === "unminted"
        ? "ready"
        : "repair_required";
  return {
    kind,
    disposition,
    chainId: ERC8004_CHAIN_ID,
    contract: lowerAddress(BRANDING_CONTRACT),
    runtimeCodeHash: BRANDING_RUNTIME_CODE_HASH,
    identityRegistry: lowerAddress(ERC8004_IDENTITY_REGISTRY),
    uwu: lowerAddress(CANONICAL_UWU),
    observedBlockNumber: snapshot.observation.blockNumber.toString(),
    observedBlockHash: snapshot.observation.blockHash.toLowerCase(),
    observedBlockTimestamp: snapshot.observation.blockTimestamp.toString(),
    minter: lowerAddress(snapshot.minter),
    acolyte: lowerAddress(operation.acolyte),
    tokenId: snapshot.tokenId.toString(),
    controllerAgentId: operation.controllerAgentId,
    referrer: lowerAddress(operation.referrer),
    initialDeclaredPrice: operation.initialDeclaredPrice,
    firstWeekUpkeep: snapshot.firstWeekUpkeep.toString(),
    acolyteName: operation.acolyteName,
    consentNonce:
      operation.type === "complete_branding"
        ? operation.nonce
        : snapshot.currentConsentNonce.toString(),
    ...(operation.type === "complete_branding"
      ? { currentConsentNonce: snapshot.currentConsentNonce.toString() }
      : {}),
    brandingStatus: snapshot.brandingStatus,
    owner: lowerAddress(snapshot.branding.owner),
    onchainControllerAgentId: snapshot.branding.controllerAgentId.toString(),
    onchainReferrer: lowerAddress(snapshot.branding.referrer),
    onchainDeclaredPrice: snapshot.branding.declaredPrice.toString(),
    paidThrough: snapshot.branding.paidThrough.toString(),
    nameTrait: snapshot.nameTrait,
    ethBalanceWei: snapshot.ethBalance.toString(),
    ethTargetWei: funding.ethTarget.toString(),
    ethShortfallWei: funding.ethShortfall.toString(),
    uwuBalance: snapshot.uwuBalance.toString(),
    uwuTarget: funding.uwuTarget.toString(),
    uwuShortfallWei: funding.uwuShortfall.toString(),
    allowance: snapshot.allowance.toString(),
    estimatedCostWei: funding.estimatedCost.toString(),
    executionGas: funding.executionGas.toString(),
    l1DataFeeWei: funding.l1DataFee.toString(),
    l1DataFeeExact: funding.l1DataFeeExact,
    maxFeePerGasWei: funding.maxFeePerGas.toString(),
    maxPriorityFeePerGasWei: funding.maxPriorityFeePerGas.toString(),
    safetyBps: funding.safetyBps.toString(),
    reserveWei: funding.reserve.toString(),
    exactOperations: funding.exactOperations,
    conservativeOperations: funding.conservativeOperations,
    pendingOperations: phases.map((phase) => phase.operation),
    ...(kind === "branding_completion" ? { transactionHashes } : {}),
  };
}

async function inspectBranding(
  publicClient: PublicClient,
  operation: BrandingInspectOperation,
  identity: LoadedIdentity,
): Promise<Record<string, unknown>> {
  const minter = assertProductionIdentity(identity);
  const snapshot = await readBrandingSnapshot(publicClient, operation, minter);
  const phases = brandingPhases(operation, snapshot);
  const funding = await brandingFunding(publicClient, snapshot, phases);
  return brandingResult(
    "branding_inspection",
    operation,
    snapshot,
    phases,
    funding,
    [],
  );
}

async function sendBrandingPhase(
  publicClient: PublicClient,
  identity: LoadedIdentity,
  actionId: string,
  phase: PreparedBrandingPhase,
  rpcEndpoint: string,
): Promise<Hex> {
  if (!isAllowedBrandingTransaction(phase.operation, phase.to, phase.data)) {
    throw new PermanentSignerError(
      "branding_calldata",
      "prepared Branding phase is outside the closed approve, mint, and name-trait allowlist",
    );
  }
  const account = privateKeyToAccount(identity.walletKey);
  const fees = await feeParameters(publicClient);
  const gasCeiling = envBigInt(
    "CTHUWU_ERC8004_MAX_GAS_PER_TRANSACTION",
    DEFAULT_MAX_GAS_PER_TRANSACTION,
  );
  const gas = await publicClient.estimateGas({
    account,
    to: phase.to,
    data: phase.data,
    value: 0n,
  });
  if (gas > gasCeiling) {
    throw new RecoverableSignerError(
      "gas_ceiling",
      "estimated Branding gas exceeds the configured per-transaction ceiling",
    );
  }
  const [pendingNonce, latestNonce] = await Promise.all([
    publicClient.getTransactionCount({ address: account.address, blockTag: "pending" }),
    publicClient.getTransactionCount({ address: account.address, blockTag: "latest" }),
  ]);
  const allocation = await authorizeBrandingSignerNonce(
    identity,
    actionId,
    phase.operation,
    phase.to,
    phase.data,
    pendingNonce,
    latestNonce,
  );
  const walletClient = createWalletClient({
    account,
    chain: base,
    transport: http(rpcEndpoint, { timeout: 20_000, retryCount: 0 }),
  });
  const hash = await walletClient.sendTransaction({
    account,
    chain: base,
    to: phase.to,
    data: phase.data,
    value: 0n,
    gas,
    ...fees,
    nonce: allocation.nonce,
  });
  let receipt;
  try {
    receipt = await publicClient.waitForTransactionReceipt({
      hash,
      confirmations: 1,
      timeout: 30_000,
    });
  } catch {
    throw new RecoverableSignerError(
      "branding_transaction_pending",
      `exact ${phase.operation} transaction remains pending and will be recovered from its durable signer allocation`,
    );
  }
  if (receipt.status !== "success") {
    throw new PermanentSignerError(
      "branding_transaction_reverted",
      `canonical ${phase.operation} transaction reverted`,
    );
  }
  return hash;
}

async function completeBranding(
  publicClient: PublicClient,
  operation: CompleteBrandingOperation,
  identity: LoadedIdentity,
  actionId: string,
  rpcEndpoint: string,
): Promise<Record<string, unknown>> {
  const minter = assertProductionIdentity(identity, operation.minter);
  let snapshot = await readBrandingSnapshot(publicClient, operation, minter);
  if (snapshot.brandingStatus === "unminted") {
    // The historical offer anchor is a precondition for exercising mint authority. Once fresh
    // state proves this exact nonce was consumed by the expected owner/controller/referrer, a
    // delayed owner-only name repair must not depend on archival access to that old block.
    const offerBlock = await publicClient.getBlock({
      blockNumber: BigInt(operation.offerBlockNumber),
    });
    if (requiredBlockHash(offerBlock.hash) !== operation.offerBlockHash.toLowerCase()) {
      throw new RecoverableSignerError(
        "branding_offer_reorg",
        "the signed Branding offer block is no longer canonical",
      );
    }
    await verifyMintConsent(publicClient, operation, snapshot);
  }
  let phases = brandingPhases(operation, snapshot);
  let funding = await brandingFunding(publicClient, snapshot, phases);
  if (funding.ethShortfall !== 0n || funding.uwuShortfall !== 0n) {
    return brandingResult(
      "branding_completion",
      operation,
      snapshot,
      phases,
      funding,
      [],
    );
  }
  const transactionHashes: Array<{
    operation: BrandingTransactionPhase;
    transactionHash: Hex;
  }> = [];

  for (;;) {
    const next = phases[0];
    if (next === undefined) break;
    // Every phase is reconstructed from freshly verified canonical state. A confirmed approve,
    // mint, or trait write is skipped after a crash; an exact pending phase is safely replaceable.
    const hash = await sendBrandingPhase(
      publicClient,
      identity,
      actionId,
      next,
      rpcEndpoint,
    );
    transactionHashes.push({ operation: next.operation, transactionHash: hash });
    snapshot = await readBrandingSnapshot(publicClient, operation, minter);
    const phaseAdvanced =
      (next.operation === "approve" && snapshot.allowance >= snapshot.firstWeekUpkeep) ||
      (next.operation === "mint" &&
        snapshot.brandingStatus !== "unminted" &&
        snapshot.currentConsentNonce === BigInt(operation.nonce) + 1n) ||
      (next.operation === "name_trait" && snapshot.nameTrait === operation.acolyteName);
    if (!phaseAdvanced) {
      throw new RecoverableSignerError(
        "branding_no_effect",
        `confirmed ${next.operation} transaction did not produce its exact expected canonical state transition`,
      );
    }
    if (snapshot.brandingStatus === "unminted") {
      await verifyMintConsent(publicClient, operation, snapshot);
    }
    phases = brandingPhases(operation, snapshot);
    funding = await brandingFunding(publicClient, snapshot, phases);
    if (funding.ethShortfall !== 0n || funding.uwuShortfall !== 0n) {
      return brandingResult(
        "branding_completion",
        operation,
        snapshot,
        phases,
        funding,
        transactionHashes,
      );
    }
  }

  if (
    snapshot.brandingStatus === "unminted" ||
    snapshot.nameTrait !== operation.acolyteName ||
    snapshot.currentConsentNonce !== BigInt(operation.nonce) + 1n ||
    getAddress(snapshot.branding.owner) !== minter ||
    snapshot.branding.controllerAgentId.toString() !== operation.controllerAgentId ||
    getAddress(snapshot.branding.referrer) !== operation.referrer
  ) {
    throw new RecoverableSignerError(
      "branding_incomplete",
      "Branding completion could not be verified from fresh canonical state",
    );
  }
  funding = await brandingFunding(publicClient, snapshot, []);
  return brandingResult(
    "branding_completion",
    operation,
    snapshot,
    [],
    funding,
    transactionHashes,
  );
}

export async function handleErc8004Request(
  request: Erc8004Request,
  rpcEndpoint: string,
  loadIdentity: () => Promise<LoadedIdentity>,
): Promise<Erc8004Response> {
  try {
    const publicClient = createRegistryPublicClient(rpcEndpoint);
    const operation = request.operation;
    let result: unknown;
    switch (operation.type) {
      case "inspect_registry":
        result = await verifyCanonicalDeployment(publicClient);
        break;
      case "resolve_inbox": {
        await verifyCanonicalDeployment(publicClient);
        const persistentWallet = assertProductionIdentity(
          await loadIdentity(),
          operation.wallet,
        );
        const identity = await resolveOperatorIdentity(operation.wallet, "production");
        result = {
          wallet: persistentWallet,
          inboxId: identity.inboxId,
          endpoint: `xmtp://${identity.inboxId}`,
          environment: "production",
        };
        break;
      }
      case "transaction_nonce": {
        await verifyCanonicalDeployment(publicClient);
        const wallet = assertProductionIdentity(await loadIdentity(), operation.wallet);
        result = await readSignerNonceState(
          publicClient,
          wallet,
          operation.observedBlockNumber === undefined ||
            operation.observedBlockHash === undefined
            ? undefined
            : {
                observedBlockNumber: operation.observedBlockNumber,
                observedBlockHash: operation.observedBlockHash,
              },
        );
        break;
      }
      case "inspect_agent":
        await verifyCanonicalDeployment(publicClient);
        result = await inspectAgent(publicClient, operation.agentId, getAddress(operation.wallet));
        break;
      case "discover": {
        await verifyCanonicalDeployment(publicClient);
        const context = operation.tentacleId === undefined || operation.xmtpInboxId === undefined
          ? undefined
          : { tentacleId: operation.tentacleId, xmtpInboxId: operation.xmtpInboxId };
        let identity: LoadedIdentity | undefined;
        let checkpoint: DiscoveryJournal | undefined;
        if (context !== undefined || operation.checkpoint !== undefined) {
          identity = await loadIdentity();
          assertProductionIdentity(identity, operation.wallet);
        }
        if (operation.checkpoint !== undefined) {
          if (identity === undefined || context === undefined) {
            throw new PermanentSignerError(
              "discovery_checkpoint",
              "checkpoint reuse requires exact local Tentacle identity evidence",
            );
          }
          checkpoint = await readDiscoveryJournal(identity);
          if (
            checkpoint.checkpointFingerprint !== operation.checkpoint.fingerprint ||
            checkpoint.wallet !== getAddress(operation.wallet) ||
            checkpoint.tentacleId !== context.tentacleId ||
            checkpoint.xmtpInboxId !== context.xmtpInboxId
          ) {
            throw new RecoverableSignerError(
              "discovery_checkpoint",
              "requested identity-discovery checkpoint does not match local durable provenance",
            );
          }
        }
        const discovered = await discoverAgents(
          publicClient,
          getAddress(operation.wallet),
          operation.registrationNonce,
          operation.scope,
          context,
          checkpoint,
        );
        if (context !== undefined && identity !== undefined && operation.scope === "exhaustive") {
          const journal = buildDiscoveryJournal(
            discovered as DiscoveryResultWithInternal,
            getAddress(operation.wallet),
            context,
          );
          await persistDiscoveryJournal(identity, journal);
          // `discoverAgents` carries its freshly-derived authorization internally. Never
          // publish that field until the durable allocation journal confirms this is either
          // an unallocated proof or the exact persisted action+nonce replay.
          const checkpointResult = {
            version: 1,
            fingerprint: journal.checkpointFingerprint,
            throughBlock: journal.throughBlock,
            throughBlockHash: journal.throughBlockHash,
          } as const;
          const publishableAuthorization = await selectDiscoveryMintAuthorization(
            identity,
            journal.mintAuthorization,
            operation.registrationActionId,
            operation.registrationNonce,
          );
          result = buildPublicDiscoveryResult(
            discovered as Record<string, unknown>,
            checkpointResult,
            publishableAuthorization,
          );
        } else {
          result = discovered;
        }
        break;
      }
      case "receipt":
        await verifyCanonicalDeployment(publicClient);
        result = await inspectReceipt(publicClient, operation.transactionHash as Hex);
        break;
      case "funding_estimate":
        await verifyCanonicalDeployment(publicClient);
        result = await fundingEstimate(publicClient, operation);
        break;
      case "branding_inspect":
        result = await inspectBranding(
          publicClient,
          operation,
          await loadIdentity(),
        );
        break;
      case "complete_branding":
        result = await completeBranding(
          publicClient,
          operation,
          await loadIdentity(),
          request.actionId,
          rpcEndpoint,
        );
        break;
      case "register":
      case "set_agent_uri":
      case "set_metadata":
      case "set_agent_wallet":
        await verifyCanonicalDeployment(publicClient);
        result = await executeWrite(
          publicClient,
          await loadIdentity(),
          request.actionId,
          operation,
          rpcEndpoint,
        );
        break;
    }
    return { version: 1, actionId: request.actionId, ok: true, result };
  } catch (error) {
    const permanent = error instanceof PermanentSignerError;
    const recoverable = error instanceof RecoverableSignerError || !permanent;
    const code = error instanceof PermanentSignerError || error instanceof RecoverableSignerError ? error.code : "rpc_or_signing_failure";
    const rawMessage = error instanceof Error ? error.message : "ERC-8004 request failed";
    const message = rawMessage
      .replace(/https?:\/\/\S+/gu, "<redacted-rpc>")
      .replace(/0x[0-9a-fA-F]{80,}/gu, "<redacted-bytes>")
      .replace(/[\r\n]+/gu, " ")
      .slice(0, 512);
    return { version: 1, actionId: request.actionId, ok: false, recoverable, code, message };
  }
}

export async function runErc8004Stdio(
  input: NodeJS.ReadableStream,
  output: NodeJS.WritableStream,
  loadIdentity: () => Promise<LoadedIdentity>,
): Promise<void> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of input) {
    const bytes = Buffer.isBuffer(chunk)
      ? chunk
      : Buffer.from(typeof chunk === "string" ? chunk : (chunk as Uint8Array));
    size += bytes.length;
    if (size > MAX_FRAME_BYTES) throw new PermanentSignerError("frame_size", "ERC-8004 request frame is oversized");
    chunks.push(bytes);
  }
  const raw = Buffer.concat(chunks).toString("utf8");
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    throw new PermanentSignerError("invalid_json", "ERC-8004 request is not valid JSON");
  }
  const request = parseErc8004Request(value);
  const rpcEndpoint = process.env.CTHUWU_RPC_ENDPOINT;
  if (rpcEndpoint === undefined || rpcEndpoint.length === 0) throw new PermanentSignerError("configuration", "CTHUWU_RPC_ENDPOINT is required");
  const response = await handleErc8004Request(request, rpcEndpoint, loadIdentity);
  const encoded = `${JSON.stringify(response, (_key, item: unknown) => typeof item === "bigint" ? item.toString() : item)}\n`;
  if (Buffer.byteLength(encoded, "utf8") > MAX_FRAME_BYTES) throw new PermanentSignerError("frame_size", "ERC-8004 response frame is oversized");
  output.write(encoded);
}

export function requestFingerprint(request: Erc8004Request): string {
  return createHash("sha256").update(JSON.stringify(request)).digest("hex");
}

export function isAllowedCalldata(data: string): boolean {
  if (!isHex(data) || data.length < 10) return false;
  const allowed = [
    encodeFunctionData({ abi: identityAbi, functionName: "register" }).slice(0, 10),
    encodeFunctionData({ abi: identityAbi, functionName: "setAgentURI", args: [0n, "x"] }).slice(0, 10),
    encodeFunctionData({ abi: identityAbi, functionName: "setMetadata", args: [0n, ALLEGIANCE_KEY, "0x"] }).slice(0, 10),
    encodeFunctionData({ abi: identityAbi, functionName: "setAgentWallet", args: [0n, ERC8004_IDENTITY_REGISTRY, 0n, "0x"] }).slice(0, 10),
  ];
  return allowed.includes(data.slice(0, 10));
}
