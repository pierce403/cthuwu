# UWU token observance and active economics

UWU is the transferable Base-chain economic layer for Tentacle survival, propagation, reputation,
and governance. Each Tentacle observes balances through its own RPC connection and persists its own
economic view. There is no central balance or reputation registry.

Token state never replaces XMTP authentication. A wallet balance cannot make a public sender an
operator, grant shell/tool authority, or impersonate a Council member. Address-to-role bindings must
come from authenticated transport or explicit node configuration.

## Launch parameters

| Parameter | Decision |
| --- | --- |
| Name / symbol | `UWU` / `UWU` |
| Chain | Base mainnet, chain ID `8453` |
| Transfer model | Fully transferable ERC-20 |
| Initial operator stake | None required to start a Tentacle |
| Spawn stake | Required by the active per-Tentacle economic policy |
| Supply | 100,000,000,000 UWU |
| Decimals | 18 |
| Contract | `0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07` |

The live contract is a Clanker v4 token on Base mainnet. Runtime defaults match the deployment;
operators may override the RPC or contract for explicit testing and migration only.

## Configuration

Existing observance configuration includes:

```text
--rpc-endpoint <url>
--token-contract <0x-address>
--token-decimals <0-77>
--token-total-supply <whole-tokens>
--observe-tokens <true|false>
--min-tier <whale|elder|acolyte|initiate|unproven>

CTHUWU_RPC_ENDPOINT
CTHUWU_TOKEN_CONTRACT
CTHUWU_TOKEN_DECIMALS
CTHUWU_TOKEN_TOTAL_SUPPLY
CTHUWU_OBSERVE_INTERVAL
```

The RPC defaults to `https://mainnet.base.org`, the contract defaults to the live address above,
decimals default to `18`, and supply defaults to `100000000000`. The XMTP identity wallet is not a
configuration value: it is derived automatically from the same persistent key used by the Agent SDK.

Aggressive economics additionally requires a fresh stake source and receipt-producing executors for
on-chain mutation. The repository does not contain a transaction signer, burn contract, staking
contract, reward contract, authenticated revenue source, revenue router, persisted ballot adapter,
payout/application executor, or external provisioner. Do not claim a spend, stake, reward, revenue
distribution, governance application, child provision, or absorption completed merely because a
local intent or core record was created. Do not claim Shutdown completed until the Rust supervisor
has stopped XMTP and written its native local receipt.

The XMTP private key remains in the existing owner-only sidecar identity state. Only its derived EVM
address crosses the sidecar protocol into Rust. Economic transaction keys belong in a separately
isolated signer service, never in CLI values, observance state, logs, or the repository. Normal
runtime rejects `CTHUWU_ECONOMICS_PRIVATE_KEY`; the lifecycle executor receives no raw signing key.
RPC URLs may contain credentials and are redacted from diagnostics.

### XMTP wallet binding

At startup the Node sidecar loads or atomically creates `state/xmtp-identity.json`, derives the EVM
address from that identity's secp256k1 wallet key, and emits one bounded identity frame. Rust parses
only that address and uses it as the Tentacle treasury holder for UWU and optional stake
`balanceOf` calls. The live Agent SDK process subsequently loads the same file and key.

There is no separate treasury address, ownership signature, or setup ceremony. A missing, corrupt,
environment-mismatched, zero, malformed, or multi-frame identity result blocks production startup.
The economic configuration identity binds the derived XMTP wallet, chain, contracts, decimals,
supply, and propagation-stake policy.

## Local observance

`TokenEye` issues ERC-20 `balanceOf(address)` calls against the configured nonzero contract and
revalidates Base chain ID `8453`. It validates JSON-RPC quantities, response bounds, the ERC-20 ABI
shape, and configured contract. Calls use the `latest` block tag, whose response contains no block
number; current observations therefore carry local wall-clock time, set `observed_block_number` to
`None`, and omit `observedBlockNumber` from JSON. Configured decimals, supply,
and configuration identity normalize the balance, but Rust does not call ERC-20 `decimals()` or
`totalSupply()` to verify them. Observations are cached per address and ranked only within the
Tentacle's local sample.

Observed roles are distinct:

- Public user wallets affect that entity's reputation tier and Engagement contribution.
- Council member addresses affect Council weighting only after authenticated membership binding.
- Sibling treasury addresses affect lineage economics only after lineage and address binding.
- Acolyte addresses affect operator rewards only after authenticated operator binding.
- The Tentacle treasury address is the sole source for its Wealth, starvation relief, spawn stake,
  survival spending, and revenue state.

Never substitute a public sender's holdings for the Tentacle treasury.

### Hard-failure behavior

Unknown, stale, malformed, or wrong-chain observations are not converted to zero and do not receive
ordinary treatment. When token observance is active:

