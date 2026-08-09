# Local Evolution layer

## Status

The Evolution layer is implemented as local Rust state machines and owner-only persistence. It runs
inside a standalone Tentacle; Council membership is optional and direct one-to-one XMTP DMs remain
the default data plane.

The current implementation does **not** provide a live Hermes transport, peer discovery/handshake,
peer-key provisioning, Council metrics publication, automatic process spawning/death/absorption, or
token transaction execution. It does provide the local, read-only UWU observance and bounded
Engagement input described in [token.md](token.md). Local deterministic gossip and token tests
do not establish live network or deployed-contract interoperability.

## Components and authority

| Component | Implemented boundary | Authority boundary |
|---|---|---|
| Nature | Seven bounded sliders, one Sacred Ban, random identity, generation, inheritance/mutation, rendering, local signed persistence | Policy data only; cannot authorize a role, tool, provider, or Council action |
| Awakening | Signed, hash-chained epochs with restart recovery and active-operator actions | Existing authenticated XMTP operator classifier; the journal does not authenticate senders |
| Scales | Bounded daily/weekly aggregate metrics, Nature/epoch/period bindings, stress, partial/final judgments, and logically append-only history | Outcomes are recommendations; authenticated operator confirmation remains required |
| UWU observer/economics | Read-only Base `balanceOf`, local tier policy, and a bounded public-sender Engagement bonus averaged over all period conversations | No private keys, transfers, central registry, operator authority, Tentacle Wealth/starvation/stake/reward state, or automatic spending |
| UWU governance core | Deterministic bounded address ballots, tier/holding weights, quorum, and approval for closed policy subjects | Library-only/advisory; no live Council, Nature mutation, persistence, RPC, process, or operator authority |
| Lineage | Founder/child/family/absorption records, identity binding, cycle checks, atomic persistence | Records do not create, kill, route, or merge a process |
| Hermes core | Closed knowledge types, HMAC author/relay provenance, per-peer anti-entropy, conflict resolution, pending retries, persistence | Requires an authenticated transport-to-key binding that is not implemented yet |

Nature and Council personality are deliberately separate. Council personality describes a durable
Cthulhu's bounded governance policy. Nature describes one local Tentacle's innate response and
resource preferences.

## Nature and awakening

Nature records four appetites—engagement, growth, wealth, and influence—and three methods—
cooperation, stability, and transparency—on inclusive 0–100 scales. The closed Sacred Ban set is
recruitment, spawning, governance, profit, or memory sharing. A child retains its parent Nature ID
and a strictly greater generation; inheritance selects bounded similarity, drift, or radical
mutation with an exact 70/20/10 selection split.

`state/nature.json` is a canonical HMAC envelope. The same owner-only local key authenticates
awakening audit entries. HMAC detects changes made without that key; it is not an asymmetric/public
signature, does not identify a peer over a network, and cannot protect state after compromise of the
service account that can read and reuse the key.

The signing key is created atomically and never rotated implicitly. If it is missing while a Nature
snapshot, awakening journal, signed Hermes state, or orphaned metrics/history/lineage projection
already exists, startup fails with recovery guidance instead of adopting that state under a new key.
Rust also holds `state/evolution-runtime.lock` for the lifetime of one Evolution runtime so two local
writers cannot mutate one data directory concurrently.

For a new epoch, Rust classifies and pins the authenticated XMTP sender before parsing ritual text.
Normal public conversation, contact mutation, inference, and tools remain gated until an active
operator supplies one action:

```text
YES
ADJUST <trait> <delta>
REROLL
KILL
```

`state/awakening_log.md` stores normalized actions, timestamps, authenticated full operator IDs, and
hashes of opaque message/event IDs. It does not store the original DM body. The signed hash chain is
logically append-only: each update first verifies the complete chain, then writes one canonical
newline-terminated replacement through an atomic copy-on-write step. This supports recovery when
the journal and Nature snapshot cross a crash boundary without treating a torn final line as an
entry. Forced rerolls start a new epoch rather than truncating prior history. `KILL` records a
terminal request and keeps normal work closed; it never terminates the OS process.

Post-confirmation changes are recorded as signed `POST_ADJUST` entries. On restart and after a
transition, Rust derives the exact current-period stress count from matching entries in that signed
chain, so a crash between the audit write and metrics snapshot cannot silently lose or double the
penalty. While awakening is still pending, an expired metrics period must be empty; it is reset to
the current period without creating a judgment. A late `YES` therefore cannot manufacture a final
result from time spent behind the gate.

