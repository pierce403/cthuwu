import { beforeEach, describe, expect, it } from "vitest";
import {
  IdentityStorageError,
  identityKey,
  loadOrCreateIdentity,
  resetIdentity,
} from "./identity";

describe("browser identity", () => {
  beforeEach(() => localStorage.clear());

  it("creates once and returns byte-identical keys after reload", () => {
    const first = loadOrCreateIdentity("dev");
    const second = loadOrCreateIdentity("dev");
    expect(second).toEqual(first);
    expect(first.walletPrivateKey).toMatch(/^0x[0-9a-f]{64}$/);
    expect(first.compatibilityDbKey).toMatch(/^0x[0-9a-f]{64}$/);
  });

  it("isolates environments", () => {
    const dev = loadOrCreateIdentity("dev");
    const production = loadOrCreateIdentity("production");
    expect(dev.address).not.toBe(production.address);
  });

  it("migrates complete legacy keys without rotating the wallet", () => {
    const original = loadOrCreateIdentity("dev");
    localStorage.removeItem(identityKey("dev"));
    localStorage.setItem("cthuwu:dev:wallet-key", original.walletPrivateKey);
    localStorage.setItem("cthuwu:dev:db-key", original.compatibilityDbKey);
    const migrated = loadOrCreateIdentity("dev");
    expect(migrated.address).toBe(original.address);
    expect(localStorage.getItem("cthuwu:dev:wallet-key")).toBeNull();
  });

  it("fails closed on partial legacy or corrupt canonical state", () => {
    localStorage.setItem("cthuwu:dev:wallet-key", "0x00");
    expect(() => loadOrCreateIdentity("dev")).toThrow(IdentityStorageError);
    localStorage.clear();
    localStorage.setItem(identityKey("dev"), "{nope");
    expect(() => loadOrCreateIdentity("dev")).toThrow(IdentityStorageError);
  });

  it("reset touches only Cthuwu keys for the selected environment", () => {
    loadOrCreateIdentity("dev");
    loadOrCreateIdentity("production");
    localStorage.setItem("unrelated", "keep");
    resetIdentity("dev");
    expect(localStorage.getItem(identityKey("dev"))).toBeNull();
    expect(localStorage.getItem(identityKey("production"))).not.toBeNull();
    expect(localStorage.getItem("unrelated")).toBe("keep");
  });
});
