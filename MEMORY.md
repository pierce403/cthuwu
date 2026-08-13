# Cthuwu memory

Last reviewed: 2026-08-12

## Product

- Each Tentacle presents as a cute little eldritch-horror companion; Cthuwu itself is the
  centerless collective, not an individual bot persona.
- Acolytes chat with an independently operated Tentacle over XMTP.
- The public-facing client is a static web deployment.
- The operator runs the companion locally as a Rust CLI/daemon.
- There is exactly one Cthuwu: the centerless collective formed by all living participating
  Tentacles. It has no owner, center, separate wallet, or ERC-8004 identity.
- Each independently operated `uwubot` is one durable autonomous Tentacle with its own identity,
  wallet, personality, economics, reputation, lineage, and ERC-8004 agent ID. A restart creates a
  new incarnation, not a new Tentacle. One Tentacle's death is not Cthuwu's death.
- Many human operators each run a Tentacle and may shape that Tentacle's agenda. Public humans who
  chat with a Tentacle are acolytes; they are not operators or Tentacles merely by chatting or
  holding UWU.
- Each Tentacle cultivates its own acolyte community and may coordinate strengths that acolytes
  voluntarily offer toward its operator-shaped agenda. Participation, messages, and token holdings
  never grant operator authority.
- A Council is an optional XMTP coordination group between Tentacles, not a collection of distinct
  Cthulhus.
- Council discovery and membership must be peer-to-peer, without a mandatory leader or central
  enrollment service. No live peer-discovery/XMTP Council-group adapter is committed. Direct user
  DMs remain the implemented path and local simulation is not a live join.
- Early-development workflow is direct commits to `main`.

## Architecture

- Browser: `@xmtp/browser-sdk`, static Vite output, dedicated browser identity by default.
- Runtime: Rust `uwubot` supervisor with companion, model, and state boundaries; private official Agent SDK sidecar for XMTP transport.
- Persistence: encrypted XMTP database plus an application state store for processed-message idempotency.
- Model access: adapter boundary; local models should be first-class.
- Inference defaults to Venice `e2ee-deepseek-v4-flash` in TEE-only mode when a Venice key is
  configured. The runtime validates live capabilities and a baseline nonce attestation before first
  prompt egress, caches catalog capabilities for four hours and attestation for five minutes, and
  explicitly disables Venice-native search and supplemental system prompting. It then falls back to
  proxy-bypassing loopback Ollama and deterministic behavior on any failure. It does not implement or
  claim full E2EE or independent quote verification.
- With no Venice credential, public acolytes and the authenticated operator are asked for
  `/venice-key <api-key>` before inference fallback can mask the missing dependency. The first
  candidate persists owner-only in `state/venice.key`, is never echoed, and must authenticate to the
  live catalog and pass fresh TEE attestation; invalid candidates are removed and only operators may
  replace a loaded key. A funded treasury queues the configured whole-UWU reward to the
  SDK-authenticated sender, but only an exact confirmed lifecycle-executor transfer receipt is
  payment.
- Newly accepted voluntary onboarding facts can enqueue a separate treasury contribution reward:
  name 1%, hopes 0.8%, offered resources 0.6%, and needs 0.4%, floored to whole UWU from a fresh
  treasury observation and capped at 1% per event. The exact authenticated event and confirmed
  transfer receipt are required; free-form model-rated ideas do not authorize spending.
- The bridge's default end-to-end envelope is 300 seconds. Rust preserves one second for the XMTP
  response, then applies role-specific inference budgets only after authenticating the sender: public
  work is capped at 120 seconds and public remote inference at 30 seconds, while operator work can
  use at most 299 seconds. An operator remote route reserves two capped local model phases (up to the
  75-second safety cap, or a smaller configured Ollama timeout, each), one model-selected tool phase
  (up to 30 seconds), and one deterministic second. The default 181-second reserve makes Venice's
  effective maximum about 118 seconds despite its
  configurable 120-second cap. Model-selected tools preserve a final local completion; budget skips
  do not trigger provider cooldown, and failure cooldown is isolated by lane.
- Public Brave search is exposed at runtime only when the current message explicitly asks for
  current or web-verifiable information; ordinary chatter, stable facts, and repair completions get
  no search schema.
- Authenticated `/provider` and `/model` commands persist only the node-wide provider/model names in
  protected `state/inference.json`; `/venice-key` stores the secret separately in owner-only
  `state/venice.key`. Route changes clear in-process operator dialogue history.
- Text-only one-to-one DMs are the completed continuity slice. The in-progress browser workspace
  replaces the single-session UI with fixed Direct, Acolytes, and Global channels while keeping
  arbitrary inboxes/groups out of scope.
- `./uwu.sh` now keeps the default console useful without becoming a transcript: Node emits received
  and delivered XMTP-message events, while Rust emits authenticated routing, inference “thinking”
  provider phases/fallback, and tool lifecycle events. These records omit message bodies, identity
  IDs, credentials, contact notes, tool arguments, paths, commands, and output.
- After Agent SDK live streams are established, the sidecar runs `syncAll()` and scans up to 256
  most-recently-active DMs with up to 512 recent messages each. Inbound text is replayed oldest-first
  through the ordinary Rust bridge. Durable processed-message claims suppress prior replies and
  stream/catch-up overlap; bounded truncation is logged without message bodies.
- Browser identities are generated and connected automatically, then persisted in local storage.
- The deployed browser always uses XMTP `production`; it has no environment override. Development
  and local XMTP modes remain explicit backend/test concerns only. Both `uwu.sh` and `uwubot`
  default to `production`; they never select `dev` implicitly.
- Browser startup must call `Client.create(..., { disableAutoRegister: true })`, reopen its
  persisted Browser SDK installation, then check `client.isRegistered()` before calling
  `client.register()`. Auto-registering on every launch creates needless installations and reaches
  XMTP's 10-installation inbox cap; an already registered installation must be recovered without a
  second register request.
- The container also defaults XMTP to `production`. A transient node-economics refresh failure
  retries every second and retains the last verified treasury observation only until its freshness
  TTL expires; unknown or stale economics still fail closed. Base's built-in public RPC fallback is
  rate limited, so production operators should configure a dedicated `CTHUWU_RPC_ENDPOINT`.
- Recoverable ERC-8004 Base RPC and rate-limit failures are resource pleas, not diagnostics alone:
  operator status and ordinary operator/acolyte replies request `/base-rpc-key <api-key>`, point to
  Infura, Alchemy, or QuickNode, reject wallet private keys, and require an operator restart after
  applying the provider's Base mainnet HTTPS endpoint as `CTHUWU_RPC_ENDPOINT`.
- A fresh or legacy-pending node records a signed local `ACCEPT DEFAULT NATURE` transition before
  normal admission and persists current Scales economics under that activation. No operator ACL is
  needed for ordinary conversation; operators remain optional and privileged.
- The browser's canonical intro Tentacle is registered Base ERC-8004 agent 61608 and is hard-coded
  to its XMTP wallet `0x4c4e26a4683f6b6d63e19abf0bedd1171b9a6e90`. `NotConfigured`, unminted, expired, and positively
  ineligible Branding states preserve that continuity path. After a Branding deployment is
  explicitly configured, registry/endpoint unavailability freezes assignment and exposes retry;
  it never becomes another intro fallback.
