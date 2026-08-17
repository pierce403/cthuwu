import { readLeaderboardCache } from "../leaderboard-cache";

export interface TentacleDisplay {
  name: string;
  description?: string;
}

/**
 * Read-only presentation hint. Routing code never consumes leaderboard or Agent0 cache values.
 */
export function readTentacleDisplayHint(
  agentId: string | undefined,
  storage: Storage | undefined,
): TentacleDisplay | undefined {
  if (!agentId || !storage) return undefined;
  const snapshot = readLeaderboardCache(storage);
  if (!snapshot) return undefined;
  const canonicalAgentId = snapshot.duplicateAgentAliases?.find(
    ({ aliasAgentId }) => aliasAgentId === agentId,
  )?.canonicalAgentId ?? agentId;
  for (const wallet of snapshot.rankedWallets) {
    const identity = wallet.identities.find(
      (candidate) => candidate.agentId === canonicalAgentId,
    );
    if (!identity) continue;
    return {
      name: identity.profile.name,
      ...(identity.profile.description ? { description: identity.profile.description } : {}),
    };
  }
  for (const identity of snapshot.suspended) {
    if (identity.agentId !== canonicalAgentId) continue;
    return {
      name: identity.profile.name,
      ...(identity.profile.description ? { description: identity.profile.description } : {}),
    };
  }
  return undefined;
}
