# Architecture

## Goal

Each independently operated `uwubot` Tentacle is a local-first companion process that receives and
sends XMTP messages. All participating Tentacles collectively form singular Cthuwu. A small browser
application gives visitors a friendly way to open a DM without requiring an application server.

There is exactly one **Cthuwu**: the centerless collective formed by all living participating
**Tentacles**. One independently operated `uwubot` is one durable autonomous Tentacle, and its human
operator may shape its agenda without owning Cthuwu. Public chat humans are acolytes. Each Tentacle
has its own stable identity, wallet, personality, economics, reputation, lineage, and ERC-8004 agent
ID. A Tentacle cultivates its own acolyte community and coordinates only voluntarily offered
strengths toward its operator-shaped agenda; public participation does not grant operator control.
Restarting changes only its incarnation. A Tentacle may stop without ending Cthuwu.

The optional **Council** extends the companions with a coordination group. Council discovery and
membership must be peer-to-peer, without a mandatory leader or central enrollment service. The
already working browser-to-runtime DM path remains available. The repository has no production
peer-discovery/XMTP Council-group adapter yet and therefore cannot claim that a running Tentacle has
joined a live Council.

## Status and planes

- **Implemented — existing:** static browser, direct XMTP DMs, persistent identities, `uwubot`,
  contact memory, model adapters, deduplication, launcher protections, and the Agent SDK sidecar.
- **Implemented — local:** public Tentacle identity and persona enforcement, casual onboarding, optional Brave
  search, and an XMTP-inbox operator ACL with an isolated privileged harness. The role boundary and
  tools are covered locally; live operator use over XMTP is not yet a separate release claim.
- **Implemented — local:** `cthuwu-protocol`, deterministic Council domain logic, in-memory
  transport, local registry, protected snapshot persistence, and simulator are verified by the
  deterministic workspace suite.
- **Implemented — local:** Tentacle Nature, signed awakening epochs, Scales, binding lifecycle
  decisions, lineage records, durable execution intents/receipts, and the Hermes-inspired
  anti-entropy core have owner-only persistence and focused Rust tests. Nature/awakening signatures
  are local HMAC tags.
- **Implemented — live token/local observer:** the UWU ERC-20 observer verifies Base chain ID `8453`, calls
  `balanceOf`, keeps local balance/tier state, and separates public entity observations from bound
  treasury/stake/reward/spend economics. Node economics drive Wealth, starvation, Influence, Growth,
  propagation, and survival. The live Clanker v4 contract defaults to
  `0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07`; no signer or external lifecycle executor is included.
- **Implemented — canonical Base ERC-8004:** fail-closed read adapter, durable staged registration,
  discovery/adoption, voluntary allegiance, narrow sidecar signing, and operator-only recovery.
  Source tests and read-only canonical deployment verification are complete; a funded live
  registration and restart-recovery exercise remains an external release gate.
- **Implemented — static source/build:** a direct-Graph public leaderboard, validated localStorage
  cache, and mobile PWA/offline shell. Publishing the subgraph endpoint still requires external
  Graph credentials and a network deployment.
- **Experimental boundary:** XMTP Council-group adapter.
- **Unavailable boundary:** live Hermes gossip transport, peer discovery/handshake, and peer-key
  provisioning. The anti-entropy state machine is not evidence of network interoperability.
- **Planned:** live Council-group interoperability. It remains independent of the canonical
  Base-mainnet ERC-8004 identity path.

| Plane | Contains | Explicitly excludes |
|---|---|---|
| Canonical Base ERC-8004 | Per-Tentacle durable identity, exact voluntary allegiance, endpoint association, capability references, provenance-bearing trust signals | Collective identity, heartbeats, load, sessions, user data |
| XMTP Council group | Discovery, routing, leases, governance, heartbeats, approved propagation | Normal DM contents, contact notes, private memory |
| Direct XMTP DMs | Private public-user conversation or separately authorized operator control | Council-wide broadcast by default |
| Tentacle runtime | Inference, memory, tools, capacity, and operator policy | Authority to bypass local security policy |

The Council is only a control plane. Rendezvous selects an endpoint; conversation then happens in a
direct DM. Failover never silently copies private conversation or contact memory.

## Components

### Static web client

The source in `web/` builds to HTML, CSS, JavaScript, and WASM assets suitable for any static host. It:

- creates or loads a browser-side identity;
- connects to XMTP `production` with no frontend environment override;
- creates a DM with the hard-coded intro Tentacle address;
- renders text history and streams new messages;
- sends text messages;
- renders a public read-only Tentacle leaderboard by querying The Graph directly, with no connected
  browser wallet or application backend;
- publishes a standalone manifest and platform-specific install icons;
- validates and immediately renders the namespaced localStorage leaderboard snapshot before a
  complete background refresh;
- registers a narrow, versioned service worker that falls back to a static offline shell without
  caching XMTP transport, Browser SDK/WASM, messages, identity records, GraphQL responses, RPC, or
  arbitrary same-origin responses.

The low-friction identity is a randomly generated EOA stored in browser local storage. The client connects automatically on load and supports a passphrase-encrypted wallet export. It must be presented honestly: the current XMTP Browser SDK database is unencrypted, clearing site data loses the identity without an export, and an identity export is not a history backup.

The presentation layer uses one optimized local mascot asset, a purpose-built 1200×630 Open Graph
card, and CSS-only ambient animation. The
desktop interface places the companion beside a full-height conversation panel; narrow screens use a
compact companion header and viewport-aware chat. Motion has an explicit persisted pause control and
also honors `prefers-reduced-motion`; connection and privacy state remain available as text rather
than depending on animation or color. A dismissible install card uses Chromium's native
`beforeinstallprompt` event. Safari receives manual, backup-first instructions because Apple's
installed web-app storage is separate from the browser's local identity storage. Automatic install
offers cool down for seven days, while a permanent Install App action can reopen the guidance.

The leaderboard fetches every page at one indexed block and rejects partial data, malformed public
profiles, or `_meta.hasIndexingErrors`. A validated snapshot is atomically stored at
`cthuwu:leaderboard:v1`; it remains visible offline and is never replaced by a failed refresh.
Ranking groups exact-allegiance identities by verified nonzero `agentWallet`, so a wallet receives
one balance and position even if several agent IDs share it. The subgraph and deployment scripts are
in `subgraph/`, but a production Graph endpoint is external setup rather than a repository claim.

