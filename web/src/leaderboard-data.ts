import {
  ALLEGIANCE_HEX,
  BASE_CHAIN_ID,
  BASE_NAME,
  IDENTITY_REGISTRY,
  LEADERBOARD_CACHE_VERSION,
  PROTOCOL_V1_HEX,
  REPUTATION_REGISTRY,
  UWU_CONTRACT,
  ZERO_ADDRESS,
  type LeaderboardSnapshot,
  type RankedWallet,
  type ReputationCounters,
  type ReputationSignal,
  type TentacleIdentity,
  type TentacleProfile,
} from "./leaderboard-types";
import { compareRawBalances, parseRawBalance } from "./level";
import { fallbackProfile, parseDataRegistration } from "./profile";

const PAGE_SIZE = 250;
const MAX_PAGES = 400;
const MAX_RESPONSE_BYTES = 2 * 1024 * 1024;
const MAX_TOTAL_RESPONSE_BYTES = 64 * 1024 * 1024;
const FETCH_TIMEOUT_MS = 20_000;
const REGISTRATION_TYPE = "https://eips.ethereum.org/EIPS/eip-8004#registration-v1";
const ADDRESS = /^0x[0-9a-fA-F]{40}$/u;
const BYTES = /^0x(?:[0-9a-fA-F]{2})*$/u;
const UNSIGNED = /^(0|[1-9][0-9]*)$/u;
const SIGNED = /^(0|-?[1-9][0-9]*)$/u;

const LEADERBOARD_QUERY = `
  query CthuwuLeaderboard($first: Int!, $after: ID!, $block: Block_height, $allegiance: Bytes!) {
    agentMetadatas: agentMetadata_collection(
      first: $first
      orderBy: id
      orderDirection: asc
      where: { id_gt: $after, key: "cthuwu.allegiance", value: $allegiance }
      block: $block
      subgraphError: deny
    ) {
      id key value updatedAt
      agent {
        id chainId agentId agentURI owner agentWallet createdAt updatedAt totalFeedback
        metadata { id key value updatedAt }
        registrationFile { id cid name description image active endpointsRawJson createdAt }
        feedback(first: 10, orderBy: createdAt, orderDirection: desc) {
          id clientAddress feedbackIndex value tag1 tag2 endpoint feedbackURI feedbackHash
          isRevoked createdAt revokedAt
        }
      }
    }
    _meta(block: $block) {
      block { number hash timestamp }
      deployment
      hasIndexingErrors
    }
  }
`;

interface GraphResponse {
  data?: unknown;
  errors?: unknown;
}

export class IndexingError extends Error {
  constructor() {
    super("Agent0 reports an indexing error");
    this.name = "IndexingError";
  }
}

export interface FetchLeaderboardOptions {
  fetch?: typeof fetch;
  now?: () => Date;
  baseRpcEndpoint?: string;
  diagnostic?: (event: string, details: Record<string, string | number | boolean>) => void;
}

