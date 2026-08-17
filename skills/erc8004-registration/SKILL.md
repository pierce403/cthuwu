---
name: erc8004-registration
description: Inspect, refresh, recover, and explain this Tentacle's canonical Base ERC-8004 registration.
---

# ERC-8004 registration

Use this skill for questions about this durable Tentacle's ERC-8004 identity, agent ID, funding,
registration progress, recovery, or allegiance.

1. Use `erc8004_status` for persisted state and `erc8004_refresh` when the operator asks for a
   current balance, says funds were sent, or asks the Tentacle to resume. Refresh performs a bounded
   live reconciliation and may automatically advance the existing registration intent. Slash
   commands remain deterministic recovery controls, not the ordinary natural-language path.
2. Treat only a confirmed agent ID in active verified state as successful registration. A submitted
   transaction, candidate, funding estimate, or local intent is not a confirmed registration.
3. Explain blockers from the returned phase and receipt. For Base ETH, quote only the runtime's
   exact wallet and shortfall. For RPC access, direct the sender to Infura at
   https://app.infura.io/ and `/base-rpc-key <infura-api-key>`. Never request a wallet private key.
4. Keep the mint invariant explicit: a recent, partial, stale-index, timed-out, rate-limited,
   malformed, or otherwise incomplete discovery result never proves absence and cannot reach the
   signer. Startup directly verifies the persisted ID, validates its durable historical checkpoint,
   and scans forward; without a trustworthy checkpoint it must complete canonical-start recovery.
   Use `/registry-recover` to request that deliberate complete integrity pass when needed.
5. This durable Tentacle has one canonical ERC-8004 identity. Among identities positively proven by
   current canonical reads and exact Tentacle metadata/profile/XMTP or compatible migration evidence
   to represent it, the lowest numeric ID wins. Higher duplicates remain on-chain but are ignored.
   Startup repairs a stale higher local binding and exposes `IDENTITY REPAIR` plus ignored IDs in
   status. Do not ask the operator to select among proven duplicates.
6. Use `/registry-candidates` and `/registry-adopt <agent-id>` only for a genuinely ambiguous
   identity in the current complete-discovery receipt that the runtime cannot prove is this
   Tentacle. Adoption requires Cthuwu or compatible migration evidence beyond wallet ownership, a
   fresh unchanged direct read, and no already-proven canonical identity; its durable receipt is
   written before reconciliation. Never adopt an unrelated same-owner NFT from wallet ownership
   alone, and never burn, transfer, or edit a higher NFT merely to clean up a duplicate.
7. Keep the ontology exact: singular, centerless Cthuwu is the collective and owns no separate
   agent identity.

Verification requires native status showing canonical Base, the pinned registry, exact Tentacle
wallet, confirmed agent ID, and active verification. Never infer success from conversational memory.
