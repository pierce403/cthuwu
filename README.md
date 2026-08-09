# cthuwu

A tiny eldritch companion people can message over [XMTP](https://xmtp.org).

Cthuwu is growing into the **Council of Cthulhus**, an optional federation of durable agent
identities and their local runtimes. A **Cthulhu** is the durable identity, personality, memory, and
governance participant. A **Tentacle** is one running runtime belonging to that Cthulhu. A
**Council** is an XMTP coordination group. The Council is a control plane only: direct user
conversations remain private one-to-one XMTP DMs.

Cthuwu has two user-facing pieces:

- `web/`: a static browser client that creates a dedicated local identity and opens a one-to-one DM;
- `cthuwu/`: the single Rust command, `uwubot`, which owns contact memory, consent, matching policy, and model access.

`uwubot` supervises the supported `@xmtp/agent-sdk` transport in `agent/`. Node is an internal transport detail: the operator still starts and stops one command. Direct libxmtp crates are not currently a stable, published Rust integration surface.

The detailed product contract and remaining release gates live in [FEATURES.md](FEATURES.md).
The Council protocol is documented in [docs/protocol/README.md](docs/protocol/README.md).
The local Evolution layer and its current network boundary are documented in
[docs/evolution.md](docs/evolution.md).
The UWU launch choices, local ERC-20 observer, and post-launch activation steps are documented in
[docs/token.md](docs/token.md). The broader policy limits found during this phase are tracked in
[docs/guardrail-audit.md](docs/guardrail-audit.md).

## Council implementation status

| Component | Status |
|---|---|
| Existing browser-to-`uwubot` direct DM path | **Implemented — existing**; remains the default |
| `cthuwu-protocol` validated, transport-independent types | **Implemented — local** |
| Deterministic Council domain components in `cthuwu-council` | **Implemented — local; verified by deterministic workspace tests** |
| In-memory Council transport, `LocalRegistry`, protected persistence, and simulator | **Implemented — local; verified by deterministic workspace tests** |
| XMTP Council-group adapter | **Experimental boundary**; no live group interoperability claim |
| ERC-8004 registry adapter | **Experimental stub / planned integration**; no chain, ABI, deployment, or draft revision selected |

Council mode is opt-in. An existing deployment with no Council configuration starts the same
standalone `uwubot`, uses the same direct-DM transport, and requires no registry or group.

## Evolution implementation status

| Component | Status |
|---|---|
| Tentacle Nature and awakening audit | **Implemented — local**; HMAC-authenticated state and operator-gated state machine |
| Scales and lineage | **Implemented — local**; bounded judgments and records, with no automatic process lifecycle effects |
| Hermes-inspired anti-entropy | **Implemented — local core and persistence**; no live transport or peer-key provisioning |
| Council metrics/lineage publication | **Not connected**; no live Council interoperability claim |
| UWU token observance and Engagement input | **Implemented — local/pre-launch**; read-only Base `balanceOf`, local tiers, and a period-averaged Engagement bonus; contract deployment still pending |
| UWU token-weighted governance core | **Implemented — local library only**; deterministic advisory ballots with no live Council or Nature-mutation adapter |

Evolution state is per standalone Tentacle and does not require a Council. Its local HMAC tags are
integrity checks under an owner-only symmetric key, not public signatures. A recorded child is not a
running process, and a death outcome is not a shutdown command. Received gossip skills remain inert
until an authenticated local operator reviews and activates them through the existing compiled skill
boundary.

## What works

- The browser generates an environment-scoped wallet before connecting, reuses it on reload, and supports passphrase-encrypted identity export/import and confirmed reset.
- The responsive web client uses a locally hosted animated Cthuwu mascot, loads and streams text
  history, preserves drafts after failed sends, and offers an explicit motion pause control. It is
  also installable as a standalone PWA with dedicated icons, a restrained install nudge, and an
  honest branded offline screen.
- `uwubot` creates a persistent XMTP wallet and encrypted database on first start, then reuses both.
- Cthuwu answers a new sender's first message, identifies itself as Cthuwu rather than the configured
  model, and uses light readable uwu speech. It appends the first optional profile question only when
  that model reply did not already ask one; otherwise all profile prompts are deferred into the
  casual conversation cadence.
- People use ordinary language to inspect, correct, pause, share, or delete their local contact note;
  public replies do not advertise command syntax.
- Inbound message IDs are durably deduplicated; storage, model context, bridge concurrency, and message sizes are bounded.
- The default inference preference is Venice's TEE-backed `e2ee-deepseek-v4-flash` when a Venice
  credential is configured, with loopback Ollama and then deterministic local behavior as automatic
  fallbacks. Public and operator routes use different authenticated work budgets, and every provider
  attempt is capped from the remaining deadline after reserving time for local fallback. An optional
  Brave Search adapter gives the public model one closed `web_search` tool and no local file or
  process tools. Supplying a remote credential is the explicit opt-in that permits message content
  to leave the node.
- A node operator can authorize an exact XMTP inbox immediately with a local CLI command.
  Active operator DMs enter a separate all-caps operator harness with bounded file/search tools and
  intentionally privileged shell execution; public, stale, and revoked messages cannot reach it.
- Structured, versioned Cthulhu personalities include deterministic Archivist, Hermit, Merchant,
  Wanderer, Oracle, and Trickster personas with different local policy positions without an LLM.
- Each Tentacle also has a random, signed local Nature with seven bounded sliders and one Sacred Ban.
  The local runtime gates first awakening to the authenticated XMTP operator lane before normal
  work; a live ritual release exercise remains open. Confirmed Nature supplies bounded
  response/resource policy and local relationship signals. Those relationship values remain local
  and are omitted from profiles sent to remote models.
- The Scales core accumulates bounded aggregate metrics, distinguishes advisory open-period
  snapshots from final judgments, and requires an authenticated operator before any lifecycle
  effect. The runtime scores Engagement only. A fresh or unexpired cached public-sender UWU balance
  can add one bounded Engagement bonus, averaged across every conversation in the period so message
  order and last-writer state cannot grant an advantage. It does not represent Tentacle Wealth,
  starvation relief, stake, rewards, Influence, Growth, propagation rights, or lifecycle authority.
  Lineage persistence still records spawns and absorptions without launching, terminating, or
  routing processes.
- Each Tentacle can observe the transferable UWU ERC-20 independently through a read-only Base
  mainnet RPC. It derives Whale, Elder, Acolyte, Initiate, and Unproven tiers from its own cache and
  uses Nature cooperation to scale response differences. Percentile ranks consider only holdings of
  at least one UWU; default Whale and Elder tiers require meaningful local samples of 100 and 10
  eligible holders. Missing configuration, an unavailable RPC, or unknown/stale data degrades
  neutrally; UWU never grants the XMTP operator role.
- A deterministic token-governance library calculates bounded, Nature-scaled ballot weights for a
  closed set of advisory subjects. It is not connected to live Council proposals, Nature mutation,
  transaction signing, process control, or operator authorization.
- The Hermes-inspired core signs and reconciles privacy-shaped knowledge between directly trusted
  peers in deterministic tests. It persists anti-entropy state locally, but no live gossip transport,
  discovery handshake, or peer-key provisioning exists yet.
- The local Council implementation models validated envelopes, Tentacle lifecycle and liveness,
  capability discovery, explainable routing, generation-fenced leases, governance, bounded referral
  propagation, contribution credit, and persistence without introducing transport or inference
  dependencies into the protocol crate.

The existing direct-DM code paths, local tests, manual browser-to-bot XMTP `dev` release gate, and
deterministic Council workspace suite are working. A live end-to-end CI job remains before this
project claims production interoperability.

## Build and verify

Requirements: Node 22 or newer and Rust 1.97 or newer.

```bash
npm --prefix agent ci
npm --prefix agent run typecheck
npm --prefix agent test
npm --prefix agent run build
npm --prefix agent audit --audit-level=high

npm --prefix web ci
npm --prefix web run typecheck
npm --prefix web test
npm --prefix web run build
npm --prefix web audit --audit-level=high

cargo fmt --manifest-path cthuwu/Cargo.toml --all -- --check
cargo test --manifest-path cthuwu/Cargo.toml --workspace --locked
cargo clippy --manifest-path cthuwu/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo build --manifest-path cthuwu/Cargo.toml --release --locked
```

## Run `uwubot`

On Unix, macOS, or WSL, the root launcher handles the repeatable setup and then replaces itself with `uwubot`:

```bash
./uwu.sh
```

It verifies Node 22+, installs Rust 1.97 through an existing `rustup` when needed, installs the locked sidecar packages when the lockfile, Node major version, or host platform changes, and builds both the sidecar and release binary. Concurrent invocations serialize that setup so they cannot corrupt the shared build tree, and only one bot may use a data directory at a time. On the first normal launch, the sidecar atomically creates the wallet and database key; later launches reuse them. The launcher never reads or prints either key.

The default is XMTP `dev` with persistent owner-only state at `${XDG_DATA_HOME:-$HOME/.local/share}/cthuwu/dev`. Each environment gets a different directory. Override either setting through the environment or the corresponding normal `uwubot` option:

```bash
UWUBOT_XMTP_ENV=production ./uwu.sh
./uwu.sh --data-dir /secure/path/cthuwu-dev --xmtp-env dev
```

The launcher must run as a dedicated unprivileged account. It accepts a new or empty data directory, or an existing Cthuwu directory for the selected environment; it rejects broad unrelated directories, paths overlapping the repository, and symlink redirection before changing permissions. It also rejects model credentials on the command line. Set credentials only in the environment. It strips model and identity secrets from dependency and compiler subprocesses; the final Rust process still enforces the narrower XMTP-sidecar environment allowlist. `XMTP_DB_DIRECTORY` is intentionally unsupported by this safe launcher so the database cannot escape the validated data root; transport developers can invoke the built binary directly when testing that low-level override.

The first successful connection logs the Tentacle's public Ethereum address without logging its
keys. The website always uses XMTP `production` and currently uses
`0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db` as its hard-coded intro Tentacle. Run that identity on
XMTP production:

```bash
UWUBOT_XMTP_ENV=production ./uwu.sh
```

Normal state is kept below `UWUBOT_DATA_DIR`:

```text
contacts/<inbox-id>.md
.uwubot.lock/<running-pid>
state/environment
state/agent/SOUL.md
state/agent/memories/MEMORY.md
state/agent/operators/<operator-inbox-id>.md
state/evolution-signing.key
state/evolution-runtime.lock
state/nature.json
state/natures/<custom-relative-path>
state/awakening_log.md
state/metrics.json
state/evolution_history.jsonl
state/lineage.json
state/hermes_gossip.json
state/inference.json
state/operators.json
state/processed/<hashed-message-id>
state/council/<bounded-state-name>.json
state/xmtp-identity.json
state/xmtp/<environment>/
```

The identity file contains a private key protected by owner-only filesystem permissions. Back up the
entire data directory securely. The launcher lock contains only a process ID and process-start
timestamp, and is recovered after a stopped process without confusing a reused PID for the old bot.
Rust separately holds the owner-only `state/evolution-runtime.lock` as the single writer for local
Evolution state. Do not delete only the XMTP database: doing so creates a new installation and can
eventually exhaust the inbox installation limit.

### Nature and first awakening

On a fresh data directory, `uwubot` creates a random Nature and waits for an active XMTP operator to
confirm it. While that epoch is pending, normal public conversation, contact mutation, model calls,
and tools remain gated. The operator replies with exactly one of these actions:

```text
YES
ADJUST <trait> <delta>
REROLL
KILL
```

`KILL` records a terminal request for the epoch but does not stop the OS process. The audit journal
records normalized actions and hashed XMTP event IDs rather than message bodies. Confirming or
skipping the gate does not authorize a Council or enable public/operator tools beyond their existing
role boundaries.

Useful local options are:

```bash
./uwu.sh --show-nature
./uwu.sh --nature-path experiments/nature.json
./uwu.sh --reroll-nature --force
./uwu.sh --skip-awakening
./uwu.sh --gossip-peers sibling-a,sibling-b
```

`--show-nature` runs normal Evolution startup reconciliation before printing, so it can initialize
missing state or finish a safe crash recovery. It is not a read-only file inspector and cannot be
combined with `--skip-awakening` or `--reroll-nature --force`.

Use `--skip-awakening` only for tests or deliberate local bring-up; it creates a signed audit event
that is visibly distinct from XMTP operator confirmation. A forced reroll creates a new immutable
awakening epoch. `--gossip-peers` supplies bootstrap identifiers only: without an out-of-band trusted
key binding and a live transport adapter, it does not connect to or authenticate those peers.
`--nature-path` accepts only a non-empty relative path below
`UWUBOT_DATA_DIR/state/natures/`; absolute paths and parent traversal are rejected. The default
snapshot remains `state/nature.json`.

Signed `POST_ADJUST` audit entries are the recovery source for the exact current-period stress
counter if a crash separates the journal and metrics writes. An expired empty metrics period while
awakening is pending is reset without a judgment, so late confirmation starts from the current
period rather than evaluating gated time.

Each signed awakening entry includes both its resulting Nature and the exact immediate-predecessor
Nature snapshot. Recovery accepts only the journal head or, for the final log-ahead crash window,
that signed predecessor. A different Nature is rejected even if its envelope validates under the
same key, so divergent Nature/log backups must be restored together rather than mixed. Before the
first action, a missing Nature is regenerated only when no Evolution projections or alternate
Nature exist; an established node with a lost Nature must restore a consistent backup.

For an offline contact-flow harness:

```bash
./uwu.sh --data-dir /tmp/cthuwu-harness --skip-awakening --stdin-inbox 012345abcdef
```

Each input line is the next message from that test inbox. The explicit testing override is required
because stdin cannot supply an authenticated operator awakening.

The hidden stdin harness is deliberately public-only. It cannot simulate an operator, even when its
inbox argument matches an active operator record.

### UWU token observation

UWU is planned as a transferable 18-decimal ERC-20 on Base mainnet (`8453`). No minimum token
balance or stake is required to start a Tentacle. Observation defaults on but waits harmlessly for a
contract address:

```bash
CTHUWU_RPC_ENDPOINT=https://mainnet.base.org \
CTHUWU_TOKEN_CONTRACT=<verified-UWU-contract-address> \
CTHUWU_TOKEN_DECIMALS=18 \
CTHUWU_TOKEN_TOTAL_SUPPLY=100000000000 \
./uwu.sh
```

The example is intentionally non-runnable; replace `<verified-UWU-contract-address>` with the
verified deployed nonzero contract. The
relevant options are `--rpc-endpoint`, `--token-contract`, `--observe-tokens`,
`--observe-interval`, `--min-tier`, `--token-tier-intensity`, `--token-decimals`, and
`--token-total-supply`; corresponding `CTHUWU_*` environment variables are listed in
[the token guide](docs/token.md).

The sidecar obtains an optional EVM address from the SDK-authenticated XMTP sender inbox. Rust checks
Base chain ID before every read and performs only `eth_call` `balanceOf(address)`. Each Tentacle
caches and ranks its own observations; there is no central holder registry. Failed refreshes use a
bounded per-holder 1–30 second retry backoff. Unknown and stale results do not gate or modify a
response and add no Engagement bonus. No RPC path accepts a private key or signs a transfer.

`--observe-tokens false` ignores stale token-only configuration. With observation enabled, malformed
and zero contract addresses fail startup. Public balances never activate the adapter-only
`RecordedTokenEconomics` APIs for Tentacle Wealth, starvation, stake, or rewards; a future adapter
needs cryptographically bound node/operator provenance before those can enter runtime state.

The requested one-billion UWU supply remains a launch choice, not a Clanker standard: current
Clanker v4 documentation specifies a fixed 100-billion-token supply with 18 decimals. Using one
billion therefore requires a reviewed custom/nonstandard deployment, or the launch must adopt the
current Clanker standard. Standard Clanker creator fees are LP/swap rewards rather than an ERC-20
fee-on-transfer.

## XMTP operator role

> [!DANGER]
> An active operator inbox has remote code execution as the OS account running `uwubot`. Use a
> dedicated unprivileged service account or container, choose a narrow `UWUBOT_OPERATOR_ROOT`, keep
> that account away from unrelated credentials and data, and secure every XMTP installation attached
> to the authorized inbox. See [the operator guide](docs/operator.md) before enabling this feature.

Operator authorization is keyed by the canonical full 64-character XMTP inbox ID, not a wallet
address, prefix, display name, or message claim. Stop the Tentacle, then authorize the inbox
locally in the same data directory and XMTP environment it uses:

```bash
./uwu.sh --xmtp-env production operator add <full-inbox-id> --label Dean
```

The command writes an active ACL record and exits; there is no XMTP activation proof to copy. Restart
the Tentacle and newly authored messages from that inbox enter the operator harness. ACL management
is not hot-reloaded: stop the Tentacle before adding, listing, or revoking, then restart after a
change.

On first start, Cthuwu seeds protected instance Markdown for its identity and curated shared memory;
it seeds a separate profile for each authenticated operator inbox on first use and never overwrites
later edits. Each operator request also loads a globally bounded project snapshot from the operator
workspace: the first supported instruction file, `MEMORY.md`, the top-level manifest, and a compact
`skills/*/SKILL.md` index. Workspace context is untrusted reference data and cannot enable effectful
or contact tools or alter Rust authorization. An operator request to inspect or work on the project
delegates bounded reads within the entire configured workspace; context may influence the paths chosen,
and file/QMD results may be sent to the selected model endpoint. Keep credentials and unrelated secrets
outside `UWUBOT_OPERATOR_ROOT`. Authentication, isolation, and tool-truth rules remain an immutable
security kernel rather than editable Markdown.

Ask Cthuwu “where are your notes?” to receive an exact local report of the active workspace,
protected soul/shared memory, current operator profile, retained-contact root, workspace memory,
project-instruction root, and skill locations. That route calls neither a model nor a file tool.

Natural-language dialogue history is bounded in memory and isolated per operator inbox; it does not
silently become a persistent transcript.

```bash
./uwu.sh --xmtp-env production operator list
./uwu.sh --xmtp-env production operator revoke <full-inbox-id>
```

Messages authored at or before the local authorization boundary, and every message from a revoked
inbox, never fall through to public chat and never create contact notes. The
sidecar does not accept a role field: Rust classifies the Agent SDK's authenticated
`senderInboxId` before interpreting message text or dispatching commands, pins that role to the
request, and rejects operator messages authored at or before the local grant. Because an XMTP inbox
can have multiple installations, every installation authorized for that inbox receives the same operator
authority. Revoke the Cthuwu role and the compromised XMTP installation immediately if any device or
installation key may be lost: stop the node first, persist the local revocation, then restart it.

Once active, the operator may use direct `/exec`, `/files`, `/read`, `/write`, `/edit`, `/search`,
`/qmd`, `/provider`, `/model`, `/users`, and `/user` commands. Evolution adds `/nature`,
`/adjust <trait> <value>`, `/lineage`, `/metrics`, `/judgment`, `/spawn [child-id]`,
`/gossip-status`, `/share-skill <name>`, and `/request-skill <name>`. During awakening, the ritual's
bare `ADJUST <trait> <delta>` is a relative change; after confirmation, `/adjust` sets an absolute
bounded value and adds a visible stress event. `/provider` and `/model` change the
persisted node-wide inference route without accepting a URL or credential over XMTP, and route changes
clear bounded in-process operator dialogue history. Each ordinary-language turn receives an exact
prompt inventory built from its closed schema. Bounded file/discovery/search tools form the base. A
current authenticated message that explicitly names a command can activate one natural `exec` call
bound to exactly that command—prefer backticks, as in “please run `cargo test`.” It remains
unsandboxed RCE as the `uwubot` account. An explicit request for a new reusable skill can activate one
create-only write to `skills/<lowercase-kebab-name>/SKILL.md`; canonical frontmatter is generated,
existing paths and overwrites are refused, and the skill is indexed on the next turn. General model
writes and edits remain unavailable, so use direct `/write` and `/edit`. The safe launcher defaults
`UWUBOT_OPERATOR_ROOT` to the repository root;
set it explicitly for a narrower production workspace. QMD is an optional external adapter; set
`UWUBOT_QMD` to a compatible executable that supports `qmd query <query> --json`. Public users are
not shown this syntax and cannot invoke these tools.

`/judgment` returns an advisory snapshot until the current daily/weekly period closes; a partial
snapshot can never grant a right. Persisted metrics and final judgments bind the signed Nature ID and
fingerprint, awakening epoch, period bounds, and scored-scale availability. The runtime accepts at
most one engagement observation per contact per UTC day, and counts a return only after prior-day
activity. Public inference reserves its Nature fingerprint, awakening epoch, and metrics period, so
mutation and rollover wait until all remote work using that binding finishes.

`JudgmentPolicy` and each `Judgment` persist the propagation evidence floors and observed counts.
Daily policy requires eight observations and four prior-day returns; weekly policy requires 32 and
16. A high score with a smaller sample is capped at `Survival`, never `PropagationRights`.

A final result still requires authenticated operator confirmation; it never shuts down the process.
`/spawn` additionally requires a final propagation-rights judgment from the exact current scoring
policy, Nature ID/fingerprint, and awakening epoch, plus at least eight daily contact observations
and four prior-day returns. It records authenticated operator and hashed event provenance and
consumes the judgment's content-derived ID exactly once. The resulting child Nature and lineage
record does not provision an XMTP identity, start another `uwubot`, or prove a live child exists.
The right expires at the end of the immediately following metrics period; missed periods invalidate
it rather than extending or reviving it.
On startup, every stored spawn receipt must resolve to its exact `Final` `PropagationRights` history
record, match the recorded parent Nature, and have a timestamp inside that immediately following
period. A well-formed but unverifiable lineage file fails closed.
`/share-skill` stages bounded operator-authored text in the local Hermes state. `/request-skill` can
inspect only knowledge already present locally until a live peer adapter exists. Gossiped skill text
is untrusted and is never installed, executed, or exposed as a model tool automatically.

The signed awakening journal and unkeyed final-judgment history are logically append-only and use
canonical, newline-terminated atomic copy-on-write replacements. Judgment history accepts only
deterministic final records evaluated exactly at period end and rejects duplicate IDs, conflicting
same-period records, reordering, and overlap. Those are consistency checks, not cryptographic tamper
evidence. The Evolution signing key is created atomically; if it is missing while signed state
or metrics/history/lineage projections exist, startup fails without silently rekeying or adopting
the orphaned projections. Startup also cross-validates open metrics against Final history. The only
accepted overlap is exact equality with the last finalized metrics payload—the history-ahead crash
window where append committed before reset—which is replayed into an empty current period. Any other
overlap fails closed. If a multi-snapshot transition reports a
persistence error after a possible partial commit, Evolution remains sticky fail-closed until
restart performs signed recovery (or a consistent backup is restored); the error response does not
promise that nothing was written.

`list_users` and `get_user` read parsed, retained `ContactStore` notes through a dedicated operator-only
boundary. They do not widen generic file-tool access to the data directory. User reports are terminal
renderings with redacted inbox fingerprints by default, cursor pagination, bounded scans, and honest
truncation markers. They mark profile claims as unverified self-report and disclose neither raw DMs
nor message counts. The reported set is retained contacts, not everyone who may ever have sent the
Tentacle a message. Affirmative natural questions such as “tell me about the users” bypass inference
and render concise deterministic prose with a default limit of five contacts; internal JSON and
profile text are never sent to a model or dumped as the reply. This terminal-data guarantee applies to the
dedicated contact tools; every `exec` route remains deliberate RCE and can read anything the service
account itself can access.

Startup rejects canonical overlap between `UWUBOT_OPERATOR_ROOT` and `UWUBOT_DATA_DIR`, including
either directory containing the other. The safe launcher and container defaults keep them separate.

Public and privileged work use separate single-request authority lanes. Rust pins the role and
durably claims the XMTP message ID before admission. If a lane or the Node bridge is full, the first
claim receives a busy reply without content/model/tool dispatch; duplicate delivery receives no
reply. Node also checks the 16 KiB UTF-8 input bound before sending content to Rust. An oversized DM
uses a metadata-only rejection control frame with an empty `text`; after classification and durable
claim, Rust returns a role-specific first reply or ignores a duplicate, without contact, model, or
tool dispatch. These tombstones mean retrying the same network message can never execute later—the
client must send a new XMTP message after capacity returns, and shorten an oversized message. The
bridge's configured 2–300 second end-to-end deadline cancels work while reserving one second to
reply. See the operator guide for the distinct 12 KiB UTF-8 read page and 1 MiB write/edit limits.

## Council architecture

The four planes remain deliberately separate:

| Plane | Responsibility |
|---|---|
| Registry / future ERC-8004 | Durable public identity, public metadata, endpoint associations, provenance-bearing trust signals |
| XMTP Council group | Discovery, routing metadata, leases, governance, heartbeats, and approved propagation |
| Direct XMTP DMs | Private user conversations with the selected Tentacle |
| Tentacle runtime | Inference, contact memory, tools, capacity, and final local policy enforcement |

Current Council types provide no dedicated field or runtime path for normal user messages, contact
notes, credentials, or private memory. Operator and adapter policy forbids placing that data in
bounded free-text summaries. Routing publishes only bounded requirements and a policy-appropriate
user reference, then rendezvous returns the selected Tentacle's XMTP endpoint for a direct DM.
Failover does not silently copy private memory.

The local deterministic simulator exercises Council joins, announcements, heartbeats, capabilities,
routing and offers, lease issuance, Tentacle failure and failover, persona arguments,
one-Cthulhu-one-vote governance, Agenda parent hashes, invitations, multi-level propagation,
loop/duplicate suppression, bounded depth/fan-out, acknowledgements, useful-outcome contribution
credit, and replay-safe persistence. It does not connect to a live XMTP group or ERC-8004 deployment.

Run that opt-in local scenario without starting the XMTP DM sidecar or a model adapter:

```bash
cargo run --manifest-path cthuwu/Cargo.toml --package cthuwu -- \
  --data-dir /tmp/cthuwu-council-sim --xmtp-env local --council-simulate
```

It prints a deterministic JSON report and stores its combined replay-safe checkpoint under
`state/council/` in the selected protected data directory. Omitting `--council-simulate` preserves
the existing standalone `uwubot` behavior.

This is local deterministic orchestration, not a live protocol dispatcher: membership, Tentacle,
and routing envelopes exercise the in-memory transport, while lease, governance, and propagation
engines are invoked directly. A general envelope-to-engine dispatcher, per-message transactional
coordinator, live XMTP group, production signature scheme, and ERC-8004 integration remain future
adapter work.

## Model modes

The compiled preference is Venice's TEE-backed DeepSeek V4 Flash model. Configure either
`VENICE_API_KEY` or `UWUBOT_VENICE_API_KEY` in the runtime environment. The optional Venice timeout
is a 1–300 second cap for operator routes and defaults to 120 seconds:

```bash
VENICE_API_KEY='...' \
UWUBOT_VENICE_TIMEOUT_SECONDS=120 \
./uwu.sh
```

Before the first prompt content is sent, Cthuwu requires the live Venice catalog to report the exact
`e2ee-deepseek-v4-flash` model as text, TEE-attestable, and function-calling, then performs a fresh
nonce-bound baseline attestation. Catalog capabilities and attestation freshness are cached
independently: a successful catalog check remains valid for four hours, while attestation must be
renewed after five minutes. A failed attestation does not discard a fresh catalog result. Requests
explicitly set `enable_e2ee=false`, so this is TEE-only inference with ordinary TLS—not Venice's
separate full E2EE streaming protocol. Cthuwu checks Venice's server verification and nonce/model
binding, rejects explicitly reported debug mode, and requires bounded nonempty provider and
signing-address fields. It does not yet independently parse Intel/NVIDIA evidence or verify response
signatures.

Venice's supplemental system prompt and provider-native web, scraping, citation, and X-search
features are explicitly disabled. Public web search remains the separate opt-in Brave tool described
below.

If the Venice key is absent, exhausted, rejected, rate-limited, or the provider/attestation fails,
Cthuwu tries the configured loopback Ollama model and then its deterministic local response. It never
silently substitutes the non-TEE `deepseek-v4-flash` model, and a locally selected provider never
falls forward to a remote provider. A provider failure enters a short lane-aware cooldown so public
failures do not suppress longer operator attempts and neither lane repeatedly hits the same failed
endpoint.

The bridge's default authenticated envelope is 300 seconds and keeps one second for returning the
XMTP response. Rust applies a 120-second work ceiling to public chat while operator requests may use
at most 299 seconds. Public remote inference gets at most 30 seconds. Before starting operator
Venice, Cthuwu reserves two capped local model phases (up to the 75-second safety cap, or a smaller
configured Ollama timeout, each), one model-selected tool phase of up to 30 seconds, and a one-second
deterministic margin. The default 181-second reserve makes Venice's effective maximum about 118
seconds even though its
configured cap defaults to 120 seconds. Model-selected operator tools also preserve enough remaining
time for a final local completion.

The provider cap applies to the whole candidate route—catalog, attestation, completion, optional
public search/continuation, and policy repair—not independently to every call. Each HTTP phase is
additionally clamped to the time remaining in that candidate. Public completions request at most 300
output tokens; operator completions retain their 1,000-token limit. Timeout logs identify the
provider, lane, and phase but never include prompt text.

For Ollama's OpenAI-compatible endpoint:

```bash
UWUBOT_MODEL=ollama \
UWUBOT_OLLAMA_ENDPOINT=http://127.0.0.1:11434/v1 \
UWUBOT_OLLAMA_MODEL=qwen3:8b \
UWUBOT_OLLAMA_TIMEOUT_SECONDS=75 \
./uwu.sh
```

Automatic Ollama fallback is restricted to a credential-free loopback HTTP endpoint and bypasses
ambient HTTP proxy settings. The configured timeout accepts 1–300 seconds, but routed local model
phases are capped at 75 seconds so a larger setting cannot consume time reserved for continuation;
each Ollama HTTP request is clamped again to the candidate's remaining authenticated time. The legacy
`UWUBOT_MODEL_ENDPOINT` and `UWUBOT_MODEL_NAME` values still override Ollama when
`UWUBOT_MODEL=ollama` is explicitly selected. That legacy startup override wins for the running
process; remove it on later launches if `/model` should use the persisted slot instead.

Treat that loopback endpoint as part of the trusted node boundary: on a shared host, another local
process could impersonate an unauthenticated Ollama listener. Run Cthuwu and Ollama in a dedicated
single-tenant account, VM, or network namespace.

For another OpenAI-compatible provider, select `UWUBOT_MODEL=openai` and set
`UWUBOT_MODEL_API_KEY`, `UWUBOT_MODEL_ENDPOINT`, and `UWUBOT_MODEL_NAME`. The XMTP transport
subprocess receives an allowlisted environment and cannot see model credentials. Operators can later
use `/provider` to inspect or select `venice`, `ollama`, `openai`, or `deterministic`, and `/model`
to inspect the current route, list configured slots, or change the selected provider's bounded model
ID. A new model ID is verified by its provider on the next inference request; failure enters the
normal local fallback path. Only provider/model names persist in owner-only `state/inference.json`;
keys and endpoints do not. Route changes apply to subsequent requests; already-running inference is
allowed to finish under the route it started with.

To allow the public model to request web results, configure Brave Search in the environment:

```bash
UWUBOT_MODEL=openai \
UWUBOT_MODEL_API_KEY='...' \
UWUBOT_WEB_SEARCH=brave \
UWUBOT_WEB_SEARCH_API_KEY='...' \
./uwu.sh
```

This is opt-in and requires the effective provider to support standard tool calls. The runtime
exposes `web_search` only when the current public message explicitly asks for current or
web-verifiable information; ordinary chatter, stable facts, and policy repair receive no search tool
schema. When the model invokes it, its bounded query is sent to Brave and up to five bounded HTTP(S)
results return as untrusted context. The public tool schema contains no shell or filesystem
capability. Search credentials, like model credentials, are rejected on the command line and
stripped from build and transport subprocesses.

## Container

The container packages Rust, Node, the Agent SDK, and its native binding while preserving the one-command runtime:

```bash
docker build -t cthuwu .
docker volume create cthuwu-data
docker volume create cthuwu-workspace
docker run --rm -it --init \
  -v cthuwu-data:/data \
  -v cthuwu-workspace:/workspace \
  cthuwu
```

The container keeps private identity/contact state under `/data` and operator-visible files under
`/workspace`; the image seeds the workspace volume with project context and skill metadata on first
creation. Bind-mount a real working tree there when the operator should inspect or change source.
File tools never receive the data directory as their root, and startup rejects an overlapping custom
root. Pass
`-e UWUBOT_XMTP_ENV=production` when operating the website's intro Tentacle.

The image does not bundle Ollama. Because automatic fallback intentionally accepts only loopback,
run Ollama in the same network namespace. On Linux, a host Ollama service can be reached by adding
`--network host` to `docker run`; otherwise the fallback proceeds to the deterministic local voice.

## Browser deployment

Pushes to `main` build and deploy `web/dist` to [cthuwu.app](https://cthuwu.app). The browser has no
XMTP build-time configuration: it always uses XMTP `production`.

The browser always opens its initial DM with the hard-coded intro Tentacle at
`0x0bf56d21a7392db33b0e646ebeb2a64c14cf04db`. A planned Base registry contract will replace this
bootstrap constant with discovery of registered intro Tentacles.

The root page publishes an absolute Open Graph/Twitter large-image card at
`https://cthuwu.app/cthuwu-og.jpg`.

The browser wallet is stored in local storage. Its XMTP Browser SDK message database is currently unencrypted. The settings dialog says this explicitly; an identity export recovers the wallet/inbox, not message history or necessarily the same installation.

## Privacy and security

- Never commit wallet keys, XMTP database keys, model credentials, generated databases, or contact notes.
- Use dedicated identities with no material funds.
- Normal logs omit keys and message bodies.
- The operator allowlist is environment-scoped, atomically stored at owner-only
  `state/operators.json` using config version 3, and keyed by exact authenticated 64-character XMTP
  inbox ID. This does not distinguish installations within one inbox.
- Operator mode is deliberate remote code execution. File helpers are rooted and reject traversal
  and direct symlink targets, but direct `/exec` and exact-command-bound natural `exec` are not a
  filesystem sandbox and can exercise every permission of the `uwubot` OS account. Run it under a
  dedicated service identity or container.
- Public model calls expose only optional web search. Operator model calls receive a current-message
  closed tool inventory: bounded reads/search, plus at most one exact natural `exec` or one create-only
  skill when explicitly authorized. Workspace, history, contact, and tool text cannot grant either
  effect. Neither tool set is available to Council Actions.
- Council envelopes are bounded, versioned, sender-checked, expiry-checked, replay-suppressed, and
  fenced by Tentacle incarnation or lease generation where applicable.
- Production signatures are not simulated. The deterministic signer is test-only; live adapters
  must bind authenticated senders to Cthulhu/Tentacle identities explicitly.
- Evolution Nature and awakening files use a separate local HMAC key. That symmetric tag detects
  unauthorized file changes only while the key and service account remain protected; it is not a
  peer-verifiable or production Council signature.
- Hermes state accepts only closed aggregate payload types and excludes raw DMs, contact IDs/notes,
  credentials, and private memory. Tool-operation records cannot carry paths, shell commands, output,
  or arguments; bounded skill prose remains hostile input. There is no live gossip transport or
  peer-key provisioning, and persisted peer IDs are not proof of authentication.
- Scales outcomes and lineage records remain non-effectful until an authenticated operator acts.
  The UWU observer is read-only: it holds no private key, signs no transfer, and cannot spend an
  emergency-survival recommendation. Public balances contribute only a period-averaged Engagement
  bonus, never Tentacle Wealth, starvation relief, stake, reward, or lifecycle authority. The latter
  remain adapter-only APIs pending cryptographically bound node/operator evidence; Council
  contribution credit remains non-financial.
- Heartbeats, load, sessions, user references, contact memory, and conversation content do not go
  on-chain.
- Council votes and propagated requests cannot override a Tentacle operator's local security policy.
- Opting into matching permits other opted-in people to see the chosen display name and matching terms, but never the inbox ID; Cthuwu does not make automatic introductions.
- A person can ask Cthuwu in ordinary language to show, correct, or delete their local contact note.
  Deletion cannot erase message copies already delivered over XMTP.

## License

Apache-2.0. See [LICENSE](LICENSE).
