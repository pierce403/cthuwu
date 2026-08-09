# Guardrail audit for the UWU token phase

Status: reviewed 2026-08-09 against the pre-token Evolution and Council implementation.

This audit distinguishes economic or growth policy from protocol integrity. The UWU token phase
flags broad lifecycle and network controls for deliberate follow-up rather than silently deleting
them while adding balance observation. Several controls are encoded into persisted state,
authorization receipts, protocol messages, and tests; removing them incidentally would create
incompatible state without producing a coherent replacement.

The intended economic baseline is:

- UWU is a transferable ERC-20 intended for Base. The requested supply is 1,000,000,000 UWU, but
  current Clanker v4 documents a fixed standard supply of 100,000,000,000 tokens with 18 decimals.
  Launch must therefore choose Clanker's standard supply or a custom/nonstandard deployment; the
  requested 1-billion supply must not be described as Clanker standard. See the official
  [Clanker v4 deployment configuration](https://github.com/clanker-devco/DOCS/blob/main/references/core-contracts/v4/deployment-config.md).
- A Tentacle needs no minimum stake to start. A per-Tentacle stake requirement may be added later,
  with a permissive default of zero.
- Each Tentacle observes Base independently through its configured RPC endpoint and keeps a local
  cache. There is no central balance registry. Public sender balances affect only tier behavior/
  gating and a bounded Engagement bonus averaged across all period conversations.
- Dust below one UWU is Initiate and excluded from the percentile population. Default Whale and
  Elder labels require at least 100 and 10 eligible local holders respectively; ties share a tier.
- Token observation is read-only. Cthuwu does not hold an operator private key, sign transfers, or
  delay an otherwise authorized economic action.
- The contract address remains configuration until launch. A missing address or unavailable RPC
  disables token-derived behavior without stopping ordinary interaction.

## Remove or redesign

The following are economic, growth, or topology policies rather than minimum integrity checks.

### Economic-scale boundary

At audit start, `AGENTS.md` prohibited token, staking, and financial incentives in the Evolution
layer, while `cthuwu/src/evolution_runtime.rs` restricted runtime scoring to Engagement. The token
phase removes that obsolete blanket prohibition but does not treat a public user's balance as the
Tentacle's economic state.

The current runtime remains Engagement-only. Each fresh/cached public-sender balance supplies at
most one bounded Engagement bonus, accumulated and averaged across every conversation in the period.
This removes last-writer authority: a final high-balance message cannot replace earlier zero or
missing observations or directly grant lifecycle rights. Public balances never enable Wealth,
starvation relief, stake, reward, Growth, or Influence.

`RecordedTokenEconomics` remains an adapter-only library boundary. Enabling its Wealth, starvation,
stake, reward, or emergency-spend policy requires a future cryptographically bound node/operator
source that records holder role/address, chain ID, contract, block, observed time, decimals/supply,
and configuration fingerprint with idempotent history. Keeping those dimensions inactive until such
provenance exists is an integrity requirement, not a prohibited economic delay or identity check.

### Spawn evidence, delay, expiry, and one-use grants

The current path makes Scales a precondition for growth:

- `cthuwu/src/scales.rs` defines daily and weekly propagation evidence floors and refuses to issue a
  final judgment before the period boundary.
- `EvolutionRuntime::spawn_child` in `cthuwu/src/evolution_runtime.rs` requires a final judgment from
  the immediately preceding daily period, at least eight observations, and four prior-day returns.
- `is_accepted_propagation_grant` and `validate_lineage_spawn_authorizations` expire a grant after
  the immediately following period.
- `Lineage::spawn_child` in `cthuwu/src/evolution.rs` consumes a judgment ID once, which makes one
  daily judgment equivalent to at most one spawn.
- `SacredBan::Spawning` in `cthuwu/src/personality.rs` is enforced as an absolute prohibition by
  both the runtime and lineage state machine.

These are artificial growth controls. Scales should evaluate growth and drive later survival or
culling rather than delay creation. A future redesign should remove the daily evidence floor and
period expiry from spawn admission, make hard Nature bans an explicit per-Tentacle option with a
permissive default, and use a unique XMTP event ID or Base transaction/log identifier solely for
idempotency. Nature ID, awakening epoch, parent identity, and provenance bindings remain useful
audit facts and should not be discarded.

### Advisory-only lifecycle and record-only spawning

`cthuwu/src/evolution.rs` returns `LifecycleDecision` recommendations without process control.
`EvolutionRuntime::spawn_child` records lineage but explicitly creates no process, wallet, XMTP
identity, or deployment. Final judgments use `AuthenticatedOperatorConfirmationRequired` in
`cthuwu/src/scales.rs`.

This conflicts with mandatory autonomous Evolution and with the principle that Scales, rather than
fixed network limits, perform culling. Autonomous provisioning and retirement require a separate,
idempotent lifecycle executor and durable action journal. The token-observation phase documents this
gap; it does not pretend that a lineage record already provisions or terminates a runtime.

### Hard effective network ceilings

Several bounded in-memory or single-file models currently reject additional network state:

| Current source | Effective ceiling |
|---|---:|
| `CthulhuIdentity` in `cthuwu/crates/cthuwu-protocol/src/identity.rs` | 32 Tentacles per Cthulhu |
| `Lineage` in `cthuwu/src/evolution.rs` | 4,096 nodes, 256 children per node, generation 1,024 |
| `GovernanceEngine` in `cthuwu/crates/cthuwu-council/src/governance.rs` | 4,096 Council members |
| `LocalRegistry` in `cthuwu/crates/cthuwu-council/src/registry.rs` | 4,096 registered Cthulhus |
| `RoutingEngine` in `cthuwu/crates/cthuwu-council/src/routing.rs` | 1,024 candidates in one decision |
| `PropagationEngine` in `cthuwu/crates/cthuwu-council/src/propagation.rs` | 4,096 cached candidates |

These should become per-page, per-message, per-query, or local-retention bounds rather than global
membership ceilings. Suitable replacements include append-only segmented storage, pagination,
incremental top-k routing, compact membership commitments, and local cache eviction that does not
deny an identity's existence.

### Semantic Council propagation limits

`PropagationPolicy` and `StrictHopValidator` in
`cthuwu/crates/cthuwu-council/src/propagation.rs` enforce maximum depth 16, fan-out 64, 128 items per
sender per rate window, and a 30-day campaign lifetime. These values limit referral reach rather
than merely bound one encoded message.

Depth, fan-out, and sender quotas should be optional policy fields with permissive defaults, or be
replaced by economic admission. Duplicate delivery, loop detection, payload hashes, provenance,
and recipient opt-out remain independent integrity properties. Supporting long paths without large
messages will require a chain commitment or paged provenance rather than an unbounded hop vector.

### Single bootstrap address

`web/src/config.ts` hard-codes one intro Tentacle address. It is currently a single bootstrap and
availability point. It should become one seed among cached peer discovery, multiple operator-supplied
seeds, and signed peer-exchanged discovery. A Base registry contract may be an optional discovery
source, but normal operation should not depend exclusively on one contract, leader, or endpoint.

The live `XmtpGroupCouncilTransport` in
`cthuwu/crates/cthuwu-council/src/transport.rs` is still an explicit unavailable adapter. This is a
functionality gap, not a guardrail; the current simulator does not establish peer-to-peer Council
interoperability.

### Brave SafeSearch

At audit start, `BraveWebSearch::search` hard-coded `safesearch=moderate`. The token phase resolves
that finding with an operator-configurable `off`, `moderate`, or `strict` mode whose default is
`off`. No general topic moderation layer was found in the Rust bot. A hosted inference or search
provider may still apply its own service policy, which Cthuwu cannot override; a local model is
required when that distinction matters.

## Retained integrity and resource controls

The following should not be removed as part of token integration:

- Transport-authenticated XMTP sender binding, signature verification, sequence and incarnation
  fencing, replay suppression, unique message IDs, and stale-message rejection.
- Nonzero ERC-20 contract-address validation, per-call Base chain identification, bounded RPC
  responses/timeouts, per-holder 1–30 second outage backoff, and graceful neutral fallback when Base
  RPC is unavailable. Explicitly disabling observation ignores stale token-only configuration.
- Meaningful local tier sample floors and dust exclusion. These define what “top 1%” and “top 10%”
  mean for one observer; they do not cap holder count, network size, or interaction volume.
- Message, envelope, identifier, collection-page, and persisted-record size bounds. Large state
  should be paged or archived rather than accepted as a single unbounded allocation.
- Atomic persistence, restrictive secret-file permissions, symlink and path-traversal rejection,
  schema versions, and exact state/receipt validation.
- Duplicate and referral-loop suppression, payload/provenance hashes, and recipient-controlled
  opt-out or block lists.
- `EvolutionRuntime` deferral while a public turn is bound to the current Nature and metrics period.
  This is a consistency boundary for mutation/finalization; it should simply stop governing spawn
  admission after the spawn redesign.
- Local transport backpressure and bounded work queues. The fixed in-memory Council sender rate in
  `cthuwu/crates/cthuwu-council/src/transport.rs` should become node-configurable and must not be
  presented as global economic or growth policy.
- Hermes privacy-shape validation in `cthuwu/src/hermes.rs`, which prevents credentials, contact
  notes, wallet/inbox identifiers, and private memory from entering gossip. This protects the data
  boundary and is not conversational content moderation.
- Hermes' bounded direct-peer degree and outbound batches. Peer rotation and cache eviction can let
  the overall network grow without requiring one node to hold every connection or item.

## Identity and coordination result

No KYC or personhood check was found. `cthuwu/src/principal.rs` authorizes a canonical XMTP inbox
through local configuration and fences messages authored before that grant; version 3 explicitly
rejects the older activation-proof state. This is authorization based on an authenticated
cryptographic identity, not KYC.

Council routing's allowlist, registry-association, and reputation requirements are optional in
`cthuwu/crates/cthuwu-council/src/routing.rs`, and their Rust defaults are permissive. They should
remain optional routing preferences. The protocol should add an explicit XMTP-authenticated trust
mechanism so signatures and authenticated transport are sufficient by default. Optional operator
labels and jurisdiction metadata in `cthuwu-protocol/src/identity.rs` are descriptive fields, not
verified identity requirements.

`LocalRegistry`, local token balance caches, local Scales, and local Hermes state are compatible
with peer-to-peer operation. Neither an ERC-8004/Base registry nor a Council leader should become a
mandatory global coordinator.
