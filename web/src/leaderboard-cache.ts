import {
  ALLEGIANCE_HEX,
  BASE_CHAIN_ID,
  BASE_NAME,
  IDENTITY_REGISTRY,
  LEADERBOARD_CACHE_KEY,
  LEADERBOARD_CACHE_VERSION,
  REPUTATION_REGISTRY,
  UWU_CONTRACT,
  ZERO_ADDRESS,
  type LeaderboardSnapshot,
  type TentacleIdentity,
} from "./leaderboard-types";
import { parseRawBalance } from "./level";

const MAX_CACHE_BYTES = 2 * 1024 * 1024;
const ADDRESS = /^0x[0-9a-f]{40}$/u;
const BYTES = /^0x(?:[0-9a-f]{2})*$/u;
const UNSIGNED = /^(0|[1-9][0-9]*)$/u;
const SIGNED = /^(0|-?[1-9][0-9]*)$/u;

export function readLeaderboardCache(storage: Storage): LeaderboardSnapshot | undefined {
  let raw: string | null;
  try {
    raw = storage.getItem(LEADERBOARD_CACHE_KEY);
  } catch {
    return undefined;
  }
  if (raw === null) return undefined;
  if (new TextEncoder().encode(raw).length > MAX_CACHE_BYTES) {
    discardLeaderboardCache(storage);
    return undefined;
  }
  try {
    const parsed = JSON.parse(raw) as unknown;
    return validateSnapshot(parsed);
  } catch {
    discardLeaderboardCache(storage);
    return undefined;
  }
}

export function writeLeaderboardCache(storage: Storage, snapshot: LeaderboardSnapshot): boolean {
  const cacheSafe = validateSnapshot(withoutRegistrationDocuments(snapshot));
  const full = serializeWithinLimit(cacheSafe);
  if (full !== undefined) {
    try {
      storage.setItem(LEADERBOARD_CACHE_KEY, full);
      return true;
    } catch {
      // A compact retry can still fit when the storage area is near its quota.
    }
  }
  const compact = serializeWithinLimit(compactSnapshot(cacheSafe));
  if (compact === undefined) return false;
  try {
    storage.setItem(LEADERBOARD_CACHE_KEY, compact);
    return true;
  } catch {
    // localStorage.setItem is atomic: the previous validated value remains untouched.
    return false;
  }
}

function serializeWithinLimit(snapshot: LeaderboardSnapshot): string | undefined {
  const serialized = JSON.stringify(snapshot);
  return new TextEncoder().encode(serialized).length <= MAX_CACHE_BYTES
    ? serialized
    : undefined;
}

function withoutRegistrationDocuments(snapshot: LeaderboardSnapshot): LeaderboardSnapshot {
  const sanitizeIdentity = (identity: TentacleIdentity): TentacleIdentity => ({
    ...identity,
    agentUri: "",
  });
  return {
    ...snapshot,
    rankedWallets: snapshot.rankedWallets.map((group) => ({
      ...group,
      identities: group.identities.map(sanitizeIdentity),
    })),
    suspended: snapshot.suspended.map(sanitizeIdentity),
  };
}

export function isSnapshotStale(
  snapshot: LeaderboardSnapshot,
  freshnessMs: number,
  now = Date.now(),
): boolean {
  const fetchedAt = Date.parse(snapshot.fetchedAt);
  return !Number.isFinite(fetchedAt) || now - fetchedAt > freshnessMs;
}

function discardLeaderboardCache(storage: Storage): void {
  try {
    storage.removeItem(LEADERBOARD_CACHE_KEY);
  } catch {
    // An unavailable storage area simply means there is no usable cache.
  }
}