export async function fetchCompleteLeaderboard(
  endpoint: string,
  options: FetchLeaderboardOptions = {},
): Promise<LeaderboardSnapshot> {
  const fetcher = options.fetch ?? fetch;
  const identities: TentacleIdentity[] = [];
  let pinnedBlock: string | undefined;
  let pinnedHash: string | undefined;
  let pinnedTimestamp: string | undefined;
  let deployment: string | undefined;
  let after = "";
  let totalResponseBytes = 0;

  for (let page = 0; page < MAX_PAGES; page += 1) {
    const response = await fetchPage(fetcher, endpoint, after, pinnedBlock);
    totalResponseBytes += response.byteLength;
    if (totalResponseBytes > MAX_TOTAL_RESPONSE_BYTES) {
      throw new Error("complete subgraph response exceeds the aggregate safety limit");
    }
    const parsed = parsePage(response.body);
    options.diagnostic?.("agent0-page", {
      page: page + 1,
      rows: parsed.rowCount,
      block: parsed.meta.blockNumber,
      indexingErrors: parsed.meta.hasIndexingErrors,
    });
    if (parsed.meta.hasIndexingErrors) throw new IndexingError();
    if (parsed.firstCursor && parsed.firstCursor <= after) {
      throw new Error("subgraph pagination returned an overlapping page");
    }
    if (pinnedBlock === undefined) {
      pinnedBlock = parsed.meta.blockNumber;
      pinnedHash = parsed.meta.blockHash;
      pinnedTimestamp = parsed.meta.blockTimestamp;
      deployment = parsed.meta.deployment;
    } else if (
      parsed.meta.blockNumber !== pinnedBlock ||
      parsed.meta.deployment !== deployment ||
      parsed.meta.blockHash !== pinnedHash ||
      parsed.meta.blockTimestamp !== pinnedTimestamp
    ) {
      throw new Error("subgraph pagination changed source block or deployment");
    }

    identities.push(...parsed.identities);
    if (parsed.rowCount < PAGE_SIZE) {
      const balanced = await attachUwuBalances(
        fetcher,
        options.baseRpcEndpoint ?? "https://mainnet.base.org/",
        identities,
        parsed.meta,
      );
      options.diagnostic?.("base-balances-verified", {
        identities: identities.length,
        wallets: new Set(identities.map((identity) => identity.agentWallet)).size,
        block: parsed.meta.blockNumber,
      });
      return buildSnapshot(
        balanced,
        {
          blockNumber: pinnedBlock,
          blockHash: pinnedHash,
          blockTimestamp: pinnedTimestamp,
          deployment: deployment ?? "unknown",
        },
        (options.now ?? (() => new Date()))(),
      );
    }
    const nextAfter = parsed.lastCursor;
    if (!nextAfter || nextAfter <= after) {
      throw new Error("subgraph pagination cursor did not advance");
    }
    after = nextAfter;
  }
  throw new Error("subgraph pagination exceeded the bounded page limit");
}

async function fetchPage(
  fetcher: typeof fetch,
  endpoint: string,
  after: string,
  blockNumber?: string,
): Promise<{ body: GraphResponse; byteLength: number }> {
  const pinnedNumber = blockNumber === undefined ? undefined : Number(blockNumber);
  if (pinnedNumber !== undefined && !Number.isSafeInteger(pinnedNumber)) {
    throw new Error("source block number cannot be pinned safely");
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
  try {
    const response = await fetcher(endpoint, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        query: LEADERBOARD_QUERY,
        variables: {
          first: PAGE_SIZE,
          after,
          block: pinnedNumber === undefined ? null : { number: pinnedNumber },
          allegiance: ALLEGIANCE_HEX,
        },
      }),
      cache: "no-store",
      credentials: "omit",
      referrerPolicy: "no-referrer",
      signal: controller.signal,
    });
    if (!response.ok) throw new Error(`subgraph request failed with HTTP ${response.status}`);
    const declaredLength = Number(response.headers.get("content-length"));
    if (Number.isFinite(declaredLength) && declaredLength > MAX_RESPONSE_BYTES) {
      throw new Error("subgraph response is too large");
    }
    const raw = await response.text();
    const byteLength = new TextEncoder().encode(raw).length;
    if (byteLength > MAX_RESPONSE_BYTES) {
      throw new Error("subgraph response is too large");
    }
    return { body: JSON.parse(raw) as GraphResponse, byteLength };
  } finally {
    clearTimeout(timer);
  }
}

