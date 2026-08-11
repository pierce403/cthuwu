const CACHE_KEY = "cthuwu:leaderboard:v1";
const CHAIN_ID = 8453;
const NETWORK = "Base mainnet";
const REGISTRY = "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432";
const REPUTATION = "0x8004baa17c55a88189ae136b182e5fda19de9b63";
const UWU = "0x9dba3ae7002daefd7324e7b9f829ed31cb5f0b07";
const ZERO_ADDRESS = "0x0000000000000000000000000000000000000000";
const ALLEGIANCE = "0x7577752d74656e7461636c652d7631";
const MAX_CACHE_BYTES = 2 * 1024 * 1024;

const source = document.querySelector("#offline-source");
const list = document.querySelector("#offline-tentacles");
document.querySelector("#offline-retry")?.addEventListener("click", () => location.reload());

try {
  const raw = localStorage.getItem(CACHE_KEY);
  const snapshot = raw && utf8ByteLength(raw) <= MAX_CACHE_BYTES ? JSON.parse(raw) : undefined;
  if (!validSnapshot(snapshot)) throw new Error("invalid snapshot");
  source.textContent = `Base ${CHAIN_ID} · block ${snapshot.sourceBlockNumber} · saved ${new Date(snapshot.fetchedAt).toLocaleString()}`;
  for (const group of snapshot.rankedWallets.slice(0, 25)) {
    const item = document.createElement("li");
    const name = document.createElement("strong");
    const profileName = group.identities?.[0]?.profile?.name;
    name.textContent = safeText(profileName, 128) ? profileName : "Tentacle";
    const level = document.createElement("span");
    level.textContent = BigInt(group.rawBalance) === 0n ? "UNFUNDED" : `Level ${levelOf(group.rawBalance)}`;
    item.append(name, level);
    list.append(item);
  }
  if (snapshot.rankedWallets.length === 0) source.textContent += " · no ranked Tentacles";
} catch {
  try {
    localStorage.removeItem(CACHE_KEY);
  } catch {
    // Storage may be unavailable; no other local application state is touched.
  }
  source.textContent = "No validated leaderboard snapshot is stored on this device.";
}

function validSnapshot(value) {
  try {
    return Boolean(
      plainObject(value) &&
        value.cacheSchemaVersion === 1 &&
        value.network === NETWORK &&
        value.chainId === CHAIN_ID &&
        value.identityRegistry === REGISTRY &&
        value.reputationRegistry === REPUTATION &&
        value.uwuContract === UWU &&
        value.hasIndexingErrors === false &&
        value.paginationComplete === true &&
        safeText(value.sourceDeployment, 256) &&
        unsigned(value.sourceBlockNumber, 78) &&
        (value.sourceBlockHash === undefined || fixedBytes(value.sourceBlockHash, 32)) &&
        (value.sourceBlockTimestamp === undefined || unsigned(value.sourceBlockTimestamp, 32)) &&
        Number.isFinite(Date.parse(value.fetchedAt)) &&
        validContents(value),
    );
  } catch {
    return false;
  }
}

function validContents(snapshot) {
  if (
    !Array.isArray(snapshot.rankedWallets) ||
    !Array.isArray(snapshot.suspended) ||
    snapshot.rankedWallets.length > 100000 ||
    snapshot.suspended.length > 100000
  ) return false;
  const wallets = new Set();
  const agentIds = new Set();
  let previousBalance;
  let previousRegistration;
  let previousRepresentative;
  let expectedRank = 0;
  for (const group of snapshot.rankedWallets) {
    if (
      !plainObject(group) ||
      !address(group.wallet) ||
      group.wallet === ZERO_ADDRESS ||
      wallets.has(group.wallet) ||
      !uint256(group.rawBalance) ||
      !Array.isArray(group.identities) ||
      group.identities.length === 0 ||
      group.identities.length > 1000
    ) return false;
    wallets.add(group.wallet);
    for (let index = 0; index < group.identities.length; index += 1) {
      const identity = group.identities[index];
      if (!validIdentity(identity, group.wallet, group.rawBalance, agentIds)) return false;
      if (
        index > 0 &&
        BigInt(group.identities[index - 1].agentId) >= BigInt(identity.agentId)
      ) return false;
    }
    if (group.representativeAgentId !== group.identities[0].agentId) return false;
    const balance = BigInt(group.rawBalance);
    const registration = group.identities.reduce(
      (earliest, identity) =>
        BigInt(identity.registrationBlock) < earliest
          ? BigInt(identity.registrationBlock)
          : earliest,
      BigInt(group.identities[0].registrationBlock),
    );
    if (previousBalance !== undefined && balance > previousBalance) return false;
    if (
      previousBalance !== undefined &&
      balance === previousBalance &&
      (registration < previousRegistration ||
        (registration === previousRegistration &&
          BigInt(group.representativeAgentId) < previousRepresentative))
    ) return false;
    if (balance > 0n) {
      expectedRank += 1;
      if (group.rank !== expectedRank) return false;
    } else if (group.rank !== undefined) {
      return false;
    }
    previousBalance = balance;
    previousRegistration = registration;
    previousRepresentative = BigInt(group.representativeAgentId);
  }
  return snapshot.suspended.every((identity) =>
    validIdentity(identity, ZERO_ADDRESS, "0", agentIds),
  );
}

