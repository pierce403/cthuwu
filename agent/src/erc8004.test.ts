import { describe, expect, it } from "vitest";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  encodeAbiParameters,
  encodeEventTopics,
  parseAbi,
  type Hex,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import type { LoadedIdentity } from "./identity.js";
import {
  ALLEGIANCE_KEY,
  ALLEGIANCE_VALUE,
  ERC8004_CHAIN_ID,
  ERC8004_IDENTITY_REGISTRY,
  ERC8004_REPUTATION_REGISTRY,
  ERC8004_START_BLOCK,
  PROTOCOL_KEY,
  assertProductionIdentity,
  assertTransactionNonceWindow,
  authorizeSignerNonce,
  discoverAgents,
  inspectAgent,
  isTransientRpcError,
  parseErc8004Request,
  prepareErc8004Transaction,
  readSignerNonceState,
  requestFingerprint,
  sumThrottledL1Fees,
  withBoundedRpcRetry,
} from "./erc8004.js";

const WALLET = "0x1111111111111111111111111111111111111111";
const SIGNER_KEY = `0x${"22".repeat(32)}` as const;

const discoveryAbi = parseAbi([
  "event Registered(uint256 indexed agentId, string agentURI, address indexed owner)",
  "event Transfer(address indexed from, address indexed to, uint256 indexed tokenId)",
  "event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId)",
  "event ApprovalForAll(address indexed owner, address indexed operator, bool approved)",
  "event MetadataSet(uint256 indexed agentId, string indexed indexedMetadataKey, string metadataKey, bytes metadataValue)",
]);

function discoveryLog(
  eventName: "Registered" | "Transfer" | "Approval" | "ApprovalForAll" | "MetadataSet",
  args: Record<string, unknown>,
  transactionHash?: Hex,
): Record<string, unknown> {
  const topics = encodeEventTopics({
    abi: discoveryAbi,
    eventName,
    args: args as never,
  });
  let data: Hex = "0x";
  switch (eventName) {
    case "Registered":
      data = encodeAbiParameters([{ type: "string" }], [String(args.agentURI ?? "")]);
      break;
    case "ApprovalForAll":
      data = encodeAbiParameters([{ type: "bool" }], [args.approved === true]);
      break;
    case "MetadataSet":
      data = encodeAbiParameters(
        [{ type: "string" }, { type: "bytes" }],
        [String(args.metadataKey), args.metadataValue as Hex],
      );
      break;
    case "Transfer":
    case "Approval":
      break;
  }
  return { data, topics, ...(transactionHash === undefined ? {} : { transactionHash }) };
}

function signerIdentity(environment: LoadedIdentity["environment"]): LoadedIdentity {
  return {
    version: 1,
    environment,
    walletKey: SIGNER_KEY,
    dbEncryptionKey: `0x${"33".repeat(32)}`,
    createdAt: "2026-01-01T00:00:00.000Z",
    identityPath: "/private/identity.json",
    dbDirectory: "/private/xmtp",
    walletAddress: privateKeyToAccount(SIGNER_KEY).address,
  };
}

async function journalIdentity(): Promise<{ identity: LoadedIdentity; directory: string }> {
  const directory = await mkdtemp(path.join(tmpdir(), "cthuwu-erc8004-signer-"));
  return {
    identity: {
      ...signerIdentity("production"),
      identityPath: path.join(directory, "xmtp-identity.json"),
    },
    directory,
  };
}

function request(operation: Record<string, unknown>): unknown {
  return { version: 1, actionId: "registration:test-1", operation };
}

