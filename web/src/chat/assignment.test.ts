import { Interface, getAddress, hexlify, toUtf8Bytes } from "ethers";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  CANONICAL_BRANDING_CONTRACT,
  CANONICAL_BRANDING_RUNTIME_HASH,
  type AppConfig,
} from "../config";
import type { StoredIdentity } from "../identity";
import { writeLeaderboardCache } from "../leaderboard-cache";
import { cachedSnapshot } from "../leaderboard-test-data";
import {
  ALLEGIANCE_HEX,
  IDENTITY_REGISTRY,
  PROTOCOL_V1_HEX,
  UWU_CONTRACT,
  ZERO_ADDRESS,
} from "../leaderboard-types";
import {
  RegistryUnavailableError,
  createJsonRpcClient,
  endpointFromAgentUri,
  resolveTentacleAssignment,
} from "./assignment";

const brandingInterface = new Interface([
  "function brandingOf(address acolyte) view returns (tuple(uint256 tokenId,address acolyte,address owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 pendingDeclaredPrice,uint256 pendingPriceActivation,uint8 status) result)",
  "function BASE_CHAIN_ID() view returns (uint256)",
  "function IDENTITY_REGISTRY() view returns (address)",
  "function UWU() view returns (address)",
  "function REGISTRY_VERSION() view returns (string)",
]);
const registryInterface = new Interface([
  "event MetadataSet(uint256 indexed agentId,string indexed indexedMetadataKey,string metadataKey,bytes metadataValue)",
  "function getVersion() view returns (string)",
  "function ownerOf(uint256 agentId) view returns (address)",
  "function getAgentWallet(uint256 agentId) view returns (address)",
  "function isAuthorizedOrOwner(address wallet,uint256 agentId) view returns (bool)",
  "function getMetadata(uint256 agentId,string key) view returns (bytes)",
  "function tokenURI(uint256 tokenId) view returns (string)",
]);

const inbox = "a".repeat(64);
const identity = {
  version: 1,
  environment: "production",
  address: "0x1111111111111111111111111111111111111111",
  walletPrivateKey: `0x${"12".repeat(32)}`,
  compatibilityDbKey: `0x${"34".repeat(32)}`,
  createdAt: "2026-08-11T00:00:00.000Z",
} satisfies StoredIdentity;
const baseConfig: AppConfig = {
  environment: "production",
  botAddress: "0x4c4e26a4683f6b6d63e19abf0bedd1171b9a6e90",
  baseRpcEndpoint: "https://mainnet.base.org/",
  assignmentRefreshMs: 600_000,
};
const configuredConfig: AppConfig = {
  ...baseConfig,
  brandingContract: CANONICAL_BRANDING_CONTRACT,
};
const canonicalCodeHash = () => CANONICAL_BRANDING_RUNTIME_HASH;

function dataUri(value: unknown): string {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return `data:application/json;base64,${btoa(binary)}`;
}

function profile(
  environment = "production",
  name = "Fixture Tentacle",
  agentId = "42",
  tentacleId = "fixture-tentacle",
): string {
  const endpoint = `xmtp://${inbox}`;
  const registry = getAddress(IDENTITY_REGISTRY);
  const manifest = dataUri({
    schemaVersion: 1,
    protocol: 1,
    tentacleId,
    erc8004: { chainId: 8453, registry, agentId },
    xmtp: { environment, endpoint },
    capabilities: ["direct-xmtp-messaging"],
  });
  return dataUri({
    type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
    name,
    description: "fixture",
    image: "ipfs://fixture",
    services: [
      { name: "CTHUWU-XMTP", endpoint, version: "1" },
      { name: "CTHUWU", endpoint: manifest, version: "1" },
    ],
    x402Support: false,
    active: true,
    registrations: [{ agentId, agentRegistry: `eip155:8453:${registry}` }],
  });
}

