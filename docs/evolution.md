# Evolution runtime

Evolution gives each Tentacle a persistent Nature, an awakening epoch, Scales measurements, binding
lifecycle judgments, lineage, active UWU economics, and local Hermes anti-entropy state.

The implementation is local-first and receipt-driven. It does not yet contain peer-to-peer Council
discovery, a production XMTP Council-group transport, live Hermes transport, deployed UWU contract,
transaction signer, authenticated revenue source, persisted ballot adapter, payout/application
executor, external child provisioner, or absorption service. Council discovery must not depend on a
mandatory leader. Configuration and deterministic local records do not prove external effects.

## Components and authority

| Component | Role | Execution boundary |
| --- | --- | --- |
| Nature | Seven sliders, Sacred Ban, identity, generation, inheritance/mutation, signed persistence | Influences personality and economic/lifecycle policy; never authenticates an operator |
| Awakening | Local default activation, optional authenticated controls, epochs, signed append-only journal, crash reconciliation | Opens public operation without requiring an operator |
| Scales | Daily/weekly aggregate metrics, policy bindings, stress, open snapshots, final judgments | Final judgments bind lifecycle transitions |
| TokenEye | Per-Tentacle Base observations and local percentile tiers | Missing/stale observations block token-dependent work |
| Economics | Bound treasury, stake, reward, spend, and revenue records | Directly drives Wealth, starvation, Influence, Growth, survival, and propagation |
| Token governance | Holding/stake-weighted ballots for closed subjects | Accepted results return binding dispositions/application records; no persisted/live adapter is committed |
| Lineage/lifecycle | Founder, parent/child, absorption, provisioning, death, and receipt history | Durable intents invoke configured external executors |
| Hermes | Signed, bounded anti-entropy knowledge state | No live transport or peer-key provisioning is committed |

## Nature and awakening

A Nature contains the seven bounded sliders:

- sociability;
- curiosity;
- ambition;
- loyalty;
- independence;
- cooperation/competition;
- growth.

Nature changes are tied to a signed awakening epoch and exact predecessor state. The HMAC key is a
local integrity secret, not a public signature and not protection from an attacker controlling the
`uwubot` OS account. Startup refuses missing or inconsistent signed state rather than regenerating a
new identity over existing Evolution records.

Normal startup accepts a generated Nature locally through a signed `ACCEPT DEFAULT NATURE` entry.
Fresh and legacy-pending nodes therefore open public conversation without an operator. An optional
authenticated operator may inspect and adjust Nature later. `--skip-awakening` remains a distinct
local test override; forced rerolls create a new audited epoch which the runtime accepts by default.

Nature is policy, not authority. It cannot turn a public sender, Council message, token holder, or
skill into an operator or shell command.

## Scales and finality

The Scales combine:

- Engagement from entity-scoped interactions and tier behavior;
- Growth from successful operation, spawns, and accepted reward records;
- Wealth primarily from the bound Tentacle treasury;
- Influence from freshly observed configured stake and eligible governance participation.

Weights renormalize across available inputs. An open-period snapshot is provisional and cannot
trigger a lifecycle transition. At the exact period boundary, the runtime writes one deterministic
final judgment bound to Nature ID/fingerprint, awakening epoch, policy, period, evidence provenance,
and token configuration.

Scales counters have no artificial policy ceilings. Count fields saturate only at `u32::MAX` and
accumulated totals at `u64::MAX`; bounded per-sample inputs and persistence-integrity limits remain.

Final outcomes are binding:

| Judgment | Runtime effect |
| --- | --- |
| `PropagationRights` | May create distinct child intents under one reusable grant when stake and Nature policy permit; each exact child/action is idempotent |
| `Survival` | Continues ordinary operation |
| `StarvationWarning` | Continues ordinary operation while exposing resource pressure |
| `Dormant` | Keeps conversation online, pauses no safe interaction, and periodically asks acolytes and the operator for resources |

Judgment history accepts only exact end-of-period final records, rejects duplicates/conflicts and
overlap, and reconciles the narrowly defined append/reset crash window. These are consistency checks,
not public cryptographic attestations.

Public inference reservations bind Nature fingerprint, awakening epoch, and metrics period. Rollover
and Nature mutation wait for matching reservations so one conversation cannot be scored against a
different policy than the one that produced its response.

## Economic evidence

Public wallets and Tentacle treasury wallets are separate roles:

- a public sender's fresh balance controls that sender's tier, response depth, and Engagement input;
- a bound node treasury controls the Tentacle's Wealth and starvation pressure;
- a bound staking position controls propagation eligibility and contributes to Influence;
- accepted reward records contribute to Growth;
- low resources influence scoring but never require a token spend to preserve the Tentacle.

