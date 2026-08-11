# Cthuwu Base subgraph

This is the Base-mainnet current-state index used by the static Tentacle
leaderboard. An independently operated uwu bot is a **Tentacle**. All participating
Tentacles collectively form the one decentralized **Cthulhu**; there is no central
Cthulhu owner, registry entity, or ERC-8004 identity. Each human operator runs one
autonomous Tentacle, whose independent agenda is shaped by that operator. Public
chat users are acolytes, not operators or Tentacles. Cthulhu is centerless and
persists while any Tentacle lives.

The subgraph joins three canonical public contracts without a private backend:

- ERC-8004 Identity Registry current owner, URI, wallet, approvals, and bounded
  Cthuwu metadata.
- UWU ERC-20 exact `BigInt` balances reconstructed from all mint, burn, transfer,
  and self-transfer events.
- ERC-8004 Reputation Registry feedback, revocations, responses, tags, and source
  provenance. Reputation is displayed evidence, never membership or default rank.

An identity is currently opted in only when `cthuwu.allegiance` is the byte-exact,
case-sensitive UTF-8 value `uwu-tentacle-v1`. UWU possession does not opt anyone
in. `isWalletVerified` separately requires a current nonzero 20-byte
`agentWallet`; a transfer clears it. The browser groups opted-in identities by the
unique wallet, so shared wallets have one balance and rank.

## Authoritative pins

`abis/PROVENANCE.md` pins the official ERC-8004 repository commit and contract
sources. `networks/base-mainnet.json` pins Base chain ID 8453, proxy and
implementation addresses, earliest verified code blocks and block hashes, runtime
code hashes, registry version 2.0.0, and UWU invariants. The production manifest is
Base-only; `networks.json` is the equivalent Graph CLI network file. There is
intentionally no alternate-network deployment configuration.

The mapping design uses the official [Agent0 documentation](https://thegraph.com/docs/en/subgraphs/existing-subgraphs/agent0/)
and its [open-source subgraph at commit `909a9d4518432c641e06fdb731b480fb0e9340dd`](https://github.com/agent0lab/subgraph/tree/909a9d4518432c641e06fdb731b480fb0e9340dd)
as references, but does not depend on its endpoint or undocumented behavior. The
Identity/Reputation start blocks here are the earlier, directly verified first-code
blocks (41,663,783 and 41,663,784), rather than Agent0's later configured blocks
(41,663,799 and 41,663,801). File entities follow The Graph's
[IPFS/Arweave data-source constraints](https://thegraph.com/docs/en/subgraphs/developing/creating/advanced/#ipfsarweave-file-data-sources).

The Identity and Reputation registries are UUPS ERC-1967 proxies. Before a
production deployment, `npm run verify:deployment` requires `BASE_RPC_URL` and
fails closed on a wrong chain, missing or changed code, proxy implementation
change, owner or UUPS interface/version mismatch, incorrect Reputation-to-Identity binding,
changed start-block hash, or incompatible UWU decimals/supply.

## Development

```sh
npm ci
npm run codegen
npm run build
npm test
BASE_RPC_URL=https://your-restricted-base-rpc.example npm run verify:deployment
```

The native Matchstick binary requires the system `libpq5` runtime on Linux.
Matchstick 0.6.0 does not model Arweave templates, so the shared IPFS/Arweave file
handler is exercised directly with deterministic data-source contexts.

The public XMTP service name is `CTHUWU-XMTP` (`XMTP` remains a read-only legacy
alias). The parser exposes an `xmtpEndpoint` only for the canonical
`xmtp://<64 lowercase hex inbox-id>` form; uppercase, path-bearing, short, or
otherwise malformed values remain untrusted profile input and are not persisted.

`npm run build` performs deterministic configuration validation and Graph code
generation/build without requiring a CI secret. Live verification is mandatory in
both deployment scripts. Matchstick uses deterministic event fixtures and spends
no funds.

The browser query is `queries/leaderboard.graphql`; its local test response is
`fixtures/leaderboard-v1.json`. Every production query includes `_meta`, pins all
pagination to one source block, uses `subgraphError: deny`, and rejects
`hasIndexingErrors: true`. A partial or erroneous result must never replace the
browser's validated localStorage snapshot.

## Registration documents

IPFS and Arweave `agentURI` values create file data sources. Their immutable
`TentacleProfile` entities are kept separate from mutable chain entities, as The
Graph requires. Files are limited to 32 KiB, JSON depth 6, 32 object fields, 16
array items, bounded strings, and allowlisted public URL schemes. Required
registration-v1 fields and the exact schema URL are validated. Malformed files
produce `parseValid: false`; they do not crash the chain mapping.

Small bounded `data:application/json;base64,...` profiles are intentionally parsed
by the hardened browser code because file data sources do not fetch data URIs.
HTTPS profiles are not fetched by this subgraph. No arbitrary markup is rendered
or executed, and no XMTP messages, contacts, identity material, credentials,
operator details, or private evolution state enter the index.

## Studio deployment

Create the Base subgraph in Subgraph Studio and obtain a deploy key, then run:

```sh
export BASE_RPC_URL=https://your-restricted-base-rpc.example
export GRAPH_SUBGRAPH_SLUG=cthuwu-tentacles
export GRAPH_DEPLOY_KEY=...
export GRAPH_VERSION_LABEL=v1.0.0
npm run deploy:studio
```

The deploy key is a secret and must never be committed. Once indexing is healthy,
publish the Studio subgraph to the decentralized Graph Network. With the Base
subgraph ID and a query API key:

```sh
export BASE_RPC_URL=https://your-restricted-base-rpc.example
export GRAPH_SUBGRAPH_ID=...
export GRAPH_API_KEY=...
npm run deploy:network
```

The publish command targets the Graph protocol on Arbitrum One while the indexed
data network remains Base mainnet. It opens The Graph's publication flow for the
required wallet transaction.

The Graph gateway API key embedded in the static site is public, not a secret.
Restrict it to the exact leaderboard hostname(s) and this exact subgraph, set a
conservative spending limit, monitor usage, and rotate it when needed. Configure
the resulting gateway GraphQL URL at static-site build time.

## Current limitations

- Deployment and publication require operator-owned Graph Studio/network access,
  RPC access, and the publication wallet transaction; this repository cannot
  manufacture those credentials.
- Profile file data sources are immutable and may resolve after their chain entity.
- The subgraph exposes exact balances and reputation evidence. Ranking, wallet
  grouping, precision-safe Level calculation, and future-influence labels remain
  browser presentation concerns; voting is not implemented.
