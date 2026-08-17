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
  type DuplicateAgentAlias,
  type LeaderboardSnapshot,
  type TentacleIdentity,
} from "./leaderboard-types";
import { parseRawBalance } from "./level";
import { canonicalizeWalletIdentities } from "./tentacle-canonical";

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

function duplicateAliases(value: unknown, label: string): DuplicateAgentAlias[] {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.length > 100_000) {
    throw new Error(`${label} is invalid`);
  }
  const aliases = value.map((entry) => {
    const alias = object(entry, label);
    if (
      Object.keys(alias).length !== 2 ||
      !("aliasAgentId" in alias) ||
      !("canonicalAgentId" in alias) ||
      !unsigned(alias.aliasAgentId) ||
      !unsigned(alias.canonicalAgentId) ||
      alias.aliasAgentId === alias.canonicalAgentId ||
      BigInt(alias.aliasAgentId) <= BigInt(alias.canonicalAgentId)
    ) {
      throw new Error(`${label} is invalid`);
    }
    return {
      aliasAgentId: alias.aliasAgentId,
      canonicalAgentId: alias.canonicalAgentId,
    };
  });
  if (new Set(aliases.map(({ aliasAgentId }) => aliasAgentId)).size !== aliases.length) {
    throw new Error(`${label} contains duplicate aliases`);
  }
  return aliases.sort((left, right) =>
    BigInt(left.aliasAgentId) < BigInt(right.aliasAgentId) ? -1 : 1);
}

