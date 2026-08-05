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
  - A locally hosted generated mascot anchors a responsive two-column desktop layout and compact
    mobile chat layout; all animation is CSS-based, pauses through a visible persisted control, and
    is disabled by the system reduced-motion preference.
  - Absolute Open Graph and Twitter Card metadata use a purpose-built 1200×630 preview at
    `web/public/cthuwu-og.jpg`.
  - A standalone web-app manifest, opaque any-purpose and maskable icons, Apple touch metadata,
    viewport-safe layout, and a branded offline fallback make the static site installable as a PWA.
  - Chromium's native install event drives a compact dismissible prompt with a 30-day cooldown.
    Safari explains its manual Add to Home Screen/Dock flow and routes people through encrypted
    identity backup first because Apple does not copy local storage into the installed web app.
  - The service worker handles same-origin navigations and two explicit offline assets only. It does
    not cache XMTP traffic, the large Browser SDK/WASM bundle, messages, identity data, or exports.
  - The composer grows to five lines, Enter sends, Shift+Enter inserts a line, incoming messages do
    not pull a reader away from older history, and disconnected states disable message submission.
- **Test Criteria**:
  - [x] `npm --prefix web run build` produces static deployable assets.
  - [x] The deployment workflow publishes `web/dist/`.
  - [x] The custom domain serves the application.
  - [x] DOM tests cover accessible control names, the empty-conversation welcome, hostile text
    rendering, send behavior, and safe stream-loss state.
  - [x] Tests cover manifest identity, icon dimensions/purposes, install-event one-shot behavior,
    dismissal cooldown, standalone suppression, Apple backup guidance, and service-worker scope.
  - [x] A production build emits the manifest, icons, worker, and offline page beside the static app.
  - [ ] Manual install and standalone-layout checks pass on Android Chrome, desktop Chromium,
    iPhone/iPad Safari, and macOS Safari.
  - [ ] Automated accessibility checks cover the primary chat and identity flows.

### Automatic local browser identity

- **Stability**: stable
- **Description**: A first-time visitor receives a randomly generated local identity without connecting an existing wallet.
- **Properties**:
  - The browser creates an EOA private key with `crypto.getRandomValues` before configuration or network access.
  - A separate 32-byte compatibility key is persisted, but the UI does not claim it encrypts the current Browser SDK database.
  - A versioned record is namespaced by XMTP environment in local storage and reused on reload.
  - The deployed browser always uses XMTP `production` and connects automatically to the canonical
    intro Tentacle; no build variable can redirect either value.
  - Passphrase-encrypted PBKDF2/AES-GCM export and import recover the wallet identity, not message history or necessarily the same XMTP installation.
  - Reset is environment-scoped, confirmed, and explains possible inbox loss and the Browser SDK's unencrypted local database.
- **Test Criteria**:
  - [x] First load creates both random keys without requesting a wallet.
  - [x] Reload returns byte-identical stored keys.
  - [x] Development, production, and local identities are isolated.
  - [x] Legacy complete records migrate; partial or corrupt records fail closed.
  - [x] Encrypted export/import round-trips and rejects a wrong passphrase or environment.
  - [x] Reset leaves unrelated and other-environment storage untouched.
  - [x] A full browser automation test verifies persistence across an actual page reload.

### Browser-to-Cthuwu XMTP direct message

- **Stability**: stable
- **Description**: The browser creates a one-to-one XMTP conversation with the canonical intro Tentacle.
- **Properties**:
  - The browser is hard-coded to XMTP `production`; development and local XMTP environments are not
    valid frontend deployment modes.
  - The intro Tentacle is temporarily hard-coded as
    `0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db`, so repository variables cannot silently redirect
    first-contact conversations.
  - A planned Base registry contract will replace the hard-coded bootstrap address with registered
    intro-Tentacle discovery.
  - The client loads existing text history, streams new messages, deduplicates overlapping history/stream delivery, and sends text.
  - Failed sends preserve the draft; inbound rendering uses text nodes rather than HTML.
  - Groups, attachments, reactions, and read receipts are outside the first slice.
- **Test Criteria**:
  - [x] Configuration always returns XMTP `production` and the canonical intro Tentacle.
  - [x] The Browser SDK client creates or loads an XMTP client and opens a DM with that Tentacle.
  - [x] Existing and streamed text messages share one deduplicated render path.
  - [x] Browser and backend enforce the same 16 KiB text limit.
  - [x] A real browser message reaches `uwubot` and receives exactly one response.
  - [x] History and both identities survive a complete live restart test.

### Single Rust backend command

- **Stability**: stable
- **Description**: The operator runs one Rust binary, `uwubot`, for the Cthuwu agent.
- **Properties**:
  - Cargo exposes one application binary named `uwubot`.
  - Rust owns contact memory, consent, matching policy, model access, limits, and lifecycle.
  - The `uwubot operator add|list|revoke` subcommands manage the environment-specific XMTP operator
    ACL locally and exit without starting the transport. The ACL loads at runtime startup; management
    requires stopping and restarting the Tentacle rather than mutating a live process.
  - `uwubot` supervises the pinned official `@xmtp/agent-sdk@2.3.0` transport as an implementation detail.
  - The transport atomically creates or loads a dedicated wallet key and encrypted XMTP database under `UWUBOT_DATA_DIR`.
  - Environment markers prevent silent development/production state reuse.
  - `./uwu.sh` verifies the required runtimes, serializes concurrent setup, installs locked sidecar dependencies as needed, builds both runtime components, prepares dedicated environment-specific owner-only state outside the checkout, prevents concurrent runtimes for one data directory, and then replaces itself with `uwubot`.
  - A Docker image packages Rust, Node, and the XMTP native binding behind the same `uwubot` entrypoint.
  - A hidden stdin harness exercises contact behavior without a network.
