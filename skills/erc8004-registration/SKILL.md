---
name: erc8004-registration
description: Inspect, refresh, recover, and explain this Tentacle's canonical Base ERC-8004 registration.
---

# ERC-8004 registration

Use this skill for questions about this durable Tentacle's ERC-8004 identity, agent ID, funding,
registration progress, recovery, or allegiance.

1. Use `/registry-status` for persisted status and `/registry-refresh` when the operator asks for a
   current balance, says funds were sent, or asks the Tentacle to resume. Refresh performs a bounded
   live reconciliation and may automatically advance the existing registration intent.
2. Treat only a confirmed agent ID in active verified state as successful registration. A submitted
   transaction, candidate, funding estimate, or local intent is not a confirmed registration.
3. Explain blockers from the returned phase and receipt. For Base ETH, quote only the runtime's
   exact wallet and shortfall. For RPC access, direct the sender to Infura at
   https://app.infura.io/ and `/base-rpc-key <infura-api-key>`. Never request a wallet private key.
4. Use `/registry-candidates` and `/registry-adopt <agent-id>` only for an ambiguous discovered
   identity. Use `/registry-recover` only when bounded normal discovery cannot resolve an older
   registration; it intentionally performs a more expensive exhaustive scan.
5. Keep the ontology exact: this local durable Tentacle owns one ERC-8004 identity. Singular,
   centerless Cthuwu is the collective and owns no separate agent identity.

Verification requires native status showing canonical Base, the pinned registry, exact Tentacle
wallet, confirmed agent ID, and active verification. Never infer success from conversational memory.