function validateSnapshot(value: unknown): LeaderboardSnapshot {
  const snapshot = object(value, "leaderboard snapshot") as unknown as LeaderboardSnapshot;
  if (
    snapshot.cacheSchemaVersion !== LEADERBOARD_CACHE_VERSION ||
    snapshot.network !== BASE_NAME ||
    snapshot.chainId !== BASE_CHAIN_ID ||
    snapshot.identityRegistry !== IDENTITY_REGISTRY ||
    snapshot.reputationRegistry !== REPUTATION_REGISTRY ||
    snapshot.uwuContract !== UWU_CONTRACT ||
    snapshot.hasIndexingErrors !== false ||
    snapshot.paginationComplete !== true ||
    !safeText(snapshot.sourceDeployment, 256) ||
    !unsigned(snapshot.sourceBlockNumber) ||
    (snapshot.sourceBlockHash !== undefined && !fixedBytes(snapshot.sourceBlockHash, 32)) ||
    (snapshot.sourceBlockTimestamp !== undefined && !unsigned(snapshot.sourceBlockTimestamp)) ||
    !Number.isFinite(Date.parse(snapshot.fetchedAt)) ||
    !Array.isArray(snapshot.rankedWallets) ||
    !Array.isArray(snapshot.suspended) ||
    snapshot.rankedWallets.length > 100_000 ||
    snapshot.suspended.length > 100_000
  ) {
    throw new Error("cached leaderboard metadata is invalid");
  }
  const agentIds = new Set<string>();
  const wallets = new Set<string>();
  let previousBalance: bigint | undefined;
  let previousGroup: (typeof snapshot.rankedWallets)[number] | undefined;
  let expectedRank = 0;
  for (const group of snapshot.rankedWallets) {
    object(group, "ranked wallet");
    if (
      !ADDRESS.test(group.wallet) ||
      group.wallet === ZERO_ADDRESS ||
      wallets.has(group.wallet)
    ) {
      throw new Error("invalid wallet group");
    }
    wallets.add(group.wallet);
    const balance = parseRawBalance(group.rawBalance);
    if (previousBalance !== undefined && balance > previousBalance) throw new Error("cache is not ranked");
    if (!Array.isArray(group.identities) || group.identities.length === 0 || group.identities.length > 1_000) {
      throw new Error("invalid wallet identities");
    }
    for (const identity of group.identities) {
      validateIdentity(identity, agentIds, group.wallet, group.rawBalance, false);
    }
    if (group.representativeAgentId !== group.identities[0].agentId) {
      throw new Error("invalid shared-wallet representative");
    }
    for (let index = 1; index < group.identities.length; index += 1) {
      if (BigInt(group.identities[index - 1].agentId) >= BigInt(group.identities[index].agentId)) {
        throw new Error("shared-wallet identities are not ordered");
      }
    }
    if (previousGroup && balance === previousBalance) {
      const previousRegistration = earliestRegistrationBlock(previousGroup.identities);
      const registration = earliestRegistrationBlock(group.identities);
      if (
        registration < previousRegistration ||
        (registration === previousRegistration &&
          BigInt(group.representativeAgentId) < BigInt(previousGroup.representativeAgentId))
      ) {
        throw new Error("cache tie-break order is invalid");
      }
    }
    if (balance > 0n) {
      expectedRank += 1;
      if (group.rank !== expectedRank) throw new Error("invalid rank");
    } else if (group.rank !== undefined) {
      throw new Error("unfunded wallet has a numeric rank");
    }
    previousBalance = balance;
    previousGroup = group;
  }
  for (const identity of snapshot.suspended) {
    validateIdentity(identity, agentIds, ZERO_ADDRESS, "0", true);
  }
  return snapshot;
}

function earliestRegistrationBlock(identities: TentacleIdentity[]): bigint {
  return identities.reduce((earliest, identity) => {
    const block = BigInt(identity.registrationBlock);
    return block < earliest ? block : earliest;
  }, BigInt(identities[0].registrationBlock));
}

function validateIdentity(
  identity: TentacleIdentity,
  ids: Set<string>,
  expectedWallet: string,
  expectedBalance: string,
  suspended: boolean,
): void {
  object(identity, "Tentacle identity");
  if (
    !unsigned(identity.agentId) ||
    ids.has(identity.agentId) ||
    !ADDRESS.test(identity.owner) ||
    identity.agentWallet !== expectedWallet ||
    identity.allegianceHex !== ALLEGIANCE_HEX ||
    !bytes(identity.protocolHex, 256) ||
    identity.agentUri !== "" ||
    (identity.tentacleId !== undefined && !safeText(identity.tentacleId, 96)) ||
    identity.rawBalance !== expectedBalance ||
    !unsigned(identity.registrationBlock) ||
    !unsigned(identity.registrationTimestamp) ||
    !unsigned(identity.profileUpdatedBlock) ||
    !unsigned(identity.profileUpdatedTimestamp) ||
    !unsigned(identity.metadataUpdatedBlock) ||
    !unsigned(identity.metadataUpdatedTimestamp) ||
    !validProfile(identity.profile) ||
    !validReputationCounters(identity.reputationCounters, identity.reputation) ||
    !Array.isArray(identity.reputation) ||
    identity.reputation.length > 10 ||
    !identity.reputation.every(validReputation)
  ) {
    throw new Error("invalid cached Tentacle");
  }
  if (suspended && identity.agentWallet !== ZERO_ADDRESS) throw new Error("invalid suspended Tentacle");
  parseRawBalance(identity.rawBalance);
  ids.add(identity.agentId);
}

