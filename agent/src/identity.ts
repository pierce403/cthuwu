import { randomBytes, randomUUID } from "node:crypto";
import {
  chmod,
  link,
  lstat,
  mkdir,
  open,
  readFile,
  unlink,
} from "node:fs/promises";
import path from "node:path";
import { privateKeyToAccount } from "viem/accounts";

export const XMTP_ENVIRONMENTS = ["local", "dev", "production"] as const;

export type XmtpEnvironment = (typeof XMTP_ENVIRONMENTS)[number];

export type PersistentIdentity = {
  version: 1;
  environment: XmtpEnvironment;
  walletKey: `0x${string}`;
  dbEncryptionKey: `0x${string}`;
  createdAt: string;
};

export type LoadedIdentity = PersistentIdentity & {
  identityPath: string;
  dbDirectory: string;
  walletAddress: `0x${string}`;
};

type RandomBytes = (size: number) => Uint8Array;

export type IdentityOptions = {
  dataDir: string;
  environment: XmtpEnvironment;
  walletKey?: string;
  dbEncryptionKey?: string;
  dbDirectory?: string;
  randomBytes?: RandomBytes;
  now?: () => Date;
};

const PRIVATE_KEY_ORDER = BigInt(
  "0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
);
const HEX_32_BYTES = /^(?:0x)?[0-9a-fA-F]{64}$/u;

export function parseEnvironment(value: string | undefined): XmtpEnvironment {
  const environment = value ?? "dev";
  if (!XMTP_ENVIRONMENTS.includes(environment as XmtpEnvironment)) {
    throw new Error(
      `XMTP_ENV must be one of ${XMTP_ENVIRONMENTS.join(", ")}; received ${JSON.stringify(environment)}`,
    );
  }
  return environment as XmtpEnvironment;
}

export function canonicalWalletKey(value: string): `0x${string}` {
  if (!value.startsWith("0x") || !HEX_32_BYTES.test(value)) {
    throw new Error("XMTP_WALLET_KEY must be 0x followed by 64 hexadecimal characters");
  }
  const canonical = `0x${value.slice(2).toLowerCase()}` as const;
  const scalar = BigInt(canonical);
  if (scalar === 0n || scalar >= PRIVATE_KEY_ORDER) {
    throw new Error("XMTP_WALLET_KEY is not a valid secp256k1 private key");
  }
  return canonical;
}

export function canonicalDbEncryptionKey(value: string): `0x${string}` {
  if (!HEX_32_BYTES.test(value)) {
    throw new Error(
      "XMTP_DB_ENCRYPTION_KEY must contain exactly 64 hexadecimal characters",
    );
  }
  return `0x${value.replace(/^0x/u, "").toLowerCase()}`;
}

export function walletAddressFromKey(walletKey: string): `0x${string}` {
  return privateKeyToAccount(canonicalWalletKey(walletKey)).address;
}

function generateWalletKey(random: RandomBytes): `0x${string}` {
  for (let attempt = 0; attempt < 128; attempt += 1) {
    const candidate = `0x${Buffer.from(random(32)).toString("hex")}`;
    try {
      return canonicalWalletKey(candidate);
    } catch {
      // Retry in the astronomically unlikely invalid-scalar case.
    }
  }
  throw new Error("secure random source did not produce a valid wallet key");
}

function generateDbEncryptionKey(random: RandomBytes): `0x${string}` {
  return canonicalDbEncryptionKey(Buffer.from(random(32)).toString("hex"));
}

function parseIdentity(raw: string): PersistentIdentity {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch (error) {
    throw new Error("the persistent XMTP identity file is not valid JSON", {
      cause: error,
    });
  }

  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("the persistent XMTP identity file has an invalid shape");
  }

  const record = value as Record<string, unknown>;
  if (
    record.version !== 1 ||
    typeof record.environment !== "string" ||
    typeof record.walletKey !== "string" ||
    typeof record.dbEncryptionKey !== "string" ||
    typeof record.createdAt !== "string" ||
    !Number.isFinite(Date.parse(record.createdAt))
  ) {
    throw new Error("the persistent XMTP identity file is incomplete or unsupported");
  }

  return {
    version: 1,
    environment: parseEnvironment(record.environment),
    walletKey: canonicalWalletKey(record.walletKey),
    dbEncryptionKey: canonicalDbEncryptionKey(record.dbEncryptionKey),
    createdAt: record.createdAt,
  };
}

async function restrictMode(filePath: string, mode: number): Promise<void> {
  try {
    await chmod(filePath, mode);
  } catch (error) {
    if (process.platform !== "win32") {
      throw error;
    }
  }
}

async function ensurePrivateDirectory(directory: string): Promise<void> {
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const stat = await lstat(directory);
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    throw new Error(`refusing to use non-directory state path ${JSON.stringify(directory)}`);
  }
  await restrictMode(directory, 0o700);
}

async function readIdentity(identityPath: string): Promise<PersistentIdentity> {
  const stat = await lstat(identityPath);
  if (!stat.isFile() || stat.isSymbolicLink()) {
    throw new Error("the persistent XMTP identity path is not a regular file");
  }
  await restrictMode(identityPath, 0o600);
  return parseIdentity(await readFile(identityPath, "utf8"));
}

