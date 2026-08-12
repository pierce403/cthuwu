import { Interface } from "ethers";
import { describe, expect, it, vi } from "vitest";

vi.mock("ethers", async (importOriginal) => {
  const actual = await importOriginal<typeof import("ethers")>();
  return {
    ...actual,
    keccak256: (value: import("ethers").BytesLike) =>
      value === "0x6000"
        ? "0x3a22b742a570dc2d030edf2bd82dceda1e068a297e2c36883f97b7b66ed4ef2d"
        : actual.keccak256(value),
  };
});

import {
  ACOLYTE_BRANDING_CONTRACT,
  ACOLYTE_DEPLOYMENT_BLOCK,
  ACOLYTE_DEPLOYMENT_BLOCK_HASH,
  fetchAcolyteCatalog,
} from "./acolyte-data";

const abi = new Interface([
  "function brandingOf(address acolyte) view returns (tuple(uint256 tokenId,address acolyte,address owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 pendingDeclaredPrice,uint256 pendingPriceActivation,uint8 status) result)",
  "function avatarURIOf(uint256 tokenId) view returns (string)",
  "function customTraitCount(uint256 tokenId) view returns (uint256)",
  "function customTraitAt(uint256 tokenId,uint256 index) view returns (string traitType,string value)",
  "event BrandingMinted(uint256 indexed tokenId,address indexed acolyte,address indexed owner,uint256 controllerAgentId,address referrer,uint256 declaredPrice,uint256 paidThrough,uint256 firstUpkeep)",
]);
const FINALIZED = ACOLYTE_DEPLOYMENT_BLOCK + 10_000n;
const FINALIZED_HASH = `0x${"ab".repeat(32)}`;
const TX_HASH = `0x${"cd".repeat(32)}`;
const ACOLYTE = "0x0000000000000000000000000000000000000011";
const OWNER = "0x0000000000000000000000000000000000000022";
const REFERRER = "0x0000000000000000000000000000000000000033";

function block(number: bigint, hash: string) {
  return { number: `0x${number.toString(16)}`, hash };
}

function rpcFetch(logPages: unknown[][], callResults: Map<string, string> = new Map()): typeof fetch {
  let page = 0;
  return vi.fn(async (_input, init) => {
    const request = JSON.parse(String(init?.body)) as {
      id: number; method: string; params: unknown[];
    };
    let result: unknown;
    switch (request.method) {
      case "eth_chainId": result = "0x2105"; break;
      case "eth_getCode": result = "0x6000"; break;
      case "eth_getBlockByNumber": {
        const tag = request.params[0];
        result = tag === `0x${ACOLYTE_DEPLOYMENT_BLOCK.toString(16)}`
          ? block(ACOLYTE_DEPLOYMENT_BLOCK, ACOLYTE_DEPLOYMENT_BLOCK_HASH)
          : block(FINALIZED, FINALIZED_HASH);
        break;
      }
      case "eth_getLogs": result = logPages[page++] ?? []; break;
      case "eth_call": {
        const transaction = request.params[0] as { data: string };
        result = callResults.get(transaction.data.slice(0, 10));
        if (!result) throw new Error(`unexpected call ${transaction.data.slice(0, 10)}`);
        break;
      }
      default: throw new Error(`unexpected RPC method ${request.method}`);
    }
    return new Response(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }));
  }) as typeof fetch;
}

function mintLog(tokenId = BigInt(ACOLYTE)) {
  const event = abi.getEvent("BrandingMinted");
  if (!event) throw new Error("missing BrandingMinted ABI");
  const encoded = abi.encodeEventLog(event, [
    tokenId, ACOLYTE, OWNER, 7n, REFERRER, 1_000n, 2_000n, 1n,
  ]);
  return {
    address: ACOLYTE_BRANDING_CONTRACT,
    blockNumber: `0x${ACOLYTE_DEPLOYMENT_BLOCK.toString(16)}`,
    transactionHash: TX_HASH,
    topics: encoded.topics,
    data: encoded.data,
  };
}

