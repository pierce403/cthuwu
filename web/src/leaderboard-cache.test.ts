import { describe, expect, it } from "vitest";
import { isSnapshotStale, readLeaderboardCache, writeLeaderboardCache } from "./leaderboard-cache";
import { LEADERBOARD_CACHE_KEY, ZERO_ADDRESS } from "./leaderboard-types";
import { cachedSnapshot } from "./leaderboard-test-data";
import { readTentacleDisplayHint } from "./chat/tentacle-display";

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
      tentacleId: `tentacle_cache_${index + 1}`,
      registrationBlock: String(100 + index),
      profile: {
        ...original.profile,
        description: "P".repeat(512),
        sourceUri: `https://profiles.example/${"x".repeat(1_800)}`,
      },
      reputationCounters: { active: "1", sampledRevoked: "0" },
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

  it("repairs an old higher-alias representative and reserves ignored IDs globally", () => {
    const storage = new MemoryStorage();
    const snapshot = cachedSnapshot();
    const canonical = snapshot.rankedWallets[0].identities[0];
    snapshot.rankedWallets[0].identities.push({
      ...structuredClone(canonical),
      agentId: "2",
    });
    snapshot.rankedWallets[0].representativeAgentId = "2";
    expect(writeLeaderboardCache(storage, snapshot)).toBe(true);
    expect(readLeaderboardCache(storage)?.rankedWallets[0]).toMatchObject({
      representativeAgentId: "1",
      ignoredDuplicateAgentIds: ["2"],
      duplicateAgentAliases: [{ aliasAgentId: "2", canonicalAgentId: "1" }],
    });

    const tampered = JSON.parse(storage.getItem(LEADERBOARD_CACHE_KEY)!) as typeof snapshot;
    tampered.suspended.push({
      ...structuredClone(canonical),
      agentId: "2",
      agentWallet: ZERO_ADDRESS,
      rawBalance: "0",
    });
    storage.setItem(LEADERBOARD_CACHE_KEY, JSON.stringify(tampered));
    expect(readLeaderboardCache(storage)).toBeUndefined();
  });

  it("validates every raw duplicate before collapse so a hostile higher alias cannot hide", () => {
    const storage = new MemoryStorage();
    const snapshot = cachedSnapshot();
    const canonical = snapshot.rankedWallets[0].identities[0];
    snapshot.rankedWallets[0].identities.push({
      ...structuredClone(canonical),
      agentId: "2",
      agentWallet: "0x9999999999999999999999999999999999999999",
    });
    storage.setItem(LEADERBOARD_CACHE_KEY, JSON.stringify(snapshot));
    expect(readLeaderboardCache(storage)).toBeUndefined();
    expect(storage.getItem(LEADERBOARD_CACHE_KEY)).toBeNull();
  });

  it("counts active and suspended aliases once and keeps the lowest proven identity", () => {
    const activeFirst = cachedSnapshot();
    const active = activeFirst.rankedWallets[0]!.identities[0]!;
    activeFirst.suspended.push({
      ...structuredClone(active),
      agentId: "2",
      owner: active.agentWallet,
      agentWallet: ZERO_ADDRESS,
      rawBalance: "0",
    });
    const activeStorage = new MemoryStorage();
    expect(writeLeaderboardCache(activeStorage, activeFirst)).toBe(true);
    expect(readLeaderboardCache(activeStorage)).toMatchObject({
      rankedWallets: [{ representativeAgentId: "1" }],
      suspended: [],
      duplicateAgentAliases: [{ aliasAgentId: "2", canonicalAgentId: "1" }],
    });

    const suspendedFirst = cachedSnapshot();
    const higherActive = suspendedFirst.rankedWallets[0]!.identities[0]!;
    higherActive.agentId = "2";
    suspendedFirst.rankedWallets[0]!.representativeAgentId = "2";
    suspendedFirst.suspended.push({
      ...structuredClone(higherActive),
      agentId: "1",
      owner: higherActive.agentWallet,
      agentWallet: ZERO_ADDRESS,
      rawBalance: "0",
    });
    const suspendedStorage = new MemoryStorage();
    expect(writeLeaderboardCache(suspendedStorage, suspendedFirst)).toBe(true);
    expect(readLeaderboardCache(suspendedStorage)).toMatchObject({
      rankedWallets: [],
      suspended: [{ agentId: "1" }],
      duplicateAgentAliases: [{ aliasAgentId: "2", canonicalAgentId: "1" }],
    });
  });

  it("resolves an alias to its own component on a genuinely shared wallet", () => {
    const storage = new MemoryStorage();
    const snapshot = cachedSnapshot();
    const unrelated = snapshot.rankedWallets[0]!.identities[0]!;
    Object.assign(unrelated, { agentId: "10", tentacleId: "unrelated-tentacle" });
    snapshot.rankedWallets[0]!.representativeAgentId = "10";
    const component = {
      ...structuredClone(unrelated),
      agentId: "20",
      tentacleId: "component-tentacle",
      profile: { ...unrelated.profile, name: "Component canonical" },
    };
    snapshot.rankedWallets[0]!.identities.push(component, {
      ...structuredClone(component),
      agentId: "30",
      profile: { ...component.profile, name: "Component alias" },
    });
    expect(writeLeaderboardCache(storage, snapshot)).toBe(true);
    const cached = readLeaderboardCache(storage);
    expect(cached?.rankedWallets[0]).toMatchObject({
      representativeAgentId: "10",
      identities: [{ agentId: "10" }, { agentId: "20" }],
      ignoredDuplicateAgentIds: ["30"],
      duplicateAgentAliases: [{ aliasAgentId: "30", canonicalAgentId: "20" }],
    });
    expect(readTentacleDisplayHint("30", storage)?.name).toBe("Component canonical");
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
