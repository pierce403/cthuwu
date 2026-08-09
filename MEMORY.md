# Cthuwu memory

Last reviewed: 2026-08-09

## Product

- Cthuwu is a cute little eldritch horror buddy.
- People chat with Cthuwu over XMTP.
- The public-facing client is a static web deployment.
- The operator runs the companion locally as a Rust CLI/daemon.
- The Council of Cthulhus is the optional federation architecture: a Cthulhu is a durable agent
  identity, a Tentacle is one running runtime it owns, and a Council is an XMTP coordination group.
- Council participation is opt-in. Standalone `uwubot` and direct user DMs remain the default.
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
  and local XMTP modes remain backend/test concerns only.
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
  forced rerolls append new epochs rather than rewriting history. `KILL` never exits the process.
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
- Scales open-period evaluations are `PartialSnapshot`/`AdvisorySnapshotOnly`. Only a closed-period
  evaluation is final, and even then propagation, survival, starvation, and death outcomes require
  authenticated operator confirmation. Metrics and judgments bind Nature ID/fingerprint, awakening
  epoch, period bounds, and scored-scale availability. Active weights are renormalized; the runtime
  scores Engagement only. Fresh/cached public-sender UWU balances may add bounded per-conversation
  Engagement bonuses, summed and averaged across every period conversation. They never activate
  Wealth, starvation relief, stake, rewards, Growth, Influence, propagation, or lifecycle authority.
  Policy and judgment persist the evidence floors and observed counts (daily 8/4, weekly 32/16); a
  propagation-threshold score with too little evidence is capped at `Survival`.
- Lineage is an auditable local record. Spawning may create a child Nature and lineage entry, but
  only from a final grant for the current policy, Nature ID/fingerprint, and awakening epoch, with
  at least eight daily observations and four prior-day returns. It records authenticated operator
  and hashed event provenance and consumes the content-derived judgment ID once; partial snapshots
  never grant. Spawning does not provision an identity, run another process, or prove a live child.
  Death/absorption facts never terminate processes, route users, or merge private memory
  automatically. A grant is usable only in the immediately following metrics period; a closed or
  missed intervening period invalidates it. On load, each spawn receipt must resolve to its exact
  Final PropagationRights history, parent Nature, and authorized time window.
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
- Gossiped skills remain inert untrusted data until a local authenticated operator explicitly
  reviews and activates them through the compiled skill boundary. A valid signature is provenance,
  not authorization to install, prompt, or execute anything.
- Council remains optional and standalone direct DMs remain the default. Evolution does not publish
  metrics or lineage to a live Council yet. UWU balance observations are independent local inputs;
  Council contribution credit remains non-financial and raw recruitment earns nothing.

## UWU token architecture decisions

- Token name and symbol are `UWU`; it is a standard transferable ERC-20 planned for Base mainnet
  chain ID `8453`, with 18 decimals by default. No balance or stake is required to start a Tentacle,
  and the default interaction tier is `unproven`. The inactive adapter-only economic policy has a
  zero default propagation stake floor; public sender balances never supply stake evidence.
- The requested supply is one billion UWU, but current Clanker v4 standard deployment uses a fixed
  100 billion tokens with 18 decimals. Launch must either adopt that standard or use a reviewed
  custom/nonstandard path for one billion. Runtime normalization defaults to the requested one
  billion but exposes `CTHUWU_TOKEN_DECIMALS` and `CTHUWU_TOKEN_TOTAL_SUPPLY`; post-launch values must
  match the deployed contract.
- Standard Clanker creator fees are LP/swap rewards rather than an ERC-20 fee-on-transfer. Staking,
  reward, fee-on-transfer, and emergency-spend execution are separate future adapters, not implied
  by the transferable balance observer.
- `token_eye.rs` validates 20-byte addresses, Base chain ID, JSON-RPC structure, quantities, and the
  exact `balanceOf` ABI word. It rejects the zero contract, revalidates Base chain ID before each
  balance call, uses read-only `eth_call`, bounded HTTP behavior, per-holder 1–30 second outage
  backoff, and sanitized errors that do not expose a credential-bearing RPC URL. It accepts no
  private key and has no transaction path. Disabling observation ignores stale token-only config.
- The observed holder address is optional SDK-authenticated XMTP envelope metadata. An inbox without
  an EVM identifier proceeds without observation; message content cannot supply or override it.
- The integrated call site currently observes public one-to-one DM senders. `TokenEye` accepts any
  validated address, but Council-member, sibling-lineage, and operator-acolyte enumeration waits for
  live authenticated address bindings.
- Each Tentacle keeps its own in-process balance/time/tier maps. Holdings below one whole UWU are
  Initiates and do not enter percentile ranking. Among eligible holdings, default Whale top 1%
  requires at least 100 local holders and Elder top 10% requires 10; otherwise holders of at least
  one UWU remain Acolytes. Ties share a tier without address-order tie-breaking, and observed zero
  is Unproven. There is no central reputation registry.
