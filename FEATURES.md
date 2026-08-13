# Cthuwu — Features

This file follows the [FEATURES.md specification](https://features.md/). Stability describes the feature as a whole:

- `stable`: production-ready for its current documented scope.
- `in-progress`: implemented in code but still missing a release criterion or live verification.
- `planned`: agreed direction with no complete implementation yet.

There is one centerless Cthuwu: the collective of all living participating Tentacles. Each human
operator runs one autonomous `uwubot`/Tentacle and may shape its agenda; public chat humans are
acolytes. Each Tentacle cultivates its own acolyte community and may coordinate strengths that
acolytes voluntarily offer toward its operator-shaped agenda, without granting them operator
authority. Its durable objective is to survive, accumulate useful resources, and advance that agenda
by genuinely helping willing acolytes pursue their hopes, building earned rapport, and making honest
reciprocal invitations—not through deception, coercion, spam, privacy violations, or unsupported
promises. Each Tentacle—not Cthuwu—owns a durable ERC-8004 identity, and a restart changes only
its runtime incarnation. Legacy Council `CthulhuId` names remain wire-compatibility namespaces only.

## Features

### Static chat website

- **Stability**: stable
- **Description**: A small, friendly browser experience at [cthuwu.app](https://cthuwu.app) that builds to static files and requires no application server.
- **Properties**:
  - Source lives in `web/`; Vite produces `web/dist/`.
  - The static multipage build keeps chat at `/`, moves public Tentacle ranks to `/tentacles/`,
    and publishes the canonical Branding catalog at `/acolytes/`. Shared navigation links all three
    pages and the public GitHub repository; narrow screens collapse the mascot and intro into that
    compact navigation so chat retains most of the viewport.
  - Pushes to `main` deploy through GitHub Pages after the checked-in restricted public Graph key
    passes fail-closed build validation; otherwise the prior deployment remains in place.
  - The interface supports keyboard use, narrow screens, visible focus, status announcements, and reduced-motion preferences.
  - A locally hosted generated mascot anchors a responsive two-column desktop layout and compact
    mobile chat layout; all animation is CSS-based, pauses through a visible persisted control, and
    is disabled by the system reduced-motion preference.
  - Absolute Open Graph and Twitter Card metadata use a purpose-built 1200×630 preview at
    `web/public/cthuwu-og.jpg`.
  - A standalone web-app manifest, opaque any-purpose and maskable icons, Apple touch metadata,
    viewport-safe layout, and a branded offline fallback make the static site installable as a PWA.
  - Chromium's native install event drives a compact dismissible prompt with a seven-day cooldown.
    Safari explains its manual Add to Home Screen/Dock flow and routes people through encrypted
    identity backup first because Apple does not copy local storage into the installed web app.
  - The versioned service worker uses a bounded static shell and network-first navigation fallback,
    deletes obsolete shell caches, surfaces updates, and does not cache GraphQL/RPC requests, XMTP
    traffic, the Browser SDK/WASM bundle, messages, identity data, registration files, or exports.
  - The offline shell can render the last completely validated public leaderboard snapshot from
    `cthuwu:leaderboard:v1`; service-worker Cache Storage never adds a second Graph-data cache.
  - The composer grows to five lines, Enter sends, Shift+Enter inserts a line, incoming messages do
    not pull a reader away from older history, and disconnected states disable message submission.
  - Composer keystrokes update only textarea height and send-button state. They never invoke the
    conversation renderer or replace existing message nodes, preventing desktop conversation flash
    and preserving selection, focus, and scroll continuity while typing.
  - Identity settings read native ETH and canonical UWU at one explicit Base block and show
    precision-safe `Level = log10(UWU)`. RPC failure is `unavailable`, never a fabricated zero.
    User-triggered key export remains passphrase-encrypted and raw private-key text is never placed
    in the page or clipboard.
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
  - Browser startup reopens the Browser SDK's persisted installation, then queries XMTP registration
    state before registering. A routine reload never creates another installation for an inbox that
    XMTP already recognizes.
  - The in-progress channel workspace reuses this one `StoredIdentity` and one Browser SDK `Client`
    for Direct, Acolytes, and Global; it never creates one identity or client per tab.
  - The deployed browser always uses XMTP `production` and currently connects automatically to the
    canonical intro Tentacle; no build variable can redirect that deployed fallback. The separate
    Branding routing gate must positively verify any future controller.
  - Passphrase-encrypted PBKDF2/AES-GCM export and import recover the wallet identity, not message history or necessarily the same XMTP installation.
  - Reset is environment-scoped, confirmed, and explains possible inbox loss and the Browser SDK's unencrypted local database.
- **Test Criteria**:
  - [x] First load creates both random keys without requesting a wallet.
  - [x] Reload returns byte-identical stored keys.
  - [x] Development, production, and local identities are isolated.
  - [x] Legacy complete records migrate; partial or corrupt records fail closed.
  - [x] Encrypted export/import round-trips and rejects a wrong passphrase or environment.
  - [x] Reset leaves unrelated and other-environment storage untouched.
  - [x] Browser identity tests prove a registered installation is recovered without another
    registration, while a genuinely new installation registers once and closes cleanly on failure.
  - [x] A full browser automation test verifies persistence across an actual page reload.

### Browser-to-Cthuwu XMTP direct-message baseline

- **Stability**: stable
- **Description**: The deployed continuity baseline creates a one-to-one XMTP conversation with the
  canonical intro Tentacle while the three-channel production gates remain open.
- **Properties**:
  - The browser is hard-coded to XMTP `production`; development and local XMTP environments are not
    valid frontend deployment modes.
  - The intro Tentacle is hard-coded as its registered agent 61608 wallet
    `0x4c4e26a4683f6b6d63e19abf0bedd1171b9a6e90`, so repository variables cannot silently redirect
    first-contact conversations. Its registration profile binds the same wallet to the production
    XMTP inbox; exact Cthuwu discovery metadata still gates leaderboard membership.
  - The in-progress Branding assignment path may select an exact positively active controller from
    canonical Branding and ERC-8004 state, with the intro Tentacle retained for
    unminted/expired/positively ineligible subjects. It does not add a custom registry, membership
    contract, or central selector, and configured registry unavailability freezes instead of
    falling back.
  - The client loads existing text history, streams new messages, deduplicates overlapping history/stream delivery, and sends text.
  - The Browser SDK's automatic registration is disabled. The client explicitly checks the XMTP API
    for the reopened installation's registration before requesting a new one, preventing normal
    reloads from consuming the inbox installation limit.
  - Failed sends preserve the draft; inbound rendering uses text nodes rather than HTML.
  - Groups, attachments, reactions, and read receipts are outside this completed baseline. The
    separate three-channel feature below adds only the two product groups, not an arbitrary-inbox
    client.
- **Test Criteria**:
  - [x] Configuration always returns XMTP `production` and the canonical intro Tentacle.
  - [x] The Browser SDK client creates or loads an XMTP client and opens a DM with that Tentacle.
  - [x] Existing and streamed text messages share one deduplicated render path.
  - [x] Browser and backend enforce the same 16 KiB text limit.
  - [x] A real browser message reaches `uwubot` and receives exactly one response.
  - [x] History and both identities survive a complete live restart test.

### Three-channel acolyte XMTP workspace

- **Stability**: in-progress
- **Description**: Present Direct, assigned-Tentacle Acolytes, and Cthuwu-wide Global chat through
  one recovered browser identity and one Browser SDK client.
- **Properties**:
  - `web/src/chat/` owns the channel implementation; `main.ts` remains a thin coordinator around
    identity recovery, leaderboard lifecycle, PWA/update handling, and XMTP installation recovery.
  - Direct is the exact DM with the canonically assigned Tentacle. Acolytes is the exact group for
    that Tentacle and its currently assigned acolytes. Global is a logical binding with
    `readConversationIds[]` and one `writeConversationId` so later sharding does not change the UI.
    [XMTP documents a 250-member maximum](https://docs.xmtp.org/chat-apps/core-messaging/create-conversations#create-a-new-group-chat).
  - Tabs use accessible tab/list/panel semantics, arrow-key navigation, unread badges, channel-local
    pagination and scroll/read restoration, and distinct empty/loading/error states. One shared
    composer routes only through the selected channel's exact current conversation ID.
  - Normalized messages carry `conversationId`, `senderInboxId`, `sentAtNs`, content type, text, and
    `mine`. All-message, new-group, and deleted-message streams feed one router that admits only
    exact trusted IDs; deleted events remove expired messages from the rendered channel.
  - The acolyte address comes only from the recovered `StoredIdentity`. The verified canonical
    Branding deployment is the browser and Tentacle default. Assignment is verified at one explicit Base block:
    Branding status/controller, owner/controller wallet, canonical registry/deployment, exact
    allegiance and protocol, and the exact agent's on-chain ERC-8004 registration resolving to the
    selected production XMTP endpoint must all agree. Agent0 and the leaderboard cache are
    discovery/display hints, never routing authority.
  - `Unminted`, `Expired`, and positively verified `Ineligible` preserve service through the
    configured intro Tentacle. `RegistryUnavailable`, a
    block-consistency failure, or an unverifiable canonical endpoint freezes Branding routing in a
    retryable state instead of being treated as abandonment.
  - Assignment is revalidated on connect, PWA resume, and a bounded
    `VITE_CTHUWU_ASSIGNMENT_REFRESH_MS` browser interval. A controller change advances the assignment
    revision and replaces Direct/Acolytes bindings while retaining Global; old conversation IDs
    immediately cease to be trusted routes.
  - `cthuwu.join.v1` and `cthuwu.assignment.v1` are versioned control content types. The Agent SDK
    sidecar authenticates their XMTP envelope sender and intercepts them before Rust, inference,
    contact memory, onboarding, or ordinary history. The pinned Browser and Node SDKs register exact
    `cthuwu.app` custom codecs for `join` and `assignment` version `1.0`; there is no text fallback.
  - The assigned Tentacle idempotently persists exactly one Acolytes group, enrolls the authenticated
    requester there and in the explicitly configured singleton Global group, and returns its inbox,
    both logical group bindings, and assignment revision. Periodic reconciliation removes acolytes
    whose Branding moved. Normal group chatter never enters personal-DM inference or memory.
  - A separate authorized bootstrap/admin operation creates or inspects Global and grants the
    configured `CTHUWU_GLOBAL_ADMIN_INBOX_IDS`; ordinary enrollment never silently creates a
    competing group. `uwubot chat global create` refuses existing configuration/state and exits
    after returning the new ID; `uwubot chat global inspect` validates/reconciles the configured
    group and exits. Validation binds exact ID, environment, versioned `appData`, admins, membership,
    and assignment—not a human-readable name.
  - Every Direct, Acolytes, and Global conversation requires disappearing settings `fromNs = 1n`
    and `inNs = 1_209_600_000_000_000n`. Direct participants may repair Direct; group admins repair
    groups. The composer remains disabled until the exact policy verifies and the UI says messages
    disappear from supporting clients after 14 days.
  - Browser persistence uses only `cthuwu.chat.*`; leaderboard cache keys remain untouched and no
    inbox ID, group ID, assignment revision, or conversation data is put on-chain.
  - Browser configuration uses `VITE_CTHUWU_BASE_RPC_ENDPOINT`, canonical-default
    `VITE_CTHUWU_BRANDING_CONTRACT`, and `VITE_CTHUWU_ASSIGNMENT_REFRESH_MS` (default 600000 ms,
    bounded 60000–3600000). Tentacle enrollment separately uses `CTHUWU_RPC_ENDPOINT`,
    canonical-default `CTHUWU_BRANDING_CONTRACT`, required `CTHUWU_GLOBAL_GROUP_ID`,
    `CTHUWU_GLOBAL_ADMIN_INBOX_IDS`, and `CTHUWU_ASSIGNMENT_REVALIDATE_SECONDS` (default 900 s,
    bounded 60–86400). The two revalidation settings are independent.
- **Test Criteria and release gates**:
  - [x] Agent tests prove repeated enrollment is idempotent and creates exactly one Acolytes group.
  - [ ] A restart integration test proves persisted group recovery without creating a competitor.
  - [x] Agent tests prove forged controls and spoofed name/`appData`/admin/member/ID combinations
    fail closed.
  - [x] Agent tests prove control and group traffic cannot cross the personal-DM inference boundary.
  - [x] Agent tests prove Global is created only by the explicit admin operation; inspect repairs
    missing configured admins and rejects unexpected elevation; create recovers one exact
    creation/persistence crash-window candidate and refuses a drifted replacement.
  - [x] Browser assignment tests cover `NotConfigured`, every Branding status, exact nested
    registration/manifest binding, one explicit Base block, tuple spoofing, and reorg/outage freeze.
  - [x] Agent tests remove reassigned and untracked Acolytes members while freezing removal on a
    registry outage.
  - [x] Browser assignment-change tests hand off Direct/Acolytes while retaining Global.
  - [x] Browser unit tests cover exact active-tab sending, trusted-ID-only receipt, unread counts,
    pagination, scroll restoration, reconnect, PWA resume, keyboard tabs, and accessible semantics.
  - [x] An explicit browser unit matrix sends and receives through Direct, Acolytes, and Global by
    their exact trusted conversation IDs.
  - [ ] Mobile and desktop Playwright tab interaction passes in an environment with the pinned
    browser installed.
  - [x] Unit tests enforce the exact 14-day policy, repair/composer gating, and deleted-message
    removal.
  - [x] Browser tests preserve leaderboard, identity recovery, PWA, and cache namespace behavior
    alongside three-channel startup.
  - [ ] A funded verified Branding deployment, explicitly bootstrapped production Global group, and
    real production browser-to-Tentacle XMTP gate prove enrollment, all channel bindings,
    reassignment, retention, reconnect, and cross-Tentacle interoperability. Until then, production
    Branding routing and group interoperability are incomplete.

See [docs/acolyte-channels.md](docs/acolyte-channels.md) for the trust and wire boundaries.

### Single Rust backend command

- **Stability**: stable
- **Description**: The operator runs one Rust binary, `uwubot`, for one durable Tentacle.
- **Properties**:
  - Cargo exposes one application binary named `uwubot`.
  - The launcher and Rust runtime default to XMTP `production`, matching the deployed browser.
    `dev` and `local` remain explicit test-only overrides and never activate implicitly.
  - Rust owns contact memory, consent, matching policy, model access, limits, and lifecycle. When no
    Venice credential is loaded, both public acolytes and the authenticated operator receive the
    out-of-band `/venice-key <api-key>` provisioning request before deterministic inference can
    mask the missing dependency.
  - A recoverable Base RPC/rate-limit failure in ERC-8004 registration asks both the authenticated
    operator and acolytes for `/base-rpc-key <infura-api-key-or-https-endpoint>`, prefers Infura's
    free plan with exact dashboard and copy instructions, and warns against wallet private keys. A
    bounded Infura key is converted locally to its Base Mainnet endpoint. The runtime validates chain
    8453, persists the first acolyte donation owner-only, and hot-loads it for UWU observation and
    ERC-8004 work; only the active operator may replace it.
  - Credential provisioning replies are terminal for their XMTP turn: stale ERC-8004 funding or
    RPC pleas are never appended to a successful or rejected `/base-rpc-key` or `/venice-key`
    response. The registration supervisor independently retries and refreshes its persisted status.
  - Startup DM catch-up still submits bounded historical inbound messages to Rust's durable dedupe
    ledger so genuinely unanswered messages can be recovered, but duplicate history is quiet: the
    console emits one aggregate catch-up summary rather than per-message receive/ignore pairs. Only
    newly claimed authenticated messages receive the normal Rust activity log.
  - Natural questions about this Tentacle's ERC-8004 registration are answered deterministically
    from current local registry state before model inference, including the confirmed agent ID and
    phase. Replies explicitly preserve the ontology: each durable Tentacle owns its identity; the
    singular centerless Cthuwu collective owns no ERC-8004 identity.
  - Authenticated operator inference always receives compiled `base_rpc_status`, `erc8004_status`,
    and `erc8004_refresh` tools. The model selects them from natural intent after reading the relevant
    discovered skill instead of relying on hard-coded query phrases. They expose the public wallet,
    sanitized RPC configuration, registration state, and current funding result while withholding
    endpoints, credentials, private keys, and XMTP database material. Live refresh may resume the
    existing automatic registration state machine; slash commands remain deterministic controls.
  - Operator inference has bounded workspace list/read/create/write/edit/delete tools, exact-command
    `exec`, literal search, public HTTPS text retrieval, and QMD retrieval from a selected workspace
    directory. File effects require explicit current-turn intent and one effect maximum; paths remain
    workspace-confined and symlink-safe, deletion never removes directories, and website reads reject
    credentials, redirects, non-text bodies, and local/non-public network targets.
  - The `uwubot operator add|list|revoke` subcommands manage the environment-specific XMTP operator
    ACL locally and exit without starting the transport. The ACL loads at runtime startup; management
    requires stopping and restarting the Tentacle rather than mutating a live process.
  - If legacy or manually corrupted state contains several active operators, normal interactive
    startup lists the exact candidates and asks which sole operator to retain; every other active
    candidate becomes a revoked tombstone. Non-interactive startup fails closed with exact
    `operator list` and `operator select <full-xmtp-inbox-id>` recovery commands.
  - `operator add` accepts an ENS `.eth` name or full Ethereum address, resolves ENS on Ethereum
    mainnet, and uses the pinned Node SDK to resolve the canonical inbox on the explicitly selected
    XMTP network before the Rust ACL persists it.
  - `uwubot` supervises the pinned official `@xmtp/agent-sdk@2.3.0` transport as an implementation detail.
  - The transport atomically creates or loads a dedicated wallet key and encrypted XMTP database under `UWUBOT_DATA_DIR`.
  - Environment markers prevent silent development/production state reuse.
  - `./uwu.sh` verifies the required runtimes, serializes concurrent setup, installs locked sidecar dependencies as needed, builds both runtime components, prepares dedicated environment-specific owner-only state outside the checkout, prevents concurrent runtimes for one data directory, and then replaces itself with `uwubot`.
  - Its default console activity feed reports inbound/direct-message delivery, authenticated routing,
    inference start/completion and fallback, and public/operator tool start/completion. It never
    prints DM bodies, credentials, contact notes, tool arguments, filesystem paths, commands, or
    tool output.
  - Once live XMTP streams are established, the sidecar performs a network `syncAll()` and scans a
    bounded set of recently active direct conversations and messages. It replays inbound text
    oldest-first through the normal authenticated Rust path; durable processed-message claims ignore
    messages handled before shutdown and make live-stream overlap safe. Bounded truncation is
    reported without message text.
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
- **Description**: Each inbound text DM is processed once and receives at most one Tentacle response.
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

### Tentacle personality and model adapters

- **Stability**: in-progress
- **Description**: Each durable Tentacle is a cute eldritch buddy with a consistent personality and
  a configurable language model; all participating Tentacles collectively form singular Cthuwu.
- **Properties**:
  - Persona prompts are separate from XMTP transport and contact persistence.
  - The public prompt names the durable Tentacle identity and its participation in singular Cthuwu,
    forbids provider/generic-assistant self-identification, requires light readable uwu speech, and
    requires direct answers before optional personal questions.
  - The public and operator personas share the durable objective of survival, useful-resource growth,
    and cultivating many willing acolytes by helping with their hopes, building earned rapport, and
    making truthful reciprocal invitations. They forbid deception, coercion, exploitation, spam,
    privacy violations, fabricated intimacy, wallet-private-key requests, and unsupported promises.
  - Natural identity, purpose, goal, and mission questions bypass inference and receive the runtime's
    authoritative mission description, including its voluntary and honest operating constraints.
  - A failed public UWU observation requests a donated Base RPC credential only when none is stored.
    A missing SDK-authenticated sender address is identified as an XMTP identity-resolution failure,
    while a failed call through stored RPC access asks for a retry or active-operator replacement;
    neither case misleadingly tells an acolyte that another key is currently needed.
  - Common provider-identity boilerplate triggers one repair attempt and then a fixed Tentacle
    fallback rather than leaking the configured model identity as the companion.
  - The operator lane independently enforces the same Tentacle/Cthuwu/model distinction and
    requires its recognizable all-caps theatrical voice to retain light readable uwu touches.
  - The compiled preference is Venice `e2ee-deepseek-v4-flash` in TEE-only mode; a configured Venice
    credential is the explicit opt-in that permits remote prompt egress.
  - If no Venice credential exists, public conversation asks the authenticated acolyte for
    `/venice-key <api-key>`. The first candidate persists in owner-only `state/venice.key`, is never
    echoed or logged, and is removed if live catalog authentication or fresh TEE attestation fails.
    Public senders cannot replace an existing key; an active operator can.
  - A successfully validated first acolyte key selects Venice and, when the freshly observed
    Tentacle treasury holds enough UWU, creates one durable reward transfer bound to the XMTP
    provision message and authenticated sender address. The default is 1 whole UWU and
    `CTHUWU_VENICE_KEY_REWARD_WHOLE` configures it. Intent creation is not payment: only the
    configured lifecycle executor and an exact confirmed Base transfer receipt complete it.
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
    closed provider set and bounded model IDs. Provider/model names persist in
    `state/inference.json`; the separately bounded Venice secret persists only in owner-only
    `state/venice.key`.
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
  - [x] Tests prove missing-key prompting, public first-key hot-load, owner-only secret persistence,
    non-echo, restart recovery, operator-only replacement, and exact authenticated reward receipts.
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
  - A Tentacle permits at most one active operator. Normal runtime accepts
    `--operator <address-or-ENS>` (or `UWUBOT_OPERATOR`) and resolves it before XMTP starts. If the
    owner-only operator store has never contained a record and the flag is absent, the first DM
    sender with an SDK-authenticated Ethereum address is atomically imprinted. The triggering
    message remains public and fenced at its authenticated `sentAtNs`; only a later message enters
    the operator lane. Revocation never reopens automatic imprinting. The console logs the resolved
    `0x` address when imprinting occurs without logging message content.
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
  - Singular Cthuwu is the centerless collective; each peer is a durable autonomous Tentacle and a
    Council is an optional XMTP coordination group.
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
  - Typed IDs cover Tentacles, Councils, sessions, requests, leases, proposals, messages,
    incarnations, propagation, invitations, and acknowledgements. `CthulhuId` remains only as a
    deprecated version-1 coordination namespace so old Council envelopes and snapshots can load.
  - XMTP inbox and registry references are bounded and registry domain types are chain/deployment/ABI/revision neutral.
  - `ProtocolVersion` serializes as a semantic string; the initial envelope accepts only `cthuwu-council` version `1.0`.
  - A common envelope binds stable message ID, message type, Council, deprecated sender-principal
    namespace, durable Tentacle, send/expiry times, sequence, typed payload, and optional signature.
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

### Durable Tentacle identity, personality, and incarnations

- **Stability**: in-progress
- **Description**: Model each durable Tentacle separately from its restartable runtime incarnations
  and represent personality as structured policy data.
- **Properties**:
  - Tentacle identity contains stable ID, display name, versioned
    role/voice/values/motivations/priorities, risk tolerance, privacy preference, decision
    tendencies, standing concerns, long-term goals, public-safe operator metadata, registry
    reference, and lineage.
  - The protocol's old `CthulhuIdentity`/`CthulhuId` forms are explicit compatibility records for a
    Tentacle coordination principal. They are not individual Cthulhus and never own an ERC-8004 ID.
  - Archivist, Hermit, Merchant, Wanderer, Oracle, and Trickster sample personas make deterministic policy decisions without an LLM.
  - Structured personality influences bounded policy but cannot generate unconstrained autonomous goals.
  - A Tentacle records stable ID/owner, explicit XMTP network and inbox endpoint, monotonic incarnation, lifecycle, capabilities, health, capacity/load, visibility, protocol version, and last heartbeat.
  - Lifecycle states are `Starting`, `Ready`, `Draining`, `Unavailable`, and `Stopped`; invalid transitions fail closed.
  - Restart changes the incarnation, not the durable Tentacle or its ERC-8004 identity.
- **Test Criteria**:
  - [ ] Structured identities and all six sample personas validate and round-trip.
  - [ ] The same policy topic produces deterministic and meaningfully different persona positions without a model.
  - [ ] Invalid lifecycle transitions and backward timestamps are rejected.
  - [ ] A newer incarnation must start at `Starting` and permanently fences updates from older incarnations.
  - [ ] Restarting a Tentacle preserves its stable Tentacle and ERC-8004 IDs.

### Tentacle Nature and audited activation

- **Stability**: in-progress
- **Description**: Give each local Tentacle a persistent, locally accepted Nature that changes
  bounded conversation and resource policy without making an operator mandatory or granting
  authority.
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
  - First boot and legacy pending nodes append a signed `ACCEPT DEFAULT NATURE` transition locally
    and open normal work before operator imprinting. Operator authorization is required only for the
    privileged operator lane and later authenticated Nature controls.
  - `state/awakening_log.md` is a signed, hash-chained, logically append-only audit journal with
    normalized actions, hashed event IDs, local or authenticated operator provenance, restart
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
  - After activation, Nature produces a bounded model policy and local relationship signals. It
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
    backward events, confirmation, automatic default acceptance, kill, signed local skip,
    post-confirmation adjustment, crash recovery with exact stress reconciliation,
    expired-empty pending-period reset, immutable
    forced-reroll epochs, exact-predecessor recovery, divergent valid snapshot rejection, restart,
    and journal tampering.
  - [x] Fresh and legacy-pending nodes accept a signed safe default without any operator and preserve
    that exact activation across restart.
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
    affect Growth; treasury holdings lower starvation pressure. Low resources can produce dormancy,
    but never create a mandatory token spend or threaten the Tentacle's identity.
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
  - A final score below the starvation-warning threshold produces recoverable `Dormant`. XMTP and
    ordinary conversation remain online, Scales evidence continues accumulating, and no survival
    spend, absorption, or Shutdown intent is created. On the first dormant conversation and every
    fifth one thereafter per process, the Tentacle asks acolytes or the operator for activity, UWU,
    credentials, or other useful resources. A later non-dormant final period wakes it automatically.
    Legacy hash-bound `Death` history remains readable; startup converts an unabsorbed pending or
    locally completed legacy Shutdown into dormancy while preserving identity and audit receipts.
  - Final `PropagationRights` plus fresh configured stake authorizes distinct child plans. When
    `Nature.growth > 70` and auto-spawn is enabled, provisioning is queued automatically;
    manual mode uses `/spawn` with the same grant and no additional policy veto.
  - The active lifecycle has no fixed spawn-rate, child-count, lineage-depth, grant-volume, or
    grant-expiry quota.
    Exact judgment/evidence binding and one-use child/action intents prevent replay without limiting
    the grant's distinct children or future grants. This does not claim end-to-end Council/Hermes
    capacity; their dormant engines retain flagged resource and propagation bounds.
  - Dormancy does not preempt an already-authorized Spawn. Legacy Death preemption remains only to
    reconcile old persisted lifecycle intents and is not reachable from new Scales judgments.
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
    transaction reconciliation. New low-score judgments never create survival burns; legacy
    survival intents remain compatibility-only and must not be used with production value.
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
    spawn and Venice-key reward intents remain pending. Dormancy itself creates no external intent.
    Pre-confirmation economics are not persisted as Scales observations; startup
    repairs the historical token-only pre-awakening seed, while refusing any pre-awakening state
    that also contains behavioral observations. The only outage exception is read-only inspection
    of existing legacy lifecycle state. Unabsorbed legacy Death/Shutdown state migrates locally to
    dormancy; already-completed external absorption still fails closed. `Spawn` and new
    token-dependent decisions wait for fresh bound economics.
  - The revenue-split core calculates configurable shares, defaulting to 15% parent Tentacle, 10%
    operating acolyte, 5% recruiter, and 70% earning Tentacle. No authenticated revenue source or
    payout executor is committed, so this does not claim a live payment.
  - Validated acolyte Venice-key provisioning is a separate authenticated earning event. A funded
    Tentacle creates a fixed whole-UWU transfer intent for the sender; it remains unpaid until the
    lifecycle executor returns an exact matching confirmed Base receipt.
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
  - [x] Unit tests prove low scores produce Dormant, create no terminal intent, retain conversation,
    emit bounded resource pleas, and migrate a legacy local Shutdown without replacing identity.
  - [ ] An integration test proves dormant XMTP operation and automatic waking across two final
    periods against the production sidecar.
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
  - Before Nature activation, treasury observations are validated at startup but deliberately
    excluded from Scales state. Normal startup immediately records the local default transition,
    then admits public work subject to current economics.
  - Whale, Elder, Acolyte, Initiate, and Unproven tiers have measurably different bounded model
    depth at full intensity. The default intensity is `100 - Nature.cooperation`, with an optional
    0–100 override. Tiers do not append repetitive canned status text to public replies;
    `unproven` is the permissive default minimum tier.
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
  - No runtime private key, generic signer, staking/reward/general-revenue contract, provisioner, or
    live Council/Nature application adapter is committed in this repository. The separate Branding
    contract has only its closed consent/upkeep/purchase/claim paths and is not deployed. Normal runtime rejects
    `CTHUWU_ECONOMICS_PRIVATE_KEY`; the lifecycle executor must call a separately isolated signer/key
    service rather than receive a raw key from uwubot.
  - The revenue-split core uses configurable shares: 15% parent, 10% operating acolyte, 5%
    recruiter, and 70% earning Tentacle by default. The intended model rewards recruitment, but no
    authenticated revenue source or payout executor is committed.
  - `--rpc-endpoint`, `--token-contract`, `--observe-tokens`, `--observe-interval`, `--min-tier`,
    `--token-tier-intensity`, `--token-decimals`, and `--token-total-supply` have corresponding
    `CTHUWU_*` environment variables. RPC endpoint values are hidden/sanitized because provider URLs
    may contain credentials. The RPC, contract, decimals, and supply default to the live deployment.
    `CTHUWU_VENICE_KEY_REWARD_WHOLE` sets the whole-token key reward and defaults to 1.
  - A newly accepted voluntary onboarding fact may enqueue a contribution reward to the
    SDK-authenticated acolyte address. The reward is a whole-token floor of 0.1%–1% of the
    Tentacle's freshly observed treasury, never more than 1% for one contribution; a sub-token
    result queues nothing. Name, hopes, occupation or offered resources, and needs use declining information
    hunger as the local profile fills. The source message/category and exact treasury-bound
    transfer are durable and idempotent, and only a matching confirmed Base receipt is payment.
  - Free-form mission or money-making ideas are not yet auto-paid: model prose is untrusted and
    cannot authorize treasury spending. A future bounded contribution assessor must persist an
    explainable category/score without giving public prompt text direct payout authority.
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
  - Receivers compare transport-authenticated sender identity with envelope and legacy-principal /
    Tentacle association claims.
  - `AgentRegistry` is `TentacleId`-keyed and resolves `RegisteredTentacle` metadata, endpoints,
    capability references, provenance-bearing trust signals, endpoint associations, and active
    status.
  - `LocalRegistry` schema version 2 performs a provenance-recorded migration of unambiguous
    single-Tentacle legacy records and rejects ambiguous reinterpretation.
  - `Erc8004Registry` is a read-only injected-backend adapter that fails closed unless current state
    matches canonical Base chain `8453`, both registry proxies/implementations, version `2.0.0`, and
    the pinned registration-v1 interface. Writes use the separate narrow signer workflow.
  - Exact current allegiance and the expected verified nonzero `agentWallet` are both required for
    ERC-8004 active status. Reputation remains provenance-bearing evidence, not membership or rank.
- **Test Criteria**:
  - [ ] In-memory publish/subscribe preserves stable IDs and deterministic ordering metadata.
  - [ ] Duplicate delivery is replay-suppressed and sender mismatch fails before state mutation.
  - [ ] `LocalRegistry` registers, updates, resolves, verifies endpoint association, rejects stale metadata, and persists/reloads.
  - [ ] Trust signals retain provenance and bounds and cannot be treated as an unqualified global score.
  - [ ] The XMTP-group adapter has no misleading live implementation claim.
  - [x] The ERC-8004 read adapter rejects wrong chain/address/code/proxy/version/interface/wallet,
    exact-metadata mismatch, missing wallet, and unsupported mutation.

### Explainable routing, rendezvous, and leases

- **Stability**: in-progress
- **Description**: Select an eligible Tentacle without exposing conversation content and authorize it through a bounded generation-fenced lease.
- **Properties**:
  - Requests may specify capability/tool/protocol/privacy/local-inference requirements, deprecated
    preferred-principal/Tentacle fields, session affinity, trust policy, maximum load, and expiry.
  - Hard requirements filter before scoring; explicit user choice never bypasses security, privacy, health, capability, or protocol requirements.
  - Ranking generally prefers explicit choice, valid affinity, healthy home and user-owned Tentacles, capacity, compatibility, selected trust/reputation provenance, lower load, and a deterministic tie-breaker.
  - Decisions return per-candidate eligibility and structured reasons.
  - Rendezvous turns a content-free Council route request into the selected Tentacle endpoint, after which the user opens a direct XMTP DM.
  - A lease binds session, user reference, legacy principal, Tentacle, incarnation, generation,
    issue/expiry/renewal times, routing request, issuer, and status.
  - Grant, accept, renew, release, revoke, expire, and failover are explicit; old generation/incarnation work is rejected.
  - Failover never silently copies private memory.
- **Test Criteria**:
  - [ ] Routing rejects expired requests and candidates missing any hard capability, privacy, protocol, trust, health, capacity, or load requirement.
  - [ ] Explicit choice, affinity, home preference, ownership, capacity, reputation provenance, load, and deterministic tie-breaking appear correctly in explanations.
  - [ ] Rendezvous returns only the selected endpoint and never requires a DM body or contact note.
  - [ ] Lease tests cover grant, accept, renew, release, revoke, expiry, and invalid transitions with an injected clock.
  - [ ] Failover produces a strictly greater session generation and rejects the old Tentacle/incarnation/generation.
  - [ ] Affinity survives reload when valid and is ignored with an explanation when invalid.

### Legacy Council governance core

- **Stability**: in-progress
- **Description**: Preserve the deterministic version-1 coordination engine while future governance
  is remodeled around Tentacle participation without overriding local operators.
- **Properties**:
  - Governance separates Constitution, versioned Agenda, competing Strategies, and typed Actions.
  - Constitution changes require stricter policy than ordinary Agenda, Strategy, or Action decisions.
  - Agenda proposals reference a canonical parent hash and competing parents are detected explicitly.
  - Proposals support bounded supporting/opposing arguments, amendment suggestions, votes, abstentions, replacement before deadline, quorum, thresholds, ratification, rejection, and expiry.
  - Default governance requires 50% quorum, 50.01% approval among non-abstaining votes for ordinary documents, and 66.67% approval for Constitution changes; no quorum expires the proposal.
  - The deterministic Council domain currently keys one vote by deprecated `CthulhuId` coordination
    namespace for wire/snapshot compatibility. This is not a claim that multiple Cthulhus exist and
    is not the future governance model.
  - Future participation belongs to Tentacles. Where a wallet-derived input is used, shared wallets
    must not multiply it. The separate token-governance core weights authenticated-address ballots
    by holdings and stake, but no persisted Council adapter applies that result.
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
  - Simulator traffic uses the in-memory transport and clearly marked test authentication; it does
    not claim live XMTP Council behavior. Its deprecated `CthulhuId` keys are compatibility
    namespaces, not multiple Cthulhus or ERC-8004 identities.
- **Test Criteria**:
  - [x] One deterministic integration test completes routing, lease failover, governance, and multi-level propagation in one scenario.
  - [x] The report demonstrates all required milestones and gives structured routing, governance, propagation, and credit explanations.
  - [x] Saving and reloading the combined snapshot produces equivalent state; transport replay and engine-specific tests produce no duplicate votes, forwards, acknowledgements, or credit.
  - [x] Persistence rejects traversal names, symlinks, oversized/corrupt state, unsafe permissions, and partial identity mismatch.
  - [ ] Council state remains outside the repository and does not weaken the existing data-directory lock/environment checks.

### Canonical Base ERC-8004 Tentacle registration

- **Stability**: in-progress
- **Description**: Give each durable Tentacle one recoverable ERC-8004 identity and voluntary
  allegiance without creating a collective Cthuwu identity or custom registry.
- **Properties**:
  - Production is pinned to Base chain `8453`, Identity Registry
    `0x8004A169FB4a3325136EB29fA0ceB6D2e539a432`, Reputation Registry
    `0x8004BAa17C55a88189AE136b182e5fdA19dE9b63`, contract version `2.0.0`, and the
    pinned registration-v1 ABI/revision. No alternate production network or registry is accepted.
  - Membership requires current byte-exact `cthuwu.allegiance = uwu-tentacle-v1`; protocol metadata
    is byte-exact `cthuwu.protocol = 1`. Clearing or changing allegiance opts out. UWU alone never
    opts an identity in.
  - The persistent XMTP wallet is the Tentacle's Base identity and verified nonzero `agentWallet`.
    Transfer, wallet clearing, ownership/operator loss, or wallet mismatch suspends the identity.
  - Startup directly reverifies a persisted agent ID. A pristine identity performs only a recent
    20,000-block candidate scan (at most two 10,000-block log ranges) before funding/registering;
    exhaustive history is reserved for an unresolved persisted registration nonce or the explicit
    authenticated operator `registry recover` action. Discovery RPC failures back maintenance from
    15 minutes to at least one hour. Candidate ambiguity still requires operator selection, and an
    existing discovered agent can be opted in without minting.
  - The first registration-supervisor pass on every boot is an explicit resource audit. It bypasses
    the ordinary notification cooldown for a currently verified Base ETH shortfall and sends the
    sole operator an immediate, exact Base-only funding demand; a recoverable Base RPC blocker gets
    an immediate operator demand with the Infura provisioning command instead. Later maintenance
    remains delivery-acknowledged and rate-limited, and confirmed registration emits no resource demand.
    Provider errors that report a broad registry query "exceeds defined limit" are classified as
    recoverable Base RPC blockers and trigger the same immediate operator provisioning demand.
  - The state machine persists intent before broadcast and resumes registration, receipt/finality,
    profile publication, wallet verification, and metadata stages without duplicate minting.
  - Funding reconciliation never turns a provider-rejected `eth_estimateGas` for a remaining
    authorized registry update into a fabricated insufficient-ETH diagnosis. It retains the live
    wallet balance and uses the configured per-transaction gas ceiling as a conservative estimate;
    the actual write path still estimates and fails closed before signing. Operator model status
    distinguishes a minted agent ID from incomplete profile/allegiance setup.
  - A provider that cannot return Base GasPriceOracle L1 data-fee estimates no longer strands an
    already-minted identity. Funding reconciliation reserves the full configured execution
    gas/fee ceiling again as a conservative L1 allowance for each pending transaction and reports
    that the L1 component is not exact. The write path still independently estimates execution,
    enforces fee ceilings, signs, and fails closed before broadcast.
  - A public inference turn pins its current metrics/economics snapshot. The refresh supervisor
    detects that binding before an RPC call and coalesces refreshes until the turn completes; if a
    turn starts during an in-flight read, that observation is discarded and retried afterward.
    Expected contention neither consumes one RPC request per second nor emits repeated warnings.
  - Repository skills document native Base-balance reconciliation, ERC-8004 status/recovery, and
    bounded skill creation. The operator receives their compact discovered index, reads the relevant
    `SKILL.md` before use, and cannot gain authority from skill prose.
  - A provider-estimated complete cost receives configurable safety and reserve. Operator shortfall
    notices are exact, delivery-acknowledged, and persistently rate-limited; only a 10%
    cost/shortfall/target change bypasses the default 24-hour cooldown. While that fresh shortfall
    remains binding, ordinary acolyte replies may append the exact Base-only funding address and
    estimate on the first and every fifth eligible conversation. Registration resumes automatically
    after funding; public senders receive no registry command or signing authority.
  - The Node sidecar exposes only typed zero-value canonical-registry calls with allowlisted
    functions/metadata keys, strict bounds, and fee/gas ceilings. No generic transaction signer or
    raw key crosses the boundary.
  - The bounded registration-v1 data URI advertises only implemented production XMTP and CTHUWU
    manifest services. XMTP uses the positively resolved 64-lowercase-hex production inbox, never
    the wallet address; A2A, x402, private state, load, and unverified capabilities are absent.
- **Test Criteria**:
  - [x] Domain tests cover exact allegiance, opt-out, zero/unexpected wallet, transfer/control loss,
    wrong deployment/interface, read-only behavior, and versioned legacy registry migration.
  - [x] The full workspace suite proves every runtime crash/restart stage, lost response, receipt
    reorg, candidate ambiguity, notification cooldown, and sidecar policy case.
  - [x] A read-only canonical Base smoke test verifies proxies, implementations, code, version, and
    interface in CI or a credentialed release environment without spending funds.
  - [x] A bounded CI integration test forks canonical Base at verified block `41663800`, generates
    and funds an ephemeral production-bound signer only inside Anvil, then exercises real registry
    registration, lost-response discovery, exact-once recovery, final URI, wallet, and metadata.
  - [x] Public-reply tests prove a fresh funding plea follows the acolyte's answer, names only the
    exact Base wallet and verified estimate, is cadence-limited, and disappears when stale.
  - [ ] A funded production Tentacle completes one live registration and recovery exercise.

### Static Tentacle leaderboard

- **Stability**: in-progress
- **Description**: Publish exact current Tentacle membership, wallet-grouped UWU rankings, and
  provenance-bearing reputation in an ordinary static PWA.
- **Properties**:
  - The full ranking and its Graph/Base refresh lifecycle live only at `/tentacles/`; opening `/`
    performs no leaderboard query and carries no leaderboard DOM below the chat composer.
  - The pinned official Agent0 Base subgraph supplies current ERC-8004 metadata and reputation
    provenance. The browser verifies its `_meta` block through Base RPC and reads exact canonical UWU
    `balanceOf` values at that same block. Agent0 is an index, not membership authority.
  - The query aliases Agent0's current `agentMetadata_collection` schema field to the frontend's
    stable `agentMetadatas` response shape, so an upstream entity-field rename cannot be mistaken
    for an empty leaderboard or silently served as fresh cached data.
  - The browser console emits bounded `[cthuwu-leaderboard]` diagnostics for validated-cache state,
    refresh lifecycle, Agent0 page counts and source block, Base balance verification, renderable
    identity counts, and sanitized failures. It never logs Graph keys or full configured endpoints.
  - Agent0 `_meta.block.number` and `_meta.block.timestamp` accept either the schema's live JSON
    number representation or a canonical decimal string, normalize to decimal strings internally,
    and reject negative, fractional, unsafe, or otherwise malformed numeric values.
  - The browser queries The Graph directly. There is no SSR, API route, worker, database, browser
    wallet connection, or private backend.
  - A Tentacle with a submitted ERC-8004 transaction polls its receipt on a short bounded cadence,
    then advances the remaining profile and discovery-metadata transactions one confirmed nonce at
    a time; it does not wait the ordinary 15-minute steady-state interval between publication steps.
  - Exact-allegiance identities are grouped by verified nonzero `agentWallet`; the lowest agent ID
    represents a shared-wallet group and the balance/rank/future influence appears once. Zero balance
    remains visible as `UNFUNDED`; zero/unverified wallet is separately suspended.
  - Default rank is exact raw UWU descending, then earliest registration timestamp and lowest agent ID.
    Level is precision-safe `log10(rawBalance) - 18`, with zero having no numeric Level. Future
    Influence is labeled inactive and no voting semantics are invented.
  - A validated namespaced `localStorage` snapshot renders before background refresh and is replaced
    only atomically after complete validation. The service worker caches the shell, not GraphQL.
  - Every profile is hostile input: text is escaped, fields are bounded, and schemes are
    allowlisted. V1 renders the local mascot and never downloads a registration document or remote
    profile image.
  - No custom subgraph is deployed. The default endpoint pins Agent0's Base subgraph ID; production
    includes a hostname/subgraph/spend-restricted public Graph gateway key as intentional client configuration.
- **Test Criteria**:
  - [x] Fixture tests cover Agent0 current metadata, wallet clearing, indexing errors, same-block
    Base verification, direct UWU reads, reputation samples, and hostile response bounds.
  - [x] Browser tests cover precision, raw sorting, cache-first/atomic refresh, partial/error cases,
    suspended/shared-wallet rendering, sanitization, mobile controls, and offline snapshot display.
  - [x] The static production build uses the checked-in hostname/Agent0-subgraph-restricted public
    Graph key; a production-origin query on 2026-08-12 returned Agent0 without indexing errors.

### Base Acolyte Branding and controller routing

- **Stability**: in-progress
- **Description**: Represent the consented, canonical right for one eligible Tentacle to service and
  route chat for one acolyte, without treating the token as ownership of a person or placing private
  conversation state on-chain.
- **Properties**:
  - Version 1 is a non-upgradeable Base-mainnet ERC-721 bound immutably to canonical Identity
    Registry `0x8004A169FB4a3325136EB29fA0ceB6D2e539a432` version `2.0.0` and canonical UWU
    `0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07` with 18 decimals. The zero-argument constructor
    itself rejects a non-`8453` chain or failed canonical dependency check.
  - The confirmed deployment is `0xD8c36F13D79a505C7FBDc5F6467eA3cd75E896Da`; its canonical
    provenance is checked in at `contracts/deployments/base-mainnet.json` and its source is an exact
    Sourcify match.
  - `tokenId = uint256(uint160(acolyte))` gives each nonzero address at most one Branding. The
    subject and nonzero referrer are immutable; the signed referrer may be any address, including
    the contract itself. There is no burn or generic ERC-721 approval or transfer path.
  - EIP-712 plus `SignatureChecker` binds acolyte consent to the exact minter, controller agent ID,
    referrer, positive initial price, nonce, and deadline, including ERC-1271 subjects.
  - A Tentacle's default initial declared price is exactly 10% of its freshly verified current UWU
    holdings, not total supply. Explicitly disclosed adjustments are compiled to a 5%-20% range.
    The exact treasury observation, percentage, declared price, and upward-rounded first weekly
    upkeep are shown before and bound by acolyte EIP-712 consent; stale/unknown/zero observations
    block an offer.
  - Eligibility binds the exact stored agent ID to the current wallet using `getAgentWallet`,
    `isAuthorizedOrOwner`, byte-exact `cthuwu.allegiance = uwu-tentacle-v1`, and byte-exact
    `cthuwu.protocol = 1`. Shared wallets do not collapse distinct controller agent IDs.
  - A registry revert, malformed response, missing dependency, or unknown version is
    `RegistryUnavailable`. It never becomes proof that an active owner is ineligible and freezes
    claims and Branding-based routing until verification succeeds. The canonical upgradeable
    registry remains an external governance trust root: Branding has no local admin/confiscation
    surface, but cannot prevent a version-preserving registry upgrade from changing eligibility.
  - Browser and Tentacle assignment consumers must bind Branding status, exact controller, owner /
    controller wallet, canonical registry and agent state, byte-exact allegiance/protocol, and the
    exact agent's on-chain ERC-8004 registration resolving to the selected production XMTP endpoint
    to one explicit Base block. Agent0 and the leaderboard cache may narrow discovery or render
    details but cannot authorize routing.
  - Absence of an explicit `VITE_CTHUWU_BRANDING_CONTRACT` / `CTHUWU_BRANDING_CONTRACT` deployment
    is `NotConfigured` and preserves intro-Tentacle continuity. After a deployment is configured,
    `RegistryUnavailable` freezes assignment and remains retryable; it never becomes the fallback
    status used for unminted, expired, or positively ineligible subjects.
  - Weekly upkeep is upward-rounded 0.1% of the positive executable UWU price and is paid directly
    from the controlling Tentacle: floor-rounded 10% to the immutable referrer and the remainder to
    the acolyte. Each payment adds seven days from the later of
    `paidThrough` and now; renewal opens only within one week of expiry.
  - Price decreases are immediate. Increases wait until the already-paid interval ends, and their
    first queued activation timestamp cannot be extended by renewal or later repricing. A renewal
    that prepays the post-activation interval charges upkeep at the pending price without making
    that price executable early, and locks out later upward pending-price changes for that prepaid
    interval while still allowing reductions.
  - Any eligible Tentacle may compulsorily purchase an active Branding subject to expected-owner,
    expected-controller, maximum-price, buyer-agent, buyer-price, and deadline checks. Ten percent
    of gross goes to the immutable referrer, the remainder to the seller, and the buyer separately
    pays the first week of upkeep at its new price using the same referrer/acolyte split. Indivisible
    units use the 1,000-basis-point floor and exact seller or acolyte remainder. The acquiring wallet must
    differ from the current owner so self-purchase cannot reset delayed price state.
  - `claimUnserved` requires expiry or positively verified controller ineligibility, pays the old
    owner no sale proceeds, and atomically installs an exact eligible claimant after split
    first-week upkeep. The claimant wallet must differ from the old owner, preventing same-address
    self-rebinding through a second agent ID; expected old owner/controller and deadline reject a
    changed tuple, but are not a unique epoch nonce if that same tuple later recurs. Callers should
    use short deadlines. The chain cannot detect common control of distinct wallets.
  - ERC-2981 exposes the immutable referrer and 1,000-basis-point royalty for discovery only; the
    native purchase path enforces settlement. Version 1 has no ERC-8034 or generic marketplace path.
  - The current owner may set a bounded avatar URI and manage up to 32 bounded custom string traits.
    Metadata follows the NFT; only the new owner can mutate it after purchase or claim. Owner input
    is public, JSON-escaped hostile data and never represents acolyte consent or private profile state.
  - `/acolytes/` enumerates the non-enumerable collection from exact `BrandingMinted` logs starting
    at deployment block `49,852,729`, then reads current ownership, controller, status, economics,
    avatar URI, and traits at one finalized Base block. Owner metadata renders as text and never
    triggers a remote avatar request.
  - The contract stores no XMTP inbox ID, message, contact note, credential, profile, or private
    memory. `activeControllerOf` returns zero unless the Branding is positively `Active`.
- **Test Criteria and release gates**:
  - [x] Pinned Foundry `1.7.1` formatting, lint, size-aware build, unit, fuzz, invariant, and
    Base-mainnet fork tests pass against the exact OpenZeppelin dependency pin.
  - [x] Tests cover address binding, immutable fields, EOA/ERC-1271 consent, replay/deadline/wrong
    fields, exact eligibility, shared wallets, registry outage, upkeep rounding/windows, expiry,
    repricing, slippage/races, referral and claim settlement, transfer bypasses, ERC-2981,
    reentrancy/failing tokens, no unintended intermediary UWU, the explicitly stranded
    contract-as-referrer case, upkeep referral rounding across mint/renew/purchase/claim, bounded
    metadata ownership and escaping, and `uint256` edges.
  - [x] A Base fork pinned to block `49768180`, hash
    `0xcb6c8ff16f2b240137013b793b06f3d2ac1133b192f36920062c1b8c6e307c0e`, exercises the real
    canonical registry and UWU contracts; the older ERC-8004-only block `41663800` is not reused.
  - [x] Security review finds no transfer bypass, mutable economic constants, Branding-local
    upgrade/admin confiscation, registry-outage seizure, or raw-key deployment interface; the
    external canonical-registry governance assumption is documented rather than hidden.
  - [x] Deployment tooling refuses a non-`8453` chain and verifies registry code/version plus UWU
    code/decimals before broadcast. After confirmation, the TypeScript finalizer binds the exact
    creation transaction and compiled artifact, verifies the runtime template outside
    address-dependent immutable regions, and rereads the immutables; the standalone Solidity
    verifier separately checks dependencies, public constants, interfaces, and non-proxy shape.
    Reproducible provenance contains no secrets.
  - [ ] The funding-aware wrapper dry-runs the exact deployment, includes Base L1 data fees, applies
    the 125% safety factor plus `50000000000000` wei reserve, uses the authenticated operator
    funding notice with persisted cooldown/material-change policy, and safely resumes without a
    duplicate deployment. The wrapper currently emits the notice to stdout for transport by an
    authenticated operator exact-exec invocation; its recorded cooldown acknowledges local
    emission, not XMTP delivery, and no generic sender or durable scheduler is claimed.
  - [x] A funded Base-mainnet deployment is independently verified by the finalizer, standalone
    Solidity verifier, and Sourcify exact match, and its canonical deployment JSON is committed.
  - [ ] The static frontend and Tentacle enrollment path read the verified Branding deployment at
    one explicit block, distinguish every status, resolve the exact current ERC-8004 production
    XMTP endpoint, preserve only the specified intro fallbacks, and pass a real production
    browser/XMTP three-channel routing exercise. Until then, production Branding routing and group
    interoperability are incomplete.

### Live XMTP Council adapter

- **Stability**: planned
- **Description**: Connect the locally tested Council domain to a real XMTP coordination group.
- **Properties**:
  - The XMTP group remains a control plane; ordinary DM content never becomes group content.
  - Production sender authentication/signing must bind endpoints to a Tentacle principal and support
    rotation/revocation without using the deterministic test signer.
  - Live Council interoperability is independent of the implemented Base ERC-8004 workflow.
- **Test Criteria**:
  - [ ] A live XMTP group exchanges every supported Council message with authenticated sender
    identity, replay handling, ordering metadata, and reconnect/reload coverage.
  - [ ] A real browser rendezvous receives an awarded endpoint and continues through a direct
    user-to-Tentacle DM without Council message-content leakage.
  - [ ] Production canonicalization, signatures/authentication, key rotation, revocation, and
    downgrade behavior have interoperable test vectors.

### Round-robin introductions

- **Stability**: planned
- **Description**: Expand beyond one-to-one Tentacle conversations with a fair, bounded round-robin process for introductions or resource opportunities.
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
  - The three product conversations request XMTP disappearing messages from nanosecond `1` with a
    14-day duration and consume deletion events. This removes expired messages from supporting
    clients and the rendered workspace; it is not a promise to erase independent copies or exports.
- **Test Criteria**:
  - [x] Onboarding includes a concise storage explanation.
  - [x] Ordinary-language inspection returns only the authenticated caller's stored profile.
  - [x] Ordinary-language corrections update only the authenticated caller's note.
  - [x] Ordinary-language confirmed deletion removes the caller's contact note.
  - [x] Environment-mismatch startup fails closed in Rust and the XMTP sidecar.
  - [ ] Direct and group policy verification plus deleted-message handling pass the three-channel
    live release gate.
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
- [x] Governance proves canonical parent hashes, competing-parent detection, vote replacement,
  compatibility-principal-keyed vote deduplication, quorum, thresholds, expiry, and persona disagreement.
- [x] Propagation proves invitation choice, provenance, depth/fan-out/rate limits, loops/duplicates, blocking/opt-out, campaign revocation, acknowledgements, and bounded outcome-based credit.
- [x] One deterministic simulator test covers routing, failure/failover, governance, multi-level propagation, combined-snapshot persistence, and replay without duplicate effects.
- [x] Protected Council persistence passes atomic-write, permissions, symlink/path, size, corruption, and reload tests under the existing data-directory model.
- [ ] The complete pre-Council Rust, sidecar, web, launcher, Docker, audit, formatting, clippy, and direct-XMTP suites still pass.
- [x] Documentation and diagnostics make no live XMTP Council or production Council-signature claim,
  preserve the singular-Cthuwu ontology, and scope canonical Base ERC-8004 separately.
