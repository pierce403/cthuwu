import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { encodeFunctionData, getAddress, parseAbi, type Hex } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import type { LoadedIdentity } from "./identity.js";
import {
  ACOLYTE_NAME_SCHEME,
  ACOLYTE_NAME_TRAIT,
  BRANDING_CONTRACT,
  BRANDING_RUNTIME_CODE_HASH,
  CANONICAL_UWU,
  ERC8004_IDENTITY_REGISTRY,
  authorizeBrandingSignerNonce,
  authorizeSignerNonce,
  brandingConsentDigest,
  deriveAcolyteName,
  isAllowedBrandingTransaction,
  parseErc8004Request,
} from "./erc8004.js";

const SIGNER_KEY = `0x${"22".repeat(32)}` as Hex;
const ACOLYTE = getAddress("0x1111111111111111111111111111111111111111");
const REFERRER = getAddress("0x3333333333333333333333333333333333333333");
const BLOCK_HASH = `0x${"44".repeat(32)}` as Hex;
const SIGNATURE = `0x${"55".repeat(65)}` as Hex;

const brandingAbi = parseAbi([
  "function mintBranding((address acolyte,address minter,uint256 controllerAgentId,address referrer,uint256 initialDeclaredPrice,uint256 nonce,uint256 deadline) consent, bytes signature) returns (uint256 tokenId)",
  "function setCustomTrait(uint256 tokenId, string traitType, string value)",
]);
const erc20Abi = parseAbi([
  "function approve(address spender, uint256 amount) returns (bool)",
]);

function helperRequest(operation: Record<string, unknown>, actionId = "branding:test-1") {
  return { version: 1, actionId, operation };
}

function inspectionOperation(extra: Record<string, unknown> = {}) {
  return {
    type: "branding_inspect",
    acolyte: ACOLYTE,
    controllerAgentId: "61608",
    referrer: REFERRER,
    treasuryBalance: "10000000000000000000",
    priceBasisPoints: 1000,
    initialDeclaredPrice: "1000000000000000000",
    acolyteName: deriveAcolyteName(ACOLYTE),
    ...extra,
  };
}

function completionOperation(extra: Record<string, unknown> = {}) {
  return {
    type: "complete_branding",
    acolyte: ACOLYTE,
    minter: privateKeyToAccount(SIGNER_KEY).address,
    controllerAgentId: "61608",
    referrer: REFERRER,
    treasuryBalance: "10000000000000000000",
    priceBasisPoints: 1000,
    initialDeclaredPrice: "1000000000000000000",
    nonce: "0",
    deadline: "2000000000",
    offerBlockNumber: "50000000",
    offerBlockHash: BLOCK_HASH,
    signature: SIGNATURE,
    acolyteName: deriveAcolyteName(ACOLYTE),
    ...extra,
  };
}

async function journalIdentity(): Promise<{ identity: LoadedIdentity; directory: string }> {
  const directory = await mkdtemp(path.join(tmpdir(), "cthuwu-branding-signer-"));
  const account = privateKeyToAccount(SIGNER_KEY);
  return {
    identity: {
      version: 1,
      environment: "production",
      walletKey: SIGNER_KEY,
      dbEncryptionKey: `0x${"66".repeat(32)}`,
      createdAt: "2026-01-01T00:00:00.000Z",
      identityPath: path.join(directory, "xmtp-identity.json"),
      dbDirectory: path.join(directory, "xmtp"),
      walletAddress: account.address,
    },
    directory,
  };
}