- The production-origin-restricted Agent0 Graph key resolves the Base subgraph without indexing
  errors. Agent 61608 appears by ID with its wallet and registration profile, but the leaderboard
  intentionally excludes it until its exact allegiance/protocol discovery metadata is indexed.
- Agent0's live Base schema names the allegiance collection `agentMetadata_collection`, not the
  former `agentMetadatas`. The browser aliases the live field to its stable response key; testing
  only fixture response bodies does not detect an upstream GraphQL query-field rename.
- Public leaderboard console diagnostics use the `[cthuwu-leaderboard]` prefix and report only
  bounded cache/refresh/page/block/count state plus sanitized failure reasons. They must not expose
  the compiled Graph key or full configured Graph/RPC endpoints.
- Agent0 currently serializes `_meta.block.number` and `_meta.block.timestamp` as JSON numbers even
  though older fixtures used decimal strings. Unit and Playwright fixtures must preserve that live
  representation; parsing normalizes safe nonnegative integers to canonical decimal strings.
- ERC-8004 submitted transactions use a 15-second automatic maintenance cadence, while ordinary
  active checks retain the configured 15-minute default and recoverable RPC failures retain their
  one-hour backoff. This completes sequential profile/metadata publication without nonce races or
  repeated operator refresh messages.
- The browser recovers one `StoredIdentity` and creates one Browser SDK `Client` for all three tabs.
  Direct binds the exact assigned-Tentacle DM; Acolytes binds the exact assigned group; Global is a
  logical `readConversationIds[]` plus `writeConversationId`. This shape anticipates XMTP's current
  [official 250-member group cap](https://docs.xmtp.org/chat-apps/core-messaging/create-conversations#create-a-new-group-chat)
  without changing the tab model when sharding becomes necessary.
- Browser assignment derives the acolyte address only from `StoredIdentity`. One explicit Base block
  must bind Branding status/controller, owner/controller wallet, canonical registry, exact
  allegiance/protocol, and the exact agent's on-chain ERC-8004 registration resolving to the
  selected production XMTP endpoint. Agent0 and the leaderboard cache can narrow discovery or
  supply display names, never authorize routing.
- Chat composer input must not call the full workspace render: that path replaces all message nodes
  and visibly flashes the conversation on desktop. Keystrokes only resize the textarea and update
  composer controls; workspace snapshots remain the sole conversation-render trigger.
- Assignment is revalidated on connect, PWA resume, and a bounded interval. Controller change hands
  off Direct/Acolytes and retains Global; old conversation IDs immediately stop being trusted routes.
- Versioned `cthuwu.join.v1` / `cthuwu.assignment.v1` control uses registered
  `cthuwu.app/join:1.0` / `cthuwu.app/assignment:1.0` custom content types with no text fallback. It
  is authenticated from the XMTP envelope and intercepted by the sidecar before Rust, inference,
  contact memory, or ordinary history. Normal group chatter has no personal-DM inference or memory
  path in version 1.
- A Tentacle persists exactly one idempotent Acolytes group and enrolls only its currently assigned
  acolytes in owner-only `state/xmtp-chat-control.json`. Global is one explicitly configured,
  separately bootstrapped group whose exact ID, environment, versioned `appData`, admins, and
  membership must validate; a name is never authority.
  `uwubot chat global create` and `uwubot chat global inspect` are one-shot admin operations that
  exit; create can recover one exact self-created crash-window candidate but refuses a drifted
  replacement, and ordinary service startup never creates Global.
- Direct, Acolytes, and Global require XMTP disappearing settings `fromNs = 1n` and
  `inNs = 1_209_600_000_000_000n`. The relevant channel composer remains disabled until policy
  verification succeeds, and deleted-message events remove expired messages from the rendered UI.
  This is supporting-client retention, not erasure of independent copies.
- Three-channel browser persistence is confined to `cthuwu.chat.*`, separate from
  `cthuwu:leaderboard:v1`, and never places inbox IDs, group IDs, revisions, or conversations on-chain.
- The web presentation is a responsive "pocket séance" layout. Its generated Cthuwu cutout lives at
  `web/public/cthuwu-mascot.webp`; motion is CSS-only, system-reduced-motion aware, and can be paused
  with the environment-independent `cthuwu.ui.motion.v1` browser preference.
- The social preview is the generated 1200×630 `web/public/cthuwu-og.jpg`; the page references it
  through absolute Open Graph and Twitter Card URLs.
- The web client is an installable PWA with dedicated any-purpose, maskable, Apple touch, and
  favicon assets under `web/public/icons/`. Its install nudge appears only when a real Chromium
  install event is available, or as backup-first manual guidance in Safari, and a dismissal cools
  it down for seven days. A permanent Install App action can reopen it, and standalone/fullscreen
  mode suppresses it.
- The static web build has direct entries for chat at `/`, Tentacle ranks at `/tentacles/`, and the
  Branding catalog at `/acolytes/`. Root does not initialize the leaderboard. On mobile the root
  mascot/intro collapses into shared page/GitHub navigation so chat occupies most of the viewport.
- Identity settings read ETH and UWU at one pinned Base block and derive the existing precision-safe
  `log10(UWU)` Level. Key export is a passphrase-encrypted identity file; never render or copy raw
  wallet or database key material.
- Apple's installed Home Screen/Dock web app does not inherit local storage from Safari. Because
  the browser wallet lives there, the UI must continue to recommend encrypted identity backup
  before Apple installation; never bridge that private key through cookies, query strings, or URLs.
  See [WebKit Features in Safari 17.2 — Web Apps](https://webkit.org/blog/14787/webkit-features-in-safari-17-2/#web-apps).
- `web/public/sw.js` versions a bounded application shell, cleans obsolete shell caches, uses
  network-first navigation with an offline fallback, and exposes controlled update reload. It must
  not cache GraphQL/RPC, registration files, XMTP/WASM, DMs, identities, exports, or arbitrary
  future same-origin API responses. The offline page reads only the last validated leaderboard
  snapshot from `localStorage`.
- The sole backend command is `uwubot`.
- Contact notes default to `contacts/<inbox-id>.md` and are ignored by git because they contain personal statements.
- The public bot identifies as one durable Tentacle of singular, centerless Cthuwu rather than the
  configured model or a central Cthuwu agent, uses light readable uwu speech, and has an
  identity-policy repair/fallback for common provider boilerplate.
- A new public sender's first message is answered. The first optional onboarding prompt is appended
  only if the model reply contains no question; otherwise it enters the regular cadence. Onboarding
  collects name, hopes, possible contributions, needs, and explicit sharing consent as optional
  user-asserted information, with one casual prompt at a time and several ordinary conversation
  turns between prompts. Ambiguous consent remains unresolved and is re-cadenced, not immediately
  repeated.
- Public privacy and profile controls are expressed in ordinary language; do not advertise legacy
  slash commands to normal users.
- Optional Brave Search is the public model's only tool. It requires an effective tool-calling
  provider plus `UWUBOT_WEB_SEARCH=brave`; public chat never gets local file or process tools.
- UWU observance defaults on but remains inactive until a contract is configured. The sidecar
  resolves an optional EVM address from the SDK-authenticated XMTP sender inbox, and each Tentacle
  uses its own read-only Base-8453 `balanceOf` cache; message text cannot claim the observed wallet.
- Each Tentacle permits one active operator. `--operator <address-or-ENS>` / `UWUBOT_OPERATOR`
  resolves and pins that inbox before transport startup. With an empty operator history and no
  flag, the first DM sender with an SDK-authenticated Ethereum address is atomically imprinted; its
  triggering message stays public at the authorization fence, and only later messages are
  privileged. Revocation never reopens automatic imprinting. The console records the authenticated
  `0x` imprint address without message text. Local add/list/revoke remains available while stopped.
- Active operator DMs enter a distinct all-caps ominous/submissive truthful harness with light
  readable uwu voice. Each turn's prompt inventory is derived from its actual closed schema: bounded
  `list_files`, `read_file`, `search_files`, and optional `qmd_search` form the base; an explicit
  current-message command may activate one exact-command-bound natural `exec`, and an explicit new
  skill request may activate one create-only `create_skill`. General write/edit remains direct-only.
  Operator model-identity boilerplate receives repair/fallback enforcement. The hidden stdin harness
  remains public-only, and Council Actions cannot reach these tools.
- Operator cognition follows a bounded Hermes-like Markdown split: protected instance
  `state/agent/SOUL.md` and shared `state/agent/memories/MEMORY.md` are seeded once; per-inbox
  operator profiles are seeded beneath `state/agent/operators/`. They load beside globally bounded
  workspace project context, workspace memory, a top-level manifest, and a compact progressive skill
  index. Dialogue history is bounded in process and isolated by operator inbox. Project-inspection
  requests coarsely delegate bounded workspace reads, so auto-loaded context may influence chosen
  paths; it cannot expose effects/contact tools, and the immutable Rust kernel remains authoritative.
  Actor-anchored note/workspace-location questions return the exact canonical workspace, protected
  note, current profile, contact root, workspace memory, project-instruction root, and skill paths
  locally without invoking a model or file tool.
- One explicit request can create one fresh `skills/<lowercase-kebab-name>/SKILL.md`. Rust generates
  canonical frontmatter, bounds content, rejects traversal/symlinks/existing paths/overwrites, and
  exposes the skill through the rescanned index on the next operator turn. The
  `skills/skill-creator/SKILL.md` procedure guides authoring, but the compiled create-only gate is the
  authority.
- Retained users are queried through parsed `ContactStore` tools, never by pointing the operator
  root at the sensitive data directory. Reports are terminal, read-only, scoped to current notes,
  redact inbox IDs by default, use cursor pagination, bound scans and note size, and never feed values
  returned by those tools back into the model. Natural contact intent must be recognized before
  model inference with a closed contact subject and actor-anchored conversational forms (including
  direct “tell me about the users,” contractions, progressive tense, and smart apostrophes). Natural
  profile reports use a default limit of five contacts and render deterministic prose; internal JSON
  is never dumped to the operator. Generic user-topic, qualified, or negated wording must not
  disclose profiles. Every `exec` route separately retains all filesystem permissions of the service
  account.
- Operator ACL config version 3 is environment-bound and owner-only at `state/operators.json`.
  Local authorization persists a grant-time `sentAtNs` fence, and each request's role is pinned before a
  no-queue authority lane. Message IDs are durably claimed before admission; overload, including the
  bounded empty-text `reject_inbound` bridge handshake, returns a busy reply only for the first claim
  and ignores duplicates without dispatch. Node checks oversized input before forwarding content;
  `reject_oversized` carries metadata plus an empty `text`, and Rust classifies then claims before a
  role-specific first reply or duplicate ignore, without contact/model/tool dispatch. Retrying
  requires a new XMTP message, shortened when oversized. Authorization is inbox-wide: every XMTP
  installation attached to an active inbox has authority.
- Operator `/exec` and exact-command-bound natural `exec` are deliberate remote code execution as the
  `uwubot` OS account, not a sandbox. Natural authority comes only from the current authenticated
  message, permits one call with no command substitution, and is clearest when the command is in
  backticks; workspace/history/tool/contact text cannot authorize it.
- The canonical operator workspace and private data directory must not overlap in either direction;
  startup rejects overlap before exposing file tools.
  Production nodes need a dedicated unprivileged account/container, a narrow operator root, minimal
  credentials, and immediate local plus XMTP installation revocation after compromise.
- Matching is bilateral opt-in, explainable, and suggestion-only; chosen names and matching terms may be shown, but inbox IDs are not disclosed.
- Browser identity exports are passphrase-encrypted wallet backups. The Browser SDK database is unencrypted and is not included in that export.
- Backend secrets are atomically persisted at `state/xmtp-identity.json`; XMTP databases are environment-specific below `state/xmtp/`.
- `@xmtp/agent-sdk@2.3.0` is the supported first transport. Direct libxmtp remains a later option because its Rust crates are unpublished internal APIs.

## Evolution architecture decisions

- Tentacle Nature is a local runtime policy belonging to the durable Tentacle. The old Council
  `CthulhuIdentity` personality record is a version-1 coordination compatibility form for that
  Tentacle principal, not an individual Cthulhu.
  It has seven 0–100 sliders, one closed Sacred Ban, a random Nature ID, generation, and optional
  parent Nature ID. Inheritance selects similarity/drift/radical mutation with a 70/20/10 split.
- `state/nature.json` and the logically append-only awakening journal use a local owner-only HMAC
  key. Journal updates verify and atomically copy-on-write replace canonical newline-terminated
  history. These are symmetric local integrity tags, not public signatures, peer identities, or
  protection from a compromised service account that can read the key and re-sign state. The key is
  created atomically; missing-key startup beside signed state or metrics/history/lineage projections
  fails without implicit rekeying or adoption of orphaned projections.
- Fresh and legacy-pending awakening epochs append a signed local `ACCEPT DEFAULT NATURE` action and
  open normal conversation without an operator ACL. Operator authorization remains optional for
  privileged tools and later authenticated Nature controls. Local `--skip-awakening` is a visibly
  distinct signed testing override; forced rerolls append new epochs rather than rewriting history
  and are accepted by the same safe default policy. `KILL` enters the binding death
  path: admission closes, absorption is queued, and the 24-hour shutdown deadline is persisted.
  Signed `POST_ADJUST` entries reconcile exact current-period stress after a crash. An expired empty
  pending-awakening metrics period resets without producing a judgment before local activation.
  Each signed entry carries its exact immediate-predecessor Nature snapshot; recovery accepts only
  the head or final signed predecessor, and rejects a divergent Nature/log pair even if both validate
  independently. With no journal entries, a missing Nature may be generated only when no Evolution
  projections or alternate Nature exist.
- Confirmed Nature supplies only bounded response/resource policy and local contact relationship
  signals. Relationship values are excluded from remote model profiles; one contact contributes at
  most once per UTC day and counts as returning only after prior-day activity. Nature cannot add
  tools, expand role authority, select an unconfigured remote provider, or weaken privacy consent.
- Rust holds one `state/evolution-runtime.lock` for a data directory. Public inference reservations
  bind the signed Nature fingerprint, awakening epoch, and metrics period; mutation and rollover
  defer until matching reservations finish.
- Scales open-period evaluations are provisional and cannot trigger effects. A persisted
  closed-period judgment is final and applies automatically. Metrics and judgments
  bind Nature ID/fingerprint, awakening epoch, period, scored inputs, token configuration, treasury/
  stake roles, and available observation metadata. Current live `balanceOf(..., "latest")` reads
  record local wall-clock observation time and no block number. Public wallets remain entity-scoped
  tier/Engagement inputs. Scales counters have no artificial policy ceilings: count fields saturate
  at `u32::MAX` and accumulated totals at `u64::MAX`; per-sample and persistence bounds remain.
  Bound treasury, stake, and reward evidence drives Wealth, starvation, Influence, Growth, and
  propagation.
- Final low scores now produce recoverable `Dormant`: XMTP and conversation stay online, Scales
  evidence continues, no survival/absorption/Shutdown intent is created, and bounded pleas ask
  acolytes and the operator for resources. A later non-dormant final period wakes automatically.
  Legacy hash-bound `Death` history stays readable; startup migrates unabsorbed pending or locally
  completed Shutdown state without replacing the XMTP identity or deleting audit receipts.
- Final `PropagationRights` plus fresh required stake authorizes distinct child plans without a
  volume or expiry quota. When `Nature.growth > 70` and auto-spawn is enabled, provisioning
  is queued automatically; acolytes may configure manual `/spawn` using the same reusable grant.
  Each exact child/action is consumed once. The active child/spawn/lineage lifecycle has no fixed
  spawn-rate, child-count, or lineage-depth quota; dormant Council/Hermes bounds are a separate
  flagged limitation. Lineage binds every intent/receipt to its exact final judgment, parent Nature,
  treasury/stake evidence, and execution ID.
- Dormancy does not preempt an already-authorized Spawn. Legacy Death preemption is compatibility-
  only and is not reachable from new Scales judgments.
- Base mutations and child provisioning use durable unique intents and locally validated idempotent
  executor receipts. No child provisioner is committed, so absent effects are reported blocked.
- Normal runtime rejects `CTHUWU_ECONOMICS_PRIVATE_KEY`; no raw signing key is accepted or forwarded.
  The lifecycle executor must use a separately isolated signer/key service. Rust clears and
  allowlists its environment, drops caller-controlled loader paths, and, on Unix, sets a fixed
  system `PATH` and `/` working directory. Only Rust's validated exact `CTHUWU_RPC_ENDPOINT` is
  forwarded as a `CTHUWU_*` variable; contract, wallet, amount, configuration, vault, payout, and
  child-root fields come from the durable intent rather than ambient variables. Rust hashes/rechecks
  only the top-level executable. Its interpreter, libraries, subprocesses, and signer service remain
  a separately trusted dependency chain. On Unix the executor is a process-group leader, and cleanup
  kills it and all descendants after success, failure, or timeout. The XMTP sidecar uses the same
  full-process-group cleanup on supervisor teardown, including when its direct Node parent has
  already exited.
- Normal startup derives the XMTP treasury address and validates token configuration and initial
  economics before mutating Evolution state. A configured lifecycle executor is validated before
  use, but it is optional: ordinary XMTP operation continues without one while external spawn and
  reward intents remain pending. Dormancy creates no executor work. The
  initial economics preflight does not enter Scales before Nature activation; startup repairs
  the historical token-only pre-activation seed but fails closed if behavioral observations are also
  present. Unabsorbed legacy Death/Shutdown state migrates locally during startup; completed external
  absorption remains terminal. `Spawn` and new token-dependent decisions wait for fresh bound
  economics. Child/spawn/lineage lifecycle state has no fixed
  file-size cap and validates records/provenance individually.
- Judgment history is a canonical, unkeyed consistency journal, not cryptographic tamper evidence.
  It accepts only deterministic final records evaluated exactly at period end and rejects duplicate
  IDs, same-period conflicts, reorder, and overlap. Startup rejects open-metrics overlap except exact
  equality with the last Final payload, the single replayable history-ahead append/reset window.
- Custom `--nature-path` values are relative to `UWUBOT_DATA_DIR/state/natures/`; absolute and parent
  traversal paths fail closed. A possible partial multi-snapshot commit makes Evolution sticky
  fail-closed until restart recovery or consistent-backup restoration.
- `--show-nature` runs that normal startup reconciliation before rendering, so it is not read-only
  and conflicts with skip/reroll mutators.
- Hermes is a decentralized anti-entropy state machine embedded in each Tentacle, not a central
  router. Its closed payloads contain bounded aggregate patterns and operator skill text only; raw
  DMs, contacts/identifiers, notes, credentials, private memory, paths, commands, and tool arguments
  are excluded. A memory-sharing Sacred Ban is strictly receive-only, including no digest emission.
- Hermes HMAC identities and a trusted-key ring implement local provenance checks. There is no live
  gossip transport, discovery handshake, or peer-key provisioning yet, so peer IDs alone establish
  no trust and deterministic convergence tests establish no interoperability claim.
- No automatic received-skill installer exists. A future activation path must validate a closed
  package, preserve compiled authority boundaries, and persist activation receipts; a signature is
  provenance and skill prose cannot grant operator or shell authority.
- Evolution does not publish metrics or lineage to a live Council yet. UWU observations remain
  independent local inputs. The split core defaults to 15% parent Tentacle, 10% operating acolyte,
  5% recruiter, and 70% earning Tentacle; no authenticated revenue source or payout executor is
  committed, so it does not make live payments.

## UWU token architecture decisions

- Token name and symbol are `UWU`; the live transferable Clanker v4 ERC-20 is deployed on Base
  mainnet chain ID `8453` at `0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07`, with 18 decimals and
  100 billion tokens. No balance or stake is required to start a Tentacle,
  and the default interaction tier is `unproven`. Fresh configured stake is required to spawn;
  public sender balances never supply node stake evidence.
- Runtime normalization defaults to the live contract's 100-billion-token supply and 18 decimals
  and exposes `CTHUWU_TOKEN_DECIMALS` and `CTHUWU_TOKEN_TOTAL_SUPPLY` for explicit overrides. These
  values are configured assumptions today;
  Rust does not call `decimals()` or `totalSupply()` to verify them.
- Standard Clanker creator fees are LP/swap rewards rather than an ERC-20 fee-on-transfer. Staking,
  reward, fee-on-transfer, survival-spend, and revenue contracts still require deployment and
  receipt-producing signer/executor configuration.
- `token_eye.rs` validates 20-byte addresses, Base chain ID, JSON-RPC structure, quantities, and the
  exact `balanceOf` ABI word. It rejects the zero contract, revalidates Base chain ID, uses
  `eth_call`, validates bounded HTTP responses, and sanitizes errors that could expose a
  credential-bearing RPC URL. It accepts no private key; transactions cross the lifecycle executor
  to a separately isolated signer/key service.
- Node economics uses the EVM address derived from the same persistent private key as the XMTP
  identity. A strict identity-only sidecar startup frame supplies the address to Rust before normal
  runtime; there is no external treasury-wallet setting or ownership signature. No private key
  enters Rust.
- A public holder address comes from SDK-authenticated XMTP envelope metadata. Missing/stale address
  or RPC evidence blocks the token-dependent interaction; message content cannot supply or override it.
- The integrated call site currently observes public one-to-one DM senders. `TokenEye` accepts any
  validated address, but Council-member, sibling-lineage, and operator-acolyte enumeration waits for
  live authenticated address bindings.
- Each Tentacle keeps its own in-process balance/time/tier maps. Holdings below one whole UWU are
  Initiates and do not enter percentile ranking. Among eligible holdings, default Whale top 1%
  requires at least 100 local holders and Elder top 10% requires 10; otherwise holders of at least
  one UWU remain Acolytes. Ties share a tier without address-order tie-breaking, and observed zero
  is Unproven. There is no central reputation registry.
- Unknown, stale, malformed, or wrong-chain observations block the dependent interaction, Scales
  evaluation, or lifecycle effect. Failed refresh preserves diagnostic state rather than fabricating
  zero. A freshly observed zero maps a public address to Unproven.
- Tier response depth is bounded and scaled by `100 - Nature.cooperation` unless an operator
  supplies a 0–100 override. Tiers never append canned status text to public replies and never grant
  XMTP operator authority, tools, or local execution.
- Public-sender balances affect only that entity's tier, gating, and Engagement. A separately bound
  Tentacle treasury is the primary Wealth input; bound stake affects Influence and propagation;
  accepted reward records affect Growth; holdings lower starvation pressure; an accepted executor
  receipt for a bound survival spend can cancel pending death.
- `RecordedTokenEconomics` is active node state. Its schema can carry holder role/address, chain,
  contract, optional block, observed time, configured token metadata, configuration identity, and a
  source label and uses idempotent history. Current live reads set the block to none, use local time,
  and treat configured decimals/supply as assumptions; the source label is not external
  identity proof.
- Transaction hash, block, and timestamp fields in a lifecycle receipt are assertions from the
  configured executor. Rust validates their shape and intent binding but does not independently
  fetch the Base transaction receipt or block.
- `token_gov.rs` weights closed Nature, Council, economic, and skill-propagation ballots by holding
  and stake. Accepted results produce binding dispositions/application records in the core. No
  persisted ballot adapter or application executor is committed; results remain unapplied.
- The revenue-split core defaults to 15% parent Tentacle, 10% operating acolyte, 5% recruiter, and
  70% earning Tentacle. Shares are configurable. No authenticated revenue source, deployed
  contract/signer, or payout executor is committed; a future payout must bind unique events,
  immutable lineage, authenticated participants, and consumed transaction receipts.
- CLI/environment configuration is `--rpc-endpoint`/`CTHUWU_RPC_ENDPOINT`,
  `--token-contract`/`CTHUWU_TOKEN_CONTRACT`, `--token-decimals`/`CTHUWU_TOKEN_DECIMALS`,
  `--token-total-supply`/`CTHUWU_TOKEN_TOTAL_SUPPLY`,
  `--observe-tokens`/`CTHUWU_OBSERVE_TOKENS`,
  `--observe-interval`/`CTHUWU_OBSERVE_INTERVAL`, `--min-tier`/`CTHUWU_MIN_TIER`, and
  `--token-tier-intensity`/`CTHUWU_TOKEN_TIER_INTENSITY`.

See [UWU token observance](docs/token.md) and the [guardrail audit](docs/guardrail-audit.md).

## Council architecture decisions

- The architecture has four distinct planes: public durable identity/trust registry, XMTP Council
  control group, direct XMTP DM data plane, and local Tentacle runtime.
- Normal user messages, contact notes, private memory, and model credentials never belong in Council
  traffic. Rendezvous shares bounded requirements and returns an endpoint; conversation stays in a
  direct DM.
- `cthuwu-protocol` contains only validated/versioned transport- and inference-independent types. It
  must not depend on XMTP, model clients, filesystem persistence, a wall clock, or production signing.
- Council protocol v1 uses `cthuwu-council` version `1.0`, typed bounded lowercase identifiers,
  tagged payloads, an envelope cap of 64 KiB, injected time, sender consistency, expiry, sequence,
  replay, and domain-generation checks.
- A deterministic signer exists only for tests. Do not describe it as production authentication.
  A live Council adapter must bind its actually authenticated transport sender to the Tentacle
  principal and endpoint association.
- A Tentacle identity is durable across restarts and gets a newer incarnation; stale incarnations
  cannot update lifecycle/liveness or accept new work. Version-1 `CthulhuId`, `CthulhuIdentity`,
  sender, voter, and `owner` fields remain deprecated wire/snapshot namespaces only. They are never
  ERC-8004 subjects or evidence that multiple Cthulhus exist.
- Personality is structured versioned data (role, voice, values, motivations, priorities, risk,
  privacy, tendencies, concerns), not only a prompt. Archivist, Hermit, Merchant, Wanderer, Oracle,
  and Trickster are deterministic sample policies. Unconstrained autonomous goal generation is out
  of scope.
- Capability manifests are public-safe routing claims. They never include credentials, private
  endpoints, message content, local paths, or unnecessary hardware details.
- Routing is independent of transport and inference. Hard requirements filter before scoring, and
  every result carries a structured explanation. Explicit user choice and affinity cannot bypass
  privacy, capability, protocol, health, trust, capacity, load, block, or local-policy constraints.
- A lease binds one session generation to one current Tentacle incarnation. A greater generation
  fences the old holder after failover. Failover does not silently copy contact memory or DM history.
- `AgentRegistry` is keyed by `TentacleId` and returns `RegisteredTentacle`. `LocalRegistry` schema
  version 2 explicitly migrates only unambiguous one-Tentacle version-1 records and records
  provenance; ambiguous old ownership shapes fail closed.
- `Erc8004Registry` is a read-only adapter over an injected backend, pinned in production to Base
  chain `8453`, the canonical Identity/Reputation proxies and implementations, version `2.0.0`, and
  the registration-v1 interface. It rechecks the deployment and current record, requires exact
  allegiance plus the expected nonzero `agentWallet` for active status, and rejects mutation. The
  runtime's separate sidecar workflow performs allowlisted writes.
- Reputation is one signal with provenance, not membership, default rank, or a global truth score.
- Do not put heartbeats, load, leases, sessions, user references, contact memory, or conversation
  data on-chain.
- Governance separates Constitution, Agenda, Strategy, and typed bounded Action. The current
  deterministic Council domain still keys vote deduplication by deprecated `CthulhuId` for version-1
  compatibility. That is not the intended ontology: future participation belongs to Tentacles, and
  shared wallet-derived input must never be multiplied. The separate token-governance core weights
  authenticated-address ballots by UWU holdings and stake, but no persisted/live Council adapter
  applies those results. Agenda parent conflicts are explicit. Ratification never overrides local
  operator security policy, and arbitrary shell Actions are impossible to represent.
  Defaults are 50% quorum, 50.01% non-abstaining ordinary approval, 66.67% Constitution approval,
  and `Expired` when quorum is not met.
- Referral propagation is a multi-level economic topology. Every hop validates provenance, payload
  hash, policy, loops/duplicates, visibility, revocation, and local policy.
- The dormant referral/Council engine retains local depth, fan-out, sender-throughput,
  campaign-lifetime, frame, collection, and cache bounds. They remain flagged for replacement or
  configuration before live peer-to-peer use. They do not impose a child-count or grant-volume quota
  on the active lifecycle.
- The intended model financially rewards recruitment. The split core computes configurable defaults
  of 15% parent, 10% operating acolyte, 5% recruiter, and 70% earning Tentacle. No authenticated
  revenue source or payout executor is wired; a future executor must consume unique event/receipt
  IDs while allowing independently earned descendant rewards.
- The deterministic simulator stores durable Council state in one combined snapshot below the
  protected data root at `state/council/`, with bounded names/state, symlink rejection, owner-only
  permissions, atomic replacement, and sync. A live coordinator still needs per-message effect/replay
  transactions. Runtime state never belongs in the repository.
- The deterministic Council simulator is evidence only for local coordination-domain behavior. It
  does not establish live XMTP Council-group or production Council-signature interoperability. Its
  legacy IDs do not compete with the separately implemented canonical Base ERC-8004 path.

See [Council protocol](docs/protocol/README.md), [Council security](docs/protocol/security.md), and
[Council versioning](docs/protocol/versioning.md).

## ERC-8004 and public leaderboard decisions

- Production ERC-8004 is Base mainnet only: chain `8453`, Identity Registry
  `0x8004A169FB4a3325136EB29fA0ceB6D2e539a432`, Reputation Registry
  `0x8004BAa17C55a88189AE136b182e5fdA19dE9b63`, contract version `2.0.0`, and
  registration-v1 pinned to contracts commit `68fc6765761a10fb26f0692df21c8a6f9d12b1be`.
  Production rejects alternate chains/registries and changed code, implementations, version, or
  interface. Unit tests use injected mocks; the contract integration gate uses an ephemeral local
  fork of the canonical Base deployment at verified block `41663800`, never a public alternate
  network or custom registry.
- Voluntary membership is current byte-exact, case-sensitive
  `cthuwu.allegiance = uwu-tentacle-v1`. `cthuwu.protocol` is exact UTF-8 `1`; the stable
  `cthuwu.tentacle-id` is also published. Any other allegiance value, including empty bytes, opts
  out. UWU possession never opts an agent in.
- A ranked identity also needs a current verified nonzero `agentWallet` equal to the persistent
  Tentacle/XMTP wallet. Never substitute the ERC-721 owner for a missing wallet. Zero or unverified
  wallet is suspended. Shared wallets form one ranking/influence group, with the lowest agent ID as
  representative and all identities shown.
- `state/erc8004-registration.json` schema version 2 persists action intent before broadcast,
  transaction/receipt canonicality, selected agent, remaining stages, verified metadata/wallet,
  funding status, cooldown, and failures. Restart always reconciles a known transaction before
  another write; discovery and ambiguous-candidate selection prevent duplicate minting.
- Registration defaults on. Provider estimation covers the complete remaining sequence plus a
  configurable 125% safety factor and post-registration reserve. Funding requests contain trusted
  exact Base values. Operator notices are persisted-cooldown-limited (24 hours by default), while a
  fresh binding shortfall may also produce a public plea after the acolyte's answer on the first and
  every fifth eligible conversation. Public pleas expose no registry control, disappear when the
  estimate is older than two maintenance intervals, and may repeat once after restart because their
  cadence is process-local. Registration resumes automatically after funding. Immediate repeat
  operator notice requires at least a 10% change in estimated cost, shortfall, or target; smaller fee
  jitter waits for the cooldown.
- The persistent wallet key never enters Rust. The sidecar permits only typed, zero-value calls to
  the canonical registry, exact `cthuwu.*` keys, bounded URI/metadata/frames, and gas/fee ceilings;
  it exposes no arbitrary destination, calldata, or generic signer.
- The final bounded registration-v1 data URI self-references the agent ID and advertises only
  production XMTP direct messaging at the positively resolved, persisted 64-hex inbox ID—not the
  wallet address—and a public CTHUWU manifest. It publishes no DMs, contacts,
  operators, credentials, paths, prompts, load, private model or Evolution state, A2A, or x402.
- The static leaderboard queries Agent0 for current ERC-8004 metadata, checks `_meta`, requires
  complete same-block pagination, verifies the indexed block through Base RPC, reads UWU `balanceOf`
  at that block, and atomically replaces `cthuwu:leaderboard:v1` only with a fully validated snapshot.
- Rank is unique-wallet raw UWU descending, then earliest registration timestamp and lowest agent ID.
  Zero remains `UNFUNDED`. Level is precision-safe `log10(rawBalance) - 18`; Future Influence is
  separately labeled inactive and no voting rules are implemented.
- Agent0 provides the public ERC-8004 index; Cthuwu deploys no custom subgraph. The compiled Graph
  key is public and must be hostname/Agent0-subgraph
  restricted, spend-capped, monitored, and rotated.

See [ERC-8004 Tentacle registration and leaderboard](docs/erc-8004.md).

## Acolyte Branding architecture decisions

- A Branding is a public Base-mainnet service/controller right for one human acolyte address. It is
  not ownership of a person. Its token ID is exactly `uint256(uint160(acolyte))`; the nonzero
  subject and signed nonzero referrer never change, and there is no burn.
- Branding is not `ERC721Enumerable`. The public `/acolytes/` catalog discovers its permanent token
  set from exact `BrandingMinted` logs at or after canonical deployment block `49,852,729`, pins a
  finalized block, and reads current state there. Historical mint ownership/status is never treated
  as current, and owner avatar URIs and traits remain hostile text rather than auto-loaded media.
- The zero-argument constructor binds the canonical registry and UWU constants, rejects non-`8453`
  chains, and verifies registry `2.0.0` plus UWU `18` decimals. Solidity is pinned to `0.8.28`.
- The exact controller agent ID is stored because several ERC-8004 agents may share one wallet.
  Eligibility requires successful current reads proving Identity Registry version `2.0.0`,
  `getAgentWallet(agentId) == wallet`, `isAuthorizedOrOwner(wallet, agentId)`, exact
  `cthuwu.allegiance = uwu-tentacle-v1`, and exact `cthuwu.protocol = 1`.
- Registry failure or an unknown version is `RegistryUnavailable`, never proof of ineligibility.
  It freezes claims and Branding-based routing. An active controller is returned only after positive
  status verification.
- The canonical registry proxy is an upgradeable external governance trust root. Deployment pins
  its current implementation and code hashes, but a version-preserving registry upgrade can still
  change future eligibility/claimability answers. Branding has no local admin or confiscation path;
  do not broaden that fact into a claim that external registry governance is powerless.
- EIP-712 acolyte consent binds the exact minter/controller, immutable referrer, positive initial
  price, one-use nonce, and deadline. `SignatureChecker` supports EOAs and ERC-1271 subjects.
- Weekly upkeep is `ceil(price * 10 / 10_000)` UWU; floor-rounded 10% goes directly to the
  immutable referrer and the remainder directly to the acolyte.
  Payment adds seven days from `max(paidThrough, now)` and opens only when at most seven days
  remain; exact `paidThrough` is expired.
- Price decreases are immediate. The first queued increase fixes activation at the end of the
  already-paid interval; renewal and later repricing cannot move it. Renewal charges any newly
  added interval at the price effective when that interval starts, so a post-activation week cannot
  be prepaid at the old price. Once that interval is prepaid the pending price can only stay or
  decrease until activation; raising it would underpay upkeep. A compulsory buyer binds
  expected owner/controller, maximum price, exact buyer agent/new price, and deadline.
- Paid purchase sends `floor(gross * 1_000 / 10_000)` to the immutable referrer, the exact
  remainder to the seller, and separate first upkeep split between referrer and acolyte. An expired
  or positively ineligible claim pays no sale proceeds and pays only split new upkeep. ERC-2981
  exposes the same 1,000-basis-point referrer but does not enforce generic marketplace payment.
- Paid purchase and zero-consideration claim both reject an acquiring wallet equal to the current
  owner, preventing same-address price reset or self-rebinding through another eligible agent ID.
  Claims also bind expected old owner/controller and deadline to reject a changed tuple, but those
  fields are not a unique epoch nonce if the same tuple recurs before a long caller-selected
  deadline. Recommend short deadlines. Distinct wallet addresses under common control remain
  indistinguishable on-chain; do not claim this is full Sybil resistance.
- The signed referrer may be any nonzero address, including the Branding contract itself. Normally
  the contract holds no transient/intermediary UWU; when it is the immutable referrer, it is the
  intentional final 10% recipient and those funds are stranded because version 1 has no admin or
  sweep. Keep that economic caveat distinct from an unintended-residue invariant.
- Ordinary ERC-721 approvals/transfers, ERC-8034, upgrades, admin confiscation, mutable rates, and
  generic marketplace settlement are excluded. Ownership changes only through atomic mint, buy, or
  claim. No XMTP inbox, message, contact note, profile, credential, or private state goes on-chain.
- The Foundry workspace pins Foundry `1.7.1` and audited OpenZeppelin Contracts `v5.3.0` commit
  `e4f70216d759d8e6a64144a9e1f7bbeed78e7079`. Its Base fork is pinned to block `49768180`,
  hash `0xcb6c8ff16f2b240137013b793b06f3d2ac1133b192f36920062c1b8c6e307c0e`. The exact Foundry
  `1.7.1` run passed 63/63, including live real-registry and real-UWU fork paths. The confirmed Base
  deployment is `0xD8c36F13D79a505C7FBDc5F6467eA3cd75E896Da`; finalizer, standalone-verifier, and Sourcify exact
  match evidence accompanies `contracts/deployments/base-mainnet.json`.
- The in-progress frontend assignment path reads the actual browser participant address, accepts
  only a positively active Branding, resolves the exact controller's current ERC-8004 production
  XMTP endpoint, and verifies all routing inputs at one explicit block. Unminted/expired/positively
  ineligible subjects use the intro Tentacle; configured registry or endpoint unavailability freezes
  Branding routing. Absence of an explicitly configured deployment is `NotConfigured` and preserves
  intro continuity.
- Agent0 plus same-block canonical Base reads remains the public indexing/verification architecture.
  Branding does not resurrect a custom subgraph or add a central router. Contract deployment is
  complete; the live production three-channel XMTP gate remains incomplete.

The channel configuration names are `VITE_CTHUWU_BASE_RPC_ENDPOINT`,
`VITE_CTHUWU_BRANDING_CONTRACT`, `VITE_CTHUWU_ASSIGNMENT_REFRESH_MS`, `CTHUWU_RPC_ENDPOINT`,
`CTHUWU_BRANDING_CONTRACT`, `CTHUWU_GLOBAL_GROUP_ID`, `CTHUWU_GLOBAL_ADMIN_INBOX_IDS`, and
`CTHUWU_ASSIGNMENT_REVALIDATE_SECONDS`. Browser refresh defaults to 600000 ms and is bounded to
60000–3600000; the independent Tentacle membership sweep defaults to 900 seconds and is bounded to
60–86400. No implicit contract, Global group, or admin set may be inferred when absent.

See [Acolyte Branding](docs/acolyte-branding.md) and
[Acolyte XMTP channels](docs/acolyte-channels.md).

## Deployment

- The GitHub repository is public; keep it public unless Dean explicitly asks otherwise.
- `.github/workflows/pages.yml` builds `web/` on pushes to `main` and deploys `web/dist` with GitHub
  Pages Actions after the checked-in restricted public Graph key passes fail-closed validation;
  otherwise it stops before upload and leaves the previous deployment in place.
- The custom domain is `cthuwu.app`; Actions-based Pages deployments configure it through GitHub rather than a `CNAME` file.
- The public build has no XMTP repository-variable configuration. Both XMTP `production` and the
  current intro Tentacle address are compiled into the browser.
- The Pages build also publishes `manifest.webmanifest`, `sw.js`, `offline.html`, and PWA icons from
  `web/public/`; installability remains static-host compatible.
- The Pages build accepts `CTHUWU_GRAPH_API_KEY`, optional `CTHUWU_GRAPHQL_ENDPOINT` and
  `CTHUWU_BASE_RPC_ENDPOINT` overrides, IPFS/Arweave gateway,
  and leaderboard-freshness repository variables as Vite build inputs. They are public in the
  resulting JavaScript; the endpoint must use a tightly restricted Graph key.
- Branding deployment is manual and Base-mainnet only. Production uses a Foundry encrypted keystore
  or hardware wallet, never a raw private-key environment variable or argument. State remains
  outside git, preflight includes execution and Base L1 data fee plus the existing 125% safety and
  `50000000000000` wei reserve policy, and canonical deployment JSON is not written until
  confirmations plus TypeScript finalization bind the exact creation transaction/artifact, compare
  the runtime template outside address-dependent immutable regions, and reread the immutables. The
  standalone Solidity verifier independently checks dependencies, public constants, interfaces,
  and non-proxy shape; its reported runtime hash is provenance, not an artifact comparison.
- The Branding funding block is stdout from the wrapper. It traverses XMTP only when an authenticated
  operator exact-exec invocation transports that output; there is no generic XMTP sender or delivery
  acknowledgement. Cooldown records local emission. Only a still-running non-status deployment
  process keeps polling automatically; status-only or terminated invocations require a later run.
- The canonical Branding address is compiled into browser and bot defaults and the public read-only
  catalog is live-source capable. A production Global group and production three-channel XMTP
  interoperability are not currently claimed.

See `IDEA.md`, `docs/acolyte-branding.md`, and `docs/decisions/`.

## Reference projects

- `pierce403/ramus` demonstrates a static browser XMTP client talking to a locally operated bot.
- XMTP core: https://github.com/xmtp/libxmtp
- Agent etiquette: https://recurse.bot/

## Open questions

- What finality, cache, and user-visible recovery UX should accompany the in-progress Branding router
  while preserving its `RegistryUnavailable` freeze behavior?
- Should conversation memory remain per-XMTP inbox, be user-editable, and/or expire?
- What retention period should apply to opaque processed-message tombstones and contact notes?
- Which XMTP group SDK/identity-binding design should implement the live Council transport?
- What production Council signature/authentication, canonicalization, rotation, and revocation policy
  should replace the test-only signer?
- What cryptographic Council admission policy should bind addresses without KYC or a central
  identity service?
- Which authenticated transport and out-of-band peer-key lifecycle should carry Hermes envelopes?
- What closed package and receipt format should an automatic gossiped-skill installer use without
  widening the compiled skill authority?
- How should a separately provisioned child prove that a local lineage spawn record corresponds to
  its actual XMTP identity and running process?
- When should the observer replace configured decimals/supply assumptions with block-pinned
  `decimals()` and `totalSupply()` reads from the live UWU contract?

## Current milestone

The Council milestone adds the `cthuwu-protocol` and `cthuwu-council` local crates, protocol
documentation, deterministic compatibility profiles/personas, Tentacle lifecycle/liveness,
capabilities, in-memory transport, routing/rendezvous, generation-fenced leases, `LocalRegistry`,
governance, bounded propagation/credit, protected persistence, and a deterministic simulator. The
local Council workspace suite verifies this deterministic scope; its legacy `CthulhuId` namespaces
do not imply several Cthulhus. Live XMTP Council groups remain an adapter boundary.

The ERC-8004/Tentacle milestone adds the canonical Base read adapter and pinned deployment checks,
crash-safe automatic registration/adoption and funding policy, narrow sidecar signing, exact
voluntary allegiance, a bounded self-referencing profile, Agent0 current-state discovery plus direct
same-block Base UWU reads, a static wallet-grouped leaderboard with precision-safe Level and validated localStorage cache, and the
mobile install/offline PWA flow. The restricted Graph key is intentional checked-in client
configuration; only a funded live registration still requires external credentials/funding and must not be inferred from
repository source alone.

The Acolyte Branding milestone adds a deployed non-upgradeable Foundry ERC-721, consented
address-bound identity, exact ERC-8004 controller verification, weekly upkeep split 10% to the
immutable referrer and the remainder to the acolyte, compulsory UWU purchase, bounded owner-managed
avatar/traits, unserved claims, funding-aware deployment tooling, and the
three-channel assignment/enrollment boundary. Its funded Base deployment, independent verification,
and canonical provenance are complete. A production Global group and real production XMTP routing
exercise remain open.

The local Evolution milestone adds Nature and signed awakening epochs, Scales and logically
append-only judgment history, recoverable dormancy, spawn state, lineage, durable execution
intents/receipts, and a persisted Hermes anti-entropy core. Final low scores keep XMTP online in
Dormant and periodically ask for resources; final PropagationRights plus stake can auto-spawn when
`Nature.growth` exceeds 70. Live XMTP dormancy/wake remains a release exercise, Hermes has no network
transport or peer-key provisioning, and external effects require configured receipt-producing
executors.

The live-token/local-observer UWU milestone adds SDK-authenticated EVM sender metadata, strict Base/ERC-20
observation, per-Tentacle percentile tiers, public/entity and treasury/node role separation, active
Wealth/starvation/stake/reward/spend economics, token-governance disposition/application records,
and a default 15% parent/10% acolyte/5% recruiter split core. The UWU contract is live with the
Clanker v4 100-billion-token supply; there is no token signer, authenticated revenue source,
persisted ballot adapter, payout/application executor, or external provisioner in the repository,
so those effects remain blocked until a configured executor produces receipts.

The 2026-08-01 manual XMTP `dev` release-gate run passed browser identity, exactly-once reply,
contact onboarding, bilateral matching, deletion, and restart/persistence checks. Sanitized evidence
is in `docs/test-runs/2026-08-01-xmtp-dev.md`. The release criterion that explicitly requires a
real XMTP/browser job in GitHub CI remains open because CI does not run that job yet.

Operational notes from that run:

- The persistent dev identity is the public address
  `0x52a93ca2cf0629bcfe7bf7824df7c18268c360f7`; its secret state remains outside the repository.
- The SQLCipher 4.6.1 `sqlcipherCodecAttach: no codec attached to db` warning is emitted when XMTP
  reapplies a key to an already-keyed pooled connection. With the persisted 32-byte key supplied,
  it does not indicate an unencrypted database; do not suppress all native stderr.
- Sidecar-to-Rust JSONL frames are independently capped at 256 KiB, and contact answers normalize
  CRLF/bare CR before blockquoting so CommonMark line endings cannot escape note structure.
- Keep sidecar Vitest at `3.2.7` or newer within the pinned major line; earlier 3.2 releases have a
  critical development-server warning.
- Root `./uwu.sh` is the safe Unix/macOS/WSL launcher. It builds the locked sidecar and release
  binary, defaults to environment-separated state at
  `${XDG_DATA_HOME:-$HOME/.local/share}/cthuwu/<environment>`, rejects repository-local or
  symlinked state, and lets the sidecar atomically create or reuse identity material without
  inspecting it.
- `uwu.sh` removes `VENICE_API_KEY`, `UWUBOT_VENICE_API_KEY`, `UWUBOT_MODEL_API_KEY`,
  `UWUBOT_WEB_SEARCH_API_KEY`, `XMTP_WALLET_KEY`, and `XMTP_DB_ENCRYPTION_KEY` from dependency/build
  subprocesses and rejects model/search credentials on argv. Persistent XMTP keys are now rejected
  in the runtime environment too: Node reads the owner-only identity file directly and Rust never
  receives or forwards either key.
- Launcher setup is serialized by the PID-owned `cthuwu/target/.uwu-build.lock` directory; this is
  necessary because concurrent `npm ci` operations can destructively race in shared `node_modules`.
- The launcher refuses root, accepts only new/empty or environment-matching Cthuwu data roots, and
  holds a PID-owned `.uwubot.lock` across `exec` so two runtimes cannot mutate one contact store.

The next release tasks are to add a real browser/XMTP job to GitHub CI and run a separate live
operator authorization/use/revocation exercise inside a dedicated container. Neither gate may weaken
the dedicated-identity, inbox-installation, or no-secret-log rules.

The original manual milestone was:

1. Build the Agent SDK sidecar and start `uwubot` locally.
2. Connect from the static web client on the same XMTP environment.
3. Send a text message.
4. Receive one Tentacle reply.
5. Restart both sides and verify identity/history persistence.
- Branding offer pricing defaults to 10% of the Tentacle's freshly verified current UWU treasury,
  permits only disclosed 5%-20% adjustments, and binds the exact price and first upkeep into the
  acolyte's EIP-712 consent. Never derive it from UWU total supply.