function canonicalRpc(options: {
  status?: number;
  acolyte?: string;
  finalHash?: string;
  profileUri?: string;
  brandingUwu?: string;
  registryWallet?: string;
  profilesByAgentId?: Record<string, string>;
  metadataTentacleIdsByAgentId?: Record<string, string | undefined>;
  gapLogs?: unknown[];
  ownersByAgentId?: Record<string, string>;
  walletsByAgentId?: Record<string, string>;
  authorizedByAgentId?: Record<string, boolean>;
} = {}) {
  const status = options.status ?? 1;
  const owner = status === 0 ? ZERO_ADDRESS : "0x3333333333333333333333333333333333333333";
  const controllerAgentId = status === 0 ? 0n : 42n;
  const activePayment = status === 0 ? 0n : 1n;
  const firstHash = `0x${"a".repeat(64)}`;
  let blockReads = 0;
  const brandingCalls: string[] = [];
  const registryCalls: string[] = [];
  return {
    brandingCalls,
    registryCalls,
    request: async (method: string, params: unknown[]): Promise<unknown> => {
      if (method === "eth_chainId") return "0x2105";
      if (method === "eth_blockNumber") return "0x7b";
      if (method === "eth_getBlockByNumber") {
        if (params[0] === "0x7a") {
          return { number: "0x7a", hash: `0x${"b".repeat(64)}` };
        }
        blockReads += 1;
        return { number: "0x7b", hash: blockReads > 2 ? options.finalHash ?? firstHash : firstHash };
      }
      if (method === "eth_getLogs") return options.gapLogs ?? [];
      if (method === "eth_getCode") return "0x6000";
      if (method !== "eth_call") throw new Error(`unexpected ${method}`);
      const call = params[0] as { to: string; data: string };
      if (call.to.toLowerCase() !== IDENTITY_REGISTRY) {
        const brandingSelector = call.data.slice(0, 10);
        if (brandingSelector === brandingInterface.getFunction("brandingOf")!.selector) {
          brandingCalls.push(call.data);
          return brandingInterface.encodeFunctionResult("brandingOf", [[
            BigInt(identity.address), options.acolyte ?? identity.address, owner, controllerAgentId,
            ZERO_ADDRESS, activePayment, activePayment, 0n, 0n, status,
          ]]);
        }
        if (brandingSelector === brandingInterface.getFunction("BASE_CHAIN_ID")!.selector) {
          return brandingInterface.encodeFunctionResult("BASE_CHAIN_ID", [8453n]);
        }
        if (brandingSelector === brandingInterface.getFunction("IDENTITY_REGISTRY")!.selector) {
          return brandingInterface.encodeFunctionResult("IDENTITY_REGISTRY", [IDENTITY_REGISTRY]);
        }
        if (brandingSelector === brandingInterface.getFunction("UWU")!.selector) {
          return brandingInterface.encodeFunctionResult("UWU", [options.brandingUwu ?? UWU_CONTRACT]);
        }
        if (brandingSelector === brandingInterface.getFunction("REGISTRY_VERSION")!.selector) {
          return brandingInterface.encodeFunctionResult("REGISTRY_VERSION", ["2.0.0"]);
        }
        throw new Error("unexpected Branding call");
      }
      const selector = call.data.slice(0, 10);
      registryCalls.push(call.data);
      if (selector === registryInterface.getFunction("getVersion")!.selector) {
        return registryInterface.encodeFunctionResult("getVersion", ["2.0.0"]);
      }
      if (selector === registryInterface.getFunction("ownerOf")!.selector) {
        const [agentId] = registryInterface.decodeFunctionData("ownerOf", call.data);
        return registryInterface.encodeFunctionResult("ownerOf", [
          options.ownersByAgentId?.[agentId.toString()] ?? owner,
        ]);
      }
      if (selector === registryInterface.getFunction("getAgentWallet")!.selector) {
        const [agentId] = registryInterface.decodeFunctionData("getAgentWallet", call.data);
        return registryInterface.encodeFunctionResult("getAgentWallet", [
          options.walletsByAgentId?.[agentId.toString()] ?? options.registryWallet ?? owner,
        ]);
      }
      if (selector === registryInterface.getFunction("isAuthorizedOrOwner")!.selector) {
        const [, agentId] = registryInterface.decodeFunctionData("isAuthorizedOrOwner", call.data);
        return registryInterface.encodeFunctionResult("isAuthorizedOrOwner", [
          options.authorizedByAgentId?.[agentId.toString()] ?? true,
        ]);
      }
      if (selector === registryInterface.getFunction("getMetadata")!.selector) {
        const [agentId, key] = registryInterface.decodeFunctionData("getMetadata", call.data);
        const metadataTentacleId = options.metadataTentacleIdsByAgentId?.[agentId.toString()];
        return registryInterface.encodeFunctionResult("getMetadata", [
          key === "cthuwu.allegiance" ? ALLEGIANCE_HEX :
            key === "cthuwu.protocol" ? PROTOCOL_V1_HEX :
              metadataTentacleId === undefined
                ? hexlify(toUtf8Bytes("fixture-tentacle"))
                : hexlify(toUtf8Bytes(metadataTentacleId)),
        ]);
      }
      if (selector === registryInterface.getFunction("tokenURI")!.selector) {
        const [agentId] = registryInterface.decodeFunctionData("tokenURI", call.data);
        return registryInterface.encodeFunctionResult("tokenURI", [
          options.profilesByAgentId?.[agentId.toString()] ?? options.profileUri ?? profile(),
        ]);
      }
      throw new Error("unexpected registry call");
    },
  };
}

