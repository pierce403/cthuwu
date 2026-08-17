import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppConfig } from "../config";
import { writeLeaderboardCache } from "../leaderboard-cache";
import { cachedSnapshot } from "../leaderboard-test-data";
import { PROTOCOL_V1_HEX, type LeaderboardSnapshot, type TentacleIdentity } from "../leaderboard-types";
import { loadLivenessCandidates } from "./liveness";

const config: AppConfig = {
  environment: "production",
  botAddress: "0x9999999999999999999999999999999999999999",
  baseRpcEndpoint: "https://rpc.example/",
  assignmentRefreshMs: 600_000,
};
const NOW = Date.parse("2026-08-11T12:05:00.000Z");

function address(index: number): string {
  return `0x${index.toString(16).padStart(40, "0")}`;
}

function inbox(index: number): string {
  return index.toString(16).repeat(64);
}

function directory(count = 7): LeaderboardSnapshot {
  const base = cachedSnapshot();
  const template = base.rankedWallets[0]!.identities[0]!;
  const rankedWallets = Array.from({ length: count }, (_, offset) => {
    const index = offset + 1;
    const wallet = address(index);
    const rawBalance = String((count - offset) * 1_000);
    const identity: TentacleIdentity = {
      ...template,
      agentId: String(index),
      owner: address(index + 100),
      agentWallet: wallet,
      rawBalance,
      protocolHex: PROTOCOL_V1_HEX,
      profile: {
        ...template.profile,
        name: `Rank ${index}`,
        active: true,
        xmtpEndpoint: `xmtp://${inbox(index)}`,
      },
    };
    return {
      wallet,
      rawBalance,
      representativeAgentId: identity.agentId,
      identities: [identity],
      rank: index,
    };
  });
  return { ...base, fetchedAt: "2026-08-11T12:00:00.000Z", rankedWallets };
}

describe("first-connect Tentacle liveness directory", () => {
  beforeEach(() => localStorage.clear());

  it("selects only the five highest positive funded ranks from validated localStorage", async () => {
    expect(writeLeaderboardCache(localStorage, directory())).toBe(true);
    const selected = await loadLivenessCandidates(
      config,
      address(99),
      "f".repeat(64),
      localStorage,
      { now: () => NOW },
    );
    expect(selected.map(({ rank }) => rank)).toEqual([1, 2, 3, 4, 5]);
    expect(selected.map(({ name }) => name)).toEqual(["Rank 1", "Rank 2", "Rank 3", "Rank 4", "Rank 5"]);
  });

  it("excludes the Acolyte wallet, its own inbox, and duplicate inbox routes", async () => {
    const snapshot = directory();
    snapshot.rankedWallets[2]!.identities[0]!.profile.xmtpEndpoint =
      snapshot.rankedWallets[1]!.identities[0]!.profile.xmtpEndpoint;
    expect(writeLeaderboardCache(localStorage, snapshot)).toBe(true);
    const selected = await loadLivenessCandidates(
      config,
      address(1),
      inbox(4),
      localStorage,
      { now: () => NOW },
    );
    expect(selected.map(({ wallet }) => wallet)).not.toContain(address(1));
    expect(selected.map(({ inboxId }) => inboxId)).not.toContain(inbox(4));
    expect(selected.filter(({ inboxId }) => inboxId === inbox(2))).toHaveLength(1);
  });

  it("requires exactly one active protocol-v1 endpoint per funded wallet", async () => {
    const snapshot = directory();
    snapshot.rankedWallets[0]!.identities[0]!.profile.active = false;
    snapshot.rankedWallets[1]!.identities[0]!.protocolHex = "0x32";
    const duplicate = {
      ...snapshot.rankedWallets[2]!.identities[0]!,
      agentId: "30",
      owner: address(130),
      tentacleId: "ambiguous-other-tentacle",
      profile: { ...snapshot.rankedWallets[2]!.identities[0]!.profile, xmtpEndpoint: `xmtp://${inbox(10)}` },
    };
    snapshot.rankedWallets[2]!.identities.push(duplicate);
    expect(writeLeaderboardCache(localStorage, snapshot)).toBe(true);
    const selected = await loadLivenessCandidates(
      config,
      address(99),
      "f".repeat(64),
      localStorage,
      { now: () => NOW },
    );
    expect(selected.map(({ rank }) => rank)).not.toContain(1);
    expect(selected.map(({ rank }) => rank)).not.toContain(2);
    expect(selected.map(({ rank }) => rank)).not.toContain(3);
  });

  it("counts a proven duplicate registration once and routes only through the lower ID", async () => {
    const snapshot = directory(1);
    const canonical = snapshot.rankedWallets[0]!.identities[0]!;
    snapshot.rankedWallets[0]!.identities.push({
      ...structuredClone(canonical),
      agentId: "63846",
      profile: { ...canonical.profile, name: "Newer duplicate" },
    });
    expect(writeLeaderboardCache(localStorage, snapshot)).toBe(true);
    const selected = await loadLivenessCandidates(
      config,
      address(99),
      "f".repeat(64),
      localStorage,
      { now: () => NOW },
    );
    expect(selected).toHaveLength(1);
    expect(selected[0]!.agentId).toBe(canonical.agentId);
    expect(selected[0]!.name).toBe(canonical.profile.name);
  });

  it("performs one complete injected refresh when no hash-bound cache exists and persists it", async () => {
    const fresh = directory(2);
    const refresh = vi.fn(async () => fresh);
    const selected = await loadLivenessCandidates(config, address(99), "f".repeat(64), localStorage, {
      refresh,
      now: () => NOW,
    });
    expect(refresh).toHaveBeenCalledOnce();
    expect(selected).toHaveLength(2);
    expect(localStorage.getItem("cthuwu:leaderboard:v1")).toContain(fresh.sourceBlockHash);
  });

  it("refreshes a stale validated cache before choosing probe endpoints", async () => {
    const stale = {
      ...directory(2),
      fetchedAt: "2026-08-11T11:00:00.000Z",
    };
    const fresh = directory(1);
    fresh.rankedWallets[0]!.identities[0]!.agentId = "77";
    fresh.rankedWallets[0]!.representativeAgentId = "77";
    expect(writeLeaderboardCache(localStorage, stale)).toBe(true);
    const refresh = vi.fn(async () => fresh);

    const selected = await loadLivenessCandidates(config, address(99), "f".repeat(64), localStorage, {
      refresh,
      now: () => NOW,
    });

    expect(refresh).toHaveBeenCalledOnce();
    expect(selected.map(({ agentId }) => agentId)).toEqual(["77"]);
    expect(localStorage.getItem("cthuwu:leaderboard:v1")).toContain('"agentId":"77"');
  });

  it("fails closed when a refresh is incomplete", async () => {
    const incomplete = { ...directory(1), sourceBlockHash: undefined };
    await expect(loadLivenessCandidates(config, address(99), "f".repeat(64), localStorage, {
      refresh: async () => incomplete,
      now: () => NOW,
    })).rejects.toThrow(/incomplete/u);
  });
});
