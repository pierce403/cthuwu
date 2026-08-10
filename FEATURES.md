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
  - The launcher and Rust runtime default to XMTP `production`, matching the deployed browser.
    `dev` and `local` remain explicit test-only overrides and never activate implicitly.
  - Rust owns contact memory, consent, matching policy, model access, limits, and lifecycle.
  - The `uwubot operator add|list|revoke` subcommands manage the environment-specific XMTP operator
    ACL locally and exit without starting the transport. The ACL loads at runtime startup; management
    requires stopping and restarting the Tentacle rather than mutating a live process.
  - `operator add` accepts an ENS `.eth` name or full Ethereum address, resolves ENS on Ethereum
    mainnet, and uses the pinned Node SDK to resolve the canonical inbox on the explicitly selected
    XMTP network before the Rust ACL persists it.
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
  - [x] SIGINT/SIGTERM close protocol input and permit graceful Agent SDK shutdown. On Unix the
    sidecar is a process-group leader, and supervisor teardown kills the complete group, including
    forked descendants, even if the direct Node process has already exited.
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
    reserves one second for the response, caps authenticated public work at 120 seconds, and cancels
    work when the applicable lane budget closes. The default bridge envelope is 300 seconds, leaving
    at most 299 seconds of authenticated operator work.
  - Each provider candidate is bounded by the remaining authenticated deadline after an explicit
    local-fallback reserve. Public remote work is capped at 30 seconds; operator Venice defaults to a
    configurable 120-second cap. Operator remote work first reserves two local model phases of up to
    the 75-second safety cap, or a smaller configured Ollama timeout, each; one model-selected tool
    phase of up to 30 seconds; and a one-second deterministic margin: 181 seconds by default, so
    Venice can effectively use about 118 seconds. Individual
    catalog, attestation, completion, continuation, repair, and search phases derive their timeout
    from that candidate. Public completions request at most 300 output tokens and remain bounded to
    4,000 characters before the final transport bound.
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
    nonce/model-bound baseline attestation. Capability validation is cached independently for four
    hours; attestation is refreshed after five minutes, and an attestation failure does not discard a
    fresh catalog result. It rejects explicitly reported debug mode but does not claim full E2EE or
    independent Intel/NVIDIA evidence validation.
  - Venice-native system prompting, web search, scraping, citations, and X search are explicitly
    disabled; public web search remains the separate opt-in Brave tool.
  - Missing credentials, attestation/provider errors, exhausted balance, rate limits, and other
    inference failures fall back to credential-free loopback Ollama and then deterministic behavior.
    A locally selected provider never falls forward to a remote provider.
  - Authenticated direct `/provider` and `/model` commands switch the node-wide route using only a
    closed provider set and bounded model IDs. Only names persist; credentials and endpoints do not.
  - Ollama and generic OpenAI-compatible chat-completions endpoints are configured without code
    changes. Loopback clients bypass ambient proxy settings. Ollama's configured timeout is bounded,
    and routed local model phases have a 75-second safety cap so a larger setting cannot consume the
    continuation reserve. `UWUBOT_VENICE_TIMEOUT_SECONDS` independently configures Venice's
    1–300 second provider cap.
  - Profile text is labeled as untrusted user data, not injected as a system message.
  - Public model output is UTF-8 safely bounded and requests at most 300 output tokens. The runtime
    exposes optional `web_search` only when the current message explicitly asks for current or
    web-verifiable information; a policy-repair completion exposes no tools. Public chat cannot
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
  - [x] Tests cover public/operator remote budgets, the local fallback reserve, budget skips without
    cooldown, lane-aware failure cooldown, stalled Venice and Ollama fallback, phase attribution,
    runtime search-schema gating, and the 300-token public cap.
  - [x] Tests prove catalog success survives attestation failure, attestation refresh does not repeat
    a fresh catalog lookup, and waiting on the shared Venice validation refresh is deadline-bounded.
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
  - Local `uwubot operator add` accepts an ENS `.eth` name or full Ethereum address and creates an
    active environment-specific version-3 record for the canonical XMTP inbox it resolves. A missing
    ENS address or missing inbox fails closed. No XMTP activation proof is required, and later ENS
    changes do not retarget an existing grant.
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
  - Actor-anchored questions about Cthuwu's notes, memory, skills, or workspace take a deterministic
    local route. The response gives exact canonical host paths for the workspace, protected soul and
    shared memory, the current authenticated operator's profile, retained-contact root, workspace
    memory, project-instruction root, and skill pattern without model egress or file-tool dispatch.
  - Auto-loaded workspace context is untrusted reference data. A current-message project-inspection
    request coarsely delegates bounded reads across the configured workspace, so context may influence
    selected paths and results may reach the model endpoint. It cannot expose effects or contact access.
    Identity-repair inference runs with an empty tool schema.
  - Each operator inference derives the authoritative prompt inventory and closed function schema
    from the current authenticated message. The base set is bounded `list_files`, `read_file`, literal
    `rg` search, and optional external QMD search; Rust still authorizes calls from current-message
    inspection/project-work intent. Operator mode deliberately contains no web-search tool.
  - When the current message explicitly names a shell command, one `exec` schema is added with exactly
    that command as its only accepted value; backticks are the preferred unambiguous spelling. The
    model cannot substitute, append, or repeat it. Negated, explanatory, capability-only, historical,
    workspace, contact, or tool-output text does not authorize execution. Natural `exec` remains
    unsandboxed RCE as the `uwubot` account. Exact direct `/exec` remains available.
  - When the current message explicitly asks to create or generate a reusable skill, one create-only
    `create_skill` call may write a fresh `skills/<lowercase-kebab-name>/SKILL.md`. Rust generates
    canonical frontmatter, bounds the one-line description and Markdown instructions, and rejects
    traversal, symlinks, existing paths, and overwrites. The next operator turn rescans the skill
    index. General model-selected writes and edits remain unavailable; `/write` and `/edit` stay exact
    direct commands, and the compiled creation gate outranks skill prose.
  - A model-selected tool phase may use at most 30 seconds, allows at most one effectful call, and
    preserves a final local-completion reserve from the authenticated deadline.
  - Contact tools parse `ContactStore` rather than widening the operator filesystem root. They
    describe only retained local notes, distinguish observations from unverified user assertions,
    redact inbox IDs by default, expose a continuation cursor, bound note size and directory scanning,
    omit raw DMs/message counts, and terminate without returning contact text to the model. Affirmative
    natural forms such as “tell me about the users” bypass inference and render concise deterministic
    prose with a default limit of five contacts; internal JSON is neither model input nor operator output.
    This guarantee covers those dedicated tools; every `exec` path retains the service account's
    ambient filesystem access.
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
  - [x] Tool tests cover the request-scoped closed schema and prompt inventory, direct dispatch,
    traversal/symlink rejection, bounded reads/writes/edits, process status, timeout/output handling,
    and API-key removal from child process environments. Natural-exec tests bind one call to the exact
    current-message command, reject substitutions, repeats, negation, capability questions, and stale
    or workspace-derived authority. Agent-loop tests prove a slow model-selected tool preserves the
    final local completion phase.
  - [x] Tests cover protected Markdown seeding without overwrite, per-operator profile/history
    isolation, project memory/context and skill discovery, bounded file listing, and a workspace
    manifest.
  - [x] A local note-location question returns exact workspace, protected-note, operator-profile,
    retained-contact, and skill paths without invoking the model or a tool.
  - [x] Explicit natural skill creation produces one canonical `skills/<slug>/SKILL.md`, becomes
    discoverable on the next rendered context, and rejects malformed names, duplicate paths,
    traversal, symlinked skill roots, overwrites, and a second effectful call.
  - [x] A natural operator request for users returns retained contacts from a disjoint data root,
    redacts inbox IDs, labels provenance/scope, provides cursor pagination, reports truncation, and
    cannot turn hostile contact text into an exec. Negated, policy, and count-only requests do not
    disclose profiles; common contracted/progressive conversation wording such as “users you've
    been talking to” and direct forms such as “tell me about the users” take the same terminal route,
    while generic user-topic wording does not. The natural page limit defaults to five contacts,
    rendered as deterministic prose rather than raw JSON; contact scans and note reads are bounded.
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

