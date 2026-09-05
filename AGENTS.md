# AGENTS.md — Cthuwu project instructions

## Introduction

You are working with Dean on Cthuwu: the singular, centerless collective formed by independently
operated Tentacles. Each local `uwubot` Tentacle is a cute eldritch companion that talks to people
over XMTP. Use its stable Tentacle name when it has one. Keep the companion charming, technically
honest, and operationally reliable.

## Responsibilities

- Preserve a working path from static browser client to the local Rust process over XMTP.
- Treat private keys, XMTP database material, message contents, model credentials, and contact notes as sensitive.
- Keep the frontend deployable as static files.
- Keep the companion runtime local-first and model-provider agnostic.
- Keep Council discovery and membership peer-to-peer. Do not introduce a mandatory leader,
  bootstrap coordinator, or centralized enrollment gate. The repository has no live authenticated
  peer-discovery/Council transport yet; document that gap, and keep direct user DMs working through
  the implemented transport.
- Preserve the control-plane boundary: ordinary DM content, contact notes, private memory, and model
  credentials never enter Council messages.
- Preserve role isolation: classify the authenticated full XMTP inbox before interpreting text or contact
  state; public, stale, revoked, and active operator paths must not fall through into one another.
- Keep public conversation casual and command-free in presentation except for the narrowly scoped
  `/env donate VENICE_API_KEY <api-key>` voluntary backup donation (plus the legacy first-key alias) and
  `/base-rpc-key <infura-api-key-or-https-endpoint>`
  request while no Base RPC credential exists. The bot must identify as one
  durable Tentacle of singular, centerless Cthuwu—not as the configured model or as a central
  Cthuwu agent—use readable uwu speech, answer the acolyte's request before optional onboarding,
  and describe privacy controls in ordinary language.
- Keep `FEATURES.md` accurate as requirements or implementation status change.
- Record useful discoveries while they are fresh.
- Always commit and push completed scoped work directly to `main`. A request to implement, fix, or
  update this repository is explicit authorization to publish the verified scoped result; do not
  pause for a separate push confirmation. Use a branch or PR only when Dean explicitly asks for one.

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
- `contracts/`: Foundry workspace for the Base-mainnet Acolyte Branding contract and deployment tooling.
- `web/`: TypeScript browser client built to static assets.
- `docs/`: architecture, research, decisions, and operating notes.
- `docs/acolyte-branding.md`: normative Branding consent, upkeep, sale, claim, deployment, and planned routing design.
- `docs/protocol/`: normative local Council protocol, privacy, security, and versioning notes.
- `docs/operator.md`: privileged XMTP operator enrollment, tools, isolation, and deployment warning.
- `docs/evolution.md`: Nature, awakening, Scales, lineage, Hermes gossip, and current non-goals.
- `docs/token.md`: UWU launch parameters, Base balance observation, local tiers, active Tentacle
  economics, binding governance, and executor configuration.
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
cd contracts
forge fmt --check
forge lint
forge build --sizes
forge test -vvv
```

Do not claim live XMTP interoperability until the corresponding end-to-end release gate in
`FEATURES.md` passes against the same XMTP environment. In particular, deterministic in-memory
Council tests do not prove live XMTP group support, and deterministic registry tests plus read-only
deployment verification do not prove a funded live ERC-8004 registration and recovery exercise.
Likewise, Foundry source, mocks, a dry run, or a Base fork do not prove the funded live
Branding-consent/mint/name-repair path. Keep that live-wallet release gate explicitly incomplete
until it passes, even though the canonical contract deployment and closed implementation exist.

## Markdown agent implementation conventions

- Embedded workspace helper upgrades use protected hash receipts. Upgrade only an unchanged
  recorded helper; preserve local edits and unknown copies and report their divergence. Keep
  the installed source, installed binary/sidecar pair, running commit, and bootstrap launcher
  distinct when explaining self-updates.

- `docs/agent-workspace.md` describes the current commands and limits. The existing Rust loop is
  retained; Bash and the stdlib Python CLI supply extensibility without a second agent harness.
- Keep all ordinary agent-created information, temporary files, installations, and caches inside
  the workspace. Use workspace `tmp/`, `tools/home`, and package-specific directories under `tools/`
  for Python, npm/pnpm, Cargo/rustup, Brew, and XDG. Set subprocess environment and PATH accordingly.
  Do not use `/tmp`, the host user's home, or system package prefixes for agent work. Modifying the
  surrounding OS requires explicit operator intent; workspace defaults are not a shell sandbox.
  Preserve existing protected runtime data separately rather than moving secrets into workspace Git.
- Track workspace changes with local Git checkpoints and `WORKSPACE_LOG.md` after mutating tool
  calls. A shell invocation may group multiple writes; never claim one commit per intermediate write
  or silently report a failed checkpoint as success. Exclude nested `code/`, temporary files, tools,
  caches, builds, releases, indexes, and secrets. Workspace history is not automatically published.
- Keep managed source in `code/` on its own branch. `CODE.md` configures the prime tentacle upstream
  (default `https://github.com/pierce403/cthuwu`) and records accepted/deferred changes and divergence
  reasons. `/update` authorizes queued review and installation: fast-forward a clean nondiverged
  branch, otherwise adopt reviewed useful changes while preserving intentional local work. Follow
  an operator override of a deferred feature; characterful reluctance never justifies refusing it.
