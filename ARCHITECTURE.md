# Architecture

## Goal

Cthuwu is a local-first companion process that receives and sends XMTP messages. A small browser application gives visitors a friendly way to open a DM without requiring an application server.

The Council of Cthulhus extends that companion into an optional federation. A durable **Cthulhu**
identity may operate one or more **Tentacle** runtimes and participate in a **Council** coordination
group. Standalone `uwubot` remains the default, and the already working browser-to-runtime DM path
does not depend on Council membership.

## Status and planes

- **Implemented — existing:** static browser, direct XMTP DMs, persistent identities, `uwubot`,
  contact memory, model adapters, deduplication, launcher protections, and the Agent SDK sidecar.
- **Implemented — local:** public Cthuwu identity enforcement, casual onboarding, optional Brave
  search, and an XMTP-inbox operator ACL with an isolated privileged harness. The role boundary and
  tools are covered locally; live operator use over XMTP is not yet a separate release claim.
- **Implemented — local:** `cthuwu-protocol`, deterministic Council domain logic, in-memory
  transport, local registry, protected snapshot persistence, and simulator are verified by the
  deterministic workspace suite.
- **Experimental boundary:** XMTP Council-group and ERC-8004 adapters.
- **Planned:** live Council-group interoperability and a configured ERC-8004 deployment.

| Plane | Contains | Explicitly excludes |
|---|---|---|
| Registry / future ERC-8004 | Durable identity, public metadata, endpoint association, capability references, provenance-bearing trust signals | Heartbeats, load, sessions, user data |
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
- publishes a standalone manifest and platform-specific install icons;
- registers a narrow service worker that falls back to a static offline page without caching XMTP
  transport, Browser SDK/WASM, messages, identity records, or arbitrary same-origin responses.

The low-friction identity is a randomly generated EOA stored in browser local storage. The client connects automatically on load and supports a passphrase-encrypted wallet export. It must be presented honestly: the current XMTP Browser SDK database is unencrypted, clearing site data loses the identity without an export, and an identity export is not a history backup.

The presentation layer uses one optimized local mascot asset, a purpose-built 1200×630 Open Graph
card, and CSS-only ambient animation. The
desktop interface places the companion beside a full-height conversation panel; narrow screens use a
compact companion header and viewport-aware chat. Motion has an explicit persisted pause control and
also honors `prefers-reduced-motion`; connection and privacy state remain available as text rather
than depending on animation or color. A dismissible install card uses Chromium's native
`beforeinstallprompt` event. Safari receives manual, backup-first instructions because Apple's
installed web-app storage is separate from the browser's local identity storage.

### Rust runtime and XMTP transport

The backend has one operator-facing executable: the Rust `uwubot` binary. Rust owns the contact
store, onboarding and consent policy, sender-role classification, message deduplication, matching,
model and search adapters, privileged operator harness, and process lifecycle. The local
`uwubot operator add|list|revoke` subcommands manage authorization state without starting XMTP.

For the first release, `uwubot` supervises a small Node subprocess built on the official `@xmtp/agent-sdk`. The subprocess owns only identity bootstrapping, the encrypted XMTP database, network streams, and text DM encoding. Rust and Node exchange bounded JSONL frames over private stdin/stdout pipes. Subprocess stdout is reserved for protocol frames; diagnostics go to stderr without message bodies.

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
typed identities and references, protocol versions, Cthulhu/personality records, Tentacle lifecycle,
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
- an operator-managed `LocalRegistry` and an unavailable ERC-8004 adapter stub;
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

### Cthulhus, Tentacles, and personalities

A Cthulhu has a stable ID, display name, structured versioned personality, motivations, values,
long-term goals, public-safe operator metadata, optional registry reference, and stable Tentacle IDs.
Personality fields are data, not only a prompt. Deterministic Archivist, Hermit, Merchant, Wanderer,
Oracle, and Trickster profiles produce different bounded policy positions without an LLM; the design
does not permit unconstrained autonomous goal generation.

A Tentacle keeps its stable owner and Tentacle ID across restart while receiving a new monotonic
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

`AgentRegistry` resolves identity, endpoints, capability references, active status, endpoint
association, and provenance-bearing trust signals. `LocalRegistry` is the deterministic local
implementation. `Erc8004Registry` is an isolated unavailable stub: domain types deliberately do not
hardcode a chain, deployment, ABI, or ERC-8004 draft revision. Mutable liveness, load, leases,
sessions, user references, and conversation data never belong on-chain.

### Governance and propagation