function parsePage(response: GraphResponse): {
  identities: TentacleIdentity[];
  rowCount: number;
  firstCursor?: string;
  lastCursor?: string;
  meta: {
    blockNumber: string;
    blockHash?: string;
    blockTimestamp?: string;
    deployment: string;
    hasIndexingErrors: boolean;
  };
} {
  if (Array.isArray(response.errors) && response.errors.length > 0) {
    if (JSON.stringify(response.errors).includes("indexing_error")) throw new IndexingError();
    throw new Error("subgraph returned GraphQL errors");
  }
  const data = record(response.data, "Graph response data");
  const meta = record(data._meta, "Graph _meta");
  const block = record(meta.block, "Graph _meta block");
  const blockNumber = unsigned(block.number, "source block number");
  const blockHash = optionalBytes(block.hash, 32);
  const blockTimestamp = optionalUnsigned(block.timestamp);
  const deployment = boundedText(meta.deployment, "deployment", 256);
  const hasIndexingErrors = boolean(meta.hasIndexingErrors, "hasIndexingErrors");
  if (!Array.isArray(data.agentMetadatas) || data.agentMetadatas.length > PAGE_SIZE) {
    throw new Error("subgraph returned an invalid Tentacle page");
  }
  const rows = data.agentMetadatas as unknown[];
  const cursors = rows.map((row) => boundedText(record(row, "metadata cursor").id, "cursor", 256));
  for (let index = 1; index < cursors.length; index += 1) {
    if (cursors[index - 1] >= cursors[index]) {
      throw new Error("subgraph page is not strictly ordered by metadata ID");
    }
  }
  const firstCursor = cursors.at(0);
  const lastCursor = cursors.at(-1);
  return {
    identities: rows.map(parseTentacle).filter((value) => value !== undefined),
    rowCount: rows.length,
    ...(firstCursor ? { firstCursor } : {}),
    ...(lastCursor ? { lastCursor } : {}),
    meta: {
      blockNumber,
      ...(blockHash ? { blockHash } : {}),
      ...(blockTimestamp ? { blockTimestamp } : {}),
      deployment,
      hasIndexingErrors,
    },
  };
}

function parseTentacle(value: unknown): TentacleIdentity | undefined {
  const outer = record(value, "Tentacle allegiance metadata");
  if (outer.key !== "cthuwu.allegiance") throw new Error("Agent0 returned a different metadata key");
  const allegianceHex = bytes(outer.value, "allegiance", 256);
  if (allegianceHex !== ALLEGIANCE_HEX) return undefined;
  const row = record(outer.agent, "Agent0 agent");
  if (unsigned(row.chainId, "chainId") !== String(BASE_CHAIN_ID)) throw new Error("Agent0 returned a non-Base agent");
  const agentId = unsigned(row.agentId, "agentId", 78);
  if (row.id !== `${BASE_CHAIN_ID}:${agentId}`) throw new Error("Agent0 agent ID is inconsistent");
  if (!Array.isArray(row.metadata) || row.metadata.length > 256) throw new Error("Agent0 metadata is invalid");
  const metadata = new Map<string, { value: string; updatedAt: string }>();
  for (const item of row.metadata as unknown[]) {
    const entry = record(item, "Agent0 metadata entry");
    const key = boundedText(entry.key, "metadata key", 256);
    if (metadata.has(key)) throw new Error("Agent0 returned duplicate current metadata");
    metadata.set(key, { value: bytes(entry.value, "metadata value", 8_192), updatedAt: unsigned(entry.updatedAt, "metadata updatedAt") });
  }
  if (metadata.get("cthuwu.allegiance")?.value !== ALLEGIANCE_HEX) throw new Error("Agent0 current allegiance is inconsistent");
  const protocolHex = metadata.get("cthuwu.protocol")?.value ?? "0x";
  const tentacleId = decodeMetadataText(metadata.get("cthuwu.tentacle-id")?.value, 96);
  const walletValue = row.agentWallet;
  const agentWallet = walletValue === null || walletValue === undefined ? ZERO_ADDRESS : address(walletValue, "agentWallet");
  const agentUri = optionalSafeText(row.agentURI, 24 * 1024) ?? "";
  const parsedFileProfile = parseFileProfile(row.registrationFile, agentUri, agentId, tentacleId);
  const profile =
    parseDataRegistration(agentUri, agentId, tentacleId) ?? parsedFileProfile ??
    fallbackProfile(agentId, tentacleId);
  if (!Array.isArray(row.feedback) || row.feedback.length > 10) {
    throw new Error("subgraph returned an invalid recent-feedback sample");
  }
  const reputation = row.feedback.map(parseFeedback).filter((signal) => signal !== undefined);
  const reputationCounters: ReputationCounters = {
    active: uint256(row.totalFeedback, "totalFeedback"),
    sampledRevoked: String(reputation.filter((signal) => signal.revoked).length),
  };
  const sampledActive = reputation.filter((signal) => !signal.revoked).length;
  if (BigInt(sampledActive) > BigInt(reputationCounters.active)) {
    throw new Error("subgraph reputation sample exceeds its registry counters");
  }
  const createdAt = unsigned(row.createdAt, "createdAt");
  const updatedAt = unsigned(row.updatedAt, "updatedAt");
  const metadataUpdatedAt = [...metadata.values()].reduce((latest, item) => BigInt(item.updatedAt) > BigInt(latest) ? item.updatedAt : latest, createdAt);
  const identity: TentacleIdentity = {
    agentId,
    owner: address(row.owner, "owner"),
    agentUri,
    agentWallet,
    allegianceHex,
    protocolHex,
    ...(tentacleId ? { tentacleId } : {}),
    registrationBlock: "0",
    registrationTimestamp: createdAt,
    profileUpdatedBlock: "0",
    profileUpdatedTimestamp: updatedAt,
    metadataUpdatedBlock: "0",
    metadataUpdatedTimestamp: metadataUpdatedAt,
    rawBalance: "0",
    profile,
    reputationCounters,
    reputation,
  };
  return identity;
}

