import { Interface, getAddress } from "ethers";
import type { AppConfig } from "../config";
import type { StoredIdentity } from "../identity";
import { fetchCompleteLeaderboard } from "../leaderboard-data";
import { parseLeaderboardConfig } from "../leaderboard-config";
import {
  ALLEGIANCE_HEX,
  BASE_CHAIN_ID,
  IDENTITY_REGISTRY,
  PROTOCOL_V1_HEX,
  UWU_CONTRACT,
  ZERO_ADDRESS,
} from "../leaderboard-types";

const BRANDING_INTERFACE = new Interface([
  "function brandingOf(address acolyte) view returns (tuple(uint256 tokenId,address acolyte,address owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 pendingDeclaredPrice,uint256 pendingPriceActivation,uint8 status) result)",
  "function BASE_CHAIN_ID() view returns (uint256)",
  "function IDENTITY_REGISTRY() view returns (address)",
  "function UWU() view returns (address)",
  "function REGISTRY_VERSION() view returns (string)",
]);
const REGISTRY_INTERFACE = new Interface([
  "function getVersion() view returns (string)",
  "function getAgentWallet(uint256 agentId) view returns (address)",
  "function isAuthorizedOrOwner(address wallet,uint256 agentId) view returns (bool)",
  "function getMetadata(uint256 agentId,string key) view returns (bytes)",
  "function tokenURI(uint256 tokenId) view returns (string)",
]);
const NONEMPTY_CODE = /^0x[0-9a-fA-F]+$/u;
const QUANTITY = /^0x(?:0|[1-9a-fA-F][0-9a-fA-F]*)$/u;
const BLOCK_HASH = /^0x[0-9a-f]{64}$/u;
const DATA_JSON_PREFIX = "data:application/json;base64,";
const REGISTRATION_TYPE = "https://eips.ethereum.org/EIPS/eip-8004#registration-v1";
const MAX_AGENT_URI_BYTES = 8 * 1024;

export type BrandingStatus = "Unminted" | "Active" | "Expired" | "Ineligible";

export type TentacleAssignment =
  | {
      source: "intro-unconfigured";
      address: string;
      notice: string;
    }
  | {
      source: "intro-fallback";
      address: string;
      brandingStatus: Exclude<BrandingStatus, "Active">;
      blockNumber: bigint;
      blockHash: string;
      notice: string;
    }
  | {
      source: "branding-active";
      address: string;
      inboxId: string;
      agentId: string;
      wallet: string;
      blockNumber: bigint;
      blockHash: string;
      notice: string;
    }
  | {
      source: "anchor-verified";
      address: string;
      inboxId: string;
      agentId: string;
      wallet: string;
      blockNumber: bigint;
      blockHash: string;
      notice: string;
    };

export class RegistryUnavailableError extends Error {
  constructor(message = "Canonical Base routing state is unavailable; retry without changing assignment") {
    super(message);
    this.name = "RegistryUnavailableError";
  }
}

export interface RpcClient {
  request(method: string, params: unknown[]): Promise<unknown>;
}

