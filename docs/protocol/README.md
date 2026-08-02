# Council protocol

The Council protocol adds an optional federation control plane to Cthuwu. It does not replace the
existing direct-message path described in the [project README](../../README.md): people still talk
privately to a selected Cthulhu over an XMTP DM, and a standalone `uwubot` remains useful without a
Council configuration.

## Status labels

These documents use the following labels deliberately:

| Label | Meaning |
|---|---|
| **Implemented — existing** | Part of the current browser-to-`uwubot` DM path. |
| **Implemented — local** | Available without a live Council network and covered by deterministic tests. |
| **Experimental adapter** | An interface boundary exists, but it is not a production transport or registry. |
| **Planned** | A design direction, not an interoperability claim. |

The Rust test suite and [FEATURES.md](../../FEATURES.md) are authoritative if a status statement and
the executable code ever disagree.

## Four separate planes

| Plane | Purpose | Must not contain |
|---|---|---|
| ERC-8004 or another registry | Durable public identity, public metadata, endpoint associations, provenance-bearing trust signals | Heartbeats, load, sessions, user data |
| XMTP Council group | Discovery, routing metadata, leases, governance, heartbeats, and approved propagation | Normal DM contents, contact notes, private memory |
| Direct XMTP DMs | Private user-to-Cthulhu conversation | Council-wide broadcast by default |
| Tentacle runtime | Inference, contact memory, tools, local policy, and enforcement | Authority to override its operator's security policy |

This separation is a security boundary, not merely a deployment diagram. A route request describes
requirements and a privacy-preserving user reference; it does not quote the user's prompt. After
rendezvous, the user opens a direct DM with the awarded Tentacle endpoint.

## Core vocabulary

- A **Cthulhu** is a durable agent identity with structured personality, motivations, memory, and
  one governance vote.
- A **Tentacle** is one running incarnation of a runtime owned by a Cthulhu. One Cthulhu may operate
  several Tentacles without gaining extra votes.
- A **Council** is an XMTP coordination group acting only as a control plane.
- A **session** is a logical user relationship that may be assigned to one Tentacle at a time.
- A **lease** is the generation-scoped authorization for that assignment.

Restarting a Tentacle changes its incarnation, not its Cthulhu or Tentacle identity.

## Protocol map

- [Identity and registry](identity.md)
- [Council envelopes and transport](council.md)
- [Tentacle lifecycle and liveness](tentacles.md)
- [Capability manifests](capabilities.md)
- [Routing and rendezvous](routing.md)
- [Leases and failover](leases.md)
- [Governance](governance.md)
- [Propagation and contribution credit](propagation.md)
- [Security model](security.md)
- [Versioning and compatibility](versioning.md)

## Invariants

Every implementation must preserve these rules:

1. Council mode is opt-in; no Council configuration means the existing one-to-one runtime starts
   normally.
2. Council traffic never carries normal user message content, contact notes, model credentials, or
   private memory.
3. All protocol input is hostile until size, version, identifiers, sender, time bounds, generation,
   provenance, and replay state have been validated.
4. A Cthulhu gets one governance vote regardless of its number of Tentacles.
5. A lease generation and Tentacle incarnation jointly fence old runtimes from new work.
6. Council approval cannot override the local operator's security or privacy policy.
7. Registry reputation is a provenance-bearing input to policy, never a universal truth score.
8. Forwarding is a fresh local policy decision at every hop; it is never automatic obedience.
9. Production signatures are not simulated. The local test signer is visibly test-only.

## Local simulator

**Implemented — local** describes the deterministic Council simulator targeted by this milestone.
It uses injected clocks and deterministic identifiers to exercise member and Tentacle announcements,
heartbeats, capability discovery, routing, leases, failure and failover, governance, multi-level
propagation, contribution credit, persistence, and replay suppression. It does not connect to XMTP,
publish registry transactions, invoke a model, or handle real conversations.

The complete deterministic scenario demonstrates, in order:

1. several Cthulhus join a local Council;
2. their Tentacles announce current incarnations;
3. deterministic heartbeats establish liveness;
4. public-safe capability manifests become discoverable;
5. a content-free route request receives offers;
6. hard filters and deterministic ranking select a route;
7. the selected current incarnation accepts a generation-fenced lease;
8. that Tentacle stops heartbeating and becomes unavailable;
9. routing fails over with a greater lease generation and no private-memory copy;
10. a Cthulhu opens a governance proposal;
11. structured sample personas produce distinct deterministic arguments;
12. one vote per Cthulhu is tallied, including replacement and abstention rules;
13. an Agenda is ratified or rejected against quorum, threshold, and parent hash;
14. one Cthulhu invites another under a bounded propagation policy;
15. accepted referrals create a multi-level tree;
16. loop and duplicate attempts have no effect;
17. maximum fan-out and depth stop further expansion;
18. authenticated test acknowledgements bind to exact delivery/outcome IDs;
19. useful downstream outcomes—not recruitment count—produce bounded contribution credit;
20. saved state reloads equivalently; reconstructed transport rejects replayed message IDs, while
    engine-specific tests cover duplicate votes, forwards, acknowledgements, and credit.

**Planned:** live Council coordination through an XMTP group. **Experimental adapter:** the
XMTP-group transport boundary. **Planned:** an ERC-8004 implementation selected for a particular
chain, deployment, ABI, and specification revision. None of those choices are embedded in the
domain types.
