import { BASE_CHAIN_ID, UWU_CONTRACT } from "./leaderboard-types";
import { formatLevel, formatWholeUwu, parseRawBalance } from "./level";

const ADDRESS = /^0x[0-9a-fA-F]{40}$/u;
const HEX_QUANTITY = /^0x(?:0|[1-9a-fA-F][0-9a-fA-F]*)$/u;
const UINT256_MAX = (1n << 256n) - 1n;
const BALANCE_OF_SELECTOR = "70a08231";
const DEFAULT_TIMEOUT_MS = 12_000;
const MAX_TIMEOUT_MS = 30_000;
const MAX_RESPONSE_BYTES = 64 * 1024;

interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params: unknown[];
}

export interface FetchAccountBalancesOptions {
  fetch?: typeof fetch;
  /** Primarily useful to give a UI refresh a shorter deadline. */
  timeoutMs?: number;
}

export interface AccountBalances {
  chainId: typeof BASE_CHAIN_ID;
  blockNumber: bigint;
  blockTag: string;
  ethWei: string;
  uwuRaw: string;
  formattedEth: string;
  formattedUwu: string;
  level: string;
}

/**
 * Reads both balances at one explicit Base block. Any unavailable or malformed
 * source data rejects the entire read so the UI cannot mistake an RPC failure
 * for an empty wallet.
 */
export async function fetchAccountBalances(
  endpoint: string,
  account: string,
  options: FetchAccountBalancesOptions = {},
): Promise<AccountBalances> {
  const rpcEndpoint = credentialFreeHttpsEndpoint(endpoint);
  const normalizedAccount = ethereumAddress(account);
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > MAX_TIMEOUT_MS) {
    throw new Error(`Base RPC timeout must be between 1 and ${MAX_TIMEOUT_MS} milliseconds`);
  }

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  const fetcher = options.fetch ?? fetch;
  try {
    const head = await rpcBatch(fetcher, rpcEndpoint, [
      request(1, "eth_chainId", []),
      request(2, "eth_blockNumber", []),
    ], controller.signal);
    const chainId = uint256Quantity(head.get(1), "Base chain ID");
    if (chainId !== BigInt(BASE_CHAIN_ID)) {
      throw new Error(`Base RPC returned chain ID ${chainId}, expected ${BASE_CHAIN_ID}`);
    }
    const blockNumber = uint256Quantity(head.get(2), "Base block number");
    const blockTag = `0x${blockNumber.toString(16)}`;
    const balanceOfData = `0x${BALANCE_OF_SELECTOR}${normalizedAccount.slice(2).padStart(64, "0")}`;

    const balances = await rpcBatch(fetcher, rpcEndpoint, [
      request(3, "eth_getBalance", [normalizedAccount, blockTag]),
      request(4, "eth_call", [{ to: UWU_CONTRACT, data: balanceOfData }, blockTag]),
    ], controller.signal);
    const ethWei = uint256Quantity(balances.get(3), "ETH balance").toString();
    const uwuRaw = uint256Word(balances.get(4), "UWU balanceOf result").toString();

    // Keep the formatting functions on their validated decimal-string path.
    parseRawBalance(ethWei);
    parseRawBalance(uwuRaw);
    return {
      chainId: BASE_CHAIN_ID,
      blockNumber,
      blockTag,
      ethWei,
      uwuRaw,
      formattedEth: formatWholeUwu(ethWei),
      formattedUwu: formatWholeUwu(uwuRaw),
      level: formatLevel(uwuRaw),
    };
  } finally {
    clearTimeout(timer);
  }
}

function request(id: number, method: string, params: unknown[]): JsonRpcRequest {
  return { jsonrpc: "2.0", id, method, params };
}

async function rpcBatch(
  fetcher: typeof fetch,
  endpoint: string,
  requests: JsonRpcRequest[],
  signal: AbortSignal,
): Promise<Map<number, unknown>> {
  const response = await fetcher(endpoint, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(requests),
    cache: "no-store",
    credentials: "omit",
    referrerPolicy: "no-referrer",
    signal,
  });
  if (!response.ok) throw new Error(`Base RPC request failed with HTTP ${response.status}`);
  const declaredLength = response.headers.get("content-length");
  if (declaredLength !== null) {
    const length = Number(declaredLength);
    if (!Number.isSafeInteger(length) || length < 0 || length > MAX_RESPONSE_BYTES) {
      throw new Error("Base RPC response is too large or has an invalid content length");
    }
  }
  const raw = await response.text();
  if (new TextEncoder().encode(raw).length > MAX_RESPONSE_BYTES) {
    throw new Error("Base RPC response is too large");
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw) as unknown;
  } catch {
    throw new Error("Base RPC returned invalid JSON");
  }
  if (!Array.isArray(parsed) || parsed.length !== requests.length) {
    throw new Error("Base RPC returned an incomplete batch");
  }

  const expectedIds = new Set(requests.map(({ id }) => id));
  const results = new Map<number, unknown>();
  for (const value of parsed) {
    if (!isRecord(value) || value.jsonrpc !== "2.0" || !Number.isSafeInteger(value.id)) {
      throw new Error("Base RPC returned an invalid JSON-RPC response");
    }
    const id = value.id as number;
    if (!expectedIds.has(id) || results.has(id) || "error" in value || !("result" in value)) {
      throw new Error("Base RPC returned an invalid JSON-RPC response");
    }
    results.set(id, value.result);
  }
  if (results.size !== requests.length) throw new Error("Base RPC returned an incomplete batch");
  return results;
}

function credentialFreeHttpsEndpoint(value: string): string {
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

function ethereumAddress(value: string): string {
  if (!ADDRESS.test(value)) throw new Error("account must be an Ethereum address");
  return value.toLowerCase();
}

function uint256Quantity(value: unknown, label: string): bigint {
  if (typeof value !== "string" || !HEX_QUANTITY.test(value)) {
    throw new Error(`${label} is not a canonical hex quantity`);
  }
  const parsed = BigInt(value);
  if (parsed > UINT256_MAX) throw new Error(`${label} exceeds uint256`);
  return parsed;
}

function uint256Word(value: unknown, label: string): bigint {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]{64}$/u.test(value)) {
    throw new Error(`${label} is not ABI-encoded uint256 data`);
  }
  return BigInt(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
