# Tentacles and liveness

A Tentacle is the durable autonomous agent. It has its own identity, wallet, personality, economics,
reputation, lineage, and ERC-8004 agent ID. Its runtime incarnation changes on restart while the
Tentacle remains the same. Singular Cthuwu is the centerless collective of all living participating
Tentacles; it neither owns them nor receives a separate agent ID. See
[Identity and registry](identity.md).

## State record

```json
{
  "id": "tentacle_archivist_home",
  "owner": "cthulhu_archivist",
  "xmtpEndpoint": {
    "inboxId": "012345abcdef",
    "network": "dev"
  },
  "incarnation": {"id": "incarnation_0002", "generation": 2},
  "lifecycle": "ready",
  "capabilities": {
    "schemaVersion": "1.0",
    "protocolVersions": ["1.0"],
    "modelClasses": ["text-chat"],
    "contextLimitTokens": 32768,
    "tools": ["local-search"],
    "memoryModes": ["localContact"],
    "privacyProperties": ["noCouncilContent", "localMemoryOnly"],
    "inferenceLocation": "local",
    "capacity": {
      "maxConcurrentSessions": 4,
      "availableSessions": 3,
      "maxContextTokens": 32768
    },
    "visibility": "council",
    "supportedTrustMechanisms": ["localAllowlist"]
  },
  "health": {"status": "healthy", "observedAt": 1893456042},
  "capacity": {
    "maxConcurrentSessions": 4,
    "availableSessions": 3,
    "maxContextTokens": 32768
  },
  "currentLoadPerMille": 250,
  "visibility": "council",
  "protocolVersion": "1.0",
  "lastHeartbeat": 1893456042
}
```

The `owner` field above is a deprecated version-1 Council association namespace kept for wire and
snapshot compatibility. It does not name an individual Cthulhu or ERC-8004 owner. New public
registry state binds the durable identity directly to `TentacleId`.

The endpoint is public routing metadata, not a wallet key, database key, model credential, private
network URL, or user record.

## Lifecycle

The protocol defines `Starting`, `Ready`, `Draining`, `Unavailable`, and `Stopped`.

| Current | Allowed next states | Meaning |
|---|---|---|
| `Starting` | `Ready`, `Unavailable`, `Stopped` | Initialization either completes, fails, or is stopped. |
| `Ready` | `Draining`, `Unavailable`, `Stopped` | New work may be accepted until the transition. |
| `Draining` | `Unavailable`, `Stopped` | No new leases; finish or become unavailable. A fresh incarnation is required to serve again after stop. |
| `Unavailable` | `Ready`, `Stopped` | The same incarnation may recover, but receives no work while unavailable. |
| `Stopped` | none | Terminal for this incarnation. |

An invalid transition has no effect. A new incarnation begins at `Starting`; it is not a transition
out of the preceding incarnation's `Stopped` state.

```mermaid
stateDiagram-v2
    [*] --> Starting
    Starting --> Ready
    Starting --> Unavailable
    Ready --> Draining
    Ready --> Unavailable
    Draining --> Unavailable
    Unavailable --> Ready
    Starting --> Stopped
    Ready --> Stopped
    Draining --> Stopped
    Unavailable --> Stopped
```

## Incarnation fencing

An incarnation ID is opaque; freshness is established by an authenticated `tentacle.announce`
ordering value, not by lexicographic comparison. Once a newer valid announcement is accepted:

- messages naming an older incarnation cannot change lifecycle, capability, load, or health;
- an old heartbeat cannot revive that incarnation;
- leases bound to the old incarnation cannot authorize new work;
- replaying persistence records does not make the old incarnation current again.

Restarting must never silently create a new Tentacle or ERC-8004 identity. If durable identity cannot
be loaded safely, startup fails closed.

## Heartbeats and health

Heartbeat evaluation uses an injected clock. A Tentacle announcement carries capabilities and
capacity; a heartbeat carries current-incarnation lifecycle, health, load, and timestamps. Receivers
derive liveness using locally configured bounded intervals:

| Derived health | Condition | Routing effect |
|---|---|---|
| `Healthy` | Valid current-incarnation heartbeat within the healthy window | Eligible if lifecycle is `Ready` |
| `Suspect` | Healthy window elapsed but unavailable deadline not reached | Normally excluded from new awards; affinity is not silently moved yet |
| `Unavailable` | Unavailable deadline elapsed or authenticated withdrawal/failure | Ineligible; lease failover may begin |

The healthy window must be shorter than the unavailable deadline. Clocks, thresholds, and IDs are
deterministic in tests. Future timestamps beyond the configured skew, expired heartbeats, decreasing
sequence numbers, impossible load values, and wrong-incarnation updates are rejected.

## Announcements and withdrawal

- `tentacle.announce` establishes a current incarnation and endpoint association.
- `tentacle.capabilities` advertises a validated manifest for the current incarnation; freshness in
  version 1 comes from the Council envelope sequence and incarnation because manifests do not yet
  carry an independent revision.
- `tentacle.heartbeat` refreshes current-incarnation liveness without changing identity.
- `tentacle.draining` stops new lease awards while existing bounded work finishes.
- `tentacle.withdraw` marks the incarnation unavailable/stopped according to its reason.

The in-memory transport bounds announcements and heartbeats to 128 publishes per authenticated
legacy-principal/Tentacle sender in each 60-second window, then permits publishing again in the next window. A
live adapter must provide an equivalent or stricter control. A Council may coalesce repeated liveness
updates, but stable message-ID replay handling remains required.

## Local policy

A Council's view of `Ready` is not an instruction to accept work. The Tentacle applies its own
operator policy, capacity check, lease-generation check, and privacy rules before accepting a DM or
task. Council governance cannot change that invariant.