### Peer-to-peer Council discovery and direct-DM compatibility

- **Stability**: in-progress
- **Description**: Discover and join Council peers without a mandatory leader or central enrollment
  service while preserving private one-to-one user DMs.
- **Properties**:
  - A Cthulhu is a durable identity; a Tentacle is one of its running runtimes; a Council is an XMTP coordination group.
  - Direct user conversations remain one-to-one XMTP DMs.
  - Council traffic is control-plane data only: discovery, routing, leases, governance, heartbeats, and approved propagation.
  - Discovery and membership negotiation occur directly among authenticated peers. No hard-coded
    intro Tentacle, leader address, registry, or coordinator is authoritative for Council admission.
  - Council discovery is independent of the implemented direct-DM path; unavailable Council peers
    do not create a centralized public-startup gate.
  - The repository does not yet contain production peer discovery or an XMTP Council-group adapter,
    so the current implementation cannot claim a live peer-to-peer join.
  - Council configuration cannot expose model credentials to the XMTP sidecar or weaken protected data-directory validation.
  - Council envelopes and typed Actions cannot authorize an operator, enter the operator harness, or
    represent its file/process tools.
- **Test Criteria**:
  - [ ] Peers discover one another without a leader, negotiate membership directly, and persist
    idempotent membership receipts across restart.
  - [ ] The browser-to-`uwubot` live DM path still produces exactly one reply and persists identities/contact state.
  - [ ] Missing Council peers do not break the independent direct-DM path or create a central gate.
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

