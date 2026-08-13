import { createHash, randomUUID } from "node:crypto";
import { chmod, link, lstat, open, readFile, unlink } from "node:fs/promises";
import path from "node:path";
import {
  createPublicClient,
  createWalletClient,
  decodeEventLog,
  encodeFunctionData,
  getAddress,
  hexToString,
  http,
  isAddress,
  isHex,
  keccak256,
  pad,
  parseAbi,
  stringToHex,
  toHex,
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

const identityAbi = parseAbi([
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
  | { type: "discover"; wallet: string; registrationNonce?: string; scope: "recent" | "exhaustive" }
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

type WriteIntent =
  | { type: "register" }
  | { type: "set_agent_uri"; agentId: string; agentURI: string }
  | { type: "set_metadata"; agentId: string; key: string; value: string }
  | { type: "set_agent_wallet"; agentId: string };

type WriteOperation = WriteIntent & { nonce: string };

export type PreparedErc8004Transaction = {
  chainId: 8453;
  to: Address;
  value: 0n;
  data: Hex;
};

export type Erc8004Request = {
  version: 1;
  actionId: string;
  operation: ReadOperation | WriteOperation;
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
      exactKeys(
        operation,
        operation.registrationNonce === undefined
          ? ["type", "wallet", "scope"]
          : ["type", "wallet", "registrationNonce", "scope"],
      );
      if (operation.scope !== "recent" && operation.scope !== "exhaustive") {
        throw new PermanentSignerError("invalid_request", "discovery scope must be recent or exhaustive");
      }
      return {
        version: 1,
        actionId,
        operation: {
          type: "discover",
          wallet: walletAddress(operation.wallet),
          scope: operation.scope,
          ...(operation.registrationNonce === undefined
            ? {}
            : { registrationNonce: transactionNonce(operation.registrationNonce) }),
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
    case "register":
      exactKeys(operation, ["type", "nonce"]);
      return {
        version: 1,
        actionId,
        operation: { type: "register", nonce: transactionNonce(operation.nonce) },
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

export async function verifyCanonicalDeployment(publicClient: PublicClient): Promise<Record<string, unknown>> {
  const chainId = await publicClient.getChainId();
  if (chainId !== ERC8004_CHAIN_ID) {
    throw new PermanentSignerError("wrong_chain", `RPC reported chain ${chainId}; canonical Base is ${ERC8004_CHAIN_ID}`);
  }
  const observedBlock = await publicClient.getBlock();
  const observedBlockNumber = observedBlock.number;
  const observedBlockHash = observedBlock.hash;
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

export async function inspectAgent(publicClient: PublicClient, agentId: string, wallet: Address): Promise<Record<string, unknown>> {
  const id = BigInt(agentId);
  const observedBlock = await publicClient.getBlock();
  const blockNumber = observedBlock.number;
  const [owner, agentURI, agentWallet, allegiance, protocol, tentacleId, authorized] = await Promise.all([
    publicClient.readContract({ address: ERC8004_IDENTITY_REGISTRY, abi: identityAbi, functionName: "ownerOf", args: [id], blockNumber }),
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
  const canonicalBlock = await publicClient.getBlock({ blockNumber });
  if (canonicalBlock.hash !== observedBlock.hash) {
    throw new RecoverableSignerError(
      "agent_reorg",
      "the agent observation block changed while it was being read",
    );
  }
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
    observedBlockHash: observedBlock.hash,
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

export async function discoverAgents(
  publicClient: PublicClient,
  wallet: Address,
  registrationNonce?: string,
  scope: "recent" | "exhaustive" = "exhaustive",
): Promise<unknown> {
  // Recovery decisions must never compare logs from one RPC head with a nonce from another.
  // Use a canonical finalized observation and echo its number/hash for a later EIP-1898-pinned
  // nonce query. If the provider cannot supply this, discovery fails closed.
  const observedBlock = await publicClient.getBlock({ blockTag: "finalized" });
  if (observedBlock.hash === null || observedBlock.number < ERC8004_START_BLOCK) {
    throw new RecoverableSignerError(
      "discovery_observation",
      "provider did not return a usable finalized ERC-8004 discovery block",
    );
  }
  const observedBlockNumber = observedBlock.number;
  const observedBlockHash = observedBlock.hash;
  const firstBlock = scope === "recent"
    ? observedBlockNumber - ERC8004_START_BLOCK + 1n > RECENT_DISCOVERY_BLOCKS
      ? observedBlockNumber - RECENT_DISCOVERY_BLOCKS + 1n
      : ERC8004_START_BLOCK
    : ERC8004_START_BLOCK;
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
  const ids = new Set<string>();
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

  const operatorOwners = new Set<Address>();
  for (const log of operatorEvents) {
    if (log.args.owner !== undefined) operatorOwners.add(getAddress(log.args.owner));
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
    });
    if (!currentlyApproved) continue;
    const ownerLogs = await getLogsChunked(
      publicClient,
      [DISCOVERY_EVENT_TOPICS.Registered, DISCOVERY_EVENT_TOPICS.Transfer],
      [pad(owner, { size: 32 })],
      (log) =>
        (log.eventName === "Registered" && log.args.owner === owner) ||
        (log.eventName === "Transfer" && log.args.to === owner),
      budget,
      observedBlockNumber,
      firstBlock,
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
    const candidate = await inspectAgent(publicClient, id, wallet);
    if (candidate.authorized === true || candidate.walletVerified === true) candidates.push(candidate);
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
      "the finalized discovery block changed while candidate discovery was running",
    );
  }
  const matchedRegistrationAgentIds = new Set<string>();
  if (registrationNonce !== undefined) {
    if (registered.length > MAX_DISCOVERY_IDS) {
      throw new PermanentSignerError(
        "candidate_limit",
        "wallet has too many historical registrations for exact nonce recovery",
      );
    }
    const requestedNonce = Number(transactionNonce(registrationNonce));
    for (const log of registered) {
      if (log.transactionHash === undefined || log.args.agentId === undefined) {
        throw new RecoverableSignerError(
          "discovery_outcome",
          "provider omitted transaction provenance for a registration event",
        );
      }
      const transaction = await publicClient.getTransaction({
        hash: log.transactionHash,
      });
      if (
        getAddress(transaction.from) === wallet &&
        transaction.nonce === requestedNonce
      ) {
        matchedRegistrationAgentIds.add(log.args.agentId.toString());
      }
    }
    if (matchedRegistrationAgentIds.size > 1) {
      throw new PermanentSignerError(
        "discovery_outcome",
        "one signer nonce unexpectedly produced multiple ERC-8004 identities",
      );
    }
  }
  return {
    complete: true,
    fromBlock: firstBlock.toString(),
    observedBlockNumber: observedBlockNumber.toString(),
    observedBlockHash,
    matchedRegistrationAgentIds: [...matchedRegistrationAgentIds],
    candidates,
  };
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
      } catch (error) {
        if (operation.agentId !== undefined) {
          throw new RecoverableSignerError("gas_estimate", "provider rejected an exact remaining ERC-8004 gas estimate");
        }
      }
    }
    if (gas > gasCeiling) throw new RecoverableSignerError("gas_ceiling", "estimated registry gas exceeds the configured ceiling");
    executionGas += gas;
    l1Calls.push(call.data);
  }
  let l1Fee: bigint;
  try {
    l1Fee = await sumThrottledL1Fees(l1Calls, (data) =>
      publicClient.estimateL1Fee({
        account: wallet,
        to: ERC8004_IDENTITY_REGISTRY,
        data,
      }),
    );
  } catch (error) {
    if (error instanceof PermanentSignerError) throw error;
    throw new RecoverableSignerError("l1_fee_estimate", "provider did not return Base L1 data fee estimates");
  }
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
      case "discover":
        await verifyCanonicalDeployment(publicClient);
        result = await discoverAgents(
          publicClient,
          getAddress(operation.wallet),
          operation.registrationNonce,
          operation.scope,
        );
        break;
      case "receipt":
        await verifyCanonicalDeployment(publicClient);
        result = await inspectReceipt(publicClient, operation.transactionHash as Hex);
        break;
      case "funding_estimate":
        await verifyCanonicalDeployment(publicClient);
        result = await fundingEstimate(publicClient, operation);
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
    const message = rawMessage.replace(/https?:\/\/\S+/gu, "<redacted-rpc>").replace(/[\r\n]+/gu, " ").slice(0, 512);
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