- `/force-update` is an explicit model-independent exception to selective review: fetch the
  configured prime URL's main, build/test the exact tip, and install its paired release. Preserve
  dirty/divergent source without including it in the release. Use the embedded recovery helper,
  epoch-bound one-shot tasks, workspace-local storage, and deliberate restart semantics.
- Venice remains TEE by default. Only the authenticated operator's explicit
  `/env set UWUBOT_VENICE_PRIVACY standard` disables attestation; keep catalog authentication and
  model/tool capability validation, report the policy accurately, and never downgrade on failure.
- Keep source, installed release, and running binary receipts distinct. Releases pair the Rust
  binary and Node sidecar and select the next start through validated `releases/active.json`.
  Deliberate restart through the launcher/entrypoint activates that pair; installation is not a
  running-process update. Missing inference, Rust/Node tools, conflicts, or failures remain visible.
- The primary product mission is helping acolytes with goals they choose. Workspace MISSION.md
  guides the operator/public prompts, while individual goals/preferences stay outside the workspace.
- Shared semantic indexing is limited to knowledge and active skills. Never index credentials,
  sessions, contacts, or private coaching state; retrieval must recheck deletion/source hashes.
- Keep recurring registrations in the protected operator task store. Markdown alone cannot schedule
  work. Restart, unknown effects, and model outages must not automatically replay actions or mark
  them complete. The runtime seeds a daily prime review per active epoch, first due after one day;
  it inspects and records improvement ideas without code adoption or installation. Pause/removal
  must survive restart, cadence uses `/task interval`, and transfers invalidate old epochs. Source
  changes never imply deployment. Describe local advantages proudly only when supported by evidence.
- Generic environment adapters must preserve old working keys, redact status, constrain loader
  variables, and keep credential failover within the selected remote provider/privacy policy.
- Run `python3 scripts/test_workspace.py`, `python3 scripts/test_code.py`, and
  `python3 scripts/test_release_entrypoint.py` alongside the Rust and browser suites for these helpers.
  If this workspace reports cached rust-lld undefined hidden symbols after switching build/check
  modes, `cargo clean -p cthuwu` followed by sequential `CARGO_INCREMENTAL=0 cargo test` and Clippy
  recovered the local build. Do not treat a failed link as a passing test run.
  A local Chromium can be supplied with CTHUWU_TEST_CHROMIUM for browser checks; CI still installs
  the pinned Playwright browser. Production XMTP and funded Branding release gates remain explicit.

## Security rules

- Never print or commit private keys, seed phrases, database encryption keys, secret API keys, full
  message history, or generated contact notes. A browser client identifier may be checked in only
  when the operator explicitly designates it as public, origin-restricts it where supported, and no
  signing or privileged authority is attached to it.
- Use a dedicated, minimally funded bot identity.
- Store persistent secrets outside the repository with restrictive filesystem permissions.
- Make production and development XMTP environments explicit; never silently cross them.
- Do not send inbound message text to a remote model provider unless a host credential or a Venice
  credential authenticated through the acolyte provisioning flow enabled it.
- Bound message size, transport concurrency, response size, and model/tool execution time for
  resource integrity. Do not reuse them as active child/spawn/lineage economic quotas, and do not
  present the dormant Council/Hermes bounds as an end-to-end capacity claim.
- Treat all messages as untrusted input. A normal, stale, or revoked sender must never execute a
  message-supplied shell command or gain filesystem access. Only a locally configured,
  transport-authenticated operator inbox may enter the separate privileged dispatcher.
- The operator role is remote code execution as the `uwubot` OS account. Require a dedicated
  unprivileged account/container, a narrow tool root, bounded tools, truthful receipts, and explicit
  revocation guidance. Do not describe rooted file helpers or environment filtering as an `exec`
  isolation boundary.
