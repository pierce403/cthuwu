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
- Runtime: Rust `uwubot` supervisor with companion, model, and state boundaries; private official Agent SDK sidecar for XMTP transport.
- Persistence: encrypted XMTP database plus an application state store for processed-message idempotency.
- Model access: adapter boundary; local models should be first-class.
- Text-only one-to-one DMs are the first vertical slice.
- Browser identities are generated and connected automatically, then persisted in local storage.
- The sole backend command is `uwubot`.
- Contact notes default to `contacts/<inbox-id>.md` and are ignored by git because they contain personal statements.
- Onboarding collects name, hopes, possible contributions, and needs as user-asserted information.
- Matching is bilateral opt-in, explainable, and suggestion-only; chosen names and matching terms may be shown, but inbox IDs are not disclosed.
- Browser identity exports are passphrase-encrypted wallet backups. The Browser SDK database is unencrypted and is not included in that export.
- Backend secrets are atomically persisted at `state/xmtp-identity.json`; XMTP databases are environment-specific below `state/xmtp/`.
- `@xmtp/agent-sdk@2.3.0` is the supported first transport. Direct libxmtp remains a later option because its Rust crates are unpublished internal APIs.

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

- What address or ENS name will be Cthuwu's production XMTP identity?
- Should one local process serve exactly one companion identity or support profiles?
- Should conversation memory remain per-XMTP inbox, be user-editable, and/or expire?
- What retention period should apply to opaque processed-message tombstones and contact notes?

## Current milestone

The 2026-08-01 manual XMTP `dev` release-gate run passed browser identity, exactly-once reply,
contact onboarding, bilateral matching, deletion, and restart/persistence checks. Sanitized evidence
is in `docs/test-runs/2026-08-01-xmtp-dev.md`. The release criterion that explicitly requires a
real XMTP/browser job in GitHub CI remains open because CI does not run that job yet.

Operational notes from that run:

- The persistent dev identity is the public address
  `0x52a93ca2cf0629bcfe7bf7824df7c18268c360f7`; its secret state remains outside the repository.
- The SQLCipher 4.6.1 `sqlcipherCodecAttach: no codec attached to db` warning is emitted when XMTP
  reapplies a key to an already-keyed pooled connection. With the persisted 32-byte key supplied,
  it does not indicate an unencrypted database; do not suppress all native stderr.
- Sidecar-to-Rust JSONL frames are independently capped at 256 KiB, and contact answers normalize
  CRLF/bare CR before blockquoting so CommonMark line endings cannot escape note structure.
- Keep sidecar Vitest at `3.2.7` or newer within the pinned major line; earlier 3.2 releases have a
  critical development-server advisory.

The next release task is to add a real browser/XMTP job to GitHub CI, then check the final release
criterion without weakening the dedicated-identity or no-secret-log rules.

The original manual milestone was:

1. Build the Agent SDK sidecar and start `uwubot` locally.
2. Connect from the static web client on the same XMTP environment.
3. Send a text message.
4. Receive one Cthuwu reply.
5. Restart both sides and verify identity/history persistence.
