# Architecture

## Goal

Cthuwu is a local-first companion process that receives and sends XMTP messages. A small browser application gives visitors a friendly way to open a DM without requiring an application server.

## Components

### Static web client

The source in `web/` builds to HTML, CSS, JavaScript, and WASM assets suitable for any static host. It:

- creates or loads a browser-side identity;
- connects to a configured XMTP environment;
- creates a DM with the configured Cthuwu address;
- renders text history and streams new messages;
- sends text messages.

The low-friction identity is a randomly generated EOA stored in browser local storage. The client connects automatically on load and supports a passphrase-encrypted wallet export. It must be presented honestly: the current XMTP Browser SDK database is unencrypted, clearing site data loses the identity without an export, and an identity export is not a history backup.

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

## Trust boundaries

| Boundary | Untrusted input | Required control |
|---|---|---|
| Browser → XMTP | Visitor text and identity | Consent, message-size limits |
| XMTP → runtime | Message content and metadata | Decode validation, deduplication, rate limits |
| Runtime → model | Conversation content | Explicit provider selection, bounded context |
| Model → runtime | Generated text/tool requests | Output limit; no implicit tool execution |
| Rust → XMTP subprocess | JSONL metadata and text | Allowlisted environment, bounded queue and frames |
| Disk | Keys, databases, history | Encryption where supported, restrictive permissions, backups |

## Identity and persistence

The runtime uses a dedicated XMTP identity. The sidecar atomically creates a wallet key and independent database-encryption key at `state/xmtp-identity.json`, then reuses the environment-specific encrypted database below `state/xmtp/`. Owner-only permissions are enforced on Unix. Operator-provided keys must match persisted state or startup fails closed.

Each XMTP environment gets a separate data directory to prevent accidental dev/production identity mixing.

## First vertical slice

The first slice deliberately excludes groups, attachments, reactions, tools, long-term semantic memory, and autonomous actions. Success means one persisted identity can exchange text DMs with the static client across restarts.

## Testing

- Unit: config parsing, filtering, deduplication, prompt assembly, model adapters.
- Integration: JSONL transport contract plus deterministic model.
- End-to-end: browser SDK and Rust runtime on XMTP dev, then production.
- Recovery: restart, duplicate delivery, network loss, corrupt/missing configuration.