describe("narrow ERC-8004 signer protocol", () => {
  it("pins canonical Base mainnet and both canonical registries", () => {
    expect(ERC8004_CHAIN_ID).toBe(8453);
    expect(ERC8004_IDENTITY_REGISTRY).toBe("0x8004A169FB4a3325136EB29fA0ceB6D2e539a432");
    expect(ERC8004_REPUTATION_REGISTRY).toBe("0x8004BAa17C55a88189AE136b182e5fdA19dE9b63");
  });

  it("prepares only a zero-value transaction to the canonical registry", () => {
    const parsed = parseErc8004Request(request({ type: "register", nonce: "7" }));
    if (parsed.operation.type !== "register") throw new Error("unexpected operation");
    const prepared = prepareErc8004Transaction(parsed.operation, WALLET, 1_700_000_000n);
    expect(prepared).toMatchObject({ chainId: 8453, to: ERC8004_IDENTITY_REGISTRY, value: 0n });
    expect(prepared.data).toMatch(/^0x[0-9a-f]+$/u);
  });

  it("rejects arbitrary chain, destination, value, calldata, and operation fields", () => {
    for (const extra of [
      { chainId: 1 },
      { to: WALLET },
      { value: "1" },
      { data: "0xdeadbeef" },
    ]) {
      expect(() => parseErc8004Request(request({ type: "register", nonce: "7", ...extra }))).toThrow("missing or unknown fields");
    }
    expect(() => parseErc8004Request(request({ type: "send_transaction" }))).toThrow("unsupported ERC-8004 operation");
  });

  it("allows only exact allegiance bytes or a deliberate clear", () => {
    const exact = parseErc8004Request(request({ type: "set_metadata", agentId: "7", key: ALLEGIANCE_KEY, value: ALLEGIANCE_VALUE, nonce: "8" }));
    expect(exact.operation).toMatchObject({ value: ALLEGIANCE_VALUE });
    expect(() => parseErc8004Request(request({ type: "set_metadata", agentId: "7", key: ALLEGIANCE_KEY, value: "UWU-TENTACLE-V1", nonce: "8" }))).toThrow("exact opt-in bytes");
    expect(() => parseErc8004Request(request({ type: "set_metadata", agentId: "7", key: ALLEGIANCE_KEY, value: " uwu-tentacle-v1", nonce: "8" }))).toThrow("exact opt-in bytes");
    expect(parseErc8004Request(request({ type: "set_metadata", agentId: "7", key: ALLEGIANCE_KEY, value: "", nonce: "8" })).operation).toMatchObject({ value: "" });
  });

  it("allows only exact protocol bytes and bounded cthuwu metadata keys", () => {
    expect(() => parseErc8004Request(request({ type: "set_metadata", agentId: "7", key: PROTOCOL_KEY, value: "01", nonce: "8" }))).toThrow("exact version 1 bytes");
    expect(() => parseErc8004Request(request({ type: "set_metadata", agentId: "7", key: "agentWallet", value: WALLET, nonce: "8" }))).toThrow("allowlist");
    expect(() => parseErc8004Request(request({ type: "set_metadata", agentId: "7", key: "cthuwu.tentacle-id", value: "x".repeat(129), nonce: "8" }))).toThrow("exceeds its public bound");
  });

  it("bounds URI, identifiers, hashes, and unknown fields", () => {
    expect(() => parseErc8004Request(request({ type: "set_agent_uri", agentId: "01", agentURI: "data:application/json,{}", nonce: "8" }))).toThrow("canonical uint256");
    expect(() => parseErc8004Request(request({ type: "set_agent_uri", agentId: "1", agentURI: `https://example.invalid/${"x".repeat(8192)}`, nonce: "8" }))).toThrow("oversized");
    expect(() => parseErc8004Request({ ...request({ type: "register", nonce: "7" }) as object, privateKey: `0x${"11".repeat(32)}` })).toThrow("missing or unknown fields");
  });

  it("produces deterministic secret-free request fingerprints", () => {
    const parsed = parseErc8004Request(request({ type: "inspect_agent", agentId: "9", wallet: WALLET }));
    expect(requestFingerprint(parsed)).toMatch(/^[0-9a-f]{64}$/u);
    expect(requestFingerprint(parsed)).toBe(requestFingerprint(parsed));
    expect(JSON.stringify(parsed)).not.toContain("private");
  });

  it("resolves only the production inbox for a bounded wallet request", () => {
    const parsed = parseErc8004Request(
      request({ type: "resolve_inbox", wallet: WALLET }),
    );
    expect(parsed.operation).toEqual({ type: "resolve_inbox", wallet: WALLET });
    expect(() =>
      parseErc8004Request(
        request({ type: "resolve_inbox", wallet: WALLET, environment: "dev" }),
      ),
    ).toThrow("missing or unknown fields");
  });

  it("requires an exact persisted sender nonce for every registry write", () => {
    expect(
      parseErc8004Request(request({ type: "register", nonce: "7" })).operation,
    ).toEqual({ type: "register", nonce: "7" });
    expect(() => parseErc8004Request(request({ type: "register" }))).toThrow(
      "missing or unknown fields",
    );
    expect(() =>
      parseErc8004Request(request({ type: "register", nonce: "07" })),
    ).toThrow("canonical");
    expect(() => parseErc8004Request(request({ type: "set_agent_wallet", agentId: "7" }))).toThrow(
      "missing or unknown fields",
    );
    expect(
      parseErc8004Request(request({ type: "set_agent_wallet", agentId: "7", nonce: "8" })).operation,
    ).toMatchObject({ nonce: "8" });
    expect(() => assertTransactionNonceWindow("7", 8, 7)).toThrow(
      "without an exact durable signer action allocation",
    );
    expect(assertTransactionNonceWindow("7", 8, 7, true)).toBe(7);
    expect(() => assertTransactionNonceWindow("6", 8, 7)).toThrow(
      "already been confirmed",
    );
    expect(() => assertTransactionNonceWindow("9", 8, 7)).toThrow(
      "above the production signer's pending nonce",
    );
    expect(
      parseErc8004Request(
        request({
          type: "transaction_nonce",
          wallet: WALLET,
          observedBlockNumber: "123",
          observedBlockHash: `0x${"aa".repeat(32)}`,
        }),
      ).operation,
    ).toMatchObject({ observedBlockNumber: "123" });
    expect(() =>
      parseErc8004Request(
        request({
          type: "transaction_nonce",
          wallet: WALLET,
          observedBlockNumber: "123",
        }),
      ),
    ).toThrow("must be provided together");
  });

  it("binds inbox publication and writes to the loaded production signer", () => {
    const identity = signerIdentity("production");
    expect(assertProductionIdentity(identity, identity.walletAddress)).toBe(
      identity.walletAddress,
    );
    expect(() => assertProductionIdentity(signerIdentity("dev"))).toThrow(
      "production identity",
    );
    let mismatch = "";
    try {
      assertProductionIdentity(identity, WALLET);
    } catch (error) {
      mismatch = error instanceof Error ? error.message : String(error);
    }
    expect(mismatch).toContain("not the persistent XMTP production signer");
    expect(mismatch).not.toContain(SIGNER_KEY);
  });

  it("durably allocates before broadcast and permits only the exact action replay", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      const parsed = parseErc8004Request(request({ type: "register", nonce: "7" }));
      if (parsed.operation.type !== "register") throw new Error("unexpected operation");
      expect(
        await authorizeSignerNonce(
          identity,
          parsed.actionId,
          parsed.operation,
          7,
          7,
        ),
      ).toBe(false);
      const journal = await readFile(
        path.join(directory, "erc8004-signer-nonce-v1-7.json"),
        "utf8",
      );
      expect(journal).toContain(parsed.actionId);
      expect(journal).not.toContain(SIGNER_KEY);
      expect(
        await authorizeSignerNonce(
          identity,
          parsed.actionId,
          parsed.operation,
          7,
          8,
        ),
      ).toBe(true);

      const changedAction = parseErc8004Request({
        ...request({ type: "register", nonce: "7" }) as object,
        actionId: "registration:changed-action",
      });
      if (changedAction.operation.type !== "register") throw new Error("unexpected operation");
      await expect(
        authorizeSignerNonce(
          identity,
          changedAction.actionId,
          changedAction.operation,
          7,
          8,
        ),
      ).rejects.toThrow("allocated to another ERC-8004 action");

      const changedOperation = parseErc8004Request(
        request({
          type: "set_agent_uri",
          agentId: "7",
          agentURI: "data:application/json;base64,e30=",
          nonce: "7",
        }),
      );
      if (changedOperation.operation.type !== "set_agent_uri") {
        throw new Error("unexpected operation");
      }
      await expect(
        authorizeSignerNonce(
          identity,
          changedOperation.actionId,
          changedOperation.operation,
          7,
          8,
        ),
      ).rejects.toThrow("allocated to another ERC-8004 action");
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("never replaces an unrelated unjournaled pending wallet nonce", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      const parsed = parseErc8004Request(request({ type: "register", nonce: "7" }));
      if (parsed.operation.type !== "register") throw new Error("unexpected operation");
      await expect(
        authorizeSignerNonce(
          identity,
          parsed.actionId,
          parsed.operation,
          7,
          8,
        ),
      ).rejects.toThrow("without an exact durable signer action allocation");
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("retries only transient RPC failures with deterministic exponential bounds", async () => {
    const delays: number[] = [];
    let calls = 0;
    const result = await withBoundedRpcRetry(
      async () => {
        calls += 1;
        if (calls < 3) throw new Error("RPC Request failed: over rate limit");
        return "ok";
      },
      {
        attempts: 3,
        baseDelayMs: 25,
        sleep: async (milliseconds) => {
          delays.push(milliseconds);
        },
      },
    );
    expect(result).toBe("ok");
    expect(calls).toBe(3);
    expect(delays).toEqual([25, 50]);
    expect(isTransientRpcError(new Error("execution reverted"))).toBe(false);

    let permanentCalls = 0;
    await expect(
      withBoundedRpcRetry(
        async () => {
          permanentCalls += 1;
          throw new Error("execution reverted");
        },
        { attempts: 3, sleep: async () => undefined },
      ),
    ).rejects.toThrow("execution reverted");
    expect(permanentCalls).toBe(1);
  });

  it("throttles every exact L1 fee query and retries a rate-limited oracle call", async () => {
    const delays: number[] = [];
    let firstAttempts = 0;
    const total = await sumThrottledL1Fees(
      [1, 2],
      async (requestValue) => {
        if (requestValue === 1 && firstAttempts++ === 0) {
          throw new Error("RPC Request failed: over rate limit");
        }
        return BigInt(requestValue * 10);
      },
      {
        throttleMs: 25,
        sleep: async (milliseconds) => {
          delays.push(milliseconds);
        },
      },
    );
    expect(total).toBe(30n);
    expect(delays).toEqual([25, 1_100, 25]);
  });

  it("discovers direct approvals, current agentWallet, and current operators but filters stale authority", async () => {
    const activeOwner = "0x3333333333333333333333333333333333333333";
    const revokedOwner = "0x4444444444444444444444444444444444444444";
    let logScans = 0;
    const fake = {
      getBlock: async () => ({
        number: ERC8004_START_BLOCK,
        hash: `0x${"aa".repeat(32)}`,
      }),
      request: async () => {
        logScans += 1;
        if (logScans === 1) {
          return [
            discoveryLog("Approval", { owner: activeOwner, approved: WALLET, tokenId: 1n }),
            discoveryLog("Approval", { owner: revokedOwner, approved: WALLET, tokenId: 2n }),
            discoveryLog("MetadataSet", {
              agentId: 3n,
              indexedMetadataKey: "agentWallet",
              metadataKey: "agentWallet",
              metadataValue: WALLET,
            }),
            discoveryLog("ApprovalForAll", { owner: activeOwner, operator: WALLET, approved: true }),
            discoveryLog("ApprovalForAll", { owner: revokedOwner, operator: WALLET, approved: false }),
          ];
        }
        return [discoveryLog("Registered", { agentId: 4n, agentURI: "", owner: activeOwner })];
      },
      readContract: async ({ functionName, args }: { functionName: string; args: readonly unknown[] }) => {
        const idArgument = functionName === "isAuthorizedOrOwner" ? args[1] : args[0];
        const id = typeof idArgument === "bigint" ? Number(idArgument) : 0;
        switch (functionName) {
          case "isApprovedForAll":
            return args[0] === activeOwner;
          case "ownerOf":
            return activeOwner;
          case "tokenURI":
            return "";
          case "getAgentWallet":
            return id === 3 ? WALLET : "0x0000000000000000000000000000000000000000";
          case "getMetadata":
            return "0x";
          case "isAuthorizedOrOwner":
            return id === 1 || id === 4;
          default:
            throw new Error(`unexpected read ${functionName}`);
        }
      },
    };
    const result = await discoverAgents(fake as never, WALLET) as {
      complete: boolean;
      observedBlockNumber: string;
      observedBlockHash: string;
      candidates: Array<{ agentId: string }>;
    };
    expect(result.complete).toBe(true);
    expect(result.observedBlockNumber).toBe(ERC8004_START_BLOCK.toString());
    expect(result.observedBlockHash).toBe(`0x${"aa".repeat(32)}`);
    expect(result.candidates.map(({ agentId }) => agentId)).toEqual(["1", "3", "4"]);
    expect(logScans).toBe(2);
  });

  it("uses one combined event scan with Base's inclusive 10,000-block range cap", async () => {
    const requests: Array<Record<string, unknown>> = [];
    const observedBlockHash = `0x${"aa".repeat(32)}` as const;
    const result = await discoverAgents(
      {
        getBlock: async () => ({
          number: ERC8004_START_BLOCK + 10_000n,
          hash: observedBlockHash,
        }),
        request: async (value: Record<string, unknown>) => {
          requests.push(value);
          return [];
        },
      } as never,
      WALLET,
    ) as { complete: boolean };
    expect(result.complete).toBe(true);
    expect(requests).toHaveLength(2);
    for (const requestValue of requests) {
      const params = requestValue.params as Array<Record<string, unknown>>;
      const filter = params[0];
      if (filter === undefined) throw new Error("missing log filter");
      const fromBlock = BigInt(String(filter.fromBlock));
      const toBlock = BigInt(String(filter.toBlock));
      expect(toBlock - fromBlock).toBeLessThanOrEqual(9_999n);
      const topics = filter.topics as unknown[];
      expect(topics[0]).toHaveLength(5);
      expect(topics[2]).toHaveLength(2);
    }
  });

  it("cancels new discovery chunks and awaits every in-flight worker before rejecting", async () => {
    let markAllStarted: (() => void) | undefined;
    const allStarted = new Promise<void>((resolve) => {
      markAllStarted = resolve;
    });
    let releaseInFlight: (() => void) | undefined;
    const inFlightGate = new Promise<void>((resolve) => {
      releaseInFlight = resolve;
    });
    let requests = 0;
    let active = 0;
    const discovery = discoverAgents(
      {
        getBlock: async () => ({
          number: ERC8004_START_BLOCK + 50_000n,
          hash: `0x${"aa".repeat(32)}`,
        }),
        request: async () => {
          const index = requests;
          requests += 1;
          active += 1;
          if (requests === 5) markAllStarted?.();
          try {
            await allStarted;
            if (index === 0) throw new Error("execution reverted");
            await inFlightGate;
            return [];
          } finally {
            active -= 1;
          }
        },
      } as never,
      WALLET,
    );
    let settled = false;
    void discovery.then(
      () => { settled = true; },
      () => { settled = true; },
    );
    await allStarted;
    await Promise.resolve();
    await Promise.resolve();
    expect(requests).toBe(5);
    expect(active).toBe(4);
    expect(settled).toBe(false);

    releaseInFlight?.();
    await expect(discovery).rejects.toThrow("execution reverted");
    expect(active).toBe(0);
    expect(requests).toBe(5);
  });

  it("fails closed when historical discovery exceeds its strict log bound", async () => {
    const log = discoveryLog("Approval", {
      owner: "0x3333333333333333333333333333333333333333",
      approved: WALLET,
      tokenId: 1n,
    });
    const fake = {
      getBlock: async () => ({
        number: ERC8004_START_BLOCK + 10_000n,
        hash: `0x${"aa".repeat(32)}`,
      }),
      request: async () => Array.from({ length: 2_500 }, () => log),
    };
    await expect(discoverAgents(fake as never, WALLET)).rejects.toThrow(
      "log budget was exceeded",
    );
  });

  it("retains the exact registration outcome after the identity is transferred", async () => {
    const observedBlockHash = `0x${"aa".repeat(32)}` as const;
    const registrationHash = `0x${"bb".repeat(32)}` as const;
    const fake = {
      getBlock: async () => ({
        number: ERC8004_START_BLOCK,
        hash: observedBlockHash,
      }),
      request: async () => [
        discoveryLog(
          "Registered",
          { agentId: 9n, agentURI: "", owner: WALLET },
          registrationHash,
        ),
      ],
      getTransaction: async () => ({ from: WALLET, nonce: 7 }),
      readContract: async ({ functionName }: { functionName: string }) => {
        switch (functionName) {
          case "ownerOf":
            return "0x3333333333333333333333333333333333333333";
          case "tokenURI":
            return "";
          case "getAgentWallet":
            return "0x0000000000000000000000000000000000000000";
          case "getMetadata":
            return "0x";
          case "isAuthorizedOrOwner":
            return false;
          default:
            throw new Error(`unexpected read ${functionName}`);
        }
      },
    };
    const result = await discoverAgents(fake as never, WALLET, "7") as {
      candidates: unknown[];
      matchedRegistrationAgentIds: string[];
    };
    expect(result.candidates).toEqual([]);
    expect(result.matchedRegistrationAgentIds).toEqual(["9"]);
  });

  it("pins every agent field to one block and rejects a changed observation hash", async () => {
    const firstHash = `0x${"aa".repeat(32)}` as const;
    const secondHash = `0x${"bb".repeat(32)}` as const;
    let blockReads = 0;
    const observedReadBlocks: bigint[] = [];
    const fake = {
      getBlock: async () => ({
        number: 123n,
        hash: blockReads++ === 0 ? firstHash : secondHash,
      }),
      readContract: async ({
        functionName,
        blockNumber,
      }: {
        functionName: string;
        blockNumber: bigint;
      }) => {
        observedReadBlocks.push(blockNumber);
        switch (functionName) {
          case "ownerOf":
          case "getAgentWallet":
            return WALLET;
          case "tokenURI":
            return "";
          case "getMetadata":
            return "0x";
          case "isAuthorizedOrOwner":
            return true;
          default:
            throw new Error(`unexpected read ${functionName}`);
        }
      },
    };
    await expect(inspectAgent(fake as never, "9", WALLET)).rejects.toThrow(
      "changed while it was being read",
    );
    expect(observedReadBlocks).toHaveLength(7);
    expect(observedReadBlocks.every((block) => block === 123n)).toBe(true);
  });

  it("reads a confirmed nonce at the exact discovery block hash and echoes the binding", async () => {
    const observedBlockHash = `0x${"aa".repeat(32)}` as const;
    const requested: unknown[] = [];
    const fake = {
      getBlock: async () => ({ hash: observedBlockHash }),
      request: async (requestValue: unknown) => {
        requested.push(requestValue);
        return "0x7";
      },
      getTransactionCount: async () => 8,
    };
    const result = await readSignerNonceState(fake as never, WALLET, {
      observedBlockNumber: "123",
      observedBlockHash,
    });
    expect(result).toMatchObject({
      latestNonce: "7",
      pendingNonce: "8",
      observedBlockNumber: "123",
      observedBlockHash,
    });
    expect(requested).toEqual([
      {
        method: "eth_getTransactionCount",
        params: [WALLET, { blockHash: observedBlockHash, requireCanonical: true }],
      },
    ]);
  });

  it("fails closed when the discovery block mismatches or changes during nonce lookup", async () => {
    const expected = `0x${"aa".repeat(32)}` as const;
    const other = `0x${"bb".repeat(32)}` as const;
    await expect(
      readSignerNonceState(
        {
          getBlock: async () => ({ hash: other }),
        } as never,
        WALLET,
        { observedBlockNumber: "123", observedBlockHash: expected },
      ),
    ).rejects.toThrow("no longer canonical");

    let reads = 0;
    await expect(
      readSignerNonceState(
        {
          getBlock: async () => ({ hash: reads++ === 0 ? expected : other }),
          request: async () => "0x7",
          getTransactionCount: async () => 8,
        } as never,
        WALLET,
        { observedBlockNumber: "123", observedBlockHash: expected },
      ),
    ).rejects.toThrow("changed during the nonce read");
  });
});