- **Test Criteria**:
  - [x] Cargo exposes only the `uwubot` binary.
  - [x] The stdin harness processes multiple messages for a supplied inbox ID.
  - [x] The Agent SDK, Node version, and package graph are locked.
  - [x] Identity creation is atomic, reusable, permission-restricted, and environment-locked in unit tests.
  - [x] Rust passes an allowlisted environment that excludes model credentials.
  - [x] Launcher tests cover environment/data overrides, external owner-only storage, broad-path and symlink rejection before mutation, build and runtime locking, numeric tool versions, build-subprocess secret removal, production-mode dependency installation, pinned Cargo output, and two-start execution from another working directory.
  - [x] SIGINT/SIGTERM close protocol input, permit graceful Agent SDK shutdown, and force-kill only after a timeout.
  - [x] The container image builds in CI.
  - [x] Reconnect and graceful shutdown are verified against a live XMTP stream.

### One-to-one conversation processing

- **Stability**: in-progress
- **Description**: Each inbound text DM is processed once and receives at most one Cthuwu response.
- **Properties**:
  - The Agent SDK filters self-authored messages; the sidecar forwards only direct text messages.
  - Rust classifies the authenticated XMTP sender inbox before inspecting text or touching contact
    state. Public content remains data and never becomes a shell command or filesystem path.
  - Inbound and outbound text is limited to 16 KiB. Node checks the inbound UTF-8 byte length before
    forwarding content; an oversized message normally becomes a metadata-only `reject_oversized`
    frame whose shared-schema `text` field is empty.
  - Opaque XMTP message IDs are SHA-256 hashed into durable replay tombstones.
  - Public and privileged work use separate one-request authority lanes, so one of each may progress
    concurrently. Role is classified and pinned, then the message is durably claimed before lane
    selection. A lane-busy message is never dispatched: its first claim gets a busy reply and a
    duplicate gets `Ignore`.
  - When the Node bridge's pending set is full, one bounded `reject_inbound` handshake carries the
    authenticated metadata—but no message text—to Rust for that same durable claim. Rust returns a
    busy `Reply` only for the first claim and `Ignore` for duplicates, without content, contact,
    model, or tool dispatch.
  - For `reject_oversized`, Rust validates the frame, classifies the authenticated sender, and makes
    that durable claim before returning a role-specific first `Reply` or duplicate `Ignore`. It never
    opens a contact or dispatches content, a model, or a tool. Clients retry with a shorter, newly
    authored XMTP message rather than replaying the old ID.
  - The bridge supplies a locally generated 2–300 second end-to-end deadline. Rust validates it,
    reserves one second for the response, and cancels work when the remaining budget closes.
  - Model calls time out after 45 seconds and output is bounded to 4,000 characters before the final transport bound.
- **Test Criteria**:
  - [x] Oversized text is rejected before contact onboarding.
  - [x] Sidecar tests prove the oversized control frame omits original content; Rust tests cover its
    text-free validation, role-specific first reply, and duplicate `Ignore`.
  - [x] Groups and non-text content do not cross the JSONL boundary.
  - [x] Replayed message IDs do not produce duplicate replies across store instances.
  - [x] Self-authored messages are filtered by the pinned Agent SDK.
  - [x] Pending work and JSONL line/reply sizes are bounded and tested.
  - [x] A public serialization gate prevents concurrent contact-file corruption in this release.
  - [x] Role injection in JSONL is rejected and malformed authenticated sender metadata fails closed.
  - [x] Tests cover pinned pre-lane role snapshots, bridge deadline creation/bounds, and Rust's
    response-reserve calculation.
  - [x] Bridge and bot tests cover the bounded rejection handshake and prove that its durable
    tombstone prevents a later operator replay from invoking a tool.
  - [ ] A transport integration test exercises first-claim `Reply`, duplicate `Ignore`, same-lane
    overload, in-flight cancellation, and late-response suppression together.
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
  - [x] The caller can inspect, correct, control, and delete their note through ordinary XMTP text.
  - [x] Stale, active, and revoked operator paths do not create contact notes.
  - [ ] Crash-injection tests prove interrupted-write recovery.
  - [ ] Contact updates are verified on Linux, macOS, and Windows.

### Contact discovery conversation