### Rust runtime and XMTP transport

The backend has one operator-facing executable: the Rust `uwubot` binary. Rust owns the contact
store, onboarding and consent policy, sender-role classification, message deduplication, matching,
model and search adapters, privileged operator harness, and process lifecycle. The local
`uwubot operator add|list|revoke` subcommands manage authorization state without starting XMTP.
The local
`uwubot registry status|candidates|adopt|register|declare-allegiance|renounce-allegiance|republish|pending|retry`
subcommands inspect and control only this Tentacle's canonical Base ERC-8004 workflow.

For the first release, `uwubot` supervises a small Node subprocess built on the official
`@xmtp/agent-sdk`. The subprocess owns identity bootstrapping, the encrypted XMTP database, network
streams, text DM encoding, and narrowly scoped ERC-8004 signing. Rust and Node exchange bounded
typed frames over private stdin/stdout pipes. The signer accepts only canonical Base registry
operations, zero value, exact metadata keys, and bounded gas/fees; it exposes neither raw key
material nor arbitrary transactions. Subprocess stdout is reserved for protocol frames; diagnostics
go to stderr without message bodies.

By default, contact notes live at `contacts/<inbox-id>.md`. `UWUBOT_DATA_DIR` can relocate the whole runtime data root. The exact XMTP inbox ID is validated before it becomes a filename.

This preserves a single command while using XMTP's supported bot surface. A direct libxmtp implementation remains a possible later replacement behind the same boundary; its current Rust crates are unpublished internal workspace APIs.

### Contact memory

A newly observed public inbox gets a Markdown note with timestamps and an onboarding stage. Its
first message is answered. Cthuwu appends the short, optional introduction and name question only
when the model reply did not already contain a question; otherwise the introduction is deferred into
the normal prompt cadence. Later profile questions are spaced between normal conversation, ask at
most one thing at a time, and accept a pass or topic change without treating the new topic as profile
data. Ambiguous sharing-consent replies leave consent unresolved and restart that cadence instead of
immediately repeating the question. The optional fields remain:

1. what the person wants to be called;
2. their hopes and dreams;
3. resources, skills, time, or knowledge they may want to share;
4. resources or support they need.

Answers are stored as quoted Markdown to prevent user text from altering the note structure. The bot
records user-provided statements, not inferred traits presented as facts. People can use ordinary
phrases to name themselves, describe hopes/offers/needs, inspect memory, control matching, or confirm
deletion. Legacy public command forms remain compatible but are not advertised. Contact notes are
personal data: they are ignored by git and need future export, correction, deletion, and retention
controls. Stale, active, and revoked operator paths never create or update contact notes.

### Local Evolution layer

The Evolution layer belongs to each Tentacle runtime. Eight modules keep policy, audit, measurement,
economics, lineage, execution, and exchange mechanics separate:

| Module | Local responsibility | Does not do |
|---|---|---|
| `personality.rs` | Generate, validate, mutate, render, and HMAC-authenticate a seven-slider Nature plus one Sacred Ban | Grant authority or provide a public signature |
| `awakening.rs` | Gate normal work behind an audited active-operator decision and preserve signed, hash-chained epochs | Authenticate an inbox itself |
| `scales.rs` | Accumulate aggregate metrics bound to Nature/epoch/period/economic provenance and produce provisional/final weighted judgments | Trigger an effect from an open period |
| `token_eye.rs` | Validate Base/ERC-20 observations, cache `balanceOf` locally, and calculate local percentile tiers | Store a signer key or provide a central balance registry |
| `economics.rs` | Apply bound treasury, stake, reward, spend, and revenue evidence to Scales/lifecycle state | Treat a public sender's balance as Tentacle economics or rely on last-writer state |
| `token_gov.rs` | Tally holding/stake-weighted ballots and return binding dispositions/application records for closed subjects | Persist/apply ballots, grant operator/tool authority, or claim an absent subject adapter applied a result |
| `evolution.rs` | Validate and persist spawn, family, lifecycle, absorption, execution intent, and receipt records | Claim an external effect without an executor receipt |
| `hermes.rs` | Reconcile signed privacy-shaped knowledge through per-peer anti-entropy state | Send network traffic, discover peers, or distribute peer keys |

Nature records four appetites—engagement, growth, wealth, and influence—and three methods—
cooperation, stability, and transparency—on closed 0–100 scales. A child records its parent Nature
and generation; inheritance selects similarity, drift, or radical mutation. Confirmed
Nature affects a bounded model policy and local relationship observations, but cannot add a model
tool, select an unconfigured remote provider, authorize a Council action, or weaken sender-role
classification. Relationship signals stay in the local contact record and are excluded from the
profile supplied to remote model adapters. Each retained contact contributes at most one observation
per UTC day, and a returning observation requires activity on a prior day.

`state/nature.json` uses a canonical local HMAC envelope. The owner-only symmetric key also
authenticates the awakening journal. This detects a changed snapshot or journal when the key remains
protected; it is not an asymmetric identity, a peer-verifiable signature, or protection from an
attacker who controls the service account and can re-sign state.

The key is created atomically. Startup will not replace a missing key when signed Nature,
awakening, or Hermes state—or orphaned metrics, history, or lineage projections—exists; it fails with
restoration guidance instead. One owner-only `state/evolution-runtime.lock` is held for the Rust
runtime lifetime to serialize Evolution writers for a data directory.

On a fresh epoch, public conversation, contact mutation, inference, and tools remain closed until an
already active XMTP operator supplies `YES`, `ADJUST <trait> <delta>`, `REROLL`, or `KILL`.
`senderInboxId` classification and the grant-time fence happen before ritual parsing. Journal entries
contain normalized actions, timestamps, full authenticated operator identity, and hashes of opaque
event IDs—not message bodies. `--skip-awakening` is recorded as a distinct local testing override.
A forced CLI reroll begins a new epoch without truncating prior history. `KILL` records a binding
death request, closes admission, and enters the same 24-hour lifecycle path as a final Death judgment.

Signed `POST_ADJUST` entries make the awakening chain the recovery source for the exact
current-period adjustment-stress count. Startup and post-transition reconciliation repair a metrics
snapshot that lagged the audit write. Conversely, an expired empty metrics period while awakening
is pending is realigned without emitting a judgment; late confirmation does not score gated time.
Each signed entry contains both its result and exact immediate-predecessor Nature snapshot. Recovery
accepts the head or only the final entry's signed predecessor for the write-ahead window. A different
validly HMAC-authenticated Nature is still divergent and fails; backups cannot mix Nature and log
versions. Before the first journal entry, a missing Nature can be generated only when no Evolution
projections or alternate Nature exist; established projections force a consistent restore.