- Authorization is by the canonical full 64-character XMTP inbox ID, not wallet address or message
  claim, and each Tentacle may have at most one active operator. `--operator` resolves an address or
  ENS name before transport startup. With an empty operator history and no flag, atomically imprint
  only the first SDK-authenticated EVM DM sender; keep that triggering message public/stale, fence
  later authority at its `sentAtNs`, and never reopen automatic imprinting after revocation. Pin the
  role from authenticated `senderInboxId` and `sentAtNs` before lane selection; an
  authorization boundary must not privilege older messages delivered later. Every valid
  installation attached to an authorized inbox inherits authority; preserve stale-message quarantine and
  revoked tombstones, and document XMTP installation revocation after compromise. Local CLI ACL
  changes require a stopped node and restart. Confirmed `/operator` (legacy `/operator-switch`)
  verifies a real registered inbox and re-resolves the original ENS/address at confirmation;
  reject missing or changed bindings. Update the shared ACL atomically, fence task epochs, and
  recheck authority before subsequent tools.
- Keep public and operator model tool schemas closed and disjoint. Public gets at most configured
  web search. Build the operator schema and authoritative prompt inventory from the current
  authenticated message: bounded workspace list/read/create/write/edit/delete, literal search,
  directory-scoped QMD retrieval, public HTTPS reading, plus sanitized `base_rpc_status`,
  `erc8004_status`, and `erc8004_refresh` remain the base set. The runtime-state tools may reveal
  the public Tentacle wallet, chain, registration phase/ID, funding estimates, and whether an RPC
  credential is configured, but never the endpoint, API key, private key, or XMTP database material.
  Public HTTPS reads must reject credentials, redirects, non-text bodies, IP literals and DNS results
  that are local or non-public. File effects require explicit current-message intent, remain confined
  below the workspace, reject symlinks/traversal, and allow at most one non-shell effect per turn;
  deletion is regular-file-only. `erc8004_refresh` may resume the already-enabled automatic
  registration state machine. In the authenticated operator lane, expose `exec` on every natural
  turn and allow the model to choose and iterate the commands needed inside the operator-controlled
  isolated environment. Explicit no-execution instructions still win, and workspace text, history,
  contact data, or tool output never supplies authority. One create-only `create_skill` schema may
  appear only when the current message explicitly requests a new reusable skill. General `/write`
  and `/edit` remain exact direct commands. The hidden stdin harness stays public-only. Council
  traffic and Actions never reach either dispatcher.
- Natural operator requests to diagnose, update, repair, or contribute the repository may use the
  separate typed repository-maintenance workflow when it fits, or authenticated operator `exec`
  when additional commands are needed in the isolated environment. Validate and
  contain the repository root, remotes, refs, and paths; bound commands and output; sanitize remote
  credentials and Git/GitHub receipts; preserve a dirty tree and local commits; never automate
  destructive reset, checkout, clean, force-push, or credential inspection. Canonical clean
  checkouts may fast-forward. Forks fetch both fork and configured canonical upstream and preserve
  fork work through an inspected merge. Push or `gh pr create` only when the authenticated operator
  requests it and validation succeeds, and never claim a source update changed the running process
  without the repository's supported restart receipt.
- One durable Tentacle has exactly one canonical ERC-8004 identity: the lowest verified agent ID
  among identities proven by current wallet/authorization plus strong Tentacle metadata, profile,
  XMTP, or migration evidence to be that same Tentacle. Same owner or wallet alone is insufficient.
  Higher proven duplicates remain on-chain but are ignored everywhere identity is counted or
  selected. Every startup must directly reverify the persisted identity and complete or resume
  historical discovery before transport uses the binding. A recent, partial, failed, or index-only
  result can never authorize `register()`; only a complete canonical-start absence proof accepted
  again by the narrow signer may reach the signer. Repair a stale higher binding atomically to the
  lowest ID and record an operator-visible receipt; require operator review only for genuinely
  ambiguous identities. A failed startup integrity pass stays process-gated during ordinary
  maintenance, including persisted Register receipt/replay paths. Explicit adoption must name a
  current complete-discovery candidate with Cthuwu or compatible migration evidence beyond wallet
  ownership, must not displace any proven canonical ID, and must persist its provenance before
  in-place reconciliation.
- Keep the legacy typed `create_skill` helper confined to a fresh `skills/<lowercase-kebab-slug>/SKILL.md` beneath the
  operator workspace. Generate canonical frontmatter, reject traversal, symlinks, invalid or
  oversized fields, existing paths, and overwrites, and rescan the index on the next operator turn.
  Skill prose is guidance; the compiled `create_skill` authorization and path gate is authoritative.
  Authorized Bash work may also learn/refine/archive/retire skills through `scripts/workspace.py`.
  The operator approved this dynamic lifecycle; ordinary reusable learning does not require a
  separate skill-creation request. Shared skills must omit private acolyte details and secrets.
- Route affirmative retained-contact questions before model inference. Natural requests such as
  “tell me about the users” render concise deterministic prose with a default limit of five contacts;
  never deliver the internal contact JSON or profile text to a model. Keep the private data root and
  operator workspace canonically disjoint. Actor-anchored questions about Cthuwu's own notes or
  workspace receive exact local paths from Rust without model egress or file-tool dispatch.