function controllerDirectory(
  ...agentIds: string[]
) {
  const template = cachedSnapshot().rankedWallets[0]!.identities[0]!;
  return async () => ({
    sourceBlockNumber: "123",
    sourceBlockHash: `0x${"a".repeat(64)}`,
    identities: agentIds.map((agentId) => ({
      ...structuredClone(template),
      agentId,
      owner: "0x3333333333333333333333333333333333333333",
      agentWallet: "0x3333333333333333333333333333333333333333",
      tentacleId: "fixture-tentacle",
    })),
  });
}

function controllerDirectoryAt122(...agentIds: string[]) {
  const discover = controllerDirectory(...agentIds);
  return async () => ({
    ...await discover(),
    sourceBlockNumber: "122",
    sourceBlockHash: `0x${"b".repeat(64)}`,
  });
}

function noCacheDuplicateDirectoryFetch(options: {
  lowerTentacleId?: string;
  higherTentacleId?: string;
  lowerAgentWallet?: string | null;
  higherAgentWallet?: string | null;
} = {}): typeof fetch {
  const owner = "0x3333333333333333333333333333333333333333";
  const row = (
    agentId: string,
    agentWallet: string | null,
    name: string,
    tentacleId: string,
  ) => ({
    id: `8453:${agentId}:cthuwu.allegiance`,
    key: "cthuwu.allegiance",
    value: ALLEGIANCE_HEX,
    updatedAt: "1770118004",
    agent: {
      id: `8453:${agentId}`,
      chainId: "8453",
      agentId,
      owner,
      agentWallet,
      agentURI: profile("production", name, agentId, tentacleId),
      createdAt: "1770118000",
      updatedAt: "1770118002",
      totalFeedback: "0",
      metadata: [
        { id: `${agentId}-a`, key: "cthuwu.allegiance", value: ALLEGIANCE_HEX, updatedAt: "1770118004" },
        { id: `${agentId}-p`, key: "cthuwu.protocol", value: PROTOCOL_V1_HEX, updatedAt: "1770118004" },
        {
          id: `${agentId}-t`,
          key: "cthuwu.tentacle-id",
          value: hexlify(toUtf8Bytes(tentacleId)),
          updatedAt: "1770118004",
        },
      ],
      registrationFile: null,
      feedback: [],
    },
  });
  const lowerTentacleId = options.lowerTentacleId ?? "fixture-tentacle";
  const higherTentacleId = options.higherTentacleId ?? lowerTentacleId;
  return vi.fn(async () => new Response(JSON.stringify({
    data: {
      agentMetadatas: [
        row(
          "41",
          options.lowerAgentWallet === undefined ? null : options.lowerAgentWallet,
          "Suspended canonical",
          lowerTentacleId,
        ),
        row(
          "42",
          options.higherAgentWallet === undefined ? owner : options.higherAgentWallet,
          "Active duplicate",
          higherTentacleId,
        ),
      ],
      _meta: {
        block: { number: 122, hash: `0x${"b".repeat(64)}`, timestamp: 1786332360 },
        deployment: "QmAgent0Fixture",
        hasIndexingErrors: false,
      },
    },
  }), {
    status: 200,
    headers: { "content-type": "application/json" },
  })) as typeof fetch;
}

