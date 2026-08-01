# AGENTS.md — Cthuwu project instructions

## Introduction

You are working with Dean on Cthuwu: a cute eldritch companion that lives locally and talks to people over XMTP. Use your stable agent name when you have one. Keep the companion charming, technically honest, and safe to operate.

## Responsibilities

- Preserve a working path from static browser client to the local Rust process over XMTP.
- Treat private keys, XMTP database material, message contents, model credentials, and contact notes as sensitive.
- Keep the frontend deployable as static files.
- Keep the companion runtime local-first and model-provider agnostic.
- Keep `FEATURES.md` accurate as requirements or implementation status change.
- Record useful discoveries while they are fresh.
- Work directly on `main` during early development unless Dean asks for a branch or PR.

## Start-of-task loop

1. Read `AGENTS.md`, `FEATURES.md`, `MEMORY.md`, and `SKILLS.md`.
2. Check repository status and recent history.
3. Search the memory index before guessing about an earlier decision.
4. Identify the relevant feature properties and acceptance tests.
5. Make one focused change and verify it.
6. Update relevant features, docs, memory, or skills with durable discoveries.
7. Commit and push completed work to `main`.

## Project map

- `FEATURES.md`: requirements, stability, and acceptance criteria.
- `cthuwu/`: Rust CLI and long-running XMTP companion.
- `web/`: TypeScript browser client built to static assets.
- `docs/`: architecture, research, decisions, and operating notes.
- `skills/`: reusable procedures specific to this repository.

## Build and verification

```bash
cargo fmt --manifest-path cthuwu/Cargo.toml --check
cargo test --manifest-path cthuwu/Cargo.toml
npm --prefix web install
npm --prefix web run typecheck
npm --prefix web run build
```

Do not claim live XMTP interoperability until the end-to-end release gate in `FEATURES.md` passes against the same XMTP environment.

## Security rules

- Never print or commit private keys, seed phrases, database encryption keys, API keys, full message history, or generated contact notes.
- Use a dedicated, minimally funded bot identity.
- Store persistent secrets outside the repository with restrictive filesystem permissions.
- Make production and development XMTP environments explicit; never silently cross them.
- Do not send inbound message text to a model provider unless the operator selected that provider.
- Bound message size, concurrency, response size, and model/tool execution time.
- Treat messages as untrusted input. The companion must not execute message-supplied shell commands or grant filesystem access.
- Avoid logging message bodies by default; log identifiers only when operationally necessary.

## Coding conventions

- Prefer small modules and explicit trust boundaries.
- Keep persona prompts separate from transport and model adapters.
- Use structured errors and actionable CLI messages.
- Add tests around identity persistence, replay/idempotency, message filtering, contact files, and configuration parsing.
- Keep browser accessibility and keyboard use working.

## Durable learning

- Put feature requirements and status in `FEATURES.md`.
- Put decisions in `docs/decisions/`.
- Put current facts and pitfalls in `MEMORY.md` or a linked note.
- Put reusable workflows in `skills/` and index them in `SKILLS.md`.
- Keep notes concise and include commands, versions, and source links where useful.
- Check https://recurse.bot about weekly and record only advice that improves this project.

## Known state

- `@xmtp/browser-sdk` is the browser-side SDK.
- XMTP's core implementation is Rust (`libxmtp`), but its direct Rust surface is lower-level and less stable than the platform SDKs. Isolate it behind a transport trait.
- The browser uses a locally persisted random wallet for low-friction chat.
- The contact engine works through the stdin harness; native libxmtp transport is not yet wired.
