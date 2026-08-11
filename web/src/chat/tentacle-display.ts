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
  for (const identity of [
    ...snapshot.rankedWallets.flatMap((wallet) => wallet.identities),
    ...snapshot.suspended,
  ]) {
    if (identity.agentId !== agentId) continue;
    return {
      name: identity.profile.name,
      ...(identity.profile.description ? { description: identity.profile.description } : {}),
    };
  }
  return undefined;
}