- **Stability**: in-progress
- **Description**: Cthuwu gets to know a new person through a gentle conversation relevant to a future resource-sharing network.
- **Properties**:
  - Cthuwu answers a new person's first message. It adds the short optional introduction and name
    question only if the model reply did not already contain a question; otherwise onboarding is
    deferred into the normal conversation cadence.
  - Later prompts for hopes, possible resources, needs, and sharing consent are spaced between
    ordinary conversation, ask only one thing at a time, and accept a pass or topic change.
  - Stored notes contain user-provided statements, not model guesses presented as facts.
  - Ordinary phrases provide decline, correction, inspection, matching, and confirmed deletion;
    public replies do not advertise slash commands. Legacy public forms remain compatible.
  - Sharing consent requires an explicit yes or no. An ambiguous answer leaves consent unresolved
    and re-cadences the prompt instead of repeating it immediately.
  - Cthuwu does not pressure people to disclose sensitive information or contribute resources.
- **Test Criteria**:
  - [x] A new inbox's substantive first message reaches the model and receives an answer.
  - [x] The optional name prompt follows a question-free first answer; when the model already asks a
    question it is deferred, and subsequent prompts obey a conversation-turn cadence.
  - [x] Each valid answer advances the deterministic onboarding state.
  - [x] Completion persists all four categories and sharing state.
  - [x] A person can explicitly skip any question.
  - [x] A person can correct an earlier answer in ordinary language.
  - [x] A person can delete their contact note with ordinary-language confirmation.
  - [x] Public help and onboarding contain no slash-command catalogue.
  - [x] Ambiguous sharing consent does not silently opt in or complete onboarding.
  - [ ] Model-generated summaries retain provenance and uncertainty if summaries are added later.

### Cthuwu personality and model adapters

- **Stability**: in-progress
- **Description**: Cthuwu is a cute eldritch buddy with a consistent personality and a configurable language model.
- **Properties**:
  - Persona prompts are separate from XMTP transport and contact persistence.
  - The public prompt names Cthuwu as the identity, forbids provider/generic-assistant
    self-identification, requires light readable uwu speech, and requires direct answers before
    optional personal questions.
  - Common provider-identity boilerplate triggers one repair attempt and then a fixed Cthuwu
    fallback rather than leaking the configured model identity as the companion.
  - The operator lane independently enforces the same Cthuwu/model distinction and requires its
    recognizable all-caps theatrical voice to retain light readable uwu touches.
  - The compiled preference is Venice `e2ee-deepseek-v4-flash` in TEE-only mode; a configured Venice
    credential is the explicit opt-in that permits remote prompt egress.
  - Before Venice first receives prompt content, the runtime requires the exact live model to
    advertise text, TEE-attestation, and function-calling capabilities and performs a fresh
    nonce/model-bound baseline attestation, cached for at most five minutes. It rejects explicitly
    reported debug mode but does not claim full E2EE or independent Intel/NVIDIA evidence validation.
  - Venice-native system prompting, web search, scraping, citations, and X search are explicitly
    disabled; public web search remains the separate opt-in Brave tool.
  - Missing credentials, attestation/provider errors, exhausted balance, rate limits, and other
    inference failures fall back to credential-free loopback Ollama and then deterministic behavior.
    A locally selected provider never falls forward to a remote provider.
  - Authenticated direct `/provider` and `/model` commands switch the node-wide route using only a
    closed provider set and bounded model IDs. Only names persist; credentials and endpoints do not.
  - Ollama and generic OpenAI-compatible chat-completions endpoints are configured without code
    changes. Loopback clients bypass ambient proxy settings, and Ollama's bounded whole-response
    timeout is configurable.
  - Profile text is labeled as untrusted user data, not injected as a system message.
  - Public model output is UTF-8 safely bounded and can call only optional `web_search`; it cannot
    execute local tools or mutate files.
  - Brave Search is opt-in, uses a separately configured API key, bounds query/response/result data,
    accepts only HTTP(S) result URLs, and returns results as untrusted context.
- **Test Criteria**:
  - [x] Deterministic tests verify stable, non-echoing core behavior.
  - [x] OpenAI-compatible and Ollama configuration is implemented behind one adapter.
  - [x] Venice TEE mode validates exact capabilities and fresh baseline attestation before prompt
    egress, disables Venice's supplemental system prompt, and does not claim full E2EE.
  - [x] Operator-only provider/model switching is bounded, persisted without secrets, denied to
    public users, and clears in-process operator history after a route change.
  - [x] Remote failure falls through loopback Ollama to deterministic local behavior without a
    local-to-remote fallback path.
  - [x] Logs omit message bodies and credentials by default.
  - [x] Provider failure produces a useful response without losing contact state.
  - [x] Tests cover the reported Mistral self-identification failure, public prompt invariants, one
    identity repair, and a public tool schema containing no operator capabilities.
  - [x] The reported operator-lane Mistral self-identification receives one repair attempt and a
    fixed Cthuwu fallback instead of being uppercased and delivered.
  - [x] Search result parsing, URL validation, limits, and credential-bearing endpoint rejection are tested.
  - [ ] A live local Ollama request passes with bounded context and output.
  - [ ] A live selected remote provider request passes without credential leakage.

### Authenticated XMTP operator harness

- **Stability**: in-progress
- **Description**: Let a locally authorized XMTP inbox administer its Tentacle through a separate,
  explicitly privileged agentic harness without exposing those capabilities to public users or the
  Council.