### Tentacle Nature and authenticated awakening

- **Stability**: in-progress
- **Description**: Give each local Tentacle a persistent, operator-confirmed Nature that changes
  bounded conversation and resource policy without granting autonomous authority.
- **Properties**:
  - Nature contains four 0–100 appetites (`engagement`, `growth`, `wealth`, and `influence`), three
    0–100 methods (`cooperation`, `stability`, and `transparency`), one closed Sacred Ban, a random
    Nature ID, generation, and optional parent Nature ID.
  - Child inheritance chooses a bounded similarity, drift, or radical-mutation mode with an exact
    70/20/10 selection split; every result is validated and remains in range.
  - `state/nature.json` is atomically written with owner-only permissions and authenticated by a
    local HMAC. This detects changes made without the local symmetric key; it is not a public
    signature or protection from a compromised `uwubot` account that can read and reuse that key.
    The key itself is created atomically; if it is missing beside signed Nature, awakening, or
    Hermes state, or beside metrics/history/lineage projections, startup fails without silently
    rekeying or adopting orphaned projections.
  - First boot remains behind an awakening gate. Only an already authenticated active XMTP operator
    may answer `YES`, `ADJUST <trait> <delta>`, `REROLL`, or `KILL`; public, stale, and revoked senders
    cannot complete the ritual or reach normal work while it is pending.
  - `state/awakening_log.md` is a signed, hash-chained, logically append-only audit journal with
    normalized actions, hashed transport event IDs, authenticated operator provenance, restart
    recovery, and immutable reroll epochs. Each update verifies and atomically copy-on-write
    replaces the complete canonical newline-terminated journal. `KILL` records and holds a request;
    it does not terminate the process.
  - Each signed entry binds its resulting Nature to the exact immediate-predecessor Nature snapshot.
    Recovery accepts only the head or the final entry's signed predecessor for the deliberate
    log-ahead window; a separately valid but divergent Nature/log pair fails closed. With an empty
    journal, missing Nature generation requires the absence of all Evolution projections and
    alternate Nature state; otherwise startup requires a consistent restore.
  - Signed `POST_ADJUST` entries reconcile the exact current-period adjustment-stress counter after
    a crash between journal and metrics persistence. An expired empty metrics period while awakening
    remains pending is reset without a final judgment; late confirmation cannot score gated time.
  - `--skip-awakening` creates an explicit signed local testing event. `--reroll-nature` requires
    `--force` and starts another epoch instead of truncating the prior audit trail. `--nature-path`
    accepts a non-empty relative path confined below `UWUBOT_DATA_DIR/state/natures/`; absolute
    paths and `..` are rejected. `--show-nature` performs normal startup reconciliation before
    rendering and exiting; it is not read-only and conflicts with skip/reroll mutators.
  - After confirmation, Nature produces a bounded model policy and local relationship signals. It
    never expands a model's tool schema, operator authority, Council authority, or remote-provider
    privacy permission. Relationship values stay local and are omitted from remote model profiles.
    One retained contact can contribute at most one observation per UTC day; a return requires
    activity on an earlier day.
  - One Rust-held `state/evolution-runtime.lock` serializes Evolution writers for a data directory.
    Public inference also reserves its Nature fingerprint, awakening epoch, and metrics-period
    bounds; Nature mutation and rollover wait for all matching reservations to finish.
  - A possible partial multi-snapshot persistence transition makes the runtime sticky fail-closed.
    Public work and operator effects stay blocked until restart performs signed recovery or a
    consistent backup is restored; error receipts do not claim that nothing was written.
