import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { fetchCompleteLeaderboard, IndexingError } from "./leaderboard-data";
import {
  ALLEGIANCE_HEX,
  IDENTITY_REGISTRY,
  PROTOCOL_V1_HEX,
  UWU_CONTRACT,
  ZERO_ADDRESS,
} from "./leaderboard-types";

const WALLET_A = "0x1111111111111111111111111111111111111111";
const WALLET_B = "0x2222222222222222222222222222222222222222";
const OWNER = "0x3333333333333333333333333333333333333333";

function row(
  agentId: string,
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    id: agentId,
    agentId,
    owner: OWNER,
    agentURI: "",
    agentWallet: WALLET_A,
    allegiance: ALLEGIANCE_HEX,
    protocol: PROTOCOL_V1_HEX,
    tentacleId: `tentacle_${agentId}`,
    isTentacle: true,
    isWalletVerified: true,
    registrationBlock: String(100 + Number(agentId)),
    registrationTimestamp: "1700000000",
    profileUpdatedBlock: "200",
    profileUpdatedTimestamp: "1700000100",
    metadataUpdatedBlock: "201",
    metadataUpdatedTimestamp: "1700000200",
    feedbackCount: "1",
    activeFeedbackCount: "1",
    revokedFeedbackCount: "0",
    wallet: {
      address: WALLET_A,
      rawBalance: "100000000000000000000",
      updatedBlock: "300",
      updatedTimestamp: "1700000300",
    },
    profile: {
      id: `profile-${agentId}`,
      schemaType: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
      name: `Tentacle ${agentId}`,
      description: "public profile",
      image: "javascript:alert(1)",
      active: true,
      xmtpEndpoint: `xmtp://${"a".repeat(64)}`,
      cthuwuEndpoint: "https://cthuwu.app/",
      sourceURI: `ipfs://bafy${agentId}`,
      contentHash: null,
      parseValid: true,
    },
    feedbacks: [
      {
        id: `${agentId}:${WALLET_B}:1`,
        clientAddress: WALLET_B,
        feedbackIndex: "1",
        value: "875",
        valueDecimals: 1,
        tag1: "quality",
        tag2: "chat",
        endpoint: "https://example.test/feedback",
        feedbackURI: "",
        feedbackHash: `0x${"00".repeat(32)}`,
        isRevoked: false,
        createdBlock: "250",
        createdTimestamp: "1700000250",
        createdTransaction: `0x${"01".repeat(32)}`,
        provenance: `eip155:8453:${IDENTITY_REGISTRY}:0x${"01".repeat(32)}:0`,
      },
    ],
    ...overrides,
  };
}