Every signed awakening entry carries the resulting Nature and its exact immediate-predecessor Nature
snapshot. Recovery accepts the journal head, or only the final entry's signed predecessor in the
deliberate log-ahead crash window. A different Nature snapshot is rejected even when it is itself a
valid HMAC envelope; independently valid but divergent Nature/log backups must be restored as one
consistent set. With an empty journal, a missing Nature is generated only when no Evolution
projections or alternate Nature exist; otherwise startup requires a consistent restore instead of
rebinding an established node to a new identity.

Local startup controls are:

| Option | Effect |
|---|---|
| `--show-nature` | Open and reconcile Evolution state, render Nature/awakening status, and exit; it is not a read-only file inspector and conflicts with skip/reroll mutators |
| `--nature-path <path>` | Use a non-empty relative path resolved below `UWUBOT_DATA_DIR/state/natures/`; absolute paths and `..` are rejected |
| `--reroll-nature --force` | Generate a new candidate in a new immutable epoch |
| `--skip-awakening` | Record a signed local testing override, visibly distinct from XMTP confirmation |
| `--gossip-peers <list>` | Supply untrusted bootstrap peer IDs; it does not provision keys or connect a transport |

After confirmation, Nature can set a bounded inference temperature, response-size/resource bias, and
response emphasis. It can also update bounded local loyalty and Nature-affinity observations in a
contact note. Those are node observations, not user assertions, and they are excluded from the
profile sent to a remote model. At most one relationship/Scales observation is accepted per contact
per UTC day; a conversation counts as returning only when that retained contact was observed on a
prior day, not merely earlier on the same day. Only aggregate anonymized patterns may enter Hermes;
raw contact values and identifiers never do.

Every public inference turn reserves the exact signed Nature fingerprint, awakening epoch, and
metrics-period bounds before the runtime releases its local mutex for a remote call. Nature mutation
and period rollover/finalization are deferred until all reservations for that binding finish. An
observation is committed only if its reservation still matches those values.

## Scales and lineage

The Scales core can represent bounded aggregate engagement, growth, optional economic efficiency,
and influence measurements. Persisted metrics and judgments bind the Nature ID and fingerprint,
awakening epoch, period bounds, and scored-scale availability. Nature appetites are normalized only
across the active scales; unavailable scales keep zero outcome weight rather than depressing or
inflating the result. The current runtime scores Engagement only. A fresh or unexpired cached
public-sender UWU balance adds one bounded Engagement bonus for that conversation. The period stores
the sum and averages it across every conversation, including those without a usable wallet
observation; ordering and last-writer state therefore cannot determine a lifecycle result. Public
balances never activate Tentacle Wealth, starvation relief, stake, reward, Growth, or Influence.
Post-confirmation Nature adjustments add the bounded, visible, audit-reconciled stress penalty.

The deterministic `JudgmentPolicy` persists propagation evidence floors, and each `Judgment`
persists the observed and required counts plus eligibility. Daily policy requires eight observations
and four prior-day returns; weekly policy requires 32 and 16. Even a score above the propagation
threshold is capped at `Survival` when its sample is below those floors, so score alone cannot yield
`PropagationRights`.

The evaluation boundary is strict:

| Period state | Evaluation status | Execution status | May grant spawn rights? |
|---|---|---|---|
| Open | `PartialSnapshot` | `AdvisorySnapshotOnly` | No |
| Closed | `Final` | `AuthenticatedOperatorConfirmationRequired` | Only after local operator/policy checks |

The four score labels are propagation rights, survival, starvation warning, and death. None is an
effect by itself. In particular, death does not exit `uwubot`, absorption does not copy private
memory, and starvation does not silently reduce or reroute user service. Final facts are logically
appended to `state/evolution_history.jsonl` by verifying and atomically replacing the canonical
newline-terminated journal; current bounded counters live in `state/metrics.json`. A partial
snapshot is never persisted as a grant. History accepts only deterministic `Final` records evaluated
exactly at the period end. It rejects duplicate judgment IDs, conflicting records for one Nature
period, reordered periods, and overlaps. These are unkeyed structural and content-consistency checks,
not cryptographic tamper evidence; unlike the awakening chain, the judgment history has no HMAC.
At startup, the open metrics snapshot is cross-validated against the last Final history record.
Overlap fails closed except for exact equality with that finalized metrics payload—the one supported
history-ahead crash case where journal append committed before metrics reset—which is replayed by
advancing to an empty current period.