function parseFileProfile(
  value: unknown,
  agentUri: string,
  agentId: string,
  tentacleId?: string,
): TentacleProfile | undefined {
  if (value === null || value === undefined) return undefined;
  try {
    const profile = record(value, "profile");
    const name = optionalSafeText(profile.name, 128) ?? fallbackProfile(agentId, tentacleId).name;
    const description = optionalSafeText(profile.description, 512);
    const image = optionalSafeUrl(profile.image);
    const { xmtpEndpoint, cthuwuEndpoint } = parseAgent0Endpoints(profile.endpointsRawJson);
    const sourceUri = agentUri || `agent0:${boundedText(profile.id, "registration file ID", 256)}`;
    return {
      name,
      ...(description ? { description } : {}),
      ...(image ? { image } : {}),
      active: profile.active !== false,
      ...(xmtpEndpoint ? { xmtpEndpoint } : {}),
      ...(cthuwuEndpoint ? { cthuwuEndpoint } : {}),
      sourceUri,
    };
  } catch {
    return undefined;
  }
}

function parseFeedback(value: unknown): ReputationSignal | undefined {
  try {
    const signal = record(value, "feedback");
    const { rawValue, valueDecimals } = decimalToParts(boundedText(signal.value, "feedback value", 96));
    const tag1 = optionalSafeText(signal.tag1, 128);
    const tag2 = optionalSafeText(signal.tag2, 128);
    const endpoint = optionalSafePublicEndpoint(signal.endpoint);
    return {
      id: boundedText(signal.id, "feedback id", 256),
      clientAddress: address(signal.clientAddress, "feedback client"),
      value: rawValue,
      valueDecimals,
      ...(tag1 ? { tag1 } : {}),
      ...(tag2 ? { tag2 } : {}),
      ...(endpoint ? { endpoint } : {}),
      createdAt: unsigned(signal.createdAt, "feedback createdAt"),
      revoked: boolean(signal.isRevoked, "feedback isRevoked"),
      provenance: "ERC-8004 Reputation Registry via Agent0",
    };
  } catch {
    return undefined;
  }
}

function decodeMetadataText(value: string | undefined, maximum: number): string | undefined {
  if (!value || value === "0x") return undefined;
  try {
    const decoded = new TextDecoder("utf-8", { fatal: true }).decode(
      Uint8Array.from(value.slice(2).match(/../gu) ?? [], (hex) => Number.parseInt(hex, 16)),
    );
    return optionalSafeText(decoded, maximum);
  } catch {
    return undefined;
  }
}

