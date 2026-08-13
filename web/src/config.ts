export type XmtpEnvironment = "dev" | "production" | "local";

export const XMTP_ENVIRONMENT: XmtpEnvironment = "production";

// Public XMTP address of the canonical intro Tentacle, whose Base ERC-8004 agent is 61608.
// Branding-controlled routing still verifies the complete on-chain profile at one pinned block.
export const INTRO_TENTACLE_ADDRESS = "0x4c4e26a4683f6b6d63e19abf0bedd1171b9a6e90";
export const DEFAULT_BASE_RPC_ENDPOINT = "https://mainnet.base.org/";
export const CANONICAL_BRANDING_CONTRACT = "0xd8c36f13d79a505c7fbdc5f6467ea3cd75e896da";
export const CANONICAL_BRANDING_DEPLOYMENT_BLOCK = 49_852_729n;

const ADDRESS = /^0x[0-9a-f]{40}$/u;

interface ConfigEnvironment {
  VITE_CTHUWU_BASE_RPC_ENDPOINT?: string;
  VITE_CTHUWU_BRANDING_CONTRACT?: string;
  VITE_CTHUWU_ASSIGNMENT_REFRESH_MS?: string;
}

export interface AppConfig {
  environment: XmtpEnvironment;
  botAddress: string;
  baseRpcEndpoint: string;
  brandingContract?: string;
  assignmentRefreshMs: number;
}

export function parseConfig(
  source: ConfigEnvironment = import.meta.env as ConfigEnvironment,
): AppConfig {
  const configuredRefresh = Number(source.VITE_CTHUWU_ASSIGNMENT_REFRESH_MS);
  const brandingContract =
    source.VITE_CTHUWU_BRANDING_CONTRACT?.trim().toLowerCase() || CANONICAL_BRANDING_CONTRACT;
  if (
    brandingContract &&
    (!ADDRESS.test(brandingContract) || brandingContract === "0x0000000000000000000000000000000000000000")
  ) {
    throw new Error("Acolyte Branding contract must be a lowercase Ethereum address");
  }
  return {
    environment: XMTP_ENVIRONMENT,
    botAddress: INTRO_TENTACLE_ADDRESS,
    baseRpcEndpoint: parseHttpsUrl(
      source.VITE_CTHUWU_BASE_RPC_ENDPOINT?.trim() || DEFAULT_BASE_RPC_ENDPOINT,
    ),
    brandingContract,
    assignmentRefreshMs:
      Number.isSafeInteger(configuredRefresh) &&
      configuredRefresh >= 60_000 &&
      configuredRefresh <= 60 * 60 * 1000
        ? configuredRefresh
        : 10 * 60 * 1000,
  };
}

function parseHttpsUrl(value: string): string {
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
