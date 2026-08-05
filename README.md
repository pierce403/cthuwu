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
- Deterministic, Ollama, and OpenAI-compatible model modes are available. An optional Brave Search
  adapter gives the public model one closed `web_search` tool and no local file or process tools. No
  message reaches a model or search provider unless the node operator explicitly configures it.
- A node operator can authorize an exact XMTP inbox immediately with a local CLI command.
  Active operator DMs enter a separate all-caps operator harness with bounded file/search tools and
  intentionally privileged shell execution; public, stale, and revoked messages cannot reach it.
- Structured, versioned Cthulhu personalities include deterministic Archivist, Hermit, Merchant,
  Wanderer, Oracle, and Trickster personas with different local policy positions without an LLM.
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
state/operators.json
state/processed/<hashed-message-id>
state/council/<bounded-state-name>.json
state/xmtp-identity.json
state/xmtp/<environment>/
```

The identity file contains a private key protected by owner-only filesystem permissions. Back up the entire data directory securely. The runtime lock contains only a process ID and process-start timestamp, and is recovered after a stopped process without confusing a reused PID for the old bot. Do not delete only the XMTP database: doing so creates a new installation and can eventually exhaust the inbox installation limit.

For an offline contact-flow harness:

```bash
./uwu.sh --data-dir /tmp/cthuwu-harness --stdin-inbox 012345abcdef
```

Each input line is the next message from that test inbox.

The hidden stdin harness is deliberately public-only. It cannot simulate an operator, even when its
inbox argument matches an active operator record.

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

Once active, the operator may use direct `/exec`, `/read`, `/write`, `/edit`, `/search`, and `/qmd`
commands or ask an Ollama/OpenAI-compatible tool-calling model to use the same closed tool set. QMD
is an optional external adapter; set `UWUBOT_QMD` to a compatible executable that supports
`qmd query <query> --json`. Public users are not shown this syntax and cannot invoke these tools.
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

The default `deterministic` mode keeps all conversation content local and is useful for bring-up.

For Ollama's OpenAI-compatible endpoint:

```bash
UWUBOT_MODEL=ollama \
UWUBOT_MODEL_ENDPOINT=http://127.0.0.1:11434/v1 \
UWUBOT_MODEL_NAME=qwen3:8b \
./uwu.sh
```

For another OpenAI-compatible provider, select `UWUBOT_MODEL=openai` and set `UWUBOT_MODEL_API_KEY`, `UWUBOT_MODEL_ENDPOINT`, and `UWUBOT_MODEL_NAME`. The XMTP transport subprocess receives an allowlisted environment and cannot see the model credential.

To allow the public model to request web results, configure Brave Search in the environment:

```bash
UWUBOT_MODEL=openai \
UWUBOT_MODEL_API_KEY='...' \
UWUBOT_WEB_SEARCH=brave \
UWUBOT_WEB_SEARCH_API_KEY='...' \
./uwu.sh
```

This is opt-in and requires an Ollama or OpenAI-compatible model that supports standard tool calls.
When the model invokes `web_search`, its bounded query is sent to Brave and up to five bounded
HTTP(S) results return as untrusted context. The public tool schema contains no shell or filesystem
capability. Search credentials, like model credentials, are rejected on the command line and stripped
from build and transport subprocesses.

## Container

The container packages Rust, Node, the Agent SDK, and its native binding while preserving the one-command runtime:

```bash
docker build -t cthuwu .
docker volume create cthuwu-data
docker run --rm -it --init -v cthuwu-data:/data cthuwu
```

Pass `-e UWUBOT_XMTP_ENV=production` when operating the website's intro Tentacle.

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
  and direct symlink targets, but `/exec` is not a filesystem sandbox and can exercise every
  permission of the `uwubot` OS account. Run it under a dedicated service identity or container.
- Public model calls expose only optional web search. Operator model calls expose only the closed
  local operator tool set. Neither tool set is available to Council Actions.
- Council envelopes are bounded, versioned, sender-checked, expiry-checked, replay-suppressed, and
  fenced by Tentacle incarnation or lease generation where applicable.
- Production signatures are not simulated. The deterministic signer is test-only; live adapters
  must bind authenticated senders to Cthulhu/Tentacle identities explicitly.
- Heartbeats, load, sessions, user references, contact memory, and conversation content do not go
  on-chain.
- Council votes and propagated requests cannot override a Tentacle operator's local security policy.
- Opting into matching permits other opted-in people to see the chosen display name and matching terms, but never the inbox ID; Cthuwu does not make automatic introductions.
- A person can ask Cthuwu in ordinary language to show, correct, or delete their local contact note.
  Deletion cannot erase message copies already delivered over XMTP.

## License

Apache-2.0. See [LICENSE](LICENSE).
