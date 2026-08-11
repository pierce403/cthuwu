# ABI and deployment provenance

The ERC-8004 event/function fragments in this directory are deliberately minimal
Graph ABIs generated from the official upgradeable contract sources at commit
[`68fc6765761a10fb26f0692df21c8a6f9d12b1be`](https://github.com/erc-8004/erc-8004-contracts/tree/68fc6765761a10fb26f0692df21c8a6f9d12b1be):

- [IdentityRegistryUpgradeable.sol](https://github.com/erc-8004/erc-8004-contracts/blob/68fc6765761a10fb26f0692df21c8a6f9d12b1be/contracts/IdentityRegistryUpgradeable.sol)
- [ReputationRegistryUpgradeable.sol](https://github.com/erc-8004/erc-8004-contracts/blob/68fc6765761a10fb26f0692df21c8a6f9d12b1be/contracts/ReputationRegistryUpgradeable.sol)
- [Canonical addresses](https://github.com/erc-8004/erc-8004-contracts/blob/68fc6765761a10fb26f0692df21c8a6f9d12b1be/scripts/addresses.ts)

The registration schema and registry semantics follow the current Draft ERC text,
last changed by official ERCs commit
[`503591a6e80e6e1affdd6403341e25269141f046`](https://github.com/ethereum/ERCs/blob/503591a6e80e6e1affdd6403341e25269141f046/ERCS/erc-8004.md)
(source blob `7653a80922c0bf0243669f30e7a2d4aabfe006aa`).

The compatible interface revision is `2.0.0`. Both Base registries are UUPS
ERC-1967 proxies. Deployments and earliest code blocks are pinned in
`networks/base-mainnet.json`; `npm run verify:deployment` checks those facts
against a caller-supplied Base RPC before a production build.

The UWU ABI is the standard ERC-20 surface needed by this subgraph. Its address,
decimals, supply, first-code block, and code hash are likewise pinned.

Do not update an ABI, proxy address, implementation address, version, start block,
or code hash independently. Re-run deployment verification against the new
official revision and commit all related pins together.