function graphFetch(rows: unknown[], indexingError = false): typeof fetch {
  return vi.fn(async () =>
    new Response(
      JSON.stringify({
        data: {
          tentacles: rows,
          _meta: {
            block: { number: "42000000", hash: `0x${"ab".repeat(32)}`, timestamp: "1786000000" },
            deployment: "QmCthuwuDeployment",
            hasIndexingErrors: indexingError,
          },
        },
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    ),
  ) as typeof fetch;
}

describe("Graph leaderboard normalization", () => {
  it("accepts the generated subgraph fixture without a compatibility adapter", async () => {
    const fixture = readFileSync(
      resolve(process.cwd(), "../subgraph/fixtures/leaderboard-v1.json"),
      "utf8",
    );
    const snapshot = await fetchCompleteLeaderboard("https://example.test/graphql", {
      fetch: vi.fn(async () => new Response(fixture, { status: 200 })) as typeof fetch,
    });
    expect(snapshot.sourceDeployment).toBe("QmFixtureCthuwuLeaderboardV1");
    expect(snapshot.rankedWallets[0]).toMatchObject({
      representativeAgentId: "7",
      rawBalance: "1000000000000000000000",
      rank: 1,
    });
  });

  it("requires exact opt-in, retains zero balances, suspends zero wallets, and groups shared wallets", async () => {
    const snapshot = await fetchCompleteLeaderboard(
      "https://example.test/graphql",
      {
        fetch: graphFetch([
          row("3"),
          row("5", {
            agentWallet: ZERO_ADDRESS,
            isWalletVerified: false,
            wallet: null,
          }),
          row("6", {
            allegiance: "0x5557552d74656e7461636c652d7631",
            isTentacle: false,
            wallet: { address: WALLET_B, rawBalance: "999999999999999999999999" },
          }),
          row("7"),
          row("8", {
            agentWallet: WALLET_B,
            wallet: {
              address: WALLET_B,
              rawBalance: "0",
              updatedBlock: "300",
              updatedTimestamp: "1700000300",
            },
          }),
        ]),
        now: () => new Date("2026-08-11T12:00:00Z"),
      },
    );

    expect(snapshot.chainId).toBe(8453);
    expect(snapshot.identityRegistry).toBe(IDENTITY_REGISTRY);
    expect(snapshot.uwuContract).toBe(UWU_CONTRACT);
    expect(snapshot.rankedWallets).toHaveLength(2);
    expect(snapshot.rankedWallets[0]).toMatchObject({
      wallet: WALLET_A,
      rawBalance: "100000000000000000000",
      representativeAgentId: "3",
      rank: 1,
    });
    expect(snapshot.rankedWallets[0].identities.map(({ agentId }) => agentId)).toEqual(["3", "7"]);
    expect(snapshot.rankedWallets[1]).toMatchObject({ wallet: WALLET_B, rawBalance: "0" });
    expect(snapshot.rankedWallets[1].rank).toBeUndefined();
    expect(snapshot.suspended.map(({ agentId }) => agentId)).toEqual(["5"]);
    expect(JSON.stringify(snapshot)).not.toContain('"agentId":"6"');
    expect(snapshot.rankedWallets[0].identities[0].profile.image).toBeUndefined();
    expect(snapshot.rankedWallets[0].identities[0].reputation[0]).toMatchObject({
      value: "875",
      valueDecimals: 1,
      tag1: "quality",
      revoked: false,
    });
    expect(snapshot.rankedWallets[0].identities[0].reputationCounters).toEqual({
      total: "1",
      active: "1",
      revoked: "0",
    });
  });

  it("preserves registry counters without presenting the bounded event sample as a total", async () => {
    const template = row("1").feedbacks as Array<Record<string, unknown>>;
    const feedbacks = Array.from({ length: 10 }, (_, index) => ({
      ...template[0],
      id: `1:${WALLET_B}:${index + 1}`,
      feedbackIndex: String(index + 1),
      createdTimestamp: String(1_700_000_250 + index),
    }));
    const snapshot = await fetchCompleteLeaderboard("https://example.test/graphql", {
      fetch: graphFetch([
        row("1", {
          feedbackCount: "25",
          activeFeedbackCount: "25",
          revokedFeedbackCount: "0",
          feedbacks,
        }),
      ]),
    });

    expect(snapshot.rankedWallets[0].identities[0].reputation).toHaveLength(10);
    expect(snapshot.rankedWallets[0].identities[0].reputationCounters).toEqual({
      total: "25",
      active: "25",
      revoked: "0",
    });
  });

  it("rejects indexing errors and GraphQL errors instead of producing a partial snapshot", async () => {
    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", {
        fetch: graphFetch([row("1")], true),
      }),
    ).rejects.toBeInstanceOf(IndexingError);

    const failed = vi.fn(async () =>
      new Response(JSON.stringify({ data: { tentacles: [row("1")] }, errors: [{ message: "boom" }] })),
    ) as typeof fetch;
    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", { fetch: failed }),
    ).rejects.toThrow("GraphQL errors");
  });

  it("rejects inconsistent shared-wallet balances", async () => {
    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", {
        fetch: graphFetch([
          row("1"),
          row("2", {
            wallet: {
              address: WALLET_A,
              rawBalance: "1",
              updatedBlock: "300",
              updatedTimestamp: "1700000300",
            },
          }),
        ]),
      }),
    ).rejects.toThrow("inconsistent UWU balances");
  });

  it("breaks equal-balance wallet ties by the group's earliest registration then lowest agent ID", async () => {
    const snapshot = await fetchCompleteLeaderboard("https://example.test/graphql", {
      fetch: graphFetch([
        row("1", { registrationBlock: "500" }),
        row("2", {
          registrationBlock: "300",
          agentWallet: WALLET_B,
          wallet: {
            address: WALLET_B,
            rawBalance: "100000000000000000000",
            updatedBlock: "300",
            updatedTimestamp: "1700000300",
          },
        }),
        row("9", { registrationBlock: "200" }),
      ]),
    });
    expect(snapshot.rankedWallets.map(({ representativeAgentId }) => representativeAgentId)).toEqual([
      "1",
      "2",
    ]);
  });

  it("fetches every page against one pinned source block before producing a snapshot", async () => {
    const firstPage = Array.from({ length: 250 }, (_, index) => row(String(index + 1)));
    const secondPage = [row("251")];
    const calls: Array<{ first: number; after: string; block: { number: number } | null }> = [];
    const paged = vi.fn(async (_endpoint: RequestInfo | URL, init?: RequestInit) => {
      const request = JSON.parse(String(init?.body)) as {
        variables: { first: number; after: string; block: { number: number } | null };
      };
      calls.push(request.variables);
      return new Response(
        JSON.stringify({
          data: {
            tentacles: calls.length === 1 ? firstPage : secondPage,
            _meta: {
              block: {
                number: "42000000",
                hash: `0x${"ab".repeat(32)}`,
                timestamp: "1786000000",
              },
              deployment: "QmCthuwuDeployment",
              hasIndexingErrors: false,
            },
          },
        }),
      );
    }) as typeof fetch;

    const snapshot = await fetchCompleteLeaderboard("https://example.test/graphql", {
      fetch: paged,
    });
    expect(calls).toEqual([
      { first: 250, after: "-1", block: null },
      { first: 250, after: "250", block: { number: 42_000_000 } },
    ]);
    expect(snapshot.paginationComplete).toBe(true);
    expect(snapshot.rankedWallets[0].identities).toHaveLength(251);
  });

  it("fails closed when a later page omits pinned block identity metadata", async () => {
    const firstPage = Array.from({ length: 250 }, (_, index) => row(String(index + 1)));
    let call = 0;
    const paged = vi.fn(async () => {
      call += 1;
      return new Response(
        JSON.stringify({
          data: {
            tentacles: call === 1 ? firstPage : [row("251")],
            _meta: {
              block: {
                number: "42000000",
                ...(call === 1 ? { hash: `0x${"ab".repeat(32)}` } : {}),
                timestamp: "1786000000",
              },
              deployment: "QmCthuwuDeployment",
              hasIndexingErrors: false,
            },
          },
        }),
      );
    }) as typeof fetch;

    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", { fetch: paged }),
    ).rejects.toThrow("pagination changed source block");
  });

  it("fails closed when a verified wallet relation or complete Graph shape is missing", async () => {
    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", {
        fetch: graphFetch([row("1", { wallet: null })]),
      }),
    ).rejects.toThrow("missing its UWU wallet state");
    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", {
        fetch: vi.fn(async () => new Response(JSON.stringify({ data: { tentacles: [] } }))) as typeof fetch,
      }),
    ).rejects.toThrow("Graph _meta");
    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", {
        fetch: graphFetch([row("1", { id: "2" })]),
      }),
    ).rejects.toThrow("does not match agentId");
    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", {
        fetch: graphFetch([row("2"), row("1")]),
      }),
    ).rejects.toThrow("strictly ordered");
    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", {
        fetch: graphFetch([row("1", { isTentacle: false })]),
      }),
    ).rejects.toThrow("disagrees with exact current allegiance");
    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", {
        fetch: graphFetch([
          row("1", {
            feedbackCount: "2",
            activeFeedbackCount: "2",
            revokedFeedbackCount: "1",
          }),
        ]),
      }),
    ).rejects.toThrow("reputation counters are inconsistent");
    const malformedMeta = vi.fn(async () =>
      new Response(
        JSON.stringify({
          data: {
            tentacles: [row("1")],
            _meta: {
              block: { number: "42000000", hash: "not-bytes", timestamp: "1786000000" },
              deployment: "QmCthuwuDeployment",
              hasIndexingErrors: false,
            },
          },
        }),
      ),
    ) as typeof fetch;
    await expect(
      fetchCompleteLeaderboard("https://example.test/graphql", { fetch: malformedMeta }),
    ).rejects.toThrow("optional bytes");
  });
});