export async function resolveTentacleAssignment(
  config: AppConfig,
  identity: StoredIdentity,
  options: { rpc?: RpcClient; fetch?: typeof fetch; discoverAgentIds?: (wallet: string) => Promise<string[]> } = {},
): Promise<TentacleAssignment> {
  if (!config.brandingContract) {
    if (config.tentacleAnchor) return resolveAnchoredTentacle(config, options);
    return {
      source: "intro-unconfigured",
      address: config.botAddress,
      notice: "Branding routing is pending deployment; using the configured intro Tentacle",
    };
  }
  try {
    const rpc = options.rpc ?? createJsonRpcClient(config.baseRpcEndpoint, options.fetch ?? fetch);
    const chainId = quantity(await rpc.request("eth_chainId", []), "Base chain ID");
    if (chainId !== BigInt(BASE_CHAIN_ID)) throw new Error("RPC returned the wrong chain");
    const blockNumber = quantity(await rpc.request("eth_blockNumber", []), "Base block number");
    const blockTag = `0x${blockNumber.toString(16)}`;
    const observedBlock = block(await rpc.request("eth_getBlockByNumber", [blockTag, false]));
    if (observedBlock.number !== blockNumber) throw new Error("RPC returned a mismatched Base block");
    const [brandingCode, registryCode] = await Promise.all([
      rpc.request("eth_getCode", [config.brandingContract, blockTag]),
      rpc.request("eth_getCode", [IDENTITY_REGISTRY, blockTag]),
    ]);
    if (!isCode(brandingCode) || !isCode(registryCode)) {
      throw new Error("canonical contract code is unavailable");
    }
    const [brandingResult, brandingChain, brandingRegistry, brandingUwu, brandingVersion] =
      await Promise.all([
        contractCall(rpc, BRANDING_INTERFACE, config.brandingContract, "brandingOf", [identity.address], blockTag),
        contractCall(rpc, BRANDING_INTERFACE, config.brandingContract, "BASE_CHAIN_ID", [], blockTag),
        contractCall(rpc, BRANDING_INTERFACE, config.brandingContract, "IDENTITY_REGISTRY", [], blockTag),
        contractCall(rpc, BRANDING_INTERFACE, config.brandingContract, "UWU", [], blockTag),
        contractCall(rpc, BRANDING_INTERFACE, config.brandingContract, "REGISTRY_VERSION", [], blockTag),
      ]);
    const branding = brandingResult[0] as unknown as {
      tokenId: bigint;
      acolyte: string;
      owner: string;
      controllerAgentId: bigint;
      status: bigint;
    };
    const acolyte = canonicalAddress(identity.address, "StoredIdentity address");
    if (
      brandingChain[0] !== BigInt(BASE_CHAIN_ID) ||
      canonicalAddress(brandingRegistry[0], "Branding identity registry") !== IDENTITY_REGISTRY ||
      canonicalAddress(brandingUwu[0], "Branding UWU") !== UWU_CONTRACT ||
      brandingVersion[0] !== "2.0.0" ||
      branding.tokenId !== BigInt(acolyte) ||
      canonicalAddress(branding.acolyte, "Branding acolyte") !== acolyte
    ) {
      throw new Error("Branding result is not bound to the StoredIdentity acolyte");
    }
    const status = brandingStatus(branding.status);
    if (status === "RegistryUnavailable") throw new Error("Branding reports RegistryUnavailable");
    if (status !== "Active") {
      if (config.tentacleAnchor) {
        return await resolveAnchoredTentacle(config, options, rpc, blockNumber, blockTag, observedBlock);
      }
      await verifyUnchangedBlock(rpc, blockTag, observedBlock);
      return {
        source: "intro-fallback",
        address: config.botAddress,
        brandingStatus: status,
        blockNumber,
        blockHash: observedBlock.hash,
        notice: `${status} Branding at Base block ${blockNumber}; using the configured intro Tentacle`,
      };
    }
    const owner = canonicalAddress(branding.owner, "Branding owner");
    if (owner === ZERO_ADDRESS) throw new Error("active Branding returned a zero owner");
    const agentId = branding.controllerAgentId.toString();
    const [version, walletResult, authorizedResult, allegianceResult, protocolResult, uriResult] =
      await Promise.all([
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getVersion", [], blockTag),
        contractCall(
          rpc,
          REGISTRY_INTERFACE,
          IDENTITY_REGISTRY,
          "getAgentWallet",
          [agentId],
          blockTag,
        ),
        contractCall(
          rpc,
          REGISTRY_INTERFACE,
          IDENTITY_REGISTRY,
          "isAuthorizedOrOwner",
          [owner, agentId],
          blockTag,
        ),
        contractCall(
          rpc,
          REGISTRY_INTERFACE,
          IDENTITY_REGISTRY,
          "getMetadata",
          [agentId, "cthuwu.allegiance"],
          blockTag,
        ),
        contractCall(
          rpc,
          REGISTRY_INTERFACE,
          IDENTITY_REGISTRY,
          "getMetadata",
          [agentId, "cthuwu.protocol"],
          blockTag,
        ),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "tokenURI", [agentId], blockTag),
      ]);
    const wallet = canonicalAddress(walletResult[0], "controller agentWallet");
    if (
      version[0] !== "2.0.0" ||
      wallet !== owner ||
      authorizedResult[0] !== true ||
      String(allegianceResult[0]).toLowerCase() !== ALLEGIANCE_HEX ||
      String(protocolResult[0]).toLowerCase() !== PROTOCOL_V1_HEX
    ) {
      throw new Error("controller eligibility changed at the verified block");
    }
    const inboxId = endpointFromAgentUri(String(uriResult[0]), agentId);
    await verifyUnchangedBlock(rpc, blockTag, observedBlock);
    return {
      source: "branding-active",
      address: wallet,
      inboxId,
      agentId,
      wallet,
      blockNumber,
      blockHash: observedBlock.hash,
      notice: `Active Branding verified at Base block ${blockNumber}`,
    };
  } catch (error) {
    if (error instanceof RegistryUnavailableError) throw error;
    throw new RegistryUnavailableError(
      error instanceof Error
        ? `Canonical Base routing state is unavailable: ${error.message}`
        : undefined,
    );
  }
}