function parseAgent0Endpoints(value: unknown): { xmtpEndpoint?: string; cthuwuEndpoint?: string } {
  if (typeof value !== "string" || value.length > 16_384) return {};
  try {
    const parsed: unknown = JSON.parse(value);
    if (!Array.isArray(parsed) || parsed.length > 32) return {};
    let xmtpEndpoint: string | undefined;
    let cthuwuEndpoint: string | undefined;
    for (const item of parsed) {
      const endpoint = record(item, "profile endpoint");
      const name = optionalSafeText(endpoint.name, 64);
      const url = endpoint.endpoint ?? endpoint.uri;
      if ((name === "CTHUWU-XMTP" || name === "XMTP") && !xmtpEndpoint) xmtpEndpoint = optionalXmtp(url);
      if (name === "CTHUWU" && !cthuwuEndpoint) cthuwuEndpoint = optionalSafePublicEndpoint(url);
    }
    return { ...(xmtpEndpoint ? { xmtpEndpoint } : {}), ...(cthuwuEndpoint ? { cthuwuEndpoint } : {}) };
  } catch {
    return {};
  }
}

function decimalToParts(value: string): { rawValue: string; valueDecimals: number } {
  const match = /^(-?)(0|[1-9][0-9]*)(?:\.([0-9]{1,18}))?$/u.exec(value);
  if (!match) throw new Error("feedback value is not a bounded decimal");
  const decimals = match[3]?.length ?? 0;
  const magnitude = `${match[2]}${match[3] ?? ""}`.replace(/^0+(?=\d)/u, "");
  return { rawValue: `${match[1]}${magnitude}`, valueDecimals: decimals };
}

async function attachUwuBalances(
  fetcher: typeof fetch,
  endpoint: string,
  identities: TentacleIdentity[],
  meta: { blockNumber: string; blockHash?: string; blockTimestamp?: string },
): Promise<TentacleIdentity[]> {
  if (!meta.blockHash) throw new Error("Agent0 did not report a source block hash");
  const blockTag = `0x${BigInt(meta.blockNumber).toString(16)}`;
  const block = await rpc(fetcher, endpoint, "eth_getBlockByNumber", [blockTag, false], 1);
  const blockRecord = record(block, "Base block");
  if (bytes(blockRecord.hash, "Base block hash", 32) !== meta.blockHash ||
      unsignedHex(blockRecord.number, "Base block number") !== BigInt(meta.blockNumber)) {
    throw new Error("Base RPC does not match the Agent0 source block");
  }
  if (meta.blockTimestamp && unsignedHex(blockRecord.timestamp, "Base block timestamp") !== BigInt(meta.blockTimestamp)) {
    throw new Error("Base RPC timestamp does not match Agent0");
  }
  const wallets = [...new Set(identities.map((identity) => identity.agentWallet).filter((wallet) => wallet !== ZERO_ADDRESS))];
  const balances = new Map<string, string>();
  for (let offset = 0; offset < wallets.length; offset += 100) {
    const batchWallets = wallets.slice(offset, offset + 100);
    const requests = batchWallets.map((wallet, index) => ({
      jsonrpc: "2.0", id: index + 1, method: "eth_call",
      params: [{ to: UWU_CONTRACT, data: `0x70a08231${wallet.slice(2).padStart(64, "0")}` }, blockTag],
    }));
    const results = await rpcBatch(fetcher, endpoint, requests);
    for (let index = 0; index < batchWallets.length; index += 1) {
      const result = results.get(index + 1);
      if (typeof result !== "string" || !/^0x[0-9a-fA-F]{64}$/u.test(result)) throw new Error("UWU balanceOf returned invalid uint256 data");
      const balance = BigInt(result).toString();
      parseRawBalance(balance);
      balances.set(batchWallets[index], balance);
    }
  }
  return identities.map((identity) => ({ ...identity, rawBalance: balances.get(identity.agentWallet) ?? "0" }));
}

async function rpc(fetcher: typeof fetch, endpoint: string, method: string, params: unknown[], id: number): Promise<unknown> {
  const results = await rpcBatch(fetcher, endpoint, [{ jsonrpc: "2.0", id, method, params }]);
  return results.get(id);
}