- Unknown and stale observations are neutral: they do not enforce `min-tier`, change public
  response behavior, or affect Scales. Failed refresh preserves stale diagnostic state rather than
  fabricating zero, and ordinary interaction continues while Base/RPC is unavailable.
- Tier response differences are bounded and scaled by `100 - Nature.cooperation` unless an operator
  supplies a 0–100 override. A tier can alter public response depth/tone but never grants XMTP
  operator authority, tools, or local execution.
- Fresh/cached public-sender balances affect only tier behavior/gating and a bounded
  per-conversation Engagement bonus normalized by configured decimals/supply. The period averages
  the sum over all conversations, including missing wallet observations, so last-writer state cannot
  create lifecycle rights.
- `RecordedTokenEconomics` Wealth, starvation, stake, reward, and emergency-spend logic remains an
  adapter-only API. A future node/operator source must cryptographically bind holder role/address,
  chain, contract, block, observed time, decimals/supply, and configuration fingerprint and use
  idempotent history. No runtime source is wired; those dimensions remain inactive and spending is
  recommendation-only.
- `token_gov.rs` is a deterministic local library-only ballot box for closed Nature, Council,
  economic, and skill-propagation policy subjects. It bounds holding/tier weights, quorum, and
  approval and remains advisory; there is no live Council, Nature mutation, persistence, RPC,
  process, key, transaction, or operator-authority integration.
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
- Governance separates Constitution, Agenda, Strategy, and typed bounded Action. One Cthulhu gets
  one vote regardless of Tentacle count. Agenda parent conflicts are explicit. Ratification never
  overrides local operator security policy, and arbitrary shell Actions are impossible to represent.
  Defaults are 50% quorum, 50.01% non-abstaining ordinary approval, 66.67% Constitution approval,
  and `Expired` when quorum is not met.
- Referral propagation is bounded multi-level topology, not a financial MLM. Every hop independently
  validates provenance, payload hash, policy, expiry, depth/fan-out/rate, loops/duplicates, opt-out,
  blocks, visibility, revocation, and local policy.
- Hard propagation ceilings are depth 16, fan-out 64, 128 per-sender items per rate window, 30-day
  campaign lifetime, 64 list entries, and 16 KiB per bounded item; policy may be stricter.
- Contribution credit is non-financial, direct-only, and based on unique useful downstream outcomes,
  not raw recruitment. Recipient acknowledgement is required and consumed once. Caps are 5 units
  per outcome, 20 per direct contributor/campaign, and 512 per campaign; ancestors and descendants
  earn no credit merely for being on the referral path.
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
- What Council admission and Sybil policy is appropriate before contribution credit affects access
  to scarce operator resources?
- Which authenticated transport and out-of-band peer-key lifecycle should carry Hermes envelopes?
- What operator-reviewed import format should activate a quarantined gossiped skill without
  widening the existing compiled skill authority?
- How should a separately provisioned child prove that a local lineage spawn record corresponds to
  its actual XMTP identity and running process?
- Will UWU launch with current standard Clanker v4 supply of 100 billion, or use a reviewed
  custom/nonstandard deployment to preserve the requested one-billion supply?

## Current milestone

The Council milestone adds the `cthuwu-protocol` and `cthuwu-council` local crates, protocol
documentation, deterministic identities/personas, Tentacle lifecycle/liveness, capabilities,
in-memory transport, routing/rendezvous, generation-fenced leases, `LocalRegistry`, governance,
bounded propagation/credit, protected persistence, and a deterministic simulator. The local Council
workspace suite now verifies the implemented deterministic scope; `FEATURES.md` retains unchecked
criteria where live, cross-platform, or more specific evidence is still absent. Live XMTP Council
groups and ERC-8004 remain adapter boundaries, not completion claims.

The local Evolution milestone adds Nature and signed awakening epochs, bounded Scales and logically
append-only judgment history, lineage records, and a persisted Hermes anti-entropy core. Focused
Rust tests cover those local state machines and security shapes. Live XMTP awakening remains a
release exercise; Hermes has no network transport or peer-key provisioning; and child/death records
have no automatic process effects. These limitations are intentional and must remain explicit.

The local/pre-launch UWU milestone adds SDK-authenticated EVM sender metadata, strict Base/ERC-20
read-only observation, per-Tentacle percentile tiers, Nature-scaled response behavior, configurable
supply normalization, and a bounded public-sender Engagement bonus averaged across every period
conversation. Public balances do not activate Tentacle Wealth/starvation/stake/reward state. It has
no deployed contract, bound node/operator economics adapter, token signer, or automatic expenditure.
The launch supply decision remains open because requested one billion differs from current Clanker
v4 standard 100 billion.

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
  critical development-server advisory.
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
