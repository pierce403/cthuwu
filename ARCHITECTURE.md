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

The current low-friction identity is an ephemeral EOA stored in browser local storage. It must be presented honestly: clearing site data loses that local key unless the identity has another recovery path.

### Rust runtime

The `cthuwu` binary will expose:

- `cthuwu init`: create/configure a dedicated companion identity and data directory;
- `cthuwu serve`: sync and stream XMTP conversations, generate replies, and shut down cleanly;
- `cthuwu status`: show non-secret identity and configuration health;
- `cthuwu doctor`: test storage, network, and model connectivity without sending a chat message.

The initial crate establishes the command and configuration boundary. Direct libxmtp integration follows behind a `Transport` trait so protocol churn does not spread through persona/model code.

### Companion core

The core owns message policy:

1. accept supported text DMs;
2. deduplicate by XMTP message ID;
3. apply consent, size, and concurrency limits;
4. construct bounded conversation context;
5. invoke the configured model adapter;
6. send a text response;
7. record completion without logging plaintext by default.

### Model adapters

A model adapter receives a structured request and returns text. Planned adapters:

- OpenAI-compatible HTTP APIs;
- Ollama/local HTTP;
- deterministic echo adapter for end-to-end tests.

The transport layer never knows which model is selected.

## Trust boundaries

| Boundary | Untrusted input | Required control |
|---|---|---|
| Browser → XMTP | Visitor text and identity | Consent, message-size limits |
| XMTP → runtime | Message content and metadata | Decode validation, deduplication, rate limits |
| Runtime → model | Conversation content | Explicit provider selection, bounded context |
| Model → runtime | Generated text/tool requests | Output limit; no implicit tool execution |
| Disk | Keys, databases, history | Encryption, restrictive permissions, backups |

## Identity and persistence

The runtime should use a dedicated XMTP identity. Identity material and database encryption keys live in the operator data directory, never environment files committed to git. Atomic creation is required: a crash during `init` must not strand half-written secrets.

Each XMTP environment gets a separate data directory to prevent accidental dev/production identity mixing.

## First vertical slice

The first slice deliberately excludes groups, attachments, reactions, tools, long-term semantic memory, and autonomous actions. Success means one persisted identity can exchange text DMs with the static client across restarts.

## Testing

- Unit: config parsing, filtering, deduplication, prompt assembly, model adapters.
- Integration: fake transport plus echo model.
- End-to-end: browser SDK and Rust runtime on XMTP dev, then production.
- Recovery: restart, duplicate delivery, network loss, corrupt/missing configuration.