async function resolveAnchoredTentacle(
  config: AppConfig,
  options: { rpc?: RpcClient; fetch?: typeof fetch; discoverAgentIds?: (wallet: string) => Promise<string[]> },
  existingRpc?: RpcClient,
  existingBlockNumber?: bigint,
  existingBlockTag?: string,
  existingBlock?: BlockHeader,
): Promise<TentacleAssignment> {
  try {
    const wallet = canonicalAddress(config.tentacleAnchor, "t link target");
    if (wallet === ZERO_ADDRESS) throw new Error("t link target is zero");
    const discover = options.discoverAgentIds ?? (async (target: string) => {
      const endpoint = parseLeaderboardConfig().graphEndpoint;
      if (!endpoint) throw new Error("Agent0 discovery is unavailable");
      const snapshot = await fetchCompleteLeaderboard(endpoint, {
        fetch: options.fetch,
        baseRpcEndpoint: config.baseRpcEndpoint,
      });
      const match = snapshot.rankedWallets.find((entry) => entry.wallet.toLowerCase() === target);
      return match?.identities.map((identity) => identity.agentId) ?? [];
    });
    const agentIds = [...new Set(await discover(wallet))];
    if (agentIds.length !== 1 || !/^(?:0|[1-9][0-9]*)$/u.test(agentIds[0]!)) {
      throw new Error(agentIds.length === 0
        ? "no discoverable Cthuwu ERC-8004 identity belongs to the t address"
        : "the t address controls more than one Tentacle and is ambiguous");
    }
    const rpc = existingRpc ?? options.rpc ?? createJsonRpcClient(config.baseRpcEndpoint, options.fetch ?? fetch);
    let blockNumber = existingBlockNumber;
    let blockTag = existingBlockTag;
    let observed = existingBlock;
    if (!blockNumber || !blockTag || !observed) {
      const chainId = quantity(await rpc.request("eth_chainId", []), "Base chain ID");
      if (chainId !== BigInt(BASE_CHAIN_ID)) throw new Error("RPC returned the wrong chain");
      blockNumber = quantity(await rpc.request("eth_blockNumber", []), "Base block number");
      blockTag = `0x${blockNumber.toString(16)}`;
      observed = block(await rpc.request("eth_getBlockByNumber", [blockTag, false]));
      const registryCode = await rpc.request("eth_getCode", [IDENTITY_REGISTRY, blockTag]);
      if (!isCode(registryCode)) throw new Error("canonical registry code is unavailable");
    }
    const agentId = agentIds[0]!;
    const [version, walletResult, authorizedResult, allegianceResult, protocolResult, uriResult] = await Promise.all([
      contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getVersion", [], blockTag),
      contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getAgentWallet", [agentId], blockTag),
      contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "isAuthorizedOrOwner", [wallet, agentId], blockTag),
      contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getMetadata", [agentId, "cthuwu.allegiance"], blockTag),
      contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getMetadata", [agentId, "cthuwu.protocol"], blockTag),
      contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "tokenURI", [agentId], blockTag),
    ]);
    if (version[0] !== "2.0.0" || canonicalAddress(walletResult[0], "agentWallet") !== wallet ||
      authorizedResult[0] !== true || String(allegianceResult[0]).toLowerCase() !== ALLEGIANCE_HEX ||
      String(protocolResult[0]).toLowerCase() !== PROTOCOL_V1_HEX) {
      throw new Error("the t address is not the current eligible controller of that Tentacle");
    }
    const inboxId = endpointFromAgentUri(String(uriResult[0]), agentId);
    await verifyUnchangedBlock(rpc, blockTag, observed);
    return { source: "anchor-verified", address: wallet, wallet, inboxId, agentId,
      blockNumber, blockHash: observed.hash, notice: `Deep-linked Tentacle verified at Base block ${blockNumber}` };
  } catch (error) {
    throw new RegistryUnavailableError(error instanceof Error ? `Tentacle link could not be verified: ${error.message}` : undefined);
  }
}

interface BlockHeader {
  number: bigint;
  hash: string;
}