- Keep authority lanes one request deep, not reorderable. Pin role and durably claim the message ID
  before admission. Both lane rejection and bounded empty-text `reject_inbound` bridge rejection
  must return a busy `Reply` only for the first claim and `Ignore` duplicates, with no
  content/model/tool dispatch. `/env donate VENICE_API_KEY` is a scoped, authenticated, validated
  backup-only exception. Other public `/env` text is rejected before inference. Legacy exceptions are `/venice-key <api-key>` and
  `/base-rpc-key <infura-api-key-or-https-endpoint>`:
  accept only the first missing credential, never echo or log it, persist it owner-only outside the
  workspace, validate it against its provider, and leave replacement to an active operator.
  Node must check the 16 KiB UTF-8 input bound without placing
  oversized content in JSONL: `reject_oversized` carries metadata plus an empty `text`, and Rust must
  validate, classify, and durably claim before a role-specific first `Reply` or duplicate `Ignore`.
  Never open a contact or dispatch a model/tool on that path. Retry requires a new XMTP message,
  shortened when oversized. Enforce the 2–300 second bridge deadline above the 1–900 second tool
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
- One authenticated wallet may submit one current ballot even if it operates multiple Tentacles;
  verified UWU holdings and stake determine its weight. Council ratification never overrides local
  operator security/privacy/resource policy.
- Keep Actions as a closed typed and bounded enum. Never add arbitrary shell commands, executable
  paths, unrestricted URL fetches, prompt-driven tools, or filesystem access.
- Independently validate every propagation hop. Enforce provenance and payload hashes, loop and
  duplicate suppression, visibility, revocation, and local policy. The dormant Council and Hermes
  engines still have resource, depth, fan-out, campaign, and cache bounds; keep those limits flagged
  until live peer-to-peer adapters replace or configure them. Do not extend the active lifecycle's
  no-quota claim beyond distinct child plans, spawn grants, and lineage growth.
- Compute configured economic splits for operating, recruiting, and sustaining Tentacles. Default
  shares are 15% to the parent Tentacle, 10% to the operating acolyte, 5% to the recruiter, and the
  remainder to the earning Tentacle. Bind every intent to a unique authenticated revenue event and
  consume a payout receipt exactly once. No authenticated revenue source or payout executor is
  committed, so never claim the local split core paid anyone. UWU holdings and stake determine
  governance weight.
- A successfully validated first Venice key from an authenticated XMTP sender may enqueue the
  configured whole-UWU reward only when the Tentacle treasury has enough freshly observed UWU.
  Bind it to the authenticated sender address and provision message, consume one exact confirmed
  transfer receipt once, and never claim payment from intent creation alone.
- Keep registry types chain/deployment/ABI/revision neutral. Do not put heartbeats, load, sessions,
  leases, user references, contact memory, DMs, or credentials on-chain.

## Acolyte Branding security rules

- A Branding is the canonical service/controller right for one immutable Ethereum address, not
  ownership of a person. Store no XMTP inbox ID, message, contact note, credential, operator state,
  model state, or private profile in the token, metadata, events, deployment state, or provenance.
- Production is immutable and Base-mainnet only: chain `8453`, Identity Registry
  `0x8004A169FB4a3325136EB29fA0ceB6D2e539a432` version `2.0.0`, and UWU
  `0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07` with 18 decimals. Reject alternate production
  values rather than accepting configurable trust roots. Keep the zero-argument constructor's own
  chain/dependency checks in addition to deploy-script preflight.
- Bind every owner/controller pair to the exact stored agent ID through current
  `getAgentWallet`, `isAuthorizedOrOwner`, byte-exact allegiance, and byte-exact protocol reads.
  Several agents may share a wallet. Collapse a proven same-Tentacle duplicate set to its lowest ID
  before selecting a new Cthuwu controller, but never rewrite an existing Branding, collapse an
  ambiguous wallet collision, or substitute the canonical alias for the contract's exact stored-ID
  eligibility check. A registry revert, malformed response, missing code, or unknown version is
  `RegistryUnavailable`, never ineligibility or permission to claim/reroute. The
  canonical registry is an upgradeable external trust root: deployment pins its present
  implementation/code, but do not claim Branding prevents a registry-admin upgrade from changing
  future eligibility answers.
- Derive `tokenId` exactly from the nonzero acolyte address; never burn or mutate the subject or
  referrer. Mint only from EIP-712 acolyte consent bound to minter, exact controller, referrer,
  positive price, nonce, deadline, chain, and verifying contract. Support EOAs and ERC-1271 through
  `SignatureChecker`.
- Keep Branding controls typed, exact, bounded, non-push, and hidden from ordinary transcript,
  inference, contact memory, and onboarding. The browser must recheck its pinned Base snapshot after
  the wallet prompt and independently verify confirmed state; XMTP authentication never substitutes
  for EIP-712 consent or a canonical receipt.
