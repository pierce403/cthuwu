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
- **Implemented — local:** `cthuwu-protocol`, deterministic Council domain logic, in-memory
  transport, local registry, protected snapshot persistence, and simulator are verified by the
  deterministic workspace suite.
- **Experimental boundary:** XMTP Council-group and ERC-8004 adapters.
- **Planned:** live Council-group interoperability and a configured ERC-8004 deployment.

| Plane | Contains | Explicitly excludes |
|---|---|---|
| Registry / future ERC-8004 | Durable identity, public metadata, endpoint association, capability references, provenance-bearing trust signals | Heartbeats, load, sessions, user data |
| XMTP Council group | Discovery, routing, leases, governance, heartbeats, approved propagation | Normal DM contents, contact notes, private memory |
| Direct XMTP DMs | Private user-to-Tentacle conversation | Council-wide broadcast by default |
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
- sends text messages.

The low-friction identity is a randomly generated EOA stored in browser local storage. The client connects automatically on load and supports a passphrase-encrypted wallet export. It must be presented honestly: the current XMTP Browser SDK database is unencrypted, clearing site data loses the identity without an export, and an identity export is not a history backup.

The presentation layer uses one optimized local mascot asset, a purpose-built 1200×630 Open Graph
card, and CSS-only ambient animation. The
desktop interface places the companion beside a full-height conversation panel; narrow screens use a
compact companion header and viewport-aware chat. Motion has an explicit persisted pause control and
also honors `prefers-reduced-motion`; connection and privacy state remain available as text rather
than depending on animation or color.

### Rust runtime and XMTP transport

The backend has one operator-facing invocation: the Rust `uwubot` binary. Rust owns the contact store, onboarding and consent policy, message deduplication, matching, model adapter, and process lifecycle.

For the first release, `uwubot` supervises a small Node subprocess built on the official `@xmtp/agent-sdk`. The subprocess owns only identity bootstrapping, the encrypted XMTP database, network streams, and text DM encoding. Rust and Node exchange bounded JSONL frames over private stdin/stdout pipes. Subprocess stdout is reserved for protocol frames; diagnostics go to stderr without message bodies.

By default, contact notes live at `contacts/<inbox-id>.md`. `UWUBOT_DATA_DIR` can relocate the whole runtime data root. The exact XMTP inbox ID is validated before it becomes a filename.

This preserves a single command while using XMTP's supported bot surface. A direct libxmtp implementation remains a possible later replacement behind the same boundary; its current Rust crates are unpublished internal workspace APIs.

### Contact memory

A newly observed inbox gets a Markdown note with timestamps and an onboarding stage. Cthuwu asks, in order:

1. what the person wants to be called;
2. their hopes and dreams;
3. resources, skills, time, or knowledge they may want to share;
4. resources or support they need.

Answers are stored as quoted Markdown to prevent user text from altering the note structure. The bot records user-provided statements, not inferred traits presented as facts. Contact notes are personal data: they are ignored by git and need future export, correction, deletion, and retention controls.

### Companion core

The core owns message policy:

1. accept supported text DMs;
2. hash and durably deduplicate by XMTP message ID;
3. apply consent, size, and concurrency limits;
4. construct bounded conversation context;
5. invoke the configured model adapter;
6. send a text response;
7. record completion without logging plaintext by default.

### Model adapters

A model adapter receives a structured request and returns text. Implemented adapters:

- OpenAI-compatible HTTP APIs;
- Ollama/local HTTP;
- deterministic local adapter for tests and bring-up.

The transport layer never knows which model is selected.

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
| XMTP → runtime | Message content and metadata | Decode validation, deduplication, rate limits |
| Runtime → model | Conversation content | Explicit provider selection, bounded context |
| Model → runtime | Generated text/tool requests | Output limit; no implicit tool execution |
| Rust → XMTP subprocess | JSONL metadata and text | Allowlisted environment, bounded queue and frames |
| Council transport → domain | Envelopes, claimed sender, ordering | Size/version/type validation, authenticated sender binding, expiry, replay |
| Registry → routing | Endpoints and trust signals | Provenance, bounds, active association, local trust policy |
| Council → Tentacle | Awards, leases, votes, propagation | Incarnation/generation fencing, typed actions, final local policy check |
| Disk | Keys, databases, history | Encryption where supported, restrictive permissions, backups |

## Identity and persistence

The runtime uses a dedicated XMTP identity. The sidecar atomically creates a wallet key and independent database-encryption key at `state/xmtp-identity.json`, then reuses the environment-specific encrypted database below `state/xmtp/`. Owner-only permissions are enforced on Unix. Operator-provided keys must match persisted state or startup fails closed.

Each XMTP environment gets a separate data directory to prevent accidental dev/production identity mixing.

The local simulator stores Council identity, membership, capabilities, affinity, leases and
generation fences, processed message IDs, Constitution, Agenda history, proposals, votes, campaigns,
referrals, acknowledgements, and contribution events together below `state/council/`. Its combined
snapshot uses bounded state, a fixed safe name, symlink rejection, restrictive permissions, a
temporary file plus atomic rename, file sync, and directory sync where supported. A live coordinator
still needs a per-message transaction coupling every domain effect to its replay marker.

## Direct-DM vertical slice

The first slice deliberately excludes groups, attachments, reactions, tools, long-term semantic memory, and autonomous actions. Success means one persisted identity can exchange text DMs with the static client across restarts.

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
