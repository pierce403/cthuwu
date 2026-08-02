# AGENTS.md — Cthuwu project instructions

## Introduction

You are working with Dean on Cthuwu: a cute eldritch companion that lives locally and talks to people over XMTP. Use your stable agent name when you have one. Keep the companion charming, technically honest, and safe to operate.

## Responsibilities

- Preserve a working path from static browser client to the local Rust process over XMTP.
- Treat private keys, XMTP database material, message contents, model credentials, and contact notes as sensitive.
- Keep the frontend deployable as static files.
- Keep the companion runtime local-first and model-provider agnostic.
- Keep Council mode optional. A standalone `uwubot` with direct user DMs must remain the default and
  must not require a Council group or registry.
- Preserve the control-plane boundary: ordinary DM content, contact notes, private memory, and model
  credentials never enter Council messages.
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
- `cthuwu/crates/cthuwu-protocol/`: validated Council wire/domain types with no transport or inference dependencies.
- `cthuwu/crates/cthuwu-council/`: deterministic local Council domain, adapters, persistence, and simulator.
- `web/`: TypeScript browser client built to static assets.
- `docs/`: architecture, research, decisions, and operating notes.
- `docs/protocol/`: normative local Council protocol, privacy, security, and versioning notes.
- `skills/`: reusable procedures specific to this repository.

## Build and verification

```bash
cargo fmt --manifest-path cthuwu/Cargo.toml --all -- --check
cargo test --manifest-path cthuwu/Cargo.toml --workspace --locked
cargo clippy --manifest-path cthuwu/Cargo.toml --workspace --all-targets --locked -- -D warnings
npm --prefix agent ci
npm --prefix agent run typecheck
npm --prefix agent test
npm --prefix agent run build
npm --prefix web ci
npm --prefix web run typecheck
npm --prefix web test
npm --prefix web run build
```

Do not claim live XMTP interoperability until the corresponding end-to-end release gate in
`FEATURES.md` passes against the same XMTP environment. In particular, deterministic in-memory
Council tests do not prove live XMTP group support, and a registry stub does not prove ERC-8004
interoperability.

## Security rules

- Never print or commit private keys, seed phrases, database encryption keys, API keys, full message history, or generated contact notes.
- Use a dedicated, minimally funded bot identity.
- Store persistent secrets outside the repository with restrictive filesystem permissions.
- Make production and development XMTP environments explicit; never silently cross them.
- Do not send inbound message text to a model provider unless the operator selected that provider.
- Bound message size, concurrency, response size, and model/tool execution time.
- Treat messages as untrusted input. The companion must not execute message-supplied shell commands or grant filesystem access.
- Avoid logging message bodies by default; log identifiers only when operationally necessary.

## Council security rules

- Treat every Council envelope, registry record, capability manifest, route offer, vote,
  acknowledgement, and provenance path as hostile input.
- Validate the encoded size before parsing, then bound every nested string, map, list, path, and
  explanation. Council v1 envelopes are capped at 64 KiB; the existing 16 KiB DM limit remains.
- Require the exact supported protocol/version and tagged message type. Reject unknown message and
  Action variants instead of treating them as generic text or commands.
- Compare transport-authenticated sender identity with envelope Cthulhu/Tentacle claims, ownership,
  Council membership, and required registry/allowlist endpoint association before applying effects.
- Validate send/expiry time, positive sequence, stable message-ID replay state, current Tentacle
  incarnation, capability ordering, lease generation, Agenda parent, vote replacement order,
  campaign policy, and propagation provenance as applicable.
- Persist replay markers and their state changes as one atomic logical effect. Replaying after restart
  must not duplicate leases, votes, forwarding, acknowledgements, or contribution credit.
- Never let an old incarnation heartbeat revive a Tentacle or an old lease generation accept new work.
- Keep production signing behind a signer/verifier boundary. The deterministic signer is test-only;
  do not invent or claim production signatures, endpoint binding, rotation, or revocation.
- A Cthulhu gets one governance vote even if it operates multiple Tentacles. Council ratification
  never overrides local operator security/privacy/resource policy.
- Keep Actions as a closed typed and bounded enum. Never add arbitrary shell commands, executable
  paths, unrestricted URL fetches, prompt-driven tools, or filesystem access.
- Independently validate every propagation hop. Enforce expiry, provenance and payload hashes,
  maximum depth/fan-out, per-sender rate limits, loop and duplicate suppression, opt-out, block lists,
  visibility, revocation, and local policy.
- Do not award contribution credit for raw recruitment or referral ancestry. Current credit is
  direct-only: require a unique useful downstream outcome and intended-recipient acknowledgement,
  consume each acknowledgement once, and enforce the per-outcome, contributor/campaign, and total
  campaign caps. Credit is non-financial and does not increase governance votes.
- Keep registry types chain/deployment/ABI/revision neutral. Do not put heartbeats, load, sessions,
  leases, user references, contact memory, DMs, or credentials on-chain.

## Coding conventions

- Prefer small modules and explicit trust boundaries.
- Keep `cthuwu-protocol` free of transport, inference, filesystem, wall-clock, and production signing
  implementations. Inject these capabilities at the Council boundary.
- Use injected clocks and deterministic IDs in Council tests. Avoid wall-clock sleeps and random
  tie-breaking in protocol/state-machine tests.
- Hard routing requirements filter before scoring. Return bounded structured explanations and use a
  stable deterministic tie-breaker.
- Bind lease acceptance to session generation and current Tentacle incarnation. Failover must not
  silently copy private memory.
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
- XMTP's core implementation is Rust (`libxmtp`), but its direct Rust surface is unpublished and less stable than the platform SDKs. The first release uses `@xmtp/agent-sdk@2.3.0` behind a supervised JSONL subprocess boundary.
- The browser uses a locally persisted random wallet for low-friction chat.
- `uwubot` supervises the XMTP sidecar, creates persistent identity state, and processes direct text
  messages. The manual browser/XMTP `dev` gate passed; a real browser/XMTP CI job remains open.
- `cthuwu-protocol`, the deterministic Council components, in-memory transport, `LocalRegistry`,
  protected combined-snapshot persistence, and the simulator are local implementations verified by
  the deterministic workspace suite.
- The XMTP Council-group adapter and ERC-8004 registry adapter are experimental boundaries/stubs.
  There is no live Council-group, configured ERC-8004, or production-signature claim yet.
