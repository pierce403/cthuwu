import { describe, expect, it } from "vitest";
import { isSnapshotStale, readLeaderboardCache, writeLeaderboardCache } from "./leaderboard-cache";
import { LEADERBOARD_CACHE_KEY } from "./leaderboard-types";
import { cachedSnapshot } from "./leaderboard-test-data";

describe("leaderboard localStorage cache", () => {
  it("round-trips a validated complete Base snapshot", () => {
    const storage = new MemoryStorage();
    expect(writeLeaderboardCache(storage, cachedSnapshot())).toBe(true);
    expect(readLeaderboardCache(storage)).toEqual(cachedSnapshot());
  });

  it("never persists an embedded registration document", () => {
    const storage = new MemoryStorage();
    const snapshot = cachedSnapshot();
    snapshot.rankedWallets[0].identities[0].agentUri =
      "data:application/json;base64,eyJuYW1lIjoicHVibGljIn0=";
    expect(writeLeaderboardCache(storage, snapshot)).toBe(true);
    expect(readLeaderboardCache(storage)?.rankedWallets[0].identities[0].agentUri).toBe("");
    expect(storage.getItem(LEADERBOARD_CACHE_KEY)).not.toContain("eyJuYW1lIjoicHVibGljIn0=");
  });

  it("discards only the leaderboard key when it is corrupt", () => {
    const storage = new MemoryStorage();
    storage.setItem(LEADERBOARD_CACHE_KEY, "{bad json");
    storage.setItem("cthuwu.identity.production.v2", "leave-me-alone");
    expect(readLeaderboardCache(storage)).toBeUndefined();
    expect(storage.getItem(LEADERBOARD_CACHE_KEY)).toBeNull();
    expect(storage.getItem("cthuwu.identity.production.v2")).toBe("leave-me-alone");
  });

  it("keeps the previous snapshot when quota blocks both replacement attempts", () => {
    const storage = new MemoryStorage();
    const original = cachedSnapshot("Original");
    expect(writeLeaderboardCache(storage, original)).toBe(true);
    storage.failWrites = true;
    expect(writeLeaderboardCache(storage, cachedSnapshot("Replacement"))).toBe(false);
    storage.failWrites = false;
    expect(readLeaderboardCache(storage)?.rankedWallets[0].identities[0].profile.name).toBe(
      "Original",
    );
  });

  it("falls back to a compact validated snapshot after one quota failure", () => {
    const storage = new MemoryStorage();
    const snapshot = cachedSnapshot();
    snapshot.rankedWallets[0].identities[0].agentUri = `data:${"A".repeat(8_000)}`;
    snapshot.rankedWallets[0].identities[0].profile.description = "Public profile";
    storage.failNextWrites = 1;
    expect(writeLeaderboardCache(storage, snapshot)).toBe(true);
    expect(readLeaderboardCache(storage)?.rankedWallets[0].identities[0]).toMatchObject({
      profile: { name: "Cache Tentacle", sourceUri: "cached" },
      agentUri: "",
      reputation: [],
    });
  });

  it("never writes an unreadable oversized record when localStorage would accept it", () => {
    const storage = new MemoryStorage();
    const snapshot = cachedSnapshot();
    const original = snapshot.rankedWallets[0].identities[0];
    snapshot.rankedWallets[0].identities = Array.from({ length: 1_000 }, (_, index) => ({
      ...structuredClone(original),
      agentId: String(index + 1),
      registrationBlock: String(100 + index),
      profile: {
        ...original.profile,
        description: "P".repeat(512),
        sourceUri: `https://profiles.example/${"x".repeat(1_800)}`,
      },
      reputationCounters: { total: "1", active: "1", revoked: "0" },
      reputation: [
        {
          id: `signal-${index}`,
          clientAddress: "0x2222222222222222222222222222222222222222",
          value: "1",
          valueDecimals: 0,
          createdAt: "1700000000",
          revoked: false,
          provenance: "p".repeat(512),
        },
      ],
    }));
    snapshot.rankedWallets[0].representativeAgentId = "1";

    expect(writeLeaderboardCache(storage, snapshot)).toBe(true);
    const stored = storage.getItem(LEADERBOARD_CACHE_KEY);
    expect(stored).not.toBeNull();
    expect(new TextEncoder().encode(stored!).length).toBeLessThanOrEqual(2 * 1024 * 1024);
    expect(readLeaderboardCache(storage)?.rankedWallets[0].identities).toHaveLength(1_000);
    expect(readLeaderboardCache(storage)?.rankedWallets[0].identities[0].profile.sourceUri).toBe(
      "cached",
    );
  });

  it("rejects a ranked zero-address wallet in a tampered cache", () => {
    const storage = new MemoryStorage();
    const snapshot = cachedSnapshot();
    snapshot.rankedWallets[0].wallet = "0x0000000000000000000000000000000000000000";
    snapshot.rankedWallets[0].identities[0].agentWallet =
      "0x0000000000000000000000000000000000000000";
    storage.setItem(LEADERBOARD_CACHE_KEY, JSON.stringify(snapshot));

    expect(readLeaderboardCache(storage)).toBeUndefined();
    expect(storage.getItem(LEADERBOARD_CACHE_KEY)).toBeNull();
  });

  it("rejects partial, indexing-error, wrong-chain, and malformed snapshots", () => {
    for (const mutation of [
      { paginationComplete: false },
      { hasIndexingErrors: true },
      { chainId: 1 },
      { identityRegistry: "0x0000000000000000000000000000000000000000" },
    ]) {
      const storage = new MemoryStorage();
      storage.setItem(LEADERBOARD_CACHE_KEY, JSON.stringify({ ...cachedSnapshot(), ...mutation }));
      expect(readLeaderboardCache(storage)).toBeUndefined();
    }
  });

  it("marks snapshots stale only after the configured freshness interval", () => {
    const snapshot = cachedSnapshot();
    const fetchedAt = Date.parse(snapshot.fetchedAt);
    expect(isSnapshotStale(snapshot, 15 * 60_000, fetchedAt + 14 * 60_000)).toBe(false);
    expect(isSnapshotStale(snapshot, 15 * 60_000, fetchedAt + 16 * 60_000)).toBe(true);
  });
});

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();
  failWrites = false;
  failNextWrites = 0;
  get length(): number {
    return this.values.size;
  }
  clear(): void {
    this.values.clear();
  }
  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }
  key(index: number): string | null {
    return [...this.values.keys()][index] ?? null;
  }
  removeItem(key: string): void {
    this.values.delete(key);
  }
  setItem(key: string, value: string): void {
    if (this.failWrites || this.failNextWrites > 0) {
      if (this.failNextWrites > 0) this.failNextWrites -= 1;
      throw new DOMException("quota", "QuotaExceededError");
    }
    this.values.set(key, value);
  }
}
