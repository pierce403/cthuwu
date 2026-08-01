# Cthuwu — Features

This file follows the [FEATURES.md specification](https://features.md/). Stability describes the feature as a whole:

- `stable`: production-ready for its current documented scope.
- `in-progress`: partially implemented or awaiting end-to-end verification.
- `planned`: agreed direction with no complete implementation yet.

## Features

### Static chat website

- **Stability**: stable
- **Description**: A small, friendly browser experience at [cthuwu.app](https://cthuwu.app) that builds to static files and requires no application server.
- **Properties**:
  - Source lives in `web/`.
  - Vite produces deployable static assets in `web/dist/`.
  - Pushes to `main` deploy through GitHub Pages.
  - The interface remains usable by keyboard and on narrow mobile screens.
- **Test Criteria**:
  - [x] `npm --prefix web run build` produces `web/dist/`.
  - [x] The deployment workflow publishes `web/dist/`.
  - [x] The custom domain serves the application.
  - [ ] Automated accessibility checks cover the primary chat flow.

### Automatic local browser identity

- **Stability**: in-progress
- **Description**: A first-time visitor receives a randomly generated local identity without connecting an existing wallet.
- **Properties**:
  - The browser generates an EOA private key with a cryptographically secure random source.
  - A separate 32-byte XMTP database-encryption key is generated.
  - Keys are namespaced by XMTP environment and persisted in local storage.
  - Returning visits reuse the same keys.
  - The client attempts to connect automatically on page load.
  - Clearing site data removes the identity; the UI must not imply that it is recoverable.
- **Test Criteria**:
  - [x] First load creates both keys without requesting a wallet.
  - [x] Reload reuses the stored keys.
  - [x] Connection starts automatically when a bot address is configured.
  - [ ] An automated browser test verifies identity persistence across reload.
  - [ ] The UI provides export, recovery, and reset controls.
  - [ ] Reset requires confirmation and explains that the old inbox may become inaccessible.

### Browser-to-Cthuwu XMTP direct message

- **Stability**: in-progress
- **Description**: The browser creates a one-to-one XMTP conversation with the configured Cthuwu identity.
- **Properties**:
  - `VITE_XMTP_ENV` explicitly selects `dev`, `production`, or `local`.
  - `VITE_XMTP_BOT_ADDRESS` accepts an Ethereum address or ENS name.
  - The client loads existing text history, streams new messages, and sends text.
  - The initial scope is direct messages only.
  - Groups, attachments, reactions, and read receipts are outside the first slice.
- **Test Criteria**:
  - [x] The browser client creates or loads an XMTP client.
  - [x] ENS names resolve before DM creation.
  - [x] Existing text messages render in the conversation.
  - [x] New text messages stream without refreshing.
  - [ ] A real browser message reaches `uwubot` and receives exactly one response.
  - [ ] History and both identities survive a complete restart.

### Single Rust backend command

- **Stability**: in-progress
- **Description**: The operator runs one Rust binary, `uwubot`, for the Cthuwu agent.
- **Properties**:
  - Cargo exposes one application binary named `uwubot`.
  - Normal operation will initialize or load the companion identity, connect XMTP, process messages, update contacts, and send replies.
  - `UWUBOT_DATA_DIR` relocates runtime state; the default produces `contacts/` in the working directory.
  - `UWUBOT_XMTP_ENV` explicitly selects the XMTP environment.
  - A hidden stdin harness exercises contact behavior without a network.
- **Test Criteria**:
  - [x] Cargo exposes only the `uwubot` binary.
  - [x] The stdin harness processes multiple messages from a supplied inbox ID.
  - [x] Startup refuses to claim an XMTP connection before transport exists.
  - [ ] Native `libxmtp` transport is pinned to a reviewed upstream revision.
  - [ ] `uwubot` creates or loads a dedicated encrypted XMTP identity.
  - [ ] `uwubot` reconnects after transient network failure.
  - [ ] Graceful shutdown persists cursors and leaves contact files valid.

### One-to-one conversation processing

- **Stability**: in-progress
- **Description**: Each inbound text DM is processed once and receives at most one Cthuwu response.
- **Properties**:
  - Only direct text messages enter the first implementation.
  - Inbound content is untrusted and never becomes a shell command implicitly.
  - Messages larger than 16 KiB are rejected with a friendly response.
  - Processing will be deduplicated by XMTP message ID.
  - Rate, concurrency, context, and response sizes must be bounded.
- **Test Criteria**:
  - [x] Oversized text is rejected before contact onboarding.
  - [ ] Non-text content is ignored or receives a supported-content response.
  - [ ] Replayed message IDs do not produce duplicate replies.
  - [ ] Messages from the companion itself do not trigger reply loops.
  - [ ] Concurrent conversations cannot corrupt contact state.

### Per-inbox contact notes

- **Stability**: in-progress
- **Description**: Cthuwu maintains a human-readable Markdown record for every inbox it meets.
- **Properties**:
  - The default path is exactly `contacts/<inbox-id>.md`.
  - Inbox IDs are normalized to lowercase and validated before use as filenames.
  - Notes include the inbox ID, first-seen time, last-seen time, and onboarding stage.
  - Personal answers are blockquoted so their Markdown cannot alter the note structure.
  - Writes use a temporary file and rename rather than overwriting in place.
  - `contacts/` is excluded from git by default because it contains personal information.
- **Test Criteria**:
  - [x] First contact creates the expected filename.
  - [x] Path traversal and malformed inbox IDs are rejected.
  - [x] Multiline answers survive a save/load cycle.
  - [x] User-supplied headings cannot create contact-note sections.
  - [ ] Interrupted writes recover without losing the last valid note.
  - [ ] Contact updates work safely on Linux, macOS, and Windows.
  - [ ] The operator can inspect, correct, export, and delete a contact.

### Contact discovery conversation

- **Stability**: in-progress
- **Description**: Cthuwu gets to know a new person through a gentle conversation relevant to a future resource-sharing network.
- **Properties**:
  - Cthuwu asks what the person wants to be called.
  - Cthuwu asks about their hopes and dreams.
  - Cthuwu asks what skills, knowledge, time, introductions, objects, space, money, or other resources they may enjoy sharing.
  - Cthuwu asks what resources or support they need.
  - Notes distinguish user-provided statements from model inference.
  - The person can decline, skip, correct, or delete an answer.
  - Cthuwu does not pressure people to disclose sensitive information or contribute resources.
- **Test Criteria**:
  - [x] A new inbox starts at the name question.
  - [x] Each answer advances the deterministic onboarding state.
  - [x] Completion writes all four categories to the contact note.
  - [ ] A person can explicitly skip any question.
  - [ ] A person can correct an earlier answer conversationally.
  - [ ] A person can request deletion of all stored contact data.
  - [ ] Model-generated summaries retain provenance and uncertainty.

### Cthuwu personality and model adapters

- **Stability**: planned
- **Description**: Cthuwu is a cute eldritch buddy with a consistent personality, backed by a configurable language model.
- **Properties**:
  - Persona prompts are separate from XMTP transport and contact persistence.
  - Local inference is first-class.
  - OpenAI-compatible HTTP and Ollama are candidate adapters.
  - The operator explicitly selects a provider before any message content leaves the machine.
  - A deterministic adapter remains available for tests.
  - Model output cannot execute tools or mutate files except through narrow, validated application operations.
- **Test Criteria**:
  - [ ] Deterministic tests verify core persona and onboarding constraints.
  - [ ] Ollama can generate a response with bounded context and output.
  - [ ] An OpenAI-compatible endpoint can be configured without code changes.
  - [ ] Logs omit message bodies and credentials by default.
  - [ ] Provider failure produces a useful response without losing the inbound message.

### Resource-sharing network

- **Stability**: planned
- **Description**: Help people discover mutually useful matches between what participants need and what they willingly offer.
- **Properties**:
  - Contributions are opt-in and revocable.
  - Needs and offers retain provenance, freshness, and visibility settings.
  - Potential matches are suggestions, not automatic commitments.
  - Contact details are not disclosed to another participant without permission.
  - Sensitive traits are not inferred for matching.
  - Participants can see why a match was suggested.
- **Test Criteria**:
  - [ ] A participant can publish, revise, pause, and remove an offer.
  - [ ] A participant can publish, revise, fulfill, and remove a need.
  - [ ] A proposed match cites the specific compatible need and offer.
  - [ ] Both parties consent before contact details or conversation context are shared.
  - [ ] Expired or withdrawn resources never appear in new matches.

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
  - [ ] Per-contact and global rate limits are enforced.

### Privacy, consent, and data lifecycle

- **Stability**: planned
- **Description**: Give participants meaningful control over identity and personal information stored by Cthuwu.
- **Properties**:
  - Cthuwu explains that contact notes are stored locally by the operator.
  - People can inspect, correct, export, and delete their information.
  - Retention is explicit rather than indefinite by accident.
  - Backups preserve confidentiality and honor deletion policy.
  - Development and production identities and databases never share a directory silently.
- **Test Criteria**:
  - [ ] Onboarding includes a concise storage and consent explanation.
  - [ ] An XMTP command returns the caller's stored profile.
  - [ ] Correction updates only the authenticated caller's note.
  - [ ] Deletion removes the caller's note and derived indexes.
  - [ ] Retention jobs remove expired data and produce non-sensitive audit records.
  - [ ] Environment-mismatch startup fails closed.

## Release gate for the first working slice

The initial end-to-end release is ready when all of these are checked:

- [ ] A fresh browser creates and persists a local identity automatically.
- [ ] `uwubot` creates or loads its persistent XMTP identity.
- [ ] Both sides use the same explicit XMTP environment.
- [ ] A browser sends a unique text message and receives exactly one reply.
- [ ] The corresponding `contacts/<inbox-id>.md` file is created.
- [ ] Onboarding answers persist across backend restart.
- [ ] Duplicate delivery does not produce duplicate replies.
- [ ] Normal logs contain no keys, credentials, or message bodies.
- [ ] The web build, Rust tests, and an XMTP end-to-end test pass in CI.