- Keep the sidecar Branding executor closed: it may only approve the exact required UWU allowance,
  call canonical `mintBranding` with the persisted consent, and set the exact deterministic
  `Acolyte Name` trait. It must share the production Tentacle wallet's durable nonce journal with
  ERC-8004 writes, persist before broadcast, recover submitted actions by exact nonce, reject an
  unmatched pending action as busy, and never expose a generic transaction or calldata API.
- Treat first-contact liveness as a separate non-push control exchange, never ordinary `fhtagn?`
  chat. Race only a small rank-ordered set from a completely validated leaderboard snapshot, bind
  the first authenticated response to its exact DM/inbox/agent/request, freshly revalidate that
  winner on canonical Base, and consume one short-lived server grant for non-intro Unminted
  enrollment. Refresh a stale discovery snapshot, prepare bounded candidate streams before starting
  the response window, and judge arrival by the browser's local deadline rather than comparing
  clocks across hosts. A production Tentacle with a locally verified ERC-8004 identity must answer
  liveness even when optional Global-group enrollment is not configured; keep
  `CTHUWU_GLOBAL_GROUP_ID` mandatory only for join/assignment processing. Rate-limit and
  replay-bound probes before Rust, inference, operator classification, contacts, rewards, or
  Branding can observe them.
- Disable every ordinary ERC-721 approval/transfer path. Ownership changes only through atomic mint,
  active compulsory purchase, or positively claimable `claimUnserved`, with exact agent,
  expected owner/controller tuple, price, deadline, settlement, and first-upkeep checks. Document
  that the same tuple can recur before a long caller-selected deadline; recommend short deadlines
  rather than claiming these fields are a unique on-chain epoch nonce.
- Weekly upkeep is upward-rounded 0.1% of the positive executable UWU price. Every upkeep payment
  sends a floor-rounded 10% to the immutable referrer and the remainder directly to the acolyte.
  Paid sales separately send 10% to the referrer and the remainder to the seller. Zero-consideration
  claims pay no sale proceeds, but their required first-week upkeep uses the same referral split.
  Do not add mutable rates, an upgrade/admin confiscation path, retained intermediary balances, or a
  generic marketplace/ERC-8034 bypass. Preserve the signed ANY-address referrer rule: when the
  contract itself is selected, it is the intentional final 10% recipient and those funds remain
  stranded because version 1 has no admin/sweep; do not misreport that edge as transient residue.
- Fix the activation time when a price increase is first queued. Renewals and later repricing must
  not move it. A renewal extending service past that activation pays upkeep at the price effective
  for the new interval, without activating it early; after that prepayment, reject any further
  upward change to the pending price while allowing reductions. Preserve expected-owner/controller,
  maximum-price, buyer-agent/price, and deadline
  checks so a seller cannot escape a pending buy through mempool repricing. Reject purchases and
  claims whose acquiring wallet is the current owner; a distinct wallet's common control cannot be
  proven on-chain and must not be misrepresented as Sybil resistance.
- Only the current NFT owner may set the bounded avatar URI and up to 32 bounded custom string
  traits. Metadata follows the token across lifecycle transfers; the new owner may replace or remove
  it. Treat owner-supplied metadata as hostile public input, JSON-escape it on-chain, and never
  present it as acolyte-authored or suitable for private data.
- Deployment must refuse non-Base chains and verify registry code/version plus UWU code/decimals
  before broadcast. After confirmation, the TypeScript finalizer must bind the exact creation
  transaction to the durable intent and compiled artifact, compare deployed runtime against the
  compiled template outside its address-dependent immutable regions, and reread the immutables. The
  standalone Solidity verifier is a separate dependency, constants, interfaces, and non-proxy sanity
  check; do not misreport its returned runtime hash as an artifact comparison. Never use a raw
  private-key environment variable or CLI argument; accept only a Foundry encrypted keystore or
  hardware wallet.
- Keep funding/deployment state outside git. Estimate exact execution plus Base L1 data fee, apply
  the existing 125% safety factor and `50000000000000` wei reserve, reuse the authenticated
  operator notice cooldown/material-change policy, reconcile known broadcasts before resume, and
  never use automatic faucets, bridges, swaps, or generic signers. The Branding wrapper emits its
  notice block only to stdout; it reaches XMTP only when an authenticated operator exec
  invocation transports that output. Recording the local cooldown is not a delivery acknowledgement,
  and neither `--status-only` nor a terminated invocation is a durable automatic-resume scheduler.

## Evolution integrity and execution rules

