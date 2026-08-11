# Security model

Council mode adds unauthenticated-looking network data, distributed clocks, mutable membership,
governance, and referral graphs to an application that already handles private DMs. Every protocol
object is hostile until validated. Enabling Council mode must not weaken the existing launcher,
secret isolation, protected data directory, XMTP identity persistence, contact-note protections,
deduplication, or direct-message limits.

## Assets and boundaries

| Asset | Boundary | Required protection |
|---|---|---|
| XMTP wallet/database keys | Runtime ↔ sidecar/disk | Existing environment allowlist, owner-only atomic persistence, never Council data |
| Model credentials | Rust runtime ↔ provider | Never passed to Council transport or XMTP sidecar; never logged |
| User DMs and contact memory | Direct DM/runtime | Never copied into Council envelopes, routing, governance, or propagation |
| Tentacle identity | Disk/registry/Council | Stable protected local identity; authenticated endpoint association; no separate Cthuwu identity |
| ERC-8004 signing key | XMTP sidecar only | Typed canonical-registry calls, zero value, allowlisted metadata, gas/fee and frame bounds; never Rust/model/frontend/logs |
| Lease authority | Router ↔ Tentacle | Expiry, monotonic generation, incarnation fencing, atomic persistence |
| Governance state | Council members | Version-1 compatibility-principal vote deduplication, parent hashes, deadlines, replay protection; future Tentacle model still unspecified |
| Propagation provenance/credit | Referral graph | Authenticated hops/acks, loop and duplicate suppression, bounded outcome credit |

## Parse and validation discipline

Validation happens before allocation or state mutation wherever possible:

1. Reject an encoded Council envelope over 64 KiB before JSON decoding.
2. Decode into versioned tagged types, not an unbounded generic value that downstream code trusts.
3. Enforce identifier, string, list, map, nesting, capability, argument, path, and provenance limits.
4. Validate protocol/version, timestamps, expiry, message type/payload agreement, sender consistency,
   signature or explicit authenticated-transport policy, membership, and endpoint association.
5. Validate domain freshness: incarnation, sequence, capability ordering, lease generation, Agenda
   parent, campaign policy, provenance path, or vote replacement order. Version 1 has no independent
   capability-manifest revision field.
6. Check stable message-ID replay state.
7. A production coordinator must compute a complete state transition, persist the effect and replay
   marker atomically, then publish any derived output. The local simulator currently proves a
   combined atomic snapshot checkpoint, not a per-message live-transport transaction.

The existing direct DM limit remains 16 KiB. Council limits do not raise it. Parser errors return
bounded reason codes and never echo attacker-controlled bodies into logs.

## Identifier and path safety

Typed protocol IDs are bounded ASCII values with fixed lowercase prefixes; they reject whitespace,
control characters, path separators, ambiguous case, and malformed separators. They are still not
trusted paths. Persistence maps records to fixed directories and storage-selected filenames, rejects
symlinks and path escape, and confirms loaded identity metadata matches its expected key.

Raw XMTP inbox IDs used by the existing contact store retain its stricter lowercase-hex filename
validation. Council identifiers never weaken that contract.

## Replay, ordering, and time

- The simulator snapshot persists stable message IDs and suppresses them when transport state is
  reconstructed; a live coordinator must couple each replay marker to its effect transaction.
- Sequence numbers detect stale/reordered sender updates but do not substitute for replay IDs.
- New Tentacle announcements fence every earlier incarnation.
- Capability updates are fenced by current incarnation and envelope ordering; lease generations are
  monotonic within their session scope. Version 1 has no independent manifest revision.
- Proposal, offer, lease, heartbeat, invitation, campaign, and acknowledgement deadlines use an
  injected clock with bounded skew policy.
- An old heartbeat cannot revive an old incarnation, and an old lease generation cannot accept work.
- Duplicate votes, forwards, acknowledgements, releases, expiries, and contribution outcomes are
  idempotent and cannot create a second effect.

## Sender authentication and signatures

The transport returns an authenticated sender independently of envelope claims. Receivers compare it
with the deprecated coordination-principal/Tentacle association and any required registry/allowlist endpoint association. A mismatch
is rejected before liveness, membership, votes, or credit change.