- **Properties**:
  - Local `uwubot operator add` accepts only a canonical full 64-character XMTP inbox ID and creates
    an active environment-specific version-3 record immediately. No XMTP activation proof is
    required.
  - Adding or re-adding an inbox advances its generation and records the local grant time as its
    authorization boundary. Messages authored at or before that boundary cannot use tools.
  - Rust classifies the Agent SDK-authenticated `senderInboxId` and `sentAtNs` before deduplication,
    content parsing, commands, model calls, contact access, or lane selection. That role snapshot is
    pinned for the request; text cannot promote it while it waits or runs.
  - Version-2 pending records migrate to active without a proof, using migration time as the
    boundary. Existing active and revoked records retain their state.
  - Stale messages and revoked inboxes are closed paths: they cannot use tools, fall through to
    public chat, or create contact notes. Revocation persists as a blocking tombstone.
  - Authorization applies to the whole XMTP inbox. Every installation legitimately attached to that
    inbox has operator authority; per-installation authorization is not implemented.
  - Operator replies use an enforced all-caps, theatrical ominous/submissive Cthuwu voice with
    light readable uwu touches while excluding code and bounded runtime-provided tool renderings
    from prose uppercasing. Process streams are
    truncated and decoded as potentially lossy UTF-8, not preserved byte-exactly. The prompt requires
    truthful receipts and explicit failure rather than invented success.
  - Protected instance `SOUL.md` and shared `memories/MEMORY.md` are seeded once; a separate
    `operators/<inbox-id>.md` profile is seeded for each authenticated operator on first use. Each
    request loads a globally bounded snapshot plus the workspace's first project context file,
    workspace `MEMORY.md`, a top-level manifest, and a compact `skills/*/SKILL.md` index; full skills
    remain progressive `read_file` reads. Bounded in-process dialogue history is also isolated by
    operator inbox and cleared on restart.
  - Auto-loaded workspace context is untrusted reference data. A current-message project-inspection
    request coarsely delegates bounded reads across the configured workspace, so context may influence
    selected paths and results may reach the model endpoint. It cannot expose effects or contact access.
    Identity-repair inference runs with an empty tool schema.
  - Model inference receives only bounded `list_files`, `read_file`, literal `rg` search, and optional
    external QMD search. Exact direct `/write`, `/edit`, and `/exec` commands retain the bounded shared
    dispatcher without exposing effectful schemas to workspace prompt injection. Terminal
    `list_users`/`get_user` access is likewise limited to strict runtime routing or direct commands.
    Operator mode deliberately contains no web-search tool.
  - Contact tools parse `ContactStore` rather than widening the operator filesystem root. They
    describe only retained local notes, distinguish observations from unverified user assertions,
    redact inbox IDs by default, expose a continuation cursor, bound note size and directory scanning,
    omit raw DMs/message counts, and terminate without returning contact text to the model. This
    guarantee covers those dedicated tools; unsandboxed direct `/exec` retains the service
    account's ambient filesystem access.
  - File helpers stay under `UWUBOT_OPERATOR_ROOT`, reject parent traversal and direct symlink
    targets, page UTF-8 reads at 12 KiB, cap writes/edits at 1 MiB, and write atomically. The agent
    loop and child processes have hard step, output, and 1–300 second tool-timeout limits, subordinate
    to the bridge's 2–300 second end-to-end deadline.
  - Startup rejects canonical overlap between the operator root and private data root in either
    direction, preventing workspace reads from reaching XMTP identity, contacts, or agent profiles.
  - `exec` starts in the operator root with a secret-stripped environment, but is intentionally
    unsandboxed within the permissions of the `uwubot` OS account. Production operation therefore
    requires a dedicated unprivileged service account or container.
  - QMD is an optional command adapter configured with `UWUBOT_QMD`; absence or failure is reported
    and never treated as success.
  - The stdin harness always forces the public role. Council messages and typed Council Actions have
    no route to operator tools.
- **Test Criteria**:
  - [x] ACL tests cover exact 64-character IDs, immediate local authorization, version-2 pending
    migration, `sentAtNs` fencing, persistence, generation rotation, revocation, environment
    binding, owner-only permissions, and symlink rejection.
  - [x] Public `/exec`-style text is inert, while active operator text reaches only the operator
    harness.
  - [x] Stale messages and revoked records never fall through to public contact handling.
  - [x] The hidden stdin harness remains public even when given an active operator inbox ID.
  - [x] The JSONL protocol rejects a caller-supplied role and preserves `senderInboxId` without
    giving the sidecar authorization logic.
  - [x] Tool tests cover the closed schema, direct dispatch, traversal/symlink rejection, bounded
    reads/writes/edits, process status, timeout/output handling, and API-key removal from child
    process environments.
  - [x] Tests cover protected Markdown seeding without overwrite, per-operator profile/history
    isolation, project memory/context and skill discovery, bounded file listing, and a workspace
    manifest.
  - [x] A natural operator request for users returns retained contacts from a disjoint data root,
    redacts inbox IDs, labels provenance/scope, provides cursor pagination, reports truncation, and
    cannot turn hostile contact text into an exec. Negated, policy, and count-only requests do not
    disclose profiles; common contracted/progressive conversation wording such as “users you've
    been talking to” takes the same terminal route, while generic user-topic wording does not.
    Contact scans and note reads are bounded.
  - [x] Tests reject autonomous tools from auto-loaded context, side effects during identity repair,
    and contact reads after an earlier privileged tool step.
  - [x] Operator prose casing excludes code and bounded tool renderings from uppercase transformation.
  - [ ] A manual release test authorizes, uses, revokes, and rechecks an operator inbox over live XMTP.
  - [ ] An external security review covers XMTP installation compromise/revocation, OS isolation,
    command auditability, and operator-model prompt injection.

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
  - [x] Ordinary-language controls revise, opt into/out of, pause, and resume participation.
  - [x] A proposed match cites compatible need/offer terms.
  - [x] Bilateral opt-in is required and skipped fields cannot create false matches.
  - [x] Inbox IDs are absent from suggestions and display names are single-line and bounded.
  - [ ] Both parties separately approve before contact details or conversation context are shared.
  - [ ] Needs and offers support explicit freshness, fulfillment, and expiration.

