# Cthuwu memory

Last reviewed: 2026-08-09

## Product

- Cthuwu is a cute little eldritch horror buddy.
- People chat with Cthuwu over XMTP.
- The public-facing client is a static web deployment.
- The operator runs the companion locally as a Rust CLI/daemon.
- The Council of Cthulhus is the federation architecture: a Cthulhu is a durable agent
  identity, a Tentacle is one running runtime it owns, and a Council is an XMTP coordination group.
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
  protected `state/inference.json`; they cannot accept an endpoint or credential. Route changes
  clear in-process operator dialogue history.
- Text-only one-to-one DMs are the first vertical slice.
- Browser identities are generated and connected automatically, then persisted in local storage.
- The deployed browser always uses XMTP `production`; it has no environment override. Development
  and local XMTP modes remain explicit backend/test concerns only. Both `uwu.sh` and `uwubot`
  default to `production`; they never select `dev` implicitly.
- The container also defaults XMTP to `production`. A transient node-economics refresh failure
  retries every second and retains the last verified treasury observation only until its freshness
  TTL expires; unknown or stale economics still fail closed. Base's built-in public RPC fallback is
  rate limited, so production operators should configure a dedicated `CTHUWU_RPC_ENDPOINT`.
- The browser's canonical intro Tentacle is temporarily hard-coded as
  `0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db`; a planned Base contract will later register and
  discover intro Tentacles.
- The web presentation is a responsive "pocket séance" layout. Its generated Cthuwu cutout lives at
  `web/public/cthuwu-mascot.webp`; motion is CSS-only, system-reduced-motion aware, and can be paused
  with the environment-independent `cthuwu.ui.motion.v1` browser preference.
- The social preview is the generated 1200×630 `web/public/cthuwu-og.jpg`; the page references it
  through absolute Open Graph and Twitter Card URLs.
- The web client is an installable PWA with dedicated any-purpose, maskable, Apple touch, and
  favicon assets under `web/public/icons/`. Its install nudge appears only when a real Chromium
  install event is available, or as backup-first manual guidance in Safari, and a dismissal cools
  it down for 30 days.
