import { beforeEach, describe, expect, it } from "vitest";
import { decryptIdentityBackup, encryptIdentityBackup } from "./identity-backup";
import { loadOrCreateIdentity } from "./identity";

describe("encrypted identity backup", () => {
  beforeEach(() => localStorage.clear());

  it("round-trips the same address and private key", async () => {
    const identity = loadOrCreateIdentity("dev");
    const backup = await encryptIdentityBackup(identity, "correct horse");
    expect(backup).not.toContain(identity.walletPrivateKey);
    const restored = await decryptIdentityBackup(backup, "correct horse", "dev");
    expect(restored).toEqual(identity);
  });

  it("rejects a wrong password or environment", async () => {
    const identity = loadOrCreateIdentity("dev");
    const backup = await encryptIdentityBackup(identity, "correct horse");
    await expect(decryptIdentityBackup(backup, "wrong password", "dev")).rejects.toThrow();
    await expect(decryptIdentityBackup(backup, "correct horse", "production")).rejects.toThrow(
      "environment",
    );
  });
});
