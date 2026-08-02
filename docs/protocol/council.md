# Council transport and envelopes

A Council is an optional coordination group. Its traffic is control-plane data; normal user messages
stay in [direct XMTP DMs](README.md#four-separate-planes).

## Status

- **Implemented — local:** validated envelopes, tagged message types, replay tracking, a deterministic
  in-memory transport, and test authentication.
- **Experimental adapter:** an XMTP-group adapter boundary.
- **Planned:** live XMTP Council-group interoperability and production sender verification.

The in-memory transport is not evidence that XMTP group transport works.

## `CouncilTransport`

The transport abstraction supplies:

- bounded `publish` and `subscribe` operations;
- authenticated sender identity separate from payload claims;
- stable transport message IDs;
- replay metadata;
- ordering metadata such as transport sequence or generation.

Transport authentication does not replace envelope validation. A consumer checks the authenticated
sender against the envelope and local membership/registry policy before applying any effect.

## Common envelope

All Council messages use one versioned envelope and a tagged payload internally.

```json
{
  "protocol": "cthuwu-council",
  "version": "1.0",
  "messageId": "msg_000042",
  "messageType": "tentacle.heartbeat",
  "councilId": "council_local",
  "senderCthulhuId": "cthulhu_archivist",
  "senderTentacleId": "tentacle_archivist_home",
  "sentAt": 1893456042,
  "expiresAt": 1893456102,
  "sequence": 42,
  "payload": {
    "update": {
      "tentacleId": "tentacle_archivist_home",
      "owner": "cthulhu_archivist",
      "incarnation": {
        "id": "incarnation_0002",
        "generation": 2
      },
      "lifecycle": "ready",
      "health": {
        "status": "healthy",
        "observedAt": 1893456042
      },
      "currentLoadPerMille": 250,
      "lastHeartbeat": 1893456042
    }
  },
  "signature": null
}
```

`signature: null` is acceptable only in the local simulator or under an explicitly configured
authenticated-transport policy. It is not a fake production signature.

## Message types

The version 1 tagged enum contains exactly these families:

| Family | Types |
|---|---|
| Membership | `council.member.announce`, `council.member.withdraw` |
| Tentacles | `tentacle.announce`, `tentacle.capabilities`, `tentacle.heartbeat`, `tentacle.draining`, `tentacle.withdraw` |
| Routing | `route.request`, `route.offer`, `route.reject`, `route.award` |
| Leases | `lease.grant`, `lease.renew`, `lease.release`, `lease.revoke`, `lease.expired` |
| Governance | `governance.proposal`, `governance.argument`, `governance.vote`, `governance.ratified`, `governance.rejected` |
| Propagation | `propagation.invite`, `propagation.accept`, `propagation.reject`, `propagation.announce`, `propagation.forward`, `propagation.ack`, `propagation.revoke` |

Unknown message types are rejected rather than coerced into a generic action.

## Validation order

An implementation validates before dispatching side effects:

1. Reject an encoded frame over 64 KiB before parsing and bound nested strings/collections after
   parsing.
2. Require protocol name `cthuwu-council` and an explicitly supported version.
3. Parse every typed identifier and non-negative Unix-seconds timestamp; require
   `sentAt < expiresAt`, `sequence > 0`, and a non-expired item under the injected clock, with only
   bounded clock skew.
4. Require a known `messageType` and decode the matching tagged payload.
5. Check Council membership and sender Cthulhu/Tentacle consistency when the integrating
   coordinator has configured that local membership/registry policy.
6. Verify the signature or the explicitly selected authenticated-transport policy.
7. Check incarnation, sequence/generation, and domain-specific freshness.
8. Reject a previously processed stable message ID.
9. Apply the state transition and persist replay state with the effect in one durable transaction.
   The local simulator demonstrates a combined atomic snapshot checkpoint; a per-message production
   coordinator transaction remains to be implemented with the live transport.

A validation failure has no partial effect and does not advance a sequence counter.

## Sender and ordering rules

- A production coordinator must prove that `senderTentacleId` belongs to `senderCthulhuId` using
  validated local membership/registry state. The in-memory transport only compares its trusted
  caller-supplied test identity tuple with the envelope; it is not an ownership registry.
- A Cthulhu-level governance vote is attributed to the Cthulhu, not multiplied by its Tentacles.
- Per-sender sequence numbers may detect stale or reordered updates, but they do not replace stable
  message-ID replay suppression.
- Domain generations take precedence over arrival order: an old lease generation or old Tentacle
  incarnation remains stale even if its message arrives later.
- In the local simulator, reconstructing transport state from the combined snapshot rejects persisted
  message IDs as replays. A live dispatcher must provide the same property transactionally.

## Signer and verifier boundary

The protocol exposes signer/verifier abstractions over canonical envelope bytes. The deterministic
test signer is clearly named and scoped to tests. A production implementation must define key
binding, canonicalization, algorithm agility, rotation, and revocation before enabling signatures.
Until then, the XMTP adapter may rely only on sender identity the chosen XMTP SDK actually
authenticates, plus local endpoint-association policy; documentation must not claim cryptographic
properties it has not tested.

## Privacy example

A valid route request may say that a session requires local inference, a filesystem-search tool,
and a 32k context. It must not include the user's prompt, contact note, hopes, resource offers,
model key, or prior DM transcript. See [Routing and rendezvous](routing.md).