function normalizeDuplicateComponents(snapshot: LeaderboardSnapshot): void {
  const rawIds = new Set<string>();
  const wallets = new Set<string>();
  for (const group of snapshot.rankedWallets) {
    object(group, "ranked wallet");
    if (
      !ADDRESS.test(group.wallet) ||
      group.wallet === ZERO_ADDRESS ||
      wallets.has(group.wallet) ||
      !Array.isArray(group.identities) ||
      group.identities.length === 0 ||
      group.identities.length > 1_000
    ) throw new Error("invalid wallet group");
    wallets.add(group.wallet);
    for (const identity of group.identities) {
      validateIdentity(identity, rawIds, group.wallet, group.rawBalance, false);
    }
  }
  for (const identity of snapshot.suspended) {
    validateIdentity(identity, rawIds, ZERO_ADDRESS, "0", true);
  }

  const canonical = canonicalizeWalletIdentities([
    ...snapshot.rankedWallets.flatMap((group) => group.identities),
    ...snapshot.suspended,
  ]);
  const aliasesById = new Map<string, string>();
  const suppliedAliases = [
    ...duplicateAliases(snapshot.duplicateAgentAliases, "snapshot duplicate aliases"),
  ];
  for (const group of snapshot.rankedWallets) {
    const explicitGroupAliases = duplicateAliases(
      group.duplicateAgentAliases,
      "wallet duplicate aliases",
    );
    const rawGroupIds = new Set(group.identities.map(({ agentId }) => agentId));
    for (const alias of explicitGroupAliases) {
      if (!rawGroupIds.has(alias.canonicalAgentId)) {
        throw new Error("wallet duplicate alias targets another wallet component");
      }
    }
    suppliedAliases.push(...explicitGroupAliases);
    const rawIgnored = group.ignoredDuplicateAgentIds;
    if (rawIgnored === undefined) continue;
    if (
      !Array.isArray(rawIgnored) ||
      rawIgnored.length > 1_000 ||
      rawIgnored.some((id, index, all) => !unsigned(id) || all.indexOf(id) !== index)
    ) throw new Error("invalid ignored duplicate identities");
    if (group.duplicateAgentAliases !== undefined) continue;
    const groupCanonical = canonicalizeWalletIdentities(group.identities).identities;
    // Version-one caches predate explicit component mappings. They can be migrated
    // only when the wallet contains one canonical Tentacle; otherwise mapping every
    // alias to the wallet representative would conflate unrelated Tentacles.
    if (rawIgnored.length > 0 && groupCanonical.length !== 1) {
      throw new Error("legacy duplicate aliases are ambiguous on a shared wallet");
    }
    suppliedAliases.push(...rawIgnored.map((aliasAgentId) => ({
      aliasAgentId,
      canonicalAgentId: groupCanonical[0]!.agentId,
    })));
  }
  if (suppliedAliases.some(({ aliasAgentId }) => rawIds.has(aliasAgentId))) {
    throw new Error("ignored duplicate identity is also present as a raw identity");
  }
  for (const alias of [...suppliedAliases, ...canonical.duplicateAgentAliases]) {
    const existing = aliasesById.get(alias.aliasAgentId);
    if (existing !== undefined && existing !== alias.canonicalAgentId) {
      throw new Error("duplicate alias maps to multiple canonical identities");
    }
    aliasesById.set(alias.aliasAgentId, alias.canonicalAgentId);
  }
  const canonicalIds = new Set(canonical.identities.map(({ agentId }) => agentId));
  const aliases = [...aliasesById].map(([aliasAgentId, canonicalAgentId]) => {
    if (!canonicalIds.has(canonicalAgentId) || canonicalIds.has(aliasAgentId)) {
      throw new Error("duplicate alias does not resolve to one canonical identity");
    }
    return { aliasAgentId, canonicalAgentId };
  }).sort((left, right) =>
    BigInt(left.aliasAgentId) < BigInt(right.aliasAgentId) ? -1 : 1);

  snapshot.rankedWallets = snapshot.rankedWallets.flatMap((group) => {
    group.identities = group.identities.filter(({ agentId }) => canonicalIds.has(agentId));
    if (group.identities.length === 0) return [];
    group.identities.sort((left, right) =>
      BigInt(left.agentId) < BigInt(right.agentId) ? -1 : 1);
    group.representativeAgentId = group.identities[0]!.agentId;
    const groupIds = new Set(group.identities.map(({ agentId }) => agentId));
    const groupAliases = aliases.filter(({ canonicalAgentId }) => groupIds.has(canonicalAgentId));
    if (groupAliases.length > 0) {
      group.duplicateAgentAliases = groupAliases;
      group.ignoredDuplicateAgentIds = groupAliases.map(({ aliasAgentId }) => aliasAgentId);
    } else {
      delete group.duplicateAgentAliases;
      delete group.ignoredDuplicateAgentIds;
    }
    return [group];
  });
  snapshot.suspended = snapshot.suspended.filter(({ agentId }) => canonicalIds.has(agentId));
  snapshot.suspended.sort((left, right) =>
    BigInt(left.agentId) < BigInt(right.agentId) ? -1 : 1);
  if (aliases.length > 0) snapshot.duplicateAgentAliases = aliases;
  else delete snapshot.duplicateAgentAliases;
  let rank = 0;
  for (const group of snapshot.rankedWallets) {
    if (BigInt(group.rawBalance) > 0n) group.rank = ++rank;
    else delete group.rank;
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
  normalizeDuplicateComponents(snapshot);
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
    if (!Array.isArray(group.identities) || group.identities.length === 0 || group.identities.length > 1_000) {
      throw new Error("invalid wallet identities");
    }
    const originalAgentIds = new Set<string>();
    for (const identity of group.identities) {
      validateIdentity(identity, originalAgentIds, group.wallet, group.rawBalance, false);
    }
    if (!originalAgentIds.has(group.representativeAgentId)) {
      throw new Error("shared-wallet representative is not a member identity");
    }
    const canonical = canonicalizeWalletIdentities(group.identities);
    if (canonical.ignoredDuplicateAgentIds.length > 0) {
      throw new Error("wallet identities were not globally canonicalized");
    }
    group.identities = canonical.identities;
    const groupAliases = duplicateAliases(group.duplicateAgentAliases, "wallet duplicate aliases");
    if (groupAliases.length > 1_000) throw new Error("too many wallet duplicate aliases");
    const ignoredDuplicateAgentIds = groupAliases.map(({ aliasAgentId }) => aliasAgentId);
    const rawIgnored = group.ignoredDuplicateAgentIds ?? [];
    if (
      rawIgnored.length !== ignoredDuplicateAgentIds.length ||
      rawIgnored.some((id, index) => id !== ignoredDuplicateAgentIds[index])
    ) throw new Error("ignored duplicate diagnostics do not match component aliases");
    const memberIds = new Set(group.identities.map(({ agentId }) => agentId));
    for (const { aliasAgentId, canonicalAgentId } of groupAliases) {
      if (!memberIds.has(canonicalAgentId) || memberIds.has(aliasAgentId)) {
        throw new Error("wallet duplicate alias has an invalid component target");
      }
    }
    const balance = parseRawBalance(group.rawBalance);
    if (previousBalance !== undefined && balance > previousBalance) throw new Error("cache is not ranked");
    if (
      group.ignoredDuplicateAgentIds !== undefined &&
      (!Array.isArray(group.ignoredDuplicateAgentIds) ||
      group.ignoredDuplicateAgentIds.length > 1_000 ||
      group.ignoredDuplicateAgentIds.some((id, index, ids) =>
        !unsigned(id) ||
        ids.indexOf(id) !== index ||
        group.identities.some((identity) => identity.agentId === id)))
    ) throw new Error("invalid ignored duplicate identities");
    const reservedAgentIds = new Set([...originalAgentIds, ...ignoredDuplicateAgentIds]);
    for (const reservedAgentId of reservedAgentIds) {
      if (agentIds.has(reservedAgentId)) throw new Error("duplicate agent identity");
      agentIds.add(reservedAgentId);
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
      const previousRegistration = earliestRegistrationTimestamp(previousGroup.identities);
      const registration = earliestRegistrationTimestamp(group.identities);
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
  const snapshotAliases = duplicateAliases(snapshot.duplicateAgentAliases, "snapshot duplicate aliases");
  const targetIds = new Set([
    ...snapshot.rankedWallets.flatMap((group) => group.identities.map(({ agentId }) => agentId)),
    ...snapshot.suspended.map(({ agentId }) => agentId),
  ]);
  for (const { aliasAgentId, canonicalAgentId } of snapshotAliases) {
    if (!targetIds.has(canonicalAgentId) || targetIds.has(aliasAgentId)) {
      throw new Error("snapshot duplicate alias has an invalid component target");
    }
    if (!agentIds.has(aliasAgentId)) agentIds.add(aliasAgentId);
  }
  return snapshot;
}

function earliestRegistrationTimestamp(identities: TentacleIdentity[]): bigint {
  return identities.reduce((earliest, identity) => {
    const block = BigInt(identity.registrationTimestamp);
    return block < earliest ? block : earliest;
  }, BigInt(identities[0].registrationTimestamp));
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
      !unsigned(counters.active) ||
      !unsigned(counters.sampledRevoked) ||
      BigInt(counters.active) >= 1n << 256n ||
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
      BigInt(active) <= BigInt(counters.active) &&
      BigInt(revoked) <= BigInt(counters.sampledRevoked)
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
