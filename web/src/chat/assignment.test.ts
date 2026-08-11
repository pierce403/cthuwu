import { Interface, getAddress } from "ethers";
import { describe, expect, it, vi } from "vitest";
import type { AppConfig } from "../config";
import type { StoredIdentity } from "../identity";
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
  "function getVersion() view returns (string)",
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
  botAddress: "0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db",
  baseRpcEndpoint: "https://mainnet.base.org/",
  assignmentRefreshMs: 600_000,
};

function dataUri(value: unknown): string {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return `data:application/json;base64,${btoa(binary)}`;
}

function profile(environment = "production"): string {
  const endpoint = `xmtp://${inbox}`;
  const registry = getAddress(IDENTITY_REGISTRY);
  const manifest = dataUri({
    schemaVersion: 1,
    protocol: 1,
    tentacleId: "fixture-tentacle",
    erc8004: { chainId: 8453, registry, agentId: "42" },
    xmtp: { environment, endpoint },
    capabilities: ["direct-xmtp-messaging"],
  });
  return dataUri({
    type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
    name: "Fixture Tentacle",
    description: "fixture",
    image: "ipfs://fixture",
    services: [
      { name: "CTHUWU-XMTP", endpoint, version: "1" },
      { name: "CTHUWU", endpoint: manifest, version: "1" },
    ],
    x402Support: false,
    active: true,
    registrations: [{ agentId: 42, agentRegistry: `eip155:8453:${registry}` }],
  });
}

function canonicalRpc(options: {
  status?: number;
  acolyte?: string;
  finalHash?: string;
  profileUri?: string;
  brandingUwu?: string;
} = {}) {
  const status = options.status ?? 1;
  const owner = status === 0 ? ZERO_ADDRESS : "0x3333333333333333333333333333333333333333";
  const firstHash = `0x${"a".repeat(64)}`;
  let blockReads = 0;
  const brandingCalls: string[] = [];
  return {
    brandingCalls,
    request: async (method: string, params: unknown[]): Promise<unknown> => {
      if (method === "eth_chainId") return "0x2105";
      if (method === "eth_blockNumber") return "0x7b";
      if (method === "eth_getBlockByNumber") {
        blockReads += 1;
        return { number: "0x7b", hash: blockReads > 1 ? options.finalHash ?? firstHash : firstHash };
      }
      if (method === "eth_getCode") return "0x6000";
      if (method !== "eth_call") throw new Error(`unexpected ${method}`);
      const call = params[0] as { to: string; data: string };
      if (call.to.toLowerCase() !== IDENTITY_REGISTRY) {
        const brandingSelector = call.data.slice(0, 10);
        if (brandingSelector === brandingInterface.getFunction("brandingOf")!.selector) {
          brandingCalls.push(call.data);
          return brandingInterface.encodeFunctionResult("brandingOf", [[
            BigInt(identity.address), options.acolyte ?? identity.address, owner, 42n,
            ZERO_ADDRESS, 1n, 1n, 0n, 0n, status,
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
      if (selector === registryInterface.getFunction("getVersion")!.selector) {
        return registryInterface.encodeFunctionResult("getVersion", ["2.0.0"]);
      }
      if (selector === registryInterface.getFunction("getAgentWallet")!.selector) {
        return registryInterface.encodeFunctionResult("getAgentWallet", [owner]);
      }
      if (selector === registryInterface.getFunction("isAuthorizedOrOwner")!.selector) {
        return registryInterface.encodeFunctionResult("isAuthorizedOrOwner", [true]);
      }
      if (selector === registryInterface.getFunction("getMetadata")!.selector) {
        const [, key] = registryInterface.decodeFunctionData("getMetadata", call.data);
        return registryInterface.encodeFunctionResult("getMetadata", [
          key === "cthuwu.allegiance" ? ALLEGIANCE_HEX : PROTOCOL_V1_HEX,
        ]);
      }
      if (selector === registryInterface.getFunction("tokenURI")!.selector) {
        return registryInterface.encodeFunctionResult("tokenURI", [options.profileUri ?? profile()]);
      }
      throw new Error("unexpected registry call");
    },
  };
}

describe("canonical Tentacle assignment", () => {
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
    const config = {
      ...baseConfig,
      brandingContract: "0x2222222222222222222222222222222222222222",
    };
    const rpc = { request: async () => { throw new Error("RPC offline"); } };
    await expect(resolveTentacleAssignment(config, identity, { rpc })).rejects.toBeInstanceOf(
      RegistryUnavailableError,
    );
  });

  it("rejects a mismatched JSON-RPC response ID", async () => {
    const client = createJsonRpcClient("https://rpc.example/", vi.fn(async () => new Response(
      JSON.stringify({ jsonrpc: "2.0", id: 999, result: "0x2105" }),
      { status: 200, headers: { "content-type": "application/json" } },
    )) as typeof fetch);
    await expect(client.request("eth_chainId", [])).rejects.toThrow(/invalid response/u);
  });

  it("resolves an Active Branding controller from one stable explicit Base block", async () => {
    const config = { ...baseConfig, brandingContract: "0x2222222222222222222222222222222222222222" };
    const rpc = canonicalRpc();
    await expect(resolveTentacleAssignment(config, identity, { rpc })).resolves.toMatchObject({
      source: "branding-active",
      inboxId: inbox,
      agentId: "42",
      wallet: "0x3333333333333333333333333333333333333333",
      blockNumber: 123n,
      blockHash: `0x${"a".repeat(64)}`,
    });
    const [queriedAcolyte] = brandingInterface.decodeFunctionData("brandingOf", rpc.brandingCalls[0]!);
    expect(String(queriedAcolyte).toLowerCase()).toBe(identity.address);
  });

  it.each([
    [0, "Unminted"],
    [2, "Expired"],
    [3, "Ineligible"],
  ])("uses intro only for canonical non-active status %s (%s)", async (status, label) => {
    const config = { ...baseConfig, brandingContract: "0x2222222222222222222222222222222222222222" };
    await expect(resolveTentacleAssignment(config, identity, { rpc: canonicalRpc({ status }) })).resolves.toMatchObject({
      source: "intro-fallback",
      brandingStatus: label,
      blockNumber: 123n,
      blockHash: `0x${"a".repeat(64)}`,
    });
  });

  it("freezes on tuple spoofing, positive RegistryUnavailable, or a reorg", async () => {
    const config = { ...baseConfig, brandingContract: "0x2222222222222222222222222222222222222222" };
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({ acolyte: "0x4444444444444444444444444444444444444444" }),
    })).rejects.toThrow(RegistryUnavailableError);
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({ status: 4 }),
    })).rejects.toThrow(RegistryUnavailableError);
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({ finalHash: `0x${"b".repeat(64)}` }),
    })).rejects.toThrow(/changed during assignment/u);
    await expect(resolveTentacleAssignment(config, identity, {
      rpc: canonicalRpc({ brandingUwu: "0x4444444444444444444444444444444444444444" }),
    })).rejects.toThrow(RegistryUnavailableError);
  });
});
