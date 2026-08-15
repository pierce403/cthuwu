import { beforeEach, describe, expect, it } from "vitest";
import {
  IdentityStorageError,
  createExternalIdentity,
  isLocalIdentity,
  identityKey,
  loadOrCreateIdentity,
  persistIdentity,
  resetIdentity,
} from "./identity";

describe("browser identity", () => {
  beforeEach(() => localStorage.clear());

  it("creates once and returns byte-identical keys after reload", () => {
    const first = loadOrCreateIdentity("dev");
    const second = loadOrCreateIdentity("dev");
    expect(second).toEqual(first);
    expect(isLocalIdentity(first)).toBe(true);
    if (!isLocalIdentity(first)) throw new Error("expected local identity");
    expect(first.walletPrivateKey).toMatch(/^0x[0-9a-f]{64}$/);
    expect(first.compatibilityDbKey).toMatch(/^0x[0-9a-f]{64}$/);
  });

  it("isolates environments", () => {
    const dev = loadOrCreateIdentity("dev");
    const production = loadOrCreateIdentity("production");
    expect(dev.address).not.toBe(production.address);
  });

  it("persists an external wallet without private-key material", () => {
    const external = createExternalIdentity(
      "production",
      "0x2222222222222222222222222222222222222222",
      "walletConnect",
      8453,
      "EOA",
      () => new Date("2026-08-15T00:00:00.000Z"),
    );
    persistIdentity(external);
    expect(loadOrCreateIdentity("production")).toEqual(external);
    const stored = localStorage.getItem(identityKey("production")) ?? "";
    expect(stored).not.toContain("privateKey");
  });

  it("fails closed on malformed external connector state", () => {
    const external = createExternalIdentity(
      "production",
      "0x2222222222222222222222222222222222222222",
      "injected",
      8453,
      "SCW",
    );
    localStorage.setItem(identityKey("production"), JSON.stringify({ ...external, chainId: 0 }));
    expect(() => loadOrCreateIdentity("production")).toThrow(IdentityStorageError);
  });

  it("migrates complete legacy keys without rotating the wallet", () => {
    const original = loadOrCreateIdentity("dev");
    if (!isLocalIdentity(original)) throw new Error("expected local identity");
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