describe("narrow Acolyte Branding executor", () => {
  it("pins the only production Branding deployment and canonical dependencies", () => {
    expect(BRANDING_CONTRACT).toBe("0xD8c36F13D79a505C7FBDc5F6467eA3cd75E896Da");
    expect(BRANDING_RUNTIME_CODE_HASH).toBe(
      "0x3a22b742a570dc2d030edf2bd82dceda1e068a297e2c36883f97b7b66ed4ef2d",
    );
    expect(ERC8004_IDENTITY_REGISTRY).toBe(
      "0x8004A169FB4a3325136EB29fA0ceB6D2e539a432",
    );
    expect(CANONICAL_UWU).toBe("0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07");
  });

  it("derives the exact frozen acolyte-v1 name used by the browser and NFT trait", () => {
    expect(ACOLYTE_NAME_SCHEME).toBe("acolyte-v1");
    expect(ACOLYTE_NAME_TRAIT).toBe("Acolyte Name");
    expect(deriveAcolyteName(ACOLYTE)).toBe("Ainsworth-Clavering of Ambercroft");
    expect(deriveAcolyteName("0x0000000000000000000000000000000000000001")).toBe(
      "Broughton-Arbuthnot of Marshborough",
    );
  });

  it("accepts only the exact typed inspection fields", () => {
    const parsed = parseErc8004Request(helperRequest(inspectionOperation()));
    expect(parsed.operation).toMatchObject({
      type: "branding_inspect",
      acolyte: ACOLYTE,
      controllerAgentId: "61608",
      referrer: REFERRER,
      treasuryBalance: "10000000000000000000",
      priceBasisPoints: 1000,
      acolyteName: "Ainsworth-Clavering of Ambercroft",
    });
    expect(() =>
      parseErc8004Request(helperRequest(inspectionOperation({ contract: BRANDING_CONTRACT }))),
    ).toThrow("missing or unknown fields");
    expect(() =>
      parseErc8004Request(helperRequest(inspectionOperation({ acolyteName: "Lord Spoof" }))),
    ).toThrow("deterministic acolyte-v1");
    expect(() =>
      parseErc8004Request(helperRequest(inspectionOperation({ initialDeclaredPrice: "0" }))),
    ).toThrow("required uint256 range");
    expect(() =>
      parseErc8004Request(helperRequest(inspectionOperation({ treasuryBalance: "999" }))),
    ).toThrow("exact treasury balance");
    expect(() =>
      parseErc8004Request(helperRequest(inspectionOperation({ treasuryBalance: "00" }))),
    ).toThrow("canonical uint256 decimal");
    expect(() =>
      parseErc8004Request(helperRequest(inspectionOperation({
        treasuryBalance: ((1n << 256n) - 1n).toString(),
        priceBasisPoints: 2000,
      }))),
    ).toThrow("exceeds uint256");
    for (const priceBasisPoints of [499, 2001, 1000.5, "1000"]) {
      expect(() =>
        parseErc8004Request(helperRequest(inspectionOperation({ priceBasisPoints }))),
      ).toThrow("integer between 500 and 2000");
    }
  });

  it("binds completion to exact consent, minter, offer block, and bounded signature", () => {
    const parsed = parseErc8004Request(helperRequest(completionOperation()));
    expect(parsed.operation).toMatchObject({
      type: "complete_branding",
      offerBlockNumber: "50000000",
      offerBlockHash: BLOCK_HASH,
      signature: SIGNATURE,
    });
    for (const changed of [
      { chainId: 1 },
      { contract: BRANDING_CONTRACT },
      { to: ACOLYTE },
      { data: "0xdeadbeef" },
      { privateKey: SIGNER_KEY },
    ]) {
      expect(() =>
        parseErc8004Request(helperRequest(completionOperation(changed))),
      ).toThrow("missing or unknown fields");
    }
    expect(() =>
      parseErc8004Request(helperRequest(completionOperation({ offerBlockHash: "0x12" }))),
    ).toThrow("32 bytes");
    expect(() =>
      parseErc8004Request(
        helperRequest(completionOperation({ signature: `0x${"11".repeat(8193)}` })),
      ),
    ).toThrow("bounded hexadecimal");
  });

  it("computes the exact EIP-712 digest and binds every signed consent field", () => {
    const parsed = parseErc8004Request(helperRequest(completionOperation()));
    if (parsed.operation.type !== "complete_branding") throw new Error("unexpected operation");
    const digest = brandingConsentDigest(parsed.operation);
    expect(digest).toMatch(/^0x[0-9a-f]{64}$/u);
    const changed = parseErc8004Request(helperRequest(completionOperation({
      treasuryBalance: "10000000000000000010",
      initialDeclaredPrice: "1000000000000000001",
    })));
    if (changed.operation.type !== "complete_branding") throw new Error("unexpected operation");
    expect(brandingConsentDigest(changed.operation)).not.toBe(digest);
  });

  it("rejects zero roles and noncanonical uint256 consent fields", () => {
    for (const changed of [
      { acolyte: "0x0000000000000000000000000000000000000000" },
      { referrer: "0x0000000000000000000000000000000000000000" },
      { controllerAgentId: "01" },
      { initialDeclaredPrice: "00" },
      { nonce: "01" },
      { deadline: "0" },
      { offerBlockNumber: "01" },
    ]) {
      expect(() =>
        parseErc8004Request(helperRequest(completionOperation(changed))),
      ).toThrow();
    }
  });

  it("has only approve, mint, and exact name-trait transaction selectors", () => {
    const approve = encodeFunctionData({
      abi: erc20Abi,
      functionName: "approve",
      args: [BRANDING_CONTRACT, 100n],
    });
    const mint = encodeFunctionData({
      abi: brandingAbi,
      functionName: "mintBranding",
      args: [{
        acolyte: ACOLYTE,
        minter: privateKeyToAccount(SIGNER_KEY).address,
        controllerAgentId: 61608n,
        referrer: REFERRER,
        initialDeclaredPrice: 1000n,
        nonce: 0n,
        deadline: 2_000_000_000n,
      }, SIGNATURE],
    });
    const trait = encodeFunctionData({
      abi: brandingAbi,
      functionName: "setCustomTrait",
      args: [BigInt(ACOLYTE), ACOLYTE_NAME_TRAIT, deriveAcolyteName(ACOLYTE)],
    });
    expect(isAllowedBrandingTransaction("approve", CANONICAL_UWU, approve)).toBe(true);
    expect(isAllowedBrandingTransaction("mint", BRANDING_CONTRACT, mint)).toBe(true);
    expect(isAllowedBrandingTransaction("name_trait", BRANDING_CONTRACT, trait)).toBe(true);
    expect(isAllowedBrandingTransaction("approve", ACOLYTE, approve)).toBe(false);
    expect(isAllowedBrandingTransaction("mint", CANONICAL_UWU, mint)).toBe(false);
    expect(isAllowedBrandingTransaction("name_trait", BRANDING_CONTRACT, approve)).toBe(false);
  });

  it("allocates a Branding nonce before broadcast and permits only exact phase replay", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      const data = "0x1234" as Hex;
      expect(
        await authorizeBrandingSignerNonce(
          identity,
          "branding:complete-1",
          "mint",
          BRANDING_CONTRACT,
          data,
          7,
          7,
        ),
      ).toEqual({ nonce: 7, existed: false });
      const journal = await readFile(
        path.join(directory, "erc8004-signer-nonce-v1-7.json"),
        "utf8",
      );
      expect(journal).toContain('"phase":"mint"');
      expect(journal).toContain('"destination":"0xD8c36F13D79a505C7FBDc5F6467eA3cd75E896Da"');
      expect(journal).not.toContain(SIGNER_KEY);
      expect(
        await authorizeBrandingSignerNonce(
          identity,
          "branding:complete-1",
          "mint",
          BRANDING_CONTRACT,
          data,
          8,
          7,
        ),
      ).toEqual({ nonce: 7, existed: true });
      await expect(
        authorizeBrandingSignerNonce(
          identity,
          "branding:complete-1",
          "name_trait",
          BRANDING_CONTRACT,
          data,
          8,
          7,
        ),
      ).rejects.toThrow("reserved by another exact action");
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("returns signer_busy for an unrelated pending wallet transaction", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      await expect(
        authorizeBrandingSignerNonce(
          identity,
          "branding:complete-1",
          "mint",
          BRANDING_CONTRACT,
          "0x1234",
          8,
          7,
        ),
      ).rejects.toMatchObject({ code: "signer_busy" });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("binds a reserved nonce to exact action ID, destination, and calldata hash", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      await authorizeBrandingSignerNonce(
        identity,
        "branding:complete-1",
        "mint",
        BRANDING_CONTRACT,
        "0x1234",
        7,
        7,
      );
      for (const changed of [
        { actionId: "branding:complete-2", to: BRANDING_CONTRACT, data: "0x1234" as Hex },
        { actionId: "branding:complete-1", to: CANONICAL_UWU, data: "0x1234" as Hex },
        { actionId: "branding:complete-1", to: BRANDING_CONTRACT, data: "0x5678" as Hex },
      ]) {
        await expect(authorizeBrandingSignerNonce(
          identity,
          changed.actionId,
          "mint",
          changed.to,
          changed.data,
          8,
          7,
        )).rejects.toMatchObject({ code: "signer_busy" });
      }
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("refuses Branding signing outside the persistent production identity", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      await expect(authorizeBrandingSignerNonce(
        { ...identity, environment: "dev" },
        "branding:complete-1",
        "mint",
        BRANDING_CONTRACT,
        "0x1234",
        7,
        7,
      )).rejects.toThrow("production identity");
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("shares the durable nonce namespace with ERC-8004 registration writes", async () => {
    const { identity, directory } = await journalIdentity();
    try {
      const registryRequest = parseErc8004Request(
        helperRequest({ type: "register", nonce: "7" }, "registration:shared"),
      );
      if (registryRequest.operation.type !== "register") throw new Error("unexpected operation");
      await authorizeSignerNonce(
        identity,
        registryRequest.actionId,
        registryRequest.operation,
        7,
        7,
      );
      await expect(
        authorizeBrandingSignerNonce(
          identity,
          "branding:complete-1",
          "mint",
          BRANDING_CONTRACT,
          "0x1234",
          8,
          7,
        ),
      ).rejects.toMatchObject({ code: "signer_busy" });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("never exposes a generic transaction request", () => {
    expect(() =>
      parseErc8004Request(helperRequest({
        type: "send_transaction",
        to: ACOLYTE,
        value: "0",
        data: "0x",
      })),
    ).toThrow("unsupported ERC-8004 operation");
  });
});