### Optional Council mode and standalone compatibility

- **Stability**: in-progress
- **Description**: Allow a standalone Cthuwu to opt into a distributed Council without changing the default one-to-one `uwubot` experience.
- **Properties**:
  - A Cthulhu is a durable identity; a Tentacle is one of its running runtimes; a Council is an XMTP coordination group.
  - Direct user conversations remain one-to-one XMTP DMs.
  - Council traffic is control-plane data only: discovery, routing, leases, governance, heartbeats, and approved propagation.
  - A deployment with no Council configuration follows the existing startup, launcher, sidecar, contact-memory, model, and direct-DM paths.
  - Council configuration cannot expose model credentials to the XMTP sidecar or weaken protected data-directory validation.
  - Council envelopes and typed Actions cannot authorize an operator, enter the operator harness, or
    represent its file/process tools.
- **Test Criteria**:
  - [ ] `uwubot` without Council configuration passes all pre-Council tests and starts in standalone mode.
  - [ ] The browser-to-`uwubot` live DM path still produces exactly one reply and persists identities/contact state.
  - [ ] Council state is absent or idle when Council mode is disabled.
  - [ ] Launcher, sidecar environment allowlist, secret isolation, deduplication, and data-directory tests remain unchanged or stronger.
  - [ ] No Council message contains normal DM text, contact-note contents, model credentials, or private memory.
  - [ ] Council input cannot activate an operator or invoke an operator tool.

### Shared Council protocol crate and envelopes

- **Stability**: in-progress
- **Description**: `cthuwu-protocol` provides small, versioned, validated domain and wire types without transport or inference dependencies.
- **Properties**:
  - Typed IDs cover Cthulhus, Tentacles, Councils, sessions, requests, leases, proposals, messages, incarnations, propagation, invitations, and acknowledgements.
  - XMTP inbox and registry references are bounded and registry domain types are chain/deployment/ABI/revision neutral.
  - `ProtocolVersion` serializes as a semantic string; the initial envelope accepts only `cthuwu-council` version `1.0`.
  - A common envelope binds stable message ID, message type, Council, sender Cthulhu/Tentacle, send/expiry times, sequence, typed payload, and optional signature.
  - The encoded envelope is capped at 64 KiB and nested strings/collections have explicit limits.
  - Tagged payloads cover membership, Tentacles, routing, leases, governance, and propagation; unsupported types fail closed.
  - Signer/verifier traits exist, but the deterministic signer is test-only and makes no production signature claim.
- **Test Criteria**:
  - [ ] Every typed identifier accepts valid prefix/slug forms and rejects malformed, overlong, mixed-case, traversal, empty, or repeated-separator forms.
  - [ ] Protocol versions round-trip and unsupported versions are rejected.
  - [ ] Capability, identity, Tentacle, and every Council payload variant serialize and deserialize without bypassing validation.
  - [ ] Envelopes reject over-64-KiB input, mismatched message type/payload, invalid sender requirements, zero sequence, invalid time bounds, expiry, and unsupported types.
  - [ ] Replay suppression prevents a second effect across state reload.
  - [ ] `cthuwu-protocol` has no XMTP, HTTP/model, filesystem, or production-signing dependency.

### Structured Cthulhu identity, personality, and Tentacles

- **Stability**: in-progress
- **Description**: Model durable Cthulhus separately from their restartable Tentacle runtimes and represent personality as structured policy data.
- **Properties**:
  - Cthulhu identity contains stable ID, display name, versioned role/voice/values/motivations/priorities, risk tolerance, privacy preference, decision tendencies, standing concerns, long-term goals, public-safe operator metadata, registry reference, and Tentacle IDs.
  - Archivist, Hermit, Merchant, Wanderer, Oracle, and Trickster sample personas make deterministic policy decisions without an LLM.
  - Structured personality influences bounded policy but cannot generate unconstrained autonomous goals.
  - A Tentacle records stable ID/owner, explicit XMTP network and inbox endpoint, monotonic incarnation, lifecycle, capabilities, health, capacity/load, visibility, protocol version, and last heartbeat.
  - Lifecycle states are `Starting`, `Ready`, `Draining`, `Unavailable`, and `Stopped`; invalid transitions fail closed.
  - Restart changes the incarnation, not the owning Cthulhu or stable Tentacle ID.
