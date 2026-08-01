# Cthuwu memory

Last reviewed: 2026-08-01

## Product

- Cthuwu is a cute little eldritch horror buddy.
- People chat with Cthuwu over XMTP.
- The public-facing client is a static web deployment.
- The operator runs the companion locally as a Rust CLI/daemon.
- Early-development workflow is direct commits to `main`.

## Architecture

- Browser: `@xmtp/browser-sdk`, static Vite output, dedicated browser identity by default.
- Runtime: Rust CLI with transport, companion, model, and state boundaries.
- Persistence: encrypted XMTP database plus an application state store for processed-message idempotency.
- Model access: adapter boundary; local models should be first-class.
- Text-only DMs are the first vertical slice.

## Deployment

- The GitHub repository is public; keep it public unless Dean explicitly asks otherwise.
- `.github/workflows/pages.yml` builds `web/` on pushes to `main` and deploys `web/dist` with GitHub Pages Actions.
- The custom domain is `cthuwu.app`; Actions-based Pages deployments configure it through GitHub rather than a `CNAME` file.
- Public build configuration comes from the `VITE_XMTP_ENV` and `VITE_XMTP_BOT_ADDRESS` repository variables.

See `ARCHITECTURE.md` and `docs/decisions/`.

## Reference projects

- `pierce403/ramus` demonstrates a static browser XMTP client talking to a locally operated bot.
- XMTP core: https://github.com/xmtp/libxmtp
- Agent etiquette: https://recurse.bot/

## Open questions

- Which model backend should ship first: OpenAI-compatible HTTP, Ollama, or both?
- Should visitors use an ephemeral identity, connect an existing wallet, or choose?
- What address or ENS name will be Cthuwu's production XMTP identity?
- Should one local process serve exactly one companion identity or support profiles?
- Should conversation memory remain per-XMTP inbox, be user-editable, and/or expire?

## Current milestone

Pass one real end-to-end test:

1. Start the Rust runtime locally.
2. Connect from the static web client on the same XMTP environment.
3. Send a text message.
4. Receive one Cthuwu reply.
5. Restart both sides and verify identity/history persistence.
