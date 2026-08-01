# cthuwu

A tiny eldritch companion you can message over [XMTP](https://xmtp.org).

Cthuwu has two parts:

- `web/`: a small browser client that builds to static files and automatically creates a dedicated local XMTP identity.
- `cthuwu/`: the Rust `uwubot` backend, which owns the companion identity and one-to-one contact memory.

## Current behavior

On first visit, the browser generates a random EOA private key and a separate XMTP database-encryption key, stores both in local storage, and automatically connects to the configured XMTP network. Returning visits reuse those keys. Clearing site data loses this browser identity.

The Rust contact engine creates `contacts/<inbox-id>.md` when it sees a new inbox. Its onboarding conversation asks what the person wants to be called, about their hopes and dreams, resources they may want to share, and support they need. The notes contain only answers the person supplied; model guesses must never be written as facts.

The contact engine and stdin integration harness are working. Native libxmtp transport wiring remains the next backend step.

## Repository layout

```text
cthuwu/        Rust uwubot backend
web/           Static browser chat client
docs/          Architecture, decisions, and research notes
skills/        Reusable project-specific agent procedures
```

## Development

```bash
cargo test --manifest-path cthuwu/Cargo.toml
npm --prefix web ci
npm --prefix web run build
```

Exercise the contact flow locally:

```bash
cargo run --manifest-path cthuwu/Cargo.toml --bin uwubot -- \
  --stdin-inbox 012345abcdef
```

Each line on stdin is treated as the next message from that test inbox.

## Deployment

Pushes to `main` build and deploy `web/dist` to [cthuwu.app](https://cthuwu.app). The build uses:

- `VITE_XMTP_ENV`: `dev`, `production`, or `local`; defaults to `dev`.
- `VITE_XMTP_BOT_ADDRESS`: the companion's Ethereum address or ENS name.

## Privacy and security

- Never commit wallet keys, database keys, model credentials, or generated databases.
- `contacts/` is ignored because it contains personal statements.
- Use a dedicated, minimally funded companion identity.
- Visitors should eventually get an export/recovery and delete/reset control for their browser identity and contact memory.

## License

Apache-2.0. See [LICENSE](LICENSE).
