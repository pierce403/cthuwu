import {
  ALLEGIANCE_HEX,
  BASE_CHAIN_ID,
  BASE_NAME,
  IDENTITY_REGISTRY,
  PROTOCOL_V1_HEX,
  REPUTATION_REGISTRY,
  UWU_CONTRACT,
  type LeaderboardSnapshot,
} from "./leaderboard-types";

export function cachedSnapshot(name = "Cache Tentacle"): LeaderboardSnapshot {
  const identity = {
    agentId: "1",
    owner: "0x3333333333333333333333333333333333333333",
    agentUri: "",
    agentWallet: "0x1111111111111111111111111111111111111111",
    allegianceHex: ALLEGIANCE_HEX,
    protocolHex: PROTOCOL_V1_HEX,
    tentacleId: "tentacle_cache",
    registrationBlock: "100",
    registrationTimestamp: "1700000000",
    profileUpdatedBlock: "101",
    profileUpdatedTimestamp: "1700000100",
    metadataUpdatedBlock: "102",
    metadataUpdatedTimestamp: "1700000200",
    rawBalance: "1000000000000000000",
    balanceUpdatedBlock: "103",
    balanceUpdatedTimestamp: "1700000300",
    profile: { name, active: true, sourceUri: "cached" },
    reputationCounters: { active: "0", sampledRevoked: "0" },
    reputation: [],
  };
  return {
    cacheSchemaVersion: 1,
    network: BASE_NAME,
    chainId: BASE_CHAIN_ID,
    identityRegistry: IDENTITY_REGISTRY,
    reputationRegistry: REPUTATION_REGISTRY,
    uwuContract: UWU_CONTRACT,
    sourceDeployment: "QmDeployment",
    sourceBlockNumber: "42000000",
    sourceBlockHash: `0x${"ab".repeat(32)}`,
    hasIndexingErrors: false,
    fetchedAt: "2026-08-11T12:00:00.000Z",
    paginationComplete: true,
    rankedWallets: [
      {
        wallet: identity.agentWallet,
        rawBalance: identity.rawBalance,
        representativeAgentId: "1",
        rank: 1,
        identities: [identity],
      },
    ],
    suspended: [],
  };
}