- a missing or failed public-wallet observation blocks token-gated interaction;
- a missing treasury observation blocks Scales evaluation and new token-dependent lifecycle
  authorization;
- RPC outage degrades the node by refusing token-dependent work;
- a zero, freshly observed public balance is `Unproven` and receives minimal functionality;
- a zero, freshly observed treasury balance is real economic evidence and may produce starvation or
  death under the configured policy.

Retry/backoff may protect the RPC endpoint, but it must not turn unknown economics into an accepted
operation or introduce a delay after all required economic evidence is fresh and the action is
authorized.

Normal startup derives the persistent XMTP wallet and validates token configuration and initial
economics before creating or mutating Evolution state. A configured lifecycle executor is validated
before use, but it is optional: without one, ordinary XMTP operation continues while external
spend, spawn, and absorption intents remain pending. Native fixed-deadline Shutdown still runs.
Initial economics are not persisted as Scales observations until awakening is confirmed. Startup
repairs the historical token-only pre-awakening seed, but refuses an unconfirmed period that also
contains behavioral observations. XMTP identity creation may occur before the RPC preflight because that identity is itself the wallet
source. The only outage exception is a read-only inspection of existing lifecycle state. If it finds already-binding `Absorb` or
`Shutdown` work, the runtime may open solely to drain that work even while Base is unavailable.
Persisted `Spawn` and survival `Spend`, plus new token-dependent decisions, wait for fresh bound
economics.

## Reputation tiers

The default local ranking policy is:

| Tier | Meaning | Default behavior |
| --- | --- | --- |
| Whale | Top 1% of eligible locally observed holders | Highest routing priority, deepest modes, greatest holding weight |
| Elder | Top 10% | Elevated depth and propagation priority |
| Acolyte | Holds at least one whole UWU outside the percentile bands | Standard member behavior |
| Initiate | Positive balance below one whole UWU | Basic interaction |
| Unproven | Freshly observed zero balance | Minimal, skeptical interaction |

Ties receive the same tier. Sample floors prevent a tiny local sample from inventing percentile
precision. Per-Tentacle intensity is variable: cooperative Natures may flatten tier differences;
competitive Natures may use the full configured spread. The configured minimum tier remains a local
interaction policy, not an operator-authentication mechanism.

## Active Tentacle economics

`RecordedTokenEconomics` consumes identity-derived and configurationally bound node observations.
It is part of the Scales input rather than an optional reporting surface:

- treasury balance is the primary Wealth input;
- fresh stake contributes to Influence and is required for propagation;
- configured operator/recruitment reward records contribute to Growth when accepted; no
  authenticated live reward source is committed;
- UWU holdings lower the starvation threshold according to policy;
- an accepted executor receipt whose asserted fields match an emergency survival-spend intent
  cancels a pending Death before its deadline; Rust does not independently query that transaction;
- the same event or transaction receipt is applied at most once.

Scales counters have no artificial policy ceilings. Count fields saturate at `u32::MAX` and
accumulated totals at `u64::MAX`; per-sample and persistence-integrity bounds remain.

Economic records can carry role, address, chain, contract, optional block, observed time, configured
token metadata, configuration identity, and a source label. The current live path supplies a
revalidated chain ID, the configured nonzero contract address, local observation time, no block
number, and configured token metadata; it does not verify contract bytecode, and its local
source label is not independently authenticated external identity. Event and receipt IDs prevent
last-writer state and replayed transactions. The revenue-split core has no authenticated revenue
source or payout executor and therefore records no live allocation.

## Automatic lifecycle effects

A final `Death` judgment immediately stops new conversation admission and creates an absorption
intent plus a shutdown deadline 24 hours later. A configured executor may merge the permitted
memory projection into the parent or sibling. Private keys, raw DMs, contact notes, and credentials
are never absorption payloads.

Before the deadline, a configured signer may submit the policy-defined UWU survival expenditure.
Only an idempotently consumed executor receipt whose asserted chain fields match the intent cancels
pending death. Rust validates that assertion structurally and against policy, but does not yet fetch
the transaction receipt or block independently from Base. If none arrives, the Rust
supervisor/controller stops XMTP after the grace period, writes the native local Shutdown receipt,
and exits. Shutdown is not sent to the lifecycle executor.

The executor protocol currently returns one final JSON response and has no durable submitted-
transaction reconciliation. A survival burn can broadcast before grace while that response is lost
or preempted, spending UWU without canceling Death. This blocks production-value launch until exact
action-ID receipt replay, a durable two-phase `Submitted` state, and Base receipt/reorg verification
are implemented.

