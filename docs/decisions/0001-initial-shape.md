# 0001: Static browser client and local Rust runtime

Date: 2026-08-01  
Status: accepted

## Decision

Build Cthuwu as two independently deployable components:

- a TypeScript browser application whose build output is entirely static;
- a Rust CLI/daemon run by the companion operator.

Use XMTP as the only required messaging bridge. Keep model providers behind an adapter and XMTP behind a transport interface.

## Why

This preserves a serverless public frontend while keeping identity control, model credentials, and future tools on the operator's machine. Rust is a good fit for a durable local process, but direct libxmtp APIs may evolve, so the rest of the program must not depend on them directly.

## Consequences

- The operator's machine must stay online to reply.
- Static hosting cannot hide frontend configuration.
- Browser identity UX and recovery need deliberate product work.
- A fake transport and echo model can test most behavior without network access.
