import { Wallet, getBytes } from "ethers";
import type { XmtpEnvironment } from "./config";

export interface StoredIdentity {
  version: 1;
  environment: XmtpEnvironment;
  createdAt: string;
  address: string;
  walletPrivateKey: string;
  compatibilityDbKey: string;
}

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
  const candidate = value as Partial<StoredIdentity>;
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
    wallet = new Wallet(candidate.walletPrivateKey);
  } catch {
    throw new IdentityStorageError("Identity contains an invalid private key");
  }
  if (wallet.address.toLowerCase() !== candidate.address.toLowerCase()) {
    throw new IdentityStorageError("Identity address does not match its private key");
  }
  try {
    if (getBytes(candidate.compatibilityDbKey).length !== 32) throw new Error();
  } catch {
    throw new IdentityStorageError("Identity contains an invalid compatibility key");
  }
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