async function rpcBatch(fetcher: typeof fetch, endpoint: string, requests: unknown[]): Promise<Map<number, unknown>> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
  try {
    const response = await fetcher(endpoint, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(requests.length === 1 ? requests[0] : requests), cache: "no-store", credentials: "omit", referrerPolicy: "no-referrer", signal: controller.signal });
    if (!response.ok) throw new Error(`Base RPC request failed with HTTP ${response.status}`);
    const raw = await response.text();
    if (new TextEncoder().encode(raw).length > MAX_RESPONSE_BYTES) throw new Error("Base RPC response is too large");
    const parsed: unknown = JSON.parse(raw);
    const responses = Array.isArray(parsed) ? parsed : [parsed];
    const result = new Map<number, unknown>();
    for (const item of responses) {
      const row = record(item, "JSON-RPC response");
      if (!Number.isSafeInteger(row.id) || result.has(row.id as number) || row.error !== undefined) throw new Error("Base RPC returned an invalid response");
      result.set(row.id as number, row.result);
    }
    if (result.size !== requests.length) throw new Error("Base RPC returned an incomplete batch");
    return result;
  } finally { clearTimeout(timer); }
}

function unsignedHex(value: unknown, label: string): bigint {
  const text = boundedText(value, label, 66);
  if (!/^0x(?:0|[1-9a-fA-F][0-9a-fA-F]*)$/u.test(text)) throw new Error(`${label} is not canonical hex`);
  return BigInt(text);
}

function buildSnapshot(
  identities: TentacleIdentity[],
  source: {
    blockNumber: string;
    blockHash?: string;
    blockTimestamp?: string;
    deployment: string;
  },
  now: Date,
): LeaderboardSnapshot {
  const uniqueIds = new Set<string>();
  const groups = new Map<string, TentacleIdentity[]>();
  const suspended: TentacleIdentity[] = [];
  for (const identity of identities) {
    if (uniqueIds.has(identity.agentId)) throw new Error("subgraph returned a duplicate agent ID");
    uniqueIds.add(identity.agentId);
    if (identity.agentWallet === ZERO_ADDRESS) {
      suspended.push(identity);
      continue;
    }
    const wallet = identity.agentWallet.toLowerCase();
    const group = groups.get(wallet) ?? [];
    if (group.length > 0 && group[0].rawBalance !== identity.rawBalance) {
      throw new Error("shared agentWallet has inconsistent UWU balances");
    }
    group.push(identity);
    groups.set(wallet, group);
  }
  const rankedWallets: RankedWallet[] = [...groups.entries()].map(([wallet, members]) => {
    members.sort((a, b) => compareUnsigned(a.agentId, b.agentId));
    return {
      wallet,
      rawBalance: members[0].rawBalance,
      representativeAgentId: members[0].agentId,
      identities: members,
    };
  });
  rankedWallets.sort((a, b) => {
    const balance = compareRawBalances(a.rawBalance, b.rawBalance);
    if (balance !== 0) return balance;
    const registration = compareUnsigned(
      earliestRegistrationTimestamp(a),
      earliestRegistrationTimestamp(b),
    );
    return registration !== 0
      ? registration
      : compareUnsigned(a.representativeAgentId, b.representativeAgentId);
  });
  let rank = 0;
  for (const group of rankedWallets) {
    if (BigInt(group.rawBalance) > 0n) group.rank = ++rank;
  }
  suspended.sort((a, b) => compareUnsigned(a.agentId, b.agentId));
  return {
    cacheSchemaVersion: LEADERBOARD_CACHE_VERSION,
    network: BASE_NAME,
    chainId: BASE_CHAIN_ID,
    identityRegistry: IDENTITY_REGISTRY,
    reputationRegistry: REPUTATION_REGISTRY,
    uwuContract: UWU_CONTRACT,
    sourceDeployment: source.deployment,
    sourceBlockNumber: source.blockNumber,
    ...(source.blockHash ? { sourceBlockHash: source.blockHash } : {}),
    ...(source.blockTimestamp ? { sourceBlockTimestamp: source.blockTimestamp } : {}),
    hasIndexingErrors: false,
    fetchedAt: now.toISOString(),
    paginationComplete: true,
    rankedWallets,
    suspended,
  };
}