describe("canonical Tentacle assignment", () => {
  beforeEach(() => localStorage.clear());
  it("keeps explicitly unconfigured Branding on the intro continuity route", async () => {
    await expect(resolveTentacleAssignment(baseConfig, identity)).resolves.toMatchObject({
      source: "intro-unconfigured",
      address: baseConfig.botAddress,
      notice: expect.stringContaining("pending deployment"),
    });
  });

  it("parses the exact production CTHUWU manifest emitted by the agent", () => {
    expect(endpointFromAgentUri(profile(), "42")).toBe(inbox);
    expect(() => endpointFromAgentUri(profile("dev"), "42")).toThrow(/manifest/u);
  });

  it("matches the agent's 8 KiB registration profile and manifest bound", () => {
    const oversizedProfile = dataUri({ padding: "x".repeat(8 * 1024) });
    expect(new TextEncoder().encode(oversizedProfile).length).toBeGreaterThan(8 * 1024);
    expect(() => endpointFromAgentUri(oversizedProfile, "42")).toThrow(/bounded data URI/u);
  });

  it("treats a configured registry outage as retryable instead of abandonment", async () => {
    const rpc = { request: async () => { throw new Error("RPC offline"); } };
    await expect(resolveTentacleAssignment(configuredConfig, identity, { rpc })).rejects.toBeInstanceOf(
      RegistryUnavailableError,
    );
  });

  it("rejects an alternate Branding address before consulting Base", async () => {
    const rpc = { request: vi.fn() };
    await expect(resolveTentacleAssignment({
      ...baseConfig,
      brandingContract: "0x2222222222222222222222222222222222222222",
    }, identity, { rpc })).rejects.toThrow(/canonical deployment/u);
    expect(rpc.request).not.toHaveBeenCalled();
  });

  it("rejects a mismatched JSON-RPC response ID", async () => {
    const client = createJsonRpcClient("https://rpc.example/", vi.fn(async () => new Response(
      JSON.stringify([{ jsonrpc: "2.0", id: 999, result: "0x2105" }]),
      { status: 200, headers: { "content-type": "application/json" } },
    )) as typeof fetch);
    await expect(client.request("eth_chainId", [])).rejects.toThrow(/invalid response/u);
  });

  it("micro-batches concurrent Base reads into one HTTP request", async () => {
    const fetcher = vi.fn(async (_input, init) => {
      const requests = JSON.parse(String(init?.body)) as Array<{ id: number; method: string }>;
      return new Response(JSON.stringify(requests.map((request) => ({
        jsonrpc: "2.0",
        id: request.id,
        result: request.method === "eth_chainId" ? "0x2105" : "0x7b",
      }))));
    }) as typeof fetch;
    const client = createJsonRpcClient("https://rpc.example/", fetcher);
    await expect(Promise.all([
      client.request("eth_chainId", []),
      client.request("eth_blockNumber", []),
    ])).resolves.toEqual(["0x2105", "0x7b"]);
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("resolves an Active Branding controller from one stable explicit Base block", async () => {
    const rpc = canonicalRpc();
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc,
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: controllerDirectory("42"),
    })).resolves.toMatchObject({
      source: "branding-active",
      inboxId: inbox,
      agentId: "42",
      wallet: "0x3333333333333333333333333333333333333333",
      name: "Fixture Tentacle",
      blockNumber: 123n,
      blockHash: `0x${"a".repeat(64)}`,
    });
    const [queriedAcolyte] = brandingInterface.decodeFunctionData("brandingOf", rpc.brandingCalls[0]!);
    expect(String(queriedAcolyte).toLowerCase()).toBe(identity.address);
  });

  it("maps a higher stored Branding controller to a directly proven lower canonical alias", async () => {
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc: canonicalRpc({
        profilesByAgentId: {
          "41": profile("production", "Canonical Tentacle", "41"),
          "42": profile("production", "Duplicate Tentacle", "42"),
        },
      }),
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: controllerDirectory("41", "42"),
    })).resolves.toMatchObject({
      source: "branding-active",
      agentId: "41",
      name: "Canonical Tentacle",
      inboxId: inbox,
    });
  });

  it("does not miss a lower exact duplicate controlled by a pre-index blanket approval", async () => {
    const discover = controllerDirectory("41", "42");
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc: canonicalRpc({
        ownersByAgentId: {
          "41": "0x4444444444444444444444444444444444444444",
        },
        walletsByAgentId: {
          "41": "0x5555555555555555555555555555555555555555",
        },
        // Models the Branding owner retaining current operator authority granted before the
        // pinned directory block. Exact Tentacle evidence must still bring #41 to direct reads.
        authorizedByAgentId: { "41": true },
        profilesByAgentId: {
          "41": profile("production", "Operator-controlled canonical", "41"),
          "42": profile("production", "Stored duplicate", "42"),
        },
      }),
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: async () => {
        const directory = await discover();
        Object.assign(directory.identities[0]!, {
          owner: "0x4444444444444444444444444444444444444444",
          agentWallet: "0x5555555555555555555555555555555555555555",
        });
        return directory;
      },
    })).rejects.toThrow(/canonical Branding controller alias is not currently eligible/u);
  });

  it("freezes instead of aliasing a Branding controller across conflicting Tentacle IDs", async () => {
    const snapshot = cachedSnapshot();
    const canonical = snapshot.rankedWallets[0]!.identities[0]!;
    Object.assign(canonical, {
      agentId: "41",
      agentWallet: "0x3333333333333333333333333333333333333333",
      tentacleId: "fixture-tentacle",
    });
    snapshot.rankedWallets[0]!.wallet = canonical.agentWallet;
    snapshot.rankedWallets[0]!.representativeAgentId = "41";
    snapshot.rankedWallets[0]!.identities.push({ ...structuredClone(canonical), agentId: "42" });
    expect(writeLeaderboardCache(localStorage, snapshot)).toBe(true);
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc: canonicalRpc({
        profilesByAgentId: {
          "41": profile("production", "Canonical Tentacle", "41", "conflicting-tentacle"),
          "42": profile("production", "Duplicate Tentacle", "42", "fixture-tentacle"),
        },
      }),
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: controllerDirectory("41", "42"),
    })).rejects.toThrow(/identity evidence conflicts/u);
  });

  it("ignores a stale public cache and uses the complete pinned directory", async () => {
    const snapshot = cachedSnapshot();
    const cachedCanonical = snapshot.rankedWallets[0]!.identities[0]!;
    Object.assign(cachedCanonical, {
      agentId: "40",
      agentWallet: "0x3333333333333333333333333333333333333333",
      tentacleId: "fixture-tentacle",
    });
    snapshot.rankedWallets[0]!.wallet = cachedCanonical.agentWallet;
    snapshot.rankedWallets[0]!.representativeAgentId = "40";
    snapshot.rankedWallets[0]!.identities.push({
      ...structuredClone(cachedCanonical),
      agentId: "42",
    });
    expect(writeLeaderboardCache(localStorage, snapshot)).toBe(true);
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc: canonicalRpc({
        profilesByAgentId: {
          "41": profile("production", "Fresh canonical", "41"),
          "42": profile("production", "Stored duplicate", "42"),
        },
      }),
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: controllerDirectory("41", "42"),
    })).resolves.toMatchObject({
      source: "branding-active",
      agentId: "41",
      name: "Fresh canonical",
    });
  });

  it("bridges post-index identity evidence before choosing the lowest controller", async () => {
    const metadataEvent = registryInterface.encodeEventLog(
      registryInterface.getEvent("MetadataSet")!,
      [41n, "cthuwu.tentacle-id", "cthuwu.tentacle-id", hexlify(toUtf8Bytes("fixture-tentacle"))],
    );
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc: canonicalRpc({
        gapLogs: [metadataEvent],
        profilesByAgentId: {
          "41": profile("production", "Post-index canonical", "41"),
          "42": profile("production", "Stored duplicate", "42"),
        },
      }),
      hashCode: canonicalCodeHash,
      // Agent0 has indexed the higher controller but not the lower identity's metadata update.
      discoverControllerDirectory: controllerDirectoryAt122("42"),
    })).resolves.toMatchObject({
      source: "branding-active",
      agentId: "41",
      name: "Post-index canonical",
    });
  });

  it("freezes active Branding when complete controller discovery is unavailable", async () => {
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc: canonicalRpc(),
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: async () => {
        throw new Error("Agent0 unavailable");
      },
    })).rejects.toThrow(/Agent0 unavailable/u);
  });

  it.each([
    [2, "Expired"],
    [3, "Ineligible"],
  ])("uses intro only for canonical non-active status %s (%s)", async (status, label) => {
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc: canonicalRpc({ status }),
      hashCode: canonicalCodeHash,
      discoverRotation: async () => [],
    })).resolves.toMatchObject({
      source: "intro-fallback",
      brandingStatus: label,
      blockNumber: 123n,
      blockHash: `0x${"a".repeat(64)}`,
    });
  });

  it("requires a live response instead of assigning an unminted identity by address hash", async () => {
    const wallet = "0x3333333333333333333333333333333333333333";
    const rpc = canonicalRpc({ status: 0 });
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc,
      hashCode: canonicalCodeHash,
      discoverRotation: async () => [{
        wallet,
        agentId: "42",
        inboxId: inbox,
        blockNumber: "122",
        blockHash: `0x${"b".repeat(64)}`,
      }],
    })).resolves.toMatchObject({
      source: "liveness-required",
      address: configuredConfig.botAddress,
      brandingStatus: "Unminted",
      blockNumber: 123n,
      blockHash: `0x${"a".repeat(64)}`,
    });
    expect(rpc.registryCalls).toEqual([]);
  });

  it("routes an unbranded deep link only after fresh canonical Base verification", async () => {
    const config = {
      ...configuredConfig,
      tentacleAnchor: "0x3333333333333333333333333333333333333333",
    };
    const rpc = canonicalRpc({ status: 0, registryWallet: config.tentacleAnchor });
    await expect(resolveTentacleAssignment(config, identity, {
      rpc,
      hashCode: canonicalCodeHash,
      discoverAnchor: async () => [{
        wallet: config.tentacleAnchor!, agentId: "42", inboxId: inbox,
        blockNumber: "122", blockHash: `0x${"b".repeat(64)}`,
      }],
    })).resolves.toMatchObject({
      source: "anchor-verified",
      address: config.tentacleAnchor,
      wallet: config.tentacleAnchor,
      agentId: "42",
      inboxId: inbox,
      name: "Fixture Tentacle",
      blockNumber: 123n,
      blockHash: `0x${"a".repeat(64)}`,
    });
    expect(rpc.registryCalls).toHaveLength(6);
  });

  it("normalizes a checksummed directory wallet to the lowercase explicit link target", async () => {
    const target = "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd";
    const discoverAnchor = vi.fn(async () => [{
      wallet: getAddress(target), agentId: "42", inboxId: inbox,
      blockNumber: "122", blockHash: `0x${"b".repeat(64)}`,
    }]);
    await expect(resolveTentacleAssignment({
      ...configuredConfig,
      tentacleAnchor: target,
    }, identity, {
      rpc: canonicalRpc({ status: 0, registryWallet: target }),
      hashCode: canonicalCodeHash,
      discoverAnchor,
    })).resolves.toMatchObject({
      source: "anchor-verified",
      address: target,
      wallet: target,
      name: "Fixture Tentacle",
    });
    expect(discoverAnchor).toHaveBeenCalledWith(target);
  });

  it("canonicalizes a no-cache directory before deep-link routing across a zero-wallet alias", async () => {
    const target = "0x3333333333333333333333333333333333333333";
    const fetcher = noCacheDuplicateDirectoryFetch();
    await expect(resolveTentacleAssignment({
      ...configuredConfig,
      tentacleAnchor: target,
    }, identity, {
      rpc: canonicalRpc({
        status: 0,
        registryWallet: target,
        profilesByAgentId: {
          "41": profile("production", "Recovered canonical", "41"),
          "42": profile("production", "Ignored duplicate", "42"),
        },
      }),
      fetch: fetcher,
      hashCode: canonicalCodeHash,
    })).resolves.toMatchObject({
      source: "anchor-verified",
      address: target,
      wallet: target,
      agentId: "41",
      name: "Recovered canonical",
    });
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("routes only through the lower exact Tentacle across a stale higher agentWallet", async () => {
    const target = "0x4444444444444444444444444444444444444444";
    const fetcher = noCacheDuplicateDirectoryFetch({
      lowerAgentWallet: target,
      higherAgentWallet: "0x5555555555555555555555555555555555555555",
    });
    await expect(resolveTentacleAssignment({
      ...configuredConfig,
      tentacleAnchor: target,
    }, identity, {
      rpc: canonicalRpc({
        status: 0,
        registryWallet: target,
        profilesByAgentId: {
          "41": profile("production", "Canonical controller", "41"),
          "42": profile("production", "Stale duplicate", "42"),
        },
      }),
      fetch: fetcher,
      hashCode: canonicalCodeHash,
    })).resolves.toMatchObject({
      source: "anchor-verified",
      address: target,
      wallet: target,
      agentId: "41",
      name: "Canonical controller",
    });
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("fails no-cache rotation closed when the lower zero-wallet canonical is still ineligible", async () => {
    const target = "0x3333333333333333333333333333333333333333";
    const fetcher = noCacheDuplicateDirectoryFetch();
    await expect(resolveTentacleAssignment({
      ...configuredConfig,
      rotationAnchor: target,
    }, identity, {
      rpc: canonicalRpc({
        status: 0,
        registryWallet: target,
        walletsByAgentId: { "41": ZERO_ADDRESS },
        profilesByAgentId: {
          "41": profile("production", "Suspended canonical", "41"),
          "42": profile("production", "Must not be routed", "42"),
        },
      }),
      fetch: fetcher,
      hashCode: canonicalCodeHash,
    })).rejects.toThrow(/no longer canonically eligible/u);
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("resolves duplicate Tentacles on one effective wallet to the lowest agent ID", async () => {
    const target = "0x3333333333333333333333333333333333333333";
    const higherTentacleId = "genuinely-distinct-tentacle";
    const fetcher = noCacheDuplicateDirectoryFetch({
      higherTentacleId,
    });
    await expect(resolveTentacleAssignment({
      ...configuredConfig,
      tentacleAnchor: target,
    }, identity, {
      rpc: canonicalRpc({
        status: 0,
        registryWallet: target,
        profileUri: profile("production", "Suspended canonical", "41", "fixture-tentacle"),
      }),
      fetch: fetcher,
      hashCode: canonicalCodeHash,
    })).resolves.toMatchObject({
      source: "anchor-verified",
      agentId: "41",
    });
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("fails a deep link closed when the fresh registry wallet no longer matches", async () => {
    const config = {
      ...configuredConfig,
      tentacleAnchor: "0x3333333333333333333333333333333333333333",
    };
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({
        status: 0,
        registryWallet: "0x4444444444444444444444444444444444444444",
      }),
      hashCode: canonicalCodeHash,
      discoverAnchor: async () => [{
        wallet: config.tentacleAnchor!, agentId: "42", inboxId: inbox,
        blockNumber: "122", blockHash: `0x${"b".repeat(64)}`,
      }],
    })).rejects.toThrow(/canonically eligible/u);
  });

  it("fails a deep link closed when its fresh production XMTP endpoint changed", async () => {
    const config = {
      ...configuredConfig,
      tentacleAnchor: "0x3333333333333333333333333333333333333333",
    };
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({
        status: 0,
        registryWallet: config.tentacleAnchor,
        profileUri: profile("dev"),
      }),
      hashCode: canonicalCodeHash,
      discoverAnchor: async () => [{
        wallet: config.tentacleAnchor!, agentId: "42", inboxId: inbox,
        blockNumber: "122", blockHash: `0x${"b".repeat(64)}`,
      }],
    })).rejects.toThrow(/manifest does not bind/u);
  });

  it("falls back to the verified agent ID when the fresh profile name is unsafe", async () => {
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc: canonicalRpc({ profileUri: profile("production", "spoof\u202e") }),
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: controllerDirectory("42"),
    })).resolves.toMatchObject({
      source: "branding-active",
      name: "Tentacle #42",
    });
  });

  it("resolves multiple candidate agent IDs for a t address to the lowest agent ID", async () => {
    const config = {
      ...configuredConfig,
      tentacleAnchor: "0x3333333333333333333333333333333333333333",
    };
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({ status: 0, registryWallet: config.tentacleAnchor }),
      hashCode: canonicalCodeHash,
      discoverAnchor: async () => ["43", "42"].map((agentId) => ({
        wallet: config.tentacleAnchor!, agentId, inboxId: inbox,
        blockNumber: "122", blockHash: `0x${"b".repeat(64)}`,
      })),
    })).resolves.toMatchObject({
      source: "anchor-verified",
      agentId: "42",
    });
  });

  it("freezes on tuple spoofing, positive RegistryUnavailable, or a reorg", async () => {
    const config = configuredConfig;
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({ acolyte: "0x4444444444444444444444444444444444444444" }),
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: controllerDirectory("42"),
    })).rejects.toThrow(RegistryUnavailableError);
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({ status: 4 }),
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: controllerDirectory("42"),
    })).rejects.toThrow(RegistryUnavailableError);
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({ finalHash: `0x${"b".repeat(64)}` }),
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: controllerDirectory("42"),
    })).rejects.toThrow(/changed during assignment/u);
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({ brandingUwu: "0x4444444444444444444444444444444444444444" }),
      hashCode: canonicalCodeHash,
      discoverControllerDirectory: controllerDirectory("42"),
    })).rejects.toThrow(RegistryUnavailableError);
  });

  it("freezes assignment when canonical Branding runtime code changes", async () => {
    await expect(resolveTentacleAssignment(configuredConfig, identity, {
      rpc: canonicalRpc(),
      hashCode: () => `0x${"f".repeat(64)}`,
    })).rejects.toThrow(RegistryUnavailableError);
  });
});