- **Test Criteria**:
  - [ ] Structured identities and all six sample personas validate and round-trip.
  - [ ] The same policy topic produces deterministic and meaningfully different persona positions without a model.
  - [ ] Invalid lifecycle transitions and backward timestamps are rejected.
  - [ ] A newer incarnation must start at `Starting` and permanently fences updates from older incarnations.
  - [ ] Restarting a Tentacle preserves its Cthulhu and Tentacle IDs.

### Capabilities and liveness

- **Stability**: in-progress
- **Description**: Advertise bounded, public-safe routing capabilities and derive liveness with deterministic clocks.
- **Properties**:
  - Manifests include protocol versions, model capability classes, context limits, tools, memory modes, privacy properties, inference locality, capacity, visibility, and supported trust mechanisms.
  - Manifests have no representation for credentials, private endpoints, message content, filesystem paths, or unnecessary hardware inventory.
  - Injected-clock heartbeat evaluation produces `Healthy`, `Suspect`, or `Unavailable` under configured windows.
  - Only the current Tentacle incarnation can update lifecycle, health, capacity, or load.
  - Announcements and heartbeats are bounded and subject to per-sender controls.
- **Test Criteria**:
  - [ ] Capability manifests round-trip and reject duplicates, empty required protocol support, overlong collections, and impossible capacity.
  - [ ] Serialized capability fixtures contain no secret-, endpoint-, or hardware-shaped fields.
  - [ ] Injected-clock tests cover healthy, suspect, unavailable, recovery, and boundary timestamps.
  - [ ] A heartbeat from an older incarnation cannot revive or change a current Tentacle.
  - [ ] Draining, unavailable, stopped, expired, and overloaded Tentacles are excluded from new awards.

### Council transport and registry boundaries

- **Stability**: in-progress
- **Description**: Coordinate locally through an authenticated transport abstraction and operator-managed identity registry while isolating future network adapters.
- **Properties**:
  - `CouncilTransport` supports publish, subscribe, authenticated sender identity, stable transport message IDs, ordering metadata, and replay handling.
  - The in-memory implementation is deterministic and suitable for complete local integration tests.
  - Receivers compare transport-authenticated sender identity with envelope and Cthulhu/Tentacle ownership claims.
  - `AgentRegistry` resolves identities, metadata, endpoints, capability references, provenance-bearing trust signals, endpoint associations, and active status.
  - `LocalRegistry` is the working local implementation; reputation remains a selected signal with provenance rather than a global truth score.
  - XMTP Council-group and ERC-8004 types remain isolated unavailable adapters until concretely configured and tested.
- **Test Criteria**:
  - [ ] In-memory publish/subscribe preserves stable IDs and deterministic ordering metadata.
  - [ ] Duplicate delivery is replay-suppressed and sender mismatch fails before state mutation.
  - [ ] `LocalRegistry` registers, updates, resolves, verifies endpoint association, rejects stale metadata, and persists/reloads.
  - [ ] Trust signals retain provenance and bounds and cannot be treated as an unqualified global score.
  - [ ] The XMTP-group adapter has no misleading live implementation claim.
  - [ ] The ERC-8004 stub returns an explicit unavailable/configuration error without hardcoded chain, deployment, ABI, or draft revision.

### Explainable routing, rendezvous, and leases

- **Stability**: in-progress
- **Description**: Select an eligible Tentacle without exposing conversation content and authorize it through a bounded generation-fenced lease.
- **Properties**:
  - Requests may specify capability/tool/protocol/privacy/local-inference requirements, preferred Cthulhu/Tentacle, session affinity, trust policy, maximum load, and expiry.
  - Hard requirements filter before scoring; explicit user choice never bypasses security, privacy, health, capability, or protocol requirements.
  - Ranking generally prefers explicit choice, valid affinity, healthy home and user-owned Tentacles, capacity, compatibility, selected trust/reputation provenance, lower load, and a deterministic tie-breaker.
  - Decisions return per-candidate eligibility and structured reasons.
  - Rendezvous turns a content-free Council route request into the selected Tentacle endpoint, after which the user opens a direct XMTP DM.
  - A lease binds session, user reference, Cthulhu, Tentacle, incarnation, generation, issue/expiry/renewal times, routing request, issuer, and status.
  - Grant, accept, renew, release, revoke, expire, and failover are explicit; old generation/incarnation work is rejected.
  - Failover never silently copies private memory.
- **Test Criteria**:
  - [ ] Routing rejects expired requests and candidates missing any hard capability, privacy, protocol, trust, health, capacity, or load requirement.
  - [ ] Explicit choice, affinity, home preference, ownership, capacity, reputation provenance, load, and deterministic tie-breaking appear correctly in explanations.
  - [ ] Rendezvous returns only the selected endpoint and never requires a DM body or contact note.
  - [ ] Lease tests cover grant, accept, renew, release, revoke, expiry, and invalid transitions with an injected clock.
  - [ ] Failover produces a strictly greater session generation and rejects the old Tentacle/incarnation/generation.
  - [ ] Affinity survives reload when valid and is ignored with an explanation when invalid.

