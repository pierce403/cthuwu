import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  ContractFunctionExecutionError,
  ContractFunctionRevertedError,
  encodeAbiParameters,
  encodeErrorResult,
  encodeEventTopics,
  parseAbi,
  stringToHex,
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
  authorizeRegistrationMint,
  buildPublicDiscoveryResult,
  classifyDiscoveredAgent,
  discoverAgents,
  inspectAgent,
  isTransientRpcError,
  parseErc8004Request,
  prepareErc8004Transaction,
  readSignerNonceState,
  requestFingerprint,
  selectDiscoveryMintAuthorization,
  sumThrottledL1Fees,
  sumL1FeesWithConservativeFallback,
  withBoundedRpcRetry,
  type MintAuthorization,
} from "./erc8004.js";

const WALLET = "0x1111111111111111111111111111111111111111";
const SIGNER_KEY = `0x${"22".repeat(32)}` as const;
const TEST_RECENT_DISCOVERY_BLOCKS_ENV =
  "CTHUWU_TEST_ERC8004_RECENT_DISCOVERY_BLOCKS";

async function withRecentDiscoveryWindow<T>(
  nodeEnvironment: string | undefined,
  override: string | undefined,
  action: () => Promise<T>,
): Promise<T> {
  const previousNodeEnvironment = process.env.NODE_ENV;
  const previousOverride = process.env[TEST_RECENT_DISCOVERY_BLOCKS_ENV];
  try {
    if (nodeEnvironment === undefined) delete process.env.NODE_ENV;
    else process.env.NODE_ENV = nodeEnvironment;
    if (override === undefined) {
      delete process.env[TEST_RECENT_DISCOVERY_BLOCKS_ENV];
    } else {
      process.env[TEST_RECENT_DISCOVERY_BLOCKS_ENV] = override;
    }
    return await action();
  } finally {
    if (previousNodeEnvironment === undefined) delete process.env.NODE_ENV;
    else process.env.NODE_ENV = previousNodeEnvironment;
    if (previousOverride === undefined) {
      delete process.env[TEST_RECENT_DISCOVERY_BLOCKS_ENV];
    } else {
      process.env[TEST_RECENT_DISCOVERY_BLOCKS_ENV] = previousOverride;
    }
  }
}

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

const TEST_TENTACLE_ID = "durable-tentacle";
const TEST_INBOX_ID = "a".repeat(64);

function sha256Json(value: unknown): string {
  return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}

function blockHash(blockNumber: bigint): Hex {
  return `0x${blockNumber.toString(16).padStart(64, "0")}` as Hex;
}

async function installEmptyDiscoveryJournal(
  identity: LoadedIdentity,
  throughBlock = ERC8004_START_BLOCK,
): Promise<MintAuthorization> {
  const throughBlockHash = blockHash(throughBlock);
  const base = {
    version: 1 as const,
    chainId: ERC8004_CHAIN_ID as 8453,
    registry: ERC8004_IDENTITY_REGISTRY,
    wallet: identity.walletAddress,
    tentacleId: TEST_TENTACLE_ID,
    xmtpInboxId: TEST_INBOX_ID,
    fromBlock: ERC8004_START_BLOCK.toString(),
    throughBlock: throughBlock.toString(),
    throughBlockHash,
    associatedAgentIds: [] as string[],
    walletRegistrations: [] as Array<{ agentId: string; transactionHash: Hex }>,
    operatorOwners: [] as string[],
  };
  const authorizationBase = {
    version: 1 as const,
    chainId: ERC8004_CHAIN_ID as 8453,
    registry: ERC8004_IDENTITY_REGISTRY,
    wallet: identity.walletAddress,
    tentacleId: TEST_TENTACLE_ID,
    xmtpInboxId: TEST_INBOX_ID,
    fromBlock: ERC8004_START_BLOCK.toString(),
    throughBlock: throughBlock.toString(),
    throughBlockHash,
    candidateSetHash: `0x${sha256Json([])}` as Hex,
  };
  const mintAuthorization: MintAuthorization = {
    ...authorizationBase,
    fingerprint: sha256Json(authorizationBase),
  };
  await writeFile(
    path.join(path.dirname(identity.identityPath), "erc8004-discovery-v1.json"),
    `${JSON.stringify({
      ...base,
      checkpointFingerprint: sha256Json(base),
      mintAuthorization,
    })}\n`,
    { mode: 0o600 },
  );
  return mintAuthorization;
}