async function installAtomic(filePath: string, contents: string): Promise<boolean> {
  const temporaryPath = `${filePath}.${process.pid}.${randomUUID()}.tmp`;
  const handle = await open(temporaryPath, "wx", 0o600);
  try {
    await handle.writeFile(contents, { encoding: "utf8" });
    await handle.sync();
  } finally {
    await handle.close();
  }

  try {
    await link(temporaryPath, filePath);
    await restrictMode(filePath, 0o600);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EEXIST") {
      return false;
    }
    throw error;
  } finally {
    await unlink(temporaryPath).catch(() => undefined);
  }
}

function assertRequestedIdentity(
  identity: PersistentIdentity,
  options: IdentityOptions,
): void {
  if (identity.environment !== options.environment) {
    throw new Error(
      `XMTP identity belongs to ${identity.environment}, not ${options.environment}; use a different UWUBOT_DATA_DIR`,
    );
  }
  if (
    options.walletKey !== undefined &&
    identity.walletKey !== canonicalWalletKey(options.walletKey)
  ) {
    throw new Error("XMTP_WALLET_KEY does not match the persistent identity");
  }
  if (
    options.dbEncryptionKey !== undefined &&
    identity.dbEncryptionKey !== canonicalDbEncryptionKey(options.dbEncryptionKey)
  ) {
    throw new Error("XMTP_DB_ENCRYPTION_KEY does not match the persistent identity");
  }
}

async function ensureEnvironmentMarker(
  dbDirectory: string,
  environment: XmtpEnvironment,
): Promise<void> {
  await ensurePrivateDirectory(dbDirectory);
  const markerPath = path.join(dbDirectory, "cthuwu-environment");
  const installed = await installAtomic(markerPath, `${environment}\n`);
  if (!installed) {
    const markerStat = await lstat(markerPath);
    if (!markerStat.isFile() || markerStat.isSymbolicLink()) {
      throw new Error("the XMTP database environment marker is not a regular file");
    }
    const marker = (await readFile(markerPath, "utf8")).trim();
    if (marker !== environment) {
      throw new Error(
        `XMTP database directory belongs to ${JSON.stringify(marker)}, not ${environment}`,
      );
    }
    await restrictMode(markerPath, 0o600);
  }
}

export async function loadOrCreateIdentity(
  options: IdentityOptions,
): Promise<LoadedIdentity> {
  const stateDirectory = path.join(path.resolve(options.dataDir), "state");
  await ensurePrivateDirectory(stateDirectory);
  const identityPath = path.join(stateDirectory, "xmtp-identity.json");

  let identity: PersistentIdentity;
  try {
    identity = await readIdentity(identityPath);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
      throw error;
    }

    const random = options.randomBytes ?? randomBytes;
    const candidate: PersistentIdentity = {
      version: 1,
      environment: options.environment,
      walletKey:
        options.walletKey === undefined
          ? generateWalletKey(random)
          : canonicalWalletKey(options.walletKey),
      dbEncryptionKey:
        options.dbEncryptionKey === undefined
          ? generateDbEncryptionKey(random)
          : canonicalDbEncryptionKey(options.dbEncryptionKey),
      createdAt: (options.now ?? (() => new Date()))().toISOString(),
    };
    const installed = await installAtomic(
      identityPath,
      `${JSON.stringify(candidate, null, 2)}\n`,
    );
    identity = installed ? candidate : await readIdentity(identityPath);
  }

  assertRequestedIdentity(identity, options);
  const dbDirectory = path.resolve(
    options.dbDirectory ?? path.join(stateDirectory, "xmtp", options.environment),
  );
  await ensureEnvironmentMarker(dbDirectory, options.environment);
  return {
    ...identity,
    identityPath,
    dbDirectory,
    walletAddress: walletAddressFromKey(identity.walletKey),
  };
}

export async function loadAgentIdentity(
  environmentVariables: NodeJS.ProcessEnv = process.env,
): Promise<LoadedIdentity> {
  const xmtpEnvironment = environmentVariables.XMTP_ENV;
  const uwubotEnvironment = environmentVariables.UWUBOT_XMTP_ENV;
  if (
    xmtpEnvironment !== undefined &&
    uwubotEnvironment !== undefined &&
    xmtpEnvironment !== uwubotEnvironment
  ) {
    throw new Error("XMTP_ENV and UWUBOT_XMTP_ENV select different networks");
  }

  if (
    environmentVariables.XMTP_WALLET_KEY !== undefined ||
    environmentVariables.XMTP_DB_ENCRYPTION_KEY !== undefined
  ) {
    throw new Error(
      "persistent XMTP private keys must be loaded from the owner-only identity file, never environment variables",
    );
  }

  return loadOrCreateIdentity({
    dataDir: environmentVariables.UWUBOT_DATA_DIR ?? ".",
    environment: parseEnvironment(xmtpEnvironment ?? uwubotEnvironment),
    ...(environmentVariables.XMTP_DB_DIRECTORY === undefined
      ? {}
      : { dbDirectory: environmentVariables.XMTP_DB_DIRECTORY }),
  });
}