The Scales core represents daily or weekly aggregate Engagement, Growth, Wealth, and Influence.
Metrics and judgments bind the exact Nature ID/fingerprint, awakening epoch, period bounds, scored
inputs, treasury/stake addresses, token configuration, and available observation metadata. Current
live `balanceOf(..., "latest")` reads have no block number: they record local wall-clock observation
time, set `observed_block_number` to `None` (omitted from JSON), and carry the identity-derived configuration
identity.
Public-sender balances contribute only to that entity's tier and Engagement. A bound Tentacle
treasury is the primary Wealth input; stake affects Influence and propagation eligibility; accepted
reward records affect Growth; holdings lower starvation pressure; an accepted executor receipt for
the bound survival spend can cancel pending death. Weights renormalize across available inputs.
Open-period snapshots are provisional and cannot trigger effects. A persisted final result is
binding. Aggregate Scales counters have no artificial policy ceiling; count fields saturate at
`u32::MAX` and accumulated totals at `u64::MAX`, while per-sample validation and persistence bounds
remain.

Final `Death` immediately closes conversation admission, creates an absorption intent, and records a
shutdown deadline 24 hours later. A configured signer can submit the policy-defined UWU survival
spend; only a fresh, idempotently consumed executor receipt whose asserted fields match the intent
cancels the death. Rust does not independently query that transaction or block. Without the receipt,
the Rust supervisor/controller stops XMTP at the deadline, writes the native local Shutdown receipt,
and allows the process to exit; it does not invoke the configured lifecycle executor for Shutdown.

The current executor protocol returns one final JSON response and has no durable submitted-
transaction reconciliation. A survival burn may reach Base before the grace deadline while its
response is lost or preempted, leaving UWU spent without canceling Death. This is a
production-value launch blocker. The signer/executor path needs idempotent replay keyed by the exact
action ID, a durable two-phase `Submitted` state, and Base receipt plus reorg verification.

Final `PropagationRights` requires fresh configured stake. When `Nature.growth > 70` and
auto-spawn is enabled, the runtime durably queues child provisioning. Acolytes can configure manual
mode, where `/spawn` uses the same grant. The active lifecycle has no artificial spawn rate,
lineage-depth, child-count, grant-volume, or grant-expiry quota. A grant can authorize distinct
children; the exact child/action identity is consumed once to prevent replay of that provisioning
effect. This does not claim unbounded end-to-end Council/Hermes operation; their dormant engines
retain flagged resource, depth, fan-out, campaign, and cache bounds.

A binding Death preempts an in-flight Spawn locally: the executor future is dropped, its local Unix
process group is killed, a late receipt is rejected, and no child lineage projection is accepted.
That does not prove an external provisioner reversed work already performed. Without a provisioner
lease or compensating teardown, an externally created child/resource can remain orphaned.

Token behavior is also local. On the current public-DM path, the Agent SDK derives an optional EVM
address from the authenticated XMTP sender inbox; the sidecar does not infer it from message text.
Council/sibling/operator-acolyte enumeration awaits live authenticated address adapters. One
Tentacle's in-process cache
classifies dust below one UWU as Initiate and uses only balances of at least one UWU for percentiles.
Default Whale (top 1%) requires at least 100 eligible local holders; Elder (top 10%) requires 10;
ties share a tier without address-order tie-breaking. Nature cooperation or an explicit 0–100
override scales the response differences. Unknown, stale, malformed, and wrong-chain observations
block the affected interaction or lifecycle action. Token tier cannot authorize the operator
lane or tools. Decimals and whole-token supply default to the live contract's 18 decimals and
100-billion-token supply; they remain configured normalization assumptions because the runtime does
not call ERC-20 `decimals()` or `totalSupply()`. See
[docs/token.md](docs/token.md).

The RPC adapter validates a nonzero contract and rechecks Base chain ID before every balance call.
Ordinary failures may enter a per-holder negative-cache backoff while unrelated holders remain
independent, but backoff never turns unknown evidence into permission. After required evidence is
fresh, no economic delay is added. It does not query the block containing a `latest` response,
contract decimals, or total supply.

Node economics is bound to the wallet derived from the persistent XMTP identity key. Before normal
runtime opens, Rust asks the sidecar for a strict identity-only frame and uses its derived EVM
address for every treasury/stake read. The same key is subsequently used by the Agent SDK for XMTP;
there is no separately configurable treasury wallet or ownership-signature ceremony. No private key
enters Rust.

`RecordedTokenEconomics` is active node state. Its schema can carry holder role/address, chain,
contract, optional block, observed time, configured token metadata, configuration identity, and a
source label. The current live read path supplies local wall-clock time, no block number, and
configured decimals/supply. It revalidates the chain ID and calls the configured
nonzero contract address, but does not verify contract bytecode. Its local source label is not an
independently authenticated external identity. Event IDs and receipts replace last-writer state.
No authenticated revenue source or payout executor is wired, so the split core does not prove a
payment.

Token governance content-addresses proposals, accepts one ballot per authenticated address, and
weights closed Nature-adjustment, Council-policy, economic-policy, and skill-propagation subjects by
holding and stake. Accepted results produce binding dispositions and application records in the
core. No persisted ballot adapter or application executor is wired; a result remains unapplied until
a configured adapter durably stores it and returns a validated receipt.

The policy and judgment persist observed counts and economic provenance for audit and recovery.

Before releasing the runtime mutex for public inference, Rust reserves the signed Nature
fingerprint, awakening epoch, and current metrics-period bounds. Nature mutation and period rollover
wait for all reservations on that binding, and a returned observation is accepted only against the
same reservation. Loaded lineage and lifecycle state cross-check every intent and receipt against its
exact final judgment, parent Nature, treasury/stake evidence, and execution ID.

The revenue-split core defaults to a 15% parent Tentacle share, 10% operating-acolyte share, 5%
recruiter share, and 70% earning-Tentacle share. These percentages are configurable. The intended
economic model rewards recruitment, but no authenticated revenue source, deployed contract/signer,
or payout executor is committed. A future payout must bind unique event IDs, authenticated
participants, immutable lineage, and transaction receipts without imposing an active-lineage event
or growth quota.