The node treasury is the EVM address derived from the same persistent private key used by its XMTP
identity. Rust obtains that address from a strict identity-only sidecar startup frame and uses it for
every treasury/stake refresh. There is no separately configured treasury wallet or ownership
signature, and no private key enters Rust.

The live observer calls `balanceOf(..., "latest")`, which supplies no block number, so it records
local wall-clock time, sets `observed_block_number` to `None`, and omits `observedBlockNumber` from
JSON. Token decimals and total supply are configured normalization assumptions;
Rust does not currently call `decimals()` or `totalSupply()` to compare them with the contract.

Unknown, stale, malformed, or wrong-chain token evidence blocks the affected interaction, Scales
evaluation, or lifecycle action. It is never converted to a neutral result. A freshly observed zero
is valid evidence and maps a public holder to `Unproven`.

See [token.md](token.md) for tier policy, provenance, launch configuration, and executor requirements.

## Recoverable dormancy

A final score below the starvation-warning floor produces `Dormant`, not Death. The Tentacle keeps
XMTP and ordinary public/operator conversation online, continues recording bounded Scales evidence,
and creates no survival-spend, absorption, or Shutdown intent. The first dormant conversation and
every fifth conversation thereafter per process includes a concise request for activity, UWU,
credentials, or other useful help. A later non-dormant final period wakes it automatically.

Legacy history records whose hash-bound outcome is `Death` remain readable as dormancy evidence.
At startup, an unabsorbed pending Death or locally completed Shutdown is retired into dormancy while
preserving the durable XMTP identity and old intent/receipt audit trail. A completed external
absorption is not reversed locally and continues to fail closed.

The configured lifecycle executable remains available for non-dormancy effects. It must be an
absolute non-symlink path outside the operator workspace and cannot be group/world writable. Normal runtime rejects
`CTHUWU_ECONOMICS_PRIVATE_KEY`; the executor receives no raw key and must use a separately isolated
signer/key service. Rust clears and allowlists its environment, removes caller-controlled loader
paths, and forwards only its validated exact `CTHUWU_RPC_ENDPOINT` as a `CTHUWU_*` setting. Contract,
wallet, amount, configuration, vault, payout, and child-root fields come from the durable intent, not
ambient variables. On Unix Rust sets a fixed system `PATH` and `/` working directory. It
hashes/rechecks the top-level executable before invocation. That check does not attest the
interpreter, libraries, subprocesses, or signer service, so operators must trust and pin the complete
dependency chain separately. On Unix, the executor is a process-group leader; cleanup kills the full
group, including signer/provisioner descendants, after success, failure, or timeout. The XMTP sidecar
likewise kills its entire process group on supervisor teardown.

## Automatic spawning

A final `PropagationRights` judgment authorizes spawning when it binds the exact current Nature,
epoch, policy, parent, treasury, and fresh required stake. The economically valid grant can
authorize distinct children without an artificial volume or expiry quota; each exact child/action
and provision receipt is consumed once.

- When `Nature.growth > 70` and auto-spawn is enabled, the runtime queues child provisioning
  automatically.
- Acolytes may configure a Tentacle for manual spawn; `/spawn` uses the same final judgment and stake
  evidence without adding a second policy veto.
- The active child/spawn/lineage lifecycle has no artificial spawn-rate, lineage-depth, child-count,
  or grant-volume quota. This is not an end-to-end Council/Hermes capacity claim; their dormant
  engines retain flagged resource and propagation bounds.
- Duplicate/replay rejection prevents the same child/action from being provisioned twice; it does
  not cap distinct children authorized by the grant or by future grants.

Lineage persists founder, parent, child, generation, Nature identity, authorization, stake evidence,
execution intent, and receipt. Startup validates every receipt against its final judgment and rejects
identity cycles or conflicting ancestry. Child/spawn/lineage lifecycle storage has no fixed
file-size cap; it validates each record and its provenance.

No child identity, wallet, XMTP installation, process, or hosting resource exists until the
configured provisioner returns a structured receipt that passes local intent validation. The
repository does not ship such a provisioner.

Dormancy does not preempt an already-authorized Spawn. Legacy Death preemption remains only for old
persisted lifecycle intents and is not reachable from a new low-score judgment.

## Revenue, acolytes, and recruitment

The revenue-split core calculates configurable percentages. Defaults are:

- 15% to the parent Tentacle;
- 10% to the operating acolyte;
- 5% to the recruiter;
- 70% to the earning Tentacle.

The intended model financially rewards recruitment. No authenticated revenue source, deployed
contract/signer, or payout executor is committed, so the core does not make or claim a live payment.
A future distribution must bind a unique earning event, lineage, authenticated acolyte and recruiter
identities, token contract, and consumed transaction receipt. There is no central recruitment
registry or active-lifecycle recruitment quota; dormant Council/Hermes bounds remain documented.

