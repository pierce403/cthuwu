# cthuwu

A tiny eldritch companion people can message over [XMTP](https://xmtp.org).

Cthuwu has two user-facing pieces:

- `web/`: a static browser client that creates a dedicated local identity and opens a one-to-one DM;
- `cthuwu/`: the single Rust command, `uwubot`, which owns contact memory, consent, matching policy, and model access.

`uwubot` supervises the supported `@xmtp/agent-sdk` transport in `agent/`. Node is an internal transport detail: the operator still starts and stops one command. Direct libxmtp crates are not currently a stable, published Rust integration surface.

The detailed product contract and remaining release gates live in [FEATURES.md](FEATURES.md).

## What works

- The browser generates an environment-scoped wallet before connecting, reuses it on reload, and supports passphrase-encrypted identity export/import and confirmed reset.
- The web client loads text history, streams new text, and preserves a draft after a failed send.
- `uwubot` creates a persistent XMTP wallet and encrypted database on first start, then reuses both.
- A new sender gets exactly `contacts/<inbox-id>.md` and a deterministic conversation about their name, hopes, possible contributions, and needs.
- `/profile`, `/set`, `/skip`, `/share`, `/matches`, `/pause`, `/resume`, and `/forget confirm` provide inspectable consent and data controls.
- Inbound message IDs are durably deduplicated; storage, model context, bridge concurrency, and message sizes are bounded.
- Deterministic, Ollama, and OpenAI-compatible model modes are available. No message reaches a model provider unless the operator explicitly selects it.

The code paths and local tests are working. A live browser-to-bot XMTP exchange still has to pass the release gate before this project claims production interoperability.

## Build and verify

Requirements: Node 22 or newer and Rust 1.97 or newer.

```bash
npm --prefix agent ci
npm --prefix agent run typecheck
npm --prefix agent test
npm --prefix agent run build

npm --prefix web ci
npm --prefix web run typecheck
npm --prefix web test
npm --prefix web run build

cargo fmt --manifest-path cthuwu/Cargo.toml --all -- --check
cargo test --manifest-path cthuwu/Cargo.toml --locked
cargo clippy --manifest-path cthuwu/Cargo.toml --all-targets --locked -- -D warnings
cargo build --manifest-path cthuwu/Cargo.toml --release --locked
```

## Run `uwubot`

Build once, then run from the repository root so the default sidecar path resolves:

```bash
UWUBOT_DATA_DIR="$PWD/cthuwu-data" \
UWUBOT_XMTP_ENV=dev \
./cthuwu/target/release/uwubot
```

The first successful connection logs the bot's public Ethereum address without logging its keys. Set the website to the same network and that address, then redeploy:

```bash
gh variable set VITE_XMTP_ENV --body dev
gh variable set VITE_XMTP_BOT_ADDRESS --body 0xYOUR_CTHUWU_ADDRESS
gh workflow run pages.yml
```

Normal state is kept below `UWUBOT_DATA_DIR`:

```text
contacts/<inbox-id>.md
state/environment
state/processed/<hashed-message-id>
state/xmtp-identity.json
state/xmtp/<environment>/
```

The identity file contains a private key protected by owner-only filesystem permissions. Back up the entire data directory securely. Do not delete only the XMTP database: doing so creates a new installation and can eventually exhaust the inbox installation limit.

For an offline contact-flow harness:

```bash
cargo run --manifest-path cthuwu/Cargo.toml --bin uwubot -- \
  --data-dir /tmp/cthuwu-harness \
  --stdin-inbox 012345abcdef
```

Each input line is the next message from that test inbox.

## Model modes

The default `deterministic` mode keeps all conversation content local and is useful for bring-up.

For Ollama's OpenAI-compatible endpoint:

```bash
UWUBOT_MODEL=ollama \
UWUBOT_MODEL_ENDPOINT=http://127.0.0.1:11434/v1 \
UWUBOT_MODEL_NAME=qwen3:8b \
./cthuwu/target/release/uwubot
```

For another OpenAI-compatible provider, select `UWUBOT_MODEL=openai` and set `UWUBOT_MODEL_API_KEY`, `UWUBOT_MODEL_ENDPOINT`, and `UWUBOT_MODEL_NAME`. The XMTP transport subprocess receives an allowlisted environment and cannot see the model credential.

## Container

The container packages Rust, Node, the Agent SDK, and its native binding while preserving the one-command runtime:

```bash
docker build -t cthuwu .
docker volume create cthuwu-data
docker run --rm -it --init -v cthuwu-data:/data cthuwu
```

Pass `-e UWUBOT_XMTP_ENV=production` only after the production website is configured for the same environment.

## Browser deployment

Pushes to `main` build and deploy `web/dist` to [cthuwu.app](https://cthuwu.app). The build reads:

- `VITE_XMTP_ENV`: `dev`, `production`, or `local`;
- `VITE_XMTP_BOT_ADDRESS`: Cthuwu's public Ethereum address or ENS name.

The browser wallet is stored in local storage. Its XMTP Browser SDK message database is currently unencrypted. The settings dialog says this explicitly; an identity export recovers the wallet/inbox, not message history or necessarily the same installation.

## Privacy and security

- Never commit wallet keys, XMTP database keys, model credentials, generated databases, or contact notes.
- Use dedicated identities with no material funds.
- Normal logs omit keys and message bodies.
- Opting into matching permits other opted-in people to see the chosen display name and matching terms, but never the inbox ID; Cthuwu does not make automatic introductions.
- `/forget confirm` deletes the caller's local contact note. It cannot erase copies already delivered over XMTP.

## License

Apache-2.0. See [LICENSE](LICENSE).