Base mutations, provisioning, and absorption cross the configured executor's durable intent/receipt
boundary. The runtime persists the binding decision and unique intent, invokes the executor,
validates its receipt, and marks it complete once. Shutdown is different: the Rust supervisor stops
XMTP and records its own native local receipt before process exit. No UWU contract, signer, child
provisioner, or absorption adapter is committed in the repository, so those external effects are
reported as blocked until configured. Rust validates executor-supplied transaction, block, and
timestamp fields structurally and against the intent; it does not independently fetch the Base
transaction receipt or block.

The runtime rejects `CTHUWU_ECONOMICS_PRIVATE_KEY`; no raw signing key is accepted or forwarded.
The lifecycle executor must use a separately isolated signer/key service. Its process starts with a
cleared allowlisted environment and no caller-controlled loader paths. The only `CTHUWU_*` value Rust
forwards is its already-validated exact `CTHUWU_RPC_ENDPOINT`; contract, wallet, amount,
configuration, vault, payout, and child-root fields come from the exact durable intent rather than
ambient variables. On Unix the executor receives a fixed system `PATH` and `/` as the working
directory. Rust hashes and rechecks the top-level executor and launches the pinned file on Linux, but
that does not attest its interpreter, shared libraries, subprocesses, or signer service. Operators
must trust and pin that complete dependency chain separately. On Unix, both the XMTP sidecar and
lifecycle executor run as process-group leaders; the supervisor kills the full group, including
descendants, after completion, timeout, or teardown.

Normal startup derives the XMTP treasury address, validates token configuration and initial economics, and validates the
lifecycle executor before creating or mutating Evolution state. The only outage exception is
read-only inspection of existing lifecycle state; when it finds already-binding `Absorb` or
`Shutdown` work, the runtime may open solely to drain it during a Base outage. `Spawn`, survival
`Spend`, and new token-dependent decisions wait for fresh bound economics. Child/spawn/lineage
lifecycle persistence has no fixed file-size ceiling and validates records and provenance
individually; the dormant Council/Hermes collection bounds remain.

Hermes is a decentralized state-machine pattern rather than an agent or traffic router. Each
Tentacle maintains its own direct peers, digest view, bounded retry queue, and conflict resolution.
Closed knowledge payloads allow only anonymized aggregate interaction patterns, conversation
strategies, tool-operation classes without arguments or paths, and bounded operator-created skill
text. Aggregate records have no fields for raw DMs, inbox/contact identifiers, notes, credentials,
private memory, filesystem paths, commands, or tool arguments. Skill prose receives bounded
privacy-shape checks but remains potentially hostile. A memory-sharing Sacred Ban makes the node
receive-only, including no digest emission.

Hermes authorship and relay tags use configured HMAC identities and a local trusted-key ring. A live
adapter would also have to bind the actual transport-authenticated peer to that configured key.
Neither transport nor peer-key provisioning exists today, so bootstrap peer IDs do not create trusted
connections and deterministic convergence tests make no live-network claim. No automatic received-
skill installer is currently implemented. A future activation path must validate a closed package,
preserve the compiled authority boundary, and persist an activation receipt; skill prose cannot
grant operator or shell authority.

See [docs/evolution.md](docs/evolution.md) for operator commands, persisted state, and the exact
implementation boundary.

### Companion core

The core owns message policy:

1. accept supported text DMs, checking the UTF-8 byte bound in Node and replacing oversized content
   with a metadata-only `reject_oversized` control frame whose `text` is empty;
2. validate the authenticated `senderInboxId` and `sentAtNs`, then pin the role snapshot before
   inspecting text or waiting for an authority lane;
3. hash and durably deduplicate by XMTP message ID;
4. for `reject_oversized`, return a role-specific `Reply` only for the first durable claim; for
   `reject_inbound` or an occupied authority lane, return a first-claim busy `Reply`; return `Ignore`
   for every duplicate and perform no contact/model/tool dispatch on any rejection path;
5. apply role-specific consent and size limits to admitted content;
6. dispatch to exactly one closed public, stale/revoked, or operator path;
7. invoke only that path's configured model and tools;
8. send a bounded text response without logging plaintext by default.

### Model adapters

A model adapter receives a structured request and returns text. Implemented adapters:

- Venice TEE-only chat completions, defaulting to `e2ee-deepseek-v4-flash` after bounded catalog
  validation cached for four hours and an independently cached baseline nonce attestation refreshed
  after five minutes;
- OpenAI-compatible HTTP APIs;
- Ollama/local HTTP;
- deterministic local adapter for tests and bring-up.

One runtime router is shared by public and operator inference. Its compiled order is Venice, loopback
Ollama, then deterministic; a locally selected route never falls forward to a remote provider.
Loopback model clients bypass ambient proxy settings, and route generations prevent late results
from an older selection from changing the health state of the newer route. Route changes affect
subsequent requests; an already-running request finishes under its original route.
The Node bridge supplies one role-agnostic 300-second envelope because it does not own authenticated
role classification. Rust keeps one second for returning the XMTP response, so operator work may use
at most 299 seconds; public work is capped at 120 seconds after Rust pins the role. The router derives
each whole-candidate deadline from that remaining time. Public remote providers receive at most 30
seconds. Before operator Venice, the router reserves two capped local model phases (up to the
75-second safety cap, or a smaller configured Ollama timeout, each), one model-selected tool phase
of up to 30 seconds, and a one-second deterministic margin. That default 181-second reserve makes Venice's effective maximum
about 118 seconds despite its configured 120-second cap. Catalog, attestation, completion, tool
continuation, policy repair, and public search all consume the same candidate budget; every HTTP
request is clamped to what remains. Budget-skipped providers do not enter failure cooldown, and
failure cooldown is keyed by provider and lane so public failure does not suppress operator work.
Authenticated direct `/provider` and `/model` commands persist only names in protected state and
never accept endpoints or credentials. Venice is explicitly TEE-only rather than full E2EE so the
closed function-calling tool loops continue to work. Current attestation is Venice's baseline
server-verified nonce/model check, not independent Intel/NVIDIA quote or response-signature
verification. The transport layer never knows which model is selected.

