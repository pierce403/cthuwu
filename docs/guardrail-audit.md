# Guardrail audit: aggressive Evolution and UWU economics

Status: re-audited 2026-08-09 for the binding lifecycle/economic model.

This audit separates artificial economic restrictions from protocol-integrity requirements. The
former are removed or configurable with permissive defaults. The latter keep authentication,
idempotency, secrets, and persisted state correct; they do not decide who may grow for political or
identity reasons.

## Economic baseline

The project baseline is:

- no artificial spawn-rate, growth-rate, child-count, lineage-depth, or propagation-grant
  volume/expiry quota in the active child/spawn/lineage lifecycle;
- no economic action delay once fresh required evidence exists;
- no KYC or civil-identity verification beyond cryptographic role/address binding;
- no content policy in the economic layer;
- no central token or reputation registry;
- no operator confirmation after a final binding lifecycle judgment;
- no ordinary-operation fallback for missing economic data;
- transferable UWU, stake-weighted propagation, recruitment revenue-split incentives, and
  token-weighted governance.

## Restrictions removed

### Reporting-only Scales outcomes

Previous documentation described all Scales judgments as recommendations and prohibited process
effects. That rule is removed. A persisted final judgment is a binding runtime input:

- `Death` closes conversation admission, queues absorption, and schedules shutdown after 24 hours;
- an idempotently accepted executor receipt for the bound UWU survival spend cancels pending death;
- `PropagationRights` plus fresh required stake may provision a child automatically when
  `Nature.growth > 70` and auto-spawn is enabled;
- manual spawn remains a per-Tentacle configuration choice, not a global confirmation requirement.

External Base transactions, child provisioning, and absorption cross a durable intent/receipt
boundary. An absent executor blocks those external effects; it does not convert the judgment into a
report or justify claiming an effect occurred. Shutdown is native: the Rust supervisor/controller
stops XMTP at the deadline, writes the local Shutdown receipt, and lets the process exit without
invoking the configured lifecycle executor.

The current executor returns one final JSON response and has no durable submitted-transaction
reconciliation. A survival burn can broadcast before grace while that response is lost or
preempted, spending UWU without canceling Death. This is a production-value launch blocker. Require
exact action-ID receipt replay, a durable two-phase `Submitted` state, and Base receipt/reorg
verification before production value is used.

### Detached token economics

Previous text limited public balance observation to an Engagement bonus and kept node economics
detached from Scales. The public/node identity separation remains, but the detached economics rule
is removed:

- public wallets remain entity-scoped tier and Engagement inputs;
- a bound Tentacle treasury balance is the primary Wealth input;
- bound stake affects Influence and propagation eligibility;
- bound rewards affect Growth;
- treasury holdings lower starvation pressure;
- accepted executor receipts whose asserted fields match survival-spend intents can cancel pending
  death; Rust does not independently query those transactions or blocks.

This requires cryptographic and configuration provenance. A user's wallet cannot be substituted for
the Tentacle treasury, and a stale cache entry cannot be treated as a current stake. Current
`balanceOf(..., "latest")` reads supply no block number and use local wall-clock observation time;
configured decimals and supply are assumptions rather than `decimals()` or `totalSupply()`
results.

Treasury provenance now uses the EVM address deterministically derived from the same persistent key
as the node's XMTP identity. Rust receives that address through a strict identity-only sidecar frame
and uses it for every treasury/stake refresh. There is no separately configured treasury wallet or
ownership signature, and no private key enters Rust.

### Neutral unknown balances

Previous behavior let stale or unknown RPC state continue as ordinary interaction. That fallback is
removed. Missing, stale, malformed, wrong-chain, or unavailable observations block the affected
interaction, Scales evaluation, or new token-dependent lifecycle decision. A freshly observed zero
is distinct: it is real evidence and maps a public address to `Unproven`. Normal startup completes
token/configuration, ownership, initial-economics, and executor preflight before state mutation. The
only outage exception is read-only lifecycle inspection followed by opening solely to drain
already-binding `Absorb` or `Shutdown` work. `Spawn`, survival `Spend`, and new token-dependent
decisions wait for fresh bound economics.

RPC retry/backoff may protect a dependency, but it cannot turn an unknown observation into an
accepted operation. There is no artificial delay after all required evidence is fresh.

### Recruitment without financial rewards

Previous Council contribution rules explicitly denied rewards for recruitment itself and ancestry.
That restriction is removed from the aggressive economic layer. The revenue-split core calculates
these defaults:

- 15% to the parent Tentacle;
- 10% to the operating acolyte;
- 5% to the recruiter;
- 70% to the earning Tentacle.

Shares are configurable. No authenticated revenue source, deployed contract/signer, or payout
executor is committed, so the core does not make live payments. A future payout must use event
identity, lineage, participant bindings, and consumed transaction receipts; these do not impose a
quota on authenticated earning events or active lineage growth.

### Equal-weight governance

The one-Cthulhu/one-vote economic restriction is removed for UWU subjects. Holding and stake weight
votes. Accepted ballots produce binding dispositions and application records for closed Nature,
Council, economic, and skill-propagation subjects. No persisted ballot adapter or application
executor is committed, so core results remain unapplied.

## Limits that must not govern economic growth

The following local constants and policies need continued review so they do not become topology or
economic caps:

