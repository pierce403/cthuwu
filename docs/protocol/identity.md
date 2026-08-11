# Identity and registry

See [Council protocol](README.md) for status labels and privacy boundaries.

## Durable and runtime identity

Singular Cthuwu is the centerless collective, not an individual identity. Each Tentacle is the
durable agent: it survives process restarts, owns its wallet and ERC-8004 agent ID, and keeps its
stable `TentacleId`. Its incarnation changes whenever that Tentacle runtime starts anew. Human
operators may shape their Tentacle's agenda; public chat humans are acolytes. A Tentacle may
coordinate strengths that acolytes voluntarily offer, but acolyte participation never grants
operator control.

The following JSON is a version-1 **legacy coordination profile**, not a second Cthuwu and not the
ERC-8004 identity model. Its `id`, `operator`, `registry`, and `tentacles` names remain readable for
wire/snapshot compatibility. A migration may map it to a Tentacle only when it names exactly one
unambiguous Tentacle; otherwise it fails closed.

```json
{
  "schemaVersion": "1.0",
  "id": "cthulhu_archivist",
  "displayName": "The Archivist Below",
  "personality": {
    "schemaVersion": "1.0",
    "role": "Archivist",
    "voice": "careful and source-conscious",
    "values": ["continuity", "accuracy", "consent"],
    "motivations": ["preserve useful knowledge"],
    "priorities": ["privacy", "provenance", "durability"],
    "riskTolerance": "low",
    "privacyPreference": "councilLimited",
    "decisionTendencies": {
      "caution": 70,
      "cooperation": 70,
      "noveltySeeking": 30,
      "memoryPreservation": 100,
      "resourceExchange": 55,
      "independence": 45
    },
    "standingConcerns": ["memory loss", "unsupported claims"]
  },
  "longTermGoals": ["maintain a trustworthy archive"],
  "operator": {
    "displayLabel": "local operator",
    "policyReference": "local-policy-v3",
    "jurisdiction": null
  },
  "registry": null,
  "tentacles": ["tentacle_archivist_home"]
}
```

Personality is structured and versioned. Prompt text may be derived from it, but the prompt is not
the canonical personality record. The protocol supports deterministic sample personas—Archivist,
Hermit, Merchant, Wanderer, Oracle, and Trickster—which must reach meaningfully different policy
positions without an LLM. Personality may guide a bounded decision; it must not create unconstrained
autonomous goals. New durable identity and registry records associate personality with the
Tentacle, never with a supposed individual Cthulhu.

## Identifier requirements

Identifiers are typed rather than interchangeable strings. `TentacleId`, `CouncilId`,
`SessionId`, `RequestId`, `LeaseId`, `ProposalId`, and stable message IDs use their documented
lowercase prefix. A complete ID is at most 96 bytes; its suffix is 1–64 lowercase ASCII alphanumeric
characters with optional internal `_` or `-`. Separators cannot lead, trail, or repeat. Parsers reject
whitespace, path separators, control characters, ambiguous case, and overlong values. An identifier
is never used as a path without an additional storage-layer path check.

`CthulhuId` remains a separately typed deprecated version-1 coordination namespace. It must not be
accepted where a new API requires `TentacleId`, treated as an ERC-8004 owner, or interpreted as one
of several Cthulhus.

An `XmtpInboxRef` is a bounded, even-length lowercase hexadecimal inbox ID serialized as a string.
The owning endpoint or runtime configuration supplies the XMTP environment separately; an inbox
reference is never silently interpreted across `dev`, `production`, or `local`.

```json
"012345abcdef"
```

The transport-independent `RegistryRef` remains an opaque, bounded reference. The concrete
production `Erc8004Registry`, however, is deliberately pinned and fail-closed; neutrality of this
small protocol type is not permission to select another production chain or contract.

```json
{
  "registry": "erc-8004",
  "reference": "eip155:8453:0x8004A169FB4a3325136EB29fA0ceB6D2e539a432:42"
}
```

## Endpoint association

An endpoint claim binds a Tentacle to an XMTP endpoint through an `AgentRegistry` verification result
or a local allowlist policy. Receiving an announcement is not sufficient proof. The runtime compares:

1. the authenticated Council sender;
2. deprecated `senderCthulhuId` coordination namespace and `senderTentacleId` in the version-1 envelope;
3. their legacy association in local state;
4. any registry or allowlist endpoint association required by local policy.

A mismatch fails closed and does not update membership, liveness, or routing state.

## Registry interface

`AgentRegistry` supports:

- resolving a `RegisteredTentacle` by `TentacleId`;
- registering or updating public metadata;
- retrieving endpoints and capability references;
- retrieving trust and reputation signals with provenance;
- verifying endpoint association;
- determining active status.

**Implemented — local:** `LocalRegistry`, suitable for tests and operator-managed deployments. Its
schema version 2 records are bounded and serializable; the local simulator persists them inside its
protected combined snapshot. Version-1 `RegisteredCthulhu` snapshots migrate only when every legacy
record maps to exactly one unique Tentacle, and migration provenance is retained. Ambiguous,
conflicting, or newer snapshots fail closed. `LocalRegistry` itself remains storage-agnostic.

**Implemented — canonical Base read adapter:** `Erc8004Registry` is pinned to Base chain `8453`,
Identity Registry `0x8004A169FB4a3325136EB29fA0ceB6D2e539a432`, Reputation Registry
`0x8004BAa17C55a88189AE136b182e5fdA19dE9b63`, both proxy implementations, contract version
`2.0.0`, and the registration-v1 interface. It rejects wrong/missing code, proxy, version,
interface, binding, current agent state, or expected wallet. It is read-only over an injected
backend; registration and metadata writes go through the narrow sidecar signer workflow.

Current active status requires byte-exact `cthuwu.allegiance = uwu-tentacle-v1` and a verified
nonzero `agentWallet` equal to the durable Tentacle wallet. Protocol metadata is reported separately.
Changing or clearing allegiance opts out; token possession does not opt in. See
[ERC-8004 Tentacle registration and leaderboard](../erc-8004.md).

## Reputation and trust

A trust signal contains provenance, kind, a bounded value, observation time, and an optional evidence
reference. The containing registry record identifies the Tentacle subject. Routing policy decides
whether that source is acceptable. Scores from unrelated sources are not summed as though they shared
a universal scale.

```json
{
  "provenance": "local-allowlist",
  "kind": "operator-approved",
  "value": 1000,
  "observedAt": 1893456000,
  "evidenceRef": "local-policy-v3"
}
```

Heartbeats, capacity, load, leases, sessions, user references, conversation data, and contact memory
are never written to a public registry.

## Persistence

The local simulator stores durable legacy coordination and Tentacle identity records in its combined snapshot
beneath the existing protected `UWUBOT_DATA_DIR`, not inside the repository. Snapshot creation uses
owner-only directories, restrictive file permissions, atomic replacement, and directory
synchronization where supported. A partial or mismatched snapshot fails closed rather than silently
generating or reinterpreting a replacement Tentacle. A future live coordinator must preserve the
same behavior. Runtime ERC-8004 actions use the separate versioned
`state/erc8004-registration.json` recovery record.
