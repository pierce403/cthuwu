# Cthuwu — Features

This file follows the [FEATURES.md specification](https://features.md/). Stability describes the feature as a whole:

- `stable`: production-ready for its current documented scope.
- `in-progress`: implemented in code but still missing a release criterion or live verification.
- `planned`: agreed direction with no complete implementation yet.

## Features

### Static chat website

- **Stability**: stable
- **Description**: A small, friendly browser experience at [cthuwu.app](https://cthuwu.app) that builds to static files and requires no application server.
- **Properties**:
  - Source lives in `web/`; Vite produces `web/dist/`.
  - Pushes to `main` deploy through GitHub Pages.
  - The interface supports keyboard use, narrow screens, visible focus, status announcements, and reduced-motion preferences.
- **Test Criteria**:
  - [x] `npm --prefix web run build` produces static deployable assets.
  - [x] The deployment workflow publishes `web/dist/`.
  - [x] The custom domain serves the application.
  - [ ] Automated accessibility checks cover the primary chat and identity flows.

### Automatic local browser identity

- **Stability**: in-progress
- **Description**: A first-time visitor receives a randomly generated local identity without connecting an existing wallet.
- **Properties**:
  - The browser creates an EOA private key with `crypto.getRandomValues` before configuration or network access.
  - A separate 32-byte compatibility key is persisted, but the UI does not claim it encrypts the current Browser SDK database.
  - A versioned record is namespaced by XMTP environment in local storage and reused on reload.
  - The client attempts to connect automatically when a bot address is configured.
  - Passphrase-encrypted PBKDF2/AES-GCM export and import recover the wallet identity, not message history or necessarily the same XMTP installation.
  - Reset is environment-scoped, confirmed, and explains possible inbox loss and the Browser SDK's unencrypted local database.
- **Test Criteria**:
  - [x] First load creates both random keys without requesting a wallet.
  - [x] Reload returns byte-identical stored keys.
  - [x] Development, production, and local identities are isolated.
  - [x] Legacy complete records migrate; partial or corrupt records fail closed.
  - [x] Encrypted export/import round-trips and rejects a wrong passphrase or environment.
  - [x] Reset leaves unrelated and other-environment storage untouched.
  - [ ] A full browser automation test verifies persistence across an actual page reload.

### Browser-to-Cthuwu XMTP direct message

- **Stability**: in-progress
- **Description**: The browser creates a one-to-one XMTP conversation with the configured Cthuwu identity.
- **Properties**:
  - `VITE_XMTP_ENV` selects `dev`, `production`, or `local`.
  - `VITE_XMTP_BOT_ADDRESS` accepts a validated Ethereum address or ENS name.
  - The client loads existing text history, streams new messages, deduplicates overlapping history/stream delivery, and sends text.
  - Failed sends preserve the draft; inbound rendering uses text nodes rather than HTML.
  - Groups, attachments, reactions, and read receipts are outside the first slice.
- **Test Criteria**:
  - [x] Configuration rejects unknown environments and malformed destinations.
  - [x] The Browser SDK client creates or loads an XMTP client and DM.
  - [x] ENS names resolve before DM creation.
  - [x] Existing and streamed text messages share one deduplicated render path.
  - [x] Browser and backend enforce the same 16 KiB text limit.
  - [ ] A real browser message reaches `uwubot` and receives exactly one response.
  - [ ] History and both identities survive a complete live restart test.

### Single Rust backend command

- **Stability**: in-progress
- **Description**: The operator runs one Rust binary, `uwubot`, for the Cthuwu agent.
- **Properties**:
  - Cargo exposes one application binary named `uwubot`.
  - Rust owns contact memory, consent, matching policy, model access, limits, and lifecycle.
  - `uwubot` supervises the pinned official `@xmtp/agent-sdk@2.3.0` transport as an implementation detail.
  - The transport atomically creates or loads a dedicated wallet key and encrypted XMTP database under `UWUBOT_DATA_DIR`.
  - Environment markers prevent silent development/production state reuse.
  - A Docker image packages Rust, Node, and the XMTP native binding behind the same `uwubot` entrypoint.
  - A hidden stdin harness exercises contact behavior without a network.
- **Test Criteria**:
  - [x] Cargo exposes only the `uwubot` binary.
  - [x] The stdin harness processes multiple messages for a supplied inbox ID.
  - [x] The Agent SDK, Node version, and package graph are locked.
  - [x] Identity creation is atomic, reusable, permission-restricted, and environment-locked in unit tests.
  - [x] Rust passes an allowlisted environment that excludes model credentials.
  - [x] SIGINT/SIGTERM close protocol input, permit graceful Agent SDK shutdown, and force-kill only after a timeout.
  - [ ] The container image builds in CI.
  - [ ] Reconnect and graceful shutdown are verified against a live XMTP stream.

### One-to-one conversation processing

- **Stability**: in-progress
- **Description**: Each inbound text DM is processed once and receives at most one Cthuwu response.
- **Properties**:
  - The Agent SDK filters self-authored messages; the sidecar forwards only direct text messages.
  - Inbound content remains data and never becomes a shell command or filesystem path.
  - Inbound and outbound text is limited to 16 KiB.
  - Opaque XMTP message IDs are SHA-256 hashed into durable replay tombstones.
  - Rust processes globally in sequence; the sidecar permits at most two pending requests and returns a friendly busy response beyond the bound.
  - Model calls time out after 45 seconds and output is bounded to 4,000 characters before the final transport bound.
- **Test Criteria**:
  - [x] Oversized text is rejected before contact onboarding.
  - [x] Groups and non-text content do not cross the JSONL boundary.
  - [x] Replayed message IDs do not produce duplicate replies across store instances.
  - [x] Self-authored messages are filtered by the pinned Agent SDK.
  - [x] Pending work and JSONL line/reply sizes are bounded and tested.
  - [x] Sequential processing prevents concurrent contact-file corruption in this release.
  - [ ] Per-sender rate limits are configurable and tested.

### Per-inbox contact notes

- **Stability**: in-progress
- **Description**: Cthuwu maintains a human-readable Markdown record for every inbox it meets.
- **Properties**:
  - The default path is exactly `contacts/<inbox-id>.md`.
  - Inbox IDs are lowercase hexadecimal and validated before becoming filenames.
  - Notes include inbox ID, first-seen time, last-seen time, onboarding stage, and sharing state.
  - Personal answers are blockquoted so their Markdown cannot alter note structure.
  - Writes use a restricted temporary file, atomic rename, file sync, and directory sync where supported.
  - Loads reject symlinks and mismatches between filename and note metadata.
  - `contacts/` and runtime state are excluded from git.
- **Test Criteria**:
  - [x] First contact creates the expected filename.
  - [x] Path traversal and malformed inbox IDs are rejected.
  - [x] Multiline answers survive save/load and cannot create note sections.
  - [x] Every accepted message updates `last_seen`, including completed chats.
  - [x] The caller can inspect, correct, export, and delete their note through XMTP commands.
  - [ ] Crash-injection tests prove interrupted-write recovery.
  - [ ] Contact updates are verified on Linux, macOS, and Windows.

### Contact discovery conversation

- **Stability**: in-progress
- **Description**: Cthuwu gets to know a new person through a gentle conversation relevant to a future resource-sharing network.
- **Properties**:
  - Cthuwu asks for a chosen name, hopes and dreams, resources they may enjoy sharing, and resources or support they need.
  - Stored notes contain user-provided statements, not model guesses presented as facts.
  - `/skip`, `/set`, `/profile`, and `/forget confirm` provide decline, correction, inspection, and deletion.
  - Sharing consent requires an explicit yes or no; an ambiguous answer repeats the question.
  - Cthuwu does not pressure people to disclose sensitive information or contribute resources.
- **Test Criteria**:
  - [x] A new inbox starts at the name question.
  - [x] Each valid answer advances the deterministic onboarding state.
  - [x] Completion persists all four categories and sharing state.
  - [x] A person can explicitly skip any question.
  - [x] A person can correct an earlier answer with `/set`.
  - [x] A person can delete their contact note with a confirmed command.
  - [x] Ambiguous sharing consent does not silently opt in or complete onboarding.
  - [ ] Model-generated summaries retain provenance and uncertainty if summaries are added later.

### Cthuwu personality and model adapters

- **Stability**: in-progress
- **Description**: Cthuwu is a cute eldritch buddy with a consistent personality and a configurable language model.
- **Properties**:
  - Persona prompts are separate from XMTP transport and contact persistence.
  - Deterministic local behavior is the default.
  - Ollama and generic OpenAI-compatible chat-completions endpoints are configured without code changes.
  - The operator must explicitly select a provider before any message content leaves the machine.
  - Profile text is labeled as untrusted user data, not injected as a system message.
  - Model output cannot execute tools or mutate files and is UTF-8 safely bounded.
- **Test Criteria**:
  - [x] Deterministic tests verify stable, non-echoing core behavior.
  - [x] OpenAI-compatible and Ollama configuration is implemented behind one adapter.
  - [x] Logs omit message bodies and credentials by default.
  - [x] Provider failure produces a useful response without losing contact state.
  - [ ] A live local Ollama request passes with bounded context and output.
  - [ ] A live selected remote provider request passes without credential leakage.

### Resource-sharing network

- **Stability**: in-progress
- **Description**: Help people discover mutually useful matches between what participants need and what they willingly offer.
- **Properties**:
  - Contributions are opt-in, revisable, pausable, and revocable.
  - Both people must opt in before either appears in a suggestion.
  - Suggestions show bounded chosen names and exact overlapping terms, never inbox IDs.
  - Consent text explains that chosen names and matching terms may be shown to other opted-in people.
  - Suggestions are not commitments or automatic introductions.
  - Sensitive traits are not inferred for matching.
- **Test Criteria**:
  - [x] `/set`, `/share on|off`, and `/pause`/`/resume` revise and control participation.
  - [x] A proposed match cites compatible need/offer terms.
  - [x] Bilateral opt-in is required and skipped fields cannot create false matches.
  - [x] Inbox IDs are absent from suggestions and display names are single-line and bounded.
  - [ ] Both parties separately approve before contact details or conversation context are shared.
  - [ ] Needs and offers support explicit freshness, fulfillment, and expiration.

### Round-robin introductions

- **Stability**: planned
- **Description**: Expand beyond one-to-one Cthuwu conversations with a fair, bounded round-robin process for introductions or resource opportunities.
- **Properties**:
  - One-to-one onboarding remains functional independently.
  - Eligibility and ordering rules are explicit and inspectable.
  - A participant can pause or leave the rotation.
  - Failed or declined introductions do not permanently penalize a participant.
  - The system prevents rapid repeated introductions and harassment.
- **Test Criteria**:
  - [ ] Deterministic tests demonstrate fair rotation across eligible contacts.
  - [ ] Paused, blocked, or opted-out contacts are excluded.
  - [ ] Rotation state survives restart without duplicate introductions.
  - [ ] Per-contact and global introduction rate limits are enforced.

### Privacy, consent, and data lifecycle

- **Stability**: in-progress
- **Description**: Give participants meaningful control over identity and personal information stored by Cthuwu.
- **Properties**:
  - Onboarding explains that contact notes are stored locally by the operator.
  - People can inspect, correct, export, and delete their contact note.
  - Replay tombstones contain only hashed opaque message IDs, not sender inbox IDs or message bodies.
  - Browser backups encrypt identity material; normal logs contain no keys or message bodies.
  - Development and production identities and databases cannot share a directory silently.
  - XMTP-delivered copies and the Browser SDK database are outside local contact-note deletion.
- **Test Criteria**:
  - [x] Onboarding includes a concise storage explanation.
  - [x] `/profile` returns only the authenticated caller's stored profile.
  - [x] `/set` updates only the authenticated caller's note.
  - [x] `/forget confirm` removes the caller's contact note.
  - [x] Environment-mismatch startup fails closed in Rust and the XMTP sidecar.
  - [ ] Configurable retention removes expired contact notes and replay tombstones with non-sensitive audit output.
  - [ ] Backup and restore procedures are tested for the complete backend data directory.

## Release gate for the first working slice

The initial end-to-end release is ready when all of these are checked:

- [x] A fresh browser creates and persists a local identity automatically in unit/build verification.
- [x] `uwubot` creates or loads persistent XMTP identity material in unit verification.
- [x] Both sides require the same explicit XMTP environment and fail closed on stored mismatch.
- [ ] A real browser sends a unique text message and receives exactly one reply.
- [ ] That live message creates the corresponding `contacts/<inbox-id>.md` file.
- [x] Contact onboarding answers and dedupe state survive store reconstruction in tests.
- [x] Duplicate delivery produces no second reply in tests.
- [x] Normal application diagnostics contain no keys, credentials, or message bodies.
- [ ] The web build, Rust suite, sidecar suite, container build, and a real XMTP end-to-end test all pass in CI.
