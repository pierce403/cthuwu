# UWU token observance and economics

## Status

The runtime has a local, read-only ERC-20 observation path for the planned transferable **UWU**
token on Base mainnet (chain ID `8453`). Token observation is enabled by default, but remains
inactive until an operator configures the deployed contract address. No token balance or stake is
required to start a Tentacle, and the default interaction tier is `unproven`, so token observation
does not create a startup gate.

This phase does not deploy a token, stake funds, sign transfers, spend tokens, or keep a private key.
It also does not turn UWU holders into authenticated operators. XMTP operator authority continues to
come only from the existing exact-inbox ACL.

## Launch parameters and unresolved supply choice

| Parameter | Current decision |
|---|---|
| Name | `UWU` |
| Symbol | `UWU` |
| Chain | Base mainnet, chain ID `8453` |
| Decimals | `18` by default; configurable to match the deployed contract |
| Transferability | Standard transferable ERC-20 |
| Minimum stake to start | `0` |
| Contract address | Supplied after deployment; never hard-coded |
| Requested supply | `1,000,000,000` UWU |

The requested one-billion supply is **not** the current Clanker v4 standard. Clanker's current v4
deployment documentation defines a fixed standard supply of **100,000,000,000 tokens with 18
decimals**. Launch therefore needs an explicit choice: use the current standard 100-billion Clanker
supply, or use a custom/nonstandard deployment path for the requested one-billion supply. Runtime
normalization defaults to one billion. Set `--token-total-supply 100000000000` (or the corresponding
environment variable) for a standard current Clanker v4 launch; configured decimals and supply must
match the deployed contract.

