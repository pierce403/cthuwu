import { Interface, getAddress, keccak256 } from "ethers";
import {
  CANONICAL_BRANDING_CONTRACT,
  CANONICAL_BRANDING_DEPLOYMENT_BLOCK,
  DEFAULT_BASE_RPC_ENDPOINT,
} from "./config";

export const ACOLYTE_CHAIN_ID = 8453 as const;
export const ACOLYTE_BRANDING_CONTRACT = CANONICAL_BRANDING_CONTRACT;
export const ACOLYTE_DEPLOYMENT_BLOCK = CANONICAL_BRANDING_DEPLOYMENT_BLOCK;
export const ACOLYTE_DEPLOYMENT_BLOCK_HASH =
  "0x21ac04bdd198b9e5219741a55bdf6da30aa43f8e7cf47cf0fd82548fcc61cfc7";
export const ACOLYTE_RUNTIME_CODE_HASH =
  "0x3a22b742a570dc2d030edf2bd82dceda1e068a297e2c36883f97b7b66ed4ef2d";

const MINTED_TOPIC = "0x25e13488f52beb232f67a5e9c62c4b5595ed1df52381104bf6463abfe4265b37";
const MAX_BLOCKS_PER_LOG_REQUEST = 10_000n;
const MAX_LOG_REQUESTS = 10_000n;
const MAX_ITEMS = 5_000;
const MAX_TRAITS = 32;
const MAX_RPC_BYTES = 512 * 1024;
const RPC_TIMEOUT_MS = 20_000;
const HASH = /^0x[0-9a-f]{64}$/u;
const DATA = /^0x(?:[0-9a-fA-F]{2})*$/u;
const QUANTITY = /^0x(?:0|[1-9a-fA-F][0-9a-fA-F]*)$/u;

const BRANDING = new Interface([
  "function brandingOf(address acolyte) view returns (tuple(uint256 tokenId,address acolyte,address owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 pendingDeclaredPrice,uint256 pendingPriceActivation,uint8 status) result)",
  "function avatarURIOf(uint256 tokenId) view returns (string)",
  "function customTraitCount(uint256 tokenId) view returns (uint256)",
  "function customTraitAt(uint256 tokenId,uint256 index) view returns (string traitType,string value)",
  "event BrandingMinted(uint256 indexed tokenId,address indexed acolyte,address indexed owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 firstUpkeep)",
]);

export type AcolyteBrandingStatus =
  | "Active"
  | "Expired"
  | "Ineligible"
  | "Registry unavailable";

export interface AcolyteTrait {
  traitType: string;
  value: string;
}

export interface AcolyteCatalogItem {
  tokenId: string;
  acolyte: string;
  owner: string;
  controllerAgentId: string;
  referrer: string;
  declaredPrice: string;
  paidThrough: string;
  pendingDeclaredPrice: string;
  pendingPriceValidAfter: string;
  status: AcolyteBrandingStatus;
  avatarUri: string;
  traits: AcolyteTrait[];
  mintBlockNumber: string;
  mintTransactionHash: string;
}

export interface AcolyteCatalogSnapshot {
  chainId: typeof ACOLYTE_CHAIN_ID;
  contractAddress: string;
  sourceBlockNumber: string;
  sourceBlockHash: string;
  items: AcolyteCatalogItem[];
}

export interface FetchAcolyteCatalogOptions {
  fetch?: typeof fetch;
  signal?: AbortSignal;
  endpoint?: string;
}

interface RpcLog {
  address: string;
  blockNumber: string;
  transactionHash: string;
  topics: string[];
  data: string;
  removed?: boolean;
}

interface MintRecord {
  tokenId: bigint;
  acolyte: string;
  referrer: string;
  blockNumber: bigint;
  transactionHash: string;
}