Lineage persists founder, child, generation, lifecycle, spawn, and absorption facts in
`state/lineage.json`. `/spawn` requires an authenticated operator and a final propagation-rights
record from the exact current scoring policy, Nature ID/fingerprint, awakening epoch, and a closed
period with at least eight daily contact observations and four prior-day returns. The lineage record
stores the authenticated operator ID, a hash of the transport event ID, and the final judgment's
content-derived ID; that judgment ID can authorize only one spawn. Creating the record verifies the
parent claim and Nature inheritance. It produces a candidate Nature and auditable record only: an
operator must separately provision credentials, authorize an XMTP identity, start a process, and
bind that live identity to the record. The same rule applies to retirement and
absorption—recording intent or destination is not proof that a process or memory operation occurred.
The grant is valid only during the immediately following metrics period. Closing that period, or
skipping past it after missed cycles, invalidates the grant even if it was never consumed.
Startup also verifies every persisted spawn receipt: its judgment ID must resolve to the exact
`Final` `PropagationRights` history record, its parent Nature must match that record, and its spawn
timestamp must fall within the immediately following period. An unverifiable lineage projection is
rejected rather than trusted because its own schema is well formed.

Recruitment is at most an aggregate local metric. It mints no token and produces no stake,
governance vote, ancestor reward, or Council contribution credit. Existing Council contribution
credit remains non-financial, outcome-based, and direct-only. UWU balance observation is independent
of recruitment.

## UWU observance boundary

The sidecar obtains an optional EVM address from the SDK-authenticated XMTP sender inbox; it does not
accept a wallet claim from message text. Each Tentacle validates Base chain ID `8453`, calls the
configured transferable UWU ERC-20 with read-only `eth_call` `balanceOf(address)`, and ranks only its
own in-process observations. Whale, Elder, Acolyte, Initiate, and Unproven are therefore local views,
not identities or global reputation facts. The current integration observes public one-to-one DM
senders; Council-member, sibling-lineage, and operator-acolyte enumeration awaits authenticated live
address adapters.

Holdings below one whole UWU are Initiates and do not enter percentile ranking. Default Whale top 1%
requires at least 100 eligible local holders and Elder top 10% requires 10; otherwise holdings of at
least one UWU remain Acolytes. Ties receive the same tier without address-order tie-breaking.

Unknown and stale observations are neutral. They cannot gate a response, change its depth, or affect
Scales, and an RPC outage does not stop ordinary conversation. A known tier changes bounded response
depth/tone in proportion to `100 - Nature.cooperation` unless an operator sets an explicit 0–100
intensity. The permissive minimum tier is `unproven`; no token or stake is required to start a
Tentacle. Token tier never grants the XMTP operator role or tools.

For current public messages, a fresh/cached balance affects only tier response/gating plus the
period-averaged Engagement bonus. `RecordedTokenEconomics` remains an adapter-only library API for
future node/operator Wealth, starvation, stake, reward, and emergency-spend evidence. Before such
evidence can reach runtime state it must cryptographically bind holder role/address, chain ID,
contract, block, observed time, decimals/supply, and configuration fingerprint, with idempotent
history instead of last-writer state. No such source exists today; those dimensions remain inactive
and emergency spending remains recommendation-only. See [token.md](token.md) for configuration and
the requested one-billion supply versus current 100-billion Clanker v4 standard.

The observer rejects the zero contract and revalidates Base chain ID before every balance call.
Failed ordinary observations use a per-holder retry backoff bounded to 1–30 seconds without blocking
unrelated holders. Disabling observation ignores stale token-only configuration rather than making
it a startup requirement.

The token-governance module is likewise local and advisory. It can deterministically tally one
bounded ballot per address for closed Nature, Council, economic, and skill-propagation policy
subjects, with Nature-scaled tier multipliers and bounded quorum/approval. Nothing currently feeds
those results into a live Council or Nature transition, and the module has no storage, network,
signer, process, command, or operator-authority surface.

## Hermes-inspired knowledge exchange

Hermes is an architecture pattern embedded in every Tentacle, not a central agent and not an XMTP
traffic router. Each node maintains its own direct peer map, known digests, bounded pending outbound
work, and local knowledge. Anti-entropy compares summaries, requests or offers missing records,
retries until acknowledgement, and resolves conflicts by configured signature authority, timestamp,
then digest.

