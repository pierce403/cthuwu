import type { XmtpEnvironment } from "./config";
import { validateIdentity, type LocalIdentity } from "./identity";

const ITERATIONS = 210_000;

interface EncryptedBackup {
  format: "cthuwu-identity-backup";
  version: 1;
  environment: XmtpEnvironment;
  address: string;
  kdf: { name: "PBKDF2-SHA256"; iterations: number; salt: string };
  cipher: { name: "AES-256-GCM"; iv: string; ciphertext: string };
}

export async function encryptIdentityBackup(
  identity: LocalIdentity,
  passphrase: string,
): Promise<string> {
  if (passphrase.length < 8) throw new Error("Use a backup passphrase of at least 8 characters");
  const salt = crypto.getRandomValues(new Uint8Array(16));
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const key = await deriveKey(passphrase, salt, ITERATIONS);
  const plaintext = new TextEncoder().encode(JSON.stringify(identity));
  const ciphertext = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: toArrayBuffer(iv) },
    key,
    toArrayBuffer(plaintext),
  );
  const backup: EncryptedBackup = {
    format: "cthuwu-identity-backup",
    version: 1,
    environment: identity.environment,
    address: identity.address,
    kdf: {
      name: "PBKDF2-SHA256",
      iterations: ITERATIONS,
      salt: base64(salt),
    },
    cipher: {
      name: "AES-256-GCM",
      iv: base64(iv),
      ciphertext: base64(new Uint8Array(ciphertext)),
    },
  };
  return JSON.stringify(backup, null, 2);
}

export async function decryptIdentityBackup(
  serialized: string,
  passphrase: string,
  expectedEnvironment: XmtpEnvironment,
): Promise<LocalIdentity> {
  if (serialized.length > 100_000) throw new Error("Identity backup is unexpectedly large");
  let backup: EncryptedBackup;
  try {
    backup = JSON.parse(serialized) as EncryptedBackup;
  } catch {
    throw new Error("Identity backup is not valid JSON");
  }
  if (
    backup.format !== "cthuwu-identity-backup" ||
    backup.version !== 1 ||
    backup.environment !== expectedEnvironment ||
    backup.kdf?.name !== "PBKDF2-SHA256" ||
    backup.cipher?.name !== "AES-256-GCM" ||
    backup.kdf.iterations < 100_000 ||
    backup.kdf.iterations > 1_000_000
  ) {
    throw new Error("Identity backup format or environment does not match");
  }
  try {
    const salt = unbase64(backup.kdf.salt);
    const iv = unbase64(backup.cipher.iv);
    if (salt.length !== 16 || iv.length !== 12) throw new Error();
    const key = await deriveKey(passphrase, salt, backup.kdf.iterations);
    const plaintext = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: toArrayBuffer(iv) },
      key,
      toArrayBuffer(unbase64(backup.cipher.ciphertext)),
    );
    const identity = JSON.parse(new TextDecoder().decode(plaintext)) as unknown;
    validateIdentity(identity, expectedEnvironment);
    if (identity.version !== 1) throw new Error();
    if (identity.address.toLowerCase() !== backup.address.toLowerCase()) throw new Error();
    return identity;
  } catch {
    throw new Error("Could not decrypt or validate the identity backup");
  }
}

async function deriveKey(
  passphrase: string,
  salt: Uint8Array,
  iterations: number,
): Promise<CryptoKey> {
  const material = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(passphrase),
    "PBKDF2",
    false,
    ["deriveKey"],
  );
  return crypto.subtle.deriveKey(
    { name: "PBKDF2", hash: "SHA-256", salt: toArrayBuffer(salt), iterations },
    material,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
}

function base64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function unbase64(value: string): Uint8Array {
  const binary = atob(value);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  return Uint8Array.from(bytes).buffer;
}