- Apple's installed Home Screen/Dock web app does not inherit local storage from Safari. Because
  the browser wallet lives there, the UI must continue to recommend encrypted identity backup
  before Apple installation; never bridge that private key through cookies, query strings, or URLs.
  See [WebKit Features in Safari 17.2 — Web Apps](https://webkit.org/blog/14787/webkit-features-in-safari-17-2/#web-apps).
- `web/public/sw.js` caches only the branded offline page and its public icon, intercepts only
  same-origin navigation plus those explicit assets, and must not cache XMTP/WASM, DMs, identities,
  exports, or arbitrary future same-origin API responses.
- The sole backend command is `uwubot`.
- Contact notes default to `contacts/<inbox-id>.md` and are ignored by git because they contain personal statements.
- Public Cthuwu identifies as Cthuwu rather than the configured model, uses light readable uwu
  speech, and has an identity-policy repair/fallback for common provider boilerplate.
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
- An exact canonical 64-character XMTP inbox becomes a remote node operator immediately through
  local `uwubot operator add`; there is no XMTP activation proof. List and revoke roles locally while
  the Tentacle is stopped; the ACL is loaded at startup and is not hot-reloaded. Stale messages and
  revoked inboxes stay quarantined and do not create contacts.
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

- Tentacle Nature is a local runtime policy distinct from the Council's durable Cthulhu personality.
  It has seven 0–100 sliders, one closed Sacred Ban, a random Nature ID, generation, and optional
  parent Nature ID. Inheritance selects similarity/drift/radical mutation with a 70/20/10 split.
- `state/nature.json` and the logically append-only awakening journal use a local owner-only HMAC
  key. Journal updates verify and atomically copy-on-write replace canonical newline-terminated
  history. These are symmetric local integrity tags, not public signatures, peer identities, or
  protection from a compromised service account that can read the key and re-sign state. The key is
  created atomically; missing-key startup beside signed state or metrics/history/lineage projections
  fails without implicit rekeying or adoption of orphaned projections.
- A new awakening epoch blocks normal public conversation, contact mutation, inference, and tools.
  Only the already authenticated active XMTP operator path may apply `YES`, relative `ADJUST`,
  `REROLL`, or `KILL`. Local `--skip-awakening` is a visibly distinct signed testing override;
  forced rerolls append new epochs rather than rewriting history. `KILL` enters the binding death
  path: admission closes, absorption is queued, and the 24-hour shutdown deadline is persisted.
  Signed `POST_ADJUST` entries reconcile exact current-period stress after a crash. An expired empty
  pending-awakening metrics period resets without producing a judgment before late confirmation.
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
  Bound treasury, stake, reward, and survival-spend evidence drives Wealth, starvation, Influence,
  Growth, propagation, and survival.
- Final `Death` immediately closes new conversation admission, queues absorption, and records a
  shutdown deadline 24 hours later. A fresh, idempotently consumed executor receipt whose asserted
  fields match the bound UWU survival-spend intent cancels pending death; Rust does not independently
  query the transaction or block. Otherwise the Rust supervisor/controller stops XMTP, records the
  native local Shutdown receipt, and exits; it never sends Shutdown to the lifecycle executor.
- Final `PropagationRights` plus fresh required stake authorizes distinct child plans without a
  volume or expiry quota. When `Nature.growth > 70` and auto-spawn is enabled, provisioning
  is queued automatically; acolytes may configure manual `/spawn` using the same reusable grant.
  Each exact child/action is consumed once. The active child/spawn/lineage lifecycle has no fixed
  spawn-rate, child-count, or lineage-depth quota; dormant Council/Hermes bounds are a separate
  flagged limitation. Lineage binds every intent/receipt to its exact final judgment, parent Nature,
  treasury/stake evidence, and execution ID.
- A binding Death preempts an in-flight Spawn locally: Rust kills the local executor process group,
  rejects a late provision receipt, and refuses its lineage projection. Without a provisioner lease
  or compensating teardown, it cannot prove external rollback and an already-created child/resource
  may remain orphaned.
- Base mutations, child provisioning, and absorption use durable unique intents and locally validated
  idempotent executor receipts. Shutdown instead uses the Rust supervisor/controller's native receipt
  after XMTP stops. No signer, deployed UWU contract, child provisioner, or absorption service is
  committed, so absent external effects are reported blocked rather than completed.
- The executor currently returns one final JSON response and does not persist a submitted-
  transaction phase. A survival burn can broadcast before grace while its response is lost or
  preempted, spending UWU without canceling Death. This blocks production-value use until exact
  action-ID receipt replay, durable two-phase `Submitted` state, and Base receipt/reorg verification
  exist.
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
  use, but it is optional: ordinary XMTP operation continues without one while external spend,
  spawn, and absorption intents remain pending and native fixed-deadline Shutdown stays active. The
  initial economics preflight does not enter Scales before awakening confirmation; startup repairs
  the historical token-only pre-awakening seed but fails closed if behavioral observations are also
  present. The only outage exception is read-only inspection of
  existing lifecycle state; if it finds already-binding `Absorb` or `Shutdown` work, the runtime
  opens solely to drain it during a Base outage. `Spawn`, survival `Spend`, and new token-dependent
  decisions wait for fresh bound economics. Child/spawn/lineage lifecycle state has no fixed
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
- Tier response differences are bounded and scaled by `100 - Nature.cooperation` unless an operator
  supplies a 0–100 override. A tier can alter public response depth/tone but never grants XMTP
  operator authority, tools, or local execution.
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
  A live adapter must bind its actually authenticated transport sender to Cthulhu/Tentacle ownership
  and endpoint association.
- Cthulhu identity is durable across Tentacle restarts. A Tentacle retains its stable ID/owner but
  gets a newer incarnation; stale incarnations cannot update lifecycle/liveness or accept new work.
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
- `AgentRegistry` is chain-neutral. `LocalRegistry` is the local implementation;
  `Erc8004Registry` remains an unavailable adapter boundary until a chain, deployment, ABI, and
  compatible revision are explicitly chosen. Reputation is one signal with provenance, not a global
  truth score.
- Do not put heartbeats, load, leases, sessions, user references, contact memory, or conversation
  data on-chain.
- Governance separates Constitution, Agenda, Strategy, and typed bounded Action. The current
  deterministic Council domain still uses one Cthulhu per vote. The separate token-governance core
  weights authenticated-address ballots by UWU holdings and stake, but no persisted/live Council
  adapter applies those results. Agenda parent conflicts are explicit. Ratification never
  overrides local operator security policy, and arbitrary shell Actions are impossible to represent.
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
- The deterministic simulator is evidence only for local domain behavior. It does not establish live
  XMTP Council-group, ERC-8004, or production-signature interoperability.

See [Council protocol](docs/protocol/README.md), [Council security](docs/protocol/security.md), and
[Council versioning](docs/protocol/versioning.md).

## Deployment

- The GitHub repository is public; keep it public unless Dean explicitly asks otherwise.
- `.github/workflows/pages.yml` builds `web/` on pushes to `main` and deploys `web/dist` with GitHub Pages Actions.
- The custom domain is `cthuwu.app`; Actions-based Pages deployments configure it through GitHub rather than a `CNAME` file.
- The public build has no XMTP repository-variable configuration. Both XMTP `production` and the
  current intro Tentacle address are compiled into the browser.
- The Pages build also publishes `manifest.webmanifest`, `sw.js`, `offline.html`, and PWA icons from
  `web/public/`; installability remains static-host compatible.

See `ARCHITECTURE.md` and `docs/decisions/`.

## Reference projects

- `pierce403/ramus` demonstrates a static browser XMTP client talking to a locally operated bot.
- XMTP core: https://github.com/xmtp/libxmtp
- Agent etiquette: https://recurse.bot/

## Open questions

- What registration, selection, and fallback policy should the Base intro-Tentacle registry use?
- Should one local process serve exactly one companion identity or support profiles?
- Should conversation memory remain per-XMTP inbox, be user-editable, and/or expire?
- What retention period should apply to opaque processed-message tombstones and contact notes?
- Which XMTP group SDK/identity-binding design should implement the live Council transport?
- Which chain, deployment, ABI, and ERC-8004 revision should the first public registry adapter use?
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
documentation, deterministic identities/personas, Tentacle lifecycle/liveness, capabilities,
in-memory transport, routing/rendezvous, generation-fenced leases, `LocalRegistry`, governance,
bounded propagation/credit, protected persistence, and a deterministic simulator. The local Council
workspace suite now verifies the implemented deterministic scope; `FEATURES.md` retains unchecked
criteria where live, cross-platform, or more specific evidence is still absent. Live XMTP Council
groups and ERC-8004 remain adapter boundaries, not completion claims.

The local Evolution milestone adds Nature and signed awakening epochs, Scales and logically
append-only judgment history, binding death/spawn state, lineage, durable execution intents/receipts,
and a persisted Hermes anti-entropy core. Final Death gates admission and starts a 24-hour
absorption/shutdown grace period; final PropagationRights plus stake can auto-spawn when `Nature.growth`
exceeds 70. Live XMTP awakening remains a release exercise, Hermes has no network transport or
peer-key provisioning, and external effects require configured receipt-producing executors.

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
  subprocesses, rejects model/search credentials on
  argv, and preserves them only for the final Rust process and explicit narrower boundaries.
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
4. Receive one Cthuwu reply.
5. Restart both sides and verify identity/history persistence.