/** Reads a reorg-checked, finalized snapshot. Owner-selected metadata is returned as hostile text. */
export async function fetchAcolyteCatalog(
  options: FetchAcolyteCatalogOptions = {},
): Promise<AcolyteCatalogSnapshot> {
  const rpc = new BoundedRpc(
    options.endpoint ?? DEFAULT_BASE_RPC_ENDPOINT,
    options.fetch ?? fetch,
    options.signal,
  );
  const chainId = quantity(await rpc.request("eth_chainId", []), "chain ID");
  if (chainId !== BigInt(ACOLYTE_CHAIN_ID)) throw new Error("Base RPC returned the wrong chain");

  const deployment = block(await rpc.request("eth_getBlockByNumber", [hex(ACOLYTE_DEPLOYMENT_BLOCK), false]));
  if (deployment.number !== ACOLYTE_DEPLOYMENT_BLOCK || deployment.hash !== ACOLYTE_DEPLOYMENT_BLOCK_HASH) {
    throw new Error("canonical Branding deployment block does not match");
  }
  const runtimeCode = bytes(await rpc.request("eth_getCode", [ACOLYTE_BRANDING_CONTRACT, "finalized"]), "runtime code");
  if (runtimeCode === "0x" || keccak256(runtimeCode) !== ACOLYTE_RUNTIME_CODE_HASH) {
    throw new Error("canonical Branding runtime code does not match");
  }

  const pinned = block(await rpc.request("eth_getBlockByNumber", ["finalized", false]));
  if (pinned.number < ACOLYTE_DEPLOYMENT_BLOCK) throw new Error("finalized block predates Branding");
  const blockTag = hex(pinned.number);
  const pinnedRuntimeCode = bytes(
    await rpc.request("eth_getCode", [ACOLYTE_BRANDING_CONTRACT, blockTag]),
    "pinned runtime code",
  );
  if (pinnedRuntimeCode !== runtimeCode) {
    throw new Error("canonical Branding runtime changed while pinning the catalog");
  }
  const requiredLogRequests =
    (pinned.number - ACOLYTE_DEPLOYMENT_BLOCK) / MAX_BLOCKS_PER_LOG_REQUEST + 1n;
  if (requiredLogRequests > MAX_LOG_REQUESTS) {
    throw new Error("Branding log scan exceeds its safety limit");
  }
  const mints: MintRecord[] = [];
  const seen = new Set<string>();
  for (let from = ACOLYTE_DEPLOYMENT_BLOCK; from <= pinned.number; from += MAX_BLOCKS_PER_LOG_REQUEST) {
    const to = min(from + MAX_BLOCKS_PER_LOG_REQUEST - 1n, pinned.number);
    const rawLogs = await rpc.request("eth_getLogs", [{
      address: ACOLYTE_BRANDING_CONTRACT,
      topics: [MINTED_TOPIC],
      fromBlock: hex(from),
      toBlock: hex(to),
    }]);
    if (!Array.isArray(rawLogs) || rawLogs.length > MAX_ITEMS) throw new Error("invalid Branding log page");
    for (const raw of rawLogs) {
      const mint = parseMint(raw, from, to, pinned.number);
      const key = mint.tokenId.toString();
      if (seen.has(key)) throw new Error("duplicate Branding mint");
      seen.add(key);
      mints.push(mint);
      if (mints.length > MAX_ITEMS) throw new Error("Branding catalog exceeds its safety limit");
    }
  }

  const items: AcolyteCatalogItem[] = [];
  for (const mint of mints) {
    const current = await call(rpc, "brandingOf", [mint.acolyte], blockTag);
    const value = current[0] as unknown as {
      tokenId: bigint; acolyte: string; owner: string; controllerAgentId: bigint;
      referrer: string; declaredPrice: bigint; paidThrough: bigint;
      pendingDeclaredPrice: bigint; pendingPriceActivation: bigint; status: bigint;
    };
    if (
      value.tokenId !== mint.tokenId ||
      address(value.acolyte, "current acolyte") !== mint.acolyte ||
      address(value.referrer, "referrer") !== mint.referrer ||
      value.status === 0n
    ) throw new Error("Branding current state does not match its mint");
    const avatarResult = await call(rpc, "avatarURIOf", [mint.tokenId], blockTag);
    const avatarUri = boundedText(avatarResult[0], "avatar URI", 2_048);
    const countResult = await call(rpc, "customTraitCount", [mint.tokenId], blockTag);
    const traitCount = countResult[0] as bigint;
    if (traitCount < 0n || traitCount > BigInt(MAX_TRAITS)) throw new Error("too many Branding traits");
    const traits: AcolyteTrait[] = [];
    for (let index = 0n; index < traitCount; index += 1n) {
      const trait = await call(rpc, "customTraitAt", [mint.tokenId, index], blockTag);
      traits.push({
        traitType: boundedText(trait[0], "trait type", 64),
        value: boundedText(trait[1], "trait value", 256),
      });
    }
    items.push({
      tokenId: mint.tokenId.toString(),
      acolyte: mint.acolyte,
      owner: nonzeroAddress(value.owner, "current owner"),
      controllerAgentId: value.controllerAgentId.toString(),
      referrer: mint.referrer,
      declaredPrice: value.declaredPrice.toString(),
      paidThrough: value.paidThrough.toString(),
      pendingDeclaredPrice: value.pendingDeclaredPrice.toString(),
      pendingPriceValidAfter: value.pendingPriceActivation.toString(),
      status: status(value.status),
      avatarUri,
      traits,
      mintBlockNumber: mint.blockNumber.toString(),
      mintTransactionHash: mint.transactionHash,
    });
  }
  const reread = block(await rpc.request("eth_getBlockByNumber", [blockTag, false]));
  if (reread.number !== pinned.number || reread.hash !== pinned.hash) {
    throw new Error("canonical finalized block changed during catalog read");
  }
  return {
    chainId: ACOLYTE_CHAIN_ID,
    contractAddress: ACOLYTE_BRANDING_CONTRACT,
    sourceBlockNumber: pinned.number.toString(),
    sourceBlockHash: pinned.hash,
    items,
  };
}

