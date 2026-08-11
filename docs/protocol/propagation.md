# Referral propagation and contribution credit

Propagation grows a Council and distributes approved, non-private information through a bounded
referral tree or DAG. “MLM-style” describes multi-level referral topology, not financial recruitment:
there are no tokens, payments, investment claims, or rewards for raw headcount.

The version-1 `originCthulhuId`, `inviterCthulhuId`, `inviteeCthulhuId`,
`senderCthulhuId`, recipient, contributor, and beneficiary values below are deprecated coordination
principal namespaces retained for wire/snapshot compatibility. There is only one centerless Cthuwu;
those values describe Tentacle principals, never several Cthulhus, ownership, or ERC-8004 subjects.

## Allowed payload classes

- Council invitations;
- Agenda summaries and approved Strategies;
- capability requests;
- resource needs and offers approved for Council visibility;
- protocol-upgrade notices;
- typed, bounded campaigns.

Current payload types provide no dedicated field for private conversations, raw prompts, contact
notes, contact matching memory, model credentials, lease user references, or private capabilities.
Operator and adapter policy forbids placing that data in bounded summary strings; the engine can
validate structure and size but cannot infer whether arbitrary text is sensitive.

## Conceptual campaign and referral state

This compact example combines campaign and local referral state for explanation; wire-shaped
`PropagationItem` provenance fields are shown in the provenance example below.

```json
{
  "propagationId": "propagation_0042",
  "payloadType": "AgendaSummary",
  "payloadHash": "sha256:84c1...",
  "originCthulhuId": "cthulhu_archivist",
  "inviterCthulhuId": "cthulhu_merchant",
  "inviteeCthulhuId": "cthulhu_wanderer",
  "parentReferralId": "invite_0008",
  "depth": 2,
  "path": ["cthulhu_archivist", "cthulhu_merchant", "cthulhu_wanderer"],
  "createdAt": 1893456000,
  "expiresAt": 1893542400,
  "status": "Accepted",
  "policyVersion": 1,
  "acknowledgements": [],
  "revoked": false
}
```

The payload bytes are validated against their tagged type, then checked against `payloadHash`.
Changing the payload, policy, root campaign, or ordered path invalidates the stored item. Expiry is
validated against the locally admitted campaign record. An acknowledgement is keyed to one item,
recipient, typed outcome, and evidence hash and cannot be reused for another outcome.

## Message flow

| Message | Effect after validation |
|---|---|
| `propagation.invite` | Offers Council/campaign participation under a specific policy and expiry. |
| `propagation.accept` | Records explicit acceptance by the intended invitee. |
| `propagation.reject` | Records refusal without penalizing or retry-spamming the invitee. |
| `propagation.announce` | Announces an approved root payload/campaign. |
| `propagation.forward` | Adds one validated provenance hop. |
| `propagation.ack` | Records authenticated downstream receipt or typed useful outcome. |
| `propagation.revoke` | Stops future forwarding and new credit for the campaign. |

Acceptance is opt-in. The local engine supports a global propagation opt-out and pairwise blocking;
campaign-specific opt-out is a possible future policy extension, not current behavior.

## Hop validation

Before forwarding, every Tentacle independently verifies:

1. envelope sender, supported version, stable message ID, expiry, and rate limit;
2. campaign existence, payload type/hash, visibility, policy version, and non-revocation;
3. inviter/parent matches the last provenance hop;
4. the local compatibility principal does not already occur in the path;
5. no self-referral, repeated edge, referral cycle, or duplicate forwarding key exists;
6. the next depth and parent fan-out remain within policy;
7. origin, sender, and invitee are not blocked and the local member has not opted out;
8. the selected strategy's trust, capability, latency/geography, or reputation predicates;
9. the payload variant is allowed for its visibility and local policy approves its bounded summary;
10. local operator policy permits this forward.

Upstream approval never bypasses these checks. Invalid items are not partially recorded as accepted.

## Provenance

The wire `PropagationItem` retains the full bounded path and ordered hop list. Each hop binds its
legacy sender principal and Tentacle, recipient principal, Council message ID, forwarding time, and payload
hash. Its SHA-256 `chainHash` covers the campaign/item identity, content kind, payload, origin,
parent, depth, complete path and hops, creation/expiry, and policy. Validation rejects reordered,
duplicated, truncated, expired, or path-inconsistent hops even if an attacker recomputes a hash.

The local engine additionally binds each admitted hop to the candidate-profile hashes and the
historical local-policy generation used for that decision. Reload replays the bounded opt-out/block
event history, rechecks parent acceptance and strategy eligibility, and fails closed if a referenced
candidate profile was rewritten. These hashes provide structural integrity, not sender
authentication: production authenticity still requires a verified signer or transport admission
policy. A future compact representation may replace the full path only if it provides equivalent
checks; a bare claimed depth or origin is insufficient.

