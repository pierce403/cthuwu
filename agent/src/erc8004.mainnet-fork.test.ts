import { createHash, randomBytes } from "node:crypto";
import { chmod, mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  encodeFunctionData,
  getAddress,
  parseAbi,
  stringToHex,
  toHex,
  type Address,
} from "viem";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { LoadedIdentity } from "./identity.js";
import {
  ALLEGIANCE_KEY,
  ALLEGIANCE_VALUE,
  ERC8004_CHAIN_ID,
  ERC8004_IDENTITY_REGISTRY,
  ERC8004_REPUTATION_REGISTRY,
  PROTOCOL_KEY,
  PROTOCOL_VALUE,
  TENTACLE_ID_KEY,
  assertProductionIdentity,
  handleErc8004Request,
  parseErc8004Request,
  type Erc8004Request,
  type Erc8004Response,
} from "./erc8004.js";

const RUN_MAINNET_FORK = process.env.CTHUWU_RUN_MAINNET_FORK_TEST === "1";
const RPC_ENDPOINT =
  process.env.CTHUWU_ERC8004_FORK_RPC ?? "http://127.0.0.1:8545";
const TEST_RECENT_DISCOVERY_BLOCKS_ENV =
  "CTHUWU_TEST_ERC8004_RECENT_DISCOVERY_BLOCKS";

// This is the first compact deterministic fork point after both canonical proxies reached the
// pinned 2.0.0 implementations. It is only seventeen blocks after the Identity Registry start,
// keeping recovery discovery to one bounded log range while still exercising the real deployment.
const FORK_BLOCK_NUMBER = 41_663_800;
const FORK_BLOCK_HASH =
  "0x8c53425fc3b456f044a5169bc32518817f7c7c7a858e804ff16f8540d0f98648";
const TEST_TENTACLE_ID = "mainnet-fork-tentacle-v1";

type JsonRecord = Record<string, unknown>;

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requiredRecord(value: unknown, label: string): JsonRecord {
  if (!isRecord(value)) throw new Error(`${label} was not an object`);
  return value;
}

