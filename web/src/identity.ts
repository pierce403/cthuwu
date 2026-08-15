import { Wallet, getAddress, getBytes } from "ethers";
import type { XmtpEnvironment } from "./config";

export interface LocalIdentity {
  version: 1;
  environment: XmtpEnvironment;
  createdAt: string;
  address: string;
  walletPrivateKey: string;
  compatibilityDbKey: string;
}

export type ExternalWalletConnector = "injected" | "walletConnect";

export interface ExternalIdentity {
  version: 2;
  environment: XmtpEnvironment;
  createdAt: string;
  address: string;
  source: "external";
  connector: ExternalWalletConnector;
  chainId: number;
  signerType: "EOA" | "SCW";
  compatibilityDbKey: string;
}

export type StoredIdentity = LocalIdentity | ExternalIdentity;

export class IdentityStorageError extends Error {}

export function identityKey(environment: XmtpEnvironment): string {
  return `cthuwu:${environment}:identity:v1`;
}

export function loadOrCreateIdentity(
  environment: XmtpEnvironment,
  storage: Storage = localStorage,
  now: () => Date = () => new Date(),
): StoredIdentity {
  const canonical = storage.getItem(identityKey(environment));
  if (canonical) return parseIdentity(canonical, environment);

  const migrated = migrateLegacyIdentity(environment, storage, now);
  if (migrated) return migrated;

  const wallet = new Wallet(randomHex32());
  const identity = makeIdentity(environment, wallet, randomHex32(), now());
  persistIdentity(identity, storage);
  return identity;
}

export function createExternalIdentity(
  environment: XmtpEnvironment,
  address: string,
  connector: ExternalWalletConnector,
  chainId: number,
  signerType: "EOA" | "SCW",
  now: () => Date = () => new Date(),
): ExternalIdentity {
  const identity: ExternalIdentity = {
    version: 2,
    environment,
    createdAt: now().toISOString(),
    address: canonicalAddress(address),
    source: "external",
    connector,
    chainId,
    signerType,
    compatibilityDbKey: randomHex32(),
  };
  validateIdentity(identity, environment);
  return identity;
}

export function isLocalIdentity(identity: StoredIdentity): identity is LocalIdentity {
  return identity.version === 1;
}

export function persistIdentity(identity: StoredIdentity, storage: Storage = localStorage): void {
  validateIdentity(identity, identity.environment);
  const key = identityKey(identity.environment);
  const serialized = JSON.stringify(identity);
  try {
    storage.setItem(key, serialized);
    if (storage.getItem(key) !== serialized) throw new Error("identity read-back mismatch");
  } catch (error) {
    throw new IdentityStorageError(
      `Could not persist the browser identity: ${error instanceof Error ? error.message : "storage failed"}`,
    );
  }
}

export function resetIdentity(environment: XmtpEnvironment, storage: Storage = localStorage): void {
  storage.removeItem(identityKey(environment));
  storage.removeItem(`cthuwu:${environment}:wallet-key`);
  storage.removeItem(`cthuwu:${environment}:db-key`);
}

export function parseIdentity(serialized: string, environment: XmtpEnvironment): StoredIdentity {
  let value: unknown;
  try {
    value = JSON.parse(serialized);
  } catch {
    throw new IdentityStorageError("The stored browser identity is corrupt; import a backup or reset it");
  }
  validateIdentity(value, environment);
  return value;
}

export function validateIdentity(
  value: unknown,
  environment: XmtpEnvironment,
): asserts value is StoredIdentity {
  if (!value || typeof value !== "object") throw new IdentityStorageError("Invalid identity backup");
  const candidate = value as Record<string, unknown>;
  if (candidate.version === 2) {
    if (
      candidate.environment !== environment ||
      typeof candidate.createdAt !== "string" ||
      typeof candidate.address !== "string" ||
      candidate.source !== "external" ||
      (candidate.connector !== "injected" && candidate.connector !== "walletConnect") ||
      typeof candidate.chainId !== "number" ||
      !Number.isSafeInteger(candidate.chainId) ||
      ![1, 8453, 84532].includes(candidate.chainId) ||
      (candidate.signerType !== "EOA" && candidate.signerType !== "SCW") ||
      typeof candidate.compatibilityDbKey !== "string" ||
      candidate.walletPrivateKey !== undefined
    ) {
      throw new IdentityStorageError("External identity schema or environment does not match");
    }
    canonicalAddress(candidate.address);
    validateCompatibilityKey(candidate.compatibilityDbKey);
    return;
  }
  if (
    candidate.version !== 1 ||
    candidate.environment !== environment ||
    typeof candidate.createdAt !== "string" ||
    typeof candidate.address !== "string" ||
    typeof candidate.walletPrivateKey !== "string" ||
    typeof candidate.compatibilityDbKey !== "string"
  ) {
    throw new IdentityStorageError("Identity backup schema or environment does not match");
  }
  let wallet: Wallet;
  try {
    wallet = new Wallet(candidate.walletPrivateKey as string);
  } catch {
    throw new IdentityStorageError("Identity contains an invalid private key");
  }
  if (wallet.address.toLowerCase() !== (candidate.address as string).toLowerCase()) {
    throw new IdentityStorageError("Identity address does not match its private key");
  }
  validateCompatibilityKey(candidate.compatibilityDbKey as string);
}

function migrateLegacyIdentity(
  environment: XmtpEnvironment,
  storage: Storage,
  now: () => Date,
): StoredIdentity | undefined {
  const walletKeyName = `cthuwu:${environment}:wallet-key`;
  const databaseKeyName = `cthuwu:${environment}:db-key`;
  const walletKey = storage.getItem(walletKeyName);
  const databaseKey = storage.getItem(databaseKeyName);
  if (!walletKey && !databaseKey) return undefined;
  if (!walletKey || !databaseKey) {
    throw new IdentityStorageError(
      "The legacy browser identity is incomplete; import a backup or explicitly reset it",
    );
  }
  let wallet: Wallet;
  try {
    wallet = new Wallet(walletKey);
  } catch {
    throw new IdentityStorageError("The legacy browser identity has an invalid private key");
  }
  const identity = makeIdentity(environment, wallet, databaseKey, now());
  persistIdentity(identity, storage);
  storage.removeItem(walletKeyName);
  storage.removeItem(databaseKeyName);
  return identity;
}

function makeIdentity(
  environment: XmtpEnvironment,
  wallet: Wallet,
  compatibilityDbKey: string,
  createdAt: Date,
): StoredIdentity {
  const identity: StoredIdentity = {
    version: 1,
    environment,
    createdAt: createdAt.toISOString(),
    address: wallet.address.toLowerCase(),
    walletPrivateKey: wallet.privateKey,
    compatibilityDbKey,
  };
  validateIdentity(identity, environment);
  return identity;
}

function randomHex32(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return `0x${Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

function canonicalAddress(address: string): string {
  try {
    const normalized = getAddress(address);
    if (/^0x0{40}$/i.test(normalized)) throw new Error();
    return normalized.toLowerCase();
  } catch {
    throw new IdentityStorageError("Identity contains an invalid Ethereum address");
  }
}

function validateCompatibilityKey(key: string): void {
  try {
    if (getBytes(key).length !== 32) throw new Error();
  } catch {
    throw new IdentityStorageError("Identity contains an invalid compatibility key");
  }
}
