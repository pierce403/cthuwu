import { describe, expect, it } from "vitest";
import { fetchCompleteLeaderboard, IndexingError } from "./leaderboard-data";
import { ALLEGIANCE_HEX, UWU_CONTRACT } from "./leaderboard-types";

const BLOCK = "49768180";
const BLOCK_HASH = `0x${"cc".repeat(32)}`;
const WALLET = `0x${"22".repeat(20)}`;
const OWNER = `0x${"11".repeat(20)}`;
const TENTACLE_ID = `0x${Buffer.from("tentacle_fixture").toString("hex")}`;

function graph(overrides: Record<string, unknown> = {}): unknown {
  return {
    data: {
      agentMetadatas: [{
        id: "8453:7:cthuwu.allegiance", key: "cthuwu.allegiance", value: ALLEGIANCE_HEX,
        updatedAt: "1770118004",
        agent: {
          id: "8453:7", chainId: "8453", agentId: "7", owner: OWNER, agentWallet: WALLET,
          agentURI: "", createdAt: "1770118000", updatedAt: "1770118002", totalFeedback: "1",
          metadata: [
            { id: "a", key: "cthuwu.allegiance", value: ALLEGIANCE_HEX, updatedAt: "1770118004" },
            { id: "p", key: "cthuwu.protocol", value: "0x31", updatedAt: "1770118004" },
            { id: "t", key: "cthuwu.tentacle-id", value: TENTACLE_ID, updatedAt: "1770118004" },
          ],
          registrationFile: { id: "f", cid: "cid", name: "Fixture Tentacle", description: "fixture", image: null, active: true, endpointsRawJson: "[]", createdAt: "1770118002" },
          feedback: [{ id: "f1", clientAddress: `0x${"33".repeat(20)}`, feedbackIndex: "1", value: "5.25", tag1: "reliability", tag2: "xmtp", endpoint: "", feedbackURI: "", feedbackHash: `0x${"aa".repeat(32)}`, isRevoked: false, createdAt: "1770120000", revokedAt: null }],
        },
      }],
      _meta: { block: { number: BLOCK, hash: BLOCK_HASH, timestamp: "1786332360" }, deployment: "QmAgent0Fixture", hasIndexingErrors: false },
      ...overrides,
    },
  };
}

function response(value: unknown): Response {
  return new Response(JSON.stringify(value), { status: 200, headers: { "content-type": "application/json" } });
}

function fixtureFetch(graphValue: unknown = graph(), balance = 1_000n * 10n ** 18n): typeof fetch {
  return (async (input, init) => {
    if (String(input).includes("graph")) return response(graphValue);
    const body = JSON.parse(String(init?.body)) as { method?: string; id?: number } | Array<{ method: string; id: number; params: unknown[] }>;
    if (Array.isArray(body)) {
      return response(body.map((request) => ({ jsonrpc: "2.0", id: request.id, result: `0x${balance.toString(16).padStart(64, "0")}` })));
    }
    if (body.method === "eth_call") return response({ jsonrpc: "2.0", id: body.id, result: `0x${balance.toString(16).padStart(64, "0")}` });
    expect(body.method).toBe("eth_getBlockByNumber");
    return response({ jsonrpc: "2.0", id: body.id, result: { number: `0x${BigInt(BLOCK).toString(16)}`, hash: BLOCK_HASH, timestamp: `0x${BigInt(1786332360).toString(16)}` } });
  }) as typeof fetch;
}

describe("Agent0 leaderboard + direct Base UWU reads", () => {
  it("filters exact current allegiance and reads balanceOf at Agent0's pinned block", async () => {
    const calls: string[] = [];
    const base = fixtureFetch();
    const fetcher = (async (input, init) => {
      calls.push(String(init?.body));
      return base(input, init);
    }) as typeof fetch;
    const snapshot = await fetchCompleteLeaderboard("https://graph.fixture.invalid", { fetch: fetcher, baseRpcEndpoint: "https://rpc.fixture.invalid", now: () => new Date("2026-08-11T00:00:00Z") });
    expect(snapshot.rankedWallets[0].rawBalance).toBe("1000000000000000000000");
    expect(snapshot.rankedWallets[0].identities[0].tentacleId).toBe("tentacle_fixture");
    expect(snapshot.rankedWallets[0].identities[0].reputation[0]).toMatchObject({ value: "525", valueDecimals: 2 });
    expect(calls[0]).toContain("agentMetadatas");
    expect(calls[0]).toContain(ALLEGIANCE_HEX);
    expect(calls.at(-1)).toContain(UWU_CONTRACT);
    expect(calls.at(-1)).toContain(`0x${BigInt(BLOCK).toString(16)}`);
  });

  it("keeps exact opt-ins with a missing agentWallet suspended and performs no balance call", async () => {
    const value = graph();
    const row = (value as any).data.agentMetadatas[0].agent;
    row.agentWallet = null;
    const snapshot = await fetchCompleteLeaderboard("https://graph.fixture.invalid", { fetch: fixtureFetch(value) });
    expect(snapshot.rankedWallets).toHaveLength(0);
    expect(snapshot.suspended).toHaveLength(1);
  });

  it("fails closed when Base RPC disagrees with Agent0's block", async () => {
    const fetcher = (async (input, init) => {
      if (String(input).includes("graph")) return response(graph());
      const body = JSON.parse(String(init?.body));
      return response({ jsonrpc: "2.0", id: body.id, result: { number: "0x1", hash: BLOCK_HASH, timestamp: "0x1" } });
    }) as typeof fetch;
    await expect(fetchCompleteLeaderboard("https://graph.fixture.invalid", { fetch: fetcher })).rejects.toThrow("does not match");
  });

  it("surfaces Agent0 indexing errors", async () => {
    const value = graph({ agentMetadatas: [] });
    (value as any).data._meta.hasIndexingErrors = true;
    await expect(fetchCompleteLeaderboard("https://graph.fixture.invalid", { fetch: fixtureFetch(value) })).rejects.toBeInstanceOf(IndexingError);
  });
});