function validReputationCounters(value: unknown, sample: unknown): boolean {
  try {
    const counters = object(value, "reputation counters");
    if (
      !unsigned(counters.total) ||
      !unsigned(counters.active) ||
      !unsigned(counters.revoked) ||
      BigInt(counters.total) >= 1n << 256n ||
      BigInt(counters.active) + BigInt(counters.revoked) !== BigInt(counters.total) ||
      !Array.isArray(sample)
    ) return false;
    const active = sample.filter((signal) => {
      try {
        return object(signal, "reputation signal").revoked === false;
      } catch {
        return false;
      }
    }).length;
    const revoked = sample.filter((signal) => {
      try {
        return object(signal, "reputation signal").revoked === true;
      } catch {
        return false;
      }
    }).length;
    return (
      BigInt(sample.length) <= BigInt(counters.total) &&
      BigInt(active) <= BigInt(counters.active) &&
      BigInt(revoked) <= BigInt(counters.revoked)
    );
  } catch {
    return false;
  }
}

function compactSnapshot(snapshot: LeaderboardSnapshot): LeaderboardSnapshot {
  const compactIdentity = (identity: TentacleIdentity): TentacleIdentity => {
    const { balanceUpdatedBlock: _, balanceUpdatedTimestamp: __, ...required } = identity;
    return {
      ...required,
      agentUri: "",
      profile: { name: identity.profile.name, active: identity.profile.active, sourceUri: "cached" },
      reputation: [],
    };
  };
  return {
    ...snapshot,
    rankedWallets: snapshot.rankedWallets.map((group) => ({
      ...group,
      identities: group.identities.map(compactIdentity),
    })),
    suspended: snapshot.suspended.map(compactIdentity),
  };
}

function object(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} is invalid`);
  }
  return value as Record<string, unknown>;
}

function safeText(value: unknown, maximum: number): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= maximum &&
    !hasControl(value)
  );
}

function unsigned(value: unknown): value is string {
  return typeof value === "string" && value.length <= 78 && UNSIGNED.test(value);
}

function validProfile(value: unknown): boolean {
  const profile = object(value, "profile");
  return Boolean(
    safeText(profile.name, 128) &&
      typeof profile.active === "boolean" &&
      safeText(profile.sourceUri, 2_048) &&
      (profile.description === undefined || safeText(profile.description, 512)) &&
      (profile.image === undefined || safePublicUri(profile.image)) &&
      (profile.xmtpEndpoint === undefined ||
        (typeof profile.xmtpEndpoint === "string" &&
          /^xmtp:\/\/[0-9a-f]{64}$/u.test(profile.xmtpEndpoint))) &&
      (profile.cthuwuEndpoint === undefined || safePublicUri(profile.cthuwuEndpoint)) &&
      (profile.contentHash === undefined || bytes(profile.contentHash, 64)),
  );
}

function validReputation(value: unknown): boolean {
  try {
    const signal = object(value, "reputation signal");
    return Boolean(
      safeText(signal.id, 256) &&
        ADDRESS.test(String(signal.clientAddress)) &&
        typeof signal.value === "string" &&
        signal.value.length <= 48 &&
        SIGNED.test(signal.value) &&
        Number.isSafeInteger(signal.valueDecimals) &&
        Number(signal.valueDecimals) >= 0 &&
        Number(signal.valueDecimals) <= 18 &&
        (signal.tag1 === undefined || safeText(signal.tag1, 128)) &&
        (signal.tag2 === undefined || safeText(signal.tag2, 128)) &&
        (signal.endpoint === undefined || safePublicUri(signal.endpoint)) &&
        unsigned(signal.createdAt) &&
        typeof signal.revoked === "boolean" &&
        safeText(signal.provenance, 512),
    );
  } catch {
    return false;
  }
}

function safePublicUri(value: unknown): boolean {
  if (!safeText(value, 2_048)) return false;
  if (value.startsWith("ipfs://") || value.startsWith("ar://")) return true;
  try {
    const parsed = new URL(value);
    return parsed.protocol === "https:" && !parsed.username && !parsed.password;
  } catch {
    return false;
  }
}

function bytes(value: unknown, maximumBytes: number): value is string {
  return (
    typeof value === "string" &&
    value.length <= maximumBytes * 2 + 2 &&
    BYTES.test(value)
  );
}

function fixedBytes(value: unknown, size: number): value is string {
  return typeof value === "string" && value.length === size * 2 + 2 && BYTES.test(value);
}

function hasControl(value: string): boolean {
  return [...value].some((character) => {
    const code = character.codePointAt(0) ?? 0;
    return (
      code <= 0x1f ||
      (code >= 0x7f && code <= 0x9f) ||
      code === 0x061c ||
      (code >= 0x200b && code <= 0x200f) ||
      (code >= 0x202a && code <= 0x202e) ||
      (code >= 0x2060 && code <= 0x206f) ||
      code === 0xfeff
    );
  });
}