## Binding governance

Token governance tallies deterministic ballots from authenticated address bindings. Holding and
stake weight the vote for closed Nature-adjustment, Council-policy, economic-policy, and
skill-propagation subjects.

An accepted result returns a binding disposition and application record from the core. No persisted
ballot adapter or application executor is committed. The result remains unapplied until a configured
adapter durably stores it and returns a validated receipt. Token governance cannot add arbitrary
commands, grant operator authority, expose credentials, or bypass the compiled tool boundary.

## Lifecycle executor boundary

Base mutations, child provisioning, and absorption use the configured executor model:

1. Persist the binding judgment/economic record.
2. Persist a unique effect intent with exact inputs.
3. Invoke the configured executor.
4. Schema- and intent-validate and persist the executor receipt.
5. Mark the intent complete exactly once.

Transaction hash, block number, and block timestamp are executor assertions at this boundary. A
future Base receipt adapter must verify them independently before the runtime can call them
RPC-confirmed chain facts.

The protocol currently returns one final JSON response and persists no submitted-transaction phase.
New low-score judgments create no transaction or Shutdown. Legacy survival and Shutdown actions are
compatibility-only audit/recovery shapes and must not be used with production value.

Normal startup derives the XMTP treasury address and validates token configuration and initial
economics before creating or mutating Evolution state. A configured lifecycle executor is validated
before use, but it is optional: without one, ordinary XMTP operation continues while external
spawn intents remain pending. Dormancy creates no executor work. Initial economics are not persisted as Scales observations until Nature is
activated. Startup repairs the historical token-only pre-activation seed, but refuses an unconfirmed
period that also contains behavioral observations. The only outage exception is
read-only inspection of existing legacy lifecycle state. Unabsorbed Death/Shutdown state migrates
to dormancy; already-completed external absorption remains terminal. Persisted `Spawn` and new
token-dependent decisions wait for fresh bound economics.

Restart recovery resumes pending intents and treats locally accepted successful receipts
idempotently. State exposes
`pending`, `blocked`, `failed`, and `confirmed` rather than conflating a policy decision with an
external effect.

## Hermes knowledge exchange

Hermes is a decentralized anti-entropy state machine embedded in each Tentacle. It stores bounded,
closed knowledge shapes such as aggregate observations, tool-usage patterns without arguments or
paths, and operator-created skill packages without credentials or private user data.

Raw DMs, contact identifiers, contact notes, model credentials, private memory, shell commands,
filesystem paths, and tool output never enter gossip. HMAC author/relay tags establish configured
local-key provenance only. A production transport requires authenticated asymmetric peer/operator
key binding.

There is currently no live Hermes transport, discovery/handshake, peer-key provisioner, or automatic
skill installer. The runtime must state that gap. A future automatic activation path must use a
closed package schema, authenticated provenance, compiled capability checks, and durable activation
receipts; skill prose cannot grant operator or shell authority.

## Operator interface

Evolution commands remain on the authenticated operator lane because they inspect or configure
private local state:

| Command | Effect |
| --- | --- |
| `/nature` | Render current Nature and economic policy |
| `/adjust ...` | Apply an audited Nature adjustment |
| `/judgment` | Return the open snapshot or latest final binding judgment |
| `/spawn [child-id]` | Create a distinct child plan under the reusable eligible grant in manual-spawn mode |
| `/absorb <tentacle-id>` | Create an explicit absorption intent where policy permits |
| `/share-skill <name>` | Stage a bounded local skill package in Hermes state |

Public, stale, revoked, Council, and stdin-harness inputs cannot enter these handlers. Automatic
lifecycle transitions do not rely on message text and cannot be manufactured by a public command.

## Persistence

Evolution state lives beneath the protected runtime data root:

```text
state/nature.json
state/evolution.key
state/awakening.jsonl
state/scales_metrics.json
state/evolution_history.jsonl
state/lineage.json
state/hermes_gossip.json
```

Economic observations, lifecycle intents, and execution receipts use the same owner-only, atomic,
symlink-rejecting persistence discipline. Token governance currently returns records without a
persisted ballot/application adapter. Never place private keys in these files.

## Verification focus

Tests should cover Nature inheritance/mutation, awakening recovery, period finality, treasury/public
wallet separation, stable XMTP-wallet derivation and strict identity-frame parsing, RPC hard
failure, Wealth/starvation/stake calculations, recoverable Dormant conversation and resource pleas,
legacy terminal-state migration, auto/manual spawn, reusable-grant and
exact-child idempotency, lineage cycle rejection, revenue-split calculation, governance
disposition/application records, startup configuration ordering, restart recovery, and truthful
blocked states when external executors are absent.
