# Identity and registry

See [Council protocol](README.md) for status labels and privacy boundaries.

## Durable and runtime identity

A Cthulhu identity survives process restarts and may own several Tentacles. A Tentacle ID is also
stable, while its incarnation changes whenever that Tentacle runtime is started anew.

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
autonomous goals.

## Identifier requirements

Identifiers are typed rather than interchangeable strings. `CthulhuId`, `TentacleId`, `CouncilId`,
`SessionId`, `RequestId`, `LeaseId`, `ProposalId`, and stable message IDs use their documented
lowercase prefix. A complete ID is at most 96 bytes; its suffix is 1–64 lowercase ASCII alphanumeric
characters with optional internal `_` or `-`. Separators cannot lead, trail, or repeat. Parsers reject
whitespace, path separators, control characters, ambiguous case, and overlong values. An identifier
is never used as a path without an additional storage-layer path check.

An `XmtpInboxRef` is a bounded, even-length lowercase hexadecimal inbox ID serialized as a string.
The owning endpoint or runtime configuration supplies the XMTP environment separately; an inbox
reference is never silently interpreted across `dev`, `production`, or `local`.

```json
"012345abcdef"
```

Registry references are opaque, versioned references. Domain objects do not assume Ethereum, one
chain ID, one contract, one ABI, or one ERC-8004 draft revision.

```json
{
  "registry": "erc-8004",
  "reference": "eip155:any:agent:42"
}
```

## Endpoint association

An endpoint claim binds a Cthulhu to an XMTP endpoint through an `AgentRegistry` verification result
or a local allowlist policy. Receiving an announcement is not sufficient proof. The runtime compares:

1. the authenticated Council sender;
2. `senderCthulhuId` and `senderTentacleId` in the envelope;
3. the Tentacle owner in local state;
4. any registry or allowlist endpoint association required by local policy.

A mismatch fails closed and does not update membership, liveness, or routing state.

## Registry interface

`AgentRegistry` supports:

- resolving a Cthulhu identity;
- registering or updating public metadata;
- retrieving endpoints and capability references;
- retrieving trust and reputation signals with provenance;
- verifying endpoint association;
- determining active status.

**Implemented — local:** `LocalRegistry`, suitable for tests and operator-managed deployments. Its
records are bounded and serializable; the local simulator persists them inside its protected combined
snapshot. `LocalRegistry` itself remains storage-agnostic.

**Experimental adapter:** `Erc8004Registry` is an isolated boundary. It must be configured with a
specific chain, deployment, ABI, and compatible ERC-8004 revision before it can be called live.
Those details belong in adapter configuration, not protocol domain types.

## Reputation and trust

A trust signal contains provenance, kind, a bounded value, observation time, and an optional evidence
reference. The containing registry record identifies the Cthulhu subject. Routing policy decides
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

The local simulator stores durable Cthulhu and Tentacle identity records in its combined snapshot
beneath the existing protected `UWUBOT_DATA_DIR`, not inside the repository. Snapshot creation uses
owner-only directories, restrictive file permissions, atomic replacement, and directory
synchronization where supported. A partial or mismatched snapshot fails closed rather than silently
generating a replacement Cthulhu. A future live coordinator must preserve the same behavior.
