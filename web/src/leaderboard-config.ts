import {
  AGENT0_ENDPOINT_TEMPLATE,
  AGENT0_PUBLIC_API_KEY,
  DEFAULT_BASE_RPC_ENDPOINT,
} from "./leaderboard-types";

export interface LeaderboardConfig {
  graphEndpoint?: string;
  baseRpcEndpoint?: string;
  cacheFreshnessMs: number;
  ipfsGateway: string;
  arweaveGateway: string;
}

interface ConfigEnvironment {
  VITE_CTHUWU_GRAPHQL_ENDPOINT?: string;
  VITE_CTHUWU_GRAPH_API_KEY?: string;
  VITE_CTHUWU_BASE_RPC_ENDPOINT?: string;
  VITE_CTHUWU_IPFS_GATEWAY?: string;
  VITE_CTHUWU_ARWEAVE_GATEWAY?: string;
  VITE_CTHUWU_LEADERBOARD_FRESH_MS?: string;
}

const DEFAULT_CACHE_FRESHNESS_MS = 15 * 60 * 1000;
const DEFAULT_IPFS_GATEWAY = "https://cloudflare-ipfs.com/ipfs/";
const DEFAULT_ARWEAVE_GATEWAY = "https://arweave.net/";

export function parseLeaderboardConfig(
  environment: ConfigEnvironment = import.meta.env as ConfigEnvironment,
): LeaderboardConfig {
  const explicitEndpoint = environment.VITE_CTHUWU_GRAPHQL_ENDPOINT?.trim();
  const endpointTemplate = explicitEndpoint || AGENT0_ENDPOINT_TEMPLATE;
  const apiKey = environment.VITE_CTHUWU_GRAPH_API_KEY?.trim() || AGENT0_PUBLIC_API_KEY;
  if (explicitEndpoint?.includes("{api-key}") && !apiKey) {
    throw new Error("GraphQL endpoint contains an unresolved API-key placeholder");
  }
  const graphEndpoint = validHttpsUrl(
    endpointTemplate.replaceAll("{api-key}", apiKey),
    "GraphQL endpoint",
  );
  if (graphEndpoint?.includes("{api-key}")) {
    throw new Error("GraphQL endpoint still contains an unresolved API-key placeholder");
  }
  const freshness = Number(environment.VITE_CTHUWU_LEADERBOARD_FRESH_MS);
  return {
    ...(graphEndpoint ? { graphEndpoint } : {}),
    baseRpcEndpoint: validHttpsUrl(
      environment.VITE_CTHUWU_BASE_RPC_ENDPOINT?.trim() || DEFAULT_BASE_RPC_ENDPOINT,
      "Base RPC endpoint",
    ),
    cacheFreshnessMs:
      Number.isSafeInteger(freshness) && freshness >= 60_000 && freshness <= 86_400_000
        ? freshness
        : DEFAULT_CACHE_FRESHNESS_MS,
    ipfsGateway: validHttpsUrl(
      environment.VITE_CTHUWU_IPFS_GATEWAY?.trim() || DEFAULT_IPFS_GATEWAY,
      "IPFS gateway",
    ),
    arweaveGateway: validHttpsUrl(
      environment.VITE_CTHUWU_ARWEAVE_GATEWAY?.trim() || DEFAULT_ARWEAVE_GATEWAY,
      "Arweave gateway",
    ),
  };
}

function validHttpsUrl(value: string, label: string): string {
  if (value.length > 2_048) throw new Error(`${label} is too long`);
  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error(`${label} must be an absolute HTTPS URL`);
  }
  if (parsed.protocol !== "https:" || parsed.username || parsed.password) {
    throw new Error(`${label} must be an absolute credential-free HTTPS URL`);
  }
  return parsed.href;
}