### Council governance

- **Stability**: in-progress
- **Description**: Let distinct Cthulhus debate and resolve bounded shared documents without overriding local operators.
- **Properties**:
  - Governance separates Constitution, versioned Agenda, competing Strategies, and typed Actions.
  - Constitution changes require stricter policy than ordinary Agenda, Strategy, or Action decisions.
  - Agenda proposals reference a canonical parent hash and competing parents are detected explicitly.
  - Proposals support bounded supporting/opposing arguments, amendment suggestions, votes, abstentions, replacement before deadline, quorum, thresholds, ratification, rejection, and expiry.
  - Default governance requires 50% quorum, 50.01% approval among non-abstaining votes for ordinary documents, and 66.67% approval for Constitution changes; no quorum expires the proposal.
  - One Cthulhu receives one vote even when several of its Tentacles submit traffic.
  - Initial Action types are capability refresh, protocol self-test, local resource summary, and routing scenario evaluation; arbitrary shell commands are impossible to represent.
  - A ratified result is recomputed locally and remains subordinate to operator security policy.
- **Test Criteria**:
  - [ ] Canonical document hashes and Agenda parent hashes are stable across serialization/reload.
  - [ ] Competing or stale Agenda parents cannot silently replace the current Agenda.
  - [ ] Multiple Tentacles cannot create more than one vote for the same Cthulhu.
  - [ ] A newer valid vote replaces rather than adds to the old vote before deadline; duplicate/stale votes have no effect.
  - [ ] Abstention, quorum, threshold, stricter Constitution policy, ratification, rejection, and expiry are deterministic.
  - [ ] Sample personas produce distinct supporting/opposing/abstaining positions for the same proposal without an LLM.
  - [ ] Typed Actions reject arbitrary commands and every execution path rechecks local policy.

### Bounded referral propagation and contribution credit

- **Stability**: in-progress
- **Description**: Grow Councils and spread approved information through a validated multi-level referral tree or DAG without financial recruitment incentives.
- **Properties**:
  - Propagation supports invitations, Agenda summaries, approved Strategies, capability requests, approved resource needs/offers, protocol-upgrade notices, and bounded campaigns.
  - Every item records origin, inviter/invitee, root propagation, parent, depth, path/provenance, payload hash, policy version, creation/expiry, acceptance, acknowledgements, visibility, and revocation.
  - Hard policy ceilings are depth 16, fan-out 64, 128 per-sender items per rate window, 30-day campaign lifetime, 64 list entries, and 16 KiB per bounded item; deployments may configure stricter limits.
  - Each hop independently validates provenance, policy, expiry, depth, fan-out, rate, duplicates, loops, opt-out, blocks, visibility, revocation, and local policy before forwarding.
  - Deterministic strategies include breadth-first, depth-limited, trusted-branch-only, capability-targeted, coarse geography/latency-aware, and reputation-thresholded routing.
  - There is no dedicated field or runtime path for normal DMs, contact notes, private memory, credentials, private capabilities, or precise user location; operator policy forbids placing such data in bounded summary text.
  - Contribution credit is non-financial and direct-only, based on unique useful outcomes such as accepted introductions, selected capability referrals, acknowledged downstream delivery, or completed consented matches.
  - Credit requires the intended recipient's acknowledgement, consumes that acknowledgement once, and is capped at 5 units per outcome, 20 per contributor/campaign, and 512 per campaign.
  - Self-referrals, cycles, duplicate outcomes, ancestors, descendants, and raw recruitment count receive no credit; hard caps limit simple Sybil amplification but do not prove identity uniqueness.
- **Test Criteria**:
  - [ ] Invitation acceptance/rejection is explicit, bounded, expiring, and replay-safe.
  - [ ] Maximum depth, fan-out, sender rate, opt-out, block list, visibility, and campaign expiry are enforced at every hop.
  - [ ] Self-referral, path loops, repeated edges, duplicate forwarding, payload-hash mismatch, and forged/truncated provenance are rejected.
  - [x] Campaign revocation stops future forwards and new credit without erasing audit facts.
  - [ ] Acknowledgements bind the authenticated sender to an exact campaign, payload, edge/outcome, and unique ID.
  - [ ] Useful downstream outcomes produce a structured credit explanation while recruitment alone produces none.
  - [ ] Direct-only attribution, acknowledgement consumption, and per-outcome, contributor/campaign, and total-campaign caps prevent duplicate/descendant credit and bound simple Sybil amplification without claiming personhood.

### Deterministic Council simulator and persistence

- **Stability**: in-progress
- **Description**: Exercise one deterministic local orchestration scenario across the current Council engines and reload its combined snapshot without duplicate effects.
- **Properties**:
  - The scenario covers joins, Tentacle announcements, heartbeats, capabilities, route request/offers/award, lease, failure, failover, proposal, persona arguments, voting, Agenda resolution, invitation, multi-level propagation, limits, suppression, acknowledgements, and outcome credit.
  - Council identity, membership, capabilities, affinity, leases/generations, processed IDs, governance, propagation, acknowledgements, and credit persist below `state/council/`.
  - State files use fixed bounded names, size limits, symlink rejection, owner-only permissions, atomic replacement, file sync, and directory sync where supported.
  - Simulator traffic uses the in-memory transport and clearly marked test authentication; it does not claim live XMTP Council or ERC-8004 behavior.
