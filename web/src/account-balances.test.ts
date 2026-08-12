import { describe, expect, it, vi } from "vitest";
import { BASE_CHAIN_ID, UWU_CONTRACT } from "./leaderboard-types";
import { fetchAccountBalances } from "./account-balances";

const ACCOUNT = "0x1111111111111111111111111111111111111111";
const ENDPOINT = "https://mainnet.base.org/";

function json(value: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(value), {
    status: 200,
    ...init,
    headers: init.headers ?? { "content-type": "application/json" },
  });
}

function successfulFetch(options: {
  chainId?: unknown;
  blockNumber?: unknown;
  ethBalance?: unknown;
  uwuBalance?: unknown;
} = {}) {
  const bodies: unknown[][] = [];
  const mock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
    const body = JSON.parse(String(init?.body)) as unknown[];
    bodies.push(body);
    if (bodies.length === 1) {
      return json([
        {
          jsonrpc: "2.0",
          id: 2,
          result: "blockNumber" in options ? options.blockNumber : "0x7b",
        },
        { jsonrpc: "2.0", id: 1, result: "chainId" in options ? options.chainId : "0x2105" },
      ]);
    }
    return json([
      {
        jsonrpc: "2.0",
        id: 4,
        result: "uwuBalance" in options
          ? options.uwuBalance
          : `0x${(2_500n * 10n ** 18n).toString(16).padStart(64, "0")}`,
      },
      {
        jsonrpc: "2.0",
        id: 3,
        result: "ethBalance" in options ? options.ethBalance : "0x1121d33597384000",
      },
    ]);
  });
  return { fetch: mock as typeof fetch, bodies };
}

describe("account balances", () => {
  it("pins ETH and canonical UWU reads to one explicit Base block", async () => {
    const rpc = successfulFetch();
    await expect(fetchAccountBalances(ENDPOINT, ACCOUNT, { fetch: rpc.fetch })).resolves.toEqual({
      chainId: BASE_CHAIN_ID,
      blockNumber: 123n,
      blockTag: "0x7b",
      ethWei: "1234500000000000000",
      uwuRaw: "2500000000000000000000",
      formattedEth: "1.2345",
      formattedUwu: "2,500",
      level: "3.40",
    });

    expect(rpc.fetch).toHaveBeenCalledTimes(2);
    const reads = rpc.bodies[1] as Array<{ method: string; params: unknown[] }>;
    expect(reads.map(({ method }) => method)).toEqual(["eth_getBalance", "eth_call"]);
    expect(reads[0]?.params).toEqual([ACCOUNT, "0x7b"]);
    expect(reads[1]?.params).toEqual([
      {
        to: UWU_CONTRACT,
        data: `0x70a08231${ACCOUNT.slice(2).padStart(64, "0")}`,
      },
      "0x7b",
    ]);
  });

  it("rejects non-Base endpoints before reading balances", async () => {
    const rpc = successfulFetch({ chainId: "0x1" });
    await expect(fetchAccountBalances(ENDPOINT, ACCOUNT, { fetch: rpc.fetch })).rejects.toThrow(
      /expected 8453/u,
    );
    expect(rpc.fetch).toHaveBeenCalledTimes(1);
  });

  it("rejects missing, duplicated, mismatched, and errored JSON-RPC responses", async () => {
    const badBatches: unknown[] = [
      [{ jsonrpc: "2.0", id: 1, result: "0x2105" }],
      [
        { jsonrpc: "2.0", id: 1, result: "0x2105" },
        { jsonrpc: "2.0", id: 1, result: "0x7b" },
      ],
      [
        { jsonrpc: "2.0", id: 1, result: "0x2105" },
        { jsonrpc: "2.0", id: 999, result: "0x7b" },
      ],
      [
        { jsonrpc: "2.0", id: 1, result: "0x2105" },
        { jsonrpc: "2.0", id: 2, error: { code: -1, message: "offline" } },
      ],
    ];
    for (const batch of badBatches) {
      const fetcher = vi.fn(async () => json(batch)) as unknown as typeof fetch;
      await expect(fetchAccountBalances(ENDPOINT, ACCOUNT, { fetch: fetcher })).rejects.toThrow(
        /Base RPC/u,
      );
    }
  });

  it("never treats malformed or unavailable values as zero", async () => {
    const malformed = [
      successfulFetch({ blockNumber: "latest" }),
      successfulFetch({ ethBalance: null }),
      successfulFetch({ ethBalance: "0x00" }),
      successfulFetch({ uwuBalance: "0x0" }),
    ];
    for (const rpc of malformed) {
      await expect(fetchAccountBalances(ENDPOINT, ACCOUNT, { fetch: rpc.fetch })).rejects.toThrow();
    }

    const httpFailure = vi.fn(async () => new Response("unavailable", { status: 503 })) as unknown as typeof fetch;
    await expect(fetchAccountBalances(ENDPOINT, ACCOUNT, { fetch: httpFailure })).rejects.toThrow(
      /HTTP 503/u,
    );
  });

  it("bounds endpoint, address, timeout, and response size", async () => {
    const never = vi.fn() as unknown as typeof fetch;
    await expect(fetchAccountBalances("http://mainnet.base.org/", ACCOUNT, { fetch: never })).rejects.toThrow(
      /credential-free HTTPS/u,
    );
    await expect(fetchAccountBalances("https://user:secret@example.com/", ACCOUNT, { fetch: never })).rejects.toThrow(
      /credential-free HTTPS/u,
    );
    await expect(fetchAccountBalances(ENDPOINT, "0x1234", { fetch: never })).rejects.toThrow(
      /Ethereum address/u,
    );
    await expect(fetchAccountBalances(ENDPOINT, ACCOUNT, { fetch: never, timeoutMs: 30_001 })).rejects.toThrow(
      /timeout/u,
    );
    expect(never).not.toHaveBeenCalled();

    const oversized = vi.fn(async () => new Response("x".repeat(64 * 1024 + 1), { status: 200 })) as unknown as typeof fetch;
    await expect(fetchAccountBalances(ENDPOINT, ACCOUNT, { fetch: oversized })).rejects.toThrow(
      /too large/u,
    );
  });

  it("aborts stalled RPC reads at the configured deadline", async () => {
    const stalled = vi.fn((_input: RequestInfo | URL, init?: RequestInit) => new Promise<Response>((_resolve, reject) => {
      init?.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")), {
        once: true,
      });
    })) as unknown as typeof fetch;
    await expect(fetchAccountBalances(ENDPOINT, ACCOUNT, { fetch: stalled, timeoutMs: 5 })).rejects.toMatchObject({
      name: "AbortError",
    });
  });
});
