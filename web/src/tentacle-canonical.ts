import {
  PROTOCOL_V1_HEX,
  ZERO_ADDRESS,
  type DuplicateAgentAlias,
  type TentacleIdentity,
} from "./leaderboard-types";

const XMTP_ENDPOINT = /^xmtp:\/\/[0-9a-f]{64}$/u;
const WALLET = /^0x[0-9a-f]{40}$/u;

export interface CanonicalTentacleSet {
  identities: TentacleIdentity[];
  ignoredDuplicateAgentIds: string[];
  duplicateAgentAliases: DuplicateAgentAlias[];
}

/**
 * Collapse only identities that carry affirmative evidence of the same durable Tentacle.
 * A shared wallet alone is deliberately insufficient: operators may own unrelated ERC-8004 NFTs.
 */
export function canonicalizeWalletIdentities(
  source: readonly TentacleIdentity[],
): CanonicalTentacleSet {
  const identities = [...source].sort(compareAgentId);
  const exact: TentacleIdentity[] = [];
  const legacy: TentacleIdentity[] = [];
  for (const identity of identities) {
    if (identity.tentacleId && currentControlAddresses(identity).length > 0) {
      exact.push(identity);
    } else {
      legacy.push(identity);
    }
  }
  // An agentWallet may have been replaced on one historical duplicate while the
  // common ERC-721 owner still proves current control of both registrations. Build
  // connected components over every nonzero current control address instead of
  // keying on the preferred agentWallet and accidentally splitting that duplicate.
  const parents = exact.map((_, index) => index);
  const firstByEvidence = new Map<string, number>();
  for (const [index, identity] of exact.entries()) {
    for (const control of currentControlAddresses(identity)) {
      const key = `${identity.tentacleId!}\u0000${control}`;
      const first = firstByEvidence.get(key);
      if (first === undefined) firstByEvidence.set(key, index);
      else union(parents, first, index);
    }
  }
  const exactComponents = new Map<number, TentacleIdentity[]>();
  for (const [index, identity] of exact.entries()) {
    const root = find(parents, index);
    exactComponents.set(root, [
      ...(exactComponents.get(root) ?? []),
      identity,
    ]);
  }
  const components = [...exactComponents.values()];
  const legacyByEndpoint = new Map<string, TentacleIdentity[]>();
  for (const identity of legacy) {
    const matches = components.filter((component) =>
      component.some((candidate) => provenSameTentacle(candidate, identity)));
    if (matches.length === 1) {
      matches[0]!.push(identity);
      continue;
    }
    // A legacy bridge that matches two conflicting exact Tentacle IDs proves neither alias.
    const wallet = canonicalControlWallet(identity);
    const endpoint = identity.profile.xmtpEndpoint;
    if (
      matches.length > 1 ||
      wallet === undefined ||
      identity.protocolHex !== PROTOCOL_V1_HEX ||
      endpoint === undefined ||
      !XMTP_ENDPOINT.test(endpoint)
    ) {
      components.push([identity]);
      continue;
    }
    const key = `${wallet}\u0000${endpoint}`;
    legacyByEndpoint.set(key, [
      ...(legacyByEndpoint.get(key) ?? []),
      identity,
    ]);
  }
  components.push(...legacyByEndpoint.values());

  const canonical: TentacleIdentity[] = [];
  const ignoredDuplicateAgentIds: string[] = [];
  const duplicateAgentAliases: DuplicateAgentAlias[] = [];
  for (const component of components) {
    component.sort(compareAgentId);
    const representative = component[0]!;
    canonical.push(representative);
    ignoredDuplicateAgentIds.push(...component.slice(1).map(({ agentId }) => agentId));
    duplicateAgentAliases.push(...component.slice(1).map(({ agentId }) => ({
      aliasAgentId: agentId,
      canonicalAgentId: representative.agentId,
    })));
  }
  return {
    identities: canonical.sort(compareAgentId),
    ignoredDuplicateAgentIds: ignoredDuplicateAgentIds.sort(compareDecimal),
    duplicateAgentAliases: duplicateAgentAliases.sort((left, right) =>
      compareDecimal(left.aliasAgentId, right.aliasAgentId)),
  };
}

export function canonicalControlWallet(identity: TentacleIdentity): string | undefined {
  const agentWallet = identity.agentWallet.toLowerCase();
  if (WALLET.test(agentWallet) && agentWallet !== ZERO_ADDRESS) return agentWallet;
  const owner = identity.owner.toLowerCase();
  return WALLET.test(owner) && owner !== ZERO_ADDRESS ? owner : undefined;
}

export function provenSameTentacle(
  left: TentacleIdentity,
  right: TentacleIdentity,
): boolean {
  if (!sharesCurrentControl(left, right)) return false;

  if (left.tentacleId && right.tentacleId) {
    return left.tentacleId === right.tentacleId;
  }

  const leftEndpoint = left.profile.xmtpEndpoint;
  const rightEndpoint = right.profile.xmtpEndpoint;
  return (
    left.protocolHex === PROTOCOL_V1_HEX &&
    right.protocolHex === PROTOCOL_V1_HEX &&
    leftEndpoint !== undefined &&
    rightEndpoint !== undefined &&
    XMTP_ENDPOINT.test(leftEndpoint) &&
    leftEndpoint === rightEndpoint
  );
}

function currentControlAddresses(identity: TentacleIdentity): string[] {
  const addresses = new Set<string>();
  for (const value of [identity.agentWallet, identity.owner]) {
    const address = value.toLowerCase();
    if (WALLET.test(address) && address !== ZERO_ADDRESS) addresses.add(address);
  }
  return [...addresses];
}

function sharesCurrentControl(
  left: TentacleIdentity,
  right: TentacleIdentity,
): boolean {
  const rightControls = new Set(currentControlAddresses(right));
  return currentControlAddresses(left).some((control) => rightControls.has(control));
}

function find(parents: number[], index: number): number {
  let root = index;
  while (parents[root] !== root) root = parents[root]!;
  while (parents[index] !== index) {
    const parent = parents[index]!;
    parents[index] = root;
    index = parent;
  }
  return root;
}

function union(parents: number[], left: number, right: number): void {
  const leftRoot = find(parents, left);
  const rightRoot = find(parents, right);
  if (leftRoot !== rightRoot) parents[rightRoot] = leftRoot;
}

function compareAgentId(left: TentacleIdentity, right: TentacleIdentity): number {
  return compareDecimal(left.agentId, right.agentId);
}

function compareDecimal(left: string, right: string): number {
  const a = BigInt(left);
  const b = BigInt(right);
  return a === b ? 0 : a < b ? -1 : 1;
}