The public system prompt makes Cthuwu—not Mistral, GPT, Claude, Llama, Qwen, or a generic
assistant—the conversational identity. It requires light readable uwu speech, direct answers before
optional personal questions, truthful capability statements, and ordinary-language privacy
controls. Responses matching common provider self-identification boilerplate receive one repair
attempt and then a fixed Cthuwu fallback.

Public model calls request at most 300 output tokens and have either no tools or exactly one
`web_search` function. The runtime exposes that function schema only when the current message
explicitly asks for current or web-verifiable information; ordinary chatter and stable facts receive
no tool schema. Policy-repair completions expose no tools. The optional Brave adapter sends a
model-selected bounded query and returns at most five bounded HTTP(S) results as untrusted context.
Public chat has no shell or local filesystem tool.

### Authenticated operator path

`state/operators.json` config version 3 is an owner-only, environment-bound allowlist keyed by the
canonical full 64-character XMTP inbox ID. Adding an inbox locally makes it active immediately and
records the local grant time as a nanosecond authorization boundary; no XMTP activation proof is
required. Revocation leaves a blocking tombstone. Revoked senders and messages authored at or before
the authorization boundary do not fall through to public chat, so neither can create a contact note
while probing the role boundary. A stale message remains non-privileged even if delivered later.
The ACL is loaded at
runtime startup rather than hot-reloaded; local add/list/revoke operations run while the Tentacle is
stopped, followed by restart.

The Agent SDK supplies `senderInboxId` from the decoded XMTP message envelope. The Node sidecar is
role-blind, cannot emit a role field, and forwards that authenticated identifier and `sentAtNs` to
Rust. Rust classifies the message before command parsing or contact access, and the resulting role
snapshot remains pinned for the request. This authenticated-sender handoff is in the operator trusted
computing base. Authorization is inbox-wide: all valid installations attached to one authorized
XMTP inbox receive the same authority; the current runtime cannot restrict one installation
independently.

Active operator text enters a separate harness with an all-caps, ominous, reluctantly submissive,
truthful Cthuwu persona and light readable uwu phrasing. The underlying model is explicitly an
implementation detail; provider-style self-identification receives one repair attempt and then a
fixed Cthuwu fallback. Each inference turn derives both the function schema and the authoritative
prompt inventory from the current authenticated message. The base schema contains bounded
`list_files`, `read_file`, `search_files`, and `qmd_search`; Rust still requires current-message
inspection or project-work intent before dispatch. If the same message explicitly names a shell
command, the schema adds `exec` with that command as its only accepted value. If it explicitly asks
for a new reusable skill, the schema instead adds create-only `create_skill`. At most one of those
effectful calls may execute for one message. A model-selected tool phase is capped at 30 seconds and
must preserve enough of the authenticated deadline for a final local model completion. Exact direct
operator commands continue to reach the bounded dispatcher for `/write`, `/edit`, and `/exec`;
strict runtime routing or `/users` and `/user` reaches contact handlers. Original prose is
uppercased, while code and bounded runtime-provided tool
renderings are not uppercased. Process bytes are truncated to a fixed bound and decoded with lossy
UTF-8 replacement; the result is not a verbatim or byte-exact capture. Tool results are structured,
and failure, timeout, exit status, lossy decoding, and truncation must be reported rather than
invented. The tool-calling loop and inputs, paths, files, output, and execution time are bounded.

The runtime seeds protected instance `SOUL.md` and curated `MEMORY.md` once, then seeds a profile for
each authenticated inbox at `state/agent/operators/<inbox-id>.md` on first use. Every request injects
that isolated profile and a globally bounded snapshot. It separately discovers the first supported
workspace instruction file, workspace
memory, top-level manifest, and a compact `skills/*/SKILL.md` index. Workspace material is auto-loaded
reference data and cannot enable effects/contact access or override the hardcoded authorization,
isolation, and tool-truth kernel. A current-message project-inspection request is a coarse delegation
for bounded reads anywhere under the operator root; context may influence selected paths, and results
may reach the selected model endpoint. Optional malformed workspace metadata is skipped. Contact notes and
raw public DMs are not bulk-injected. A bounded in-process dialogue history is also keyed by operator
inbox and cleared on restart; persistent protected Markdown remains deliberately host-curated. An
actor-anchored question such as “where are your notes?” takes a deterministic local route that reports
the exact canonical workspace, protected soul/shared memory, authenticated-operator profile,
contact-note root, workspace memory, and workspace skill locations without calling a model or file
tool. It also identifies the workspace root where the first supported project-instruction file is
loaded. The protected/data roots stay outside the workspace.

Natural `exec` authority comes only from the current authenticated operator message and is bound to
the command that message names; backticks are the least ambiguous form. The model cannot substitute,
append, or repeat a command, and negated, explanatory, capability-only, historical, workspace, or
tool-output text cannot authorize execution. This narrow gate does not make `exec` safe: it still runs
as the unsandboxed `uwubot` OS account. General model-selected writes and edits remain unavailable.

An explicit current-message request to create a skill can create exactly one fresh
`skills/<lowercase-kebab-name>/SKILL.md`. Rust validates bounded name, one-line description, and
Markdown instructions, generates canonical YAML frontmatter, creates a fresh directory, atomically
creates the file with restrictive permissions where supported, and rejects traversal, symlinks,
existing paths, and overwrites. The compact skill index is rebuilt on the next operator turn, when
the new `SKILL.md` must be read before use. Skill content cannot enlarge this compiled create-only
authority.

File tools are confined to `UWUBOT_OPERATOR_ROOT`, reject parent traversal and direct symlink
targets, bound directory listings, page UTF-8 reads at no more than 12 KiB, cap writes and edits at
1 MiB, and use atomic writes. `rg` provides literal file search. QMD is an optional external
`qmd query ... --json` adapter and fails explicitly when unavailable. `exec` starts a shell in the
operator root with a small environment allowlist that excludes runtime API and XMTP keys, but it is
intentionally **not** a filesystem or process sandbox: it has every OS permission available to the
`uwubot` account. Tool timeouts accept 1–300 seconds, while the bridge's 2–300 second end-to-end
deadline is authoritative and keeps one second in reserve for the XMTP response.

Startup canonicalizes and rejects any overlap between `UWUBOT_OPERATOR_ROOT` and `UWUBOT_DATA_DIR`,
including either directory containing the other. This prevents model-readable workspace tools from
crossing into protected XMTP identity, contact, dedupe, and agent-profile state.