The allowed knowledge schema is closed:

| Allowed | Explicitly excluded |
|---|---|
| Aggregate anonymized interaction categories/counts | Raw DMs, inbox/wallet/contact IDs, emails, or contact notes |
| Bounded conversation strategy outcomes | Private memory, profiles, or prompt transcripts |
| Tool-operation classes and aggregate success counts | Paths, arguments, shell commands, output, or credentials |
| Bounded operator-created skill text, filtered for common private-data shapes | Any assumption that the prose is safe; automatic installation, prompt injection, or tool authorization |

Authorship and relay envelopes use configured HMAC identities and a local trusted-key ring. A peer ID
must also be bound to the actual authenticated transport sender; persisted IDs or valid-looking tags
alone are insufficient. The current repository has no live transport, authenticated handshake,
discovery service, or peer-key provisioning mechanism. `state/hermes_gossip.json` therefore persists
the core's peer/digest/knowledge/pending state but does not establish a connected network and does not
contain signing secrets.

A Nature with the memory-sharing Sacred Ban may receive verified knowledge but emits neither
envelopes nor digest summaries. Operator-created skills received from any peer remain inert untrusted
data. They must be reviewed by a local authenticated operator and separately admitted through the
existing compiled, create-only skill checks before they can influence operator context; they never
become public or operator model tools automatically.

## Authenticated operator interface

Only the active operator lane handles these commands:

| Command | Local result |
|---|---|
| `/nature` | Render Nature and awakening state |
| `/adjust <trait> <value>` | Set a confirmed trait to an absolute bounded value; append audit and stress |
| `/lineage` | Render bounded parent/child/sibling records |
| `/metrics` | Render current bounded aggregate metrics |
| `/judgment` | Return an advisory open-period snapshot or a final recommendation |
| `/spawn [child-id]` | Create a child Nature/lineage record only when final rights and Sacred Ban policy permit |
| `/gossip-status` | Report local core/persistence status without claiming live connectivity |
| `/share-skill <name>` | Stage bounded locally reviewed skill text in Hermes state; no delivery claim |
| `/request-skill <name>` | Inspect locally held gossip knowledge; no live network-query claim |
| `/recovery-status` | Describe a sticky fail-closed Evolution transition; available only after a persistence error |

Public, stale, revoked, Council, and stdin-harness inputs cannot reach these handlers. No command
changes the existing rule that role classification happens before text interpretation.

Evolution transitions can span several owner-only snapshots. If a write fails after an earlier
snapshot may already have committed, the in-memory runtime enters a sticky fail-closed mode: public
work and operator effects remain blocked, with only bounded status commands available. Restart runs
signed recovery; if that cannot reconcile the canonical journals and snapshots, the operator must
restore a consistent backup. An error receipt therefore does not claim that nothing was persisted.

## Persistence and verification

Default Evolution state lives below `UWUBOT_DATA_DIR/state/`:

```text
evolution-signing.key
evolution-runtime.lock
nature.json
natures/<custom-relative-path>
awakening_log.md
metrics.json
evolution_history.jsonl
lineage.json
hermes_gossip.json
```

The signed awakening journal and unkeyed judgment history preserve append-only logical history
through canonical atomic copy-on-write replacements. State files and journal entries are bounded,
owner-only, and reject symlinks or inconsistent bindings. The judgment history provides
deterministic consistency validation, not cryptographic tamper evidence. A missing Evolution key
beside signed state fails closed without rekeying.

The balance/tier cache itself is local in-process state, not a durable global registry.
`metrics.json` may contain the sum of bounded public-sender Engagement bonuses and the full
conversation denominator. The current runtime leaves `token_economics` and Wealth absent; stale or
unknown state contributes zero and cannot enable a Scale.

The Rust suite covers generation/mutation ranges, Sacred Bans, signed Nature tamper detection,
awakening parsing/provenance/audit/recovery, bounded Scales math and partial/final status, inference
reservations, one-use spawn grants, lineage identity/cycle checks, ERC-20 ABI/RPC/chain validation,
local tier/sample-floor/cache/backoff behavior, period-averaged Engagement persistence, and Hermes
signatures/privacy/conflicts/partition convergence/persistence.
Remaining release evidence is tracked in [FEATURES.md](../FEATURES.md), especially live XMTP
awakening, cross-platform persistence, separately provisioned child lifecycle, operator-reviewed skill
activation, and a live authenticated gossip adapter with peer-key lifecycle.
