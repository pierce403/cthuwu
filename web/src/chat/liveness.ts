import type { AppConfig } from "../config";
import {
  isSnapshotStale,
  readLeaderboardCache,
  writeLeaderboardCache,
} from "../leaderboard-cache";
import { parseLeaderboardConfig } from "../leaderboard-config";
import { fetchCompleteLeaderboard } from "../leaderboard-data";
import { PROTOCOL_V1_HEX, type LeaderboardSnapshot } from "../leaderboard-types";

const XMTP_ENDPOINT = /^xmtp:\/\/([0-9a-f]{64})$/u;
const MAX_PROBES = 5;

export interface LivenessCandidate {
  wallet: string;
  agentId: string;
  inboxId: string;
  name: string;
  rank: number;
  blockNumber: string;
  blockHash: string;
}

export interface LivenessDirectoryDependencies {
  fetch?: typeof fetch;
  refresh?: () => Promise<LeaderboardSnapshot>;
  now?: () => number;
  logger?: LivenessLogger;
}

export interface LivenessLogger {
  debug(message: string, details?: Record<string, unknown>): void;
  info(message: string, details?: Record<string, unknown>): void;
  warn(message: string, details?: Record<string, unknown>): void;
}

/**
 * Read the complete, schema-validated local leaderboard first. If it is missing, stale, or predates
 * the mandatory source hash, perform one complete pinned Agent0 refresh with Base UWU balances at
 * the exact same block, persist it, and then select only the five highest funded usable routes.
 */
export async function loadLivenessCandidates(
  config: AppConfig,
  acolyteAddress: string,
  ownInboxId: string,
  storage: Storage | undefined,
  dependencies: LivenessDirectoryDependencies = {},
): Promise<LivenessCandidate[]> {
  const leaderboardConfig = parseLeaderboardConfig();
  const logger = dependencies.logger ?? console;
  const now = Math.floor((dependencies.now ?? Date.now)());
  if (!Number.isSafeInteger(now) || now < 0) {
    throw new Error("The browser clock is unavailable for the Tentacle directory");
  }
  let snapshot = storage ? readLeaderboardCache(storage) : undefined;
  const cacheAgeMs = snapshot ? Math.max(0, now - Date.parse(snapshot.fetchedAt)) : undefined;
  const stale = snapshot
    ? isSnapshotStale(snapshot, leaderboardConfig.cacheFreshnessMs, now)
    : false;
  if (!snapshot?.sourceBlockHash || stale) {
    if (!leaderboardConfig.graphEndpoint) {
      throw new Error("The complete Tentacle directory is unavailable");
    }
    logger.info("[cthuwu-liveness] refreshing Tentacle directory", {
      reason: stale ? "stale-cache" : "missing-cache",
      cachedBlock: snapshot?.sourceBlockNumber ?? null,
      cacheAgeMs: cacheAgeMs ?? null,
    });
    snapshot = await (dependencies.refresh ?? (() => fetchCompleteLeaderboard(
      leaderboardConfig.graphEndpoint!,
      {
        fetch: dependencies.fetch,
        baseRpcEndpoint: config.baseRpcEndpoint,
      },
    )))();
    if (!snapshot.sourceBlockHash || !snapshot.paginationComplete || snapshot.hasIndexingErrors) {
      throw new Error("The refreshed Tentacle directory is incomplete");
    }
    if (storage && !writeLeaderboardCache(storage, snapshot)) {
      // Persistence is useful for later first-connects, but a complete in-memory snapshot is still
      // safe for this one bounded race.
      logger.info("[cthuwu-liveness] validated directory could not be cached");
    }
  } else {
    logger.debug("[cthuwu-liveness] using fresh validated Tentacle directory", {
      sourceBlock: snapshot.sourceBlockNumber,
      cacheAgeMs: cacheAgeMs ?? null,
    });
  }

  const usedWallets = new Set<string>();
  const usedInboxes = new Set<string>();
  const selected: LivenessCandidate[] = [];
  const retainedAddress = config.rotationAnchor ?? (storage
    ? storage.getItem(`cthuwu.rotation.v1:${config.environment}:${acolyteAddress}`) ?? undefined
    : undefined);
  const rankedWallets = retainedAddress
    ? [
        ...snapshot.rankedWallets.filter(
          (g) => g.wallet.toLowerCase() === retainedAddress.toLowerCase(),
        ),
        ...snapshot.rankedWallets.filter(
          (g) => g.wallet.toLowerCase() !== retainedAddress.toLowerCase(),
        ),
      ]
    : snapshot.rankedWallets;
  for (const group of rankedWallets) {
    if (selected.length >= MAX_PROBES) break;
    if (group.rank === undefined || group.rank <= 0 || BigInt(group.rawBalance) <= 0n) continue;
    if (group.wallet === acolyteAddress || usedWallets.has(group.wallet)) continue;
    const eligible = group.identities.flatMap((identity) => {
      const endpoint = identity.profile.xmtpEndpoint?.match(XMTP_ENDPOINT);
      return identity.protocolHex === PROTOCOL_V1_HEX && identity.profile.active && endpoint
        ? [{ identity, inboxId: endpoint[1]! }]
        : [];
    });
    if (eligible.length === 0) continue;
    eligible.sort((a, b) => {
      const left = BigInt(a.identity.agentId);
      const right = BigInt(b.identity.agentId);
      return left === right ? 0 : left < right ? -1 : 1;
    });
    const { identity, inboxId } = eligible[0]!;
    if (inboxId === ownInboxId || usedInboxes.has(inboxId)) continue;
    usedWallets.add(group.wallet);
    usedInboxes.add(inboxId);
    selected.push({
      wallet: group.wallet,
      agentId: identity.agentId,
      inboxId,
      name: identity.profile.name,
      rank: group.rank,
      blockNumber: snapshot.sourceBlockNumber,
      blockHash: snapshot.sourceBlockHash,
    });
  }
  logger.info("[cthuwu-liveness] selected ranked candidates", {
    sourceBlock: snapshot.sourceBlockNumber,
    candidateCount: selected.length,
    candidates: selected.map(({ rank, agentId }) => ({ rank, agentId })),
  });
  return selected;
}
