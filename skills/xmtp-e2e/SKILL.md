---
name: xmtp-e2e
description: Verify Cthuwu browser-to-runtime XMTP messaging and persistence.
---

# XMTP end-to-end verification

## Preconditions

- Use dedicated test identities with no material funds.
- Use Node 22 or newer and Rust 1.97 or newer.
- Confirm browser and runtime use the same explicit XMTP environment.
- Confirm logs do not include private keys or plaintext message bodies.
- Keep the runtime data directory outside the repository with owner-only permissions.
- Use the deterministic local model unless the test explicitly covers another provider.
- Record dependency versions and the runtime commit.
- Record only public Ethereum addresses. Do not record inbox IDs, inspect browser local storage,
  or read identity/database files.

## Procedure

1. Run the locked sidecar, web, Rust, and container verification commands from `README.md`, then
   run `npm --prefix agent audit --audit-level=low` and
   `npm --prefix web audit --audit-level=low`.
2. Build `agent/dist/index.js` and the release `uwubot` binary.
3. Start `uwubot` with explicit `UWUBOT_DATA_DIR`, `UWUBOT_XMTP_ENV`, and
   `UWUBOT_MODEL=deterministic`. Remove ambient wallet, database, and model credentials from the
   launch environment. Record only the public Ethereum address from the successful connection
   diagnostic.
4. Configure the web build for the same environment and public bot address, then deploy or serve
   that exact build.
5. Open the site in a clean browser profile with no wallet interaction. Read the public browser
   address from the identity dialog, reload, and confirm the same public address persists.
6. Send a unique marker. Verify one outbound bubble, exactly one inbound reply, one owner-only
   `contacts/<lowercase-inbox-id>.md`, and one new hashed replay tombstone. Do not print the inbox
   ID, note contents, or message body in runtime diagnostics.
7. Complete onboarding with harmless answers, including Markdown-like input. Verify exact storage
   with quiet matches and confirm every logical answer line remains blockquoted.
8. Exercise `/profile`, `/set`, `/share off`, `/share on`, `/pause`, `/resume`, `/matches`, and
   `/forget confirm`. A comparison contact must remain hidden until both contacts explicitly opt
   in; suggestions may expose only chosen names and matching terms, never inbox IDs or unrelated
   profile text.
9. Stop `uwubot` gracefully and restart the same binary with the same data directory. Confirm the
   bot public address and identity-file metadata are unchanged, then reload the same browser
   profile and verify its public address, history, contact state, replay state, and conversation
   continuity.
10. Remove generated contact records through `/forget confirm` and record sanitized results in
    `docs/test-runs/`. Keep generated identities, databases, contacts, and test data out of git.

## Pass criteria

- One inbound message produces exactly one response.
- Both identities and conversation history persist across restart.
- Contact state and replay tombstones persist across restart.
- Matching is bilateral opt-in and discloses only consented suggestion fields.
- No secret or message body appears in normal logs.
- A failure is surfaced with an actionable error rather than silent loss.
- Manual live evidence does not satisfy a criterion that explicitly requires the real XMTP test to
  run in CI.
