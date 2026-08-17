import { describe, expect, it } from "vitest";
import { cachedSnapshot } from "./leaderboard-test-data";
import { canonicalizeWalletIdentities, provenSameTentacle } from "./tentacle-canonical";

function fixture(agentId: string) {
  return {
    ...structuredClone(cachedSnapshot().rankedWallets[0]!.identities[0]!),
    agentId,
  };
}

describe("canonical Tentacle identity collapse", () => {
  it("chooses the lowest agent ID for exact same-Tentacle duplicates", () => {
    const old = fixture("61766");
    const newer = {
      ...fixture("63846"),
      profile: {
        ...fixture("63846").profile,
        name: "Newer metadata must not win",
        xmtpEndpoint: `xmtp://${"b".repeat(64)}`,
      },
    };
    const result = canonicalizeWalletIdentities([newer, old]);
    expect(result.identities.map(({ agentId }) => agentId)).toEqual(["61766"]);
    expect(result.ignoredDuplicateAgentIds).toEqual(["63846"]);
    expect(result.duplicateAgentAliases).toEqual([{
      aliasAgentId: "63846",
      canonicalAgentId: "61766",
    }]);
    expect(result.identities[0]!.profile.name).toBe(old.profile.name);
  });

  it("keeps the lowest exact Tentacle when a higher alias has a stale agentWallet", () => {
    const canonical = fixture("61766");
    const staleAlias = {
      ...fixture("63846"),
      agentWallet: "0x9999999999999999999999999999999999999999",
    };
    expect(staleAlias.owner).toBe(canonical.owner);

    const result = canonicalizeWalletIdentities([staleAlias, canonical]);
    expect(result.identities.map(({ agentId }) => agentId)).toEqual(["61766"]);
    expect(result.duplicateAgentAliases).toEqual([{
      aliasAgentId: "63846",
      canonicalAgentId: "61766",
    }]);
    expect(result.identities[0]!.agentWallet).toBe(canonical.agentWallet);
  });

  it("supports a missing-tentacle-id migration only with an exact shared XMTP endpoint", () => {
    const endpoint = `xmtp://${"c".repeat(64)}`;
    const canonical = { ...fixture("7"), profile: { ...fixture("7").profile, xmtpEndpoint: endpoint } };
    const legacy = {
      ...fixture("8"),
      tentacleId: undefined,
      profile: { ...fixture("8").profile, xmtpEndpoint: endpoint },
    };
    expect(provenSameTentacle(canonical, legacy)).toBe(true);
    expect(canonicalizeWalletIdentities([legacy, canonical]).ignoredDuplicateAgentIds).toEqual(["8"]);
  });

  it("retains genuinely ambiguous same-wallet identities", () => {
    const first = fixture("7");
    const second = { ...fixture("8"), tentacleId: "another-tentacle" };
    const result = canonicalizeWalletIdentities([second, first]);
    expect(result.identities.map(({ agentId }) => agentId)).toEqual(["7", "8"]);
    expect(result.ignoredDuplicateAgentIds).toEqual([]);
  });

  it("never lets a legacy endpoint bridge conflicting exact Tentacle IDs", () => {
    const endpoint = `xmtp://${"d".repeat(64)}`;
    const first = {
      ...fixture("7"),
      tentacleId: "tentacle-foo",
      profile: { ...fixture("7").profile, xmtpEndpoint: endpoint },
    };
    const legacy = {
      ...fixture("8"),
      tentacleId: undefined,
      profile: { ...fixture("8").profile, xmtpEndpoint: endpoint },
    };
    const second = {
      ...fixture("9"),
      tentacleId: "tentacle-bar",
      profile: { ...fixture("9").profile, xmtpEndpoint: endpoint },
    };
    const result = canonicalizeWalletIdentities([legacy, second, first]);
    expect(result.identities.map(({ agentId }) => agentId)).toEqual(["7", "8", "9"]);
    expect(result.ignoredDuplicateAgentIds).toEqual([]);
  });

  it("never collapses copied evidence without a shared current control relationship", () => {
    const endpoint = `xmtp://${"e".repeat(64)}`;
    const canonical = {
      ...fixture("7"),
      profile: { ...fixture("7").profile, xmtpEndpoint: endpoint },
    };
    const otherWallet = {
      ...fixture("8"),
      owner: "0x8888888888888888888888888888888888888888",
      agentWallet: "0x9999999999999999999999999999999999999999",
      profile: { ...fixture("8").profile, xmtpEndpoint: endpoint },
    };
    const zeroWallet = {
      ...fixture("9"),
      agentWallet: "0x0000000000000000000000000000000000000000",
      owner: "0x0000000000000000000000000000000000000000",
      profile: { ...fixture("9").profile, xmtpEndpoint: endpoint },
    };
    const result = canonicalizeWalletIdentities([zeroWallet, otherWallet, canonical]);
    expect(result.identities.map(({ agentId }) => agentId)).toEqual(["7", "8", "9"]);
    expect(result.ignoredDuplicateAgentIds).toEqual([]);
  });

  it("keeps legacy endpoint matches separate without exact protocol-v1 endpoint proof", () => {
    const endpoint = `xmtp://${"f".repeat(64)}`;
    const validLegacy = {
      ...fixture("7"),
      tentacleId: undefined,
      profile: { ...fixture("7").profile, xmtpEndpoint: endpoint },
    };
    const incompatibleProtocol = {
      ...fixture("8"),
      tentacleId: undefined,
      protocolHex: "0x32",
      profile: { ...fixture("8").profile, xmtpEndpoint: endpoint },
    };
    expect(canonicalizeWalletIdentities([
      incompatibleProtocol,
      validLegacy,
    ]).ignoredDuplicateAgentIds).toEqual([]);

    const malformedFirst = {
      ...fixture("9"),
      tentacleId: undefined,
      profile: { ...fixture("9").profile, xmtpEndpoint: "xmtp://malformed" },
    };
    const malformedSecond = {
      ...fixture("10"),
      tentacleId: undefined,
      profile: { ...fixture("10").profile, xmtpEndpoint: "xmtp://malformed" },
    };
    expect(canonicalizeWalletIdentities([
      malformedSecond,
      malformedFirst,
    ]).ignoredDuplicateAgentIds).toEqual([]);
  });

  it("collapses an active and suspended alias only with the same exact control and Tentacle evidence", () => {
    const active = fixture("61766");
    const suspended = {
      ...fixture("63846"),
      agentWallet: "0x0000000000000000000000000000000000000000",
      owner: active.agentWallet,
      rawBalance: "0",
    };
    const result = canonicalizeWalletIdentities([suspended, active]);
    expect(result.identities.map(({ agentId }) => agentId)).toEqual(["61766"]);
    expect(result.duplicateAgentAliases).toEqual([{
      aliasAgentId: "63846",
      canonicalAgentId: "61766",
    }]);

    suspended.owner = "0x9999999999999999999999999999999999999999";
    expect(canonicalizeWalletIdentities([suspended, active]).identities).toHaveLength(2);
  });

  it("maps each duplicate to its own component rather than the wallet representative", () => {
    const unrelated = { ...fixture("10"), tentacleId: "unrelated-tentacle" };
    const canonical = { ...fixture("20"), tentacleId: "duplicate-component" };
    const alias = { ...fixture("30"), tentacleId: "duplicate-component" };
    const result = canonicalizeWalletIdentities([alias, canonical, unrelated]);
    expect(result.identities.map(({ agentId }) => agentId)).toEqual(["10", "20"]);
    expect(result.duplicateAgentAliases).toEqual([{
      aliasAgentId: "30",
      canonicalAgentId: "20",
    }]);
  });
});