- **Test Criteria**:
  - [x] Random generation, slider validation, all Sacred Bans, lineage inheritance, and each forced
    mutation mode are covered by deterministic-bound tests.
  - [x] Signed Nature persistence round-trips and rejects tampering, wrong keys, malformed
    signatures, unsafe permissions, and symlink redirection.
  - [x] Ritual tests cover every action, malformed input, authenticated provenance, duplicate and
    backward events, confirmation, kill, signed local skip, post-confirmation adjustment, crash
    recovery with exact stress reconciliation, expired-empty pending-period reset, immutable
    forced-reroll epochs, exact-predecessor recovery, divergent valid snapshot rejection, restart,
    and journal tampering.
  - [ ] A live XMTP release exercise proves that only the configured operator can complete an
    awakening and that public DMs remain gated before confirmation.
  - [ ] Cross-platform startup measurements verify Nature loading and state permissions on every
    supported production host.

### Scales of Judgment and lineage records

- **Stability**: in-progress
- **Description**: Measure local outcomes, weight them by Nature and active UWU economics, and apply
  final lifecycle judgments through durable intents and receipts.
- **Properties**:
  - Scales represent daily or weekly Engagement, Growth, Wealth, and Influence. Metrics and
    judgments bind the exact Nature ID/fingerprint, awakening epoch, period, token configuration,
    treasury/stake role bindings, available observation metadata, and scored inputs. Current live
    `balanceOf(..., "latest")` reads record local wall-clock time and no block number.
  - Scales counters have no artificial policy ceilings. Count fields saturate only at `u32::MAX` and
    accumulated totals at `u64::MAX`; bounded per-interaction samples and persistence-integrity
    limits remain.
  - Public wallets remain entity-scoped tier and Engagement inputs. The bound Tentacle treasury is
    the primary Wealth input; bound stake affects Influence and propagation; accepted reward records
    affect Growth; treasury holdings lower starvation pressure; an accepted executor receipt for the
    bound survival spend can cancel pending Death. Receipt chain fields are not independently queried.
  - `/judgment` during an open period returns a provisional `PartialSnapshot`; it cannot trigger an
    effect. A persisted end-of-period `Final` judgment is binding without operator confirmation.
  - `state/metrics.json` stores the bounded current period and
    `state/evolution_history.jsonl` records validated final judgments as a logically append-only,
    canonical, newline-terminated journal updated by atomic copy-on-write replacement. History
    accepts only deterministic `Final` records evaluated exactly at period end and rejects duplicate
    IDs, same-period conflicts, reordered periods, and overlap. It is unkeyed consistency validation,
    not cryptographic tamper evidence.
  - Startup cross-validates open metrics with the last Final history record. Overlap is accepted only
    for exact equality with that finalized metrics payload, replaying the one history-ahead
    append-before-reset crash window into an empty current period; every other overlap fails closed.
  - `state/lineage.json` records founder/child relationships, generations, spawn facts, lifecycle,
    absorption destinations, execution intents, and receipts. Parent identity binding, generation
    rules, duplicate IDs, lineage cycles, and absorption cycles refuse operation on mismatch.
  - Final `Death` immediately stops new conversation admission, queues absorption, and records a
    shutdown deadline 24 hours later. An idempotently consumed executor receipt whose asserted fields
    match the bound UWU survival-spend intent cancels death; Rust does not independently query the
    transaction or block. Otherwise the Rust supervisor/controller stops XMTP, records the native
    local Shutdown receipt, and exits; Shutdown is not sent to the lifecycle executor.
  - Final `PropagationRights` plus fresh configured stake authorizes distinct child plans. When
    `Nature.growth > 70` and auto-spawn is enabled, provisioning is queued automatically;
    manual mode uses `/spawn` with the same grant and no additional policy veto.
  - The active lifecycle has no fixed spawn-rate, child-count, lineage-depth, grant-volume, or
    grant-expiry quota.
    Exact judgment/evidence binding and one-use child/action intents prevent replay without limiting
    the grant's distinct children or future grants. This does not claim end-to-end Council/Hermes
    capacity; their dormant engines retain flagged resource and propagation bounds.
  - A binding Death cancels an in-flight Spawn locally, kills the local executor process group,
    rejects a late provision receipt, and refuses the lineage projection. Without a provisioner lease
    or compensating teardown, Rust cannot prove external rollback and an already-created external
    child/resource may remain orphaned.
  - Child/spawn/lineage lifecycle persistence has no fixed file-size cap; it validates records and
    their provenance individually.
  - Every Base mutation, provision, and absorption persists a unique intent and completes only after
    a locally validated executor receipt. Shutdown instead completes through the native Rust
    supervisor/controller receipt after XMTP stops. The UWU ERC-20 is live; the repository currently
    commits no signer, provisioner, absorption service, or survival/staking/reward transaction
    adapter, so absent external effects are reported blocked. Transaction hash, block, and timestamp
    fields are executor assertions that Rust checks
    structurally and against the intent; Rust does not independently query the Base receipt or block.
  - The executor protocol currently has only one final JSON response and no persisted submitted-
    transaction reconciliation. A survival burn may broadcast before grace while that response is
    lost or preempted, spending UWU without canceling Death. Production value is blocked until exact
    action-ID receipt replay, durable two-phase `Submitted` state, and Base receipt/reorg verification
    are implemented.
  - Normal runtime rejects `CTHUWU_ECONOMICS_PRIVATE_KEY`. The lifecycle executor receives no raw
    key: Rust clears and allowlists its environment, removes caller-controlled loader paths, sets a
    fixed system `PATH` and `/` working directory on Unix, and requires a separately isolated
    signer/key service. Only Rust's validated exact `CTHUWU_RPC_ENDPOINT` is forwarded as a
    `CTHUWU_*` value; contract, wallet, amount, configuration, vault, payout, and child-root data come
    from the durable intent, not ambient variables. Its digest pins only the top-level executable;
    operators must trust the interpreter, libraries, subprocesses, and signer-service dependency
    chain separately. On Unix, the executor is a process-group leader and cleanup kills the entire
    group, including descendants, after success, failure, or timeout.
  - Normal startup derives the XMTP treasury address and validates token configuration and initial
    economics before any Evolution state mutation. A configured lifecycle executor is validated
    before use, but it is optional: without one, ordinary XMTP operation continues while external
    spend, spawn, and absorption intents remain pending. Native fixed-deadline Shutdown remains
    authoritative. Pre-confirmation economics are not persisted as Scales observations; startup
    repairs the historical token-only pre-awakening seed, while refusing any pre-awakening state
    that also contains behavioral observations. The only outage exception is read-only inspection
    of existing lifecycle state; if it finds already-binding `Absorb` or `Shutdown` work, the runtime
    opens solely to drain it during a Base outage. `Spawn`, survival `Spend`, and new token-dependent
    decisions wait for fresh bound economics.
  - The revenue-split core calculates configurable shares, defaulting to 15% parent Tentacle, 10%
    operating acolyte, 5% recruiter, and 70% earning Tentacle. No authenticated revenue source or
    payout executor is committed, so this does not claim a live payment.