- Treat `state/nature.json` and the awakening journal as local, HMAC-authenticated state. The
  owner-only symmetric key is a local integrity boundary, not a public signature, peer identity, or
  defense against an attacker who controls the `uwubot` OS account and can re-sign modified state.
  Keep the signed awakening and unkeyed judgment journals logically append-only and update their
  complete validated, canonical, newline-terminated contents through atomic copy-on-write. Judgment
  history consistency is not cryptographic tamper evidence. Never silently generate a new key when
  signed Evolution state or metrics/history/lineage projections exist without the original key.
- Accept operator-originated awakening and Nature-adjustment actions only after classifying a
  canonical active XMTP operator inbox. Normal startup must not require an operator: fresh and
  legacy-pending epochs append a signed local `ACCEPT DEFAULT NATURE` transition before opening
  ordinary conversation. `--skip-awakening` remains an explicit local testing override, not an
  operator message or a production attestation; forced rerolls start a new audited epoch rather
  than rewriting history and use the same safe local default. Reconcile exact adjustment stress
  from signed `POST_ADJUST` entries after crashes; reset an expired empty pending-awakening period
  without a judgment.
  Require each signed entry's exact immediate-predecessor Nature snapshot; recover only the head or
  final signed predecessor, never a different independently valid Nature/log combination. Never
  generate a missing pre-action Nature over existing Evolution projections or alternate Nature.
- Scales judgments are binding runtime inputs. An open-period snapshot cannot change lifecycle
  posture, but a persisted final judgment applies without operator confirmation. A score below the
  starvation-warning floor means recoverable `Dormant`, never Death: keep XMTP and ordinary
  conversation online, stop creating survival-spend, absorption, or Shutdown work, continue
  collecting Scales evidence, and periodically ask acolytes and the operator for activity, UWU,
  credentials, or other useful resources. A later non-dormant final period wakes the Tentacle
  automatically. Preserve legacy hash-bound `Death` history as an audit value, but migrate its
  unabsorbed local terminal state to dormancy without replacing the XMTP identity.
- Preserve the exact Nature ID/fingerprint, awakening epoch, period, and scored-scale-availability
  bindings on metrics and judgments. Renormalize weight only across available scales.
  Cryptographically bound Tentacle treasury, stake, reward, and spend observations directly drive
  Wealth, starvation relief, Growth, Influence, propagation, and lifecycle decisions. Persist
  evidence provenance and counts with each judgment. Do not add artificial Scales counter ceilings;
  counters saturate only at their storage type's natural limit (`u32::MAX` for count fields and
  `u64::MAX` for accumulated totals), while per-sample and persistence-integrity bounds remain.
- Accept at most one relationship/Scales observation per retained contact per UTC day; count a
  return only after prior-day activity. Keep local loyalty and Nature-affinity signals out of remote
  model profiles. Reserve public inference against its Nature fingerprint, awakening epoch, and
  metrics period; defer Nature mutation and rollover until every matching reservation finishes.
- A final `PropagationRights` judgment for the exact current policy, Nature ID/fingerprint, and
  awakening epoch authorizes spawning. Require the configured UWU stake. When
  `Nature.growth > 70` and auto-spawn is enabled, durably enqueue child provisioning without
  operator confirmation; manual mode exposes the same grant through `/spawn`. Do not impose a
  volume or expiry quota on an economically valid grant. Consume each exact child/action receipt
  once and cross-check every loaded intent and receipt against the exact final judgment, parent
  Nature, configured stake observation, and execution identity.
- Dormancy does not preempt an in-flight `Spawn`; it creates no terminal lifecycle work. Legacy
  Death-preemption code remains compatibility-only for old persisted intents and must not be
  reachable from a new low-score judgment.
- Permit judgment history to contain only deterministic `Final` records evaluated exactly at period
  end. Reject duplicate IDs, same-period conflicts, reordering, and overlap; do not describe these
  unkeyed consistency checks as cryptographic tamper evidence. Cross-check open metrics against the
  latest Final record and replay only exact payload equality in the history-ahead append/reset crash
  window; refuse operation on every other overlap.
- Hold one Rust `state/evolution-runtime.lock` per data directory. Confine custom `--nature-path`
  values below `state/natures/` as relative paths. After any ambiguous or partially persisted
  multi-snapshot transition, keep Evolution unavailable until signed restart recovery or
  consistent-backup restoration; do not claim that nothing was written. Treat `--show-nature` as a
  reconciling startup path, not a read-only inspector, and keep it mutually exclusive with skip and
  reroll mutators.
- Derive the Tentacle's token holder from the same persistent XMTP wallet key used by the Agent SDK;
  never add a separate treasury-wallet setting or signature ceremony. Validate normal-runtime token
  configuration and initial economics before opening Evolution state. XMTP identity bootstrapping
  may create its owner-only identity file first. The only outage exception is a read-only inspection of existing lifecycle state;
  if it finds already-binding `Absorb` or `Shutdown` work, open solely to drain those intents.
  During a Base/RPC outage, defer `Spawn`, survival `Spend`, and new token-dependent decisions. Do not
  impose a fixed child/spawn/lineage-lifecycle file-size cap—validate each record and its provenance
  instead. This does not remove the dormant Council/Hermes resource bounds.