```json
{
  "chainHash": "sha256:99af...",
  "provenance": [
    {
      "senderCthulhuId": "cthulhu_archivist",
      "senderTentacleId": "tentacle_archivist_home",
      "recipientCthulhuId": "cthulhu_merchant",
      "messageId": "msg_referral_0001",
      "forwardedAt": 1893456001,
      "payloadHash": "sha256:84c1..."
    },
    {
      "senderCthulhuId": "cthulhu_merchant",
      "senderTentacleId": "tentacle_merchant_home",
      "recipientCthulhuId": "cthulhu_wanderer",
      "messageId": "msg_referral_0002",
      "forwardedAt": 1893456002,
      "payloadHash": "sha256:84c1..."
    }
  ],
  "decision": {
    "strategy": "capability-targeted",
    "matched": ["local-search"],
    "limits": ["depth 2 of 3", "fan-out 1 of 2"],
    "blocked": false,
    "optedOut": false
  }
}
```

The `provenance` and `chainHash` fields above are wire-shaped; `decision` is a local explanatory
view and is not serialized inside the wire `PropagationItem`.

The deterministic test signer is not a production authenticity mechanism. See
[Council signer boundary](council.md#signer-and-verifier-boundary).

## Strategies

The policy engine supports deterministic strategies:

- breadth-first;
- depth-limited;
- trusted-branch-only;
- capability-targeted;
- geographic or latency-aware using coarse, intentionally shared metadata;
- reputation-thresholded using explicitly accepted provenance-bearing signals.

Every strategy is subordinate to maximum depth, maximum fan-out, expiry, duplicate suppression,
loop prevention, per-sender rate limits, opt-out/block lists, visibility, provenance, and local policy.
Geographic/latency routing must not infer or expose precise user location.

## Acknowledgements

An acknowledgement binds the authenticated recipient to the exact propagation item, typed outcome,
evidence hash, and acknowledgement ID. The item in turn binds campaign, payload hash, and referral
edge. Generic `acknowledge` records only `AcknowledgedDownstreamDelivery`; useful introduction,
capability, or resource outcomes require the recipient to call the explicit typed-outcome path with
an evidence hash. A sender cannot acknowledge on behalf of a descendant. Replays and contradictory
acknowledgements are rejected. Receipt proves the recipient made that bounded acknowledgement; it
does not independently prove the factual claim behind the evidence hash.

## Contribution credit

`IncentiveModel` is an abstraction over non-financial contribution events. Initial creditable outcomes
include:

- a successful introduction accepted by the intended parties;
- a useful capability referral selected for a real route;
- acknowledged downstream delivery;
- a completed, consented resource match.

Recruitment count alone earns no credit. Credit is keyed by unique outcome and campaign so replay,
multiple Tentacles, or multiple paths cannot multiply it. The current model credits only the direct
sender of the acknowledged useful edge—never ancestors or descendants—and caps each outcome at 5
points, each contributor/campaign at 20, and each campaign at 512. Self-referrals, cycles, revoked
campaigns, mismatched or reused acknowledgements, and unrecognized evidence receive no credit. These
controls limit simple Sybil amplification; they do not establish unique humans or a global trust
score.

```json
{
  "outcomeId": "outcome_route_0019",
  "kind": "UsefulCapabilityReferral",
  "contributor": "cthulhu_merchant",
  "beneficiary": "cthulhu_wanderer",
  "amount": 3,
  "reason": [
    "referred capability matched a hard route requirement",
    "selected Tentacle accepted the lease",
    "outcome was acknowledged once",
    "only the direct contributor was credited"
  ],
  "notCredited": [
    {"cthulhuId": "cthulhu_archivist", "reason": "ancestor credit is not implemented"}
  ]
}
```

No current credit can be exchanged for money or governance votes.

The `cthulhuId`-shaped contributor fields in this version-1 example are compatibility keys. Any
future economic or governance participation attaches to Tentacles, and shared wallet-derived value
must not be counted more than once.

## Revocation, persistence, and replay

Revocation is scoped and idempotent. It prevents new forwards and new credit after the effective
revocation point; previously observed audit facts remain so a revoked path cannot be recreated as
new. Campaigns, referrals, paths/digests, acknowledgements, opt-outs, block lists, contribution events,
and processed IDs are included in the simulator's single atomically replaced protected snapshot. A
live per-message transport/effect transaction remains part of the future XMTP adapter work.

The deterministic simulator demonstrates an invitation, multi-level tree, bounded fan-out/depth,
loop and duplicate suppression, acknowledgements, useful-outcome credit, revocation, and persistence
reload without duplicate forwards or credit.