function block(value: unknown): BlockHeader {
  if (!isRecord(value) || !BLOCK_HASH.test(String(value.hash))) {
    throw new Error("Base block header is invalid");
  }
  return {
    number: quantity(value.number, "Base block header number"),
    hash: String(value.hash),
  };
}

async function verifyUnchangedBlock(
  rpc: RpcClient,
  blockTag: string,
  observed: BlockHeader,
): Promise<void> {
  const canonical = block(await rpc.request("eth_getBlockByNumber", [blockTag, false]));
  if (canonical.number !== observed.number || canonical.hash !== observed.hash) {
    throw new Error("canonical Base state changed during assignment verification");
  }
}

export function endpointFromAgentUri(agentUri: string, agentId: string): string {
  const profile = decodeDataJson(agentUri, MAX_AGENT_URI_BYTES, "controller registration profile");
  if (
    !isRecord(profile) ||
    profile.type !== REGISTRATION_TYPE ||
    profile.active !== true ||
    !Array.isArray(profile.services) ||
    profile.services.length > 16 ||
    !Array.isArray(profile.registrations)
  ) {
    throw new Error("controller registration profile is not active registration-v1");
  }
  const canonicalRegistry = getAddress(IDENTITY_REGISTRY);
  const registryReference = `eip155:${BASE_CHAIN_ID}:${canonicalRegistry}`;
  const registrations = profile.registrations.filter((entry) => {
    if (!isRecord(entry) || !hasExactKeys(entry, ["agentId", "agentRegistry"])) return false;
    if (entry.agentRegistry !== registryReference) return false;
    return entry.agentId === agentId ||
      (typeof entry.agentId === "number" && Number.isSafeInteger(entry.agentId) &&
        entry.agentId >= 0 && BigInt(entry.agentId) === BigInt(agentId));
  });
  const xmtpServices = profile.services.filter((service) =>
    isRecord(service) && hasExactKeys(service, ["endpoint", "name", "version"]) &&
    service.name === "CTHUWU-XMTP" && service.version === "1" &&
    typeof service.endpoint === "string",
  ) as Record<string, unknown>[];
  const manifestServices = profile.services.filter((service) =>
    isRecord(service) && hasExactKeys(service, ["endpoint", "name", "version"]) &&
    service.name === "CTHUWU" && typeof service.endpoint === "string" && service.version === "1",
  ) as Record<string, unknown>[];
  if (registrations.length !== 1 || xmtpServices.length !== 1 || manifestServices.length !== 1) {
    throw new Error("controller profile does not have one exact canonical service binding");
  }
  const endpoint = xmtpServices[0]?.endpoint;
  if (typeof endpoint !== "string" || !/^xmtp:\/\/[0-9a-f]{64}$/u.test(endpoint)) {
    throw new Error("controller XMTP endpoint is invalid");
  }
  const manifestUri = manifestServices[0]?.endpoint;
  if (typeof manifestUri !== "string") throw new Error("controller CTHUWU manifest is missing");
  const manifest = decodeDataJson(manifestUri, MAX_AGENT_URI_BYTES, "controller CTHUWU manifest");
  if (
    !isRecord(manifest) ||
    !hasExactKeys(manifest, ["capabilities", "erc8004", "protocol", "schemaVersion", "tentacleId", "xmtp"]) ||
    manifest.schemaVersion !== 1 || manifest.protocol !== 1 ||
    typeof manifest.tentacleId !== "string" || manifest.tentacleId.length === 0 ||
    new TextEncoder().encode(manifest.tentacleId).length > 128 ||
    !isRecord(manifest.erc8004) ||
    !hasExactKeys(manifest.erc8004, ["agentId", "chainId", "registry"]) ||
    manifest.erc8004.chainId !== BASE_CHAIN_ID ||
    typeof manifest.erc8004.registry !== "string" ||
    canonicalAddress(manifest.erc8004.registry, "manifest registry") !== IDENTITY_REGISTRY ||
    manifest.erc8004.agentId !== agentId ||
    !isRecord(manifest.xmtp) || !hasExactKeys(manifest.xmtp, ["endpoint", "environment"]) ||
    manifest.xmtp.environment !== "production" || manifest.xmtp.endpoint !== endpoint ||
    !Array.isArray(manifest.capabilities) || manifest.capabilities.length > 16 ||
    !manifest.capabilities.every((capability) =>
      typeof capability === "string" && capability.length > 0 &&
      new TextEncoder().encode(capability).length <= 64) ||
    !manifest.capabilities.includes("direct-xmtp-messaging")
  ) {
    throw new Error("controller CTHUWU manifest does not bind its canonical identity and endpoint");
  }
  return endpoint.slice("xmtp://".length);
}

