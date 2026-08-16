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
export const AGENT0_BASE_SUBGRAPH_ID = "43s9hQRurMGjuYnC1r2ZwS6xSQktbFyXMPMqGKUFJojb";
// Public browser credential. Its security boundary is Graph origin/subgraph/spend restrictions,
// not secrecy; every static-site visitor necessarily sends it to the Graph gateway.
export const AGENT0_PUBLIC_API_KEY = "2636605c8c75cc8a1b8ddb5c07f8c563";
export const AGENT0_ENDPOINT_TEMPLATE = `https://gateway.thegraph.com/api/{api-key}/subgraphs/id/${AGENT0_BASE_SUBGRAPH_ID}`;
// Public, origin-restricted browser project ID; no signing authority or private key is present.
export const DEFAULT_BASE_RPC_ENDPOINT =
  "https://base-mainnet.infura.io/v3/e1656809acaa4db18ea2ea40e489c4c8";
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
  active: string;
  sampledRevoked: string;
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