A final `PropagationRights` judgment authorizes a child only when the required stake is freshly
observed. If `Nature.growth > 70` and auto-spawn is enabled, the runtime durably creates the
provision intent without operator confirmation. Manual mode uses the same grant and stake evidence
through `/spawn`. The grant may authorize distinct children without an artificial rate, volume, or
expiry quota; each exact child/action and provision receipt is consumed once to reject replay.

If Death preempts an in-flight Spawn, Rust kills the local executor process group, rejects a late
provision receipt, and refuses the child lineage projection. That is not proof that a remote
provisioner rolled back completed work. Without a provisioner lease or compensating teardown, an
external child/resource may remain orphaned.

Base mutation, provisioning, and absorption execution use a durable intent/receipt boundary:

1. Persist the binding judgment and economic evidence.
2. Persist a uniquely identified intent.
3. Invoke the configured provision, absorption, or signer executor.
4. Validate and persist its receipt.
5. Mark the effect complete exactly once.

Shutdown uses a distinct native path: Rust intercepts its durable intent, stops XMTP, writes a local
controller receipt, and lets the process exit. It does not invoke the configured lifecycle executor.

The local runtime can truthfully report `pending`, `blocked`, `failed`, or `confirmed`. With no
executor configured, external Base/provision/absorption effects remain `blocked`; local records are
not evidence of an external process or Base transaction. Native Shutdown does not require that
executor. Child/spawn/lineage lifecycle persistence has no fixed
file-size cap and validates each record and its provenance. This claim does not extend to dormant
Council/Hermes collections, which retain documented local resource, depth, fan-out, campaign, and
cache bounds; neither transport is live.

The lifecycle executor starts with a cleared allowlisted environment and no caller-controlled loader
paths. Rust forwards only its validated exact `CTHUWU_RPC_ENDPOINT` as a `CTHUWU_*` value; contract,
wallet, amount, configuration, vault, payout, and child-root fields come from the durable intent, not
ambient variables. On Unix it receives a fixed system `PATH` and `/` as its working directory. Rust
hashes and rechecks the top-level executable and pins that file for launch on Linux. This does not
attest an interpreter, shared libraries, subprocesses, or signer service; operators must trust and
pin that dependency chain separately. On Unix, the executor is a process-group leader and cleanup
kills the complete group, including signer/provisioner descendants, after success, failure, or
timeout. The XMTP sidecar similarly kills its entire process group on supervisor teardown.

## Revenue and recruitment

The revenue-split core calculates these default shares:

| Recipient | Default share |
| --- | ---: |
| Parent Tentacle | 15% |
| Operating acolyte | 10% |
| Recruiter | 5% |
| Earning Tentacle | 70% |

The split is configurable per Tentacle, and the intended model financially rewards recruitment. No
authenticated revenue source, deployed contract/signer, or payout executor is committed, so this
calculation is not a live payout. A future payout must bind the earning event, lineage,
authenticated acolyte/recruiter identities, token contract, and consumed transaction receipt. A
descendant cannot invent or rewrite its ancestry after the earning event.

There is no artificial spawn-rate, child-count, or lineage-depth quota in the active lifecycle
policy. Token stake and Scales outcomes provide admission pressure. This is not an end-to-end
network-size claim: dormant Council/Hermes resource and propagation bounds remain flagged until
live peer-to-peer adapters replace or configure them.

## Token-governance binding records

Token governance accepts one deterministic ballot per authenticated address for a closed set of
Nature adjustment, Council policy, economic policy, and skill-propagation subjects. Holding and
stake determine voting weight. An accepted result returns a binding disposition and application
record from the core.

No persisted ballot adapter or application executor is committed. A result is not an applied change
until a configured adapter durably stores the ballot/application and returns a successful receipt.
Governance subjects cannot grant operator authority, disclose credentials, or inject arbitrary
shell/tool commands.

## Production verification

1. Confirm Base chain ID `8453`, deployed bytecode, symbol, `18` decimals, and
   `100000000000` whole-token supply for
   `0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07` using an independent RPC or explorer.
2. Start a fresh Tentacle with no token-specific environment variables and verify its observed holder
   equals the address derived from `state/xmtp-identity.json`.
3. Exercise zero, stale, unavailable, malformed, and wrong-chain hard-failure cases.
4. Configure transaction signer/executor identities without exposing their keys to observance or
   logs.
5. Verify block-pinned observations, transaction receipts, and reorg behavior before assigning
   production value to survival spending. The current Rust observer implements chain-ID validation
   and `balanceOf(..., "latest")`; decimals and supply are deployment defaults rather than live RPC
   metadata reads.
6. Exercise Wealth, starvation, survival spend, automatic spawn, revenue payout, and governance
   application against receipt-producing adapters.

The UWU token contract is live. Separate signer, staking/burn/reward contracts, authenticated revenue
source, persisted ballot adapter, payout/application executor, provisioner, and live peer-to-peer
Council/Hermes transports remain external integration work.