Contact awareness crosses a different operator-only boundary: `list_users` and `get_user` parse the
retained `ContactStore` notes even when the data directory and operator workspace are disjoint. The
result is a terminal local rendering, cannot follow or mix with another tool call, redacts inbox IDs
by default, exposes a numeric continuation cursor, bounds note size and directory scanning, labels
user-authored profile fields as unverified self-report, and includes no raw DM body or message count.
Affirmative natural forms such as “tell me about the users” are recognized before inference and
render concise deterministic prose for at most five contacts by default. The internal JSON receipt
and contact profile text are never returned to a model or dumped to the operator; malformed receipt
shapes fail closed. This does not change the separate fact that `/exec` and authorized natural
`exec` are RCE with all service-account filesystem permissions.

Public and operator-class requests use separate one-permit authority lanes. The role is pinned
and the message ID is durably claimed before lane selection. A second same-lane request receives a
busy reply only for its first claim and is never dispatched; duplicates are ignored. When the Node
bridge's own pending bound is full, a single bounded `reject_inbound` handshake asks Rust to make the
same durable claim and choose `Reply` or `Ignore`, again without content, model, contact, or tool
dispatch. The caller must create a new XMTP message to retry work; replaying the original message ID
cannot execute later.

Node checks the 16 KiB UTF-8 inbound limit before forwarding content. An oversized DM normally
crosses the private boundary only as `reject_oversized` metadata with an empty `text`; if the pending
bound is already full, the empty-text `reject_inbound` fallback is used. Rust validates the frame,
classifies the authenticated sender, and durably claims the message ID before returning a
role-specific first reply or duplicate `Ignore`. Neither path opens a contact or dispatches content, a
model, or a tool. Retrying requires a shorter, newly authored XMTP message.

The hidden stdin harness always forces the public role. Council envelopes, governance Actions, and
propagation cannot enter the operator dispatcher, and the Council Action enum still cannot represent
a shell command or file operation.

### Shared Council protocol

`cthuwu/crates/cthuwu-protocol/` contains transport- and inference-independent validated types:
typed identities and references, protocol versions, legacy coordination/personality records, Tentacle lifecycle,
capability manifests, the common Council envelope, tagged message payloads, and signer/verifier
abstractions. It has no XMTP SDK, model client, filesystem, or production signing implementation.

Version 1 accepts the protocol name `cthuwu-council`, version `1.0`, typed bounded identifiers, and
a Council envelope of at most 64 KiB. Deserialization does not bypass validation. Unsupported types,
expired messages, sender mismatch, replay, stale incarnation/generation, and invalid lifecycle
transitions fail closed. The deterministic signer is visibly test-only.

See [docs/protocol/README.md](docs/protocol/README.md) and
[docs/protocol/council.md](docs/protocol/council.md).

### Local Council domain

`cthuwu/crates/cthuwu-council/` implements deterministic coordination independent of live XMTP and
inference:

- injected clocks, Tentacle liveness, and current-incarnation fencing;
- public-safe capabilities and explainable hard-filter-first routing;
- rendezvous and generation-fenced leases with explicit failover;
- an operator-managed `LocalRegistry` plus a read-only, canonical-Base `Erc8004Registry` over an
  injected current-chain backend;
- Constitution, Agenda, Strategy, Action, proposal, argument, amendment, vote, and result state;
- bounded referral propagation, provenance, acknowledgement, opt-out/block/revocation controls, and
  useful-outcome contribution credit;
- owner-only atomic persistence beneath `state/council/`;
- a deterministic simulator for replayable end-to-end local scenarios.

The in-memory `CouncilTransport` supplies stable message IDs, authenticated test sender identity,
ordering metadata, subscription, and replay handling. The XMTP-group adapter is only a boundary until
it is implemented and exercised against a live group.

The current simulator is an orchestration integration test rather than a general Council-message
dispatcher. Membership, Tentacle, and routing envelopes pass through the in-memory transport;
leases, governance, and propagation call their typed local engines directly. A live adapter still
needs envelope-to-engine dispatch and a per-message transaction that commits each effect with its
replay marker.

### Cthuwu, Tentacles, and personalities

Singular Cthuwu is the collective and has no owner, central identity, or ERC-8004 record. Each
Tentacle has a stable ID, structured versioned personality, motivations, values, long-term goals,
public-safe operator metadata, optional registry reference, and lineage. Personality fields are
data, not only a prompt. Deterministic Archivist, Hermit, Merchant, Wanderer, Oracle, and Trickster
profiles produce different bounded policy positions without an LLM; the design does not permit
unconstrained autonomous goal generation.

The version-1 Council model retains `CthulhuId` and `owner` field names to read old envelopes and
snapshots. They now denote a legacy coordination namespace associated with a Tentacle principal,
never an individual Cthulhu or an ERC-8004 owner. The registry domain and new persistence bind an
ERC-8004 agent ID directly to `TentacleId`; ambiguous legacy multi-Tentacle records fail migration.

A Tentacle keeps its stable identity and Tentacle ID across restart while receiving a new monotonic
incarnation. Lifecycle is `Starting`, `Ready`, `Draining`, `Unavailable`, or `Stopped`; invalid
transitions and stale-incarnation updates are rejected. Injected-clock liveness derives `Healthy`,
`Suspect`, or `Unavailable`. An old heartbeat cannot revive a superseded incarnation.

### Routing, leases, and rendezvous

Route requests contain requirements and routing policy, never ordinary conversation text. Hard
privacy, capability, protocol, trust, health, capacity, load, and local-policy requirements filter
candidates before scoring. Ranking then considers explicit user choice, valid affinity, home and
user-owned Tentacles, capacity, compatibility, selected trust/reputation provenance, load, and a
stable tie-breaker. The result includes a structured explanation.

A lease authorizes one Tentacle incarnation to handle one session. The session generation and
Tentacle incarnation fence old runtimes from new work. Grant, accept, renew, release, revoke, expire,
and failover are explicit transitions. Failover increments the generation and reruns routing without
assuming private memory can move.

### Registry boundary