Clanker's standard creator rewards are liquidity-pool/swap fees. They are not an ERC-20
fee-on-transfer mechanism. A transfer tax would require a custom contract and separate review;
Cthuwu does not assume one. See the current
[Clanker v4 deployment configuration](https://github.com/clanker-devco/DOCS/blob/main/references/core-contracts/v4/deployment-config.md),
[token implementation reference](https://github.com/clanker-devco/DOCS/blob/main/references/core-contracts/clankertoken-v3.1.0-and-v4.0.0.md),
and [creator rewards documentation](https://github.com/clanker-devco/DOCS/blob/main/general/creator-rewards-and-fees.md).

## Local observation path

The Agent SDK resolves an optional EVM address from the transport-authenticated XMTP sender inbox.
The sidecar passes that address as authenticated envelope metadata; message text cannot supply or
override it. XMTP inboxes without an EVM identifier continue normally without a token observation.

Each Tentacle owns an in-process `TokenObservance` cache. It validates the configured nonzero
20-byte contract and holder addresses, rechecks that the RPC endpoint reports chain ID `8453`
before every balance read, and then issues the standard read-only `eth_call` for
`balanceOf(address)` at `latest`. There is no central token registry, global holder service, or
shared tier authority. The RPC endpoint supplies chain data, but every cache and tier decision
belongs to the observing Tentacle.

The wire call follows Base's documented
[`eth_call` API](https://docs.base.org/base-chain/api-reference/ethereum-json-rpc-api/eth_call); it
does not create a transaction or consume gas from a Cthuwu-controlled account.

The running DM path currently observes authenticated EVM addresses for public one-to-one senders.
`TokenEye` can observe any validated address, but Council-member, sibling-lineage, and operator
acolyte enumeration are not connected because those live identity/address adapters do not yet exist.
They must use the same transport-authenticated or cryptographically bound address path when added;
they must not infer wallets from display names or message text.

Fresh values are cached for the configured observation interval. A failed refresh retains an old
value as stale diagnostic context instead of changing it to zero. Ordinary observations use a
per-holder negative-cache backoff derived from that interval and bounded to 1–30 seconds, so one
outage does not hammer Base or serialize unrelated holders behind network I/O. Unknown and stale
observations are neutral: they do not enforce `--min-tier`, modify a response, or add an Engagement
bonus. Ordinary conversation continues when Base or the RPC provider is unavailable.

## Reputation tiers and response behavior

Tiers are recalculated from the positive balances that one Tentacle has observed locally:

| Tier | Local calculation | Default response effect at full intensity |
|---|---|---|
| `whale` | Top 1% of eligible local balances, only with at least 100 eligible holders | Priority/deep-lore treatment and more response depth |
| `elder` | Top 10% after whales, only with at least 10 eligible holders | Elevated conversation depth |
| `acolyte` | At least one whole UWU, outside an available percentile tier | Standard interaction |
| `initiate` | Positive balance below one whole UWU | Focused/basic interaction |
| `unproven` | Observed zero balance | Skeptical, proof-oriented interaction |

Only balances of at least one whole UWU enter the percentile population; dust wallets cannot create
a sample or promote another holder. With the default policy, Whale is unavailable below 100 eligible
local holders and Elder is unavailable below 10. Until a percentile has a meaningful sample,
otherwise eligible holders remain Acolytes. Equal balances receive the same tier because ranking
counts strictly greater balances rather than using address order. Because ranks are local, two
Tentacles with different observations may classify the same address differently; that is
intentional and avoids a central holder registry.

The difference between tiers is Nature-adjustable. With no explicit override, effective intensity
is `100 - cooperation`: a maximally cooperative Tentacle ignores tier differences, while a
competitive Tentacle applies more of them. `--token-tier-intensity 0` disables the response
difference and `100` applies it fully. A configured `--min-tier` can gate known fresh/cached
observations, but defaults to `unproven`; unknown or stale chain state does not become a denial.
Token tier never grants the operator role, operator commands, local tools, or shell access.

## Engagement-only Scales integration

A fresh or unexpired cached public-sender balance can contribute one bounded Engagement bonus for
that conversation. The runtime removes the configured decimals, normalizes whole tokens against the
configured total supply, and bounds the result to 0–10,000 basis points. The period stores the sum
of those per-observation bonuses and divides by **all** conversations, including conversations with
no usable wallet observation. Ordering therefore does not matter and a high-balance final message
cannot replace earlier observations or win through last-writer state.

This public-user balance is evidence about the sender, not the Tentacle. It does **not** activate or
modify Wealth, starvation relief, stake eligibility, Growth rewards, Influence, propagation rights,
or a separate lifecycle scale. Its only possible judgment influence is through the period-averaged
Engagement score and the existing evidence/operator gates; it cannot create a direct or last-writer
lifecycle grant. Current runtime periods keep `token_economics` absent, Wealth inactive, and the
scored-scale set at Engagement only.

`RecordedTokenEconomics` and its Wealth, starvation, stake, reward, and emergency-spend calculations
remain adapter-only library APIs. Before a future node/operator economic adapter can reach runtime
state, its evidence must cryptographically bind at least the holder role and address, chain ID,
token contract, block, observation time, decimals, total supply, and configuration fingerprint. It
must also prevent a later observation from silently replacing an earlier lifecycle-relevant fact.
No such source is wired today, and the existing emergency-survival result remains a recommendation
with no transaction signer or expenditure path. Existing Scales period, evidence, spawn, and
lifecycle limits are tracked in [the guardrail audit](guardrail-audit.md).

## Token-weighted governance core

`token_gov.rs` implements a deterministic local ballot box for content-addressed proposals. Its
closed subjects are Nature adjustment, Council policy, economic policy, and skill-propagation
priority. It accepts one ballot per validated address, bounds raw holding weight to 0–10,000 basis
points, applies Nature-scaled tier multipliers, and calculates quorum and approval without network or
wall-clock input. The permissive default minimum is Unproven; a zero-balance voter may record a
zero-weight ballot rather than being treated as an identity failure.

This is a library-only, advisory core. It is not wired to live Council proposals, does not mutate a
Nature, and has no persistence, RPC, key, transaction, command, process, or operator-authorization
surface. A future adapter must bind each ballot to an authenticated address and an exact trustworthy
balance snapshot before treating the result as evidence. Transferable UWU can never authorize OS
tools or replace the XMTP operator ACL.

## Configuration

| CLI option | Environment variable | Default / meaning |
|---|---|---|
| `--rpc-endpoint <url>` | `CTHUWU_RPC_ENDPOINT` | `https://mainnet.base.org`; read-only Base RPC |
| `--token-contract <address>` | `CTHUWU_TOKEN_CONTRACT` | unset; when set it must be nonzero |
| `--token-decimals <count>` | `CTHUWU_TOKEN_DECIMALS` | `18`; must match the deployed token |
| `--token-total-supply <tokens>` | `CTHUWU_TOKEN_TOTAL_SUPPLY` | `1000000000`; positive whole-token normalization reference |
| `--observe-tokens <true|false>` | `CTHUWU_OBSERVE_TOKENS` | `true` |
| `--observe-interval <seconds>` | `CTHUWU_OBSERVE_INTERVAL` | `60`; must be at least one second |
| `--min-tier <tier>` | `CTHUWU_MIN_TIER` | `unproven` |
| `--token-tier-intensity <0..100>` | `CTHUWU_TOKEN_TIER_INTENSITY` | unset; derive from Nature cooperation |

The RPC URL may contain a provider credential, so the CLI hides its environment value and normal
diagnostics report only `disabled`, `waiting-for-contract`, or `enabled`. Transport errors are
sanitized so the URL is not included. Put any RPC credential in the environment, not on a committed
command line or configuration file.

`--observe-tokens false` is an operational escape hatch: token-only endpoint, contract, decimals,
supply, interval, minimum-tier, and intensity values are ignored rather than allowing stale token
configuration to block an otherwise valid Tentacle. When observation is enabled, the contract must
be a valid nonzero address; the zero address is rejected.

Pre-launch, the default is intentionally usable:

```bash
./uwu.sh
```

After deployment:

```bash
CTHUWU_RPC_ENDPOINT=https://mainnet.base.org \
CTHUWU_TOKEN_CONTRACT=<verified-UWU-contract-address> \
CTHUWU_TOKEN_DECIMALS=18 \
CTHUWU_TOKEN_TOTAL_SUPPLY=100000000000 \
./uwu.sh
```

This is intentionally a non-runnable template: replace `<verified-UWU-contract-address>` with the
verified deployed UWU address. Startup rejects malformed and zero addresses, and every balance
observation rejects an RPC endpoint reporting a chain other than Base mainnet.

## Launch and activation checklist

1. Resolve the supply mismatch: standard current Clanker v4 100 billion, or a reviewed custom path
   for the requested 1 billion.
2. Confirm name `UWU`, symbol `UWU`, 18 decimals, transferability, Base chain ID `8453`, and the
   desired Clanker LP fee/reward configuration.
3. Deploy without giving the observer a private key. Record and independently verify the contract
   address and deployment transaction.
4. Set `CTHUWU_TOKEN_DECIMALS` and `CTHUWU_TOKEN_TOTAL_SUPPLY` to the verified deployed values before
   enabling holder-tier and Engagement effects; current standard Clanker v4 uses `18` and
   `100000000000`.
5. Configure `CTHUWU_RPC_ENDPOINT` and `CTHUWU_TOKEN_CONTRACT`, start one Tentacle, and confirm its
   status says `enabled` without logging the endpoint credential.
6. Test zero, sub-token, ordinary, top-10%, and top-1% balances; an unavailable RPC; a wrong-chain
   endpoint; and cache expiry against the deployed contract.
7. Connect Council-member, sibling-lineage, and operator-acolyte address enumeration only after each
   source has an authenticated wallet binding; keep every resulting cache local to its Tentacle.
8. Add a node/operator Wealth, starvation, stake, reward, or expenditure adapter only when its
   cryptographic evidence binds holder role/address, chain, contract, block, time, decimals/supply,
   and configuration fingerprint, with idempotent history and transaction handling. Until then those
   library dimensions remain inactive or recommendation-only.