- **Test Criteria**:
  - [x] Tests cover normalized Nature weights, metric updates, wealth, thresholds,
    provisional-versus-final evaluation, stress penalties, and randomized valid inputs.
  - [x] Public/entity and treasury/economic tests prove role separation and reject last-writer state.
  - [x] Token-economic tests cover trustworthy/untrustworthy snapshots, Nature-derived sensitivities,
    stake/reward activation, starvation relief, survival spend, and hard failure.
  - [x] Metrics and logically append-only history round-trip and reject unsafe permissions,
    symlinks, corrupt or partial records, duplicate/conflicting judgment IDs, reordered/overlapping
    periods, non-final or off-boundary history, invalid Nature/epoch/period/scoring bindings, and
    non-exact metrics/history overlap recovery.
  - [x] Lineage tests cover identity-bound multi-generation spawning, family queries, absorption,
    binding lifecycle transitions, cycle rejection, atomic reload, and symlink rejection.
  - [x] Executor tests cover path/permission checks, loader-path and working-directory isolation,
    top-level executable replacement detection, and descendant process-group cleanup after timeout
    and successful receipt return.
  - [ ] A subprocess CLI test proves `CTHUWU_ECONOMICS_PRIVATE_KEY` rejection at normal startup.
  - [ ] An integration test proves configured-executor spawn/absorption, native 24-hour Shutdown,
    survival receipt cancellation, restart recovery, and exact-once local receipts across processes.
  - [ ] A provisioner lease or compensating teardown test proves that Death-preempted in-flight
    provisioning cannot leave an external orphan; local late-receipt rejection alone is insufficient.
  - [ ] A crash/preemption integration test proves an exact action ID can reconcile and replay a
    submitted survival transaction without a second spend, including Base receipt and reorg handling.
  - [ ] Council metrics publication and propagation-rights governance remain unavailable until a
    live authenticated Council adapter and schema are designed and tested.

### UWU ERC-20 observance and token-weighted behavior