function decodeDataJson(value: string, maximumBytes: number, label: string): unknown {
  if (!value.startsWith(DATA_JSON_PREFIX) || new TextEncoder().encode(value).length > maximumBytes) {
    throw new Error(`${label} is not a bounded data URI`);
  }
  const encoded = value.slice(DATA_JSON_PREFIX.length);
  if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/u.test(encoded)) {
    throw new Error(`${label} has invalid base64`);
  }
  try {
    const bytes = Uint8Array.from(atob(encoded), (character) => character.charCodeAt(0));
    if (bytes.length > maximumBytes) throw new Error("too large");
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch {
    throw new Error(`${label} has invalid JSON`);
  }
}

function hasExactKeys(value: Record<string, unknown>, expected: string[]): boolean {
  const keys = Object.keys(value).sort();
  return keys.length === expected.length && keys.every((key, index) => key === expected[index]);
}

export function createJsonRpcClient(endpoint: string, fetcher: typeof fetch): RpcClient {
  let id = 0;
  type Pending = {
    id: number;
    method: string;
    params: unknown[];
    resolve: (value: unknown) => void;
    reject: (reason: unknown) => void;
  };
  let pending: Pending[] = [];
  let scheduled = false;

  const flush = async (): Promise<void> => {
    scheduled = false;
    const batch = pending;
    pending = [];
    try {
      const response = await fetcher(endpoint, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(batch.map(({ id: requestId, method, params }) => ({
          jsonrpc: "2.0", id: requestId, method, params,
        }))),
      });
      const length = Number(response.headers.get("content-length"));
      if (!response.ok || (Number.isFinite(length) && length > 128 * 1024)) {
        throw new Error(`Base RPC failed with HTTP ${response.status}`);
      }
      const body = await response.text();
      if (body.length > 128 * 1024) throw new Error("Base RPC response is too large");
      const parsed = JSON.parse(body) as unknown;
      if (!Array.isArray(parsed) || parsed.length !== batch.length) {
        throw new Error("Base RPC returned an invalid batch response");
      }
      const byId = new Map(parsed.filter(isRecord).map((item) => [item.id, item]));
      for (const request of batch) {
        const item = byId.get(request.id);
        if (!item || item.jsonrpc !== "2.0" || "error" in item || !("result" in item)) {
          request.reject(new Error("Base RPC returned an invalid response"));
        } else {
          request.resolve(item.result);
        }
      }
    } catch (error) {
      for (const request of batch) request.reject(error);
    }
  };
  return {
    request: (method, params) => new Promise((resolve, reject) => {
      pending.push({ id: ++id, method, params, resolve, reject });
      if (!scheduled) {
        scheduled = true;
        queueMicrotask(() => void flush());
      }
    }),
  };
}

async function contractCall(
  rpc: RpcClient,
  abi: Interface,
  to: string,
  method: string,
  args: unknown[],
  blockTag: string,
): Promise<ReturnType<Interface["decodeFunctionResult"]>> {
  const data = abi.encodeFunctionData(method, args);
  const result = await rpc.request("eth_call", [{ to, data }, blockTag]);
  if (typeof result !== "string" || !/^0x(?:[0-9a-fA-F]{2})*$/u.test(result)) {
    throw new Error(`${method} returned malformed data`);
  }
  return abi.decodeFunctionResult(method, result);
}

function brandingStatus(value: bigint): BrandingStatus | "RegistryUnavailable" {
  switch (Number(value)) {
    case 0:
      return "Unminted";
    case 1:
      return "Active";
    case 2:
      return "Expired";
    case 3:
      return "Ineligible";
    case 4:
      return "RegistryUnavailable";
    default:
      throw new Error("Branding returned an unknown status");
  }
}

function quantity(value: unknown, label: string): bigint {
  if (typeof value !== "string" || !QUANTITY.test(value)) throw new Error(`${label} is invalid`);
  return BigInt(value);
}

function isCode(value: unknown): value is string {
  return typeof value === "string" && value !== "0x" && NONEMPTY_CODE.test(value);
}

function canonicalAddress(value: unknown, label: string): string {
  if (typeof value !== "string") throw new Error(`${label} is invalid`);
  try {
    return getAddress(value).toLowerCase();
  } catch {
    throw new Error(`${label} is invalid`);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
