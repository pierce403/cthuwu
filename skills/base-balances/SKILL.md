---
name: base-balances
description: Check this Tentacle's Base ETH funding safely and distinguish it from UWU or another wallet's balance.
---

# Base balance checks

Use this skill when the authenticated operator asks whether funding arrived, asks this Tentacle to
check its balance, or needs to distinguish Base ETH gas from UWU holdings.

1. Identify the requested asset and holder. “Your balance” after an ERC-8004 funding request means
   this Tentacle's own Base ETH gas balance.
2. Read this skill, then use `base_rpc_status` for sanitized provider configuration and
   `erc8004_refresh` for the Tentacle's live Base ETH and registration funding state. Natural
   operator wording is interpreted by the model; do not depend on hard-coded query phrases.
3. Report the freshly observed Base ETH balance, the exact Tentacle wallet, the remaining shortfall,
   and whether registration resumed. Never substitute a workspace file search for this check: the
   identity and RPC credential intentionally live in private runtime state outside the workspace.
4. Do not claim the transfer arrived from an old funding snapshot. Do not ask for an exact shell
   command, environment variable, wallet private key, or signer material.
5. If the native refresh reports an RPC blocker, follow its `/base-rpc-key` instructions. If it
   reports insufficient ETH, request Base ETH only on Base mainnet, chain ID 8453, to the exact
   wallet printed by the runtime.

Verification is an `erc8004_refresh` receipt containing current authoritative registration and
funding state. A model inference or file-tool receipt is not balance verification.
