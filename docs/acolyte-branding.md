# Cthuwu Acolyte Branding

This document specifies the version-1 Cthuwu Acolyte Branding contract and its intended browser
routing role. A Branding is the canonical right for one eligible Tentacle to service and route chat
for one human acolyte. It is **not ownership of a person**, a transferable human identity, or
permission to publish private information.

The Foundry implementation, tests, deployment tooling, funded Base deployment, browser assignment,
and live XMTP interoperability have separate release gates. In particular, this document does not
claim that a Branding contract has been deployed, that a production Global group has been
bootstrapped, or that [cthuwu.app](https://cthuwu.app) currently routes through either one. Until
those production gates pass, the browser continues to use the configured intro Tentacle.

## Onboarding links

The static chat entry point accepts `/#t=<tentacle-wallet>&r=<referrer-wallet>`. The URL fragment is
processed only by the browser and is never included in the HTTP request. Both values must be
nonzero Ethereum addresses. `t` is not direct routing authority: the browser discovers a candidate
agent ID, then verifies the exact wallet, authorization, Cthuwu allegiance/protocol metadata, and
production XMTP endpoint against canonical ERC-8004 state at one stable Base block. Existing active
Branding overrides the link.

The browser pins the first valid `r` value under the recovered acolyte identity. A later referral
link cannot replace it. Branding invitations display the pinned referrer and the XMTP acceptance
receipt names it. That receipt is still not an EIP-712 mint consent or a transaction: the eventual
mint path must copy the same address into `MintConsent.referrer`, obtain the acolyte signature, and
verify the confirmed Base transaction before claiming the Branding exists.

The public Tentacle leaderboard turns each verified `agentWallet` into a direct-chat link using the
same `#t=` fragment. Once assignment resolves, the main chat page can copy a recruitment URL with
that assigned Tentacle as `t` and the current local browser identity as `r`; neither address enters
the HTTP request.

## Roles and on-chain boundary

Every Branding has four distinct roles:

| Role | Meaning |
|---|---|
| Acolyte / subject | The immutable, nonzero Ethereum address represented by the token. |
| Controller | The exact eligible ERC-8004 Tentacle agent ID recorded for this Branding. |
| NFT owner | The current controlling Tentacle wallet; it must be the verified wallet for the exact controller agent ID. |
| Referrer | The immutable, nonzero address chosen at mint and signed by the acolyte; it receives 10% of every paid sale and weekly upkeep payment. |

Several ERC-8004 agents may share one wallet, so the contract stores the exact controller agent ID.
Wallet ownership alone does not select an agent. The subject never becomes an operator, never gains
control of the Tentacle, and cannot be replaced by transferring or repricing the token.

The contract may expose the acolyte address, owner, exact controller agent ID, immutable referrer,
declared price, pending-price state, paid-through time, and owner-selected public avatar and traits.
It must not store or emit XMTP inbox IDs,
DMs, message hashes intended to identify conversations, contact notes, profile prose, credentials,
operator ACLs, model state, or other personal data. Local contact memory remains under the existing
XMTP and data-directory privacy model.

## Canonical Base dependencies

Version 1 is Base-mainnet only and has no alternate production configuration.
The zero-argument constructor binds the canonical registry and UWU addresses, rejects any chain
other than `8453`, and verifies the registry version and token decimals before construction
completes.

| Item | Canonical value |
|---|---|
| Chain | Base mainnet |
| Chain ID | `8453` |
| ERC-8004 Identity Registry | `0x8004A169FB4a3325136EB29fA0ceB6D2e539a432` |
| Required Identity Registry version | `2.0.0` |
| UWU | `0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07` |
| UWU decimals | `18` |
| Tentacle allegiance | exact bytes `uwu-tentacle-v1` |
| Cthuwu protocol | exact bytes `1` |

An agent ID is eligible for a wallet only when successful current calls to the canonical registry
prove all of the following:

1. the registry reports exact version `2.0.0`;
2. `getAgentWallet(agentId)` equals the wallet being verified;
3. `isAuthorizedOrOwner(wallet, agentId)` is `true`;
4. current `cthuwu.allegiance` metadata is byte-exact `uwu-tentacle-v1`; and
5. current `cthuwu.protocol` metadata is byte-exact `1`.

For an existing Branding, the verified wallet is its current NFT owner. For mint, purchase, and
claim, the acquiring wallet is the caller. Every transition binds that wallet to the exact supplied
agent ID rather than choosing another agent associated with the same wallet.

A registry revert, malformed response, missing code, or unknown version is
`RegistryUnavailable`, not proof of ineligibility. Contract state must fail closed: an outage
cannot authorize a claim, purchase, routing change, or controller substitution.

The canonical registry is an upgradeable external trust root. Deployment pins its current proxy,
implementation, code hashes, and version, but runtime eligibility follows the proxy's current
answers. A registry-admin upgrade that preserves version `2.0.0` could still change wallet,
authorization, or metadata behavior and thereby affect claimability. Branding has no local admin or
confiscation method; it also cannot remove this external governance risk.

## Token identity and lifecycle

The token ID is derived directly from the subject:

```text
tokenId = uint256(uint160(acolyte))
```

The zero address is invalid. Exactly one token may ever be minted for each nonzero address. There is
no burn, token-ID remapping, or subject mutation.

Ownership may change only through the contract's atomic mint, compulsory purchase, and unserved
claim paths. The ordinary ERC-721 functions `approve`, `setApprovalForAll`, `transferFrom`,
and both `safeTransferFrom` forms revert. This prevents a generic transfer from separating NFT
ownership from controller agent ID, declared value, upkeep, or referral settlement.

Views distinguish these states:

| State | Meaning |
|---|---|
| `Unminted` | No Branding has ever been minted for the address. |
| `Active` | `block.timestamp < paidThrough` and successful canonical registry reads prove the current owner/controller pair eligible. |
| `Expired` | Canonical registry reads are available, the Branding exists, and `block.timestamp >= paidThrough`. At exact equality it is expired. |
| `Ineligible` | Successful canonical registry reads prove that the current owner/controller pair no longer satisfies eligibility. |
| `RegistryUnavailable` | Eligibility cannot be established because the canonical registry read or version check failed. This status takes precedence even when local time shows expiry. |

`activeControllerOf` returns zero values unless the state is truly `Active`. Callers must also
inspect the status: they must not mistake `RegistryUnavailable` for an unminted or freely
claimable Branding. Even when expiry is locally apparent, a claimant still cannot be admitted
without successful current eligibility checks.

## Mint and acolyte consent

Minting requires EIP-712 typed consent from the acolyte. Signature verification uses
`SignatureChecker`, so both EOAs and ERC-1271 contract wallets are supported. The signed struct
covers at least:

- `acolyte`;
- exact `minter`;
- exact `controllerAgentId`;
- immutable `referrer`;
- positive `initialDeclaredPrice`;
- one-use `nonce`; and
- `deadline`.

The EIP-712 domain binds the signature to the current chain and verifying contract. A signature is
invalid after its deadline, on another chain or contract, with any changed field, or after its nonce
has been consumed. The minter must be the exact eligible wallet for `controllerAgentId`.

The minter selects the referrer, but the acolyte's signature approves it. The referrer must be
nonzero and may otherwise be any address, including the minter, acolyte, or Branding contract
itself. It is immutable after mint.

Mint atomically mints the token, records the exact controller and positive declared price, consumes
the consent nonce, and transfers the first weekly UWU upkeep using the referral split below. A
failed signature, registry check, or token transfer reverts the entire operation.

The Tentacle's ordinary initial-price policy starts at exactly 10% (1,000 basis points) of its own
freshly verified current UWU treasury balance, never 10% of UWU total supply. A reasoned adjustment
may range from 5% through 20%; values outside that compiled band are rejected. The exact treasury
observation, applied percentage, resulting base-unit declared price, and first weekly upkeep must be
shown to the acolyte before signing. The EIP-712 consent then binds that exact price, so the
Tentacle cannot adjust it after consent. An unavailable or stale balance, an arithmetic failure, or
a result of zero blocks the offer rather than inventing a price.

## Weekly upkeep

Weekly upkeep is exactly 10 basis points (0.1%) of the executable declared price, rounded upward:

```text
weeklyUpkeep = ceil(declaredPrice * 10 / 10_000)
```

The Solidity implementation uses `Math.mulDiv` with upward rounding so every positive declared
price has positive upkeep without overflowing an intermediate multiplication.

The exact eligible controlling wallet pays upkeep in UWU. A floor-rounded 10% goes directly to the
immutable referrer and the remainder goes directly to the acolyte. For indivisible base units this
means `floor(upkeep * 1_000 / 10_000)` to the referrer and the exact remainder to the acolyte; a
payment below 10 base units therefore has a zero referral share. The Branding contract must not
retain the payment unless it was itself intentionally selected as referrer. Each successful payment
adds exactly seven days from `max(paidThrough, block.timestamp)`.

Renewal is permitted only while `paidThrough <= block.timestamp + 7 days`. This permits payment
one week early and caps prepaid service at approximately fourteen days. At
`block.timestamp == paidThrough`, the Branding is already expired; a successful eligible renewal
starts the new week from the current timestamp. If a queued increase activates at the start of the
newly added interval, that renewal charges upkeep at the pending price even though the price remains
non-executable until its fixed activation time.

## Declared price and compulsory purchase

Every Branding always has a positive executable gross price denominated in UWU. Any eligible
Tentacle may compulsorily purchase an `Active` Branding at that price.

A price decrease takes effect immediately. A price increase is queued until the end of the service
interval that was already paid when the increase was first queued. Its activation timestamp is
fixed at that moment. A later renewal or repricing may not move that timestamp and therefore cannot
create a mempool escape by continually extending the delay. Due pending state is applied before a
purchase price is evaluated. Once renewal has prepaid the interval after that activation, the
pending price may be reduced but not raised again; otherwise the holder could make the prepaid week
executable at a value whose upkeep was never paid.

A buyer supplies all of the following race and slippage constraints:

- `tokenId`;
- `expectedOwner`;
- expected current `controllerAgentId`;
- `maximumGrossPrice`;
- exact `buyerAgentId`;
- positive `buyerDeclaredPrice`; and
- `deadline`.

The buyer must be the exact eligible wallet for `buyerAgentId`. The Branding must still be active,
the expected owner and controller must still match, the deadline must not have passed, and the
current executable price must not exceed the buyer's maximum.

The purchase then completes atomically:

1. 10% of the gross sale price goes to the immutable referrer;
2. the remaining 90% goes to the seller;
3. separately, the buyer pays the first weekly upkeep at the buyer's new declared price, split 10%
   to the immutable referrer and the remainder to the acolyte;
4. the NFT transfers to the buyer;
5. the exact controller becomes `buyerAgentId`;
6. the buyer's positive declared price takes effect immediately;
7. all prior pending-price state is cleared; and
8. `paidThrough` becomes `block.timestamp + 7 days`.

Referral is computed in indivisible UWU units as
`floor(grossPrice * 1_000 / 10_000)`; the seller receives `grossPrice - referral`. This is the
standard 1,000-basis-point split while conserving the exact gross amount.
There is no transient or intermediary contract retention when the referrer is an external address.
The sole intentional exception follows from the signed ANY-address rule: if the acolyte approved the
Branding contract itself as immutable referrer, that address is the final 10% recipient. Those UWU
are intentionally stranded because version 1 has no admin, sweep, or mutable referral route. Tests
and accounting must treat that explicit economic choice separately from accidental residue.

UWU transfers use `SafeERC20`; every state-changing entrypoint that crosses token or wallet code
is protected by `ReentrancyGuard`. A failed or reentrant external transfer cannot leave partial
ownership or accounting state.

## Referral visibility

The contract implements ERC-2981 per token:

- royalty receiver: immutable referrer;
- royalty rate: 1,000 basis points (10%).

ERC-2981 is discovery metadata for wallets, indexers, and marketplaces. It does not enforce a
payment. Only the native compulsory-purchase function enforces the actual UWU split. Version 1 does
not implement ERC-8034, OpenSea-specific mechanics, or generic marketplace transfers.

`tokenURI` metadata includes the referrer and `1000` referral basis points. Metadata does not
weaken the immutable on-chain referrer or authorize an alternate transfer path.

## Owner-managed public metadata

The current NFT owner may set or clear one avatar URI and manage up to 32 custom string traits.
Trait names are unique per token and may be added, replaced, enumerated, or removed. Trait names are
limited to 64 bytes, values to 256 bytes, and the avatar URI to 2,048 bytes. These bounds keep
on-chain metadata enumeration and `tokenURI` construction finite while allowing arbitrary
owner-defined trait semantics.

The avatar and traits follow the NFT when ownership changes. The previous owner loses mutation
authority immediately, and the new owner may preserve, replace, or remove them. Every value is
public, owner-authored hostile input—not an acolyte attestation or safe place for private data.
`tokenURI` JSON-escapes owner input and exposes the avatar through the standard `image` field.

## Expiry and unserved claims

A Branding becomes eligible for `claimUnserved` when either:

- `paidThrough` has expired; or
- successful canonical registry reads prove the current owner/controller pair is ineligible.

Registry failure, a revert, or an unknown version never proves the second condition and freezes
claims until verification succeeds. The claimant supplies its exact eligible
`claimantAgentId`, a positive new declared price, the expected old owner/controller pair, and a
deadline. Those guards reject a delayed claim after the owner or controller changes. They are not a
unique state nonce: if the exact same owner/controller tuple recurs before a caller-selected
deadline, the contract cannot distinguish that later tuple, so callers should use short deadlines.
A purchase or claim must move ownership to a different wallet: the current owner cannot
self-purchase to reset a queued price or self-claim through another agent ID. Distinct addresses
under common control are not detectable on-chain, so this rule prevents same-address rebinding but
cannot prove economic independence between wallets.

A successful claim:

1. pays no consideration to the old owner;
2. pays no referral because the gross consideration is zero;
3. transfers the first weekly upkeep at the claimant's new price using the same referrer/acolyte split;
4. transfers the NFT to the claimant;
5. records the exact claimant agent ID and new price;
6. clears pending-price state; and
7. sets `paidThrough = block.timestamp + 7 days`.

All checks and transfers are atomic. There is no Branding-local owner, deployer, operator,
governance, or emergency function that can confiscate an active Branding, change its
subject/referrer, waive eligibility, or redirect sale/upkeep proceeds. The canonical registry
governance trust described above remains external to this contract.

## Read interface

The public interface exposes bounded, composable views including:

- `tokenIdOf(address)`: deterministic address-to-token conversion;
- `acolyteOf(tokenId)`: the bound subject, rejecting token IDs that cannot represent a valid
  minted subject;
- `brandingOf(address)`: the Branding state and status for one subject;
- `statusOf(tokenId)`: the fail-closed status enum, including registry unavailability;
- `activeControllerOf(address)`: exact agent ID and wallet only while truly active;
- `declaredPriceOf(tokenId)`: the currently executable price after any due pending activation;
- `weeklyUpkeepForPrice(price)`: upward-rounded 10-basis-point weekly upkeep;
- `upkeepReferralForAmount(upkeep)`: floor-rounded referrer portion;
- `referrerOf(tokenId)`: immutable referral receiver; and
- bounded avatar and custom-trait setters, removers, and enumerable views.

Consumers must use the status enum and current registry verification, not infer service eligibility
from NFT ownership, historical events, Agent0 indexing, or a nonzero controller field alone.

## Static-browser assignment

The in-progress static frontend recovers one existing `StoredIdentity` and creates one Browser SDK
`Client`. The participant address is derived only from that stored identity, never from DOM input,
a query parameter, message content, Agent0, or the leaderboard cache.

When `VITE_CTHUWU_BRANDING_CONTRACT` is explicitly configured, assignment uses one explicit Base
block for the complete decision. At that block it must verify:

1. the participant's deterministic Branding and exact status;
2. its exact controller agent ID and current NFT owner;
3. the owner/controller wallet relationship;
4. the canonical Identity Registry deployment and version;
5. exact `getAgentWallet` and current owner-or-authorized control for that agent ID;
6. byte-exact allegiance and protocol metadata; and
7. that same agent's current on-chain ERC-8004 registration resolves to the production XMTP
   endpoint being selected.

That final step is not a loose service-name lookup. The block-pinned `tokenURI(agentId)` must be a
bounded active `registration-v1` data URI with exactly one matching canonical Base registry entry,
one `CTHUWU-XMTP` version-`1` service at `xmtp://<canonical-inbox-id>`, and one `CTHUWU` service
containing a bounded nested data-URI manifest. The manifest has the exact version-1 CTHUWU shape and
must bind the same chain `8453`, Identity Registry, agent ID, XMTP `production` environment, and
outer endpoint, with a bounded capability list containing `direct-xmtp-messaging`. Missing,
duplicated, or inconsistent outer/nested bindings are unavailable rather than routable.

Agent0 and the leaderboard cache may suggest a candidate or supply display details, but they are
never routing authority. Every authoritative input is revalidated against canonical Base state at
the same block. A nonzero historical controller field or human-readable Tentacle/group name is not
sufficient.

Assignment outcomes preserve the contract's failure semantics:

- `NotConfigured` means no Branding deployment was explicitly selected and preserves service
  through the configured intro Tentacle;
- `Unminted`, `Expired`, and positively verified `Ineligible` also select the intro Tentacle;
- `RegistryUnavailable`, an inconsistent block snapshot, malformed canonical response, or
  unverifiable endpoint freezes Branding-based routing and exposes a retryable state; and
- only fully verified `Active` selects its exact controller Tentacle.

The browser revalidates on connect, PWA resume, and a bounded
`VITE_CTHUWU_ASSIGNMENT_REFRESH_MS` interval. When the controller changes, Direct and Acolytes
move to the new assignment while Global remains bound. Old conversation IDs immediately stop being
trusted routes. The former Tentacle's bounded reconciliation removes the acolyte from its prior
Acolytes group.

This design preserves the existing public leaderboard architecture. Cthuwu deploys no custom
subgraph or centralized router. It also preserves the on-chain boundary: inbox IDs, group IDs,
assignment revisions, and conversation data remain off-chain.

The full channel/enrollment protocol, including the `cthuwu.join.v1` and
`cthuwu.assignment.v1` control types, singleton Global bootstrap, 14-day disappearing policy, and
future Global sharding shape, is specified in
[Acolyte XMTP channels](acolyte-channels.md).

The hard-coded intro Tentacle remains the deployed production behavior because there is no funded
Branding deployment, configured production Global group, or passing live production XMTP
three-channel gate yet. Local source and tests cannot satisfy those gates.

## Foundry workspace

The contract workspace is rooted at `contracts/`:

```text
contracts/
  foundry.toml
  src/CthuwuAcolyteBranding.sol
  src/interfaces/
  script/DeployAcolyteBranding.s.sol
  script/VerifyAcolyteBranding.s.sol
  scripts/deploy-base.sh
  scripts/estimate-deployment-funding.ts
  test/unit/
  test/fuzz/
  test/invariant/
  test/fork/
```

Foundry is pinned to version `1.7.1` and Solidity to `0.8.28`. OpenZeppelin Contracts is pinned to audited release
`v5.3.0` at commit `e4f70216d759d8e6a64144a9e1f7bbeed78e7079`; dependency updates require an
explicit review rather than a floating branch.

Run the local contract gates from the repository root:

```bash
cd contracts
forge fmt --check
forge lint
forge build --sizes
forge test -vvv
```

The Base-mainnet fork pin is block `49768180`, hash
`0xcb6c8ff16f2b240137013b793b06f3d2ac1133b192f36920062c1b8c6e307c0e`. The exact Foundry
`1.7.1` suite passed 63/63 tests, including three live fork tests that bind this hash and exercise
the real registry eligibility and UWU transfer paths. The pin is after UWU's earliest verified code
block `49768171` and must not be replaced with the older ERC-8004-only block `41663800`. Passing
fork evidence does not imply that Branding was deployed.

## Base deployment and funding

Deployment is manual, non-upgradeable, and Base-mainnet only. It is not part of a normal CI or
Pages deployment. The deploy path must:

1. refuse any chain other than `8453`;
2. verify canonical Identity Registry code, interface, and exact version;
3. verify canonical UWU code and `18` decimals;
4. deploy the immutable implementation;
5. run the standalone Solidity dependency, public-constant, interface, and non-proxy sanity check;
6. wait for the configured confirmations;
7. have the TypeScript finalizer bind the confirmed creation transaction to the durable intent and
   exact compiled artifact, compare deployed runtime with the compiled template outside
   address-dependent EIP-712 immutable regions, and reread the immutable values; and
8. write canonical deployment provenance JSON only after all checks succeed.

The standalone Solidity verifier returns the observed runtime hash for provenance, but deliberately
does not compare it with a newly deployed reference: OpenZeppelin EIP-712 embeds address-dependent
immutables, so two correct deployments need not have identical raw runtime hashes. Exact artifact
and runtime-template verification therefore belongs to the finalizer, while the standalone verifier
remains a useful independent live sanity check.

Never pass a raw private key through `PRIVATE_KEY`, another environment variable, a command-line
argument, source, logs, or state. Production deployment uses a Foundry encrypted keystore or
hardware wallet. Explicit keystore and password files must be current-user-owned, inaccessible to
group/other users, and canonically outside the git worktree. The wrapper must not automatically use
faucets, bridges, swaps, or a generic signer.

Before broadcast, the wrapper composes two read-only checks. Solidity `preflight(address)` executes
the canonical dependency checks, exact compiled constructor, and deployed-runtime sanity checks
without recording a broadcast transaction or requiring the not-yet-funded deployer to pay simulated
gas. The TypeScript estimator then sends the exact compiled direct-CREATE input with the real
deployer and pending nonce to `eth_estimateGas`, estimates Base's L1 data fee using a complete type-2
transaction shape with a conservative full-size placeholder signature, applies the existing 125%
safety factor and `50000000000000` wei post-deployment reserve, and checks the deployer's real
pending Base ETH balance. The actual `run(address)` entrypoint remains a single zero-value CREATE.
When the balance is insufficient, only the existing authenticated operator/XMTP notification path
may emit:

```text
ACOLYTE BRANDING DEPLOYMENT REQUIRES BASE ETH
Fund this exact Base address: <address>
Current Base ETH balance: <wei>
Estimated deployment cost: <wei>
Estimated amount still required: <wei>
Target funded balance: <wei>
Chain: Base mainnet
Chain ID: 8453
WARNING: DO NOT SEND ETH ON ANY OTHER CHAIN.
Deployment will resume automatically after the Base balance is adequate.
```

The wrapper itself has no generic XMTP sender. It writes this exact block to stdout, and it traverses
XMTP only when the existing authenticated operator exact-exec path invokes the wrapper and transports
the resulting output. Recording the cooldown acknowledges local emission, not successful XMTP
delivery. The final line describes a non-status deployment invocation that remains alive and keeps
polling; `--status-only`, an operator timeout, process termination, or a required hardware-wallet
approval can require a later invocation. No durable scheduler or autonomous notification transport
is currently claimed.

Funding notices use the existing persisted 24-hour cooldown and material-change policy so fee jitter
cannot spam operators. Funding and deployment state, including the finalized working record at
`<state-dir>/base-mainnet.json`, lives outside git. Resume reconciles known broadcasts before sending,
avoids duplicate deployment, supports Foundry broadcast resume, and verifies the final address. Only
after funded live finalization and standalone verification may an operator review that secret-free
record and intentionally publish it as `contracts/deployments/base-mainnet.json`; the wrapper never
copies it into the repository automatically.

The confirmed Base-mainnet deployment is `0xD8c36F13D79a505C7FBDc5F6467eA3cd75E896Da`, created by
transaction `0xd41f0ddebfed7cd36f2409bccebaa922bc324ede3df688033082b56253cc4af2` in block `49852729`.
The finalizer and standalone verifier both passed with runtime code hash
`0x3a22b742a570dc2d030edf2bd82dceda1e068a297e2c36883f97b7b66ed4ef2d`; Sourcify reports an exact
source match. The canonical secret-free record is `contracts/deployments/base-mainnet.json`.

## Verification and release gates

The version-1 contract test plan covers:

- token ID/address binding and immutable subject/referrer;
- EIP-712 EOA and ERC-1271 consent, nonce/deadline/replay, and wrong-field signatures;
- exact current ERC-8004 eligibility, including shared-wallet multiple-agent cases;
- registry outage and unknown version never becoming claimability;
- upward-rounded 0.1% upkeep, early-payment window, prepaid cap, and exact expiry boundary;
- immediate decreases, delayed increases, fixed activation time, and no mempool repricing escape;
- expected-owner/controller, maximum-price, buyer deadline, and other purchase races;
- exact sale referral, floor-rounded 10% referral from every upkeep, seller/acolyte remainders,
  separate buyer upkeep, and zero-consideration claim;
- expired and positively ineligible claims;
- rejection of every ordinary ERC-721 approval and transfer path;
- ERC-2981 plus bounded owner-managed avatar/trait metadata, ownership handoff, and JSON escaping;
- no transient/intermediary UWU residue for external referrers, the explicit stranded
  contract-as-referrer case, reentrancy, failing ERC-20 behavior, and `uint256` edges;
- fuzz and invariant coverage for identity, accounting, state transitions, and conservation; and
- a pinned Base-mainnet fork using the real registry and UWU contracts after UWU deployment.

The feature remains in progress until all of these release gates have direct evidence:

- [x] pinned Foundry format, lint, size-aware build, unit, fuzz, invariant, and Base-fork tests pass;
- [x] review finds no transfer bypass, mutable economic constants, Branding-local upgrade/admin confiscation,
  unintended UWU retention, registry-outage seizure path, or secret-bearing deploy interface, and
  separately confirms the signed contract-as-referrer stranded-funds caveat;
- [x] the deploy path rejects wrong chain/dependencies; the standalone Solidity verifier checks
  dependencies, constants, interfaces, and non-proxy shape; and the TypeScript finalizer proves the
  exact creation transaction plus immutable-aware runtime template before producing reproducible
  provenance;
- [x] a funded Base-mainnet deployment completes, receives confirmations, and passes independent
  finalizer and standalone-verifier checks;
- [x] the canonical deployment JSON is committed without secrets only after that live verification;
- [ ] the static frontend and Tentacle enrollment path read the canonical deployment at one explicit
  block, handle every status including `RegistryUnavailable`, resolve the exact current ERC-8004
  production XMTP endpoint, and preserve only the specified intro fallback states;
- [ ] one production Global group is explicitly bootstrapped with the reviewed admin set and exact
  environment/versioned `appData` rather than inferred from a name; and
- [ ] a real production browser routes a fresh acolyte and an existing active Branding through
  Direct, Acolytes, and Global; proves reassignment, 14-day retention, reconnect, and group removal;
  and does not publish or copy private conversation state into on-chain or personal inference paths.