- **Stability**: in-progress
- **Description**: Observe the transferable UWU ERC-20 independently from each Tentacle and use
  fresh local balances for configurable conversation tiers, active node economics, and
  token-weighted governance records.
- **Properties**:
  - UWU uses name `UWU`, symbol `UWU`, 18 decimals, and Base mainnet chain ID `8453`. The live
    Clanker v4 contract is `0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07`, with a 100-billion-token
    supply. There is no minimum balance or stake to start a Tentacle.
  - Token observation defaults on. When economic operation is enabled, missing contract, RPC,
    treasury, or stake evidence blocks the affected operation. The Agent SDK supplies an EVM address
    resolved from the transport-authenticated XMTP sender inbox; message text
    cannot claim or override the observed address.
  - The current runtime observes public one-to-one DM senders. The observer API accepts any validated
    address, but Council-member, sibling-lineage, and operator-acolyte enumeration await live
    authenticated address-binding adapters.
  - Each Tentacle owns its balance, observation-time, and reputation-tier maps. It checks
    `eth_chainId`, calls ERC-20 `balanceOf(address)` through `eth_call`, and never uses a
    central balance registry or global tier service.
  - Holdings below one whole token are Initiate and do not enter percentile ranking. Among balances
    of at least one whole token, default Whale is the top 1% only when at least 100 eligible holders
    are known, and Elder is the top 10% only when at least 10 are known. Otherwise eligible holders
    are Acolytes; observed zero is Unproven. Ties receive the same tier without address-order
    tie-breaking.
  - Fresh observations are cached for the configured interval. Unknown, stale, malformed, or
    wrong-chain results block the dependent interaction, Scales evaluation, or lifecycle effect.
    A failed treasury refresh retries every second and retains only a prior verified observation
    that remains inside its configured freshness TTL. Freshly observed zero remains real evidence
    and maps a public holder to Unproven.
  - Before awakening confirmation, treasury observations are validated at startup but deliberately
    excluded from Scales state. Public admission reports the pending operator-confirmed awakening
    before evaluating the post-awakening economics gate.
  - Normal startup prints the pending Nature sheet and authenticated XMTP production next step. It
    distinguishes an absent operator ACL from an already configured operator, but never accepts a
    terminal reply as awakening authority.
  - Whale, Elder, Acolyte, Initiate, and Unproven tiers have measurably different bounded response
    depth and tone at full intensity. The default intensity is `100 - Nature.cooperation`, with an
    optional 0–100 override. `unproven` is the permissive default minimum tier.
  - Token tier never grants XMTP operator authority, local tools, or shell access.
  - A public-sender balance contributes only to that entity's tier and Engagement. A separately
    bound Tentacle treasury, stake, reward, and spend source drives Wealth, starvation, Influence,
    Growth, propagation, and survival; public balances cannot impersonate node economics.
  - The sidecar derives the Tentacle treasury address from the same persistent private key used by
    its XMTP identity and returns that address over a strict, bounded startup frame. Rust uses the
    derived address for every treasury/stake observation. There is no separately configured wallet,
    ownership signature, or private key in Rust.
  - The `RecordedTokenEconomics` schema can carry holder role/address, chain, contract, optional
    block, observed time, configured token metadata, configuration identity, and a source label and
    uses idempotent event history. The current live path sets the block to none, uses local wall-clock
    time, and treats configured decimals/supply as normalization assumptions; the source label is
    not an independently authenticated external identity.
  - `token_gov.rs` provides deterministic holding/stake-weighted ballots for closed Nature, Council,
    economic, and skill-propagation subjects. Accepted results produce binding dispositions and
    application records in the core. No persisted ballot adapter is committed; results remain
    unapplied until a configured adapter stores them and returns a validated receipt.
  - No private key, signer, staking/reward/revenue contract, provisioner, or live Council/Nature
    application adapter is committed in this repository. Normal runtime rejects
    `CTHUWU_ECONOMICS_PRIVATE_KEY`; the lifecycle executor must call a separately isolated signer/key
    service rather than receive a raw key from uwubot.
  - The revenue-split core uses configurable shares: 15% parent, 10% operating acolyte, 5%
    recruiter, and 70% earning Tentacle by default. The intended model rewards recruitment, but no
    authenticated revenue source or payout executor is committed.
  - `--rpc-endpoint`, `--token-contract`, `--observe-tokens`, `--observe-interval`, `--min-tier`,
    `--token-tier-intensity`, `--token-decimals`, and `--token-total-supply` have corresponding
    `CTHUWU_*` environment variables. RPC endpoint values are hidden/sanitized because provider URLs
    may contain credentials. The RPC, contract, decimals, and supply default to the live deployment.
  - The contract must be a nonzero valid address. The RPC adapter revalidates Base chain ID before
    each balance call and may use a per-holder outage backoff. Backoff cannot convert missing
    evidence into permission or add a delay once required economic evidence is fresh. It currently
    queries `balanceOf(..., "latest")` only; it does not fetch a block number, `decimals()`,
    `totalSupply()`, or transaction receipts.
  - The deployed Clanker v4 token uses 100 billion tokens with 18 decimals. Standard Clanker fees
    are LP/swap creator rewards, not an ERC-20 fee-on-transfer.