class BoundedRpc {
  #id = 0;
  constructor(
    endpoint: string,
    private readonly fetcher: typeof fetch,
    private readonly outerSignal?: AbortSignal,
  ) {
    this.endpoint = safeEndpoint(endpoint);
  }

  private readonly endpoint: string;

  async request(method: string, params: unknown[]): Promise<unknown> {
    const id = ++this.#id;
    const controller = new AbortController();
    const abort = (): void => controller.abort(this.outerSignal?.reason);
    if (this.outerSignal?.aborted) abort();
    this.outerSignal?.addEventListener("abort", abort, { once: true });
    const timer = setTimeout(() => controller.abort(), RPC_TIMEOUT_MS);
    try {
      const response = await this.fetcher(this.endpoint, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id, method, params }),
        cache: "no-store",
        credentials: "omit",
        referrerPolicy: "no-referrer",
        signal: controller.signal,
      });
      const declared = Number(response.headers.get("content-length"));
      if (
        !response.ok ||
        (response.headers.has("content-length") &&
          (!Number.isSafeInteger(declared) || declared < 0 || declared > MAX_RPC_BYTES))
      ) {
        throw new Error(`Base RPC failed with HTTP ${response.status}`);
      }
      const raw = await response.text();
      if (new TextEncoder().encode(raw).length > MAX_RPC_BYTES) throw new Error("Base RPC response is too large");
      const parsed = JSON.parse(raw) as unknown;
      if (!record(parsed) || parsed.jsonrpc !== "2.0" || parsed.id !== id || "error" in parsed || !("result" in parsed)) {
        throw new Error("Base RPC returned an invalid response");
      }
      return parsed.result;
    } finally {
      clearTimeout(timer);
      this.outerSignal?.removeEventListener("abort", abort);
    }
  }
}

async function call(rpc: BoundedRpc, method: string, args: unknown[], blockTag: string) {
  const result = bytes(await rpc.request("eth_call", [{
    to: ACOLYTE_BRANDING_CONTRACT,
    data: BRANDING.encodeFunctionData(method, args),
  }, blockTag]), `${method} result`);
  return BRANDING.decodeFunctionResult(method, result);
}

