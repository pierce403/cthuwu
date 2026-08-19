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
 * Collapse identities on the same wallet / controller: the canonical Tentacle is the
 * first one registered (lowest agent ID in the 8004 registry), and all others are ignored.
 */
export function canonicalizeWalletIdentities(
  source: readonly TentacleIdentity[],
): CanonicalTentacleSet {
  const identities = [...source].sort(compareAgentId);
  const parents = identities.map((_, index) => index);
  const firstByControl = new Map<string, number>();

  for (const [index, identity] of identities.entries()) {
    for (const control of currentControlAddresses(identity)) {
      const first = firstByControl.get(control);
      if (first === undefined) {
        firstByControl.set(control, index);
      } else {
        union(parents, first, index);
      }
    }
  }

  const componentsMap = new Map<number, TentacleIdentity[]>();
  for (const [index, identity] of identities.entries()) {
    const root = find(parents, index);
    componentsMap.set(root, [
      ...(componentsMap.get(root) ?? []),
      identity,
    ]);
  }

  const canonical: TentacleIdentity[] = [];
  const ignoredDuplicateAgentIds: string[] = [];
  const duplicateAgentAliases: DuplicateAgentAlias[] = [];

  for (const component of componentsMap.values()) {
    component.sort(compareAgentId);
    const lowest = component[0]!;
    const activeMember = component.find((id) => id.profile.active && id.profile.xmtpEndpoint) ?? lowest;
    const representative: TentacleIdentity = {
      ...lowest,
      tentacleId: lowest.tentacleId ?? component.find((id) => id.tentacleId)?.tentacleId,
      profile: {
        ...lowest.profile,
        ...(lowest.profile.xmtpEndpoint ? {} : { xmtpEndpoint: activeMember.profile.xmtpEndpoint }),
      },
    };
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
