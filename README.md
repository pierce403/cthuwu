# cthuwu

A tiny eldritch companion you can message over [XMTP](https://xmtp.org).

Cthuwu has two parts:

- `web/`: a deliberately small browser client that builds to static files.
- `cthuwu/`: a local Rust CLI/daemon that owns the companion identity, receives XMTP messages, and replies.

## Status

The repository currently contains the project architecture, agent memory, an initial frontend shell, and the Rust CLI boundary. The first implementation milestone is a real end-to-end XMTP text exchange between the browser and the locally running CLI.

## Repository layout

```text
cthuwu/        Rust CLI and long-running companion process
web/           Static browser chat client
docs/          Architecture, decisions, and research notes
skills/        Reusable project-specific agent procedures
AGENTS.md      Canonical instructions for coding agents
MEMORY.md      Durable project memory index
SKILLS.md      Project skill index
```

## Intended flow

1. The operator starts `cthuwu serve` locally.
2. The CLI opens its encrypted XMTP database and listens for text messages.
3. A visitor opens the static site, connects a wallet, and starts a DM with Cthuwu.
4. The CLI sends each inbound message to a configurable local or hosted model backend.
5. Cthuwu replies over XMTP.

See [ARCHITECTURE.md](ARCHITECTURE.md) for trust boundaries and implementation decisions.

## Development

The concrete build commands will stabilize with the first working XMTP slice:

```bash
cargo test --manifest-path cthuwu/Cargo.toml
npm --prefix web install
npm --prefix web run build
```

## Security

Never commit wallet keys, XMTP database keys, model API keys, or generated local databases. The bot should use a dedicated identity with limited funds.

## License

Apache-2.0. See [LICENSE](LICENSE).
