# Capability manifests

Capability manifests let routing compare requirements without exposing secrets or normal user
messages. They are declarations, not authorization: every Tentacle rechecks its local policy when
work arrives.

## Manifest shape

```json
{
  "schemaVersion": "1.0",
  "protocolVersions": ["1.0"],
  "modelClasses": ["text-chat", "summarization", "code"],
  "contextLimitTokens": 32768,
  "tools": ["local-search"],
  "memoryModes": ["session", "localContact"],
  "privacyProperties": ["noCouncilContent", "localMemoryOnly", "noRemoteInference"],
  "inferenceLocation": "local",
  "capacity": {
    "maxConcurrentSessions": 4,
    "availableSessions": 3,
    "maxContextTokens": 32768
  },
  "visibility": "council",
  "supportedTrustMechanisms": ["localAllowlist"]
}
```

All strings, maps, lists, numeric values, and the encoded manifest have implementation-defined but
tested upper bounds. Duplicate entries are normalized or rejected consistently. Unknown protocol
versions do not become compatible merely because their JSON parses.

## Safe advertisement

A manifest may advertise:

- compatible Council protocol versions;
- coarse model capability classes, not provider credentials or private model identifiers unless the
  operator intentionally makes them public;
- bounded context limits;
- tool capability identifiers;
- memory modes and retention/privacy properties;
- local versus remote inference;
- coarse capacity; current load is separate Tentacle status rather than a manifest field;
- Council/public/private visibility;
- supported trust mechanisms and public capability-reference hashes.

A manifest must never include private keys, API keys, tokens, database material, prompt or transcript
content, contact memory, private endpoint URLs, local filesystem paths, exact unnecessary hardware
inventory, or debug environment dumps.

## Visibility

| Visibility | Meaning |
|---|---|
| `Private` | Known only to the local runtime and never announced. |
| `Council` | May be announced to authenticated members of the selected Council. |
| `Public` | Operator has explicitly approved publication through a registry or public document. |

Changing from a narrower to a broader visibility requires an explicit local configuration change.
Joining a Council does not automatically make a manifest public.

## Requirements versus preferences

A route request distinguishes hard requirements from scoring preferences.

```json
{
  "required": {
    "protocolVersions": ["1.0"],
    "modelClasses": ["code"],
    "tools": ["local-search"],
    "privacyProperties": ["localMemoryOnly", "noRemoteInference"],
    "minimumContextTokens": 16000,
    "maximumLoadPercent": 75
  },
  "preferred": {
    "memoryModes": ["session"],
    "inferenceLocation": "local"
  }
}
```

Hard requirements filter candidates before scoring. A high reputation, affinity, or explicit
preference cannot override a missing hard privacy property or unsupported protocol version.

## Ordering and refresh

Capability announcements name the current Tentacle incarnation and travel in an ordered Council
envelope. Updates from an older incarnation or stale sender sequence are rejected. Version 1 has no
independent capability-manifest revision or expiry field; those require an explicit protocol change
before they can be enforced. A `capability refresh` governance action requests re-advertisement but
cannot force a Tentacle to disclose anything forbidden by local policy.

## Capability references

A registry may hold a content hash or locator for intentionally public capabilities. The Council
still validates the resolved document, its provenance, bounds, version, and endpoint association.
Mutable load and liveness remain Council-local and never go on-chain. See
[Identity and registry](identity.md) and [Routing](routing.md).
