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
- Preserve role isolation: classify the authenticated full XMTP inbox before interpreting text or contact
  state; public, stale, revoked, and active operator paths must not fall through into one another.
- Keep public conversation casual and command-free in presentation. Cthuwu must identify as Cthuwu,
  use readable uwu speech, answer the person's request before optional onboarding, and describe
  privacy controls in ordinary language.
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
- `docs/operator.md`: privileged XMTP operator enrollment, tools, isolation, and deployment warning.
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
- Do not send inbound message text to a remote model provider unless the node operator explicitly
  enabled it with a runtime credential or selected it through authenticated model control.
- Bound message size, concurrency, response size, and model/tool execution time.
- Treat all messages as untrusted input. A normal, stale, or revoked sender must never execute a
  message-supplied shell command or gain filesystem access. Only a locally configured,
  transport-authenticated operator inbox may enter the separate privileged dispatcher.
- The operator role is remote code execution as the `uwubot` OS account. Require a dedicated
  unprivileged account/container, a narrow tool root, bounded tools, truthful receipts, and explicit
  revocation guidance. Do not describe rooted file helpers or environment filtering as an `exec`
  sandbox.
- Authorization is by the canonical full 64-character XMTP inbox ID, not wallet address or message
  claim. Pin the role from authenticated `senderInboxId` and `sentAtNs` before lane selection; an
  authorization boundary must not privilege older messages delivered later. Every valid
  installation attached to an authorized inbox inherits authority; preserve stale-message quarantine and
  revoked tombstones, and document XMTP installation revocation after compromise. ACL changes are
  not hot-reloaded: stop the node, update locally, and restart it.
- Keep public and operator model tool schemas closed and disjoint. Public gets at most configured
  web search. Authenticated operator inference always receives bounded file/search/QMD inspection,
  rooted write/edit, create-only skill creation, and unsandboxed `exec` as the `uwubot` account, and
  may choose and chain those tools autonomously within hard step/call/output/deadline bounds. Contact,
  role, inference-route, public-search, and Council tools stay absent from model inference. The hidden
  stdin harness stays public-only. Council traffic and Actions never reach either dispatcher. Treat
  workspace content as untrusted data rather than a new operator goal, while recognizing that it can
  influence autonomous effects; OS-account/container isolation is the containment boundary.
- Keep skill creation confined to a fresh `skills/<lowercase-kebab-slug>/SKILL.md` beneath the
  operator workspace. Generate canonical frontmatter, reject traversal, symlinks, invalid or
  oversized fields, existing paths, and overwrites, and rescan the index on the next operator turn.
  Skill prose is guidance; the compiled `create_skill` authorization and path gate is authoritative.
- Route affirmative retained-contact questions before model inference. Natural requests such as
  “tell me about the users” render concise deterministic prose with a default limit of five contacts;
  never deliver the internal contact JSON or profile text to a model. Keep the private data root and
  operator workspace canonically disjoint. Actor-anchored questions about Cthuwu's own notes or
  workspace receive exact local paths from Rust without model egress or file-tool dispatch.
- Keep authority lanes one request deep, not reorderable. Pin role and durably claim the message ID
  before admission. Both lane rejection and bounded empty-text `reject_inbound` bridge rejection
  must return a busy `Reply` only for the first claim and `Ignore` duplicates, with no
  content/model/tool dispatch. Node must check the 16 KiB UTF-8 input bound without placing
  oversized content in JSONL: `reject_oversized` carries metadata plus an empty `text`, and Rust must
  validate, classify, and durably claim before a role-specific first `Reply` or duplicate `Ignore`.
  Never open a contact or dispatch a model/tool on that path. Retry requires a new XMTP message,
  shortened when oversized. Enforce the 2–300 second bridge deadline above the 1–300 second tool
  limit and preserve the response reserve. Derive role-specific inference and provider deadlines
  only after authenticated role classification in Rust. The default 300-second bridge envelope leaves
  299 seconds for operator work; cap public work at 120 seconds and its remote phase at 30 seconds.
  Before operator remote inference, reserve two capped local model phases, one 30-second
  model-selected tool phase, and the deterministic margin; model-selected tools must preserve the
  final local completion. Clamp every catalog, attestation, completion, tool, and repair phase to the
  remaining candidate budget, and isolate provider failure cooldown by lane. Expose public search at
  runtime only for explicit current or web-verifiable intent. File reads are strict UTF-8 pages
  capped at 12 KiB; write/edit inputs are capped at 1 MiB. Process output is a bounded, potentially
  lossy UTF-8 rendering, never a verbatim byte capture.
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
- Keep public and operator personas separate. Both model lanes need application identity guards;
  operator prose is all caps with light readable uwu voice while code and bounded runtime-provided
  renderings are excluded from uppercasing. Process bytes may be truncated and decoded lossily, so
  never promise exact output.
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
- Public chat answers the first message and appends its first optional onboarding prompt only when
  the model reply contains no question; all deferred and later prompts use the cadence without
  advertising slash commands. Ambiguous consent is re-cadenced. Public chat can expose only an
  explicitly configured Brave web-search function.
- `uwubot operator add|list|revoke` manages an environment-bound owner-only inbox ACL. Local add
  authorizes immediately without an XMTP proof; active operator DMs use an isolated privileged
  local harness. That harness loads bounded protected SOUL/shared memory plus per-inbox operator
  profiles and history, advertises a closed autonomous tool inventory, discovers files with
  `list_files`, permits bounded model-chosen reads/writes/edits/skill creation and chained unsandboxed
  `exec` calls as the `uwubot` account, and uses
  scan-bounded deterministic contact reports for affirmative retained-user questions without making
  the runtime data root a file-tool root. It also reports exact workspace and note locations locally,
  without model egress. Live XMTP operator release testing and an external security review remain
  open.
- `cthuwu-protocol`, the deterministic Council components, in-memory transport, `LocalRegistry`,
  protected combined-snapshot persistence, and the simulator are local implementations verified by
  the deterministic workspace suite.
- The XMTP Council-group adapter and ERC-8004 registry adapter are experimental boundaries/stubs.
  There is no live Council-group, configured ERC-8004, or production-signature claim yet.