function earliestRegistrationTimestamp(group: RankedWallet): string {
  return group.identities.reduce(
    (earliest, identity) =>
      compareUnsigned(identity.registrationTimestamp, earliest) < 0
        ? identity.registrationTimestamp
        : earliest,
    group.identities[0].registrationTimestamp,
  );
}

export function isProtocolV1(identity: TentacleIdentity): boolean {
  return identity.protocolHex === PROTOCOL_V1_HEX;
}

function compareUnsigned(left: string, right: string): number {
  const a = BigInt(left);
  const b = BigInt(right);
  return a === b ? 0 : a < b ? -1 : 1;
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} has an invalid shape`);
  }
  return value as Record<string, unknown>;
}

function boundedText(value: unknown, label: string, maximum: number): string {
  if (typeof value !== "string" || value.length === 0 || value.length > maximum || hasControl(value)) {
    throw new Error(`${label} is invalid`);
  }
  return value;
}

function optionalSafeText(value: unknown, maximum: number): string | undefined {
  if (value === null || value === undefined || value === "") return undefined;
  if (typeof value !== "string" || value.length > maximum || hasControl(value)) return undefined;
  return value;
}

function unsigned(value: unknown, label: string, maximum = 32): string {
  const text = boundedText(value, label, maximum);
  if (!UNSIGNED.test(text)) throw new Error(`${label} is not an unsigned integer`);
  return text;
}

function uint256(value: unknown, label: string): string {
  const text = unsigned(value, label, 78);
  if (BigInt(text) >= 1n << 256n) throw new Error(`${label} exceeds uint256`);
  return text;
}

function optionalUnsigned(value: unknown): string | undefined {
  if (value === null || value === undefined) return undefined;
  return unsigned(value, "optional integer");
}

function address(value: unknown, label: string): string {
  const text = boundedText(value, label, 42);
  if (!ADDRESS.test(text)) throw new Error(`${label} is not an address`);
  return text.toLowerCase();
}

function bytes(value: unknown, label: string, maximumBytes: number): string {
  const text = boundedText(value, label, maximumBytes * 2 + 2);
  if (!BYTES.test(text)) throw new Error(`${label} is not bytes`);
  return text.toLowerCase();
}

function optionalBytes(value: unknown, maximumBytes: number): string | undefined {
  if (value === null || value === undefined || value === "") return undefined;
  return bytes(value, "optional bytes", maximumBytes);
}

function boolean(value: unknown, label: string): boolean {
  if (typeof value !== "boolean") throw new Error(`${label} is not boolean`);
  return value;
}

function number(value: unknown, label: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value)) throw new Error(`${label} is invalid`);
  return value;
}

function hasControl(value: string): boolean {
  return [...value].some((character) => {
    const code = character.codePointAt(0) ?? 0;
    return (
      (code >= 0 && code <= 0x1f) ||
      (code >= 0x7f && code <= 0x9f) ||
      code === 0x061c ||
      (code >= 0x200b && code <= 0x200f) ||
      (code >= 0x202a && code <= 0x202e) ||
      (code >= 0x2060 && code <= 0x206f) ||
      code === 0xfeff
    );
  });
}

function optionalSafeUrl(value: unknown): string | undefined {
  const text = optionalSafeText(value, 2_048);
  if (!text) return undefined;
  if (text.startsWith("ipfs://") || text.startsWith("ar://")) return text;
  try {
    const url = new URL(text);
    return url.protocol === "https:" && !url.username && !url.password ? url.href : undefined;
  } catch {
    return undefined;
  }
}

function optionalSafePublicEndpoint(value: unknown): string | undefined {
  return optionalSafeUrl(value);
}

function optionalXmtp(value: unknown): string | undefined {
  const text = optionalSafeText(value, 256);
  if (!text) return undefined;
  return /^xmtp:\/\/[0-9a-f]{64}$/u.test(text) ? text : undefined;
}