function requiredString(value: unknown, label: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${label} was not a nonempty string`);
  }
  return value;
}

async function rpc(method: string, params: readonly unknown[]): Promise<unknown> {
  const response = await fetch(RPC_ENDPOINT, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  if (!response.ok) throw new Error(`fork RPC returned HTTP ${response.status}`);
  const envelope = requiredRecord(await response.json(), "fork RPC response");
  if (envelope.error !== undefined) {
    const error = requiredRecord(envelope.error, "fork RPC error");
    throw new Error(
      typeof error.message === "string" ? error.message : "fork RPC request failed",
    );
  }
  if (!("result" in envelope)) throw new Error("fork RPC response omitted result");
  return envelope.result;
}

function parsedRequest(
  actionId: string,
  operation: Record<string, unknown>,
): Erc8004Request {
  return parseErc8004Request({ version: 1, actionId, operation });
}

function successfulResult(response: Erc8004Response): JsonRecord {
  if (!response.ok) {
    throw new Error(
      `ERC-8004 request failed with ${response.code}: ${response.message}`,
    );
  }
  expect(response.ok).toBe(true);
  return requiredRecord(response.result, "ERC-8004 result");
}

function stringArray(value: unknown, label: string): string[] {
  if (!Array.isArray(value) || value.some((entry) => typeof entry !== "string")) {
    throw new Error(`${label} was not a string array`);
  }
  return value as string[];
}

function responseHex(value: unknown, label: string): string {
  const result = requiredString(value, label);
  if (!/^0x[0-9a-fA-F]+$/u.test(result)) throw new Error(`${label} was not hex`);
  return result;
}

describe.skipIf(!RUN_MAINNET_FORK)(
  "canonical Base ERC-8004 registration through the narrow signer",
  () => {
    let stateDirectory = "";
    let identity: LoadedIdentity;
    let wallet: Address;
    let previousRecentDiscoveryBlocks: string | undefined;

    const loadIdentity = async (): Promise<LoadedIdentity> => identity;

    async function call(
      actionId: string,
      operation: Record<string, unknown>,
    ): Promise<Erc8004Response> {
      return handleErc8004Request(
        parsedRequest(actionId, operation),
        RPC_ENDPOINT,
        loadIdentity,
      );
    }

    async function currentNonce(action: string): Promise<string> {
      const result = successfulResult(
        await call(`${action}:nonce`, { type: "transaction_nonce", wallet }),
      );
      const pending = requiredString(result.pendingNonce, "pending nonce");
      expect(result.latestNonce).toBe(pending);
      expect(result.wallet).toBe(wallet);
      return pending;
    }

    async function writeAndConfirm(
      actionId: string,
      operation: Record<string, unknown>,
    ): Promise<JsonRecord> {
      const nonce = await currentNonce(actionId);
      const write = successfulResult(
        await call(actionId, { ...operation, nonce }),
      );
      expect(write.wallet).toBe(wallet);
      expect(write.chainId).toBe(ERC8004_CHAIN_ID);
      expect(write.registry).toBe(ERC8004_IDENTITY_REGISTRY);
      expect(write.valueWei).toBe("0");
      expect(write.transactionNonce).toBe(nonce);
      const transactionHash = requiredString(
        write.transactionHash,
        "transaction hash",
      );
      const receipt = successfulResult(
        await call(`${actionId}:receipt`, {
          type: "receipt",
          transactionHash,
        }),
      );
      expect(receipt.status).toBe("success");
      expect(receipt.transactionHash).toBe(transactionHash);
      expect(receipt.blockHash).toBe(receipt.canonicalBlockHash);
      return receipt;
    }

    beforeAll(async () => {
      previousRecentDiscoveryBlocks =
        process.env[TEST_RECENT_DISCOVERY_BLOCKS_ENV];
      process.env[TEST_RECENT_DISCOVERY_BLOCKS_ENV] = "32";
      stateDirectory = await mkdtemp(
        path.join(tmpdir(), "cthuwu-mainnet-fork-signer-"),
      );
      await chmod(stateDirectory, 0o700);
      const databaseDirectory = path.join(stateDirectory, "xmtp");
      await mkdir(databaseDirectory, { mode: 0o700 });

      // Both private values are generated for this process, retained only in memory, never printed,
      // and destroyed with the temporary state directory after the fork process exits.
      const walletKey = generatePrivateKey();
      const account = privateKeyToAccount(walletKey);
      wallet = getAddress(account.address);
      identity = {
        version: 1,
        environment: "production",
        walletKey,
        dbEncryptionKey: `0x${randomBytes(32).toString("hex")}`,
        createdAt: new Date().toISOString(),
        identityPath: path.join(stateDirectory, "xmtp-identity.json"),
        dbDirectory: databaseDirectory,
        walletAddress: wallet,
      };

      expect(assertProductionIdentity(identity, wallet)).toBe(wallet);
      expect(await rpc("eth_chainId", [])).toBe(toHex(ERC8004_CHAIN_ID));
      const forkBlock = requiredRecord(
        await rpc("eth_getBlockByNumber", [toHex(FORK_BLOCK_NUMBER), false]),
        "fork anchor block",
      );
      expect(forkBlock.hash).toBe(FORK_BLOCK_HASH);

      // Anvil exposes `finalized` as latest minus 64 blocks. The pinned anchor deliberately sits
      // only seventeen blocks after registry deployment, so advance local empty blocks before the
      // first exhaustive discovery while retaining the exact canonical fork-anchor assertion.
      await rpc("anvil_mine", [toHex(70)]);

      // This is an Anvil-only chain-control operation. It funds the ephemeral identity on the local
      // fork and cannot touch Base; every registry transaction still goes through the narrow signer.
      const fundedBalance = 5n * 10n ** 18n;
      await rpc("anvil_setBalance", [wallet, toHex(fundedBalance)]);
      expect(
        BigInt(
          responseHex(
            await rpc("eth_getBalance", [wallet, "latest"]),
            "funded fork balance",
          ),
        ),
      ).toBe(fundedBalance);
    }, 60_000);

    afterAll(async () => {
      if (previousRecentDiscoveryBlocks === undefined) {
        delete process.env[TEST_RECENT_DISCOVERY_BLOCKS_ENV];
      } else {
        process.env[TEST_RECENT_DISCOVERY_BLOCKS_ENV] =
          previousRecentDiscoveryBlocks;
      }
      if (stateDirectory !== "") {
        await rm(stateDirectory, { recursive: true, force: true });
      }
    });

    it(
      "registers exactly once, recovers a lost response, and publishes verified Tentacle state",
      async () => {
        const deployment = successfulResult(
          await call("mainnet-fork:inspect-registry", {
            type: "inspect_registry",
          }),
        );
        expect(deployment).toMatchObject({
          chainId: 8453,
          identityRegistry: ERC8004_IDENTITY_REGISTRY,
          reputationRegistry: ERC8004_REPUTATION_REGISTRY,
          identityVersion: "2.0.0",
          reputationVersion: "2.0.0",
          interfaceRevision: "registration-v1",
          interfaceComplete: true,
        });

        const inboxId = createHash("sha256").update(wallet).digest("hex");
        const initialDiscovery = successfulResult(
          await call("mainnet-fork:pre-mint-discovery", {
            type: "discover",
            wallet,
            scope: "exhaustive",
            tentacleId: TEST_TENTACLE_ID,
            xmtpInboxId: inboxId,
          }),
        );
        expect(initialDiscovery.complete).toBe(true);
        expect(initialDiscovery.candidates).toEqual([]);
        const mintAuthorization = requiredRecord(
          initialDiscovery.mintAuthorization,
          "mint authorization",
        );

        const registrationAction = "mainnet-fork:register-lost-response";
        const registrationNonce = await currentNonce(registrationAction);
        const registrationRequest = parsedRequest(registrationAction, {
          type: "register",
          nonce: registrationNonce,
          mintAuthorization,
        });
        const registrationResponse = await handleErc8004Request(
          registrationRequest,
          RPC_ENDPOINT,
          loadIdentity,
        );
        const submitted = successfulResult(registrationResponse);
        const registrationHash = requiredString(
          submitted.transactionHash,
          "registration transaction hash",
        );

        // Model the transport losing the response after broadcast: retry the exact persisted action
        // without consuming its returned ID. Latest discovery already sees the wallet-controlled
        // registration, so the consumed proof fails closed before finalized nonce recovery below.
        const replay = await handleErc8004Request(
          registrationRequest,
          RPC_ENDPOINT,
          loadIdentity,
        );
        expect(replay).toMatchObject({
          ok: false,
          recoverable: true,
          code: "mint_authorization_stale",
        });

        // Anvil models Base's finalized tag as latest minus 64 blocks. Advance only local empty
        // blocks so the real registration is visible to the production finalized-only discovery
        // policy; no wall-clock wait or additional upstream history is needed.
        await rpc("anvil_mine", [toHex(70)]);
        const discovered = successfulResult(
          await call("mainnet-fork:recover-registration", {
            type: "discover",
            wallet,
            registrationNonce,
            scope: "exhaustive",
          }),
        );
        expect(discovered.complete).toBe(true);
        const matched = stringArray(
          discovered.matchedRegistrationAgentIds,
          "matched registration IDs",
        );
        expect(matched).toHaveLength(1);
        const agentId = requiredString(matched[0], "recovered agent ID");

        const candidates = discovered.candidates;
        expect(Array.isArray(candidates)).toBe(true);
        if (!Array.isArray(candidates)) throw new Error("candidates was not an array");
        expect(
          candidates.filter(
            (candidate) =>
              isRecord(candidate) && candidate.agentId === agentId,
          ),
        ).toHaveLength(1);

        const registrationReceipt = successfulResult(
          await call("mainnet-fork:registration-receipt", {
            type: "receipt",
            transactionHash: registrationHash,
          }),
        );
        expect(registrationReceipt).toMatchObject({
          status: "success",
          transactionHash: registrationHash,
          agentId,
        });
        expect(registrationReceipt.blockHash).toBe(
          registrationReceipt.canonicalBlockHash,
        );

        // The generated wallet had no pre-fork history. One current ERC-721 proves the lost-response
        // replay did not create a second identity; this read is not a signing path.
        const balanceData = encodeFunctionData({
          abi: parseAbi([
            "function balanceOf(address owner) view returns (uint256)",
          ]),
          functionName: "balanceOf",
          args: [wallet],
        });
        const ownerBalance = responseHex(
          await rpc("eth_call", [
            { to: ERC8004_IDENTITY_REGISTRY, data: balanceData },
            "latest",
          ]),
          "registry owner balance",
        );
        expect(BigInt(ownerBalance)).toBe(1n);

        let inspected = successfulResult(
          await call("mainnet-fork:inspect-initial-agent", {
            type: "inspect_agent",
            agentId,
            wallet,
          }),
        );
        expect(inspected.owner).toBe(wallet);
        if (inspected.walletVerified !== true) {
          await writeAndConfirm("mainnet-fork:set-agent-wallet", {
            type: "set_agent_wallet",
            agentId,
          });
          inspected = successfulResult(
            await call("mainnet-fork:inspect-published-wallet", {
              type: "inspect_agent",
              agentId,
              wallet,
            }),
          );
        }
        expect(inspected.agentWallet).toBe(wallet);
        expect(inspected.walletVerified).toBe(true);

        const manifest = {
          schemaVersion: 1,
          protocol: 1,
          tentacleId: TEST_TENTACLE_ID,
          erc8004: {
            chainId: ERC8004_CHAIN_ID,
            registry: ERC8004_IDENTITY_REGISTRY,
            agentId,
          },
          xmtp: {
            environment: "production",
            endpoint: `xmtp://${inboxId}`,
          },
          capabilities: ["direct-xmtp-messaging"],
        };
        const manifestUri = `data:application/json;base64,${Buffer.from(
          JSON.stringify(manifest),
        ).toString("base64")}`;
        const numericAgentId = Number(agentId);
        expect(Number.isSafeInteger(numericAgentId)).toBe(true);
        const profile = {
          type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
          name: "Cthuwu Mainnet-Fork Tentacle",
          description:
            "Ephemeral verification of one independently operated Tentacle on the canonical Base registry.",
          image: "https://cthuwu.app/icons/cthuwu-192.png",
          services: [
            {
              name: "CTHUWU-XMTP",
              endpoint: `xmtp://${inboxId}`,
              version: "1",
            },
            { name: "CTHUWU", endpoint: manifestUri, version: "1" },
          ],
          x402Support: false,
          active: true,
          registrations: [
            {
              agentId: numericAgentId,
              agentRegistry: `eip155:${ERC8004_CHAIN_ID}:${ERC8004_IDENTITY_REGISTRY}`,
            },
          ],
        };
        const finalAgentUri = `data:application/json;base64,${Buffer.from(
          JSON.stringify(profile),
        ).toString("base64")}`;

        await writeAndConfirm("mainnet-fork:set-final-uri", {
          type: "set_agent_uri",
          agentId,
          agentURI: finalAgentUri,
        });
        await writeAndConfirm("mainnet-fork:set-allegiance", {
          type: "set_metadata",
          agentId,
          key: ALLEGIANCE_KEY,
          value: ALLEGIANCE_VALUE,
        });
        await writeAndConfirm("mainnet-fork:set-protocol", {
          type: "set_metadata",
          agentId,
          key: PROTOCOL_KEY,
          value: PROTOCOL_VALUE,
        });
        await writeAndConfirm("mainnet-fork:set-tentacle-id", {
          type: "set_metadata",
          agentId,
          key: TENTACLE_ID_KEY,
          value: TEST_TENTACLE_ID,
        });

        const verified = successfulResult(
          await call("mainnet-fork:inspect-complete-agent", {
            type: "inspect_agent",
            agentId,
            wallet,
          }),
        );
        expect(verified).toMatchObject({
          agentId,
          owner: wallet,
          agentURI: finalAgentUri,
          agentWallet: wallet,
          authorized: true,
          declaresTentacleAllegiance: true,
          protocolCompatible: true,
          walletVerified: true,
          allegiance: {
            hex: stringToHex(ALLEGIANCE_VALUE),
            utf8: ALLEGIANCE_VALUE,
          },
          protocol: {
            hex: stringToHex(PROTOCOL_VALUE),
            utf8: PROTOCOL_VALUE,
          },
          tentacleId: {
            hex: stringToHex(TEST_TENTACLE_ID),
            utf8: TEST_TENTACLE_ID,
          },
        });

        // Reproduce the 54be471 failure shape without asking Anvil to materialize 20,000 empty
        // fork blocks. Test mode narrows only the explicitly incomplete recent refresh to 32
        // blocks; production remains fixed at 20,000 and the unit regression exercises that exact
        // boundary. Advancing 110 retained-history blocks also clears Anvil's 64-block finalized
        // lag, placing every registration/profile write outside this scaled recent window.
        await rpc("anvil_mine", [toHex(110)]);
        const recent = successfulResult(
          await call("mainnet-fork:recent-after-history-gap", {
            type: "discover",
            wallet,
            scope: "recent",
            tentacleId: TEST_TENTACLE_ID,
            xmtpInboxId: inboxId,
          }),
        );
        expect(recent).toMatchObject({
          complete: false,
          rangeComplete: true,
          scope: "recent",
          candidates: [],
        });

        const historical = successfulResult(
          await call("mainnet-fork:historical-rediscovery", {
            type: "discover",
            wallet,
            scope: "exhaustive",
            tentacleId: TEST_TENTACLE_ID,
            xmtpInboxId: inboxId,
          }),
        );
        expect(historical.complete).toBe(true);
        expect(historical.mintAuthorization).toBeUndefined();
        expect(historical.candidates).toEqual([
          expect.objectContaining({
            agentId,
            sameTentacle: true,
            authorized: true,
            walletVerified: true,
          }),
        ]);

        const refusedNonce = await currentNonce("mainnet-fork:refuse-second-register");
        const refused = await call("mainnet-fork:refuse-second-register", {
          type: "register",
          nonce: refusedNonce,
          mintAuthorization,
        });
        expect(refused).toMatchObject({
          ok: false,
          recoverable: false,
          code: "mint_authorization_reused",
        });
        expect(await currentNonce("mainnet-fork:after-refused-register")).toBe(refusedNonce);
      },
      180_000,
    );
  },
);