The signer/verifier abstraction does not imply a deployed production signature scheme. The
deterministic signer is test-only and must be visibly named as such. Unsigned envelopes are limited
to the simulator or an explicit policy relying on transport authentication with understood security
properties. Algorithm selection, key rotation/revocation, canonical bytes, and identity binding must
be specified and tested before a production signature claim.

## Threat controls

| Threat | Control |
|---|---|
| Oversized or deeply nested input | Pre-parse frame bound plus per-field/collection/depth limits |
| Malformed IDs/path traversal | Typed parsing; fixed storage roots; no protocol value used directly as a path |
| Replay/duplicate effect | Stable IDs; combined snapshot replay state locally; per-message atomic coordinator transaction required for live transport |
| Expired route offers or leases | Injected-clock expiry validation before award/work |
| Stale heartbeat/incarnation | Current-incarnation fence and ordered announcement metadata |
| Split-brain lease | Per-session monotonic generation plus incarnation-bound acceptance |
| Duplicate version-1 votes | Vote map keyed by deprecated coordination-principal ID; ordered replacement before deadline; no claim about future Tentacle governance |
| Conflicting Agenda history | Canonical parent hashes and explicit competing-parent state |
| Sender mismatch/fake acknowledgement | Transport sender binding, endpoint policy, exact outcome/campaign binding |
| Announcement spam/referral bomb | Per-sender rate limit, bounded fan-out/depth/collections, expiry |
| Referral loop/duplicate forward | Full path validation and durable campaign/edge dedupe |
| Altered/spliced provenance | Recomputed bounded path hash detects structural alteration after authenticated local admission; production authenticity still requires signed/authenticated hops |
| Sybil credit amplification | Recipient-bound unique outcomes, direct-contributor-only credit, per-outcome/contributor/campaign caps, no raw recruitment credit; no claim of personhood |
| Secret/private-data logging | Structured IDs/reason codes only; no bodies, keys, credentials, contact notes |
| Governance code execution | Closed typed Action enum and final local policy check; no shell Action |

## Sybil limitations

The local registry, ERC-8004, and test signer cannot establish real-world uniqueness. Credit caps and useful-
outcome requirements reduce amplification but do not solve Sybil identity. A deployment must select
acceptable identity and trust provenance, and it should avoid converting contribution credit into
money, governance weight, or unrestricted resources. ERC-8004 reputation is displayed with
provenance—not proof of personhood, membership, default rank, or a global truth score. Exact
allegiance is voluntary, and shared verified wallets receive only one leaderboard position.

## Persistence

Council durable state lives below the already validated `UWUBOT_DATA_DIR`: legacy coordination/Tentacle identity,
membership, capabilities, affinity, leases and generation fences, processed IDs, Constitution,
Agenda history, proposals/votes, propagation/referrals/acks, and contribution events. It uses the
existing owner-only, non-symlink, atomic-write and directory-sync model. Runtime state, generated
identities, Council databases, contact records, and test fixtures containing secrets are never
committed to the repository.

Corruption, environment mismatch, identity mismatch, unsafe permissions, or partial state fails
closed. Backups must preserve the complete data directory securely; copying only part of lease or
identity state can violate recovery assumptions.

The separate `state/erc8004-registration.json` snapshot persists intent before broadcast, the known
transaction and canonical receipt block, selected agent, remaining stages, last verified
wallet/metadata, funding status, notification cooldown, and sanitized failures. Restart reconciles
that state before another write. The signer rejects a wrong chain/registry, arbitrary calldata,
nonzero value, unsupported keys, oversized values, and configured gas/fee ceilings.

## Privacy review checklist

Before adding a Council field, ask:

- Can routing work with a capability or privacy-preserving reference instead?
- Is the field necessary to every recipient at this visibility?
- Can it reveal a user's DM, contact memory, precise location, private endpoint, provider, hardware,
  credentials, or local policy?
- Is its retention and revocation behavior explicit?
- Is it bounded, versioned, authenticated, and covered by replay tests?

If ordinary conversation content is needed, the operation belongs on the direct DM data plane, not
in the Council protocol.
