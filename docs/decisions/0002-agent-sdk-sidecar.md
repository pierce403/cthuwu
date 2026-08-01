# 0002: Official Agent SDK behind the Rust command

Date: 2026-08-01  
Status: accepted

## Decision

Ship the first working XMTP backend as the Rust `uwubot` supervisor plus a private Node subprocess pinned to `@xmtp/agent-sdk@2.3.0`. Keep one operator-facing command. Use newline-delimited JSON over subprocess pipes as the transport boundary.

Rust owns product behavior, persistence of contact notes, consent, deduplication, matching, model access, limits, and shutdown. The sidecar owns XMTP identity/database initialization, text-DM filtering, streams, reconnect behavior, and send operations.

## Why

XMTP publishes and documents the Agent SDK as its server-side bot surface. The shared libxmtp implementation is written in Rust, but its crates are an unpublished workspace with active internal API and dependency churn. A direct integration would make Cthuwu responsible for maintaining protocol/database internals before the product flow has passed its first live test.

The process boundary also keeps XMTP types out of the companion core. Replacing the sidecar later does not change contact files, commands, model adapters, or message policy.

## Boundary controls

- Standard output is JSONL only; diagnostics use standard error.
- Rust passes an allowlisted environment, so the transport cannot read model credentials.
- Only one-to-one text messages cross the boundary.
- Inbound and outbound text is limited to 16 KiB.
- At most two requests may wait for Rust; additional messages receive a retry response.
- Request IDs correlate replies; XMTP message IDs are hashed and durably deduplicated.
- Both processes fail closed when stored keys, directories, or XMTP environments disagree.

## Consequences

- Development requires Node 22 in addition to Rust.
- A release directory is not a single file because the XMTP N-API binding must ship beside Node modules.
- The Docker image is the simplest reproducible one-command package.
- Live interoperability must still be tested on the same explicit XMTP environment before a production claim.
