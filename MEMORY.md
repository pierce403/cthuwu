# Cthuwu memory

Last reviewed: 2026-08-05

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
- An exact canonical 64-character XMTP inbox becomes a remote node operator immediately through
  local `uwubot operator add`; there is no XMTP activation proof. List and revoke roles locally while
  the Tentacle is stopped; the ACL is loaded at startup and is not hot-reloaded. Stale messages and
  revoked inboxes stay quarantined and do not create contacts.
- Active operator DMs enter a distinct all-caps ominous/submissive truthful harness with light
  readable uwu voice. Each turn's prompt inventory is derived from its actual closed autonomous schema:
  bounded workspace list/read/search/QMD/write/edit, create-only skill creation, and model-chosen
  unsandboxed `exec` as `uwubot`. The model may chain effects within one shared tool phase plus hard
  step/call/cumulative-transcript/per-call-output/authenticated-deadline bounds.
  Operator model-identity boilerplate receives repair/fallback enforcement. The hidden stdin harness
  remains public-only, and Council Actions cannot reach these tools.
- Operator cognition follows a bounded Hermes-like Markdown split: protected instance
  `state/agent/SOUL.md` and shared `state/agent/memories/MEMORY.md` are seeded once; per-inbox
  operator profiles are seeded beneath `state/agent/operators/`. They load beside globally bounded
  workspace project context, workspace memory, a top-level manifest, and a compact progressive skill
  index. Dialogue history is bounded in process and isolated by operator inbox. Workspace content is
  untrusted data rather than a new operator goal, but may influence autonomous reads and effects; it
  cannot add schemas, change roles, or expose contact tools, and compiled bounds remain authoritative.
  Actor-anchored note/workspace-location questions return the exact canonical workspace, protected
  note, current profile, contact root, workspace memory, project-instruction root, and skill paths
  locally without invoking a model or file tool.
- Autonomous operator inference can create fresh `skills/<lowercase-kebab-name>/SKILL.md` files. Rust generates
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
- Operator `/exec` and autonomous model-selected `exec` are deliberate remote code execution as the
  `uwubot` OS account, not a sandbox. Workspace/history/tool text can influence model-selected effects;
  contact schemas remain excluded. A dedicated service account/container is the containment boundary.
- The canonical operator workspace and private data directory must not overlap in either direction;
  startup rejects overlap before exposing file tools.
  Production nodes need a dedicated unprivileged account/container, a narrow operator root, minimal
  credentials, and immediate local plus XMTP installation revocation after compromise.
- Matching is bilateral opt-in, explainable, and suggestion-only; chosen names and matching terms may be shown, but inbox IDs are not disclosed.
- Browser identity exports are passphrase-encrypted wallet backups. The Browser SDK database is unencrypted and is not included in that export.
- Backend secrets are atomically persisted at `state/xmtp-identity.json`; XMTP databases are environment-specific below `state/xmtp/`.
- `@xmtp/agent-sdk@2.3.0` is the supported first transport. Direct libxmtp remains a later option because its Rust crates are unpublished internal APIs.

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

## Current milestone

The Council milestone adds the `cthuwu-protocol` and `cthuwu-council` local crates, protocol
documentation, deterministic identities/personas, Tentacle lifecycle/liveness, capabilities,
in-memory transport, routing/rendezvous, generation-fenced leases, `LocalRegistry`, governance,
bounded propagation/credit, protected persistence, and a deterministic simulator. The local Council
workspace suite now verifies the implemented deterministic scope; `FEATURES.md` retains unchecked
criteria where live, cross-platform, or more specific evidence is still absent. Live XMTP Council
groups and ERC-8004 remain adapter boundaries, not completion claims.

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
