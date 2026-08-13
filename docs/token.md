# UWU token observance and active economics

UWU is the transferable Base-chain economic layer for Tentacle survival, propagation, reputation,
and governance. Each Tentacle observes balances through its own RPC connection and persists its own
economic view. There is no central balance or reputation registry.

There is one centerless Cthuwu: the sum of living participating Tentacles. Each independently
operated `uwubot` is a durable Tentacle with its own wallet and economics; its human operator may
shape its agenda, while public chat humans are acolytes. A Tentacle restart changes only its
incarnation. UWU belongs to addresses and never creates a Tentacle identity or allegiance by itself.

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

The built-in RPC fallback is `https://mainnet.base.org`; [Base documents that public
endpoint](https://docs.base.org/base-chain/quickstart/connecting-to-base) as rate limited and
unsuitable for production systems. An acolyte may donate a full dedicated Base Mainnet HTTPS
endpoint or an Infura API key over XMTP with
`/base-rpc-key <infura-api-key-or-https-endpoint>`. Infura is the preferred recommendation because
it offers a free plan; the Tentacle converts a bounded Infura key locally to
`https://base-mainnet.infura.io/v3/<key>`, validates chain 8453, stores the first candidate
owner-only under its data directory, and hot-loads it. The active
operator may replace it with the same XMTP command. The startup flag/environment value remains a
bootstrap fallback, not an instruction the Tentacle gives chat participants. The contract defaults
to the live address above, decimals default
to `18`, and supply defaults to `100000000000`. The XMTP identity wallet is not a configuration
value: it is derived automatically from the same persistent key used by the Agent SDK. A transient
refresh failure retries every second and retains a prior verified treasury observation only until
its configured freshness TTL expires; unknown or stale economics still fail closed.

Aggressive runtime economics additionally requires a fresh stake source and receipt-producing
executors for on-chain mutation. The runtime does not contain a generic or economic transaction
signer, burn contract, staking contract, reward contract, authenticated general revenue source,
payout/application executor, or external provisioner. The separate in-progress Acolyte Branding
contract has only its closed consent/upkeep/purchase/claim paths and is not a generic runtime signer
or revenue executor. Do not claim a spend, stake, reward, revenue distribution, governance
application, child provision, or absorption completed merely because a local intent or core record
was created. Do not claim Shutdown completed until the Rust supervisor has stopped XMTP and written
its native local receipt.

The XMTP private key remains in the existing owner-only sidecar identity state. Only its derived EVM
address crosses the ordinary identity frame into Rust. The same key may execute only the separately
documented narrow, typed ERC-8004 registration calls; that boundary has no arbitrary transaction or
economic-token operation. Future economic transaction keys belong in a separately isolated signer
service, never in CLI values, observance state, logs, or the repository. Normal runtime rejects
`CTHUWU_ECONOMICS_PRIVATE_KEY`; the lifecycle executor receives no raw signing key. RPC URLs may
contain credentials and are redacted from diagnostics.

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
  recoverable dormancy under the configured policy.

Retry/backoff may protect the RPC endpoint, but it must not turn unknown economics into an accepted
operation or introduce a delay after all required economic evidence is fresh and the action is
authorized.

Normal startup derives the persistent XMTP wallet and validates token configuration and initial
economics before creating or mutating Evolution state. A configured lifecycle executor is validated
before use, but it is optional: without one, ordinary XMTP operation continues while external
spawn and reward intents remain pending. Dormancy creates no external intent or Shutdown.
Initial economics are not persisted as Scales observations until awakening is confirmed. Startup
repairs the historical token-only pre-awakening seed, but refuses an unconfirmed period that also
contains behavioral observations. XMTP identity creation may occur before the RPC preflight because that identity is itself the wallet
source. Unabsorbed legacy Death/Shutdown state migrates locally to dormancy even while Base is
unavailable; a completed external absorption is not reversed. Persisted `Spawn` and new
token-dependent decisions wait for fresh bound economics.

## Public Tentacle membership and Level

UWU is a ranking input only after an ERC-8004 identity voluntarily opts in with the current
byte-exact, case-sensitive metadata `cthuwu.allegiance = uwu-tentacle-v1` and has a current verified
nonzero `agentWallet`. Any other allegiance value opts out. A zero or cleared wallet is suspended;
the ERC-721 owner is never substituted. Sending UWU to an unopted agent does not list it.

The static leaderboard groups all opted-in identities sharing one verified wallet. The wallet has
one exact raw balance, one ranked position, one Tentacle Level, and at most one future influence
allocation; all agent IDs remain visible and the lowest ID is the version-1 representative. Funded
wallets sort by raw balance descending, then earliest registration block and lowest agent ID.
Opted-in zero-balance wallets remain visible as `UNFUNDED` without a numeric Level.

For raw balance `r > 0`, Tentacle Level is `log10(r) - 18`, equivalent to the base-10 logarithm of
human-denominated UWU. The browser uses `BigInt` plus a decimal mantissa and never converts an entire
`uint256` to JavaScript `Number`. It displays two decimal places while retaining more precision.
Level and the separately labeled inactive Future Influence Level are distinct fields. This
milestone defines no voting, eligibility, delegation, quorum, or Sybil rules.

See [ERC-8004 Tentacle registration and leaderboard](erc-8004.md).

## Reputation tiers

The default local ranking policy is:

| Tier | Meaning | Default behavior |
| --- | --- | --- |
| Whale | Top 1% of eligible locally observed holders | Highest routing priority, deepest modes, greatest holding weight |
| Elder | Top 10% | Elevated depth and propagation priority |
| Acolyte | Holds at least one whole UWU outside the percentile bands | Standard local token-tier behavior |
| Initiate | Positive balance below one whole UWU | Basic interaction |
| Unproven | Freshly observed zero balance | Minimal, skeptical interaction |

These capitalized local economic tiers are not the ontology. Every public chat human is an acolyte
in the social sense regardless of holdings; the legacy `Acolyte` tier label merely denotes one local
balance band and grants no operator, Tentacle, registry, or membership authority.

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
- low treasury resources can contribute to dormancy but never require a token spend to preserve
  the Tentacle or its XMTP identity;
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

A final low score produces recoverable `Dormant`. XMTP and ordinary conversation stay online,
Scales evidence continues accumulating, and the runtime periodically asks acolytes and the operator
for activity, UWU, credentials, or other resources. Dormancy creates no token spend, absorption, or
Shutdown intent; the next non-dormant final period wakes the Tentacle automatically.

Legacy hash-bound `Death` history remains readable. Startup converts an unabsorbed pending Death or
locally completed Shutdown into dormancy without changing the XMTP identity and retains the old
intent/receipt as audit history. It never claims to reverse an already-completed external absorption.

A final `PropagationRights` judgment authorizes a child only when the required stake is freshly
observed. If `Nature.growth > 70` and auto-spawn is enabled, the runtime durably creates the
provision intent without operator confirmation. Manual mode uses the same grant and stake evidence
through `/spawn`. The grant may authorize distinct children without an artificial rate, volume, or
expiry quota; each exact child/action and provision receipt is consumed once to reject replay.

Dormancy does not preempt an already-authorized Spawn. Legacy Death preemption remains only for old
persisted lifecycle state and is not reachable from a new low-score judgment.

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

The [Acolyte Branding design](acolyte-branding.md) is a separate closed UWU economy. A controlling
Tentacle pays upward-rounded 0.1% weekly upkeep directly to its immutable acolyte address. A native
compulsory purchase sends 10% to the immutable signed referrer, the remainder to the seller, and a
separate first upkeep payment to the acolyte. Zero-consideration unserved claims pay neither seller
nor referrer. The signed referrer may be the Branding contract itself; in that explicit case its 10%
is intentionally stranded because version 1 has no admin or sweep.

Branding source, tests, and deployment tooling do not mean this economy is live. A funded verified
Base deployment, canonical provenance, frontend controller routing, and a real browser/XMTP exercise
remain open. Branding settlement also does not activate the following general local revenue-split
core.

Valid Venice-key provisioning is a separate authenticated earning event. When no key exists, an
XMTP sender may provision one with `/venice-key <api-key>`. Rust accepts a reward only after the
candidate authenticates to Venice's live catalog and passes fresh TEE attestation. If the freshly
observed Tentacle treasury has at least the configured amount, it persists a Base UWU transfer
intent bound to the provision message, SDK-authenticated sender address, treasury, token contract,
economic configuration, and exact whole-token amount. The default is 1 UWU through
`CTHUWU_VENICE_KEY_REWARD_WHOLE`. A matching confirmed transfer receipt is consumed once; a queued
intent is not payment, and no raw signing key enters uwubot.

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

This existing local core is not the future Cthuwu governance design. Future participation belongs
to Tentacles, not multiple “Cthulhus,” and a shared wallet-derived input must not be counted more
than once. The ERC-8004/leaderboard milestone exposes Level and an inactive future-influence label;
it does not connect or invent ballot mechanics.

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

The UWU token contract is live. A Branding contract is not deployed, and its frontend routing is not
integrated. Separate runtime signer, staking/burn/reward contracts, authenticated general revenue
source, persisted ballot adapter, payout/application executor, provisioner, and live peer-to-peer
Council/Hermes transports remain external integration work.