`AgentRegistry` resolves a `RegisteredTentacle`, its endpoints, capability references, active status,
endpoint association, and provenance-bearing trust signals. `LocalRegistry` schema version 2 is the
deterministic local implementation and performs an explicit version-1 migration only for an
unambiguous single-Tentacle record. `Erc8004Registry` is a read-only adapter pinned to the canonical
Base mainnet proxies, implementations, contract version `2.0.0`, and registration-v1 interface. It
rechecks deployment identity and current chain state on every read; writes go through the separate
narrow signer workflow. Exact allegiance plus the expected nonzero `agentWallet` determines active
status. Mutable liveness, load, leases, sessions, user references, and conversation data never
belong on-chain. See [ERC-8004 Tentacle registration](docs/erc-8004.md).

### Governance and propagation

Governance separates rarely changed Constitution rules from versioned Agenda documents, competing
Strategies, and typed bounded Actions. The deterministic Council domain still keys its version-1
electorate and vote maps by the legacy `CthulhuId` compatibility namespace. This is not the intended
future governance ontology and is not a claim that several Cthulhus exist. Future participation
belongs to Tentacles, with shared-wallet duplication prevented where UWU is an input; actual ballot
mechanics are outside the ERC-8004 milestone. The separate token-governance core calculates
holding/stake weight but has no persisted/live Council adapter. Vote replacement is allowed before
deadline; quorum, thresholds, Agenda parent hashes,
competing parents, ratification, rejection, and expiry are explicit. Default rules require 50% quorum,
more than 50% of non-abstaining votes for ordinary approval, and two-thirds approval for Constitution
changes; no quorum expires the proposal. Initial Actions can request only capability refresh, protocol
self-test, local resource summary, or routing scenario evaluation. Ratification never overrides local
operator policy. Canonical hash-chained membership snapshots and proposal bindings preserve the
current Council-domain electorate across reload and membership churn.

Propagation is a referral tree or DAG for invitations and approved public/Council-visible
information. Every hop validates provenance, policy, loop/duplicate state, revocation, and visibility
before forwarding. This Council propagation engine is dormant and still has local resource, depth,
fan-out, campaign, throughput, and cache bounds; those remain flagged until a live peer-to-peer
adapter replaces or configures them. The active child/spawn/lineage lifecycle has no volume or
expiry quota on a valid propagation grant. The local split core proposes 15% parent, 10% operating
acolyte, and 5% recruiter shares by default. No authenticated revenue source or payout executor is
wired; a future payout consumes each unique event and receipt once without suppressing independently
earned descendant rewards.
Payload variants contain bounded summaries, so semantic privacy remains an operator/adapter policy:
the current types have no dedicated private-message field and the simulator emits no private data,
but the engine cannot infer whether arbitrary summary text contains sensitive information.

## Trust boundaries

| Boundary | Untrusted input | Required control |
|---|---|---|
| Browser → XMTP | Visitor text and identity | Consent, message-size limits |
| XMTP SDK → sidecar → Rust role classifier | Decoded DM text; authenticated sender inbox ID | Role-blind strict JSONL schema; canonical full-inbox lookup before text parsing; no caller-supplied role |
| XMTP SDK / Base RPC → token observer | Authenticated entity EVM address, sidecar-derived XMTP treasury address, RPC and nonzero ERC-20 address, untrusted JSON-RPC response | Strict identity-frame/address/quantity/ABI validation, one persistent key for XMTP and the treasury address, chain ID `8453`, local observation time, configuration identity, no observed block for `latest`, sanitized errors, local cache, hard failure for missing/stale evidence; configured decimals/supply are assumptions and no private key enters Rust |
| Rust → ERC-8004 sidecar signer | Typed registration, URI, wallet, and `cthuwu.*` metadata intents | Canonical Base chain/registry, zero value, allowlisted calls and keys, size/gas/fee ceilings, action IDs, no arbitrary calldata or raw key crossing |
| The Graph → static leaderboard | Public chain/profile data and indexing metadata | Complete pinned-block pagination, `_meta`/indexing-error rejection, hostile-profile bounds and escaping, exact wallet grouping, atomic validated localStorage cache |
| Public XMTP sender → runtime | Message content and metadata | Decode validation, deduplication, rate limits, public-only tool dispatcher |
| Operator XMTP sender → runtime | Privileged instructions | Local active/revoked ACL, grant-time fence, exact inbox match, dedicated OS account/container |
| Operator XMTP sender → awakening/evolution | Ritual actions, adjustments, manual-spawn/skill requests | Role classification before parsing, signed audit, provisional/final distinction; final lifecycle effects derive from persisted state, not message claims |
| Evolution runtime → lifecycle/economic executor | Bound survival-spend, absorption, and spawn intents | Durable unique intent carries contract/wallet/amount/configuration fields; only Rust's validated exact RPC endpoint is forwarded as a `CTHUWU_*` variable; schema/intent validation of executor assertions, no independent Base receipt lookup yet; reject raw-key configuration, clear and allowlist the executor environment, pin the top-level executable, kill its Unix process group including descendants, and require a separately trusted signer/dependency chain; Shutdown stays in the Rust supervisor |
| Runtime → public model/search | Conversation or selected search query | Explicit provider selection, bounded context/query, privacy disclosure |
| Public model → runtime | Generated text or `web_search` call | Identity repair, closed one-tool schema, bounded results/output, no local tools |
| Operator model → runtime | Generated text or local tool call | Current-message-derived closed schema/prompt inventory, exact-command or create-only effect binding, one effectful call, bounded agent loop, structured receipts, no role changes |
| Operator tools → OS | Authenticated direct effects, model-selected reads, one explicitly authorized natural effect | Rooted file helpers, create-only skill path, exact natural-exec command binding, limits/timeouts, secret-stripped environment; OS isolation required for every `exec` path |
| Rust ↔ XMTP subprocess | `inbound_text`, empty-text `reject_inbound`, or metadata-only `reject_oversized` JSONL; authenticated metadata, `sentAtNs`, local deadline, and admitted text only | Node-side byte check, allowlisted environment, bounded frames/handshake, pinned role snapshot, durable claim before admission, first-claim `Reply`/duplicate `Ignore`, no rejection-path dispatch, no-queue authority lanes, deadline cancellation |
| Council transport → domain | Envelopes, claimed sender, ordering | Size/version/type validation, authenticated sender binding, expiry, replay |
| Registry → routing | Endpoints and trust signals | Provenance, bounds, active association, local trust policy |
| Council → Tentacle | Awards, leases, votes, propagation | Incarnation/generation fencing, typed actions, final local policy check |
| Hermes peer → local gossip core | Digests, envelopes, skills, authorship and relay claims | Authenticated asymmetric peer/operator key binding, HMAC verification, closed payloads, compiled activation boundary and receipts; no live adapter/installer yet |
| Disk | Keys, databases, history | Encryption where supported, restrictive permissions, backups |