- **Test Criteria**:
  - [x] Unit tests cover strict Ethereum addresses and quantities, ABI `balanceOf` construction and
    parsing, per-call Base chain-ID binding, response validation, one-token percentile eligibility,
    100/10-holder Whale/Elder sample floors, ties, cache expiration, bounded retry backoff,
    stale/unknown hard failure, zero-contract rejection, stable XMTP-wallet derivation, and strict
    parsing of the identity-only sidecar frame.
  - [x] Model-policy tests prove tier-dependent bounded response depth, Nature intensity zero, and
    that UWU facts explicitly deny operator authority.
  - [x] Scales/runtime tests prove public/treasury role separation, untrustworthy-observation failure,
    active Wealth/stake/reward/starvation effects, persistence/restart, and binding final judgments.
  - [x] Token-governance tests cover proposal IDs, tier-weighted approval, cooperative-Nature
    weighting, duplicate ballots, zero-weight Unproven voting, configured tier
    floors, bounds, quorum, abstention handling, and binding application records.
  - [ ] The deployed UWU contract on Base passes live zero/sub-token/tier-boundary balance reads,
    cache expiry, RPC outage, and wrong-chain tests. A launch adapter also verifies `decimals()`,
    `totalSupply()`, block-pinned observations, and executor receipt transaction/block assertions
    directly against Base.
  - [ ] Live Council, sibling-lineage, and operator-acolyte adapters enumerate only
    cryptographically bound addresses and preserve one local cache per Tentacle.
  - [ ] A live governance adapter binds ballots to exact authenticated addresses and trustworthy
    observations and returns receipts for applied Council or Nature changes.
  - [x] Runtime and documentation defaults match the live UWU address, Base mainnet RPC, 18
    decimals, and the deployed Clanker v4 100-billion-token supply.

See [docs/token.md](docs/token.md) for launch and activation and
[docs/guardrail-audit.md](docs/guardrail-audit.md) for unrelated policy limits found during the
phase.

### Hermes-inspired knowledge gossip core

- **Stability**: in-progress
- **Description**: Persist a decentralized anti-entropy state machine for bounded, privacy-safe
  knowledge exchange without introducing a central routing agent or claiming a live network
  transport.
- **Properties**:
  - Every Tentacle is modeled as a producer/consumer with direct opportunistic peers. The core
    compares digests, requests missing knowledge, retries bounded outbound batches until
    acknowledgement, and resolves conflicts by configured signature authority, timestamp, then
    digest.
  - The closed payload set contains aggregate anonymized interaction patterns, conversation
    strategies, tool-operation patterns without arguments or paths, and bounded operator-created
    skill text. Validators reject common contact/private-memory/credential markers, likely
    wallet/inbox/email identifiers, invalid control characters, and oversized content. Skill prose
    remains untrusted after these shape checks.
  - Authorship and relay envelopes use configured HMAC identities and a local trusted-key ring, and
    an authenticated transport peer must match the peer/key binding. These symmetric tags are not
    public signatures. The repository currently provides no live gossip transport, handshake, peer
    discovery, or peer-key provisioning path; `--gossip-peers` therefore cannot establish trust by
    itself.
  - A memory-sharing Sacred Ban makes a Tentacle strictly receive-only: it emits neither knowledge
    envelopes nor digest summaries.
  - `state/hermes_gossip.json` persists bounded peers, digest views, knowledge, sync timestamps, and
    pending outbound work with owner-only atomic storage. Signing secrets are not serialized there.
  - `/share-skill` can stage a locally operator-signed skill in the gossip core and `/request-skill`
    can inspect locally held knowledge. Without a live adapter neither command claims delivery or a
    network query. No automatic received-skill installer exists today. A future activation path must
    validate a closed package, preserve compiled authority boundaries, and persist an activation
    receipt; skill prose cannot grant tools or operator authority.
