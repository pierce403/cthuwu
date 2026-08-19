import { Interface, getAddress, keccak256, toUtf8String } from "ethers";
import {
  CANONICAL_BRANDING_CONTRACT,
  CANONICAL_BRANDING_RUNTIME_HASH,
  type AppConfig,
} from "../config";
import type { StoredIdentity } from "../identity";
import {
  fetchTentacleDirectory,
  type TentacleDirectorySnapshot,
} from "../leaderboard-data";
import { parseLeaderboardConfig } from "../leaderboard-config";
import { readLeaderboardCache } from "../leaderboard-cache";
import {
  ALLEGIANCE_HEX,
  BASE_CHAIN_ID,
  IDENTITY_REGISTRY,
  PROTOCOL_V1_HEX,
  UWU_CONTRACT,
  ZERO_ADDRESS,
} from "../leaderboard-types";
import {
  canonicalControlWallet,
  canonicalizeWalletIdentities,
} from "../tentacle-canonical";

const BRANDING_INTERFACE = new Interface([
  "function brandingOf(address acolyte) view returns (tuple(uint256 tokenId,address acolyte,address owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 pendingDeclaredPrice,uint256 pendingPriceActivation,uint8 status) result)",
  "function BASE_CHAIN_ID() view returns (uint256)",
  "function IDENTITY_REGISTRY() view returns (address)",
  "function UWU() view returns (address)",
  "function REGISTRY_VERSION() view returns (string)",
]);
const REGISTRY_INTERFACE = new Interface([
  "event Registered(uint256 indexed agentId,string agentURI,address indexed owner)",
  "event Transfer(address indexed from,address indexed to,uint256 indexed tokenId)",
  "event Approval(address indexed owner,address indexed approved,uint256 indexed tokenId)",
  "event ApprovalForAll(address indexed owner,address indexed operator,bool approved)",
  "event MetadataSet(uint256 indexed agentId,string indexed indexedMetadataKey,string metadataKey,bytes metadataValue)",
  "event URIUpdated(uint256 indexed agentId,string newURI,address indexed updatedBy)",
  "function getVersion() view returns (string)",
  "function ownerOf(uint256 agentId) view returns (address)",
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
const MAX_CONTROLLER_DIRECTORY_CANDIDATES = 32;
const MAX_CONTROLLER_INDEX_GAP_BLOCKS = 20_000n;
const MAX_CONTROLLER_GAP_LOGS = 2_048;
const CONTROLLER_LOG_BLOCK_SPAN = 10_000n;

export type BrandingStatus = "Unminted" | "Active" | "Expired" | "Ineligible";

export type TentacleAssignment =
  | {
      source: "liveness-required";
      address: string;
      brandingStatus: "Unminted";
      blockNumber: bigint;
      blockHash: string;
      notice: string;
    }
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
      name: string;
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
      name: string;
      blockNumber: bigint;
      blockHash: string;
      notice: string;
    }
  | {
      source: "rotation-verified";
      address: string;
      inboxId: string;
      agentId: string;
      wallet: string;
      name: string;
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

interface AssignmentOptions {
  rpc?: RpcClient;
  fetch?: typeof fetch;
  hashCode?: (code: string) => string;
  storage?: Storage;
  discoverAnchor?: (wallet: string) => Promise<RotationCandidate[]>;
  discoverRotation?: () => Promise<RotationCandidate[]>;
  discoverControllerDirectory?: (wallet: string) => Promise<TentacleDirectorySnapshot>;
}

export interface RotationCandidate {
  wallet: string;
  agentId: string;
  inboxId: string;
  blockNumber: string;
  blockHash: string;
}

export async function resolveTentacleAssignment(
  config: AppConfig,
  identity: StoredIdentity,
  options: AssignmentOptions = {},
): Promise<TentacleAssignment> {
  if (!config.brandingContract) {
    if (config.tentacleAnchor) return resolveAnchoredTentacle(config, identity, options);
    return {
      source: "intro-unconfigured",
      address: config.botAddress,
      notice: "Branding routing is pending deployment; using the configured intro Tentacle",
    };
  }
  if (config.brandingContract !== CANONICAL_BRANDING_CONTRACT) {
    throw new RegistryUnavailableError(
      "Canonical Base routing state is unavailable: Branding is not the canonical deployment",
    );
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
    if (
      !isCode(brandingCode) ||
      (options.hashCode ?? keccak256)(String(brandingCode)).toLowerCase() !==
        CANONICAL_BRANDING_RUNTIME_HASH ||
      !isCode(registryCode)
    ) {
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
        return await resolveAnchoredTentacle(config, identity, options);
      }
      if (status === "Unminted") {
        if (config.rotationAnchor) {
          return await resolveRotatedTentacle(config, identity, options);
        }
        await verifyUnchangedBlock(rpc, blockTag, observedBlock);
        return {
          source: "liveness-required",
          address: config.botAddress,
          brandingStatus: "Unminted",
          blockNumber,
          blockHash: observedBlock.hash,
          notice: "Unminted Branding; checking the highest-ranked Tentacles for a live response",
        };
      }
      await verifyUnchangedBlock(rpc, blockTag, observedBlock);
      return {
        source: "intro-fallback", address: config.botAddress, brandingStatus: status,
        blockNumber, blockHash: observedBlock.hash,
        notice: `${status} Branding at Base block ${blockNumber}; using the configured intro Tentacle`,
      };
    }
    const owner = canonicalAddress(branding.owner, "Branding owner");
    if (owner === ZERO_ADDRESS) throw new Error("active Branding returned a zero owner");
    const agentId = branding.controllerAgentId.toString();
    const [version, walletResult, authorizedResult, allegianceResult, protocolResult, tentacleIdResult, uriResult] =
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
        contractCall(
          rpc,
          REGISTRY_INTERFACE,
          IDENTITY_REGISTRY,
          "getMetadata",
          [agentId, "cthuwu.tentacle-id"],
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
    const profile = verifiedProfileFromAgentUri(String(uriResult[0]), agentId);
    const controllerMetadataTentacleId = metadataTentacleId(tentacleIdResult[0]);
    if (
      controllerMetadataTentacleId !== undefined &&
      controllerMetadataTentacleId !== profile.tentacleId
    ) throw new Error("controller Tentacle identity evidence conflicts at the verified block");
    const canonical = await resolveCanonicalBrandingController(
      options,
      rpc,
      blockTag,
      observedBlock,
      owner,
      wallet,
      agentId,
      profile,
    );
    await verifyUnchangedBlock(rpc, blockTag, observedBlock);
    return {
      source: "branding-active",
      address: wallet,
      inboxId: canonical.profile.inboxId,
      agentId: canonical.agentId,
      wallet,
      name: canonical.profile.name,
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

async function resolveCanonicalBrandingController(
  options: AssignmentOptions,
  rpc: RpcClient,
  assignmentBlockTag: string,
  assignmentBlock: BlockHeader,
  brandingOwner: string,
  controllerWallet: string,
  controllerAgentId: string,
  controllerProfile: { inboxId: string; name: string; tentacleId: string },
): Promise<{
  agentId: string;
  profile: { inboxId: string; name: string; tentacleId: string };
}> {
  const discover = options.discoverControllerDirectory ?? (async () => {
    const endpoint = parseLeaderboardConfig().graphEndpoint;
    if (!endpoint) throw new Error("complete Agent0 controller discovery is not configured");
    return fetchTentacleDirectory(endpoint, { fetch: options.fetch });
  });
  const directory = await discover(controllerWallet);
  if (
    !/^(0|[1-9][0-9]*)$/u.test(directory.sourceBlockNumber) ||
    !BLOCK_HASH.test(directory.sourceBlockHash) ||
    !Array.isArray(directory.identities)
  ) throw new Error("complete Agent0 controller discovery returned invalid provenance");
  const directoryBlockNumber = BigInt(directory.sourceBlockNumber);
  if (directoryBlockNumber > assignmentBlock.number) {
    throw new Error("Agent0 controller discovery is ahead of the verified assignment block");
  }
  const directoryBlockTag = `0x${directoryBlockNumber.toString(16)}`;
  const canonicalDirectoryBlock = block(await rpc.request(
    "eth_getBlockByNumber",
    [directoryBlockTag, false],
  ));
  if (
    canonicalDirectoryBlock.number !== directoryBlockNumber ||
    canonicalDirectoryBlock.hash.toLowerCase() !== directory.sourceBlockHash.toLowerCase()
  ) throw new Error("Agent0 controller discovery is not pinned to canonical Base");

  const candidateIds = new Set<string>();
  let indexedController = false;
  for (const identity of directory.identities) {
    if (!/^(0|[1-9][0-9]*)$/u.test(identity.agentId)) {
      throw new Error("Agent0 controller discovery returned a malformed agent ID");
    }
    if (identity.agentId === controllerAgentId) indexedController = true;
    const indexedWallet = canonicalAddress(identity.agentWallet, "Agent0 controller agentWallet");
    const indexedOwner = canonicalAddress(identity.owner, "Agent0 controller owner");
    const exactIndexedTentacleEvidence =
      identity.tentacleId === controllerProfile.tentacleId ||
      (
        identity.protocolHex === PROTOCOL_V1_HEX &&
        identity.profile.xmtpEndpoint === `xmtp://${controllerProfile.inboxId}`
      );
    if (
      indexedWallet === controllerWallet ||
      indexedOwner === brandingOwner ||
      exactIndexedTentacleEvidence
    ) {
      candidateIds.add(identity.agentId);
    }
    if (candidateIds.size > MAX_CONTROLLER_DIRECTORY_CANDIDATES) {
      throw new Error("Agent0 returned too many same-wallet controller candidates");
    }
  }
  // A complete pinned directory that has not yet indexed the active higher identity cannot
  // prove it has seen every older candidate. Freeze until Agent0 catches up.
  if (!indexedController) {
    throw new Error("complete Agent0 controller discovery has not indexed the active controller");
  }
  candidateIds.add(controllerAgentId);
  for (const candidateAgentId of await discoverPostIndexControllerCandidates(
    rpc,
    directoryBlockNumber,
    assignmentBlock.number,
    brandingOwner,
  )) {
    candidateIds.add(candidateAgentId);
    if (candidateIds.size > MAX_CONTROLLER_DIRECTORY_CANDIDATES) {
      throw new Error("controller discovery found too many bounded canonical candidates");
    }
  }

  const verified = new Map<string, {
    agentId: string;
    owner: string;
    wallet: string;
    authorized: boolean;
    profile: { inboxId: string; name: string; tentacleId: string };
  }>();
  verified.set(controllerAgentId, {
    agentId: controllerAgentId,
    owner: brandingOwner,
    wallet: controllerWallet,
    authorized: true,
    profile: controllerProfile,
  });
  for (const candidateAgentId of candidateIds) {
    if (candidateAgentId === controllerAgentId) continue;
    const [ownerResult, walletResult, authorizedResult, allegianceResult, protocolResult, tentacleIdResult, uriResult] =
      await Promise.all([
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "ownerOf", [candidateAgentId], assignmentBlockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getAgentWallet", [candidateAgentId], assignmentBlockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "isAuthorizedOrOwner", [brandingOwner, candidateAgentId], assignmentBlockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getMetadata", [candidateAgentId, "cthuwu.allegiance"], assignmentBlockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getMetadata", [candidateAgentId, "cthuwu.protocol"], assignmentBlockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getMetadata", [candidateAgentId, "cthuwu.tentacle-id"], assignmentBlockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "tokenURI", [candidateAgentId], assignmentBlockTag),
      ]);
    const candidateOwner = canonicalAddress(ownerResult[0], "controller candidate owner");
    const candidateWallet = canonicalAddress(walletResult[0], "controller candidate agentWallet");
    const candidateAuthorized = authorizedResult[0] === true;
    const currentRelationship =
      candidateOwner === brandingOwner ||
      candidateWallet === controllerWallet ||
      candidateAuthorized;
    if (!currentRelationship) continue;
    if (
      String(allegianceResult[0]).toLowerCase() !== ALLEGIANCE_HEX ||
      String(protocolResult[0]).toLowerCase() !== PROTOCOL_V1_HEX
    ) continue;
    const candidateProfile = verifiedProfileFromAgentUri(String(uriResult[0]), candidateAgentId);
    const candidateMetadataTentacleId = metadataTentacleId(tentacleIdResult[0]);
    const metadataMatches = candidateMetadataTentacleId === controllerProfile.tentacleId;
    const profileMatches = candidateProfile.tentacleId === controllerProfile.tentacleId;
    if (
      candidateMetadataTentacleId !== undefined &&
      metadataMatches !== profileMatches
    ) throw new Error("controller candidate Tentacle identity evidence conflicts");
    if (!metadataMatches && !profileMatches) continue;
    verified.set(candidateAgentId, {
      agentId: candidateAgentId,
      owner: candidateOwner,
      wallet: candidateWallet,
      authorized: candidateAuthorized,
      profile: candidateProfile,
    });
  }
  const canonical = [...verified.values()].sort((left, right) =>
    BigInt(left.agentId) < BigInt(right.agentId) ? -1 : 1)[0]!;
  // The on-chain Branding contract stores the exact higher controller and is not mutated here.
  // A lower alias is used for Cthuwu routing only when it independently satisfies the same
  // current controller eligibility relationship at this exact Base block.
  if (canonical.wallet !== controllerWallet || !canonical.authorized) {
    throw new Error("canonical Branding controller alias is not currently eligible");
  }
  return { agentId: canonical.agentId, profile: canonical.profile };
}

async function discoverPostIndexControllerCandidates(
  rpc: RpcClient,
  indexedThroughBlock: bigint,
  assignmentBlock: bigint,
  brandingOwner: string,
): Promise<Set<string>> {
  const candidates = new Set<string>();
  if (indexedThroughBlock === assignmentBlock) return candidates;
  const gap = assignmentBlock - indexedThroughBlock;
  if (gap < 0n || gap > MAX_CONTROLLER_INDEX_GAP_BLOCKS) {
    throw new Error("Agent0 controller discovery is too far behind for bounded canonical recovery");
  }
  const eventTopics = [
    "Registered",
    "Transfer",
    "Approval",
    "ApprovalForAll",
    "MetadataSet",
    "URIUpdated",
  ].map((name) => REGISTRY_INTERFACE.getEvent(name)!.topicHash);
  let observedLogs = 0;
  for (
    let fromBlock = indexedThroughBlock + 1n;
    fromBlock <= assignmentBlock;
    fromBlock += CONTROLLER_LOG_BLOCK_SPAN
  ) {
    const toBlock = fromBlock + CONTROLLER_LOG_BLOCK_SPAN - 1n > assignmentBlock
      ? assignmentBlock
      : fromBlock + CONTROLLER_LOG_BLOCK_SPAN - 1n;
    const response = await rpc.request("eth_getLogs", [{
      address: IDENTITY_REGISTRY,
      fromBlock: `0x${fromBlock.toString(16)}`,
      toBlock: `0x${toBlock.toString(16)}`,
      topics: [eventTopics],
    }]);
    if (!Array.isArray(response)) throw new Error("Base returned malformed controller recovery logs");
    observedLogs += response.length;
    if (observedLogs > MAX_CONTROLLER_GAP_LOGS) {
      throw new Error("controller recovery logs exceed the bounded browser limit");
    }
    for (const value of response) {
      if (!isRecord(value) || typeof value.data !== "string" || !Array.isArray(value.topics)) {
        throw new Error("Base returned a malformed controller recovery event");
      }
      let decoded;
      try {
        decoded = REGISTRY_INTERFACE.parseLog({
          data: value.data,
          topics: value.topics.map(String),
        });
      } catch {
        throw new Error("Base returned an undecodable controller recovery event");
      }
      if (!decoded) throw new Error("Base returned an unknown controller recovery event");
      switch (decoded.name) {
        case "Registered":
          if (canonicalAddress(decoded.args.owner, "registered controller owner") === brandingOwner) {
            candidates.add(decoded.args.agentId.toString());
          }
          break;
        case "Transfer": {
          const from = canonicalAddress(decoded.args.from, "controller transfer sender");
          const to = canonicalAddress(decoded.args.to, "controller transfer recipient");
          if (from === brandingOwner || to === brandingOwner) {
            candidates.add(decoded.args.tokenId.toString());
          }
          break;
        }
        case "Approval": {
          const owner = canonicalAddress(decoded.args.owner, "controller approval owner");
          const approved = canonicalAddress(decoded.args.approved, "controller approval recipient");
          if (owner === brandingOwner || approved === brandingOwner) {
            candidates.add(decoded.args.tokenId.toString());
          }
          break;
        }
        case "ApprovalForAll":
          if (
            canonicalAddress(decoded.args.operator, "controller approval operator") === brandingOwner &&
            decoded.args.approved === true
          ) {
            // The approval can expose arbitrarily old owner identities that the browser cannot
            // enumerate from this forward interval. Freeze until Agent0 indexes the relation.
            throw new Error("post-index blanket approval requires a refreshed complete directory");
          }
          break;
        case "MetadataSet":
        case "URIUpdated":
          // Either event can turn a pre-existing lower ID into exact current Tentacle evidence.
          // Direct canonical reads below decide whether it belongs to this wallet/component.
          candidates.add(decoded.args.agentId.toString());
          break;
        default:
          throw new Error("Base returned an unsupported controller recovery event");
      }
      if (candidates.size > MAX_CONTROLLER_DIRECTORY_CANDIDATES) {
        throw new Error("controller recovery found too many canonical candidates");
      }
    }
  }
  return candidates;
}

async function resolveRotatedTentacle(
  config: AppConfig,
  identity: StoredIdentity,
  options: AssignmentOptions,
): Promise<TentacleAssignment> {
  const discover = options.discoverRotation ?? (() => discoverDirectoryCandidates(options));
  const target = canonicalAddress(config.rotationAnchor, "retained Tentacle");
  const candidates = (await discover()).filter((candidate) =>
    canonicalAddress(candidate.wallet, "retained Tentacle candidate") === target);
  if (candidates.length !== 1) throw new Error("the retained live Tentacle is no longer uniquely discoverable");
  return verifyLivenessCandidate(config, identity, candidates[0]!, options);
}

export async function verifyLivenessCandidate(
  config: AppConfig,
  identity: StoredIdentity,
  candidate: RotationCandidate,
  options: Pick<AssignmentOptions, "rpc" | "fetch" | "hashCode"> = {},
): Promise<Extract<TentacleAssignment, { source: "rotation-verified" }>> {
  try {
    const rpc = options.rpc ?? createJsonRpcClient(config.baseRpcEndpoint, options.fetch ?? fetch);
    if (quantity(await rpc.request("eth_chainId", []), "Base chain ID") !== BigInt(BASE_CHAIN_ID)) {
      throw new Error("RPC returned the wrong chain");
    }
    const blockNumber = quantity(await rpc.request("eth_blockNumber", []), "Base block number");
    const blockTag = `0x${blockNumber.toString(16)}`;
    const observedBlock = block(await rpc.request("eth_getBlockByNumber", [blockTag, false]));
    if (observedBlock.number !== blockNumber) throw new Error("RPC returned a mismatched Base block");
    const brandingContract = config.brandingContract;
    if (!brandingContract) throw new Error("the canonical Branding deployment is unavailable");
    if (brandingContract !== CANONICAL_BRANDING_CONTRACT) {
      throw new Error("Branding is not the canonical deployment");
    }
    const [brandingCode, registryCode, uwuCode] = await Promise.all([
      rpc.request("eth_getCode", [brandingContract, blockTag]),
      rpc.request("eth_getCode", [IDENTITY_REGISTRY, blockTag]),
      rpc.request("eth_getCode", [UWU_CONTRACT, blockTag]),
    ]);
    if (!isCode(brandingCode) ||
        (options.hashCode ?? keccak256)(String(brandingCode)).toLowerCase() !== CANONICAL_BRANDING_RUNTIME_HASH ||
        !isCode(registryCode) || !isCode(uwuCode)) {
      throw new Error("canonical contract code is unavailable or changed");
    }
    const [brandingResult, brandingChain, brandingRegistry, brandingUwu, brandingVersion,
      version, walletResult, authorizedResult, allegianceResult, protocolResult, uriResult] =
      await Promise.all([
        contractCall(rpc, BRANDING_INTERFACE, brandingContract, "brandingOf", [identity.address], blockTag),
        contractCall(rpc, BRANDING_INTERFACE, brandingContract, "BASE_CHAIN_ID", [], blockTag),
        contractCall(rpc, BRANDING_INTERFACE, brandingContract, "IDENTITY_REGISTRY", [], blockTag),
        contractCall(rpc, BRANDING_INTERFACE, brandingContract, "UWU", [], blockTag),
        contractCall(rpc, BRANDING_INTERFACE, brandingContract, "REGISTRY_VERSION", [], blockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getVersion", [], blockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getAgentWallet", [candidate.agentId], blockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "isAuthorizedOrOwner", [candidate.wallet, candidate.agentId], blockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getMetadata", [candidate.agentId, "cthuwu.allegiance"], blockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "getMetadata", [candidate.agentId, "cthuwu.protocol"], blockTag),
        contractCall(rpc, REGISTRY_INTERFACE, IDENTITY_REGISTRY, "tokenURI", [candidate.agentId], blockTag),
      ]);
    const branding = brandingResult[0] as unknown as {
      tokenId: bigint; acolyte: string; owner: string; controllerAgentId: bigint;
      referrer: string; declaredPrice: bigint; paidThrough: bigint;
      pendingDeclaredPrice: bigint; pendingPriceActivation: bigint; status: bigint;
    };
    const acolyte = canonicalAddress(identity.address, "StoredIdentity address");
    const wallet = canonicalAddress(candidate.wallet, "live Tentacle wallet");
    if (
      brandingChain[0] !== BigInt(BASE_CHAIN_ID) ||
      canonicalAddress(brandingRegistry[0], "Branding identity registry") !== IDENTITY_REGISTRY ||
      canonicalAddress(brandingUwu[0], "Branding UWU") !== UWU_CONTRACT || brandingVersion[0] !== "2.0.0" ||
      branding.tokenId !== BigInt(acolyte) || canonicalAddress(branding.acolyte, "Branding acolyte") !== acolyte ||
      canonicalAddress(branding.owner, "Branding owner") !== ZERO_ADDRESS || branding.controllerAgentId !== 0n ||
      canonicalAddress(branding.referrer, "Branding referrer") !== ZERO_ADDRESS || branding.declaredPrice !== 0n ||
      branding.paidThrough !== 0n || branding.pendingDeclaredPrice !== 0n ||
      branding.pendingPriceActivation !== 0n || branding.status !== 0n ||
      version[0] !== "2.0.0" || canonicalAddress(walletResult[0], "live agentWallet") !== wallet ||
      authorizedResult[0] !== true || String(allegianceResult[0]).toLowerCase() !== ALLEGIANCE_HEX ||
      String(protocolResult[0]).toLowerCase() !== PROTOCOL_V1_HEX
    ) throw new Error("the live Tentacle or Acolyte is no longer canonically eligible");
    const profile = verifiedProfileFromAgentUri(String(uriResult[0]), candidate.agentId);
    if (profile.inboxId !== candidate.inboxId) throw new Error("the live Tentacle endpoint changed");
    await verifyUnchangedBlock(rpc, blockTag, observedBlock);
    return {
      source: "rotation-verified", address: wallet, wallet, agentId: candidate.agentId,
      inboxId: profile.inboxId, name: profile.name, blockNumber, blockHash: observedBlock.hash,
      notice: `Live ${candidate.agentId} Tentacle verified at Base block ${blockNumber}`,
    };
  } catch (error) {
    if (error instanceof RegistryUnavailableError) throw error;
    throw new RegistryUnavailableError(error instanceof Error
      ? `Live Tentacle verification is unavailable: ${error.message}`
      : undefined);
  }
}

async function resolveAnchoredTentacle(
  config: AppConfig,
  identity: StoredIdentity,
  options: AssignmentOptions,
): Promise<TentacleAssignment> {
  try {
    const wallet = canonicalAddress(config.tentacleAnchor, "t link target");
    if (wallet === ZERO_ADDRESS) throw new Error("t link target is zero");
    const discover = options.discoverAnchor ?? (async () => discoverDirectoryCandidates(options));
    const candidates = (await discover(wallet)).filter((candidate) =>
      canonicalAddress(candidate.wallet, "t link candidate") === wallet);
    if (candidates.length === 0) {
      throw new Error("no discoverable Cthuwu ERC-8004 identity belongs to the t address");
    }
    candidates.sort((a, b) => {
      const left = BigInt(a.agentId);
      const right = BigInt(b.agentId);
      return left === right ? 0 : left < right ? -1 : 1;
    });
    const selected = candidates[0]!;
    const verified = await verifyLivenessCandidate(config, identity, selected, options);
    return {
      ...verified,
      source: "anchor-verified",
      notice: `Deep-linked Tentacle canonically verified at Base block ${verified.blockNumber}`,
    };
  } catch (error) {
    if (error instanceof RegistryUnavailableError) throw error;
    throw new RegistryUnavailableError(error instanceof Error ? `Tentacle link could not be verified: ${error.message}` : undefined);
  }
}

async function discoverDirectoryCandidates(options: AssignmentOptions): Promise<RotationCandidate[]> {
  const cached = readLeaderboardCache(localStorage);
  const endpoint = parseLeaderboardConfig().graphEndpoint;
  const directory = cached?.sourceBlockHash
    ? { sourceBlockNumber: cached.sourceBlockNumber, sourceBlockHash: cached.sourceBlockHash,
        identities: cached.rankedWallets.flatMap((group) => group.identities) }
    : endpoint ? await fetchTentacleDirectory(endpoint, { fetch: options.fetch }) : undefined;
  if (!directory) return [];
  // Canonicalize the complete directory before deriving routes. Grouping by raw
  // agentWallet first would split a suspended lower identity (zero agentWallet,
  // current owner retained) from a higher active alias and could route the alias.
  // Distinct strongly evidenced Tentacles on one wallet remain separate candidates
  // so the caller can reject that genuinely ambiguous shared-wallet assignment.
  const canonical = canonicalizeWalletIdentities(directory.identities);
  return canonical.identities.flatMap((candidate) => {
    const wallet = canonicalControlWallet(candidate);
    const endpoint = candidate.profile.xmtpEndpoint;
    if (
      wallet === undefined ||
      !candidate.profile.active ||
      candidate.protocolHex !== PROTOCOL_V1_HEX ||
      endpoint === undefined
    ) return [];
    return [{
      wallet,
      agentId: candidate.agentId,
      inboxId: endpoint.slice("xmtp://".length),
      blockNumber: directory.sourceBlockNumber,
      blockHash: directory.sourceBlockHash,
    }];
  });
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
  return verifiedProfileFromAgentUri(agentUri, agentId).inboxId;
}

function verifiedProfileFromAgentUri(
  agentUri: string,
  agentId: string,
): { inboxId: string; name: string; tentacleId: string } {
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
  return {
    inboxId: endpoint.slice("xmtp://".length),
    name: verifiedProfileName(profile.name) ?? `Tentacle #${agentId}`,
    tentacleId: manifest.tentacleId,
  };
}

function metadataTentacleId(value: unknown): string | undefined {
  if (value === "0x") return undefined;
  if (typeof value !== "string" || !/^0x(?:[0-9a-fA-F]{2}){1,128}$/u.test(value)) {
    throw new Error("controller Tentacle metadata is malformed");
  }
  let decoded: string;
  try {
    decoded = toUtf8String(value);
  } catch {
    throw new Error("controller Tentacle metadata is not UTF-8");
  }
  if (
    decoded.length === 0 ||
    new TextEncoder().encode(decoded).length > 128 ||
    [...decoded].some((character) => {
      const code = character.codePointAt(0) ?? 0;
      return code <= 0x1f || (code >= 0x7f && code <= 0x9f);
    })
  ) throw new Error("controller Tentacle metadata is unsafe");
  return decoded;
}

function verifiedProfileName(value: unknown): string | undefined {
  if (
    typeof value !== "string" ||
    value.trim().length === 0 ||
    new TextEncoder().encode(value).length > 128
  ) return undefined;
  for (const character of value) {
    const code = character.codePointAt(0) ?? 0;
    if (
      code <= 0x1f ||
      (code >= 0x7f && code <= 0x9f) ||
      code === 0x061c ||
      (code >= 0x200b && code <= 0x200f) ||
      (code >= 0x202a && code <= 0x202e) ||
      (code >= 0x2060 && code <= 0x206f) ||
      code === 0xfeff
    ) return undefined;
  }
  return value;
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
