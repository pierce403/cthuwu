export const BASE_CHAIN_ID = 8453;
export const BASE_NAME = "Base mainnet";
export const BASE_EXPLORER = "https://basescan.org";
export const IDENTITY_REGISTRY = "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432";
export const REPUTATION_REGISTRY = "0x8004baa17c55a88189ae136b182e5fda19de9b63";
export const UWU_CONTRACT = "0x9dba3ae7002daefd7324e7b9f829ed31cb5f0b07";
export const UWU_DECIMALS = 18;
export const ALLEGIANCE_HEX = "0x7577752d74656e7461636c652d7631";
export const PROTOCOL_V1_HEX = "0x31";
export const ZERO_ADDRESS = "0x0000000000000000000000000000000000000000";
export const LEADERBOARD_CACHE_KEY = "cthuwu:leaderboard:v1";
export const LEADERBOARD_CACHE_VERSION = 1;

export interface TentacleProfile {
  name: string;
  description?: string;
  image?: string;
  active: boolean;
  xmtpEndpoint?: string;
  cthuwuEndpoint?: string;
  sourceUri: string;
  contentHash?: string;
}

export interface ReputationSignal {
  id: string;
  clientAddress: string;
  value: string;
  valueDecimals: number;
  tag1?: string;
  tag2?: string;
  endpoint?: string;
  createdAt: string;
  revoked: boolean;
  provenance: string;
}

export interface ReputationCounters {
  total: string;
  active: string;
  revoked: string;
}

export interface TentacleIdentity {
  agentId: string;
  owner: string;
  agentUri: string;
  agentWallet: string;
  allegianceHex: string;
  protocolHex: string;
  tentacleId?: string;
  registrationBlock: string;
  registrationTimestamp: string;
  profileUpdatedBlock: string;
  profileUpdatedTimestamp: string;
  metadataUpdatedBlock: string;
  metadataUpdatedTimestamp: string;
  rawBalance: string;
  balanceUpdatedBlock?: string;
  balanceUpdatedTimestamp?: string;
  profile: TentacleProfile;
  reputationCounters: ReputationCounters;
  reputation: ReputationSignal[];
}

export interface RankedWallet {
  wallet: string;
  rawBalance: string;
  representativeAgentId: string;
  identities: TentacleIdentity[];
  rank?: number;
}

export interface LeaderboardSnapshot {
  cacheSchemaVersion: 1;
  network: typeof BASE_NAME;
  chainId: typeof BASE_CHAIN_ID;
  identityRegistry: typeof IDENTITY_REGISTRY;
  reputationRegistry: typeof REPUTATION_REGISTRY;
  uwuContract: typeof UWU_CONTRACT;
  sourceDeployment: string;
  sourceBlockNumber: string;
  sourceBlockHash?: string;
  sourceBlockTimestamp?: string;
  hasIndexingErrors: false;
  fetchedAt: string;
  paginationComplete: true;
  rankedWallets: RankedWallet[];
  suspended: TentacleIdentity[];
}

export type LeaderboardState =
  | "CURRENT"
  | "OFFLINE"
  | "STALE"
  | "REFRESHING"
  | "INDEXING ERROR"
  | "UNAVAILABLE";