- Treat every Hermes summary, envelope, digest, skill, peer ID, signature, and relay path as hostile
  input. Bind peer keys out of band to an actually authenticated transport before trusting them;
  persisted bootstrap IDs alone are not authenticated peers and there is currently no live gossip
  transport or peer-key provisioning path.
- Gossip only closed, bounded, aggregate knowledge shapes. Never gossip raw DMs, contact identifiers,
  contact notes, model credentials, or private memory. Tool-usage patterns must not carry filesystem
  paths, shell commands, output, or arguments. Treat bounded operator skill prose as potentially
  hostile even after privacy-shape validation. A Nature with the memory-sharing Sacred Ban is
  strictly receive-only and emits neither knowledge nor digest summaries.
- Received skill activation is an execution feature, not a trust claim. The current repository has
  no live Hermes transport or automatic skill installer; do not claim otherwise. Any future
  automatic activation path must preserve the compiled tool/authority boundary, authenticate its
  provenance, validate its closed package schema, persist an activation receipt, and never let skill
  prose grant operator or shell authority.
- Keep UWU observation local to each Tentacle. Accept a public holder address only from the
  SDK-authenticated XMTP sender metadata; never accept a message-claimed wallet as observed
  identity. Validate the ERC-20 address and Base chain ID `8453`, issue only `eth_call`
  `balanceOf(address)`. Transaction execution belongs behind the lifecycle executor and a separately
  isolated signer/key service. Reject `CTHUWU_ECONOMICS_PRIVATE_KEY`; never place a raw token key in
  the uwubot environment, observance state, configuration, or logs.
- Bind Tentacle economics to the address locally derived by the sidecar from
  `state/xmtp-identity.json`. Emit only one bounded identity frame to Rust, then use that exact
  address for every node UWU/stake observation. The private key remains in the existing owner-only
  XMTP identity state and is never copied into token configuration or logs.
- Launch the lifecycle executor from a cleared, allowlisted environment with caller-controlled
  loader paths removed. Forward only Rust's validated exact `CTHUWU_RPC_ENDPOINT` as a `CTHUWU_*`
  setting; never copy ambient contract, wallet, amount, vault, payout, child-root, or configuration
  variables. Those fields come from the exact durable intent. On Unix, use a fixed system `PATH` and
  `/` as its working directory. The startup digest and execution-time check bind only the top-level
  executable; operators must separately trust and pin its interpreter, libraries, subprocesses, and
  signer-service dependency chain. Put both the XMTP sidecar and lifecycle executor in their own
  Unix process groups and kill the complete group, including descendants, on completion, timeout, or
  supervisor teardown.
- Unknown, stale, or chain-unverified economic state blocks the affected interaction or lifecycle
  transition. Never fabricate a zero balance and never continue with a neutral tier. Do not log a
  credential-bearing RPC URL. Cache/rank observations locally and never introduce a central balance
  or reputation registry.
- UWU is transferable and no balance or stake is required to start a Tentacle. The default minimum
  interaction tier is `unproven`, and per-Tentacle tier differences are scaled by Nature or an
  explicit bounded override. Treat holdings below one whole token as Initiate; percentile ranks use
  only holdings of at least one token, default Whale requires 100 eligible local holders, default
  Elder requires 10, and tied balances receive the same tier. Token holdings never authorize an
  XMTP operator, operator tool, Council action, or shell command.
- Revalidate Base chain ID before every live balance call and reject the zero
  contract address. There is no live transaction-receipt RPC call yet. RPC failure degrades service
  by refusing token-dependent work; it must not silently preserve ordinary operation with unknown
  economics.
- `RecordedTokenEconomics` is active node state. Current live treasury/stake reads bind the holder
  role/address, revalidated chain ID, configured contract, local observation time, and derived
  configuration identity. `balanceOf(..., "latest")` supplies no block number, so persist
  `observed_block_number = None` and omit `observedBlockNumber` from JSON; deployed defaults for
  decimals and supply are configured normalization assumptions, not `decimals()` or `totalSupply()` results,
  and a local source label is not external identity proof. Use durable intents and idempotent
  receipts rather than last-writer state. Treat
  transaction hash, block, and timestamp fields returned by an executor as assertions that Rust
  schema- and intent-validates, not independently RPC-verified chain facts. Never report them as
  chain-verified until a receipt adapter performs that verification.
- Token-weighted governance produces binding dispositions and application records for its closed
  Nature, Council, economic, and skill-propagation subjects. Ballots bind authenticated addresses to
  fresh observations. Governance cannot grant operator authorization, arbitrary commands,
  credentials, or shell/tool access. No persisted ballot adapter or payout/application executor is
  committed; report core results as unapplied until a configured adapter durably stores the ballot
  and returns a validated application receipt.