describe("Acolyte Branding catalog", () => {
  it("returns an empty finalized snapshot and uses inclusive chunks of at most 10,000 blocks", async () => {
    const fetcher = rpcFetch([[], []]) as ReturnType<typeof vi.fn>;
    const snapshot = await fetchAcolyteCatalog({ fetch: fetcher as typeof fetch, endpoint: "https://rpc.test/" });
    expect(snapshot).toMatchObject({
      chainId: 8453,
      contractAddress: ACOLYTE_BRANDING_CONTRACT,
      sourceBlockNumber: FINALIZED.toString(),
      sourceBlockHash: FINALIZED_HASH,
      items: [],
    });
    const requests = fetcher.mock.calls.map((call) => JSON.parse(String(call[1]?.body)));
    const ranges = requests.filter((request) => request.method === "eth_getLogs")
      .map((request) => request.params[0]);
    expect(ranges).toEqual([
      { address: ACOLYTE_BRANDING_CONTRACT, topics: [expect.any(String)], fromBlock: "0x2f8b139", toBlock: "0x2f8d848" },
      { address: ACOLYTE_BRANDING_CONTRACT, topics: [expect.any(String)], fromBlock: "0x2f8d849", toBlock: "0x2f8d849" },
    ]);
  });

  it("enumerates one mint and reads current owner state and hostile metadata at the pinned block", async () => {
    const brandingOf = abi.getFunction("brandingOf");
    const avatarURIOf = abi.getFunction("avatarURIOf");
    const customTraitCount = abi.getFunction("customTraitCount");
    const customTraitAt = abi.getFunction("customTraitAt");
    if (!brandingOf || !avatarURIOf || !customTraitCount || !customTraitAt) {
      throw new Error("missing Branding read ABI");
    }
    const calls = new Map<string, string>([
      [brandingOf.selector, abi.encodeFunctionResult("brandingOf", [[
        BigInt(ACOLYTE), ACOLYTE, OWNER, 7n, REFERRER, 1_000n, 2_000n, 3_000n, 4_000n, 1n,
      ]])],
      [avatarURIOf.selector, abi.encodeFunctionResult("avatarURIOf", ["javascript:alert(1)"])],
      [customTraitCount.selector, abi.encodeFunctionResult("customTraitCount", [1n])],
      [customTraitAt.selector, abi.encodeFunctionResult("customTraitAt", ["mood", "<img onerror=boom>"])],
    ]);
    const snapshot = await fetchAcolyteCatalog({
      fetch: rpcFetch([[mintLog()], []], calls), endpoint: "https://rpc.test/",
    });
    expect(snapshot.items).toEqual([expect.objectContaining({
      tokenId: BigInt(ACOLYTE).toString(),
      acolyte: ACOLYTE,
      owner: OWNER,
      controllerAgentId: "7",
      referrer: REFERRER,
      declaredPrice: "1000",
      paidThrough: "2000",
      pendingDeclaredPrice: "3000",
      pendingPriceValidAfter: "4000",
      status: "Active",
      avatarUri: "javascript:alert(1)",
      traits: [{ traitType: "mood", value: "<img onerror=boom>" }],
      mintTransactionHash: TX_HASH,
    })]);
  });

  it("rejects duplicate mint events", async () => {
    await expect(fetchAcolyteCatalog({
      fetch: rpcFetch([[mintLog(), mintLog()], []]), endpoint: "https://rpc.test/",
    })).rejects.toThrow("duplicate Branding mint");
  });

  it("rejects unsafe endpoints and token IDs that do not match the acolyte", async () => {
    const never = vi.fn() as unknown as typeof fetch;
    await expect(fetchAcolyteCatalog({ fetch: never, endpoint: "http://rpc.test/" })).rejects.toThrow(
      /credential-free HTTPS/u,
    );
    expect(never).not.toHaveBeenCalled();

    await expect(fetchAcolyteCatalog({
      fetch: rpcFetch([[mintLog(18n)], []]), endpoint: "https://rpc.test/",
    })).rejects.toThrow("token ID does not match");
  });
});
