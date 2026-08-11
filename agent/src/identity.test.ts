import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  canonicalDbEncryptionKey,
  canonicalWalletKey,
  loadAgentIdentity,
  loadOrCreateIdentity,
  parseEnvironment,
  walletAddressFromKey,
} from "./identity.js";

const temporaryDirectories: string[] = [];

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(path.join(os.tmpdir(), "cthuwu-agent-test-"));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map((directory) =>
      rm(directory, { recursive: true, force: true }),
    ),
  );
});

function deterministicRandom(): (size: number) => Uint8Array {
  let byte = 1;
  return (size) => new Uint8Array(size).fill(byte++);
}

describe("XMTP identity persistence", () => {
  it("atomically creates one identity and reuses it", async () => {
    const dataDir = await temporaryDirectory();
    const options = {
      dataDir,
      environment: "dev" as const,
      randomBytes: deterministicRandom(),
      now: () => new Date("2026-08-01T00:00:00.000Z"),
    };

    const first = await loadOrCreateIdentity(options);
    const second = await loadOrCreateIdentity({
      ...options,
      randomBytes: () => new Uint8Array(32).fill(9),
    });

    expect(second.walletKey).toBe(first.walletKey);
    expect(second.dbEncryptionKey).toBe(first.dbEncryptionKey);
    expect(first.walletKey).toMatch(/^0x[0-9a-f]{64}$/u);
    expect(first.walletAddress).toMatch(/^0x[0-9a-fA-F]{40}$/u);
    expect(second.walletAddress).toBe(first.walletAddress);
    expect(first.dbEncryptionKey).toMatch(/^0x[0-9a-f]{64}$/u);
    expect(JSON.parse(await readFile(first.identityPath, "utf8"))).toMatchObject({
      version: 1,
      environment: "dev",
      walletKey: first.walletKey,
      dbEncryptionKey: first.dbEncryptionKey,
    });
    if (process.platform !== "win32") {
      expect((await stat(first.identityPath)).mode & 0o777).toBe(0o600);
      expect((await stat(path.dirname(first.identityPath))).mode & 0o777).toBe(0o700);
      expect((await stat(first.dbDirectory)).mode & 0o777).toBe(0o700);
    }
  });

  it("makes concurrent creators converge on the same identity", async () => {
    const dataDir = await temporaryDirectory();
    let seed = 1;
    const create = () => {
      const random = deterministicRandom();
      const prefix = seed++;
      return loadOrCreateIdentity({
        dataDir,
        environment: "production",
        randomBytes: (size) => {
          const bytes = random(size);
          bytes[0] = prefix;
          return bytes;
        },
      });
    };

    const identities = await Promise.all([create(), create(), create(), create()]);
    expect(new Set(identities.map(({ walletKey }) => walletKey))).toHaveLength(1);
    expect(new Set(identities.map(({ dbEncryptionKey }) => dbEncryptionKey))).toHaveLength(
      1,
    );
  });

  it("persists supplied keys and refuses later key or environment changes", async () => {
    const dataDir = await temporaryDirectory();
    const walletKey = `0x${"11".repeat(32)}`;
    const dbEncryptionKey = "22".repeat(32);
    const identity = await loadOrCreateIdentity({
      dataDir,
      environment: "dev",
      walletKey,
      dbEncryptionKey,
    });

    expect(identity.walletKey).toBe(walletKey);
    expect(identity.dbEncryptionKey).toBe(`0x${dbEncryptionKey}`);
    await expect(
      loadOrCreateIdentity({
        dataDir,
        environment: "dev",
        walletKey: `0x${"33".repeat(32)}`,
      }),
    ).rejects.toThrow("does not match");
    await expect(
      loadOrCreateIdentity({ dataDir, environment: "production" }),
    ).rejects.toThrow("belongs to dev");
  });

  it("fails closed on a corrupt identity instead of rotating it", async () => {
    const dataDir = await temporaryDirectory();
    const stateDirectory = path.join(dataDir, "state");
    await mkdir(stateDirectory, { mode: 0o700 });
    const identityPath = path.join(stateDirectory, "xmtp-identity.json");
    await writeFile(identityPath, "not-json", { mode: 0o600 });

    await expect(
      loadOrCreateIdentity({ dataDir, environment: "dev" }),
    ).rejects.toThrow("not valid JSON");
    expect(await readFile(identityPath, "utf8")).toBe("not-json");
  });

  it("loads the agent identity without placing private keys in the environment", async () => {
    const dataDir = await temporaryDirectory();
    const environment: NodeJS.ProcessEnv = {
      UWUBOT_DATA_DIR: dataDir,
      UWUBOT_XMTP_ENV: "local",
    };

    const identity = await loadAgentIdentity(environment);
    expect(identity.environment).toBe("local");
    expect(environment.XMTP_ENV).toBeUndefined();
    expect(environment.XMTP_WALLET_KEY).toBeUndefined();
    expect(environment.XMTP_DB_ENCRYPTION_KEY).toBeUndefined();
    expect(environment.XMTP_DB_DIRECTORY).toBeUndefined();
  });

  it("rejects private key environment variables instead of importing them", async () => {
    const dataDir = await temporaryDirectory();
    await expect(
      loadAgentIdentity({
        UWUBOT_DATA_DIR: dataDir,
        UWUBOT_XMTP_ENV: "local",
        XMTP_WALLET_KEY: `0x${"44".repeat(32)}`,
      }),
    ).rejects.toThrow("never environment variables");
    await expect(
      loadAgentIdentity({
        UWUBOT_DATA_DIR: dataDir,
        UWUBOT_XMTP_ENV: "local",
        XMTP_DB_ENCRYPTION_KEY: "55".repeat(32),
      }),
    ).rejects.toThrow("never environment variables");
  });
});

describe("identity configuration validation", () => {
  it("accepts only explicit supported networks", () => {
    expect(parseEnvironment(undefined)).toBe("dev");
    expect(parseEnvironment("production")).toBe("production");
    expect(() => parseEnvironment("mainnet")).toThrow("must be one of");
  });

  it("validates and canonicalizes secret material without exposing it", () => {
    expect(canonicalWalletKey(`0x${"AA".repeat(32)}`)).toBe(`0x${"aa".repeat(32)}`);
    expect(canonicalDbEncryptionKey("BB".repeat(32))).toBe(`0x${"bb".repeat(32)}`);
    expect(() => canonicalWalletKey(`0x${"00".repeat(32)}`)).toThrow("not a valid");
    expect(() => canonicalDbEncryptionKey("abcd")).toThrow("exactly 64");
  });

  it("derives the canonical EVM wallet from the persisted XMTP key", () => {
    expect(walletAddressFromKey(`0x${"00".repeat(31)}01`)).toBe(
      "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
    );
  });
});