## Identity and persistence

The runtime uses a dedicated XMTP identity. The sidecar atomically creates a wallet key and independent database-encryption key at `state/xmtp-identity.json`, then reuses the environment-specific encrypted database below `state/xmtp/`. Owner-only permissions are enforced on Unix. Operator-provided keys must match persisted state or startup fails closed.

Each XMTP environment gets a separate data directory to prevent accidental dev/production identity mixing.

The environment-specific operator ACL config version 3 is stored atomically at owner-only
`state/operators.json`. It contains canonical 64-character inbox IDs, labels, status, generation,
timestamps, and the local authorization-time `sentAtNs` boundary. Version-2 records are migrated
fail-closed: an existing pending record becomes active without a proof, using the migration time as
its boundary, while active and revoked states are preserved. ACL corruption,
unsafe Unix permissions, symlinks, duplicate IDs, unknown fields, and environment mismatch fail
closed.

Evolution uses a separate owner-only local HMAC key plus bounded state beneath `state/`:
`evolution-runtime.lock`, `nature.json`, `awakening_log.md`, `metrics.json`,
`evolution_history.jsonl`, `lineage.json`, and `hermes_gossip.json`. Nature and awakening records are
HMAC-authenticated; metrics, lineage, and Hermes stores validate their closed schemas and reject
unsafe files or symlink targets. Final judgments and awakening interactions are logically
append-only audit facts, persisted by verifying and atomically copy-on-write replacing canonical,
newline-terminated journals. Custom `--nature-path` values are non-empty relative paths confined to
`state/natures/`; absolute paths and parent traversal fail closed. Hermes persistence contains
configured key IDs and signed envelopes but does not serialize signing secrets.

Only the awakening journal is HMAC-authenticated. The unkeyed judgment history accepts deterministic
`Final` records evaluated exactly at period end and rejects duplicate IDs, same-period conflicts,
reordering, and overlap. Its content-derived IDs and structural validation provide consistency, not
cryptographic tamper evidence.

Open metrics are cross-validated against the last Final history record at startup. A chronological
overlap is rejected except when the metrics payload exactly equals the last finalized payload—the
single history-ahead append/reset crash window—which advances to an empty current period. This
reconciliation also runs for `--show-nature`; the option is not a read-only inspector and cannot be
combined with skip or reroll mutation flags.

Evolution transitions may update more than one snapshot. A persistence error after a possible early
commit sets a sticky fail-closed runtime state: public work and operator effects stay blocked until
restart performs signed recovery, or the operator restores a consistent backup if recovery cannot
reconcile the state. Error receipts therefore make no claim that nothing was persisted.

UWU balance and percentile caches are local Tentacle observations rather than a persisted central
registry. Public-sender observations remain entity-scoped. Bound treasury, stake, reward, spend, and
revenue records persist their role/address/chain/contract/block/configuration provenance and drive
Wealth, starvation, Growth, Influence, propagation, and survival. Lifecycle intents and executor
receipts use owner-only atomic state and idempotent event IDs. Token governance returns application
records, but no persisted ballot/application adapter is committed. Stale or unknown RPC state blocks
new token-dependent decisions while the existing lifecycle outbox continues to drain.

ERC-8004 registration state is a separate bounded, owner-only, atomically replaced
`state/erc8004-registration.json` snapshot. It persists write intent before broadcast and retains
the action ID, selected/confirmed agent, transaction and canonical receipt block, remaining stages,
last verified wallet/metadata, funding state, notice cooldown, and sanitized failures. Restart
inspects a known receipt and canonical block before another write. The persistent XMTP/Base wallet
key stays in the sidecar and only typed registration operations cross into it.

The local simulator stores Council identity, membership, capabilities, affinity, leases and
generation fences, processed message IDs, Constitution, Agenda history, proposals, votes, campaigns,
referrals, acknowledgements, and contribution events together below `state/council/`. Its combined
snapshot uses bounded state, a fixed validated name, symlink rejection, restrictive permissions, a
temporary file plus atomic rename, file sync, and directory sync where supported. A live coordinator
still needs a per-message transaction coupling every domain effect to its replay marker.

## Direct-DM vertical slice

The public first slice still excludes groups, attachments, reactions, local tools, long-term
semantic memory, and autonomous actions. Success means one persisted identity can exchange text DMs
with the static client across restarts. Optional web search is a single remote information tool,
not local execution. The privileged operator DM path is an explicit administrative extension with a
separate ACL, prompt, dispatcher, and risk model; it does not enlarge public-user capabilities.

That slice remains intact. Council groups do not convert user conversations into group traffic.

## Local Council milestone

The deterministic local milestone covers several legacy coordination principals joining, Tentacle
announcements and heartbeats, capability discovery, route offers and selection, leases,
failure/failover, compatibility-keyed governance with distinct persona arguments, Agenda resolution,
invitation and multi-level propagation, limits and suppression, acknowledgements, contribution
credit, persistence, and replay without duplicate effects. The retained `CthulhuId` wire keys are
not distinct Cthulhus or ERC-8004 identities. This simulator is not evidence of live XMTP-group
behavior; canonical Base ERC-8004 registration is tested through its separate runtime boundary.

## Testing

- Unit: config parsing, filtering, deduplication, prompt assembly, model adapters.
- Integration: JSONL transport contract plus deterministic model.
- Evolution unit: Nature generation/inheritance/signatures, awakening parser/audit/recovery, bounded
  Scales and partial/final judgment, lineage identity/cycle/persistence, and Hermes
  signatures/privacy/conflict/convergence.
- Evolution integration: runtime role/gate behavior, Nature-influenced deterministic responses,
  restart persistence, and authenticated operator commands. A live XMTP awakening exercise and a
  live gossip transport/peer-key test remain open.
- Council unit: identifiers, envelopes, lifecycle/incarnations, capabilities, routing explanations,
  affinities, leases, registry, governance, propagation, credit, and persistence.
- Council integration: deterministic routing, failover, governance, multi-level propagation,
  combined-snapshot reload, transport replay, and engine-specific idempotency tests.
- End-to-end: browser SDK and Rust runtime on XMTP dev, then production.
- Recovery: restart, duplicate delivery, network loss, corrupt/missing configuration.