function validIdentity(identity, expectedWallet, expectedBalance, agentIds) {
  if (
    !plainObject(identity) ||
    !unsigned(identity.agentId, 78) ||
    agentIds.has(identity.agentId) ||
    !address(identity.owner) ||
    identity.agentWallet !== expectedWallet ||
    identity.allegianceHex !== ALLEGIANCE ||
    !bytes(identity.protocolHex, 256) ||
    identity.agentUri !== "" ||
    (identity.tentacleId !== undefined && !safeText(identity.tentacleId, 96)) ||
    identity.rawBalance !== expectedBalance ||
    !unsigned(identity.registrationBlock, 32) ||
    !unsigned(identity.registrationTimestamp, 32) ||
    !unsigned(identity.profileUpdatedBlock, 32) ||
    !unsigned(identity.profileUpdatedTimestamp, 32) ||
    !unsigned(identity.metadataUpdatedBlock, 32) ||
    !unsigned(identity.metadataUpdatedTimestamp, 32) ||
    !validProfile(identity.profile) ||
    !validReputationCounters(identity.reputationCounters, identity.reputation) ||
    !Array.isArray(identity.reputation) ||
    identity.reputation.length > 10
  ) return false;
  agentIds.add(identity.agentId);
  return true;
}

function validReputationCounters(counters, sample) {
  if (
    !plainObject(counters) ||
    !uint256(counters.total) ||
    !uint256(counters.active) ||
    !uint256(counters.revoked) ||
    !Array.isArray(sample)
  ) return false;
  const active = sample.filter((signal) => plainObject(signal) && signal.revoked === false).length;
  const revoked = sample.filter((signal) => plainObject(signal) && signal.revoked === true).length;
  return (
    BigInt(counters.active) + BigInt(counters.revoked) === BigInt(counters.total) &&
    BigInt(sample.length) <= BigInt(counters.total) &&
    BigInt(active) <= BigInt(counters.active) &&
    BigInt(revoked) <= BigInt(counters.revoked)
  );
}

function validProfile(profile) {
  return Boolean(
    plainObject(profile) &&
      safeText(profile.name, 128) &&
      typeof profile.active === "boolean" &&
      safeText(profile.sourceUri, 2048) &&
      (profile.description === undefined || safeText(profile.description, 512)) &&
      (profile.xmtpEndpoint === undefined || /^xmtp:\/\/[0-9a-f]{64}$/.test(profile.xmtpEndpoint)),
  );
}

function plainObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function address(value) {
  return typeof value === "string" && /^0x[0-9a-f]{40}$/.test(value);
}

function unsigned(value, maximum) {
  return (
    typeof value === "string" &&
    value.length <= maximum &&
    /^(0|[1-9][0-9]*)$/.test(value)
  );
}

function uint256(value) {
  return unsigned(value, 78) && BigInt(value) < 2n ** 256n;
}

function bytes(value, maximumBytes) {
  return (
    typeof value === "string" &&
    value.length <= maximumBytes * 2 + 2 &&
    /^0x(?:[0-9a-f]{2})*$/.test(value)
  );
}

function fixedBytes(value, size) {
  return typeof value === "string" && value.length === size * 2 + 2 && bytes(value, size);
}

function safeText(value, maximum) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= maximum &&
    ![...value].some((character) => {
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
    })
  );
}

function levelOf(raw) {
  const digits = BigInt(raw).toString();
  const significant = digits.slice(0, 15).padEnd(15, "0");
  const mantissa = Number(`${significant[0]}.${significant.slice(1)}`);
  const level = digits.length - 19 + Math.log10(mantissa);
  const rendered = level.toFixed(2);
  return rendered === "-0.00" ? "0.00" : rendered;
}

function utf8ByteLength(value) {
  let bytes = 0;
  for (const character of value) {
    const code = character.codePointAt(0) ?? 0;
    bytes += code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
  }
  return bytes;
}