Governance separates rarely changed Constitution rules from versioned Agenda documents, competing
Strategies, and typed bounded Actions. One Cthulhu has one vote even if it operates several
Tentacles. Vote replacement is allowed before deadline; quorum, thresholds, Agenda parent hashes,
competing parents, ratification, rejection, and expiry are explicit. Default rules require 50% quorum,
more than 50% of non-abstaining votes for ordinary approval, and two-thirds approval for Constitution
changes; no quorum expires the proposal. Initial Actions can request only capability refresh, protocol
self-test, local resource summary, or routing scenario evaluation. Ratification never overrides local
operator policy. Canonical hash-chained membership snapshots and proposal bindings preserve the
historical one-Cthulhu-one-vote electorate across reload and membership churn.

Propagation is a bounded referral tree or DAG for invitations and approved public/Council-visible
information. Every hop validates provenance, policy, expiry, loop/duplicate state, depth, fan-out,
rate, opt-out, block list, revocation, and visibility before forwarding. Contribution credit is
non-financial and outcome-based; raw recruitment count earns nothing. Current credit is direct-only:
the useful contributor needs a recipient acknowledgement, each acknowledgement is consumed once, and
per-outcome/contributor/campaign caps prevent duplicate or descendant credit and limit simple Sybil
amplification. They do not establish identity uniqueness or solve Sybil resistance generally.
Payload variants contain bounded summaries, so semantic privacy remains an operator/adapter policy:
the current types have no dedicated private-message field and the simulator emits no private data,
but the engine cannot infer whether arbitrary summary text contains sensitive information.

## Trust boundaries

| Boundary | Untrusted input | Required control |
|---|---|---|
| Browser → XMTP | Visitor text and identity | Consent, message-size limits |
| XMTP SDK → sidecar → Rust role classifier | Decoded DM text; authenticated sender inbox ID | Role-blind strict JSONL schema; canonical full-inbox lookup before text parsing; no caller-supplied role |
| Public XMTP sender → runtime | Message content and metadata | Decode validation, deduplication, rate limits, public-only tool dispatcher |
| Operator XMTP sender → runtime | Privileged instructions | Local active/revoked ACL, grant-time fence, exact inbox match, dedicated OS account/container |
| Runtime → public model/search | Conversation or selected search query | Explicit provider selection, bounded context/query, privacy disclosure |
| Public model → runtime | Generated text or `web_search` call | Identity repair, closed one-tool schema, bounded results/output, no local tools |
| Operator model → runtime | Generated text or local tool call | Current-message-derived closed schema/prompt inventory, exact-command or create-only effect binding, one effectful call, bounded agent loop, structured receipts, no role changes |
| Operator tools → OS | Authenticated direct effects, model-selected reads, one explicitly authorized natural effect | Rooted file helpers, create-only skill path, exact natural-exec command binding, limits/timeouts, secret-stripped environment; OS isolation required for every `exec` path |
| Rust ↔ XMTP subprocess | `inbound_text`, empty-text `reject_inbound`, or metadata-only `reject_oversized` JSONL; authenticated metadata, `sentAtNs`, local deadline, and admitted text only | Node-side byte check, allowlisted environment, bounded frames/handshake, pinned role snapshot, durable claim before admission, first-claim `Reply`/duplicate `Ignore`, no rejection-path dispatch, no-queue authority lanes, deadline cancellation |
| Council transport → domain | Envelopes, claimed sender, ordering | Size/version/type validation, authenticated sender binding, expiry, replay |
| Registry → routing | Endpoints and trust signals | Provenance, bounds, active association, local trust policy |
| Council → Tentacle | Awards, leases, votes, propagation | Incarnation/generation fencing, typed actions, final local policy check |
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

The local simulator stores Council identity, membership, capabilities, affinity, leases and
generation fences, processed message IDs, Constitution, Agenda history, proposals, votes, campaigns,
referrals, acknowledgements, and contribution events together below `state/council/`. Its combined
snapshot uses bounded state, a fixed safe name, symlink rejection, restrictive permissions, a
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

The deterministic local milestone covers multiple Cthulhus joining, Tentacle announcements and
heartbeats, capability discovery, route offers and selection, leases, failure/failover, governance
with distinct persona arguments and one vote per Cthulhu, Agenda resolution, invitation and
multi-level propagation, limits and suppression, acknowledgements, contribution credit, persistence,
and replay without duplicate effects. It is not evidence of live XMTP-group or ERC-8004 behavior.

## Testing

- Unit: config parsing, filtering, deduplication, prompt assembly, model adapters.
- Integration: JSONL transport contract plus deterministic model.
- Council unit: identifiers, envelopes, lifecycle/incarnations, capabilities, routing explanations,
  affinities, leases, registry, governance, propagation, credit, and persistence.
- Council integration: deterministic routing, failover, governance, multi-level propagation,
  combined-snapshot reload, transport replay, and engine-specific idempotency tests.
- End-to-end: browser SDK and Rust runtime on XMTP dev, then production.
- Recovery: restart, duplicate delivery, network loss, corrupt/missing configuration.