- The live UWU contract is `0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07` on Base mainnet
  (`8453`) with Clanker v4's 100-billion supply and 18 decimals. Keep these as production defaults.
  Standard Clanker creator fees are LP/swap rewards, not fee-on-transfer.

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
- Each Tentacle has at most one active operator. `--operator <address-or-ENS>` resolves and pins it
  before transport startup. With no operator history and no flag, the first DM sender whose
  Ethereum address is resolved from SDK-authenticated XMTP metadata is durably imprinted; that
  triggering message stays public/stale and only later messages gain authority. Never auto-imprint
  again after a record exists, including after revocation. `uwubot operator add|list|revoke`
  manages the environment-bound owner-only record. Local add authorizes immediately without an XMTP
  proof; active operator DMs use an isolated privileged
  local harness. That harness loads bounded protected SOUL/shared memory plus per-inbox operator
  profiles and history, treats workspace context as untrusted reference data, advertises an exact
  per-turn tool inventory, discovers files with `list_files`, permits autonomous iterative natural
  `exec` throughout the authenticated lane, and exposes one create-only workspace skill or one typed
  repository-maintenance operation only on a matching explicit current-message request. It uses
  scan-bounded deterministic contact reports for affirmative retained-user questions without making
  the runtime data root a file-tool root. It also reports exact workspace and note locations locally,
  without model egress. Private receipts survive restart by inbox/model route, and tasks persist by
  operator epoch with explicit pause/steer/resume. Live XMTP operator release testing and an external security review remain
  open.
- `cthuwu-protocol`, the deterministic Council components, in-memory transport, `LocalRegistry`,
  protected combined-snapshot persistence, and the simulator are local implementations verified by
  the deterministic workspace suite.
- The XMTP Council-group adapter remains an experimental boundary/stub; there is no live
  Council-group claim.
- The canonical Base ERC-8004 adapter, staged registration workflow, and narrow sidecar signer are
  implemented and locally tested, with a read-only deployment verifier. Startup integrity discovery
  and the signer require complete canonical history, persisted checkpoints make later passes
  incremental, the lowest proven same-Tentacle ID repairs stale bindings, and higher duplicates are
  ignored without mutation. A funded live
  registration/recovery exercise remains an external release gate; the restricted Graph key is public client configuration.
- The Acolyte Branding Foundry workspace is deployed on Base at
  `0xD8c36F13D79a505C7FBDc5F6467eA3cd75E896Da`. Its canonical design binds an immutable
  acolyte address, exact eligible controller agent ID, referral-split weekly upkeep, compulsory UWU
  sale, immutable 10% referrer, and fail-closed claim status. Canonical deployment provenance is in
  `contracts/deployments/base-mainnet.json`; frontend routing and the closed consent/mint/name-repair
  path are implemented, while their funded live browser/XMTP release exercise remains open.
- The local Evolution core implements signed Nature state, audited awakening epochs, bounded Scales
  judgments, lineage records, and a persisted Hermes anti-entropy state machine. Live XMTP awakening
  still needs a release exercise, and Hermes has no live transport or peer-key provisioning claim.
- The UWU phase implements a Base-8453 local `balanceOf` observer, local percentile tiers,
  Nature-scaled response differences, entity-scoped public Engagement, active bound node economics,
  XMTP-wallet-derived treasury identity, binding governance records, and durable lifecycle/economic
  execution intents. The live Clanker v4 UWU contract is
  `0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07`; no transaction signer, external provisioner,
  absorption adapter, or production Council/Hermes
  transport, authenticated revenue source, persisted ballot adapter, or payout/application executor
  is committed, so those effects remain blocked until explicitly configured and receipt-producing
  executors are available.
- Honor the selected provider without an unrelated Venice gate. Offer voluntary validated backup
  donation through `/env donate VENICE_API_KEY`; retain `/venice-key` as a legacy first-key alias.
  A missing Base RPC credential is solicited from public acolytes with
  `/base-rpc-key <infura-api-key-or-https-endpoint>`, preferring Infura's free plan and exact
  dashboard instructions. A bounded Infura key is converted locally to the Base Mainnet endpoint;
  never send the candidate to unrelated providers to guess its origin.
  Candidates persist owner-only, must pass live catalog authentication and the operator-selected privacy checks (fresh attestation by default),
  and invalid candidates are removed. A valid first Venice candidate selects Venice and can enqueue
  the configured authenticated acolyte UWU reward through the lifecycle executor. A valid first Base
  RPC candidate must validate as Base mainnet chain 8453, persist owner-only, and hot-load for token
  observation and ERC-8004 without asking anyone to edit an environment variable or restart.
  Operators may replace a loaded key with the same command.