function parseMint(raw: unknown, from: bigint, to: bigint, pinned: bigint): MintRecord {
  if (!record(raw) || !Array.isArray(raw.topics) || raw.topics.length !== 4 || raw.removed === true) {
    throw new Error("malformed Branding mint log");
  }
  const log: RpcLog = raw as unknown as RpcLog;
  if (address(log.address, "log address") !== ACOLYTE_BRANDING_CONTRACT || log.topics[0]?.toLowerCase() !== MINTED_TOPIC) {
    throw new Error("unexpected Branding log source");
  }
  for (const topic of log.topics) if (!HASH.test(topic.toLowerCase())) throw new Error("malformed Branding log topic");
  const blockNumber = quantity(log.blockNumber, "mint block number");
  if (blockNumber < from || blockNumber > to || blockNumber > pinned) throw new Error("Branding log is outside its requested range");
  if (!HASH.test(String(log.transactionHash).toLowerCase())) throw new Error("malformed mint transaction hash");
  const parsed = BRANDING.parseLog({ topics: log.topics, data: bytes(log.data, "mint log data") });
  if (!parsed || parsed.name !== "BrandingMinted") throw new Error("malformed Branding mint log");
  const tokenId = parsed.args.tokenId as bigint;
  const acolyte = nonzeroAddress(parsed.args.acolyte, "mint acolyte");
  nonzeroAddress(parsed.args.owner, "mint owner");
  const referrer = nonzeroAddress(parsed.args.referrer, "mint referrer");
  if (tokenId !== BigInt(acolyte)) {
    throw new Error("Branding token ID does not match its acolyte");
  }
  return { tokenId, acolyte, referrer, blockNumber, transactionHash: log.transactionHash.toLowerCase() };
}

function status(value: bigint): AcolyteBrandingStatus {
  switch (value) {
    case 1n: return "Active";
    case 2n: return "Expired";
    case 3n: return "Ineligible";
    case 4n: return "Registry unavailable";
    default: throw new Error("unknown Branding status");
  }
}

function block(value: unknown): { number: bigint; hash: string } {
  if (!record(value) || typeof value.hash !== "string" || !HASH.test(value.hash.toLowerCase())) {
    throw new Error("invalid Base block header");
  }
  return { number: quantity(value.number, "block number"), hash: value.hash.toLowerCase() };
}

function address(value: unknown, label: string): string {
  if (typeof value !== "string") throw new Error(`${label} is invalid`);
  try { return getAddress(value).toLowerCase(); } catch { throw new Error(`${label} is invalid`); }
}

function nonzeroAddress(value: unknown, label: string): string {
  const parsed = address(value, label);
  if (BigInt(parsed) === 0n) throw new Error(`${label} is zero`);
  return parsed;
}

function quantity(value: unknown, label: string): bigint {
  if (typeof value !== "string" || !QUANTITY.test(value)) throw new Error(`${label} is invalid`);
  return BigInt(value);
}

function bytes(value: unknown, label: string): string {
  if (typeof value !== "string" || !DATA.test(value)) throw new Error(`${label} is invalid`);
  return value;
}

function boundedText(value: unknown, label: string, maximumBytes: number): string {
  if (
    typeof value !== "string" ||
    new TextEncoder().encode(value).length > maximumBytes ||
    /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2060-\u206f\ufeff]/u.test(value)
  ) {
    throw new Error(`${label} is invalid`);
  }
  return value;
}

function safeEndpoint(value: string): string {
  if (value.length > 2_048) throw new Error("Base RPC endpoint is too long");
  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error("Base RPC endpoint must be an absolute HTTPS URL");
  }
  if (parsed.protocol !== "https:" || parsed.username || parsed.password) {
    throw new Error("Base RPC endpoint must be a credential-free HTTPS URL");
  }
  return parsed.href;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hex(value: bigint): string { return `0x${value.toString(16)}`; }
function min(left: bigint, right: bigint): bigint { return left < right ? left : right; }