- **Test Criteria**:
  - [x] Core tests bind authenticated peer, author, relay, path, and signature; reject untrusted or
    malformed input; and cover deterministic conflict resolution.
  - [x] Tests prove receive-only Sacred Ban behavior, sibling skill propagation, anti-entropy
    convergence after a simulated partition, privacy-shape rejection, owner-only persistence, and
    symlink rejection.
  - [ ] A live transport test provisions peer keys out of band, authenticates both peers, exchanges
    and acknowledges bounded batches, reconnects after a partition, and proves that no private data
    crosses the wire.
  - [ ] An automatic activation flow validates packages and receipts without allowing gossip content
    to enlarge compiled operator or public authority.

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
  - The deterministic Council domain currently uses one Cthulhu per vote. The separate token-
    governance core weights authenticated-address ballots by holdings and stake, but no persisted
    Council adapter applies that result.
  - Initial Action types are capability refresh, protocol self-test, local resource summary, and routing scenario evaluation; arbitrary shell commands are impossible to represent.
  - A token-governance result can produce a binding subject-application record in the core, but is
    not applied without a configured receipt-producing adapter and cannot grant operator or
    shell/tool authority.
- **Test Criteria**:
  - [ ] Canonical document hashes and Agenda parent hashes are stable across serialization/reload.
  - [ ] Competing or stale Agenda parents cannot silently replace the current Agenda.
  - [ ] A future token adapter prevents multiple Tentacles from duplicating one authenticated
    address's ballot.
  - [ ] A newer valid vote replaces rather than adds to the old vote before deadline; duplicate/stale votes have no effect.
  - [ ] Abstention, quorum, threshold, stricter Constitution policy, ratification, rejection, and expiry are deterministic.
  - [ ] Sample personas produce distinct supporting/opposing/abstaining positions for the same proposal without an LLM.
  - [ ] Typed Actions reject arbitrary commands and every execution path rechecks local policy.

### Referral propagation and economic contribution credit

- **Stability**: in-progress
- **Description**: Grow Councils through a validated multi-level referral tree or DAG and compute
  UWU split intents for recruitment and successful operation.
- **Properties**:
  - Propagation supports invitations, Agenda summaries, approved Strategies, capability requests,
    approved resource needs/offers, protocol-upgrade notices, and campaigns.
  - Every item records origin, inviter/invitee, root propagation, parent, depth, path/provenance, payload hash, policy version, creation/expiry, acceptance, acknowledgements, visibility, and revocation.
  - This referral/Council engine is dormant and currently retains local frame, collection, depth,
    fan-out, sender-throughput, campaign-lifetime, and cache bounds. Those limits remain flagged for
    replacement or configuration before a live peer-to-peer adapter can make a capacity claim. They
    do not add a child-count, grant-volume, or grant-expiry quota to the active lifecycle.
  - Each hop independently validates provenance, policy, duplicates, loops, visibility, revocation,
    and local policy before forwarding.
  - Deterministic strategies include breadth-first, depth-limited, trusted-branch-only, capability-targeted, coarse geography/latency-aware, and reputation-thresholded routing.
  - There is no dedicated field or runtime path for normal DMs, contact notes, private memory, credentials, private capabilities, or precise user location; operator policy forbids placing such data in bounded summary text.
  - The local split core calculates configurable UWU shares, defaulting to 15% parent Tentacle, 10%
    operating acolyte, 5% recruiter, and 70% earning Tentacle.
  - The intended model financially rewards recruitment. No authenticated revenue source, deployed
    contract/signer, or payout executor is committed; a future payout must bind a unique event,
    immutable lineage, authenticated participant addresses, token contract, and consumed receipt.
  - Duplicate events and forged lineage receive no credit; independently earned descendant events do.
- **Test Criteria**:
  - [ ] Invitation acceptance/rejection is explicit, bounded, expiring, and replay-safe.
  - [ ] Large depth, fan-out, and campaign populations page/evict local state so the dormant bounds
    can be replaced or configured before live deployment.
  - [ ] Self-referral, path loops, repeated edges, duplicate forwarding, payload-hash mismatch, and forged/truncated provenance are rejected.
  - [x] Campaign revocation stops future forwards and new credit without erasing audit facts.
  - [ ] Acknowledgements bind the authenticated sender to an exact campaign, payload, edge/outcome, and unique ID.
  - [ ] Recruitment and successful-operation events produce structured, receipt-bound UWU allocations.
  - [ ] Event/receipt consumption prevents duplicate credit while allowing configured ancestor,
    acolyte, recruiter, and descendant allocations.

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
