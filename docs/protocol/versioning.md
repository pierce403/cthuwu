# Versioning and compatibility

The Council protocol is independently versioned from the `uwubot` application, XMTP SDK, registry
adapter, and individual document schemas.

## Current protocol

The envelope protocol name is exactly `cthuwu-council`. The initial supported version is `1.0`,
represented by a structured `ProtocolVersion { major, minor }`, not a floating-point number.

```json
{
  "protocol": "cthuwu-council",
  "version": "1.0",
  "messageType": "council.member.announce"
}
```

For this milestone, receivers accept exactly `1.0`. A syntactically valid `1.1`, `2.0`, missing
version, or alternate protocol name is unsupported until explicitly implemented and enabled.

## Compatibility rules

- A **major** version change may alter required semantics, validation, canonicalization, security
  assumptions, or state transitions and is incompatible by default.
- A **minor** version may add explicitly negotiated behavior without weakening version 1 invariants.
  Receivers still accept it only after implementation and tests; they do not guess compatibility.
- Payload documents such as personality, capability manifests, governance documents, and propagation
  policies carry their own integer schema/policy version where their evolution requires it.
- Unknown message types and unknown Action variants are rejected. They are never downgraded to text
  or a generic executable action.
- Fields that affect hashes or signatures require defined canonical treatment. A receiver must not
  sign one interpretation and execute another.

## Negotiation

Tentacle capability manifests list all explicitly supported Council versions. Routing computes the
intersection with the request and Council policy as a hard requirement. Highest-looking versions are
not automatically preferred if local policy has not enabled them.

```json
{
  "requestSupported": ["1.0"],
  "tentacleSupported": ["1.0", "2.0"],
  "councilAllowed": ["1.0"],
  "selected": "1.0"
}
```

No intersection means route rejection with a bounded `unsupported-protocol` explanation. It must not
silently fall back to a version that drops privacy, signature, generation, or provenance checks.

## Upgrade flow

1. Specify message/schema changes and security invariants in these docs.
2. Implement decoding, validation, state transition, downgrade protection, and golden test vectors.
3. Add mixed-version tests and persistence migration/reload tests.
4. Publish a bounded protocol-upgrade propagation item with provenance and expiry.
5. Let operators explicitly enable the new version; Council governance cannot force it.
6. Advertise support only after the implementation passes tests.
7. Route to the intersection while the migration window is active.
8. Retire an old version only under local policy with a documented recovery path.

A protocol-upgrade campaign is notice and coordination, not code distribution or execution.

## Canonical hashes and signatures

Agenda parent hashes, governance document hashes, propagation payload hashes/hop digests, and future
signatures must use a specified canonical byte representation. Pretty-printed JSON, map iteration
order, locale-sensitive formatting, and floating-point values are unsuitable. Until production
canonicalization and key binding are selected and tested, only the clearly marked deterministic
test signer is implemented; no production signature claim is made.

## Persistence migrations

The simulator's combined snapshot carries a schema version and rejects incompatible state. Explicit
migrations for independently persisted production records are planned; they must be bounded and
atomic, and unknown newer state must fail closed rather than be partially loaded. Replay markers and
greatest lease generations must survive any future migration unchanged so an upgrade cannot reapply
old effects or revive stale authority.

## Existing deployment compatibility

Council mode is opt-in. With no Council configuration:

- the existing static browser creates/loads its persistent identity;
- direct browser-to-`uwubot` XMTP DMs continue to work;
- `uwubot` keeps its persistent sidecar identity, contact notes, model adapters, deduplication,
  launcher/data-directory protections, and current commands;
- no Council group, registry, simulator, heartbeat, routing, governance, or propagation process is
  required.

The local Council simulator does not establish live XMTP interoperability. The XMTP-group boundary
remains **experimental**, live Council transport remains **planned**, and the ERC-8004 adapter remains
isolated until a deployment and compatible revision are explicitly configured and tested.
