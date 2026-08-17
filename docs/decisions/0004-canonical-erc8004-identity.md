# 0004: Fail-closed canonical ERC-8004 identity recovery

Date: 2026-08-17

## Status

Accepted.

## Context

Commit `54be4717b6f50b16a2c8683b9972a343527889d6` bounded ordinary automatic
ERC-8004 discovery to the most recent 20,000 Base blocks. That was a useful refresh optimization,
but the registration state machine continued to treat an empty bounded result as evidence that the
durable wallet had never registered. If `state/erc8004-registration.json` was absent, stale, or
restored from an older backup, an identity outside that window was invisible and startup could
prepare another `register()` transaction. The observed agent IDs `61766` and `63846` on the same
durable Tentacle wallet are consistent with that failure mode.

Incomplete discovery is not an absence proof. A timeout, range limit, stale index, pagination
failure, malformed response, or partial scan must never move registration toward a mint.

## Decision

One durable Tentacle has exactly one canonical ERC-8004 identity. Among identities positively
proven to represent that same Tentacle, the canonical identity is the lowest numeric agent ID.
Higher IDs are historical aliases: Cthuwu neither destroys nor mutates them merely because they are
duplicates, and it never counts or routes them as additional Tentacles.

Candidate classification requires current canonical-registry verification. A shared owner alone is
insufficient. Strong same-Tentacle evidence combines the current nonzero `agentWallet`, current
authorization or ownership, an exact `cthuwu.tentacle-id` when present, exact Cthuwu
allegiance/protocol, the registration-v1 CTHUWU manifest and production XMTP endpoint, persisted
action provenance, and explicit legacy-migration compatibility. An explicit conflicting Tentacle ID
or endpoint is not silently adopted. Genuinely ambiguous identities remain operator decisions;
proven duplicates do not. Explicit migration adoption must name an ambiguous entry from the
current complete candidate receipt, retain Cthuwu or compatible migration evidence beyond wallet
ownership, pass an unchanged direct inspection, and cannot override a distinct identity already
proven canonical. The selection provenance is persisted before reconciliation.

Every startup directly revalidates the persisted agent and completes an identity-integrity pass
before registration can be prepared. A durable complete-discovery checkpoint may cover history
through a canonical block; startup revalidates that block and the canonical agent directly, then
scans or indexes forward. With no trustworthy checkpoint, pre-mint recovery must establish complete
coverage from the canonical deployment start. Indexed discovery may find candidates efficiently,
but only canonical Base reads can classify or select them.

The state machine distinguishes range-complete refreshes, complete historical recovery, and
incomplete discovery. Only a positive complete historical result with no same-Tentacle or ambiguous
Cthuwu candidate can authorize one registration. The narrow signer independently requires the
bound mint authorization produced by that result. Any uncertainty is recoverable degraded state,
not `Unregistered` mint authority. The degraded startup gate remains closed on ordinary
maintenance, including persisted Register receipt recovery and exact-nonce replay, until a later
exhaustive integrity pass succeeds.

When local state names a higher proven duplicate, startup atomically rewrites the selected and
confirmed binding to the lower ID, clears stale higher-ID transaction/profile projections, records
a bounded repair receipt, and reconciles the canonical identity's wallet, profile, allegiance,
protocol, and Tentacle ID. Repeating the pass after a crash converges to the same lower ID.

## Consequences

Losing local ERC-8004 bookkeeping may make a restart slower, but cannot by itself create a second
identity. Normal maintenance may retain bounded recent discovery because it is not mint authority.
Successful historical coverage is checkpointed so routine restarts need direct revalidation plus
bounded forward discovery rather than replaying all history.

The leaderboard, liveness selection, assignment, routing, new Branding controller selection, UWU,
Level, and future influence collapse a proven duplicate set to its canonical ID. The Branding
contract still verifies the exact controller ID stored in an existing token; the application does
not silently rewrite that on-chain relationship or substitute an alias during eligibility reads. A
shared-wallet warning remains only when the evidence cannot prove that the identities are one
Tentacle.