- **Test Criteria**:
  - [x] One deterministic integration test completes routing, lease failover, governance, and multi-level propagation in one scenario.
  - [x] The report demonstrates all required milestones and gives structured routing, governance, propagation, and credit explanations.
  - [x] Saving and reloading the combined snapshot produces equivalent state; transport replay and engine-specific tests produce no duplicate votes, forwards, acknowledgements, or credit.
  - [x] Persistence rejects traversal names, symlinks, oversized/corrupt state, unsafe permissions, and partial identity mismatch.
  - [ ] Council state remains outside the repository and does not weaken the existing data-directory lock/environment checks.

### Live XMTP Council and ERC-8004 adapters

- **Stability**: planned
- **Description**: Connect the locally tested Council domain to a real XMTP coordination group and an explicitly selected ERC-8004 deployment.
- **Properties**:
  - The XMTP group remains a control plane; ordinary DM content never becomes group content.
  - Production sender authentication/signing must bind endpoints to Cthulhu/Tentacle identity and support rotation/revocation without using the deterministic test signer.
  - The ERC-8004 adapter will select its chain, deployment, ABI, and compatible specification revision through configuration rather than domain types.
  - Only durable public identity, metadata, endpoint/capability references, and provenance-bearing trust signals belong in the registry.
  - Heartbeats, load, leases, sessions, user references, contact memory, and conversation content never go on-chain.
- **Test Criteria**:
  - [ ] A live XMTP group exchanges every supported Council message with authenticated sender identity, replay handling, ordering metadata, and reconnect/reload coverage.
  - [ ] A real browser rendezvous receives an awarded endpoint and continues through a direct user-to-Tentacle DM without Council message content leakage.
  - [ ] A configured ERC-8004 adapter resolves identity, endpoints, capabilities, trust provenance, association, and active status against the selected deployment.
  - [ ] Production canonicalization, signatures/authentication, key rotation, revocation, and downgrade behavior have interoperable test vectors.
  - [ ] Threat-model review confirms no dynamic runtime or private user data is published on-chain or to the Council.

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
  - [x] Ordinary-language inspection returns only the authenticated caller's stored profile.
  - [x] Ordinary-language corrections update only the authenticated caller's note.
  - [x] Ordinary-language confirmed deletion removes the caller's contact note.
  - [x] Environment-mismatch startup fails closed in Rust and the XMTP sidecar.
  - [ ] Configurable retention removes expired contact notes and replay tombstones with non-sensitive audit output.
  - [ ] Backup and restore procedures are tested for the complete backend data directory.

## Release gate for the first working slice

The initial end-to-end release is ready when all of these are checked:

- [x] A fresh browser creates and persists a local identity automatically in unit/build verification.
- [x] `uwubot` creates or loads persistent XMTP identity material in unit verification.
- [x] Both sides require the same explicit XMTP environment and fail closed on stored mismatch.
- [x] A real browser sends a unique text message and receives exactly one reply.
- [x] That live message creates the corresponding `contacts/<inbox-id>.md` file.
- [x] Contact onboarding answers and dedupe state survive store reconstruction in tests.
- [x] Duplicate delivery produces no second reply in tests.
- [x] Normal application diagnostics contain no keys, credentials, or message bodies.
- [ ] The web build, Rust suite, sidecar suite, container build, and a real XMTP end-to-end test all pass in CI.

## Release gate for the deterministic local Council

The local Council milestone is complete only when all of these are checked with direct evidence:

- [x] `cthuwu-protocol` validation, serialization, identifier, envelope, signature-boundary, capability, identity, and Tentacle tests pass.
- [x] In-memory transport proves authenticated sender checks, stable IDs, deterministic ordering, replay suppression, and a hard per-sender publish-rate bound.
- [x] Liveness rejects stale heartbeats/incarnations and routing filters hard requirements before producing a deterministic explanation.
- [x] Lease grant/accept/renew/release/revoke/expiry/failover tests prove generation and incarnation fencing.
- [x] `LocalRegistry` resolves and updates bounded records, verifies endpoint association, preserves trust provenance, and survives reload.
- [x] Governance proves canonical parent hashes, competing-parent detection, vote replacement, one-Cthulhu-one-vote, quorum, thresholds, expiry, and persona disagreement.
- [x] Propagation proves invitation choice, provenance, depth/fan-out/rate limits, loops/duplicates, blocking/opt-out, campaign revocation, acknowledgements, and bounded outcome-based credit.
- [x] One deterministic simulator test covers routing, failure/failover, governance, multi-level propagation, combined-snapshot persistence, and replay without duplicate effects.
- [x] Protected Council persistence passes atomic-write, permissions, symlink/path, size, corruption, and reload tests under the existing data-directory model.
- [ ] The complete pre-Council Rust, sidecar, web, launcher, Docker, audit, formatting, clippy, and direct-XMTP suites still pass.
- [x] Documentation and diagnostics make no live XMTP Council, ERC-8004 deployment, or production-signature claim.