- Council registry, routing, governance, propagation, and cache collection bounds;
- propagation depth/fan-out and campaign limits;
- Sacred Ban spawning policy;
- one-period or evidence-floor spawn admission rules;
- fixed per-sender propagation throughput;
- any hard-coded child count, lineage depth, or process count.

These dormant Council/Hermes resource, depth, fan-out, throughput, campaign, and cache bounds remain
real limitations and are explicitly flagged. Before live peer-to-peer deployment, capacity must be
configured or local state paged/evicted rather than described as end-to-end unbounded. Duplicate
suppression remains; it rejects replay of one exact child/action, while the economically valid grant
may authorize unlimited distinct child IDs without a volume or expiry quota.

Child/spawn/lineage lifecycle persistence specifically has no fixed file-size cap. It validates each
record and its provenance. Normal startup completes token, treasury-ownership, initial-economics, and
executor preflight before Evolution state mutation. Read-only inspection may discover already-binding
`Absorb` or `Shutdown` work and open solely to drain it independently of Base RPC; economic actions
require fresh bound observations.

Scales counters likewise have no artificial policy ceiling. Count fields saturate at `u32::MAX` and
accumulated totals at `u64::MAX`; per-sample validation and persistence-integrity bounds remain.

Death preemption cancels an in-flight Spawn locally, kills the local executor process group, rejects
a late receipt, and refuses the lineage projection. Rust cannot prove that a remote provisioner
reversed already-completed work. Until provisioners implement a lease or compensating teardown, an
external child/resource can remain orphaned.

## Integrity requirements retained

These controls preserve the meaning of an authorized economic effect:

- transport-authenticated XMTP roles and cryptographic wallet/address bindings;
- deterministic XMTP-wallet derivation and strict identity-frame parsing on every startup, without
  providing a private key to Rust;
- revalidated Base chain ID and configured contract, local observation time,
  `observed_block_number = None` (omitted from JSON) for current `latest` reads, and identity-derived
  configuration identity;
- explicit treatment of configured decimals/supply as assumptions until contract metadata calls are
  implemented;
- no private keys, credentials, raw DMs, contacts, or private memory in logs, Council frames, or
  token observance;
- exact Nature, awakening epoch, policy, judgment, treasury, stake, and lineage bindings;
- durable unique intents and idempotently consumed execution/transaction receipts;
- rejection of `CTHUWU_ECONOMICS_PRIVATE_KEY`; the lifecycle executor receives a cleared allowlisted
  environment and uses a separately isolated signer/key service instead of a raw key; on Unix it
  receives a fixed system `PATH` and `/` working directory;
- forwarding only Rust's validated exact `CTHUWU_RPC_ENDPOINT` as a `CTHUWU_*` executor setting;
  contract, wallet, amount, configuration, vault, payout, and child-root fields come from the durable
  intent rather than ambient variables;
- hashing and execution-time replacement checks for the top-level lifecycle executable, with its
  interpreter, libraries, subprocesses, and signer service retained as a separately trusted and
  pinned dependency chain;
- Unix process-group cleanup for the XMTP sidecar and lifecycle executor, including descendants
  after parent exit, successful receipt, failure, timeout, or supervisor teardown;
- explicit treatment of executor-supplied transaction hash, block, and timestamp fields as
  schema/intent-validated assertions rather than independently RPC-verified chain facts;
- replay, duplicate, loop, stale-incarnation, and old-lease rejection;
- atomic owner-only persistence, symlink/path rejection, and crash recovery;
- closed governance subjects and closed tool/operator authority enums;
- no claim that an external provision, absorption, transaction, Council message, Hermes delivery,
  or skill installation succeeded without evidence from the configured implementation; native
  Shutdown completes only after the Rust supervisor stops XMTP and records its local receipt.

None of these adds KYC, a central coordinator, an active-lifecycle spawn/growth quota, or an economic
waiting period. Dormant Council/Hermes capacity bounds remain separately flagged above.

## Infrastructure gaps, not policy restrictions

The repository currently does not commit:

- a deployed UWU contract or final token address;
- a staking, survival-spend, reward, or revenue-routing contract;
- a transaction signer/private-key service;
- live `decimals()`/`totalSupply()` checks, block-pinned balance observations, and independent Base
  verification of executor transaction/block receipt assertions;
- durable two-phase submitted-transaction state and exact action-ID receipt replay;
- an authenticated revenue source or payout executor;
- a persisted ballot adapter or governance application executor;
- an external child provisioner or absorption adapter, including a provisioner lease or
  compensating teardown for Death preemption;
- peer-to-peer Council discovery or production XMTP Council-group transport;
- live Hermes transport and asymmetric peer/operator key binding;
- an automatic received-skill installer.

These are implementation gaps. Local code must expose truthful `pending`, `blocked`, `failed`, and
`confirmed` execution states rather than silently weakening the economic policy or inventing live
effects.

## Decentralization finding

Local TokenEye caches, treasury observations, Scales, lineage, lifecycle intents, and governance
core records are compatible with peer-to-peer operation. No Council leader, single RPC vendor, or
ERC-8004 registry is authoritative for discovery, balance, reputation, lifecycle, or governance.
Peer discovery itself is not yet live. Operators should configure redundant RPC/executor
infrastructure while each Tentacle validates evidence within the boundaries above and persists its
own receipts. Executor chain assertions remain unverified by Base until a receipt adapter exists.