function sameTentacleRead(
  wallet: string,
  tentacleId = TEST_TENTACLE_ID,
): (request: { functionName: string; args: readonly unknown[] }) => Promise<unknown> {
  return async ({ functionName, args }) => {
    switch (functionName) {
      case "ownerOf":
      case "getAgentWallet":
        return wallet;
      case "tokenURI":
        return "";
      case "getMetadata":
        switch (args[1]) {
          case ALLEGIANCE_KEY:
            return stringToHex(ALLEGIANCE_VALUE);
          case PROTOCOL_KEY:
            return stringToHex("1");
          case "cthuwu.tentacle-id":
            return stringToHex(tentacleId);
          default:
            return "0x";
        }
      case "isAuthorizedOrOwner":
        return true;
      default:
        throw new Error(`unexpected read ${functionName}`);
    }
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

  it("uses a bounded conservative L1 allowance when the provider lacks the oracle read", async () => {
    const fallback = await sumL1FeesWithConservativeFallback(
      ["a", "b", "c"],
      async () => {
        throw new Error("provider does not support the Base L1 fee oracle call");
      },
      25n,
      { throttleMs: 0, sleep: async () => undefined },
    );
    expect(fallback).toEqual({ fee: 75n, exact: false });

    const exact = await sumL1FeesWithConservativeFallback(
      [1, 2],
      async (value) => BigInt(value * 3),
      100n,
      { throttleMs: 0, sleep: async () => undefined },
    );
    expect(exact).toEqual({ fee: 9n, exact: true });

    await expect(
      sumL1FeesWithConservativeFallback([], async () => 0n, -1n),
    ).rejects.toThrow("cannot be negative");
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

  it("keeps the default recent window at exactly 20,000 blocks", async () => {
    const requests: Array<Record<string, unknown>> = [];
    const observedBlockNumber = ERC8004_START_BLOCK + 1_000_000n;
    const result = await withRecentDiscoveryWindow("test", undefined, async () =>
      await discoverAgents(
        {
          getBlock: async () => ({
            number: observedBlockNumber,
            hash: `0x${"aa".repeat(32)}`,
          }),
          request: async (value: Record<string, unknown>) => {
            requests.push(value);
            return [];
          },
        } as never,
        WALLET,
        undefined,
        "recent",
      ) as { complete: boolean; scope: string; fromBlock: string; candidates: unknown[] });

    expect(requests).toHaveLength(2);
    expect(result.complete).toBe(false);
    expect(result.scope).toBe("recent");
    expect(result.fromBlock).toBe((observedBlockNumber - 19_999n).toString());
    expect(result.candidates).toEqual([]);
  });

  it("permits only a smaller explicit recent window in test mode", async () => {
    const requests: Array<Record<string, unknown>> = [];
    const observedBlockNumber = ERC8004_START_BLOCK + 1_000_000n;
    const result = await withRecentDiscoveryWindow("test", "32", async () =>
      await discoverAgents(
        {
          getBlock: async () => ({
            number: observedBlockNumber,
            hash: `0x${"aa".repeat(32)}`,
          }),
          request: async (value: Record<string, unknown>) => {
            requests.push(value);
            return [];
          },
        } as never,
        WALLET,
        undefined,
        "recent",
      ) as { complete: boolean; fromBlock: string; mintAuthorization?: unknown });

    expect(requests).toHaveLength(1);
    expect(result.complete).toBe(false);
    expect(result.fromBlock).toBe((observedBlockNumber - 31n).toString());
    expect(result.mintAuthorization).toBeUndefined();
  });

  it("ignores the test-only recent-window setting outside test mode", async () => {
    const observedBlockNumber = ERC8004_START_BLOCK + 1_000_000n;
    for (const override of ["not-a-number", "20001"]) {
      const requests: Array<Record<string, unknown>> = [];
      const result = await withRecentDiscoveryWindow("production", override, async () =>
        await discoverAgents(
          {
            getBlock: async () => ({
              number: observedBlockNumber,
              hash: `0x${"aa".repeat(32)}`,
            }),
            request: async (value: Record<string, unknown>) => {
              requests.push(value);
              return [];
            },
          } as never,
          WALLET,
          undefined,
          "recent",
        ) as { fromBlock: string });
      expect(requests).toHaveLength(2);
      expect(result.fromBlock).toBe((observedBlockNumber - 19_999n).toString());
    }
  });

  it("rejects malformed or enlarging recent-window overrides in test mode", async () => {
    const observedBlockNumber = ERC8004_START_BLOCK + 1_000_000n;
    const fake = {
      getBlock: async () => ({
        number: observedBlockNumber,
        hash: `0x${"aa".repeat(32)}`,
      }),
      request: async () => [],
    };
    for (const override of ["0", "01", "20001"]) {
      await expect(withRecentDiscoveryWindow("test", override, async () =>
        await discoverAgents(
          fake as never,
          WALLET,
          undefined,
          "recent",
        ))).rejects.toThrow(/canonical positive integer|cannot exceed/u);
    }
  });

  it("never applies the test-only recent window to exhaustive discovery", async () => {
    const requests: Array<Record<string, unknown>> = [];
    const observedBlockNumber = ERC8004_START_BLOCK + 10_000n;
    const result = await withRecentDiscoveryWindow("test", "0", async () =>
      await discoverAgents(
        {
          getBlock: async () => ({
            number: observedBlockNumber,
            hash: `0x${"aa".repeat(32)}`,
          }),
          request: async (value: Record<string, unknown>) => {
            requests.push(value);
            return [];
          },
        } as never,
        WALLET,
        undefined,
        "exhaustive",
      ) as { complete: boolean; fromBlock: string });

    expect(requests).toHaveLength(2);
    expect(result.complete).toBe(true);
    expect(result.fromBlock).toBe(ERC8004_START_BLOCK.toString());
  });

  it("finds an identity older than 20,000 blocks only through complete discovery", async () => {
    const observedBlockNumber = ERC8004_START_BLOCK + 30_000n;
    const observedBlockHash = blockHash(observedBlockNumber);
    const registrationBlock = ERC8004_START_BLOCK + 1n;
    const registration = discoveryLog(
      "Registered",
      { agentId: 61_766n, agentURI: "", owner: WALLET },
      `0x${"61".repeat(32)}`,
    );
    const fake = {
      getBlock: async () => ({ number: observedBlockNumber, hash: observedBlockHash }),
      request: async (value: Record<string, unknown>) => {
        const filter = (value.params as Array<Record<string, unknown>>)[0]!;
        const fromBlock = BigInt(String(filter.fromBlock));
        const toBlock = BigInt(String(filter.toBlock));
        return fromBlock <= registrationBlock && registrationBlock <= toBlock ? [registration] : [];
      },
      readContract: sameTentacleRead(WALLET),
    };
    const context = { tentacleId: TEST_TENTACLE_ID, xmtpInboxId: TEST_INBOX_ID };
    const recent = await discoverAgents(
      fake as never,
      WALLET,
      undefined,
      "recent",
      context,
    ) as { complete: boolean; candidates: unknown[] };
    expect(recent).toMatchObject({ complete: false, candidates: [] });

    const exhaustive = await discoverAgents(
      fake as never,
      WALLET,
      undefined,
      "exhaustive",
      context,
    ) as {
      complete: boolean;
      fromBlock: string;
      candidates: Array<{ agentId: string; sameTentacle: boolean }>;
    };
    expect(exhaustive.complete).toBe(true);
    expect(exhaustive.fromBlock).toBe(ERC8004_START_BLOCK.toString());
    expect(exhaustive.candidates).toEqual([
      expect.objectContaining({ agentId: "61766", sameTentacle: true }),
    ]);
  });

  it("directly verifies both historical duplicates and returns the lower ID first", async () => {
    const observedBlockNumber = ERC8004_START_BLOCK + 1n;
    const observedBlockHash = blockHash(observedBlockNumber);
    const fake = {
      getBlock: async () => ({ number: observedBlockNumber, hash: observedBlockHash }),
      request: async () => [
        discoveryLog(
          "Registered",
          { agentId: 63_846n, agentURI: "", owner: WALLET },
          `0x${"63".repeat(32)}`,
        ),
        discoveryLog(
          "Registered",
          { agentId: 61_766n, agentURI: "", owner: WALLET },
          `0x${"62".repeat(32)}`,
        ),
      ],
      readContract: sameTentacleRead(WALLET),
    };
    const result = await discoverAgents(
      fake as never,
      WALLET,
      undefined,
      "exhaustive",
      { tentacleId: TEST_TENTACLE_ID, xmtpInboxId: TEST_INBOX_ID },
    ) as { complete: boolean; candidates: Array<{ agentId: string; sameTentacle: boolean }> };
    expect(result.complete).toBe(true);
    expect(result.candidates.map(({ agentId }) => agentId)).toEqual(["61766", "63846"]);
    expect(result.candidates.every(({ sameTentacle }) => sameTentacle)).toBe(true);
  });

  it("advances a complete canonical checkpoint without rescanning old history", async () => {
    const checkpointBlock = ERC8004_START_BLOCK + 9_999n;
    const observedBlockNumber = checkpointBlock + 10_000n;
    const requested: Array<{ fromBlock: bigint; toBlock: bigint }> = [];
    const fake = {
      getBlock: async ({ blockNumber }: { blockNumber?: bigint } = {}) => ({
        number: blockNumber ?? observedBlockNumber,
        hash: blockHash(blockNumber ?? observedBlockNumber),
      }),
      request: async (value: Record<string, unknown>) => {
        const filter = (value.params as Array<Record<string, unknown>>)[0]!;
        requested.push({
          fromBlock: BigInt(String(filter.fromBlock)),
          toBlock: BigInt(String(filter.toBlock)),
        });
        return [];
      },
    };
    const result = await discoverAgents(
      fake as never,
      WALLET,
      undefined,
      "exhaustive",
      { tentacleId: TEST_TENTACLE_ID, xmtpInboxId: TEST_INBOX_ID },
      {
        version: 1,
        chainId: ERC8004_CHAIN_ID,
        registry: ERC8004_IDENTITY_REGISTRY,
        wallet: WALLET,
        tentacleId: TEST_TENTACLE_ID,
        xmtpInboxId: TEST_INBOX_ID,
        fromBlock: ERC8004_START_BLOCK.toString(),
        throughBlock: checkpointBlock.toString(),
        throughBlockHash: blockHash(checkpointBlock),
        associatedAgentIds: [],
        walletRegistrations: [],
        operatorOwners: [],
        checkpointFingerprint: "b".repeat(64),
      } as never,
    ) as {
      complete: boolean;
      source: string;
      fromBlock: string;
      coverage: { throughBlock: string; throughBlockHash: Hex };
    };
    expect(result).toMatchObject({
      complete: true,
      source: "canonical-logs-checkpoint",
      fromBlock: ERC8004_START_BLOCK.toString(),
      coverage: {
        throughBlock: observedBlockNumber.toString(),
        throughBlockHash: blockHash(observedBlockNumber),
      },
    });
    expect(requested[0]!.fromBlock).toBe(checkpointBlock + 1n);
    expect(requested.at(-1)!.toBlock).toBe(observedBlockNumber);
  });

  it("classifies exact same-Tentacle duplicates without treating wallet-only identities as aliases", () => {
    const base = {
      agentId: "61766",
      owner: WALLET,
      agentURI: "",
      agentWallet: WALLET,
      authorized: true,
      walletVerified: true,
      declaresTentacleAllegiance: true,
      protocolCompatible: true,
      allegiance: { hex: "0x", utf8: ALLEGIANCE_VALUE },
      protocol: { hex: "0x", utf8: "1" },
      tentacleId: { hex: "0x", utf8: "durable-tentacle" },
    };
    expect(classifyDiscoveredAgent(base, {
      tentacleId: "durable-tentacle",
      xmtpInboxId: "a".repeat(64),
    })).toMatchObject({
      sameTentacle: true,
      ambiguousTentacle: false,
      identityEvidence: ["exact-tentacle-id", "legacy-allegiance"],
    });
    expect(classifyDiscoveredAgent({
      ...base,
      declaresTentacleAllegiance: false,
      protocolCompatible: false,
      tentacleId: { hex: "0x", utf8: null },
    }, {
      tentacleId: "durable-tentacle",
      xmtpInboxId: "a".repeat(64),
    })).toMatchObject({
      sameTentacle: false,
      ambiguousTentacle: true,
      identityEvidence: ["wallet-only"],
    });
    expect(classifyDiscoveredAgent({
      ...base,
      tentacleId: { hex: "0x", utf8: "another-durable-tentacle" },
    }, {
      tentacleId: "durable-tentacle",
      xmtpInboxId: "a".repeat(64),
    })).toMatchObject({
      sameTentacle: false,
      ambiguousTentacle: false,
    });
  });

  it("rejects register before fee, nonce, preparation, or signer work without complete discovery", async () => {
    const identity = signerIdentity("production");
    await expect(authorizeRegistrationMint(
      {} as never,
      identity,
      "registration:no-discovery",
      { type: "register", nonce: "0" },
    )).rejects.toThrow("requires positively complete historical identity discovery");
  });

  it("allocates a fresh complete proof exactly once and permits only its exact lost-response replay", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      const mintAuthorization = await installEmptyDiscoveryJournal(identity);
      let observedBlockNumber = ERC8004_START_BLOCK + 1n;
      let logRequests = 0;
      const fake = {
        getBlock: async ({ blockNumber }: { blockNumber?: bigint } = {}) => ({
          number: blockNumber ?? observedBlockNumber,
          hash: blockHash(blockNumber ?? observedBlockNumber),
        }),
        request: async (requestValue: { method: string }) => {
          if (requestValue.method === "eth_getTransactionCount") return "0x7";
          logRequests += 1;
          return [];
        },
        getTransactionCount: async () => 7,
      };
      const operation = { type: "register" as const, nonce: "7", mintAuthorization };
      await expect(authorizeRegistrationMint(
        fake as never,
        identity,
        "registration:fresh",
        operation,
      )).resolves.toBeUndefined();
      expect(await readdir(directory)).toContain("erc8004-registration-mint-v1.json");

      const advancedJournal = JSON.parse(await readFile(
        path.join(directory, "erc8004-discovery-v1.json"),
        "utf8",
      )) as { mintAuthorization: MintAuthorization };
      expect(advancedJournal.mintAuthorization.fingerprint).not.toBe(
        mintAuthorization.fingerprint,
      );
      const replayAuthorization = await selectDiscoveryMintAuthorization(
        identity,
        advancedJournal.mintAuthorization,
        "registration:fresh",
        "7",
      );
      expect(replayAuthorization).toEqual(advancedJournal.mintAuthorization);
      await expect(selectDiscoveryMintAuthorization(
        identity,
        advancedJournal.mintAuthorization,
        "registration:duplicate",
        "8",
      )).resolves.toBeUndefined();
      expect(buildPublicDiscoveryResult(
        { complete: true, mintAuthorization: advancedJournal.mintAuthorization },
        {
          version: 1,
          fingerprint: "b".repeat(64),
          throughBlock: advancedJournal.mintAuthorization.throughBlock,
          throughBlockHash: advancedJournal.mintAuthorization.throughBlockHash,
        },
        undefined,
      )).not.toHaveProperty("mintAuthorization");

      // A later head replaces the compact discovery checkpoint. The exact action/proof/nonce
      // remains replayable after a lost RPC response without authorizing a second action.
      observedBlockNumber += 1n;
      await expect(authorizeRegistrationMint(
        fake as never,
        identity,
        "registration:fresh",
        { ...operation, mintAuthorization: replayAuthorization! },
      )).resolves.toBeUndefined();
      const requestsBeforeConflict = logRequests;
      await expect(authorizeRegistrationMint(
        fake as never,
        identity,
        "registration:duplicate",
        { ...operation, nonce: "8" },
      )).rejects.toThrow("already authorized another registration action");
      expect(logRequests).toBe(requestsBeforeConflict);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("refuses a fresh proof when a transaction exists beyond finalized discovery coverage", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      const mintAuthorization = await installEmptyDiscoveryJournal(identity);
      const observedBlockNumber = ERC8004_START_BLOCK + 1n;
      const fake = {
        getBlock: async ({ blockNumber }: { blockNumber?: bigint } = {}) => ({
          number: blockNumber ?? observedBlockNumber,
          hash: blockHash(blockNumber ?? observedBlockNumber),
        }),
        request: async (requestValue: { method: string }) =>
          requestValue.method === "eth_getTransactionCount" ? "0x7" : [],
        // Model a register confirmed only above the finalized discovery block: current pending
        // and latest would both be 8, while the exact covered block nonce is still 7.
        getTransactionCount: async () => 8,
      };
      await expect(authorizeRegistrationMint(
        fake as never,
        identity,
        "registration:pending-gap",
        { type: "register", nonce: "8", mintAuthorization },
      )).rejects.toThrow("confirmed or pending beyond complete discovery coverage");
      expect(await readdir(directory)).not.toContain("erc8004-registration-mint-v1.json");
      expect(await readdir(directory)).not.toContain("erc8004-signer-nonce-v1-8.json");
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("never allocates when an external post-finalized association appears at the latest head", async () => {
    const existing = await journalIdentity();
    try {
      const mintAuthorization = await installEmptyDiscoveryJournal(existing.identity);
      const finalizedBlockNumber = ERC8004_START_BLOCK;
      const latestBlockNumber = finalizedBlockNumber + 1n;
      // Anyone can transfer an existing ERC-721 identity to this wallet. That changes no
      // durable-wallet nonce, so the signer gate must cover current canonical logs as well as
      // compare its nonce at the discovery head.
      const externalTransfer = discoveryLog(
        "Transfer",
        {
          from: "0x3333333333333333333333333333333333333333",
          to: existing.identity.walletAddress,
          tokenId: 61_766n,
        },
      );
      const fake = {
        getBlock: async ({ blockNumber, blockTag }: {
          blockNumber?: bigint;
          blockTag?: "finalized" | "latest";
        } = {}) => {
          const number = blockNumber ??
            (blockTag === "finalized" ? finalizedBlockNumber : latestBlockNumber);
          return { number, hash: blockHash(number) };
        },
        request: async (value: Record<string, unknown>) => {
          const filter = (value.params as Array<Record<string, unknown>>)[0]!;
          return BigInt(String(filter.toBlock)) >= latestBlockNumber ? [externalTransfer] : [];
        },
        readContract: sameTentacleRead(existing.identity.walletAddress),
      };
      await expect(authorizeRegistrationMint(
        fake as never,
        existing.identity,
        "registration:stale",
        { type: "register", nonce: "7", mintAuthorization },
      )).rejects.toThrow("found an existing or ambiguous Cthuwu identity");
      expect(await readdir(existing.directory)).not.toContain("erc8004-registration-mint-v1.json");
    } finally {
      await rm(existing.directory, { recursive: true, force: true });
    }
  });

  it("never allocates a signer action when forward discovery is incomplete", async () => {
    const unavailable = await journalIdentity();
    try {
      const mintAuthorization = await installEmptyDiscoveryJournal(unavailable.identity);
      const observedBlockNumber = ERC8004_START_BLOCK + 1n;
      const fake = {
        getBlock: async ({ blockNumber }: { blockNumber?: bigint } = {}) => ({
          number: blockNumber ?? observedBlockNumber,
          hash: blockHash(blockNumber ?? observedBlockNumber),
        }),
        request: async () => ({ malformed: true }),
      };
      await expect(authorizeRegistrationMint(
        fake as never,
        unavailable.identity,
        "registration:unavailable",
        { type: "register", nonce: "7", mintAuthorization },
      )).rejects.toThrow("malformed eth_getLogs response");
      expect(await readdir(unavailable.directory)).not.toContain("erc8004-registration-mint-v1.json");
    } finally {
      await rm(unavailable.directory, { recursive: true, force: true });
    }
  });

  it("does not adopt a wallet-only identity and blocks registration until its provenance is resolved", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      const mintAuthorization = await installEmptyDiscoveryJournal(identity);
      const observedBlockNumber = ERC8004_START_BLOCK + 1n;
      const registration = discoveryLog(
        "Registered",
        { agentId: 61_766n, agentURI: "", owner: identity.walletAddress },
        `0x${"66".repeat(32)}`,
      );
      const fake = {
        getBlock: async ({ blockNumber }: { blockNumber?: bigint } = {}) => ({
          number: blockNumber ?? observedBlockNumber,
          hash: blockHash(blockNumber ?? observedBlockNumber),
        }),
        request: async () => [registration],
        readContract: async ({ functionName }: { functionName: string }) => {
          switch (functionName) {
            case "ownerOf":
            case "getAgentWallet":
              return identity.walletAddress;
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
      const discovered = await discoverAgents(
        fake as never,
        identity.walletAddress,
        undefined,
        "exhaustive",
        { tentacleId: TEST_TENTACLE_ID, xmtpInboxId: TEST_INBOX_ID },
      ) as { candidates: Array<{ sameTentacle: boolean; ambiguousTentacle: boolean }> };
      expect(discovered.candidates).toEqual([
        expect.objectContaining({ sameTentacle: false, ambiguousTentacle: true }),
      ]);
      await expect(authorizeRegistrationMint(
        fake as never,
        identity,
        "registration:wallet-only",
        { type: "register", nonce: "7", mintAuthorization },
      )).rejects.toThrow("found an existing or ambiguous Cthuwu identity");
      expect(await readdir(directory)).not.toContain("erc8004-registration-mint-v1.json");
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("rescans old owner history when a post-checkpoint blanket approval exposes an identity", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      const checkpointBlock = ERC8004_START_BLOCK + 9_999n;
      const observedBlockNumber = checkpointBlock + 1n;
      const mintAuthorization = await installEmptyDiscoveryJournal(identity, checkpointBlock);
      const owner = "0x3333333333333333333333333333333333333333";
      const approval = discoveryLog("ApprovalForAll", {
        owner,
        operator: identity.walletAddress,
        approved: true,
      });
      const registration = discoveryLog(
        "Registered",
        { agentId: 61_766n, agentURI: "", owner },
        `0x${"65".repeat(32)}`,
      );
      const ownerHistoryRanges: Array<{ fromBlock: bigint; toBlock: bigint }> = [];
      const readSameTentacle = sameTentacleRead(identity.walletAddress);
      const fake = {
        getBlock: async ({ blockNumber }: { blockNumber?: bigint } = {}) => ({
          number: blockNumber ?? observedBlockNumber,
          hash: blockHash(blockNumber ?? observedBlockNumber),
        }),
        request: async (value: Record<string, unknown>) => {
          const filter = (value.params as Array<Record<string, unknown>>)[0]!;
          const eventTopics = filter.topics as unknown[];
          const selectedEvents = eventTopics[0] as unknown[];
          const fromBlock = BigInt(String(filter.fromBlock));
          const toBlock = BigInt(String(filter.toBlock));
          if (selectedEvents.length === 5) return [approval];
          ownerHistoryRanges.push({ fromBlock, toBlock });
          return fromBlock <= ERC8004_START_BLOCK && ERC8004_START_BLOCK <= toBlock
            ? [registration]
            : [];
        },
        readContract: async (requestValue: { functionName: string; args: readonly unknown[] }) =>
          requestValue.functionName === "isApprovedForAll"
            ? true
            : readSameTentacle(requestValue),
      };
      await expect(authorizeRegistrationMint(
        fake as never,
        identity,
        "registration:new-operator-history",
        { type: "register", nonce: "7", mintAuthorization },
      )).rejects.toThrow("found an existing or ambiguous Cthuwu identity");
      expect(ownerHistoryRanges[0]!.fromBlock).toBe(ERC8004_START_BLOCK);
      expect(await readdir(directory)).not.toContain("erc8004-registration-mint-v1.json");
    } finally {
      await rm(directory, { recursive: true, force: true });
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

  it("returns an exact authoritative wire result only for canonical ownerOf nonexistent-token", async () => {
    const observedBlockHash = `0x${"aa".repeat(32)}` as const;
    const nonexistentAbi = parseAbi([
      "error ERC721NonexistentToken(uint256 tokenId)",
    ]);
    const reverted = new ContractFunctionRevertedError({
      abi: nonexistentAbi,
      data: encodeErrorResult({
        abi: nonexistentAbi,
        errorName: "ERC721NonexistentToken",
        args: [63846n],
      }),
      functionName: "ownerOf",
    });
    const notFound = new ContractFunctionExecutionError(reverted, {
      abi: nonexistentAbi,
      args: [63846n],
      contractAddress: ERC8004_IDENTITY_REGISTRY,
      functionName: "ownerOf",
    });
    const calls: string[] = [];
    const fake = {
      getBlock: async () => ({ number: 50_000_000n, hash: observedBlockHash }),
      readContract: async ({ functionName }: { functionName: string }) => {
        calls.push(functionName);
        throw notFound;
      },
    };

    const result = await inspectAgent(fake as never, "63846", WALLET);

    expect(JSON.parse(JSON.stringify(result))).toEqual({
      agentId: "63846",
      agentExists: false,
      authority: "canonical-base-ownerOf",
      observedBlock: "50000000",
      observedBlockHash,
    });
    expect(calls).toEqual(["ownerOf"]);
  });

  it("does not convert provider uncertainty or a mismatched revert into not-found", async () => {
    const observedBlockHash = `0x${"aa".repeat(32)}` as const;
    const base = {
      getBlock: async () => ({ number: 50_000_000n, hash: observedBlockHash }),
    };
    await expect(
      inspectAgent(
        {
          ...base,
          readContract: async () => {
            throw new Error("provider timed out");
          },
        } as never,
        "63846",
        WALLET,
      ),
    ).rejects.toThrow("provider timed out");

    const nonexistentAbi = parseAbi([
      "error ERC721NonexistentToken(uint256 tokenId)",
    ]);
    const anotherTokenRevert = new ContractFunctionRevertedError({
      abi: nonexistentAbi,
      data: encodeErrorResult({
        abi: nonexistentAbi,
        errorName: "ERC721NonexistentToken",
        args: [61766n],
      }),
      functionName: "ownerOf",
    });
    const anotherToken = new ContractFunctionExecutionError(anotherTokenRevert, {
      abi: nonexistentAbi,
      args: [63846n],
      contractAddress: ERC8004_IDENTITY_REGISTRY,
      functionName: "ownerOf",
    });
    await expect(
      inspectAgent(
        {
          ...base,
          readContract: async () => {
            throw anotherToken;
          },
        } as never,
        "63846",
        WALLET,
      ),
    ).rejects.toBe(anotherToken);
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
