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
  query CthuwuLeaderboard($first: Int!, $after: BigInt!, $block: Block_height) {
    tentacles(
      first: $first
      orderBy: agentId
      orderDirection: asc
      where: { agentId_gt: $after }
      block: $block
      subgraphError: deny
    ) {
      id
      agentId
      owner
      agentURI
      agentWallet
      allegiance
      protocol
      tentacleId
      isTentacle
      isWalletVerified
      registrationBlock
      registrationTimestamp
      profileUpdatedBlock
      profileUpdatedTimestamp
      metadataUpdatedBlock
      metadataUpdatedTimestamp
      feedbackCount
      activeFeedbackCount
      revokedFeedbackCount
      wallet { address rawBalance updatedBlock updatedTimestamp }
      profile {
        id schemaType name description image active xmtpEndpoint cthuwuEndpoint
        sourceURI contentHash parseValid
      }
      feedbacks(first: 10, orderBy: createdTimestamp, orderDirection: desc) {
        id clientAddress feedbackIndex value valueDecimals tag1 tag2 endpoint
        feedbackURI feedbackHash isRevoked createdBlock createdTimestamp
        createdTransaction provenance
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
    super("The Cthuwu subgraph reports an indexing error");
    this.name = "IndexingError";
  }
}

export interface FetchLeaderboardOptions {
  fetch?: typeof fetch;
  now?: () => Date;
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
  let after = "-1";
  let totalResponseBytes = 0;

  for (let page = 0; page < MAX_PAGES; page += 1) {
    const response = await fetchPage(fetcher, endpoint, after, pinnedBlock);
    totalResponseBytes += response.byteLength;
    if (totalResponseBytes > MAX_TOTAL_RESPONSE_BYTES) {
      throw new Error("complete subgraph response exceeds the aggregate safety limit");
    }
    const parsed = parsePage(response.body);
    if (parsed.meta.hasIndexingErrors) throw new IndexingError();
    if (parsed.firstAgentId && BigInt(parsed.firstAgentId) <= BigInt(after)) {
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
      return buildSnapshot(
        identities,
        {
          blockNumber: pinnedBlock,
          blockHash: pinnedHash,
          blockTimestamp: pinnedTimestamp,
          deployment: deployment ?? "unknown",
        },
        (options.now ?? (() => new Date()))(),
      );
    }
    const nextAfter = parsed.lastAgentId;
    if (!nextAfter || BigInt(nextAfter) <= BigInt(after)) {
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
  firstAgentId?: string;
  lastAgentId?: string;
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
  if (!Array.isArray(data.tentacles) || data.tentacles.length > PAGE_SIZE) {
    throw new Error("subgraph returned an invalid Tentacle page");
  }
  const rows = data.tentacles as unknown[];
  const rowAgentIds = rows.map((row) =>
    unsigned(record(row, "Tentacle cursor").agentId, "cursor agentId", 78),
  );
  for (let index = 1; index < rowAgentIds.length; index += 1) {
    if (BigInt(rowAgentIds[index - 1]) >= BigInt(rowAgentIds[index])) {
      throw new Error("subgraph page is not strictly ordered by agentId");
    }
  }
  const firstAgentId = rowAgentIds.at(0);
  const lastAgentId = rowAgentIds.at(-1);
  return {
    identities: rows.map(parseTentacle).filter((value) => value !== undefined),
    rowCount: rows.length,
    ...(firstAgentId ? { firstAgentId } : {}),
    ...(lastAgentId ? { lastAgentId } : {}),
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
  const row = record(value, "Tentacle");
  const agentId = unsigned(row.agentId, "agentId", 78);
  if (row.id !== agentId) throw new Error("Tentacle entity ID does not match agentId");
  const allegianceHex = bytes(row.allegiance, "allegiance", 256);
  const isTentacle = boolean(row.isTentacle, "isTentacle");
  const hasExactAllegiance = allegianceHex === ALLEGIANCE_HEX;
  if (isTentacle !== hasExactAllegiance) {
    throw new Error("subgraph Tentacle flag disagrees with exact current allegiance bytes");
  }
  if (!hasExactAllegiance) return undefined;
  const agentWallet = address(row.agentWallet, "agentWallet");
  const verified = boolean(row.isWalletVerified, "isWalletVerified");
  const wallet = row.wallet === null || row.wallet === undefined ? undefined : record(row.wallet, "wallet");
  if (verified && agentWallet !== ZERO_ADDRESS && !wallet) {
    throw new Error("verified agentWallet is missing its UWU wallet state");
  }
  const rawBalance = wallet ? unsigned(wallet.rawBalance, "rawBalance", 78) : "0";
  parseRawBalance(rawBalance);
  if (wallet && address(wallet.address, "wallet address") !== agentWallet) {
    throw new Error("subgraph wallet relation does not match agentWallet");
  }
  const tentacleId = optionalSafeText(row.tentacleId, 96);
  const agentUri = optionalSafeText(row.agentURI, 24 * 1024) ?? "";
  const parsedFileProfile = parseFileProfile(row.profile, agentId, tentacleId);
  const profile =
    parsedFileProfile ??
    parseDataRegistration(agentUri, agentId, tentacleId) ??
    fallbackProfile(agentId, tentacleId);
  if (!Array.isArray(row.feedbacks) || row.feedbacks.length > 10) {
    throw new Error("subgraph returned an invalid recent-feedback sample");
  }
  const reputationCounters: ReputationCounters = {
    total: uint256(row.feedbackCount, "feedbackCount"),
    active: uint256(row.activeFeedbackCount, "activeFeedbackCount"),
    revoked: uint256(row.revokedFeedbackCount, "revokedFeedbackCount"),
  };
  if (
    BigInt(reputationCounters.active) + BigInt(reputationCounters.revoked) !==
    BigInt(reputationCounters.total)
  ) {
    throw new Error("subgraph reputation counters are inconsistent");
  }
  const reputation = row.feedbacks.map(parseFeedback).filter((signal) => signal !== undefined);
  const sampledActive = reputation.filter((signal) => !signal.revoked).length;
  const sampledRevoked = reputation.length - sampledActive;
  if (
    BigInt(reputation.length) > BigInt(reputationCounters.total) ||
    BigInt(sampledActive) > BigInt(reputationCounters.active) ||
    BigInt(sampledRevoked) > BigInt(reputationCounters.revoked)
  ) {
    throw new Error("subgraph reputation sample exceeds its registry counters");
  }
  const identity: TentacleIdentity = {
    agentId,
    owner: address(row.owner, "owner"),
    agentUri,
    agentWallet: verified ? agentWallet : ZERO_ADDRESS,
    allegianceHex,
    protocolHex: bytes(row.protocol, "protocol", 256),
    ...(tentacleId ? { tentacleId } : {}),
    registrationBlock: unsigned(row.registrationBlock, "registrationBlock"),
    registrationTimestamp: unsigned(row.registrationTimestamp, "registrationTimestamp"),
    profileUpdatedBlock: unsigned(row.profileUpdatedBlock, "profileUpdatedBlock"),
    profileUpdatedTimestamp: unsigned(row.profileUpdatedTimestamp, "profileUpdatedTimestamp"),
    metadataUpdatedBlock: unsigned(row.metadataUpdatedBlock, "metadataUpdatedBlock"),
    metadataUpdatedTimestamp: unsigned(row.metadataUpdatedTimestamp, "metadataUpdatedTimestamp"),
    rawBalance: verified && agentWallet !== ZERO_ADDRESS ? rawBalance : "0",
    ...(wallet ? { balanceUpdatedBlock: unsigned(wallet.updatedBlock, "wallet updatedBlock") } : {}),
    ...(wallet
      ? { balanceUpdatedTimestamp: unsigned(wallet.updatedTimestamp, "wallet updatedTimestamp") }
      : {}),
    profile,
    reputationCounters,
    reputation,
  };
  return identity;
}

function parseFileProfile(
  value: unknown,
  agentId: string,
  tentacleId?: string,
): TentacleProfile | undefined {
  if (value === null || value === undefined) return undefined;
  try {
    const profile = record(value, "profile");
    if (profile.parseValid !== true || profile.schemaType !== REGISTRATION_TYPE) return undefined;
    const name = optionalSafeText(profile.name, 128) ?? fallbackProfile(agentId, tentacleId).name;
    const description = optionalSafeText(profile.description, 512);
    const image = optionalSafeUrl(profile.image);
    const xmtpEndpoint = optionalXmtp(profile.xmtpEndpoint);
    const cthuwuEndpoint = optionalSafePublicEndpoint(profile.cthuwuEndpoint);
    const sourceUri = optionalSafePublicEndpoint(profile.sourceURI) ?? "content-addressed profile";
    const contentHash = optionalBytes(profile.contentHash, 64);
    return {
      name,
      ...(description ? { description } : {}),
      ...(image ? { image } : {}),
      active: profile.active === true,
      ...(xmtpEndpoint ? { xmtpEndpoint } : {}),
      ...(cthuwuEndpoint ? { cthuwuEndpoint } : {}),
      sourceUri,
      ...(contentHash ? { contentHash } : {}),
    };
  } catch {
    return undefined;
  }
}

function parseFeedback(value: unknown): ReputationSignal | undefined {
  try {
    const signal = record(value, "feedback");
    const rawValue = boundedText(signal.value, "feedback value", 48);
    if (!SIGNED.test(rawValue)) return undefined;
    const valueDecimals = number(signal.valueDecimals, "feedback valueDecimals");
    if (!Number.isInteger(valueDecimals) || valueDecimals < 0 || valueDecimals > 18) return undefined;
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
      createdAt: unsigned(signal.createdTimestamp, "feedback createdTimestamp"),
      revoked: boolean(signal.isRevoked, "feedback isRevoked"),
      provenance: boundedText(signal.provenance, "feedback provenance", 512),
    };
  } catch {
    return undefined;
  }
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
      earliestRegistrationBlock(a),
      earliestRegistrationBlock(b),
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

function earliestRegistrationBlock(group: RankedWallet): string {
  return group.identities.reduce(
    (earliest, identity) =>
      compareUnsigned(identity.registrationBlock, earliest) < 0
        ? identity.registrationBlock
        : earliest,
    group.identities[0].registrationBlock,
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
